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

#[panic_handler]
fn on_panic(_info: &core::panic::PanicInfo) -> ! {
    // In a no-std wasm build, panics abort the module. No formatting, no alloc.
    core::arch::wasm32::unreachable()
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
    unsafe {
        let p = BUMP;
        BUMP = BUMP.saturating_add(n as usize);
        p as i32
    }
}

/// Entry point from the host.
///
/// Phase B2–B10 replace the body with real logic. For now we return deny (0)
/// so the crate compiles to wasm without pulling in any IR or JSON machinery.
#[unsafe(no_mangle)]
pub extern "C" fn sealstack_evaluate(_ptr: i32, _len: i32) -> i32 {
    0
}
