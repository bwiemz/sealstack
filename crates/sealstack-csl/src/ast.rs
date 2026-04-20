//! Abstract syntax tree for the Context Schema Language.
//!
//! The AST is closely aligned with the grammar in §2 of the CSL specification.
//! Every node carries a [`Span`] so that downstream passes can produce spanful
//! diagnostics.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::span::Span;

// --- Top-level file -----------------------------------------------------------------

/// A parsed CSL source file.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct File {
    /// Namespace declaration, if present. Absent files live in the anonymous namespace.
    pub namespace: Option<NamespaceDecl>,
    /// `import ... ;` statements, in source order.
    pub imports: Vec<ImportStmt>,
    /// Schema, enum, policy, and context-profile declarations, in source order.
    pub decls: Vec<TopDecl>,
    /// Optional source filename used only for diagnostics.
    pub filename: Option<String>,
    /// The raw source text, retained for span-based error rendering.
    pub source: String,
}

/// `namespace acme.crm ;`
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NamespaceDecl {
    /// Dot-separated path, e.g., `acme.crm`.
    pub path: Path,
    /// Span covering the declaration.
    pub span: Span,
}

/// `import "stdlib/profiles.csl" as profiles ;`
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportStmt {
    /// Quoted path string from the source.
    pub path: String,
    /// Optional alias introduced by `as`.
    pub alias: Option<String>,
    /// Span covering the declaration.
    pub span: Span,
}

/// A top-level declaration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TopDecl {
    /// `schema X { ... }`
    Schema(SchemaDecl),
    /// `enum X { ... }`
    Enum(EnumDecl),
    /// `policy X { ... }` (named reusable policy)
    Policy(PolicyDecl),
    /// `context profile X { ... }`
    ContextProfile(ContextProfileDecl),
}

// --- Identifiers and paths ---------------------------------------------------------

/// Dotted path: `acme.crm.Customer`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Path {
    /// Segments of the path.
    pub segments: Vec<String>,
    /// Span covering the path.
    pub span: Span,
}

impl Path {
    /// Join the path into a single string with `.` separators.
    #[must_use]
    pub fn joined(&self) -> String {
        self.segments.join(".")
    }

    /// Number of segments.
    #[must_use]
    pub fn len(&self) -> usize {
        self.segments.len()
    }

    /// Whether the path is empty (invariant: parser never yields this, but check in `debug_assert`).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }
}

// --- Schema -----------------------------------------------------------------------

/// A schema declaration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SchemaDecl {
    /// Schema name (must begin uppercase).
    pub name: String,
    /// Explicit version number; defaults to 1 if absent.
    pub version: Option<u32>,
    /// Declared fields, in source order.
    pub fields: Vec<FieldDecl>,
    /// Optional `relations { ... }` block.
    pub relations: Vec<RelationDecl>,
    /// Optional `context { ... }` block.
    pub context: Option<ContextBlock>,
    /// Optional `policy { ... }` block.
    pub policy: Option<PolicyBlock>,
    /// Schema-level decorators (e.g., `@audit`, `@retention(7y)`).
    pub decorators: Vec<Decorator>,
    /// Span covering the whole schema.
    pub span: Span,
}

/// A single field in a schema.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FieldDecl {
    /// Field name.
    pub name: String,
    /// Declared type.
    pub ty: TypeExpr,
    /// Decorators attached to the field.
    pub decorators: Vec<Decorator>,
    /// Span covering the field declaration.
    pub span: Span,
}

// --- Types ------------------------------------------------------------------------

/// Type expression.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TypeExpr {
    /// Built-in primitive.
    Primitive(PrimitiveType, Span),
    /// `Ref<T>` — foreign key to entity `T`.
    Ref(String, Span),
    /// `List<T>`.
    List(Box<TypeExpr>, Span),
    /// `Map<K, V>`. Reserved; not yet accepted by the type checker.
    Map(Box<TypeExpr>, Box<TypeExpr>, Span),
    /// User-defined type (enum or schema).
    Named(String, Span),
    /// `T?` — optional wrapper.
    Optional(Box<TypeExpr>, Span),
    /// `Vector<N>`.
    Vector(u32, Span),
}

impl TypeExpr {
    /// Returns the span of this type expression.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::Primitive(_, s)
            | Self::Ref(_, s)
            | Self::List(_, s)
            | Self::Map(_, _, s)
            | Self::Named(_, s)
            | Self::Optional(_, s)
            | Self::Vector(_, s) => *s,
        }
    }

    /// Whether this is `T?` at the outermost level.
    #[must_use]
    pub fn is_optional(&self) -> bool {
        matches!(self, Self::Optional(_, _))
    }
}

/// Primitive types.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[allow(missing_docs)]
pub enum PrimitiveType {
    String,
    Text,
    Ulid,
    Uuid,
    I32,
    I64,
    F32,
    F64,
    Bool,
    Instant,
    Duration,
    Json,
}

impl PrimitiveType {
    /// Resolve a primitive name, case-sensitive. Returns `None` for user-defined types.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "String" => Self::String,
            "Text" => Self::Text,
            "Ulid" => Self::Ulid,
            "Uuid" => Self::Uuid,
            "I32" => Self::I32,
            "I64" => Self::I64,
            "F32" => Self::F32,
            "F64" => Self::F64,
            "Bool" => Self::Bool,
            "Instant" => Self::Instant,
            "Duration" => Self::Duration,
            "Json" => Self::Json,
            _ => return None,
        })
    }

    /// Human-readable name, suitable for diagnostics.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::String => "String",
            Self::Text => "Text",
            Self::Ulid => "Ulid",
            Self::Uuid => "Uuid",
            Self::I32 => "I32",
            Self::I64 => "I64",
            Self::F32 => "F32",
            Self::F64 => "F64",
            Self::Bool => "Bool",
            Self::Instant => "Instant",
            Self::Duration => "Duration",
            Self::Json => "Json",
        }
    }
}

// --- Decorators -------------------------------------------------------------------

/// `@path(arg1, arg2) [= expr]` attached to a field or schema.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Decorator {
    /// Decorator path (e.g., `primary` or `permission.read`).
    pub path: Path,
    /// Positional arguments inside `()`.
    pub args: Vec<Expr>,
    /// Optional `= expr` — used by `@permission.read = (predicate)` shorthand.
    pub assign: Option<Expr>,
    /// Span covering the decorator.
    pub span: Span,
}

impl Decorator {
    /// Whether the decorator path matches the given dotted name.
    #[must_use]
    pub fn is(&self, name: &str) -> bool {
        self.path.joined() == name
    }
}

// --- Relations --------------------------------------------------------------------

/// `tickets: many Ticket via Ticket.customer on_delete cascade`
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RelationDecl {
    /// Relation name on the owning schema.
    pub name: String,
    /// `one` or `many`.
    pub cardinality: Cardinality,
    /// Target schema name.
    pub target: String,
    /// Reverse path — e.g., `Ticket.customer` means "Ticket has a `customer` field pointing back here".
    pub via: Path,
    /// Optional delete cascade policy.
    pub on_delete: Option<DeletePolicy>,
    /// Span covering the relation declaration.
    pub span: Span,
}

/// Relation cardinality.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[allow(missing_docs)]
pub enum Cardinality {
    One,
    Many,
}

/// On-delete behavior for a relation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[allow(missing_docs)]
pub enum DeletePolicy {
    Cascade,
    Restrict,
    SetNull,
}

// --- Context blocks ---------------------------------------------------------------

/// `context { key = value; ... }`
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContextBlock {
    /// Statements inside the block, in source order.
    pub stmts: Vec<ContextStmt>,
    /// Span covering the block.
    pub span: Span,
}

impl ContextBlock {
    /// Flatten statements into an order-preserving map keyed by `key`.
    ///
    /// When the same key appears twice, the last value wins (shadowing).
    #[must_use]
    pub fn as_map(&self) -> IndexMap<String, Expr> {
        let mut m = IndexMap::with_capacity(self.stmts.len());
        for stmt in &self.stmts {
            m.insert(stmt.key.clone(), stmt.value.clone());
        }
        m
    }
}

/// `key = value` inside a context block.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContextStmt {
    /// Statement key.
    pub key: String,
    /// Statement value expression.
    pub value: Expr,
    /// Span covering the statement.
    pub span: Span,
}

// --- Policy -----------------------------------------------------------------------

/// `policy { read | list: expr; write: expr }`
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PolicyBlock {
    /// Rules in declaration order.
    pub rules: Vec<PolicyRule>,
    /// Span covering the block.
    pub span: Span,
}

/// A single `read|write|...: predicate` line.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PolicyRule {
    /// Optional name label (rarely used; reserved for a future syntax).
    pub name: Option<String>,
    /// Actions this rule applies to (union, so `read | list` is two entries).
    pub actions: Vec<Action>,
    /// Predicate expression.
    pub predicate: Expr,
    /// Span covering the rule.
    pub span: Span,
}

/// Actions a policy rule may authorize.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[allow(missing_docs)]
pub enum Action {
    Read,
    List,
    Write,
    Delete,
}

impl Action {
    /// Display name used in diagnostics.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::List => "list",
            Self::Write => "write",
            Self::Delete => "delete",
        }
    }
}

// --- Enums -----------------------------------------------------------------------

/// `enum Tier { Free, Pro, Enterprise }`
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnumDecl {
    /// Enum name.
    pub name: String,
    /// Declared variants.
    pub variants: Vec<EnumVariant>,
    /// Span covering the declaration.
    pub span: Span,
}

/// A variant inside an enum.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnumVariant {
    /// Variant identifier.
    pub name: String,
    /// Optional wire form — `Free("free")` sets `Some("free")`.
    pub wire: Option<String>,
    /// Span covering the variant.
    pub span: Span,
}

/// Named, reusable policy.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PolicyDecl {
    /// Policy name.
    pub name: String,
    /// Rules.
    pub rules: Vec<PolicyRule>,
    /// Span.
    pub span: Span,
}

/// Named context profile.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContextProfileDecl {
    /// Profile name.
    pub name: String,
    /// Statements.
    pub stmts: Vec<ContextStmt>,
    /// Span.
    pub span: Span,
}

// --- Expressions -----------------------------------------------------------------

/// Expression — used in decorator arguments, policy predicates, and context values.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Expr {
    /// Literal value.
    Literal(Literal, Span),
    /// Dotted identifier path.
    Path(Path),
    /// `-x`, `not x`.
    Unary(UnaryOp, Box<Expr>, Span),
    /// Binary operation.
    Binary(BinaryOp, Box<Expr>, Box<Expr>, Span),
    /// `path(arg, arg, ...)`.
    Call(Path, Vec<Expr>, Span),
    /// `[a, b, c]`.
    List(Vec<Expr>, Span),
}

impl Expr {
    /// Span covering this expression.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::Literal(_, s)
            | Self::Unary(_, _, s)
            | Self::Binary(_, _, _, s)
            | Self::Call(_, _, s)
            | Self::List(_, s) => *s,
            Self::Path(p) => p.span,
        }
    }
}

/// Literal values.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[allow(missing_docs)]
pub enum Literal {
    Integer(i64),
    Float(f64),
    String(String),
    Duration(i64, DurationUnit),
    Bool(bool),
    Null,
}

/// Duration units recognized by the lexer.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[allow(missing_docs)]
pub enum DurationUnit {
    Ns,
    Us,
    Ms,
    S,
    M,
    H,
    D,
    W,
    Mo,
    Y,
}

impl DurationUnit {
    /// Source representation of the unit.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ns => "ns",
            Self::Us => "us",
            Self::Ms => "ms",
            Self::S => "s",
            Self::M => "m",
            Self::H => "h",
            Self::D => "d",
            Self::W => "w",
            Self::Mo => "mo",
            Self::Y => "y",
        }
    }
}

/// Unary operators.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[allow(missing_docs)]
pub enum UnaryOp {
    Not,
    Neg,
}

/// Binary operators. Precedence is handled by the parser; this enum just tags the AST node.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[allow(missing_docs)]
pub enum BinaryOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    In,
    NotIn,
    Add,
    Sub,
    Mul,
    Div,
}
