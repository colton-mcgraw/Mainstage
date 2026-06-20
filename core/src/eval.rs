//! Phase 3 — Expression Evaluator & Built-in Variables.
//!
//! Converts AST [`Expr`] nodes into runtime [`Value`]s given an [`EvalContext`].
//! Evaluates string literals, string interpolation, booleans, lists, glob patterns,
//! `if/else` expressions, the `platform` built-in, and `project.<field>` access.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::{
    ast::*,
    error::{Diagnostic, Error, Result, Span},
    modules::{self, ModuleRegistry},
    runner::Reporter,
};

// ── Value types ───────────────────────────────────────────────────────────────

/// A single file matched by a `glob(...)` expression.
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// Absolute path to the file.
    pub path: PathBuf,
    /// File name including extension (e.g. `"main.rs"`).
    pub name: String,
    /// File name without extension (e.g. `"main"`).
    pub stem: String,
    /// Extension without leading dot (e.g. `"rs"`); empty string if none.
    pub ext: String,
    /// Parent directory (absolute).
    pub dir: PathBuf,
}

impl FileEntry {
    pub(crate) fn from_path(path: PathBuf) -> Self {
        let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let ext = path.extension().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let dir = path.parent().unwrap_or(Path::new("")).to_path_buf();
        FileEntry { path, name, stem, ext, dir }
    }

    /// Return the value of a named file property (`.path`, `.name`, `.stem`, `.ext`, `.dir`).
    pub fn get_field(&self, field: &str) -> Option<String> {
        match field {
            "path" => Some(self.path.display().to_string()),
            "name" => Some(self.name.clone()),
            "stem" => Some(self.stem.clone()),
            "ext" => Some(self.ext.clone()),
            "dir" => Some(self.dir.display().to_string()),
            _ => None,
        }
    }
}

/// A Mainstage runtime value.
#[derive(Debug, Clone)]
pub enum Value {
    String(String),
    Int(i64),
    Bool(bool),
    List(Vec<Value>),
    FileSet(Vec<FileEntry>),
}

impl Value {
    /// Render to a string for use inside `${...}` interpolations.
    pub fn display_string(&self) -> String {
        match self {
            Value::String(s) => s.clone(),
            Value::Int(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::List(items) => {
                items.iter().map(|v| v.display_string()).collect::<Vec<_>>().join(", ")
            }
            Value::FileSet(entries) => {
                entries.iter().map(|e| e.path.display().to_string()).collect::<Vec<_>>().join(", ")
            }
        }
    }
}

// ── Output capture ──────────────────────────────────────────────────────────────

/// A per-stage buffer that captures the stdout/stderr of side-effecting steps
/// (notably the `$` exec step).
///
/// When an [`EvalContext`] carries an `OutputSink`, the executor writes a step's
/// output here instead of letting it stream to the process's inherited terminal.
/// The Phase 24 parallel runner installs one sink per stage and flushes its contents
/// atomically, so the output of concurrently-running stages never interleaves on the
/// terminal. When no sink is present (the sequential `--jobs 1` path), steps stream
/// their output live as before.
#[derive(Debug, Default)]
pub struct OutputSink {
    buf: Mutex<Vec<u8>>,
}

impl OutputSink {
    /// Append raw bytes (typically a child process's captured stdout/stderr).
    pub fn write(&self, bytes: &[u8]) {
        self.buf.lock().unwrap().extend_from_slice(bytes);
    }

    /// Drain and return everything captured so far.
    pub fn take(&self) -> Vec<u8> {
        std::mem::take(&mut self.buf.lock().unwrap())
    }
}

// ── Reporter handle (Phase 43) ────────────────────────────────────────────────────

/// A cheaply-clonable handle to the run's [`Reporter`], carried by the [`EvalContext`]
/// so the `log` step can route its message through the reporter — honoring `--quiet` and
/// the per-stage buffered output — exactly as the runner's own lifecycle events do.
///
/// Installed by the front end (the CLI) onto the base context before a run; absent for
/// library use, in which case `log` steps render nothing. Wraps the trait object so the
/// derived `Debug` of [`EvalContext`] keeps working (a `dyn Reporter` is not `Debug`).
#[derive(Clone)]
pub struct ReporterHandle(pub Arc<dyn Reporter>);

impl std::fmt::Debug for ReporterHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReporterHandle(..)")
    }
}

// ── Test harness (Phase 39) ──────────────────────────────────────────────────────

/// The outcome of a single `expect` / `assert` step inside a `test` stage.
#[derive(Debug, Clone)]
pub struct AssertionResult {
    /// Human-readable description of what was asserted (e.g. `expect ok $ make test`).
    pub description: String,
    /// Whether the assertion held.
    pub passed: bool,
    /// On failure, why it failed — an expected/actual diff or a captured-output snippet.
    pub detail: Option<String>,
}

/// Accumulates assertion outcomes for a `test` stage so the runner can report a pass/fail
/// tally instead of collapsing the stage to a single exit code.
///
/// When an [`EvalContext`] carries a `TestRecorder`, the `expect` / `assert` steps record
/// their outcome here and *continue* rather than aborting at the first failed assertion,
/// so every assertion in the stage runs and is reported. Behind a mutex so the recorder is
/// `Sync` and shareable across the per-iteration context clones of a `for` loop.
#[derive(Debug, Default)]
pub struct TestRecorder {
    results: Mutex<Vec<AssertionResult>>,
}

impl TestRecorder {
    /// Append one assertion's outcome.
    pub fn record(&self, result: AssertionResult) {
        self.results.lock().unwrap().push(result);
    }

    /// A snapshot of every recorded assertion, in execution order.
    pub fn results(&self) -> Vec<AssertionResult> {
        self.results.lock().unwrap().clone()
    }

    /// Number of assertions that passed.
    pub fn passed(&self) -> usize {
        self.results.lock().unwrap().iter().filter(|r| r.passed).count()
    }

    /// Number of assertions that failed.
    pub fn failed(&self) -> usize {
        self.results.lock().unwrap().iter().filter(|r| !r.passed).count()
    }
}

// ── Evaluation context ────────────────────────────────────────────────────────

/// Runtime context threaded through expression evaluation.
///
/// Build the initial context for a script with [`eval_program`], then extend it with
/// for-loop variable bindings (via [`EvalContext::with_for_var`]) inside step execution
/// (Phase 5).
#[derive(Debug)]
pub struct EvalContext {
    /// Directory containing the `.ms` file; `glob` patterns are resolved relative to it.
    pub script_dir: PathBuf,
    /// Host platform string: `"windows"`, `"linux"`, or `"macos"`.
    pub platform: String,
    /// Evaluated `let` bindings in declaration order.
    pub let_values: Vec<(String, Value)>,
    /// Evaluated `project` block fields.
    pub project_fields: HashMap<String, Value>,
    /// Active `for`-loop variable bindings set by the step executor (Phase 5).
    pub for_vars: HashMap<String, FileEntry>,
    /// Matrix dimension bindings for the currently executing stage (Phase 37). A stage
    /// generated from a `matrix { dim: [...] }` block resolves each `dim` to its value
    /// as a built-in string variable, alongside `platform`. Empty outside a matrix stage.
    pub matrix_vars: HashMap<String, String>,
    /// Maps each import alias to the raw module name from the `import` declaration.
    /// e.g. `import "git" as vcs` → `"vcs" → "git"`.
    pub import_aliases: HashMap<String, String>,
    /// Input file paths whose `for`-loop iterations the executor should skip because
    /// their content is unchanged since the last successful run and the stage's declared
    /// outputs are all present (Phase 38 per-file incremental change detection). Empty
    /// except inside a stage the runner is rebuilding incrementally.
    pub skip_inputs: std::collections::HashSet<PathBuf>,
    /// Resolved `inputs` fileset for the currently executing stage (set by Phase 6 runner).
    pub stage_inputs: Option<Value>,
    /// Resolved `outputs` value for the currently executing stage (set by Phase 6 runner).
    pub stage_outputs: Option<Value>,
    /// All stage names declared in the program — lets bare stage-name identifiers resolve
    /// to their string value (e.g. in `pipeline { stages: [compile, test] }`).
    pub stage_names: HashSet<String>,
    /// Resolved `outputs` values of stages that have already completed, keyed by stage
    /// name. Lets `<stage>.outputs` references evaluate at runtime once the producing
    /// stage has run; populated by the Phase 6 runner in dependency order.
    pub stage_output_refs: HashMap<String, Value>,
    /// The module registry used to resolve and dispatch `import`ed module calls.
    /// `Arc`-backed, so cloning this context per stage / loop iteration is cheap.
    pub registry: ModuleRegistry,
    /// Optional per-stage output buffer. When `Some`, side-effecting steps capture
    /// their output here instead of streaming to the terminal, so the Phase 24
    /// parallel runner can flush each stage's output atomically. `None` (the default
    /// and the sequential path) streams output live.
    pub output: Option<Arc<OutputSink>>,
    /// Optional assertion tally for a `test` stage (Phase 39). When `Some`, the `expect`
    /// and `assert` steps record their outcome here and continue instead of failing the
    /// stage at the first failed assertion. `None` (an ordinary stage) makes a failed
    /// assertion fail the step like any other.
    pub tests: Option<Arc<TestRecorder>>,
    /// Active working-directory override for step execution (Phase 42). When `Some`, `$`
    /// exec commands run in this directory and relative file-step paths resolve against
    /// it; when `None`, both fall back to `script_dir`. Set by a `workdir { … }` block.
    pub cwd_override: Option<PathBuf>,
    /// Environment variables overlaid onto spawned commands (`$` exec / `expect`) by an
    /// enclosing `with_env { … }` block (Phase 42). Empty outside such a block; nested
    /// blocks merge with inner keys winning.
    pub env_overlay: HashMap<String, String>,
    /// Optional handle to the run's reporter (Phase 43). When `Some`, a `log` step renders
    /// its message through `Reporter::step_log` and writes it to the per-stage output sink
    /// (or the terminal in the sequential path). `None` (library use) makes `log` a no-op.
    pub reporter: Option<ReporterHandle>,
}

impl EvalContext {
    fn lookup_let(&self, name: &str) -> Option<&Value> {
        self.let_values.iter().rev().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// Return a clone of this context with `var` bound to `entry` for `for`-loop iteration.
    pub fn with_for_var(&self, var: String, entry: FileEntry) -> Self {
        let mut child = self.clone_base();
        child.for_vars = self.for_vars.clone();
        child.for_vars.insert(var, entry);
        child
    }

    /// Return a stage-execution context: fresh `for_vars`, `stage_inputs`, and `stage_outputs` set.
    pub fn with_stage(&self, inputs: Option<Value>, outputs: Option<Value>) -> Self {
        let mut child = self.clone_base();
        child.stage_inputs = inputs;
        child.stage_outputs = outputs;
        child
    }

    /// Return a context whose `<stage>.outputs` references resolve against `registry`
    /// (the resolved outputs of already-completed stages, keyed by stage name).
    pub fn with_stage_outputs(&self, registry: HashMap<String, Value>) -> Self {
        let mut child = self.clone_base();
        child.stage_output_refs = registry;
        child
    }

    /// Return a context that captures step output into `sink` instead of streaming it
    /// to the terminal. Used by the parallel runner to buffer each stage's output.
    pub fn with_output(&self, sink: Arc<OutputSink>) -> Self {
        let mut child = self.clone_base();
        child.stage_inputs = self.stage_inputs.clone();
        child.stage_outputs = self.stage_outputs.clone();
        child.output = Some(sink);
        child
    }

    /// Return a context with `bindings` installed as matrix variables, so a generated
    /// matrix-variant stage resolves each dimension name to its value. Preserves the
    /// stage inputs/outputs already set on `self`.
    pub fn with_matrix_vars(&self, bindings: &[crate::ast::MatrixBinding]) -> Self {
        let mut child = self.clone_base();
        child.stage_inputs = self.stage_inputs.clone();
        child.stage_outputs = self.stage_outputs.clone();
        child.matrix_vars = bindings.iter().map(|b| (b.name.clone(), b.value.clone())).collect();
        child
    }

    /// The effective working directory for step execution: the active `workdir` override
    /// if one is in scope, otherwise the script directory.
    pub fn effective_cwd(&self) -> &Path {
        self.cwd_override.as_deref().unwrap_or(&self.script_dir)
    }

    /// Return a child context whose steps run with `dir` as the working directory
    /// (Phase 42 `workdir`). Preserves the stage and loop bindings already in scope.
    pub fn with_workdir(&self, dir: PathBuf) -> Self {
        let mut child = self.clone_base();
        child.stage_inputs = self.stage_inputs.clone();
        child.stage_outputs = self.stage_outputs.clone();
        child.for_vars = self.for_vars.clone();
        child.cwd_override = Some(dir);
        child
    }

    /// Return a child context whose spawned commands carry `overlay` as their environment
    /// overlay (Phase 42 `with_env`). The caller merges any outer overlay before passing
    /// it in. Preserves the stage and loop bindings already in scope.
    pub fn with_env_overlay(&self, overlay: HashMap<String, String>) -> Self {
        let mut child = self.clone_base();
        child.stage_inputs = self.stage_inputs.clone();
        child.stage_outputs = self.stage_outputs.clone();
        child.for_vars = self.for_vars.clone();
        child.env_overlay = overlay;
        child
    }

    /// Return a child context with the block-scoped binding `name = value` added (Phase 44).
    /// The binding joins the `let` environment so subsequent steps in the block resolve it;
    /// because the child is a fresh clone, the binding falls out of scope when the block ends
    /// (and is re-evaluated per iteration inside a `for` loop). Preserves the stage and loop
    /// bindings already in scope.
    pub fn with_local_let(&self, name: String, value: Value) -> Self {
        let mut child = self.clone_base();
        child.stage_inputs = self.stage_inputs.clone();
        child.stage_outputs = self.stage_outputs.clone();
        child.for_vars = self.for_vars.clone();
        child.let_values.push((name, value));
        child
    }

    /// Return a context where `failed_stage` resolves to `stage_name` (for pipeline on_failure).
    pub fn with_failed_stage(&self, stage_name: String) -> Self {
        let mut child = self.clone_base();
        child.stage_inputs = self.stage_inputs.clone();
        child.stage_outputs = self.stage_outputs.clone();
        child.let_values.push(("failed_stage".to_string(), Value::String(stage_name)));
        child
    }

    fn clone_base(&self) -> Self {
        EvalContext {
            script_dir: self.script_dir.clone(),
            platform: self.platform.clone(),
            let_values: self.let_values.clone(),
            project_fields: self.project_fields.clone(),
            for_vars: HashMap::new(),
            // Matrix bindings belong to a stage, so they carry across the per-iteration
            // and per-stage context clones (like `platform`), unlike `for_vars`.
            matrix_vars: self.matrix_vars.clone(),
            // The incremental skip set is stage-scoped; preserve it so the `for`-loop's
            // per-iteration context clones still see which inputs to skip.
            skip_inputs: self.skip_inputs.clone(),
            import_aliases: self.import_aliases.clone(),
            stage_inputs: None,
            stage_outputs: None,
            stage_names: self.stage_names.clone(),
            stage_output_refs: self.stage_output_refs.clone(),
            registry: self.registry.clone(),
            output: self.output.clone(),
            // Step-execution context (Phase 42) is block-scoped but must survive the
            // per-iteration clones of a `for` loop nested inside a `workdir` / `with_env`.
            cwd_override: self.cwd_override.clone(),
            env_overlay: self.env_overlay.clone(),
            // The test recorder is stage-scoped; preserve it so a `for` loop's per-iteration
            // context clones still tally their `expect` / `assert` outcomes.
            tests: self.tests.clone(),
            // The reporter handle is run-scoped: carry it across every context clone so a
            // `log` step deep inside a stage still routes through the run's reporter.
            reporter: self.reporter.clone(),
        }
    }

    /// Return a copy of this context with `reporter` installed, so its `log` steps route
    /// through the run's reporter. Used by the front end to attach the reporter to the base
    /// context before a run; the handle then propagates to every stage / loop context clone.
    pub fn with_reporter(&self, reporter: ReporterHandle) -> Self {
        let mut child = self.clone_base();
        child.stage_inputs = self.stage_inputs.clone();
        child.stage_outputs = self.stage_outputs.clone();
        child.reporter = Some(reporter);
        child
    }
}

// ── Public API ─────────────────────────────────────────────────────────────────

/// Evaluate all `let` bindings and the `project` block in `program`, producing an
/// [`EvalContext`] for subsequent stage and step evaluation.
///
/// `import`, `stage`, and `pipeline` items are skipped — they belong to later phases.
/// Errors are accumulated; the first fatal non-eval error short-circuits immediately.
///
/// Uses the standard-library [`ModuleRegistry`]; see [`eval_program_with`] to supply
/// a custom registry (e.g. one extended with plugins).
pub fn eval_program(program: &Program, script_dir: &Path) -> Result<EvalContext> {
    eval_program_with(program, script_dir, ModuleRegistry::standard())
}

/// Like [`eval_program`], but uses the provided module `registry` to resolve and
/// dispatch module calls. Construct the registry once and share it with
/// [`analyze_with`](crate::sema::analyze_with) so analysis and evaluation agree on
/// the set of available modules.
pub fn eval_program_with(
    program: &Program,
    script_dir: &Path,
    registry: ModuleRegistry,
) -> Result<EvalContext> {
    // Collect all stage names in one pass before evaluation so bare stage-name
    // identifiers (e.g. in `stages: [compile, test]`) resolve to their string value.
    let stage_names: HashSet<String> = program
        .items
        .iter()
        .filter_map(|item| if let Item::Stage(s) = item { Some(s.name.clone()) } else { None })
        .collect();

    let mut ctx = EvalContext {
        script_dir: script_dir.to_path_buf(),
        platform: host_platform().to_string(),
        let_values: Vec::new(),
        project_fields: HashMap::new(),
        for_vars: HashMap::new(),
        matrix_vars: HashMap::new(),
        skip_inputs: HashSet::new(),
        import_aliases: HashMap::new(),
        stage_inputs: None,
        stage_outputs: None,
        stage_names,
        stage_output_refs: HashMap::new(),
        registry,
        output: None,
        tests: None,
        cwd_override: None,
        env_overlay: HashMap::new(),
        reporter: None,
    };
    let mut errors: Vec<Diagnostic> = Vec::new();

    for item in &program.items {
        match item {
            Item::Import(d) => {
                ctx.import_aliases.insert(d.alias.clone(), d.module.clone());
            }
            Item::Let(d) => match eval_expr(&d.value, &ctx) {
                Ok(v) => ctx.let_values.push((d.name.clone(), v)),
                Err(Error::Eval(diags)) => errors.extend(diags),
                Err(e) => return Err(e),
            },
            Item::Project(b) => {
                for field in &b.fields {
                    match eval_expr(&field.value, &ctx) {
                        Ok(v) => {
                            ctx.project_fields.insert(field.name.clone(), v);
                        }
                        Err(Error::Eval(diags)) => errors.extend(diags),
                        Err(e) => return Err(e),
                    }
                }
            }
            _ => {}
        }
    }

    if errors.is_empty() { Ok(ctx) } else { Err(Error::Eval(errors)) }
}

/// Evaluate a single expression within `ctx`.
pub fn eval_expr(expr: &Expr, ctx: &EvalContext) -> Result<Value> {
    Evaluator { ctx }.eval(expr)
}

/// Evaluate a boolean condition within `ctx`.
pub fn eval_condition(condition: &Condition, ctx: &EvalContext) -> Result<bool> {
    Evaluator { ctx }.eval_condition(condition)
}

// ── General comparison conditions (Phase 41) ───────────────────────────────────

/// Apply a comparison operator to two evaluated values. Operand type compatibility for
/// `==` / `!=` is checked statically in `sema`; at runtime, mismatched types simply
/// compare unequal rather than erroring.
fn eval_compare(op: &CondOp, lhs: &Value, rhs: &Value) -> bool {
    match op {
        CondOp::Eq => values_equal(lhs, rhs),
        CondOp::Ne => !values_equal(lhs, rhs),
        CondOp::Contains => value_contains(lhs, rhs),
        // `a in b` is the mirror of `b contains a`.
        CondOp::In => value_contains(rhs, lhs),
    }
}

/// Structural equality between two runtime values. Different value kinds are never equal.
fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::List(x), Value::List(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| values_equal(p, q))
        }
        (Value::FileSet(x), Value::FileSet(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| p.path == q.path)
        }
        _ => false,
    }
}

/// Whether `haystack` contains `needle`: substring for strings, membership for lists
/// and filesets.
fn value_contains(haystack: &Value, needle: &Value) -> bool {
    match haystack {
        Value::String(s) => s.contains(needle.display_string().as_str()),
        Value::List(items) => items.iter().any(|item| values_equal(item, needle)),
        Value::FileSet(entries) => {
            let n = needle.display_string();
            entries.iter().any(|e| e.path.to_string_lossy() == n)
        }
        // Scalars have no notion of containment.
        Value::Int(_) | Value::Bool(_) => false,
    }
}

/// Emptiness is defined for strings, lists, and filesets; scalars are never empty.
fn value_is_empty(v: &Value) -> bool {
    match v {
        Value::String(s) => s.is_empty(),
        Value::List(items) => items.is_empty(),
        Value::FileSet(entries) => entries.is_empty(),
        Value::Int(_) | Value::Bool(_) => false,
    }
}

// ── Evaluator ─────────────────────────────────────────────────────────────────

struct Evaluator<'a> {
    ctx: &'a EvalContext,
}

impl<'a> Evaluator<'a> {
    fn eval_err(&self, msg: impl Into<String>, span: &Span) -> Error {
        Error::Eval(vec![Diagnostic::new(msg).with_span(span.clone())])
    }

    fn eval(&self, expr: &Expr) -> Result<Value> {
        match expr {
            Expr::String(s) => self.eval_string(s),
            Expr::Int(i) => Ok(Value::Int(i.value)),
            Expr::Bool(b) => Ok(Value::Bool(b.value)),
            Expr::List(list) => self.eval_list(list),
            Expr::Glob(g) => self.eval_glob(g),
            Expr::If(if_expr) => self.eval_if(if_expr),
            Expr::ModuleCall(c) => {
                let module_name = self
                    .ctx
                    .import_aliases
                    .get(&c.module)
                    .ok_or_else(|| {
                        self.eval_err(
                            format!(
                                "undeclared module alias '{}' (should have been caught in semantic analysis)",
                                c.module
                            ),
                            &c.span,
                        )
                    })?
                    .clone();
                let resolved: Vec<modules::ResolvedArg> = c
                    .args
                    .iter()
                    .map(|a| {
                        Ok(modules::ResolvedArg {
                            name: a.name.clone(),
                            value: self.eval(&a.value)?,
                        })
                    })
                    .collect::<Result<_>>()?;
                let cx = modules::ModuleCx {
                    span: &c.span,
                    script_dir: &self.ctx.script_dir,
                    permissions: self.ctx.registry.permissions(),
                };
                self.ctx.registry.dispatch(&module_name, &c.method, &resolved, &cx)
            }
            Expr::StageRef(r) => {
                self.ctx.stage_output_refs.get(&r.stage).cloned().ok_or_else(|| {
                    self.eval_err(
                        format!("'{}' outputs are not available until the stage has run", r.stage),
                        &r.span,
                    )
                })
            }
            Expr::MemberAccess(m) => self.eval_member_access(m),
            Expr::Ident(ident) => self.eval_ident(ident),
        }
    }

    fn eval_string(&self, s: &StringExpr) -> Result<Value> {
        let mut buf = String::new();
        for part in &s.parts {
            match part {
                StringPart::Literal(text) => buf.push_str(text),
                StringPart::Interpolation(expr) => {
                    let val = self.eval(expr)?;
                    buf.push_str(&val.display_string());
                }
            }
        }
        Ok(Value::String(buf))
    }

    fn eval_list(&self, list: &ListExpr) -> Result<Value> {
        let mut items = Vec::with_capacity(list.items.len());
        for item in &list.items {
            items.push(self.eval(item)?);
        }
        Ok(Value::List(items))
    }

    fn eval_glob(&self, g: &GlobExpr) -> Result<Value> {
        let mut entries: Vec<FileEntry> = Vec::new();
        for pattern in &g.patterns {
            let full = self.ctx.script_dir.join(pattern);
            // glob crate expects forward slashes on all platforms
            let pattern_str = full.to_string_lossy().replace('\\', "/");
            let iter = glob::glob(&pattern_str).map_err(|e| {
                self.eval_err(format!("invalid glob pattern '{}': {}", pattern, e), &g.span)
            })?;
            for result in iter {
                match result {
                    Ok(path) => entries.push(FileEntry::from_path(path)),
                    Err(e) => {
                        return Err(self.eval_err(
                            format!("error reading '{}': {}", e.path().display(), e.error()),
                            &g.span,
                        ));
                    }
                }
            }
        }
        Ok(Value::FileSet(entries))
    }

    fn eval_if(&self, if_expr: &IfExpr) -> Result<Value> {
        if self.eval_condition(&if_expr.condition)? {
            self.eval(&if_expr.then_expr)
        } else {
            self.eval(&if_expr.else_expr)
        }
    }

    fn eval_condition(&self, cond: &Condition) -> Result<bool> {
        match cond {
            Condition::Env(c) => {
                let val = std::env::var(&c.var).unwrap_or_default();
                Ok(match &c.comparison {
                    None => !val.is_empty(),
                    Some((CompareOp::Eq, expected)) => &val == expected,
                    Some((CompareOp::Ne, expected)) => &val != expected,
                })
            }
            Condition::Platform(c) => {
                let rhs = match c.value {
                    Platform::Windows => "windows",
                    Platform::Linux => "linux",
                    Platform::MacOs => "macos",
                };
                Ok(match c.op {
                    CompareOp::Eq => self.ctx.platform == rhs,
                    CompareOp::Ne => self.ctx.platform != rhs,
                })
            }
            Condition::Compare(c) => {
                let lhs = self.eval(&c.lhs)?;
                let rhs = self.eval(&c.rhs)?;
                Ok(eval_compare(&c.op, &lhs, &rhs))
            }
            Condition::Empty(c) => Ok(value_is_empty(&self.eval(&c.expr)?)),
            Condition::Not(inner, _) => Ok(!self.eval_condition(inner)?),
            Condition::And(a, b, _) => Ok(self.eval_condition(a)? && self.eval_condition(b)?),
            Condition::Or(a, b, _) => Ok(self.eval_condition(a)? || self.eval_condition(b)?),
        }
    }

    fn eval_member_access(&self, m: &MemberAccessExpr) -> Result<Value> {
        if m.object == "project" {
            return self.ctx.project_fields.get(&m.field).cloned().ok_or_else(|| {
                self.eval_err(format!("unknown project field '{}'", m.field), &m.span)
            });
        }
        if let Some(entry) = self.ctx.for_vars.get(&m.object) {
            return entry.get_field(&m.field).map(Value::String).ok_or_else(|| {
                self.eval_err(format!("unknown file property '{}'", m.field), &m.span)
            });
        }
        Err(self.eval_err(
            format!(
                "'{}' is not a valid member-access target here; \
                 `project` fields and `for`-loop variables are the only valid objects",
                m.object
            ),
            &m.span,
        ))
    }

    fn eval_ident(&self, ident: &IdentExpr) -> Result<Value> {
        if ident.name == "platform" {
            return Ok(Value::String(self.ctx.platform.clone()));
        }
        if ident.name == "inputs" {
            return self.ctx.stage_inputs.clone().ok_or_else(|| {
                self.eval_err("'inputs' is only available inside a stage's step block", &ident.span)
            });
        }
        if ident.name == "outputs" {
            return self.ctx.stage_outputs.clone().ok_or_else(|| {
                self.eval_err(
                    "'outputs' is only available inside a stage's step block",
                    &ident.span,
                )
            });
        }
        if ident.name == "failed_stage" {
            return self.ctx.lookup_let("failed_stage").cloned().ok_or_else(|| {
                self.eval_err(
                    "'failed_stage' is only available inside a pipeline on_failure block",
                    &ident.span,
                )
            });
        }
        // Matrix dimension bindings of the executing stage resolve as built-in strings.
        // They take precedence over `let` bindings so a dimension shadows an outer name
        // only within its own stage.
        if let Some(val) = self.ctx.matrix_vars.get(&ident.name) {
            return Ok(Value::String(val.clone()));
        }
        // User let-bindings (take precedence over stage names to respect shadowing)
        if let Some(val) = self.ctx.lookup_let(&ident.name) {
            return Ok(val.clone());
        }
        // Bare stage-name identifiers — used in `stages: [compile, test]` lists
        if self.ctx.stage_names.contains(&ident.name) {
            return Ok(Value::String(ident.name.clone()));
        }
        Err(self.eval_err(format!("undefined name '{}'", ident.name), &ident.span))
    }
}

// ── Platform detection ────────────────────────────────────────────────────────

fn host_platform() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "unknown"
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Span;
    use std::collections::HashSet;
    use std::path::PathBuf;

    fn dummy_span() -> Span {
        Span {
            file: PathBuf::from("test.ms"),
            line_start: 1,
            col_start: 1,
            line_end: 1,
            col_end: 1,
        }
    }

    fn empty_ctx() -> EvalContext {
        EvalContext {
            script_dir: PathBuf::from("."),
            platform: "windows".to_string(),
            let_values: Vec::new(),
            project_fields: HashMap::new(),
            for_vars: HashMap::new(),
            matrix_vars: HashMap::new(),
            skip_inputs: HashSet::new(),
            import_aliases: HashMap::new(),
            stage_inputs: None,
            stage_outputs: None,
            stage_names: HashSet::new(),
            stage_output_refs: HashMap::new(),
            registry: ModuleRegistry::standard(),
            output: None,
            tests: None,
            cwd_override: None,
            env_overlay: HashMap::new(),
            reporter: None,
        }
    }

    #[test]
    fn string_literal() {
        let expr = Expr::String(StringExpr {
            parts: vec![StringPart::Literal("hello".to_string())],
            span: dummy_span(),
        });
        let val = eval_expr(&expr, &empty_ctx()).unwrap();
        assert!(matches!(val, Value::String(s) if s == "hello"));
    }

    #[test]
    fn bool_literal() {
        let expr = Expr::Bool(BoolExpr { value: true, span: dummy_span() });
        let val = eval_expr(&expr, &empty_ctx()).unwrap();
        assert!(matches!(val, Value::Bool(true)));
    }

    #[test]
    fn string_interpolation() {
        let ctx = empty_ctx();
        let inner = Expr::Bool(BoolExpr { value: false, span: dummy_span() });
        let expr = Expr::String(StringExpr {
            parts: vec![
                StringPart::Literal("flag=".to_string()),
                StringPart::Interpolation(Box::new(inner)),
            ],
            span: dummy_span(),
        });
        let val = eval_expr(&expr, &ctx).unwrap();
        assert!(matches!(val, Value::String(s) if s == "flag=false"));
    }

    #[test]
    fn platform_ident() {
        let expr = Expr::Ident(IdentExpr { name: "platform".to_string(), span: dummy_span() });
        let val = eval_expr(&expr, &empty_ctx()).unwrap();
        assert!(matches!(val, Value::String(s) if s == "windows"));
    }

    #[test]
    fn if_else_platform_condition() {
        let cond = Condition::Platform(PlatformCondition {
            op: CompareOp::Eq,
            value: Platform::Windows,
            span: dummy_span(),
        });
        let expr = Expr::If(Box::new(IfExpr {
            condition: cond,
            then_expr: Expr::String(StringExpr {
                parts: vec![StringPart::Literal("win".to_string())],
                span: dummy_span(),
            }),
            else_expr: Expr::String(StringExpr {
                parts: vec![StringPart::Literal("unix".to_string())],
                span: dummy_span(),
            }),
            span: dummy_span(),
        }));
        let val = eval_expr(&expr, &empty_ctx()).unwrap();
        assert!(matches!(val, Value::String(s) if s == "win"));
    }

    #[test]
    fn list_literal() {
        let expr = Expr::List(ListExpr {
            items: vec![
                Expr::String(StringExpr {
                    parts: vec![StringPart::Literal("a".to_string())],
                    span: dummy_span(),
                }),
                Expr::String(StringExpr {
                    parts: vec![StringPart::Literal("b".to_string())],
                    span: dummy_span(),
                }),
            ],
            span: dummy_span(),
        });
        let val = eval_expr(&expr, &empty_ctx()).unwrap();
        assert!(matches!(val, Value::List(items) if items.len() == 2));
    }

    #[test]
    fn project_field_access() {
        let mut ctx = empty_ctx();
        ctx.project_fields.insert("name".to_string(), Value::String("myapp".to_string()));
        let expr = Expr::MemberAccess(MemberAccessExpr {
            object: "project".to_string(),
            field: "name".to_string(),
            span: dummy_span(),
        });
        let val = eval_expr(&expr, &ctx).unwrap();
        assert!(matches!(val, Value::String(s) if s == "myapp"));
    }

    #[test]
    fn stage_ref_resolves_from_registry() {
        // Once a producing stage has run, `<stage>.outputs` resolves to its outputs.
        let mut ctx = empty_ctx();
        ctx.stage_output_refs
            .insert("compile".to_string(), Value::List(vec![Value::String("bin/app".to_string())]));
        let expr =
            Expr::StageRef(StageRefExpr { stage: "compile".to_string(), span: dummy_span() });
        let val = eval_expr(&expr, &ctx).unwrap();
        assert!(matches!(val, Value::List(items) if items.len() == 1));
    }

    #[test]
    fn stage_ref_unresolved_errors() {
        // Before the producing stage runs, the reference is an error.
        let expr = Expr::StageRef(StageRefExpr { stage: "ghost".to_string(), span: dummy_span() });
        assert!(eval_expr(&expr, &empty_ctx()).is_err());
    }

    #[test]
    fn for_var_field_access() {
        let entry = FileEntry::from_path(PathBuf::from("/src/main.rs"));
        let mut ctx = empty_ctx();
        ctx.for_vars.insert("f".to_string(), entry);
        let expr = Expr::MemberAccess(MemberAccessExpr {
            object: "f".to_string(),
            field: "name".to_string(),
            span: dummy_span(),
        });
        let val = eval_expr(&expr, &ctx).unwrap();
        assert!(matches!(val, Value::String(s) if s == "main.rs"));
    }
}
