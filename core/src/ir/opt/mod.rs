//! IR optimizer module: constant-folding and simple interprocedural substitution
use crate::ir::module::IrModule;
use crate::ir::op::IROp;
use crate::ir::value::Value;
use std::collections::HashMap;

mod const_fold;
mod const_prop;
mod const_canon;
mod dce;

/// Run optimization passes on the lowered IR in-place.
pub fn optimize(ir: &mut IrModule) {
    // Run passes to a fixed point (or until max iterations) so chained
    // transformations can expose further opportunities. Use exact IR
    // equality to detect convergence rather than just op count.
    let mut last_ops = ir.ops.clone();
    let mut iter = 0usize;
    const MAX_ITERS: usize = 8;
    loop {
        iter += 1;
        const_canon::canonicalize_constants(ir);
        const_prop::const_prop(ir);
        const_fold::constant_fold(ir);
        // Run DCE after folding to remove newly-dead ops
        dce::dce(ir);

        // conservative interproc placeholder
        interproc_substitute(ir);
        remove_noop_jumps_and_reindex(ir);

        if ir.ops == last_ops || iter >= MAX_ITERS { break; }
        last_ops = ir.ops.clone();
    }
}


// Helper: compute simple binary op results at constant-fold time
fn compute_binop(op: &IROp, v1: &Value, v2: &Value) -> Option<Value> {
    use super::value::Value::*;
    match op {
        IROp::Add { .. } => match (v1, v2) {
            (Int(a), Int(b)) => Some(Int(a + b)),
            (Float(a), Float(b)) => Some(Float(a + b)),
            (Int(a), Float(b)) => Some(Float((*a as f64) + b)),
            (Float(a), Int(b)) => Some(Float(a + (*b as f64))),
            (Str(a), Str(b)) => Some(Str(format!("{}{}", a, b))),
            _ => None,
        },
        IROp::Sub { .. } => match (v1, v2) {
            (Int(a), Int(b)) => Some(Int(a - b)),
            (Float(a), Float(b)) => Some(Float(a - b)),
            (Int(a), Float(b)) => Some(Float((*a as f64) - b)),
            (Float(a), Int(b)) => Some(Float(a - (*b as f64))),
            _ => None,
        },
        IROp::Mul { .. } => match (v1, v2) {
            (Int(a), Int(b)) => Some(Int(a * b)),
            (Float(a), Float(b)) => Some(Float(a * b)),
            (Int(a), Float(b)) => Some(Float((*a as f64) * b)),
            (Float(a), Int(b)) => Some(Float(a * (*b as f64))),
            _ => None,
        },
        IROp::Div { .. } => match (v1, v2) {
            (Int(a), Int(b)) => if *b == 0 { None } else { Some(Int(a / b)) },
            (Float(a), Float(b)) => if *b == 0.0 { None } else { Some(Float(a / b)) },
            (Int(a), Float(b)) => if *b == 0.0 { None } else { Some(Float((*a as f64) / b)) },
            (Float(a), Int(b)) => if *b == 0 { None } else { Some(Float(a / (*b as f64))) },
            _ => None,
        },
        IROp::Mod { .. } => match (v1, v2) {
            (Int(a), Int(b)) => if *b == 0 { None } else { Some(Int(a % b)) },
            _ => None,
        },
        IROp::Eq { .. } => Some(Value::Bool(v1 == v2)),
        IROp::Neq { .. } => Some(Value::Bool(v1 != v2)),
        IROp::Lt { .. } => match (v1, v2) {
            (Int(a), Int(b)) => Some(Value::Bool(a < b)),
            (Float(a), Float(b)) => Some(Value::Bool(a < b)),
            (Int(a), Float(b)) => Some(Value::Bool((*a as f64) < *b)),
            (Float(a), Int(b)) => Some(Value::Bool(*a < (*b as f64))),
            (Str(a), Str(b)) => Some(Value::Bool(a < b)),
            _ => None,
        },
        IROp::Lte { .. } => match (v1, v2) {
            (Int(a), Int(b)) => Some(Value::Bool(a <= b)),
            (Float(a), Float(b)) => Some(Value::Bool(a <= b)),
            (Int(a), Float(b)) => Some(Value::Bool((*a as f64) <= *b)),
            (Float(a), Int(b)) => Some(Value::Bool(*a <= (*b as f64))),
            (Str(a), Str(b)) => Some(Value::Bool(a <= b)),
            _ => None,
        },
        IROp::Gt { .. } => match (v1, v2) {
            (Int(a), Int(b)) => Some(Value::Bool(a > b)),
            (Float(a), Float(b)) => Some(Value::Bool(a > b)),
            (Int(a), Float(b)) => Some(Value::Bool((*a as f64) > *b)),
            (Float(a), Int(b)) => Some(Value::Bool(*a > (*b as f64))),
            (Str(a), Str(b)) => Some(Value::Bool(a > b)),
            _ => None,
        },
        IROp::Gte { .. } => match (v1, v2) {
            (Int(a), Int(b)) => Some(Value::Bool(a >= b)),
            (Float(a), Float(b)) => Some(Value::Bool(a >= b)),
            (Int(a), Float(b)) => Some(Value::Bool((*a as f64) >= *b)),
            (Float(a), Int(b)) => Some(Value::Bool(*a >= (*b as f64))),
            (Str(a), Str(b)) => Some(Value::Bool(a >= b)),
            _ => None,
        },
        IROp::And { .. } => match (v1, v2) {
            (Value::Bool(a), Value::Bool(b)) => Some(Value::Bool(*a && *b)),
            _ => None,
        },
        IROp::Or { .. } => match (v1, v2) {
            (Value::Bool(a), Value::Bool(b)) => Some(Value::Bool(*a || *b)),
            _ => None,
        },
        _ => None,
    }
}

/// TODO: implement interprocedural substitution
fn interproc_substitute(ir: &mut IrModule) {
    // Interprocedural substitution not yet implemented; reserve the
    // function for future work and keep it a no-op so the optimizer
    // can be enabled for the local constant-fold pass.
    let _ = ir; // silence unused variable warnings
}

fn remove_noop_jumps_and_reindex(ir: &mut IrModule) {
    // Remove jumps/branches that target the next instruction (no-op)
    // and reindex all numeric targets to account for removed ops.
    let mut keep_flags: Vec<bool> = Vec::with_capacity(ir.ops.len());
    for i in 0..ir.ops.len() {
        match &ir.ops[i] {
            IROp::Jump { target } if *target == i + 1 => keep_flags.push(false),
            IROp::BrTrue { target, .. } if *target == i + 1 => keep_flags.push(false),
            IROp::BrFalse { target, .. } if *target == i + 1 => keep_flags.push(false),
            _ => keep_flags.push(true),
        }
    }

    // Build mapping old_index -> new_index
    let mut mapping: HashMap<usize, usize> = HashMap::new();
    let mut new_ops: Vec<IROp> = Vec::with_capacity(ir.ops.len());
    for (old_idx, op) in ir.ops.drain(..).enumerate() {
        if keep_flags[old_idx] {
            let new_idx = new_ops.len();
            mapping.insert(old_idx, new_idx);
            new_ops.push(op);
        }
    }

    // Now update numeric targets within new_ops
    for op in new_ops.iter_mut() {
        match op {
            IROp::Jump { target } => {
                if let Some(&n) = mapping.get(target) { *target = n; }
            }
            IROp::BrTrue { target, .. } => {
                if let Some(&n) = mapping.get(target) { *target = n; }
            }
            IROp::BrFalse { target, .. } => {
                if let Some(&n) = mapping.get(target) { *target = n; }
            }
            IROp::CallLabel { label_index, .. } => {
                if let Some(&n) = mapping.get(label_index) { *label_index = n; }
            }
            _ => {}
        }
    }

    // Updating stage label indices requires stage metadata helpers on
    // `IrModule` which are not available yet; skip that step for now.
    ir.ops = new_ops;
}
