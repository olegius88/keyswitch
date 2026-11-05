//! What threads other than the worker touch: atomics the keyboard hook reads, the inbox, and the guarded snapshot.

use super::logging::technical_event;
use super::types::*;
use crate::constants::*;
use ks_backend::{FocusInfo, InputBackend, KeyDisposition, KeyEvent, KeyFilter, KeyListener};
use ks_config::config::SettingsStore;
use ks_config::pycompat::truthy;
use ks_context::input_context::Clock;
use ks_text::pystr;
use serde_json::{Map, Value};
use std::cell::Cell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, RwLock};
use std::time::Duration;

const NO_WINDOW: i64 = i64::MIN;
const ACTION_RETURN: u8 = 1;
const ACTION_KP_ENTER: u8 = 2;
const ACTION_TAB: u8 = 4;

thread_local! {
    /// The thread that holds `EngineCore` is changing a setting itself and applies the change right after it.
    pub(crate) static SETTING_FROM_CORE: Cell<bool> = const { Cell::new(false) };
}

/// One entry of `_events`: a key, a layout picked from the menu, or the end of the worker (the Python's None).
#[derive(Clone, Debug)]
pub enum QueueItem {
    Key(Stroke),
    LayoutSelection(i64),
    Stop,
}

/// Work another thread leaves for the worker, which alone may touch the engine's state.
#[derive(Clone, Debug)]
pub enum Command {
    SettingsChanged(String, Value),
}

/// What the worker takes next.
pub(crate) enum Next {
    Command(Command),
    Item(QueueItem),
    Timeout,
}

#[derive(Default)]
struct Inbox {
    events: VecDeque<QueueItem>,
    commands: VecDeque<Command>,
}

pub type SnapshotCallback = Arc<dyn Fn(&EngineSnapshot) + Send + Sync>;
pub type CorrectionCallback = Arc<dyn Fn(&CorrectionPlan) + Send + Sync>;
pub type LearningPromptCallback = Arc<dyn Fn(Option<&LearningPrompt>) + Send + Sync>;
pub type RuleRequestCallback = Arc<dyn Fn(&LearningPrompt) + Send + Sync>;

/// What the Python's `_lock` guards. It is never held while calling a callback, the backend or the settings.
struct Guarded {
    _snapshot: EngineSnapshot,
    _callbacks: Vec<SnapshotCallback>,
    _correction_callbacks: Vec<CorrectionCallback>,
    _learning_prompt: Option<LearningPrompt>,
    _learning_prompt_deadline: Option<f64>,
    _learning_prompt_callbacks: Vec<LearningPromptCallback>,
    _rule_request_callbacks: Vec<RuleRequestCallback>,
}

pub struct Shared {
    pub settings: Arc<SettingsStore>,
    pub backend: Arc<dyn InputBackend>,
    pub(crate) clock: Clock,
    pub(crate) models_len: usize,
    swallows_hotkeys: bool,
    // Read from the keyboard hook without a lock: see `consumes_key`.
    _prompt_key_deadline: AtomicU64,
    _input_suspended: AtomicBool,
    _test_field_focused: AtomicBool,
    // Input other programs inject (remote control, macros) counts as typing
    // only with detection.injected_input; read by the hook, so cached here.
    _injected_input: AtomicBool,
    _sensitive_context_window: AtomicI64,
    _focus_window: AtomicI64,
    _action_keys: AtomicU8,
    /// Swapped whole when the settings change; the hook only tries it, so it never waits.
    _own_hotkeys: RwLock<Arc<Vec<Hotkey>>>,
    pub _typed_events: AtomicI64,
    pub _typed_presses: AtomicI64,
    pub _input_overflow: AtomicBool,
    pub(crate) _running: AtomicBool,
    inbox: Mutex<Inbox>,
    wake: Condvar,
    guarded: Mutex<Guarded>,
    pub(crate) _log_redactor: Mutex<ks_config::log_privacy::LogRedactor>,
    /// The settings' unsubscribe call, made when the engine goes.
    unsubscribe: Mutex<Option<ks_config::config::Unsubscribe>>,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    pub(crate) fn new(
        settings: Arc<SettingsStore>, backend: Arc<dyn InputBackend>, clock: Clock, models_len: usize, snapshot: EngineSnapshot,
    ) -> Arc<Shared> {
        let swallows_hotkeys = backend.swallows_hotkeys();
        let shared = Arc::new(Shared {
            settings,
            backend,
            clock,
            models_len,
            swallows_hotkeys,
            _prompt_key_deadline: AtomicU64::new(0f64.to_bits()),
            _input_suspended: AtomicBool::new(false),
            _test_field_focused: AtomicBool::new(false),
            _injected_input: AtomicBool::new(false),
            _sensitive_context_window: AtomicI64::new(NO_WINDOW),
            _focus_window: AtomicI64::new(NO_WINDOW),
            _action_keys: AtomicU8::new(0),
            _own_hotkeys: RwLock::new(Arc::new(Vec::new())),
            _typed_events: AtomicI64::new(0),
            _typed_presses: AtomicI64::new(0),
            _input_overflow: AtomicBool::new(false),
            _running: AtomicBool::new(false),
            inbox: Mutex::new(Inbox::default()),
            wake: Condvar::new(),
            guarded: Mutex::new(Guarded {
                _snapshot: snapshot,
                _callbacks: Vec::new(),
                _correction_callbacks: Vec::new(),
                _learning_prompt: None,
                _learning_prompt_deadline: None,
                _learning_prompt_callbacks: Vec::new(),
                _rule_request_callbacks: Vec::new(),
            }),
            _log_redactor: Mutex::new(ks_config::log_privacy::LogRedactor::new()),
            unsubscribe: Mutex::new(None),
        });
        shared.refresh_hook_settings();
        shared
    }

    /// `self.settings.subscribe(self._settings_changed)`. What the hook reads changes at once, on the caller's thread; the
    /// rest of the change is the worker's.
    pub(crate) fn subscribe_settings(shared: &Arc<Shared>) {
        let weak = Arc::downgrade(shared);
        let unsubscribe = shared.settings.subscribe(Arc::new(move |path: &str, value: &Value| {
            if let Some(shared) = weak.upgrade() {
                shared.refresh_hook_settings();
                if !SETTING_FROM_CORE.get() {
                    shared.post(Command::SettingsChanged(path.to_owned(), value.clone()));
                }
            }
        }));
        *locked(&shared.unsubscribe) = Some(unsubscribe);
    }

    fn flag(&self, path: &str, default: bool) -> bool {
        self.settings.with_value(path, |value| value.map_or(default, truthy))
    }

    /// `_action_keys`, `_own_hotkeys` and `_injected_input`, read from the settings for the hook.
    pub(crate) fn refresh_hook_settings(&self) {
        self._action_keys.store(self._configured_action_keys(), Ordering::Relaxed);
        let hotkeys = Arc::new(self._configured_hotkeys());
        *self._own_hotkeys.write().unwrap_or_else(PoisonError::into_inner) = hotkeys;
        self._injected_input.store(self.flag("detection.injected_input", false), Ordering::Relaxed);
    }

    fn _configured_action_keys(&self) -> u8 {
        if !self.flag("enabled", true) {
            return 0;
        }
        let mut keys = 0;
        if self.flag("detection.correct_on_enter", true) {
            keys |= ACTION_RETURN | ACTION_KP_ENTER;
        }
        if self.flag("detection.correct_on_tab", true) {
            keys |= ACTION_TAB;
        }
        keys
    }

    /// Every hotkey of the settings, parsed once for the keyboard hook (see `consumes_key`).
    fn _configured_hotkeys(&self) -> Vec<Hotkey> {
        self.settings.with_value("hotkeys", |configured| match configured {
            Some(Value::Object(map)) => map
                .values()
                .map(|value| match value {
                    Value::String(text) => Hotkey::new(text),
                    other => Hotkey::new(&other.to_string()),
                })
                .collect(),
            _ => Vec::new(),
        })
    }

    pub fn _prompt_key_deadline(&self) -> f64 {
        f64::from_bits(self._prompt_key_deadline.load(Ordering::Relaxed))
    }

    pub(crate) fn set_prompt_key_deadline(&self, deadline: f64) {
        self._prompt_key_deadline.store(deadline.to_bits(), Ordering::Relaxed);
    }

    pub fn _sensitive_context_window(&self) -> Option<i64> {
        let window = self._sensitive_context_window.load(Ordering::Relaxed);
        (window != NO_WINDOW).then_some(window)
    }

    pub(crate) fn set_sensitive_context_window(&self, window: Option<i64>) {
        self._sensitive_context_window.store(window.unwrap_or(NO_WINDOW), Ordering::Relaxed);
    }

    pub fn _focus_window(&self) -> Option<i64> {
        let window = self._focus_window.load(Ordering::Relaxed);
        (window != NO_WINDOW).then_some(window)
    }

    pub fn set_focus_window(&self, window: i64) {
        self._focus_window.store(window, Ordering::Relaxed);
    }

    pub fn _input_suspended(&self) -> bool {
        self._input_suspended.load(Ordering::Relaxed)
    }

    pub fn _test_field_focused(&self) -> bool {
        self._test_field_focused.load(Ordering::Relaxed)
    }

    pub(crate) fn _accepts_injected(&self, event: &KeyEvent) -> bool {
        // Whether input another program injected is treated as the user's typing.
        //
        // A key that carries a character instead of a key position (VK_PACKET on
        // Windows) never is: it cannot be typed again in another layout.
        self._injected_input.load(Ordering::Relaxed) && event.key_name != ks_backend::constants::UNICODE_PACKET_KEY_NAME
    }

    /// `consumes_key`.
    /// Whether KeySwitch answers this key itself and the window must not.
    ///
    /// Called from the keyboard hook while the system waits for the answer,
    /// so it only reads plain attributes: taking the engine lock here would
    /// hold every key of the system for as long as the worker keeps the lock
    /// (and past the low-level timeout, after which Microsoft documents that
    /// Windows removes the hook without telling anyone).
    pub fn consumes_key(&self, event: &KeyEvent) -> KeyDisposition {
        if !event.pressed || event.synthetic {
            return KeyDisposition::Pass;
        }
        if self._input_suspended() || (event.foreign && !self._accepts_injected(event)) {
            return KeyDisposition::Pass;
        }
        if self._sensitive_context_window.load(Ordering::Relaxed) != NO_WINDOW {
            return KeyDisposition::Pass;
        }
        if is_modifier_key(&event.key_name) {
            return KeyDisposition::Pass;
        }
        if event.control() || event.alt() || event.super_key() || event.shift() {
            self.set_prompt_key_deadline(0.0);
            // A backend that can keep a key from the window keeps KeySwitch's own hotkeys: on macOS
            // TextEdit typed Control+Option+Z as U+001A, which the undo then deleted in place of the
            // word's last letter. The Win32 hook and XRecord leave them to the window, as before.
            return if self.swallows_hotkeys
                && self._own_hotkeys.try_read().is_ok_and(|hotkeys| hotkeys.iter().any(|hotkey| hotkey.matches(event)))
            {
                KeyDisposition::Consume
            } else {
                KeyDisposition::Pass
            };
        }
        if (self.clock)() < self._prompt_key_deadline() && PROMPT_KEYS.contains(&event.key_name.as_str()) {
            return KeyDisposition::Consume;
        }
        self.set_prompt_key_deadline(0.0);
        let action = match event.key_name.as_str() {
            "Return" => ACTION_RETURN,
            "KP_Enter" => ACTION_KP_ENTER,
            "Tab" => ACTION_TAB,
            _ => 0,
        };
        if action & self._action_keys.load(Ordering::Relaxed) != 0 { KeyDisposition::Defer } else { KeyDisposition::Pass }
    }

    /// `enqueue`: on the hook's thread, behind the inbox's short lock, which nothing holds while it blocks.
    pub fn enqueue(&self, event: KeyEvent) {
        if event.synthetic {
            return;
        }
        if event.key_name == "Pointer" {
            self.set_prompt_key_deadline(0.0);
        }
        // Physical events and presses are separate: a key-up is not text.
        self._typed_events.fetch_add(1, Ordering::Relaxed);
        self._typed_presses.fetch_add(i64::from(event.pressed), Ordering::Relaxed);
        let mut inbox = locked(&self.inbox);
        if inbox.events.len() < ENGINE_EVENT_QUEUE_MAX_SIZE {
            inbox.events.push_back(QueueItem::Key(Arc::new(event)));
            drop(inbox);
            self.wake.notify_one();
        } else {
            drop(inbox);
            // The hook must not mutate the worker's state or invoke UI/file
            // callbacks. The worker invalidates the text before its next action.
            self._input_overflow.store(true, Ordering::Relaxed);
        }
    }

    pub(crate) fn key_filter(self: &Arc<Self>) -> KeyFilter {
        let shared = self.clone();
        Arc::new(move |event: &KeyEvent| shared.consumes_key(event))
    }

    pub(crate) fn key_listener(self: &Arc<Self>) -> KeyListener {
        let shared = self.clone();
        Arc::new(move |event: KeyEvent| shared.enqueue(event))
    }

    pub(crate) fn post(&self, command: Command) {
        locked(&self.inbox).commands.push_back(command);
        self.wake.notify_one();
    }

    /// `self._events.put(item)`; false when the queue is full.
    pub fn put_nowait(&self, item: QueueItem) -> bool {
        let mut inbox = locked(&self.inbox);
        if inbox.events.len() >= ENGINE_EVENT_QUEUE_MAX_SIZE {
            return false;
        }
        inbox.events.push_back(item);
        drop(inbox);
        self.wake.notify_one();
        true
    }

    /// The next thing for the worker, waiting up to `timeout` seconds.
    pub(crate) fn next(&self, timeout: f64) -> Next {
        let duration = Duration::from_secs_f64(timeout.max(0.0));
        let mut inbox = locked(&self.inbox);
        let deadline = std::time::Instant::now() + duration;
        loop {
            if let Some(command) = inbox.commands.pop_front() {
                return Next::Command(command);
            }
            if let Some(item) = inbox.events.pop_front() {
                return Next::Item(item);
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                return Next::Timeout;
            }
            inbox = self.wake.wait_timeout(inbox, deadline - now).unwrap_or_else(PoisonError::into_inner).0;
        }
    }

    /// A setting another thread changed, for `apply_commands`.
    pub(crate) fn take_command(&self) -> Option<Command> {
        locked(&self.inbox).commands.pop_front()
    }

    /// `tuple(self._events.queue)`, under the inbox's lock.
    pub(crate) fn queued(&self) -> Vec<QueueItem> {
        locked(&self.inbox).events.iter().cloned().collect()
    }

    /// Everything queued, taken out; `put_back` returns what stays, in front of whatever arrived since.
    pub(crate) fn take_queued(&self) -> Vec<QueueItem> {
        locked(&self.inbox).events.drain(..).collect()
    }

    pub(crate) fn put_back(&self, items: Vec<QueueItem>) {
        let mut inbox = locked(&self.inbox);
        for item in items.into_iter().rev() {
            inbox.events.push_front(item);
        }
        drop(inbox);
        self.wake.notify_one();
    }

    pub fn qsize(&self) -> usize {
        locked(&self.inbox).events.len()
    }

    /// A key queued for the replay to take: the harness's own `_events.get_nowait`.
    pub fn get_nowait(&self) -> Option<QueueItem> {
        locked(&self.inbox).events.pop_front()
    }

    pub(crate) fn _application_excluded(&self, application: &str) -> bool {
        if let Some(window) = self._sensitive_context_window()
            && Some(window) == self._focus_window()
        {
            return true;
        }
        let normalized = pystr::casefold(application);
        self.settings.with_value("exclusions.applications", |applications| match applications {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(Value::as_str)
                .any(|item| !pystr::strip(item).is_empty() && normalized.contains(&pystr::casefold(item))),
            _ => false,
        })
    }

    /// `self.snapshot`.
    pub fn snapshot(&self) -> EngineSnapshot {
        locked(&self.guarded)._snapshot.clone()
    }

    pub(crate) fn current_group(&self) -> i64 {
        locked(&self.guarded)._snapshot.current_group
    }

    pub(crate) fn correction_count(&self) -> i64 {
        locked(&self.guarded)._snapshot.correction_count
    }

    pub fn subscribe(&self, callback: SnapshotCallback) {
        locked(&self.guarded)._callbacks.push(callback.clone());
        callback(&self.snapshot());
    }

    pub fn subscribe_corrections(&self, callback: CorrectionCallback) {
        locked(&self.guarded)._correction_callbacks.push(callback);
    }

    pub(crate) fn correction_callbacks(&self) -> Vec<CorrectionCallback> {
        locked(&self.guarded)._correction_callbacks.clone()
    }

    /// `self.learning_prompt`.
    pub fn learning_prompt(&self) -> Option<LearningPrompt> {
        locked(&self.guarded)._learning_prompt.clone()
    }

    pub fn subscribe_learning_prompts(&self, callback: LearningPromptCallback) {
        let prompt = {
            let mut guarded = locked(&self.guarded);
            guarded._learning_prompt_callbacks.push(callback.clone());
            guarded._learning_prompt.clone()
        };
        callback(prompt.as_ref());
    }

    /// Be told when Enter on the prompt asks for the rule window.
    pub fn subscribe_rule_requests(&self, callback: RuleRequestCallback) {
        locked(&self.guarded)._rule_request_callbacks.push(callback);
    }

    /// Enter on the prompt: open the rule window, filled in from the prompt.
    ///
    /// Nothing is remembered yet. A rule exists only once the user presses OK in
    /// that window, which calls :meth:`add_learning_rule`.
    pub fn confirm_learning_prompt(&self, prompt: Option<&LearningPrompt>) -> bool {
        let (current, callbacks, requests) = {
            let mut guarded = locked(&self.guarded);
            let Some(current) = guarded._learning_prompt.clone() else {
                return false;
            };
            if prompt.is_some_and(|prompt| *prompt != current) {
                return false;
            }
            guarded._learning_prompt = None;
            guarded._learning_prompt_deadline = None;
            self.set_prompt_key_deadline(0.0);
            (current, guarded._learning_prompt_callbacks.clone(), guarded._rule_request_callbacks.clone())
        };
        technical_event!(
            self, "learning_prompt_confirmed",
            "source_group": current.source_group,
            "target_group": current.target_group,
            "application": current.application,
            "action": current.action.as_str(),
            "has_word": !current.original.is_empty(),
        );
        for callback in callbacks {
            callback(None);
        }
        for request in requests {
            request(&current);
        }
        true
    }

    pub fn dismiss_learning_prompt(&self, prompt: Option<&LearningPrompt>, reason: &str) -> bool {
        let (current, callbacks) = {
            let mut guarded = locked(&self.guarded);
            let Some(current) = guarded._learning_prompt.clone() else {
                return false;
            };
            if prompt.is_some_and(|prompt| *prompt != current) {
                return false;
            }
            guarded._learning_prompt = None;
            guarded._learning_prompt_deadline = None;
            self.set_prompt_key_deadline(0.0);
            (current, guarded._learning_prompt_callbacks.clone())
        };
        technical_event!(
            self, "learning_prompt_dismissed",
            "reason": reason,
            "source_group": current.source_group,
            "target_group": current.target_group,
            "application": current.application,
        );
        for callback in callbacks {
            callback(None);
        }
        true
    }

    pub fn _show_learning_prompt(&self, prompt: LearningPrompt) {
        let now = (self.clock)();
        let callbacks = {
            let mut guarded = locked(&self.guarded);
            guarded._learning_prompt = Some(prompt.clone());
            let deadline = now + LEARNING_PROMPT_TIMEOUT_SECONDS;
            guarded._learning_prompt_deadline = Some(deadline);
            self.set_prompt_key_deadline(deadline);
            guarded._learning_prompt_callbacks.clone()
        };
        let excluded = self._application_excluded(&prompt.application);
        technical_event!(
            self, "learning_prompt_shown",
            "original": if excluded { "<redacted>" } else { prompt.original.as_str() },
            "replacement": if excluded { "<redacted>" } else { prompt.replacement.as_str() },
            "source_group": prompt.source_group,
            "target_group": prompt.target_group,
            "application": prompt.application,
            "action": prompt.action.as_str(),
            "timeout_seconds": LEARNING_PROMPT_TIMEOUT_SECONDS,
        );
        for callback in callbacks {
            callback(Some(&prompt));
        }
    }

    pub fn _expire_learning_prompt(&self, now: Option<f64>) -> bool {
        let Some(deadline) = locked(&self.guarded)._learning_prompt_deadline else {
            return false;
        };
        let current_time = now.unwrap_or_else(|| (self.clock)());
        if current_time < deadline {
            return false;
        }
        self.dismiss_learning_prompt(None, "timeout")
    }

    /// While the rule window has the keyboard its fields are typed into, not text to correct.
    ///
    /// The window reports when it gains and loses the keyboard, so typing elsewhere
    /// while it stays open is corrected as usual.
    pub fn set_rule_editor_open(&self, opened: bool) {
        if opened == self._input_suspended() {
            return;
        }
        self._input_suspended.store(opened, Ordering::Relaxed);
        technical_event!(self, if opened { "rule_editor_focused" } else { "rule_editor_left" });
    }

    /// The test field of the settings window gained or lost the keyboard.
    ///
    /// The windows of KeySwitch itself are not where the user types (FocusInfo.own): moving to
    /// the settings window or the learning prompt is no move to another program, and Windows
    /// keeps a layout per window, so a layout seen there says nothing of the text and is ignored.
    /// The test field is the exception: it is there to try KeySwitch as in any other program,
    /// and a layout picked by hand while it has the keyboard must protect the next word there
    /// too. Only the window knows which of its controls has the keyboard, so it reports it here.
    pub fn set_test_field_focused(&self, focused: bool) {
        if focused == self._test_field_focused() {
            return;
        }
        self._test_field_focused.store(focused, Ordering::Relaxed);
        technical_event!(self, if focused { "test_field_focused" } else { "test_field_left" });
    }

    pub fn _update(&self, update: Update) {
        let (callbacks, snapshot) = {
            let mut guarded = locked(&self.guarded);
            let current = &mut guarded._snapshot;
            if let Some(value) = update.running {
                current.running = value;
            }
            if let Some(value) = update.enabled {
                current.enabled = value;
            }
            if let Some(value) = update.backend {
                current.backend = value;
            }
            if let Some(value) = update.current_group {
                current.current_group = value;
            }
            if let Some(value) = update.current_word {
                current.current_word = value;
            }
            if let Some(value) = update.correction_count {
                current.correction_count = value;
            }
            if let Some(value) = update.last_action {
                current.last_action = value;
            }
            if let Some(value) = update.last_error {
                current.last_error = value;
            }
            if let Some(value) = update.context_action {
                current.context_action = value;
            }
            if let Some(value) = update.context_model {
                current.context_model = value;
            }
            if guarded._callbacks.is_empty() {
                return;
            }
            (guarded._callbacks.clone(), guarded._snapshot.clone())
        };
        for callback in callbacks {
            callback(&snapshot);
        }
    }

    pub(crate) fn technical_logging(&self) -> bool {
        self.settings.with_value("diagnostics.technical_logging", |value| value.is_some_and(truthy))
    }

    /// The technical record, after its words that look like a password went into stars (log_privacy).
    pub(crate) fn emit_technical(&self, payload: Map<String, Value>) {
        let record = locked(&self._log_redactor).record(&payload);
        let options = ks_text::JsonOptions {
            ensure_ascii: false,
            indent: None,
            separators: Some((",".to_owned(), ":".to_owned())),
            sort_keys: true,
        };
        let message = format!("TECHNICAL {}", ks_text::json_dumps(&Value::Object(record), &options));
        ks_config::logsetup::info("keyswitch.engine", &message);
    }

    pub(crate) fn focused_window(&self) -> Option<FocusInfo> {
        self.backend.focused_window()
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        if let Some(unsubscribe) = locked(&self.unsubscribe).take() {
            unsubscribe();
        }
    }
}
