//! Host-side native Rust interpreter for the same IR that the wasm runtime
//! executes. Used by the CSL emitter's self-pass validation (Task C5.5)
//! and available to host-side tests that want to avoid spinning wasmtime
//! for every assertion.
//!
//! Populated in Task C5.5. For now, this module exposes only the error
//! type so dependent code can name it.

use serde_json::Value;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IrError {
    #[error("bad magic number")]
    BadMagic,
    #[error("length header exceeds payload")]
    BadLength,
    #[error("unknown opcode {0:#04x}")]
    UnknownOpcode(u8),
    #[error("stack underflow")]
    StackUnderflow,
    #[error("stack overflow")]
    StackOverflow,
    #[error("type mismatch")]
    TypeMismatch,
    #[error("unexpected end of bytecode")]
    UnexpectedEof,
    #[error("unknown action")]
    UnknownAction,
}

/// Interpret an IR against a caller + record + action. Returns `Ok(true)`
/// for allow, `Ok(false)` for deny. Populated in Task C5.5.
///
/// # Errors
/// Returns [`IrError`] for any malformed IR or type mismatch.
pub fn interpret(
    _ir: &[u8],
    _caller: &Value,
    _record: &Value,
    _action: u8,
) -> Result<bool, IrError> {
    // Body lands in Task C5.5.
    Err(IrError::UnknownOpcode(0))
}
