//! file: core/src/ast/node.rs
//! description: AST node container and helpers
//!
//! This module defines the `AstNode` record used throughout the compiler to
//! represent parsed declarations and statements. An `AstNode` carries:
//!
//! - a generated numeric `id` (opaque identifier used for debugging/tracking),
//! - a `kind` (the `AstNodeKind` enum describing node shape and payload),
//! - optional `location` and `span` metadata for diagnostics, and
//! - an `attributes` list for user-provided annotations.
//!
//! authors: Colton McGraw <github.com/ColtMcG0>
//! created: 2025-11-23
//! updated: 2025-12-04
//! license: LICENSE.md
//!
//! # Usage notes
//! - `AstNode` values are `Clone` and `PartialEq` to make testing and
//!   transformations convenient. `id` is regenerated on `new()` and preserved
//!   across clones.
//! - `location`/`span` are optional, but when present they should be used in
//!   error messages to point back to source positions.
//!
//! # Error semantics
//! - This module does not define errors itself, but `AstNode` construction may
//!   fail in higher-level parsing functions which produce `MainstageErrorExt`
//!   diagnostics.
//!
//! # See also
//!
//! - `crate::ast::rules` helpers: `get_location_from_pair` / `get_span_from_pair`
//! - `grammar.pest` (top-level rule: `Rule::script`)
//! - `crate::script::Script`
//!
//! # Thread-safety & performance
//! - `AstNode` is thread-safe (no interior mutability or global state). It can be
//!   shared across threads as needed.
//! - `AstNode` is lightweight to clone (simple data structure with owned fields). 
//!   It is suitable for use in compiler passes that transform or analyze the AST.
//!
use crate::location;

use super::kind::AstNodeKind;

/// An AST node with optional source `location` and `span` metadata.
#[derive(Clone, PartialEq)]
pub struct AstNode {
    id: usize,
    /// The semantic kind/payload of this node.
    pub kind: AstNodeKind,
    /// Arbitrary attributes attached to the node (e.g. from `@attr` syntax).
    pub attributes: Vec<String>,
    /// Optional source location (file, line, column) for the node's start.
    pub location: Option<location::Location>,
    /// Optional span covering the node's start and end positions.
    pub span: Option<location::Span>,
}

impl AstNode {
    /// Generate a monotonic id for new `AstNode`s.
    ///
    /// The id is created using an atomic counter and is intended for
    /// diagnostics and developer-facing tooling; it is not part of the
    /// language semantics.
    ///
    /// # Returns
    ///
    /// - `usize`: A unique numeric identifier for the node.
    ///
    fn create_id() -> usize {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(1);
        COUNTER.fetch_add(1, Ordering::Relaxed)
    }

    /// Create a new `AstNode` of the given `kind` with optional `location` and
    /// `span` metadata.
    ///
    /// Typical callers obtain location/span from the `rules` helpers and pass
    /// them through when building nodes so downstream phases (analyzers,
    /// emitters) can attach diagnostics to source regions.
    ///
    /// # Arguments
    ///
    /// - `node_type`: The `AstNodeKind` enum variant representing the node's kind.
    /// - `location`: Optional `Location` for the node's start position.
    /// - `span`: Optional `Span` covering the node's start and end positions.
    ///
    /// # Returns
    ///
    /// - `AstNode`: The newly created AST node.
    ///
    pub fn new(
        node_type: AstNodeKind,
        location: Option<location::Location>,
        span: Option<location::Span>,
    ) -> Self {
        AstNode {
            id: Self::create_id(),
            kind: node_type,
            location,
            span,
            attributes: vec![],
        }
    }

    /// Attach a `Location` to the node, returning the modified node.
    ///
    /// # Arguments
    ///
    /// - `location`: The `Location` to attach to the node.
    ///
    /// # Returns
    ///
    /// - `Self`: The modified node with the attached location.
    ///
    pub fn with_location(mut self, location: crate::location::Location) -> Self {
        self.location = Some(location);
        self
    }

    /// Attach a `Span` to the node, returning the modified node.
    ///
    /// # Arguments
    ///
    /// - `span`: The `Span` to attach to the node.
    ///
    /// # Returns
    ///
    /// - `Self`: The modified node with the attached span.
    ///
    pub fn with_span(mut self, span: crate::location::Span) -> Self {
        self.span = Some(span);
        self
    }

    /// Set attributes on the node, returning the modified node.
    ///
    /// # Arguments
    ///
    /// - `attributes`: A vector of attribute strings to attach to the node.
    ///
    /// # Returns
    ///
    /// - `Self`: The modified node with the attached attributes.
    ///
    pub fn with_attributes(mut self, attributes: Vec<String>) -> Self {
        self.attributes = attributes;
        self
    }

    /// Return the opaque numeric id for this node.
    ///
    /// # Returns
    ///
    /// - `usize`: The node's unique identifier.
    ///
    pub fn get_id(&self) -> usize {
        self.id
    }

    /// Return a reference to the node's `AstNodeKind`.
    ///
    /// # Returns
    ///
    /// - `&AstNodeKind`: A reference to the node's kind.
    ///
    pub fn get_kind(&self) -> &AstNodeKind {
        &self.kind
    }

    /// Return an optional reference to the node's `Location`.
    ///
    /// # Returns
    ///
    /// - `Option<&Location>`: An optional reference to the node's `Location`.
    ///
    pub fn get_location(&self) -> Option<&crate::location::Location> {
        self.location.as_ref()
    }

    /// Return an optional reference to the node's `Span`.
    ///
    /// # Returns
    ///
    /// - `Option<&Span>`: An optional reference to the node's `Span`.
    ///
    pub fn get_span(&self) -> Option<&crate::location::Span> {
        self.span.as_ref()
    }
}

use std::fmt;

/// Pretty `Display` implementation for `AstNode` used in debugging and tests.
/// The output includes id, kind (pretty-printed), location/span (if present),
/// and attributes.
impl fmt::Display for AstNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn fmt_indent(f: &mut fmt::Formatter<'_>, s: &str, indent: usize) -> fmt::Result {
            for _ in 0..indent {
                write!(f, " ")?;
            }
            write!(f, "{}", s)
        }

        // Header
        writeln!(f, "AstNode {{")?;
        fmt_indent(f, &format!("id: {},\n", self.id), 2)?;
        // Kind with pretty debug (allows readable nested enums/vecs)
        fmt_indent(f, "kind: ", 2)?;
        writeln!(f, "{:#?},", &self.kind)?;

        // Location
        if let Some(loc) = &self.location {
            fmt_indent(
                f,
                &format!("location: {}:{}:{}\n", loc.file, loc.line, loc.column),
                2,
            )?;
        } else {
            fmt_indent(f, "location: None\n", 2)?;
        }

        // Span
        if let Some(span) = &self.span {
            fmt_indent(
                f,
                &format!(
                    "span: start={}:{}:{} end={}:{}:{}\n",
                    span.start.file,
                    span.start.line,
                    span.start.column,
                    span.end.file,
                    span.end.line,
                    span.end.column
                ),
                2,
            )?;
        } else {
            fmt_indent(f, "span: None\n", 2)?;
        }

        // Attributes
        fmt_indent(f, &format!("attributes: {:?}\n", self.attributes), 2)?;

        writeln!(f, "}}")?;
        Ok(())
    }
}

impl fmt::Debug for AstNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Delegate to Display so both "{}" and "{:?}" are pretty
        write!(f, "{}", self)
    }
}
