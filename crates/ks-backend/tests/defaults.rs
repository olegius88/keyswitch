//! What a backend answers when it does not say otherwise, and how errors and field readers behave by default.

use std::sync::Arc;

use ks_backend::{BackendError, BackendProbe, FieldContext, FieldReader, FocusInfo, InputBackend, InputDelay, Injection, KeyEvent, KeyFilter, KeyListener, RefusalReason, SelectedText, SelectionError, SelectionRefused};

struct Minimal;

impl InputBackend for Minimal {
    fn start(&self, listener: KeyListener) -> Result<(), BackendError> {
        listener(KeyEvent::default());
        Ok(())
    }
    fn set_key_filter(&self, _predicate: Option<KeyFilter>) {}
    fn hold_input(&self) {}
    fn release_input(&self) -> Result<i32, BackendError> {
        Ok(0)
    }
    fn complete_action(&self, _deliver: bool) -> Result<i32, BackendError> {
        Ok(0)
    }
    fn stop(&self) {}
    fn close(&self) {}
    fn current_group(&self) -> Result<i32, BackendError> {
        Ok(0)
    }
    fn switch_group(&self, _group: i32) -> Result<(), BackendError> {
        Ok(())
    }
    fn active_application(&self) -> String {
        String::new()
    }
    fn focused_window(&self) -> Option<FocusInfo> {
        None
    }
    fn probe(&self) -> BackendProbe {
        BackendProbe::default()
    }
    fn inject_correction(&self, _injection: Injection<'_>) -> Result<i32, BackendError> {
        Ok(0)
    }
}

struct Silent;

impl FieldReader for Silent {
    fn read(&self, _application: &str, _window: i64) -> Option<FieldContext> {
        None
    }
}

#[test]
fn what_a_backend_answers_by_default() {
    let backend = Minimal;
    backend.start(Arc::new(|_| {})).unwrap();
    backend.set_key_filter(None);
    backend.stop();
    backend.close();
    assert_eq!(backend.try_hold_input(), Ok(()));
    assert_eq!((backend.release_input(), backend.complete_action(true)), (Ok(0), Ok(0)));
    assert_eq!((backend.current_group(), backend.switch_group(1)), (Ok(0), Ok(())));
    assert!(backend.active_application().is_empty() && backend.focused_window().is_none());
    let _ = backend.probe();
    let strokes = [KeyEvent::default()];
    assert_eq!(backend.inject_correction(Injection { strokes: &strokes, target_group: 0, boundary: None, source_group: None, late: &[], trailing: &[], kept_tail: 0, erased: None }), Ok(0));
    assert!(backend.erases_any_length() && !backend.swallows_hotkeys() && !backend.supports_selection());
    assert_eq!(backend.take_unseen_input(), Ok(None));
    assert!(backend.take_input_delay().is_none() && backend.take_hook_gap().is_none() && backend.input_anchor().is_none());
    assert!(!backend.restore_window(None) && !backend.keep_window_inactive(1));
    let refused = SelectionError::Refused(SelectionRefused::new(RefusalReason::Unsupported));
    assert_eq!(backend.read_selection("app", 1).unwrap_err(), refused);
    let selected = SelectedText { text: String::new(), source: String::new() };
    assert_eq!(backend.replace_selection(&selected, "x", None).unwrap_err(), refused);
}

#[test]
fn a_reader_that_says_nothing_was_not_asked_and_errors_convert() {
    assert_eq!(Silent.status(), "not_requested");
    assert!(Silent.read("app", 1).is_none());
    let shift = KeyEvent { state: ks_backend::constants::SHIFT_MASK, ..KeyEvent::default() };
    assert!(shift.shift());
    assert_eq!(SelectionError::from(BackendError::Failed("x".to_owned())), SelectionError::Failed(BackendError::Failed("x".to_owned())));
}

#[test]
fn a_callback_that_crosses_the_gap_is_noted_and_a_foreign_one_too() {
    let delay = InputDelay::new();
    delay.record(1, 1);
    assert_eq!(delay.take_gap(), None);
    delay.record_foreign(1);
    assert_eq!(delay.take_gap(), None);
    delay.record_foreign(100_000);
    assert_eq!(delay.take_gap().map(|gap| gap.callback_ms), Some(100_000));
}
