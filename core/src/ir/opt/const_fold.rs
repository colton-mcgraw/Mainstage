//! file: core/src/ir/opt/const_fold.rs
//! description: constant folding optimizer pass.
//!
//! Performs local constant folding on IR ops where both operands are
//! compile-time constants. This pass is intentionally conservative and
//! works together with propagation and DCE passes.
//!
use crate::ir::module::IrModule;
use crate::ir::op::IROp;
use crate::ir::value::Value;
use std::collections::HashMap;

/// Constant-fold simple, localizable IR ops.
pub(crate) fn constant_fold(ir: &mut IrModule) {
    let mut const_map: HashMap<usize, Value> = HashMap::new();
    // Track locals that are known constants: local_index -> Value
    let mut local_const_map: HashMap<usize, Value> = HashMap::new();
    let mut new_ops: Vec<IROp> = Vec::with_capacity(ir.ops.len());

    for op in ir.ops.drain(..) {
        match &op {
            IROp::LConst { dest, value } => {
                const_map.insert(*dest, value.clone());
                new_ops.push(op);
            }
            IROp::SLocal { src, local_index } => {
                // If the source register is a known constant, mark the local as constant.
                if let Some(v) = const_map.get(src).cloned() {
                    local_const_map.insert(*local_index, v);
                } else {
                    local_const_map.remove(local_index);
                }
                new_ops.push(op);
            }
            IROp::LLocal { dest, local_index } => {
                // If the local has a known constant value, fold into LConst.
                if let Some(v) = local_const_map.get(local_index).cloned() {
                    const_map.insert(*dest, v.clone());
                    new_ops.push(IROp::LConst {
                        dest: *dest,
                        value: v,
                    });
                } else {
                    const_map.remove(dest);
                    new_ops.push(op);
                }
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
                // Avoid folding self-referential updates like `r = r + c` which
                // are loop-carried -- folding these in a single linear pass can
                // remove the runtime increment semantics. If either source is
                // the same as the destination, skip folding here.
                if *src1 == *dest || *src2 == *dest {
                    const_map.remove(dest);
                    new_ops.push(op);
                    continue;
                }
                let maybe_v1 = const_map.get(src1).cloned();
                let maybe_v2 = const_map.get(src2).cloned();
                if let (Some(v1), Some(v2)) = (maybe_v1, maybe_v2)
                    && let Some(res) = super::compute_binop(&op, &v1, &v2)
                {
                    let d = *dest;
                    const_map.insert(d, res.clone());
                    new_ops.push(IROp::LConst {
                        dest: d,
                        value: res,
                    });
                    continue;
                }
                const_map.remove(dest);
                new_ops.push(op);
            }
            IROp::Not { dest, src } => {
                if let Some(v) = const_map.get(src).cloned()
                    && let Value::Bool(b) = v
                {
                    let d = *dest;
                    const_map.insert(d, Value::Bool(!b));
                    new_ops.push(IROp::LConst {
                        dest: d,
                        value: Value::Bool(!b),
                    });
                    continue;
                }
                const_map.remove(dest);
                new_ops.push(op);
            }
            // ops that write to a destination register should invalidate that register
            IROp::Inc { .. } | IROp::Dec { .. } | IROp::CallLabel { .. } | IROp::CLoad { .. } => {
                let d = match &op {
                    IROp::Inc { dest } => *dest,
                    IROp::Dec { dest } => *dest,
                    IROp::CallLabel { dest, .. } => *dest,
                    IROp::CLoad { dest, .. } => *dest,
                    _ => unreachable!(),
                };
                const_map.remove(&d);
                new_ops.push(op);
            }
            _ => {
                new_ops.push(op);
            }
        }
    }

    ir.ops = new_ops;
}
