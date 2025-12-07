//! Types and utilities for inferred kinds used by semantic analysis.
//!
//! Defines `Kind`, `InferredKind` and helpers for unification/compatibility
//! checks used throughout analysis passes.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Integer,
    Float,
    String,
    Boolean,
    Void,
    Null,
    Object,
    Array,
    Dynamic,
    Plugin
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Kind::Integer => "Integer",
            Kind::Float => "Float",
            Kind::String => "String",
            Kind::Boolean => "Boolean",
            Kind::Void => "Void",
            Kind::Null => "Null",
            Kind::Object => "Object",
            Kind::Array => "Array",
            Kind::Dynamic => "Dynamic",
            Kind::Plugin => "Plugin",
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Origin {
    Expression,
    Coerced,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InferredKind {
    pub kind: Kind,
    pub origin: Origin,
    pub location: Option<crate::location::Location>,
    pub span: Option<crate::location::Span>,
    // For container types (e.g., Array) we may carry an element type.
    pub element: Option<Box<InferredKind>>,
}

impl Default for InferredKind {
    fn default() -> Self {
        InferredKind {
            kind: Kind::Dynamic,
            origin: Origin::Unknown,
            location: None,
            span: None,
            element: None,
        }
    }
}

impl InferredKind {
    pub fn new(
        kind: Kind,
        origin: Origin,
        location: Option<crate::location::Location>,
        span: Option<crate::location::Span>,
    ) -> Self {
        InferredKind {
            kind,
            origin,
            location,
            span,
            element: None,
        }
    }

    // Convenience constructors
    pub fn integer() -> Self {
        Self::new(Kind::Integer, Origin::Expression, None, None)
    }
    pub fn float() -> Self {
        Self::new(Kind::Float, Origin::Expression, None, None)
    }
    pub fn string() -> Self {
        Self::new(Kind::String, Origin::Expression, None, None)
    }
    pub fn boolean() -> Self {
        Self::new(Kind::Boolean, Origin::Expression, None, None)
    }
    pub fn dynamic() -> Self {
        Self::default()
    }
    pub fn plugin() -> Self {
        Self::new(Kind::Plugin, Origin::Expression, None, None)
    }

    /// Attach an element kind for container types like Array.
    pub fn with_element(mut self, elem: InferredKind) -> Self {
        self.element = Some(Box::new(elem));
        self
    }

    // Builder-style modifiers useful in analysis passes
    pub fn with_origin(mut self, origin: Origin) -> Self {
        self.origin = origin;
        self
    }
    pub fn with_location(mut self, loc: crate::location::Location) -> Self {
        self.location = Some(loc);
        self
    }
    pub fn with_span(mut self, span: crate::location::Span) -> Self {
        self.span = Some(span);
        self
    }

    // Predicates
    pub fn is_numeric(&self) -> bool {
        matches!(self.kind, Kind::Integer | Kind::Float)
    }
    pub fn is_dynamic(&self) -> bool {
        matches!(self.kind, Kind::Dynamic)
    }
    pub fn is_null(&self) -> bool {
        matches!(self.kind, Kind::Null)
    }
    pub fn is_plugin(&self) -> bool {
        matches!(self.kind, Kind::Plugin)
    }

    pub fn element_kind(&self) -> Option<&InferredKind> {
        self.element.as_deref()
    }

    // Compatibility test: true if values of `other` can be used where `self` is expected.
    // Dynamic and Null are treated permissively; caller can tighten rules if needed.
    pub fn is_compatible_with(&self, other: &InferredKind) -> bool {
        if self.is_dynamic() || other.is_dynamic() {
            return true;
        }
        // If both are arrays, element compatibility matters
        if matches!(self.kind, Kind::Array) && matches!(other.kind, Kind::Array) {
            match (self.element_kind(), other.element_kind()) {
                (Some(a), Some(b)) => return a.is_compatible_with(b),
                // If either lacks element info, be permissive
                _ => return true,
            }
        }
        if self.kind == other.kind {
            return true;
        }
        // allow Null to be used with non-primitive containers or Dynamic
        if other.is_null() {
            return true;
        }
        // numeric coercion allowed both ways (Integer <-> Float)
        if (matches!(self.kind, Kind::Float) && matches!(other.kind, Kind::Integer))
            || (matches!(self.kind, Kind::Integer) && matches!(other.kind, Kind::Float))
        {
            return true;
        }

        // Bool and Integer are compatible both ways (allow implicit promotion)
        if (matches!(self.kind, Kind::Integer) && matches!(other.kind, Kind::Boolean))
            || (matches!(self.kind, Kind::Boolean) && matches!(other.kind, Kind::Integer))
        {
            return true;
        }

        // Allow any type to be used where a string is expected (implicit to-string)
        if matches!(self.kind, Kind::String) {
            return true;
        }
        false
    }

    // Return the unified/coerced kind for two operands (used for binary arithmetic, etc.)
    // Simple rules:
    // - same => same
    // - Integer + Float => Float
    // - anything with Dynamic => Dynamic
    // - if incompatible => Dynamic (caller may treat as error)
    pub fn unify(&self, other: &InferredKind) -> InferredKind {
        use Kind::*;
        if self.kind == other.kind {
            return InferredKind {
                kind: self.kind.clone(),
                origin: Origin::Coerced,
                location: self.location.clone().or(other.location.clone()),
                span: self.span.clone().or(other.span.clone()),
                element: self.element.clone().or(other.element.clone()),
            };
        }
        if self.is_dynamic() || other.is_dynamic() {
            return InferredKind::dynamic();
        }
        match (&self.kind, &other.kind) {
            // Numeric coercion: Float wins
            (Float, Integer) | (Integer, Float) => InferredKind::new(
                Float,
                Origin::Coerced,
                self.location.clone().or(other.location.clone()),
                self.span.clone().or(other.span.clone()),
            ),
            // Array + Array => unify element kinds
            (Array, Array) => {
                // if either has no element info, result is Array(dynamic)
                match (self.element_kind(), other.element_kind()) {
                    (Some(a), Some(b)) => {
                        let unified = a.unify(b);
                        let mut out = InferredKind::new(
                            Array,
                            Origin::Coerced,
                            self.location.clone().or(other.location.clone()),
                            self.span.clone().or(other.span.clone()),
                        );
                        out = out.with_element(unified);
                        out
                    }
                    _ => InferredKind::new(
                        Array,
                        Origin::Coerced,
                        self.location.clone().or(other.location.clone()),
                        self.span.clone().or(other.span.clone()),
                    ),
                }
            }
            // Bool + Integer => Integer
            (Integer, Boolean) | (Boolean, Integer) => InferredKind::new(
                Integer,
                Origin::Coerced,
                self.location.clone().or(other.location.clone()),
                self.span.clone().or(other.span.clone()),
            ),
            // Bool + Float => Float
            (Float, Boolean) | (Boolean, Float) => InferredKind::new(
                Float,
                Origin::Coerced,
                self.location.clone().or(other.location.clone()),
                self.span.clone().or(other.span.clone()),
            ),
            (Null, k) | (k, Null) => InferredKind::new(
                k.clone(),
                Origin::Coerced,
                self.location.clone().or(other.location.clone()),
                self.span.clone().or(other.span.clone()),
            ),
            _ => InferredKind::dynamic(),
        }
    }
}

impl fmt::Display for InferredKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let origin = match &self.origin {
            Origin::Expression => "inferred",
            Origin::Coerced => "coerced",
            Origin::Unknown => "unknown",
        };

        // For array kinds include the element type if present: Array<Integer>
        if let Kind::Array = &self.kind {
            if let Some(elem) = &self.element {
                return write!(f, "Array<{}> ({})", elem, origin);
            } else {
                return write!(f, "Array<?> ({})", origin);
            }
        }

        write!(f, "{} ({})", self.kind, origin)
    }
}