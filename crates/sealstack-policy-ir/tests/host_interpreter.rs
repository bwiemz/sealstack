#![cfg(feature = "host")]

use sealstack_policy_ir::{MAGIC, action_bit, host, op};

fn build_ir(rules: &[(u8, &[u8])]) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(rules.len() as u8);
    let mut running = 0u16;
    let mut offsets = Vec::new();
    for (_mask, bytes) in rules {
        offsets.push(running);
        running += bytes.len() as u16;
    }
    for ((mask, _), off) in rules.iter().zip(&offsets) {
        body.push(*mask);
        body.extend_from_slice(&off.to_le_bytes());
    }
    for (_mask, bytes) in rules {
        body.extend_from_slice(bytes);
    }
    let mut ir = Vec::new();
    ir.extend_from_slice(&MAGIC);
    ir.extend_from_slice(&(body.len() as u32).to_le_bytes());
    ir.extend_from_slice(&body);
    ir
}

#[test]
fn empty_action_table_denies() {
    let ir = build_ir(&[]);
    assert_eq!(
        host::interpret(
            &ir,
            &serde_json::json!({}),
            &serde_json::json!({}),
            action_bit::READ
        ),
        Ok(false)
    );
}

#[test]
fn literal_true_allows_read() {
    // rule bytes: LIT_BOOL 1, RESULT
    let rule: &[u8] = &[op::LIT_BOOL, 1, op::RESULT];
    let ir = build_ir(&[(action_bit::READ, rule)]);
    assert_eq!(
        host::interpret(
            &ir,
            &serde_json::json!({}),
            &serde_json::json!({}),
            action_bit::READ
        ),
        Ok(true)
    );
    // write is not in the table → deny
    assert_eq!(
        host::interpret(
            &ir,
            &serde_json::json!({}),
            &serde_json::json!({}),
            action_bit::WRITE
        ),
        Ok(false)
    );
}

#[test]
fn bad_magic_is_an_error() {
    let mut ir = build_ir(&[]);
    ir[0] = b'X';
    assert!(matches!(
        host::interpret(
            &ir,
            &serde_json::json!({}),
            &serde_json::json!({}),
            action_bit::READ
        ),
        Err(host::IrError::BadMagic)
    ));
}

// ---------------------------------------------------------------------------
// Opcode parity tests — mirror the wasm-side roundtrip coverage so host and
// wasm interpreters are exercised on the same opcode shapes.
// ---------------------------------------------------------------------------

fn caller_with_roles(roles: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "id": "u",
        "email": "",
        "groups": [],
        "team": "",
        "tenant": "",
        "roles": roles,
        "attrs": {},
    })
}

#[test]
fn has_role_allows_admin() {
    // rule bytes: LOAD_CALLER [], LIT_STR "admin", CALL_HAS_ROLE, RESULT
    let mut rule: Vec<u8> = Vec::new();
    rule.push(op::LOAD_CALLER);
    rule.push(0); // n_seg = 0
    rule.push(op::LIT_STR);
    rule.extend_from_slice(&5u16.to_le_bytes());
    rule.extend_from_slice(b"admin");
    rule.push(op::CALL_HAS_ROLE);
    rule.push(op::RESULT);
    let ir = build_ir(&[(action_bit::READ, &rule)]);

    let caller_admin = caller_with_roles(&["admin"]);
    let caller_user = caller_with_roles(&["user"]);
    let record = serde_json::json!({"id": "r", "tenant": ""});

    assert_eq!(
        host::interpret(&ir, &caller_admin, &record, action_bit::READ),
        Ok(true)
    );
    assert_eq!(
        host::interpret(&ir, &caller_user, &record, action_bit::READ),
        Ok(false)
    );
}

#[test]
fn tenant_match_allows_matching_tenants() {
    // rule bytes: LOAD_CALLER [], LOAD_SELF [], CALL_TENANT_MATCH, RESULT
    let mut rule: Vec<u8> = Vec::new();
    rule.push(op::LOAD_CALLER);
    rule.push(0);
    rule.push(op::LOAD_SELF);
    rule.push(0);
    rule.push(op::CALL_TENANT_MATCH);
    rule.push(op::RESULT);
    let ir = build_ir(&[(action_bit::READ, &rule)]);

    let caller = serde_json::json!({
        "id": "u", "email": "", "groups": [], "team": "",
        "tenant": "acme", "roles": [], "attrs": {}
    });
    let record_match = serde_json::json!({"id": "r", "tenant": "acme"});
    let record_mismatch = serde_json::json!({"id": "r", "tenant": "other"});

    assert_eq!(
        host::interpret(&ir, &caller, &record_match, action_bit::READ),
        Ok(true)
    );
    assert_eq!(
        host::interpret(&ir, &caller, &record_mismatch, action_bit::READ),
        Ok(false)
    );
}

#[test]
fn in_op_finds_value_in_list() {
    // rule bytes: LIT_STR "u1", LOAD_SELF [shared_with], IN, RESULT
    let mut rule: Vec<u8> = Vec::new();
    rule.push(op::LIT_STR);
    rule.extend_from_slice(&2u16.to_le_bytes());
    rule.extend_from_slice(b"u1");
    rule.push(op::LOAD_SELF);
    rule.push(1); // n_seg = 1
    rule.extend_from_slice(&11u16.to_le_bytes());
    rule.extend_from_slice(b"shared_with");
    rule.push(op::IN);
    rule.push(op::RESULT);
    let ir = build_ir(&[(action_bit::READ, &rule)]);

    let caller = caller_with_roles(&[]);
    let record_hit = serde_json::json!({
        "id": "r", "tenant": "",
        "shared_with": ["u1", "u2"]
    });
    let record_miss = serde_json::json!({
        "id": "r", "tenant": "",
        "shared_with": ["u3"]
    });

    assert_eq!(
        host::interpret(&ir, &caller, &record_hit, action_bit::READ),
        Ok(true)
    );
    assert_eq!(
        host::interpret(&ir, &caller, &record_miss, action_bit::READ),
        Ok(false)
    );
}

#[test]
fn missing_path_segment_returns_null() {
    // rule bytes: LOAD_CALLER [region], LIT_STR "us-east-1", EQ, RESULT
    let mut rule: Vec<u8> = Vec::new();
    rule.push(op::LOAD_CALLER);
    rule.push(1); // n_seg = 1
    rule.extend_from_slice(&6u16.to_le_bytes());
    rule.extend_from_slice(b"region");
    rule.push(op::LIT_STR);
    rule.extend_from_slice(&9u16.to_le_bytes());
    rule.extend_from_slice(b"us-east-1");
    rule.push(op::EQ);
    rule.push(op::RESULT);
    let ir = build_ir(&[(action_bit::READ, &rule)]);

    // Caller has no `region` field — path resolves to Null; Null == Str → false.
    let caller = caller_with_roles(&[]);
    let record = serde_json::json!({"id": "r", "tenant": ""});

    assert_eq!(
        host::interpret(&ir, &caller, &record, action_bit::READ),
        Ok(false)
    );
}

#[test]
fn not_in_returns_true_when_absent() {
    // rule bytes: LIT_STR "u9", LOAD_SELF [members], NOT_IN, RESULT
    let mut rule: Vec<u8> = Vec::new();
    rule.push(op::LIT_STR);
    rule.extend_from_slice(&2u16.to_le_bytes());
    rule.extend_from_slice(b"u9");
    rule.push(op::LOAD_SELF);
    rule.push(1); // n_seg = 1
    rule.extend_from_slice(&7u16.to_le_bytes());
    rule.extend_from_slice(b"members");
    rule.push(op::NOT_IN);
    rule.push(op::RESULT);
    let ir = build_ir(&[(action_bit::READ, &rule)]);

    let caller = caller_with_roles(&[]);
    let record_absent = serde_json::json!({
        "id": "r", "tenant": "",
        "members": ["u1", "u2", "u3"]
    });
    let record_present = serde_json::json!({
        "id": "r", "tenant": "",
        "members": ["u1", "u9", "u3"]
    });

    assert_eq!(
        host::interpret(&ir, &caller, &record_absent, action_bit::READ),
        Ok(true)
    );
    assert_eq!(
        host::interpret(&ir, &caller, &record_present, action_bit::READ),
        Ok(false)
    );
}
