//! Versioned, privacy-aware configuration deltas for the technical journal: typed records of what `settings_file` found.

use crate::config::SettingsStore;
use crate::constants::SettingChange;
use crate::settings_file::{overrides, schema_of};
pub use crate::settings_file::{defaults_sha256, setting_change, without_settings_since_python};
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Number;

/// A value of a setting as the log shows it: a list or a table only by its length, a long text cut short.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum LoggedValue {
    Flag(bool),
    Number(Number),
    Text(String),
    Collection {
        #[serde(rename = "type")]
        r#type: &'static str,
        items: usize,
    },
    /// A known setting the file no longer holds.
    Missing { missing: bool },
    Nothing,
}

/// One atomic picture of the settings: where they differ from the shipped defaults.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SettingsSnapshot {
    pub format: &'static str,
    pub defaults_schema: i64,
    pub defaults_sha256: String,
    pub overrides: IndexMap<String, LoggedValue>,
}

/// What became of a setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    /// A change of many settings at once: the record holds the whole picture.
    Snapshot,
    /// The setting is back at its default, which the log does not repeat.
    Reset,
    Set,
}

/// A record of the technical log about one change of a setting.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SettingChangeRecord {
    pub path: String,
    pub operation: Operation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings: Option<SettingsSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<LoggedValue>,
}

/// One atomic snapshot; omitted known paths use the shipped defaults of this platform.
#[must_use]
pub fn settings_snapshot(settings: &SettingsStore) -> SettingsSnapshot {
    let defaults = settings.defaults();
    SettingsSnapshot {
        format: "overrides-v1",
        defaults_schema: schema_of(&defaults),
        defaults_sha256: defaults_sha256(&defaults),
        overrides: overrides(&defaults, &settings.snapshot()),
    }
}

/// [`setting_change`] for a typed change: the record of the value as the key read it, which is the value itself for every
/// typed write (an untyped `set` of `1` under a flag shows here as `true`; [`setting_change`] takes the tree as it was set).
#[must_use]
pub fn change_record(settings: &SettingsStore, change: &SettingChange) -> Option<SettingChangeRecord> {
    setting_change(settings, change.path(), &change.stored())
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::constants::{DETECTION_CONFIDENCE, DETECTION_EARLY_SWITCH, EXCLUSIONS_WORDS};
    use ks_text::json::{Kept, dumps};
    use ks_text::JsonOptions;

    fn store(name: &str) -> SettingsStore {
        let directory = std::env::temp_dir().join(format!("ks-diagnostics-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        SettingsStore::new(Some(directory.join("config.json")))
    }

    fn operation_of(record: Option<SettingChangeRecord>) -> Option<Operation> {
        record.map(|record| record.operation)
    }

    #[test]
    fn a_typed_change_reads_as_the_record_of_its_value() {
        let settings = store("typed");
        assert_eq!(operation_of(change_record(&settings, &SettingChange::Reset)), Some(Operation::Snapshot));
        settings.write(DETECTION_CONFIDENCE, 3.25, false).unwrap();
        let set = change_record(&settings, &SettingChange::DetectionConfidence(3.25)).unwrap();
        assert_eq!(dumps(&set, &JsonOptions::default()), r#"{"path": "detection.confidence", "operation": "set", "value": 3.25}"#);
        assert_eq!(operation_of(change_record(&settings, &SettingChange::DetectionConfidence(2.0))), Some(Operation::Reset));
        settings.write(DETECTION_EARLY_SWITCH, false, false).unwrap();
        settings.write(EXCLUSIONS_WORDS, vec!["a".to_owned(), "b".to_owned()], false).unwrap();
        let list = change_record(&settings, &SettingChange::ExclusionsWords(vec!["a".to_owned(), "b".to_owned()])).unwrap();
        assert_eq!(dumps(&list, &JsonOptions::default()), r#"{"path": "exclusions.words", "operation": "set", "value": {"type": "list", "items": 2}}"#);
        assert_eq!(operation_of(change_record(&settings, &SettingChange::SchemaVersion(1))), None);
        // A change no row reads goes by its path and its value, as the record of an untyped change does.
        let other = |path: &str, text: &str| SettingChange::Other { path: path.to_owned(), value: Kept::text(text.to_owned()) };
        assert_eq!(change_record(&settings, &other("mine.own", "x")), None);
        let odd = change_record(&settings, &other("detection.confidence", "x")).unwrap();
        assert_eq!(dumps(&odd, &JsonOptions::default()), r#"{"path": "detection.confidence", "operation": "set", "value": "x"}"#);
    }
}
