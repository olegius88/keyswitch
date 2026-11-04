//! Keyboard event state machine and correction orchestration.
//!
//! Threads. `EngineCore` holds every field of the engine and belongs to one thread, the worker (or, in a replay, the
//! thread that drives it): it has no lock at all, and another thread reaches it only by a `Command` the worker takes
//! from its inbox. The Python's `RLock` became a `Mutex` around the few things other threads read and write
//! (`shared::Guarded`: the snapshot, the learning prompt, the callbacks); it is never held while calling out. What the
//! keyboard hook reads (`consumes_key`) is atomics in `Shared`; `enqueue` takes only the inbox's short lock.

mod commit;
mod decide;
mod early;
mod execute;
mod handle;
mod input;
mod logging;
mod manual;
mod mention;
mod neighbours;
mod pause;
mod selection;
mod shared;
mod state;
mod types;
mod word_parts;

pub use handle::KeySwitchEngine;
pub use shared::{Command, QueueItem, Shared};
pub use types::*;

use crate::constants::*;
use ks_backend::{FieldContext, FieldReader, InputBackend, KeyEvent};
use ks_config::config::SettingsStore;
use ks_config::history::HistoryStore;
use ks_config::learning::LearningStore;
use ks_config::personal_words::PersonalWords;
use ks_config::pycompat::truthy;
use ks_context::{ContextPolicy, ContextResult, KeyTemplates, TermTables, TypoCorrector, VersionedPrefixModel};
use ks_lang::boundary_policy::BoundaryPolicy;
use ks_lang::detector::{DetectionDecision, IntentClassifier, LanguageDetector, LanguageScorer, ProtectedTokens};
use ks_lang::early_switch::PrefixIndex;
use ks_lang::intent_model::{IntentModelStatus, LinearNgramModel};
use ks_lang::ortho_model::{KnownWord, LOCALES as ORTHO_LOCALES};
use ks_lang::{LanguageModel, Resources};
use ks_text::pystr;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

pub(crate) use decide::WordOptions;
pub(crate) use logging::technical_event;

/// The clock of the engine: `time.monotonic`, in seconds. A replay injects its own.
pub use ks_context::input_context::Clock;

/// What an engine is built from when the caller has the models already: a replay builds many engines over one set.
pub struct EngineParts {
    pub settings: Arc<SettingsStore>,
    pub history: Arc<HistoryStore>,
    pub backend: Arc<dyn InputBackend>,
    pub learning: Option<Arc<LearningStore>>,
    pub backend_label: String,
    pub models: Vec<Arc<LanguageModel>>,
    pub intent_model: Option<Arc<LinearNgramModel>>,
    pub intent_model_status: IntentModelStatus,
    pub protected_tokens: Arc<ProtectedTokens>,
    pub context_policy: ContextPolicy,
    pub boundary_model: Option<Arc<BoundaryPolicy>>,
    pub prefix_model: Option<Arc<VersionedPrefixModel>>,
    pub prefix_indexes: Vec<Arc<PrefixIndex>>,
    pub resource_dir: PathBuf,
    pub clock: Clock,
    /// The platform reader, if the context policy's reader is one: what the diagnostics of the reader name.
    pub platform_reader: Option<Arc<crate::context_access::PlatformFieldReader>>,
}

/// The state of the engine, with the Python's names. Every method but the few the handle calls runs on one thread.
pub struct EngineCore {
    pub shared: Arc<Shared>,
    pub settings: Arc<SettingsStore>,
    pub history: Arc<HistoryStore>,
    pub models: Vec<Arc<LanguageModel>>,
    pub intent_model_status: IntentModelStatus,
    pub detector: LanguageDetector,
    pub backend: Arc<dyn InputBackend>,
    pub backend_label: String,
    pub learning: Arc<LearningStore>,
    pub context_policy: ContextPolicy,
    pub platform_reader: Option<Arc<crate::context_access::PlatformFieldReader>>,
    pub resource_dir: PathBuf,
    pub clock: Clock,
    pub _context_result: Option<Arc<ContextResult>>,
    pub boundary_model: Option<Arc<BoundaryPolicy>>,
    pub prefix_model: Option<Arc<VersionedPrefixModel>>,
    pub _early_switch_confidence: f64,
    pub _context_waiting: Option<WaitingContextWord>,
    pub _provisional: Option<ProvisionalWord>,
    pub _paused_conversion: Option<ProvisionalWord>,
    pub _kept: Vec<Arc<KeptWord>>,
    pub _last_baseline: Option<DetectionDecision>,
    pub _context_wait_sequence: i64,
    pub _correction_sequence: i64,
    pub _late_refusal: String,
    pub _unseen_applications: HashSet<String>,
    pub _untracked_token: bool,
    pub _deferred_action: Option<Stroke>,
    pub _action_deadline: f64,
    pub _strokes: Strokes,
    pub _source_group: i64,
    pub _last_word_input_at: Option<f64>,
    pub _pause_correction_pending: bool,
    pub _manual_layout_group: Option<i64>,
    pub _own_layout_ignored: bool,
    pub _pressed: HashSet<u32>,
    pub _pressed_since: Vec<(u32, f64)>,
    pub _modifier_keycodes: HashSet<u32>,
    pub _symbol_strokes: Strokes,
    pub _mention_shown: Option<Stroke>,
    pub _last_key_character: String,
    pub _character_before_key: String,
    pub _position_unknown: bool,
    pub _insertion: Option<InsertionPoint>,
    pub _last_committed_stale: bool,
    pub _caret_moved: bool,
    pub _early_switch_origin: Option<i64>,
    pub _early_switch_at: Option<f64>,
    pub _engine_switch_at: Option<f64>,
    pub _engine_switch_group: Option<i64>,
    pub _manual_layout_observed_at: Option<f64>,
    pub _manual_layout_source: String,
    pub _manual_layout_word: bool,
    pub _early_switch_undone: bool,
    pub _pause_deferral_logged: bool,
    pub _wait_deferral_logged: i64,
    pub _prefix_indexes: Vec<Arc<PrefixIndex>>,
    pub _pending: Option<PlanRef>,
    pub _field_wait: Option<FieldWait>,
    pub _pending_trigger_keycode: i64,
    pub _input_delay_due: f64,
    pub _manual_release_deadline: f64,
    pub _last_committed: Option<PlanRef>,
    pub _last_correction: Option<PlanRef>,
    pub _last_correction_time: f64,
    pub _pause_agreement_open: bool,
    pub _learning_prompt_after: Option<LearningPrompt>,
    pub _correction_origin: Option<PlanRef>,
    pub _last_requested_plan: Option<PlanRef>,
    pub _convert_press_at: f64,
    pub _convert_keycode: i64,
    pub _convert_released: bool,
    pub _convert_outcome: Option<ConvertOutcome>,
    pub _selection_pending: Option<ks_config::selection_commands::PendingSelection>,
    pub _foreign_input_active: bool,
    pub _contexts: Vec<(String, LanguageContext)>,
    pub _typo: Option<Arc<TypoCorrector>>,
    pub _typo_keys: KeyTemplates,
    pub _personal: Arc<PersonalWords>,
    pub _personal_pending: String,
    term_tables: RefCell<Option<Arc<TermTables>>>,
    hotkey_cache: RefCell<HashMap<String, Arc<Hotkey>>>,
    /// Every plan `_execute_correction` carried out, when a replay asks for them.
    pub executed: Option<Vec<ExecutedPlan>>,
}

/// A failure the worker's loop recovers from, as `_run` catches an exception: `str(error)` is what the status line shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineError(pub String);

impl std::fmt::Display for EngineError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for EngineError {}

macro_rules! engine_error_from {
    ($($error:ty),*) => {$(
        impl From<$error> for EngineError {
            fn from(error: $error) -> Self {
                EngineError(error.to_string())
            }
        }
    )*};
}

engine_error_from!(
    ks_backend::BackendError,
    ks_lang::detector::DetectorError,
    ks_context::PolicyError,
    ks_lang::LoadError,
    ks_text::layouts::LayoutError
);

pub type EngineResult<T> = Result<T, EngineError>;

impl EngineCore {
    /// `KeySwitchEngine.__init__` on models already loaded.
    pub fn from_parts(parts: EngineParts) -> EngineResult<EngineCore> {
        let EngineParts {
            settings, history, backend, learning, backend_label, models, intent_model, intent_model_status, protected_tokens,
            context_policy, boundary_model, prefix_model, prefix_indexes, resource_dir, clock, platform_reader,
        } = parts;
        let learning = match learning {
            Some(learning) => learning,
            None => Arc::new(default_learning_store(&settings, &history).map_err(|error| EngineError(error.to_string()))?),
        };
        let scorers: Vec<(i64, Arc<dyn LanguageScorer>)> =
            models.iter().enumerate().map(|(group, model)| (group as i64, model.clone() as Arc<dyn LanguageScorer>)).collect();
        let classifier: Option<Arc<dyn IntentClassifier>> = intent_model.map(|model| model as _);
        let detector = LanguageDetector::new(scorers, classifier, protected_tokens)?;
        let personal_settings = settings.clone();
        let personal = PersonalWords::new(
            Some(sibling(&history.path, "personal-words.json")),
            Box::new(move || personal_settings.with_value("general.keep_history", |value| value.is_none_or(truthy))),
        );
        let snapshot = EngineSnapshot {
            enabled: settings.with_value("enabled", |value| value.is_none_or(truthy)),
            correction_count: history.read(None).map_or(0, |entries| entries.len() as i64),
            ..EngineSnapshot::default()
        };
        let shared = Shared::new(settings.clone(), backend.clone(), clock.clone(), models.len(), snapshot);
        Shared::subscribe_settings(&shared);
        let now = clock();
        let mut core = EngineCore {
            shared,
            settings,
            history,
            models,
            intent_model_status,
            detector,
            backend,
            backend_label,
            learning,
            context_policy,
            platform_reader,
            resource_dir,
            clock,
            _context_result: None,
            boundary_model,
            prefix_model,
            _early_switch_confidence: EARLY_SWITCH_CONFIDENCE,
            _context_waiting: None,
            _provisional: None,
            _paused_conversion: None,
            _kept: Vec::new(),
            _last_baseline: None,
            _context_wait_sequence: 0,
            _correction_sequence: 0,
            _late_refusal: String::new(),
            _unseen_applications: HashSet::new(),
            _untracked_token: false,
            _deferred_action: None,
            _action_deadline: 0.0,
            _strokes: Vec::new(),
            _source_group: -1,
            _last_word_input_at: None,
            _pause_correction_pending: false,
            _manual_layout_group: None,
            _own_layout_ignored: false,
            _pressed: HashSet::new(),
            _pressed_since: Vec::new(),
            _modifier_keycodes: HashSet::new(),
            _symbol_strokes: Vec::new(),
            _mention_shown: None,
            _last_key_character: String::new(),
            _character_before_key: String::new(),
            _position_unknown: true,
            _insertion: None,
            _last_committed_stale: false,
            _caret_moved: false,
            _early_switch_origin: None,
            _early_switch_at: None,
            _engine_switch_at: None,
            _engine_switch_group: None,
            _manual_layout_observed_at: None,
            _manual_layout_source: String::new(),
            _manual_layout_word: false,
            _early_switch_undone: false,
            _pause_deferral_logged: false,
            _wait_deferral_logged: 0,
            _prefix_indexes: prefix_indexes,
            _pending: None,
            _field_wait: None,
            _pending_trigger_keycode: -1,
            _input_delay_due: now + INPUT_DELAY_REPORT_INTERVAL_SECONDS,
            _manual_release_deadline: 0.0,
            _last_committed: None,
            _last_correction: None,
            _last_correction_time: 0.0,
            _pause_agreement_open: false,
            _learning_prompt_after: None,
            _correction_origin: None,
            _last_requested_plan: None,
            _convert_press_at: 0.0,
            _convert_keycode: -1,
            _convert_released: false,
            _convert_outcome: None,
            _selection_pending: None,
            _foreign_input_active: false,
            _contexts: Vec::new(),
            _typo: None,
            _typo_keys: KeyTemplates::default(),
            _personal: Arc::new(personal),
            _personal_pending: String::new(),
            term_tables: RefCell::new(None),
            hotkey_cache: RefCell::new(HashMap::new()),
            executed: None,
        };
        core._typo_corrector()?;
        core._technical_session_event("engine_initialized");
        if core.learning.migrated() != 0 {
            technical_event!(core.shared, "learning_rules_migrated", "rules": core.learning.migrated());
        }
        // The models are built: what their parsing left behind goes back to the system.
        ks_config::memory_release::release_free_memory();
        Ok(core)
    }

    /// `time.monotonic()` of the engine.
    #[inline]
    pub(crate) fn monotonic(&self) -> f64 {
        (self.clock)()
    }

    /// `bool(self.settings.get(path, default))`.
    #[inline]
    pub(crate) fn flag(&self, path: &str, default: bool) -> bool {
        self.settings.with_value(path, |value| value.map_or(default, truthy))
    }

    /// `str(self.settings.get(path, default))` as the checks against a name read it.
    pub(crate) fn text_setting(&self, path: &str, default: &str) -> String {
        self.settings.with_value(path, |value| match value {
            None => default.to_owned(),
            Some(Value::String(text)) => text.clone(),
            Some(other) => other.to_string(),
        })
    }

    /// `self.settings.get(path, [])`, a list of texts.
    pub(crate) fn words_setting(&self, path: &str) -> Vec<String> {
        self.settings.with_value(path, |value| match value {
            Some(Value::Array(items)) => items.iter().filter_map(|item| item.as_str().map(str::to_owned)).collect(),
            _ => Vec::new(),
        })
    }

    #[inline]
    pub(crate) fn in_models(&self, group: i64) -> bool {
        0 <= group && (group as usize) < self.models.len()
    }

    #[inline]
    pub(crate) fn model(&self, group: i64) -> &Arc<LanguageModel> {
        &self.models[group as usize]
    }

    /// `[group for group in self.models if group != source_group]`.
    pub(crate) fn other_groups(&self, source_group: i64) -> Vec<i64> {
        (0..self.models.len() as i64).filter(|&group| group != source_group).collect()
    }

    /// `{group: self._text_for_group(strokes, group) for group in self.models if group != source_group}`.
    pub(crate) fn alternatives_of(&self, strokes: &[Stroke], source_group: i64) -> Alternatives {
        self.other_groups(source_group).into_iter().map(|group| (group, _text_for_group(strokes, group))).collect()
    }

    /// `_term_frequency()`: the tables the context model holds, or read once without one.
    pub(crate) fn _term_frequency(&self) -> EngineResult<Arc<TermTables>> {
        if let Some(model) = &self.context_policy.model {
            return Ok(model.tables.clone());
        }
        let mut cached = self.term_tables.borrow_mut();
        if let Some(tables) = &*cached {
            return Ok(tables.clone());
        }
        let tables = Arc::new(TermTables::load(&self.resource_dir.join("models"))?);
        *cached = Some(tables.clone());
        Ok(tables)
    }

    /// `self._focus_window or 0`.
    #[inline]
    pub(crate) fn focus_or_zero(&self) -> i64 {
        self.shared._focus_window().unwrap_or(0)
    }

    pub(crate) fn _application_excluded(&self, application: &str) -> bool {
        self.shared._application_excluded(application)
    }

    /// `_bounded_setting`.
    /// A numeric setting within the range the settings windows offer for it.
    ///
    /// Both windows take their ranges from the same constants, and a value edited
    /// into the settings file by hand is brought into that range here; one that is
    /// not a number at all falls back to the default.
    pub(crate) fn _bounded_setting(&self, path: &str, default: f64, minimum: f64, maximum: f64) -> f64 {
        let value = self.settings.with_value(path, |value| match value {
            None => default,
            Some(Value::Bool(flag)) => f64::from(u8::from(*flag)),
            Some(Value::Number(number)) => number.as_f64().unwrap_or(default),
            Some(Value::String(text)) => ks_text::py_float(text).unwrap_or(default),
            Some(_) => default,
        });
        // `min(maximum, max(minimum, value))`, a NaN included.
        let low = if value > minimum { value } else { minimum };
        if low < maximum { low } else { maximum }
    }

    pub(crate) fn _pause_delay(&self) -> f64 {
        self._bounded_setting("detection.pause_delay_seconds", DEFAULT_PAUSE_DELAY_SECONDS, PAUSE_DELAY_SETTING_MIN_SECONDS, PAUSE_DELAY_SETTING_MAX_SECONDS)
    }

    pub(crate) fn _confidence_threshold(&self) -> f64 {
        self._bounded_setting("detection.confidence", DEFAULT_CONFIDENCE_THRESHOLD, CONFIDENCE_SETTING_MIN, CONFIDENCE_SETTING_MAX)
    }

    pub(crate) fn _minimum_word_length(&self) -> i64 {
        self._bounded_setting("detection.minimum_length", DEFAULT_MINIMUM_WORD_LENGTH, MINIMUM_WORD_LENGTH_SETTING_MIN, MINIMUM_WORD_LENGTH_SETTING_MAX)
            as i64
    }

    /// `known` of the policy built with an engine's models, as `KeySwitchEngine.__init__` hands it over: the orthotactic
    /// model's dictionaries are these models when they are the two it would load itself.
    pub fn known_of(models: &[Arc<LanguageModel>]) -> Option<KnownWord> {
        let dictionaries: Vec<Arc<LanguageModel>> = models.to_vec();
        let same = dictionaries.iter().map(|model| model.locale.as_str()).eq(ORTHO_LOCALES.iter().map(|(_, locale)| *locale));
        same.then(|| -> KnownWord {
            Arc::new(move |script, word| {
                let group = ORTHO_LOCALES.iter().position(|(name, _)| *name == script).unwrap_or(0);
                dictionaries[group].score(word).known
            })
        })
    }
}

/// `path.with_name(name)`.
pub(crate) fn sibling(path: &std::path::Path, name: &str) -> PathBuf {
    path.with_file_name(name)
}

/// `default_learning_store`.
/// The switching rules beside the history, as the engine keeps them when it is given none.
pub fn default_learning_store(
    settings: &SettingsStore,
    history: &HistoryStore,
) -> Result<LearningStore, ks_config::learning::LearningError> {
    let legacy = settings.get("detection.learning_confirmations").and_then(|value| ks_config::pycompat::as_py_int(&value));
    LearningStore::new(Some(sibling(&history.path, "learning.json")), Some(legacy.unwrap_or(LEGACY_RULE_CONFIRMATIONS_REQUIRED)))
}

/// `_text_for_group`.
#[inline]
pub fn _text_for_group(strokes: &[Stroke], group: i64) -> String {
    let group = group as i32;
    let mut text = String::with_capacity(strokes.len() * 2);
    for stroke in strokes {
        text.push_str(stroke.character_for(group));
    }
    text
}

/// Rounded milliseconds, as `round(seconds * MILLISECONDS_PER_SECOND)`.
#[inline]
pub(crate) fn ms(seconds: f64) -> i64 {
    ks_text::py_round(seconds * MILLISECONDS_PER_SECOND) as i64
}

/// `round(value, LOGGED_SCORE_DECIMALS)`.
#[inline]
pub(crate) fn logged(value: f64) -> Option<f64> {
    ks_text::py_round_ndigits(value, LOGGED_SCORE_DECIMALS)
}

/// `str.isalpha` of one character; `any(char.isalpha() for char in text)`.
pub(crate) fn any_alpha(text: &str) -> bool {
    text.chars().any(ks_text::ucd::isalpha)
}

/// `str.isspace` of one character.
pub(crate) fn char_isspace(character: char) -> bool {
    let mut buffer = [0; 4];
    pystr::isspace(character.encode_utf8(&mut buffer))
}

/// `sorted(keycodes)`.
pub(crate) fn sorted_keycodes(keys: &HashSet<u32>) -> Vec<u32> {
    let mut sorted: Vec<u32> = keys.iter().copied().collect();
    sorted.sort_unstable();
    sorted
}

/// `replace(field, before=..., after=...)`.
pub(crate) fn field_with(field: &FieldContext, before: String, after: String) -> FieldContext {
    FieldContext { before, after, ..field.clone() }
}

pub(crate) fn key_of(event: KeyEvent) -> Stroke {
    Arc::new(event)
}
