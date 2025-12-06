use std::fs;

// Integration tests that verify `read` and globbing resolve paths relative
// to the script file's directory by running the full pipeline in-process.

#[test]
fn test_read_resolves_relative_to_script_dir() {
    let td = tempfile::tempdir().expect("tempdir");
    let dir = td.path();
    let file_path = dir.join("data.txt");
    fs::write(&file_path, "hello-world\n").expect("write file");

    let script_src = r#"
[entrypoint]
workspace w { projects = []; }

stage main() {
    val = read("data.txt");
    say(val[0]);
}
"#;

    let script_file = dir.join("script.ms");
    fs::write(&script_file, script_src).expect("write script");

    let script = mainstage_core::script::Script::new(script_file.clone()).expect("Failed to load script file");
    let mut ast = match mainstage_core::ast::generate_ast_from_source(&script) {
        Ok(a) => a,
        Err(diags) => panic!("generate ast diags: {:?}", diags),
    };
    let analysis = match mainstage_core::analyzers::semantic::analyze_semantic_rules(&mut ast, None) {
        Ok((_,a)) => a,
        Err(diags) => panic!("analysis diags: {:?}", diags),
    };
    mainstage_core::analyzers::acyclic::analyze_acyclic_rules(&ast).expect("acyclic");
    let ir_module = mainstage_core::ir::lower_ast_to_ir(&ast, false, Some(&analysis));
    let bytecode = mainstage_core::ir::emit_bytecode(&ir_module);

    let result = mainstage_core::VM::new(bytecode).run(true);
    assert!(result.is_ok(), "VM run failed: {:?}", result.err());
}

#[test]
fn test_glob_resolves_relative_to_script_dir() {
    let td = tempfile::tempdir().expect("tempdir");
    let dir = td.path();
    let file1 = dir.join("a.ms");
    let file2 = dir.join("b.ms");
    fs::write(&file1, "content-a").expect("write a");
    fs::write(&file2, "content-b").expect("write b");

    let script_src = r#"
[entrypoint]
workspace w { projects = []; }

stage L1(var) {
    val = read(var);
    say(val[0]);
}

stage main() {
    sources = ["*.ms"];
    L1(sources[0]);
}
"#;

    let script_file = dir.join("script.ms");
    fs::write(&script_file, script_src).expect("write script");

    let script = mainstage_core::script::Script::new(script_file.clone()).expect("Failed to load script file");
    let mut ast = match mainstage_core::ast::generate_ast_from_source(&script) {
        Ok(a) => a,
        Err(diags) => panic!("generate ast diags: {:?}", diags),
    };
    let analysis = match mainstage_core::analyzers::semantic::analyze_semantic_rules(&mut ast, None) {
        Ok((_,a)) => a,
        Err(diags) => panic!("generate ast diags: {:?}", diags),
    };
    mainstage_core::analyzers::acyclic::analyze_acyclic_rules(&ast).expect("acyclic");
    let ir_module = mainstage_core::ir::lower_ast_to_ir(&ast, false, Some(&analysis));
    let bytecode = mainstage_core::ir::emit_bytecode(&ir_module);

    let result = mainstage_core::VM::new(bytecode).run(true);
    assert!(result.is_ok(), "VM run failed: {:?}", result.err());
}
