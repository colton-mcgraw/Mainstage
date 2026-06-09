//! Phase 3 integration tests — expression evaluator and built-in variables.
//!
//! Drive `eval_program` end-to-end (parse → evaluate) over real source and a real
//! script directory, covering `let`/`project` evaluation, string interpolation,
//! `glob`/`fileset`, and `if/else` conditional resolution.

use std::path::PathBuf;

use mainstage_core::{eval_program, parse, EvalContext, Source, Value};

/// Parse `src` and evaluate it relative to `script_dir`, asserting success.
fn eval_in(src: &str, script_dir: &std::path::Path) -> EvalContext {
    let program = parse(&Source::from_str("test.ms", src)).expect("parse should succeed");
    eval_program(&program, script_dir)
        .unwrap_or_else(|e| panic!("expected eval to succeed, got: {e}"))
}

/// Parse and evaluate `src` relative to the current directory.
fn eval(src: &str) -> EvalContext {
    eval_in(src, &PathBuf::from("."))
}

/// Look up an evaluated `let` binding by name.
fn let_val<'a>(ctx: &'a EvalContext, name: &str) -> &'a Value {
    ctx.let_values
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v)
        .unwrap_or_else(|| panic!("no let binding named '{name}'"))
}

/// A unique temporary directory for filesystem-touching tests.
fn unique_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("ms_eval_{tag}_{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ── let / project evaluation ────────────────────────────────────────────────────

#[test]
fn evaluates_string_and_bool_lets() {
    let ctx = eval(
        r#"
        let name = "app";
        let release = true;
        "#,
    );
    assert!(matches!(let_val(&ctx, "name"), Value::String(s) if s == "app"));
    assert!(matches!(let_val(&ctx, "release"), Value::Bool(true)));
}

#[test]
fn evaluates_project_fields() {
    let ctx = eval(r#"project { name: "app" version: "2.0" }"#);
    assert!(matches!(ctx.project_fields.get("name"), Some(Value::String(s)) if s == "app"));
    assert!(matches!(ctx.project_fields.get("version"), Some(Value::String(s)) if s == "2.0"));
}

#[test]
fn evaluates_list_let() {
    let ctx = eval(r#"let items = ["a", "b", "c"];"#);
    match let_val(&ctx, "items") {
        Value::List(items) => assert_eq!(items.len(), 3),
        other => panic!("expected list, got {other:?}"),
    }
}

// ── String interpolation ────────────────────────────────────────────────────────

#[test]
fn interpolates_prior_let_binding() {
    let ctx = eval(
        r#"
        let base = "core";
        let full = "lib-${base}-x";
        "#,
    );
    assert!(matches!(let_val(&ctx, "full"), Value::String(s) if s == "lib-core-x"));
}

#[test]
fn interpolates_project_field() {
    let ctx = eval(
        r#"
        project { name: "demo" }
        let tag = "v-${project.name}";
        "#,
    );
    assert!(matches!(let_val(&ctx, "tag"), Value::String(s) if s == "v-demo"));
}

// ── platform / conditionals ─────────────────────────────────────────────────────

#[test]
fn platform_builtin_resolves() {
    let ctx = eval("let p = platform;");
    // Host platform should be one of the known strings (or "unknown").
    let val = let_val(&ctx, "p");
    assert!(matches!(val, Value::String(s) if !s.is_empty()));
}

#[test]
fn if_else_selects_branch_by_platform() {
    // Whatever the host platform is, exactly one branch must be selected, and the
    // result must equal the platform-driven choice.
    let ctx = eval(
        r#"
        let target = if platform == "windows" { "win" } else { "nix" };
        "#,
    );
    let expected = if ctx.platform == "windows" { "win" } else { "nix" };
    assert!(matches!(let_val(&ctx, "target"), Value::String(s) if s == expected));
}

#[test]
fn env_condition_drives_if_else() {
    // SAFETY: single-threaded within this test; the var name is unique to this test.
    unsafe { std::env::set_var("_MS_EVAL_FLAG", "1") };
    let ctx = eval(r#"let v = if env("_MS_EVAL_FLAG") { "on" } else { "off" };"#);
    assert!(matches!(let_val(&ctx, "v"), Value::String(s) if s == "on"));
    unsafe { std::env::remove_var("_MS_EVAL_FLAG") };
}

// ── glob / fileset ──────────────────────────────────────────────────────────────

#[test]
fn glob_resolves_fileset_relative_to_script_dir() {
    let dir = unique_dir("glob");
    std::fs::write(dir.join("a.rs"), "").unwrap();
    std::fs::write(dir.join("b.rs"), "").unwrap();
    std::fs::write(dir.join("c.txt"), "").unwrap();

    let ctx = eval_in(r#"let sources = glob("*.rs");"#, &dir);
    match let_val(&ctx, "sources") {
        Value::FileSet(entries) => {
            assert_eq!(entries.len(), 2, "expected 2 .rs files, got {entries:?}");
            let mut names: Vec<_> = entries.iter().map(|e| e.name.clone()).collect();
            names.sort();
            assert_eq!(names, vec!["a.rs".to_string(), "b.rs".to_string()]);
        }
        other => panic!("expected fileset, got {other:?}"),
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fileset_entry_exposes_file_properties() {
    let dir = unique_dir("props");
    std::fs::write(dir.join("main.rs"), "").unwrap();

    let ctx = eval_in(r#"let f = glob("*.rs");"#, &dir);
    match let_val(&ctx, "f") {
        Value::FileSet(entries) => {
            let e = &entries[0];
            assert_eq!(e.name, "main.rs");
            assert_eq!(e.stem, "main");
            assert_eq!(e.ext, "rs");
            assert_eq!(e.get_field("name").as_deref(), Some("main.rs"));
            assert_eq!(e.get_field("stem").as_deref(), Some("main"));
            assert_eq!(e.get_field("ext").as_deref(), Some("rs"));
            assert!(e.get_field("nonexistent").is_none());
        }
        other => panic!("expected fileset, got {other:?}"),
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn glob_with_no_matches_is_empty_fileset() {
    let dir = unique_dir("empty");
    let ctx = eval_in(r#"let f = glob("*.nope");"#, &dir);
    assert!(matches!(let_val(&ctx, "f"), Value::FileSet(entries) if entries.is_empty()));
    let _ = std::fs::remove_dir_all(&dir);
}
