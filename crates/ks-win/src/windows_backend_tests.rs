//! The Python's tests of this logic (`test_windows_backend.py`, `test_windows_selection.py`, `test_input_integrity.py`,
//! `test_input_delay.py`, and of `test_action_boundary.py` what needs no engine), driven by a fake API.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicUsize;
use std::sync::{Condvar, mpsc};

use super::*;
use crate::constants::VK_LCONTROL as CTRL;

const ENGLISH_HKL: i64 = 0x0000_0409;
const RUSSIAN_HKL: i64 = 0x0000_0419;
const ENGLISH_HKL_WITH_DEVICE_FLAGS: i64 = 0xF001_0409;
const ENGLISH_HKL_WITH_HANDLE: i64 = 0x1234_0409;
const RUSSIAN_HKL_WITH_HANDLE: i64 = 0x1234_0419;
const GERMAN_HKL: i64 = 0x0000_0407;
const SCAN_CODE_A: u32 = 30;
const SCAN_CODE_ENTER: u32 = 0x1C;
const SCAN_CODE_SEMICOLON: u32 = 39;
const SCAN_CODE_LEFT_ALT: u32 = 0x38;
const SCAN_CODE_LEFT_SHIFT: u32 = 42;
const SCAN_CODE_PAUSE: u32 = 0x45;
const LATE_KEY_SCAN_CODE: u32 = 31;
const HELD_KEY_SCAN_CODE: u32 = 32;
const INJECTED_KEY_SCAN_CODE: u32 = 45;
const PACKET_CHARACTER_CODE: u32 = 0x0430;
const VK_D: u32 = 0x44;
const VK_X: u32 = 0x58;
const WINDOWS_FAKE_HWND: i64 = 300;
const WINDOWS_OTHER_HWND: i64 = 301;
const WINDOWS_FOREGROUND_HWND: i64 = 0x5150;
const WINDOWS_INACTIVE_HWND: i64 = 55;
const KEYSWITCH_WINDOW_HWND: i64 = 302;
const TASKBAR_HWND: i64 = 303;
const EXPLORER_PROCESS_ID: i64 = 5151;
const FAKE_FOREGROUND_PROCESS_ID: i64 = 4242;
const WINDOWS_OWN_PROCESS_ID: i64 = 7777;
const SELECTION_FIXTURE_WINDOW: i64 = 7;
const SELECTION_FIXTURE_OTHER_WINDOW: i64 = 8;
const SELECTION_FIXTURE_CLIPBOARD_FORMAT: u32 = 0xC0DE;
const SELECTION_FIXTURE_SEQUENCE_BEFORE: i64 = 40;
const HOOK_KEY_TIME_MS: i64 = 1_431_655_765;
const INPUT_DELAY_LATE_MS: i64 = ks_backend::constants::INPUT_DELAY_LATE_MS;
const INPUT_DELAY_SLOW_CALLBACK_MS: i64 = ks_backend::constants::INPUT_DELAY_SLOW_CALLBACK_MS;
const INPUT_DELAY_UNSTAMPED_MS: i64 = ks_backend::constants::INPUT_DELAY_UNSTAMPED_MS;
const HOOK_GAP_MS: i64 = ks_backend::constants::HOOK_GAP_MS;
const UNSEEN_TIMES: i64 = ks_cast::const_i64(UNSEEN_INPUT_MIN_TIMES);

fn saved_clipboard() -> ClipboardContents {
    ClipboardContents { formats: vec![(SELECTION_FIXTURE_CLIPBOARD_FORMAT, b"user data".to_vec())], complete: true }
}

fn copy_keys() -> Vec<NativeInput> {
    vec![
        NativeInput::key(true, VK_CONTROL),
        NativeInput::extended_key(true, VK_INSERT),
        NativeInput::extended_key(false, VK_INSERT),
        NativeInput::key(false, VK_CONTROL),
    ]
}

fn paste_keys() -> Vec<NativeInput> {
    vec![
        NativeInput::key(true, VK_CONTROL),
        NativeInput::key(true, VK_V),
        NativeInput::key(false, VK_V),
        NativeInput::key(false, VK_CONTROL),
    ]
}

type OnSend = Arc<dyn Fn(&[NativeInput]) + Send + Sync>;
type OnRequest = Arc<dyn Fn(i64) + Send + Sync>;

struct FakeState {
    layout_values: Vec<i64>,
    current_layout: i64,
    application: String,
    caps_lock: bool,
    requests: Vec<i64>,
    sent: Vec<Vec<NativeInput>>,
    send_count: Option<usize>,
    send_error: bool,
    accept_switch: bool,
    apply_switch: bool,
    refuse_after_first_request: Option<i64>,
    hook_error: Option<String>,
    signal_ready: bool,
    stopped: bool,
    hook_listener: Option<HookListener>,
    stop_calls: usize,
    layout_calls: usize,
    translation: HashMap<(u32, i64), String>,
    anchor: Option<ScreenAnchor>,
    activated_windows: Vec<i64>,
    /// The layouts of windows not in front; one not here has the layout of the window in front.
    window_layouts: HashMap<i64, i64>,
    /// When set, an activated window comes to the front after this many more looks at the front; when not, it stays
    /// where it was, as a test before these needs.
    foreground_after_activation: Option<usize>,
    pending_foreground: Option<(usize, i64)>,
    activation_refused: bool,
    foreground: i64,
    window_owners: HashMap<i64, i64>,
    process_id: i64,
    ticks: i64,
    last_input: i64,
    input_times: Vec<i64>,
    inactive_windows: Vec<i64>,
    released_keys: HashSet<u32>,
    key_down_calls: Vec<u32>,
    on_send: Option<OnSend>,
    on_request: Option<OnRequest>,
    /// Translate as the generator of the golden sequences does, instead of from `translation`.
    scripted_translation: bool,
    // A Windows with a clipboard: the copy keys put `copied` on it after `copy_delay` looks at the sequence.
    window_classes: HashMap<i64, String>,
    caret: bool,
    caret_windows: Vec<i64>,
    sequence: i64,
    saved: ClipboardContents,
    copied: Option<String>,
    copies: bool,
    copy_delay: usize,
    copy_pending: bool,
    busy_reads: usize,
    busy_texts: usize,
    restores: Vec<bool>,
    written: Vec<ClipboardContents>,
    text_limit: usize,
    no_control: bool,
}

struct FakeWindowsAPI {
    state: Mutex<FakeState>,
    stop_event: Condvar,
    clipboard: bool,
}

impl FakeWindowsAPI {
    fn new(clipboard: bool) -> Arc<Self> {
        Arc::new(FakeWindowsAPI {
            state: Mutex::new(FakeState {
                layout_values: vec![ENGLISH_HKL, RUSSIAN_HKL],
                current_layout: ENGLISH_HKL,
                application: "Notepad".to_owned(),
                caps_lock: false,
                requests: Vec::new(),
                sent: Vec::new(),
                send_count: None,
                send_error: false,
                accept_switch: true,
                apply_switch: true,
                refuse_after_first_request: None,
                hook_error: None,
                signal_ready: true,
                stopped: false,
                hook_listener: None,
                stop_calls: 0,
                layout_calls: 0,
                translation: HashMap::new(),
                anchor: Some(ScreenAnchor { x: 11, y: 22, window: Some(WINDOWS_FAKE_HWND) }),
                activated_windows: Vec::new(),
                window_layouts: HashMap::new(),
                foreground_after_activation: None,
                pending_foreground: None,
                activation_refused: false,
                foreground: WINDOWS_FAKE_HWND,
                window_owners: HashMap::from([(WINDOWS_FAKE_HWND, FAKE_FOREGROUND_PROCESS_ID)]),
                process_id: WINDOWS_OWN_PROCESS_ID,
                ticks: 0,
                last_input: 0,
                input_times: Vec::new(),
                inactive_windows: Vec::new(),
                released_keys: HashSet::new(),
                key_down_calls: Vec::new(),
                on_send: None,
                on_request: None,
                scripted_translation: false,
                window_classes: HashMap::new(),
                caret: true,
                caret_windows: Vec::new(),
                sequence: SELECTION_FIXTURE_SEQUENCE_BEFORE,
                saved: saved_clipboard(),
                copied: Some("ghbdtn".to_owned()),
                copies: true,
                copy_delay: 0,
                copy_pending: false,
                busy_reads: 0,
                busy_texts: 0,
                restores: Vec::new(),
                written: Vec::new(),
                text_limit: 0,
                no_control: false,
            }),
            stop_event: Condvar::new(),
            clipboard,
        })
    }

    fn plain() -> Arc<Self> {
        Self::new(false)
    }

    fn with_clipboard() -> Arc<Self> {
        Self::new(true)
    }

    fn state(&self) -> MutexGuard<'_, FakeState> {
        self.state.lock().unwrap()
    }

    fn sent(&self) -> Vec<Vec<NativeInput>> {
        self.state().sent.clone()
    }

    fn requests(&self) -> Vec<i64> {
        self.state().requests.clone()
    }

    fn hook(&self) -> HookListener {
        self.state().hook_listener.clone().expect("the hook runs")
    }
}

impl WindowsAPI for FakeWindowsAPI {
    fn loaded_layouts(&self) -> Result<Vec<i64>, WindowsBackendError> {
        let mut state = self.state();
        state.layout_calls += 1;
        Ok(state.layout_values.clone())
    }

    fn foreground_layout(&self) -> i64 {
        self.state().current_layout
    }

    fn window_layout(&self, window: i64) -> i64 {
        let state = self.state();
        state.window_layouts.get(&window).copied().unwrap_or(state.current_layout)
    }

    fn request_layout(&self, layout: i64) -> bool {
        let callback = self.state().on_request.clone();
        if let Some(callback) = callback {
            callback(layout);
        }
        let mut state = self.state();
        if state.refuse_after_first_request == Some(layout) && !state.requests.is_empty() {
            state.requests.push(layout);
            return false;
        }
        state.requests.push(layout);
        if state.accept_switch && state.apply_switch {
            state.current_layout = layout;
        }
        state.accept_switch
    }

    fn translate_key(&self, virtual_key: u32, _scan_code: u32, modifiers: u32, layout: i64) -> String {
        let state = self.state();
        if !state.scripted_translation {
            return state.translation.get(&(virtual_key, layout)).cloned().unwrap_or_default();
        }
        if virtual_key == VK_SPACE {
            return " ".to_owned();
        }
        if virtual_key == VK_OEM_1 {
            return if layout == ENGLISH_HKL { ";" } else { "ж" }.to_owned();
        }
        if !(VK_A..=VK_Z).contains(&virtual_key) {
            return String::new();
        }
        let code = if layout == ENGLISH_HKL { virtual_key + 32 } else { 0x430 + virtual_key - VK_A };
        let base = char::from_u32(code).unwrap().to_string();
        if (modifiers & SHIFT_MASK != 0) == (modifiers & LOCK_MASK != 0) { base } else { pystr::upper(&base) }
    }

    fn send_inputs(&self, inputs: &[NativeInput]) -> Result<usize, WindowsBackendError> {
        let (count, callback) = {
            let mut state = self.state();
            if state.send_error {
                return Err(failed("send refused"));
            }
            state.sent.push(inputs.to_vec());
            if self.clipboard && inputs == copy_keys() && state.copies {
                state.copy_pending = true;
            }
            (state.send_count.unwrap_or(inputs.len()), state.on_send.clone())
        };
        if let Some(callback) = callback {
            callback(inputs);
        }
        Ok(count)
    }

    fn active_application(&self) -> String {
        self.state().application.clone()
    }

    fn foreground_window(&self) -> i64 {
        let mut state = self.state();
        if let Some((looks, window)) = state.pending_foreground {
            if looks == 0 {
                state.foreground = window;
                state.pending_foreground = None;
            } else {
                state.pending_foreground = Some((looks - 1, window));
            }
        }
        state.foreground
    }

    fn focused_control(&self) -> i64 {
        let state = self.state();
        if state.no_control { 0 } else { state.foreground }
    }

    fn window_process_id(&self, window: i64) -> i64 {
        self.state().window_owners.get(&window).copied().unwrap_or(0)
    }

    fn current_process_id(&self) -> i64 {
        self.state().process_id
    }

    fn input_anchor(&self) -> Option<ScreenAnchor> {
        self.state().anchor
    }

    fn activate_window(&self, window: i64) -> bool {
        let mut state = self.state();
        state.activated_windows.push(window);
        if let Some(looks) = state.foreground_after_activation {
            state.pending_foreground = Some((looks, window));
        }
        !state.activation_refused
    }

    fn keep_window_inactive(&self, window: i64) -> bool {
        self.state().inactive_windows.push(window);
        true
    }

    fn caps_lock_enabled(&self) -> bool {
        self.state().caps_lock
    }

    fn key_down(&self, virtual_key: u32) -> bool {
        let mut state = self.state();
        state.key_down_calls.push(virtual_key);
        !state.released_keys.contains(&virtual_key)
    }

    fn tick_count(&self) -> i64 {
        self.state().ticks
    }

    fn last_input_time(&self) -> Result<i64, WindowsBackendError> {
        Ok(self.state().last_input)
    }

    fn hook_input_times(&self) -> Vec<i64> {
        self.state().input_times.clone()
    }

    fn run_keyboard_hook(&self, listener: HookListener, ready: HookReady) -> Result<(), WindowsBackendError> {
        let signal_ready = {
            let mut state = self.state();
            state.hook_listener = Some(listener);
            if let Some(message) = state.hook_error.clone() {
                return Err(failed(message));
            }
            state.signal_ready
        };
        if signal_ready {
            ready();
        }
        let state = self.state();
        let _state = self.stop_event.wait_timeout_while(state, Duration::from_secs(1), |state| !state.stopped).unwrap();
        Ok(())
    }

    fn stop_keyboard_hook(&self) {
        let mut state = self.state();
        state.stop_calls += 1;
        state.stopped = true;
        self.stop_event.notify_all();
    }

    fn selection_api(&self) -> Option<&dyn WindowsSelectionAPI> {
        if self.clipboard { Some(self) } else { None }
    }
}

impl WindowsSelectionAPI for FakeWindowsAPI {
    fn window_class(&self, window: i64) -> String {
        self.state().window_classes.get(&window).cloned().unwrap_or_else(|| "Notepad".to_owned())
    }

    fn text_caret(&self, window: i64) -> bool {
        let mut state = self.state();
        state.caret_windows.push(window);
        state.caret
    }

    fn clipboard_sequence(&self) -> i64 {
        let mut state = self.state();
        if state.copy_pending && state.copy_delay != 0 {
            state.copy_delay -= 1;
        } else if state.copy_pending {
            state.copy_pending = false;
            state.sequence += 1;
        }
        state.sequence
    }

    fn read_clipboard(&self, _limit_bytes: usize) -> Option<ClipboardContents> {
        let mut state = self.state();
        if state.busy_reads != 0 {
            state.busy_reads -= 1;
            return None;
        }
        Some(state.saved.clone())
    }

    fn clipboard_text(&self, limit_characters: usize) -> Option<String> {
        let mut state = self.state();
        state.text_limit = limit_characters;
        if state.busy_texts != 0 {
            state.busy_texts -= 1;
            return None;
        }
        state.copied.clone()
    }

    fn write_clipboard(&self, contents: &ClipboardContents) -> Result<bool, WindowsBackendError> {
        let mut state = self.state();
        state.written.push(contents.clone());
        if state.restores.is_empty() {
            return Ok(true);
        }
        Ok(state.restores.remove(0))
    }
}

/// A clock that moves only when asked: each sleep moves it, and scripted readings come first where a test has them.
#[derive(Default)]
struct FakeClock {
    now: Mutex<f64>,
    step: f64,
    readings: Mutex<VecDeque<f64>>,
    perf: Mutex<VecDeque<f64>>,
    sleeps: Mutex<Vec<f64>>,
}

impl FakeClock {
    fn stepping(step: f64) -> Arc<Self> {
        Arc::new(FakeClock { step, ..FakeClock::default() })
    }
}

impl Clock for FakeClock {
    fn monotonic(&self) -> f64 {
        if let Some(reading) = self.readings.lock().unwrap().pop_front() {
            return reading;
        }
        let mut now = self.now.lock().unwrap();
        *now += self.step;
        *now
    }

    fn perf_counter(&self) -> f64 {
        self.perf.lock().unwrap().pop_front().unwrap_or(0.0)
    }

    fn sleep(&self, seconds: f64) {
        self.sleeps.lock().unwrap().push(seconds);
        *self.now.lock().unwrap() += seconds;
    }
}

/// What UI Automation answers in a test, and what it was asked.
struct ReaderScript {
    answer: Result<Option<SelectedText>, SelectionError>,
    editable: Result<bool, SelectionError>,
    /// The field shows this after `late` more looks.
    shown: Option<String>,
    late: usize,
    on_read: Option<Arc<dyn Fn() + Send + Sync>>,
    windows: Vec<i64>,
    attempts: usize,
    unavailable: bool,
}

impl Default for ReaderScript {
    fn default() -> Self {
        ReaderScript {
            answer: Ok(None),
            editable: Ok(false),
            shown: None,
            late: 0,
            on_read: None,
            windows: Vec::new(),
            attempts: 0,
            unavailable: false,
        }
    }
}

type SharedScript = Arc<Mutex<ReaderScript>>;

fn script(setup: impl FnOnce(&mut ReaderScript)) -> SharedScript {
    let mut script = ReaderScript::default();
    setup(&mut script);
    Arc::new(Mutex::new(script))
}

struct FakeReader(SharedScript);

impl SelectionReader for FakeReader {
    fn read_selection(&self, window: i64) -> Result<Option<SelectedText>, SelectionError> {
        let (on_read, answer) = {
            let mut script = self.0.lock().unwrap();
            script.windows.push(window);
            let answer = if let Some(shown) = script.shown.clone() {
                if script.late != 0 {
                    script.late -= 1;
                    Ok(None)
                } else {
                    Ok(Some(SelectedText { text: shown, source: SOURCE_UIA.to_owned() }))
                }
            } else {
                script.answer.clone()
            };
            (script.on_read.clone(), answer)
        };
        if let Some(on_read) = on_read {
            on_read();
        }
        answer
    }

    fn focused_editable(&self, _window: i64) -> Result<bool, SelectionError> {
        self.0.lock().unwrap().editable.clone()
    }
}

/// A backend whose jobs run at once and whose clock never really waits.
fn backend_with(api: &Arc<FakeWindowsAPI>, clock: Arc<dyn Clock>, script: SharedScript) -> WindowsBackend {
    let parts = BackendParts {
        clock,
        start_job: Box::new(|job| {
            job();
            Ok(())
        }),
        selection_reader_factory: Box::new(move || {
            let unavailable = {
                let mut state = script.lock().unwrap();
                state.attempts += 1;
                state.unavailable
            };
            if unavailable {
                return Err(failed("no UI Automation"));
            }
            Ok(Box::new(FakeReader(Arc::clone(&script))))
        }),
        listener_start_timeout_seconds: KEYBOARD_LISTENER_START_TIMEOUT_SECONDS,
    };
    WindowsBackend::with_parts(api.clone(), parts)
}

fn backend(api: &Arc<FakeWindowsAPI>) -> WindowsBackend {
    backend_with(api, FakeClock::stepping(0.0), script(|_| {}))
}

fn reading_backend(api: &Arc<FakeWindowsAPI>, reader: &SharedScript) -> WindowsBackend {
    backend_with(api, FakeClock::stepping(0.25), Arc::clone(reader))
}

fn key_event() -> KeyEvent {
    KeyEvent {
        pressed: true,
        keycode: SCAN_CODE_A,
        key_name: "a".to_owned(),
        character: "a".to_owned(),
        characters: vec!["a".to_owned(), "ф".to_owned()],
        group: 0,
        state: 0,
        timestamp: 100,
        ..KeyEvent::default()
    }
}

fn key_with(keycode: u32, character: &str, characters: [&str; 2], group: i32) -> KeyEvent {
    KeyEvent {
        keycode,
        character: character.to_owned(),
        characters: characters.map(str::to_owned).to_vec(),
        group,
        ..key_event()
    }
}

fn injection(strokes: &[KeyEvent], target_group: i32) -> Injection<'_> {
    Injection {
        strokes,
        target_group,
        boundary: None,
        source_group: None,
        late: &[],
        trailing: &[],
        kept_tail: 0,
        erased: None,
    }
}

fn message(error: &BackendError) -> String {
    error.to_string()
}

type Collected = Arc<Mutex<Vec<KeyEvent>>>;

fn collecting(events: &Collected) -> KeyListener {
    let events = Arc::clone(events);
    Arc::new(move |event| events.lock().unwrap().push(event))
}

fn collected() -> Collected {
    Arc::new(Mutex::new(Vec::new()))
}

fn native(pressed: bool, virtual_key: u32, scan_code: u32, injected: bool, timestamp: i64) -> NativeKeyEvent {
    NativeKeyEvent::new(pressed, virtual_key, scan_code, false, injected, timestamp)
}

fn replayed_key(pressed: bool, virtual_key: u32, scan_code: u32) -> NativeInput {
    NativeInput { pressed, virtual_key, scan_code, synthetic: false, replayed: true, ..NativeInput::new(pressed) }
}

fn replayed_scan(pressed: bool, scan_code: u32) -> NativeInput {
    NativeInput { synthetic: false, replayed: true, ..NativeInput::scan(pressed, scan_code) }
}

// ------------------------------------------------------------------------------------------- helpers and lifecycle

#[test]
fn small_answers_of_the_backend() {
    assert!(!later(5, 5));
    assert!(later(6, 5));
    assert!(!later(5, 6));
    assert!(message(&spawn_failed(std::io::Error::other("no threads"))).contains("no threads"));
    let api = FakeWindowsAPI::plain();
    let shared: Arc<dyn WindowsAPI> = api.clone();
    assert!(WindowsBackend::new(Some(shared)).is_ok());
}

#[test]
fn a_window_without_a_focused_control_is_its_own_place() {
    let api = FakeWindowsAPI::plain();
    let subject = backend(&api);
    {
        let mut state = api.state();
        state.foreground = 77;
        state.no_control = true;
    }
    let info = subject.focused_window().expect("a window");
    assert_eq!(info.window, 77);
}

#[test]
fn a_layout_nobody_can_name_lets_the_key_pass() {
    let api = FakeWindowsAPI::plain();
    let subject = backend(&api);
    api.state().layout_values.clear();
    assert!(!subject.handle_native(native(true, VK_A, VK_A, false, 10)));
    let api = FakeWindowsAPI::plain();
    let subject = backend(&api);
    api.state().current_layout = 999;
    assert!(!subject.handle_native(native(true, VK_A, VK_A, false, 11)));
    assert!(subject.switch_group(2).is_err());
    assert!(subject.inner.switch_group_inner(2).is_err());
}

#[test]
fn modifiers_lost_on_the_way_are_named_together() {
    let api = FakeWindowsAPI::plain();
    let subject = backend(&api);
    let (first, second) = (MODIFIER_KEYS[0], MODIFIER_KEYS[1]);
    let _ = subject.handle_native(native(true, first, first, false, 20));
    let _ = subject.handle_native(native(true, second, second, false, 21));
    {
        let mut state = api.state();
        state.released_keys.insert(first);
        state.released_keys.insert(second);
    }
    let _ = subject.handle_native(native(true, VK_A, VK_A, false, 21 + MODIFIER_RECHECK_GAP_MS + 1));
    let hook = lock(&subject.inner.hook);
    assert!(!hook.pressed.contains(first) && !hook.pressed.contains(second));
}

#[test]
fn a_mask_key_that_cannot_be_sent_is_only_noted() {
    let api = FakeWindowsAPI::plain();
    let parts = BackendParts {
        clock: FakeClock::stepping(0.0),
        start_job: Box::new(|_| Err(failed("no thread"))),
        selection_reader_factory: Box::new(|| Err(failed("no UI Automation"))),
        listener_start_timeout_seconds: KEYBOARD_LISTENER_START_TIMEOUT_SECONDS,
    };
    let subject = WindowsBackend::with_parts(api.clone(), parts);
    subject.set_key_filter(Some(Arc::new(|_: &KeyEvent| KeyDisposition::Consume)));
    let _ = subject.handle_native(native(true, ALT_KEYS[1], ALT_KEYS[1], false, 30));
    assert!(subject.handle_native(native(true, VK_A, VK_A, false, 31)));
}

#[test]
fn a_stop_with_keys_that_cannot_be_given_back_leaves_the_hook_in_place() {
    let api = FakeWindowsAPI::plain();
    let subject = backend(&api);
    let events = collected();
    subject.start(collecting(&events)).unwrap();
    subject.set_key_filter(Some(Arc::new(|event: &KeyEvent| if event.key_name == "Return" { KeyDisposition::Defer } else { KeyDisposition::Pass })));
    let hook = api.hook();
    assert!(hook(native(true, VK_RETURN, SCAN_CODE_ENTER, false, 40)));
    let _ = hook(native(true, VK_A, VK_A, false, 41));
    api.state().send_error = true;
    subject.stop();
    assert!(subject.running());
    api.state().send_error = false;
    subject.set_key_filter(None);
    let _ = subject.complete_action(false);
    subject.stop();
}

#[test]
fn a_replacement_goes_on_while_the_input_held_is_where_it_was() {
    let api = FakeWindowsAPI::with_clipboard();
    let subject = reading_backend(&api, &script(|_| {}));
    subject.try_hold_input().unwrap();
    assert!(subject.replace_selection(&selected_by(SOURCE_UIA, "ghbdtn"), "пр", None).unwrap());
    let _ = subject.release_input();
}

#[test]
fn layout_languages_pair_selection_and_key_names() {
    assert_eq!(primary_language(ENGLISH_HKL_WITH_DEVICE_FLAGS), LANG_ENGLISH);
    assert_eq!(primary_language(RUSSIAN_HKL), LANG_RUSSIAN);
    assert_eq!(select_layout_pair(&[RUSSIAN_HKL, ENGLISH_HKL, ENGLISH_HKL]).unwrap(), (ENGLISH_HKL, RUSSIAN_HKL));
    for layouts in [[ENGLISH_HKL], [RUSSIAN_HKL]] {
        assert!(message(&select_layout_pair(&layouts).unwrap_err()).contains("английская и русская"));
    }
    assert_eq!(key_name(VK_BACK), "BackSpace");
    assert_eq!(key_name(u32::from(b'A')), "a");
    assert_eq!(key_name(u32::from(b'7')), "7");
    assert_eq!(key_name(0xFE), "VK_FE");
    assert_eq!(key_name(0x07), "VK_07");
    // Punctuation keys carry the X11 keysym names, so a log line reads "comma", never "VK_BC".
    assert_eq!(key_name(VK_OEM_COMMA), "comma");
    assert_eq!(key_name(VK_OEM_PERIOD), "period");
    assert_eq!(key_name(VK_OEM_7), "apostrophe");
    assert_eq!(key_name(VK_SCROLL), "Scroll_Lock");
    assert_eq!(key_name(VK_PACKET), UNICODE_PACKET_KEY_NAME);
}

#[cfg(not(windows))]
#[test]
fn the_constructor_rejects_the_default_native_api_outside_windows() {
    let error = WindowsBackend::new(None).err().unwrap();
    assert!(message(&error).contains("только в Windows"));
}

#[test]
fn layouts_probe_group_and_application() {
    let api = FakeWindowsAPI::plain();
    let subject = backend(&api);
    assert_eq!(subject.layouts().unwrap(), (ENGLISH_HKL, RUSSIAN_HKL));
    assert_eq!(subject.layouts().unwrap(), (ENGLISH_HKL, RUSSIAN_HKL));
    assert_eq!(api.state().layout_calls, 1);
    let probe = subject.probe();
    assert!(probe.available);
    assert_eq!(probe.session_type, "windows");
    assert_eq!(probe.xkb_version, "HKL 00000409,00000419");
    assert_eq!(subject.current_group().unwrap(), 0);
    assert_eq!(subject.active_application(), "Notepad");
    assert_eq!(subject.input_anchor(), Some(ScreenAnchor { x: 11, y: 22, window: Some(WINDOWS_FAKE_HWND) }));
    assert!(!subject.restore_window(None));
    assert!(subject.restore_window(Some(WINDOWS_FAKE_HWND)));
    assert_eq!(api.state().activated_windows, [WINDOWS_FAKE_HWND]);

    api.state().current_layout = ENGLISH_HKL_WITH_HANDLE;
    assert_eq!(subject.current_group().unwrap(), 0);
    api.state().current_layout = RUSSIAN_HKL_WITH_HANDLE;
    assert_eq!(subject.current_group().unwrap(), 1);
    api.state().current_layout = GERMAN_HKL;
    assert_eq!(subject.current_group().unwrap(), -1);

    assert!(message(&subject.switch_group(2).unwrap_err()).contains("Неизвестная группа"));
    subject.switch_group(1).unwrap();
    assert_eq!(api.requests().last(), Some(&RUSSIAN_HKL));
}

#[test]
fn the_probe_reports_a_layout_failure() {
    let api = FakeWindowsAPI::plain();
    api.state().layout_values = vec![ENGLISH_HKL];
    let probe = backend(&api).probe();
    assert!(!probe.available);
    assert_eq!(probe.current_group, -1);
    assert_eq!((probe.record_version.as_str(), probe.xtest_version.as_str(), probe.xkb_version.as_str()), ("—", "—", "—"));
    assert!(probe.error.contains("русская"));
}

#[test]
fn start_events_idempotence_stop_and_close() {
    let api = FakeWindowsAPI::plain();
    {
        let mut state = api.state();
        state.translation.insert((u32::from(b'A'), ENGLISH_HKL), "a".to_owned());
        state.translation.insert((u32::from(b'A'), RUSSIAN_HKL), "ф".to_owned());
        state.caps_lock = true;
    }
    let subject = backend(&api);
    let events = collected();
    subject.start(collecting(&events)).unwrap();
    assert!(subject.running());
    subject.start(collecting(&events)).unwrap();
    let hook = api.hook();
    hook(native(true, u32::from(b'A'), SCAN_CODE_A, true, 123));
    {
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].characters, ["a", "ф"]);
        assert_eq!(events[0].character, "a");
        assert!(events[0].synthetic);
        assert!(events[0].caps_lock());
    }

    // Without a filter every key reaches the window.
    assert!(!hook(native(true, VK_RETURN, SCAN_CODE_ENTER, false, 124)));

    // The filter answers for presses; the matching release is hidden with them so the window never sees a key-up it has
    // no key-down for.
    let consumed: Arc<Mutex<Vec<String>>> = Arc::default();
    let seen = Arc::clone(&consumed);
    subject.set_key_filter(Some(Arc::new(move |event: &KeyEvent| {
        seen.lock().unwrap().push(event.key_name.clone());
        if event.key_name == "Return" { KeyDisposition::Consume } else { KeyDisposition::Pass }
    })));
    // The earlier press of Return was let through: this press is the first the filter answers for.
    assert!(hook(native(true, VK_RETURN, SCAN_CODE_ENTER, false, 125)));
    assert!(hook(native(false, VK_RETURN, SCAN_CODE_ENTER, false, 126)));
    // A release with no swallowed press behind it passes through.
    assert!(!hook(native(false, VK_RETURN, SCAN_CODE_ENTER, false, 127)));
    assert!(!hook(native(true, u32::from(b'A'), SCAN_CODE_A, false, 128)));
    assert_eq!(*consumed.lock().unwrap(), ["Return", "a"]);

    subject.set_key_filter(None);
    assert!(!hook(native(true, VK_RETURN, SCAN_CODE_ENTER, false, 129)));
    subject.stop();
    assert!(!subject.running());
    assert_eq!(api.state().stop_calls, 1);
    subject.stop();
    subject.close();
}

#[test]
fn start_propagates_a_hook_failure() {
    let api = FakeWindowsAPI::plain();
    api.state().hook_error = Some("hook denied".to_owned());
    let subject = backend(&api);
    let error = subject.start(collecting(&collected())).unwrap_err();
    assert!(message(&error).contains("hook denied"));
    assert!(!subject.running());
    subject.stop();
}

#[test]
fn a_start_timeout_stops_the_native_hook() {
    let api = FakeWindowsAPI::plain();
    api.state().signal_ready = false;
    let parts = BackendParts { listener_start_timeout_seconds: 0.01, ..BackendParts::default() };
    let subject = WindowsBackend::with_parts(api.clone(), parts);
    let error = subject.start(collecting(&collected())).unwrap_err();
    assert_eq!(message(&error), format!("Win32 hook не подтвердил запуск за {}", quantity(0.01, SECONDS)));
    assert_eq!(api.state().stop_calls, 1);
}

#[test]
fn the_start_message_counts_the_seconds_of_its_constant() {
    assert_eq!(quantity(KEYBOARD_LISTENER_START_TIMEOUT_SECONDS, SECONDS), "5 секунд");
}

#[test]
fn the_focused_window_marks_windows_of_this_process() {
    let api = FakeWindowsAPI::plain();
    let subject = backend(&api);
    assert_eq!(subject.focused_window(), Some(FocusInfo { window: WINDOWS_FAKE_HWND, own: false, isolated_layout: false }));
    let own = api.state().process_id;
    api.state().window_owners.insert(WINDOWS_FAKE_HWND, own);
    assert_eq!(subject.focused_window(), Some(FocusInfo { window: WINDOWS_FAKE_HWND, own: true, isolated_layout: true }));
    api.state().foreground = WINDOWS_OTHER_HWND;
    assert_eq!(subject.focused_window(), Some(FocusInfo { window: WINDOWS_OTHER_HWND, own: false, isolated_layout: false }));
    api.state().foreground = 0;
    assert_eq!(subject.focused_window(), None);
    assert!(subject.keep_window_inactive(WINDOWS_INACTIVE_HWND));
    assert_eq!(api.state().inactive_windows, [WINDOWS_INACTIVE_HWND]);
}

#[test]
fn stop_does_not_wait_for_the_current_thread() {
    let api = FakeWindowsAPI::plain();
    let subject = backend(&api);
    // The sender stays alive, so a wait would last its whole timeout.
    let (_sender, finished) = mpsc::channel();
    *subject.inner.thread.lock().unwrap() = Some(HookThread { id: thread::current().id(), finished });
    subject.inner.running.store(true, Ordering::Release);
    let started = Instant::now();
    subject.stop();
    assert!(started.elapsed() < Duration::from_secs_f64(KEYBOARD_LISTENER_STOP_TIMEOUT_SECONDS));
    assert!(!subject.running());
    assert_eq!(api.state().stop_calls, 1);
}

#[test]
fn a_modifier_whose_release_the_hook_missed_is_forgotten() {
    log::capture::install();
    let api = FakeWindowsAPI::plain();
    let subject = backend(&api);
    let events = collected();
    subject.set_listener(Some(collecting(&events)));
    let press = |virtual_key: u32, timestamp: i64| {
        let _ = subject.handle_native(native(true, virtual_key, virtual_key, false, timestamp));
    };
    let last_state = || events.lock().unwrap().last().unwrap().state;

    // Ctrl+A in one go: the keyboard is not asked, the letter carries Ctrl.
    press(CTRL, 5000);
    press(VK_A, 5100);
    assert_eq!(last_state(), CONTROL_MASK);
    assert!(api.state().key_down_calls.is_empty());

    // A UAC prompt took the release of Ctrl; the next key comes in another window.
    api.state().released_keys.insert(CTRL);
    api.state().foreground = WINDOWS_FOREGROUND_HWND;
    press(VK_A, 5200);
    assert_eq!(last_state(), 0);
    assert_eq!(api.state().key_down_calls, [CTRL]);
    assert!(log::capture::contains(
        "keyswitch.windows_backend: Отпускание модификаторов не дошло до перехватчика, они забыты: Control_L"
    ));

    // Ctrl held down through a pause stays down.
    api.state().released_keys.clear();
    api.state().key_down_calls.clear();
    press(CTRL, 5000);
    press(VK_A, 5000 + MODIFIER_RECHECK_GAP_MS);
    assert_eq!(last_state(), CONTROL_MASK);
    assert_eq!(api.state().key_down_calls, [CTRL]);

    // Keys another program injected, and KeySwitch's own, ask nothing of the keyboard.
    api.state().released_keys.insert(CTRL);
    api.state().foreground = WINDOWS_FAKE_HWND;
    let _ = subject.handle_native(NativeKeyEvent { foreign: true, ..native(true, VK_A, VK_A, false, 5200) });
    let _ = subject.handle_native(NativeKeyEvent { replayed: true, ..native(true, VK_A, VK_A, false, 5200) });
    assert_eq!(last_state(), CONTROL_MASK);
    assert_eq!(api.state().key_down_calls, [CTRL]);
}

#[test]
fn the_event_state_tracks_modifiers_caps_and_an_unknown_group() {
    let api = FakeWindowsAPI::plain();
    let subject = backend(&api);
    let events = collected();
    subject.set_listener(Some(collecting(&events)));
    let modifiers = [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_CAPITAL];
    for virtual_key in modifiers {
        let _ = subject.handle_native(native(true, virtual_key, virtual_key, false, 1));
    }
    // A repeated CapsLock key-down must not toggle the lock back off.
    let _ = subject.handle_native(native(true, VK_CAPITAL, VK_CAPITAL, false, 2));
    assert_eq!(events.lock().unwrap().last().unwrap().state, SHIFT_MASK | CONTROL_MASK | ALT_MASK | SUPER_MASK | LOCK_MASK);
    for virtual_key in modifiers {
        let _ = subject.handle_native(native(false, virtual_key, virtual_key, false, 3));
    }
    assert_eq!(events.lock().unwrap().last().unwrap().state, LOCK_MASK);

    api.state().current_layout = GERMAN_HKL;
    let _ = subject.handle_native(native(true, u32::from(b'A'), SCAN_CODE_A, false, 4));
    assert_eq!(events.lock().unwrap().last().unwrap().character, "");
    assert_eq!(events.lock().unwrap().last().unwrap().group, -1);
    subject.set_listener(None);
    let _ = subject.handle_native(native(false, u32::from(b'A'), SCAN_CODE_A, false, 5));
}

// --------------------------------------------------------------------------------------------------------- injection

#[test]
fn a_correction_deletes_switches_replays_shift_and_boundary() {
    let api = FakeWindowsAPI::plain();
    let subject = backend(&api);
    let stroke = KeyEvent { state: SHIFT_MASK, ..key_event() };
    let boundary = key_with(SCAN_CODE_SEMICOLON, ";", [";", "ж"], 0);
    let strokes = [stroke];
    let held = subject
        .inject_correction(Injection { boundary: Some(&boundary), source_group: Some(0), ..injection(&strokes, 1) })
        .unwrap();
    assert_eq!(held, 0);
    // The layout switches first; the deletion needs no layout and goes at once, and the keys of the new layout follow once
    // the window's text service has it (WINDOWS_LAYOUT_SETTLE_SECONDS). The user's keys are held by the hook meanwhile, so
    // none slips in between.
    let sent = api.sent();
    assert_eq!(
        sent[0],
        [
            NativeInput::key(true, VK_BACK),
            NativeInput::key(false, VK_BACK),
            NativeInput::key(true, VK_BACK),
            NativeInput::key(false, VK_BACK),
        ]
    );
    assert_eq!(
        sent[1],
        [
            NativeInput::key(true, VK_SHIFT),
            NativeInput::scan(true, SCAN_CODE_A),
            NativeInput::scan(false, SCAN_CODE_A),
            NativeInput::key(false, VK_SHIFT),
        ]
    );
    // The boundary keeps its own layout: typed after a switch back.
    assert_eq!(sent[2][0].scan_code, SCAN_CODE_SEMICOLON);
    assert_eq!(sent.len(), 3);
    assert_eq!(api.requests(), [RUSSIAN_HKL, ENGLISH_HKL, RUSSIAN_HKL]);
}

#[test]
fn the_layout_settles_before_the_keys_of_a_new_layout() {
    let api = FakeWindowsAPI::plain();
    let clock = FakeClock::stepping(0.0);
    let subject = backend_with(&api, clock.clone(), script(|_| {}));
    let strokes = [key_event()];
    subject.inject_correction(injection(&strokes, 1)).unwrap();
    assert_eq!(*clock.sleeps.lock().unwrap(), [WINDOWS_LAYOUT_SETTLE_SECONDS]);
    // No switch, no pause: the deletion and the replacement travel in one call.
    clock.sleeps.lock().unwrap().clear();
    api.state().sent.clear();
    subject.inject_correction(injection(&strokes, 1)).unwrap();
    assert!(clock.sleeps.lock().unwrap().is_empty());
    let sent = api.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0][0], NativeInput::key(true, VK_BACK));
    assert_eq!(sent[0][2], NativeInput::scan(true, SCAN_CODE_A));
}

#[test]
fn a_boundary_can_stay_in_the_target_and_empty_batches_are_skipped() {
    let api = FakeWindowsAPI::plain();
    let subject = backend(&api);
    let boundary = KeyEvent { character: " ".to_owned(), characters: vec![" ".to_owned(), " ".to_owned()], ..key_event() };
    subject.inject_correction(injection(&[], 0)).unwrap();
    assert!(api.sent().is_empty());
    let strokes = [key_event()];
    subject.inject_correction(Injection { boundary: Some(&boundary), ..injection(&strokes, 1) }).unwrap();
    assert_eq!(api.requests(), [RUSSIAN_HKL]);
    // The deletion, then the keys typed in the layout switched to.
    let sent = api.sent();
    assert_eq!(sent.len(), 2);
    let scans: Vec<u32> = sent.iter().flatten().map(|item| item.scan_code).filter(|&code| code != 0).collect();
    assert_eq!(scans, [SCAN_CODE_A; 4]);
}

#[test]
fn a_kept_tail_is_typed_back_in_the_source_layout_where_the_layout_stays() {
    let api = FakeWindowsAPI::plain();
    let subject = backend(&api);
    let space = key_with(SCAN_CODE_A, " ", [" ", " "], 1);
    let word = KeyEvent { group: 1, ..key_event() };
    let strokes = [word.clone(), space.clone(), word];
    let kept = |kept_tail, source_group| Injection {
        boundary: Some(&space),
        source_group,
        kept_tail,
        ..injection(&strokes, 0)
    };
    subject.inject_correction(kept(1, Some(1))).unwrap();
    // Deletion and the converted keys in English, then the kept word and the space in Russian.
    assert_eq!(api.state().current_layout, RUSSIAN_HKL);
    let sent = api.sent();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].iter().filter(|item| item.virtual_key == VK_BACK).count(), 8);
    let scans: Vec<usize> = sent.iter().map(|batch| batch.iter().filter(|item| item.scan_code != 0).count()).collect();
    assert_eq!(scans, [4, 4]);
    for refused in [kept(strokes.len(), Some(1)), kept(1, None)] {
        assert!(message(&subject.inject_correction(refused).unwrap_err()).contains("сохраняемого слова"));
    }
}

#[test]
fn erased_says_how_many_characters_a_typo_correction_deletes() {
    let api = FakeWindowsAPI::plain();
    let subject = backend(&api);
    api.state().current_layout = RUSSIAN_HKL;
    let strokes = [key_event()];
    subject.inject_correction(Injection { erased: Some(3), ..injection(&strokes, 1) }).unwrap();
    assert_eq!(api.sent()[0].iter().filter(|item| item.virtual_key == VK_BACK).count(), 6);
}

#[test]
fn invalid_groups_a_partial_send_and_a_rejected_switch_fail_loudly() {
    let api = FakeWindowsAPI::plain();
    let subject = backend(&api);
    let error = subject.inject_correction(injection(&[], 2)).unwrap_err();
    assert!(message(&error).contains("Неизвестная группа"));
    let strokes = [KeyEvent { group: 7, ..key_event() }];
    let error = subject.inject_correction(injection(&strokes, 1)).unwrap_err();
    assert!(message(&error).contains("исходная группа"));

    let strokes = [key_event()];
    api.state().send_count = Some(0);
    let error = subject.inject_correction(injection(&strokes, 1)).unwrap_err();
    assert_eq!(
        message(&error),
        "SendInput отправил 0 из 2 событий; результат текста неизвестен; возможно, целевое приложение запущено с правами администратора"
    );

    api.state().send_count = None;
    // The layout now switches before anything is sent, so the refused switch needs a layout that still has to change.
    api.state().current_layout = ENGLISH_HKL;
    api.state().accept_switch = false;
    let error = subject.inject_correction(injection(&strokes, 1)).unwrap_err();
    assert!(message(&error).contains("отклонило"));
}

#[test]
fn literal_keys_of_two_layouts_are_refused() {
    let api = FakeWindowsAPI::plain();
    let subject = backend(&api);
    let first = key_with(SCAN_CODE_A, " ", [" ", " "], 0);
    let second = key_with(SCAN_CODE_A, " ", [" ", " "], 1);
    let trailing = [first];
    let strokes = [key_event()];
    let error = subject
        .inject_correction(Injection { boundary: Some(&second), trailing: &trailing, ..injection(&strokes, 1) })
        .unwrap_err();
    assert!(message(&error).contains("раскладка пунктуации"));
    assert!(api.sent().is_empty());
}

#[test]
fn late_and_held_keys_are_deleted_and_typed_again_as_the_users_own() {
    let api = FakeWindowsAPI::plain();
    let subject = backend(&api);
    let delivered = collected();
    subject.start(collecting(&delivered)).unwrap();
    let hook = api.hook();
    subject.hold_input();
    // Held keys are swallowed and never reach the engine directly.
    assert!(hook(native(true, VK_D, HELD_KEY_SCAN_CODE, false, 10)));
    assert!(hook(native(false, VK_D, HELD_KEY_SCAN_CODE, false, 11)));
    // The backend's own injection is never held.
    assert!(!hook(native(true, VK_X, INJECTED_KEY_SCAN_CODE, true, 12)));
    assert_eq!(delivered.lock().unwrap().iter().map(|event| event.synthetic).collect::<Vec<_>>(), [true]);

    let late = [KeyEvent { keycode: LATE_KEY_SCAN_CODE, ..key_event() }];
    let strokes = [key_event()];
    let held = subject
        .inject_correction(Injection { source_group: Some(0), late: &late, ..injection(&strokes, 1) })
        .unwrap();
    assert_eq!(held, 2);
    assert_eq!(api.requests(), [RUSSIAN_HKL]);
    let sent = api.sent();
    let batch: Vec<NativeInput> = sent[0].iter().chain(&sent[1]).copied().collect();
    // Two characters to delete: the word and the late key after it.
    assert_eq!(batch.iter().filter(|item| item.virtual_key == VK_BACK).count(), 4);
    let scans: Vec<u32> = batch.iter().map(|item| item.scan_code).filter(|&code| code != 0).collect();
    assert_eq!(scans, [SCAN_CODE_A; 2]);
    assert!(batch.iter().all(|item| item.synthetic));
    assert_eq!(sent[2], [replayed_scan(true, LATE_KEY_SCAN_CODE), replayed_scan(false, LATE_KEY_SCAN_CODE)]);
    let held_sent: Vec<NativeInput> = sent[3..].iter().flatten().copied().collect();
    assert_eq!(held_sent, [replayed_key(true, VK_D, HELD_KEY_SCAN_CODE), replayed_key(false, VK_D, HELD_KEY_SCAN_CODE)]);
    // The hold is over: the next key is delivered as before.
    assert!(!hook(native(true, u32::from(b'A'), SCAN_CODE_A, false, 13)));
    assert_eq!(delivered.lock().unwrap().len(), 2);
    subject.stop();
}

#[test]
fn a_failed_injection_still_types_the_held_keys_again() {
    let api = FakeWindowsAPI::plain();
    let subject = backend(&api);
    subject.start(collecting(&collected())).unwrap();
    let hook = api.hook();
    let late = [KeyEvent { keycode: LATE_KEY_SCAN_CODE, ..key_event() }];
    let strokes = [key_event()];
    let with_late = || Injection { late: &late, ..injection(&strokes, 1) };

    // The switch is refused before anything was deleted: the late key is still on screen and must not be typed twice; the
    // held one is typed.
    subject.hold_input();
    assert!(hook(native(true, VK_D, HELD_KEY_SCAN_CODE, false, 10)));
    api.state().accept_switch = false;
    assert!(message(&subject.inject_correction(with_late()).unwrap_err()).contains("отклонило"));
    assert_eq!(api.sent(), [[replayed_key(true, VK_D, HELD_KEY_SCAN_CODE)]]);
    assert!(!subject.inner.hold.lock().unwrap().holding);

    // The batch fails after the switch: nothing was deleted either, and a restore that fails too keeps the original error.
    api.state().accept_switch = true;
    api.state().sent.clear();
    subject.hold_input();
    assert!(hook(native(false, VK_D, HELD_KEY_SCAN_CODE, false, 11)));
    api.state().send_count = Some(0);
    assert!(message(&subject.inject_correction(with_late()).unwrap_err()).contains("SendInput"));
    assert_eq!(api.sent().len(), 2);
    assert!(!subject.inner.hold.lock().unwrap().holding);

    // The boundary switch fails after the batch: the late key was deleted, so it is typed again together with the held keys.
    {
        let mut state = api.state();
        state.send_count = None;
        state.sent.clear();
        state.current_layout = ENGLISH_HKL;
        state.requests.clear();
        state.refuse_after_first_request = Some(ENGLISH_HKL);
    }
    subject.inner.hold.lock().unwrap().held.clear();
    let boundary = key_with(SCAN_CODE_SEMICOLON, ";", [";", "ж"], 0);
    subject.hold_input();
    let error = subject
        .inject_correction(Injection {
            boundary: Some(&boundary),
            source_group: Some(0),
            late: &late,
            ..injection(&strokes, 1)
        })
        .unwrap_err();
    assert!(message(&error).contains("отклонило"));
    assert_eq!(api.sent().last().unwrap(), &[replayed_scan(true, LATE_KEY_SCAN_CODE), replayed_scan(false, LATE_KEY_SCAN_CODE)]);
    subject.stop();
}

#[test]
fn keys_other_programs_inject_are_marked_and_replayed_as_theirs() {
    let api = FakeWindowsAPI::plain();
    let subject = backend(&api);
    let delivered = collected();
    subject.start(collecting(&delivered)).unwrap();
    let hook = api.hook();
    let foreign = |event: NativeKeyEvent| NativeKeyEvent { foreign: true, ..event };

    // Remote control types a character, not a key: it is marked as foreign and nothing waits for its release.
    assert!(!hook(foreign(native(true, VK_PACKET, PACKET_CHARACTER_CODE, false, 10))));
    {
        let delivered = delivered.lock().unwrap();
        let event = delivered.last().unwrap();
        assert_eq!(event.key_name, UNICODE_PACKET_KEY_NAME);
        assert!(event.foreign);
    }
    assert!(!subject.inner.hook.lock().unwrap().pressed.contains(VK_PACKET));
    assert!(!hook(foreign(native(false, VK_PACKET, PACKET_CHARACTER_CODE, false, 11))));

    // Held behind a correction, both come back in order, each marked as what it was.
    subject.hold_input();
    assert!(hook(foreign(native(true, VK_PACKET, PACKET_CHARACTER_CODE, false, 10))));
    assert!(hook(foreign(native(true, VK_D, HELD_KEY_SCAN_CODE, false, 10))));
    subject.release_input().unwrap();
    assert_eq!(
        api.sent(),
        [
            [NativeInput {
                virtual_key: 0,
                unicode: true,
                foreign: true,
                ..replayed_key(true, 0, PACKET_CHARACTER_CODE)
            }],
            [NativeInput { foreign: true, ..replayed_key(true, VK_D, HELD_KEY_SCAN_CODE) }],
        ]
    );
    assert!(!subject.inner.hold.lock().unwrap().holding);
    subject.stop();
}

#[test]
fn a_switch_timeout_and_an_already_selected_group() {
    let api = FakeWindowsAPI::plain();
    let clock = FakeClock::stepping(0.0);
    let subject = backend_with(&api, clock.clone(), script(|_| {}));
    subject.inner.switch_group_inner(0).unwrap();
    assert!(api.requests().is_empty());

    api.state().apply_switch = false;
    clock.readings.lock().unwrap().extend([0.0, 0.6, 1.6]);
    let error = subject.inner.switch_group_inner(1).unwrap_err();
    // Half a second is no longer the end of the wait: a busy editor answers later.
    assert_eq!(*clock.sleeps.lock().unwrap(), [LAYOUT_SWITCH_POLL_SECONDS]);
    // The request stays posted: the engine books the layout as its own if it comes.
    assert_eq!(
        error,
        BackendError::LayoutSwitchUnconfirmed {
            message: "Приложение не подтвердило смену раскладки".to_owned(),
            group: 1,
        }
    );
}

// ----------------------------------------------------------------------------------- a layout chosen in the tray menu

/// KeySwitch's own window in front, as after a click on its tray icon, with the window the user worked in noted before.
fn behind_the_tray_menu(api: &Arc<FakeWindowsAPI>, subject: &WindowsBackend) {
    let own = api.state().process_id;
    let _ = subject.focused_window();
    let mut state = api.state();
    state.window_owners.insert(KEYSWITCH_WINDOW_HWND, own);
    state.foreground = KEYSWITCH_WINDOW_HWND;
}

#[test]
fn a_layout_chosen_in_the_tray_menu_switches_the_window_the_user_worked_in() {
    let api = FakeWindowsAPI::plain();
    let clock = FakeClock::stepping(0.0);
    let subject = backend_with(&api, clock.clone(), script(|_| {}));
    behind_the_tray_menu(&api, &subject);
    // KeySwitch's own window is no window the user works in: the one before it stays noted.
    assert_eq!(subject.focused_window().map(|focus| focus.own), Some(true));
    api.state().foreground_after_activation = Some(2);
    subject.aim_next_switch_at_work_window(true);
    subject.switch_group(1).unwrap();
    let state = api.state();
    assert_eq!(state.activated_windows, [WINDOWS_FAKE_HWND]);
    assert_eq!(state.foreground, WINDOWS_FAKE_HWND);
    assert_eq!(state.requests, [RUSSIAN_HKL]);
    drop(state);
    // The switch waited for the window to come to the front before it asked for the layout.
    assert_eq!(*clock.sleeps.lock().unwrap(), [LAYOUT_SWITCH_POLL_SECONDS, LAYOUT_SWITCH_POLL_SECONDS]);

    // The aim is spent: the next switch is a key's, and goes to the window in front.
    api.state().foreground = KEYSWITCH_WINDOW_HWND;
    subject.switch_group(0).unwrap();
    assert_eq!(api.state().activated_windows, [WINDOWS_FAKE_HWND]);
    assert_eq!(api.requests(), [RUSSIAN_HKL, ENGLISH_HKL]);
}

#[test]
fn the_shell_taskbar_is_no_window_the_user_worked_in() {
    let api = FakeWindowsAPI::with_clipboard();
    let subject = backend(&api);
    {
        let mut state = api.state();
        state.window_owners.insert(TASKBAR_HWND, EXPLORER_PROCESS_ID);
        state.window_classes.insert(TASKBAR_HWND, SHELL_TRAY_WINDOW_CLASSES[0].to_owned());
        state.foreground_after_activation = Some(0);
    }
    // The press on the tray icon comes while the window the user typed in is in front; the taskbar takes the front
    // after it, and nothing in front with no window is noted either.
    assert!(!subject.handle_native(NativeKeyEvent::new(true, 0, 0, false, false, 1)));
    for front in [TASKBAR_HWND, 0] {
        api.state().foreground = front;
        assert!(!subject.handle_native(NativeKeyEvent::new(true, 0, 0, false, false, 2)));
    }
    api.state().foreground = TASKBAR_HWND;
    subject.aim_next_switch_at_work_window(true);
    subject.switch_group(1).unwrap();
    assert_eq!(api.state().activated_windows, [WINDOWS_FAKE_HWND]);
    assert_eq!(api.requests(), [RUSSIAN_HKL]);

    // With the window the user works in already in front, nothing is brought forward.
    subject.aim_next_switch_at_work_window(true);
    subject.switch_group(0).unwrap();
    assert_eq!(api.state().activated_windows, [WINDOWS_FAKE_HWND]);
    assert_eq!(api.requests(), [RUSSIAN_HKL, ENGLISH_HKL]);
}

#[test]
fn while_the_taskbar_is_in_front_the_layout_told_is_the_work_windows() {
    let api = FakeWindowsAPI::with_clipboard();
    let subject = backend(&api);
    let events = collected();
    subject.set_listener(Some(collecting(&events)));
    {
        let mut state = api.state();
        state.window_owners.insert(TASKBAR_HWND, EXPLORER_PROCESS_ID);
        state.window_classes.insert(TASKBAR_HWND, SHELL_TRAY_WINDOW_CLASSES[0].to_owned());
        state.foreground = TASKBAR_HWND;
    }
    // Before a window was worked in, the taskbar's own layout is told.
    assert_eq!(subject.current_group().unwrap(), 0);
    api.state().foreground = WINDOWS_FAKE_HWND;
    let _ = subject.focused_window();
    {
        let mut state = api.state();
        state.window_layouts.insert(WINDOWS_FAKE_HWND, RUSSIAN_HKL);
        state.foreground = TASKBAR_HWND;
    }
    // The taskbar keeps English; the window the user typed in has Russian, and the engine and the keys are told so.
    assert_eq!(subject.current_group().unwrap(), 1);
    assert!(!subject.handle_native(native(true, VK_A, SCAN_CODE_A, false, 1)));
    assert_eq!(events.lock().unwrap().last().map(|event| event.group), Some(1));
    // KeySwitch's own window in front tells its own layout: the engine leaves it out by itself.
    let own = api.state().process_id;
    api.state().window_owners.insert(KEYSWITCH_WINDOW_HWND, own);
    api.state().foreground = KEYSWITCH_WINDOW_HWND;
    assert_eq!(subject.current_group().unwrap(), 0);
}

#[test]
fn without_a_window_to_return_to_the_menu_switch_fails_where_it_is() {
    let api = FakeWindowsAPI::plain();
    let clock = FakeClock::stepping(0.0);
    let subject = backend_with(&api, clock.clone(), script(|_| {}));
    let own = api.state().process_id;
    api.state().window_owners.insert(WINDOWS_FAKE_HWND, own);
    let _ = subject.focused_window();
    api.state().apply_switch = false;
    clock.readings.lock().unwrap().extend([0.0, 1.6]);
    subject.aim_next_switch_at_work_window(true);
    let error = subject.switch_group(1).unwrap_err();
    assert!(matches!(error, BackendError::LayoutSwitchUnconfirmed { group: 1, .. }));
    assert!(api.state().activated_windows.is_empty());

    // An aim the engine did not take is taken back, and the switch after it goes where the keys go.
    subject.aim_next_switch_at_work_window(true);
    subject.aim_next_switch_at_work_window(false);
    api.state().apply_switch = true;
    subject.switch_group(1).unwrap();
    assert!(api.state().activated_windows.is_empty());
}

#[test]
fn a_window_that_will_not_come_back_is_waited_for_a_bounded_time() {
    let api = FakeWindowsAPI::plain();
    let clock = FakeClock::stepping(0.0);
    let subject = backend_with(&api, clock.clone(), script(|_| {}));
    behind_the_tray_menu(&api, &subject);
    // Refused: no wait at all.
    api.state().activation_refused = true;
    subject.aim_next_switch_at_work_window(true);
    subject.switch_group(1).unwrap();
    assert!(clock.sleeps.lock().unwrap().is_empty());

    // Accepted but never in front: the wait ends at its deadline, and the switch goes to the window in front.
    api.state().activation_refused = false;
    api.state().current_layout = ENGLISH_HKL;
    clock.readings.lock().unwrap().extend([0.0, 0.5, 1.5]);
    subject.aim_next_switch_at_work_window(true);
    subject.switch_group(1).unwrap();
    assert_eq!(*clock.sleeps.lock().unwrap(), [LAYOUT_SWITCH_POLL_SECONDS]);
    assert_eq!(api.state().foreground, KEYSWITCH_WINDOW_HWND);

    // In front but without a focused control yet: waited for too, as far as the deadline.
    clock.sleeps.lock().unwrap().clear();
    {
        let mut state = api.state();
        state.current_layout = ENGLISH_HKL;
        state.foreground = KEYSWITCH_WINDOW_HWND;
        state.foreground_after_activation = Some(0);
        state.no_control = true;
    }
    clock.readings.lock().unwrap().extend([0.0, 0.5, 1.5]);
    subject.aim_next_switch_at_work_window(true);
    subject.switch_group(1).unwrap();
    assert_eq!(api.state().foreground, WINDOWS_FAKE_HWND);
    assert_eq!(*clock.sleeps.lock().unwrap(), [LAYOUT_SWITCH_POLL_SECONDS]);
}

// --------------------------------------------------------------------------------------------- test_input_integrity.py

#[test]
fn swallowed_enter_repeats_never_reach_the_chat() {
    let api = FakeWindowsAPI::plain();
    let subject = backend(&api);
    subject.set_key_filter(Some(Arc::new(|event: &KeyEvent| {
        if event.key_name == "Return" { KeyDisposition::Consume } else { KeyDisposition::Pass }
    })));
    let event = native(true, VK_RETURN, SCAN_CODE_ENTER, false, 1);
    assert!(subject.handle_native(event));
    subject.set_key_filter(None); // the prompt already confirmed on the first press
    for _ in 0..5 {
        assert!(subject.handle_native(event));
    }
    assert!(!subject.handle_native(NativeKeyEvent { injected: true, ..event }));
    assert!(subject.handle_native(NativeKeyEvent { pressed: false, ..event }));
    assert!(!subject.handle_native(event));
}

#[test]
fn replayed_keys_reenter_the_hook_without_being_held_again() {
    let api = FakeWindowsAPI::plain();
    let subject = backend(&api);
    let delivered = collected();
    subject.set_listener(Some(collecting(&delivered)));
    // The native hook is called synchronously by SendInput.
    let inner = Arc::clone(&subject.inner);
    api.state().on_send = Some(Arc::new(move |inputs: &[NativeInput]| {
        for item in inputs {
            inner.handle_native(NativeKeyEvent {
                replayed: !item.synthetic,
                ..NativeKeyEvent::new(
                    item.pressed,
                    if item.virtual_key != 0 { item.virtual_key } else { VK_A },
                    if item.scan_code != 0 { item.scan_code } else { SCAN_CODE_A },
                    item.extended,
                    item.synthetic,
                    1,
                )
            });
        }
    }));
    subject.hold_input();
    assert!(subject.handle_native(native(true, VK_D, HELD_KEY_SCAN_CODE, false, 1)));
    let late = [KeyEvent { keycode: LATE_KEY_SCAN_CODE, ..key_event() }];
    let strokes = [key_event()];
    subject.inject_correction(Injection { late: &late, ..injection(&strokes, 1) }).unwrap();
    assert!(!subject.inner.hold.lock().unwrap().holding);
    assert!(subject.inner.hold.lock().unwrap().held.is_empty());
    let typed: Vec<u32> =
        delivered.lock().unwrap().iter().filter(|event| !event.synthetic).map(|event| event.keycode).collect();
    assert_eq!(typed, [LATE_KEY_SCAN_CODE, LATE_KEY_SCAN_CODE, HELD_KEY_SCAN_CODE]);
}

#[test]
fn a_validation_error_and_a_pointer_during_a_layout_switch_release_input() {
    let api = FakeWindowsAPI::plain();
    let subject = backend(&api);
    subject.hold_input();
    let error = subject.inject_correction(injection(&[], 8)).unwrap_err();
    assert!(message(&error).contains("Неизвестная группа"));
    assert!(!subject.inner.hold.lock().unwrap().holding);
    let inner = Arc::clone(&subject.inner);
    api.state().on_request = Some(Arc::new(move |_layout| {
        inner.handle_native(NativeKeyEvent::new(true, 0, 0, false, false, 1));
    }));
    subject.hold_input();
    let strokes = [key_event()];
    let error = subject.inject_correction(injection(&strokes, 1)).unwrap_err();
    assert!(message(&error).contains("Место ввода изменилось"));
    assert!(api.sent().is_empty());
    assert!(!subject.inner.hold.lock().unwrap().holding);
}

#[test]
fn caps_state_changes_only_replay_case_not_punctuation() {
    let subject = backend(&FakeWindowsAPI::plain());
    for caps_now in [false, true] {
        subject.inner.caps_lock.store(caps_now, Ordering::Relaxed);
        for caps_then in [false, true] {
            for shift_then in [false, true] {
                let state = if caps_then { LOCK_MASK } else { 0 } | if shift_then { SHIFT_MASK } else { 0 };
                let letter = KeyEvent { state, ..key_event() };
                let events = subject.inner.stroke_inputs(&letter, true, Some(1));
                let shifted = events.iter().any(|item| item.virtual_key == VK_SHIFT);
                assert_eq!(shifted, shift_then ^ caps_then ^ caps_now);
                let punctuation = KeyEvent { characters: vec![".".to_owned(), ".".to_owned()], ..letter };
                let events = subject.inner.stroke_inputs(&punctuation, true, Some(1));
                assert_eq!(events.iter().any(|item| item.virtual_key == VK_SHIFT), shift_then);
            }
        }
    }
}

#[test]
fn pointer_events_are_observed_without_being_suppressed() {
    let subject = backend(&FakeWindowsAPI::plain());
    let events = collected();
    subject.set_listener(Some(collecting(&events)));
    subject.hold_input();
    assert!(!subject.handle_native(NativeKeyEvent::new(true, 0, 0, false, false, 1)));
    let event = events.lock().unwrap()[0].clone();
    assert_eq!(event.key_name, "Pointer");
    assert_eq!(event.group, POINTER_EVENT_GROUP);
    assert_eq!(event.characters, ["", ""]);
    assert_eq!(subject.release_input().unwrap(), 0);
}

#[test]
fn a_turned_wheel_is_reported_as_a_turn_and_moves_no_pointer_epoch() {
    let subject = backend(&FakeWindowsAPI::plain());
    let events = collected();
    // No listener yet: the turn goes nowhere.
    assert!(!subject.handle_native(NativeKeyEvent { wheel: true, ..NativeKeyEvent::new(false, 0, 0, false, false, 7) }));
    subject.set_listener(Some(collecting(&events)));
    let epoch = subject.inner.pointer_epoch.load(Ordering::Relaxed);
    assert!(!subject.handle_native(NativeKeyEvent { wheel: true, ..NativeKeyEvent::new(false, 0, 0, false, false, 7) }));
    let event = events.lock().unwrap()[0].clone();
    assert_eq!((event.key_name.as_str(), event.pressed, event.timestamp, event.group), ("Wheel", false, 7, POINTER_EVENT_GROUP));
    assert_eq!(subject.inner.pointer_epoch.load(Ordering::Relaxed), epoch);
}

// --------------------------------------------------------------------------------- the withheld Enter (action barrier)

fn deferring_backend(api: &Arc<FakeWindowsAPI>) -> WindowsBackend {
    let subject = backend(api);
    subject.set_key_filter(Some(Arc::new(|event: &KeyEvent| {
        if event.key_name == "Return" && event.state == 0 { KeyDisposition::Defer } else { KeyDisposition::Pass }
    })));
    subject
}

const ENTER_DOWN: NativeKeyEvent = NativeKeyEvent::new(true, VK_RETURN, SCAN_CODE_ENTER, false, false, 1);
const ENTER_UP: NativeKeyEvent = NativeKeyEvent::new(false, VK_RETURN, SCAN_CODE_ENTER, false, false, 1);

#[test]
fn a_withheld_enter_goes_once_before_what_was_typed_after_it() {
    let api = FakeWindowsAPI::plain();
    let subject = deferring_backend(&api);
    let events = collected();
    subject.set_listener(Some(collecting(&events)));
    assert!(subject.handle_native(ENTER_DOWN));
    assert!(events.lock().unwrap().last().unwrap().deferred);
    // Its own release passes the hold, and is hidden from the window with the press.
    assert!(subject.handle_native(ENTER_UP));
    assert!(subject.handle_native(native(true, VK_D, HELD_KEY_SCAN_CODE, false, 2)));
    // Nothing of what is held goes while the action waits.
    assert_eq!(subject.release_input().unwrap(), 0);
    assert!(api.sent().is_empty());

    assert_eq!(subject.complete_action(true).unwrap(), COMPLETED_ACTION_EVENT_COUNT);
    let enter = |pressed| NativeInput { virtual_key: VK_RETURN, scan_code: SCAN_CODE_ENTER, ..NativeInput::new(pressed) };
    assert_eq!(api.sent(), [vec![enter(true), enter(false)], vec![replayed_key(true, VK_D, HELD_KEY_SCAN_CODE)]]);
    assert!(!subject.inner.hold.lock().unwrap().holding);
    assert_eq!(subject.complete_action(true).unwrap(), 0);
}

#[test]
fn keypad_enter_preserves_its_extended_flag() {
    let api = FakeWindowsAPI::plain();
    let subject = deferring_backend(&api);
    let _ = subject.handle_native(NativeKeyEvent { extended: true, ..ENTER_DOWN });
    subject.complete_action(true).unwrap();
    assert!(api.sent()[0].iter().all(|item| item.extended && item.virtual_key == VK_RETURN));
}

#[test]
fn shift_enter_is_not_intercepted() {
    let api = FakeWindowsAPI::plain();
    let subject = deferring_backend(&api);
    assert!(!subject.handle_native(native(true, VK_SHIFT, SCAN_CODE_LEFT_SHIFT, false, 1)));
    assert!(!subject.handle_native(ENTER_DOWN));
    assert!(subject.inner.hold.lock().unwrap().deferred_action.is_none());
}

#[test]
fn a_cancelled_action_sends_nothing_and_lets_go() {
    let api = FakeWindowsAPI::plain();
    let subject = deferring_backend(&api);
    let _ = subject.handle_native(ENTER_DOWN);
    assert_eq!(subject.complete_action(false).unwrap(), 0);
    assert!(api.sent().is_empty());
    assert!(!subject.inner.hold.lock().unwrap().holding);
}

#[test]
fn pointer_and_focus_change_cancel_without_submitting_to_another_field() {
    for pointer in [false, true] {
        let api = FakeWindowsAPI::plain();
        let subject = deferring_backend(&api);
        let _ = subject.handle_native(ENTER_DOWN);
        if pointer {
            let _ = subject.handle_native(NativeKeyEvent::new(true, 0, 0, false, false, 1));
        } else {
            api.state().foreground += 1;
        }
        let error = subject.complete_action(true).unwrap_err();
        assert_eq!(message(&error), "Enter/Tab не передан: место ввода изменилось");
        assert!(api.sent().is_empty());
        assert!(!subject.inner.hold.lock().unwrap().holding);
        assert!(subject.inner.hold.lock().unwrap().deferred_action.is_none());
    }
}

#[test]
fn a_partial_action_send_is_reported_without_a_duplicate_retry() {
    let api = FakeWindowsAPI::plain();
    let subject = deferring_backend(&api);
    let _ = subject.handle_native(ENTER_DOWN);
    api.state().send_count = Some(1);
    let error = subject.complete_action(true).unwrap_err();
    assert!(message(&error).contains("1 из 2"));
    assert_eq!(api.sent().len(), 1);
    assert!(!subject.inner.hold.lock().unwrap().holding);
}

#[test]
fn a_letter_autorepeat_after_enter_retains_its_balancing_release() {
    let api = FakeWindowsAPI::plain();
    let subject = deferring_backend(&api);
    let letter = native(true, VK_D, HELD_KEY_SCAN_CODE, false, 1);
    let letter_up = NativeKeyEvent { pressed: false, ..letter };
    assert!(!subject.handle_native(letter));
    assert!(subject.handle_native(ENTER_DOWN));
    assert!(subject.handle_native(letter));
    assert!(subject.handle_native(ENTER_UP));
    // The release of the key that was down before Enter goes to the window, and one more follows the held repeat.
    assert!(!subject.handle_native(letter_up));
    assert!(subject.inner.hook.lock().unwrap().pressed.0.is_empty());
    subject.complete_action(true).unwrap();
    let sent = api.sent();
    assert_eq!(
        sent[1..],
        [vec![replayed_key(true, VK_D, HELD_KEY_SCAN_CODE)], vec![replayed_key(false, VK_D, HELD_KEY_SCAN_CODE)]]
    );
}

#[test]
fn a_replayed_second_enter_keeps_what_follows_it_until_it_is_completed() {
    let api = FakeWindowsAPI::plain();
    let subject = deferring_backend(&api);
    let inner = Arc::clone(&subject.inner);
    // The hook is called back by SendInput, on this thread here: a lock held across the send would never come back.
    api.state().on_send = Some(Arc::new(move |inputs: &[NativeInput]| {
        for item in inputs {
            inner.handle_native(NativeKeyEvent {
                replayed: item.replayed,
                ..NativeKeyEvent::new(item.pressed, item.virtual_key, item.scan_code, item.extended, item.synthetic, 1)
            });
        }
    }));
    let h_down = native(true, VK_D, HELD_KEY_SCAN_CODE, false, 2);
    assert!(subject.handle_native(ENTER_DOWN));
    assert!(subject.handle_native(ENTER_UP));
    for event in [h_down, NativeKeyEvent { pressed: false, ..h_down }, ENTER_DOWN, ENTER_UP, h_down] {
        assert!(subject.handle_native(event));
    }
    assert_eq!(subject.complete_action(true).unwrap(), COMPLETED_ACTION_EVENT_COUNT);
    // The first Enter, the keys typed before the second, and the second Enter defers again. Its own release comes along, as
    // a key that was down when it was pressed: only what was typed after it stays held.
    assert_eq!(api.sent().len(), 5);
    assert!(subject.inner.hold.lock().unwrap().deferred_action.is_some());
    assert!(subject.inner.hold.lock().unwrap().holding);
    assert_eq!(subject.inner.hold.lock().unwrap().held.len(), 1);

    assert_eq!(subject.complete_action(true).unwrap(), COMPLETED_ACTION_EVENT_COUNT);
    assert!(subject.inner.hold.lock().unwrap().held.is_empty());
    assert!(!subject.inner.hold.lock().unwrap().holding);
    let sent = api.sent();
    assert_eq!(sent[4], [replayed_key(false, VK_RETURN, SCAN_CODE_ENTER)]);
    assert_eq!(sent[5].len(), 2);
    assert!(sent[5].iter().all(|item| item.virtual_key == VK_RETURN && item.synthetic));
    assert_eq!(sent[6], [replayed_key(true, VK_D, HELD_KEY_SCAN_CODE)]);
}

/// `SendInput` returns only after the hook, on its own thread, has answered every event. If the engine's thread held any
/// lock the hook takes across the send, this would stop for good.
#[test]
fn the_hook_on_its_own_thread_never_waits_for_a_send_in_progress() {
    let api = FakeWindowsAPI::plain();
    let subject = Arc::new(backend(&api));
    let delivered = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&delivered);
    subject.set_listener(Some(Arc::new(move |_event| {
        counter.fetch_add(1, Ordering::Relaxed);
    })));
    let (to_hook, from_engine) = mpsc::channel::<(NativeKeyEvent, mpsc::Sender<bool>)>();
    let hook_inner = Arc::clone(&subject.inner);
    let hook_thread = thread::spawn(move || {
        while let Ok((event, answer)) = from_engine.recv() {
            let _ = answer.send(hook_inner.handle_native(event));
        }
    });
    let sender = Mutex::new(to_hook.clone());
    api.state().on_send = Some(Arc::new(move |inputs: &[NativeInput]| {
        for item in inputs {
            let (answer, answered) = mpsc::channel();
            let event = NativeKeyEvent {
                replayed: item.replayed,
                ..NativeKeyEvent::new(
                    item.pressed,
                    if item.virtual_key != 0 { item.virtual_key } else { VK_A },
                    if item.scan_code != 0 { item.scan_code } else { SCAN_CODE_A },
                    item.extended,
                    item.synthetic,
                    1,
                )
            };
            sender.lock().unwrap().send((event, answer)).unwrap();
            answered.recv().unwrap();
        }
    }));
    // The user keeps typing on the hook's thread while the engine corrects word after word. Not with no pause at all: the
    // replay of held keys ends when none is left, and a typist as fast as the replay itself would keep it going for good.
    let typing_stop = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&typing_stop);
    let typist = thread::spawn(move || {
        let mut count = 0u64;
        while !stop.load(Ordering::Relaxed) {
            let (answer, answered) = mpsc::channel();
            let pressed = count.is_multiple_of(2);
            if to_hook.send((native(pressed, VK_D, HELD_KEY_SCAN_CODE, false, 1), answer)).is_err() {
                return;
            }
            let _ = answered.recv();
            count += 1;
            thread::sleep(Duration::from_micros(200));
        }
    });
    let (finished, engine_done) = mpsc::channel();
    let engine = Arc::clone(&subject);
    let worker = thread::spawn(move || {
        let strokes = [key_event()];
        for round in 0..300 {
            engine.hold_input();
            engine.inject_correction(injection(&strokes, round % 2)).unwrap();
            let _ = engine.take_hook_gap();
        }
        finished.send(()).unwrap();
    });
    engine_done.recv_timeout(Duration::from_secs(60)).expect("the engine and the hook kept going");
    worker.join().unwrap();
    typing_stop.store(true, Ordering::Relaxed);
    typist.join().unwrap();
    api.state().on_send = None;
    drop(subject);
    drop(hook_thread);
    assert!(delivered.load(Ordering::Relaxed) > 0);
    assert!(!api.sent().is_empty());
}

// ------------------------------------------------------------------------------------------------- test_input_delay.py

#[test]
fn the_hook_measures_typed_keys_only() {
    let api = FakeWindowsAPI::plain();
    let subject = backend(&api);
    let key = native(true, VK_RETURN, SCAN_CODE_ENTER, false, HOOK_KEY_TIME_MS);
    api.state().ticks = HOOK_KEY_TIME_MS + INPUT_DELAY_LATE_MS;
    let _ = subject.handle_native(key);
    // KeySwitch's own keys, their replays and the pointer are no typing.
    let _ = subject.handle_native(NativeKeyEvent { pressed: false, injected: true, ..key });
    let _ = subject.handle_native(NativeKeyEvent { pressed: false, replayed: true, ..key });
    let _ = subject.handle_native(NativeKeyEvent::new(true, 0, 0, false, false, 0));
    // A key time ahead of the clock says nothing about a delay.
    let ahead = api.state().ticks + 1;
    let _ = subject.handle_native(NativeKeyEvent { pressed: false, timestamp: ahead, ..key });
    let report = subject.take_input_delay().unwrap();
    assert_eq!((report.keys, report.late_keys, report.worst_late_ms), (1, 1, INPUT_DELAY_LATE_MS));
    // The counter wraps after 49.7 days; a key stamped just before is still on time.
    api.state().ticks = 0;
    let _ = subject.handle_native(NativeKeyEvent { pressed: false, timestamp: DWORD_MASK, ..key });
    assert_eq!(
        subject.take_input_delay().unwrap(),
        InputDelayReport { keys: 1, worst_late_ms: 1, ..InputDelayReport::default() }
    );
    // A key another program sent with a time of zero, long after the computer started.
    api.state().ticks = INPUT_DELAY_UNSTAMPED_MS;
    let _ = subject.handle_native(NativeKeyEvent { timestamp: 0, ..key });
    assert_eq!(
        subject.take_input_delay().unwrap(),
        InputDelayReport { keys: 1, unstamped_keys: 1, ..InputDelayReport::default() }
    );
}

#[test]
fn the_hook_times_its_answer_and_counts_a_slow_one() {
    let api = FakeWindowsAPI::plain();
    let clock = FakeClock::stepping(0.0);
    let subject = backend_with(&api, clock.clone(), script(|_| {}));
    let seconds = INPUT_DELAY_SLOW_CALLBACK_MS.to::<f64>() / 1000.0;
    api.state().ticks = HOOK_KEY_TIME_MS;
    clock.perf.lock().unwrap().extend([0.0, seconds]);
    let _ = subject.handle_native(NativeKeyEvent { foreign: true, ..native(true, VK_RETURN, SCAN_CODE_ENTER, false, HOOK_KEY_TIME_MS) });
    assert_eq!(
        subject.take_input_delay().unwrap(),
        InputDelayReport {
            slow_callbacks: 1,
            worst_callback_ms: INPUT_DELAY_SLOW_CALLBACK_MS,
            foreign_keys: 1,
            ..InputDelayReport::default()
        }
    );
    clock.perf.lock().unwrap().extend([0.0, seconds]);
    let _ = subject.handle_native(native(false, VK_RETURN, SCAN_CODE_ENTER, false, HOOK_KEY_TIME_MS));
    let report = subject.take_input_delay().unwrap();
    assert_eq!((report.keys, report.slow_callbacks, report.worst_callback_ms), (1, 1, INPUT_DELAY_SLOW_CALLBACK_MS));
    // Half a millisecond rounds to the even number, as the Python's round does.
    clock.perf.lock().unwrap().extend([0.0, 0.0025]);
    let _ = subject.handle_native(native(false, VK_RETURN, SCAN_CODE_ENTER, false, HOOK_KEY_TIME_MS));
    assert_eq!(subject.take_input_delay().unwrap().worst_callback_ms, 2);
}

#[test]
fn the_hook_notes_the_gap_before_the_key_goes_on() {
    let api = FakeWindowsAPI::plain();
    let subject = backend(&api);
    let key = native(true, VK_RETURN, SCAN_CODE_ENTER, false, HOOK_KEY_TIME_MS);
    api.state().ticks = HOOK_KEY_TIME_MS + HOOK_GAP_MS;
    let seen: Arc<Mutex<Vec<Option<HookGap>>>> = Arc::default();
    let (noted, inner) = (Arc::clone(&seen), Arc::clone(&subject.inner));
    subject.set_listener(Some(Arc::new(move |_event| {
        noted.lock().unwrap().push(inner.input_delay.take_gap());
    })));
    let _ = subject.handle_native(key);
    let expected = HookGap { late_ms: HOOK_GAP_MS, callback_ms: 0 };
    assert_eq!(*seen.lock().unwrap(), [Some(expected)]);
    // A key time ahead of the clock, and another program's key, are not waits the hook can know.
    let ahead = api.state().ticks + 1;
    let _ = subject.handle_native(NativeKeyEvent { pressed: false, timestamp: ahead, ..key });
    let _ = subject.handle_native(NativeKeyEvent { timestamp: 0, foreign: true, ..key });
    assert_eq!(subject.take_hook_gap(), None);
}

#[test]
fn an_input_time_no_hook_got_is_certain_after_a_later_event_or_a_long_silence() {
    let api = FakeWindowsAPI::plain();
    let subject = backend(&api);
    let take = || subject.take_unseen_input().unwrap();
    // No hook event yet: an input before the hooks came says nothing.
    api.state().last_input = HOOK_KEY_TIME_MS;
    assert_eq!(take(), None);
    // Every input the hooks got.
    api.state().input_times = vec![HOOK_KEY_TIME_MS];
    assert_eq!(take(), None);
    // A key the hooks have not got yet: perhaps late, so nothing is said until a later event comes.
    {
        let mut state = api.state();
        state.last_input = HOOK_KEY_TIME_MS + 1;
        state.ticks = state.last_input;
    }
    assert_eq!(take(), None);
    assert_eq!(take(), None);
    // It came late after all: nothing passed the hooks.
    {
        let mut state = api.state();
        let last = state.last_input;
        state.input_times.push(last);
    }
    assert_eq!(take(), None);
    // One input the hooks never get between keys they get is no program keeping keys away.
    api.state().last_input = HOOK_KEY_TIME_MS + HOOK_GAP_MS;
    assert_eq!(take(), None);
    {
        let mut state = api.state();
        let next = state.last_input + 1;
        state.input_times.push(next);
        state.last_input += 1;
    }
    assert_eq!(take(), None);
    // Keys the hooks never get, and a pointer move they get after them.
    let first = api.state().last_input + HOOK_GAP_MS;
    for _ in 0..UNSEEN_TIMES {
        {
            let mut state = api.state();
            state.last_input += HOOK_GAP_MS;
            state.ticks = state.last_input;
        }
        assert_eq!(take(), None);
    }
    let answer = {
        let mut state = api.state();
        let next = state.last_input + 1;
        state.input_times.push(next);
        state.ticks = next;
        next - first
    };
    assert_eq!(take(), Some(answer));
    // Keys in a silent program and nothing else: certain once HOOK_SILENCE_MS went by.
    {
        let mut state = api.state();
        state.ticks = *state.input_times.last().unwrap() + 1;
        state.last_input = state.ticks;
    }
    assert_eq!(take(), None);
    for _ in 0..UNSEEN_TIMES - 1 {
        api.state().last_input += 1;
        assert_eq!(take(), None);
    }
    api.state().ticks += HOOK_SILENCE_MS - 1;
    assert_eq!(take(), None);
    api.state().ticks += 1;
    assert_eq!(take(), Some(HOOK_SILENCE_MS));
    // The same input is reported once, however long the silence lasts.
    api.state().ticks += HOOK_SILENCE_MS;
    assert_eq!(take(), None);
    assert_eq!(take(), None);
    // A lone input in a long silence is reported no more than one between keys the hooks got.
    {
        let mut state = api.state();
        state.last_input = state.ticks;
    }
    assert_eq!(take(), None);
    api.state().ticks += HOOK_SILENCE_MS;
    assert_eq!(take(), None);
}

#[test]
fn the_tick_clock_wraps_and_a_hook_restart_forgets_nothing_it_needs() {
    let api = FakeWindowsAPI::plain();
    let subject = backend(&api);
    let take = || subject.take_unseen_input().unwrap();
    api.state().input_times = vec![DWORD_MASK];
    api.state().last_input = 0;
    assert_eq!(take(), None);
    // The event times are gone (a new hook): only the silence decides, over inputs that kept coming.
    api.state().input_times.clear();
    for _ in 0..UNSEEN_TIMES - 1 {
        api.state().last_input += 1;
        assert_eq!(take(), None);
    }
    api.state().ticks = HOOK_SILENCE_MS;
    assert_eq!(take(), Some(HOOK_SILENCE_MS));
    // An input time older than the newest event got is no input the hooks missed after it.
    api.state().input_times = vec![HOOK_KEY_TIME_MS];
    api.state().last_input = HOOK_KEY_TIME_MS - 1;
    assert_eq!(take(), None);
    assert_eq!(subject.inner.unseen.lock().unwrap().unseen_input, None);
}

// ---------------------------------------------------------------------------------------- test_windows_selection.py

fn refusal(subject: &WindowsBackend) -> RefusalReason {
    match subject.read_selection("Notepad", SELECTION_FIXTURE_WINDOW) {
        Err(SelectionError::Refused(refused)) => refused.reason,
        other => panic!("not a refusal: {other:?}"),
    }
}

fn selected_by(source: &str, text: &str) -> SelectedText {
    SelectedText { text: text.to_owned(), source: source.to_owned() }
}

fn refusing(reason: RefusalReason) -> SelectionError {
    SelectionRefused::new(reason).into()
}

/// A provider that failed: a refusal it is not, and it tells no more than one that is missing.
fn provider_gone() -> SelectionError {
    failed("provider gone").into()
}

#[test]
fn ui_automation_answers_first_and_nothing_is_copied() {
    let api = FakeWindowsAPI::with_clipboard();
    let reader = script(|script| script.answer = Ok(Some(selected_by(SOURCE_UIA, "ghbdtn"))));
    let subject = reading_backend(&api, &reader);
    assert_eq!(subject.read_selection("Notepad", SELECTION_FIXTURE_WINDOW).unwrap(), selected_by(SOURCE_UIA, "ghbdtn"));
    subject.read_selection("Notepad", 0).unwrap();
    assert_eq!(reader.lock().unwrap().windows, [SELECTION_FIXTURE_WINDOW, WINDOWS_FAKE_HWND]);
    assert!(api.sent().is_empty());
}

#[test]
fn a_console_is_left_alone() {
    let api = FakeWindowsAPI::with_clipboard();
    api.state().window_classes.insert(WINDOWS_FAKE_HWND, "ConsoleWindowClass".to_owned());
    assert_eq!(refusal(&reading_backend(&api, &script(|_| {}))), RefusalReason::Terminal);
    api.state().window_classes.insert(WINDOWS_FAKE_HWND, "CASCADIA_HOSTING_WINDOW_CLASS".to_owned());
    assert_eq!(refusal(&reading_backend(&api, &script(|_| {}))), RefusalReason::Terminal);
    assert!(api.sent().is_empty());
}

#[test]
fn a_refusal_of_ui_automation_stands() {
    let api = FakeWindowsAPI::with_clipboard();
    let reader = script(|script| script.answer = Err(refusing(RefusalReason::ReadOnly)));
    assert_eq!(refusal(&reading_backend(&api, &reader)), RefusalReason::ReadOnly);
}

#[test]
fn without_ui_automation_the_selection_is_copied_and_the_clipboard_given_back() {
    for answer in [Ok(None), Err(provider_gone())] {
        let api = FakeWindowsAPI::with_clipboard();
        let reader = script(|script| script.answer = answer);
        let selected = reading_backend(&api, &reader).read_selection("Notepad", SELECTION_FIXTURE_WINDOW).unwrap();
        assert_eq!(selected, selected_by(SOURCE_CLIPBOARD, "ghbdtn"));
        assert_eq!(api.sent(), [copy_keys()]);
        assert_eq!(api.state().written, [saved_clipboard()]);
        assert_eq!(api.state().text_limit, SELECTION_COMMAND_MAX_CHARACTERS + 1);
    }
}

#[test]
fn a_reader_that_cannot_be_built_is_tried_again_next_time() {
    log::capture::install();
    let api = FakeWindowsAPI::with_clipboard();
    let reader = script(|script| script.unavailable = true);
    let subject = reading_backend(&api, &reader);
    subject.read_selection("Notepad", SELECTION_FIXTURE_WINDOW).unwrap();
    subject.read_selection("Notepad", SELECTION_FIXTURE_WINDOW).unwrap();
    assert_eq!(reader.lock().unwrap().attempts, 2);
    assert!(log::capture::contains(
        "keyswitch.windows_backend: UI Automation недоступна для чтения выделения, выделение копируется"
    ));
    {
        let mut script = reader.lock().unwrap();
        script.unavailable = false;
        script.answer = Ok(Some(selected_by(SOURCE_UIA, "x")));
    }
    subject.read_selection("Notepad", SELECTION_FIXTURE_WINDOW).unwrap();
    // Once built, the reader stays.
    reader.lock().unwrap().unavailable = true;
    subject.read_selection("Notepad", SELECTION_FIXTURE_WINDOW).unwrap();
    assert_eq!(reader.lock().unwrap().attempts, 3);
    assert_eq!(reader.lock().unwrap().windows, [SELECTION_FIXTURE_WINDOW, SELECTION_FIXTURE_WINDOW]);
}

#[test]
fn a_windows_without_the_clipboard_calls_cannot_read_it() {
    let api = FakeWindowsAPI::plain();
    assert_eq!(refusal(&reading_backend(&api, &script(|_| {}))), RefusalReason::Unsupported);
}

#[test]
fn without_a_caret_or_an_edit_field_nothing_is_copied() {
    let api = FakeWindowsAPI::with_clipboard();
    api.state().caret = false;
    for reader in [
        script(|_| {}),
        script(|script| script.editable = Err(provider_gone())),
        script(|script| script.unavailable = true),
    ] {
        assert_eq!(refusal(&reading_backend(&api, &reader)), RefusalReason::NoCaret);
    }
    assert!(api.sent().is_empty());
    assert!(api.state().caret_windows.iter().all(|&window| window == WINDOWS_FAKE_HWND));
}

#[test]
fn an_edit_field_ui_automation_names_is_copied_without_a_caret() {
    let api = FakeWindowsAPI::with_clipboard();
    api.state().caret = false;
    let reader = script(|script| script.editable = Ok(true));
    let selected = reading_backend(&api, &reader).read_selection("Notepad", SELECTION_FIXTURE_WINDOW).unwrap();
    assert_eq!(selected, selected_by(SOURCE_CLIPBOARD, "ghbdtn"));
    assert_eq!(api.sent(), [copy_keys()]);
}

#[test]
fn a_busy_clipboard_is_asked_again() {
    let api = FakeWindowsAPI::with_clipboard();
    {
        let mut state = api.state();
        state.busy_reads = 1;
        state.busy_texts = 1;
        state.copy_delay = 1;
    }
    let clock = FakeClock::stepping(0.0);
    let subject = backend_with(&api, clock, script(|_| {}));
    assert_eq!(subject.read_selection("Notepad", 0).unwrap().text, "ghbdtn");
}

#[test]
fn a_clipboard_that_cannot_be_kept_or_read_is_not_borrowed() {
    let api = FakeWindowsAPI::with_clipboard();
    let reader = script(|_| {});
    api.state().busy_reads = 256;
    assert_eq!(refusal(&reading_backend(&api, &reader)), RefusalReason::ClipboardBusy);
    {
        let mut state = api.state();
        state.busy_reads = 0;
        state.saved = ClipboardContents { formats: Vec::new(), complete: false };
    }
    assert_eq!(refusal(&reading_backend(&api, &reader)), RefusalReason::ClipboardUnrestorable);
    assert!(api.sent().is_empty());
}

#[test]
fn no_copy_in_time_means_no_selection_and_an_untouched_clipboard() {
    let api = FakeWindowsAPI::with_clipboard();
    api.state().copies = false;
    assert_eq!(refusal(&reading_backend(&api, &script(|_| {}))), RefusalReason::NoSelection);
    assert!(api.state().written.is_empty());
}

#[test]
fn a_copy_that_cannot_be_read_still_gives_the_clipboard_back() {
    let api = FakeWindowsAPI::with_clipboard();
    api.state().busy_texts = 256;
    assert_eq!(refusal(&reading_backend(&api, &script(|_| {}))), RefusalReason::ClipboardBusy);
    assert_eq!(api.state().written, [saved_clipboard()]);
}

#[test]
fn a_clipboard_that_could_not_be_given_back_is_an_error() {
    let api = FakeWindowsAPI::with_clipboard();
    api.state().restores = vec![false; 64];
    match reading_backend(&api, &script(|_| {})).read_selection("Notepad", 0) {
        Err(SelectionError::Failed(error)) => assert!(message(&error).contains("Буфер обмена не удалось вернуть")),
        other => panic!("not a failure: {other:?}"),
    }
}

#[test]
fn the_result_is_typed_as_characters_selected_back_and_the_layout_moved() {
    let api = FakeWindowsAPI::with_clipboard();
    let subject = reading_backend(&api, &script(|_| {}));
    assert!(subject.replace_selection(&selected_by(SOURCE_UIA, "ghbdtn"), "пр", Some(1)).unwrap());
    let character = |pressed, unit: char| NativeInput { scan_code: unit.to::<u32>(), unicode: true, ..NativeInput::new(pressed) };
    let left = [NativeInput::extended_key(true, VK_LEFT), NativeInput::extended_key(false, VK_LEFT)];
    let mut expected = vec![character(true, 'п'), character(false, 'п'), character(true, 'р'), character(false, 'р')];
    expected.push(NativeInput::key(true, VK_SHIFT));
    expected.extend(left);
    expected.extend(left);
    expected.push(NativeInput::key(false, VK_SHIFT));
    assert_eq!(api.sent(), [expected]);
    assert_eq!(api.requests(), [RUSSIAN_HKL]);
}

#[test]
fn a_character_beyond_the_basic_plane_goes_as_its_two_units_and_is_not_selected_back() {
    let api = FakeWindowsAPI::with_clipboard();
    let subject = reading_backend(&api, &script(|_| {}));
    assert!(!subject.replace_selection(&selected_by(SOURCE_UIA, "x"), "😀", None).unwrap());
    let sent = api.sent();
    let pressed: Vec<u32> = sent[0].iter().filter(|item| item.pressed).map(|item| item.scan_code).collect();
    assert_eq!(pressed, "😀".encode_utf16().map(u32::from).collect::<Vec<_>>());
    assert!(sent[0].iter().all(|item| item.unicode));
    assert!(api.requests().is_empty());
}

#[test]
fn another_window_or_a_short_send_changes_nothing_more() {
    let api = FakeWindowsAPI::with_clipboard();
    let subject = reading_backend(&api, &script(|_| {}));
    subject.hold_input();
    api.state().foreground = SELECTION_FIXTURE_OTHER_WINDOW;
    let selected = selected_by(SOURCE_UIA, "x");
    assert_eq!(
        subject.replace_selection(&selected, "y", None).unwrap_err(),
        SelectionError::Refused(SelectionRefused::new(RefusalReason::FocusChanged))
    );
    assert!(api.sent().is_empty());
    subject.release_input().unwrap();
    api.state().send_count = Some(0);
    assert!(matches!(subject.replace_selection(&selected, "y", None), Err(SelectionError::Failed(_))));
}

/// Notepad of Windows 11 (`SLOW_INPUT_FIELD_CLASSES`) gets the result pasted, and the clipboard back.
fn pasting_api() -> Arc<FakeWindowsAPI> {
    let api = FakeWindowsAPI::with_clipboard();
    api.state().window_classes.insert(WINDOWS_FAKE_HWND, "RichEditD2DPT".to_owned());
    api
}

fn ours() -> ClipboardContents {
    let bytes = "пр\0".encode_utf16().flat_map(u16::to_le_bytes).collect();
    ClipboardContents { formats: vec![(CF_UNICODETEXT, bytes)], complete: true }
}

fn showing(shown: &str, late: usize) -> SharedScript {
    script(|script| {
        script.shown = Some(shown.to_owned());
        script.late = late;
    })
}

#[test]
fn the_result_is_pasted_selected_back_and_the_clipboard_given_back() {
    let api = pasting_api();
    let reader = showing("пр", 1);
    let subject = reading_backend(&api, &reader);
    assert!(subject.replace_selection(&selected_by(SOURCE_UIA, "gh"), "пр", Some(1)).unwrap());
    let left = [NativeInput::extended_key(true, VK_LEFT), NativeInput::extended_key(false, VK_LEFT)];
    let mut expected = paste_keys();
    expected.push(NativeInput::key(true, VK_SHIFT));
    expected.extend(left);
    expected.extend(left);
    expected.push(NativeInput::key(false, VK_SHIFT));
    assert_eq!(api.sent(), [expected]);
    assert_eq!(api.state().written, [ours(), saved_clipboard()]);
    assert_eq!(reader.lock().unwrap().windows, [WINDOWS_FAKE_HWND, WINDOWS_FAKE_HWND]);
    assert_eq!(api.requests(), [RUSSIAN_HKL]);
}

#[test]
fn a_result_never_shown_still_gives_the_clipboard_back_and_is_not_called_selected() {
    let api = pasting_api();
    let subject = reading_backend(&api, &showing("gh", 0));
    assert!(!subject.replace_selection(&selected_by(SOURCE_UIA, "gh"), "пр", None).unwrap());
    assert_eq!(api.state().written, [ours(), saved_clipboard()]);
    let refusing_reader = script(|script| script.answer = Err(refusing(RefusalReason::ReadOnly)));
    let subject = reading_backend(&api, &refusing_reader);
    assert!(!subject.replace_selection(&selected_by(SOURCE_UIA, "gh"), "пр", None).unwrap());
}

#[test]
fn something_copied_meanwhile_is_the_users_and_stays() {
    let api = pasting_api();
    let reader = showing("пр", 0);
    let copied = Arc::clone(&api);
    reader.lock().unwrap().on_read = Some(Arc::new(move || copied.state().sequence += 1));
    let subject = reading_backend(&api, &reader);
    assert!(subject.replace_selection(&selected_by(SOURCE_UIA, "gh"), "пр", None).unwrap());
    assert_eq!(api.state().written, [ours()]);
}

#[test]
fn a_clipboard_that_cannot_be_kept_borrowed_or_given_back() {
    let api = pasting_api();
    let subject = reading_backend(&api, &showing("пр", 0));
    let selected = selected_by(SOURCE_UIA, "gh");
    let incomplete = ClipboardContents { formats: Vec::new(), complete: false };
    for (saved, busy, reason) in
        [(incomplete, 0, RefusalReason::ClipboardUnrestorable), (saved_clipboard(), 256, RefusalReason::ClipboardBusy)]
    {
        {
            let mut state = api.state();
            state.saved = saved;
            state.busy_reads = busy;
        }
        assert_eq!(
            subject.replace_selection(&selected, "пр", None).unwrap_err(),
            SelectionError::Refused(SelectionRefused::new(reason))
        );
    }
    {
        let mut state = api.state();
        state.busy_reads = 0;
        state.restores = vec![false; 64];
    }
    assert_eq!(
        subject.replace_selection(&selected, "пр", None).unwrap_err(),
        SelectionError::Refused(SelectionRefused::new(RefusalReason::ClipboardBusy))
    );
    assert!(api.sent().is_empty());
}

#[test]
fn a_clipboard_not_given_back_is_an_error() {
    let api = pasting_api();
    let mut writes = vec![true];
    writes.extend([false; 64]);
    api.state().restores = writes;
    let subject = reading_backend(&api, &showing("пр", 0));
    match subject.replace_selection(&selected_by(SOURCE_UIA, "gh"), "пр", None) {
        Err(SelectionError::Failed(error)) => assert!(message(&error).contains("в нём остался вставленный текст")),
        other => panic!("not a failure: {other:?}"),
    }
}

#[test]
fn a_selection_read_from_the_clipboard_or_another_field_is_typed() {
    let api = pasting_api();
    let subject = reading_backend(&api, &showing("пр", 0));
    subject.replace_selection(&selected_by(SOURCE_CLIPBOARD, "gh"), "пр", None).unwrap();
    api.state().window_classes.insert(WINDOWS_FAKE_HWND, "Edit".to_owned());
    subject.replace_selection(&selected_by(SOURCE_UIA, "gh"), "пр", None).unwrap();
    assert!(api.state().written.is_empty());
    assert!(api.sent().iter().all(|batch| batch[0].unicode));
}

/// Alt+Pause: the window must not see Alt go down and up alone, or it opens its menu bar.
fn mask_setup() -> (Arc<FakeWindowsAPI>, WindowsBackend, Arc<Mutex<Vec<String>>>) {
    let api = FakeWindowsAPI::with_clipboard();
    let subject = backend(&api);
    let names: Arc<Mutex<Vec<String>>> = Arc::default();
    let seen = Arc::clone(&names);
    subject.start(Arc::new(move |event: KeyEvent| seen.lock().unwrap().push(event.key_name))).unwrap();
    subject.set_key_filter(Some(Arc::new(|event: &KeyEvent| {
        if event.key_name == "Pause" { KeyDisposition::Consume } else { KeyDisposition::Pass }
    })));
    (api, subject, names)
}

fn chord(hook: &HookListener, modifier: u32, scan_code: u32) -> bool {
    hook(native(true, modifier, scan_code, false, 5000));
    hook(native(true, VK_PAUSE, SCAN_CODE_PAUSE, false, 5100))
}

#[test]
fn a_key_kept_from_the_window_under_alt_presses_the_mask_key_which_no_one_sees() {
    let (api, subject, names) = mask_setup();
    let hook = api.hook();
    assert!(chord(&hook, VK_LMENU, SCAN_CODE_LEFT_ALT));
    assert_eq!(api.sent(), [[NativeInput::key(true, VK_MENU_MASK), NativeInput::key(false, VK_MENU_MASK)]]);
    assert!(!hook(native(true, VK_MENU_MASK, 0, true, 5100)));
    assert_eq!(*names.lock().unwrap(), ["Alt_L", "Pause"]);
    subject.stop();
}

#[test]
fn the_mask_key_also_covers_win_and_never_goes_on_a_thread_of_its_own_in_this_test() {
    let (api, subject, _names) = mask_setup();
    assert!(chord(&api.hook(), VK_LWIN, 0x5B));
    assert_eq!(api.sent().len(), 1);
    subject.stop();
}

#[test]
fn without_alt_nothing_is_pressed_and_a_failed_mask_is_survived() {
    log::capture::install();
    let (api, subject, _names) = mask_setup();
    let hook = api.hook();
    assert!(chord(&hook, VK_LSHIFT, SCAN_CODE_LEFT_SHIFT));
    assert!(api.sent().is_empty());
    hook(native(false, VK_PAUSE, SCAN_CODE_PAUSE, false, 5200));
    hook(native(false, VK_LSHIFT, SCAN_CODE_LEFT_SHIFT, false, 5200));
    api.state().send_error = true;
    assert!(chord(&hook, VK_LMENU, SCAN_CODE_LEFT_ALT));
    assert!(log::capture::contains("keyswitch.windows_backend: Клавиша-маска для Alt не отправлена"));
    api.state().send_error = false;
    subject.stop();
}

#[test]
fn the_default_job_starter_runs_the_mask_off_the_hook_thread() {
    let api = FakeWindowsAPI::plain();
    let subject = WindowsBackend::with_api(api.clone());
    let here = thread::current().id();
    let (sent, on_thread) = mpsc::channel();
    let sent = Mutex::new(sent);
    api.state().on_send = Some(Arc::new(move |_inputs: &[NativeInput]| {
        sent.lock().unwrap().send(thread::current().id()).unwrap();
    }));
    subject.set_key_filter(Some(Arc::new(|event: &KeyEvent| {
        if event.key_name == "Pause" { KeyDisposition::Consume } else { KeyDisposition::Pass }
    })));
    let _ = subject.handle_native(native(true, VK_LMENU, SCAN_CODE_LEFT_ALT, false, 5000));
    let _ = subject.handle_native(native(true, VK_PAUSE, SCAN_CODE_PAUSE, false, 5100));
    assert_ne!(on_thread.recv_timeout(Duration::from_secs(10)).unwrap(), here);
}

/// Random sequences of keys and engine calls dumped from the Python backend: after each step everything the fake and the
/// backend's books hold must be what the Python's held, down to the moment of the fake clock.
mod golden {
    use std::io::{BufRead, BufReader};
    use std::path::PathBuf;

    use flate2::read::GzDecoder;
    use serde_json::{Value, json};

    use super::*;

    const DEFAULT_GOLDEN_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../.t/golden");

    fn golden_file() -> PathBuf {
        let root = std::env::var("KS_GOLDEN_DIR").unwrap_or_else(|_| DEFAULT_GOLDEN_DIR.to_owned());
        PathBuf::from(root).join("windows_backend").join("sequences.jsonl.gz")
    }

    fn number(value: &Value) -> i64 {
        value.as_i64().unwrap_or_else(|| panic!("not a number: {value}"))
    }

    fn flag(value: &Value) -> bool {
        value.as_bool().unwrap_or_else(|| panic!("not a flag: {value}"))
    }

    fn text(value: &Value) -> String {
        value.as_str().unwrap_or_else(|| panic!("not a text: {value}")).to_owned()
    }

    fn numbers(value: &Value) -> Vec<i64> {
        value.as_array().unwrap().iter().map(number).collect()
    }

    fn stroke(value: &Value) -> KeyEvent {
        KeyEvent {
            pressed: true,
            keycode: number(&value["keycode"]).low::<u32>(),
            key_name: "k".to_owned(),
            character: text(&value["character"]),
            characters: value["characters"].as_array().unwrap().iter().map(text).collect(),
            group: number(&value["group"]).low::<i32>(),
            state: number(&value["state"]).low::<u32>(),
            timestamp: 100,
            ..KeyEvent::default()
        }
    }

    fn strokes(value: &Value) -> Vec<KeyEvent> {
        value.as_array().unwrap().iter().map(stroke).collect()
    }

    fn error_text(error: &BackendError) -> String {
        match error {
            BackendError::Failed(message) => format!("err:{message}"),
            BackendError::LayoutSwitchUnconfirmed { message, group } => format!("err:{message}|group={group}"),
        }
    }

    fn selection_error_text(error: &SelectionError) -> String {
        match error {
            SelectionError::Refused(refused) => format!("refused:{}", refused.reason.as_str()),
            SelectionError::Failed(error) => error_text(error),
        }
    }

    fn answer(value: &Value) -> Result<Option<SelectedText>, SelectionError> {
        match value.as_str() {
            None => Ok(None),
            Some("raise") => Err(provider_gone()),
            Some("refused:read_only") => Err(refusing(RefusalReason::ReadOnly)),
            Some("refused:no_selection") => Err(refusing(RefusalReason::NoSelection)),
            Some(text) => Ok(Some(selected_by(SOURCE_UIA, text))),
        }
    }

    fn set_field(api: &FakeWindowsAPI, reader: &SharedScript, name: &str, value: &Value) {
        let mut state = api.state();
        let mut script = reader.lock().unwrap();
        match name {
            "current_layout" => state.current_layout = number(value),
            "foreground" => state.foreground = number(value),
            "ticks" => state.ticks = number(value),
            "send_count" => state.send_count = value.as_u64().map(ks_cast::To::to::<usize>),
            "accept_switch" => state.accept_switch = flag(value),
            "apply_switch" => state.apply_switch = flag(value),
            "released" => state.released_keys = numbers(value).into_iter().map(ks_cast::To::low::<u32>).collect(),
            "window_class" => {
                let foreground = state.foreground;
                state.window_classes.insert(foreground, text(value));
            }
            "caret" => state.caret = flag(value),
            "copied" => state.copied = Some(text(value)),
            "copies" => state.copies = flag(value),
            "copy_delay" => state.copy_delay = number(value).to::<usize>(),
            "busy_reads" => state.busy_reads = number(value).to::<usize>(),
            "busy_texts" => state.busy_texts = number(value).to::<usize>(),
            "saved_complete" => state.saved.complete = flag(value),
            "restores" => state.restores = value.as_array().unwrap().iter().map(flag).collect(),
            "last_input" => state.last_input = number(value),
            "input_times" => state.input_times = numbers(value),
            "answer" => script.answer = answer(value),
            "editable" => {
                script.editable = if value.is_string() { Err(provider_gone()) } else { Ok(flag(value)) };
            }
            "shown" => script.shown = value.as_str().map(str::to_owned),
            "late" => script.late = number(value).to::<usize>(),
            "factory_fails" => script.unavailable = flag(value),
            other => panic!("no such field: {other}"),
        }
    }

    fn set_filter(subject: &WindowsBackend, mode: &str) {
        match mode {
            "none" => subject.set_key_filter(None),
            "defer" => subject.set_key_filter(Some(Arc::new(|event: &KeyEvent| {
                if matches!(event.key_name.as_str(), "Return" | "Tab") && event.state == 0 {
                    KeyDisposition::Defer
                } else {
                    KeyDisposition::Pass
                }
            }))),
            _ => subject.set_key_filter(Some(Arc::new(|event: &KeyEvent| {
                if matches!(event.key_name.as_str(), "Pause" | "Return") {
                    KeyDisposition::Consume
                } else {
                    KeyDisposition::Pass
                }
            }))),
        }
    }

    fn inject(subject: &WindowsBackend, op: &Value) -> Value {
        let (all, late, trailing) = (strokes(&op["strokes"]), strokes(&op["late"]), strokes(&op["trailing"]));
        let boundary = (!op["boundary"].is_null()).then(|| stroke(&op["boundary"]));
        let outcome = subject.inject_correction(Injection {
            strokes: &all,
            target_group: number(&op["target"]).low::<i32>(),
            boundary: boundary.as_ref(),
            source_group: op["source"].as_i64().map(ks_cast::To::low::<i32>),
            late: &late,
            trailing: &trailing,
            kept_tail: number(&op["kept_tail"]).to::<usize>(),
            erased: op["erased"].as_u64().map(ks_cast::To::to::<usize>),
        });
        outcome.map_or_else(|error| json!(error_text(&error)), |count| json!(count))
    }

    fn step(subject: &WindowsBackend, api: &FakeWindowsAPI, reader: &SharedScript, op: &Value) -> Value {
        let backend_result = |outcome: Result<i32, BackendError>| outcome.map_or_else(|error| json!(error_text(&error)), |count| json!(count));
        match op["op"].as_str().unwrap() {
            "key" => json!(subject.handle_native(NativeKeyEvent {
                pressed: flag(&op["pressed"]),
                virtual_key: number(&op["vk"]).low::<u32>(),
                scan_code: number(&op["scan"]).low::<u32>(),
                extended: flag(&op["extended"]),
                injected: flag(&op["injected"]),
                timestamp: number(&op["ts"]),
                replayed: flag(&op["replayed"]),
                foreign: flag(&op["foreign"]),
                wheel: false,
            })),
            "hold" => {
                subject.hold_input();
                Value::Null
            }
            "release" => backend_result(subject.release_input()),
            "complete" => backend_result(subject.complete_action(flag(&op["deliver"]))),
            "filter" => {
                set_filter(subject, op["mode"].as_str().unwrap());
                Value::Null
            }
            "inject" => inject(subject, op),
            "switch" => subject
                .switch_group(number(&op["group"]).low::<i32>())
                .map_or_else(|error| json!(error_text(&error)), |()| Value::Null),
            "group" => backend_result(subject.current_group()),
            "focused" => subject
                .focused_window()
                .map_or(Value::Null, |focus| json!([focus.window, i32::from(focus.own), i32::from(focus.isolated_layout)])),
            "probe" => {
                let probe = subject.probe();
                json!([i32::from(probe.available), probe.xkb_version, probe.current_group, probe.error])
            }
            "delay" => {
                let report = subject.take_input_delay().unwrap();
                let gap = subject.take_hook_gap().map_or(Value::Null, |gap| json!([gap.late_ms, gap.callback_ms]));
                json!([
                    report.keys,
                    report.late_keys,
                    report.worst_late_ms,
                    report.slow_callbacks,
                    report.worst_callback_ms,
                    report.unstamped_keys,
                    report.foreign_keys,
                    gap
                ])
            }
            "unseen" => json!(subject.take_unseen_input().unwrap()),
            "read_selection" => match subject.read_selection("Notepad", number(&op["window"])) {
                Ok(selected) => json!([selected.text, selected.source]),
                Err(error) => json!(selection_error_text(&error)),
            },
            "replace_selection" => {
                let selected = selected_by(op["source"].as_str().unwrap(), op["selected"].as_str().unwrap());
                let final_group = op["final"].as_i64().map(ks_cast::To::low::<i32>);
                match subject.replace_selection(&selected, op["replacement"].as_str().unwrap(), final_group) {
                    Ok(reselected) => json!(reselected),
                    Err(error) => json!(selection_error_text(&error)),
                }
            }
            "stop" => {
                subject.stop();
                Value::Null
            }
            "set" => {
                set_field(api, reader, op["name"].as_str().unwrap(), &op["value"]);
                Value::Null
            }
            other => panic!("no such step: {other}"),
        }
    }

    fn sorted_keys(keys: &KeySet) -> Vec<u32> {
        let mut sorted = keys.0.clone();
        sorted.sort_unstable();
        sorted
    }

    fn input_json(item: &NativeInput) -> Value {
        json!([
            i32::from(item.pressed),
            item.virtual_key,
            item.scan_code,
            i32::from(item.extended),
            i32::from(item.synthetic),
            i32::from(item.replayed),
            i32::from(item.unicode),
            i32::from(item.foreign)
        ])
    }

    fn event_json(event: &KeyEvent) -> Value {
        json!([
            i32::from(event.pressed),
            event.keycode,
            event.key_name,
            event.character,
            event.characters,
            event.group,
            event.state,
            event.timestamp,
            i32::from(event.synthetic),
            i32::from(event.deferred),
            i32::from(event.foreign)
        ])
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// Everything observable, in the form the generator wrote it; what a step changed in the fake is taken from it.
    fn observed(subject: &WindowsBackend, api: &FakeWindowsAPI, events: &Collected, clock: &FakeClock) -> Value {
        let mut state = api.state();
        let sent: Vec<Value> = state.sent.drain(..).map(|batch| batch.iter().map(input_json).collect()).collect();
        let requests = std::mem::take(&mut state.requests);
        let written: Vec<Value> = state
            .written
            .drain(..)
            .map(|contents| contents.formats.iter().map(|(item, data)| json!([item, hex(data)])).collect())
            .collect();
        drop(state);
        let delivered: Vec<Value> = events.lock().unwrap().drain(..).map(|event| event_json(&event)).collect();
        let inner = &subject.inner;
        let hold = inner.hold.lock().unwrap();
        let hook = inner.hook.lock().unwrap();
        let unseen = inner.unseen.lock().unwrap();
        let held: Vec<Value> = hold
            .held
            .iter()
            .map(|item| {
                json!([
                    i32::from(item.pressed),
                    item.virtual_key,
                    item.scan_code,
                    i32::from(item.foreign),
                    i32::from(item.replayed)
                ])
            })
            .collect();
        json!({
            "sent": sent,
            "events": delivered,
            "requests": requests,
            "written": written,
            "now": *clock.now.lock().unwrap(),
            "holding": i32::from(hold.holding),
            "held": held,
            "deferred": hold.deferred_action.map(|action| action.virtual_key),
            "prior": sorted_keys(&hold.action_prior_keys),
            "pressed": sorted_keys(&hook.pressed),
            "consumed": sorted_keys(&hook.consumed_keys),
            "caps": i32::from(inner.caps_lock.load(Ordering::Relaxed)),
            "epoch": inner.pointer_epoch.load(Ordering::Relaxed),
            "hold_epoch": hold.hold_pointer_epoch,
            "hold_window": hold.hold_window,
            "unseen": [unseen.unseen_input, unseen.unseen_times.iter().collect::<Vec<_>>(), unseen.unseen_reported],
        })
    }

    fn replay(scenario: &Value) {
        let api = FakeWindowsAPI::with_clipboard();
        {
            let mut state = api.state();
            state.scripted_translation = true;
            state.window_owners = HashMap::from([(300, 4242), (301, 7777), (302, 0)]);
            state.process_id = 7777;
        }
        let clock = FakeClock::stepping(scenario["step"].as_f64().unwrap());
        let reader = script(|_| {});
        let subject = backend_with(&api, clock.clone(), Arc::clone(&reader));
        let events = collected();
        subject.set_listener(Some(collecting(&events)));
        if flag(&scenario["reenter"]) {
            let inner = Arc::clone(&subject.inner);
            let fake = Arc::clone(&api);
            api.state().on_send = Some(Arc::new(move |inputs: &[NativeInput]| {
                let count = fake.state().send_count.unwrap_or(inputs.len()).min(inputs.len());
                for item in &inputs[..count] {
                    let virtual_key = if item.virtual_key != 0 {
                        item.virtual_key
                    } else if item.unicode {
                        VK_PACKET
                    } else {
                        VK_A
                    };
                    let scan_code = if item.scan_code != 0 { item.scan_code } else { SCAN_CODE_A };
                    inner.handle_native(NativeKeyEvent {
                        replayed: item.replayed,
                        foreign: item.foreign,
                        ..NativeKeyEvent::new(item.pressed, virtual_key, scan_code, item.extended, item.synthetic, 1)
                    });
                }
            }));
        }
        let seed = &scenario["seed"];
        for (index, op) in scenario["ops"].as_array().unwrap().iter().enumerate() {
            let result = step(&subject, &api, &reader, op);
            let expect = &op["expect"];
            // The Python's stop raises where the trait's stop has no error to give; what it did is the same.
            if op["op"] != "stop" {
                assert_eq!(result, expect["result"], "result, seed {seed}, step {index}: {op}");
            }
            let state = json!(subject.normalized_state());
            assert_eq!(state, expect["state"], "state, seed {seed}, step {index}: {op}");
            let seen = observed(&subject, &api, &events, &clock);
            for (name, value) in seen.as_object().unwrap() {
                assert_eq!(value, &expect[name], "{name}, seed {seed}, step {index}: {op}");
            }
        }
    }

    #[test]
    fn the_python_sequences_replay_alike() {
        let path = golden_file();
        let Ok(file) = std::fs::File::open(&path) else {
            eprintln!("skipped: no golden sequences at {}", path.display());
            return;
        };
        let mut scenarios = 0;
        for line in BufReader::new(GzDecoder::new(file)).lines() {
            let scenario: Value = serde_json::from_str(&line.unwrap()).unwrap();
            replay(&scenario);
            scenarios += 1;
        }
        assert!(scenarios > 0, "empty golden file");
        eprintln!("replayed {scenarios} sequences");
    }
}

#[test]
fn every_key_with_a_name_has_the_engines_name() {
    for (key, name) in [
        (VK_BACK, "BackSpace"), (VK_TAB, "Tab"), (VK_RETURN, "Return"), (VK_SHIFT, "Shift_L"), (VK_CONTROL, "Control_L"), (VK_MENU, "Alt_L"), (VK_PAUSE, "Pause"),
        (VK_SCROLL, "Scroll_Lock"), (VK_CAPITAL, "Caps_Lock"), (VK_ESCAPE, "Escape"), (VK_SPACE, "space"), (VK_PRIOR, "Page_Up"), (VK_NEXT, "Page_Down"), (VK_END, "End"),
        (VK_HOME, "Home"), (VK_LEFT, "Left"), (VK_UP, "Up"), (VK_RIGHT, "Right"), (VK_DOWN, "Down"), (VK_INSERT, "Insert"), (VK_DELETE, "Delete"), (VK_LWIN, "Super_L"),
        (VK_RWIN, "Super_R"), (VK_LSHIFT, "Shift_L"), (VK_RSHIFT, "Shift_R"), (VK_LCONTROL, "Control_L"), (VK_RCONTROL, "Control_R"), (VK_LMENU, "Alt_L"), (VK_RMENU, "Alt_R"),
        (VK_OEM_1, "semicolon"), (VK_OEM_PLUS, "equal"), (VK_OEM_COMMA, "comma"), (VK_OEM_MINUS, "minus"), (VK_OEM_PERIOD, "period"), (VK_OEM_2, "slash"), (VK_OEM_3, "grave"),
        (VK_OEM_4, "bracketleft"), (VK_OEM_5, "backslash"), (VK_OEM_6, "bracketright"), (VK_OEM_7, "apostrophe"),
    ] {
        assert_eq!(named_key(key), Some(name));
    }
    assert_eq!(named_key(0xFFFF), None);
}

#[test]
fn the_real_clock_moves_and_sleeps() {
    let clock = SystemClock::default();
    let first = clock.monotonic();
    clock.sleep(0.001);
    assert!(clock.monotonic() > first);
    assert!(clock.perf_counter() > 0.0);
}

#[cfg(not(windows))]
#[test]
fn the_field_reader_is_only_on_windows() {
    assert!(uia_selection_reader().is_err());
}

#[test]
fn an_api_with_no_selection_answers_none_and_the_backend_can_select() {
    let api = FakeWindowsAPI::plain();
    let bare = backend(&api);
    assert!(bare.supports_selection());
    let _ = bare.normalized_state();
}

/// A small generator with a fixed start: the walk is the same on every run.
struct Dice(u64);

impl Dice {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, limit: usize) -> usize {
        (self.next() % limit.to::<u64>()).to::<usize>()
    }
    fn of<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
    fn one_in(&mut self, odds: usize) -> bool {
        self.below(odds) == 0
    }
}

const WALK_KEYS: [u32; 28] = [VK_A, VK_RETURN, VK_TAB, VK_BACK, VK_ESCAPE, VK_SPACE, VK_LEFT, VK_RIGHT, VK_HOME, VK_END, VK_DELETE, VK_PAUSE, VK_CAPITAL, VK_SHIFT, VK_LSHIFT, VK_RSHIFT, VK_CONTROL, VK_LCONTROL, VK_RCONTROL, VK_MENU, VK_LMENU, VK_RMENU, VK_LWIN, VK_RWIN, VK_OEM_1, VK_OEM_COMMA, VK_PACKET, 0xE7];

fn walk_stroke(dice: &mut Dice) -> KeyEvent {
    let code = *dice.of(&WALK_KEYS);
    let group = *dice.of(&[0, 0, 1, 1, -1, 5]);
    key_with(code, dice.of(&["a", "A", "ф", "Ф", ",", " "]), [*dice.of(&["a", "A", ",", ""]), *dice.of(&["ф", "Ф", "б", ""])], group)
}

#[test]
fn a_long_walk_through_the_backend_stops_nowhere() {
    for seed in 1..=300_u64 {
        let mut dice = Dice(0x9e37_79b9_7f4a_7c15 ^ seed.wrapping_mul(0xbf58_476d_1ce4_e5b9));
        let api = FakeWindowsAPI::plain();
        let subject = backend(&api);
        let mut time = 1000_i64;
        for _ in 0..60 {
            time += 5;
            let code = *dice.of(&WALK_KEYS);
            match dice.below(30) {
                0..=11 => {
                    let _ = subject.handle_native(native(true, code, code, false, time));
                    let _ = subject.handle_native(native(false, code, code, false, time + 1));
                }
                12..=14 => {
                    let _ = subject.handle_native(native(true, code, code, false, time));
                }
                15 => {
                    let _ = subject.handle_native(NativeKeyEvent { foreign: true, ..native(dice.one_in(2), code, code, false, time) });
                }
                16 => {
                    let _ = subject.handle_native(NativeKeyEvent { replayed: true, ..native(dice.one_in(2), code, code, false, time) });
                }
                17 => {
                    let _ = subject.handle_native(native(dice.one_in(2), code, code, true, time));
                }
                18..=19 => subject.hold_input(),
                20 => {
                    let _ = subject.release_input();
                }
                21 => {
                    let _ = subject.complete_action(dice.one_in(2));
                }
                22 => {
                    let strokes: Vec<KeyEvent> = (0..dice.below(4)).map(|_| walk_stroke(&mut dice)).collect();
                    let trailing: Vec<KeyEvent> = (0..dice.below(2)).map(|_| walk_stroke(&mut dice)).collect();
                    let late: Vec<KeyEvent> = (0..dice.below(2)).map(|_| walk_stroke(&mut dice)).collect();
                    let boundary = dice.one_in(2).then(|| walk_stroke(&mut dice));
                    let _ = subject.inject_correction(Injection {
                        strokes: &strokes,
                        target_group: *dice.of(&[0, 1, -1, 7]),
                        boundary: boundary.as_ref(),
                        source_group: *dice.of(&[None, None, Some(0), Some(1), Some(-1), Some(9)]),
                        late: &late,
                        trailing: &trailing,
                        kept_tail: *dice.of(&[0, 0, 1, 9]),
                        erased: *dice.of(&[None, None, Some(1)]),
                    });
                }
                23 => match dice.below(4) {
                    0 => subject.set_key_filter(Some(Arc::new(|event: &KeyEvent| if event.key_name == "Return" { KeyDisposition::Defer } else { KeyDisposition::Pass }))),
                    1 => subject.set_key_filter(Some(Arc::new(|_: &KeyEvent| KeyDisposition::Consume))),
                    2 => subject.set_key_filter(Some(Arc::new(|_: &KeyEvent| KeyDisposition::Defer))),
                    _ => subject.set_key_filter(None),
                },
                24 => api.state.lock().unwrap().foreground = *dice.of(&[0, 1, 2]),
                25 => api.state.lock().unwrap().caps_lock = dice.one_in(2),
                26 => {
                    let mut state = api.state.lock().unwrap();
                    state.send_error = dice.one_in(4);
                    state.send_count = *dice.of(&[None, None, Some(0), Some(1)]);
                    state.accept_switch = !dice.one_in(6);
                    state.apply_switch = !dice.one_in(6);
                }
                27 => {
                    api.state.lock().unwrap().released_keys.insert(code);
                    time += 100_000;
                }
                28 => {
                    let _ = subject.switch_group(*dice.of(&[0, 1, -1, 9]));
                }
                _ => {
                    let _ = (subject.current_group(), subject.active_application(), subject.focused_window(), subject.probe(), subject.input_anchor());
                }
            }
        }
        let _ = subject.complete_action(false);
        let _ = subject.release_input();
    }
}
