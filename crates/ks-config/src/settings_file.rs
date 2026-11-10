//! The settings file and the JSON tree it is held in: read, merged over the defaults, saved, and handed out.
//!
//! A settings file is written by this program and by older versions of it, and edited by hand. Keys the program does not
//! know are kept at every nesting level, and a known key holding a value of another type stays as it is, so the tree is
//! what the file is, and its shape is known here and nowhere else: the rest of the program reaches a setting through a
//! typed [`Key`] (`constants.rs` has one for every setting).
//!
//! Python's `==`, truth value and `int()` over the values of the file live here too, because this is where they are
//! asked. The dotted-path calls that take and return the tree (`SettingsStore::get`, `set`, `with_value`, `subscribe`,
//! `snapshot` and the small readers next to them) serve the crates that are not typed yet; they go away as those crates
//! move to the keys.
#![expect(clippy::disallowed_types, clippy::disallowed_methods, reason = "the settings file keeps unknown keys and values of any type at every nesting level, read and written as the tree they are; the dotted-path calls of the crates not yet typed stay here until they move to Key<T>")]
#![cfg_attr(test, expect(clippy::disallowed_macros, reason = "the tests build the trees a hand-edited file holds"))]

use crate::config::{ChangeCallback, SettingsStore, lock, set_aside_unreadable};
use crate::constants::{
    DEFAULT_CONVERT_LAST_HOTKEY, DEFAULT_HISTORY_LIMIT, DEFAULT_PAUSE_DELAY_SECONDS, DEFAULT_SETTINGS, EARLY_SWITCH_DEFAULT_SCHEMA_VERSION,
    HOTKEY_CONVERT_LAST, LOGGED_SETTING_VALUE_MAX_CHARACTERS, MACOS_CONVERT_HOTKEY_SCHEMA_VERSION, MACOS_HOTKEY_DEFAULTS,
    PAUSE_DELAY_DEFAULT_SCHEMA_VERSION, PREVIOUS_DEFAULT_PAUSE_DELAY_SECONDS, SETTINGS_SINCE_PYTHON, SettingChange, USER_DATA_JSON_INDENT,
    for_each_setting,
};
use crate::indicator::INDICATOR_STYLES;
use crate::pycompat::{ReadError, create_parent, read_text, with_suffix, write_text};
use crate::settings_diagnostics::{LoggedValue, Operation, SettingChangeRecord};
use indexmap::IndexMap;
use ks_cast::To as _;
use ks_text::pystr;
use ks_text::{Indent, JsonOptions, json_dumps, py_float, py_int};
use serde_json::{Map, Number, Value};
use sha2::{Digest, Sha256};
use std::io;
use std::sync::Arc;

/// `dict[str, object]`: a JSON object, in the order of its file.
pub type SettingsData = Map<String, Value>;
/// `Callable[[str, object], None]`: called with the dotted path that changed (`"*"` for a reset) and its new value.
pub type SettingCallback = Arc<dyn Fn(&str, &Value) + Send + Sync>;

/// `_deep_merge`: the defaults with every loaded key laid over them; mappings on both sides are merged, anything else
/// is replaced. Keys the defaults do not know are kept, after the known ones.
pub(crate) fn deep_merge(defaults: &SettingsData, loaded: &SettingsData) -> SettingsData {
    let mut result = defaults.clone();
    for (key, value) in loaded {
        let merged = match (value.as_object(), result.get(key).and_then(Value::as_object)) {
            (Some(loaded_mapping), Some(default_mapping)) => Value::Object(deep_merge(default_mapping, loaded_mapping)),
            _ => value.clone(),
        };
        result.insert(key.clone(), merged);
    }
    result
}

/// The shipped settings of `platform` (the running one by default, `Some("darwin")` for macOS): `DEFAULT_SETTINGS`
/// everywhere, but on macOS the conversion and selection hotkeys use keys a Mac keyboard has (`MACOS_HOTKEY_DEFAULTS`),
/// in schema `MACOS_CONVERT_HOTKEY_SCHEMA_VERSION`.
pub fn default_settings(platform: Option<&str>) -> SettingsData {
    let mut defaults = DEFAULT_SETTINGS.clone();
    let running = if cfg!(target_os = "macos") { "darwin" } else { "other" };
    if platform.unwrap_or(running) == "darwin" {
        let hotkeys = defaults.get_mut("hotkeys").and_then(Value::as_object_mut).expect("the defaults have a hotkeys table");
        for (name, value) in MACOS_HOTKEY_DEFAULTS {
            hotkeys.insert(name.to_string(), Value::String(value.to_string()));
        }
        defaults.insert("schema_version".into(), Value::from(MACOS_CONVERT_HOTKEY_SCHEMA_VERSION));
    }
    defaults
}

/// `json.loads(text)`, or `None` for a `ValueError` (see the note in `pycompat` about the few texts Python reads and
/// `serde_json` does not).
#[must_use]
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
        Value::Null | Value::String(_) | Value::Array(_) | Value::Object(_) => None,
    }
}

/// Python `a == b` for JSON values: `True == 1 == 1.0`, strings by code points, lists in order, dicts in any order.
#[must_use]
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
                f.fract() == 0.0 && f.abs() < 1.0e38 && f.to::<i128>() == i
            }
            _ => false,
        },
    }
}

/// Python `bool(value)` of a JSON value.
#[must_use]
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
#[must_use]
pub fn as_py_int(value: &Value) -> Option<i64> {
    match value {
        Value::Number(n) => n.as_i64(),
        Value::Null | Value::Bool(_) | Value::String(_) | Value::Array(_) | Value::Object(_) => None,
    }
}

/// `isinstance(value, int) and not isinstance(value, bool)`, whatever the size (a float is not an int).
#[must_use]
pub fn is_py_int(value: &Value) -> bool {
    matches!(value, Value::Number(n) if n.is_i64() || n.is_u64())
}

/// `history_limit(value)`: the `history.limit` setting as a count of entries.
///
/// The settings file can be edited by hand; a value that is not a number falls back to the default instead of
/// stopping the application at start. A count beyond 64 bits is kept at the largest one.
#[must_use]
pub fn history_limit(value: &Value) -> i64 {
    let parsed: Option<i128> = match value {
        Value::Number(n) => match n.as_i64() {
            Some(i) => Some(i128::from(i)),
            None => match n.as_u64() {
                Some(u) => Some(i128::from(u)),
                // int(float): the fraction is cut off; an infinity or a NaN has no int.
                None => n.as_f64().filter(|f| f.is_finite()).map(|f| f.trunc().to::<i128>()),
            },
        },
        Value::String(text) => py_int(text),
        Value::Null | Value::Bool(_) | Value::Array(_) | Value::Object(_) => None,
    };
    match parsed {
        Some(count) => (count.clamp(1, i128::from(i64::MAX))).to::<i64>(),
        None => DEFAULT_HISTORY_LIMIT,
    }
}

/// `normalize_indicator_style(value)`: `str(value)` when it is a known style, "letters" otherwise. Only a string can
/// read as a style, whatever else `str` would make of the value.
#[must_use]
pub fn normalize_indicator_style(value: &Value) -> &'static str {
    match value {
        Value::String(style) => INDICATOR_STYLES.iter().copied().find(|known| known == style).unwrap_or("letters"),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => "letters",
    }
}

#[must_use]
pub fn layout_icon_name(style: &Value, group: i64) -> &'static str {
    match (normalize_indicator_style(style), group) {
        ("letters", 0) => "keyswitch-en",
        ("letters", 1) => "keyswitch-ru",
        ("flags", 0) => "keyswitch-flag-us",
        ("flags", 1) => "keyswitch-flag-ru",
        _ => "keyswitch",
    }
}

/// `history.limit` read the way `history_limit` reads it: any number or numeric text, kept at least 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryLimit(pub i64);

/// How a value of a setting is read out of the file and put back into it. A file may hold anything under a known key, so
/// each type reads a stored value the way the program has always read that kind of setting: `bool` is Python's truth value
/// of whatever is stored (`flag`), `String` a text as it is and anything else its JSON text (`text_setting`), `f64` and
/// `i64` a number, a flag or a numeric text (`bounded_setting`), `Vec<String>` the texts of a list. These are the engine's
/// readings; some readers of the settings windows differ on a value of another type (`null` as a theme, a list holding a
/// number) and keep their own reading until they are converted. `None` is a value the type cannot read at all: the caller
/// falls back as it does for a setting that is not there.
pub(crate) trait SettingValue: Sized {
    fn from_tree(tree: &Value) -> Option<Self>;
    fn into_tree(self) -> Value;
    /// Whether a default of the table has this type (the test of the table): stricter than reading, which forgives.
    #[cfg(test)]
    fn fits_default(tree: &Value) -> bool;
}

impl SettingValue for bool {
    fn from_tree(tree: &Value) -> Option<bool> {
        Some(truthy(tree))
    }

    fn into_tree(self) -> Value {
        Value::Bool(self)
    }

    #[cfg(test)]
    fn fits_default(tree: &Value) -> bool {
        tree.is_boolean()
    }
}

impl SettingValue for String {
    fn from_tree(tree: &Value) -> Option<String> {
        Some(match tree {
            Value::String(text) => text.clone(),
            other @ (Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_)) => other.to_string(),
        })
    }

    fn into_tree(self) -> Value {
        Value::String(self)
    }

    #[cfg(test)]
    fn fits_default(tree: &Value) -> bool {
        tree.is_string()
    }
}

impl SettingValue for f64 {
    fn from_tree(tree: &Value) -> Option<f64> {
        match tree {
            Value::Bool(flag) => Some(f64::from(u8::from(*flag))),
            Value::Number(number) => number.as_f64(),
            Value::String(text) => py_float(text),
            Value::Null | Value::Array(_) | Value::Object(_) => None,
        }
    }

    fn into_tree(self) -> Value {
        Value::from(self)
    }

    #[cfg(test)]
    fn fits_default(tree: &Value) -> bool {
        tree.is_number()
    }
}

impl SettingValue for i64 {
    fn from_tree(tree: &Value) -> Option<i64> {
        // The cast saturates (a NaN is 0, an infinity the largest): the range the caller brings it into is what
        // `bounded_setting` brings the float into, and a whole number in range comes out the same.
        f64::from_tree(tree).map(ks_cast::To::to::<i64>)
    }

    fn into_tree(self) -> Value {
        Value::from(self)
    }

    #[cfg(test)]
    fn fits_default(tree: &Value) -> bool {
        tree.is_i64()
    }
}

impl SettingValue for Vec<String> {
    fn from_tree(tree: &Value) -> Option<Vec<String>> {
        tree.as_array().map(|items| items.iter().filter_map(|item| item.as_str().map(str::to_owned)).collect())
    }

    fn into_tree(self) -> Value {
        Value::from(self)
    }

    #[cfg(test)]
    fn fits_default(tree: &Value) -> bool {
        tree.as_array().is_some_and(|items| items.iter().all(Value::is_string))
    }
}

impl SettingValue for HistoryLimit {
    fn from_tree(tree: &Value) -> Option<HistoryLimit> {
        Some(HistoryLimit(history_limit(tree)))
    }

    fn into_tree(self) -> Value {
        Value::from(self.0)
    }

    #[cfg(test)]
    fn fits_default(tree: &Value) -> bool {
        tree.is_i64()
    }
}

/// A setting: its dotted path and how a value of type `T` is read from the file and written to it.
pub struct Key<T> {
    path: &'static str,
    read: fn(&Value) -> Option<T>,
    write: fn(T) -> Value,
}

impl<T> Clone for Key<T> {
    fn clone(&self) -> Key<T> {
        *self
    }
}

impl<T> Copy for Key<T> {}

impl<T> Key<T> {
    pub(crate) const fn new(path: &'static str, read: fn(&Value) -> Option<T>, write: fn(T) -> Value) -> Key<T> {
        Key { path, read, write }
    }

    /// The setting's dotted path.
    #[must_use]
    pub const fn path(self) -> &'static str {
        self.path
    }
}

/// `constants.rs` names every setting with this macro's help: for a row of the table it writes the `Key` constant, and the
/// variant of `SettingChange` that the row's setting takes when it changes.
macro_rules! setting_keys {
    ($($constant:ident, $variant:ident, $value:ty, $path:literal;)*) => {
        $(pub const $constant: $crate::settings_file::Key<$value> = $crate::settings_file::Key::new(
            $path,
            <$value as $crate::settings_file::SettingValue>::from_tree,
            <$value as $crate::settings_file::SettingValue>::into_tree,
        );)*

        /// What changed in the settings: a setting of the table with its new value, typed as its key is, or everything
        /// at once.
        #[derive(Clone, Debug, PartialEq)]
        pub enum SettingChange {
            /// `reset()`: every setting is back at its default.
            Reset,
            /// A change no row of the table reads: a path outside the table (a whole table, a key the program does not
            /// know) or a value its key cannot read. Whoever must notice every change hears of these too.
            Other { path: String, value: ks_text::json::Kept },
            $($variant($value),)*
        }

        impl SettingChange {
            /// The dotted path of the setting that changed, `"*"` for a reset.
            #[must_use]
            pub fn path(&self) -> &str {
                match self {
                    SettingChange::Reset => "*",
                    SettingChange::Other { path, .. } => path,
                    $(SettingChange::$variant(_) => $path,)*
                }
            }
        }
    };
}
pub(crate) use setting_keys;

/// One row of the table of settings, for what is done with it without knowing its type.
pub(crate) struct SettingRow {
    pub(crate) path: &'static str,
    parse: fn(&Value) -> Option<SettingChange>,
    #[cfg(test)]
    fits_default: fn(&Value) -> bool,
}

macro_rules! setting_rows {
    ($($constant:ident, $variant:ident, $value:ty, $path:literal;)*) => {
        /// Every setting of the table, in the order of `DEFAULT_SETTINGS`.
        pub(crate) const SETTING_ROWS: &[SettingRow] = &[
            $(SettingRow {
                path: $path,
                parse: |tree| <$value as SettingValue>::from_tree(tree).map(SettingChange::$variant),
                #[cfg(test)]
                fits_default: <$value as SettingValue>::fits_default,
            },)*
        ];

        impl SettingChange {
            /// The new value as the file would hold it (`null` for a reset, which has no value of its own).
            pub(crate) fn stored(&self) -> Value {
                match self {
                    SettingChange::Reset => Value::Null,
                    SettingChange::Other { value, .. } => serde_json::to_value(value).expect("a kept value is JSON"),
                    $(SettingChange::$variant(value) => <$value as SettingValue>::into_tree(value.clone()),)*
                }
            }
        }
    };
}
for_each_setting!(setting_rows);

/// The typed news of a change: the setting's row reads it, as the value it holds now; a path the table does not have (the
/// untyped `set` of a whole table or of a key the program does not know) or a value its key cannot read is `Other`.
fn change_of(path: &str, value: &Value) -> SettingChange {
    if path == "*" {
        return SettingChange::Reset;
    }
    SETTING_ROWS.iter().find(|row| row.path == path).and_then(|row| (row.parse)(value)).unwrap_or_else(|| SettingChange::Other {
        path: path.to_owned(),
        value: serde_json::from_value(value.clone()).expect("a tree reads as a kept tree"),
    })
}

/// Who wants to know about a change.
#[derive(Clone)]
pub(crate) enum Subscriber {
    /// A crate not yet typed: called with the dotted path and the tree.
    Tree(SettingCallback),
    Change(ChangeCallback),
}

impl Subscriber {
    fn notify(&self, path: &str, value: &Value) {
        match self {
            Subscriber::Tree(callback) => callback(path, value),
            Subscriber::Change(callback) => callback(&change_of(path, value)),
        }
    }
}

impl SettingsStore {
    pub fn load(&self) {
        let mut state = lock(&self.state);
        if !self.path.exists() {
            return;
        }
        // utf-8-sig: Notepad saves UTF-8 with a byte order mark, which json rejects.
        let loaded: Option<Value> = match read_text(&self.path, true) {
            Ok(text) => json_loads(&text),
            Err(ReadError::Os(_) | ReadError::Decode) => None,
        };
        let loaded_mapping = loaded.as_ref().and_then(Value::as_object);
        let Some(loaded_mapping) = loaded_mapping else {
            // Keep safe defaults, and keep the user's file: the next change of any setting would otherwise save the
            // defaults over it.
            state.data.clone_from(&self.defaults);
            state.unreadable = set_aside_unreadable(&self.path);
            return;
        };
        state.data = deep_merge(&self.defaults, loaded_mapping);
        Self::adopt_new_defaults(&mut state.data, loaded_mapping, &self.defaults);
    }

    /// Let a file written by an older schema take the defaults that changed since.
    ///
    /// The persisted file is seeded from the shipped defaults, so a value equal to the old default is most likely the
    /// seed and not a choice. Schema 7 turned the early layout switch on; a schema-6 file with it off adopts that
    /// once. Schema 8 shortened the pause correction from 1.5 s; an older file with the old pause adopts the new one
    /// once. Schema 9, on macOS only, made Control+Option+K the conversion hotkey: a file holding Pause, which a Mac
    /// keyboard has not, takes it once (elsewhere Pause is the default and stays). The schema version written back
    /// marks the adoption done, so a value set again afterwards holds.
    fn adopt_new_defaults(data: &mut SettingsData, loaded: &SettingsData, defaults: &SettingsData) {
        let Some(version) = loaded.get("schema_version") else { return };
        // isinstance(version, int) and not a bool; a size beyond 64 bits reads as a float here and gives the same answer.
        let Some(version) = version.as_i64().filter(|_| is_py_int(version)) else { return };
        let schema = defaults["schema_version"].as_i64().expect("the defaults name their schema");
        if version >= schema {
            return;
        }
        let detection = loaded.get("detection").and_then(Value::as_object);
        if version < PAUSE_DELAY_DEFAULT_SCHEMA_VERSION
            && let (Some(detection), Some(Value::Object(current))) = (detection, data.get_mut("detection"))
        {
            if version < EARLY_SWITCH_DEFAULT_SCHEMA_VERSION && detection.get("early_switch") == Some(&Value::Bool(false)) {
                current.insert("early_switch".into(), Value::Bool(true));
            }
            let previous = Number::from_f64(PREVIOUS_DEFAULT_PAUSE_DELAY_SECONDS).map(Value::Number);
            if let (Some(stored), Some(previous)) = (detection.get("pause_delay_seconds"), previous)
                && py_eq(stored, &previous)
            {
                let default = Number::from_f64(DEFAULT_PAUSE_DELAY_SECONDS).map(Value::Number);
                current.insert("pause_delay_seconds".into(), default.unwrap_or(Value::Null));
            }
        }
        let seeded = loaded.get("hotkeys").and_then(|hotkeys| hotkeys.get(HOTKEY_CONVERT_LAST)).and_then(Value::as_str) == Some(DEFAULT_CONVERT_LAST_HOTKEY);
        if seeded {
            let hotkeys = data.get_mut("hotkeys").and_then(Value::as_object_mut).expect("a table of hotkeys merged over the defaults' table");
            hotkeys.insert(HOTKEY_CONVERT_LAST.into(), defaults["hotkeys"][HOTKEY_CONVERT_LAST].clone());
        }
        data.insert("schema_version".into(), defaults["schema_version"].clone());
    }

    pub(crate) fn save_locked(&self, data: &SettingsData) -> io::Result<()> {
        create_parent(&self.path)?;
        let temporary = with_suffix(&self.path, ".json.tmp");
        let options = JsonOptions {
            ensure_ascii: false,
            indent: Some(Indent::Spaces(USER_DATA_JSON_INDENT)),
            ..JsonOptions::default()
        };
        let mut text = json_dumps(&Value::Object(data.clone()), &options);
        text.push('\n');
        write_text(&temporary, &text)?;
        std::fs::rename(&temporary, &self.path)
    }

    /// A setting as its key's type reads it, or `None` where the path leads nowhere or the file holds what the type cannot
    /// read (a known key can hold anything: such a value stays in the file as it is, and only the reader falls back).
    pub fn read<T>(&self, key: Key<T>) -> Option<T> {
        self.with_value(key.path, |value| value.and_then(key.read))
    }

    /// Set a setting to a value of its type; see [`SettingsStore::set`].
    pub fn write<T>(&self, key: Key<T>, value: T, persist: bool) -> io::Result<()> {
        self.set(key.path, (key.write)(value), persist)
    }

    /// `get(dotted_path)`: a copy of the value, or `None` where the path leads nowhere (a stored `null` is `Some(Null)`).
    pub fn get(&self, dotted_path: &str) -> Option<Value> {
        #[expect(clippy::redundant_closure_for_method_calls, reason = "the method path is ambiguous between the two Option impls of cloned")]
        self.with_value(dotted_path, |value| value.cloned())
    }

    /// `get(dotted_path, default)`.
    pub fn get_or(&self, dotted_path: &str, default: Value) -> Value {
        self.get(dotted_path).unwrap_or(default)
    }

    /// The value at the path, looked at without a copy (the engine reads settings on every key).
    pub fn with_value<R>(&self, dotted_path: &str, look: impl FnOnce(Option<&Value>) -> R) -> R {
        let state = lock(&self.state);
        let mut current: Option<&Value> = None;
        let mut mapping: Option<&SettingsData> = Some(&state.data);
        for part in dotted_path.split('.') {
            // The stored settings are JSON objects (string keys) merged onto the defaults: a lookup needs no copy of
            // each mapping on the way.
            let Some(found) = mapping.and_then(|map| map.get(part)) else { return look(None) };
            current = Some(found);
            mapping = found.as_object();
        }
        look(current)
    }

    /// `set(dotted_path, value, persist=...)`. Intermediate levels that are not mappings are replaced by empty ones. An
    /// unchanged value (Python `==`: `1` equals `True`) changes nothing and calls nobody. A failure to save is returned
    /// after the value has been changed in memory, and the callbacks are not called.
    pub fn set(&self, dotted_path: &str, value: Value, persist: bool) -> io::Result<()> {
        let callbacks;
        {
            let mut state = lock(&self.state);
            let parts: Vec<&str> = dotted_path.split('.').collect();
            let (last, inner) = parts.split_last().expect("split gives a part");
            let mut target: &mut SettingsData = &mut state.data;
            for part in inner {
                let is_mapping = target.get(*part).is_some_and(Value::is_object);
                if !is_mapping {
                    target.insert((*part).to_string(), Value::Object(Map::new()));
                }
                target = target.get_mut(*part).and_then(Value::as_object_mut).expect("a mapping was just put there");
            }
            // Python's `target.get(last) == value`: a missing key reads as None.
            let current = target.get(*last).unwrap_or(&Value::Null);
            if py_eq(current, &value) {
                return Ok(());
            }
            target.insert((*last).to_string(), value.clone());
            if persist {
                let data = state.data.clone();
                self.save_locked(&data)?;
            }
            callbacks = self.callbacks_snapshot();
        }
        for callback in callbacks {
            callback.notify(dotted_path, &value);
        }
        Ok(())
    }

    /// The shipped value of a path without touching stored data; `None` for a path the defaults do not have.
    pub fn default(&self, dotted_path: &str) -> Option<Value> {
        let mut value: Option<&Value> = None;
        let mut mapping: Option<&SettingsData> = Some(&self.defaults);
        for part in dotted_path.split('.') {
            let found = mapping.and_then(|map| map.get(part))?;
            value = Some(found);
            mapping = found.as_object();
        }
        value.cloned()
    }

    pub fn is_default(&self, dotted_path: &str) -> bool {
        match (self.get(dotted_path), self.default(dotted_path)) {
            (None, None) => true,
            (Some(a), Some(b)) => py_eq(&a, &b),
            _ => false,
        }
    }

    /// Reset a single path; unknown paths are left untouched.
    pub fn restore_default(&self, dotted_path: &str) -> io::Result<bool> {
        match self.default(dotted_path) {
            None => Ok(false),
            Some(value) => {
                self.set(dotted_path, value, true)?;
                Ok(true)
            }
        }
    }

    /// The shipped settings of this platform (`default_settings`), as a copy.
    pub fn defaults(&self) -> SettingsData {
        self.defaults.clone()
    }

    pub fn snapshot(&self) -> SettingsData {
        lock(&self.state).data.clone()
    }

    pub fn reset(&self) -> io::Result<()> {
        let callbacks;
        {
            let mut state = lock(&self.state);
            state.data.clone_from(&self.defaults);
            let data = state.data.clone();
            self.save_locked(&data)?;
            callbacks = self.callbacks_snapshot();
        }
        for callback in callbacks {
            callback.notify("*", &Value::Object(self.snapshot()));
        }
        Ok(())
    }
}

/// The name of a table of defaults in the technical journal: the SHA-256 of its JSON with sorted keys.
#[must_use]
pub fn defaults_sha256(defaults: &SettingsData) -> String {
    let options = JsonOptions {
        ensure_ascii: false,
        sort_keys: true,
        separators: Some((",".to_string(), ":".to_string())),
        ..JsonOptions::default()
    };
    let text = json_dumps(&Value::Object(defaults.clone()), &options);
    Sha256::digest(text.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// `settings` without the dotted paths of `SETTINGS_SINCE_PYTHON`: the table as the Python program shipped it.
#[must_use]
pub fn without_settings_since_python(settings: &SettingsData) -> SettingsData {
    let mut kept = settings.clone();
    for path in SETTINGS_SINCE_PYTHON {
        let (parents, name) = path.rsplit_once('.').unwrap_or(("", path));
        let mut table = Some(&mut kept);
        for part in parents.split('.').filter(|part| !part.is_empty()) {
            table = table.and_then(|map| map.get_mut(part)).and_then(Value::as_object_mut);
        }
        if let Some(map) = table {
            // `remove` of a map that keeps its order swaps the last key into the place of the one removed; the
            // table must keep the Python's order.
            map.shift_remove(name);
        }
    }
    kept
}

const LOGGABLE_STRING_TRUNCATED_CHARACTERS: usize = LOGGED_SETTING_VALUE_MAX_CHARACTERS - "...".len();

fn loggable_value(value: &Value) -> LoggedValue {
    match value {
        Value::Null => LoggedValue::Nothing,
        Value::Bool(flag) => LoggedValue::Flag(*flag),
        Value::Number(number) => LoggedValue::Number(number.clone()),
        Value::String(text) => LoggedValue::Text(if pystr::len(text) <= LOGGED_SETTING_VALUE_MAX_CHARACTERS {
            text.clone()
        } else {
            format!("{}...", pystr::slice(text, None, Some(LOGGABLE_STRING_TRUNCATED_CHARACTERS.to::<i64>())))
        }),
        Value::Array(items) => LoggedValue::Collection { r#type: "list", items: items.len() },
        Value::Object(items) => LoggedValue::Collection { r#type: "dict", items: items.len() },
    }
}

fn overrides_into(defaults: &SettingsData, current: &SettingsData, prefix: &str, out: &mut IndexMap<String, LoggedValue>) {
    // Walk known defaults, never arbitrary names from user configuration.
    for (name, default) in defaults {
        let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}.{name}") };
        if path == "schema_version" {
            continue;
        }
        let value = current.get(name);
        if let (Value::Object(default_map), Some(Value::Object(value_map))) = (default, value) {
            overrides_into(default_map, value_map, &path, out);
        } else {
            match value {
                None => {
                    out.insert(path, LoggedValue::Missing { missing: true });
                }
                Some(value) if !py_eq(value, default) => {
                    out.insert(path, loggable_value(value));
                }
                Some(_) => {}
            }
        }
    }
}

/// What the file holds differently from the defaults, by dotted path: only known settings are named.
pub(crate) fn overrides(defaults: &SettingsData, current: &SettingsData) -> IndexMap<String, LoggedValue> {
    let mut out = IndexMap::new();
    overrides_into(defaults, current, "", &mut out);
    out
}

/// The schema of a table of defaults.
pub(crate) fn schema_of(defaults: &SettingsData) -> i64 {
    defaults["schema_version"].as_i64().expect("the defaults name their schema")
}

/// Represent reset without repeating its default; never disclose unknown keys.
pub fn setting_change(settings: &SettingsStore, path: &str, value: &Value) -> Option<SettingChangeRecord> {
    let default = settings.default(path);
    if path == "*" || matches!(default, Some(Value::Object(_))) {
        return Some(SettingChangeRecord {
            path: path.to_owned(),
            operation: Operation::Snapshot,
            settings: Some(crate::settings_diagnostics::settings_snapshot(settings)),
            value: None,
        });
    }
    let default = match default {
        Some(default) if path != "schema_version" => default,
        _ => return None,
    };
    Some(if py_eq(value, &default) {
        SettingChangeRecord { path: path.to_owned(), operation: Operation::Reset, settings: None, value: None }
    } else {
        SettingChangeRecord { path: path.to_owned(), operation: Operation::Set, settings: None, value: Some(loggable_value(value)) }
    })
}

impl SettingChangeRecord {
    /// The record as the tree a technical line is built from, while the engine still writes its lines as trees.
    #[must_use]
    pub fn into_tree(self) -> SettingsData {
        serde_json::to_value(self).expect("a record is JSON").as_object().cloned().expect("a record is an object")
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::config::{ChangeCallback, Unsubscribe};
    use crate::constants::{
        DETECTION_CONFIDENCE, DETECTION_EARLY_SWITCH, DETECTION_LAYOUT_SWITCH_KEY, DETECTION_MINIMUM_LENGTH, ENABLED,
        EXCLUSIONS_WORDS, HISTORY_LIMIT, HOTKEYS_CONVERT_LAST,
    };
    use crate::settings_diagnostics::{SettingsSnapshot, settings_snapshot};
    use ks_text::json::dumps;
    use serde_json::json;
    use std::sync::Mutex;

    #[test]
    fn the_edges_of_numbers_and_of_truth() {
        assert!(!py_eq(&json!(1), &json!(1e39)));
        assert!(!truthy(&Value::Null) && !truthy(&json!(0)) && truthy(&json!(5)));
        assert!(is_py_int(&json!(u64::MAX)));
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
        assert!(!py_eq(&json!(u64::MAX), &json!(1.844_674_407_370_955_2e19)));
    }

    #[test]
    fn truthiness_and_ints() {
        assert!(!truthy(&json!(0.0)) && !truthy(&json!("")) && !truthy(&json!([])) && truthy(&json!({"a": 1})));
        assert!(is_py_int(&json!(3)) && !is_py_int(&json!(3.0)) && !is_py_int(&json!(true)));
        assert_eq!(as_py_int(&json!(-4)), Some(-4));
        assert_eq!((as_py_int(&json!(true)), as_py_int(&json!("4")), as_py_int(&json!(4.5)), as_py_int(&Value::Null)), (None, None, None, None));
    }

    #[test]
    fn a_limit_above_the_signed_range_is_still_a_number() {
        assert!(history_limit(&json!(u64::MAX)) > 0);
    }

    #[test]
    fn limits() {
        assert_eq!(history_limit(&json!("abc")), DEFAULT_HISTORY_LIMIT);
        assert_eq!(history_limit(&Value::Null), DEFAULT_HISTORY_LIMIT);
        assert_eq!(history_limit(&json!(true)), DEFAULT_HISTORY_LIMIT);
        assert_eq!(history_limit(&json!([200])), DEFAULT_HISTORY_LIMIT);
        assert_eq!(history_limit(&json!("200")), 200);
        assert_eq!(history_limit(&json!(" 12 ")), 12);
        assert_eq!(history_limit(&json!(0)), 1);
        assert_eq!(history_limit(&json!(7.9)), 7);
        assert_eq!(history_limit(&json!(-3.5)), 1);
    }

    #[test]
    fn mapping() {
        assert_eq!(normalize_indicator_style(&json!("flags")), "flags");
        assert_eq!(normalize_indicator_style(&json!("x")), "letters");
        assert_eq!(normalize_indicator_style(&Value::Null), "letters");
        assert_eq!(layout_icon_name(&json!("flags"), 1), "keyswitch-flag-ru");
        assert_eq!(layout_icon_name(&json!("letters"), 7), "keyswitch");
    }

    #[test]
    fn a_file_that_cannot_be_moved_stays_and_the_macos_hotkeys_are_laid_over() {
        assert_eq!(set_aside_unreadable(std::path::Path::new("/nonexistent-folder/settings.json")), None);
        assert_ne!(default_settings(Some("darwin"))["hotkeys"], default_settings(Some("linux"))["hotkeys"]);
        let store = SettingsStore::new(Some(temporary("unknown").join("config.json")));
        store.set("unknown.key", json!(1), false).unwrap();
        assert!(!store.is_default("unknown.key"));
        assert!(store.is_default("missing.key"));
    }

    fn temporary(name: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!("ks-config-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn merge_replaces_scalars_and_merges_mappings() {
        let base = json!({"a": 1, "b": {"c": 2, "d": [1]}, "e": {"x": 1}});
        let over = json!({"b": {"c": 3, "z": 0}, "e": 5, "n": null, "a": {"k": 1}});
        let merged = deep_merge(base.as_object().unwrap(), over.as_object().unwrap());
        assert_eq!(
            Value::Object(merged),
            json!({"a": {"k": 1}, "b": {"c": 3, "d": [1], "z": 0}, "e": 5, "n": null})
        );
    }

    #[test]
    fn malformed_files_keep_defaults_and_stay_aside() {
        let directory = temporary("malformed");
        let path = directory.join("config.json");
        for payload in ["{broken", "[]"] {
            std::fs::write(&path, payload).unwrap();
            let store = SettingsStore::new(Some(path.clone()));
            assert_eq!(store.get("schema_version"), Some(default_settings(None)["schema_version"].clone()));
            let aside = store.unreadable().expect("moved aside");
            assert_eq!(std::fs::read_to_string(aside).unwrap(), payload);
            assert!(!path.exists());
        }
    }

    #[test]
    fn a_byte_order_mark_is_read_and_unknown_keys_survive() {
        let directory = temporary("bom");
        let path = directory.join("config.json");
        std::fs::write(&path, b"\xEF\xBB\xBF{\"exclusions\": {\"applications\": [\"vault\"]}, \"mine\": [1]}").unwrap();
        let store = SettingsStore::new(Some(path.clone()));
        assert_eq!(store.get("exclusions.applications"), Some(json!(["vault"])));
        store.set("enabled", json!(false), true).unwrap();
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["mine"], json!([1]));
        assert!(std::fs::read_to_string(&path).unwrap().replace("\r\n", "\n").ends_with("}\n"));
    }

    #[test]
    fn old_schema_adopts_new_defaults_once() {
        let directory = temporary("schema");
        let path = directory.join("config.json");
        let old = r#"{"schema_version": 6, "detection": {"early_switch": false, "pause_delay_seconds": 1.5}}"#;
        std::fs::write(&path, old).unwrap();
        let store = SettingsStore::new(Some(path.clone()));
        assert_eq!(store.get("detection.early_switch"), Some(json!(true)));
        assert_eq!(store.get("detection.pause_delay_seconds"), Some(json!(0.7)));
        // Schema 8, or 9 on macOS (MACOS_CONVERT_HOTKEY_SCHEMA_VERSION).
        assert_eq!(store.get("schema_version"), Some(default_settings(None)["schema_version"].clone()));
        let new = r#"{"schema_version": 8, "detection": {"early_switch": false, "pause_delay_seconds": 1.5}}"#;
        std::fs::write(&path, new).unwrap();
        let store = SettingsStore::new(Some(path));
        assert_eq!(store.get("detection.early_switch"), Some(json!(false)));
        assert_eq!(store.get("detection.pause_delay_seconds"), Some(json!(1.5)));
    }

    #[test]
    fn get_set_callbacks_snapshot_and_reset() {
        let directory = temporary("callbacks");
        let path = directory.join("config.json");
        let store = SettingsStore::new(Some(path.clone()));
        let calls: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
        let recorder = calls.clone();
        let unsubscribe = store.subscribe(Arc::new(move |key: &str, value: &Value| {
            recorder.lock().unwrap().push((key.to_string(), value.clone()));
        }));
        store.set("temporary", json!(1), false).unwrap();
        assert!(!path.exists());
        store.set("temporary.child", json!(["a"]), true).unwrap();
        assert_eq!(store.get("temporary.child"), Some(json!(["a"])));
        assert_eq!(store.get_or("missing.path", json!("fallback")), json!("fallback"));
        let before = calls.lock().unwrap().len();
        store.set("temporary.child", json!(["a"]), true).unwrap();
        // 1 == True: the stored 1 is not replaced by true.
        store.set("temporary2", json!(1), false).unwrap();
        store.set("temporary2", json!(true), false).unwrap();
        assert_eq!(store.get("temporary2"), Some(json!(1)));
        assert_eq!(calls.lock().unwrap().len(), before + 1);
        unsubscribe();
        unsubscribe();
        store.set("enabled", json!(false), true).unwrap();
        assert_eq!(calls.lock().unwrap().len(), before + 1);
        let reset_calls: Arc<Mutex<Vec<String>>> = Arc::default();
        let recorder = reset_calls.clone();
        let _ = store.subscribe(Arc::new(move |key: &str, _: &Value| recorder.lock().unwrap().push(key.to_string())));
        store.reset().unwrap();
        assert_eq!(reset_calls.lock().unwrap().as_slice(), ["*"]);
        assert_eq!(Value::Object(store.snapshot()), Value::Object(default_settings(None)));
        let on_disk: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(on_disk, Value::Object(default_settings(None)));
        assert!(store.is_default("enabled"));
        store.set("enabled", json!(false), false).unwrap();
        assert!(!store.is_default("enabled"));
        assert!(store.restore_default("enabled").unwrap());
        assert!(!store.restore_default("nonexistent.path").unwrap());
        assert_eq!(store.default("hotkeys.undo"), Some(json!("Ctrl+Alt+Z")));
    }

    #[test]
    fn macos_hotkeys() {
        let mac = default_settings(Some("darwin"));
        let linux = default_settings(Some("linux"));
        assert_eq!(mac["hotkeys"]["selection_layout"], json!("Ctrl+Alt+L"));
        assert_eq!(linux["hotkeys"]["selection_layout"], json!("Shift+Pause"));
        assert_eq!((mac["hotkeys"]["convert_last"].clone(), mac["schema_version"].clone()), (json!("Ctrl+Alt+K"), json!(9)));
        assert_eq!((linux["hotkeys"]["convert_last"].clone(), linux["schema_version"].clone()), (json!("Pause"), json!(8)));
    }

    /// What `load` makes of `file` with the defaults of `platform`.
    fn loaded_on(platform: &str, file: &Value) -> SettingsData {
        let defaults = default_settings(Some(platform));
        let loaded = file.as_object().cloned().unwrap();
        let mut data = deep_merge(&defaults, &loaded);
        SettingsStore::adopt_new_defaults(&mut data, &loaded, &defaults);
        data
    }

    #[test]
    fn the_macos_conversion_hotkey_takes_the_place_of_pause_once() {
        let seeded = json!({"schema_version": 8, "hotkeys": {"convert_last": "Pause"}, "detection": {"pause_delay_seconds": 1.5}});
        let mac = loaded_on("darwin", &seeded);
        assert_eq!((mac["hotkeys"]["convert_last"].clone(), mac["schema_version"].clone()), (json!("Ctrl+Alt+K"), json!(9)));
        // A schema-8 file took the shorter pause already: its value is the user's.
        assert_eq!(mac["detection"]["pause_delay_seconds"], json!(1.5));
        // Elsewhere Pause is the default, and the schema stays.
        let linux = loaded_on("linux", &seeded);
        assert_eq!((linux["hotkeys"]["convert_last"].clone(), linux["schema_version"].clone()), (json!("Pause"), json!(8)));
        // A key the user chose stays, and so does Pause chosen again after the step.
        let chosen = loaded_on("darwin", &json!({"schema_version": 8, "hotkeys": {"convert_last": "Ctrl+Alt+J"}}));
        assert_eq!((chosen["hotkeys"]["convert_last"].clone(), chosen["schema_version"].clone()), (json!("Ctrl+Alt+J"), json!(9)));
        let again = loaded_on("darwin", &json!({"schema_version": 9, "hotkeys": {"convert_last": "Pause"}}));
        assert_eq!(again["hotkeys"]["convert_last"], json!("Pause"));
        // An older Mac file takes both steps; one with no hotkeys, or another value there, keeps the rest.
        let old = loaded_on("darwin", &json!({"schema_version": 6, "hotkeys": {"convert_last": "Pause"}, "detection": {"early_switch": false}}));
        assert_eq!((old["hotkeys"]["convert_last"].clone(), old["detection"]["early_switch"].clone()), (json!("Ctrl+Alt+K"), json!(true)));
        let bare = loaded_on("darwin", &json!({"schema_version": 8}));
        assert_eq!((bare["hotkeys"]["convert_last"].clone(), bare["schema_version"].clone()), (json!("Ctrl+Alt+K"), json!(9)));
        let odd = loaded_on("darwin", &json!({"schema_version": 8, "hotkeys": {"convert_last": 5}}));
        assert_eq!(odd["hotkeys"]["convert_last"], json!(5));
    }

    #[test]
    fn python_equality_of_values() {
        let answers = [
            (json!(null), json!(null), true),
            (json!(null), json!(0), false),
            (json!("a"), json!("a"), true),
            (json!([1]), json!([1, 2]), false),
            (json!([1, 2]), json!([1.0, 2]), true),
            (json!({"a": 1}), json!({"a": 1.0}), true),
            (json!({"a": 1}), json!({"b": 1}), false),
            (json!({"a": 1}), json!({"a": 1, "b": 2}), false),
            (json!(1), json!(1.5), false),
            (json!(1), json!(1.0), true),
            (json!(true), json!(1), true),
            (json!(1.0), json!(1.0), true),
            (json!(1e300), json!(1), false),
        ];
        for (left, right, answer) in answers {
            assert_eq!(py_eq(&left, &right), answer, "{left} {right}");
        }
    }

    #[test]
    fn every_layout_has_its_icon() {
        for (style, group, name) in [
            (json!("letters"), 0, "keyswitch-en"),
            (json!("letters"), 1, "keyswitch-ru"),
            (json!("letters"), 2, "keyswitch"),
            (json!("flags"), 0, "keyswitch-flag-us"),
            (json!("flags"), 1, "keyswitch-flag-ru"),
            (json!("flags"), 2, "keyswitch"),
        ] {
            assert_eq!(layout_icon_name(&style, group), name, "{style} {group}");
        }
    }

    #[test]
    fn a_value_of_another_type_is_logged_as_it_is() {
        let settings = store("odd");
        let changed = |value: Value| setting_change(&settings, "detection.early_switch", &value).unwrap().value.unwrap();
        assert_eq!(changed(json!("short")), LoggedValue::Text("short".to_owned()));
        assert_eq!(changed(json!([1, 2])), LoggedValue::Collection { r#type: "list", items: 2 });
        assert_eq!(changed(json!({"a": 1})), LoggedValue::Collection { r#type: "dict", items: 1 });
        assert_eq!(setting_change(&settings, "no.such.path", &Value::Null), None);
    }

    // ---- the table of settings

    fn leaves_of(table: &SettingsData, prefix: &str, out: &mut Vec<(String, Value)>) {
        for (name, value) in table {
            let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}.{name}") };
            match value {
                Value::Object(inner) => leaves_of(inner, &path, out),
                other @ (Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_)) => out.push((path, other.clone())),
            }
        }
    }

    #[test]
    fn the_table_of_settings_is_the_table_of_defaults() {
        let defaults = default_settings(Some("linux"));
        let mut leaves = Vec::new();
        leaves_of(&defaults, "", &mut leaves);
        assert_eq!(leaves.iter().map(|(path, _)| path.as_str()).collect::<Vec<_>>(), SETTING_ROWS.iter().map(|row| row.path).collect::<Vec<_>>());
        for ((path, default), row) in leaves.iter().zip(SETTING_ROWS) {
            assert!((row.fits_default)(default), "{path}: the default {default} is not of the key's type");
            let change = (row.parse)(default).expect("a default reads as its key's type");
            assert_eq!(change.path(), *path);
            assert!(py_eq(&change.stored(), default), "{path}: {:?} stands for {default}", change.stored());
        }
    }

    #[test]
    fn a_stored_value_reads_the_way_its_kind_of_setting_has_always_been_read() {
        let flag = <bool as SettingValue>::from_tree;
        assert_eq!((flag(&json!(0)), flag(&json!("")), flag(&json!([])), flag(&json!(1)), flag(&json!("x")), flag(&Value::Null)), (Some(false), Some(false), Some(false), Some(true), Some(true), Some(false)));
        let text = <String as SettingValue>::from_tree;
        assert_eq!((text(&json!("a")), text(&json!(5)), text(&json!(true)), text(&Value::Null)), (Some("a".to_owned()), Some("5".to_owned()), Some("true".to_owned()), Some("null".to_owned())));
        let float = <f64 as SettingValue>::from_tree;
        assert_eq!((float(&json!(1.5)), float(&json!(2)), float(&json!(true)), float(&json!(" 2.5 ")), float(&json!("x")), float(&Value::Null), float(&json!([1]))), (Some(1.5), Some(2.0), Some(1.0), Some(2.5), None, None, None));
        let whole = <i64 as SettingValue>::from_tree;
        assert_eq!((whole(&json!(3)), whole(&json!(3.9)), whole(&json!(-1.5)), whole(&json!("7")), whole(&json!("nan")), whole(&json!("inf")), whole(&json!({}))), (Some(3), Some(3), Some(-1), Some(7), Some(0), Some(i64::MAX), None));
        let words = <Vec<String> as SettingValue>::from_tree;
        assert_eq!((words(&json!(["a", 1, "b", null])), words(&json!("a")), words(&json!([]))), (Some(vec!["a".to_owned(), "b".to_owned()]), None, Some(Vec::new())));
        let limit = <HistoryLimit as SettingValue>::from_tree;
        assert_eq!((limit(&json!(5)), limit(&json!("x"))), (Some(HistoryLimit(5)), Some(HistoryLimit(DEFAULT_HISTORY_LIMIT))));
        // Each writes the kind of value the settings windows write, and the defaults are of those kinds.
        assert_eq!((true.into_tree(), "a".to_owned().into_tree(), 0.5.into_tree(), 3_i64.into_tree(), vec!["a".to_owned()].into_tree(), HistoryLimit(2).into_tree()), (json!(true), json!("a"), json!(0.5), json!(3), json!(["a"]), json!(2)));
        let fits = [<bool as SettingValue>::fits_default, <String as SettingValue>::fits_default, <f64 as SettingValue>::fits_default, <i64 as SettingValue>::fits_default, <Vec<String> as SettingValue>::fits_default, <HistoryLimit as SettingValue>::fits_default];
        let right = [json!(true), json!("a"), json!(1.5), json!(1), json!(["a"]), json!(5)];
        let wrong = [json!(1), json!(1), json!("1"), json!(1.5), json!([1]), json!("5")];
        for ((fits, right), wrong) in fits.iter().zip(&right).zip(&wrong) {
            assert!(fits(right) && !fits(wrong));
        }
    }

    #[test]
    fn a_key_names_its_path_and_a_copy_is_the_key() {
        fn copy_of<T: Clone>(value: &T) -> T {
            value.clone()
        }
        assert_eq!(ENABLED.path(), "enabled");
        assert_eq!(copy_of(&ENABLED).path(), "enabled");
        let made = Key::new("a.b", <bool as SettingValue>::from_tree, <bool as SettingValue>::into_tree);
        assert_eq!(made.path(), "a.b");
    }

    #[test]
    fn a_setting_is_read_and_written_through_its_key() {
        let directory = temporary("keys");
        let path = directory.join("config.json");
        std::fs::write(&path, r#"{"detection": {"minimum_length": "seven", "confidence": [1]}, "hotkeys": 5}"#).unwrap();
        let store = SettingsStore::new(Some(path.clone()));
        assert_eq!(store.read(ENABLED), Some(true));
        // A value of another type stays in the file; a key that cannot read it answers None, and the caller's fallback holds.
        assert_eq!(store.read(DETECTION_MINIMUM_LENGTH), None);
        assert_eq!(store.read(DETECTION_CONFIDENCE), None);
        assert_eq!(store.get("detection.minimum_length"), Some(json!("seven")));
        // The table of hotkeys was replaced by a number in the file: the path leads nowhere.
        assert_eq!(store.read(HOTKEYS_CONVERT_LAST), None);
        store.write(ENABLED, false, true).unwrap();
        store.write(EXCLUSIONS_WORDS, vec!["слово".to_owned()], true).unwrap();
        store.write(HISTORY_LIMIT, HistoryLimit(7), true).unwrap();
        store.write(DETECTION_LAYOUT_SWITCH_KEY, "Shift_L".to_owned(), true).unwrap();
        assert_eq!(store.read(ENABLED), Some(false));
        assert_eq!(store.read(EXCLUSIONS_WORDS), Some(vec!["слово".to_owned()]));
        assert_eq!(store.read(HISTORY_LIMIT), Some(HistoryLimit(7)));
        assert_eq!(store.read(DETECTION_LAYOUT_SWITCH_KEY), Some("Shift_L".to_owned()));
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!((saved["enabled"].clone(), saved["detection"]["minimum_length"].clone(), saved["hotkeys"].clone(), saved["history"]["limit"].clone()), (json!(false), json!("seven"), json!(5), json!(7)));
        assert_eq!(saved["exclusions"]["words"], json!(["слово"]));
    }

    #[test]
    fn a_writer_and_the_two_kinds_of_subscriber_see_the_same_changes() {
        let store = SettingsStore::new(Some(temporary("subscribers").join("config.json")));
        let changes: Arc<Mutex<Vec<SettingChange>>> = Arc::default();
        let recorder = changes.clone();
        let callback: ChangeCallback = Arc::new(move |change: &SettingChange| recorder.lock().unwrap().push(change.clone()));
        let unsubscribe: Unsubscribe = store.subscribe_changes(callback);
        let trees: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
        let recorder = trees.clone();
        let _ = store.subscribe(Arc::new(move |path: &str, value: &Value| recorder.lock().unwrap().push((path.to_string(), value.clone()))));
        store.write(DETECTION_EARLY_SWITCH, false, false).unwrap();
        store.set("detection.early_switch", json!(true), false).unwrap();
        // Not a setting of the table, or a value its key cannot read: the typed subscriber hears of it as it was.
        store.set("mine.own", json!(1), false).unwrap();
        store.set("detection.confidence", json!([1]), false).unwrap();
        store.reset().unwrap();
        let heard_changes = changes.lock().unwrap().clone();
        assert_eq!(heard_changes.len(), 5);
        assert_eq!(heard_changes[0], SettingChange::DetectionEarlySwitch(false));
        assert_eq!(heard_changes[1], SettingChange::DetectionEarlySwitch(true));
        assert_eq!((heard_changes[2].path(), heard_changes[2].stored()), ("mine.own", json!(1)));
        assert_eq!((heard_changes[3].path(), heard_changes[3].stored()), ("detection.confidence", json!([1])));
        assert_eq!(heard_changes[4], SettingChange::Reset);
        assert_eq!(heard_changes[4].path(), "*");
        assert_eq!(heard_changes[0].stored(), json!(false));
        assert_eq!(SettingChange::Reset.stored(), Value::Null);
        let heard = trees.lock().unwrap();
        assert_eq!(heard.iter().map(|(path, _)| path.as_str()).collect::<Vec<_>>(), ["detection.early_switch", "detection.early_switch", "mine.own", "detection.confidence", "*"]);
        drop(heard);
        unsubscribe();
        store.write(ENABLED, false, false).unwrap();
        assert_eq!(changes.lock().unwrap().len(), 5);
    }

    // ---- what the technical log says of the settings

    fn store(name: &str) -> SettingsStore {
        SettingsStore::new(Some(temporary(&format!("diag-{name}")).join("config.json")))
    }

    fn dump(snapshot: &SettingsSnapshot) -> String {
        dumps(snapshot, &JsonOptions { ensure_ascii: false, sort_keys: true, ..JsonOptions::default() })
    }

    #[test]
    fn the_python_table_is_the_defaults_without_the_settings_it_never_had() {
        let defaults = default_settings(Some("linux"));
        let python = without_settings_since_python(&defaults);
        assert!(defaults["detection"].get("layout_switch_key").is_some());
        assert_eq!(defaults["detection"].get("convert_last_key"), Some(&json!("")));
        assert!(defaults["general"].get("hints").is_some());
        assert!(python["detection"].get("layout_switch_key").is_none());
        assert!(python["detection"].get("convert_last_key").is_none());
        assert!(python["general"].get("hints").is_none());
        // The hash the traces of the Python's engine name its defaults by.
        assert_eq!(defaults_sha256(&python), "bb6151bb4466c9d4f0e156483a52bf40135d7bdd01b62ae115553caf66ecfe9f");
        assert_ne!(defaults_sha256(&defaults), defaults_sha256(&python));
        // A table without the parent of such a setting loses nothing.
        let bare = json!({"enabled": true}).as_object().cloned().unwrap();
        assert_eq!(without_settings_since_python(&bare), bare);
    }

    #[test]
    fn a_known_name_that_is_gone_is_reported_missing() {
        let defaults = json!({"a": 1}).as_object().cloned().unwrap();
        let current = json!({}).as_object().cloned().unwrap();
        assert_eq!(overrides(&defaults, &current)["a"], LoggedValue::Missing { missing: true });
    }

    #[test]
    fn defaults_have_no_overrides() {
        let settings = store("defaults");
        let snapshot = settings_snapshot(&settings);
        assert_eq!(snapshot.format, "overrides-v1");
        assert_eq!(Value::from(snapshot.defaults_schema), default_settings(None)["schema_version"]);
        assert!(snapshot.overrides.is_empty());
        assert_eq!(snapshot.defaults_sha256.len(), 64);
    }

    #[test]
    fn collections_are_summarised_and_strings_bounded() {
        let settings = store("collections");
        settings.set("exclusions.words", json!(["secret", "words"]), false).unwrap();
        settings.set("appearance.theme", json!("я".repeat(100)), false).unwrap();
        settings.set("unknown.thing", json!("hidden"), false).unwrap();
        settings.set("detection.confidence", json!(true), false).unwrap();
        let snapshot = settings_snapshot(&settings);
        let overrides = &snapshot.overrides;
        assert_eq!(overrides["exclusions.words"], LoggedValue::Collection { r#type: "list", items: 2 });
        assert_eq!(overrides["appearance.theme"], LoggedValue::Text(format!("{}...", "я".repeat(77))));
        assert_eq!(overrides["detection.confidence"], LoggedValue::Flag(true));
        assert!(!overrides.contains_key("unknown.thing"));
        let change = setting_change(&settings, "detection.confidence", &json!(2.0)).unwrap();
        assert_eq!(change.operation, Operation::Reset);
        assert!(setting_change(&settings, "unknown.thing", &json!(1)).is_none());
        assert!(setting_change(&settings, "schema_version", &json!(1)).is_none());
        assert_eq!(setting_change(&settings, "detection", &json!({})).unwrap().operation, Operation::Snapshot);
    }

    #[test]
    fn a_record_of_the_log_is_written_key_by_key_as_it_was() {
        let settings = store("records");
        settings.set("exclusions.words", json!(["a"]), false).unwrap();
        settings.set("detection.minimum_length", json!(null), false).unwrap();
        settings.set("appearance.theme", json!("dark"), false).unwrap();
        settings.set("detection.confidence", json!(1), false).unwrap();
        let snapshot = settings_snapshot(&settings);
        let text = dump(&snapshot);
        assert_eq!(
            text,
            format!(
                "{{\"defaults_schema\": {}, \"defaults_sha256\": \"{}\", \"format\": \"overrides-v1\", \"overrides\": {{\"appearance.theme\": \"dark\", \"detection.confidence\": 1, \"detection.minimum_length\": null, \"exclusions.words\": {{\"items\": 1, \"type\": \"list\"}}}}}}",
                snapshot.defaults_schema, snapshot.defaults_sha256
            )
        );
        // The order of the overrides is the order of the defaults (the log sorts them; the snapshot does not).
        assert_eq!(snapshot.overrides.keys().map(String::as_str).collect::<Vec<_>>(), ["detection.minimum_length", "detection.confidence", "exclusions.words", "appearance.theme"]);
        let change = setting_change(&settings, "appearance.theme", &json!("light")).unwrap();
        assert_eq!(dumps(&change, &JsonOptions::default()), r#"{"path": "appearance.theme", "operation": "set", "value": "light"}"#);
        assert_eq!(change.clone().into_tree(), json!({"path": "appearance.theme", "operation": "set", "value": "light"}).as_object().cloned().unwrap());
        let reset = setting_change(&settings, "detection.confidence", &json!(2.0)).unwrap();
        assert_eq!(dumps(&reset, &JsonOptions::default()), r#"{"path": "detection.confidence", "operation": "reset"}"#);
        let all = setting_change(&settings, "*", &Value::Null).unwrap();
        assert_eq!(dumps(&all, &JsonOptions { sort_keys: true, ..JsonOptions::default() }), format!("{{\"operation\": \"snapshot\", \"path\": \"*\", \"settings\": {text}}}"));
    }
}
