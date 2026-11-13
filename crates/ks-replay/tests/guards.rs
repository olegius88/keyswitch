//! Grids over the checks a word passes at its boundary: each word, each thing that may stand in the way, each boundary.
//! What must hold: no failure, no stop, and the text stays where the caret can reach it.
#![cfg(any(not(debug_assertions), coverage))]

use std::sync::Arc;

use ks_backend::KeyEvent;
use ks_backend::constants::SHIFT_MASK;
use ks_engine::engine::InsertionPoint;
use ks_replay::harness::{Bundle, CaretReader, KEYS, PAUSE_KEYCODE, Session};
use serde_json::json;

const OPENINGS: &[&str] = &["", "начало ", "start "];
const WORDS: &[&str] = &[
    "ghbdtn", "ghbdtn,", "ghbdtn.", "vbh;", "tot", "ещ", "hello", "hello,", "привет", "привет,", "руддщ", "lf", "да", "z", "я", "ntcn/", "'nb/", "сдела.",
    "t`", "a.b", "x-y", "кто-то", "rnj-nj", "/usr/bin", "HEllo", "GHbdtn", "ПРивет", "пРИВЕТ", "123", "v67", "@ghbdtn", "ghb", "cj,thb", "ye;yj", "[jhjij",
    "хорош", "tot,", "yt", "t", "'",
];
const BOUNDARIES: &[&str] = &[" ", ",", ".", "Return", "Tab", "deferred Return", "deferred Tab", "deferred space", "idle", "pause"];

const STATES: usize = 18;

/// The engine writes its technical log: the lines that say why a correction was not made run only then.
fn log_technically(session: &mut Session) {
    session.core.settings.set("diagnostics.technical_logging", json!(true), false).expect("in memory");
}

fn press(session: &mut Session, character: char) {
    press_in(session, character, 0);
}

/// A press of the key of `character` that the hook saw on its way to `window`; 0 for a hook that names none.
fn press_in(session: &mut Session, character: char, window: i64) {
    let Some((key, group)) = KEYS.iter().find_map(|key| Some((key, key.characters.iter().position(|typed| *typed == character)?))) else {
        return;
    };
    if usize::try_from(session.backend.group()).ok() != Some(group) {
        session.select_layout(i32::try_from(group).expect("two layouts"));
    }
    session.tap(KeyEvent {
        pressed: true,
        keycode: key.keycode,
        key_name: character.to_string(),
        character: character.to_string(),
        characters: key.strings(),
        group: session.backend.group(),
        state: if key.shift { SHIFT_MASK } else { 0 },
        window,
        ..KeyEvent::default()
    });
}

fn type_text(session: &mut Session, text: &str) {
    for character in text.chars() {
        press(session, character);
    }
}

fn end_word(session: &mut Session, boundary: &str) {
    let group = session.backend.group();
    let named = |name: &str, deferred: bool| KeyEvent { pressed: true, key_name: name.to_owned(), characters: vec![String::new(), String::new()], deferred, group, ..KeyEvent::default() };
    match boundary {
        "Return" | "Tab" => session.tap(named(boundary, false)),
        "deferred Return" => session.handle(named("Return", true)),
        "deferred Tab" => session.handle(named("Tab", true)),
        "deferred space" => session.handle(KeyEvent { character: " ".to_owned(), characters: vec![" ".to_owned(), " ".to_owned()], ..named("space", true) }),
        "idle" => session.idle(1.0),
        "pause" => session.tap(KeyEvent { keycode: PAUSE_KEYCODE, ..named("Pause", false) }),
        other => type_text(session, other),
    }
}

/// One thing that may stand in the way of a word.
fn disturb(session: &mut Session, state: usize) {
    let now = session.core.monotonic();
    let core = &mut session.core;
    match state {
        1 => core.insertion = Some(InsertionPoint { head: "ab".to_owned(), tail: String::new() }),
        2 => core.insertion = Some(InsertionPoint { head: String::new(), tail: "ab".to_owned() }),
        3 => {
            core.early_switch_origin = Some(0);
            core.early_switch_at = Some(now);
        }
        4 => {
            core.early_switch_origin = Some(1);
            core.early_switch_at = Some(now);
        }
        5 => core.pending.clone_from(&core.last_committed),
        6 => core.untracked_token = true,
        7 => core.caret_moved = true,
        8 => core.position_unknown = true,
        9 => core.foreign_input_active = true,
        10 => core.manual_layout_group = Some(0),
        11 => core.manual_layout_group = Some(1),
        12 => core.settings.set("exclusions.applications", json!(["firefox"]), false).expect("in memory"),
        13 => core.settings.set("detection.context_policy", json!("off"), false).expect("in memory"),
        14 => core.settings.set("detection.correct_on_space", json!(false), false).expect("in memory"),
        15 => core.settings.set("detection.context_read_field", json!(true), false).expect("in memory"),
        16 => core.last_committed_stale = true,
        17 => core.settings.set("detection.correct_typos", json!(true), false).expect("in memory"),
        _ => {}
    }
}

#[test]
fn a_word_meets_everything_at_its_boundary() {
    let mut bundle = Bundle::load(None);
    let mut tried = 0;
    for (index, word) in WORDS.iter().enumerate() {
        for state in 0..STATES {
            for (place, boundary) in BOUNDARIES.iter().enumerate() {
                let opening = OPENINGS[(index + state + place) % OPENINGS.len()];
                let mut session = bundle.session("firefox", 0, opening.chars().collect());
                session.recovers = true;
                let backend = session.backend.clone();
                session.core.context_policy.reader = Some(Arc::new(CaretReader { backend }));
                log_technically(&mut session);
                // A word typed before it, so that the word has something to go with.
                if (index + place).is_multiple_of(2) {
                    type_text(&mut session, "ytn ");
                }
                if state % 2 == 0 {
                    disturb(&mut session, state);
                }
                type_text(&mut session, word);
                if state % 2 == 1 || state == 0 {
                    disturb(&mut session, state);
                }
                end_word(&mut session, boundary);
                type_text(&mut session, "ghbdtn ");
                session.idle(1.0);
                tried += 1;
                assert!(session.backend.caret() <= session.backend.text().len());
                bundle.give_back(session);
            }
        }
    }
    println!("{tried} tried");
    bundle.clean();
}

/// What stands in the way of a word that waits for its neighbour, one thing at a time, at a pause or at an action key.
fn hinder_waiting(session: &mut Session, way: usize) {
    let core = &mut session.core;
    let id = core.context_waiting.as_ref().map_or(0, |waiting| waiting.diagnostic_id);
    match way {
        1 => core.last_committed_stale = true,
        2 => {
            if let Some(waiting) = &mut core.context_waiting {
                waiting.window = -1;
            }
        }
        3 => core.settings.set("detection.context_policy", json!("off"), false).expect("in memory"),
        4 => core.settings.set("enabled", json!(false), false).expect("in memory"),
        5 => core.settings.set("detection.correct_on_pause", json!(false), false).expect("in memory"),
        6 => {
            core.pressed.insert(200);
        }
        7 => {
            core.modifier_keycodes.insert(50);
        }
        8 => {
            core.pressed.insert(200);
            core.wait_deferral_logged = id;
        }
        9 | 10 => {
            if let Some(waiting) = &mut core.context_waiting {
                waiting.settles_on_pause = way == 9;
            }
        }
        11 => {
            if let Some(waiting) = &mut core.context_waiting {
                let mut plan = (*waiting.plan).clone();
                "another".clone_into(&mut plan.application);
                waiting.plan = Arc::new(plan);
                waiting.settles_on_pause = true;
            }
        }
        12 => {
            core.settings.set("exclusions.applications", json!(["firefox"]), false).expect("in memory");
            if let Some(waiting) = &mut core.context_waiting {
                waiting.settles_on_pause = true;
            }
        }
        13 => core.pending.clone_from(&core.last_committed),
        14 => {
            if let Some(waiting) = &mut core.context_waiting {
                waiting.converted = true;
            }
        }
        15 => core.last_word_input_at = None,
        16 => core.settings.set("detection.correct_on_enter", json!(false), false).expect("in memory"),
        _ => {}
    }
}

#[test]
fn a_waiting_word_settles_only_with_nothing_in_its_way() {
    let mut bundle = Bundle::load(None);
    let mut waited = 0;
    for first in ["tot", "ещ", "yt", "lf", "да", "z", "я", "x", "ч", "gj", "по", "rfr", "как", "qr", "йк", "d", "в", "c", "с"] {
        for way in 0..17 {
            for at in ["pause", "Return", "deferred Return", "pause then word", "word", "deferred word"] {
                let mut session = bundle.session("firefox", 0, "начало ".chars().collect());
                session.recovers = true;
                let backend = session.backend.clone();
                session.core.context_policy.reader = Some(Arc::new(CaretReader { backend }));
                log_technically(&mut session);
                type_text(&mut session, first);
                type_text(&mut session, " ");
                if session.core.context_waiting.is_some() {
                    waited += 1;
                }
                hinder_waiting(&mut session, way);
                match at {
                    "pause" => {
                        session.advance(5.0);
                        let _ = session.core.run_timers();
                    }
                    "pause then word" => {
                        session.advance(5.0);
                        let _ = session.core.run_timers();
                        session.core.pressed.clear();
                        session.core.modifier_keycodes.clear();
                        type_text(&mut session, "ghbdtn ");
                    }
                    "word" => type_text(&mut session, "ghbdtn "),
                    "deferred word" => {
                        type_text(&mut session, "ghbdtn");
                        end_word(&mut session, "deferred space");
                    }
                    other => end_word(&mut session, other),
                }
                assert!(session.backend.caret() <= session.backend.text().len());
                session.core.pressed.clear();
                session.core.modifier_keycodes.clear();
                bundle.give_back(session);
            }
        }
    }
    assert!(waited > 0);
    bundle.clean();
}

/// A pause after a word with signs, quotes or a head around it, with a word before it that waits.
#[test]
fn a_pause_takes_along_what_stands_before_the_word() {
    let mut bundle = Bundle::load(None);
    for first in ["", "tot ", "ещ ", "yt ", "z ", "я ", "lf ", "ghbdtn "] {
        for word in ["ghbdtn", "ghbdtn,", "ghbdtn.", "ntcn/", "\"ghbdtn\"", "\"ghbdtn", "@ghbdtn", "@ghbdtn\"", "(ghbdtn)", "ghbdtn!", "rfr", "vfvf", "'nb/", "vfvf,"] {
            for application in ["firefox", "telegram-desktop"] {
                for end in ["idle", "pause"] {
                    let mut session = bundle.session(application, 0, Vec::new());
                    session.recovers = true;
                    let backend = session.backend.clone();
                    session.core.context_policy.reader = Some(Arc::new(CaretReader { backend }));
                    type_text(&mut session, first);
                    type_text(&mut session, word);
                    end_word(&mut session, end);
                    session.idle(1.0);
                    assert!(session.backend.caret() <= session.backend.text().len());
                    bundle.give_back(session);
                }
            }
        }
    }
    bundle.clean();
}

/// Words after words, at a space and at a pause, the second one also as if the early switch had converted it: the questions
/// about a pair come up only for some pairs, and only the pairs tell which.
#[test]
fn pairs_of_words_ask_each_other_about_their_layout() {
    const FIRST: &[&str] = &[
        "z", "я", "x", "ч", "d", "в", "r", "к", "f", "а", "b", "и", "c", "с", "tot", "ещ", "ещу", "lf", "да", "yt", "не", "yf", "на", "ytn", "нет", "gj", "по", "dct", "все",
        "rfr", "как", "qr", "йк", "v67", "м67", "2ю", "webjs", "htmx", "git", "npm", "тзь", "ntcn", "тест", "ghbdtn", "привет", "the", "еру", "and", "фтв", "м", "ь", "н", "y", "T",
        "Z", "Я", "ТОТ", "ьщтпщ", "vjq", "мой", "ns", "ты", "jy", "он", "jyf", "она",
    ];
    const NEXT: &[&str] = &["ghbdtn", "привет", "hello", "руддщ", "d", "в", "rfr", "как", "+", "-", "=", "2", "vbh", "мир", "git", "пше", "npm", "the", "еру", "ytn", "нет", "lj,hsq", "ghbdtn,"];
    let mut bundle = Bundle::load(None);
    for start in ["", "начало ", "git "] {
        for first in FIRST {
            for next in NEXT {
                for way in 0..3 {
                    let mut session = bundle.session("firefox", 0, start.chars().collect());
                    session.recovers = true;
                    let backend = session.backend.clone();
                    session.core.context_policy.reader = Some(Arc::new(CaretReader { backend }));
                    type_text(&mut session, first);
                    type_text(&mut session, " ");
                    type_text(&mut session, next);
                    match way {
                        0 => type_text(&mut session, " "),
                        1 => session.idle(1.0),
                        _ => {
                            session.core.early_switch_origin = Some(1 - session.core.source_group.clamp(0, 1));
                            session.core.early_switch_at = Some(session.core.monotonic());
                            if start.is_empty() {
                                session.idle(1.0);
                            } else {
                                type_text(&mut session, " ");
                            }
                        }
                    }
                    type_text(&mut session, "d ");
                    assert!(session.backend.caret() <= session.backend.text().len());
                    bundle.give_back(session);
                }
            }
        }
    }
    bundle.clean();
}

/// A quote typed in the second layout is `@` of the first where the program reads it as a mention, and a second one closes it.
#[test]
fn a_quote_closed_after_a_word_is_a_quote_at_a_pause() {
    let quote_key = KEYS.iter().find(|key| key.characters[1] == '"').expect("the key of the quote");
    let mut bundle = Bundle::load(None);
    for application in ["firefox", "telegram-desktop", "slack"] {
        for word in ["ощрт", "руддщ", "john"] {
            for closes in [false, true] {
                let mut session = bundle.session(application, 1, Vec::new());
                session.recovers = true;
                let quote = KeyEvent {
                    pressed: true,
                    keycode: quote_key.keycode,
                    key_name: "quotedbl".to_owned(),
                    character: "\"".to_owned(),
                    characters: quote_key.strings(),
                    group: 1,
                    state: SHIFT_MASK,
                    ..KeyEvent::default()
                };
                session.tap(quote.clone());
                type_text(&mut session, word);
                if closes {
                    session.select_layout(1);
                    session.tap(quote);
                }
                session.idle(1.0);
                assert!(session.backend.caret() <= session.backend.text().len());
                bundle.give_back(session);
            }
        }
    }
    bundle.clean();
}

/// A correction erases no key the hook saw on its way to another window than the one in front. KeySwitch's own window in
/// front is no change of the window the user works in, so the word typed there is still the engine's: asked to convert
/// it, the engine typed the conversion into its own window. While the backend names no window in front, a key is a key
/// of the window in front whatever its hook named.
#[test]
fn a_word_of_another_window_is_not_corrected_in_the_window_in_front() {
    let mut bundle = Bundle::load(None);
    for own in [false, true] {
        let mut session = bundle.session("app", 0, Vec::new());
        log_technically(&mut session);
        for character in "ytn".chars() {
            press_in(&mut session, character, 1);
        }
        assert!(session.backend.corrections().is_empty(), "three letters are not converted as they are typed");
        session.backend.set_own(own);
        session.backend.focus(if own { 2 } else { 1 }, "app");
        end_word(&mut session, "pause");
        session.idle(1.0);
        assert_eq!(session.backend.corrections().len(), usize::from(!own), "KeySwitch's own window in front: {own}");
        bundle.give_back(session);
    }

    let mut session = bundle.session("app", 0, Vec::new());
    session.backend.focus(0, "app");
    for character in "ytn ".chars() {
        press_in(&mut session, character, 7);
    }
    assert_eq!(session.backend.corrections().len(), 1);
    bundle.give_back(session);

    // A backend that names the window for a key and a field of it for the engine (Windows): a word typed into the next
    // field of one window is a word of the window in front, and a word of another window is not.
    for (window, corrected) in [(7, 1), (8, 0)] {
        let mut session = bundle.session("app", 0, Vec::new());
        session.backend.set_key_window(Some(7));
        session.backend.focus(70, "app");
        for character in "ytn ".chars() {
            press_in(&mut session, character, window);
        }
        assert_eq!(session.backend.corrections().len(), corrected, "typed into window {window}, window 7 in front");
        bundle.give_back(session);
    }
    bundle.clean();
}

/// The layout that comes with another window is that window's own, not one the user picked by hand: the word typed there
/// is judged like any other. A system keeps a layout per window and reports it with the first key pressed there, and the
/// engine notices the move to that window at that same key; asked a second time, it said the window had not changed,
/// booked the layout as a manual choice and left the word typed there as typed.
#[test]
fn the_layout_of_another_window_is_no_choice_of_the_users() {
    let mut bundle = Bundle::load(None);
    let mut session = bundle.session("app", 0, Vec::new());
    session.backend.focus(1, "app");
    session.select_layout(1);
    // Another window, with a layout of its own, and no look at the desktop between it and the first key pressed there.
    session.backend.focus(2, "app");
    session.backend.set_group(0);
    press_in(&mut session, 'g', 0);
    assert_eq!(session.core.manual_layout_group, None, "the layout of the window the user moved to");
    // The layout the user picks while the window stays in front protects the word typed in it.
    session.backend.set_group(1);
    press_in(&mut session, 'п', 0);
    assert_eq!(session.core.manual_layout_group, Some(1));
    bundle.give_back(session);
    bundle.clean();
}
