//! The Python's `test_wayland_backend.py` on a fake of the compositor and the helper.

use crate::wayland_constants::{EVDEV_KEY_A, EVDEV_KEY_Z};
use super::*;
use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;

const WINDOW: i64 = 42;
const SECOND_WINDOW: i64 = 77;
const SEQ_FIRST: u32 = 5;
const SEQ_SECOND: u32 = 6;
const SEQ_THIRD: u32 = 7;
const KEY_G: u32 = 34;
const EN: i64 = 0;
const RU: i64 = 1;

/// (plain, shifted) in English and Russian, and the keysym name.
type Spec = ((&'static str, &'static str), (&'static str, &'static str), &'static str);

fn qwerty(keycode: u32) -> Option<Spec> {
    Some(match keycode {
        EVDEV_KEY_Q => (("q", "Q"), ("й", "Й"), "q"),
        EVDEV_KEY_A => (("a", "A"), ("ф", "Ф"), "a"),
        EVDEV_KEY_Z => (("z", "Z"), ("я", "Я"), "z"),
        KEY_G => (("g", "G"), ("п", "П"), "g"),
        EVDEV_KEY_SEMICOLON => ((";", ":"), ("ж", "Ж"), "semicolon"),
        EVDEV_KEY_COMMA => ((",", "<"), ("б", "Б"), "comma"),
        // The Russian layout has `,` on Shift with the slash key: a comma after a word is typed there.
        EVDEV_KEY_SLASH => (("/", "?"), (".", ","), "slash"),
        EVDEV_KEY_SPACE => ((" ", " "), (" ", " "), "space"),
        _ => return None,
    })
}

#[derive(Default)]
struct FakeState {
    denied: String,
    names: Vec<String>,
    current_index: i64,
    accept_switch: bool,
    switch_requests: Vec<i64>,
    fail_switch_from: Option<usize>,
    posted: Vec<Vec<(bool, u32)>>,
    known_seqs: Vec<Option<u32>>,
    actions: Vec<bool>,
    post_result: Option<u32>,
    refuse_checked: bool,
    window: i64,
    owners: HashMap<i64, i64>,
    caps_lock: bool,
    released_keys: HashSet<u32>,
    now_ms: u32,
    password_windows: HashSet<i64>,
    signal_ready: bool,
    listener_error: Option<String>,
    stop_calls: usize,
    close_calls: usize,
    listener: Option<NativeListener>,
}

struct FakeApi {
    state: Mutex<FakeState>,
    stopped: Flag,
    switch_calls: AtomicUsize,
}

impl FakeApi {
    fn new() -> Arc<FakeApi> {
        Arc::new(FakeApi {
            state: Mutex::new(FakeState {
                names: vec!["us".to_owned(), "ru".to_owned()],
                accept_switch: true,
                window: WINDOW,
                owners: HashMap::from([(WINDOW, 4242)]),
                now_ms: 1015,
                signal_ready: true,
                ..FakeState::default()
            }),
            stopped: Flag::default(),
            switch_calls: AtomicUsize::new(0),
        })
    }
    fn st(&self) -> MutexGuard<'_, FakeState> {
        lock(&self.state)
    }
}

impl WaylandApi for FakeApi {
    fn desktop_name(&self) -> String {
        "GNOME".to_owned()
    }
    fn permission_error(&self) -> String {
        self.st().denied.clone()
    }
    fn layout_names(&self) -> Vec<String> {
        self.st().names.clone()
    }
    fn current_layout_index(&self) -> i64 {
        self.st().current_index
    }
    fn select_layout_index(&self, index: i64) -> bool {
        let call = self.switch_calls.fetch_add(1, Ordering::SeqCst) + 1;
        let mut state = self.st();
        state.switch_requests.push(index);
        if state.fail_switch_from.is_some_and(|from| call >= from) {
            return false;
        }
        if state.accept_switch {
            state.current_index = index;
        }
        state.accept_switch
    }
    fn translate_key(&self, keycode: u32, state: u32, layout_index: i64) -> String {
        if state & (CONTROL_MASK | ALT_MASK | SUPER_MASK) != 0 {
            return String::new();
        }
        let Some((english, russian, _)) = qwerty(keycode) else { return String::new() };
        let pair = if layout_index == RU { russian } else { english };
        let mut shifted = state & SHIFT_MASK != 0;
        if pystr::isalpha(pair.0) && state & LOCK_MASK != 0 {
            shifted = !shifted;
        }
        (if shifted { pair.1 } else { pair.0 }).to_owned()
    }
    fn keysym_name(&self, keycode: u32, _layout_index: i64) -> String {
        qwerty(keycode).map_or_else(String::new, |(_, _, name)| name.to_owned())
    }
    fn post_inputs(&self, inputs: &[(bool, u32)], known_seq: Option<u32>, action: bool) -> Result<u32, String> {
        let mut state = self.st();
        state.posted.push(inputs.to_vec());
        state.known_seqs.push(known_seq);
        state.actions.push(action);
        if known_seq.is_some() && state.refuse_checked {
            return Ok(0);
        }
        Ok(state.post_result.unwrap_or(inputs.len().low::<u32>()))
    }
    fn active_application(&self) -> String {
        "org.gnome.TextEditor".to_owned()
    }
    fn focused_window(&self) -> i64 {
        self.st().window
    }
    fn window_process_id(&self, window: i64) -> i64 {
        self.st().owners.get(&window).copied().unwrap_or(0)
    }
    fn current_process_id(&self) -> i64 {
        9001
    }
    fn window_origin(&self, _window: i64) -> Option<(i64, i64)> {
        None
    }
    fn window_is_password(&self, window: i64) -> bool {
        self.st().password_windows.contains(&window)
    }
    fn input_anchor(&self) -> Option<ScreenAnchor> {
        Some(ScreenAnchor { x: 10, y: 20, window: Some(WINDOW) })
    }
    fn position_window(&self, _window: i64, _x: i64, _y: i64) -> bool {
        true
    }
    fn activate_window(&self, _window: i64) -> bool {
        true
    }
    fn isolated_layout(&self) -> bool {
        false
    }
    fn caps_lock_enabled(&self) -> bool {
        self.st().caps_lock
    }
    fn key_down(&self, keycode: u32) -> bool {
        !self.st().released_keys.contains(&keycode)
    }
    fn monotonic_ms(&self) -> u32 {
        self.st().now_ms
    }
    fn run_input_listener(&self, listener: NativeListener, ready: ReadyCallback) -> Result<(), String> {
        let (error, signal) = {
            let mut state = self.st();
            state.listener = Some(listener);
            (state.listener_error.clone(), state.signal_ready)
        };
        if let Some(error) = error {
            return Err(error);
        }
        if signal {
            ready();
        }
        self.stopped.wait(Duration::from_secs(5));
        Ok(())
    }
    fn stop_input_listener(&self) {
        self.st().stop_calls += 1;
        self.stopped.set();
    }
    fn close(&self) {
        self.st().close_calls += 1;
    }
}

fn stroke(keycode: u32, character: &str, characters: &[&str], group: i32, state: u32) -> KeyEvent {
    KeyEvent {
        pressed: true,
        keycode,
        key_name: key_name(keycode, "", ""),
        character: character.to_owned(),
        characters: characters.iter().map(|text| (*text).to_owned()).collect(),
        group,
        state,
        timestamp: 1000,
        ..KeyEvent::default()
    }
}

fn letter(keycode: u32) -> KeyEvent {
    let (english, russian) = match keycode {
        EVDEV_KEY_A => ("a", "ф"),
        EVDEV_KEY_Q => ("q", "й"),
        _ => ("z", "я"),
    };
    stroke(keycode, english, &[english, russian], 0, 0)
}

fn injection<'a>(strokes: &'a [KeyEvent], target_group: i32, boundary: Option<&'a KeyEvent>) -> Injection<'a> {
    Injection { strokes, target_group, boundary, source_group: None, late: &[], trailing: &[], kept_tail: 0, erased: None }
}

fn make() -> (Arc<FakeApi>, WaylandBackend, Arc<Mutex<Vec<KeyEvent>>>) {
    let api = FakeApi::new();
    let backend = WaylandBackend::new(api.clone(), None);
    let events: Arc<Mutex<Vec<KeyEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    *lock(&backend.inner.listener) = Some(Arc::new(move |event| lock(&sink).push(event)));
    (api, backend, events)
}

fn key(code: u32, seq: u32, pressed: bool) -> NativeKeyEvent {
    NativeKeyEvent { seq, ..NativeKeyEvent::new(pressed, code, 1000) }
}

fn forced(code: u32, seq: u32) -> NativeKeyEvent {
    NativeKeyEvent { forced: true, ..key(code, seq, true) }
}

#[expect(clippy::unnecessary_wraps, reason = "the signature a key filter has")]
fn filter(predicate: impl Fn(&KeyEvent) -> KeyDisposition + Send + Sync + 'static) -> Option<KeyFilter> {
    Some(Arc::new(predicate))
}

fn error_text<T: std::fmt::Debug>(result: Result<T, BackendError>) -> String {
    result.expect_err("an error was expected").to_string()
}

#[test]
fn layouts_are_found_by_probing_the_q_key_and_keys_get_the_engines_names() {
    assert!(is_cyrillic("й") && !is_cyrillic("q") && !is_cyrillic(""));
    let api = FakeApi::new();
    let found = select_layout_indices(2, &|code, state, index| api.translate_key(code, state, index));
    assert_eq!(found.unwrap(), (EN, RU));
    let only_english = select_layout_indices(2, &|_, _, _| "q".to_owned());
    assert!(error_text(only_english).contains("английская и русская"));
    assert_eq!(key_name(EVDEV_KEY_ENTER, "x", ""), "Return");
    assert_eq!(key_name(99999, "A", ""), "a");
    assert_eq!(key_name(99999, "Cyrillic_a", ""), "Cyrillic_a");
    assert_eq!(key_name(99999, "", "Q"), "q");
    assert_eq!(key_name(99999, "", ""), "KEY_99999");
}

#[test]
fn the_probe_names_the_layouts_or_the_reason_there_are_none() {
    let (api, backend, _events) = make();
    let probe = backend.probe();
    assert!(probe.available);
    assert_eq!((probe.session_type.as_str(), probe.display.as_str()), ("wayland", "GNOME"));
    assert_eq!((probe.record_version.as_str(), probe.xtest_version.as_str(), probe.xkb_version.as_str()), ("evdev (EVIOCGRAB)", "uinput", "us,ru"));
    assert_eq!(probe.current_group, 0);
    api.st().current_index = RU;
    assert_eq!(backend.probe().current_group, 1);
    api.st().denied = "no access".to_owned();
    let denied = backend.probe();
    assert!(!denied.available && denied.error == "no access" && denied.current_group == -1 && denied.record_version == "—");
    api.st().denied.clear();
    api.st().names = vec!["us".to_owned()];
    let (api, backend, _events) = make();
    api.st().names = vec!["us".to_owned()];
    assert!(backend.probe().error.contains("английская и русская"));
}

#[test]
fn focus_window_and_group_helpers_follow_the_compositor() {
    let (api, backend, _events) = make();
    assert_eq!(backend.focused_window(), Some(FocusInfo { window: WINDOW, own: false, isolated_layout: false }));
    api.st().owners.insert(WINDOW, 9001);
    assert!(backend.focused_window().unwrap().own);
    api.st().window = 0;
    assert_eq!(backend.focused_window(), None);
    assert!(backend.restore_window(Some(WINDOW)) && !backend.restore_window(None) && !backend.restore_window(Some(0)));
    assert!(backend.swallows_hotkeys() && backend.supports_selection());
    assert_eq!(backend.current_group().unwrap(), 0);
    backend.switch_group(1).unwrap();
    assert_eq!(api.st().switch_requests, vec![RU]);
    assert!(error_text(backend.switch_group(5)).contains("Неизвестная группа"));
    // A request the compositor accepts but does not apply is reported as unconfirmed.
    api.st().accept_switch = true;
    api.st().current_index = EN;
    api.st().fail_switch_from = None;
    let calls_before = api.switch_calls.load(Ordering::SeqCst);
    api.st().accept_switch = false;
    assert!(error_text(backend.switch_group(1)).contains("отклонила"));
    assert_eq!(api.switch_calls.load(Ordering::SeqCst), calls_before + 1);
}

#[test]
fn start_reports_what_stops_it_and_stop_joins_the_thread() {
    let (api, backend, _events) = make();
    api.st().denied = "denied".to_owned();
    assert_eq!(error_text(backend.start(Arc::new(|_| {}))), "denied");
    api.st().denied.clear();
    api.st().listener_error = Some("helper failed".to_owned());
    assert_eq!(error_text(backend.start(Arc::new(|_| {}))), "helper failed");
    api.st().listener_error = None;
    backend.start(Arc::new(|_| {})).unwrap();
    assert!(backend.running());
    // A second start is no start.
    backend.start(Arc::new(|_| {})).unwrap();
    backend.stop();
    assert!(!backend.running());
    assert_eq!(api.st().stop_calls, 1);
    backend.close();
    assert_eq!(api.st().close_calls, 1);
}

#[test]
fn keys_reach_the_listener_with_their_characters_and_the_filter_decides() {
    let api = FakeApi::new();
    let backend = WaylandBackend::new(api.clone(), None);
    let seen: Arc<Mutex<Vec<KeyEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    backend.start(Arc::new(move |event| lock(&sink).push(event))).unwrap();
    backend.set_key_filter(filter(|event| if event.key_name == "Pause" { KeyDisposition::Consume } else { KeyDisposition::Pass }));
    let listener = api.st().listener.clone().expect("the backend gave its handler to the api");
    assert!(!listener(key(EVDEV_KEY_A, 1, true)));
    assert!(listener(key(EVDEV_KEY_PAUSE, 2, true)));
    // The release of a consumed key is consumed with it; the repeats of its press are not shown to the engine again.
    assert!(listener(key(EVDEV_KEY_PAUSE, 3, true)));
    assert!(listener(key(EVDEV_KEY_PAUSE, 4, false)));
    let events = lock(&seen).clone();
    assert_eq!(events.len(), 3);
    assert_eq!((events[0].character.as_str(), events[0].characters.clone(), events[0].key_name.as_str()), ("a", vec!["a".to_owned(), "ф".to_owned()], "a"));
    assert_eq!(events[0].timestamp, 1000);
    backend.stop();
    assert!(backend.take_input_delay().is_some());
}

#[test]
fn modifiers_released_unseen_are_forgotten_after_a_gap() {
    let (api, backend, _events) = make();
    let _ = backend.handle_native(key(EVDEV_KEY_LEFTSHIFT, 1, true));
    api.st().released_keys.insert(EVDEV_KEY_LEFTSHIFT);
    // Soon after, in the same window: nothing is asked of the helper.
    let _ = backend.handle_native(NativeKeyEvent { seq: 2, ..NativeKeyEvent::new(true, EVDEV_KEY_A, 1000 + MODIFIER_RECHECK_GAP_MS - 1) });
    assert!(lock(&backend.inner.state).pressed.contains(&EVDEV_KEY_LEFTSHIFT));
    let _ = backend.handle_native(NativeKeyEvent { seq: 3, ..NativeKeyEvent::new(true, EVDEV_KEY_Q, 1000 + 3 * MODIFIER_RECHECK_GAP_MS) });
    assert!(!lock(&backend.inner.state).pressed.contains(&EVDEV_KEY_LEFTSHIFT));
}

#[test]
fn a_correction_types_the_word_in_the_target_layout_and_keeps_shift_and_caps_lock_right() {
    let (api, backend, _events) = make();
    // Physically held Shift, Caps Lock on, and a boundary the target layout has to keep in its own layout.
    lock(&backend.inner.state).pressed.insert(EVDEV_KEY_LEFTSHIFT);
    api.st().caps_lock = true;
    let shifted = letter(EVDEV_KEY_A);
    let shifted = KeyEvent { state: SHIFT_MASK, ..shifted };
    let plain = letter(EVDEV_KEY_Q);
    let semicolon = stroke(EVDEV_KEY_SEMICOLON, ";", &[";", "ж"], 0, 0);
    let strokes = [shifted.clone(), plain.clone()];
    let held = backend.inject_correction(Injection { source_group: Some(0), ..injection(&strokes, 1, Some(&semicolon)) }).unwrap();
    assert_eq!(held, 0);
    let posted = api.st().posted.clone();
    assert_eq!(posted.len(), 3);
    // The first batch releases the held Shift, the last types it down again.
    assert_eq!(posted[0][0], (false, EVDEV_KEY_LEFTSHIFT));
    assert_eq!(*posted[2].last().unwrap(), (true, EVDEV_KEY_LEFTSHIFT));
    lock(&backend.inner.state).pressed.clear();
    api.st().caps_lock = false;

    // Nothing to type is nothing posted; a boundary that stays in the target layout goes in the first batch.
    api.st().posted.clear();
    backend.inject_correction(injection(&[], 1, None)).unwrap();
    assert!(api.st().posted.is_empty());
    let space = stroke(EVDEV_KEY_SPACE, " ", &[" ", " "], 0, 0);
    backend.inject_correction(injection(&[letter(EVDEV_KEY_A)], 0, Some(&space))).unwrap();

    // The kept tail is typed again in the source group, and the layout ends there.
    api.st().posted.clear();
    api.st().current_index = RU;
    let word = stroke(EVDEV_KEY_A, "ф", &["a", "ф"], 1, 0);
    let blank = stroke(EVDEV_KEY_SPACE, " ", &[" ", " "], 1, 0);
    let tail = stroke(EVDEV_KEY_Q, "й", &["q", "й"], 1, 0);
    let strokes = [word, blank.clone(), tail];
    backend.inject_correction(Injection { source_group: Some(1), kept_tail: 1, ..injection(&strokes, 0, Some(&blank)) }).unwrap();
    assert_eq!(api.st().current_index, RU);
    for (source_group, kept_tail) in [(Some(1), 3), (None, 1)] {
        let refused = backend.inject_correction(Injection { source_group, kept_tail, ..injection(&strokes, 0, Some(&blank)) });
        assert!(error_text(refused).contains("сохраняемого слова"));
    }

    // Groups that do not exist are refused before anything is typed.
    assert!(error_text(backend.inject_correction(injection(&[], 7, None))).contains("Неизвестная группа"));
    let strange = KeyEvent { group: 9, ..letter(EVDEV_KEY_A) };
    assert!(error_text(backend.inject_correction(injection(&[strange], 0, None))).contains("исходная группа"));
    let bad_semicolon = stroke(EVDEV_KEY_SEMICOLON, ";", &[";", "ж"], 9, 0);
    let strokes = [letter(EVDEV_KEY_A)];
    let refused = backend.inject_correction(Injection { source_group: Some(0), ..injection(&strokes, 0, Some(&bad_semicolon)) });
    assert!(error_text(refused).contains("раскладка пунктуации"));

    // Focus changed during a hold aborts before anything is deleted.
    backend.hold_input();
    api.st().window = SECOND_WINDOW;
    let refused = backend.inject_correction(injection(&[letter(EVDEV_KEY_A)], 0, None));
    assert!(error_text(refused).contains("Место ввода изменилось до замены"));
    api.st().window = WINDOW;

    // A short answer from uinput is an error.
    api.st().post_result = Some(1);
    assert!(error_text(backend.inject_correction(injection(&[letter(EVDEV_KEY_A)], 0, None))).contains("uinput принял 1 из"));
    api.st().post_result = None;
}

#[test]
fn a_failure_after_deleting_puts_the_late_keys_back_and_the_first_failure_stays() {
    let (api, backend, _events) = make();
    let semicolon = stroke(EVDEV_KEY_SEMICOLON, ";", &[";", "ж"], 0, 0);
    api.st().fail_switch_from = Some(api.switch_calls.load(Ordering::SeqCst) + 2);
    let late = [letter(EVDEV_KEY_Z)];
    let strokes = [letter(EVDEV_KEY_A)];
    let result = backend.inject_correction(Injection { source_group: Some(0), late: &late, ..injection(&strokes, 1, Some(&semicolon)) });
    assert!(error_text(result).contains("отклонила"));
    let posted = api.st().posted.clone();
    assert_eq!(posted[0].iter().filter(|(_, code)| *code == EVDEV_KEY_BACKSPACE).count(), 6);
    // The keys typed after the word are put back one by one: they are replayed, not injected.
    assert_eq!(posted[posted.len() - 2..], [vec![(true, EVDEV_KEY_Z)], vec![(false, EVDEV_KEY_Z)]]);
}

#[test]
fn a_sign_the_target_layout_has_is_typed_there_without_switching_back() {
    let (api, backend, _events) = make();
    let comma = stroke(EVDEV_KEY_COMMA, ",", &[",", "б"], 0, 0);
    let strokes = [letter(EVDEV_KEY_A)];
    let started = Instant::now();
    backend.inject_correction(Injection { source_group: Some(0), ..injection(&strokes, 1, Some(&comma)) }).unwrap();
    assert!(started.elapsed() < seconds(WAYLAND_TYPED_INPUT_SETTLE_SECONDS));
    assert_eq!(api.st().switch_requests, vec![RU]);
    let posted = api.st().posted.clone();
    assert_eq!(posted.len(), 1);
    let shifted_comma = [(true, EVDEV_KEY_LEFTSHIFT), (true, EVDEV_KEY_SLASH), (false, EVDEV_KEY_SLASH), (false, EVDEV_KEY_LEFTSHIFT)];
    assert_eq!(posted[0][posted[0].len() - 4..], shifted_comma);
}

#[test]
fn signs_after_a_word_are_typed_with_the_keys_the_target_layout_has_them_on() {
    let (_api, backend, _events) = make();
    let comma = stroke(EVDEV_KEY_COMMA, ",", &[",", "б"], 0, 0);
    let period = stroke(EVDEV_KEY_SLASH, ".", &["/", "."], 1, 0);
    let literal = backend.inner.typed_in_group(std::slice::from_ref(&comma), 1).unwrap().unwrap();
    let twice = backend.inner.typed_in_group(&[comma.clone(), comma], 1).unwrap().unwrap();
    assert_eq!(twice, [literal.clone(), literal].concat());
    // Found on its own key, unshifted.
    assert_eq!(
        backend.inner.typed_in_group(&[period], 1).unwrap().unwrap(),
        [NativeInput::synthetic(true, EVDEV_KEY_SLASH), NativeInput::synthetic(false, EVDEV_KEY_SLASH)]
    );
    // A character no key gives is no reason to look further.
    assert_eq!(backend.inner.typed_in_group(&[stroke(EVDEV_KEY_A, "", &["a", "ф"], 0, 0)], 1).unwrap(), None);
}

#[test]
fn a_sign_missing_from_the_target_layout_waits_before_each_switch() {
    let (api, backend, _events) = make();
    let semicolon = stroke(EVDEV_KEY_SEMICOLON, ";", &[";", "ж"], 0, 0);
    let strokes = [letter(EVDEV_KEY_A)];
    let started = Instant::now();
    backend.inject_correction(Injection { source_group: Some(0), ..injection(&strokes, 1, Some(&semicolon)) }).unwrap();
    assert!(started.elapsed() >= seconds(2.0 * WAYLAND_TYPED_INPUT_SETTLE_SECONDS));
    assert_eq!(api.st().switch_requests, vec![RU, EN, RU]);
    // Already there: no wait and no switch.
    let started = Instant::now();
    backend.inner.switch_group_after_typing(backend.inner.current_group().unwrap()).unwrap();
    assert!(started.elapsed() < seconds(WAYLAND_TYPED_INPUT_SETTLE_SECONDS));
}

#[test]
fn a_held_key_typed_by_the_helper_leaves_the_hold_and_the_correction_is_refused() {
    let (api, backend, events) = make();
    let _ = backend.handle_native(key(EVDEV_KEY_A, SEQ_FIRST, true));
    backend.hold_input();
    assert_eq!(lock(&backend.inner.state).hold_known_seq, SEQ_FIRST);
    assert!(backend.handle_native(key(EVDEV_KEY_Q, SEQ_SECOND, true)));
    let seen = lock(&events).len();
    assert!(!backend.handle_native(forced(EVDEV_KEY_Q, SEQ_SECOND)));
    assert!(lock(&backend.inner.state).held.is_empty());
    let after: Vec<u32> = lock(&events)[seen..].iter().map(|event| event.keycode).collect();
    assert_eq!(after, vec![EVDEV_KEY_Q]);
    assert_eq!(lock(&backend.inner.state).known_seq, SEQ_SECOND);
    assert_eq!(error_text(backend.inject_correction(injection(&[letter(EVDEV_KEY_A)], 1, None))), INPUT_AHEAD_ERROR);
    assert!(api.st().switch_requests.is_empty());
    assert!(!lock(&backend.inner.state).holding);

    // A forced key whose answer is gone, or was a pass, changes nothing.
    let _ = backend.handle_native(forced(EVDEV_KEY_A, SEQ_THIRD));
    let _ = backend.handle_native(key(EVDEV_KEY_A, SEQ_THIRD, true));
    let _ = backend.handle_native(forced(EVDEV_KEY_A, SEQ_THIRD));
    assert!(!lock(&backend.inner.state).hold_broken);
    // A held key already typed again before the helper's word came.
    backend.hold_input();
    assert!(backend.handle_native(key(EVDEV_KEY_Z, SEQ_FIRST, true)));
    lock(&backend.inner.state).held.clear();
    let _ = backend.handle_native(forced(EVDEV_KEY_Z, SEQ_FIRST));
    assert!(!lock(&backend.inner.state).hold_broken);
}

#[test]
fn a_consumed_hotkey_typed_by_the_helper_lets_its_release_through() {
    let (_api, backend, _events) = make();
    backend.set_key_filter(filter(|event| if event.key_name == "Pause" { KeyDisposition::Consume } else { KeyDisposition::Pass }));
    assert!(backend.handle_native(key(EVDEV_KEY_PAUSE, SEQ_FIRST, true)));
    let _ = backend.handle_native(forced(EVDEV_KEY_PAUSE, SEQ_FIRST));
    assert!(!backend.handle_native(key(EVDEV_KEY_PAUSE, SEQ_SECOND, false)));
}

fn defer_enter() -> Option<KeyFilter> {
    filter(|event| if event.key_name == "Return" { KeyDisposition::Defer } else { KeyDisposition::Pass })
}

#[test]
fn a_withheld_enter_typed_by_the_helper_is_not_typed_again() {
    let (api, backend, _events) = make();
    backend.set_key_filter(defer_enter());
    assert!(backend.handle_native(key(EVDEV_KEY_ENTER, SEQ_FIRST, true)));
    // The helper reports the forced key before any later one (its queue is typed after it).
    let _ = backend.handle_native(forced(EVDEV_KEY_ENTER, SEQ_FIRST));
    assert!(lock(&backend.inner.state).deferred_action.is_none());
    assert!(!backend.handle_native(key(EVDEV_KEY_ENTER, SEQ_SECOND, false)));
    assert_eq!(error_text(backend.inject_correction(injection(&[letter(EVDEV_KEY_A)], 1, None))), INPUT_AHEAD_ERROR);
    // The failed correction ended the hold; the action handed back types nothing.
    assert!(lock(&backend.inner.state).action_forced);
    assert_eq!(backend.complete_action(true).unwrap(), 0);
    assert!(!lock(&backend.inner.state).action_forced);
    assert!(api.st().posted.is_empty());
    assert!(!lock(&backend.inner.state).holding);
    assert_eq!(backend.complete_action(true).unwrap(), 0);
}

#[test]
fn a_withheld_enter_is_typed_checked_against_its_own_key() {
    let (api, backend, _events) = make();
    backend.set_key_filter(defer_enter());
    assert!(backend.handle_native(key(EVDEV_KEY_ENTER, SEQ_FIRST, true)));
    assert_eq!(backend.complete_action(true).unwrap(), COMPLETED_ACTION_EVENT_COUNT);
    let (last_seq, last_action) = {
        let state = api.st();
        (state.known_seqs.last().copied().flatten(), state.actions.last().copied())
    };
    assert_eq!((last_seq, last_action), (Some(SEQ_FIRST), Some(true)));
    assert!(backend.handle_native(key(EVDEV_KEY_ENTER, SEQ_FIRST, false)));
    // The helper typed it itself first: refused, and not typed twice.
    assert!(backend.handle_native(key(EVDEV_KEY_ENTER, SEQ_SECOND, true)));
    api.st().refuse_checked = true;
    assert_eq!(error_text(backend.complete_action(true)), INPUT_AHEAD_ERROR);
    assert!(!lock(&backend.inner.state).holding);
    // Its late word, after the action was given up, only lets the release through.
    api.st().refuse_checked = false;
    assert!(backend.handle_native(key(EVDEV_KEY_ENTER, SEQ_SECOND, false)));
    assert!(backend.handle_native(key(EVDEV_KEY_ENTER, SEQ_THIRD, true)));
    assert_eq!(backend.complete_action(false).unwrap(), 0);
    let _ = backend.handle_native(forced(EVDEV_KEY_ENTER, SEQ_THIRD));
    assert!(!lock(&backend.inner.state).action_forced);
    assert!(!lock(&backend.inner.state).consumed_keys.contains(&EVDEV_KEY_ENTER));
}

#[test]
fn keys_typed_unasked_reach_the_engine_and_spoil_a_hold() {
    let (_api, backend, events) = make();
    backend.set_key_filter(filter(|_| KeyDisposition::Consume));
    lock(&backend.inner.state).consumed_keys.insert(EVDEV_KEY_Q);
    assert!(!backend.handle_native(NativeKeyEvent { delivered: true, ..key(EVDEV_KEY_A, SEQ_FIRST, true) }));
    assert_eq!(lock(&events).last().unwrap().keycode, EVDEV_KEY_A);
    backend.hold_input();
    assert!(!backend.handle_native(NativeKeyEvent { delivered: true, ..key(EVDEV_KEY_Q, SEQ_SECOND, false) }));
    let state = lock(&backend.inner.state);
    assert!(state.hold_broken && !state.consumed_keys.contains(&EVDEV_KEY_Q));
    assert_eq!(state.known_seq, SEQ_SECOND);
}

#[test]
fn a_correction_the_helper_refuses_changes_no_text() {
    let (api, backend, _events) = make();
    let _ = backend.handle_native(key(EVDEV_KEY_A, SEQ_FIRST, true));
    backend.hold_input();
    api.st().refuse_checked = true;
    assert_eq!(error_text(backend.inject_correction(injection(&[letter(EVDEV_KEY_A)], 1, None))), INPUT_AHEAD_ERROR);
    assert_eq!(api.st().known_seqs, vec![Some(SEQ_FIRST)]);
}

#[test]
fn a_grab_change_forgets_held_keys_and_resets_the_word() {
    let (_api, backend, events) = make();
    backend.set_key_filter(filter(|event| if event.key_name == "Pause" { KeyDisposition::Consume } else { KeyDisposition::Pass }));
    let _ = backend.handle_native(key(EVDEV_KEY_LEFTSHIFT, SEQ_FIRST, true));
    let _ = backend.handle_native(key(EVDEV_KEY_PAUSE, SEQ_SECOND, true));
    backend.hold_input();
    assert!(!backend.handle_native(NativeKeyEvent { input_reset: true, ..NativeKeyEvent::new(true, 0, 1000) }));
    {
        let state = lock(&backend.inner.state);
        assert!(state.pressed.is_empty() && state.consumed_keys.is_empty() && state.hold_broken);
    }
    assert_eq!(lock(&events).last().unwrap().group, POINTER_EVENT_GROUP);
    backend.release_input().unwrap();
    let _ = backend.handle_native(NativeKeyEvent { input_reset: true, ..NativeKeyEvent::new(true, 0, 1000) });
    assert!(!lock(&backend.inner.state).hold_broken);
}

#[test]
fn releases_of_keys_down_when_a_hold_began_pass_but_not_a_modifier_or_a_held_press() {
    let (_api, backend, _events) = make();
    for code in [EVDEV_KEY_A, EVDEV_KEY_Q, EVDEV_KEY_LEFTSHIFT] {
        let _ = backend.handle_native(key(code, SEQ_FIRST, true));
    }
    backend.hold_input();
    // The client repeats a key whose release is kept back: the release goes on.
    assert!(!backend.handle_native(key(EVDEV_KEY_A, SEQ_SECOND, false)));
    // Shift's release would change the case of what the correction types: it waits.
    assert!(backend.handle_native(key(EVDEV_KEY_LEFTSHIFT, SEQ_SECOND, false)));
    // Pressed again during the hold, its release belongs to the held press.
    assert!(backend.handle_native(key(EVDEV_KEY_Q, SEQ_THIRD, true)));
    assert!(backend.handle_native(key(EVDEV_KEY_Q, SEQ_THIRD, false)));
    backend.release_input().unwrap();
    assert!(lock(&backend.inner.state).hold_prior_keys.is_empty());
}

#[test]
fn held_keys_are_replayed_in_order_when_the_hold_ends() {
    let (api, backend, events) = make();
    backend.hold_input();
    assert!(backend.handle_native(key(EVDEV_KEY_A, SEQ_FIRST, true)));
    assert!(backend.handle_native(key(EVDEV_KEY_A, SEQ_FIRST, false)));
    assert!(lock(&events).is_empty());
    assert_eq!(backend.release_input().unwrap(), 2);
    assert_eq!(api.st().posted, vec![vec![(true, EVDEV_KEY_A)], vec![(false, EVDEV_KEY_A)]]);
    assert_eq!(lock(&events).len(), 2);
    assert!(!lock(&backend.inner.state).holding);
    // A replayed key the filter consumes is not typed.
    backend.hold_input();
    let _ = backend.handle_native(key(EVDEV_KEY_Q, SEQ_SECOND, true));
    backend.set_key_filter(filter(|event| if event.key_name == "q" { KeyDisposition::Consume } else { KeyDisposition::Pass }));
    backend.release_input().unwrap();
    assert_eq!(api.st().posted.len(), 2);
}

#[test]
fn a_password_prompt_the_compositor_draws_refuses_the_selection_and_is_known_as_one() {
    let (api, backend, _events) = make();
    api.st().password_windows.insert(SECOND_WINDOW);
    assert!(backend.window_is_password(SECOND_WINDOW) && !backend.window_is_password(WINDOW));
    let refused = backend.read_selection("gnome-shell", SECOND_WINDOW);
    assert!(matches!(refused, Err(SelectionError::Refused(reason)) if reason == SelectionRefused::new(RefusalReason::SensitiveField)));
    // With no reader of accessibility the selection is unsupported.
    let unsupported = backend.read_selection("app", WINDOW);
    assert!(matches!(unsupported, Err(SelectionError::Refused(reason)) if reason == SelectionRefused::new(RefusalReason::Unsupported)));
}

#[test]
fn a_selection_is_typed_run_by_run_in_the_layouts_that_have_its_characters() {
    let (api, backend, _events) = make();
    let selected = SelectedText { text: "q".to_owned(), source: "atspi".to_owned() };
    let typed = backend.replace_selection(&selected, "aй ", Some(0)).unwrap();
    assert!(typed);
    let posted = api.st().posted.clone();
    // Each run goes on its own keys, the layout is switched between runs, and the typed text is selected back.
    assert_eq!(posted[0], vec![(true, EVDEV_KEY_A), (false, EVDEV_KEY_A)]);
    assert_eq!(api.st().switch_requests, vec![RU, EN]);
    assert_eq!(posted.last().unwrap()[0], (true, EVDEV_KEY_LEFTSHIFT));
    // A character no key of either layout gives leaves the text as it is.
    api.st().posted.clear();
    let refused = backend.replace_selection(&selected, "q€", None);
    assert!(matches!(refused, Err(SelectionError::Refused(reason)) if reason == SelectionRefused::new(RefusalReason::Untypeable)));
    assert!(api.st().posted.is_empty());
}

// ---------------------------------------------------------------- a long walk

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

const WALK_KEYS: [u32; 43] = [
    EVDEV_KEY_A, EVDEV_KEY_Q, EVDEV_KEY_Z, KEY_G, EVDEV_KEY_SEMICOLON, EVDEV_KEY_COMMA, EVDEV_KEY_SLASH, EVDEV_KEY_SPACE, EVDEV_KEY_ENTER, EVDEV_KEY_KPENTER, EVDEV_KEY_TAB, EVDEV_KEY_ESC,
    EVDEV_KEY_BACKSPACE, EVDEV_KEY_CAPSLOCK, EVDEV_KEY_LEFTSHIFT, EVDEV_KEY_RIGHTSHIFT, EVDEV_KEY_LEFTCTRL, EVDEV_KEY_RIGHTCTRL, EVDEV_KEY_LEFTALT, EVDEV_KEY_RIGHTALT, EVDEV_KEY_LEFTMETA,
    EVDEV_KEY_RIGHTMETA, EVDEV_KEY_HOME, EVDEV_KEY_END, EVDEV_KEY_PAGEUP, EVDEV_KEY_PAGEDOWN, EVDEV_KEY_LEFT, EVDEV_KEY_RIGHT, EVDEV_KEY_UP, EVDEV_KEY_DOWN, EVDEV_KEY_INSERT, EVDEV_KEY_DELETE,
    EVDEV_KEY_EQUAL, EVDEV_KEY_MINUS, EVDEV_KEY_DOT, EVDEV_KEY_GRAVE, EVDEV_KEY_LEFTBRACE, EVDEV_KEY_BACKSLASH, EVDEV_KEY_RIGHTBRACE, EVDEV_KEY_APOSTROPHE, EVDEV_KEY_PAUSE, 2, 999,
];

fn walk_stroke(dice: &mut Dice) -> KeyEvent {
    let code = *dice.of(&WALK_KEYS[..WALK_KEYS.len() - 1]);
    let (plain, shifted) = qwerty(code).map_or(("x", "X"), |spec| (spec.0.0, spec.1.0));
    let group = *dice.of(&[0, 0, 0, 1, 1, 1, -1, 5]);
    let state = *dice.of(&[0, 0, 0, SHIFT_MASK, LOCK_MASK, CONTROL_MASK]);
    stroke(code, plain, &[plain, shifted], group, state)
}

fn walk_injection(backend: &WaylandBackend, dice: &mut Dice) {
    let strokes: Vec<KeyEvent> = (0..dice.below(5)).map(|_| walk_stroke(dice)).collect();
    let trailing: Vec<KeyEvent> = (0..dice.below(2)).map(|_| walk_stroke(dice)).collect();
    let late: Vec<KeyEvent> = (0..dice.below(2)).map(|_| walk_stroke(dice)).collect();
    let boundary = dice.one_in(2).then(|| walk_stroke(dice));
    let source_group = *dice.of(&[None, None, Some(0), Some(1), Some(-1), Some(9)]);
    let erased = *dice.of(&[None, None, Some(1), Some(2)]);
    let _ = backend.inject_correction(Injection {
        strokes: &strokes,
        target_group: *dice.of(&[0, 1, 1, 0, -1, 7]),
        boundary: boundary.as_ref(),
        source_group,
        late: &late,
        trailing: &trailing,
        kept_tail: *dice.of(&[0, 0, 0, 1, 9]),
        erased,
    });
}

#[test]
fn a_long_walk_through_the_backend_stops_nowhere() {
    for seed in 1..=WALKS {
        let mut dice = Dice(0x9e37_79b9_7f4a_7c15 ^ seed.wrapping_mul(0xbf58_476d_1ce4_e5b9));
        let (api, backend, _events) = make();
        let mut sequence = 1;
        for _ in 0..STEPS {
            sequence += 1;
            let code = *dice.of(&WALK_KEYS);
            match dice.below(36) {
                0..=13 => {
                    let _ = backend.handle_native(key(code, sequence, true));
                    let _ = backend.handle_native(key(code, sequence + 1, false));
                    sequence += 1;
                }
                14..=16 => {
                    let _ = backend.handle_native(key(code, sequence, true));
                }
                17 => {
                    let _ = backend.handle_native(NativeKeyEvent { input_reset: true, ..key(code, sequence, true) });
                }
                18 => {
                    let _ = backend.handle_native(forced(code, sequence));
                }
                19 => {
                    let _ = backend.handle_native(NativeKeyEvent { pointer: true, ..key(0, sequence, true) });
                }
                20 => {
                    let _ = backend.handle_native(NativeKeyEvent { repeat_reset: true, ..key(0, sequence, true) });
                }
                21 => {
                    let _ = backend.handle_native(NativeKeyEvent { injected: true, ..key(code, sequence, dice.one_in(2)) });
                }
                22 => {
                    let _ = backend.handle_native(NativeKeyEvent { replayed: true, ..key(code, sequence, dice.one_in(2)) });
                }
                23..=25 => backend.hold_input(),
                26 => {
                    let _ = backend.release_input();
                }
                27 => {
                    let _ = backend.complete_action(dice.one_in(2));
                }
                28 => walk_injection(&backend, &mut dice),
                29 => match dice.below(4) {
                    0 => backend.set_key_filter(defer_enter()),
                    1 => backend.set_key_filter(filter(|event| if event.key_name == "Pause" { KeyDisposition::Consume } else { KeyDisposition::Pass })),
                    2 => backend.set_key_filter(filter(|_| KeyDisposition::Defer)),
                    _ => backend.set_key_filter(None),
                },
                30 => api.st().window = *dice.of(&[WINDOW, SECOND_WINDOW]),
                31 => api.st().caps_lock = dice.one_in(2),
                32 => {
                    let mut state = api.st();
                    state.refuse_checked = dice.one_in(4);
                    state.post_result = *dice.of(&[None, None, None, Some(0), Some(1)]);
                    state.fail_switch_from = None;
                }
                33 => {
                    api.st().released_keys.insert(code);
                    let _ = backend.handle_native(key(EVDEV_KEY_A, sequence, true));
                    api.st().now_ms += 100_000;
                }
                34 => {
                    let _ = backend.switch_group(*dice.of(&[0, 1, -1, 9]));
                }
                _ => {
                    let _ = (backend.current_group(), backend.active_application(), backend.focused_window(), backend.probe());
                }
            }
        }
        let _ = backend.complete_action(false);
        let _ = backend.release_input();
    }
}

const WALKS: u64 = 300;
const STEPS: usize = 60;

#[test]
fn every_key_of_the_table_has_its_name() {
    for code in WALK_KEYS {
        let _ = key_name(code, "", "");
    }
    assert_eq!(key_name(EVDEV_KEY_TAB, "", ""), "Tab");
    assert_eq!(key_name(EVDEV_KEY_Q, "q", "q"), "q");
    assert_eq!(key_name(EVDEV_KEY_Q, "Cyrillic_shorti", "й"), "Cyrillic_shorti");
    assert_eq!(key_name(999, "", "z"), "z");
    assert_eq!(key_name(999, "", ""), "KEY_999");
    assert!(select_layout_indices(1, &|_, _, _| "й".to_owned()).is_err());
    assert_eq!(select_layout_indices(3, &|_, _, index| ["é", "q", "й"][index.to::<usize>()].to_owned()).ok(), Some((1, 2)));
}
