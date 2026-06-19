//! Phase 6 integration tests — pipeline runner and failure handling.
//!
//! Drive a full `parse → analyze → eval_program → run_pipeline` flow and observe
//! side effects (marker files written by `write` steps) to verify sequential
//! execution, failure propagation through the DAG, `allow_failure`, and the
//! pipeline-level `on_success` / `on_failure` hooks with `failed_stage` binding.

use std::path::{Path, PathBuf};

use std::sync::Mutex;

use mainstage_core::{
    AssertionResult, CancelToken, NoopReporter, PlanStatus, Reporter, Source, analyze,
    eval_program, parse, plan_pipeline, run_pipeline, run_pipeline_cancellable,
    run_pipeline_reported_jobs,
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

// ── Build matrix (Phase 37) ───────────────────────────────────────────────────────

/// Parse, lower the `matrix` blocks, analyze, evaluate, and run the named pipeline —
/// mirroring the CLI's `prepare`, which expands matrices before everything else.
fn run_lowered(src: &str, dir: &Path, pipeline: Option<&str>) -> mainstage_core::Result<()> {
    let parsed = parse(&Source::from_str("test.ms", src)).expect("parse should succeed");
    let program = mainstage_core::expand_matrix(&parsed).expect("matrix lowering should succeed");
    let analysis = analyze(&program).expect("analysis should succeed");
    let ctx = eval_program(&program, dir).expect("eval should succeed");
    run_pipeline(&program, pipeline, &ctx, &analysis)
}

#[test]
fn matrix_stage_runs_each_variant_with_its_binding() {
    let dir = unique_dir("matrix");
    let d = dir.display();
    // One authored stage expands to two; the `${arch}` matrix variable selects the
    // output path per variant. Listing the base name in the pipeline runs both.
    let src = format!(
        r#"
        default pipeline build {{
            stages: [bootloader]
        }}
        stage bootloader {{
            matrix {{ arch: ["x64", "arm64"] }}
            steps {{ write "{d}/boot-${{arch}}.efi" content: "${{arch}}" }}
        }}
        "#
    );

    run_lowered(&src, &dir, None).expect("matrixed pipeline should succeed");

    assert!(exists(&dir, "boot-x64.efi"), "the x64 variant should have run");
    assert!(exists(&dir, "boot-arm64.efi"), "the arm64 variant should have run");
    // The matrix value is bound, not literal — the file content is the resolved value.
    assert_eq!(std::fs::read_to_string(dir.join("boot-x64.efi")).unwrap(), "x64");
    assert_eq!(std::fs::read_to_string(dir.join("boot-arm64.efi")).unwrap(), "arm64");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn matrix_variant_outputs_feed_a_downstream_stage() {
    let dir = unique_dir("matrix_deps");
    let d = dir.display();
    // `package` depends on the matrixed `bootloader` by its base name, so it runs after
    // every variant. The variants write their outputs; `package` records that it ran.
    let src = format!(
        r#"
        default pipeline build {{
            stages: [bootloader, package]
        }}
        stage bootloader {{
            matrix {{ arch: ["x64", "arm64"] }}
            outputs: ["{d}/boot-${{arch}}.efi"]
            steps {{ write "{d}/boot-${{arch}}.efi" content: "ok" }}
        }}
        stage package {{
            depends_on: [bootloader]
            steps {{ write "{d}/packaged" content: "done" }}
        }}
        "#
    );

    run_lowered(&src, &dir, None).expect("matrixed pipeline should succeed");

    assert!(exists(&dir, "boot-x64.efi"));
    assert!(exists(&dir, "boot-arm64.efi"));
    assert!(exists(&dir, "packaged"), "the downstream stage should have run after the variants");

    let _ = std::fs::remove_dir_all(&dir);
}

// ── Per-file incremental change detection (Phase 38) ──────────────────────────────

#[test]
fn for_loop_reruns_only_changed_input_files() {
    let dir = unique_dir("incremental");
    let d = dir.display();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/a.txt"), "1").unwrap();
    std::fs::write(dir.join("src/b.txt"), "2").unwrap();

    // A per-file compile loop: each input writes a corresponding object file. The output
    // directory is the declared output, so the incremental gate (outputs present) holds.
    let src = format!(
        r#"
        default pipeline build {{ stages: [compile] }}
        stage compile {{
            inputs: glob("{d}/src/*.txt")
            outputs: ["{d}/obj"]
            steps {{
                for f in inputs {{
                    write "{d}/obj/${{f.stem}}.o" content: "built"
                }}
            }}
        }}
        "#
    );

    // First run builds both objects.
    run(&src, &dir, None).expect("first run should succeed");
    assert!(exists(&dir, "obj/a.o"));
    assert!(exists(&dir, "obj/b.o"));

    // Overwrite both objects with sentinels, then change only a.txt. A second run should
    // re-run a's iteration (overwriting its sentinel) but skip b's (sentinel survives).
    std::fs::write(dir.join("obj/a.o"), "SENTINEL-A").unwrap();
    std::fs::write(dir.join("obj/b.o"), "SENTINEL-B").unwrap();
    std::fs::write(dir.join("src/a.txt"), "1-changed").unwrap();

    run(&src, &dir, None).expect("second run should succeed");

    assert_eq!(
        std::fs::read_to_string(dir.join("obj/a.o")).unwrap(),
        "built",
        "the changed input's iteration must re-run"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("obj/b.o")).unwrap(),
        "SENTINEL-B",
        "the unchanged input's iteration must be skipped"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn incremental_skip_disabled_when_outputs_missing() {
    let dir = unique_dir("incremental_no_out");
    let d = dir.display();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/a.txt"), "1").unwrap();
    std::fs::write(dir.join("src/b.txt"), "2").unwrap();

    let src = format!(
        r#"
        default pipeline build {{ stages: [compile] }}
        stage compile {{
            inputs: glob("{d}/src/*.txt")
            outputs: ["{d}/obj"]
            steps {{
                for f in inputs {{
                    write "{d}/obj/${{f.stem}}.o" content: "built"
                }}
            }}
        }}
        "#
    );

    run(&src, &dir, None).expect("first run should succeed");

    // Remove the whole output directory: the incremental gate fails, so a changed-input
    // run rebuilds every object, restoring b.o too.
    std::fs::remove_dir_all(dir.join("obj")).unwrap();
    std::fs::write(dir.join("src/a.txt"), "1-changed").unwrap();

    run(&src, &dir, None).expect("second run should succeed");
    assert!(exists(&dir, "obj/a.o"), "changed input rebuilt");
    assert!(exists(&dir, "obj/b.o"), "missing outputs force a full rebuild of unchanged inputs");

    let _ = std::fs::remove_dir_all(&dir);
}

// ── Test harness (Phase 39) ───────────────────────────────────────────────────────

/// One test stage's captured tally: `(stage, passed, failed, descriptions-of-failures)`.
type StageTally = (String, usize, usize, Vec<String>);

/// A reporter that records each test stage's assertion outcomes, so tests can inspect the
/// tally the runner produced.
#[derive(Default)]
struct TestCapture {
    stages: Mutex<Vec<StageTally>>,
}

impl Reporter for TestCapture {
    fn stage_tests(&self, _out: &mut dyn std::io::Write, stage: &str, results: &[AssertionResult]) {
        let passed = results.iter().filter(|r| r.passed).count();
        let failed = results.len() - passed;
        let failures =
            results.iter().filter(|r| !r.passed).map(|r| r.description.clone()).collect();
        self.stages.lock().unwrap().push((stage.to_string(), passed, failed, failures));
    }
}

/// Run a script with a [`TestCapture`] reporter (sequentially, for deterministic output).
fn run_with_capture(src: &str, dir: &Path) -> (mainstage_core::Result<()>, TestCapture) {
    let program = parse(&Source::from_str("test.ms", src)).expect("parse should succeed");
    let analysis = analyze(&program).expect("analysis should succeed");
    let ctx = eval_program(&program, dir).expect("eval should succeed");
    let reporter = TestCapture::default();
    let result = run_pipeline_reported_jobs(&program, None, &ctx, &analysis, &reporter, 1);
    (result, reporter)
}

#[test]
fn test_stage_all_assertions_pass_pipeline_succeeds() {
    let dir = unique_dir("test_pass");
    let src = r#"
        project { name: "demo" }
        default pipeline check { stages: [unit] }
        stage unit {
            test: true
            steps {
                assert "${project.name}" equals "demo"
                assert "release-${project.name}" contains "demo"
            }
        }
    "#;

    let (result, cap) = run_with_capture(src, &dir);
    result.expect("a test stage whose assertions all pass should succeed");

    let stages = cap.stages.lock().unwrap();
    assert_eq!(stages.len(), 1, "the test stage reported its tally once");
    assert_eq!(stages[0], ("unit".to_string(), 2, 0, Vec::new()));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_stage_failing_assertion_fails_pipeline_and_runs_remaining_assertions() {
    let dir = unique_dir("test_fail");
    // The first assertion fails; the harness must still run the second (the stage does not
    // short-circuit), tally one pass and one fail, and report overall failure.
    let src = r#"
        project { name: "demo" }
        default pipeline check { stages: [unit] }
        stage unit {
            test: true
            steps {
                assert "${project.name}" equals "wrong"
                assert "${project.name}" equals "demo"
            }
        }
    "#;

    let (result, cap) = run_with_capture(src, &dir);
    assert!(result.is_err(), "a failed assertion must fail the pipeline");

    let stages = cap.stages.lock().unwrap();
    assert_eq!(stages[0].1, 1, "the second assertion still ran and passed");
    assert_eq!(stages[0].2, 1, "the first assertion failed");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_stage_is_never_cached() {
    // A test stage with unchanged inputs would normally be skipped, but a test stage must
    // run its assertions every time. Plan it twice across a run and confirm it stays "run".
    let dir = unique_dir("test_nocache");
    let d = dir.display();
    std::fs::write(dir.join("in.txt"), "seed").unwrap();
    let src = format!(
        r#"
        default pipeline check {{ stages: [unit] }}
        stage unit {{
            test: true
            inputs: ["{d}/in.txt"]
            steps {{ assert "x" equals "x" }}
        }}
        "#
    );

    let program = parse(&Source::from_str("test.ms", &src)).expect("parse");
    let analysis = analyze(&program).expect("analyze");
    let ctx = eval_program(&program, &dir).expect("eval");

    run_pipeline(&program, None, &ctx, &analysis).expect("first run");
    let plan = plan_pipeline(&program, None, &ctx, &analysis).expect("re-plan");
    assert!(
        plan.stages().all(|s| s.status == PlanStatus::Run),
        "a test stage is never cached, so it always plans to run"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn committed_testing_example_runs_green() {
    // The repo-root `tests/testing.ms` example must parse, analyze, evaluate, and run with
    // all its assertions passing.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("tests");
    let source = Source::from_file(dir.join("testing.ms")).expect("example file should exist");
    let program = parse(&source).expect("example should parse");
    let analysis = analyze(&program).expect("example should analyze");
    let ctx = eval_program(&program, &dir).expect("example should evaluate");

    let reporter = TestCapture::default();
    run_pipeline_reported_jobs(&program, None, &ctx, &analysis, &reporter, 1)
        .expect("the committed testing example should run green");
    assert_eq!(reporter.stages.lock().unwrap()[0].2, 0, "no assertion in the example should fail");
}

// `expect` exercises real commands; gate it on unix where `true` / `false` / `echo` exist.
#[cfg(unix)]
#[test]
fn expect_checks_exit_status_and_output() {
    let dir = unique_dir("test_expect");
    let src = r#"
        default pipeline check { stages: [unit] }
        stage unit {
            test: true
            steps {
                expect ok $ true
                expect fails $ false
                expect output contains "hello" $ echo hello world
                expect output equals "hi" $ echo hi
            }
        }
    "#;

    let (result, cap) = run_with_capture(src, &dir);
    result.expect("all command expectations should hold");
    let stages = cap.stages.lock().unwrap();
    assert_eq!(stages[0], ("unit".to_string(), 4, 0, Vec::new()));

    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn expect_records_failures() {
    let dir = unique_dir("test_expect_fail");
    let src = r#"
        default pipeline check { stages: [unit] }
        stage unit {
            test: true
            steps {
                expect ok $ false
                expect output contains "missing" $ echo present
            }
        }
    "#;

    let (result, cap) = run_with_capture(src, &dir);
    assert!(result.is_err(), "failed command expectations must fail the pipeline");
    assert_eq!(cap.stages.lock().unwrap()[0].2, 2, "both expectations failed");

    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn expect_output_contains_stops_early_on_timeout() {
    // A never-exiting process that prints a marker: `expect output contains` with a timeout
    // must find the marker and pass without waiting out the full timeout.
    let dir = unique_dir("test_expect_timeout");
    let src = r#"
        default pipeline check { stages: [unit] }
        stage unit {
            test: true
            steps {
                expect output contains "READY" timeout 5 $ sh -c "echo READY; sleep 30"
            }
        }
    "#;

    let start = std::time::Instant::now();
    let (result, cap) = run_with_capture(src, &dir);
    result.expect("the marker appears, so the expectation passes");
    assert_eq!(cap.stages.lock().unwrap()[0].1, 1, "the output-contains check passed");
    assert!(start.elapsed().as_secs() < 25, "early-stop must not wait out the 30s sleep");

    let _ = std::fs::remove_dir_all(&dir);
}

// An `expect` / `assert` outside a test stage is a hard assertion: a failure fails the stage.
#[test]
fn assert_outside_test_stage_hard_fails() {
    let dir = unique_dir("assert_hard");
    let d = dir.display();
    let src = format!(
        r#"
        default pipeline build {{
            stages: [a]
            on_failure {{ write "{d}/failed" content: "x" }}
        }}
        stage a {{
            steps {{ assert "x" equals "y" }}
        }}
        "#
    );

    let result = run(&src, &dir, None);
    assert!(result.is_err(), "a failed assert in an ordinary stage fails the stage");
    assert!(exists(&dir, "failed"), "the stage's failure triggers pipeline on_failure");

    let _ = std::fs::remove_dir_all(&dir);
}

// ── Diagnostic & control-flow steps (Phase 43) ────────────────────────────────────

/// Captures the interpolated messages emitted by `log` steps so a test can inspect them.
#[derive(Default)]
struct LogCapture {
    messages: Mutex<Vec<String>>,
}

impl Reporter for LogCapture {
    fn step_log(&self, _out: &mut dyn std::io::Write, message: &str) {
        // Record only (don't echo to the captured `out`), so the test stays quiet on stdout;
        // routing the rendered bytes into the per-stage sink is covered by an executor test.
        self.messages.lock().unwrap().push(message.to_string());
    }
}

/// Run the default pipeline with a [`LogCapture`] installed on the context, returning the
/// run result and the captured `log` messages in emission order.
fn run_capturing_logs(src: &str, dir: &Path) -> (mainstage_core::Result<()>, Vec<String>) {
    let program = parse(&Source::from_str("test.ms", src)).expect("parse should succeed");
    let analysis = analyze(&program).expect("analysis should succeed");
    let base = eval_program(&program, dir).expect("eval should succeed");
    let cap = std::sync::Arc::new(LogCapture::default());
    let ctx = base.with_reporter(mainstage_core::ReporterHandle(cap.clone()));
    let result = run_pipeline(&program, None, &ctx, &analysis);
    let messages = cap.messages.lock().unwrap().clone();
    (result, messages)
}

#[test]
fn log_routes_interpolated_messages_through_the_reporter() {
    let dir = unique_dir("log_route");
    let src = r#"
        project { name: "demo" }
        default pipeline build { stages: [setup] }
        stage setup {
            always_run: true
            steps {
                log "building ${project.name}"
                log "platform ${platform}"
            }
        }
    "#;

    let (result, messages) = run_capturing_logs(src, &dir);
    result.expect("a stage with only log steps should succeed");
    assert_eq!(messages.first().map(String::as_str), Some("building demo"));
    assert!(messages.iter().any(|m| m.starts_with("platform ")), "got: {messages:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fail_fails_the_stage_and_runs_on_failure() {
    let dir = unique_dir("fail_onfail");
    let d = dir.display();
    let src = format!(
        r#"
        default pipeline build {{ stages: [check] }}
        stage check {{
            always_run: true
            steps {{
                fail "deliberate stop"
            }}
            on_failure {{ write "{d}/handled" content: "x" }}
        }}
        "#
    );

    let result = run(&src, &dir, None);
    assert!(result.is_err(), "a `fail` step must fail the stage and pipeline");
    assert!(exists(&dir, "handled"), "the stage's on_failure block fires on a `fail`");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fail_inside_try_does_not_propagate() {
    let dir = unique_dir("fail_try");
    let d = dir.display();
    let src = format!(
        r#"
        default pipeline build {{ stages: [setup] }}
        stage setup {{
            always_run: true
            steps {{
                try {{
                    fail "optional refresh failed"
                }}
                write "{d}/after" content: "x"
            }}
        }}
        "#
    );

    run(&src, &dir, None).expect("a `fail` inside `try` is swallowed, so the stage succeeds");
    assert!(exists(&dir, "after"), "execution continues after a swallowed `fail`");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn committed_diagnostics_example_runs_green() {
    // The repo-root `tests/diagnostics.ms` example must parse, analyze, evaluate, and run:
    // its only `fail` sits inside a `try`, so the pipeline succeeds.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("tests");
    let source = Source::from_file(dir.join("diagnostics.ms")).expect("example file should exist");
    let program = parse(&source).expect("example should parse");
    let analysis = analyze(&program).expect("example should analyze");
    let ctx = eval_program(&program, &dir).expect("example should evaluate");
    run_pipeline(&program, None, &ctx, &analysis)
        .expect("the committed diagnostics example should run green");
}

#[test]
fn fail_inside_if_fails_only_when_the_branch_runs() {
    let dir = unique_dir("fail_if");
    // The condition is false (no env var), so the `fail` branch is skipped and the stage
    // succeeds — proving `fail` participates in conditional control flow.
    let src = r#"
        default pipeline build { stages: [gate] }
        stage gate {
            always_run: true
            steps {
                if env("MS_FORCE_FAIL_PHASE43") {
                    fail "forced"
                }
            }
        }
    "#;

    run(src, &dir, None).expect("the unselected `fail` branch does not run, so the stage passes");

    let _ = std::fs::remove_dir_all(&dir);
}
