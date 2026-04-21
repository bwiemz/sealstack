//! CSL policy-predicate WASM runtime. Built once per release and committed to
//! `crates/sealstack-csl/assets/policy_runtime.wasm`. The CSL compiler patches
//! the `.sealstack_predicate_ir` data section at `sealstack compile` time.
//!
//! ABI contract — mirrors `sealstack_engine::policy::WasmPolicy` expectations:
//! * `memory` — default linear memory.
//! * `sealstack_alloc(n: i32) -> i32` — bump allocator, returns offset.
//! * `sealstack_evaluate(ptr: i32, len: i32) -> i32` — 1 allow, 0 deny, -1 error.

#![no_std]
#![no_main]

mod json;
mod interp;

#[panic_handler]
fn on_panic(_info: &core::panic::PanicInfo) -> ! {
    // In a no-std wasm build, panics abort the module. No formatting, no alloc.
    core::arch::wasm32::unreachable()
}

use sealstack_policy_ir::{IR_MAX_BYTES, IR_SECTION_BYTES, MAGIC};

/// Predicate IR, stamped in place by the CSL compiler.
///
/// Layout (little-endian where applicable):
///
/// * bytes 0..4:    magic number `"SLIR"`
/// * bytes 4..8:    u32 — payload length in bytes (excluding magic+length)
/// * bytes 8..:     `payload_length` bytes of IR; remaining bytes are zero padding
#[used]
#[unsafe(link_section = ".sealstack_predicate_ir")]
pub static PREDICATE_IR: [u8; IR_SECTION_BYTES] = [0; IR_SECTION_BYTES];

#[allow(dead_code)]
fn ir_payload() -> Option<&'static [u8]> {
    let section = &PREDICATE_IR;
    if section[0..4] != MAGIC {
        return None;
    }
    let len = u32::from_le_bytes([section[4], section[5], section[6], section[7]]) as usize;
    if len > IR_MAX_BYTES {
        return None;
    }
    Some(&section[8..8 + len])
}

// Bump allocator backing `sealstack_alloc`. Coarse but fine for a single
// evaluation pass per instance.
static mut BUMP: usize = 1024;

/// Allocate `n` bytes inside linear memory, returning the offset.
///
/// # Safety
/// The host is expected to treat the returned offset as the start of a
/// `memory.write` region and nothing else touches `BUMP` across calls.
#[unsafe(no_mangle)]
pub extern "C" fn sealstack_alloc(n: i32) -> i32 {
    if n < 0 {
        return -1;
    }
    unsafe {
        let new_bump = match BUMP.checked_add(n as usize) {
            Some(v) => v,
            None => return -1,
        };
        if new_bump > i32::MAX as usize {
            return -1;
        }
        let p = BUMP;
        BUMP = new_bump;
        p as i32
    }
}

/// Entry point from the host.
#[unsafe(no_mangle)]
pub extern "C" fn sealstack_evaluate(ptr: i32, len: i32) -> i32 {
    if ptr < 0 || len < 0 {
        return -1;
    }
    let input = unsafe {
        core::slice::from_raw_parts(ptr as usize as *const u8, len as usize)
    };
    let ir_section = &PREDICATE_IR;
    match interp::evaluate(input, ir_section) {
        interp::Verdict::Allow => 1,
        interp::Verdict::Deny => 0,
        interp::Verdict::Error => -1,
    }
}
