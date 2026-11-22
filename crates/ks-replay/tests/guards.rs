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

/// A press of the key that types `character` in the layout the window is in: no layout is picked for it.
fn press_as_laid_out(session: &mut Session, character: char) {
    let group = session.backend.group();
    let key = KEYS.iter().find(|key| usize::try_from(group).ok().and_then(|place| key.characters.get(place)) == Some(&character)).expect("a key of the layout");
    session.tap(KeyEvent { pressed: true, keycode: key.keycode, key_name: character.to_string(), character: character.to_string(), characters: key.strings(), group, ..KeyEvent::default() });
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

/// A word that stands as a hotkey of the user's left it is theirs when they go back into it. `ру` after a Russian word
/// became `he`, Pause brought `ру` back, and a full stop typed over the space after it made it `he/` again: Backspace
/// took the space away, the word was open again and was judged at the full stop as a word just typed.
#[test]
fn a_word_a_hotkey_left_as_it_stands_is_not_converted_when_the_user_goes_back_into_it() {
    let mut bundle = Bundle::load(None);
    for (undo, sign) in [(false, '.'), (true, ','), (false, ' ')] {
        let mut session = bundle.session("app", 0, Vec::new());
        type_text(&mut session, "ghbdtn ");
        session.idle(1.0);
        assert_eq!(session.backend.text().iter().collect::<String>(), "привет ");
        let hotkey = if undo { "undo" } else { "Pause" };
        if undo {
            session.core.schedule_undo_hotkey(i64::from(PAUSE_KEYCODE));
            session.handle(KeyEvent { pressed: false, keycode: PAUSE_KEYCODE, key_name: "Pause".to_owned(), ..KeyEvent::default() });
        } else {
            session.tap_named(hotkey);
        }
        session.idle(1.0);
        assert_eq!(session.backend.text().iter().collect::<String>(), "ghbdtn ", "{hotkey} puts the keys back as typed");
        session.tap_named("BackSpace");
        assert_eq!(session.core.manual_layout_group, Some(0), "the word is open again, and it is the user's");
        press_as_laid_out(&mut session, sign);
        session.idle(1.0);
        assert_eq!(session.backend.text().iter().collect::<String>(), format!("ghbdtn{sign}"), "after {hotkey}, Backspace and {sign:?}");
        // The next word is judged as any other.
        type_text(&mut session, if sign == ' ' { "ghbdtn " } else { " ghbdtn " });
        session.idle(1.0);
        assert!(session.backend.text().iter().collect::<String>().ends_with(" привет "), "the word after it, after {hotkey}, Backspace and {sign:?}");
        bundle.give_back(session);
    }
    // The hotkey left another word, not the one the user goes back into: that one is judged as any word.
    let mut session = bundle.session("app", 0, Vec::new());
    type_text(&mut session, "ghbdtn ");
    session.idle(1.0);
    session.tap_named("Pause");
    session.idle(1.0);
    type_text(&mut session, "hello ");
    session.idle(1.0);
    session.tap_named("BackSpace");
    assert_eq!(session.core.manual_layout_group, None, "a word no hotkey touched");
    bundle.give_back(session);
    bundle.clean();
}


/// A pause after the first letters of a word is a pause to think: the word is decided at its boundary. `ин` after a
/// Russian word became `by` at the pause, in the middle of `инструкция`, and the layout changed under the rest of it;
/// the letters were judged there because their other reading is a word of English prose.
#[test]
fn a_pause_after_the_first_letters_of_a_word_decides_nothing() {
    let mut bundle = Bundle::load(None);
    for (before, start, rest) in [("в ", "ин", "струкция "), ("идет ", "уд", "ачно "), ("вот ", "рез", "ультат ")] {
        let mut session = bundle.session("app", 1, Vec::new());
        for character in before.chars().chain(start.chars()) {
            press_as_laid_out(&mut session, character);
        }
        session.idle(1.0);
        assert_eq!(session.backend.text().iter().collect::<String>(), format!("{before}{start}"), "at the pause");
        for character in rest.chars() {
            press_as_laid_out(&mut session, character);
        }
        session.idle(1.0);
        assert_eq!(session.backend.text().iter().collect::<String>(), format!("{before}{start}{rest}"), "the word typed on");
        // What tells a start of a word from a word: how many letters, of which layout, after what.
        let core = &session.core;
        assert!(core.word_begun(start, 1, "x", 0, "app").expect("no failure"), "{start}");
        assert!(!core.word_begun("инст", 1, "x", 0, "app").expect("no failure"), "four letters are judged at a pause");
        assert!(!core.word_begun("и", 1, "x", 0, "app").expect("no failure"), "and one letter");
        assert!(!core.word_begun("и1", 1, "x", 0, "app").expect("no failure"), "letters only");
        assert!(!core.word_begun("ъы", 1, "x", 0, "app").expect("no failure"), "no word begins so");
        assert!(!core.word_begun("by", 0, "ин", 1, "app").expect("no failure"), "the word before was left in the other layout");
        assert!(!core.word_begun("рез", 1, "htp", 0, "another").expect("no failure"), "three letters with nothing typed there");
        bundle.give_back(session);
    }
    // Few words begin with `ес` or `оши`, and those that do are met often in prose (`есть`, `если`, `ошибка`): such
    // letters wait too. They became `tc` and `jib` at the pause, and the rest of the word was typed in the layout that
    // left.
    for (before, start, other, rest) in [("вот ", "ес", "tc", "ли "), ("если ", "оши", "jib", "бка ")] {
        let mut session = bundle.session("app", 1, Vec::new());
        for character in before.chars().chain(start.chars()) {
            press_as_laid_out(&mut session, character);
        }
        session.idle(1.0);
        assert_eq!(session.backend.text().iter().collect::<String>(), format!("{before}{start}"), "at the pause");
        for character in rest.chars() {
            press_as_laid_out(&mut session, character);
        }
        session.idle(1.0);
        assert_eq!(session.backend.text().iter().collect::<String>(), format!("{before}{start}{rest}"), "the word typed on");
        let core = &session.core;
        assert!(core.word_begun(start, 1, other, 0, "app").expect("no failure"), "{start}");
        assert!(!core.word_begun("афи", 1, "fab", 0, "app").expect("no failure"), "few words, and prose seldom has them");
        assert!(!core.word_begun("что", 1, "xnj", 0, "app").expect("no failure"), "a word itself, though it begins `чтобы`");
        assert!(!core.word_begun("fab", 2, "афи", 1, "app").expect("no failure"), "few words of a layout with no prose");
        bundle.give_back(session);
    }
    // With no word before them two letters wait only when their other reading is no word of English prose (`иг` for `bu`
    // waits, `ша` for `if` does not), and three letters are judged as before: two letters and a wait are as often a whole
    // word, an address typed into a browser.
    let session = bundle.session("app", 1, Vec::new());
    assert!(session.core.word_begun("иг", 1, "bu", 0, "app").expect("no failure"));
    assert!(!session.core.word_begun("ша", 1, "if", 0, "app").expect("no failure"));
    assert!(!session.core.word_begun("рез", 1, "htp", 0, "app").expect("no failure"));
    assert!(!session.core.word_begun("иг", 1, "bu", 2, "app").expect("no failure"), "a layout with no prose");
    bundle.give_back(session);
    bundle.clean();
}

/// A word left as typed only because its layout was picked by hand is told to the windows when the model would have
/// converted it, and only then: `gfgefcs` typed right after switching to English is `папуасы` to the model, and the user
/// testing the program that way sees nothing happen. `hello` there is what the pick was for, and `gfgefcs` with no pick
/// is converted.
#[test]
fn a_word_kept_for_a_layout_picked_by_hand_is_told_when_the_model_would_convert_it() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let mut bundle = Bundle::load(None);
    for (picked, typed, told, shown) in [(true, "gfgefcs ", 1, "gfgefcs "), (true, "hello ", 0, "hello "), (false, "gfgefcs ", 0, "папуасы ")] {
        let mut session = bundle.session("app", if picked { 1 } else { 0 }, Vec::new());
        let count = Arc::new(AtomicUsize::new(0));
        let counter = count.clone();
        session.core.shared.subscribe_kept_by_choice(Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }));
        // The window is in front a moment before the user switches: the layout it came with is its own, not a pick.
        session.idle(1.0);
        if picked {
            session.select_layout(0);
        }
        type_text(&mut session, typed);
        session.idle(1.0);
        assert_eq!(session.backend.text().iter().collect::<String>(), shown, "{typed:?}, picked by hand: {picked}");
        assert_eq!(count.load(Ordering::SeqCst), told, "{typed:?}, picked by hand: {picked}");
        bundle.give_back(session);
    }
    bundle.clean();
}

/// A stop after a word that waits for its next word leaves it waiting, and the next word settles it as it does with no
/// stop: `redis` typed in the Russian layout becomes Latin and the layout English, `b` waits for its neighbour, and
/// `nats` makes it `и` whether it comes at once or after a thought. The wait used to end at the stop, and `b` stood:
/// `compose`, `b`, a second and a half, `runbook` (the laptop, 15.11.2026, `и` meant). A second stop asks nothing more.
#[test]
fn a_stop_after_a_word_that_waits_for_its_neighbour_leaves_it_to_the_neighbour() {
    let mut bundle = Bundle::load(None);
    for stop in [0.2, 1.5, 3.0] {
        let mut session = bundle.session("app", 1, Vec::new());
        log_technically(&mut session);
        for (text, shown) in [("кувшы ", "redis "), ("b ", "redis b ")] {
            for character in text.chars() {
                press_as_laid_out(&mut session, character);
                session.idle(0.12);
            }
            assert_eq!(session.backend.text().iter().collect::<String>(), shown, "a stop of {stop} s");
        }
        session.idle(stop);
        for character in "nats ".chars() {
            press_as_laid_out(&mut session, character);
            session.idle(0.12);
        }
        session.idle(1.0);
        assert_eq!(session.backend.text().iter().collect::<String>(), "redis и nats ", "a stop of {stop} s");
        bundle.give_back(session);
    }
    bundle.clean();
}
