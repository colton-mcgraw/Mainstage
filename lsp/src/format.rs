//! Document formatting.
//!
//! Reuses the `mainstage_core` formatter — the same engine the `mainstage format`
//! CLI runs — so editor formatting matches the command line exactly. The whole
//! document is replaced with the formatted text via a single full-range [`TextEdit`].

use std::path::Path;

use mainstage_core::Source;
use tower_lsp::lsp_types::{Position, Range, TextEdit};

/// Format `text` (located at `path`), returning a single whole-document edit, or
/// `None` when the document does not parse (nothing to format) or is already
/// formatted (no edit needed).
pub fn formatting(text: &str, path: &Path) -> Option<Vec<TextEdit>> {
    let source = Source::from_str(path.to_path_buf(), text.to_string());
    let formatted = mainstage_core::format(&source).ok()?;
    if formatted == text {
        return None;
    }
    Some(vec![TextEdit { range: full_range(text), new_text: formatted }])
}

/// The range covering the entire document, from the start to one past the final
/// character. Columns are measured in UTF-16 code units, per the LSP spec.
fn full_range(text: &str) -> Range {
    let line = text.matches('\n').count() as u32;
    let last_line = text.rsplit('\n').next().unwrap_or("");
    let character = last_line.encode_utf16().count() as u32;
    Range::new(Position::new(0, 0), Position::new(line, character))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn path() -> PathBuf {
        PathBuf::from("test.ms")
    }

    #[test]
    fn produces_full_document_edit_for_unformatted_text() {
        let edits = formatting("let   x=1 ;", &path()).expect("should produce an edit");
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "let x = 1;\n");
        // The edit replaces the whole single line (11 UTF-16 units, no trailing newline).
        assert_eq!(edits[0].range, Range::new(Position::new(0, 0), Position::new(0, 11)));
    }

    #[test]
    fn already_formatted_text_yields_no_edit() {
        assert!(formatting("let x = 1;\n", &path()).is_none());
    }

    #[test]
    fn unparseable_text_yields_no_edit() {
        assert!(formatting("stage {", &path()).is_none());
    }

    #[test]
    fn full_range_spans_trailing_newline() {
        // Two lines plus a trailing newline → end position is line 2, column 0.
        let range = full_range("ab\ncd\n");
        assert_eq!(range, Range::new(Position::new(0, 0), Position::new(2, 0)));
    }
}
