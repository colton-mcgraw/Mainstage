//! Navigation: go-to-definition, find-references, and the document outline.
//!
//! All three reuse the name-based resolution rules the analyzer applies — a bare
//! identifier resolves to a `let` binding or stage name, `<stage>.outputs` to its
//! declaring stage, and an alias to its `import` — so editor navigation tracks
//! command-line semantics. References are gathered by walking the parsed
//! [`Program`] once and collecting every identifier and `<stage>.outputs` site.

use mainstage_core::ast::{Condition, Expr, Item, Program, Step, StringPart};
use mainstage_core::Span;
use tower_lsp::lsp_types::{Position, Range};

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
    let index = DocumentIndex::from_program(program);
    let target = target_at(text, pos, &index)?;
    decl_range(text, &index, &target)
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

/// The document outline: top-level `let` bindings, stages, and pipelines in
/// source order.
pub fn document_symbols(text: &str, program: Option<&Program>) -> Vec<Symbol> {
    let Some(program) = program else { return Vec::new() };
    let mut symbols = Vec::new();
    for item in &program.items {
        let (name, kind, span) = match item {
            Item::Let(d) => (d.name.clone(), SymbolKind::Let, &d.span),
            Item::Stage(s) => (s.name.clone(), SymbolKind::Stage, &s.span),
            Item::Pipeline(p) => (p.name.clone(), SymbolKind::Pipeline, &p.span),
            Item::Import(_) | Item::Project(_) => continue,
        };
        let full = span_to_range(text, span);
        let selection = name_range(text, span, &name).unwrap_or(full);
        symbols.push(Symbol { name, kind, range: full, selection });
    }
    symbols
}

/// A single entry in the document outline.
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    /// The full source range of the declaration.
    pub range: Range,
    /// The range of just the declared name, for the editor to highlight.
    pub selection: Range,
}

/// The category of an outline [`Symbol`].
pub enum SymbolKind {
    Let,
    Stage,
    Pipeline,
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
    if index.lets.iter().any(|l| l.name == word) {
        return Some(Target::Let(word.to_string()));
    }
    if index.module_for_alias(word).is_some() {
        return Some(Target::Alias(word.to_string()));
    }
    None
}

/// The declaration name range for `target`, searched within its declaring item.
fn decl_range(text: &str, index: &DocumentIndex, target: &Target) -> Option<Range> {
    let (span, name) = match target {
        Target::Let(name) => (&index.lets.iter().find(|l| &l.name == name)?.span, name),
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
            Item::Import(_) => {}
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

fn walk_condition(_condition: &Condition, _occ: &mut [Occurrence]) {
    // Conditions reference only `env` variables and the `platform` built-in,
    // neither of which is a navigable `let`/stage symbol.
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
    fn no_definition_without_a_program() {
        assert!(definition("let x = 1;", Position::new(0, 4), None).is_none());
    }
}
