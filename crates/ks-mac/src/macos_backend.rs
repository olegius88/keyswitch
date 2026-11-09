//! Observe global macOS keyboard events and inject physical corrections.
//!
//! The module holds the whole decision half of the macOS backend and reaches the system only through `MacAPI`, so the
//! behaviour below is exercised on any platform with a substitute. `macos_native` supplies the real implementation.
//!
//! Two things differ from the other platforms and shape what follows.
//!
//! A Quartz event tap placed as an active filter may delete the event it is given, which Windows cannot do: there a key
//! already on its way to the window can only be raced, never withheld. Here a withheld key is genuinely withheld, and the
//! backend puts it back itself by posting it again.
//!
//! The system switches off a tap whose callback answers too slowly and says so with an event of its own. A backend that
//! ignores that message keeps a dead tap and stops seeing the keyboard without any error, so the message is handled like
//! any other event and the tap is switched back on.

use ks_cast::To as _;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

use ks_backend::constants::{
    ALT_MASK, COMPLETED_ACTION_EVENT_COUNT, CONTROL_MASK, LAYOUT_GROUP_COUNT, LOCK_MASK, POINTER_EVENT_GROUP, SHIFT_MASK, SUPER_MASK,
};
use ks_backend::{
    BackendError, BackendProbe, FocusInfo, InputBackend, Injection, KeyDisposition, KeyEvent, KeyFilter, KeyListener, RefusalReason,
    ScreenAnchor, SelectedText, SelectionError, SelectionRefused,
};
use ks_config::selection_commands::caret_steps;
use ks_text::pystr;

use crate::constants::{
    CLIPBOARD_SNAPSHOT_MAX_BYTES, CYRILLIC_CODEPOINT_RANGE, EVENT_SCROLL_WHEEL, EVENT_TAP_DISABLED_BY_TIMEOUT, EVENT_TAP_DISABLED_BY_USER_INPUT,
    KEYBOARD_LISTENER_START_TIMEOUT_SECONDS, KEYBOARD_LISTENER_STOP_TIMEOUT_SECONDS, LAYOUT_SWITCH_POLL_SECONDS,
    LAYOUT_SWITCH_TIMEOUT_SECONDS, MAC_ALT_KEYS, MAC_CONTROL_KEYS, MAC_DEVICE_FLAGS_MASK, MAC_MODIFIER_DEVICE_FLAGS,
    MAC_MODIFIER_SHARED_FLAGS, MAC_SCRIPT_PROBE_KEYCODE, MAC_SHIFT_KEYS, MAC_SUPER_KEYS, MAC_VK_ANSI_V, MAC_VK_BACKSPACE,
    MAC_VK_CAPS_LOCK, MAC_VK_COMMAND, MAC_VK_CONTROL, MAC_VK_DOWN_ARROW, MAC_VK_END, MAC_VK_ESCAPE, MAC_VK_FORWARD_DELETE,
    MAC_VK_FUNCTION, MAC_VK_HELP, MAC_VK_HOME, MAC_VK_KEYPAD_ENTER, MAC_VK_LEFT_ARROW, MAC_VK_OPTION, MAC_VK_PAGE_DOWN,
    MAC_VK_PAGE_UP, MAC_VK_RETURN, MAC_VK_RIGHT_ARROW, MAC_VK_RIGHT_COMMAND, MAC_VK_RIGHT_CONTROL, MAC_VK_RIGHT_OPTION,
    MAC_VK_RIGHT_SHIFT, MAC_VK_SHIFT, MAC_VK_SPACE, MAC_VK_TAB, MAC_VK_UP_ARROW, MACOS_PASTE_RESTORE_DELAY_SECONDS,
};
use crate::constants::{
    HEXADECIMAL, MAC_VK_F1, MAC_VK_F2, MAC_VK_F3, MAC_VK_F4, MAC_VK_F5, MAC_VK_F6, MAC_VK_F7, MAC_VK_F8, MAC_VK_F9, MAC_VK_F10,
    MAC_VK_F11, MAC_VK_F12, MAC_VK_F13, MAC_VK_F14, MAC_VK_F15, MAC_VK_F16, MAC_VK_F17, MAC_VK_F18, MAC_VK_F19, MAC_VK_F20, UNNAMED_KEY_CODE_DIGITS, UNNAMED_KEY_PREFIX,
};
use crate::macos_context::{MacSelectionAccess, SelectionAccess, SelectionAfter, selection_after};

pub type MacBackendError = BackendError;

fn failed(message: impl Into<String>) -> MacBackendError {
    BackendError::Failed(message.into())
}

fn refused(reason: RefusalReason) -> SelectionError {
    SelectionRefused::new(reason).into()
}

/// The engine speaks X11 key names on every platform; the Windows backend translates into the same vocabulary.
const KEY_NAMES: [(u32, &str); 46] = [
    (MAC_VK_RETURN, "Return"),
    (MAC_VK_KEYPAD_ENTER, "KP_Enter"),
    (MAC_VK_TAB, "Tab"),
    (MAC_VK_SPACE, "space"),
    (MAC_VK_BACKSPACE, "BackSpace"),
    (MAC_VK_FORWARD_DELETE, "Delete"),
    (MAC_VK_ESCAPE, "Escape"),
    (MAC_VK_CAPS_LOCK, "Caps_Lock"),
    (MAC_VK_SHIFT, "Shift_L"),
    (MAC_VK_RIGHT_SHIFT, "Shift_R"),
    (MAC_VK_CONTROL, "Control_L"),
    (MAC_VK_RIGHT_CONTROL, "Control_R"),
    (MAC_VK_OPTION, "Alt_L"),
    (MAC_VK_RIGHT_OPTION, "Alt_R"),
    (MAC_VK_COMMAND, "Super_L"),
    (MAC_VK_RIGHT_COMMAND, "Super_R"),
    (MAC_VK_FUNCTION, "Function"),
    (MAC_VK_HELP, "Insert"),
    (MAC_VK_HOME, "Home"),
    (MAC_VK_END, "End"),
    (MAC_VK_PAGE_UP, "Page_Up"),
    (MAC_VK_PAGE_DOWN, "Page_Down"),
    (MAC_VK_LEFT_ARROW, "Left"),
    (MAC_VK_RIGHT_ARROW, "Right"),
    (MAC_VK_UP_ARROW, "Up"),
    (MAC_VK_DOWN_ARROW, "Down"),
    (MAC_VK_F1, "F1"),
    (MAC_VK_F2, "F2"),
    (MAC_VK_F3, "F3"),
    (MAC_VK_F4, "F4"),
    (MAC_VK_F5, "F5"),
    (MAC_VK_F6, "F6"),
    (MAC_VK_F7, "F7"),
    (MAC_VK_F8, "F8"),
    (MAC_VK_F9, "F9"),
    (MAC_VK_F10, "F10"),
    (MAC_VK_F11, "F11"),
    (MAC_VK_F12, "F12"),
    (MAC_VK_F13, "F13"),
    (MAC_VK_F14, "F14"),
    (MAC_VK_F15, "F15"),
    (MAC_VK_F16, "F16"),
    (MAC_VK_F17, "F17"),
    (MAC_VK_F18, "F18"),
    (MAC_VK_F19, "F19"),
    (MAC_VK_F20, "F20"),
];

/// A pointer event carries no key; the engine recognises it by this name.
pub const POINTER_KEY_NAME: &str = "Pointer";

pub const PERMISSION_MISSING_MESSAGE: &str =
    "Нет разрешения на перехват клавиатуры: включите KeySwitch в разделе «Конфиденциальность и безопасность» → «Универсальный доступ»";

const SESSION_TYPE: &str = "quartz";
const DISPLAY_NAME: &str = "macOS window server";
const TAP_NAME: &str = "CGEventTap";
const POST_NAME: &str = "CGEventPost";

/// One keyboard event as the tap saw it.
///
/// `event_type` carries the tap's own messages, such as the one announcing that the system switched the tap off;
/// `keycode` is meaningless then.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NativeKeyEvent {
    pub pressed: bool,
    pub keycode: u32,
    pub timestamp: i64,
    pub injected: bool,
    pub replayed: bool,
    pub event_type: u32,
    pub pointer: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NativeInput {
    pub pressed: bool,
    pub keycode: u32,
    pub replayed: bool,
    /// The key goes out with the Shift flag on it. A Shift posted on its own does not reach the letters after it:
    /// `Ghbdtn` was retyped as `привет`.
    pub shift: bool,
    /// The same for Command: the Cmd+V of a selection hotkey's paste.
    pub command: bool,
}

impl NativeInput {
    #[must_use]
    pub const fn new(pressed: bool, keycode: u32) -> Self {
        NativeInput { pressed, keycode, replayed: false, shift: false, command: false }
    }
}

/// The pasteboard as a selection hotkey found it: each item's types and their bytes, in order.
///
/// `complete` is false when a type gives no data (a promise its owner does not keep) or the pasteboard holds more than the
/// hotkeys keep: such a pasteboard is not borrowed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasteboardContents {
    pub items: Vec<Vec<(String, Vec<u8>)>>,
    pub complete: bool,
}

/// The pasteboard calls of the selection hotkeys' paste (`macos_native::NativeMacAPI`).
pub trait MacPasteboardAPI {
    fn pasteboard_change_count(&self) -> i64;
    fn read_pasteboard(&self, limit_bytes: usize) -> PasteboardContents;
    fn write_pasteboard(&self, contents: &PasteboardContents) -> bool;
    /// Put `text` on the pasteboard as its only content; its change count then, or None if it did not take it.
    fn put_text(&self, text: &str) -> Option<i64>;
}

/// Answers on the tap's thread whether the window must not get the key.
pub type TapListener = Arc<dyn Fn(NativeKeyEvent) -> bool + Send + Sync>;
/// Called on the tap's thread once the tap is in place.
pub type TapReady = Box<dyn FnOnce() + Send>;

/// Everything the backend needs from macOS, and nothing else.
pub trait MacAPI: Send + Sync {
    fn input_sources(&self) -> Result<Vec<String>, MacBackendError>;
    fn current_input_source(&self) -> String;
    fn select_input_source(&self, identifier: &str) -> bool;
    fn translate_key(&self, keycode: u32, state: u32, source: &str) -> String;
    /// How many of the events the system took, in order.
    fn post_inputs(&self, inputs: &[NativeInput]) -> usize;
    fn active_application(&self) -> String;
    fn focused_window(&self) -> i64;
    fn window_process_id(&self, window: i64) -> i32;
    fn current_process_id(&self) -> i32;
    fn input_anchor(&self) -> Option<ScreenAnchor>;
    fn caps_lock_enabled(&self) -> bool;
    fn accessibility_trusted(&self, prompt: bool) -> bool;
    fn open_accessibility_settings(&self) -> bool;
    fn layout_follows_window(&self) -> bool;
    /// Runs the tap on this thread until `stop_event_tap`; `ready` is called once it is in place.
    fn run_event_tap(&self, listener: TapListener, ready: TapReady) -> Result<(), MacBackendError>;
    fn enable_event_tap(&self);
    fn stop_event_tap(&self);
    /// The pasteboard of the selection hotkeys' paste, where the system has one.
    fn pasteboard(&self) -> Option<&dyn MacPasteboardAPI>;
}

/// Whether a flags-changed event leaves its modifier key down.
///
/// Shift, Control, Option and Command arrive as kCGEventFlagsChanged, not as key-down and key-up, so the state is read
/// from the event's flags: the bit of that very key where the event carries side-specific bits, the modifier's shared bit
/// where it does not.
///
/// Caps Lock is never down. macOS sends one flags change per toggle and none when the key comes up, and its flag holds the
/// lock rather than the key: read as a held modifier, an engaged lock kept every correction waiting for a release that
/// comes only when the lock is turned off.
#[must_use]
pub fn modifier_pressed(keycode: u32, flags: u64) -> bool {
    let device = MAC_MODIFIER_DEVICE_FLAGS.iter().find(|(key, _)| *key == keycode).map(|(_, flag)| *flag);
    if let Some(device) = device.filter(|_| flags & MAC_DEVICE_FLAGS_MASK != 0) {
        return flags & device != 0;
    }
    MAC_MODIFIER_SHARED_FLAGS.iter().find(|(key, _)| *key == keycode).is_some_and(|(_, shared)| flags & shared != 0)
}

#[must_use]
pub fn is_cyrillic(text: &str) -> bool {
    text.chars().next().is_some_and(|first| (CYRILLIC_CODEPOINT_RANGE.0..=CYRILLIC_CODEPOINT_RANGE.1).contains(&first.to::<u32>()))
}

/// Name the English and the Russian layout, in the engine's group order.
///
/// The choice is made from what a key actually produces rather than from the identifier, because a layout's name is not a
/// promise about its script: `ABC`, `US` and `British` are all English, and a Russian layout may be `Russian`, `RussianWin`
/// or a third-party one.
pub fn select_source_pair(sources: &[String], translate: &dyn Fn(u32, u32, &str) -> String) -> Result<(String, String), MacBackendError> {
    let mut english = String::new();
    let mut russian = String::new();
    let mut seen: Vec<&String> = Vec::new();
    for source in sources {
        if seen.contains(&source) {
            continue;
        }
        seen.push(source);
        let character = translate(MAC_SCRIPT_PROBE_KEYCODE, 0, source);
        if english.is_empty() && pystr::isascii(&character) && pystr::isalpha(&character) {
            english.clone_from(source);
        } else if russian.is_empty() && is_cyrillic(&character) {
            russian.clone_from(source);
        }
    }
    if english.is_empty() || russian.is_empty() {
        return Err(failed("В macOS должны быть включены английская и русская раскладки"));
    }
    Ok((english, russian))
}

/// Name a key by its position, falling back to the letter it types.
///
/// macOS has no per-key names, so a letter key is named after the character it produces in the English layout, which is
/// how the engine's own key names for letters and digits are spelled.
#[must_use]
pub fn key_name(keycode: u32, character: &str) -> String {
    if let Some((_, name)) = KEY_NAMES.iter().find(|(key, _)| *key == keycode) {
        return (*name).to_owned();
    }
    if character.chars().count() == 1 && pystr::isascii(character) && pystr::isalnum(character) {
        return character.to_lowercase();
    }
    format!("VK_{keycode:02X}")
}

/// Whether the keyboard has a key of this name: `name` is casefolded, as a hotkey of the settings holds it
/// (`Hotkey::problem`). A key `key_name` has no name for is `VK_` and its code in two hexadecimal digits.
#[must_use]
pub fn key_name_exists(name: &str) -> bool {
    KEY_NAMES.iter().any(|(_, known)| pystr::casefold(known) == name)
        || (name.chars().count() == 1 && pystr::isascii(name) && pystr::isalnum(name))
        || name.strip_prefix(UNNAMED_KEY_PREFIX).is_some_and(|code| code.len() == UNNAMED_KEY_CODE_DIGITS && u32::from_str_radix(code, HEXADECIMAL).is_ok())
}

fn lock<T: ?Sized>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The monotonic clock and the sleep of the module, replaced in tests.
pub trait Clock: Send + Sync {
    fn monotonic(&self) -> f64;
    fn sleep(&self, seconds: f64);
}

pub struct SystemClock {
    started: Instant,
}

impl Default for SystemClock {
    fn default() -> Self {
        SystemClock { started: Instant::now() }
    }
}

impl Clock for SystemClock {
    fn monotonic(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    fn sleep(&self, seconds: f64) {
        thread::sleep(Duration::from_secs_f64(seconds));
    }
}

/// Makes the accessibility side of the selection hotkeys on first use.
pub type SelectionAccessFactory = Box<dyn Fn() -> Box<dyn SelectionAccess> + Send + Sync>;

/// What the backend needs apart from the API; tests replace these.
pub struct BackendParts {
    pub clock: Arc<dyn Clock>,
    pub selection_access_factory: SelectionAccessFactory,
    pub listener_start_timeout_seconds: f64,
}

impl Default for BackendParts {
    fn default() -> Self {
        BackendParts {
            clock: Arc::new(SystemClock::default()),
            selection_access_factory: Box::new(default_selection_access),
            listener_start_timeout_seconds: KEYBOARD_LISTENER_START_TIMEOUT_SECONDS,
        }
    }
}

#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))] // Only the native access, checked on the Mac by the live test.
fn default_selection_access() -> Box<dyn SelectionAccess> {
    Box::new(MacSelectionAccess::new(crate::macos_native::NativeAccessibilityAPI::new()))
}

/// Off macOS nothing answers: every read finds no focused element.
#[cfg(not(target_os = "macos"))]
fn default_selection_access() -> Box<dyn SelectionAccess> {
    Box::new(MacSelectionAccess::new(NoAccessibility))
}

#[cfg(not(target_os = "macos"))]
struct NoAccessibility;

#[cfg(not(target_os = "macos"))]
impl crate::macos_context::AccessibilityAPI for NoAccessibility {
    fn focused_element(&self) -> usize {
        0
    }
    fn string_attribute(&self, _element: usize, _name: &str) -> Option<String> {
        None
    }
    fn range_attribute(&self, _element: usize, _name: &str) -> Option<(i64, i64)> {
        None
    }
    fn release(&self, _element: usize) {}
}

#[cfg(not(target_os = "macos"))]
impl crate::macos_context::SelectionAccessibilityAPI for NoAccessibility {
    fn attribute_settable(&self, _element: usize, _name: &str) -> bool {
        false
    }
    fn set_string_attribute(&self, _element: usize, _name: &str, _value: &str) -> bool {
        false
    }
}

/// The keys down and the keys this backend is withholding; touched by the tap's thread only.
#[derive(Default)]
struct TapState {
    pressed: Vec<u32>,
    consumed_keys: Vec<u32>,
}

/// While a correction is being injected the user's own keys are kept back here and posted again afterwards, so they can
/// neither land between the backspaces and the replacement nor be lost. The lock is never held across a post, which
/// calls the tap back on another thread.
#[derive(Default)]
struct HoldState {
    holding: bool,
    held: VecDeque<NativeKeyEvent>,
    hold_pointer_epoch: u64,
    hold_window: i64,
    deferred_action: Option<NativeKeyEvent>,
    action_prior_keys: Vec<u32>,
}

struct TapThread {
    id: ThreadId,
    /// Disconnects when the thread ends: `thread.join(timeout=...)`, which std's handle does not offer.
    finished: mpsc::Receiver<()>,
}

struct Inner {
    api: Arc<dyn MacAPI>,
    clock: Arc<dyn Clock>,
    sources: Mutex<Option<(String, String)>>,
    listener: Mutex<Option<KeyListener>>,
    key_filter: Mutex<Option<KeyFilter>>,
    thread: Mutex<Option<TapThread>>,
    running: AtomicBool,
    start_error: Mutex<Option<MacBackendError>>,
    listener_start_timeout_seconds: f64,
    tap: Mutex<TapState>,
    hold: Mutex<HoldState>,
    /// Counts pointer presses; the engine's thread compares the count.
    pointer_epoch: AtomicU64,
    /// Held by the engine's thread across posts and layout waits. The tap's thread never takes it.
    inject_lock: Mutex<()>,
    tap_revivals: AtomicU64,
    /// The accessibility side of the selection hotkeys, made on first use (`read_selection`).
    selection_access: Mutex<Option<Arc<dyn SelectionAccess>>>,
    selection_access_factory: SelectionAccessFactory,
}

/// Observe global Quartz keyboard events and inject physical corrections.
pub struct MacBackend {
    inner: Arc<Inner>,
}

impl MacBackend {
    /// The backend over `api`, or over the real macOS for None.
    pub fn new(api: Option<Arc<dyn MacAPI>>) -> Result<Self, MacBackendError> {
        match api {
            Some(api) => Ok(Self::with_api(api)),
            None => Self::native_api().map(Self::with_api),
        }
    }

    #[cfg(target_os = "macos")]
    #[cfg_attr(target_os = "macos", expect(clippy::unnecessary_wraps, reason = "other systems answer with the error"))]
    fn native_api() -> Result<Arc<dyn MacAPI>, MacBackendError> {
        Ok(Arc::new(crate::macos_native::NativeMacAPI::new()))
    }

    #[cfg(not(target_os = "macos"))]
    fn native_api() -> Result<Arc<dyn MacAPI>, MacBackendError> {
        Err(failed("Quartz доступен только в macOS"))
    }

    pub fn with_api(api: Arc<dyn MacAPI>) -> Self {
        Self::with_parts(api, BackendParts::default())
    }

    pub fn with_parts(api: Arc<dyn MacAPI>, parts: BackendParts) -> Self {
        MacBackend {
            inner: Arc::new(Inner {
                api,
                clock: parts.clock,
                sources: Mutex::new(None),
                listener: Mutex::new(None),
                key_filter: Mutex::new(None),
                thread: Mutex::new(None),
                running: AtomicBool::new(false),
                start_error: Mutex::new(None),
                listener_start_timeout_seconds: parts.listener_start_timeout_seconds,
                tap: Mutex::new(TapState::default()),
                hold: Mutex::new(HoldState::default()),
                pointer_epoch: AtomicU64::new(0),
                inject_lock: Mutex::new(()),
                tap_revivals: AtomicU64::new(0),
                selection_access: Mutex::new(None),
                selection_access_factory: parts.selection_access_factory,
            }),
        }
    }

    #[must_use]
    pub fn running(&self) -> bool {
        self.inner.running.load(Ordering::Acquire)
    }

    /// How often the system switched the tap off and it was revived.
    #[must_use]
    pub fn tap_revivals(&self) -> u64 {
        self.inner.tap_revivals.load(Ordering::Relaxed)
    }

    pub fn sources(&self) -> Result<(String, String), MacBackendError> {
        self.inner.sources()
    }

    /// The tap's answer for one event: whether the window must not get it.
    #[must_use]
    pub fn handle_native(&self, native: NativeKeyEvent) -> bool {
        self.inner.handle_native(native)
    }

    /// `_listener` set without the tap, for what drives `handle_native` itself.
    pub fn set_listener(&self, listener: Option<KeyListener>) {
        *lock(&self.inner.listener) = listener;
    }

    /// Whether macOS currently lets the program watch the keyboard.
    #[must_use]
    pub fn permission_granted(&self) -> bool {
        self.inner.api.accessibility_trusted(false)
    }

    /// Ask for the permission and show where it is given.
    ///
    /// macOS grants nothing on request: it shows its own dialog, and the pane is opened as well because that dialog appears
    /// only once per program.
    #[must_use]
    pub fn request_permission(&self) -> bool {
        let granted = self.inner.api.accessibility_trusted(true);
        if !granted {
            self.inner.api.open_accessibility_settings();
        }
        granted
    }

    /// Show the pane where the permission is given, without asking for it again: the menu's way there while the program
    /// waits.
    #[must_use]
    pub fn open_permission_settings(&self) -> bool {
        self.inner.api.open_accessibility_settings()
    }
}

impl InputBackend for MacBackend {
    fn start(&self, listener: KeyListener) -> Result<(), BackendError> {
        self.inner.start(listener)
    }

    /// Decide, inside the tap, which keys must not reach the window.
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
        self.inner.stop();
    }

    fn close(&self) {
        self.inner.stop();
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
        let api = &self.inner.api;
        let window = api.focused_window();
        if window == 0 {
            return None;
        }
        let own = api.window_process_id(window) == api.current_process_id();
        Some(FocusInfo { window, own, isolated_layout: api.layout_follows_window() })
    }

    fn probe(&self) -> BackendProbe {
        self.inner.probe()
    }

    fn inject_correction(&self, injection: Injection<'_>) -> Result<i32, BackendError> {
        self.inner.inject_correction(&injection)
    }

    /// The tap can keep a key from the window, so `KeySwitch`'s own hotkeys stay its own (`engine.consumes_key`): `TextEdit` typed
    /// Control+Option+Z as an invisible U+001A, and the undo deleted that in place of the word's last letter.
    fn swallows_hotkeys(&self) -> bool {
        true
    }

    fn supports_selection(&self) -> bool {
        true
    }

    fn read_selection(&self, _application: &str, _window: i64) -> Result<SelectedText, SelectionError> {
        self.inner.selection().read()
    }

    fn replace_selection(&self, selected: &SelectedText, replacement: &str, final_group: Option<i32>) -> Result<bool, SelectionError> {
        self.inner.replace_selection(selected, replacement, final_group)
    }

    fn input_anchor(&self) -> Option<ScreenAnchor> {
        self.inner.api.input_anchor()
    }

    /// macOS gives the focus back to the program a prompt covered on its own, because `KeySwitch` runs without a place in the
    /// dock and never becomes the active program, so there is nothing here to undo.
    fn restore_window(&self, _window: Option<i64>) -> bool {
        false
    }

    /// For the same reason nothing is needed to show a window without taking the focus.
    fn keep_window_inactive(&self, _window: i64) -> bool {
        false
    }
}

impl Inner {
    fn sources(&self) -> Result<(String, String), MacBackendError> {
        if let Some(pair) = lock(&self.sources).clone() {
            return Ok(pair);
        }
        let listed = self.api.input_sources()?;
        let api = self.api.clone();
        let pair = select_source_pair(&listed, &move |keycode, state, source| api.translate_key(keycode, state, source))?;
        *lock(&self.sources) = Some(pair.clone());
        Ok(pair)
    }

    fn group_for_source(&self, source: &str) -> Result<i32, MacBackendError> {
        let (english, russian) = self.sources()?;
        Ok(i32::from(source != english && source == russian))
    }

    fn current_group(&self) -> Result<i32, MacBackendError> {
        self.group_for_source(&self.api.current_input_source())
    }

    fn probe(&self) -> BackendProbe {
        let unavailable = |tap: &str, post: &str, layouts: &str, error: String| BackendProbe {
            available: false,
            session_type: SESSION_TYPE.to_owned(),
            display: DISPLAY_NAME.to_owned(),
            record_version: tap.to_owned(),
            xtest_version: post.to_owned(),
            xkb_version: layouts.to_owned(),
            current_group: -1,
            error,
        };
        if !self.api.accessibility_trusted(false) {
            return unavailable(TAP_NAME, POST_NAME, "—", PERMISSION_MISSING_MESSAGE.to_owned());
        }
        let answer = (|| -> Result<BackendProbe, MacBackendError> {
            let (english, russian) = self.sources()?;
            let group = self.group_for_source(&self.api.current_input_source())?;
            Ok(BackendProbe {
                available: true,
                session_type: SESSION_TYPE.to_owned(),
                display: DISPLAY_NAME.to_owned(),
                record_version: TAP_NAME.to_owned(),
                xtest_version: POST_NAME.to_owned(),
                xkb_version: format!("{english},{russian}"),
                current_group: group,
                error: String::new(),
            })
        })();
        answer.unwrap_or_else(|error| unavailable("—", "—", "—", error.to_string()))
    }

    /// A thread the system refused: nothing runs.
    fn spawn_failed(&self, error: &std::io::Error) -> MacBackendError {
        self.running.store(false, Ordering::Release);
        failed(error.to_string())
    }

    fn start(self: &Arc<Self>, listener: KeyListener) -> Result<(), MacBackendError> {
        if self.running.load(Ordering::Acquire) {
            return Ok(());
        }
        self.sources()?;
        *lock(&self.listener) = Some(listener);
        *lock(&self.start_error) = None;
        self.running.store(true, Ordering::Release);
        let (ready_sender, ready) = mpsc::channel::<()>();
        let (finished_sender, finished) = mpsc::channel::<()>();
        let inner = Arc::clone(self);
        let spawned = thread::Builder::new().name("keyswitch-tap".to_owned()).spawn(move || {
            // Dropped as the thread ends, whatever ended it.
            let _finished = finished_sender;
            inner.tap_loop(ready_sender);
        });
        let handle = spawned.map_err(|error| self.spawn_failed(&error))?;
        *lock(&self.thread) = Some(TapThread { id: handle.thread().id(), finished });
        // Not joined: as a daemon thread of the Python, it ends with the process.
        drop(handle);
        let _ = ready.recv_timeout(Duration::from_secs_f64(self.listener_start_timeout_seconds));
        if let Some(error) = self.startup_error() {
            self.running.store(false, Ordering::Release);
            return Err(error);
        }
        Ok(())
    }

    /// Read the failure through a call, so its assignment cannot be assumed.
    fn startup_error(&self) -> Option<MacBackendError> {
        lock(&self.start_error).clone()
    }

    fn tap_loop(self: &Arc<Self>, ready: mpsc::Sender<()>) {
        let handling = Arc::clone(self);
        let listener: TapListener = Arc::new(move |native| handling.handle_native(native));
        let signal = ready.clone();
        let mark_ready: TapReady = Box::new(move || {
            let _ = signal.send(());
        });
        if let Err(error) = self.api.run_event_tap(listener, mark_ready) {
            *lock(&self.start_error) = Some(error);
        }
        self.running.store(false, Ordering::Release);
        let _ = ready.send(());
    }

    fn stop(&self) {
        if !self.running.load(Ordering::Acquire) {
            return;
        }
        self.api.stop_event_tap();
        let tap_thread = lock(&self.thread).take();
        if let Some(tap_thread) = tap_thread
            && tap_thread.id != thread::current().id() {
                // Disconnected or timed out: either way the wait is over, and the thread is left to end alone.
                let _ = tap_thread.finished.recv_timeout(Duration::from_secs_f64(KEYBOARD_LISTENER_STOP_TIMEOUT_SECONDS));
            }
        self.running.store(false, Ordering::Release);
    }

    fn switch_group(&self, group: i32) -> Result<(), MacBackendError> {
        let (english, russian) = self.sources()?;
        let source = match group {
            0 => english,
            1 => russian,
            _ => return Err(failed(format!("Неизвестная группа раскладки: {group}"))),
        };
        if !self.api.select_input_source(&source) {
            return Err(failed("macOS отказалась переключить раскладку"));
        }
        Ok(())
    }

    fn hold_input(&self) {
        let mut hold = lock(&self.hold);
        if !hold.holding {
            hold.hold_window = self.api.focused_window();
            hold.hold_pointer_epoch = self.pointer_epoch.load(Ordering::Acquire);
        }
        hold.holding = true;
    }

    /// Post held events again before letting fresh physical input through.
    ///
    /// A reposted event carries its own mark: it reaches the engine but must never enter this buffer again. The lock is
    /// never held across a post, which calls the tap back on another thread.
    fn release_input(&self) -> Result<i32, MacBackendError> {
        if lock(&self.hold).deferred_action.is_some() {
            return Ok(0);
        }
        let mut count = 0;
        let mut outcome = Ok(());
        loop {
            let item = {
                let mut hold = lock(&self.hold);
                if hold.deferred_action.is_some() {
                    // A reposted second Enter starts the next transaction. Let its preceding key-ups reach the worker,
                    // retaining all following text until that action is completed.
                    let index = hold.held.iter().position(|item| !item.pressed && hold.action_prior_keys.contains(&item.keycode));
                    match index.and_then(|index| hold.held.remove(index)) {
                        Some(item) => item,
                        None => break,
                    }
                } else if let Some(item) = hold.held.pop_front() {
                    item
                } else {
                    hold.holding = false;
                    break;
                }
            };
            let input = NativeInput { replayed: true, ..NativeInput::new(item.pressed, item.keycode) };
            if let Err(error) = self.post_exact(&[input]) {
                outcome = Err(error);
                break;
            }
            count += 1;
        }
        // A failed post must never leave the keyboard captured.
        let mut hold = lock(&self.hold);
        if hold.deferred_action.is_none() {
            hold.holding = false;
        }
        outcome.map(|()| count)
    }

    /// Deliver a withheld Enter/Tab once, before releasing subsequent input.
    fn complete_action(&self, deliver: bool) -> Result<i32, MacBackendError> {
        let Some(action) = lock(&self.hold).deferred_action else {
            return Ok(0);
        };
        let mut outcome = Ok(());
        if deliver {
            let (epoch, window) = {
                let hold = lock(&self.hold);
                (hold.hold_pointer_epoch, hold.hold_window)
            };
            outcome = if self.pointer_epoch.load(Ordering::Acquire) != epoch || self.api.focused_window() != window {
                Err(failed("Enter/Tab не передан: место ввода изменилось"))
            } else {
                self.post_exact(&[NativeInput::new(true, action.keycode), NativeInput::new(false, action.keycode)])
            };
        }
        {
            let mut hold = lock(&self.hold);
            hold.deferred_action = None;
            hold.action_prior_keys.clear();
        }
        let released = self.release_input();
        outcome?;
        released?;
        Ok(if deliver { COMPLETED_ACTION_EVENT_COUNT } else { 0 })
    }

    /// Answer the tap: true swallows the event, false lets it through.
    fn handle_native(&self, native: NativeKeyEvent) -> bool {
        if native.event_type == EVENT_TAP_DISABLED_BY_TIMEOUT || native.event_type == EVENT_TAP_DISABLED_BY_USER_INPUT {
            // The system switches off a tap it considers unresponsive. Reviving it here is the difference between a brief
            // stall and a program that has silently stopped seeing the keyboard.
            self.tap_revivals.fetch_add(1, Ordering::Relaxed);
            self.api.enable_event_tap();
            return false;
        }
        if native.event_type == EVENT_SCROLL_WHEEL {
            // The view moves, not the caret, and no correction waits on it (pointer_epoch).
            let listener = lock(&self.listener).clone();
            if let Some(listener) = listener {
                listener(KeyEvent::wheel(native.timestamp));
            }
            return false;
        }
        if native.pointer {
            self.pointer_epoch.fetch_add(1, Ordering::AcqRel);
            let listener = lock(&self.listener).clone();
            if let Some(listener) = listener {
                listener(KeyEvent {
                    pressed: true,
                    key_name: POINTER_KEY_NAME.to_owned(),
                    characters: vec![String::new(), String::new()],
                    group: POINTER_EVENT_GROUP,
                    timestamp: native.timestamp,
                    ..KeyEvent::default()
                });
            }
            return false;
        }
        if !native.injected && !native.replayed {
            let tap = lock(&self.tap);
            let mut hold = lock(&self.hold);
            let prior_release = !native.pressed && hold.action_prior_keys.contains(&native.keycode);
            let action_key = hold.deferred_action.is_some() && tap.consumed_keys.contains(&native.keycode);
            if hold.holding && !prior_release && !action_key {
                hold.held.push_back(native);
                return true;
            }
        }
        {
            let mut tap = lock(&self.tap);
            if native.pressed {
                if !tap.pressed.contains(&native.keycode) {
                    tap.pressed.push(native.keycode);
                }
            } else {
                tap.pressed.retain(|key| *key != native.keycode);
                let mut hold = lock(&self.hold);
                if hold.action_prior_keys.contains(&native.keycode)
                    && hold.held.iter().any(|item| item.pressed && item.keycode == native.keycode)
                {
                    // An auto-repeat after Enter belongs to the held next input. Its key-up also has to follow that repost.
                    hold.held.push_back(NativeKeyEvent { replayed: false, ..native });
                }
                hold.action_prior_keys.retain(|key| *key != native.keycode);
            }
        }
        let Ok((english, russian)) = self.sources() else {
            return false;
        };
        let state = self.normalized_state();
        let characters: Vec<String> = [&english, &russian].iter().map(|source| self.api.translate_key(native.keycode, state, source)).collect();
        let group = self.current_group().unwrap_or(0);
        let character = usize::try_from(group).ok().and_then(|index| characters.get(index)).cloned().unwrap_or_default();
        // The name comes from the key with no modifier held: under Control the letter types a control code (Control+Z is
        // \x1a) and under Option another letter, and Ctrl+Alt+Z must still be z.
        let plain = self.api.translate_key(native.keycode, 0, &english);
        let mut event = KeyEvent {
            pressed: native.pressed,
            keycode: native.keycode,
            key_name: key_name(native.keycode, &plain),
            character,
            characters,
            group,
            state,
            timestamp: native.timestamp,
            synthetic: native.injected,
            deferred: false,
            foreign: false,
        };
        let repeated_answer = !event.synthetic && event.pressed && lock(&self.tap).consumed_keys.contains(&event.keycode);
        let consumed = self.consumes(&event);
        if consumed == KeyDisposition::Defer {
            self.hold_input();
            let pressed = lock(&self.tap).pressed.clone();
            let mut hold = lock(&self.hold);
            hold.deferred_action = Some(native);
            hold.action_prior_keys = pressed;
            event.deferred = true;
        }
        let listener = lock(&self.listener).clone();
        if let Some(listener) = listener.filter(|_| !repeated_answer) {
            listener(event);
        }
        consumed != KeyDisposition::Pass
    }

    /// Ask the filter about a press, and hide the matching release.
    ///
    /// A window that saw no key-down must not receive the key-up either, so the release of a swallowed key is swallowed
    /// with it.
    fn consumes(&self, event: &KeyEvent) -> KeyDisposition {
        if event.synthetic {
            return KeyDisposition::Pass;
        }
        if !event.pressed {
            let mut tap = lock(&self.tap);
            return match tap.consumed_keys.iter().position(|key| *key == event.keycode) {
                Some(index) => {
                    tap.consumed_keys.remove(index);
                    KeyDisposition::Consume
                }
                None => KeyDisposition::Pass,
            };
        }
        if lock(&self.tap).consumed_keys.contains(&event.keycode) {
            return KeyDisposition::Consume;
        }
        let predicate = lock(&self.key_filter).clone();
        let decision = predicate.map_or(KeyDisposition::Pass, |predicate| predicate(event));
        if decision != KeyDisposition::Pass {
            lock(&self.tap).consumed_keys.push(event.keycode);
        }
        decision
    }

    fn normalized_state(&self) -> u32 {
        let mut state = if self.api.caps_lock_enabled() { LOCK_MASK } else { 0 };
        let tap = lock(&self.tap);
        let down = |keys: &[u32]| keys.iter().any(|key| tap.pressed.contains(key));
        if down(&MAC_SHIFT_KEYS) {
            state |= SHIFT_MASK;
        }
        if down(&MAC_CONTROL_KEYS) {
            state |= CONTROL_MASK;
        }
        if down(&MAC_ALT_KEYS) {
            state |= ALT_MASK;
        }
        if down(&MAC_SUPER_KEYS) {
            state |= SUPER_MASK;
        }
        state
    }

    /// Replace the word and return how many held keys were typed again.
    ///
    /// `late` are keys the user typed after the word but before this call: their characters already follow the word on
    /// screen, so they are deleted with it and typed again after the replacement, in the new layout. Keys arriving during
    /// the injection are held by the tap (see `hold_input`) and typed again last. The last `kept_tail` strokes are a word
    /// that stays as it was: they, the boundary and the late keys are typed in the source layout, and the layout stays
    /// there.
    fn inject_correction(&self, injection: &Injection<'_>) -> Result<i32, MacBackendError> {
        let outcome = self.inject(injection);
        let released = self.release_input();
        outcome.and_then(|held| released.map(|_| held))
    }

    fn stroke_inputs(&self, stroke: &KeyEvent, synthetic: bool, group: Option<i32>) -> Vec<NativeInput> {
        let rendered_group = group.unwrap_or(stroke.group);
        let mut shifted = stroke.shift();
        if pystr::isalpha(stroke.character_for(rendered_group)) {
            shifted ^= stroke.caps_lock() != self.api.caps_lock_enabled();
        }
        let replayed = !synthetic;
        let mut result = Vec::with_capacity(4);
        if shifted {
            result.push(NativeInput { replayed, ..NativeInput::new(true, MAC_VK_SHIFT) });
        }
        result.push(NativeInput { replayed, shift: shifted, ..NativeInput::new(true, stroke.keycode) });
        result.push(NativeInput { replayed, shift: shifted, ..NativeInput::new(false, stroke.keycode) });
        if shifted {
            result.push(NativeInput { replayed, ..NativeInput::new(false, MAC_VK_SHIFT) });
        }
        result
    }

    fn inject(&self, injection: &Injection<'_>) -> Result<i32, MacBackendError> {
        self.sources()?;
        let valid = |group: i32| (0..LAYOUT_GROUP_COUNT).contains(&group);
        let target_group = injection.target_group;
        if !valid(target_group) {
            return Err(failed(format!("Неизвестная группа раскладки {target_group}")));
        }
        let strokes = injection.strokes;
        let late = injection.late;
        let literal: Vec<&KeyEvent> = injection.trailing.iter().chain(injection.boundary).collect();
        let rendered_source_group = injection.source_group.unwrap_or_else(|| strokes.first().map_or(target_group, |stroke| stroke.group));
        if !valid(rendered_source_group) {
            return Err(failed(format!("Неизвестная исходная группа раскладки {rendered_source_group}")));
        }
        let kept_tail = injection.kept_tail;
        if kept_tail != 0 && (kept_tail >= strokes.len() || injection.source_group.is_none()) {
            return Err(failed("Некорректная граница сохраняемого слова; замена отменена"));
        }
        let final_group = if kept_tail != 0 { rendered_source_group } else { target_group };
        let delete_count = injection.erased.unwrap_or(strokes.len()) + literal.len() + late.len();
        let delete_inputs: Vec<NativeInput> = (0..delete_count)
            .flat_map(|_| [NativeInput::new(true, MAC_VK_BACKSPACE), NativeInput::new(false, MAC_VK_BACKSPACE)])
            .collect();
        let (converted, kept) = strokes.split_at(strokes.len() - kept_tail);
        let replay_inputs: Vec<NativeInput> = converted.iter().flat_map(|stroke| self.stroke_inputs(stroke, true, Some(target_group))).collect();
        let kept_inputs: Vec<NativeInput> = kept.iter().flat_map(|stroke| self.stroke_inputs(stroke, true, Some(final_group))).collect();
        let boundary_inputs: Vec<NativeInput> = literal.iter().flat_map(|stroke| self.stroke_inputs(stroke, true, None)).collect();
        // Typed again as the user's own input: the engine must see these keys as the start of the next word, not as its own
        // injection.
        let late_inputs: Vec<NativeInput> = late.iter().flat_map(|stroke| self.stroke_inputs(stroke, false, Some(final_group))).collect();
        let preserve_boundary_layout = literal.iter().any(|stroke| stroke.character_for(target_group) != stroke.character);
        let literal_group = literal.first().map_or(rendered_source_group, |stroke| stroke.group);
        if !valid(literal_group) || literal.iter().any(|stroke| stroke.group != literal_group) {
            return Err(failed("Некорректная раскладка пунктуации; замена отменена"));
        }
        let _inject_inner = lock(&self.inject_lock);
        let mut late_deleted = false;
        let mut late_typed = false;
        let mut failure: Option<MacBackendError> = (|| -> Result<(), MacBackendError> {
            self.switch_group_confirmed(target_group)?;
            if self.pointer_or_focus_moved() {
                return Err(failed("Место ввода изменилось до замены; текст не изменён"));
            }
            let first: Vec<NativeInput> = [&delete_inputs[..], &replay_inputs[..]]
                .concat()
                .into_iter()
                .chain(if preserve_boundary_layout || kept_tail != 0 { Vec::new() } else { boundary_inputs.clone() })
                .collect();
            self.post_exact(&first)?;
            late_deleted = true;
            if kept_tail != 0 {
                // The kept word is typed in its own layout, and the text goes on there.
                self.switch_group_confirmed(final_group)?;
                self.post_exact(&[&kept_inputs[..], &boundary_inputs[..]].concat())?;
            } else if preserve_boundary_layout {
                self.switch_group_confirmed(literal_group)?;
                self.post_exact(&boundary_inputs)?;
                self.switch_group_confirmed(target_group)?;
            }
            // A partial post is not an all-or-nothing failure; retrying this whole batch could duplicate an already
            // delivered prefix.
            late_typed = true;
            self.post_exact(&late_inputs)
        })()
        .err();
        let restore: &[NativeInput] = if late_deleted && !late_typed { &late_inputs } else { &[] };
        let mut held_count = 0;
        let released = self.post_exact(restore).and_then(|()| self.release_input());
        match released {
            Ok(count) => held_count = count,
            // The primary failure explains more than a failed restore.
            Err(error) => failure = failure.or(Some(error)),
        }
        failure.map_or(Ok(held_count), Err)
    }

    fn pointer_or_focus_moved(&self) -> bool {
        let (holding, epoch, window) = {
            let hold = lock(&self.hold);
            (hold.holding, hold.hold_pointer_epoch, hold.hold_window)
        };
        holding && (self.pointer_epoch.load(Ordering::Acquire) != epoch || self.api.focused_window() != window)
    }

    fn selection(&self) -> Arc<dyn SelectionAccess> {
        lock(&self.selection_access).get_or_insert_with(|| Arc::from((self.selection_access_factory)())).clone()
    }

    /// Write `replacement` over the selection through the accessibility tree, then select it back.
    ///
    /// A field that keeps the replacement selected needs nothing more; one that leaves the caret right after it gets
    /// Shift+Left as many times as the result has characters. The layout moves to `final_group` afterwards.
    fn replace_selection(&self, selected: &SelectedText, replacement: &str, final_group: Option<i32>) -> Result<bool, SelectionError> {
        let steps = caret_steps(replacement);
        let _inject_inner = lock(&self.inject_lock);
        if self.pointer_or_focus_moved() {
            return Err(refused(RefusalReason::FocusChanged));
        }
        let access = self.selection();
        let where_ = match access.replace(&selected.text, replacement) {
            Ok(after) => after,
            Err(SelectionError::Refused(refusal)) if refusal.reason == RefusalReason::NotSettable => self.pasted(&*access, &selected.text, replacement)?,
            Err(error) => return Err(error),
        };
        let mut reselected = where_ == SelectionAfter::Kept;
        if where_ == SelectionAfter::CaretAfter
            && let Some(steps) = steps {
                let mut inputs = vec![NativeInput::new(true, MAC_VK_SHIFT)];
                for _ in 0..steps {
                    for pressed in [true, false] {
                        inputs.push(NativeInput { shift: true, ..NativeInput::new(pressed, MAC_VK_LEFT_ARROW) });
                    }
                }
                inputs.push(NativeInput::new(false, MAC_VK_SHIFT));
                self.post_exact(&inputs)?;
                reselected = true;
            }
        if let Some(group) = final_group {
            self.switch_group_confirmed(group)?;
        }
        Ok(reselected)
    }

    /// Paste `replacement` over the selection with Cmd+V where the field does not let it be written, and give the pasteboard
    /// back what it held.
    ///
    /// A pasteboard that cannot be put back as it was is not borrowed at all. The user's pasteboard is restored
    /// `MACOS_PASTE_RESTORE_DELAY_SECONDS` after the keys, and only while it still holds the replacement: anything copied
    /// meanwhile is the user's. One that could not be put back is an error the status line reports.
    fn pasted(&self, access: &dyn SelectionAccess, selected: &str, replacement: &str) -> Result<SelectionAfter, SelectionError> {
        let Some(board) = self.api.pasteboard() else {
            return Err(refused(RefusalReason::NotSettable));
        };
        let saved = board.read_pasteboard(CLIPBOARD_SNAPSHOT_MAX_BYTES);
        if !saved.complete {
            return Err(refused(RefusalReason::ClipboardUnrestorable));
        }
        let before = access.selected_range(selected)?;
        let ours = board.put_text(replacement);
        let pasted = if ours.is_none() { Err(failed("macOS не положила текст в буфер обмена")) } else {
            let outcome = self.post_exact(&[
                NativeInput::new(true, MAC_VK_COMMAND),
                NativeInput { command: true, ..NativeInput::new(true, MAC_VK_ANSI_V) },
                NativeInput { command: true, ..NativeInput::new(false, MAC_VK_ANSI_V) },
                NativeInput::new(false, MAC_VK_COMMAND),
            ]);
            if outcome.is_ok() {
                self.clock.sleep(MACOS_PASTE_RESTORE_DELAY_SECONDS);
            }
            outcome
        };
        let restored = ours.is_some_and(|ours| board.pasteboard_change_count() != ours) || board.write_pasteboard(&saved);
        pasted?;
        if !restored {
            return Err(failed("Буфер обмена не удалось вернуть: в нём остался вставленный текст").into());
        }
        Ok(selection_after(before, access.caret_range(), replacement))
    }

    fn switch_group_confirmed(&self, group: i32) -> Result<(), MacBackendError> {
        if self.current_group()? == group {
            return Ok(());
        }
        self.switch_group(group)?;
        let deadline = self.clock.monotonic() + LAYOUT_SWITCH_TIMEOUT_SECONDS;
        while self.clock.monotonic() < deadline {
            if self.current_group()? == group {
                return Ok(());
            }
            self.clock.sleep(LAYOUT_SWITCH_POLL_SECONDS);
        }
        Err(BackendError::LayoutSwitchUnconfirmed { message: "macOS не подтвердила смену раскладки".to_owned(), group })
    }

    fn post_exact(&self, inputs: &[NativeInput]) -> Result<(), MacBackendError> {
        if inputs.is_empty() {
            return Ok(());
        }
        let posted = self.api.post_inputs(inputs);
        if posted != inputs.len() {
            return Err(failed(format!("macOS приняла {posted} из {} событий ввода", inputs.len())));
        }
        Ok(())
    }
}
#[cfg(test)]
#[path = "macos_backend_tests.rs"]
mod tests;
