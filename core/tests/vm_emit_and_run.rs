use std::fs;
use tempfile::tempdir;
use mainstage_core::{ast, analyzers, ir, script::Script};

#[test]
fn vm_emits_and_runs_simple_script() {
    let td = tempdir().expect("tempdir");
    let dir = td.path();

    let src = r#"
[entrypoint]
workspace w { projects = []; }

stage main() {
    a = 10;
    b = 32;
    c = a + b;
    say(c);
}
"#;

    let script_file = dir.join("script.ms");
    fs::write(&script_file, src).expect("write script");

    let script = Script::new(script_file.clone()).expect("Failed to load script file");
    let mut ast = ast::generate_ast_from_source(&script).expect("generate ast");
    let analysis = match analyzers::semantic::analyze_semantic_rules(&mut ast, None) {
        Ok((_,a)) => a,
        Err(diags) => panic!("analysis diags: {:?}", diags),
    };
    analyzers::acyclic::analyze_acyclic_rules(&ast).expect("acyclic");
    let ir_module = ir::lower_ast_to_ir(&ast, false, Some(&analysis));
    let bytecode = ir::emit_bytecode(&ir_module);

    let result = mainstage_core::VM::new(bytecode).run(false);
    assert!(result.is_ok(), "VM run failed: {:?}", result.err());
}
