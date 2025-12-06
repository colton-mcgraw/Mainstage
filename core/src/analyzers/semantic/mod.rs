//! Semantic analyzer pass.
//!
//! This module implements the top-level semantic analysis pass which builds a
//! `SymbolTable`, infers types, and constructs an `AnalyzerOutput` summary
//! consumed by lowering and tooling.

use crate::ast::AstNode;
use crate::error::MainstageErrorExt;
use crate::vm::plugin::PluginDescriptor;
use std::collections::HashMap;
use crate::analyzers::output::{AnalyzerOutput, NodeId};

mod err;
mod kind;
mod stmt;
mod expr;
mod node;
mod symbol;
pub mod table;
mod analyzer;

pub use kind::InferredKind;

pub fn analyze_semantic_rules(ast: &mut AstNode, manifests: Option<&HashMap<String, PluginDescriptor>>) -> Result<(String, AnalyzerOutput), Vec<Box<dyn MainstageErrorExt>>> {
    // Clone manifests into the analyzer so it owns the data for the duration
    // of analysis. We clone to avoid tying lifetimes across callers.
    let cloned = manifests.map(|m| m.clone());
    let mut analyzer = analyzer::Analyzer::new_with_manifests(cloned);

    // Run the analyzer and propagate any fatal analysis error immediately.
    if let Err(e) = analyzer.analyze(ast) {
        return Err(vec![e]);
    }

    // Collect diagnostics produced by the analyzer. Only abort on errors;
    // continue building the analysis on warnings so downstream steps
    // (like call graph collection and usage recording) can still run.
    let mut diagnostics = analyzer.take_diagnostics();
    // Partition diagnostics into errors vs non-errors
    let mut errors: Vec<Box<dyn MainstageErrorExt>> = Vec::new();
    let mut non_errors: Vec<Box<dyn MainstageErrorExt>> = Vec::new();
    for d in diagnostics.drain(..) {
        // Use the common diagnostic trait to determine severity.
        let is_error = d.level() == crate::error::Level::Error || d.level() == crate::error::Level::Critical;
        if is_error { errors.push(d); } else { non_errors.push(d); }
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    // The symbol table holds the chosen entrypoint workspace name (if any).
    // Return it to the caller as a String. If no workspace was found, return
    // a diagnostic error.
    let entrypoint = analyzer.get_symbol_table().entrypoint();
    if let Some(node) = entrypoint {
        // Build a minimal AnalyzerOutput from the symbol table so lowering can
        // consume resolved symbols without re-traversing the AST. For now we
        // produce object and function entries with analyzer-local NodeIds.
        let mut analysis = AnalyzerOutput::new();

        // Assign incremental node ids for each discovered symbol
        let mut next_node_id: NodeId = 1;

        // Build initial function/object entries from the symbol table (visible symbols)
        use std::collections::HashMap;
        let mut func_name_to_node: HashMap<String, NodeId> = HashMap::new();
        
        // First pass: create function/object entries and build a name->node map
        for scope in &analyzer.get_symbol_table().symbols {
            for (name, syms) in scope.iter() {
                if let Some(sym) = syms.last() {
                    match sym.kind() {
                        crate::analyzers::semantic::symbol::SymbolKind::Object => {
                            analysis.objects.push(crate::analyzers::output::ObjectInfo {
                                node_id: next_node_id,
                                name: name.clone(),
                                span: sym.span().clone(),
                                members: Vec::new(),
                                parent: None,
                            });
                            next_node_id += 1;
                        }
                        crate::analyzers::semantic::symbol::SymbolKind::Function => {
                            analysis.functions.push(crate::analyzers::output::FunctionInfo {
                                node_id: next_node_id,
                                name: Some(name.clone()),
                                span: sym.span().clone(),
                                params: Vec::new(),
                                return_type: sym.returns().cloned(),
                                prototype_id: None,
                                captures: Vec::new(),
                            });
                            func_name_to_node.insert(name.clone(), next_node_id);
                            next_node_id += 1;
                        }
                        _ => {}
                    }
                }
            }
        }

        // Second pass: build scope info with symbol nodes
        for scope in &analyzer.get_symbol_table().symbols {
            let scope_node = next_node_id;
            next_node_id += 1;
            let mut symbols = Vec::new();
            for (name, syms) in scope.iter() {
                if let Some(sym) = syms.last() {
                    // find an existing node id for functions/objects, or allocate a new one
                    let node_id = if let Some(&nid) = func_name_to_node.get(name) {
                        nid
                    } else {
                        let nid = next_node_id; next_node_id += 1; nid
                    };
                    let kind = match sym.kind() {
                        crate::analyzers::semantic::symbol::SymbolKind::Object => crate::analyzers::output::SymbolKind::Object,
                        crate::analyzers::semantic::symbol::SymbolKind::Function => crate::analyzers::output::SymbolKind::Function,
                        crate::analyzers::semantic::symbol::SymbolKind::Variable => crate::analyzers::output::SymbolKind::Variable,
                    };
                    symbols.push(crate::analyzers::output::SymbolInfo {
                        name: name.clone(),
                        kind,
                        node_id,
                        span: sym.span().clone(),
                        ty: sym.inferred_type().cloned(),
                        usages: sym.usages.clone(),
                    });
                }
            }
            analysis.scopes.push(crate::analyzers::output::ScopeInfo { node_id: scope_node, parent: None, symbols });
        }

        // Traverse AST to fill function params and call graph.
        fn collect_from_node(
            node: &crate::ast::AstNode,
            current_func: Option<NodeId>,
            analysis: &mut AnalyzerOutput,
            func_name_to_node: &HashMap<String, NodeId>,
        ) {
            use crate::ast::AstNodeKind;

            match node.get_kind() {
                AstNodeKind::Workspace { name, body } => {
                    if let Some(fi) = analysis.objects.iter_mut().find(|o| o.name == *name) {
                        // Update node id and span to the AST node
                        let nid = node.get_id();
                        fi.node_id = nid;
                        fi.span = node.get_span().cloned();

                        // traverse body with current function = None
                        collect_from_node(body, Some(nid), analysis, func_name_to_node);
                    }
                }
                AstNodeKind::Stage { name, args, body } => {
                    // Prefer to locate the function info by name and update its
                    // node id/span if we discover the AST node for it.
                    if let Some(fi) = analysis.functions.iter_mut().find(|f| f.name.as_deref() == Some(name.as_str())) {
                        // Update node id and span to the AST node
                        let nid = node.get_id();
                        fi.node_id = nid;
                        fi.span = node.get_span().cloned();

                        // collect params from AST
                        if let Some(args_node) = args.as_ref() {
                            if let AstNodeKind::Arguments { args: param_nodes } = args_node.get_kind() {
                                let mut params = Vec::new();
                                for p in param_nodes {
                                    if let AstNodeKind::Identifier { name: pname } = p.get_kind() {
                                        params.push(crate::analyzers::output::ParamInfo {
                                            name: pname.clone(),
                                            span: p.get_span().cloned(),
                                            ty: None,
                                        });
                                    }
                                }
                                fi.params = params;
                            }
                        }

                        // record mapping from name -> node id for call graph collection
                        // (overwrite any earlier synthetic node id)
                        // Note: we don't need to update func_name_to_node map here for
                        // the outer scope since we use function name lookups by name.

                        // traverse body with current function = nid
                        collect_from_node(body, Some(nid), analysis, func_name_to_node);
                    }
                }
                AstNodeKind::Call { callee, args } => {
                    // If callee is an identifier and resolves to a known function, add edge
                    if let AstNodeKind::Identifier { name } = callee.get_kind() {
                        if let Some(target_f) = analysis.functions.iter().find(|f| f.name.as_deref() == Some(name.as_str())) {
                            let target_nid = target_f.node_id;
                            if let Some(src) = current_func {
                                analysis.call_graph.push((src, target_nid));
                            }
                        }
                    }
                    // Treat all identifier arguments as reads/usages
                    for a in args.iter() {
                        if let AstNodeKind::Identifier { name } = a.get_kind() {
                            // Find the innermost scope containing this symbol and record usage
                            for scope in &mut analysis.scopes {
                                if let Some(sym) = scope.symbols.iter_mut().find(|s| s.name == *name) {
                                    sym.usages.push((a.location.clone().unwrap_or_default(), a.span.clone()));
                                }
                            }
                        }
                    }
                    // traverse args
                    for a in args {
                        collect_from_node(a, current_func, analysis, func_name_to_node);
                    }
                }
                AstNodeKind::Index { object: array, index: _ } => {
                    // Mark base identifier as used
                    if let AstNodeKind::Identifier { name } = array.get_kind() {
                        for scope in &mut analysis.scopes {
                            if let Some(sym) = scope.symbols.iter_mut().find(|s| s.name == *name) {
                                sym.usages.push((array.location.clone().unwrap_or_default(), array.span.clone()));
                            }
                        }
                    }
                    collect_from_node(array, current_func, analysis, func_name_to_node);
                }
                AstNodeKind::Member { object, .. } => {
                    // Mark base identifier as used
                    if let AstNodeKind::Identifier { name } = object.get_kind() {
                        for scope in &mut analysis.scopes {
                            if let Some(sym) = scope.symbols.iter_mut().find(|s| s.name == *name) {
                                sym.usages.push((object.location.clone().unwrap_or_default(), object.span.clone()));
                            }
                        }
                    }
                    collect_from_node(object, current_func, analysis, func_name_to_node);
                }
                AstNodeKind::Script { body } => {
                    for b in body {
                        collect_from_node(b, current_func, analysis, func_name_to_node);
                    }
                }
                AstNodeKind::Block { statements } => {
                    for s in statements {
                        collect_from_node(s, current_func, analysis, func_name_to_node);
                    }
                }
                AstNodeKind::If { condition, body } => {
                    collect_from_node(condition, current_func, analysis, func_name_to_node);
                    collect_from_node(body, current_func, analysis, func_name_to_node);
                }
                AstNodeKind::IfElse { condition, if_body, else_body } => {
                    collect_from_node(condition, current_func, analysis, func_name_to_node);
                    collect_from_node(if_body, current_func, analysis, func_name_to_node);
                    collect_from_node(else_body, current_func, analysis, func_name_to_node);
                }
                AstNodeKind::ForIn { iterable, body, .. } => {
                    collect_from_node(iterable, current_func, analysis, func_name_to_node);
                    collect_from_node(body, current_func, analysis, func_name_to_node);
                }
                AstNodeKind::ForTo { initializer, limit, body } => {
                    collect_from_node(initializer, current_func, analysis, func_name_to_node);
                    collect_from_node(limit, current_func, analysis, func_name_to_node);
                    collect_from_node(body, current_func, analysis, func_name_to_node);
                }
                AstNodeKind::While { condition, body } => {
                    collect_from_node(condition, current_func, analysis, func_name_to_node);
                    collect_from_node(body, current_func, analysis, func_name_to_node);
                }
                AstNodeKind::UnaryOp { expr, .. } => {
                    collect_from_node(expr, current_func, analysis, func_name_to_node);
                }
                AstNodeKind::BinaryOp { left, right, .. } => {
                    collect_from_node(left, current_func, analysis, func_name_to_node);
                    collect_from_node(right, current_func, analysis, func_name_to_node);
                }
                AstNodeKind::Assignment { target, value } => {
                    collect_from_node(target, current_func, analysis, func_name_to_node);
                    collect_from_node(value, current_func, analysis, func_name_to_node);
                }
                _ => {}
            }
        }

        // Collect star imports and plugin function mappings from manifests.
        if let Some(mmap) = manifests {
            // Walk AST to find Import nodes
            fn collect_imports(node: &crate::ast::AstNode, imports: &mut Vec<(String, String)>) {
                use crate::ast::AstNodeKind;
                match node.get_kind() {
                    AstNodeKind::Import { module, alias } => {
                        imports.push((module.clone(), alias.clone()));
                    }
                    AstNodeKind::Script { body } | AstNodeKind::Block { statements: body } => {
                        for b in body {
                            collect_imports(b, imports);
                        }
                    }
                    AstNodeKind::If { condition, body } => {
                        collect_imports(condition, imports);
                        collect_imports(body, imports);
                    }
                    AstNodeKind::IfElse { condition, if_body, else_body } => {
                        collect_imports(condition, imports);
                        collect_imports(if_body, imports);
                        collect_imports(else_body, imports);
                    }
                    AstNodeKind::ForIn { iterable, body, .. } => {
                        collect_imports(iterable, imports);
                        collect_imports(body, imports);
                    }
                    AstNodeKind::ForTo { initializer, limit, body } => {
                        collect_imports(initializer, imports);
                        collect_imports(limit, imports);
                        collect_imports(body, imports);
                    }
                    AstNodeKind::While { condition, body } => {
                        collect_imports(condition, imports);
                        collect_imports(body, imports);
                    }
                    AstNodeKind::Stage { args, body, .. } => {
                        if let Some(a) = args { collect_imports(a, imports); }
                        collect_imports(body, imports);
                    }
                    AstNodeKind::Assignment { target, value } => {
                        collect_imports(target, imports);
                        collect_imports(value, imports);
                    }
                    AstNodeKind::UnaryOp { expr, .. } => collect_imports(expr, imports),
                    AstNodeKind::BinaryOp { left, right, .. } => { collect_imports(left, imports); collect_imports(right, imports); }
                    _ => {}
                }
            }
            let mut imports: Vec<(String, String)> = Vec::new();
            collect_imports(ast, &mut imports);
            for (module_raw, alias_raw) in imports.into_iter() {
                let module = module_raw.trim().trim_matches('"').to_string();
                let alias = alias_raw.trim().to_string();
                // Some parser builds incorrectly set alias to the module name for star imports.
                // Treat alias==module (possibly quoted) as star-import for robustness.
                let alias_unquoted = alias.trim_matches('"');
                let is_star = alias == "*" || alias_unquoted == module;
                if is_star {
                    analysis.star_imports.push(module.clone());
                    if let Some(desc) = mmap.get(&module) {
                        // Use runtime name: manifest.entry if provided, else manifest.name
                        let plugin_name = desc.manifest.entry.clone().unwrap_or_else(|| desc.manifest.name.clone());
                        for f in &desc.manifest.functions {
                            // Build qualified name from explicit domain if provided, else use the name as-is.
                            let qualified = if let Some(dom) = &f.domain {
                                format!("{}.{}", dom, f.name)
                            } else {
                                f.name.clone()
                            };
                            // Bare name is the final segment.
                            let bare = match qualified.rsplit_once('.') { Some((_, tail)) => tail.to_string(), None => qualified.clone() };
                            analysis.plugin_func_mappings.push((bare, plugin_name.clone(), qualified));
                        }
                    } else {
                        log::warn!("analyzer: no manifest descriptor found for star import module '{}'", module);
                    }
                } else {
                    // Alias import: record alias -> plugin name and also bare mappings
                    if let Some(desc) = mmap.get(&module) {
                        // Use runtime name: manifest.entry if provided, else manifest.name
                        let plugin_name = desc.manifest.entry.clone().unwrap_or_else(|| desc.manifest.name.clone());
                        analysis.plugin_aliases.push((alias.clone(), plugin_name.clone()));
                        for f in &desc.manifest.functions {
                            let qualified = if let Some(dom) = &f.domain { format!("{}.{}", dom, f.name) } else { f.name.clone() };
                            let bare = match qualified.rsplit_once('.') { Some((_, tail)) => tail.to_string(), None => qualified.clone() };
                            analysis.plugin_func_mappings.push((bare, plugin_name.clone(), qualified));
                        }
                    } else {
                        log::warn!("analyzer: no manifest descriptor found for alias import module '{}'", module);
                    }
                }
            }
            log::debug!("analyzer: total plugin function mappings: {}", analysis.plugin_func_mappings.len());
        }

        // Start traversal from script root
        collect_from_node(ast, None, &mut analysis, &func_name_to_node);

        // Map the analyzer-chosen entrypoint name to an AST node id. Prefer
        // a workspace explicitly marked with the "entrypoint" attribute,
        // or fall back to the name returned by the symbol table. If no
        // workspaces exist, return an error.
        use crate::ast::AstNodeKind;
        let mut workspaces: Vec<(usize, Option<String>, Vec<String>)> = Vec::new();
        fn collect_workspaces(n: &crate::ast::AstNode, out: &mut Vec<(usize, Option<String>, Vec<String>)>) {
            match n.get_kind() {
                AstNodeKind::Workspace { name, body: _ } => {
                    out.push((n.get_id(), Some(name.clone()), n.attributes.clone()));
                }
                AstNodeKind::Script { body } => {
                    for b in body {
                        collect_workspaces(b, out);
                    }
                }
                AstNodeKind::Block { statements } => {
                    for s in statements {
                        collect_workspaces(s, out);
                    }
                }
                _ => {}
            }
        }
        collect_workspaces(ast, &mut workspaces);

        if workspaces.is_empty() {
            return Err(vec![Box::new(
                err::SemanticError::with(
                    crate::error::Level::Error,
                    "No workspaces found in script; analyzer requires at least one workspace.".to_string(),
                    "mainstage.analyzers.semantic.analyze_semantic_rules".to_string(),
                    ast.location.clone(),
                    ast.span.clone(),
                ),
            )]);
        }

        // Choose entrypoint: prefer explicit attribute, then match by name,
        // otherwise pick the first workspace encountered.
        let mut chosen: Option<usize> = None;
        for (id, _name, attrs) in &workspaces {
            if attrs.iter().any(|a| a == "entrypoint") {
                chosen = Some(*id);
                break;
            }
        }
        if chosen.is_none() {
            for (id, name_opt, _attrs) in &workspaces {
                if let Some(name) = name_opt {
                    if name == &node {
                        chosen = Some(*id);
                        break;
                    }
                }
            }
        }
        if chosen.is_none() {
            chosen = Some(workspaces[0].0);
        }
        if let Some(ep_nid) = chosen {
            analysis.entry_point = ep_nid;
        }

        Ok((node, analysis))
    } else {
        Err(vec![Box::new(
            err::SemanticError::with(
                crate::error::Level::Error,
                "No entrypoint workspace found in script.".to_string(),
                "mainstage.analyzers.semantic.analyze_semantic_rules".to_string(),
                ast.location.clone(),
                ast.span.clone(),
            ),
        )])
    }
}