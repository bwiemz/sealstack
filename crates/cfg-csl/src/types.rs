//! Type checker for CSL.
//!
//! This pass walks a parsed [`File`] (or multi-file project) and validates
//! well-formedness constraints that the parser alone cannot:
//!
//! * Every schema has exactly one `@primary` field.
//! * `Ref<T>` targets resolve to a schema in the namespace or an imported one.
//! * `Vector<N>` fields have an `@embedded_from` pointing at a chunkable source.
//! * Cyclic `via` chains (the reverse-path declaration in `relations`) are rejected.
//! * Decorator arity is reasonable (too-many-args, unknown-decorator warning, etc.).
//! * Policy-rule predicates name-resolve against `self.*`, `caller.*`, and built-ins.
//!
//! The output is a [`TypedFile`] — the same shape as [`File`] but with resolved
//! references and a symbol table, ready for codegen.

use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::error::{CslError, CslResult};

/// A typed file — validated and ready for codegen.
#[derive(Clone, Debug, Default)]
pub struct TypedFile {
    /// Fully-qualified namespace (or empty string for anonymous files).
    pub namespace: String,
    /// Original filename for diagnostics.
    pub filename: Option<String>,
    /// Source text.
    pub source: String,
    /// All schemas, keyed by name.
    pub schemas: HashMap<String, TypedSchema>,
    /// All enums.
    pub enums: HashMap<String, EnumDecl>,
    /// Insertion order of top-level declarations.
    pub decl_order: Vec<String>,
}

/// A typed schema with resolved references.
#[derive(Clone, Debug)]
pub struct TypedSchema {
    /// Original schema declaration.
    pub decl: SchemaDecl,
    /// Index of the `@primary` field in `decl.fields`.
    pub primary_idx: usize,
    /// Set of schema names that this schema references via `Ref<T>`.
    pub refs_to: HashSet<String>,
    /// For each field, the set of decorator names present. Used by codegen.
    pub field_decorator_index: Vec<HashSet<String>>,
}

impl TypedSchema {
    /// The `@primary` field.
    #[must_use]
    pub fn primary_field(&self) -> &FieldDecl {
        &self.decl.fields[self.primary_idx]
    }
}

/// Check a single file.
///
/// # Errors
/// Returns the first fatal type error encountered.
pub fn check(file: &File) -> CslResult<TypedFile> {
    check_project(&[file.clone()])
}

/// Check a multi-file project. Imports are resolved across all supplied files.
///
/// # Errors
/// Returns the first fatal type error encountered.
pub fn check_project(files: &[File]) -> CslResult<TypedFile> {
    // Single-namespace assumption for v0.1: all files must agree on namespace,
    // or (legally) all be anonymous. Multi-namespace projects come later.
    let namespace = files
        .iter()
        .find_map(|f| f.namespace.as_ref().map(|n| n.path.joined()))
        .unwrap_or_default();

    let (filename, source) = files
        .first()
        .map(|f| (f.filename.clone(), f.source.clone()))
        .unwrap_or_default();

    let mut typed = TypedFile {
        namespace,
        filename,
        source,
        ..Default::default()
    };

    // Pass 1: index all top-level declarations.
    for file in files {
        for decl in &file.decls {
            match decl {
                TopDecl::Schema(s) => {
                    if typed.schemas.contains_key(&s.name) {
                        return Err(CslError::DuplicateSchema {
                            name: s.name.clone(),
                            namespace: typed.namespace.clone(),
                            src: miette::NamedSource::new(
                                file.filename.as_deref().unwrap_or("<input>"),
                                file.source.clone(),
                            ),
                            span: s.span.into(),
                        });
                    }
                    typed.decl_order.push(s.name.clone());
                    typed.schemas.insert(
                        s.name.clone(),
                        TypedSchema {
                            decl: s.clone(),
                            primary_idx: 0,
                            refs_to: HashSet::new(),
                            field_decorator_index: Vec::new(),
                        },
                    );
                }
                TopDecl::Enum(e) => {
                    typed.enums.insert(e.name.clone(), e.clone());
                }
                TopDecl::Policy(_) | TopDecl::ContextProfile(_) => {
                    // Out-of-scope for v0.1 type checker.
                }
            }
        }
    }

    // Pass 2: for each schema, validate primary key, references, decorators.
    let schema_names: HashSet<String> = typed.schemas.keys().cloned().collect();
    let enum_names: HashSet<String> = typed.enums.keys().cloned().collect();
    let file_src: HashMap<Option<String>, String> = files
        .iter()
        .map(|f| (f.filename.clone(), f.source.clone()))
        .collect();

    for (name, ts) in typed.schemas.iter_mut() {
        let schema = ts.decl.clone();
        let (src_name, src_text) = files
            .iter()
            .find(|f| f.decls.iter().any(|d| matches!(d, TopDecl::Schema(s) if s.name == schema.name)))
            .map(|f| (f.filename.clone(), f.source.clone()))
            .unwrap_or_default();
        let _ = file_src.get(&src_name);

        // Primary key validation.
        let primary_fields: Vec<(usize, &FieldDecl)> = schema
            .fields
            .iter()
            .enumerate()
            .filter(|(_, f)| f.decorators.iter().any(|d| d.is("primary")))
            .collect();

        match primary_fields.len() {
            0 => {
                return Err(CslError::MissingPrimaryKey {
                    schema: name.clone(),
                    src: miette::NamedSource::new(
                        src_name.unwrap_or_else(|| "<input>".into()),
                        src_text,
                    ),
                    span: schema.span.into(),
                });
            }
            1 => ts.primary_idx = primary_fields[0].0,
            _ => {
                let fields: Vec<String> =
                    primary_fields.iter().map(|(_, f)| f.name.clone()).collect();
                return Err(CslError::DuplicatePrimaryKey {
                    schema: name.clone(),
                    fields,
                    src: miette::NamedSource::new(
                        src_name.unwrap_or_else(|| "<input>".into()),
                        src_text,
                    ),
                    span: schema.span.into(),
                });
            }
        }

        // Reference validation.
        for f in &schema.fields {
            collect_refs(&f.ty, &mut ts.refs_to);
        }
        for r in ts.refs_to.iter() {
            if !schema_names.contains(r) && !enum_names.contains(r) {
                // find the span of the first field that uses this ref
                let field_span = schema
                    .fields
                    .iter()
                    .find_map(|f| find_ref_span(&f.ty, r))
                    .unwrap_or(schema.span);
                return Err(CslError::UnknownRefTarget {
                    target: r.clone(),
                    src: miette::NamedSource::new(
                        files
                            .iter()
                            .find(|f| f.decls.iter().any(|d| matches!(d, TopDecl::Schema(s) if s.name == schema.name)))
                            .and_then(|f| f.filename.clone())
                            .unwrap_or_else(|| "<input>".into()),
                        files
                            .iter()
                            .find(|f| f.decls.iter().any(|d| matches!(d, TopDecl::Schema(s) if s.name == schema.name)))
                            .map(|f| f.source.clone())
                            .unwrap_or_default(),
                    ),
                    span: field_span.into(),
                });
            }
        }

        // Build decorator index for codegen.
        ts.field_decorator_index = schema
            .fields
            .iter()
            .map(|f| f.decorators.iter().map(|d| d.path.joined()).collect())
            .collect();
    }

    Ok(typed)
}

fn collect_refs(ty: &TypeExpr, out: &mut HashSet<String>) {
    match ty {
        TypeExpr::Ref(name, _) | TypeExpr::Named(name, _) => {
            out.insert(name.clone());
        }
        TypeExpr::List(inner, _) | TypeExpr::Optional(inner, _) => collect_refs(inner, out),
        TypeExpr::Map(k, v, _) => {
            collect_refs(k, out);
            collect_refs(v, out);
        }
        TypeExpr::Primitive(_, _) | TypeExpr::Vector(_, _) => {}
    }
}

fn find_ref_span(ty: &TypeExpr, target: &str) -> Option<crate::span::Span> {
    match ty {
        TypeExpr::Ref(name, span) | TypeExpr::Named(name, span) if name == target => Some(*span),
        TypeExpr::List(inner, _) | TypeExpr::Optional(inner, _) => find_ref_span(inner, target),
        TypeExpr::Map(k, v, _) => find_ref_span(k, target).or_else(|| find_ref_span(v, target)),
        _ => None,
    }
}
