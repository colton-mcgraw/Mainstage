//! file: core/src/ir/lower/lowering_context.rs
//! description: shared lowering context used during AST->IR lowering.
//!
//! `LoweringContext` contains the temporary maps and helper state that
//! lowering passes consume while transforming `AstNode` trees into an
//! `IrModule`. It is intentionally lightweight and in-memory; callers
//! should populate it from `AnalyzerOutput` before lowering.

use std::collections::HashMap;

use crate::analyzers::output::{AnalyzerOutput, NodeId};
use crate::ir::module::IrModule;

/// LoweringContext holds mappings and helper state that lowering passes use.
///
/// This is a lightweight, in-memory scaffold: analyzers produce an
/// `AnalyzerOutput` which lowering can consume to pre-populate symbol tables
/// and create prototypes. For now this file uses provisional numeric IDs for
/// functions/objects; replace the provisional registration with calls into
/// `IrModule` when wiring to real IR creation APIs.
#[derive(Debug, Clone)]
pub struct LoweringContext {
    pub functions: HashMap<NodeId, u32>,
    pub objects: HashMap<NodeId, u32>,
    /// Map object IR id -> runtime register so lookup by declared symbol works
    pub object_id_regs: HashMap<u32, usize>,
    pub object_regs: HashMap<NodeId, usize>,
    pub symbols: HashMap<String, u32>,
    pub functions_params: HashMap<NodeId, Vec<String>>,
    pub list_arrays: HashMap<NodeId, usize>,
    /// Temporary identifier -> reg bindings used during lowering of constructs
    /// like workspace `for x in ...` so module-level lowering can resolve
    /// the loop iterator name to the per-iteration register when needed.
    pub temp_idents: HashMap<String, usize>,
    /// When >0, module-level emission of side-effecting statements should
    /// be suppressed (used while workspace bodies are being collected so
    /// they can be lowered into wrappers instead of emitted at module scope).
    pub suppress_module_emits: usize,
    /// Registry mapping bare names (e.g., "say") to `(plugin_name, qualified_func)`
    /// so lowering can emit `PluginCall` ops without hardcoding tables.
    /// Map bare name -> list of (plugin_name, qualified_func)
    pub plugin_func_registry: HashMap<String, Vec<(String, String)>>,
    /// Alias name -> plugin name (manifest name) for alias-qualified resolution
    pub alias_to_plugin: HashMap<String, String>,
    // next_provisional_id is no longer used - IR module provides real ids
}

impl LoweringContext {
    /// Create an empty lowering context.
    pub fn new() -> Self {
        LoweringContext {
            functions: HashMap::new(),
            objects: HashMap::new(),
            symbols: HashMap::new(),
            functions_params: HashMap::new(),
            list_arrays: HashMap::new(),
            object_regs: HashMap::new(),
            object_id_regs: HashMap::new(),
            temp_idents: HashMap::new(),
            suppress_module_emits: 0,
            plugin_func_registry: HashMap::new(),
            alias_to_plugin: HashMap::new(),
        }
    }

    pub fn push_suppress_module_emits(&mut self) {
        self.suppress_module_emits = self.suppress_module_emits.saturating_add(1);
    }

    pub fn pop_suppress_module_emits(&mut self) {
        if self.suppress_module_emits > 0 {
            self.suppress_module_emits -= 1;
        }
    }

    pub fn module_emits_suppressed(&self) -> bool {
        self.suppress_module_emits > 0
    }

    /// Construct a context pre-populated from analyzer output and register the
    /// prototypes/objects with the provided `IrModule` so lowering emits can
    /// reference real IR ids instead of provisional placeholders.
    pub fn from_analyzer_output(analysis: &AnalyzerOutput, ir_mod: &mut IrModule) -> Self {
        let mut ctx = LoweringContext::default();

        for func in &analysis.functions {
            let name = func.name.as_deref().unwrap_or("<anon>");
            let id = ir_mod.declare_function(name);
            ctx.functions.insert(func.node_id, id);
            if let Some(name) = &func.name {
                ctx.symbols.insert(name.clone(), id);
            }
            // capture parameter names for later per-function lowering
            let params = func.params.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
            ctx.functions_params.insert(func.node_id, params);
        }

        for obj in &analysis.objects {
            let id = ir_mod.declare_object(&obj.name);
            ctx.objects.insert(obj.node_id, id);
            ctx.symbols.insert(obj.name.clone(), id);
        }

        // Register plugin function mappings discovered during analysis.
        for (bare, plugin, qualified) in &analysis.plugin_func_mappings {
            ctx.register_plugin_func(bare, plugin, qualified);
        }
        for (alias, plugin) in &analysis.plugin_aliases {
            ctx.alias_to_plugin.insert(alias.clone(), plugin.clone());
        }
        if !analysis.plugin_func_mappings.is_empty() {
            log::debug!("lowering: registered {} plugin function mappings from manifests", analysis.plugin_func_mappings.len());
        }
        // Pre-create module-level runtime object registers for all
        // analyzer-discovered objects. This ensures lowering can resolve
        // property Get/Set ops and static list initializers even if the
        // corresponding AST node lowering hasn't yet run or was emitted
        // in a different pass. It mirrors the behavior of
        // `lower_project_object` when projects are lowered individually.
        for obj in &analysis.objects {
            if let Some(&obj_id) = ctx.objects.get(&obj.node_id) {
                let obj_reg = ir_mod.alloc_reg();
                let empty_map: std::collections::HashMap<String, crate::ir::value::Value> =
                    std::collections::HashMap::new();
                ir_mod.emit_op(crate::ir::op::IROp::LConst {
                    dest: obj_reg,
                    value: crate::ir::value::Value::Object(empty_map),
                });
                ctx.bind_object_reg(obj.node_id, obj_reg);
                ctx.bind_object_reg_by_objid(obj_id, obj_reg);
            }
        }
        
        // Note: scopes and call_graph are available in `analysis` for more
        // advanced population of the context (scoped symbol tables, topo order).

        ctx
    }



    pub fn get_function_id(&self, node_id: NodeId) -> Option<u32> {
        self.functions.get(&node_id).copied()
    }

    pub fn get_object_id(&self, node_id: NodeId) -> Option<u32> {
        self.objects.get(&node_id).copied()
    }

    pub fn bind_function_id(&mut self, node_id: NodeId, id: u32) {
        self.functions.insert(node_id, id);
    }

    pub fn bind_object_id(&mut self, node_id: NodeId, id: u32) {
        self.objects.insert(node_id, id);
    }

    /// Bind a module-level register that will hold the runtime object for
    /// the given AST node id (workspace/project). This lets other lowering
    /// phases reference the same runtime slot for property ops.
    pub fn bind_object_reg(&mut self, node_id: NodeId, reg: usize) {
        self.object_regs.insert(node_id, reg);
    }

    /// Bind a runtime register for an object by its declared object id
    /// (the numeric id returned by `IrModule::declare_object`). This lets
    /// lowering look up object runtime slots by symbol->object id mapping.
    pub fn bind_object_reg_by_objid(&mut self, obj_id: u32, reg: usize) {
        self.object_id_regs.insert(obj_id, reg);
    }

    pub fn get_object_reg(&self, node_id: NodeId) -> Option<usize> {
        self.object_regs.get(&node_id).copied()
    }

    pub fn get_object_reg_by_objid(&self, obj_id: u32) -> Option<usize> {
        self.object_id_regs.get(&obj_id).copied()
    }

    /// Bind a statically-created list (variable name) to a module register
    /// so other lowering passes can reference the array register.
    pub fn bind_list_array(&mut self, node_id: NodeId, reg: usize) {
        self.list_arrays.insert(node_id, reg);
    }

    /// Lookup a previously bound list array register by the AST node id of
    /// the assignment target or iterable identifier.
    pub fn get_list_array(&self, node_id: NodeId) -> Option<usize> {
        self.list_arrays.get(&node_id).copied()
    }

    /// Bind a temporary identifier name to a runtime register for the
    /// duration of a lowering operation (e.g. lowering a workspace for-in
    /// body). This lets module-level lowering (which sometimes runs in the
    /// same pass) refer to the item register by name.
    pub fn bind_temp_ident(&mut self, name: &str, reg: usize) {
        self.temp_idents.insert(name.to_string(), reg);
    }

    pub fn unbind_temp_ident(&mut self, name: &str) {
        self.temp_idents.remove(name);
    }

    pub fn get_temp_ident(&self, name: &str) -> Option<usize> {
        self.temp_idents.get(name).copied()
    }

    /// Register a plugin function mapping. `bare` should be the unqualified call
    /// name (e.g., "say"), `plugin_name` the plugin identifier (e.g., "stdlib_plugin"),
    /// and `qualified_func` the domain-qualified function name (e.g., "util.say").
    pub fn register_plugin_func(&mut self, bare: &str, plugin_name: &str, qualified_func: &str) {
        self.plugin_func_registry.entry(bare.to_string()).or_default().push((plugin_name.to_string(), qualified_func.to_string()));
    }

    /// Lookup a plugin function mapping by its bare name.
    pub fn lookup_plugin_func(&self, bare: &str) -> Vec<(String, String)> {
        self.plugin_func_registry.get(bare).cloned().unwrap_or_default()
    }

    /// Lookup a plugin function mapping by its fully-qualified domain name
    /// (e.g., "util.array.append"). Returns the `(plugin_name, qualified)`
    /// pair if found.
    pub fn lookup_plugin_func_qualified(&self, qualified: &str) -> Option<(String, String)> {
        for (_bare, vec) in self.plugin_func_registry.iter() {
            for (plugin, qual) in vec.iter() {
                if qual == qualified {
                    return Some((plugin.clone(), qual.clone()));
                }
            }
        }
        None
    }
}

// Keep Default minimal: no built-in mappings. Lowering will rely on
// analyzer-provided `plugin_func_mappings` exclusively.
impl Default for LoweringContext {
    fn default() -> Self { LoweringContext::new() }
}
