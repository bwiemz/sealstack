//! MCP tool-descriptor code generation.
//!
//! For every typed schema `S`, this module emits:
//!
//! * A **server descriptor** — a manifest entry pointing to the MCP HTTP endpoint
//!   that will serve the schema, plus its metadata (auth, rate limits, capabilities).
//!
//! * A **tool set** — concrete MCP tool definitions with input and output JSON Schemas:
//!
//!     * `search_<s>`             — hybrid (vector + keyword) search over chunks.
//!     * `get_<s>`                — fetch a single record by primary key.
//!     * `list_<s>`               — paginated listing, constrained by `@facet` filters.
//!     * `list_<s>_<rel>`         — walk `many` relations declared in the schema.
//!     * `aggregate_<s>_<facet>`  — simple counts/histograms over `@facet` fields.
//!
//! The output of this module is the **static contract**. The runtime dispatcher
//! (see `sealstack-gateway/src/mcp`) consumes this contract at boot and binds each tool
//! to a concrete handler implementation.

use serde_json::{Value, json};

use crate::ast::{Cardinality, Expr, FieldDecl, Literal, PrimitiveType, TypeExpr};
use crate::error::CslResult;
use crate::types::{TypedFile, TypedSchema};

/// Emit a JSON manifest of all MCP servers and tools produced from the typed file.
///
/// The returned structure is **not** the JSON-RPC wire format MCP clients see at
/// runtime — that is produced by the gateway's protocol handler in response to
/// `tools/list`. Rather, this is the static design-time manifest used by the
/// gateway to register handlers and by documentation tools to render references.
///
/// # Errors
/// Currently never returns an error; signature kept fallible for future growth.
pub fn emit_mcp_descriptors(typed: &TypedFile) -> CslResult<Value> {
    let mut servers = Vec::with_capacity(typed.schemas.len());

    for name in &typed.decl_order {
        let Some(schema) = typed.schemas.get(name) else {
            continue;
        };

        let server_name = qualified_name(&typed.namespace, &schema.decl.name);
        let tools = generate_tools(schema);
        let uri = format!(
            "/mcp/{}",
            if typed.namespace.is_empty() {
                schema.decl.name.to_ascii_lowercase()
            } else {
                format!("{}.{}", typed.namespace, schema.decl.name.to_ascii_lowercase())
            }
        );

        servers.push(json!({
            "name":         server_name,
            "uri":          uri,
            "description":  format!("Context tools for schema `{}`", schema.decl.name),
            "capabilities": {
                "tools":     { "listChanged": true },
                "resources": { "subscribe":   true, "listChanged": true }
            },
            "auth": {
                "type":   "oauth2.1",
                "scopes": ["context.read", "context.search"]
            },
            "tools":     tools,
            "resources": generate_resources(schema)
        }));
    }

    Ok(json!({
        "$schema": "https://modelcontextprotocol.io/schemas/server-manifest-2025-11.json",
        "servers": servers
    }))
}

fn qualified_name(namespace: &str, schema_name: &str) -> String {
    if namespace.is_empty() {
        schema_name.to_string()
    } else {
        format!("{namespace}.{schema_name}")
    }
}

fn snake(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() && i != 0 {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

// --- Tool generation ------------------------------------------------------------------

fn generate_tools(schema: &TypedSchema) -> Vec<Value> {
    let mut tools = Vec::new();
    let name = &schema.decl.name;
    let snake_name = snake(name);

    // Always-emitted core tools.
    tools.push(tool_search(schema, &snake_name));
    tools.push(tool_get(schema, &snake_name));
    tools.push(tool_list(schema, &snake_name));

    // Relation walkers for `many` relations.
    for r in &schema.decl.relations {
        if r.cardinality == Cardinality::Many {
            tools.push(tool_list_relation(schema, &snake_name, &r.name, &r.target));
        }
    }

    // Aggregates over every `@facet` field.
    for (idx, f) in schema.decl.fields.iter().enumerate() {
        if schema.field_decorator_index[idx].contains("facet") {
            tools.push(tool_aggregate(&snake_name, f));
        }
    }

    tools
}

fn tool_search(schema: &TypedSchema, snake_name: &str) -> Value {
    let filter_properties = facet_filter_properties(schema);

    let input_schema = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "query": {
                "type":        "string",
                "minLength":   1,
                "maxLength":   2048,
                "description": "Natural-language query. Hybrid vector + BM25 search is used."
            },
            "top_k": {
                "type":        "integer",
                "minimum":     1,
                "maximum":     100,
                "default":     default_top_k(schema).unwrap_or(12),
                "description": "Maximum number of results to return."
            },
            "filters": {
                "type":        "object",
                "description": "Optional facet filters.",
                "additionalProperties": false,
                "properties":  filter_properties
            },
            "freshness": {
                "type":        "string",
                "enum":        ["any", "fresh", "stale_ok"],
                "default":     "any",
                "description": "Controls inclusion of stale chunks per the schema's freshness_decay."
            }
        },
        "required": ["query"]
    });

    json!({
        "name":         format!("search_{snake_name}"),
        "title":        format!("Search {}", schema.decl.name),
        "description":  format!(
            "Hybrid retrieval over `{}` records. Respects caller permissions declared in the schema `policy` block.",
            schema.decl.name
        ),
        "inputSchema":  input_schema,
        "outputSchema": search_result_schema(schema),
        "annotations": {
            "readOnly":       true,
            "idempotent":     true,
            "category":       "retrieval"
        }
    })
}

fn tool_get(schema: &TypedSchema, snake_name: &str) -> Value {
    let pk = schema.primary_field();
    let pk_json_type = json_type_for(&pk.ty);

    let input_schema = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "id": {
                "type":        pk_json_type,
                "description": format!("The `{}` field.", pk.name)
            }
        },
        "required": ["id"]
    });

    json!({
        "name":         format!("get_{snake_name}"),
        "title":        format!("Get {}", schema.decl.name),
        "description":  format!("Fetch a single `{}` record by primary key.", schema.decl.name),
        "inputSchema":  input_schema,
        "outputSchema": record_schema(schema),
        "annotations":  { "readOnly": true, "idempotent": true, "category": "retrieval" }
    })
}

fn tool_list(schema: &TypedSchema, snake_name: &str) -> Value {
    let filter_properties = facet_filter_properties(schema);
    let input_schema = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "filters":   { "type": "object", "additionalProperties": false, "properties": filter_properties },
            "limit":     { "type": "integer", "minimum": 1, "maximum": 100, "default": 20 },
            "cursor":    { "type": "string", "description": "Opaque pagination cursor." },
            "order_by":  { "type": "string", "description": "Field name to order by; must be @indexed or @primary." },
            "direction": { "type": "string", "enum": ["asc", "desc"], "default": "desc" }
        }
    });
    json!({
        "name":         format!("list_{snake_name}"),
        "title":        format!("List {}", schema.decl.name),
        "description":  format!("Paginated, permission-filtered list of `{}`.", schema.decl.name),
        "inputSchema":  input_schema,
        "outputSchema": paged_result_schema(schema),
        "annotations":  { "readOnly": true, "idempotent": true, "category": "retrieval" }
    })
}

fn tool_list_relation(schema: &TypedSchema, owner_snake: &str, relation: &str, target: &str) -> Value {
    let pk = schema.primary_field();
    let pk_json_type = json_type_for(&pk.ty);

    let input_schema = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "parent_id": { "type": pk_json_type, "description": format!("The {} being queried.", schema.decl.name) },
            "limit":     { "type": "integer", "minimum": 1, "maximum": 100, "default": 20 },
            "cursor":    { "type": "string" }
        },
        "required": ["parent_id"]
    });

    json!({
        "name":         format!("list_{owner_snake}_{relation}"),
        "title":        format!("List {} of {}", target, schema.decl.name),
        "description":  format!(
            "Walk the `{relation}` relation from `{}` to `{target}`.",
            schema.decl.name
        ),
        "inputSchema":  input_schema,
        "outputSchema": json!({
            "type": "object",
            "properties": {
                "items":       { "type": "array",  "items": { "type": "object" } },
                "next_cursor": { "type": "string" }
            }
        }),
        "annotations":  { "readOnly": true, "idempotent": true, "category": "relations" }
    })
}

fn tool_aggregate(snake_name: &str, f: &FieldDecl) -> Value {
    let facet_type = json_type_for(&f.ty);
    let agg_kind = match facet_type {
        Value::String(ref t) if t == "string"  => "histogram",
        Value::String(ref t) if t == "integer" => "histogram",
        Value::String(ref t) if t == "number"  => "bucket_histogram",
        _ => "histogram",
    };
    json!({
        "name":         format!("aggregate_{snake_name}_{}", f.name),
        "title":        format!("Aggregate by {}", f.name),
        "description":  format!("Counts over the `{}` facet. Returns a {agg_kind}.", f.name),
        "inputSchema":  json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "filters": { "type": "object", "additionalProperties": true },
                "buckets": { "type": "integer", "minimum": 1, "maximum": 500, "default": 20 }
            }
        }),
        "outputSchema": json!({
            "type": "object",
            "properties": {
                "facet":   { "type": "string" },
                "buckets": { "type": "array", "items": { "type": "object" } }
            }
        }),
        "annotations": { "readOnly": true, "idempotent": true, "category": "analytics" }
    })
}

// --- Resources (MCP concept: addressable read-only things) ---------------------------

fn generate_resources(schema: &TypedSchema) -> Value {
    // For v0.1, we publish a single resource template per schema: the record by ID.
    json!([
        {
            "uriTemplate": format!("context://{}/{{id}}", snake(&schema.decl.name)),
            "name":        format!("{} record", schema.decl.name),
            "description": format!(
                "A single `{}` record, returned as JSON. Subject to the schema's read policy.",
                schema.decl.name
            ),
            "mimeType": "application/json"
        }
    ])
}

// --- JSON Schema helpers --------------------------------------------------------------

/// Map a CSL type to a JSON Schema `type` string.
pub fn json_type_for(ty: &TypeExpr) -> Value {
    fn prim(p: PrimitiveType) -> &'static str {
        match p {
            PrimitiveType::String
            | PrimitiveType::Text
            | PrimitiveType::Ulid
            | PrimitiveType::Uuid
            | PrimitiveType::Instant
            | PrimitiveType::Duration => "string",
            PrimitiveType::I32 | PrimitiveType::I64 => "integer",
            PrimitiveType::F32 | PrimitiveType::F64 => "number",
            PrimitiveType::Bool => "boolean",
            PrimitiveType::Json => "object",
        }
    }
    match ty {
        TypeExpr::Optional(inner, _) => json_type_for(inner),
        TypeExpr::Primitive(p, _) => Value::String(prim(*p).to_owned()),
        TypeExpr::Ref(_, _) => Value::String("string".into()),
        TypeExpr::List(_, _) => Value::String("array".into()),
        TypeExpr::Map(_, _, _) => Value::String("object".into()),
        TypeExpr::Named(_, _) => Value::String("string".into()),
        TypeExpr::Vector(n, _) => json!({ "type": "array", "items": { "type": "number" }, "minItems": n, "maxItems": n }),
    }
}

/// Build the `filters` property map from any `@facet` fields on the schema.
fn facet_filter_properties(schema: &TypedSchema) -> Value {
    let mut props = serde_json::Map::new();
    for (idx, f) in schema.decl.fields.iter().enumerate() {
        if schema.field_decorator_index[idx].contains("facet") {
            props.insert(
                f.name.clone(),
                json!({
                    "oneOf": [
                        json_type_for(&f.ty),
                        { "type": "array", "items": json_type_for(&f.ty) }
                    ],
                    "description": format!("Exact-match filter on `{}`; array matches any.", f.name)
                }),
            );
        }
    }
    Value::Object(props)
}

/// JSON Schema representing a single record shaped by `schema`.
fn record_schema(schema: &TypedSchema) -> Value {
    let mut props = serde_json::Map::new();
    let mut required = Vec::new();
    for f in &schema.decl.fields {
        props.insert(f.name.clone(), field_schema(f));
        if !f.ty.is_optional() {
            required.push(Value::String(f.name.clone()));
        }
    }
    json!({
        "type":     "object",
        "properties": props,
        "required": required
    })
}

/// JSON Schema for a single field, including description pulled from decorators.
fn field_schema(f: &FieldDecl) -> Value {
    let base = json_type_for(&f.ty);
    let description = field_description(f);
    match base {
        Value::Object(mut m) => {
            if let Some(desc) = description {
                m.insert("description".into(), Value::String(desc));
            }
            Value::Object(m)
        }
        Value::String(t) => {
            let mut m = serde_json::Map::new();
            m.insert("type".into(), Value::String(t));
            if let Some(desc) = description {
                m.insert("description".into(), Value::String(desc));
            }
            Value::Object(m)
        }
        other => other,
    }
}

fn field_description(f: &FieldDecl) -> Option<String> {
    // Pull a best-effort description from @description or first string arg of @doc.
    for d in &f.decorators {
        if d.is("description") || d.is("doc") {
            if let Some(Expr::Literal(Literal::String(s), _)) = d.args.first() {
                return Some(s.clone());
            }
        }
    }
    None
}

/// Wrap a record schema in a search-hit envelope.
fn search_result_schema(schema: &TypedSchema) -> Value {
    json!({
        "type": "object",
        "properties": {
            "results": {
                "type":  "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "record":     record_schema(schema),
                        "score":      { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "highlights": { "type": "array",  "items": { "type": "string" } },
                        "freshness":  { "type": "string", "enum": ["fresh", "stale", "unknown"] }
                    },
                    "required": ["record", "score"]
                }
            },
            "receipt_id": { "type": "string", "description": "ID of the grounded-answer receipt for audit." },
            "took_ms":    { "type": "integer" }
        },
        "required": ["results", "receipt_id"]
    })
}

/// Wrap a record schema in a paginated envelope.
fn paged_result_schema(schema: &TypedSchema) -> Value {
    json!({
        "type": "object",
        "properties": {
            "items":       { "type": "array", "items": record_schema(schema) },
            "next_cursor": { "type": "string" },
            "total_est":   { "type": "integer" }
        },
        "required": ["items"]
    })
}

/// Read `default_top_k` out of the schema's `context` block if present.
fn default_top_k(schema: &TypedSchema) -> Option<u64> {
    let ctx = schema.decl.context.as_ref()?;
    for stmt in &ctx.stmts {
        if stmt.key == "default_top_k" {
            if let Expr::Literal(Literal::Integer(n), _) = &stmt.value {
                return u64::try_from(*n).ok();
            }
        }
    }
    None
}

// --- Tests -------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CompileTargets, compile};

    #[test]
    fn emits_server_descriptor_for_note() {
        let src = r#"
            schema Note {
                id:    Ulid   @primary
                title: String @searchable
                body:  Text   @chunked

                context {
                    chunking    = semantic(max_tokens = 512)
                    embedder    = "stub"
                    vector_dims = 64
                }
            }
        "#;
        let out = compile(src, CompileTargets::MCP).expect("compile ok");
        let manifest = out.mcp_tools;
        let servers = manifest.get("servers").and_then(Value::as_array).unwrap();
        assert_eq!(servers.len(), 1);
        let s = &servers[0];
        assert_eq!(s.get("name").unwrap().as_str().unwrap(), "Note");
        let tools = s.get("tools").unwrap().as_array().unwrap();
        let names: Vec<_> = tools
            .iter()
            .map(|t| t.get("name").unwrap().as_str().unwrap())
            .collect();
        assert!(names.contains(&"search_note"));
        assert!(names.contains(&"get_note"));
        assert!(names.contains(&"list_note"));
    }
}
