//! `DriveFile + body + ACLs → Resource` projection — the connector's product.
//!
//! v0.2 Shared Drives lands here by extending the projection with
//! drive-level ACL inheritance/merging.

// pub(crate) items in this private module trigger clippy::redundant_pub_crate
// because the module itself is not pub. Suppress: these items are deliberately
// pub(crate) to signal "crate-internal only".
#![allow(clippy::redundant_pub_crate)]

use sealstack_common::{SealStackError, SealStackResult};
use sealstack_connector_sdk::{PermissionPredicate, Resource, ResourceId};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::files::DriveFile;
use crate::permissions::drive_permission_to_predicate;

/// Project a [`DriveFile`] + fetched body into a [`Resource`].
///
/// Caller invokes only when `body` is `Some(content)` from `fetch_body`.
/// Skipped files (None body) should be dropped before reaching this function.
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
        _ => "unknown", // unreachable in practice — callers filter by allowlist
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
