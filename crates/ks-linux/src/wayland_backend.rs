//! Wayland keyboard backend with exclusive evdev capture, uinput injection and compositor IPC (the Python's
//! `wayland_backend.py`).
//!
//! Everything the backend needs from Linux, XKB and the compositor is behind `WaylandApi`, so the logic below runs against a
//! fake in the tests. Threads: the helper's dispatch thread calls the handler for every physical key and waits for the
//! verdict it returns; the engine's worker injects corrections and holds `inject_lock` across a batch. The state shared by
//! the two sits behind one small lock that is never held across a call to the API, the listener or the key filter.

use ks_cast::To as _;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ks_backend::constants::{COMPLETED_ACTION_EVENT_COUNT, LAYOUT_GROUP_COUNT, LOCK_MASK, POINTER_EVENT_GROUP, SHIFT_MASK};
use ks_backend::constants::{ALT_MASK, CONTROL_MASK, SUPER_MASK};
use ks_backend::input_delay::{InputDelay, InputDelayReport};
use ks_backend::selected_text::{RefusalReason, SelectedText, SelectionError, SelectionRefused};
use ks_backend::{BackendError, BackendProbe, FocusInfo, InputBackend, Injection, KeyDisposition, KeyEvent, KeyFilter, KeyListener, ScreenAnchor};
use ks_config::logsetup;
use ks_config::selection_commands::{KeyChoice, caret_steps, typing_runs};
use ks_text::pystr;
use ks_text::russian_text::{SECONDS, quantity};

use crate::constants::{KEYBOARD_LISTENER_START_TIMEOUT_SECONDS, KEYBOARD_LISTENER_STOP_TIMEOUT_SECONDS};
use crate::wayland_constants::*;
use crate::wayland_helper::{Flag, seconds};

const LOGGER: &str = "keyswitch.wayland_backend";

pub const SESSION_TYPE: &str = "wayland";
pub const CAPTURE_METHOD: &str = "evdev (EVIOCGRAB)";
pub const INJECT_METHOD: &str = "uinput";
pub const POINTER_KEY_NAME: &str = "Pointer";
pub const INPUT_AHEAD_ERROR: &str = "Клавиша дошла до приложения раньше замены; текст не изменён";

/// The name of a key the engine knows by its X11 name.
fn evdev_key_name(keycode: u32) -> Option<&'static str> {
    Some(match keycode {
        EVDEV_KEY_BACKSPACE => "BackSpace",
        EVDEV_KEY_TAB => "Tab",
        EVDEV_KEY_ENTER => "Return",
        EVDEV_KEY_KPENTER => "KP_Enter",
        EVDEV_KEY_ESC => "Escape",
        EVDEV_KEY_SPACE => "space",
        EVDEV_KEY_PAUSE => "Pause",
        EVDEV_KEY_CAPSLOCK => "Caps_Lock",
        EVDEV_KEY_LEFTSHIFT => "Shift_L",
        EVDEV_KEY_RIGHTSHIFT => "Shift_R",
        EVDEV_KEY_LEFTCTRL => "Control_L",
        EVDEV_KEY_RIGHTCTRL => "Control_R",
        EVDEV_KEY_LEFTALT => "Alt_L",
        EVDEV_KEY_RIGHTALT => "Alt_R",
        EVDEV_KEY_LEFTMETA => "Super_L",
        EVDEV_KEY_RIGHTMETA => "Super_R",
        EVDEV_KEY_HOME => "Home",
        EVDEV_KEY_END => "End",
        EVDEV_KEY_PAGEUP => "Page_Up",
        EVDEV_KEY_PAGEDOWN => "Page_Down",
        EVDEV_KEY_LEFT => "Left",
        EVDEV_KEY_RIGHT => "Right",
        EVDEV_KEY_UP => "Up",
        EVDEV_KEY_DOWN => "Down",
        EVDEV_KEY_INSERT => "Insert",
        EVDEV_KEY_DELETE => "Delete",
        EVDEV_KEY_SEMICOLON => "semicolon",
        EVDEV_KEY_EQUAL => "equal",
        EVDEV_KEY_COMMA => "comma",
        EVDEV_KEY_MINUS => "minus",
        EVDEV_KEY_DOT => "period",
        EVDEV_KEY_SLASH => "slash",
        EVDEV_KEY_GRAVE => "grave",
        EVDEV_KEY_LEFTBRACE => "bracketleft",
        EVDEV_KEY_BACKSLASH => "backslash",
        EVDEV_KEY_RIGHTBRACE => "bracketright",
        EVDEV_KEY_APOSTROPHE => "apostrophe",
        _ => return None,
    })
}

fn failed(message: impl Into<String>) -> BackendError {
    BackendError::Failed(message.into())
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One keyboard or pointer event received from the Wayland input helper.
///
/// `seq` numbers the helper's physical keys. `delivered` is a key the helper typed without asking (its verdict was late);
/// `forced` repeats an asked key the helper typed when its time ran out; `input_reset` says the grab changed and the keys held
/// down are unknown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct NativeKeyEvent {
    pub pressed: bool,
    pub keycode: u32,
    pub timestamp: u32,
    pub injected: bool,
    pub replayed: bool,
    pub pointer: bool,
    pub repeat_reset: bool,
    pub seq: u32,
    pub delivered: bool,
    pub forced: bool,
    pub input_reset: bool,
}

impl NativeKeyEvent {
    pub fn new(pressed: bool, keycode: u32, timestamp: u32) -> NativeKeyEvent {
        NativeKeyEvent { pressed, keycode, timestamp, ..NativeKeyEvent::default() }
    }
}

/// One synthetic or replayed key transition destined for `/dev/uinput`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeInput {
    pub pressed: bool,
    pub keycode: u32,
    pub synthetic: bool,
    pub replayed: bool,
}

impl NativeInput {
    pub fn synthetic(pressed: bool, keycode: u32) -> NativeInput {
        NativeInput { pressed, keycode, synthetic: true, replayed: false }
    }
}

/// Answers every key the helper reports: true to swallow it.
pub type NativeListener = Arc<dyn Fn(NativeKeyEvent) -> bool + Send + Sync>;
/// Tells the backend that capture works.
pub type ReadyCallback = Arc<dyn Fn() + Send + Sync>;

/// Everything `WaylandBackend` needs from Linux, XKB and the Wayland compositor.
pub trait WaylandApi: Send + Sync {
    fn desktop_name(&self) -> String;
    fn permission_error(&self) -> String;
    fn layout_names(&self) -> Vec<String>;
    /// The compositor's current layout; -1 while it does not say.
    fn current_layout_index(&self) -> i64;
    fn select_layout_index(&self, index: i64) -> bool;
    fn translate_key(&self, keycode: u32, state: u32, layout_index: i64) -> String;
    fn keysym_name(&self, keycode: u32, layout_index: i64) -> String;
    /// Types the events through the helper and answers how many it typed (0 for a correction it refused).
    fn post_inputs(&self, inputs: &[(bool, u32)], known_seq: Option<u32>, action: bool) -> Result<u32, String>;
    fn active_application(&self) -> String;
    fn focused_window(&self) -> i64;
    fn window_process_id(&self, window: i64) -> i64;
    fn current_process_id(&self) -> i64;
    fn window_origin(&self, window: i64) -> Option<(i64, i64)>;
    fn window_is_password(&self, window: i64) -> bool;
    fn input_anchor(&self) -> Option<ScreenAnchor>;
    fn position_window(&self, window: i64, x: i64, y: i64) -> bool;
    fn activate_window(&self, window: i64) -> bool;
    fn isolated_layout(&self) -> bool;
    fn caps_lock_enabled(&self) -> bool;
    fn key_down(&self, keycode: u32) -> bool;
    fn monotonic_ms(&self) -> u32;
    /// Runs the helper until it is stopped; `listener` answers every key (true: swallow), `ready` says capture works.
    fn run_input_listener(&self, listener: NativeListener, ready: ReadyCallback) -> Result<(), String>;
    fn stop_input_listener(&self);
    fn close(&self);
}

pub fn is_cyrillic(text: &str) -> bool {
    text.chars().next().is_some_and(|first| (0x0400..=0x04FF).contains(&first.to::<u32>()))
}

/// Identify the English and Russian compositor layout indices by probing `KEY_Q`.
pub fn select_layout_indices(layout_count: usize, translate: &dyn Fn(u32, u32, i64) -> String) -> Result<(i64, i64), BackendError> {
    let mut english: Option<i64> = None;
    let mut russian: Option<i64> = None;
    for index in 0..layout_count.to::<i64>() {
        let sample = translate(EVDEV_KEY_Q, 0, index);
        if english.is_none() && pystr::isascii(&sample) && pystr::isalpha(&sample) {
            english = Some(index);
        } else if russian.is_none() && is_cyrillic(&sample) {
            russian = Some(index);
        }
    }
    match (english, russian) {
        (Some(english), Some(russian)) => Ok((english, russian)),
        _ => Err(failed("В среде Wayland должны быть включены английская и русская раскладки")),
    }
}

/// Translate an evdev keycode and XKB keysym into the engine's X11 key vocabulary.
pub fn key_name(keycode: u32, symbol_name: &str, plain_char: &str) -> String {
    if let Some(name) = evdev_key_name(keycode) {
        return name.to_owned();
    }
    let single_alnum = |text: &str| text.chars().count() == 1 && pystr::isascii(text) && pystr::isalnum(text);
    if !symbol_name.is_empty() {
        return if single_alnum(symbol_name) { pystr::casefold(symbol_name) } else { symbol_name.to_owned() };
    }
    if single_alnum(plain_char) {
        return pystr::casefold(plain_char);
    }
    format!("KEY_{keycode}")
}

/// The last asked key and what was made of it, for a verdict the helper got too late.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Answer {
    Held,
    Consumed,
    Deferred,
    Passed,
}

#[derive(Default)]
struct State {
    pressed: HashSet<u32>,
    consumed_keys: HashSet<u32>,
    holding: bool,
    held: Vec<NativeKeyEvent>,
    pointer_epoch: u64,
    hold_pointer_epoch: u64,
    hold_window: i64,
    deferred_action: Option<NativeKeyEvent>,
    action_prior_keys: HashSet<u32>,
    /// Keys down when a hold began: their releases pass, or the client repeats them meanwhile.
    hold_prior_keys: HashSet<u32>,
    last_answer: Option<(u32, Answer, NativeKeyEvent)>,
    /// The last helper key handled, and the one handled when the current hold began.
    known_seq: u32,
    hold_known_seq: u32,
    /// A key reached the application during this hold: a correction would edit unknown text.
    hold_broken: bool,
    /// The withheld Enter/Tab reached the application: `complete_action` must not type it again.
    action_forced: bool,
    last_typed: (u32, i64),
}

struct Inner {
    api: Arc<dyn WaylandApi>,
    group_count: usize,
    selection_factory: Option<crate::x11_backend::SelectionFactory>,
    layouts: Mutex<Option<(i64, i64)>>,
    listener: Mutex<Option<KeyListener>>,
    key_filter: Mutex<Option<KeyFilter>>,
    thread: Mutex<Option<JoinHandle<()>>>,
    running: AtomicBool,
    ready: Flag,
    start_error: Mutex<Option<String>>,
    state: Mutex<State>,
    inject_lock: Mutex<()>,
    input_delay: InputDelay,
    /// The reader of the selection hotkeys, made on first use (`read_selection`).
    selection_source: Mutex<Option<Box<dyn crate::x11_backend::SelectionSource>>>,
}

/// Global Wayland keyboard observer, key filter, input holder and injector.
pub struct WaylandBackend {
    inner: Arc<Inner>,
}

fn sleep(value: f64) {
    thread::sleep(seconds(value));
}

impl WaylandBackend {
    pub fn new(api: Arc<dyn WaylandApi>, selection_factory: Option<crate::x11_backend::SelectionFactory>) -> WaylandBackend {
        WaylandBackend::with_group_count(api, LAYOUT_GROUP_COUNT.to::<usize>(), selection_factory)
    }

    pub fn with_group_count(api: Arc<dyn WaylandApi>, group_count: usize, selection_factory: Option<crate::x11_backend::SelectionFactory>) -> WaylandBackend {
        WaylandBackend {
            inner: Arc::new(Inner {
                api,
                group_count,
                selection_factory,
                layouts: Mutex::new(None),
                listener: Mutex::new(None),
                key_filter: Mutex::new(None),
                thread: Mutex::new(None),
                running: AtomicBool::new(false),
                ready: Flag::default(),
                start_error: Mutex::new(None),
                state: Mutex::new(State::default()),
                inject_lock: Mutex::new(()),
                input_delay: InputDelay::new(),
                selection_source: Mutex::new(None),
            }),
        }
    }

    pub fn running(&self) -> bool {
        self.inner.running.load(Ordering::SeqCst)
    }

    pub fn layouts(&self) -> Result<(i64, i64), BackendError> {
        self.inner.layouts()
    }

    /// The same handler the helper's keys go through, for a test or a caller that feeds keys itself.
    pub fn handle_native(&self, native: NativeKeyEvent) -> bool {
        self.inner.handle_native(native)
    }

    pub fn window_process_id(&self, window: i64) -> i64 {
        self.inner.api.window_process_id(window)
    }

    /// The same, as a function a reader of accessibility can keep.
    pub fn process_for_window(&self) -> crate::x11_backend::ProcessForWindow {
        let inner = self.inner.clone();
        Arc::new(move |window| u32::try_from(inner.api.window_process_id(window)).unwrap_or(0))
    }

    pub fn window_origin(&self, window: i64) -> Option<(i64, i64)> {
        self.inner.api.window_origin(window)
    }

    /// Whether the compositor says the keyboard of `window` is in a password field.
    pub fn window_is_password(&self, window: i64) -> bool {
        self.inner.api.window_is_password(window)
    }

    pub fn position_window(&self, window: i64, x: i64, y: i64) -> bool {
        self.inner.api.position_window(window, x, y)
    }

    fn stop_inner(&self) {
        let inner = &self.inner;
        *lock(&inner.key_filter) = None;
        // A failure to deliver what was withheld cannot be reported from here; the keys are dropped with the capture.
        let _ = inner.complete_action(false);
        let Some(handle) = lock(&inner.thread).take() else { return };
        inner.api.stop_input_listener();
        if handle.thread().id() != thread::current().id() {
            join_within(handle, KEYBOARD_LISTENER_STOP_TIMEOUT_SECONDS);
        }
        inner.running.store(false, Ordering::SeqCst);
    }
}

/// Waits for a thread to end, up to `timeout` seconds; a thread still running is left behind.
pub fn join_within(handle: JoinHandle<()>, timeout: f64) {
    let deadline = Instant::now() + seconds(timeout);
    while !handle.is_finished() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
    if handle.is_finished() {
        let _ = handle.join();
    }
}

impl Drop for WaylandBackend {
    fn drop(&mut self) {
        self.close();
    }
}

impl Inner {
    fn layouts(&self) -> Result<(i64, i64), BackendError> {
        let mut cached = lock(&self.layouts);
        if let Some(layouts) = *cached {
            return Ok(layouts);
        }
        let names = self.api.layout_names();
        let layouts = select_layout_indices(names.len(), &|keycode, state, index| self.api.translate_key(keycode, state, index))?;
        *cached = Some(layouts);
        Ok(layouts)
    }

    fn layout_list(&self) -> Result<[i64; 2], BackendError> {
        let (english, russian) = self.layouts()?;
        Ok([english, russian])
    }

    fn current_group(&self) -> Result<i32, BackendError> {
        let active = self.api.current_layout_index();
        let indices = self.layout_list()?;
        Ok(indices.iter().position(|index| *index == active).map_or(-1, |position| position.low::<i32>()))
    }

    fn state(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }

    fn listener(&self) -> Option<KeyListener> {
        lock(&self.listener).clone()
    }

    fn mark_ready(&self) {
        self.running.store(true, Ordering::SeqCst);
        self.ready.set();
    }

    fn consumes(&self, event: &KeyEvent) -> KeyDisposition {
        if event.synthetic {
            return KeyDisposition::Pass;
        }
        if !event.pressed {
            return if self.state().consumed_keys.remove(&event.keycode) { KeyDisposition::Consume } else { KeyDisposition::Pass };
        }
        if self.state().consumed_keys.contains(&event.keycode) {
            return KeyDisposition::Consume;
        }
        let predicate = lock(&self.key_filter).clone();
        let decision = predicate.map_or(KeyDisposition::Pass, |predicate| predicate(event));
        if decision == KeyDisposition::Pass {
            return KeyDisposition::Pass;
        }
        self.state().consumed_keys.insert(event.keycode);
        decision
    }

    fn hold_input(&self) {
        let window = self.api.focused_window();
        let mut state = self.state();
        if !state.holding {
            state.hold_window = window;
            state.hold_pointer_epoch = state.pointer_epoch;
            state.hold_known_seq = state.known_seq;
            state.hold_broken = false;
            state.hold_prior_keys = state.pressed.iter().copied().filter(|key| !EVDEV_MODIFIER_KEYS.contains(key)).collect();
        }
        state.holding = true;
    }

    fn release_input(&self) -> Result<i32, BackendError> {
        if self.state().deferred_action.is_some() {
            return Ok(0);
        }
        let mut count = 0;
        let outcome = (|| -> Result<(), BackendError> {
            loop {
                let item = {
                    let mut state = self.state();
                    if state.deferred_action.is_some() {
                        let found = state.held.iter().position(|item| !item.pressed && state.action_prior_keys.contains(&item.keycode));
                        match found {
                            Some(index) => state.held.remove(index),
                            None => return Ok(()),
                        }
                    } else if !state.held.is_empty() {
                        state.held.remove(0)
                    } else {
                        state.holding = false;
                        return Ok(());
                    }
                };
                self.deliver_inputs(&[NativeInput { pressed: item.pressed, keycode: item.keycode, synthetic: false, replayed: true }], None, false)?;
                count += 1;
            }
        })();
        {
            let mut state = self.state();
            if state.deferred_action.is_none() {
                state.holding = false;
                state.hold_prior_keys.clear();
                state.hold_broken = false;
            }
        }
        outcome.map(|()| count)
    }

    fn complete_action(&self, deliver: bool) -> Result<i32, BackendError> {
        let action = self.state().deferred_action;
        let Some(action) = action else {
            let forced = std::mem::take(&mut self.state().action_forced);
            if forced {
                // The helper typed the withheld key itself; what it held back still has to go.
                self.release_input()?;
            }
            return Ok(0);
        };
        let outcome = (|| -> Result<(), BackendError> {
            if deliver {
                let (epoch_changed, hold_window) = {
                    let state = self.state();
                    (state.pointer_epoch != state.hold_pointer_epoch, state.hold_window)
                };
                if epoch_changed || self.api.focused_window() != hold_window {
                    return Err(failed("Enter/Tab не передан: место ввода изменилось"));
                }
                let taps = [NativeInput::synthetic(true, action.keycode), NativeInput::synthetic(false, action.keycode)];
                self.deliver_inputs(&taps, (action.seq != 0).then_some(action.seq), true)?;
            }
            Ok(())
        })();
        {
            let mut state = self.state();
            state.deferred_action = None;
            state.action_prior_keys.clear();
        }
        let released = self.release_input();
        outcome?;
        released?;
        Ok(if deliver { COMPLETED_ACTION_EVENT_COUNT } else { 0 })
    }

    fn handle_native(&self, native: NativeKeyEvent) -> bool {
        if native.input_reset {
            self.reset_input_state();
            return false;
        }
        if native.forced {
            self.settle_forced(&native);
            self.state().known_seq = native.seq;
            return false;
        }
        if native.pointer || native.repeat_reset || native.injected || native.replayed {
            return self.answer_native(&native);
        }
        let started = Instant::now();
        let late = self.api.monotonic_ms().wrapping_sub(native.timestamp);
        let consumed = self.answer_native(&native);
        // Handled, and handed to the engine's queue: a correction decided from here knows it.
        self.state().known_seq = native.seq;
        let callback = ((started.elapsed().as_secs_f64() * 1000.0).round_ties_even()).to::<i64>();
        if late <= UINT32_MODULO_MASK >> 1 {
            self.input_delay.record(i64::from(late), callback);
        }
        consumed
    }

    /// The helper typed an asked key itself, its verdict too late: undo what the verdict assumed.
    ///
    /// The key is in the application. Held, it leaves the hold and reaches the engine as typed; a consumed hotkey lets its
    /// release through; a withheld Enter/Tab is not typed a second time. A correction of this hold can no longer know the
    /// text and is refused.
    fn settle_forced(&self, native: &NativeKeyEvent) {
        let last = {
            let mut state = self.state();
            match state.last_answer {
                Some((seq, _, _)) if seq == native.seq => state.last_answer.take(),
                _ => None,
            }
        };
        let Some((_, kind, original)) = last else { return };
        match kind {
            Answer::Held => {
                {
                    let mut state = self.state();
                    let Some(index) = state.held.iter().position(|item| *item == original) else { return };
                    state.held.remove(index);
                    state.hold_broken = true;
                }
                self.answer_native(&NativeKeyEvent { delivered: true, ..original });
            }
            Answer::Consumed => {
                self.state().consumed_keys.remove(&original.keycode);
                logsetup::info(LOGGER, "Ответ на клавишу опоздал: она дошла до приложения вместе с действием KeySwitch");
            }
            Answer::Deferred => {
                {
                    let mut state = self.state();
                    state.consumed_keys.remove(&original.keycode);
                    if state.deferred_action == Some(original) {
                        state.deferred_action = None;
                        state.action_prior_keys.clear();
                        state.action_forced = true;
                        state.hold_broken = true;
                    }
                }
                logsetup::info(LOGGER, "Ответ на Enter/Tab опоздал: клавиша дошла до приложения до замены");
            }
            Answer::Passed => {}
        }
    }

    /// The grab changed: the keys believed held down and swallowed are unknown now.
    fn reset_input_state(&self) {
        {
            let mut state = self.state();
            state.pressed.clear();
            state.consumed_keys.clear();
            state.hold_prior_keys.clear();
            if state.holding {
                state.hold_broken = true;
            }
        }
        let reset = NativeKeyEvent { repeat_reset: true, ..NativeKeyEvent::new(true, 0, self.api.monotonic_ms()) };
        self.answer_native(&reset);
    }

    fn answer_native(&self, native: &NativeKeyEvent) -> bool {
        if native.pointer || native.repeat_reset {
            if native.pointer {
                self.state().pointer_epoch += 1;
            }
            if let Some(listener) = self.listener() {
                listener(KeyEvent {
                    pressed: true,
                    keycode: 0,
                    key_name: POINTER_KEY_NAME.to_owned(),
                    character: String::new(),
                    characters: vec![String::new(), String::new()],
                    group: POINTER_EVENT_GROUP,
                    state: 0,
                    timestamp: i64::from(native.timestamp),
                    ..KeyEvent::default()
                });
            }
            return false;
        }

        if native.delivered {
            let mut state = self.state();
            if state.holding {
                state.hold_broken = true;
            }
            state.consumed_keys.remove(&native.keycode);
        } else if !native.injected && !native.replayed {
            let mut state = self.state();
            let prior_release = !native.pressed && state.action_prior_keys.contains(&native.keycode);
            let hold_prior = !native.pressed
                && state.hold_prior_keys.contains(&native.keycode)
                && !state.held.iter().any(|item| item.pressed && item.keycode == native.keycode);
            let action_key = state.deferred_action.is_some() && state.consumed_keys.contains(&native.keycode);
            if state.holding && !prior_release && !hold_prior && !action_key {
                state.held.push(*native);
                state.last_answer = Some((native.seq, Answer::Held, *native));
                return true;
            }
            state.hold_prior_keys.remove(&native.keycode);
        }

        if !native.injected && !native.replayed {
            self.recheck_modifiers(native);
        }
        self.track_key_state(native);

        let state_mask = self.normalized_state();
        let Ok(indices) = self.layout_list() else { return false };
        let characters: Vec<String> = indices.iter().map(|index| self.api.translate_key(native.keycode, state_mask, *index)).collect();
        let group = self.current_group().unwrap_or(-1);
        let character = usize::try_from(group).ok().and_then(|group| characters.get(group)).cloned().unwrap_or_default();
        let plain = self.api.translate_key(native.keycode, 0, indices[0]);
        let symbol = self.api.keysym_name(native.keycode, indices[0]);
        let mut event = KeyEvent {
            pressed: native.pressed,
            keycode: native.keycode,
            key_name: key_name(native.keycode, &symbol, &plain),
            character,
            characters,
            group,
            state: state_mask,
            timestamp: i64::from(native.timestamp),
            synthetic: native.injected,
            ..KeyEvent::default()
        };
        let repeated_answer = !event.synthetic && event.pressed && self.state().consumed_keys.contains(&event.keycode);
        // A key typed without asking has reached the window already: nothing can be withheld.
        let consumed = if native.delivered { KeyDisposition::Pass } else { self.consumes(&event) };
        if consumed == KeyDisposition::Defer {
            self.hold_input();
            let mut state = self.state();
            state.deferred_action = Some(*native);
            state.action_prior_keys = state.pressed.clone();
            event.deferred = true;
        }
        if native.seq != 0 && !native.delivered {
            let kind = match consumed {
                KeyDisposition::Defer => Answer::Deferred,
                KeyDisposition::Consume => Answer::Consumed,
                KeyDisposition::Pass => Answer::Passed,
            };
            self.state().last_answer = Some((native.seq, kind, *native));
        }
        if let (Some(listener), false) = (self.listener(), repeated_answer) {
            listener(event);
        }
        consumed != KeyDisposition::Pass
    }

    fn recheck_modifiers(&self, native: &NativeKeyEvent) {
        let window = self.api.focused_window();
        let held: Vec<u32> = {
            let mut state = self.state();
            let (previous_time, previous_window) = state.last_typed;
            state.last_typed = (native.timestamp, window);
            let held: Vec<u32> = state.pressed.iter().copied().filter(|key| EVDEV_MODIFIER_KEYS.contains(key) && *key != native.keycode).collect();
            if held.is_empty()
                || (window == previous_window && native.timestamp.wrapping_sub(previous_time) & UINT32_MODULO_MASK < MODIFIER_RECHECK_GAP_MS)
            {
                return;
            }
            held
        };
        let mut released: Vec<u32> = held.into_iter().filter(|key| !self.api.key_down(*key)).collect();
        released.sort_unstable();
        if !released.is_empty() {
            {
                let mut state = self.state();
                for key in &released {
                    state.pressed.remove(key);
                }
            }
            let names: Vec<String> = released.iter().map(|key| key_name(*key, "", "")).collect();
            logsetup::info(LOGGER, &format!("Отпускание модификаторов не дошло до перехватчика, они забыты: {}", names.join(", ")));
        }
    }

    fn track_key_state(&self, native: &NativeKeyEvent) {
        let mut state = self.state();
        if native.pressed {
            state.pressed.insert(native.keycode);
            return;
        }
        state.pressed.remove(&native.keycode);
        if state.action_prior_keys.contains(&native.keycode) && state.held.iter().any(|item| item.pressed && item.keycode == native.keycode) {
            state.held.push(NativeKeyEvent { replayed: false, ..*native });
        }
        state.action_prior_keys.remove(&native.keycode);
    }

    fn normalized_state(&self) -> u32 {
        let caps = self.api.caps_lock_enabled();
        let state = self.state();
        let mut mask = if caps { LOCK_MASK } else { 0 };
        let down = |keys: &[u32]| keys.iter().any(|key| state.pressed.contains(key));
        if down(&EVDEV_SHIFT_KEYS) {
            mask |= SHIFT_MASK;
        }
        if down(&EVDEV_CONTROL_KEYS) {
            mask |= CONTROL_MASK;
        }
        if down(&EVDEV_ALT_KEYS) {
            mask |= ALT_MASK;
        }
        if down(&EVDEV_SUPER_KEYS) {
            mask |= SUPER_MASK;
        }
        mask
    }

    fn switch_group(&self, group: i32) -> Result<(), BackendError> {
        if self.current_group()? == group {
            return Ok(());
        }
        let target_index = self.layout_list()?[group.to::<usize>()];
        if !self.api.select_layout_index(target_index) {
            return Err(failed("Среда Wayland отклонила запрос смены раскладки"));
        }
        let deadline = Instant::now() + seconds(LAYOUT_SWITCH_TIMEOUT_SECONDS);
        while Instant::now() < deadline {
            if self.current_group()? == group {
                return Ok(());
            }
            sleep(LAYOUT_SWITCH_POLL_SECONDS);
        }
        Err(BackendError::LayoutSwitchUnconfirmed { message: "Среда Wayland не подтвердила смену раскладки".to_owned(), group })
    }

    /// Switch the layout once the keys just typed had time to reach the client in the old one.
    fn switch_group_after_typing(&self, group: i32) -> Result<(), BackendError> {
        if self.current_group()? != group {
            sleep(WAYLAND_TYPED_INPUT_SETTLE_SECONDS);
        }
        self.switch_group(group)
    }

    /// Type `inputs`; with `known_seq` the first batch is typed only if no key reached the application unasked after that seq,
    /// or for an `action` the key of that seq is not there already (the helper checks; nothing is typed otherwise).
    fn deliver_inputs(&self, inputs: &[NativeInput], mut known_seq: Option<u32>, action: bool) -> Result<(), BackendError> {
        if inputs.is_empty() {
            return Ok(());
        }
        let timestamp = self.api.monotonic_ms();
        let mut index = 0;
        while index < inputs.len() {
            let current = inputs[index];
            if current.synthetic && !current.replayed {
                let mut end = index + 1;
                while end < inputs.len() && inputs[end].synthetic && !inputs[end].replayed {
                    end += 1;
                }
                let batch = &inputs[index..end];
                let raw: Vec<(bool, u32)> = batch.iter().map(|item| (item.pressed, item.keycode)).collect();
                let posted = self.api.post_inputs(&raw, known_seq, action).map_err(failed)?;
                if known_seq.is_some() && posted == 0 {
                    return Err(failed(INPUT_AHEAD_ERROR));
                }
                known_seq = None;
                if posted.to::<usize>() != raw.len() {
                    return Err(failed(format!("uinput принял {posted} из {} событий ввода", raw.len())));
                }
                for item in batch {
                    self.handle_native(NativeKeyEvent { injected: true, ..NativeKeyEvent::new(item.pressed, item.keycode, timestamp) });
                }
                index = end;
            } else {
                let swallow = self.handle_native(NativeKeyEvent { replayed: true, ..NativeKeyEvent::new(current.pressed, current.keycode, timestamp) });
                if !swallow {
                    let posted = self.api.post_inputs(&[(current.pressed, current.keycode)], None, false).map_err(failed)?;
                    if posted != 1 {
                        return Err(failed(format!("uinput принял {posted} из 1 события ввода")));
                    }
                }
                index += 1;
            }
        }
        Ok(())
    }

    fn stroke_inputs(&self, stroke: &KeyEvent, synthetic: bool, group: Option<i32>) -> Vec<NativeInput> {
        let rendered_group = group.unwrap_or(stroke.group);
        let mut shifted = stroke.shift();
        if pystr::isalpha(stroke.character_for(rendered_group)) {
            shifted ^= stroke.caps_lock() != self.api.caps_lock_enabled();
        }
        let replayed = !synthetic;
        let input = |pressed: bool, keycode: u32| NativeInput { pressed, keycode, synthetic, replayed };
        let mut result = Vec::new();
        if shifted {
            result.push(input(true, EVDEV_KEY_LEFTSHIFT));
        }
        result.push(input(true, stroke.keycode));
        result.push(input(false, stroke.keycode));
        if shifted {
            result.push(input(false, EVDEV_KEY_LEFTSHIFT));
        }
        result
    }

    /// The keys that type the characters of `strokes` in `group`, or None if one is missing there.
    fn typed_in_group(&self, strokes: &[KeyEvent], group: i32) -> Result<Option<Vec<NativeInput>>, BackendError> {
        let layout_index = self.layout_list()?[group.to::<usize>()];
        let lock_state = if self.api.caps_lock_enabled() { LOCK_MASK } else { 0 };
        let mut result = Vec::new();
        for stroke in strokes {
            let mut found: Option<(u32, bool)> = None;
            if !stroke.character.is_empty() {
                'search: for code in std::iter::once(stroke.keycode).chain(1..=WAYLAND_UINPUT_MAX_KEYCODE) {
                    for shifted in [false, true] {
                        let state = lock_state | if shifted { SHIFT_MASK } else { 0 };
                        if self.api.translate_key(code, state, layout_index) == stroke.character {
                            found = Some((code, shifted));
                            break 'search;
                        }
                    }
                }
            }
            let Some((code, shifted)) = found else { return Ok(None) };
            if shifted {
                result.push(NativeInput::synthetic(true, EVDEV_KEY_LEFTSHIFT));
            }
            result.extend([NativeInput::synthetic(true, code), NativeInput::synthetic(false, code)]);
            if shifted {
                result.push(NativeInput::synthetic(false, EVDEV_KEY_LEFTSHIFT));
            }
        }
        Ok(Some(result))
    }

    fn group_count(&self) -> usize {
        self.layout_list().map_or(self.group_count, |layouts| layouts.len())
    }

    fn inject_correction(&self, injection: &Injection<'_>) -> Result<i32, BackendError> {
        let outcome = self.inject_correction_inner(injection);
        let released = self.release_input();
        outcome?;
        released
    }

    fn inject_correction_inner(&self, injection: &Injection<'_>) -> Result<i32, BackendError> {
        let Injection { strokes, target_group, boundary, source_group, late, trailing, kept_tail, erased } = *injection;
        let group_count = self.group_count().low::<i32>();
        if !(0 <= target_group && target_group < group_count) {
            return Err(failed(format!("Неизвестная группа раскладки {target_group}")));
        }
        let literal: Vec<KeyEvent> = trailing.iter().cloned().chain(boundary.cloned()).collect();
        let rendered_source_group = source_group.unwrap_or_else(|| strokes.first().map_or(target_group, |stroke| stroke.group));
        if !(0 <= rendered_source_group && rendered_source_group < group_count) {
            return Err(failed(format!("Неизвестная исходная группа раскладки {rendered_source_group}")));
        }
        if kept_tail != 0 && (kept_tail >= strokes.len() || source_group.is_none()) {
            return Err(failed("Некорректная граница сохраняемого слова; замена отменена"));
        }

        let final_group = if kept_tail != 0 { rendered_source_group } else { target_group };
        let delete_count = erased.unwrap_or(strokes.len()) + literal.len() + late.len();
        let delete_inputs: Vec<NativeInput> =
            (0..delete_count).flat_map(|_| [true, false].map(|pressed| NativeInput::synthetic(pressed, EVDEV_KEY_BACKSPACE))).collect();
        let converted = &strokes[..strokes.len() - kept_tail];
        let replay_inputs: Vec<NativeInput> = converted.iter().flat_map(|stroke| self.stroke_inputs(stroke, true, Some(target_group))).collect();
        let kept_inputs: Vec<NativeInput> = strokes[converted.len()..].iter().flat_map(|stroke| self.stroke_inputs(stroke, true, Some(final_group))).collect();
        let mut boundary_inputs: Vec<NativeInput> = literal.iter().flat_map(|stroke| self.stroke_inputs(stroke, true, None)).collect();
        let late_inputs: Vec<NativeInput> = late.iter().flat_map(|stroke| self.stroke_inputs(stroke, false, Some(final_group))).collect();
        let mut preserve_boundary_layout = literal.iter().any(|stroke| stroke.character_for(target_group) != stroke.character);
        let literal_group = literal.first().map_or(rendered_source_group, |stroke| stroke.group);
        if !(0 <= literal_group && literal_group < group_count) || literal.iter().any(|stroke| stroke.group != literal_group) {
            return Err(failed("Некорректная раскладка пунктуации; замена отменена"));
        }
        // A sign the target layout has on another key is typed there: switching back and forth around it races the keys still
        // on their way to the client.
        let literal_in_target = if preserve_boundary_layout && kept_tail == 0 { self.typed_in_group(&literal, target_group)? } else { None };
        if let Some(typed) = literal_in_target {
            boundary_inputs = typed;
            preserve_boundary_layout = false;
        }

        let mut held_shifts: Vec<u32> = {
            let state = self.state();
            EVDEV_SHIFT_KEYS.iter().copied().filter(|key| state.pressed.contains(key)).collect()
        };
        held_shifts.sort_unstable();
        let unshift_inputs: Vec<NativeInput> = held_shifts.iter().map(|code| NativeInput::synthetic(false, *code)).collect();
        let reshift_inputs: Vec<NativeInput> = held_shifts.iter().map(|code| NativeInput::synthetic(true, *code)).collect();

        let mut late_deleted = false;
        let mut late_typed = false;
        let mut failure: Option<BackendError> = None;
        let known_seq = {
            let state = self.state();
            if state.holding { state.hold_known_seq } else { state.known_seq }
        };
        let inject_inner = lock(&self.inject_lock);
        let main = (|| -> Result<(), BackendError> {
            if self.state().hold_broken {
                return Err(failed(INPUT_AHEAD_ERROR));
            }
            self.switch_group(target_group)?;
            {
                let state = self.state();
                let changed = state.holding && (state.pointer_epoch != state.hold_pointer_epoch || self.api.focused_window() != state.hold_window);
                if changed {
                    return Err(failed("Место ввода изменилось до замены; текст не изменён"));
                }
            }
            let mut first: Vec<NativeInput> = unshift_inputs.clone();
            first.extend(&delete_inputs);
            first.extend(&replay_inputs);
            if !(preserve_boundary_layout || kept_tail != 0) {
                first.extend(&boundary_inputs);
            }
            self.deliver_inputs(&first, Some(known_seq), false)?;
            late_deleted = true;
            if kept_tail != 0 {
                self.switch_group_after_typing(final_group)?;
                let mut second = kept_inputs.clone();
                second.extend(&boundary_inputs);
                self.deliver_inputs(&second, None, false)?;
            } else if preserve_boundary_layout {
                self.switch_group_after_typing(literal_group)?;
                self.deliver_inputs(&boundary_inputs, None, false)?;
                self.switch_group_after_typing(target_group)?;
            }
            late_typed = true;
            let mut tail = late_inputs.clone();
            tail.extend(&reshift_inputs);
            self.deliver_inputs(&tail, None, false)
        })();
        if let Err(error) = main {
            failure = Some(error);
        }
        let restore: Vec<NativeInput> = if late_deleted && !late_typed { late_inputs.iter().chain(&reshift_inputs).copied().collect() } else { Vec::new() };
        let mut held_count = 0;
        let rest = self.deliver_inputs(&restore, None, false).and_then(|()| self.release_input());
        match rest {
            Ok(count) => held_count = count,
            Err(error) => failure = failure.or(Some(error)),
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(held_count),
        }
    }

    fn check_hold(&self) -> Result<(), SelectionError> {
        let (broken, holding, epoch_changed, hold_window) = {
            let state = self.state();
            (state.hold_broken, state.holding, state.pointer_epoch != state.hold_pointer_epoch, state.hold_window)
        };
        if broken {
            return Err(failed(INPUT_AHEAD_ERROR).into());
        }
        if holding && (epoch_changed || self.api.focused_window() != hold_window) {
            return Err(SelectionRefused::new(RefusalReason::FocusChanged).into());
        }
        Ok(())
    }

    /// For each layout group, the key that types each character with Caps Lock as it is now: plain before shifted, the lowest
    /// code first (the main keys come before the keypad).
    fn typing_tables(&self) -> Result<Vec<std::collections::HashMap<char, KeyChoice>>, BackendError> {
        let lock_state = if self.api.caps_lock_enabled() { LOCK_MASK } else { 0 };
        let mut tables = Vec::new();
        for layout_index in self.layout_list()? {
            let mut table = std::collections::HashMap::new();
            for code in 1..=WAYLAND_UINPUT_MAX_KEYCODE {
                for shifted in [false, true] {
                    let character = self.api.translate_key(code, lock_state | if shifted { SHIFT_MASK } else { 0 }, layout_index);
                    let mut chars = character.chars();
                    if let (Some(only), None) = (chars.next(), chars.next()) {
                        table.entry(only).or_insert((i64::from(code), shifted));
                    }
                }
            }
            tables.push(table);
        }
        Ok(tables)
    }

    fn key_inputs(key: KeyChoice) -> Vec<NativeInput> {
        let (code, shifted) = key;
        let code = code.low::<u32>();
        let tap = [NativeInput::synthetic(true, code), NativeInput::synthetic(false, code)];
        if !shifted {
            return tap.to_vec();
        }
        let mut keys = vec![NativeInput::synthetic(true, EVDEV_KEY_LEFTSHIFT)];
        keys.extend(tap);
        keys.push(NativeInput::synthetic(false, EVDEV_KEY_LEFTSHIFT));
        keys
    }

    /// Write `replacement` over the selection, then leave the layout in `final_group`.
    ///
    /// A field with AT-SPI's EditableText takes the text from the reader itself, any character included. Otherwise it is typed
    /// through the input helper: keys, not characters, reach the compositor, so each run of one script is typed in its layout
    /// group, and the layout changes only after the keys before had time to arrive (`switch_group_after_typing`). A character
    /// no key of either layout types then leaves the selection as it is, with nothing typed: Wayland lets no program but the
    /// focused one put text on the clipboard. The layout ends in `final_group` or where it was.
    fn replace_selection(&self, selected: &SelectedText, replacement: &str, final_group: Option<i32>) -> Result<bool, SelectionError> {
        {
            let inject_inner = lock(&self.inject_lock);
            self.check_hold()?;
            let written = match lock(&self.selection_source).as_mut() {
                Some(source) => source.write_selection(selected, replacement)?,
                None => None,
            };
            if let Some(written) = written {
                if let Some(group) = final_group {
                    self.switch_group(group)?;
                }
                return Ok(written);
            }
        }
        let original_group = self.current_group()?;
        let runs = typing_runs(replacement, &self.typing_tables()?, i64::from(original_group))?;
        let steps = caret_steps(replacement);
        let reselect: Vec<NativeInput> = match steps {
            None => Vec::new(),
            Some(steps) => {
                let mut keys = vec![NativeInput::synthetic(true, EVDEV_KEY_LEFTSHIFT)];
                for _ in 0..steps {
                    keys.extend([NativeInput::synthetic(true, EVDEV_KEY_LEFT), NativeInput::synthetic(false, EVDEV_KEY_LEFT)]);
                }
                keys.push(NativeInput::synthetic(false, EVDEV_KEY_LEFTSHIFT));
                keys
            }
        };
        let mut known_seq = {
            let state = self.state();
            Some(if state.holding { state.hold_known_seq } else { state.known_seq })
        };
        let inject_inner = lock(&self.inject_lock);
        self.check_hold()?;
        for (index, (group, keys)) in runs.iter().enumerate() {
            let group = (*group).low::<i32>();
            if index > 0 {
                self.switch_group_after_typing(group)?;
            } else {
                self.switch_group(group)?;
            }
            // The first batch goes only if no key reached the program unasked since the hold began.
            let inputs: Vec<NativeInput> = keys.iter().flat_map(|key| Inner::key_inputs(*key)).collect();
            self.deliver_inputs(&inputs, known_seq, false)?;
            known_seq = None;
        }
        self.deliver_inputs(&reselect, None, false)?;
        let final_group = final_group.unwrap_or(original_group);
        if 0 <= final_group && final_group.to::<usize>() < self.group_count() {
            self.switch_group_after_typing(final_group)?;
        }
        Ok(steps.is_some())
    }

    fn read_selection(&self, application: &str, window: i64, process: crate::x11_backend::ProcessForWindow) -> Result<SelectedText, SelectionError> {
        if window != 0 && self.api.window_is_password(window) {
            return Err(SelectionRefused::new(RefusalReason::SensitiveField).into());
        }
        let mut source = lock(&self.selection_source);
        if source.is_none() {
            match &self.selection_factory {
                Some(factory) => *source = Some(factory(process)),
                None => return Err(SelectionRefused::new(RefusalReason::Unsupported).into()),
            }
        }
        source.as_mut().expect("just built").read_selection(application, window)
    }

    fn probe(&self) -> BackendProbe {
        let display = self.api.desktop_name();
        let unavailable = |error: String| BackendProbe {
            available: false,
            session_type: SESSION_TYPE.to_owned(),
            display: display.clone(),
            record_version: "—".to_owned(),
            xtest_version: "—".to_owned(),
            xkb_version: "—".to_owned(),
            current_group: -1,
            error,
        };
        let denied = self.api.permission_error();
        if !denied.is_empty() {
            return unavailable(denied);
        }
        let outcome = (|| -> Result<BackendProbe, BackendError> {
            let names = self.api.layout_names();
            let indices = self.layout_list()?;
            let selected: Vec<String> = indices
                .iter()
                .map(|index| usize::try_from(*index).ok().and_then(|position| names.get(position)).cloned().unwrap_or_else(|| index.to_string()))
                .collect();
            Ok(BackendProbe {
                available: true,
                session_type: SESSION_TYPE.to_owned(),
                display: display.clone(),
                record_version: CAPTURE_METHOD.to_owned(),
                xtest_version: INJECT_METHOD.to_owned(),
                xkb_version: selected.join(","),
                current_group: self.current_group()?,
                error: String::new(),
            })
        })();
        outcome.unwrap_or_else(|error| unavailable(error.to_string()))
    }
}

impl InputBackend for WaylandBackend {
    fn start(&self, listener: KeyListener) -> Result<(), BackendError> {
        let inner = &self.inner;
        if inner.running.load(Ordering::SeqCst) {
            return Ok(());
        }
        let denied = inner.api.permission_error();
        if !denied.is_empty() {
            return Err(failed(denied));
        }
        inner.layouts()?;
        *lock(&inner.listener) = Some(listener);
        inner.ready.clear();
        *lock(&inner.start_error) = None;
        let worker = inner.clone();
        let handle = thread::Builder::new()
            .name("keyswitch-wayland-input".to_owned())
            .spawn(move || {
                let handler: NativeListener = {
                    let worker = worker.clone();
                    Arc::new(move |native| worker.handle_native(native))
                };
                let ready: ReadyCallback = {
                    let worker = worker.clone();
                    Arc::new(move || worker.mark_ready())
                };
                let outcome = worker.api.run_input_listener(handler, ready);
                if let Err(error) = outcome {
                    *lock(&worker.start_error) = Some(error);
                    worker.ready.set();
                }
                worker.running.store(false, Ordering::SeqCst);
            })
            .map_err(|error| failed(error.to_string()))?;
        *lock(&inner.thread) = Some(handle);
        if !inner.ready.wait(seconds(KEYBOARD_LISTENER_START_TIMEOUT_SECONDS)) {
            self.stop_inner();
            return Err(failed(format!(
                "Перехватчик клавиатуры Wayland не подтвердил запуск за {}",
                quantity(KEYBOARD_LISTENER_START_TIMEOUT_SECONDS, SECONDS)
            )));
        }
        let error = lock(&inner.start_error).take();
        if let Some(error) = error {
            *lock(&inner.thread) = None;
            return Err(failed(error));
        }
        Ok(())
    }

    fn set_key_filter(&self, predicate: Option<KeyFilter>) {
        *lock(&self.inner.key_filter) = predicate;
    }

    fn hold_input(&self) {
        self.inner.hold_input();
    }

    fn release_input(&self) -> Result<i32, BackendError> {
        self.inner.release_input()
    }

    fn complete_action(&self, deliver: bool) -> Result<i32, BackendError> {
        self.inner.complete_action(deliver)
    }

    fn stop(&self) {
        self.stop_inner();
    }

    fn close(&self) {
        self.stop_inner();
        *lock(&self.inner.listener) = None;
        self.inner.api.close();
    }

    fn current_group(&self) -> Result<i32, BackendError> {
        self.inner.current_group()
    }

    fn switch_group(&self, group: i32) -> Result<(), BackendError> {
        if !(0 <= group && group.to::<usize>() < self.inner.group_count()) {
            return Err(failed(format!("Неизвестная группа раскладки {group}")));
        }
        let inject_inner = lock(&self.inner.inject_lock);
        self.inner.switch_group(group)
    }

    fn active_application(&self) -> String {
        self.inner.api.active_application()
    }

    fn focused_window(&self) -> Option<FocusInfo> {
        let api = &self.inner.api;
        let window = api.focused_window();
        if window == 0 {
            return None;
        }
        let pid = api.window_process_id(window);
        let own = pid != 0 && pid == api.current_process_id();
        Some(FocusInfo { window, own, isolated_layout: api.isolated_layout() })
    }

    fn probe(&self) -> BackendProbe {
        self.inner.probe()
    }

    fn inject_correction(&self, injection: Injection<'_>) -> Result<i32, BackendError> {
        self.inner.inject_correction(&injection)
    }

    fn swallows_hotkeys(&self) -> bool {
        true
    }

    fn take_input_delay(&self) -> Option<InputDelayReport> {
        Some(self.inner.input_delay.take())
    }

    fn supports_selection(&self) -> bool {
        true
    }

    /// The text selected in the focused field, through AT-SPI; never in a field the compositor says is a password field (GNOME
    /// Shell draws its own prompts, which AT-SPI does not read).
    fn read_selection(&self, application: &str, window: i64) -> Result<SelectedText, SelectionError> {
        self.inner.read_selection(application, window, self.process_for_window())
    }

    fn replace_selection(&self, selected: &SelectedText, replacement: &str, final_group: Option<i32>) -> Result<bool, SelectionError> {
        self.inner.replace_selection(selected, replacement, final_group)
    }

    fn input_anchor(&self) -> Option<ScreenAnchor> {
        self.inner.api.input_anchor()
    }

    fn restore_window(&self, window: Option<i64>) -> bool {
        window.is_some_and(|window| window > 0 && self.inner.api.activate_window(window))
    }
}

#[cfg(test)]
#[path = "wayland_backend_tests.rs"]
mod tests;
