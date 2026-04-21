//! Host-side native Rust interpreter for the same IR that the wasm runtime
//! executes. Used by the CSL emitter's self-pass validation (Task C5.5)
//! and available to host-side tests that want to avoid spinning wasmtime
//! for every assertion.

use serde_json::Value;

use crate::{MAGIC, action_bit_for, op};

const MAX_STACK: usize = 32;

/// Errors surfaced by the native IR interpreter.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum IrError {
    /// IR does not start with the `"SLIR"` magic bytes.
    #[error("bad magic number")]
    BadMagic,
    /// The declared length header overruns the payload.
    #[error("length header exceeds payload")]
    BadLength,
    /// Encountered an unknown opcode during execution.
    #[error("unknown opcode {0:#04x}")]
    UnknownOpcode(u8),
    /// Pop from empty stack.
    #[error("stack underflow")]
    StackUnderflow,
    /// Push beyond the stack depth cap.
    #[error("stack overflow")]
    StackOverflow,
    /// Operand types don't match what the opcode expects.
    #[error("type mismatch")]
    TypeMismatch,
    /// Ran past the end of the bytecode while decoding an operand.
    #[error("unexpected end of bytecode")]
    UnexpectedEof,
    /// Wire action string does not map to a known action bit.
    #[error("unknown action")]
    UnknownAction,
}

#[derive(Clone, Debug)]
enum NativeVal<'a> {
    Null,
    Bool(bool),
    I64(i64),
    F64(f64),
    Str(&'a str),
    Array(&'a Vec<Value>),
}

/// Interpret an IR against a caller + record + action. Returns `Ok(true)`
/// for allow, `Ok(false)` for deny.
///
/// # Errors
///
/// Returns [`IrError`] for any malformed IR or type mismatch.
pub fn interpret(
    ir_full: &[u8],
    caller: &Value,
    record: &Value,
    action: u8,
) -> Result<bool, IrError> {
    if ir_full.len() < 8 || ir_full[0..4] != MAGIC {
        return Err(IrError::BadMagic);
    }
    let declared_len =
        u32::from_le_bytes([ir_full[4], ir_full[5], ir_full[6], ir_full[7]]) as usize;
    if declared_len + 8 > ir_full.len() {
        return Err(IrError::BadLength);
    }
    let ir = &ir_full[8..8 + declared_len];

    if ir.is_empty() {
        return Ok(false);
    }
    let count = ir[0] as usize;
    if count == 0 {
        return Ok(false);
    }
    let table_end = 1 + count * 3;
    if ir.len() < table_end {
        return Err(IrError::BadLength);
    }

    let mut entry: Option<usize> = None;
    for i in 0..count {
        let off = 1 + i * 3;
        let mask = ir[off];
        if mask & action != 0 {
            let rel = u16::from_le_bytes([ir[off + 1], ir[off + 2]]) as usize;
            entry = Some(table_end + rel);
            break;
        }
    }
    let Some(ip) = entry else {
        return Ok(false);
    };

    run(ir, ip, caller, record)
}

/// Host-side helper: take a wire action string and produce the bit used by
/// [`interpret`].
///
/// # Errors
///
/// Returns [`IrError::UnknownAction`] if `name` is not one of
/// `"read" | "list" | "write" | "delete"`.
pub fn action_from_wire(name: &str) -> Result<u8, IrError> {
    action_bit_for(name.as_bytes()).ok_or(IrError::UnknownAction)
}

fn run<'a>(
    ir: &'a [u8],
    mut ip: usize,
    caller: &'a Value,
    record: &'a Value,
) -> Result<bool, IrError> {
    let mut stack: Vec<NativeVal<'a>> = Vec::with_capacity(MAX_STACK);

    macro_rules! pop {
        () => {
            stack.pop().ok_or(IrError::StackUnderflow)?
        };
    }
    macro_rules! push {
        ($v:expr) => {{
            if stack.len() >= MAX_STACK {
                return Err(IrError::StackOverflow);
            }
            stack.push($v);
        }};
    }

    loop {
        let tag = *ir.get(ip).ok_or(IrError::UnexpectedEof)?;
        ip += 1;
        match tag {
            op::LIT_NULL => push!(NativeVal::Null),
            op::LIT_BOOL => {
                let b = *ir.get(ip).ok_or(IrError::UnexpectedEof)?;
                ip += 1;
                push!(NativeVal::Bool(b != 0));
            }
            op::LIT_I64 => {
                let v = read_i64(ir, &mut ip)?;
                push!(NativeVal::I64(v));
            }
            op::LIT_F64 => {
                let v = read_f64(ir, &mut ip)?;
                push!(NativeVal::F64(v));
            }
            op::LIT_STR => {
                let len = read_u16(ir, &mut ip)? as usize;
                let end = ip + len;
                let bytes = ir.get(ip..end).ok_or(IrError::UnexpectedEof)?;
                ip = end;
                let s = core::str::from_utf8(bytes).map_err(|_| IrError::TypeMismatch)?;
                push!(NativeVal::Str(s));
            }
            op::LIT_DURATION_SECS => {
                let v = read_i64(ir, &mut ip)?;
                push!(NativeVal::I64(v));
            }
            op::LOAD_CALLER => push!(load_path(ir, &mut ip, caller)?),
            op::LOAD_SELF => push!(load_path(ir, &mut ip, record)?),
            op::EQ | op::NE => {
                let b = pop!();
                let a = pop!();
                let eq = eq_native(&a, &b);
                push!(NativeVal::Bool(if tag == op::EQ { eq } else { !eq }));
            }
            op::LT | op::LE | op::GT | op::GE => {
                let b = pop!();
                let a = pop!();
                let (x, y) = match (&a, &b) {
                    (NativeVal::I64(x), NativeVal::I64(y)) => (*x as f64, *y as f64),
                    (NativeVal::F64(x), NativeVal::F64(y)) => (*x, *y),
                    (NativeVal::I64(x), NativeVal::F64(y)) => (*x as f64, *y),
                    (NativeVal::F64(x), NativeVal::I64(y)) => (*x, *y as f64),
                    _ => return Err(IrError::TypeMismatch),
                };
                let r = match tag {
                    op::LT => x < y,
                    op::LE => x <= y,
                    op::GT => x > y,
                    _ => x >= y,
                };
                push!(NativeVal::Bool(r));
            }
            op::AND => {
                let b = pop_bool(&mut stack)?;
                let a = pop_bool(&mut stack)?;
                push!(NativeVal::Bool(a && b));
            }
            op::OR => {
                let b = pop_bool(&mut stack)?;
                let a = pop_bool(&mut stack)?;
                push!(NativeVal::Bool(a || b));
            }
            op::NOT => {
                let a = pop_bool(&mut stack)?;
                push!(NativeVal::Bool(!a));
            }
            op::IN | op::NOT_IN => {
                let haystack = pop!();
                let needle = pop!();
                let NativeVal::Array(arr) = haystack else {
                    return Err(IrError::TypeMismatch);
                };
                let found = arr.iter().any(|v| {
                    let elem = native_of(v);
                    eq_native(&needle, &elem)
                });
                push!(NativeVal::Bool(if tag == op::IN { found } else { !found }));
            }
            op::CALL_HAS_ROLE => {
                let role = pop!();
                let _caller_val = pop!();
                let role_str = match role {
                    NativeVal::Str(s) => s,
                    _ => return Err(IrError::TypeMismatch),
                };
                let matched = caller
                    .pointer("/roles")
                    .and_then(|v| v.as_array())
                    .is_some_and(|arr| arr.iter().any(|r| r.as_str() == Some(role_str)));
                push!(NativeVal::Bool(matched));
            }
            op::CALL_TENANT_MATCH => {
                let _rhs = pop!();
                let _lhs = pop!();
                let ct = caller.pointer("/tenant").and_then(|v| v.as_str());
                let st = record.pointer("/tenant").and_then(|v| v.as_str());
                let m = matches!((ct, st), (Some(a), Some(b)) if a == b);
                push!(NativeVal::Bool(m));
            }
            op::RESULT => {
                let b = pop_bool(&mut stack)?;
                return Ok(b);
            }
            op::DENY => return Ok(false),
            op::ALLOW => return Ok(true),
            other => return Err(IrError::UnknownOpcode(other)),
        }
    }
}

fn read_u16(ir: &[u8], ip: &mut usize) -> Result<u16, IrError> {
    let b0 = *ir.get(*ip).ok_or(IrError::UnexpectedEof)?;
    let b1 = *ir.get(*ip + 1).ok_or(IrError::UnexpectedEof)?;
    *ip += 2;
    Ok(u16::from_le_bytes([b0, b1]))
}

fn read_i64(ir: &[u8], ip: &mut usize) -> Result<i64, IrError> {
    let mut buf = [0u8; 8];
    for b in &mut buf {
        *b = *ir.get(*ip).ok_or(IrError::UnexpectedEof)?;
        *ip += 1;
    }
    Ok(i64::from_le_bytes(buf))
}

fn read_f64(ir: &[u8], ip: &mut usize) -> Result<f64, IrError> {
    let mut buf = [0u8; 8];
    for b in &mut buf {
        *b = *ir.get(*ip).ok_or(IrError::UnexpectedEof)?;
        *ip += 1;
    }
    Ok(f64::from_le_bytes(buf))
}

fn pop_bool<'a>(stack: &mut Vec<NativeVal<'a>>) -> Result<bool, IrError> {
    match stack.pop().ok_or(IrError::StackUnderflow)? {
        NativeVal::Bool(b) => Ok(b),
        _ => Err(IrError::TypeMismatch),
    }
}

fn load_path<'a>(ir: &'a [u8], ip: &mut usize, root: &'a Value) -> Result<NativeVal<'a>, IrError> {
    let n = *ir.get(*ip).ok_or(IrError::UnexpectedEof)?;
    *ip += 1;
    let mut cursor = root;
    for _ in 0..n {
        let len = read_u16(ir, ip)? as usize;
        let end = *ip + len;
        let seg = core::str::from_utf8(ir.get(*ip..end).ok_or(IrError::UnexpectedEof)?)
            .map_err(|_| IrError::TypeMismatch)?;
        *ip = end;
        cursor = match cursor.get(seg) {
            Some(v) => v,
            None => return Ok(NativeVal::Null),
        };
    }
    Ok(native_of(cursor))
}

fn native_of(v: &Value) -> NativeVal<'_> {
    match v {
        Value::Null => NativeVal::Null,
        Value::Bool(b) => NativeVal::Bool(*b),
        Value::Number(n) => n
            .as_i64()
            .map(NativeVal::I64)
            .or_else(|| n.as_f64().map(NativeVal::F64))
            .unwrap_or(NativeVal::Null),
        Value::String(s) => NativeVal::Str(s.as_str()),
        Value::Array(a) => NativeVal::Array(a),
        Value::Object(_) => NativeVal::Null,
    }
}

fn eq_native(a: &NativeVal<'_>, b: &NativeVal<'_>) -> bool {
    match (a, b) {
        (NativeVal::Null, NativeVal::Null) => true,
        (NativeVal::Bool(x), NativeVal::Bool(y)) => x == y,
        (NativeVal::I64(x), NativeVal::I64(y)) => x == y,
        (NativeVal::F64(x), NativeVal::F64(y)) => x == y,
        (NativeVal::I64(x), NativeVal::F64(y)) | (NativeVal::F64(y), NativeVal::I64(x)) => {
            (*x as f64) == *y
        }
        (NativeVal::Str(x), NativeVal::Str(y)) => x == y,
        _ => false,
    }
}
