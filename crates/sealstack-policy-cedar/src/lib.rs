//! Cedar ABAC policy engine adapter.
//!
//! Implements [`sealstack_engine::policy::PolicyEngine`] backed by
//! [Cedar](https://www.cedarpolicy.com/) — AWS's open-source policy language.
//! Cedar fits SealStack's needs because:
//!
//! 1. **Attribute-based**: every field on the principal or resource is
//!    available in policies, not just role names.
//! 2. **Formally verified evaluator**: the Cedar team has done the work
//!    on policy semantics. We just route data into it.
//! 3. **Familiar shape**: many teams already write Cedar policies for
//!    AWS Verified Permissions or Amazon Cognito.
//!
//! # Bundle layout
//!
//! Policy bundles live in the directory passed to [`CedarPolicy::load_from_dir`].
//! Filename convention: `<namespace>.<schema>.cedar`, containing one or more
//! Cedar policies. Example for `acme.Doc`:
//!
//! ```cedar
//! // acme.Doc.cedar
//! permit (
//!   principal in Sealstack::Group::"acme:eng",
//!   action == Sealstack::Action::"read",
//!   resource is Sealstack::Resource
//! );
//! ```
//!
//! # Entity model
//!
//! At evaluation time the adapter constructs three entities:
//!
//! - **Principal**: `Sealstack::User::"<caller.id>"` with attrs `tenant`,
//!   `groups`, `roles`, and any `caller.attrs` keys. Parent entities are
//!   `Sealstack::Group::"<g>"` for each group and `Sealstack::Role::"<r>"`
//!   for each role.
//! - **Action**: `Sealstack::Action::"read|write|list|delete"`.
//! - **Resource**: `Sealstack::Resource::"<record.id or 'unknown'>"`,
//!   typed `Sealstack::Resource`, with record fields as attributes.
//!
//! # Missing-bundle behavior
//!
//! Same as the WASM engine: by default, missing bundles return
//! [`PolicyVerdict::Allow`] for schemas with no `policy { ... }` block.
//! [`CedarPolicy::load_from_dir_deny_missing`] flips to fail-closed.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use cedar_policy::{
    Authorizer, Context, Decision, Entities, Entity, EntityId, EntityTypeName, EntityUid,
    PolicySet, Request, RestrictedExpression,
};
use dashmap::DashMap;
use sealstack_engine::api::EngineError;
use sealstack_engine::policy::{PolicyAction, PolicyEngine, PolicyInput, PolicyVerdict};
use serde_json::Value;

const PRINCIPAL_TYPE: &str = "Sealstack::User";
const ACTION_TYPE: &str = "Sealstack::Action";
const RESOURCE_TYPE: &str = "Sealstack::Resource";
const GROUP_TYPE: &str = "Sealstack::Group";
const ROLE_TYPE: &str = "Sealstack::Role";

/// Cedar-backed policy engine.
pub struct CedarPolicy {
    authorizer: Authorizer,
    bundles: DashMap<(String, String), PolicySet>,
    default_verdict: PolicyVerdict,
}

impl CedarPolicy {
    /// Load every `<namespace>.<schema>.cedar` file in `dir`. Missing
    /// bundles default to [`PolicyVerdict::Allow`].
    ///
    /// # Errors
    /// Returns an error if `dir` cannot be read. Individual files that fail
    /// to parse are logged and skipped — one bad file must not stall the
    /// whole engine.
    pub fn load_from_dir(dir: impl AsRef<Path>) -> std::io::Result<Self> {
        Self::load_impl(dir.as_ref(), PolicyVerdict::Allow)
    }

    /// Fail-closed variant: missing bundles deny.
    ///
    /// # Errors
    /// Same as [`Self::load_from_dir`].
    pub fn load_from_dir_deny_missing(dir: impl AsRef<Path>) -> std::io::Result<Self> {
        Self::load_impl(
            dir.as_ref(),
            PolicyVerdict::Deny {
                reason: "no cedar bundle registered for this schema".into(),
            },
        )
    }

    fn load_impl(dir: &Path, default_verdict: PolicyVerdict) -> std::io::Result<Self> {
        let bundles = DashMap::<(String, String), PolicySet>::new();

        if !dir.exists() {
            tracing::warn!(
                path = %dir.display(),
                "cedar policy dir does not exist; registry is empty",
            );
            return Ok(Self {
                authorizer: Authorizer::new(),
                bundles,
                default_verdict,
            });
        }

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("cedar") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Some((namespace, schema)) = stem.rsplit_once('.') else {
                tracing::warn!(file = %path.display(), "skipping unparseable cedar filename");
                continue;
            };
            let source = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(error = %e, file = %path.display(), "cedar read failed");
                    continue;
                }
            };
            match PolicySet::from_str(&source) {
                Ok(set) => {
                    tracing::info!(namespace, schema, "registered cedar bundle");
                    bundles.insert((namespace.to_owned(), schema.to_owned()), set);
                }
                Err(e) => {
                    tracing::error!(error = %e, file = %path.display(), "cedar parse failed");
                }
            }
        }

        Ok(Self {
            authorizer: Authorizer::new(),
            bundles,
            default_verdict,
        })
    }

    /// Construct a Cedar engine from in-memory policies, keyed by
    /// `(namespace, schema)`. Useful in tests.
    ///
    /// # Errors
    /// Returns [`PolicyLoadError::Parse`] if any policy source fails to parse.
    pub fn from_sources<I, S>(
        sources: I,
        default_verdict: PolicyVerdict,
    ) -> Result<Self, PolicyLoadError>
    where
        I: IntoIterator<Item = ((S, S), S)>,
        S: AsRef<str>,
    {
        let bundles = DashMap::new();
        for ((namespace, schema), source) in sources {
            let set = PolicySet::from_str(source.as_ref())
                .map_err(|e| PolicyLoadError::Parse(format!("{e}")))?;
            bundles.insert(
                (namespace.as_ref().to_owned(), schema.as_ref().to_owned()),
                set,
            );
        }
        Ok(Self {
            authorizer: Authorizer::new(),
            bundles,
            default_verdict,
        })
    }
}

#[async_trait]
impl PolicyEngine for CedarPolicy {
    async fn evaluate(&self, input: PolicyInput<'_>) -> Result<PolicyVerdict, EngineError> {
        let Some(bundle_ref) = self
            .bundles
            .get(&(input.namespace.to_owned(), input.schema.to_owned()))
        else {
            return Ok(self.default_verdict.clone());
        };
        let bundle = bundle_ref.clone();
        drop(bundle_ref);

        let principal_uid = make_uid(PRINCIPAL_TYPE, &input.caller.id)
            .map_err(|e| EngineError::Backend(format!("cedar principal uid: {e}")))?;
        let action_uid = make_uid(ACTION_TYPE, action_str(input.action))
            .map_err(|e| EngineError::Backend(format!("cedar action uid: {e}")))?;
        let resource_id = input
            .record
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let resource_uid = make_uid(RESOURCE_TYPE, resource_id)
            .map_err(|e| EngineError::Backend(format!("cedar resource uid: {e}")))?;

        let principal_entity = build_principal_entity(input.caller, &principal_uid)
            .map_err(|e| EngineError::Backend(format!("cedar principal entity: {e}")))?;
        let resource_entity = build_resource_entity(input.record, &resource_uid)
            .map_err(|e| EngineError::Backend(format!("cedar resource entity: {e}")))?;

        let mut all = vec![principal_entity, resource_entity];
        // Surface each group/role as its own entity so `principal in
        // Sealstack::Group::"x"` is checkable. Cedar requires the parent
        // entities to exist in the store even if they have no attrs.
        for g in &input.caller.groups {
            if let Ok(uid) = make_uid(GROUP_TYPE, g)
                && let Ok(entity) = Entity::new(uid, HashMap::new(), HashSet::new())
            {
                all.push(entity);
            }
        }
        for r in &input.caller.roles {
            if let Ok(uid) = make_uid(ROLE_TYPE, r)
                && let Ok(entity) = Entity::new(uid, HashMap::new(), HashSet::new())
            {
                all.push(entity);
            }
        }

        let entities = Entities::from_entities(all, None)
            .map_err(|e| EngineError::Backend(format!("cedar entities: {e}")))?;

        let request = Request::new(
            principal_uid,
            action_uid,
            resource_uid,
            Context::empty(),
            None,
        )
        .map_err(|e| EngineError::Backend(format!("cedar request: {e}")))?;

        let response = self.authorizer.is_authorized(&request, &bundle, &entities);
        match response.decision() {
            Decision::Allow => Ok(PolicyVerdict::Allow),
            Decision::Deny => Ok(PolicyVerdict::Deny {
                reason: format!(
                    "cedar denied {}.{} for action {}",
                    input.namespace,
                    input.schema,
                    action_str(input.action),
                ),
            }),
        }
    }
}

fn action_str(action: PolicyAction) -> &'static str {
    match action {
        PolicyAction::Read => "read",
        PolicyAction::Write => "write",
        PolicyAction::List => "list",
        PolicyAction::Delete => "delete",
    }
}

fn make_uid(type_name: &str, id: &str) -> Result<EntityUid, PolicyLoadError> {
    let type_name = EntityTypeName::from_str(type_name)
        .map_err(|e| PolicyLoadError::Parse(format!("cedar type `{type_name}`: {e}")))?;
    let id = EntityId::from_str(id)
        .map_err(|e| PolicyLoadError::Parse(format!("cedar id `{id}`: {e}")))?;
    Ok(EntityUid::from_type_name_and_id(type_name, id))
}

fn build_principal_entity(
    caller: &sealstack_engine::api::Caller,
    uid: &EntityUid,
) -> Result<Entity, PolicyLoadError> {
    let mut attrs: HashMap<String, RestrictedExpression> = HashMap::new();
    attrs.insert(
        "tenant".into(),
        RestrictedExpression::new_string(caller.tenant.clone()),
    );
    attrs.insert(
        "groups".into(),
        RestrictedExpression::new_set(
            caller
                .groups
                .iter()
                .map(|g| RestrictedExpression::new_string(g.clone())),
        ),
    );
    attrs.insert(
        "roles".into(),
        RestrictedExpression::new_set(
            caller
                .roles
                .iter()
                .map(|r| RestrictedExpression::new_string(r.clone())),
        ),
    );
    for (k, v) in &caller.attrs {
        if let Some(expr) = json_to_cedar(v) {
            attrs.insert(k.clone(), expr);
        }
    }

    let mut parents: HashSet<EntityUid> = HashSet::new();
    for g in &caller.groups {
        if let Ok(uid) = make_uid(GROUP_TYPE, g) {
            parents.insert(uid);
        }
    }
    for r in &caller.roles {
        if let Ok(uid) = make_uid(ROLE_TYPE, r) {
            parents.insert(uid);
        }
    }

    Entity::new(uid.clone(), attrs, parents)
        .map_err(|e| PolicyLoadError::Parse(format!("cedar entity build: {e}")))
}

fn build_resource_entity(record: &Value, uid: &EntityUid) -> Result<Entity, PolicyLoadError> {
    let mut attrs: HashMap<String, RestrictedExpression> = HashMap::new();
    if let Some(obj) = record.as_object() {
        for (k, v) in obj {
            if let Some(expr) = json_to_cedar(v) {
                attrs.insert(k.clone(), expr);
            }
        }
    }
    Entity::new(uid.clone(), attrs, HashSet::new())
        .map_err(|e| PolicyLoadError::Parse(format!("cedar resource build: {e}")))
}

/// Convert a JSON value to a Cedar `RestrictedExpression`. Cedar's type
/// system is stricter than JSON: nested arrays must be homogeneous, and
/// records can't have key collisions. We do a best-effort flatten —
/// values we can't represent are dropped (with a trace log) rather than
/// failing the whole evaluation, which would deny otherwise-legitimate
/// requests over a non-essential attribute.
fn json_to_cedar(v: &Value) -> Option<RestrictedExpression> {
    match v {
        Value::Null => None,
        Value::Bool(b) => Some(RestrictedExpression::new_bool(*b)),
        Value::Number(n) => n.as_i64().map(RestrictedExpression::new_long).or_else(|| {
            n.as_f64()
                .map(|f| RestrictedExpression::new_decimal(f.to_string()))
        }),
        Value::String(s) => Some(RestrictedExpression::new_string(s.clone())),
        Value::Array(arr) => Some(RestrictedExpression::new_set(
            arr.iter().filter_map(json_to_cedar),
        )),
        Value::Object(obj) => {
            let pairs: HashMap<String, RestrictedExpression> = obj
                .iter()
                .filter_map(|(k, v)| json_to_cedar(v).map(|e| (k.clone(), e)))
                .collect();
            RestrictedExpression::new_record(pairs).ok()
        }
    }
}

/// Failure modes from loading or validating Cedar bundles.
#[derive(Debug, thiserror::Error)]
pub enum PolicyLoadError {
    /// Cedar policy source did not parse.
    #[error("parse: {0}")]
    Parse(String),
    /// IO failure walking the bundle directory.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Build a [`CedarPolicy`] as an `Arc<dyn PolicyEngine>` for the gateway.
///
/// # Errors
/// Returns an error if the directory cannot be read.
pub fn build(
    dir: Option<&Path>,
    deny_missing: bool,
) -> Result<Arc<dyn PolicyEngine>, PolicyLoadError> {
    match dir {
        Some(d) if deny_missing => Ok(Arc::new(CedarPolicy::load_from_dir_deny_missing(d)?)),
        Some(d) => Ok(Arc::new(CedarPolicy::load_from_dir(d)?)),
        None => {
            tracing::warn!(
                "cedar engine constructed without bundle dir; every request \
                 returns the default verdict",
            );
            let default_verdict = if deny_missing {
                PolicyVerdict::Deny {
                    reason: "cedar engine started without bundle dir".into(),
                }
            } else {
                PolicyVerdict::Allow
            };
            Ok(Arc::new(CedarPolicy {
                authorizer: Authorizer::new(),
                bundles: DashMap::new(),
                default_verdict,
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sealstack_engine::api::Caller;

    fn caller(id: &str, groups: &[&str]) -> Caller {
        Caller {
            id: id.into(),
            tenant: "acme".into(),
            groups: groups.iter().map(|s| (*s).to_owned()).collect(),
            roles: vec![],
            attrs: Default::default(),
        }
    }

    fn record(id: &str, owner: &str) -> Value {
        serde_json::json!({ "id": id, "owner": owner })
    }

    #[tokio::test]
    async fn allow_when_group_matches() {
        // `permit` for users in group "eng" reading any Doc.
        let src = r#"
            permit (
              principal in Sealstack::Group::"eng",
              action == Sealstack::Action::"read",
              resource
            );
        "#;
        let policy = CedarPolicy::from_sources(
            [(("acme", "Doc"), src)],
            PolicyVerdict::Deny {
                reason: "default deny".into(),
            },
        )
        .expect("policy parses");

        let caller = caller("alice", &["eng"]);
        let rec = record("r1", "alice");
        let v = policy
            .evaluate(PolicyInput {
                namespace: "acme",
                schema: "Doc",
                action: PolicyAction::Read,
                caller: &caller,
                record: &rec,
            })
            .await
            .unwrap();
        assert!(v.is_allow(), "got {v:?}");
    }

    #[tokio::test]
    async fn deny_when_group_missing() {
        let src = r#"
            permit (
              principal in Sealstack::Group::"eng",
              action == Sealstack::Action::"read",
              resource
            );
        "#;
        let policy = CedarPolicy::from_sources(
            [(("acme", "Doc"), src)],
            PolicyVerdict::Deny {
                reason: "default deny".into(),
            },
        )
        .expect("parses");

        let caller = caller("bob", &["sales"]);
        let rec = record("r1", "bob");
        let v = policy
            .evaluate(PolicyInput {
                namespace: "acme",
                schema: "Doc",
                action: PolicyAction::Read,
                caller: &caller,
                record: &rec,
            })
            .await
            .unwrap();
        assert!(matches!(v, PolicyVerdict::Deny { .. }));
    }

    #[tokio::test]
    async fn deny_when_action_mismatched() {
        let src = r#"
            permit (
              principal,
              action == Sealstack::Action::"read",
              resource
            );
        "#;
        let policy = CedarPolicy::from_sources(
            [(("acme", "Doc"), src)],
            PolicyVerdict::Deny {
                reason: "default deny".into(),
            },
        )
        .expect("parses");

        let caller = caller("alice", &["eng"]);
        let rec = record("r1", "alice");
        let v = policy
            .evaluate(PolicyInput {
                namespace: "acme",
                schema: "Doc",
                action: PolicyAction::Write,
                caller: &caller,
                record: &rec,
            })
            .await
            .unwrap();
        assert!(matches!(v, PolicyVerdict::Deny { .. }));
    }

    #[tokio::test]
    async fn missing_bundle_uses_default_allow() {
        let policy =
            CedarPolicy::from_sources::<_, &str>([], PolicyVerdict::Allow).expect("empty parses");
        let caller = caller("alice", &[]);
        let rec = record("r1", "alice");
        let v = policy
            .evaluate(PolicyInput {
                namespace: "acme",
                schema: "Nothing",
                action: PolicyAction::Read,
                caller: &caller,
                record: &rec,
            })
            .await
            .unwrap();
        assert!(v.is_allow());
    }

    #[tokio::test]
    async fn missing_bundle_deny_variant_denies() {
        let policy = CedarPolicy::from_sources::<_, &str>(
            [],
            PolicyVerdict::Deny {
                reason: "no bundle".into(),
            },
        )
        .expect("empty parses");
        let caller = caller("alice", &[]);
        let rec = record("r1", "alice");
        let v = policy
            .evaluate(PolicyInput {
                namespace: "acme",
                schema: "Nothing",
                action: PolicyAction::Read,
                caller: &caller,
                record: &rec,
            })
            .await
            .unwrap();
        assert!(matches!(v, PolicyVerdict::Deny { .. }));
    }

    #[tokio::test]
    async fn allow_using_resource_attribute() {
        // Resource is allowed only if the principal's id matches the
        // resource's owner attribute. Exercises field projection from JSON.
        let src = r#"
            permit (
              principal,
              action == Sealstack::Action::"read",
              resource
            )
            when {
              principal == Sealstack::User::"alice" &&
              resource.owner == "alice"
            };
        "#;
        let policy = CedarPolicy::from_sources(
            [(("acme", "Doc"), src)],
            PolicyVerdict::Deny {
                reason: "default deny".into(),
            },
        )
        .expect("parses");

        let alice = caller("alice", &[]);
        let v = policy
            .evaluate(PolicyInput {
                namespace: "acme",
                schema: "Doc",
                action: PolicyAction::Read,
                caller: &alice,
                record: &record("r1", "alice"),
            })
            .await
            .unwrap();
        assert!(v.is_allow());

        let v = policy
            .evaluate(PolicyInput {
                namespace: "acme",
                schema: "Doc",
                action: PolicyAction::Read,
                caller: &alice,
                record: &record("r2", "bob"),
            })
            .await
            .unwrap();
        assert!(matches!(v, PolicyVerdict::Deny { .. }));
    }

    #[test]
    fn json_to_cedar_handles_primitives_and_collections() {
        assert!(json_to_cedar(&Value::Null).is_none());
        assert!(json_to_cedar(&Value::Bool(true)).is_some());
        assert!(json_to_cedar(&Value::Number(serde_json::Number::from(42))).is_some());
        assert!(json_to_cedar(&Value::String("x".into())).is_some());
        assert!(json_to_cedar(&serde_json::json!([1, 2, 3])).is_some());
        assert!(json_to_cedar(&serde_json::json!({ "a": 1 })).is_some());
    }
}
