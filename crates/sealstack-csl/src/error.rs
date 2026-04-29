//! Error and diagnostic types.

use std::fmt;

use miette::{Diagnostic, NamedSource, SourceSpan};
use thiserror::Error;

use crate::span::Span;

/// Result type used throughout the crate.
pub type CslResult<T> = Result<T, CslError>;

/// Top-level error type. All variants carry enough spans to be rendered via `miette`.
#[derive(Debug, Error, Diagnostic)]
pub enum CslError {
    /// Parse error — source did not match the grammar.
    #[error("parse error: {message}")]
    #[diagnostic(code(sealstack_csl::parse))]
    Parse {
        /// Error message.
        message: String,
        /// Named source for rendering.
        #[source_code]
        src: NamedSource<String>,
        /// Span of the failing input.
        #[label = "here"]
        span: SourceSpan,
    },

    /// Type error — AST was well-formed but ill-typed.
    #[error("type error: {message}")]
    #[diagnostic(code(sealstack_csl::type_check))]
    Type {
        /// Error message.
        message: String,
        /// Named source.
        #[source_code]
        src: NamedSource<String>,
        /// Primary span.
        #[label("{label}")]
        span: SourceSpan,
        /// Label text for the primary span.
        label: String,
        /// Optional help hint.
        #[help]
        help: Option<String>,
    },

    /// Missing `@primary` field.
    #[error("schema `{schema}` has no `@primary` field")]
    #[diagnostic(
        code(sealstack_csl::missing_primary),
        help("add `@primary` to exactly one field")
    )]
    MissingPrimaryKey {
        /// Schema name.
        schema: String,
        /// Source.
        #[source_code]
        src: NamedSource<String>,
        /// Where the schema was declared.
        #[label("schema declared here")]
        span: SourceSpan,
    },

    /// Multiple `@primary` fields on one schema.
    #[error("schema `{schema}` has multiple `@primary` fields: {fields:?}")]
    #[diagnostic(
        code(sealstack_csl::duplicate_primary),
        help("exactly one field must be annotated `@primary`")
    )]
    DuplicatePrimaryKey {
        /// Schema name.
        schema: String,
        /// Conflicting field names.
        fields: Vec<String>,
        /// Source.
        #[source_code]
        src: NamedSource<String>,
        /// Span of the schema.
        #[label("schema declared here")]
        span: SourceSpan,
    },

    /// Reference target not found in any known schema.
    #[error("reference target `{target}` not found")]
    #[diagnostic(
        code(sealstack_csl::unknown_ref_target),
        help("declare schema `{target}` or import it from another namespace")
    )]
    UnknownRefTarget {
        /// Target type name.
        target: String,
        /// Source.
        #[source_code]
        src: NamedSource<String>,
        /// Span of the reference.
        #[label("here")]
        span: SourceSpan,
    },

    /// Duplicate schema name within a namespace.
    #[error("duplicate schema `{name}` in namespace `{namespace}`")]
    #[diagnostic(code(sealstack_csl::duplicate_schema))]
    DuplicateSchema {
        /// Duplicated name.
        name: String,
        /// Namespace.
        namespace: String,
        /// Source.
        #[source_code]
        src: NamedSource<String>,
        /// Span of the duplicate declaration.
        #[label("re-declared here")]
        span: SourceSpan,
    },

    /// Codegen error — the compiler failed to emit a target.
    #[error("codegen error: {message}")]
    #[diagnostic(code(sealstack_csl::codegen))]
    Codegen {
        /// Error message.
        message: String,
    },

    /// I/O error bubbled up from a caller.
    #[error("io error: {0}")]
    #[diagnostic(code(sealstack_csl::io))]
    Io(#[from] std::io::Error),
}

impl CslError {
    /// Helper for building a plain parse error from a winnow context message.
    #[must_use]
    pub fn parse(
        filename: Option<&str>,
        source: &str,
        span: Span,
        message: impl Into<String>,
    ) -> Self {
        Self::Parse {
            message: message.into(),
            src: NamedSource::new(filename.unwrap_or("<input>"), source.to_owned()),
            span: span.into(),
        }
    }

    /// Helper for type-error construction.
    #[must_use]
    pub fn type_err(
        filename: Option<&str>,
        source: &str,
        span: Span,
        message: impl Into<String>,
        label: impl Into<String>,
        help: Option<String>,
    ) -> Self {
        Self::Type {
            message: message.into(),
            src: NamedSource::new(filename.unwrap_or("<input>"), source.to_owned()),
            span: span.into(),
            label: label.into(),
            help,
        }
    }
}

/// A collection of non-fatal warnings emitted during compilation.
#[derive(Clone, Debug, Default)]
pub struct Diagnostics {
    /// The warnings themselves.
    pub warnings: Vec<Warning>,
}

impl Diagnostics {
    /// Whether there are any warnings.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.warnings.is_empty()
    }

    /// Push a warning.
    pub fn push(&mut self, w: Warning) {
        self.warnings.push(w);
    }
}

/// A non-fatal warning.
#[derive(Clone, Debug)]
pub struct Warning {
    /// Warning code (e.g., `W001`).
    pub code: &'static str,
    /// Span the warning is attached to.
    pub span: Span,
    /// Message.
    pub message: String,
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}
