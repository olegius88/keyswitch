//! Packaged identifier lexicon: executable names known to the distribution index.
//!
//! A command name typed in the wrong layout looks like gibberish in both readings to the natural-language lexicons and
//! n-gram models, so this list supplies the same kind of lexical evidence for identifiers that dictionaries supply for
//! words. It is a resource, not a label: it never decides anything by itself.

use crate::constants::{IDENTIFIER_LEXICON_MAX_BYTES, IDENTIFIER_LEXICON_MAX_ENTRIES, VERSION_HASH_CHARACTERS};
use crate::load_error::{json_loads, py_equals_int, read_limited, LoadError};
use crate::tables::WordList;
use ks_text::pystr;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt::Write;
use std::path::{Path, PathBuf};

/// `RESOURCE_PATH` under the `resources` folder of the install.
#[must_use]
pub fn resource_path(resource_dir: &Path) -> PathBuf {
    resource_dir.join("identifiers.json")
}

/// `IDENTIFIER`, `[a-z][a-z0-9]{2,63}\Z`.
fn is_identifier(entry: &str) -> bool {
    let bytes = entry.as_bytes();
    (3..=64).contains(&bytes.len()) && bytes[0].is_ascii_lowercase() && bytes[1..].iter().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

#[derive(Clone, Debug)]
pub struct IdentifierLexicon {
    /// Sorted; the frozenset of the Python.
    pub identifiers: WordList,
    pub name: String,
    pub version: String,
}

impl IdentifierLexicon {
    #[must_use]
    pub fn normalize(token: &str) -> String {
        pystr::casefold(token)
    }

    /// Exact, case-insensitive membership; no prefixes, no fuzzy matching.
    #[must_use]
    pub fn contains(&self, token: &str) -> bool {
        self.identifiers.contains_sorted(&Self::normalize(token))
    }

    /// `IdentifierLexicon.load(path)`; the caching of the Python lives in `Resources::identifier_lexicon`.
    pub fn load(path: &Path) -> Result<IdentifierLexicon, LoadError> {
        let raw = read_limited(path, IDENTIFIER_LEXICON_MAX_BYTES)?;
        if raw.len() > IDENTIFIER_LEXICON_MAX_BYTES {
            return Err(LoadError::value("identifier lexicon is too large"));
        }
        let payload = json_loads(&raw)?;
        let (Value::Object(payload), true) = (&payload, true) else {
            return Err(LoadError::value("invalid identifier lexicon"));
        };
        let (Some(true), Some(Value::String(name)), Some(Value::Array(entries))) =
            (payload.get("schema_version").map(|v| py_equals_int(v, 1)), payload.get("name"), payload.get("identifiers"))
        else {
            return Err(LoadError::value("invalid identifier lexicon"));
        };
        let sorted_set = "identifier lexicon must be a sorted set";
        if entries.is_empty() || entries.len() > IDENTIFIER_LEXICON_MAX_ENTRIES {
            return Err(LoadError::value(sorted_set));
        }
        let mut identifiers = WordList::with_capacity(entries.len(), 0);
        for entry in entries {
            match entry {
                Value::String(text) => identifiers.push(text),
                // Elements other than strings fail one of the two checks in the Python.
                Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => return Err(LoadError::value("invalid identifier entry")),
            }
        }
        if !identifiers.is_sorted_set() {
            return Err(LoadError::value(sorted_set));
        }
        if !identifiers.iter().all(is_identifier) {
            return Err(LoadError::value("invalid identifier entry"));
        }
        let mut digest = String::with_capacity(64);
        for byte in Sha256::digest(&raw) {
            write!(digest, "{byte:02x}").expect("a String takes the text");
        }
        digest.truncate(VERSION_HASH_CHARACTERS);
        Ok(IdentifierLexicon { identifiers: identifiers.shrunk(), name: name.clone(), version: format!("identifiers-{digest}") })
    }

    /// `IdentifierLexicon.try_load(path)`: the lexicon and its version, or nothing and the reason.
    #[must_use]
    pub fn try_load(path: &Path) -> (Option<IdentifierLexicon>, String) {
        match Self::load(path) {
            Ok(lexicon) => {
                let version = lexicon.version.clone();
                (Some(lexicon), version)
            }
            Err(error) => (None, error.message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shape_of_an_identifier() {
        assert!(is_identifier("htop") && is_identifier("a12") && is_identifier(&"a".repeat(64)));
        assert!(!is_identifier("ab") && !is_identifier("1ab") && !is_identifier("Abc") && !is_identifier("abc\n"));
        assert!(!is_identifier(&"a".repeat(65)) && !is_identifier("абв") && !is_identifier(""));
    }

    #[test]
    fn a_small_lexicon() {
        let path = std::env::temp_dir().join(format!("ks-lang-ident-{}.json", std::process::id()));
        std::fs::write(&path, br#"{"schema_version": 1, "name": "n", "identifiers": ["abc", "z12"]}"#).unwrap();
        let (lexicon, status) = IdentifierLexicon::try_load(&path);
        std::fs::remove_file(&path).unwrap();
        let lexicon = lexicon.unwrap();
        assert_eq!(lexicon.name, "n");
        assert_eq!(status, lexicon.version);
        assert_eq!(status.len(), "identifiers-".len() + 12);
        assert!(lexicon.contains("ABC") && lexicon.contains("z12") && !lexicon.contains("ab"));
        assert_eq!(IdentifierLexicon::try_load(Path::new("/nonexistent-ks-lang")).0.map(|l| l.name), None);
    }
}
