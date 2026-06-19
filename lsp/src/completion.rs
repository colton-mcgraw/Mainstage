//! Completion: module names inside `import "…"`, methods after `<alias>.`,
//! `project` fields, stage `outputs`, and `let` / stage / alias identifiers in
//! expression positions. The [`ModuleRegistry`] is the single source of truth
//! for available modules and their methods.

use mainstage_core::ModuleRegistry;
use mainstage_core::ast::{Item, Program, Step};
use mainstage_core::modules::MethodSig;
use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind, InsertTextFormat, Position};

use crate::cursor::{line_prefix, offset_at};
use crate::index::{DocumentIndex, import_aliases};

/// The metadata properties exposed by each item of a `for <var> in <fileset>`
/// loop, per `docs/GRAMMAR.md`. Offered after `<loop-var>.`.
const FILE_PROPERTIES: [&str; 5] = ["path", "name", "stem", "ext", "dir"];

/// Compute completion items for the cursor at `pos`. `program` is the latest
/// successful parse, if any; completion still works without it (e.g. mid-edit)
/// via a lexical scan of imports.
pub fn completions(
    text: &str,
    pos: Position,
    registry: &ModuleRegistry,
    program: Option<&Program>,
) -> Vec<CompletionItem> {
    let prefix = line_prefix(text, offset_at(text, pos));

    // Inside an `import "…"` string: offer module names.
    if inside_import_string(prefix) {
        return registry.module_names().into_iter().map(module_item).collect();
    }

    // Inside a plain string literal (not inside a `${…}` interpolation): nothing to complete.
    if in_plain_string(prefix) {
        return Vec::new();
    }

    // After `<receiver>.`: methods, project fields, or stage outputs.
    if let Some(receiver) = dotted_receiver(prefix) {
        return member_completions(&receiver, registry, text, program);
    }

    // Otherwise: identifiers valid in an expression position.
    expression_completions(text, program)
}

/// Completions following `<receiver>.`.
fn member_completions(
    receiver: &str,
    registry: &ModuleRegistry,
    text: &str,
    program: Option<&Program>,
) -> Vec<CompletionItem> {
    let aliases = import_aliases(text, program);
    if let Some(module) = aliases.get(receiver)
        && let Some(m) = registry.get(module)
    {
        return m.methods().iter().map(method_item).collect();
    }

    if let Some(program) = program {
        let idx = DocumentIndex::from_program(program);
        if receiver == "project" {
            return idx.project_fields.iter().map(|f| field_item(&f.name)).collect();
        }
        if idx.is_stage(receiver) {
            return vec![field_item("outputs")];
        }
        // A `for <var> in <fileset>` loop variable: offer the file item's metadata
        // properties (`<var>.path`, `<var>.name`, …).
        if for_loop_vars(program).iter().any(|v| v == receiver) {
            return FILE_PROPERTIES.iter().map(|f| field_item(f)).collect();
        }
    }

    Vec::new()
}

/// Collect the names of every `for <var> in …` loop variable declared anywhere
/// in the program, descending through nested `if`/`for` step blocks.
fn for_loop_vars(program: &Program) -> Vec<String> {
    let mut vars = Vec::new();
    for item in &program.items {
        match item {
            Item::Stage(s) => {
                collect_for_vars(&s.steps, &mut vars);
                collect_for_vars(&s.on_failure, &mut vars);
            }
            Item::Pipeline(p) => {
                collect_for_vars(&p.on_failure, &mut vars);
                collect_for_vars(&p.on_success, &mut vars);
            }
            _ => {}
        }
    }
    vars
}

fn collect_for_vars(steps: &[Step], vars: &mut Vec<String>) {
    for step in steps {
        match step {
            Step::For(f) => {
                vars.push(f.var.clone());
                collect_for_vars(&f.steps, vars);
            }
            Step::If(i) => {
                collect_for_vars(&i.then_steps, vars);
                collect_for_vars(&i.else_steps, vars);
            }
            Step::Try(t) => collect_for_vars(&t.steps, vars),
            Step::Workdir(w) => collect_for_vars(&w.steps, vars),
            Step::WithEnv(e) => collect_for_vars(&e.steps, vars),
            _ => {}
        }
    }
}

/// Identifiers valid in an expression position: `let` bindings, stage names,
/// import aliases, and the `project` / `platform` built-ins.
fn expression_completions(text: &str, program: Option<&Program>) -> Vec<CompletionItem> {
    let mut items = Vec::new();

    if let Some(program) = program {
        let idx = DocumentIndex::from_program(program);
        for l in &idx.lets {
            items.push(simple_item(&l.name, CompletionItemKind::VARIABLE));
        }
        for s in &idx.stages {
            items.push(simple_item(&s.name, CompletionItemKind::CLASS));
        }
        if !idx.project_fields.is_empty() {
            items.push(simple_item("project", CompletionItemKind::STRUCT));
        }
    }

    for (alias, _) in import_aliases(text, program) {
        items.push(simple_item(&alias, CompletionItemKind::MODULE));
    }
    items.push(simple_item("platform", CompletionItemKind::CONSTANT));

    items
}

// ── Context detection ────────────────────────────────────────────────────────

/// Whether the cursor sits inside the string of an `import "…"` declaration.
fn inside_import_string(prefix: &str) -> bool {
    prefix.trim_start().starts_with("import") && count_quotes(prefix) % 2 == 1
}

/// Count unescaped double quotes in `s`.
fn count_quotes(s: &str) -> usize {
    let mut count = 0;
    let mut escaped = false;
    for c in s.chars() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '"' {
            count += 1;
        }
    }
    count
}

/// Whether the cursor sits inside a plain (non-interpolated) string literal.
///
/// Returns `false` when inside a `${…}` interpolation, because an interpolation
/// is an expression context where completions are valid. Only `"` characters at
/// interpolation depth 0 toggle the "in string" state, so the outer string
/// boundaries are tracked correctly even when the interpolation contains nested
/// string arguments (e.g. `"${env.get("KEY")}"`).
fn in_plain_string(prefix: &str) -> bool {
    let mut in_string = false;
    let mut interp_depth: u32 = 0;
    let mut chars = prefix.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if in_string && interp_depth == 0 => {
                chars.next(); // skip the escaped character
            }
            '"' if interp_depth == 0 => {
                in_string = !in_string;
            }
            '$' if in_string => {
                if chars.peek() == Some(&'{') {
                    chars.next(); // consume '{'
                    interp_depth += 1;
                }
            }
            '}' if in_string && interp_depth > 0 => {
                interp_depth -= 1;
            }
            _ => {}
        }
    }
    in_string && interp_depth == 0
}

/// If `prefix` ends in `<receiver>.<partial>` (partial possibly empty), return
/// the receiver identifier.
fn dotted_receiver(prefix: &str) -> Option<String> {
    let bytes = prefix.as_bytes();
    let is_id = |b: u8| b.is_ascii_alphanumeric() || b == b'_';

    let mut i = prefix.len();
    while i > 0 && is_id(bytes[i - 1]) {
        i -= 1; // skip the partial method/field name being typed
    }
    if i == 0 || bytes[i - 1] != b'.' {
        return None;
    }
    i -= 1; // step over the dot
    let end = i;
    while i > 0 && is_id(bytes[i - 1]) {
        i -= 1; // collect the receiver identifier
    }
    (i < end).then(|| prefix[i..end].to_string())
}

// ── Item builders ────────────────────────────────────────────────────────────

fn module_item(name: &str) -> CompletionItem {
    simple_item(name, CompletionItemKind::MODULE)
}

fn field_item(name: &str) -> CompletionItem {
    simple_item(name, CompletionItemKind::FIELD)
}

fn simple_item(name: &str, kind: CompletionItemKind) -> CompletionItem {
    CompletionItem { label: name.to_string(), kind: Some(kind), ..Default::default() }
}

fn method_item(sig: &MethodSig) -> CompletionItem {
    CompletionItem {
        label: sig.name.clone(),
        kind: Some(CompletionItemKind::METHOD),
        detail: Some(sig.signature()),
        insert_text: Some(call_snippet(sig)),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        ..Default::default()
    }
}

/// A snippet that inserts the call with tab stops for each required parameter,
/// e.g. `get(${1:var}, default: ${2:default})`. Optional parameters are omitted.
fn call_snippet(sig: &MethodSig) -> String {
    let mut tab = 1;
    let mut parts = Vec::new();
    for p in sig.params.iter().filter(|p| p.required) {
        parts.push(format!("${{{tab}:{}}}", p.name));
        tab += 1;
    }
    for n in sig.named.iter().filter(|n| n.required) {
        parts.push(format!("{}: ${{{tab}:{}}}", n.name, n.name));
        tab += 1;
    }
    format!("{}({})", sig.name, parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mainstage_core::Source;

    fn parse(text: &str) -> Program {
        mainstage_core::parse(&Source::from_str("test.ms", text)).expect("parse")
    }

    /// Position just past the end of `text`.
    fn end(text: &str) -> Position {
        crate::cursor::position_at(text, text.len())
    }

    #[test]
    fn detects_import_string_context() {
        assert!(inside_import_string("import \""));
        assert!(inside_import_string("  import \"gi"));
        assert!(!inside_import_string("import \"git\" as g"));
        assert!(!inside_import_string("let x = \"gi"));
    }

    #[test]
    fn detects_dotted_receiver() {
        assert_eq!(dotted_receiver("let x = git."), Some("git".to_string()));
        assert_eq!(dotted_receiver("  git.sh"), Some("git".to_string()));
        assert_eq!(dotted_receiver("project.na"), Some("project".to_string()));
        assert_eq!(dotted_receiver("git"), None);
        assert_eq!(dotted_receiver(".sha"), None);
    }

    #[test]
    fn call_snippet_uses_required_params_only() {
        let registry = ModuleRegistry::standard();
        let sig = registry.method_sig("env", "get").expect("env.get");
        // env.get(var: string, default?: string): only `var` is required.
        assert_eq!(call_snippet(sig), "get(${1:var})");
    }

    #[test]
    fn import_string_offers_module_names() {
        let registry = ModuleRegistry::standard();
        let text = "import \"";
        let items = completions(text, end(text), &registry, None);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"git"));
        assert!(labels.contains(&"env"));
    }

    #[test]
    fn member_access_offers_methods_with_snippets() {
        let registry = ModuleRegistry::standard();
        // Mid-edit, so the document does not parse: aliases come from a lexical scan.
        let text = "import \"git\" as g;\nlet v = g.";
        let items = completions(text, end(text), &registry, None);
        let sha = items.iter().find(|i| i.label == "sha").expect("git.sha");
        assert_eq!(sha.kind, Some(CompletionItemKind::METHOD));
        assert_eq!(sha.insert_text_format, Some(InsertTextFormat::SNIPPET));
    }

    #[test]
    fn expression_position_offers_lets_and_stages() {
        let registry = ModuleRegistry::standard();
        let text = "let foo = \"x\";\nstage build {\n    steps {\n        $ echo hi\n    }\n}\nlet bar = foo;";
        let program = parse(text);
        // Cursor inside the `foo` reference of the last binding.
        let pos = crate::cursor::position_at(text, text.rfind("foo").unwrap() + 1);
        let items = completions(text, pos, &registry, Some(&program));
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"foo"));
        assert!(labels.contains(&"build"));
        assert!(labels.contains(&"platform"));
    }

    #[test]
    fn loop_variable_offers_file_properties() {
        let registry = ModuleRegistry::standard();
        let text = "stage s {\n    steps {\n        for file in glob(\"*.rs\") {\n            $ echo ${file.\n        }\n    }\n}";
        let program = parse(
            "stage s {\n    steps {\n        for file in glob(\"*.rs\") {\n            $ echo done\n        }\n    }\n}",
        );
        // Cursor right after `file.` in the exec line.
        let pos = crate::cursor::position_at(text, text.find("file.").unwrap() + "file.".len());
        let items = completions(text, pos, &registry, Some(&program));
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"path"));
        assert!(labels.contains(&"stem"));
        assert_eq!(items.len(), FILE_PROPERTIES.len());
    }

    #[test]
    fn inside_plain_string_offers_nothing() {
        let registry = ModuleRegistry::standard();
        let text = "let x = \"hello";
        assert!(completions(text, end(text), &registry, None).is_empty());
    }

    #[test]
    fn detects_plain_string_vs_interpolation() {
        assert!(in_plain_string("let x = \"hello"));
        assert!(!in_plain_string("let x = \"${proj"));
        assert!(!in_plain_string("\"${out}/${project."));
        assert!(!in_plain_string("let x = \"${env.get(\"KEY\"")); // nested string
        assert!(!in_plain_string("let x = \"done\"")); // closed string
    }

    #[test]
    fn member_access_inside_interpolation_offers_completions() {
        let registry = ModuleRegistry::standard();
        // Mid-edit: cursor is after `g.` inside a string interpolation. The doc
        // doesn't parse in this state, so we pass the last-good program explicitly.
        let text = "import \"git\" as g;\nstage s {\n    steps {\n        $ echo \"${g.";
        let program =
            parse("import \"git\" as g;\nstage s {\n    steps {\n        $ echo done\n    }\n}");
        let items = completions(text, end(text), &registry, Some(&program));
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(!items.is_empty(), "expected completions inside string interpolation");
        assert!(labels.contains(&"sha"), "expected git methods like 'sha' in {labels:?}");
    }
}
