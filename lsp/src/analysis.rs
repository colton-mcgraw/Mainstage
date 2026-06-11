//! Shared analysis entry point.
//!
//! A single, non-panicking helper that turns document text into a parsed
//! [`Program`] plus the diagnostics found while parsing and analyzing it. Every
//! later language-server feature (diagnostics, completion, hover, navigation)
//! is built on top of this, so editor behavior stays in lock-step with the
//! `parse` → `analyze_with` pipeline the CLI runs.

use std::path::{Path, PathBuf};

use mainstage_core::{
    analyze_with, ast::Program, parse, Diagnostic, Error, ModuleRegistry, Source,
};
use tower_lsp::lsp_types::Url;

/// The outcome of analyzing one document.
///
/// `program` is `Some` whenever parsing succeeded — even if semantic analysis
/// then reported errors — so editor features can keep working over a partially
/// invalid document. `diagnostics` collects every parse and semantic error,
/// ready to be mapped to LSP diagnostics by later phases.
#[derive(Debug, Default)]
pub struct Analysis {
    pub program: Option<Program>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Parse and analyze `text` as a `.ms` document located at `path`, resolving
/// modules (including any plugins discovered under `script_dir`) the same way
/// the CLI does so analysis in the editor matches the command line.
///
/// Never panics: every failure mode is captured as diagnostics on the returned
/// [`Analysis`].
pub fn analyze_document(text: &str, path: &Path, script_dir: &Path) -> Analysis {
    let source = Source::from_str(path.to_path_buf(), text.to_string());
    let program = match parse(&source) {
        Ok(program) => program,
        Err(err) => return Analysis { program: None, diagnostics: diagnostics_of(err) },
    };

    // Mirror the CLI's registry. Plugin discovery is best-effort: fall back to
    // the standard registry if it fails rather than dropping analysis entirely.
    let registry =
        ModuleRegistry::with_plugins(script_dir).unwrap_or_else(|_| ModuleRegistry::standard());

    let diagnostics = match analyze_with(&program, &registry) {
        Ok(_) => Vec::new(),
        Err(err) => diagnostics_of(err),
    };

    Analysis { program: Some(program), diagnostics }
}

/// Convenience wrapper that derives the document path and its script directory
/// from a document `uri` before delegating to [`analyze_document`].
pub fn analyze_uri(uri: &Url, text: &str) -> Analysis {
    let path = uri.to_file_path().unwrap_or_else(|_| PathBuf::from(uri.path()));
    let script_dir = path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    analyze_document(text, &path, &script_dir)
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
