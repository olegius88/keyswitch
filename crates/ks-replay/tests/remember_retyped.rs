//! A word KeySwitch converted by itself, erased with Backspace and typed again as it was first typed: the word becomes a
//! "keep" rule of the person's and is not converted again. Typed in another way, after another edit, or erased sooner
//! than a person reads it, it is not remembered.
#![cfg(any(not(debug_assertions), coverage))]

use std::sync::{Arc, Mutex};

use ks_backend::KeyEvent;
use ks_backend::constants::SHIFT_MASK;
use ks_config::constants::{DETECTION_EARLY_SWITCH, DETECTION_LEARNING, DETECTION_REMEMBER_RETYPED, DETECTION_RESPECT_MANUAL_LAYOUT};
use ks_config::learning::{LearnedRule, RuleAction};
use ks_engine::RememberedWord;
use ks_replay::harness::{Bundle, KEYS, Session};

/// A press of the key of `character`, in the layout that types it: a layout other than the window's is picked first, by
/// hand.
fn press(session: &mut Session, character: char) {
    let Some((key, group)) = KEYS.iter().find_map(|key| Some((key, key.characters.iter().position(|typed| *typed == character)?))) else {
        panic!("no key types {character:?}");
    };
    if usize::try_from(session.backend.group()).ok() != Some(group) && character != ' ' {
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
        ..KeyEvent::default()
    });
}

/// A press of the key that types `character` in the layout the window is in.
fn press_as_laid_out(session: &mut Session, character: char) {
    let group = session.backend.group();
    let key = KEYS.iter().find(|key| usize::try_from(group).ok().and_then(|place| key.characters.get(place)) == Some(&character)).expect("a key of the layout");
    session.tap(KeyEvent {
        pressed: true,
        keycode: key.keycode,
        key_name: character.to_string(),
        character: character.to_string(),
        characters: key.strings(),
        group,
        state: if key.shift { SHIFT_MASK } else { 0 },
        ..KeyEvent::default()
    });
}

fn type_text(session: &mut Session, text: &str) {
    for character in text.chars() {
        press(session, character);
    }
}

fn erase(session: &mut Session, characters: usize) {
    for _ in 0..characters {
        session.tap_named("BackSpace");
    }
}

fn shown(session: &Session) -> String {
    session.backend.text().iter().collect()
}

fn keeps(session: &Session, group: i64, word: &str) -> bool {
    session.core.learning.r#match(group, word).is_some_and(|rule| rule.action == RuleAction::Keep)
}

/// The words the windows were told of.
fn told(session: &Session) -> Arc<Mutex<Vec<RememberedWord>>> {
    let words: Arc<Mutex<Vec<RememberedWord>>> = Arc::default();
    let sink = words.clone();
    session.core.shared.subscribe_remembered(Arc::new(move |word: &RememberedWord| sink.lock().expect("the list").push(word.clone())));
    words
}

/// An empty field in the Russian layout, the early switch off: `руддщ` (the keys of `hello`) is converted at its space.
fn russian(bundle: &mut Bundle) -> Session {
    let session = bundle.session("app", 1, Vec::new());
    session.core.settings.write(DETECTION_EARLY_SWITCH, false, false).expect("in memory");
    session.core.settings.set("diagnostics.technical_logging", true.into(), false).expect("in memory");
    session
}

/// `руддщ` typed in the Russian layout is converted to `hello`, the person reads it, erases it and types `руддщ` again in
/// the Russian layout, which they pick by hand: from then on `руддщ` stays as typed there, and the windows are told.
fn converted_and_erased(session: &mut Session) {
    type_text(session, "руддщ ");
    session.idle(1.0);
    assert_eq!(shown(session), "hello ", "converted at its space");
    erase(session, "hello ".chars().count());
}

/// The rules of a session go with it: the stores of a bundle are shared by its sessions.
fn done(bundle: &mut Bundle, session: Session) {
    session.core.learning.clear().expect("the rules cleared");
    bundle.give_back(session);
}

#[test]
fn a_word_converted_erased_and_typed_again_as_first_typed_is_remembered_and_kept_from_then_on() {
    let mut bundle = Bundle::load(None);
    let mut session = russian(&mut bundle);
    let words = told(&session);
    converted_and_erased(&mut session);
    type_text(&mut session, "руддщ ");
    session.idle(1.0);
    assert_eq!(shown(&session), "руддщ ", "typed again as first typed, the word stays");
    assert!(keeps(&session, 1, "руддщ"), "and is a keep rule of the Russian layout");
    let rule = session.core.learning.r#match(1, "руддщ").expect("the rule");
    assert_eq!(rule, LearnedRule { action: RuleAction::Keep, ..LearnedRule::new("руддщ", 1, 0) });
    assert_eq!(words.lock().expect("the list").iter().map(|word| word.rule.clone()).collect::<Vec<_>>(), [rule]);
    // No layout picked by hand now: the rule keeps the word.
    type_text(&mut session, "руддщ ");
    session.idle(1.0);
    assert_eq!(shown(&session), "руддщ руддщ ", "the rule keeps the word typed later");
    assert_eq!(words.lock().expect("the list").len(), 1, "told once");
    done(&mut bundle, session);
    bundle.clean();
}

/// «Отменить» of the notice takes the rule back, and the word is converted as before; taken back twice, nothing is left.
#[test]
fn the_undo_of_the_notice_takes_the_rule_back() {
    let mut bundle = Bundle::load(None);
    let mut session = russian(&mut bundle);
    let words = told(&session);
    converted_and_erased(&mut session);
    type_text(&mut session, "руддщ ");
    session.idle(1.0);
    let word = words.lock().expect("the list").first().cloned().expect("the windows were told");
    assert!(word.forget(), "the rule was there");
    assert!(!keeps(&session, 1, "руддщ") && session.core.learning.count() == 0);
    assert!(!word.forget(), "and is gone");
    type_text(&mut session, "руддщ ");
    session.idle(1.0);
    assert_eq!(shown(&session), "руддщ hello ", "converted again");
    done(&mut bundle, session);
    bundle.clean();
}

/// Backspaces begun as the correction came, sooner than anyone reads the word, were meant for the text as typed: the word
/// typed again there is not remembered. The same erased once read is.
#[test]
fn backspaces_begun_before_the_change_could_be_read_remember_nothing() {
    let mut bundle = Bundle::load(None);
    let mut session = russian(&mut bundle);
    type_text(&mut session, "руддщ ");
    assert_eq!(shown(&session), "hello ");
    erase(&mut session, "hello ".chars().count());
    type_text(&mut session, "руддщ ");
    session.idle(1.0);
    assert_eq!(shown(&session), "руддщ ", "kept by the layout picked by hand");
    assert_eq!(session.core.learning.count(), 0, "but not remembered");
    done(&mut bundle, session);

    let mut session = russian(&mut bundle);
    converted_and_erased(&mut session);
    type_text(&mut session, "руддщ ");
    session.idle(1.0);
    assert!(keeps(&session, 1, "руддщ"), "erased once read, it is");
    done(&mut bundle, session);
    bundle.clean();
}

/// Typed again as the correction made it, in the layout the correction left, the word is accepted: nothing is remembered.
/// Another word, a longer or a shorter one, or the same word in other letters, is no word as first typed either. A
/// hyphen before it, as before a part of a joined word, leaves it the word.
#[test]
fn the_corrected_form_or_another_word_typed_there_remembers_nothing() {
    let mut bundle = Bundle::load(None);
    for (again, kept) in [("hello ", "hello "), ("руддщщ ", "руддщщ "), ("рудд ", "рудд "), ("руддо ", "руддо ")] {
        let mut session = russian(&mut bundle);
        converted_and_erased(&mut session);
        type_text(&mut session, again);
        session.idle(1.0);
        assert_eq!(shown(&session), kept);
        assert_eq!(session.core.learning.count(), 0, "{again:?} is not the word as first typed");
        done(&mut bundle, session);
    }
    let mut session = russian(&mut bundle);
    converted_and_erased(&mut session);
    type_text(&mut session, "Руддщ ");
    session.idle(1.0);
    assert!(keeps(&session, 1, "руддщ"), "a capital is the same letter");
    done(&mut bundle, session);
    let mut session = russian(&mut bundle);
    converted_and_erased(&mut session);
    session.select_layout(1);
    press_as_laid_out(&mut session, '-');
    type_text(&mut session, "руддщ ");
    session.idle(1.0);
    assert!(keeps(&session, 1, "руддщ"), "after a hyphen");
    done(&mut bundle, session);
    bundle.clean();
}

/// A click, a caret move or another window between the erasing and the typing: the word is typed somewhere else, and is
/// not remembered.
#[test]
fn a_click_a_caret_move_or_another_window_after_the_erasing_remember_nothing() {
    let mut bundle = Bundle::load(None);
    let moves: [fn(&mut Session); 3] = [
        |session| session.pointer(),
        |session| session.tap_named("Left"),
        |session| session.backend.focus(2, "other"),
    ];
    for (number, moved) in moves.into_iter().enumerate() {
        let mut session = russian(&mut bundle);
        converted_and_erased(&mut session);
        moved(&mut session);
        type_text(&mut session, "руддщ ");
        session.idle(1.0);
        assert_eq!(session.core.learning.count(), 0, "edit {number}");
        done(&mut bundle, session);
    }
    let mut session = russian(&mut bundle);
    converted_and_erased(&mut session);
    type_text(&mut session, "руддщ ");
    session.idle(1.0);
    assert!(keeps(&session, 1, "руддщ"), "with nothing between, it is");
    done(&mut bundle, session);
    bundle.clean();
}

/// With the setting off, or with the rules off, nothing is remembered; a rule that already decides about the word is
/// left as it is.
#[test]
fn the_setting_off_the_rules_off_or_a_rule_of_the_word_remember_nothing() {
    let mut bundle = Bundle::load(None);
    for setting in [DETECTION_REMEMBER_RETYPED, DETECTION_LEARNING] {
        let mut session = russian(&mut bundle);
        session.core.settings.write(setting, false, false).expect("in memory");
        converted_and_erased(&mut session);
        type_text(&mut session, "руддщ ");
        session.idle(1.0);
        assert_eq!(session.core.learning.count(), 0, "{} off", setting.path());
        done(&mut bundle, session);
    }
    // Turned off between the erasing and the typing.
    let mut session = russian(&mut bundle);
    converted_and_erased(&mut session);
    session.core.settings.write(DETECTION_REMEMBER_RETYPED, false, false).expect("in memory");
    type_text(&mut session, "руддщ ");
    session.idle(1.0);
    assert_eq!(session.core.learning.count(), 0, "turned off before the word was typed again");
    done(&mut bundle, session);
    // A file of rules that cannot be written: the word stays as typed by the layout picked by hand, the store keeps the rule
    // in memory, as it does a rule of the rule window, and nothing is told.
    let mut session = russian(&mut bundle);
    let words = told(&session);
    let file = session.core.learning.path.clone();
    let _ = std::fs::remove_file(&file);
    std::fs::create_dir_all(&file).expect("a folder in place of the file");
    converted_and_erased(&mut session);
    type_text(&mut session, "руддщ ");
    session.idle(1.0);
    assert_eq!(shown(&session), "руддщ ");
    assert!(words.lock().expect("the list").is_empty(), "nothing told of a rule not written");
    assert_eq!(session.core.learning.count(), 1, "the store keeps what it could not write, as for a rule of the window");
    std::fs::remove_dir(&file).expect("the folder taken away");
    done(&mut bundle, session);
    // A rule of the user's own converts every word that begins with `рудд`: the word is converted by it, and erased and
    // typed again, it stays by the layout picked by hand, and the user's rule stays the only one.
    let mut session = russian(&mut bundle);
    let mine = LearnedRule { r#match: ks_config::learning::RuleMatch::Prefix, ..LearnedRule::new("рудд", 1, 0) };
    session.core.learning.add_rule(mine.clone()).expect("a rule");
    converted_and_erased(&mut session);
    type_text(&mut session, "руддщ ");
    session.idle(1.0);
    assert_eq!(shown(&session), "руддщ ");
    assert_eq!(session.core.learning.rules(), [mine], "the user's rule is left as it is");
    done(&mut bundle, session);

    let mut session = russian(&mut bundle);
    converted_and_erased(&mut session);
    type_text(&mut session, "руддщ ");
    session.idle(1.0);
    assert!(keeps(&session, 1, "руддщ"), "with all of them on and no rule, it is");
    done(&mut bundle, session);
    bundle.clean();
}

/// With the layout picked by hand not protecting the next word, the word typed again is the engine's to decide: it is
/// remembered before it is decided, so it stays too, and nothing converts its first letters while it is typed.
#[test]
fn the_word_typed_again_stays_without_the_protection_of_a_layout_picked_by_hand() {
    let mut bundle = Bundle::load(None);
    let mut session = russian(&mut bundle);
    session.core.settings.write(DETECTION_RESPECT_MANUAL_LAYOUT, false, false).expect("in memory");
    session.core.settings.write(DETECTION_EARLY_SWITCH, true, false).expect("in memory");
    converted_and_erased(&mut session);
    for character in "руддщ".chars() {
        press(&mut session, character);
        // Slowly, with a pause after each letter: neither the early switch nor the pause converts them.
        session.idle(1.0);
    }
    assert_eq!(shown(&session), "руддщ");
    press(&mut session, ' ');
    session.idle(1.0);
    assert_eq!(shown(&session), "руддщ ", "kept at its space");
    assert!(keeps(&session, 1, "руддщ"));
    assert_eq!(session.core.shared.snapshot().last_action, "Слово запомнено: «руддщ»");
    done(&mut bundle, session);
    // Letters that leave the word as first typed, by a letter more, another letter or another layout, are the engine's to
    // convert as any: nothing is remembered.
    for again in ["руддщщ", "рудзщ", "ghbdtn"] {
        let mut session = russian(&mut bundle);
        session.core.settings.write(DETECTION_RESPECT_MANUAL_LAYOUT, false, false).expect("in memory");
        session.core.settings.write(DETECTION_EARLY_SWITCH, true, false).expect("in memory");
        converted_and_erased(&mut session);
        for character in again.chars() {
            press(&mut session, character);
            session.idle(1.0);
        }
        press(&mut session, ' ');
        session.idle(1.0);
        assert_eq!(session.core.learning.count(), 0, "{again}");
        done(&mut bundle, session);
    }
    bundle.clean();
}

/// A word converted while it is typed, by a pause or by the early switch, is the whole word typed: erased while it is
/// still typed and typed again whole as first typed, it is remembered.
#[test]
fn a_word_converted_while_it_is_typed_is_remembered_whole() {
    let mut bundle = Bundle::load(None);
    // A pause after `руддщ` converts it before its space.
    let mut session = russian(&mut bundle);
    type_text(&mut session, "руддщ");
    session.idle(1.0);
    assert_eq!(shown(&session), "hello", "converted at the pause");
    session.idle(1.0);
    erase(&mut session, "hello".chars().count());
    type_text(&mut session, "руддщ ");
    session.idle(1.0);
    assert_eq!(shown(&session), "руддщ ");
    assert!(keeps(&session, 1, "руддщ"), "remembered after a pause");
    done(&mut bundle, session);

    // The early switch converts `ghbd` as it is typed, and the word goes on in the other layout: `ет` is `tn` there.
    let mut session = bundle.session("app", 0, Vec::new());
    type_text(&mut session, "ghbd");
    assert_eq!(shown(&session), "прив", "switched while typed");
    type_text(&mut session, "ет");
    assert_eq!(shown(&session), "привет");
    session.idle(1.0);
    erase(&mut session, "привет".chars().count());
    type_text(&mut session, "ghbdtn ");
    session.idle(1.0);
    assert_eq!(shown(&session), "ghbdtn ");
    assert!(keeps(&session, 0, "ghbdtn"), "remembered after the early switch");
    type_text(&mut session, "ghbdtn ");
    session.idle(1.0);
    assert_eq!(shown(&session), "ghbdtn ghbdtn ", "and kept from then on");
    done(&mut bundle, session);
    bundle.clean();
}
