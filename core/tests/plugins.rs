//! Phase 13 integration tests — external subprocess plugins.
//!
//! These drive the full discovery → load → analyze → evaluate path against a real
//! plugin process. They are unix-only because the test plugin is a `/bin/sh` script;
//! the host code path is identical on other platforms.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use mainstage_core::{analyze_with, eval_program_with, parse, Error, ModuleRegistry, Source};

/// A plugin exposing `say.hello(name) -> string` and `say.shout() -> string`.
/// It returns constants (parsing JSON args in `sh` is impractical); argument
/// encoding is covered by the protocol unit tests in `modules::external`.
const PLUGIN: &str = r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *describe*)
      printf '%s\n' '{"name":"say","methods":[{"name":"hello","params":[{"name":"name","ty":"string"}],"returns":"string"},{"name":"shout","returns":"string"}]}'
      ;;
    *hello*)
      printf '%s\n' '{"ok":{"type":"string","value":"hi there"}}'
      ;;
    *)
      printf '%s\n' '{"ok":{"type":"string","value":"HELLO"}}'
      ;;
  esac
done
"#;

fn unique_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("ms_plugins_it_{tag}_{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write the `say` plugin into `<dir>/.mainstage/plugins/say` and make it executable.
fn install_plugin(dir: &Path) {
    let plugins = dir.join(".mainstage").join("plugins");
    std::fs::create_dir_all(&plugins).unwrap();
    let exe = plugins.join("say");
    std::fs::write(&exe, PLUGIN).unwrap();
    let mut perms = std::fs::metadata(&exe).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&exe, perms).unwrap();
}

/// Build a registry with the project's imported plugins loaded.
fn registry_for(src: &str, dir: &Path) -> ModuleRegistry {
    let program = parse(&Source::from_str("main.ms", src)).expect("parse");
    let imports: Vec<&str> = src
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            l.strip_prefix("import \"").and_then(|rest| rest.split('"').next())
        })
        .collect();
    let mut registry = ModuleRegistry::standard();
    registry.load_plugins(&imports, dir).expect("plugins load");
    let _ = program;
    registry
}

#[test]
fn imported_plugin_loads_and_call_evaluates() {
    let dir = unique_dir("eval");
    install_plugin(&dir);

    let src = r#"
        import "say" as say;
        let greeting = say.hello("world");
    "#;
    let program = parse(&Source::from_str("main.ms", src)).unwrap();
    let registry = registry_for(src, &dir);

    // Analysis validates the plugin call against the describe-provided signature.
    analyze_with(&program, &registry).expect("analysis should pass");

    let ctx = eval_program_with(&program, &dir, registry).expect("eval should pass");
    let greeting = ctx
        .let_values
        .iter()
        .find(|(n, _)| n == "greeting")
        .map(|(_, v)| v)
        .expect("greeting binding");
    assert!(
        matches!(greeting, mainstage_core::Value::String(s) if s == "hi there"),
        "got: {greeting:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn analysis_validates_plugin_methods_like_builtins() {
    let dir = unique_dir("validate");
    install_plugin(&dir);

    // `say` has no `nope` method — this must fail at analysis time, just like a
    // bad built-in call would.
    let src = r#"
        import "say" as say;
        let x = say.nope();
    "#;
    let program = parse(&Source::from_str("main.ms", src)).unwrap();
    let registry = registry_for(src, &dir);

    match analyze_with(&program, &registry) {
        Err(Error::Semantic(diags)) => {
            assert!(
                diags.iter().any(|d| d.message.contains("has no method 'nope'")),
                "diags: {diags:?}"
            );
        }
        Ok(_) => panic!("expected a semantic error, but analysis succeeded"),
        Err(other) => panic!("expected Error::Semantic, got: {other:?}"),
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn undiscovered_plugin_import_is_an_unknown_module() {
    // No plugin installed: importing it leaves the name unregistered, and analysis
    // reports it as an unknown module.
    let dir = unique_dir("missing");

    let src = r#"
        import "ghost" as ghost;
        let x = ghost.run();
    "#;
    let program = parse(&Source::from_str("main.ms", src)).unwrap();
    let registry = registry_for(src, &dir);

    match analyze_with(&program, &registry) {
        Err(Error::Semantic(diags)) => {
            assert!(
                diags.iter().any(|d| d.message.contains("unknown module")),
                "diags: {diags:?}"
            );
        }
        Ok(_) => panic!("expected a semantic error, but analysis succeeded"),
        Err(other) => panic!("expected Error::Semantic, got: {other:?}"),
    }

    let _ = std::fs::remove_dir_all(&dir);
}
