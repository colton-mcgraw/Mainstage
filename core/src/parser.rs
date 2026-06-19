use std::path::PathBuf;

use pest::{Parser, iterators::Pair};
use pest_derive::Parser;

use crate::{
    ast::*,
    error::{Diagnostic, Error, Result, Span},
    source::Source,
};

#[derive(Parser)]
#[grammar = "grammar.pest"]
struct MainstageParser;

// ── Public entry point ────────────────────────────────────────────────────────

/// Parse a [`Source`] and return its typed [`Program`] AST, or an [`Error::Parse`]
/// containing all diagnostics collected during parsing.
pub fn parse(source: &Source) -> Result<Program> {
    let pairs = MainstageParser::parse(Rule::program, &source.text)
        .map_err(|e| Error::Parse(vec![pest_error_to_diagnostic(e, &source.path)]))?;

    let mut b = Builder { path: source.path.clone(), errors: Vec::new() };
    let program_pair = pairs.into_iter().next().unwrap();
    let program = b.build_program(program_pair);

    if b.errors.is_empty() { Ok(program) } else { Err(Error::Parse(b.errors)) }
}

// ── Builder ───────────────────────────────────────────────────────────────────

/// Walks the pest parse tree and constructs the typed AST, accumulating non-fatal
/// errors rather than aborting at the first one.
struct Builder {
    path: PathBuf,
    errors: Vec<Diagnostic>,
}

impl Builder {
    fn span(&self, pair: &Pair<Rule>) -> Span {
        let s = pair.as_span();
        let (line_start, col_start) = s.start_pos().line_col();
        let (line_end, col_end) = s.end_pos().line_col();
        Span { file: self.path.clone(), line_start, col_start, line_end, col_end }
    }

    fn error(&mut self, msg: impl Into<String>, span: Span) {
        self.errors.push(Diagnostic::new(msg).with_span(span));
    }

    // ── Program ───────────────────────────────────────────────────────────────

    fn build_program(&mut self, pair: Pair<Rule>) -> Program {
        let span = self.span(&pair);
        let items = pair
            .into_inner()
            .filter(|p| p.as_rule() != Rule::EOI)
            .map(|p| self.build_item(p))
            .collect();
        Program { items, span }
    }

    // ── Items ─────────────────────────────────────────────────────────────────
    // `item` is a silent rule; we receive the concrete rule pairs directly.

    fn build_item(&mut self, pair: Pair<Rule>) -> Item {
        match pair.as_rule() {
            Rule::import_decl => Item::Import(self.build_import(pair)),
            Rule::let_decl => Item::Let(self.build_let(pair)),
            Rule::project_block => Item::Project(self.build_project(pair)),
            Rule::stage_block => Item::Stage(self.build_stage(pair)),
            Rule::pipeline_block => Item::Pipeline(self.build_pipeline(pair)),
            r => unreachable!("unexpected item rule: {:?}", r),
        }
    }

    fn build_import(&mut self, pair: Pair<Rule>) -> ImportDecl {
        let span = self.span(&pair);
        let mut inner = pair.into_inner();
        let module = self.extract_raw_string(inner.next().unwrap());
        let alias = inner.next().unwrap().as_str().to_string();
        ImportDecl { module, alias, span }
    }

    fn build_let(&mut self, pair: Pair<Rule>) -> LetDecl {
        let span = self.span(&pair);
        let mut inner = pair.into_inner();
        let name = inner.next().unwrap().as_str().to_string();
        let value = self.build_expr(inner.next().unwrap());
        LetDecl { name, value, span }
    }

    // ── Project ───────────────────────────────────────────────────────────────

    fn build_project(&mut self, pair: Pair<Rule>) -> ProjectBlock {
        let span = self.span(&pair);
        let fields = pair.into_inner().map(|p| self.build_project_field(p)).collect();
        ProjectBlock { fields, span }
    }

    fn build_project_field(&mut self, pair: Pair<Rule>) -> ProjectField {
        let span = self.span(&pair);
        let mut inner = pair.into_inner();
        let name = inner.next().unwrap().as_str().to_string();
        let value = self.build_expr(inner.next().unwrap());
        ProjectField { name, value, span }
    }

    // ── Stage ─────────────────────────────────────────────────────────────────

    fn build_stage(&mut self, pair: Pair<Rule>) -> StageBlock {
        let span = self.span(&pair);
        let mut inner = pair.into_inner();
        let name = inner.next().unwrap().as_str().to_string();

        let mut inputs = None;
        let mut outputs = None;
        let mut depends_on = Vec::new();
        let mut matrix = Vec::new();
        let mut allow_failure = false;
        let mut always_run = false;
        let mut run_once = false;
        let mut steps = Vec::new();
        let mut on_failure = Vec::new();

        for field in inner {
            match field.as_rule() {
                Rule::stage_inputs => {
                    inputs = Some(self.build_expr(field.into_inner().next().unwrap()));
                }
                Rule::stage_outputs => {
                    outputs = Some(self.build_expr(field.into_inner().next().unwrap()));
                }
                Rule::stage_depends_on => {
                    depends_on = field
                        .into_inner()
                        .map(|p| StageDep { name: p.as_str().to_string(), span: self.span(&p) })
                        .collect();
                }
                Rule::stage_matrix => {
                    matrix = field.into_inner().map(|p| self.build_matrix_dim(p)).collect();
                }
                Rule::stage_allow_failure => {
                    let val = field.into_inner().next().unwrap().as_str();
                    allow_failure = val == "true";
                }
                Rule::stage_always_run => {
                    let val = field.into_inner().next().unwrap().as_str();
                    always_run = val == "true";
                }
                Rule::stage_run_once => {
                    let val = field.into_inner().next().unwrap().as_str();
                    run_once = val == "true";
                }
                Rule::steps_block => {
                    steps = field.into_inner().map(|p| self.build_step(p)).collect();
                }
                Rule::stage_on_failure => {
                    on_failure = field.into_inner().map(|p| self.build_step(p)).collect();
                }
                r => unreachable!("unexpected stage_field rule: {:?}", r),
            }
        }

        StageBlock {
            name,
            inputs,
            outputs,
            depends_on,
            matrix,
            matrix_bindings: Vec::new(),
            allow_failure,
            always_run,
            run_once,
            steps,
            on_failure,
            span,
        }
    }

    /// Build one `matrix` dimension: an identifier name followed by a bracketed list of
    /// static string values. Interpolation in a value is rejected (it must be static so
    /// it can form part of a generated stage name).
    fn build_matrix_dim(&mut self, pair: Pair<Rule>) -> MatrixDim {
        let span = self.span(&pair);
        let mut inner = pair.into_inner();
        let name = inner.next().unwrap().as_str().to_string();
        let values = inner
            .map(|p| {
                let span = self.span(&p);
                MatrixValue { value: self.extract_raw_string(p), span }
            })
            .collect();
        MatrixDim { name, values, span }
    }

    // ── Pipeline ──────────────────────────────────────────────────────────────

    fn build_pipeline(&mut self, pair: Pair<Rule>) -> PipelineBlock {
        let span = self.span(&pair);
        let mut inner = pair.into_inner().peekable();

        let is_default = if inner.peek().map(|p| p.as_rule()) == Some(Rule::pipeline_default) {
            inner.next();
            true
        } else {
            false
        };

        let name = inner.next().unwrap().as_str().to_string();

        let mut input = None;
        let mut stages = None;
        let mut on_failure = Vec::new();
        let mut on_success = Vec::new();

        for field in inner {
            match field.as_rule() {
                Rule::pipeline_input => {
                    input = Some(self.build_expr(field.into_inner().next().unwrap()));
                }
                Rule::pipeline_stages => {
                    stages = Some(self.build_expr(field.into_inner().next().unwrap()));
                }
                Rule::pipeline_on_failure => {
                    on_failure = field.into_inner().map(|p| self.build_step(p)).collect();
                }
                Rule::pipeline_on_success => {
                    on_success = field.into_inner().map(|p| self.build_step(p)).collect();
                }
                r => unreachable!("unexpected pipeline_field rule: {:?}", r),
            }
        }

        PipelineBlock { is_default, name, input, stages, on_failure, on_success, span }
    }

    // ── Steps ─────────────────────────────────────────────────────────────────
    // `step` is a silent rule; we receive the concrete step rule pairs directly.

    fn build_step(&mut self, pair: Pair<Rule>) -> Step {
        match pair.as_rule() {
            Rule::exec_step => Step::Exec(self.build_exec_step(pair)),
            Rule::copy_step => Step::Copy(self.build_copy_step(pair)),
            Rule::move_step => Step::Move(self.build_move_step(pair)),
            Rule::mkdir_step => Step::Mkdir(self.build_mkdir_step(pair)),
            Rule::delete_step => Step::Delete(self.build_delete_step(pair)),
            Rule::write_step => Step::Write(self.build_write_step(pair)),
            Rule::if_step => Step::If(self.build_if_step(pair)),
            Rule::for_step => Step::For(self.build_for_step(pair)),
            Rule::try_step => Step::Try(self.build_try_step(pair)),
            r => unreachable!("unexpected step rule: {:?}", r),
        }
    }

    fn build_exec_step(&mut self, pair: Pair<Rule>) -> ExecStep {
        let span = self.span(&pair);
        // exec_step is compound-atomic ($); exec_line is its only inner rule.
        let command =
            pair.into_inner().next().map(|p| p.as_str().trim_end().to_string()).unwrap_or_default();
        ExecStep { command, span }
    }

    fn build_copy_step(&mut self, pair: Pair<Rule>) -> CopyStep {
        let span = self.span(&pair);
        let mut inner = pair.into_inner();
        let src = self.build_expr(inner.next().unwrap());
        let dest = self.build_expr(inner.next().unwrap());
        CopyStep { src, dest, span }
    }

    fn build_move_step(&mut self, pair: Pair<Rule>) -> MoveStep {
        let span = self.span(&pair);
        let mut inner = pair.into_inner();
        let src = self.build_expr(inner.next().unwrap());
        let dest = self.build_expr(inner.next().unwrap());
        MoveStep { src, dest, span }
    }

    fn build_mkdir_step(&mut self, pair: Pair<Rule>) -> MkdirStep {
        let span = self.span(&pair);
        let path = self.build_expr(pair.into_inner().next().unwrap());
        MkdirStep { path, span }
    }

    fn build_delete_step(&mut self, pair: Pair<Rule>) -> DeleteStep {
        let span = self.span(&pair);
        let path = self.build_expr(pair.into_inner().next().unwrap());
        DeleteStep { path, span }
    }

    fn build_write_step(&mut self, pair: Pair<Rule>) -> WriteStep {
        let span = self.span(&pair);
        let mut inner = pair.into_inner();
        let path = self.build_expr(inner.next().unwrap());
        let content = self.build_string(inner.next().unwrap());
        WriteStep { path, content, span }
    }

    fn build_if_step(&mut self, pair: Pair<Rule>) -> IfStep {
        let span = self.span(&pair);
        let mut inner = pair.into_inner();
        let condition = self.build_condition(inner.next().unwrap());
        let then_steps = self.build_step_block(inner.next().unwrap());
        let else_steps = match inner.next() {
            Some(else_block) => self.build_step_block(else_block),
            None => Vec::new(),
        };
        IfStep { condition, then_steps, else_steps, span }
    }

    fn build_step_block(&mut self, pair: Pair<Rule>) -> Vec<Step> {
        pair.into_inner().map(|p| self.build_step(p)).collect()
    }

    fn build_for_step(&mut self, pair: Pair<Rule>) -> ForStep {
        let span = self.span(&pair);
        let mut inner = pair.into_inner();
        let var = inner.next().unwrap().as_str().to_string();
        let iterable = self.build_expr(inner.next().unwrap());
        let steps = inner.map(|p| self.build_step(p)).collect();
        ForStep { var, iterable, steps, span }
    }

    fn build_try_step(&mut self, pair: Pair<Rule>) -> TryStep {
        let span = self.span(&pair);
        let steps = pair.into_inner().map(|p| self.build_step(p)).collect();
        TryStep { steps, span }
    }

    // ── Expressions ───────────────────────────────────────────────────────────

    fn build_expr(&mut self, pair: Pair<Rule>) -> Expr {
        let inner = pair.into_inner().next().unwrap();
        match inner.as_rule() {
            Rule::if_expr => Expr::If(Box::new(self.build_if_expr(inner))),
            Rule::glob_expr => Expr::Glob(self.build_glob_expr(inner)),
            Rule::module_call => Expr::ModuleCall(self.build_module_call(inner)),
            Rule::stage_ref => Expr::StageRef(self.build_stage_ref(inner)),
            Rule::member_access => Expr::MemberAccess(self.build_member_access(inner)),
            Rule::list_expr => Expr::List(self.build_list_expr(inner)),
            Rule::string => Expr::String(self.build_string(inner)),
            Rule::int_lit => Expr::Int(self.build_int_lit(inner)),
            Rule::bool_lit => Expr::Bool(self.build_bool_lit(inner)),
            Rule::ident => Expr::Ident(self.build_ident_expr(inner)),
            r => unreachable!("unexpected expr rule: {:?}", r),
        }
    }

    fn build_if_expr(&mut self, pair: Pair<Rule>) -> IfExpr {
        let span = self.span(&pair);
        let mut inner = pair.into_inner();
        let condition = self.build_condition(inner.next().unwrap());
        let then_expr = self.build_expr(inner.next().unwrap());
        let else_expr = self.build_expr(inner.next().unwrap());
        IfExpr { condition, then_expr, else_expr, span }
    }

    fn build_glob_expr(&mut self, pair: Pair<Rule>) -> GlobExpr {
        let span = self.span(&pair);
        let patterns = pair.into_inner().map(|p| self.extract_raw_string(p)).collect();
        GlobExpr { patterns, span }
    }

    fn build_module_call(&mut self, pair: Pair<Rule>) -> ModuleCallExpr {
        let span = self.span(&pair);
        let mut inner = pair.into_inner();
        let module = inner.next().unwrap().as_str().to_string();
        let method = inner.next().unwrap().as_str().to_string();
        let args = match inner.next() {
            Some(arg_list) => self.build_arg_list(arg_list),
            None => Vec::new(),
        };
        ModuleCallExpr { module, method, args, span }
    }

    fn build_arg_list(&mut self, pair: Pair<Rule>) -> Vec<CallArg> {
        pair.into_inner().map(|p| self.build_arg(p)).collect()
    }

    fn build_arg(&mut self, pair: Pair<Rule>) -> CallArg {
        let span = self.span(&pair);
        let inner = pair.into_inner().next().unwrap();
        match inner.as_rule() {
            Rule::named_arg => {
                let mut parts = inner.into_inner();
                // arg_key (allows keywords as names) followed by expr
                let name = parts.next().unwrap().as_str().to_string();
                let value = self.build_expr(parts.next().unwrap());
                CallArg { name: Some(name), value, span }
            }
            Rule::expr => {
                let value = self.build_expr(inner);
                CallArg { name: None, value, span }
            }
            r => unreachable!("unexpected arg rule: {:?}", r),
        }
    }

    fn build_stage_ref(&self, pair: Pair<Rule>) -> StageRefExpr {
        let span = self.span(&pair);
        let stage = pair.into_inner().next().unwrap().as_str().to_string();
        StageRefExpr { stage, span }
    }

    fn build_member_access(&self, pair: Pair<Rule>) -> MemberAccessExpr {
        let span = self.span(&pair);
        let mut inner = pair.into_inner();
        let object = inner.next().unwrap().as_str().to_string();
        let field = inner.next().unwrap().as_str().to_string();
        MemberAccessExpr { object, field, span }
    }

    fn build_list_expr(&mut self, pair: Pair<Rule>) -> ListExpr {
        let span = self.span(&pair);
        let items = pair.into_inner().map(|p| self.build_expr(p)).collect();
        ListExpr { items, span }
    }

    fn build_bool_lit(&self, pair: Pair<Rule>) -> BoolExpr {
        let span = self.span(&pair);
        let value = pair.as_str() == "true";
        BoolExpr { value, span }
    }

    fn build_int_lit(&mut self, pair: Pair<Rule>) -> IntExpr {
        let span = self.span(&pair);
        // The grammar guarantees `[-]?digits`; the only failure mode is i64 overflow.
        let value = pair.as_str().parse::<i64>().unwrap_or_else(|_| {
            self.error(
                format!("integer literal '{}' is out of range for a 64-bit integer", pair.as_str()),
                span.clone(),
            );
            0
        });
        IntExpr { value, span }
    }

    fn build_ident_expr(&self, pair: Pair<Rule>) -> IdentExpr {
        let span = self.span(&pair);
        let name = pair.as_str().to_string();
        IdentExpr { name, span }
    }

    // ── Strings ───────────────────────────────────────────────────────────────

    fn build_string(&mut self, pair: Pair<Rule>) -> StringExpr {
        let span = self.span(&pair);
        let string_inner = pair.into_inner().next().unwrap();
        let mut parts = Vec::new();
        for part_pair in string_inner.into_inner() {
            let inner = part_pair.into_inner().next().unwrap();
            match inner.as_rule() {
                Rule::string_raw => {
                    parts.push(StringPart::Literal(inner.as_str().to_string()));
                }
                Rule::interpolation => {
                    let expr_pair = inner.into_inner().next().unwrap();
                    parts.push(StringPart::Interpolation(Box::new(self.build_expr(expr_pair))));
                }
                r => unreachable!("unexpected string_part rule: {:?}", r),
            }
        }
        StringExpr { parts, span }
    }

    /// Extract a plain string value (no interpolation) — used for import paths,
    /// env var names, and condition comparands where interpolation is not allowed.
    fn extract_raw_string(&mut self, pair: Pair<Rule>) -> String {
        let string_inner = pair.into_inner().next().unwrap();
        let mut s = String::new();
        for part_pair in string_inner.into_inner() {
            let inner = part_pair.into_inner().next().unwrap();
            match inner.as_rule() {
                Rule::string_raw => s.push_str(inner.as_str()),
                Rule::interpolation => {
                    let span = self.span(&inner);
                    self.error("string interpolation is not allowed here", span);
                }
                r => unreachable!("unexpected string_part rule: {:?}", r),
            }
        }
        s
    }

    // ── Conditions ────────────────────────────────────────────────────────────

    fn build_condition(&mut self, pair: Pair<Rule>) -> Condition {
        self.build_or_cond(pair.into_inner().next().unwrap())
    }

    fn build_or_cond(&mut self, pair: Pair<Rule>) -> Condition {
        let span = self.span(&pair);
        let mut inner = pair.into_inner();
        let mut result = self.build_and_cond(inner.next().unwrap());
        for rhs_pair in inner {
            let rhs = self.build_and_cond(rhs_pair);
            let combined = self.merge_spans(result.span(), rhs.span());
            result = Condition::Or(Box::new(result), Box::new(rhs), combined);
        }
        let _ = span;
        result
    }

    fn build_and_cond(&mut self, pair: Pair<Rule>) -> Condition {
        let span = self.span(&pair);
        let mut inner = pair.into_inner();
        let mut result = self.build_unary_cond(inner.next().unwrap());
        for rhs_pair in inner {
            let rhs = self.build_unary_cond(rhs_pair);
            let combined = self.merge_spans(result.span(), rhs.span());
            result = Condition::And(Box::new(result), Box::new(rhs), combined);
        }
        let _ = span;
        result
    }

    fn build_unary_cond(&mut self, pair: Pair<Rule>) -> Condition {
        let span = self.span(&pair);
        let inner = pair.into_inner().next().unwrap();
        match inner.as_rule() {
            Rule::unary_cond => Condition::Not(Box::new(self.build_unary_cond(inner)), span),
            Rule::primary_cond => self.build_primary_cond(inner),
            r => unreachable!("unexpected unary_cond rule: {:?}", r),
        }
    }

    fn build_primary_cond(&mut self, pair: Pair<Rule>) -> Condition {
        let inner = pair.into_inner().next().unwrap();
        match inner.as_rule() {
            Rule::condition => self.build_condition(inner),
            Rule::env_cond => self.build_env_cond(inner),
            Rule::platform_cond => self.build_platform_cond(inner),
            r => unreachable!("unexpected primary_cond rule: {:?}", r),
        }
    }

    fn build_env_cond(&mut self, pair: Pair<Rule>) -> Condition {
        let span = self.span(&pair);
        let mut inner = pair.into_inner();
        let var = self.extract_raw_string(inner.next().unwrap());
        let comparison = match (inner.next(), inner.next()) {
            (Some(op_pair), Some(val_pair)) => {
                let op = parse_compare_op(op_pair.as_str());
                let val = self.extract_raw_string(val_pair);
                Some((op, val))
            }
            _ => None,
        };
        Condition::Env(EnvCondition { var, comparison, span })
    }

    fn build_platform_cond(&mut self, pair: Pair<Rule>) -> Condition {
        let span = self.span(&pair);
        let mut inner = pair.into_inner();
        let op = parse_compare_op(inner.next().unwrap().as_str());
        let val_pair = inner.next().unwrap();
        let val_span = self.span(&val_pair);
        let val_str = self.extract_raw_string(val_pair);
        let platform = match val_str.as_str() {
            "windows" => Platform::Windows,
            "linux" => Platform::Linux,
            "macos" => Platform::MacOs,
            other => {
                self.error(
                    format!(
                        "unknown platform '{}'; expected \"windows\", \"linux\", or \"macos\"",
                        other
                    ),
                    val_span,
                );
                Platform::Windows
            }
        };
        Condition::Platform(PlatformCondition { op, value: platform, span })
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn merge_spans(&self, a: &Span, b: &Span) -> Span {
        Span {
            file: self.path.clone(),
            line_start: a.line_start,
            col_start: a.col_start,
            line_end: b.line_end,
            col_end: b.col_end,
        }
    }
}

// ── Free helpers ──────────────────────────────────────────────────────────────

fn parse_compare_op(s: &str) -> CompareOp {
    match s {
        "==" => CompareOp::Eq,
        "!=" => CompareOp::Ne,
        _ => unreachable!("unexpected compare_op: {}", s),
    }
}

fn pest_error_to_diagnostic(e: pest::error::Error<Rule>, path: &std::path::Path) -> Diagnostic {
    let (line, col) = match e.line_col {
        pest::error::LineColLocation::Pos((l, c)) => (l, c),
        pest::error::LineColLocation::Span((l, c), _) => (l, c),
    };
    let span = Span {
        file: path.to_path_buf(),
        line_start: line,
        col_start: col,
        line_end: line,
        col_end: col,
    };
    Diagnostic::new(e.variant.message().to_string()).with_span(span)
}
