//! Phase 15 integration tests — the example scripts under the repo-root `tests/`.
//!
//! These drive the real `parse → analyze → eval` flow over the committed example
//! `.ms` files, proving the standard-library modules, the new integer/boolean literal
//! types, and the external-plugin path all work end to end. They also assert that the
//! intentionally-invalid example is rejected during semantic analysis.

use std::path::{Path, PathBuf};

use mainstage_core::{
    Error, ModuleRegistry, Source, Value, analyze_with, eval_program_with, parse,
};

/// The repo-root `tests/` directory (one level above this crate's manifest dir).
fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("tests")
}

/// Look up an evaluated `let` binding by name.
fn let_val<'a>(lets: &'a [(String, Value)], name: &str) -> &'a Value {
    lets.iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v)
        .unwrap_or_else(|| panic!("no let binding named '{name}'"))
}

/// Parse, analyze, and evaluate the example at `dir/file`, sharing one plugin-aware
/// registry between analysis and evaluation exactly as the CLI does.
fn run_example(dir: &Path, file: &str) -> Vec<(String, Value)> {
    let path = dir.join(file);
    let source = Source::from_file(&path).expect("example file should exist");
    let program = parse(&source).expect("example should parse");
    let registry = ModuleRegistry::with_plugins(dir).expect("registry should build");
    analyze_with(&program, &registry).expect("example should analyze");
    let ctx = eval_program_with(&program, dir, registry).expect("example should evaluate");
    ctx.let_values
}

#[test]
fn stdlib_example_evaluates_successfully() {
    let dir = examples_dir();
    let lets = run_example(&dir, "stdlib.ms");

    // Integer & boolean literals.
    assert!(matches!(let_val(&lets, "year"), Value::Int(2026)));
    assert!(matches!(let_val(&lets, "offset"), Value::Int(-5)));
    assert!(matches!(let_val(&lets, "verbose"), Value::Bool(true)));
    assert!(matches!(let_val(&lets, "port_url"), Value::String(s) if s == "http://localhost:2026"));

    // str / path module results.
    assert!(matches!(let_val(&lets, "shout"), Value::String(s) if s == "MAINSTAGE"));
    assert!(matches!(let_val(&lets, "slug"), Value::String(s) if s == "a-b-c"));
    assert!(matches!(let_val(&lets, "out_path"), Value::String(s) if s == "dist/app"));

    // json + fs (read-only) results.
    assert!(matches!(let_val(&lets, "app_name"), Value::String(s) if s == "demo"));
    assert!(matches!(let_val(&lets, "app_port"), Value::String(s) if s == "8080"));
    assert!(matches!(let_val(&lets, "feature0"), Value::String(s) if s == "build"));
    assert!(matches!(let_val(&lets, "present"), Value::Bool(true)));

    // General comparison conditions (Phase 41): a `let`/`project.<field>`-driven `if`.
    assert!(matches!(let_val(&lets, "release"), Value::String(s) if s == "ga"));
    assert!(matches!(let_val(&lets, "mode"), Value::String(s) if s == "missing"));
    assert!(matches!(let_val(&lets, "tagged"), Value::String(s) if s == "yes"));
    assert!(matches!(let_val(&lets, "no_ports"), Value::String(s) if s == "some"));
}

#[test]
fn validation_errors_example_is_rejected() {
    let dir = examples_dir();
    let source = Source::from_file(dir.join("validation_errors.ms")).unwrap();
    let program = parse(&source).expect("file should still parse — the errors are semantic");
    let registry = ModuleRegistry::with_plugins(&dir).unwrap();

    match analyze_with(&program, &registry) {
        Ok(_) => panic!("expected analysis to reject the invalid example"),
        Err(Error::Semantic(diags)) => {
            let joined = diags.iter().map(|d| d.message.as_str()).collect::<Vec<_>>().join("\n");
            assert!(joined.contains("must be string, found int"), "{joined}");
            assert!(joined.contains("has no method 'nonexistent'"), "{joined}");
            assert!(joined.contains("undeclared module 'path'"), "{joined}");
        }
        Err(other) => panic!("expected a semantic error, got: {other:?}"),
    }
}

#[test]
fn params_example_resolves_defaults_and_overrides() {
    use mainstage_core::eval_program_with_overrides;
    use std::collections::HashMap;

    let dir = examples_dir().parent().unwrap().join("examples/params");
    let source = Source::from_file(dir.join("main.ms")).expect("params example should exist");
    let program = parse(&source).expect("params example should parse");
    let registry = ModuleRegistry::with_plugins(&dir).unwrap();
    analyze_with(&program, &registry).expect("params example should analyze");

    // With no overrides the declared defaults resolve, and `profile` follows `release`.
    let ctx = eval_program_with_overrides(&program, &dir, registry.clone(), &HashMap::new())
        .expect("defaults should evaluate");
    assert!(matches!(let_val(&ctx.params, "release"), Value::Bool(false)));
    assert!(matches!(let_val(&ctx.let_values, "profile"), Value::String(s) if s == "debug"));

    // A `-D release=true` override flips the boolean and the derived `profile`.
    let overrides = HashMap::from([("release".to_string(), "true".to_string())]);
    let ctx = eval_program_with_overrides(&program, &dir, registry, &overrides)
        .expect("overrides should evaluate");
    assert!(matches!(let_val(&ctx.params, "release"), Value::Bool(true)));
    assert!(matches!(let_val(&ctx.let_values, "profile"), Value::String(s) if s == "release"));
}

/// The plugin example shells out to `tests/plugin/greet.sh`; gate it on unix, which
/// is where the POSIX-shell plugin and the execute bit are meaningful.
#[cfg(unix)]
#[test]
fn plugin_example_evaluates_successfully() {
    let dir = examples_dir().join("plugin");
    let lets = run_example(&dir, "main.ms");

    // A string round-trips through the plugin and back into a built-in (`str.upper`).
    assert!(matches!(let_val(&lets, "who"), Value::String(s) if s == "hello, World"));
    assert!(matches!(let_val(&lets, "shout"), Value::String(s) if s == "HELLO, WORLD"));
    // An integer literal round-trips through the plugin's int-typed method.
    assert!(matches!(let_val(&lets, "n"), Value::Int(21)));
}
