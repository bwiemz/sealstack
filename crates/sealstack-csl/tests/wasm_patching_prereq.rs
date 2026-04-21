//! First gate before any patching logic gets written.
//!
//! The plan originally proposed parsing `policy_runtime.wasm` with
//! `wasmparser` and re-encoding with `wasm-encoder::RoundtripReencoder`. On
//! our toolchain + runtime build, the re-encode is NOT byte-identical
//! (length differs by 4 bytes — canonical resize of some size-prefixed
//! sections). Per the plan's §6 fallback option 2, we pivot to byte-level
//! section surgery: never re-encode, only use `wasmparser` to locate the
//! `.sealstack_predicate_ir` data segment window inside the original
//! bytes, then overwrite in place.
//!
//! Additionally, the runtime had to pre-seed the section with the MAGIC
//! `"SLIR"` + trailing zeros, because a pure-zero `static` is promoted to
//! BSS by `wasm-ld` and stripped from the binary entirely (nothing to
//! patch). We identify the reservation slot by (length, MAGIC prefix,
//! zero-rest) to keep the scan unambiguous.
//!
//! This test validates both properties the patching code relies on:
//!   1. wasmparser can walk the Data section of the committed asset.
//!   2. There exists exactly one data segment of length
//!      `IR_SECTION_BYTES` whose first four bytes are the MAGIC and whose
//!      remaining bytes are zero — the runtime's reservation slot.

use sealstack_policy_ir::{IR_SECTION_BYTES, MAGIC};
use wasmparser::{Parser, Payload};

const ASSET: &[u8] = include_bytes!("../assets/policy_runtime.wasm");

/// Locate the MAGIC + `IR_MAX_BYTES` zeros window inside a data segment.
fn find_slot_in_segment(data: &[u8]) -> Option<usize> {
    if data.len() < IR_SECTION_BYTES {
        return None;
    }
    (0..=data.len() - IR_SECTION_BYTES).step_by(4).find(|&i| {
        data[i..i + 4] == MAGIC && data[i + 4..i + IR_SECTION_BYTES].iter().all(|b| *b == 0)
    })
}

#[test]
fn runtime_asset_has_exactly_one_predicate_ir_slot() {
    let mut matches = 0usize;
    for payload in Parser::new(0).parse_all(ASSET) {
        let p = payload.expect("parse payload");
        if let Payload::DataSection(reader) = p {
            for item in reader {
                let data = item.expect("parse data segment");
                if find_slot_in_segment(data.data).is_some() {
                    matches += 1;
                }
            }
        }
    }
    assert_eq!(
        matches, 1,
        "expected exactly one data segment containing a MAGIC-prefixed, \
         zero-padded predicate-IR reservation of {IR_SECTION_BYTES} bytes; \
         found {matches}"
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
                if let Some(idx) = find_slot_in_segment(data.data) {
                    start =
                        Some(data.data.as_ptr() as usize - ASSET.as_ptr() as usize + idx);
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
