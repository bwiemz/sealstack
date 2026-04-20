//! Bearer-token authentication middleware.
//!
//! Applied to `/mcp` routes (and any other route group that opts in) to
//! extract and validate the caller identity from an `Authorization: Bearer`
//! header. The validated claims are injected into the request's extension map
//! as an [`AuthenticatedCaller`] and consumed by downstream handlers.
//!
//! # Modes
//!
//! * [`AuthMode::Disabled`] — accept every request. Useful for dev and
//!   locally-scoped tools. Missing or malformed tokens fall through with an
//!   anonymous caller.
//! * [`AuthMode::Hs256`] — validate signature against a shared HS256 secret.
//!   Production deployments should point at a real IdP and validate RS256 via
//!   JWKS instead; that lands in v0.2.
//!
//! # Why the middleware doesn't read `X-Cfg-*` headers
//!
//! The REST extractor in `rest.rs` still honors `X-Cfg-User`/`X-Cfg-Tenant`
//! for CLI traffic that can't mint JWTs. The MCP path must never honor them
//! because MCP clients can be arbitrary third-party software — trusting a
//! plain header there would let any client claim any tenant.

use std::sync::Arc;

use axum::{
    body::Body,
    extract::State,
    http::{HeaderValue, Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Runtime authentication mode.
#[derive(Clone, Debug)]
pub enum AuthMode {
    /// Accept all requests. Dev-only.
    Disabled,
    /// Validate HS256-signed JWTs against a shared secret. The secret is the
    /// only trust material; rotate it via env without re-deploying the
    /// gateway.
    Hs256 {
        /// Shared secret used to validate signatures.
        secret: String,
        /// Accepted `iss` claim values. Empty = accept any issuer.
        issuers: Vec<String>,
        /// Accepted `aud` claim values. Empty = skip audience validation.
        audiences: Vec<String>,
    },
}

impl AuthMode {
    /// Resolve an [`AuthMode`] from process environment.
    ///
    /// * `SIGNET_AUTH_MODE=disabled|hs256`
    /// * `SIGNET_AUTH_HS256_SECRET=<bytes>` (required when `hs256`)
    /// * `SIGNET_AUTH_ISSUERS=iss1,iss2` (optional)
    /// * `SIGNET_AUTH_AUDIENCES=aud1,aud2` (optional)
    #[must_use]
    pub fn from_env() -> Self {
        let mode = std::env::var("SIGNET_AUTH_MODE").unwrap_or_else(|_| "disabled".into());
        match mode.as_str() {
            "hs256" => {
                let secret = std::env::var("SIGNET_AUTH_HS256_SECRET").unwrap_or_default();
                if secret.is_empty() {
                    tracing::warn!(
                        "SIGNET_AUTH_MODE=hs256 but SIGNET_AUTH_HS256_SECRET is unset; falling back to disabled",
                    );
                    return Self::Disabled;
                }
                let issuers = split_csv("SIGNET_AUTH_ISSUERS");
                let audiences = split_csv("SIGNET_AUTH_AUDIENCES");
                Self::Hs256 {
                    secret,
                    issuers,
                    audiences,
                }
            }
            _ => Self::Disabled,
        }
    }

    /// True when the middleware should reject unauthenticated requests.
    #[must_use]
    pub const fn enforces(&self) -> bool {
        matches!(self, Self::Hs256 { .. })
    }
}

fn split_csv(env_var: &str) -> Vec<String> {
    std::env::var(env_var)
        .ok()
        .map(|v| {
            v.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Authenticated caller injected by the middleware into the request extensions.
#[derive(Clone, Debug)]
pub struct AuthenticatedCaller {
    /// Subject / user id.
    pub id: String,
    /// Tenant / workspace.
    pub tenant: String,
    /// Roles granted to the subject.
    pub roles: Vec<String>,
    /// Arbitrary extra claims surfaced to handlers verbatim.
    pub attrs: serde_json::Map<String, Value>,
}

/// Wire shape of the JWT claims we understand. Anything else is surfaced via
/// the catch-all `extras` map.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Claims {
    sub: String,
    #[serde(default)]
    tenant: String,
    #[serde(default)]
    roles: Vec<String>,
    /// Standard `exp` / `iat` / `iss` / `aud` validated by `jsonwebtoken`.
    #[serde(flatten)]
    extras: serde_json::Map<String, Value>,
}

/// Middleware entry point. Registered via `axum::middleware::from_fn_with_state`.
///
/// Always inserts an [`AuthenticatedCaller`] into the request extensions —
/// anonymous when running in [`AuthMode::Disabled`], or the validated JWT
/// subject otherwise. Downstream extractors can therefore rely on the
/// extension being present and focus on authorization, not authentication.
pub async fn require_bearer(
    State(mode): State<Arc<AuthMode>>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    let caller_result = match &*mode {
        AuthMode::Disabled => Ok(anonymous_caller()),
        AuthMode::Hs256 {
            secret,
            issuers,
            audiences,
        } => extract_and_validate(&req, secret, issuers, audiences),
    };
    match caller_result {
        Ok(caller) => {
            req.extensions_mut().insert(caller);
            next.run(req).await
        }
        Err(resp) => resp,
    }
}

fn anonymous_caller() -> AuthenticatedCaller {
    AuthenticatedCaller {
        id: "anonymous".to_owned(),
        tenant: String::new(),
        roles: Vec::new(),
        attrs: serde_json::Map::new(),
    }
}

fn extract_and_validate(
    req: &Request<Body>,
    secret: &str,
    issuers: &[String],
    audiences: &[String],
) -> Result<AuthenticatedCaller, Response> {
    let Some(auth_header) = req.headers().get(header::AUTHORIZATION) else {
        return Err(unauthorized("missing Authorization header"));
    };
    let Ok(auth_str) = auth_header.to_str() else {
        return Err(unauthorized("invalid Authorization header encoding"));
    };
    let token = match auth_str.strip_prefix("Bearer ") {
        Some(t) => t,
        None => return Err(unauthorized("Authorization scheme must be Bearer")),
    };

    let mut validation = Validation::new(Algorithm::HS256);
    if !issuers.is_empty() {
        validation.set_issuer(issuers);
    }
    if !audiences.is_empty() {
        validation.set_audience(audiences);
    } else {
        validation.validate_aud = false;
    }

    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map_err(|e| unauthorized(&format!("invalid token: {e}")))?;

    Ok(AuthenticatedCaller {
        id: data.claims.sub,
        tenant: data.claims.tenant,
        roles: data.claims.roles,
        attrs: data.claims.extras,
    })
}

fn unauthorized(reason: &str) -> Response {
    let mut resp = (StatusCode::UNAUTHORIZED, reason.to_owned()).into_response();
    // Per RFC 6750: advertise how to authenticate. The `resource_metadata`
    // parameter is an MCP 2025-11 extension that points clients at the
    // OAuth protected-resource metadata document.
    if let Ok(v) = HeaderValue::from_str(
        "Bearer realm=\"signet\", error=\"invalid_token\", \
         resource_metadata=\"/.well-known/oauth-protected-resource\"",
    ) {
        resp.headers_mut().insert(header::WWW_AUTHENTICATE, v);
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use time::OffsetDateTime;

    fn make_token(secret: &str, sub: &str, tenant: &str) -> String {
        #[derive(Serialize)]
        struct C<'a> {
            sub: &'a str,
            tenant: &'a str,
            roles: Vec<&'a str>,
            exp: i64,
        }
        let exp = OffsetDateTime::now_utc().unix_timestamp() + 3600;
        encode(
            &Header::default(),
            &C {
                sub,
                tenant,
                roles: vec!["reader"],
                exp,
            },
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap()
    }

    #[test]
    fn hs256_rejects_missing_header() {
        let req = Request::builder().uri("/").body(Body::empty()).unwrap();
        let err = extract_and_validate(&req, "s", &[], &[]).unwrap_err();
        assert_eq!(err.status(), StatusCode::UNAUTHORIZED);
        assert!(err.headers().contains_key(header::WWW_AUTHENTICATE));
    }

    #[test]
    fn hs256_accepts_valid_token() {
        let secret = "super-secret";
        let token = make_token(secret, "u1", "acme");
        let req = Request::builder()
            .uri("/")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let caller = extract_and_validate(&req, secret, &[], &[]).unwrap();
        assert_eq!(caller.id, "u1");
        assert_eq!(caller.tenant, "acme");
        assert!(caller.roles.iter().any(|r| r == "reader"));
    }

    #[test]
    fn hs256_rejects_wrong_secret() {
        let token = make_token("right", "u1", "t");
        let req = Request::builder()
            .uri("/")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let err = extract_and_validate(&req, "wrong", &[], &[]).unwrap_err();
        assert_eq!(err.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn disabled_mode_reports_not_enforcing() {
        let m = AuthMode::Disabled;
        assert!(!m.enforces());
    }
}
