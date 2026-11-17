//! The Shift keys a new installation starts with (`ks_config::installation`): either Shift switches the layout, two quick
//! taps of it do what Pause does (Shift+Pause with text selected and no word to convert), and both Shifts together turn
//! the automatic switching off or on, unless the system answers that chord itself. An installation that ran before keeps
//! its settings, where none of this is on.
#![cfg(any(not(debug_assertions), coverage))]

use std::sync::{Arc, Mutex};

use ks_backend::KeyEvent;
use ks_backend::constants::SHIFT_MASK;
use ks_config::config::SettingsStore;
use ks_config::constants::{
    DETECTION_BOTH_SHIFTS_TOGGLE, DETECTION_CONVERT_LAST_KEY, DETECTION_DOUBLE_TAP_CONVERTS, DETECTION_LAYOUT_SWITCH_KEY, ENABLED,
    NEW_INSTALLATION_BOTH_SHIFTS_TOGGLE, NEW_INSTALLATION_DOUBLE_TAP_CONVERTS, NEW_INSTALLATION_LAYOUT_SWITCH_KEY, SELECTION_COMMAND_MAX_CHARACTERS,
};
use ks_config::installation::give_new_installation_keys;
use ks_config::learning::RuleAction;
use ks_engine::constants::{BOTH_SHIFTS_MAX_SECONDS, LAYOUT_DOUBLE_TAP_SECONDS, STALE_PRESS_SECONDS};
use ks_engine::engine::{EngineSnapshot, LearningPrompt, Shared, TapAction, single_key_conflict};
use ks_replay::harness::{Bundle, KEYS, PAUSE_KEYCODE, Session};
use serde_json::json;

fn keycode(name: &str) -> u32 {
    match name {
        "Shift_L" => 150,
        "Shift_R" => 162,
        "Control_L" => 137,
        "Control_R" => 205,
        _ => 0,
    }
}

fn modifier(session: &Session, name: &str, pressed: bool) -> KeyEvent {
    KeyEvent {
        pressed,
        keycode: keycode(name),
        key_name: name.to_owned(),
        characters: vec![String::new(), String::new()],
        group: session.backend.group(),
        ..KeyEvent::default()
    }
}

fn down(session: &mut Session, name: &str) {
    let event = modifier(session, name, true);
    session.handle(event);
}

fn up(session: &mut Session, name: &str) {
    let event = modifier(session, name, false);
    session.handle(event);
}

fn tap_alone(session: &mut Session, name: &str) {
    down(session, name);
    session.advance(0.08);
    up(session, name);
}

/// Two quick taps, `gap` seconds between the first coming up and the second going down, and the timers after them.
fn double_tap(session: &mut Session, first: &str, second: &str, gap: f64) {
    tap_alone(session, first);
    session.advance(gap);
    tap_alone(session, second);
    session.idle(0.05);
}

/// Both Shifts: `first` goes down, then `second`, and they come up in the order given.
fn both_shifts(session: &mut Session, first: &str, second: &str, released_first: &str) {
    down(session, first);
    session.advance(0.05);
    down(session, second);
    session.advance(0.1);
    let released_last = if released_first == first { second } else { first };
    up(session, released_first);
    session.advance(0.02);
    up(session, released_last);
    session.idle(0.05);
}

fn letter_event(session: &Session, english: char, pressed: bool) -> KeyEvent {
    let key = KEYS.iter().find(|key| key.characters[0] == english).expect("a key of the US layout");
    let group = session.backend.group();
    let strings = key.strings();
    KeyEvent {
        pressed,
        keycode: key.keycode,
        key_name: english.to_string(),
        character: strings[usize::try_from(group).expect("a layout of the two")].clone(),
        characters: strings,
        group,
        state: if key.shift { SHIFT_MASK } else { 0 },
        ..KeyEvent::default()
    }
}

fn type_keys(session: &mut Session, english: &str) {
    for character in english.chars() {
        let event = letter_event(session, character, true);
        session.tap(event);
    }
}

fn pause(session: &mut Session) {
    let group = session.backend.group();
    session.tap(KeyEvent { pressed: true, keycode: PAUSE_KEYCODE, key_name: "Pause".to_owned(), characters: vec![String::new(), String::new()], group, ..KeyEvent::default() });
}

/// No early switch and no correction at a pause: only the keys convert a word here.
fn quiet(session: &Session) {
    session.core.settings.set("detection.early_switch", json!(false), false).expect("in memory");
    session.core.settings.set("detection.correct_on_pause", json!(false), false).expect("in memory");
}

/// The keys of a new installation, in memory: the file of the settings is shared by the sessions of a bundle.
fn new_keys(settings: &SettingsStore) {
    settings.write(DETECTION_LAYOUT_SWITCH_KEY, NEW_INSTALLATION_LAYOUT_SWITCH_KEY.to_owned(), false).expect("in memory");
    settings.write(DETECTION_DOUBLE_TAP_CONVERTS, NEW_INSTALLATION_DOUBLE_TAP_CONVERTS, false).expect("in memory");
    settings.write(DETECTION_BOTH_SHIFTS_TOGGLE, NEW_INSTALLATION_BOTH_SHIFTS_TOGGLE, false).expect("in memory");
}

/// A field in EN with the keys of a new installation, the early switch and the correction at a pause off (the keys come
/// in bursts here, and a word converted while the taps wait would be converted by neither) and the technical log on, so
/// that every record of the keys is written.
fn session(bundle: &mut Bundle) -> Session {
    let mut session = bundle.session("firefox", 0, Vec::new());
    quiet(&session);
    session.core.settings.set("diagnostics.technical_logging", json!(true), false).expect("in memory");
    new_keys(&session.core.settings);
    session.core.shared.refresh_hook_settings();
    session
}

/// A session that saved its settings (the toggle does) leaves no file for the next one.
fn close(bundle: &mut Bundle, session: Session) {
    let _ = std::fs::remove_file(&session.core.settings.path);
    bundle.give_back(session);
}

fn text(session: &Session) -> String {
    session.backend.text().iter().collect()
}

#[test]
fn either_shift_switches_the_layout_and_a_key_has_one_role() {
    let mut bundle = Bundle::load(None);
    let mut field = session(&mut bundle);
    assert_eq!((field.core.tap_action("Shift_L"), field.core.tap_action("Shift_R"), field.core.tap_action("Control_L")), (Some(TapAction::SwitchLayout), Some(TapAction::SwitchLayout), None));
    tap_alone(&mut field, "Shift_L");
    assert_eq!(field.backend.group(), 1);
    // Longer than a double tap apart: the second tap switches back.
    field.idle(LAYOUT_DOUBLE_TAP_SECONDS + 0.5);
    tap_alone(&mut field, "Shift_R");
    assert_eq!(field.backend.group(), 0);
    // A Shift named for the conversion key while either Shift switches: the key keeps its older meaning, the switch.
    field.core.settings.write(DETECTION_CONVERT_LAST_KEY, "Shift_L".to_owned(), false).expect("in memory");
    assert_eq!((field.core.convert_last_key(), field.core.tap_action("Shift_L")), (None, Some(TapAction::SwitchLayout)));
    // The settings windows refuse either Shift where the conversion key is a Shift, and a Shift where either Shift switches,
    // naming the key the other setting holds.
    let refused = single_key_conflict(&field.core.settings, DETECTION_CONVERT_LAST_KEY.path(), "Shift_R").expect("either Shift has it");
    assert!(refused.starts_with("Любой Shift уже выбран в настройке «Переключать раскладку одной клавишей»"), "{refused}");
    assert_eq!(single_key_conflict(&field.core.settings, DETECTION_CONVERT_LAST_KEY.path(), "Control_R"), None);
    field.core.settings.write(DETECTION_LAYOUT_SWITCH_KEY, String::new(), false).expect("in memory");
    let refused = single_key_conflict(&field.core.settings, DETECTION_LAYOUT_SWITCH_KEY.path(), "Shift").expect("the left Shift converts");
    assert!(refused.starts_with("Левый Shift уже выбран в настройке «Преобразовать последнее слово одной клавишей»"), "{refused}");
    assert_eq!(single_key_conflict(&field.core.settings, DETECTION_LAYOUT_SWITCH_KEY.path(), "Shift_R"), None);
    bundle.give_back(field);
    bundle.clean();
}

/// `typed` in EN, then Pause or a double tap of `first` and `second`: the text, the layout, and whether a rule is offered.
fn converted(bundle: &mut Bundle, typed: &str, double: Option<(&str, &str)>) -> (String, i32, bool) {
    let mut field = session(bundle);
    type_keys(&mut field, typed);
    field.idle(0.05);
    match double {
        Some((first, second)) => double_tap(&mut field, first, second, 0.1),
        None => pause(&mut field),
    }
    field.idle(0.05);
    let outcome = (text(&field), field.backend.group(), field.core.shared.learning_prompt().is_some());
    bundle.give_back(field);
    outcome
}

#[test]
fn a_double_tap_does_what_pause_does() {
    let mut bundle = Bundle::load(None);
    // The word being typed, a finished word, an automatic correction just made (given back), nothing at all.
    for (typed, expected, group) in [("ghbdtn", "привет", 1), ("hello ", "руддщ ", 1), ("ghbdtn ", "ghbdtn ", 0), ("", "", 1)] {
        let by_pause = converted(&mut bundle, typed, None);
        assert_eq!(by_pause, (expected.to_owned(), group, false), "{typed:?} by Pause");
        for keys in [("Shift_L", "Shift_L"), ("Shift_R", "Shift_R"), ("Shift_L", "Shift_R")] {
            assert_eq!(converted(&mut bundle, typed, Some(keys)), by_pause, "{typed:?} by {keys:?}");
        }
    }
    // The layout the conversion leaves is booked as Pause books it.
    let mut paused = session(&mut bundle);
    type_keys(&mut paused, "ghbdtn");
    pause(&mut paused);
    paused.idle(0.05);
    let booked = (paused.core.manual_layout_group, paused.core.manual_layout_source.clone());
    bundle.give_back(paused);
    let mut field = session(&mut bundle);
    type_keys(&mut field, "ghbdtn");
    double_tap(&mut field, "Shift_L", "Shift_L", 0.1);
    assert_eq!((text(&field), field.backend.group()), ("привет".to_owned(), 1));
    assert_eq!((field.core.manual_layout_group, field.core.manual_layout_source.clone()), booked);
    // Two double taps: the word converted, then back, and no rule is offered as for two presses of Pause.
    double_tap(&mut field, "Shift_L", "Shift_L", 0.1);
    assert_eq!((text(&field), field.backend.group(), field.core.shared.learning_prompt().is_some()), ("ghbdtn".to_owned(), 0, false));
    bundle.give_back(field);
    bundle.clean();
}

#[test]
fn two_taps_that_are_no_double_tap_switch_twice() {
    let mut bundle = Bundle::load(None);
    let outcome = |field: &Session| (text(field), field.backend.group());
    // The double tap off, the second tap too late, a letter between: each tap switches, the word stays.
    let mut off = session(&mut bundle);
    off.core.settings.write(DETECTION_DOUBLE_TAP_CONVERTS, false, false).expect("in memory");
    type_keys(&mut off, "ghbdtn");
    double_tap(&mut off, "Shift_L", "Shift_L", 0.1);
    assert_eq!(outcome(&off), ("ghbdtn".to_owned(), 0));
    bundle.give_back(off);
    let mut late = session(&mut bundle);
    type_keys(&mut late, "ghbdtn");
    double_tap(&mut late, "Shift_L", "Shift_L", LAYOUT_DOUBLE_TAP_SECONDS);
    assert_eq!(outcome(&late), ("ghbdtn".to_owned(), 0));
    bundle.give_back(late);
    // A Shift held with a letter is no second tap: a capital in the layout the tap switched to.
    let mut capital = session(&mut bundle);
    tap_alone(&mut capital, "Shift_L");
    down(&mut capital, "Shift_L");
    let letter = KeyEvent { state: SHIFT_MASK, ..letter_event(&capital, 'H', true) };
    capital.tap(letter);
    up(&mut capital, "Shift_L");
    capital.idle(0.05);
    assert_eq!(outcome(&capital), ("Р".to_owned(), 1));
    bundle.give_back(capital);
    // A click between the taps.
    let mut clicked = session(&mut bundle);
    tap_alone(&mut clicked, "Shift_L");
    clicked.pointer();
    tap_alone(&mut clicked, "Shift_L");
    assert_eq!(clicked.backend.group(), 0);
    bundle.give_back(clicked);
    // A layout of neither group: the first tap switches nothing, and the second is a tap of its own.
    let mut unknown = session(&mut bundle);
    unknown.backend.set_group(2);
    double_tap(&mut unknown, "Shift_L", "Shift_L", 0.1);
    assert_eq!(unknown.backend.group(), 2);
    bundle.give_back(unknown);
    bundle.clean();
}

#[test]
fn a_double_tap_takes_back_the_switch_the_system_made() {
    let mut bundle = Bundle::load(None);
    // The system switches on each Shift itself (an XKB option): the first switch was its own, and so is the way back.
    let mut system = session(&mut bundle);
    type_keys(&mut system, "ghbdtn");
    for _ in 0..2 {
        down(&mut system, "Shift_L");
        let switched = 1 - system.backend.group();
        system.backend.set_group(switched);
        system.advance(0.05);
        up(&mut system, "Shift_L");
        system.advance(0.1);
    }
    system.idle(0.05);
    assert_eq!((text(&system), system.backend.group()), ("привет".to_owned(), 1));
    bundle.give_back(system);
    // A window that does not confirm the way back: the word stays, and the error is said.
    let mut stuck = session(&mut bundle);
    type_keys(&mut stuck, "ghbdtn");
    tap_alone(&mut stuck, "Shift_L");
    stuck.backend.set_unconfirmed(true);
    stuck.advance(0.1);
    tap_alone(&mut stuck, "Shift_L");
    stuck.idle(0.05);
    assert_eq!(text(&stuck), "ghbdtn");
    assert_eq!(stuck.core.shared.snapshot().last_error, "the window did not confirm the layout");
    stuck.backend.set_unconfirmed(false);
    bundle.give_back(stuck);
    bundle.clean();
}

#[test]
fn a_double_tap_converts_the_selection_when_no_word_is_typed() {
    let mut bundle = Bundle::load(None);
    let notices: Arc<Mutex<Vec<String>>> = Arc::default();
    // Selected after a click, which lets the typed word go: the selection is converted, as Shift+Pause converts it.
    let mut selected = session(&mut bundle);
    type_keys(&mut selected, "hello");
    selected.pointer();
    selected.backend.set_selection(Some("ghbdtn"));
    double_tap(&mut selected, "Shift_L", "Shift_L", 0.1);
    assert_eq!((selected.backend.selection().as_deref(), selected.backend.group(), text(&selected)), (Some("привет"), 1, "hello".to_owned()));
    bundle.give_back(selected);
    // A word typed: the word is converted, whatever the window says is selected (an address bar completing the word).
    let mut typed = session(&mut bundle);
    typed.backend.set_selection(Some("tn.ru"));
    type_keys(&mut typed, "ghbd");
    double_tap(&mut typed, "Shift_L", "Shift_L", 0.1);
    assert_eq!((typed.backend.selection().as_deref(), text(&typed)), (Some("tn.ru"), "привет".chars().take(4).collect::<String>()));
    bundle.give_back(typed);
    // Nothing selected, or the selection not read (KeySwitch paused): what Pause does with nothing to convert.
    for paused in [false, true] {
        let mut empty = session(&mut bundle);
        empty.core.settings.write(ENABLED, !paused, false).expect("in memory");
        empty.backend.set_selection((paused).then_some("ghbdtn"));
        let sink = notices.clone();
        empty.core.shared.subscribe_selection_notices(Arc::new(move |notice: &str| sink.lock().unwrap().push(notice.to_owned())));
        double_tap(&mut empty, "Shift_L", "Shift_L", 0.1);
        assert_eq!((empty.backend.group(), empty.backend.selection().as_deref()), (1, paused.then_some("ghbdtn")), "paused {paused}");
        bundle.give_back(empty);
    }
    assert!(notices.lock().unwrap().is_empty(), "{notices:?}");
    // A selection Shift+Pause refuses (too long): refused the same way, said where the person looks, the layout as it was.
    let mut long = session(&mut bundle);
    let sink = notices.clone();
    long.core.shared.subscribe_selection_notices(Arc::new(move |notice: &str| sink.lock().unwrap().push(notice.to_owned())));
    let too_long = "a".repeat(SELECTION_COMMAND_MAX_CHARACTERS + 1);
    long.backend.set_selection(Some(&too_long));
    double_tap(&mut long, "Shift_L", "Shift_L", 0.1);
    assert_eq!((long.backend.group(), long.backend.selection()), (0, Some(too_long)));
    assert_eq!(notices.lock().unwrap().len(), 1);
    bundle.give_back(long);
    bundle.clean();
}

/// Both Shifts, `between` done while both are down, in a field `before` set up: whether the automatic switching is on
/// after them, what the platforms were told, and the layout.
fn chord(bundle: &mut Bundle, before: &dyn Fn(&mut Session), between: &dyn Fn(&mut Session)) -> (bool, Vec<bool>, i32) {
    let mut field = session(bundle);
    before(&mut field);
    let told: Arc<Mutex<Vec<bool>>> = Arc::default();
    let sink = told.clone();
    field.core.shared.subscribe_auto_switch(Arc::new(move |enabled| sink.lock().unwrap().push(enabled)));
    down(&mut field, "Shift_L");
    field.advance(0.05);
    down(&mut field, "Shift_R");
    between(&mut field);
    up(&mut field, "Shift_R");
    field.advance(0.02);
    up(&mut field, "Shift_L");
    field.idle(0.05);
    let outcome = (field.core.settings.read(ENABLED).unwrap_or(true), told.lock().unwrap().clone(), field.backend.group());
    close(bundle, field);
    outcome
}

#[test]
fn both_shifts_turn_the_automatic_switching_off_and_on() {
    let mut bundle = Bundle::load(None);
    let nothing = |_: &mut Session| {};
    // Off, told to the platforms, the layout where it was.
    assert_eq!(chord(&mut bundle, &nothing, &nothing), (false, vec![false], 0));
    // Off, a word typed next stays as typed; on again, in any order of the keys, and the next word is corrected.
    let mut field = session(&mut bundle);
    let told: Arc<Mutex<Vec<bool>>> = Arc::default();
    let sink = told.clone();
    field.core.shared.subscribe_auto_switch(Arc::new(move |enabled| sink.lock().unwrap().push(enabled)));
    both_shifts(&mut field, "Shift_L", "Shift_R", "Shift_L");
    type_keys(&mut field, "ghbdtn ");
    assert_eq!((field.core.settings.read(ENABLED), text(&field)), (Some(false), "ghbdtn ".to_owned()));
    both_shifts(&mut field, "Shift_R", "Shift_L", "Shift_R");
    type_keys(&mut field, "ghbdtn ");
    assert_eq!((field.core.settings.read(ENABLED), text(&field)), (Some(true), "ghbdtn привет ".to_owned()));
    assert_eq!(told.lock().unwrap().clone(), [false, true]);
    close(&mut bundle, field);
    bundle.clean();
}

#[test]
fn both_shifts_are_no_chord_with_anything_else_and_are_left_to_the_system() {
    let mut bundle = Bundle::load(None);
    let nothing = |_: &mut Session| {};
    let untouched = (true, Vec::new(), 0);
    // The setting off (an installation that ran before), held too long, a letter, a click or a wheel between.
    let off = |field: &mut Session| field.core.settings.write(DETECTION_BOTH_SHIFTS_TOGGLE, false, false).expect("in memory");
    assert_eq!(chord(&mut bundle, &off, &nothing), untouched);
    assert_eq!(chord(&mut bundle, &nothing, &|field| field.advance(BOTH_SHIFTS_MAX_SECONDS)), untouched);
    let letter = |field: &mut Session| {
        let event = KeyEvent { state: SHIFT_MASK, ..letter_event(field, 'a', true) };
        field.tap(event);
    };
    assert_eq!(chord(&mut bundle, &nothing, &letter), untouched);
    assert_eq!(chord(&mut bundle, &nothing, &|field| field.pointer()), untouched);
    assert_eq!(chord(&mut bundle, &nothing, &|field| field.handle(KeyEvent::wheel(1))), untouched);
    // A third press of a Shift: the first one released and pressed again.
    let again = |field: &mut Session| {
        up(field, "Shift_L");
        down(field, "Shift_L");
    };
    assert_eq!(chord(&mut bundle, &nothing, &again), untouched);
    // A Ctrl held before: no chord begins.
    assert_eq!(chord(&mut bundle, &|field| down(field, "Control_L"), &nothing), untouched);
    // The system answers both Shifts: an option it reports, or the layout it switched while they were down.
    assert_eq!(chord(&mut bundle, &|field| field.backend.set_both_shifts_taken(true), &nothing), untouched);
    assert_eq!(chord(&mut bundle, &nothing, &|field| field.backend.set_group(1)), (true, Vec::new(), 1));
    bundle.clean();
}

#[test]
fn an_installation_that_ran_before_has_none_of_the_keys() {
    let mut bundle = Bundle::load(None);
    // The defaults: a tap, a double tap and both Shifts change nothing.
    let mut earlier = bundle.session("firefox", 0, Vec::new());
    quiet(&earlier);
    type_keys(&mut earlier, "ghbdtn");
    double_tap(&mut earlier, "Shift_L", "Shift_L", 0.1);
    both_shifts(&mut earlier, "Shift_L", "Shift_R", "Shift_L");
    assert_eq!((text(&earlier), earlier.backend.group(), earlier.core.settings.read(ENABLED)), ("ghbdtn".to_owned(), 0, Some(true)));
    // The keys of a new installation, written as its first start writes them: the same keys convert.
    give_new_installation_keys(&earlier.core.settings).expect("saved");
    double_tap(&mut earlier, "Shift_L", "Shift_L", 0.1);
    assert_eq!((text(&earlier), earlier.backend.group()), ("привет".to_owned(), 1));
    close(&mut bundle, earlier);
    bundle.clean();
}

#[test]
fn a_turned_wheel_is_queued_while_only_both_shifts_are_set() {
    let mut bundle = Bundle::load(None);
    let session = bundle.session("firefox", 0, Vec::new());
    let settings = session.core.settings.clone();
    let shared = Shared::new(settings.clone(), session.core.backend.clone(), session.core.clock.clone(), 2, EngineSnapshot::default());
    shared.enqueue(KeyEvent::wheel(1));
    assert_eq!(shared.qsize(), 0);
    settings.write(DETECTION_BOTH_SHIFTS_TOGGLE, true, false).expect("in memory");
    shared.refresh_hook_settings();
    shared.enqueue(KeyEvent::wheel(1));
    assert_eq!(shared.qsize(), 1);
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn both_shifts_with_the_window_gone_are_recovered_from() {
    let mut bundle = Bundle::load(None);
    let mut gone = session(&mut bundle);
    gone.recovers = true;
    down(&mut gone, "Shift_L");
    down(&mut gone, "Shift_R");
    up(&mut gone, "Shift_R");
    gone.backend.set_failing(true);
    up(&mut gone, "Shift_L");
    gone.backend.set_failing(false);
    assert_eq!(gone.core.settings.read(ENABLED), Some(true));
    assert!(!gone.core.shared.snapshot().last_error.is_empty());
    close(&mut bundle, gone);
    bundle.clean();
}

#[test]
fn a_double_tap_with_the_window_gone_is_recovered_from() {
    let mut bundle = Bundle::load(None);
    let mut gone = session(&mut bundle);
    gone.recovers = true;
    type_keys(&mut gone, "ghbdtn");
    tap_alone(&mut gone, "Shift_L");
    gone.advance(0.1);
    down(&mut gone, "Shift_L");
    gone.backend.set_failing(true);
    up(&mut gone, "Shift_L");
    gone.backend.set_failing(false);
    assert_eq!(text(&gone), "ghbdtn");
    assert!(!gone.core.shared.snapshot().last_error.is_empty());
    bundle.give_back(gone);
    bundle.clean();
}

#[test]
fn a_double_tap_answers_an_offered_rule_as_another_key_does() {
    let mut bundle = Bundle::load(None);
    let mut offered = session(&mut bundle);
    type_keys(&mut offered, "ghbdtn");
    offered.core.shared.show_learning_prompt(LearningPrompt {
        source_group: 0,
        target_group: 1,
        original: "hello".to_owned(),
        replacement: "руддщ".to_owned(),
        application: "firefox".to_owned(),
        action: RuleAction::Convert,
    });
    double_tap(&mut offered, "Shift_L", "Shift_L", 0.1);
    assert_eq!((text(&offered), offered.core.shared.learning_prompt()), ("привет".to_owned(), None));
    bundle.give_back(offered);
    bundle.clean();
}

#[test]
fn a_shift_pressed_again_after_its_release_was_lost_is_no_chord() {
    let mut bundle = Bundle::load(None);
    let mut lost = session(&mut bundle);
    let told: Arc<Mutex<Vec<bool>>> = Arc::default();
    let sink = told.clone();
    lost.core.shared.subscribe_auto_switch(Arc::new(move |enabled| sink.lock().unwrap().push(enabled)));
    down(&mut lost, "Shift_L");
    // The release never came; the engine forgets the press as it forgets any stale one, and the key goes down again.
    lost.advance(STALE_PRESS_SECONDS + 1.0);
    let now = lost.core.monotonic();
    lost.core.prune_stale_presses(now, STALE_PRESS_SECONDS, None);
    lost.core.modifier_keycodes.clear();
    down(&mut lost, "Shift_L");
    up(&mut lost, "Shift_L");
    lost.idle(0.05);
    assert_eq!((lost.core.settings.read(ENABLED), told.lock().unwrap().clone()), (Some(true), Vec::new()));
    close(&mut bundle, lost);
    bundle.clean();
}
