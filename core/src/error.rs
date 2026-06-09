use std::{fmt, path::PathBuf};

/// A half-open source range used for error reporting: file, start line/col, end line/col.
///
/// Line and column numbers are 1-based, matching the convention used by pest.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub file: PathBuf,
    pub line_start: usize,
    pub col_start: usize,
    pub line_end: usize,
    pub col_end: usize,
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.file.display(), self.line_start, self.col_start)
    }
}

/// A single user-facing error or warning with an optional source location and extra notes.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub message: String,
    pub span: Option<Span>,
    pub notes: Vec<String>,
}

impl Diagnostic {
    /// Create a diagnostic with only a message; attach a span with [`with_span`](Self::with_span).
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into(), span: None, notes: Vec::new() }
    }

    /// Attach a source location to this diagnostic.
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }

    /// Append a supplementary note shown below the primary message.
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(span) = &self.span {
            write!(f, "{}: {}", span, self.message)?;
        } else {
            write!(f, "{}", self.message)?;
        }
        for note in &self.notes {
            write!(f, "\n  note: {}", note)?;
        }
        Ok(())
    }
}

/// Top-level error type returned by all public `mainstage_core` APIs.
#[derive(Debug)]
pub enum Error {
    /// A filesystem operation failed while reading a source file.
    Io { path: PathBuf, source: std::io::Error },
    /// The pest grammar rejected the source; contains one or more diagnostics.
    Parse(Vec<Diagnostic>),
    /// The AST passed parsing but failed semantic validation.
    Semantic(Vec<Diagnostic>),
    /// Expression evaluation failed at runtime; contains one or more diagnostics.
    Eval(Vec<Diagnostic>),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path, source } => {
                write!(f, "error reading '{}': {}", path.display(), source)
            }
            Error::Parse(diags) | Error::Semantic(diags) | Error::Eval(diags) => {
                for (i, d) in diags.iter().enumerate() {
                    if i > 0 {
                        writeln!(f)?;
                    }
                    write!(f, "error: {}", d)?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
