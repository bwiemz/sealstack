# Connector SDK Hardening — Design Spec

**Date:** 2026-04-24
**Status:** Approved for implementation
**Author:** collaborative brainstorm, captured from session on 2026-04-24

## 1. Goals

Harden `sealstack-connector-sdk` so that every remaining Phase-1 connector
(Google Drive, Notion, Linear, Confluence, Jira, GitLab, Dropbox, Teams, Gmail,
Outlook) can be built on shared primitives instead of re-rolling HTTP,
authentication, retry, and pagination logic per crate. Prove the shape by
refactoring the three existing connectors (`local-files`, `slack`, `github`)
onto the new SDK within this slice.

## 2. Non-goals

- **No Google Drive connector in this slice.** Drive is the next slice and
  will exercise the OAuth 2.0 credential impl. Designing SDK and Drive together
  would risk shaping the SDK to Drive's quirks rather than the general case.
- **No OAuth 2.0 implementation in this slice.** The `Credential` trait is
  added now with `StaticToken` as the only impl. OAuth lands in the Drive
  slice as another `Credential` impl.
- **No proactive token-bucket rate limiting.** Reactive retry on 429/5xx is
  sufficient for single-replica v0.1. Token buckets are YAGNI until observed
  saturation.
- **No changes to `PermissionPredicate`.** The three refactored connectors
  already produce consistent strings. Guessing at rich ACL shapes (Drive's
  "anyone with link", domain-shared) without a real consumer risks
  wrong-abstraction. Revisit when Drive lands.
- **No streaming upload support in `HttpClient` v1.** Only buffered bodies
  (`.get()`, `.post().json()`, `.post().form()`) so retries can clone the
  request. Streaming upload is a v2 concern.

## 3. Scope

One slice, six sequenced steps, landing in this order:

| # | Step                                          | Blast radius                  |
|---|-----------------------------------------------|-------------------------------|
| 1 | Add `Credential` trait + `StaticToken` impl   | SDK only                      |
| 2 | Add `HttpClient` + retry middleware           | SDK only                      |
| 3 | Add `Paginator` trait + three builders        | SDK only                      |
| 4 | Refactor `local-files`                        | One connector                 |
| 5 | Refactor `slack`                              | One connector                 |
| 6 | Refactor `github` (+ 403 shim)                | One connector                 |

Steps 1–3 preserve the existing `sealstack_connector_sdk::{Connector, Resource,
ChangeEvent, ResourceId, PermissionPredicate, ...}` public paths via
re-exports in `lib.rs`. Connectors compile without modification until step 6
removes the re-exports.

## 4. Module layout

Current state: one flat `src/lib.rs` (290 lines). New layout:

```text
crates/sealstack-connector-sdk/src/
├── lib.rs            — re-exports + Connector trait + Resource/ResourceId/ChangeEvent
├── auth.rs           — Credential trait, StaticToken impl
├── http.rs           — HttpClient (retry baked in)
├── retry.rs          — RetryPolicy type only (integration lives in http.rs)
├── paginate.rs       — Paginator trait + three reference impls + stream adapter
└── change_streams.rs — existing helpers, moved out of lib.rs
```

**Module sizing discipline:** target focused-concept-per-file; ≤200 lines is a
heuristic signal, not a hard rule. `retry.rs` (~250) and `paginate.rs` (~400)
are expected to exceed it. If `paginate.rs` grows past three reference impls,
promoting it to `paginate/` with `cursor.rs`, `offset.rs`, `link_header.rs` is
a future evolution, not a pre-emptive split.

**Retry ownership:** retry is baked into `HttpClient`. `HttpClient::new(cred,
policy)` applies the policy to every request; there is no non-retrying request
path in v1. `retry.rs` exports only the `RetryPolicy` type; `http.rs` owns the
integration. If a v2 caller needs retry-free requests, that is a separate API
extension with its own justification.

## 5. `Credential` trait

```rust
#[async_trait]
pub trait Credential: Send + Sync + 'static {
    /// Returns the full value to insert into the Authorization header,
    /// including scheme prefix. v1 implementations return "Bearer <token>";
    /// v2+ may return "Basic <...>" or other schemes without trait changes.
    ///
    /// Per-request allocation is intentional: OAuth's lock-guarded refresh
    /// path requires owned values to cross `.await` safely. The ~50-byte
    /// clone is dominated by HTTP costs.
    ///
    /// Caching implementations must use async-aware synchronization
    /// (`tokio::sync::Mutex`, `arc-swap`, etc.) to avoid holding locks
    /// across `.await` points.
    async fn authorization_header(&self) -> SealStackResult<String>;

    /// Invalidate any cached credential material. Called by HttpClient
    /// before retrying a 401. Default no-op for credentials that cannot
    /// refresh (e.g. StaticToken).
    async fn invalidate(&self) {}
}
```

### `StaticToken`

```rust
pub struct StaticToken(secrecy::SecretString);

impl StaticToken {
    pub fn new(token: impl Into<String>) -> Self { ... }
    pub fn from_env(var: &str) -> SealStackResult<Self> { ... }
}

impl std::fmt::Debug for StaticToken { /* redacts */ }

#[async_trait]
impl Credential for StaticToken {
    async fn authorization_header(&self) -> SealStackResult<String> {
        Ok(format!("Bearer {}", self.0.expose_secret()))
    }
    // invalidate() default no-op preserves "same token → same 401 → Unauthorized"
}
```

- Use `secrecy::SecretString` for zeroize-on-drop and redacted `Debug`. Adds
  one small dependency; worth it for a credential type.
- `from_env` distinguishes two failure modes via distinct messages on
  `SealStackError::Config`: `"env var <NAME> not set"` vs `"env var <NAME>
  is empty"`. Typed variants on `SealStackError` are a separate follow-up if
  a consumer needs to match programmatically.

## 6. `HttpClient` + retry middleware

```rust
pub struct HttpClient {
    inner: reqwest::Client,
    credential: Arc<dyn Credential>,
    retry: RetryPolicy,
    body_cap_bytes: usize,   // default 50 MB, hard max 500 MB
    user_agent: String,      // "sealstack-connector-sdk/<ver> (rust) [<suffix>]"
}

#[derive(Clone, Debug)]
pub struct RetryPolicy {
    pub max_attempts: u32,     // default 5 (1 initial + 4 retries)
    pub base_delay: Duration,  // default 500ms
    pub max_delay: Duration,   // default 30s
}
```

### Request path

1. Acquire `authorization_header()` from credential (await).
2. Inject into `Authorization` header; apply `User-Agent`.
3. Send.
4. Apply retry policy.

### Retry policy

Attempt counter starts at 0 for the first retry.

| Response                            | Action                                                         |
|-------------------------------------|----------------------------------------------------------------|
| 2xx                                 | Return.                                                        |
| 401                                 | Call `credential.invalidate().await`, retry **at most once per `send()`**. Second 401 (whether immediate or after intervening retries) → `Unauthorized`. `tracing::warn!` on invalidation. |
| 403                                 | Return `Backend`. No retry. (GitHub connector's shim wraps this — see §9.) |
| 408, 429                            | Retry. If `Retry-After` present, use `min(max_delay, retry_after + rand(0..1000ms))`. Else use exponential. |
| 5xx                                 | Retry. `delay = min(max_delay, base_delay * 2^attempt)`, then `sleep(rand(0..delay))` (full jitter). |
| Network (connect/timeout/reset)     | Retry with same exponential backoff as 5xx.                    |
| Other 4xx                           | Return `Backend` with status. No retry.                        |
| Attempts exhausted                  | Return `SealStackError::RetryExhausted`.                       |

**401 / retry-budget interaction:** a single `send()` may do one 401-triggered
invalidation-retry *in addition to* up to `max_attempts` general retries. The
401 retry does not consume a budget slot, but a 401 after invalidation is
final (no second invalidate). This keeps OAuth's stale-token case from being
silently masked by server-side instability, while still preventing infinite
401 loops.

**Worked example (defaults):** retry delays 500 ms, 1 s, 2 s, 4 s. Worst-case
cumulative before final error: **~7.5 s** (no jitter; less with jitter).

**`Retry-After` parsing:** integer-seconds only in v1 — the form used by
GitHub, Slack, Stripe, and the vast majority of servers. HTTP-date form
(`"Wed, 21 Oct 2099 07:28:00 GMT"`) returns `None` and falls through to
exponential backoff; this is acceptable because servers that send HTTP-date
typically send integer seconds in sibling headers, and exponential backoff
is a safe fallback. Negative or unparseable values also fall through. Small
random jitter (0..1000ms) is added to prevent synchronized retries across
many clients. HTTP-date support is a deliberate follow-up if observed need
warrants it.

### Error shape

```rust
SealStackError::RetryExhausted {
    attempts: u32,
    total_duration: Duration,
    last_error: Box<SealStackError>,
}

SealStackError::BodyTooLarge { cap_bytes: usize }
```

### Body-size cap

Enforced **during streaming**, not buffer-then-check. Uses
`response.bytes_stream()` with accumulated-size tracking; abort on overrun.
Default cap: **50 MB**. Configurable via `HttpClient::with_body_cap(usize)` up
to hard maximum **500 MB** — exceeding the max is a configuration error at
`HttpClient::new`. Misconfiguration cannot disable DoS protection entirely.

### User-Agent

Exact format: `sealstack-connector-sdk/<CARGO_PKG_VERSION> (rust)`, with
optional suffix via `HttpClient::with_user_agent_suffix("github-connector/0.1.0")`
producing `sealstack-connector-sdk/1.0.0 (rust) github-connector/0.1.0`. The
`(rust)` comment is conventional; satisfies GitHub's "identify your client"
requirement out of the box.

### Metrics / tracing

One `tracing::span!("http.request", method, url)` per request. No
OpenTelemetry integration in v1; downstream consumers can add it via
`tracing-opentelemetry` without changes to the SDK.

### Tests

`wiremock`-backed (port-binding requirement noted in test-file docstring;
`mockito` is the documented fallback if CI restricts binding). Covers:

- 2xx happy path.
- 401 → invalidate called, retry once, second 401 → `Unauthorized`.
- 429 with `Retry-After: 1` → wait + retry + success.
- 500 twice, then 200 → retry + success.
- 500 five times → `RetryExhausted { attempts: 5, .. }`.
- 403 → `Backend` with status, no retry.
- `Retry-After` parsing: integer-seconds happy path, garbage, negative, and HTTP-date (which falls through to exponential backoff per v1 scope) — unit tests.
- Body cap: 51 MB response with 50 MB cap → `BodyTooLarge`.
- User-Agent emission + suffix composition.

## 7. `Paginator` + reference impls

`paginate.rs` documentation **leads with the three builders**; the trait is the
extension point. 95% of connectors should never implement `Paginator` directly.

### Trait

```rust
#[async_trait]
pub trait Paginator: Send + 'static {
    type Item: Send + 'static;

    /// Fetch one page, given the cursor from the previous page (None on
    /// first call). Returns the page's items and the next cursor (None when
    /// the stream is exhausted).
    ///
    /// Empty pages with a valid next cursor are expected behavior;
    /// implementations must not short-circuit on `items.is_empty()` —
    /// continue until `next_cursor == None`.
    ///
    /// Once this method returns `Err`, the paginator is considered
    /// poisoned. The stream adapter will not call `fetch_page` again after
    /// an error. Implementations do not need to handle re-entry after
    /// failure.
    async fn fetch_page(
        &mut self,
        client: &HttpClient,
        cursor: Option<String>,
    ) -> SealStackResult<(Vec<Self::Item>, Option<String>)>;
}

pub fn paginate<P: Paginator>(
    paginator: P,
    client: Arc<HttpClient>,
) -> Pin<Box<dyn Stream<Item = SealStackResult<P::Item>> + Send>> { ... }
```

### Adapter behavior

- Drives `fetch_page` to exhaustion.
- Yields items one at a time (flattened across pages).
- **Cursor-non-advancement guard:** tracks the previous cursor. If `fetch_page`
  returns the same cursor twice consecutively, errors with
  `SealStackError::PaginatorCursorLoop { cursor: String }`. Contract:
  paginators must return either a new cursor or `None` — same cursor twice is
  a bug, surfaced immediately instead of hanging.
- On first `Err`, yields the error and terminates. Does not call `fetch_page`
  again.

### Paginator ownership

`&mut self` + adapter-owns-by-value. **Do not** add a `Clone` bound to allow
value semantics — paginators may hold non-cloneable or expensive-to-clone state
(TCP sessions, dedupe sets, file handles). This decision is load-bearing; a
module-level comment in `paginate.rs` guards against regression.

### Reference implementations

All three are generic over `T: DeserializeOwned + Send + 'static`. Commits the
builders to JSON/serde_json; non-JSON APIs implement `Paginator` directly.

```rust
pub struct BodyCursorPaginator<T, ReqBuilder, ExtractItems, ExtractCursor>
where
    T: DeserializeOwned + Send + 'static,
    ReqBuilder:     Fn(&HttpClient, Option<&str>) -> reqwest::RequestBuilder + Send + 'static,
    ExtractItems:   Fn(&serde_json::Value) -> SealStackResult<Vec<T>> + Send + 'static,
    ExtractCursor:  Fn(&serde_json::Value) -> Option<String> + Send + 'static,
{ ... }

pub struct LinkHeaderPaginator<T, ReqBuilder>
where
    T: DeserializeOwned + Send + 'static,
    ReqBuilder: Fn(&HttpClient, Option<&str>) -> reqwest::RequestBuilder + Send + 'static,
{ ... }

pub struct OffsetPaginator<T, ReqBuilder>
where
    T: DeserializeOwned + Send + 'static,
    ReqBuilder: Fn(&HttpClient, u64 /* start */, u64 /* limit */) -> reqwest::RequestBuilder + Send + 'static,
{ ... }
```

- `BodyCursorPaginator` — for Slack (`response_metadata.next_cursor`), Drive
  (`nextPageToken`), Notion, Linear, etc.
- `LinkHeaderPaginator` — for GitHub, GitLab (Link: rel=next).
- `OffsetPaginator` — for Jira REST API v3 (`startAt`/`maxResults` with
  `total`) and Confluence Cloud REST (`start`/`limit` with `_links.next`).
  Forward-looking but shape validated against those two APIs' docs.

### Paginator tests

- Trait tested via in-memory mock impl yielding 3 pages then `None`.
- Each reference impl tested with `wiremock` exercising real cursor-threading.
- Stream adapter tests:
  - Normal completion (multi-page → exhausted).
  - Single-page response.
  - Empty first page → adapter returns immediately.
  - Empty page with valid next cursor → adapter continues to next page.
  - Same cursor twice consecutively → `PaginatorCursorLoop`.
  - Error mid-stream → error yielded, paginator not re-entered.

## 8. GitHub 403 shim (step 6)

GitHub emits three distinct 403 patterns with different correct handling:

| Case | Signal                                                             | Handling                                                    |
|------|--------------------------------------------------------------------|-------------------------------------------------------------|
| A    | `X-RateLimit-Remaining: 0` + `X-RateLimit-Reset: <unix-ts>`        | Primary rate limit — wait until reset, retry.              |
| B1   | `Retry-After: <seconds>`                                           | Secondary rate limit — honor (with jitter), retry.          |
| B2   | Body contains `"secondary rate limit"`                             | Secondary rate limit — exponential backoff, retry.          |
| C    | None of the above                                                  | Permission denied — return `Backend`, no retry.             |

Conflating cases would ship bugs:

- A ↔ B: primary's until-reset wait can be minutes; applying exponential
  backoff instead wastes quota.
- Either ↔ C: retrying permission failures wastes time and can trigger
  additional secondary limits on the hostile endpoint.

Shim lives in `connectors/github/src/retry_shim.rs`. Not generic middleware —
GitHub-specific logic. Estimated ~60 lines (three-case discrimination, safe
header/body parsing, tests).

**Test coverage:** all three cases via `wiremock`. Without per-case fixtures,
the "secondary rate limit silently classified as permission denied" bug ships
unnoticed.

**If a second API later exhibits the same pattern:** introduce a pluggable
status classifier on `HttpClient`. Until then, YAGNI.

## 9. Refactor plan (steps 4–6)

### Step 4 — `local-files`

Line count essentially unchanged. No HTTP, no credential, no pagination.
**Purpose:** verification probe. If refactoring requires adding `HttpClient`,
`Credential`, or `Paginator` dependencies, the SDK has accidentally coupled
the `Connector` trait to HTTP — fix in the SDK before proceeding to step 5.

### Step 5 — `slack`

- **Removes:** `reqwest::Client` builder, bearer-auth + 401 mapping, cursor
  loops in `list_channels` and `fetch_messages`.
- **Adopts:** `HttpClient` with `StaticToken` (config key `token` or
  `SLACK_BOT_TOKEN` env var); two `BodyCursorPaginator` instances extracting
  cursor from `response_metadata.next_cursor`.
- Permission predicate emission unchanged (`channel:C01234` strings).

### Step 6 — `github` (+ 403 shim)

- **Removes:** `reqwest::Client` builder, bearer auth, `next_link` parser,
  Link-header loops.
- **Adopts:** `HttpClient` with `StaticToken` (config key `token` or
  `GITHUB_TOKEN` env var); UA suffix `github-connector/<pkg-ver>`; two
  `LinkHeaderPaginator` instances; 403 shim per §8.
- Ships 403 shim as its own sub-task within step 6 (likely its own commit).

### Credential source precedence

For both slack and github:

1. Config value present → use it.
2. Config absent, env var present → use env var.
3. Both present → use config, `tracing::warn!` that env var is ignored.
4. Neither present → error at **construction**, message names both sources.

Erroring at construction (not at first request) prevents opaque "no
credentials" failures at runtime.

### Line-count estimates

Rough scoping: slack 376 → ~240, github 454 → ~300. These are estimates, not
targets. Failure to hit them by a significant margin is a signal to
investigate: boilerplate may have been absorbed into ad-hoc adapter code
rather than genuinely eliminated.

## 10. Dependencies added

- `secrecy` (for `SecretString` in `StaticToken`).
- `wiremock` (dev-dep, for HTTP middleware tests).
- No other new runtime dependencies; `reqwest`, `futures`, `tokio`, `async-trait`,
  `serde`, `serde_json`, `tracing`, `time` are already workspace members.

Hand-rolled backoff over `tokio-retry`: `tokio-retry` lacks native
`Retry-After` parsing, so wrapping it reproduces most of the hand-roll. Direct
`tokio::time::sleep` + a small policy struct is cleaner (~80–120 lines) and
drops a dependency.

## 11. Testing strategy summary

- **SDK unit tests** (`cargo test -p sealstack-connector-sdk`): retry policy,
  `Retry-After` parsing, `StaticToken` env-var handling, `Paginator` adapter
  with in-memory mock, cursor-loop detection.
- **SDK integration tests** (behind `wiremock`): HTTP client retry behavior,
  body-size cap streaming, all three reference paginators against a live
  mock HTTP server.
- **Connector tests** (per-connector, via `wiremock`): each connector's list
  and fetch paths through the new SDK. GitHub's 403 shim tested for all three
  cases.

CI expectation: all existing connector tests pass unchanged after refactor.

## 12. Out-of-scope (follow-ups)

- Google Drive connector (next slice; will land OAuth 2.0 `Credential` impl).
- Proactive token-bucket rate limiting (if observed saturation warrants it).
- Pluggable status classifier on `HttpClient` (if a second API exhibits
  GitHub's 403 pattern).
- Streaming upload support in `HttpClient`.
- Typed `PermissionPredicate` shape (first pass when Drive's ACL model lands).
- Typed `SealStackError` variants for "env var missing" vs "env var empty"
  (if a consumer needs to match programmatically).
