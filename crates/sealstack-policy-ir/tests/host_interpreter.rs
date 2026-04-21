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
