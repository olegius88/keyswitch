//! A word the model waited on, then the layout picked by hand and a word typed in it: the picked word stays as typed,
//! and it is the context the waiting word waited for.
#![cfg(any(not(debug_assertions), coverage))]

use std::sync::Arc;

use ks_backend::KeyEvent;
use ks_backend::constants::SHIFT_MASK;
use ks_config::constants::{DETECTION_CONTEXT_POLICY, DETECTION_CONTEXT_READ_FIELD, DIAGNOSTICS_TECHNICAL_LOGGING, EXCLUSIONS_APPLICATIONS};
use ks_engine::engine::{CorrectionPlan, Stroke, Strokes, WaitingContextWord};
use ks_replay::harness::{Bundle, KEYS, PAUSE_KEYCODE, Session};

/// The key that types `english` in the US layout, pressed in whatever layout is on.
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

/// What happens after the text is typed.
#[derive(Clone, Copy)]
enum After {
    Nothing,
    Pause,
    Undo,
}

/// The field and its layout after `steps` were typed from an empty field in the layout `group`: each step is a layout
/// picked by hand first, if any, and the keys as on a US keyboard, a short gap after each space.
fn typed(bundle: &mut Bundle, group: i32, steps: &[(Option<i32>, &str)], after: After) -> (String, i32) {
    let mut session = bundle.session("firefox", group, Vec::new());
    session.core.settings.write(DETECTION_CONTEXT_READ_FIELD, true, false).expect("in memory");
    // The log says what the engine saw, the resolved wait included.
    session.core.settings.write(DIAGNOSTICS_TECHNICAL_LOGGING, true, false).expect("in memory");
    for (picked, keys) in steps {
        if let Some(layout) = picked {
            session.select_layout(*layout);
        }
        for character in keys.chars() {
            press(&mut session, character);
            if character == ' ' {
                session.idle(0.4);
            }
        }
    }
    session.idle(3.0);
    match after {
        After::Nothing => {}
        After::Pause => session.tap_named("Pause"),
        After::Undo => {
            session.core.schedule_undo_hotkey(i64::from(PAUSE_KEYCODE));
            session.handle(KeyEvent { pressed: false, keycode: PAUSE_KEYCODE, key_name: "Pause".to_owned(), ..KeyEvent::default() });
        }
    }
    session.idle(2.0);
    let shown = (session.backend.text().iter().collect(), session.backend.group());
    bundle.give_back(session);
    shown
}

const PHRASE: &str = "f xnj gj gjdjle ghj,ktvs? rjulf yfgbcfk yjdsq rjynfrn yf ";

#[test]
fn a_waiting_word_converts_before_a_word_typed_in_the_layout_picked_for_it() {
    let mut bundle = Bundle::load(None);
    // `z` waits after `olegius888 ? `; the Russian layout is picked by hand and `ожидал,` typed in it. The wait was
    // dropped with the analysis of the protected word, and `z` stayed (field typing, 10.11.2026).
    let steps = [(None, PHRASE), (Some(0), "olegius888 ? z "), (Some(1), "j;blfk? xnj ")];
    let (text, layout) = typed(&mut bundle, 1, &steps, After::Nothing);
    assert_eq!(text, "а что по поводу проблемы, когда написал новый контакт на olegius888 ? я ожидал, что ");
    assert_eq!(layout, 1, "the layout picked by hand stays");
    // Pause right after the correction, and the undo hotkey, give the text back as it was typed, the picked word in
    // the layout it was typed in.
    let steps = [(None, PHRASE), (Some(0), "olegius888 ? z "), (Some(1), "j;blfk? ")];
    for after in [After::Pause, After::Undo] {
        assert_eq!(typed(&mut bundle, 1, &steps, after), ("а что по поводу проблемы, когда написал новый контакт на olegius888 ? z ожидал, ".to_owned(), 1));
    }
    bundle.clean();
}

#[test]
fn a_word_the_model_keeps_before_a_picked_word_stays() {
    let mut bundle = Bundle::load(None);
    // English typed on purpose before a Russian word picked by hand: the model keeps it, as it keeps a term before
    // a Russian word typed in the Russian layout.
    let steps = [(None, PHRASE), (Some(0), "olegius888 ? I "), (Some(1), "j;blfk? xnj ")];
    assert_eq!(typed(&mut bundle, 1, &steps, After::Nothing).0, "а что по поводу проблемы, когда написал новый контакт на olegius888 ? I ожидал, что ");
    assert_eq!(typed(&mut bundle, 1, &[(None, "yf "), (Some(0), "github "), (Some(1), "ytn ")], After::Nothing).0, "на github нет ");
    bundle.clean();
}

/// The key that types `character`, in the layout it types it in.
fn stroke(character: char) -> Stroke {
    let (physical, group) = KEYS
        .iter()
        .find_map(|key| Some((key, key.characters.iter().position(|typed| *typed == character)?)))
        .unwrap_or_else(|| panic!("no key types {character}"));
    Arc::new(KeyEvent {
        pressed: true,
        keycode: physical.keycode,
        key_name: character.to_string(),
        character: character.to_string(),
        characters: physical.strings(),
        group: i32::try_from(group).expect("a layout of the two"),
        ..KeyEvent::default()
    })
}

fn strokes(text: &str) -> Strokes {
    text.chars().map(stroke).collect()
}

#[test]
fn a_waiting_word_is_asked_before_a_picked_word_only_when_all_holds() {
    let mut bundle = Bundle::load(None);
    let mut session = bundle.session("firefox", 1, Vec::new());
    session.core.settings.write(DETECTION_CONTEXT_READ_FIELD, true, false).expect("in memory");
    for character in "yf ".chars() {
        press(&mut session, character);
    }
    session.select_layout(0);
    for character in "olegius888 ? z ".chars() {
        press(&mut session, character);
    }
    let waiting = session.core.context_waiting.clone().expect("`z` waits for its neighbour");
    let core = &mut session.core;
    let (word, space, application) = (strokes("ожидал"), stroke(' '), waiting.plan.application.clone());
    let with_plan = |change: &dyn Fn(&mut CorrectionPlan)| {
        let mut plan = (*waiting.plan).clone();
        change(&mut plan);
        WaitingContextWord { plan: Arc::new(plan), ..waiting.clone() }
    };
    let declined = |core: &mut ks_engine::engine::EngineCore, waiting: &WaitingContextWord, keys: &Strokes, boundary: &Stroke, text: &str, picked: i64, application: &str| {
        core.decide_before_picked_word(waiting, keys, boundary, text, picked, application, true).expect("no failure").is_none()
    };
    // As typed so far the text ends before the picked word: nothing to decide yet.
    assert!(declined(core, &waiting, &word, &space, "ожидал", 1, &application));
    // A word that could not be decided at all, a program excluded.
    assert!(core.decide_before_picked_word(&waiting, &word, &space, "ожидал", 1, &application, false).expect("no failure").is_none());
    core.settings.write(EXCLUSIONS_APPLICATIONS, vec![application.clone()], false).expect("in memory");
    assert!(declined(core, &waiting, &word, &space, "ожидал", 1, &application));
    core.settings.write(EXCLUSIONS_APPLICATIONS, Vec::new(), false).expect("in memory");
    // A waiting word whose space is another sign, a model of another version, a word converted already, the policy off.
    assert!(declined(core, &with_plan(&|plan| plan.boundary = Some(stroke(','))), &word, &space, "ожидал", 1, &application));
    let model = core.context_policy.model.take();
    assert!(declined(core, &waiting, &word, &space, "ожидал", 1, &application));
    core.context_policy.model = model;
    assert!(declined(core, &WaitingContextWord { converted: true, ..waiting.clone() }, &word, &space, "ожидал", 1, &application));
    core.settings.write(DETECTION_CONTEXT_POLICY, "off".to_owned(), false).expect("in memory");
    assert!(declined(core, &waiting, &word, &space, "ожидал", 1, &application));
    core.settings.write(DETECTION_CONTEXT_POLICY, "assist".to_owned(), false).expect("in memory");
    // The layout picked away from the waiting word's reading, a boundary still held back, no word or two.
    assert!(declined(core, &waiting, &word, &space, "ожидал", 0, &application));
    let deferred = Arc::new(KeyEvent { deferred: true, ..(*space).clone() });
    assert!(declined(core, &waiting, &word, &deferred, "ожидал", 1, &application));
    assert!(declined(core, &waiting, &word, &space, "", 1, &application));
    assert!(declined(core, &waiting, &word, &space, "ожидал всё", 1, &application));
    // Too late, in another window or program, keys of the waiting word or of the picked word in the other layout.
    assert!(declined(core, &WaitingContextWord { deadline: 0.0, ..waiting.clone() }, &word, &space, "ожидал", 1, &application));
    assert!(declined(core, &WaitingContextWord { window: waiting.window + 1, ..waiting.clone() }, &word, &space, "ожидал", 1, &application));
    assert!(declined(core, &waiting, &word, &space, "ожидал", 1, "another"));
    assert!(declined(core, &with_plan(&|plan| plan.strokes.push(stroke('я'))), &word, &space, "ожидал", 1, &application));
    assert!(declined(core, &waiting, &strokes("j;blfk"), &space, "ожидал", 1, &application));
    bundle.give_back(session);
    bundle.clean();
}
