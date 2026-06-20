//! Navigation: go-to-definition, find-references, and the document outline.
//!
//! All three reuse the name-based resolution rules the analyzer applies — a bare
//! identifier resolves to a `let` binding or stage name, `<stage>.outputs` to its
//! declaring stage, and an alias to its `import` — so editor navigation tracks
//! command-line semantics. References are gathered by walking the parsed
//! [`Program`] once and collecting every identifier and `<stage>.outputs` site.

use mainstage_core::Span;
use mainstage_core::ast::{Condition, ExpectCheck, Expr, Item, Program, Step, StringPart};
use tower_lsp::lsp_types::{DocumentHighlight, DocumentHighlightKind, Position, Range};

use crate::cursor::{ident_at, offset_at, position_at, receiver_before, span_offsets};
use crate::index::DocumentIndex;

/// What an identifier under the cursor resolves to. Aliases participate in
/// go-to-definition; only `let` bindings and stages support find-references.
#[derive(Clone, PartialEq)]
enum Target {
    Let(String),
    Stage(String),
    Alias(String),
}

/// The declaration site an identifier at `pos` points to, as a range in the same
/// document, or `None` when the cursor is not on a navigable reference.
pub fn definition(text: &str, pos: Position, program: Option<&Program>) -> Option<Range> {
    let program = program?;
    // A block-scoped local `let` (Phase 44) is resolved first: it is scope-sensitive and not
    // in the top-level index, so jumping to it takes precedence over the index lookup.
    if let Some(range) = local_let_definition(text, pos, program) {
        return Some(range);
    }
    let index = DocumentIndex::from_program(program);
    // A `use <template>;` step navigates to the template's declaration (Phase 46). Resolved
    // before the general index lookup and only in `use` argument position, so a bare
    // identifier that merely shares a template's name elsewhere is unaffected.
    if let Some(range) = use_definition(text, pos, &index) {
        return Some(range);
    }
    let target = target_at(text, pos, &index)?;
    decl_range(text, &index, &target)
}

/// When the cursor at `pos` lands inside an `include "<path>";` item, returns that raw
/// include path (as written, relative to the including file). The server resolves it
/// against the document's own directory to produce a cross-file go-to-definition target —
/// jumping into the included file. `None` when the cursor is not on an include.
pub fn include_target(text: &str, pos: Position, program: Option<&Program>) -> Option<String> {
    let program = program?;
    let offset = offset_at(text, pos);
    program.items.iter().find_map(|item| {
        let Item::Include(inc) = item else { return None };
        let (start, end) = span_offsets(text, &inc.span);
        (start <= offset && offset < end).then(|| inc.path.clone())
    })
}

/// Resolve the cursor to the `template` declaration named by a `use <template>;` step, or
/// `None` when the cursor is not on a `use` argument that names a declared template.
fn use_definition(text: &str, pos: Position, index: &DocumentIndex) -> Option<Range> {
    let offset = offset_at(text, pos);
    let (start, end) = ident_at(text, offset)?;
    let word = &text[start..end];
    if !preceded_by_keyword(text, start, "use") {
        return None;
    }
    let info = index.templates.iter().find(|t| t.name == word)?;
    name_range(text, &info.span, &info.name)
}

/// Whether the token immediately before byte offset `at` (skipping inline whitespace) is the
/// whole keyword `kw` — used to recognize a `use` argument without a full re-parse.
fn preceded_by_keyword(text: &str, at: usize, kw: &str) -> bool {
    let before = text[..at].trim_end_matches([' ', '\t']);
    let Some(stripped) = before.strip_suffix(kw) else { return false };
    stripped.chars().next_back().is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
}

/// A block-scoped local `let` declaration and the byte offset at which its scope ends
/// (the end of its enclosing block), used to resolve go-to-definition for locals.
struct LocalDecl {
    name: String,
    decl_span: Span,
    scope_end: usize,
}

/// Resolve the cursor at `pos` to a block-scoped local `let` declaration, when it lands on a
/// local binding's name or a reference to one in scope. Returns the declaration's name range.
fn local_let_definition(text: &str, pos: Position, program: &Program) -> Option<Range> {
    let offset = offset_at(text, pos);
    let (start, end) = ident_at(text, offset)?;
    let word = &text[start..end];
    // A `<receiver>.<field>` access never denotes a bare local binding.
    if receiver_before(text, start).is_some() {
        return None;
    }

    let mut decls = Vec::new();
    for item in &program.items {
        match item {
            Item::Stage(s) => {
                let scope_end = span_offsets(text, &s.span).1;
                collect_local_decls(&s.steps, scope_end, text, &mut decls);
                collect_local_decls(&s.on_failure, scope_end, text, &mut decls);
            }
            Item::Pipeline(p) => {
                let scope_end = span_offsets(text, &p.span).1;
                collect_local_decls(&p.on_failure, scope_end, text, &mut decls);
                collect_local_decls(&p.on_success, scope_end, text, &mut decls);
            }
            _ => {}
        }
    }

    // Among the locals named `word` whose scope covers the cursor, the innermost (latest-
    // declared) one wins — mirroring how the evaluator's reverse lookup resolves the name.
    let best = decls
        .iter()
        .filter(|d| d.name == word)
        .filter(|d| {
            let decl_start = span_offsets(text, &d.decl_span).0;
            decl_start <= offset && offset <= d.scope_end
        })
        .max_by_key(|d| span_offsets(text, &d.decl_span).0)?;
    name_range(text, &best.decl_span, word)
}

/// Collect every block-scoped local `let` reachable from `steps`, recording each binding's
/// declaration span and the offset at which its scope ends (`container_end`). Nested blocks
/// narrow the scope to that block's span.
fn collect_local_decls(steps: &[Step], container_end: usize, text: &str, out: &mut Vec<LocalDecl>) {
    for step in steps {
        match step {
            Step::Let(l) => out.push(LocalDecl {
                name: l.name.clone(),
                decl_span: l.span.clone(),
                scope_end: container_end,
            }),
            Step::If(s) => {
                let end = span_offsets(text, &s.span).1;
                collect_local_decls(&s.then_steps, end, text, out);
                collect_local_decls(&s.else_steps, end, text, out);
            }
            Step::For(s) => collect_local_decls(&s.steps, span_offsets(text, &s.span).1, text, out),
            Step::Try(s) => collect_local_decls(&s.steps, span_offsets(text, &s.span).1, text, out),
            Step::Workdir(s) => {
                collect_local_decls(&s.steps, span_offsets(text, &s.span).1, text, out)
            }
            Step::WithEnv(s) => {
                collect_local_decls(&s.steps, span_offsets(text, &s.span).1, text, out)
            }
            _ => {}
        }
    }
}

/// Every occurrence of the symbol at `pos`, including its declaration when
/// `include_declaration` is set. Empty when the cursor is not on a `let` binding
/// or stage (the two symbols references are tracked for).
pub fn references(
    text: &str,
    pos: Position,
    program: Option<&Program>,
    include_declaration: bool,
) -> Vec<Range> {
    let Some(program) = program else { return Vec::new() };
    let index = DocumentIndex::from_program(program);
    let Some(target) = target_at(text, pos, &index) else { return Vec::new() };

    // References are only meaningful for declarations the document defines.
    if matches!(target, Target::Alias(_)) {
        return Vec::new();
    }

    let mut ranges: Vec<Range> = collect_occurrences(program)
        .into_iter()
        .filter(|occ| occ.matches(&target))
        .map(|occ| span_to_range(text, &occ.span))
        .collect();

    if include_declaration && let Some(decl) = decl_range(text, &index, &target) {
        ranges.push(decl);
    }

    dedup_ranges(&mut ranges);
    ranges
}

/// Every occurrence of the symbol at `pos`, each tagged with a highlight kind for
/// `textDocument/documentHighlight`: the declaration is a `WRITE`, every use a
/// `READ`. This is what lets an editor highlight a variable and all its uses when
/// the cursor lands on it. Empty when the cursor is not on a `let` binding or
/// stage (the symbols whose occurrences are tracked).
pub fn highlights(text: &str, pos: Position, program: Option<&Program>) -> Vec<DocumentHighlight> {
    let Some(program) = program else { return Vec::new() };
    let index = DocumentIndex::from_program(program);
    let Some(target) = target_at(text, pos, &index) else { return Vec::new() };

    // Highlights, like references, are only meaningful for document-defined symbols.
    if matches!(target, Target::Alias(_)) {
        return Vec::new();
    }

    let mut highlights: Vec<DocumentHighlight> = collect_occurrences(program)
        .into_iter()
        .filter(|occ| occ.matches(&target))
        .map(|occ| DocumentHighlight {
            range: span_to_range(text, &occ.span),
            kind: Some(DocumentHighlightKind::READ),
        })
        .collect();

    // The declaration site itself is a write.
    if let Some(decl) = decl_range(text, &index, &target) {
        highlights
            .push(DocumentHighlight { range: decl, kind: Some(DocumentHighlightKind::WRITE) });
    }

    dedup_highlights(&mut highlights);
    highlights
}

/// The document outline: top-level `let` bindings, stages, and pipelines in
/// source order.
pub fn document_symbols(text: &str, program: Option<&Program>) -> Vec<Symbol> {
    let Some(program) = program else { return Vec::new() };
    let mut symbols = Vec::new();
    for item in &program.items {
        let (name, kind, span, detail) = match item {
            Item::Let(d) => (d.name.clone(), SymbolKind::Let, &d.span, None),
            // A build parameter (Phase 49) appears in the outline with its type as detail.
            Item::Param(d) => {
                (d.name.clone(), SymbolKind::Param, &d.span, Some(d.ty.keyword().to_string()))
            }
            // A stage carries its description and ordering as the outline detail, so a
            // multi-stage build is navigable from the editor's symbol list.
            Item::Stage(s) => (s.name.clone(), SymbolKind::Stage, &s.span, stage_detail(s)),
            Item::Pipeline(p) => (p.name.clone(), SymbolKind::Pipeline, &p.span, None),
            // A reusable step template (Phase 46) appears in the outline so `use` targets
            // are discoverable.
            Item::Template(t) => (t.name.clone(), SymbolKind::Template, &t.span, None),
            // Imports, project blocks, and includes are not outline symbols.
            Item::Import(_) | Item::Project(_) | Item::Include(_) => continue,
        };
        let full = span_to_range(text, span);
        let selection = name_range(text, span, &name).unwrap_or(full);
        symbols.push(Symbol { name, kind, range: full, selection, detail });
    }
    symbols
}

/// The outline detail for a stage: its description and any `depends_on` ordering, e.g.
/// `"Compile the kernel · after build"`. `None` when it has neither.
fn stage_detail(stage: &mainstage_core::ast::StageBlock) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(desc) = &stage.description {
        parts.push(desc.clone());
    }
    if !stage.depends_on.is_empty() {
        let names = stage.depends_on.iter().map(|d| d.name.as_str()).collect::<Vec<_>>().join(", ");
        parts.push(format!("after {names}"));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// A single entry in the document outline.
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    /// The full source range of the declaration.
    pub range: Range,
    /// The range of just the declared name, for the editor to highlight.
    pub selection: Range,
    /// Optional detail shown beside the name (e.g. a stage's description and ordering).
    pub detail: Option<String>,
}

/// The category of an outline [`Symbol`].
pub enum SymbolKind {
    Let,
    Param,
    Stage,
    Pipeline,
    Template,
}

// ── Cursor resolution ──────────────────────────────────────────────────────────

/// Resolve the identifier under the cursor at `pos` to its [`Target`].
fn target_at(text: &str, pos: Position, index: &DocumentIndex) -> Option<Target> {
    let offset = offset_at(text, pos);
    let (start, end) = ident_at(text, offset)?;
    let word = &text[start..end];

    // `<stage>.outputs` — clicking either the stage name or `outputs` navigates
    // to the stage. The receiver carries the stage name when the word is `outputs`.
    if word == "outputs"
        && let Some(receiver) = receiver_before(text, start)
        && index.is_stage(&receiver)
    {
        return Some(Target::Stage(receiver));
    }

    // A member access like `project.name` or `file.path` is not a navigable
    // reference to a top-level symbol.
    if receiver_before(text, start).is_some() {
        return None;
    }

    if index.is_stage(word) {
        return Some(Target::Stage(word.to_string()));
    }
    // A `let` and a `param` (Phase 49) are both value bindings; a reference to either
    // resolves the same way, so they share the `Let` target.
    if index.lets.iter().any(|l| l.name == word) || index.params.iter().any(|p| p.name == word) {
        return Some(Target::Let(word.to_string()));
    }
    if index.module_for_alias(word).is_some() {
        return Some(Target::Alias(word.to_string()));
    }
    None
}

/// The declaration name range for `target`, searched within its declaring item.
fn decl_range(text: &str, index: &DocumentIndex, target: &Target) -> Option<Range> {
    let (span, name) =
        match target {
            Target::Let(name) => {
                let span =
                    index.lets.iter().find(|l| &l.name == name).map(|l| &l.span).or_else(|| {
                        index.params.iter().find(|p| &p.name == name).map(|p| &p.span)
                    })?;
                (span, name)
            }
            Target::Stage(name) => (&index.stages.iter().find(|s| &s.name == name)?.span, name),
            Target::Alias(name) => (&index.imports.iter().find(|i| &i.alias == name)?.span, name),
        };
    name_range(text, span, name)
}

// ── Reference collection ─────────────────────────────────────────────────────

/// One identifier site found while walking the program.
struct Occurrence {
    name: String,
    span: Span,
    /// True for a `<stage>.outputs` reference, which always denotes a stage.
    is_stage_ref: bool,
}

impl Occurrence {
    /// Whether this occurrence refers to `target`. A bare identifier matching a
    /// stage name is a stage reference (e.g. in a pipeline `stages:` list); a
    /// `<stage>.outputs` site never refers to a `let`.
    fn matches(&self, target: &Target) -> bool {
        match target {
            Target::Stage(name) => &self.name == name,
            Target::Let(name) => !self.is_stage_ref && &self.name == name,
            Target::Alias(_) => false,
        }
    }
}

/// Walk `program`, collecting every bare identifier and `<stage>.outputs`
/// reference. `let`/stage declarations themselves are not occurrences.
fn collect_occurrences(program: &Program) -> Vec<Occurrence> {
    let mut occ = Vec::new();
    for item in &program.items {
        match item {
            Item::Let(d) => walk_expr(&d.value, &mut occ),
            // A param's default may reference earlier bindings in interpolations.
            Item::Param(d) => walk_expr(&d.default, &mut occ),
            Item::Project(p) => {
                for field in &p.fields {
                    walk_expr(&field.value, &mut occ);
                }
            }
            Item::Stage(s) => {
                walk_opt(&s.inputs, &mut occ);
                walk_opt(&s.outputs, &mut occ);
                walk_steps(&s.steps, &mut occ);
                walk_steps(&s.on_failure, &mut occ);
            }
            Item::Pipeline(p) => {
                walk_opt(&p.input, &mut occ);
                walk_opt(&p.stages, &mut occ);
                walk_steps(&p.on_failure, &mut occ);
                walk_steps(&p.on_success, &mut occ);
            }
            // A template's steps may reference navigable symbols in interpolations.
            Item::Template(t) => walk_steps(&t.steps, &mut occ),
            Item::Import(_) | Item::Include(_) => {}
        }
    }
    occ
}

fn walk_opt(expr: &Option<Expr>, occ: &mut Vec<Occurrence>) {
    if let Some(expr) = expr {
        walk_expr(expr, occ);
    }
}

fn walk_expr(expr: &Expr, occ: &mut Vec<Occurrence>) {
    match expr {
        Expr::Ident(e) => {
            occ.push(Occurrence { name: e.name.clone(), span: e.span.clone(), is_stage_ref: false })
        }
        Expr::StageRef(e) => occ.push(Occurrence {
            // Narrow the span to just the stage name, dropping `.outputs`.
            name: e.stage.clone(),
            span: name_span(&e.span, &e.stage),
            is_stage_ref: true,
        }),
        Expr::String(e) => {
            for part in &e.parts {
                if let StringPart::Interpolation(inner) = part {
                    walk_expr(inner, occ);
                }
            }
        }
        Expr::List(e) => {
            for item in &e.items {
                walk_expr(item, occ);
            }
        }
        Expr::If(e) => {
            walk_condition(&e.condition, occ);
            walk_expr(&e.then_expr, occ);
            walk_expr(&e.else_expr, occ);
        }
        Expr::ModuleCall(e) => {
            for arg in &e.args {
                walk_expr(&arg.value, occ);
            }
        }
        // Member access objects (`project`, loop variables) and literals carry no
        // top-level symbol references.
        Expr::MemberAccess(_) | Expr::Glob(_) | Expr::Int(_) | Expr::Bool(_) => {}
    }
}

fn walk_condition(condition: &Condition, occ: &mut Vec<Occurrence>) {
    match condition {
        // `env` variables and the `platform` built-in are not navigable symbols.
        Condition::Env(_) | Condition::Platform(_) => {}
        Condition::Not(inner, _) => walk_condition(inner, occ),
        Condition::And(a, b, _) | Condition::Or(a, b, _) => {
            walk_condition(a, occ);
            walk_condition(b, occ);
        }
        // General comparisons (Phase 41) carry arbitrary operand expressions, which may
        // reference navigable `let` bindings, stages, or `project.<field>`.
        Condition::Compare(c) => {
            walk_expr(&c.lhs, occ);
            walk_expr(&c.rhs, occ);
        }
        Condition::Empty(c) => walk_expr(&c.expr, occ),
    }
}

fn walk_steps(steps: &[Step], occ: &mut Vec<Occurrence>) {
    for step in steps {
        match step {
            Step::Copy(s) => {
                walk_expr(&s.src, occ);
                walk_expr(&s.dest, occ);
            }
            Step::Move(s) => {
                walk_expr(&s.src, occ);
                walk_expr(&s.dest, occ);
            }
            Step::Mkdir(s) => walk_expr(&s.path, occ),
            Step::Delete(s) => walk_expr(&s.path, occ),
            Step::Write(s) => {
                walk_expr(&s.path, occ);
                for part in &s.content.parts {
                    if let StringPart::Interpolation(inner) = part {
                        walk_expr(inner, occ);
                    }
                }
            }
            Step::If(s) => {
                walk_condition(&s.condition, occ);
                walk_steps(&s.then_steps, occ);
                walk_steps(&s.else_steps, occ);
            }
            Step::For(s) => {
                walk_expr(&s.iterable, occ);
                walk_steps(&s.steps, occ);
            }
            Step::Try(s) => walk_steps(&s.steps, occ),
            Step::Workdir(s) => {
                walk_expr(&s.path, occ);
                walk_steps(&s.steps, occ);
            }
            Step::WithEnv(s) => {
                for binding in &s.vars {
                    walk_expr(&binding.value, occ);
                }
                walk_steps(&s.steps, occ);
            }
            Step::Assert(s) => {
                walk_expr(&s.actual, occ);
                for part in &s.expected.parts {
                    if let StringPart::Interpolation(inner) = part {
                        walk_expr(inner, occ);
                    }
                }
            }
            // `log` / `fail` carry an interpolated string whose `${…}` expressions may
            // reference navigable symbols.
            Step::Log(s) => {
                for part in &s.message.parts {
                    if let StringPart::Interpolation(inner) = part {
                        walk_expr(inner, occ);
                    }
                }
            }
            Step::Fail(s) => {
                for part in &s.reason.parts {
                    if let StringPart::Interpolation(inner) = part {
                        walk_expr(inner, occ);
                    }
                }
            }
            // A block-scoped `let`: its value expression may reference navigable symbols.
            Step::Let(s) => walk_expr(&s.value, occ),
            // `use <template>;` carries only a template name, resolved separately (it is not
            // a `let`/stage/alias reference, so it is not an occurrence here).
            Step::Use(_) => {}
            Step::Expect(s) => {
                // The `$` command keeps its argument as a raw string (not walked); only an
                // `output` check's expected value carries parsed `${…}` expressions.
                if let ExpectCheck::Output { expected, .. } = &s.check {
                    for part in &expected.parts {
                        if let StringPart::Interpolation(inner) = part {
                            walk_expr(inner, occ);
                        }
                    }
                }
            }
            // The `$` command keeps its argument as a raw string; its `${…}`
            // interpolations are not parsed into expression nodes.
            Step::Exec(_) => {}
        }
    }
}

// ── Span helpers ────────────────────────────────────────────────────────────

/// A sub-span covering just `name` at the start of `span` (stage names and
/// identifiers are single-line ASCII, so byte and char lengths agree).
fn name_span(span: &Span, name: &str) -> Span {
    Span {
        file: span.file.clone(),
        line_start: span.line_start,
        col_start: span.col_start,
        line_end: span.line_start,
        col_end: span.col_start + name.chars().count(),
    }
}

/// The range of the first whole-word occurrence of `name` within the source
/// covered by `span` — used to point at a declared name inside its block.
fn name_range(text: &str, span: &Span, name: &str) -> Option<Range> {
    let (start, end) = span_offsets(text, span);
    let rel = find_word(&text[start..end], name)?;
    let from = start + rel;
    Some(Range::new(position_at(text, from), position_at(text, from + name.len())))
}

/// Byte offset of the first whole-word match of `name` in `haystack`, requiring
/// non-identifier characters (or string ends) on both sides.
fn find_word(haystack: &str, name: &str) -> Option<usize> {
    let is_id = |c: char| c.is_alphanumeric() || c == '_';
    let mut from = 0;
    while let Some(rel) = haystack[from..].find(name) {
        let at = from + rel;
        let before_ok = haystack[..at].chars().next_back().is_none_or(|c| !is_id(c));
        let after_ok = haystack[at + name.len()..].chars().next().is_none_or(|c| !is_id(c));
        if before_ok && after_ok {
            return Some(at);
        }
        from = at + name.len();
    }
    None
}

/// Convert a core span to an LSP range against `text`.
fn span_to_range(text: &str, span: &Span) -> Range {
    crate::convert::span_to_range(text, span)
}

/// Remove duplicate ranges in place, preserving first-seen order. A bare stage
/// reference and a declaration search can land on the same span.
fn dedup_ranges(ranges: &mut Vec<Range>) {
    let mut seen = Vec::new();
    ranges.retain(|r| {
        if seen.contains(r) {
            false
        } else {
            seen.push(*r);
            true
        }
    });
}

/// Remove highlights that cover a range already seen, preserving first-seen order
/// (so a use's `READ` wins over a coincident declaration `WRITE`, which only
/// happens for the degenerate self-referential case).
fn dedup_highlights(highlights: &mut Vec<DocumentHighlight>) {
    let mut seen = Vec::new();
    highlights.retain(|h| {
        if seen.contains(&h.range) {
            false
        } else {
            seen.push(h.range);
            true
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::position_at;
    use mainstage_core::Source;

    fn parse(text: &str) -> Program {
        mainstage_core::parse(&Source::from_str("test.ms", text)).expect("parse")
    }

    /// Position at the first occurrence of `needle`.
    fn at(text: &str, needle: &str) -> Position {
        position_at(text, text.find(needle).expect("needle"))
    }

    /// Position at the last occurrence of `needle`.
    fn at_last(text: &str, needle: &str) -> Position {
        position_at(text, text.rfind(needle).expect("needle"))
    }

    /// A two-stage script where `pkg` consumes `build.outputs`.
    const STAGES: &str = "stage build {\n    outputs: [\"a\"]\n    steps {\n        $ echo hi\n    }\n}\nstage pkg {\n    inputs: [build.outputs]\n    steps {\n        $ echo hi\n    }\n}";

    #[test]
    fn definition_of_let_reference() {
        let text = "let name = \"demo\";\nlet other = name;";
        let program = parse(text);
        let def = definition(text, at_last(text, "name"), Some(&program)).expect("definition");
        // Points at the declared `name` on line 1.
        assert_eq!(def.start, Position::new(0, 4));
    }

    #[test]
    fn definition_of_param_reference() {
        // A reference to a `param` navigates to its declaration, exactly like a `let`.
        let text = "param target: string = \"release\";\nlet label = target;";
        let program = parse(text);
        let def = definition(text, at_last(text, "target"), Some(&program)).expect("definition");
        assert_eq!(def.start, at(text, "target"));
    }

    #[test]
    fn document_symbols_include_params() {
        let text = "param target: string = \"release\";\nlet name = \"demo\";";
        let program = parse(text);
        let symbols = document_symbols(text, Some(&program));
        let param = symbols.iter().find(|s| s.name == "target").expect("param symbol");
        assert!(matches!(param.kind, SymbolKind::Param));
        assert_eq!(param.detail.as_deref(), Some("string"));
    }

    #[test]
    fn definition_of_alias_reference() {
        let text = "import \"git\" as g;\nlet v = g.sha();";
        let program = parse(text);
        let def = definition(text, at_last(text, "g"), Some(&program)).expect("definition");
        // The alias `g` is declared after `as ` on line 1.
        assert_eq!(def.start.line, 0);
        assert_eq!(def.start, position_at(text, text.find("g;").unwrap()));
    }

    #[test]
    fn definition_of_stage_outputs_reference() {
        let program = parse(STAGES);
        // Click on `build` in `build.outputs`.
        let def = definition(STAGES, at_last(STAGES, "build"), Some(&program)).expect("definition");
        assert_eq!(def.start, at(STAGES, "build"));
    }

    #[test]
    fn definition_clicking_outputs_navigates_to_stage() {
        let program = parse(STAGES);
        // The `outputs:` field key of `build` itself has no navigable target.
        assert!(definition(STAGES, at(STAGES, "outputs:"), Some(&program)).is_none());
        // But the `outputs` in `build.outputs` resolves to the stage.
        let pos = position_at(STAGES, STAGES.find("build.outputs").unwrap() + "build.".len());
        let def = definition(STAGES, pos, Some(&program)).expect("definition");
        assert_eq!(def.start, at(STAGES, "build"));
    }

    #[test]
    fn references_to_a_let_finds_all_uses() {
        let text = "let name = \"demo\";\nlet a = name;\nlet b = name;";
        let program = parse(text);
        let refs = references(text, at(text, "name"), Some(&program), true);
        // Declaration + two uses.
        assert_eq!(refs.len(), 3);
    }

    #[test]
    fn references_exclude_declaration_when_requested() {
        let text = "let name = \"demo\";\nlet a = name;\nlet b = name;";
        let program = parse(text);
        let refs = references(text, at(text, "name"), Some(&program), false);
        assert_eq!(refs.len(), 2);
    }

    #[test]
    fn references_to_a_stage_covers_outputs_and_list_uses() {
        let text = format!("{STAGES}\npipeline ci {{\n    stages: [build, pkg]\n}}");
        let program = parse(&text);
        // Click the stage's declared name.
        let refs = references(&text, at(&text, "build"), Some(&program), false);
        // `build.outputs` reference and the `build` in the stages list.
        assert_eq!(refs.len(), 2);
        // The `build.outputs` reference highlights just the stage name.
        let outputs_ref = position_at(&text, text.find("build.outputs").unwrap());
        assert!(refs.iter().any(|r| r.start == outputs_ref));
    }

    #[test]
    fn references_on_alias_are_empty() {
        let text = "import \"git\" as g;\nlet v = g.sha();";
        let program = parse(text);
        assert!(references(text, at(text, "as g"), Some(&program), true).is_empty());
    }

    #[test]
    fn highlights_mark_declaration_as_write_and_uses_as_read() {
        let text = "let name = \"demo\";\nlet a = name;\nlet b = name;";
        let program = parse(text);
        // Cursor on a use of `name`.
        let hl = highlights(text, at_last(text, "name"), Some(&program));
        assert_eq!(hl.len(), 3, "declaration plus two uses");
        let writes = hl.iter().filter(|h| h.kind == Some(DocumentHighlightKind::WRITE)).count();
        let reads = hl.iter().filter(|h| h.kind == Some(DocumentHighlightKind::READ)).count();
        assert_eq!(writes, 1, "the declaration is the only write");
        assert_eq!(reads, 2, "both uses are reads");
        // The write lands on the declared `name`.
        let decl = at(text, "name");
        assert!(
            hl.iter()
                .any(|h| h.kind == Some(DocumentHighlightKind::WRITE) && h.range.start == decl)
        );
    }

    #[test]
    fn highlights_on_alias_or_literal_are_empty() {
        let text = "import \"git\" as g;\nlet v = g.sha();";
        let program = parse(text);
        // An import alias is navigable but has no tracked occurrences to highlight.
        assert!(highlights(text, at(text, "as g"), Some(&program)).is_empty());
        // A non-symbol position yields nothing.
        assert!(
            highlights("let x = 1;", Position::new(0, 8), Some(&parse("let x = 1;"))).is_empty()
        );
    }

    #[test]
    fn document_symbols_lists_lets_stages_and_pipelines() {
        let text = "import \"git\" as g;\nlet name = \"demo\";\n\
            stage build {\n    steps {\n        $ echo hi\n    }\n}\n\
            pipeline ci {\n    stages: [build]\n}";
        let program = parse(text);
        let symbols = document_symbols(text, Some(&program));
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["name", "build", "ci"]);
        // The `let` selection range covers just the name.
        let name_sym = &symbols[0];
        assert!(matches!(name_sym.kind, SymbolKind::Let));
        assert_eq!(name_sym.selection.start, at(text, "name"));
    }

    #[test]
    fn document_symbols_list_templates() {
        let text = "template setup {\n    $ checkout\n}\n\
            stage build {\n    steps {\n        use setup;\n    }\n}";
        let program = parse(text);
        let symbols = document_symbols(text, Some(&program));
        let tmpl = symbols.iter().find(|s| s.name == "setup").expect("template symbol");
        assert!(matches!(tmpl.kind, SymbolKind::Template));
        assert_eq!(tmpl.selection.start, at(text, "setup"), "selection covers the template name");
    }

    #[test]
    fn definition_of_a_use_jumps_to_the_template() {
        let text = "template setup {\n    $ checkout\n}\n\
            stage build {\n    steps {\n        use setup;\n    }\n}";
        let program = parse(text);
        // Cursor on the `setup` inside `use setup;` (the last occurrence).
        let def = definition(text, at_last(text, "setup"), Some(&program)).expect("definition");
        assert_eq!(def.start, at(text, "setup"), "jumps to the `template setup` declaration");
    }

    #[test]
    fn document_symbols_carry_stage_description_and_ordering() {
        let text = "stage setup {\n    steps {\n        $ echo hi\n    }\n}\n\
            stage build {\n    description: \"Compile it\"\n    depends_on: [setup]\n    steps {\n        $ echo hi\n    }\n}";
        let program = parse(text);
        let symbols = document_symbols(text, Some(&program));
        let build = symbols.iter().find(|s| s.name == "build").expect("build symbol");
        assert_eq!(build.detail.as_deref(), Some("Compile it · after setup"));
        let setup = symbols.iter().find(|s| s.name == "setup").expect("setup symbol");
        assert_eq!(setup.detail, None, "a stage with no description or ordering has no detail");
    }

    #[test]
    fn no_definition_without_a_program() {
        assert!(definition("let x = 1;", Position::new(0, 4), None).is_none());
    }

    #[test]
    fn include_target_resolves_when_cursor_is_on_an_include() {
        let text =
            "include \"components/build.ms\";\nstage s {\n    steps {\n        $ echo hi\n    }\n}";
        let program = parse(text);
        // Cursor inside the include path string.
        let pos = at(text, "components/build.ms");
        assert_eq!(
            include_target(text, pos, Some(&program)).as_deref(),
            Some("components/build.ms")
        );
        // Cursor on the stage name is not an include target.
        assert!(include_target(text, at(text, "stage"), Some(&program)).is_none());
    }

    #[test]
    fn definition_of_a_block_scoped_local_let() {
        let text = "stage s {\n    steps {\n        let obj = \"o\";\n        write obj content: \"x\"\n    }\n}";
        let program = parse(text);
        // Click on the `obj` reference in the `write` step → jumps to the local declaration.
        let def = definition(text, at_last(text, "obj"), Some(&program)).expect("definition");
        assert_eq!(def.start, at(text, "obj"), "jumps to the local `let` declaration");
    }

    #[test]
    fn local_let_out_of_scope_has_no_definition() {
        // A local declared inside an `if` block is not navigable from after the block.
        let text = "stage s {\n    steps {\n        if env(\"CI\") {\n            let inner = \"x\";\n        }\n        write inner content: \"y\"\n    }\n}";
        let program = parse(text);
        // The `inner` in the `write` after the `if` is out of scope: no local definition.
        assert!(definition(text, at_last(text, "inner"), Some(&program)).is_none());
    }
}
