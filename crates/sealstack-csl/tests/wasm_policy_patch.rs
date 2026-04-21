use sealstack_csl::codegen::policy;
use sealstack_csl::{parser, types};

#[test]
fn bundle_starts_with_valid_wasm_header_and_embeds_slir() {
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
    // SLIR magic must appear somewhere in the bytes (patched into data section).
    let has_slir = b.wasm.windows(4).any(|w| w == b"SLIR");
    assert!(has_slir, "SLIR magic not found in patched wasm");
}
