//! `cfg schema` — apply / list / get / validate.

use anyhow::{Context, bail};
use cfg_csl::{CompileTargets, compile as csl_compile};
use serde_json::json;

use crate::cli::{SchemaApplyArgs, SchemaCommand, SchemaGetArgs, SchemaValidateArgs};
use crate::commands::Context as CliContext;
use crate::output;

pub(crate) async fn run(ctx: &CliContext, sub: SchemaCommand) -> anyhow::Result<()> {
    match sub {
        SchemaCommand::Apply(args) => apply(ctx, args).await,
        SchemaCommand::List => list(ctx).await,
        SchemaCommand::Get(args) => get(ctx, args).await,
        SchemaCommand::Validate(args) => validate(ctx, args).await,
    }
}

/// Compile locally, then upload metadata + DDL to the gateway.
async fn apply(ctx: &CliContext, args: SchemaApplyArgs) -> anyhow::Result<()> {
    let src = std::fs::read_to_string(&args.path)
        .with_context(|| format!("read {}", args.path.display()))?;
    let out = csl_compile(&src, CompileTargets::all())?;

    if out.schemas_meta.is_empty() {
        bail!("no schemas compiled from {}", args.path.display());
    }

    let client = ctx.client();

    let mut applied = Vec::new();
    for meta in &out.schemas_meta {
        let qualified = format!(
            "{}.{}",
            meta.get("namespace").and_then(|v| v.as_str()).unwrap_or("default"),
            meta.get("name").and_then(|v| v.as_str()).unwrap_or("Unnamed"),
        );
        tracing::info!(qualified = %qualified, "registering schema");
        client.register_schema(meta.clone()).await?;

        if !out.sql.is_empty() {
            tracing::info!(qualified = %qualified, bytes = out.sql.len(), "applying DDL");
            client.apply_schema_ddl(&qualified, &out.sql).await?;
        }
        applied.push(json!({ "qualified": qualified, "status": "applied" }));
    }

    output::print(ctx.format, &json!({ "applied": applied }));
    Ok(())
}

async fn list(ctx: &CliContext) -> anyhow::Result<()> {
    let client = ctx.client();
    let data = client.list_schemas().await?;
    let schemas = data
        .get("schemas")
        .cloned()
        .unwrap_or_else(|| serde_json::Value::Array(vec![]));
    output::print(ctx.format, &schemas);
    Ok(())
}

async fn get(ctx: &CliContext, args: SchemaGetArgs) -> anyhow::Result<()> {
    let client = ctx.client();
    let data = client.get_schema(&args.qualified).await?;
    output::print(ctx.format, &data);
    Ok(())
}

/// Local-only: compile and report diagnostics without touching the gateway.
async fn validate(ctx: &CliContext, args: SchemaValidateArgs) -> anyhow::Result<()> {
    let src = std::fs::read_to_string(&args.path)
        .with_context(|| format!("read {}", args.path.display()))?;
    match csl_compile(&src, CompileTargets::all()) {
        Ok(out) => {
            let qualified: Vec<String> = out
                .schemas_meta
                .iter()
                .filter_map(|m| {
                    let ns = m.get("namespace")?.as_str()?;
                    let name = m.get("name")?.as_str()?;
                    Some(format!("{ns}.{name}"))
                })
                .collect();
            output::print(
                ctx.format,
                &json!({
                    "status":     "ok",
                    "source":     args.path.display().to_string(),
                    "schemas":    qualified,
                    "sql_bytes":  out.sql.len(),
                    "mcp_tools":  tool_count(&out.mcp_tools),
                }),
            );
            Ok(())
        }
        Err(e) => {
            // The error's `Display` rendering includes span info via miette
            // when the caller renders it with `{:?}`; the CLI prints the
            // short form here and leaves the full rendering to a future
            // `--explain` flag.
            bail!("compilation failed: {e}")
        }
    }
}

fn tool_count(v: &serde_json::Value) -> usize {
    v.get("tools")
        .and_then(|t| t.as_array())
        .map(Vec::len)
        .unwrap_or(0)
}
