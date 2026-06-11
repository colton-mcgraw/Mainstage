//! Signature help: when the cursor is inside a module call's `(...)`, render the
//! method's [`MethodSig::signature`] and highlight the active positional or named
//! parameter.

use mainstage_core::ModuleRegistry;
use mainstage_core::ast::Program;
use mainstage_core::modules::MethodSig;
use tower_lsp::lsp_types::{
    ParameterInformation, ParameterLabel, Position, SignatureHelp, SignatureInformation,
};

use crate::cursor::offset_at;
use crate::index::import_aliases;

/// The innermost open call surrounding the cursor.
struct Call {
    /// The identifier chain immediately before the `(`, e.g. `git.sha`.
    word: String,
    /// Number of top-level commas before the cursor — the active positional index.
    commas: usize,
    /// Byte offset of the current argument's start (after the `(` or last comma).
    arg_start: usize,
}

/// Compute signature help for the cursor at `pos`, or `None` when the cursor is
/// not inside a recognized module call.
pub fn signature_help(
    text: &str,
    pos: Position,
    registry: &ModuleRegistry,
    program: Option<&Program>,
) -> Option<SignatureHelp> {
    let before = &text[..offset_at(text, pos)];
    let call = active_call(before)?;
    let (alias, method) = call.word.rsplit_once('.')?;

    let aliases = import_aliases(text, program);
    let module = aliases.get(alias)?;
    let sig = registry.method_sig(module, method)?;

    let active = active_param(sig, &call, before);
    let parameters = sig
        .params
        .iter()
        .map(|p| param_info(&p.name, p.ty.describe()))
        .chain(sig.named.iter().map(|n| param_info(&n.name, n.ty.describe())))
        .collect();

    let info = SignatureInformation {
        label: sig.signature(),
        documentation: None,
        parameters: Some(parameters),
        active_parameter: Some(active),
    };
    Some(SignatureHelp {
        signatures: vec![info],
        active_signature: Some(0),
        active_parameter: Some(active),
    })
}

/// Find the innermost unclosed call bracket before the cursor, tracking strings
/// and nested `()`/`[]` so commas in nested calls or lists don't leak out.
fn active_call(before: &str) -> Option<Call> {
    let mut stack: Vec<Call> = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    let mut chain_start: Option<usize> = None;

    for (i, c) in before.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }

        if c.is_alphanumeric() || c == '_' || c == '.' {
            chain_start.get_or_insert(i);
            continue;
        }

        match c {
            '"' => in_string = true,
            '(' => {
                let word = chain_start.map(|s| before[s..i].to_string()).unwrap_or_default();
                stack.push(Call { word, commas: 0, arg_start: i + 1 });
            }
            '[' => stack.push(Call { word: String::new(), commas: 0, arg_start: i + 1 }),
            ')' | ']' => {
                stack.pop();
            }
            ',' => {
                if let Some(call) = stack.last_mut() {
                    call.commas += 1;
                    call.arg_start = i + 1;
                }
            }
            _ => {}
        }
        chain_start = None;
    }

    stack.pop()
}

/// The 0-based index of the active parameter: the named parameter currently
/// being typed, or the positional index from the comma count.
fn active_param(sig: &MethodSig, call: &Call, before: &str) -> u32 {
    let arg = before[call.arg_start..].trim_start();
    if let Some(name) = leading_named(arg)
        && let Some(i) = sig.named.iter().position(|n| n.name == name)
    {
        return (sig.params.len() + i) as u32;
    }
    let total = sig.params.len() + sig.named.len();
    (call.commas as u32).min(total.saturating_sub(1) as u32)
}

/// If `arg` begins with `<name>:`, return the keyword name.
fn leading_named(arg: &str) -> Option<String> {
    let name: String = arg.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    if name.is_empty() {
        return None;
    }
    arg[name.len()..].trim_start().starts_with(':').then_some(name)
}

fn param_info(name: &str, ty: &str) -> ParameterInformation {
    ParameterInformation {
        label: ParameterLabel::Simple(format!("{name}: {ty}")),
        documentation: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mainstage_core::Source;

    fn parse(text: &str) -> Program {
        mainstage_core::parse(&Source::from_str("test.ms", text)).expect("parse")
    }

    fn end(text: &str) -> Position {
        crate::cursor::position_at(text, text.len())
    }

    #[test]
    fn finds_innermost_call_and_word() {
        let call = active_call("let v = g.sha(").expect("call");
        assert_eq!(call.word, "g.sha");
        assert_eq!(call.commas, 0);
    }

    #[test]
    fn counts_top_level_commas_only() {
        let call = active_call("a.b(x, [1, 2], ").expect("call");
        assert_eq!(call.word, "a.b");
        assert_eq!(call.commas, 2);
    }

    #[test]
    fn ignores_commas_and_parens_in_strings() {
        let call = active_call("a.b(\"x, (y\", ").expect("call");
        assert_eq!(call.commas, 1);
    }

    #[test]
    fn detects_leading_named_argument() {
        assert_eq!(leading_named("short: tr"), Some("short".to_string()));
        assert_eq!(leading_named("  value"), None);
    }

    #[test]
    fn renders_signature_for_module_call() {
        let registry = ModuleRegistry::standard();
        let text = "import \"git\" as g;\nlet v = g.sha(";
        let program = parse("import \"git\" as g;\nlet v = g.sha();");
        let help = signature_help(text, end(text), &registry, Some(&program)).expect("help");
        assert_eq!(help.signatures.len(), 1);
        assert!(help.signatures[0].label.starts_with("sha("));
        assert_eq!(help.active_parameter, Some(0));
    }

    #[test]
    fn highlights_named_parameter() {
        let registry = ModuleRegistry::standard();
        // git.sha(short: bool): `short` is a named parameter.
        let sig = registry.method_sig("git", "sha").expect("git.sha");
        let named_index = sig.params.len(); // first named param
        let text = "import \"git\" as g;\nlet v = g.sha(short: ";
        let program = parse("import \"git\" as g;\nlet v = g.sha();");
        let help = signature_help(text, end(text), &registry, Some(&program)).expect("help");
        assert_eq!(help.active_parameter, Some(named_index as u32));
    }
}
