use sealstack_csl::codegen::policy;
use sealstack_csl::{parser, types};

#[test]
fn bundle_contains_expected_patched_ir_bytes() {
    let src = r#"
        schema Doc {
            id: Ulid @primary
            policy { read: true }
        }
    "#;
    let file = parser::parse_file(src).unwrap();
    let typed = types::check(&file).unwrap();
    let bundles = policy::emit_policy_bundles(&typed).unwrap();
    assert_eq!(bundles.len(), 1);
    let b = &bundles[0];
    assert_eq!(&b.wasm[0..4], b"\0asm", "not a wasm file");

    // Find SLIR and verify the full expected IR follows it.
    let pos = b
        .wasm
        .windows(4)
        .position(|w| w == b"SLIR")
        .expect("SLIR magic not found");

    // Expected rule bytecode for `read: true`:
    //   action_table_count = 1
    //   entry: mask = 0x01 (READ), offset = 0
    //   rule body: LIT_BOOL, 1, RESULT
    let expected_header_and_rule: &[u8] = &[
        b'S', b'L', b'I', b'R',
        // ir_len: u32 LE = 7 (count byte + 3-byte entry + 3-byte rule)
        0x07, 0x00, 0x00, 0x00,
        // action_table_count
        0x01,
        // entry: mask=READ(0x01), offset=0 LE
        0x01, 0x00, 0x00,
        // rule: LIT_BOOL(0x02), 1, RESULT(0xFD)
        0x02, 0x01, 0xFD,
    ];

    let actual = &b.wasm[pos..pos + expected_header_and_rule.len()];
    assert_eq!(
        actual, expected_header_and_rule,
        "patched IR bytes don't match expected content at offset {pos}",
    );
}
