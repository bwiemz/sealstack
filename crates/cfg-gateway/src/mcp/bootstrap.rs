//! Boot-time MCP tool registration from hydrated `SchemaMeta`.
//!
//! The canonical tool descriptors are emitted at CSL-compile time into
//! `out/mcp/*.json`. The gateway process, however, needs to stand up a working
//! `ToolRegistry` even when the compile directory is empty or out of sync with
//! the live `cfg_schemas` table — for example, after a restart where schemas
//! were registered via `POST /v1/schemas`.
//!
//! This module produces a minimal-but-functional `ToolDescriptor` directly from
//! `SchemaMeta`. The descriptors advertise the right tool names and the
//! argument shapes the `GeneratedHandler` dispatcher expects; callers that need
//! full JSON-Schema validation should still consume the compile-time artifacts.

use std::sync::Arc;

use cfg_engine::facade::EngineFacade;
use cfg_engine::schema_registry::{RelationKind, SchemaMeta};
use serde_json::json;

use super::handlers::{GeneratedHandler, HandlerKind};
use super::registry::ToolRegistry;
use super::types::ToolDescriptor;

const SERVER_NAME: &str = "contextforge";

/// Register every default tool for every schema currently in `registry`.
pub fn register_all(
    tools: &ToolRegistry,
    engine: Arc<dyn EngineFacade>,
    schemas: &[Arc<SchemaMeta>],
) -> usize {
    let mut count = 0;
    for meta in schemas {
        count += register_schema_tools(tools, engine.clone(), meta);
    }
    tracing::info!(count, "registered MCP tools");
    count
}

/// Register the default `search/get/list/...` tools for one schema.
pub fn register_schema_tools(
    tools: &ToolRegistry,
    engine: Arc<dyn EngineFacade>,
    meta: &SchemaMeta,
) -> usize {
    // `qualified` is used for engine dispatch and keeps the real dotted form
    // (e.g. `acme.crm.Customer`). `tool_slug` is used only to build MCP tool
    // names, which the spec restricts to `^[a-zA-Z0-9_-]{1,64}$` — so dots in
    // the namespace get rewritten to underscores.
    let qualified = format!("{}.{}", meta.namespace, meta.name);
    let tool_slug = slugify_for_tool_name(&qualified);
    let mut n = 0;

    tools.register(
        SERVER_NAME,
        Arc::new(GeneratedHandler {
            descriptor: search_descriptor(&tool_slug, &qualified),
            kind: HandlerKind::Search,
            schema: qualified.clone(),
            relation: None,
            facet: None,
            engine: engine.clone(),
        }),
    );
    n += 1;

    tools.register(
        SERVER_NAME,
        Arc::new(GeneratedHandler {
            descriptor: get_descriptor(&tool_slug, &qualified),
            kind: HandlerKind::Get,
            schema: qualified.clone(),
            relation: None,
            facet: None,
            engine: engine.clone(),
        }),
    );
    n += 1;

    tools.register(
        SERVER_NAME,
        Arc::new(GeneratedHandler {
            descriptor: list_descriptor(&tool_slug, &qualified),
            kind: HandlerKind::List,
            schema: qualified.clone(),
            relation: None,
            facet: None,
            engine: engine.clone(),
        }),
    );
    n += 1;

    for (rel_name, rel) in &meta.relations {
        if rel.kind != RelationKind::Many {
            continue;
        }
        tools.register(
            SERVER_NAME,
            Arc::new(GeneratedHandler {
                descriptor: list_relation_descriptor(&tool_slug, &qualified, rel_name),
                kind: HandlerKind::ListRelation,
                schema: qualified.clone(),
                relation: Some(rel_name.clone()),
                facet: None,
                engine: engine.clone(),
            }),
        );
        n += 1;
    }

    for facet in &meta.facets {
        tools.register(
            SERVER_NAME,
            Arc::new(GeneratedHandler {
                descriptor: aggregate_descriptor(&tool_slug, &qualified, facet),
                kind: HandlerKind::Aggregate,
                schema: qualified.clone(),
                relation: None,
                facet: Some(facet.clone()),
                engine: engine.clone(),
            }),
        );
        n += 1;
    }

    n
}

/// Rewrite a qualified schema name into a legal MCP tool-name token.
///
/// MCP 2025-11-25 restricts tool names to `^[a-zA-Z0-9_-]{1,64}$`. The only
/// character our schema names can introduce that violates that is `.`, which
/// we map to `_`. Anything else non-matching is also mapped to `_` as a
/// conservative fallback.
fn slugify_for_tool_name(qualified: &str) -> String {
    qualified
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect()
}

fn search_descriptor(slug: &str, qualified: &str) -> ToolDescriptor {
    ToolDescriptor {
        name: format!("search_{slug}"),
        title: Some(format!("Search {qualified}")),
        description: format!("Hybrid BM25 + vector search over {qualified}."),
        input_schema: json!({
            "type": "object",
            "properties": {
                "query":   { "type": "string" },
                "top_k":   { "type": "integer", "minimum": 1, "maximum": 200 },
                "filters": { "type": "object" }
            },
            "required": ["query"],
            "additionalProperties": false
        }),
        output_schema: None,
        annotations: None,
    }
}

fn get_descriptor(slug: &str, qualified: &str) -> ToolDescriptor {
    ToolDescriptor {
        name: format!("get_{slug}"),
        title: Some(format!("Get {qualified} by id")),
        description: format!("Fetch a single {qualified} record by primary key."),
        input_schema: json!({
            "type": "object",
            "properties": { "id": {} },
            "required": ["id"],
            "additionalProperties": false
        }),
        output_schema: None,
        annotations: None,
    }
}

fn list_descriptor(slug: &str, qualified: &str) -> ToolDescriptor {
    ToolDescriptor {
        name: format!("list_{slug}"),
        title: Some(format!("List {qualified}")),
        description: format!("List {qualified} records with optional filters and cursor pagination."),
        input_schema: json!({
            "type": "object",
            "properties": {
                "filters":   { "type": "object" },
                "limit":     { "type": "integer", "minimum": 1, "maximum": 500 },
                "cursor":    { "type": "string" },
                "order_by":  { "type": "string" },
                "direction": { "type": "string", "enum": ["asc", "desc"] }
            },
            "additionalProperties": false
        }),
        output_schema: None,
        annotations: None,
    }
}

fn list_relation_descriptor(slug: &str, qualified: &str, relation: &str) -> ToolDescriptor {
    let rel_slug = slugify_for_tool_name(relation);
    ToolDescriptor {
        name: format!("list_{slug}_{rel_slug}"),
        title: Some(format!("List {relation} of {qualified}")),
        description: format!("List the `{relation}` related records for a parent {qualified}."),
        input_schema: json!({
            "type": "object",
            "properties": {
                "parent_id": {},
                "limit":     { "type": "integer", "minimum": 1, "maximum": 500 },
                "cursor":    { "type": "string" }
            },
            "required": ["parent_id"],
            "additionalProperties": false
        }),
        output_schema: None,
        annotations: None,
    }
}

#[cfg(test)]
mod tests {
    use super::slugify_for_tool_name;

    #[test]
    fn slug_rewrites_dots_and_preserves_legal_chars() {
        assert_eq!(slugify_for_tool_name("acme.crm.Customer"), "acme_crm_Customer");
        assert_eq!(slugify_for_tool_name("Doc"), "Doc");
        assert_eq!(slugify_for_tool_name("my-schema_v2"), "my-schema_v2");
        assert_eq!(slugify_for_tool_name("weird name!"), "weird_name_");
    }
}

fn aggregate_descriptor(slug: &str, qualified: &str, facet: &str) -> ToolDescriptor {
    let facet_slug = slugify_for_tool_name(facet);
    ToolDescriptor {
        name: format!("aggregate_{slug}_{facet_slug}"),
        title: Some(format!("Aggregate {qualified} by {facet}")),
        description: format!("Facet histogram over `{facet}` on {qualified}."),
        input_schema: json!({
            "type": "object",
            "properties": {
                "filters": { "type": "object" },
                "buckets": { "type": "integer", "minimum": 1, "maximum": 1000 }
            },
            "additionalProperties": false
        }),
        output_schema: None,
        annotations: None,
    }
}
