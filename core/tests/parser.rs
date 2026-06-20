//! Phase 1 integration tests — lexer, parser, and AST construction.
//!
//! These exercise the public `parse` entry point against real `.ms` source text,
//! asserting on the shape of the produced AST, source spans, and parse-error paths.

use mainstage_core::ast::*;
use mainstage_core::{Error, Source, parse};

/// Parse `src`, asserting success, and return the `Program`.
fn parse_ok(src: &str) -> Program {
    let source = Source::from_str("test.ms", src);
    parse(&source).unwrap_or_else(|e| panic!("expected parse to succeed, got: {e}"))
}

/// Parse `src`, asserting it fails, and return the parse diagnostics.
fn parse_err(src: &str) -> Vec<mainstage_core::Diagnostic> {
    let source = Source::from_str("test.ms", src);
    match parse(&source) {
        Ok(_) => panic!("expected parse to fail for: {src:?}"),
        Err(Error::Parse(diags)) => diags,
        Err(other) => panic!("expected Error::Parse, got: {other:?}"),
    }
}

// ── Declarations ────────────────────────────────────────────────────────────────

#[test]
fn parses_import_decl() {
    let program = parse_ok(r#"import "git" as vcs;"#);
    assert_eq!(program.items.len(), 1);
    match &program.items[0] {
        Item::Import(d) => {
            assert_eq!(d.module, "git");
            assert_eq!(d.alias, "vcs");
        }
        other => panic!("expected import, got {other:?}"),
    }
}

#[test]
fn parses_include_decl() {
    let program = parse_ok(r#"include "components/build.ms";"#);
    assert_eq!(program.items.len(), 1);
    match &program.items[0] {
        Item::Include(d) => assert_eq!(d.path, "components/build.ms"),
        other => panic!("expected include, got {other:?}"),
    }
}

#[test]
fn parses_let_decl_with_string() {
    let program = parse_ok(r#"let target = "release";"#);
    match &program.items[0] {
        Item::Let(d) => {
            assert_eq!(d.name, "target");
            match &d.value {
                Expr::String(s) => {
                    assert!(matches!(&s.parts[0], StringPart::Literal(l) if l == "release"))
                }
                other => panic!("expected string expr, got {other:?}"),
            }
        }
        other => panic!("expected let, got {other:?}"),
    }
}

// ── Params (Phase 49) ─────────────────────────────────────────────────────────────

#[test]
fn parses_param_decl_of_each_type() {
    let program = parse_ok(
        r#"param target: string = "release";
           param jobs: int = 4;
           param release: bool = true;
           param features: list = ["a", "b"];"#,
    );
    assert_eq!(program.items.len(), 4);
    let types: Vec<(String, ParamType)> = program
        .items
        .iter()
        .map(|item| match item {
            Item::Param(d) => (d.name.clone(), d.ty),
            other => panic!("expected param, got {other:?}"),
        })
        .collect();
    assert_eq!(
        types,
        vec![
            ("target".to_string(), ParamType::String),
            ("jobs".to_string(), ParamType::Int),
            ("release".to_string(), ParamType::Bool),
            ("features".to_string(), ParamType::List),
        ]
    );
}

#[test]
fn param_default_is_a_full_expression() {
    // The default may reference an earlier binding, like any `let` value.
    let program = parse_ok(
        r#"let base = "rel";
           param target: string = "${base}-build";"#,
    );
    match &program.items[1] {
        Item::Param(d) => {
            assert_eq!(d.name, "target");
            assert!(matches!(d.default, Expr::String(_)));
        }
        other => panic!("expected param, got {other:?}"),
    }
}

#[test]
fn param_without_type_is_a_parse_error() {
    parse_err(r#"param target = "release";"#);
}

// ── Project ─────────────────────────────────────────────────────────────────────

#[test]
fn parses_project_block_with_fields() {
    let program = parse_ok(
        r#"project {
            name: "my-app"
            version: "1.0.0"
        }"#,
    );
    match &program.items[0] {
        Item::Project(b) => {
            assert_eq!(b.fields.len(), 2);
            assert_eq!(b.fields[0].name, "name");
            assert_eq!(b.fields[1].name, "version");
        }
        other => panic!("expected project, got {other:?}"),
    }
}

// ── Stage ───────────────────────────────────────────────────────────────────────

#[test]
fn parses_stage_with_all_fields() {
    let program = parse_ok(
        r#"stage compile {
            inputs: glob("src/**/*.rs")
            outputs: ["bin/app"]
            allow_failure: true
            steps {
                $ cargo build
            }
            on_failure {
                delete "bin/"
            }
        }"#,
    );
    match &program.items[0] {
        Item::Stage(s) => {
            assert_eq!(s.name, "compile");
            assert!(s.inputs.is_some());
            assert!(s.outputs.is_some());
            assert!(s.allow_failure);
            assert_eq!(s.steps.len(), 1);
            assert_eq!(s.on_failure.len(), 1);
            assert!(matches!(s.steps[0], Step::Exec(_)));
            assert!(matches!(s.on_failure[0], Step::Delete(_)));
        }
        other => panic!("expected stage, got {other:?}"),
    }
}

#[test]
fn stage_allow_failure_defaults_false() {
    let program = parse_ok("stage build {\n steps {\n $ make\n }\n}");
    match &program.items[0] {
        Item::Stage(s) => assert!(!s.allow_failure),
        other => panic!("expected stage, got {other:?}"),
    }
}

// ── Pipeline ────────────────────────────────────────────────────────────────────

#[test]
fn parses_default_pipeline() {
    let program = parse_ok(
        r#"default pipeline dev {
            stages: [compile]
        }"#,
    );
    match &program.items[0] {
        Item::Pipeline(p) => {
            assert!(p.is_default);
            assert_eq!(p.name, "dev");
            assert!(p.stages.is_some());
        }
        other => panic!("expected pipeline, got {other:?}"),
    }
}

#[test]
fn parses_named_pipeline_with_hooks() {
    let program = parse_ok(
        r#"pipeline release {
            stages: [compile, test]
            on_success {
                $ echo done
            }
            on_failure {
                $ echo failed
            }
        }"#,
    );
    match &program.items[0] {
        Item::Pipeline(p) => {
            assert!(!p.is_default);
            assert_eq!(p.name, "release");
            assert_eq!(p.on_success.len(), 1);
            assert_eq!(p.on_failure.len(), 1);
        }
        other => panic!("expected pipeline, got {other:?}"),
    }
}

// ── Template / use (Phase 46) ─────────────────────────────────────────────────────

#[test]
fn parses_template_item_and_use_step() {
    let program = parse_ok(
        r#"template setup {
            $ checkout
            log "ready"
        }
        stage build {
            steps {
                use setup;
                $ make
            }
        }"#,
    );
    match &program.items[0] {
        Item::Template(t) => {
            assert_eq!(t.name, "setup");
            assert_eq!(t.steps.len(), 2);
            assert!(matches!(t.steps[0], Step::Exec(_)));
            assert!(matches!(t.steps[1], Step::Log(_)));
        }
        other => panic!("expected template, got {other:?}"),
    }
    match &program.items[1] {
        Item::Stage(s) => match &s.steps[0] {
            Step::Use(u) => assert_eq!(u.name, "setup"),
            other => panic!("expected use step, got {other:?}"),
        },
        other => panic!("expected stage, got {other:?}"),
    }
}

#[test]
fn parses_empty_template() {
    let program = parse_ok("template noop {\n}\n");
    match &program.items[0] {
        Item::Template(t) => {
            assert_eq!(t.name, "noop");
            assert!(t.steps.is_empty());
        }
        other => panic!("expected template, got {other:?}"),
    }
}

// ── Expressions ─────────────────────────────────────────────────────────────────

fn first_let_value(src: &str) -> Expr {
    let program = parse_ok(src);
    match program.items.into_iter().next().unwrap() {
        Item::Let(d) => d.value,
        other => panic!("expected let, got {other:?}"),
    }
}

#[test]
fn parses_bool_literal() {
    assert!(matches!(first_let_value("let x = true;"), Expr::Bool(b) if b.value));
    assert!(matches!(first_let_value("let x = false;"), Expr::Bool(b) if !b.value));
}

#[test]
fn parses_integer_literal() {
    assert!(matches!(first_let_value("let x = 42;"), Expr::Int(i) if i.value == 42));
    assert!(matches!(first_let_value("let x = 0;"), Expr::Int(i) if i.value == 0));
    assert!(matches!(first_let_value("let x = -7;"), Expr::Int(i) if i.value == -7));
}

#[test]
fn integer_literals_appear_in_lists() {
    match first_let_value("let x = [1, 2, 3];") {
        Expr::List(l) => {
            assert_eq!(l.items.len(), 3);
            assert!(matches!(&l.items[0], Expr::Int(i) if i.value == 1));
        }
        other => panic!("expected list, got {other:?}"),
    }
}

#[test]
fn integer_with_trailing_letters_is_a_parse_error() {
    // `12abc` must not silently parse as the integer `12`.
    parse_err("let x = 12abc;");
}

#[test]
fn integer_out_of_range_errors() {
    // Beyond i64::MAX — reported rather than wrapping.
    let diags = parse_err("let x = 99999999999999999999;");
    assert!(diags.iter().any(|d| d.message.contains("out of range")), "{diags:?}");
}

#[test]
fn parses_list_expr() {
    match first_let_value(r#"let x = ["a", "b", "c"];"#) {
        Expr::List(l) => assert_eq!(l.items.len(), 3),
        other => panic!("expected list, got {other:?}"),
    }
}

#[test]
fn parses_glob_expr_with_multiple_patterns() {
    match first_let_value(r#"let x = glob("src/*.rs", "lib/*.rs");"#) {
        Expr::Glob(g) => assert_eq!(g.patterns, vec!["src/*.rs", "lib/*.rs"]),
        other => panic!("expected glob, got {other:?}"),
    }
}

#[test]
fn parses_module_call_with_named_arg() {
    match first_let_value("let x = git.sha(short: true);") {
        Expr::ModuleCall(c) => {
            assert_eq!(c.module, "git");
            assert_eq!(c.method, "sha");
            assert_eq!(c.args.len(), 1);
            assert_eq!(c.args[0].name.as_deref(), Some("short"));
        }
        other => panic!("expected module call, got {other:?}"),
    }
}

#[test]
fn parses_stage_ref_vs_member_access() {
    // `<stage>.outputs` must parse as StageRef, not MemberAccess.
    match first_let_value("let x = compile.outputs;") {
        Expr::StageRef(r) => assert_eq!(r.stage, "compile"),
        other => panic!("expected stage ref, got {other:?}"),
    }
    match first_let_value("let x = project.name;") {
        Expr::MemberAccess(m) => {
            assert_eq!(m.object, "project");
            assert_eq!(m.field, "name");
        }
        other => panic!("expected member access, got {other:?}"),
    }
}

#[test]
fn parses_if_else_expr() {
    match first_let_value(r#"let x = if platform == "windows" { "win" } else { "unix" };"#) {
        Expr::If(e) => {
            assert!(matches!(e.condition, Condition::Platform(_)));
        }
        other => panic!("expected if expr, got {other:?}"),
    }
}

#[test]
fn parses_string_interpolation_parts() {
    match first_let_value(r#"let x = "a-${platform}-b";"#) {
        Expr::String(s) => {
            assert_eq!(s.parts.len(), 3);
            assert!(matches!(&s.parts[0], StringPart::Literal(l) if l == "a-"));
            assert!(matches!(&s.parts[1], StringPart::Interpolation(_)));
            assert!(matches!(&s.parts[2], StringPart::Literal(l) if l == "-b"));
        }
        other => panic!("expected string, got {other:?}"),
    }
}

// ── Steps ───────────────────────────────────────────────────────────────────────

fn stage_steps(src: &str) -> Vec<Step> {
    let program = parse_ok(src);
    match program.items.into_iter().next().unwrap() {
        Item::Stage(s) => s.steps,
        other => panic!("expected stage, got {other:?}"),
    }
}

#[test]
fn parses_all_filesystem_steps() {
    let steps = stage_steps(
        r#"stage s {
            steps {
                copy "a" to "b"
                move "c" to "d"
                mkdir "out"
                delete "tmp"
                write "f" content: "data"
            }
        }"#,
    );
    assert!(matches!(steps[0], Step::Copy(_)));
    assert!(matches!(steps[1], Step::Move(_)));
    assert!(matches!(steps[2], Step::Mkdir(_)));
    assert!(matches!(steps[3], Step::Delete(_)));
    assert!(matches!(steps[4], Step::Write(_)));
}

#[test]
fn parses_exec_step_command_trimmed() {
    let steps = stage_steps("stage s {\n steps {\n $ cargo build --release\n }\n}");
    match &steps[0] {
        Step::Exec(e) => assert_eq!(e.command, "cargo build --release"),
        other => panic!("expected exec, got {other:?}"),
    }
}

#[test]
fn parses_if_step_with_else() {
    let steps = stage_steps(
        r#"stage s {
            steps {
                if platform == "linux" {
                    $ echo linux
                } else {
                    $ echo other
                }
            }
        }"#,
    );
    match &steps[0] {
        Step::If(s) => {
            assert_eq!(s.then_steps.len(), 1);
            assert_eq!(s.else_steps.len(), 1);
        }
        other => panic!("expected if step, got {other:?}"),
    }
}

#[test]
fn parses_for_step() {
    let steps = stage_steps(
        r#"stage s {
            steps {
                for f in inputs {
                    $ echo file
                }
            }
        }"#,
    );
    match &steps[0] {
        Step::For(s) => {
            assert_eq!(s.var, "f");
            assert_eq!(s.steps.len(), 1);
        }
        other => panic!("expected for step, got {other:?}"),
    }
}

#[test]
fn parses_try_step() {
    let steps = stage_steps(
        r#"stage s {
            steps {
                try {
                    $ apt-get update
                    mkdir "x"
                }
            }
        }"#,
    );
    match &steps[0] {
        Step::Try(s) => assert_eq!(s.steps.len(), 2),
        other => panic!("expected try step, got {other:?}"),
    }
}

#[test]
fn parses_workdir_step() {
    let steps = stage_steps(
        r#"stage s {
            steps {
                workdir "build" {
                    $ make
                    write "out.txt" content: "x"
                }
            }
        }"#,
    );
    match &steps[0] {
        Step::Workdir(s) => {
            assert!(matches!(&s.path, Expr::String(_)));
            assert_eq!(s.steps.len(), 2);
        }
        other => panic!("expected workdir step, got {other:?}"),
    }
}

#[test]
fn parses_with_env_step() {
    let steps = stage_steps(
        r#"stage s {
            steps {
                with_env { RUSTFLAGS: "-Dwarnings", CC: "clang" } {
                    $ cargo build
                }
            }
        }"#,
    );
    match &steps[0] {
        Step::WithEnv(s) => {
            assert_eq!(s.vars.len(), 2);
            assert_eq!(s.vars[0].key, "RUSTFLAGS");
            assert_eq!(s.vars[1].key, "CC");
            assert_eq!(s.steps.len(), 1);
        }
        other => panic!("expected with_env step, got {other:?}"),
    }
}

// ── Conditions ──────────────────────────────────────────────────────────────────

fn if_condition(src: &str) -> Condition {
    match first_let_value(src) {
        Expr::If(e) => e.condition,
        other => panic!("expected if expr, got {other:?}"),
    }
}

#[test]
fn parses_env_condition_forms() {
    // bare presence test
    assert!(matches!(
        if_condition(r#"let x = if env("CI") { "a" } else { "b" };"#),
        Condition::Env(c) if c.var == "CI" && c.comparison.is_none()
    ));
    // comparison form
    match if_condition(r#"let x = if env("MODE") == "prod" { "a" } else { "b" };"#) {
        Condition::Env(c) => {
            let (op, val) = c.comparison.unwrap();
            assert_eq!(op, CompareOp::Eq);
            assert_eq!(val, "prod");
        }
        other => panic!("expected env condition, got {other:?}"),
    }
}

#[test]
fn parses_boolean_condition_operators() {
    assert!(matches!(
        if_condition(r#"let x = if !env("CI") { "a" } else { "b" };"#),
        Condition::Not(_, _)
    ));
    assert!(matches!(
        if_condition(r#"let x = if env("A") and env("B") { "a" } else { "b" };"#),
        Condition::And(_, _, _)
    ));
    assert!(matches!(
        if_condition(r#"let x = if env("A") or env("B") { "a" } else { "b" };"#),
        Condition::Or(_, _, _)
    ));
}

#[test]
fn parses_general_comparison_conditions() {
    // `==` between an identifier operand and a string literal (name resolution is a
    // separate pass, so the parser accepts a free `m` here).
    match if_condition(r#"let x = if m == "x" { "a" } else { "b" };"#) {
        Condition::Compare(c) => {
            assert_eq!(c.op, CondOp::Eq);
            assert!(matches!(c.lhs, Expr::Ident(_)));
            assert!(matches!(c.rhs, Expr::String(_)));
        }
        other => panic!("expected compare condition, got {other:?}"),
    }
    // Each operator form is recognized.
    for (src, want) in [
        (r#"let x = if "a" != "b" { "y" } else { "n" };"#, CondOp::Ne),
        (r#"let x = if "rc" contains "r" { "y" } else { "n" };"#, CondOp::Contains),
        (r#"let x = if "a" in ["a", "b"] { "y" } else { "n" };"#, CondOp::In),
    ] {
        match if_condition(src) {
            Condition::Compare(c) => assert_eq!(c.op, want, "for {src}"),
            other => panic!("expected compare condition for {src}, got {other:?}"),
        }
    }
}

#[test]
fn parses_empty_condition() {
    assert!(matches!(
        if_condition(r#"let x = if empty("") { "a" } else { "b" };"#),
        Condition::Empty(_)
    ));
    // `!empty(...)` composes with the existing negation operator.
    assert!(matches!(
        if_condition(r#"let x = if !empty(["a"]) { "a" } else { "b" };"#),
        Condition::Not(_, _)
    ));
}

#[test]
fn env_and_platform_forms_still_win_over_general_comparison() {
    // The specific `env(...)` / `platform` forms must be preferred, not parsed as a
    // general comparison with `env`/`platform` as a bare identifier.
    assert!(matches!(
        if_condition(r#"let x = if env("MODE") == "prod" { "a" } else { "b" };"#),
        Condition::Env(_)
    ));
    assert!(matches!(
        if_condition(r#"let x = if platform == "linux" { "a" } else { "b" };"#),
        Condition::Platform(_)
    ));
}

// ── Spans, comments, and errors ─────────────────────────────────────────────────

#[test]
fn attaches_line_spans() {
    let program = parse_ok("\n\nlet x = \"v\";");
    // `let` is on the third line.
    assert_eq!(program.items[0].span().line_start, 3);
}

#[test]
fn ignores_line_comments() {
    let program = parse_ok(
        r#"// a comment
        let x = "v"; // trailing comment"#,
    );
    assert_eq!(program.items.len(), 1);
}

#[test]
fn syntax_error_returns_parse_error() {
    // Missing closing quote / malformed let.
    let diags = parse_err("let x = ;");
    assert!(!diags.is_empty());
}

#[test]
fn interpolation_not_allowed_in_glob_pattern() {
    // glob patterns are raw strings — interpolation should be rejected.
    let diags = parse_err(r#"let x = glob("src/${platform}/*.rs");"#);
    assert!(
        diags.iter().any(|d| d.message.contains("interpolation")),
        "expected an interpolation diagnostic, got: {diags:?}"
    );
}

#[test]
fn parses_readme_example() {
    // The full example from the README must parse without error.
    let src = r#"
import "env" as env;
import "git" as git;

project {
    name:    "my-app"
    version: git.tag()
}

let sources = glob("src/**/*.rs");
let out     = env.get("OUT_DIR", default: "dist");
let target  = if platform == "windows" {
    "x86_64-pc-windows-msvc"
} else {
    "x86_64-unknown-linux-gnu"
};

default pipeline dev {
    stages: [compile]
}

stage compile {
    inputs:  sources
    outputs: ["target/app"]

    steps {
        $ cargo build --release --target ${target}
    }

    on_failure {
        delete "target/"
    }
}
"#;
    let program = parse_ok(src);
    assert!(program.items.len() >= 6);
}
