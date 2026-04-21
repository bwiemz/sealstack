//! Shared invariants for the CSL policy IR. Opcode tags, action-mask bit
//! positions, data-section layout constants, and (under the `host` feature)
//! a native-Rust interpreter used for self-pass validation by the CSL
//! emitter and for host-side testing of the same IR shape.
//!
//! This crate exists to eliminate a class of bugs where the wasm runtime
//! and the emitter agree on opcode values by coincidence and drift apart
//! silently. Any new opcode or mask bit lands here first.

#![cfg_attr(not(feature = "host"), no_std)]
#![forbid(unsafe_code)]

// ---------------------------------------------------------------------------
// Data-section layout
// ---------------------------------------------------------------------------

/// Upper bound on IR payload size (excluding magic + length header).
pub const IR_MAX_BYTES: usize = 4096;

/// Full data-section footprint: magic (4) + length (4) + payload.
pub const IR_SECTION_BYTES: usize = 8 + IR_MAX_BYTES;

/// Magic number stamped at offset 0 of every well-formed IR section.
pub const MAGIC: [u8; 4] = *b"SLIR";

// ---------------------------------------------------------------------------
// Action masks
// ---------------------------------------------------------------------------

/// Bit position within an action_mask byte. These are the **single source of
/// truth**; both the emitter and the runtime consume this module.
pub mod action_bit {
    pub const READ: u8 = 0b0000_0001;
    pub const LIST: u8 = 0b0000_0010;
    pub const WRITE: u8 = 0b0000_0100;
    pub const DELETE: u8 = 0b0000_1000;
}

/// Resolve an action wire-string to its bit. Unknown actions return `None`,
/// which the runtime surfaces as `-1` and the host surfaces as an IR error.
#[must_use]
pub fn action_bit_for(name: &[u8]) -> Option<u8> {
    match name {
        b"read" => Some(action_bit::READ),
        b"list" => Some(action_bit::LIST),
        b"write" => Some(action_bit::WRITE),
        b"delete" => Some(action_bit::DELETE),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Opcodes
// ---------------------------------------------------------------------------

pub mod op {
    // Literals
    pub const LIT_NULL: u8 = 0x01;
    pub const LIT_BOOL: u8 = 0x02;
    pub const LIT_I64: u8 = 0x03;
    pub const LIT_F64: u8 = 0x04;
    pub const LIT_STR: u8 = 0x05;
    pub const LIT_DURATION_SECS: u8 = 0x06;
    // Loads
    pub const LOAD_CALLER: u8 = 0x10;
    pub const LOAD_SELF: u8 = 0x11;
    // Comparisons
    pub const EQ: u8 = 0x20;
    pub const NE: u8 = 0x21;
    pub const LT: u8 = 0x22;
    pub const LE: u8 = 0x23;
    pub const GT: u8 = 0x24;
    pub const GE: u8 = 0x25;
    // Logical
    pub const AND: u8 = 0x30;
    pub const OR: u8 = 0x31;
    pub const NOT: u8 = 0x32;
    // Set membership
    pub const IN: u8 = 0x40;
    pub const NOT_IN: u8 = 0x41;
    // Calls
    pub const CALL_HAS_ROLE: u8 = 0x50;
    pub const CALL_TENANT_MATCH: u8 = 0x51;
    // Terminals
    pub const RESULT: u8 = 0xFD;
    pub const DENY: u8 = 0xFE;
    pub const ALLOW: u8 = 0xFF;
}

// ---------------------------------------------------------------------------
// Host-side native interpreter (feature-gated; not compiled into wasm)
// ---------------------------------------------------------------------------

#[cfg(feature = "host")]
pub mod host;
