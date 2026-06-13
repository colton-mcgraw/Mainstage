//! Phase 6 — Pipeline Runner & Failure Handling.
//! Phase 24 — Parallel Stage Execution.
//!
//! Orchestrates stages in dependency order, propagates failures through the DAG,
//! and invokes pipeline-level `on_failure` / `on_success` lifecycle hooks.
//!
//! Independent branches of the dependency DAG run concurrently: instead of walking a
//! single linear topological sort, stages are scheduled by *readiness* (all of their
//! dependencies have completed) across a bounded pool of worker threads. The number of
//! workers is controlled by `--jobs N`; `--jobs 1` reproduces the original sequential
//! behavior exactly, including live (unbuffered) step output. Failure propagation,
//! `allow_failure`, change detection, and the `on_failure` / `on_success` hooks behave
//! identically to the sequential runner.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};

use crate::{
    ast::*,
    cache::{self, Cache},
    error::{Diagnostic, Error, Result},
    eval::{EvalContext, OutputSink, Value, eval_expr},
    executor::execute_steps,
    sema::AnalysisResult,
};

// ── Reporting ──────────────────────────────────────────────────────────────────

/// Receives stage- and pipeline-level lifecycle events during a run so a frontend
/// (e.g. the CLI) can render progress. Every method has a default no-op body, so
/// implementors override only the events they care about.
///
/// Each method is handed a `&mut dyn Write` rather than printing directly. Under the
/// parallel runner, a stage's events — and the captured output of its steps — are
/// written to a single buffer and flushed atomically, so the output of concurrently
/// executing stages never interleaves on the terminal. The trait is `Sync` so a single
/// reporter can be shared across worker threads.
pub trait Reporter: Sync {
    /// A stage is about to execute its steps.
    fn stage_start(&self, _out: &mut dyn Write, _stage: &str) {}
    /// A stage was skipped because its inputs are unchanged and outputs present.
    fn stage_skipped(&self, _out: &mut dyn Write, _stage: &str) {}
    /// A stage finished successfully.
    fn stage_passed(&self, _out: &mut dyn Write, _stage: &str) {}
    /// A stage failed; `allow_failure` indicates whether the failure is tolerated.
    fn stage_failed(
        &self,
        _out: &mut dyn Write,
        _stage: &str,
        _error: &Error,
        _allow_failure: bool,
    ) {
    }
    /// A stage was cancelled because a dependency failed.
    fn stage_cancelled(&self, _out: &mut dyn Write, _stage: &str) {}
    /// The pipeline completed; `failed_stage` is `Some` when it failed.
    fn pipeline_finished(
        &self,
        _out: &mut dyn Write,
        _pipeline: &str,
        _failed_stage: Option<&str>,
    ) {
    }
}

/// A [`Reporter`] that does nothing — the default used by [`run_pipeline`].
pub struct NoopReporter;
impl Reporter for NoopReporter {}

// ── Public API ─────────────────────────────────────────────────────────────────

/// Run a pipeline from `program`.
///
/// - `pipeline_name` — the pipeline to run; `None` selects the `default pipeline`.
/// - `ctx` — the fully-evaluated program context produced by [`eval_program`](crate::eval::eval_program).
/// - `analysis` — the `AnalysisResult` from [`analyze`](crate::sema::analyze), supplying
///   the stage dependency graph used for scheduling.
///
/// Stages run in dependency order — independent branches concurrently across the host's
/// CPU cores. When a stage fails:
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
    run_pipeline_reported(program, pipeline_name, ctx, analysis, &NoopReporter)
}

/// Like [`run_pipeline`], but emits lifecycle events to `reporter` as stages run,
/// skip, pass, or fail. Uses the host core count as the worker budget.
pub fn run_pipeline_reported(
    program: &Program,
    pipeline_name: Option<&str>,
    ctx: &EvalContext,
    analysis: &AnalysisResult,
    reporter: &dyn Reporter,
) -> Result<()> {
    run_pipeline_reported_jobs(program, pipeline_name, ctx, analysis, reporter, default_jobs())
}

/// Like [`run_pipeline_reported`], but runs at most `jobs` stages concurrently.
///
/// `jobs == 1` forces sequential execution with live (unbuffered) step output,
/// reproducing the pre-parallel behavior exactly. `jobs == 0` is treated as `1`.
///
/// Change detection (Phase 7) is applied per stage: a stage whose resolved `inputs`
/// digest matches the cache and whose declared `outputs` all still exist is skipped.
/// The cache lives at `<script_dir>/.mainstage/cache.json` and is updated after each
/// stage, then saved (best-effort) when the run completes.
pub fn run_pipeline_reported_jobs(
    program: &Program,
    pipeline_name: Option<&str>,
    ctx: &EvalContext,
    analysis: &AnalysisResult,
    reporter: &dyn Reporter,
    jobs: usize,
) -> Result<()> {
    let pipeline = find_pipeline(program, pipeline_name)?;
    let stage_names = pipeline_stage_names(pipeline, ctx)?;
    // Reject dependency cycles up front (and validate the stage set) before scheduling.
    let sorted = toposort(&stage_names, &analysis.dependency_graph)?;

    let project_dir = ctx.script_dir.clone();
    let cache = Mutex::new(Cache::load(&project_dir));
    // Shared across stages so a file in several stages' inputs is hashed at most once.
    let run_cache = cache::RunFileCache::new();

    // Buffer and atomically flush per-stage output only when stages can actually run
    // concurrently; a single worker streams output live, preserving the sequential UX.
    let workers = jobs.max(1).min(sorted.len().max(1));
    let buffered = workers > 1;

    let outcome = schedule(
        program,
        ctx,
        analysis,
        &sorted,
        &cache,
        &run_cache,
        &project_dir,
        reporter,
        workers,
        buffered,
    );

    // A fatal error (e.g. an unresolved `<stage>.outputs` reference while building a
    // stage context) aborts the run without saving the cache, matching the sequential
    // runner's early return.
    let outcome = outcome?;

    // Persist change-detection state for every stage that succeeded this run, even when
    // the pipeline as a whole failed. Best-effort: a save failure does not fail an
    // otherwise-successful run.
    let _ = cache.into_inner().unwrap().save(&project_dir);

    match outcome.first_failure {
        Some(failed) => {
            // Pipeline on_failure: bind `failed_stage` and run; ignore its own errors.
            let failure_ctx = ctx.with_failed_stage(failed.clone());
            let _ = execute_steps(&pipeline.on_failure, &failure_ctx);
            emit(reporter, |r, out| r.pipeline_finished(out, &pipeline.name, Some(&failed)));
            Err(runner_err(format!(
                "pipeline '{}' failed: stage '{}' did not succeed",
                pipeline.name, failed
            )))
        }
        None => {
            execute_steps(&pipeline.on_success, ctx)?;
            emit(reporter, |r, out| r.pipeline_finished(out, &pipeline.name, None));
            Ok(())
        }
    }
}

/// The host's available parallelism, falling back to a single worker.
fn default_jobs() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
}

// ── Scheduler ────────────────────────────────────────────────────────────────────

/// The result of a scheduled run: the name of the first stage to fail (if any). A fatal
/// error is reported through the `Result` rather than this struct.
struct Outcome {
    first_failure: Option<String>,
}

/// Status of a stage within the scheduler.
#[derive(Clone, Copy, PartialEq, Eq)]
enum NodeState {
    /// Has unmet dependencies; not yet runnable.
    Waiting,
    /// All dependencies satisfied; queued or running.
    Active,
    /// Finished (passed, skipped, or failed) or cancelled.
    Settled,
}

/// Mutable scheduler state shared across worker threads behind a single mutex.
struct Sched {
    /// Remaining unmet in-pipeline dependencies per stage.
    in_degree: HashMap<String, usize>,
    state: HashMap<String, NodeState>,
    /// Stages ready to run (in-degree 0, not yet claimed).
    ready: VecDeque<String>,
    /// Workers currently executing a stage.
    running: usize,
    /// Stages that have settled (completed or cancelled).
    settled: usize,
    /// Resolved `outputs` of completed stages, so a dependent's `<stage>.outputs`
    /// references resolve. A dependent is only marked ready after every dependency has
    /// published here under this same lock, so it always observes them.
    resolved_outputs: HashMap<String, Value>,
    /// The first stage to fail (non-`allow_failure`), in completion order.
    first_failure: Option<String>,
    /// A fatal error that aborts the entire run.
    fatal: Option<Error>,
}

/// Immutable scheduling topology plus the shared mutable state and its condition var.
struct Shared {
    inner: Mutex<Sched>,
    cv: Condvar,
    /// `reverse_adj[d]` = pipeline stages that directly depend on `d`.
    reverse_adj: HashMap<String, Vec<String>>,
    total: usize,
}

/// Run all stages across `workers` threads, returning the run outcome or a fatal error.
#[allow(clippy::too_many_arguments)]
fn schedule(
    program: &Program,
    base: &EvalContext,
    analysis: &AnalysisResult,
    sorted: &[String],
    cache: &Mutex<Cache>,
    run_cache: &cache::RunFileCache,
    project_dir: &Path,
    reporter: &dyn Reporter,
    workers: usize,
    buffered: bool,
) -> Result<Outcome> {
    if sorted.is_empty() {
        return Ok(Outcome { first_failure: None });
    }

    let (in_degree, reverse_adj) = build_dag(sorted, &analysis.dependency_graph);

    let ready: VecDeque<String> = sorted.iter().filter(|s| in_degree[*s] == 0).cloned().collect();
    let state: HashMap<String, NodeState> = sorted
        .iter()
        .map(|s| {
            let st = if in_degree[s] == 0 { NodeState::Active } else { NodeState::Waiting };
            (s.clone(), st)
        })
        .collect();

    let shared = Shared {
        inner: Mutex::new(Sched {
            in_degree,
            state,
            ready,
            running: 0,
            settled: 0,
            resolved_outputs: HashMap::new(),
            first_failure: None,
            fatal: None,
        }),
        cv: Condvar::new(),
        reverse_adj,
        total: sorted.len(),
    };

    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                worker(&shared, program, base, cache, run_cache, project_dir, reporter, buffered);
            });
        }
    });

    let inner = shared.inner.into_inner().unwrap();
    match inner.fatal {
        Some(e) => Err(e),
        None => Ok(Outcome { first_failure: inner.first_failure }),
    }
}

/// A single worker: claim ready stages, run them, then publish results and unblock or
/// cancel dependents — looping until the run is fully settled or aborted.
#[allow(clippy::too_many_arguments)]
fn worker(
    shared: &Shared,
    program: &Program,
    base: &EvalContext,
    cache: &Mutex<Cache>,
    run_cache: &cache::RunFileCache,
    project_dir: &Path,
    reporter: &dyn Reporter,
    buffered: bool,
) {
    loop {
        // ── Claim a ready stage (or exit) ──────────────────────────────────────
        let (name, deps_outputs) = {
            let mut g = shared.inner.lock().unwrap();
            loop {
                if g.fatal.is_some() || g.settled == shared.total {
                    shared.cv.notify_all();
                    return;
                }
                if let Some(name) = g.ready.pop_front() {
                    g.running += 1;
                    // Snapshot of completed stages' outputs: every dependency of `name`
                    // has already published here, so the snapshot resolves its refs.
                    let snapshot = g.resolved_outputs.clone();
                    break (name, snapshot);
                }
                if g.running == 0 {
                    // Nothing ready and nothing running: the remainder is unreachable.
                    shared.cv.notify_all();
                    return;
                }
                g = shared.cv.wait(g).unwrap();
            }
        };

        // ── Run the stage without holding the lock ─────────────────────────────
        let stage = match find_stage(program, &name) {
            Some(s) => s,
            None => {
                let mut g = shared.inner.lock().unwrap();
                g.fatal = Some(runner_err(format!(
                    "stage '{}' listed in pipeline but not declared",
                    name
                )));
                shared.cv.notify_all();
                return;
            }
        };

        let run = run_one_stage(
            stage,
            &name,
            base,
            deps_outputs,
            cache,
            run_cache,
            project_dir,
            reporter,
            buffered,
        );

        // ── Publish the result and wake dependents ─────────────────────────────
        let mut g = shared.inner.lock().unwrap();
        g.running -= 1;
        match run {
            StageRun::Fatal(e) => {
                if g.fatal.is_none() {
                    g.fatal = Some(e);
                }
                shared.cv.notify_all();
                return;
            }
            StageRun::Done { outputs, success } => {
                g.state.insert(name.clone(), NodeState::Settled);
                g.settled += 1;
                if success {
                    if let Some(v) = outputs {
                        g.resolved_outputs.insert(name.clone(), v);
                    }
                    // Decrement dependents; any that reach in-degree 0 become ready.
                    for dep in shared.reverse_adj.get(&name).into_iter().flatten() {
                        if let Some(d) = g.in_degree.get_mut(dep) {
                            *d = d.saturating_sub(1);
                            if *d == 0 && g.state.get(dep) == Some(&NodeState::Waiting) {
                                g.state.insert(dep.clone(), NodeState::Active);
                                g.ready.push_back(dep.clone());
                            }
                        }
                    }
                } else {
                    if g.first_failure.is_none() {
                        g.first_failure = Some(name.clone());
                    }
                    cancel_dependents(&mut g, &shared.reverse_adj, &name, reporter);
                }
                shared.cv.notify_all();
            }
        }
    }
}

/// Cancel every stage that depends (directly or transitively) on `failed`, reporting
/// each cancellation once. Cancelled stages are removed from the ready queue and counted
/// as settled so the run can terminate.
fn cancel_dependents(
    g: &mut Sched,
    reverse_adj: &HashMap<String, Vec<String>>,
    failed: &str,
    reporter: &dyn Reporter,
) {
    let mut queue: VecDeque<String> =
        reverse_adj.get(failed).cloned().unwrap_or_default().into_iter().collect();
    while let Some(node) = queue.pop_front() {
        // Skip stages already settled (a running stage cannot depend on a just-failed
        // one, since a stage only runs after every dependency has succeeded).
        match g.state.get(&node) {
            Some(NodeState::Settled) | None => continue,
            _ => {}
        }
        // The node is Active-and-queued here, not running. Remove it from the ready
        // queue and cancel it.
        g.ready.retain(|n| n != &node);
        g.state.insert(node.clone(), NodeState::Settled);
        g.settled += 1;
        emit(reporter, |r, out| r.stage_cancelled(out, &node));
        for d in reverse_adj.get(&node).into_iter().flatten() {
            queue.push_back(d.clone());
        }
    }
}

/// Build the in-pipeline dependency topology: remaining in-degree per stage and the
/// reverse adjacency (dependents) used to wake stages as their dependencies complete.
/// Edges to stages outside the pipeline are ignored.
fn build_dag(
    stages: &[String],
    dep_graph: &HashMap<String, Vec<String>>,
) -> (HashMap<String, usize>, HashMap<String, Vec<String>>) {
    let stage_set: HashSet<&str> = stages.iter().map(String::as_str).collect();
    let mut in_degree: HashMap<String, usize> = stages.iter().map(|s| (s.clone(), 0)).collect();
    let mut reverse_adj: HashMap<String, Vec<String>> =
        stages.iter().map(|s| (s.clone(), Vec::new())).collect();

    for stage in stages {
        if let Some(deps) = dep_graph.get(stage) {
            for dep in deps {
                if stage_set.contains(dep.as_str()) {
                    *in_degree.get_mut(stage).unwrap() += 1;
                    reverse_adj.get_mut(dep).unwrap().push(stage.clone());
                }
            }
        }
    }
    (in_degree, reverse_adj)
}

// ── Single-stage execution ───────────────────────────────────────────────────────

/// The outcome of executing one stage.
enum StageRun {
    /// The stage settled. `success` is `true` for a pass, a skip, or an `allow_failure`
    /// failure (downstream proceeds); `false` for a real failure (downstream cancelled).
    /// `outputs` carries the stage's resolved outputs to publish for dependents.
    Done { outputs: Option<Value>, success: bool },
    /// An unrecoverable error (e.g. a context-build failure) that aborts the whole run.
    Fatal(Error),
}

/// Build the stage's context, apply change detection, run its steps (capturing output
/// when `buffered`), and report the lifecycle events. All of a stage's output — its
/// start/end markers and its captured step output — is flushed as one atomic block.
#[allow(clippy::too_many_arguments)]
fn run_one_stage(
    stage: &StageBlock,
    name: &str,
    base: &EvalContext,
    deps_outputs: HashMap<String, Value>,
    cache: &Mutex<Cache>,
    run_cache: &cache::RunFileCache,
    project_dir: &Path,
    reporter: &dyn Reporter,
    buffered: bool,
) -> StageRun {
    // Evaluate inputs/outputs with the completed dependencies' outputs in scope so that
    // `inputs: [<stage>.outputs]` resolves. A failure here aborts the run.
    let stage_ctx = match build_stage_ctx(stage, base, &deps_outputs) {
        Ok(c) => c,
        Err(e) => return StageRun::Fatal(e),
    };
    let stage_outputs_value = stage_ctx.stage_outputs.clone();

    // Change detection: fingerprint the inputs (reusing unchanged files' hashes via the
    // mtime/size fast path and the within-run cache), then skip the stage when its digest
    // is unchanged and its outputs are all present. The prior run's per-file metadata is
    // snapshotted under the lock so hashing itself happens lock-free.
    let fingerprint = stage_ctx.stage_inputs.as_ref().map(|inputs| {
        let prior = cache.lock().unwrap().input_meta(name);
        cache::fingerprint_inputs(inputs, &prior, run_cache)
    });
    let output_paths = stage_outputs_value.as_ref().map(cache::output_paths).unwrap_or_default();

    if let Some(fp) = &fingerprint {
        let fresh = cache.lock().unwrap().is_fresh(name, fp.digest(), project_dir);
        if fresh {
            emit(reporter, |r, out| r.stage_skipped(out, name));
            return StageRun::Done { outputs: stage_outputs_value, success: true };
        }
    }

    // When buffered, capture step output into a per-stage sink and assemble one block:
    // start marker, captured output, end marker. When not, stream output live and flush
    // each marker immediately.
    let sink = if buffered { Some(Arc::new(OutputSink::default())) } else { None };
    let exec_ctx = match &sink {
        Some(s) => stage_ctx.with_output(s.clone()),
        None => stage_ctx,
    };

    let mut buf = Vec::new();
    write_event(reporter, buffered, &mut buf, |r, out| r.stage_start(out, name));

    let result = execute_steps(&stage.steps, &exec_ctx);
    if result.is_err() {
        // Stage on_failure: run but do not propagate its own errors.
        let _ = execute_steps(&stage.on_failure, &exec_ctx);
    }
    if let Some(s) = &sink {
        buf.extend_from_slice(&s.take());
    }

    let run = match &result {
        Ok(()) => {
            // Record the stage as up-to-date for the next run, persisting per-file
            // metadata so the next run can take the fast path.
            if let Some(fp) = fingerprint {
                cache.lock().unwrap().update_fingerprint(name, fp, output_paths);
            }
            write_event(reporter, buffered, &mut buf, |r, out| r.stage_passed(out, name));
            StageRun::Done { outputs: stage_outputs_value, success: true }
        }
        Err(e) => {
            write_event(reporter, buffered, &mut buf, |r, out| {
                r.stage_failed(out, name, e, stage.allow_failure)
            });
            if stage.allow_failure {
                // Treat as success — the cache is not updated, so the stage re-runs next
                // time, but its declared outputs are still published for dependents.
                StageRun::Done { outputs: stage_outputs_value, success: true }
            } else {
                StageRun::Done { outputs: None, success: false }
            }
        }
    };

    flush_to_stdout(&buf);
    run
}

// ── Output helpers ───────────────────────────────────────────────────────────────

/// Render a reporter event. When buffered, append to the stage's buffer (flushed later
/// as one block); otherwise render and flush it to stdout immediately.
fn write_event(
    reporter: &dyn Reporter,
    buffered: bool,
    buf: &mut Vec<u8>,
    f: impl FnOnce(&dyn Reporter, &mut dyn Write),
) {
    if buffered {
        f(reporter, buf);
    } else {
        emit(reporter, f);
    }
}

/// Render a single reporter event and flush it to stdout atomically.
fn emit(reporter: &dyn Reporter, f: impl FnOnce(&dyn Reporter, &mut dyn Write)) {
    let mut tmp = Vec::new();
    f(reporter, &mut tmp);
    flush_to_stdout(&tmp);
}

/// Write a block to stdout under the stdout lock so concurrent stages never interleave.
fn flush_to_stdout(bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(bytes);
    let _ = out.flush();
}

// ── Pipeline lookup ────────────────────────────────────────────────────────────

fn find_pipeline<'a>(program: &'a Program, name: Option<&str>) -> Result<&'a PipelineBlock> {
    for item in &program.items {
        if let Item::Pipeline(p) = item {
            match name {
                Some(n) if p.name == n => return Ok(p),
                None if p.is_default => return Ok(p),
                _ => {}
            }
        }
    }
    match name {
        Some(n) => Err(runner_err(format!("no pipeline named '{}'", n))),
        None => Err(runner_err("no default pipeline declared")),
    }
}

// ── Stage lookup ───────────────────────────────────────────────────────────────

fn find_stage<'a>(program: &'a Program, name: &str) -> Option<&'a StageBlock> {
    program.items.iter().find_map(|item| {
        if let Item::Stage(s) = item {
            if s.name == name {
                return Some(s);
            }
        }
        None
    })
}

// ── Stage names from pipeline.stages expression ───────────────────────────────

fn pipeline_stage_names(pipeline: &PipelineBlock, ctx: &EvalContext) -> Result<Vec<String>> {
    match &pipeline.stages {
        None => Ok(Vec::new()),
        Some(expr) => collect_stage_names(expr, ctx),
    }
}

/// Walk the pipeline stages expression and collect names.
/// Bare `Ident` nodes are returned directly as their identifier string (the common
/// case: `stages: [compile, test, package]`).  Other expression forms are evaluated.
fn collect_stage_names(expr: &Expr, ctx: &EvalContext) -> Result<Vec<String>> {
    match expr {
        Expr::Ident(i) => Ok(vec![i.name.clone()]),
        Expr::List(l) => {
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

fn build_stage_ctx(
    stage: &StageBlock,
    base: &EvalContext,
    resolved_outputs: &HashMap<String, Value>,
) -> Result<EvalContext> {
    // A context in which `<stage>.outputs` references resolve to the outputs of
    // stages that have already run this pipeline.
    let with_refs = base.with_stage_outputs(resolved_outputs.clone());
    let inputs = stage.inputs.as_ref().map(|e| eval_expr(e, &with_refs)).transpose()?;
    let outputs = stage.outputs.as_ref().map(|e| eval_expr(e, &with_refs)).transpose()?;
    Ok(with_refs.with_stage(inputs, outputs))
}

// ── Topological sort (Kahn's algorithm) ───────────────────────────────────────

fn toposort(stages: &[String], dep_graph: &HashMap<String, Vec<String>>) -> Result<Vec<String>> {
    let stage_set: HashSet<&str> = stages.iter().map(String::as_str).collect();

    // in_degree[s] = number of pipeline-member stages that s directly depends on.
    let mut in_degree: HashMap<&str, usize> = stages.iter().map(|s| (s.as_str(), 0usize)).collect();
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

    let mut queue: VecDeque<&str> =
        in_degree.iter().filter(|&(_, d)| *d == 0).map(|(&s, _)| s).collect();

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

    #[test]
    fn build_dag_diamond_in_degrees() {
        let g = graph(&[("a", &[]), ("b", &["a"]), ("c", &["a"]), ("d", &["b", "c"])]);
        let (in_degree, reverse_adj) = build_dag(&names(&["a", "b", "c", "d"]), &g);
        assert_eq!(in_degree["a"], 0);
        assert_eq!(in_degree["b"], 1);
        assert_eq!(in_degree["c"], 1);
        assert_eq!(in_degree["d"], 2);
        let mut a_deps = reverse_adj["a"].clone();
        a_deps.sort();
        assert_eq!(a_deps, names(&["b", "c"]));
        assert_eq!(reverse_adj["d"], Vec::<String>::new());
    }

    #[test]
    fn build_dag_ignores_out_of_pipeline_deps() {
        let g = graph(&[("a", &["external"]), ("b", &["a"])]);
        let (in_degree, _) = build_dag(&names(&["a", "b"]), &g);
        assert_eq!(in_degree["a"], 0);
        assert_eq!(in_degree["b"], 1);
    }
}
