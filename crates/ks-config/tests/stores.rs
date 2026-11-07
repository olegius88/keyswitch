//! The stores over files that are odd, damaged or written by older versions, and the small values around them.
#![expect(clippy::disallowed_types, clippy::disallowed_macros, clippy::disallowed_methods, reason = "temporary: these tests still build and read their JSON as untyped trees; step 7 of the strict-json plan types them and removes this")]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use ks_config::app_quirks::TELEGRAM_QUOTE_MENTION;
use ks_config::config::SettingsStore;
use ks_config::constants::{LOG_SECRET_REMEMBERED_WORDS, PERSONAL_WORDS_MAX};
use ks_config::history::{HistoryEntry, HistoryStore, data_dir_from};
use ks_config::indicator::layout_icon_name;
use ks_config::learning::{LearnedRule, LearningError, LearningStore, RuleAction, RuleMatch, migrate_legacy};
use ks_config::log_privacy::{LogRedactor, masked_text, secret_token};
use ks_config::logsetup::{LOG_FILE_NAME, Level, LogRecord, RotatingFileHandler, configure_logging, utc_time};
use ks_config::personal_words::PersonalWords;
use ks_config::pycompat::{encoded_text, py_eq};
use ks_config::settings_diagnostics::setting_change;
use serde_json::{Map, Value, json};

fn folder(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let number = NEXT.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!("ks-stores-{name}-{}-{number}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("the folder");
    directory
}

fn object(value: &Value) -> Map<String, Value> {
    value.as_object().expect("an object").clone()
}

// ---------------------------------------------------------------- settings

#[test]
fn settings_of_older_files_adopt_new_defaults_once() {
    let cases = [
        (json!({"detection": {"early_switch": true}}), true),
        (json!({"schema_version": "x"}), true),
        (json!({"schema_version": 1}), true),
        (json!({"schema_version": 6, "detection": {"early_switch": false}}), true),
        (json!({"schema_version": 7, "detection": {"early_switch": false}}), false),
        (json!({"schema_version": 5, "detection": {"early_switch": true}}), true),
        (json!({"schema_version": 7, "detection": {"pause_delay_seconds": 1.5}}), true),
        (json!({"schema_version": 7, "detection": {"pause_delay_seconds": 0.5}}), true),
        (json!({"schema_version": 99, "detection": {"early_switch": false}}), false),
    ];
    let directory = folder("settings");
    for (number, (file, early)) in cases.iter().enumerate() {
        let path = directory.join(format!("{number}.json"));
        std::fs::write(&path, file.to_string()).expect("the file");
        let store = SettingsStore::new(Some(path));
        store.load();
        assert_eq!(store.get("detection.early_switch"), Some(Value::Bool(*early)), "{file}");
        store.save().expect("saved");
    }
    let path = directory.join("pause.json");
    std::fs::write(&path, json!({"schema_version": 7, "detection": {"pause_delay_seconds": 1.5}}).to_string()).expect("the file");
    let store = SettingsStore::new(Some(path));
    store.load();
    assert_ne!(store.get("detection.pause_delay_seconds"), Some(json!(1.5)));
}

#[test]
fn settings_in_a_folder_instead_of_a_file_are_set_aside() {
    let path = folder("settings-dir");
    let store = SettingsStore::new(Some(path));
    store.load();
    assert_eq!(store.get("detection.early_switch"), Some(Value::Bool(true)));
}

#[test]
fn a_change_of_a_setting_for_the_log() {
    let directory = folder("diagnostics");
    let store = SettingsStore::new(Some(directory.join("settings.json")));
    store.load();
    let all = setting_change(&store, "*", &Value::Null).expect("a snapshot");
    assert_eq!(all.get("operation"), Some(&json!("snapshot")));
    assert_eq!(setting_change(&store, "no.such.path", &Value::Null), None);
    assert_eq!(setting_change(&store, "schema_version", &json!(1)), None);
    let default = store.default("detection.early_switch").expect("a default");
    assert_eq!(setting_change(&store, "detection.early_switch", &default).and_then(|change| change.get("operation").cloned()), Some(json!("reset")));
    let path = "detection.early_switch";
    let changed = |value: Value| setting_change(&store, path, &value).expect("known").get("value").cloned();
    assert_eq!(changed(json!(false)), Some(json!(false)));
    assert_eq!(changed(json!("short")), Some(json!("short")));
    assert!(changed(json!("x".repeat(10_000))).is_some_and(|value| value.as_str().is_some_and(|text| text.ends_with("...") && text.len() < 10_000)));
    assert_eq!(changed(json!([1, 2])), Some(json!({"type": "list", "items": 2})));
    assert_eq!(changed(json!({"a": 1})), Some(json!({"type": "dict", "items": 1})));
}

// ---------------------------------------------------------------- history

#[test]
fn the_data_folder_comes_from_the_environment_or_the_home() {
    let home = PathBuf::from("/home/user");
    let values = [("KEYSWITCH_DATA_DIR", "/over")];
    let over = |name: &str| values.iter().find(|(key, _)| *key == name).map(|(_, value)| (*value).to_owned());
    assert_eq!(data_dir_from(false, &home, &over), PathBuf::from("/over"));
    assert_eq!(data_dir_from(true, &home, &over), PathBuf::from("/over"));
    let nothing = |_: &str| None;
    assert_eq!(data_dir_from(true, &home, &nothing), home.join("AppData").join("Local").join("KeySwitch"));
    let local = |name: &str| (name == "LOCALAPPDATA").then(|| "/local".to_owned());
    assert_eq!(data_dir_from(true, &home, &local), PathBuf::from("/local").join("KeySwitch"));
}

#[test]
fn history_skips_what_is_not_an_entry() {
    let directory = folder("history");
    let path = directory.join("history.jsonl");
    let store = HistoryStore::new(Some(path.clone()), 10);
    for number in 0..3 {
        store.append(&HistoryEntry::create(&format!("o{number}"), "r", "app", 0.5)).expect("appended");
    }
    let mut text = std::fs::read_to_string(&path).expect("the file");
    text.push_str("[1]\n7\nnot json\n{\"a\": 1}\n");
    std::fs::write(&path, text).expect("the file");
    assert_eq!(store.read(None).expect("read").len(), 3);
    assert_eq!(store.read(Some(0)).expect("read").len(), 3);
    assert_eq!(store.read(Some(2)).expect("read").len(), 2);
    assert_eq!(store.read(Some(5)).expect("read").len(), 3);
    store.set_limit(2);
    assert_eq!(store.limit(), 2);
    std::fs::write(&path, b"\xff\xfe\xff").expect("the file");
    assert!(store.read(None).is_err());
}

// ---------------------------------------------------------------- rules

fn rule(pattern: &str, source: i64, target: i64) -> LearnedRule {
    LearnedRule::new(pattern, source, target)
}

#[test]
fn a_continuation_of_a_rule_respects_case_and_group() {
    let mut exact = rule("AbC", 0, 1);
    exact.case_sensitive = true;
    assert!(exact.may_match_continuation("Ab"));
    assert!(!exact.may_match_continuation("ab"));
    exact.case_sensitive = false;
    assert!(exact.may_match_continuation("ab"));
    let directory = folder("continuation");
    let store = LearningStore::new(Some(directory.join("rules.json")), None).expect("the store");
    let mut keep = rule("слово", 1, 0);
    keep.action = RuleAction::Keep;
    keep.r#match = RuleMatch::Prefix;
    store.add_rule(keep).expect("added");
    assert!(store.keeps_continuation(1, "сл"));
    assert!(!store.keeps_continuation(0, "сл"));
}

#[test]
fn old_rule_files_carry_over_only_what_acted() {
    let cases = [
        (json!({"rules": [1]}), 0),
        (json!({"rules": {"nocolon": {"target_group": 1, "confirmations": 9}}}), 0),
        (json!({"rules": {"x:word": {"target_group": 1, "confirmations": 9}}}), 0),
        (json!({"rules": {"0:word": 1}}), 0),
        (json!({"rules": {"0:word": {}}}), 0),
        (json!({"rules": {"0:word": {"target_group": 1}}}), 0),
        (json!({"rules": {"0:word": {"target_group": 1, "confirmations": "x"}}}), 0),
        (json!({"rules": {"0:word": {"target_group": 1, "confirmations": 9}}}), 1),
        (json!({"rules": {"0:": {"target_group": 1, "confirmations": 9}}}), 0),
        (json!({"rules": {"0:word": {"target_group": 0, "confirmations": 9}}}), 0),
        (json!({"rejections": []}), 0),
        (json!({"rejections": {"nocolon": [1]}}), 0),
        (json!({"rejections": {"0:word": 1}}), 0),
        (json!({"rejections": {"0:word": ["x", 1]}}), 1),
        (json!({"rules": {"0:word": {"target_group": 1, "confirmations": 9}}, "rejections": {"0:word": [1]}}), 1),
    ];
    for (payload, count) in cases {
        assert_eq!(migrate_legacy(&object(&payload), 3).expect("no error").len(), count, "{payload}");
    }
}

#[test]
fn a_rule_of_the_file_with_an_odd_flag_is_no_rule() {
    let directory = folder("flag");
    let path = directory.join("rules.json");
    let good = json!({"pattern": "word", "source_group": 0, "target_group": 1, "action": "convert", "match": "exact", "case_sensitive": true});
    let odd = json!({"pattern": "other", "source_group": 0, "target_group": 1, "action": "convert", "match": "exact", "case_sensitive": 1});
    std::fs::write(&path, json!({"schema_version": 2, "rules": [good, odd]}).to_string()).expect("the file");
    let store = LearningStore::new(Some(path), None).expect("the store");
    assert_eq!(store.count(), 1);
}

#[test]
fn rules_in_a_folder_instead_of_a_file_read_as_none() {
    let store = LearningStore::new(Some(folder("rules-dir")), None).expect("the store");
    assert_eq!(store.count(), 0);
}

/// The owner of the file cannot read it either; root can, and then the rule is simply read.
#[cfg(unix)]
#[test]
fn a_rule_file_that_cannot_be_read_counts_as_damaged() {
    use std::os::unix::fs::PermissionsExt as _;
    let path = folder("no-read").join("rules.json");
    std::fs::write(&path, b"{}").expect("the file");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).expect("the mode");
    let store = LearningStore::new(Some(path), None).expect("the store");
    assert_eq!(store.count(), 0);
}

#[test]
fn replacing_a_rule() {
    let directory = folder("replace");
    let store = LearningStore::new(Some(directory.join("rules.json")), None).expect("the store");
    let old = store.add_rule(rule("word", 0, 1)).expect("added");
    let new = store.replace_rule(&old, rule("other", 0, 1)).expect("replaced");
    assert_eq!(store.rules(), std::slice::from_ref(&new));
    let refused = store.replace_rule(&new, rule("", 0, 1));
    assert!(matches!(refused, Err(LearningError::Invalid(_))), "{refused:?}");
    assert_eq!(store.rules(), [new]);
}

// ---------------------------------------------------------------- the log

#[test]
fn secrets_are_masked_at_any_depth() {
    assert!(secret_token("parol123"));
    assert!(!secret_token("12345678901"));
    assert!(!secret_token("onlyletters"));
    let none = std::collections::HashSet::new();
    assert_eq!(masked_text("ok ", &none), "ok ");
    assert_eq!(masked_text("ok", &none), "ok");
    let mut redactor = LogRedactor::new();
    let masked = redactor.record(&object(&json!({"list": ["parol123", 7, null, true], "number": 5})));
    assert_eq!(masked.get("number"), Some(&json!(5)));
    assert_ne!(masked.get("list"), Some(&json!(["parol123", 7, null, true])));
    assert_eq!(masked.get("list").and_then(|list| list.get(1)), Some(&json!(7)));
}

#[test]
fn the_words_after_a_password_marker_are_remembered_and_the_oldest_forgotten() {
    let mut redactor = LogRedactor::new();
    let after_marker = |redactor: &mut LogRedactor, word: &str| {
        let _ = redactor.record(&object(&json!({"context": {"words": {"before": "пароль"}}, "nested": [{"original": word}]})));
    };
    after_marker(&mut redactor, "");
    after_marker(&mut redactor, "!!!");
    after_marker(&mut redactor, "пароль");
    for number in 0..=LOG_SECRET_REMEMBERED_WORDS {
        after_marker(&mut redactor, &format!("word{number}x"));
    }
    let last = format!("word{LOG_SECRET_REMEMBERED_WORDS}x");
    let again = redactor.record(&object(&json!({"text": last})));
    assert_ne!(again.get("text"), Some(&json!(last)));
    let first = redactor.record(&object(&json!({"text": "word0x"})));
    assert_eq!(first.get("text"), Some(&json!("word0x")));
}

#[test]
fn a_journal_without_limits_never_rolls_over() {
    let directory = folder("journal");
    let path = directory.join("keyswitch.log");
    let handler = RotatingFileHandler::new(&path, 0, 0).expect("the handler");
    let record = LogRecord { time: utc_time(0), level: Level::Info, name: "test".to_owned(), message: "x".repeat(100) };
    handler.emit(&record);
    handler.emit(&record);
    handler.do_rollover().expect("rolled");
    handler.close();
    assert!(!directory.join("keyswitch.log.1").exists());
    for (level, name) in [(Level::Debug, "DEBUG"), (Level::Info, "INFO"), (Level::Warning, "WARNING"), (Level::Error, "ERROR"), (Level::Critical, "CRITICAL")] {
        assert_eq!(level.name(), name);
    }
}

// ---------------------------------------------------------------- personal words

#[test]
fn a_personal_file_without_its_parts_is_set_aside() {
    let directory = folder("personal");
    let parts = [
        json!({"schema_version": 1}),
        json!({"schema_version": 1, "salt": "s"}),
        json!({"schema_version": 1, "salt": "s", "personal": []}),
        json!({"schema_version": 1, "salt": 1, "personal": [], "counts": {}}),
        json!({"schema_version": 1, "salt": "s", "personal": 1, "counts": {}}),
        json!({"schema_version": 1, "salt": "s", "personal": [], "counts": 1}),
    ];
    for (number, payload) in parts.iter().enumerate() {
        let path = directory.join(format!("{number}.json"));
        std::fs::write(&path, payload.to_string()).expect("the file");
        let words = PersonalWords::always_persisting(Some(path));
        assert!(words.unreadable().is_some(), "{payload}");
    }
}

#[test]
fn a_personal_file_with_doubles_and_odd_counts() {
    let directory = folder("doubles");
    let path = directory.join("words.json");
    let payload = json!({"schema_version": 1, "salt": "s", "personal": ["a", "a", 7, "b"], "counts": {"a": 1, "b": 0, "c": "x", "d": -1, "e": 2}});
    std::fs::write(&path, payload.to_string()).expect("the file");
    let words = PersonalWords::always_persisting(Some(path.clone()));
    assert_eq!(words.unreadable(), None);
    assert_eq!(words.len(), 2);
    words.save();
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("saved")).expect("JSON");
    assert_eq!(saved["counts"], json!({"a": 1, "e": 2}));
}

#[test]
fn the_oldest_personal_word_goes_out_over_the_limit() {
    let words = PersonalWords::new(Some(folder("limit").join("words.json")), Box::new(|| false));
    for number in 0..=PERSONAL_WORDS_MAX {
        words.add(&format!("word{number}"));
    }
    assert_eq!(words.len(), PERSONAL_WORDS_MAX);
    assert!(!words.contains("word0"));
    assert!(words.contains("word1"));
}

// ---------------------------------------------------------------- small values

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
fn text_for_a_file_has_the_line_ends_of_the_system() {
    assert_eq!(encoded_text("ab"), b"ab");
    let lines = encoded_text("a\nb");
    assert_eq!(lines, if cfg!(windows) { b"a\r\nb".as_slice() } else { b"a\nb".as_slice() });
}

/// The environment is set by starting this test again: nothing else in this file reads it.
#[test]
fn logging_goes_to_the_data_folder() {
    const CHILD: &str = "KS_TEST_LOG_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let directory = folder("logging");
        let status = std::process::Command::new(std::env::current_exe().expect("this test"))
            .args(["--exact", "logging_goes_to_the_data_folder"])
            .env(CHILD, "1")
            .env("KEYSWITCH_DATA_DIR", &directory)
            .status()
            .expect("started");
        assert!(status.success());
        assert!(directory.join(LOG_FILE_NAME).exists());
        return;
    }
    let handler = configure_logging(None).expect("the journal");
    handler.close();
}

#[test]
fn a_mention_applies_only_to_its_own_pair() {
    let head = &TELEGRAM_QUOTE_MENTION;
    let application = head.applications[0];
    assert!(head.applies(application, head.typed, head.meant));
    assert!(!head.applies(application, head.typed, "other"));
    assert!(!head.applies(application, "other", head.meant));
    assert!(!head.applies("no such program", head.typed, head.meant));
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
