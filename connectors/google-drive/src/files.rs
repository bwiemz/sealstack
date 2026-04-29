//! Drive `files.list` pagination + MIME allowlist + skip logic.

// pub(crate) items in this private module trigger clippy::redundant_pub_crate
// because the module itself is not pub. Suppress: these items are deliberately
// pub(crate) to signal "crate-internal only", matching the task spec. The
// exception is `DriveFile` and `list_files_for_test` which must be `pub` so
// integration tests in tests/ can reach them via `test_only`.
#![allow(clippy::redundant_pub_crate)]

use std::collections::HashSet;
use std::sync::Arc;

use sealstack_common::{SealStackError, SealStackResult};
use sealstack_connector_sdk::ResourceId;
use sealstack_connector_sdk::http::HttpClient;
use sealstack_connector_sdk::paginate::{BodyCursorPaginator, paginate};
use serde::Deserialize;

/// Drive file metadata as returned by `files.list`.
///
/// Only the fields we use are deserialized.
#[derive(Debug, Clone, Deserialize)]
pub struct DriveFile {
    pub id: String,
    #[allow(dead_code)] // surfaced as Resource.title in Task 12
    pub name: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    #[serde(rename = "modifiedTime")]
    #[allow(dead_code)] // surfaced as Resource.source_updated_at in Task 12
    pub modified_time: String,
    /// Present for items in Shared Drives. v1 (corpora=user) skips these.
    #[serde(rename = "driveId")]
    pub drive_id: Option<String>,
    /// Optional. Present for binary content; absent for Google-native types.
    #[allow(dead_code)] // consumed by per-file size cap in Task 10
    pub size: Option<String>,
    /// Inline ACLs from `fields=files(permissions(...))`.
    #[serde(default)]
    #[allow(dead_code)] // consumed by resource.rs in Task 12
    pub(crate) permissions: Vec<crate::permissions::DrivePermission>,
}

/// Connector-internal "log once per resource id" dedup helper.
///
/// Used to surface MIME-skip decisions without spamming logs on every sync
/// cycle. Reset on connector restart, scoped per-connector-instance. v1
/// implementation; promoted to SDK if a second connector wants similar
/// dedup (see spec §13).
#[allow(dead_code)] // wired into body-fetch path in Task 10
#[derive(Debug, Default)]
pub(crate) struct SkipLog {
    seen: tokio::sync::Mutex<HashSet<ResourceId>>,
}

impl SkipLog {
    #[allow(dead_code)] // wired into body-fetch path in Task 10
    pub(crate) async fn note_once<F: FnOnce()>(&self, id: &ResourceId, log_fn: F) {
        let mut seen = self.seen.lock().await;
        if seen.insert(id.clone()) {
            log_fn();
        }
    }
}

const FILES_LIST_FIELDS: &str = "files(id,name,mimeType,modifiedTime,driveId,size,\
                                 permissions(type,emailAddress,domain,role,allowFileDiscovery)),\
                                 nextPageToken";

const FILES_LIST_QUERY: &str = "trashed = false and ('me' in owners or sharedWithMe)";

/// Build the [`BodyCursorPaginator`] over `files.list`.
///
/// Filters out:
/// - MIME types not in the v1 allowlist (logged via [`SkipLog`] at the body-fetch
///   site in Task 10; here we just emit them for size-cap evaluation).
/// - `driveId`-bearing items (v1 corpora=user constraint).
///
/// Returns a boxed stream to avoid propagating the four-impl-Fn type
/// parameters up to callers that don't need them.
pub(crate) fn files_stream(
    http: Arc<HttpClient>,
    api_base: &str,
) -> std::pin::Pin<Box<dyn futures::Stream<Item = SealStackResult<DriveFile>> + Send>> {
    let url = format!("{}/drive/v3/files", api_base.trim_end_matches('/'));
    let pg = BodyCursorPaginator::<DriveFile, _, _, _>::new(
        move |c, cursor: Option<&str>| {
            let mut rb = c.get(&url).query(&[
                ("q", FILES_LIST_QUERY),
                ("fields", FILES_LIST_FIELDS),
                ("pageSize", "1000"),
                ("supportsAllDrives", "false"),
            ]);
            if let Some(cur) = cursor {
                rb = rb.query(&[("pageToken", cur)]);
            }
            rb
        },
        |body: &serde_json::Value| {
            let arr = body
                .get("files")
                .and_then(|a| a.as_array())
                .ok_or_else(|| SealStackError::backend("drive: missing files array"))?;
            let mut out = Vec::with_capacity(arr.len());
            for item in arr {
                let f: DriveFile = serde_json::from_value(item.clone())
                    .map_err(|e| SealStackError::backend(format!("drive file parse: {e}")))?;
                if let Some(drive_id) = &f.drive_id {
                    tracing::info!(
                        file_id = %f.id, %drive_id,
                        "drive: skipping item from Shared Drive (v1 corpora=user)"
                    );
                    continue;
                }
                if !is_allowed_mime(&f.mime_type) {
                    // MIME-skip dedup happens at body-fetch (Task 10). At the
                    // paginator level we filter without per-id dedup; the same
                    // file would get the same info log on every sync cycle if
                    // we logged here. Filter silently.
                    continue;
                }
                out.push(f);
            }
            Ok(out)
        },
        |body: &serde_json::Value| {
            body.get("nextPageToken")
                .and_then(|t| t.as_str())
                .map(str::to_owned)
        },
    );
    paginate(pg, http)
}

fn is_allowed_mime(mime: &str) -> bool {
    matches!(
        mime,
        "application/vnd.google-apps.document" | "text/plain" | "text/markdown"
    )
}

/// Test-only: drive the paginator and yield filtered [`DriveFile`]s.
///
/// Exposed so integration tests can exercise the paginator without standing
/// up the full `DriveConnector` + OAuth machinery.
#[doc(hidden)]
#[must_use]
pub fn list_files_for_test(
    http: Arc<HttpClient>,
    api_base: &str,
) -> std::pin::Pin<Box<dyn futures::Stream<Item = SealStackResult<DriveFile>> + Send>> {
    files_stream(http, api_base)
}

/// Fetch a file's body, respecting MIME allowlist + per-file size cap.
///
/// Returns `Ok(None)` for files that should be skipped (oversized, non-allowlist
/// MIME, non-UTF-8 text claim). Returns `Err` only for genuine failures
/// (Google's docs export contract violated, network error after retries).
pub(crate) async fn fetch_body(
    http: &Arc<HttpClient>,
    api_base: &str,
    file: &DriveFile,
    max_file_bytes: u64,
    skip_log: &SkipLog,
) -> SealStackResult<Option<String>> {
    let id = ResourceId::new(file.id.clone());

    // Per-file size cap (separate from SDK's HTTP body cap).
    let size: Option<u64> = file.size.as_ref().and_then(|s| s.parse().ok());
    if let Some(s) = size
        && s > max_file_bytes
    {
        skip_log
            .note_once(&id, || {
                tracing::info!(
                    file_id = %file.id,
                    size = s,
                    cap = max_file_bytes,
                    "drive: skipping file exceeding per-file size cap"
                );
            })
            .await;
        return Ok(None);
    }

    match file.mime_type.as_str() {
        "application/vnd.google-apps.document" => {
            // Google Docs: export as text/plain.
            // Routed through send_with_drive_shim so a 403 userRateLimitExceeded
            // during the bulk list() body-fetch path is retried with backoff
            // rather than aborting the entire sync cycle.
            let url = format!(
                "{}/drive/v3/files/{}/export",
                api_base.trim_end_matches('/'),
                file.id
            );
            let make = || http.get(&url).query(&[("mimeType", "text/plain")]);
            let resp = crate::retry_shim::send_with_drive_shim(http, make).await?;
            let bytes = resp.bytes().await?;
            // Strict UTF-8 — Docs export contract guarantees UTF-8; a violation
            // is a Google-side bug, not a user-side mistake, so error rather
            // than silently dropping.
            String::from_utf8(bytes.to_vec())
                .map(Some)
                .map_err(|_| SealStackError::backend("drive: docs export returned non-UTF-8"))
        }
        "text/plain" | "text/markdown" => {
            // Direct binary fetch.
            // Same shim wrapping as Docs export — Drive 403 rate-limits during
            // bulk body-fetch are retried with backoff, not aborting list().
            let url = format!(
                "{}/drive/v3/files/{}",
                api_base.trim_end_matches('/'),
                file.id
            );
            let make = || http.get(&url).query(&[("alt", "media")]);
            let resp = crate::retry_shim::send_with_drive_shim(http, make).await?;
            let bytes = resp.bytes().await?;
            // Strict UTF-8 — text MIME is a user-supplied claim Drive doesn't
            // validate. A non-UTF-8 file claimed as text/plain is a config
            // error or a deliberate skip case (binary mislabeled as text).
            // Skip without erroring; from_utf8_lossy would silently embed
            // U+FFFD pollution into the index.
            if let Ok(s) = String::from_utf8(bytes.to_vec()) {
                Ok(Some(s))
            } else {
                skip_log
                    .note_once(&id, || {
                        tracing::info!(
                            file_id = %file.id,
                            mime_type = %file.mime_type,
                            "drive: skipping file with non-UTF-8 body (claimed text MIME)"
                        );
                    })
                    .await;
                Ok(None)
            }
        }
        other => {
            skip_log
                .note_once(&id, || {
                    tracing::info!(
                        file_id = %file.id,
                        mime_type = %other,
                        "drive: skipping unsupported MIME type \
                         (v1 allowlist: docs, text, markdown)"
                    );
                })
                .await;
            Ok(None)
        }
    }
}

/// Test-only stub of [`DriveFile`] carrying just the fields [`fetch_body`] needs.
#[doc(hidden)]
pub struct DriveFileTestStub {
    /// Drive file ID.
    pub id: String,
    /// MIME type.
    pub mime_type: String,
    /// Reported file size in bytes, as a string (mirrors the Drive API field).
    pub size: Option<String>,
}

/// Test-only entry point for [`fetch_body`] that bypasses the full [`DriveConnector`].
#[doc(hidden)]
pub async fn fetch_body_for_test(
    http: Arc<HttpClient>,
    api_base: &str,
    file: &DriveFileTestStub,
    max_file_bytes: u64,
) -> SealStackResult<Option<String>> {
    let f = DriveFile {
        id: file.id.clone(),
        name: String::new(),
        mime_type: file.mime_type.clone(),
        modified_time: String::new(),
        drive_id: None,
        size: file.size.clone(),
        permissions: Vec::new(),
    };
    fetch_body(&http, api_base, &f, max_file_bytes, &SkipLog::default()).await
}
