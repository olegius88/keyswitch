//! Persistent, observable application settings.

use crate::constants::{
    DEFAULT_PAUSE_DELAY_SECONDS, DEFAULT_SETTINGS, EARLY_SWITCH_DEFAULT_SCHEMA_VERSION, MACOS_SELECTION_HOTKEY_DEFAULTS,
    PAUSE_DELAY_DEFAULT_SCHEMA_VERSION, PREVIOUS_DEFAULT_PAUSE_DELAY_SECONDS, UNREADABLE_FILE_SUFFIX,
    UNREADABLE_FILE_TIME_FORMAT, USER_DATA_JSON_INDENT,
};
use crate::logsetup::{local_time, strftime, warning};
use crate::pycompat::{
    is_py_int, json_loads, py_eq, read_text, with_name_suffix, with_suffix, write_text, ReadError,
};
use ks_text::{json_dumps, Indent, JsonOptions};
use serde_json::{Map, Value};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

const LOGGER_NAME: &str = "keyswitch.config";

/// `dict[str, object]`: a JSON object, in the order of its file.
pub type SettingsData = Map<String, Value>;
/// `Callable[[str, object], None]`: called with the dotted path that changed (`"*"` for a reset) and its new value.
pub type SettingCallback = Arc<dyn Fn(&str, &Value) + Send + Sync>;
/// What [`SettingsStore::subscribe`] returns; calling it more than once is harmless.
pub type Unsubscribe = Box<dyn Fn() + Send + Sync>;

/// A JSON object with string keys; every JSON object has them, so this is the type test of the Python helper.
#[must_use]
pub fn string_keyed_mapping(value: &Value) -> Option<&SettingsData> {
    value.as_object()
}

/// `_deep_merge`: the defaults with every loaded key laid over them; mappings on both sides are merged, anything else
/// is replaced. Keys the defaults do not know are kept, after the known ones.
pub fn deep_merge(defaults: &SettingsData, loaded: &SettingsData) -> SettingsData {
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

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// `Path.home()`.
pub fn home_dir() -> PathBuf {
    let variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    env_nonempty(variable).map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

/// `config_dir()` with the environment given: `KEYSWITCH_CONFIG_DIR`, else `%APPDATA%\KeySwitch` on Windows and
/// `$XDG_CONFIG_HOME/keyswitch` (`~/.config/keyswitch`) elsewhere. macOS follows the second rule, as the Python does.
pub fn config_dir_from(windows: bool, home: &Path, env: &dyn Fn(&str) -> Option<String>) -> PathBuf {
    let get = |name: &str| env(name).filter(|value| !value.is_empty());
    if let Some(over) = get("KEYSWITCH_CONFIG_DIR") {
        return PathBuf::from(over);
    }
    if windows {
        let base = match get("APPDATA") {
            Some(roaming) => PathBuf::from(roaming),
            None => home.join("AppData").join("Roaming"),
        };
        return base.join("KeySwitch");
    }
    let base = match get("XDG_CONFIG_HOME") {
        Some(xdg) => PathBuf::from(xdg),
        None => home.join(".config"),
    };
    base.join("keyswitch")
}

#[must_use]
pub fn config_dir() -> PathBuf {
    config_dir_from(running_on_windows(), &home_dir(), &|name| std::env::var(name).ok())
}

fn running_on_windows() -> bool {
    cfg!(windows)
}

/// The shipped settings of `platform` (the running one by default, `Some("darwin")` for macOS): `DEFAULT_SETTINGS`
/// everywhere, but on macOS the selection hotkeys use keys a Mac keyboard has (`MACOS_SELECTION_HOTKEY_DEFAULTS`).
pub fn default_settings(platform: Option<&str>) -> SettingsData {
    let mut defaults = DEFAULT_SETTINGS.clone();
    let running = if cfg!(target_os = "macos") { "darwin" } else { "other" };
    if platform.unwrap_or(running) == "darwin"
        && let Some(Value::Object(hotkeys)) = defaults.get_mut("hotkeys")
    {
        for (name, value) in MACOS_SELECTION_HOTKEY_DEFAULTS {
            hotkeys.insert(name.to_string(), Value::String(value.to_string()));
        }
    }
    defaults
}

/// Move a user file that could not be read out of the way of the next save.
///
/// The application then runs on defaults, and saving them must not destroy what the user had: the bytes stay next to
/// the file under a dated name.
#[must_use]
pub fn set_aside_unreadable(path: &Path) -> Option<PathBuf> {
    let suffix = format!("{UNREADABLE_FILE_SUFFIX}{}", strftime(UNREADABLE_FILE_TIME_FORMAT, &local_time()));
    let target = with_name_suffix(path, &suffix);
    if let Ok(()) = std::fs::rename(path, &target) {
        let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        warning(LOGGER_NAME, &format!("{} could not be read; kept as {}, defaults in use", path.display(), name));
        Some(target)
    } else {
        warning(LOGGER_NAME, &format!("{} could not be read or moved aside", path.display()));
        None
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

struct Data {
    data: SettingsData,
    unreadable: Option<PathBuf>,
}

type Callbacks = Arc<Mutex<Vec<(u64, SettingCallback)>>>;

/// Thread-safe JSON settings with dotted-path access and change callbacks.
pub struct SettingsStore {
    pub path: PathBuf,
    defaults: SettingsData,
    state: Mutex<Data>,
    callbacks: Callbacks,
    next_callback: Mutex<u64>,
}

impl SettingsStore {
    /// `SettingsStore(path)`: the file is read at once (`config_dir()/config.json` for `None`).
    #[must_use]
    pub fn new(path: Option<PathBuf>) -> SettingsStore {
        let defaults = default_settings(None);
        let store = SettingsStore {
            path: path.unwrap_or_else(|| config_dir().join("config.json")),
            state: Mutex::new(Data { data: defaults.clone(), unreadable: None }),
            defaults,
            callbacks: Arc::new(Mutex::new(Vec::new())),
            next_callback: Mutex::new(0),
        };
        store.load();
        store
    }

    /// Where an unreadable settings file was moved by the last load, if anywhere.
    pub fn unreadable(&self) -> Option<PathBuf> {
        lock(&self.state).unreadable.clone()
    }

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
        let loaded_mapping = loaded.as_ref().and_then(|value| string_keyed_mapping(value));
        let Some(loaded_mapping) = loaded_mapping else {
            // Keep safe defaults, and keep the user's file: the next change of any setting would otherwise save the
            // defaults over it.
            state.data.clone_from(&self.defaults);
            state.unreadable = set_aside_unreadable(&self.path);
            return;
        };
        state.data = deep_merge(&self.defaults, loaded_mapping);
        Self::adopt_new_defaults(&mut state.data, loaded_mapping);
    }

    /// Let a file written by an older schema take the defaults that changed since.
    ///
    /// The persisted file is seeded from the shipped defaults, so a value equal to the old default is most likely the
    /// seed and not a choice. Schema 7 turned the early layout switch on; a schema-6 file with it off adopts that
    /// once. Schema 8 shortened the pause correction from 1.5 s; an older file with the old pause adopts the new one
    /// once. The schema version written back marks the adoption done, so a value set again afterwards holds.
    fn adopt_new_defaults(data: &mut SettingsData, loaded: &SettingsData) {
        let Some(version) = loaded.get("schema_version") else { return };
        // isinstance(version, int) and not a bool; a size beyond 64 bits reads as a float here and gives the same answer.
        let Some(version) = version.as_i64().filter(|_| is_py_int(version)) else { return };
        if version >= PAUSE_DELAY_DEFAULT_SCHEMA_VERSION {
            return;
        }
        let detection = loaded.get("detection").and_then(Value::as_object);
        if let (Some(detection), Some(Value::Object(current))) = (detection, data.get_mut("detection")) {
            if version < EARLY_SWITCH_DEFAULT_SCHEMA_VERSION && detection.get("early_switch") == Some(&Value::Bool(false)) {
                current.insert("early_switch".into(), Value::Bool(true));
            }
            let previous = serde_json::Number::from_f64(PREVIOUS_DEFAULT_PAUSE_DELAY_SECONDS).map(Value::Number);
            if let (Some(stored), Some(previous)) = (detection.get("pause_delay_seconds"), previous)
                && py_eq(stored, &previous)
            {
                let default = serde_json::Number::from_f64(DEFAULT_PAUSE_DELAY_SECONDS).map(Value::Number);
                current.insert("pause_delay_seconds".into(), default.unwrap_or(Value::Null));
            }
        }
        data.insert("schema_version".into(), DEFAULT_SETTINGS["schema_version"].clone());
    }

    fn save_locked(&self, data: &SettingsData) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
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

    /// Writes the settings: aside under a temporary name, then swapped in.
    pub fn save(&self) -> io::Result<()> {
        let state = lock(&self.state);
        self.save_locked(&state.data)
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
                target = match target.get_mut(*part) {
                    Some(Value::Object(map)) => map,
                    _ => unreachable!("a mapping was just put there"),
                };
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
            callback(dotted_path, &value);
        }
        Ok(())
    }

    fn callbacks_snapshot(&self) -> Vec<SettingCallback> {
        lock(&self.callbacks).iter().map(|(_, callback)| callback.clone()).collect()
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

    pub fn subscribe(&self, callback: SettingCallback) -> Unsubscribe {
        let id = {
            let mut next = lock(&self.next_callback);
            *next += 1;
            *next
        };
        lock(&self.callbacks).push((id, callback));
        let callbacks = Arc::clone(&self.callbacks);
        Box::new(move || {
            lock(&callbacks).retain(|(held, _)| *held != id);
        })
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
            callback("*", &Value::Object(self.snapshot()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temporary(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("ks-config-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn paths_honor_overrides_and_xdg() {
        let home = Path::new("/home/u");
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string())
        };
        assert_eq!(config_dir_from(false, home, &env(&[("KEYSWITCH_CONFIG_DIR", "/tmp/key-conf")])), Path::new("/tmp/key-conf"));
        assert_eq!(config_dir_from(false, home, &env(&[("XDG_CONFIG_HOME", "/tmp/xdg")])), Path::new("/tmp/xdg/keyswitch"));
        assert_eq!(config_dir_from(false, home, &env(&[("XDG_CONFIG_HOME", "")])), Path::new("/home/u/.config/keyswitch"));
        assert_eq!(config_dir_from(true, home, &env(&[("APPDATA", "C:/R")])), Path::new("C:/R/KeySwitch"));
        assert_eq!(config_dir_from(true, home, &env(&[])), Path::new("/home/u/AppData/Roaming/KeySwitch"));
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
            assert_eq!(store.get("schema_version"), Some(json!(8)));
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
        assert!(std::fs::read_to_string(&path).unwrap().ends_with("}\n"));
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
        assert_eq!(store.get("schema_version"), Some(json!(8)));
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
        assert_eq!(Value::Object(store.snapshot()), Value::Object(DEFAULT_SETTINGS.clone()));
        let on_disk: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(on_disk, Value::Object(DEFAULT_SETTINGS.clone()));
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
        assert_eq!(mac["hotkeys"]["selection_layout"], json!("Ctrl+Alt+L"));
        assert_eq!(default_settings(Some("linux"))["hotkeys"]["selection_layout"], json!("Shift+Pause"));
    }
}
