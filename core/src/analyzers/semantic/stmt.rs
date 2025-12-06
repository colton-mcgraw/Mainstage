//! Statement-level semantic analysis helpers.
//!
//! Functions in this module analyze top-level script statements (workspaces,
//! projects, stages, imports) and populate the `SymbolTable`. The analysis may
//! consult optional plugin manifests to register imported module symbols.

use super::{
    symbol::{SymbolKind},
    table::SymbolTable,
};
use crate::analyzers::semantic::symbol::Symbol;
use crate::ast::{AstNode, AstNodeKind};
use crate::error::{Level, MainstageErrorExt};

pub(crate) fn analyze_script_statements(
    node: &mut AstNode,
    tbl: &mut SymbolTable,
    manifests: Option<&std::collections::HashMap<String, crate::vm::plugin::PluginDescriptor>>,
) -> Result<(), Box<dyn MainstageErrorExt>> {
    let script_body = match &mut node.kind {
        crate::ast::AstNodeKind::Script { body } => body,
        _ => {
            return Err(Box::new(
                crate::analyzers::semantic::err::SemanticError::with(
                    Level::Error,
                    "Expected a Script node.".to_string(),
                    "mainstage.analyzers.semantic.stmt.analyze_script_statements".to_string(),
                    node.location.clone(),
                    node.span.clone(),
                ),
            ));
        }
    };

    // Determine the workspace entrypoint: prefer a workspace explicitly
    // marked with the `entrypoint` attribute; otherwise select the first
    // Workspace node encountered in the script body.
    let mut chosen: Option<String> = None;
    let mut first_workspace: Option<String> = None;
    for stmt in script_body.iter() {
        if let AstNodeKind::Workspace { name, .. } = &stmt.kind {
            if first_workspace.is_none() {
                first_workspace = Some(name.clone());
            }
            // Check attributes vector on the node for "entrypoint"
            if stmt.attributes.iter().any(|a| a == "entrypoint") {
                chosen = Some(name.clone());
                break;
            }
        }
    }
    if chosen.is_none() {
        chosen = first_workspace.clone();
    }
    if let Some(name) = chosen {
        tbl.set_entrypoint(name);
    }

    for statement in script_body.iter_mut() {
        analyze_statement(statement, tbl, manifests)?;
    }

    Ok(())
}


fn analyze_statement(
    node: &mut AstNode,
    tbl: &mut SymbolTable,
    manifests: Option<&std::collections::HashMap<String, crate::vm::plugin::PluginDescriptor>>,
) -> Result<(), Box<dyn MainstageErrorExt>> {
    match &mut node.kind {
        AstNodeKind::Workspace { name, body } => {

            // ensure body is not empty
            check_for_empty_block(body)?;

            // register workspace in current (global) scope
            tbl.insert_symbol(Symbol::new_object(
                name.clone(),
                None,
                node.location.clone(),
                node.span.clone(),
            ));

            // analyze children inside a new object declaration scope (workspace)
            tbl.enter_object_scope(name.clone());
            super::node::analyze_node(body, tbl)?;
            tbl.exit_scope();
        }
        AstNodeKind::Project { name, body } => {

            // ensure body is not empty
            check_for_empty_block(body)?;

            // register project in current (global) scope
            tbl.insert_symbol(Symbol::new_object(
                name.clone(),
                None,
                node.location.clone(),
                node.span.clone(),
            ));

            // analyze children inside a new object declaration scope (project)
            tbl.enter_object_scope(name.clone());
            super::node::analyze_node(body, tbl)?;
            tbl.exit_scope();
        }

        AstNodeKind::Stage { name, args, body } => {

            // ensure body is not empty
            check_for_empty_block(body)?;
            
            // Build parameter symbol list (do not insert yet)
            let params_symbols = if let Some(params_node) = args {
                analyze_parameters(params_node, tbl)?
            } else {
                Vec::new()
            };

            // Insert stage symbol into global scope with parameter metadata
            tbl.insert_symbol(Symbol::new(
                name.clone(),
                SymbolKind::Function,
                None,
                None,
                node.location.clone(),
                node.span.clone(),
            ));

            // Enter stage-local scope and insert parameter symbols for use inside the body
            tbl.enter_scope();
            for p in params_symbols.iter() {
                tbl.insert_symbol(p.clone());
            }
            super::node::analyze_node(body, tbl)?;

            // Collect returns from the stage body before exiting the scope so return expressions
            // can be analyzed with access to local symbols/params.
            if let Some(returns_kind) = super::expr::collect_returns(body, tbl)? {
                // set the stage symbol's returns metadata in the global scope
                if let Some(sym) = tbl.get_latest_symbol_mut(name) {
                    // ensure we are updating the global-stage symbol specifically
                    // if the found symbol is in the current scope this will still update
                    // the most-recent visible symbol; for strict global-only update,
                    // use a dedicated get_symbol_in_global_scope_mut helper instead.
                    sym.set_returns(returns_kind);
                }
            }

            tbl.exit_scope();
        }
        AstNodeKind::Import { module, alias } => {
            // Attempt to parse module string and alias. The parser stores the raw
            // matched text; try to extract a quoted module name and an alias if present.
            let text = module.clone();

            // Trim quotes from module name and whitespace from alias
            let mod_name = text.trim().trim_matches('"').to_string();
            let alias = alias.trim().to_string();

            // If we have manifests available, register the imported module as
            // an external object in the symbol table with function symbols.
            if let Some(map) = manifests {
                if let Some(desc) = map.get(&mod_name) {
                    let star_import = alias == "*";
                    if !star_import {
                        // Insert module/object symbol under the alias name
                        tbl.insert_symbol(crate::analyzers::semantic::symbol::Symbol::new_object(
                            alias.clone(),
                            None,
                            node.location.clone(),
                            node.span.clone(),
                        ));
                    }

                    // For each function in the manifest, insert function symbols.
                    for f in &desc.manifest.functions {
                        // If star import, insert bare function name; otherwise qualified with alias.
                        let full_name = if star_import {
                            f.name.clone()
                        } else {
                            format!("{}.{}", alias, f.name)
                        };
                        let mut sym = crate::analyzers::semantic::symbol::Symbol::new(
                            full_name.clone(),
                            SymbolKind::Function,
                            None,
                            None,
                            node.location.clone(),
                            node.span.clone(),
                        );
                        // Set return type if provided by manifest (map simple kinds)
                        let return_kind = if let Some(ret) = &f.returns {
                            match ret.kind.as_deref() {
                                Some("Integer") => crate::analyzers::semantic::kind::InferredKind::integer(),
                                Some("Float") => crate::analyzers::semantic::kind::InferredKind::float(),
                                Some("String") => crate::analyzers::semantic::kind::InferredKind::string(),
                                Some("Boolean") => crate::analyzers::semantic::kind::InferredKind::boolean(),
                                Some("Object") => crate::analyzers::semantic::kind::InferredKind::new(
                                    crate::analyzers::semantic::kind::Kind::Object,
                                    crate::analyzers::semantic::kind::Origin::Unknown,
                                    node.location.clone(),
                                    node.span.clone(),
                                ),
                                _ => crate::analyzers::semantic::kind::InferredKind::dynamic(),
                            }
                        } else {
                            crate::analyzers::semantic::kind::InferredKind::dynamic()
                        };
                        sym.set_returns(return_kind);
                        tbl.insert_symbol(sym);
                    }
                } else {
                    // Manifest not found: emit a diagnostic but still create a placeholder
                    if alias != "*" {
                        tbl.insert_symbol(crate::analyzers::semantic::symbol::Symbol::new_object(
                            alias.clone(),
                            None,
                            node.location.clone(),
                            node.span.clone(),
                        ));
                    }
                }
            } else {
                // No manifests available: register placeholder object symbol for the alias
                if alias != "*" {
                    tbl.insert_symbol(crate::analyzers::semantic::symbol::Symbol::new_object(
                        alias.clone(),
                        None,
                        node.location.clone(),
                        node.span.clone(),
                    ));
                }
            }
        }
        AstNodeKind::Null => {
            // EOI emits a Null node, do nothing
        }
        _ => {
            return Err(Box::new(
                crate::analyzers::semantic::err::SemanticError::with(
                    Level::Error,
                    format!(
                        "Unsupported statement type in script body: {}\nSupport types are objects such as Workspace, Project, and Stage.",
                        node.kind
                    ),
                    "mainstage.analyzers.semantic.stmt.analyze_statement".to_string(),
                    node.location.clone(),
                    node.span.clone(),
                ),
            ));
        }
    }
    Ok(())
}

fn check_for_empty_block(
    block_node: &AstNode,
) -> Result<(), Box<dyn MainstageErrorExt>> {
    if let AstNodeKind::Block { statements } = &block_node.kind {
        if statements.is_empty() {
            return Err(Box::new(
                crate::analyzers::semantic::err::SemanticError::with(
                    Level::Error,
                    "Block cannot be empty.".to_string(),
                    "mainstage.analyzers.semantic.stmt.check_for_empty_block".to_string(),
                    block_node.location.clone(),
                    block_node.span.clone(),
                ),
            ));
        }
    }
    Ok(())
}

fn analyze_parameters(
    args: &mut AstNode,
    _tbl: &mut SymbolTable,
) -> Result<Vec<Symbol>, Box<dyn MainstageErrorExt>> {
    let mut params_symbols = Vec::new();

    if let AstNodeKind::Arguments { args } = &mut args.kind {
        for param in args.iter_mut() {
            if let AstNodeKind::Identifier { name } = &mut param.kind {
                let symbol = Symbol::new(
                    name.clone(),
                    SymbolKind::Variable,
                    None,
                    None,
                    param.location.clone(),
                    param.span.clone(),
                );
                // do NOT insert into the table here; caller will insert into the correct scope
                params_symbols.push(symbol);
            } else {
                return Err(Box::new(
                    crate::analyzers::semantic::err::SemanticError::with(
                        Level::Error,
                        "Expected Parameter node.".to_string(),
                        "mainstage.analyzers.semantic.stmt.analyze_parameters".to_string(),
                        param.location.clone(),
                        param.span.clone(),
                    ),
                ));
            }
        }
    } else {
        return Err(Box::new(
            crate::analyzers::semantic::err::SemanticError::with(
                Level::Error,
                "Expected Parameters node.".to_string(),
                "mainstage.analyzers.semantic.stmt.analyze_parameters".to_string(),
                args.location.clone(),
                args.span.clone(),
            ),
        ));
    }

    Ok(params_symbols)
}
