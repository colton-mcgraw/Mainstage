//! file: core/src/ir/opt/const_canon.rs
//! description: canonicalize equivalent constant `LConst` ops.
//!
//! Merges duplicate `LConst` values into canonical registers when it is
//! safe to do so to reduce IR size and improve downstream optimization
//! effects.
//!
use crate::ir::module::IrModule;
use crate::ir::op::IROp;
use crate::ir::value::Value;

/// Conservative constant canonicalization: merge duplicate `LConst` ops
/// when it is safe to do so. Safety checks:
/// - the earlier canonical register must not be overwritten later;
/// - the duplicate register must not be overwritten later (so remapping
///   uses of it to the canonical register is valid until end-of-ops).
pub(crate) fn canonicalize_constants(ir: &mut IrModule) {
    use std::collections::HashMap;

    // Pre-scan: record last write position for each register
    let mut last_write: HashMap<usize, usize> = HashMap::new();
    for (i, op) in ir.ops.iter().enumerate() {
        match op {
            IROp::LConst { dest, .. }
            | IROp::LLocal { dest, .. }
            | IROp::AllocClosure { dest }
            | IROp::CLoad { dest, .. }
            | IROp::ArrayNew { dest, .. }
            | IROp::CallLabel { dest, .. }
            | IROp::LoadGlobal { dest, .. }
            | IROp::ArrayGet { dest, .. }
            | IROp::GetProp { dest, .. }
            | IROp::Inc { dest }
            | IROp::Dec { dest }
            => { last_write.insert(*dest, i); }
            IROp::PluginCall { dest: Some(d), .. } => { last_write.insert(*d, i); }
            _ => {}
        }
    }

    // Map Value -> (canonical_reg, pos_of_canonical_write)
    let mut value_to_canon: HashMap<Value, (usize, usize)> = HashMap::new();
    // remap register -> canonical register
    let mut remap: HashMap<usize, usize> = HashMap::new();

    let mut out_ops: Vec<IROp> = Vec::with_capacity(ir.ops.len());

    for (i, op) in ir.ops.iter().cloned().enumerate() {
        match op.clone() {
            IROp::LConst { dest, value } => {
                if let Some(&(canon_reg, canon_pos)) = value_to_canon.get(&value) {
                    // check canonical register is stable (no writes after its write)
                    let canon_last = last_write.get(&canon_reg).copied().unwrap_or(canon_pos);
                    if canon_last == canon_pos {
                        // check this dest is not overwritten later
                        let dest_last = last_write.get(&dest).copied().unwrap_or(i);
                        if dest_last == i {
                            // safe to canonicalize: record remap and skip emitting this LConst
                            remap.insert(dest, canon_reg);
                            continue;
                        }
                    }
                }
                // otherwise make this the canonical for the value
                value_to_canon.insert(value.clone(), (dest, i));
                out_ops.push(IROp::LConst { dest, value });
            }
            other => {
                // rewrite any register operands according to remap mapping
                let rewritten = rewrite_op_regs(other, &remap);
                out_ops.push(rewritten);
            }
        }
    }

    ir.ops = out_ops;

    // Update module externally-visible register metadata to account for any
    // remapping we performed so later passes seed liveness correctly.
    if !remap.is_empty() {
        // Compute the rewritten externally-visible set and atomically replace
        // the module's metadata so it reflects canonical registers only.
        let mut new_vis: std::collections::HashSet<usize> = std::collections::HashSet::new();
        for &r in ir.get_externally_visible().iter() {
            new_vis.insert(rewrite_reg(r, &remap));
        }
        ir.set_externally_visible(new_vis);
    }
}

fn rewrite_reg(mut r: usize, remap: &std::collections::HashMap<usize, usize>) -> usize {
    // follow remap chain until stable
    while let Some(&n) = remap.get(&r) {
        if n == r { break; }
        r = n;
    }
    r
}

fn rewrite_op_regs(op: IROp, remap: &std::collections::HashMap<usize, usize>) -> IROp {
    match op {
        IROp::LConst { dest, value } => IROp::LConst { dest: rewrite_reg(dest, remap), value },
        IROp::LLocal { dest, local_index } => IROp::LLocal { dest: rewrite_reg(dest, remap), local_index },
        IROp::SLocal { src, local_index } => IROp::SLocal { src: rewrite_reg(src, remap), local_index },
        IROp::Add { dest, src1, src2 } => IROp::Add { dest: rewrite_reg(dest, remap), src1: rewrite_reg(src1, remap), src2: rewrite_reg(src2, remap) },
        IROp::Sub { dest, src1, src2 } => IROp::Sub { dest: rewrite_reg(dest, remap), src1: rewrite_reg(src1, remap), src2: rewrite_reg(src2, remap) },
        IROp::Mul { dest, src1, src2 } => IROp::Mul { dest: rewrite_reg(dest, remap), src1: rewrite_reg(src1, remap), src2: rewrite_reg(src2, remap) },
        IROp::Div { dest, src1, src2 } => IROp::Div { dest: rewrite_reg(dest, remap), src1: rewrite_reg(src1, remap), src2: rewrite_reg(src2, remap) },
        IROp::Mod { dest, src1, src2 } => IROp::Mod { dest: rewrite_reg(dest, remap), src1: rewrite_reg(src1, remap), src2: rewrite_reg(src2, remap) },
        IROp::Eq { dest, src1, src2 } => IROp::Eq { dest: rewrite_reg(dest, remap), src1: rewrite_reg(src1, remap), src2: rewrite_reg(src2, remap) },
        IROp::Neq { dest, src1, src2 } => IROp::Neq { dest: rewrite_reg(dest, remap), src1: rewrite_reg(src1, remap), src2: rewrite_reg(src2, remap) },
        IROp::Lt { dest, src1, src2 } => IROp::Lt { dest: rewrite_reg(dest, remap), src1: rewrite_reg(src1, remap), src2: rewrite_reg(src2, remap) },
        IROp::Lte { dest, src1, src2 } => IROp::Lte { dest: rewrite_reg(dest, remap), src1: rewrite_reg(src1, remap), src2: rewrite_reg(src2, remap) },
        IROp::Gt { dest, src1, src2 } => IROp::Gt { dest: rewrite_reg(dest, remap), src1: rewrite_reg(src1, remap), src2: rewrite_reg(src2, remap) },
        IROp::Gte { dest, src1, src2 } => IROp::Gte { dest: rewrite_reg(dest, remap), src1: rewrite_reg(src1, remap), src2: rewrite_reg(src2, remap) },
        IROp::And { dest, src1, src2 } => IROp::And { dest: rewrite_reg(dest, remap), src1: rewrite_reg(src1, remap), src2: rewrite_reg(src2, remap) },
        IROp::Or { dest, src1, src2 } => IROp::Or { dest: rewrite_reg(dest, remap), src1: rewrite_reg(src1, remap), src2: rewrite_reg(src2, remap) },
        IROp::Not { dest, src } => IROp::Not { dest: rewrite_reg(dest, remap), src: rewrite_reg(src, remap) },
        IROp::Inc { dest } => IROp::Inc { dest: rewrite_reg(dest, remap) },
        IROp::Dec { dest } => IROp::Dec { dest: rewrite_reg(dest, remap) },
        IROp::Label { name } => IROp::Label { name },
        IROp::Jump { target } => IROp::Jump { target },
        IROp::BrTrue { cond, target } => IROp::BrTrue { cond: rewrite_reg(cond, remap), target },
        IROp::BrFalse { cond, target } => IROp::BrFalse { cond: rewrite_reg(cond, remap), target },
        IROp::Halt => IROp::Halt,
        IROp::AllocClosure { dest } => IROp::AllocClosure { dest: rewrite_reg(dest, remap) },
        IROp::CStore { closure, field, src } => IROp::CStore { closure: rewrite_reg(closure, remap), field, src: rewrite_reg(src, remap) },
        IROp::CLoad { dest, closure, field } => IROp::CLoad { dest: rewrite_reg(dest, remap), closure: rewrite_reg(closure, remap), field },
        IROp::ArrayNew { dest, elems } => IROp::ArrayNew { dest: rewrite_reg(dest, remap), elems: elems.into_iter().map(|r| rewrite_reg(r, remap)).collect() },
        IROp::LoadGlobal { dest, src } => IROp::LoadGlobal { dest: rewrite_reg(dest, remap), src: rewrite_reg(src, remap) },
        IROp::ArrayGet { dest, array, index } => IROp::ArrayGet { dest: rewrite_reg(dest, remap), array: rewrite_reg(array, remap), index: rewrite_reg(index, remap) },
        IROp::ArraySet { array, index, src } => IROp::ArraySet { array: rewrite_reg(array, remap), index: rewrite_reg(index, remap), src: rewrite_reg(src, remap) },
        IROp::GetProp { dest, obj, key } => IROp::GetProp { dest: rewrite_reg(dest, remap), obj: rewrite_reg(obj, remap), key: rewrite_reg(key, remap) },
        IROp::SetProp { obj, key, src } => IROp::SetProp { obj: rewrite_reg(obj, remap), key: rewrite_reg(key, remap), src: rewrite_reg(src, remap) },
        IROp::CallLabel { dest, label_index, args } => IROp::CallLabel { dest: rewrite_reg(dest, remap), label_index, args: args.into_iter().map(|r| rewrite_reg(r, remap)).collect() },
        IROp::PluginCall { dest, plugin_name, func_name, args } => IROp::PluginCall { dest: dest.map(|d| rewrite_reg(d, remap)), plugin_name, func_name, args: args.into_iter().map(|r| rewrite_reg(r, remap)).collect() },
        IROp::Ret { src } => IROp::Ret { src: rewrite_reg(src, remap) },
    }
}
