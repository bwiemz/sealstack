//! Signet CLI.
//!
//! Thin client over the gateway's REST surface plus a few local operations
//! (CSL compilation, `signet dev` docker shelling).
//!
//! # Most commands talk to the gateway
//!
//! | Command                          | Gateway call                              |
//! |----------------------------------|-------------------------------------------|
//! | `signet schema apply <path>`        | compile locally + `POST /v1/schemas` + `POST /v1/schemas/:q/ddl` |
//! | `signet schema list`                | `GET /v1/schemas`                         |
//! | `signet connector add <kind>`       | `POST /v1/connectors`                     |
//! | `signet connector list`             | `GET /v1/connectors`                      |
//! | `signet connector sync <id>`        | `POST /v1/connectors/:id/sync`            |
//! | `signet query <text>`               | `POST /v1/query`                          |
//! | `signet receipt <id>`               | `GET /v1/receipts/:id`                    |
//!
//! # Local-only commands
//!
//! | Command           | Does                                                       |
//! |-------------------|------------------------------------------------------------|
//! | `signet compile`     | Compiles every `.csl` file under `./schemas`               |
//! | `signet init`        | Scaffolds a `cfg.toml` + `schemas/` directory              |
//! | `signet dev`         | `docker compose -f <compose_file> up -d` + waits for ready |
//! | `signet version`     | Prints the CLI version                                     |

#![forbid(unsafe_code)]
#![warn(unreachable_pub)]

mod cli;
mod client;
mod commands;
mod output;
mod project;

use clap::Parser;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = cli::Cli::parse();

    // Honor --quiet / --verbose / default. CLI does structured logs to stderr,
    // user-facing output to stdout; the formatter in `output` writes to stdout.
    let level_filter = if args.verbose {
        "debug"
    } else if args.quiet {
        "error"
    } else {
        "warn,signet_cli=info"
    };
    tracing_subscriber::registry()
        .with(EnvFilter::try_new(level_filter).unwrap_or_else(|_| EnvFilter::new("warn")))
        .with(fmt::layer().with_writer(std::io::stderr).without_time())
        .init();

    let format = if args.json {
        output::Format::Json
    } else {
        output::Format::Human
    };

    let ctx = commands::Context {
        gateway_url: args.gateway_url.clone(),
        user: args.user.clone(),
        project_root: std::env::current_dir()?,
        format,
    };

    let result = match args.command {
        cli::Command::Version => commands::version::run(&ctx).await,
        cli::Command::Dev(cmd) => commands::dev::run(&ctx, cmd).await,
        cli::Command::Init(cmd) => commands::init::run(&ctx, cmd).await,
        cli::Command::Compile(cmd) => commands::compile::run(&ctx, cmd).await,
        cli::Command::Schema(sub) => commands::schema::run(&ctx, sub).await,
        cli::Command::Connector(sub) => commands::connector::run(&ctx, sub).await,
        cli::Command::Query(cmd) => commands::query::run(&ctx, cmd).await,
        cli::Command::Receipt(cmd) => commands::receipt::run(&ctx, cmd).await,
    };

    if let Err(e) = result {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
    Ok(())
}
