//! Phase 26 — stress tests under the parallel scheduler.
//!
//! Exercise the runner at the shapes Goal 4 cares about — large filesets, deep
//! dependency chains, and wide fan-out — with more than one worker, asserting the
//! pipeline completes correctly (every expected output produced) and without deadlock,
//! panic, or lost work. All generated stages use only `write` steps, so the programs
//! are safe to actually execute.

use std::path::{Path, PathBuf};

use mainstage_core::{Source, analyze, eval_program, parse, run_pipeline_reported_jobs};

fn unique_dir(tag: &str) -> PathBuf {
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("ms_stress_{tag}_{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Parse, analyze, evaluate, and run `src` in `dir` with `jobs` workers.
fn run(src: &str, dir: &Path, jobs: usize) -> mainstage_core::Result<()> {
    let program = parse(&Source::from_str("test.ms", src)).expect("parse should succeed");
    let analysis = analyze(&program).expect("analysis should succeed");
    let ctx = eval_program(&program, dir).expect("eval should succeed");
    run_pipeline_reported_jobs(&program, None, &ctx, &analysis, &mainstage_core::NoopReporter, jobs)
}

fn jobs() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).max(4)
}

#[test]
fn wide_fan_out_completes() {
    // One source stage feeding a wide layer of independent dependents — maximum
    // concurrency pressure on the scheduler and on shared output publication.
    let dir = unique_dir("wide");
    let d = dir.display().to_string().replace('\\', "/");
    let width = 200;

    let mut src = String::new();
    src.push_str("default pipeline build {\n    stages: [root");
    for i in 0..width {
        src.push_str(&format!(", w{i}"));
    }
    src.push_str("]\n}\n");
    src.push_str(&format!(
        "stage root {{\n    outputs: [\"{d}/out/root.bin\"]\n    \
         steps {{ write \"{d}/out/root.bin\" content: \"r\" }}\n}}\n"
    ));
    for i in 0..width {
        src.push_str(&format!(
            "stage w{i} {{\n    inputs: [root.outputs]\n    outputs: [\"{d}/out/w{i}.bin\"]\n    \
             steps {{ write \"{d}/out/w{i}.bin\" content: \"w\" }}\n}}\n"
        ));
    }

    run(&src, &dir, jobs()).expect("wide fan-out pipeline should succeed");

    assert!(dir.join("out/root.bin").exists());
    for i in 0..width {
        assert!(dir.join(format!("out/w{i}.bin")).exists(), "fan-out stage w{i} must run");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn deep_chain_completes_in_order() {
    // A long linear dependency chain: each stage consumes the previous stage's output, so
    // the scheduler must serialize them correctly even with many workers available.
    let dir = unique_dir("deep");
    let d = dir.display().to_string().replace('\\', "/");
    let depth = 150;

    let mut src = String::new();
    src.push_str("default pipeline build {\n    stages: [s0");
    for i in 1..depth {
        src.push_str(&format!(", s{i}"));
    }
    src.push_str("]\n}\n");
    src.push_str(&format!(
        "stage s0 {{\n    outputs: [\"{d}/out/s0.bin\"]\n    \
         steps {{ write \"{d}/out/s0.bin\" content: \"0\" }}\n}}\n"
    ));
    for i in 1..depth {
        src.push_str(&format!(
            "stage s{i} {{\n    inputs: [s{}.outputs]\n    outputs: [\"{d}/out/s{i}.bin\"]\n    \
             steps {{ write \"{d}/out/s{i}.bin\" content: \"{i}\" }}\n}}\n",
            i - 1
        ));
    }

    run(&src, &dir, jobs()).expect("deep chain pipeline should succeed");

    for i in 0..depth {
        assert!(dir.join(format!("out/s{i}.bin")).exists(), "chain stage s{i} must run");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn large_fileset_hashes_and_skips() {
    // A stage globbing a large input set: the first run hashes them all; a second run hits
    // the change-detection fast path and skips. Verifies the scheduler and fingerprinting
    // hold up on many files.
    let dir = unique_dir("fileset");
    let d = dir.display().to_string().replace('\\', "/");
    let n_files = 1_000;

    let src_dir = dir.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    for i in 0..n_files {
        std::fs::write(src_dir.join(format!("f{i}.txt")), format!("file {i}\n")).unwrap();
    }

    let src = format!(
        r#"
        default pipeline build {{
            stages: [gen]
        }}
        stage gen {{
            inputs:  glob("{d}/src/*.txt")
            outputs: ["{d}/out/result.bin"]
            steps {{ write "{d}/out/result.bin" content: "done" }}
        }}
        "#
    );

    run(&src, &dir, jobs()).expect("first run over a large fileset should succeed");
    assert!(dir.join("out/result.bin").exists());
    // A second run with unchanged inputs must still succeed (and take the skip path).
    run(&src, &dir, jobs()).expect("second run over an unchanged large fileset should succeed");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn diamond_lattice_completes() {
    // A multi-layer lattice where each stage depends on two stages in the previous layer:
    // dense cross-dependencies stress readiness tracking and output propagation.
    let dir = unique_dir("lattice");
    let d = dir.display().to_string().replace('\\', "/");
    let layers = 12;
    let width = 8;

    let mut src = String::new();
    src.push_str("default pipeline build {\n    stages: [");
    let mut first = true;
    for l in 0..layers {
        for w in 0..width {
            if !first {
                src.push_str(", ");
            }
            first = false;
            src.push_str(&format!("n{l}_{w}"));
        }
    }
    src.push_str("]\n}\n");

    for l in 0..layers {
        for w in 0..width {
            let out = format!("{d}/out/n{l}_{w}.bin");
            if l == 0 {
                src.push_str(&format!(
                    "stage n{l}_{w} {{\n    outputs: [\"{out}\"]\n    \
                     steps {{ write \"{out}\" content: \"x\" }}\n}}\n"
                ));
            } else {
                let p1 = (l - 1, w);
                let p2 = (l - 1, (w + 1) % width);
                src.push_str(&format!(
                    "stage n{l}_{w} {{\n    inputs: [n{}_{}.outputs, n{}_{}.outputs]\n    \
                     outputs: [\"{out}\"]\n    steps {{ write \"{out}\" content: \"x\" }}\n}}\n",
                    p1.0, p1.1, p2.0, p2.1
                ));
            }
        }
    }

    run(&src, &dir, jobs()).expect("lattice pipeline should succeed");

    for l in 0..layers {
        for w in 0..width {
            assert!(
                dir.join(format!("out/n{l}_{w}.bin")).exists(),
                "lattice node n{l}_{w} must run"
            );
        }
    }

    let _ = std::fs::remove_dir_all(&dir);
}
