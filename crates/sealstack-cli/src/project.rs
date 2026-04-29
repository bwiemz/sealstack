//! Project configuration file (`cfg.toml`) discovery and parsing.
//!
//! `cfg.toml` sits at the root of a SealStack project and tells the CLI:
//!
//! ```toml
//! [project]
//! name = "engineering-context"
//! namespace = "examples"
//!
//! [paths]
//! schemas = "schemas"
//! out     = "out"
//!
//! [gateway]
//! url = "http://localhost:7070"
//! ```
//!
//! Every field is optional; missing values fall back to the per-invocation
//! defaults in `cli::Cli`. Multiple `cfg.toml` files in a workspace are not
//! supported — the nearest ancestor wins.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

/// Parsed `cfg.toml`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct ProjectConfig {
    #[serde(default)]
    pub project: Project,
    #[serde(default)]
    pub paths: Paths,
    #[serde(default)]
    pub gateway: Gateway,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct Project {
    #[serde(default)]
    pub name: Option<String>,
    /// Default namespace applied to schemas that don't declare one.
    #[serde(default)]
    pub namespace: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Paths {
    #[serde(default = "default_schemas")]
    pub schemas: PathBuf,
    #[serde(default = "default_out")]
    pub out: PathBuf,
}

impl Default for Paths {
    fn default() -> Self {
        Self {
            schemas: default_schemas(),
            out: default_out(),
        }
    }
}

fn default_schemas() -> PathBuf {
    PathBuf::from("schemas")
}
fn default_out() -> PathBuf {
    PathBuf::from("out")
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct Gateway {
    #[serde(default)]
    pub url: Option<String>,
}

impl ProjectConfig {
    /// Walk up from `start` looking for a `cfg.toml`. Returns the project
    /// config and the absolute root directory it was found in.
    pub(crate) fn discover(start: &Path) -> anyhow::Result<Option<(Self, PathBuf)>> {
        let mut current = start.canonicalize().context("canonicalize start path")?;
        loop {
            let candidate = current.join("cfg.toml");
            if candidate.is_file() {
                let bytes = std::fs::read_to_string(&candidate)
                    .with_context(|| format!("read {}", candidate.display()))?;
                let cfg: ProjectConfig = toml::from_str(&bytes)
                    .with_context(|| format!("parse {}", candidate.display()))?;
                return Ok(Some((cfg, current)));
            }
            if !current.pop() {
                return Ok(None);
            }
        }
    }

    /// Write a minimal default `cfg.toml` at `root`.
    pub(crate) fn write_default(
        root: &Path,
        name: &str,
        overwrite: bool,
    ) -> anyhow::Result<PathBuf> {
        let path = root.join("cfg.toml");
        if path.exists() && !overwrite {
            bail!(
                "{} already exists (pass --force to overwrite)",
                path.display()
            );
        }
        let body = format!(
            "[project]\nname = \"{name}\"\nnamespace = \"examples\"\n\n\
             [paths]\nschemas = \"schemas\"\nout     = \"out\"\n\n\
             [gateway]\nurl = \"http://localhost:7070\"\n",
        );
        std::fs::write(&path, body).with_context(|| format!("write {}", path.display()))?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_returns_none_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let got = ProjectConfig::discover(dir.path()).unwrap();
        assert!(got.is_none());
    }

    #[test]
    fn discover_walks_up() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a/b/c");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(dir.path().join("cfg.toml"), "[project]\nname = \"demo\"\n").unwrap();
        let (cfg, root) = ProjectConfig::discover(&nested).unwrap().unwrap();
        assert_eq!(cfg.project.name.as_deref(), Some("demo"));
        assert_eq!(root, dir.path().canonicalize().unwrap());
    }

    #[test]
    fn write_default_refuses_overwrite_without_force() {
        let dir = tempfile::tempdir().unwrap();
        ProjectConfig::write_default(dir.path(), "x", false).unwrap();
        let err = ProjectConfig::write_default(dir.path(), "x", false).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }
}
