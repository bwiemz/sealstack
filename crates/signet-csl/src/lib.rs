//! Context Schema Language (CSL) — parser, type checker, and codegen.
//!
//! The entry point most callers want is [`compile`], which takes a CSL source string
//! (or a multi-file project via [`compile_project`]) and returns a [`CompileOutput`]
//! bundle with every artifact produced by the compiler.
//!
//! ```no_run
//! use signet_csl::{compile, CompileTargets};
//!
//! let src = r#"
//!     schema Note {
//!         id:    Ulid    @primary
//!         title: String  @searchable
//!         body:  Text    @chunked
//!
//!         context {
//!             chunking = semantic(max_tokens = 512)
//!             embedder = "stub"
//!             vector_dims = 64
//!         }
//!     }
//! "#;
//! let out = compile(src, CompileTargets::all()).unwrap();
//! assert!(!out.sql.is_empty());
//! ```
//!
//! The parser is implemented with `winnow`; the type checker is a small bidirectional
//! pass over a closed universe of primitive and user-declared types; codegen is a
//! collection of target-specific visitors.

#![forbid(unsafe_code)]
#![warn(missing_docs, unreachable_pub)]

pub mod ast;
pub mod codegen;
pub mod error;
pub mod parser;
pub mod span;
pub mod types;

pub use crate::ast::*;
pub use crate::error::{CslError, CslResult, Diagnostics};
pub use crate::types::{TypedFile, TypedSchema};

use bitflags::bitflags;

bitflags! {
    /// Which code-generation targets to emit.
    #[derive(Clone, Copy, Debug, Default)]
    pub struct CompileTargets: u32 {
        /// Postgres forward + down migration SQL.
        const SQL         = 0b0000_0001;
        /// Rust type declarations (`#[derive(Serialize, Deserialize)]`).
        const RUST        = 0b0000_0010;
        /// MCP tool descriptors (JSON).
        const MCP         = 0b0000_0100;
        /// Vector-store plan (YAML).
        const VECTOR_PLAN = 0b0000_1000;
        /// TypeScript type declarations.
        const TYPESCRIPT  = 0b0001_0000;
        /// Python (Pydantic v2) models.
        const PYTHON      = 0b0010_0000;
    }
}

impl CompileTargets {
    /// Emit every available target.
    #[must_use]
    pub fn all_targets() -> Self {
        Self::all()
    }
}

/// The bundle of artifacts emitted by [`compile`].
#[derive(Clone, Debug, Default)]
pub struct CompileOutput {
    /// `CREATE TABLE` / index DDL for Postgres. Empty if [`CompileTargets::SQL`] was not set.
    pub sql: String,
    /// Rust type declarations.
    pub rust: String,
    /// MCP tool descriptors, serialized as JSON.
    pub mcp_tools: serde_json::Value,
    /// Vector-store plan, serialized as YAML.
    pub vector_plan: String,
    /// TypeScript type declarations.
    pub typescript: String,
    /// Python Pydantic v2 models.
    pub python: String,
    /// Any diagnostics (warnings) emitted during compilation. Errors abort with an `Err`.
    pub diagnostics: Diagnostics,
    /// One JSON metadata document per compiled schema.
    ///
    /// Shape is compatible with `signet_engine::schema_registry::SchemaMeta`. The
    /// CLI forwards each entry verbatim to `POST /v1/schemas` so the gateway
    /// can hydrate its in-memory registry.
    pub schemas_meta: Vec<serde_json::Value>,
}

/// Compile a single CSL source string into the requested artifact bundle.
///
/// # Errors
///
/// Returns an error if parsing fails, the type checker rejects the file, or codegen
/// encounters an internal contradiction. All error variants carry source spans and
/// render as user-readable diagnostics via `miette`.
pub fn compile(source: &str, targets: CompileTargets) -> CslResult<CompileOutput> {
    let file = parser::parse_file(source)?;
    let typed = types::check(&file)?;
    codegen::emit(&typed, targets)
}

/// Compile a project (multiple files) into a single artifact bundle.
///
/// Each entry is `(filename, source)`. Imports resolve across all supplied files.
///
/// # Errors
///
/// Same failure modes as [`compile`].
pub fn compile_project(
    files: &[(&str, &str)],
    targets: CompileTargets,
) -> CslResult<CompileOutput> {
    let parsed: Vec<_> = files
        .iter()
        .map(|(name, src)| parser::parse_file_named(name, src))
        .collect::<CslResult<_>>()?;
    let typed = types::check_project(&parsed)?;
    codegen::emit(&typed, targets)
}
