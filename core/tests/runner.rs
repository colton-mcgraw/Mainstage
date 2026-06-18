//! Phase 6 integration tests — pipeline runner and failure handling.
//!
//! Drive a full `parse → analyze → eval_program → run_pipeline` flow and observe
//! side effects (marker files written by `write` steps) to verify sequential
//! execution, failure propagation through the DAG, `allow_failure`, and the
//! pipeline-level `on_success` / `on_failure` hooks with `failed_stage` binding.

use std::path::{Path, PathBuf};

use mainstage_core::{
    CancelToken, NoopReporter, PlanStatus, Source, analyze, eval_program, parse, plan_pipeline,
    run_pipeline, run_pipeline_cancellable, run_pipeline_reported_jobs,
};

/// A unique temporary directory for a single test's marker files.
fn unique_dir(tag: &str) -> PathBuf {
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("ms_run_{tag}_{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Parse, analyze, evaluate, and run the named pipeline (`None` = default).
fn run(src: &str, dir: &Path, pipeline: Option<&str>) -> mainstage_core::Result<()> {
    let program = parse(&Source::from_str("test.ms", src)).expect("parse should succeed");
    let analysis = analyze(&program).expect("analysis should succeed");
    let ctx = eval_program(&program, dir).expect("eval should succeed");
    run_pipeline(&program, pipeline, &ctx, &analysis)
}

/// Like [`run`], but pins the worker budget so the parallel scheduler is exercised.
fn run_jobs(
    src: &str,
    dir: &Path,
    pipeline: Option<&str>,
    jobs: usize,
) -> mainstage_core::Result<()> {
    let program = parse(&Source::from_str("test.ms", src)).expect("parse should succeed");
    let analysis = analyze(&program).expect("analysis should succeed");
    let ctx = eval_program(&program, dir).expect("eval should succeed");
    run_pipeline_reported_jobs(
        &program,
        pipeline,
        &ctx,
        &analysis,
        &mainstage_core::NoopReporter,
        jobs,
    )
}

fn exists(dir: &Path, name: &str) -> bool {
    dir.join(name).exists()
}

// ── Success path ────────────────────────────────────────────────────────────────

#[test]
fn runs_stages_and_on_success_hook() {
    let dir = unique_dir("success");
    let d = dir.display();
    let src = format!(
        r#"
        default pipeline build {{
            stages: [a, b]
            on_success {{ write "{d}/success" content: "ok" }}
        }}
        stage a {{ steps {{ write "{d}/a" content: "x" }} }}
        stage b {{ steps {{ write "{d}/b" content: "x" }} }}
        "#
    );

    run(&src, &dir, None).expect("pipeline should succeed");

    assert!(exists(&dir, "a"), "stage a should have run");
    assert!(exists(&dir, "b"), "stage b should have run");
    assert!(exists(&dir, "success"), "on_success hook should have run");

    let _ = std::fs::remove_dir_all(&dir);
}

// ── Failure propagation ─────────────────────────────────────────────────────────

#[test]
fn failure_propagates_and_binds_failed_stage() {
    let dir = unique_dir("failprop");
    let d = dir.display();
    // Stage `a` fails (no such program). Stage `b` depends on a.outputs, so it must
    // be cancelled. The pipeline on_failure hook binds `failed_stage`.
    let src = format!(
        r#"
        default pipeline build {{
            stages: [a, b]
            on_failure {{ write "{d}/failed_${{failed_stage}}" content: "x" }}
        }}
        stage a {{
            steps {{
                $ ms_no_such_binary_zzz
            }}
            on_failure {{ write "{d}/a_onfail" content: "x" }}
        }}
        stage b {{
            inputs: [a.outputs]
            steps {{ write "{d}/b" content: "x" }}
        }}
        "#
    );

    let result = run(&src, &dir, None);

    assert!(result.is_err(), "pipeline must report failure");
    assert!(exists(&dir, "a_onfail"), "stage on_failure should have run");
    assert!(exists(&dir, "failed_a"), "pipeline on_failure should bind failed_stage=a");
    assert!(!exists(&dir, "b"), "downstream stage b must be cancelled");

    let _ = std::fs::remove_dir_all(&dir);
}

// ── allow_failure ───────────────────────────────────────────────────────────────

#[test]
fn allow_failure_treats_stage_as_succeeded() {
    let dir = unique_dir("allowfail");
    let d = dir.display();
    // Stage `a` fails but is marked allow_failure — the pipeline should still succeed
    // and run its on_success hook.
    let src = format!(
        r#"
        default pipeline build {{
            stages: [a]
            on_success {{ write "{d}/success" content: "ok" }}
            on_failure {{ write "{d}/failure" content: "no" }}
        }}
        stage a {{
            allow_failure: true
            steps {{
                $ ms_no_such_binary_zzz
            }}
        }}
        "#
    );

    run(&src, &dir, None).expect("allow_failure should keep the pipeline green");

    assert!(exists(&dir, "success"), "on_success should run despite the failed stage");
    assert!(!exists(&dir, "failure"), "on_failure must not run");

    let _ = std::fs::remove_dir_all(&dir);
}

// ── Pipeline selection ──────────────────────────────────────────────────────────

#[test]
fn runs_named_pipeline() {
    let dir = unique_dir("named");
    let d = dir.display();
    let src = format!(
        r#"
        pipeline release {{
            stages: [a]
            on_success {{ write "{d}/released" content: "ok" }}
        }}
        stage a {{ steps {{ write "{d}/a" content: "x" }} }}
        "#
    );

    run(&src, &dir, Some("release")).expect("named pipeline should run");
    assert!(exists(&dir, "released"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unknown_pipeline_name_errors() {
    let dir = unique_dir("unknown");
    let src = r#"
        default pipeline dev { stages: [] }
    "#;
    assert!(run(src, &dir, Some("ghost")).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_default_pipeline_errors() {
    let dir = unique_dir("nodefault");
    let src = r#"
        pipeline ci { stages: [] }
    "#;
    assert!(run(src, &dir, None).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

// ── Cross-stage output references ────────────────────────────────────────────────

#[test]
fn downstream_stage_consumes_upstream_outputs() {
    let dir = unique_dir("stageref");
    let d = dir.display();
    // `package` references `compile.outputs` in its inputs. With dependency-ordered
    // resolution, compile runs first and publishes its outputs, so package's context
    // builds and the stage runs (previously this errored at context build).
    let src = format!(
        r#"
        default pipeline build {{
            stages: [compile, package]
        }}
        stage compile {{
            outputs: ["{d}/out/app"]
            steps {{
                write "{d}/out/app" content: "binary"
            }}
        }}
        stage package {{
            inputs:  [compile.outputs]
            outputs: ["{d}/out/app.tar"]
            steps {{
                write "{d}/out/app.tar" content: "archive"
            }}
        }}
        "#
    );

    run(&src, &dir, None).expect("a stage consuming upstream outputs should run");

    assert!(exists(&dir, "out/app"), "compile must produce its output");
    assert!(exists(&dir, "out/app.tar"), "package must run and produce its output");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn outputs_of_stage_outside_pipeline_error() {
    let dir = unique_dir("missingref");
    let d = dir.display();
    // The pipeline runs only `package`; `compile` never executes, so its outputs are
    // unavailable and the reference is a runtime error.
    let src = format!(
        r#"
        default pipeline only_package {{
            stages: [package]
        }}
        stage compile {{
            outputs: ["{d}/out/app"]
            steps {{ mkdir "{d}/out" }}
        }}
        stage package {{
            inputs: [compile.outputs]
            steps {{ mkdir "{d}/pkg" }}
        }}
        "#
    );

    assert!(
        run(&src, &dir, None).is_err(),
        "referencing the outputs of a stage that did not run must error"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ── Phase 24: parallel stage execution ───────────────────────────────────────────

#[test]
fn parallel_runs_all_independent_stages() {
    let dir = unique_dir("par_indep");
    let d = dir.display();
    // Many independent stages with no dependencies — they may run concurrently across
    // workers. Every one must still execute and produce its marker.
    let stages: Vec<String> = (0..12).map(|i| format!("s{i}")).collect();
    let stage_list = stages.join(", ");
    let stage_decls: String = stages
        .iter()
        .map(|s| format!("stage {s} {{ steps {{ write \"{d}/{s}\" content: \"x\" }} }}\n"))
        .collect();
    let src = format!(
        r#"
        default pipeline build {{
            stages: [{stage_list}]
            on_success {{ write "{d}/success" content: "ok" }}
        }}
        {stage_decls}
        "#
    );

    run_jobs(&src, &dir, None, 4).expect("all independent stages should run under 4 workers");

    for s in &stages {
        assert!(exists(&dir, s), "stage {s} should have run");
    }
    assert!(exists(&dir, "success"), "on_success should run after all stages");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn parallel_respects_dependency_order() {
    let dir = unique_dir("par_order");
    let d = dir.display();
    // A diamond: b and c both depend on a's outputs; d depends on both. Each downstream
    // stage consumes its upstream's outputs, so it can only build once the upstream has
    // published them — exercising synchronized output propagation under concurrency.
    let src = format!(
        r#"
        default pipeline build {{
            stages: [a, b, c, e]
        }}
        stage a {{
            outputs: ["{d}/a.out"]
            steps {{ write "{d}/a.out" content: "a" }}
        }}
        stage b {{
            inputs:  [a.outputs]
            outputs: ["{d}/b.out"]
            steps {{ write "{d}/b.out" content: "b" }}
        }}
        stage c {{
            inputs:  [a.outputs]
            outputs: ["{d}/c.out"]
            steps {{ write "{d}/c.out" content: "c" }}
        }}
        stage e {{
            inputs:  [b.outputs, c.outputs]
            outputs: ["{d}/e.out"]
            steps {{ write "{d}/e.out" content: "e" }}
        }}
        "#
    );

    run_jobs(&src, &dir, None, 4).expect("diamond pipeline should succeed under 4 workers");

    assert!(exists(&dir, "a.out"));
    assert!(exists(&dir, "b.out"));
    assert!(exists(&dir, "c.out"));
    assert!(exists(&dir, "e.out"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn try_block_keeps_stage_passing_after_failure() {
    let dir = unique_dir("try_block");
    let d = dir.display();
    // The `try` wraps a command that does not exist; without `try` the stage would fail
    // and the pipeline would error. The post-`try` write proves the stage continued.
    let src = format!(
        r#"
        default pipeline p {{
            stages: [setup]
            on_success {{ write "{d}/ok" content: "x" }}
        }}
        stage setup {{
            steps {{
                try {{
                    $ ms_no_such_binary_zzz
                }}
                write "{d}/after" content: "x"
            }}
        }}
        "#
    );

    run(&src, &dir, None).expect("try must swallow the failure so the stage passes");
    assert!(exists(&dir, "after"), "execution continues after the try block");
    assert!(exists(&dir, "ok"), "pipeline on_success runs — the stage was treated as passed");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn depends_on_orders_stages_without_file_edge() {
    let dir = unique_dir("depends_on_order");
    let d = dir.display();
    // `build` shares no file inputs/outputs with `setup`, so only the explicit
    // `depends_on` edge can sequence them. Its step copies a file `setup` produces;
    // were `build` allowed to run first (or concurrently, under 4 workers), the copy
    // would fail because the source would not yet exist.
    let src = format!(
        r#"
        default pipeline build {{
            stages: [setup, build]
        }}
        stage setup {{
            steps {{ write "{d}/setup.out" content: "ready" }}
        }}
        stage build {{
            depends_on: [setup]
            steps {{ copy "{d}/setup.out" to "{d}/build.out" }}
        }}
        "#
    );

    run_jobs(&src, &dir, None, 4).expect("build must run after setup via depends_on");

    assert!(exists(&dir, "build.out"), "build ran after setup and copied its output");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn parallel_failure_cancels_downstream_only() {
    let dir = unique_dir("par_fail");
    let d = dir.display();
    // `a` fails; `b` depends on it and must be cancelled. `indep` is independent and must
    // still run. The pipeline reports failure overall.
    let src = format!(
        r#"
        default pipeline build {{
            stages: [a, b, indep]
            on_failure {{ write "{d}/failed_${{failed_stage}}" content: "x" }}
        }}
        stage a {{
            outputs: ["{d}/a.out"]
            steps {{
                $ ms_no_such_binary_zzz
            }}
        }}
        stage b {{
            inputs: [a.outputs]
            steps {{ write "{d}/b" content: "x" }}
        }}
        stage indep {{
            steps {{ write "{d}/indep" content: "x" }}
        }}
        "#
    );

    let result = run_jobs(&src, &dir, None, 4);

    assert!(result.is_err(), "pipeline must report failure");
    assert!(!exists(&dir, "b"), "downstream stage b must be cancelled");
    assert!(exists(&dir, "indep"), "independent stage must still run");
    assert!(exists(&dir, "failed_a"), "pipeline on_failure must bind failed_stage=a");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn jobs_one_matches_parallel_results() {
    let dir = unique_dir("par_seq");
    let d = dir.display();
    // The same diamond run with a single worker (sequential) must produce identical side
    // effects to the parallel run.
    let src = format!(
        r#"
        default pipeline build {{
            stages: [a, b, c, e]
            on_success {{ write "{d}/ok" content: "ok" }}
        }}
        stage a {{
            outputs: ["{d}/a.out"]
            steps {{ write "{d}/a.out" content: "a" }}
        }}
        stage b {{ inputs: [a.outputs] outputs: ["{d}/b.out"] steps {{ write "{d}/b.out" content: "b" }} }}
        stage c {{ inputs: [a.outputs] outputs: ["{d}/c.out"] steps {{ write "{d}/c.out" content: "c" }} }}
        stage e {{ inputs: [b.outputs, c.outputs] steps {{ write "{d}/e.out" content: "e" }} }}
        "#
    );

    run_jobs(&src, &dir, None, 1).expect("sequential run should succeed");

    for f in ["a.out", "b.out", "c.out", "e.out", "ok"] {
        assert!(exists(&dir, f), "{f} should exist after a --jobs 1 run");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn parallel_allow_failure_keeps_pipeline_green() {
    let dir = unique_dir("par_allow");
    let d = dir.display();
    // `a` fails but is allow_failure; its dependent `b` must still run because a's
    // outputs are published, and the pipeline succeeds.
    let src = format!(
        r#"
        default pipeline build {{
            stages: [a, b]
            on_success {{ write "{d}/success" content: "ok" }}
            on_failure {{ write "{d}/failure" content: "no" }}
        }}
        stage a {{
            outputs: ["{d}/a.out"]
            allow_failure: true
            steps {{
                $ ms_no_such_binary_zzz
            }}
        }}
        stage b {{
            inputs: [a.outputs]
            steps {{ write "{d}/b" content: "x" }}
        }}
        "#
    );

    run_jobs(&src, &dir, None, 4).expect("allow_failure should keep the pipeline green");

    assert!(exists(&dir, "b"), "dependent of an allow_failure stage must still run");
    assert!(exists(&dir, "success"), "on_success should run");
    assert!(!exists(&dir, "failure"), "on_failure must not run");

    let _ = std::fs::remove_dir_all(&dir);
}

// ── Phase 26: cancellation & cache consistency ───────────────────────────────────

fn run_cancellable(src: &str, dir: &Path, cancel: &CancelToken) -> mainstage_core::Result<()> {
    let program = parse(&Source::from_str("test.ms", src)).expect("parse should succeed");
    let analysis = analyze(&program).expect("analysis should succeed");
    let ctx = eval_program(&program, dir).expect("eval should succeed");
    run_pipeline_cancellable(&program, None, &ctx, &analysis, &NoopReporter, 4, cancel)
}

#[test]
fn cancel_before_run_launches_no_stages() {
    let dir = unique_dir("cancel_pre");
    let d = dir.display();
    // A token cancelled before the run starts: no stage should execute, the run reports
    // cancellation, and the pipeline's on_success hook must not fire.
    let src = format!(
        r#"
        default pipeline build {{
            stages: [a, b]
            on_success {{ write "{d}/success" content: "ok" }}
        }}
        stage a {{ steps {{ write "{d}/a" content: "x" }} }}
        stage b {{ steps {{ write "{d}/b" content: "x" }} }}
        "#
    );

    let cancel = CancelToken::new();
    cancel.cancel();
    let result = run_cancellable(&src, &dir, &cancel);

    assert!(result.is_err(), "a cancelled run must report an error");
    assert!(!exists(&dir, "a"), "no stage should run after pre-cancellation");
    assert!(!exists(&dir, "b"), "no stage should run after pre-cancellation");
    assert!(!exists(&dir, "success"), "on_success must not run for a cancelled pipeline");
    // The cache file exists and is whole (the atomic save leaves no partial write).
    let cache_file = dir.join(".mainstage").join("cache.json");
    assert!(cache_file.exists(), "cache must be saved even on cancellation");
    let text = std::fs::read_to_string(&cache_file).unwrap();
    assert!(text.contains("stages"), "cache.json should be a complete document");
    // No temporary cache files should linger after the atomic rename.
    let leftover = std::fs::read_dir(dir.join(".mainstage"))
        .unwrap()
        .filter_map(|e| e.ok())
        .any(|e| e.file_name().to_string_lossy().ends_with(".tmp"));
    assert!(!leftover, "atomic save must not leave .tmp files behind");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cancel_mid_run_lets_inflight_finish_and_stops_new_work() {
    // A dependency chain a → b → c forces a deterministic order: only `a` is ready first.
    // `a` writes its marker then sleeps; cancellation lands during that sleep. `a` runs to
    // completion, but its dependents `b` and `c` — ready only after `a` — never start.
    if cfg!(windows) {
        return; // relies on a `sleep` binary
    }
    let dir = unique_dir("cancel_mid");
    let d = dir.display();
    let src = format!(
        r#"
        default pipeline build {{
            stages: [a, b, c]
        }}
        stage a {{
            outputs: ["{d}/a.out"]
            steps {{
                write "{d}/a.out" content: "x"
                $ sleep 0.5
            }}
        }}
        stage b {{
            inputs:  [a.outputs]
            outputs: ["{d}/b.out"]
            steps {{ write "{d}/b.out" content: "x" }}
        }}
        stage c {{
            inputs: [b.outputs]
            steps {{ write "{d}/c.out" content: "x" }}
        }}
        "#
    );

    let program = parse(&Source::from_str("test.ms", &src)).expect("parse");
    let analysis = analyze(&program).expect("analyze");
    let ctx = eval_program(&program, &dir).expect("eval");

    let cancel = CancelToken::new();
    let canceller = cancel.clone();
    let handle = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(150));
        canceller.cancel();
    });

    let result =
        run_pipeline_cancellable(&program, None, &ctx, &analysis, &NoopReporter, 4, &cancel);
    handle.join().unwrap();

    assert!(result.is_err(), "a cancelled run reports an error");
    assert!(exists(&dir, "a.out"), "the in-flight stage should finish");
    assert!(!exists(&dir, "b.out"), "a dependent must not start after cancellation");
    assert!(!exists(&dir, "c.out"), "a transitive dependent must not start after cancellation");

    let _ = std::fs::remove_dir_all(&dir);
}

// ── Phase 27: dry-run planning ───────────────────────────────────────────────────

#[test]
fn plan_reports_run_then_skip_and_groups_waves() {
    // A two-stage chain gen → bundle, where gen reads a real input file. Before any run
    // the plan marks both as "run"; after a run, gen still runs (its input is unchanged
    // but so is its output, so it skips) and bundle skips too. The waves reflect the
    // dependency: [gen], [bundle].
    let dir = unique_dir("plan");
    let d = dir.display();
    std::fs::write(dir.join("src.txt"), "seed").unwrap();
    let src = format!(
        r#"
        default pipeline build {{
            stages: [gen, bundle]
        }}
        stage gen {{
            inputs:  ["{d}/src.txt"]
            outputs: ["{d}/gen.out"]
            steps {{ write "{d}/gen.out" content: "x" }}
        }}
        stage bundle {{
            inputs:  [gen.outputs]
            outputs: ["{d}/bundle.out"]
            steps {{ write "{d}/bundle.out" content: "y" }}
        }}
        "#
    );

    let program = parse(&Source::from_str("test.ms", &src)).expect("parse");
    let analysis = analyze(&program).expect("analyze");
    let ctx = eval_program(&program, &dir).expect("eval");

    // Before running, with no cache, every stage would run.
    let plan = plan_pipeline(&program, None, &ctx, &analysis).expect("plan");
    assert_eq!(plan.pipeline, "build");
    assert_eq!(plan.waves.len(), 2, "gen and bundle form two dependency waves");
    assert_eq!(plan.waves[0][0].name, "gen");
    assert_eq!(plan.waves[1][0].name, "bundle");
    assert!(plan.stages().all(|s| s.status == PlanStatus::Run), "no cache → all run");
    // gen's resolved inputs are surfaced for `watch`.
    assert!(plan.waves[0][0].inputs.iter().any(|p| p.ends_with("src.txt")));

    // Run the pipeline, then re-plan: unchanged inputs and present outputs make both skip.
    run_pipeline(&program, None, &ctx, &analysis).expect("run");
    let plan = plan_pipeline(&program, None, &ctx, &analysis).expect("re-plan");
    assert!(
        plan.stages().all(|s| s.status == PlanStatus::Skip),
        "after a clean run every stage should plan to skip"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pipeline_input_paths_collects_and_dedups() {
    // Two stages reading the same input file; the watch path set lists it once.
    let dir = unique_dir("inputs");
    let d = dir.display();
    std::fs::write(dir.join("shared.txt"), "data").unwrap();
    let src = format!(
        r#"
        default pipeline build {{
            stages: [a, b]
        }}
        stage a {{ inputs: ["{d}/shared.txt"] steps {{ write "{d}/a" content: "x" }} }}
        stage b {{ inputs: ["{d}/shared.txt"] steps {{ write "{d}/b" content: "y" }} }}
        "#
    );

    let program = parse(&Source::from_str("test.ms", &src)).expect("parse");
    let analysis = analyze(&program).expect("analyze");
    let ctx = eval_program(&program, &dir).expect("eval");

    let paths =
        mainstage_core::pipeline_input_paths(&program, None, &ctx, &analysis).expect("inputs");
    let shared: Vec<_> = paths.iter().filter(|p| p.ends_with("shared.txt")).collect();
    assert_eq!(shared.len(), 1, "the shared input is de-duplicated across stages");

    let _ = std::fs::remove_dir_all(&dir);
}
