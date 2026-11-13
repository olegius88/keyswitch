//! What threads other than the worker touch: atomics the keyboard hook reads, the inbox, and the guarded snapshot.

use super::logging::technical_event;
use super::types::{Stroke, EngineSnapshot, CorrectionPlan, LearningPrompt, Hotkey, is_modifier_key, PROMPT_KEYS, Update};
use super::layout_tap::{configured_convert_last_key, configured_layout_switch_key};
use crate::constants::{ENGINE_EVENT_QUEUE_MAX_SIZE, LEARNING_PROMPT_TIMEOUT_SECONDS, POINTER_KEY_NAME, WHEEL_KEY_NAME};
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
    /// Keys that wait for the letters a correction types again (`put_behind`).
    parked: Option<Parked>,
}

/// What waits while the letters a correction types again come back through the hook. The worker sees none of it until
/// the last letter has come: it would plan on half of the word, and it would take an Enter that stands behind the letters
/// in the window for one typed before them.
struct Parked {
    /// How many of the letters are still to come back.
    left: usize,
    /// The letters that came back, and whatever else the hook reported meanwhile, in the order it came.
    returned: Vec<QueueItem>,
    /// The keys that stand behind the letters in the window: an Enter or Tab the hook keeps back, and what was queued
    /// after it. The key-up of a key that waits here waits here too, wherever it arrived.
    behind: Vec<QueueItem>,
}

impl Parked {
    /// Whether `event` lets go of a key whose press waits behind the letters: an Enter let go while its correction is
    /// typed is reported at once, and queued then, it came before its own press.
    fn lets_go(&self, event: &KeyEvent) -> bool {
        !event.pressed && self.behind.iter().any(|item| matches!(item, QueueItem::Key(key) if key.pressed && key.keycode == event.keycode))
    }
}

impl Inbox {
    /// A key the hook reports while keys are parked: kept with them, and when it is the last of the letters, all of them
    /// are queued. False when nothing is parked.
    fn park(&mut self, event: &Arc<KeyEvent>, letter: bool) -> bool {
        let Some(parked) = self.parked.as_mut() else {
            return false;
        };
        if parked.lets_go(event) {
            parked.behind.push(QueueItem::Key(event.clone()));
            return true;
        }
        parked.returned.push(QueueItem::Key(event.clone()));
        if letter {
            parked.left = parked.left.saturating_sub(1);
            if parked.left == 0 {
                self.release_parked();
            }
        }
        true
    }

    /// Everything parked is queued: the letters that came back, then the keys behind them. Gives how many letters had
    /// not come back; None when nothing was parked.
    fn release_parked(&mut self) -> Option<usize> {
        let parked = self.parked.take()?;
        self.events.extend(parked.returned);
        self.events.extend(parked.behind);
        Some(parked.left)
    }
}

pub(super) type SnapshotCallback = Arc<dyn Fn(&EngineSnapshot) + Send + Sync>;
pub(super) type CorrectionCallback = Arc<dyn Fn(&CorrectionPlan) + Send + Sync>;
pub(super) type SelectionNoticeCallback = Arc<dyn Fn(&str) + Send + Sync>;
pub(super) type LearningPromptCallback = Arc<dyn Fn(Option<&LearningPrompt>) + Send + Sync>;
pub(super) type RuleRequestCallback = Arc<dyn Fn(&LearningPrompt) + Send + Sync>;

/// What the Python's `_lock` guards. It is never held while calling a callback, the backend or the settings.
struct Guarded {
    snapshot: EngineSnapshot,
    callbacks: Vec<SnapshotCallback>,
    correction_callbacks: Vec<CorrectionCallback>,
    selection_notice_callbacks: Vec<SelectionNoticeCallback>,
    learning_prompt: Option<LearningPrompt>,
    learning_prompt_deadline: Option<f64>,
    learning_prompt_callbacks: Vec<LearningPromptCallback>,
    rule_request_callbacks: Vec<RuleRequestCallback>,
}

pub struct Shared {
    pub settings: Arc<SettingsStore>,
    pub backend: Arc<dyn InputBackend>,
    pub(crate) clock: Clock,
    pub(crate) models_len: usize,
    swallows_hotkeys: bool,
    // Read from the keyboard hook without a lock: see `consumes_key`.
    prompt_key_deadline: AtomicU64,
    input_suspended: AtomicBool,
    test_field_focused: AtomicBool,
    // Input other programs inject (remote control, macros) counts as typing
    // only with detection.injected_input; read by the hook, so cached here.
    injected_input: AtomicBool,
    // Whether a layout switch key or a conversion key is set: only then is a turned wheel queued (`enqueue_wheel`).
    layout_tap_enabled: AtomicBool,
    sensitive_context_window: AtomicI64,
    focus_window: AtomicI64,
    /// The program in front is one of the user's exclusions, as the engine saw it last (`set_front_excluded`).
    front_excluded: AtomicBool,
    /// A key other than Enter or Tab has been pressed since the hook last kept one of them back (`consumes_key`).
    typed_since_action: AtomicBool,
    action_keys: AtomicU8,
    /// Swapped whole when the settings change; the hook only tries it, so it never waits.
    own_hotkeys: RwLock<Arc<Vec<Hotkey>>>,
    pub typed_events: AtomicI64,
    pub typed_presses: AtomicI64,
    pub input_overflow: AtomicBool,
    pub running: AtomicBool,
    inbox: Mutex<Inbox>,
    wake: Condvar,
    guarded: Mutex<Guarded>,
    pub(crate) log_redactor: Mutex<ks_config::log_privacy::LogRedactor>,
    /// The settings' unsubscribe call, made when the engine goes.
    unsubscribe: Mutex<Option<ks_config::config::Unsubscribe>>,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    pub fn new(
        settings: Arc<SettingsStore>, backend: Arc<dyn InputBackend>, clock: Clock, models_len: usize, snapshot: EngineSnapshot,
    ) -> Arc<Shared> {
        let swallows_hotkeys = backend.swallows_hotkeys();
        let shared = Arc::new(Shared {
            settings,
            backend,
            clock,
            models_len,
            swallows_hotkeys,
            prompt_key_deadline: AtomicU64::new(0f64.to_bits()),
            input_suspended: AtomicBool::new(false),
            test_field_focused: AtomicBool::new(false),
            injected_input: AtomicBool::new(false),
            layout_tap_enabled: AtomicBool::new(false),
            sensitive_context_window: AtomicI64::new(NO_WINDOW),
            focus_window: AtomicI64::new(NO_WINDOW),
            front_excluded: AtomicBool::new(false),
            // Nothing is known of what was typed before the program started.
            typed_since_action: AtomicBool::new(true),
            action_keys: AtomicU8::new(0),
            own_hotkeys: RwLock::new(Arc::new(Vec::new())),
            typed_events: AtomicI64::new(0),
            typed_presses: AtomicI64::new(0),
            input_overflow: AtomicBool::new(false),
            running: AtomicBool::new(false),
            inbox: Mutex::new(Inbox::default()),
            wake: Condvar::new(),
            guarded: Mutex::new(Guarded {
                snapshot,
                callbacks: Vec::new(),
                correction_callbacks: Vec::new(),
                selection_notice_callbacks: Vec::new(),
                learning_prompt: None,
                learning_prompt_deadline: None,
                learning_prompt_callbacks: Vec::new(),
                rule_request_callbacks: Vec::new(),
            }),
            log_redactor: Mutex::new(ks_config::log_privacy::LogRedactor::new()),
            unsubscribe: Mutex::new(None),
        });
        shared.refresh_hook_settings();
        shared
    }

    /// `self.settings.subscribe(self.settings_changed)`. What the hook reads changes at once, on the caller's thread; the
    /// rest of the change is the worker's.
    pub fn subscribe_settings(shared: &Arc<Shared>) {
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

    /// `action_keys`, `own_hotkeys`, `injected_input` and `layout_tap_enabled`, read from the settings for the hook.
    pub fn refresh_hook_settings(&self) {
        self.action_keys.store(self.configured_action_keys(), Ordering::Relaxed);
        let hotkeys = Arc::new(self.configured_hotkeys());
        *self.own_hotkeys.write().unwrap_or_else(PoisonError::into_inner) = hotkeys;
        self.injected_input.store(self.flag("detection.injected_input", false), Ordering::Relaxed);
        let tap_key_set = configured_layout_switch_key(&self.settings).is_some() || configured_convert_last_key(&self.settings).is_some();
        self.layout_tap_enabled.store(tap_key_set, Ordering::Relaxed);
    }

    fn configured_action_keys(&self) -> u8 {
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
    fn configured_hotkeys(&self) -> Vec<Hotkey> {
        self.settings.with_value("hotkeys", |configured| match configured {
            Some(Value::Object(map)) => map
                .values()
                .map(|value| match value {
                    Value::String(text) => Hotkey::new(text),
                    other @ (Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) |
Value::Object(_)) => Hotkey::new(&other.to_string()),
                })
                .collect(),
            _ => Vec::new(),
        })
    }

    pub fn prompt_key_deadline(&self) -> f64 {
        f64::from_bits(self.prompt_key_deadline.load(Ordering::Relaxed))
    }

    pub fn set_prompt_key_deadline(&self, deadline: f64) {
        self.prompt_key_deadline.store(deadline.to_bits(), Ordering::Relaxed);
    }

    pub fn sensitive_context_window(&self) -> Option<i64> {
        let window = self.sensitive_context_window.load(Ordering::Relaxed);
        (window != NO_WINDOW).then_some(window)
    }

    pub fn set_sensitive_context_window(&self, window: Option<i64>) {
        self.sensitive_context_window.store(window.unwrap_or(NO_WINDOW), Ordering::Relaxed);
    }

    pub fn focus_window(&self) -> Option<i64> {
        let window = self.focus_window.load(Ordering::Relaxed);
        (window != NO_WINDOW).then_some(window)
    }

    pub fn set_focus_window(&self, window: i64) {
        self.focus_window.store(window, Ordering::Relaxed);
    }

    /// The engine tells the hook whether the program in front is in the user's exclusions, as of the last key it took
    /// (`EngineCore::front_application`).
    pub fn set_front_excluded(&self, excluded: bool) {
        self.front_excluded.store(excluded, Ordering::Relaxed);
    }

    pub fn front_excluded(&self) -> bool {
        self.front_excluded.load(Ordering::Relaxed)
    }

    /// A key that may have left something to correct was pressed: the next Enter or Tab is kept back for it. The hook
    /// says so for every key it is asked about, and the engine for every key it takes, which covers a key that reached
    /// the engine by another way than the hook's question.
    pub fn note_typed(&self) {
        self.typed_since_action.store(true, Ordering::Relaxed);
    }

    pub fn input_suspended(&self) -> bool {
        self.input_suspended.load(Ordering::Relaxed)
    }

    pub fn test_field_focused(&self) -> bool {
        self.test_field_focused.load(Ordering::Relaxed)
    }

    pub fn accepts_injected(&self, event: &KeyEvent) -> bool {
        // Whether input another program injected is treated as the user's typing.
        //
        // A key that carries a character instead of a key position (VK_PACKET on
        // Windows) never is: it cannot be typed again in another layout.
        self.injected_input.load(Ordering::Relaxed) && event.key_name != ks_backend::constants::UNICODE_PACKET_KEY_NAME
    }

    /// `consumes_key`.
    /// Whether `KeySwitch` answers this key itself and the window must not.
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
        let action = match event.key_name.as_str() {
            "Return" => ACTION_RETURN,
            "KP_Enter" => ACTION_KP_ENTER,
            "Tab" => ACTION_TAB,
            _ => 0,
        };
        if action == 0 {
            self.note_typed();
        }
        if self.input_suspended() || (event.foreign && !self.accepts_injected(event)) {
            return KeyDisposition::Pass;
        }
        if self.sensitive_context_window.load(Ordering::Relaxed) != NO_WINDOW {
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
                && self.own_hotkeys.try_read().is_ok_and(|hotkeys| hotkeys.iter().any(|hotkey| hotkey.matches(event)))
            {
                KeyDisposition::Consume
            } else {
                KeyDisposition::Pass
            };
        }
        if (self.clock)() < self.prompt_key_deadline() && PROMPT_KEYS.contains(&event.key_name.as_str()) {
            return KeyDisposition::Consume;
        }
        self.set_prompt_key_deadline(0.0);
        if action & self.action_keys.load(Ordering::Relaxed) == 0 {
            return KeyDisposition::Pass;
        }
        // A hotkey of the program's own set to Tab or Enter alone is that hotkey, not an action to keep back for a
        // correction: the engine answers it as the hotkey and never as the action.
        if self.own_hotkeys.try_read().is_ok_and(|hotkeys| hotkeys.iter().any(|hotkey| hotkey.matches(event))) {
            return KeyDisposition::Pass;
        }
        // A program the user excluded is left alone, its Enter and Tab too: kept back, they reached it only once every
        // key had come up and as typed by a program, in the very programs people exclude for that (a remote desktop, a
        // virtual machine, a game where Tab is held). The first key after such a program comes to the front may still
        // be kept back: the engine learns of the program in front from the keys it takes.
        if self.front_excluded.load(Ordering::Relaxed) {
            return KeyDisposition::Pass;
        }
        // Nothing was typed since the last Enter or Tab kept back, so there is nothing to correct before this one: it
        // goes to the window as it is pressed, and repeats while it is held. Kept back, every Enter and Tab waited for
        // its key to come up, a held one came once, and one held for two seconds did not come at all (a review of the
        // Windows hook, 11.11.2026).
        if !self.typed_since_action.swap(false, Ordering::Relaxed) {
            return KeyDisposition::Pass;
        }
        KeyDisposition::Defer
    }

    /// `enqueue`: on the hook's thread, behind the inbox's short lock, which nothing holds while it blocks.
    pub fn enqueue(&self, event: KeyEvent) {
        if event.synthetic {
            return;
        }
        if event.key_name == WHEEL_KEY_NAME {
            self.enqueue_wheel(event);
            return;
        }
        if event.key_name == "Pointer" {
            self.set_prompt_key_deadline(0.0);
        }
        // Physical events and presses are separate: a key-up is not text.
        self.typed_events.fetch_add(1, Ordering::Relaxed);
        self.typed_presses.fetch_add(i64::from(event.pressed), Ordering::Relaxed);
        // A press that types: one of the letters a correction types again, when keys wait for those (`put_behind`).
        let letter = event.pressed && !is_modifier_key(&event.key_name) && event.key_name != POINTER_KEY_NAME;
        let event = Arc::new(event);
        let mut inbox = locked(&self.inbox);
        if inbox.park(&event, letter) {
            drop(inbox);
            self.wake.notify_one();
        } else if inbox.events.len() < ENGINE_EVENT_QUEUE_MAX_SIZE {
            inbox.events.push_back(QueueItem::Key(event));
            drop(inbox);
            self.wake.notify_one();
        } else {
            drop(inbox);
            // The hook must not mutate the worker's state or invoke UI/file
            // callbacks. The worker invalidates the text before its next action.
            self.input_overflow.store(true, Ordering::Relaxed);
        }
    }

    /// A turned wheel only ends a lone press of the layout switch key or the conversion key (`follow_layout_tap`). It is
    /// queued only while such a key is set, once for a run of turns, never in place of a key, and it is no typing
    /// (`typed_events` counts what the hook saw during a correction, a wheel included would read as a key that went past
    /// it).
    fn enqueue_wheel(&self, event: KeyEvent) {
        if !self.layout_tap_enabled.load(Ordering::Relaxed) {
            return;
        }
        let mut inbox = locked(&self.inbox);
        let after_wheel = inbox.events.back().is_some_and(|item| matches!(item, QueueItem::Key(key) if key.key_name == WHEEL_KEY_NAME));
        if after_wheel || inbox.events.len() >= ENGINE_EVENT_QUEUE_MAX_SIZE {
            return;
        }
        inbox.events.push_back(QueueItem::Key(Arc::new(event)));
        drop(inbox);
        self.wake.notify_one();
    }

    pub fn key_filter(self: &Arc<Self>) -> KeyFilter {
        let shared = self.clone();
        Arc::new(move |event: &KeyEvent| shared.consumes_key(event))
    }

    pub fn key_listener(self: &Arc<Self>) -> KeyListener {
        let shared = self.clone();
        Arc::new(move |event: KeyEvent| shared.enqueue(event))
    }

    pub fn post(&self, command: Command) {
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
    pub fn take_command(&self) -> Option<Command> {
        locked(&self.inbox).commands.pop_front()
    }

    /// `tuple(self._events.queue)`, under the inbox's lock.
    pub fn queued(&self) -> Vec<QueueItem> {
        locked(&self.inbox).events.iter().cloned().collect()
    }

    /// Everything queued, taken out; `put_back` returns what stays, in front of whatever arrived since.
    pub fn take_queued(&self) -> Vec<QueueItem> {
        locked(&self.inbox).events.drain(..).collect()
    }

    pub fn put_back(&self, items: Vec<QueueItem>) {
        let mut inbox = locked(&self.inbox);
        for item in items.into_iter().rev() {
            inbox.events.push_front(item);
        }
        drop(inbox);
        self.wake.notify_one();
    }

    /// Keys that stand in the window behind letters a correction is about to type again: an Enter or Tab the hook keeps
    /// back, and what was queued after it. The letters come back through the hook as keys just typed, at the end of the
    /// queue; these keys are queued again right behind the last of them, as they stand in the window, and until then
    /// the worker sees neither.
    pub fn put_behind(&self, letters: usize, behind: Vec<QueueItem>) {
        let mut inbox = locked(&self.inbox);
        // Never two at once: what still waited for the letters of an earlier correction goes on first.
        inbox.release_parked();
        let mut parked = Parked { left: letters, returned: Vec::new(), behind };
        // A key of `behind` let go since the queue was taken is in the queue already: it follows its press.
        let (let_go, queued): (Vec<QueueItem>, Vec<QueueItem>) = inbox.events.drain(..).partition(|item| matches!(item, QueueItem::Key(key) if parked.lets_go(key)));
        inbox.events.extend(queued);
        parked.behind.extend(let_go);
        inbox.parked = Some(parked);
    }

    /// Whether keys wait for letters a correction types again.
    pub fn waits_behind(&self) -> bool {
        locked(&self.inbox).parked.is_some()
    }

    /// The keys of `put_behind` are queued now, whatever has come back. Gives how many of the letters had not; None when
    /// nothing waited.
    pub fn release_behind(&self) -> Option<usize> {
        let owed = locked(&self.inbox).release_parked();
        self.wake.notify_one();
        owed
    }

    pub fn qsize(&self) -> usize {
        locked(&self.inbox).events.len()
    }

    /// A key queued for the replay to take: the harness's own `_events.get_nowait`.
    pub fn get_nowait(&self) -> Option<QueueItem> {
        locked(&self.inbox).events.pop_front()
    }

    pub fn application_excluded(&self, application: &str) -> bool {
        if let Some(window) = self.sensitive_context_window()
            && Some(window) == self.focus_window()
        {
            return true;
        }
        self.application_listed(application)
    }

    /// Whether `application` is in the user's list of excluded programs.
    pub fn application_listed(&self, application: &str) -> bool {
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
        locked(&self.guarded).snapshot.clone()
    }

    pub fn current_group(&self) -> i64 {
        locked(&self.guarded).snapshot.current_group
    }

    pub fn correction_count(&self) -> i64 {
        locked(&self.guarded).snapshot.correction_count
    }

    pub fn subscribe(&self, callback: SnapshotCallback) {
        locked(&self.guarded).callbacks.push(callback.clone());
        callback(&self.snapshot());
    }

    pub fn subscribe_corrections(&self, callback: CorrectionCallback) {
        locked(&self.guarded).correction_callbacks.push(callback);
    }

    pub fn subscribe_selection_notices(&self, callback: SelectionNoticeCallback) {
        locked(&self.guarded).selection_notice_callbacks.push(callback);
    }

    pub fn selection_notice_callbacks(&self) -> Vec<SelectionNoticeCallback> {
        locked(&self.guarded).selection_notice_callbacks.clone()
    }

    pub fn correction_callbacks(&self) -> Vec<CorrectionCallback> {
        locked(&self.guarded).correction_callbacks.clone()
    }

    /// `self.learning_prompt`.
    pub fn learning_prompt(&self) -> Option<LearningPrompt> {
        locked(&self.guarded).learning_prompt.clone()
    }

    pub fn subscribe_learning_prompts(&self, callback: LearningPromptCallback) {
        let prompt = {
            let mut guarded = locked(&self.guarded);
            guarded.learning_prompt_callbacks.push(callback.clone());
            guarded.learning_prompt.clone()
        };
        callback(prompt.as_ref());
    }

    /// Be told when Enter on the prompt asks for the rule window.
    pub fn subscribe_rule_requests(&self, callback: RuleRequestCallback) {
        locked(&self.guarded).rule_request_callbacks.push(callback);
    }

    /// Enter on the prompt: open the rule window, filled in from the prompt.
    ///
    /// Nothing is remembered yet. A rule exists only once the user presses OK in
    /// that window, which calls :meth:`add_learning_rule`.
    pub fn confirm_learning_prompt(&self, prompt: Option<&LearningPrompt>) -> bool {
        let (current, callbacks, requests) = {
            let mut guarded = locked(&self.guarded);
            let Some(current) = guarded.learning_prompt.clone() else {
                return false;
            };
            if prompt.is_some_and(|prompt| *prompt != current) {
                return false;
            }
            guarded.learning_prompt = None;
            guarded.learning_prompt_deadline = None;
            self.set_prompt_key_deadline(0.0);
            (current, guarded.learning_prompt_callbacks.clone(), guarded.rule_request_callbacks.clone())
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
            let Some(current) = guarded.learning_prompt.clone() else {
                return false;
            };
            if prompt.is_some_and(|prompt| *prompt != current) {
                return false;
            }
            guarded.learning_prompt = None;
            guarded.learning_prompt_deadline = None;
            self.set_prompt_key_deadline(0.0);
            (current, guarded.learning_prompt_callbacks.clone())
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

    pub fn show_learning_prompt(&self, prompt: LearningPrompt) {
        let now = (self.clock)();
        let callbacks = {
            let mut guarded = locked(&self.guarded);
            guarded.learning_prompt = Some(prompt.clone());
            let deadline = now + LEARNING_PROMPT_TIMEOUT_SECONDS;
            guarded.learning_prompt_deadline = Some(deadline);
            self.set_prompt_key_deadline(deadline);
            guarded.learning_prompt_callbacks.clone()
        };
        let excluded = self.application_excluded(&prompt.application);
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

    pub fn expire_learning_prompt(&self, now: Option<f64>) -> bool {
        let Some(deadline) = locked(&self.guarded).learning_prompt_deadline else {
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
        if opened == self.input_suspended() {
            return;
        }
        self.input_suspended.store(opened, Ordering::Relaxed);
        technical_event!(self, if opened { "rule_editor_focused" } else { "rule_editor_left" });
    }

    /// The test field of the settings window gained or lost the keyboard.
    ///
    /// The windows of `KeySwitch` itself are not where the user types (FocusInfo.own): moving to
    /// the settings window or the learning prompt is no move to another program, and Windows
    /// keeps a layout per window, so a layout seen there says nothing of the text and is ignored.
    /// The test field is the exception: it is there to try `KeySwitch` as in any other program,
    /// and a layout picked by hand while it has the keyboard must protect the next word there
    /// too. Only the window knows which of its controls has the keyboard, so it reports it here.
    pub fn set_test_field_focused(&self, focused: bool) {
        if focused == self.test_field_focused() {
            return;
        }
        self.test_field_focused.store(focused, Ordering::Relaxed);
        technical_event!(self, if focused { "test_field_focused" } else { "test_field_left" });
    }

    pub fn update(&self, update: Update) {
        let (callbacks, snapshot) = {
            let mut guarded = locked(&self.guarded);
            let current = &mut guarded.snapshot;
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
            if guarded.callbacks.is_empty() {
                return;
            }
            (guarded.callbacks.clone(), guarded.snapshot.clone())
        };
        for callback in callbacks {
            callback(&snapshot);
        }
    }

    pub fn technical_logging(&self) -> bool {
        self.settings.with_value("diagnostics.technical_logging", |value| value.is_some_and(truthy))
    }

    /// The technical record, after its words that look like a password went into stars (`log_privacy`).
    pub fn emit_technical(&self, payload: Map<String, Value>) {
        let record = locked(&self.log_redactor).record(&payload);
        let options = ks_text::JsonOptions {
            ensure_ascii: false,
            indent: None,
            separators: Some((",".to_owned(), ":".to_owned())),
            sort_keys: true,
        };
        let message = format!("TECHNICAL {}", ks_text::json_dumps(&Value::Object(record), &options));
        ks_config::logsetup::info("keyswitch.engine", &message);
    }

    pub fn focused_window(&self) -> Option<FocusInfo> {
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
