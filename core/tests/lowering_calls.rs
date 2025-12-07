use mainstage_core::{ast, ir, script::Script};
use std::path::PathBuf;

// Sample script content (mirrors `cli/samples/e2e/2.ms`)
const SAMPLE: &str = r#"[entrypoint]
workspace demo_ws {
    projects = [test_pj];

    for p in projects
    {
        say(test_pj.sources);
        process_project_stage(p);
    }
}

project test_pj {
    sources = ["./samples/e2e/*.ms"];
}

stage load_stage(var)
{
    return read(var);
}

stage process_project_stage(prj)
{
    if prj.sources == null {
        say("No sources found.");
        return;
    }
    in = load_stage(prj.sources[0]);
    say(in);
}
"#;

#[test]
fn calllabel_args_are_present_after_lowering() {
    // Construct a Script value with the sample content
    let script = Script {
        name: "test.ms".to_string(),
        path: PathBuf::from("test.ms"),
        content: SAMPLE.to_string(),
    };

    // Parse AST
    let ast = ast::generate_ast_from_source(&script).expect("failed to parse sample");

    // Lower AST to IR targeting the workspace `demo_ws`
    let ir_mod = ir::lower_ast_to_ir(&ast, false, None);

    // Debug print IR if test fails
    let ir_str = format!("{}", ir_mod);

    // Ensure every CallLabel op has at least one arg (calls from workspace/stages)
    let mut found = false;
    for op in ir_mod.ops.iter() {
        if let mainstage_core::ir::op::IROp::CallLabel {
            dest: _,
            label_index: _,
            args,
        } = op
        {
            found = true;
            assert!(
                !args.is_empty(),
                "CallLabel emitted with no args: {}",
                ir_str
            );
        }
    }
    assert!(found, "No CallLabel ops found in lowered IR: {}", ir_str);
}
