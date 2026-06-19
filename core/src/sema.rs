use std::collections::HashMap;

use crate::{
    ast::*,
    error::{Diagnostic, Error, Result, Span},
    modules::{MethodSig, ModuleRegistry, ValueTy},
};

// ── Public API ─────────────────────────────────────────────────────────────────

/// The validated output of Phase 2 semantic analysis.
pub struct AnalysisResult {
    /// Stage dependency adjacency list: stage name → stages it directly depends on.
    /// Edges are inferred from `<stage>.outputs` references in a stage's `inputs` field.
    pub dependency_graph: HashMap<String, Vec<String>>,
}

/// Run all Phase 2 semantic checks on `program`, using the standard module registry.
///
/// Returns `Ok(AnalysisResult)` when the program is semantically valid, or
/// `Err(Error::Semantic(...))` with every diagnostic found during analysis.
pub fn analyze(program: &Program) -> Result<AnalysisResult> {
    analyze_with(program, &ModuleRegistry::standard())
}

/// Like [`analyze`], but validates against the provided module `registry`. Pass the
/// same registry used by [`eval_program_with`](crate::eval::eval_program_with) so
/// analysis and evaluation agree on the available modules.
pub fn analyze_with(program: &Program, registry: &ModuleRegistry) -> Result<AnalysisResult> {
    let mut a = Analyzer::new(registry.clone());
    let result = a.run(program);
    if a.errors.is_empty() { Ok(result) } else { Err(Error::Semantic(a.errors)) }
}

// ── Analyzer ───────────────────────────────────────────────────────────────────

struct Analyzer {
    errors: Vec<Diagnostic>,
    /// The module registry — the source of truth for which modules exist and the
    /// signatures of their methods. Used to validate `import`s and module calls.
    registry: ModuleRegistry,
    /// Matrix dimension names bound by the stage currently being resolved (Phase 37).
    /// These resolve as built-in identifiers anywhere inside the stage. Empty outside a
    /// matrix variant; the lowering pass runs before analysis, so a variant arrives here
    /// with its bindings already attached.
    matrix_vars: Vec<String>,
}

impl Analyzer {
    fn new(registry: ModuleRegistry) -> Self {
        Self { errors: Vec::new(), registry, matrix_vars: Vec::new() }
    }

    fn error(&mut self, msg: impl Into<String>, span: Span) {
        self.errors.push(Diagnostic::new(msg).with_span(span));
    }

    fn run(&mut self, program: &Program) -> AnalysisResult {
        let scope = self.collect_scope(program);
        self.resolve_program(program, &scope);
        let dependency_graph = build_dependency_graph(program);
        // Reject cycles over the combined inputs/outputs + depends_on graph here, with a
        // source span, rather than leaving it to the runtime toposort's generic error.
        for diag in detect_cycles(program, &dependency_graph) {
            self.errors.push(diag);
        }
        AnalysisResult { dependency_graph }
    }

    // ── Pass 1: collect declarations ──────────────────────────────────────────
    //
    // Gathers every top-level name into `Scope` and emits errors for duplicates
    // and multiple `default` pipelines.  The full scope must exist before name
    // resolution so that forward declarations of stages and pipelines are legal.

    fn collect_scope(&mut self, program: &Program) -> Scope {
        let mut scope = Scope::default();
        let mut default_pipeline_seen = false;

        for item in &program.items {
            match item {
                Item::Import(d) => {
                    if !self.registry.contains(&d.module) {
                        self.error(
                            format!(
                                "unknown module \"{}\"; no built-in module or discovered plugin with that name",
                                d.module
                            ),
                            d.span.clone(),
                        );
                    }
                    if scope.import_aliases.contains_key(&d.alias) {
                        self.error(
                            format!("import alias '{}' is already defined", d.alias),
                            d.span.clone(),
                        );
                    } else {
                        scope.import_aliases.insert(d.alias.clone(), d.module.clone());
                    }
                }
                Item::Let(d) => {
                    if scope.let_index(&d.name).is_some() {
                        self.error(
                            format!("let binding '{}' is already defined", d.name),
                            d.span.clone(),
                        );
                    } else {
                        scope.let_bindings.push((d.name.clone(), d.span.clone()));
                    }
                }
                Item::Project(b) => {
                    scope.has_project = true;
                    for field in &b.fields {
                        if scope.project_fields.contains_key(&field.name) {
                            self.error(
                                format!("project field '{}' is already defined", field.name),
                                field.span.clone(),
                            );
                        } else {
                            scope.project_fields.insert(field.name.clone(), field.span.clone());
                        }
                    }
                }
                Item::Stage(b) => {
                    if scope.stage_names.contains_key(&b.name) {
                        self.error(
                            format!("stage '{}' is already defined", b.name),
                            b.span.clone(),
                        );
                    } else {
                        scope.stage_names.insert(b.name.clone(), b.span.clone());
                    }
                }
                Item::Pipeline(b) => {
                    if scope.pipeline_names.contains_key(&b.name) {
                        self.error(
                            format!("pipeline '{}' is already defined", b.name),
                            b.span.clone(),
                        );
                    } else {
                        scope.pipeline_names.insert(b.name.clone(), b.span.clone());
                    }
                    if b.is_default {
                        if default_pipeline_seen {
                            self.error(
                                "at most one pipeline may be declared as `default`",
                                b.span.clone(),
                            );
                        } else {
                            default_pipeline_seen = true;
                        }
                    }
                }
            }
        }

        scope
    }

    // ── Pass 2: name resolution ────────────────────────────────────────────────
    //
    // Walks every expression and step, checking that all referenced names are
    // declared.  For `let` bindings it also enforces that expressions may only
    // reference bindings declared earlier (no forward references).

    fn resolve_program(&mut self, program: &Program, scope: &Scope) {
        let mut let_idx = 0usize;
        for item in &program.items {
            match item {
                Item::Import(_) => {}
                Item::Let(d) => {
                    let ctx = ExprCtx {
                        current_let_index: Some(let_idx),
                        for_vars: &[],
                        in_steps: false,
                    };
                    self.resolve_expr(&d.value, scope, ctx);
                    let_idx += 1;
                }
                Item::Project(b) => {
                    let ctx = ExprCtx::top_level();
                    for field in &b.fields {
                        self.resolve_expr(&field.value, scope, ctx);
                    }
                }
                Item::Stage(b) => self.resolve_stage(b, scope),
                Item::Pipeline(b) => self.resolve_pipeline(b, scope),
            }
        }
    }

    fn resolve_stage(&mut self, stage: &StageBlock, scope: &Scope) {
        // Bind this stage's matrix dimensions as valid built-in identifiers for the
        // duration of its resolution, then clear them so they don't leak to later items.
        self.matrix_vars = stage.matrix_bindings.iter().map(|b| b.name.clone()).collect();
        let ctx = ExprCtx::top_level();
        if let Some(expr) = &stage.inputs {
            self.resolve_expr(expr, scope, ctx);
        }
        if let Some(expr) = &stage.outputs {
            self.resolve_expr(expr, scope, ctx);
        }
        if stage.always_run && stage.run_once {
            self.error(
                format!(
                    "stage '{}' sets both always_run and run_once, which are contradictory",
                    stage.name
                ),
                stage.span.clone(),
            );
        }
        for dep in &stage.depends_on {
            if dep.name == stage.name {
                self.error(
                    format!("stage '{}' cannot depend on itself", stage.name),
                    dep.span.clone(),
                );
            } else if !scope.stage_names.contains_key(&dep.name) {
                self.error(format!("unknown stage '{}' in depends_on", dep.name), dep.span.clone());
            }
        }
        for step in &stage.steps {
            self.resolve_step(step, scope, &[]);
        }
        for step in &stage.on_failure {
            self.resolve_step(step, scope, &[]);
        }
        self.matrix_vars.clear();
    }

    fn resolve_pipeline(&mut self, pipeline: &PipelineBlock, scope: &Scope) {
        let ctx = ExprCtx::top_level();
        if let Some(expr) = &pipeline.input {
            self.resolve_expr(expr, scope, ctx);
        }
        if let Some(expr) = &pipeline.stages {
            self.resolve_pipeline_stages(expr, scope);
        }
        for step in &pipeline.on_failure {
            self.resolve_step(step, scope, &[]);
        }
        for step in &pipeline.on_success {
            self.resolve_step(step, scope, &[]);
        }
    }

    // The `stages:` field is commonly a list of bare stage-name identifiers.
    // Validate them as stage names (also accepting let-bindings) and generate a
    // stage-specific error message rather than the generic "undefined name" one.
    fn resolve_pipeline_stages(&mut self, expr: &Expr, scope: &Scope) {
        match expr {
            Expr::List(list) => {
                for item in &list.items {
                    if let Expr::Ident(ident) = item {
                        if !scope.stage_names.contains_key(&ident.name)
                            && scope.let_index(&ident.name).is_none()
                        {
                            self.error(
                                format!("unknown stage '{}'", ident.name),
                                ident.span.clone(),
                            );
                        }
                    } else {
                        self.resolve_expr(item, scope, ExprCtx::top_level());
                    }
                }
            }
            _ => self.resolve_expr(expr, scope, ExprCtx::top_level()),
        }
    }

    fn resolve_step(&mut self, step: &Step, scope: &Scope, for_vars: &[String]) {
        let ctx = ExprCtx { current_let_index: None, for_vars, in_steps: true };
        match step {
            Step::Exec(_) => {}
            Step::Copy(s) => {
                self.resolve_expr(&s.src, scope, ctx);
                self.resolve_expr(&s.dest, scope, ctx);
            }
            Step::Move(s) => {
                self.resolve_expr(&s.src, scope, ctx);
                self.resolve_expr(&s.dest, scope, ctx);
            }
            Step::Mkdir(s) => self.resolve_expr(&s.path, scope, ctx),
            Step::Delete(s) => self.resolve_expr(&s.path, scope, ctx),
            Step::Write(s) => {
                self.resolve_expr(&s.path, scope, ctx);
                self.resolve_string_parts(&s.content.parts, scope, ctx);
            }
            Step::If(s) => {
                for step in &s.then_steps {
                    self.resolve_step(step, scope, for_vars);
                }
                for step in &s.else_steps {
                    self.resolve_step(step, scope, for_vars);
                }
            }
            Step::For(s) => {
                self.resolve_expr(&s.iterable, scope, ctx);
                let mut inner = for_vars.to_vec();
                inner.push(s.var.clone());
                for step in &s.steps {
                    self.resolve_step(step, scope, &inner);
                }
            }
            Step::Try(s) => {
                for step in &s.steps {
                    self.resolve_step(step, scope, for_vars);
                }
            }
        }
    }

    fn resolve_expr(&mut self, expr: &Expr, scope: &Scope, ctx: ExprCtx<'_>) {
        match expr {
            Expr::String(s) => self.resolve_string_parts(&s.parts, scope, ctx),
            Expr::Int(_) | Expr::Bool(_) | Expr::Glob(_) => {}
            Expr::List(list) => {
                for item in &list.items {
                    self.resolve_expr(item, scope, ctx);
                }
            }
            Expr::If(if_expr) => {
                self.resolve_expr(&if_expr.then_expr, scope, ctx);
                self.resolve_expr(&if_expr.else_expr, scope, ctx);
                self.check_if_type_compat(if_expr, scope);
            }
            Expr::ModuleCall(call) => {
                // Resolve argument expressions for name resolution, then validate the
                // call (module/method existence, arity, argument types) against the registry.
                for arg in &call.args {
                    self.resolve_expr(&arg.value, scope, ctx);
                }
                self.validate_module_call(call, scope);
            }
            Expr::StageRef(r) => {
                if !scope.stage_names.contains_key(&r.stage) {
                    self.error(
                        format!("unknown stage '{}' in '{}.outputs'", r.stage, r.stage),
                        r.span.clone(),
                    );
                }
            }
            Expr::MemberAccess(m) => self.resolve_member_access(m, scope, ctx),
            Expr::Ident(ident) => self.resolve_ident(ident, scope, ctx),
        }
    }

    fn resolve_string_parts(&mut self, parts: &[StringPart], scope: &Scope, ctx: ExprCtx<'_>) {
        for part in parts {
            if let StringPart::Interpolation(expr) = part {
                self.resolve_expr(expr, scope, ctx);
            }
        }
    }

    fn resolve_member_access(&mut self, m: &MemberAccessExpr, scope: &Scope, ctx: ExprCtx<'_>) {
        if m.object == "project" {
            if !scope.has_project {
                self.error(
                    "cannot access `project.*`: no `project` block is declared",
                    m.span.clone(),
                );
            } else if !scope.project_fields.contains_key(&m.field) {
                self.error(format!("unknown project field '{}'", m.field), m.span.clone());
            }
        } else if ctx.for_vars.contains(&m.object) || scope.let_index(&m.object).is_some() {
            // for-loop variables and let-bound names are valid member-access targets
        } else {
            self.error(
                format!("undefined name '{}' in '{}.{}'", m.object, m.object, m.field),
                m.span.clone(),
            );
        }
    }

    fn resolve_ident(&mut self, ident: &IdentExpr, scope: &Scope, ctx: ExprCtx<'_>) {
        // Built-in variable available everywhere
        if ident.name == "platform" {
            return;
        }
        // Context variables only valid inside step blocks
        if ctx.in_steps
            && (ident.name == "inputs" || ident.name == "outputs" || ident.name == "failed_stage")
        {
            return;
        }
        // for-loop iteration variable
        if ctx.for_vars.contains(&ident.name) {
            return;
        }
        // Matrix dimension bound by the enclosing stage (Phase 37).
        if self.matrix_vars.contains(&ident.name) {
            return;
        }
        // let binding — also enforce forward-reference rule
        if let Some(binding_idx) = scope.let_index(&ident.name) {
            if let Some(current) = ctx.current_let_index
                && binding_idx >= current
            {
                self.error(
                    format!(
                        "forward reference to '{}': a `let` binding may not reference one declared after it",
                        ident.name
                    ),
                    ident.span.clone(),
                );
            }
            return;
        }
        // Stage names are valid bare identifiers (e.g., in pipeline `stages:` lists)
        if scope.stage_names.contains_key(&ident.name) {
            return;
        }
        self.error(format!("undefined name '{}'", ident.name), ident.span.clone());
    }

    // ── Type compatibility ────────────────────────────────────────────────────

    /// Statically infer an expression's type, when known. Module-call results are
    /// resolved from the registry's declared return types; identifiers (whose type
    /// needs binding lookup) yield `None`.
    fn infer_type(&self, expr: &Expr, scope: &Scope) -> Option<ExprType> {
        match expr {
            Expr::String(_) => Some(ExprType::String),
            Expr::Int(_) => Some(ExprType::Int),
            Expr::Bool(_) => Some(ExprType::Bool),
            Expr::List(_) => Some(ExprType::List),
            Expr::Glob(_) | Expr::StageRef(_) => Some(ExprType::FileSet),
            Expr::If(if_expr) => {
                let t = self.infer_type(&if_expr.then_expr, scope);
                let e = self.infer_type(&if_expr.else_expr, scope);
                if t == e { t } else { None }
            }
            Expr::ModuleCall(call) => {
                let module = scope.import_aliases.get(&call.module)?;
                let sig = self.registry.method_sig(module, &call.method)?;
                exprtype_of_valuety(sig.returns)
            }
            // Project field access and file properties are always strings.
            Expr::MemberAccess(_) => Some(ExprType::String),
            // Identifier types require binding lookup — not available statically.
            Expr::Ident(_) => None,
        }
    }

    fn check_if_type_compat(&mut self, if_expr: &IfExpr, scope: &Scope) {
        let then_ty = self.infer_type(&if_expr.then_expr, scope);
        let else_ty = self.infer_type(&if_expr.else_expr, scope);
        if let (Some(t), Some(e)) = (then_ty, else_ty)
            && t != e
        {
            self.error(
                format!(
                    "if/else branches have incompatible types: `then` produces {}, `else` produces {}",
                    t.describe(),
                    e.describe()
                ),
                if_expr.span.clone(),
            );
        }
    }

    // ── Module-call validation ────────────────────────────────────────────────
    //
    // Validates a `<alias>.<method>(...)` call against the registry: the alias is
    // declared, the method exists, and the arguments match the declared signature.
    // The evaluator keeps eval-time checks as a defensive fallback, but a call that
    // passes here should never trip them.

    fn validate_module_call(&mut self, call: &ModuleCallExpr, scope: &Scope) {
        let module_name = match scope.import_aliases.get(&call.module) {
            Some(name) => name.clone(),
            None => {
                self.error(
                    format!(
                        "use of undeclared module '{}'; add `import \"{}\" as {};` at the top of the file",
                        call.module, call.module, call.module
                    ),
                    call.span.clone(),
                );
                return;
            }
        };

        // If the bound module name is not registered, the `import` was already
        // reported as unknown; skip method/argument checks to avoid cascading errors.
        if !self.registry.contains(&module_name) {
            return;
        }

        let sig = match self.registry.method_sig(&module_name, &call.method) {
            Some(s) => s.clone(),
            None => {
                self.error(
                    format!("module '{}' has no method '{}'", module_name, call.method),
                    call.span.clone(),
                );
                return;
            }
        };

        self.check_call_args(call, &module_name, &sig, scope);
    }

    fn check_call_args(
        &mut self,
        call: &ModuleCallExpr,
        module: &str,
        sig: &MethodSig,
        scope: &Scope,
    ) {
        let positional: Vec<&CallArg> = call.args.iter().filter(|a| a.name.is_none()).collect();

        // Positional arity: within [min, max] required/declared positionals.
        let (min, max) = (sig.min_positional(), sig.max_positional());
        if positional.len() < min || positional.len() > max {
            let expected = if min == max { min.to_string() } else { format!("{min} to {max}") };
            self.error(
                format!(
                    "{}.{} expects {} positional argument(s), found {}",
                    module,
                    sig.name,
                    expected,
                    positional.len()
                ),
                call.span.clone(),
            );
        }

        // Positional argument types (only where a literal type is statically known).
        for (i, arg) in positional.iter().enumerate() {
            if let Some(param) = sig.params.get(i) {
                self.check_arg_type(
                    arg,
                    param.ty,
                    &format!("{}.{} positional argument {}", module, sig.name, i + 1),
                    scope,
                );
            }
        }

        // Named arguments: each must be a recognized keyword, and well-typed.
        for arg in call.args.iter().filter(|a| a.name.is_some()) {
            let name = arg.name.as_deref().unwrap();
            match sig.named_param(name) {
                Some(np) => self.check_arg_type(
                    arg,
                    np.ty,
                    &format!("{}.{} argument '{}'", module, sig.name, name),
                    scope,
                ),
                None => self.error(
                    format!("{}.{} has no named argument '{}'", module, sig.name, name),
                    arg.span.clone(),
                ),
            }
        }

        // Required named arguments must be present.
        for np in sig.named.iter().filter(|p| p.required) {
            let present = call.args.iter().any(|a| a.name.as_deref() == Some(np.name.as_str()));
            if !present {
                self.error(
                    format!("{}.{} requires named argument '{}'", module, sig.name, np.name),
                    call.span.clone(),
                );
            }
        }
    }

    /// Check a single argument's statically-inferable type against its declared type.
    /// Arguments whose type is not known until runtime (e.g. identifiers) are skipped.
    fn check_arg_type(&mut self, arg: &CallArg, expected: ValueTy, what: &str, scope: &Scope) {
        if let Some(actual) = self.infer_type(&arg.value, scope)
            && !accepts_ty(expected, &actual)
        {
            self.error(
                format!("{} must be {}, found {}", what, expected.describe(), actual.describe()),
                arg.span.clone(),
            );
        }
    }
}

/// Whether a parameter declared as `expected` accepts a statically-inferred `actual`.
fn accepts_ty(expected: ValueTy, actual: &ExprType) -> bool {
    match expected {
        ValueTy::Any => true,
        ValueTy::String => matches!(actual, ExprType::String),
        ValueTy::Int => matches!(actual, ExprType::Int),
        ValueTy::Bool => matches!(actual, ExprType::Bool),
        ValueTy::List => matches!(actual, ExprType::List),
        ValueTy::FileSet => matches!(actual, ExprType::FileSet),
    }
}

// ── Dependency graph ───────────────────────────────────────────────────────────

fn build_dependency_graph(program: &Program) -> HashMap<String, Vec<String>> {
    let mut graph: HashMap<String, Vec<String>> = HashMap::new();
    for item in &program.items {
        if let Item::Stage(stage) = item {
            let mut deps = Vec::new();
            if let Some(inputs) = &stage.inputs {
                collect_stage_refs(inputs, &mut deps);
            }
            // Explicit `depends_on` edges sit alongside the inferred `<stage>.outputs` ones.
            for dep in &stage.depends_on {
                if !deps.contains(&dep.name) {
                    deps.push(dep.name.clone());
                }
            }
            graph.insert(stage.name.clone(), deps);
        }
    }
    graph
}

/// Detect dependency cycles over the full stage graph and return one diagnostic per cycle,
/// anchored at a stage involved in it. Uses Kahn's algorithm (the same approach as the
/// runtime toposort) so it never recurses on adversarial input: nodes whose dependencies
/// can all be removed are peeled away; whatever remains forms one or more cycles.
fn detect_cycles(program: &Program, graph: &HashMap<String, Vec<String>>) -> Vec<Diagnostic> {
    // Stage declaration order and spans, for deterministic, locatable diagnostics.
    let order: Vec<&str> = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Stage(s) => Some(s.name.as_str()),
            _ => None,
        })
        .collect();
    let spans: HashMap<&str, &Span> = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Stage(s) => Some((s.name.as_str(), &s.span)),
            _ => None,
        })
        .collect();

    // Remaining unsatisfied dependencies per node, counting only edges to real stages.
    let mut remaining: HashMap<&str, usize> = HashMap::new();
    // Reverse adjacency: for each dependency, the stages that depend on it.
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();
    for &name in &order {
        let deps: Vec<&str> =
            graph[name].iter().map(String::as_str).filter(|d| graph.contains_key(*d)).collect();
        remaining.insert(name, deps.len());
        for d in deps {
            dependents.entry(d).or_default().push(name);
        }
    }

    // Peel nodes with no remaining dependencies, in declaration order.
    let mut queue: Vec<&str> = order.iter().copied().filter(|n| remaining[n] == 0).collect();
    let mut removed = 0usize;
    while let Some(node) = queue.pop() {
        removed += 1;
        if let Some(deps) = dependents.get(node) {
            for &m in deps {
                let r = remaining.get_mut(m).unwrap();
                *r -= 1;
                if *r == 0 {
                    queue.push(m);
                }
            }
        }
    }

    if removed == order.len() {
        return Vec::new();
    }

    // Report the earliest-declared stage still entangled in a cycle.
    let culprit = order.iter().copied().find(|n| remaining[n] > 0).unwrap();
    let span = spans[culprit].clone();
    vec![
        Diagnostic::new(format!("stage '{culprit}' is part of a dependency cycle")).with_span(span),
    ]
}

fn collect_stage_refs(expr: &Expr, out: &mut Vec<String>) {
    match expr {
        Expr::StageRef(r) => out.push(r.stage.clone()),
        Expr::List(list) => {
            for item in &list.items {
                collect_stage_refs(item, out);
            }
        }
        Expr::If(if_expr) => {
            collect_stage_refs(&if_expr.then_expr, out);
            collect_stage_refs(&if_expr.else_expr, out);
        }
        _ => {}
    }
}

// ── Type inference ─────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
enum ExprType {
    String,
    Int,
    Bool,
    List,
    FileSet,
}

impl ExprType {
    fn describe(&self) -> &'static str {
        match self {
            ExprType::String => "string",
            ExprType::Int => "int",
            ExprType::Bool => "bool",
            ExprType::List => "list",
            ExprType::FileSet => "fileset",
        }
    }
}

/// The `ExprType` corresponding to a declared module-return `ValueTy`, or `None`
/// for `ValueTy::Any` (no statically-known type).
fn exprtype_of_valuety(ty: ValueTy) -> Option<ExprType> {
    match ty {
        ValueTy::String => Some(ExprType::String),
        ValueTy::Int => Some(ExprType::Int),
        ValueTy::Bool => Some(ExprType::Bool),
        ValueTy::List => Some(ExprType::List),
        ValueTy::FileSet => Some(ExprType::FileSet),
        ValueTy::Any => None,
    }
}

// ── Scope ──────────────────────────────────────────────────────────────────────

#[derive(Default)]
struct Scope {
    // Vec preserves declaration order — required for forward-reference checking.
    let_bindings: Vec<(String, Span)>,
    stage_names: HashMap<String, Span>,
    pipeline_names: HashMap<String, Span>,
    /// Maps each import alias to the raw module name it binds (e.g. `vcs → "git"`),
    /// so module calls can be validated against the registry.
    import_aliases: HashMap<String, String>,
    project_fields: HashMap<String, Span>,
    has_project: bool,
}

impl Scope {
    fn let_index(&self, name: &str) -> Option<usize> {
        self.let_bindings.iter().position(|(n, _)| n == name)
    }
}

// ── Expression context ─────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct ExprCtx<'a> {
    /// The declaration index of the `let` binding currently being resolved, if any.
    /// Used to detect forward references.
    current_let_index: Option<usize>,
    /// Names bound by enclosing `for` loops — valid as bare identifiers and as
    /// member-access objects inside the loop body.
    for_vars: &'a [String],
    /// True when the expression appears inside a step block, making `inputs` and
    /// `outputs` valid as built-in identifiers.
    in_steps: bool,
}

impl ExprCtx<'static> {
    fn top_level() -> Self {
        Self { current_let_index: None, for_vars: &[], in_steps: false }
    }
}
