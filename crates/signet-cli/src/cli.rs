//! Clap argument types.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// Signet CLI.
#[derive(Parser, Debug)]
#[command(
    name = "cfg",
    version,
    about = "Signet command-line interface",
    long_about = None,
)]
pub(crate) struct Cli {
    /// Gateway base URL. Defaults to `$SIGNET_GATEWAY_URL` or `http://localhost:7070`.
    #[arg(
        long,
        global = true,
        env = "SIGNET_GATEWAY_URL",
        default_value = "http://localhost:7070"
    )]
    pub gateway_url: String,

    /// Caller user id sent via `X-Cfg-User`. Defaults to `$USER` or `"anon"`.
    #[arg(long, global = true, env = "SIGNET_USER")]
    pub user: Option<String>,

    /// Emit JSON for machine consumption. Default: human-readable tables.
    #[arg(long, global = true)]
    pub json: bool,

    /// Verbose logging (`debug` level to stderr).
    #[arg(short, long, global = true, conflicts_with = "quiet")]
    pub verbose: bool,

    /// Suppress non-error logging.
    #[arg(short, long, global = true)]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    /// Print version and build info.
    Version,

    /// Boot the local dev stack (Postgres + Qdrant + gateway) via Docker Compose.
    Dev(DevArgs),

    /// Scaffold a Signet project in the current directory.
    Init(InitArgs),

    /// Compile every `.csl` file in `./schemas`.
    Compile(CompileArgs),

    /// Manage context schemas.
    #[command(subcommand)]
    Schema(SchemaCommand),

    /// Manage connector bindings.
    #[command(subcommand)]
    Connector(ConnectorCommand),

    /// Run a search query against a registered schema.
    Query(QueryArgs),

    /// Fetch a receipt by id.
    Receipt(ReceiptArgs),
}

// ---------------------------------------------------------------------------
// dev / init / compile
// ---------------------------------------------------------------------------

#[derive(Args, Debug)]
pub(crate) struct DevArgs {
    /// Alternative compose file. Defaults to `deploy/docker/compose.dev.yaml`.
    #[arg(long)]
    pub compose_file: Option<PathBuf>,

    /// Skip building fresh images; run whatever's already in the local registry.
    #[arg(long)]
    pub no_build: bool,

    /// Tear the stack down instead of bringing it up.
    #[arg(long, conflicts_with_all = ["no_build"])]
    pub down: bool,

    /// Seconds to wait for healthz before giving up.
    #[arg(long, default_value = "60")]
    pub timeout_secs: u64,
}

#[derive(Args, Debug)]
pub(crate) struct InitArgs {
    /// Directory to initialize. Defaults to the current directory.
    #[arg(long)]
    pub path: Option<PathBuf>,

    /// Overwrite existing files.
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug)]
pub(crate) struct CompileArgs {
    /// Source directory (defaults to `./schemas`).
    #[arg(long)]
    pub input: Option<PathBuf>,

    /// Output directory (defaults to `./out`).
    #[arg(long)]
    pub output: Option<PathBuf>,
}

// ---------------------------------------------------------------------------
// schema
// ---------------------------------------------------------------------------

#[derive(Subcommand, Debug)]
pub(crate) enum SchemaCommand {
    /// Register a CSL file with the gateway.
    ///
    /// Compiles the file locally, uploads the schema metadata, then uploads
    /// the generated DDL. The gateway applies the DDL via `sqlx`.
    Apply(SchemaApplyArgs),
    /// List every schema registered with the gateway.
    List,
    /// Print metadata for one qualified schema name.
    Get(SchemaGetArgs),
    /// Compile a CSL file and print the results without uploading.
    Validate(SchemaValidateArgs),
}

#[derive(Args, Debug)]
pub(crate) struct SchemaApplyArgs {
    /// Path to a `.csl` file.
    pub path: PathBuf,
}

#[derive(Args, Debug)]
pub(crate) struct SchemaGetArgs {
    /// Qualified schema name, e.g. `"acme.crm.Customer"`.
    pub qualified: String,
}

#[derive(Args, Debug)]
pub(crate) struct SchemaValidateArgs {
    /// Path to a `.csl` file.
    pub path: PathBuf,
}

// ---------------------------------------------------------------------------
// connector
// ---------------------------------------------------------------------------

#[derive(Subcommand, Debug)]
pub(crate) enum ConnectorCommand {
    /// Register a connector binding with the gateway.
    Add(ConnectorAddArgs),
    /// List every registered connector binding.
    List,
    /// Trigger one synchronous sync run for the given binding id.
    Sync(ConnectorSyncArgs),
}

#[derive(Args, Debug)]
pub(crate) struct ConnectorAddArgs {
    /// Connector kind — currently `"local-files"`.
    pub kind: String,
    /// Target qualified schema, e.g. `"examples.Doc"`.
    #[arg(long)]
    pub schema: String,
    /// Root directory for `local-files`. Required for that kind.
    #[arg(long)]
    pub root: Option<PathBuf>,
    /// Arbitrary JSON blob merged into the connector config.
    #[arg(long)]
    pub config: Option<String>,
}

#[derive(Args, Debug)]
pub(crate) struct ConnectorSyncArgs {
    /// Binding id (from `signet connector list`), e.g. `"local-files/examples.Doc"`.
    pub id: String,
}

// ---------------------------------------------------------------------------
// query / receipt
// ---------------------------------------------------------------------------

#[derive(Args, Debug)]
pub(crate) struct QueryArgs {
    /// The search query.
    pub query: String,
    /// Qualified schema to query, e.g. `"examples.Doc"`.
    #[arg(long)]
    pub schema: String,
    /// Maximum results to return.
    #[arg(long)]
    pub top_k: Option<usize>,
    /// Facet filters as inline JSON, e.g. `--filters '{"status": "open"}'`.
    #[arg(long)]
    pub filters: Option<String>,
}

#[derive(Args, Debug)]
pub(crate) struct ReceiptArgs {
    /// Receipt id.
    pub id: String,
}
