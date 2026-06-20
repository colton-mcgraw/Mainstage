use crate::error::Span;

// ── Program ───────────────────────────────────────────────────────────────────

/// The root node of the AST — the complete parsed contents of a single `.ms` file.
#[derive(Debug, Clone)]
pub struct Program {
    pub items: Vec<Item>,
    pub span: Span,
}

// ── Top-level items ───────────────────────────────────────────────────────────

/// A single top-level declaration or block within a `Program`.
#[derive(Debug, Clone)]
pub enum Item {
    Import(ImportDecl),
    Let(LetDecl),
    Project(ProjectBlock),
    Stage(StageBlock),
    Pipeline(PipelineBlock),
}

impl Item {
    /// Returns the source span of this item, regardless of its variant.
    pub fn span(&self) -> &Span {
        match self {
            Item::Import(d) => &d.span,
            Item::Let(d) => &d.span,
            Item::Project(b) => &b.span,
            Item::Stage(b) => &b.span,
            Item::Pipeline(b) => &b.span,
        }
    }
}

/// An `import "<module>" as <alias>;` declaration that binds a built-in module to a local name.
#[derive(Debug, Clone)]
pub struct ImportDecl {
    /// The module name string literal, e.g. `"git"` or `"env"`.
    pub module: String,
    /// The identifier to which the module is bound in scope.
    pub alias: String,
    pub span: Span,
}

/// A `let <name> = <expr>;` top-level binding.
#[derive(Debug, Clone)]
pub struct LetDecl {
    pub name: String,
    pub value: Expr,
    pub span: Span,
}

// ── Project ───────────────────────────────────────────────────────────────────

/// The `project { … }` block — holds named scalar fields describing the project.
#[derive(Debug, Clone)]
pub struct ProjectBlock {
    pub fields: Vec<ProjectField>,
    pub span: Span,
}

/// A single `<name>: <expr>` field inside a `project` block.
#[derive(Debug, Clone)]
pub struct ProjectField {
    pub name: String,
    pub value: Expr,
    pub span: Span,
}

// ── Stage ─────────────────────────────────────────────────────────────────────

/// A `stage <name> { … }` block — declares a single build step with optional
/// inputs, outputs, failure handling, and a sequence of steps to execute.
#[derive(Debug, Clone)]
pub struct StageBlock {
    pub name: String,
    /// Optional human-readable summary of what the stage does, from `description: "…"`.
    /// Surfaced by `mainstage list --describe` and in the editor (LSP symbols / hover);
    /// it has no effect on execution. A static string — interpolation is not allowed.
    pub description: Option<String>,
    /// File set expression describing the stage's input dependencies; drives change detection.
    pub inputs: Option<Expr>,
    /// Expression describing the paths this stage is expected to produce.
    pub outputs: Option<Expr>,
    /// Explicit ordering edges to other stages, declared via `depends_on: [a, b]`.
    /// These add dependency edges the inferred `inputs`/`outputs` graph cannot express
    /// (side-effecting setup, "run after" relationships with no shared file artifact).
    pub depends_on: Vec<StageDep>,
    /// When `true`, a non-zero exit from this stage does not cancel downstream stages.
    pub allow_failure: bool,
    /// When `true`, the stage runs on every invocation, bypassing change detection —
    /// the explicit form of an action stage (e.g. `run`) that must never be cached.
    pub always_run: bool,
    /// When `true`, the stage's success is recorded in the cache even with no file
    /// `outputs`, so a side-effecting setup stage (e.g. `initialize`) runs once and is
    /// skipped thereafter until its inputs change or the cache is cleared.
    pub run_once: bool,
    /// When `true`, the stage is a *test* stage: it is never cached (like `always_run`),
    /// and its `expect` / `assert` steps are tallied into a pass/fail count rather than
    /// collapsing to a single exit code. The stage fails when any assertion fails.
    pub test: bool,
    /// Declared build-matrix dimensions, from `matrix { <dim>: [<values>] }`. Empty for
    /// an ordinary stage. Present only on the *authored* stage: the matrix lowering pass
    /// ([`crate::matrix::expand`]) replaces a matrixed stage with one concrete variant per
    /// combination of values, each carrying its bindings in `matrix_bindings` and an empty
    /// `matrix`, so semantic analysis, change detection, and the scheduler only ever see
    /// ordinary stages.
    pub matrix: Vec<MatrixDim>,
    /// Resolved `dimension → value` bindings for a generated matrix variant (e.g.
    /// `[("arch", "x64")]`). Empty for authored and non-matrix stages. Each binding is
    /// exposed as a built-in string variable inside the stage, alongside `platform`.
    pub matrix_bindings: Vec<MatrixBinding>,
    pub steps: Vec<Step>,
    /// Steps executed when the main `steps` block fails, before failure propagates.
    pub on_failure: Vec<Step>,
    pub span: Span,
}

/// One dimension of a stage's build matrix: a dimension name and the values it ranges
/// over (e.g. `arch: ["x64", "arm64"]`). The cartesian product of every dimension's
/// values determines how many concrete stages the authored stage expands into.
#[derive(Debug, Clone)]
pub struct MatrixDim {
    pub name: String,
    pub values: Vec<MatrixValue>,
    pub span: Span,
}

/// A single value within a matrix dimension — a static string literal — and its span,
/// so duplicate-value and empty-dimension diagnostics can point at the offending entry.
#[derive(Debug, Clone)]
pub struct MatrixValue {
    pub value: String,
    pub span: Span,
}

/// A resolved `dimension = value` binding carried by a generated matrix variant. The
/// lowering pass produces one binding per dimension for each variant; the runtime
/// exposes them as built-in string variables while the variant's steps execute.
#[derive(Debug, Clone)]
pub struct MatrixBinding {
    pub name: String,
    pub value: String,
}

/// A single entry in a stage's `depends_on` list: the referenced stage name plus the
/// source span of that reference, so unknown-stage and cycle diagnostics can point at it.
#[derive(Debug, Clone)]
pub struct StageDep {
    pub name: String,
    pub span: Span,
}

// ── Pipeline ──────────────────────────────────────────────────────────────────

/// A `[default] pipeline <name> { … }` block — orders stages and wires lifecycle hooks.
#[derive(Debug, Clone)]
pub struct PipelineBlock {
    /// `true` if the `default` keyword preceded `pipeline`.
    pub is_default: bool,
    pub name: String,
    /// Optional expression that defines the pipeline's input file set.
    pub input: Option<Expr>,
    /// Expression that resolves to the ordered list of stage names.
    pub stages: Option<Expr>,
    /// Steps executed when any stage in this pipeline fails.
    pub on_failure: Vec<Step>,
    /// Steps executed when all stages in this pipeline succeed.
    pub on_success: Vec<Step>,
    pub span: Span,
}

// ── Expressions ───────────────────────────────────────────────────────────────

/// Any value-producing expression in a Mainstage script.
#[derive(Debug, Clone)]
pub enum Expr {
    String(StringExpr),
    Int(IntExpr),
    Bool(BoolExpr),
    List(ListExpr),
    Glob(GlobExpr),
    If(Box<IfExpr>),
    ModuleCall(ModuleCallExpr),
    /// `<stage>.outputs` — references the declared outputs of a named stage.
    StageRef(StageRefExpr),
    /// `<ident>.<ident>` — field access on a project variable or module value.
    MemberAccess(MemberAccessExpr),
    Ident(IdentExpr),
}

impl Expr {
    /// Returns the source span of this expression, regardless of its variant.
    pub fn span(&self) -> &Span {
        match self {
            Expr::String(e) => &e.span,
            Expr::Int(e) => &e.span,
            Expr::Bool(e) => &e.span,
            Expr::List(e) => &e.span,
            Expr::Glob(e) => &e.span,
            Expr::If(e) => &e.span,
            Expr::ModuleCall(e) => &e.span,
            Expr::StageRef(e) => &e.span,
            Expr::MemberAccess(e) => &e.span,
            Expr::Ident(e) => &e.span,
        }
    }
}

/// A double-quoted string literal, optionally containing `${…}` interpolations.
#[derive(Debug, Clone)]
pub struct StringExpr {
    /// Sequence of literal text segments and interpolated sub-expressions.
    pub parts: Vec<StringPart>,
    pub span: Span,
}

/// One segment of a string literal — either a run of literal characters or an interpolation.
#[derive(Debug, Clone)]
pub enum StringPart {
    Literal(String),
    /// A `${<expr>}` interpolation embedded within a string.
    Interpolation(Box<Expr>),
}

/// A signed integer literal (e.g. `42`, `-7`), parsed as an `i64`.
#[derive(Debug, Clone)]
pub struct IntExpr {
    pub value: i64,
    pub span: Span,
}

/// A `true` or `false` boolean literal.
#[derive(Debug, Clone)]
pub struct BoolExpr {
    pub value: bool,
    pub span: Span,
}

/// A `[expr, …]` list literal.
#[derive(Debug, Clone)]
pub struct ListExpr {
    pub items: Vec<Expr>,
    pub span: Span,
}

/// A `glob("pattern", …)` call — expanded to a `fileset` at evaluation time.
#[derive(Debug, Clone)]
pub struct GlobExpr {
    /// One or more glob patterns as raw (non-interpolated) string values.
    pub patterns: Vec<String>,
    pub span: Span,
}

/// An `if <condition> { <then> } else { <else> }` expression.
#[derive(Debug, Clone)]
pub struct IfExpr {
    pub condition: Condition,
    pub then_expr: Expr,
    pub else_expr: Expr,
    pub span: Span,
}

/// A `<module>.<method>(…)` call into a built-in module (e.g. `git.sha(short: true)`).
#[derive(Debug, Clone)]
pub struct ModuleCallExpr {
    /// The local alias bound by the corresponding `import` declaration.
    pub module: String,
    pub method: String,
    pub args: Vec<CallArg>,
    pub span: Span,
}

/// A single positional or named argument in a module call.
#[derive(Debug, Clone)]
pub struct CallArg {
    /// `Some(name)` for `key: value` named arguments; `None` for positional arguments.
    pub name: Option<String>,
    pub value: Expr,
    pub span: Span,
}

/// `<stage>.outputs` — a reference to the declared output set of a named stage.
#[derive(Debug, Clone)]
pub struct StageRefExpr {
    pub stage: String,
    pub span: Span,
}

/// `<object>.<field>` — member access on a known object such as `project` or `file`.
#[derive(Debug, Clone)]
pub struct MemberAccessExpr {
    pub object: String,
    pub field: String,
    pub span: Span,
}

/// A bare identifier expression that refers to a `let` binding or built-in variable.
#[derive(Debug, Clone)]
pub struct IdentExpr {
    pub name: String,
    pub span: Span,
}

// ── Conditions ────────────────────────────────────────────────────────────────

/// A boolean condition used in `if` expressions and `if` steps.
#[derive(Debug, Clone)]
pub enum Condition {
    Env(EnvCondition),
    Platform(PlatformCondition),
    /// `<expr> <op> <expr>` — compare two arbitrary expression values (Phase 41).
    Compare(CompareCondition),
    /// `empty(<expr>)` — true when the operand is an empty string, list, or fileset.
    Empty(EmptyCondition),
    /// `!<cond>` — logical negation.
    Not(Box<Condition>, Span),
    /// `<cond> and <cond>` — short-circuit logical conjunction.
    And(Box<Condition>, Box<Condition>, Span),
    /// `<cond> or <cond>` — short-circuit logical disjunction.
    Or(Box<Condition>, Box<Condition>, Span),
}

impl Condition {
    /// Returns the source span of this condition node, regardless of its variant.
    pub fn span(&self) -> &Span {
        match self {
            Condition::Env(c) => &c.span,
            Condition::Platform(c) => &c.span,
            Condition::Compare(c) => &c.span,
            Condition::Empty(c) => &c.span,
            Condition::Not(_, s) | Condition::And(_, _, s) | Condition::Or(_, _, s) => s,
        }
    }
}

/// `env("VAR")` or `env("VAR") == "value"` — tests whether an environment variable
/// is set, or compares its value to a string literal.
#[derive(Debug, Clone)]
pub struct EnvCondition {
    /// Name of the environment variable to test.
    pub var: String,
    /// If `Some`, the variable's value is compared to the string using the given operator.
    /// If `None`, the condition passes whenever the variable is set to any non-empty value.
    pub comparison: Option<(CompareOp, String)>,
    pub span: Span,
}

/// `platform == "windows"` — tests the host operating system at runtime.
#[derive(Debug, Clone)]
pub struct PlatformCondition {
    pub op: CompareOp,
    pub value: Platform,
    pub span: Span,
}

/// `<expr> == <expr>`, `<expr> != <expr>`, `<expr> contains <expr>`, or
/// `<expr> in <expr>` — compares two evaluated values (Phase 41). Unlike
/// `EnvCondition` / `PlatformCondition`, either operand may be any expression
/// (a `let`, module-call result, `project.<field>`, list, literal, …).
#[derive(Debug, Clone)]
pub struct CompareCondition {
    pub lhs: Expr,
    pub op: CondOp,
    pub rhs: Expr,
    pub span: Span,
}

/// `empty(<expr>)` — true when the operand evaluates to an empty string, an empty
/// list, or an empty fileset.
#[derive(Debug, Clone)]
pub struct EmptyCondition {
    pub expr: Expr,
    pub span: Span,
}

/// Operator in a general expression-comparison condition (Phase 41).
#[derive(Debug, Clone, PartialEq)]
pub enum CondOp {
    /// `==` — the two operands are equal.
    Eq,
    /// `!=` — the two operands are not equal.
    Ne,
    /// `contains` — the left operand (string substring, or list/fileset membership)
    /// contains the right operand.
    Contains,
    /// `in` — the left operand is contained in the right (the mirror of `contains`).
    In,
}

/// `==` or `!=` comparison operator used in conditions.
#[derive(Debug, Clone, PartialEq)]
pub enum CompareOp {
    Eq,
    Ne,
}

/// The set of platforms that can appear in a `platform` condition.
#[derive(Debug, Clone, PartialEq)]
pub enum Platform {
    Windows,
    Linux,
    MacOs,
}

// ── Steps ─────────────────────────────────────────────────────────────────────

/// An executable instruction inside a `steps {}`, `on_failure {}`, or `on_success {}` block.
#[derive(Debug, Clone)]
pub enum Step {
    /// `$ <command>` — run an external program.
    Exec(ExecStep),
    /// `copy <src> to <dest>` — copy a file or directory.
    Copy(CopyStep),
    /// `move <src> to <dest>` — move a file or directory.
    Move(MoveStep),
    /// `mkdir <path>` — create a directory tree.
    Mkdir(MkdirStep),
    /// `delete <path>` — remove a file or directory; no-op if absent.
    Delete(DeleteStep),
    /// `write <path> content: <string>` — write or overwrite a file.
    Write(WriteStep),
    /// `if <cond> { … } [else { … }]` — conditional branch.
    If(IfStep),
    /// `for <var> in <expr> { … }` — iterate over a fileset.
    For(ForStep),
    /// `try { … }` — run the inner steps, swallowing a failure so the stage continues.
    Try(TryStep),
    /// `workdir <path> { … }` — run the inner steps with the working directory set to
    /// `<path>` (for `$` exec and relative file-step paths alike).
    Workdir(WorkdirStep),
    /// `with_env { KEY: <value>, … } { … }` — run the inner steps with extra environment
    /// variables set on spawned commands.
    WithEnv(WithEnvStep),
    /// `expect <check> [timeout N] $ <command>` — run a command and assert on its exit
    /// status or captured output (test-harness step).
    Expect(ExpectStep),
    /// `assert <expr> equals|contains <string>` — compare a value to an expected one.
    Assert(AssertStep),
    /// `log "<msg>"` — print an interpolated progress message via the reporter.
    Log(LogStep),
    /// `fail "<reason>"` — fail the enclosing stage deliberately with a diagnostic.
    Fail(FailStep),
    /// `let <ident> = <expr>;` — a block-scoped binding visible to the steps that follow
    /// it within the same block (Phase 44).
    Let(LetStep),
}

impl Step {
    /// Returns the source span of this step, regardless of its variant.
    pub fn span(&self) -> &Span {
        match self {
            Step::Exec(s) => &s.span,
            Step::Copy(s) => &s.span,
            Step::Move(s) => &s.span,
            Step::Mkdir(s) => &s.span,
            Step::Delete(s) => &s.span,
            Step::Write(s) => &s.span,
            Step::If(s) => &s.span,
            Step::For(s) => &s.span,
            Step::Try(s) => &s.span,
            Step::Workdir(s) => &s.span,
            Step::WithEnv(s) => &s.span,
            Step::Expect(s) => &s.span,
            Step::Assert(s) => &s.span,
            Step::Log(s) => &s.span,
            Step::Fail(s) => &s.span,
            Step::Let(s) => &s.span,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExecStep {
    /// Raw command line after `$`, with whitespace trimmed. Interpolation is resolved at eval time.
    pub command: String,
    pub span: Span,
}

/// `copy <src> to <dest>` — cross-platform file or directory copy.
#[derive(Debug, Clone)]
pub struct CopyStep {
    pub src: Expr,
    pub dest: Expr,
    pub span: Span,
}

/// `move <src> to <dest>` — cross-platform file or directory move.
#[derive(Debug, Clone)]
pub struct MoveStep {
    pub src: Expr,
    pub dest: Expr,
    pub span: Span,
}

/// `mkdir <path>` — creates the full directory tree; succeeds if it already exists.
#[derive(Debug, Clone)]
pub struct MkdirStep {
    pub path: Expr,
    pub span: Span,
}

/// `delete <path>` — removes a file or directory recursively; no-op if the path does not exist.
#[derive(Debug, Clone)]
pub struct DeleteStep {
    pub path: Expr,
    pub span: Span,
}

/// `write <path> content: <string>` — creates or overwrites a file with the given string.
#[derive(Debug, Clone)]
pub struct WriteStep {
    pub path: Expr,
    pub content: StringExpr,
    pub span: Span,
}

/// `if <condition> { … } [else { … }]` — conditionally executes one of two step sequences.
#[derive(Debug, Clone)]
pub struct IfStep {
    pub condition: Condition,
    pub then_steps: Vec<Step>,
    /// Empty when no `else` branch is present.
    pub else_steps: Vec<Step>,
    pub span: Span,
}

/// `for <var> in <expr> { … }` — iterates over a fileset, binding each item to `<var>`.
#[derive(Debug, Clone)]
pub struct ForStep {
    /// The loop variable name, available as `<var>.*` inside the body.
    pub var: String,
    pub iterable: Expr,
    pub steps: Vec<Step>,
    pub span: Span,
}

/// `try { … }` — executes its steps in order but does not propagate a failure: if a step
/// fails, the remaining steps in the block are skipped and the stage continues as if the
/// block succeeded. The native, checkable replacement for the `$ sh -c "… || true"` idiom.
#[derive(Debug, Clone)]
pub struct TryStep {
    pub steps: Vec<Step>,
    pub span: Span,
}

/// `workdir <path> { … }` — runs its steps with the working directory set to `<path>`.
/// A relative `<path>` is resolved against the enclosing working directory (the script
/// directory at the top level, or an outer `workdir` when nested). Applies uniformly to
/// `$` exec commands and to relative paths in `copy` / `move` / `write` / `mkdir` /
/// `delete`. The native replacement for `$ sh -c "cd … && …"`.
#[derive(Debug, Clone)]
pub struct WorkdirStep {
    pub path: Expr,
    pub steps: Vec<Step>,
    pub span: Span,
}

/// `with_env { KEY: <value>, … } { … }` — runs its steps with the given environment
/// variables set on spawned commands (`$` exec and `expect`). Nested `with_env` blocks
/// merge, with the inner block overriding outer keys. The native replacement for
/// `$ sh -c "VAR=… cmd"`.
#[derive(Debug, Clone)]
pub struct WithEnvStep {
    pub vars: Vec<EnvBinding>,
    pub steps: Vec<Step>,
    pub span: Span,
}

/// A single `KEY: <value>` entry inside a `with_env` block.
#[derive(Debug, Clone)]
pub struct EnvBinding {
    pub key: String,
    pub value: Expr,
    pub span: Span,
}

/// `expect <check> [timeout <n>] $ <command>` — run a command and assert something about
/// how it ran. Inside a `test` stage a failed expectation is tallied and execution
/// continues; in an ordinary stage a failed expectation fails the step like any other.
#[derive(Debug, Clone)]
pub struct ExpectStep {
    /// What to assert about the command (exit status or captured output).
    pub check: ExpectCheck,
    /// Optional timeout in seconds. When set, the command is killed if it does not finish
    /// in time; for an `output contains` check the command is also stopped early as soon
    /// as the marker appears (so a long-running boot-smoke process need not run to the end).
    pub timeout_secs: Option<i64>,
    /// Raw command line after `$`, with trailing whitespace trimmed. Interpolation is
    /// resolved at eval time, exactly like the `$` exec step.
    pub command: String,
    pub span: Span,
}

/// The assertion an [`ExpectStep`] makes about its command.
#[derive(Debug, Clone)]
pub enum ExpectCheck {
    /// The command exits successfully (status 0).
    Ok,
    /// The command exits with a non-zero status.
    Fails,
    /// The command's combined stdout/stderr matches `expected` per `op`.
    Output { op: MatchOp, expected: StringExpr },
}

/// `assert <expr> equals|contains <string>` — compare an evaluated value against an
/// expected string. The expected value supports `${…}` interpolation.
#[derive(Debug, Clone)]
pub struct AssertStep {
    /// The value under test (rendered to a string for comparison).
    pub actual: Expr,
    pub op: MatchOp,
    pub expected: StringExpr,
    pub span: Span,
}

/// `log "<msg>"` — print a progress message. The message supports `${…}` interpolation
/// and is routed through the runner's reporter, so it honors `--quiet` and is captured in
/// the per-stage buffered output exactly like the captured output of a `$` exec step.
#[derive(Debug, Clone)]
pub struct LogStep {
    pub message: StringExpr,
    pub span: Span,
}

/// `fail "<reason>"` — fail the enclosing stage deliberately. The interpolated reason
/// becomes a user-facing `Error::Eval` diagnostic carrying the step span. It behaves like
/// any other failed step: a `try` block swallows it, and a stage's `on_failure` block fires.
#[derive(Debug, Clone)]
pub struct FailStep {
    pub reason: StringExpr,
    pub span: Span,
}

/// `let <ident> = <expr>;` — a block-scoped local binding (Phase 44). It names a derived
/// value once for the steps that follow it in the same block. Inside a `for` loop body the
/// binding is re-evaluated per iteration. Shadowing an outer binding is a semantic error.
#[derive(Debug, Clone)]
pub struct LetStep {
    pub name: String,
    pub value: Expr,
    pub span: Span,
}

/// How an `expect output` / `assert` comparison matches its expected value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchOp {
    /// The actual value contains the expected value as a substring.
    Contains,
    /// The actual value equals the expected value (compared after trimming).
    Equals,
    /// The actual value does **not** contain the expected value as a substring — the
    /// negation of [`MatchOp::Contains`], for asserting a marker's *absence*.
    NotContains,
    /// The actual value begins with the expected value as a prefix.
    StartsWith,
    /// The actual value ends with the expected value as a suffix.
    EndsWith,
    /// The actual value matches the expected value as an anchored glob pattern (`*`, `?`,
    /// `[…]` — the whole value must match, like `glob`'s path patterns). No regex dependency.
    Matches,
}
