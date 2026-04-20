//! `sealstack connector` — add / list / sync.

use anyhow::{Context, bail};
use serde_json::{Value, json};

use crate::cli::{ConnectorAddArgs, ConnectorCommand, ConnectorSyncArgs};
use crate::commands::Context as CliContext;
use crate::output;

pub(crate) async fn run(ctx: &CliContext, sub: ConnectorCommand) -> anyhow::Result<()> {
    match sub {
        ConnectorCommand::Add(args) => add(ctx, args).await,
        ConnectorCommand::List => list(ctx).await,
        ConnectorCommand::Sync(args) => sync(ctx, args).await,
    }
}

async fn add(ctx: &CliContext, args: ConnectorAddArgs) -> anyhow::Result<()> {
    // Start with the --config JSON (if any), then layer per-kind knobs on top.
    let mut cfg: Value = match args.config.as_deref() {
        Some(raw) => serde_json::from_str(raw).context("parse --config JSON")?,
        None => json!({}),
    };
    if !cfg.is_object() {
        bail!("--config must be a JSON object");
    }

    match args.kind.as_str() {
        "local-files" => {
            let root = args
                .root
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("local-files requires --root <path>"))?;
            let abs = root.canonicalize().with_context(|| {
                format!("canonicalize --root {}", root.display())
            })?;
            cfg.as_object_mut()
                .unwrap()
                .insert("root".into(), Value::String(abs.display().to_string()));
        }
        other => {
            // Unknown kinds are allowed — the gateway factory will reject
            // them authoritatively if no handler is registered. The CLI
            // does not hardcode the connector catalog.
            tracing::debug!(kind = %other, "no CLI-side validation for connector kind");
        }
    }

    let client = ctx.client();
    let data = client
        .register_connector(&args.kind, &args.schema, cfg)
        .await?;
    output::print(ctx.format, &data);
    Ok(())
}

async fn list(ctx: &CliContext) -> anyhow::Result<()> {
    let client = ctx.client();
    let data = client.list_connectors().await?;
    let items = data
        .get("connectors")
        .cloned()
        .unwrap_or_else(|| Value::Array(vec![]));
    output::print(ctx.format, &items);
    Ok(())
}

async fn sync(ctx: &CliContext, args: ConnectorSyncArgs) -> anyhow::Result<()> {
    let client = ctx.client();
    let data = client.sync_connector(&args.id).await?;
    output::print(ctx.format, &data);
    Ok(())
}
