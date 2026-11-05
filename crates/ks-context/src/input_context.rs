//! Bounded, field-scoped context. User text is kept in RAM, never serialized.
//!
//! An observed suffix is not a copy of the editor. Navigation, a shortcut, a focus change or lost input invalidates it.
//! Accessibility snapshots may add context, but never grant permission to replace text outside tracked strokes.
//!
//! `FieldContext` and `FieldRole` are those of `ks-backend`; they are exported here under the Python's names.

use ks_cast::To as _;
use crate::constants::FIELD_CONTEXT_TTL_SECONDS;
use ks_text::pystr;
use std::sync::Arc;
#[cfg(test)]
use std::sync::Mutex;
use std::time::Instant;

pub use ks_backend::constants::FIELD_CONTEXT_MAX_CHARACTERS as CONTEXT_LIMIT;
pub use ks_backend::{FieldContext, FieldReader, FieldRole, KeyEvent};

/// `CONTEXT_TTL`.
pub const CONTEXT_TTL: f64 = FIELD_CONTEXT_TTL_SECONDS;

/// A clock in seconds, as `time.monotonic` is. `InputContext` reads its own: a replay patches the engine's clock and
/// not this one, so the caller says which clock the context runs on.
pub type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;

/// The real monotonic clock, seconds since the first call of this function.
#[must_use]
pub fn monotonic_clock() -> Clock {
    let start = Instant::now();
    Arc::new(move || start.elapsed().as_secs_f64())
}

/// One active field, no background history and no cross-window cache.
pub struct InputContext {
    pub application: String,
    pub window: i64,
    pub text: String,
    pub updated_at: f64,
    pub revision: u64,
    clock: Clock,
}

impl Default for InputContext {
    fn default() -> Self {
        InputContext::new()
    }
}

impl InputContext {
    #[must_use]
    pub fn new() -> Self {
        InputContext::with_clock(monotonic_clock())
    }

    pub fn with_clock(clock: Clock) -> Self {
        InputContext { application: String::new(), window: 0, text: String::new(), updated_at: 0.0, revision: 0, clock }
    }

    fn now(&self) -> f64 {
        (self.clock)()
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.updated_at = 0.0;
        self.revision += 1;
    }

    pub fn focus(&mut self, application: &str, window: i64) {
        if (application, window) != (self.application.as_str(), self.window) {
            self.clear();
            self.application = application.to_string();
            self.window = window;
        }
    }

    pub fn observe(&mut self, event: &KeyEvent) {
        if !event.pressed || event.synthetic {
            return;
        }
        if event.control() || event.alt() || event.super_key() {
            self.clear();
            return;
        }
        if matches!(event.key_name.as_str(), "Return" | "KP_Enter" | "Tab" | "ISO_Left_Tab") {
            if !event.deferred {
                self.clear();
            }
            return;
        }
        if matches!(
            event.key_name.as_str(),
            "Left" | "Right" | "Up" | "Down" | "Home" | "End" | "Page_Up" | "Page_Down" | "Delete" | "Insert" | "Escape" | "Pointer"
        ) {
            self.clear();
            return;
        }
        let now = self.now();
        if now - self.updated_at > CONTEXT_TTL {
            self.text.clear();
        }
        if event.key_name == "BackSpace" {
            self.text = pystr::slice(&self.text, None, Some(-1)).to_string();
        } else if !event.character.is_empty() && (pystr::isprintable(&event.character) || pystr::isspace(&event.character)) {
            let joined = format!("{}{}", self.text, event.character);
            self.text = pystr::slice(&joined, Some(-CONTEXT_LIMIT.to::<i64>()), None).to_string();
        } else {
            return;
        }
        self.updated_at = now;
        self.revision += 1;
    }

    #[must_use]
    pub fn before_word(&self, original: &str) -> String {
        if self.now() - self.updated_at > CONTEXT_TTL {
            return String::new();
        }
        // A normal boundary has already reached the editor; an intercepted Enter has not. Only strip a single observed
        // boundary, never search backwards for an older occurrence of the same word.
        let length = pystr::len(original).to::<i64>();
        if !original.is_empty() && self.text.ends_with(original) {
            return pystr::slice(&self.text, None, Some(-length)).to_string();
        }
        if !original.is_empty() && pystr::slice(&self.text, None, Some(-1)).ends_with(original) {
            return pystr::slice(&self.text, None, Some(-length - 1)).to_string();
        }
        String::new()
    }

    /// Take back the text observed last, to be typed and observed again; false if it is not there.
    pub fn withdraw(&mut self, typed: &str) -> bool {
        if !self.text.ends_with(typed) {
            return false;
        }
        self.text.truncate(self.text.len() - typed.len());
        self.revision += 1;
        true
    }

    pub fn replace_suffix(&mut self, original: &str, replacement: &str, boundary: &str) {
        let old = format!("{original}{boundary}");
        if !old.is_empty() && self.text.ends_with(&old) {
            let joined = format!("{}{replacement}{boundary}", self.text.get(..self.text.len() - old.len()).unwrap_or_default());
            self.text = pystr::slice(&joined, Some(-CONTEXT_LIMIT.to::<i64>()), None).to_string();
            self.updated_at = self.now();
            self.revision += 1;
        } else {
            self.clear();
        }
    }

    #[must_use]
    pub fn snapshot(&self, original: &str) -> FieldContext {
        let mut field = FieldContext::new(self.application.clone(), self.window.to_string());
        field.before = self.before_word(original);
        field
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn what_a_stream_ignores_and_the_clock_it_keeps_by_default() {
        let mut context = InputContext::default();
        context.observe(&KeyEvent { pressed: true, character: "а".to_owned(), ..KeyEvent::default() });
        context.observe(&KeyEvent { pressed: false, character: "б".to_owned(), ..KeyEvent::default() });
        context.observe(&KeyEvent { pressed: true, synthetic: true, character: "в".to_owned(), ..KeyEvent::default() });
        assert_eq!(context.text, "а");
        context.observe(&KeyEvent { pressed: true, state: ks_backend::constants::ALT_MASK, ..KeyEvent::default() });
        assert_eq!(context.text, "");
        context.observe(&KeyEvent { pressed: true, character: "а".to_owned(), ..KeyEvent::default() });
        context.observe(&KeyEvent { pressed: true, state: ks_backend::constants::SUPER_MASK, ..KeyEvent::default() });
        assert_eq!(context.text, "");
    }

    #[test]
    fn text_is_kept_until_the_time_to_live_passes() {
        let now = Arc::new(Mutex::new(100.0));
        let clock: Clock = {
            let now = now.clone();
            Arc::new(move || *now.lock().expect("lock"))
        };
        let mut context = InputContext::with_clock(clock);
        let key = |character: &str| KeyEvent { pressed: true, key_name: character.to_string(), character: character.to_string(), ..KeyEvent::default() };
        for character in ["a", "b", " "] {
            context.observe(&key(character));
        }
        assert_eq!(context.before_word("b"), "a");
        *now.lock().expect("lock") += CONTEXT_TTL + 1.0;
        assert_eq!(context.before_word("b"), "");
    }
}
