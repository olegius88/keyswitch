//! Optional, explicitly enabled accessibility reads outside keyboard hooks.

use ks_cast::To as _;
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
    failure_stage: Option<&'static str>,
    failure_type: Option<&'static str>,
    failure_name: Option<&'static str>,
    failure_code: Option<i64>,
    last_read_ms: Option<i64>,
    retry_attempts: usize,
    retry_after: Option<f64>,
}

pub struct PlatformFieldReader {
    factory: ReaderFactory,
    clock: Clock,
    password: Option<PasswordFocus>,
    state: Mutex<ReaderState>,
}

impl PlatformFieldReader {
    pub fn new(factory: ReaderFactory, clock: Clock) -> PlatformFieldReader {
        PlatformFieldReader {
            factory,
            clock,
            password: None,
            state: Mutex::new(ReaderState { status: "not_requested".to_owned(), ..ReaderState::default() }),
        }
    }

    #[must_use]
    pub fn with_password_focus(mut self, password: PasswordFocus) -> PlatformFieldReader {
        self.password = Some(password);
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
        result.insert("failure_stage".into(), json!(state.failure_stage));
        result.insert("failure_type".into(), json!(state.failure_type));
        if let Some(name) = state.failure_name {
            result.insert("failure_name".into(), json!(name));
        }
        if let Some(code) = state.failure_code {
            result.insert("failure_code".into(), json!(code));
        }
        if let Some(elapsed) = state.last_read_ms {
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
        result.insert("attempts".into(), json!(state.retry_attempts));
        result.insert("limit".into(), json!(FIELD_READER_RETRY_DELAYS_SECONDS.len()));
        let after = state.retry_after.map(|after| (ks_text::py_round((after - (self.clock)()) * MILLISECONDS_PER_SECOND)).to::<i64>().max(0));
        result.insert("after_ms".into(), json!(after));
        result
    }

    fn fail(&self, stage: &'static str, failure: ReaderFailure) -> Option<FieldContext> {
        // Never format provider exceptions: they may contain user text. Missing dependencies, access denial and failed
        // cleanup require an explicit reset, not repeated calls against the same failure.
        let mut state = self.state();
        "unavailable".clone_into(&mut state.status);
        state.failure_stage = Some(stage);
        state.failure_name = Some(failure.failure_name);
        state.failure_code = failure.failure_code;
        state.failure_type = Some(failure.failure_type);
        // A missing module or a denied right needs a new process, so nothing is gained by asking again. Everything else
        // is about one window at one moment - a provider that has not woken up, a call that ran out of time - and the
        // next application may answer immediately, so the attempts slow down to one a minute instead of stopping for good.
        let delay = FIELD_READER_RETRY_DELAYS_SECONDS[state.retry_attempts.min(FIELD_READER_RETRY_DELAYS_SECONDS.len() - 1)];
        state.retry_after = if failure.permanent { None } else { Some((self.clock)() + delay) };
        None
    }
}

impl FieldReader for PlatformFieldReader {
    fn read(&self, application: &str, window: i64) -> Option<FieldContext> {
        if application.is_empty() || window == 0 {
            return None;
        }
        if self.password.as_ref().is_some_and(|password| password(window)) {
            let mut field = FieldContext::new(application, format!("{window}:compositor"));
            field.role = FieldRole::Password;
            field.sensitive = true;
            "compositor".clone_into(&mut field.source);
            return Some(field);
        }
        let unavailable = self.state().status == "unavailable";
        if unavailable {
            let mut state = self.state();
            if state.retry_after.is_none_or(|after| (self.clock)() < after) {
                return None;
            }
            state.retry_attempts += 1;
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
        self.state().last_read_ms = Some((ks_text::py_round(((self.clock)() - started) * MILLISECONDS_PER_SECOND)).to::<i64>());
        match result {
            Err(failure) => self.fail("read", failure),
            Ok(result) => {
                let mut state = self.state();
                if result.is_some() { "available" } else { "unsupported_field" }.clone_into(&mut state.status);
                state.failure_stage = None;
                state.failure_type = None;
                state.failure_name = None;
                state.failure_code = None;
                state.retry_attempts = 0;
                state.retry_after = None;
                result.map(|field| field.bounded())
            }
        }
    }

    fn status(&self) -> String {
        self.state().status.clone()
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    type Answer = Result<Option<FieldContext>, ReaderFailure>;

    struct Scripted {
        answers: Arc<Mutex<VecDeque<Answer>>>,
        closed: Arc<AtomicU64>,
    }

    impl ManagedFieldReader for Scripted {
        fn read(&self, _application: &str, _window: i64) -> Answer {
            self.answers.lock().unwrap().pop_front().unwrap_or(Ok(None))
        }
        fn close(&mut self) {
            self.closed.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn failure(permanent: bool) -> ReaderFailure {
        ReaderFailure { failure_type: "T", failure_name: "N", failure_code: Some(7), permanent }
    }

    #[test]
    fn a_reader_opens_fails_retries_and_closes() {
        let answers: Arc<Mutex<VecDeque<Answer>>> = Arc::default();
        let closed = Arc::new(AtomicU64::new(0));
        let opens: Arc<Mutex<VecDeque<Result<(), ReaderFailure>>>> = Arc::default();
        let now = Arc::new(AtomicU64::new(0));
        let clock_now = now.clone();
        let clock: Clock = Arc::new(move || clock_now.load(Ordering::SeqCst).to::<f64>());
        let (factory_answers, factory_closed, factory_opens) = (answers.clone(), closed.clone(), opens.clone());
        let factory: ReaderFactory = Arc::new(move || {
            factory_opens.lock().unwrap().pop_front().unwrap_or(Ok(()))?;
            let opened: Box<dyn ManagedFieldReader> = Box::new(Scripted { answers: factory_answers.clone(), closed: factory_closed.clone() });
            Ok(opened)
        });
        let reader = PlatformFieldReader::new(factory, clock);
        assert_eq!(reader.status(), "not_requested");
        assert!(reader.read("", 1).is_none() && reader.read("app", 0).is_none());

        // An opening that fails for now, and the attempts that wait for their time.
        opens.lock().unwrap().push_back(Err(failure(false)));
        assert!(reader.read("app", 1).is_none());
        assert_eq!(reader.diagnostics()["failure_stage"], "initialization");
        assert_eq!((&reader.diagnostics()["failure_name"], &reader.diagnostics()["failure_code"]), (&json!("N"), &json!(7)));
        assert!(reader.retry_diagnostics()["after_ms"].is_i64());
        assert!(reader.read("app", 1).is_none());
        now.store(1000, Ordering::SeqCst);

        // A text, then a field it cannot read, then a failure in the read, which closes the old reader on the next try.
        answers.lock().unwrap().push_back(Ok(Some(FieldContext::new("app", "1"))));
        assert!(reader.read("app", 1).is_some());
        assert_eq!(reader.status(), "available");
        assert!(reader.diagnostics().contains_key("last_read_ms"));
        assert!(reader.read("app", 1).is_none());
        assert_eq!(reader.status(), "unsupported_field");
        answers.lock().unwrap().push_back(Err(failure(false)));
        assert!(reader.read("app", 1).is_none());
        assert_eq!(reader.diagnostics()["failure_stage"], "read");
        now.store(2000, Ordering::SeqCst);
        answers.lock().unwrap().push_back(Ok(Some(FieldContext::new("app", "1"))));
        assert!(reader.read("app", 1).is_some());
        assert_eq!(closed.load(Ordering::SeqCst), 1);

        // A failure that lasts: no more attempts, whatever the clock says.
        answers.lock().unwrap().push_back(Err(failure(true)));
        assert!(reader.read("app", 1).is_none());
        now.store(100_000, Ordering::SeqCst);
        assert!(reader.read("app", 1).is_none());
        assert_eq!(reader.retry_diagnostics()["after_ms"], Value::Null);
        reader.close();
        reader.close();
        assert_eq!(reader.status(), "not_requested");
        assert_eq!(closed.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_password_field_the_compositor_knows_is_never_read() {
        let factory: ReaderFactory = Arc::new(|| Err(failure(true)));
        let reader = PlatformFieldReader::new(factory, Arc::new(|| 0.0)).with_password_focus(Arc::new(|window| window == 5));
        let field = reader.read("app", 5).unwrap();
        assert!(field.sensitive && field.role == FieldRole::Password && field.source == "compositor");
        assert!(reader.read("app", 6).is_none());
        assert!(!reader.diagnostics().is_empty());
    }
}
