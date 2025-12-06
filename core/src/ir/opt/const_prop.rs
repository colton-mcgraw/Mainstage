//! file: core/src/ir/opt/const_prop.rs
//! description: constant propagation pass for the IR optimizer.
//!
//! Performs forward constant propagation and removes dead `LConst` ops
//! when possible. This is a conservative, single-pass propagation used
//! as part of the optimizer pipeline.
//!
use crate::ir::module::IrModule;
use crate::ir::op::IROp;
use crate::ir::value::Value;

// Simple constant propagation + dead LConst elimination.
pub(crate) fn const_prop(ir: &mut IrModule) {
    use std::collections::HashMap;

    // Map register -> constant value when known
    let mut consts: HashMap<usize, Value> = HashMap::new();
    // Map local_index -> constant value for locals within the current
    // function. Cleared on encountering a Label (function boundary).
    let mut local_consts: HashMap<usize, Value> = HashMap::new();
    let mut new_ops: Vec<IROp> = Vec::with_capacity(ir.ops.len());

    for op in ir.ops.drain(..) {
        match &op {
            IROp::LConst { dest, value } => {
                consts.insert(*dest, value.clone());
                new_ops.push(op);
            }
            IROp::Label { .. } => {
                // Entering a new function/label: clear any tracked local constants
                local_consts.clear();
                new_ops.push(op);
            }
            // binary ops: try folding if both srcs are constant
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
                // are loop-carried and should not be turned into a constant by a
                // single-pass forward propagation. If either source is the same
                // as the destination, skip folding here.
                if *src1 == *dest || *src2 == *dest {
                    consts.remove(dest);
                    new_ops.push(op);
                    continue;
                }
                let v1 = consts.get(src1).cloned();
                let v2 = consts.get(src2).cloned();
                if let (Some(a), Some(b)) = (v1, v2) {
                    if let Some(res) = super::compute_binop(&op, &a, &b) {
                        // replace with LConst
                        let lc = IROp::LConst { dest: *dest, value: res.clone() };
                        consts.insert(*dest, res);
                        new_ops.push(lc);
                        continue;
                    }
                }
                // otherwise, this dest is not constant
                consts.remove(dest);
                new_ops.push(op);
            }
            // unary Not
            IROp::Not { dest, src } => {
                if let Some(v) = consts.get(src).cloned() {
                    match v {
                        Value::Bool(b) => {
                            let lc = IROp::LConst { dest: *dest, value: Value::Bool(!b) };
                            consts.insert(*dest, Value::Bool(!b));
                            new_ops.push(lc);
                            continue;
                        }
                        _ => {}
                    }
                }
                consts.remove(dest);
                new_ops.push(op);
            }
            // ops that read array/object props where container is a known const
            IROp::GetProp { dest, obj, key } => {
                if let Some(objv) = consts.get(obj).cloned() {
                    if let Value::Object(map) = objv {
                        if let Some(keyv) = consts.get(key).cloned() {
                            if let Value::Symbol(k) = keyv {
                                if let Some(v) = map.get(&k) {
                                    let lc = IROp::LConst { dest: *dest, value: v.clone() };
                                    consts.insert(*dest, v.clone());
                                    new_ops.push(lc);
                                    continue;
                                }
                            }
                        }
                    }
                }
                consts.remove(dest);
                new_ops.push(op);
            }
            // ArrayGet when array is a constant array
            IROp::ArrayGet { dest, array, index } => {
                if let Some(arrv) = consts.get(array).cloned() {
                    if let Value::Array(elems) = arrv {
                        if let Some(idxv) = consts.get(index).cloned() {
                            if let Value::Int(i) = idxv {
                                let idxusize = i as usize;
                                if idxusize < elems.len() {
                                    let v = elems[idxusize].clone();
                                    let lc = IROp::LConst { dest: *dest, value: v.clone() };
                                    consts.insert(*dest, v);
                                    new_ops.push(lc);
                                    continue;
                                }
                            }
                        }
                    }
                }
                consts.remove(dest);
                new_ops.push(op);
            }
            // Handle SLocal specially to track local slot constants. Also
            // treat control ops conservatively.
            IROp::SLocal { src, local_index } => {
                // If the source register is a known constant, record it for the
                // local slot so subsequent LLocal can be replaced.
                if let Some(v) = consts.get(src).cloned() {
                    local_consts.insert(*local_index, v);
                } else {
                    local_consts.remove(local_index);
                }
                new_ops.push(op);
            }
            // Replace LLocal loads with LConst when the local slot holds a
            // tracked constant value.
            IROp::LLocal { dest, local_index } => {
                if let Some(v) = local_consts.get(local_index).cloned() {
                    // fold into LConst
                    consts.insert(*dest, v.clone());
                    new_ops.push(IROp::LConst { dest: *dest, value: v });
                    continue;
                }
                // otherwise the load is not a constant
                consts.remove(dest);
                new_ops.push(op);
            }
            IROp::Ret { .. } | IROp::BrTrue { .. } | IROp::BrFalse { .. } | IROp::Jump { .. } | IROp::Halt => {
                new_ops.push(op);
            }
            // general case: conservatively drop any const mapping for
            // registers written by this op (if we can detect them) and
            // keep the op.
            _ => {
                if let IROp::LLocal { dest, .. } = &op { consts.remove(dest); }
                else if let IROp::AllocClosure { dest } = &op { consts.remove(dest); }
                else if let IROp::CLoad { dest, .. } = &op { consts.remove(dest); }
                else if let IROp::ArrayNew { dest, .. } = &op { consts.remove(dest); }
                else if let IROp::CallLabel { dest, .. } = &op { consts.remove(dest); }
                else if let IROp::PluginCall { dest: Some(d), .. } = &op { consts.remove(d); }
                else if let IROp::LoadGlobal { dest, .. } = &op { consts.remove(dest); }
                else if let IROp::ArrayGet { dest, .. } = &op { consts.remove(dest); }
                else if let IROp::GetProp { dest, .. } = &op { consts.remove(dest); }
                else if let IROp::Inc { dest } = &op { consts.remove(dest); }
                else if let IROp::Dec { dest } = &op { consts.remove(dest); }
                new_ops.push(op);
            }
        }
    }

    // Simple DCE: drop LConst ops whose dest isn't used by any later op.
    let mut used: std::collections::HashSet<usize> = std::collections::HashSet::new();
    // scan forward collecting all read registers
    for op in new_ops.iter() {
        match op {
            IROp::LLocal { dest: _, local_index: _ } => {}
            IROp::SLocal { src, local_index: _ } => { used.insert(*src); }
            IROp::Add { dest: _, src1, src2 }
            | IROp::Sub { dest: _, src1, src2 }
            | IROp::Mul { dest: _, src1, src2 }
            | IROp::Div { dest: _, src1, src2 }
            | IROp::Mod { dest: _, src1, src2 }
            | IROp::Eq { dest: _, src1, src2 }
            | IROp::Neq { dest: _, src1, src2 }
            | IROp::Lt { dest: _, src1, src2 }
            | IROp::Lte { dest: _, src1, src2 }
            | IROp::Gt { dest: _, src1, src2 }
            | IROp::Gte { dest: _, src1, src2 }
            | IROp::And { dest: _, src1, src2 }
            | IROp::Or { dest: _, src1, src2 } => { used.insert(*src1); used.insert(*src2); }
            IROp::Not { dest: _, src } => { used.insert(*src); }
            IROp::BrTrue { cond, .. } | IROp::BrFalse { cond, .. } => { used.insert(*cond); }
            IROp::CallLabel { dest: _, label_index: _, args } => { for a in args.iter() { used.insert(*a); } }
            IROp::PluginCall { dest, plugin_name: _, func_name: _, args } => {
                if let Some(d) = dest { used.insert(*d); }
                for a in args.iter() { used.insert(*a); }
            }
            IROp::GetProp { dest: _, obj, key } => { used.insert(*obj); used.insert(*key); }
            IROp::ArrayNew { dest: _, elems } => { for e in elems.iter() { used.insert(*e); } }
            IROp::SetProp { obj, key, src } => { used.insert(*obj); used.insert(*key); used.insert(*src); }
            IROp::ArrayGet { dest: _, array, index } => { used.insert(*array); used.insert(*index); }
            IROp::ArraySet { array, index, src } => { used.insert(*array); used.insert(*index); used.insert(*src); }
            IROp::LoadGlobal { dest: _, src } => { used.insert(*src); }
            IROp::Ret { src } => { used.insert(*src); }
            _ => {}
        }
    }

    let mut final_ops: Vec<IROp> = Vec::with_capacity(new_ops.len());
    for op in new_ops.into_iter() {
        if let IROp::LConst { dest, .. } = &op {
            if !used.contains(dest) {
                // drop dead constant
                continue;
            }
        }
        final_ops.push(op);
    }

    ir.ops = final_ops;
}
