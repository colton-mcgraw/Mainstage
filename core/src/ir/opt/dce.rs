//! file: core/src/ir/opt/dce.rs
//! description: dead-code-elimination optimizer pass.
//!
//! Performs a conservative backward liveness analysis to remove dead/pure
//! instructions while preserving side-effects and plugin/host-visible
//! registers.
use crate::ir::module::IrModule;
use crate::ir::op::IROp;

/// Conservative dead code elimination.
/// - Performs a backward liveness analysis on registers and local slots.
/// - Removes pure ops whose destination registers are not live and which
///   have no side effects.
/// - Removes `SLocal` stores when the local slot is not subsequently read.
pub(crate) fn dce(ir: &mut IrModule) {
    use std::collections::{HashSet, VecDeque, HashMap};

    // (no early-skip) we now handle plugin args/results via liveness seeding

    let mut live_regs: HashSet<usize> = HashSet::new();
    let mut used_locals: HashSet<usize> = HashSet::new();

    // Keep ops in a buffer, we'll scan backwards
    let mut kept: Vec<bool> = vec![false; ir.ops.len()];

    // Helper: mark a register as used
    let mark_reg = |r: usize, live_regs: &mut HashSet<usize>| { live_regs.insert(r); };

    // Seed liveness with any registers that are externally visible (e.g.
    // plugin call arguments or results) so we don't remove values needed by
    // host/plugin boundaries. Also, explicitly mark plugin call args as
    // live in case the `externally_visible_regs` set wasn't populated.
    for &r in ir.get_externally_visible().iter() { live_regs.insert(r); }
    for op in ir.get_ops().iter() {
        if let IROp::PluginCall { dest: _, plugin_name: _, func_name: _, args } = op {
            for a in args.iter() { live_regs.insert(*a); }
        }
    }
    // Build writer lists so we can find the last writer before a use-site.
    let mut writers: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut slocal_writers: HashMap<usize, Vec<usize>> = HashMap::new();
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
            => { writers.entry(*dest).or_default().push(i); }
            IROp::PluginCall { dest: Some(d), .. } => { writers.entry(*d).or_default().push(i); }
            IROp::SLocal { src: _, local_index } => { slocal_writers.entry(*local_index).or_default().push(i); }
            _ => {}
        }
    }

    // Worklist seeded from uses: (reg, use_index). use_index is the op index
    // where the register is observed (e.g. the plugin call index). For
    // externally-visible regs without a specific use-site, we use ops.len()
    // so we pick the last writer.
    let mut work: VecDeque<(usize, usize)> = VecDeque::new();
    let last_index = ir.ops.len();
    for (i, op) in ir.ops.iter().enumerate() {
        match op {
            IROp::PluginCall { dest, args, .. } => {
                if let Some(d) = dest { work.push_back((*d, i)); }
                for a in args.iter() { work.push_back((*a, i)); }
            }
            IROp::CallLabel { dest: _, label_index: _, args } => {
                for a in args.iter() { work.push_back((*a, i)); }
            }
            IROp::BrTrue { cond, .. } => { work.push_back((*cond, i)); }
            IROp::BrFalse { cond, .. } => { work.push_back((*cond, i)); }
            IROp::SetProp { obj, key, src } => { work.push_back((*obj, i)); work.push_back((*key, i)); work.push_back((*src, i)); }
            IROp::CStore { closure, src, .. } => { work.push_back((*closure, i)); work.push_back((*src, i)); }
            IROp::ArraySet { array, index, src } => { work.push_back((*array, i)); work.push_back((*index, i)); work.push_back((*src, i)); }
            IROp::GetProp { dest: _, obj, key } => { work.push_back((*obj, i)); work.push_back((*key, i)); }
            IROp::ArrayGet { dest: _, array, index } => { work.push_back((*array, i)); work.push_back((*index, i)); }
            IROp::LoadGlobal { dest: _, src } => { work.push_back((*src, i)); }
            IROp::CLoad { dest: _, closure, .. } => { work.push_back((*closure, i)); }
            _ => {}
        }
    }
    // Also seed from module-declared externally visible regs (use_index = end)
    for &r in ir.get_externally_visible().iter() { work.push_back((r, last_index)); }

    // Trace producers: for each (reg, use_idx), find the writer that occurs
    // before the use index and enqueue its inputs. Similarly handle SLocal
    // writers by choosing the store before the load.
    while let Some((r, use_idx)) = work.pop_front() {
        if !live_regs.insert(r) { continue; }

        // find the writer for `r` whose index is < use_idx (the nearest
        // preceding writer). writers entries are ascending by construction.
        if let Some(vec) = writers.get(&r) {
            // walk backwards to find first writer < use_idx
            if let Some(&idx) = vec.iter().rev().find(|&&idx| idx < use_idx) {
                match &ir.ops[idx] {
                    IROp::LConst { .. } => { /* no inputs */ }
                    IROp::LLocal { dest: _, local_index } => {
                        if let Some(svec) = slocal_writers.get(local_index) {
                            if let Some(&sidx) = svec.iter().rev().find(|&&sidx| sidx < idx) {
                                if let IROp::SLocal { src, local_index: _ } = &ir.ops[sidx] {
                                    work.push_back((*src, sidx));
                                    used_locals.insert(*local_index);
                                }
                            }
                        }
                    }
                    IROp::SLocal { src, local_index } => { work.push_back((*src, idx)); used_locals.insert(*local_index); }
                    IROp::Add { src1, src2, .. }
                    | IROp::Sub { src1, src2, .. }
                    | IROp::Mul { src1, src2, .. }
                    | IROp::Div { src1, src2, .. }
                    | IROp::Mod { src1, src2, .. }
                    | IROp::Eq { src1, src2, .. }
                    | IROp::Neq { src1, src2, .. }
                    | IROp::Lt { src1, src2, .. }
                    | IROp::Lte { src1, src2, .. }
                    | IROp::Gt { src1, src2, .. }
                    | IROp::Gte { src1, src2, .. }
                    | IROp::And { src1, src2, .. }
                    | IROp::Or { src1, src2, .. } => { work.push_back((*src1, idx)); work.push_back((*src2, idx)); }
                    IROp::Not { src, .. } => { work.push_back((*src, idx)); }
                    IROp::ArrayNew { elems, .. } => { for e in elems.iter() { work.push_back((*e, idx)); } }
                    IROp::ArrayGet { array, index, .. } => { work.push_back((*array, idx)); work.push_back((*index, idx)); }
                    IROp::GetProp { obj, key, .. } => { work.push_back((*obj, idx)); work.push_back((*key, idx)); }
                    IROp::LoadGlobal { src, .. } => { work.push_back((*src, idx)); }
                    IROp::CLoad { closure, .. } => { work.push_back((*closure, idx)); }
                    IROp::CallLabel { args, .. } => { for a in args.iter() { work.push_back((*a, idx)); } }
                    IROp::PluginCall { dest: _, args, .. } => { for a in args.iter() { work.push_back((*a, idx)); } }
                    IROp::ArraySet { array, index, src } => { work.push_back((*array, idx)); work.push_back((*index, idx)); work.push_back((*src, idx)); }
                    IROp::SetProp { obj, key, src } => { work.push_back((*obj, idx)); work.push_back((*key, idx)); work.push_back((*src, idx)); }
                    IROp::CStore { closure, src, .. } => { work.push_back((*closure, idx)); work.push_back((*src, idx)); }
                    IROp::Inc { .. } | IROp::Dec { .. } | IROp::AllocClosure { .. } | IROp::Label { .. } | IROp::Jump { .. } | IROp::BrTrue { .. } | IROp::BrFalse { .. } | IROp::Halt | IROp::Ret { .. } => { /* conservatively no extra inputs */ }
                }
            }
        }
    }

    // Walk ops backwards
    for (i, op) in ir.ops.iter().enumerate().rev() {
        let mut keep = false;
        match op {
            // Control flow / side-effecting ops: always keep. For conditional
            // branches, mark the condition register as used so we don't drop
            // the ops that compute it.
            IROp::Label { .. } | IROp::Jump { .. } | IROp::Halt | IROp::Ret { .. } => {
                keep = true;
            }
            IROp::BrTrue { cond, .. } => {
                keep = true;
                mark_reg(*cond, &mut live_regs);
            }
            IROp::BrFalse { cond, .. } => {
                keep = true;
                mark_reg(*cond, &mut live_regs);
            }
            
            IROp::CallLabel { dest: _, label_index: _, args } => {
                keep = true;
                for a in args.iter() { mark_reg(*a, &mut live_regs); }
            }
            IROp::PluginCall { dest, plugin_name: _, func_name: _, args } => {
                // Plugin calls may have side effects even without dest
                keep = true;
                if let Some(d) = dest { mark_reg(*d, &mut live_regs); }
                for a in args.iter() { mark_reg(*a, &mut live_regs); }
            }
            // Stores with side effects: SetProp, CStore, ArraySet
            IROp::SetProp { obj, key, src } => { keep = true; mark_reg(*obj, &mut live_regs); mark_reg(*key, &mut live_regs); mark_reg(*src, &mut live_regs); }
            IROp::CStore { closure, field: _, src } => { keep = true; mark_reg(*closure, &mut live_regs); mark_reg(*src, &mut live_regs); }
            IROp::ArraySet { array, index, src } => { keep = true; mark_reg(*array, &mut live_regs); mark_reg(*index, &mut live_regs); mark_reg(*src, &mut live_regs); }

            // LConst: keep if dest is live
            IROp::LConst { dest, value: _ } => {
                if live_regs.contains(dest) { keep = true; }
            }

            // LLocal (load local -> dest): dest reg liveness
            IROp::LLocal { dest, local_index } => {
                if live_regs.contains(dest) {
                    keep = true;
                    used_locals.insert(*local_index);
                } else {
                    // even if dest not live, the load might have been used for side-effects? assume not
                }
            }

            // SLocal: store to local slot. Keep only if local is used later.
            IROp::SLocal { src, local_index } => {
                if used_locals.contains(local_index) {
                    keep = true;
                    // reading the source makes src live
                    mark_reg(*src, &mut live_regs);
                } else {
                    // drop store
                }
            }

            // Arithmetic and pure ops: keep only if their dest is live
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
                if live_regs.contains(dest) {
                    keep = true;
                    mark_reg(*src1, &mut live_regs);
                    mark_reg(*src2, &mut live_regs);
                }
            }

            IROp::Not { dest, src } => {
                if live_regs.contains(dest) { keep = true; mark_reg(*src, &mut live_regs); }
            }

            IROp::ArrayNew { dest, elems } => {
                if live_regs.contains(dest) { keep = true; for e in elems.iter() { mark_reg(*e, &mut live_regs); } }
            }

            IROp::ArrayGet { dest, array, index } => {
                if live_regs.contains(dest) { keep = true; mark_reg(*array, &mut live_regs); mark_reg(*index, &mut live_regs); }
            }

            IROp::GetProp { dest, obj, key } => {
                if live_regs.contains(dest) { keep = true; mark_reg(*obj, &mut live_regs); mark_reg(*key, &mut live_regs); }
            }

            IROp::LoadGlobal { dest, src } => {
                if live_regs.contains(dest) { keep = true; mark_reg(*src, &mut live_regs); }
            }

            IROp::CLoad { dest, closure, field: _ } => {
                if live_regs.contains(dest) { keep = true; mark_reg(*closure, &mut live_regs); }
            }

            IROp::AllocClosure { dest } => { if live_regs.contains(dest) { keep = true; } }

            IROp::Inc { dest } | IROp::Dec { dest } => { if live_regs.contains(dest) { keep = true; } }
        }

        // If we decided to keep the op and it writes to a register, then that
        // register's liveness is satisfied here, so we can remove it from live
        // set (standard backward liveness semantics). Also, record source regs
        // were already marked above when we handled each case.
        if keep {
            match op {
                IROp::LConst { dest, .. }
                | IROp::LLocal { dest, .. }
                | IROp::AllocClosure { dest }
                | IROp::CLoad { dest, .. }
                | IROp::ArrayNew { dest, .. }
                | IROp::CallLabel { dest, .. }
                | IROp::ArrayGet { dest, .. }
                | IROp::GetProp { dest, .. }
                => {
                    live_regs.remove(dest);
                }
                IROp::PluginCall { dest: Some(d), .. } => { live_regs.remove(d); }
                _ => {}
            }
        }

        kept[i] = keep;
    }

    // Reconstruct ops keeping only those marked
    let mut new_ops: Vec<IROp> = Vec::with_capacity(ir.ops.len());
    for (i, op) in ir.ops.drain(..).enumerate() {
        if kept[i] { new_ops.push(op); }
    }

    ir.ops = new_ops;
}
