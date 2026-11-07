//! A modifier pressed and released alone switches the layout (`detection.layout_switch_key`): what is a tap and what is
//! not, what the engine books for it, and that nothing changes while no key is set.
#![cfg(any(not(debug_assertions), coverage))]

use std::sync::Arc;

use ks_backend::KeyEvent;
use ks_backend::constants::{CONTROL_MASK, SHIFT_MASK};
use ks_config::learning::RuleAction;
use ks_engine::constants::{ENGINE_EVENT_QUEUE_MAX_SIZE, LAYOUT_TAP_MAX_SECONDS, STALE_PRESS_SECONDS};
use ks_engine::engine::{CorrectionPlan, EngineSnapshot, FieldWait, LAYOUT_TAP_SOURCE, LearningPrompt, QueueItem, Shared};
use ks_replay::harness::{Bundle, KEYS, PAUSE_KEYCODE, Session};
use serde_json::json;

/// Keycodes of the modifiers, apart from those of the letter keys (`KEYS`).
fn keycode(name: &str) -> u32 {
    match name {
        "Shift_L" => 150,
        "Shift_R" => 162,
        "Control_L" => 137,
        "Control_R" => 205,
        "Alt_L" => 164,
        "Alt_R" => 208,
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

/// A quick press and release of the key alone.
fn tap_alone(session: &mut Session, name: &str) {
    down(session, name);
    session.advance(0.08);
    up(session, name);
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

/// The key that types `character`, in the layout it types it in.
fn typed(character: char) -> KeyEvent {
    let (physical, group) = KEYS
        .iter()
        .find_map(|key| Some((key, key.characters.iter().position(|typed| *typed == character)?)))
        .expect("a key types it");
    KeyEvent {
        pressed: true,
        keycode: physical.keycode,
        key_name: character.to_string(),
        character: character.to_string(),
        characters: physical.strings(),
        group: i32::try_from(group).expect("two layouts"),
        state: if physical.shift { SHIFT_MASK } else { 0 },
        ..KeyEvent::default()
    }
}

/// Keys by their place on a US keyboard, in whatever layout is on.
fn type_keys(session: &mut Session, english: &str) {
    for character in english.chars() {
        let event = letter_event(session, character, true);
        session.tap(event);
    }
}

fn choose(session: &mut Session, key: &str) {
    session.core.settings.set("detection.layout_switch_key", json!(key), false).expect("in memory");
}

/// A field in EN with `key` as the layout switch key. The early switch is off: the keys come in bursts here, and a word
/// switched while it is typed would take the layout on its own.
fn session(bundle: &mut Bundle, key: &str) -> Session {
    let mut session = bundle.session("firefox", 0, Vec::new());
    session.core.settings.set("detection.early_switch", json!(false), false).expect("in memory");
    choose(&mut session, key);
    session
}

#[test]
fn a_lone_tap_switches_the_layout_once_as_a_keyboard_switch() {
    let mut bundle = Bundle::load(None);
    // 1. A plain tap: the layout switches, booked as the person's switch, and what Pause and the prompt had stays.
    let mut tapped = session(&mut bundle, "Shift_L");
    type_keys(&mut tapped, "hello ");
    assert!(tapped.core.last_committed.is_some() && !tapped.core.last_committed_stale);
    let prompt = LearningPrompt {
        source_group: 0,
        target_group: 1,
        original: "hello".to_owned(),
        replacement: "руддщ".to_owned(),
        application: "firefox".to_owned(),
        action: RuleAction::Convert,
    };
    tapped.core.shared.show_learning_prompt(prompt.clone());
    tapped.core.convert_press_at = tapped.core.monotonic();
    tap_alone(&mut tapped, "Shift_L");
    assert_eq!((tapped.backend.group(), tapped.core.shared.current_group()), (1, 1));
    assert_eq!((tapped.core.manual_layout_group, tapped.core.manual_layout_source.as_str()), (Some(1), LAYOUT_TAP_SOURCE));
    assert_eq!(tapped.core.engine_switch_group, Some(1));
    assert!(tapped.core.last_committed.is_some() && !tapped.core.last_committed_stale);
    assert_eq!(tapped.core.shared.learning_prompt(), Some(prompt));
    assert_eq!(tapped.core.convert_press_at, 0.0);
    assert!(tapped.core.layout_tap.is_none());
    // Two taps: the second takes the layout back.
    tap_alone(&mut tapped, "Shift_L");
    assert_eq!(tapped.backend.group(), 0);
    bundle.give_back(tapped);
    // The same with the protection of the next word turned off: the layout switches, nothing is protected.
    let mut unprotected = session(&mut bundle, "Shift_L");
    unprotected.core.settings.set("detection.respect_manual_layout", json!(false), false).expect("in memory");
    tap_alone(&mut unprotected, "Shift_L");
    assert_eq!((unprotected.backend.group(), unprotected.core.manual_layout_group), (1, None));
    assert_eq!(unprotected.core.shared.current_group(), 1);
    bundle.give_back(unprotected);
    // 16. With the press carrying Shift in its state (Windows, macOS and Wayland count the key itself) or not (X11).
    for state in [0, SHIFT_MASK] {
        let mut stated = session(&mut bundle, "Shift_L");
        let press = KeyEvent { state, ..modifier(&stated, "Shift_L", true) };
        stated.handle(press);
        stated.advance(0.05);
        let release = KeyEvent { state, ..modifier(&stated, "Shift_L", false) };
        stated.handle(release);
        assert_eq!(stated.backend.group(), 1, "state {state}");
        bundle.give_back(stated);
    }
    // A pair replayed at once after the hold of a correction (Windows replays held keys, not marked as its own): a tap.
    let mut replayed = session(&mut bundle, "Shift_L");
    down(&mut replayed, "Shift_L");
    up(&mut replayed, "Shift_L");
    assert_eq!(replayed.backend.group(), 1);
    bundle.give_back(replayed);
    bundle.clean();
}

#[test]
fn a_tap_is_the_chosen_key_alone_and_quick() {
    let mut bundle = Bundle::load(None);
    // 16. No key set, or a key not offered: nothing switches. Each of the four keys switches only by itself.
    for (setting, tapped, switches) in [
        (json!(""), "Shift_L", false),
        (json!("Alt_L"), "Alt_L", false),
        (json!(5), "Shift_L", false),
        (json!("Shift_L"), "Shift_R", false),
        (json!("Shift_R"), "Shift_R", true),
        (json!("Shift_R"), "Shift_L", false),
        (json!("Control_L"), "Control_L", true),
        (json!("Control_L"), "Control_R", false),
        (json!("Control_R"), "Control_R", true),
        (json!("Control_R"), "Shift_R", false),
    ] {
        let mut keyed = bundle.session("firefox", 0, Vec::new());
        keyed.core.settings.set("detection.layout_switch_key", setting.clone(), false).expect("in memory");
        tap_alone(&mut keyed, tapped);
        assert_eq!(keyed.backend.group(), i32::from(switches), "{setting} by {tapped}");
        assert_eq!(keyed.core.manual_layout_group.is_some(), switches, "{setting} by {tapped}");
        bundle.give_back(keyed);
    }
    // 2. Held longer than the window: no tap.
    let mut slow = session(&mut bundle, "Shift_L");
    down(&mut slow, "Shift_L");
    slow.advance(LAYOUT_TAP_MAX_SECONDS + 0.1);
    up(&mut slow, "Shift_L");
    assert_eq!(slow.backend.group(), 0);
    bundle.give_back(slow);
    // 3. Repeats of the held key: the time counts from the first press, so a short hold switches and a long one does not.
    for (gap, switches) in [(0.1, true), (0.3, false)] {
        let mut repeating = session(&mut bundle, "Shift_L");
        down(&mut repeating, "Shift_L");
        for _ in 0..2 {
            repeating.advance(gap);
            down(&mut repeating, "Shift_L");
        }
        repeating.advance(0.05);
        up(&mut repeating, "Shift_L");
        assert_eq!(repeating.backend.group(), i32::from(switches), "repeats {gap} s apart");
        bundle.give_back(repeating);
    }
    // 5. Shift, a letter, Shift: a capital, no tap.
    let mut capital = session(&mut bundle, "Shift_L");
    down(&mut capital, "Shift_L");
    let letter = KeyEvent { state: SHIFT_MASK, ..letter_event(&capital, 'H', true) };
    capital.tap(letter);
    up(&mut capital, "Shift_L");
    assert_eq!(capital.backend.group(), 0);
    assert_eq!(capital.backend.text(), vec!['H']);
    bundle.give_back(capital);
    // Ctrl+C with the chosen Ctrl.
    let mut copy = session(&mut bundle, "Control_L");
    down(&mut copy, "Control_L");
    let letter = KeyEvent { state: CONTROL_MASK, ..letter_event(&copy, 'c', true) };
    copy.tap(letter);
    up(&mut copy, "Control_L");
    assert_eq!(copy.backend.group(), 0);
    bundle.give_back(copy);
    // 6. Shift held over a click or a turn of the wheel.
    for between in ["Pointer", "Wheel"] {
        let mut held = session(&mut bundle, "Shift_L");
        down(&mut held, "Shift_L");
        if between == "Pointer" {
            held.pointer();
        } else {
            held.handle(KeyEvent::wheel(0));
        }
        up(&mut held, "Shift_L");
        assert_eq!(held.backend.group(), 0, "{between}");
        bundle.give_back(held);
    }
    // A turned wheel moves no caret: the word being typed stays whole.
    let mut scrolled = session(&mut bundle, "Shift_L");
    type_keys(&mut scrolled, "ghbd");
    scrolled.handle(KeyEvent::wheel(0));
    assert_eq!(scrolled.core.strokes.len(), 4);
    bundle.give_back(scrolled);
    // AltGr on Windows: a Left Ctrl the system makes up, then Right Alt.
    let mut altgr = session(&mut bundle, "Control_L");
    down(&mut altgr, "Control_L");
    down(&mut altgr, "Alt_R");
    up(&mut altgr, "Alt_R");
    up(&mut altgr, "Control_L");
    assert_eq!(altgr.backend.group(), 0);
    bundle.give_back(altgr);
    // 8. Shift pressed while Pause is down: Pause converts the word, and the Shift that came up after it is no tap.
    let mut converting = session(&mut bundle, "Shift_L");
    type_keys(&mut converting, "ghbdtn");
    let pause = |pressed: bool| KeyEvent {
        pressed,
        keycode: PAUSE_KEYCODE,
        key_name: "Pause".to_owned(),
        characters: vec![String::new(), String::new()],
        ..KeyEvent::default()
    };
    converting.handle(pause(true));
    down(&mut converting, "Shift_L");
    converting.handle(pause(false));
    up(&mut converting, "Shift_L");
    converting.idle(0.05);
    assert_eq!(converting.backend.text().iter().collect::<String>(), "привет");
    assert_eq!(converting.backend.group(), 1);
    bundle.give_back(converting);
    bundle.clean();
}

#[test]
fn the_system_switches_with_another_key_held() {
    let mut bundle = Bundle::load(None);
    // 4. Alt+Shift and Ctrl+Shift (Windows), both Shifts (GNOME): the other key went down first. Nothing switches here, and
    // the switch the system made is still the person's.
    for (chosen, first) in [("Shift_L", "Alt_L"), ("Shift_L", "Control_L"), ("Shift_L", "Shift_R"), ("Control_L", "Shift_L")] {
        let mut chord = session(&mut bundle, chosen);
        down(&mut chord, first);
        down(&mut chord, chosen);
        up(&mut chord, chosen);
        up(&mut chord, first);
        assert_eq!(chord.backend.group(), 0, "{first}+{chosen}");
        chord.backend.set_group(1);
        chord.idle(0.1);
        assert_eq!((chord.core.manual_layout_group, chord.core.manual_layout_source.as_str()), (Some(1), "poll"), "{first}+{chosen}");
        bundle.give_back(chord);
    }
    // The chosen key first, the other one released before it: still no tap.
    let mut both = session(&mut bundle, "Shift_L");
    down(&mut both, "Shift_L");
    down(&mut both, "Shift_R");
    up(&mut both, "Shift_R");
    up(&mut both, "Shift_L");
    assert_eq!(both.backend.group(), 0);
    bundle.give_back(both);
    // 7. A key whose release was lost stays down in the engine's books and holds the tap back, however old, until the
    // engine forgets it as it forgets any stale press.
    let mut stuck = session(&mut bundle, "Shift_L");
    down(&mut stuck, "Alt_L");
    stuck.advance(STALE_PRESS_SECONDS + 1.0);
    tap_alone(&mut stuck, "Shift_L");
    assert_eq!(stuck.backend.group(), 0);
    let now = stuck.core.monotonic();
    stuck.core.prune_stale_presses(now, STALE_PRESS_SECONDS, None);
    tap_alone(&mut stuck, "Shift_L");
    assert_eq!(stuck.backend.group(), 1);
    bundle.give_back(stuck);
    // 12. The system switched on this very key (an XKB option): the switch is booked, not made again.
    let mut system = session(&mut bundle, "Shift_L");
    down(&mut system, "Shift_L");
    system.backend.set_group(1);
    system.advance(0.05);
    up(&mut system, "Shift_L");
    assert_eq!(system.backend.group(), 1);
    assert_eq!((system.core.manual_layout_group, system.core.manual_layout_source.as_str()), (Some(1), LAYOUT_TAP_SOURCE));
    assert_eq!(system.core.engine_switch_group, None);
    bundle.give_back(system);
    // A layout of neither group: no other layout to go to.
    let mut unknown = session(&mut bundle, "Shift_L");
    unknown.backend.set_group(2);
    tap_alone(&mut unknown, "Shift_L");
    assert_eq!((unknown.backend.group(), unknown.core.manual_layout_group), (2, None));
    bundle.give_back(unknown);
    // 13. A switch the window did not confirm is the engine's when it comes: it protects nothing.
    let mut unconfirmed = session(&mut bundle, "Shift_L");
    unconfirmed.backend.set_unconfirmed(true);
    tap_alone(&mut unconfirmed, "Shift_L");
    assert_eq!((unconfirmed.backend.group(), unconfirmed.core.engine_switch_group, unconfirmed.core.manual_layout_group), (0, Some(1), None));
    assert_eq!(unconfirmed.core.shared.snapshot().last_error, "the window did not confirm the layout");
    unconfirmed.backend.set_unconfirmed(false);
    unconfirmed.backend.set_group(1);
    unconfirmed.idle(0.1);
    assert_eq!((unconfirmed.core.shared.current_group(), unconfirmed.core.manual_layout_group), (1, None));
    bundle.give_back(unconfirmed);
    // The window gone before the key came up: the engine recovers as from any failure of the backend.
    let mut gone = session(&mut bundle, "Shift_L");
    gone.recovers = true;
    down(&mut gone, "Shift_L");
    gone.backend.set_failing(true);
    up(&mut gone, "Shift_L");
    assert_eq!(gone.core.manual_layout_group, None);
    assert!(!gone.core.shared.snapshot().last_error.is_empty());
    bundle.give_back(gone);
    bundle.clean();
}

#[test]
fn what_comes_between_the_press_and_the_release() {
    let mut bundle = Bundle::load(None);
    // 14. A key another program injected: not a tap while such input is no typing, a tap once the settings count it.
    for injected in [false, true] {
        let mut remote = session(&mut bundle, "Shift_L");
        remote.core.settings.set("detection.injected_input", json!(injected), false).expect("in memory");
        remote.core.settings_changed("detection.injected_input", &json!(injected));
        for pressed in [true, false] {
            let event = KeyEvent { foreign: true, ..modifier(&remote, "Shift_L", pressed) };
            remote.handle(event);
        }
        assert_eq!(remote.backend.group(), i32::from(injected), "injected input {injected}");
        bundle.give_back(remote);
    }
    // A local press and an injected release that is no typing: no tap.
    let mut mixed = session(&mut bundle, "Shift_L");
    down(&mut mixed, "Shift_L");
    let release = KeyEvent { foreign: true, ..modifier(&mixed, "Shift_L", false) };
    mixed.handle(release);
    assert_eq!(mixed.backend.group(), 0);
    bundle.give_back(mixed);
    // 15. The rule window took the keyboard, or another window came to the front, while the key was down.
    let mut ruled = session(&mut bundle, "Shift_L");
    down(&mut ruled, "Shift_L");
    ruled.core.shared.set_rule_editor_open(true);
    up(&mut ruled, "Shift_L");
    ruled.core.shared.set_rule_editor_open(false);
    assert_eq!(ruled.backend.group(), 0);
    bundle.give_back(ruled);
    let mut moved = session(&mut bundle, "Shift_L");
    down(&mut moved, "Shift_L");
    moved.backend.focus(2, "telegram-desktop");
    up(&mut moved, "Shift_L");
    assert_eq!(moved.backend.group(), 0);
    bundle.give_back(moved);
    // The settings changed to another key while this one was down.
    let mut rechosen = session(&mut bundle, "Shift_L");
    down(&mut rechosen, "Shift_L");
    choose(&mut rechosen, "Shift_R");
    up(&mut rechosen, "Shift_L");
    assert_eq!(rechosen.backend.group(), 0);
    bundle.give_back(rechosen);
    // Keys a backend reports the same keycode for (Windows gives both Ctrl keys one scan code): the other one ends it.
    let mut twins = session(&mut bundle, "Control_L");
    down(&mut twins, "Control_L");
    let other = KeyEvent { key_name: "Control_R".to_owned(), ..modifier(&twins, "Control_L", false) };
    twins.handle(other);
    up(&mut twins, "Control_L");
    assert_eq!(twins.backend.group(), 0);
    bundle.give_back(twins);
    bundle.clean();
}

#[test]
fn a_tap_keeps_the_words_the_engine_holds() {
    let mut bundle = Bundle::load(None);
    // 10. After a boundary: the next word stays as typed, once.
    let mut after = session(&mut bundle, "Shift_L");
    type_keys(&mut after, "hello ");
    tap_alone(&mut after, "Shift_L");
    type_keys(&mut after, "hello ");
    type_keys(&mut after, "hello ");
    assert_eq!(after.backend.text().iter().collect::<String>(), "hello руддщ hello ");
    bundle.give_back(after);
    // In the middle of a word: its next letter in the other layout lets it go, as after the system's own switch, and the
    // word begun there stays as typed.
    let mut middle = session(&mut bundle, "Shift_L");
    type_keys(&mut middle, "hel");
    tap_alone(&mut middle, "Shift_L");
    assert_eq!(middle.core.strokes.len(), 3);
    type_keys(&mut middle, "lo ");
    assert_eq!(middle.backend.text().iter().collect::<String>(), "helдщ ");
    bundle.give_back(middle);
    // 9. An early switch took the word to RU, a tap takes the layout back, and a letter typed at once in EN is the
    // person's, not a key that reached the window before the early switch.
    let mut early = session(&mut bundle, "Shift_L");
    early.backend.set_group(1);
    early.idle(0.01);
    type_keys(&mut early, "ghbd");
    early.core.early_switch_origin = Some(0);
    early.core.early_switch_at = Some(early.core.monotonic());
    tap_alone(&mut early, "Shift_L");
    assert_eq!((early.backend.group(), early.core.early_switch_at, early.core.early_switch_origin), (0, None, Some(0)));
    type_keys(&mut early, "e");
    assert!(early.core.pending.is_none());
    assert_eq!((early.core.strokes.len(), early.core.source_group), (1, 0));
    assert_eq!(early.backend.text().iter().collect::<String>(), "привe");
    bundle.give_back(early);
    // 11. A word waiting for its neighbour, and a correction waiting for its field, wait on.
    let mut found = None;
    for first in ["tot", "ещ", "yt", "lf", "да", "z", "я", "x", "ч", "gj", "по", "rfr", "как", "qr", "йк", "d", "в", "c", "с"] {
        let mut waiting = session(&mut bundle, "Shift_L");
        for character in format!("{first} ").chars() {
            let event = typed(character);
            waiting.select_layout(event.group);
            waiting.tap(event);
        }
        if waiting.core.context_waiting.is_some() {
            found = Some(waiting);
            break;
        }
        bundle.give_back(waiting);
    }
    let mut waiting = found.expect("some word waits for its neighbour");
    let before = waiting.core.context_waiting.as_ref().map(|word| word.diagnostic_id);
    let now = waiting.core.monotonic();
    let plan = Arc::new(CorrectionPlan::new(Vec::new(), None, 0, 1, "a".to_owned(), "ф".to_owned(), 1.0, "firefox"));
    waiting.core.field_wait = Some(FieldWait { plan, learning_prompt: None, deadline: now + 10.0 });
    tap_alone(&mut waiting, "Shift_L");
    assert!(waiting.core.manual_layout_group.is_some());
    assert_eq!(waiting.core.context_waiting.as_ref().map(|word| word.diagnostic_id), before);
    assert!(waiting.core.field_wait.is_some());
    bundle.give_back(waiting);
    bundle.clean();
}

#[test]
fn a_turned_wheel_is_queued_only_while_a_key_is_set() {
    let mut bundle = Bundle::load(None);
    let session = bundle.session("firefox", 0, Vec::new());
    let wheel = || KeyEvent::wheel(1);
    let key = || KeyEvent { pressed: true, keycode: 40, key_name: "a".to_owned(), character: "a".to_owned(), ..KeyEvent::default() };
    let settings = session.core.settings.clone();
    let shared = Shared::new(settings.clone(), session.core.backend.clone(), session.core.clock.clone(), 2, EngineSnapshot::default());
    // No key set: a wheel is nothing, and it is never counted as typing.
    shared.enqueue(wheel());
    assert_eq!((shared.qsize(), shared.typed_events.load(std::sync::atomic::Ordering::Relaxed)), (0, 0));
    settings.set("detection.layout_switch_key", json!("Control_L"), false).expect("in memory");
    shared.refresh_hook_settings();
    // A run of turns is queued once; a key between them makes two runs; something else before them leaves them alone.
    shared.enqueue(wheel());
    shared.enqueue(wheel());
    shared.enqueue(key());
    shared.enqueue(wheel());
    let names: Vec<String> = shared
        .queued()
        .iter()
        .map(|item| match item {
            QueueItem::Key(event) => event.key_name.clone(),
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(names, ["Wheel", "a", "Wheel"]);
    assert_eq!(shared.typed_events.load(std::sync::atomic::Ordering::Relaxed), 1);
    let _ = shared.take_queued();
    assert!(shared.put_nowait(QueueItem::Stop));
    shared.enqueue(wheel());
    assert_eq!(shared.qsize(), 2);
    // A full queue takes no wheel and is not overflowed by one.
    let _ = shared.take_queued();
    for _ in 0..ENGINE_EVENT_QUEUE_MAX_SIZE {
        assert!(shared.put_nowait(QueueItem::Stop));
    }
    shared.enqueue(wheel());
    assert_eq!(shared.qsize(), ENGINE_EVENT_QUEUE_MAX_SIZE);
    assert!(!shared.input_overflow.load(std::sync::atomic::Ordering::Relaxed));
    bundle.give_back(session);
    bundle.clean();
}
