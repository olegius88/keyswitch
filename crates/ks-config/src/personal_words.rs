//! The user's own words, which the typo correction leaves as typed (`KeySwitchEngine` typo plan).
//!
//! A word becomes the user's own when they type it and leave it as it is `PERSONAL_WORD_MIN_COUNT` times, or at once when
//! they take back a typo correction of it: `комить`, `иишка` and `митап` are words of their language for them, whatever
//! a word list says. The file beside the history keeps no text: each word is stored as a salted SHA-256 (the salt is
//! drawn once per installation and kept in the same file), so the file lists which hashes are the user's words and how
//! often each word not yet theirs was left as typed. Both lists are bounded; the oldest entry goes first.

use crate::config::set_aside_unreadable;
use crate::constants::{
    PERSONAL_WORDS_MAX, PERSONAL_WORDS_SCHEMA_VERSION, PERSONAL_WORD_COUNTS_MAX, PERSONAL_WORD_HASH_HEX_CHARACTERS,
    PERSONAL_WORD_MIN_COUNT, PERSONAL_WORD_SALT_BYTES, USER_DATA_JSON_INDENT,
};
use crate::history::data_dir;
use crate::pycompat::create_parent;
use crate::pycompat::{decode_text, with_suffix, write_text};
use indexmap::IndexMap;
use ks_cast::To as _;
use ks_text::json::{dumps, loads_lenient, ObjectOf, Tolerant};
use ks_text::pystr::casefold;
use ks_text::{Indent, JsonOptions};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::hash_map::RandomState;
use std::collections::{BTreeMap, HashMap};
use std::hash::{BuildHasher, Hasher};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// `secrets.token_hex(nbytes)`. Randomness comes from the operating system through the seeds of `RandomState` (which
/// the standard library draws from `getrandom`, `BCryptGenRandom` or `arc4random`), mixed with SHA-256; this needs no
/// `unsafe` and no extra crate. A salt is not a key: it only has to differ between installations.
#[must_use]
pub fn token_hex(nbytes: usize) -> String {
    let mut pool: Vec<u8> = Vec::new();
    for round in 0u64..8 {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(round);
        pool.extend_from_slice(&hasher.finish().to_le_bytes());
    }
    let digest = Sha256::digest(&pool);
    let mut out = Vec::new();
    let mut block = digest.to_vec();
    while out.len() < nbytes {
        out.extend_from_slice(&block);
        block = Sha256::digest(&block).to_vec();
    }
    out.truncate(nbytes);
    hex(&out)
}

/// A dict that remembers the order of insertion: the first entry is the one used longest ago.
#[derive(Default)]
struct Ordered<V> {
    positions: HashMap<String, u64>,
    entries: BTreeMap<u64, (String, V)>,
    next: u64,
}

impl<V> Ordered<V> {
    fn new() -> Self {
        Ordered { positions: HashMap::new(), entries: BTreeMap::new(), next: 0 }
    }

    fn contains(&self, key: &str) -> bool {
        self.positions.contains_key(key)
    }

    fn len(&self) -> usize {
        self.positions.len()
    }

    /// `d[key] = value`: a key that is there keeps its place.
    fn set(&mut self, key: String, value: V) {
        if let Some(entry) = self.positions.get(&key).and_then(|position| self.entries.get_mut(position)) {
            entry.1 = value;
        } else {
            self.positions.insert(key.clone(), self.next);
            self.entries.insert(self.next, (key, value));
            self.next += 1;
        }
    }

    /// `d.pop(key, None)`.
    fn pop(&mut self, key: &str) -> Option<V> {
        let position = self.positions.remove(key)?;
        self.entries.remove(&position).map(|entry| entry.1)
    }

    /// `del d[next(iter(d))]`.
    fn pop_first(&mut self) {
        if let Some((_, (key, _))) = self.entries.pop_first() {
            self.positions.remove(&key);
        }
    }

    fn iter(&self) -> impl Iterator<Item = (&String, &V)> {
        self.entries.values().map(|(key, value)| (key, value))
    }

    fn clear(&mut self) {
        *self = Ordered::new();
    }
}

/// `schema_version` of the file as Python compared it: `1`, `1.0` and `true` are all the same number.
#[derive(Deserialize)]
#[serde(untagged)]
enum SchemaNumber {
    Flag(bool),
    Whole(i64),
    Fractional(f64),
}

impl SchemaNumber {
    /// Python's `==` with a whole number.
    fn equals(&self, number: i64) -> bool {
        match self {
            SchemaNumber::Flag(flag) => number == i64::from(*flag),
            SchemaNumber::Whole(whole) => *whole == number,
            SchemaNumber::Fractional(fractional) => {
                fractional.fract() == 0.0 && fractional.abs() < 1.0e38 && fractional.to::<i128>() == i128::from(number)
            }
        }
    }
}

/// The file as `load` reads it; what is missing or of another type makes the whole file unreadable, and what is not a
/// word or a positive count inside the two lists is skipped.
#[derive(Deserialize)]
struct WordsFile {
    schema_version: SchemaNumber,
    salt: String,
    personal: Vec<Tolerant<String>>,
    counts: IndexMap<String, Tolerant<i64>>,
}

/// The file as it is written.
#[derive(Serialize)]
struct SavedWords<'a> {
    schema_version: i64,
    salt: &'a str,
    personal: Vec<&'a str>,
    counts: IndexMap<&'a str, i64>,
}

struct State {
    salt: String,
    personal: Ordered<()>,
    counts: Ordered<i64>,
    unreadable: Option<PathBuf>,
}

/// The user's own words as salted hashes, in `personal-words.json`.
///
/// `persist` says whether the file may be written now: with the history off (`general.keep_history`) nothing typed is
/// stored, and the words stay the user's for the session only.
pub struct PersonalWords {
    pub path: PathBuf,
    persist: Box<dyn Fn() -> bool + Send + Sync>,
    /// Draws the salt (`secrets.token_hex(PERSONAL_WORD_SALT_BYTES)`); a test supplies a fixed one.
    salt_source: Box<dyn Fn() -> String + Send + Sync>,
    state: Mutex<State>,
}

impl PersonalWords {
    #[must_use]
    pub fn new(path: Option<PathBuf>, persist: Box<dyn Fn() -> bool + Send + Sync>) -> PersonalWords {
        PersonalWords::with_salt_source(path, persist, Box::new(|| token_hex(PERSONAL_WORD_SALT_BYTES)))
    }

    /// `new` with the draw of the salt given.
    #[must_use]
    pub fn with_salt_source(
        path: Option<PathBuf>,
        persist: Box<dyn Fn() -> bool + Send + Sync>,
        salt_source: Box<dyn Fn() -> String + Send + Sync>,
    ) -> PersonalWords {
        let words = PersonalWords {
            path: path.unwrap_or_else(|| data_dir().join("personal-words.json")),
            persist,
            salt_source,
            state: Mutex::new(State { salt: String::new(), personal: Ordered::new(), counts: Ordered::new(), unreadable: None }),
        };
        words.load();
        words
    }

    /// `PersonalWords(persist=lambda: True)`.
    #[must_use]
    pub fn always_persisting(path: Option<PathBuf>) -> PersonalWords {
        PersonalWords::new(path, Box::new(|| true))
    }

    /// Where an unreadable file was moved by the last load, if anywhere.
    pub fn unreadable(&self) -> Option<PathBuf> {
        lock(&self.state).unreadable.clone()
    }

    pub fn load(&self) {
        let mut state = lock(&self.state);
        state.salt = String::new();
        state.personal = Ordered::new();
        state.counts = Ordered::new();
        if !self.path.is_file() {
            return;
        }
        let payload = std::fs::read(&self.path)
            .ok()
            .and_then(|bytes| decode_text(&bytes, true))
            .and_then(|text| loads_lenient::<ObjectOf<WordsFile>>(text.as_bytes()).ok())
            .map(|file| file.0)
            .filter(|file| file.schema_version.equals(PERSONAL_WORDS_SCHEMA_VERSION));
        let Some(payload) = payload else {
            // The next word would save over the file: keep the user's words aside.
            state.unreadable = set_aside_unreadable(&self.path);
            return;
        };
        state.salt = payload.salt;
        for key in payload.personal.into_iter().filter_map(Tolerant::into_read) {
            state.personal.set(key, ());
        }
        for (key, count) in payload.counts {
            if let Some(count) = count.into_read().filter(|count| *count > 0) {
                state.counts.set(key, count);
            }
        }
    }

    fn key(&self, state: &mut State, word: &str) -> String {
        if state.salt.is_empty() {
            state.salt = (self.salt_source)();
        }
        let digest = Sha256::digest(format!("{}{}", state.salt, casefold(word)).as_bytes());
        hex(&digest).get(..PERSONAL_WORD_HASH_HEX_CHARACTERS).unwrap_or_default().to_string()
    }

    /// `word in personal_words`.
    pub fn contains(&self, word: &str) -> bool {
        let mut state = lock(&self.state);
        if state.salt.is_empty() {
            return false;
        }
        let key = self.key(&mut state, word);
        state.personal.contains(&key)
    }

    /// `len(personal_words)`.
    pub fn len(&self) -> usize {
        lock(&self.state).personal.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Make the word the user's own at once (a typo correction of it was taken back).
    pub fn add(&self, word: &str) {
        let mut state = lock(&self.state);
        let key = self.key(&mut state, word);
        state.counts.pop(&key);
        Self::remember(&mut state, key);
        self.save_locked(&state);
    }

    /// Count the word left as typed once more; whether it is the user's own now.
    pub fn keep(&self, word: &str) -> bool {
        let mut state = lock(&self.state);
        let key = self.key(&mut state, word);
        if state.personal.contains(&key) {
            return true;
        }
        let count = state.counts.pop(&key).unwrap_or(0) + 1;
        if count >= PERSONAL_WORD_MIN_COUNT {
            Self::remember(&mut state, key);
        } else {
            state.counts.set(key, count);
            while state.counts.len() > PERSONAL_WORD_COUNTS_MAX {
                state.counts.pop_first();
            }
        }
        self.save_locked(&state);
        count >= PERSONAL_WORD_MIN_COUNT
    }

    fn remember(state: &mut State, key: String) {
        state.personal.pop(&key);
        state.personal.set(key, ());
        while state.personal.len() > PERSONAL_WORDS_MAX {
            state.personal.pop_first();
        }
    }

    /// Forget every word, with a new salt: hashes kept elsewhere match nothing any more.
    pub fn clear(&self) {
        let mut state = lock(&self.state);
        state.salt = String::new();
        state.personal.clear();
        state.counts.clear();
        self.save_locked(&state);
    }

    pub fn save(&self) {
        let state = lock(&self.state);
        self.save_locked(&state);
    }

    fn save_locked(&self, state: &State) {
        if !(self.persist)() {
            return;
        }
        let saved = SavedWords {
            schema_version: PERSONAL_WORDS_SCHEMA_VERSION,
            salt: &state.salt,
            personal: state.personal.iter().map(|(key, ())| key.as_str()).collect(),
            counts: state.counts.iter().map(|(key, count)| (key.as_str(), *count)).collect(),
        };
        // json.dumps(data, indent=2) with the default ensure_ascii: every character is ASCII anyway.
        let options = JsonOptions { indent: Some(Indent::Spaces(USER_DATA_JSON_INDENT)), ..JsonOptions::default() };
        let mut text = dumps(&saved, &options);
        text.push('\n');
        let result: std::io::Result<()> = (|| {
            create_parent(&self.path)?;
            let temporary = with_suffix(&self.path, ".json.tmp");
            write_text(&temporary, &text)?;
            std::fs::rename(&temporary, &self.path)
        })();
        // The words stay the user's for this session; the next change tries again.
        let _ = result;
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn a_file_without_lists_is_an_empty_one_and_nothing_can_be_dropped_from_nothing() {
        let file = path("bare");
        std::fs::write(&file, r#"{"salt": "x"}"#).unwrap();
        let words = PersonalWords::always_persisting(Some(file));
        words.load();
        assert!(!words.contains("слово"));
        let mut ordered: Ordered<()> = Ordered::new();
        ordered.pop_first();
        assert_eq!(ordered.len(), 0);
    }

    fn path(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("ks-words-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory.join("personal-words.json")
    }

    #[test]
    fn a_word_is_theirs_after_two_keeps_or_at_once() {
        let file = path("keep");
        let words = PersonalWords::always_persisting(Some(file.clone()));
        assert!(!words.contains("митап"));
        assert!(!words.keep("Митап"));
        assert!(!words.contains("митап"));
        assert!(words.keep("митап"));
        assert!(words.contains("МИТАП") && words.len() == 1);
        words.add("иишка");
        assert!(words.contains("иишка") && words.keep("иишка"));
        let text = std::fs::read_to_string(&file).unwrap().replace("\r\n", "\n");
        assert!(text.starts_with("{\n  \"schema_version\": 1,\n  \"salt\": \""));
        assert!(!text.contains("митап"));
        let again = PersonalWords::always_persisting(Some(file));
        assert!(again.contains("митап") && again.contains("иишка") && again.len() == 2);
        again.clear();
        assert!(!again.contains("митап") && again.is_empty());
    }

    #[test]
    fn a_foreign_file_is_kept_aside_and_persist_can_be_off() {
        let file = path("foreign");
        std::fs::write(&file, "{\"schema_version\": 2}").unwrap();
        let words = PersonalWords::new(Some(file.clone()), Box::new(|| false));
        assert!(words.unreadable().is_some() && !file.exists());
        words.add("слово");
        assert!(words.contains("слово") && !file.exists());
    }

    #[test]
    fn a_file_is_read_as_far_as_python_read_it() {
        for (number, version) in ["1", "1.0", "true"].iter().enumerate() {
            let file = path(&format!("version-{number}"));
            std::fs::write(&file, format!(r#"{{"schema_version": {version}, "salt": "s", "personal": [], "counts": {{}}}}"#)).unwrap();
            assert!(PersonalWords::always_persisting(Some(file)).unreadable().is_none(), "{version}");
        }
        for (number, version) in ["2", "2.5", "1e39", "false", "\"1\"", "null", "18446744073709551616"].iter().enumerate() {
            let file = path(&format!("foreign-{number}"));
            std::fs::write(&file, format!(r#"{{"schema_version": {version}, "salt": "s", "personal": [], "counts": {{}}}}"#)).unwrap();
            assert!(PersonalWords::always_persisting(Some(file)).unreadable().is_some(), "{version}");
        }
        for (number, text) in [
            "[1, \"s\", [], {}]",
            r#"{"schema_version": 1, "salt": 5, "personal": [], "counts": {}}"#,
            r#"{"schema_version": 1, "salt": "s", "personal": {}, "counts": {}}"#,
            r#"{"schema_version": 1, "salt": "s", "personal": [], "counts": []}"#,
        ].iter().enumerate() {
            let file = path(&format!("broken-{number}"));
            std::fs::write(&file, text).unwrap();
            assert!(PersonalWords::always_persisting(Some(file)).unreadable().is_some(), "{text}");
        }
        // What is no word or no positive count inside the two lists is skipped, and the salt, the words and the counts stay.
        let file = path("kept");
        std::fs::write(&file, r#"{"schema_version": 1, "salt": "x", "salt": "s", "personal": ["aa", 5, null, "bb"], "counts": {"c": 2, "d": 0, "e": -1, "f": 1.5, "g": "1", "c": 3}}"#).unwrap();
        let words = PersonalWords::always_persisting(Some(file));
        let state = lock(&words.state);
        assert_eq!(state.salt, "s");
        assert_eq!(state.personal.iter().map(|(key, ())| key.as_str()).collect::<Vec<_>>(), ["aa", "bb"]);
        assert_eq!(state.counts.iter().map(|(key, count)| (key.as_str(), *count)).collect::<Vec<_>>(), [("c", 3)]);
    }

    #[test]
    fn the_file_is_written_key_by_key_as_it_was() {
        let file = path("bytes");
        let words = PersonalWords::with_salt_source(Some(file.clone()), Box::new(|| true), Box::new(|| "abc".to_owned()));
        words.add("Слово");
        assert!(!words.keep("другое"));
        let hash = |word: &str| hex(&Sha256::digest(format!("abc{word}").as_bytes())).get(..PERSONAL_WORD_HASH_HEX_CHARACTERS).unwrap().to_owned();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap().replace("\r\n", "\n"),
            format!("{{\n  \"schema_version\": 1,\n  \"salt\": \"abc\",\n  \"personal\": [\n    \"{}\"\n  ],\n  \"counts\": {{\n    \"{}\": 1\n  }}\n}}\n", hash("слово"), hash("другое"))
        );
    }

    #[test]
    fn the_oldest_counts_go_first_and_tokens_are_hex() {
        let words = PersonalWords::new(Some(path("bound")), Box::new(|| false));
        for index in 0..PERSONAL_WORD_COUNTS_MAX + 3 {
            words.keep(&format!("w{index}"));
        }
        assert_eq!(lock(&words.state).counts.len(), PERSONAL_WORD_COUNTS_MAX);
        assert!(!words.keep("w0") || words.contains("w0"), "w0 was forgotten, its count starts again");
        let token = token_hex(16);
        assert_eq!(token.len(), 32);
        assert_ne!(token, token_hex(16));
        assert!(token.bytes().all(|b| b.is_ascii_hexdigit()));
    }
}
