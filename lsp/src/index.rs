//! A lightweight index of a document's declarations, derived from the parsed
//! [`Program`]. Feature modules use it to resolve aliases, `let` bindings, stage
//! names, and `project` fields, and to render their resolved forms on hover.

use std::collections::HashMap;

use mainstage_core::Span;
use mainstage_core::ast::{Item, Program};

/// An `import "<module>" as <alias>` declaration.
pub struct ImportInfo {
    pub alias: String,
    pub module: String,
    pub span: Span,
}

/// A top-level `let <name> = <value>` binding.
pub struct LetInfo {
    pub name: String,
    pub span: Span,
    pub value_span: Span,
}

/// A `stage <name> { … }` declaration.
pub struct StageInfo {
    pub name: String,
    pub span: Span,
    pub outputs: Option<Span>,
    /// The stage's `description:` text, if any (surfaced on hover and in the outline).
    pub description: Option<String>,
    /// Explicit `depends_on` ordering edges, by stage name.
    pub depends_on: Vec<String>,
}

/// A single `<name>: <value>` field of the `project` block.
pub struct FieldInfo {
    pub name: String,
    pub span: Span,
    pub value_span: Span,
}

/// A `template <name> { … }` declaration (Phase 46).
pub struct TemplateInfo {
    pub name: String,
    pub span: Span,
}

/// The declarations extracted from a single program.
#[derive(Default)]
pub struct DocumentIndex {
    pub imports: Vec<ImportInfo>,
    pub lets: Vec<LetInfo>,
    pub stages: Vec<StageInfo>,
    pub project_fields: Vec<FieldInfo>,
    pub templates: Vec<TemplateInfo>,
}

impl DocumentIndex {
    /// Build an index from a parsed program.
    pub fn from_program(program: &Program) -> Self {
        let mut idx = Self::default();
        for item in &program.items {
            match item {
                Item::Import(d) => idx.imports.push(ImportInfo {
                    alias: d.alias.clone(),
                    module: d.module.clone(),
                    span: d.span.clone(),
                }),
                Item::Let(d) => idx.lets.push(LetInfo {
                    name: d.name.clone(),
                    span: d.span.clone(),
                    value_span: d.value.span().clone(),
                }),
                Item::Stage(s) => idx.stages.push(StageInfo {
                    name: s.name.clone(),
                    span: s.span.clone(),
                    outputs: s.outputs.as_ref().map(|e| e.span().clone()),
                    description: s.description.clone(),
                    depends_on: s.depends_on.iter().map(|d| d.name.clone()).collect(),
                }),
                Item::Project(p) => {
                    for f in &p.fields {
                        idx.project_fields.push(FieldInfo {
                            name: f.name.clone(),
                            span: f.span.clone(),
                            value_span: f.value.span().clone(),
                        });
                    }
                }
                Item::Template(t) => {
                    idx.templates.push(TemplateInfo { name: t.name.clone(), span: t.span.clone() })
                }
                Item::Pipeline(_) => {}
            }
        }
        idx
    }

    /// Whether `name` is a declared template.
    pub fn is_template(&self, name: &str) -> bool {
        self.templates.iter().any(|t| t.name == name)
    }

    /// The module name bound to `alias`, if any.
    pub fn module_for_alias(&self, alias: &str) -> Option<&str> {
        self.imports.iter().find(|i| i.alias == alias).map(|i| i.module.as_str())
    }

    /// Whether `name` is a declared stage.
    pub fn is_stage(&self, name: &str) -> bool {
        self.stages.iter().any(|s| s.name == name)
    }
}

/// The alias→module map for the document, preferring the parsed program and
/// falling back to a lexical scan so the mapping survives an unparseable edit
/// elsewhere (the common case while the user is mid-keystroke).
pub fn import_aliases(text: &str, program: Option<&Program>) -> HashMap<String, String> {
    match program {
        Some(p) => p
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Import(d) => Some((d.alias.clone(), d.module.clone())),
                _ => None,
            })
            .collect(),
        None => scan_import_aliases(text).into_iter().collect(),
    }
}

/// Lexically extract `import "<module>" as <alias>` pairs, tolerating
/// surrounding lines that do not parse.
pub fn scan_import_aliases(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim_start();
        let Some(rest) = line.strip_prefix("import") else { continue };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('"') else { continue };
        let Some(end) = rest.find('"') else { continue };
        let module = rest[..end].to_string();
        let rest = rest[end + 1..].trim_start();
        let Some(rest) = rest.strip_prefix("as") else { continue };
        let alias: String =
            rest.trim_start().chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        if !alias.is_empty() && !module.is_empty() {
            out.push((alias, module));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_imports_without_a_full_parse() {
        let text = "import \"git\" as g;\nimport \"env\" as e;\nthis line is broken @@@";
        let aliases = scan_import_aliases(text);
        assert_eq!(
            aliases,
            vec![("g".to_string(), "git".to_string()), ("e".to_string(), "env".to_string())]
        );
    }

    #[test]
    fn ignores_non_import_lines() {
        assert!(scan_import_aliases("let x = 1\nstage build {}").is_empty());
    }
}
