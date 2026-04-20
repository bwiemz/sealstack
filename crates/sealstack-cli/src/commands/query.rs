//! `sealstack query` — run a search against a registered schema.

use anyhow::Context;
use serde_json::{Value, json};

use crate::cli::QueryArgs;
use crate::commands::Context as CliContext;
use crate::output::{self, Format};

pub(crate) async fn run(ctx: &CliContext, args: QueryArgs) -> anyhow::Result<()> {
    let filters: Value = match args.filters.as_deref() {
        Some(s) => serde_json::from_str(s).context("parse --filters JSON")?,
        None => json!({}),
    };

    let client = ctx.client();
    let data = client
        .query(&args.schema, &args.query, args.top_k, filters)
        .await?;

    match ctx.format {
        Format::Json => output::print(ctx.format, &data),
        Format::Human => render_human(&data),
    }
    Ok(())
}

fn render_human(data: &Value) {
    let receipt_id = data
        .get("receipt_id")
        .and_then(Value::as_str)
        .unwrap_or("(none)");
    let results = data
        .get("results")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    if results.is_empty() {
        println!("(no hits)");
        println!();
        println!("receipt: {receipt_id}");
        return;
    }

    // Render a focused per-hit table: id | score | excerpt
    let table_rows: Vec<Value> = results
        .iter()
        .map(|hit| {
            json!({
                "id":      hit.get("id").cloned().unwrap_or(Value::Null),
                "score":   hit.get("score").cloned().unwrap_or(Value::Null),
                "excerpt": truncate_string(hit.get("excerpt").and_then(Value::as_str).unwrap_or(""), 80),
            })
        })
        .collect();

    output::print(Format::Human, &Value::Array(table_rows));
    println!();
    println!("{} hit(s). receipt: {receipt_id}", results.len());
    println!("details: sealstack receipt {receipt_id}");
}

fn truncate_string(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let mut out: String = s.chars().take(max - 1).collect();
        out.push('…');
        out
    }
}
