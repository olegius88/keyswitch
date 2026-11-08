//! `config.json` after a load and a save, byte for byte: what was edited by hand or written by an older program stays, and a key the
//! program does not know is kept at every level. The expected files are what the program wrote before its settings were
//! typed (the same inputs through the store over the JSON tree).

use ks_config::config::SettingsStore;
use ks_config::constants::SETTINGS_SINCE_PYTHON;
use std::path::PathBuf;

const EDITED: &str = r#"{"schema_version": 8, "note": "keep me", "mine": {"kept": [1, {"b": null}], "a": 1.5}, "detection": {"minimum_length": "seven", "unknown_inside": {"deep": true}, "confidence": 3.25}, "hotkeys": {"toggle": 5, "another": "F9"}, "updates": "none", "enabled": false, "extra": [1, "two", null]}"#;
const OLD_SCHEMA: &str = r#"{"schema_version": 7, "detection": {"early_switch": false, "pause_delay_seconds": 1.5, "unknown": 1}, "general": {"sound": true, "new_in_future": "x"}, "future_table": {"a": {"b": {"c": []}}}}"#;

fn file(name: &str, text: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("ks-config-bytes-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("the folder");
    let path = directory.join("config.json");
    std::fs::write(&path, text).expect("the file");
    path
}

fn saved(path: &PathBuf) -> String {
    std::fs::read_to_string(path).expect("the file").replace("\r\n", "\n")
}

/// A text without the lines of the settings the Python never had, and without the commas at the ends of its lines (a setting
/// that came later moves them): a newer program adds such a setting with its default, and that is not what this test pins.
fn without_newer_settings(text: &str) -> String {
    let names: Vec<String> = SETTINGS_SINCE_PYTHON.iter().map(|path| format!("\"{}\":", path.rsplit('.').next().expect("a name"))).collect();
    text.lines()
        .filter(|line| !names.iter().any(|name| line.trim_start().starts_with(name.as_str())))
        .map(|line| format!("{}\n", line.trim_end_matches(',')))
        .collect()
}

/// Schema 9 on a Mac: the conversion hotkey differs there, the file and the rest stay.
fn for_this_system(text: &str) -> String {
    if cfg!(target_os = "macos") { text.replace("\"schema_version\": 8", "\"schema_version\": 9").replace("\"convert_last\": \"Pause\"", "\"convert_last\": \"Ctrl+Alt+K\"").replace("\"selection_layout\": \"Shift+Pause\"", "\"selection_layout\": \"Ctrl+Alt+L\"").replace("\"selection_case\": \"Alt+Pause\"", "\"selection_case\": \"Ctrl+Alt+U\"").replace("\"selection_transliterate\": \"Alt+Scroll_Lock\"", "\"selection_transliterate\": \"Ctrl+Alt+T\"") } else { text.to_owned() }
}

#[test]
fn unknown_keys_at_two_levels_and_a_known_key_of_another_type_survive_a_load_and_a_save() {
    let path = file("edited", EDITED);
    let store = SettingsStore::new(Some(path.clone()));
    store.save().expect("saved");
    assert_eq!(without_newer_settings(&saved(&path)), without_newer_settings(&for_this_system(include_str!("fixtures/config_after_save_edited.json"))));
}

#[test]
fn a_file_of_an_older_schema_is_written_with_the_defaults_it_adopted() {
    let path = file("old", OLD_SCHEMA);
    let store = SettingsStore::new(Some(path.clone()));
    store.save().expect("saved");
    assert_eq!(without_newer_settings(&saved(&path)), without_newer_settings(&for_this_system(include_str!("fixtures/config_after_save_old_schema.json"))));
}

#[test]
fn a_second_load_and_save_changes_nothing_more() {
    let path = file("twice", EDITED);
    SettingsStore::new(Some(path.clone())).save().expect("saved");
    let first = saved(&path);
    SettingsStore::new(Some(path.clone())).save().expect("saved");
    assert_eq!(saved(&path), first);
}
