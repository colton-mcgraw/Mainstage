//! Trivia-preserving syntax layer (Goal 3, Phase 20).
//!
//! The pest grammar treats `WHITESPACE` and `COMMENT` as silent rules, so neither
//! reaches the AST — formatting a script through the AST alone would erase every
//! comment. This module adds a *lossless* lexical pass alongside the AST: it splits
//! source into a flat token stream where concatenating the tokens reproduces the
//! original bytes exactly ([`render`]), and it attaches the captured comments and
//! blank-line grouping back onto AST nodes ([`attach`]) for the formatter to consume.
//!
//! The lexer here is deliberately independent of pest. pest's `COMMENT` is a special
//! built-in rule that is always silent and cannot be un-silenced, so the design note
//! for this phase calls for a "lossless token pass" instead — which is what [`lex`]
//! provides.

use std::collections::BTreeMap;
use std::path::Path;

use crate::ast::*;
use crate::error::Span;
use crate::source::Source;

// ── Token model ───────────────────────────────────────────────────────────────

/// The lexical category of a [`SyntaxToken`]. Together the four kinds partition the
/// source completely: every byte belongs to exactly one token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// Any run of non-trivia source text: identifiers, punctuation, and string
    /// literals (which are consumed whole, so a `//` or newline *inside* a string
    /// is never mistaken for a comment or line break).
    Code,
    /// A `//` line comment, excluding its terminating newline.
    Comment,
    /// A run of spaces and tabs.
    Whitespace,
    /// A single line terminator: `\n`, `\r\n`, or a lone `\r`.
    Newline,
}

/// One lexical token covering a contiguous slice of source. The token list produced
/// by [`lex`] is *lossless*: `tokens.iter().map(|t| &t.text).collect::<String>()`
/// equals the original source byte-for-byte.
#[derive(Debug, Clone)]
pub struct SyntaxToken {
    pub kind: TokenKind,
    /// The exact source slice this token covers.
    pub text: String,
    pub span: Span,
}

/// Whether a comment sits at the end of a line of code or stands on its own line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentKind {
    /// The comment is the only content on its line (only whitespace precedes it).
    Standalone,
    /// Code precedes the comment on the same line (a trailing, end-of-line comment).
    EndOfLine,
}

/// A single captured comment with its classification and source location.
#[derive(Debug, Clone)]
pub struct Comment {
    /// The full comment text including the leading `//`, excluding the line terminator.
    pub text: String,
    pub kind: CommentKind,
    pub span: Span,
}

// ── Lexer ─────────────────────────────────────────────────────────────────────

/// Tokenize `source` into a lossless [`SyntaxToken`] stream.
///
/// The result round-trips through [`render`] byte-for-byte. String literals
/// (including `${…}` interpolations and multi-line content) are consumed as part of
/// a single [`TokenKind::Code`] token, and exec-step lines (`$ …`) are taken
/// verbatim to end-of-line, so neither can spawn a spurious comment.
pub fn lex(source: &Source) -> Vec<SyntaxToken> {
    let mut lexer =
        Lexer { chars: source.text.chars().collect(), path: &source.path, pos: 0, line: 1, col: 1 };
    lexer.run()
}

/// Reconstruct source text from a token stream. The inverse of [`lex`]: for any
/// `s`, `render(&lex(&Source::from_str(p, s)))` equals `s`.
pub fn render(tokens: &[SyntaxToken]) -> String {
    tokens.iter().map(|t| t.text.as_str()).collect()
}

struct Lexer<'a> {
    chars: Vec<char>,
    path: &'a Path,
    pos: usize,
    /// 1-based line of the next unconsumed char.
    line: usize,
    /// 1-based column of the next unconsumed char.
    col: usize,
}

impl Lexer<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek2(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }

    /// Consume one character into `buf`, advancing the position and line/col cursor.
    fn bump(&mut self, buf: &mut String) {
        let c = self.chars[self.pos];
        buf.push(c);
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
    }

    fn run(&mut self) -> Vec<SyntaxToken> {
        let mut tokens = Vec::new();
        // `true` while only whitespace has been seen on the current line, used to
        // recognize an exec line (`$ …`) by its first non-whitespace character.
        let mut at_line_start = true;
        while let Some(c) = self.peek() {
            let (sl, sc) = (self.line, self.col);
            let mut buf = String::new();
            let kind = match c {
                '\r' | '\n' => {
                    if c == '\r' && self.peek2() == Some('\n') {
                        self.bump(&mut buf);
                    }
                    self.bump(&mut buf);
                    at_line_start = true;
                    TokenKind::Newline
                }
                ' ' | '\t' => {
                    while matches!(self.peek(), Some(' ') | Some('\t')) {
                        self.bump(&mut buf);
                    }
                    TokenKind::Whitespace
                }
                '/' if self.peek2() == Some('/') => {
                    while let Some(ch) = self.peek() {
                        if ch == '\n' || ch == '\r' {
                            break;
                        }
                        self.bump(&mut buf);
                    }
                    at_line_start = false;
                    TokenKind::Comment
                }
                _ => {
                    let exec_line = at_line_start && c == '$';
                    self.consume_code(&mut buf, exec_line);
                    at_line_start = false;
                    TokenKind::Code
                }
            };
            let span = Span {
                file: self.path.to_path_buf(),
                line_start: sl,
                col_start: sc,
                line_end: self.line,
                col_end: self.col,
            };
            tokens.push(SyntaxToken { kind, text: buf, span });
        }
        tokens
    }

    /// Consume a run of code into `buf`, stopping at the first trivia boundary.
    ///
    /// String literals are consumed whole so their contents never break the run.
    /// When `exec_line` is set the entire rest of the line is taken verbatim — exec
    /// steps capture everything after `$` up to the newline, including any `//`.
    fn consume_code(&mut self, buf: &mut String, exec_line: bool) {
        if exec_line {
            while let Some(c) = self.peek() {
                if c == '\n' || c == '\r' {
                    break;
                }
                self.bump(buf);
            }
            return;
        }
        while let Some(c) = self.peek() {
            match c {
                '\n' | '\r' | ' ' | '\t' => break,
                '/' if self.peek2() == Some('/') => break,
                '"' => self.consume_string(buf),
                _ => self.bump(buf),
            }
        }
    }

    /// Consume a `"…"` string literal into `buf`, including any `${…}` interpolations
    /// and multi-line content. Assumes the current character is the opening quote.
    fn consume_string(&mut self, buf: &mut String) {
        self.bump(buf); // opening quote
        while let Some(c) = self.peek() {
            match c {
                '"' => {
                    self.bump(buf); // closing quote
                    return;
                }
                '$' if self.peek2() == Some('{') => {
                    self.bump(buf); // $
                    self.bump(buf); // {
                    self.consume_braces(buf);
                }
                _ => self.bump(buf),
            }
        }
    }

    /// Consume the body of a `${…}` interpolation up to its matching `}`, tracking
    /// brace nesting and recursing into nested string literals. The opening `{` has
    /// already been consumed by the caller.
    fn consume_braces(&mut self, buf: &mut String) {
        let mut depth = 1usize;
        while let Some(c) = self.peek() {
            match c {
                '"' => self.consume_string(buf),
                '{' => {
                    depth += 1;
                    self.bump(buf);
                }
                '}' => {
                    depth -= 1;
                    self.bump(buf);
                    if depth == 0 {
                        return;
                    }
                }
                _ => self.bump(buf),
            }
        }
    }
}

// ── Comment extraction ──────────────────────────────────────────────────────────

/// Extract the comments from a token stream, classifying each as [`CommentKind`].
///
/// A comment is [`CommentKind::EndOfLine`] when a code token precedes it on the same
/// line, and [`CommentKind::Standalone`] otherwise.
pub fn comments(tokens: &[SyntaxToken]) -> Vec<Comment> {
    let mut out = Vec::new();
    let mut code_on_line = false;
    for t in tokens {
        match t.kind {
            TokenKind::Newline => code_on_line = false,
            TokenKind::Code => code_on_line = true,
            TokenKind::Whitespace => {}
            TokenKind::Comment => {
                let kind =
                    if code_on_line { CommentKind::EndOfLine } else { CommentKind::Standalone };
                out.push(Comment { text: t.text.clone(), kind, span: t.span.clone() });
            }
        }
    }
    out
}

// ── Trivia attachment ───────────────────────────────────────────────────────────

/// Comments and blank-line grouping attached to a single AST node.
#[derive(Debug, Clone, Default)]
pub struct NodeTrivia {
    /// Standalone comments on the lines immediately above the node.
    pub leading: Vec<Comment>,
    /// End-of-line comments on the node's last line (and any standalone comments that
    /// trail the final node with nothing following them).
    pub trailing: Vec<Comment>,
    /// Number of blank lines immediately preceding the node (or its leading comments).
    /// Lets the formatter preserve grouping between top-level items.
    pub blank_lines_before: usize,
}

impl NodeTrivia {
    fn is_meaningful(&self) -> bool {
        !self.leading.is_empty() || !self.trailing.is_empty() || self.blank_lines_before > 0
    }
}

/// Trivia attached to AST nodes, keyed by each node's starting `(line, col)`.
#[derive(Debug, Clone, Default)]
pub struct TriviaMap {
    nodes: BTreeMap<(usize, usize), NodeTrivia>,
}

impl TriviaMap {
    /// The trivia attached to the node beginning at `span`, if any.
    pub fn get(&self, span: &Span) -> Option<&NodeTrivia> {
        self.nodes.get(&(span.line_start, span.col_start))
    }

    /// Total number of nodes carrying trivia.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// `true` when no node carries any trivia.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// Attach the comments and blank-line grouping captured in `tokens` onto the nodes
/// of `program`, producing a [`TriviaMap`] the formatter can query by span.
///
/// Standalone comments become the leading trivia of the next node; end-of-line
/// comments become the trailing trivia of the closest node ending before them; and
/// standalone comments with no following node trail the final node.
pub fn attach(program: &Program, tokens: &[SyntaxToken]) -> TriviaMap {
    let anchors = collect_anchors(program);
    let comments = comments(tokens);
    let mut nodes: BTreeMap<(usize, usize), NodeTrivia> = BTreeMap::new();

    for c in &comments {
        match c.kind {
            CommentKind::EndOfLine => {
                // Attach to the node whose span ends closest before the comment.
                let anchor = anchors
                    .iter()
                    .filter(|a| (a.line_end, a.col_end) <= (c.span.line_start, c.span.col_start))
                    .max_by_key(|a| (a.line_end, a.col_end));
                if let Some(a) = anchor {
                    nodes.entry((a.line_start, a.col_start)).or_default().trailing.push(c.clone());
                }
            }
            CommentKind::Standalone => {
                // Attach to the next node that starts at or after the comment.
                let next = anchors
                    .iter()
                    .filter(|a| (a.line_start, a.col_start) >= (c.span.line_end, c.span.col_end))
                    .min_by_key(|a| (a.line_start, a.col_start));
                match next {
                    Some(a) => {
                        nodes
                            .entry((a.line_start, a.col_start))
                            .or_default()
                            .leading
                            .push(c.clone());
                    }
                    // No following node — trail the last anchor in the file.
                    None => {
                        if let Some(a) = anchors.iter().max_by_key(|a| (a.line_end, a.col_end)) {
                            nodes
                                .entry((a.line_start, a.col_start))
                                .or_default()
                                .trailing
                                .push(c.clone());
                        }
                    }
                }
            }
        }
    }

    // Blank-line grouping: for every anchor, count the blank lines immediately above
    // its leading comments (or above the node itself when it has none).
    let blank = blank_line_flags(tokens);
    for a in &anchors {
        let entry = nodes.entry((a.line_start, a.col_start)).or_default();
        let top = entry.leading.first().map(|c| c.span.line_start).unwrap_or(a.line_start);
        let mut count = 0;
        let mut line = top.saturating_sub(1);
        while line >= 1 && blank.get(line).copied().unwrap_or(false) {
            count += 1;
            line -= 1;
        }
        entry.blank_lines_before = count;
    }

    nodes.retain(|_, t| t.is_meaningful());
    TriviaMap { nodes }
}

/// Build a 1-based lookup of which lines are blank (contain no code or comment).
/// Index 0 is unused; lines beyond the source are reported as non-blank.
fn blank_line_flags(tokens: &[SyntaxToken]) -> Vec<bool> {
    let max_line = tokens.iter().map(|t| t.span.line_end).max().unwrap_or(1);
    let mut has_content = vec![false; max_line + 2];
    for t in tokens {
        if matches!(t.kind, TokenKind::Code | TokenKind::Comment) {
            for line in t.span.line_start..=t.span.line_end {
                if let Some(slot) = has_content.get_mut(line) {
                    *slot = true;
                }
            }
        }
    }
    has_content.iter().map(|&c| !c).collect()
}

// ── Anchor collection ───────────────────────────────────────────────────────────

/// Collect the spans of the line-level nodes that comments can attach to: top-level
/// items, project fields, and steps (recursively through `if`/`for` bodies).
fn collect_anchors(program: &Program) -> Vec<Span> {
    let mut anchors = Vec::new();
    for item in &program.items {
        anchors.push(item.span().clone());
        match item {
            Item::Project(p) => {
                for field in &p.fields {
                    anchors.push(field.span.clone());
                }
            }
            Item::Stage(s) => {
                collect_step_anchors(&s.steps, &mut anchors);
                collect_step_anchors(&s.on_failure, &mut anchors);
            }
            Item::Pipeline(p) => {
                collect_step_anchors(&p.on_failure, &mut anchors);
                collect_step_anchors(&p.on_success, &mut anchors);
            }
            Item::Import(_) | Item::Let(_) => {}
        }
    }
    anchors
}

fn collect_step_anchors(steps: &[Step], anchors: &mut Vec<Span>) {
    for step in steps {
        anchors.push(step.span().clone());
        match step {
            Step::If(s) => {
                collect_step_anchors(&s.then_steps, anchors);
                collect_step_anchors(&s.else_steps, anchors);
            }
            Step::For(s) => collect_step_anchors(&s.steps, anchors),
            Step::Try(s) => collect_step_anchors(&s.steps, anchors),
            Step::Workdir(s) => collect_step_anchors(&s.steps, anchors),
            Step::WithEnv(s) => collect_step_anchors(&s.steps, anchors),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn lex_str(src: &str) -> Vec<SyntaxToken> {
        lex(&Source::from_str("test.ms", src))
    }

    // ── Round-trip ──────────────────────────────────────────────────────────────

    fn assert_round_trips(src: &str) {
        let tokens = lex_str(src);
        assert_eq!(render(&tokens), src, "round-trip mismatch for {src:?}");
    }

    #[test]
    fn round_trips_empty() {
        assert_round_trips("");
    }

    #[test]
    fn round_trips_comments_and_blank_lines() {
        assert_round_trips("// header\n\nimport \"git\" as git; // trailing\n\n\n// dangling\n");
    }

    #[test]
    fn round_trips_crlf() {
        assert_round_trips("let a = 1;\r\nlet b = 2;\r\n");
    }

    #[test]
    fn round_trips_comment_marker_inside_string() {
        // The `//` lives inside a string literal and must not become a comment.
        assert_round_trips("let url = \"http://example.com\"; // real comment\n");
    }

    #[test]
    fn round_trips_interpolation_and_nested_strings() {
        assert_round_trips("let p = \"${ env.get(\"X\", default: \"a//b\") }/c\";\n");
    }

    #[test]
    fn round_trips_multiline_string() {
        assert_round_trips("let s = \"line1\n// still string\nline3\";\n");
    }

    #[test]
    fn round_trips_exec_line_with_comment_marker() {
        // Inside an exec line the `//` is part of the command, not a comment.
        assert_round_trips("stage s {\n  steps {\n    $ echo http://x // y\n  }\n}\n");
    }

    // ── Token classification ──────────────────────────────────────────────────────

    #[test]
    fn classifies_standalone_and_eol_comments() {
        let tokens = lex_str("// lead\nlet a = 1; // tail\n");
        let cs = comments(&tokens);
        assert_eq!(cs.len(), 2);
        assert_eq!(cs[0].kind, CommentKind::Standalone);
        assert_eq!(cs[0].text, "// lead");
        assert_eq!(cs[1].kind, CommentKind::EndOfLine);
        assert_eq!(cs[1].text, "// tail");
    }

    #[test]
    fn comment_marker_in_string_is_not_a_comment() {
        let tokens = lex_str("let u = \"a//b\";\n");
        assert!(comments(&tokens).is_empty());
    }

    #[test]
    fn newline_inside_string_does_not_split_into_blank_line() {
        let tokens = lex_str("let s = \"a\n\nb\";\n");
        // The whole string is one Code token spanning three lines.
        let code: Vec<_> = tokens.iter().filter(|t| t.kind == TokenKind::Code).collect();
        assert!(code.iter().any(|t| t.text.contains("a\n\nb")));
    }

    // ── Attachment ────────────────────────────────────────────────────────────────

    fn attach_str(src: &str) -> TriviaMap {
        let source = Source::from_str("test.ms", src);
        let program = parse(&source).expect("should parse");
        attach(&program, &lex(&source))
    }

    #[test]
    fn leading_comment_attaches_to_following_item() {
        let src = "// about git\nimport \"git\" as git;\n";
        let map = attach_str(src);
        let source = Source::from_str("test.ms", src);
        let program = parse(&source).unwrap();
        let import = program.items[0].span();
        let trivia = map.get(import).expect("import should carry trivia");
        assert_eq!(trivia.leading.len(), 1);
        assert_eq!(trivia.leading[0].text, "// about git");
        assert_eq!(trivia.leading[0].kind, CommentKind::Standalone);
    }

    #[test]
    fn trailing_comment_attaches_to_preceding_item() {
        let src = "import \"git\" as git; // vcs\n";
        let map = attach_str(src);
        let source = Source::from_str("test.ms", src);
        let program = parse(&source).unwrap();
        let trivia = map.get(program.items[0].span()).expect("trivia present");
        assert_eq!(trivia.trailing.len(), 1);
        assert_eq!(trivia.trailing[0].text, "// vcs");
        assert_eq!(trivia.trailing[0].kind, CommentKind::EndOfLine);
    }

    #[test]
    fn blank_lines_before_item_are_counted() {
        let src = "let a = 1;\n\n\nlet b = 2;\n";
        let source = Source::from_str("test.ms", src);
        let program = parse(&source).unwrap();
        let map = attach(&program, &lex(&source));
        let trivia = map.get(program.items[1].span()).expect("second let has grouping");
        assert_eq!(trivia.blank_lines_before, 2);
    }

    #[test]
    fn comment_inside_steps_attaches_to_step() {
        let src = "stage s {\n  steps {\n    // do it\n    delete \"x\"\n  }\n}\n";
        let source = Source::from_str("test.ms", src);
        let program = parse(&source).unwrap();
        let map = attach(&program, &lex(&source));
        let step_span = match &program.items[0] {
            Item::Stage(s) => s.steps[0].span(),
            _ => panic!("expected stage"),
        };
        let trivia = map.get(step_span).expect("step should carry the comment");
        assert_eq!(trivia.leading.len(), 1);
        assert_eq!(trivia.leading[0].text, "// do it");
    }

    #[test]
    fn dangling_comment_trails_final_node() {
        let src = "let a = 1;\n// the end\n";
        let source = Source::from_str("test.ms", src);
        let program = parse(&source).unwrap();
        let map = attach(&program, &lex(&source));
        let trivia = map.get(program.items[0].span()).expect("trivia present");
        assert_eq!(trivia.trailing.len(), 1);
        assert_eq!(trivia.trailing[0].text, "// the end");
    }
}
