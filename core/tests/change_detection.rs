//! Phase 7 integration tests — change detection.
//!
//! Run a pipeline twice over a real temp directory and observe (via a recording
//! [`Reporter`]) whether a stage runs or is skipped: unchanged inputs skip, changed
//! inputs re-run, a missing output re-runs, and `clean` forces a full rebuild.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use mainstage_core::{
    Reporter, Source, analyze, cache, eval_program, parse, run_pipeline_reported,
};

/// Records which stages ran vs. were skipped during a single pipeline run. Uses a
/// `Mutex` so the recorder is `Sync` and shareable across the runner's worker threads.
#[derive(Default)]
struct Recorder {
    ran: Mutex<Vec<String>>,
    skipped: Mutex<Vec<String>>,
}

impl Reporter for Recorder {
    fn stage_start(&self, _out: &mut dyn Write, stage: &str) {
        self.ran.lock().unwrap().push(stage.to_string());
    }
    fn stage_skipped(&self, _out: &mut dyn Write, stage: &str) {
        self.skipped.lock().unwrap().push(stage.to_string());
    }
}

impl Recorder {
    fn ran(&self, stage: &str) -> bool {
        self.ran.lock().unwrap().iter().any(|s| s == stage)
    }
    fn skipped(&self, stage: &str) -> bool {
        self.skipped.lock().unwrap().iter().any(|s| s == stage)
    }
}

fn unique_dir(tag: &str) -> PathBuf {
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("ms_cd_{tag}_{nanos}"));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    dir
}

/// A pipeline whose single stage globs `src/*.txt` as inputs and writes one output.
/// Inputs (`src/`) and the output (`out/`) live in separate subtrees so the output
/// is never re-globbed as an input.
fn script(dir: &Path) -> String {
    let d = dir.display();
    format!(
        r#"
        default pipeline build {{
            stages: [gen]
        }}
        stage gen {{
            inputs:  glob("src/*.txt")
            outputs: ["{d}/out/result.bin"]
            steps {{
                write "{d}/out/result.bin" content: "built"
            }}
        }}
        "#
    )
}

/// Run the pipeline once against `dir`, returning the recorder for assertions.
fn run_once(dir: &Path) -> Recorder {
    let src = script(dir);
    let program = parse(&Source::from_str("test.ms", &src)).expect("parse");
    let analysis = analyze(&program).expect("analyze");
    let ctx = eval_program(&program, dir).expect("eval");
    let recorder = Recorder::default();
    run_pipeline_reported(&program, None, &ctx, &analysis, &recorder).expect("run");
    recorder
}

#[test]
fn unchanged_inputs_skip_on_second_run() {
    let dir = unique_dir("skip");
    std::fs::write(dir.join("src/a.txt"), "alpha").unwrap();

    let first = run_once(&dir);
    assert!(first.ran("gen"), "first run must execute the stage");
    assert!(dir.join("out/result.bin").exists());

    let second = run_once(&dir);
    assert!(second.skipped("gen"), "unchanged inputs must skip the stage");
    assert!(!second.ran("gen"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn changed_input_triggers_rerun() {
    let dir = unique_dir("change");
    std::fs::write(dir.join("src/a.txt"), "alpha").unwrap();
    run_once(&dir);

    // Mutate the input — the digest must change and the stage must re-run.
    std::fs::write(dir.join("src/a.txt"), "BETA").unwrap();
    let second = run_once(&dir);
    assert!(second.ran("gen"), "a changed input must re-run the stage");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn new_input_file_triggers_rerun() {
    let dir = unique_dir("newfile");
    std::fs::write(dir.join("src/a.txt"), "alpha").unwrap();
    run_once(&dir);

    // Adding a file to the input set changes the digest.
    std::fs::write(dir.join("src/b.txt"), "beta").unwrap();
    let second = run_once(&dir);
    assert!(second.ran("gen"), "a new input file must re-run the stage");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_output_triggers_rerun() {
    let dir = unique_dir("missingout");
    std::fs::write(dir.join("src/a.txt"), "alpha").unwrap();
    run_once(&dir);

    // Delete the declared output — even with unchanged inputs, the stage must re-run.
    std::fs::remove_file(dir.join("out/result.bin")).unwrap();
    let second = run_once(&dir);
    assert!(second.ran("gen"), "a missing output must re-run the stage");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clean_forces_full_rebuild() {
    let dir = unique_dir("clean");
    std::fs::write(dir.join("src/a.txt"), "alpha").unwrap();
    run_once(&dir);

    // After clean, the cache is gone and the stage must run again.
    cache::clean(&dir).unwrap();
    assert!(!dir.join(".mainstage").exists());
    let second = run_once(&dir);
    assert!(second.ran("gen"), "after clean the stage must re-run");

    let _ = std::fs::remove_dir_all(&dir);
}

// ── Phase 35: always_run / run_once ──────────────────────────────────────────────

/// Run an arbitrary pipeline script once against `dir`, returning the recorder.
fn run_src(src: &str, dir: &Path) -> Recorder {
    let program = parse(&Source::from_str("test.ms", src)).expect("parse");
    let analysis = analyze(&program).expect("analyze");
    let ctx = eval_program(&program, dir).expect("eval");
    let recorder = Recorder::default();
    run_pipeline_reported(&program, None, &ctx, &analysis, &recorder).expect("run");
    recorder
}

#[test]
fn run_once_stage_runs_then_skips() {
    let dir = unique_dir("run_once");
    let d = dir.display();
    // No inputs and no outputs: without run_once this would run every time (Phase 7).
    let src = format!(
        r#"
        default pipeline p {{ stages: [setup] }}
        stage setup {{
            run_once: true
            steps {{ write "{d}/marker" content: "x" }}
        }}
        "#
    );

    assert!(run_src(&src, &dir).ran("setup"), "first run executes the run_once stage");

    let second = run_src(&src, &dir);
    assert!(second.skipped("setup"), "run_once stage is skipped on re-run");
    assert!(!second.ran("setup"));

    // Clearing the cache drops the stamp, so it runs again.
    cache::clean(&dir).unwrap();
    assert!(run_src(&src, &dir).ran("setup"), "after clean the run_once stamp is gone");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn no_inputs_stage_without_run_once_always_runs() {
    let dir = unique_dir("no_once");
    let d = dir.display();
    let src = format!(
        r#"
        default pipeline p {{ stages: [setup] }}
        stage setup {{
            steps {{ write "{d}/marker" content: "x" }}
        }}
        "#
    );

    assert!(run_src(&src, &dir).ran("setup"));
    assert!(run_src(&src, &dir).ran("setup"), "a plain no-inputs stage runs every time");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn always_run_stage_never_skips() {
    let dir = unique_dir("always");
    std::fs::write(dir.join("src/a.txt"), "alpha").unwrap();
    let d = dir.display();
    // Unchanged inputs plus a present output would normally skip on the second run;
    // always_run forces execution regardless.
    let src = format!(
        r#"
        default pipeline p {{ stages: [act] }}
        stage act {{
            inputs:  glob("src/*.txt")
            outputs: ["{d}/out/result.bin"]
            always_run: true
            steps {{ write "{d}/out/result.bin" content: "built" }}
        }}
        "#
    );

    assert!(run_src(&src, &dir).ran("act"));
    let second = run_src(&src, &dir);
    assert!(second.ran("act"), "always_run forces re-run despite unchanged inputs");
    assert!(!second.skipped("act"));

    let _ = std::fs::remove_dir_all(&dir);
}
