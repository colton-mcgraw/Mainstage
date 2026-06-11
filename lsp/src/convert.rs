//! Position mapping between `mainstage_core` spans and LSP ranges.
//!
//! Core [`Span`]s use 1-based `(line, col)` pairs where `col` counts Unicode
//! scalar values (chars), matching pest's convention. LSP positions are 0-based
//! and measure columns in UTF-16 code units, so converting a column requires the
//! line's text to re-encode the leading characters.

use mainstage_core::{Diagnostic as CoreDiagnostic, Span};
use tower_lsp::lsp_types::{
    Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, Location, Position, Range, Url,
};

/// Convert a 1-based, char-counted `(line, col)` core position into a 0-based
/// LSP [`Position`] with UTF-16 column semantics.
///
/// A `col` past the end of the line clamps to the line's length, and a `line`
/// past the end of the text yields an empty line, so out-of-range inputs never
/// panic.
fn position(text: &str, line: usize, col: usize) -> Position {
    let line_idx = line.saturating_sub(1);
    let line_text = text.lines().nth(line_idx).unwrap_or("");
    let char_idx = col.saturating_sub(1);
    let utf16: usize = line_text.chars().take(char_idx).map(char::len_utf16).sum();
    Position::new(line_idx as u32, utf16 as u32)
}

/// Convert a core [`Span`] into an LSP [`Range`]. `text` is the full document
/// the span refers to; it is needed to translate char columns into UTF-16
/// code-unit offsets.
pub fn span_to_range(text: &str, span: &Span) -> Range {
    Range::new(
        position(text, span.line_start, span.col_start),
        position(text, span.line_end, span.col_end),
    )
}

/// Convert a core [`CoreDiagnostic`] into an LSP [`Diagnostic`] for `uri`.
///
/// The diagnostic's span maps to the LSP range (defaulting to the document
/// start when the diagnostic carries no span); its `notes` become related
/// information anchored at the same location. Every core diagnostic is an error.
pub fn to_lsp_diagnostic(uri: &Url, text: &str, diag: &CoreDiagnostic) -> Diagnostic {
    let range = diag.span.as_ref().map(|span| span_to_range(text, span)).unwrap_or_default();
    let related_information = (!diag.notes.is_empty()).then(|| {
        diag.notes
            .iter()
            .map(|note| DiagnosticRelatedInformation {
                location: Location::new(uri.clone(), range),
                message: note.clone(),
            })
            .collect()
    });
    Diagnostic {
        range,
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("mainstage".to_string()),
        message: diag.message.clone(),
        related_information,
        ..Diagnostic::default()
    }
}

/// Convert a batch of core diagnostics into LSP diagnostics for `uri`.
pub fn to_lsp_diagnostics(uri: &Url, text: &str, diags: &[CoreDiagnostic]) -> Vec<Diagnostic> {
    diags.iter().map(|diag| to_lsp_diagnostic(uri, text, diag)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Build a span on `file.ms` from explicit 1-based coordinates.
    fn span(
        line_start: usize,
        col_start: usize,
        line_end: usize,
        col_end: usize,
    ) -> Span {
        Span { file: PathBuf::from("file.ms"), line_start, col_start, line_end, col_end }
    }

    #[test]
    fn ascii_single_line() {
        let range = span_to_range("abc", &span(1, 1, 1, 4));
        assert_eq!(range, Range::new(Position::new(0, 0), Position::new(0, 3)));
    }

    #[test]
    fn spans_multiple_lines() {
        // Second line, covering "cd".
        let range = span_to_range("ab\ncd", &span(2, 1, 2, 3));
        assert_eq!(range, Range::new(Position::new(1, 0), Position::new(1, 2)));
    }

    #[test]
    fn bmp_multibyte_chars_count_one_utf16_unit() {
        // 'é' is a single UTF-16 unit but two UTF-8 bytes; columns are char-based.
        let range = span_to_range("héllo", &span(1, 1, 1, 6));
        assert_eq!(range, Range::new(Position::new(0, 0), Position::new(0, 5)));
    }

    #[test]
    fn astral_chars_count_two_utf16_units() {
        // "a😀b": 'a' (1 unit), '😀' (surrogate pair, 2 units), 'b' (1 unit).
        // Select just the emoji at char column 2..3.
        let range = span_to_range("a😀b", &span(1, 2, 1, 3));
        assert_eq!(range, Range::new(Position::new(0, 1), Position::new(0, 3)));
    }

    #[test]
    fn column_past_end_clamps_to_line_length() {
        let range = span_to_range("ab", &span(1, 10, 1, 20));
        assert_eq!(range, Range::new(Position::new(0, 2), Position::new(0, 2)));
    }

    #[test]
    fn line_past_end_yields_empty_line() {
        let range = span_to_range("ab", &span(5, 1, 5, 1));
        assert_eq!(range, Range::new(Position::new(4, 0), Position::new(4, 0)));
    }

    fn uri() -> Url {
        Url::parse("file:///tmp/main.ms").unwrap()
    }

    #[test]
    fn diagnostic_maps_span_message_and_severity() {
        let core = CoreDiagnostic::new("bad thing").with_span(span(1, 1, 1, 3));
        let lsp = to_lsp_diagnostic(&uri(), "abc", &core);
        assert_eq!(lsp.message, "bad thing");
        assert_eq!(lsp.range, Range::new(Position::new(0, 0), Position::new(0, 2)));
        assert_eq!(lsp.severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(lsp.source.as_deref(), Some("mainstage"));
        assert!(lsp.related_information.is_none());
    }

    #[test]
    fn diagnostic_notes_become_related_information() {
        let core = CoreDiagnostic::new("oops")
            .with_span(span(1, 1, 1, 2))
            .with_note("first hint")
            .with_note("second hint");
        let lsp = to_lsp_diagnostic(&uri(), "abc", &core);
        let related = lsp.related_information.expect("notes should map to related info");
        assert_eq!(related.len(), 2);
        assert_eq!(related[0].message, "first hint");
        assert_eq!(related[1].message, "second hint");
        assert_eq!(related[0].location.uri, uri());
        assert_eq!(related[0].location.range, lsp.range);
    }

    #[test]
    fn spanless_diagnostic_defaults_to_document_start() {
        let core = CoreDiagnostic::new("no location");
        let lsp = to_lsp_diagnostic(&uri(), "abc", &core);
        assert_eq!(lsp.range, Range::default());
    }
}
