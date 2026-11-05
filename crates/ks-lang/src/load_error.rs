//! What goes wrong loading a resource, by the name of the exception the Python raises there.
//!
//! `try_load` turns the name into a status (`unavailable: ValueError`) and `str(error)` into a message, so both are
//! kept. Python's own messages for a broken JSON document are not reproduced, only its class.

use ks_cast::To as _;
use std::fmt;
use std::io;
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadError {
    /// `type(error).__name__`.
    pub name: &'static str,
    /// `str(error)`.
    pub message: String,
}

impl LoadError {
    pub fn value(message: impl Into<String>) -> Self {
        LoadError { name: "ValueError", message: message.into() }
    }

    pub fn type_error(message: impl Into<String>) -> Self {
        LoadError { name: "TypeError", message: message.into() }
    }

    /// The `OSError` subclass Python raises for an I/O failure on `path`, with its text.
    #[must_use]
    #[expect(clippy::wildcard_enum_match_arm, reason = "std::io::ErrorKind is non-exhaustive: a new kind has to land somewhere")]
    pub fn os(error: &io::Error, path: &Path) -> Self {
        let name = match error.kind() {
            io::ErrorKind::NotFound => "FileNotFoundError",
            io::ErrorKind::PermissionDenied => "PermissionError",
            io::ErrorKind::IsADirectory => "IsADirectoryError",
            io::ErrorKind::NotADirectory => "NotADirectoryError",
            _ => "OSError",
        };
        let text = error.to_string();
        let message = match error.raw_os_error() {
            Some(code) => {
                let reason = text.strip_suffix(&format!(" (os error {code})")).unwrap_or(&text);
                format!("[Errno {code}] {reason}: '{}'", path.display())
            }
            None => text,
        };
        LoadError { name, message }
    }

    /// `JSONDecodeError` or, for bytes that are no UTF-8, `UnicodeDecodeError`: both are `ValueError`s.
    pub(crate) fn json(error: &serde_json::Error) -> Self {
        use serde_json::error::Category;
        match error.classify() {
            // A value of the wrong type met by a visitor: the check after `json.loads` that would refuse it.
            Category::Data => LoadError::value(error.to_string()),
            Category::Io | Category::Syntax | Category::Eof => LoadError { name: "JSONDecodeError", message: error.to_string() },
        }
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for LoadError {}

/// `handle.read(limit + 1)` of a whole file.
pub(crate) fn read_limited(path: &Path, limit: usize) -> Result<Vec<u8>, LoadError> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|error| LoadError::os(&error, path))?;
    let mut content = Vec::new();
    file.take(limit.to::<u64>() + 1).read_to_end(&mut content).map_err(|error| LoadError::os(&error, path))?;
    Ok(content)
}

/// `path.read_bytes()`.
pub(crate) fn read_all(path: &Path) -> Result<Vec<u8>, LoadError> {
    std::fs::read(path).map_err(|error| LoadError::os(&error, path))
}

/// What `json.loads` accepts of bytes before parsing: an optional byte order mark and UTF-8.
pub(crate) fn json_text(raw: &[u8]) -> Result<&[u8], LoadError> {
    let raw = raw.strip_prefix(b"\xef\xbb\xbf").unwrap_or(raw);
    match std::str::from_utf8(raw) {
        Ok(_) => Ok(raw),
        Err(error) => Err(LoadError { name: "UnicodeDecodeError", message: error.to_string() }),
    }
}

/// `json.loads(raw)` into a tree: for the small files, where a tree costs nothing worth saving.
pub(crate) fn json_loads(raw: &[u8]) -> Result<serde_json::Value, LoadError> {
    serde_json::from_slice(json_text(raw)?).map_err(|error| LoadError { name: "JSONDecodeError", message: error.to_string() })
}

/// `value == number` in Python: `1 == 1.0 == True`.
pub(crate) fn py_equals_int(value: &serde_json::Value, number: i64) -> bool {
    match value {
        serde_json::Value::Bool(flag) => i64::from(*flag) == number,
        serde_json::Value::Number(found) => {
            found.as_i64().map_or_else(|| found.as_f64() == Some(number.to::<f64>()), |found| found == number)
        }
        serde_json::Value::Null | serde_json::Value::String(_) | serde_json::Value::Array(_) | serde_json::Value::Object(_) => false,
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn every_kind_of_failure_has_its_python_name() {
        let path = Path::new("x");
        for (kind, name) in [
            (io::ErrorKind::NotFound, "FileNotFoundError"),
            (io::ErrorKind::PermissionDenied, "PermissionError"),
            (io::ErrorKind::IsADirectory, "IsADirectoryError"),
            (io::ErrorKind::NotADirectory, "NotADirectoryError"),
            (io::ErrorKind::Other, "OSError"),
        ] {
            let error = LoadError::os(&io::Error::new(kind, "why"), path);
            assert_eq!((error.name, error.message.as_str()), (name, "why"));
            assert_eq!(error.to_string(), "why");
        }
    }

    #[test]
    fn os_errors_read_like_python() {
        let path = Path::new("/nonexistent-ks-lang/file");
        let error = read_all(path).unwrap_err();
        assert_eq!(error.name, "FileNotFoundError");
        #[cfg(unix)]
        assert_eq!(error.message, "[Errno 2] No such file or directory: '/nonexistent-ks-lang/file'");
    }

    #[test]
    fn json_text_rules() {
        assert_eq!(json_text(b"\xef\xbb\xbf{}").unwrap(), b"{}");
        assert_eq!(json_text(b"\xff").unwrap_err().name, "UnicodeDecodeError");
        assert_eq!(json_loads(b"{").unwrap_err().name, "JSONDecodeError");
        assert!(py_equals_int(&serde_json::json!(true), 1) && py_equals_int(&serde_json::json!(1.0), 1) && py_equals_int(&serde_json::json!(1), 1));
        assert!(!py_equals_int(&serde_json::json!("1"), 1) && !py_equals_int(&serde_json::json!(2), 1) && !py_equals_int(&serde_json::json!(null), 1));
    }
}
