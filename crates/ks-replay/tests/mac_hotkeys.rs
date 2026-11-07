//! The conversion hotkey of a Mac, Control+Option+K: a Mac keyboard has no Pause, and the macOS backend names no such key.
//! The engine converts on the configured key with the events a Mac gives (the plain letter's name, Control and Option in
//! the state), its double press offers a rule, and the backend keeps the key from the window. A program of its own: the
//! editors of this one keep the program's hotkeys from the window, as a Mac does.
#![cfg(any(not(debug_assertions), coverage))]

use std::sync::atomic::Ordering;

use ks_backend::constants::{ALT_MASK, CONTROL_MASK, SHIFT_MASK};
use ks_backend::{KeyDisposition, KeyEvent};
use ks_config::config::default_settings;
use ks_engine::engine::{EngineSnapshot, Shared};
use ks_replay::harness::{Bundle, KEYS, SWALLOWS, Session};

const MAC_CONTROL: u32 = 59;
const MAC_OPTION: u32 = 58;
const MAC_K: u32 = 40;

fn modifier(session: &Session, name: &str, keycode: u32, pressed: bool, state: u32) -> KeyEvent {
    KeyEvent {
        pressed,
        keycode,
        key_name: name.to_owned(),
        characters: vec![String::new(), String::new()],
        group: session.backend.group(),
        state,
        ..KeyEvent::default()
    }
}

/// K under Control and Option, as the macOS backend names it: by the letter with no modifier, typing a control code.
fn k(session: &Session, pressed: bool) -> KeyEvent {
    KeyEvent {
        pressed,
        keycode: MAC_K,
        key_name: "k".to_owned(),
        character: "\u{b}".to_owned(),
        characters: vec!["\u{b}".to_owned(), "\u{b}".to_owned()],
        group: session.backend.group(),
        state: CONTROL_MASK | ALT_MASK,
        ..KeyEvent::default()
    }
}

fn type_keys(session: &mut Session, english: &str) {
    for character in english.chars() {
        let key = KEYS.iter().find(|key| key.characters[0] == character).expect("a key of the US layout");
        let group = session.backend.group();
        let strings = key.strings();
        session.tap(KeyEvent {
            pressed: true,
            keycode: key.keycode,
            key_name: character.to_string(),
            character: strings[usize::try_from(group).expect("a layout of the two")].clone(),
            characters: strings,
            group,
            state: if key.shift { SHIFT_MASK } else { 0 },
            ..KeyEvent::default()
        });
    }
}

#[test]
fn control_option_k_converts_and_its_double_press_offers_a_rule() {
    SWALLOWS.store(true, Ordering::Relaxed);
    let mac = default_settings(Some("darwin"));
    let mut bundle = Bundle::load(None);
    let mut session = bundle.session("TextEdit", 0, Vec::new());
    session.core.settings.set("hotkeys.convert_last", mac["hotkeys"]["convert_last"].clone(), false).expect("in memory");
    session.core.settings.set("detection.early_switch", serde_json::json!(false), false).expect("in memory");
    // The window never gets the K: the backend keeps the program's own hotkey.
    let shared = Shared::new(session.core.settings.clone(), session.core.backend.clone(), session.core.clock.clone(), 2, EngineSnapshot::default());
    assert_eq!(shared.consumes_key(&k(&session, true)), KeyDisposition::Consume);
    // One press converts the word typed.
    type_keys(&mut session, "hello");
    let control = modifier(&session, "Control_L", MAC_CONTROL, true, CONTROL_MASK);
    session.handle(control);
    let option = modifier(&session, "Alt_L", MAC_OPTION, true, CONTROL_MASK | ALT_MASK);
    session.handle(option);
    for pressed in [true, false] {
        let key = k(&session, pressed);
        session.handle(key);
    }
    // The second press at once offers a rule for the word.
    session.advance(0.1);
    for pressed in [true, false] {
        let key = k(&session, pressed);
        session.handle(key);
    }
    let option = modifier(&session, "Alt_L", MAC_OPTION, false, CONTROL_MASK);
    session.handle(option);
    let control = modifier(&session, "Control_L", MAC_CONTROL, false, 0);
    session.handle(control);
    session.idle(0.05);
    assert_eq!(session.backend.text().iter().collect::<String>(), "руддщ");
    assert_eq!(session.backend.group(), 1);
    assert!(session.core.shared.learning_prompt().is_some_and(|prompt| prompt.original == "hello"));
    bundle.give_back(session);
    bundle.clean();
}
