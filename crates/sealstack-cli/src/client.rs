//! HTTP client for the gateway's REST surface.
//!
//! Every method returns `serde_json::Value` extracted from the `data` field of
//! the gateway's envelope `{ "data": ..., "error": null }`. Errors on the
//! envelope surface as `anyhow::Error` with the gateway's code+message.

use anyhow::{Context, bail};
use reqwest::StatusCode;
use serde_json::{Value, json};

/// Thin wrapper around `reqwest::Client` with the gateway base URL and caller headers.
pub(crate) struct Client {
    base: String,
    http: reqwest::Client,
    user: String,
}

impl Client {
    /// Construct a new client.
    pub(crate) fn new(base: impl Into<String>, user: impl Into<String>) -> Self {
        Self {
            base: base.into().trim_end_matches('/').to_string(),
            http: reqwest::Client::builder()
                .user_agent(concat!("sealstack-cli/", env!("CARGO_PKG_VERSION")))
                .build()
                .expect("build reqwest client"),
            user: user.into(),
        }
    }

    fn url(&self, path: &str) -> String {
        if path.starts_with('/') {
            format!("{}{}", self.base, path)
        } else {
            format!("{}/{}", self.base, path)
        }
    }

    async fn get_inner(&self, path: &str) -> anyhow::Result<Value> {
        let resp = self
            .http
            .get(self.url(path))
            .header("X-Cfg-User", &self.user)
            .send()
            .await
            .with_context(|| format!("GET {path}"))?;
        unwrap_envelope(resp).await
    }

    async fn post_inner(&self, path: &str, body: Value) -> anyhow::Result<Value> {
        let resp = self
            .http
            .post(self.url(path))
            .header("X-Cfg-User", &self.user)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("POST {path}"))?;
        unwrap_envelope(resp).await
    }

    // ---- Typed endpoints -------------------------------------------------

    /// `GET /healthz`. Returns `true` on 200.
    pub(crate) async fn healthz(&self) -> anyhow::Result<bool> {
        let resp = self
            .http
            .get(self.url("/healthz"))
            .send()
            .await
            .context("healthz")?;
        Ok(resp.status() == StatusCode::OK)
    }

    /// `GET /v1/schemas`.
    pub(crate) async fn list_schemas(&self) -> anyhow::Result<Value> {
        self.get_inner("/v1/schemas").await
    }

    /// `GET /v1/schemas/:qualified`.
    pub(crate) async fn get_schema(&self, qualified: &str) -> anyhow::Result<Value> {
        self.get_inner(&format!("/v1/schemas/{qualified}")).await
    }

    /// `POST /v1/schemas` — register compiled schema metadata.
    pub(crate) async fn register_schema(&self, meta: Value) -> anyhow::Result<Value> {
        self.post_inner("/v1/schemas", json!({ "meta": meta })).await
    }

    /// `POST /v1/schemas/:qualified/ddl`.
    pub(crate) async fn apply_schema_ddl(&self, qualified: &str, ddl: &str) -> anyhow::Result<Value> {
        self.post_inner(
            &format!("/v1/schemas/{qualified}/ddl"),
            json!({ "ddl": ddl }),
        )
        .await
    }

    /// `GET /v1/connectors`.
    pub(crate) async fn list_connectors(&self) -> anyhow::Result<Value> {
        self.get_inner("/v1/connectors").await
    }

    /// `POST /v1/connectors`.
    pub(crate) async fn register_connector(
        &self,
        kind: &str,
        schema: &str,
        config: Value,
    ) -> anyhow::Result<Value> {
        self.post_inner(
            "/v1/connectors",
            json!({ "kind": kind, "schema": schema, "config": config }),
        )
        .await
    }

    /// `POST /v1/connectors/:id/sync`.
    pub(crate) async fn sync_connector(&self, id: &str) -> anyhow::Result<Value> {
        self.post_inner(&format!("/v1/connectors/{id}/sync"), json!({}))
            .await
    }

    /// `POST /v1/query`.
    pub(crate) async fn query(
        &self,
        schema: &str,
        query: &str,
        top_k: Option<usize>,
        filters: Value,
    ) -> anyhow::Result<Value> {
        self.post_inner(
            "/v1/query",
            json!({
                "schema":  schema,
                "query":   query,
                "top_k":   top_k,
                "filters": filters,
            }),
        )
        .await
    }

    /// `GET /v1/receipts/:id`.
    pub(crate) async fn receipt(&self, id: &str) -> anyhow::Result<Value> {
        self.get_inner(&format!("/v1/receipts/{id}")).await
    }
}

/// Unwrap the gateway's `{ data, error }` envelope, propagating `error` as anyhow.
async fn unwrap_envelope(resp: reqwest::Response) -> anyhow::Result<Value> {
    let status = resp.status();
    let body: Value = resp
        .json()
        .await
        .unwrap_or_else(|_| json!({ "data": null, "error": { "code": "bad_response", "message": "non-JSON body" } }));

    if let Some(err) = body.get("error").filter(|v| !v.is_null()) {
        let code = err
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let message = err
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unspecified");
        bail!("gateway error ({status} {code}): {message}");
    }

    Ok(body.get("data").cloned().unwrap_or(Value::Null))
}
