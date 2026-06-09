//! Phase 7 integration tests — change detection.
//!
//! Run a pipeline twice over a real temp directory and observe (via a recording
//! [`Reporter`]) whether a stage runs or is skipped: unchanged inputs skip, changed
//! inputs re-run, a missing output re-runs, and `clean` forces a full rebuild.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use mainstage_core::{analyze, cache, eval_program, parse, run_pipeline_reported, Reporter, Source};

/// Records which stages ran vs. were skipped during a single pipeline run.
#[derive(Default)]
struct Recorder {
    ran: RefCell<Vec<String>>,
    skipped: RefCell<Vec<String>>,
}

impl Reporter for Recorder {
    fn stage_start(&self, stage: &str) {
        self.ran.borrow_mut().push(stage.to_string());
    }
    fn stage_skipped(&self, stage: &str) {
        self.skipped.borrow_mut().push(stage.to_string());
    }
}

impl Recorder {
    fn ran(&self, stage: &str) -> bool {
        self.ran.borrow().iter().any(|s| s == stage)
    }
    fn skipped(&self, stage: &str) -> bool {
        self.skipped.borrow().iter().any(|s| s == stage)
    }
}

fn unique_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
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
