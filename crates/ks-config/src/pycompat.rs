//! The behaviours of Python's standard library that the stores of this crate depend on, so that the files they write
//! and the way they read hand-edited ones stay the same as the Python program's.
//!
//! - text files: `Path.read_text` / `write_text` translate newlines (universal newlines on reading; `\n` becomes
//!   `\r\n` on Windows when writing), and `utf-8-sig` drops a byte order mark;
//! - `Path.with_suffix`, which the atomic writes use for their temporary files;
//! - `==` and truthiness of JSON values (`1 == True == 1.0`, a dict equals a dict in any order);
//! - `json.loads`: serde_json parses the text. It differs from Python on `NaN`/`Infinity`, integers beyond 64 bits,
//!   a lone surrogate escape, a float that overflows and nesting deeper than 128 levels: Python reads those, serde_json
//!   refuses them, so the Rust side treats such a file as unreadable. A file written by either program holds none of them.

use serde_json::{Number, Value};
use std::io;
use std::path::{Path, PathBuf};

/// Whether this program writes text files with Windows newlines (what `open(..., "w")` does on Windows).
pub const WINDOWS_NEWLINES: bool = cfg!(windows);

/// The text a Python `read_text` returns for these bytes: decoded strictly, a byte order mark dropped when
/// `skip_bom` (the `utf-8-sig` codec), `\r\n` and a lone `\r` turned into `\n`. `None` is a `UnicodeDecodeError`.
pub fn decode_text(bytes: &[u8], skip_bom: bool) -> Option<String> {
    let bytes = if skip_bom { bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes) } else { bytes };
    let text = std::str::from_utf8(bytes).ok()?;
    if !text.contains('\r') {
        return Some(text.to_string());
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    Some(out)
}

/// Why a text file could not be read: an operating system error (`OSError`) or a decoding error (`ValueError`).
#[derive(Debug)]
pub enum ReadError {
    Os(io::Error),
    Decode,
}

/// `path.read_text(encoding="utf-8")` (`skip_bom` false) or `encoding="utf-8-sig"` (`skip_bom` true).
pub fn read_text(path: &Path, skip_bom: bool) -> Result<String, ReadError> {
    let bytes = std::fs::read(path).map_err(ReadError::Os)?;
    decode_text(&bytes, skip_bom).ok_or(ReadError::Decode)
}

/// The bytes `write_text` puts in the file for `text`.
pub fn encoded_text(text: &str) -> Vec<u8> {
    if WINDOWS_NEWLINES && text.contains('\n') {
        text.replace('\n', "\r\n").into_bytes()
    } else {
        text.as_bytes().to_vec()
    }
}

/// `path.write_text(text, encoding="utf-8")`: created or truncated, then written.
pub fn write_text(path: &Path, text: &str) -> io::Result<()> {
    std::fs::write(path, encoded_text(text))
}

/// `Path.with_suffix(suffix)`: the last suffix of the name (the part from its last dot, unless that dot starts the
/// name or ends it) is replaced, or the new one is appended.
pub fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let existing = match name.rfind('.') {
        Some(i) if i > 0 && i < name.len() - 1 => name.len() - i,
        _ => 0,
    };
    path.with_file_name(format!("{}{}", &name[..name.len() - existing], suffix))
}

/// `path.with_name(name + suffix)`.
pub fn with_name_suffix(path: &Path, suffix: &str) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    path.with_file_name(format!("{name}{suffix}"))
}

/// `json.loads(text)`, or `None` for a `ValueError` (see the note on top about the few texts Python reads and
/// serde_json does not).
pub fn json_loads(text: &str) -> Option<Value> {
    serde_json::from_str(text).ok()
}

fn exact_int(number: &Number) -> Option<i128> {
    if let Some(i) = number.as_i64() {
        Some(i128::from(i))
    } else {
        number.as_u64().map(i128::from)
    }
}

fn as_number(value: &Value) -> Option<Result<i128, f64>> {
    match value {
        Value::Bool(b) => Some(Ok(i128::from(*b))),
        Value::Number(n) => Some(match exact_int(n) {
            Some(i) => Ok(i),
            None => Err(n.as_f64().unwrap_or(f64::NAN)),
        }),
        _ => None,
    }
}

/// Python `a == b` for JSON values: `True == 1 == 1.0`, strings by code points, lists in order, dicts in any order.
pub fn py_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| py_eq(p, q)),
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, p)| y.get(k).is_some_and(|q| py_eq(p, q)))
        }
        _ => match (as_number(a), as_number(b)) {
            (Some(Ok(x)), Some(Ok(y))) => x == y,
            (Some(Err(x)), Some(Err(y))) => x == y,
            (Some(Ok(i)), Some(Err(f))) | (Some(Err(f)), Some(Ok(i))) => {
                f.is_finite() && f.fract() == 0.0 && f.abs() < 1.0e38 && f as i128 == i
            }
            _ => false,
        },
    }
}

/// Python `bool(value)` of a JSON value.
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => match exact_int(n) {
            Some(i) => i != 0,
            None => n.as_f64().is_some_and(|f| f != 0.0),
        },
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// `type(value) is int`: an integer that is not a bool and not a float.
pub fn as_py_int(value: &Value) -> Option<i64> {
    match value {
        Value::Number(n) => n.as_i64(),
        _ => None,
    }
}

/// `isinstance(value, int) and not isinstance(value, bool)`, whatever the size (a float is not an int).
pub fn is_py_int(value: &Value) -> bool {
    matches!(value, Value::Number(n) if n.is_i64() || n.is_u64())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn suffix_like_pathlib() {
        let s = |p: &str, x: &str| with_suffix(Path::new(p), x).to_string_lossy().into_owned();
        assert_eq!(s("/a/config.json", ".json.tmp"), "/a/config.json.tmp");
        assert_eq!(s("/a/history.jsonl", ".jsonl.tmp"), "/a/history.jsonl.tmp");
        assert_eq!(s("/a/noext", ".json.tmp"), "/a/noext.json.tmp");
        assert_eq!(s("/a/.hidden", ".json.tmp"), "/a/.hidden.json.tmp");
        assert_eq!(s("/a/dot.", ".json.tmp"), "/a/dot..json.tmp");
        assert_eq!(s("/a/x.tar.gz", ".z"), "/a/x.tar.z");
    }

    #[test]
    fn text_decoding() {
        assert_eq!(decode_text(b"\xEF\xBB\xBF{}\r\n\r", true).as_deref(), Some("{}\n\n"));
        assert_eq!(decode_text(b"\xEF\xBB\xBF{}", false).as_deref(), Some("\u{feff}{}"));
        assert_eq!(decode_text(b"a\xFFb", true), None);
    }

    #[test]
    fn equality_like_python() {
        assert!(py_eq(&json!(1), &json!(true)));
        assert!(py_eq(&json!(1), &json!(1.0)));
        assert!(py_eq(&json!(0.0), &json!(false)));
        assert!(!py_eq(&json!(1), &json!("1")));
        assert!(py_eq(&json!({"a": 1, "b": [1, 2]}), &json!({"b": [1.0, 2], "a": true})));
        assert!(!py_eq(&json!([1, 2]), &json!([2, 1])));
        assert!(!py_eq(&Value::Null, &json!(0)));
        assert!(py_eq(&json!(1.5), &json!(1.5)));
        assert!(!py_eq(&json!(u64::MAX), &json!(1.8446744073709552e19)));
    }

    #[test]
    fn truthiness_and_ints() {
        assert!(!truthy(&json!(0.0)) && !truthy(&json!("")) && !truthy(&json!([])) && truthy(&json!({"a": 1})));
        assert!(is_py_int(&json!(3)) && !is_py_int(&json!(3.0)) && !is_py_int(&json!(true)));
        assert_eq!(as_py_int(&json!(-4)), Some(-4));
    }
}
