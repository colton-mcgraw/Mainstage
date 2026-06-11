//! Cursor helpers bridging LSP positions and byte offsets in the document text.
//!
//! LSP positions are 0-based with UTF-16 column semantics; core spans are
//! 1-based with char columns. These helpers convert between those and raw byte
//! offsets so the feature modules can slice the source safely.

use mainstage_core::Span;
use tower_lsp::lsp_types::Position;

/// Byte offset in `text` of the 0-based, UTF-16 LSP `pos`. Clamps to the end of
/// the targeted line and the end of the text, never panicking.
pub fn offset_at(text: &str, pos: Position) -> usize {
    let bytes = text.as_bytes();
    let mut byte = 0;
    let mut line = 0;
    while line < pos.line && byte < bytes.len() {
        if bytes[byte] == b'\n' {
            line += 1;
        }
        byte += 1;
    }
    let mut utf16 = 0;
    for ch in text[byte..].chars() {
        if ch == '\n' || utf16 >= pos.character {
            break;
        }
        utf16 += ch.len_utf16() as u32;
        byte += ch.len_utf8();
    }
    byte
}

/// The 0-based, UTF-16 LSP position of byte `offset` in `text`.
pub fn position_at(text: &str, offset: usize) -> Position {
    let offset = offset.min(text.len());
    let mut line = 0;
    let mut line_start = 0;
    for (i, ch) in text[..offset].char_indices() {
        if ch == '\n' {
            line += 1;
            line_start = i + 1;
        }
    }
    let character = text[line_start..offset].chars().map(|c| c.len_utf16() as u32).sum();
    Position::new(line, character)
}

/// The slice of the current line from its start up to `offset`.
pub fn line_prefix(text: &str, offset: usize) -> &str {
    let start = text[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0);
    &text[start..offset]
}

/// The byte span `[start, end)` of the identifier covering `offset`, or `None`
/// when the cursor is not on an identifier character. The cursor sitting at the
/// trailing edge of an identifier still selects it.
pub fn ident_at(text: &str, offset: usize) -> Option<(usize, usize)> {
    let is_id = |c: char| c.is_alphanumeric() || c == '_';
    let offset = offset.min(text.len());

    let mut start = offset;
    for (i, ch) in text[..offset].char_indices().rev() {
        if is_id(ch) {
            start = i;
        } else {
            break;
        }
    }

    let mut end = offset;
    for (i, ch) in text[offset..].char_indices() {
        if is_id(ch) {
            end = offset + i + ch.len_utf8();
        } else {
            break;
        }
    }

    (start != end).then_some((start, end))
}

/// The source slice covered by a core [`Span`], clamped to `text`.
pub fn slice_span<'a>(text: &'a str, span: &Span) -> &'a str {
    let start = byte_offset(text, span.line_start, span.col_start);
    let end = byte_offset(text, span.line_end, span.col_end).clamp(start, text.len());
    &text[start..end]
}

/// Byte offset of a 1-based `(line, col)` core position (col counts chars).
fn byte_offset(text: &str, line: usize, col: usize) -> usize {
    let bytes = text.as_bytes();
    let mut byte = 0;
    let mut cur_line = 1;
    while cur_line < line && byte < bytes.len() {
        if bytes[byte] == b'\n' {
            cur_line += 1;
        }
        byte += 1;
    }
    let mut cur_col = 1;
    for ch in text[byte..].chars() {
        if cur_col >= col || ch == '\n' {
            break;
        }
        cur_col += 1;
        byte += ch.len_utf8();
    }
    byte
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn offset_round_trips_with_position() {
        let text = "abc\nde😀f\nghi";
        for offset in 0..=text.len() {
            if !text.is_char_boundary(offset) {
                continue;
            }
            let pos = position_at(text, offset);
            assert_eq!(offset_at(text, pos), offset, "offset {offset}");
        }
    }

    #[test]
    fn offset_at_handles_utf16_columns() {
        // "a😀b": the 'b' is at UTF-16 column 3 (emoji is a surrogate pair).
        let text = "a😀b";
        assert_eq!(offset_at(text, Position::new(0, 3)), "a😀".len());
    }

    #[test]
    fn line_prefix_is_text_before_cursor_on_its_line() {
        let text = "let x = 1\nlet y = git.";
        let offset = text.len();
        assert_eq!(line_prefix(text, offset), "let y = git.");
    }

    #[test]
    fn ident_at_selects_word_under_and_after_cursor() {
        let text = "git.sha";
        assert_eq!(ident_at(text, 0), Some((0, 3))); // start of "git"
        assert_eq!(ident_at(text, 3), Some((0, 3))); // trailing edge of "git"
        assert_eq!(ident_at(text, 4), Some((4, 7))); // start of "sha"
        assert_eq!(ident_at(text, 3 + 0), Some((0, 3)));
        assert_eq!(ident_at(".", 0), None);
    }

    #[test]
    fn slice_span_extracts_source() {
        let text = "let name = \"value\"";
        let span = Span {
            file: PathBuf::from("x.ms"),
            line_start: 1,
            col_start: 12,
            line_end: 1,
            col_end: 19,
        };
        assert_eq!(slice_span(text, &span), "\"value\"");
    }
}
