//! `sealstack compile` — compile every `.csl` file under the project's schemas dir.

use std::path::{Path, PathBuf};

use anyhow::Context;
use sealstack_csl::{CompileOutput, CompileTargets, compile as csl_compile};
use serde_json::json;

use crate::cli::CompileArgs;
use crate::commands::Context as CliContext;
use crate::output;
use crate::project::ProjectConfig;

pub(crate) async fn run(ctx: &CliContext, args: CompileArgs) -> anyhow::Result<()> {
    let (project, project_root) =
        ProjectConfig::discover(&ctx.project_root)?.unwrap_or_else(|| {
            (
                ProjectConfig::default(),
                ctx.project_root.clone(),
            )
        });
    let input = args
        .input
        .unwrap_or_else(|| project_root.join(&project.paths.schemas));
    let output_dir = args
        .output
        .unwrap_or_else(|| project_root.join(&project.paths.out));

    let sources = collect_csl_files(&input)?;
    if sources.is_empty() {
        eprintln!("no .csl files found under {}", input.display());
        output::print(ctx.format, &json!({ "compiled": [] }));
        return Ok(());
    }

    let mut compiled = Vec::new();
    for path in &sources {
        let src = std::fs::read_to_string(path)
            .with_context(|| format!("read {}", path.display()))?;
        let out = csl_compile(&src, CompileTargets::all())?;
        write_outputs(&output_dir, path, &out)?;
        for meta in &out.schemas_meta {
            let qualified = format!(
                "{}.{}",
                meta.get("namespace").and_then(|v| v.as_str()).unwrap_or("default"),
                meta.get("name").and_then(|v| v.as_str()).unwrap_or("Unnamed"),
            );
            compiled.push(json!({
                "source":    path.display().to_string(),
                "qualified": qualified,
                "version":   meta.get("version"),
            }));
        }
    }

    output::print(ctx.format, &json!({ "output_dir": output_dir.display().to_string(), "compiled": compiled }));
    Ok(())
}

/// Walk `dir` and return every file whose extension is `.csl`.
fn collect_csl_files(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("csl") {
            out.push(path);
        } else if path.is_dir() {
            out.extend(collect_csl_files(&path)?);
        }
    }
    out.sort();
    Ok(out)
}

/// Write a `CompileOutput` bundle under `<output_dir>/...`.
fn write_outputs(output_dir: &Path, source: &Path, out: &CompileOutput) -> anyhow::Result<()> {
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("schema");
    let schemas_dir = output_dir.join("schemas");
    let sql_dir = output_dir.join("sql");
    let mcp_dir = output_dir.join("mcp");
    let vector_dir = output_dir.join("vector");
    let rust_dir = output_dir.join("rust");
    let ts_dir = output_dir.join("ts");
    let py_dir = output_dir.join("py");
    let policy_dir = output_dir.join("policy");
    std::fs::create_dir_all(&schemas_dir)?;
    std::fs::create_dir_all(&sql_dir)?;
    std::fs::create_dir_all(&mcp_dir)?;
    std::fs::create_dir_all(&vector_dir)?;
    std::fs::create_dir_all(&rust_dir)?;
    std::fs::create_dir_all(&ts_dir)?;
    std::fs::create_dir_all(&py_dir)?;
    std::fs::create_dir_all(&policy_dir)?;

    if !out.rust.is_empty()
        && !out.rust.starts_with("// Rust codegen not yet implemented")
    {
        std::fs::write(rust_dir.join("generated.rs"), &out.rust)?;
    }

    if !out.typescript.is_empty()
        && !out.typescript.starts_with("// TypeScript codegen not yet implemented")
    {
        std::fs::write(ts_dir.join("generated.ts"), &out.typescript)?;
    }

    if !out.python.is_empty()
        && !out.python.starts_with("# Python codegen not yet implemented")
    {
        std::fs::write(py_dir.join("generated.py"), &out.python)?;
    }

    for bundle in &out.policy_bundles {
        let name = format!("{}.{}.wasm", bundle.namespace, bundle.schema);
        std::fs::write(policy_dir.join(name), &bundle.wasm)?;
    }

    if !out.sql.is_empty() {
        std::fs::write(sql_dir.join(format!("{stem}_up.sql")), &out.sql)?;
    }
    if !out.vector_plan.is_empty() {
        std::fs::write(vector_dir.join(format!("{stem}.plan.yaml")), &out.vector_plan)?;
    }
    if !out.mcp_tools.is_null() {
        std::fs::write(
            mcp_dir.join(format!("{stem}.tools.json")),
            serde_json::to_string_pretty(&out.mcp_tools)?,
        )?;
    }
    for meta in &out.schemas_meta {
        let qualified = format!(
            "{}.{}",
            meta.get("namespace").and_then(|v| v.as_str()).unwrap_or("default"),
            meta.get("name").and_then(|v| v.as_str()).unwrap_or("Unnamed"),
        );
        std::fs::write(
            schemas_dir.join(format!("{qualified}.schema.json")),
            serde_json::to_string_pretty(meta)?,
        )?;
    }
    Ok(())
}
