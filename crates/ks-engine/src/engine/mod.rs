//! Keyboard event state machine and correction orchestration.
//!
//! Threads. `EngineCore` holds every field of the engine and belongs to one thread, the worker (or, in a replay, the
//! thread that drives it): it has no lock at all, and another thread reaches it only by a `Command` the worker takes
//! from its inbox. The Python's `RLock` became a `Mutex` around the few things other threads read and write
//! (`shared::Guarded`: the snapshot, the learning prompt, the callbacks); it is never held while calling out. What the
//! keyboard hook reads (`consumes_key`) is atomics in `Shared`; `enqueue` takes only the inbox's short lock.

use ks_cast::To as _;
mod commit;
mod decide;
mod early;
mod execute;
mod handle;
mod input;
mod layout_tap;
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

pub use decide::WordOptions;
pub use logging::WordEvaluation;
pub use handle::KeySwitchEngine;
pub use layout_tap::{
    CONVERT_LAST_KEY_DESCRIPTION, CONVERT_LAST_KEY_LABELS, CONVERT_LAST_KEY_TITLE, LAYOUT_SWITCH_KEY_DESCRIPTION, LAYOUT_SWITCH_KEY_LABELS,
    LAYOUT_SWITCH_KEY_TITLE, LAYOUT_TAP_SOURCE, single_key_conflict,
};
pub use shared::{Command, QueueItem, Shared};
pub use types::*;

use crate::constants::{WHOLE_TOKEN_WORD_MIN_LETTERS, DOUBLE_CAPITALS, DOUBLE_CAPITALS_MIN_LETTERS, UNSCORED_CORRECTION_CONFIDENCE, STRANDED_TERM_MAX_WORDS, MAX_REMEMBERED_APPLICATION_CONTEXTS, DEFAULT_EARLY_SWITCH_MIN_LENGTH, EARLY_SWITCH_MIN_LENGTH_SETTING_MIN, EARLY_SWITCH_MIN_LENGTH_SETTING_MAX, EARLY_SWITCH_CONFIDENCE, PREFIX_MIN_CHARACTERS, PREFIX_MAX_CHARACTERS, EARLY_PLAN_MODE, LATE_KEYS_OWED_SECONDS, LATE_KEYS_RETURN_SECONDS, LATE_STROKE_GRACE_SECONDS, TYPO_ENGLISH_LOCALE, UNDO_AVAILABLE_WINDOW_SECONDS, DEFAULT_FIELD_CATCH_UP, FIELD_CATCH_UP_CHOICES, FIELD_CATCH_UP_OFF, MILLISECONDS_PER_SECOND, HOOK_SILENCE_MS, INPUT_DELAY_REPORT_INTERVAL_SECONDS, INPUT_DELAY_LATE_MS, INPUT_DELAY_SLOW_CALLBACK_MS, INPUT_DELAY_UNSTAMPED_MS, HOOK_GAP_MS, SELECTION_HOTKEYS, ACTION_TIMEOUT_SECONDS, MAX_WORD_STROKES, BASIC_MULTILINGUAL_PLANE_MAX_CODEPOINT, MANUAL_RELEASE_TIMEOUT_SECONDS, PAUSE_CORRECTION_AGREEMENT_SECONDS, DOUBLE_CONVERT_PRESS_WINDOW_SECONDS, TRUSTED_SHORT_WORD_MAX_LENGTH, CONTEXT_ACTION_FEATURE_VERSION, KEPT_CONTEXT_WORD_MAX_CHARACTERS, PLANNED_CONTEXT_WORD_MAX_CHARACTERS, KEPT_WORDS_TAKEN_ALONG, CONTEXT_WAIT_SETTLE_MIN_SECONDS, STALE_PRESS_SECONDS, ENGINE_SWITCH_GRACE_SECONDS, NATURAL_SOURCE_BOUNDARY_MIN_CHARACTERS, NATURAL_SOURCE_BOUNDARY_NGRAM_FLOOR, JOINED_PART_MIN_LETTERS, REPLAYED_SIGNS_MIN_STEM_LETTERS, PAUSE_WORD_START_LETTERS, PAUSE_WORD_START_MIN_WORDS, CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS, PAUSE_TRAILING_MAX_KEYS, KEPT_WORD_PROSE_MIN_COUNT, DEFAULT_PAUSE_DELAY_SECONDS, PAUSE_DELAY_SETTING_MIN_SECONDS, PAUSE_DELAY_SETTING_MAX_SECONDS, DEFAULT_CONFIDENCE_THRESHOLD, CONFIDENCE_SETTING_MIN, CONFIDENCE_SETTING_MAX, DEFAULT_MINIMUM_WORD_LENGTH, MINIMUM_WORD_LENGTH_SETTING_MIN, MINIMUM_WORD_LENGTH_SETTING_MAX, LEGACY_RULE_CONFIRMATIONS_REQUIRED, LOGGED_SCORE_DECIMALS};
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
use std::sync::{Arc, OnceLock};

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
    pub context_result: Option<Arc<ContextResult>>,
    pub boundary_model: Option<Arc<BoundaryPolicy>>,
    pub prefix_model: Option<Arc<VersionedPrefixModel>>,
    pub early_switch_confidence: f64,
    pub context_waiting: Option<WaitingContextWord>,
    pub provisional: Option<ProvisionalWord>,
    pub paused_conversion: Option<ProvisionalWord>,
    pub kept: Vec<Arc<KeptWord>>,
    pub last_baseline: Option<DetectionDecision>,
    pub context_wait_sequence: i64,
    pub correction_sequence: i64,
    pub late_refusal: String,
    /// The window in front as `track_focus` last read it, an own one included; 0 when the backend named none. A key the
    /// hook saw on its way to a window is compared with it (`typed_elsewhere`).
    pub window_in_front: i64,
    /// Until when keys wait behind the letters a correction types again (`Shared::put_behind`).
    pub behind_late_until: Option<f64>,
    /// Letters of such a correction that had not come back when the keys behind them went on, and until when one that
    /// comes is still taken for one of them (`owed_late_letter`).
    pub late_letters_owed: usize,
    pub late_letters_owed_until: f64,
    pub unseen_applications: HashSet<String>,
    pub untracked_token: bool,
    pub deferred_action: Option<Stroke>,
    pub action_deadline: f64,
    pub strokes: Strokes,
    pub source_group: i64,
    pub last_word_input_at: Option<f64>,
    pub pause_correction_pending: bool,
    pub manual_layout_group: Option<i64>,
    pub own_layout_ignored: bool,
    pub pressed: HashSet<u32>,
    pub pressed_since: Vec<(u32, f64)>,
    pub modifier_keycodes: HashSet<u32>,
    /// A lone press of the layout switch key or the conversion key, until it comes up or anything else happens
    /// (`layout_tap`).
    pub layout_tap: Option<LayoutTap>,
    pub symbol_strokes: Strokes,
    pub mention_shown: Option<Stroke>,
    pub last_key_character: String,
    pub character_before_key: String,
    pub position_unknown: bool,
    pub insertion: Option<InsertionPoint>,
    pub last_committed_stale: bool,
    pub caret_moved: bool,
    pub early_switch_origin: Option<i64>,
    pub early_switch_at: Option<f64>,
    pub engine_switch_at: Option<f64>,
    pub engine_switch_group: Option<i64>,
    pub manual_layout_observed_at: Option<f64>,
    pub manual_layout_source: String,
    pub manual_layout_word: bool,
    pub early_switch_undone: bool,
    pub pause_deferral_logged: bool,
    pub wait_deferral_logged: i64,
    pub prefix_indexes: Vec<Arc<PrefixIndex>>,
    pub pending: Option<PlanRef>,
    pub field_wait: Option<FieldWait>,
    /// Every key press the worker has answered, but those of the modifiers: what a correction that waits for its field
    /// counts the keys typed meanwhile by (`typed_on_since`).
    pub presses_handled: u64,
    /// How many times the engine has answered an Enter or Tab the hook keeps back (`complete_action`), and that count as
    /// it stood when the key in hand is such a key nobody has answered yet: see `handle_inner`.
    pub actions_completed: u64,
    pub withheld_in_hand: Option<u64>,
    pub pending_trigger_keycode: i64,
    pub input_delay_due: f64,
    pub manual_release_deadline: f64,
    pub last_committed: Option<PlanRef>,
    pub last_correction: Option<PlanRef>,
    pub last_correction_time: f64,
    pub pause_agreement_open: bool,
    pub learning_prompt_after: Option<LearningPrompt>,
    pub correction_origin: Option<PlanRef>,
    pub last_requested_plan: Option<PlanRef>,
    pub convert_press_at: f64,
    pub convert_keycode: i64,
    pub convert_released: bool,
    pub convert_outcome: Option<ConvertOutcome>,
    pub selection_pending: Option<ks_config::selection_commands::PendingSelection>,
    pub foreign_input_active: bool,
    pub contexts: Vec<(String, LanguageContext)>,
    pub typo: Option<Arc<TypoCorrector>>,
    pub typo_keys: KeyTemplates,
    pub personal: Arc<PersonalWords>,
    pub personal_pending: String,
    term_tables: RefCell<Option<Arc<TermTables>>>,
    hotkey_cache: RefCell<HashMap<String, Arc<Hotkey>>>,
    /// Every plan `execute_correction` carried out, when a replay asks for them.
    pub executed: Option<Vec<ExecutedPlan>>,
}

/// A failure the worker's loop recovers from, as `run` catches an exception: `str(error)` is what the status line shows.
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
            models.iter().enumerate().map(|(group, model)| -> (i64, Arc<dyn LanguageScorer>) { (group.to::<i64>(), model.clone()) }).collect();
        let classifier: Option<Arc<dyn IntentClassifier>> = intent_model.map(|model| -> Arc<dyn IntentClassifier> { model });
        let detector = LanguageDetector::new(scorers, classifier, protected_tokens)?;
        let personal_settings = settings.clone();
        let personal = PersonalWords::new(
            Some(sibling(&history.path, "personal-words.json")),
            Box::new(move || personal_settings.with_value("general.keep_history", |value| value.is_none_or(truthy))),
        );
        let snapshot = EngineSnapshot {
            enabled: settings.with_value("enabled", |value| value.is_none_or(truthy)),
            correction_count: history.read(None).map_or(0, |entries| entries.len().to::<i64>()),
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
            context_result: None,
            boundary_model,
            prefix_model,
            early_switch_confidence: EARLY_SWITCH_CONFIDENCE,
            context_waiting: None,
            provisional: None,
            paused_conversion: None,
            kept: Vec::new(),
            last_baseline: None,
            context_wait_sequence: 0,
            correction_sequence: 0,
            late_refusal: String::new(),
            window_in_front: 0,
            behind_late_until: None,
            late_letters_owed: 0,
            late_letters_owed_until: 0.0,
            unseen_applications: HashSet::new(),
            untracked_token: false,
            deferred_action: None,
            action_deadline: 0.0,
            strokes: Vec::new(),
            source_group: -1,
            last_word_input_at: None,
            pause_correction_pending: false,
            manual_layout_group: None,
            own_layout_ignored: false,
            pressed: HashSet::new(),
            pressed_since: Vec::new(),
            modifier_keycodes: HashSet::new(),
            layout_tap: None,
            symbol_strokes: Vec::new(),
            mention_shown: None,
            last_key_character: String::new(),
            character_before_key: String::new(),
            position_unknown: true,
            insertion: None,
            last_committed_stale: false,
            caret_moved: false,
            early_switch_origin: None,
            early_switch_at: None,
            engine_switch_at: None,
            engine_switch_group: None,
            manual_layout_observed_at: None,
            manual_layout_source: String::new(),
            manual_layout_word: false,
            early_switch_undone: false,
            pause_deferral_logged: false,
            wait_deferral_logged: 0,
            prefix_indexes,
            pending: None,
            field_wait: None,
            presses_handled: 0,
            actions_completed: 0,
            withheld_in_hand: None,
            pending_trigger_keycode: -1,
            input_delay_due: now + INPUT_DELAY_REPORT_INTERVAL_SECONDS,
            manual_release_deadline: 0.0,
            last_committed: None,
            last_correction: None,
            last_correction_time: 0.0,
            pause_agreement_open: false,
            learning_prompt_after: None,
            correction_origin: None,
            last_requested_plan: None,
            convert_press_at: 0.0,
            convert_keycode: -1,
            convert_released: false,
            convert_outcome: None,
            selection_pending: None,
            foreign_input_active: false,
            contexts: Vec::new(),
            typo: None,
            typo_keys: KeyTemplates::default(),
            personal: Arc::new(personal),
            personal_pending: String::new(),
            term_tables: RefCell::new(None),
            hotkey_cache: RefCell::new(HashMap::new()),
            executed: None,
        };
        core.typo_corrector()?;
        core.technical_session_event("engine_initialized");
        if core.learning.migrated() != 0 {
            technical_event!(core.shared, "learning_rules_migrated", "rules": core.learning.migrated());
        }
        // The models are built: what their parsing left behind goes back to the system.
        ks_config::memory_release::release_free_memory();
        Ok(core)
    }

    /// `time.monotonic()` of the engine.
    #[inline]
    pub fn monotonic(&self) -> f64 {
        (self.clock)()
    }

    /// `bool(self.settings.get(path, default))`.
    #[inline]
    pub fn flag(&self, path: &str, default: bool) -> bool {
        self.settings.with_value(path, |value| value.map_or(default, truthy))
    }

    /// `str(self.settings.get(path, default))` as the checks against a name read it.
    pub fn text_setting(&self, path: &str, default: &str) -> String {
        self.settings.with_value(path, |value| match value {
            None => default.to_owned(),
            Some(Value::String(text)) => text.clone(),
            Some(other) => other.to_string(),
        })
    }

    /// `self.settings.get(path, [])`, a list of texts.
    pub fn words_setting(&self, path: &str) -> Vec<String> {
        self.settings.with_value(path, |value| match value {
            Some(Value::Array(items)) => items.iter().filter_map(|item| item.as_str().map(str::to_owned)).collect(),
            _ => Vec::new(),
        })
    }

    #[inline]
    pub fn in_models(&self, group: i64) -> bool {
        0 <= group && group.to::<usize>() < self.models.len()
    }

    #[inline]
    pub fn model(&self, group: i64) -> &Arc<LanguageModel> {
        &self.models[group.to::<usize>()]
    }

    /// `[group for group in self.models if group != source_group]`.
    pub fn other_groups(&self, source_group: i64) -> Vec<i64> {
        (0..self.models.len().to::<i64>()).filter(|&group| group != source_group).collect()
    }

    /// `{group: self.text_for_group(strokes, group) for group in self.models if group != source_group}`.
    pub fn alternatives_of(&self, strokes: &[Stroke], source_group: i64) -> Alternatives {
        self.other_groups(source_group).into_iter().map(|group| (group, text_for_group(strokes, group))).collect()
    }

    /// `term_frequency()`: the tables the context model holds, or read once without one.
    pub fn term_frequency(&self) -> EngineResult<Arc<TermTables>> {
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

    /// `self.focus_window or 0`.
    #[inline]
    pub fn focus_or_zero(&self) -> i64 {
        self.shared.focus_window().unwrap_or(0)
    }

    pub fn application_excluded(&self, application: &str) -> bool {
        self.shared.application_excluded(application)
    }

    /// `bounded_setting`.
    /// A numeric setting within the range the settings windows offer for it.
    ///
    /// Both windows take their ranges from the same constants, and a value edited
    /// into the settings file by hand is brought into that range here; one that is
    /// not a number at all falls back to the default.
    pub fn bounded_setting(&self, path: &str, default: f64, minimum: f64, maximum: f64) -> f64 {
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

    pub fn pause_delay(&self) -> f64 {
        self.bounded_setting("detection.pause_delay_seconds", DEFAULT_PAUSE_DELAY_SECONDS, PAUSE_DELAY_SETTING_MIN_SECONDS, PAUSE_DELAY_SETTING_MAX_SECONDS)
    }

    pub fn confidence_threshold(&self) -> f64 {
        self.bounded_setting("detection.confidence", DEFAULT_CONFIDENCE_THRESHOLD, CONFIDENCE_SETTING_MIN, CONFIDENCE_SETTING_MAX)
    }

    pub fn minimum_word_length(&self) -> i64 {
        self.bounded_setting("detection.minimum_length", DEFAULT_MINIMUM_WORD_LENGTH, MINIMUM_WORD_LENGTH_SETTING_MIN, MINIMUM_WORD_LENGTH_SETTING_MAX).to::<i64>()
    }

    /// `known_of` for an orthotactic model read beside the dictionaries, before they are there: the answer comes from
    /// `dictionaries` once they are set, which `KeySwitchEngine::new` does while it alone holds the model. `locales`
    /// are the locales the dictionaries are asked for, which is what `known_of` sees in them once they are read: a
    /// model names the locale it was asked for.
    #[must_use]
    pub fn known_later(locales: &[String], dictionaries: &Arc<OnceLock<Vec<Arc<LanguageModel>>>>) -> Option<KnownWord> {
        let same = locales.iter().map(String::as_str).eq(ORTHO_LOCALES.iter().map(|(_, locale)| *locale));
        let dictionaries = dictionaries.clone();
        same.then(|| -> KnownWord {
            Arc::new(move |script, word| {
                let group = ORTHO_LOCALES.iter().position(|(name, _)| *name == script).unwrap_or(0);
                dictionaries.get().and_then(|models| models.get(group)).is_some_and(|model| model.score(word).known)
            })
        })
    }

    /// `known` of the policy built with an engine's models, as `KeySwitchEngine.__init__` hands it over: the orthotactic
    /// model's dictionaries are these models when they are the two it would load itself.
    #[must_use]
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

/// `text_for_group`.
#[inline]
#[must_use]
pub fn text_for_group(strokes: &[Stroke], group: i64) -> String {
    let group = group.to::<i32>();
    let mut text = String::with_capacity(strokes.len() * 2);
    for stroke in strokes {
        text.push_str(stroke.character_for(group));
    }
    text
}

/// Rounded milliseconds, as `round(seconds * MILLISECONDS_PER_SECOND)`.
#[inline]
pub(crate) fn ms(seconds: f64) -> i64 {
    ks_text::py_round(seconds * MILLISECONDS_PER_SECOND).to::<i64>()
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
