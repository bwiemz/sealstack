//! Drive 403 classifier unit tests.

use sealstack_connector_google_drive::retry_shim::{Drive403Action, classify_drive_403};

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
