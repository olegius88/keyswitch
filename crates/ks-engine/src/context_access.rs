//! Optional, explicitly enabled accessibility reads outside keyboard hooks.

use crate::constants::{FIELD_READER_RETRY_DELAYS_SECONDS, MILLISECONDS_PER_SECOND};
use ks_backend::{FieldContext, FieldReader, FieldRole, ManagedFieldReader, ReaderFailure};
use ks_context::input_context::Clock;
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::sync::{Arc, Mutex, PoisonError};

/// Opens the reader of this platform, on the thread that will read with it.
pub type ReaderFactory = Arc<dyn Fn() -> Result<Box<dyn ManagedFieldReader>, ReaderFailure> + Send + Sync>;
/// A compositor that knows the keyboard is in a password field it draws itself says so before any accessibility read.
pub type PasswordFocus = Arc<dyn Fn(i64) -> bool + Send + Sync>;

thread_local! {
    // COM/provider ownership stays on the engine worker: the reader is built on the thread that reads with it and
    // never leaves it, and only one engine runs in a process.
    static READER: RefCell<Option<Box<dyn ManagedFieldReader>>> = const { RefCell::new(None) };
}

#[derive(Default)]
struct ReaderState {
    status: String,
    _failure_stage: Option<&'static str>,
    _failure_type: Option<&'static str>,
    _failure_name: Option<&'static str>,
    _failure_code: Option<i64>,
    _last_read_ms: Option<i64>,
    _retry_attempts: usize,
    _retry_after: Option<f64>,
}

pub struct PlatformFieldReader {
    factory: ReaderFactory,
    clock: Clock,
    _password: Option<PasswordFocus>,
    state: Mutex<ReaderState>,
}

impl PlatformFieldReader {
    pub fn new(factory: ReaderFactory, clock: Clock) -> PlatformFieldReader {
        PlatformFieldReader {
            factory,
            clock,
            _password: None,
            state: Mutex::new(ReaderState { status: "not_requested".to_owned(), ..ReaderState::default() }),
        }
    }

    pub fn with_password_focus(mut self, password: PasswordFocus) -> PlatformFieldReader {
        self._password = Some(password);
        self
    }

    fn state(&self) -> std::sync::MutexGuard<'_, ReaderState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Provider status without field contents.
    ///
    /// The exception's class name and, where the provider gives one, its HRESULT are named here on purpose: they say
    /// which call fails without carrying a single character the user typed. Anything else the provider puts into an
    /// exception stays out of the log.
    pub fn diagnostics(&self) -> Map<String, Value> {
        let state = self.state();
        let mut result = Map::new();
        result.insert("status".into(), json!(state.status));
        result.insert("failure_stage".into(), json!(state._failure_stage));
        result.insert("failure_type".into(), json!(state._failure_type));
        if let Some(name) = state._failure_name {
            result.insert("failure_name".into(), json!(name));
        }
        if let Some(code) = state._failure_code {
            result.insert("failure_code".into(), json!(code));
        }
        if let Some(elapsed) = state._last_read_ms {
            result.insert("last_read_ms".into(), json!(elapsed));
        }
        result
    }

    pub fn close(&self) {
        READER.with_borrow_mut(|reader| {
            if let Some(mut reader) = reader.take() {
                reader.close();
            }
        });
        *self.state() = ReaderState { status: "not_requested".to_owned(), ..ReaderState::default() };
    }

    pub fn retry_diagnostics(&self) -> Map<String, Value> {
        let state = self.state();
        let mut result = Map::new();
        result.insert("attempts".into(), json!(state._retry_attempts));
        result.insert("limit".into(), json!(FIELD_READER_RETRY_DELAYS_SECONDS.len()));
        let after = state._retry_after.map(|after| (ks_text::py_round((after - (self.clock)()) * MILLISECONDS_PER_SECOND) as i64).max(0));
        result.insert("after_ms".into(), json!(after));
        result
    }

    fn fail(&self, stage: &'static str, failure: ReaderFailure) -> Option<FieldContext> {
        // Never format provider exceptions: they may contain user text. Missing dependencies, access denial and failed
        // cleanup require an explicit reset, not repeated calls against the same failure.
        let mut state = self.state();
        state.status = "unavailable".to_owned();
        state._failure_stage = Some(stage);
        state._failure_name = Some(failure.failure_name);
        state._failure_code = failure.failure_code;
        state._failure_type = Some(failure.failure_type);
        // A missing module or a denied right needs a new process, so nothing is gained by asking again. Everything else
        // is about one window at one moment - a provider that has not woken up, a call that ran out of time - and the
        // next application may answer immediately, so the attempts slow down to one a minute instead of stopping for good.
        let delay = FIELD_READER_RETRY_DELAYS_SECONDS[state._retry_attempts.min(FIELD_READER_RETRY_DELAYS_SECONDS.len() - 1)];
        state._retry_after = if failure.permanent { None } else { Some((self.clock)() + delay) };
        None
    }
}

impl FieldReader for PlatformFieldReader {
    fn read(&self, application: &str, window: i64) -> Option<FieldContext> {
        if application.is_empty() || window == 0 {
            return None;
        }
        if self._password.as_ref().is_some_and(|password| password(window)) {
            let mut field = FieldContext::new(application, format!("{window}:compositor"));
            field.role = FieldRole::Password;
            field.sensitive = true;
            field.source = "compositor".to_owned();
            return Some(field);
        }
        let unavailable = self.state().status == "unavailable";
        if unavailable {
            let mut state = self.state();
            if state._retry_after.is_none_or(|after| (self.clock)() < after) {
                return None;
            }
            state._retry_attempts += 1;
            drop(state);
            // Reopen a failed bridge, never carry stale field ranges into a retry. A bridge that cannot even be closed
            // is dropped all the same: holding the reference would only send the next attempt into the same broken object.
            READER.with_borrow_mut(|reader| {
                if let Some(mut reader) = reader.take() {
                    reader.close();
                }
            });
        }
        let opened = READER.with_borrow_mut(|reader| {
            if reader.is_none() {
                *reader = Some((self.factory)()?);
            }
            Ok::<(), ReaderFailure>(())
        });
        if let Err(failure) = opened {
            return self.fail("initialization", failure);
        }
        let started = (self.clock)();
        let result = READER.with_borrow(|reader| reader.as_ref().expect("just opened").read(application, window));
        // How long the provider took is a fact about the provider, and the only way to tell "too slow for the timeout"
        // from "cannot do it at all" without printing anything the user typed.
        self.state()._last_read_ms = Some(ks_text::py_round(((self.clock)() - started) * MILLISECONDS_PER_SECOND) as i64);
        match result {
            Err(failure) => self.fail("read", failure),
            Ok(result) => {
                let mut state = self.state();
                state.status = if result.is_some() { "available" } else { "unsupported_field" }.to_owned();
                state._failure_stage = None;
                state._failure_type = None;
                state._failure_name = None;
                state._failure_code = None;
                state._retry_attempts = 0;
                state._retry_after = None;
                result.map(|field| field.bounded())
            }
        }
    }

    fn status(&self) -> String {
        self.state().status.clone()
    }
}
