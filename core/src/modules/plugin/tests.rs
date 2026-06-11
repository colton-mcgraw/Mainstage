//! Tests for the plugin protocol, `ExternalModule`, and discovery.
//!
//! Protocol (de)serialization is verified in-process. The subprocess-driven tests
//! write a small POSIX-shell plugin to a temp directory and are gated on `unix`,
//! since they rely on the execute bit and a `/bin/sh` interpreter.

use super::protocol::*;
use super::*;
use crate::error::Span;
use crate::eval::{FileEntry, Value};
use crate::modules::{MethodSig, ModuleRegistry, NamedParam, Param, ResolvedArg, ValueTy};
use std::path::PathBuf;

fn unique_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("ms_plugin_{tag}_{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ── Protocol round-trips ────────────────────────────────────────────────────────

#[test]
fn wire_value_round_trips_every_variant() {
    let value = Value::List(vec![
        Value::String("hi".into()),
        Value::Bool(true),
        Value::FileSet(vec![FileEntry::from_path(PathBuf::from("/tmp/a.rs"))]),
    ]);
    let restored = WireValue::from_value(&value).into_value();
    // Compare via display since `Value` has no `PartialEq`.
    assert_eq!(restored.display_string(), value.display_string());
    match restored {
        Value::List(items) => assert_eq!(items.len(), 3),
        other => panic!("expected list, got {other:?}"),
    }
}

#[test]
fn wire_value_round_trips_integers() {
    let restored = WireValue::from_value(&Value::Int(-42)).into_value();
    assert!(matches!(restored, Value::Int(-42)), "{restored:?}");
    let json = serde_json::to_string(&WireValue::from_value(&Value::Int(7))).unwrap();
    assert_eq!(json, r#"{"type":"int","value":7}"#);
}

#[test]
fn int_type_tag_round_trips_in_signatures() {
    let wire: WireMethodSig = serde_json::from_str(
        r#"{"name":"add","params":[{"name":"a","type":"int","required":true}],"returns":"int"}"#,
    )
    .unwrap();
    let sig = wire.into_sig().unwrap();
    assert_eq!(sig.params[0].ty, ValueTy::Int);
    assert_eq!(sig.returns, ValueTy::Int);
}

#[test]
fn wire_value_json_is_internally_tagged() {
    let json = serde_json::to_string(&WireValue::from_value(&Value::String("x".into()))).unwrap();
    assert_eq!(json, r#"{"type":"string","value":"x"}"#);
}

#[test]
fn call_request_serializes_with_op_and_args() {
    let request = Request::Call {
        method: "echo".into(),
        args: vec![WireArg::from_resolved(&ResolvedArg {
            name: None,
            value: Value::String("hello".into()),
        })],
    };
    let json = serde_json::to_string(&request).unwrap();
    assert_eq!(
        json,
        r#"{"op":"call","method":"echo","args":[{"value":{"type":"string","value":"hello"}}]}"#
    );
}

#[test]
fn describe_op_serializes_without_fields() {
    assert_eq!(serde_json::to_string(&Request::Describe).unwrap(), r#"{"op":"describe"}"#);
}

#[test]
fn method_sig_round_trips_through_the_wire() {
    let sig = MethodSig {
        name: "lint".into(),
        params: vec![Param { name: "path".into(), ty: ValueTy::String, required: true }],
        named: vec![NamedParam { name: "strict".into(), ty: ValueTy::Bool, required: false }],
        returns: ValueTy::List,
    };
    let restored = sig_to_wire(&sig).into_sig().unwrap();
    assert_eq!(restored.name, "lint");
    assert_eq!(restored.params[0].ty, ValueTy::String);
    assert!(restored.params[0].required);
    assert_eq!(restored.named[0].ty, ValueTy::Bool);
    assert!(!restored.named[0].required);
    assert_eq!(restored.returns, ValueTy::List);
}

#[test]
fn method_sig_defaults_unspecified_fields() {
    // Only `name` is given: params/named default to empty, return type to `any`.
    let wire: WireMethodSig = serde_json::from_str(r#"{"name":"now"}"#).unwrap();
    let sig = wire.into_sig().unwrap();
    assert_eq!(sig.min_positional(), 0);
    assert_eq!(sig.returns, ValueTy::Any);
}

#[test]
fn unknown_type_tag_is_rejected() {
    let wire: WireMethodSig =
        serde_json::from_str(r#"{"name":"f","returns":"frobnicate"}"#).unwrap();
    let err = wire.into_sig().unwrap_err();
    assert!(err.contains("frobnicate"), "{err}");
}

// ── Subprocess-driven tests ─────────────────────────────────────────────────────

/// Write an executable POSIX-shell plugin to `dir/<name>` and return its path. The
/// plugin answers `describe` with a fixed signature set and a handful of `call`
/// methods (`ping`, `boom`, `echo`).
#[cfg(unix)]
fn write_plugin(dir: &std::path::Path, name: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let script = r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"op":"describe"'*)
      printf '%s\n' '{"name":"ignored","methods":[{"name":"ping","params":[],"returns":"string"},{"name":"echo","params":[{"name":"text","type":"string","required":true}],"returns":"string"},{"name":"boom","params":[],"returns":"string"}]}'
      ;;
    *'"method":"ping"'*)
      printf '%s\n' '{"ok":{"type":"string","value":"pong"}}'
      ;;
    *'"method":"boom"'*)
      printf '%s\n' '{"err":"kaboom"}'
      ;;
    *'"method":"echo"'*)
      v=$(printf '%s' "$line" | sed 's/.*"value":"\([^"]*\)"}.*/\1/')
      printf '%s\n' "{\"ok\":{\"type\":\"string\",\"value\":\"$v\"}}"
      ;;
    *)
      printf '%s\n' '{"err":"unknown method"}'
      ;;
  esac
done
"#;
    let path = dir.join(name);
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(unix)]
fn span() -> Span {
    Span { file: PathBuf::from("test.ms"), line_start: 1, col_start: 1, line_end: 1, col_end: 1 }
}

#[cfg(unix)]
#[test]
fn external_module_describes_and_calls() {
    let dir = unique_dir("call");
    let exe = write_plugin(&dir, "demo");
    let module = ExternalModule::load("demo", &exe, &dir).unwrap();

    assert_eq!(module.name(), "demo");
    let names: Vec<&str> = module.methods().iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["ping", "echo", "boom"]);

    let span = span();
    let cx = ModuleCx {
        span: &span,
        script_dir: &dir,
        permissions: crate::modules::Permissions::all(),
    };

    // A successful call returns the plugin's `ok` value.
    let out = module.call("ping", &[], &cx).unwrap();
    assert_eq!(out.display_string(), "pong");

    // Arguments round-trip to the plugin and back.
    let arg = ResolvedArg { name: None, value: Value::String("roundtrip".into()) };
    let echoed = module.call("echo", &[arg], &cx).unwrap();
    assert_eq!(echoed.display_string(), "roundtrip");
}

#[cfg(unix)]
#[test]
fn plugin_err_maps_to_eval_error_with_span() {
    let dir = unique_dir("err");
    let exe = write_plugin(&dir, "demo");
    let module = ExternalModule::load("demo", &exe, &dir).unwrap();

    let span = span();
    let cx = ModuleCx {
        span: &span,
        script_dir: &dir,
        permissions: crate::modules::Permissions::all(),
    };
    let err = module.call("boom", &[], &cx).unwrap_err();
    match err {
        Error::Eval(diags) => {
            assert!(diags[0].message.contains("kaboom"), "{}", diags[0].message);
            assert_eq!(diags[0].span.as_ref(), Some(&span));
        }
        other => panic!("expected Eval error, got {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn missing_executable_fails_to_load() {
    let dir = unique_dir("missing");
    match ExternalModule::load("ghost", &dir.join("does-not-exist"), &dir) {
        Err(Error::Eval(diags)) => assert!(diags[0].message.contains("ghost")),
        Ok(_) => panic!("expected load to fail"),
        Err(other) => panic!("expected Eval error, got {other:?}"),
    }
}

// ── Discovery ───────────────────────────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn discovers_directory_and_namespaced_plugins() {
    let dir = unique_dir("discover");
    let plugins = dir.join(PLUGINS_DIR);
    std::fs::create_dir_all(plugins.join("acme")).unwrap();
    write_plugin(&plugins, "mytool");
    write_plugin(&plugins.join("acme"), "lint");

    let reserved = HashSet::new();
    let found = discover(&dir, &reserved).unwrap();
    let mut names: Vec<&str> = found.iter().map(|m| m.name()).collect();
    names.sort_unstable();
    assert_eq!(names, ["acme/lint", "mytool"]);
}

#[cfg(unix)]
#[test]
fn manifest_plugins_are_discovered_and_resolved() {
    let dir = unique_dir("manifest");
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    write_plugin(&bin, "fmt");
    std::fs::write(dir.join(MANIFEST), "[plugins]\n\"acme/fmt\" = \"bin/fmt\"\n").unwrap();

    let found = discover(&dir, &HashSet::new()).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name(), "acme/fmt");
}

#[cfg(unix)]
#[test]
fn built_in_names_are_never_shadowed() {
    let dir = unique_dir("shadow");
    let plugins = dir.join(PLUGINS_DIR);
    std::fs::create_dir_all(&plugins).unwrap();
    // A plugin named `git` must be skipped — `git` is a built-in.
    write_plugin(&plugins, "git");
    write_plugin(&plugins, "ok");

    let mut reserved = HashSet::new();
    reserved.insert("git".to_string());
    let found = discover(&dir, &reserved).unwrap();
    let names: Vec<&str> = found.iter().map(|m| m.name()).collect();
    assert_eq!(names, ["ok"]);
}

#[cfg(unix)]
#[test]
fn directory_entry_wins_over_manifest_conflict() {
    let dir = unique_dir("conflict");
    let plugins = dir.join(PLUGINS_DIR);
    std::fs::create_dir_all(&plugins).unwrap();
    write_plugin(&plugins, "tool");
    // Manifest declares the same name pointing elsewhere; the directory entry wins.
    std::fs::write(dir.join(MANIFEST), "[plugins]\ntool = \"nonexistent\"\n").unwrap();

    // Discovery must succeed — proving the manifest's bogus path was never used.
    let found = discover(&dir, &HashSet::new()).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name(), "tool");
}

#[cfg(unix)]
#[test]
fn with_plugins_registry_validates_plugin_signatures() {
    let dir = unique_dir("registry");
    let plugins = dir.join(PLUGINS_DIR);
    std::fs::create_dir_all(&plugins).unwrap();
    write_plugin(&plugins, "demo");

    let registry = ModuleRegistry::with_plugins(&dir).unwrap();
    // Built-ins remain available alongside the plugin.
    assert!(registry.contains("git"));
    assert!(registry.contains("demo"));
    // The plugin's signatures are visible to semantic analysis.
    assert!(registry.method_sig("demo", "ping").is_some());
    assert_eq!(registry.method_sig("demo", "echo").unwrap().min_positional(), 1);
}

#[test]
fn malformed_manifest_is_a_hard_error() {
    let dir = unique_dir("badmanifest");
    std::fs::write(dir.join(MANIFEST), "this is not = valid = toml").unwrap();
    assert!(discover(&dir, &HashSet::new()).is_err());
}

#[test]
fn no_plugins_dir_yields_empty_discovery() {
    let dir = unique_dir("empty");
    assert!(discover(&dir, &HashSet::new()).unwrap().is_empty());
}
