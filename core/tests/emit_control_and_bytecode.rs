use std::path::PathBuf;
use mainstage_core::{ast, ir, script::Script};

#[test]
fn lowering_if_emits_brfalse() {
    let src = r#"
stage f() { if true { return; } }
"#;
    let script = Script { name: "t.ms".to_string(), path: PathBuf::from("t.ms"), content: src.to_string() };
    let ast = ast::generate_ast_from_source(&script).expect("parse");
    let ir_mod = ir::lower_ast_to_ir(&ast, false, None);
    // Expect a BrFalse in the lowered IR
    let has_brfalse = ir_mod.get_ops().iter().any(|op| matches!(op, mainstage_core::ir::op::IROp::BrFalse { .. }));
    assert!(has_brfalse, "expected BrFalse in IR");
}

#[test]
fn lowering_while_emits_jump_back_and_brfalse() {
    let src = r#"
stage f() { while true { return; } }
"#;
    let script = Script { name: "t.ms".to_string(), path: PathBuf::from("t.ms"), content: src.to_string() };
    let ast = ast::generate_ast_from_source(&script).expect("parse");
    let ir_mod = ir::lower_ast_to_ir(&ast, false, None);
    let has_jump = ir_mod.get_ops().iter().any(|op| matches!(op, mainstage_core::ir::op::IROp::Jump { .. }));
    let has_brfalse = ir_mod.get_ops().iter().any(|op| matches!(op, mainstage_core::ir::op::IROp::BrFalse { .. }));
    assert!(has_jump, "expected Jump in IR for while loop");
    assert!(has_brfalse, "expected BrFalse in IR for while loop");
}

#[test]
fn lowering_produces_ops_for_stage_call() {
    let src = r#"
stage callee(x) { return; }
stage caller() { callee("arg"); }
"#;
    let script = Script { name: "t.ms".to_string(), path: PathBuf::from("t.ms"), content: src.to_string() };
    let ast = ast::generate_ast_from_source(&script).expect("parse");
    let ir_mod = ir::lower_ast_to_ir(&ast, false, None);

    // Ensure lowering produced some IR ops (either in module or function bodies)
    let has_any_ops = ir_mod.len() > 0;
    assert!(has_any_ops, "expected IR ops to be produced for stage call script");
}
