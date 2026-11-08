//! The behaviours of Python's standard library that the stores of this crate depend on, so that the files they write
//! and the way they read hand-edited ones stay the same as the Python program's.
//!
//! - text files: `Path.read_text` / `write_text` translate newlines (universal newlines on reading; `\n` becomes
//!   `\r\n` on Windows when writing), and `utf-8-sig` drops a byte order mark;
//! - `Path.with_suffix`, which the atomic writes use for their temporary files;
//! - `json.loads` is `serde_json` reading the text. It differs from Python on `NaN`/`Infinity`, integers beyond 64 bits,
//!   a lone surrogate escape, a float that overflows and nesting deeper than 128 levels: Python reads those, `serde_json`
//!   refuses them, so the Rust side treats such a file as unreadable. A file written by either program holds none of them.
//!
//! `==` and truth value over JSON values (`1 == True == 1.0`, a dict equals a dict in any order) live in `settings_file`,
//! with the tree they compare; the crates not typed yet still call them from here.

pub use crate::settings_file::{as_py_int, is_py_int, json_loads, py_eq, truthy};
use std::io;
use std::path::{Path, PathBuf};

/// Whether this program writes text files with Windows newlines (what `open(..., "w")` does on Windows).
pub const WINDOWS_NEWLINES: bool = cfg!(windows);

/// The text a Python `read_text` returns for these bytes: decoded strictly, a byte order mark dropped when
/// `skip_bom` (the `utf-8-sig` codec), `\r\n` and a lone `\r` turned into `\n`. `None` is a `UnicodeDecodeError`.
#[must_use]
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

/// Creates the folder of a file; a path without one has nothing to create.
pub fn create_parent(path: &Path) -> io::Result<()> {
    path.parent().map_or(Ok(()), std::fs::create_dir_all)
}

/// The bytes `write_text` puts in the file for `text`.
#[must_use]
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
#[must_use]
pub fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let existing = match name.rfind('.') {
        Some(i) if i > 0 && i < name.len() - 1 => name.len() - i,
        _ => 0,
    };
    path.with_file_name(format!("{}{}", name.get(..name.len() - existing).unwrap_or_default(), suffix))
}

/// `path.with_name(name + suffix)`.
#[must_use]
pub fn with_name_suffix(path: &Path, suffix: &str) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    path.with_file_name(format!("{name}{suffix}"))
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn a_path_without_a_folder_has_none_to_make() {
        create_parent(Path::new("")).unwrap();
    }

    #[test]
    fn suffix_like_pathlib() {
        let s = |p: &str, x: &str| with_suffix(Path::new(p), x).to_string_lossy().into_owned();
        assert_eq!(s("/a/config.json", ".json.tmp"), Path::new("/a").join("config.json.tmp").to_string_lossy());
        assert_eq!(s("/a/history.jsonl", ".jsonl.tmp"), Path::new("/a").join("history.jsonl.tmp").to_string_lossy());
        assert_eq!(s("/a/noext", ".json.tmp"), Path::new("/a").join("noext.json.tmp").to_string_lossy());
        assert_eq!(s("/a/.hidden", ".json.tmp"), Path::new("/a").join(".hidden.json.tmp").to_string_lossy());
        assert_eq!(s("/a/dot.", ".json.tmp"), Path::new("/a").join("dot..json.tmp").to_string_lossy());
        assert_eq!(s("/a/x.tar.gz", ".z"), Path::new("/a").join("x.tar.z").to_string_lossy());
    }

    #[test]
    fn text_decoding() {
        assert_eq!(decode_text(b"\xEF\xBB\xBF{}\r\n\r", true).as_deref(), Some("{}\n\n"));
        assert_eq!(decode_text(b"\xEF\xBB\xBF{}", false).as_deref(), Some("\u{feff}{}"));
        assert_eq!(decode_text(b"a\xFFb", true), None);
    }
}
