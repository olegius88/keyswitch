//! The stores over files that are odd, damaged or written by older versions, and the small values around them.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use ks_config::app_quirks::TELEGRAM_QUOTE_MENTION;
use ks_config::config::SettingsStore;
use ks_config::constants::{DETECTION_EARLY_SWITCH, DETECTION_PAUSE_DELAY_SECONDS, PERSONAL_WORDS_MAX, SettingChange};
use ks_config::history::{HistoryEntry, HistoryStore, data_dir_from};
use ks_config::learning::{LearnedRule, LearningError, LearningFile, LearningStore, RuleAction, RuleMatch, migrate_legacy};
use ks_config::logsetup::{LOG_FILE_NAME, Level, LogRecord, RotatingFileHandler, configure_logging, utc_time};
use ks_config::personal_words::PersonalWords;
use ks_config::pycompat::encoded_text;
use ks_config::settings_diagnostics::{LoggedValue, Operation, change_record};
use ks_text::json::{ObjectOf, loads_lenient};

fn folder(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let number = NEXT.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!("ks-stores-{name}-{}-{number}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("the folder");
    directory
}

/// A legacy learning file as `load` reads it.
fn legacy_file(text: &str) -> LearningFile {
    loads_lenient::<ObjectOf<LearningFile>>(text).expect("a legacy file").0
}

// ---------------------------------------------------------------- settings

#[test]
fn settings_of_older_files_adopt_new_defaults_once() {
    let cases = [
        (r#"{"detection": {"early_switch": true}}"#, true),
        (r#"{"schema_version": "x"}"#, true),
        (r#"{"schema_version": 1}"#, true),
        (r#"{"schema_version": 6, "detection": {"early_switch": false}}"#, true),
        (r#"{"schema_version": 7, "detection": {"early_switch": false}}"#, false),
        (r#"{"schema_version": 5, "detection": {"early_switch": true}}"#, true),
        (r#"{"schema_version": 7, "detection": {"pause_delay_seconds": 1.5}}"#, true),
        (r#"{"schema_version": 7, "detection": {"pause_delay_seconds": 0.5}}"#, true),
        (r#"{"schema_version": 99, "detection": {"early_switch": false}}"#, false),
    ];
    let directory = folder("settings");
    for (number, (file, early)) in cases.iter().enumerate() {
        let path = directory.join(format!("{number}.json"));
        std::fs::write(&path, file).expect("the file");
        let store = SettingsStore::new(Some(path));
        store.load();
        assert_eq!(store.read(DETECTION_EARLY_SWITCH), Some(*early), "{file}");
        store.save().expect("saved");
    }
    let path = directory.join("pause.json");
    std::fs::write(&path, r#"{"schema_version": 7, "detection": {"pause_delay_seconds": 1.5}}"#).expect("the file");
    let store = SettingsStore::new(Some(path));
    store.load();
    assert_ne!(store.read(DETECTION_PAUSE_DELAY_SECONDS), Some(1.5));
}

#[test]
fn settings_in_a_folder_instead_of_a_file_are_set_aside() {
    let path = folder("settings-dir");
    let store = SettingsStore::new(Some(path));
    store.load();
    assert_eq!(store.read(DETECTION_EARLY_SWITCH), Some(true));
}

#[test]
fn a_change_of_a_setting_for_the_log() {
    let directory = folder("diagnostics");
    let store = SettingsStore::new(Some(directory.join("settings.json")));
    store.load();
    assert_eq!(change_record(&store, &SettingChange::Reset).expect("a snapshot").operation, Operation::Snapshot);
    assert_eq!(change_record(&store, &SettingChange::SchemaVersion(1)), None);
    assert_eq!(change_record(&store, &SettingChange::DetectionEarlySwitch(true)).expect("known").operation, Operation::Reset);
    assert_eq!(change_record(&store, &SettingChange::DetectionEarlySwitch(false)).expect("known").value, Some(LoggedValue::Flag(false)));
    let cut = change_record(&store, &SettingChange::AppearanceTheme("x".repeat(10_000))).expect("known").value.expect("a value");
    let LoggedValue::Text(cut) = cut else { panic!("a text") };
    assert!(cut.ends_with("...") && cut.len() < 10_000);
    let words = change_record(&store, &SettingChange::ExclusionsWords(vec!["a".to_owned(), "b".to_owned()])).expect("known");
    assert_eq!(words.value, Some(LoggedValue::Collection { r#type: "list", items: 2 }));
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
        (r#"{"rules": [1]}"#, 0),
        (r#"{"rules": {"nocolon": {"target_group": 1, "confirmations": 9}}}"#, 0),
        (r#"{"rules": {"x:word": {"target_group": 1, "confirmations": 9}}}"#, 0),
        (r#"{"rules": {"0:word": 1}}"#, 0),
        (r#"{"rules": {"0:word": {}}}"#, 0),
        (r#"{"rules": {"0:word": {"target_group": 1}}}"#, 0),
        (r#"{"rules": {"0:word": {"target_group": 1, "confirmations": "x"}}}"#, 0),
        (r#"{"rules": {"0:word": {"target_group": 1, "confirmations": 9}}}"#, 1),
        (r#"{"rules": {"0:": {"target_group": 1, "confirmations": 9}}}"#, 0),
        (r#"{"rules": {"0:word": {"target_group": 0, "confirmations": 9}}}"#, 0),
        (r#"{"rejections": []}"#, 0),
        (r#"{"rejections": {"nocolon": [1]}}"#, 0),
        (r#"{"rejections": {"0:word": 1}}"#, 0),
        (r#"{"rejections": {"0:word": ["x", 1]}}"#, 1),
        (r#"{"rules": {"0:word": {"target_group": 1, "confirmations": 9}}, "rejections": {"0:word": [1]}}"#, 1),
    ];
    for (payload, count) in cases {
        assert_eq!(migrate_legacy(&legacy_file(payload), 3).expect("no error").len(), count, "{payload}");
    }
}

#[test]
fn a_rule_of_the_file_with_an_odd_flag_is_no_rule() {
    let directory = folder("flag");
    let path = directory.join("rules.json");
    let text = r#"{"schema_version": 2, "rules": [
        {"pattern": "word", "source_group": 0, "target_group": 1, "action": "convert", "match": "exact", "case_sensitive": true},
        {"pattern": "other", "source_group": 0, "target_group": 1, "action": "convert", "match": "exact", "case_sensitive": 1}
    ]}"#;
    std::fs::write(&path, text).expect("the file");
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
        r#"{"schema_version": 1}"#,
        r#"{"schema_version": 1, "salt": "s"}"#,
        r#"{"schema_version": 1, "salt": "s", "personal": []}"#,
        r#"{"schema_version": 1, "salt": 1, "personal": [], "counts": {}}"#,
        r#"{"schema_version": 1, "salt": "s", "personal": 1, "counts": {}}"#,
        r#"{"schema_version": 1, "salt": "s", "personal": [], "counts": 1}"#,
    ];
    for (number, payload) in parts.iter().enumerate() {
        let path = directory.join(format!("{number}.json"));
        std::fs::write(&path, payload).expect("the file");
        let words = PersonalWords::always_persisting(Some(path));
        assert!(words.unreadable().is_some(), "{payload}");
    }
}

#[test]
fn a_personal_file_with_doubles_and_odd_counts() {
    let directory = folder("doubles");
    let path = directory.join("words.json");
    let payload = r#"{"schema_version": 1, "salt": "s", "personal": ["a", "a", 7, "b"], "counts": {"a": 1, "b": 0, "c": "x", "d": -1, "e": 2}}"#;
    std::fs::write(&path, payload).expect("the file");
    let words = PersonalWords::always_persisting(Some(path.clone()));
    assert_eq!(words.unreadable(), None);
    assert_eq!(words.len(), 2);
    words.save();
    let saved = std::fs::read_to_string(&path).expect("saved").replace("\r\n", "\n");
    assert!(saved.ends_with("\"counts\": {\n    \"a\": 1,\n    \"e\": 2\n  }\n}\n"), "{saved}");
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
