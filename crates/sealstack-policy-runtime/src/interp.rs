//! Predicate IR interpreter. Executes straight-line bytecode against a
//! `PolicyInputWire` JSON buffer and returns {allow=1, deny=0, error=-1}.

#![allow(dead_code)]

use crate::json;

const MAX_STACK: usize = 32;

#[derive(Clone, Copy)]
pub(crate) enum Val<'a> {
    Null,
    Bool(bool),
    I64(i64),
    F64(f64),
    /// Slice into the original input JSON bytes. May be a string, number, or anything else.
    Raw(&'a [u8]),
    /// Start index of a JSON array in the input buffer.
    ArrayAt(usize),
}

pub(crate) struct Interp<'a> {
    ir: &'a [u8],
    ip: usize,
    stack: [Option<Val<'a>>; MAX_STACK],
    sp: usize,
    input: &'a [u8],
    /// Offset of the `caller` value in `input`, or usize::MAX if missing.
    caller_at: usize,
    /// Offset of the `record` (alias `self`) value in `input`.
    self_at: usize,
}

pub(crate) enum Verdict {
    Allow,
    Deny,
    Error,
}

pub(crate) fn evaluate(input: &[u8], ir_full: &[u8]) -> Verdict {
    use sealstack_policy_ir::{action_bit_for, MAGIC};

    if ir_full.len() < 8 || ir_full[0..4] != MAGIC {
        return Verdict::Error;
    }
    let declared_len =
        u32::from_le_bytes([ir_full[4], ir_full[5], ir_full[6], ir_full[7]]) as usize;
    if declared_len + 8 > ir_full.len() {
        return Verdict::Error;
    }
    let ir = &ir_full[8..8 + declared_len];

    let caller_at = match json::find_path(input, &[b"caller"]) {
        Ok(Some((start, _))) => start,
        _ => usize::MAX,
    };
    let self_at = match json::find_path(input, &[b"record"]) {
        Ok(Some((start, _))) => start,
        _ => usize::MAX,
    };
    let action = match json::find_path(input, &[b"action"]) {
        Ok(Some((start, _))) => match json::as_str(input, start) {
            Ok(bytes) => bytes,
            Err(_) => return Verdict::Error,
        },
        _ => return Verdict::Error,
    };

    if ir.is_empty() {
        return Verdict::Deny;
    }

    // Action table layout:
    //   byte 0: action_table_count (u8)
    //   next 3*count bytes: { action_mask: u8, offset: u16 LE }
    let count = ir[0] as usize;
    if count == 0 {
        return Verdict::Deny;
    }
    let table_end = 1 + count * 3;
    if ir.len() < table_end {
        return Verdict::Error;
    }

    let action_bit = match action_bit_for(action) {
        Some(bit) => bit,
        None => return Verdict::Error,
    };

    let mut rule_entry: Option<usize> = None;
    for i in 0..count {
        let off = 1 + i * 3;
        let mask = ir[off];
        if mask & action_bit != 0 {
            let rel = u16::from_le_bytes([ir[off + 1], ir[off + 2]]) as usize;
            rule_entry = Some(table_end + rel);
            break;
        }
    }

    let Some(entry) = rule_entry else {
        return Verdict::Deny;
    };

    if entry >= ir.len() {
        return Verdict::Error;
    }

    let mut interp = Interp {
        ir,
        ip: entry,
        stack: [None; MAX_STACK],
        sp: 0,
        input,
        caller_at,
        self_at,
    };
    match interp.run() {
        Ok(true) => Verdict::Allow,
        Ok(false) => Verdict::Deny,
        Err(()) => Verdict::Error,
    }
}

impl<'a> Interp<'a> {
    pub(crate) fn run(&mut self) -> Result<bool, ()> {
        // Opcode handlers land in Task B5; for now this skeleton
        // just handles terminal ALLOW / DENY so the end-to-end dispatch path
        // can be smoke-tested.
        loop {
            let op = *self.ir.get(self.ip).ok_or(())?;
            self.ip += 1;
            match op {
                0xFE => return Ok(false), // DENY
                0xFF => return Ok(true),  // ALLOW
                _ => return Err(()),      // unimplemented opcodes land in B5
            }
        }
    }
}
