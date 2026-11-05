//! X11 RECORD/XKB/XTEST backend (the Python's `x11_backend.py`).
//!
//! The Python declares the X libraries with ctypes. Here the protocol is spoken by `x11rb` (see `x11_native`), and this
//! file holds what the backend decides: the threads, the order of the operations, the keys it expects to see again, the
//! characters of a key in each layout group. The server is reached through `X11Api`, so a fake can stand in for it in the
//! tests.
//!
//! Threads: the RECORD thread reads the data connection and calls the listener for every key of the system; the engine's
//! worker injects corrections and holds `inject_lock` across a batch. The expected-key queue has its own small lock, taken
//! by both and never across a call to the server.

use ks_cast::To as _;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ks_backend::constants::{LAYOUT_GROUP_COUNT, LOCK_MASK, SHIFT_MASK};
use ks_backend::selected_text::{RefusalReason, SelectedText, SelectionError, SelectionRefused};
use ks_backend::{BackendError, BackendProbe, FocusInfo, InputBackend, Injection, KeyEvent, KeyFilter, KeyListener, ScreenAnchor};
use ks_config::logsetup;
use ks_config::selection_commands::{KeyChoice, caret_steps, typing_runs};
use ks_text::pystr;
use ks_text::russian_text::{SECONDS, quantity};

use crate::constants::{
    BACKSPACE_KEYSYM, INJECTION_SECONDS_PER_EVENT, KEYBOARD_LISTENER_START_TIMEOUT_SECONDS, KEYBOARD_LISTENER_STOP_TIMEOUT_SECONDS,
    LEFT_KEYSYM, MAX_TRACKED_OWN_WINDOWS, MAX_UNICODE_CODEPOINT, MAX_WINDOW_ANCESTOR_DEPTH, MAX_XKB_GROUPS,
    MIN_INJECTION_DEADLINE_SECONDS, SHIFT_L_KEYSYM, X11_BUTTON_PRESS, X11_KEY_PRESS, X11_KEY_RELEASE, X11_MAX_KEYCODE,
    X11_MIN_KEYCODE, X11_WHEEL_BUTTONS, XKB_GROUP_STATE_MASK, XKB_GROUP_STATE_SHIFT, XRECORD_EVENT_SIZE_BYTES,
    XRECORD_EVENT_TYPE_MASK, XRECORD_FROM_SERVER, XRECORD_START_OF_DATA,
};
use crate::keysym_table::keysym_to_utf32;

const LOGGER: &str = "keyswitch.x11_backend";

/// `X11Error`: a failure of the backend, with the message the log and the status line show.
pub type X11Error = BackendError;

fn failed(message: impl Into<String>) -> X11Error {
    BackendError::Failed(message.into())
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `XkbStateRec`, the two fields the backend reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XkbState {
    /// The effective group.
    pub group: u8,
    pub locked_mods: u8,
}

/// One reply of an enabled RECORD context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordChunk {
    pub category: u8,
    pub client_swapped: bool,
    pub data: Vec<u8>,
}

/// What the backend asks of an X server: one control connection and one data connection, with the extensions it needs.
/// A call that cannot be made says so in its result and never panics.
pub trait X11Api: Send + Sync {
    fn xkb_version(&self) -> String;
    /// `XRecordQueryVersion`.
    fn record_version(&self) -> Result<(u16, u16), String>;
    /// `XTestQueryExtension`: the version of XTEST.
    fn xtest_version(&self) -> Result<(u8, u16), String>;
    /// `XRecordCreateContext` for the key and button presses of all clients.
    fn record_create_context(&self) -> Result<u32, String>;
    /// `XRecordEnableContext` on the data connection: blocks, handing every reply over, until the context is disabled.
    fn record_enable(&self, context: u32, sink: &mut dyn FnMut(RecordChunk)) -> Result<(), String>;
    fn record_disable(&self, context: u32);
    fn record_free(&self, context: u32);
    /// `XSync` of the control connection.
    fn sync(&self);
    fn flush(&self);
    /// `XkbKeycodeToKeysym`: 0 for NoSymbol.
    fn keysym(&self, keycode: u8, group: u8, level: u8) -> u32;
    /// `XkbLookupKeySym` with a core state (modifiers and the group in bits 13-14).
    fn lookup_keysym(&self, keycode: u8, state: u16) -> Option<u32>;
    /// `XKeysymToKeycode`: 0 for none.
    fn keycode_for_keysym(&self, keysym: u32) -> u8;
    /// `XkbGetState` of the core keyboard.
    fn xkb_state(&self) -> Option<XkbState>;
    /// `XkbLockGroup`.
    fn lock_group(&self, group: u8) -> bool;
    /// `XTestFakeKeyEvent` with no delay.
    fn fake_key(&self, pressed: bool, keycode: u8) -> bool;
    /// `XTestGrabControl`.
    fn grab_control(&self, impervious: bool);
    /// `XGetInputFocus`: the window with the focus.
    fn input_focus(&self) -> Option<u32>;
    /// `XQueryPointer` on the root window of the default screen.
    fn pointer(&self) -> Option<(i32, i32)>;
    /// The parent of a window (0 for none).
    fn parent_window(&self, window: u32) -> Option<u32>;
    /// The `_NET_WM_PID` of a window itself; 0 for none or when the atom does not exist.
    fn window_pid(&self, window: u32) -> u32;
    /// `XGetClassHint`: the resource name and class.
    fn class_hint(&self, window: u32) -> Option<(String, String)>;
    fn set_input_focus(&self, window: u32);
    fn move_raise(&self, window: u32, x: i32, y: i32);
}

/// Opens the X server: the control and data connections.
pub type Connector = Box<dyn Fn() -> Result<Arc<dyn X11Api>, X11Error> + Send + Sync>;
/// `window_process_id` as a reader of accessibility needs it.
pub type ProcessForWindow = Arc<dyn Fn(i64) -> u32 + Send + Sync>;

/// The reader of the selection hotkeys (the Python's `SelectionSource`, built on first use).
pub trait SelectionSource: Send {
    fn read_selection(&mut self, application: &str, window: i64) -> Result<SelectedText, SelectionError>;
    /// Writes over the selection last read; None when the field cannot take text itself.
    fn write_selection(&mut self, selected: &SelectedText, replacement: &str) -> Result<Option<bool>, SelectionError>;
}
pub type SelectionFactory = Box<dyn Fn(ProcessForWindow) -> Box<dyn SelectionSource> + Send + Sync>;

/// A flag a thread waits for.
#[derive(Default)]
struct Event {
    set: Mutex<bool>,
    changed: Condvar,
}

impl Event {
    fn set(&self) {
        *lock(&self.set) = true;
        self.changed.notify_all();
    }
    fn clear(&self) {
        *lock(&self.set) = false;
    }
    fn is_set(&self) -> bool {
        *lock(&self.set)
    }
    fn wait(&self, timeout: Duration) -> bool {
        let guard = lock(&self.set);
        let (guard, _) = self.changed.wait_timeout_while(guard, timeout, |set| !*set).unwrap_or_else(PoisonError::into_inner);
        *guard
    }
}

struct Expected {
    queue: VecDeque<(bool, u8)>,
    deadline: Instant,
}

struct Shared {
    group_count: usize,
    connector: Connector,
    selection_factory: Option<SelectionFactory>,
    api: Mutex<Option<Arc<dyn X11Api>>>,
    /// Own-window verdicts by X window id; ids of another client can never collide with ours.
    own_windows: Mutex<HashMap<u32, bool>>,
    context: Mutex<u32>,
    thread: Mutex<Option<(JoinHandle<()>, Arc<Event>)>>,
    listener: Mutex<Option<KeyListener>>,
    running: AtomicBool,
    capture_ready: Event,
    capture_start_finished: Event,
    expected: Mutex<Expected>,
    inject_lock: Mutex<()>,
    xkb_version: Mutex<String>,
    selection_source: Mutex<Option<Box<dyn SelectionSource>>>,
}

/// Observe all core keyboard events and inject deterministic corrections.
pub struct X11Backend {
    shared: Arc<Shared>,
}

impl X11Backend {
    pub fn new(connector: Connector, selection_factory: Option<SelectionFactory>) -> X11Backend {
        X11Backend::with_group_count(LAYOUT_GROUP_COUNT.to::<usize>(), connector, selection_factory)
    }

    pub fn with_group_count(group_count: usize, connector: Connector, selection_factory: Option<SelectionFactory>) -> X11Backend {
        X11Backend {
            shared: Arc::new(Shared {
                group_count: group_count.clamp(LAYOUT_GROUP_COUNT.to::<usize>(), MAX_XKB_GROUPS),
                connector,
                selection_factory,
                api: Mutex::new(None),
                own_windows: Mutex::new(HashMap::new()),
                context: Mutex::new(0),
                thread: Mutex::new(None),
                listener: Mutex::new(None),
                running: AtomicBool::new(false),
                capture_ready: Event::default(),
                capture_start_finished: Event::default(),
                expected: Mutex::new(Expected { queue: VecDeque::new(), deadline: Instant::now() }),
                inject_lock: Mutex::new(()),
                xkb_version: Mutex::new("—".to_owned()),
                selection_source: Mutex::new(None),
            }),
        }
    }

    /// Opens the connections without starting the capture (the Python's `_open`), for a reader of accessibility that only
    /// needs the focused window.
    pub fn open(&self) -> Result<(), X11Error> {
        if self.shared.api().is_some() { Ok(()) } else { self.shared.open().map(|_| ()) }
    }

    pub fn running(&self) -> bool {
        self.shared.running.load(Ordering::SeqCst)
    }

    /// The exact client process of a window, for a reader of accessibility.
    pub fn window_process_id(&self, window: i64) -> u32 {
        self.shared.window_process_id(window)
    }

    /// The same, as a function a reader can keep.
    pub fn process_for_window(&self) -> ProcessForWindow {
        let shared = self.shared.clone();
        Arc::new(move |window| shared.window_process_id(window))
    }

    pub fn position_window(&self, window: i64, x: i32, y: i32) -> bool {
        match (self.shared.api(), u32::try_from(window)) {
            (Some(api), Ok(window)) if window != 0 => {
                api.move_raise(window, x, y);
                api.flush();
                true
            }
            _ => false,
        }
    }

    /// The characters of a key in each group, for a test of the engine's view of the keyboard.
    pub fn character_for_keycode(&self, keycode: u8, group: usize, state: u16) -> String {
        self.shared.api().map_or_else(String::new, |api| self.shared.character_for_keycode(&*api, keycode, group, state))
    }

    pub fn close_backend(&self) {
        self.shared.close();
    }
}

impl Shared {
    fn api(&self) -> Option<Arc<dyn X11Api>> {
        lock(&self.api).clone()
    }

    fn session_type(default: &str) -> String {
        std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| default.to_owned())
    }

    fn open(&self) -> Result<Arc<dyn X11Api>, X11Error> {
        if Self::session_type("x11").to_lowercase() == "wayland" {
            return Err(failed("Активна Wayland-сессия; backend этой версии работает через X11"));
        }
        let api = (self.connector)()?;
        *lock(&self.xkb_version) = api.xkb_version();
        *lock(&self.api) = Some(api.clone());
        Ok(api)
    }

    fn probe(&self) -> BackendProbe {
        let temporary = self.api().is_none();
        let outcome = (|| -> Result<BackendProbe, X11Error> {
            let api = if temporary { self.open()? } else { self.api().ok_or_else(|| failed("X11 backend не запущен"))? };
            let (major, minor) = api.record_version().map_err(|_| failed("Расширение XRecord недоступно"))?;
            let (xtest_major, xtest_minor) = api.xtest_version().map_err(|_| failed("Расширение XTEST недоступно"))?;
            Ok(BackendProbe {
                available: true,
                session_type: Self::session_type("x11"),
                display: std::env::var("DISPLAY").unwrap_or_default(),
                record_version: format!("{major}.{minor}"),
                xtest_version: format!("{xtest_major}.{xtest_minor}"),
                xkb_version: lock(&self.xkb_version).clone(),
                current_group: self.current_group(),
                error: String::new(),
            })
        })();
        let probe = outcome.unwrap_or_else(|error| BackendProbe {
            available: false,
            session_type: Self::session_type("неизвестно"),
            display: std::env::var("DISPLAY").unwrap_or_default(),
            record_version: "—".to_owned(),
            xtest_version: "—".to_owned(),
            xkb_version: "—".to_owned(),
            current_group: -1,
            error: error.to_string(),
        });
        if temporary {
            self.close();
        }
        probe
    }

    fn start(self: &Arc<Self>, listener: KeyListener) -> Result<(), X11Error> {
        if self.running.load(Ordering::SeqCst) {
            return Ok(());
        }
        *lock(&self.listener) = Some(listener);
        let api = match self.api() {
            Some(api) => api,
            None => self.open()?,
        };
        if api.record_version().is_err() {
            self.close();
            return Err(failed("XRecord не поддерживается X-сервером"));
        }
        let previous = std::mem::take(&mut *lock(&self.context));
        if previous != 0 {
            api.record_free(previous);
        }
        let context = match api.record_create_context() {
            Ok(context) if context != 0 => context,
            _ => {
                self.close();
                return Err(failed("Не удалось создать контекст XRecord"));
            }
        };
        *lock(&self.context) = context;
        // XRecord uses a separate data connection. Flush the context creation on the control connection before the data
        // connection enables it.
        api.sync();
        self.capture_ready.clear();
        self.capture_start_finished.clear();
        self.running.store(true, Ordering::SeqCst);
        let done = Arc::new(Event::default());
        let worker = {
            let shared = self.clone();
            let api = api.clone();
            let done = done.clone();
            thread::Builder::new().name("keyswitch-xrecord".to_owned()).spawn(move || {
                shared.record_loop(&*api, context);
                done.set();
            })
        };
        match worker {
            Ok(handle) => *lock(&self.thread) = Some((handle, done)),
            Err(error) => {
                self.running.store(false, Ordering::SeqCst);
                self.close();
                return Err(failed(format!("Не удалось запустить поток XRecord: {error}")));
            }
        }
        if !self.capture_start_finished.wait(Duration::from_secs_f64(KEYBOARD_LISTENER_START_TIMEOUT_SECONDS)) {
            self.close();
            return Err(failed(format!(
                "XRecord не подтвердил запуск за {}",
                quantity(KEYBOARD_LISTENER_START_TIMEOUT_SECONDS, SECONDS)
            )));
        }
        if !self.capture_ready.is_set() {
            self.close();
            return Err(failed("XRecord завершился до подтверждения запуска"));
        }
        Ok(())
    }

    fn record_loop(&self, api: &dyn X11Api, context: u32) {
        let result = api.record_enable(context, &mut |chunk| self.handle_record_data(api, chunk));
        if self.running.load(Ordering::SeqCst) && result.is_err() {
            logsetup::error(LOGGER, "XRecordEnableContext завершился ошибкой");
        }
        self.running.store(false, Ordering::SeqCst);
        self.capture_start_finished.set();
    }

    fn handle_record_data(&self, api: &dyn X11Api, chunk: RecordChunk) {
        if chunk.category == XRECORD_START_OF_DATA {
            self.capture_ready.set();
            self.capture_start_finished.set();
        }
        if chunk.category != XRECORD_FROM_SERVER || chunk.client_swapped || chunk.data.is_empty() {
            return;
        }
        let listener = lock(&self.listener).clone();
        for block in chunk.data.as_chunks::<XRECORD_EVENT_SIZE_BYTES>().0 {
            if let (Some(event), Some(listener)) = (self.decode_event(api, block), listener.as_ref()) {
                listener(event);
            }
        }
    }

    /// `_decode_event` of a 32-byte core event: `=BBHIIIIhhhhHBB`.
    fn decode_event(&self, api: &dyn X11Api, payload: &[u8]) -> Option<KeyEvent> {
        let event_type = payload[0] & XRECORD_EVENT_TYPE_MASK;
        let keycode = payload[1];
        let timestamp = i64::from(u32::from_ne_bytes([payload[4], payload[5], payload[6], payload[7]]));
        let state = u16::from_ne_bytes([payload[28], payload[29]]);
        if event_type == X11_BUTTON_PRESS {
            // For a button event the detail byte is the button: the wheel moves the view, not the caret.
            return if X11_WHEEL_BUTTONS.contains(&keycode) {
                None
            } else {
                Some(KeyEvent {
                    pressed: true,
                    keycode: 0,
                    key_name: "Pointer".to_owned(),
                    character: String::new(),
                    characters: vec![String::new(), String::new()],
                    group: -1,
                    state: 0,
                    timestamp,
                    ..KeyEvent::default()
                })
            };
        }
        if event_type != X11_KEY_PRESS && event_type != X11_KEY_RELEASE {
            return None;
        }
        let pressed = event_type == X11_KEY_PRESS;
        let group = (state >> XKB_GROUP_STATE_SHIFT) & XKB_GROUP_STATE_MASK;
        let characters: Vec<String> =
            (0..self.group_count).map(|candidate| self.character_for_keycode(api, keycode, candidate, state)).collect();
        let mut character = characters.get(usize::from(group)).cloned().unwrap_or_default();
        let key_name = self.key_name(api, keycode);
        if key_name == "Return" {
            character = "\n".to_owned();
        } else if key_name == "Tab" || key_name == "ISO_Left_Tab" {
            character = "\t".to_owned();
        }
        let synthetic = self.consume_expected(pressed, keycode);
        Some(KeyEvent {
            pressed,
            keycode: u32::from(keycode),
            key_name,
            character,
            characters,
            group: i32::from(group),
            state: u32::from(state),
            timestamp,
            synthetic,
            ..KeyEvent::default()
        })
    }

    fn character_for_keycode(&self, api: &dyn X11Api, keycode: u8, group: usize, state: u16) -> String {
        let shift = u32::from(state) & SHIFT_MASK != 0;
        let caps = u32::from(state) & LOCK_MASK != 0;
        let mut keysym = api.keysym(keycode, group.low::<u8>(), u8::from(shift));
        if keysym == 0 {
            // Common keys such as Space often have only group 0 in XKB. Direct indexing returns NoSymbol for RU; the actual
            // key lookup applies the key's group fallback, just as the editor does.
            let lookup_state = (state & !(XKB_GROUP_STATE_MASK << XKB_GROUP_STATE_SHIFT))
                | ((group.low::<u16>() & XKB_GROUP_STATE_MASK) << XKB_GROUP_STATE_SHIFT);
            match api.lookup_keysym(keycode, lookup_state) {
                Some(resolved) => keysym = resolved,
                None => return String::new(),
            }
        }
        let codepoint = keysym_to_utf32(keysym);
        if codepoint == 0 || codepoint > MAX_UNICODE_CODEPOINT {
            return String::new();
        }
        let Some(base) = char::from_u32(codepoint) else { return String::new() };
        let mut character = base.to_string();
        if caps && pystr::isalpha(&character) {
            character = if shift { pystr::lower(&character) } else { pystr::upper(&character) };
        }
        if pystr::isprintable(&character) || character == "\t" || character == "\n" { character } else { String::new() }
    }

    fn key_name(&self, api: &dyn X11Api, keycode: u8) -> String {
        crate::keysym_names::keysym_name(api.keysym(keycode, 0, 0)).unwrap_or_default()
    }

    fn consume_expected(&self, pressed: bool, keycode: u8) -> bool {
        let mut expected = lock(&self.expected);
        if !expected.queue.is_empty() && Instant::now() > expected.deadline {
            expected.queue.clear();
        }
        if expected.queue.front() == Some(&(pressed, keycode)) {
            expected.queue.pop_front();
            return true;
        }
        false
    }

    fn current_group(&self) -> i32 {
        self.api().and_then(|api| api.xkb_state()).map_or(-1, |state| i32::from(state.group))
    }

    fn focused_window(&self) -> Option<FocusInfo> {
        let api = self.api()?;
        let focus = api.input_focus()?;
        if focus == 0 || focus == 1 {
            // None or PointerRoot: nobody has the focus.
            return None;
        }
        Some(FocusInfo { window: i64::from(focus), own: self.window_is_own(&*api, focus), isolated_layout: false })
    }

    fn window_is_own(&self, api: &dyn X11Api, window: u32) -> bool {
        if let Some(cached) = lock(&self.own_windows).get(&window) {
            return *cached;
        }
        let own = Self::process_id_of(api, window) == std::process::id();
        let mut windows = lock(&self.own_windows);
        if windows.len() >= MAX_TRACKED_OWN_WINDOWS {
            windows.clear();
        }
        windows.insert(window, own);
        own
    }

    fn window_process_id(&self, window: i64) -> u32 {
        match (self.api(), u32::try_from(window)) {
            (Some(api), Ok(window)) => Self::process_id_of(&*api, window),
            _ => 0,
        }
    }

    /// `_NET_WM_PID` of the client owning `window` (GTK sets it). Toolkits give the focus to a child window that carries no
    /// properties of its own, so the search walks up a bounded number of parents.
    fn process_id_of(api: &dyn X11Api, window: u32) -> u32 {
        let mut current = window;
        for _ in 0..MAX_WINDOW_ANCESTOR_DEPTH {
            let process_id = api.window_pid(current);
            if process_id != 0 {
                return process_id;
            }
            match api.parent_window(current) {
                Some(parent) if parent != 0 && parent != current => current = parent,
                _ => return 0,
            }
        }
        0
    }

    fn active_application(&self) -> String {
        let Some(api) = self.api() else { return String::new() };
        let Some(mut current) = api.input_focus() else { return String::new() };
        for _ in 0..MAX_WINDOW_ANCESTOR_DEPTH {
            if current == 0 {
                break;
            }
            if let Some((resource_name, class_name)) = api.class_hint(current) {
                return if class_name.is_empty() { resource_name } else { class_name };
            }
            match api.parent_window(current) {
                Some(parent) if parent != 0 && parent != current => current = parent,
                _ => break,
            }
        }
        String::new()
    }

    fn switch_group(&self, group: i32) -> Result<(), X11Error> {
        if group < 0 || group.to::<usize>() >= self.group_count {
            return Err(failed(format!("Неизвестная группа раскладки {group}")));
        }
        let Some(api) = self.api() else { return Err(failed("X11 backend не запущен")) };
        let inject_inner = lock(&self.inject_lock);
        if !api.lock_group(group.low::<u8>()) {
            return Err(failed(format!("Не удалось переключить XKB-группу на {group}")));
        }
        api.flush();
        Ok(())
    }

    fn stop(&self) {
        let context = *lock(&self.context);
        if self.running.load(Ordering::SeqCst)
            && context != 0
            && let Some(api) = self.api()
        {
            self.running.store(false, Ordering::SeqCst);
            api.record_disable(context);
            api.sync();
        }
        let worker = lock(&self.thread).take();
        if let Some((handle, done)) = worker {
            if handle.thread().id() == thread::current().id() {
                return;
            }
            if done.wait(Duration::from_secs_f64(KEYBOARD_LISTENER_STOP_TIMEOUT_SECONDS)) {
                let _ = handle.join();
            }
        }
    }

    fn close(&self) {
        self.stop();
        let context = std::mem::take(&mut *lock(&self.context));
        let api = lock(&self.api).take();
        if let Some(api) = api {
            if context != 0 {
                api.record_free(context);
            }
            api.flush();
        }
    }

    /// The keys that type each character in each layout group, without Caps Lock: plain before shifted, the lowest keycode
    /// first (the main keys come before the keypad).
    fn typing_tables(&self, api: &dyn X11Api) -> Vec<HashMap<char, KeyChoice>> {
        let mut tables: Vec<HashMap<char, KeyChoice>> = vec![HashMap::new(); self.group_count];
        for keycode in X11_MIN_KEYCODE..=X11_MAX_KEYCODE {
            for (group, table) in tables.iter_mut().enumerate() {
                for shifted in [false, true] {
                    let character = self.character_for_keycode(api, keycode, group, if shifted { SHIFT_MASK.low::<u16>() } else { 0 });
                    let mut chars = character.chars();
                    if let (Some(only), None) = (chars.next(), chars.next()) {
                        table.entry(only).or_insert((i64::from(keycode), shifted));
                    }
                }
            }
        }
        tables
    }

    fn fake(api: &dyn X11Api, events: &[(bool, u8)]) -> Result<(), X11Error> {
        for &(pressed, keycode) in events {
            if !api.fake_key(pressed, keycode) {
                return Err(failed(format!("XTest отклонил keycode {keycode}")));
            }
        }
        Ok(())
    }

    fn lock_to(api: &dyn X11Api, group: i32, message: &str) -> Result<(), X11Error> {
        if api.lock_group(group.low::<u8>()) { Ok(()) } else { Err(failed(format!("{message}{group}"))) }
    }

    fn expect(&self, sequences: &[&[(bool, u8)]]) {
        let total: usize = sequences.iter().map(|sequence| sequence.len()).sum();
        let mut expected = lock(&self.expected);
        for sequence in sequences {
            expected.queue.extend(sequence.iter().copied());
        }
        expected.deadline =
            Instant::now() + Duration::from_secs_f64(MIN_INJECTION_DEADLINE_SECONDS.max(total.to::<f64>() * INJECTION_SECONDS_PER_EVENT));
    }

    fn forget_expected(&self) {
        lock(&self.expected).queue.clear();
    }

    fn inject_correction(&self, injection: Injection<'_>) -> Result<i32, X11Error> {
        let Some(api) = self.api() else { return Err(failed("X11 backend не запущен")) };
        let Injection { strokes, target_group, boundary, source_group, late, trailing, kept_tail, erased } = injection;
        if kept_tail != 0 && (kept_tail >= strokes.len() || source_group.is_none()) {
            return Err(failed("Некорректная граница сохраняемого слова; замена отменена"));
        }
        let literal: Vec<&KeyEvent> = trailing.iter().chain(boundary).collect();
        let Some(keyboard_state) = api.xkb_state() else {
            return Err(failed("Не удалось прочитать состояние Caps Lock перед заменой"));
        };
        let caps_lock = u32::from(keyboard_state.locked_mods) & LOCK_MASK != 0;
        let shifted = |stroke: &KeyEvent, group: i32| -> bool {
            stroke.shift() ^ (pystr::isalpha(stroke.character_for(group)) && stroke.caps_lock() != caps_lock)
        };
        let backspace_keycode = api.keycode_for_keysym(BACKSPACE_KEYSYM);
        let shift_keycode = api.keycode_for_keysym(SHIFT_L_KEYSYM);
        if backspace_keycode == 0 || shift_keycode == 0 {
            return Err(failed("X-сервер не вернул keycode для BackSpace/Shift"));
        }
        let tap = |target: &mut Vec<(bool, u8)>, keycode: u8, with_shift: bool| {
            if with_shift {
                target.push((true, shift_keycode));
            }
            target.push((true, keycode));
            target.push((false, keycode));
            if with_shift {
                target.push((false, shift_keycode));
            }
        };
        let mut sequence: Vec<(bool, u8)> = Vec::new();
        let delete_count = erased.unwrap_or(strokes.len()) + literal.len() + late.len();
        for _ in 0..delete_count {
            tap(&mut sequence, backspace_keycode, false);
        }
        let rendered_source_group = match source_group {
            Some(group) => group,
            None => strokes.first().map_or(target_group, |stroke| stroke.group),
        };
        let final_group = if kept_tail != 0 { rendered_source_group } else { target_group };
        let converted = &strokes[..strokes.len() - kept_tail];
        for stroke in converted {
            tap(&mut sequence, stroke.keycode.low::<u8>(), shifted(stroke, target_group));
        }
        // A kept word is typed again in its own layout, after the switch back.
        let mut kept_sequence: Vec<(bool, u8)> = Vec::new();
        for stroke in &strokes[converted.len()..] {
            tap(&mut kept_sequence, stroke.keycode.low::<u8>(), shifted(stroke, final_group));
        }
        // Keys typed after the word are deleted with it and typed again in the new layout. They are left out of `expected` on
        // purpose: the engine must see them come back as the user's own input.
        let mut late_sequence: Vec<(bool, u8)> = Vec::new();
        for stroke in late {
            tap(&mut late_sequence, stroke.keycode.low::<u8>(), shifted(stroke, final_group));
        }
        let preserve_boundary_layout = literal.iter().any(|stroke| stroke.character_for(target_group) != stroke.character);
        let literal_group = literal.first().map_or(rendered_source_group, |stroke| stroke.group);
        if literal_group < 0 || literal_group.to::<usize>() >= self.group_count || literal.iter().any(|stroke| stroke.group != literal_group) {
            return Err(failed("Некорректная раскладка пунктуации; замена отменена"));
        }
        let mut boundary_sequence: Vec<(bool, u8)> = Vec::new();
        for stroke in &literal {
            tap(&mut boundary_sequence, stroke.keycode.low::<u8>(), shifted(stroke, literal_group));
        }
        let none: Vec<(bool, u8)> = Vec::new();
        let target_boundary_sequence = if preserve_boundary_layout || kept_tail != 0 { &none } else { &boundary_sequence };
        let source_boundary_sequence = if preserve_boundary_layout && kept_tail == 0 { &boundary_sequence } else { &none };
        // After a kept word the boundary follows it in the same layout.
        let kept_boundary_sequence = if kept_tail != 0 { &boundary_sequence } else { &none };
        let inject_inner = lock(&self.inject_lock);
        self.expect(&[&sequence, target_boundary_sequence, source_boundary_sequence, &kept_sequence, kept_boundary_sequence]);
        api.grab_control(true);
        let outcome = (|| -> Result<(), X11Error> {
            Self::lock_to(&*api, target_group, "Не удалось включить XKB-группу ")?;
            Self::fake(&*api, &sequence)?;
            Self::fake(&*api, target_boundary_sequence)?;
            if !source_boundary_sequence.is_empty() {
                Self::lock_to(&*api, literal_group, "Не удалось временно включить XKB-группу ")?;
                Self::fake(&*api, source_boundary_sequence)?;
                Self::lock_to(&*api, target_group, "Не удалось восстановить XKB-группу ")?;
            }
            if kept_tail != 0 {
                Self::lock_to(&*api, final_group, "Не удалось вернуть XKB-группу ")?;
                Self::fake(&*api, &kept_sequence)?;
                Self::fake(&*api, kept_boundary_sequence)?;
            }
            Self::fake(&*api, &late_sequence)?;
            api.sync();
            Ok(())
        })();
        if outcome.is_err() {
            self.forget_expected();
        }
        api.grab_control(false);
        api.flush();
        outcome.map(|()| 0)
    }

    fn read_selection(&self, application: &str, window: i64, process: ProcessForWindow) -> Result<SelectedText, SelectionError> {
        let mut source = lock(&self.selection_source);
        if source.is_none() {
            match &self.selection_factory {
                Some(factory) => *source = Some(factory(process)),
                None => return Err(SelectionRefused::new(RefusalReason::Unsupported).into()),
            }
        }
        source.as_mut().expect("just built").read_selection(application, window)
    }

    fn replace_selection(&self, selected: &SelectedText, replacement: &str, final_group: Option<i32>) -> Result<bool, SelectionError> {
        let Some(api) = self.api() else { return Err(failed("X11 backend не запущен").into()) };
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
        let Some(keyboard_state) = api.xkb_state() else {
            return Err(failed("Не удалось прочитать состояние клавиатуры перед заменой выделения").into());
        };
        let caps_lock = u32::from(keyboard_state.locked_mods) & LOCK_MASK != 0;
        let original_group = i32::from(keyboard_state.group);
        let shift_keycode = api.keycode_for_keysym(SHIFT_L_KEYSYM);
        let left_keycode = api.keycode_for_keysym(LEFT_KEYSYM);
        if shift_keycode == 0 || left_keycode == 0 {
            return Err(failed("X-сервер не вернул keycode для Shift/Left").into());
        }
        let tables = self.typing_tables(&*api);
        let mut runs: Vec<(i32, Vec<(bool, u8)>)> = Vec::new();
        let mut characters = replacement.chars();
        for (group, keys) in typing_runs(replacement, &tables, i64::from(original_group))? {
            let mut events: Vec<(bool, u8)> = Vec::new();
            for ((keycode, shifted), character) in keys.into_iter().zip(characters.by_ref()) {
                // Caps Lock turns a letter over, and Shift turns it back.
                let with_shift = shifted ^ (pystr::isalpha(&character.to_string()) && caps_lock);
                if with_shift {
                    events.push((true, shift_keycode));
                }
                events.push((true, keycode.low::<u8>()));
                events.push((false, keycode.low::<u8>()));
                if with_shift {
                    events.push((false, shift_keycode));
                }
            }
            runs.push((group.low::<i32>(), events));
        }
        let steps = caret_steps(replacement);
        let mut reselect: Vec<(bool, u8)> = Vec::new();
        if let Some(steps) = steps {
            reselect.push((true, shift_keycode));
            for _ in 0..steps {
                reselect.push((true, left_keycode));
                reselect.push((false, left_keycode));
            }
            reselect.push((false, shift_keycode));
        }
        let typed: Vec<(bool, u8)> = runs.iter().flat_map(|(_, events)| events.iter().copied()).collect();
        let inject_inner = lock(&self.inject_lock);
        self.expect(&[&typed, &reselect]);
        api.grab_control(true);
        let outcome = (|| -> Result<(), X11Error> {
            let last = (final_group.unwrap_or(original_group), reselect.clone());
            for (group, events) in runs.iter().chain(std::iter::once(&last)) {
                Self::lock_to(&*api, *group, "Не удалось включить XKB-группу ")?;
                Self::fake(&*api, events)?;
            }
            api.sync();
            Ok(())
        })();
        if outcome.is_err() {
            self.forget_expected();
        }
        api.grab_control(false);
        api.flush();
        outcome?;
        Ok(steps.is_some())
    }
}

impl InputBackend for X11Backend {
    fn start(&self, listener: KeyListener) -> Result<(), BackendError> {
        self.shared.start(listener)
    }

    /// Accepted for the shared backend contract and deliberately ignored.
    ///
    /// XRecord is a passive observer: X11 delivers the key to the focused client no matter what this process decides, so a
    /// key can never be withheld the way the Windows hook withholds it.
    fn set_key_filter(&self, _predicate: Option<KeyFilter>) {}

    /// Accepted for the shared contract; XRecord cannot withhold keys.
    fn hold_input(&self) {}

    fn release_input(&self) -> Result<i32, BackendError> {
        Ok(0)
    }

    /// XRecord cannot defer actions; there is nothing to deliver here.
    fn complete_action(&self, _deliver: bool) -> Result<i32, BackendError> {
        Ok(0)
    }

    fn stop(&self) {
        self.shared.stop();
    }

    fn close(&self) {
        self.shared.close();
    }

    fn current_group(&self) -> Result<i32, BackendError> {
        Ok(self.shared.current_group())
    }

    fn switch_group(&self, group: i32) -> Result<(), BackendError> {
        self.shared.switch_group(group)
    }

    fn active_application(&self) -> String {
        self.shared.active_application()
    }

    fn focused_window(&self) -> Option<FocusInfo> {
        self.shared.focused_window()
    }

    fn probe(&self) -> BackendProbe {
        self.shared.probe()
    }

    fn inject_correction(&self, injection: Injection<'_>) -> Result<i32, BackendError> {
        self.shared.inject_correction(injection)
    }

    fn supports_selection(&self) -> bool {
        true
    }

    /// The text selected in the focused field, read through AT-SPI.
    fn read_selection(&self, application: &str, window: i64) -> Result<SelectedText, SelectionError> {
        self.shared.read_selection(application, window, self.process_for_window())
    }

    /// Writes `replacement` over the selection, then leaves the layout group in `final_group`.
    ///
    /// A field with AT-SPI's EditableText takes the text from the reader itself, any character included. Otherwise it is
    /// typed: XTest types keys, not characters, so each character goes on the key and in the group that type it, and a run of
    /// one script is typed in one group. A character no key of either group types leaves the selection as it is, with nothing
    /// typed: a paste would need the clipboard, whose owner on X11 has to answer every program that asks for it. Typed text
    /// is selected back with Shift+Left, and the group is left in `final_group` or where it was.
    fn replace_selection(&self, selected: &SelectedText, replacement: &str, final_group: Option<i32>) -> Result<bool, SelectionError> {
        self.shared.replace_selection(selected, replacement, final_group)
    }

    fn input_anchor(&self) -> Option<ScreenAnchor> {
        let api = self.shared.api()?;
        let (x, y) = api.pointer()?;
        let window = api.input_focus().filter(|window| *window != 0).map(i64::from);
        Some(ScreenAnchor { x, y, window })
    }

    fn restore_window(&self, window: Option<i64>) -> bool {
        match (self.shared.api(), window.and_then(|window| u32::try_from(window).ok())) {
            (Some(api), Some(window)) => {
                api.set_input_focus(window);
                api.flush();
                true
            }
            _ => false,
        }
    }
}

impl Drop for X11Backend {
    fn drop(&mut self) {
        self.shared.close();
    }
}

#[cfg(test)]
#[path = "x11_backend_tests.rs"]
mod tests;
