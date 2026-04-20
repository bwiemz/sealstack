//! `cfg dev` — boot / teardown the local Docker Compose stack.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use serde_json::json;
use tokio::process::Command;

use crate::cli::DevArgs;
use crate::commands::Context as CliContext;
use crate::output;

pub(crate) async fn run(ctx: &CliContext, args: DevArgs) -> anyhow::Result<()> {
    let compose_file = resolve_compose_file(args.compose_file.as_deref(), &ctx.project_root)?;

    if args.down {
        return teardown(ctx, &compose_file).await;
    }

    boot(ctx, &compose_file, args.no_build, args.timeout_secs).await
}

/// `docker compose -f <file> up -d [--build]` + wait for `/healthz`.
async fn boot(
    ctx: &CliContext,
    compose_file: &Path,
    no_build: bool,
    timeout_secs: u64,
) -> anyhow::Result<()> {
    tracing::info!(compose = %compose_file.display(), "docker compose up");

    let mut cmd = Command::new("docker");
    cmd.arg("compose")
        .arg("-f")
        .arg(compose_file)
        .arg("up")
        .arg("-d");
    if !no_build {
        cmd.arg("--build");
    }
    let status = cmd
        .status()
        .await
        .context("failed to run `docker compose`")?;
    if !status.success() {
        bail!("docker compose up exited with status {status}");
    }

    eprintln!("waiting for gateway at {} …", ctx.gateway_url);
    let start = Instant::now();
    let budget = Duration::from_secs(timeout_secs);
    let client = ctx.client();
    let ready = loop {
        match client.healthz().await {
            Ok(true) => break true,
            Ok(false) | Err(_) if start.elapsed() < budget => {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            _ => break false,
        }
    };

    if !ready {
        bail!(
            "gateway did not become ready within {timeout_secs}s; check `docker compose logs -f`"
        );
    }

    output::print(
        ctx.format,
        &json!({
            "status":      "ready",
            "gateway":     ctx.gateway_url,
            "postgres":    "localhost:5432",
            "qdrant":      "http://localhost:6333/dashboard",
            "elapsed_ms":  start.elapsed().as_millis() as u64,
        }),
    );
    Ok(())
}

async fn teardown(_ctx: &CliContext, compose_file: &Path) -> anyhow::Result<()> {
    tracing::info!(compose = %compose_file.display(), "docker compose down");
    let status = Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg(compose_file)
        .arg("down")
        .status()
        .await
        .context("failed to run `docker compose`")?;
    if !status.success() {
        bail!("docker compose down exited with status {status}");
    }
    eprintln!("stack stopped.");
    Ok(())
}

/// Resolve the compose file path. Preference order:
///
/// 1. Explicit `--compose-file` flag.
/// 2. `./deploy/docker/compose.dev.yaml` under the project root.
/// 3. `CFG_HOME/compose.dev.yaml` if `$CFG_HOME` is set.
fn resolve_compose_file(explicit: Option<&Path>, project_root: &Path) -> anyhow::Result<PathBuf> {
    if let Some(p) = explicit {
        let p = if p.is_absolute() {
            p.to_path_buf()
        } else {
            project_root.join(p)
        };
        if !p.is_file() {
            bail!("compose file {} does not exist", p.display());
        }
        return Ok(p);
    }

    let primary = project_root.join("deploy/docker/compose.dev.yaml");
    if primary.is_file() {
        return Ok(primary);
    }

    if let Ok(home) = std::env::var("CFG_HOME") {
        let fallback = PathBuf::from(home).join("compose.dev.yaml");
        if fallback.is_file() {
            return Ok(fallback);
        }
    }

    bail!(
        "could not find a compose file; searched `deploy/docker/compose.dev.yaml` and $CFG_HOME. \
         Pass --compose-file <path> explicitly."
    )
}
