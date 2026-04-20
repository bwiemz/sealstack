//! `cfg receipt` — fetch one receipt by id.

use crate::cli::ReceiptArgs;
use crate::commands::Context as CliContext;
use crate::output;

pub(crate) async fn run(ctx: &CliContext, args: ReceiptArgs) -> anyhow::Result<()> {
    let client = ctx.client();
    let data = client.receipt(&args.id).await?;
    // Receipts are nested objects — JSON mode shows everything, human mode
    // renders the top-level fields as aligned key:value lines.
    output::print(ctx.format, &data);
    Ok(())
}
