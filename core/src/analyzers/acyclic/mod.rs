//! file: core/src/analyzers/acyclic/mod.rs
//! description: detect cycles in the stage call graph.
//!
//! This pass walks the AST to build a call-graph of stage references and
//! reports cycles as diagnostics. It returns an error when a cycle is found.
use crate::ast::{AstNode, AstNodeKind};
use crate::error::Level;
use crate::error::MainstageErrorExt;

use std::collections::{HashMap, HashSet};

mod err;

/// Analyze the AST for cycles in the stage call-graph. A cycle is reported as
/// a SemanticError pushed into the symbol table diagnostics.
pub fn analyze_acyclic_rules(root: &AstNode) -> Result<(), Box<dyn MainstageErrorExt>> {
    // Map of stage -> set of callees (stage names)
    let mut edges: HashMap<String, HashSet<String>> = HashMap::new();
    // Map of stage -> (location, span) for diagnostics
    let mut loc_map: HashMap<
        String,
        (
            Option<crate::location::Location>,
            Option<crate::location::Span>,
        ),
    > = HashMap::new();

    // Walk the AST, collecting call edges scoped to the current stage
    fn walk(
        node: &AstNode,
        cur_stage: &mut Option<String>,
        edges: &mut HashMap<String, HashSet<String>>,
        loc_map: &mut HashMap<
            String,
            (
                Option<crate::location::Location>,
                Option<crate::location::Span>,
            ),
        >,
    ) {
        match &node.kind {
            AstNodeKind::Script { body } => {
                for n in body.iter() {
                    walk(n, cur_stage, edges, loc_map);
                }
            }
            AstNodeKind::Stage {
                name,
                args: _,
                body,
            } => {
                // enter stage
                let prev = cur_stage.clone();
                cur_stage.replace(name.clone());
                // record location for this stage
                loc_map.insert(name.clone(), (node.location.clone(), node.span.clone()));
                // ensure stage node exists in edges map
                edges.entry(name.clone()).or_default();
                walk(body, cur_stage, edges, loc_map);
                // exit stage
                *cur_stage = prev;
            }
            AstNodeKind::Call { callee, args } => {
                // Only consider simple identifier callees as stage references
                if let Some(stage_name) = cur_stage.as_ref()
                    && let AstNodeKind::Identifier { name } = &callee.kind
                {
                    edges
                        .entry(stage_name.clone())
                        .or_default()
                        .insert(name.clone());
                }
                // always walk into args
                for a in args.iter() {
                    walk(a, cur_stage, edges, loc_map);
                }
            }
            // Recurse into child nodes for general cases
            AstNodeKind::Workspace { name: _, body } => {
                walk(body, cur_stage, edges, loc_map);
            }
            AstNodeKind::Project { name: _, body } => {
                walk(body, cur_stage, edges, loc_map);
            }
            AstNodeKind::Block { statements } => {
                for s in statements.iter() {
                    walk(s, cur_stage, edges, loc_map);
                }
            }
            AstNodeKind::If { condition, body } => {
                walk(condition, cur_stage, edges, loc_map);
                walk(body, cur_stage, edges, loc_map);
            }
            AstNodeKind::IfElse {
                condition,
                if_body,
                else_body,
            } => {
                walk(condition, cur_stage, edges, loc_map);
                walk(if_body, cur_stage, edges, loc_map);
                walk(else_body, cur_stage, edges, loc_map);
            }
            AstNodeKind::ForIn {
                iterator: _,
                iterable,
                body,
            } => {
                walk(iterable, cur_stage, edges, loc_map);
                walk(body, cur_stage, edges, loc_map);
            }
            AstNodeKind::ForTo {
                initializer,
                limit,
                body,
            } => {
                walk(initializer, cur_stage, edges, loc_map);
                walk(limit, cur_stage, edges, loc_map);
                walk(body, cur_stage, edges, loc_map);
            }
            AstNodeKind::While { condition, body } => {
                walk(condition, cur_stage, edges, loc_map);
                walk(body, cur_stage, edges, loc_map);
            }
            AstNodeKind::UnaryOp { expr, .. } => {
                walk(expr, cur_stage, edges, loc_map);
            }
            AstNodeKind::BinaryOp { left, right, .. } => {
                walk(left, cur_stage, edges, loc_map);
                walk(right, cur_stage, edges, loc_map);
            }
            AstNodeKind::Assignment { target, value } => {
                walk(target, cur_stage, edges, loc_map);
                walk(value, cur_stage, edges, loc_map);
            }
            AstNodeKind::Member { object, .. } => {
                walk(object, cur_stage, edges, loc_map);
            }
            AstNodeKind::Index { object, index } => {
                walk(object, cur_stage, edges, loc_map);
                walk(index, cur_stage, edges, loc_map);
            }
            AstNodeKind::Return { value: Some(v) } => {
                walk(v, cur_stage, edges, loc_map);
            }
            AstNodeKind::Return { value: None } => {}
            AstNodeKind::Arguments { args } => {
                for a in args.iter() {
                    walk(a, cur_stage, edges, loc_map);
                }
            }
            AstNodeKind::List { elements } => {
                for e in elements.iter() {
                    walk(e, cur_stage, edges, loc_map);
                }
            }
            _ => {}
        }
    }

    walk(root, &mut None, &mut edges, &mut loc_map);

    // Now detect cycles in edges using DFS
    #[derive(PartialEq, Eq, Clone, Copy)]
    enum VisitState {
        Unseen,
        Visiting,
        Done,
    }

    let mut state: HashMap<String, VisitState> = HashMap::new();
    for k in edges.keys() {
        state.insert(k.clone(), VisitState::Unseen);
    }
    // Also include any targets that didn't appear as keys
    // Collect targets first to avoid borrowing `edges` immutably while mutating it.
    let extra_targets: Vec<String> = edges.values().flat_map(|s| s.iter().cloned()).collect();
    for t in extra_targets {
        state.entry(t.clone()).or_insert(VisitState::Unseen);
        edges.entry(t.clone()).or_default();
    }

    let mut stack: Vec<String> = Vec::new();
    let mut found_cycles: Vec<Vec<String>> = Vec::new();

    fn dfs(
        node: &str,
        edges: &HashMap<String, HashSet<String>>,
        state: &mut HashMap<String, VisitState>,
        stack: &mut Vec<String>,
        found_cycles: &mut Vec<Vec<String>>,
    ) {
        state.insert(node.to_string(), VisitState::Visiting);
        stack.push(node.to_string());

        if let Some(neighbors) = edges.get(node) {
            for n in neighbors.iter() {
                match state.get(n).cloned().unwrap_or(VisitState::Unseen) {
                    VisitState::Unseen => {
                        dfs(n, edges, state, stack, found_cycles);
                    }
                    VisitState::Visiting => {
                        // found cycle: capture the cycle slice from stack
                        if let Some(pos) = stack.iter().position(|s| s == n) {
                            let mut cycle = stack[pos..].to_vec();
                            cycle.push(n.clone());
                            found_cycles.push(cycle);
                        }
                    }
                    VisitState::Done => {}
                }
            }
        }

        stack.pop();
        state.insert(node.to_string(), VisitState::Done);
    }

    for node in state.keys().cloned().collect::<Vec<_>>() {
        if state.get(&node) == Some(&VisitState::Unseen) {
            dfs(&node, &edges, &mut state, &mut stack, &mut found_cycles);
        }
    }

    // Report cycles as diagnostics
    for cycle in found_cycles.into_iter() {
        if cycle.is_empty() {
            continue;
        }
        let human = cycle.join(" -> ");
        let msg = format!("Cycle detected in stage call-graph: {}", human);
        // Use location of the first stage in the cycle if available
        let (loc, span) = loc_map.get(&cycle[0]).cloned().unwrap_or((None, None));
        return Err(Box::new(err::AcyclicError::with(
            Level::Error,
            msg,
            "mainstage.analyzers.acyclic".to_string(),
            loc,
            span,
        )));
    }

    Ok(())
}
