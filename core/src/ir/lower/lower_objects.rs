//! file: core/src/ir/lower/lower_objects.rs
//! description: top-level lowering for script/workspace/project objects.
//!
//! This module orchestrates the multi-pass lowering of a script AST into
//! module-level IR ops. It registers prototypes, handles static list
//! initialization, and emits per-workspace or per-stage wrappers as needed.
//!
use crate::ir::op::IROp;
// intentionally reference lower_stmt via the module path where needed

pub fn lower_script_objects(
    script: &crate::ast::AstNode,
    ir_mod: &mut crate::ir::module::IrModule,
    analysis: Option<&crate::analyzers::output::AnalyzerOutput>,
) {
    // Create a lowering context from analyzer output if provided so lowering
    // can use pre-resolved symbols and prototypes.
    let mut ctx = match analysis {
        Some(a) => super::lowering_context::LoweringContext::from_analyzer_output(a, ir_mod),
        None => super::lowering_context::LoweringContext::new(),
    };

    // If the script node has a body, perform lowering passes.
    if let Some(body) = match script.get_kind() {
        crate::ast::AstNodeKind::Script { body, .. } => Some(body),
        _ => None,
    } {
        // First pass: register prototypes. Register projects first so that
        // stages (which may reference project prototypes) can rely on
        // project symbols being present during later lowering.
        for stmt in body.iter() {
            if let crate::ast::AstNodeKind::Project { .. } = stmt.get_kind() {
                lower_project_object(stmt, ir_mod, &mut ctx);
            }
        }

        // Then register stages (function prototypes)
        for stmt in body.iter() {
            if let crate::ast::AstNodeKind::Stage { .. } = stmt.get_kind() {
                lower_stage_object(stmt, ir_mod, &mut ctx);
            }
        }

        // If analyzer provided entry_points, ensure any workspace entrypoints
        // are declared as functions so later lowering can emit entrypoint
        // wrappers and module-level calls. This bridges analyzer output
        // (which may mark a workspace as an entrypoint) to the IR module.
        if let Some(a) = analysis {
            for stmt in body.iter() {
                if let crate::ast::AstNodeKind::Workspace { name, .. } = stmt.get_kind()
                    && a.entry_point == stmt.get_id()
                    && ctx.get_function_id(stmt.get_id()).is_none()
                {
                    let func_name = name.clone();
                    let fid = ir_mod.declare_function(&func_name);
                    ctx.bind_function_id(stmt.get_id(), fid);
                    // also populate symbols so calls can resolve by name
                    ctx.symbols.insert(func_name, fid);
                }
            }
            // Analyzer may mark a workspace as the program entrypoint.
            // Do NOT emit a module-level `Jump` at this early stage —
            // emitting a Jump before module initializers (ArrayNew, LConst,
            // SetProp, etc.) makes those initializers unreachable. We
            // instead emit a module-level `CallLabel` into the entrypoint
            // after lowering the workspace so initializers run first.
        }

        // Second pass: walk the AST and emit simple CallLabel ops for Call nodes
        for stmt in body.iter() {
            match stmt.get_kind() {
                // Now perform workspace lowering in the second pass so that
                // project declarations and other prototypes are already in
                // the context.
                crate::ast::AstNodeKind::Workspace { .. } => {
                    lower_workspace_object(stmt, ir_mod, &mut ctx)
                }
                // Projects were handled in first pass; skip them here.
                crate::ast::AstNodeKind::Project { .. } => {
                    continue;
                }
                // Stages still need their function bodies emitted.
                _ => emit_calls_in_node(stmt, ir_mod, &ctx),
            }
        }
    }
    // Resolve any deferred branch targets (labels emitted later)
    ir_mod.patch_unresolved_branches();
    // Emit a final Halt to terminate execution cleanly
    ir_mod.emit_op(IROp::Halt);
}

fn emit_calls_in_node(
    node: &crate::ast::AstNode,
    ir_mod: &mut crate::ir::module::IrModule,
    ctx: &super::lowering_context::LoweringContext,
) {
    use super::function_builder::FunctionBuilder;

    match node.get_kind() {
        // If this kind is a container (workspace/project/stage), recurse into its body.
        // For stage nodes, create a per-function `FunctionBuilder` so registers
        // allocated while lowering the stage are local to that stage.
        k if k.container_body().is_some() => {
            if let Some(body) = k.container_body() {
                if let crate::ast::AstNodeKind::Stage { name, .. } = k {
                    // If we have a bound function id, emit the label inside the
                    // function builder so labels and body ops stay together.
                    let mut fb = FunctionBuilder::new();
                    // Pre-create locals for function parameters so VM arg seeding
                    // (which places args into frame.locals[0..]) lines up with
                    // these local indices.
                    if let Some(params) = ctx.functions_params.get(&node.get_id()) {
                        for p in params.iter() {
                            fb.get_or_create_local(p);
                        }
                    }
                    if let Some(id) = ctx.symbols.get(name).copied() {
                        let label_idx = (id as usize).saturating_sub(1);
                        let label_name = format!("L{}", label_idx);
                        fb.emit_op(IROp::Label { name: label_name });
                    }
                    // Lower the body using the function builder
                    super::lower_stmt::emit_calls_in_node_with_builder(body, &mut fb, ir_mod, ctx);
                    // finalize into the module
                    fb.finalize_into(ir_mod)
                }

                // Non-stage containers: just descend
                emit_calls_in_node(body, ir_mod, ctx)
            }
        }
        _ => {
            // Delegate all other node kinds to the statement-level lowering
            // which will use expression lowering helpers as needed.
            super::lower_stmt::lower_statment(node, ir_mod, ctx);
        }
    }
}

/// Workspace object lowering. Can contain both members and logic.
fn lower_workspace_object(
    workspace_node: &crate::ast::AstNode,
    ir_mod: &mut crate::ir::module::IrModule,
    ctx: &mut super::lowering_context::LoweringContext,
) {
    let body = workspace_node
        .get_kind()
        .container_body()
        .expect("Workspace node should have a body");

    // If this workspace has a name, declare it as an object so other lowering
    // can refer to it (and so analyzer-backed contexts map names -> ids).
    if let crate::ast::AstNodeKind::Workspace { name, .. } = workspace_node.get_kind()
        && ctx.get_object_id(workspace_node.get_id()).is_none()
    {
        // If context already has a symbol->id mapping, use it.
        if let Some(&existing_id) = ctx.symbols.get(name) {
            ctx.bind_object_id(workspace_node.get_id(), existing_id);
        } else if let Some(existing_id) = ir_mod.find_object_id_by_name(name) {
            // IR module already declared this object earlier; bind it into ctx.
            ctx.bind_object_id(workspace_node.get_id(), existing_id);
            ctx.symbols.insert(name.clone(), existing_id);
        } else {
            // Not found anywhere — declare a new object id and bind it.
            let oid = ir_mod.declare_object(name);
            ctx.bind_object_id(workspace_node.get_id(), oid);
            ctx.symbols.insert(name.clone(), oid);
        }
    }

    // Collect the workspace body statements (do not lower them yet) so we
    // can detect project declarations and static list assignments to emit
    // array constants and wire iteration lowering. While collecting we
    // suppress module-level emission so any side-effecting statements that
    // belong to the workspace body aren't emitted at module scope.
    ctx.push_suppress_module_emits();
    let members = collect_member_definitions(body, ir_mod, ctx);
    ctx.pop_suppress_module_emits();

    // If this workspace has an associated entrypoint function (from analyzer)
    // we may want to lower its executable body into a per-workspace
    // FunctionBuilder instead of emitting the body directly into the
    // module. Create the builder up-front and pre-emit the entry label so
    // recorded local op indices match final placement when finalized.
    use super::function_builder::FunctionBuilder;
    let entry_fid = ctx.get_function_id(workspace_node.get_id());
    let mut fb_opt: Option<FunctionBuilder> = if let Some(fid) = entry_fid {
        let mut fb = FunctionBuilder::new();
        let label_idx = (fid as usize).saturating_sub(1);
        let label_name = format!("L{}", label_idx);
        fb.emit_op(IROp::Label { name: label_name });
        Some(fb)
    } else {
        None
    };

    // Track unresolved branches emitted while building into the function
    // so we can remap them after the builder is finalized into the module.
    let mut local_unresolved: Vec<(usize, String)> = Vec::new();
    // Hold function builders that should be finalized after module entry
    // has been emitted (so their bodies don't execute as inline module ops).
    let mut postponed_fbs: Vec<(
        super::function_builder::FunctionBuilder,
        Vec<(usize, String)>,
    )> = Vec::new();

    // list collected members (debug removed)

    // First pass: emit array constants for assignments like `ident = [a, b]`
    for stmt in members.iter() {
        if let crate::ast::AstNodeKind::Assignment { target, value } = stmt.get_kind()
            && let crate::ast::AstNodeKind::Identifier { .. } = target.get_kind()
            && let crate::ast::AstNodeKind::List { elements } = value.get_kind()
        {
            // Attempt to build an array of the actual project object runtime
            // registers. Prefer emitting an `ArrayNew` that references the
            // object regs directly so consumers get real objects, not
            // bare symbols. Fall back to the previous Symbol-valued
            // constant array if any element cannot be resolved to a
            // runtime object register.
            let mut elem_regs: Vec<usize> = Vec::new();
            let mut fallback_items: Vec<crate::ir::value::Value> = Vec::new();
            let mut all_resolved = true;
            for el in elements.iter() {
                if let crate::ast::AstNodeKind::Identifier { name } = el.get_kind() {
                    // try to resolve the identifier name to a declared
                    // object id via the lowering context symbol map and
                    // then to a runtime register holding that object.
                    if let Some(&obj_id) = ctx.symbols.get(name)
                        && let Some(obj_reg) = ctx.get_object_reg_by_objid(obj_id)
                    {
                        elem_regs.push(obj_reg);
                        continue;
                    }
                    // couldn't resolve to a runtime object reg; record
                    // a Symbol fallback and mark as not fully resolved.
                    fallback_items.push(crate::ir::value::Value::Symbol(name.clone()));
                    all_resolved = false;
                } else {
                    all_resolved = false;
                    break;
                }
            }
            if all_resolved && !elem_regs.is_empty() {
                // Emit an ArrayNew that constructs the array at runtime
                // from the element registers (which reference project
                // object runtime slots).
                let arr_reg = ir_mod.alloc_reg();
                ir_mod.emit_op(IROp::ArrayNew {
                    dest: arr_reg,
                    elems: elem_regs,
                });
                ctx.bind_list_array(target.get_id(), arr_reg);
                // don't lower this assignment later
            } else if !fallback_items.is_empty() {
                // Fall back to previous behavior: emit a constant array
                // of Symbols if we couldn't resolve all elements to
                // object regs.
                let arr_val = crate::ir::value::Value::Array(fallback_items);
                let arr_reg = ir_mod.alloc_reg();
                ir_mod.emit_op(IROp::LConst {
                    dest: arr_reg,
                    value: arr_val,
                });
                ctx.bind_list_array(target.get_id(), arr_reg);
            }
        }
    }

    // Second pass: emit iteration lowering for `for x in ident` where `ident`
    // refers to one of the statically-created array registers above. Other
    // statements are lowered normally (projects will be handled by
    // `lower_project_object` in the first pass of `lower_script_objects`).
    for stmt in members.iter() {
        // skip project declarations (handled earlier) and static list assigns
        if let crate::ast::AstNodeKind::Project { .. } = stmt.get_kind() {
            continue;
        }
        if let crate::ast::AstNodeKind::Assignment { target, value } = stmt.get_kind()
            && let crate::ast::AstNodeKind::Identifier { .. } = target.get_kind()
            && let crate::ast::AstNodeKind::List { .. } = value.get_kind()
            && ctx.get_list_array(target.get_id()).is_some()
        {
            continue;
        }

        // Detect `for <ident> in <iterable> { body }` and lower to a simple
        // index-based loop over the array register we emitted above.
        if let crate::ast::AstNodeKind::ForIn {
            iterator,
            iterable,
            body,
        } = stmt.get_kind()
            && let crate::ast::AstNodeKind::Identifier { .. } = iterable.get_kind()
        {
            // Try lookup by the iterable AST node id first (fast-path).
            // If not present, try to find a matching assignment target
            // among the collected `members` by identifier name and use
            // its node id to retrieve the bound static array register.
            let mut arr_reg_opt = ctx.get_list_array(iterable.get_id());

            if arr_reg_opt.is_none()
                && let crate::ast::AstNodeKind::Identifier { name: iter_name } = iterable.get_kind()
            {
                for m in members.iter() {
                    if let crate::ast::AstNodeKind::Assignment { target, .. } = m.get_kind()
                        && let crate::ast::AstNodeKind::Identifier { name: target_name } =
                            target.get_kind()
                        && target_name == iter_name
                    {
                        let tid = target.get_id();
                        let got = ctx.get_list_array(tid);
                        if let Some(r) = got {
                            arr_reg_opt = Some(r);
                            break;
                        }
                    }
                }
            }

            if let Some(arr_reg) = arr_reg_opt {
                // If we're lowering the loop body into a FunctionBuilder and
                // the iterable refers to a module-level array register,
                // load that module register into a function-local via
                // `LoadGlobal` so subsequent ops in the builder reference
                // a local register that will be remapped safely during
                // finalization. This avoids accidental remapping of
                // operands that intentionally target module registers.
                let arr_operand = if let Some(fb) = fb_opt.as_mut() {
                    let local_arr = fb.alloc_reg();
                    fb.emit_op(IROp::LoadGlobal {
                        dest: local_arr,
                        src: arr_reg,
                    });
                    local_arr
                } else {
                    arr_reg
                };
                // Emit: idx = 0
                let idx_reg = if let Some(fb) = fb_opt.as_mut() {
                    fb.alloc_reg()
                } else {
                    ir_mod.alloc_reg()
                };
                if let Some(fb) = fb_opt.as_mut() {
                    fb.emit_op(IROp::LConst {
                        dest: idx_reg,
                        value: crate::ir::value::Value::Int(0),
                    });
                } else {
                    ir_mod.emit_op(IROp::LConst {
                        dest: idx_reg,
                        value: crate::ir::value::Value::Int(0),
                    });
                }

                // Emit: key = Str("length"); len = GetProp arr[key]
                let key_reg = if let Some(fb) = fb_opt.as_mut() {
                    fb.alloc_reg()
                } else {
                    ir_mod.alloc_reg()
                };
                if let Some(fb) = fb_opt.as_mut() {
                    fb.emit_op(IROp::LConst {
                        dest: key_reg,
                        value: crate::ir::value::Value::Str("length".to_string()),
                    });
                } else {
                    ir_mod.emit_op(IROp::LConst {
                        dest: key_reg,
                        value: crate::ir::value::Value::Str("length".to_string()),
                    });
                }
                let len_reg = if let Some(fb) = fb_opt.as_mut() {
                    fb.alloc_reg()
                } else {
                    ir_mod.alloc_reg()
                };
                if let Some(fb) = fb_opt.as_mut() {
                    fb.emit_op(IROp::GetProp {
                        dest: len_reg,
                        obj: arr_operand,
                        key: key_reg,
                    });
                } else {
                    ir_mod.emit_op(IROp::GetProp {
                        dest: len_reg,
                        obj: arr_operand,
                        key: key_reg,
                    });
                }

                // loop_cond: cmp = idx < len
                let loop_cond_pos = if let Some(fb) = fb_opt.as_ref() {
                    fb.current_len()
                } else {
                    ir_mod.len()
                };
                let cmp_reg = if let Some(fb) = fb_opt.as_mut() {
                    fb.alloc_reg()
                } else {
                    ir_mod.alloc_reg()
                };
                if let Some(fb) = fb_opt.as_mut() {
                    fb.emit_op(IROp::Lt {
                        dest: cmp_reg,
                        src1: idx_reg,
                        src2: len_reg,
                    });
                } else {
                    ir_mod.emit_op(IROp::Lt {
                        dest: cmp_reg,
                        src1: idx_reg,
                        src2: len_reg,
                    });
                }
                // placeholder BrFalse (will be patched to a generated label)
                let br_pos = if let Some(fb) = fb_opt.as_ref() {
                    fb.current_len()
                } else {
                    ir_mod.len()
                };
                // create a unique after-loop label name to resolve later
                let after_label = format!("__after_ws_{}_{}", workspace_node.get_id(), br_pos);
                if let Some(fb) = fb_opt.as_mut() {
                    fb.emit_op(IROp::BrFalse {
                        cond: cmp_reg,
                        target: 0,
                    });
                } else {
                    ir_mod.emit_op(IROp::BrFalse {
                        cond: cmp_reg,
                        target: 0,
                    });
                }
                // record unresolved branch pointing to our after-label. If
                // we're building into a function builder, record locally so
                // we can remap after finalize; otherwise record on module.
                if fb_opt.is_some() {
                    local_unresolved.push((br_pos, after_label.clone()));
                } else {
                    ir_mod.record_unresolved_branch(br_pos, after_label.clone());
                }

                // body: item = ArrayGet arr[idx]
                let item_reg = if let Some(fb) = fb_opt.as_mut() {
                    fb.alloc_reg()
                } else {
                    ir_mod.alloc_reg()
                };
                if let Some(fb) = fb_opt.as_mut() {
                    fb.emit_op(IROp::ArrayGet {
                        dest: item_reg,
                        array: arr_operand,
                        index: idx_reg,
                    });
                } else {
                    ir_mod.emit_op(IROp::ArrayGet {
                        dest: item_reg,
                        array: arr_operand,
                        index: idx_reg,
                    });
                }

                // Bind the iterator name to the item register temporarily
                // so any lowering that happens in module-context can still
                // resolve references to the iterator identifier.
                ctx.bind_temp_ident(iterator, item_reg);

                // Create a wrapper function to contain the loop body so the
                // loop variable can be a real function-local binding. This
                // avoids complex module-level name binding and keeps semantics
                // predictable: for each iteration we `CallLabel` the wrapper
                // with the item as the first argument which the wrapper will
                // materialize into a local slot matching `iterator`.
                let ws_name = if let crate::ast::AstNodeKind::Workspace { name, .. } =
                    workspace_node.get_kind()
                {
                    name.clone()
                } else {
                    "<anon_ws>".to_string()
                };
                let loop_fn_name = format!(
                    "{}_forin_{}",
                    ws_name,
                    if let Some(fb) = fb_opt.as_ref() {
                        fb.current_len()
                    } else {
                        ir_mod.len()
                    }
                );
                let loop_fn_id = ir_mod.declare_function(&loop_fn_name);
                let label_idx = (loop_fn_id as usize).saturating_sub(1);
                // build the wrapper function body
                let mut fb = super::function_builder::FunctionBuilder::new();
                // create a local for the iterator name so args[0] seeds it
                fb.get_or_create_local(iterator);
                let label_name = format!("L{}", label_idx);
                fb.emit_op(IROp::Label {
                    name: label_name.clone(),
                });
                // lower the loop body into the function builder
                super::lower_stmt::emit_calls_in_node_with_builder(body, &mut fb, ir_mod, ctx);
                // Defer finalization so the wrapper body is appended after
                // the module-level entrypoint and `Halt`, preventing the
                // function body ops from being executed inline at module
                // startup. There are no local unresolved branches here,
                // so push an empty list for that fb.
                postponed_fbs.push((fb, Vec::new()));

                // Now call the wrapper from the loop body with the item as arg
                let dest = if let Some(fb) = fb_opt.as_mut() {
                    fb.alloc_reg()
                } else {
                    ir_mod.alloc_reg()
                };
                if let Some(fb) = fb_opt.as_mut() {
                    fb.emit_op(IROp::CallLabel {
                        dest,
                        label_index: label_idx,
                        args: vec![item_reg],
                    });
                } else {
                    ir_mod.emit_op(IROp::CallLabel {
                        dest,
                        label_index: label_idx,
                        args: vec![item_reg],
                    });
                }

                // increment idx: idx = idx + 1
                let one_reg = if let Some(fb) = fb_opt.as_mut() {
                    fb.alloc_reg()
                } else {
                    ir_mod.alloc_reg()
                };
                if let Some(fb) = fb_opt.as_mut() {
                    fb.emit_op(IROp::LConst {
                        dest: one_reg,
                        value: crate::ir::value::Value::Int(1),
                    });
                    fb.emit_op(IROp::Add {
                        dest: idx_reg,
                        src1: idx_reg,
                        src2: one_reg,
                    });
                } else {
                    ir_mod.emit_op(IROp::LConst {
                        dest: one_reg,
                        value: crate::ir::value::Value::Int(1),
                    });
                    ir_mod.emit_op(IROp::Add {
                        dest: idx_reg,
                        src1: idx_reg,
                        src2: one_reg,
                    });
                }

                // jump back to condition
                if let Some(fb) = fb_opt.as_mut() {
                    fb.emit_op(IROp::Jump {
                        target: loop_cond_pos,
                    });
                } else {
                    ir_mod.emit_op(IROp::Jump {
                        target: loop_cond_pos,
                    });
                }

                // emit a label at the end of the loop body so the earlier
                // placeholder can be resolved to this exact op index later.
                if let Some(fb) = fb_opt.as_mut() {
                    fb.emit_op(IROp::Label {
                        name: after_label.clone(),
                    });
                } else {
                    ir_mod.emit_op(IROp::Label {
                        name: after_label.clone(),
                    });
                }
                // done with the temporary iterator binding
                ctx.unbind_temp_ident(iterator);
                continue;
            }
        }

        // Default: lower the statement. If we have a function builder for
        // this workspace, route lowering into the builder so the emitted
        // ops become part of the function body; otherwise emit at module
        // scope as before.
        if let Some(fb) = fb_opt.as_mut() {
            super::lower_stmt::emit_calls_in_node_with_builder(stmt, fb, ir_mod, ctx);
        } else {
            super::lower_stmt::lower_statment(stmt, ir_mod, ctx);
        }
    }

    // Emit a final Halt to ensure workspace logic terminates cleanly
    // If we built the workspace body into a FunctionBuilder, the
    // executable body will be finalized into the module below; in that
    // case avoid emitting a module-level Halt here which would terminate
    // execution before the entrypoint call. Only emit a module Halt when
    // there is no entrypoint function builder.
    if fb_opt.is_none() {
        ir_mod.emit_op(IROp::Halt);
    }

    // If analyzer marked this workspace as an entrypoint, prepare its
    // wrapper function to be finalized after module entry is emitted so
    // its body does not execute inline during module startup. We keep the
    // fb in `postponed_fbs` along with any locally-recorded unresolved
    // branches so we can remap them after finalization.
    if entry_fid.is_some()
        && let Some(fb) = fb_opt.take()
    {
        // Ensure the entrypoint function returns to its caller instead
        // of halting the whole VM. Emit a `Ret Null` so the module-level
        // `CallLabel` can receive a (null) return value and continue.
        let mut fb = fb;
        let ret_reg = fb.alloc_reg();
        fb.emit_op(IROp::LConst {
            dest: ret_reg,
            value: crate::ir::value::Value::Null,
        });
        fb.emit_op(IROp::Ret { src: ret_reg });
        // Defer finalize: push the builder and its unresolved branches
        // for remapping after we have emitted the module entrypoint.
        postponed_fbs.push((
            fb,
            std::mem::take(&mut local_unresolved).into_iter().collect(),
        ));
    }

    // Emit module-level entrypoint and Halt. If there was no entrypoint
    // builder, emit a Halt immediately (so module-level init code stops).
    if postponed_fbs.is_empty() {
        ir_mod.emit_op(IROp::Halt);
    } else {
        // Emit `main` label and call the (declared) workspace entrypoint.
        ir_mod.emit_op(IROp::Label {
            name: "main".to_string(),
        });
        // pick the first postponed fb's function id if it corresponds to
        // the workspace entrypoint (we declared it earlier as `entry_fid`).
        if let Some(fid) = entry_fid {
            let label_idx = (fid as usize).saturating_sub(1);
            let call_dest = ir_mod.alloc_reg();
            ir_mod.emit_op(IROp::CallLabel {
                dest: call_dest,
                label_index: label_idx,
                args: vec![],
            });
        }
        ir_mod.emit_op(IROp::Halt);
    }

    // Finalize any postponed function builders now that the module entry
    // and Halt are in place. This ensures their bodies are appended after
    // the Halt and won't be executed inline at startup.
    for (fb, unresolved) in postponed_fbs.into_iter() {
        let base_op_index = ir_mod.len();
        fb.finalize_into(ir_mod);
        for (local_pos, lbl) in unresolved.into_iter() {
            ir_mod.record_unresolved_branch(base_op_index + local_pos, lbl);
        }
    }
}

fn lower_project_object(
    project_node: &crate::ast::AstNode,
    ir_mod: &mut crate::ir::module::IrModule,
    ctx: &mut super::lowering_context::LoweringContext,
) {
    // Register the project as an object and lower its member assignments
    if let crate::ast::AstNodeKind::Project { name, body } = project_node.get_kind() {
        // ensure object id exists
        if ctx.get_object_id(project_node.get_id()).is_none() {
            let oid = ir_mod.declare_object(name);
            ctx.bind_object_id(project_node.get_id(), oid);
            ctx.symbols.insert(name.clone(), oid);
        }

        // Create a module-level object runtime slot (a register holding the object)
        // only if one hasn't already been pre-created from analyzer output.
        if ctx.get_object_reg(project_node.get_id()).is_none() {
            let obj_reg = ir_mod.alloc_reg();
            // initialize to an empty object so SetProp writes into a real object
            let empty_map: std::collections::HashMap<String, crate::ir::value::Value> =
                std::collections::HashMap::new();
            ir_mod.emit_op(IROp::LConst {
                dest: obj_reg,
                value: crate::ir::value::Value::Object(empty_map),
            });
            // record runtime register in lowering context for other passes
            ctx.bind_object_reg(project_node.get_id(), obj_reg);
            // Also bind by declared object id so lookups via symbol->object id
            // mapping can find the runtime register during Member lowering.
            if let Some(obj_id) = ctx.get_object_id(project_node.get_id()) {
                ctx.bind_object_reg_by_objid(obj_id, obj_reg);
            }
        }

        // Lower each statement in the project body; treat assignments to
        // identifiers as setting properties on this object.
        if let crate::ast::AstNodeKind::Block { statements } = body.get_kind() {
            // Ensure obj_reg is available in this scope
            let obj_reg = ctx
                .get_object_reg(project_node.get_id())
                .expect("Object register should be initialized");
            for stmt in statements.iter() {
                if let crate::ast::AstNodeKind::Assignment { target, value } = stmt.get_kind()
                    && let crate::ast::AstNodeKind::Identifier { name: prop_name } =
                        target.get_kind()
                {
                    // evaluate value into a register (use the builder-aware helper
                    // so list literals and other expressions are handled)
                    let val_reg =
                        super::lower_expr::lower_expr_to_reg_with_builder(value, ir_mod, ctx, None);
                    // emit key symbol const
                    let key_reg = ir_mod.alloc_reg();
                    ir_mod.emit_op(IROp::LConst {
                        dest: key_reg,
                        value: crate::ir::value::Value::Symbol(prop_name.clone()),
                    });
                    // emit SetProp obj.prop = val
                    ir_mod.emit_op(IROp::SetProp {
                        obj: obj_reg,
                        key: key_reg,
                        src: val_reg,
                    });
                    continue;
                }
                // fallback: lower the statement normally to preserve side-effects
                super::lower_stmt::lower_statment(stmt, ir_mod, ctx)
            }
        }
    }
}

fn lower_stage_object(
    _stage_node: &crate::ast::AstNode,
    _ir_mod: &mut crate::ir::module::IrModule,
    _ctx: &mut super::lowering_context::LoweringContext,
) {
    // Register the stage as a function prototype so calls can reference it.
    if let crate::ast::AstNodeKind::Stage { name, .. } = _stage_node.get_kind() {
        // If not already declared, declare and bind
        if _ctx.get_function_id(_stage_node.get_id()).is_none() {
            let id = _ir_mod.declare_function(name);
            _ctx.bind_function_id(_stage_node.get_id(), id);
            // Also populate the symbol map for name->id lookups
            _ctx.symbols.insert(name.clone(), id);
        }
    }
}

/// Collect member definitions from a container body node.
fn collect_member_definitions(
    object_node: &crate::ast::AstNode,
    _ir_mod: &mut crate::ir::module::IrModule,
    _ctx: &mut super::lowering_context::LoweringContext,
) -> Vec<crate::ast::AstNode> {
    let mut members: Vec<crate::ast::AstNode> = Vec::new();
    if let crate::ast::AstNodeKind::Block { statements } = object_node.get_kind() {
        for stmt in statements {
            match stmt.get_kind() {
                // Collect project declarations and simple assignments so we can
                // detect static list initializers (e.g. `projects = [p]`) and
                // lower iteration in a later pass. Also collect `ForIn`
                // so workspace-level loops are handled after array constants
                // have been emitted. Other statements are lowered immediately
                // to preserve side-effects.
                crate::ast::AstNodeKind::Project { .. } => {
                    members.push(stmt.clone());
                }
                crate::ast::AstNodeKind::Assignment { .. } => {
                    members.push(stmt.clone());
                }
                crate::ast::AstNodeKind::ForIn { .. } => {
                    members.push(stmt.clone());
                }
                _ => {
                    // Preserve side-effecting top-level statements by collecting
                    // them for the second pass. We previously lowered these
                    // eagerly which could cause bodies (e.g. ForIn inner
                    // statements) to be emitted at module scope before wrapper
                    // functions were created. Collecting here lets the
                    // workspace lowering decide how to lower each member in
                    // the correct context (module vs wrapper).
                    members.push(stmt.clone());
                }
            }
        }
    }
    members
}
