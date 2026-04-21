//! First gate before any patching logic gets written.
//!
//! The plan proposes parsing `policy_runtime.wasm` with `wasmparser` and
//! re-encoding with `wasm-encoder::RoundtripReencoder`. On our toolchain +
//! runtime build, the re-encode is NOT byte-identical (length differs by 4
//! bytes — the reencoder re-serializes some size-prefixed sections in a
//! canonical form that differs from what `rustc`/`lld` emitted).
//!
//! Per the plan's §6 risk fallback list (option 2): switch to byte-level
//! section surgery — never run the re-encoder, never reserialize sections.
//! Use `wasmparser` only to locate the `.sealstack_predicate_ir` data
//! segment inside the original bytes, then overwrite that window in place.
//!
//! This test validates both properties the patching code relies on:
//!   1. wasmparser can walk the Data section of the committed asset.
//!   2. There exists exactly one data segment of length `IR_SECTION_BYTES`
//!      whose initial contents are all-zero — the runtime's reservation
//!      slot.
//! If either property breaks in the future (e.g. the runtime adds another
//! all-zero blob of the same size), this test surfaces it up-front.

use sealstack_policy_ir::IR_SECTION_BYTES;
use wasmparser::{Parser, Payload};

const ASSET: &[u8] = include_bytes!("../assets/policy_runtime.wasm");

#[test]
fn runtime_asset_has_exactly_one_predicate_ir_slot() {
    let mut matches = 0usize;
    for payload in Parser::new(0).parse_all(ASSET) {
        let p = payload.expect("parse payload");
        if let Payload::DataSection(reader) = p {
            for item in reader {
                let data = item.expect("parse data segment");
                if data.data.len() == IR_SECTION_BYTES && data.data.iter().all(|b| *b == 0) {
                    matches += 1;
                }
            }
        }
    }
    assert_eq!(
        matches, 1,
        "expected exactly one all-zero data segment of length {IR_SECTION_BYTES} (the \
         .sealstack_predicate_ir reservation); found {matches}"
    );
}

#[test]
fn byte_level_surgery_preserves_all_other_bytes() {
    // Locate the predicate-IR segment's offset within the wasm bytes.
    let mut start: Option<usize> = None;
    for payload in Parser::new(0).parse_all(ASSET) {
        let p = payload.expect("parse payload");
        if let Payload::DataSection(reader) = p {
            for item in reader {
                let data = item.expect("parse data segment");
                if data.data.len() == IR_SECTION_BYTES && data.data.iter().all(|b| *b == 0) {
                    start =
                        Some(data.data.as_ptr() as usize - ASSET.as_ptr() as usize);
                }
            }
        }
    }
    let start = start.expect("predicate IR slot");

    // Overwrite the slot with a recognizable pattern, verify every other byte
    // is untouched.
    let mut patched = ASSET.to_vec();
    for (i, b) in patched[start..start + IR_SECTION_BYTES].iter_mut().enumerate() {
        *b = (i as u8).wrapping_add(1);
    }
    assert_eq!(patched.len(), ASSET.len(), "patching must not change length");
    assert_eq!(
        &patched[..start],
        &ASSET[..start],
        "bytes before the patch window must be identical"
    );
    assert_eq!(
        &patched[start + IR_SECTION_BYTES..],
        &ASSET[start + IR_SECTION_BYTES..],
        "bytes after the patch window must be identical"
    );
}
