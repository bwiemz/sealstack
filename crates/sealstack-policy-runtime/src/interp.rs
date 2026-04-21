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

enum Ord {
    Lt,
    Le,
    Gt,
    Ge,
}

mod heapless_u8_path {
    const MAX_SEGS: usize = 8;
    pub(crate) struct PathBuf<'a> {
        segs: [Option<&'a [u8]>; MAX_SEGS],
        n: usize,
    }
    impl<'a> PathBuf<'a> {
        pub(crate) fn new() -> Self {
            Self { segs: [None; MAX_SEGS], n: 0 }
        }
        pub(crate) fn push(&mut self, s: &'a [u8]) -> Result<(), ()> {
            if self.n >= MAX_SEGS {
                return Err(());
            }
            self.segs[self.n] = Some(s);
            self.n += 1;
            Ok(())
        }
        pub(crate) fn iter(&self) -> impl Iterator<Item = &'a [u8]> + '_ {
            self.segs[..self.n].iter().filter_map(|x| *x)
        }
    }
}

pub(crate) fn evaluate(input: &[u8], ir_full: &[u8]) -> Verdict {
    use sealstack_policy_ir::{action_bit_for, MAGIC};

    if ir_full.len() < 8 || ir_full[0..4] != MAGIC {
        return Verdict::Error;
    }
    let declared_len =
        u32::from_le_bytes([ir_full[4], ir_full[5], ir_full[6], ir_full[7]]) as usize;
    // `declared_len` comes from untrusted bundle bytes. On wasm32 `usize` is
    // 32 bits, so `declared_len + 8` can wrap for a hostile length close to
    // u32::MAX and bypass the guard, causing the slice below to trap. Use
    // checked_add to catch the overflow.
    let Some(total) = declared_len.checked_add(8) else {
        return Verdict::Error;
    };
    if total > ir_full.len() {
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
        loop {
            let op = *self.ir.get(self.ip).ok_or(())?;
            self.ip += 1;
            match op {
                // Literals
                0x01 => self.push(Val::Null)?,
                0x02 => {
                    let b = *self.ir.get(self.ip).ok_or(())?;
                    self.ip += 1;
                    self.push(Val::Bool(b != 0))?;
                }
                0x03 => {
                    let v = self.read_i64()?;
                    self.push(Val::I64(v))?;
                }
                0x04 => {
                    let v = self.read_f64()?;
                    self.push(Val::F64(v))?;
                }
                0x05 => {
                    let len = self.read_u16()? as usize;
                    let end = self.ip + len;
                    let slice = self.ir.get(self.ip..end).ok_or(())?;
                    self.ip = end;
                    self.push(Val::Raw(slice))?;
                }
                0x06 => {
                    let v = self.read_i64()?;
                    self.push(Val::I64(v))?;
                }
                // Loads
                0x10 => self.load_path(/* from_caller */ true)?,
                0x11 => self.load_path(/* from_caller */ false)?,
                // Comparisons
                0x20 => self.cmp_eq(false)?,
                0x21 => self.cmp_eq(true)?,
                0x22 => self.cmp_ord(Ord::Lt)?,
                0x23 => self.cmp_ord(Ord::Le)?,
                0x24 => self.cmp_ord(Ord::Gt)?,
                0x25 => self.cmp_ord(Ord::Ge)?,
                // Logical
                0x30 => self.logic_and()?,
                0x31 => self.logic_or()?,
                0x32 => {
                    let a = self.pop_bool()?;
                    self.push(Val::Bool(!a))?;
                }
                // Set membership
                0x40 => self.in_op(false)?,
                0x41 => self.in_op(true)?,
                // Calls
                0x50 => self.call_has_role()?,
                0x51 => self.call_tenant_match()?,
                // Terminals
                0xFD => {
                    let b = self.pop_bool()?;
                    return Ok(b);
                }
                0xFE => return Ok(false),
                0xFF => return Ok(true),
                _ => return Err(()),
            }
        }
    }

    fn push(&mut self, v: Val<'a>) -> Result<(), ()> {
        if self.sp >= MAX_STACK {
            return Err(());
        }
        self.stack[self.sp] = Some(v);
        self.sp += 1;
        Ok(())
    }

    fn pop(&mut self) -> Result<Val<'a>, ()> {
        if self.sp == 0 {
            return Err(());
        }
        self.sp -= 1;
        self.stack[self.sp].take().ok_or(())
    }

    fn pop_bool(&mut self) -> Result<bool, ()> {
        match self.pop()? {
            Val::Bool(b) => Ok(b),
            _ => Err(()),
        }
    }

    fn read_u16(&mut self) -> Result<u16, ()> {
        let b0 = *self.ir.get(self.ip).ok_or(())?;
        let b1 = *self.ir.get(self.ip + 1).ok_or(())?;
        self.ip += 2;
        Ok(u16::from_le_bytes([b0, b1]))
    }

    fn read_i64(&mut self) -> Result<i64, ()> {
        let mut buf = [0u8; 8];
        for b in &mut buf {
            *b = *self.ir.get(self.ip).ok_or(())?;
            self.ip += 1;
        }
        Ok(i64::from_le_bytes(buf))
    }

    fn read_f64(&mut self) -> Result<f64, ()> {
        let mut buf = [0u8; 8];
        for b in &mut buf {
            *b = *self.ir.get(self.ip).ok_or(())?;
            self.ip += 1;
        }
        Ok(f64::from_le_bytes(buf))
    }

    fn load_path(&mut self, from_caller: bool) -> Result<(), ()> {
        let nseg = *self.ir.get(self.ip).ok_or(())?;
        self.ip += 1;
        let mut segs: heapless_u8_path::PathBuf = heapless_u8_path::PathBuf::new();
        for _ in 0..(nseg as usize) {
            let len = self.read_u16()? as usize;
            let end = self.ip + len;
            let slice = self.ir.get(self.ip..end).ok_or(())?;
            self.ip = end;
            segs.push(slice)?;
        }

        let root_at = if from_caller { self.caller_at } else { self.self_at };
        if root_at == usize::MAX {
            self.push(Val::Null)?;
            return Ok(());
        }

        // Zero-segment load: push a sentinel representing "the root object
        // itself". The only opcodes that legitimately consume this (and know
        // what to do with it) are the built-in calls like CALL_HAS_ROLE,
        // which inspect `self.caller_at` / `self.self_at` directly rather
        // than the popped value. We represent the root with Val::Null to
        // distinguish from Val::Raw (scalar string) and Val::ArrayAt, both
        // of which the calls use as an "invalid" signal.
        if segs.iter().next().is_none() {
            self.push(Val::Null)?;
            return Ok(());
        }

        // Resolve path segments through the input JSON.
        let mut cursor = root_at;
        for seg in segs.iter() {
            match json::find_path(&self.input[cursor..], &[seg]) {
                Ok(Some((s, _))) => cursor += s,
                Ok(None) => {
                    self.push(Val::Null)?;
                    return Ok(());
                }
                Err(()) => return Err(()),
            }
        }

        // Turn the located slice into a Val. We look at the first non-ws byte.
        let start = skip_ws_fwd(self.input, cursor);
        let byte = *self.input.get(start).ok_or(())?;
        let v = match byte {
            b't' => Val::Bool(json::as_bool(self.input, start).map_err(|()| ())?),
            b'f' => Val::Bool(json::as_bool(self.input, start).map_err(|()| ())?),
            b'n' => Val::Null,
            b'"' => {
                let s = json::as_str(self.input, start).map_err(|()| ())?;
                Val::Raw(s)
            }
            b'[' => Val::ArrayAt(start),
            b'{' => Val::Null, // object reference: host treats as "use root"
            b'-' | b'0'..=b'9' => {
                // Prefer i64; fall back to f64.
                match json::as_i64(self.input, start) {
                    Ok(i) => Val::I64(i),
                    Err(()) => Val::F64(json::as_f64(self.input, start).map_err(|()| ())?),
                }
            }
            _ => return Err(()),
        };
        self.push(v)
    }

    fn cmp_eq(&mut self, invert: bool) -> Result<(), ()> {
        let b = self.pop()?;
        let a = self.pop()?;
        let eq = match (a, b) {
            (Val::Null, Val::Null) => true,
            (Val::Bool(x), Val::Bool(y)) => x == y,
            (Val::I64(x), Val::I64(y)) => x == y,
            (Val::F64(x), Val::F64(y)) => x == y,
            (Val::I64(x), Val::F64(y)) | (Val::F64(y), Val::I64(x)) => (x as f64) == y,
            (Val::Raw(x), Val::Raw(y)) => x == y,
            _ => false, // mixed types: not equal, not an error
        };
        self.push(Val::Bool(if invert { !eq } else { eq }))
    }

    fn cmp_ord(&mut self, op: Ord) -> Result<(), ()> {
        let b = self.pop()?;
        let a = self.pop()?;
        let (x, y) = match (a, b) {
            (Val::I64(x), Val::I64(y)) => (x as f64, y as f64),
            (Val::F64(x), Val::F64(y)) => (x, y),
            (Val::I64(x), Val::F64(y)) => (x as f64, y),
            (Val::F64(x), Val::I64(y)) => (x, y as f64),
            _ => return Err(()),
        };
        let r = match op {
            Ord::Lt => x < y,
            Ord::Le => x <= y,
            Ord::Gt => x > y,
            Ord::Ge => x >= y,
        };
        self.push(Val::Bool(r))
    }

    fn logic_and(&mut self) -> Result<(), ()> {
        let b = self.pop_bool()?;
        let a = self.pop_bool()?;
        self.push(Val::Bool(a && b))
    }

    fn logic_or(&mut self) -> Result<(), ()> {
        let b = self.pop_bool()?;
        let a = self.pop_bool()?;
        self.push(Val::Bool(a || b))
    }

    fn in_op(&mut self, invert: bool) -> Result<(), ()> {
        let haystack = self.pop()?;
        let needle = self.pop()?;
        let arr_at = match haystack {
            Val::ArrayAt(at) => at,
            _ => return Err(()),
        };

        let mut found = false;
        json::each_element(self.input, arr_at, |start, _end| {
            let start = skip_ws_fwd(self.input, start);
            let matches = match self.input.get(start).copied() {
                Some(b'"') => match (&needle, json::as_str(self.input, start)) {
                    (Val::Raw(n), Ok(s)) => *n == s,
                    _ => false,
                },
                Some(b'-') | Some(b'0'..=b'9') => match (&needle, json::as_i64(self.input, start)) {
                    (Val::I64(n), Ok(v)) => *n == v,
                    _ => false,
                },
                Some(b't') | Some(b'f') => match (&needle, json::as_bool(self.input, start)) {
                    (Val::Bool(n), Ok(v)) => *n == v,
                    _ => false,
                },
                _ => false,
            };
            if matches {
                found = true;
                false // early exit
            } else {
                true
            }
        })
        .map_err(|()| ())?;

        self.push(Val::Bool(if invert { !found } else { found }))
    }

    fn call_has_role(&mut self) -> Result<(), ()> {
        let role = self.pop()?;
        let caller = self.pop()?;
        let role_bytes = match role {
            Val::Raw(r) => r,
            _ => return Err(()),
        };
        let caller_at = match caller {
            Val::Raw(_) | Val::ArrayAt(_) => return Err(()),
            _ => {
                // The compiler always loads `caller` via LOAD_CALLER with an
                // empty path. In that case we read the roles directly off
                // self.caller_at.
                self.caller_at
            }
        };
        if caller_at == usize::MAX {
            self.push(Val::Bool(false))?;
            return Ok(());
        }
        let Ok(Some((roles_at, _))) =
            json::find_path(&self.input[caller_at..], &[b"roles"])
        else {
            self.push(Val::Bool(false))?;
            return Ok(());
        };

        // `roles_at` is relative to `self.input[caller_at..]`; rebasing to
        // `caller_at + roles_at` yields an absolute offset into `self.input`,
        // which is what `each_element` expects. (No parallel to the
        // `call_tenant_match` rebase bug.)
        let mut matched = false;
        json::each_element(self.input, caller_at + roles_at, |start, _end| {
            if let Ok(s) = json::as_str(self.input, skip_ws_fwd(self.input, start)) {
                if s == role_bytes {
                    matched = true;
                    return false;
                }
            }
            true
        })
        .map_err(|()| ())?;
        self.push(Val::Bool(matched))
    }

    fn call_tenant_match(&mut self) -> Result<(), ()> {
        let _rhs = self.pop()?;
        let _lhs = self.pop()?;
        if self.caller_at == usize::MAX || self.self_at == usize::MAX {
            self.push(Val::Bool(false))?;
            return Ok(());
        }
        let caller_tenant = json::find_path(&self.input[self.caller_at..], &[b"tenant"]);
        let self_tenant = json::find_path(&self.input[self.self_at..], &[b"tenant"]);
        let (ct, st) = match (caller_tenant, self_tenant) {
            (Ok(Some((a, _))), Ok(Some((b, _)))) => {
                // find_path returns offsets relative to the slice it scanned;
                // rebase to absolute indices into self.input before skip_ws_fwd
                // and as_str (both of which index into the full buffer).
                let abs_a = self.caller_at + a;
                let abs_b = self.self_at + b;
                (
                    json::as_str(self.input, skip_ws_fwd(self.input, abs_a)).ok(),
                    json::as_str(self.input, skip_ws_fwd(self.input, abs_b)).ok(),
                )
            }
            _ => (None, None),
        };
        let m = matches!((ct, st), (Some(a), Some(b)) if a == b);
        self.push(Val::Bool(m))
    }
}

fn skip_ws_fwd(bytes: &[u8], mut at: usize) -> usize {
    while at < bytes.len() && matches!(bytes[at], b' ' | b'\t' | b'\n' | b'\r') {
        at += 1;
    }
    at
}
