//! Pull-based JSON reader sized for the fixed `PolicyInputWire` shape.
//!
//! Intentionally does not handle Unicode escape sequences (`\uXXXX`); callers
//! requiring them should return -1. Numbers tolerate integer/float ambiguity.
//!
//! All operations return byte-slice indices into the original input and do
//! not allocate.

#![allow(dead_code)]

pub(crate) type JsonResult<T> = Result<T, ()>;

/// Find the start..end indices of the value at `path` inside `bytes`.
///
/// `path` is a sequence of object keys. Returns `Ok(None)` if the path doesn't
/// resolve (missing key, traversal through a non-object). Returns `Err(())` on
/// malformed JSON.
pub(crate) fn find_path<'a>(bytes: &'a [u8], path: &[&[u8]]) -> JsonResult<Option<(usize, usize)>> {
    let mut cursor = skip_ws(bytes, 0);
    for key in path {
        if cursor >= bytes.len() || bytes[cursor] != b'{' {
            return Ok(None);
        }
        match find_key_in_object(bytes, cursor, key)? {
            Some(value_start) => cursor = value_start,
            None => return Ok(None),
        }
    }
    let end = skip_value(bytes, cursor)?;
    Ok(Some((cursor, end)))
}

/// Read a boolean at the given position.
pub(crate) fn as_bool(bytes: &[u8], at: usize) -> JsonResult<bool> {
    if bytes[at..].starts_with(b"true") {
        Ok(true)
    } else if bytes[at..].starts_with(b"false") {
        Ok(false)
    } else {
        Err(())
    }
}

/// Read an integer at the given position (tolerant of floats with zero fraction).
pub(crate) fn as_i64(bytes: &[u8], at: usize) -> JsonResult<i64> {
    let end = skip_number(bytes, at)?;
    let slice = &bytes[at..end];
    core::str::from_utf8(slice)
        .ok()
        .and_then(|s| s.parse::<i64>().ok().or_else(|| s.parse::<f64>().ok().map(|f| f as i64)))
        .ok_or(())
}

/// Read a float at the given position.
pub(crate) fn as_f64(bytes: &[u8], at: usize) -> JsonResult<f64> {
    let end = skip_number(bytes, at)?;
    let slice = &bytes[at..end];
    core::str::from_utf8(slice).ok().and_then(|s| s.parse().ok()).ok_or(())
}

/// Read a string at the given position as a raw &[u8] slice (contents between
/// the quotes). Does not decode escape sequences.
pub(crate) fn as_str<'a>(bytes: &'a [u8], at: usize) -> JsonResult<&'a [u8]> {
    if bytes.get(at) != Some(&b'"') {
        return Err(());
    }
    let mut i = at + 1;
    let start = i;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            if bytes.get(i + 1) == Some(&b'u') {
                return Err(()); // Unicode escapes not supported.
            }
            i += 2;
        } else if bytes[i] == b'"' {
            return Ok(&bytes[start..i]);
        } else {
            i += 1;
        }
    }
    Err(())
}

/// Iterate array elements by starting index. Calls `f` with each element's
/// (start, end) indices. Stops and returns early if `f` returns false.
pub(crate) fn each_element<F>(bytes: &[u8], at: usize, mut f: F) -> JsonResult<()>
where
    F: FnMut(usize, usize) -> bool,
{
    if bytes.get(at) != Some(&b'[') {
        return Err(());
    }
    let mut i = skip_ws(bytes, at + 1);
    if bytes.get(i) == Some(&b']') {
        return Ok(());
    }
    loop {
        let start = i;
        i = skip_value(bytes, i)?;
        if !f(start, i) {
            return Ok(());
        }
        i = skip_ws(bytes, i);
        match bytes.get(i) {
            Some(b',') => {
                i = skip_ws(bytes, i + 1);
            }
            Some(b']') => return Ok(()),
            _ => return Err(()),
        }
    }
}

// --- internals ---

fn skip_ws(bytes: &[u8], mut at: usize) -> usize {
    while at < bytes.len()
        && matches!(bytes[at], b' ' | b'\t' | b'\n' | b'\r')
    {
        at += 1;
    }
    at
}

fn find_key_in_object(bytes: &[u8], at: usize, key: &[u8]) -> JsonResult<Option<usize>> {
    // at points at '{'
    let mut i = skip_ws(bytes, at + 1);
    if bytes.get(i) == Some(&b'}') {
        return Ok(None);
    }
    loop {
        // key string
        if bytes.get(i) != Some(&b'"') {
            return Err(());
        }
        let k = as_str(bytes, i)?;
        i += 2 + k.len(); // opening quote + contents + closing quote
        i = skip_ws(bytes, i);
        if bytes.get(i) != Some(&b':') {
            return Err(());
        }
        i = skip_ws(bytes, i + 1);
        if k == key {
            return Ok(Some(i));
        }
        // skip value
        i = skip_value(bytes, i)?;
        i = skip_ws(bytes, i);
        match bytes.get(i) {
            Some(b',') => {
                i = skip_ws(bytes, i + 1);
            }
            Some(b'}') => return Ok(None),
            _ => return Err(()),
        }
    }
}

fn skip_value(bytes: &[u8], at: usize) -> JsonResult<usize> {
    if at >= bytes.len() {
        return Err(());
    }
    match bytes[at] {
        b'"' => {
            let s = as_str(bytes, at)?;
            Ok(at + 2 + s.len())
        }
        b'{' => skip_container(bytes, at, b'{', b'}'),
        b'[' => skip_container(bytes, at, b'[', b']'),
        b't' | b'f' | b'n' => {
            for kw in [b"true".as_slice(), b"false".as_slice(), b"null".as_slice()] {
                if bytes[at..].starts_with(kw) {
                    return Ok(at + kw.len());
                }
            }
            Err(())
        }
        b'-' | b'0'..=b'9' => skip_number(bytes, at),
        _ => Err(()),
    }
}

fn skip_container(bytes: &[u8], at: usize, open: u8, close: u8) -> JsonResult<usize> {
    let mut depth: i32 = 0;
    let mut i = at;
    let mut in_str = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == b'"' {
                in_str = false;
            }
        } else {
            if b == b'"' {
                in_str = true;
            } else if b == open {
                depth += 1;
            } else if b == close {
                depth -= 1;
                if depth == 0 {
                    return Ok(i + 1);
                }
            }
        }
        i += 1;
    }
    Err(())
}

fn skip_number(bytes: &[u8], at: usize) -> JsonResult<usize> {
    let mut i = at;
    if bytes.get(i) == Some(&b'-') {
        i += 1;
    }
    while i < bytes.len() && matches!(bytes[i], b'0'..=b'9' | b'.' | b'e' | b'E' | b'-' | b'+') {
        i += 1;
    }
    if i == at {
        Err(())
    } else {
        Ok(i)
    }
}
