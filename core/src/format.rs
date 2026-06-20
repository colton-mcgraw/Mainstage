//! Source formatter (Goal 3, Phase 21).
//!
//! A pretty-printer that renders a parsed [`Program`] back to canonical Mainstage
//! source. It is built on the trivia-aware layer from Phase 20: comments captured by
//! [`crate::trivia::lex`] are attached to AST nodes by [`crate::trivia::attach`] and
//! re-emitted here, so formatting never drops a comment.
//!
//! Canonical style is deliberately simple and deterministic — four-space indentation,
//! single spaces around `=` / after `:`, no column alignment — which makes the output
//! stable under repeated formatting (`format(format(x)) == format(x)`).

use crate::ast::*;
use crate::error::{Result, Span};
use crate::parser::parse;
use crate::source::Source;
use crate::trivia::{CommentKind, TriviaMap, attach, lex};

/// Format `source` into canonical Mainstage source text.
///
/// Returns the original parse error when the source is syntactically invalid — a
/// formatter cannot lay out a tree it cannot build.
pub fn format(source: &Source) -> Result<String> {
    let program = parse(source)?;
    let tokens = lex(source);
    let trivia = attach(&program, &tokens);
    let mut printer = Printer { out: String::new(), indent: 0, trivia: &trivia };
    printer.program(&program);
    Ok(printer.out)
}

/// One indentation level — four spaces.
const INDENT: &str = "    ";

struct Printer<'a> {
    out: String,
    indent: usize,
    trivia: &'a TriviaMap,
}

impl Printer<'_> {
    // ── Output primitives ────────────────────────────────────────────────────────

    /// Append `text` on its own line at the current indentation, then a newline.
    /// An empty `text` emits a bare blank line.
    fn push_line(&mut self, text: &str) {
        if !text.is_empty() {
            for _ in 0..self.indent {
                self.out.push_str(INDENT);
            }
            self.out.push_str(text);
        }
        self.out.push('\n');
    }

    /// Emit a bare blank line (no indentation).
    fn blank(&mut self) {
        self.out.push('\n');
    }

    // ── Trivia lookups (owned copies, to avoid borrowing `self` while mutating) ──

    fn leading_texts(&self, span: &Span) -> Vec<String> {
        self.trivia
            .get(span)
            .map(|t| t.leading.iter().map(|c| c.text.clone()).collect())
            .unwrap_or_default()
    }

    /// The end-of-line comment to append after the node, if any.
    fn eol_text(&self, span: &Span) -> Option<String> {
        self.trivia.get(span).and_then(|t| {
            t.trailing.iter().find(|c| c.kind == CommentKind::EndOfLine).map(|c| c.text.clone())
        })
    }

    /// Standalone comments that trail the node on their own lines (e.g. a dangling
    /// comment at end of file with nothing following it).
    fn trailing_standalone_texts(&self, span: &Span) -> Vec<String> {
        self.trivia
            .get(span)
            .map(|t| {
                t.trailing
                    .iter()
                    .filter(|c| c.kind == CommentKind::Standalone)
                    .map(|c| c.text.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn emit_leading(&mut self, span: &Span) {
        for text in self.leading_texts(span) {
            self.push_line(&text);
        }
    }

    fn emit_trailing_standalone(&mut self, span: &Span) {
        for text in self.trailing_standalone_texts(span) {
            self.push_line(&text);
        }
    }

    /// Emit a single-line node: leading comments, then `content` with any end-of-line
    /// comment appended, then trailing standalone comments.
    fn node_line(&mut self, span: &Span, content: &str) {
        self.emit_leading(span);
        let line = match self.eol_text(span) {
            Some(comment) => format!("{content} {comment}"),
            None => content.to_string(),
        };
        self.push_line(&line);
        self.emit_trailing_standalone(span);
    }

    /// Emit the closing `}` of a block, carrying any end-of-line and standalone
    /// trailing comments that attached to the block node.
    fn close_block(&mut self, span: &Span) {
        let line = match self.eol_text(span) {
            Some(comment) => format!("}} {comment}"),
            None => "}".to_string(),
        };
        self.push_line(&line);
        self.emit_trailing_standalone(span);
    }

    // ── Program & items ──────────────────────────────────────────────────────────

    fn program(&mut self, program: &Program) {
        for (i, item) in program.items.iter().enumerate() {
            // Preserve blank-line grouping between top-level items: a recorded gap
            // collapses to exactly one blank line.
            if i > 0 && self.blank_before(item.span()) {
                self.blank();
            }
            self.item(item);
        }
    }

    fn blank_before(&self, span: &Span) -> bool {
        self.trivia.get(span).map(|t| t.blank_lines_before > 0).unwrap_or(false)
    }

    fn item(&mut self, item: &Item) {
        match item {
            Item::Import(d) => {
                let content = format!("import \"{}\" as {};", d.module, d.alias);
                self.node_line(&d.span, &content);
            }
            Item::Let(d) => {
                let content = format!("let {} = {};", d.name, render_expr(&d.value));
                self.node_line(&d.span, &content);
            }
            Item::Project(p) => self.project(p),
            Item::Stage(s) => self.stage(s),
            Item::Pipeline(p) => self.pipeline(p),
        }
    }

    fn project(&mut self, p: &ProjectBlock) {
        self.emit_leading(&p.span);
        if p.fields.is_empty() {
            self.push_line("project {}");
            self.emit_trailing_standalone(&p.span);
            return;
        }
        self.push_line("project {");
        self.indent += 1;
        for field in &p.fields {
            let content = format!("{}: {}", field.name, render_expr(&field.value));
            self.node_line(&field.span, &content);
        }
        self.indent -= 1;
        self.close_block(&p.span);
    }

    fn stage(&mut self, s: &StageBlock) {
        self.emit_leading(&s.span);
        self.push_line(&format!("stage {} {{", s.name));
        self.indent += 1;

        let mut wrote = false;
        // The description documents the stage, so it leads the block.
        if let Some(desc) = &s.description {
            self.push_line(&format!("description: \"{desc}\""));
            wrote = true;
        }
        // `matrix` defines the stage's variants, so it leads the block.
        if !s.matrix.is_empty() {
            self.matrix_block(&s.matrix);
            wrote = true;
        }
        if let Some(inputs) = &s.inputs {
            self.push_line(&format!("inputs: {}", render_expr(inputs)));
            wrote = true;
        }
        if let Some(outputs) = &s.outputs {
            self.push_line(&format!("outputs: {}", render_expr(outputs)));
            wrote = true;
        }
        if !s.depends_on.is_empty() {
            let names = s.depends_on.iter().map(|d| d.name.as_str()).collect::<Vec<_>>().join(", ");
            self.push_line(&format!("depends_on: [{names}]"));
            wrote = true;
        }
        if s.allow_failure {
            self.push_line("allow_failure: true");
            wrote = true;
        }
        if s.always_run {
            self.push_line("always_run: true");
            wrote = true;
        }
        if s.run_once {
            self.push_line("run_once: true");
            wrote = true;
        }
        if s.test {
            self.push_line("test: true");
            wrote = true;
        }
        if !s.steps.is_empty() {
            if wrote {
                self.blank();
            }
            self.step_block("steps", &s.steps);
            wrote = true;
        }
        if !s.on_failure.is_empty() {
            if wrote {
                self.blank();
            }
            self.step_block("on_failure", &s.on_failure);
        }

        self.indent -= 1;
        self.close_block(&s.span);
    }

    fn matrix_block(&mut self, dims: &[MatrixDim]) {
        self.push_line("matrix {");
        self.indent += 1;
        for dim in dims {
            let values = dim
                .values
                .iter()
                .map(|v| format!("\"{}\"", v.value))
                .collect::<Vec<_>>()
                .join(", ");
            self.push_line(&format!("{}: [{}]", dim.name, values));
        }
        self.indent -= 1;
        self.push_line("}");
    }

    fn pipeline(&mut self, p: &PipelineBlock) {
        self.emit_leading(&p.span);
        let keyword = if p.is_default { "default pipeline" } else { "pipeline" };
        self.push_line(&format!("{keyword} {} {{", p.name));
        self.indent += 1;

        let mut wrote = false;
        if let Some(input) = &p.input {
            self.push_line(&format!("input: {}", render_expr(input)));
            wrote = true;
        }
        if let Some(stages) = &p.stages {
            self.push_line(&format!("stages: {}", render_expr(stages)));
            wrote = true;
        }
        if !p.on_failure.is_empty() {
            if wrote {
                self.blank();
            }
            self.step_block("on_failure", &p.on_failure);
            wrote = true;
        }
        if !p.on_success.is_empty() {
            if wrote {
                self.blank();
            }
            self.step_block("on_success", &p.on_success);
        }

        self.indent -= 1;
        self.close_block(&p.span);
    }

    // ── Steps ────────────────────────────────────────────────────────────────────

    fn step_block(&mut self, keyword: &str, steps: &[Step]) {
        self.push_line(&format!("{keyword} {{"));
        self.indent += 1;
        for step in steps {
            self.step(step);
        }
        self.indent -= 1;
        self.push_line("}");
    }

    fn step(&mut self, step: &Step) {
        match step {
            Step::Exec(s) => {
                let content =
                    if s.command.is_empty() { "$".to_string() } else { format!("$ {}", s.command) };
                self.node_line(&s.span, &content);
            }
            Step::Copy(s) => {
                let content = format!("copy {} to {}", render_expr(&s.src), render_expr(&s.dest));
                self.node_line(&s.span, &content);
            }
            Step::Move(s) => {
                let content = format!("move {} to {}", render_expr(&s.src), render_expr(&s.dest));
                self.node_line(&s.span, &content);
            }
            Step::Mkdir(s) => {
                self.node_line(&s.span, &format!("mkdir {}", render_expr(&s.path)));
            }
            Step::Delete(s) => {
                self.node_line(&s.span, &format!("delete {}", render_expr(&s.path)));
            }
            Step::Write(s) => {
                let content = format!(
                    "write {} content: {}",
                    render_expr(&s.path),
                    render_string(&s.content)
                );
                self.node_line(&s.span, &content);
            }
            Step::If(s) => self.if_step(s),
            Step::For(s) => self.for_step(s),
            Step::Try(s) => self.try_step(s),
            Step::Workdir(s) => self.workdir_step(s),
            Step::WithEnv(s) => self.with_env_step(s),
            Step::Expect(s) => self.node_line(&s.span, &render_expect(s)),
            Step::Assert(s) => {
                let content = format!(
                    "assert {} {} {}",
                    render_expr(&s.actual),
                    render_match_op(s.op),
                    render_string(&s.expected)
                );
                self.node_line(&s.span, &content);
            }
            Step::Log(s) => {
                self.node_line(&s.span, &format!("log {}", render_string(&s.message)));
            }
            Step::Fail(s) => {
                self.node_line(&s.span, &format!("fail {}", render_string(&s.reason)));
            }
            Step::Let(s) => {
                self.node_line(&s.span, &format!("let {} = {};", s.name, render_expr(&s.value)));
            }
        }
    }

    fn if_step(&mut self, s: &IfStep) {
        self.emit_leading(&s.span);
        self.push_line(&format!("if {} {{", render_cond(&s.condition)));
        self.indent += 1;
        for step in &s.then_steps {
            self.step(step);
        }
        self.indent -= 1;
        if s.else_steps.is_empty() {
            self.close_block(&s.span);
        } else {
            self.push_line("} else {");
            self.indent += 1;
            for step in &s.else_steps {
                self.step(step);
            }
            self.indent -= 1;
            self.close_block(&s.span);
        }
    }

    fn for_step(&mut self, s: &ForStep) {
        self.emit_leading(&s.span);
        self.push_line(&format!("for {} in {} {{", s.var, render_expr(&s.iterable)));
        self.indent += 1;
        for step in &s.steps {
            self.step(step);
        }
        self.indent -= 1;
        self.close_block(&s.span);
    }

    fn try_step(&mut self, s: &TryStep) {
        self.emit_leading(&s.span);
        self.push_line("try {");
        self.indent += 1;
        for step in &s.steps {
            self.step(step);
        }
        self.indent -= 1;
        self.close_block(&s.span);
    }

    fn workdir_step(&mut self, s: &WorkdirStep) {
        self.emit_leading(&s.span);
        self.push_line(&format!("workdir {} {{", render_expr(&s.path)));
        self.indent += 1;
        for step in &s.steps {
            self.step(step);
        }
        self.indent -= 1;
        self.close_block(&s.span);
    }

    fn with_env_step(&mut self, s: &WithEnvStep) {
        self.emit_leading(&s.span);
        // Render the env map inline: `with_env { KEY: <value>, … } {`.
        let bindings = s
            .vars
            .iter()
            .map(|b| format!("{}: {}", b.key, render_expr(&b.value)))
            .collect::<Vec<_>>()
            .join(", ");
        self.push_line(&format!("with_env {{ {bindings} }} {{"));
        self.indent += 1;
        for step in &s.steps {
            self.step(step);
        }
        self.indent -= 1;
        self.close_block(&s.span);
    }
}

// ── Test-harness step rendering ──────────────────────────────────────────────────

fn render_expect(s: &ExpectStep) -> String {
    let mut out = String::from("expect ");
    match &s.check {
        ExpectCheck::Ok => out.push_str("ok"),
        ExpectCheck::Fails => out.push_str("fails"),
        ExpectCheck::Output { op, expected } => {
            out.push_str("output ");
            out.push_str(render_match_op(*op));
            out.push(' ');
            out.push_str(&render_string(expected));
        }
    }
    if let Some(t) = s.timeout_secs {
        out.push_str(&format!(" timeout {t}"));
    }
    if s.command.is_empty() {
        out.push_str(" $");
    } else {
        out.push_str(" $ ");
        out.push_str(&s.command);
    }
    out
}

fn render_match_op(op: MatchOp) -> &'static str {
    match op {
        MatchOp::Contains => "contains",
        MatchOp::Equals => "equals",
        MatchOp::NotContains => "not_contains",
        MatchOp::StartsWith => "starts_with",
        MatchOp::EndsWith => "ends_with",
        MatchOp::Matches => "matches",
    }
}

// ── Expression rendering (pure, single-line) ────────────────────────────────────

fn render_expr(expr: &Expr) -> String {
    match expr {
        Expr::String(s) => render_string(s),
        Expr::Int(i) => i.value.to_string(),
        Expr::Bool(b) => b.value.to_string(),
        Expr::List(l) => {
            let items: Vec<String> = l.items.iter().map(render_expr).collect();
            format!("[{}]", items.join(", "))
        }
        Expr::Glob(g) => {
            let patterns: Vec<String> = g.patterns.iter().map(|p| format!("\"{p}\"")).collect();
            format!("glob({})", patterns.join(", "))
        }
        Expr::If(e) => format!(
            "if {} {{ {} }} else {{ {} }}",
            render_cond(&e.condition),
            render_expr(&e.then_expr),
            render_expr(&e.else_expr),
        ),
        Expr::ModuleCall(c) => {
            format!("{}.{}({})", c.module, c.method, render_args(&c.args))
        }
        Expr::StageRef(r) => format!("{}.outputs", r.stage),
        Expr::MemberAccess(m) => format!("{}.{}", m.object, m.field),
        Expr::Ident(i) => i.name.clone(),
    }
}

fn render_args(args: &[CallArg]) -> String {
    let parts: Vec<String> = args
        .iter()
        .map(|arg| match &arg.name {
            Some(name) => format!("{name}: {}", render_expr(&arg.value)),
            None => render_expr(&arg.value),
        })
        .collect();
    parts.join(", ")
}

fn render_string(s: &StringExpr) -> String {
    let mut out = String::from("\"");
    for part in &s.parts {
        match part {
            StringPart::Literal(text) => out.push_str(text),
            StringPart::Interpolation(expr) => {
                out.push_str("${");
                out.push_str(&render_expr(expr));
                out.push('}');
            }
        }
    }
    out.push('"');
    out
}

// ── Condition rendering (with minimal precedence parentheses) ────────────────────

/// Precedence levels: `or` binds loosest, then `and`, then `!`/primaries. A child is
/// parenthesized only when its operator binds looser than the context requires.
const PREC_OR: u8 = 1;
const PREC_AND: u8 = 2;
const PREC_UNARY: u8 = 3;

fn render_cond(cond: &Condition) -> String {
    render_cond_prec(cond, PREC_OR)
}

fn render_cond_prec(cond: &Condition, parent: u8) -> String {
    match cond {
        Condition::Env(c) => render_env_cond(c),
        Condition::Platform(c) => {
            format!("platform {} {}", render_compare_op(&c.op), render_platform(&c.value))
        }
        // Comparisons and `empty(...)` are primaries: they bind tighter than `and`/`or`
        // and never need precedence parentheses relative to the parent.
        Condition::Compare(c) => {
            format!("{} {} {}", render_expr(&c.lhs), render_cond_op(&c.op), render_expr(&c.rhs))
        }
        Condition::Empty(c) => format!("empty({})", render_expr(&c.expr)),
        Condition::Not(inner, _) => format!("!{}", render_cond_prec(inner, PREC_UNARY)),
        Condition::And(a, b, _) => {
            let text =
                format!("{} and {}", render_cond_prec(a, PREC_AND), render_cond_prec(b, PREC_AND));
            parenthesize(text, PREC_AND, parent)
        }
        Condition::Or(a, b, _) => {
            let text =
                format!("{} or {}", render_cond_prec(a, PREC_OR), render_cond_prec(b, PREC_OR));
            parenthesize(text, PREC_OR, parent)
        }
    }
}

fn parenthesize(text: String, own: u8, parent: u8) -> String {
    if own < parent { format!("({text})") } else { text }
}

fn render_env_cond(c: &EnvCondition) -> String {
    match &c.comparison {
        Some((op, value)) => {
            format!("env(\"{}\") {} \"{}\"", c.var, render_compare_op(op), value)
        }
        None => format!("env(\"{}\")", c.var),
    }
}

fn render_compare_op(op: &CompareOp) -> &'static str {
    match op {
        CompareOp::Eq => "==",
        CompareOp::Ne => "!=",
    }
}

fn render_cond_op(op: &CondOp) -> &'static str {
    match op {
        CondOp::Eq => "==",
        CondOp::Ne => "!=",
        CondOp::Contains => "contains",
        CondOp::In => "in",
    }
}

fn render_platform(p: &Platform) -> &'static str {
    match p {
        Platform::Windows => "\"windows\"",
        Platform::Linux => "\"linux\"",
        Platform::MacOs => "\"macos\"",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(src: &str) -> String {
        format(&Source::from_str("test.ms", src)).expect("should format")
    }

    /// Formatting must be idempotent: a second pass changes nothing.
    fn assert_idempotent(src: &str) {
        let once = fmt(src);
        let twice = format(&Source::from_str("test.ms", &once)).expect("should re-format");
        assert_eq!(once, twice, "formatting is not idempotent for {src:?}");
    }

    #[test]
    fn formats_import_and_let() {
        let out = fmt("import   \"git\"   as   git ;\nlet  x   =   1 ;");
        assert_eq!(out, "import \"git\" as git;\nlet x = 1;\n");
    }

    #[test]
    fn collapses_blank_groups_between_items() {
        let out = fmt("let a = 1;\n\n\n\nlet b = 2;");
        assert_eq!(out, "let a = 1;\n\nlet b = 2;\n");
    }

    #[test]
    fn adjacent_items_stay_adjacent() {
        let out = fmt("import \"a\" as a;\nimport \"b\" as b;");
        assert_eq!(out, "import \"a\" as a;\nimport \"b\" as b;\n");
    }

    #[test]
    fn formats_project_block() {
        let out = fmt("project{name:\"app\"\nversion:\"1.0\",}");
        assert_eq!(out, "project {\n    name: \"app\"\n    version: \"1.0\"\n}\n");
    }

    #[test]
    fn formats_stage_with_steps_and_failure() {
        let src = "stage build{inputs:src outputs:[\"out\"] steps{$ make\nmkdir \"d\"} on_failure{delete \"d\"}}";
        let out = fmt(src);
        let expected = "stage build {\n    inputs: src\n    outputs: [\"out\"]\n\n    steps {\n        $ make\n        mkdir \"d\"\n    }\n\n    on_failure {\n        delete \"d\"\n    }\n}\n";
        assert_eq!(out, expected);
    }

    #[test]
    fn formats_stage_depends_on() {
        let src = "stage build{inputs:src depends_on:[a,b,] steps{$ make\n}}";
        let out = fmt(src);
        let expected = "stage build {\n    inputs: src\n    depends_on: [a, b]\n\n    steps {\n        $ make\n    }\n}\n";
        assert_eq!(out, expected);
    }

    #[test]
    fn formats_try_step() {
        let src = "stage s{steps{try{$ apt-get update\nmkdir \"x\"}}}";
        let expected = "stage s {\n    steps {\n        try {\n            $ apt-get update\n            mkdir \"x\"\n        }\n    }\n}\n";
        assert_eq!(fmt(src), expected);
    }

    #[test]
    fn formats_stage_caching_knobs() {
        let src = "stage act{always_run:true steps{$ run\n}}";
        assert_eq!(
            fmt(src),
            "stage act {\n    always_run: true\n\n    steps {\n        $ run\n    }\n}\n"
        );
        let src = "stage setup{run_once:true steps{$ install\n}}";
        assert_eq!(
            fmt(src),
            "stage setup {\n    run_once: true\n\n    steps {\n        $ install\n    }\n}\n"
        );
    }

    #[test]
    fn formats_stage_matrix_block() {
        let src = "stage b{matrix{arch:[\"x64\",\"arm64\"]}inputs:src steps{$ go\n}}";
        let expected = "stage b {\n    matrix {\n        arch: [\"x64\", \"arm64\"]\n    }\n    inputs: src\n\n    steps {\n        $ go\n    }\n}\n";
        assert_eq!(fmt(src), expected);
        assert_idempotent(src);
    }

    #[test]
    fn formats_stage_description() {
        let src = "stage build{steps{$ make\n}description:\"Compile the app\"}";
        let expected = "stage build {\n    description: \"Compile the app\"\n\n    steps {\n        $ make\n    }\n}\n";
        assert_eq!(fmt(src), expected);
        assert_idempotent(src);
    }

    #[test]
    fn formats_test_stage_and_assertions() {
        let src = "stage unit{test:true steps{assert \"${x}\"  equals  \"y\"\nexpect ok $ make test\nexpect output contains \"OK\" timeout 5 $ run\n}}";
        let expected = "stage unit {\n    test: true\n\n    steps {\n        assert \"${x}\" equals \"y\"\n        expect ok $ make test\n        expect output contains \"OK\" timeout 5 $ run\n    }\n}\n";
        assert_eq!(fmt(src), expected);
        assert_idempotent(src);
    }

    #[test]
    fn formats_log_and_fail_steps() {
        let src = "stage s{steps{log \"building ${project.name}\"\nfail  \"nope\"}}";
        let expected = "stage s {\n    steps {\n        log \"building ${project.name}\"\n        fail \"nope\"\n    }\n}\n";
        assert_eq!(fmt(src), expected);
        assert_idempotent(src);
    }

    #[test]
    fn formats_block_scoped_local_let() {
        let src =
            "stage s{steps{let   x  =  \"${project.name}-rc\";\nwrite \"o\" content: \"${x}\"}}";
        let expected = "stage s {\n    steps {\n        let x = \"${project.name}-rc\";\n        write \"o\" content: \"${x}\"\n    }\n}\n";
        assert_eq!(fmt(src), expected);
        assert_idempotent(src);
    }

    #[test]
    fn formats_default_pipeline_and_lists() {
        let out = fmt("default pipeline dev{stages:[a,b,c]}");
        assert_eq!(out, "default pipeline dev {\n    stages: [a, b, c]\n}\n");
    }

    #[test]
    fn formats_if_step_with_condition_precedence() {
        let src = "stage s {\n steps {\n if platform==\"linux\" and (env(\"CI\") or env(\"DEV\")) {\n mkdir \"a\"\n } else {\n mkdir \"b\"\n }\n }\n}";
        let out = fmt(src);
        assert!(out.contains("if platform == \"linux\" and (env(\"CI\") or env(\"DEV\")) {"));
        assert!(out.contains("} else {"));
        assert_idempotent(src);
    }

    #[test]
    fn formats_if_expression_inline() {
        let out = fmt("let t = if platform == \"windows\" { \"w\" } else { \"u\" };");
        assert_eq!(out, "let t = if platform == \"windows\" { \"w\" } else { \"u\" };\n");
    }

    #[test]
    fn formats_general_comparison_conditions() {
        // `==`, `in`, `contains`, and `empty(...)` round-trip with canonical spacing.
        let out = fmt("let a=\"x\";let b=if a==\"x\"{\"y\"}else{\"n\"};");
        assert!(out.contains("if a == \"x\" {"), "got: {out}");

        let out = fmt("let b = if \"a\" in [\"a\",\"b\"] { \"y\" } else { \"n\" };");
        assert_eq!(out, "let b = if \"a\" in [\"a\", \"b\"] { \"y\" } else { \"n\" };\n");

        let out = fmt("let b = if \"rc\" contains \"r\" { \"y\" } else { \"n\" };");
        assert_eq!(out, "let b = if \"rc\" contains \"r\" { \"y\" } else { \"n\" };\n");

        let src = "let s = glob(\"x/*\");\nlet b = if !empty(s) { \"y\" } else { \"n\" };\n";
        let out = fmt(src);
        assert!(out.contains("if !empty(s) {"), "got: {out}");
        assert_idempotent(src);
    }

    #[test]
    fn formats_workdir_and_with_env_steps() {
        let src = "stage s {\n steps {\n workdir \"build\" {\n $ make\n write \"o\" content: \"x\"\n }\n with_env{RUSTFLAGS:\"-Dwarnings\",CC:\"clang\"}{\n $ cargo build\n }\n }\n}";
        let out = fmt(src);
        assert!(out.contains("        workdir \"build\" {"), "got: {out}");
        assert!(
            out.contains("        with_env { RUSTFLAGS: \"-Dwarnings\", CC: \"clang\" } {"),
            "got: {out}"
        );
        assert_idempotent(src);
    }

    #[test]
    fn preserves_leading_and_trailing_comments() {
        let src = "// header\nlet x = 1; // inline\n";
        let out = fmt(src);
        assert_eq!(out, "// header\nlet x = 1; // inline\n");
    }

    #[test]
    fn preserves_comment_inside_steps() {
        let src = "stage s {\n  steps {\n    // do it\n    delete \"x\"\n  }\n}\n";
        let out = fmt(src);
        assert!(out.contains("        // do it\n        delete \"x\""));
        assert_idempotent(src);
    }

    #[test]
    fn module_call_with_named_args() {
        let out = fmt("let v = git.tag( default : \"0.0.0\" );");
        assert_eq!(out, "let v = git.tag(default: \"0.0.0\");\n");
    }

    #[test]
    fn idempotent_on_complex_script() {
        let src = "import \"env\" as env;\nlet sources = glob(\"src/**/*.rs\", \"a/*\");\nstage c {\n  inputs: sources\n  steps {\n    for f in sources {\n      $ echo ${f.path}\n    }\n  }\n}\n";
        assert_idempotent(src);
    }
}
