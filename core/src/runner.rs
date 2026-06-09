//! Phase 6 — Pipeline Runner & Failure Handling.
//!
//! Orchestrates stages in dependency order, propagates failures through the DAG,
//! and invokes pipeline-level `on_failure` / `on_success` lifecycle hooks.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::{
    ast::*,
    error::{Diagnostic, Error, Result},
    eval::{eval_expr, EvalContext, Value},
    executor::execute_steps,
    sema::AnalysisResult,
};

// ── Public API ─────────────────────────────────────────────────────────────────

/// Run a pipeline from `program`.
///
/// - `pipeline_name` — the pipeline to run; `None` selects the `default pipeline`.
/// - `ctx` — the fully-evaluated program context produced by [`eval_program`](crate::eval::eval_program).
/// - `analysis` — the `AnalysisResult` from [`analyze`](crate::sema::analyze), supplying
///   the stage dependency graph used for topological ordering.
///
/// Stages execute in dependency order. When a stage fails:
/// 1. Its `on_failure` steps run immediately.
/// 2. Unless `allow_failure: true`, all stages that depend (directly or transitively)
///    on its outputs are cancelled.
/// 3. After all stages complete, the pipeline's `on_failure` hook runs with `failed_stage`
///    bound to the first failing stage name, and an error is returned.
///
/// When all stages succeed, the pipeline's `on_success` hook runs and `Ok(())` is returned.
pub fn run_pipeline(
    program: &Program,
    pipeline_name: Option<&str>,
    ctx: &EvalContext,
    analysis: &AnalysisResult,
) -> Result<()> {
    let pipeline    = find_pipeline(program, pipeline_name)?;
    let stage_names = pipeline_stage_names(pipeline, ctx)?;
    let sorted      = toposort(&stage_names, &analysis.dependency_graph)?;

    // Track stages that have failed or been cancelled so dependents can be skipped.
    let mut cancelled: HashSet<String> = HashSet::new();
    let mut first_failure: Option<String> = None;

    for stage_name in &sorted {
        // Skip if any dependency in this pipeline already failed or was cancelled.
        let dep_failed = analysis
            .dependency_graph
            .get(stage_name.as_str())
            .map(|deps| deps.iter().any(|d| cancelled.contains(d)))
            .unwrap_or(false);

        if dep_failed {
            cancelled.insert(stage_name.clone());
            continue;
        }

        let stage = find_stage(program, stage_name)
            .ok_or_else(|| runner_err(format!("stage '{}' listed in pipeline but not declared", stage_name)))?;

        let stage_ctx = build_stage_ctx(stage, ctx)?;

        match execute_steps(&stage.steps, &stage_ctx) {
            Ok(()) => {} // stage succeeded — continue
            Err(_) => {
                // Stage on_failure: run but do not propagate its errors.
                let _ = execute_steps(&stage.on_failure, &stage_ctx);

                if stage.allow_failure {
                    // Treat as success — downstream stages are unaffected.
                } else {
                    cancelled.insert(stage_name.clone());
                    if first_failure.is_none() {
                        first_failure = Some(stage_name.clone());
                    }
                }
            }
        }
    }

    match first_failure {
        Some(failed) => {
            // Pipeline on_failure: bind `failed_stage` and run; ignore its own errors.
            let failure_ctx = ctx.with_failed_stage(failed.clone());
            let _ = execute_steps(&pipeline.on_failure, &failure_ctx);
            Err(runner_err(format!(
                "pipeline '{}' failed: stage '{}' did not succeed",
                pipeline.name, failed
            )))
        }
        None => {
            execute_steps(&pipeline.on_success, ctx)?;
            Ok(())
        }
    }
}

// ── Pipeline lookup ────────────────────────────────────────────────────────────

fn find_pipeline<'a>(program: &'a Program, name: Option<&str>) -> Result<&'a PipelineBlock> {
    for item in &program.items {
        if let Item::Pipeline(p) = item {
            match name {
                Some(n) if p.name == n => return Ok(p),
                None if p.is_default   => return Ok(p),
                _                      => {}
            }
        }
    }
    match name {
        Some(n) => Err(runner_err(format!("no pipeline named '{}'", n))),
        None    => Err(runner_err("no default pipeline declared")),
    }
}

// ── Stage lookup ───────────────────────────────────────────────────────────────

fn find_stage<'a>(program: &'a Program, name: &str) -> Option<&'a StageBlock> {
    program.items.iter().find_map(|item| {
        if let Item::Stage(s) = item { if s.name == name { return Some(s); } }
        None
    })
}

// ── Stage names from pipeline.stages expression ───────────────────────────────

fn pipeline_stage_names(pipeline: &PipelineBlock, ctx: &EvalContext) -> Result<Vec<String>> {
    match &pipeline.stages {
        None       => Ok(Vec::new()),
        Some(expr) => collect_stage_names(expr, ctx),
    }
}

/// Walk the pipeline stages expression and collect names.
/// Bare `Ident` nodes are returned directly as their identifier string (the common
/// case: `stages: [compile, test, package]`).  Other expression forms are evaluated.
fn collect_stage_names(expr: &Expr, ctx: &EvalContext) -> Result<Vec<String>> {
    match expr {
        Expr::Ident(i) => Ok(vec![i.name.clone()]),
        Expr::List(l)  => {
            let mut names = Vec::new();
            for item in &l.items {
                names.extend(collect_stage_names(item, ctx)?);
            }
            Ok(names)
        }
        _ => match eval_expr(expr, ctx)? {
            Value::String(s) => Ok(vec![s]),
            Value::List(items) => items
                .into_iter()
                .map(|v| match v {
                    Value::String(s) => Ok(s),
                    _ => Err(runner_err("pipeline stage names must be strings")),
                })
                .collect(),
            _ => Err(runner_err("pipeline stages expression must produce a list of stage names")),
        },
    }
}

// ── Stage execution context ────────────────────────────────────────────────────

fn build_stage_ctx(stage: &StageBlock, base: &EvalContext) -> Result<EvalContext> {
    let inputs  = stage.inputs .as_ref().map(|e| eval_expr(e, base)).transpose()?;
    let outputs = stage.outputs.as_ref().map(|e| eval_expr(e, base)).transpose()?;
    Ok(base.with_stage(inputs, outputs))
}

// ── Topological sort (Kahn's algorithm) ───────────────────────────────────────

fn toposort(stages: &[String], dep_graph: &HashMap<String, Vec<String>>) -> Result<Vec<String>> {
    let stage_set: HashSet<&str> = stages.iter().map(String::as_str).collect();

    // in_degree[s] = number of pipeline-member stages that s directly depends on.
    let mut in_degree: HashMap<&str, usize> =
        stages.iter().map(|s| (s.as_str(), 0usize)).collect();
    // reverse_adj[d] = pipeline stages that directly depend on d.
    let mut reverse_adj: HashMap<&str, Vec<&str>> =
        stages.iter().map(|s| (s.as_str(), vec![])).collect();

    for stage in stages {
        if let Some(deps) = dep_graph.get(stage.as_str()) {
            for dep in deps {
                if stage_set.contains(dep.as_str()) {
                    *in_degree.get_mut(stage.as_str()).unwrap() += 1;
                    reverse_adj.get_mut(dep.as_str()).unwrap().push(stage.as_str());
                }
            }
        }
    }

    let mut queue: VecDeque<&str> = in_degree
        .iter()
        .filter(|&(_, d)| *d == 0)
        .map(|(&s, _)| s)
        .collect();

    let mut sorted = Vec::with_capacity(stages.len());
    while let Some(stage) = queue.pop_front() {
        sorted.push(stage.to_string());
        for &dependent in reverse_adj.get(stage).into_iter().flatten() {
            let deg = in_degree.get_mut(dependent).unwrap();
            *deg -= 1;
            if *deg == 0 {
                queue.push_back(dependent);
            }
        }
    }

    if sorted.len() != stages.len() {
        Err(runner_err("cycle detected in stage dependency graph"))
    } else {
        Ok(sorted)
    }
}

// ── Error helper ───────────────────────────────────────────────────────────────

fn runner_err(msg: impl Into<String>) -> Error {
    Error::Eval(vec![Diagnostic::new(msg)])
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(pairs: &[(&str, &[&str])]) -> HashMap<String, Vec<String>> {
        pairs
            .iter()
            .map(|(k, vs)| (k.to_string(), vs.iter().map(|s| s.to_string()).collect()))
            .collect()
    }

    fn names(ss: &[&str]) -> Vec<String> {
        ss.iter().map(|s| s.to_string()).collect()
    }

    fn pos(sorted: &[String], name: &str) -> usize {
        sorted.iter().position(|s| s == name).unwrap()
    }

    #[test]
    fn toposort_no_deps() {
        let g = graph(&[("a", &[]), ("b", &[]), ("c", &[])]);
        let sorted = toposort(&names(&["a", "b", "c"]), &g).unwrap();
        assert_eq!(sorted.len(), 3);
    }

    #[test]
    fn toposort_linear() {
        let g = graph(&[("a", &[]), ("b", &["a"]), ("c", &["b"])]);
        let sorted = toposort(&names(&["a", "b", "c"]), &g).unwrap();
        assert!(pos(&sorted, "a") < pos(&sorted, "b"));
        assert!(pos(&sorted, "b") < pos(&sorted, "c"));
    }

    #[test]
    fn toposort_diamond() {
        // d depends on b and c, both depend on a
        let g = graph(&[("a", &[]), ("b", &["a"]), ("c", &["a"]), ("d", &["b", "c"])]);
        let sorted = toposort(&names(&["a", "b", "c", "d"]), &g).unwrap();
        assert_eq!(sorted.len(), 4);
        assert!(pos(&sorted, "a") < pos(&sorted, "b"));
        assert!(pos(&sorted, "a") < pos(&sorted, "c"));
        assert!(pos(&sorted, "b") < pos(&sorted, "d"));
        assert!(pos(&sorted, "c") < pos(&sorted, "d"));
    }

    #[test]
    fn toposort_cycle_errors() {
        let g = graph(&[("a", &["b"]), ("b", &["a"])]);
        assert!(toposort(&names(&["a", "b"]), &g).is_err());
    }

    #[test]
    fn toposort_ignores_out_of_pipeline_deps() {
        // "external" is not in the stage list — its edge should be ignored
        let g = graph(&[("a", &["external"]), ("b", &["a"])]);
        let sorted = toposort(&names(&["a", "b"]), &g).unwrap();
        assert!(pos(&sorted, "a") < pos(&sorted, "b"));
    }
}
