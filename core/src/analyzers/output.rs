//! Analyzer output types and serializable summaries.
//!
//! This module defines `AnalyzerOutput` and related structs used to convey
//! discovered objects, functions, scope information and diagnostics from the
//! analyzer passes to later lowering stages or tooling.

use crate::location::Span;

/// Analyzer-local node identifier. Use `usize` so it can directly hold
/// `AstNode::get_id()` values without truncation.
pub type NodeId = usize;

// Use `crate::location::Span` for accurate source locations.

#[derive(Debug, Clone)]
pub struct AnalyzerOutput {
    pub objects: Vec<ObjectInfo>,
    pub functions: Vec<FunctionInfo>,
    pub scopes: Vec<ScopeInfo>,
    pub call_graph: Vec<(NodeId, NodeId)>, // caller -> callee
    pub entry_point: NodeId,
    pub diagnostics: Vec<DiagnosticInfo>,
    pub version: u32,
    /// Imported modules using star import (e.g., import "stdlib" as *)
    pub star_imports: Vec<String>,
    /// Plugin function mappings discovered from manifests to support lowering of
    /// bare calls. Each entry is (bare_name, plugin_name, qualified_func_name).
    pub plugin_func_mappings: Vec<(String, String, String)>,
    /// Alias imports mapping alias name -> plugin name (manifest name) to support
    /// alias-qualified call resolution during lowering.
    pub plugin_aliases: Vec<(String, String)>,
}

impl AnalyzerOutput {
    pub fn new() -> Self {
        AnalyzerOutput {
            objects: Vec::new(),
            functions: Vec::new(),
            scopes: Vec::new(),
            call_graph: Vec::new(),
            entry_point: 0,
            diagnostics: Vec::new(),
            version: 1,
            star_imports: Vec::new(),
            plugin_func_mappings: Vec::new(),
            plugin_aliases: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ObjectInfo {
    pub node_id: NodeId,
    pub name: String,
    pub span: Option<Span>,
    pub members: Vec<MemberInfo>,
    pub parent: Option<NodeId>,
}

#[derive(Debug, Clone)]
pub struct MemberInfo {
    pub name: String,
    pub node_id: NodeId,
    pub span: Option<Span>,
    pub kind: MemberKind,
}

#[derive(Debug, Clone)]
pub enum MemberKind {
    Variable,
    Field,
    MethodPlaceholder,
}

#[derive(Debug, Clone)]
pub struct FunctionInfo {
    pub node_id: NodeId,
    pub name: Option<String>,
    pub span: Option<Span>,
    pub params: Vec<ParamInfo>,
    /// Inferred return type (if any) from semantic analysis.
    pub return_type: Option<crate::analyzers::semantic::InferredKind>,
    pub prototype_id: Option<u32>,
    pub captures: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ParamInfo {
    pub name: String,
    pub span: Option<Span>,
    /// Optional inferred type for the parameter.
    pub ty: Option<crate::analyzers::semantic::InferredKind>,
}

#[derive(Debug, Clone)]
pub struct TypeInfo {
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct ScopeInfo {
    pub node_id: NodeId,
    pub parent: Option<NodeId>,
    pub symbols: Vec<SymbolInfo>,
}

#[derive(Debug, Clone)]
pub struct SymbolInfo {
    pub name: String,
    pub kind: SymbolKind,
    pub node_id: NodeId,
    pub span: Option<Span>,
    /// Optional inferred type for the symbol (variables/functions return type etc.)
    pub ty: Option<crate::analyzers::semantic::InferredKind>,
    /// Recorded usage locations collected by the semantic analyzer.
    pub usages: Vec<(crate::location::Location, Option<crate::location::Span>)>,
}

#[derive(Debug, Clone)]
pub enum SymbolKind {
    Object,
    Function,
    Variable,
}

#[derive(Debug, Clone)]
pub struct DiagnosticInfo {
    pub message: String,
    pub span: Option<Span>,
    pub severity: DiagnosticSeverity,
}

#[derive(Debug, Clone)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
}
