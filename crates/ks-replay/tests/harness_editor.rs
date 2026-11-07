//! The model editor of the replays asked directly: what it records of a correction, and how it fails when told to.

use ks_backend::{BackendError, Injection, InputBackend as _, KeyEvent, SelectedText, SelectionError};
use ks_replay::harness::{Backend, Correction, FakeClock};

fn key(characters: [&str; 2], group: i32) -> KeyEvent {
    KeyEvent {
        pressed: true,
        key_name: characters[0].to_owned(),
        character: characters[usize::try_from(group).expect("a layout")].to_owned(),
        characters: characters.iter().map(|text| (*text).to_owned()).collect(),
        group,
        ..KeyEvent::default()
    }
}

#[test]
fn a_correction_is_recorded_with_the_key_it_came_at() {
    let backend = Backend::new("app", 0, "ghb".chars().collect());
    let strokes = [key(["g", "п"], 0), key(["h", "р"], 0), key(["b", "и"], 0)];
    backend.set_serial(7);
    let injection = Injection { strokes: &strokes, target_group: 1, boundary: None, source_group: Some(0), late: &[], trailing: &[], kept_tail: 0, erased: None };
    assert_eq!(backend.inject_correction(injection).ok(), Some(0));
    let corrections: Vec<Correction> = backend.corrections();
    assert_eq!(corrections.len(), 1);
    assert_eq!((corrections[0].event, corrections[0].before.as_str(), corrections[0].after.as_str()), (7, "ghb", "при"));
    assert_eq!((corrections[0].source_group, corrections[0].target_group, corrections[0].actual_group), (0, 1, 1));
}

#[test]
fn a_window_that_closed_refuses_what_it_is_asked() {
    let backend = Backend::new("app", 0, "ghb".chars().collect());
    backend.set_failing(true);
    let strokes = [key(["g", "п"], 0)];
    let injection = Injection { strokes: &strokes, target_group: 1, boundary: None, source_group: None, late: &[], trailing: &[], kept_tail: 0, erased: None };
    assert!(matches!(backend.inject_correction(injection), Err(BackendError::Failed(_))));
    let selected = SelectedText { text: "ghb".to_owned(), source: "clipboard".to_owned() };
    assert!(matches!(backend.replace_selection(&selected, "при", Some(1)), Err(SelectionError::Failed(_))));
    assert!(backend.try_hold_input().is_err());
    backend.set_failing(false);
    assert_eq!(backend.replace_selection(&selected, "при", None).ok(), Some(true));
    // What it says of itself.
    backend.hold_input();
    let probe = backend.probe();
    assert!(probe.available && probe.session_type == "x11" && probe.current_group == 0);
    assert!(!backend.erases_any_length());
    assert!(backend.supports_selection());
}

#[test]
fn a_new_clock_starts_where_every_replay_starts() {
    let clock = FakeClock::default();
    assert!((clock.now() - FakeClock::new().now()).abs() < f64::EPSILON);
    clock.advance(1.5);
    assert!((clock.read()() - FakeClock::new().now() - 1.5).abs() < 1e-9);
}
