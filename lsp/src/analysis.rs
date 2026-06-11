//! Shared analysis entry point.
//!
//! Non-panicking helpers that turn document text into a parsed [`Program`] plus
//! the diagnostics found while parsing and analyzing it. Every language-server
//! feature is built on these, so editor behavior stays in lock-step with the
//! `parse` → `analyze_with` pipeline the CLI runs.

use std::path::Path;

use mainstage_core::{
    analyze_with, ast::Program, parse, Diagnostic, Error, ModuleRegistry, Source,
};

/// The outcome of analyzing one document.
///
/// `program` is `Some` whenever parsing succeeded — even if semantic analysis
/// then reported errors — so editor features can keep working over a partially
/// invalid document. `diagnostics` collects every parse and semantic error,
/// ready to be mapped to LSP diagnostics.
#[derive(Debug, Default)]
pub struct Analysis {
    pub program: Option<Program>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Parse and analyze `text` (located at `path`) against `registry`. Never
/// panics: every failure mode is captured as diagnostics on the returned
/// [`Analysis`].
pub fn analyze(text: &str, path: &Path, registry: &ModuleRegistry) -> Analysis {
    let program = match parse_text(text, path) {
        Some(program) => program,
        None => {
            // Re-run the parse to recover its diagnostics (parse_text discards them).
            let source = Source::from_str(path.to_path_buf(), text.to_string());
            let diagnostics = parse(&source).err().map(diagnostics_of).unwrap_or_default();
            return Analysis { program: None, diagnostics };
        }
    };

    let diagnostics = match analyze_with(&program, registry) {
        Ok(_) => Vec::new(),
        Err(err) => diagnostics_of(err),
    };

    Analysis { program: Some(program), diagnostics }
}

/// Parse `text` (located at `path`), returning the [`Program`] on success and
/// `None` on any parse error. Used by features that only need the syntax tree.
pub fn parse_text(text: &str, path: &Path) -> Option<Program> {
    let source = Source::from_str(path.to_path_buf(), text.to_string());
    parse(&source).ok()
}

/// Build the module registry for a document's `script_dir`, mirroring the CLI:
/// plugin discovery, falling back to the standard registry on error.
pub fn build_registry(script_dir: &Path) -> ModuleRegistry {
    ModuleRegistry::with_plugins(script_dir).unwrap_or_else(|_| ModuleRegistry::standard())
}

/// Extract the diagnostics carried by an [`Error`]. The diagnostic-bearing
/// variants are returned as-is; an I/O error (parsing happens from an in-memory
/// string, so this should not occur) is surfaced as a single message.
fn diagnostics_of(err: Error) -> Vec<Diagnostic> {
    match err {
        Error::Parse(diags) | Error::Semantic(diags) | Error::Eval(diags) => diags,
        other => vec![Diagnostic::new(other.to_string())],
    }
}
