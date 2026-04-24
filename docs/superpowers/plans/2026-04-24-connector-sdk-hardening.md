# Connector SDK Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split `sealstack-connector-sdk` into focused modules, add a `Credential` trait + `StaticToken` impl, an `HttpClient` with reactive-retry middleware, a `Paginator` trait with three reference builders, then refactor `local-files`, `slack`, and `github` onto them.

**Architecture:** Six sequenced phases (see spec §3). SDK infrastructure lands first with re-exports preserving existing public paths; connector refactors land second, smallest-blast-radius first (local-files, slack, github). GitHub's 403 shim is GitHub-specific code in the github crate, not generic middleware.

**Tech Stack:** Rust 2021, `reqwest` 0.12, `async-trait` 0.1, `futures` 0.3, `tokio` 1.42, `secrecy` (new), `wiremock` (new dev-dep), `tracing`, `time`.

**Spec:** [docs/superpowers/specs/2026-04-24-connector-sdk-hardening-design.md](../specs/2026-04-24-connector-sdk-hardening-design.md)

---

## Phase 1 — `Credential` trait + `StaticToken`

### Task 1: Module skeleton + re-exports

Split the flat `lib.rs` into `lib.rs + auth.rs + change_streams.rs`. No behavior change — existing `Connector`, `Resource`, etc. still reachable from `sealstack_connector_sdk::*`.

**Files:**
- Modify: `crates/sealstack-connector-sdk/src/lib.rs`
- Create: `crates/sealstack-connector-sdk/src/change_streams.rs`
- Create: `crates/sealstack-connector-sdk/src/auth.rs` (empty skeleton for next task)

- [ ] **Step 1: Extract `change_streams` helpers to their own file**

Move the `pub mod change_streams { ... }` block from `lib.rs` into a new file `change_streams.rs`. The content is unchanged.

Create `crates/sealstack-connector-sdk/src/change_streams.rs`:

```rust
//! Helpers for building the stream types the [`super::Connector`] trait returns.

use super::{ChangeEvent, ChangeStream, Resource, ResourceStream};
use futures::stream;

/// Build a [`ResourceStream`] from an owned `Vec`.
///
/// Fine for connectors whose source fits in memory. Large sources should
/// implement a lazy stream directly.
#[must_use]
pub fn resource_stream(resources: Vec<Resource>) -> ResourceStream {
    Box::pin(stream::iter(resources))
}

/// Build a [`ChangeStream`] from an owned `Vec`.
#[must_use]
pub fn change_stream(events: Vec<ChangeEvent>) -> ChangeStream {
    Box::pin(stream::iter(events))
}
```

- [ ] **Step 2: Create empty `auth.rs` module**

Create `crates/sealstack-connector-sdk/src/auth.rs`:

```rust
//! Authentication primitives for connectors.
//!
//! v1 ships `StaticToken` (PATs, bot tokens, API keys). Future modules add
//! OAuth 2.0 authorization-code + refresh, Google service-account JWTs, etc.,
//! each as an additional [`Credential`] implementation.

// Trait + impl land in Task 2.
```

- [ ] **Step 3: Update `lib.rs` to declare the new modules and drop the inline `change_streams`**

Replace the existing `pub mod change_streams { ... }` block near the bottom of `lib.rs` with module declarations. Everything else (the `Connector` trait, `Resource`, `ResourceId`, `PermissionPredicate`, `ChangeEvent`) stays exactly as it is today.

Add near the top of `lib.rs` (after the `#![forbid]` / `#![warn]` lines and before the existing `use` block):

```rust
pub mod auth;
pub mod change_streams;
```

Remove the inline `pub mod change_streams { ... }` block.

- [ ] **Step 4: Verify build + existing tests still pass**

Run: `cargo test -p sealstack-connector-sdk`
Expected: all existing tests pass (resource_stream_helper_roundtrips, permission_public_read_round_trips, resource_id_display).

- [ ] **Step 5: Verify existing connectors still compile**

Run: `cargo check --workspace`
Expected: clean compile. `connectors/github`, `connectors/slack`, `connectors/local-files` continue to `use sealstack_connector_sdk::{...}` at the same paths.

- [ ] **Step 6: Commit**

```bash
git add crates/sealstack-connector-sdk/src/
git commit -m "refactor(connector-sdk): split change_streams + seed auth module"
```

---

### Task 2: `Credential` trait + `StaticToken` impl

**Files:**
- Modify: `crates/sealstack-connector-sdk/Cargo.toml` (add `secrecy`)
- Modify: `crates/sealstack-connector-sdk/src/auth.rs`

- [ ] **Step 1: Add `secrecy` dependency**

Modify `crates/sealstack-connector-sdk/Cargo.toml`. In the `[dependencies]` table, add:

```toml
secrecy = "0.10"
```

Run: `cargo check -p sealstack-connector-sdk`
Expected: clean compile; `secrecy` downloaded.

- [ ] **Step 2: Write the failing tests for `StaticToken`**

Append to `crates/sealstack-connector-sdk/src/auth.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn static_token_emits_bearer_header() {
        let t = StaticToken::new("abc123");
        assert_eq!(t.authorization_header().await.unwrap(), "Bearer abc123");
    }

    #[tokio::test]
    async fn invalidate_is_noop_by_default() {
        let t = StaticToken::new("abc123");
        t.invalidate().await;
        assert_eq!(t.authorization_header().await.unwrap(), "Bearer abc123");
    }

    #[test]
    fn debug_redacts_secret() {
        let t = StaticToken::new("super-secret-value");
        let s = format!("{t:?}");
        assert!(!s.contains("super-secret-value"), "Debug leaked: {s}");
        assert!(s.contains("StaticToken"));
    }

    #[test]
    fn from_env_reports_missing_distinctly_from_empty() {
        // Use a name that is guaranteed not to be set.
        let missing = "SEALSTACK_TEST_NOPE_NEVER_SET_XYZ";
        // SAFETY: env ops are unsafe in Rust 2024; single-threaded test context.
        unsafe { std::env::remove_var(missing) };
        let err = StaticToken::from_env(missing).unwrap_err().to_string();
        assert!(err.contains("not set"), "{err}");

        unsafe { std::env::set_var("SEALSTACK_TEST_EMPTY_XYZ", "") };
        let err = StaticToken::from_env("SEALSTACK_TEST_EMPTY_XYZ")
            .unwrap_err()
            .to_string();
        assert!(err.contains("is empty"), "{err}");
        unsafe { std::env::remove_var("SEALSTACK_TEST_EMPTY_XYZ") };
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p sealstack-connector-sdk --lib auth::tests`
Expected: FAIL — `Credential` / `StaticToken` not defined.

- [ ] **Step 4: Implement `Credential` + `StaticToken`**

Replace the skeleton in `crates/sealstack-connector-sdk/src/auth.rs` (keep the existing `//!` module doc-comment and the `#[cfg(test)]` block from Step 2; only the non-test code changes):

```rust
//! Authentication primitives for connectors.
//!
//! v1 ships `StaticToken` (PATs, bot tokens, API keys). Future modules add
//! OAuth 2.0 authorization-code + refresh, Google service-account JWTs, etc.,
//! each as an additional [`Credential`] implementation.

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};

use sealstack_common::{SealStackError, SealStackResult};

/// Source of the `Authorization` header value for an outbound request.
///
/// # Contract
///
/// `authorization_header` returns the full header value including the scheme
/// prefix (e.g. `"Bearer abc123"`). v1 implementations always use the bearer
/// scheme; future non-bearer schemes (Basic, HMAC-signed) add new
/// implementations without changing the trait.
///
/// Per-request allocation is intentional: OAuth's lock-guarded refresh path
/// requires owned `String` values to cross `.await` safely. The ~50-byte
/// clone is dominated by HTTP transport costs.
///
/// Caching implementations (e.g. OAuth) must use async-aware synchronization
/// primitives (`tokio::sync::Mutex`, `arc-swap`, etc.) to avoid holding
/// locks across `.await` points.
#[async_trait]
pub trait Credential: Send + Sync + 'static {
    /// Returns the full `Authorization` header value, including scheme prefix.
    async fn authorization_header(&self) -> SealStackResult<String>;

    /// Invalidate any cached credential material.
    ///
    /// Called by `HttpClient` before retrying a 401. Default is a no-op for
    /// credentials that cannot refresh (e.g. [`StaticToken`]).
    async fn invalidate(&self) {}
}

/// Long-lived static bearer token (PAT, bot token, API key).
///
/// Token material is held in a [`SecretString`] so it zeroes on drop and
/// cannot be accidentally printed via `Debug`.
pub struct StaticToken(SecretString);

impl StaticToken {
    /// Build from any string value.
    pub fn new(token: impl Into<String>) -> Self {
        Self(SecretString::from(token.into()))
    }

    /// Read a token from an environment variable.
    ///
    /// Distinguishes "variable not set" from "variable set to empty string"
    /// with two distinct error messages — both surface as
    /// [`SealStackError::Config`].
    pub fn from_env(var: &str) -> SealStackResult<Self> {
        match std::env::var(var) {
            Err(_) => Err(SealStackError::Config(format!(
                "env var `{var}` not set"
            ))),
            Ok(s) if s.is_empty() => Err(SealStackError::Config(format!(
                "env var `{var}` is empty"
            ))),
            Ok(s) => Ok(Self::new(s)),
        }
    }
}

impl std::fmt::Debug for StaticToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("StaticToken").field(&"<redacted>").finish()
    }
}

#[async_trait]
impl Credential for StaticToken {
    async fn authorization_header(&self) -> SealStackResult<String> {
        Ok(format!("Bearer {}", self.0.expose_secret()))
    }
    // invalidate() default no-op preserves v1 "same token → same 401 →
    // Unauthorized" semantic.
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p sealstack-connector-sdk --lib auth::tests`
Expected: all four auth tests pass.

- [ ] **Step 6: Verify workspace still builds**

Run: `cargo check --workspace`
Expected: clean compile.

- [ ] **Step 7: Commit**

```bash
git add crates/sealstack-connector-sdk/Cargo.toml crates/sealstack-connector-sdk/src/auth.rs
git commit -m "feat(connector-sdk): Credential trait + StaticToken impl"
```

---

## Phase 2 — `HttpClient` + retry middleware

### Task 3: Extend `SealStackError` with new variants

Add `RetryExhausted`, `BodyTooLarge`, `PaginatorCursorLoop` to the shared error enum. Kept in Phase 2 because `HttpClient` is where the first two are produced; the third is produced in Phase 3 but is cheap to add now alongside.

**Files:**
- Modify: `crates/sealstack-common/src/lib.rs`
- Test: `crates/sealstack-common/src/lib.rs` (inline `#[cfg(test)]` if absent, else append)

- [ ] **Step 1: Write the failing tests**

Append inside the `#[cfg(test)] mod tests { ... }` block in `crates/sealstack-common/src/lib.rs` (create the block if not present):

```rust
#[test]
fn retry_exhausted_renders_attempts_and_duration() {
    use std::time::Duration;
    let e = SealStackError::RetryExhausted {
        attempts: 5,
        total_duration: Duration::from_millis(7500),
        last_error: Box::new(SealStackError::Backend("502 bad gateway".into())),
    };
    let msg = e.to_string();
    assert!(msg.contains("5"), "missing attempts: {msg}");
    assert!(msg.contains("502"), "missing last_error detail: {msg}");
}

#[test]
fn body_too_large_reports_cap() {
    let e = SealStackError::BodyTooLarge { cap_bytes: 52_428_800 };
    assert!(e.to_string().contains("52428800"));
}

#[test]
fn paginator_cursor_loop_reports_cursor() {
    let e = SealStackError::PaginatorCursorLoop {
        cursor: "abc".into(),
    };
    assert!(e.to_string().contains("abc"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p sealstack-common`
Expected: FAIL — three new variants undefined.

- [ ] **Step 3: Add the variants**

Modify `crates/sealstack-common/src/lib.rs`. Add inside the `SealStackError` enum (after the existing `Other(String)` variant):

```rust
    /// HTTP retry loop exhausted its attempt budget.
    #[error(
        "retry exhausted after {attempts} attempts over {total_duration:?}: {last_error}"
    )]
    RetryExhausted {
        /// Number of attempts made.
        attempts: u32,
        /// Wall time elapsed across all attempts.
        total_duration: std::time::Duration,
        /// Final error observed.
        last_error: Box<SealStackError>,
    },

    /// Response body exceeded the configured size cap.
    #[error("response body exceeded cap: {cap_bytes} bytes")]
    BodyTooLarge {
        /// The cap that was exceeded, in bytes.
        cap_bytes: usize,
    },

    /// Paginator returned the same cursor twice consecutively.
    #[error("paginator cursor did not advance: {cursor}")]
    PaginatorCursorLoop {
        /// The repeated cursor value.
        cursor: String,
    },
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p sealstack-common`
Expected: PASS.

- [ ] **Step 5: Verify workspace still builds**

Run: `cargo check --workspace`
Expected: clean compile (no match-exhaustiveness regression — all existing callers use `_ =>` fallthroughs or specific arms).

- [ ] **Step 6: Commit**

```bash
git add crates/sealstack-common/src/lib.rs
git commit -m "feat(common): error variants for retry/body-cap/cursor-loop"
```

---

### Task 4: `RetryPolicy` type + `Retry-After` parser

**Files:**
- Create: `crates/sealstack-connector-sdk/src/retry.rs`
- Modify: `crates/sealstack-connector-sdk/src/lib.rs` (declare module)
- Modify: `crates/sealstack-connector-sdk/Cargo.toml` (add `reqwest`, `rand`)

- [ ] **Step 1: Add dependencies**

Modify `crates/sealstack-connector-sdk/Cargo.toml`. In `[dependencies]`, add:

```toml
reqwest = { workspace = true }
rand    = "0.8"
```

Run: `cargo check -p sealstack-connector-sdk`
Expected: clean compile.

- [ ] **Step 2: Declare the module**

Add to `crates/sealstack-connector-sdk/src/lib.rs` (alongside the other `pub mod` declarations):

```rust
pub mod retry;
```

- [ ] **Step 3: Write failing tests for `Retry-After` parsing**

Create `crates/sealstack-connector-sdk/src/retry.rs`:

```rust
//! Retry policy for [`super::http::HttpClient`].
//!
//! Exports the [`RetryPolicy`] configuration type and the [`parse_retry_after`]
//! helper. The retry loop itself lives in `http.rs` because it is tightly
//! integrated with request sending.

use std::time::Duration;

/// Reactive retry policy.
///
/// Applied by `HttpClient` to every outbound request. See the spec §6 for the
/// backoff schedule and the 401 invalidate-retry rule.
#[derive(Clone, Debug)]
pub struct RetryPolicy {
    /// Maximum total attempts (1 initial + up to max_attempts-1 retries).
    pub max_attempts: u32,
    /// Base delay for exponential backoff.
    pub base_delay: Duration,
    /// Cap on any single sleep between attempts.
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
        }
    }
}

/// Parse a `Retry-After` header value.
///
/// v1 supports integer seconds only (the form used by GitHub, Slack, Stripe,
/// and the vast majority of servers). HTTP-date form (`"Wed, 21 Oct 2015
/// 07:28:00 GMT"`) returns `None`, and the caller falls back to exponential
/// backoff — acceptable because servers that send HTTP-date also send
/// integer seconds in sibling headers, and exponential backoff is a safe
/// fallback. HTTP-date support is a follow-up if observed need warrants it.
///
/// Returns `None` for negative or unparseable values.
pub fn parse_retry_after(value: &str) -> Option<Duration> {
    let trimmed = value.trim();
    let secs = trimmed.parse::<i64>().ok()?;
    if secs < 0 {
        return None;
    }
    Some(Duration::from_secs(secs as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_retry_after_integer_seconds() {
        assert_eq!(parse_retry_after("30"), Some(Duration::from_secs(30)));
        assert_eq!(parse_retry_after(" 0 "), Some(Duration::from_secs(0)));
    }

    #[test]
    fn parse_retry_after_rejects_negative() {
        assert_eq!(parse_retry_after("-1"), None);
    }

    #[test]
    fn parse_retry_after_rejects_garbage() {
        assert_eq!(parse_retry_after("soon"), None);
    }

    #[test]
    fn parse_retry_after_http_date_returns_none_in_v1() {
        // v1 supports integer seconds only; HTTP-date falls through to
        // exponential backoff rather than being parsed.
        assert_eq!(
            parse_retry_after("Wed, 21 Oct 2099 07:28:00 GMT"),
            None,
        );
    }

    #[test]
    fn default_policy_matches_spec() {
        let p = RetryPolicy::default();
        assert_eq!(p.max_attempts, 5);
        assert_eq!(p.base_delay, Duration::from_millis(500));
        assert_eq!(p.max_delay, Duration::from_secs(30));
    }
}
```

- [ ] **Step 4: Run tests to verify they pass (implementation is inline above)**

Run: `cargo test -p sealstack-connector-sdk --lib retry::tests`
Expected: all six pass.

- [ ] **Step 5: Commit**

```bash
git add crates/sealstack-connector-sdk/Cargo.toml crates/sealstack-connector-sdk/src/retry.rs crates/sealstack-connector-sdk/src/lib.rs
git commit -m "feat(connector-sdk): RetryPolicy + Retry-After parser"
```

---

### Task 5: `HttpClient` scaffold (no retry yet)

Ship the type, constructor, and User-Agent handling. The retry loop lands in Task 6.

**Files:**
- Create: `crates/sealstack-connector-sdk/src/http.rs`
- Modify: `crates/sealstack-connector-sdk/src/lib.rs` (declare module)

- [ ] **Step 1: Declare the module**

Add to `crates/sealstack-connector-sdk/src/lib.rs`:

```rust
pub mod http;
```

- [ ] **Step 2: Write failing tests for the scaffold**

Create `crates/sealstack-connector-sdk/src/http.rs`:

```rust
//! HTTP client wrapper with auth injection, UA composition, and retry.
//!
//! The retry policy is baked in: every request through [`HttpClient`] goes
//! through the policy from [`super::retry::RetryPolicy`]. There is no
//! non-retrying request path in v1.

use std::sync::Arc;

use sealstack_common::{SealStackError, SealStackResult};

use crate::auth::Credential;
use crate::retry::RetryPolicy;

/// Hard upper bound on the response body-size cap, in bytes (500 MiB).
///
/// Configuring [`HttpClient::with_body_cap`] above this is a configuration
/// error — misconfiguration cannot disable DoS protection entirely.
pub const MAX_BODY_CAP_BYTES: usize = 500 * 1024 * 1024;

/// Default response body-size cap, in bytes (50 MiB).
pub const DEFAULT_BODY_CAP_BYTES: usize = 50 * 1024 * 1024;

/// Connector-side HTTP client.
pub struct HttpClient {
    inner: reqwest::Client,
    credential: Arc<dyn Credential>,
    retry: RetryPolicy,
    user_agent: String,
    body_cap_bytes: usize,
}

impl HttpClient {
    /// Base User-Agent without suffix.
    fn base_ua() -> String {
        format!("sealstack-connector-sdk/{} (rust)", env!("CARGO_PKG_VERSION"))
    }

    /// Build a client with the given credential and retry policy.
    pub fn new(
        credential: Arc<dyn Credential>,
        retry: RetryPolicy,
    ) -> SealStackResult<Self> {
        let inner = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| SealStackError::backend(format!("reqwest client build: {e}")))?;
        Ok(Self {
            inner,
            credential,
            retry,
            user_agent: Self::base_ua(),
            body_cap_bytes: DEFAULT_BODY_CAP_BYTES,
        })
    }

    /// Append a connector-identifying suffix to the User-Agent.
    ///
    /// Produces e.g. `sealstack-connector-sdk/1.0.0 (rust) github-connector/0.1.0`.
    #[must_use]
    pub fn with_user_agent_suffix(mut self, suffix: impl Into<String>) -> Self {
        self.user_agent = format!("{} {}", Self::base_ua(), suffix.into());
        self
    }

    /// Configure the response body-size cap.
    ///
    /// Capped at [`MAX_BODY_CAP_BYTES`] — requests above this return
    /// [`SealStackError::Config`].
    pub fn with_body_cap(mut self, cap_bytes: usize) -> SealStackResult<Self> {
        if cap_bytes > MAX_BODY_CAP_BYTES {
            return Err(SealStackError::Config(format!(
                "body cap {cap_bytes} exceeds hard maximum {MAX_BODY_CAP_BYTES}"
            )));
        }
        self.body_cap_bytes = cap_bytes;
        Ok(self)
    }

    /// Returns the composed User-Agent string (for tests + diagnostics).
    #[must_use]
    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }

    /// Returns the current body cap, in bytes (for tests + diagnostics).
    #[must_use]
    pub fn body_cap_bytes(&self) -> usize {
        self.body_cap_bytes
    }

    /// Begin a GET request.
    ///
    /// Callers chain `.query()`, `.header()`, etc. and then pass to
    /// [`HttpClient::send`] to execute under the retry policy.
    pub fn get(&self, url: impl reqwest::IntoUrl) -> reqwest::RequestBuilder {
        self.inner.get(url)
    }

    /// Begin a POST request.
    pub fn post(&self, url: impl reqwest::IntoUrl) -> reqwest::RequestBuilder {
        self.inner.post(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::StaticToken;

    fn test_client() -> HttpClient {
        HttpClient::new(
            Arc::new(StaticToken::new("t")),
            RetryPolicy::default(),
        )
        .unwrap()
    }

    #[test]
    fn user_agent_has_expected_shape() {
        let c = test_client();
        let ua = c.user_agent();
        assert!(ua.starts_with("sealstack-connector-sdk/"));
        assert!(ua.contains("(rust)"));
    }

    #[test]
    fn user_agent_suffix_appended() {
        let c = test_client().with_user_agent_suffix("github-connector/0.1.0");
        assert!(c.user_agent().ends_with(" github-connector/0.1.0"));
    }

    #[test]
    fn body_cap_rejects_above_hard_max() {
        let c = test_client();
        let too_big = MAX_BODY_CAP_BYTES + 1;
        let err = c.with_body_cap(too_big).unwrap_err().to_string();
        assert!(err.contains("hard maximum"), "{err}");
    }

    #[test]
    fn body_cap_default_is_50_mib() {
        let c = test_client();
        assert_eq!(c.body_cap_bytes(), 50 * 1024 * 1024);
    }
}
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test -p sealstack-connector-sdk --lib http::tests`
Expected: all four pass.

- [ ] **Step 4: Commit**

```bash
git add crates/sealstack-connector-sdk/src/http.rs crates/sealstack-connector-sdk/src/lib.rs
git commit -m "feat(connector-sdk): HttpClient scaffold with UA + body-cap"
```

---

### Task 6: Retry loop — 5xx, 408, 429, network errors

Implement `HttpClient::send` with the exponential-backoff path. 401 handling comes in Task 7; body-cap in Task 8.

**Files:**
- Modify: `crates/sealstack-connector-sdk/src/http.rs`
- Modify: `crates/sealstack-connector-sdk/Cargo.toml` (add `wiremock` dev-dep)

- [ ] **Step 1: Add `wiremock` as dev-dep**

Modify `crates/sealstack-connector-sdk/Cargo.toml`. In `[dev-dependencies]`:

```toml
wiremock = "0.6"
```

- [ ] **Step 2: Write the failing integration tests**

Create `crates/sealstack-connector-sdk/tests/http_retry.rs`:

```rust
//! Integration tests for [`HttpClient`] retry behavior.
//!
//! These tests spin up a local HTTP server via `wiremock`. If CI ever
//! restricts port binding, see `mockito` as the documented fallback.

use std::sync::Arc;
use std::time::Duration;

use sealstack_common::SealStackError;
use sealstack_connector_sdk::auth::StaticToken;
use sealstack_connector_sdk::http::HttpClient;
use sealstack_connector_sdk::retry::RetryPolicy;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn tight_policy() -> RetryPolicy {
    // Short delays so tests don't drag.
    RetryPolicy {
        max_attempts: 4,
        base_delay: Duration::from_millis(10),
        max_delay: Duration::from_millis(200),
    }
}

async fn client(server: &MockServer) -> HttpClient {
    let _ = server; // url is passed per-request
    HttpClient::new(Arc::new(StaticToken::new("t")), tight_policy()).unwrap()
}

#[tokio::test]
async fn happy_path_200() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ok"))
        .respond_with(ResponseTemplate::new(200).set_body_string("hi"))
        .mount(&server)
        .await;

    let c = client(&server).await;
    let rb = c.get(format!("{}/ok", server.uri()));
    let resp = c.send(rb).await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn fivehundred_then_ok_retries_and_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/flaky"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/flaky"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let c = client(&server).await;
    let rb = c.get(format!("{}/flaky", server.uri()));
    let resp = c.send(rb).await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn fivehundred_all_attempts_returns_retry_exhausted() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/always5xx"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let c = client(&server).await;
    let rb = c.get(format!("{}/always5xx", server.uri()));
    let err = c.send(rb).await.unwrap_err();
    match err {
        SealStackError::RetryExhausted { attempts, .. } => {
            assert_eq!(attempts, 4);
        }
        other => panic!("expected RetryExhausted, got {other}"),
    }
}

#[tokio::test]
async fn fourhundred_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/bad"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1) // exactly one call — no retries.
        .mount(&server)
        .await;

    let c = client(&server).await;
    let rb = c.get(format!("{}/bad", server.uri()));
    let err = c.send(rb).await.unwrap_err();
    assert!(
        matches!(err, SealStackError::Backend(_)),
        "404 should map to Backend, got {err}"
    );
}

#[tokio::test]
async fn respects_retry_after_on_429() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/throttle"))
        .respond_with(
            ResponseTemplate::new(429).append_header("Retry-After", "0"),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/throttle"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let c = client(&server).await;
    let rb = c.get(format!("{}/throttle", server.uri()));
    let resp = c.send(rb).await.unwrap();
    assert_eq!(resp.status(), 200);
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p sealstack-connector-sdk --test http_retry`
Expected: FAIL — `HttpClient::send` does not exist.

- [ ] **Step 4: Add `HttpResponse` wrapper and the retry `send` implementation**

Modify `crates/sealstack-connector-sdk/src/http.rs`. Add at the top (after the existing `use` block):

```rust
use std::time::Instant;

use rand::Rng;

use crate::retry::parse_retry_after;
```

Append the following after the existing `impl HttpClient { ... }`:

```rust
/// Wrapped HTTP response returned by [`HttpClient::send`].
///
/// The body-size cap is enforced by the `bytes` / `json` accessors (Task 8).
pub struct HttpResponse {
    inner: reqwest::Response,
    body_cap_bytes: usize,
}

impl HttpResponse {
    /// HTTP status code.
    pub fn status(&self) -> reqwest::StatusCode {
        self.inner.status()
    }

    /// Access a response header value.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.inner
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
    }

    /// Consume the response and yield the underlying `reqwest::Response`.
    ///
    /// Escape hatch for callers that want full access before the body-cap
    /// machinery lands.
    pub fn into_inner(self) -> reqwest::Response {
        self.inner
    }

    /// Expose the body cap for downstream body-reading helpers.
    pub(crate) fn body_cap_bytes(&self) -> usize {
        self.body_cap_bytes
    }
}

impl HttpClient {
    /// Execute a request under the retry policy.
    ///
    /// Injects the `Authorization` header from the configured [`Credential`]
    /// and the client's `User-Agent`. Applies retry logic per the policy.
    /// See the spec §6 for the retry-decision table.
    pub async fn send(
        &self,
        rb: reqwest::RequestBuilder,
    ) -> SealStackResult<HttpResponse> {
        let start = Instant::now();
        let mut attempt: u32 = 0;
        let mut last_err: Option<SealStackError> = None;

        loop {
            let try_rb = rb
                .try_clone()
                .ok_or_else(|| SealStackError::backend("request body not cloneable"))?;
            let auth = self.credential.authorization_header().await?;
            let req = try_rb
                .header("Authorization", auth)
                .header("User-Agent", &self.user_agent);

            let result = req.send().await;
            attempt += 1;

            match result {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        return Ok(HttpResponse {
                            inner: resp,
                            body_cap_bytes: self.body_cap_bytes,
                        });
                    }
                    // 4xx non-retryable (except 408, 429 — handled below).
                    if status.is_client_error()
                        && status != reqwest::StatusCode::REQUEST_TIMEOUT
                        && status != reqwest::StatusCode::TOO_MANY_REQUESTS
                    {
                        return Err(SealStackError::Backend(format!(
                            "HTTP {}: {}",
                            status.as_u16(),
                            status.canonical_reason().unwrap_or("")
                        )));
                    }
                    // Retryable: 408, 429, 5xx.
                    let delay = retry_delay_for(
                        &self.retry,
                        attempt - 1,
                        resp.headers().get("Retry-After").and_then(|v| v.to_str().ok()),
                    );
                    last_err = Some(SealStackError::Backend(format!(
                        "HTTP {} (attempt {attempt})",
                        status.as_u16()
                    )));
                    if attempt >= self.retry.max_attempts {
                        break;
                    }
                    tokio::time::sleep(delay).await;
                }
                Err(e) => {
                    last_err = Some(SealStackError::backend(format!("network: {e}")));
                    if attempt >= self.retry.max_attempts {
                        break;
                    }
                    let delay = retry_delay_for(&self.retry, attempt - 1, None);
                    tokio::time::sleep(delay).await;
                }
            }
        }

        Err(SealStackError::RetryExhausted {
            attempts: attempt,
            total_duration: start.elapsed(),
            last_error: Box::new(
                last_err.unwrap_or_else(|| SealStackError::backend("unknown")),
            ),
        })
    }
}

/// Compute the next sleep duration.
///
/// - If `retry_after` is present and parseable, use `min(max_delay,
///   retry_after + jitter(0..1000ms))`.
/// - Otherwise exponential: `delay = min(max_delay, base * 2^attempt)`, then
///   full-jitter with `rand(0..delay)`.
fn retry_delay_for(
    policy: &RetryPolicy,
    attempt: u32,
    retry_after_header: Option<&str>,
) -> std::time::Duration {
    use std::time::Duration;

    if let Some(raw) = retry_after_header {
        if let Some(base) = parse_retry_after(raw) {
            let jitter_ms = rand::thread_rng().gen_range(0..1000);
            let with_jitter = base + Duration::from_millis(jitter_ms);
            return std::cmp::min(policy.max_delay, with_jitter);
        }
    }

    let shift = attempt.min(20); // cap at 2^20 to avoid overflow
    let exp = policy
        .base_delay
        .saturating_mul(1u32.checked_shl(shift).unwrap_or(u32::MAX));
    let capped = std::cmp::min(policy.max_delay, exp);
    let jittered_ms = rand::thread_rng().gen_range(0..=capped.as_millis() as u64);
    Duration::from_millis(jittered_ms)
}

#[cfg(test)]
mod retry_delay_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn respects_max_delay_cap() {
        let p = RetryPolicy {
            max_attempts: 5,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(500),
        };
        // Even at a huge attempt count, delay ≤ max_delay.
        for _ in 0..20 {
            let d = retry_delay_for(&p, 30, None);
            assert!(d <= p.max_delay);
        }
    }

    #[test]
    fn retry_after_dominates_exponential() {
        let p = RetryPolicy::default();
        let d = retry_delay_for(&p, 0, Some("2"));
        assert!(d >= Duration::from_secs(2));
        assert!(d < Duration::from_secs(4)); // 2s + <1s jitter
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p sealstack-connector-sdk`
Expected: all unit + integration tests (http_retry) pass.

- [ ] **Step 6: Commit**

```bash
git add crates/sealstack-connector-sdk/Cargo.toml crates/sealstack-connector-sdk/src/http.rs crates/sealstack-connector-sdk/tests/http_retry.rs
git commit -m "feat(connector-sdk): HttpClient::send with reactive retry"
```

---

### Task 7: 401 invalidate-once retry path

**Files:**
- Modify: `crates/sealstack-connector-sdk/src/http.rs` (extend `send`)
- Modify: `crates/sealstack-connector-sdk/tests/http_retry.rs` (add tests)

- [ ] **Step 1: Add a 401 test using a refreshable credential double**

Append to `crates/sealstack-connector-sdk/tests/http_retry.rs`:

```rust
use async_trait::async_trait;
use sealstack_connector_sdk::auth::Credential;
use std::sync::atomic::{AtomicU32, Ordering};

struct CountingCredential {
    invalidations: AtomicU32,
    tokens: Vec<&'static str>,
}

#[async_trait]
impl Credential for CountingCredential {
    async fn authorization_header(&self) -> sealstack_common::SealStackResult<String> {
        let n = self.invalidations.load(Ordering::SeqCst) as usize;
        let t = self.tokens.get(n).unwrap_or(&"exhausted");
        Ok(format!("Bearer {t}"))
    }
    async fn invalidate(&self) {
        self.invalidations.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn fourhundred_one_triggers_invalidate_and_retries_once() {
    let server = MockServer::start().await;
    // First request: token-1 → 401. Second: token-2 → 200.
    Mock::given(method("GET"))
        .and(path("/auth"))
        .and(wiremock::matchers::header("Authorization", "Bearer token-1"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/auth"))
        .and(wiremock::matchers::header("Authorization", "Bearer token-2"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let cred = Arc::new(CountingCredential {
        invalidations: AtomicU32::new(0),
        tokens: vec!["token-1", "token-2"],
    });
    let client = HttpClient::new(cred.clone(), tight_policy()).unwrap();
    let rb = client.get(format!("{}/auth", server.uri()));
    let resp = client.send(rb).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(cred.invalidations.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn second_fourhundred_one_returns_unauthorized() {
    let server = MockServer::start().await;
    // Always 401 regardless of token.
    Mock::given(method("GET"))
        .and(path("/locked"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let cred = Arc::new(CountingCredential {
        invalidations: AtomicU32::new(0),
        tokens: vec!["t1", "t2"],
    });
    let client = HttpClient::new(cred.clone(), tight_policy()).unwrap();
    let rb = client.get(format!("{}/locked", server.uri()));
    let err = client.send(rb).await.unwrap_err();
    assert!(
        matches!(err, SealStackError::Unauthorized(_)),
        "expected Unauthorized, got {err}"
    );
    // Exactly one invalidation; second 401 is final.
    assert_eq!(cred.invalidations.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn static_token_401_returns_unauthorized_without_retry_loop() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/locked"))
        .respond_with(ResponseTemplate::new(401))
        .expect(2) // initial + one invalidate-retry; no more.
        .mount(&server)
        .await;

    let cred = Arc::new(StaticToken::new("t"));
    let client = HttpClient::new(cred, tight_policy()).unwrap();
    let rb = client.get(format!("{}/locked", server.uri()));
    let err = client.send(rb).await.unwrap_err();
    assert!(matches!(err, SealStackError::Unauthorized(_)));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p sealstack-connector-sdk --test http_retry -- fourhundred_one second_fourhundred static_token_401`
Expected: FAIL — current `send` maps all 4xx to `Backend` uniformly.

- [ ] **Step 3: Implement the 401 invalidate-once path**

Modify `crates/sealstack-connector-sdk/src/http.rs` in `HttpClient::send`. Replace the `Ok(resp) => { ... }` match arm inside the retry `loop` with:

```rust
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        return Ok(HttpResponse {
                            inner: resp,
                            body_cap_bytes: self.body_cap_bytes,
                        });
                    }
                    // 401 — invalidate-once escape hatch for OAuth-like creds.
                    if status == reqwest::StatusCode::UNAUTHORIZED {
                        if !invalidated_once {
                            tracing::warn!(
                                attempt,
                                "401 received; invalidating credential and retrying once"
                            );
                            self.credential.invalidate().await;
                            invalidated_once = true;
                            continue; // no budget consumed, no sleep
                        }
                        return Err(SealStackError::Unauthorized(format!(
                            "HTTP 401 after credential invalidation"
                        )));
                    }
                    // 4xx non-retryable (except 408, 429 — handled below).
                    if status.is_client_error()
                        && status != reqwest::StatusCode::REQUEST_TIMEOUT
                        && status != reqwest::StatusCode::TOO_MANY_REQUESTS
                    {
                        return Err(SealStackError::Backend(format!(
                            "HTTP {}: {}",
                            status.as_u16(),
                            status.canonical_reason().unwrap_or("")
                        )));
                    }
                    // Retryable: 408, 429, 5xx.
                    let delay = retry_delay_for(
                        &self.retry,
                        attempt - 1,
                        resp.headers().get("Retry-After").and_then(|v| v.to_str().ok()),
                    );
                    last_err = Some(SealStackError::Backend(format!(
                        "HTTP {} (attempt {attempt})",
                        status.as_u16()
                    )));
                    if attempt >= self.retry.max_attempts {
                        break;
                    }
                    tokio::time::sleep(delay).await;
                }
```

Add at the top of `HttpClient::send`, right after the `let mut last_err ...` line:

```rust
        let mut invalidated_once = false;
```

Also note: the `continue` path above bypasses the budget-increment. Adjust by moving `attempt += 1` out of the single `match` arm. Replace the `attempt += 1;` line with:

```rust
            // Only count against the retry budget if we got a response or a
            // network error that consumes the attempt. 401-invalidation uses
            // `continue` before this increment so it does not consume budget.
```

And move the actual `attempt += 1;` into each arm after the early-continue point. Concretely the final `send` body reads:

```rust
    pub async fn send(
        &self,
        rb: reqwest::RequestBuilder,
    ) -> SealStackResult<HttpResponse> {
        let start = Instant::now();
        let mut attempt: u32 = 0;
        let mut last_err: Option<SealStackError> = None;
        let mut invalidated_once = false;

        loop {
            let try_rb = rb
                .try_clone()
                .ok_or_else(|| SealStackError::backend("request body not cloneable"))?;
            let auth = self.credential.authorization_header().await?;
            let req = try_rb
                .header("Authorization", auth)
                .header("User-Agent", &self.user_agent);

            match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        return Ok(HttpResponse {
                            inner: resp,
                            body_cap_bytes: self.body_cap_bytes,
                        });
                    }
                    if status == reqwest::StatusCode::UNAUTHORIZED {
                        if !invalidated_once {
                            tracing::warn!(
                                attempt,
                                "401 received; invalidating credential and retrying once"
                            );
                            self.credential.invalidate().await;
                            invalidated_once = true;
                            continue;
                        }
                        return Err(SealStackError::Unauthorized(
                            "HTTP 401 after credential invalidation".into(),
                        ));
                    }
                    attempt += 1;
                    if status.is_client_error()
                        && status != reqwest::StatusCode::REQUEST_TIMEOUT
                        && status != reqwest::StatusCode::TOO_MANY_REQUESTS
                    {
                        return Err(SealStackError::Backend(format!(
                            "HTTP {}: {}",
                            status.as_u16(),
                            status.canonical_reason().unwrap_or("")
                        )));
                    }
                    let delay = retry_delay_for(
                        &self.retry,
                        attempt - 1,
                        resp.headers().get("Retry-After").and_then(|v| v.to_str().ok()),
                    );
                    last_err = Some(SealStackError::Backend(format!(
                        "HTTP {} (attempt {attempt})",
                        status.as_u16()
                    )));
                    if attempt >= self.retry.max_attempts {
                        break;
                    }
                    tokio::time::sleep(delay).await;
                }
                Err(e) => {
                    attempt += 1;
                    last_err = Some(SealStackError::backend(format!("network: {e}")));
                    if attempt >= self.retry.max_attempts {
                        break;
                    }
                    let delay = retry_delay_for(&self.retry, attempt - 1, None);
                    tokio::time::sleep(delay).await;
                }
            }
        }

        Err(SealStackError::RetryExhausted {
            attempts: attempt,
            total_duration: start.elapsed(),
            last_error: Box::new(
                last_err.unwrap_or_else(|| SealStackError::backend("unknown")),
            ),
        })
    }
```

Replace the existing `send` with the body above.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p sealstack-connector-sdk`
Expected: all tests pass, including the three new 401 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/sealstack-connector-sdk/src/http.rs crates/sealstack-connector-sdk/tests/http_retry.rs
git commit -m "feat(connector-sdk): 401 invalidate-once retry path"
```

---

### Task 8: Body-size cap via streaming

**Files:**
- Modify: `crates/sealstack-connector-sdk/src/http.rs` (`HttpResponse::bytes`, `HttpResponse::json`)
- Modify: `crates/sealstack-connector-sdk/tests/http_retry.rs` (add test)

- [ ] **Step 1: Write the failing test**

Append to `crates/sealstack-connector-sdk/tests/http_retry.rs`:

```rust
#[tokio::test]
async fn body_cap_rejects_oversized_response() {
    let server = MockServer::start().await;
    let big = vec![b'x'; 2048];
    Mock::given(method("GET"))
        .and(path("/big"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(big))
        .mount(&server)
        .await;

    let cred = Arc::new(StaticToken::new("t"));
    let client = HttpClient::new(cred, tight_policy())
        .unwrap()
        .with_body_cap(1024) // smaller than response
        .unwrap();
    let rb = client.get(format!("{}/big", server.uri()));
    let resp = client.send(rb).await.unwrap();
    let err = resp.bytes().await.unwrap_err();
    assert!(
        matches!(err, SealStackError::BodyTooLarge { cap_bytes: 1024 }),
        "expected BodyTooLarge {{ cap_bytes: 1024 }}, got {err}"
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p sealstack-connector-sdk --test http_retry -- body_cap_rejects_oversized_response`
Expected: FAIL — `HttpResponse::bytes` does not exist.

- [ ] **Step 3: Implement `HttpResponse::bytes` + `HttpResponse::json`**

Modify `crates/sealstack-connector-sdk/src/http.rs`. Add `futures::StreamExt;` import and then, inside `impl HttpResponse`, append:

```rust
    /// Read the response body into memory, enforcing the body-size cap via
    /// streaming read.
    ///
    /// Returns [`SealStackError::BodyTooLarge`] if the running total exceeds
    /// the cap mid-stream. The response connection is dropped on overrun.
    pub async fn bytes(self) -> SealStackResult<bytes::Bytes> {
        use futures::StreamExt;

        let cap = self.body_cap_bytes;
        let mut stream = self.inner.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk
                .map_err(|e| SealStackError::backend(format!("body stream: {e}")))?;
            if buf.len() + chunk.len() > cap {
                return Err(SealStackError::BodyTooLarge { cap_bytes: cap });
            }
            buf.extend_from_slice(&chunk);
        }
        Ok(bytes::Bytes::from(buf))
    }

    /// Read the response body as JSON, enforcing the body-size cap via
    /// streaming read.
    pub async fn json<T: serde::de::DeserializeOwned>(self) -> SealStackResult<T> {
        let bytes = self.bytes().await?;
        serde_json::from_slice(&bytes)
            .map_err(|e| SealStackError::backend(format!("json parse: {e}")))
    }
```

Also add `bytes = "1"` under `[dependencies]` in `crates/sealstack-connector-sdk/Cargo.toml` (reqwest already depends on it transitively but we need it directly for the return type).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p sealstack-connector-sdk`
Expected: all pass, including `body_cap_rejects_oversized_response`.

- [ ] **Step 5: Commit**

```bash
git add crates/sealstack-connector-sdk/Cargo.toml crates/sealstack-connector-sdk/src/http.rs crates/sealstack-connector-sdk/tests/http_retry.rs
git commit -m "feat(connector-sdk): body-size cap via streaming read"
```

---

## Phase 3 — `Paginator` + reference builders

### Task 9: `Paginator` trait + `paginate()` stream adapter

**Files:**
- Create: `crates/sealstack-connector-sdk/src/paginate.rs`
- Modify: `crates/sealstack-connector-sdk/src/lib.rs` (declare module)

- [ ] **Step 1: Declare the module**

Add to `crates/sealstack-connector-sdk/src/lib.rs`:

```rust
pub mod paginate;
```

- [ ] **Step 2: Write the failing tests**

Create `crates/sealstack-connector-sdk/src/paginate.rs`:

```rust
//! Pagination primitives for connectors.
//!
//! **Users should reach for one of the three reference builders first:**
//! - [`BodyCursorPaginator`] — cursor is in the response body (Slack, Drive,
//!   Notion, Linear).
//! - [`LinkHeaderPaginator`] — cursor is in a `Link: rel="next"` header
//!   (GitHub, GitLab).
//! - [`OffsetPaginator`] — numeric `start`/`limit` with a total (Jira,
//!   Confluence).
//!
//! The [`Paginator`] trait is the extension point for APIs that do not fit
//! any of these three patterns (e.g. Stripe's `starts_with`).

use std::sync::Arc;

use async_trait::async_trait;
use futures::Stream;

use sealstack_common::{SealStackError, SealStackResult};

use crate::http::HttpClient;

/// A paginator drives the SDK's stream adapter, returning one page of
/// deserialized items at a time plus the cursor for the next page (or
/// `None` to terminate).
///
/// # Contract
///
/// - Empty pages with a valid next cursor are expected; implementations must
///   not short-circuit on `items.is_empty()`. Continue until `next_cursor ==
///   None`.
/// - Once [`Paginator::fetch_page`] returns `Err`, the paginator is
///   considered poisoned. The stream adapter will not call `fetch_page`
///   again. Implementations do not need to handle re-entry after failure.
/// - Returning the same cursor twice consecutively is a paginator bug.
///   The adapter detects this and returns
///   [`SealStackError::PaginatorCursorLoop`].
#[async_trait]
pub trait Paginator: Send + 'static {
    /// Item yielded by the stream — typically a deserialized DTO.
    type Item: Send + 'static;

    /// Fetch one page, given the cursor returned by the previous page.
    ///
    /// `None` on the first call; subsequent calls pass the previous call's
    /// returned cursor. Returns items in the page plus the next cursor
    /// (`None` when exhausted).
    async fn fetch_page(
        &mut self,
        client: &HttpClient,
        cursor: Option<String>,
    ) -> SealStackResult<(Vec<Self::Item>, Option<String>)>;
}

/// Drive a [`Paginator`] to exhaustion.
///
/// NOTE: paginator ownership is `&mut self` + adapter-owns-by-value. Do not
/// add a `Clone` bound later — paginators may hold non-cloneable or
/// expensive-to-clone state (TCP sessions, dedupe sets).
pub fn paginate<P: Paginator>(
    mut paginator: P,
    client: Arc<HttpClient>,
) -> std::pin::Pin<Box<dyn Stream<Item = SealStackResult<P::Item>> + Send>> {
    Box::pin(async_stream::try_stream! {
        let mut cursor: Option<String> = None;
        let mut prev_cursor: Option<String> = None;
        loop {
            let (items, next) = paginator.fetch_page(&client, cursor.clone()).await?;
            for it in items {
                yield it;
            }
            match next {
                None => break,
                Some(n) => {
                    if prev_cursor.as_deref() == Some(n.as_str()) {
                        Err(SealStackError::PaginatorCursorLoop { cursor: n })?;
                    }
                    prev_cursor = cursor;
                    cursor = Some(n);
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::StaticToken;
    use crate::retry::RetryPolicy;
    use futures::StreamExt;

    /// Yields two pages then exhausts.
    struct TwoPage {
        calls: u32,
    }

    #[async_trait]
    impl Paginator for TwoPage {
        type Item = u32;
        async fn fetch_page(
            &mut self,
            _c: &HttpClient,
            cursor: Option<String>,
        ) -> SealStackResult<(Vec<u32>, Option<String>)> {
            self.calls += 1;
            match cursor.as_deref() {
                None => Ok((vec![1, 2], Some("p2".into()))),
                Some("p2") => Ok((vec![3, 4], None)),
                Some(other) => panic!("unexpected cursor {other}"),
            }
        }
    }

    /// Yields an empty page with a next cursor, then the real page.
    struct EmptyThenFull;

    #[async_trait]
    impl Paginator for EmptyThenFull {
        type Item = u32;
        async fn fetch_page(
            &mut self,
            _c: &HttpClient,
            cursor: Option<String>,
        ) -> SealStackResult<(Vec<u32>, Option<String>)> {
            match cursor.as_deref() {
                None => Ok((vec![], Some("next".into()))),
                Some("next") => Ok((vec![42], None)),
                Some(other) => panic!("unexpected {other}"),
            }
        }
    }

    /// Returns the same cursor twice — triggers PaginatorCursorLoop.
    struct StuckCursor;

    #[async_trait]
    impl Paginator for StuckCursor {
        type Item = u32;
        async fn fetch_page(
            &mut self,
            _c: &HttpClient,
            _cursor: Option<String>,
        ) -> SealStackResult<(Vec<u32>, Option<String>)> {
            Ok((vec![1], Some("same".into())))
        }
    }

    fn dummy_client() -> Arc<HttpClient> {
        Arc::new(
            HttpClient::new(Arc::new(StaticToken::new("t")), RetryPolicy::default())
                .unwrap(),
        )
    }

    #[tokio::test]
    async fn drives_paginator_to_exhaustion() {
        let items: Vec<_> = paginate(TwoPage { calls: 0 }, dummy_client())
            .collect()
            .await;
        let ok: Vec<u32> = items.into_iter().map(Result::unwrap).collect();
        assert_eq!(ok, vec![1, 2, 3, 4]);
    }

    #[tokio::test]
    async fn continues_through_empty_page() {
        let items: Vec<_> = paginate(EmptyThenFull, dummy_client()).collect().await;
        let ok: Vec<u32> = items.into_iter().map(Result::unwrap).collect();
        assert_eq!(ok, vec![42]);
    }

    #[tokio::test]
    async fn detects_cursor_loop() {
        let mut s = paginate(StuckCursor, dummy_client());
        // First page yields 1 (with cursor "same").
        assert_eq!(s.next().await.unwrap().unwrap(), 1);
        // Second page yields 1 again (cursor "same" repeated → loop).
        assert_eq!(s.next().await.unwrap().unwrap(), 1);
        // Third fetch yields the loop error.
        let err = s.next().await.unwrap().unwrap_err();
        assert!(
            matches!(err, SealStackError::PaginatorCursorLoop { .. }),
            "expected PaginatorCursorLoop, got {err}"
        );
    }
}
```

- [ ] **Step 3: Add `async-stream` dependency**

Modify `crates/sealstack-connector-sdk/Cargo.toml`. In `[dependencies]`:

```toml
async-stream = "0.3"
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p sealstack-connector-sdk --lib paginate::tests`
Expected: all three tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/sealstack-connector-sdk/Cargo.toml crates/sealstack-connector-sdk/src/paginate.rs crates/sealstack-connector-sdk/src/lib.rs
git commit -m "feat(connector-sdk): Paginator trait + paginate() stream adapter"
```

---

### Task 10: `BodyCursorPaginator` reference impl

**Files:**
- Modify: `crates/sealstack-connector-sdk/src/paginate.rs`
- Create: `crates/sealstack-connector-sdk/tests/paginate_body_cursor.rs`

- [ ] **Step 1: Write the failing integration test**

Create `crates/sealstack-connector-sdk/tests/paginate_body_cursor.rs`:

```rust
use std::sync::Arc;

use futures::StreamExt;
use serde::Deserialize;

use sealstack_connector_sdk::auth::StaticToken;
use sealstack_connector_sdk::http::HttpClient;
use sealstack_connector_sdk::paginate::{BodyCursorPaginator, paginate};
use sealstack_connector_sdk::retry::RetryPolicy;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Deserialize, Debug, PartialEq)]
struct Item {
    id: u32,
}

#[tokio::test]
async fn body_cursor_paginator_walks_pages() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/list"))
        .and(wiremock::matchers::query_param_is_missing("cursor"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({ "items": [{ "id": 1 }, { "id": 2 }], "next": "p2" }),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/list"))
        .and(wiremock::matchers::query_param("cursor", "p2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({ "items": [{ "id": 3 }], "next": null }),
        ))
        .mount(&server)
        .await;

    let client = Arc::new(
        HttpClient::new(Arc::new(StaticToken::new("t")), RetryPolicy::default()).unwrap(),
    );
    let url = format!("{}/list", server.uri());

    let pg = BodyCursorPaginator::<Item, _, _, _>::new(
        move |c: &HttpClient, cursor: Option<&str>| {
            let mut rb = c.get(&url);
            if let Some(cur) = cursor {
                rb = rb.query(&[("cursor", cur)]);
            }
            rb
        },
        |v: &serde_json::Value| {
            let arr = v
                .get("items")
                .and_then(|a| a.as_array())
                .ok_or_else(|| sealstack_common::SealStackError::backend("missing items"))?;
            arr.iter()
                .map(|x| {
                    serde_json::from_value::<Item>(x.clone())
                        .map_err(|e| sealstack_common::SealStackError::backend(format!("{e}")))
                })
                .collect()
        },
        |v: &serde_json::Value| {
            v.get("next")
                .and_then(|c| c.as_str())
                .map(str::to_owned)
        },
    );
    let items: Vec<_> = paginate(pg, client).collect().await;
    let ok: Vec<Item> = items.into_iter().map(Result::unwrap).collect();
    assert_eq!(ok, vec![Item { id: 1 }, Item { id: 2 }, Item { id: 3 }]);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p sealstack-connector-sdk --test paginate_body_cursor`
Expected: FAIL — `BodyCursorPaginator` does not exist.

- [ ] **Step 3: Implement `BodyCursorPaginator`**

Append to `crates/sealstack-connector-sdk/src/paginate.rs`:

```rust
use serde::de::DeserializeOwned;

/// Body-cursor paginator: the cursor lives inside the response body.
///
/// Use for Slack (`response_metadata.next_cursor`), Google Drive
/// (`nextPageToken`), Notion (`next_cursor`), Linear (`pageInfo.endCursor`).
pub struct BodyCursorPaginator<T, Req, ExtractItems, ExtractCursor>
where
    T: DeserializeOwned + Send + 'static,
    Req: for<'a> Fn(&'a HttpClient, Option<&'a str>) -> reqwest::RequestBuilder
        + Send
        + 'static,
    ExtractItems: Fn(&serde_json::Value) -> SealStackResult<Vec<T>> + Send + 'static,
    ExtractCursor: Fn(&serde_json::Value) -> Option<String> + Send + 'static,
{
    request: Req,
    extract_items: ExtractItems,
    extract_cursor: ExtractCursor,
    _marker: std::marker::PhantomData<fn() -> T>,
}

impl<T, Req, EI, EC> BodyCursorPaginator<T, Req, EI, EC>
where
    T: DeserializeOwned + Send + 'static,
    Req: for<'a> Fn(&'a HttpClient, Option<&'a str>) -> reqwest::RequestBuilder
        + Send
        + 'static,
    EI: Fn(&serde_json::Value) -> SealStackResult<Vec<T>> + Send + 'static,
    EC: Fn(&serde_json::Value) -> Option<String> + Send + 'static,
{
    /// Build a new paginator.
    ///
    /// - `request`: closure that composes a request given the client and the
    ///   current cursor (or `None` on the first call).
    /// - `extract_items`: extracts the page's item array from the response
    ///   JSON.
    /// - `extract_cursor`: extracts the next-page cursor from the response
    ///   JSON. Return `None` to terminate the stream.
    pub fn new(request: Req, extract_items: EI, extract_cursor: EC) -> Self {
        Self {
            request,
            extract_items,
            extract_cursor,
            _marker: std::marker::PhantomData,
        }
    }
}

#[async_trait]
impl<T, Req, EI, EC> Paginator for BodyCursorPaginator<T, Req, EI, EC>
where
    T: DeserializeOwned + Send + 'static,
    Req: for<'a> Fn(&'a HttpClient, Option<&'a str>) -> reqwest::RequestBuilder
        + Send
        + 'static,
    EI: Fn(&serde_json::Value) -> SealStackResult<Vec<T>> + Send + 'static,
    EC: Fn(&serde_json::Value) -> Option<String> + Send + 'static,
{
    type Item = T;
    async fn fetch_page(
        &mut self,
        client: &HttpClient,
        cursor: Option<String>,
    ) -> SealStackResult<(Vec<T>, Option<String>)> {
        let rb = (self.request)(client, cursor.as_deref());
        let resp = client.send(rb).await?;
        let body: serde_json::Value = resp.json().await?;
        let items = (self.extract_items)(&body)?;
        let next = (self.extract_cursor)(&body);
        Ok((items, next))
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p sealstack-connector-sdk --test paginate_body_cursor`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/sealstack-connector-sdk/src/paginate.rs crates/sealstack-connector-sdk/tests/paginate_body_cursor.rs
git commit -m "feat(connector-sdk): BodyCursorPaginator reference builder"
```

---

### Task 11: `LinkHeaderPaginator` reference impl

**Files:**
- Modify: `crates/sealstack-connector-sdk/src/paginate.rs`
- Create: `crates/sealstack-connector-sdk/tests/paginate_link_header.rs`

- [ ] **Step 1: Write the failing integration test**

Create `crates/sealstack-connector-sdk/tests/paginate_link_header.rs`:

```rust
use std::sync::Arc;

use futures::StreamExt;
use serde::Deserialize;

use sealstack_connector_sdk::auth::StaticToken;
use sealstack_connector_sdk::http::HttpClient;
use sealstack_connector_sdk::paginate::{LinkHeaderPaginator, paginate};
use sealstack_connector_sdk::retry::RetryPolicy;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Deserialize, Debug, PartialEq)]
struct Issue {
    id: u32,
}

#[tokio::test]
async fn link_header_paginator_walks_pages() {
    let server = MockServer::start().await;
    let next_url = format!("{}/issues?page=2", server.uri());

    Mock::given(method("GET"))
        .and(path("/issues"))
        .and(wiremock::matchers::query_param_is_missing("page"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!([{ "id": 1 }, { "id": 2 }]))
                .append_header("Link", format!("<{next_url}>; rel=\"next\"")),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/issues"))
        .and(wiremock::matchers::query_param("page", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([{ "id": 3 }])))
        .mount(&server)
        .await;

    let client = Arc::new(
        HttpClient::new(Arc::new(StaticToken::new("t")), RetryPolicy::default()).unwrap(),
    );
    let initial = format!("{}/issues", server.uri());

    let pg = LinkHeaderPaginator::<Issue, _>::new(
        move |c: &HttpClient, cursor: Option<&str>| match cursor {
            None => c.get(&initial),
            Some(url) => c.get(url),
        },
    );
    let items: Vec<_> = paginate(pg, client).collect().await;
    let ok: Vec<Issue> = items.into_iter().map(Result::unwrap).collect();
    assert_eq!(ok, vec![Issue { id: 1 }, Issue { id: 2 }, Issue { id: 3 }]);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p sealstack-connector-sdk --test paginate_link_header`
Expected: FAIL — `LinkHeaderPaginator` does not exist.

- [ ] **Step 3: Implement `LinkHeaderPaginator`**

Append to `crates/sealstack-connector-sdk/src/paginate.rs`:

```rust
/// Parse a `Link` header and return the URL of the `rel="next"` entry.
///
/// Used by [`LinkHeaderPaginator`]; `pub` so connectors with Link-like
/// custom headers can reuse it.
pub fn next_link(header: &str) -> Option<String> {
    for part in header.split(',') {
        let part = part.trim();
        let mut it = part.split(';');
        let url_bracket = it.next()?.trim();
        let url = url_bracket.strip_prefix('<')?.strip_suffix('>')?;
        let mut is_next = false;
        for attr in it {
            let attr = attr.trim();
            if attr == "rel=\"next\"" || attr == "rel=next" {
                is_next = true;
                break;
            }
        }
        if is_next {
            return Some(url.to_owned());
        }
    }
    None
}

/// Link-header paginator: cursor is the URL from `Link: rel="next"`.
///
/// Use for GitHub REST, GitLab REST, anything following RFC 8288 link headers.
pub struct LinkHeaderPaginator<T, Req>
where
    T: DeserializeOwned + Send + 'static,
    Req: for<'a> Fn(&'a HttpClient, Option<&'a str>) -> reqwest::RequestBuilder
        + Send
        + 'static,
{
    request: Req,
    _marker: std::marker::PhantomData<fn() -> T>,
}

impl<T, Req> LinkHeaderPaginator<T, Req>
where
    T: DeserializeOwned + Send + 'static,
    Req: for<'a> Fn(&'a HttpClient, Option<&'a str>) -> reqwest::RequestBuilder
        + Send
        + 'static,
{
    /// Build a new paginator.
    ///
    /// - `request`: first call receives `cursor = None`; subsequent calls
    ///   receive the previous page's `Link: rel="next"` URL as the cursor.
    pub fn new(request: Req) -> Self {
        Self {
            request,
            _marker: std::marker::PhantomData,
        }
    }
}

#[async_trait]
impl<T, Req> Paginator for LinkHeaderPaginator<T, Req>
where
    T: DeserializeOwned + Send + 'static,
    Req: for<'a> Fn(&'a HttpClient, Option<&'a str>) -> reqwest::RequestBuilder
        + Send
        + 'static,
{
    type Item = T;
    async fn fetch_page(
        &mut self,
        client: &HttpClient,
        cursor: Option<String>,
    ) -> SealStackResult<(Vec<T>, Option<String>)> {
        let rb = (self.request)(client, cursor.as_deref());
        let resp = client.send(rb).await?;
        let next = resp.header("Link").and_then(next_link);
        let items: Vec<T> = resp.json().await?;
        Ok((items, next))
    }
}

#[cfg(test)]
mod link_header_tests {
    use super::next_link;

    #[test]
    fn parses_next_link() {
        let hdr = r#"<https://api.example.com/p?page=2>; rel="next", <https://api.example.com/p?page=9>; rel="last""#;
        assert_eq!(
            next_link(hdr),
            Some("https://api.example.com/p?page=2".to_owned())
        );
    }

    #[test]
    fn no_next_link_returns_none() {
        let hdr = r#"<https://api.example.com/p?page=9>; rel="last""#;
        assert_eq!(next_link(hdr), None);
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p sealstack-connector-sdk`
Expected: all pass, including `paginate_link_header`.

- [ ] **Step 5: Commit**

```bash
git add crates/sealstack-connector-sdk/src/paginate.rs crates/sealstack-connector-sdk/tests/paginate_link_header.rs
git commit -m "feat(connector-sdk): LinkHeaderPaginator reference builder"
```

---

### Task 12: `OffsetPaginator` reference impl

**Files:**
- Modify: `crates/sealstack-connector-sdk/src/paginate.rs`
- Create: `crates/sealstack-connector-sdk/tests/paginate_offset.rs`

- [ ] **Step 1: Write the failing integration test**

Create `crates/sealstack-connector-sdk/tests/paginate_offset.rs`:

```rust
use std::sync::Arc;

use futures::StreamExt;
use serde::Deserialize;

use sealstack_connector_sdk::auth::StaticToken;
use sealstack_connector_sdk::http::HttpClient;
use sealstack_connector_sdk::paginate::{OffsetPaginator, paginate};
use sealstack_connector_sdk::retry::RetryPolicy;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Deserialize, Debug, PartialEq)]
struct Row {
    id: u32,
}

#[tokio::test]
async fn offset_paginator_walks_pages() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rows"))
        .and(wiremock::matchers::query_param("startAt", "0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "startAt": 0, "maxResults": 2, "total": 3,
            "values": [{ "id": 1 }, { "id": 2 }],
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rows"))
        .and(wiremock::matchers::query_param("startAt", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "startAt": 2, "maxResults": 2, "total": 3,
            "values": [{ "id": 3 }],
        })))
        .mount(&server)
        .await;

    let client = Arc::new(
        HttpClient::new(Arc::new(StaticToken::new("t")), RetryPolicy::default()).unwrap(),
    );
    let url = format!("{}/rows", server.uri());

    let pg = OffsetPaginator::<Row, _>::new(
        2,
        move |c: &HttpClient, start: u64, limit: u64| {
            c.get(&url)
                .query(&[("startAt", start.to_string()), ("maxResults", limit.to_string())])
        },
        "values",
    );
    let items: Vec<_> = paginate(pg, client).collect().await;
    let ok: Vec<Row> = items.into_iter().map(Result::unwrap).collect();
    assert_eq!(ok, vec![Row { id: 1 }, Row { id: 2 }, Row { id: 3 }]);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p sealstack-connector-sdk --test paginate_offset`
Expected: FAIL — `OffsetPaginator` does not exist.

- [ ] **Step 3: Implement `OffsetPaginator`**

Append to `crates/sealstack-connector-sdk/src/paginate.rs`:

```rust
/// Offset/limit paginator: numeric `start` + `limit` against a `total`.
///
/// Use for Jira REST API v3 (`startAt`/`maxResults`) and Confluence Cloud
/// REST (`start`/`limit`). The response body must include both the item
/// array (under a configurable JSON key) and a numeric `total` at the top
/// level — the paginator uses `total` to detect exhaustion.
pub struct OffsetPaginator<T, Req>
where
    T: DeserializeOwned + Send + 'static,
    Req: for<'a> Fn(&'a HttpClient, u64, u64) -> reqwest::RequestBuilder + Send + 'static,
{
    limit: u64,
    request: Req,
    items_key: &'static str,
    next_start: u64,
    total: Option<u64>,
    _marker: std::marker::PhantomData<fn() -> T>,
}

impl<T, Req> OffsetPaginator<T, Req>
where
    T: DeserializeOwned + Send + 'static,
    Req: for<'a> Fn(&'a HttpClient, u64, u64) -> reqwest::RequestBuilder + Send + 'static,
{
    /// Build a new paginator with the given page size, request closure, and
    /// response key under which items live.
    pub fn new(limit: u64, request: Req, items_key: &'static str) -> Self {
        Self {
            limit,
            request,
            items_key,
            next_start: 0,
            total: None,
            _marker: std::marker::PhantomData,
        }
    }
}

#[async_trait]
impl<T, Req> Paginator for OffsetPaginator<T, Req>
where
    T: DeserializeOwned + Send + 'static,
    Req: for<'a> Fn(&'a HttpClient, u64, u64) -> reqwest::RequestBuilder + Send + 'static,
{
    type Item = T;
    async fn fetch_page(
        &mut self,
        client: &HttpClient,
        _cursor: Option<String>,
    ) -> SealStackResult<(Vec<T>, Option<String>)> {
        let rb = (self.request)(client, self.next_start, self.limit);
        let resp = client.send(rb).await?;
        let body: serde_json::Value = resp.json().await?;

        if self.total.is_none() {
            self.total = body.get("total").and_then(|v| v.as_u64());
        }
        let items: Vec<T> = body
            .get(self.items_key)
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                SealStackError::backend(format!("missing array at `{}`", self.items_key))
            })?
            .iter()
            .map(|x| {
                serde_json::from_value::<T>(x.clone())
                    .map_err(|e| SealStackError::backend(format!("{e}")))
            })
            .collect::<SealStackResult<Vec<_>>>()?;

        let returned = items.len() as u64;
        self.next_start += returned;

        let done = match self.total {
            Some(total) => self.next_start >= total,
            // No total header — stop when the server returns fewer than limit.
            None => returned < self.limit,
        };

        // The adapter threads cursors by value equality; use a monotonically
        // increasing offset-as-string so the cursor-loop detector works.
        let next = if done {
            None
        } else {
            Some(self.next_start.to_string())
        };
        Ok((items, next))
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p sealstack-connector-sdk`
Expected: all pass, including `paginate_offset`.

- [ ] **Step 5: Commit**

```bash
git add crates/sealstack-connector-sdk/src/paginate.rs crates/sealstack-connector-sdk/tests/paginate_offset.rs
git commit -m "feat(connector-sdk): OffsetPaginator reference builder"
```

---

## Phase 4 — Connector refactors

### Task 13: Refactor `local-files` (verification probe)

The point of this task: verify the SDK's `Connector` trait can be implemented without dragging in HTTP / Credential / Paginator.

**Files:**
- Modify: `connectors/local-files/src/lib.rs`

- [ ] **Step 1: Inspect the current file for accidental coupling to new modules**

Run: `cargo check -p sealstack-connector-local-files`
Expected: clean compile. The connector only uses `Connector`, `Resource`, `ResourceId`, `PermissionPredicate` from the SDK.

- [ ] **Step 2: If needed, adjust imports to reflect the split (no behavior change)**

Current connectors import via `sealstack_connector_sdk::{...}` top-level re-exports. No import changes needed — the re-exports in `lib.rs` preserve the existing paths. This step is a no-op verification.

- [ ] **Step 3: Run the connector's test suite**

Run: `cargo test -p sealstack-connector-local-files`
Expected: all existing tests pass unchanged.

- [ ] **Step 4: Confirm dependency footprint is unchanged**

Run: `cargo tree -p sealstack-connector-local-files --depth 1`
Expected: the connector does not depend on `reqwest`, `secrecy`, or `wiremock` transitively — it only depends on `sealstack-connector-sdk` and its existing deps. If any of those new deps leak in via the trait path, the SDK has accidentally coupled to HTTP and must be fixed before proceeding.

- [ ] **Step 5: Commit (empty commit acceptable; marks the verification)**

If any source changes were needed, commit them. Otherwise:

```bash
git commit --allow-empty -m "chore(local-files): verify trait-only SDK path (no code changes)"
```

---

### Task 14: Refactor `slack` onto `HttpClient`

**Files:**
- Modify: `connectors/slack/src/lib.rs`
- Modify: `connectors/slack/Cargo.toml` (drop `reqwest`, keep SDK dep)

- [ ] **Step 1: Read the current slack connector to map the refactor boundaries**

Run: `cargo tree -p sealstack-connector-slack --depth 1`
Note which crates are direct deps today.

- [ ] **Step 2: Replace direct `reqwest::Client` with `HttpClient`**

Modify `connectors/slack/src/lib.rs`. Replace the `struct SlackConnector { config: ..., client: reqwest::Client }` field and its construction with `http: Arc<HttpClient>`. Concretely:

- Change the struct field `client: reqwest::Client` → `http: Arc<HttpClient>`.
- In the constructor, build `HttpClient` from a `StaticToken`:

```rust
use std::sync::Arc;
use sealstack_connector_sdk::auth::StaticToken;
use sealstack_connector_sdk::http::HttpClient;
use sealstack_connector_sdk::retry::RetryPolicy;

let credential = Arc::new(StaticToken::new(config.token.clone()));
let http = Arc::new(
    HttpClient::new(credential, RetryPolicy::default())?
);
```

- Replace each callsite that did `self.client.get(url).bearer_auth(&self.config.token).send().await` with:

```rust
let rb = self.http.get(url);
let resp = self.http.send(rb).await?;
let body: SlackResponse<...> = resp.json().await?;
```

- Delete the manual 401 → `Unauthorized` mapping — `HttpClient::send` already does it.

- [ ] **Step 3: Handle config-vs-env var precedence explicitly**

In the constructor, ensure both sources are consulted with config winning, and neither → error at construction:

```rust
let token = match (v.get("token").and_then(|x| x.as_str()), std::env::var("SLACK_BOT_TOKEN").ok()) {
    (Some(t), env_present) => {
        if env_present.is_some() {
            tracing::warn!("slack: config `token` set; SLACK_BOT_TOKEN env ignored");
        }
        t.to_owned()
    }
    (None, Some(env)) if !env.is_empty() => env,
    _ => return Err(SealStackError::Config(
        "slack connector requires `token` in config or SLACK_BOT_TOKEN env".into(),
    )),
};
```

- [ ] **Step 4: Drop `reqwest` from the connector's Cargo.toml if no longer used**

Remove the `reqwest = { workspace = true }` line from `connectors/slack/Cargo.toml` if it is not referenced directly after the refactor.

- [ ] **Step 5: Update existing unit tests to use `HttpClient`**

Tests that previously built a direct `reqwest::Client` should now build an `HttpClient` (same patterns, just at the SDK level). The two `reqwest::header::HeaderValue::from_static(...)` test helpers for `next_link` parsing are not slack-specific — they can be deleted here (they are GitHub-specific and that copy lives in `connectors/github`).

- [ ] **Step 6: Run the slack connector tests**

Run: `cargo test -p sealstack-connector-slack`
Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add connectors/slack/
git commit -m "refactor(slack): adopt HttpClient + StaticToken; drop direct reqwest usage"
```

---

### Task 15: Refactor `slack` pagination to `BodyCursorPaginator`

**Files:**
- Modify: `connectors/slack/src/lib.rs`

- [ ] **Step 1: Identify the two pagination loops**

Current code has hand-rolled cursor loops for `list_channels` (conversations.list) and `list_messages` (conversations.history). Both walk `response_metadata.next_cursor`.

- [ ] **Step 2: Replace `list_channels` with `BodyCursorPaginator`**

```rust
use sealstack_connector_sdk::paginate::{BodyCursorPaginator, paginate};

let base_url = format!("{SLACK_API}/conversations.list");
let pg = BodyCursorPaginator::<Channel, _, _, _>::new(
    move |c: &HttpClient, cursor: Option<&str>| {
        let mut rb = c
            .get(&base_url)
            .query(&[
                ("limit", "1000"),
                ("exclude_archived", "true"),
                ("types", "public_channel,private_channel"),
            ]);
        if let Some(cur) = cursor {
            rb = rb.query(&[("cursor", cur)]);
        }
        rb
    },
    |v: &serde_json::Value| {
        let arr = v
            .get("channels")
            .and_then(|a| a.as_array())
            .ok_or_else(|| SealStackError::backend("missing channels"))?;
        arr.iter()
            .map(|x| serde_json::from_value::<Channel>(x.clone()).map_err(SealStackError::backend))
            .collect()
    },
    |v: &serde_json::Value| {
        v.get("response_metadata")
            .and_then(|m| m.get("next_cursor"))
            .and_then(|c| c.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    },
);
let stream = paginate(pg, self.http.clone());
```

Collect the stream into the existing `Vec<Channel>` (or adapt the return type to stream if callers can take it).

- [ ] **Step 3: Replace `list_messages` similarly**

Same shape, different path (`conversations.history`), different items key (`messages`).

- [ ] **Step 4: Run the slack connector tests**

Run: `cargo test -p sealstack-connector-slack`
Expected: all pass. Line-count estimate from the spec (~240) is a rough target; a reasonable delta from 376 → 260 or so is acceptable.

- [ ] **Step 5: Commit**

```bash
git add connectors/slack/src/lib.rs
git commit -m "refactor(slack): adopt BodyCursorPaginator for channels + messages"
```

---

### Task 16: Refactor `github` onto `HttpClient` + UA suffix

**Files:**
- Modify: `connectors/github/src/lib.rs`
- Modify: `connectors/github/Cargo.toml`

- [ ] **Step 1: Replace `reqwest::Client` with `HttpClient` carrying a UA suffix**

Parallel to Task 14, but with a UA suffix. In the constructor:

```rust
let credential = Arc::new(StaticToken::new(config.token.clone()));
let http = Arc::new(
    HttpClient::new(credential, RetryPolicy::default())?
        .with_user_agent_suffix(format!("github-connector/{}", env!("CARGO_PKG_VERSION"))),
);
```

- [ ] **Step 2: Handle config/env precedence for `token` / `GITHUB_TOKEN`**

Same pattern as Task 14, with `GITHUB_TOKEN` env var name.

- [ ] **Step 3: Drop the inline `reqwest::Client::builder()` code and the manual 401 mapping**

These are now handled by `HttpClient`.

- [ ] **Step 4: Run the github connector tests**

Run: `cargo test -p sealstack-connector-github`
Expected: pass; the existing `next_link` tests still pass (they live on the github crate — `next_link` is the same algorithm but the github copy stays put until Task 17, which moves or deletes it).

- [ ] **Step 5: Commit**

```bash
git add connectors/github/
git commit -m "refactor(github): adopt HttpClient + StaticToken + UA suffix"
```

---

### Task 17: Refactor `github` pagination to `LinkHeaderPaginator`

**Files:**
- Modify: `connectors/github/src/lib.rs`

- [ ] **Step 1: Replace both Link-header loops with `LinkHeaderPaginator`**

```rust
use sealstack_connector_sdk::paginate::{LinkHeaderPaginator, paginate};

let initial = format!("{GITHUB_API}/user/repos?per_page=100");
let pg = LinkHeaderPaginator::<Repo, _>::new(
    move |c: &HttpClient, cursor: Option<&str>| match cursor {
        None => c.get(&initial).header("Accept", "application/vnd.github+json"),
        Some(url) => c.get(url).header("Accept", "application/vnd.github+json"),
    },
);
let stream = paginate(pg, self.http.clone());
```

Repeat for issues (`list_issues`).

- [ ] **Step 2: Delete the local `next_link` function**

The SDK's `sealstack_connector_sdk::paginate::next_link` replaces it; the github copy is now redundant. Delete the function and its tests (the SDK's version is already covered by tests in Task 11).

- [ ] **Step 3: Run the github connector tests**

Run: `cargo test -p sealstack-connector-github`
Expected: pass.

- [ ] **Step 4: Commit**

```bash
git add connectors/github/src/lib.rs
git commit -m "refactor(github): adopt LinkHeaderPaginator for repos + issues"
```

---

### Task 18: SDK — `HttpStatus` error variant + capture on non-retryable 4xx

Lays the SDK-side foundation the GitHub 403 shim needs. Surfaces headers and body to connectors so they can apply vendor-specific logic without re-issuing requests.

**Files:**
- Modify: `crates/sealstack-common/src/lib.rs` (add `HttpStatus` variant)
- Modify: `crates/sealstack-connector-sdk/src/http.rs` (capture in 4xx arm)
- Modify: `crates/sealstack-connector-sdk/tests/http_retry.rs` (update 404 assertion)

- [ ] **Step 1: Add the `HttpStatus` variant to `SealStackError`**

Modify `crates/sealstack-common/src/lib.rs`. Inside the `SealStackError` enum, after the variants added in Task 3:

```rust
    /// HTTP request produced a non-retryable status with headers + body the
    /// caller may need to inspect (e.g. GitHub's 403 discrimination).
    #[error("HTTP {status}")]
    HttpStatus {
        /// Status code.
        status: u16,
        /// Headers from the response, copied as `(name, value)` pairs.
        headers: Vec<(String, String)>,
        /// Body as UTF-8 text; empty if the body was non-UTF-8 or read
        /// failed. The body is already size-capped per the `HttpClient`'s
        /// configured cap.
        body: String,
    },
```

- [ ] **Step 2: Write the failing test**

Modify `crates/sealstack-connector-sdk/tests/http_retry.rs`. Update the existing `fourhundred_not_retried` test assertion:

```rust
#[tokio::test]
async fn fourhundred_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/bad"))
        .respond_with(
            ResponseTemplate::new(404)
                .append_header("X-Trace-Id", "abc123")
                .set_body_string(r#"{"error":"not found"}"#),
        )
        .expect(1) // exactly one call — no retries.
        .mount(&server)
        .await;

    let c = client(&server).await;
    let rb = c.get(format!("{}/bad", server.uri()));
    let err = c.send(rb).await.unwrap_err();
    match err {
        SealStackError::HttpStatus { status, headers, body } => {
            assert_eq!(status, 404);
            assert!(
                headers.iter().any(|(k, v)| k.eq_ignore_ascii_case("X-Trace-Id") && v == "abc123"),
                "missing X-Trace-Id header: {headers:?}",
            );
            assert!(body.contains("not found"), "body: {body}");
        }
        other => panic!("expected HttpStatus, got {other}"),
    }
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test -p sealstack-connector-sdk --test http_retry -- fourhundred_not_retried`
Expected: FAIL — current 4xx arm returns `Backend`, not `HttpStatus`.

- [ ] **Step 4: Update `HttpClient::send` to return `HttpStatus` for non-retryable 4xx**

Modify `crates/sealstack-connector-sdk/src/http.rs`. Replace the non-retryable-4xx branch inside `send`:

```rust
                    if status.is_client_error()
                        && status != reqwest::StatusCode::REQUEST_TIMEOUT
                        && status != reqwest::StatusCode::TOO_MANY_REQUESTS
                    {
                        let code = status.as_u16();
                        let headers: Vec<(String, String)> = resp
                            .headers()
                            .iter()
                            .filter_map(|(k, v)| {
                                v.to_str().ok().map(|s| (k.as_str().to_owned(), s.to_owned()))
                            })
                            .collect();
                        // Stream the body under the cap — same code path as
                        // the success case, so a hostile 4xx body cannot
                        // exhaust memory.
                        let cap = self.body_cap_bytes;
                        let body = read_body_capped(resp, cap).await.unwrap_or_default();
                        return Err(SealStackError::HttpStatus {
                            status: code,
                            headers,
                            body,
                        });
                    }
```

Add a new private helper at module scope in `http.rs`:

```rust
/// Read a `reqwest::Response` body into a `String` under the given cap.
///
/// Used by both the successful-body path (via `HttpResponse::bytes`) and
/// the 4xx-capture path so the cap is uniform.
async fn read_body_capped(
    resp: reqwest::Response,
    cap: usize,
) -> SealStackResult<String> {
    use futures::StreamExt;
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk
            .map_err(|e| SealStackError::backend(format!("body stream: {e}")))?;
        if buf.len() + chunk.len() > cap {
            return Err(SealStackError::BodyTooLarge { cap_bytes: cap });
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}
```

- [ ] **Step 5: Run all SDK tests**

Run: `cargo test -p sealstack-common -p sealstack-connector-sdk`
Expected: all pass — the updated `fourhundred_not_retried` test now asserts `HttpStatus` and other tests are unaffected.

- [ ] **Step 6: Commit**

```bash
git add crates/sealstack-common/src/lib.rs crates/sealstack-connector-sdk/src/http.rs crates/sealstack-connector-sdk/tests/http_retry.rs
git commit -m "feat(connector-sdk): HttpStatus error variant with captured headers + body"
```

---

### Task 19: GitHub 403 shim (three-case discrimination)

Per the spec §8. Consumes the `HttpStatus` variant added in Task 18.

**Files:**
- Create: `connectors/github/src/retry_shim.rs`
- Modify: `connectors/github/src/lib.rs` (wire the shim in)
- Create: `connectors/github/tests/retry_shim.rs`

- [ ] **Step 1: Write failing tests for the three 403 cases**

Create `connectors/github/tests/retry_shim.rs`:

```rust
use std::time::Duration;

use sealstack_connector_github::retry_shim::{classify_github_403, Github403Action};

fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
}

#[test]
fn primary_rate_limit_waits_until_reset() {
    let reset = (time::OffsetDateTime::now_utc().unix_timestamp() + 60).to_string();
    let headers = pairs(&[
        ("X-RateLimit-Remaining", "0"),
        ("X-RateLimit-Reset", reset.as_str()),
    ]);
    match classify_github_403(&headers, "") {
        Github403Action::WaitThenRetry(d) => {
            assert!(d > Duration::from_secs(55) && d < Duration::from_secs(65), "{d:?}");
        }
        other => panic!("expected WaitThenRetry, got {other:?}"),
    }
}

#[test]
fn secondary_rate_limit_retry_after_honored() {
    let headers = pairs(&[("Retry-After", "15")]);
    match classify_github_403(&headers, "") {
        Github403Action::WaitThenRetry(d) => {
            assert_eq!(d, Duration::from_secs(15));
        }
        other => panic!("expected WaitThenRetry, got {other:?}"),
    }
}

#[test]
fn secondary_rate_limit_body_marker_uses_backoff() {
    let body = r#"{"message":"You have exceeded a secondary rate limit."}"#;
    match classify_github_403(&[], body) {
        Github403Action::BackoffThenRetry => {}
        other => panic!("expected BackoffThenRetry, got {other:?}"),
    }
}

#[test]
fn plain_403_is_permission_denied() {
    match classify_github_403(&[], r#"{"message":"Resource not accessible"}"#) {
        Github403Action::PermissionDenied => {}
        other => panic!("expected PermissionDenied, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p sealstack-connector-github --test retry_shim`
Expected: FAIL — `retry_shim` module does not exist.

- [ ] **Step 3: Implement the classifier**

Create `connectors/github/src/retry_shim.rs`:

```rust
//! GitHub-specific 403 discrimination.
//!
//! GitHub's REST API emits three distinct 403 patterns that require
//! different client-side handling. See the design spec at
//! `docs/superpowers/specs/2026-04-24-connector-sdk-hardening-design.md` §8.

use std::time::Duration;

/// Classification of a GitHub 403 response.
#[derive(Debug)]
pub enum Github403Action {
    /// Case (a) primary rate limit or (b1) explicit `Retry-After`.
    ///
    /// The duration is how long to wait before a single retry.
    WaitThenRetry(Duration),
    /// Case (b2) secondary rate limit without explicit `Retry-After`.
    ///
    /// Caller applies its own exponential backoff (typically ~500ms to 1s).
    BackoffThenRetry,
    /// Case (c) real permission failure — do not retry.
    PermissionDenied,
}

/// Classify a GitHub 403 response.
///
/// `headers` are `(name, value)` pairs as surfaced by
/// `SealStackError::HttpStatus`. `body` is the response body as UTF-8 text
/// (already size-capped by `HttpClient`).
#[must_use]
pub fn classify_github_403(
    headers: &[(String, String)],
    body: &str,
) -> Github403Action {
    // Case (a): primary rate limit.
    if header_eq(headers, "X-RateLimit-Remaining", "0") {
        if let Some(reset_unix) = header_value(headers, "X-RateLimit-Reset")
            .and_then(|s| s.parse::<i64>().ok())
        {
            let now = time::OffsetDateTime::now_utc().unix_timestamp();
            let delta = reset_unix.saturating_sub(now).max(0) as u64;
            // One extra second of slack for clock skew between client and server.
            return Github403Action::WaitThenRetry(Duration::from_secs(delta + 1));
        }
    }
    // Case (b1): explicit Retry-After.
    if let Some(secs) = header_value(headers, "Retry-After").and_then(|s| s.parse::<u64>().ok()) {
        return Github403Action::WaitThenRetry(Duration::from_secs(secs));
    }
    // Case (b2): body marker for secondary rate limit.
    if body.to_ascii_lowercase().contains("secondary rate limit") {
        return Github403Action::BackoffThenRetry;
    }
    // Case (c): everything else is permission-denied.
    Github403Action::PermissionDenied
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn header_eq(headers: &[(String, String)], name: &str, value: &str) -> bool {
    header_value(headers, name) == Some(value)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p sealstack-connector-github --test retry_shim`
Expected: all four tests pass.

- [ ] **Step 5: Wire the shim into github's send path**

Modify `connectors/github/src/lib.rs`. Declare the module at the top:

```rust
pub mod retry_shim;
```

Add a helper that wraps `HttpClient::send` with 403 retry:

```rust
use sealstack_common::{SealStackError, SealStackResult};
use sealstack_connector_sdk::http::{HttpClient, HttpResponse};

/// Send a request under GitHub's 403 discrimination rules.
///
/// `make_request` is called afresh for each attempt so the request body
/// (if any) is rebuilt and the retry path does not depend on request-builder
/// clonability.
async fn send_with_gh_shim(
    http: &HttpClient,
    make_request: impl Fn() -> reqwest::RequestBuilder,
) -> SealStackResult<HttpResponse> {
    // At most: initial + one shim-retry.
    for attempt in 0..2 {
        match http.send(make_request()).await {
            Ok(resp) => return Ok(resp),
            Err(SealStackError::HttpStatus { status: 403, headers, body }) => {
                match retry_shim::classify_github_403(&headers, &body) {
                    retry_shim::Github403Action::WaitThenRetry(d) if attempt == 0 => {
                        tracing::warn!(?d, "github: 403 rate-limit, waiting before retry");
                        tokio::time::sleep(d).await;
                        continue;
                    }
                    retry_shim::Github403Action::BackoffThenRetry if attempt == 0 => {
                        tracing::warn!(
                            "github: 403 secondary rate-limit, backing off before retry"
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        continue;
                    }
                    retry_shim::Github403Action::PermissionDenied => {
                        return Err(SealStackError::Backend(
                            "github 403: permission denied".into(),
                        ));
                    }
                    _ => {
                        return Err(SealStackError::Backend(
                            "github 403: rate-limit retry exhausted".into(),
                        ));
                    }
                }
            }
            Err(e) => return Err(e),
        }
    }
    Err(SealStackError::Backend(
        "github: 403 retry loop terminated unexpectedly".into(),
    ))
}
```

Route each call-site that previously did `self.http.send(rb).await?` through `send_with_gh_shim(&self.http, || <build rb>)`.

- [ ] **Step 6: Run all github tests**

Run: `cargo test -p sealstack-connector-github`
Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add connectors/github/
git commit -m "feat(github): retry_shim for 403 three-case discrimination"
```

---

## Phase 5 — Cleanup

### Task 20: CHANGELOG entry + final verification

Tasks 1–19 kept the `Connector`, `Resource`, `ResourceId`, `PermissionPredicate`, and `ChangeEvent` definitions in `lib.rs` throughout (only `change_streams` moved, and the other new modules are additions), so there are no transitional re-exports to drop. This task is purely the changelog + full-workspace verification.

**Files:**
- Create: `changelog/snippets/connector-sdk-hardening.md` (if the repo uses towncrier-style snippets) **OR** update the top-level `CHANGELOG.md`.

- [ ] **Step 1: Determine the changelog convention**

Run: `ls changelog 2>/dev/null || ls CHANGELOG.md`

If `changelog/snippets/` exists, add a snippet there. Otherwise prepend to `CHANGELOG.md`.

- [ ] **Step 2: Write the changelog entry**

Content:

```markdown
## connector-sdk hardening (2026-04-24)

- **`sealstack-connector-sdk`** split from a flat `lib.rs` into focused modules:
  `auth` (`Credential` trait + `StaticToken`), `http` (`HttpClient` with
  reactive retry middleware, streaming body-size cap), `retry` (policy +
  `Retry-After` parser), `paginate` (`Paginator` trait plus three reference
  builders: `BodyCursorPaginator`, `LinkHeaderPaginator`, `OffsetPaginator`),
  and `change_streams` helpers.
- **`sealstack-common`:** new error variants `RetryExhausted`, `BodyTooLarge`,
  `PaginatorCursorLoop`, `HttpStatus`.
- **connectors refactored** onto the SDK: `local-files` (verification probe),
  `slack` (uses `HttpClient` + `BodyCursorPaginator`), `github` (uses
  `HttpClient` + `LinkHeaderPaginator` + UA suffix + `retry_shim` for
  three-case 403 discrimination).
- **Scope excludes:** OAuth 2.0 authorization code flow (lands with the
  Google Drive connector), proactive rate limiting, streaming upload, typed
  `PermissionPredicate`. See the design spec for deferred items.
```

- [ ] **Step 3: Verify the full workspace builds and tests pass**

Run: `cargo test --workspace`
Expected: all pass.

- [ ] **Step 4: Commit**

```bash
git add changelog/ CHANGELOG.md 2>/dev/null || true
git add -u
git commit -m "docs: changelog entry for connector-sdk hardening"
```

---

## Final verification

- [ ] Run the full workspace build and test: `cargo test --workspace`.
- [ ] Confirm `cargo clippy --workspace --all-targets` passes with no new warnings.
- [ ] Confirm `connectors/local-files` does not transitively depend on `reqwest`, `secrecy`, or `wiremock`: `cargo tree -p sealstack-connector-local-files -e normal | grep -E "reqwest|secrecy|wiremock" || echo OK`. Expected output: `OK`.
- [ ] Confirm the spec's §12 "out-of-scope" items are not accidentally implemented: no Drive connector code, no OAuth impl, no token bucket, no streaming upload.
