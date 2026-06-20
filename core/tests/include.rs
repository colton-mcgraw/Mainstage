//! Phase 48 integration tests — multi-file composition (`include`).
//!
//! Drive the full `parse → expand_includes → analyze → eval → run` flow over a real
//! multi-file project on disk, verifying that included items merge into one flat build,
//! that cross-file name collisions are rejected by the existing duplicate-name checks
//! (flat-namespace rule), and that a `glob` in an included file resolves against *that*
//! file's directory rather than the root script's.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use mainstage_core::{
    Error, Source, analyze, ast::Item, eval_expr, eval_program, expand_includes, parse,
    run_pipeline,
};

/// A unique temp directory for one test's files.
fn temp_project(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ms_inc_{tag}_{nanos}_{n}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Parse the root file and flatten its includes, returning the merged program.
fn merged(root: &Path) -> mainstage_core::Result<mainstage_core::ast::Program> {
    let source = Source::from_file(root).expect("root should exist");
    let program = parse(&source).expect("root should parse");
    expand_includes(&program)
}

#[test]
fn multi_file_build_runs_end_to_end() {
    let dir = temp_project("e2e");
    let d = dir.display();
    // A component file declares the `compile` stage; the root wires it into a pipeline
    // alongside a local `package` stage that consumes `compile.outputs`.
    std::fs::write(
        dir.join("build.ms"),
        format!(
            "stage compile {{\n  outputs: [\"{d}/obj\"]\n  steps {{\n    write \"{d}/obj\" content: \"x\"\n  }}\n}}\n"
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("main.ms"),
        format!(
            "include \"build.ms\";\n\
             default pipeline ci {{\n  stages: [compile, package]\n}}\n\
             stage package {{\n  inputs: [compile.outputs]\n  steps {{\n    write \"{d}/pkg\" content: \"y\"\n  }}\n}}\n"
        ),
    )
    .unwrap();

    let program = merged(&dir.join("main.ms")).expect("includes should flatten");
    let analysis = analyze(&program).expect("flat program should analyze");
    let ctx = eval_program(&program, &dir).expect("eval should succeed");
    run_pipeline(&program, None, &ctx, &analysis).expect("multi-file pipeline should run");

    // Both stages — one authored in each file — ran.
    assert!(dir.join("obj").exists(), "the included `compile` stage ran");
    assert!(dir.join("pkg").exists(), "the root `package` stage ran");
}

#[test]
fn cross_file_name_collision_is_rejected() {
    let dir = temp_project("collision");
    // Both files define a stage named `build`: a flat-namespace collision.
    std::fs::write(dir.join("a.ms"), "stage build {\n  steps {\n    $ a\n  }\n}\n").unwrap();
    std::fs::write(
        dir.join("main.ms"),
        "include \"a.ms\";\nstage build {\n  steps {\n    $ b\n  }\n}\n",
    )
    .unwrap();

    let program = merged(&dir.join("main.ms")).expect("includes flatten without error");
    // The collision is caught by the ordinary duplicate-name check in sema.
    match analyze(&program) {
        Err(Error::Semantic(diags)) => {
            assert!(
                diags.iter().any(|d| d.message.contains("stage 'build' is already defined")),
                "expected a duplicate-stage diagnostic, got: {diags:?}"
            );
        }
        Ok(_) => panic!("expected a semantic collision error, but analysis succeeded"),
        Err(other) => panic!("expected a semantic collision error, got: {other:?}"),
    }
}

#[test]
fn glob_in_included_file_resolves_to_its_own_directory() {
    let dir = temp_project("glob");
    std::fs::create_dir_all(dir.join("components")).unwrap();
    // A source file lives next to the *included* component, not the root.
    std::fs::write(dir.join("components/data.in"), "payload").unwrap();
    std::fs::write(
        dir.join("components/comp.ms"),
        "stage gen {\n  inputs: glob(\"*.in\")\n  steps {\n    $ noop\n  }\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("main.ms"), "include \"components/comp.ms\";\n").unwrap();

    let program = merged(&dir.join("main.ms")).expect("includes flatten");
    // Evaluate from the project root: the glob must still find `components/data.in`,
    // because it resolves against the included file's directory, not the root.
    let ctx = eval_program(&program, &dir).expect("eval");
    let inputs = program
        .items
        .iter()
        .find_map(|i| match i {
            Item::Stage(s) if s.name == "gen" => s.inputs.as_ref(),
            _ => None,
        })
        .expect("gen stage has inputs");

    let value = eval_expr(inputs, &ctx).expect("glob should evaluate");
    let names = match value {
        mainstage_core::Value::FileSet(entries) => {
            entries.into_iter().map(|e| e.name).collect::<Vec<_>>()
        }
        other => panic!("expected a fileset, got {other:?}"),
    };
    assert!(
        names.contains(&"data.in".to_string()),
        "glob found the component's own file: {names:?}"
    );
}
