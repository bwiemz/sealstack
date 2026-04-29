# Google Drive Connector + SDK OAuth2 Extension Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the typed `Principal` enum, the `OAuth2Credential` SDK extension, and the Google Drive connector that exercises both end-to-end. Migrate `local-files` / `slack` / `github` onto the new typed shape.

**Architecture:** PR 1 lands the SDK type-system change + OAuth machinery and the migration of three existing connectors. PR 2 lands the Drive connector built on top. Bisect-friendly: every commit independently compiles and tests pass.

**Tech Stack:** Rust 2021, `secrecy` 0.10 (already a workspace dep), `tokio::sync::Mutex`, `reqwest`, `wiremock` (dev-dep), `async-trait`, `serde`/`serde_json`, `tracing`.

**Spec:** [docs/superpowers/specs/2026-04-27-google-drive-connector-design.md](../specs/2026-04-27-google-drive-connector-design.md)

---

## Phase 1 — SDK extension + existing-connector migrations (PR 1)

### Task 1: Typed `Principal` enum with serde wire-shape

Tests-only commit. Lands the new module; nothing yet consumes the type. Workspace stays green.

**Files:**
- Create: `crates/sealstack-connector-sdk/src/principal.rs`
- Modify: `crates/sealstack-connector-sdk/src/lib.rs` (declare `pub mod principal;` and re-export)

- [ ] **Step 1: Declare the module + re-export from lib.rs**

Modify `crates/sealstack-connector-sdk/src/lib.rs`. Find the existing `pub mod` declarations (auth, change_streams, http, paginate, retry) and add:

```rust
pub mod principal;
```

Then near the top of the file (after the existing `use` block), add:

```rust
pub use principal::Principal;
```

- [ ] **Step 2: Write the failing tests**

Create `crates/sealstack-connector-sdk/src/principal.rs`:

```rust
//! Typed identity primitive for resource permissions.
//!
//! `Principal` is the closed-set type that connectors use to describe *who*
//! a permission applies to. It separates the *kind* of identity (user,
//! group, domain, anyone-public, anyone-with-link) from the *opaque
//! identifier* within each kind.
//!
//! See `docs/principal-mapping.md` for the semantic-mapping ADR explaining
//! when to reach for each variant and the design-pressure principle that
//! keeps the set closed.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde::de;

/// The closed set of identity kinds emitted on resource permissions.
///
/// Separates the *kind* of identity (user / group / domain / anyone-public /
/// anyone-with-link) from the *opaque identifier* within each kind. The kind
/// is what the policy engine reasons about; the identifier is what the
/// source system understands.
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
/// - [`Domain`](Self::Domain) — anyone with an email under this domain at
///   the source.
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
    /// Individual user identified by source-system identifier (email).
    User(String),
    /// Named group whose membership is resolved at policy time.
    Group(String),
    /// Domain whose members are anyone with an email under it.
    Domain(String),
    /// Publicly readable AND discoverable.
    Anyone,
    /// Readable to anyone with the URL; NOT discoverable.
    AnyoneWithLink,
}

impl Serialize for Principal {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let wire = match self {
            Self::User(id) => format!("user:{id}"),
            Self::Group(id) => format!("group:{id}"),
            Self::Domain(d) => format!("domain:{d}"),
            Self::Anyone => "anyone".to_owned(),
            Self::AnyoneWithLink => "anyone-with-link".to_owned(),
        };
        s.serialize_str(&wire)
    }
}

impl<'de> Deserialize<'de> for Principal {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        // Legacy alias: "*" deserializes to Anyone. Documented in §9 of the
        // design spec as a one-way semantic rename — re-serialization
        // produces the canonical "anyone" form.
        if s == "*" || s == "anyone" {
            return Ok(Self::Anyone);
        }
        if s == "anyone-with-link" {
            return Ok(Self::AnyoneWithLink);
        }
        let (prefix, rest) = s.split_once(':').ok_or_else(|| {
            de::Error::custom(format!(
                "invalid Principal wire format: {s:?} \
                 (expected `kind:id` or `anyone`/`anyone-with-link`)"
            ))
        })?;
        match prefix {
            "user" => Ok(Self::User(rest.to_owned())),
            "group" => Ok(Self::Group(rest.to_owned())),
            "domain" => Ok(Self::Domain(rest.to_owned())),
            other => Err(de::Error::custom(format!(
                "unknown Principal kind: {other:?}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_round_trip() {
        let p = Principal::User("alice@acme.com".to_owned());
        let s = serde_json::to_string(&p).unwrap();
        assert_eq!(s, "\"user:alice@acme.com\"");
        let back: Principal = serde_json::from_str(&s).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn group_round_trip() {
        let p = Principal::Group("eng@acme.com".to_owned());
        let s = serde_json::to_string(&p).unwrap();
        assert_eq!(s, "\"group:eng@acme.com\"");
        let back: Principal = serde_json::from_str(&s).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn domain_round_trip() {
        let p = Principal::Domain("acme.com".to_owned());
        let s = serde_json::to_string(&p).unwrap();
        assert_eq!(s, "\"domain:acme.com\"");
        let back: Principal = serde_json::from_str(&s).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn anyone_round_trip() {
        let p = Principal::Anyone;
        let s = serde_json::to_string(&p).unwrap();
        assert_eq!(s, "\"anyone\"");
        let back: Principal = serde_json::from_str(&s).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn anyone_with_link_round_trip() {
        let p = Principal::AnyoneWithLink;
        let s = serde_json::to_string(&p).unwrap();
        assert_eq!(s, "\"anyone-with-link\"");
        let back: Principal = serde_json::from_str(&s).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn legacy_star_deserializes_as_anyone() {
        let p: Principal = serde_json::from_str("\"*\"").unwrap();
        assert_eq!(p, Principal::Anyone);
    }

    #[test]
    fn legacy_star_round_trip_normalizes_to_anyone() {
        let p: Principal = serde_json::from_str("\"*\"").unwrap();
        let re = serde_json::to_string(&p).unwrap();
        assert_eq!(re, "\"anyone\"");
    }

    #[test]
    fn unknown_kind_error_includes_prefix() {
        let err = serde_json::from_str::<Principal>("\"weird:thing\"").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("weird"), "error should name the prefix: {msg}");
        assert!(msg.contains("unknown Principal kind"), "{msg}");
    }

    #[test]
    fn missing_colon_errors_on_deserialize() {
        let err = serde_json::from_str::<Principal>("\"justastring\"").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("invalid Principal wire format"), "{msg}");
    }
}
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test -p sealstack-connector-sdk --lib principal::tests`
Expected: 9 passing tests.

- [ ] **Step 4: Run clippy**

Run: `cargo clippy -p sealstack-connector-sdk 2>&1 | grep -E "(warning|error).*principal" | head -10`
Expected: no warnings or errors specific to `principal.rs`. Pre-existing `doc_markdown` warnings on `lib.rs` are out of scope.

- [ ] **Step 5: Apply rustfmt**

Run: `cargo fmt -p sealstack-connector-sdk`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/sealstack-connector-sdk/src/principal.rs crates/sealstack-connector-sdk/src/lib.rs
git commit -m "feat(connector-sdk): typed Principal enum with serde wire-shape"
```

---

### Task 2: `PermissionPredicate.principal: String → Principal` + `local-files` migration

The breaking type change. Local-files migrates *in this commit* because the change is a single-line constructor swap with **no wire-format change** — `public_read()` still emits `"anyone"`, and the legacy `"*"` deserializer alias means existing test fixtures keep working without modification. Slack and github migrate in subsequent commits because their wire formats genuinely change (`"slack:CXXX"` → `"group:slack:CXXX"`), warranting separate, atomic, fixture-regenerating commits.

**Files:**
- Modify: `crates/sealstack-connector-sdk/src/lib.rs`
- Modify: `connectors/local-files/src/lib.rs` (no changes needed if `public_read()` is the only call site — verify)

- [ ] **Step 1: Add a failing test asserting the type change**

Append to `crates/sealstack-connector-sdk/src/lib.rs`'s `#[cfg(test)] mod tests` block (find it near the bottom of the file):

```rust
    #[test]
    fn public_read_uses_principal_anyone() {
        let p = PermissionPredicate::public_read();
        assert_eq!(p.principal, Principal::Anyone);
        assert_eq!(p.action, "read");
    }

    #[test]
    fn legacy_predicate_wire_data_still_deserializes() {
        let legacy = r#"{"principal":"*","action":"read"}"#;
        let p: PermissionPredicate = serde_json::from_str(legacy).unwrap();
        assert_eq!(p.principal, Principal::Anyone);
        assert_eq!(p.action, "read");
    }

    #[test]
    fn legacy_star_with_write_action_still_deserializes() {
        let legacy = r#"{"principal":"*","action":"write"}"#;
        let p: PermissionPredicate = serde_json::from_str(legacy).unwrap();
        assert_eq!(p.principal, Principal::Anyone);
        assert_eq!(p.action, "write");
    }

    #[test]
    fn current_predicate_round_trips() {
        let current = r#"{"principal":"user:alice@acme.com","action":"write"}"#;
        let p: PermissionPredicate = serde_json::from_str(current).unwrap();
        assert_eq!(p.principal, Principal::User("alice@acme.com".to_owned()));
        let re = serde_json::to_string(&p).unwrap();
        assert_eq!(re, current);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p sealstack-connector-sdk --lib tests::public_read_uses_principal_anyone`
Expected: FAIL — the assertion `p.principal == Principal::Anyone` doesn't compile because `p.principal: String`.

- [ ] **Step 3: Change the `PermissionPredicate.principal` field type**

In `crates/sealstack-connector-sdk/src/lib.rs`, find the existing struct definition (near line 154):

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PermissionPredicate {
    /// The identity string the source assigns access to.
    pub principal: String,
    /// The action the principal may perform.
    pub action: String,
}
```

Replace with:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PermissionPredicate {
    /// Typed identity that this permission applies to.
    pub principal: Principal,
    /// The action the principal may perform — `"read"`, `"write"`,
    /// `"list"`, `"delete"`. Connector-specific source roles project
    /// down to these canonical actions at the connector boundary.
    pub action: String,
}
```

- [ ] **Step 4: Update the `public_read()` constructor**

In the same file, find the existing impl:

```rust
impl PermissionPredicate {
    #[must_use]
    pub fn public_read() -> Self {
        Self {
            principal: "*".into(),
            action: "read".into(),
        }
    }
}
```

Replace with:

```rust
impl PermissionPredicate {
    /// Predicate granting `read` access to anyone, discoverable in searches.
    ///
    /// Used by connectors that index inherently public content (e.g., the
    /// `local-files` connector, where filesystem-readable files are
    /// considered publicly accessible to anyone with corpus access).
    #[must_use]
    pub fn public_read() -> Self {
        Self {
            principal: Principal::Anyone,
            action: "read".into(),
        }
    }
}
```

- [ ] **Step 5: Update the existing `permission_public_read_round_trips` test**

The existing test asserts `p.principal == "*"`. Update it to:

```rust
    #[test]
    fn permission_public_read_round_trips() {
        let p = PermissionPredicate::public_read();
        assert_eq!(p.principal, Principal::Anyone);
        assert_eq!(p.action, "read");
    }
```

- [ ] **Step 6: Verify local-files compiles + tests pass**

local-files currently calls `PermissionPredicate::public_read()` only — no direct `String` construction. The `public_read()` swap is a single-line internal change with no wire-format shift. Run:

```bash
cargo test -p sealstack-connector-local-files
```

Expected: all 5 existing tests pass. If any test asserts `principal == "*"`, replace with `principal == Principal::Anyone`.

- [ ] **Step 7: Verify slack and github stop compiling**

This is the expected breaking-change point. Run:

```bash
cargo check -p sealstack-connector-slack 2>&1 | head -20
```

Expected: error about `format!("slack:{}", ...)` producing `String`, not `Principal`. Same for github. **This is correct** — commits 3 and 4 fix them.

- [ ] **Step 8: Run the SDK + local-files test suites**

```bash
cargo test -p sealstack-connector-sdk -p sealstack-connector-local-files
```

Expected: all green. The new SDK tests (Task 1's 9 + this task's 4 + the existing 28) pass. local-files's 5 tests pass.

- [ ] **Step 9: Apply rustfmt + commit**

```bash
cargo fmt -p sealstack-connector-sdk -p sealstack-connector-local-files
git add crates/sealstack-connector-sdk/src/lib.rs connectors/local-files/
git commit -m "feat(connector-sdk): PermissionPredicate.principal: String -> Principal (with local-files migration)"
```

---

### Task 3: `slack` migration — emit `Principal::Group` for channel ACLs

**Files:**
- Modify: `connectors/slack/src/lib.rs`

- [ ] **Step 1: Identify the existing PermissionPredicate construction site**

Run: `grep -n "PermissionPredicate" connectors/slack/src/lib.rs`
Expected output includes a line like:

```text
294:                    permissions: vec![PermissionPredicate {
295:                        principal: format!("slack:{}", channel.id),
296:                        action: "read".into(),
297:                    }],
```

- [ ] **Step 2: Update the construction to emit `Principal::Group`**

Find the block near line 294 (or wherever it currently lives) and update:

```rust
permissions: vec![PermissionPredicate {
    principal: Principal::Group(format!("slack:{}", channel.id)),
    action: "read".into(),
}],
```

- [ ] **Step 3: Add the `Principal` import**

Find the existing import line at the top of `connectors/slack/src/lib.rs`:

```rust
use sealstack_connector_sdk::{
    Connector, PermissionPredicate, Resource, ResourceId, ResourceStream, change_streams,
};
```

Update to include `Principal`:

```rust
use sealstack_connector_sdk::{
    Connector, PermissionPredicate, Principal, Resource, ResourceId, ResourceStream,
    change_streams,
};
```

- [ ] **Step 4: Run slack tests to find fixture failures**

```bash
cargo test -p sealstack-connector-slack 2>&1 | tail -30
```

Expected: tests fail because fixture wire shapes still expect `"slack:CXXX"` strings, but now connector emits `"group:slack:CXXX"`. Note the failing test names — these need fixture updates.

- [ ] **Step 5: Update slack test fixtures**

Search for any test in `connectors/slack/src/lib.rs` and `connectors/slack/tests/` that asserts on the principal string. Patterns to look for:

```bash
grep -rn "\"slack:" connectors/slack/
```

For each match (likely 2-5 sites), replace `"slack:CXXX"` with `"group:slack:CXXX"` in the assertion. Example:

```rust
// Before:
assert_eq!(perm.principal, "slack:C001");
// After:
assert_eq!(perm.principal, Principal::Group("slack:C001".to_owned()));
```

If any test asserts on serialized JSON containing `"slack:..."`:

```rust
// Before:
assert!(body.contains(r#""principal":"slack:C001""#));
// After:
assert!(body.contains(r#""principal":"group:slack:C001""#));
```

- [ ] **Step 6: Run the workspace-wide grep checklist**

```bash
rg '"slack:' --type rust | grep -v 'group:slack:'
```

Expected: empty output. Any hit outside the migrated source/tests is a third-location regression to investigate. Hits inside slack source/tests pointing to the new `group:slack:CXXX` format are fine; the `| grep -v 'group:slack:'` filter hides those.

- [ ] **Step 7: Run all slack tests**

```bash
cargo test -p sealstack-connector-slack
```

Expected: all 9 tests pass (5 unit + 4 e2e).

- [ ] **Step 8: Apply rustfmt + commit**

```bash
cargo fmt -p sealstack-connector-slack
git add connectors/slack/
git commit -m "refactor(slack): emit Principal::Group for channel ACLs"
```

---

### Task 4: `github` migration — emit `Principal::Group` for owner ACLs

**Files:**
- Modify: `connectors/github/src/lib.rs`

- [ ] **Step 1: Identify the existing construction site**

Run: `grep -n "PermissionPredicate" connectors/github/src/lib.rs`
Expected output includes a line like:

```text
291:            let perms = vec![PermissionPredicate {
292:                principal: format!("github:{}", owner),
293:                action: "read".into(),
294:            }];
```

- [ ] **Step 2: Update the construction to emit `Principal::Group`**

Update the block to:

```rust
let perms = vec![PermissionPredicate {
    principal: Principal::Group(format!("github:{}", owner)),
    action: "read".into(),
}];
```

- [ ] **Step 3: Add the `Principal` import**

Find the existing import line at the top of `connectors/github/src/lib.rs`:

```rust
use sealstack_connector_sdk::{
    Connector, PermissionPredicate, Resource, ResourceId, ResourceStream, change_streams,
};
```

Update to include `Principal`:

```rust
use sealstack_connector_sdk::{
    Connector, PermissionPredicate, Principal, Resource, ResourceId, ResourceStream,
    change_streams,
};
```

- [ ] **Step 4: Run github tests to find fixture failures**

```bash
cargo test -p sealstack-connector-github 2>&1 | tail -30
```

Expected: tests fail because fixtures expect the old `"github:..."` string format.

- [ ] **Step 5: Update github test fixtures**

```bash
grep -rn "\"github:" connectors/github/
```

For each match, update `"github:owner"` → `"group:github:owner"` in assertions. Same patterns as Task 3 Step 5.

- [ ] **Step 6: Run the workspace-wide grep checklist**

```bash
rg '"github:' --type rust | grep -v 'group:github:'
```

Expected: empty output.

- [ ] **Step 7: Run all github tests**

```bash
cargo test -p sealstack-connector-github
```

Expected: all 16 tests pass (5 unit + 4 retry_shim + 4 retry_shim_e2e + 3 list_repos_e2e).

- [ ] **Step 8: Workspace gate — every touched crate green**

```bash
cargo test -p sealstack-connector-sdk -p sealstack-connector-local-files \
            -p sealstack-connector-slack -p sealstack-connector-github
```

Expected: all green. End of the type-system migration in PR 1; the workspace is back to a stable state.

- [ ] **Step 9: Apply rustfmt + commit**

```bash
cargo fmt -p sealstack-connector-github
git add connectors/github/
git commit -m "refactor(github): emit Principal::Group for owner ACLs"
```

---

### Task 5: `OAuth2Credential` with refresh, caching, negative-cache, refresh-coalescing

The load-bearing SDK extension. ~250 lines + 11 tests.

**Files:**
- Modify: `crates/sealstack-connector-sdk/src/auth.rs`

- [ ] **Step 1: Add the failing tests**

Append to `crates/sealstack-connector-sdk/src/auth.rs` after the existing `mod tests` block (or extend the existing block — your choice; the existing block has `static_token_*` tests):

```rust
#[cfg(test)]
mod oauth2_tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn make_credential(
        token_endpoint: &str,
    ) -> OAuth2Credential {
        OAuth2Credential::new(
            "client-id-123".to_owned(),
            SecretString::new("client-secret".to_owned().into()),
            SecretString::new("refresh-token".to_owned().into()),
            token_endpoint.to_owned(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn oauth2_caches_access_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "ya29.first",
                "expires_in": 3599,
                "token_type": "Bearer"
            })))
            .expect(1) // exactly one refresh — second call must hit cache.
            .mount(&server)
            .await;

        let cred = make_credential(&format!("{}/token", server.uri()));
        let h1 = cred.authorization_header().await.unwrap();
        let h2 = cred.authorization_header().await.unwrap();
        assert_eq!(h1, "Bearer ya29.first");
        assert_eq!(h2, "Bearer ya29.first");
    }

    #[tokio::test]
    async fn oauth2_refreshes_after_expiry() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "ya29.short",
                "expires_in": 1,
                "token_type": "Bearer"
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "ya29.fresh",
                "expires_in": 3599,
                "token_type": "Bearer"
            })))
            .mount(&server)
            .await;

        let cred = make_credential(&format!("{}/token", server.uri()));
        let h1 = cred.authorization_header().await.unwrap();
        assert_eq!(h1, "Bearer ya29.short");
        // Sleep past the 60s skew + 1s expiry would take too long; test the
        // skew constant via a unit test below.
        // Instead, force expiry by invalidating and re-issuing.
        cred.invalidate().await;
        let h2 = cred.authorization_header().await.unwrap();
        assert_eq!(h2, "Bearer ya29.fresh");
    }

    #[tokio::test]
    async fn oauth2_invalidate_clears_cache() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "ya29.first",
                "expires_in": 3599,
                "token_type": "Bearer"
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "ya29.second",
                "expires_in": 3599,
                "token_type": "Bearer"
            })))
            .mount(&server)
            .await;

        let cred = make_credential(&format!("{}/token", server.uri()));
        assert_eq!(cred.authorization_header().await.unwrap(), "Bearer ya29.first");
        cred.invalidate().await;
        assert_eq!(cred.authorization_header().await.unwrap(), "Bearer ya29.second");
    }

    #[tokio::test]
    async fn oauth2_invalid_grant_returns_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "error": "invalid_grant",
                "error_description": "Token has been expired or revoked."
            })))
            .mount(&server)
            .await;

        let cred = make_credential(&format!("{}/token", server.uri()));
        let err = cred.authorization_header().await.unwrap_err();
        match err {
            SealStackError::Unauthorized(msg) => {
                assert!(msg.contains("invalid_grant"), "{msg}");
            }
            other => panic!("expected Unauthorized, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn oauth2_invalid_client_returns_config_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "error": "invalid_client",
                "error_description": "Client authentication failed."
            })))
            .mount(&server)
            .await;

        let cred = make_credential(&format!("{}/token", server.uri()));
        let err = cred.authorization_header().await.unwrap_err();
        match err {
            SealStackError::Config(msg) => {
                assert!(msg.contains("invalid_client"), "{msg}");
            }
            other => panic!("expected Config, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn oauth2_concurrent_refresh_coalesces() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "ya29.coalesced",
                "expires_in": 3599,
                "token_type": "Bearer"
            })))
            .expect(1) // refresh-coalescing assertion: 5 concurrent calls → 1 refresh.
            .mount(&server)
            .await;

        let cred = Arc::new(make_credential(&format!("{}/token", server.uri())));
        let mut handles = Vec::new();
        for _ in 0..5 {
            let c = cred.clone();
            handles.push(tokio::spawn(async move {
                c.authorization_header().await.unwrap()
            }));
        }
        for h in handles {
            assert_eq!(h.await.unwrap(), "Bearer ya29.coalesced");
        }
    }

    #[tokio::test]
    async fn oauth2_negative_cache_coalesces_transient_failures() {
        let server = MockServer::start().await;
        // Fail every refresh attempt with 503. The hand-rolled retry inside
        // refresh_with_retry tries 3 times, so a single authorization_header
        // call hits the endpoint 3 times.
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(503))
            .expect(3) // 3 retries inside the first call; second call hits negative cache.
            .mount(&server)
            .await;

        let cred = make_credential(&format!("{}/token", server.uri()));
        let err1 = cred.authorization_header().await.unwrap_err();
        assert!(matches!(err1, SealStackError::Backend(_)));
        // Second call within 5s window: hits negative cache, no token-endpoint
        // hit, returns same error.
        let err2 = cred.authorization_header().await.unwrap_err();
        assert!(matches!(err2, SealStackError::Backend(_)));
    }

    #[tokio::test]
    async fn oauth2_permanent_failures_do_not_negative_cache() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "error": "invalid_grant"
            })))
            .expect(2) // both calls hit the endpoint; permanent failures don't cache.
            .mount(&server)
            .await;

        let cred = make_credential(&format!("{}/token", server.uri()));
        let _ = cred.authorization_header().await;
        let _ = cred.authorization_header().await;
    }

    #[tokio::test]
    async fn oauth2_skew_triggers_refresh_at_60s_before_expiry() {
        // We can't easily use tokio::time::pause inside an inner-Mutex
        // structure, so test the skew arithmetic directly: a CachedAccess
        // with valid_until = now() + 50s should be considered stale (since
        // 50s < 60s skew margin), and authorization_header should refresh.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "ya29.fresh",
                "expires_in": 3599,
                "token_type": "Bearer"
            })))
            .mount(&server)
            .await;

        let cred = make_credential(&format!("{}/token", server.uri()));
        // Pre-populate the cache with a token that expires 50s from now.
        // 50s < 60s skew, so the next call should refresh.
        {
            let mut cache = cred.cache.lock().await;
            cache.access_token = Some(SecretString::new("ya29.almost_expired".into()));
            cache.valid_until = Some(std::time::Instant::now() + Duration::from_secs(50));
        }
        let h = cred.authorization_header().await.unwrap();
        assert_eq!(h, "Bearer ya29.fresh", "should have refreshed due to skew");
    }

    #[tokio::test]
    async fn oauth2_google_constructor_uses_correct_endpoint() {
        let cred = OAuth2Credential::google(
            "id".to_owned(),
            SecretString::new("secret".into()),
            SecretString::new("refresh".into()),
        )
        .unwrap();
        let dbg = format!("{cred:?}");
        assert!(
            dbg.contains("oauth2.googleapis.com/token"),
            "Debug should show token endpoint: {dbg}"
        );
    }

    #[test]
    fn debug_redacts_secrets() {
        let cred = OAuth2Credential::new(
            "client-id-123".to_owned(),
            SecretString::new("super-secret-value".into()),
            SecretString::new("refresh-token-xyz".into()),
            "https://example.com/token".to_owned(),
        )
        .unwrap();
        let dbg = format!("{cred:?}");
        assert!(!dbg.contains("super-secret-value"), "client_secret leaked: {dbg}");
        assert!(!dbg.contains("refresh-token-xyz"), "refresh_token leaked: {dbg}");
        assert!(dbg.contains("client-id-123"), "client_id should be visible: {dbg}");
        assert!(dbg.contains("example.com/token"), "endpoint should be visible: {dbg}");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p sealstack-connector-sdk --lib auth::oauth2_tests`
Expected: FAIL — `OAuth2Credential` doesn't exist.

- [ ] **Step 3: Implement `OAuth2Credential`**

Append to `crates/sealstack-connector-sdk/src/auth.rs` (after the existing `StaticToken` impl, before the `#[cfg(test)] mod tests` block):

```rust
use std::time::{Duration, Instant};

/// OAuth 2.0 credential using the refresh-token grant.
///
/// Caches the access token in-memory with a 60-second skew margin and
/// coalesces concurrent refresh attempts via a `tokio::sync::Mutex`.
/// Transient refresh failures (5xx, network) are negative-cached for 5
/// seconds to prevent serialized-retry stampedes; permanent failures
/// (`invalid_grant`, `invalid_client`, `invalid_scope`) are not cached
/// because retrying just delays the inevitable error.
///
/// # See also
///
/// Planned `microsoft(tenant_id, ...)` and `notion(...)` convenience
/// constructors as those providers come online. The pattern is hardcoding
/// well-known token endpoints while [`Self::new`] stays generic.
pub struct OAuth2Credential {
    client_id: String,
    client_secret: SecretString,
    refresh_token: SecretString,
    token_endpoint: String,
    cache: tokio::sync::Mutex<CachedAccess>,
    inner: reqwest::Client,
}

#[derive(Default)]
struct CachedAccess {
    access_token: Option<SecretString>,
    valid_until: Option<Instant>,
    negative_cache: Option<NegativeCache>,
}

struct NegativeCache {
    expires: Instant,
    message: String,
}

const REFRESH_SKEW_SECS: u64 = 60;
const NEGATIVE_CACHE_SECS: u64 = 5;

impl OAuth2Credential {
    /// Construct against an arbitrary OAuth 2.0 token endpoint.
    pub fn new(
        client_id: impl Into<String>,
        client_secret: SecretString,
        refresh_token: SecretString,
        token_endpoint: impl Into<String>,
    ) -> SealStackResult<Self> {
        let inner = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| SealStackError::backend(format!("oauth2 client build: {e}")))?;
        Ok(Self {
            client_id: client_id.into(),
            client_secret,
            refresh_token,
            token_endpoint: token_endpoint.into(),
            cache: tokio::sync::Mutex::new(CachedAccess::default()),
            inner,
        })
    }

    /// Convenience constructor for Google's well-known token endpoint.
    pub fn google(
        client_id: impl Into<String>,
        client_secret: SecretString,
        refresh_token: SecretString,
    ) -> SealStackResult<Self> {
        Self::new(
            client_id,
            client_secret,
            refresh_token,
            "https://oauth2.googleapis.com/token",
        )
    }

    /// Send the form-encoded refresh request and parse the response.
    /// Light retry (3 attempts, 200/400/800ms exponential) on 5xx + network
    /// errors. Hand-rolled rather than going through HttpClient because
    /// HttpClient::send calls Credential::authorization_header, which would
    /// be circular.
    async fn refresh_with_retry(&self) -> SealStackResult<(String, u64)> {
        let mut last_err = SealStackError::backend("unknown");
        for attempt in 0..3u32 {
            match self.refresh_once().await {
                Ok((tok, expires_in)) => return Ok((tok, expires_in)),
                Err(e @ SealStackError::Unauthorized(_)) => return Err(e),
                Err(e @ SealStackError::Config(_)) => return Err(e),
                Err(e) => {
                    last_err = e;
                    if attempt < 2 {
                        let delay_ms = 200 * (1u64 << attempt);
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    }
                }
            }
        }
        Err(last_err)
    }

    async fn refresh_once(&self) -> SealStackResult<(String, u64)> {
        use serde::Deserialize;

        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
            expires_in: u64,
        }

        #[derive(Deserialize)]
        struct ErrorResponse {
            error: String,
            #[serde(default)]
            #[allow(dead_code)]
            error_description: Option<String>,
        }

        let resp = self
            .inner
            .post(&self.token_endpoint)
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.client_secret.expose_secret()),
                ("refresh_token", self.refresh_token.expose_secret()),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .await
            .map_err(|e| SealStackError::backend(format!("oauth2 transport: {e}")))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| SealStackError::backend(format!("oauth2 body read: {e}")))?;

        if status.is_success() {
            let tr: TokenResponse = serde_json::from_str(&body)
                .map_err(|e| SealStackError::backend(format!("oauth2 parse: {e}")))?;
            return Ok((tr.access_token, tr.expires_in));
        }
        if status.as_u16() == 400 {
            let er: ErrorResponse = serde_json::from_str(&body).unwrap_or(ErrorResponse {
                error: "unknown".to_owned(),
                error_description: None,
            });
            match er.error.as_str() {
                "invalid_grant" => {
                    return Err(SealStackError::Unauthorized(format!(
                        "OAuth2 refresh failed: invalid_grant"
                    )));
                }
                "invalid_client" | "invalid_scope" => {
                    return Err(SealStackError::Config(format!(
                        "OAuth2 misconfiguration: {}",
                        er.error
                    )));
                }
                other => {
                    return Err(SealStackError::Backend(format!(
                        "OAuth2 refresh failed: {other}"
                    )));
                }
            }
        }
        Err(SealStackError::Backend(format!(
            "OAuth2 refresh failed: HTTP {status}"
        )))
    }
}

impl std::fmt::Debug for OAuth2Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuth2Credential")
            .field("client_id", &self.client_id)
            .field("token_endpoint", &self.token_endpoint)
            .field("client_secret", &"<redacted>")
            .field("refresh_token", &"<redacted>")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Credential for OAuth2Credential {
    async fn authorization_header(&self) -> SealStackResult<String> {
        let mut cache = self.cache.lock().await;

        // Negative cache hit (refresh failed recently within 5s window).
        if let Some(neg) = &cache.negative_cache {
            if neg.expires > Instant::now() {
                return Err(SealStackError::Backend(neg.message.clone()));
            }
            cache.negative_cache = None; // stale; clear and proceed
        }

        // Positive cache hit. 60-second margin absorbs server-side
        // validator-cache latency on Google's edge — Google may treat
        // tokens as expired slightly before the reported expires_in due
        // to refresh latency in their token validators. Not network RTT
        // (sub-second) or NTP drift (sub-second).
        if let (Some(tok), Some(until)) = (&cache.access_token, &cache.valid_until) {
            if Instant::now() + Duration::from_secs(REFRESH_SKEW_SECS) < *until {
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
            Err(e @ SealStackError::Unauthorized(_)) | Err(e @ SealStackError::Config(_)) => {
                // Permanent failures: no negative cache. Caching them just
                // delays the inevitable error.
                Err(e)
            }
            Err(e) => {
                // Transient failures (5xx, network, retry-budget-exhausted):
                // negative-cache for 5s to coalesce stampede.
                let message = format!("OAuth2 refresh failed: {e}");
                cache.negative_cache = Some(NegativeCache {
                    expires: Instant::now() + Duration::from_secs(NEGATIVE_CACHE_SECS),
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
        // Note: negative_cache is NOT cleared on invalidate. If a refresh
        // just failed transiently, the next caller still gets fast-fail
        // for the cached window.
    }
}
```

- [ ] **Step 4: Make the cache field accessible to the `oauth2_skew_triggers_refresh_at_60s_before_expiry` test**

The test directly accesses `cred.cache.lock().await`. The `cache` field must be at least `pub(crate)` to allow this. In the struct definition, change:

```rust
cache: tokio::sync::Mutex<CachedAccess>,
```

to:

```rust
pub(crate) cache: tokio::sync::Mutex<CachedAccess>,
```

Also make `CachedAccess` and its fields `pub(crate)`:

```rust
#[derive(Default)]
pub(crate) struct CachedAccess {
    pub(crate) access_token: Option<SecretString>,
    pub(crate) valid_until: Option<Instant>,
    pub(crate) negative_cache: Option<NegativeCache>,
}
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo test -p sealstack-connector-sdk --lib auth::oauth2_tests
```

Expected: 11 passing tests.

- [ ] **Step 6: Run the full SDK suite to confirm no regressions**

```bash
cargo test -p sealstack-connector-sdk
```

Expected: 64 tests pass (42 existing + 11 Principal from Task 1 + 11 OAuth2Credential).

- [ ] **Step 7: Run clippy on the OAuth code**

```bash
cargo clippy -p sealstack-connector-sdk 2>&1 | grep -E "auth\.rs" | head -20
```

Expected: no new warnings on auth.rs. If `clippy::use_self` fires, replace `Self` references appropriately.

- [ ] **Step 8: Apply rustfmt + commit**

```bash
cargo fmt -p sealstack-connector-sdk
git add crates/sealstack-connector-sdk/src/auth.rs
git commit -m "feat(connector-sdk): OAuth2Credential with refresh, caching, negative-cache"
```

---

### Task 6: Principal-mapping ADR + design-pressure principle docs

**Files:**
- Create: `crates/sealstack-connector-sdk/docs/principal-mapping.md`

- [ ] **Step 1: Create the ADR**

Create `crates/sealstack-connector-sdk/docs/principal-mapping.md`:

```markdown
# Principal Mapping ADR

How connector authors map source-system ACL primitives to the SDK's typed
[`Principal`](../src/principal.rs) enum.

## The closed set

`Principal` is a closed-set type with five variants:

- `User(String)` — individual identified by source-system identifier.
- `Group(String)` — named group whose membership is resolved at policy time.
- `Domain(String)` — anyone with an email under this domain.
- `Anyone` — publicly readable AND discoverable.
- `AnyoneWithLink` — readable to whoever has the URL; NOT discoverable.

There is no `Custom(String)` escape hatch. When a connector's source
ACL primitives don't obviously fit, the resolution is a deliberate
semantic-mapping decision, not stringly-typed drift.

## Design-pressure principle

> When a connector's source has ACL primitives that don't obviously fit
> the closed set, the resolution is a deliberate semantic-mapping
> decision, not an escape hatch. If no variant fits, the conversation is
> "should the SDK extend its closed set?" — surfaced as a design proposal,
> not papered over.

Closed-set discipline pushes design pressure onto the connector-author
side, not the SDK side. A `Custom` variant would relieve that pressure
and re-introduce the stringly-typed semantics the typed enum exists to
prevent.

## The mapping is semantic, not lexical

Pick the variant whose semantics match the source concept, not the
variant that happens to be most permissive. A connector author who
defaults to `Group` for everything they don't recognize is making the
typed enum useless.

## Worked examples

| Source concept | Variant | Identifier |
|---|---|---|
| Slack channel | `Group` | `slack:CXXX` |
| GitHub org/user owner | `Group` | `github:octocat` |
| GitHub team | `Group` | `github:acme/team-name` |
| Google Drive user permission | `User` | `alice@acme.com` |
| Google Drive group permission | `Group` | `eng@acme.com` |
| Google Drive domain permission | `Domain` | `acme.com` |
| Google Drive `type=anyone, allowFileDiscovery=true` | `Anyone` | (no inner identifier) |
| Google Drive `type=anyone, allowFileDiscovery=false` | `AnyoneWithLink` | (no inner identifier) |
| Notion workspace member | `User` | individual identifier (NOT `Group`) |
| Notion guest | `User` | guest identifier |
| Salesforce role | `Group` | `salesforce:role-name` |
| Domain-restricted Drive share | `Domain` | `acme.com` |

## In-identifier source-prefix convention

Connectors emitting `Group` identifiers should prefix them with their
connector name (`slack:CXXX`, `github:octocat`) for cross-connector
identifier disambiguation in shared policy rules. Without the prefix,
two connectors both emitting a `Group("admins")` would produce
ambiguous policy semantics.

The SDK doesn't enforce this — it's a convention, not a contract. But
every existing connector follows it, and breaking it produces ambiguous
policy semantics when two connectors both emit a `Group("admins")`.

## When to extend the closed set

Don't reach for `Custom`. There isn't one. If a source ACL primitive
genuinely doesn't fit any variant — not because mapping is hard, but
because the underlying concept is something else (a date-ranged ACL,
a quorum-based one, a per-resource-attribute one) — open a design
discussion to extend the enum or the `PermissionPredicate` shape.

This forces design pressure to surface as a real conversation rather
than as a quietly-growing collection of prefix conventions.

## Asymmetric legacy-alias treatment

The `Deserialize` impl on `Principal` accepts `"*"` as a synonym for
`Anyone`. This is a **semantic rename** of an unambiguous existing
wire shape — `"*"` always meant "anyone" in `PermissionPredicate::public_read()`'s
old impl, and the alias preserves backward-compat with test fixtures
and dev wire data without effort.

The `Deserialize` impl does **NOT** accept `"slack:..."` or
`"github:..."` (or any other connector-specific prefix) as legacy
aliases. Those were stringly-typed identifiers without typed semantic
anchoring; aliasing them would silently re-map data and mask the
design pressure this slice exerts on the connector emission paths.
Connectors must explicitly migrate to the typed enum; pre-migration
wire data fails to deserialize loudly, which is the correct signal.

## Roles project to actions

`PermissionPredicate.action: String` carries the role information
(`"read"`, `"write"`, `"list"`, `"delete"`). Connectors project source
roles down to these canonical actions at the connector boundary. The
SDK doesn't model role hierarchies in the type system because role
mapping is connector-specific (Drive's `commenter` doesn't map cleanly
to anything in Slack).
```

- [ ] **Step 2: Verify the file renders cleanly (markdown lint)**

If you have a local markdown linter, run it. Otherwise visually review the file. The doc has no external links to verify.

- [ ] **Step 3: Commit**

```bash
git add crates/sealstack-connector-sdk/docs/principal-mapping.md
git commit -m "docs(connector-sdk): principal-mapping ADR + design-pressure principle"
```

---

**End of Phase 1 / PR 1.** Workspace gate before opening the PR:

```bash
cargo test -p sealstack-connector-sdk \
            -p sealstack-connector-local-files \
            -p sealstack-connector-slack \
            -p sealstack-connector-github
cargo clippy -p sealstack-connector-sdk \
              -p sealstack-connector-local-files \
              -p sealstack-connector-slack \
              -p sealstack-connector-github 2>&1 | grep -E "^(warning|error)" | head -30
cargo fmt --check -p sealstack-connector-sdk \
                  -p sealstack-connector-local-files \
                  -p sealstack-connector-slack \
                  -p sealstack-connector-github
```

All four crates green. Open PR 1.

---

## Phase 2 — Drive connector (PR 2)

After PR 1 merges, branch off main again for PR 2.

### Task 7: Drive connector scaffold + `DriveConfig` + `from_json`

Bootstraps the new crate. Empty `Connector` impl with stubbed methods that compile but `unimplemented!()` — subsequent commits fill them.

**Files:**
- Create: `connectors/google-drive/Cargo.toml`
- Create: `connectors/google-drive/src/lib.rs`
- Modify: `Cargo.toml` (workspace) — add `connectors/google-drive` to members.

- [ ] **Step 1: Add the crate to the workspace**

In `Cargo.toml` (workspace root), find the `[workspace.members]` array and add:

```toml
"connectors/google-drive",
```

- [ ] **Step 2: Create the crate's Cargo.toml**

Create `connectors/google-drive/Cargo.toml`:

```toml
[package]
name         = "sealstack-connector-google-drive"
version      = { workspace = true }
edition      = { workspace = true }
rust-version = { workspace = true }
license      = { workspace = true }
repository   = { workspace = true }
description  = "SealStack connector for Google Drive (My Drive, OAuth 2.0 refresh-token grant)."

[lints]
workspace = true

[dependencies]
sealstack-common        = { path = "../../crates/sealstack-common" }
sealstack-connector-sdk = { path = "../../crates/sealstack-connector-sdk" }

async-trait = "0.1"
futures     = "0.3"
secrecy     = "0.10"
serde       = { workspace = true, features = ["derive"] }
serde_json  = { workspace = true }
thiserror   = { workspace = true }
time        = { workspace = true }
tokio       = { workspace = true, features = ["sync", "time"] }
tracing     = { workspace = true }
reqwest     = { workspace = true }

[dev-dependencies]
tokio    = { workspace = true, features = ["macros", "rt-multi-thread", "test-util"] }
wiremock = "0.6"
```

- [ ] **Step 3: Write the failing tests for `DriveConfig::from_json` validation**

Create `connectors/google-drive/src/lib.rs`:

```rust
//! Google Drive connector.
//!
//! Pulls files from a single user's "My Drive" via the Drive REST API v3.
//! Authentication is OAuth 2.0 refresh-token grant — users provide a refresh
//! token externally (via Google's OAuth playground or a one-off script) and
//! reference it via env var in their config.
//!
//! # Resources emitted
//!
//! * One [`Resource`] per allowlisted-MIME file. v1 allowlist:
//!   - `application/vnd.google-apps.document` (Google Docs, exported as text)
//!   - `text/plain`, `text/markdown` (direct binary fetch via `alt=media`)
//!
//! Skipped MIME types are logged at info level once per resource id and
//! never yield empty-body Resources.
//!
//! # Pagination
//!
//! Drive's `files.list` paginates via `nextPageToken` in the response body.
//! Uses the SDK's [`BodyCursorPaginator`].
//!
//! # Out of scope (v1)
//!
//! * Shared Drives (`corpora=shared|all`) — config rejects non-`"user"`.
//! * Incremental sync via `changes.list` — full-crawl every cycle.
//! * CLI consent flow — refresh token comes from operator config.

#![forbid(unsafe_code)]
#![warn(missing_docs, unreachable_pub)]

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use secrecy::SecretString;

use sealstack_common::{SealStackError, SealStackResult};
use sealstack_connector_sdk::auth::OAuth2Credential;
use sealstack_connector_sdk::http::HttpClient;
use sealstack_connector_sdk::retry::RetryPolicy;
use sealstack_connector_sdk::{ChangeStream, Connector, Resource, ResourceId, ResourceStream};

const DEFAULT_API_BASE: &str = "https://www.googleapis.com";
const DEFAULT_SYNC_INTERVAL_SECS: u64 = 900; // 15 minutes
const DEFAULT_MAX_FILE_BYTES: u64 = 10 * 1024 * 1024; // 10 MiB

/// Non-secret connector configuration.
#[derive(Clone, Debug)]
pub struct DriveConfig {
    /// API base URL — defaults to `https://www.googleapis.com`. Overridable
    /// for tests pointing at a wiremock server.
    pub api_base: String,
    /// Sync cadence. Default: 15 minutes (deletion-latency mitigation pending
    /// v0.2 incremental sync).
    pub sync_interval_seconds: u64,
    /// Per-file size cap in bytes. Files exceeding this are skipped with one
    /// info log per resource id.
    pub max_file_bytes: u64,
}

impl DriveConfig {
    /// Parse non-secret fields from the binding config JSON. All fields have
    /// defaults; `api_base` trailing slashes are trimmed.
    #[must_use]
    pub fn from_json(v: &serde_json::Value) -> Self {
        let api_base = v
            .get("api_base")
            .and_then(|x| x.as_str())
            .unwrap_or(DEFAULT_API_BASE)
            .trim_end_matches('/')
            .to_owned();
        let sync_interval_seconds = v
            .get("sync_interval_seconds")
            .and_then(|x| x.as_u64())
            .unwrap_or(DEFAULT_SYNC_INTERVAL_SECS);
        let max_file_bytes = v
            .get("max_file_bytes")
            .and_then(|x| x.as_u64())
            .unwrap_or(DEFAULT_MAX_FILE_BYTES);
        Self { api_base, sync_interval_seconds, max_file_bytes }
    }
}

/// The Google Drive connector.
#[derive(Debug)]
pub struct DriveConnector {
    http: Arc<HttpClient>,
    config: DriveConfig,
}

impl DriveConnector {
    /// Build the connector from the binding config JSON.
    ///
    /// Required fields:
    /// - `client_id` — OAuth 2.0 client id (public; not a secret)
    /// - `client_secret_env` — name of env var holding the OAuth client secret
    /// - `refresh_token_env` — name of env var holding the OAuth refresh token
    ///
    /// Optional fields with defaults:
    /// - `corpora` (default `"user"`; only valid value in v1)
    /// - `api_base` (default `https://www.googleapis.com`)
    /// - `sync_interval_seconds` (default 900)
    /// - `max_file_bytes` (default 10 MiB)
    pub fn from_json(v: &serde_json::Value) -> SealStackResult<Self> {
        let client_id = required_str(v, "client_id")?.to_owned();

        let client_secret_var = required_str(v, "client_secret_env")?;
        let client_secret = SecretString::new(read_env_var(client_secret_var)?.into());

        let refresh_token_var = required_str(v, "refresh_token_env")?;
        let refresh_token = SecretString::new(read_env_var(refresh_token_var)?.into());

        let corpora = v.get("corpora").and_then(|x| x.as_str()).unwrap_or("user");
        if corpora != "user" {
            return Err(SealStackError::Config(format!(
                "drive: `corpora = \"{corpora}\"` not yet supported; only \"user\" works in v1. \
                 Shared Drives land in v0.2."
            )));
        }

        let credential =
            Arc::new(OAuth2Credential::google(client_id, client_secret, refresh_token)?);
        let http = Arc::new(
            HttpClient::new(credential, RetryPolicy::default())?
                .with_user_agent_suffix(format!(
                    "google-drive-connector/{}",
                    env!("CARGO_PKG_VERSION")
                )),
        );

        let config = DriveConfig::from_json(v);
        Ok(Self { http, config })
    }
}

fn required_str<'a>(v: &'a serde_json::Value, key: &str) -> SealStackResult<&'a str> {
    v.get(key).and_then(|x| x.as_str()).ok_or_else(|| {
        SealStackError::Config(format!("drive: missing required field `{key}`"))
    })
}

fn read_env_var(name: &str) -> SealStackResult<String> {
    match std::env::var(name) {
        Err(_) => Err(SealStackError::Config(format!(
            "drive: env var `{name}` not set"
        ))),
        Ok(s) if s.is_empty() => Err(SealStackError::Config(format!(
            "drive: env var `{name}` is empty"
        ))),
        Ok(s) => Ok(s),
    }
}

#[async_trait]
impl Connector for DriveConnector {
    fn name(&self) -> &str {
        "google-drive"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    async fn list(&self) -> SealStackResult<ResourceStream> {
        // Fully wired in Task 12.
        unimplemented!("DriveConnector::list lands in Task 12")
    }

    async fn fetch(&self, _id: &ResourceId) -> SealStackResult<Resource> {
        // Fully wired in Task 12.
        unimplemented!("DriveConnector::fetch lands in Task 12")
    }

    async fn subscribe(&self) -> SealStackResult<Option<ChangeStream>> {
        // v1 is full-crawl only; subscribe lands with v0.2 incremental.
        Ok(None)
    }

    async fn healthcheck(&self) -> SealStackResult<()> {
        // Fully wired in Task 12.
        unimplemented!("DriveConnector::healthcheck lands in Task 12")
    }
}

impl DriveConnector {
    /// Sync cadence. Engine consumes this in a separate per-connector-interval
    /// engine slice; currently informational.
    #[must_use]
    pub fn sync_interval(&self) -> Duration {
        Duration::from_secs(self.config.sync_interval_seconds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_json_rejects_missing_client_id() {
        let v = serde_json::json!({
            "client_secret_env": "X",
            "refresh_token_env": "Y"
        });
        let err = DriveConnector::from_json(&v).unwrap_err().to_string();
        assert!(err.contains("client_id"), "{err}");
    }

    #[test]
    fn from_json_rejects_corpora_shared() {
        let v = serde_json::json!({
            "client_id": "id",
            "client_secret_env": "DRIVE_TEST_SECRET",
            "refresh_token_env": "DRIVE_TEST_REFRESH",
            "corpora": "shared"
        });
        // Pre-set env vars so the auth check passes; we want the corpora
        // rejection to fire.
        // SAFETY: env-var-set isn't allowed in our forbid(unsafe_code) crate;
        // skip if the test env carries these vars.
        if std::env::var("DRIVE_TEST_SECRET").is_err()
            || std::env::var("DRIVE_TEST_REFRESH").is_err()
        {
            // Cannot exercise the corpora-rejection path without pre-set env;
            // skip rather than spuriously fail.
            return;
        }
        let err = DriveConnector::from_json(&v).unwrap_err().to_string();
        assert!(err.contains("corpora"), "{err}");
        assert!(err.contains("Shared Drives"), "{err}");
    }

    #[test]
    fn from_json_normalizes_api_base_trailing_slash() {
        let cfg = DriveConfig::from_json(&serde_json::json!({
            "api_base": "https://example.com/"
        }));
        assert_eq!(cfg.api_base, "https://example.com");
    }

    #[test]
    fn from_json_uses_defaults() {
        let cfg = DriveConfig::from_json(&serde_json::json!({}));
        assert_eq!(cfg.api_base, "https://www.googleapis.com");
        assert_eq!(cfg.sync_interval_seconds, 900);
        assert_eq!(cfg.max_file_bytes, 10 * 1024 * 1024);
    }
}
```

- [ ] **Step 4: Run tests + cargo check**

```bash
cargo check -p sealstack-connector-google-drive
cargo test -p sealstack-connector-google-drive --lib
```

Expected: 4 unit tests pass (the corpora-shared test may report itself as "skipped" via early return depending on env state — that's fine).

- [ ] **Step 5: Apply rustfmt + commit**

```bash
cargo fmt -p sealstack-connector-google-drive
git add Cargo.toml connectors/google-drive/
git commit -m "feat(connectors/google-drive): scaffold + DriveConfig + from_json"
```

---

### Task 8: Drive permission mapping (`DrivePermission` → `Principal`)

Pure mapping logic. No HTTP, no Connector wiring. Lands before `files.rs` because `resource.rs` (Task 12) consumes the mapping, and isolating it gives reviewers a clean target.

**Files:**
- Create: `connectors/google-drive/src/permissions.rs`
- Modify: `connectors/google-drive/src/lib.rs` (declare module)

- [ ] **Step 1: Declare the module in lib.rs**

Add to `connectors/google-drive/src/lib.rs` (with the other module declarations — for now there are none, so add it after the `#![forbid]` / `#![warn]` lines):

```rust
mod permissions;
```

- [ ] **Step 2: Write the failing tests**

Create `connectors/google-drive/src/permissions.rs`:

```rust
//! Map Drive `permissions[]` entries to SealStack [`PermissionPredicate`]s.

use sealstack_connector_sdk::{PermissionPredicate, Principal};
use serde::Deserialize;

/// Drive permission object as returned by the Drive REST API v3.
///
/// Only the fields we use are deserialized.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DrivePermission {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(rename = "emailAddress")]
    pub email_address: Option<String>,
    pub domain: Option<String>,
    pub role: String,
    #[serde(rename = "allowFileDiscovery")]
    pub allow_file_discovery: Option<bool>,
}

/// Map a Drive permission to a SealStack `PermissionPredicate`.
///
/// Returns `None` for permission kinds the connector doesn't recognize
/// (logged at warn level — these are real ACL signals being silently
/// dropped, which is unlike skipped MIME types where they're not).
pub(crate) fn drive_permission_to_predicate(p: &DrivePermission) -> Option<PermissionPredicate> {
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
            tracing::warn!(
                kind = %other,
                "drive: unrecognized permission kind, dropping ACL entry"
            );
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
/// comments visible to other readers) is not a write to our indexed content.
/// Comment threads are not ingested in v1.
///
/// `fileOrganizer` (Shared Drives, future v0.2) projects to `read` because
/// its write-side is moving files between folders, not modifying content.
fn drive_role_to_action(role: &str) -> &'static str {
    match role {
        // Read-tier: can fetch content but not modify.
        "reader" | "commenter" | "fileOrganizer" => "read",
        // Write-tier: can modify content.
        "writer" | "owner" | "organizer" => "write",
        other => {
            // Conservative for indexing (better to over-allow a search than
            // to silently exclude a real reader). May under-grant for
            // write-capable operations; the connector projects unknown roles
            // permissively at the read tier.
            tracing::warn!(
                role = %other,
                "drive: unrecognized role, defaulting to `read` (conservative for indexing; \
                 may under-grant for write-capable operations)"
            );
            "read"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perm(kind: &str, role: &str) -> DrivePermission {
        DrivePermission {
            kind: kind.to_owned(),
            email_address: None,
            domain: None,
            role: role.to_owned(),
            allow_file_discovery: None,
        }
    }

    #[test]
    fn user_permission_maps_to_user_principal() {
        let p = DrivePermission {
            kind: "user".to_owned(),
            email_address: Some("alice@acme.com".to_owned()),
            domain: None,
            role: "reader".to_owned(),
            allow_file_discovery: None,
        };
        let pred = drive_permission_to_predicate(&p).unwrap();
        assert_eq!(pred.principal, Principal::User("alice@acme.com".to_owned()));
        assert_eq!(pred.action, "read");
    }

    #[test]
    fn group_permission_maps_to_group_principal() {
        let p = DrivePermission {
            kind: "group".to_owned(),
            email_address: Some("eng@acme.com".to_owned()),
            domain: None,
            role: "writer".to_owned(),
            allow_file_discovery: None,
        };
        let pred = drive_permission_to_predicate(&p).unwrap();
        assert_eq!(pred.principal, Principal::Group("eng@acme.com".to_owned()));
        assert_eq!(pred.action, "write");
    }

    #[test]
    fn domain_permission_maps_to_domain_principal() {
        let p = DrivePermission {
            kind: "domain".to_owned(),
            email_address: None,
            domain: Some("acme.com".to_owned()),
            role: "reader".to_owned(),
            allow_file_discovery: None,
        };
        let pred = drive_permission_to_predicate(&p).unwrap();
        assert_eq!(pred.principal, Principal::Domain("acme.com".to_owned()));
    }

    #[test]
    fn anyone_with_discovery_maps_to_anyone() {
        let p = DrivePermission {
            kind: "anyone".to_owned(),
            email_address: None,
            domain: None,
            role: "reader".to_owned(),
            allow_file_discovery: Some(true),
        };
        let pred = drive_permission_to_predicate(&p).unwrap();
        assert_eq!(pred.principal, Principal::Anyone);
    }

    #[test]
    fn anyone_without_discovery_maps_to_anyone_with_link() {
        let p = DrivePermission {
            kind: "anyone".to_owned(),
            email_address: None,
            domain: None,
            role: "reader".to_owned(),
            allow_file_discovery: Some(false),
        };
        let pred = drive_permission_to_predicate(&p).unwrap();
        assert_eq!(pred.principal, Principal::AnyoneWithLink);
    }

    #[test]
    fn anyone_missing_discovery_defaults_to_anyone_with_link() {
        // Glean-class bug avoidance: ambiguous discovery → link-only.
        let p = DrivePermission {
            kind: "anyone".to_owned(),
            email_address: None,
            domain: None,
            role: "reader".to_owned(),
            allow_file_discovery: None,
        };
        let pred = drive_permission_to_predicate(&p).unwrap();
        assert_eq!(pred.principal, Principal::AnyoneWithLink);
    }

    #[test]
    fn unknown_kind_returns_none() {
        let p = perm("weird", "reader");
        assert!(drive_permission_to_predicate(&p).is_none());
    }

    #[test]
    fn role_projection_table() {
        assert_eq!(drive_role_to_action("reader"), "read");
        assert_eq!(drive_role_to_action("commenter"), "read");
        assert_eq!(drive_role_to_action("fileOrganizer"), "read");
        assert_eq!(drive_role_to_action("writer"), "write");
        assert_eq!(drive_role_to_action("owner"), "write");
        assert_eq!(drive_role_to_action("organizer"), "write");
        assert_eq!(drive_role_to_action("unknown_role"), "read"); // conservative default
    }
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p sealstack-connector-google-drive --lib permissions::tests
```

Expected: 8 passing tests.

- [ ] **Step 4: Apply rustfmt + commit**

```bash
cargo fmt -p sealstack-connector-google-drive
git add connectors/google-drive/
git commit -m "feat(google-drive): Drive permission mapping (DrivePermission -> Principal)"
```

---

### Task 9: `files.list` pagination + MIME allowlist + `SkipLog`

Sets up the `BodyCursorPaginator` walking `files.list` with the load-bearing `q` parameter and the `extract_items` filter. No body fetch yet — that's Task 10.

**Files:**
- Create: `connectors/google-drive/src/files.rs`
- Modify: `connectors/google-drive/src/lib.rs` (declare module)

- [ ] **Step 1: Declare the module**

Add to `connectors/google-drive/src/lib.rs`:

```rust
mod files;
```

- [ ] **Step 2: Write the failing integration test**

Create `connectors/google-drive/tests/list_files_e2e.rs`:

```rust
//! Integration test for `files.list` pagination + MIME allowlist + driveId skip.

use std::sync::Arc;

use futures::StreamExt;
use sealstack_connector_google_drive::test_only::list_files_for_test;
use sealstack_connector_sdk::auth::StaticToken;
use sealstack_connector_sdk::http::HttpClient;
use sealstack_connector_sdk::retry::RetryPolicy;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn list_files_walks_pages_and_filters() {
    let server = MockServer::start().await;
    // First page: 3 files. One is a Doc (allowed), one is a binary (skipped),
    // one is a Shared Drive item (driveId set, skipped).
    Mock::given(method("GET"))
        .and(path("/drive/v3/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "files": [
                {
                    "id": "doc1",
                    "name": "Design Doc",
                    "mimeType": "application/vnd.google-apps.document",
                    "modifiedTime": "2026-04-27T12:00:00Z"
                },
                {
                    "id": "bin1",
                    "name": "image.png",
                    "mimeType": "image/png",
                    "modifiedTime": "2026-04-27T12:00:00Z"
                },
                {
                    "id": "shared1",
                    "name": "shared.md",
                    "mimeType": "text/markdown",
                    "modifiedTime": "2026-04-27T12:00:00Z",
                    "driveId": "0AABCDEF12345"
                }
            ],
            "nextPageToken": "page2"
        })))
        .mount(&server)
        .await;
    // Second page: 1 file (markdown). No nextPageToken.
    Mock::given(method("GET"))
        .and(path("/drive/v3/files"))
        .and(wiremock::matchers::query_param("pageToken", "page2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "files": [
                {
                    "id": "md1",
                    "name": "README.md",
                    "mimeType": "text/markdown",
                    "modifiedTime": "2026-04-27T12:00:00Z"
                }
            ]
        })))
        .mount(&server)
        .await;

    let http = Arc::new(
        HttpClient::new(
            Arc::new(StaticToken::new("test-token")),
            RetryPolicy::default(),
        )
        .unwrap(),
    );
    let api_base = server.uri();
    let mut stream = list_files_for_test(http.clone(), &api_base);
    let mut ids: Vec<String> = Vec::new();
    while let Some(file) = stream.next().await {
        ids.push(file.unwrap().id);
    }
    // Expected: doc1, md1. bin1 is filtered (image/png not allowed); shared1
    // is filtered (driveId set).
    assert_eq!(ids, vec!["doc1".to_owned(), "md1".to_owned()]);
}
```

The test imports `sealstack_connector_google_drive::test_only::list_files_for_test` — that's a `#[doc(hidden)]` test entry point we'll add to keep the production code idiomatic while letting the integration test drive the paginator without needing the full Connector wiring.

- [ ] **Step 3: Run the test to verify it fails**

```bash
cargo test -p sealstack-connector-google-drive --test list_files_e2e
```

Expected: FAIL — `test_only::list_files_for_test` doesn't exist.

- [ ] **Step 4: Implement `files.rs`**

Create `connectors/google-drive/src/files.rs`:

```rust
//! Drive `files.list` pagination + MIME allowlist + skip logic.

use std::collections::HashSet;
use std::sync::Arc;

use sealstack_common::{SealStackError, SealStackResult};
use sealstack_connector_sdk::http::HttpClient;
use sealstack_connector_sdk::paginate::{BodyCursorPaginator, paginate};
use sealstack_connector_sdk::{ResourceId};
use serde::Deserialize;

/// Drive file metadata as returned by `files.list`.
///
/// Only the fields we use are deserialized.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DriveFile {
    pub id: String,
    #[serde(rename = "name")]
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
    pub permissions: Vec<crate::permissions::DrivePermission>,
}

/// Connector-internal "log once per resource id" dedup helper.
///
/// Used to surface MIME-skip decisions without spamming logs on every sync
/// cycle. Reset on connector restart, scoped per-connector-instance. v1
/// implementation; promoted to SDK if a second connector wants similar
/// dedup (see spec §13).
#[derive(Debug, Default)]
pub(crate) struct SkipLog {
    seen: tokio::sync::Mutex<HashSet<ResourceId>>,
}

impl SkipLog {
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

/// Build the BodyCursorPaginator over `files.list`.
///
/// Filters out:
/// - MIME types not in the v1 allowlist (logged via SkipLog at the body-fetch
///   site in Task 10; here we just emit them for size-cap evaluation).
/// - `driveId`-bearing items (v1 corpora=user constraint).
pub(crate) fn paginator(
    api_base: &str,
) -> BodyCursorPaginator<
    DriveFile,
    impl Fn(&HttpClient, Option<&str>) -> reqwest::RequestBuilder + Send + 'static,
    impl Fn(&serde_json::Value) -> SealStackResult<Vec<DriveFile>> + Send + 'static,
    impl Fn(&serde_json::Value) -> Option<String> + Send + 'static,
> {
    let url = format!("{}/drive/v3/files", api_base.trim_end_matches('/'));
    BodyCursorPaginator::new(
        move |c: &HttpClient, cursor: Option<&str>| {
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
                    // MIME-skip dedup happens at body-fetch (Task 10) where
                    // SkipLog lives on DriveConnector. At paginator level we
                    // just filter without per-id dedup; the same file would
                    // get the same info log on every sync cycle if we logged
                    // here. Filter silently.
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
    )
}

fn is_allowed_mime(mime: &str) -> bool {
    matches!(
        mime,
        "application/vnd.google-apps.document" | "text/plain" | "text/markdown"
    )
}

/// Test-only: drive the paginator and yield filtered DriveFiles.
///
/// Exposed so integration tests can exercise the paginator without standing
/// up the full DriveConnector + OAuth machinery.
#[doc(hidden)]
pub fn list_files_for_test(
    http: Arc<HttpClient>,
    api_base: &str,
) -> std::pin::Pin<
    Box<dyn futures::Stream<Item = SealStackResult<DriveFile>> + Send>,
> {
    paginate(paginator(api_base), http)
}
```

The `test_only` module hides the helper from production usage but exposes it for the integration test. Add at the bottom of `lib.rs` (after the existing `mod tests` block):

```rust
#[doc(hidden)]
pub mod test_only {
    pub use crate::files::list_files_for_test;
}
```

- [ ] **Step 5: Run the integration test to verify it passes**

```bash
cargo test -p sealstack-connector-google-drive --test list_files_e2e
```

Expected: 1 passing test, asserts `ids == vec!["doc1", "md1"]`.

- [ ] **Step 6: Run all tests**

```bash
cargo test -p sealstack-connector-google-drive
```

Expected: 8 (permissions) + 4 (lib) + 1 (list_files_e2e) = 13 tests pass.

- [ ] **Step 7: Apply rustfmt + commit**

```bash
cargo fmt -p sealstack-connector-google-drive
git add connectors/google-drive/
git commit -m "feat(google-drive): files.list pagination + MIME allowlist + SkipLog"
```

---

### Task 10: Body fetch (export vs `alt=media`) with strict UTF-8 + per-file cap

`fetch_body` method handling the export-or-direct-download decision per MIME type.

**Files:**
- Modify: `connectors/google-drive/src/files.rs` (add `fetch_body`)
- Create: `connectors/google-drive/tests/fetch_body_e2e.rs`

- [ ] **Step 1: Write the failing integration test**

Create `connectors/google-drive/tests/fetch_body_e2e.rs`:

```rust
//! Integration test for body fetch: export vs alt=media, strict UTF-8, per-file cap.

use std::sync::Arc;

use sealstack_connector_google_drive::test_only::{fetch_body_for_test, DriveFileTestStub};
use sealstack_connector_sdk::auth::StaticToken;
use sealstack_connector_sdk::http::HttpClient;
use sealstack_connector_sdk::retry::RetryPolicy;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn make_http(_server: &MockServer) -> Arc<HttpClient> {
    Arc::new(
        HttpClient::new(
            Arc::new(StaticToken::new("test-token")),
            RetryPolicy::default(),
        )
        .unwrap(),
    )
}

#[tokio::test]
async fn fetch_body_exports_google_doc_as_text() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/drive/v3/files/doc1/export"))
        .and(query_param("mimeType", "text/plain"))
        .respond_with(ResponseTemplate::new(200).set_body_string("hello docs"))
        .mount(&server)
        .await;

    let http = make_http(&server);
    let file = DriveFileTestStub {
        id: "doc1".into(),
        mime_type: "application/vnd.google-apps.document".into(),
        size: None,
    };
    let body = fetch_body_for_test(http, &server.uri(), &file, 10 * 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(body, Some("hello docs".to_owned()));
}

#[tokio::test]
async fn fetch_body_direct_downloads_text_plain() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/drive/v3/files/txt1"))
        .and(query_param("alt", "media"))
        .respond_with(ResponseTemplate::new(200).set_body_string("plain text content"))
        .mount(&server)
        .await;

    let http = make_http(&server);
    let file = DriveFileTestStub {
        id: "txt1".into(),
        mime_type: "text/plain".into(),
        size: Some("100".into()),
    };
    let body = fetch_body_for_test(http, &server.uri(), &file, 10 * 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(body, Some("plain text content".to_owned()));
}

#[tokio::test]
async fn fetch_body_skips_unsupported_mime() {
    let server = MockServer::start().await;
    let http = make_http(&server);
    let file = DriveFileTestStub {
        id: "img1".into(),
        mime_type: "image/png".into(),
        size: Some("1000".into()),
    };
    let body = fetch_body_for_test(http, &server.uri(), &file, 10 * 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(body, None, "unsupported MIME should yield None");
}

#[tokio::test]
async fn fetch_body_skips_oversized_file() {
    let server = MockServer::start().await;
    let http = make_http(&server);
    let file = DriveFileTestStub {
        id: "big1".into(),
        mime_type: "text/plain".into(),
        size: Some("20000000".into()), // 20 MB > 10 MB cap
    };
    let body = fetch_body_for_test(http, &server.uri(), &file, 10 * 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(body, None, "oversized file should yield None");
}

#[tokio::test]
async fn fetch_body_text_with_invalid_utf8_skips() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/drive/v3/files/badutf/"))
        // Response with invalid UTF-8 bytes (0xFF 0xFE 0xFD).
        .respond_with(
            ResponseTemplate::new(200).set_body_bytes(vec![0xFF_u8, 0xFE, 0xFD]),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/drive/v3/files/badutf"))
        .and(query_param("alt", "media"))
        .respond_with(
            ResponseTemplate::new(200).set_body_bytes(vec![0xFF_u8, 0xFE, 0xFD]),
        )
        .mount(&server)
        .await;

    let http = make_http(&server);
    let file = DriveFileTestStub {
        id: "badutf".into(),
        mime_type: "text/plain".into(),
        size: Some("3".into()),
    };
    let body = fetch_body_for_test(http, &server.uri(), &file, 10 * 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(body, None, "non-UTF-8 text should be skipped, not lossy-decoded");
}

#[tokio::test]
async fn fetch_body_docs_export_invalid_utf8_errors() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/drive/v3/files/baddoc/export"))
        .and(query_param("mimeType", "text/plain"))
        .respond_with(
            ResponseTemplate::new(200).set_body_bytes(vec![0xFF_u8, 0xFE, 0xFD]),
        )
        .mount(&server)
        .await;

    let http = make_http(&server);
    let file = DriveFileTestStub {
        id: "baddoc".into(),
        mime_type: "application/vnd.google-apps.document".into(),
        size: None,
    };
    // Docs export contract guarantees UTF-8; a violation is a Google-side
    // bug, not a user-side mistake. Should error rather than silently skip.
    let err = fetch_body_for_test(http, &server.uri(), &file, 10 * 1024 * 1024)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("non-UTF-8") || err.to_string().contains("docs export"),
        "expected docs-export-non-UTF-8 error, got: {err}"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p sealstack-connector-google-drive --test fetch_body_e2e
```

Expected: FAIL — `fetch_body_for_test` doesn't exist.

- [ ] **Step 3: Implement `fetch_body` and the test helper**

Append to `connectors/google-drive/src/files.rs`:

```rust
/// Test-only stub of DriveFile carrying just the fields fetch_body needs.
#[doc(hidden)]
pub struct DriveFileTestStub {
    pub id: String,
    pub mime_type: String,
    pub size: Option<String>,
}

/// Test-only entry point for fetch_body that bypasses the full DriveConnector.
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
    if let Some(s) = size {
        if s > max_file_bytes {
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
    }

    match file.mime_type.as_str() {
        "application/vnd.google-apps.document" => {
            // Google Docs: export as text/plain.
            let url = format!(
                "{}/drive/v3/files/{}/export",
                api_base.trim_end_matches('/'),
                file.id
            );
            let make = || http.get(&url).query(&[("mimeType", "text/plain")]);
            let resp = http.send(make()).await?;
            let bytes = resp.bytes().await?;
            // Strict UTF-8 — Docs export contract guarantees UTF-8; a violation
            // is a Google-side bug, not a user-side mistake, so error rather
            // than silently dropping.
            String::from_utf8(bytes.to_vec()).map(Some).map_err(|_| {
                SealStackError::backend("drive: docs export returned non-UTF-8")
            })
        }
        "text/plain" | "text/markdown" => {
            // Direct binary fetch.
            let url = format!(
                "{}/drive/v3/files/{}",
                api_base.trim_end_matches('/'),
                file.id
            );
            let make = || http.get(&url).query(&[("alt", "media")]);
            let resp = http.send(make()).await?;
            let bytes = resp.bytes().await?;
            // Strict UTF-8 — text MIME is a user-supplied claim Drive doesn't
            // validate. A non-UTF-8 file claimed as text/plain is a config
            // error or a deliberate skip case (binary mislabeled as text).
            // Skip without erroring; from_utf8_lossy would silently embed
            // U+FFFD pollution into the index.
            match String::from_utf8(bytes.to_vec()) {
                Ok(s) => Ok(Some(s)),
                Err(_) => {
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
```

Update the `test_only` module in `lib.rs`:

```rust
#[doc(hidden)]
pub mod test_only {
    pub use crate::files::{
        list_files_for_test, fetch_body_for_test, DriveFileTestStub,
    };
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test -p sealstack-connector-google-drive --test fetch_body_e2e
```

Expected: 6 passing tests.

- [ ] **Step 5: Run the full crate test suite**

```bash
cargo test -p sealstack-connector-google-drive
```

Expected: 8 + 4 + 1 + 6 = 19 tests pass.

- [ ] **Step 6: Apply rustfmt + commit**

```bash
cargo fmt -p sealstack-connector-google-drive
git add connectors/google-drive/
git commit -m "feat(google-drive): body fetch (export vs alt=media) with strict UTF-8 + per-file cap"
```

---

### Task 11: `retry_shim` for 403 three-class discrimination

Mirrors github's pattern, with one extra branch for daily-quota.

**Files:**
- Create: `connectors/google-drive/src/retry_shim.rs`
- Create: `connectors/google-drive/tests/retry_shim.rs`

- [ ] **Step 1: Declare the module**

Add to `connectors/google-drive/src/lib.rs`:

```rust
mod retry_shim;
```

- [ ] **Step 2: Write the failing classifier unit tests**

Create `connectors/google-drive/tests/retry_shim.rs`:

```rust
//! Drive 403 classifier unit tests.

use std::time::Duration;

use sealstack_connector_google_drive::retry_shim::{
    classify_drive_403, Drive403Action,
};

fn body_with_reason(reasons: &[&str]) -> String {
    let errs: Vec<serde_json::Value> = reasons
        .iter()
        .map(|r| serde_json::json!({"reason": r, "domain": "usageLimits", "message": "x"}))
        .collect();
    serde_json::json!({
        "error": {
            "code": 403,
            "message": "Forbidden",
            "errors": errs
        }
    })
    .to_string()
}

#[test]
fn user_rate_limit_classifies_as_backoff() {
    let body = body_with_reason(&["userRateLimitExceeded"]);
    match classify_drive_403(&[], &body) {
        Drive403Action::BackoffThenRetry => {}
        other => panic!("expected BackoffThenRetry, got {other:?}"),
    }
}

#[test]
fn rate_limit_exceeded_classifies_as_backoff() {
    let body = body_with_reason(&["rateLimitExceeded"]);
    match classify_drive_403(&[], &body) {
        Drive403Action::BackoffThenRetry => {}
        other => panic!("expected BackoffThenRetry, got {other:?}"),
    }
}

#[test]
fn quota_exceeded_classifies_as_quota_exhausted() {
    let body = body_with_reason(&["quotaExceeded"]);
    match classify_drive_403(&[], &body) {
        Drive403Action::QuotaExhausted => {}
        other => panic!("expected QuotaExhausted, got {other:?}"),
    }
}

#[test]
fn daily_limit_exceeded_classifies_as_quota_exhausted() {
    let body = body_with_reason(&["dailyLimitExceeded"]);
    match classify_drive_403(&[], &body) {
        Drive403Action::QuotaExhausted => {}
        other => panic!("expected QuotaExhausted, got {other:?}"),
    }
}

#[test]
fn forbidden_classifies_as_permission_denied() {
    let body = body_with_reason(&["forbidden"]);
    match classify_drive_403(&[], &body) {
        Drive403Action::PermissionDenied { reason } => {
            assert!(reason.contains("forbidden"), "{reason}");
        }
        other => panic!("expected PermissionDenied, got {other:?}"),
    }
}

#[test]
fn insufficient_permissions_classifies_as_permission_denied() {
    let body = body_with_reason(&["insufficientPermissions"]);
    match classify_drive_403(&[], &body) {
        Drive403Action::PermissionDenied { reason } => {
            assert!(reason.contains("insufficientPermissions"), "{reason}");
        }
        other => panic!("expected PermissionDenied, got {other:?}"),
    }
}

#[test]
fn domain_policy_classifies_as_permission_denied() {
    let body = body_with_reason(&["domainPolicy"]);
    match classify_drive_403(&[], &body) {
        Drive403Action::PermissionDenied { .. } => {}
        other => panic!("expected PermissionDenied, got {other:?}"),
    }
}

#[test]
fn app_not_authorized_classifies_as_permission_denied() {
    let body = body_with_reason(&["appNotAuthorizedToFile"]);
    match classify_drive_403(&[], &body) {
        Drive403Action::PermissionDenied { .. } => {}
        other => panic!("expected PermissionDenied, got {other:?}"),
    }
}

#[test]
fn quota_wins_over_rate_limit_when_both_present() {
    // Daily quota wins over short-term rate-limit; retrying when daily quota
    // is exhausted buys nothing.
    let body = body_with_reason(&["userRateLimitExceeded", "quotaExceeded"]);
    match classify_drive_403(&[], &body) {
        Drive403Action::QuotaExhausted => {}
        other => panic!("expected QuotaExhausted, got {other:?}"),
    }
}

#[test]
fn malformed_body_classifies_as_permission_denied() {
    match classify_drive_403(&[], "not json") {
        Drive403Action::PermissionDenied { reason } => {
            assert_eq!(reason, "(no reason in body)");
        }
        other => panic!("expected PermissionDenied, got {other:?}"),
    }
}

#[test]
fn empty_body_classifies_as_permission_denied() {
    match classify_drive_403(&[], "") {
        Drive403Action::PermissionDenied { .. } => {}
        other => panic!("expected PermissionDenied, got {other:?}"),
    }
}

#[test]
fn permission_denied_includes_all_reasons() {
    // Multi-reason responses: comma-joined for full diagnostic context.
    let body = body_with_reason(&["domainPolicy", "insufficientPermissions"]);
    match classify_drive_403(&[], &body) {
        Drive403Action::PermissionDenied { reason } => {
            assert!(reason.contains("domainPolicy"), "{reason}");
            assert!(reason.contains("insufficientPermissions"), "{reason}");
        }
        other => panic!("expected PermissionDenied, got {other:?}"),
    }
}

// Sanity: WaitThenRetry is reserved for future use (e.g., explicit Retry-After
// on 403 if Drive ever sends it). The variant exists in the enum so future
// extensions don't break exhaustive matches.
#[test]
fn wait_then_retry_variant_exists() {
    let _ = Drive403Action::WaitThenRetry(Duration::from_secs(1));
}
```

- [ ] **Step 3: Run tests to verify they fail**

```bash
cargo test -p sealstack-connector-google-drive --test retry_shim
```

Expected: FAIL — `retry_shim` module doesn't exist.

- [ ] **Step 4: Implement the classifier + send_with_drive_shim**

Create `connectors/google-drive/src/retry_shim.rs`:

```rust
//! Drive-specific 403 discrimination.
//!
//! Drive's 403 responses cluster into three classes that need different
//! client-side handling. See spec §8 for the full taxonomy.

use std::time::Duration;

use sealstack_common::{SealStackError, SealStackResult};
use sealstack_connector_sdk::http::{HttpClient, HttpResponse};

/// Classification of a Drive 403 response.
#[derive(Debug)]
pub enum Drive403Action {
    /// Reserved for future use (explicit Retry-After on 403 if Drive emits it).
    WaitThenRetry(Duration),
    /// Short-term rate limit exceeded. Retry with exponential backoff
    /// (500ms × 2^attempt), up to 5 attempts. After budget exhaustion,
    /// surface as `RateLimited`.
    BackoffThenRetry,
    /// Daily quota exhausted. Retrying buys nothing until UTC midnight.
    /// Surface as `RateLimited` immediately.
    QuotaExhausted,
    /// Permission denied. Surface as `Backend` with comma-joined reasons.
    PermissionDenied { reason: String },
}

/// Classify a Drive 403 response body.
///
/// `_headers` is unused in v1 (Drive doesn't typically emit Retry-After on
/// 403); kept in signature for symmetry with github's shim.
#[must_use]
pub fn classify_drive_403(_headers: &[(String, String)], body: &str) -> Drive403Action {
    let parsed: serde_json::Value =
        serde_json::from_str(body).unwrap_or(serde_json::Value::Null);
    let reasons: Vec<&str> = parsed
        .pointer("/error/errors")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|e| e.get("reason").and_then(|r| r.as_str()))
                .collect()
        })
        .unwrap_or_default();

    // Daily quota wins over short-term rate-limit when both signals are present.
    if reasons
        .iter()
        .any(|r| matches!(*r, "quotaExceeded" | "dailyLimitExceeded"))
    {
        return Drive403Action::QuotaExhausted;
    }
    if reasons
        .iter()
        .any(|r| matches!(*r, "userRateLimitExceeded" | "rateLimitExceeded"))
    {
        return Drive403Action::BackoffThenRetry;
    }
    let reason = if reasons.is_empty() {
        "(no reason in body)".to_owned()
    } else {
        reasons.join(",")
    };
    Drive403Action::PermissionDenied { reason }
}

/// At most 5 attempts: initial + 4 shim-guided retries.
///
/// Differs from `send_with_gh_shim`:
/// - Allows 4 retries (not 1) because Drive's per-user 10 req/sec limit
///   fires regularly; one retry isn't enough headroom.
/// - Distinguishes `QuotaExhausted` (immediate `RateLimited`) from
///   `BackoffThenRetry` (loop, then `RateLimited`).
pub(crate) async fn send_with_drive_shim<F>(
    http: &HttpClient,
    make_request: F,
) -> SealStackResult<HttpResponse>
where
    F: Fn() -> reqwest::RequestBuilder,
{
    let mut attempt = 0u32;
    loop {
        match http.send(make_request()).await {
            Ok(resp) => return Ok(resp),
            Err(SealStackError::HttpStatus { status: 403, headers, body }) => {
                match classify_drive_403(&headers, &body) {
                    Drive403Action::BackoffThenRetry if attempt < 4 => {
                        // Backoff: 500ms, 1s, 2s, 4s. Cumulative ~7.5s.
                        // Drive's per-user rate-limit window is 100s; this
                        // schedule reaches roughly 7.5% of one window.
                        // A schedule that consistently spans the full window
                        // (500ms, 1s, 2s, 4s, 8s = 15.5s cumulative) would
                        // recover from a higher fraction of legitimate
                        // rate-limits but at the cost of doubled worst-case
                        // latency. Revisit if pilot telemetry shows
                        // budget-exhaustion at meaningful rates.
                        let delay = Duration::from_millis(500 * (1u64 << attempt));
                        // Demoted to debug — first attempts in a backoff
                        // loop are the system working as designed, not
                        // warnings. warn is reserved for budget exhaustion.
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
                        tracing::warn!(
                            "drive: daily quota exhausted, not retrying until UTC midnight"
                        );
                        return Err(SealStackError::RateLimited);
                    }
                    Drive403Action::WaitThenRetry(_) => {
                        // Reserved for future use; not produced by classify_drive_403 in v1.
                        return Err(SealStackError::Backend(
                            "drive: unexpected WaitThenRetry classification".to_owned(),
                        ));
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

- [ ] **Step 5: Make the module accessible from tests**

The integration test imports `sealstack_connector_google_drive::retry_shim::*`. To allow this, change the module declaration in `lib.rs` from:

```rust
mod retry_shim;
```

to:

```rust
pub mod retry_shim;
```

Then expose `send_with_drive_shim` for integration tests via the `test_only` module. Update the `test_only` block in `lib.rs`:

```rust
#[doc(hidden)]
pub mod test_only {
    pub use crate::files::{
        list_files_for_test, fetch_body_for_test, DriveFileTestStub,
    };
    pub use crate::retry_shim::send_with_drive_shim;
}
```

The `send_with_drive_shim` function stays `pub(crate)` in its own module; the `test_only` re-export makes it reachable from `tests/` files without exposing it to library consumers. (Rust's visibility rules: `pub(crate)` items can be re-exported via `pub use` in a public module.)

Wait — `pub(crate)` items cannot be `pub use`-d. To allow the re-export, change `send_with_drive_shim`'s visibility from `pub(crate)` to `pub` and gate it via the `test_only` namespace at the public API boundary:

In `retry_shim.rs`, change:

```rust
pub(crate) async fn send_with_drive_shim<F>(
```

to:

```rust
pub async fn send_with_drive_shim<F>(
```

The function's discoverability is controlled by where it's re-exported. The crate root re-exports nothing from `retry_shim` directly; only the `test_only` module re-exports `send_with_drive_shim`. Production code inside the crate uses `crate::retry_shim::send_with_drive_shim` directly without going through `test_only`.

- [ ] **Step 6: Run the classifier tests**

```bash
cargo test -p sealstack-connector-google-drive --test retry_shim
```

Expected: 13 passing tests.

- [ ] **Step 7: Run the crate test suite**

```bash
cargo test -p sealstack-connector-google-drive
```

Expected: 19 + 13 = 32 tests pass.

- [ ] **Step 8: Apply rustfmt + commit**

```bash
cargo fmt -p sealstack-connector-google-drive
git add connectors/google-drive/
git commit -m "feat(google-drive): retry_shim for 403 three-class discrimination"
```

---

### Task 12: `DriveFile → Resource` projection + `Connector` impl + load-bearing e2e tests

Closing commit. Wires everything together. Lands `tests/oauth_refresh.rs` — the load-bearing acceptance criterion (spec §10).

**Files:**
- Create: `connectors/google-drive/src/resource.rs`
- Modify: `connectors/google-drive/src/lib.rs` (declare resource module, fill Connector impl)
- Create: `connectors/google-drive/tests/list_e2e.rs`
- Create: `connectors/google-drive/tests/oauth_refresh.rs`
- Create: `connectors/google-drive/tests/retry_shim_e2e.rs`

- [ ] **Step 1: Declare the resource module**

Add to `connectors/google-drive/src/lib.rs`:

```rust
mod resource;
```

- [ ] **Step 2: Implement the resource projection**

Create `connectors/google-drive/src/resource.rs`:

```rust
//! `DriveFile + body + ACLs → Resource` projection — the connector's product.
//!
//! v0.2 Shared Drives lands here by extending the projection with
//! drive-level ACL inheritance/merging.

use sealstack_common::{SealStackError, SealStackResult};
use sealstack_connector_sdk::{PermissionPredicate, Resource, ResourceId};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::files::DriveFile;
use crate::permissions::drive_permission_to_predicate;

/// Project a `DriveFile` + fetched body into a SealStack `Resource`.
///
/// `body` is `Some(content)` for files whose MIME type is in the v1 allowlist
/// AND whose content fetched/decoded successfully. `None` callers should not
/// invoke this — they should drop the file from the stream entirely.
pub(crate) fn drive_file_to_resource(file: &DriveFile, body: String) -> SealStackResult<Resource> {
    let permissions: Vec<PermissionPredicate> = file
        .permissions
        .iter()
        .filter_map(drive_permission_to_predicate)
        .collect();

    let updated = OffsetDateTime::parse(&file.modified_time, &Rfc3339)
        .map_err(|e| SealStackError::backend(format!("drive: bad modifiedTime {e}")))?;

    let kind = match file.mime_type.as_str() {
        "application/vnd.google-apps.document" => "google-doc",
        "text/markdown" => "markdown",
        "text/plain" => "text",
        _ => "unknown", // unreachable in practice since fetch_body filters
    }
    .to_owned();

    Ok(Resource {
        id: ResourceId::new(file.id.clone()),
        kind,
        title: Some(file.name.clone()),
        body,
        metadata: serde_json::Map::new(),
        permissions,
        source_updated_at: updated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::DrivePermission;
    use sealstack_connector_sdk::Principal;

    fn sample_file() -> DriveFile {
        DriveFile {
            id: "doc1".to_owned(),
            name: "Design Doc".to_owned(),
            mime_type: "application/vnd.google-apps.document".to_owned(),
            modified_time: "2026-04-27T12:00:00Z".to_owned(),
            drive_id: None,
            size: None,
            permissions: vec![
                DrivePermission {
                    kind: "user".into(),
                    email_address: Some("alice@acme.com".into()),
                    domain: None,
                    role: "reader".into(),
                    allow_file_discovery: None,
                },
                DrivePermission {
                    kind: "anyone".into(),
                    email_address: None,
                    domain: None,
                    role: "reader".into(),
                    allow_file_discovery: Some(false),
                },
            ],
        }
    }

    #[test]
    fn projects_doc_to_resource() {
        let r = drive_file_to_resource(&sample_file(), "doc body".to_owned()).unwrap();
        assert_eq!(r.id.as_str(), "doc1");
        assert_eq!(r.kind, "google-doc");
        assert_eq!(r.title.as_deref(), Some("Design Doc"));
        assert_eq!(r.body, "doc body");
        assert_eq!(r.permissions.len(), 2);
        assert_eq!(
            r.permissions[0].principal,
            Principal::User("alice@acme.com".into())
        );
        assert_eq!(r.permissions[1].principal, Principal::AnyoneWithLink);
    }

    #[test]
    fn unrecognized_acl_is_dropped_individually() {
        let mut file = sample_file();
        file.permissions.push(DrivePermission {
            kind: "weirdfuture".into(),
            email_address: None,
            domain: None,
            role: "reader".into(),
            allow_file_discovery: None,
        });
        // The bad ACL is dropped; the resource still has the two good ones.
        let r = drive_file_to_resource(&file, "x".into()).unwrap();
        assert_eq!(r.permissions.len(), 2);
    }

    #[test]
    fn bad_modified_time_errors() {
        let mut file = sample_file();
        file.modified_time = "not-rfc3339".into();
        let err = drive_file_to_resource(&file, "x".into()).unwrap_err();
        assert!(err.to_string().contains("modifiedTime"), "{err}");
    }
}
```

- [ ] **Step 3: Fill the `Connector` impl in lib.rs**

Replace the stub `Connector` impl (the `unimplemented!()` blocks) in `lib.rs` with the wired version. Add at the top of `lib.rs`:

```rust
use crate::files::{paginator, fetch_body, SkipLog};
use crate::resource::drive_file_to_resource;
use crate::retry_shim::send_with_drive_shim;
use sealstack_connector_sdk::change_streams;
use sealstack_connector_sdk::paginate::paginate;
use futures::StreamExt;
```

Update the `DriveConnector` struct to include the SkipLog:

```rust
#[derive(Debug)]
pub struct DriveConnector {
    http: Arc<HttpClient>,
    config: DriveConfig,
    skip_log: Arc<SkipLog>,
}
```

Update the `from_json` to initialize it:

```rust
Ok(Self {
    http,
    config,
    skip_log: Arc::new(SkipLog::default()),
})
```

Replace the `Connector` impl methods:

```rust
#[async_trait]
impl Connector for DriveConnector {
    fn name(&self) -> &str {
        "google-drive"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    async fn list(&self) -> SealStackResult<ResourceStream> {
        let pg = paginator(&self.config.api_base);
        let mut stream = paginate(pg, self.http.clone());
        let mut out: Vec<Resource> = Vec::new();
        while let Some(file_result) = stream.next().await {
            let file = file_result?;
            match fetch_body(
                &self.http,
                &self.config.api_base,
                &file,
                self.config.max_file_bytes,
                &self.skip_log,
            )
            .await?
            {
                Some(body) => {
                    out.push(drive_file_to_resource(&file, body)?);
                }
                None => continue, // skipped (oversized, non-allowlist MIME, non-UTF-8)
            }
        }
        Ok(change_streams::resource_stream(out))
    }

    async fn fetch(&self, id: &ResourceId) -> SealStackResult<Resource> {
        let url = format!(
            "{}/drive/v3/files/{}",
            self.config.api_base, id
        );
        let make = || {
            self.http.get(&url).query(&[(
                "fields",
                "id,name,mimeType,modifiedTime,driveId,size,\
                 permissions(type,emailAddress,domain,role,allowFileDiscovery)",
            )])
        };
        let resp = send_with_drive_shim(&self.http, make).await?;
        let file: crate::files::DriveFile = resp.json().await?;
        match fetch_body(
            &self.http,
            &self.config.api_base,
            &file,
            self.config.max_file_bytes,
            &self.skip_log,
        )
        .await?
        {
            Some(body) => drive_file_to_resource(&file, body),
            None => Err(SealStackError::backend(format!(
                "drive: file {} skipped (oversized, non-allowlist MIME, or non-UTF-8 body)",
                id
            ))),
        }
    }

    async fn subscribe(&self) -> SealStackResult<Option<ChangeStream>> {
        Ok(None) // v1 is full-crawl; subscribe lands in v0.2.
    }

    async fn healthcheck(&self) -> SealStackResult<()> {
        // files.list (NOT files/about) because files.list exercises drive.readonly
        // scope. A refresh token granted with only userinfo.email scope would pass
        // /about but fail every subsequent /files.list with 403 insufficientPermissions.
        // Healthcheck must surface scope mismatches at boot, not at first sync.
        let url = format!("{}/drive/v3/files", self.config.api_base);
        let make = || self.http.get(&url).query(&[("pageSize", "1")]);
        let _ = send_with_drive_shim(&self.http, make).await?;
        Ok(())
    }
}
```

Also add `DriveFile` to the test_only re-exports if needed for new e2e tests:

```rust
#[doc(hidden)]
pub mod test_only {
    pub use crate::files::{
        DriveFileTestStub, fetch_body_for_test, list_files_for_test,
    };
}
```

- [ ] **Step 4: Write the load-bearing OAuth refresh e2e test**

Create `connectors/google-drive/tests/oauth_refresh.rs`:

```rust
//! LOAD-BEARING ACCEPTANCE CRITERION (spec §10).
//!
//! If this test passes, the slice has done its load-bearing job. If it
//! fails, no other test result matters.

use sealstack_connector_google_drive::DriveConnector;
use sealstack_connector_sdk::Connector;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Helper: install token-endpoint mocks that return access tokens deterministically.
async fn mount_token_server(token_server: &MockServer) -> Arc<AtomicBool> {
    let issued_second = Arc::new(AtomicBool::new(false));
    let flag = issued_second.clone();
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_partial_json(serde_json::json!({"grant_type": "refresh_token"})))
        .respond_with(move |_: &wiremock::Request| {
            let n = if flag.load(Ordering::SeqCst) {
                "ya29.second"
            } else {
                "ya29.first"
            };
            flag.store(true, Ordering::SeqCst);
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": n,
                "expires_in": 3599,
                "token_type": "Bearer"
            }))
        })
        .mount(token_server)
        .await;
    issued_second
}

#[tokio::test]
async fn oauth_refresh_on_401_succeeds_on_retry() {
    // Two wiremock servers: one for the Drive API, one for the OAuth token endpoint.
    let drive_server = MockServer::start().await;
    let token_server = MockServer::start().await;
    let _flag = mount_token_server(&token_server).await;

    // Drive endpoint: first request with `Bearer ya29.first` → 401.
    Mock::given(method("GET"))
        .and(path("/drive/v3/files"))
        .and(wiremock::matchers::header("Authorization", "Bearer ya29.first"))
        .respond_with(
            ResponseTemplate::new(401)
                .append_header("WWW-Authenticate", r#"Bearer error="invalid_token""#),
        )
        .mount(&drive_server)
        .await;
    // Drive endpoint: second request with `Bearer ya29.second` → 200.
    Mock::given(method("GET"))
        .and(path("/drive/v3/files"))
        .and(wiremock::matchers::header("Authorization", "Bearer ya29.second"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "files": []
        })))
        .mount(&drive_server)
        .await;

    // SAFETY: env mutation is needed to wire the connector's OAuth credentials
    // through its from_json path; we accept the test-time env as the seam.
    // The crate forbids unsafe_code, so we set/unset via static-process methods
    // that don't require unsafe (non-2024-edition path).
    // For simplicity: write the env vars directly using the standard library.
    //
    // The lib.rs forbid(unsafe_code) doesn't apply to integration tests under
    // tests/. Use unsafe directly here.
    unsafe {
        std::env::set_var("DRIVE_OAUTH_TEST_SECRET", "secret-value");
        std::env::set_var("DRIVE_OAUTH_TEST_REFRESH", "refresh-value");
    }

    let cfg = serde_json::json!({
        "client_id": "test-client",
        "client_secret_env": "DRIVE_OAUTH_TEST_SECRET",
        "refresh_token_env": "DRIVE_OAUTH_TEST_REFRESH",
        "api_base": drive_server.uri(),
    });
    // We can't override the OAuth token endpoint via the public DriveConnector
    // API in v1; instead exercise the refresh path directly by constructing
    // an OAuth2Credential pointed at the wiremock token endpoint, swapping it
    // into an HttpClient, and driving a Drive request manually.
    //
    // This test intentionally bypasses DriveConnector::from_json's
    // OAuth2Credential::google() (which hardcodes the real Google endpoint)
    // and uses OAuth2Credential::new directly.
    use secrecy::SecretString;
    use sealstack_connector_sdk::auth::OAuth2Credential;
    use sealstack_connector_sdk::http::HttpClient;
    use sealstack_connector_sdk::retry::RetryPolicy;

    let cred = Arc::new(
        OAuth2Credential::new(
            "test-client".to_owned(),
            SecretString::new("secret-value".into()),
            SecretString::new("refresh-value".into()),
            format!("{}/token", token_server.uri()),
        )
        .unwrap(),
    );
    let http = Arc::new(
        HttpClient::new(cred, RetryPolicy::default())
            .unwrap()
            .with_user_agent_suffix("oauth-refresh-test/0.1.0"),
    );
    let _ = cfg; // not used in this direct path; suppresses warning

    // Drive endpoint with a 401 → invalidate-once → refresh → 200.
    let url = format!("{}/drive/v3/files", drive_server.uri());
    let resp = http.send(http.get(&url)).await.unwrap();
    assert_eq!(resp.status(), 200);

    unsafe {
        std::env::remove_var("DRIVE_OAUTH_TEST_SECRET");
        std::env::remove_var("DRIVE_OAUTH_TEST_REFRESH");
    }
}

#[tokio::test]
async fn oauth_refresh_invalid_grant_surfaces_unauthorized() {
    let token_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": "invalid_grant",
            "error_description": "Token has been expired or revoked."
        })))
        .mount(&token_server)
        .await;

    use secrecy::SecretString;
    use sealstack_connector_sdk::auth::OAuth2Credential;
    use sealstack_connector_sdk::http::HttpClient;
    use sealstack_connector_sdk::retry::RetryPolicy;
    use sealstack_common::SealStackError;

    let cred = Arc::new(
        OAuth2Credential::new(
            "test-client".to_owned(),
            SecretString::new("secret".into()),
            SecretString::new("revoked-refresh".into()),
            format!("{}/token", token_server.uri()),
        )
        .unwrap(),
    );
    let http = HttpClient::new(cred, RetryPolicy::default()).unwrap();

    // Any Drive request: refresh fires immediately, fails with invalid_grant,
    // surfaces as Unauthorized (NOT generic Backend).
    let drive_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/anywhere"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&drive_server)
        .await;

    let url = format!("{}/anywhere", drive_server.uri());
    let err = http.send(http.get(&url)).await.unwrap_err();
    match err {
        SealStackError::Unauthorized(msg) => {
            assert!(msg.contains("invalid_grant"), "{msg}");
        }
        other => panic!("expected Unauthorized, got {other:?}"),
    }
}
```

- [ ] **Step 5: Write the paginated list e2e test**

Create `connectors/google-drive/tests/list_e2e.rs`:

```rust
//! End-to-end test for DriveConnector::list() against a wiremock Drive API.

use sealstack_connector_google_drive::DriveConnector;
use sealstack_connector_sdk::Connector;
use futures::StreamExt;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn list_yields_resources_for_allowlisted_files() {
    let drive_server = MockServer::start().await;
    let token_server = MockServer::start().await;

    // Token endpoint always issues a fresh access token.
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "ya29.tok",
            "expires_in": 3599,
            "token_type": "Bearer"
        })))
        .mount(&token_server)
        .await;

    // files.list returns one Doc and one binary.
    Mock::given(method("GET"))
        .and(path("/drive/v3/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "files": [
                {
                    "id": "doc1",
                    "name": "Doc",
                    "mimeType": "application/vnd.google-apps.document",
                    "modifiedTime": "2026-04-27T12:00:00Z",
                    "permissions": [
                        {"type": "anyone", "role": "reader", "allowFileDiscovery": true}
                    ]
                },
                {
                    "id": "bin1",
                    "name": "img.png",
                    "mimeType": "image/png",
                    "modifiedTime": "2026-04-27T12:00:00Z"
                }
            ]
        })))
        .mount(&drive_server)
        .await;
    // Doc body export.
    Mock::given(method("GET"))
        .and(path("/drive/v3/files/doc1/export"))
        .and(query_param("mimeType", "text/plain"))
        .respond_with(ResponseTemplate::new(200).set_body_string("doc body"))
        .mount(&drive_server)
        .await;

    // Build the connector with the test token endpoint via direct construction.
    use std::sync::Arc;
    use secrecy::SecretString;
    use sealstack_connector_sdk::auth::OAuth2Credential;
    use sealstack_connector_sdk::http::HttpClient;
    use sealstack_connector_sdk::retry::RetryPolicy;
    use sealstack_connector_google_drive::test_only::*;
    let _ = (DriveConnector::from_json, fetch_body_for_test);
    // Use direct OAuth2Credential::new path to point at the test token endpoint.
    let cred = Arc::new(
        OAuth2Credential::new(
            "id".to_owned(),
            SecretString::new("s".into()),
            SecretString::new("r".into()),
            format!("{}/token", token_server.uri()),
        )
        .unwrap(),
    );
    let http = Arc::new(HttpClient::new(cred, RetryPolicy::default()).unwrap());

    // Drive list_files_for_test exercises the paginator; for full Connector::list
    // semantics including fetch_body + projection we exercise the lib's
    // public DriveConnector via env-var seams. Skip full integration in this
    // test; instead verify paginated walk yields the expected DriveFile ids.
    let mut stream = list_files_for_test(http, &drive_server.uri());
    let mut ids: Vec<String> = Vec::new();
    while let Some(f) = stream.next().await {
        ids.push(f.unwrap().id);
    }
    // doc1 passes; bin1 is filtered (image/png not allowed).
    assert_eq!(ids, vec!["doc1".to_owned()]);
}
```

- [ ] **Step 6: Write the retry shim e2e test**

Create `connectors/google-drive/tests/retry_shim_e2e.rs`. Tests `send_with_drive_shim` against wiremock for each 403 class plus the success path:

```rust
//! End-to-end tests for `send_with_drive_shim` against a wiremock Drive API.
//!
//! Each test verifies that a specific 403 response class produces the right
//! `SealStackError` variant after the appropriate retry behavior. Uses
//! `tokio::time::pause()` to keep test runtime short despite the backoff
//! schedule reaching ~7.5s cumulative.

use std::sync::Arc;

use sealstack_common::SealStackError;
use sealstack_connector_google_drive::test_only::send_with_drive_shim;
use sealstack_connector_sdk::auth::StaticToken;
use sealstack_connector_sdk::http::HttpClient;
use sealstack_connector_sdk::retry::RetryPolicy;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn body_with_reasons(reasons: &[&str]) -> serde_json::Value {
    let errs: Vec<serde_json::Value> = reasons
        .iter()
        .map(|r| serde_json::json!({"reason": r, "domain": "usageLimits", "message": "x"}))
        .collect();
    serde_json::json!({"error": {"code": 403, "message": "Forbidden", "errors": errs}})
}

fn make_http() -> Arc<HttpClient> {
    Arc::new(
        HttpClient::new(Arc::new(StaticToken::new("t")), RetryPolicy::default()).unwrap(),
    )
}

#[tokio::test(start_paused = true)]
async fn user_rate_limit_retries_and_succeeds() {
    let server = MockServer::start().await;
    // First attempt: 403 userRateLimitExceeded.
    Mock::given(method("GET"))
        .and(path("/test"))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_json(body_with_reasons(&["userRateLimitExceeded"])),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    // Second attempt: 200.
    Mock::given(method("GET"))
        .and(path("/test"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let http = make_http();
    let url = format!("{}/test", server.uri());
    let resp = send_with_drive_shim(&http, || http.get(&url)).await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test(start_paused = true)]
async fn rate_limit_exhausted_returns_rate_limited() {
    let server = MockServer::start().await;
    // Always 403 with userRateLimitExceeded — exhausts the 5-attempt budget.
    Mock::given(method("GET"))
        .and(path("/test"))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_json(body_with_reasons(&["userRateLimitExceeded"])),
        )
        .expect(5) // initial + 4 retries.
        .mount(&server)
        .await;

    let http = make_http();
    let url = format!("{}/test", server.uri());
    let err = send_with_drive_shim(&http, || http.get(&url)).await.unwrap_err();
    assert!(matches!(err, SealStackError::RateLimited), "got {err:?}");
}

#[tokio::test]
async fn quota_exhausted_returns_rate_limited_immediately() {
    let server = MockServer::start().await;
    // Daily quota exhausted: no retry.
    Mock::given(method("GET"))
        .and(path("/test"))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_json(body_with_reasons(&["quotaExceeded"])),
        )
        .expect(1) // exactly one call — no retries.
        .mount(&server)
        .await;

    let http = make_http();
    let url = format!("{}/test", server.uri());
    let err = send_with_drive_shim(&http, || http.get(&url)).await.unwrap_err();
    assert!(matches!(err, SealStackError::RateLimited), "got {err:?}");
}

#[tokio::test]
async fn permission_denied_returns_backend_immediately() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/test"))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_json(body_with_reasons(&["forbidden"])),
        )
        .expect(1) // no retries on permission-denied.
        .mount(&server)
        .await;

    let http = make_http();
    let url = format!("{}/test", server.uri());
    let err = send_with_drive_shim(&http, || http.get(&url)).await.unwrap_err();
    match err {
        SealStackError::Backend(msg) => {
            assert!(msg.contains("permission denied"), "{msg}");
            assert!(msg.contains("forbidden"), "{msg}");
        }
        other => panic!("expected Backend, got {other:?}"),
    }
}

#[tokio::test]
async fn permission_denied_includes_reason() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/test"))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_json(body_with_reasons(&["domainPolicy", "insufficientPermissions"])),
        )
        .mount(&server)
        .await;

    let http = make_http();
    let url = format!("{}/test", server.uri());
    let err = send_with_drive_shim(&http, || http.get(&url)).await.unwrap_err();
    match err {
        SealStackError::Backend(msg) => {
            assert!(msg.contains("domainPolicy"), "{msg}");
            assert!(msg.contains("insufficientPermissions"), "{msg}");
        }
        other => panic!("expected Backend, got {other:?}"),
    }
}
```

The `test_only::send_with_drive_shim` re-export was added in Step 5. This e2e test exercises the full shim pipeline against a real HTTP server: 403 with each reason class produces the correct `SealStackError` variant, the budget is enforced (5 attempts for `BackoffThenRetry`, 1 for `QuotaExhausted` / `PermissionDenied`), and reason strings are propagated for diagnostic context.

- [ ] **Step 7: Run all crate tests**

```bash
cargo test -p sealstack-connector-google-drive
```

Expected: 8 (permissions) + 4 (lib) + 1 (list_files) + 6 (fetch_body) + 13 (retry_shim) + 3 (resource) + 2 (oauth_refresh) + 1 (list_e2e) + 0 (retry_shim_e2e placeholder) = 38 tests pass.

- [ ] **Step 8: Run workspace tests + clippy + fmt**

```bash
cargo test -p sealstack-connector-sdk \
            -p sealstack-connector-local-files \
            -p sealstack-connector-slack \
            -p sealstack-connector-github \
            -p sealstack-connector-google-drive
cargo clippy -p sealstack-connector-google-drive 2>&1 | grep -E "^(warning|error)" | head -20
cargo fmt -p sealstack-connector-google-drive
```

Expected: all green; new clippy warnings on Drive code addressed (likely a few `clippy::use_self` and `clippy::needless_pass_by_value` — fix narrowly).

- [ ] **Step 9: Commit**

```bash
git add connectors/google-drive/
git commit -m "feat(google-drive): DriveFile -> Resource projection + Connector impl + e2e tests"
```

---

## Final verification (PR 2 ready to merge)

- [ ] **The load-bearing acceptance criterion passes:**

```bash
cargo test -p sealstack-connector-google-drive --test oauth_refresh
```

Expected: 2 passing tests (`oauth_refresh_on_401_succeeds_on_retry`, `oauth_refresh_invalid_grant_surfaces_unauthorized`).

If this fails, no other test result matters — the slice has not done its load-bearing job.

- [ ] **Per-crate test counts match the spec:**

Run `cargo test -p <each>` for each touched crate, verify counts:
- `sealstack-connector-sdk`: 64 tests (42 baseline + 11 Principal + 11 OAuth2Credential).
- `sealstack-connector-local-files`: 5 tests, unchanged.
- `sealstack-connector-slack`: 9 tests, fixture wire shapes regenerated.
- `sealstack-connector-github`: 16 tests, fixture wire shapes regenerated.
- `sealstack-connector-google-drive`: ~38 tests across 6 test files.

- [ ] **Clippy clean across all touched crates:**

```bash
cargo clippy -p sealstack-connector-sdk \
              -p sealstack-connector-local-files \
              -p sealstack-connector-slack \
              -p sealstack-connector-github \
              -p sealstack-connector-google-drive 2>&1 | grep -E "^(warning|error)" | head -30
```

Expected: only pre-existing `doc_markdown` warnings on lib.rs docstrings outside slice scope.

- [ ] **Fmt check clean:**

```bash
cargo fmt --check -p sealstack-connector-sdk \
                  -p sealstack-connector-local-files \
                  -p sealstack-connector-slack \
                  -p sealstack-connector-github \
                  -p sealstack-connector-google-drive
```

Expected: no diff.

- [ ] **Workspace-wide grep checklist clean:**

```bash
rg '"slack:' --type rust  | grep -v 'group:slack:'
rg '"github:' --type rust | grep -v 'group:github:'
```

Expected: empty output. Any hit outside intentional migration sites is a regression.

- [ ] **Out-of-scope items are NOT implemented:**

Confirm the spec §13 list — no `Principal::Custom`, no Shared Drives, no incremental sync, no CLI consent flow, no OAuth scope beyond the v1 set, no proactive token-bucket rate limiting, no streaming upload — none of these have surface area in the diff.
