//! The engine's handle, the hook's questions and the deferred start, over states the replays do not reach.
#![cfg(any(not(debug_assertions), coverage))]

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use ks_backend::constants::{ALT_MASK, CONTROL_MASK, SHIFT_MASK, SUPER_MASK};
use ks_backend::{KeyDisposition, KeyEvent};
use ks_config::learning::{LearnedRule, LearningStore};
use ks_engine::engine::{EngineError, KeySwitchEngine, QueueItem};
use ks_engine::engine_loader::{Build, DeferredEngine, Post};
use ks_engine::rule_editor::pattern_group;
use ks_replay::harness::{Bundle, SWALLOWS};
use serde_json::json;

fn key(name: &str, state: u32) -> KeyEvent {
    KeyEvent { pressed: true, key_name: name.to_owned(), character: name.to_owned(), characters: vec![name.to_owned(), name.to_owned()], state, ..KeyEvent::default() }
}

#[test]
fn the_hook_asks_about_every_kind_of_key() {
    let mut bundle = Bundle::load(None);
    SWALLOWS.store(true, Ordering::Relaxed);
    let mut session = bundle.session("app", 0, Vec::new());
    SWALLOWS.store(false, Ordering::Relaxed);
    let shared = session.core.shared.clone();
    let settings = session.core.settings.clone();
    assert_eq!(shared.consumes_key(&KeyEvent { pressed: false, ..key("a", 0) }), KeyDisposition::Pass);
    assert_eq!(shared.consumes_key(&KeyEvent { synthetic: true, ..key("a", 0) }), KeyDisposition::Pass);
    assert_eq!(shared.consumes_key(&KeyEvent { foreign: true, ..key("a", 0) }), KeyDisposition::Pass);
    settings.set("detection.injected_input", json!(true), false).unwrap();
    session.core.apply_commands();
    let _ = shared.consumes_key(&KeyEvent { foreign: true, ..key("a", 0) });
    for state in [CONTROL_MASK, ALT_MASK, SUPER_MASK, SHIFT_MASK] {
        assert_eq!(shared.consumes_key(&key("q", state)), KeyDisposition::Pass);
    }
    assert_eq!(shared.consumes_key(&key("p", CONTROL_MASK | ALT_MASK)), KeyDisposition::Consume, "the program's own hotkey");
    for name in ["Return", "KP_Enter", "Tab", "a"] {
        let _ = shared.consumes_key(&key(name, 0));
    }
    settings.set("hotkeys.toggle", json!(5), false).unwrap();
    settings.set("hotkeys.undo", json!({"a": 1}), false).unwrap();
    settings.set("hotkeys.convert_last", json!(["Ctrl"]), false).unwrap();
    settings.set("hotkeys.selection_layout", json!("super+x"), false).unwrap();
    session.core.apply_commands();
    settings.set("hotkeys", json!(1), false).unwrap();
    session.core.apply_commands();
    settings.set("exclusions.applications", json!(["  ", "ap", 5]), false).unwrap();
    assert!(shared.queued().is_empty());
    for _ in 0..100_000 {
        if !shared.put_nowait(QueueItem::Stop) {
            break;
        }
    }
    assert!(!shared.put_nowait(QueueItem::Stop), "the queue has an end");
    bundle.clean();
}

#[test]
fn the_handle_says_what_it_cannot_do() {
    let mut bundle = Bundle::load(None);
    let session = bundle.session("app", 5, Vec::new());
    let backend = session.backend.clone();
    let engine = KeySwitchEngine::of(session.core);
    engine.start().unwrap();
    assert!(!engine.select_alternate_group(), "a third layout has no other side");
    assert!(!engine.dismiss_learning_prompt(None));
    backend.set_failing(true);
    std::thread::sleep(std::time::Duration::from_millis(300));
    backend.set_failing(false);
    backend.set_group(0);
    for _ in 0..200_000 {
        engine.enqueue(key("a", 0));
    }
    let _ = engine.select_alternate_group();
    engine.stop();
    drop(engine.core().take());
    assert!(engine.start().is_err(), "nothing left to start");
    engine.stop();
    bundle.clean();
}

#[test]
fn the_worker_wakes_sooner_for_a_pause_or_a_field_that_lags() {
    let mut bundle = Bundle::load(None);
    let mut session = bundle.session("app", 0, Vec::new());
    assert!(session.core.loop_timeout() > 0.0);
    session.core.pause_correction_pending = true;
    session.core.last_word_input_at = Some(0.0);
    let soon = session.core.loop_timeout();
    session.advance(1000.0);
    let due = session.core.loop_timeout();
    assert!(due <= soon);
    session.core.pause_correction_pending = false;
    session.backend.set_failing(true);
    session.core.turn(0.0);
    session.backend.set_failing(false);
    assert!(!session.core.input_overflow());
    bundle.give_back(session);
    bundle.clean();
}

fn learning(folder: &std::path::Path) -> Arc<LearningStore> {
    Arc::new(LearningStore::new(Some(folder.join("rules.json")), None).unwrap())
}

type Posted = Arc<Mutex<Vec<Box<dyn FnOnce() + Send>>>>;

fn posting() -> (Post, Posted) {
    let posted: Posted = Arc::default();
    let queue = posted.clone();
    (Arc::new(move |call| queue.lock().unwrap().push(call)), posted)
}

#[test]
fn the_deferred_engine_starts_what_was_built_and_passes_calls_on() {
    let folder = std::env::temp_dir().join(format!("ks-shell-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let mut bundle = Bundle::load(None);
    let session = bundle.session("app", 0, Vec::new());
    let backend = session.backend.clone();
    let core = session.core;
    let (post, posted) = posting();
    let build: Build = Box::new(move || Ok(KeySwitchEngine::of(core)));
    let deferred = DeferredEngine::new(build, learning(&folder), true, post, false);
    assert!(deferred.ready(), "built on the spot");
    deferred.set_test_field_focused(true);
    deferred.set_test_field_focused(false);
    assert!(deferred.add_learning_rule(LearnedRule::new("слово", 1, 0), None, "app").is_ok());
    let heard: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = heard.clone();
    backend.set_failing(true);
    deferred.start(Arc::new(move |error: &str| sink.lock().unwrap().push(error.to_owned()))).unwrap();
    backend.set_failing(false);
    assert_eq!(heard.lock().unwrap().len(), 1, "a start that fails is heard");
    deferred.stop();
    posted.lock().unwrap().drain(..).for_each(|call| call());
    bundle.clean();

    let (post, posted) = posting();
    let build: Build = Box::new(|| Err(EngineError("no models".to_owned())));
    let failed = DeferredEngine::new(build, learning(&folder), true, post, false);
    failed.set_test_field_focused(true);
    assert!(failed.add_learning_rule(LearnedRule::new("слово", 1, 0), None, "app").is_err());
    posted.lock().unwrap().drain(..).for_each(|call| call());
    std::fs::remove_dir_all(&folder).unwrap();
}

#[test]
fn a_pattern_of_letters_of_neither_alphabet_has_no_layout() {
    assert_eq!(pattern_group("abc"), Some(0));
    assert_eq!(pattern_group("абв"), Some(1));
    assert_eq!(pattern_group("aб"), None);
    assert_eq!(pattern_group("中"), None);
    assert_eq!(pattern_group("a中"), None);
    assert_eq!(pattern_group("123"), None);
}
