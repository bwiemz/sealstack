//! `cfg version` — print CLI version and build info.

use serde_json::json;

use crate::commands::Context;
use crate::output;

pub(crate) async fn run(ctx: &Context) -> anyhow::Result<()> {
    let gateway_reachable = {
        let client = ctx.client();
        client.healthz().await.unwrap_or(false)
    };
    let data = json!({
        "cli_version":       env!("CARGO_PKG_VERSION"),
        "gateway_url":       ctx.gateway_url,
        "gateway_reachable": gateway_reachable,
        "caller":            ctx.effective_user(),
    });
    output::print(ctx.format, &data);
    Ok(())
}
