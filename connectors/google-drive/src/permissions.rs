//! Map Drive `permissions[]` entries to `SealStack` [`PermissionPredicate`]s.

#![allow(clippy::redundant_pub_crate)]

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

/// Map a Drive permission to a `SealStack` `PermissionPredicate`.
///
/// Returns `None` for permission kinds the connector doesn't recognize
/// (logged at warn level — these are real ACL signals being silently
/// dropped, which is unlike skipped MIME types where they're not).
pub(crate) fn drive_permission_to_predicate(p: &DrivePermission) -> Option<PermissionPredicate> {
    let principal = match p.kind.as_str() {
        "user" => p
            .email_address
            .as_ref()
            .map(|e| Principal::User(e.clone()))?,
        "group" => p
            .email_address
            .as_ref()
            .map(|e| Principal::Group(e.clone()))?,
        "domain" => p.domain.as_ref().map(|d| Principal::Domain(d.clone()))?,
        "anyone" => {
            // CRITICAL: ambiguous discovery defaults to link-only (not discoverable).
            // Treating absence as `Anyone` would be the Glean-class bug — link-only
            // docs leaking into public search results.
            let discoverable = p.allow_file_discovery.unwrap_or(false);
            if discoverable {
                Principal::Anyone
            } else {
                Principal::AnyoneWithLink
            }
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

/// Project a Drive role to a `SealStack` action.
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
