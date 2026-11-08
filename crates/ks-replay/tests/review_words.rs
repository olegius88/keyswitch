//! Long words typed in the wrong layout: a word ending on a key that is a sign in the other layout, and a word with a slip
//! in it, and what a selection hotkey that could not do its work tells the person.
#![cfg(any(not(debug_assertions), coverage))]

use std::sync::{Arc, Mutex};

use ks_backend::KeyEvent;
use ks_backend::constants::SHIFT_MASK;
use ks_config::selection_commands::SelectionCommand;
use ks_engine::engine::Stroke;
use ks_replay::harness::{Bundle, KEYS, Session};
use serde_json::json;

/// The press of the key that types `character` in the English layout, in whatever layout is on now.
fn press(session: &mut Session, character: char) {
    let key = KEYS.iter().find(|key| key.characters.first() == Some(&character)).unwrap_or_else(|| panic!("no key types {character}"));
    let group = session.backend.group();
    let characters = key.strings();
    let typed = characters[usize::try_from(group).expect("a layout")].clone();
    session.tap(KeyEvent {
        pressed: true,
        keycode: key.keycode,
        key_name: typed.clone(),
        character: typed,
        characters,
        group,
        state: if key.shift { SHIFT_MASK } else { 0 },
        ..KeyEvent::default()
    });
}

fn typed(session: &mut Session, keys: &str) -> String {
    for character in keys.chars() {
        press(session, character);
    }
    session.idle(1.0);
    session.backend.text().into_iter().collect()
}

fn alone(bundle: &mut Bundle, keys: &str, typos: bool) -> String {
    let mut session = bundle.session("notepad", 0, Vec::new());
    session.core.settings.set("detection.correct_typos", json!(typos), false).expect("in memory");
    let text = typed(&mut session, keys);
    bundle.give_back(session);
    text
}

fn strokes(keys: &str) -> Vec<Stroke> {
    keys.chars()
        .map(|character| {
            let key = KEYS.iter().find(|key| key.characters.first() == Some(&character)).expect("a key");
            Arc::new(KeyEvent {
                pressed: true,
                keycode: key.keycode,
                key_name: character.to_string(),
                character: character.to_string(),
                characters: key.strings(),
                ..KeyEvent::default()
            })
        })
        .collect()
}

#[test]
fn a_long_word_on_a_sign_key_is_converted_when_the_boundary_model_cannot_split_it() {
    let mut bundle = Bundle::load(None);
    // The boundary model abstains on these, and there is no next word to bring them along.
    assert_eq!(alone(&mut bundle, "nehybhf[ ", false), "турнирах ");
    assert_eq!(alone(&mut bundle, "cntgtym. ", false), "степенью ");
    // English and a sign, and a token no language has, stay as typed.
    assert_eq!(alone(&mut bundle, "report. ", false), "report. ");
    assert_eq!(alone(&mut bundle, "qzxqzx[ ", false), "qzxqzx[ ");
    bundle.clean();
}

#[test]
fn a_slip_in_a_word_the_early_switch_converted_is_corrected_too() {
    let mut bundle = Bundle::load(None);
    // `gcb[jkujbb` switches to the Russian layout in the middle of the word, and is `психолгоии` with two letters swapped.
    assert_eq!(alone(&mut bundle, "gcb[jkujbb ", false), "психолгоии ");
    assert_eq!(alone(&mut bundle, "gcb[jkujbb ", true), "психологии ");
    // A word without a slip stays what the switch made it.
    assert_eq!(alone(&mut bundle, "gcb[jkjubb ", true), "психологии ");
    bundle.clean();
}

#[test]
fn only_a_known_word_of_enough_letters_decides_where_a_token_ends() {
    let mut bundle = Bundle::load(None);
    let session = bundle.session("notepad", 0, Vec::new());
    let core = &session.core;
    assert_eq!(core.signs_after_known_word(&strokes("nehybhf["), 1, 0, 1), Some(0));
    // With a sign after the letter, the longest reading that is a word leaves that sign out.
    assert_eq!(core.signs_after_known_word(&strokes("Ljk.?"), 2, 0, 1), Some(1));
    // Too short to tell a word from a word and a sign: `qx.` is no word here, and `йчю` is too short to count as one.
    assert_eq!(core.signs_after_known_word(&strokes("qx."), 1, 0, 1), None);
    assert_eq!(core.signs_after_known_word(&strokes("vs."), 1, 0, 1), None);
    // The other layout spells no word.
    assert_eq!(core.signs_after_known_word(&strokes("qzxqzx["), 1, 0, 1), None);
    // `bye.` spells `иную`, and `bye` is a word of its own: it is that word and a full stop.
    assert_eq!(core.signs_after_known_word(&strokes("bye."), 1, 0, 1), None);
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn a_selection_hotkey_that_could_not_do_its_work_says_why() {
    let mut bundle = Bundle::load(None);
    let mut session = bundle.session("notepad", 0, Vec::new());
    let heard: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = heard.clone();
    session.core.shared.subscribe_selection_notices(Arc::new(move |message: &str| sink.lock().expect("not poisoned").push(message.to_owned())));
    let told = |heard: &Arc<Mutex<Vec<String>>>| std::mem::take(&mut *heard.lock().expect("not poisoned"));
    // Nothing selected: the backend refuses, and that is the message.
    session.backend.set_selection(None);
    session.core.run_selection_command(SelectionCommand::Layout);
    assert_eq!(told(&heard), ["Нет выделенного текста"]);
    // The engine refuses before it asks the backend.
    session.core.settings.set("enabled", json!(false), false).expect("in memory");
    session.core.run_selection_command(SelectionCommand::Layout);
    assert_eq!(told(&heard), ["Выделение не изменено: KeySwitch на паузе"]);
    session.core.settings.set("enabled", json!(true), false).expect("in memory");
    // The backend fails.
    session.backend.set_selection(Some("ghbdtn"));
    session.backend.set_failing(true);
    session.core.run_selection_command(SelectionCommand::Layout);
    session.backend.set_failing(false);
    assert_eq!(told(&heard), ["Ошибка замены выделения · проверьте текст в приложении"]);
    // The keys of the hotkey are not seen to come up.
    session.core.schedule_selection_command("selection_layout");
    session.core.modifier_keycodes.insert(50);
    session.idle(10.0);
    session.core.expire_selection_command();
    session.core.modifier_keycodes.clear();
    assert_eq!(told(&heard), ["Выделение не изменено: не получено отпускание клавиш"]);
    // A hotkey that did its work says nothing here.
    session.backend.set_selection(Some("ghbdtn"));
    session.core.run_selection_command(SelectionCommand::Layout);
    assert!(told(&heard).is_empty());
    bundle.give_back(session);
    bundle.clean();
}
