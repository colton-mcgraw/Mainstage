//! Hover: show a module alias's binding, a method's signature and return type,
//! and the resolved form of `let` bindings, stage names, and `project.<field>`.

use mainstage_core::ast::Program;
use mainstage_core::{ModuleRegistry, Source, Span, TriviaMap, attach_trivia, lex};
use tower_lsp::lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind, Position, Range};

use crate::cursor::{ident_at, offset_at, position_at, receiver_before, slice_span};
use crate::index::{DocumentIndex, import_aliases};

/// Compute hover information for the cursor at `pos`, or `None` when there is
/// nothing to show.
pub fn hover(
    text: &str,
    pos: Position,
    registry: &ModuleRegistry,
    program: Option<&Program>,
) -> Option<Hover> {
    let (start, end) = ident_at(text, offset_at(text, pos))?;
    let word = &text[start..end];
    let aliases = import_aliases(text, program);
    let index = program.map(DocumentIndex::from_program);
    // Comments attached to declarations, so hover can surface a binding's doc.
    let trivia = program.map(|p| attach_trivia(p, &lex(&Source::from_str("hover.ms", text))));

    // A `<receiver>.` immediately before the word marks a member access.
    let receiver = receiver_before(text, start);

    let value = match receiver {
        Some(recv) => {
            member_hover(&recv, word, registry, &aliases, index.as_ref(), trivia.as_ref(), text)?
        }
        None => symbol_hover(word, registry, &aliases, index.as_ref(), trivia.as_ref(), text)?,
    };

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent { kind: MarkupKind::Markdown, value }),
        range: Some(Range::new(position_at(text, start), position_at(text, end))),
    })
}

/// Hover over `<receiver>.<word>`.
fn member_hover(
    receiver: &str,
    word: &str,
    registry: &ModuleRegistry,
    aliases: &std::collections::HashMap<String, String>,
    index: Option<&DocumentIndex>,
    trivia: Option<&TriviaMap>,
    text: &str,
) -> Option<String> {
    if let Some(module) = aliases.get(receiver) {
        let sig = registry.method_sig(module, word)?;
        return Some(code_block(&format!("{receiver}.{}", sig.signature())));
    }

    let index = index?;
    if receiver == "project" {
        let field = index.project_fields.iter().find(|f| f.name == word)?;
        return Some(with_docs(
            trivia,
            &field.span,
            code_block(&format!("project.{word} = {}", slice_span(text, &field.value_span))),
        ));
    }
    if let Some(stage) = index.stages.iter().find(|s| s.name == receiver)
        && word == "outputs"
    {
        let outputs =
            stage.outputs.as_ref().map(|sp| slice_span(text, sp)).unwrap_or("(none declared)");
        return Some(code_block(&format!("{receiver}.outputs = {outputs}")));
    }
    None
}

/// Hover over a bare identifier.
fn symbol_hover(
    word: &str,
    registry: &ModuleRegistry,
    aliases: &std::collections::HashMap<String, String>,
    index: Option<&DocumentIndex>,
    trivia: Option<&TriviaMap>,
    text: &str,
) -> Option<String> {
    if let Some(module) = aliases.get(word) {
        let methods = registry.get(module).map(|m| m.methods().len()).unwrap_or(0);
        return Some(format!(
            "{}\n\nmodule `{module}` imported as `{word}` — {methods} methods",
            code_block(&format!("import \"{module}\" as {word}"))
        ));
    }

    if let Some(index) = index {
        if let Some(binding) = index.lets.iter().find(|l| l.name == word) {
            return Some(with_docs(
                trivia,
                &binding.span,
                code_block(&format!("let {word} = {}", slice_span(text, &binding.value_span))),
            ));
        }
        // A build parameter (Phase 49) hovers as its declaration, showing the declared type
        // and default so the override-able knobs of a build are discoverable in the editor.
        if let Some(param) = index.params.iter().find(|p| p.name == word) {
            return Some(with_docs(
                trivia,
                &param.span,
                code_block(&format!(
                    "param {word}: {} = {}",
                    param.ty,
                    slice_span(text, &param.default_span)
                )),
            ));
        }
        if let Some(stage) = index.stages.iter().find(|s| s.name == word) {
            let after = if stage.depends_on.is_empty() {
                String::new()
            } else {
                format!(" (after {})", stage.depends_on.join(", "))
            };
            let outputs = stage
                .outputs
                .as_ref()
                .map(|sp| format!(" → outputs {}", slice_span(text, sp)))
                .unwrap_or_default();
            let code = code_block(&format!("stage {word}{after}{outputs}"));
            // Surface the stage's `description:` text alongside its signature.
            let body = match &stage.description {
                Some(desc) => format!("{code}\n\n{desc}"),
                None => code,
            };
            return Some(with_docs(trivia, &stage.span, body));
        }
        if word == "project" && !index.project_fields.is_empty() {
            return Some(format!("the `project` block — {} fields", index.project_fields.len()));
        }
    }

    match word {
        "platform" => Some("built-in variable `platform` — the host operating system".to_string()),
        _ => None,
    }
}

fn code_block(body: &str) -> String {
    format!("```mainstage\n{body}\n```")
}

/// Prepend the standalone (doc) comments attached above the node at `span` to
/// `body`, rendered as plain Markdown lines. Returns `body` unchanged when the
/// node carries no leading comments.
fn with_docs(trivia: Option<&TriviaMap>, span: &Span, body: String) -> String {
    let Some(node) = trivia.and_then(|t| t.get(span)) else { return body };
    if node.leading.is_empty() {
        return body;
    }
    let doc = node
        .leading
        .iter()
        .map(|c| c.text.trim_start_matches('/').trim())
        .collect::<Vec<_>>()
        .join("  \n");
    format!("{doc}\n\n{body}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use mainstage_core::Source;

    fn parse(text: &str) -> Program {
        mainstage_core::parse(&Source::from_str("test.ms", text)).expect("parse")
    }

    /// Position of the first occurrence of `needle` in `text` (at its first char).
    fn at(text: &str, needle: &str) -> Position {
        position_at(text, text.find(needle).expect("needle"))
    }

    fn markup(hover: &Hover) -> &str {
        match &hover.contents {
            HoverContents::Markup(m) => &m.value,
            _ => panic!("expected markup"),
        }
    }

    #[test]
    fn hovers_method_signature() {
        let registry = ModuleRegistry::standard();
        let text = "import \"git\" as g;\nlet v = g.sha();";
        let program = parse(text);
        let pos = at(text, "sha");
        let hover = hover(text, pos, &registry, Some(&program)).expect("hover");
        assert!(markup(&hover).contains("sha("));
        assert!(markup(&hover).contains("-> string"));
    }

    #[test]
    fn hovers_module_alias() {
        let registry = ModuleRegistry::standard();
        let text = "import \"git\" as g;\nlet v = g.sha();";
        let program = parse(text);
        // The alias `g` in the let binding.
        let pos = position_at(text, text.find("g.sha").unwrap());
        let hover = hover(text, pos, &registry, Some(&program)).expect("hover");
        assert!(markup(&hover).contains("module `git`"));
    }

    #[test]
    fn hovers_let_binding_resolved_form() {
        let registry = ModuleRegistry::standard();
        let text = "let name = \"demo\";\nlet other = name;";
        let program = parse(text);
        // The `name` reference in the second binding.
        let pos = position_at(text, text.rfind("name").unwrap());
        let hover = hover(text, pos, &registry, Some(&program)).expect("hover");
        assert!(markup(&hover).contains("let name = \"demo\""));
    }

    #[test]
    fn hovers_param_declaration() {
        let registry = ModuleRegistry::standard();
        let text = "param target: string = \"release\";\nlet label = target;";
        let program = parse(text);
        // The `target` reference in the let binding.
        let pos = position_at(text, text.rfind("target").unwrap());
        let hover = hover(text, pos, &registry, Some(&program)).expect("hover");
        assert!(markup(&hover).contains("param target: string = \"release\""));
    }

    #[test]
    fn hover_includes_leading_doc_comment() {
        let registry = ModuleRegistry::standard();
        let text = "// the build output directory\nlet out = \"dist\";\nlet other = out;";
        let program = parse(text);
        // The `out` reference in the second binding.
        let pos = position_at(text, text.rfind("out").unwrap());
        let hover = hover(text, pos, &registry, Some(&program)).expect("hover");
        let md = markup(&hover);
        assert!(md.contains("the build output directory"));
        assert!(md.contains("let out = \"dist\""));
    }

    #[test]
    fn hovers_stage_description_and_ordering() {
        let registry = ModuleRegistry::standard();
        let text = "stage setup {\n  steps {\n    $ echo hi\n  }\n}\n\
            stage build {\n  description: \"Compile it\"\n  depends_on: [setup]\n  steps {\n    $ echo hi\n  }\n}\n\
            pipeline ci {\n  stages: [build]\n}";
        let program = parse(text);
        // The `build` reference in the pipeline `stages:` list.
        let pos = position_at(text, text.find("[build]").unwrap() + 1);
        let hover = hover(text, pos, &registry, Some(&program)).expect("hover");
        let md = markup(&hover);
        assert!(md.contains("after setup"), "ordering shown: {md}");
        assert!(md.contains("Compile it"), "description shown: {md}");
    }

    #[test]
    fn no_hover_on_unknown_word() {
        let registry = ModuleRegistry::standard();
        let text = "let x = 1;";
        // The integer literal is not a known symbol.
        let pos = position_at(text, text.find('1').unwrap());
        assert!(hover(text, pos, &registry, Some(&parse(text))).is_none());
    }
}
