//! Symbol table used by semantic analyzer.
//!
//! This module provides `SymbolTable`, a lightweight hierarchical scope map
//! that stores `Symbol` entries, tracks object-contexts, and accumulates
//! diagnostics produced during analysis.

use super::symbol::{Symbol};
use crate::error::MainstageErrorExt;
use std::collections::HashMap;

// A single scope: name -> overload set
type Scope = HashMap<String, Vec<Symbol>>;

pub struct SymbolTable {
    pub symbols: Vec<Scope>,
    pub diagnostics: Vec<Box<dyn MainstageErrorExt>>,
    /// Parallel stack tracking whether a scope is an object/project/workspace
    /// declaration scope and, if so, the object name. This lets the analyzer
    /// skip unused-variable warnings for object fields assigned inside those
    /// declaration bodies.
    object_contexts: Vec<Option<String>>,
    /// Optional entrypoint workspace name for the script. Set during analysis
    /// if a workspace is marked with the `entrypoint` attribute; otherwise
    /// the first workspace encountered will be used.
    entrypoint: Option<String>,
}

impl SymbolTable {
    pub fn new() -> Self {
        SymbolTable {
            symbols: vec![HashMap::new()],
            diagnostics: Vec::new(),
            object_contexts: vec![None],
            entrypoint: None,
        }
    }

    /// ------- Scope Helpers -------

    pub fn enter_scope(&mut self) {
        self.symbols.push(HashMap::new());
        self.object_contexts.push(None);
    }

    pub fn exit_scope(&mut self) {
        // If this scope is an object/project/workspace declaration body, skip
        // emitting unused-variable warnings for symbols declared here because
        // they are treated as object fields (they'll be referenced via member
        // access) rather than local variables.
        let skip_warnings = match self.object_contexts.last() {
            Some(Some(_)) => true,
            _ => false,
        };

        if !skip_warnings {
            // Before popping the current scope, emit warnings for any variables
            // that were declared in this scope but never referenced.
            if let Some(current_scope) = self.symbols.last() {
                for symbols in current_scope.values() {
                    for sym in symbols {
                        if !sym.is_referenced() {
                            // Build a warning diagnostic for this unused variable.
                            let msg = format!("Variable '{}' is declared but never used", sym.name);
                            self.diagnostics.push(Box::new(
                                crate::analyzers::semantic::err::SemanticError::with(
                                    crate::error::Level::Warning,
                                    msg,
                                    "mainstage.analyzers.semantic.table.exit_scope".to_string(),
                                    sym.location(),
                                    sym.span(),
                                ),
                            ));
                        }
                    }
                }
            }
        }

        self.symbols.pop();
        self.object_contexts.pop();
    }

    /// Enter a new scope that corresponds to an object/project/workspace
    /// declaration body. The `name` should be the declared object's name.
    pub fn enter_object_scope(&mut self, name: String) {
        self.symbols.push(HashMap::new());
        self.object_contexts.push(Some(name));
    }

    /// Take and return all diagnostics collected so far.
    pub fn take_diagnostics(&mut self) -> Vec<Box<dyn MainstageErrorExt>> {
        std::mem::take(&mut self.diagnostics)
    }

    /// Return the current object declaration name, if the current scope is an
    /// object/project/workspace body. Clones the name so callers don't need
    /// mutable access.
    pub fn current_object_name(&self) -> Option<String> {
        self.object_contexts.last().and_then(|o| o.clone())
    }

    /// Set the entrypoint workspace name. Analyzer should call this once the
    /// chosen workspace name is determined.
    pub fn set_entrypoint(&mut self, name: String) {
        self.entrypoint = Some(name);
    }

    /// Get the configured entrypoint workspace name, if any.
    pub fn entrypoint(&self) -> Option<String> {
        self.entrypoint.clone()
    }

    /// ------- Symbol Helpers -------

    pub fn insert_symbol(&mut self, symbol: Symbol) {
        if let Some(current_scope) = self.symbols.last_mut() {
            current_scope
                .entry(symbol.name.clone())
                .or_insert_with(Vec::new)
                .push(symbol);
        }
    }

    /// Return the most-recent (visible) symbol for `name` as a mutable reference.
    /// This is a convenience wrapper over `lookup_symbol_mut(...).and_then(|v| v.last_mut())`.
    pub fn get_latest_symbol_mut(&mut self, name: &str) -> Option<&mut Symbol> {
        for scope in self.symbols.iter_mut().rev() {
            if let Some(symbols) = scope.get_mut(name) {
                return symbols.last_mut();
            }
        }
        None
    }
}
