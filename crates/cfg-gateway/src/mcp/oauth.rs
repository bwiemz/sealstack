//! OAuth 2.1 discovery endpoints for MCP (per 2025-11 spec).
//!
//! MCP clients discover authorization requirements by fetching:
//!
//! * `/.well-known/oauth-protected-resource` — identifies this service as a
//!   protected resource and names the authorization server(s) clients should
//!   bounce through.
//!
//! * `/.well-known/oauth-authorization-server` — (optional) if ContextForge is
//!   acting as its own authorization server, this endpoint exposes its issuer,
//!   authorization/token endpoints, and supported scopes. In most deployments
//!   this is hosted by the customer's identity provider (Okta, Auth0, Entra ID),
//!   in which case we return a redirect or a 404 here and advertise that IdP as
//!   the authorization server in the protected-resource metadata.
//!
//! # Configuration
//!
//! All metadata values come from [`OAuthMetadataConfig`]. In production, set
//! them from environment variables in your deployment; in dev, `::dev_default`
//! gives you a self-signed local setup.

use axum::{
    Json, Router,
    extract::State,
    response::IntoResponse,
    routing::get,
};
use serde::{Deserialize, Serialize};

/// Values advertised on the two well-known endpoints.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OAuthMetadataConfig {
    /// Canonical URL of this gateway (the "resource").
    pub resource: String,
    /// List of acceptable authorization servers (issuer URLs).
    pub authorization_servers: Vec<String>,
    /// OAuth scopes this resource recognizes.
    pub scopes_supported: Vec<String>,
    /// Bearer methods accepted (`header` is required; `form` and `query` are not).
    pub bearer_methods_supported: Vec<String>,
    /// Optional token introspection endpoint (for resource-server-side validation).
    pub introspection_endpoint: Option<String>,
    /// Optional revocation endpoint.
    pub revocation_endpoint: Option<String>,
    /// Whether this gateway also acts as its own authorization server. Default: false.
    pub act_as_authorization_server: bool,
}

impl OAuthMetadataConfig {
    /// Minimal dev default — trusts a locally-running Keycloak or Auth0 tenant
    /// specified by env var, falling back to a placeholder.
    #[must_use]
    pub fn dev_default() -> Self {
        let base = std::env::var("CFG_PUBLIC_URL")
            .unwrap_or_else(|_| "http://localhost:7070".into());
        let auth_server = std::env::var("CFG_OAUTH_ISSUER")
            .unwrap_or_else(|_| "http://localhost:8080/realms/contextforge".into());
        Self {
            resource: base,
            authorization_servers: vec![auth_server],
            scopes_supported: vec![
                "context.read".into(),
                "context.search".into(),
                "context.write".into(),
                "context.admin".into(),
            ],
            bearer_methods_supported: vec!["header".into()],
            introspection_endpoint: None,
            revocation_endpoint: None,
            act_as_authorization_server: false,
        }
    }
}

/// Response body for `/.well-known/oauth-protected-resource`.
///
/// Owned (not borrowed) so the struct can outlive the handler stack frame
/// when serialized through `axum::Json`.
#[derive(Clone, Debug, Serialize)]
struct ProtectedResourceMetadata {
    resource: String,
    authorization_servers: Vec<String>,
    scopes_supported: Vec<String>,
    bearer_methods_supported: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resource_documentation: Option<String>,
}

/// Response body for `/.well-known/oauth-authorization-server`.
/// Only returned when `act_as_authorization_server == true`.
#[derive(Clone, Debug, Serialize)]
struct AuthorizationServerMetadata {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    scopes_supported: Vec<String>,
    response_types_supported: Vec<&'static str>,
    grant_types_supported: Vec<&'static str>,
    code_challenge_methods_supported: Vec<&'static str>,
    token_endpoint_auth_methods_supported: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    introspection_endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revocation_endpoint: Option<String>,
}

/// Build the well-known routes.
#[must_use]
pub fn router(config: OAuthMetadataConfig) -> Router {
    Router::new()
        .route(
            "/.well-known/oauth-protected-resource",
            get(handle_protected_resource),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(handle_authorization_server),
        )
        .with_state(std::sync::Arc::new(config))
}

async fn handle_protected_resource(
    State(cfg): State<std::sync::Arc<OAuthMetadataConfig>>,
) -> impl IntoResponse {
    Json(ProtectedResourceMetadata {
        resource: cfg.resource.clone(),
        authorization_servers: cfg.authorization_servers.clone(),
        scopes_supported: cfg.scopes_supported.clone(),
        bearer_methods_supported: cfg.bearer_methods_supported.clone(),
        resource_documentation: Some("https://docs.contextforge.dev/mcp/auth".to_owned()),
    })
}

async fn handle_authorization_server(
    State(cfg): State<std::sync::Arc<OAuthMetadataConfig>>,
) -> axum::response::Response {
    if !cfg.act_as_authorization_server {
        return (
            axum::http::StatusCode::NOT_FOUND,
            "this gateway is not an authorization server; see /.well-known/oauth-protected-resource",
        )
            .into_response();
    }
    let issuer = cfg.resource.clone();
    Json(AuthorizationServerMetadata {
        authorization_endpoint: format!("{issuer}/oauth/authorize"),
        token_endpoint: format!("{issuer}/oauth/token"),
        issuer,
        scopes_supported: cfg.scopes_supported.clone(),
        response_types_supported: vec!["code"],
        grant_types_supported: vec!["authorization_code", "refresh_token", "client_credentials"],
        code_challenge_methods_supported: vec!["S256"],
        token_endpoint_auth_methods_supported: vec![
            "client_secret_basic",
            "client_secret_post",
            "private_key_jwt",
            "none",
        ],
        introspection_endpoint: cfg.introspection_endpoint.clone(),
        revocation_endpoint: cfg.revocation_endpoint.clone(),
    })
    .into_response()
}
