//! file: core/src/ir/lower/function_builder.rs
//! description: per-function lowering helper.
//!
//! `FunctionBuilder` provides a small, function-scoped lowering arena with
//! a local register allocator and op buffer. After lowering a function body
//! the builder can finalize its ops into the parent `IrModule`.
//!
use crate::ir::op::IROp;
use crate::ir::module::IrModule;
use std::collections::HashMap;

/// A per-function lowering helper that provides a function-local virtual
/// register allocator, a local slot map, and an op buffer. After lowering a
/// function, its ops can be finalized into the parent `IrModule`.
#[derive(Debug, Clone, Default)]
pub struct FunctionBuilder {
    next_reg: usize,
    next_local: usize,
    locals: HashMap<String, usize>,
    pub ops: Vec<IROp>,
}

impl FunctionBuilder {
    pub fn new() -> Self {
        FunctionBuilder { next_reg: 0, next_local: 0, locals: HashMap::new(), ops: Vec::new() }
    }

    pub fn alloc_reg(&mut self) -> usize {
        let r = self.next_reg;
        self.next_reg = self.next_reg.wrapping_add(1);
        r
    }

    pub fn alloc_local(&mut self) -> usize {
        let l = self.next_local;
        self.next_local = self.next_local.wrapping_add(1);
        l
    }

    pub fn get_or_create_local(&mut self, name: &str) -> usize {
        if let Some(&idx) = self.locals.get(name) {
            idx
        } else {
            let idx = self.alloc_local();
            self.locals.insert(name.to_string(), idx);
            idx
        }
    }

    pub fn lookup_local(&self, name: &str) -> Option<usize> {
        self.locals.get(name).copied()
    }

    pub fn emit_op(&mut self, op: IROp) {
        self.ops.push(op);
    }

    pub fn current_len(&self) -> usize { self.ops.len() }

    pub fn patch_op(&mut self, idx: usize, op: IROp) {
        if idx < self.ops.len() {
            self.ops[idx] = op;
        }
    }

    /// Finalize this function's ops into the provided module.
    /// This appends ops in order to the module's op list.
    pub fn finalize_into(self, module: &mut IrModule) {
        // Reserve a range of module-level registers for this function's
        // local virtual registers and compute a base offset so we can map
        // all register indices from the function-local space into the
        // module-global space. Also compute base op index for label target
        // relocation.
        let base_op_index = module.len();

        // Reserve a contiguous range of registers in the module for this
        // function's local registers. We cannot access `module.next_reg`
        // directly since it's private, so call `alloc_reg` repeatedly to
        // claim the needed registers and use the first allocated register
        // as the base offset.
        let local_reg_count = self.next_reg;
        let reg_base = if local_reg_count == 0 {
            0usize
        } else {
            let first = module.alloc_reg();
            for _ in 1..local_reg_count {
                let _ = module.alloc_reg();
            }
            first
        };

        // Remap and emit all ops into the module
        for mut op in self.ops.into_iter() {
            // Remap register indices in the op from local->global by adding
            // `reg_base`. Also adjust intra-function numeric branch targets
            // to module-global op indices by adding `base_op_index`.
                match &mut op {
                IROp::LConst { dest, .. } => {
                    let orig = *dest;
                    if orig < local_reg_count { *dest += reg_base; }
                }
                IROp::LLocal { dest, local_index: _ } => {
                    let orig = *dest;
                    if orig < local_reg_count { *dest += reg_base; }
                }
                IROp::SLocal { src, local_index: _ } => {
                    let orig = *src;
                    if orig < local_reg_count { *src += reg_base; }
                }

                IROp::Add { dest, src1, src2 }
                | IROp::Sub { dest, src1, src2 }
                | IROp::Mul { dest, src1, src2 }
                | IROp::Div { dest, src1, src2 }
                | IROp::Mod { dest, src1, src2 }
                | IROp::Eq { dest, src1, src2 }
                | IROp::Neq { dest, src1, src2 }
                | IROp::Lt { dest, src1, src2 }
                | IROp::Lte { dest, src1, src2 }
                | IROp::Gt { dest, src1, src2 }
                | IROp::Gte { dest, src1, src2 }
                | IROp::And { dest, src1, src2 }
                | IROp::Or { dest, src1, src2 } => {
                    let od = *dest; if od < local_reg_count { *dest += reg_base; }
                    let os1 = *src1; if os1 < local_reg_count { *src1 += reg_base; }
                    let os2 = *src2; if os2 < local_reg_count { *src2 += reg_base; }
                }
                IROp::Not { dest, src } => { let od=*dest; if od < local_reg_count { *dest += reg_base; } let os=*src; if os < local_reg_count { *src += reg_base; } }

                IROp::Inc { dest } | IROp::Dec { dest } => { let od=*dest; if od < local_reg_count { *dest += reg_base; } }

                IROp::Jump { target } => { *target += base_op_index; }
                IROp::BrTrue { cond, target } => { let oc=*cond; if oc < local_reg_count { *cond += reg_base; } *target += base_op_index; }
                IROp::BrFalse { cond, target } => { let oc=*cond; if oc < local_reg_count { *cond += reg_base; } *target += base_op_index; }

                IROp::AllocClosure { dest } => { let od=*dest; if od < local_reg_count { *dest += reg_base; } }
                IROp::CStore { closure, field: _, src } => { let oc=*closure; if oc < local_reg_count { *closure += reg_base; } let os=*src; if os < local_reg_count { *src += reg_base; } }
                IROp::CLoad { dest, closure, field: _ } => { let od=*dest; if od < local_reg_count { *dest += reg_base; } let oc=*closure; if oc < local_reg_count { *closure += reg_base; } }

                IROp::ArrayNew { dest, elems } => { let od=*dest; if od < local_reg_count { *dest += reg_base; } for e in elems.iter_mut() { let oe=*e; if oe < local_reg_count { *e += reg_base; } } }
                IROp::LoadGlobal { dest, src: _ } => { let od=*dest; if od < local_reg_count { *dest += reg_base; } /* src is module-global and must not be remapped */ }
                IROp::ArrayGet { dest, array, index } => { let od=*dest; if od < local_reg_count { *dest += reg_base; } let oa=*array; if oa < local_reg_count { *array += reg_base; } let oi=*index; if oi < local_reg_count { *index += reg_base; } }
                IROp::ArraySet { array, index, src } => { let oa=*array; if oa < local_reg_count { *array += reg_base; } let oi=*index; if oi < local_reg_count { *index += reg_base; } let os=*src; if os < local_reg_count { *src += reg_base; } }

                IROp::GetProp { dest, obj, key } => { let od=*dest; if od < local_reg_count { *dest += reg_base; } let oo=*obj; if oo < local_reg_count { *obj += reg_base; } let ok=*key; if ok < local_reg_count { *key += reg_base; } }
                IROp::SetProp { obj, key, src } => { let oo=*obj; if oo < local_reg_count { *obj += reg_base; } let ok=*key; if ok < local_reg_count { *key += reg_base; } let os=*src; if os < local_reg_count { *src += reg_base; } }

                IROp::PluginCall { dest, plugin_name: _, func_name: _, args } => {
                    if let Some(d) = dest {
                        let od = *d; if od < local_reg_count { *d += reg_base; }
                    }
                    for a in args.iter_mut() { let oa=*a; if oa < local_reg_count { *a += reg_base; } }
                }
                IROp::CallLabel { dest, label_index: _, args } => { let od=*dest; if od < local_reg_count { *dest += reg_base; } for a in args.iter_mut() { let oa=*a; if oa < local_reg_count { *a += reg_base; } } }
                IROp::Ret { src } => { let os=*src; if os < local_reg_count { *src += reg_base; } }

                _ => {}
            }
            module.emit_op(op);
        }
    }
}
