use mainstage_core::ir::op::IROp;
use mainstage_core::ir::value::Value;
use mainstage_core::{ast, ir, script::Script};
use std::path::PathBuf;

const WORKSPACE_SAMPLE: &str = r#"[entrypoint]
workspace demo_ws {
    projects = [test_pj];

    for p in projects
    {
        process_project_stage(p);
    }
}

project test_pj {
    sources = ["./samples/e2e/*.ms"];
}

stage process_project_stage(prj)
{
    return;
}
"#;

const STAGE_SAMPLE: &str = r#"stage s() {
    arr = ["a"];
    for v in arr {
        say(v);
    }
}
"#;

#[test]
fn stage_forin_creates_local_binding() {
    let script = Script {
        name: "stage_loop.ms".to_string(),
        path: PathBuf::from("stage_loop.ms"),
        content: STAGE_SAMPLE.to_string(),
    };
    let ast = ast::generate_ast_from_source(&script).expect("failed to parse stage sample");
    let ir_mod = ir::lower_ast_to_ir(&ast, false, None);

    // Look for a local index that is both SLocal (store) and LLocal (load)
    let mut stores: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut loads: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for op in ir_mod.ops.iter() {
        match op {
            IROp::SLocal {
                src: _,
                local_index,
            } => {
                stores.insert(*local_index);
            }
            IROp::LLocal {
                dest: _,
                local_index,
            } => {
                loads.insert(*local_index);
            }
            _ => {}
        }
    }

    let intersection: Vec<usize> = stores.intersection(&loads).copied().collect();
    assert!(
        !intersection.is_empty(),
        "expected a local index that is both stored and loaded in stage loop lowering, ops:\n{}",
        ir_mod
    );
}

#[test]
fn workspace_forin_creates_wrapper_function() {
    let script = Script {
        name: "ws_loop.ms".to_string(),
        path: PathBuf::from("ws_loop.ms"),
        content: WORKSPACE_SAMPLE.to_string(),
    };
    let ast = ast::generate_ast_from_source(&script).expect("failed to parse workspace sample");
    let ir_mod = ir::lower_ast_to_ir(&ast, false, None);

    // Ensure an Array constant was emitted for the static list
    let mut found_array_const = false;
    for op in ir_mod.ops.iter() {
        if let IROp::LConst { dest: _, value } = op
            && let Value::Array(_) = value
        {
            found_array_const = true;
            break;
        }
    }
    assert!(
        found_array_const,
        "expected an Array LConst for workspace list, IR:\n{}",
        ir_mod
    );

    // Ensure there is at least one CallLabel emitted (calling the loop wrapper)
    let mut found_calllabel = false;
    for op in ir_mod.ops.iter() {
        if let IROp::CallLabel {
            dest: _,
            label_index: _,
            args,
        } = op
            && !args.is_empty()
        {
            found_calllabel = true;
            break;
        }
    }
    assert!(
        found_calllabel,
        "expected a CallLabel into loop wrapper for workspace for-in, IR:\n{}",
        ir_mod
    );
}
