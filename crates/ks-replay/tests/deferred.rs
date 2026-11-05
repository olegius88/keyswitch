//! The engine built on a worker thread, and the window's calls before and after it is ready.
#![cfg(any(not(debug_assertions), coverage))]

use std::sync::{Arc, Mutex};

use ks_config::learning::LearningStore;
use ks_engine::engine::{EngineError, KeySwitchEngine};
use ks_engine::engine_loader::{Build, DeferredEngine, Post, StartFailed, LOADING_ACTION};
use ks_replay::harness::Bundle;

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
    run(&posted);
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
