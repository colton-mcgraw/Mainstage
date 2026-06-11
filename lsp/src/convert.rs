//! Position mapping between `mainstage_core` spans and LSP ranges.
//!
//! Core [`Span`]s use 1-based `(line, col)` pairs where `col` counts Unicode
//! scalar values (chars), matching pest's convention. LSP positions are 0-based
//! and measure columns in UTF-16 code units, so converting a column requires the
//! line's text to re-encode the leading characters.

use mainstage_core::Span;
use tower_lsp::lsp_types::{Position, Range};

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
}
