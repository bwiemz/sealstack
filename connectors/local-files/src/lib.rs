//! ContextForge connector for a local filesystem directory.
//!
//! Each text file under the configured root becomes one [`Resource`]:
//!
//! | Field                  | Value                                          |
//! |------------------------|------------------------------------------------|
//! | `id`                   | Absolute canonical path of the file            |
//! | `kind`                 | File extension (or `"text"` if none)           |
//! | `title`                | Filename without extension                     |
//! | `body`                 | Full file contents (UTF-8)                     |
//! | `metadata.path`        | Canonical path                                 |
//! | `metadata.size_bytes`  | File size in bytes                             |
//! | `permissions`          | `[{ principal: "*", action: "read" }]`         |
//! | `source_updated_at`    | File's `mtime`                                 |
//!
//! # Supported extensions
//!
//! `.md`, `.markdown`, `.txt`, `.rst`, `.csv`, `.json`, `.yaml`, `.yml`, `.rs`,
//! `.py`, `.js`, `.ts`, `.go`, `.java`, `.html`, `.log`. Everything else is
//! skipped — a typical repo has binaries, lockfiles, and images we don't want
//! to embed. Override via [`LocalFilesConnector::with_extensions`].
//!
//! # Watching
//!
//! With the `watch` feature enabled (default), [`Connector::subscribe`] returns
//! a change stream driven by the `notify` crate. File creation, modification,
//! and deletion all propagate to the ingest runtime as [`ChangeEvent`]s.

#![forbid(unsafe_code)]
#![warn(missing_docs, unreachable_pub)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use cfg_common::{CfgError, CfgResult};
use cfg_connector_sdk::{
    ChangeStream, Connector, PermissionPredicate, Resource, ResourceId, ResourceStream,
    change_streams,
};
use serde_json::Value;
use time::OffsetDateTime;
use walkdir::WalkDir;

/// Default set of file extensions this connector ingests.
pub const DEFAULT_EXTENSIONS: &[&str] = &[
    "md", "markdown", "txt", "rst", "csv", "json", "yaml", "yml", "rs", "py", "js", "ts", "tsx",
    "jsx", "go", "java", "html", "htm", "log", "toml",
];

/// The connector.
pub struct LocalFilesConnector {
    root: PathBuf,
    extensions: Arc<HashSet<String>>,
    /// Cap for the size of a single file (in bytes) that will be ingested.
    /// Default 2 MiB. Larger files are skipped with a warning.
    max_file_bytes: u64,
}

impl LocalFilesConnector {
    /// Construct a connector rooted at `root`.
    ///
    /// The path is canonicalized eagerly — if it doesn't exist, construction
    /// fails with [`CfgError::Config`].
    pub fn new(root: impl Into<PathBuf>) -> CfgResult<Self> {
        let root = root.into();
        let root = std::fs::canonicalize(&root).map_err(|e| {
            CfgError::Config(format!(
                "local-files: root `{}` is not readable: {e}",
                root.display()
            ))
        })?;
        if !root.is_dir() {
            return Err(CfgError::Config(format!(
                "local-files: root `{}` is not a directory",
                root.display()
            )));
        }
        let extensions = DEFAULT_EXTENSIONS
            .iter()
            .map(|s| s.to_string())
            .collect::<HashSet<_>>();
        Ok(Self {
            root,
            extensions: Arc::new(extensions),
            max_file_bytes: 2 * 1024 * 1024,
        })
    }

    /// Override the recognized extension set.
    #[must_use]
    pub fn with_extensions<I, S>(mut self, iter: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let set: HashSet<String> = iter
            .into_iter()
            .map(|s| s.into().trim_start_matches('.').to_lowercase())
            .collect();
        self.extensions = Arc::new(set);
        self
    }

    /// Override the per-file size cap (bytes).
    #[must_use]
    pub fn with_max_file_bytes(mut self, n: u64) -> Self {
        self.max_file_bytes = n;
        self
    }

    /// Canonical root directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn is_ingestible(&self, path: &Path) -> bool {
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            return false;
        };
        self.extensions.contains(&ext.to_lowercase())
    }

    async fn read_resource(&self, path: &Path) -> CfgResult<Resource> {
        let metadata = tokio::fs::metadata(path).await.map_err(CfgError::backend)?;
        if metadata.len() > self.max_file_bytes {
            return Err(CfgError::Validation(format!(
                "file `{}` is {} bytes, exceeds max_file_bytes {}",
                path.display(),
                metadata.len(),
                self.max_file_bytes
            )));
        }
        let body = tokio::fs::read_to_string(path).await.map_err(|e| {
            CfgError::Backend(format!(
                "failed to read `{}` as utf-8: {e}",
                path.display()
            ))
        })?;

        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unnamed")
            .to_owned();
        let title = path
            .file_stem()
            .and_then(|n| n.to_str())
            .map(str::to_owned);
        let kind = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_lowercase)
            .unwrap_or_else(|| "text".into());

        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| {
                OffsetDateTime::from_unix_timestamp(d.as_secs() as i64)
                    .unwrap_or_else(|_| OffsetDateTime::now_utc())
            })
            .unwrap_or_else(OffsetDateTime::now_utc);

        let mut meta = serde_json::Map::new();
        meta.insert("path".into(), Value::String(path.display().to_string()));
        meta.insert("size_bytes".into(), Value::from(metadata.len()));
        meta.insert("filename".into(), Value::String(filename));

        Ok(Resource {
            id: ResourceId::new(path.display().to_string()),
            kind,
            title,
            body,
            metadata: meta,
            permissions: vec![PermissionPredicate::public_read()],
            source_updated_at: modified,
        })
    }
}

#[async_trait]
impl Connector for LocalFilesConnector {
    fn name(&self) -> &str {
        "local-files"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    async fn list(&self) -> CfgResult<ResourceStream> {
        let root = self.root.clone();
        let connector = self.clone_lite();

        // Walk synchronously (the walk itself is cheap; IO is in read_resource)
        // and produce resources lazily via tokio::task::spawn_blocking for the
        // walk followed by per-file async reads.
        let paths: Vec<PathBuf> = tokio::task::spawn_blocking(move || {
            WalkDir::new(&root)
                .follow_links(false)
                .into_iter()
                .filter_map(Result::ok)
                .filter(|e| e.file_type().is_file())
                .map(walkdir::DirEntry::into_path)
                .collect()
        })
        .await
        .map_err(|e| CfgError::Backend(format!("walkdir join: {e}")))?;

        // Filter and read each file. Collecting eagerly keeps error handling
        // simple; the connector is sized for typical codebases (10³–10⁴ files).
        let mut out = Vec::new();
        for path in paths {
            if !connector.is_ingestible(&path) {
                continue;
            }
            match connector.read_resource(&path).await {
                Ok(r) => out.push(r),
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "skipping unreadable file");
                }
            }
        }

        Ok(change_streams::resource_stream(out))
    }

    async fn fetch(&self, id: &ResourceId) -> CfgResult<Resource> {
        let path = PathBuf::from(id.as_str());
        if !path.starts_with(&self.root) {
            return Err(CfgError::Unauthorized(format!(
                "path `{}` is outside connector root `{}`",
                path.display(),
                self.root.display()
            )));
        }
        if !path.exists() {
            return Err(CfgError::NotFound(format!(
                "local-files resource `{}` does not exist",
                path.display()
            )));
        }
        self.read_resource(&path).await
    }

    #[cfg(feature = "watch")]
    async fn subscribe(&self) -> CfgResult<Option<ChangeStream>> {
        Ok(Some(watch::watch_stream(self.clone_lite())?))
    }

    #[cfg(not(feature = "watch"))]
    async fn subscribe(&self) -> CfgResult<Option<ChangeStream>> {
        Ok(None)
    }

    async fn healthcheck(&self) -> CfgResult<()> {
        if !self.root.exists() {
            return Err(CfgError::Config(format!(
                "local-files root `{}` does not exist",
                self.root.display()
            )));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Clone lite — enough to send into spawned tasks.
// ---------------------------------------------------------------------------

impl LocalFilesConnector {
    fn clone_lite(&self) -> Self {
        Self {
            root: self.root.clone(),
            extensions: self.extensions.clone(),
            max_file_bytes: self.max_file_bytes,
        }
    }
}

// ---------------------------------------------------------------------------
// Watch support (feature = "watch")
// ---------------------------------------------------------------------------

#[cfg(feature = "watch")]
mod watch {
    use std::path::PathBuf;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use cfg_common::{CfgError, CfgResult};
    use cfg_connector_sdk::{ChangeEvent, ChangeStream, ResourceId};
    use futures::{Stream, StreamExt, stream};
    use notify::{Event, EventKind, RecursiveMode, Watcher};
    use tokio::sync::mpsc::{self, UnboundedReceiver};

    use super::LocalFilesConnector;

    /// Minimal `UnboundedReceiver`-to-`Stream` adapter, inlined to keep the
    /// connector's dependency list small.
    struct ReceiverStream<T> {
        inner: UnboundedReceiver<T>,
    }

    impl<T> Stream for ReceiverStream<T> {
        type Item = T;
        fn poll_next(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<Option<Self::Item>> {
            self.inner.poll_recv(cx)
        }
    }

    /// Build a [`ChangeStream`] that forwards filesystem events for the
    /// connector's root. Events are translated as follows:
    ///
    /// * `Create(_)` / `Modify(_)` → `Upsert` with the current file contents.
    /// * `Remove(_)`               → `Delete(path)`.
    ///
    /// Ignored events: metadata changes without content change, access events.
    pub(super) fn watch_stream(connector: LocalFilesConnector) -> CfgResult<ChangeStream> {
        let (tx, rx) = mpsc::unbounded_channel::<Event>();

        // Build the watcher on a dedicated OS thread; `notify` binds to the
        // current thread's event loop on some platforms so we don't want to
        // tie it to the tokio executor thread.
        let root = connector.root.clone();
        std::thread::Builder::new()
            .name("cfg-local-files-watch".into())
            .spawn(move || {
                let mut watcher = match notify::recommended_watcher(move |res| {
                    if let Ok(event) = res {
                        let _ = tx.send(event);
                    }
                }) {
                    Ok(w) => w,
                    Err(e) => {
                        tracing::error!(error = %e, "failed to start notify watcher");
                        return;
                    }
                };
                if let Err(e) = watcher.watch(&root, RecursiveMode::Recursive) {
                    tracing::error!(error = %e, "failed to watch path");
                    return;
                }
                // Park forever; dropping the watcher cancels it.
                loop {
                    std::thread::park();
                }
            })
            .map_err(|e| CfgError::Backend(format!("spawn watch thread: {e}")))?;

        let rx_stream = ReceiverStream { inner: rx };
        let mapped = rx_stream.then(move |event| {
            let c = connector.clone_lite();
            async move { translate(&c, event).await }
        });
        let flat = mapped.flat_map(stream::iter);

        Ok(Box::pin(flat))
    }

    async fn translate(connector: &LocalFilesConnector, event: Event) -> Vec<ChangeEvent> {
        let mut out = Vec::new();
        match event.kind {
            EventKind::Create(_) | EventKind::Modify(_) => {
                for path in event.paths {
                    if !connector.is_ingestible(&path) {
                        continue;
                    }
                    match connector.read_resource(&path).await {
                        Ok(r) => out.push(ChangeEvent::Upsert(r)),
                        Err(e) => tracing::warn!(
                            path = %path.display(),
                            error = %e,
                            "watch upsert read failed",
                        ),
                    }
                }
            }
            EventKind::Remove(_) => {
                for path in event.paths {
                    out.push(ChangeEvent::Delete(ResourceId::new(
                        path.display().to_string(),
                    )));
                }
            }
            _ => {}
        }
        out
    }

    // Silence the unused import — `PathBuf` isn't used above but keeping it in
    // scope lets future additions (path canonicalization inside `translate`)
    // land without re-adding the import.
    #[allow(dead_code)]
    fn _unused(_p: PathBuf) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[tokio::test]
    async fn lists_only_supported_extensions() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), "# Hello\n\nworld").unwrap();
        std::fs::write(dir.path().join("b.txt"), "plain text").unwrap();
        std::fs::write(dir.path().join("c.bin"), &[0u8, 1, 2]).unwrap();
        std::fs::write(dir.path().join("d.png"), &[0u8; 10]).unwrap();

        let c = LocalFilesConnector::new(dir.path()).unwrap();
        let mut stream = c.list().await.unwrap();
        let mut names = Vec::new();
        while let Some(r) = stream.next().await {
            names.push(r.title.unwrap_or_default());
        }
        names.sort();
        assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
    }

    #[tokio::test]
    async fn fetch_rejects_path_outside_root() {
        let dir = tempfile::tempdir().unwrap();
        let c = LocalFilesConnector::new(dir.path()).unwrap();
        let err = c
            .fetch(&ResourceId::new("/etc/passwd"))
            .await
            .unwrap_err();
        assert!(matches!(err, CfgError::Unauthorized(_)));
    }

    #[tokio::test]
    async fn oversize_file_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let big = dir.path().join("big.md");
        std::fs::write(&big, vec![b'a'; 10]).unwrap();
        let c = LocalFilesConnector::new(dir.path())
            .unwrap()
            .with_max_file_bytes(4);
        let mut stream = c.list().await.unwrap();
        assert!(stream.next().await.is_none());
    }

    #[tokio::test]
    async fn resource_carries_public_read_permission() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), "hi").unwrap();
        let c = LocalFilesConnector::new(dir.path()).unwrap();
        let mut s = c.list().await.unwrap();
        let r = s.next().await.unwrap();
        assert_eq!(r.permissions.len(), 1);
        assert_eq!(r.permissions[0].principal, "*");
    }

    #[tokio::test]
    async fn healthcheck_passes_for_existing_root() {
        let dir = tempfile::tempdir().unwrap();
        let c = LocalFilesConnector::new(dir.path()).unwrap();
        assert!(c.healthcheck().await.is_ok());
    }
}
