//! file: core/src/ir/module.rs
//! description: IR module container and helpers.
//!
//! `IrModule` is the main container for lowered instructions (IROp). It
//! manages register allocation, label positions, declared functions/objects
//! and provides helper APIs used by optimizations and the final bytecode
//! emitter.
//!
use crate::ir::{ op::IROp };
use std::collections::{HashMap, HashSet};

/// # IrModule
/// Contains a sequence of IR operations along with metadata
/// such as declared functions, objects, labels, and register allocation.
/// 
/// # Notes
/// This structure is used during the lowering phase to build up the IR
/// representation of a module before final bytecode emission. It also
/// provides helper methods for optimizations and analyses that operate
/// on the IR.
#[derive(Debug, Clone, Default)]
pub struct IrModule {
    /// Sequence of IR operations in the module.
    pub ops: Vec<IROp>,
    /// Next available virtual register index.
    next_reg: usize,
    /// Next available function/object id.
    next_id: u32,
    /// Mapping of declared function names to their ids.
    functions: HashMap<String, u32>,
    /// Mapping of declared object names to their ids.
    objects: HashMap<String, u32>,
    /// Mapping of label names to their positions in `ops`.
    labels: HashMap<String, usize>,
    /// List of unresolved branches to patch later.
    unresolved_branches: Vec<(usize, String)>,
    /// Registers that are intended to be externally-observable (e.g. plugin
    /// call arguments or plugin call results). Optimizations should treat
    /// these registers as live so we don't remove values expected by hosts
    /// or plugins.
    externally_visible_regs: HashSet<usize>,
}

impl IrModule {
    pub fn new() -> Self {
        IrModule {
            ops: Vec::new(),
            next_reg: 0,
            next_id: 1,
            functions: HashMap::new(),
            objects: HashMap::new(),
            labels: HashMap::new(),
            unresolved_branches: Vec::new(),
            externally_visible_regs: HashSet::new(),
        }
    }

    /// Allocate a new virtual register and return its index.
    pub fn alloc_reg(&mut self) -> usize {
        let r = self.next_reg;
        self.next_reg = self.next_reg.wrapping_add(1);
        r
    }

    /// Emit an IR operation into the module.
    pub fn emit_op(&mut self, op: IROp) {
        // record the index where the op will be inserted
        let idx = self.ops.len();
        // push the op
        self.ops.push(op.clone());
        // if this op is a Label, record its position for later patching
        if let IROp::Label { name } = &op {
            self.labels.insert(name.clone(), idx);
        }
        // Record any registers that are externally visible via plugin calls
        if let IROp::PluginCall { dest, plugin_name: _, func_name: _, args } = &op {
            for a in args.iter() { self.externally_visible_regs.insert(*a); }
            if let Some(d) = dest { self.externally_visible_regs.insert(*d); }
        }
    }

    /// Mark a register as externally visible (for use by lowering/tests).
    pub fn mark_externally_visible(&mut self, reg: usize) {
        self.externally_visible_regs.insert(reg);
    }

    /// Replace the externally-visible register set with `new_set`.
    /// This is useful after remapping/canonicalization so the metadata
    /// exactly reflects the canonical registers present in `ops`.
    pub fn set_externally_visible(&mut self, new_set: std::collections::HashSet<usize>) {
        self.externally_visible_regs = new_set;
    }

    /// Accessor for externally-visible registers.
    pub fn get_externally_visible(&self) -> &HashSet<usize> {
        &self.externally_visible_regs
    }

    pub fn peek_op(&self) -> Option<&IROp> {
        self.ops.last()
    }

    pub fn pop_op(&mut self) -> Option<IROp> {
        self.ops.pop()
    }

    pub fn get_ops(&self) -> &Vec<IROp> {
        &self.ops
    }

    pub fn len(&self) -> usize {
        self.ops.len()
    }

    /// Check whether any previously-emitted op wrote to the given register
    /// index. Used by finalization to avoid remapping operands that refer to
    /// module-level registers.
    pub fn reg_has_writer(&self, reg: usize) -> bool {
        for op in self.ops.iter() {
            match op {
                IROp::LConst { dest, .. } if *dest == reg => return true,
                IROp::ArrayNew { dest, .. } if *dest == reg => return true,
                IROp::ArrayGet { dest, .. } if *dest == reg => return true,
                IROp::CallLabel { dest, .. } if *dest == reg => return true,
                IROp::Add { dest, .. } if *dest == reg => return true,
                IROp::Lt { dest, .. } if *dest == reg => return true,
                IROp::LLocal { dest, .. } if *dest == reg => return true,
                IROp::Ret { src } if *src == reg => return true,
                _ => {}
            }
        }
        false
    }

    /// Record an unresolved branch that should be patched to a label later.
    /// `op_index` is the index of the branch op in `ops`, and `label_name` is
    /// the `IROp::Label` name that will be emitted later at the final target
    /// position.
    pub fn record_unresolved_branch(&mut self, op_index: usize, label_name: String) {
        self.unresolved_branches.push((op_index, label_name));
    }

    /// Patch any unresolved branches recorded earlier. This resolves branch
    /// placeholders (which were emitted with a dummy target) to the final
    /// op indices where the corresponding `Label` ops were emitted.
    pub fn patch_unresolved_branches(&mut self) {
        for (op_index, label_name) in self.unresolved_branches.drain(..) {
            eprintln!("[ir] resolving branch at {} -> '{}'", op_index, label_name);
            if let Some(&target_idx) = self.labels.get(&label_name) {
                if op_index < self.ops.len() {
                    match &mut self.ops[op_index] {
                        IROp::BrFalse { cond: _, target } => { *target = target_idx; }
                        IROp::BrTrue { cond: _, target } => { *target = target_idx; }
                        IROp::Jump { target } => { *target = target_idx; }
                        other => {
                            eprintln!("[ir] attempted to patch non-branch op at {}: {}", op_index, other);
                        }
                    }
                } else {
                    eprintln!("[ir] unresolved branch op_index out of range: {}", op_index);
                }
            } else {
                eprintln!("[ir] unresolved branch: label '{}' not found", label_name);
            }
        }
        // Fallback: any remaining branch ops with target==0 likely point to
        // the next label emitted after them. Patch those automatically by
        // searching forward for a Label op.
        let mut patched_fallback = 0usize;
        for i in 0..self.ops.len() {
            // inspect op immutably first to avoid mutable/immutable borrow conflicts
            match &self.ops[i] {
                IROp::BrFalse { cond: _, target } if *target == 0 => {
                    // find next label immutably
                    if let Some(j) = (i+1..self.ops.len()).find(|&k| matches!(&self.ops[k], IROp::Label { .. })) {
                        // now mutate the op to set target
                        if let IROp::BrFalse { cond: _, target: tgt } = &mut self.ops[i] { *tgt = j; patched_fallback += 1; }
                    } else {
                        eprintln!("[ir] fallback patch: no label found after op {}", i);
                    }
                }
                IROp::BrTrue { cond: _, target } if *target == 0 => {
                    if let Some(j) = (i+1..self.ops.len()).find(|&k| matches!(&self.ops[k], IROp::Label { .. })) {
                        if let IROp::BrTrue { cond: _, target: tgt } = &mut self.ops[i] { *tgt = j; patched_fallback += 1; }
                    } else {
                        eprintln!("[ir] fallback patch: no label found after op {}", i);
                    }
                }
                IROp::Jump { target } if *target == 0 => {
                    if let Some(j) = (i+1..self.ops.len()).find(|&k| matches!(&self.ops[k], IROp::Label { .. })) {
                        if let IROp::Jump { target: tgt } = &mut self.ops[i] { *tgt = j; patched_fallback += 1; }
                    } else {
                        eprintln!("[ir] fallback patch: no label found after op {}", i);
                    }
                }
                _ => {}
            }
        }
        if patched_fallback > 0 {
            eprintln!("[ir] fallback patched {} branch(es)", patched_fallback);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Declare a function prototype in the module and return a function id.
    /// This is a thin registration API intended for lowering to reserve
    /// function identifiers before emitting bodies. The current implementation
    /// stores the name and returns a numeric id; expand this to store
    /// prototype metadata as needed.
    pub fn declare_function(&mut self, name: &str) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
            self.functions.insert(name.to_string(), id);
        id
    }

    /// Declare an object (workspace/project) and return an object id.
    pub fn declare_object(&mut self, name: &str) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.objects.insert(name.to_string(), id);
        id
    }

    /// Optional helpers to inspect declared names (useful for tests/debugging).
    pub fn get_function_name(&self, id: u32) -> Option<&str> {
        // find the function name by its id (reverse lookup)
        self.functions.iter().find(|(_, v)| **v == id).map(|(k, _)| k.as_str())
    }

    pub fn get_object_name(&self, id: u32) -> Option<&str> {
        // find the object name by its id (reverse lookup)
        self.objects.iter().find(|(_, v)| **v == id).map(|(k, _)| k.as_str())
    }

    pub fn find_object_id_by_name(&self, name: &str) -> Option<u32> {
        self.objects.get(name).copied()
    }

    pub fn find_function_id_by_name(&self, name: &str) -> Option<u32> {
        self.functions.get(name).copied()
    }

    /// Return a mapping op_index -> human-readable local name for any
    /// `SLocal` ops present in the module. Since the lowering phase
    /// does not keep original identifier names in the module, this
    /// function returns a best-effort synthesized name of the form
    /// `local[<idx>]` keyed by the op index where the `SLocal` appears.
    pub fn get_op_slocal_name(&self) -> std::collections::HashMap<usize, String> {
        let mut map: std::collections::HashMap<usize, String> = std::collections::HashMap::new();
        for (i, op) in self.ops.iter().enumerate() {
            if let IROp::SLocal { src: _, local_index } = op {
                map.insert(i, format!("local[{}]", local_index));
            }
        }
        map
    }

    /// Return a mapping of declared stage/function names -> module op
    /// index of the corresponding entry `Label` op. This uses the
    /// function id recorded when the function was declared: `L{fid-1}`
    /// is the label name emitted for that function. Only entries for
    /// which a label op exists are returned.
    pub fn get_stage_labels(&self) -> std::collections::HashMap<String, usize> {
        let mut out = std::collections::HashMap::new();
        for (name, &id) in self.functions.iter() {
            let label_name = format!("L{}", (id as usize).saturating_sub(1));
            if let Some(&idx) = self.labels.get(&label_name) {
                out.insert(name.clone(), idx);
            }
        }
        out
    }

    /// Attempt to infer stage parameter *names* for the given `stage`.
    /// The lowering stage does not persist original parameter names in
    /// the `IrModule`, so this routine synthesizes placeholder names
    /// `arg0..argN` when it can determine a local count by scanning the
    /// function body for `LLocal`/`SLocal` usages. Returns `None` when
    /// the stage is unknown or no locals are found.
    pub fn get_stage_param_names(&self, stage: &str) -> Option<Vec<String>> {
        let func_id = self.functions.get(stage)?;
        let label_name = format!("L{}", ( *func_id as usize ).saturating_sub(1));
        let label_pos = *self.labels.get(&label_name)?;

        // Determine function body end (next Label or end-of-ops)
        let end = (label_pos + 1..self.ops.len()).find(|&i| matches!(&self.ops[i], IROp::Label { .. })).unwrap_or(self.ops.len());

        // Find max local_index used in this function body
        let mut max_local: Option<usize> = None;
        for op in &self.ops[label_pos + 1..end] {
            match op {
                IROp::LLocal { dest: _, local_index } | IROp::SLocal { src: _, local_index } => {
                    max_local = Some(match max_local { Some(m) => (*local_index).max(m), None => *local_index });
                }
                _ => {}
            }
        }
        let count = max_local.map(|m| m + 1)?;
        let names = (0..count).map(|i| format!("arg{}", i)).collect();
        Some(names)
    }

    /// Attempt to determine module-level register indices that correspond
    /// to function-local parameter slots for `stage`. For each inferred
    /// parameter index `0..N`, this finds the first `LLocal` op inside
    /// the function body that targets that local and returns its
    /// `dest` (the module register holding the local). Returns `None`
    /// if the stage is unknown or if no local usage is found.
    pub fn get_stage_param_local_indices(&self, stage: &str) -> Option<Vec<usize>> {
        let func_id = self.functions.get(stage)?;
        let label_name = format!("L{}", ( *func_id as usize ).saturating_sub(1));
        let label_pos = *self.labels.get(&label_name)?;

        let end = (label_pos + 1..self.ops.len()).find(|&i| matches!(&self.ops[i], IROp::Label { .. })).unwrap_or(self.ops.len());

        // Collect mapping local_index -> first dest register seen for LLocal
        let mut first_dest_for_local: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
        let mut max_local: Option<usize> = None;
        for op in &self.ops[label_pos + 1..end] {
            match op {
                IROp::LLocal { dest, local_index } => {
                    first_dest_for_local.entry(*local_index).or_insert(*dest);
                    max_local = Some(match max_local { Some(m) => (*local_index).max(m), None => *local_index });
                }
                IROp::SLocal { src: _, local_index } => {
                    max_local = Some(match max_local { Some(m) => (*local_index).max(m), None => *local_index });
                }
                _ => {}
            }
        }
        let count = max_local.map(|m| m + 1)?;
        let mut out: Vec<usize> = Vec::with_capacity(count);
        for i in 0..count {
            if let Some(&r) = first_dest_for_local.get(&i) {
                out.push(r);
            } else {
                // missing mapping for this param -> push a sentinel (usize::MAX)
                out.push(usize::MAX);
            }
        }
        Some(out)
    }
}

impl std::fmt::Display for IrModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, op) in self.ops.iter().enumerate() {
            writeln!(f, "{:04}: {}", i, op)?;
        }
        Ok(())
    }
}