use mainstage_core::ast::kind::AstNodeKind;
use mainstage_core::ast::node::AstNode;

#[test]
fn dump_lowered_forin_ir() {
    // Build a minimal AST:
    // Script {
    //   Workspace { name: "ws", body: Block { statements: [
    //     projects = [test_pj]
    //     for p in projects { var(p) }
    //   ]}}
    //   Stage { name: "var", body: Block { } }
    // }

    // stage body (empty)
    let stage_body = AstNode::new(AstNodeKind::Block { statements: vec![] }, None, None);
    let stage_node = AstNode::new(
        AstNodeKind::Stage {
            name: "var".to_string(),
            args: None,
            body: Box::new(stage_body),
        },
        None,
        None,
    );

    // assignment: projects = [test_pj]
    let list_elem = AstNode::new(AstNodeKind::Identifier { name: "test_pj".to_string() }, None, None);
    let list_node = AstNode::new(AstNodeKind::List { elements: vec![list_elem] }, None, None);
    // single shared identifier node for `projects` used both as assignment target and iterable
    let projects_ident = AstNode::new(AstNodeKind::Identifier { name: "projects".to_string() }, None, None);
    let assign_node = AstNode::new(AstNodeKind::Assignment { target: Box::new(projects_ident.clone()), value: Box::new(list_node) }, None, None);

    // call: var(p)
    let callee = AstNode::new(AstNodeKind::Identifier { name: "var".to_string() }, None, None);
    let arg = AstNode::new(AstNodeKind::Identifier { name: "p".to_string() }, None, None);
    let call_node = AstNode::new(AstNodeKind::Call { callee: Box::new(callee), args: vec![arg] }, None, None);

    // for p in projects { call }
    let iterable = projects_ident.clone();
    let for_body = AstNode::new(AstNodeKind::Block { statements: vec![call_node] }, None, None);
    let for_node = AstNode::new(AstNodeKind::ForIn { iterator: "p".to_string(), iterable: Box::new(iterable), body: Box::new(for_body) }, None, None);

    // workspace body block
    let ws_block = AstNode::new(AstNodeKind::Block { statements: vec![assign_node, for_node] }, None, None);
    let workspace = AstNode::new(AstNodeKind::Workspace { name: "ws".to_string(), body: Box::new(ws_block) }, None, None);

    // script
    let script = AstNode::new(AstNodeKind::Script { body: vec![workspace, stage_node] }, None, None);

    // Lower to IR using public wrapper
    let ir_mod = mainstage_core::ir::lower_ast_to_ir(&script, false, None);

    // Print the IR for inspection
    println!("Lowered IR:\n{}", ir_mod);
    // Basic assertion to ensure we produced ops
    assert!(!ir_mod.get_ops().is_empty());
}
