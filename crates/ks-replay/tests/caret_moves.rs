//! The first word typed after the caret was moved with the keyboard in a field that cannot be read. It is not decided
//! alone, for what stands around it is not known; it waits for the word after it, which shows the layout the person is
//! typing in, and goes with that word when the model converts the pair: End after a fix earlier in the line, then
//! `b ghjdthm`, is `и проверь`. With no word after it, beside a word that stays, or after another caret move it stays
//! as typed.
#![cfg(any(not(debug_assertions), coverage))]

use ks_backend::KeyEvent;
use ks_backend::constants::SHIFT_MASK;
use ks_config::constants::{DETECTION_CONTEXT_POLICY, DETECTION_CONTEXT_READ_FIELD, DETECTION_HOLD_AFTER_CARET_MOVE, DIAGNOSTICS_TECHNICAL_LOGGING};
use ks_replay::harness::{Bundle, KEYS, PAUSE_KEYCODE, Session};

const RUSSIAN: i32 = 1;

/// One step of a scene.
enum Step {
    /// The keys that type this text in the US layout, pressed in whatever layout is on.
    Keys(&'static str),
    /// Arrow keys to the left, then End: the caret is back at the end of the line, where the engine cannot see it.
    LeftThenEnd,
    /// No key for this many seconds.
    Idle(f64),
    /// Enter, held back until the word before it is decided and then delivered: the field is empty after it.
    Enter,
    /// The layout picked by hand.
    Pick(i32),
    /// One key that moves the caret, by its name; the field's caret is put back at the end of the line.
    Tap(&'static str),
    /// The conversion hotkey.
    Pause,
}
use Step::{Enter, Idle, Keys, LeftThenEnd, Pause, Pick, Tap};

fn press(session: &mut Session, english: char) {
    let key = KEYS.iter().find(|key| key.characters[0] == english).expect("a key of the US layout");
    let group = session.backend.group();
    let strings = key.strings();
    session.tap(KeyEvent {
        pressed: true,
        keycode: key.keycode,
        key_name: english.to_string(),
        character: strings[usize::try_from(group).expect("a layout of the two")].clone(),
        characters: strings,
        group,
        state: if key.shift { SHIFT_MASK } else { 0 },
        ..KeyEvent::default()
    });
}

fn play(session: &mut Session, steps: &[Step]) {
    for step in steps {
        match step {
            Keys(text) => {
                for character in text.chars() {
                    press(session, character);
                    session.idle(if character == ' ' { 0.3 } else { 0.1 });
                }
            }
            LeftThenEnd => {
                session.tap_named("Left");
                session.tap_named("Left");
                session.tap_named("End");
                // The replay's field moves its caret for Left only.
                session.backend.set_caret(session.backend.text().len());
                session.idle(0.2);
            }
            Idle(seconds) => session.idle(*seconds),
            Enter => {
                let group = session.backend.group();
                let enter = KeyEvent { pressed: true, deferred: true, key_name: "Return".to_owned(), characters: vec![String::new(), String::new()], group, ..KeyEvent::default() };
                session.backend.watch_enter(enter.clone());
                session.handle(enter.clone());
                session.idle(0.2);
                session.handle(KeyEvent { pressed: false, deferred: false, ..enter });
                session.backend.stop_watching_enter();
                session.idle(0.2);
            }
            Pick(group) => session.select_layout(*group),
            Tap(name) => {
                session.tap_named(name);
                session.backend.set_caret(session.backend.text().len());
                session.idle(0.2);
            }
            Pause => {
                let group = session.backend.group();
                session.tap(KeyEvent { pressed: true, keycode: PAUSE_KEYCODE, key_name: "Pause".to_owned(), characters: vec![String::new(), String::new()], group, ..KeyEvent::default() });
            }
        }
    }
    session.idle(3.0);
}

/// What stands in a field the engine cannot read after the steps, begun empty in the Russian layout.
fn shown_in(bundle: &mut Bundle, prepare: impl Fn(&mut Session), steps: &[Step]) -> String {
    let mut session = bundle.session("remote desktop", RUSSIAN, Vec::new());
    session.core.settings.write(DETECTION_CONTEXT_READ_FIELD, false, false).expect("in memory");
    prepare(&mut session);
    play(&mut session, steps);
    let shown = session.backend.text().iter().collect();
    bundle.give_back(session);
    shown
}

fn shown(bundle: &mut Bundle, steps: &[Step]) -> String {
    shown_in(bundle, |_| {}, steps)
}

/// `мы взяли redis `, the term typed in the Russian layout and converted, which leaves the English layout on; then the
/// caret moved back into the line and to its end again.
const TERM_THEN_CARET: [Step; 2] = [Keys("vs dpzkb redis "), LeftThenEnd];

fn after_the_caret_move(bundle: &mut Bundle, steps: Vec<Step>) -> String {
    let all: Vec<Step> = TERM_THEN_CARET.into_iter().chain(steps).collect();
    shown(bundle, &all)
}

#[test]
fn the_first_word_after_a_caret_move_goes_with_the_converted_word_after_it() {
    let mut bundle = Bundle::load(None);
    // The term itself, to see what the scenes begin with.
    assert_eq!(shown(&mut bundle, &[Keys("vs dpzkb redis ")]), "мы взяли redis ");
    for (typed, ends) in [
        // A Russian word of one letter, the word after it converted while it was typed or at its space.
        ("b ghjdthm ", "и проверь "),
        ("d ghjtrnt ", "в проекте "),
        ("z gjnjv ", "я потом "),
        // A short word after it, which only the pair shows to be Russian.
        ("d vs ", "в мы "),
        ("b c ybv ", "и с ним "),
        ("z yt pyf. ", "я не знаю "),
        // A longer first word.
        ("gjnjv ghjdthm ", "потом проверь "),
        // A sign split off the end of the word after it.
        ("b ghjdthm/ ", "и проверь. "),
        // A sign at the end of the first word that the Russian layout types on the same key.
        ("nfr/ ghbdtn ", "так. привет "),
    ] {
        assert_eq!(after_the_caret_move(&mut bundle, vec![Keys(typed)]), format!("мы взяли redis {ends}"), "{typed}");
    }
    // A stop for thought after the letter, and one in the middle of the word after it.
    assert_eq!(after_the_caret_move(&mut bundle, vec![Keys("b "), Idle(6.0), Keys("ghjdthm ")]), "мы взяли redis и проверь ");
    assert_eq!(after_the_caret_move(&mut bundle, vec![Keys("b ghj"), Idle(1.5), Keys("dthm ")]), "мы взяли redis и проверь ");
    // The Russian layout picked by hand for the word after it: the letter was typed before the person noticed.
    assert_eq!(after_the_caret_move(&mut bundle, vec![Keys("b "), Pick(RUSSIAN), Keys("ghjdthm ")]), "мы взяли redis и проверь ");
    bundle.clean();
}

#[test]
fn what_is_typed_right_after_a_caret_move_stays_as_typed() {
    let mut bundle = Bundle::load(None);
    for (typed, ends) in [
        // English after the term, a letter among it.
        ("option b is ", "option b is "),
        ("the next one ", "the next one "),
        // A term typed right before Russian words typed in the English layout: only they convert.
        ("nginx ghjdthm ", "nginx проверь "),
        // The letter before a word that stays: nothing shows it was meant Russian.
        ("b nginx ghjdthm ", "b nginx проверь "),
        // A dash is no word of either layout.
        ("- ghjdthm ", "- проверь "),
        // English that ends its sentence before Russian words typed in the English layout.
        ("ok. ghbdtn ", "ok. привет "),
        // Tokens that do not wait, as things stand: one the boundary model cannot split, and one whose sign it splits off
        // as typed. They stay, and the word after them converts alone.
        ("dc` ghbdtn ", "dc` привет "),
        ("ghbdtn? vbh ", "ghbdtn? мир "),
    ] {
        assert_eq!(after_the_caret_move(&mut bundle, vec![Keys(typed)]), format!("мы взяли redis {ends}"), "{typed}");
    }
    // No word after it: no pause decides the letter.
    assert_eq!(after_the_caret_move(&mut bundle, vec![Keys("b "), Idle(8.0)]), "мы взяли redis b ");
    // The caret moved once more: the word after it is the first after a move itself.
    assert_eq!(after_the_caret_move(&mut bundle, vec![Keys("b "), LeftThenEnd, Keys("ghjdthm ")]), "мы взяли redis b ghjdthm ");
    // Enter ends the letter's line: the word of the next line is decided on its own.
    assert_eq!(after_the_caret_move(&mut bundle, vec![Keys("b"), Enter, Keys("ghjdthm ")]), "проверь ");
    // Russian typed right after the move, in the layout picked for it.
    assert_eq!(shown(&mut bundle, &[Keys("vs dpzkb "), LeftThenEnd, Keys("b gjnjv ")]), "мы взяли и потом ");
    bundle.clean();
}

/// What the rule was before the word came to wait, and still is: every key that moves the caret holds the word after
/// it, the setting turns the rule off, a field that can be read needs no rule, and the hotkey converts the held word.
/// The recorded traces of the previous engine checked these; they stopped at the wait the word now starts.
#[test]
fn every_caret_key_holds_the_word_after_it_and_the_rule_has_its_ways_out() {
    let mut bundle = Bundle::load(None);
    for key in ["Left", "Right", "Up", "Down", "Home", "End", "Page_Up", "Page_Down"] {
        // `привет` typed in the English layout with no word after it: it is left alone.
        assert_eq!(shown(&mut bundle, &[Keys("vs dpzkb redis "), Tap(key), Keys("ghbdtn ")]), "мы взяли redis ghbdtn ", "{key}");
    }
    // Only the first word after the move is held: the second is decided as any word, and takes the first along.
    assert_eq!(shown(&mut bundle, &[Keys("vs dpzkb redis "), Tap("Up"), Keys("ghbdtn vbh ")]), "мы взяли redis привет мир ");
    // The hotkey converts the held word by hand.
    assert_eq!(shown(&mut bundle, &[Keys("vs dpzkb redis "), Tap("Left"), Keys("ghbdtn "), Pause]), "мы взяли redis привет ");
    // The rule turned off: the word is decided at its space, as at the start of a field.
    let off = |session: &mut Session| session.core.settings.write(DETECTION_HOLD_AFTER_CARET_MOVE, false, false).expect("in memory");
    assert_eq!(shown_in(&mut bundle, off, &[Keys("vs dpzkb redis "), Tap("Left"), Keys("ghbdtn ")]), "мы взяли redis привет ");
    // A field that can be read answers what stands before the caret: the word is decided at its space.
    let readable = |session: &mut Session| session.core.settings.write(DETECTION_CONTEXT_READ_FIELD, true, false).expect("in memory");
    assert_eq!(shown_in(&mut bundle, readable, &[Keys("vs dpzkb redis "), Tap("End"), Keys("ghbdtn ")]), "мы взяли redis привет ");
    // Reading is on and there is no reader to ask: the rule holds the word.
    let no_reader = |session: &mut Session| {
        session.core.settings.write(DETECTION_CONTEXT_READ_FIELD, true, false).expect("in memory");
        session.core.context_policy.reader = None;
    };
    assert_eq!(shown_in(&mut bundle, no_reader, &[Keys("vs dpzkb redis "), Tap("End"), Keys("ghbdtn ")]), "мы взяли redis ghbdtn ");
    // In a chat program a quotation mark before the word may open a mention: such a word does not wait.
    let mut session = bundle.session("telegram-desktop", RUSSIAN, Vec::new());
    session.core.settings.write(DETECTION_CONTEXT_READ_FIELD, false, false).expect("in memory");
    play(&mut session, &[Keys("vs dpzkb "), Tap("End"), Keys("@ghbdtn ")]);
    assert!(session.core.context_waiting.is_none());
    assert_eq!(session.backend.text().iter().collect::<String>(), "мы взяли \"привет ");
    bundle.give_back(session);
    bundle.clean();
}

/// A word that waits for its next word stops waiting when the caret moves, a key erases or its time runs out, and the word
/// typed after that goes without it.
#[test]
fn a_caret_move_an_erasure_and_time_end_a_wait() {
    let mut bundle = Bundle::load(None);
    for (name, ends) in [("caret", Tap("Left")), ("erasure", Tap("BackSpace"))] {
        let mut session = bundle.session("remote desktop", 0, Vec::new());
        session.core.settings.write(DETECTION_CONTEXT_READ_FIELD, false, false).expect("in memory");
        // A letter after an English word may be the Russian `и`: only the word after it can tell, so it waits.
        for character in "nginx b ".chars() {
            press(&mut session, character);
        }
        assert!(session.core.context_waiting.is_some(), "{name}: the word waits");
        play(&mut session, &[ends]);
        assert!(session.core.context_waiting.is_none(), "{name}: the wait is over");
        bundle.give_back(session);
    }
    // In time the letter is waited for, the word after it takes it along; two minutes later it goes alone.
    let english = |session: &mut Session| session.backend.set_group(0);
    assert_eq!(shown_in(&mut bundle, english, &[Keys("nginx b "), Idle(10.0), Keys("ghjdthm ")]), "nginx и проверь ");
    assert_eq!(shown_in(&mut bundle, english, &[Keys("nginx b "), Idle(120.0), Keys("ghjdthm ")]), "nginx b проверь ");
    bundle.clean();
}

#[test]
fn the_wait_after_a_caret_move_is_the_context_models_to_settle() {
    let mut bundle = Bundle::load(None);
    // With the context model only watching, no word waits for its neighbour: the letter stays, the word after it converts.
    let watching = |session: &mut Session| session.core.settings.write(DETECTION_CONTEXT_POLICY, "shadow".to_owned(), false).expect("in memory");
    let all: Vec<Step> = TERM_THEN_CARET.into_iter().chain([Keys("b ghjdthm ")]).collect();
    assert_eq!(shown_in(&mut bundle, watching, &all), "мы взяли redis b проверь ");
    // The log says why the letter waits.
    let logging = |session: &mut Session| session.core.settings.write(DIAGNOSTICS_TECHNICAL_LOGGING, true, false).expect("in memory");
    assert_eq!(shown_in(&mut bundle, logging, &all), "мы взяли redis и проверь ");
    bundle.clean();
}
