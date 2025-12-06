//! Lowering helpers for statement nodes.
//!
//! This module contains routines that lower AST statement nodes into
//! the IR representation (`IrModule`). The functions here are called
//! by the lowering pipeline and should not perform IO or runtime work.
//!
//! See also: `ir::lower::lower_expr` for expression lowering helpers.

pub fn lower_statment(
    stmt_node: &crate::ast::AstNode,
    ir_mod: &mut crate::ir::module::IrModule,
    ctx: &super::lowering_context::LoweringContext,
) {
    use crate::ast::AstNodeKind;

    match stmt_node.get_kind() {
        AstNodeKind::Call { .. } => {
            // If workspace lowering has requested suppression of module-level
            // emissions (we're collecting body statements to move into a
            // wrapper), skip emitting calls at module scope here.
            if ctx.module_emits_suppressed() {
                return;
            }
            // Reuse the same lowering as in the old top-level walker: call lowering
            // into module-level registers.
            if let AstNodeKind::Call { callee, args } = stmt_node.get_kind() {
                if let AstNodeKind::Identifier { name } = callee.get_kind() {
                    if ctx.symbols.get(name).is_some() {
                        let mut regs: Vec<usize> = Vec::new();
                        for arg in args.iter() {
                            let r = super::lower_expr::lower_expr_to_reg_with_builder(arg, ir_mod, ctx, None);
                            regs.push(r);
                        }
                        let _name = name.clone();
                    }
                }
            }
        }
        AstNodeKind::Return { value } => {
            if let Some(v) = value {
                let r = super::lower_expr::lower_expr_to_reg_with_builder(v, ir_mod, ctx, None);
                ir_mod.emit_op(crate::ir::op::IROp::Ret { src: r });
            } else {
                let r = ir_mod.alloc_reg();
                ir_mod.emit_op(crate::ir::op::IROp::LConst { dest: r, value: crate::ir::value::Value::Null });
                ir_mod.emit_op(crate::ir::op::IROp::Ret { src: r });
            }
        }
        AstNodeKind::Block { statements } => {
            for s in statements.iter() { lower_statment(s, ir_mod, ctx); }
        }
        AstNodeKind::If { condition, body } => {
            lower_statment(condition, ir_mod, ctx);
            lower_statment(body, ir_mod, ctx);
        }
        AstNodeKind::IfElse { condition, if_body, else_body } => {
            lower_statment(condition, ir_mod, ctx);
            lower_statment(if_body, ir_mod, ctx);
            lower_statment(else_body, ir_mod, ctx);
        }
        AstNodeKind::ForIn { iterator: _, iterable, body: _ } => {
            // When lowering at module scope, only lower the iterable
            // expression (so array constants are produced). Do not lower
            // the loop body here — workspace-level lowering will lower
            // the body into wrapper functions where appropriate.
            lower_statment(iterable, ir_mod, ctx);
        }
        AstNodeKind::ForTo { initializer, limit, body } => {
            lower_statment(initializer, ir_mod, ctx);
            lower_statment(limit, ir_mod, ctx);
            lower_statment(body, ir_mod, ctx);
        }
        AstNodeKind::While { condition, body } => {
            lower_statment(condition, ir_mod, ctx);
            lower_statment(body, ir_mod, ctx);
        }
        AstNodeKind::Assignment { target, value } => {
            // top-level assignment: handle member sets, otherwise evaluate value and ignore target
            match target.get_kind() {
                crate::ast::AstNodeKind::Member { object, property } => {
                    // Prefer module-level bound object runtime register when
                    // available (projects/workspaces). Fall back to evaluating
                    // the object expression into a register (which may be a
                    // Symbol const) if no bound runtime slot is known.
                    let obj_reg = match object.get_kind() {
                        crate::ast::AstNodeKind::Identifier { name } => {
                            if let Some(reg) = ctx.get_object_reg(object.get_id()) {
                                reg
                            } else if let Some(obj_id) = ctx.symbols.get(name).copied() {
                                if let Some(reg2) = ctx.get_object_reg_by_objid(obj_id) {
                                    reg2
                                } else {
                                                super::lower_expr::lower_expr_to_reg_helper(object, ir_mod, Some(ctx))
                                    }
                            } else {
                                super::lower_expr::lower_expr_to_reg_helper(object, ir_mod, Some(ctx))
                            }
                        }
                        _ => super::lower_expr::lower_expr_to_reg_helper(object, ir_mod, Some(ctx)),
                    };
                    let key_reg = ir_mod.alloc_reg();
                    ir_mod.emit_op(IROp::LConst { dest: key_reg, value: crate::ir::value::Value::Symbol(property.clone()) });
                    let val_reg = super::lower_expr::lower_expr_to_reg_helper(value, ir_mod, Some(ctx));
                    ir_mod.emit_op(IROp::SetProp { obj: obj_reg, key: key_reg, src: val_reg });
                }
                _ => {
                    // top-level assignment: lower value then ignore target (globals not implemented)
                    lower_statment(value, ir_mod, ctx);
                }
            }
        }
        AstNodeKind::UnaryOp { expr, .. } => { let _ = super::lower_expr::lower_expr_to_reg_with_builder(expr, ir_mod, ctx, None); }
        AstNodeKind::BinaryOp { left, right, .. } => { lower_statment(left, ir_mod, ctx); lower_statment(right, ir_mod, ctx); }
        _ => {}
    }
}

use crate::ir::op::IROp;

/// Builder-aware walker that mirrors `emit_calls_in_node` but routes
/// allocations/emits into the provided `FunctionBuilder` so function-local
/// ops stay grouped.
pub fn emit_calls_in_node_with_builder(
    node: &crate::ast::AstNode,
    fb: &mut super::function_builder::FunctionBuilder,
    ir_mod: &mut crate::ir::module::IrModule,
    ctx: &super::lowering_context::LoweringContext,
) {
    use crate::ast::AstNodeKind;

    match node.get_kind() {
        k if k.container_body().is_some() => {
            if let Some(body) = k.container_body() {
                // nested containers: descend, keep using same builder
                emit_calls_in_node_with_builder(body, fb, ir_mod, ctx);
                return;
            }
        }
        AstNodeKind::Call { callee, args } => {
            // If callee is a simple identifier, handle as before.
            if let AstNodeKind::Identifier { name } = callee.get_kind() {
                // Lower bare identifier calls either when present in symbols
                // or when matching a known stdlib function name.
                let mut regs: Vec<usize> = Vec::new();
                for arg in args.iter() {
                    let r = super::lower_expr::lower_expr_to_reg_with_builder(arg, ir_mod, ctx, Some(fb));
                    regs.push(r);
                }
                // Consult lowering context plugin function registry for bare name calls.
                let candidates = ctx.lookup_plugin_func(name);
                if candidates.len() == 1 {
                    let (plugin_name, qualified) = candidates[0].clone();
                    fb.emit_op(IROp::PluginCall { dest: None, plugin_name, func_name: qualified, args: regs });
                } else if candidates.len() > 1 {
                    log::error!("lowering: ambiguous bare function '{}' resolves to multiple plugins; specify a domain alias.", name);
                }
            } else {
                // Fallback: evaluate the full call expression (member-style calls,
                // plugin calls, etc.) into a temporary register using the
                // expression lowering helper; that helper will emit the proper
                // IROp (including PluginCall) into this builder when applicable.
                let _ = super::lower_expr::lower_expr_to_reg_with_builder(node, ir_mod, ctx, Some(fb));
            }
        }
        AstNodeKind::Return { value } => {
            if let Some(v) = value {
                let r = super::lower_expr::lower_expr_to_reg_with_builder(v, ir_mod, ctx, Some(fb));
                fb.emit_op(IROp::Ret { src: r });
            } else {
                let r = fb.alloc_reg();
                fb.emit_op(IROp::LConst { dest: r, value: crate::ir::value::Value::Null });
                fb.emit_op(IROp::Ret { src: r });
            }
        }
        AstNodeKind::Block { statements } => {
            for s in statements.iter() { emit_calls_in_node_with_builder(s, fb, ir_mod, ctx); }
        }
        AstNodeKind::If { condition, body } => {
            let cond_reg = super::lower_expr::lower_expr_to_reg_with_builder(condition, ir_mod, ctx, Some(fb));
            // emit placeholder BrFalse
            let br_pos = fb.current_len();
            fb.emit_op(IROp::BrFalse { cond: cond_reg, target: 0 });
            // body
            emit_calls_in_node_with_builder(body, fb, ir_mod, ctx);
            // patch placeholder to jump to next op after body
            let after = fb.current_len();
            fb.patch_op(br_pos, IROp::BrFalse { cond: cond_reg, target: after });
        }
        AstNodeKind::IfElse { condition, if_body, else_body } => {
            let cond_reg = super::lower_expr::lower_expr_to_reg_with_builder(condition, ir_mod, ctx, Some(fb));
            let br_to_else = fb.current_len();
            fb.emit_op(IROp::BrFalse { cond: cond_reg, target: 0 });
            // if body
            emit_calls_in_node_with_builder(if_body, fb, ir_mod, ctx);
            // jump over else
            let jmp_pos = fb.current_len();
            fb.emit_op(IROp::Jump { target: 0 });
            // else start
            let else_start = fb.current_len();
            fb.patch_op(br_to_else, IROp::BrFalse { cond: cond_reg, target: else_start });
            // else body
            emit_calls_in_node_with_builder(else_body, fb, ir_mod, ctx);
            // after else
            let after = fb.current_len();
            fb.patch_op(jmp_pos, IROp::Jump { target: after });
        }
        AstNodeKind::ForIn { iterable, body, .. } => {
            // Lower for-in into an index-based loop with a real local binding
            // for the iterator variable so body lowering can reference it.
            // Evaluate the iterable into a register (builder-aware).
            let arr_reg = super::lower_expr::lower_expr_to_reg_with_builder(iterable, ir_mod, ctx, Some(fb));

            // idx = 0
            let idx_reg = fb.alloc_reg();
            fb.emit_op(IROp::LConst { dest: idx_reg, value: crate::ir::value::Value::Int(0) });

            // key = "length"; len = GetProp arr[key]
            let key_reg = fb.alloc_reg();
            fb.emit_op(IROp::LConst { dest: key_reg, value: crate::ir::value::Value::Str("length".to_string()) });
            let len_reg = fb.alloc_reg();
            fb.emit_op(IROp::GetProp { dest: len_reg, obj: arr_reg, key: key_reg });

            // loop condition: cmp = idx < len
            let loop_cond_pos = fb.current_len();
            let cmp_reg = fb.alloc_reg();
            fb.emit_op(IROp::Lt { dest: cmp_reg, src1: idx_reg, src2: len_reg });
            // placeholder BrFalse to be patched after body
            let br_pos = fb.current_len();
            fb.emit_op(IROp::BrFalse { cond: cmp_reg, target: 0 });
            // loop condition emitted

            // body: item = ArrayGet arr[idx]
            let item_reg = fb.alloc_reg();
            fb.emit_op(IROp::ArrayGet { dest: item_reg, array: arr_reg, index: idx_reg });

            // Bind iterator name as a local and store the item into it
            let iterator_name = if let crate::ast::AstNodeKind::ForIn { iterator, .. } = node.get_kind() { iterator.clone() } else { String::new() };
            let local_idx = fb.get_or_create_local(&iterator_name);
            fb.emit_op(IROp::SLocal { src: item_reg, local_index: local_idx });

            // Lower the loop body in the same builder so identifier refs map to locals
            emit_calls_in_node_with_builder(body, fb, ir_mod, ctx);

            // increment idx: idx = idx + 1
            let one_reg = fb.alloc_reg();
            fb.emit_op(IROp::LConst { dest: one_reg, value: crate::ir::value::Value::Int(1) });
            fb.emit_op(IROp::Add { dest: idx_reg, src1: idx_reg, src2: one_reg });

            // jump back to condition
            fb.emit_op(IROp::Jump { target: loop_cond_pos });

            // patch BrFalse to jump here (after loop)
            let after = fb.current_len();
            // patched for-in loop end
            fb.patch_op(br_pos, IROp::BrFalse { cond: cmp_reg, target: after });
        }
        AstNodeKind::ForTo { initializer, limit, body } => {
            let _i = super::lower_expr::lower_expr_to_reg_with_builder(initializer, ir_mod, ctx, Some(fb));
            let _l = super::lower_expr::lower_expr_to_reg_with_builder(limit, ir_mod, ctx, Some(fb));
            emit_calls_in_node_with_builder(body, fb, ir_mod, ctx);
        }
        AstNodeKind::While { condition, body } => {
            // loop start
            let loop_start = fb.current_len();
            let cond_reg = super::lower_expr::lower_expr_to_reg_with_builder(condition, ir_mod, ctx, Some(fb));
            let br_pos = fb.current_len();
            fb.emit_op(IROp::BrFalse { cond: cond_reg, target: 0 });
            emit_calls_in_node_with_builder(body, fb, ir_mod, ctx);
            // jump back to loop start
            fb.emit_op(IROp::Jump { target: loop_start });
            let after = fb.current_len();
            fb.patch_op(br_pos, IROp::BrFalse { cond: cond_reg, target: after });
        }
        AstNodeKind::Assignment { target: tgt, value } => {
            // Handle simple local assignment: `ident = expr`.
            match tgt.get_kind() {
                crate::ast::AstNodeKind::Identifier { name } => {
                    let val_reg = super::lower_expr::lower_expr_to_reg_with_builder(value, ir_mod, ctx, Some(fb));
                    let local_idx = fb.get_or_create_local(name);
                    fb.emit_op(IROp::SLocal { src: val_reg, local_index: local_idx });
                }
                crate::ast::AstNodeKind::Member { object, property } => {
                    // obj.prop = value -> emit SetProp
                    let obj_reg = super::lower_expr::lower_expr_to_reg_with_builder(object, ir_mod, ctx, Some(fb));
                    // create key register as a symbol const
                    let key_reg = {
                        let r = fb.alloc_reg();
                        fb.emit_op(IROp::LConst { dest: r, value: crate::ir::value::Value::Symbol(property.clone()) });
                        r
                    };
                    let val_reg = super::lower_expr::lower_expr_to_reg_with_builder(value, ir_mod, ctx, Some(fb));
                    fb.emit_op(IROp::SetProp { obj: obj_reg, key: key_reg, src: val_reg });
                }
                _ => {
                    // Fallback: evaluate both sides to preserve side-effects
                    emit_calls_in_node_with_builder(tgt, fb, ir_mod, ctx);
                    emit_calls_in_node_with_builder(value, fb, ir_mod, ctx);
                }
            }
        }
        AstNodeKind::UnaryOp { expr, .. } => { let _ = super::lower_expr::lower_expr_to_reg_with_builder(expr, ir_mod, ctx, Some(fb)); }
        AstNodeKind::BinaryOp { left, op, right } => {
            let l = super::lower_expr::lower_expr_to_reg_with_builder(left, ir_mod, ctx, Some(fb));
            let r = super::lower_expr::lower_expr_to_reg_with_builder(right, ir_mod, ctx, Some(fb));
            let dest = fb.alloc_reg();
            match op {
                crate::ast::BinaryOperator::Eq => fb.emit_op(IROp::Eq { dest, src1: l, src2: r }),
                crate::ast::BinaryOperator::Ne => fb.emit_op(IROp::Neq { dest, src1: l, src2: r }),
                crate::ast::BinaryOperator::Lt => fb.emit_op(IROp::Lt { dest, src1: l, src2: r }),
                crate::ast::BinaryOperator::Le => fb.emit_op(IROp::Lte { dest, src1: l, src2: r }),
                crate::ast::BinaryOperator::Gt => fb.emit_op(IROp::Gt { dest, src1: l, src2: r }),
                crate::ast::BinaryOperator::Ge => fb.emit_op(IROp::Gte { dest, src1: l, src2: r }),
                crate::ast::BinaryOperator::Add => fb.emit_op(IROp::Add { dest, src1: l, src2: r }),
                crate::ast::BinaryOperator::Sub => fb.emit_op(IROp::Sub { dest, src1: l, src2: r }),
                crate::ast::BinaryOperator::Mul => fb.emit_op(IROp::Mul { dest, src1: l, src2: r }),
                crate::ast::BinaryOperator::Div => fb.emit_op(IROp::Div { dest, src1: l, src2: r }),
                crate::ast::BinaryOperator::Mod => fb.emit_op(IROp::Mod { dest, src1: l, src2: r }),
            }
        }
        _ => {}
    }
}