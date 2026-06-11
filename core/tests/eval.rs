//! Phase 3 integration tests — expression evaluator and built-in variables.
//!
//! Drive `eval_program` end-to-end (parse → evaluate) over real source and a real
//! script directory, covering `let`/`project` evaluation, string interpolation,
//! `glob`/`fileset`, and `if/else` conditional resolution.

use std::path::PathBuf;

use mainstage_core::{
    eval_program, eval_program_with, parse, EvalContext, Error, ModuleRegistry, Permissions,
    Source, Value,
};

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
fn evaluates_integer_lets() {
    let ctx = eval(
        r#"
        let year   = 2026;
        let offset = -5;
        "#,
    );
    assert!(matches!(let_val(&ctx, "year"), Value::Int(2026)));
    assert!(matches!(let_val(&ctx, "offset"), Value::Int(-5)));
}

#[test]
fn integers_interpolate_as_their_decimal_form() {
    let ctx = eval(
        r#"
        let port = 8080;
        let url  = "http://localhost:${port}/";
        "#,
    );
    assert!(matches!(let_val(&ctx, "url"), Value::String(s) if s == "http://localhost:8080/"));
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

// ── Standard-library modules (Phase 11) ──────────────────────────────────────────

fn string_of<'a>(ctx: &'a EvalContext, name: &str) -> &'a str {
    match let_val(ctx, name) {
        Value::String(s) => s,
        other => panic!("expected string for '{name}', got {other:?}"),
    }
}

#[test]
fn str_module_transforms_and_predicates() {
    let ctx = eval(
        r#"
        import "str" as str;
        let up = str.upper("abc");
        let rep = str.replace("a.b.c", ".", "/");
        let has = str.contains("hello", "ell");
        let n = str.len("café");
        "#,
    );
    assert_eq!(string_of(&ctx, "up"), "ABC");
    assert_eq!(string_of(&ctx, "rep"), "a/b/c");
    assert!(matches!(let_val(&ctx, "has"), Value::Bool(true)));
    assert_eq!(string_of(&ctx, "n"), "4");
}

#[test]
fn str_split_join_nested_passes_analysis_and_evaluates() {
    // The nested call exercises registry-aware type inference: `str.split` returns a
    // list, which `str.join` accepts — this must not be a static type error.
    let ctx = eval(
        r#"
        import "str" as str;
        let joined = str.join(str.split("a,b,c", ","), "/");
        "#,
    );
    assert_eq!(string_of(&ctx, "joined"), "a/b/c");
}

#[test]
fn path_module_components() {
    let ctx = eval(
        r#"
        import "path" as path;
        let p = path.join("src", "main.rs");
        let d = path.dir(p);
        let stem = path.stem(p);
        let renamed = path.with_ext(p, "o");
        "#,
    );
    assert_eq!(string_of(&ctx, "p"), "src/main.rs");
    assert_eq!(string_of(&ctx, "d"), "src");
    assert_eq!(string_of(&ctx, "stem"), "main");
    assert_eq!(string_of(&ctx, "renamed"), "src/main.o");
}

#[test]
fn hash_module_sha256() {
    let ctx = eval(
        r#"
        import "hash" as hash;
        let empty = hash.sha256("");
        "#,
    );
    assert_eq!(
        string_of(&ctx, "empty"),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn fs_module_reads_relative_to_script_dir() {
    let dir = unique_dir("fsmod");
    std::fs::write(dir.join("config.txt"), "value").unwrap();

    let ctx = eval_in(
        r#"
        import "fs" as fs;
        let here = fs.exists("config.txt");
        let gone = fs.exists("missing");
        let body = fs.read("config.txt");
        let n = fs.size("config.txt");
        "#,
        &dir,
    );
    assert!(matches!(let_val(&ctx, "here"), Value::Bool(true)));
    assert!(matches!(let_val(&ctx, "gone"), Value::Bool(false)));
    assert_eq!(string_of(&ctx, "body"), "value");
    assert_eq!(string_of(&ctx, "n"), "5");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn json_module_reads_and_queries_a_file() {
    // `.ms` string literals can't contain double quotes, so JSON lives in a file and
    // is read with `fs.read` — also exercising fs + json composition.
    let dir = unique_dir("jsonmod");
    std::fs::write(dir.join("pkg.json"), r#"{"name": "app", "tags": ["a", "b"]}"#).unwrap();

    let ctx = eval_in(
        r#"
        import "fs" as fs;
        import "json" as json;
        let doc = fs.read("pkg.json");
        let name = json.get(doc, "name");
        let tag = json.get(doc, "tags.1");
        "#,
        &dir,
    );
    assert_eq!(string_of(&ctx, "name"), "app");
    assert_eq!(string_of(&ctx, "tag"), "b");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn env_module_reads_variables_and_defaults() {
    // A uniquely named variable keeps this hermetic under parallel test execution.
    // SAFETY: the variable name is unique to this test, so no other test races on it.
    unsafe { std::env::set_var("_MS_EVAL_ENV_PRESENT", "yes") };

    let ctx = eval(
        r#"
        import "env" as env;
        let present = env.get("_MS_EVAL_ENV_PRESENT");
        let here = env.has("_MS_EVAL_ENV_PRESENT");
        let fallback = env.get("_MS_EVAL_ENV_ABSENT", default: "dflt");
        let absent = env.has("_MS_EVAL_ENV_ABSENT");
        "#,
    );

    assert_eq!(string_of(&ctx, "present"), "yes");
    assert!(matches!(let_val(&ctx, "here"), Value::Bool(true)));
    assert_eq!(string_of(&ctx, "fallback"), "dflt");
    assert!(matches!(let_val(&ctx, "absent"), Value::Bool(false)));

    unsafe { std::env::remove_var("_MS_EVAL_ENV_PRESENT") };
}

// ── Permissioned modules (Phase 14) ───────────────────────────────────────────────

/// Evaluate `src` with `perms` granted, returning the raw result so a permission
/// denial can be asserted as an error rather than panicking.
fn eval_with_perms(src: &str, perms: Permissions) -> Result<EvalContext, Error> {
    let program = parse(&Source::from_str("test.ms", src)).expect("parse should succeed");
    let registry = ModuleRegistry::standard().with_permissions(perms);
    eval_program_with(&program, &PathBuf::from("."), registry)
}

#[test]
fn time_module_now_unix_and_format() {
    // `time` is ungated, so the default registry evaluates it directly.
    let ctx = eval(
        r#"
        import "time" as time;
        let secs = time.unix();
        let year = time.format("%Y");
        "#,
    );
    assert!(string_of(&ctx, "secs").parse::<i64>().unwrap() > 0);
    assert_eq!(string_of(&ctx, "year").len(), 4);
}

#[test]
fn shell_run_denied_without_run_capability() {
    let err = eval_with_perms(
        r#"
        import "shell" as shell;
        let out = shell.run("echo hi");
        "#,
        Permissions::default(),
    )
    .expect_err("shell.run must be denied without the run capability");
    assert!(format!("{err}").contains("permission denied"), "got: {err}");
}

#[test]
fn shell_run_succeeds_with_run_capability() {
    let ctx = eval_with_perms(
        r#"
        import "shell" as shell;
        let out = shell.run("echo hi there");
        "#,
        Permissions::all(),
    )
    .expect("shell.run should succeed once granted");
    assert_eq!(string_of(&ctx, "out"), "hi there");
}

#[test]
fn http_get_denied_without_net_capability() {
    // The capability gate fires before any network access is attempted.
    let err = eval_with_perms(
        r#"
        import "http" as http;
        let body = http.get("https://example.com");
        "#,
        Permissions::default(),
    )
    .expect_err("http.get must be denied without the net capability");
    assert!(format!("{err}").contains("permission denied"), "got: {err}");
}
