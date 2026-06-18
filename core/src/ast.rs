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
    pub steps: Vec<Step>,
    /// Steps executed when the main `steps` block fails, before failure propagates.
    pub on_failure: Vec<Step>,
    pub span: Span,
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
