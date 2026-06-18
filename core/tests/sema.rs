//! Phase 2 integration tests — semantic analysis.
//!
//! Drive the public `analyze` entry point with parsed programs and assert on name
//! resolution, forward-reference enforcement, uniqueness checks, the stage dependency
//! graph, and `if/else` type compatibility.

use mainstage_core::{Error, Source, analyze, parse};

/// Parse and analyze `src`, asserting both succeed; returns the analysis result.
fn analyze_ok(src: &str) -> mainstage_core::AnalysisResult {
    let program = parse(&Source::from_str("test.ms", src)).expect("parse should succeed");
    analyze(&program).unwrap_or_else(|e| panic!("expected analysis to succeed, got: {e}"))
}

/// Parse and analyze `src`, asserting analysis fails; returns the semantic diagnostics.
fn analyze_err(src: &str) -> Vec<mainstage_core::Diagnostic> {
    let program = parse(&Source::from_str("test.ms", src)).expect("parse should succeed");
    match analyze(&program) {
        Ok(_) => panic!("expected analysis to fail for: {src:?}"),
        Err(Error::Semantic(diags)) => diags,
        Err(other) => panic!("expected Error::Semantic, got: {other:?}"),
    }
}

/// True if any diagnostic message contains `needle`.
fn has_msg(diags: &[mainstage_core::Diagnostic], needle: &str) -> bool {
    diags.iter().any(|d| d.message.contains(needle))
}

// ── Name resolution ─────────────────────────────────────────────────────────────

#[test]
fn resolves_backward_let_reference() {
    analyze_ok(
        r#"
        let a = "x";
        let b = a;
        "#,
    );
}

#[test]
fn undefined_name_errors() {
    let diags = analyze_err("let a = missing;");
    assert!(has_msg(&diags, "undefined name 'missing'"));
}

#[test]
fn forward_let_reference_errors() {
    let diags = analyze_err(
        r#"
        let a = b;
        let b = "x";
        "#,
    );
    assert!(has_msg(&diags, "forward reference"));
}

#[test]
fn self_referential_let_errors() {
    // A binding may not reference itself (index >= current).
    let diags = analyze_err("let a = a;");
    assert!(has_msg(&diags, "forward reference"));
}

#[test]
fn undeclared_module_errors() {
    let diags = analyze_err(r#"let v = git.tag();"#);
    assert!(has_msg(&diags, "undeclared module 'git'"));
}

#[test]
fn declared_module_resolves() {
    analyze_ok(
        r#"
        import "git" as git;
        let v = git.tag();
        "#,
    );
}

// ── Module-call validation ────────────────────────────────────────────────────────

#[test]
fn unknown_imported_module_errors() {
    let diags = analyze_err(r#"import "bogus" as b;"#);
    assert!(has_msg(&diags, r#"unknown module "bogus""#));
}

#[test]
fn unknown_method_errors() {
    let diags = analyze_err(
        r#"
        import "git" as git;
        let v = git.branch();
        "#,
    );
    assert!(has_msg(&diags, "module 'git' has no method 'branch'"));
}

#[test]
fn permissioned_modules_resolve_and_validate() {
    // The capability-gated modules are registered, so their calls pass analysis
    // (permission is a runtime concern, not a static one); arity is still checked.
    analyze_ok(
        r#"
        import "shell" as shell;
        import "http" as http;
        import "time" as time;
        let a = shell.run("echo hi");
        let b = http.get("https://example.com");
        let c = time.format("%Y");
        let d = time.now();
        "#,
    );

    let diags = analyze_err(
        r#"
        import "time" as time;
        let bad = time.now("unexpected");
        "#,
    );
    assert!(has_msg(&diags, "time"));
}

#[test]
fn too_many_positional_args_errors() {
    // env.get takes exactly one positional (the variable name).
    let diags = analyze_err(
        r#"
        import "env" as env;
        let v = env.get("A", "B");
        "#,
    );
    assert!(has_msg(&diags, "expects 1 positional argument(s), found 2"));
}

#[test]
fn missing_positional_arg_errors() {
    let diags = analyze_err(
        r#"
        import "env" as env;
        let v = env.get();
        "#,
    );
    assert!(has_msg(&diags, "expects 1 positional argument(s), found 0"));
}

#[test]
fn unknown_named_arg_errors() {
    let diags = analyze_err(
        r#"
        import "git" as git;
        let v = git.sha(long: true);
        "#,
    );
    assert!(has_msg(&diags, "has no named argument 'long'"));
}

#[test]
fn wrong_positional_arg_type_errors() {
    // env.get's variable name must be a string, not a bool.
    let diags = analyze_err(
        r#"
        import "env" as env;
        let v = env.get(true);
        "#,
    );
    assert!(has_msg(&diags, "must be string, found bool"));
}

#[test]
fn wrong_named_arg_type_errors() {
    // git.sha's `short` must be a bool, not a string.
    let diags = analyze_err(
        r#"
        import "git" as git;
        let v = git.sha(short: "yes");
        "#,
    );
    assert!(has_msg(&diags, "must be bool, found string"));
}

#[test]
fn integer_literal_in_string_position_errors() {
    // An integer literal is rejected where a string parameter is declared.
    let diags = analyze_err(
        r#"
        import "str" as str;
        let v = str.upper(42);
        "#,
    );
    assert!(has_msg(&diags, "must be string, found int"));
}

#[test]
fn valid_calls_with_args_ok() {
    analyze_ok(
        r#"
        import "git" as git;
        import "env" as env;
        let a = git.sha(short: true);
        let b = env.get("HOME", default: "/tmp");
        "#,
    );
}

// ── Project access ──────────────────────────────────────────────────────────────

#[test]
fn project_access_without_block_errors() {
    let diags = analyze_err(r#"let n = project.name;"#);
    assert!(has_msg(&diags, "no `project` block"));
}

#[test]
fn unknown_project_field_errors() {
    let diags = analyze_err(
        r#"
        project { name: "app" }
        let v = project.version;
        "#,
    );
    assert!(has_msg(&diags, "unknown project field 'version'"));
}

#[test]
fn known_project_field_resolves() {
    analyze_ok(
        r#"
        project { name: "app" }
        let n = project.name;
        "#,
    );
}

// ── Stage references ────────────────────────────────────────────────────────────

#[test]
fn unknown_stage_ref_errors() {
    let diags = analyze_err(
        r#"
        stage build {
            inputs: [missing.outputs]
            steps { mkdir "x" }
        }
        "#,
    );
    assert!(has_msg(&diags, "unknown stage 'missing'"));
}

#[test]
fn unknown_stage_in_pipeline_list_errors() {
    let diags = analyze_err(
        r#"
        default pipeline dev { stages: [ghost] }
        "#,
    );
    assert!(has_msg(&diags, "unknown stage 'ghost'"));
}

// ── Uniqueness ──────────────────────────────────────────────────────────────────

#[test]
fn duplicate_stage_name_errors() {
    let diags = analyze_err(
        r#"
        stage a { steps { mkdir "x" } }
        stage a { steps { mkdir "y" } }
        "#,
    );
    assert!(has_msg(&diags, "stage 'a' is already defined"));
}

#[test]
fn duplicate_pipeline_name_errors() {
    let diags = analyze_err(
        r#"
        pipeline p { stages: [] }
        pipeline p { stages: [] }
        "#,
    );
    assert!(has_msg(&diags, "pipeline 'p' is already defined"));
}

#[test]
fn duplicate_let_binding_errors() {
    let diags = analyze_err(
        r#"
        let a = "x";
        let a = "y";
        "#,
    );
    assert!(has_msg(&diags, "let binding 'a' is already defined"));
}

#[test]
fn duplicate_import_alias_errors() {
    let diags = analyze_err(
        r#"
        import "git" as m;
        import "env" as m;
        "#,
    );
    assert!(has_msg(&diags, "import alias 'm' is already defined"));
}

#[test]
fn duplicate_project_field_errors() {
    let diags = analyze_err(
        r#"
        project {
            name: "a"
            name: "b"
        }
        "#,
    );
    assert!(has_msg(&diags, "project field 'name' is already defined"));
}

#[test]
fn multiple_default_pipelines_error() {
    let diags = analyze_err(
        r#"
        default pipeline a { stages: [] }
        default pipeline b { stages: [] }
        "#,
    );
    assert!(has_msg(&diags, "at most one pipeline"));
}

// ── if/else type compatibility ──────────────────────────────────────────────────

#[test]
fn incompatible_if_branches_error() {
    let diags = analyze_err(r#"let x = if platform == "linux" { "s" } else { true };"#);
    assert!(has_msg(&diags, "incompatible types"));
}

#[test]
fn compatible_if_branches_ok() {
    analyze_ok(r#"let x = if platform == "linux" { "a" } else { "b" };"#);
}

// ── Dependency graph ────────────────────────────────────────────────────────────

#[test]
fn dependency_graph_links_stage_outputs() {
    let result = analyze_ok(
        r#"
        stage compile { steps { mkdir "x" } }
        stage package {
            inputs: [compile.outputs]
            steps { mkdir "x" }
        }
        "#,
    );
    let pkg_deps = result.dependency_graph.get("package").expect("package in graph");
    assert_eq!(pkg_deps, &vec!["compile".to_string()]);
    // A stage with no stage refs has no dependencies.
    assert!(result.dependency_graph.get("compile").unwrap().is_empty());
}

#[test]
fn dependency_graph_collects_refs_through_if_branches() {
    let result = analyze_ok(
        r#"
        stage a { steps { mkdir "x" } }
        stage b { steps { mkdir "y" } }
        stage c {
            inputs: if platform == "linux" { [a.outputs] } else { [b.outputs] }
            steps { mkdir "x" }
        }
        "#,
    );
    let mut deps = result.dependency_graph.get("c").unwrap().clone();
    deps.sort();
    assert_eq!(deps, vec!["a".to_string(), "b".to_string()]);
}

// ── Explicit ordering (depends_on) ───────────────────────────────────────────────

#[test]
fn depends_on_adds_graph_edges() {
    let result = analyze_ok(
        r#"
        stage setup { steps { mkdir "x" } }
        stage build {
            depends_on: [setup]
            steps { mkdir "y" }
        }
        "#,
    );
    let deps = result.dependency_graph.get("build").expect("build in graph");
    assert_eq!(deps, &vec!["setup".to_string()]);
}

#[test]
fn depends_on_merges_with_inferred_edges_without_duplicates() {
    let result = analyze_ok(
        r#"
        stage setup   { steps { mkdir "x" } }
        stage compile { steps { mkdir "y" } }
        stage build {
            inputs: [compile.outputs]
            depends_on: [setup, compile]
            steps { mkdir "z" }
        }
        "#,
    );
    let mut deps = result.dependency_graph.get("build").unwrap().clone();
    deps.sort();
    // `compile` appears via both inputs and depends_on but is listed once.
    assert_eq!(deps, vec!["compile".to_string(), "setup".to_string()]);
}

#[test]
fn depends_on_unknown_stage_errors() {
    let diags = analyze_err(
        r#"
        stage build {
            depends_on: [nope]
            steps { mkdir "y" }
        }
        "#,
    );
    assert!(has_msg(&diags, "unknown stage 'nope' in depends_on"));
}

#[test]
fn depends_on_self_errors() {
    let diags = analyze_err(
        r#"
        stage build {
            depends_on: [build]
            steps { mkdir "y" }
        }
        "#,
    );
    assert!(has_msg(&diags, "cannot depend on itself"));
}

#[test]
fn always_run_and_run_once_conflict_errors() {
    let diags = analyze_err(
        r#"
        stage setup {
            always_run: true
            run_once: true
            steps { mkdir "x" }
        }
        "#,
    );
    assert!(has_msg(&diags, "both always_run and run_once"));
}

#[test]
fn always_run_alone_ok() {
    analyze_ok(r#"stage act { always_run: true steps { mkdir "x" } }"#);
}

#[test]
fn run_once_alone_ok() {
    analyze_ok(r#"stage setup { run_once: true steps { mkdir "x" } }"#);
}

#[test]
fn depends_on_cycle_errors() {
    let diags = analyze_err(
        r#"
        stage a { depends_on: [b] steps { mkdir "x" } }
        stage b { depends_on: [a] steps { mkdir "y" } }
        "#,
    );
    assert!(has_msg(&diags, "dependency cycle"));
}

#[test]
fn mixed_inputs_and_depends_on_cycle_errors() {
    // a → b via depends_on, b → a via inferred inputs edge; the combined graph cycles.
    let diags = analyze_err(
        r#"
        stage a {
            depends_on: [b]
            steps { mkdir "x" }
        }
        stage b {
            inputs: [a.outputs]
            steps { mkdir "y" }
        }
        "#,
    );
    assert!(has_msg(&diags, "dependency cycle"));
}
