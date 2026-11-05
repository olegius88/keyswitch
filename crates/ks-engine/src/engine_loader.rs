//! The keyboard engine built on a worker thread while the window and the tray come up.
//!
//! Building the engine loads every model: about 7 s on a fast computer and three times that on a busy one, and until then
//! the program showed nothing, neither its window nor its tray icon, so it looked as if it had not started. `DeferredEngine`
//! stands where the window code expects the engine: the switching rules, which the window shows at once, are its own;
//! subscriptions and the start wait for the engine and are passed on to it on the window's thread (`post`); until then the
//! snapshot says the models are loading, and so do the diagnostics, which name the models.

use crate::engine::{EngineError, EngineSnapshot, KeySwitchEngine, LearningPrompt};
use ks_backend::BackendError;
use ks_config::learning::{LearnedRule, LearningError, LearningStore};
use std::sync::{Arc, Condvar, Mutex, PoisonError};

pub const LOADING_ACTION: &str = "Загрузка языковых моделей…";

pub type Build = Box<dyn FnOnce() -> Result<KeySwitchEngine, EngineError> + Send>;
pub type Post = Arc<dyn Fn(Box<dyn FnOnce() + Send>) + Send + Sync>;
pub type StartFailed = Arc<dyn Fn(&str) + Send + Sync>;
type Subscription = Box<dyn FnOnce(&KeySwitchEngine) + Send>;

#[derive(Default)]
struct Loading {
    engine: Option<Arc<KeySwitchEngine>>,
    error: Option<String>,
    ready: bool,
    // Subscriptions made before the engine is built, each a call to make on it.
    subscriptions: Vec<Subscription>,
    start_failed: Option<StartFailed>,
    stopped: bool,
}

pub struct DeferredEngine {
    pub learning: Arc<LearningStore>,
    post: Post,
    state: Mutex<Loading>,
    built: Condvar,
    loading: EngineSnapshot,
}

impl DeferredEngine {
    pub fn new(build: Build, learning: Arc<LearningStore>, enabled: bool, post: Post, background: bool) -> Arc<DeferredEngine> {
        let deferred = Arc::new(DeferredEngine {
            learning,
            post,
            state: Mutex::new(Loading::default()),
            built: Condvar::new(),
            loading: EngineSnapshot { enabled, last_action: LOADING_ACTION.to_owned(), ..EngineSnapshot::default() },
        });
        if background {
            let loader = deferred.clone();
            let thread = std::thread::Builder::new().name("KeySwitchEngineLoader".to_owned()).spawn(move || loader.load(build));
            if let Err(error) = thread {
                deferred.lock().error = Some(error.to_string());
                deferred.lock().ready = true;
            }
        } else {
            deferred.load(build);
        }
        deferred
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Loading> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn load(self: &Arc<Self>, build: Build) {
        match build() {
            Err(error) => {
                // Reported on the window's thread, as a failed start is.
                {
                    let mut state = self.lock();
                    state.error = Some(error.to_string());
                    state.ready = true;
                }
                self.built.notify_all();
                let this = self.clone();
                (self.post)(Box::new(move || this.report_error()));
            }
            Ok(engine) => {
                {
                    let mut state = self.lock();
                    state.engine = Some(Arc::new(engine));
                    state.ready = true;
                }
                self.built.notify_all();
                let this = self.clone();
                (self.post)(Box::new(move || this.attach()));
            }
        }
    }

    /// On the window's thread: hand the waiting subscriptions to the engine and start it if it was asked to.
    fn attach(&self) {
        let (engine, subscriptions, start_failed, stopped) = {
            let mut state = self.lock();
            (state.engine.clone(), std::mem::take(&mut state.subscriptions), state.start_failed.clone(), state.stopped)
        };
        let engine = engine.expect("attached once built");
        for subscription in subscriptions {
            subscription(&engine);
        }
        if stopped {
            // The window closed while the models loaded: the engine never starts, and its stop closes the backend.
            engine.stop();
        } else if let Some(start_failed) = start_failed
            && let Err(error) = engine.start()
        {
            start_failed(&error.to_string());
        }
    }

    fn report_error(&self) {
        // Posted by the failed build and by a start asked for after it: the window hears of it once.
        let (start_failed, error) = {
            let mut state = self.lock();
            (state.start_failed.take(), state.error.clone())
        };
        if let (Some(start_failed), Some(error)) = (start_failed, error) {
            start_failed(&error);
        }
    }

    /// The engine, waited for; the error that kept it from being built, returned.
    pub fn engine(&self) -> Result<Arc<KeySwitchEngine>, EngineError> {
        let mut state = self.lock();
        while !state.ready {
            state = self.built.wait(state).unwrap_or_else(PoisonError::into_inner);
        }
        match (&state.engine, &state.error) {
            (Some(engine), _) => Ok(engine.clone()),
            (None, error) => Err(EngineError(error.clone().unwrap_or_default())),
        }
    }

    fn built_engine(&self) -> Option<Arc<KeySwitchEngine>> {
        self.lock().engine.clone()
    }

    /// Whether the engine is built; the diagnostics show the loading or its error until it is, not wait for it.
    pub fn ready(&self) -> bool {
        self.lock().engine.is_some()
    }

    pub fn snapshot(&self) -> EngineSnapshot {
        let state = self.lock();
        if let Some(engine) = &state.engine {
            return engine.snapshot();
        }
        if let Some(error) = &state.error {
            return EngineSnapshot { enabled: self.loading.enabled, last_error: error.clone(), ..EngineSnapshot::default() };
        }
        self.loading.clone()
    }

    pub fn learning_prompt(&self) -> Option<LearningPrompt> {
        self.built_engine().and_then(|engine| engine.learning_prompt())
    }

    /// Subscribe now if the engine is built, else once it is.
    pub fn when_built(&self, subscription: Subscription) {
        let engine = {
            let mut state = self.lock();
            if let Some(engine) = state.engine.clone() { engine } else {
                state.subscriptions.push(subscription);
                return;
            }
        };
        subscription(&engine);
    }

    /// Start the engine once it is built, on the window's thread; `failed` gets the error that stopped it.
    pub fn start(self: &Arc<Self>, failed: StartFailed) -> Result<(), BackendError> {
        let engine = {
            let mut state = self.lock();
            if let Some(engine) = state.engine.clone() { engine } else {
                state.start_failed = Some(failed);
                if state.error.is_some() {
                    let this = self.clone();
                    (self.post)(Box::new(move || this.report_error()));
                }
                return Ok(());
            }
        };
        if let Err(error) = engine.start() {
            failed(&error.to_string());
        }
        Ok(())
    }

    pub fn stop(&self) {
        let engine = {
            let mut state = self.lock();
            state.stopped = true;
            state.engine.clone()
        };
        if let Some(engine) = engine {
            engine.stop();
        }
    }

    pub fn confirm_learning_prompt(&self, prompt: Option<&LearningPrompt>) -> bool {
        self.built_engine().is_some_and(|engine| engine.confirm_learning_prompt(prompt))
    }

    pub fn dismiss_learning_prompt(&self, prompt: Option<&LearningPrompt>) -> bool {
        self.built_engine().is_some_and(|engine| engine.dismiss_learning_prompt(prompt))
    }

    pub fn set_rule_editor_open(&self, opened: bool) {
        if let Some(engine) = self.built_engine() {
            engine.set_rule_editor_open(opened);
        }
    }

    pub fn set_test_field_focused(&self, focused: bool) {
        if let Some(engine) = self.built_engine() {
            engine.set_test_field_focused(focused);
        }
    }

    pub fn select_alternate_group(&self) -> bool {
        self.built_engine().is_some_and(|engine| engine.select_alternate_group())
    }

    pub fn add_learning_rule(&self, rule: LearnedRule, replacing: Option<&LearnedRule>, application: &str) -> Result<LearnedRule, LearningError> {
        let engine = self.engine().map_err(|error| LearningError::from(std::io::Error::other(error.0)))?;
        engine.add_learning_rule(rule, replacing, application)
    }
}
