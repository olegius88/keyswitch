//! Versioned, privacy-aware configuration deltas for the technical journal.

use crate::config::{SettingsData, SettingsStore};
use crate::constants::LOGGED_SETTING_VALUE_MAX_CHARACTERS;
use crate::pycompat::py_eq;
use ks_text::pystr;
use ks_text::{json_dumps, JsonOptions};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

fn defaults_sha256(defaults: &SettingsData) -> String {
    let options = JsonOptions {
        ensure_ascii: false,
        sort_keys: true,
        separators: Some((",".to_string(), ":".to_string())),
        ..JsonOptions::default()
    };
    let text = json_dumps(&Value::Object(defaults.clone()), &options);
    Sha256::digest(text.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect()
}

const LOGGABLE_STRING_TRUNCATED_CHARACTERS: usize = LOGGED_SETTING_VALUE_MAX_CHARACTERS - "...".len();

fn loggable_value(value: &Value) -> Value {
    match value {
        Value::Bool(_) | Value::Number(_) | Value::Null => value.clone(),
        Value::String(text) => {
            if pystr::len(text) <= LOGGED_SETTING_VALUE_MAX_CHARACTERS {
                value.clone()
            } else {
                Value::String(format!("{}...", pystr::slice(text, None, Some(LOGGABLE_STRING_TRUNCATED_CHARACTERS as i64))))
            }
        }
        Value::Array(items) => json!({"type": "list", "items": items.len()}),
        Value::Object(items) => json!({"type": "dict", "items": items.len()}),
    }
}

fn overrides(defaults: &SettingsData, current: &SettingsData, prefix: &str) -> SettingsData {
    let mut result = Map::new();
    // Walk known defaults, never arbitrary names from user configuration.
    for (name, default) in defaults {
        let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}.{name}") };
        if path == "schema_version" {
            continue;
        }
        let value = current.get(name);
        if let (Value::Object(default_map), Some(Value::Object(value_map))) = (default, value) {
            for (key, item) in overrides(default_map, value_map, &path) {
                result.insert(key, item);
            }
        } else {
            match value {
                None => {
                    result.insert(path, json!({"missing": true}));
                }
                Some(value) if !py_eq(value, default) => {
                    result.insert(path, loggable_value(value));
                }
                Some(_) => {}
            }
        }
    }
    result
}

/// One atomic snapshot; omitted known paths use the shipped defaults of this platform.
pub fn settings_snapshot(settings: &SettingsStore) -> SettingsData {
    let defaults = settings.defaults();
    let mut snapshot = Map::new();
    snapshot.insert("format".into(), json!("overrides-v1"));
    snapshot.insert("defaults_schema".into(), defaults["schema_version"].clone());
    snapshot.insert("defaults_sha256".into(), json!(defaults_sha256(&defaults)));
    snapshot.insert("overrides".into(), Value::Object(overrides(&defaults, &settings.snapshot(), "")));
    snapshot
}

/// Represent reset without repeating its default; never disclose unknown keys.
pub fn setting_change(settings: &SettingsStore, path: &str, value: &Value) -> Option<SettingsData> {
    let default = settings.default(path);
    let mut change = Map::new();
    if path == "*" || matches!(default, Some(Value::Object(_))) {
        change.insert("path".into(), json!(path));
        change.insert("operation".into(), json!("snapshot"));
        change.insert("settings".into(), Value::Object(settings_snapshot(settings)));
        return Some(change);
    }
    let default = match default {
        Some(default) if path != "schema_version" => default,
        _ => return None,
    };
    change.insert("path".into(), json!(path));
    if py_eq(value, &default) {
        change.insert("operation".into(), json!("reset"));
    } else {
        change.insert("operation".into(), json!("set"));
        change.insert("value".into(), loggable_value(value));
    }
    Some(change)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> SettingsStore {
        let directory = std::env::temp_dir().join(format!("ks-diag-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        SettingsStore::new(Some(directory.join("config.json")))
    }

    #[test]
    fn defaults_have_no_overrides() {
        let settings = store("defaults");
        let snapshot = settings_snapshot(&settings);
        assert_eq!(snapshot["format"], "overrides-v1");
        assert_eq!(snapshot["defaults_schema"], 8);
        assert_eq!(snapshot["overrides"], json!({}));
        assert_eq!(snapshot["defaults_sha256"].as_str().unwrap().len(), 64);
    }

    #[test]
    fn collections_are_summarised_and_strings_bounded() {
        let settings = store("collections");
        settings.set("exclusions.words", json!(["secret", "words"]), false).unwrap();
        settings.set("appearance.theme", json!("я".repeat(100)), false).unwrap();
        settings.set("unknown.thing", json!("hidden"), false).unwrap();
        settings.set("detection.confidence", json!(true), false).unwrap();
        let snapshot = settings_snapshot(&settings);
        let overrides = snapshot["overrides"].as_object().unwrap();
        assert_eq!(overrides["exclusions.words"], json!({"type": "list", "items": 2}));
        assert_eq!(overrides["appearance.theme"], json!(format!("{}...", "я".repeat(77))));
        assert_eq!(overrides["detection.confidence"], json!(true));
        assert!(!overrides.contains_key("unknown.thing"));
        let change = setting_change(&settings, "detection.confidence", &json!(2.0)).unwrap();
        assert_eq!(change["operation"], "reset");
        assert!(setting_change(&settings, "unknown.thing", &json!(1)).is_none());
        assert!(setting_change(&settings, "schema_version", &json!(1)).is_none());
        assert_eq!(setting_change(&settings, "detection", &json!({})).unwrap()["operation"], "snapshot");
    }
}
