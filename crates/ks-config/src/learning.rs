//! Switching rules the user added one by one in the rule window.
//!
//! A rule says what to do with a typed word that meets its condition: convert it to the other layout, or keep it as
//! typed. Nothing is written here unless the user pressed OK in the rule window: a manual conversion, an undo or a
//! cancelled correction teaches nothing on its own.

use crate::config::set_aside_unreadable;
use crate::constants::{
    LEARNING_STORE_BACKUP_SUFFIX, LEARNING_STORE_SCHEMA_VERSION, LEGACY_RULE_CONFIRMATIONS_REQUIRED, MAX_WORD_STROKES,
    USER_DATA_JSON_INDENT,
};
use crate::history::data_dir;
use crate::pycompat::{decode_text, json_loads, with_name_suffix, with_suffix, write_text};
use ks_text::pystr::{casefold, isdigit, isspace, strip};
use ks_text::{json_dumps, max_by_key_first, py_int, Indent, JsonOptions};
use serde_json::{Map, Value};
use std::fmt;
use std::io;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleMatch {
    Exact,
    Prefix,
    Contains,
}

impl RuleMatch {
    pub fn as_str(self) -> &'static str {
        match self {
            RuleMatch::Exact => "exact",
            RuleMatch::Prefix => "prefix",
            RuleMatch::Contains => "contains",
        }
    }

    fn parse(text: &str) -> Option<RuleMatch> {
        RULE_MATCHES.iter().copied().find(|item| item.as_str() == text)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleAction {
    Convert,
    Keep,
}

impl RuleAction {
    pub fn as_str(self) -> &'static str {
        match self {
            RuleAction::Convert => "convert",
            RuleAction::Keep => "keep",
        }
    }

    fn parse(text: &str) -> Option<RuleAction> {
        RULE_ACTIONS.iter().copied().find(|item| item.as_str() == text)
    }
}

/// The order the rule window offers the conditions in, widest first. When several rules fit one word the most specific
/// decides, so the position is also the rank: a whole word before a beginning, a beginning before a part.
pub const RULE_MATCHES: [RuleMatch; 3] = [RuleMatch::Contains, RuleMatch::Prefix, RuleMatch::Exact];
pub const RULE_ACTIONS: [RuleAction; 2] = [RuleAction::Convert, RuleAction::Keep];

/// A rule the store refuses to keep (`InvalidRule`, a `ValueError`): the sentence the rule window shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidRule(pub String);

impl fmt::Display for InvalidRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for InvalidRule {}

/// What a store call can fail with: a refused rule, a file error, or the `ValueError` a legacy file's odd key makes.
#[derive(Debug)]
pub enum LearningError {
    Invalid(InvalidRule),
    Io(io::Error),
    /// A `ValueError` that the Python lets out of `load`: `int()` of a key that `isdigit` calls a number (`²:word`).
    Value(String),
}

impl fmt::Display for LearningError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LearningError::Invalid(error) => error.fmt(f),
            LearningError::Io(error) => error.fmt(f),
            LearningError::Value(text) => f.write_str(text),
        }
    }
}

impl std::error::Error for LearningError {}

impl From<InvalidRule> for LearningError {
    fn from(error: InvalidRule) -> Self {
        LearningError::Invalid(error)
    }
}

impl From<io::Error> for LearningError {
    fn from(error: io::Error) -> Self {
        LearningError::Io(error)
    }
}

/// One rule as the rule window and the settings list show it.
///
/// `pattern` is the text as typed in `source_group`'s layout: the same keys typed in the other layout are a different
/// word and need their own rule. `target_group` is the layout a `convert` rule switches to; a `keep` rule names it too,
/// as the layout the word is not taken to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LearnedRule {
    pub pattern: String,
    pub source_group: i64,
    pub target_group: i64,
    pub action: RuleAction,
    pub r#match: RuleMatch,
    pub case_sensitive: bool,
}

/// `(source_group, match, case_sensitive, key)`: two rules with one identity cannot both exist.
pub type RuleIdentity = (i64, RuleMatch, bool, String);

impl LearnedRule {
    /// `LearnedRule(pattern, source, target)`: a whole-word, case-insensitive "convert" rule.
    pub fn new(pattern: &str, source_group: i64, target_group: i64) -> LearnedRule {
        LearnedRule {
            pattern: pattern.to_string(),
            source_group,
            target_group,
            action: RuleAction::Convert,
            r#match: RuleMatch::Exact,
            case_sensitive: false,
        }
    }

    pub fn key(&self) -> String {
        if self.case_sensitive { self.pattern.clone() } else { casefold(&self.pattern) }
    }

    /// Two rules with one identity cannot both exist: the later replaces the earlier.
    pub fn identity(&self) -> RuleIdentity {
        (self.source_group, self.r#match, self.case_sensitive, self.key())
    }

    pub fn matches(&self, word: &str) -> bool {
        let text = if self.case_sensitive { word.to_string() } else { casefold(word) };
        let key = self.key();
        match self.r#match {
            RuleMatch::Exact => text == key,
            RuleMatch::Prefix => text.starts_with(&key),
            RuleMatch::Contains => text.contains(&key),
        }
    }

    /// Whether a word that begins with `prefix` can still meet this rule.
    ///
    /// A rule on a part of the word can be met by letters not typed yet, so it answers yes only once the typed letters
    /// already lead into the part.
    pub fn may_match_continuation(&self, prefix: &str) -> bool {
        let text = if self.case_sensitive { prefix.to_string() } else { casefold(prefix) };
        let key = self.key();
        match self.r#match {
            RuleMatch::Exact => key.starts_with(&text),
            RuleMatch::Prefix => key.starts_with(&text) || text.starts_with(&key),
            RuleMatch::Contains => {
                text.contains(&key)
                    || text.char_indices().any(|(start, _)| key.starts_with(&text[start..]))
                    // range(len(text)) starts at every character, and never at the end: no empty suffix.
            }
        }
    }

    /// `(rank of the condition, length of the pattern, is a "keep" rule)`: the greater decides.
    pub fn specificity(&self) -> (usize, usize, bool) {
        let rank = RULE_MATCHES.iter().position(|item| *item == self.r#match).expect("a known condition");
        (rank, self.pattern.chars().count(), self.action == RuleAction::Keep)
    }
}

/// Raise `InvalidRule` for a rule that cannot work as intended.
pub fn validate_rule(rule: &LearnedRule) -> Result<(), InvalidRule> {
    let refuse = |text: &str| Err(InvalidRule(text.to_string()));
    if rule.pattern.is_empty() || rule.pattern != strip(&rule.pattern) {
        return refuse("Сочетание букв не может быть пустым или начинаться и кончаться пробелом");
    }
    if rule.pattern.chars().any(|character| isspace(&character.to_string())) {
        return refuse("Сочетание букв набирается одним словом, без пробелов");
    }
    if !rule.pattern.chars().any(ks_text::ucd::isalpha) {
        return refuse("В сочетании нужна хотя бы одна буква");
    }
    if rule.pattern.chars().count() > MAX_WORD_STROKES {
        return refuse("Сочетание длиннее самого длинного слова, которое отслеживает KeySwitch");
    }
    if rule.source_group == rule.target_group || rule.source_group.min(rule.target_group) < 0 {
        return refuse("Правило должно связывать две разные раскладки");
    }
    // The action and the condition are enums: "Неизвестное условие или действие правила" cannot happen.
    Ok(())
}

/// A rule from a JSON value of the file, or `None` for one that is not a rule.
fn rule_from_json(value: &Value) -> Option<LearnedRule> {
    let object = value.as_object()?;
    let pattern = object.get("pattern")?.as_str()?;
    let source = crate::pycompat::as_py_int(object.get("source_group")?)?;
    let target = crate::pycompat::as_py_int(object.get("target_group")?)?;
    let action = RuleAction::parse(object.get("action")?.as_str()?)?;
    let r#match = RuleMatch::parse(object.get("match")?.as_str()?)?;
    let case_sensitive = match object.get("case_sensitive") {
        None => false,
        Some(Value::Bool(flag)) => *flag,
        Some(_) => return None,
    };
    let rule = LearnedRule { pattern: pattern.to_string(), source_group: source, target_group: target, action, r#match, case_sensitive };
    validate_rule(&rule).ok()?;
    Some(rule)
}

fn rule_to_json(rule: &LearnedRule) -> Value {
    let mut map = Map::new();
    map.insert("pattern".into(), Value::String(rule.pattern.clone()));
    map.insert("source_group".into(), Value::from(rule.source_group));
    map.insert("target_group".into(), Value::from(rule.target_group));
    map.insert("action".into(), Value::String(rule.action.as_str().to_string()));
    map.insert("match".into(), Value::String(rule.r#match.as_str().to_string()));
    map.insert("case_sensitive".into(), Value::Bool(rule.case_sensitive));
    Value::Object(map)
}

/// `(group, word)` of a legacy key `"<digits>:<word>"`.
fn legacy_identity(key: &str) -> Result<Option<(i64, String)>, LearningError> {
    let Some((group, word)) = key.split_once(':') else { return Ok(None) };
    if !isdigit(group) {
        return Ok(None);
    }
    // isdigit() is true for `²` and its kind, and int() of them raises ValueError, which nothing in the Python catches.
    match py_int(group) {
        Some(number) => Ok(i64::try_from(number).ok().map(|number| (number, word.to_string()))),
        None => Err(LearningError::Value(format!("invalid literal for int() with base 10: {group:?}"))),
    }
}

/// Rules from a learning file of schema 2 and older.
///
/// Such a file kept a counter per word and a list of forbidden directions. A word whose counter had reached the
/// threshold acted as a rule and stays one, as a whole-word "convert" rule; a forbidden direction becomes a whole-word
/// "keep" rule. A counter still short of the threshold never acted and is not carried over: the old versions wrote one
/// on every manual conversion, without asking.
pub fn migrate_legacy(payload: &Map<String, Value>, confirmations_required: i64) -> Result<Vec<LearnedRule>, LearningError> {
    let mut rules: Vec<LearnedRule> = Vec::new();
    if let Some(Value::Object(stored)) = payload.get("rules") {
        for (key, value) in stored {
            let Some(identity) = legacy_identity(key)? else { continue };
            let Some(value) = value.as_object() else { continue };
            let target = value.get("target_group").and_then(crate::pycompat::as_py_int);
            // value.get("confirmations", 0): absent is 0, anything but an int is skipped.
            let confirmations = match value.get("confirmations") {
                None => Some(0),
                Some(other) => crate::pycompat::as_py_int(other),
            };
            let (Some(target), Some(confirmations)) = (target, confirmations) else { continue };
            if confirmations < confirmations_required.max(1) {
                continue;
            }
            rules.push(LearnedRule::new(&identity.1, identity.0, target));
        }
    }
    if let Some(Value::Object(rejections)) = payload.get("rejections") {
        for (key, value) in rejections {
            let Some(identity) = legacy_identity(key)? else { continue };
            let Some(targets) = value.as_array() else { continue };
            for target in targets {
                if let Some(target) = crate::pycompat::as_py_int(target) {
                    let mut rule = LearnedRule::new(&identity.1, identity.0, target);
                    rule.action = RuleAction::Keep;
                    rules.push(rule);
                }
            }
        }
    }
    // dict keyed by identity: the first place of an identity stays, the last "keep" (or first rule) is its value.
    let mut valid: Vec<(RuleIdentity, LearnedRule)> = Vec::new();
    for rule in rules {
        if validate_rule(&rule).is_err() {
            continue;
        }
        let identity = rule.identity();
        match valid.iter_mut().find(|(held, _)| *held == identity) {
            // A word both forced and forbidden ended up untouched by the old engine (the forbidden direction removed
            // the rule): the "keep" rule says that.
            Some(slot) => {
                if rule.action == RuleAction::Keep {
                    slot.1 = rule;
                }
            }
            None => valid.push((identity, rule)),
        }
    }
    Ok(valid.into_iter().map(|(_, rule)| rule).collect())
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct State {
    rules: Vec<LearnedRule>,
    migrated: usize,
    unreadable: Option<PathBuf>,
}

/// Persist the user's switching rules in `learning.json`.
///
/// `legacy_confirmations` is the threshold an older learning file was used with; it only decides which of its counters
/// were rules when the file is converted to the current schema.
pub struct LearningStore {
    pub path: PathBuf,
    legacy_confirmations: i64,
    state: Mutex<State>,
}

impl LearningStore {
    /// `LearningStore(path, legacy_confirmations=...)`; the file is read at once.
    pub fn new(path: Option<PathBuf>, legacy_confirmations: Option<i64>) -> Result<LearningStore, LearningError> {
        let store = LearningStore {
            path: path.unwrap_or_else(|| data_dir().join("learning.json")),
            legacy_confirmations: legacy_confirmations.unwrap_or(LEGACY_RULE_CONFIRMATIONS_REQUIRED),
            state: Mutex::new(State { rules: Vec::new(), migrated: 0, unreadable: None }),
        };
        store.load()?;
        Ok(store)
    }

    /// How many rules the last load carried over from an older file.
    pub fn migrated(&self) -> usize {
        lock(&self.state).migrated
    }

    /// Where an unreadable learning file was moved by the last load, if anywhere.
    pub fn unreadable(&self) -> Option<PathBuf> {
        lock(&self.state).unreadable.clone()
    }

    pub fn load(&self) -> Result<(), LearningError> {
        let mut state = lock(&self.state);
        state.rules = Vec::new();
        if !self.path.is_file() {
            return Ok(());
        }
        let mut raw: Vec<u8> = Vec::new();
        let payload: Option<Value> = match std::fs::read(&self.path) {
            Ok(bytes) => {
                raw = bytes;
                // utf-8-sig: Notepad saves UTF-8 with a byte order mark, which json rejects.
                decode_text(&raw, true).and_then(|text| json_loads(&text))
            }
            Err(_) => None,
        };
        let payload_object = payload.as_ref().and_then(Value::as_object);
        let stored = payload_object.and_then(|payload| payload.get("rules"));
        if let Some(Value::Array(stored)) = stored {
            for value in stored {
                if let Some(rule) = rule_from_json(value) {
                    Self::insert(&mut state.rules, rule);
                }
            }
            return Ok(());
        }
        let legacy = payload_object.filter(|payload| {
            matches!(stored, Some(Value::Object(_))) || matches!(payload.get("rejections"), Some(Value::Object(_)))
        });
        let Some(payload) = legacy else {
            // The next rule would save over the file: keep the user's rules aside.
            state.unreadable = set_aside_unreadable(&self.path);
            return Ok(());
        };
        for rule in migrate_legacy(payload, self.legacy_confirmations)? {
            Self::insert(&mut state.rules, rule);
        }
        state.migrated = state.rules.len();
        let backup = with_name_suffix(&self.path, LEARNING_STORE_BACKUP_SUFFIX);
        let saved: io::Result<()> = (|| {
            if !backup.exists() {
                std::fs::write(&backup, &raw)?;
            }
            self.save_locked(&state.rules)
        })();
        // An OSError here ends the load; the rules are kept in memory.
        let _ = saved;
        Ok(())
    }

    fn save_locked(&self, rules: &[LearnedRule]) -> io::Result<()> {
        let mut data = Map::new();
        data.insert("schema_version".into(), Value::from(LEARNING_STORE_SCHEMA_VERSION));
        data.insert("rules".into(), Value::Array(rules.iter().map(rule_to_json).collect()));
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = with_suffix(&self.path, ".json.tmp");
        let options =
            JsonOptions { ensure_ascii: false, indent: Some(Indent::Spaces(USER_DATA_JSON_INDENT)), ..JsonOptions::default() };
        let mut text = json_dumps(&Value::Object(data), &options);
        text.push('\n');
        write_text(&temporary, &text)?;
        std::fs::rename(&temporary, &self.path)
    }

    pub fn save(&self) -> io::Result<()> {
        let state = lock(&self.state);
        self.save_locked(&state.rules)
    }

    fn insert(rules: &mut Vec<LearnedRule>, rule: LearnedRule) {
        let identity = rule.identity();
        rules.retain(|item| item.identity() != identity);
        rules.push(rule);
    }

    /// Every rule, in the order the settings list shows them.
    pub fn rules(&self) -> Vec<LearnedRule> {
        let mut stored = lock(&self.state).rules.clone();
        // A stable sort on (pattern.casefold(), source_group, match as text, case_sensitive).
        stored.sort_by(|a, b| {
            casefold(&a.pattern)
                .cmp(&casefold(&b.pattern))
                .then(a.source_group.cmp(&b.source_group))
                .then(a.r#match.as_str().cmp(b.r#match.as_str()))
                .then(a.case_sensitive.cmp(&b.case_sensitive))
        });
        stored
    }

    pub fn count(&self) -> usize {
        lock(&self.state).rules.len()
    }

    /// Keep a rule; one with the same pattern and condition is replaced.
    pub fn add_rule(&self, rule: LearnedRule) -> Result<LearnedRule, LearningError> {
        validate_rule(&rule)?;
        let mut state = lock(&self.state);
        Self::insert(&mut state.rules, rule.clone());
        self.save_locked(&state.rules)?;
        Ok(rule)
    }

    /// Store an edited rule in place of the one the user opened.
    pub fn replace_rule(&self, old: &LearnedRule, new: LearnedRule) -> Result<LearnedRule, LearningError> {
        validate_rule(&new)?;
        let mut state = lock(&self.state);
        state.rules.retain(|item| item != old);
        Self::insert(&mut state.rules, new.clone());
        self.save_locked(&state.rules)?;
        Ok(new)
    }

    /// Forget one rule; true when it was there.
    pub fn remove_rule(&self, rule: &LearnedRule) -> io::Result<bool> {
        let mut state = lock(&self.state);
        let remaining: Vec<LearnedRule> = state.rules.iter().filter(|item| *item != rule).cloned().collect();
        if remaining.len() == state.rules.len() {
            return Ok(false);
        }
        state.rules = remaining;
        self.save_locked(&state.rules)?;
        Ok(true)
    }

    pub fn clear(&self) -> io::Result<()> {
        let mut state = lock(&self.state);
        state.rules = Vec::new();
        self.save_locked(&state.rules)
    }

    /// The rule that decides about `word` typed in `source_group`, if any.
    pub fn r#match(&self, source_group: i64, word: &str) -> Option<LearnedRule> {
        if word.is_empty() {
            return None;
        }
        let fitting: Vec<LearnedRule> = lock(&self.state)
            .rules
            .iter()
            .filter(|rule| rule.source_group == source_group && rule.matches(word))
            .cloned()
            .collect();
        max_by_key_first(fitting, LearnedRule::specificity)
    }

    pub fn forced_target(&self, source_group: i64, word: &str) -> Option<i64> {
        self.r#match(source_group, word).filter(|rule| rule.action == RuleAction::Convert).map(|rule| rule.target_group)
    }

    pub fn keeps(&self, source_group: i64, word: &str) -> bool {
        self.r#match(source_group, word).is_some_and(|rule| rule.action == RuleAction::Keep)
    }

    /// Whether a "keep" rule may still apply once the word started by `prefix` ends.
    pub fn keeps_continuation(&self, source_group: i64, prefix: &str) -> bool {
        lock(&self.state).rules.iter().any(|rule| {
            rule.action == RuleAction::Keep && rule.source_group == source_group && rule.may_match_continuation(prefix)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn directory(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("ks-learning-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn rule(pattern: &str, matching: RuleMatch, action: RuleAction) -> LearnedRule {
        LearnedRule { pattern: pattern.into(), source_group: 0, target_group: 1, action, r#match: matching, case_sensitive: false }
    }

    #[test]
    fn conditions() {
        let exact = rule("Abc", RuleMatch::Exact, RuleAction::Convert);
        assert!(exact.matches("ABC") && !exact.matches("abcd"));
        assert!(rule("ab", RuleMatch::Prefix, RuleAction::Keep).matches("abcd"));
        assert!(rule("bc", RuleMatch::Contains, RuleAction::Keep).matches("abcd"));
        let sensitive = LearnedRule { case_sensitive: true, ..exact.clone() };
        assert!(!sensitive.matches("abc") && sensitive.matches("Abc"));
        let part = rule("tion", RuleMatch::Contains, RuleAction::Keep);
        assert!(part.may_match_continuation("xxti") && part.may_match_continuation("xtion") && !part.may_match_continuation("zz"));
        assert!(rule("abc", RuleMatch::Prefix, RuleAction::Keep).may_match_continuation("abcdef"));
        assert!(rule("abc", RuleMatch::Exact, RuleAction::Keep).may_match_continuation("ab"));
        assert!(!rule("abc", RuleMatch::Exact, RuleAction::Keep).may_match_continuation("abd"));
    }

    #[test]
    fn rules_that_cannot_work_are_refused() {
        let ok = rule("abc", RuleMatch::Exact, RuleAction::Convert);
        assert!(validate_rule(&ok).is_ok());
        for pattern in ["", " abc", "a b", "123", &"x".repeat(257)] {
            assert!(validate_rule(&rule(pattern, RuleMatch::Exact, RuleAction::Convert)).is_err(), "{pattern:?}");
        }
        assert!(validate_rule(&LearnedRule { target_group: 0, ..ok.clone() }).is_err());
        assert!(validate_rule(&LearnedRule { source_group: -1, target_group: 2, ..ok }).is_err());
    }

    #[test]
    fn kept_replaced_forgotten_and_the_most_specific_decides() {
        let path = directory("kept").join("learning.json");
        let store = LearningStore::new(Some(path.clone()), None).unwrap();
        store.add_rule(rule("ghbdtn", RuleMatch::Exact, RuleAction::Convert)).unwrap();
        store.add_rule(rule("ghb", RuleMatch::Prefix, RuleAction::Keep)).unwrap();
        store.add_rule(rule("hb", RuleMatch::Contains, RuleAction::Keep)).unwrap();
        assert_eq!(store.count(), 3);
        assert_eq!(store.r#match(0, "GHBDTN").unwrap().pattern, "ghbdtn");
        assert_eq!(store.forced_target(0, "ghbdtn"), Some(1));
        assert!(store.keeps(0, "ghbxyz") && !store.keeps(1, "ghbxyz"));
        assert!(store.keeps_continuation(0, "gh"));
        assert!(store.r#match(0, "").is_none());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("{\n  \"schema_version\": 3,\n  \"rules\": [\n    {\n      \"pattern\": \"ghbdtn\","));
        let again = LearningStore::new(Some(path), None).unwrap();
        assert_eq!(again.rules(), store.rules());
        assert_eq!(again.rules()[0].pattern, "ghb");
        assert!(again.remove_rule(&rule("hb", RuleMatch::Contains, RuleAction::Keep)).unwrap());
        assert!(!again.remove_rule(&rule("hb", RuleMatch::Contains, RuleAction::Keep)).unwrap());
        again.clear().unwrap();
        assert_eq!(again.count(), 0);
    }

    #[test]
    fn a_foreign_file_is_kept_aside_and_a_bom_is_read() {
        let dir = directory("foreign");
        let path = dir.join("learning.json");
        std::fs::write(&path, "[1, 2]").unwrap();
        let store = LearningStore::new(Some(path.clone()), None).unwrap();
        assert_eq!(store.count(), 0);
        assert!(store.unreadable().is_some() && !path.exists());
        let mut with_bom = b"\xEF\xBB\xBF".to_vec();
        with_bom.extend_from_slice(br#"{"schema_version": 3, "rules": [{"pattern": "abc", "source_group": 0, "target_group": 1, "action": "keep", "match": "prefix"}, {"pattern": "bad pattern"}, 5]}"#);
        std::fs::write(&path, with_bom).unwrap();
        let store = LearningStore::new(Some(path), None).unwrap();
        assert_eq!(store.rules(), vec![rule("abc", RuleMatch::Prefix, RuleAction::Keep)]);
    }

    #[test]
    fn an_old_file_becomes_rules_and_is_kept_aside() {
        let dir = directory("legacy");
        let path = dir.join("learning.json");
        let old = json!({
            "schema_version": 2,
            "rules": {"0:ghbdtn": {"target_group": 1, "confirmations": 3}, "0:cold": {"target_group": 1, "confirmations": 1}, "bad": {}, "1:ПРИВЕТ": {"target_group": 0}},
            "rejections": {"0:ghbdtn": [1], "1:xx": [0, "z"]}
        });
        std::fs::write(&path, old.to_string()).unwrap();
        let store = LearningStore::new(Some(path.clone()), None).unwrap();
        // ghbdtn is forced and forbidden: the "keep" rule stands, in the place of the first.
        assert_eq!(store.migrated(), 2);
        let rules = store.rules();
        assert_eq!(rules.iter().map(|r| (r.pattern.as_str(), r.action)).collect::<Vec<_>>(), [("ghbdtn", RuleAction::Keep), ("xx", RuleAction::Keep)]);
        assert_eq!(std::fs::read_to_string(dir.join("learning.json.v2-backup")).unwrap(), old.to_string());
        assert!(std::fs::read_to_string(&path).unwrap().contains("\"schema_version\": 3"));
    }

    #[test]
    fn a_legacy_key_that_isdigit_but_is_no_int_stops_the_load() {
        let dir = directory("superscript");
        let path = dir.join("learning.json");
        std::fs::write(&path, r#"{"rules": {"²:word": {"target_group": 1, "confirmations": 5}}}"#).unwrap();
        assert!(matches!(LearningStore::new(Some(path), None), Err(LearningError::Value(_))));
    }
}
