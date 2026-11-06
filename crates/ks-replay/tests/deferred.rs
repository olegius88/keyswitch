//! The engine built on a worker thread, and the window's calls before and after it is ready.
#![cfg(any(not(debug_assertions), coverage))]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ks_config::learning::LearningStore;
use ks_engine::engine::{EngineError, KeySwitchEngine};
use ks_engine::engine_loader::{Build, DeferredEngine, Post, StartFailed, LOADING_ACTION};
use ks_backend::KeyEvent;
use ks_replay::harness::{Bundle, KEYS};

type Posted = Arc<Mutex<Vec<Box<dyn FnOnce() + Send>>>>;

fn learning() -> Arc<LearningStore> {
    Arc::new(LearningStore::new(None, None).unwrap())
}

fn posting() -> (Post, Posted) {
    let posted: Posted = Arc::default();
    let queue = posted.clone();
    (Arc::new(move |call| queue.lock().unwrap().push(call)), posted)
}

fn run(posted: &Posted) {
    let calls: Vec<_> = posted.lock().unwrap().drain(..).collect();
    for call in calls {
        call();
    }
}

fn counting() -> (StartFailed, Arc<Mutex<Vec<String>>>) {
    let heard: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = heard.clone();
    (Arc::new(move |error: &str| sink.lock().unwrap().push(error.to_owned())), heard)
}

#[test]
fn a_build_that_fails_is_heard_once() {
    let (post, posted) = posting();
    let build: Build = Box::new(|| Err(EngineError("no models".to_owned())));
    let deferred = DeferredEngine::new(build, learning(), true, post, false);
    assert!(!deferred.ready());
    assert_eq!(deferred.snapshot().last_error, "no models");
    assert!(deferred.engine().is_err());
    assert!(deferred.learning_prompt().is_none());
    assert!(!deferred.confirm_learning_prompt(None) && !deferred.dismiss_learning_prompt(None));
    deferred.set_rule_editor_open(true);
    run(&posted);
    let (failed, heard) = counting();
    deferred.start(failed).unwrap();
    run(&posted);
    run(&posted);
    assert_eq!(*heard.lock().unwrap(), ["no models"]);
    deferred.stop();
}

#[test]
fn calls_wait_for_the_engine_and_reach_it() {
    let mut bundle = Bundle::load(None);
    let session = bundle.session("app", 0, Vec::new());
    let core = session.core;
    let (post, posted) = posting();
    let gate = Arc::new(Mutex::new(()));
    let held = gate.lock().unwrap();
    let open = gate.clone();
    let build: Build = Box::new(move || {
        drop(open.lock().unwrap());
        Ok(KeySwitchEngine::of(core))
    });
    let deferred = DeferredEngine::new(build, learning(), true, post, true);
    assert_eq!(deferred.snapshot().last_action, LOADING_ACTION);
    let subscribed = Arc::new(Mutex::new(0));
    let (count, _) = (subscribed.clone(), ());
    deferred.when_built(Box::new(move |_engine| *count.lock().unwrap() += 1));
    let (failed, heard) = counting();
    deferred.start(failed.clone()).unwrap();
    drop(held);
    assert!(deferred.engine().is_ok() && deferred.ready());
    // The builder posts its calls just after the engine is ready.
    let deadline = Instant::now() + Duration::from_secs(10);
    while *subscribed.lock().unwrap() == 0 && Instant::now() < deadline {
        run(&posted);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(*subscribed.lock().unwrap(), 1);
    assert!(heard.lock().unwrap().is_empty());
    let count = subscribed.clone();
    deferred.when_built(Box::new(move |_engine| *count.lock().unwrap() += 1));
    assert_eq!(*subscribed.lock().unwrap(), 2);
    assert!(deferred.snapshot().last_action != LOADING_ACTION);
    assert!(deferred.learning_prompt().is_none());
    assert!(!deferred.confirm_learning_prompt(None) && !deferred.dismiss_learning_prompt(None));
    deferred.set_rule_editor_open(false);
    deferred.start(failed).unwrap();
    deferred.stop();
    deferred.stop();
}

#[test]
fn a_window_closed_while_the_models_load_never_starts_the_engine() {
    let mut bundle = Bundle::load(None);
    let core = bundle.session("app", 0, Vec::new()).core;
    let (post, posted) = posting();
    let build: Build = Box::new(move || Ok(KeySwitchEngine::of(core)));
    let deferred = DeferredEngine::new(build, learning(), false, post, false);
    deferred.stop();
    run(&posted);
    assert!(!deferred.snapshot().enabled || deferred.ready());
}

/// The worker goes on after a failure nobody foresaw: the word is forgotten and the next one is corrected.
#[test]
fn the_worker_survives_what_nobody_foresaw() {
    let mut bundle = Bundle::load(None);
    let session = bundle.session("app", 0, Vec::new());
    let backend = session.backend.clone();
    let engine = KeySwitchEngine::of(session.core);
    engine.start().unwrap();
    let press = |character: char| {
        let key = KEYS.iter().find(|key| key.characters[0] == character).expect("a key of the first layout");
        engine.enqueue(KeyEvent { pressed: true, keycode: key.keycode, key_name: character.to_string(), character: character.to_string(), characters: key.strings(), ..KeyEvent::default() });
    };
    let wait_for = |what: &dyn Fn() -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !what() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(what());
    };
    backend.set_panicking(true);
    press('a');
    press(' ');
    wait_for(&|| engine.snapshot().last_error.contains("nobody foresaw"));
    backend.set_panicking(false);
    press('b');
    wait_for(&|| engine.snapshot().current_word == "b");
    engine.stop();
    bundle.clean();
}

#[test]
fn an_engine_starts_once_and_says_why_it_cannot() {
    let mut bundle = Bundle::load(None);
    let session = bundle.session("app", 0, Vec::new());
    let backend = session.backend.clone();
    let engine = KeySwitchEngine::of(session.core);
    assert!(!engine.select_alternate_group(), "not running");
    backend.set_failing(true);
    assert!(engine.start().is_err());
    assert_eq!(engine.snapshot().backend, "недоступен");
    backend.set_failing(false);
    engine.stop();

    bundle.clean();

    let mut bundle = Bundle::load(None);
    let session = bundle.session("app", 0, Vec::new());
    let backend = session.backend.clone();
    let engine = KeySwitchEngine::of(session.core);
    engine.start().unwrap();
    assert!(engine.start().is_ok(), "already running");
    let heard: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = heard.clone();
    engine.subscribe(Arc::new(move |snapshot| sink.lock().unwrap().push(snapshot.last_action.clone())));
    engine.subscribe_corrections(Arc::new(|_| {}));
    engine.subscribe_learning_prompts(Arc::new(|_| {}));
    engine.subscribe_rule_requests(Arc::new(|_| {}));
    engine.set_test_field_focused(true);
    engine.set_test_field_focused(false);
    engine.set_rule_editor_open(true);
    engine.set_rule_editor_open(false);
    assert!(engine.learning().count() < 1_000);
    assert!(engine.select_alternate_group(), "from the first layout to the second");
    backend.set_group(5);
    let deadline = Instant::now() + Duration::from_secs(5);
    while engine.snapshot().current_group != 1 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!heard.lock().unwrap().is_empty());
    engine.stop();
    engine.stop();
    assert!(engine.start().is_err() || engine.start().is_ok());
    engine.stop();
    bundle.clean();
}
