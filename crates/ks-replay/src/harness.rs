//! The model editor and the fake clock a battery is typed over, and the models one replay shares between its engines.

use ks_backend::{BackendError, BackendProbe, FieldContext, FieldReader, FieldRole, FocusInfo, InputBackend, Injection, KeyEvent, KeyFilter, KeyListener};
use ks_config::config::SettingsStore;
use ks_config::history::HistoryStore;
use ks_config::logsetup;
use ks_context::input_context::{Clock, InputContext};
use ks_context::{ContextPolicy, VersionedPrefixModel};
use ks_engine::engine::{EngineCore, EngineParts};
use ks_lang::boundary_policy::BoundaryPolicy;
use ks_lang::detector::ProtectedTokens;
use ks_lang::early_switch::PrefixIndex;
use ks_lang::intent_model::{IntentModelStatus, LinearNgramModel};
use ks_lang::{LanguageModel, Resources};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

pub const PAUSE_KEYCODE: u32 = 127;
const PHYSICAL_KEY_KEYCODE_BASE: u32 = 20;
const FAKE_CLOCK_START_SECONDS: f64 = 1000.0;
const SIMULATED_KEY_HOLD_SECONDS: f64 = 0.05;
const SIMULATED_KEY_RELEASE_GAP_SECONDS: f64 = 0.03;
const MILLISECONDS_PER_SECOND: f64 = 1000.0;
// The tree that holds the packaged resources and the pinned lexicons; `KS_REPLAY_TREE` names another.
const REFERENCE_TREE: &str = "/mnt/data2/_projects/puntoswitcher";

fn reference_tree() -> PathBuf {
    std::env::var_os("KS_REPLAY_TREE").map_or_else(|| PathBuf::from(REFERENCE_TREE), PathBuf::from)
}

/// Exact US/RU key pairs shared by context training and physical replay.
#[derive(Debug, PartialEq, Eq)]
pub struct PhysicalKey {
    pub characters: [char; 2],
    pub shift: bool,
    pub keycode: u32,
}

impl PhysicalKey {
    pub fn strings(&self) -> Vec<String> {
        self.characters.iter().map(char::to_string).collect()
    }
}

pub static KEYS: LazyLock<Vec<PhysicalKey>> = LazyLock::new(|| {
    let rows = [
        (false, "`1234567890-=qwertyuiop[]\\asdfghjkl;'zxcvbnm,./ ", "ё1234567890-=йцукенгшщзхъ\\фывапролджэячсмитьбю. "),
        (true, "~!@#$%^&*()_+QWERTYUIOP{}|ASDFGHJKL:\"ZXCVBNM<>? ", "Ё!\"№;%:?*()_+ЙЦУКЕНГШЩЗХЪ/ФЫВАПРОЛДЖЭЯЧСМИТЬБЮ, "),
    ];
    let mut keys = Vec::new();
    for (shift, us, ru) in rows {
        for (index, (left, right)) in us.chars().zip(ru.chars()).enumerate() {
            keys.push(PhysicalKey { characters: [left, right], shift, keycode: index as u32 + PHYSICAL_KEY_KEYCODE_BASE });
        }
    }
    keys
});

/// `key_of.get(char)`: the first key that types the character in either layout, as its place in `KEYS`.
pub fn key_of_char(character: char) -> Option<usize> {
    KEYS.iter().position(|key| key.characters.contains(&character))
}

struct EditorState {
    text: Vec<char>,
    caret: usize,
    group: i32,
    window: i64,
    application: String,
    enter: Option<KeyEvent>,
    delivered: Option<String>,
}

/// Model editing and submission before the passive observer sees a key.
pub struct Backend {
    state: Mutex<EditorState>,
}

impl Backend {
    fn new(application: &str, group: i32, text: Vec<char>) -> Backend {
        let caret = text.len();
        Backend { state: Mutex::new(EditorState { text, caret, group, window: 1, application: application.to_owned(), enter: None, delivered: None }) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, EditorState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn text(&self) -> Vec<char> {
        self.lock().text.clone()
    }

    pub fn set_text(&self, text: Vec<char>) {
        self.lock().text = text;
    }

    pub fn caret(&self) -> usize {
        self.lock().caret
    }

    pub fn set_caret(&self, caret: usize) {
        self.lock().caret = caret;
    }

    pub fn group(&self) -> i32 {
        self.lock().group
    }

    pub fn set_group(&self, group: i32) {
        self.lock().group = group;
    }

    /// `text[start:stop]`.
    pub fn slice(&self, start: usize, stop: usize) -> Vec<char> {
        self.lock().text.iter().skip(start).take(stop.saturating_sub(start)).copied().collect()
    }

    pub fn insert_at_caret(&self, inserted: &str) {
        let mut state = self.lock();
        let caret = state.caret;
        let added: Vec<char> = inserted.chars().collect();
        state.text.splice(caret..caret, added.iter().copied());
        state.caret += added.len();
    }

    /// `complete_action` delivers this Return while it is watched: the replay reads the text it submits.
    pub fn watch_enter(&self, action: KeyEvent) {
        let mut state = self.lock();
        state.enter = Some(action);
        state.delivered = None;
    }

    pub fn stop_watching_enter(&self) -> Option<String> {
        let mut state = self.lock();
        state.enter = None;
        state.delivered.take()
    }

    fn type_into(state: &mut EditorState, event: &KeyEvent) {
        if !event.pressed {
            return;
        }
        match event.key_name.as_str() {
            "Return" | "KP_Enter" => {
                state.text.clear();
                state.caret = 0;
            }
            "Tab" | "ISO_Left_Tab" => {
                state.window += 1;
                state.text = "another field".chars().collect();
                state.caret = state.text.len();
            }
            "Left" => state.caret = state.caret.saturating_sub(1),
            "BackSpace" => {
                if state.caret != 0 {
                    let caret = state.caret;
                    state.text.remove(caret - 1);
                    state.caret -= 1;
                }
            }
            _ if !event.character.is_empty() => {
                let caret = state.caret;
                let added: Vec<char> = event.character.chars().collect();
                state.text.splice(caret..caret, added.iter().copied());
                state.caret += added.len();
            }
            _ => {}
        }
    }

    pub fn type_key(&self, event: &KeyEvent) {
        Self::type_into(&mut self.lock(), event);
    }
}

impl InputBackend for Backend {
    fn start(&self, _listener: KeyListener) -> Result<(), BackendError> {
        Ok(())
    }

    fn set_key_filter(&self, _predicate: Option<KeyFilter>) {}

    fn hold_input(&self) {}

    fn release_input(&self) -> Result<i32, BackendError> {
        Ok(0)
    }

    fn complete_action(&self, deliver: bool) -> Result<i32, BackendError> {
        let mut state = self.lock();
        if deliver && let Some(action) = state.enter.clone() {
            state.delivered = Some(state.text.iter().collect());
            Self::type_into(&mut state, &KeyEvent { deferred: false, ..action });
        }
        Ok(0)
    }

    fn stop(&self) {}

    fn close(&self) {}

    fn current_group(&self) -> Result<i32, BackendError> {
        Ok(self.lock().group)
    }

    fn switch_group(&self, group: i32) -> Result<(), BackendError> {
        self.lock().group = group;
        Ok(())
    }

    fn active_application(&self) -> String {
        self.lock().application.clone()
    }

    fn focused_window(&self) -> Option<FocusInfo> {
        Some(FocusInfo { window: self.lock().window, own: false, isolated_layout: false })
    }

    fn probe(&self) -> BackendProbe {
        BackendProbe { available: true, session_type: "x11".to_owned(), current_group: self.group(), ..BackendProbe::default() }
    }

    fn inject_correction(&self, injection: Injection<'_>) -> Result<i32, BackendError> {
        let mut state = self.lock();
        let word = injection.strokes;
        let count = word.len() + injection.trailing.len() + usize::from(injection.boundary.is_some()) + injection.late.len();
        if state.caret < count {
            return Err(BackendError::Failed("Correction attempted to erase text outside the word".to_owned()));
        }
        let start = state.caret - count;
        // The kept tail and the keys after it are typed in the layout the tail was typed in.
        let final_group = if injection.kept_tail == 0 {
            injection.target_group
        } else {
            injection.source_group.unwrap_or_else(|| word[0].group)
        };
        let mut replacement = String::new();
        for (index, stroke) in word.iter().enumerate() {
            let group = if index >= word.len() - injection.kept_tail { final_group } else { injection.target_group };
            replacement.push_str(stroke.character_for(group));
        }
        for stroke in injection.trailing {
            replacement.push_str(&stroke.character);
        }
        if let Some(boundary) = injection.boundary {
            replacement.push_str(&boundary.character);
        }
        for stroke in injection.late {
            replacement.push_str(stroke.character_for(final_group));
        }
        let added: Vec<char> = replacement.chars().collect();
        let caret = state.caret;
        state.text.splice(start..caret, added.iter().copied());
        state.caret = start + added.len();
        state.group = final_group;
        Ok(0)
    }

    fn erases_any_length(&self) -> bool {
        false
    }
}

/// The field as UI Automation reads it: the text before and after the caret.
pub struct CaretReader {
    pub backend: Arc<Backend>,
}

impl FieldReader for CaretReader {
    fn read(&self, application: &str, _window: i64) -> Option<FieldContext> {
        let state = self.backend.lock();
        let mut field = FieldContext::new(application, "1");
        field.before = state.text[..state.caret].iter().collect();
        field.after = state.text[state.caret..].iter().collect();
        field.role = FieldRole::Text;
        field.source = "uia".to_owned();
        Some(field)
    }

    fn status(&self) -> String {
        "available".to_owned()
    }
}

/// A field no reader can read (the log's role `unknown`): the engine keeps to the keys it observed.
pub struct UnreadField;

impl FieldReader for UnreadField {
    fn read(&self, _application: &str, _window: i64) -> Option<FieldContext> {
        None
    }

    fn status(&self) -> String {
        "unavailable".to_owned()
    }
}

/// `_Clock`: the seconds as a shared atomic, read by the engine and by the text it observes.
#[derive(Clone)]
struct FakeClock(Arc<AtomicU64>);

impl FakeClock {
    fn new() -> FakeClock {
        FakeClock(Arc::new(AtomicU64::new(FAKE_CLOCK_START_SECONDS.to_bits())))
    }

    fn now(&self) -> f64 {
        f64::from_bits(self.0.load(Ordering::Relaxed))
    }

    fn advance(&self, seconds: f64) {
        self.0.store((self.now() + seconds).to_bits(), Ordering::Relaxed);
    }

    fn read(&self) -> Clock {
        let clock = self.clone();
        Arc::new(move || clock.now())
    }
}

/// `_Session`: the keys of one field on the fake clock, and the five timers the harness pumps.
pub struct Session {
    pub core: EngineCore,
    pub backend: Arc<Backend>,
    clock: FakeClock,
}

impl Session {
    fn timers(&mut self) {
        self.core._expire_deferred_action().expect("a timer of the harness");
        self.core._expire_manual_correction();
        self.core._poll_current_group().expect("a timer of the harness");
        self.core._maybe_correct_after_pause(None).expect("a timer of the harness");
        self.core._expire_learning_prompt(None);
    }

    pub fn idle(&mut self, seconds: f64) {
        self.clock.advance(seconds);
        self.timers();
    }

    pub fn select_layout(&mut self, group: i32) {
        self.backend.set_group(group);
        self.core._poll_current_group().expect("the layout of the editor");
    }

    pub fn handle(&mut self, event: KeyEvent) {
        self.core._handle(Arc::new(event)).expect("a key the engine handles");
    }

    fn stamp(&self) -> i64 {
        ks_text::py_round(self.clock.now() * MILLISECONDS_PER_SECOND) as i64
    }

    pub fn tap(&mut self, event: KeyEvent) {
        self.clock.advance(SIMULATED_KEY_HOLD_SECONDS);
        let pressed = KeyEvent { timestamp: self.stamp(), ..event };
        self.backend.type_key(&pressed);
        self.handle(pressed.clone());
        self.clock.advance(SIMULATED_KEY_RELEASE_GAP_SECONDS);
        let released = KeyEvent { pressed: false, timestamp: self.stamp(), ..pressed };
        self.backend.type_key(&released);
        self.handle(released);
        self.timers();
    }

    pub fn tap_named(&mut self, name: &str) {
        let group = self.backend.group();
        self.tap(KeyEvent {
            pressed: true,
            key_name: name.to_owned(),
            characters: vec![String::new(), String::new()],
            group,
            ..KeyEvent::default()
        });
    }

    pub fn pointer(&mut self) {
        let group = self.backend.group();
        self.handle(KeyEvent {
            pressed: true,
            key_name: "Pointer".to_owned(),
            characters: vec![String::new(), String::new()],
            group,
            ..KeyEvent::default()
        });
    }
}

/// `reference_models(False)`: the pinned onboard lexicons plus the packaged supplements, without a spell checker.
fn reference_models(resources: &Arc<Resources>) -> Vec<Arc<LanguageModel>> {
    ["en_US", "ru_RU"]
        .iter()
        .map(|locale| {
            let words = resources.supplement_words(locale).expect("the packaged supplement");
            let roots = vec![reference_tree().join("model/intent_v1/sources")];
            Arc::new(LanguageModel::load(locale, words.iter(), &roots, LanguageModel::disabled_speller()))
        })
        .collect()
}

/// What the Python's patches hand every engine of a replay: one set of models, one context policy going from engine to
/// engine as the one model object does.
pub struct Bundle {
    resources: Arc<Resources>,
    models: Vec<Arc<LanguageModel>>,
    intent_model: Option<Arc<LinearNgramModel>>,
    intent_model_status: IntentModelStatus,
    protected_tokens: Arc<ProtectedTokens>,
    boundary_model: Option<Arc<BoundaryPolicy>>,
    prefix_model: Option<Arc<VersionedPrefixModel>>,
    prefix_indexes: Vec<Arc<PrefixIndex>>,
    context_policy: Option<ContextPolicy>,
    root: PathBuf,
    debug: bool,
}

impl Bundle {
    pub fn load(debug_log: Option<&Path>) -> Bundle {
        let resources = Resources::new(reference_tree().join("src/keyswitch/resources"));
        let models = reference_models(&resources);
        let (intent_model, intent_model_status) = resources.intent_model();
        let known = EngineCore::known_of(&models);
        let context_policy = ContextPolicy::new(&resources, None, known, Some(Arc::new(|| 0.0)));
        // The reference lexicon names no Hunspell dictionary, so the index holds the words of that lexicon alone.
        let prefix_indexes = models.iter().map(|model| Arc::new(PrefixIndex::from_lexicon(model.frequencies.clone(), &model.source))).collect();
        let prefix_model = VersionedPrefixModel::default_model(&resources.resource_dir().join("models")).map(Arc::new);
        let root = std::env::temp_dir().join(format!("ks-replay-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("a folder for the stores");
        let debug = debug_log.is_some();
        if let Some(path) = debug_log {
            let _ = std::fs::remove_file(path);
            let handler = logsetup::RotatingFileHandler::new(path, 1 << 40, 0).expect("the debug log");
            logsetup::install_handler(&handler);
            logsetup::set_root_level(logsetup::Level::Info);
        }
        Bundle {
            protected_tokens: resources.protected_tokens(),
            boundary_model: resources.boundary_policy(),
            intent_model,
            intent_model_status,
            prefix_model,
            prefix_indexes,
            models,
            context_policy: Some(context_policy),
            resources,
            root,
            debug,
        }
    }

    /// `Field(app, group, text)`: a new engine and a new clock over the shared models.
    pub fn session(&mut self, application: &str, group: i32, text: Vec<char>) -> Session {
        let settings = Arc::new(SettingsStore::new(Some(self.root.join("settings.json"))));
        // The gate compares the layout decision: the typo correction is measured apart.
        settings.set("detection.correct_typos", json!(false), false).expect("in memory");
        if self.debug {
            settings.set("diagnostics.technical_logging", json!(true), false).expect("in memory");
        }
        let backend = Arc::new(Backend::new(application, group, text));
        let clock = FakeClock::new();
        let mut context_policy = self.context_policy.take().expect("one engine at a time");
        context_policy.stream = InputContext::with_clock(clock.read());
        context_policy.reader = Some(Arc::new(CaretReader { backend: backend.clone() }));
        let history = Arc::new(HistoryStore::new(Some(self.root.join("history.jsonl")), 200));
        let mut core = EngineCore::from_parts(EngineParts {
            settings,
            history,
            backend: backend.clone(),
            learning: None,
            backend_label: "X11 RECORD + XTEST".to_owned(),
            models: self.models.clone(),
            intent_model: self.intent_model.clone(),
            intent_model_status: self.intent_model_status.clone(),
            protected_tokens: self.protected_tokens.clone(),
            context_policy,
            boundary_model: self.boundary_model.clone(),
            prefix_model: self.prefix_model.clone(),
            prefix_indexes: self.prefix_indexes.clone(),
            resource_dir: self.resources.resource_dir().to_path_buf(),
            clock: clock.read(),
            platform_reader: None,
        })
        .expect("an engine over the shared models");
        core.executed = Some(Vec::new());
        Session { core, backend, clock }
    }

    /// `field.close()`: the policy goes on to the next engine.
    pub fn give_back(&mut self, session: Session) {
        self.context_policy = Some(session.core.context_policy);
        let _ = std::fs::remove_file(self.root.join("history.jsonl"));
    }

    pub fn clean(&self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
