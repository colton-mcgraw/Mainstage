use std::collections::HashMap;

use crate::{
    ast::*,
    error::{Diagnostic, Error, Result, Span},
};

// ── Public API ─────────────────────────────────────────────────────────────────

/// The validated output of Phase 2 semantic analysis.
pub struct AnalysisResult {
    /// Stage dependency adjacency list: stage name → stages it directly depends on.
    /// Edges are inferred from `<stage>.outputs` references in a stage's `inputs` field.
    pub dependency_graph: HashMap<String, Vec<String>>,
}

/// Run all Phase 2 semantic checks on `program`.
///
/// Returns `Ok(AnalysisResult)` when the program is semantically valid, or
/// `Err(Error::Semantic(...))` with every diagnostic found during analysis.
pub fn analyze(program: &Program) -> Result<AnalysisResult> {
    let mut a = Analyzer::new();
    let result = a.run(program);
    if a.errors.is_empty() {
        Ok(result)
    } else {
        Err(Error::Semantic(a.errors))
    }
}

// ── Analyzer ───────────────────────────────────────────────────────────────────

struct Analyzer {
    errors: Vec<Diagnostic>,
}

impl Analyzer {
    fn new() -> Self {
        Self { errors: Vec::new() }
    }

    fn error(&mut self, msg: impl Into<String>, span: Span) {
        self.errors.push(Diagnostic::new(msg).with_span(span));
    }

    fn run(&mut self, program: &Program) -> AnalysisResult {
        let scope = self.collect_scope(program);
        self.resolve_program(program, &scope);
        AnalysisResult { dependency_graph: build_dependency_graph(program) }
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
                    if scope.import_aliases.contains_key(&d.alias) {
                        self.error(
                            format!("import alias '{}' is already defined", d.alias),
                            d.span.clone(),
                        );
                    } else {
                        scope.import_aliases.insert(d.alias.clone(), d.span.clone());
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
                    let ctx =
                        ExprCtx { current_let_index: Some(let_idx), for_vars: &[], in_steps: false };
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
        let ctx = ExprCtx::top_level();
        if let Some(expr) = &stage.inputs {
            self.resolve_expr(expr, scope, ctx);
        }
        if let Some(expr) = &stage.outputs {
            self.resolve_expr(expr, scope, ctx);
        }
        for step in &stage.steps {
            self.resolve_step(step, scope, &[]);
        }
        for step in &stage.on_failure {
            self.resolve_step(step, scope, &[]);
        }
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
        }
    }

    fn resolve_expr(&mut self, expr: &Expr, scope: &Scope, ctx: ExprCtx<'_>) {
        match expr {
            Expr::String(s) => self.resolve_string_parts(&s.parts, scope, ctx),
            Expr::Bool(_) | Expr::Glob(_) => {}
            Expr::List(list) => {
                for item in &list.items {
                    self.resolve_expr(item, scope, ctx);
                }
            }
            Expr::If(if_expr) => {
                self.resolve_expr(&if_expr.then_expr, scope, ctx);
                self.resolve_expr(&if_expr.else_expr, scope, ctx);
                self.check_if_type_compat(if_expr);
            }
            Expr::ModuleCall(call) => {
                if !scope.import_aliases.contains_key(&call.module) {
                    self.error(
                        format!(
                            "use of undeclared module '{}'; add `import \"{}\" as {};` at the top of the file",
                            call.module, call.module, call.module
                        ),
                        call.span.clone(),
                    );
                }
                for arg in &call.args {
                    self.resolve_expr(&arg.value, scope, ctx);
                }
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

    fn resolve_member_access(
        &mut self,
        m: &MemberAccessExpr,
        scope: &Scope,
        ctx: ExprCtx<'_>,
    ) {
        if m.object == "project" {
            if !scope.has_project {
                self.error(
                    "cannot access `project.*`: no `project` block is declared",
                    m.span.clone(),
                );
            } else if !scope.project_fields.contains_key(&m.field) {
                self.error(
                    format!("unknown project field '{}'", m.field),
                    m.span.clone(),
                );
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
            && (ident.name == "inputs"
                || ident.name == "outputs"
                || ident.name == "failed_stage")
        {
            return;
        }
        // for-loop iteration variable
        if ctx.for_vars.contains(&ident.name) {
            return;
        }
        // let binding — also enforce forward-reference rule
        if let Some(binding_idx) = scope.let_index(&ident.name) {
            if let Some(current) = ctx.current_let_index {
                if binding_idx >= current {
                    self.error(
                        format!(
                            "forward reference to '{}': a `let` binding may not reference one declared after it",
                            ident.name
                        ),
                        ident.span.clone(),
                    );
                }
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

    fn check_if_type_compat(&mut self, if_expr: &IfExpr) {
        let then_ty = infer_expr_type(&if_expr.then_expr);
        let else_ty = infer_expr_type(&if_expr.else_expr);
        if let (Some(t), Some(e)) = (then_ty, else_ty) {
            if t != e {
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
            graph.insert(stage.name.clone(), deps);
        }
    }
    graph
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
    Bool,
    List,
    FileSet,
}

impl ExprType {
    fn describe(&self) -> &'static str {
        match self {
            ExprType::String => "string",
            ExprType::Bool => "bool",
            ExprType::List => "list",
            ExprType::FileSet => "fileset",
        }
    }
}

fn infer_expr_type(expr: &Expr) -> Option<ExprType> {
    match expr {
        Expr::String(_) => Some(ExprType::String),
        Expr::Bool(_) => Some(ExprType::Bool),
        Expr::List(_) => Some(ExprType::List),
        Expr::Glob(_) | Expr::StageRef(_) => Some(ExprType::FileSet),
        Expr::If(if_expr) => {
            let t = infer_expr_type(&if_expr.then_expr);
            let e = infer_expr_type(&if_expr.else_expr);
            if t == e { t } else { None }
        }
        // Module calls and project field access always produce strings
        Expr::ModuleCall(_) | Expr::MemberAccess(_) => Some(ExprType::String),
        // Ident types require binding lookup — not available statically
        Expr::Ident(_) => None,
    }
}

// ── Scope ──────────────────────────────────────────────────────────────────────

#[derive(Default)]
struct Scope {
    // Vec preserves declaration order — required for forward-reference checking.
    let_bindings: Vec<(String, Span)>,
    stage_names: HashMap<String, Span>,
    pipeline_names: HashMap<String, Span>,
    import_aliases: HashMap<String, Span>,
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
