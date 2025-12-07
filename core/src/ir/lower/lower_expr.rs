//! file: core/src/ir/lower/lower_expr.rs
//! description: expression lowering helpers.
//!
//! Helpers to lower expression AST nodes into IR registers. The functions
//! here are used by higher-level lowering passes (statement and module
//! lowering) and support both module-level emission and `FunctionBuilder`
//! usage.
//!
use crate::ir::op::IROp;

/// Lower an assignment statement node at top-level. Currently this evaluates
/// the right-hand expression and drops the result (globals not implemented).
pub fn lower_assignment_expr(
    assign_node: &crate::ast::AstNode,
    ir_mod: &mut crate::ir::module::IrModule,
) {
    if let crate::ast::AstNodeKind::Assignment { target: _, value } = assign_node.get_kind() {
        let v = value.as_ref();
        // evaluate into a temporary register and ignore
        let _ = lower_expr_to_reg_helper(v, ir_mod, None);
    }
}

/// Helper used by the old top-level path: evaluate an expression into a module-level register.
pub fn lower_expr_to_reg_helper(
    expr: &crate::ast::AstNode,
    ir_mod: &mut crate::ir::module::IrModule,
    ctx_opt: Option<&super::lowering_context::LoweringContext>,
) -> usize {
    use crate::ast::AstNodeKind;
    match expr.get_kind() {
        AstNodeKind::String { value } => {
            let r = ir_mod.alloc_reg();
            ir_mod.emit_op(IROp::LConst {
                dest: r,
                value: crate::ir::value::Value::Str(value.clone()),
            });
            r
        }
        AstNodeKind::Integer { value } => {
            let r = ir_mod.alloc_reg();
            ir_mod.emit_op(IROp::LConst {
                dest: r,
                value: crate::ir::value::Value::Int(*value),
            });
            r
        }
        AstNodeKind::Float { value } => {
            let r = ir_mod.alloc_reg();
            ir_mod.emit_op(IROp::LConst {
                dest: r,
                value: crate::ir::value::Value::Float(*value),
            });
            r
        }
        AstNodeKind::Bool { value } => {
            let r = ir_mod.alloc_reg();
            ir_mod.emit_op(IROp::LConst {
                dest: r,
                value: crate::ir::value::Value::Bool(*value),
            });
            r
        }
        AstNodeKind::Identifier { name } => {
            // If a temporary identifier binding exists (e.g. loop iterator),
            // return that runtime register directly.
            if let Some(ctx) = ctx_opt {
                if let Some(r) = ctx.get_temp_ident(name) {
                    return r;
                }
                // If the identifier names a declared module-level object
                // (workspace/project), return its runtime register directly
                // so property access targets the real object rather than a
                // Symbol value.
                if let Some(obj_id) = ctx.symbols.get(name).copied()
                    && let Some(reg) = ctx.get_object_reg_by_objid(obj_id)
                {
                    return reg;
                }
            }
            let r = ir_mod.alloc_reg();
            ir_mod.emit_op(IROp::LConst {
                dest: r,
                value: crate::ir::value::Value::Symbol(name.clone()),
            });
            r
        }

        _ => {
            let r = ir_mod.alloc_reg();
            ir_mod.emit_op(IROp::LConst {
                dest: r,
                value: crate::ir::value::Value::Null,
            });
            r
        }
    }
}

/// Lower an expression node into a register using an optional `FunctionBuilder`.
pub fn lower_expr_to_reg_with_builder(
    expr: &crate::ast::AstNode,
    ir_mod: &mut crate::ir::module::IrModule,
    _ctx: &super::lowering_context::LoweringContext,
    mut builder: Option<&mut super::function_builder::FunctionBuilder>,
) -> usize {
    use crate::ast::AstNodeKind;
    match expr.get_kind() {
        AstNodeKind::Call { callee, args } => {
            // simple identifier callee -> CallLabel
            if let AstNodeKind::Identifier { name } = callee.get_kind() {
                // Lower bare identifier calls either when present in symbols
                // or when matching a known stdlib function name.
                let mut regs = Vec::new();
                for a in args.iter() {
                    let builder_arg = builder.as_deref_mut();
                    let r = lower_expr_to_reg_with_builder(a, ir_mod, _ctx, builder_arg);
                    regs.push(r);
                }
                // Consult lowering context plugin function registry for bare name calls.
                let candidates = _ctx.lookup_plugin_func(name);
                if candidates.len() == 1 {
                    let (plugin_name, qualified) = candidates[0].clone();
                    if let Some(b) = builder.as_mut() {
                        let dest = b.alloc_reg();
                        b.emit_op(IROp::PluginCall {
                            dest: Some(dest),
                            plugin_name,
                            func_name: qualified,
                            args: regs,
                        });
                        return dest;
                    } else {
                        let dest = ir_mod.alloc_reg();
                        ir_mod.emit_op(IROp::PluginCall {
                            dest: Some(dest),
                            plugin_name,
                            func_name: qualified,
                            args: regs,
                        });
                        return dest;
                    }
                } else if candidates.len() > 1 {
                    // ambiguous: require domain alias to disambiguate
                    log::error!(
                        "lowering: ambiguous bare function '{}' resolves to multiple plugins; specify a domain alias.",
                        name
                    );
                    // evaluate args for side-effects and return Null
                    for a in args.iter() {
                        let builder_arg = builder.as_deref_mut();
                        let _ = lower_expr_to_reg_with_builder(a, ir_mod, _ctx, builder_arg);
                    }
                    if let Some(b) = builder.as_mut() {
                        let r = b.alloc_reg();
                        b.emit_op(IROp::LConst {
                            dest: r,
                            value: crate::ir::value::Value::Null,
                        });
                        return r;
                    } else {
                        let r = ir_mod.alloc_reg();
                        ir_mod.emit_op(IROp::LConst {
                            dest: r,
                            value: crate::ir::value::Value::Null,
                        });
                        return r;
                    }
                }
                // If not a stdlib bare name, but symbol exists, lower as stage call.
                if let Some(&fid) = _ctx.symbols.get(name) {
                    let label_idx = (fid as usize).saturating_sub(1);
                    // Emit CallLabel with args and return dest register
                    if let Some(b) = builder.as_deref_mut() {
                        let dest = b.alloc_reg();
                        b.emit_op(IROp::CallLabel {
                            dest,
                            label_index: label_idx,
                            args: regs.clone(),
                        });
                        return dest;
                    } else {
                        let dest = ir_mod.alloc_reg();
                        ir_mod.emit_op(IROp::CallLabel {
                            dest,
                            label_index: label_idx,
                            args: regs.clone(),
                        });
                        return dest;
                    }
                }
            }
            // Member-style callee: resolve nested domain names like util.array.append
            if let crate::ast::AstNodeKind::Member { .. } = callee.get_kind() {
                // Walk the Member chain to build a qualified name.
                fn collect_member_chain<'a>(
                    n: &'a crate::ast::AstNode,
                    out: &mut Vec<String>,
                ) -> Option<&'a crate::ast::AstNode> {
                    match n.get_kind() {
                        crate::ast::AstNodeKind::Member { object, property } => {
                            out.push(property.clone());
                            collect_member_chain(object, out)
                        }
                        crate::ast::AstNodeKind::Identifier { .. } => Some(n),
                        _ => None,
                    }
                }

                let mut segments: Vec<String> = Vec::new();
                if let Some(root) = collect_member_chain(callee, &mut segments)
                    && let crate::ast::AstNodeKind::Identifier { name: base } = root.get_kind()
                {
                    segments.reverse();
                    let qualified = if segments.is_empty() {
                        base.clone()
                    } else {
                        format!("{}.{}", base, segments.join("."))
                    };

                    // Try qualified lookup in plugin registry
                    // If root is an alias, translate to domain-qualified using alias_to_plugin
                    let mut resolved_plugin: Option<String> = None;
                    let mut resolved_qualified: Option<String> = None;
                    if let Some(pname) = _ctx.alias_to_plugin.get(base) {
                        // rewrite qualified by replacing alias with plugin domain name
                        let domain_pref = pname.clone();
                        // domain_pref should match the manifest domain root (e.g., 'cpp' or 'stdlib')
                        // segments already contains subdomains+func
                        let dq = if segments.is_empty() {
                            domain_pref.clone()
                        } else {
                            format!("{}.{}", domain_pref, segments.join("."))
                        };
                        if let Some((plugin_name, func_qualified)) =
                            _ctx.lookup_plugin_func_qualified(&dq)
                        {
                            resolved_plugin = Some(plugin_name);
                            resolved_qualified = Some(func_qualified);
                        }
                    } else if let Some((plugin_name, func_qualified)) =
                        _ctx.lookup_plugin_func_qualified(&qualified)
                    {
                        resolved_plugin = Some(plugin_name);
                        resolved_qualified = Some(func_qualified);
                    }

                    if let (Some(plugin_name), Some(func_qualified)) =
                        (resolved_plugin, resolved_qualified)
                    {
                        // lower args
                        let mut regs: Vec<usize> = Vec::new();
                        for a in args.iter() {
                            let builder_arg = builder.as_deref_mut();
                            let r = lower_expr_to_reg_with_builder(a, ir_mod, _ctx, builder_arg);
                            regs.push(r);
                        }
                        if let Some(b) = builder.as_mut() {
                            let dest = b.alloc_reg();
                            b.emit_op(IROp::PluginCall {
                                dest: Some(dest),
                                plugin_name,
                                func_name: func_qualified,
                                args: regs,
                            });
                            return dest;
                        } else {
                            let dest = ir_mod.alloc_reg();
                            ir_mod.emit_op(IROp::PluginCall {
                                dest: Some(dest),
                                plugin_name,
                                func_name: func_qualified,
                                args: regs,
                            });
                            return dest;
                        }
                    } else {
                        // Fallback: resolve by bare function name (last segment)
                        let bare = segments
                            .last()
                            .cloned()
                            .unwrap_or_else(|| qualified.clone());
                        let candidates = _ctx.lookup_plugin_func(&bare);
                        if candidates.len() == 1 {
                            let (plugin_name, func_qualified) = candidates[0].clone();
                            let mut regs: Vec<usize> = Vec::new();
                            for a in args.iter() {
                                let builder_arg = builder.as_deref_mut();
                                let r =
                                    lower_expr_to_reg_with_builder(a, ir_mod, _ctx, builder_arg);
                                regs.push(r);
                            }
                            if let Some(b) = builder.as_mut() {
                                let dest = b.alloc_reg();
                                b.emit_op(IROp::PluginCall {
                                    dest: Some(dest),
                                    plugin_name,
                                    func_name: func_qualified,
                                    args: regs,
                                });
                                return dest;
                            } else {
                                let dest = ir_mod.alloc_reg();
                                ir_mod.emit_op(IROp::PluginCall {
                                    dest: Some(dest),
                                    plugin_name,
                                    func_name: func_qualified,
                                    args: regs,
                                });
                                return dest;
                            }
                        } else if candidates.len() > 1 {
                            log::error!(
                                "lowering: ambiguous bare function '{}' resolves to multiple plugins; specify a domain alias.",
                                bare
                            );
                            for a in args.iter() {
                                let builder_arg = builder.as_deref_mut();
                                let _ =
                                    lower_expr_to_reg_with_builder(a, ir_mod, _ctx, builder_arg);
                            }
                            if let Some(b) = builder.as_mut() {
                                let r = b.alloc_reg();
                                b.emit_op(IROp::LConst {
                                    dest: r,
                                    value: crate::ir::value::Value::Null,
                                });
                                return r;
                            } else {
                                let r = ir_mod.alloc_reg();
                                ir_mod.emit_op(IROp::LConst {
                                    dest: r,
                                    value: crate::ir::value::Value::Null,
                                });
                                return r;
                            }
                        } else {
                            // If we cannot resolve, emit an error and return Null
                            log::error!(
                                "lowering: unresolved function '{}': no matching plugin or script function",
                                qualified
                            );
                            for a in args.iter() {
                                let builder_arg = builder.as_deref_mut();
                                let _ =
                                    lower_expr_to_reg_with_builder(a, ir_mod, _ctx, builder_arg);
                            }
                            if let Some(b) = builder.as_mut() {
                                let r = b.alloc_reg();
                                b.emit_op(IROp::LConst {
                                    dest: r,
                                    value: crate::ir::value::Value::Null,
                                });
                                return r;
                            } else {
                                let r = ir_mod.alloc_reg();
                                ir_mod.emit_op(IROp::LConst {
                                    dest: r,
                                    value: crate::ir::value::Value::Null,
                                });
                                return r;
                            }
                        }
                    }
                }
            }

            // fallback: evaluate args for side-effects and return Null
            for a in args.iter() {
                let builder_arg = builder.as_deref_mut();
                let _ = lower_expr_to_reg_with_builder(a, ir_mod, _ctx, builder_arg);
            }
            if let Some(b) = builder.as_mut() {
                let r = (*b).alloc_reg();
                (*b).emit_op(IROp::LConst {
                    dest: r,
                    value: crate::ir::value::Value::Null,
                });
                r
            } else {
                let r = ir_mod.alloc_reg();
                ir_mod.emit_op(IROp::LConst {
                    dest: r,
                    value: crate::ir::value::Value::Null,
                });
                r
            }
        }
        AstNodeKind::String { value } => {
            if let Some(b) = builder.as_mut() {
                let r = b.alloc_reg();
                b.emit_op(IROp::LConst {
                    dest: r,
                    value: crate::ir::value::Value::Str(value.clone()),
                });
                r
            } else {
                let r = ir_mod.alloc_reg();
                ir_mod.emit_op(IROp::LConst {
                    dest: r,
                    value: crate::ir::value::Value::Str(value.clone()),
                });
                r
            }
        }
        AstNodeKind::BinaryOp { left, op, right } => {
            if let Some(b) = builder.as_mut() {
                let l = lower_expr_to_reg_with_builder(left, ir_mod, _ctx, Some(b));
                let r = lower_expr_to_reg_with_builder(right, ir_mod, _ctx, Some(b));
                let dest = b.alloc_reg();
                match op {
                    crate::ast::BinaryOperator::Eq => b.emit_op(IROp::Eq {
                        dest,
                        src1: l,
                        src2: r,
                    }),
                    crate::ast::BinaryOperator::Ne => b.emit_op(IROp::Neq {
                        dest,
                        src1: l,
                        src2: r,
                    }),
                    crate::ast::BinaryOperator::Lt => b.emit_op(IROp::Lt {
                        dest,
                        src1: l,
                        src2: r,
                    }),
                    crate::ast::BinaryOperator::Le => b.emit_op(IROp::Lte {
                        dest,
                        src1: l,
                        src2: r,
                    }),
                    crate::ast::BinaryOperator::Gt => b.emit_op(IROp::Gt {
                        dest,
                        src1: l,
                        src2: r,
                    }),
                    crate::ast::BinaryOperator::Ge => b.emit_op(IROp::Gte {
                        dest,
                        src1: l,
                        src2: r,
                    }),
                    crate::ast::BinaryOperator::Add => b.emit_op(IROp::Add {
                        dest,
                        src1: l,
                        src2: r,
                    }),
                    crate::ast::BinaryOperator::Sub => b.emit_op(IROp::Sub {
                        dest,
                        src1: l,
                        src2: r,
                    }),
                    crate::ast::BinaryOperator::Mul => b.emit_op(IROp::Mul {
                        dest,
                        src1: l,
                        src2: r,
                    }),
                    crate::ast::BinaryOperator::Div => b.emit_op(IROp::Div {
                        dest,
                        src1: l,
                        src2: r,
                    }),
                    crate::ast::BinaryOperator::Mod => b.emit_op(IROp::Mod {
                        dest,
                        src1: l,
                        src2: r,
                    }),
                }
                dest
            } else {
                let l = lower_expr_to_reg_with_builder(left, ir_mod, _ctx, None);
                let r = lower_expr_to_reg_with_builder(right, ir_mod, _ctx, None);
                let dest = ir_mod.alloc_reg();
                match op {
                    crate::ast::BinaryOperator::Eq => ir_mod.emit_op(IROp::Eq {
                        dest,
                        src1: l,
                        src2: r,
                    }),
                    crate::ast::BinaryOperator::Ne => ir_mod.emit_op(IROp::Neq {
                        dest,
                        src1: l,
                        src2: r,
                    }),
                    crate::ast::BinaryOperator::Lt => ir_mod.emit_op(IROp::Lt {
                        dest,
                        src1: l,
                        src2: r,
                    }),
                    crate::ast::BinaryOperator::Le => ir_mod.emit_op(IROp::Lte {
                        dest,
                        src1: l,
                        src2: r,
                    }),
                    crate::ast::BinaryOperator::Gt => ir_mod.emit_op(IROp::Gt {
                        dest,
                        src1: l,
                        src2: r,
                    }),
                    crate::ast::BinaryOperator::Ge => ir_mod.emit_op(IROp::Gte {
                        dest,
                        src1: l,
                        src2: r,
                    }),
                    crate::ast::BinaryOperator::Add => ir_mod.emit_op(IROp::Add {
                        dest,
                        src1: l,
                        src2: r,
                    }),
                    crate::ast::BinaryOperator::Sub => ir_mod.emit_op(IROp::Sub {
                        dest,
                        src1: l,
                        src2: r,
                    }),
                    crate::ast::BinaryOperator::Mul => ir_mod.emit_op(IROp::Mul {
                        dest,
                        src1: l,
                        src2: r,
                    }),
                    crate::ast::BinaryOperator::Div => ir_mod.emit_op(IROp::Div {
                        dest,
                        src1: l,
                        src2: r,
                    }),
                    crate::ast::BinaryOperator::Mod => ir_mod.emit_op(IROp::Mod {
                        dest,
                        src1: l,
                        src2: r,
                    }),
                }
                dest
            }
        }
        AstNodeKind::Integer { value } => {
            if let Some(b) = builder.as_mut() {
                let r = b.alloc_reg();
                b.emit_op(IROp::LConst {
                    dest: r,
                    value: crate::ir::value::Value::Int(*value),
                });
                r
            } else {
                let r = ir_mod.alloc_reg();
                ir_mod.emit_op(IROp::LConst {
                    dest: r,
                    value: crate::ir::value::Value::Int(*value),
                });
                r
            }
        }
        AstNodeKind::Float { value } => {
            if let Some(b) = builder.as_mut() {
                let r = b.alloc_reg();
                b.emit_op(IROp::LConst {
                    dest: r,
                    value: crate::ir::value::Value::Float(*value),
                });
                r
            } else {
                let r = ir_mod.alloc_reg();
                ir_mod.emit_op(IROp::LConst {
                    dest: r,
                    value: crate::ir::value::Value::Float(*value),
                });
                r
            }
        }
        AstNodeKind::Bool { value } => {
            if let Some(b) = builder.as_mut() {
                let r = b.alloc_reg();
                b.emit_op(IROp::LConst {
                    dest: r,
                    value: crate::ir::value::Value::Bool(*value),
                });
                r
            } else {
                let r = ir_mod.alloc_reg();
                ir_mod.emit_op(IROp::LConst {
                    dest: r,
                    value: crate::ir::value::Value::Bool(*value),
                });
                r
            }
        }
        AstNodeKind::Identifier { name } => {
            // If this identifier corresponds to a local in the function builder,
            // load it from the local slot. Otherwise represent as a Symbol const.
            if let Some(b) = builder.as_mut() {
                if let Some(local_idx) = b.lookup_local(name) {
                    let r = b.alloc_reg();
                    b.emit_op(IROp::LLocal {
                        dest: r,
                        local_index: local_idx,
                    });
                    return r;
                } else if let Some(obj_id) = _ctx.symbols.get(name).copied()
                    && let Some(mod_reg) = _ctx.get_object_reg_by_objid(obj_id)
                {
                    let r = b.alloc_reg();
                    b.emit_op(IROp::LoadGlobal {
                        dest: r,
                        src: mod_reg,
                    });
                    return r;
                }
                let r = b.alloc_reg();
                b.emit_op(IROp::LConst {
                    dest: r,
                    value: crate::ir::value::Value::Symbol(name.clone()),
                });
                r
            } else {
                let r = ir_mod.alloc_reg();
                ir_mod.emit_op(IROp::LConst {
                    dest: r,
                    value: crate::ir::value::Value::Symbol(name.clone()),
                });
                r
            }
        }
        AstNodeKind::Member { object, property } => {
            // evaluate object and property, then emit GetProp
            // evaluate object into a register (module-level helper used to avoid borrow issues)
            // If the object is a known declared object (workspace/project),
            // use its module-level runtime register from the lowering context
            // so property ops target the actual object slot rather than a
            // mere Symbol value.
            // property is a string name (AstNodeKind::Member stores property as String)
            let key_name = property.clone();
            // Create the key symbol register first. This borrows `builder` but
            // does not move it, allowing us to call builder-aware lowering for
            // the object afterwards (which may consume `builder`).
            let key_reg = if let Some(b) = builder.as_mut() {
                let r = b.alloc_reg();
                b.emit_op(IROp::LConst {
                    dest: r,
                    value: crate::ir::value::Value::Symbol(key_name.clone()),
                });
                r
            } else {
                let r = ir_mod.alloc_reg();
                ir_mod.emit_op(IROp::LConst {
                    dest: r,
                    value: crate::ir::value::Value::Symbol(key_name.clone()),
                });
                r
            };

            // Evaluate the object expression. If the object corresponds to a
            // declared module-level object, use the bound runtime register
            // from the lowering context. If it's an identifier that maps to a
            // function-local, emit an `LLocal` into the builder. Otherwise
            // fall back to the module-level helper which produces a Symbol
            // const.
            let obj_reg = match object.get_kind() {
                AstNodeKind::Identifier { name } => {
                    // Try lookup by the identifier's own AST node id first. If
                    // found and we're lowering into a FunctionBuilder, load
                    // the module-level register into a function-local via
                    // `LoadGlobal` so subsequent ops reference a local reg.
                    if let Some(reg) = _ctx.get_object_reg(object.get_id()) {
                        if let Some(b) = builder.as_mut() {
                            let r = b.alloc_reg();
                            b.emit_op(IROp::LoadGlobal { dest: r, src: reg });
                            r
                        } else {
                            reg
                        }
                    } else {
                        // If that fails, try resolving by symbol -> object id -> reg
                        if let Some(obj_id) = _ctx.symbols.get(name).copied() {
                            if let Some(reg2) = _ctx.get_object_reg_by_objid(obj_id) {
                                if let Some(b) = builder.as_mut() {
                                    let r = b.alloc_reg();
                                    b.emit_op(IROp::LoadGlobal { dest: r, src: reg2 });
                                    r
                                } else {
                                    reg2
                                }
                            } else if let Some(b) = builder.as_mut() {
                                if let Some(local_idx) = b.lookup_local(name) {
                                    let r = b.alloc_reg();
                                    b.emit_op(IROp::LLocal {
                                        dest: r,
                                        local_index: local_idx,
                                    });
                                    r
                                } else {
                                    let r = b.alloc_reg();
                                    b.emit_op(IROp::LConst {
                                        dest: r,
                                        value: crate::ir::value::Value::Symbol(name.clone()),
                                    });
                                    r
                                }
                            } else {
                                lower_expr_to_reg_helper(object, ir_mod, Some(_ctx))
                            }
                        } else if let Some(b) = builder.as_mut() {
                            if let Some(local_idx) = b.lookup_local(name) {
                                let r = b.alloc_reg();
                                b.emit_op(IROp::LLocal {
                                    dest: r,
                                    local_index: local_idx,
                                });
                                r
                            } else {
                                let r = b.alloc_reg();
                                b.emit_op(IROp::LConst {
                                    dest: r,
                                    value: crate::ir::value::Value::Symbol(name.clone()),
                                });
                                r
                            }
                        } else {
                            lower_expr_to_reg_helper(object, ir_mod, Some(_ctx))
                        }
                    }
                }
                _ => lower_expr_to_reg_helper(object, ir_mod, Some(_ctx)),
            };

            if let Some(b) = builder.as_mut() {
                let dest = b.alloc_reg();
                b.emit_op(IROp::GetProp {
                    dest,
                    obj: obj_reg,
                    key: key_reg,
                });
                dest
            } else {
                let dest = ir_mod.alloc_reg();
                ir_mod.emit_op(IROp::GetProp {
                    dest,
                    obj: obj_reg,
                    key: key_reg,
                });
                dest
            }
        }
        AstNodeKind::Index { object, index } => {
            // evaluate object and index using helper to avoid mutable-borrow recursion
            // Create the index register without moving `builder` so we can
            // still emit the following ArrayGet using the same `builder`.
            let idx_reg = match index.get_kind() {
                AstNodeKind::Integer { value } => {
                    if let Some(b) = builder.as_mut() {
                        let r = b.alloc_reg();
                        b.emit_op(IROp::LConst {
                            dest: r,
                            value: crate::ir::value::Value::Int(*value),
                        });
                        r
                    } else {
                        let r = ir_mod.alloc_reg();
                        ir_mod.emit_op(IROp::LConst {
                            dest: r,
                            value: crate::ir::value::Value::Int(*value),
                        });
                        r
                    }
                }
                AstNodeKind::Identifier { name } => {
                    if let Some(b) = builder.as_mut() {
                        if let Some(local_idx) = b.lookup_local(name) {
                            let r = b.alloc_reg();
                            b.emit_op(IROp::LLocal {
                                dest: r,
                                local_index: local_idx,
                            });
                            r
                        } else {
                            let r = b.alloc_reg();
                            b.emit_op(IROp::LConst {
                                dest: r,
                                value: crate::ir::value::Value::Symbol(name.clone()),
                            });
                            r
                        }
                    } else {
                        lower_expr_to_reg_helper(index, ir_mod, Some(_ctx))
                    }
                }
                _ => lower_expr_to_reg_helper(index, ir_mod, Some(_ctx)),
            };

            // Evaluate the object expression for the array access. Handle a
            // common nested case where the object is a `Member` (e.g.
            // `prj.sources[0]`) by lowering the inner `GetProp` here so we
            // produce the correct runtime register. Otherwise, if the object
            // is an identifier, emit an `LLocal` when available; fall back to
            // the module helper for other expressions.
            let obj_reg = match object.get_kind() {
                AstNodeKind::Member {
                    object: inner_obj,
                    property: inner_prop,
                } => {
                    // Build key reg first
                    let inner_key = inner_prop.clone();
                    let inner_key_reg = if let Some(b) = builder.as_mut() {
                        let r = b.alloc_reg();
                        b.emit_op(IROp::LConst {
                            dest: r,
                            value: crate::ir::value::Value::Symbol(inner_key.clone()),
                        });
                        r
                    } else {
                        let r = ir_mod.alloc_reg();
                        ir_mod.emit_op(IROp::LConst {
                            dest: r,
                            value: crate::ir::value::Value::Symbol(inner_key.clone()),
                        });
                        r
                    };

                    // Evaluate inner object (may be a function-local)
                    let inner_obj_reg = match inner_obj.get_kind() {
                        AstNodeKind::Identifier { name } => {
                            if let Some(b) = builder.as_mut() {
                                if let Some(local_idx) = b.lookup_local(name) {
                                    let r = b.alloc_reg();
                                    b.emit_op(IROp::LLocal {
                                        dest: r,
                                        local_index: local_idx,
                                    });
                                    r
                                } else {
                                    lower_expr_to_reg_helper(inner_obj, ir_mod, Some(_ctx))
                                }
                            } else {
                                lower_expr_to_reg_helper(inner_obj, ir_mod, Some(_ctx))
                            }
                        }
                        _ => lower_expr_to_reg_helper(inner_obj, ir_mod, Some(_ctx)),
                    };

                    // Emit GetProp into a dest reg
                    if let Some(b) = builder.as_mut() {
                        let dst = b.alloc_reg();
                        b.emit_op(IROp::GetProp {
                            dest: dst,
                            obj: inner_obj_reg,
                            key: inner_key_reg,
                        });
                        dst
                    } else {
                        let dst = ir_mod.alloc_reg();
                        ir_mod.emit_op(IROp::GetProp {
                            dest: dst,
                            obj: inner_obj_reg,
                            key: inner_key_reg,
                        });
                        dst
                    }
                }
                AstNodeKind::Identifier { name } => {
                    if let Some(b) = builder.as_mut() {
                        if let Some(local_idx) = b.lookup_local(name) {
                            let r = b.alloc_reg();
                            b.emit_op(IROp::LLocal {
                                dest: r,
                                local_index: local_idx,
                            });
                            r
                        } else {
                            lower_expr_to_reg_helper(object, ir_mod, Some(_ctx))
                        }
                    } else {
                        lower_expr_to_reg_helper(object, ir_mod, Some(_ctx))
                    }
                }
                _ => lower_expr_to_reg_helper(object, ir_mod, Some(_ctx)),
            };
            if let Some(b) = builder.as_mut() {
                let dest = b.alloc_reg();
                b.emit_op(IROp::ArrayGet {
                    dest,
                    array: obj_reg,
                    index: idx_reg,
                });
                dest
            } else {
                let dest = ir_mod.alloc_reg();
                ir_mod.emit_op(IROp::ArrayGet {
                    dest,
                    array: obj_reg,
                    index: idx_reg,
                });
                dest
            }
        }
        AstNodeKind::List { elements } => {
            let mut consts: Option<Vec<crate::ir::value::Value>> = Some(Vec::new());
            for el in elements.iter() {
                match el.get_kind() {
                    AstNodeKind::Integer { value } => consts
                        .as_mut()
                        .unwrap()
                        .push(crate::ir::value::Value::Int(*value)),
                    AstNodeKind::Float { value } => consts
                        .as_mut()
                        .unwrap()
                        .push(crate::ir::value::Value::Float(*value)),
                    AstNodeKind::Bool { value } => consts
                        .as_mut()
                        .unwrap()
                        .push(crate::ir::value::Value::Bool(*value)),
                    AstNodeKind::String { value } => consts
                        .as_mut()
                        .unwrap()
                        .push(crate::ir::value::Value::Str(value.clone())),
                    _ => {
                        consts = None;
                        break;
                    }
                }
            }
            if let Some(vec) = consts {
                if let Some(b) = builder.as_mut() {
                    let r = b.alloc_reg();
                    b.emit_op(IROp::LConst {
                        dest: r,
                        value: crate::ir::value::Value::Array(vec),
                    });
                    r
                } else {
                    let r = ir_mod.alloc_reg();
                    ir_mod.emit_op(IROp::LConst {
                        dest: r,
                        value: crate::ir::value::Value::Array(vec),
                    });
                    r
                }
            } else {
                // non-constant list: build elements into regs and emit ArrayNew
                let mut regs: Vec<usize> = Vec::new();
                for el in elements.iter() {
                    let builder_arg = builder.as_deref_mut();
                    let r = lower_expr_to_reg_with_builder(el, ir_mod, _ctx, builder_arg);
                    regs.push(r);
                }
                if let Some(b) = builder.as_mut() {
                    let dest = b.alloc_reg();
                    // convert usize regs -> Vec<Register>
                    let elems: Vec<usize> = regs.clone();
                    b.emit_op(IROp::ArrayNew { dest, elems });
                    dest
                } else {
                    let dest = ir_mod.alloc_reg();
                    let elems: Vec<usize> = regs.clone();
                    ir_mod.emit_op(IROp::ArrayNew { dest, elems });
                    dest
                }
            }
        }
        _ => {
            // Fallback: allocate a register and initialize to Null
            if let Some(b) = builder.as_mut() {
                let r = b.alloc_reg();
                b.emit_op(IROp::LConst {
                    dest: r,
                    value: crate::ir::value::Value::Null,
                });
                r
            } else {
                let r = ir_mod.alloc_reg();
                ir_mod.emit_op(IROp::LConst {
                    dest: r,
                    value: crate::ir::value::Value::Null,
                });
                r
            }
        }
    }
}
