//! `signet init` — scaffold a new Signet project.

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde_json::json;

use crate::cli::InitArgs;
use crate::commands::Context as CliContext;
use crate::output;
use crate::project::ProjectConfig;

pub(crate) async fn run(ctx: &CliContext, args: InitArgs) -> anyhow::Result<()> {
    let root = args
        .path
        .clone()
        .unwrap_or_else(|| ctx.project_root.clone());
    std::fs::create_dir_all(&root).context("create project root")?;

    let name = root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("signet-project")
        .to_owned();

    let toml_path = ProjectConfig::write_default(&root, &name, args.force)?;
    create_subdir(&root, "schemas", args.force)?;
    create_subdir(&root, "sample-docs", args.force)?;

    // A starter .csl file — demonstrative, not required.
    let starter_path = root.join("schemas/doc.csl");
    if !starter_path.exists() || args.force {
        std::fs::write(&starter_path, STARTER_CSL).context("write starter schema")?;
    }

    // A starter sample doc.
    let sample_path = root.join("sample-docs/getting-started.md");
    if !sample_path.exists() || args.force {
        std::fs::write(&sample_path, STARTER_DOC).context("write starter doc")?;
    }

    output::print(
        ctx.format,
        &json!({
            "project":  name,
            "root":     root.display().to_string(),
            "signet_toml": toml_path.display().to_string(),
            "schemas":  "schemas/doc.csl",
            "docs":     "sample-docs/getting-started.md",
            "next": [
                "signet dev",
                "signet schema apply schemas/doc.csl",
                "signet connector add local-files --schema examples.Doc --root ./sample-docs",
                "signet connector sync local-files/examples.Doc",
                "signet query \"getting started\" --schema examples.Doc"
            ],
        }),
    );
    Ok(())
}

fn create_subdir(root: &Path, name: &str, force: bool) -> anyhow::Result<PathBuf> {
    let path = root.join(name);
    if path.exists() && !force {
        return Ok(path);
    }
    std::fs::create_dir_all(&path).with_context(|| format!("create {}", path.display()))?;
    Ok(path)
}

const STARTER_CSL: &str = r#"// Starter schema. Run `signet schema apply` to register it with the gateway.

schema Doc version 1 {
    id:         Ulid    @primary
    path:       String  @searchable @indexed
    title:      String  @searchable
    body:       Text    @chunked
    updated_at: Instant

    context {
        chunking        = semantic(max_tokens = 512, overlap = 64)
        embedder        = "stub"
        vector_dims     = 64
        default_top_k   = 8
        freshness_decay = exponential(half_life = 30d)
    }
}
"#;

const STARTER_DOC: &str = r#"# Getting started

Signet stands up in three commands once you have Docker running:

    signet dev
    signet schema apply schemas/doc.csl
    signet connector add local-files --schema examples.Doc --root ./sample-docs

Once the connector syncs, you can query the corpus:

    signet query "getting started" --schema examples.Doc

The gateway serves the same data over MCP at /mcp/examples.Doc.
"#;
