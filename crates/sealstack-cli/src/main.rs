//! SealStack CLI.
//!
//! Thin client over the gateway's REST surface plus a few local operations
//! (CSL compilation, `sealstack dev` docker shelling).
//!
//! # Most commands talk to the gateway
//!
//! | Command                          | Gateway call                              |
//! |----------------------------------|-------------------------------------------|
//! | `sealstack schema apply <path>`        | compile locally + `POST /v1/schemas` + `POST /v1/schemas/:q/ddl` |
//! | `sealstack schema list`                | `GET /v1/schemas`                         |
//! | `sealstack connector add <kind>`       | `POST /v1/connectors`                     |
//! | `sealstack connector list`             | `GET /v1/connectors`                      |
//! | `sealstack connector sync <id>`        | `POST /v1/connectors/:id/sync`            |
//! | `sealstack query <text>`               | `POST /v1/query`                          |
//! | `sealstack receipt <id>`               | `GET /v1/receipts/:id`                    |
//!
//! # Local-only commands
//!
//! | Command           | Does                                                       |
//! |-------------------|------------------------------------------------------------|
//! | `sealstack compile`     | Compiles every `.csl` file under `./schemas`               |
//! | `sealstack init`        | Scaffolds a `cfg.toml` + `schemas/` directory              |
//! | `sealstack dev`         | `docker compose -f <compose_file> up -d` + waits for ready |
//! | `sealstack version`     | Prints the CLI version                                     |

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
        "warn,sealstack_cli=info"
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
