//! The backend over a fake server: the tests of the Python's `test_x11_backend.py` that pin a decision.

use super::*;
use crate::xkb_keymap::{KeyMap, KeySyms, KeyType, TypeEntry};
use ks_backend::KeyDisposition;
use ks_backend::selected_text::SelectionError;

const A: u8 = 38;
const COMMA: u8 = 59;
/// The key of `/` in the US layout and of `.` and `,` in the Russian one.
const SLASH: u8 = 61;
const SPACE: u8 = 65;
const BACKSPACE: u8 = 22;
const SHIFT_L: u8 = 50;
const LEFT: u8 = 113;
const RETURN: u8 = 36;
const TAB: u8 = 23;
const BACK_TAB: u8 = 24;
const EMOJI: u8 = 25;
const SURROGATE: u8 = 26;
const LINE_FEED: u8 = 27;
const SHIFT: u32 = SHIFT_MASK;

fn sym(keysyms: &[u32]) -> Vec<u32> {
    keysyms.to_vec()
}

/// A `us,ru` keyboard of a few keys.
fn keymap() -> KeyMap {
    let alphabetic = KeyType {
        mods_mask: 0x3,
        num_levels: 2,
        entries: vec![TypeEntry { active: true, mods_mask: 1, level: 1 }, TypeEntry { active: true, mods_mask: 2, level: 1 }],
    };
    let one_level = KeyType { mods_mask: 0, num_levels: 1, entries: vec![] };
    let two_level = KeyType { mods_mask: 0x1, num_levels: 2, entries: vec![TypeEntry { active: true, mods_mask: 1, level: 1 }] };
    let mut keys = vec![KeySyms::default(); 120 - 8 + 1];
    let mut put = |keycode: u8, kt: u8, groups: u8, width: u8, syms: Vec<u32>| {
        keys[usize::from(keycode - 8)] = KeySyms { kt_index: [kt, kt, kt, kt], group_info: groups, width, syms };
    };
    put(A, 0, 2, 2, sym(&[0x61, 0x41, 0x6c6, 0x6e6]));
    put(COMMA, 2, 2, 2, sym(&[0x2c, 0x3c, 0x6c2, 0x6e2]));
    put(SLASH, 2, 2, 2, sym(&[0x2f, 0x3f, 0x2e, 0x2c]));
    put(SPACE, 1, 1, 1, sym(&[0x20]));
    put(BACKSPACE, 1, 1, 1, sym(&[0xff08]));
    put(SHIFT_L, 1, 1, 1, sym(&[0xffe1]));
    put(LEFT, 1, 1, 1, sym(&[0xff51]));
    put(RETURN, 1, 1, 1, sym(&[0xff0d]));
    put(TAB, 1, 1, 1, sym(&[0xff09]));
    put(BACK_TAB, 1, 1, 1, sym(&[0xfe20]));
    put(EMOJI, 1, 1, 1, sym(&[0x0101_f600]));
    put(SURROGATE, 1, 1, 1, sym(&[0x0100_d800]));
    put(LINE_FEED, 1, 1, 1, sym(&[0xff0a]));
    KeyMap { min_keycode: 8, max_keycode: 120, types: vec![alphabetic, one_level, two_level], keys }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Call {
    Lock(u8),
    Fake(bool, u8),
    Grab(bool),
    Sync,
    Flush,
}

#[derive(Default)]
struct FakeState {
    calls: Vec<Call>,
    locks_seen: usize,
    fakes_seen: usize,
    /// The n-th (from 0) lock or fake call that fails.
    fail_lock: Option<usize>,
    fail_fake: Option<usize>,
    group: u8,
    locked_mods: u8,
    chunks: Vec<RecordChunk>,
    block_record: bool,
    disabled: bool,
    record_error: bool,
    record_version_error: bool,
    focus: Option<u32>,
    parents: HashMap<u32, u32>,
    pids: HashMap<u32, u32>,
    classes: HashMap<u32, (String, String)>,
    pid_queries: usize,
    no_context: bool,
    no_keyboard_state: bool,
    missing_keysyms: Vec<u32>,
}

struct Fake {
    map: KeyMap,
    state: Mutex<FakeState>,
    wake: Condvar,
}

impl Fake {
    fn new() -> Arc<Fake> {
        Arc::new(Fake { map: keymap(), state: Mutex::new(FakeState::default()), wake: Condvar::new() })
    }

    fn with<R>(&self, work: impl FnOnce(&mut FakeState) -> R) -> R {
        work(&mut lock(&self.state))
    }

    fn calls(&self) -> Vec<Call> {
        self.with(|state| state.calls.clone())
    }

    fn backend(self: &Arc<Self>) -> X11Backend {
        let api = self.clone();
        X11Backend::new(Box::new(move || Ok(api.clone())), None)
    }
}

impl X11Api for Fake {
    fn xkb_version(&self) -> String {
        "1.0".to_owned()
    }
    fn record_version(&self) -> Result<(u16, u16), String> {
        if self.with(|state| state.record_version_error) { Err("no".to_owned()) } else { Ok((1, 13)) }
    }
    fn xtest_version(&self) -> Result<(u8, u16), String> {
        Ok((2, 2))
    }
    fn record_create_context(&self) -> Result<u32, String> {
        Ok(if self.with(|state| state.no_context) { 0 } else { 77 })
    }
    fn record_enable(&self, _context: u32, sink: &mut dyn FnMut(RecordChunk)) -> Result<(), String> {
        for chunk in self.with(|state| std::mem::take(&mut state.chunks)) {
            sink(chunk);
        }
        let mut state = lock(&self.state);
        while state.block_record && !state.disabled {
            state = self.wake.wait(state).unwrap_or_else(PoisonError::into_inner);
        }
        if state.record_error { Err("broken".to_owned()) } else { Ok(()) }
    }
    fn record_disable(&self, _context: u32) {
        self.with(|state| state.disabled = true);
        self.wake.notify_all();
    }
    fn record_free(&self, _context: u32) {}
    fn sync(&self) {
        self.with(|state| state.calls.push(Call::Sync));
    }
    fn flush(&self) {
        self.with(|state| state.calls.push(Call::Flush));
    }
    fn keysym(&self, keycode: u8, group: u8, level: u8) -> u32 {
        self.map.keysym(keycode, group, level)
    }
    fn lookup_keysym(&self, keycode: u8, state: u16) -> Option<u32> {
        self.map.lookup_keysym(keycode, state)
    }
    fn keycode_for_keysym(&self, keysym: u32) -> u8 {
        if self.with(|state| state.missing_keysyms.contains(&keysym)) {
            return 0;
        }
        self.map.keycode_for_keysym(keysym)
    }
    fn xkb_state(&self) -> Option<XkbState> {
        if self.with(|state| state.no_keyboard_state) {
            return None;
        }
        self.with(|state| Some(XkbState { group: state.group, locked_mods: state.locked_mods }))
    }
    fn lock_group(&self, group: u8) -> bool {
        self.with(|state| {
            let index = state.locks_seen;
            state.locks_seen += 1;
            state.calls.push(Call::Lock(group));
            state.group = group;
            state.fail_lock != Some(index)
        })
    }
    fn fake_key(&self, pressed: bool, keycode: u8) -> bool {
        self.with(|state| {
            let index = state.fakes_seen;
            state.fakes_seen += 1;
            state.calls.push(Call::Fake(pressed, keycode));
            state.fail_fake != Some(index)
        })
    }
    fn grab_control(&self, impervious: bool) {
        self.with(|state| state.calls.push(Call::Grab(impervious)));
    }
    fn input_focus(&self) -> Option<u32> {
        self.with(|state| state.focus)
    }
    fn pointer(&self) -> Option<(i32, i32)> {
        Some((10, 20))
    }
    fn parent_window(&self, window: u32) -> Option<u32> {
        self.with(|state| state.parents.get(&window).copied())
    }
    fn window_pid(&self, window: u32) -> u32 {
        self.with(|state| {
            state.pid_queries += 1;
            state.pids.get(&window).copied().unwrap_or(0)
        })
    }
    fn class_hint(&self, window: u32) -> Option<(String, String)> {
        self.with(|state| state.classes.get(&window).cloned())
    }
    fn set_input_focus(&self, _window: u32) {}
    fn move_raise(&self, _window: u32, _x: i32, _y: i32) {}
}

fn payload(event_type: u8, keycode: u8, state: u16, timestamp: u32) -> Vec<u8> {
    let mut bytes = vec![0u8; 32];
    bytes[0] = event_type;
    bytes[1] = keycode;
    bytes[4..8].copy_from_slice(&timestamp.to_ne_bytes());
    bytes[28..30].copy_from_slice(&state.to_ne_bytes());
    bytes
}

fn stroke(shifted: bool) -> KeyEvent {
    KeyEvent {
        pressed: true,
        keycode: u32::from(A),
        key_name: "a".to_owned(),
        character: if shifted { "A" } else { "a" }.to_owned(),
        characters: vec!["a".to_owned(), "ф".to_owned()],
        group: 0,
        state: if shifted { SHIFT } else { 0 },
        timestamp: 1,
        ..KeyEvent::default()
    }
}

fn key(keycode: u8, name: &str, character: &str, group: i32, state: u32) -> KeyEvent {
    let both = vec![character.to_owned(), character.to_owned()];
    KeyEvent { pressed: true, keycode: u32::from(keycode), key_name: name.to_owned(), character: character.to_owned(), characters: both, group, state, timestamp: 1, ..KeyEvent::default() }
}

fn comma(group: i32, state: u32) -> KeyEvent {
    KeyEvent { characters: vec![",".to_owned(), "б".to_owned()], group, ..key(COMMA, "comma", ",", group, state) }
}

fn tap(keycode: u8) -> [Call; 2] {
    [Call::Fake(true, keycode), Call::Fake(false, keycode)]
}

fn shifted_tap(keycode: u8) -> [Call; 4] {
    [Call::Fake(true, SHIFT_L), Call::Fake(true, keycode), Call::Fake(false, keycode), Call::Fake(false, SHIFT_L)]
}

/// The calls that type: locks and fakes, without the grab, flush and sync around them.
fn typing(fake: &Fake) -> Vec<Call> {
    fake.calls().into_iter().filter(|call| matches!(call, Call::Lock(_) | Call::Fake(..))).collect()
}

fn injection<'a>(strokes: &'a [KeyEvent], target_group: i32, boundary: Option<&'a KeyEvent>) -> Injection<'a> {
    Injection { strokes, target_group, boundary, source_group: None, late: &[], trailing: &[], kept_tail: 0, erased: None }
}

fn opened(fake: &Arc<Fake>) -> X11Backend {
    let backend = fake.backend();
    backend.shared.open().expect("fake server opens");
    backend
}

fn expected_len(backend: &X11Backend) -> usize {
    lock(&backend.shared.expected).queue.len()
}

// ---------------------------------------------------------------------------------------------------------- events

#[test]
fn a_key_event_carries_its_characters_in_every_group() {
    let fake = Fake::new();
    let backend = opened(&fake);
    let event = backend.shared.decode_event(&*fake, &payload(2, A, 0, 77)).expect("a key");
    assert_eq!((event.pressed, event.keycode, event.key_name.as_str()), (true, u32::from(A), "a"));
    assert_eq!((event.character.as_str(), event.characters.clone(), event.group, event.timestamp), ("a", vec!["a".to_owned(), "ф".to_owned()], 0, 77));
    let second_group = backend.shared.decode_event(&*fake, &payload(3, A, 1 << 13 | 1, 5)).expect("a release");
    assert!(!second_group.pressed && second_group.group == 1);
    assert_eq!((second_group.character.as_str(), second_group.characters.clone()), ("Ф", vec!["A".to_owned(), "Ф".to_owned()]));
}

#[test]
fn the_group_is_the_one_in_the_event_state_and_a_missing_one_gives_no_character() {
    let fake = Fake::new();
    let backend = opened(&fake);
    let third = backend.shared.decode_event(&*fake, &payload(2, A, 2 << 13, 1)).expect("a key");
    assert_eq!((third.group, third.character.as_str()), (2, ""), "two characters are known, the group is the third");
}

#[test]
fn return_and_tab_carry_their_control_characters_and_a_wheel_is_only_a_turn() {
    let fake = Fake::new();
    let backend = opened(&fake);
    assert_eq!(backend.shared.decode_event(&*fake, &payload(2, RETURN, 0, 1)).expect("return").character, "\n");
    assert_eq!(backend.shared.decode_event(&*fake, &payload(2, TAB, 0, 1)).expect("tab").character, "\t");
    for button in [4, 5, 6, 7] {
        let turn = backend.shared.decode_event(&*fake, &payload(4, button, 0, 1)).expect("a turn of the wheel");
        assert_eq!((turn.key_name.as_str(), turn.pressed, turn.timestamp), ("Wheel", false, 1), "the wheel moves the view, not the caret");
    }
    let click = backend.shared.decode_event(&*fake, &payload(4, 1, 0, 9)).expect("a click");
    assert_eq!((click.key_name.as_str(), click.group, click.timestamp, click.characters.len()), ("Pointer", -1, 9, 2));
    assert!(backend.shared.decode_event(&*fake, &payload(6, 1, 0, 9)).is_none(), "a motion is not a key");
}

#[test]
fn characters_follow_shift_caps_lock_and_the_group_fallback() {
    let fake = Fake::new();
    let backend = opened(&fake);
    let shared = &backend.shared;
    assert_eq!(shared.character_for_keycode(&*fake, A, 0, 0), "a");
    assert_eq!(shared.character_for_keycode(&*fake, A, 0, SHIFT_MASK.low::<u16>()), "A");
    assert_eq!(shared.character_for_keycode(&*fake, A, 0, LOCK_MASK.low::<u16>()), "A", "caps lock turns a letter over");
    assert_eq!(shared.character_for_keycode(&*fake, A, 0, (LOCK_MASK | SHIFT_MASK).low::<u16>()), "a", "shift and caps lock cancel");
    assert_eq!(shared.character_for_keycode(&*fake, COMMA, 1, 0), "б");
    assert_eq!(shared.character_for_keycode(&*fake, COMMA, 0, LOCK_MASK.low::<u16>()), ",", "caps lock leaves a sign");
    // Space has only group 0: the lookup applies the group fallback.
    assert_eq!(shared.character_for_keycode(&*fake, SPACE, 1, 0), " ");
    // Backspace is no printable character, and an unknown key has none.
    assert_eq!(shared.character_for_keycode(&*fake, BACKSPACE, 0, 0), "");
    assert_eq!(shared.character_for_keycode(&*fake, 99, 0, 0), "");
    assert_eq!(shared.key_name(&*fake, A), "a");
    assert_eq!(shared.key_name(&*fake, BACKSPACE), "BackSpace");
    assert_eq!(shared.key_name(&*fake, 99), "");
}

#[test]
fn expected_keys_are_recognised_once_in_order_and_forgotten_after_the_deadline() {
    let fake = Fake::new();
    let backend = opened(&fake);
    let shared = &backend.shared;
    assert!(!shared.consume_expected(true, A));
    shared.expect(&[&[(true, A), (false, A)]]);
    assert!(!shared.consume_expected(false, A), "the head of the queue is the press");
    assert!(shared.consume_expected(true, A));
    assert!(shared.consume_expected(false, A));
    assert!(!shared.consume_expected(false, A));
    shared.expect(&[&[(true, A)]]);
    lock(&shared.expected).deadline = Instant::now().checked_sub(Duration::from_secs(1)).unwrap();
    assert!(!shared.consume_expected(true, A));
    assert_eq!(expected_len(&backend), 0, "a late key empties the queue");
}

// ------------------------------------------------------------------------------------------------- open and probe

#[test]
fn the_group_count_is_clamped_and_the_filter_is_ignored() {
    let fake = Fake::new();
    let connector = |fake: &Arc<Fake>| -> Connector {
        let api = fake.clone();
        Box::new(move || Ok(api.clone()))
    };
    assert_eq!(X11Backend::with_group_count(1, connector(&fake), None).shared.group_count, 2);
    assert_eq!(X11Backend::with_group_count(9, connector(&fake), None).shared.group_count, 4);
    let backend = fake.backend();
    assert!(!backend.running());
    backend.set_key_filter(Some(Arc::new(|_| KeyDisposition::Consume)));
    backend.hold_input();
    assert_eq!((backend.release_input(), backend.complete_action(true)), (Ok(0), Ok(0)));
}

#[test]
fn a_probe_reports_the_versions_and_closes_a_connection_it_opened() {
    let fake = Fake::new();
    let backend = fake.backend();
    let probe = backend.probe();
    assert!(probe.available && probe.error.is_empty());
    assert_eq!((probe.record_version.as_str(), probe.xtest_version.as_str(), probe.xkb_version.as_str()), ("1.13", "2.2", "1.0"));
    assert_eq!(probe.current_group, 0);
    assert!(backend.shared.api().is_none(), "the temporary connection is closed");
    fake.with(|state| state.record_version_error = true);
    let failed = backend.probe();
    assert!(!failed.available && failed.error == "Расширение XRecord недоступно" && failed.record_version == "—" && failed.current_group == -1);
}

#[test]
fn a_connector_that_fails_makes_an_unavailable_probe_with_its_message() {
    let backend = X11Backend::new(Box::new(|| Err(failed("Переменная DISPLAY не задана"))), None);
    let probe = backend.probe();
    assert!(!probe.available && probe.error == "Переменная DISPLAY не задана" && probe.xkb_version == "—");
}

// --------------------------------------------------------------------------------------------------------- capture

fn chunk(category: u8, data: Vec<u8>) -> RecordChunk {
    RecordChunk { category, client_swapped: false, data }
}

#[test]
fn start_hands_every_key_of_a_reply_to_the_listener_and_stop_ends_the_thread() {
    let fake = Fake::new();
    let backend = fake.backend();
    let mut data = payload(2, A, 0, 1);
    data.extend(payload(3, A, 0, 2));
    fake.with(|state| {
        state.block_record = true;
        state.chunks = vec![
            chunk(XRECORD_START_OF_DATA, vec![]),
            RecordChunk { category: 0, client_swapped: true, data: payload(2, A, 0, 9) },
            chunk(0, data),
            chunk(0, vec![]),
        ];
    });
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    backend.start(Arc::new(move |event| lock(&sink).push((event.pressed, event.timestamp)))).expect("starts");
    assert!(backend.running());
    // The replies after the start-of-data one are handled on the capture thread, after `start` returns.
    let deadline = Instant::now() + Duration::from_secs(2);
    while lock(&seen).len() < 2 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(*lock(&seen), vec![(true, 1), (false, 2)], "the swapped and the empty replies are skipped");
    backend.start(Arc::new(|_| {})).expect("a second start does nothing");
    backend.stop();
    assert!(!backend.running());
    backend.close();
    assert!(backend.shared.api().is_none());
}

#[test]
fn start_needs_the_server_to_confirm_the_capture() {
    let fake = Fake::new();
    let backend = fake.backend();
    // The stream ends without a start-of-data reply.
    let error = backend.start(Arc::new(|_| {})).expect_err("no confirmation");
    assert_eq!(error.to_string(), "XRecord завершился до подтверждения запуска");
    assert!(backend.shared.api().is_none(), "a failed start closes the connection");
    fake.with(|state| state.record_version_error = true);
    assert_eq!(backend.start(Arc::new(|_| {})).expect_err("no record").to_string(), "XRecord не поддерживается X-сервером");
}

#[test]
fn a_stream_that_breaks_while_running_clears_the_running_flag() {
    let fake = Fake::new();
    let backend = fake.backend();
    fake.with(|state| {
        state.chunks = vec![chunk(XRECORD_START_OF_DATA, vec![])];
        state.record_error = true;
    });
    backend.start(Arc::new(|_| {})).expect("started before it broke");
    let deadline = Instant::now() + Duration::from_secs(2);
    while backend.running() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert!(!backend.running());
}

// ------------------------------------------------------------------------------------------------------ groups

#[test]
fn the_group_is_read_and_switched_inside_the_known_range() {
    let fake = Fake::new();
    let backend = fake.backend();
    assert_eq!(backend.current_group(), Ok(-1), "no connection");
    assert_eq!(backend.switch_group(1).expect_err("not running").to_string(), "X11 backend не запущен");
    backend.shared.open().expect("open");
    fake.with(|state| state.group = 1);
    assert_eq!(backend.current_group(), Ok(1));
    assert_eq!(backend.switch_group(2).expect_err("unknown").to_string(), "Неизвестная группа раскладки 2");
    assert!(backend.switch_group(0).is_ok());
    assert_eq!(fake.calls(), vec![Call::Lock(0), Call::Flush]);
    fake.with(|state| state.fail_lock = Some(1));
    assert_eq!(backend.switch_group(1).expect_err("refused").to_string(), "Не удалось переключить XKB-группу на 1");
}

/// A desktop that keeps the group: it notes each request, and when it takes one the server shows the group, unless it never
/// gets round to it.
struct Desktop {
    fake: Arc<Fake>,
    takes: bool,
    shows: bool,
    asked: Mutex<Vec<u8>>,
}

impl DesktopLayouts for Desktop {
    fn activate(&self, group: u8) -> bool {
        lock(&self.asked).push(group);
        if self.takes && self.shows {
            self.fake.with(|state| state.group = group);
        }
        self.takes
    }
}

fn with_desktop(fake: &Arc<Fake>, takes: bool, shows: bool) -> (X11Backend, Arc<Desktop>) {
    let backend = opened(fake);
    let desktop = Arc::new(Desktop { fake: fake.clone(), takes, shows, asked: Mutex::new(Vec::new()) });
    backend.use_desktop_layouts(desktop.clone());
    (backend, desktop)
}

#[test]
fn a_desktop_that_keeps_the_group_is_asked_for_it_and_xkb_is_left_alone() {
    let fake = Fake::new();
    let (backend, desktop) = with_desktop(&fake, true, true);
    assert!(backend.switch_group(1).is_ok());
    assert_eq!(backend.current_group(), Ok(1));
    let strokes = [stroke(true)];
    assert_eq!(backend.inject_correction(Injection { source_group: Some(0), ..injection(&strokes, 1, None) }), Ok(0));
    assert_eq!(*lock(&desktop.asked), vec![1, 1]);
    let mut want = tap(BACKSPACE).to_vec();
    want.extend(shifted_tap(A));
    assert_eq!(typing(&fake), want, "no group is locked past the desktop");
}

#[test]
fn a_sign_that_keeps_its_glyph_is_typed_in_the_target_layout_where_the_desktop_keeps_the_group() {
    let fake = Fake::new();
    let (backend, desktop) = with_desktop(&fake, true, true);
    let boundary = comma(0, 0);
    let strokes = [stroke(true)];
    assert_eq!(backend.inject_correction(Injection { source_group: Some(0), ..injection(&strokes, 1, Some(&boundary)) }), Ok(0));
    // One switch: the comma of the Russian layout is Shift and the key of `/`.
    assert_eq!(*lock(&desktop.asked), vec![1]);
    let mut want = tap(BACKSPACE).to_vec();
    want.extend(tap(BACKSPACE));
    want.extend(shifted_tap(A));
    want.extend(shifted_tap(SLASH));
    assert_eq!(typing(&fake), want);
    // A sign of two characters has no one key: the layout is switched for it, a moment after the keys before it.
    let fake = Fake::new();
    let (backend, desktop) = with_desktop(&fake, true, true);
    let wide = KeyEvent { character: ",,".to_owned(), ..comma(0, 0) };
    let started = Instant::now();
    assert_eq!(backend.inject_correction(Injection { source_group: Some(0), ..injection(&strokes, 1, Some(&wide)) }), Ok(0));
    assert_eq!(*lock(&desktop.asked), vec![1, 0, 1]);
    assert!(started.elapsed() >= Duration::from_millis(DESKTOP_SWITCH_SETTLE_MS), "the keys before a switch are given time");
}

#[test]
fn a_desktop_that_refuses_or_never_shows_the_group_leaves_it_to_xkb() {
    let fake = Fake::new();
    let (backend, desktop) = with_desktop(&fake, false, false);
    assert!(backend.switch_group(1).is_ok());
    assert_eq!((lock(&desktop.asked).clone(), typing(&fake)), (vec![1], vec![Call::Lock(1)]));
    let fake = Fake::new();
    let (backend, desktop) = with_desktop(&fake, true, false);
    assert!(backend.switch_group(1).is_ok());
    assert_eq!((lock(&desktop.asked).clone(), typing(&fake)), (vec![1], vec![Call::Lock(1)]));
}

#[test]
fn cinnamon_is_known_by_its_name_and_its_layouts_give_the_groups() {
    assert!(is_cinnamon_desktop("X-Cinnamon"));
    assert!(is_cinnamon_desktop("ubuntu:cinnamon"));
    assert!(!is_cinnamon_desktop("ubuntu:GNOME"));
    assert!(!is_cinnamon_desktop(""));
    let source = |kind: &str, index: i32| (kind.to_owned(), index);
    // An input method between two layouts has no group of its own.
    let sources = [source("xkb", 0), source("ibus", 1), source("xkb", 2)];
    assert_eq!((0..3).map(|group| cinnamon_source_for_group(&sources, group)).collect::<Vec<_>>(), vec![Some(0), Some(2), None]);
    let four: Vec<(String, i32)> = (0..4).map(|index| source("xkb", index)).collect();
    assert_eq!(cinnamon_source_for_group(&four, 0), None, "more layouts than one keymap holds");
}

// ---------------------------------------------------------------------------------------------------- injection

#[test]
fn an_injection_needs_an_open_backend_and_the_keys_it_types() {
    let fake = Fake::new();
    let backend = fake.backend();
    assert!(backend.inject_correction(injection(&[], 1, None)).expect_err("closed").to_string().contains("не запущен"));
    // A server without BackSpace has nothing to delete with.
    struct NoBackspace(Arc<Fake>);
    impl X11Api for NoBackspace {
        fn xkb_version(&self) -> String { self.0.xkb_version() }
        fn record_version(&self) -> Result<(u16, u16), String> { self.0.record_version() }
        fn xtest_version(&self) -> Result<(u8, u16), String> { self.0.xtest_version() }
        fn record_create_context(&self) -> Result<u32, String> { self.0.record_create_context() }
        fn record_enable(&self, context: u32, sink: &mut dyn FnMut(RecordChunk)) -> Result<(), String> { self.0.record_enable(context, sink) }
        fn record_disable(&self, context: u32) { self.0.record_disable(context) }
        fn record_free(&self, context: u32) { self.0.record_free(context) }
        fn sync(&self) { self.0.sync() }
        fn flush(&self) { self.0.flush() }
        fn keysym(&self, keycode: u8, group: u8, level: u8) -> u32 { self.0.keysym(keycode, group, level) }
        fn lookup_keysym(&self, keycode: u8, state: u16) -> Option<u32> { self.0.lookup_keysym(keycode, state) }
        fn keycode_for_keysym(&self, keysym: u32) -> u8 { if keysym == BACKSPACE_KEYSYM { 0 } else { self.0.keycode_for_keysym(keysym) } }
        fn xkb_state(&self) -> Option<XkbState> { self.0.xkb_state() }
        fn lock_group(&self, group: u8) -> bool { self.0.lock_group(group) }
        fn fake_key(&self, pressed: bool, keycode: u8) -> bool { self.0.fake_key(pressed, keycode) }
        fn grab_control(&self, impervious: bool) { self.0.grab_control(impervious) }
        fn input_focus(&self) -> Option<u32> { self.0.input_focus() }
        fn pointer(&self) -> Option<(i32, i32)> { self.0.pointer() }
        fn parent_window(&self, window: u32) -> Option<u32> { self.0.parent_window(window) }
        fn window_pid(&self, window: u32) -> u32 { self.0.window_pid(window) }
        fn class_hint(&self, window: u32) -> Option<(String, String)> { self.0.class_hint(window) }
        fn set_input_focus(&self, window: u32) { self.0.set_input_focus(window) }
        fn move_raise(&self, window: u32, x: i32, y: i32) { self.0.move_raise(window, x, y) }
    }
    let api: Arc<dyn X11Api> = Arc::new(NoBackspace(Fake::new()));
    let broken = X11Backend::new(Box::new(move || Ok(api.clone())), None);
    broken.shared.open().expect("open");
    assert!(broken.inject_correction(injection(&[], 1, None)).expect_err("no BackSpace").to_string().contains("BackSpace"));
}

#[test]
fn a_correction_deletes_the_word_then_types_it_in_the_target_group_with_its_boundary() {
    let fake = Fake::new();
    let backend = opened(&fake);
    let boundary = comma(0, SHIFT);
    let strokes = [stroke(true)];
    assert_eq!(backend.inject_correction(Injection { source_group: Some(0), ..injection(&strokes, 1, Some(&boundary)) }), Ok(0));
    // The comma key types a letter in the target group, so the boundary is kept as typed: target, source, target.
    let mut want = vec![Call::Lock(1)];
    want.extend(tap(BACKSPACE));
    want.extend(tap(BACKSPACE));
    want.extend(shifted_tap(A));
    want.extend([Call::Lock(0)]);
    want.extend(shifted_tap(COMMA));
    want.push(Call::Lock(1));
    assert_eq!(typing(&fake), want);
    let calls = fake.calls();
    assert_eq!(calls[0], Call::Grab(true));
    assert_eq!(calls[calls.len() - 2..], [Call::Grab(false), Call::Flush]);
    assert!(calls.contains(&Call::Sync));
    assert_eq!(expected_len(&backend), 4 + 4 + 4, "the deletes, the word and the boundary are expected echoes");
}

#[test]
fn a_boundary_that_means_the_same_in_both_layouts_goes_with_the_word() {
    let fake = Fake::new();
    let backend = opened(&fake);
    let space = key(SPACE, "space", " ", 0, 0);
    assert_eq!(backend.inject_correction(injection(&[], 1, Some(&space))), Ok(0));
    let mut want = vec![Call::Lock(1)];
    want.extend(tap(BACKSPACE));
    want.extend(tap(SPACE));
    assert_eq!(typing(&fake), want);
    assert_eq!(expected_len(&backend), 4);
}

#[test]
fn caps_lock_flips_the_shift_of_a_letter_and_only_of_a_letter() {
    let fake = Fake::new();
    let backend = opened(&fake);
    fake.with(|state| state.locked_mods = LOCK_MASK.low::<u8>());
    // The user typed with caps lock on: the stroke is a lower-case letter at the shift level. Typed again with the lock on,
    // the letter needs a shift of its own to come out as it was.
    let mut typed = stroke(false);
    typed.state = LOCK_MASK;
    backend.inject_correction(injection(&[typed], 1, None)).expect("injects");
    let mut want = vec![Call::Lock(1)];
    want.extend(tap(BACKSPACE));
    want.extend(tap(A));
    assert_eq!(typing(&fake), want, "the stroke carried the lock itself: nothing to flip");
}

#[test]
fn late_keys_are_deleted_and_typed_again_outside_the_expected_echo() {
    let fake = Fake::new();
    let backend = opened(&fake);
    let strokes = [stroke(false)];
    let late = [stroke(true)];
    assert_eq!(backend.inject_correction(Injection { late: &late, ..injection(&strokes, 1, None) }), Ok(0));
    let mut want = vec![Call::Lock(1)];
    want.extend(tap(BACKSPACE));
    want.extend(tap(BACKSPACE));
    want.extend(tap(A));
    want.extend(shifted_tap(A));
    assert_eq!(typing(&fake), want);
    assert_eq!(expected_len(&backend), 3 * 2, "the late taps are not expected: the engine must see them as typed");
    fake.with(|state| state.fail_fake = Some(state.fakes_seen + 6));
    assert!(backend.inject_correction(Injection { late: &late, ..injection(&strokes, 1, None) }).is_err());
    assert_eq!(expected_len(&backend), 0);
}

#[test]
fn a_refused_lock_or_key_clears_the_expected_keys_and_lets_go_of_the_grab() {
    for lock_fails in [true, false] {
        let fake = Fake::new();
        let backend = opened(&fake);
        fake.with(|state| if lock_fails { state.fail_lock = Some(0) } else { state.fail_fake = Some(1) });
        let strokes = [stroke(false)];
        assert!(backend.inject_correction(injection(&strokes, 1, None)).is_err());
        assert_eq!(expected_len(&backend), 0);
        let calls = fake.calls();
        assert_eq!(calls[calls.len() - 2..], [Call::Grab(false), Call::Flush]);
    }
}

#[test]
fn a_refused_key_is_named_by_its_keycode() {
    let fake = Fake::new();
    let backend = opened(&fake);
    fake.with(|state| state.fail_fake = Some(2));
    let space = key(SPACE, "space", " ", 0, 0);
    assert_eq!(backend.inject_correction(injection(&[], 1, Some(&space))).expect_err("refused").to_string(), format!("XTest отклонил keycode {SPACE}"));
}

#[test]
fn a_kept_tail_is_typed_back_in_the_source_group_where_the_layout_stays() {
    let fake = Fake::new();
    let backend = opened(&fake);
    let space = key(SPACE, "space", " ", 1, 0);
    let mut word = stroke(false);
    word.group = 1;
    let strokes = [word.clone(), space.clone(), word.clone()];
    let late = [stroke(false)];
    let outcome = backend.inject_correction(Injection { source_group: Some(1), late: &late, kept_tail: 1, ..injection(&strokes, 0, Some(&space)) });
    assert_eq!(outcome, Ok(0));
    let mut want = vec![Call::Lock(0)];
    for _ in 0..=(3 + 1) {
        want.extend(tap(BACKSPACE));
    }
    want.extend(tap(A));
    want.extend(tap(SPACE));
    want.push(Call::Lock(1));
    want.extend(tap(A));
    want.extend(tap(SPACE));
    want.extend(tap(A));
    assert_eq!(typing(&fake), want, "converted keys in the target group; the kept word, the boundary and the late key in the source");
    assert_eq!(expected_len(&backend), (5 + 2 + 1 + 1) * 2);
    for refused in [
        Injection { kept_tail: strokes.len(), source_group: Some(1), ..injection(&strokes, 0, Some(&space)) },
        Injection { kept_tail: 1, source_group: None, ..injection(&strokes, 0, Some(&space)) },
    ] {
        let error = backend.inject_correction(refused).expect_err("refused");
        assert!(error.to_string().contains("сохраняемого слова"));
    }
}

#[test]
fn a_kept_tail_whose_group_or_key_is_refused_fails_and_clears_the_expected_keys() {
    let space = key(SPACE, "space", " ", 1, 0);
    let mut word = stroke(false);
    word.group = 1;
    let strokes = [word.clone(), space.clone(), word];
    for lock_fails in [true, false] {
        let fake = Fake::new();
        let backend = opened(&fake);
        if lock_fails {
            fake.with(|state| state.fail_lock = Some(1));
        } else {
            // The five deletes and the two converted keys go through; the kept one fails.
            fake.with(|state| state.fail_fake = Some((5 + 2) * 2));
        }
        let outcome = backend.inject_correction(Injection { source_group: Some(1), kept_tail: 1, ..injection(&strokes, 0, Some(&space)) });
        assert!(outcome.is_err());
        assert_eq!(expected_len(&backend), 0);
    }
}

#[test]
fn a_boundary_of_two_layouts_is_refused_before_any_key_is_sent() {
    let fake = Fake::new();
    let backend = opened(&fake);
    let first = comma(0, 0);
    let second = comma(1, 0);
    let trailing = [first];
    let error = backend.inject_correction(Injection { trailing: &trailing, ..injection(&[], 1, Some(&second)) }).expect_err("refused");
    assert!(error.to_string().contains("раскладка пунктуации"));
    assert!(fake.calls().is_empty());
}

#[test]
fn an_erased_count_replaces_the_number_of_strokes_in_the_deletes() {
    let fake = Fake::new();
    let backend = opened(&fake);
    let strokes = [stroke(false)];
    backend.inject_correction(Injection { erased: Some(3), ..injection(&strokes, 1, None) }).expect("injects");
    let deletes = typing(&fake).iter().filter(|call| **call == Call::Fake(true, BACKSPACE)).count();
    assert_eq!(deletes, 3);
}

#[test]
fn the_preserved_boundary_errors_name_the_group_that_failed() {
    let boundary = comma(0, 0);
    let strokes = [stroke(false)];
    for (failing_lock, message) in [(1usize, "Не удалось временно включить XKB-группу 0"), (2, "Не удалось восстановить XKB-группу 1")] {
        let fake = Fake::new();
        let backend = opened(&fake);
        fake.with(|state| state.fail_lock = Some(failing_lock));
        let error = backend.inject_correction(Injection { source_group: Some(0), ..injection(&strokes, 1, Some(&boundary)) }).expect_err("refused");
        assert_eq!(error.to_string(), message);
        assert_eq!(expected_len(&backend), 0);
    }
}

// --------------------------------------------------------------------------------------------- windows and anchor

#[test]
fn the_focused_window_is_own_by_the_pid_of_it_or_of_a_parent_and_the_verdict_is_cached() {
    let fake = Fake::new();
    let backend = opened(&fake);
    assert!(backend.focused_window().is_none(), "no focus");
    fake.with(|state| state.focus = Some(1));
    assert!(backend.focused_window().is_none(), "PointerRoot is nobody");
    fake.with(|state| {
        state.focus = Some(300);
        state.parents.insert(300, 200);
        state.parents.insert(200, 100);
        state.pids.insert(100, std::process::id());
    });
    let focus = backend.focused_window().expect("a window");
    assert_eq!((focus.window, focus.own, focus.isolated_layout), (300, true, false));
    let queries = fake.with(|state| state.pid_queries);
    assert!(backend.focused_window().expect("again").own);
    assert_eq!(fake.with(|state| state.pid_queries), queries, "the verdict is cached");
    fake.with(|state| {
        state.focus = Some(400);
        state.pids.insert(400, std::process::id() + 1);
    });
    assert!(!backend.focused_window().expect("foreign").own);
    assert_eq!(backend.window_process_id(300), std::process::id());
    assert_eq!(backend.window_process_id(-5), 0);
}

#[test]
fn the_ancestor_walk_is_bounded_and_stops_at_a_loop() {
    let fake = Fake::new();
    let backend = opened(&fake);
    fake.with(|state| {
        for window in 1..40u32 {
            state.parents.insert(window, window + 1);
        }
        state.pids.insert(30, 5);
    });
    assert_eq!(backend.window_process_id(1), 0, "more than sixteen levels up is not searched");
    assert_eq!(backend.window_process_id(20), 5);
    fake.with(|state| {
        state.parents.insert(50, 50);
    });
    assert_eq!(backend.window_process_id(50), 0);
}

#[test]
fn the_active_application_is_the_class_of_the_window_or_of_its_nearest_parent() {
    let fake = Fake::new();
    let backend = fake.backend();
    assert_eq!(backend.active_application(), "");
    backend.shared.open().expect("open");
    assert_eq!(backend.active_application(), "", "no focus");
    fake.with(|state| {
        state.focus = Some(30);
        state.parents.insert(30, 20);
        state.parents.insert(20, 10);
        state.classes.insert(10, ("navigator".to_owned(), "Firefox".to_owned()));
    });
    assert_eq!(backend.active_application(), "Firefox");
    fake.with(|state| state.classes.insert(10, ("navigator".to_owned(), String::new())));
    assert_eq!(backend.active_application(), "navigator", "the resource name when the class is empty");
    fake.with(|state| state.classes.clear());
    assert_eq!(backend.active_application(), "");
}

#[test]
fn the_anchor_is_the_pointer_and_the_focused_window() {
    let fake = Fake::new();
    let backend = opened(&fake);
    fake.with(|state| state.focus = Some(0));
    assert_eq!(backend.input_anchor(), Some(ScreenAnchor { x: 10, y: 20, window: None }));
    fake.with(|state| state.focus = Some(44));
    assert_eq!(backend.input_anchor(), Some(ScreenAnchor { x: 10, y: 20, window: Some(44) }));
    assert!(backend.restore_window(Some(44)) && !backend.restore_window(None));
    assert!(backend.position_window(44, 1, 2) && !backend.position_window(0, 1, 2));
}

// ---------------------------------------------------------------------------------------------------- selection

struct Source {
    written: Option<bool>,
}

impl SelectionSource for Source {
    fn read_selection(&mut self, _application: &str, _window: i64) -> Result<SelectedText, SelectionError> {
        Ok(SelectedText { text: "ghbdtn".to_owned(), source: "atspi".to_owned() })
    }
    fn write_selection(&mut self, _selected: &SelectedText, _replacement: &str) -> Result<Option<bool>, SelectionError> {
        Ok(self.written)
    }
}

fn selection_backend(fake: &Arc<Fake>, written: Option<bool>) -> X11Backend {
    let api = fake.clone();
    let factory: SelectionFactory = Box::new(move |_| Box::new(Source { written }));
    let backend = X11Backend::new(Box::new(move || Ok(api.clone())), Some(factory));
    backend.shared.open().expect("open");
    backend
}

#[test]
fn a_selection_written_by_the_field_leaves_the_group_where_asked_and_types_nothing() {
    let fake = Fake::new();
    let backend = selection_backend(&fake, Some(true));
    let selected = backend.read_selection("app", 5).expect("selected");
    assert_eq!(backend.replace_selection(&selected, "привет", Some(1)), Ok(true));
    assert_eq!(typing(&fake), vec![Call::Lock(1)]);
}

#[test]
fn a_selection_is_typed_by_runs_and_selected_again() {
    let fake = Fake::new();
    let backend = selection_backend(&fake, None);
    let selected = backend.read_selection("app", 5).expect("selected");
    // Group 0 types `a`, group 1 types `ф`: two runs, then the group is set and the text selected back with Shift+Left.
    assert_eq!(backend.replace_selection(&selected, "aф", Some(1)), Ok(true));
    let mut want = vec![Call::Lock(0)];
    want.extend(tap(A));
    want.push(Call::Lock(1));
    want.extend(tap(A));
    want.push(Call::Lock(1));
    want.push(Call::Fake(true, SHIFT_L));
    for _ in 0..2 {
        want.extend(tap(LEFT));
    }
    want.push(Call::Fake(false, SHIFT_L));
    assert_eq!(typing(&fake), want);
    assert_eq!(expected_len(&backend), 2 + 2 + 2 + 4 + 2 - 2, "the typed taps and the reselection are expected echoes");
}

#[test]
fn a_character_no_key_types_leaves_the_selection_alone() {
    let fake = Fake::new();
    let backend = selection_backend(&fake, None);
    let selected = backend.read_selection("app", 5).expect("selected");
    let refused = backend.replace_selection(&selected, "a—", None);
    assert!(matches!(refused, Err(SelectionError::Refused(_))), "{refused:?}");
    assert!(typing(&fake).is_empty());
}

#[test]
fn without_a_reader_the_selection_is_unsupported() {
    let fake = Fake::new();
    let backend = opened(&fake);
    assert!(matches!(backend.read_selection("app", 5), Err(SelectionError::Refused(_))));
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

const WALK_KEYS: [u8; 9] = [A, COMMA, SPACE, BACKSPACE, SHIFT_L, LEFT, RETURN, TAB, 200];

fn walk_stroke(dice: &mut Dice) -> KeyEvent {
    let code = *dice.of(&WALK_KEYS);
    key(code, dice.of(&["a", "comma", "space", "Return", "ISO_Left_Tab"]), dice.of(&["a", "A", ",", " ", ""]), *dice.of(&[0, 0, 1, 1, -1, 5]), *dice.of(&[0, 0, SHIFT, LOCK_MASK]))
}

#[test]
fn a_long_walk_through_the_backend_stops_nowhere() {
    for seed in 1..=300_u64 {
        let mut dice = Dice(0x9e37_79b9_7f4a_7c15 ^ seed.wrapping_mul(0xbf58_476d_1ce4_e5b9));
        let fake = Fake::new();
        let backend = opened(&fake);
        let api = backend.shared.api().expect("open");
        for _ in 0..60 {
            let code = *dice.of(&WALK_KEYS);
            match dice.below(24) {
                0..=7 => {
                    let state = *dice.of(&[0_u16, 0, 1, 2, 0x2000, 0x2001, 0x4000]);
                    backend.shared.handle_record_data(&*api, RecordChunk { category: 0, client_swapped: false, data: payload(2, code, state, 1) });
                    backend.shared.handle_record_data(&*api, RecordChunk { category: 0, client_swapped: false, data: payload(3, code, state, 2) });
                }
                8 => backend.shared.handle_record_data(&*api, RecordChunk { category: 0, client_swapped: false, data: payload(*dice.of(&[4_u8, 5, 6, 33]), code, 0, 3) }),
                9 => backend.shared.handle_record_data(&*api, RecordChunk { category: 0, client_swapped: false, data: vec![2, 3] }),
                10..=12 => {
                    let strokes: Vec<KeyEvent> = (0..dice.below(4)).map(|_| walk_stroke(&mut dice)).collect();
                    let trailing: Vec<KeyEvent> = (0..dice.below(2)).map(|_| walk_stroke(&mut dice)).collect();
                    let late: Vec<KeyEvent> = (0..dice.below(2)).map(|_| walk_stroke(&mut dice)).collect();
                    let boundary = dice.one_in(2).then(|| walk_stroke(&mut dice));
                    let _ = backend.inject_correction(Injection {
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
                13 => fake.with(|state| {
                    state.fail_lock = *dice.of(&[None, None, Some(0), Some(1)]);
                    state.fail_fake = *dice.of(&[None, None, Some(0), Some(1)]);
                    state.locks_seen = 0;
                    state.fakes_seen = 0;
                }),
                14 => fake.with(|state| state.focus = *dice.of(&[None, Some(0), Some(1), Some(5)])),
                15 => fake.with(|state| {
                    state.parents.insert(5, *dice.of(&[0, 5, 6]));
                    state.parents.insert(6, 0);
                    state.pids.insert(6, *dice.of(&[0, 42, std::process::id()]));
                    state.classes.insert(6, ("instance".to_owned(), "Class".to_owned()));
                }),
                16 => fake.with(|state| {
                    state.group = *dice.of(&[0, 1, 2]);
                    state.locked_mods = *dice.of(&[0, 2]);
                }),
                17 => {
                    let _ = backend.switch_group(*dice.of(&[0, 1, -1, 9]));
                }
                18 => {
                    let _ = backend.position_window(*dice.of(&[0, 5, 6]), 1, 2);
                    let _ = backend.window_process_id(*dice.of(&[0, 5, 6]));
                    let _ = (backend.process_for_window())(5);
                }
                19 => {
                    let _ = backend.character_for_keycode(code, *dice.of(&[0, 1, 5]), *dice.of(&[0, 1, 2]));
                }
                20 => backend.hold_input(),
                21 => {
                    let _ = backend.release_input();
                    let _ = backend.complete_action(dice.one_in(2));
                }
                _ => {
                    let _ = (backend.current_group(), backend.active_application(), backend.focused_window(), backend.probe(), backend.input_anchor());
                }
            }
        }
        backend.close_backend();
        assert!(backend.shared.api().is_none());
    }
}

#[test]
fn a_connection_is_opened_once_and_a_start_uses_it() {
    let fake = Fake::new();
    let backend = fake.backend();
    backend.open().expect("open");
    backend.open().expect("already open");
    assert!(backend.supports_selection());
    fake.with(|state| state.block_record = true);
    *lock(&backend.shared.context) = 5;
    let starts = thread::scope(|scope| {
        let starting = scope.spawn(|| backend.start(Arc::new(|_| {})));
        thread::sleep(Duration::from_millis(50));
        backend.shared.capture_ready.set();
        backend.shared.capture_start_finished.set();
        starting.join().unwrap()
    });
    assert!(starts.is_ok());
    backend.close();
}

#[test]
fn a_context_the_server_refuses_or_a_capture_it_never_confirms_stops_the_start() {
    let fake = Fake::new();
    fake.with(|state| state.no_context = true);
    let backend = fake.backend();
    assert_eq!(backend.start(Arc::new(|_| {})).expect_err("no context").to_string(), "Не удалось создать контекст XRecord");
    let fake = Fake::new();
    fake.with(|state| state.block_record = true);
    let backend = fake.backend();
    assert!(backend.start(Arc::new(|_| {})).expect_err("no answer").to_string().contains("не подтвердил"));
    assert!(backend.shared.api().is_none());
    assert!(fake.backend().shared.spawn_failed(&io::Error::other("no threads")).to_string().contains("no threads"));
}

#[test]
fn keys_of_rare_keysyms_read_as_what_they_type() {
    let fake = Fake::new();
    let backend = opened(&fake);
    assert_eq!(backend.character_for_keycode(EMOJI, 0, 0), "😀");
    assert_eq!(backend.character_for_keycode(SURROGATE, 0, 0), "");
    assert_eq!(backend.character_for_keycode(LINE_FEED, 0, 0), "\u{a}");
    assert_eq!(backend.character_for_keycode(BACKSPACE, 0, 0), "");
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    fake.with(|state| {
        state.chunks = vec![chunk(4, Vec::new()), chunk(0, payload(2, BACK_TAB, 0, 1))];
    });
    backend.start(Arc::new(move |event| lock(&sink).push(event))).expect("started");
    let deadline = Instant::now() + Duration::from_secs(5);
    while lock(&events).is_empty() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
    backend.stop();
    assert_eq!(lock(&events).first().map(|event: &KeyEvent| event.character.clone()), Some("\t".to_owned()));
}

#[test]
fn the_list_of_own_windows_is_kept_short() {
    let fake = Fake::new();
    let backend = opened(&fake);
    for window in 0..u32::try_from(MAX_TRACKED_OWN_WINDOWS).unwrap() + 2 {
        fake.with(|state| state.focus = Some(window + 1000));
        let _ = backend.focused_window();
    }
    assert!(lock(&backend.shared.own_windows).len() <= MAX_TRACKED_OWN_WINDOWS);
}

#[test]
fn a_stop_waits_for_the_capture_only_so_long_and_never_for_itself() {
    let fake = Fake::new();
    // Marked running with no context, and with a context and no connection.
    let backend = fake.backend();
    backend.shared.running.store(true, Ordering::SeqCst);
    backend.stop();
    *lock(&backend.shared.context) = 5;
    backend.stop();
    backend.shared.running.store(false, Ordering::SeqCst);
    // A capture that does not end in time is left behind.
    let slow = thread::spawn(|| thread::sleep(Duration::from_secs(3)));
    *lock(&backend.shared.thread) = Some((slow, Arc::new(Event::default())));
    let began = Instant::now();
    backend.stop();
    assert!(began.elapsed() < Duration::from_secs(3));
    // A stop from the capture's own thread.
    let backend = Arc::new(fake.backend());
    let go = Arc::new(Event::default());
    let (worker, signal) = (backend.clone(), go.clone());
    let handle = thread::spawn(move || {
        assert!(signal.wait(Duration::from_secs(5)));
        worker.stop();
    });
    *lock(&backend.shared.thread) = Some((handle, Arc::new(Event::default())));
    go.set();
    let deadline = Instant::now() + Duration::from_secs(5);
    while lock(&backend.shared.thread).is_some() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
    assert!(lock(&backend.shared.thread).is_none());
}

#[test]
fn a_correction_needs_the_state_of_the_keyboard_and_a_shift_key() {
    let fake = Fake::new();
    let backend = opened(&fake);
    let strokes = [stroke(false)];
    fake.with(|state| state.no_keyboard_state = true);
    assert!(backend.inject_correction(injection(&strokes, 1, None)).expect_err("no state").to_string().contains("Caps Lock"));
    fake.with(|state| {
        state.no_keyboard_state = false;
        state.missing_keysyms = vec![SHIFT_L_KEYSYM];
    });
    assert!(backend.inject_correction(injection(&strokes, 1, None)).expect_err("no shift").to_string().contains("BackSpace/Shift"));
}

#[test]
fn a_selection_is_read_and_written_as_far_as_the_server_lets_it() {
    let selected = SelectedText { text: "ghbdtn".to_owned(), source: "atspi".to_owned() };
    // Nothing open yet.
    let fake = Fake::new();
    let backend = fake.backend();
    assert!(backend.replace_selection(&selected, "a", None).is_err());
    let backend = selection_backend(&fake, None);
    // Read twice: one source.
    assert!(backend.read_selection("app", 5).is_ok());
    assert!(backend.read_selection("app", 5).is_ok());
    // A field that writes it itself and no layout asked for.
    let written = selection_backend(&fake, Some(false));
    assert!(written.read_selection("app", 5).is_ok());
    assert_eq!(written.replace_selection(&selected, "привет", None), Ok(false));
    // A field nobody read yet takes the keys.
    let unread = opened(&fake);
    assert_eq!(unread.replace_selection(&selected, "a", None), Ok(true));
    // What the keys need.
    fake.with(|state| state.no_keyboard_state = true);
    assert!(backend.replace_selection(&selected, "a", None).is_err());
    fake.with(|state| {
        state.no_keyboard_state = false;
        state.missing_keysyms = vec![SHIFT_L_KEYSYM];
    });
    assert!(backend.replace_selection(&selected, "a", None).is_err());
    fake.with(|state| state.missing_keysyms = vec![LEFT_KEYSYM]);
    assert!(backend.replace_selection(&selected, "a", None).is_err());
    fake.with(|state| state.missing_keysyms.clear());
    // A capital with Shift, a capital and a sign under Caps Lock, a character the caret cannot be counted over.
    assert_eq!(backend.replace_selection(&selected, "A", None), Ok(true));
    assert!(fake.calls().contains(&Call::Fake(true, SHIFT_L)));
    fake.with(|state| state.locked_mods = u8::try_from(LOCK_MASK).unwrap());
    assert_eq!(backend.replace_selection(&selected, "A,", None), Ok(true));
    fake.with(|state| state.locked_mods = 0);
    assert_eq!(backend.replace_selection(&selected, "😀", None), Ok(false));
    // A key the server refuses is forgotten with what was expected.
    fake.with(|state| {
        state.fakes_seen = 0;
        state.fail_fake = Some(0);
    });
    assert!(backend.replace_selection(&selected, "a", None).is_err());
    assert_eq!(expected_len(&backend), 0);
}
