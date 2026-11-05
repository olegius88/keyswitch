//! Words that look like a password never reach the technical log as typed.
//!
//! The technical log holds every word typed, and it is what goes with a report (`LogCourier` delivers it to a chat). A
//! word typed right after «пароль» or «password» (in either layout) and every long token that mixes letters and digits
//! go into it as stars, as many as the word has characters, in every text a record carries: the word evaluated, its
//! readings and replacements, and the words around it. The passwords that reached the log before the rule were such
//! tokens or the words after such a marker.

use crate::constants::{
    LOG_SECRET_MARKERS, LOG_SECRET_MIN_CHARACTERS, LOG_SECRET_REMEMBERED_WORDS, LOG_STRUCTURE_KEYS,
    LOG_STRUCTURE_KEY_SUFFIXES, LOG_WORD_KEYS,
};
use ks_text::layouts::LayoutPair;
use ks_text::pystr::{casefold, strip_chars};
use ks_text::ucd::{isalpha, isdigit, re_s};
use serde_json::{Map, Value};
use std::collections::{HashSet, VecDeque};
use std::sync::OnceLock;

/// Signs around a word that are no part of it (`пароль:`, `"секрет"`).
const EDGE_SIGNS: &str = ".,!?:;\"'()[]{}<>«»";
pub const MASK: char = '*';

/// The runs of characters that are not whitespace (`re.finditer(r"\S+", text)`): the byte range of each.
fn tokens(text: &str) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut start: Option<usize> = None;
    for (index, c) in text.char_indices() {
        if re_s(c) {
            if let Some(begin) = start.take() {
                found.push((begin, index));
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }
    if let Some(begin) = start {
        found.push((begin, text.len()));
    }
    found
}

fn core(token: &str) -> String {
    casefold(strip_chars(token, EDGE_SIGNS))
}

fn compute_markers() -> HashSet<String> {
    let pair = LayoutPair::default();
    let words: Vec<String> = LOG_SECRET_MARKERS.iter().map(|word| casefold(word)).collect();
    let mut all: Vec<String> = words.clone();
    // The marker typed in the other layout is the same word on the same keys (`gfhjkm` is `пароль`).
    for word in &words {
        all.push(pair.translate(word, "ru", "us").expect("us/ru is supported"));
    }
    for word in &words {
        all.push(pair.translate(word, "us", "ru").expect("us/ru is supported"));
    }
    all.iter().map(|word| core(word)).collect()
}

/// `MARKERS`: the words after which the next word is a password, in both layouts.
pub fn markers() -> &'static HashSet<String> {
    static MARKERS: OnceLock<HashSet<String>> = OnceLock::new();
    MARKERS.get_or_init(compute_markers)
}

/// A token of `LOG_SECRET_MIN_CHARACTERS` characters or more, edge signs aside, with letters and digits in it: a password
/// or a key looks like it (`parol123`).
pub fn secret_token(token: &str) -> bool {
    let core = strip_chars(token, EDGE_SIGNS);
    core.chars().count() >= LOG_SECRET_MIN_CHARACTERS && core.chars().any(isalpha) && core.chars().any(isdigit)
}

/// `text` with each secret word as stars: a long token of letters and digits, the word right after a password marker,
/// and each word of `secrets` (casefolded, edge signs aside).
#[must_use]
pub fn masked_text<S: ::std::hash::BuildHasher>(text: &str, secrets: &HashSet<String, S>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut after_marker = false;
    for (start, end) in tokens(text) {
        let token = text.get(start..end).unwrap_or_default();
        let word = core(token);
        let hidden = after_marker || secret_token(token) || secrets.contains(&word);
        out.push_str(text.get(last..start).unwrap_or_default());
        if hidden {
            out.extend(std::iter::repeat_n(MASK, token.chars().count()));
        } else {
            out.push_str(token);
        }
        last = end;
        after_marker = markers().contains(&word);
    }
    out.push_str(text.get(last..).unwrap_or_default());
    out
}

fn structure(key: &str) -> bool {
    LOG_STRUCTURE_KEYS.contains(&key) || LOG_STRUCTURE_KEY_SUFFIXES.iter().any(|suffix| key.ends_with(suffix))
}

fn masked(value: &Value, secrets: &HashSet<String>) -> Value {
    match value {
        Value::String(text) => Value::String(masked_text(text, secrets)),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| (key.clone(), if structure(key) { item.clone() } else { masked(item, secrets) }))
                .collect::<Map<String, Value>>(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(|item| masked(item, secrets)).collect()),
        other @ (Value::Null | Value::Bool(_) | Value::Number(_)) => other.clone(),
    }
}

/// The words of the texts under a record's word keys (`LOG_WORD_KEYS`), at any depth.
fn words(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, item) in map {
                if LOG_WORD_KEYS.contains(&key.as_str()) {
                    texts(item, out);
                } else if !structure(key) {
                    words(item, out);
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|item| words(item, out)),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn texts(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => out.extend(tokens(text).into_iter().map(|(start, end)| core(text.get(start..end).unwrap_or_default()))),
        Value::Object(map) => map.values().for_each(|item| texts(item, out)),
        Value::Array(items) => items.iter().for_each(|item| texts(item, out)),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// Masks the secret words of each technical record (`masked_text`), and remembers the last words typed right after a
/// password marker: a later record names such a word again without the marker (a correction, an edit).
#[derive(Default)]
pub struct LogRedactor {
    secrets: VecDeque<String>,
}

impl LogRedactor {
    #[must_use]
    pub fn new() -> LogRedactor {
        LogRedactor::default()
    }

    #[expect(clippy::unreachable, reason = "masking keeps the kind of a value")]
    pub fn record(&mut self, payload: &Map<String, Value>) -> Map<String, Value> {
        let before = payload.get("context").and_then(Value::as_object).and_then(|context| context.get("words"));
        let previous: Vec<String> = match before.and_then(Value::as_object) {
            Some(map) => map
                .values()
                .filter_map(Value::as_str)
                .filter_map(|text| ks_text::pystr::split(text).last().map(|word| core(word)))
                .collect(),
            None => Vec::new(),
        };
        if previous.iter().any(|word| markers().contains(word)) {
            let mut found = Vec::new();
            words(&Value::Object(payload.clone()), &mut found);
            for word in found {
                if !word.is_empty() && !markers().contains(&word) {
                    // deque(maxlen=...): the oldest word goes out when a new one comes in.
                    self.secrets.push_back(word);
                    if self.secrets.len() > LOG_SECRET_REMEMBERED_WORDS {
                        self.secrets.pop_front();
                    }
                }
            }
        }
        let secrets: HashSet<String> = self.secrets.iter().cloned().collect();
        match masked(&Value::Object(payload.clone()), &secrets) {
            Value::Object(map) => map,
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => unreachable!("an object stays an object"),
        }
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn long_mixed_tokens_are_secrets() {
        let none = HashSet::new();
        assert_eq!(masked_text("ключ parol123x тут", &none), "ключ ********* тут");
        assert_eq!(masked_text("abc12", &none), "abc12");
        assert_eq!(masked_text("(parol123)", &none), "**********");
    }

    #[test]
    fn the_word_after_a_marker_in_either_layout() {
        let none = HashSet::new();
        assert_eq!(masked_text("мой пароль: секрет тут", &none), "мой пароль: ****** тут");
        assert_eq!(masked_text("gfhjkm  qwerty", &none), "gfhjkm  ******");
        assert_eq!(masked_text("Password hunter2", &none), "Password *******");
        assert_eq!(masked_text("Password", &none), "Password");
    }

    #[test]
    fn later_records_remember_the_word() {
        let mut redactor = LogRedactor::new();
        let first = json!({"event": "word", "context": {"words": {"a": "мой пароль"}}, "original": "секрет", "version": "пароль"});
        let out = redactor.record(first.as_object().unwrap());
        assert_eq!(out["original"], "******");
        assert_eq!(out["event"], "word");
        assert_eq!(out["version"], "пароль");
        let second = json!({"event": "word", "original": "секрет", "other": "тут секрет"});
        let out = redactor.record(second.as_object().unwrap());
        assert_eq!(out["original"], "******");
        assert_eq!(out["other"], "тут ******");
    }
}
