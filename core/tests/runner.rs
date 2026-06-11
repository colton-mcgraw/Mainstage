//! Phase 6 integration tests — pipeline runner and failure handling.
//!
//! Drive a full `parse → analyze → eval_program → run_pipeline` flow and observe
//! side effects (marker files written by `write` steps) to verify sequential
//! execution, failure propagation through the DAG, `allow_failure`, and the
//! pipeline-level `on_success` / `on_failure` hooks with `failed_stage` binding.

use std::path::{Path, PathBuf};

use mainstage_core::{Source, analyze, eval_program, parse, run_pipeline};

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
