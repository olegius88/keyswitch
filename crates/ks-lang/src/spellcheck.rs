//! Optional Hunspell integration used as morphological dictionary evidence.
//!
//! Hunspell through a C library exists on Linux only when it is installed, and packaged Windows builds have none,
//! so the language model sees a `SpellChecker`. The real binding comes with the Linux port: it owns one dictionary
//! handle and asks `check_argument` what to pass on. A speller that is not there answers as `DisabledSpellChecker`
//! does: not available, an empty source, every word unknown.

use crate::constants::HUNSPELL_MAX_CHECK_CHARACTERS;
use ks_text::pystr;
use std::path::{Path, PathBuf};

pub const DEFAULT_DICTIONARY_ROOTS: [&str; 3] = ["/usr/share/hunspell", "/usr/share/myspell/dicts", "/usr/local/share/hunspell"];

/// What `HunspellDictionary` offers the language model. `Send + Sync`: the Python guards its handle with a lock.
pub trait SpellChecker: Send + Sync {
    /// `HunspellDictionary.available`: a dictionary is open.
    fn available(&self) -> bool;
    /// `HunspellDictionary.source`: the path of the dictionary file, empty without one.
    fn source(&self) -> &str;
    /// `HunspellDictionary.check`.
    fn check(&self, word: &str) -> bool;
}

/// Deterministic no-op spell checker for sealed offline model training (`language_model._DisabledSpellChecker`),
/// and what a build without Hunspell runs with.
#[derive(Clone, Copy, Debug, Default)]
pub struct DisabledSpellChecker;

impl SpellChecker for DisabledSpellChecker {
    fn available(&self) -> bool {
        false
    }
    fn source(&self) -> &'static str {
        ""
    }
    fn check(&self, _word: &str) -> bool {
        false
    }
}

/// A spell checker answered by a closure: for tests and for replaying what Hunspell once answered.
pub struct FnSpellChecker<F> {
    source: String,
    answer: F,
}

impl<F: Fn(&str) -> bool + Send + Sync> FnSpellChecker<F> {
    pub fn new(source: impl Into<String>, answer: F) -> Self {
        FnSpellChecker { source: source.into(), answer }
    }
}

impl<F: Fn(&str) -> bool + Send + Sync> SpellChecker for FnSpellChecker<F> {
    fn available(&self) -> bool {
        true
    }
    fn source(&self) -> &str {
        &self.source
    }
    fn check(&self, word: &str) -> bool {
        (self.answer)(word)
    }
}

/// The word `HunspellDictionary.check` hands to the library: stripped; `None` when it answers False without asking.
#[must_use]
pub fn check_argument(word: &str) -> Option<&str> {
    let normalized = pystr::strip(word);
    (!normalized.is_empty() && pystr::len(normalized) <= HUNSPELL_MAX_CHECK_CHARACTERS).then_some(normalized)
}

/// `_dictionary_roots` from the values it reads from the environment: `KEYSWITCH_HUNSPELL_PATH` split by
/// `os.pathsep`, `XDG_DATA_HOME`, the home folder; `data_dir` is `history.data_dir()`, which the app wires.
pub fn dictionary_roots_from(hunspell_path: Option<&str>, xdg_data_home: Option<&str>, home: Option<&Path>, data_dir: &Path) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = split_path_list(hunspell_path.unwrap_or("")).map(PathBuf::from).collect();
    roots.push(data_dir.join("dictionaries"));
    match xdg_data_home.filter(|value| !value.is_empty()) {
        Some(value) => roots.push(Path::new(value).join("hunspell")),
        None => {
            if let Some(home) = home {
                roots.push(home.join(".local").join("share").join("hunspell"));
            }
        }
    }
    roots.extend(DEFAULT_DICTIONARY_ROOTS.iter().map(PathBuf::from));
    roots
}

/// `HunspellDictionary._dictionary_roots`.
#[must_use]
pub fn dictionary_roots(data_dir: &Path) -> Vec<PathBuf> {
    let override_path = std::env::var_os("KEYSWITCH_HUNSPELL_PATH").map(|value| value.to_string_lossy().into_owned());
    let xdg = std::env::var_os("XDG_DATA_HOME").map(|value| value.to_string_lossy().into_owned());
    dictionary_roots_from(override_path.as_deref(), xdg.as_deref(), std::env::home_dir().as_deref(), data_dir)
}

/// `HunspellDictionary._find_dictionary` over given roots: the affix and dictionary paths of the first variant found.
#[must_use]
pub fn find_dictionary_in(locale: &str, roots: &[PathBuf]) -> Option<(PathBuf, PathBuf)> {
    let replaced = locale.replace('-', "_");
    let language = locale.split_once('_').map_or(locale, |(language, _)| language);
    let mut variants: Vec<&str> = Vec::with_capacity(3);
    for variant in [locale, replaced.as_str(), language] {
        if !variants.contains(&variant) {
            variants.push(variant);
        }
    }
    for root in roots {
        for variant in &variants {
            let affix = root.join(format!("{variant}.aff"));
            let dictionary = root.join(format!("{variant}.dic"));
            if affix.is_file() && dictionary.is_file() {
                return Some((affix, dictionary));
            }
        }
    }
    None
}

/// `HunspellDictionary._find_dictionary`.
#[must_use]
pub fn find_dictionary(locale: &str, data_dir: &Path) -> Option<(PathBuf, PathBuf)> {
    find_dictionary_in(locale, &dictionary_roots(data_dir))
}

/// The parts of a path list in an environment variable: `value.split(os.pathsep)` without the empty ones.
pub(crate) fn split_path_list(value: &str) -> impl Iterator<Item = &str> {
    let separator = if cfg!(windows) { ';' } else { ':' };
    value.split(separator).filter(|item| !item.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkers() {
        let disabled = DisabledSpellChecker;
        assert!(!disabled.available() && disabled.source().is_empty() && !disabled.check("word"));
        let table = FnSpellChecker::new("/x.dic", |word: &str| word == "word");
        assert!(table.available() && table.source() == "/x.dic" && table.check("word") && !table.check("other"));
    }

    #[test]
    fn the_argument_of_a_check() {
        assert_eq!(check_argument("  слово\x1c"), Some("слово"));
        assert_eq!(check_argument("   "), None);
        assert_eq!(check_argument(""), None);
        assert_eq!(check_argument(&"я".repeat(128)).map(str::len), Some(256));
        assert_eq!(check_argument(&"я".repeat(129)), None);
    }

    #[test]
    fn roots_in_order() {
        let data = Path::new("/data");
        let roots = dictionary_roots_from(Some("/a::/b"), None, Some(Path::new("/home/u")), data);
        let shown: Vec<_> = roots.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        if cfg!(windows) {
            return;
        }
        assert_eq!(shown, ["/a", "/b", "/data/dictionaries", "/home/u/.local/share/hunspell", "/usr/share/hunspell", "/usr/share/myspell/dicts", "/usr/local/share/hunspell"]);
        let roots = dictionary_roots_from(None, Some("/xdg"), Some(Path::new("/home/u")), data);
        assert_eq!(roots[1], Path::new("/xdg/hunspell"));
        let roots = dictionary_roots_from(None, Some(""), Some(Path::new("/home/u")), data);
        assert_eq!(roots[1], Path::new("/home/u/.local/share/hunspell"));
    }

    #[test]
    fn finds_a_dictionary_by_variant() {
        let root = std::env::temp_dir().join(format!("ks-lang-spell-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("temp dir");
        for name in ["en.aff", "en.dic", "ru_RU.aff"] {
            std::fs::write(root.join(name), "").expect("write");
        }
        let roots = vec![root.clone()];
        assert_eq!(find_dictionary_in("en_US", &roots), Some((root.join("en.aff"), root.join("en.dic"))));
        assert_eq!(find_dictionary_in("ru_RU", &roots), None);
        assert_eq!(find_dictionary_in("de-DE", &roots), None);
        std::fs::remove_dir_all(&root).expect("cleanup");
    }
}
