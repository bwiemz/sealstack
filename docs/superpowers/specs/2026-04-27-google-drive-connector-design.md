# Google Drive Connector + SDK OAuth2 Extension — Design Spec

**Date:** 2026-04-27
**Status:** Approved for implementation
**Author:** collaborative brainstorm captured 2026-04-27
**Predecessor:** [`docs/superpowers/specs/2026-04-24-connector-sdk-hardening-design.md`](2026-04-24-connector-sdk-hardening-design.md) (SDK hardening) — establishes the `Credential` trait, `HttpClient`, `Paginator` trait, and reference paginator builders this slice builds on.

## 1. Goals

Land the Google Drive connector and the SDK extension that unblocks every other OAuth-based connector (Notion, Linear, Dropbox, Microsoft Graph, Gmail). Specifically:

- Add `OAuth2Credential` to `sealstack-connector-sdk::auth` as the second `Credential` impl (alongside `StaticToken`), validating the trait shape against a real OAuth provider.
- Land the deferred SDK §5 typed `Principal` enum, replacing `PermissionPredicate.principal: String` with a closed-set type that future OAuth connectors plug into.
- Build `connectors/google-drive` as the consumer that exercises both pieces end-to-end: OAuth refresh, Drive's typed permission model, MIME-aware body fetching, and a Drive-specific 403 retry shim.

The OAuth machinery is the load-bearing piece. Drive validates it. Future OAuth connectors inherit it without rebuilding.

## 2. Non-goals

- **No CLI consent flow.** Users provide a refresh token externally (Google's OAuth playground or a one-off script) and reference it via env var in their config. The CLI follow-up slice (`sealstack auth google`) writes to the same config-file shape and lands separately.
- **No Shared Drives** (`corpora=shared|all`). v1 is `corpora=user` only — personal Drive plus files explicitly shared in. The `corpora` config field is accepted but rejected at validation if not `"user"`. Shared Drives land in v0.2.
- **No file formats beyond plain text + Google Docs.** MIME allowlist is `text/plain`, `text/markdown`, `application/vnd.google-apps.document`. Sheets / PDFs / .docx / Office formats are skipped at the metadata stage.
- **No incremental sync via `changes.list`.** v1 is full-crawl every cycle, default 15-minute interval as deletion-latency mitigation. v0.2 incremental layers a state file *on top* of the full-crawl path; a connector with no state file degrades to v1 behavior.
- **No `Principal::Custom` escape hatch.** When a connector's source ACL primitives don't fit `User`/`Group`/`Domain`/`Anyone`/`AnyoneWithLink`, the resolution is a deliberate semantic-mapping decision or a design proposal to extend the enum — not opaque-string drift.
- **No engine integration of per-connector `sync_interval`.** Connector exposes the accessor; engine adoption is a separate engine slice.
- **No paginator-side 403 shim.** Pagination calls (`BodyCursorPaginator` driving `files.list`) bypass the shim; matches github precedent. v0.2 SDK work to push the shim into paginators fixes both connectors at once.
- **No multi-replica connector deployments with shared OAuth credentials.** v1 backoff has no jitter; if multi-replica + shared credential surfaces a synchronized-retry issue, jitter gets added (small change).

## 3. Scope

Two PRs, eleven commits. PR 1 closes the typed-Principal migration as a self-contained, mergeable, bisect-friendly unit; PR 2 builds Drive on top.

| PR | Commit | Purpose |
|----|--------|---------|
| 1 | 1 | Typed `Principal` enum with serde wire-shape + `"*"` legacy alias |
| 1 | 2 | `PermissionPredicate.principal: String → Principal` (with local-files migration) |
| 1 | 3 | `slack`: emit `Principal::Group` for channel ACLs (wire shape changes) |
| 1 | 4 | `github`: emit `Principal::Group` for owner ACLs (wire shape changes) |
| 1 | 5 | `OAuth2Credential` with refresh, caching, negative-cache, refresh-coalescing |
| 1 | 6 | Principal-mapping ADR + design-pressure principle docs |
| 2 | 1 | `google-drive` scaffold + `DriveConfig::from_json` + corpora validation |
| 2 | 2 | Drive permission mapping (`DrivePermission` → `Principal`) |
| 2 | 3 | `files.list` pagination + MIME allowlist + `SkipLog` dedup |
| 2 | 4 | Body fetch (export vs `alt=media`) with strict UTF-8 + per-file cap |
| 2 | 5 | `retry_shim` for 403 three-class discrimination |
| 2 | 6 | `DriveFile → Resource` projection + `Connector` impl + e2e tests |

Commit count (5 + 6 = 11) is deliberate. Each commit independently compiles + tests. Bisect-friendliness across the type-system migration in PR 1 commits 1–4 requires this granularity; large monolithic commits would be much harder to review.

## 4. Module layout

### SDK extension (additive)

```text
crates/sealstack-connector-sdk/
├── src/
│   ├── lib.rs                  — adds `pub use principal::Principal;`
│   ├── auth.rs                 — adds `OAuth2Credential` alongside StaticToken
│   ├── principal.rs (new)      — Principal enum + serde wire-shape + tests
│   └── ...                     — http/retry/paginate unchanged
└── docs/
    └── principal-mapping.md (new)  — ADR: semantic mapping, design-pressure principle, in-identifier prefix convention
```

`PermissionPredicate` in `lib.rs` changes its `principal` field type from `String` to `Principal`. Wire format is unchanged (the `Principal` serde impl produces strings).

### Drive connector (new crate)

```text
connectors/google-drive/
├── Cargo.toml
├── src/
│   ├── lib.rs              — DriveConnector + Connector impl + sync loop
│   ├── files.rs            — files.list pagination, MIME filtering, body fetch (export vs alt=media)
│   ├── resource.rs         — DriveFile → Resource projection (the connector's product)
│   ├── permissions.rs      — Drive permission objects → Principal mapping
│   └── retry_shim.rs       — 403 reason-code classification (rate-limit vs permission-denied)
└── tests/
    ├── retry_shim.rs       — classifier unit tests (12 cases)
    ├── retry_shim_e2e.rs   — wiremock e2e through send_with_drive_shim
    ├── permissions.rs      — Drive ACL JSON → Principal round-trip
    ├── list_e2e.rs         — paginated files.list + body fetch via wiremock
    └── oauth_refresh.rs    — load-bearing assertion (see §10)
```

Module split rationale: `resource.rs` owns the `DriveFile → Resource` projection — the connector's product surface. Reviewers read it first to evaluate slice shape. v0.2 Shared Drives lands by extending `resource.rs` with inheritance/merging logic. `files.rs` owns the export-vs-direct-download decision; `permissions.rs` owns the Principal mapping; `retry_shim.rs` mirrors the github crate's pattern (cross-connector shape consistency).

## 5. `OAuth2Credential` — SDK extension

Lives in `crates/sealstack-connector-sdk/src/auth.rs` alongside `StaticToken`. The load-bearing piece of the slice.

### Type shape

```rust
pub struct OAuth2Credential {
    client_id: String,                 // public; not a secret
    client_secret: SecretString,
    refresh_token: SecretString,
    token_endpoint: String,
    cache: tokio::sync::Mutex<CachedAccess>,
    inner: reqwest::Client,            // bare client, not our HttpClient (avoids circular dep)
}

#[derive(Default)]
struct CachedAccess {
    access_token: Option<SecretString>,
    valid_until: Option<std::time::Instant>,
    negative_cache: Option<NegativeCache>,
}

struct NegativeCache {
    expires: std::time::Instant,
    message: String,
}

impl OAuth2Credential {
    /// Construct against an arbitrary OAuth 2.0 token endpoint.
    pub fn new(
        client_id: impl Into<String>,
        client_secret: SecretString,
        refresh_token: SecretString,
        token_endpoint: impl Into<String>,
    ) -> SealStackResult<Self>;

    /// Convenience constructor for Google's well-known token endpoint.
    /// See also: planned `microsoft(tenant_id, ...)` and `notion(...)`
    /// constructors as those providers come online. The pattern is
    /// hardcoding well-known token endpoints while `new()` stays generic.
    pub fn google(
        client_id: impl Into<String>,
        client_secret: SecretString,
        refresh_token: SecretString,
    ) -> SealStackResult<Self> {
        Self::new(client_id, client_secret, refresh_token,
                  "https://oauth2.googleapis.com/token")
    }
}
```

Three load-bearing properties:

- **`client_id: String` is intentionally NOT `SecretString`.** OAuth client IDs are public — embedded in installed-app executables and exposed in browser URL parameters during consent. Treating them as secrets would be cargo-cult.
- **`reqwest::Client` directly, NOT our `HttpClient`.** Circular dependency: `HttpClient::send` calls `credential.authorization_header()` which would call `HttpClient` again. A plain `reqwest::Client` with 30s timeout + hand-rolled retry is the right primitive.
- **`tokio::sync::Mutex` for the cache, NOT `std::sync::Mutex`.** Per the trait docstring (committed in SDK hardening): caching impls must hold async-aware locks across `.await` points.

### Refresh flow

```rust
async fn authorization_header(&self) -> SealStackResult<String> {
    let mut cache = self.cache.lock().await;

    // Negative cache hit (refresh failed recently within 5s window).
    if let Some(neg) = &cache.negative_cache {
        if neg.expires > Instant::now() {
            return Err(SealStackError::Backend(neg.message.clone()));
        }
        cache.negative_cache = None;  // stale; clear and proceed
    }

    // Positive cache hit.
    // 60-second margin absorbs server-side validator-cache latency on
    // Google's edge — Google may treat tokens as expired slightly before
    // the reported expires_in due to refresh latency in their token
    // validators. Not network RTT (sub-second) or NTP drift (sub-second).
    if let (Some(tok), Some(until)) = (&cache.access_token, &cache.valid_until) {
        if Instant::now() + Duration::from_secs(60) < *until {
            return Ok(format!("Bearer {}", tok.expose_secret()));
        }
    }

    // Refresh.
    match self.refresh_with_retry().await {
        Ok((new_tok, expires_in)) => {
            cache.access_token = Some(SecretString::new(new_tok.clone().into()));
            cache.valid_until = Some(Instant::now() + Duration::from_secs(expires_in));
            Ok(format!("Bearer {new_tok}"))
        }
        Err(SealStackError::Unauthorized(msg)) | Err(SealStackError::Config(msg)) => {
            // Permanent failures (invalid_grant, invalid_client, invalid_scope)
            // — no negative cache; caching just delays the inevitable error.
            Err(SealStackError::Unauthorized(msg))
        }
        Err(e) => {
            // Transient (5xx, network) — negative-cache for 5s to coalesce
            // stampede. Without this: a 1.4s retry sequence with the lock
            // held serializes all concurrent waiters through their own
            // 1.4s waits, producing ~7s of fast-fail and 5 separate
            // Backend errors for a single transient outage.
            let message = format!("OAuth2 refresh failed: {e}");
            cache.negative_cache = Some(NegativeCache {
                expires: Instant::now() + Duration::from_secs(5),
                message: message.clone(),
            });
            Err(SealStackError::Backend(message))
        }
    }
}

async fn invalidate(&self) {
    let mut cache = self.cache.lock().await;
    cache.access_token = None;
    cache.valid_until = None;
    // Note: negative_cache is NOT cleared on invalidate. If a refresh just
    // failed, the next caller still gets fast-fail for the cached window.
}
```

### Token-endpoint request

```text
POST <token_endpoint>
Content-Type: application/x-www-form-urlencoded
Body: client_id=<id>&client_secret=<redacted>&refresh_token=<redacted>&grant_type=refresh_token
```

(Wire body shape per RFC 6749 §6. Placeholder `<redacted>` markers used in
this spec to match the connector's `Debug` impl behavior — anyone copying
this paragraph into a chat or issue tracker won't paste a literal-looking
secret.)

Response (200):
```json
{ "access_token": "ya29...", "expires_in": 3599, "token_type": "Bearer" }
```

Error mapping:

| Outcome | Mapping |
|---------|---------|
| 200 + valid body | Cache and return |
| 400 `{"error":"invalid_grant", ...}` | `SealStackError::Unauthorized("OAuth2 refresh failed: invalid_grant")`. No retry. **No negative cache.** |
| 400 `{"error":"invalid_client"\|"invalid_scope", ...}` | `SealStackError::Config("OAuth2 misconfiguration: <code>")`. No retry. **No negative cache.** |
| 5xx / network error | Light retry (3 attempts, exponential 200ms → 400ms → 800ms). Final → `SealStackError::Backend("OAuth2 token endpoint unreachable")`. Negative-cached. |
| Other | `Backend` with status. Negative-cached if transient. |

The light retry on the token endpoint is hand-rolled (~25 lines) — not via our `HttpClient` because of the circular-dependency issue. The hand-roll has a code comment explaining this is the *only* place in the SDK that hand-rolls retry, and that consolidation should be revisited if a second OAuth credential type appears with different retry semantics.

### One-refresh-per-request bound

The SDK performs at most one refresh-on-401 per logical `HttpClient::send` call, regardless of whether the bearer returned by `authorization_header()` after `invalidate()` is identical to the one that triggered the 401. A second 401 surfaces as `Unauthorized` immediately, without further refresh attempts. This bound is enforced by the existing `invalidated_once` flag in `HttpClient::send` (Task 7 of the SDK hardening slice).

This matters: Google can return the same access token from a refresh request inside its caching window if the token hasn't actually expired on their side. The "401 → invalidate → refresh returns same token → 401 → ..." loop is reachable; the bound prevents it.

### `Debug` impl

Manual, redacts everything except `client_id` (which is public) and `token_endpoint`. Uses `finish_non_exhaustive()` so future fields default to redacted.

### Tests (in `auth.rs::tests`)

1. `oauth2_caches_access_token` — first call refreshes; second within 1 minute hits cache (wiremock token endpoint `.expect(1)`).
2. `oauth2_refreshes_after_expiry` — `expires_in: 1`; second call after `tokio::time::pause()` past 2s refreshes again (`.expect(2)`).
3. `oauth2_skew_triggers_refresh_at_60s_before_expiry` — `expires_in: 100`; advance clock past 40s; next call refreshes. Pins the 60s skew constant.
4. `oauth2_invalidate_clears_cache` — refresh → invalidate → next call refreshes again.
5. `oauth2_invalid_grant_returns_unauthorized` — wiremock 400 invalid_grant → `Unauthorized`.
6. `oauth2_invalid_client_returns_config_error` — 400 invalid_client → `Config`.
7. `oauth2_concurrent_refresh_coalesces` — 5 concurrent tasks call `authorization_header()`; token endpoint `.expect(1)`. Refresh-coalescing assertion.
8. `oauth2_negative_cache_coalesces_transient_failures` — token endpoint returns 503 once; first call gets `Backend`; subsequent calls within 5s return same `Backend` *without* hitting the token endpoint. Wiremock `.expect(1)`.
9. `oauth2_negative_cache_clears_after_window` — after 5s+ via `tokio::time::pause()`, next call attempts refresh.
10. `oauth2_permanent_failures_do_not_negative_cache` — invalid_grant doesn't populate negative_cache; subsequent call also tries refresh and gets same error (verified by `.expect(2)`).
11. `debug_redacts_secrets` — `format!("{:?}", cred)` does NOT contain refresh_token or client_secret values; DOES contain client_id.

## 6. Drive connector structure

Lives in `connectors/google-drive/`. Mirrors the slack/github structure refined during SDK hardening.

### Config shape

The TOML binding shape (committed in question 1 as the seam for the future CLI consent slice):

```toml
[connectors.drive]
client_id              = "123-abc.apps.googleusercontent.com"
client_secret_env      = "SEALSTACK_DRIVE_CLIENT_SECRET"
refresh_token_env      = "SEALSTACK_DRIVE_REFRESH_TOKEN"
sync_interval_seconds  = 900                          # default 15 min
corpora                = "user"                       # only "user" valid in v1
api_base               = "https://www.googleapis.com" # overrideable for tests
max_file_bytes         = 10485760                     # 10 MiB default per-file cap
```

Two principles committed in question 1 as the seam to the v0.2 CLI slice: **(1) secrets are env-var references in the config file, never inline. (2) The CLI in the next slice writes to this same shape so user config does not break across versions.**

### Construction (`DriveConnector::from_json`)

```rust
pub fn from_json(v: &serde_json::Value) -> SealStackResult<Self> {
    let client_id = required_str(v, "client_id")?;
    let client_secret = SecretString::new(read_env_var(required_str(v, "client_secret_env")?)?.into());
    let refresh_token = SecretString::new(read_env_var(required_str(v, "refresh_token_env")?)?.into());

    let corpora = v.get("corpora").and_then(|x| x.as_str()).unwrap_or("user");
    if corpora != "user" {
        return Err(SealStackError::Config(format!(
            "drive: `corpora = \"{corpora}\"` not yet supported; only \"user\" works in v1. \
             Shared Drives land in v0.2."
        )));
    }

    let credential = Arc::new(OAuth2Credential::google(
        client_id, client_secret, refresh_token,
    )?);
    let http = Arc::new(
        HttpClient::new(credential, RetryPolicy::default())?
            .with_user_agent_suffix(format!(
                "google-drive-connector/{}",
                env!("CARGO_PKG_VERSION")
            )),
    );

    let api_base = v.get("api_base").and_then(|x| x.as_str())
        .unwrap_or("https://www.googleapis.com")
        .trim_end_matches('/').to_owned();
    let config = DriveConfig::from_json(v);  // sync_interval, max_file_bytes
    Ok(Self { http, config, api_base })
}
```

`read_env_var` distinguishes "env var name not specified" / "env var unset" / "env var empty" — same three-branch shape `StaticToken::from_env_result` uses.

### `Connector` trait impl

```rust
#[async_trait]
impl Connector for DriveConnector {
    fn name(&self) -> &str { "google-drive" }
    fn version(&self) -> &str { env!("CARGO_PKG_VERSION") }

    async fn list(&self) -> SealStackResult<ResourceStream> {
        // BodyCursorPaginator over files.list.
        // - q = "trashed = false and ('me' in owners or sharedWithMe)"
        //   (parens are load-bearing — without them, AND binds tighter
        //   than OR and 'me' in owners becomes the only effective owner
        //   constraint, leaking unrelated sharedWithMe items.)
        // - fields = "files(id,name,mimeType,modifiedTime,driveId,
        //             permissions(type,emailAddress,domain,role,allowFileDiscovery)),
        //             nextPageToken"
        //   (permissions inline — no per-file permissions.list round-trip.)
        // - pageSize = 1000 (Drive's max).
        // - supportsAllDrives = false (v1: My Drive only).
        //
        // Stream pipeline:
        // 1. files.list paginated via BodyCursorPaginator.
        // 2. extract_items closure filters out driveId-bearing items
        //    (Shared Drive shortcuts) with one tracing::info! per id.
        // 3. fetch_body via export-or-alt-media (files.rs).
        // 4. resource.rs projection: DriveFile + body + ACLs → Resource.
        // 5. Yield to engine.
    }

    async fn fetch(&self, id: &ResourceId) -> SealStackResult<Resource> {
        // GET <api_base>/drive/v3/files/<id>?fields=...
        // Same MIME filter + body fetch + projection as list().
    }

    async fn subscribe(&self) -> SealStackResult<Option<ChangeStream>> {
        Ok(None)  // v1 is full-crawl only; subscribe lands with v0.2 incremental.
    }

    async fn healthcheck(&self) -> SealStackResult<()> {
        // GET <api_base>/drive/v3/files?pageSize=1
        //
        // Uses files.list (NOT files/about) because files.list exercises
        // drive.readonly scope. A refresh token granted with only
        // userinfo.email scope would pass /about but fail every subsequent
        // /files.list with 403 insufficientPermissions. Healthcheck must
        // surface scope mismatches at boot, not at first sync.
        //
        // Routed through send_with_drive_shim so 403 rate-limit responses
        // surface as RateLimited rather than Backend.
    }

    fn sync_interval(&self) -> Duration {
        // Currently informational. Engine consumes a hardcoded interval
        // today; per-connector intervals land in a separate engine slice.
        Duration::from_secs(self.config.sync_interval_seconds)
    }
}
```

### Body fetch (export vs `alt=media`)

In `files.rs`:

```rust
async fn fetch_body(&self, file: &DriveFile) -> SealStackResult<Option<String>> {
    // Per-file size cap check (separate from the SDK's HTTP body cap).
    if file.size.is_some_and(|s| s > self.config.max_file_bytes) {
        self.skip_log.note_once(&file.id, || tracing::info!(
            file_id = %file.id, size = file.size,
            cap = %self.config.max_file_bytes,
            "drive: skipping file exceeding per-file size cap"
        ));
        return Ok(None);
    }

    match file.mime_type.as_str() {
        "application/vnd.google-apps.document" => {
            // Google Docs: export as text/plain.
            let url = format!("{}/drive/v3/files/{}/export", self.api_base, file.id);
            let make = || self.http.get(&url).query(&[("mimeType", "text/plain")]);
            let resp = send_with_drive_shim(&self.http, make).await?;
            let bytes = resp.bytes().await?;
            // Strict UTF-8 — Docs export contract guarantees UTF-8; a
            // violation is a Google-side bug, not a user-side mistake,
            // so error rather than silently dropping.
            String::from_utf8(bytes.to_vec())
                .map(Some)
                .map_err(|_| SealStackError::backend(
                    "drive: docs export returned non-UTF-8"
                ))
        }
        "text/plain" | "text/markdown" => {
            // Direct binary fetch.
            let url = format!("{}/drive/v3/files/{}", self.api_base, file.id);
            let make = || self.http.get(&url).query(&[("alt", "media")]);
            let resp = send_with_drive_shim(&self.http, make).await?;
            let bytes = resp.bytes().await?;
            // Strict UTF-8 — text MIME is a user-supplied claim Drive
            // doesn't validate. A non-UTF-8 file claimed as text/plain is
            // a configuration error or a deliberate skip case (binary
            // mislabeled as text). Skip without erroring; from_utf8_lossy
            // would silently embed U+FFFD pollution into the index.
            match String::from_utf8(bytes.to_vec()) {
                Ok(s) => Ok(Some(s)),
                Err(_) => {
                    self.skip_log.note_once(&file.id, || tracing::info!(
                        file_id = %file.id, mime_type = %file.mime_type,
                        "drive: skipping file with non-UTF-8 body (claimed text MIME)"
                    ));
                    Ok(None)
                }
            }
        }
        other => {
            // Skipped per v1 MIME allowlist. Log INFO once per resource id
            // (deduplicated via SkipLog). Not warn — these are deliberate
            // v1 scope decisions, not warnings.
            self.skip_log.note_once(&file.id, || tracing::info!(
                file_id = %file.id, mime_type = %other,
                "drive: skipping unsupported MIME type (v1 allowlist: docs, text, markdown)"
            ));
            Ok(None)  // None = caller drops the resource entirely; never
                      // yields empty body. Empty-body Resources are forbidden
                      // — they silently pollute the corpus.
        }
    }
}
```

`SkipLog` is a small connector-internal struct holding a `tokio::sync::Mutex<HashSet<ResourceId>>` for dedup. Reset on connector restart, scoped per-connector-instance. Promoted to SDK only if a second connector wants it.

### `try_clone()` invariants

All `make` closures use `rb.try_clone().unwrap()` patterns inside `send_with_drive_shim`. `// safe: GET with no body always clones` comment on each call site. Reqwest's `try_clone` only returns `None` when the body is non-cloneable (streaming, files); for v1 we don't issue streaming uploads.

## 7. Typed `Principal` enum (SDK §5 deferred decision)

Lives in its own file `crates/sealstack-connector-sdk/src/principal.rs`.

### Principal type shape

```rust
/// The closed set of identity kinds emitted on resource permissions.
///
/// `Principal` separates the *kind* of identity (user / group / domain /
/// anyone-public / anyone-with-link) from the *opaque identifier* within
/// each kind. The kind is what the policy engine reasons about; the
/// identifier is what the source system understands.
///
/// # Variants
///
/// - [`User`](Self::User) — individual identified by their primary email
///   from the source system.
/// - [`Group`](Self::Group) — named group whose membership is resolved by
///   the source system at policy-evaluation time. The inner string
///   typically carries a connector-prefix convention, e.g.,
///   `slack:CXXX`, for cross-connector identifier disambiguation in
///   shared policy rules. The SDK doesn't enforce this, but every
///   existing connector follows the convention.
/// - [`Domain`](Self::Domain) — anyone with an email under this domain
///   at the source.
/// - [`Anyone`](Self::Anyone) — publicly readable AND discoverable.
/// - [`AnyoneWithLink`](Self::AnyoneWithLink) — readable to whoever has
///   the resource URL; NOT discoverable. **Distinct from `Anyone` on
///   purpose** — collapsing them is a well-known search-platform bug
///   pattern (link-only docs leak into public search results).
///
/// `Hash + Eq` because the policy engine indexes resources by
/// permission-set membership; `HashSet<PermissionPredicate>` and
/// `HashMap<Principal, ...>` need to work directly.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Principal {
    User(String),
    Group(String),
    Domain(String),
    Anyone,
    AnyoneWithLink,
}
```

### Wire shape (serde)

`Serialize` produces strings:

| Variant | Wire |
|---|---|
| `User("alice@acme.com")` | `"user:alice@acme.com"` |
| `Group("eng@acme.com")` | `"group:eng@acme.com"` |
| `Domain("acme.com")` | `"domain:acme.com"` |
| `Anyone` | `"anyone"` |
| `AnyoneWithLink` | `"anyone-with-link"` |

`Deserialize` parses the same strings back. Hand-rolled (not `#[serde(tag = "kind")]`) because the wire shape is a *string*, not a tagged object — backward compat with existing storage.

**Legacy alias:** `"*"` deserializes to `Principal::Anyone`. Round-trip is asymmetric: `"*"` parses, but `Anyone` re-serializes as `"anyone"` (canonical). Connector wire data over time normalizes to the canonical form.

**Unknown prefixes error.** `"slack:CXXX"`, `"github:octocat"`, `"weird:xyz"` all error during `Deserialize` with a message that includes the offending prefix. This is the load-bearing design pressure on the connector emission paths — see §9.

### `PermissionPredicate` change

```rust
// Before:
pub struct PermissionPredicate {
    pub principal: String,
    pub action: String,
}

// After:
pub struct PermissionPredicate {
    pub principal: Principal,
    pub action: String,
}
```

Wire format of `PermissionPredicate` is unchanged because `Principal`'s serde produces strings.

### `public_read()` migration

```rust
impl PermissionPredicate {
    /// Predicate granting `read` access to anyone, discoverable in searches.
    /// Used by connectors that index inherently public content.
    pub fn public_read() -> Self {
        Self { principal: Principal::Anyone, action: "read".into() }
    }
}
```

### Drive permission mapping

`connectors/google-drive/src/permissions.rs`:

```rust
pub fn drive_permission_to_predicate(p: &DrivePermission) -> Option<PermissionPredicate> {
    let principal = match p.kind.as_str() {
        "user" => p.email_address.as_ref().map(|e| Principal::User(e.clone()))?,
        "group" => p.email_address.as_ref().map(|e| Principal::Group(e.clone()))?,
        "domain" => p.domain.as_ref().map(|d| Principal::Domain(d.clone()))?,
        "anyone" => {
            // CRITICAL: ambiguous discovery defaults to link-only (not discoverable).
            // Treating absence as `Anyone` would be the Glean-class bug — link-only
            // docs leaking into public search results.
            let discoverable = p.allow_file_discovery.unwrap_or(false);
            if discoverable { Principal::Anyone } else { Principal::AnyoneWithLink }
        }
        other => {
            tracing::warn!(kind = %other, "drive: unrecognized permission kind, dropping ACL entry");
            return None;
        }
    };
    Some(PermissionPredicate {
        principal,
        action: drive_role_to_action(&p.role).into(),
    })
}

/// Project a Drive role to a SealStack action.
///
/// # Read-tier mapping rationale
///
/// `commenter` projects to `read` because the connector ingests document
/// bodies, not comments. A commenter's write-side capability (creating
/// comments visible to other readers) is not a write to our indexed
/// content. Comment threads are not ingested in v1.
///
/// `fileOrganizer` (Shared Drives, future v0.2) projects to `read` because
/// its write-side is moving files between folders, not modifying content.
fn drive_role_to_action(role: &str) -> &'static str {
    match role {
        "reader" | "commenter" | "fileOrganizer" => "read",
        "writer" | "owner" | "organizer" => "write",
        other => {
            // Conservative for indexing (better to over-allow a search than
            // to silently exclude a real reader). May under-grant for
            // write-capable operations; the connector projects unknown roles
            // permissively at the read tier.
            tracing::warn!(role = %other, "drive: unrecognized role, defaulting to `read`");
            "read"
        }
    }
}
```

Call site in `resource.rs` drops bad ACL entries individually, never bails the whole resource:

```rust
let perms: Vec<PermissionPredicate> = drive_perms.iter()
    .filter_map(drive_permission_to_predicate)
    .collect();
```

### Tests (in `principal.rs::tests`)

1. `user_round_trip` — `Principal::User("alice@acme.com")` ↔ `"user:alice@acme.com"`.
2. `group_round_trip`.
3. `domain_round_trip`.
4. `anyone_round_trip`.
5. `anyone_with_link_round_trip`.
6. `legacy_star_deserializes_as_anyone` — `"*"` parses to `Anyone`.
7. `legacy_star_with_write_action_still_deserializes` — alias works for any action.
8. `legacy_star_round_trip_normalizes_to_anyone` — legacy parses, re-serializes canonical (asymmetric in the right direction).
9. `unknown_kind_error_includes_prefix` — `"weird:something"` returns serde error mentioning the prefix.
10. `missing_colon_errors_on_deserialize` — `"justastring"` returns serde error.
11. `predicate_full_round_trip` — full `PermissionPredicate` JSON round-trip.

### Tests (in `connectors/google-drive/tests/permissions.rs`)

- `drive_user_permission_maps`, `drive_group_permission_maps`, `drive_domain_permission_maps`.
- `drive_anyone_discoverable` — `allowFileDiscovery: true` → `Anyone`.
- `drive_anyone_link_only` — `allowFileDiscovery: false` → `AnyoneWithLink`.
- `drive_anyone_missing_discovery_defaults_to_link_only` — field absent → `AnyoneWithLink` (the conservative default).
- `drive_unknown_kind_returns_none` — `type="unknown"` → `None` + warn.
- `drive_role_projection_table` — table-driven covering reader/commenter/fileOrganizer→read; writer/owner/organizer→write; unknown→read+warn.

## 8. Drive 403 retry shim

Lives in `connectors/google-drive/src/retry_shim.rs`. Mirrors github's pattern (commit `6248b0d` from SDK hardening) with one extra branch for daily-quota.

### Drive403Action type shape

```rust
#[derive(Debug)]
pub enum Drive403Action {
    /// Short-term rate limit exceeded. Retry with exponential backoff
    /// (500ms × 2^attempt), up to 5 attempts. After budget exhaustion,
    /// surface as RateLimited.
    BackoffThenRetry,
    /// Daily quota exhausted. Retrying buys nothing until UTC midnight.
    /// Surface as RateLimited immediately.
    QuotaExhausted,
    /// Permission denied. Surface as Backend with comma-joined reasons
    /// for diagnostic context.
    PermissionDenied { reason: String },
}

#[must_use]
pub fn classify_drive_403(_headers: &[(String, String)], body: &str) -> Drive403Action {
    let parsed: serde_json::Value =
        serde_json::from_str(body).unwrap_or(serde_json::Value::Null);
    let reasons: Vec<&str> = parsed
        .pointer("/error/errors")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter()
            .filter_map(|e| e.get("reason").and_then(|r| r.as_str()))
            .collect())
        .unwrap_or_default();

    // Daily quota wins over short-term rate-limit when both signals are
    // present: retrying a daily-quota-exhausted endpoint buys nothing.
    if reasons.iter().any(|r| matches!(*r, "quotaExceeded" | "dailyLimitExceeded")) {
        return Drive403Action::QuotaExhausted;
    }
    if reasons.iter().any(|r| matches!(*r, "userRateLimitExceeded" | "rateLimitExceeded")) {
        return Drive403Action::BackoffThenRetry;
    }
    // Comma-join all reasons in PermissionDenied for full diagnostic
    // context. Operator-visible message: "drive 403: permission denied
    // (domainPolicy,insufficientPermissions)".
    let reason = if reasons.is_empty() {
        "(no reason in body)".to_owned()
    } else {
        reasons.join(",")
    };
    Drive403Action::PermissionDenied { reason }
}
```

The `_headers` parameter is unused in v1 (Drive doesn't typically emit `Retry-After` on 403); kept in the signature to mirror github's shape so future provider shims can copy it wholesale.

### `send_with_drive_shim` wrapper

```rust
async fn send_with_drive_shim<F>(http: &HttpClient, make_request: F) -> SealStackResult<HttpResponse>
where F: Fn() -> reqwest::RequestBuilder
{
    let mut attempt = 0u32;
    loop {
        match http.send(make_request()).await {
            Ok(resp) => return Ok(resp),
            Err(SealStackError::HttpStatus { status: 403, headers, body }) => {
                match classify_drive_403(&headers, &body) {
                    Drive403Action::BackoffThenRetry if attempt < 4 => {
                        // Backoff: 500ms, 1s, 2s, 4s. Cumulative ~7.5s.
                        //
                        // Drive's per-user rate-limit window is 100s; this
                        // schedule reaches roughly 7.5% of one window. A
                        // schedule that consistently spans the full window
                        // (e.g., 500ms, 1s, 2s, 4s, 8s = 15.5s cumulative)
                        // would recover from a higher fraction of legitimate
                        // rate-limits but at the cost of doubled worst-case
                        // latency. Revisit if pilot telemetry shows budget-
                        // exhaustion at meaningful rates.
                        let delay = Duration::from_millis(500 * (1u64 << attempt));
                        // Demoted to debug — first attempts in a backoff loop
                        // are the system working as designed, not warnings.
                        // warn is reserved for budget exhaustion below.
                        tracing::debug!(
                            ?delay, attempt,
                            "drive: 403 rate-limit, backing off before retry"
                        );
                        tokio::time::sleep(delay).await;
                        attempt += 1;
                    }
                    Drive403Action::BackoffThenRetry => {
                        tracing::warn!(
                            attempts = 5,
                            "drive: 403 rate-limit retry budget exhausted"
                        );
                        return Err(SealStackError::RateLimited);
                    }
                    Drive403Action::QuotaExhausted => {
                        // v0.2: include X-Goog-Quota-Limit/X-Goog-Quota-Used
                        // headers in this log line for operator diagnosis
                        // ("you're at 95% of your daily quota").
                        tracing::warn!(
                            "drive: daily quota exhausted, not retrying until UTC midnight"
                        );
                        return Err(SealStackError::RateLimited);
                    }
                    Drive403Action::PermissionDenied { reason } => {
                        return Err(SealStackError::Backend(format!(
                            "drive 403: permission denied ({reason})"
                        )));
                    }
                }
            }
            Err(e) => return Err(e),
        }
    }
}
```

Loop-with-explicit-returns shape (no `unreachable!`) — every path either `return`s or `attempt += 1`s. Cheaper alternative to fall-through-then-`unreachable!()` because there's no panic site to defend.

### No jitter in v1

Unlike the SDK's main retry loop, the Drive shim has no jitter. Drive's per-user rate-limit is per-OAuth-credential, not per-region. Synchronized retries across multiple connector instances each hit their own per-user limit independently; jitter buys nothing. Single-connector sequential retries also don't benefit from jitter (no contention with self).

If a future deployment runs many connector replicas sharing one OAuth credential (sharded ingestion), the per-user limit would funnel them through one queue and jitter would help. Flagged in non-goals.

### Pagination paths bypass the shim

`BodyCursorPaginator` driving `files.list` calls `client.send(rb)` internally — these do NOT route through the shim. Matches github precedent (Task 19 of SDK hardening). When pagination hits 403 mid-stream, the surfaced behavior diverges from healthcheck/list/fetch (where the shim catches it). v0.2 SDK work to push the shim down into paginators fixes both connectors at once.

### Tests (in `tests/retry_shim.rs`)

12 classifier unit tests:
1. `user_rate_limit_classifies_as_backoff`.
2. `rate_limit_exceeded_classifies_as_backoff`.
3. `quota_exceeded_classifies_as_quota_exhausted`.
4. `daily_limit_exceeded_classifies_as_quota_exhausted`.
5. `forbidden_classifies_as_permission_denied`.
6. `insufficient_permissions_classifies_as_permission_denied`.
7. `domain_policy_classifies_as_permission_denied`.
8. `app_not_authorized_classifies_as_permission_denied`.
9. `quota_wins_over_rate_limit_when_both_present` — pins the priority rule.
10. `malformed_body_classifies_as_permission_denied` — fall-through to `PermissionDenied { reason: "(no reason in body)" }`.
11. `empty_body_classifies_as_permission_denied`.
12. `permission_denied_includes_all_reasons` — multi-reason responses comma-joined in the `reason` field.

### Tests (in `tests/retry_shim_e2e.rs`)

5 wiremock end-to-end tests through `healthcheck`:
1. `user_rate_limit_retries_and_succeeds`.
2. `rate_limit_exhausted_returns_rate_limited` — 5 consecutive 403s with `userRateLimitExceeded`. Uses `tokio::time::pause()` to keep test runtime sane.
3. `quota_exhausted_returns_rate_limited_immediately` — `.expect(1)`, no retry.
4. `permission_denied_returns_backend_immediately` — `.expect(1)`, no retry.
5. `permission_denied_includes_reason` — assert error message contains the reason string for operator diagnostics.

## 9. Implementation sequence + migration

### PR 1 commit-by-commit

#### Commit 1: `feat(connector-sdk): typed Principal enum with serde wire-shape`

New `crates/sealstack-connector-sdk/src/principal.rs`. Enum, `Serialize`/`Deserialize`, `"*"` legacy alias for `Anyone`, all 11 unit tests. `pub use principal::Principal;` re-exported from `lib.rs`. **Tests-only commit** — the wire shape is established but not yet consumed by `PermissionPredicate` (still `String` at this point). Workspace stays green.

#### Commit 2: `feat(connector-sdk): PermissionPredicate.principal: String → Principal (with local-files migration)`

The breaking type change. `PermissionPredicate::public_read()` migrates to `Principal::Anyone`. Local-files migrates *in this commit* because the change is a single-line constructor swap with **no wire-format change** — `public_read()` still emits `"anyone"`, and the legacy `"*"` deserializer alias means existing test fixtures keep working without modification. Slack and github migrate in subsequent commits because their wire formats genuinely change (`"slack:CXXX"` → `"group:slack:CXXX"`), warranting separate, atomic, fixture-regenerating commits.

**At this commit, slack and github stop compiling** because `format!("slack:{id}")` and `format!("github:{owner}")` produce `String`, not `Principal`. Commits 3 and 4 fix them. The workspace gate moves to "compiles after commit 4" — bisect across PR 1 must land on commit 1 (workspace green) or commit 4+ (workspace green), not on intermediate states.

#### Commits 3 and 4: `refactor(slack)` and `refactor(github)`: emit `Principal::Group`

One-line code change in resource construction:

```rust
// slack:
principal: Principal::Group(format!("slack:{}", channel.id)),
// github:
principal: Principal::Group(format!("github:{}", owner)),
```

**Fixture regeneration is the bigger part of these commits.** "Test fixtures regenerated" hides real work — every wiremock fixture and snapshot test asserting an emitted Resource's permissions needs to be touched. Estimated 10–20 fixture files apiece across both connectors.

**Workspace-wide grep checklist before merging each commit:**

```bash
# Verify no third-location wire-shape references survived.
# The `| grep -v 'group:'` excludes intentional new-format hits like
# "group:slack:CXXX" and surfaces only the un-migrated bare prefixes.
rg '"slack:' --type rust  | grep -v 'group:slack:'
rg '"github:' --type rust | grep -v 'group:github:'
```

The targeted grep catches the silent-break failure mode: an unintentional surviving reference in engine-side test fixtures, gateway integration tests, or any other third-location consumer that the per-connector migration commit didn't touch. A bare `"slack:CXXX"` outside the slack crate after commit 3 lands is a regression to investigate, not a hit to hand-wave.

#### Commit 5: `feat(connector-sdk): OAuth2Credential with refresh, caching, negative-cache`

Additive: new `OAuth2Credential` in `auth.rs` alongside `StaticToken`. 11 tests in `auth.rs::tests`. Lands last in PR 1 because:
- Decoupled from the type-system migration in commits 1–4 (a hypothetical revert touches only one of the two).
- Workspace stays green at every prior commit; OAuth landing last means the breaking change has already settled.

#### Commit 6: `docs(connector-sdk): principal-mapping ADR`

New `crates/sealstack-connector-sdk/docs/principal-mapping.md` (~50 lines). Codifies:

- **The mapping is semantic, not lexical.** Pick the variant whose semantics match the source concept, not the most permissive variant.
- **Worked examples per variant.** Slack channel → `Group`. GitHub org/user → `Group` (group of users authorized at that namespace). Notion workspace member → `User` (individual identifier). Salesforce role → `Group`. Domain-restricted Drive → `Domain`.
- **In-identifier source-prefix convention.** Connectors emitting `Group` identifiers should prefix them with their connector name (`slack:CXXX`, `github:octocat`) for cross-connector disambiguation in shared policy rules. Convention, not contract.
- **The escape valve.** When a source ACL primitive *genuinely* doesn't fit any variant, the answer is to open a design discussion to extend the enum, not to shove it into stringly-typed territory.
- **The design-pressure principle, stated explicitly:** *"When a connector's source has ACL primitives that don't obviously fit the closed set, the resolution is a deliberate semantic-mapping decision, not an escape hatch. If no variant fits, the conversation is 'should the SDK extend its closed set?' — surfaced as a design proposal, not papered over."*

### PR 2 commit-by-commit

#### Commit 1: `feat(connectors/google-drive): scaffold + DriveConfig + from_json`

Crate scaffold (`Cargo.toml` modeled on github's), `DriveConfig` struct, `from_json` constructor implementing the env-var resolution + corpora-not-`"user"` rejection + `api_base` override default. `OAuth2Credential::google()` construction wired via `HttpClient::new(...).with_user_agent_suffix("google-drive-connector/<ver>")`. Empty `Connector` trait impl with stubbed methods (compile-time scaffold; subsequent commits fill the methods). 4 unit tests covering construction-error paths (missing required fields, corpora rejected, env-var unset/empty, api_base normalization).

#### Commit 2: `feat(google-drive): Drive permission mapping (DrivePermission → Principal)`

`permissions.rs` with `drive_permission_to_predicate` + `drive_role_to_action`. Pure mapping — no HTTP, no Connector wiring. Lands before `files.rs` because `resource.rs` (commit 6) consumes the mapping, and isolating it as its own commit gives the reviewer a clean target for the variant-by-variant review of the Principal mapping decisions. 8 unit tests covering each variant + role projection table + missing-discovery default + unknown-kind warn-and-drop.

#### Commit 3: `feat(google-drive): files.list pagination + MIME allowlist + SkipLog`

`files.rs` scaffolds the `BodyCursorPaginator<DriveFile, _, _, _>` over `files.list`. Includes the load-bearing `q` parameter (with the parens that bind `'me' in owners or sharedWithMe` correctly under `and trashed=false`), the `fields=...` projection that pulls permissions inline, the `extract_items` filter that drops `driveId`-bearing items with one info log per id, and the `SkipLog` connector-internal struct for MIME-skip dedup. No body fetch yet (commit 4) and no Connector trait wiring (commit 6). Wiremock test for paginated `files.list` + `driveId` filter + cursor advancement.

#### Commit 4: `feat(google-drive): body fetch (export vs alt=media) with strict UTF-8 + per-file cap`

`fetch_body` method in `files.rs`. Pre-flight per-file size cap check (`max_file_bytes` config, default 10 MiB). Three-arm match on MIME: Google Docs export with strict-UTF-8-or-Backend (export contract violation is a Google-side bug, not a user mistake); text/plain + text/markdown with strict-UTF-8-or-skip-info (text MIME is a user-supplied claim Drive doesn't validate); fallthrough skip with one info per resource id. Wiremock tests for each arm, including the non-UTF-8-text → skip path and the export-violation → Backend path.

#### Commit 5: `feat(google-drive): retry_shim for 403 three-class discrimination`

`retry_shim.rs` with `Drive403Action`, `classify_drive_403`, and `send_with_drive_shim`. Lands before the Connector impl (commit 6) so `healthcheck` can route through the shim from the moment it exists — there's never a point where a healthcheck against a rate-limited Drive endpoint produces a misleading `Backend` error. 12 classifier unit tests covering all 4 reason classes + priority ordering + malformed body + comma-joined diagnostic.

#### Commit 6: `feat(google-drive): DriveFile → Resource projection + Connector impl + e2e tests`

`resource.rs` owns the `DriveFile + body + ACLs → Resource` projection — the connector's product. `lib.rs` ties the Connector trait impl together: `list()` drives the paginator from commit 3, fetches bodies via commit 4, projects via `resource.rs`, yields the stream. `fetch()` does the per-id variant. `healthcheck()` routes through `send_with_drive_shim` (commit 5) against `/files?pageSize=1`. The closing commit's e2e test files exercise everything that came before:

- `tests/list_e2e.rs` — paginated `files.list` + body fetch via wiremock; asserts the full pipeline yields the expected `Resource` shapes.
- `tests/oauth_refresh.rs` — **the load-bearing acceptance criterion** (§10).
- `tests/retry_shim_e2e.rs` — 5 cases through `healthcheck`.
- `tests/permissions.rs` — already landed in commit 2; revalidated as part of the full suite.

The PR 2 sequencing isn't arbitrary: commit 2 (permissions) and commit 5 (shim) land before commit 6 (Connector impl) so the impl can use them; commit 3 (pagination) before commit 4 (body fetch) because pagination is the data source; commit 1 (scaffold) first so subsequent commits compile against an existing crate skeleton.

### Migration scope statement

> **This slice predates v1 release.** Storage of `PermissionPredicate` records exists only in dev environments and integration test fixtures, both of which are reset on schema changes. The wire-format changes in commits 2–4 (`"slack:CXXX"` → `"group:slack:CXXX"`, `"github:octocat"` → `"group:github:octocat"`) can land without a one-shot Postgres migration script, because no production deployment carries pre-Principal wire data. A future v1 deployment that subsequently extends `Principal` (adding a new variant) would require a migration script for stored receipts; this slice does not.

### Asymmetric legacy-alias treatment

> The `Deserialize` impl on `Principal` accepts `"*"` as a synonym for `Anyone`. This is a **semantic rename** of an unambiguous existing wire shape — `"*"` always meant "anyone" in `PermissionPredicate::public_read()`'s old impl, and the alias preserves backward-compat with test fixtures and dev wire data without effort.
>
> The `Deserialize` impl does **NOT** accept `"slack:..."` or `"github:..."` (or any other connector-specific prefix) as legacy aliases. Those were stringly-typed identifiers without typed semantic anchoring; aliasing them would silently re-map data and **mask the design pressure** this slice is exerting on the connector emission paths. Connectors must explicitly migrate to the typed enum; pre-migration wire data fails to deserialize loudly, which is the correct signal.

## 10. Load-bearing acceptance criterion

The OAuth refresh path is exercised end-to-end in `connectors/google-drive/tests/oauth_refresh.rs`. A wiremock server returns 401 with `WWW-Authenticate: Bearer error="invalid_token"` on the first request to a Drive endpoint. The connector's middleware invokes `OAuth2Credential::invalidate()`, then `authorization_header()` performs a token-endpoint refresh against a separate wiremock server, then the original request retries with the new bearer and succeeds.

A sibling test exercises the failure mode: the token endpoint returns 400 `invalid_grant`, and the SDK surfaces `SealStackError::Unauthorized` rather than a generic `Backend` error.

Together these assert that the SDK's new `OAuth2Credential` impl integrates correctly with the existing `Credential` trait's `invalidate` contract. **If this test passes, the slice has done its load-bearing job. If it fails, no other test result matters.**

The whole question 1 reasoning ("Drive forces the OAuth shape to be right; without Drive, we're guessing at the API") gets validated or invalidated by these two tests. This is the criterion that justifies the slice's scope decisions all the way back to the SDK hardening's deferral of OAuth.

## 11. Testing strategy summary

### Per-crate test counts

- **`sealstack-connector-sdk`** — 42 existing (28 lib + 10 `http_retry` + 1 each of `paginate_body_cursor`/`paginate_link_header`/`paginate_offset` + 1 doctest) + 11 new (`Principal`) + 11 new (`OAuth2Credential`) = 64 tests.
- **`sealstack-connector-local-files`** — 5 unchanged.
- **`sealstack-connector-slack`** — 9 (5 unit + 4 wiremock e2e); fixture wire shapes regenerated.
- **`sealstack-connector-github`** — 16 (5 unit + 4 retry_shim classifier + 4 retry_shim e2e + 3 list_repos e2e); fixture wire shapes regenerated.
- **`sealstack-connector-google-drive`** — new crate. ~25 tests:
  - `tests/retry_shim.rs` — 12 classifier units.
  - `tests/retry_shim_e2e.rs` — 5 wiremock e2e through `healthcheck`.
  - `tests/permissions.rs` — 8 ACL mapping cases.
  - `tests/list_e2e.rs` — paginated `files.list` + body fetch via wiremock.
  - `tests/oauth_refresh.rs` — **load-bearing acceptance criterion** (§10).

### Workspace gates

- `cargo test -p <each>` clean.
- `cargo clippy -p <each>` clean (only pre-existing `doc_markdown` warnings on lib.rs docstrings outside slice scope).
- `cargo fmt --check -p <each>` clean.
- **No `cargo check --workspace`** — pre-existing `sealstack-policy-runtime` host-target compilation issue from PR #20 is unrelated and tracked separately.

## 12. Dependencies

**Zero new direct workspace dependencies.** Everything needed is already present from the SDK hardening slice:

- `secrecy` (token redaction), `wiremock` (test infra), `reqwest`, `tokio::sync::Mutex`, `serde`/`serde_json`, `async-trait`, `time`, `tracing` — all present.

The Drive connector crate's new `Cargo.toml` mirrors github's shape — local path deps on `sealstack-common` and `sealstack-connector-sdk`, plus the workspace deps already in use across other connectors.

A reviewer's first instinct on a new connector crate is "what new deps am I auditing?" — and "none" is the cleanest answer possible.

## 13. Out-of-scope (consolidated)

Items deferred to follow-up slices, each tied back to where it was decided:

- **CLI consent flow** (`sealstack auth google`) — Q1. Separate slice. Writes to the same config-file env-var-ref shape this slice establishes.
- **Shared Drives** (`corpora=shared|all`) — Q5. v0.2. `corpora` field accepted but rejected at validation; `driveId`-bearing items skipped at extract_items.
- **File formats beyond plain text + Google Docs** — Q2. Future tightly-scoped slices, OAuth path proven.
- **Incremental sync via `changes.list`** — Q4. v0.2. Layered on top of full-crawl; state file optional.
- **Receipt-render presence-check** (gateway-side `files.get` on receipt view) — Q4. Pairs with v0.2 incremental sync.
- **Engine integration of per-connector `sync_interval`** — separate engine slice.
- **Pagination paths through 403 retry shim** — matches github precedent. v0.2 SDK work pushes shim into paginators.
- **`Principal::Custom` escape hatch** — Q6. Explicitly rejected. New variants land via design discussion.
- **HTTP-date `Retry-After` parsing** — already pinned in SDK as integer-seconds-only.
- **Multi-replica connector deployment with shared OAuth credentials** — backoff has no jitter on single-replica assumption.
- **Promote `SkipLog` to SDK** — connector-local in v1; promote when **N=2 connectors** want it. The trigger is a second connector with a non-MIME skip rule (a github connector that skips PRs with `.pull_request.is_some()`, a slack connector that skips bot messages with `.subtype == "bot_message"`); when that case lands, copy the slack/github precedent of "skip rule lives at the connector boundary, dedup helper lives in the SDK." Until then, keeping `SkipLog` connector-local matches the slice's discipline of not promoting abstractions on N=1.
- **`X-Goog-Quota-Limit`/`X-Goog-Quota-Used` headers in quota-exhausted log output** — operator-diagnostic improvement, v0.2.

## 14. Acceptance summary

This slice is shipped when:

1. PR 1 lands all 6 commits with workspace green at each (modulo bisect-aware rebase across commits 2–4).
2. PR 2 lands all 6 commits with `cargo test -p sealstack-connector-google-drive` green.
3. **The load-bearing acceptance criterion (§10) passes** — the OAuth refresh e2e test asserts the slice has done its load-bearing job.
4. Per-crate clippy + fmt checks are clean for every touched crate.
5. The workspace-wide grep checklist (§9, commits 3 and 4) shows no surviving references to the old `"slack:..."` / `"github:..."` wire shapes outside intentional migration sites.
6. The principal-mapping ADR (§9, commit 6) lands so the next OAuth connector author has the design-pressure principle on first read.
