//! Windows keyboard backend built around a small, testable native API.
//!
//! A private `_name` of the Python is `name` here, and methods live on the shared `Inner` that the hook thread also
//! holds. `_switch_group` and `_inject_correction` keep their underscore, because `switch_group` and `inject_correction`
//! exist beside them.
//!
//! Two threads run this code, and the Python's interpreter lock is gone, so each field says how it is shared:
//!
//! * The hook thread runs `handle_native` for every key of the system, and every key waits for its answer. It takes only
//!   small mutexes that are held for field copies, reads atomics, and makes short calls that read system state. It
//!   never sends input: SendInput makes the system call the hook, which would then wait for itself.
//! * The engine's worker types corrections. It holds `inject_lock` across SendInput and across its waits for a layout,
//!   and the hook never takes that lock. `hold` is taken by both, and never across SendInput or a wait.
//!
//! A panic on the hook's path takes the program down (a callback of the system cannot unwind, and the release profile
//! aborts), so nothing on that path indexes or unwraps; what the Python's callback would have raised lets the key pass.

use std::collections::{BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

use ks_backend::constants::{
    ALT_MASK, COMPLETED_ACTION_EVENT_COUNT, CONTROL_MASK, LAYOUT_GROUP_COUNT, LOCK_MASK, POINTER_EVENT_GROUP, SHIFT_MASK,
    SUPER_MASK, UNICODE_PACKET_KEY_NAME,
};
use ks_backend::selected_text::{SOURCE_CLIPBOARD, SOURCE_UIA};
use ks_backend::{
    BackendError, BackendProbe, FocusInfo, HookGap, InputBackend, InputDelay, InputDelayReport, Injection, KeyDisposition,
    KeyEvent, KeyFilter, KeyListener, RefusalReason, ScreenAnchor, SelectedText, SelectionError, SelectionRefused,
};
use ks_config::selection_commands::caret_steps;
use ks_text::pystr;
use ks_text::russian_text::{SECONDS, quantity};

use crate::constants::{
    ALT_KEYS, CF_UNICODETEXT, CLIPBOARD_SNAPSHOT_MAX_BYTES, CONSOLE_WINDOW_CLASSES, CONTROL_KEYS, DWORD_MASK, HOOK_SILENCE_MS,
    KEYBOARD_LISTENER_START_TIMEOUT_SECONDS, KEYBOARD_LISTENER_STOP_TIMEOUT_SECONDS, LANG_ENGLISH, LANG_RUSSIAN,
    LAYOUT_SWITCH_POLL_SECONDS, LOWORD_MASK, MILLISECONDS_PER_SECOND, MODIFIER_KEYS, MODIFIER_RECHECK_GAP_MS,
    PRIMARY_LANGID_MASK, SELECTION_CLIPBOARD_POLL_SECONDS, SELECTION_CLIPBOARD_TIMEOUT_SECONDS,
    SELECTION_COMMAND_MAX_CHARACTERS, SELECTION_PASTE_CONFIRM_SECONDS, SHIFT_KEYS, SLOW_INPUT_FIELD_CLASSES, SUPER_KEYS,
    UNSEEN_INPUT_MIN_TIMES, VK_0, VK_9, VK_A, VK_BACK, VK_CAPITAL, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE,
    VK_HOME, VK_INSERT, VK_LCONTROL, VK_LEFT, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_MENU_MASK, VK_NEXT, VK_OEM_1,
    VK_OEM_2, VK_OEM_3, VK_OEM_4, VK_OEM_5, VK_OEM_6, VK_OEM_7, VK_OEM_COMMA, VK_OEM_MINUS, VK_OEM_PERIOD, VK_OEM_PLUS,
    VK_PACKET, VK_PAUSE, VK_PRIOR, VK_RCONTROL, VK_RETURN, VK_RIGHT, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SCROLL, VK_SHIFT,
    VK_SPACE, VK_TAB, VK_UP, VK_V, VK_Z, WINDOWS_LAYOUT_SETTLE_SECONDS, WINDOWS_LAYOUT_SWITCH_TIMEOUT_SECONDS,
};
use crate::log;

const LOGGER: &str = "keyswitch.windows_backend";

/// The Python's `WindowsBackendError` and `WindowsLayoutSwitchUnconfirmed` are the two variants of `BackendError`.
pub type WindowsBackendError = BackendError;

fn failed(message: impl Into<String>) -> WindowsBackendError {
    BackendError::Failed(message.into())
}

fn refused(reason: RefusalReason) -> SelectionError {
    SelectionRefused::new(reason).into()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeKeyEvent {
    pub pressed: bool,
    pub virtual_key: u32,
    pub scan_code: u32,
    pub extended: bool,
    pub injected: bool,
    pub timestamp: i64,
    pub replayed: bool,
    /// Injected by another program; `injected` means KeySwitch's own keys only.
    pub foreign: bool,
}

impl NativeKeyEvent {
    pub const fn new(pressed: bool, virtual_key: u32, scan_code: u32, extended: bool, injected: bool, timestamp: i64) -> Self {
        NativeKeyEvent { pressed, virtual_key, scan_code, extended, injected, timestamp, replayed: false, foreign: false }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeInput {
    pub pressed: bool,
    pub virtual_key: u32,
    pub scan_code: u32,
    pub extended: bool,
    pub synthetic: bool,
    pub replayed: bool,
    /// `scan_code` is a Unicode character (KEYEVENTF_UNICODE), not a key position.
    pub unicode: bool,
    /// A replay of a key another program injected.
    pub foreign: bool,
}

impl NativeInput {
    pub const fn new(pressed: bool) -> Self {
        NativeInput {
            pressed,
            virtual_key: 0,
            scan_code: 0,
            extended: false,
            synthetic: true,
            replayed: false,
            unicode: false,
            foreign: false,
        }
    }

    pub const fn key(pressed: bool, virtual_key: u32) -> Self {
        NativeInput { virtual_key, ..NativeInput::new(pressed) }
    }

    pub const fn extended_key(pressed: bool, virtual_key: u32) -> Self {
        NativeInput { virtual_key, extended: true, ..NativeInput::new(pressed) }
    }

    pub const fn scan(pressed: bool, scan_code: u32) -> Self {
        NativeInput { scan_code, ..NativeInput::new(pressed) }
    }
}

/// Answers on the hook's thread whether the window must not get the key.
pub type HookListener = Arc<dyn Fn(NativeKeyEvent) -> bool + Send + Sync>;
/// Called on the hook's thread once the hooks are in place.
pub type HookReady = Box<dyn FnOnce() + Send>;

pub trait WindowsAPI: Send + Sync {
    fn loaded_layouts(&self) -> Result<Vec<i64>, WindowsBackendError>;
    fn foreground_layout(&self) -> i64;
    fn request_layout(&self, layout: i64) -> bool;
    fn translate_key(&self, virtual_key: u32, scan_code: u32, state: u32, layout: i64) -> String;
    /// SendInput: how many events it took. Never called on the hook's thread.
    fn send_inputs(&self, inputs: &[NativeInput]) -> Result<usize, WindowsBackendError>;
    fn active_application(&self) -> String;
    fn foreground_window(&self) -> i64;
    fn focused_control(&self) -> i64;
    fn window_process_id(&self, window: i64) -> i64;
    fn current_process_id(&self) -> i64;
    fn input_anchor(&self) -> Option<ScreenAnchor>;
    fn activate_window(&self, window: i64) -> bool;
    fn keep_window_inactive(&self, window: i64) -> bool;
    fn caps_lock_enabled(&self) -> bool;
    /// GetAsyncKeyState: whether the key is down on the keyboard right now.
    fn key_down(&self, virtual_key: u32) -> bool;
    /// GetTickCount: milliseconds since boot, the clock of a hook's key time.
    fn tick_count(&self) -> i64;
    /// GetLastInputInfo: the time of the newest key or pointer event of the session, on the clock of tick_count.
    fn last_input_time(&self) -> Result<i64, WindowsBackendError>;
    /// The times of the recent key and pointer events the hooks got, newest last, and of the hooks' start.
    fn hook_input_times(&self) -> Vec<i64>;
    /// Runs the hooks on this thread until `stop_keyboard_hook`.
    fn run_keyboard_hook(&self, listener: HookListener, ready: HookReady) -> Result<(), WindowsBackendError>;
    fn stop_keyboard_hook(&self);
    /// The clipboard calls, where this Windows has them (Python: `isinstance(api, WindowsSelectionAPI)`).
    fn selection_api(&self) -> Option<&dyn WindowsSelectionAPI> {
        None
    }
}

fn named_key(virtual_key: u32) -> Option<&'static str> {
    Some(match virtual_key {
        VK_BACK => "BackSpace",
        VK_TAB => "Tab",
        VK_RETURN => "Return",
        VK_SHIFT => "Shift_L",
        VK_CONTROL => "Control_L",
        VK_MENU => "Alt_L",
        VK_PAUSE => "Pause",
        VK_SCROLL => "Scroll_Lock",
        VK_CAPITAL => "Caps_Lock",
        VK_ESCAPE => "Escape",
        VK_SPACE => "space",
        VK_PRIOR => "Page_Up",
        VK_NEXT => "Page_Down",
        VK_END => "End",
        VK_HOME => "Home",
        VK_LEFT => "Left",
        VK_UP => "Up",
        VK_RIGHT => "Right",
        VK_DOWN => "Down",
        VK_INSERT => "Insert",
        VK_DELETE => "Delete",
        VK_LWIN => "Super_L",
        VK_RWIN => "Super_R",
        VK_LSHIFT => "Shift_L",
        VK_RSHIFT => "Shift_R",
        VK_LCONTROL => "Control_L",
        VK_RCONTROL => "Control_R",
        VK_LMENU => "Alt_L",
        VK_RMENU => "Alt_R",
        VK_OEM_1 => "semicolon",
        VK_OEM_PLUS => "equal",
        VK_OEM_COMMA => "comma",
        VK_OEM_MINUS => "minus",
        VK_OEM_PERIOD => "period",
        VK_OEM_2 => "slash",
        VK_OEM_3 => "grave",
        VK_OEM_4 => "bracketleft",
        VK_OEM_5 => "backslash",
        VK_OEM_6 => "bracketright",
        VK_OEM_7 => "apostrophe",
        VK_PACKET => UNICODE_PACKET_KEY_NAME,
        _ => return None,
    })
}

/// The clipboard as a selection hotkey found it: each format and its bytes, in the order they were offered.
///
/// `complete` is false when something on it cannot be kept as bytes and put back (a metafile, a private handle, a format
/// that would not render) or it holds more than the hotkeys keep: such a clipboard is not borrowed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardContents {
    pub formats: Vec<(u32, Vec<u8>)>,
    pub complete: bool,
}

/// What the selection hotkeys need of Windows besides typing: the window class and the clipboard.
///
/// Each clipboard call opens the clipboard once and closes it again; None (false for the write) says another program
/// has it open, and the backend asks again until its deadline.
pub trait WindowsSelectionAPI {
    fn window_class(&self, window: i64) -> String;
    /// Whether the thread of `window` shows a text caret: GetGUIThreadInfo's hwndCaret, with a rcCaret that is not empty.
    fn text_caret(&self, window: i64) -> bool;
    fn clipboard_sequence(&self) -> i64;
    fn read_clipboard(&self, limit_bytes: usize) -> Option<ClipboardContents>;
    fn clipboard_text(&self, limit_characters: usize) -> Option<String>;
    fn write_clipboard(&self, contents: &ClipboardContents) -> Result<bool, WindowsBackendError>;
}

/// UI Automation as the selection hotkeys ask it (`windows_context::WindowsFieldReader`).
pub trait SelectionReader: Send {
    /// The selected text, None where UI Automation cannot say, or why the selection is not one to type over.
    fn read_selection(&self, window: i64) -> Result<Option<SelectedText>, SelectionError>;
    /// Whether the focused element of `window` is an edit field, which may still have no text pattern.
    fn focused_editable(&self, window: i64) -> Result<bool, SelectionError>;
}

pub type SelectionReaderFactory = Box<dyn Fn() -> Result<Box<dyn SelectionReader>, WindowsBackendError> + Send + Sync>;
/// Runs a short job off the hook's thread.
pub type JobStarter = Box<dyn Fn(Box<dyn FnOnce() + Send>) -> Result<(), WindowsBackendError> + Send + Sync>;

/// UI Automation, built on the engine worker that asks it (COM objects belong to their thread).
#[cfg(windows)]
fn uia_selection_reader() -> Result<Box<dyn SelectionReader>, WindowsBackendError> {
    match crate::windows_context::WindowsFieldReader::new() {
        Ok(reader) => Ok(Box::new(reader)),
        Err(failure) => Err(failed(failure.failure_name)),
    }
}

#[cfg(not(windows))]
fn uia_selection_reader() -> Result<Box<dyn SelectionReader>, WindowsBackendError> {
    Err(failed("UI Automation есть только в Windows"))
}

/// The clocks of the module: `time.monotonic`, `time.perf_counter` and `time.sleep`.
pub trait Clock: Send + Sync {
    fn monotonic(&self) -> f64;
    fn perf_counter(&self) -> f64;
    fn sleep(&self, seconds: f64);
}

pub struct SystemClock {
    started: Instant,
}

impl SystemClock {
    pub fn new() -> Self {
        SystemClock { started: Instant::now() }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn monotonic(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    fn perf_counter(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    fn sleep(&self, seconds: f64) {
        thread::sleep(Duration::from_secs_f64(seconds));
    }
}

pub fn primary_language(layout: i64) -> i64 {
    (layout & LOWORD_MASK) & PRIMARY_LANGID_MASK
}

pub fn select_layout_pair(layouts: &[i64]) -> Result<(i64, i64), WindowsBackendError> {
    let first_of = |language: i64| layouts.iter().copied().find(|&layout| primary_language(layout) == language).unwrap_or(0);
    let english = first_of(LANG_ENGLISH);
    let russian = first_of(LANG_RUSSIAN);
    if english == 0 || russian == 0 {
        return Err(failed("В Windows должны быть установлены английская и русская раскладки"));
    }
    Ok((english, russian))
}

pub fn key_name(virtual_key: u32) -> String {
    if let Some(name) = named_key(virtual_key) {
        return name.to_owned();
    }
    if (VK_0..=VK_9).contains(&virtual_key) || (VK_A..=VK_Z).contains(&virtual_key) {
        // The digits and capital letters of ASCII: the casefold of each is its lower case.
        return char::from_u32(virtual_key).map_or_else(String::new, |key| key.to_ascii_lowercase().to_string());
    }
    format!("VK_{virtual_key:02X}")
}

/// A key time after another on the 32-bit tick clock, which wraps after 49.7 days.
fn later(time: i64, than: i64) -> bool {
    let distance = (time - than) & DWORD_MASK;
    0 < distance && distance <= DWORD_MASK >> 1
}

fn lock<T: ?Sized>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn valid_group(group: i32) -> bool {
    (0..LAYOUT_GROUP_COUNT).contains(&group)
}

/// A value the engine's thread sets now and then and the hook's thread copies at each key. The mutex is held only for
/// the copy of the handle; what the old value owned is released after it, so the hook never waits for that.
struct Slot<T>(Mutex<Option<T>>);

impl<T: Clone> Slot<T> {
    const fn new() -> Self {
        Slot(Mutex::new(None))
    }

    fn get(&self) -> Option<T> {
        lock(&self.0).clone()
    }

    fn set(&self, value: Option<T>) {
        let previous = std::mem::replace(&mut *lock(&self.0), value);
        drop(previous);
    }
}

/// A set of keys. Few are ever down together, so a short list is enough and, once it has grown, allocates no more.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct KeySet(Vec<u32>);

impl KeySet {
    fn contains(&self, key: u32) -> bool {
        self.0.contains(&key)
    }

    fn insert(&mut self, key: u32) {
        if !self.contains(key) {
            self.0.push(key);
        }
    }

    fn remove(&mut self, key: u32) -> bool {
        match self.0.iter().position(|&item| item == key) {
            Some(index) => {
                self.0.swap_remove(index);
                true
            }
            None => false,
        }
    }

    fn clear(&mut self) {
        self.0.clear();
    }

    fn intersects(&self, keys: &[u32]) -> bool {
        self.0.iter().any(|key| keys.contains(key))
    }
}

/// What only the hook's thread touches. The mutex exists because `handle_native` takes `&self`; nothing else ever holds
/// it, and it is never held across the filter, the listener or another lock's owner.
#[derive(Default)]
struct HookState {
    pressed: KeySet,
    consumed_keys: KeySet,
    /// Key time and window in front at the last key typed on this keyboard.
    last_typed: (i64, i64),
}

/// While a correction is being injected the user's own keys are kept back here and typed again afterwards, so they can
/// neither land between the backspaces and the replacement nor be lost. Both threads take this mutex; it is held for
/// field copies and for the short calls of `hold_input` that read the focus, never across SendInput or a wait.
#[derive(Default)]
struct HoldState {
    holding: bool,
    held: VecDeque<NativeKeyEvent>,
    hold_pointer_epoch: u64,
    hold_window: i64,
    deferred_action: Option<NativeKeyEvent>,
    action_prior_keys: KeySet,
}

/// An input time the hooks have not got yet, the input times seen since it with no hook event between, and the last one
/// reported as passed them (take_unseen_input). Only the engine's thread asks.
#[derive(Default)]
struct UnseenState {
    unseen_input: Option<i64>,
    unseen_times: BTreeSet<i64>,
    unseen_reported: Option<i64>,
}

struct HookThread {
    id: ThreadId,
    /// Disconnects when the thread ends: `thread.join(timeout=...)`, which std's handle does not offer.
    finished: mpsc::Receiver<()>,
}

/// What the backend needs apart from the API; tests replace these.
pub struct BackendParts {
    pub clock: Arc<dyn Clock>,
    pub start_job: JobStarter,
    pub selection_reader_factory: SelectionReaderFactory,
    pub listener_start_timeout_seconds: f64,
}

impl Default for BackendParts {
    fn default() -> Self {
        BackendParts {
            clock: Arc::new(SystemClock::new()),
            start_job: Box::new(|job| {
                thread::Builder::new().spawn(job).map(drop).map_err(|error| failed(error.to_string()))
            }),
            selection_reader_factory: Box::new(uia_selection_reader),
            listener_start_timeout_seconds: KEYBOARD_LISTENER_START_TIMEOUT_SECONDS,
        }
    }
}

struct Inner {
    api: Arc<dyn WindowsAPI>,
    clock: Arc<dyn Clock>,
    /// Asked before the hook starts, so the hook's thread only ever reads the value.
    layouts: OnceLock<(i64, i64)>,
    listener: Slot<KeyListener>,
    key_filter: Slot<KeyFilter>,
    thread: Mutex<Option<HookThread>>,
    /// Atomic: set on the hook's thread, read on any.
    running: AtomicBool,
    start_error: Mutex<Option<WindowsBackendError>>,
    listener_start_timeout_seconds: f64,
    hook: Mutex<HookState>,
    /// Atomic: the hook's thread toggles it and the engine's thread reads it while it types.
    caps_lock: AtomicBool,
    /// Runs a short job off the hook's thread (mask_menu): SendInput there would wait for the hook it is in.
    start_job: JobStarter,
    /// Atomic: the hook's thread counts pointer presses, the engine's thread compares the count.
    pointer_epoch: AtomicU64,
    hold: Mutex<HoldState>,
    /// Held by the engine's thread across SendInput and layout waits. The hook's thread never takes it.
    inject_lock: Mutex<()>,
    input_delay: InputDelay,
    unseen: Mutex<UnseenState>,
    /// UI Automation for the selection hotkeys, made on first use by the factory (read_selection). Engine's thread only.
    selection_reader: Mutex<Option<Box<dyn SelectionReader>>>,
    selection_reader_factory: SelectionReaderFactory,
}

/// Observe global Win32 keyboard events and inject physical corrections.
pub struct WindowsBackend {
    inner: Arc<Inner>,
}

impl WindowsBackend {
    /// The backend over `api`, or over the real Windows for None.
    pub fn new(api: Option<Arc<dyn WindowsAPI>>) -> Result<Self, WindowsBackendError> {
        match api {
            Some(api) => Ok(Self::with_api(api)),
            None => Self::native_api().map(Self::with_api),
        }
    }

    #[cfg(windows)]
    fn native_api() -> Result<Arc<dyn WindowsAPI>, WindowsBackendError> {
        Ok(Arc::new(crate::windows_native::NativeWindowsAPI::new()))
    }

    #[cfg(not(windows))]
    fn native_api() -> Result<Arc<dyn WindowsAPI>, WindowsBackendError> {
        Err(failed("Win32 backend доступен только в Windows"))
    }

    pub fn with_api(api: Arc<dyn WindowsAPI>) -> Self {
        Self::with_parts(api, BackendParts::default())
    }

    pub fn with_parts(api: Arc<dyn WindowsAPI>, parts: BackendParts) -> Self {
        WindowsBackend {
            inner: Arc::new(Inner {
                api,
                clock: parts.clock,
                layouts: OnceLock::new(),
                listener: Slot::new(),
                key_filter: Slot::new(),
                thread: Mutex::new(None),
                running: AtomicBool::new(false),
                start_error: Mutex::new(None),
                listener_start_timeout_seconds: parts.listener_start_timeout_seconds,
                hook: Mutex::new(HookState::default()),
                caps_lock: AtomicBool::new(false),
                start_job: parts.start_job,
                pointer_epoch: AtomicU64::new(0),
                hold: Mutex::new(HoldState::default()),
                inject_lock: Mutex::new(()),
                input_delay: InputDelay::new(),
                unseen: Mutex::new(UnseenState::default()),
                selection_reader: Mutex::new(None),
                selection_reader_factory: parts.selection_reader_factory,
            }),
        }
    }

    pub fn running(&self) -> bool {
        self.inner.running.load(Ordering::Acquire)
    }

    pub fn layouts(&self) -> Result<(i64, i64), WindowsBackendError> {
        self.inner.layouts()
    }

    /// The hook's answer for one key: whether the window must not get it.
    pub fn handle_native(&self, native: NativeKeyEvent) -> bool {
        self.inner.handle_native(native)
    }

    /// The modifiers and Caps Lock as the hook's thread keeps them.
    pub fn normalized_state(&self) -> u32 {
        self.inner.normalized_state()
    }

    /// `_listener` set without the hook, for what drives `handle_native` itself.
    pub fn set_listener(&self, listener: Option<KeyListener>) {
        self.inner.listener.set(listener);
    }
}

impl InputBackend for WindowsBackend {
    fn start(&self, listener: KeyListener) -> Result<(), BackendError> {
        self.inner.start(listener)
    }

    /// Decide, inside the hook, which keys must not reach the window.
    fn set_key_filter(&self, predicate: Option<KeyFilter>) {
        self.inner.key_filter.set(predicate);
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
        self.inner.stop();
    }

    fn close(&self) {
        self.inner.close();
    }

    fn current_group(&self) -> Result<i32, BackendError> {
        self.inner.current_group()
    }

    fn switch_group(&self, group: i32) -> Result<(), BackendError> {
        self.inner.switch_group(group)
    }

    fn active_application(&self) -> String {
        self.inner.api.active_application()
    }

    fn focused_window(&self) -> Option<FocusInfo> {
        self.inner.focused_window()
    }

    fn probe(&self) -> BackendProbe {
        self.inner.probe()
    }

    fn inject_correction(&self, injection: Injection<'_>) -> Result<i32, BackendError> {
        self.inner.inject_correction(&injection)
    }

    fn take_unseen_input(&self) -> Result<Option<i64>, BackendError> {
        self.inner.take_unseen_input()
    }

    fn take_input_delay(&self) -> Option<InputDelayReport> {
        Some(self.inner.input_delay.take())
    }

    fn take_hook_gap(&self) -> Option<HookGap> {
        self.inner.input_delay.take_gap()
    }

    fn supports_selection(&self) -> bool {
        true
    }

    fn read_selection(&self, application: &str, window: i64) -> Result<SelectedText, SelectionError> {
        self.inner.read_selection(application, window)
    }

    fn replace_selection(
        &self,
        selected: &SelectedText,
        replacement: &str,
        final_group: Option<i32>,
    ) -> Result<bool, SelectionError> {
        self.inner.replace_selection(selected, replacement, final_group)
    }

    fn input_anchor(&self) -> Option<ScreenAnchor> {
        self.inner.api.input_anchor()
    }

    fn restore_window(&self, window: Option<i64>) -> bool {
        window.is_some_and(|window| self.inner.api.activate_window(window))
    }

    fn keep_window_inactive(&self, window: i64) -> bool {
        self.inner.api.keep_window_inactive(window)
    }
}

impl Inner {
    fn layouts(&self) -> Result<(i64, i64), WindowsBackendError> {
        if let Some(layouts) = self.layouts.get() {
            return Ok(*layouts);
        }
        let layouts = select_layout_pair(&self.api.loaded_layouts()?)?;
        Ok(*self.layouts.get_or_init(|| layouts))
    }

    fn probe(&self) -> BackendProbe {
        let answer = (|| -> Result<BackendProbe, WindowsBackendError> {
            let (english, russian) = self.layouts()?;
            let group = self.group_for_layout(self.api.foreground_layout())?;
            let layout_names = format!("{:08X},{:08X}", english & DWORD_MASK, russian & DWORD_MASK);
            Ok(BackendProbe {
                available: true,
                session_type: "windows".to_owned(),
                display: "Win32 desktop".to_owned(),
                record_version: "WH_KEYBOARD_LL".to_owned(),
                xtest_version: "SendInput".to_owned(),
                xkb_version: format!("HKL {layout_names}"),
                current_group: group,
                error: String::new(),
            })
        })();
        answer.unwrap_or_else(|error| BackendProbe {
            available: false,
            session_type: "windows".to_owned(),
            display: "Win32 desktop".to_owned(),
            record_version: "—".to_owned(),
            xtest_version: "—".to_owned(),
            xkb_version: "—".to_owned(),
            current_group: -1,
            error: error.to_string(),
        })
    }

    fn start(self: &Arc<Self>, listener: KeyListener) -> Result<(), WindowsBackendError> {
        if self.running.load(Ordering::Acquire) {
            return Ok(());
        }
        self.layouts()?;
        self.listener.set(Some(listener));
        self.caps_lock.store(self.api.caps_lock_enabled(), Ordering::Relaxed);
        *lock(&self.start_error) = None;
        let (ready_sender, ready) = mpsc::channel::<()>();
        let (finished_sender, finished) = mpsc::channel::<()>();
        let inner = Arc::clone(self);
        let handle = thread::Builder::new()
            .name("keyswitch-win32-hook".to_owned())
            .spawn(move || {
                // Dropped as the thread ends, whatever ended it.
                let _finished = finished_sender;
                inner.hook_loop(ready_sender);
            })
            .map_err(|error| failed(error.to_string()))?;
        *lock(&self.thread) = Some(HookThread { id: handle.thread().id(), finished });
        // Not joined: as a daemon thread of the Python, it ends with the process.
        drop(handle);
        let timeout = self.listener_start_timeout_seconds;
        if ready.recv_timeout(Duration::from_secs_f64(timeout)).is_err() {
            self.stop();
            return Err(failed(format!("Win32 hook не подтвердил запуск за {}", quantity(timeout, SECONDS))));
        }
        if let Some(error) = self.startup_error() {
            *lock(&self.thread) = None;
            return Err(failed(error.to_string()));
        }
        Ok(())
    }

    /// Read the error set by the hook thread after its ready signal.
    fn startup_error(&self) -> Option<WindowsBackendError> {
        lock(&self.start_error).clone()
    }

    fn hook_loop(self: &Arc<Self>, ready: mpsc::Sender<()>) {
        let handling = Arc::clone(self);
        let listener: HookListener = Arc::new(move |native| handling.handle_native(native));
        let marking = Arc::clone(self);
        let signal = ready.clone();
        let mark_ready: HookReady = Box::new(move || {
            marking.mark_ready();
            let _ = signal.send(());
        });
        if let Err(error) = self.api.run_keyboard_hook(listener, mark_ready) {
            *lock(&self.start_error) = Some(error);
            let _ = ready.send(());
        }
        self.running.store(false, Ordering::Release);
    }

    fn mark_ready(&self) {
        self.running.store(true, Ordering::Release);
    }

    fn stop(&self) {
        self.key_filter.set(None);
        // The Python's stop raises here and leaves the hook in place; the trait's stop has no error to give.
        if let Err(error) = self.complete_action(false) {
            log::error(LOGGER, &error.to_string());
            return;
        }
        let hook_thread = lock(&self.thread).take();
        let Some(hook_thread) = hook_thread else {
            return;
        };
        self.api.stop_keyboard_hook();
        if hook_thread.id != thread::current().id() {
            // Disconnected or timed out: either way the wait is over, and the thread is left to end alone.
            let _ = hook_thread.finished.recv_timeout(Duration::from_secs_f64(KEYBOARD_LISTENER_STOP_TIMEOUT_SECONDS));
        }
        self.running.store(false, Ordering::Release);
    }

    fn close(&self) {
        self.stop();
        self.listener.set(None);
    }

    fn current_group(&self) -> Result<i32, WindowsBackendError> {
        self.group_for_layout(self.api.foreground_layout())
    }

    fn switch_group(&self, group: i32) -> Result<(), WindowsBackendError> {
        self.layouts()?;
        if !valid_group(group) {
            return Err(failed(format!("Неизвестная группа раскладки {group}")));
        }
        let _inject = lock(&self.inject_lock);
        self._switch_group(group).map(drop)
    }

    fn group_for_layout(&self, layout: i64) -> Result<i32, WindowsBackendError> {
        let (english, russian) = self.layouts()?;
        if layout == english {
            return Ok(0);
        }
        if layout == russian {
            return Ok(1);
        }
        let language = primary_language(layout);
        if primary_language(english) == language {
            return Ok(0);
        }
        if primary_language(russian) == language {
            return Ok(1);
        }
        Ok(-1)
    }

    fn focused_window(&self) -> Option<FocusInfo> {
        let window = self.api.foreground_window();
        if window == 0 {
            return None;
        }
        let process_id = self.api.window_process_id(window);
        let own = process_id != 0 && process_id == self.api.current_process_id();
        let control = self.api.focused_control();
        Some(FocusInfo { window: if control != 0 { control } else { window }, own, isolated_layout: own })
    }

    /// The place typed keys go to: the focused control, or the window in front when it names none.
    fn input_place(&self) -> i64 {
        let control = self.api.focused_control();
        if control != 0 { control } else { self.api.foreground_window() }
    }

    /// Whether a pointer press or a change of focus came since the hold began. Called with the fields of the hold copied
    /// out of its mutex.
    fn input_place_changed(&self, hold_pointer_epoch: u64, hold_window: i64) -> bool {
        self.pointer_epoch.load(Ordering::Acquire) != hold_pointer_epoch || self.input_place() != hold_window
    }

    fn holding_place(&self) -> Option<(u64, i64)> {
        let hold = lock(&self.hold);
        hold.holding.then_some((hold.hold_pointer_epoch, hold.hold_window))
    }

    /// Ask the filter about a press, and hide the matching release.
    ///
    /// A window that saw no key-down must not receive the key-up either, so the release of a swallowed key is swallowed
    /// with it.
    fn consumes(&self, event: &KeyEvent) -> KeyDisposition {
        if event.synthetic {
            return KeyDisposition::Pass;
        }
        {
            let mut hook = lock(&self.hook);
            if !event.pressed {
                return if hook.consumed_keys.remove(event.keycode) { KeyDisposition::Consume } else { KeyDisposition::Pass };
            }
            if hook.consumed_keys.contains(event.keycode) {
                return KeyDisposition::Consume;
            }
        }
        let Some(predicate) = self.key_filter.get() else {
            return KeyDisposition::Pass;
        };
        let decision = predicate(event);
        if decision != KeyDisposition::Pass {
            lock(&self.hook).consumed_keys.insert(event.keycode);
        }
        decision
    }

    /// Press the mask key (VK_MENU_MASK) while Alt or Win is held under a hotkey key the window does not get.
    ///
    /// Seeing Alt go down and up with no key between, a window opens its menu bar on the release (Win opens the Start
    /// menu): after Alt+Pause, Notepad of Windows 11 took the result of the selection hotkey into its menu and left the
    /// text as it was; with a window whose menu has access keys, the characters typed would have chosen menu items. The
    /// key goes off the hook's thread, before the release.
    fn mask_menu(&self) {
        let mask = [NativeInput::key(true, VK_MENU_MASK), NativeInput::key(false, VK_MENU_MASK)];
        let api = Arc::clone(&self.api);
        let started = (self.start_job)(Box::new(move || send_mask(api.as_ref(), &mask)));
        if let Err(error) = started {
            log::info_with_error(LOGGER, "Клавиша-маска для Alt не отправлена", &error);
        }
    }

    /// Keep the user's keys back until the next correction has landed.
    fn hold_input(&self) {
        self.hold_input_in(&mut lock(&self.hold));
    }

    /// Reads the focus under the mutex, as the Python does, so that the place and `holding` change together. Both calls
    /// read state of the system and return: they neither wait on another thread nor reach the hook.
    fn hold_input_in(&self, hold: &mut HoldState) {
        if !hold.holding {
            hold.hold_window = self.input_place();
            hold.hold_pointer_epoch = self.pointer_epoch.load(Ordering::Acquire);
        }
        hold.holding = true;
    }

    /// Replay held events before allowing fresh physical input through.
    ///
    /// Replays have their own native marker: they reach the engine but must never enter this buffer again. Never hold the
    /// mutex across SendInput, which calls the hook synchronously on another thread.
    fn release_input(&self) -> Result<i32, WindowsBackendError> {
        if lock(&self.hold).deferred_action.is_some() {
            return Ok(0);
        }
        let mut count = 0;
        let outcome = loop {
            let item = {
                let mut guard = lock(&self.hold);
                let hold = &mut *guard;
                if hold.deferred_action.is_some() {
                    // A replayed second Enter starts the next transaction. Let its preceding key-ups reach the worker,
                    // retaining all following text until that action is completed.
                    let index = hold
                        .held
                        .iter()
                        .position(|item| !item.pressed && hold.action_prior_keys.contains(item.virtual_key));
                    match index.and_then(|index| hold.held.remove(index)) {
                        Some(item) => item,
                        None => break Ok(count),
                    }
                } else if let Some(item) = hold.held.pop_front() {
                    item
                } else {
                    hold.holding = false;
                    break Ok(count);
                }
            };
            let packet = item.virtual_key == VK_PACKET;
            let sent = self.send_exact(&[NativeInput {
                pressed: item.pressed,
                virtual_key: if packet { 0 } else { item.virtual_key },
                scan_code: item.scan_code,
                extended: item.extended,
                synthetic: false,
                replayed: true,
                unicode: packet,
                foreign: item.foreign,
            }]);
            if let Err(error) = sent {
                break Err(error);
            }
            count += 1;
        };
        // An OS injection failure must never leave the keyboard captured.
        let mut hold = lock(&self.hold);
        if hold.deferred_action.is_none() {
            hold.holding = false;
        }
        outcome
    }

    /// Deliver a withheld Enter/Tab once, before releasing subsequent input.
    fn complete_action(&self, deliver: bool) -> Result<i32, WindowsBackendError> {
        let (action, hold_pointer_epoch, hold_window) = {
            let hold = lock(&self.hold);
            (hold.deferred_action, hold.hold_pointer_epoch, hold.hold_window)
        };
        let Some(action) = action else {
            return Ok(0);
        };
        let delivered = if !deliver {
            Ok(())
        } else if self.input_place_changed(hold_pointer_epoch, hold_window) {
            Err(failed("Enter/Tab не передан: место ввода изменилось"))
        } else {
            self.send_exact(&[true, false].map(|pressed| NativeInput {
                virtual_key: action.virtual_key,
                scan_code: action.scan_code,
                extended: action.extended,
                ..NativeInput::new(pressed)
            }))
        };
        {
            let mut hold = lock(&self.hold);
            hold.deferred_action = None;
            hold.action_prior_keys.clear();
        }
        // In the Python this runs in a `finally`: its failure takes the place of the one above.
        self.release_input()?;
        delivered?;
        Ok(if deliver { COMPLETED_ACTION_EVENT_COUNT } else { 0 })
    }

    /// How long ago input the hooks never got was made, once that is certain; None otherwise.
    ///
    /// A program in front that reads the keyboard as raw input and has a keyboard hook of its own keeps every keyboard hook
    /// of the system silent, its own too, while GetLastInputInfo moves on with each key (Windows 10 19045 and 11 26200; raw
    /// input alone silences nothing): KeySwitch sees none of what is typed there. An input time no hook got is certain to
    /// have passed them once they got a later event, since they get events in order, or once HOOK_SILENCE_MS went by
    /// without one, and it counts only when GetLastInputInfo moved on UNSEEN_INPUT_MIN_TIMES times meanwhile: a lone input
    /// the hooks did not get between keys they got is no program keeping keys away.
    fn take_unseen_input(&self) -> Result<Option<i64>, WindowsBackendError> {
        let seen = self.api.hook_input_times();
        let last = self.api.last_input_time()?;
        let newest = seen.last().copied();
        let mut unseen = lock(&self.unseen);
        if let Some(waiting) = unseen.unseen_input
            && !seen.contains(&waiting)
        {
            if !seen.contains(&last) && newest.is_none_or(|newest| later(last, newest)) {
                unseen.unseen_times.insert(last);
            }
            let waited = (self.api.tick_count() - waiting) & DWORD_MASK;
            if newest.is_some_and(|newest| later(newest, waiting)) || waited >= HOOK_SILENCE_MS {
                unseen.unseen_input = None;
                if unseen.unseen_times.len() < UNSEEN_INPUT_MIN_TIMES {
                    return Ok(None);
                }
                unseen.unseen_reported = Some(waiting);
                return Ok(Some(waited));
            }
            return Ok(None);
        }
        let reported = unseen.unseen_reported;
        let missed = newest.is_some_and(|newest| {
            !seen.contains(&last) && later(last, newest) && reported.is_none_or(|reported| later(last, reported))
        });
        unseen.unseen_input = missed.then_some(last);
        unseen.unseen_times = BTreeSet::from([last]);
        Ok(None)
    }

    fn handle_native(&self, native: NativeKeyEvent) -> bool {
        if native.virtual_key == 0 || native.injected || native.replayed {
            return self.answer_native(native);
        }
        // A typed key: how long after its key time the hook ran, and how long the answer took. A key another program
        // injected (TeamViewer types what it receives on the computer it controls) was pressed at no time the hook can
        // know: only its answer is timed.
        let started = self.clock.perf_counter();
        let late = (self.api.tick_count() - native.timestamp) & DWORD_MASK;
        if !native.foreign && late <= DWORD_MASK >> 1 {
            // Noted before the key goes on: the engine must not act on the word as this key reaches it.
            self.input_delay.note_wait(late);
        }
        let answer = self.answer_native(native);
        let callback = ((self.clock.perf_counter() - started) * MILLISECONDS_PER_SECOND).round_ties_even() as i64;
        if native.foreign {
            self.input_delay.record_foreign(callback);
        } else if late <= DWORD_MASK >> 1 {
            // A key time ahead of the clock (a program stamping its own keys) says nothing.
            self.input_delay.record(late, callback);
        }
        answer
    }

    fn answer_native(&self, native: NativeKeyEvent) -> bool {
        if native.virtual_key == 0 {
            self.pointer_epoch.fetch_add(1, Ordering::AcqRel);
            if let Some(listener) = self.listener.get() {
                listener(KeyEvent {
                    pressed: true,
                    key_name: "Pointer".to_owned(),
                    characters: vec![String::new(), String::new()],
                    group: POINTER_EVENT_GROUP,
                    timestamp: native.timestamp,
                    ..KeyEvent::default()
                });
            }
            return false;
        }
        if native.injected && native.virtual_key == VK_MENU_MASK {
            // KeySwitch's own mask key (mask_menu) is no key of the user's.
            return false;
        }
        if !native.injected && !native.replayed {
            let consumed_scan_code = lock(&self.hook).consumed_keys.contains(native.scan_code);
            let mut hold = lock(&self.hold);
            let prior_release = !native.pressed && hold.action_prior_keys.contains(native.virtual_key);
            let action_key = hold.deferred_action.is_some() && consumed_scan_code;
            if hold.holding && !prior_release && !action_key {
                hold.held.push_back(native);
                return true;
            }
        }
        if native.virtual_key != VK_PACKET {
            // A packet is a character, not a key: nothing is held down by it.
            if !native.injected && !native.replayed && !native.foreign {
                self.recheck_modifiers(native);
            }
            self.track_key_state(native);
        }
        let state = self.normalized_state();
        // The layouts are known before the hook starts. Were they not, the Python's callback would raise, and the key
        // would pass; so it does here.
        let Ok((english, russian)) = self.layouts() else {
            return false;
        };
        let characters = vec![
            self.api.translate_key(native.virtual_key, native.scan_code, state, english),
            self.api.translate_key(native.virtual_key, native.scan_code, state, russian),
        ];
        let Ok(group) = self.current_group() else {
            return false;
        };
        let character = usize::try_from(group).ok().and_then(|index| characters.get(index)).cloned().unwrap_or_default();
        let mut event = KeyEvent {
            pressed: native.pressed,
            keycode: native.scan_code,
            key_name: key_name(native.virtual_key),
            character,
            characters,
            group,
            state,
            timestamp: native.timestamp,
            synthetic: native.injected,
            deferred: false,
            foreign: native.foreign,
        };
        let repeated_answer = !event.synthetic && event.pressed && lock(&self.hook).consumed_keys.contains(event.keycode);
        let consumed = self.consumes(&event);
        if consumed != KeyDisposition::Pass && event.pressed && !event.synthetic && self.alt_or_super_down() {
            self.mask_menu();
        }
        if consumed == KeyDisposition::Defer {
            let prior_keys = lock(&self.hook).pressed.clone();
            // One step, where the Python has three: an engine's release_input cannot run between the hold and the action.
            let mut hold = lock(&self.hold);
            self.hold_input_in(&mut hold);
            hold.deferred_action = Some(native);
            hold.action_prior_keys = prior_keys;
            drop(hold);
            event.deferred = true;
        }
        if !repeated_answer && let Some(listener) = self.listener.get() {
            listener(event);
        }
        consumed != KeyDisposition::Pass
    }

    fn alt_or_super_down(&self) -> bool {
        let hook = lock(&self.hook);
        hook.pressed.intersects(&ALT_KEYS) || hook.pressed.intersects(&SUPER_KEYS)
    }

    /// Forget a modifier whose release went where the hook does not reach.
    ///
    /// A UAC prompt, the Ctrl+Alt+Del screen or an elevated window takes the key-up of a modifier held when it appeared,
    /// and the hook never sees it: every later letter would carry Ctrl and pass for a shortcut until that key was pressed
    /// again. So when a key comes after a pause or in another window, each modifier still in the books is asked of the
    /// keyboard. Only then: the key being answered is not in the keyboard state yet, and asking at every key would race
    /// the user's own chord.
    fn recheck_modifiers(&self, native: NativeKeyEvent) {
        let window = self.api.foreground_window();
        let mut hook = lock(&self.hook);
        let (previous_time, previous_window) = hook.last_typed;
        hook.last_typed = (native.timestamp, window);
        let is_held = |key: u32| key != native.virtual_key && hook.pressed.contains(key);
        if !MODIFIER_KEYS.into_iter().any(is_held)
            || (window == previous_window && (native.timestamp - previous_time) & DWORD_MASK < MODIFIER_RECHECK_GAP_MS)
        {
            return;
        }
        let mut released = String::new();
        // MODIFIER_KEYS is in ascending order: the Python sorts the keys it names.
        for key in MODIFIER_KEYS {
            if key != native.virtual_key && hook.pressed.contains(key) && !self.api.key_down(key) {
                hook.pressed.remove(key);
                if !released.is_empty() {
                    released.push_str(", ");
                }
                released.push_str(&key_name(key));
            }
        }
        drop(hook);
        if !released.is_empty() {
            log::info(LOGGER, &format!("Отпускание модификаторов не дошло до перехватчика, они забыты: {released}"));
        }
    }

    fn track_key_state(&self, native: NativeKeyEvent) {
        let mut hook = lock(&self.hook);
        if native.pressed {
            if native.virtual_key == VK_CAPITAL && !hook.pressed.contains(native.virtual_key) {
                self.caps_lock.fetch_xor(true, Ordering::Relaxed);
            }
            hook.pressed.insert(native.virtual_key);
            return;
        }
        hook.pressed.remove(native.virtual_key);
        drop(hook);
        let mut hold = lock(&self.hold);
        if hold.action_prior_keys.contains(native.virtual_key)
            && hold.held.iter().any(|item| item.pressed && item.virtual_key == native.virtual_key)
        {
            // An auto-repeat after Enter belongs to the held next input. Its key-up also has to follow that replay.
            hold.held.push_back(NativeKeyEvent { replayed: false, ..native });
        }
        hold.action_prior_keys.remove(native.virtual_key);
    }

    fn normalized_state(&self) -> u32 {
        let mut state = if self.caps_lock.load(Ordering::Relaxed) { LOCK_MASK } else { 0 };
        let hook = lock(&self.hook);
        if hook.pressed.intersects(&SHIFT_KEYS) {
            state |= SHIFT_MASK;
        }
        if hook.pressed.intersects(&CONTROL_KEYS) {
            state |= CONTROL_MASK;
        }
        if hook.pressed.intersects(&ALT_KEYS) {
            state |= ALT_MASK;
        }
        if hook.pressed.intersects(&SUPER_KEYS) {
            state |= SUPER_MASK;
        }
        state
    }

    /// Replace the word and return how many held keys were typed again.
    ///
    /// `late` are keys the user typed after the word but before this call: their characters already follow the word on
    /// screen, so they are deleted with it and typed again after the replacement, in the new layout. Keys arriving during
    /// the injection are held by the hook (see `hold_input`) and typed again last. The last `kept_tail` strokes are a word
    /// that stays as it was: they, the boundary and the late keys are typed in the source layout, and the layout stays
    /// there.
    fn inject_correction(&self, injection: &Injection<'_>) -> Result<i32, WindowsBackendError> {
        let result = self._inject_correction(injection);
        // In the Python this runs in a `finally`: its failure takes the place of the outcome above.
        self.release_input()?;
        result
    }

    fn _inject_correction(&self, injection: &Injection<'_>) -> Result<i32, WindowsBackendError> {
        let Injection { strokes, target_group, boundary, source_group, late, trailing, kept_tail, erased } = *injection;
        self.layouts()?;
        if !valid_group(target_group) {
            return Err(failed(format!("Неизвестная группа раскладки {target_group}")));
        }
        let literal: Vec<&KeyEvent> = trailing.iter().chain(boundary).collect();
        let rendered_source_group =
            source_group.unwrap_or_else(|| strokes.first().map_or(target_group, |stroke| stroke.group));
        if !valid_group(rendered_source_group) {
            return Err(failed(format!("Неизвестная исходная группа раскладки {rendered_source_group}")));
        }
        if kept_tail != 0 && (kept_tail >= strokes.len() || source_group.is_none()) {
            return Err(failed("Некорректная граница сохраняемого слова; замена отменена"));
        }
        let final_group = if kept_tail != 0 { rendered_source_group } else { target_group };
        let delete_count = erased.unwrap_or(strokes.len()) + literal.len() + late.len();
        let mut delete_inputs = Vec::with_capacity(delete_count * 2);
        for _ in 0..delete_count {
            delete_inputs.extend([NativeInput::key(true, VK_BACK), NativeInput::key(false, VK_BACK)]);
        }
        let (converted, kept) = strokes.split_at(strokes.len() - kept_tail);
        let replay_inputs = self.strokes_inputs(converted.iter(), true, Some(target_group));
        let kept_inputs = self.strokes_inputs(kept.iter(), true, Some(final_group));
        let boundary_inputs = self.strokes_inputs(literal.iter().copied(), true, None);
        // Typed again as the user's own input: the engine must see these keys as the start of the next word, not as its
        // own injection.
        let late_inputs = self.strokes_inputs(late.iter(), false, Some(final_group));
        let preserve_boundary_layout = literal.iter().any(|stroke| stroke.character_for(target_group) != stroke.character);
        let literal_group = literal.first().map_or(rendered_source_group, |stroke| stroke.group);
        if !valid_group(literal_group) || literal.iter().any(|stroke| stroke.group != literal_group) {
            return Err(failed("Некорректная раскладка пунктуации; замена отменена"));
        }
        let mut late_deleted = false;
        let mut late_typed = false;
        let mut failure: Option<WindowsBackendError> = None;
        let held_count = {
            let _inject = lock(&self.inject_lock);
            let typed = (|| -> Result<(), WindowsBackendError> {
                // The layout is switched first so that the deletion and the replacement travel in one SendInput call:
                // Windows keeps the events of a single call together, but lets the user's keys slip in between separate
                // calls.
                let switched = self._switch_group(target_group)?;
                if let Some((hold_pointer_epoch, hold_window)) = self.holding_place()
                    && self.input_place_changed(hold_pointer_epoch, hold_window)
                {
                    return Err(failed("Место ввода изменилось до замены; текст не изменён"));
                }
                if switched {
                    // The keys of the new layout wait for the window's text service (WINDOWS_LAYOUT_SETTLE_SECONDS); the
                    // user's keys are held meanwhile (hold_input).
                    self.send_exact(&delete_inputs)?;
                    late_deleted = true;
                    self.clock.sleep(WINDOWS_LAYOUT_SETTLE_SECONDS);
                }
                let mut batch = Vec::new();
                if !switched {
                    batch.extend_from_slice(&delete_inputs);
                }
                batch.extend_from_slice(&replay_inputs);
                if !preserve_boundary_layout && kept_tail == 0 {
                    batch.extend_from_slice(&boundary_inputs);
                }
                self.send_exact(&batch)?;
                late_deleted = true;
                if kept_tail != 0 {
                    // The kept word is typed in its own layout, and the text goes on there.
                    self._switch_group(final_group)?;
                    self.clock.sleep(WINDOWS_LAYOUT_SETTLE_SECONDS);
                    let mut batch = kept_inputs.clone();
                    batch.extend_from_slice(&boundary_inputs);
                    self.send_exact(&batch)?;
                } else if preserve_boundary_layout {
                    self._switch_group(literal_group)?;
                    self.clock.sleep(WINDOWS_LAYOUT_SETTLE_SECONDS);
                    self.send_exact(&boundary_inputs)?;
                    self._switch_group(target_group)?;
                }
                // A partial send is not an all-or-nothing failure; retrying this whole batch could duplicate an already
                // delivered prefix.
                late_typed = true;
                self.send_exact(&late_inputs)
            })();
            if let Err(error) = typed {
                failure = Some(error);
            }
            let restore: &[NativeInput] = if late_deleted && !late_typed { &late_inputs } else { &[] };
            match self.send_exact(restore).and_then(|()| self.release_input()) {
                Ok(count) => count,
                Err(error) => {
                    // The primary failure explains more than a failed restore.
                    failure.get_or_insert(error);
                    0
                }
            }
        };
        match failure {
            Some(error) => Err(error),
            None => Ok(held_count),
        }
    }

    fn strokes_inputs<'a>(
        &self,
        strokes: impl Iterator<Item = &'a KeyEvent>,
        synthetic: bool,
        group: Option<i32>,
    ) -> Vec<NativeInput> {
        strokes.flat_map(|stroke| self.stroke_inputs(stroke, synthetic, group)).collect()
    }

    fn stroke_inputs(&self, stroke: &KeyEvent, synthetic: bool, group: Option<i32>) -> Vec<NativeInput> {
        let marks = |input: NativeInput| NativeInput { synthetic, replayed: !synthetic, ..input };
        let mut result = Vec::with_capacity(4);
        let rendered_group = group.unwrap_or(stroke.group);
        let mut shifted = stroke.shift();
        if pystr::isalpha(stroke.character_for(rendered_group)) {
            shifted ^= stroke.caps_lock() != self.caps_lock.load(Ordering::Relaxed);
        }
        if shifted {
            result.push(marks(NativeInput::key(true, VK_SHIFT)));
        }
        result.push(marks(NativeInput::scan(true, stroke.keycode)));
        result.push(marks(NativeInput::scan(false, stroke.keycode)));
        if shifted {
            result.push(marks(NativeInput::key(false, VK_SHIFT)));
        }
        result
    }

    fn send_exact(&self, inputs: &[NativeInput]) -> Result<(), WindowsBackendError> {
        if inputs.is_empty() {
            return Ok(());
        }
        let sent = self.api.send_inputs(inputs)?;
        if sent != inputs.len() {
            return Err(failed(format!(
                "SendInput отправил {sent} из {} событий; результат текста неизвестен; возможно, целевое приложение запущено с правами администратора",
                inputs.len()
            )));
        }
        Ok(())
    }

    /// The selected text: through UI Automation where the field offers it, through the clipboard where it does not.
    ///
    /// A console keeps out of both: typing there goes to the command line rather than over the selection.
    fn read_selection(&self, _application: &str, window: i64) -> Result<SelectedText, SelectionError> {
        let foreground = self.api.foreground_window();
        let clipboard = self.api.selection_api();
        if let Some(clipboard) = clipboard
            && CONSOLE_WINDOW_CLASSES.contains(&clipboard.window_class(foreground).as_str())
        {
            return Err(refused(RefusalReason::Terminal));
        }
        let field = if window != 0 { window } else { foreground };
        let clipboard = {
            let mut slot = lock(&self.selection_reader);
            let reader = self.uia_reader(&mut slot);
            if let Some(selected) = Self::uia_selection(reader, field)? {
                return Ok(selected);
            }
            let Some(clipboard) = clipboard else {
                return Err(refused(RefusalReason::Unsupported));
            };
            // Copying a selection that is not in a field would type the result into whatever has the keys: a web page
            // takes letters for its own shortcuts. A text caret, or an edit field UI Automation names, says it is a field.
            if !(clipboard.text_caret(foreground) || Self::uia_editable(reader, field)) {
                return Err(refused(RefusalReason::NoCaret));
            }
            clipboard
        };
        self.copied_selection(clipboard)
    }

    fn uia_reader<'a>(&self, slot: &'a mut Option<Box<dyn SelectionReader>>) -> Option<&'a dyn SelectionReader> {
        if slot.is_none() {
            match (self.selection_reader_factory)() {
                Ok(reader) => *slot = Some(reader),
                Err(_) => {
                    // Without UI Automation (a refused COM call) the clipboard is the way left.
                    log::info(LOGGER, "UI Automation недоступна для чтения выделения, выделение копируется");
                    return None;
                }
            }
        }
        slot.as_deref()
    }

    fn uia_selection(reader: Option<&dyn SelectionReader>, window: i64) -> Result<Option<SelectedText>, SelectionRefused> {
        let Some(reader) = reader else {
            return Ok(None);
        };
        match reader.read_selection(window) {
            Ok(selected) => Ok(selected),
            Err(SelectionError::Refused(refusal)) => Err(refusal),
            // A provider that fails tells no more than one that is missing.
            Err(SelectionError::Failed(_)) => Ok(None),
        }
    }

    fn uia_editable(reader: Option<&dyn SelectionReader>, window: i64) -> bool {
        reader.is_some_and(|reader| reader.focused_editable(window).unwrap_or(false))
    }

    /// Ask the clipboard until it answers or `deadline` passes: another program may hold it open for a moment.
    fn clipboard_call<T>(
        &self,
        mut call: impl FnMut() -> Result<Option<T>, WindowsBackendError>,
        deadline: f64,
    ) -> Result<Option<T>, WindowsBackendError> {
        loop {
            let answer = call()?;
            if answer.is_some() || self.clock.monotonic() >= deadline {
                return Ok(answer);
            }
            self.clock.sleep(SELECTION_CLIPBOARD_POLL_SECONDS);
        }
    }

    /// Give the clipboard back: asks until `write_clipboard` takes it or a fresh deadline passes; whether it did.
    fn put_back_clipboard(
        &self,
        clipboard: &dyn WindowsSelectionAPI,
        saved: &ClipboardContents,
    ) -> Result<bool, WindowsBackendError> {
        let deadline = self.clock.monotonic() + SELECTION_CLIPBOARD_TIMEOUT_SECONDS;
        let answer = self.clipboard_call(|| Ok(clipboard.write_clipboard(saved)?.then_some(())), deadline)?;
        Ok(answer.is_some())
    }

    /// The clipboard as it is, for a hotkey to put back: refused when it is busy or cannot be kept.
    fn borrow_clipboard(
        &self,
        clipboard: &dyn WindowsSelectionAPI,
        deadline: f64,
    ) -> Result<ClipboardContents, SelectionError> {
        let saved = self.clipboard_call(|| Ok(clipboard.read_clipboard(CLIPBOARD_SNAPSHOT_MAX_BYTES)), deadline)?;
        let Some(saved) = saved else {
            return Err(refused(RefusalReason::ClipboardBusy));
        };
        if !saved.complete {
            return Err(refused(RefusalReason::ClipboardUnrestorable));
        }
        Ok(saved)
    }

    /// Copy the selection with Ctrl+Insert and give the clipboard back what it held.
    ///
    /// Ctrl+Insert is the copy of Windows besides Ctrl+C ("Ctrl+C (or Ctrl+Insert): Copy the selected item", Keyboard
    /// shortcuts in Windows, Microsoft Support), and unlike Ctrl+C it is no interrupt: in a terminal program the console
    /// classes do not catch (mintty, PuTTY, a remote session) Ctrl+C with nothing selected stops the running program. The
    /// user's clipboard is kept before the copy, and the copy is waited for by the clipboard's sequence number: no change
    /// in time means nothing was selected (or the program does not copy with Ctrl+Insert), and the clipboard is then
    /// untouched. A clipboard that cannot be put back as it was is not borrowed at all. One that could not be put back
    /// after the copy is an error the status line reports, since the user's data is no longer on it.
    fn copied_selection(&self, clipboard: &dyn WindowsSelectionAPI) -> Result<SelectedText, SelectionError> {
        let deadline = self.clock.monotonic() + SELECTION_CLIPBOARD_TIMEOUT_SECONDS;
        let saved = self.borrow_clipboard(clipboard, deadline)?;
        let before = clipboard.clipboard_sequence();
        // Insert is an extended key: without the flag it is the keypad's 0, which types a digit under Num Lock.
        self.send_exact(&[
            NativeInput::key(true, VK_CONTROL),
            NativeInput::extended_key(true, VK_INSERT),
            NativeInput::extended_key(false, VK_INSERT),
            NativeInput::key(false, VK_CONTROL),
        ])?;
        while clipboard.clipboard_sequence() == before {
            if self.clock.monotonic() >= deadline {
                return Err(refused(RefusalReason::NoSelection));
            }
            self.clock.sleep(SELECTION_CLIPBOARD_POLL_SECONDS);
        }
        let text = self.clipboard_call(|| Ok(clipboard.clipboard_text(SELECTION_COMMAND_MAX_CHARACTERS + 1)), deadline)?;
        if !self.put_back_clipboard(clipboard, &saved)? {
            return Err(failed("Буфер обмена не удалось вернуть: в нём осталось скопированное выделение").into());
        }
        let Some(text) = text else {
            return Err(refused(RefusalReason::ClipboardBusy));
        };
        Ok(SelectedText { text, source: SOURCE_CLIPBOARD.to_owned() })
    }

    /// Type `replacement` over the selection as Unicode characters, then select it again where that is exact.
    ///
    /// Characters typed with KEYEVENTF_UNICODE need no layout, so every character the selection may hold is typed as it
    /// is; the layout moves to `final_group` afterwards. Shift+Left selects the result back, so that another press of a
    /// selection hotkey acts on it. A field that loses characters typed in a row (SLOW_INPUT_FIELD_CLASSES) gets the result
    /// pasted instead, where UI Automation can show it selected (pasted_selection).
    fn replace_selection(
        &self,
        selected: &SelectedText,
        replacement: &str,
        final_group: Option<i32>,
    ) -> Result<bool, SelectionError> {
        let clipboard = self.api.selection_api();
        let field = self.input_place();
        let mut typed = Vec::new();
        for unit in replacement.encode_utf16() {
            let unit = u32::from(unit);
            typed.push(NativeInput { scan_code: unit, unicode: true, ..NativeInput::new(true) });
            typed.push(NativeInput { scan_code: unit, unicode: true, ..NativeInput::new(false) });
        }
        let steps = caret_steps(replacement);
        let mut reselect = Vec::new();
        if let Some(steps) = steps {
            reselect.push(NativeInput::key(true, VK_SHIFT));
            for _ in 0..steps {
                reselect.push(NativeInput::extended_key(true, VK_LEFT));
                reselect.push(NativeInput::extended_key(false, VK_LEFT));
            }
            reselect.push(NativeInput::key(false, VK_SHIFT));
        }
        let _inject = lock(&self.inject_lock);
        if let Some((hold_pointer_epoch, hold_window)) = self.holding_place()
            && self.input_place_changed(hold_pointer_epoch, hold_window)
        {
            return Err(refused(RefusalReason::FocusChanged));
        }
        let pasted_into = clipboard.filter(|clipboard| {
            steps.is_some()
                && selected.source == SOURCE_UIA
                && SLOW_INPUT_FIELD_CLASSES.contains(&clipboard.window_class(field).as_str())
        });
        let reselected = if let Some(clipboard) = pasted_into {
            self.pasted_selection(clipboard, field, replacement, &reselect)?
        } else {
            // One SendInput call keeps the events together: the user's keys cannot land among them.
            typed.extend_from_slice(&reselect);
            self.send_exact(&typed)?;
            steps.is_some()
        };
        if let Some(final_group) = final_group {
            self._switch_group(final_group)?;
        }
        Ok(reselected)
    }

    /// Paste `replacement` with Ctrl+V and select it back, then give the clipboard back what it held.
    ///
    /// The field takes the text when it handles Ctrl+V, at a moment nobody announces: the user's clipboard comes back once
    /// UI Automation shows the result selected, or after SELECTION_PASTE_CONFIRM_SECONDS, and only while it still holds the
    /// replacement (anything copied meanwhile is the user's). The replacement and the restored data are both kept out of
    /// the clipboard history (write_clipboard). A clipboard that cannot be put back as it was is not borrowed at all, and
    /// one that could not be put back is an error the status line reports. Says whether the result was seen selected.
    fn pasted_selection(
        &self,
        clipboard: &dyn WindowsSelectionAPI,
        field: i64,
        replacement: &str,
        reselect: &[NativeInput],
    ) -> Result<bool, SelectionError> {
        let deadline = self.clock.monotonic() + SELECTION_CLIPBOARD_TIMEOUT_SECONDS;
        let saved = self.borrow_clipboard(clipboard, deadline)?;
        let mut text_bytes = Vec::with_capacity((replacement.len() + 1) * 2);
        for unit in replacement.encode_utf16().chain([0]) {
            text_bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let ours = ClipboardContents { formats: vec![(CF_UNICODETEXT, text_bytes)], complete: true };
        if self.clipboard_call(|| Ok(clipboard.write_clipboard(&ours)?.then_some(())), deadline)?.is_none() {
            return Err(refused(RefusalReason::ClipboardBusy));
        }
        let sequence = clipboard.clipboard_sequence();
        let shown = (|| -> Result<bool, WindowsBackendError> {
            let mut batch = vec![
                NativeInput::key(true, VK_CONTROL),
                NativeInput::key(true, VK_V),
                NativeInput::key(false, VK_V),
                NativeInput::key(false, VK_CONTROL),
            ];
            batch.extend_from_slice(reselect);
            self.send_exact(&batch)?;
            let mut slot = lock(&self.selection_reader);
            let reader = self.uia_reader(&mut slot);
            let confirm = self.clock.monotonic() + SELECTION_PASTE_CONFIRM_SECONDS;
            let mut shown = false;
            while !shown && self.clock.monotonic() < confirm {
                self.clock.sleep(SELECTION_CLIPBOARD_POLL_SECONDS);
                let result = Self::uia_selection(reader, field).unwrap_or(None);
                shown = result.is_some_and(|result| result.text == replacement);
            }
            Ok(shown)
        })();
        // In the Python this runs in a `finally`: a failure here takes the place of one above.
        let restored = clipboard.clipboard_sequence() != sequence || self.put_back_clipboard(clipboard, &saved)?;
        let shown = shown?;
        if !restored {
            return Err(failed("Буфер обмена не удалось вернуть: в нём остался вставленный текст").into());
        }
        Ok(shown)
    }

    /// Switch the window's layout to `group` and wait for it to say so; whether the layout changed.
    fn _switch_group(&self, group: i32) -> Result<bool, WindowsBackendError> {
        let (english, russian) = self.layouts()?;
        let layout = match group {
            0 => english,
            1 => russian,
            _ => return Err(failed("tuple index out of range")),
        };
        if self.group_for_layout(self.api.foreground_layout())? == group {
            return Ok(false);
        }
        if !self.api.request_layout(layout) {
            return Err(failed("Окно отклонило запрос смены раскладки"));
        }
        let deadline = self.clock.monotonic() + WINDOWS_LAYOUT_SWITCH_TIMEOUT_SECONDS;
        while self.clock.monotonic() < deadline {
            if self.group_for_layout(self.api.foreground_layout())? == group {
                return Ok(true);
            }
            self.clock.sleep(LAYOUT_SWITCH_POLL_SECONDS);
        }
        Err(BackendError::LayoutSwitchUnconfirmed { message: "Приложение не подтвердило смену раскладки".to_owned(), group })
    }
}

fn send_mask(api: &dyn WindowsAPI, mask: &[NativeInput]) {
    if let Err(error) = api.send_inputs(mask) {
        // The menu may open on the release; nothing else depends on the mask.
        log::info_with_error(LOGGER, "Клавиша-маска для Alt не отправлена", &error);
    }
}

#[cfg(test)]
#[path = "windows_backend_tests.rs"]
mod tests;
