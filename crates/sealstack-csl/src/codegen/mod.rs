//! Code-generation dispatch.
//!
//! `emit` looks at the [`CompileTargets`] flags and invokes each target's
//! module-level entry point to populate the corresponding field on
//! [`CompileOutput`].

use crate::error::CslResult;
use crate::types::TypedFile;
use crate::{CompileOutput, CompileTargets};

pub mod mcp;
pub mod policy;
pub mod python;
pub mod rust;
pub mod sql;
pub mod typescript;

/// Emit all requested targets. The MCP and SQL codegen are in this crate;
/// Rust/TypeScript/Python codegen stubs live in sibling modules once implemented.
///
/// # Errors
/// Propagates any error from an individual target generator.
pub fn emit(typed: &TypedFile, targets: CompileTargets) -> CslResult<CompileOutput> {
    let mut out = CompileOutput::default();

    if targets.contains(CompileTargets::SQL) {
        out.sql = sql::emit_sql(typed)?;
    }
    if targets.contains(CompileTargets::MCP) {
        out.mcp_tools = mcp::emit_mcp_descriptors(typed)?;
    }
    if targets.contains(CompileTargets::VECTOR_PLAN) {
        out.vector_plan = emit_vector_plan(typed);
    }
    // Always emit schema metadata — the engine needs it to dispatch any of the
    // above. Gating behind a target flag would break the CLI's schema-apply path.
    out.schemas_meta = emit_schemas_meta(typed);
    if targets.contains(CompileTargets::RUST) {
        out.rust = rust::emit_rust(typed)?;
    }
    if targets.contains(CompileTargets::TYPESCRIPT) {
        out.typescript = typescript::emit_typescript(typed)?;
    }
    if targets.contains(CompileTargets::PYTHON) {
        out.python = python::emit_python(typed)?;
    }
    if targets.contains(CompileTargets::WASM_POLICY) {
        out.policy_bundles = policy::emit_policy_bundles(typed)?;
    }

    Ok(out)
}

fn emit_vector_plan(typed: &TypedFile) -> String {
    let mut yaml = String::from("# Vector store plan (generated from CSL)\ncollections:\n");
    for name in &typed.decl_order {
        if let Some(schema) = typed.schemas.get(name) {
            // Only schemas with a `Vector<N>` field or a `@chunked` text field need a collection.
            let needs_collection = schema
                .decl
                .fields
                .iter()
                .any(|f| matches!(f.ty, crate::ast::TypeExpr::Vector(_, _)))
                || schema
                    .decl
                    .fields
                    .iter()
                    .any(|f| f.decorators.iter().any(|d| d.is("chunked")));
            if !needs_collection {
                continue;
            }
            let version = schema.decl.version.unwrap_or(1);
            yaml.push_str(&format!(
                "  - name: {collection}_{version}\n    schema: {name}\n    version: {version}\n",
                collection = schema.decl.name.to_ascii_lowercase(),
                name = schema.decl.name,
                version = version
            ));
            // Infer dims from the context block, default 1024.
            if let Some(ctx) = &schema.decl.context {
                for stmt in &ctx.stmts {
                    if stmt.key == "vector_dims" {
                        if let crate::ast::Expr::Literal(crate::ast::Literal::Integer(n), _) =
                            &stmt.value
                        {
                            yaml.push_str(&format!("    dims: {n}\n"));
                        }
                    }
                    if stmt.key == "embedder" {
                        if let crate::ast::Expr::Literal(crate::ast::Literal::String(s), _) =
                            &stmt.value
                        {
                            yaml.push_str(&format!("    embedder: {s}\n"));
                        }
                    }
                }
            }
        }
    }
    yaml
}

/// Emit one `SchemaMeta`-shaped JSON document per compiled schema.
///
/// The shape mirrors `sealstack_engine::schema_registry::SchemaMeta`. The CLI
/// forwards each entry to `POST /v1/schemas`, where the gateway deserializes
/// into the engine's strongly-typed struct.
fn emit_schemas_meta(typed: &TypedFile) -> Vec<serde_json::Value> {
    use serde_json::{Map, Value, json};

    let mut out = Vec::new();
    for name in &typed.decl_order {
        let Some(schema) = typed.schemas.get(name) else {
            continue;
        };
        let decl = &schema.decl;
        let version = decl.version.unwrap_or(1);
        let namespace = if typed.namespace.is_empty() {
            "default".to_string()
        } else {
            typed.namespace.clone()
        };

        // Fields.
        let mut primary_key = String::new();
        let mut facets: Vec<String> = Vec::new();
        let mut chunked_fields: Vec<String> = Vec::new();
        let mut fields_json = Vec::new();
        for f in &decl.fields {
            let primary = f.decorators.iter().any(|d| d.is("primary"));
            let indexed = f.decorators.iter().any(|d| d.is("indexed"));
            let searchable = f.decorators.iter().any(|d| d.is("searchable"));
            let chunked = f.decorators.iter().any(|d| d.is("chunked"));
            let facet = f.decorators.iter().any(|d| d.is("facet"));
            let optional = matches!(f.ty, crate::ast::TypeExpr::Optional(_, _));
            let unique = f.decorators.iter().any(|d| d.is("unique"));

            if primary {
                primary_key = f.name.clone();
            }
            if facet {
                facets.push(f.name.clone());
            }
            if chunked {
                chunked_fields.push(f.name.clone());
            }

            fields_json.push(json!({
                "name":       f.name,
                "column":     to_snake(&f.name),
                "ty":         render_type_expr(&f.ty),
                "primary":    primary,
                "indexed":    indexed,
                "searchable": searchable,
                "chunked":    chunked,
                "facet":      facet,
                "optional":   optional,
                "unique":     unique,
                "boost":      boost_value(f),
                "pii":        pii_value(f),
            }));
        }

        // Relations.
        let mut relations = Map::new();
        for rel in &decl.relations {
            let (target_namespace, target_schema) = rel
                .target
                .split_once('.')
                .map_or_else(
                    || (namespace.clone(), rel.target.clone()),
                    |(a, b)| (a.to_owned(), b.to_owned()),
                );
            let kind = match rel.cardinality {
                crate::ast::Cardinality::One => "one",
                crate::ast::Cardinality::Many => "many",
            };
            relations.insert(
                rel.name.clone(),
                json!({
                    "name":             rel.name,
                    "kind":             kind,
                    "target_namespace": target_namespace,
                    "target_schema":    target_schema,
                    "foreign_key":      rel.via.joined(),
                }),
            );
        }

        // Context.
        let context = emit_context_meta(decl);

        let table = to_snake(&decl.name);
        let collection = format!("{}_v{version}", table);

        out.push(json!({
            "namespace":     namespace,
            "name":          decl.name,
            "version":       version,
            "primary_key":   primary_key,
            "fields":        fields_json,
            "relations":     Value::Object(relations),
            "facets":        facets,
            "chunked_fields": chunked_fields,
            "context":       context,
            "collection":    collection,
            "table":         table,
            "hybrid_alpha":  schema_level_hybrid_alpha(decl),
        }));
    }
    out
}

fn emit_context_meta(decl: &crate::ast::SchemaDecl) -> serde_json::Value {
    use serde_json::json;

    let mut embedder = String::from("stub");
    let mut vector_dims: u64 = 64;
    let mut chunking = json!({ "kind": "semantic", "max_tokens": 512, "overlap": 64 });
    let mut freshness_decay = json!({ "kind": "none" });
    let mut default_top_k: Option<u64> = None;

    if let Some(ctx) = &decl.context {
        for stmt in &ctx.stmts {
            match stmt.key.as_str() {
                "embedder" => {
                    if let crate::ast::Expr::Literal(crate::ast::Literal::String(s), _) =
                        &stmt.value
                    {
                        embedder = s.clone();
                    }
                }
                "vector_dims" => {
                    if let crate::ast::Expr::Literal(crate::ast::Literal::Integer(n), _) =
                        &stmt.value
                    {
                        vector_dims = u64::try_from(*n).unwrap_or(64);
                    }
                }
                "default_top_k" => {
                    if let crate::ast::Expr::Literal(crate::ast::Literal::Integer(n), _) =
                        &stmt.value
                    {
                        default_top_k = u64::try_from(*n).ok();
                    }
                }
                "chunking" => {
                    chunking = chunking_to_json(&stmt.value).unwrap_or(chunking);
                }
                "freshness_decay" => {
                    freshness_decay = decay_to_json(&stmt.value).unwrap_or(freshness_decay);
                }
                _ => {}
            }
        }
    }

    json!({
        "embedder":        embedder,
        "vector_dims":     vector_dims,
        "chunking":        chunking,
        "freshness_decay": freshness_decay,
        "default_top_k":   default_top_k,
    })
}

fn chunking_to_json(expr: &crate::ast::Expr) -> Option<serde_json::Value> {
    use serde_json::json;
    if let crate::ast::Expr::Call(name, args, _) = expr {
        let joined = name.joined();
        match joined.as_str() {
            "semantic" => {
                // Positional: semantic(max_tokens, overlap). Kwarg names were
                // discarded by the parser's `call_arg` helper in v0.1.
                let max_tokens = nth_int(args, 0).unwrap_or(512);
                let overlap = nth_int(args, 1).unwrap_or(64);
                return Some(
                    json!({ "kind": "semantic", "max_tokens": max_tokens, "overlap": overlap }),
                );
            }
            "fixed" => {
                let size = nth_int(args, 0).unwrap_or(1024);
                return Some(json!({ "kind": "fixed", "size": size }));
            }
            _ => {}
        }
    }
    None
}

fn decay_to_json(expr: &crate::ast::Expr) -> Option<serde_json::Value> {
    use serde_json::json;
    if let crate::ast::Expr::Call(name, args, _) = expr {
        let joined = name.joined();
        match joined.as_str() {
            "exponential" => {
                let half_life_secs = nth_duration(args, 0).unwrap_or(30 * 24 * 3600);
                return Some(
                    json!({ "kind": "exponential", "half_life_secs": half_life_secs }),
                );
            }
            "linear" => {
                let window_secs = nth_duration(args, 0).unwrap_or(30 * 24 * 3600);
                return Some(json!({ "kind": "linear", "window_secs": window_secs }));
            }
            _ => {}
        }
    }
    None
}

/// Extract the `n`-th integer literal (positional) from a call's argument list.
fn nth_int(args: &[crate::ast::Expr], n: usize) -> Option<i64> {
    args.iter()
        .filter_map(|e| match e {
            crate::ast::Expr::Literal(crate::ast::Literal::Integer(i), _) => Some(*i),
            _ => None,
        })
        .nth(n)
}

/// Extract the `n`-th duration-like literal as seconds.
///
/// Accepts `Duration(v, unit)` and `Integer(v)` (bare integer treated as seconds).
fn nth_duration(args: &[crate::ast::Expr], n: usize) -> Option<u64> {
    args.iter()
        .filter_map(|e| match e {
            crate::ast::Expr::Literal(crate::ast::Literal::Duration(v, unit), _) => {
                Some(duration_to_secs(*v, *unit))
            }
            crate::ast::Expr::Literal(crate::ast::Literal::Integer(i), _) => {
                u64::try_from(*i).ok()
            }
            _ => None,
        })
        .nth(n)
}

fn duration_to_secs(n: i64, unit: crate::ast::DurationUnit) -> u64 {
    use crate::ast::DurationUnit::{D, H, M, Mo, Ms, Ns, S, Us, W, Y};
    let mult: u64 = match unit {
        // Sub-second units round down to 0 seconds for policy / freshness purposes.
        Ns | Us | Ms => return 0,
        S => 1,
        M => 60,
        H => 3_600,
        D => 86_400,
        W => 7 * 86_400,
        Mo => 30 * 86_400,
        Y => 365 * 86_400,
    };
    u64::try_from(n.max(0))
        .ok()
        .and_then(|v| v.checked_mul(mult))
        .unwrap_or(u64::MAX)
}

fn boost_value(field: &crate::ast::FieldDecl) -> Option<f64> {
    for d in &field.decorators {
        if d.is("boost") {
            for a in &d.args {
                if let crate::ast::Expr::Literal(crate::ast::Literal::Float(f), _) = a {
                    return Some(*f);
                }
                if let crate::ast::Expr::Literal(crate::ast::Literal::Integer(n), _) = a {
                    #[allow(clippy::cast_precision_loss)]
                    return Some(*n as f64);
                }
            }
        }
    }
    None
}

fn pii_value(field: &crate::ast::FieldDecl) -> Option<String> {
    for d in &field.decorators {
        if d.is("pii") {
            if let Some(a) = d.args.first() {
                if let crate::ast::Expr::Literal(crate::ast::Literal::String(s), _) = a {
                    return Some(s.clone());
                }
            }
            return Some("unspecified".to_string());
        }
    }
    None
}

fn schema_level_hybrid_alpha(decl: &crate::ast::SchemaDecl) -> Option<f64> {
    if let Some(ctx) = &decl.context {
        for stmt in &ctx.stmts {
            if stmt.key == "hybrid_alpha" {
                if let crate::ast::Expr::Literal(crate::ast::Literal::Float(f), _) = &stmt.value {
                    return Some(*f);
                }
            }
        }
    }
    None
}

fn render_type_expr(ty: &crate::ast::TypeExpr) -> String {
    use crate::ast::TypeExpr::{List, Map, Named, Optional, Primitive, Ref, Vector};
    match ty {
        Primitive(p, _) => format!("{p:?}"),
        Ref(target, _) => format!("Ref<{target}>"),
        List(inner, _) => format!("List<{}>", render_type_expr(inner)),
        Map(k, v, _) => format!("Map<{},{}>", render_type_expr(k), render_type_expr(v)),
        Named(name, _) => name.clone(),
        Optional(inner, _) => format!("{}?", render_type_expr(inner)),
        Vector(n, _) => format!("Vector<{n}>"),
    }
}

pub(super) fn to_snake(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for (i, c) in s.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}
