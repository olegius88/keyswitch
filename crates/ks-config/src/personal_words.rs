//! The user's own words, which the typo correction leaves as typed (KeySwitchEngine typo plan).
//!
//! A word becomes the user's own when they type it and leave it as it is PERSONAL_WORD_MIN_COUNT times, or at once when
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
use crate::pycompat::{decode_text, json_loads, py_eq, with_suffix, write_text};
use ks_text::pystr::casefold;
use ks_text::{json_dumps, Indent, JsonOptions};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::hash_map::RandomState;
use std::collections::{BTreeMap, HashMap};
use std::hash::{BuildHasher, Hasher};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// `secrets.token_hex(nbytes)`. Randomness comes from the operating system through the seeds of `RandomState` (which
/// the standard library draws from `getrandom`, `BCryptGenRandom` or `arc4random`), mixed with SHA-256; this needs no
/// `unsafe` and no extra crate. A salt is not a key: it only has to differ between installations.
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
        match self.positions.get(&key) {
            Some(position) => {
                if let Some(entry) = self.entries.get_mut(position) {
                    entry.1 = value;
                }
            }
            None => {
                self.positions.insert(key.clone(), self.next);
                self.entries.insert(self.next, (key, value));
                self.next += 1;
            }
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

struct State {
    salt: String,
    personal: Ordered<()>,
    counts: Ordered<i64>,
    unreadable: Option<PathBuf>,
}

/// The user's own words as salted hashes, in `personal-words.json`.
///
/// `persist` says whether the file may be written now: with the history off (general.keep_history) nothing typed is
/// stored, and the words stay the user's for the session only.
pub struct PersonalWords {
    pub path: PathBuf,
    persist: Box<dyn Fn() -> bool + Send + Sync>,
    /// Draws the salt (`secrets.token_hex(PERSONAL_WORD_SALT_BYTES)`); a test supplies a fixed one.
    salt_source: Box<dyn Fn() -> String + Send + Sync>,
    state: Mutex<State>,
}

impl PersonalWords {
    pub fn new(path: Option<PathBuf>, persist: Box<dyn Fn() -> bool + Send + Sync>) -> PersonalWords {
        PersonalWords::with_salt_source(path, persist, Box::new(|| token_hex(PERSONAL_WORD_SALT_BYTES)))
    }

    /// `new` with the draw of the salt given.
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
        let payload = std::fs::read(&self.path).ok().and_then(|bytes| decode_text(&bytes, true)).and_then(|text| json_loads(&text));
        let valid = payload.as_ref().and_then(Value::as_object).filter(|payload| {
            payload.get("schema_version").is_some_and(|version| py_eq(version, &Value::from(PERSONAL_WORDS_SCHEMA_VERSION)))
                && payload.get("salt").is_some_and(Value::is_string)
                && payload.get("personal").is_some_and(Value::is_array)
                && payload.get("counts").is_some_and(Value::is_object)
        });
        let Some(payload) = valid else {
            // The next word would save over the file: keep the user's words aside.
            state.unreadable = set_aside_unreadable(&self.path);
            return;
        };
        state.salt = payload["salt"].as_str().unwrap_or_default().to_string();
        if let Value::Array(personal) = &payload["personal"] {
            for key in personal.iter().filter_map(Value::as_str) {
                state.personal.set(key.to_string(), ());
            }
        }
        if let Value::Object(counts) = &payload["counts"] {
            for (key, count) in counts {
                if let Some(count) = crate::pycompat::as_py_int(count).filter(|count| *count > 0) {
                    state.counts.set(key.clone(), count);
                }
            }
        }
    }

    fn key(&self, state: &mut State, word: &str) -> String {
        if state.salt.is_empty() {
            state.salt = (self.salt_source)();
        }
        let digest = Sha256::digest(format!("{}{}", state.salt, casefold(word)).as_bytes());
        hex(&digest)[..PERSONAL_WORD_HASH_HEX_CHARACTERS].to_string()
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
        let mut counts = Map::new();
        for (key, count) in state.counts.iter() {
            counts.insert(key.clone(), Value::from(*count));
        }
        let mut data = Map::new();
        data.insert("schema_version".into(), Value::from(PERSONAL_WORDS_SCHEMA_VERSION));
        data.insert("salt".into(), Value::String(state.salt.clone()));
        data.insert("personal".into(), Value::Array(state.personal.iter().map(|(key, _)| Value::String(key.clone())).collect()));
        data.insert("counts".into(), Value::Object(counts));
        // json.dumps(data, indent=2) with the default ensure_ascii: every character is ASCII anyway.
        let options = JsonOptions { indent: Some(Indent::Spaces(USER_DATA_JSON_INDENT)), ..JsonOptions::default() };
        let mut text = json_dumps(&Value::Object(data), &options);
        text.push('\n');
        let result: std::io::Result<()> = (|| {
            if let Some(parent) = self.path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let temporary = with_suffix(&self.path, ".json.tmp");
            write_text(&temporary, &text)?;
            std::fs::rename(&temporary, &self.path)
        })();
        // The words stay the user's for this session; the next change tries again.
        let _ = result;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let text = std::fs::read_to_string(&file).unwrap();
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
