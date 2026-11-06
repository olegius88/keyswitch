//! The small values of a backend: what a key event says, how a field is cut for the log, the names of the reasons.

use ks_backend::{BackendError, FieldContext, FieldRole, InputDelayReport, KeyEvent, RefusalReason};

#[test]
fn a_key_event_says_what_is_held() {
    for bit in 0..8 {
        let event = KeyEvent { state: 1 << bit, ..KeyEvent::default() };
        let held = [event.shift(), event.caps_lock(), event.control(), event.alt(), event.super_key()];
        assert!(held.iter().filter(|flag| **flag).count() <= 1, "state bit {bit}");
    }
    let shown = KeyEvent { characters: vec!["a".to_owned(), "ф".to_owned()], ..KeyEvent::default() };
    assert_eq!((shown.character_for(0), shown.character_for(1), shown.character_for(2), shown.character_for(-1)), ("a", "ф", "", ""));
    let all = KeyEvent { state: u32::MAX, ..KeyEvent::default() };
    assert!(all.shift() && all.caps_lock() && all.control() && all.alt() && all.super_key());
}

#[test]
fn an_error_prints_its_message() {
    assert_eq!(BackendError::Failed("no display".to_owned()).to_string(), "no display");
}

#[test]
fn every_field_role_has_its_name() {
    let names = [
        (FieldRole::Unknown, "unknown"),
        (FieldRole::Text, "text"),
        (FieldRole::Code, "code"),
        (FieldRole::Search, "search"),
        (FieldRole::Terminal, "terminal"),
        (FieldRole::Password, "password"),
    ];
    for (role, name) in names {
        assert_eq!(role.as_str(), name);
    }
}

#[test]
fn a_private_field_gives_no_text() {
    let mut field = FieldContext::new("app", "id");
    field.before = "before".to_owned();
    field.after = "after".to_owned();
    assert_eq!((field.bounded().before.as_str(), field.bounded().after.as_str()), ("before", "after"));
    field.sensitive = true;
    assert_eq!((field.bounded().before.as_str(), field.bounded().after.as_str()), ("", ""));
    field.sensitive = false;
    field.role = FieldRole::Password;
    assert_eq!((field.bounded().before.as_str(), field.bounded().after.as_str()), ("", ""));
}

#[test]
fn a_delay_is_a_stall_when_keys_or_callbacks_ran_late() {
    assert!(!InputDelayReport::default().stalled());
    assert!(InputDelayReport { late_keys: 1, ..InputDelayReport::default() }.stalled());
    assert!(InputDelayReport { slow_callbacks: 1, ..InputDelayReport::default() }.stalled());
}

#[test]
fn every_refusal_has_its_name() {
    let names = [
        (RefusalReason::Disabled, "disabled"),
        (RefusalReason::OwnWindow, "own_window"),
        (RefusalReason::ExcludedApplication, "excluded_application"),
        (RefusalReason::SensitiveField, "sensitive_field"),
        (RefusalReason::Unsupported, "unsupported"),
        (RefusalReason::NoSelection, "no_selection"),
        (RefusalReason::MultipleSelection, "multiple_selection"),
        (RefusalReason::ReadOnly, "read_only"),
        (RefusalReason::TooLong, "too_long"),
        (RefusalReason::LineBreaks, "line_breaks"),
        (RefusalReason::Untypeable, "untypeable"),
        (RefusalReason::Terminal, "terminal"),
        (RefusalReason::ClipboardBusy, "clipboard_busy"),
        (RefusalReason::ClipboardUnrestorable, "clipboard_unrestorable"),
        (RefusalReason::NotSettable, "not_settable"),
        (RefusalReason::FocusChanged, "focus_changed"),
        (RefusalReason::ReleaseTimeout, "release_timeout"),
        (RefusalReason::NoLayout, "no_layout"),
        (RefusalReason::EmptyResult, "empty_result"),
        (RefusalReason::NoCaret, "no_caret"),
    ];
    for (reason, name) in names {
        assert_eq!(reason.as_str(), name);
    }
}
