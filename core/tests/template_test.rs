use std::path::PathBuf;
use mainstage_core::{ast, analyzers, ir, script::Script};

// A template integration test demonstrating the full pipeline:
// 1) parse AST, 2) semantic analysis, 3) lower to IR, 4) emit bytecode, 5) run VM
#[test]
fn template_pipeline_smoke() {
    let src = r#"
[entrypoint]
workspace w {
    projects = [];
}

    stage main() {
    a = 3;
    say(a);
}
"#;

    let script = Script { name: "t.ms".to_string(), path: PathBuf::from("t.ms"), content: src.to_string() };
    let mut ast = ast::generate_ast_from_source(&script).expect("parse");
    let analysis = match analyzers::semantic::analyze_semantic_rules(&mut ast, None) {
        Ok((_,a)) => a,
        Err(diags) => panic!("analysis diags: {:?}", diags),
    };
    analyzers::acyclic::analyze_acyclic_rules(&ast).expect("acyclic");

    let ir_mod = ir::lower_ast_to_ir(&ast, false, Some(&analysis));
    let bytes = ir::emit_bytecode(&ir_mod);

    // Run VM (no plugins required for this simple script)
    let result = mainstage_core::VM::new(bytes).run(false);
    assert!(result.is_ok(), "VM run failed: {:?}", result.err());
}
