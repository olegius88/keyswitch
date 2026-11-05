//! `tests/test_macos_backend.py`: the decision half of the macOS backend over a fake of the system.

use ks_cast::To as _;
use std::sync::atomic::AtomicUsize;
use std::sync::{Condvar};

use super::*;
use crate::macos_context::SelectionAfter;

const ABC: &str = "com.apple.keylayout.ABC";
const RUSSIAN: &str = "com.apple.keylayout.Russian";
const KEY_Z: u32 = 0x06;
const KEY_Q: u32 = 0x0C;

#[derive(Default)]
struct Board {
    changes: Mutex<i64>,
    contents: Mutex<Option<PasteboardContents>>,
    text: Mutex<String>,
    complete: bool,
}

impl MacPasteboardAPI for Board {
    fn pasteboard_change_count(&self) -> i64 {
        *self.changes.lock().unwrap()
    }
    fn read_pasteboard(&self, _limit: usize) -> PasteboardContents {
        PasteboardContents { items: vec![vec![("public.utf8-plain-text".into(), b"mine".to_vec())]], complete: self.complete }
    }
    fn write_pasteboard(&self, contents: &PasteboardContents) -> bool {
        *self.contents.lock().unwrap() = Some(contents.clone());
        true
    }
    fn put_text(&self, text: &str) -> Option<i64> {
        *self.text.lock().unwrap() = text.to_owned();
        *self.changes.lock().unwrap() += 1;
        Some(*self.changes.lock().unwrap())
    }
}

struct FakeMac {
    sources: Vec<String>,
    current: Mutex<String>,
    /// A switch the system accepts but never confirms.
    switch_ignored: AtomicBool,
    posted: Mutex<Vec<NativeInput>>,
    /// How many events of a post the system takes; None for all.
    take: Mutex<Option<usize>>,
    caps: AtomicBool,
    trusted: AtomicBool,
    window: AtomicU64,
    tap: Mutex<Option<TapListener>>,
    tap_stopped: (Mutex<bool>, Condvar),
    tap_error: Mutex<Option<MacBackendError>>,
    enabled: AtomicUsize,
    board: Board,
}

impl FakeMac {
    fn new() -> Arc<FakeMac> {
        Arc::new(FakeMac {
            sources: vec![ABC.into(), RUSSIAN.into()],
            current: Mutex::new(ABC.into()),
            switch_ignored: AtomicBool::new(false),
            posted: Mutex::new(Vec::new()),
            take: Mutex::new(None),
            caps: AtomicBool::new(false),
            trusted: AtomicBool::new(true),
            window: AtomicU64::new(10),
            tap: Mutex::new(None),
            tap_stopped: (Mutex::new(false), Condvar::new()),
            tap_error: Mutex::new(None),
            enabled: AtomicUsize::new(0),
            board: Board { complete: true, ..Board::default() },
        })
    }

    fn posted(&self) -> Vec<NativeInput> {
        self.posted.lock().unwrap().clone()
    }
}

impl MacAPI for FakeMac {
    fn input_sources(&self) -> Result<Vec<String>, MacBackendError> {
        Ok(self.sources.clone())
    }
    fn current_input_source(&self) -> String {
        self.current.lock().unwrap().clone()
    }
    fn select_input_source(&self, identifier: &str) -> bool {
        if !self.switch_ignored.load(Ordering::SeqCst) {
            *self.current.lock().unwrap() = identifier.to_owned();
        }
        true
    }
    fn translate_key(&self, keycode: u32, state: u32, source: &str) -> String {
        let russian = source == RUSSIAN;
        let letter = match (keycode, russian) {
            (KEY_Z, false) => "z",
            (KEY_Z, true) => "я",
            (KEY_Q, false) => "q",
            (KEY_Q, true) => "й",
            (0x31, _) => " ",
            _ => return String::new(),
        };
        if state & (SHIFT_MASK | LOCK_MASK) != 0 { letter.to_uppercase() } else { letter.to_owned() }
    }
    fn post_inputs(&self, inputs: &[NativeInput]) -> usize {
        let count = self.take.lock().unwrap().map_or(inputs.len(), |limit| limit.min(inputs.len()));
        self.posted.lock().unwrap().extend_from_slice(&inputs[..count]);
        count
    }
    fn active_application(&self) -> String {
        "TextEdit".to_owned()
    }
    fn focused_window(&self) -> i64 {
        self.window.load(Ordering::SeqCst).bits::<i64>()
    }
    fn window_process_id(&self, window: i64) -> i32 {
        if window == 99 { 4242 } else { 1 }
    }
    fn current_process_id(&self) -> i32 {
        4242
    }
    fn input_anchor(&self) -> Option<ScreenAnchor> {
        None
    }
    fn caps_lock_enabled(&self) -> bool {
        self.caps.load(Ordering::SeqCst)
    }
    fn accessibility_trusted(&self, _prompt: bool) -> bool {
        self.trusted.load(Ordering::SeqCst)
    }
    fn open_accessibility_settings(&self) -> bool {
        true
    }
    fn layout_follows_window(&self) -> bool {
        false
    }
    fn run_event_tap(&self, listener: TapListener, ready: TapReady) -> Result<(), MacBackendError> {
        if let Some(error) = self.tap_error.lock().unwrap().clone() {
            return Err(error);
        }
        *self.tap.lock().unwrap() = Some(listener);
        ready();
        let (flag, signal) = &self.tap_stopped;
        let mut stopped = flag.lock().unwrap();
        while !*stopped {
            stopped = signal.wait(stopped).unwrap();
        }
        Ok(())
    }
    fn enable_event_tap(&self) {
        self.enabled.fetch_add(1, Ordering::SeqCst);
    }
    fn stop_event_tap(&self) {
        *self.tap_stopped.0.lock().unwrap() = true;
        self.tap_stopped.1.notify_all();
    }
    fn pasteboard(&self) -> Option<&dyn MacPasteboardAPI> {
        Some(&self.board)
    }
}

/// A clock that moves only when slept on.
#[derive(Default)]
struct StepClock(Mutex<f64>);

impl Clock for StepClock {
    fn monotonic(&self) -> f64 {
        *self.0.lock().unwrap()
    }
    fn sleep(&self, seconds: f64) {
        *self.0.lock().unwrap() += seconds;
    }
}

fn backend(api: &Arc<FakeMac>) -> MacBackend {
    let parts = BackendParts { clock: Arc::new(StepClock::default()), ..BackendParts::default() };
    MacBackend::with_parts(api.clone(), parts)
}

fn collector() -> (KeyListener, Arc<Mutex<Vec<KeyEvent>>>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    (Arc::new(move |event| sink.lock().unwrap().push(event)), events)
}

fn native(pressed: bool, keycode: u32) -> NativeKeyEvent {
    NativeKeyEvent { pressed, keycode, timestamp: 5, ..NativeKeyEvent::default() }
}

/// A stroke of the engine: the key, its characters in both layouts, and the layout it was typed in.
fn stroke(keycode: u32, group: i32, state: u32) -> KeyEvent {
    let (english, russian) = match keycode {
        KEY_Z => ("z", "я"),
        KEY_Q => ("q", "й"),
        _ => (" ", " "),
    };
    let cased = |text: &str| if state & SHIFT_MASK != 0 { text.to_uppercase() } else { text.to_owned() };
    let characters = vec![cased(english), cased(russian)];
    KeyEvent {
        pressed: true,
        keycode,
        key_name: english.to_owned(),
        character: characters[group.to::<usize>()].clone(),
        characters,
        group,
        state,
        ..KeyEvent::default()
    }
}

fn injection<'a>(strokes: &'a [KeyEvent], target_group: i32) -> Injection<'a> {
    Injection { strokes, target_group, boundary: None, source_group: None, late: &[], trailing: &[], kept_tail: 0, erased: None }
}

#[test]
fn a_modifier_is_down_by_its_own_side_bit_where_the_event_has_one() {
    let both_shifts = EVENT_FLAG_DEVICE_LEFT_SHIFT_BIT | EVENT_FLAG_DEVICE_RIGHT_SHIFT_BIT;
    assert!(modifier_pressed(MAC_VK_SHIFT, both_shifts));
    // The left Shift went up while the right one is held: the shared bit is still set, the left bit is not.
    assert!(!modifier_pressed(MAC_VK_SHIFT, crate::constants::EVENT_FLAG_SHIFT | EVENT_FLAG_DEVICE_RIGHT_SHIFT_BIT));
    assert!(modifier_pressed(MAC_VK_RIGHT_SHIFT, crate::constants::EVENT_FLAG_SHIFT | EVENT_FLAG_DEVICE_RIGHT_SHIFT_BIT));
    // No side bits at all: the shared bit decides.
    assert!(modifier_pressed(MAC_VK_COMMAND, crate::constants::EVENT_FLAG_COMMAND));
    assert!(!modifier_pressed(MAC_VK_COMMAND, 0));
    // Caps Lock is never a held key, whatever its flag says.
    assert!(!modifier_pressed(MAC_VK_CAPS_LOCK, crate::constants::EVENT_FLAG_ALPHA_SHIFT));
}

const EVENT_FLAG_DEVICE_LEFT_SHIFT_BIT: u64 = crate::constants::EVENT_FLAG_DEVICE_LEFT_SHIFT;
const EVENT_FLAG_DEVICE_RIGHT_SHIFT_BIT: u64 = crate::constants::EVENT_FLAG_DEVICE_RIGHT_SHIFT;

#[test]
fn the_layouts_are_chosen_by_what_a_key_types() {
    let translate = |_: u32, _: u32, source: &str| match source {
        "us" | "british" => "q".to_owned(),
        "ru-win" => "й".to_owned(),
        _ => String::new(),
    };
    let sources: Vec<String> = ["emoji", "british", "us", "ru-win", "british"].iter().map(|s| (*s).to_owned()).collect();
    assert_eq!(select_source_pair(&sources, &translate), Ok(("british".to_owned(), "ru-win".to_owned())));
    let no_russian: Vec<String> = vec!["us".to_owned()];
    assert!(select_source_pair(&no_russian, &translate).is_err());
}

#[test]
fn keys_are_named_by_position_then_by_their_english_letter() {
    assert_eq!(key_name(MAC_VK_RETURN, ""), "Return");
    assert_eq!(key_name(MAC_VK_RIGHT_OPTION, "x"), "Alt_R");
    assert_eq!(key_name(KEY_Z, "Z"), "z");
    assert_eq!(key_name(0x2B, ","), "VK_2B");
    assert_eq!(key_name(0x7F, ""), "VK_7F");
}

#[test]
fn the_probe_reports_the_permission_before_anything_else() {
    let api = FakeMac::new();
    api.trusted.store(false, Ordering::SeqCst);
    let probe = backend(&api).probe();
    assert!(!probe.available);
    assert_eq!((probe.error.as_str(), probe.current_group), (PERMISSION_MISSING_MESSAGE, -1));
    api.trusted.store(true, Ordering::SeqCst);
    *api.current.lock().unwrap() = RUSSIAN.to_owned();
    let probe = backend(&api).probe();
    assert!(probe.available);
    assert_eq!((probe.xkb_version.as_str(), probe.current_group, probe.record_version.as_str()), (format!("{ABC},{RUSSIAN}").as_str(), 1, "CGEventTap"));
}

#[test]
fn a_key_reaches_the_engine_with_both_layouts_and_the_modifiers_held() {
    let api = FakeMac::new();
    let backend = backend(&api);
    let (listener, events) = collector();
    backend.set_listener(Some(listener));
    assert!(!backend.handle_native(native(true, MAC_VK_SHIFT)));
    assert!(!backend.handle_native(native(true, KEY_Z)));
    let events = events.lock().unwrap();
    let key = &events[1];
    assert_eq!((key.key_name.as_str(), key.character.as_str(), key.group), ("z", "Z", 0));
    assert_eq!(key.characters, vec!["Z".to_owned(), "Я".to_owned()]);
    assert_eq!(key.state, SHIFT_MASK);
    assert!(!key.synthetic);
}

#[test]
fn a_name_does_not_follow_the_modifiers() {
    let api = FakeMac::new();
    let backend = backend(&api);
    let (listener, events) = collector();
    backend.set_listener(Some(listener));
    backend.handle_native(native(true, MAC_VK_CONTROL));
    backend.handle_native(native(true, KEY_Z));
    assert_eq!(events.lock().unwrap()[1].key_name, "z");
}

#[test]
fn the_state_carries_caps_lock_and_a_released_modifier_is_forgotten() {
    let api = FakeMac::new();
    let backend = backend(&api);
    api.caps.store(true, Ordering::SeqCst);
    backend.handle_native(native(true, MAC_VK_OPTION));
    assert_eq!(backend.inner.normalized_state(), LOCK_MASK | ALT_MASK);
    backend.handle_native(native(false, MAC_VK_OPTION));
    assert_eq!(backend.inner.normalized_state(), LOCK_MASK);
}

#[test]
fn a_pointer_press_is_an_event_of_its_own() {
    let api = FakeMac::new();
    let backend = backend(&api);
    let (listener, events) = collector();
    backend.set_listener(Some(listener));
    assert!(!backend.handle_native(NativeKeyEvent { pointer: true, pressed: true, timestamp: 9, ..NativeKeyEvent::default() }));
    let events = events.lock().unwrap();
    assert_eq!((events[0].key_name.as_str(), events[0].group, events[0].timestamp), ("Pointer", POINTER_EVENT_GROUP, 9));
    assert_eq!(backend.inner.pointer_epoch.load(Ordering::SeqCst), 1);
}

#[test]
fn a_tap_the_system_switched_off_is_switched_on_again() {
    let api = FakeMac::new();
    let backend = backend(&api);
    let swallowed = backend.handle_native(NativeKeyEvent { event_type: EVENT_TAP_DISABLED_BY_TIMEOUT, ..NativeKeyEvent::default() });
    assert!(!swallowed);
    assert_eq!((api.enabled.load(Ordering::SeqCst), backend.tap_revivals()), (1, 1));
}

#[test]
fn a_filtered_key_is_withheld_and_so_is_its_release() {
    let api = FakeMac::new();
    let backend = backend(&api);
    let (listener, events) = collector();
    backend.set_listener(Some(listener));
    backend.set_key_filter(Some(Arc::new(|event: &KeyEvent| if event.keycode == KEY_Z { KeyDisposition::Consume } else { KeyDisposition::Pass })));
    assert!(backend.handle_native(native(true, KEY_Z)));
    // The auto-repeat is withheld too, and the engine hears of it once.
    assert!(backend.handle_native(native(true, KEY_Z)));
    assert!(backend.handle_native(native(false, KEY_Z)));
    assert!(!backend.handle_native(native(false, KEY_Z)));
    assert!(!backend.handle_native(native(true, KEY_Q)));
    assert_eq!(events.lock().unwrap().len(), 4);
}

#[test]
fn the_backends_own_keys_are_never_filtered() {
    let api = FakeMac::new();
    let backend = backend(&api);
    backend.set_key_filter(Some(Arc::new(|_: &KeyEvent| KeyDisposition::Consume)));
    assert!(!backend.handle_native(NativeKeyEvent { injected: true, ..native(true, KEY_Z) }));
}

#[test]
fn keys_typed_during_a_correction_are_held_and_posted_again_as_replays() {
    let api = FakeMac::new();
    let backend = backend(&api);
    let (listener, events) = collector();
    backend.set_listener(Some(listener));
    backend.hold_input();
    assert!(backend.handle_native(native(true, KEY_Q)));
    assert!(backend.handle_native(native(false, KEY_Q)));
    assert!(events.lock().unwrap().is_empty());
    assert_eq!(backend.release_input(), Ok(2));
    assert_eq!(api.posted(), vec![NativeInput { replayed: true, ..NativeInput::new(true, KEY_Q) }, NativeInput { replayed: true, ..NativeInput::new(false, KEY_Q) }]);
    // Released: the next key goes through.
    assert!(!backend.handle_native(native(true, KEY_Q)));
}

#[test]
fn a_replayed_key_is_never_held() {
    let api = FakeMac::new();
    let backend = backend(&api);
    backend.hold_input();
    assert!(!backend.handle_native(NativeKeyEvent { replayed: true, ..native(true, KEY_Q) }));
}

#[test]
fn a_failed_repost_still_ends_the_hold() {
    let api = FakeMac::new();
    let backend = backend(&api);
    backend.hold_input();
    backend.handle_native(native(true, KEY_Q));
    *api.take.lock().unwrap() = Some(0);
    assert!(backend.release_input().is_err());
    assert!(!backend.handle_native(native(true, KEY_Q)));
}

#[test]
fn a_withheld_enter_is_delivered_once_and_the_text_behind_it_follows() {
    let api = FakeMac::new();
    let backend = backend(&api);
    let (listener, events) = collector();
    backend.set_listener(Some(listener));
    backend.set_key_filter(Some(Arc::new(|event: &KeyEvent| if event.keycode == MAC_VK_RETURN { KeyDisposition::Defer } else { KeyDisposition::Pass })));
    assert!(backend.handle_native(native(true, MAC_VK_RETURN)));
    assert!(events.lock().unwrap()[0].deferred);
    // Typed while the Enter waits: held, not lost.
    assert!(backend.handle_native(native(true, KEY_Z)));
    assert_eq!(backend.release_input(), Ok(0));
    assert_eq!(backend.complete_action(true), Ok(COMPLETED_ACTION_EVENT_COUNT));
    assert_eq!(
        api.posted(),
        vec![
            NativeInput::new(true, MAC_VK_RETURN),
            NativeInput::new(false, MAC_VK_RETURN),
            NativeInput { replayed: true, ..NativeInput::new(true, KEY_Z) },
        ]
    );
}

#[test]
fn a_withheld_enter_is_dropped_when_the_place_of_input_changed() {
    let api = FakeMac::new();
    let backend = backend(&api);
    backend.set_key_filter(Some(Arc::new(|event: &KeyEvent| if event.keycode == MAC_VK_RETURN { KeyDisposition::Defer } else { KeyDisposition::Pass })));
    backend.handle_native(native(true, MAC_VK_RETURN));
    api.window.store(11, Ordering::SeqCst);
    assert!(backend.complete_action(true).is_err());
    assert!(api.posted().is_empty());
    // Forgotten: the keyboard is free again.
    assert!(!backend.handle_native(native(true, KEY_Q)));
}

#[test]
fn a_correction_deletes_the_word_switches_the_layout_and_types_it_again() {
    let api = FakeMac::new();
    let backend = backend(&api);
    let strokes = [stroke(KEY_Z, 0, 0), stroke(KEY_Q, 0, 0)];
    let boundary = stroke(0x31, 0, 0);
    let held = backend.inject_correction(Injection { boundary: Some(&boundary), ..injection(&strokes, 1) });
    assert_eq!(held, Ok(0));
    assert_eq!(api.current_input_source(), RUSSIAN);
    let posted = api.posted();
    let keys: Vec<(bool, u32)> = posted.iter().map(|input| (input.pressed, input.keycode)).collect();
    // Three backspaces (two letters and the boundary), the two letters, the boundary.
    let expected: Vec<(bool, u32)> = [MAC_VK_BACKSPACE; 3]
        .iter()
        .flat_map(|key| [(true, *key), (false, *key)])
        .chain([(true, KEY_Z), (false, KEY_Z), (true, KEY_Q), (false, KEY_Q), (true, 0x31), (false, 0x31)])
        .collect();
    assert_eq!(keys, expected);
    assert!(posted.iter().all(|input| !input.replayed));
}

#[test]
fn a_capital_is_typed_with_the_shift_flag_on_the_key_itself() {
    let api = FakeMac::new();
    let backend = backend(&api);
    let strokes = [stroke(KEY_Z, 0, SHIFT_MASK)];
    backend.inject_correction(injection(&strokes, 1)).unwrap();
    let posted = api.posted();
    let tail = &posted[2..];
    assert_eq!(
        tail,
        &[
            NativeInput::new(true, MAC_VK_SHIFT),
            NativeInput { shift: true, ..NativeInput::new(true, KEY_Z) },
            NativeInput { shift: true, ..NativeInput::new(false, KEY_Z) },
            NativeInput::new(false, MAC_VK_SHIFT),
        ]
    );
}

#[test]
fn caps_lock_flips_what_the_shift_means_for_a_letter() {
    let api = FakeMac::new();
    api.caps.store(true, Ordering::SeqCst);
    let backend = backend(&api);
    // Typed with Caps Lock off and no Shift: with it on, the same key needs Shift to stay lower case... and a stroke that
    // was typed under Caps Lock needs none.
    let lower = stroke(KEY_Z, 0, 0);
    let under_caps = stroke(KEY_Z, 0, LOCK_MASK);
    let strokes = [lower, under_caps];
    backend.inject_correction(injection(&strokes, 1)).unwrap();
    let shifts: Vec<bool> = api.posted()[4..].iter().filter(|input| input.keycode == KEY_Z && input.pressed).map(|input| input.shift).collect();
    assert_eq!(shifts, vec![true, false]);
}

#[test]
fn a_typo_correction_erases_the_length_it_names() {
    let api = FakeMac::new();
    let backend = backend(&api);
    let strokes = [stroke(KEY_Z, 0, 0)];
    backend.inject_correction(Injection { erased: Some(3), ..injection(&strokes, 0) }).unwrap();
    let backspaces = api.posted().iter().filter(|input| input.keycode == MAC_VK_BACKSPACE && input.pressed).count();
    assert_eq!(backspaces, 3);
}

#[test]
fn keys_typed_after_the_word_are_typed_again_as_the_users_own() {
    let api = FakeMac::new();
    let backend = backend(&api);
    let strokes = [stroke(KEY_Z, 0, 0)];
    let late = [stroke(KEY_Q, 0, 0)];
    backend.inject_correction(Injection { late: &late, ..injection(&strokes, 1) }).unwrap();
    let posted = api.posted();
    let last = posted.last().unwrap();
    assert_eq!((last.keycode, last.replayed), (KEY_Q, true));
    let backspaces = posted.iter().filter(|input| input.keycode == MAC_VK_BACKSPACE && input.pressed).count();
    assert_eq!(backspaces, 2);
}

#[test]
fn a_kept_word_is_typed_in_its_own_layout_and_the_layout_stays_there() {
    let api = FakeMac::new();
    let backend = backend(&api);
    let strokes = [stroke(KEY_Z, 0, 0), stroke(KEY_Q, 0, 0)];
    backend.inject_correction(Injection { kept_tail: 1, source_group: Some(0), ..injection(&strokes, 1) }).unwrap();
    assert_eq!(api.current_input_source(), ABC);
    let mut bad = injection(&strokes, 1);
    bad.kept_tail = 1;
    assert!(backend.inject_correction(bad).is_err());
}

#[test]
fn bad_arguments_change_nothing() {
    let api = FakeMac::new();
    let backend = backend(&api);
    let strokes = [stroke(KEY_Z, 0, 0)];
    assert!(backend.inject_correction(injection(&strokes, 5)).is_err());
    assert!(backend.inject_correction(Injection { source_group: Some(7), ..injection(&strokes, 1) }).is_err());
    assert!(api.posted().is_empty());
}

#[test]
fn a_post_the_system_cut_short_is_an_error_naming_the_count() {
    let api = FakeMac::new();
    let backend = backend(&api);
    *api.take.lock().unwrap() = Some(1);
    let strokes = [stroke(KEY_Z, 0, 0)];
    let error = backend.inject_correction(injection(&strokes, 1)).unwrap_err();
    assert!(error.to_string().starts_with("macOS приняла 1 из "), "{error}");
}

#[test]
fn a_layout_the_system_never_confirms_is_reported_as_unconfirmed() {
    let api = FakeMac::new();
    api.switch_ignored.store(true, Ordering::SeqCst);
    let backend = backend(&api);
    let strokes = [stroke(KEY_Z, 0, 0)];
    let error = backend.inject_correction(injection(&strokes, 1)).unwrap_err();
    assert!(matches!(error, BackendError::LayoutSwitchUnconfirmed { group: 1, .. }), "{error:?}");
    assert!(api.posted().is_empty());
}

#[test]
fn the_place_of_input_must_not_change_between_the_hold_and_the_replacement() {
    let api = FakeMac::new();
    let backend = backend(&api);
    backend.hold_input();
    api.window.store(11, Ordering::SeqCst);
    let strokes = [stroke(KEY_Z, 0, 0)];
    let error = backend.inject_correction(injection(&strokes, 0)).unwrap_err();
    assert_eq!(error.to_string(), "Место ввода изменилось до замены; текст не изменён");
    // The hold ended with the failed correction.
    assert!(!backend.handle_native(native(true, KEY_Q)));
}

#[test]
fn the_group_follows_the_current_input_source() {
    let api = FakeMac::new();
    let backend = backend(&api);
    assert_eq!(backend.current_group(), Ok(0));
    backend.switch_group(1).unwrap();
    assert_eq!(backend.current_group(), Ok(1));
    *api.current.lock().unwrap() = "com.apple.keylayout.Other".to_owned();
    assert_eq!(backend.current_group(), Ok(0));
    assert!(backend.switch_group(2).is_err());
}

#[test]
fn the_focused_window_knows_whether_it_is_ours() {
    let api = FakeMac::new();
    let backend = backend(&api);
    assert_eq!(backend.focused_window(), Some(FocusInfo { window: 10, own: false, isolated_layout: false }));
    api.window.store(99, Ordering::SeqCst);
    assert!(backend.focused_window().unwrap().own);
    api.window.store(0, Ordering::SeqCst);
    assert_eq!(backend.focused_window(), None);
}

#[test]
fn the_tap_starts_delivers_and_stops() {
    let api = FakeMac::new();
    let backend = backend(&api);
    let (listener, events) = collector();
    backend.start(listener).unwrap();
    assert!(backend.running());
    let tap = api.tap.lock().unwrap().clone().unwrap();
    assert!(!tap(native(true, KEY_Q)));
    assert_eq!(events.lock().unwrap().len(), 1);
    backend.stop();
    assert!(!backend.running());
}

#[test]
fn a_tap_that_cannot_start_is_an_error_and_not_running() {
    let api = FakeMac::new();
    *api.tap_error.lock().unwrap() = Some(failed("macOS отказала в перехвате клавиатуры"));
    let backend = backend(&api);
    let (listener, _events) = collector();
    assert_eq!(backend.start(listener), Err(failed("macOS отказала в перехвате клавиатуры")));
    assert!(!backend.running());
}

#[test]
fn the_permission_is_asked_for_and_its_pane_opened_while_it_is_missing() {
    let api = FakeMac::new();
    api.trusted.store(false, Ordering::SeqCst);
    let backend = backend(&api);
    assert!(!backend.permission_granted());
    assert!(!backend.request_permission());
    api.trusted.store(true, Ordering::SeqCst);
    assert!(backend.request_permission());
}

/// A selection the test scripts: what `replace` answers and what the keyboard then sees.
struct FakeSelection {
    replace: Mutex<Option<Result<SelectionAfter, SelectionError>>>,
    caret: Option<(i64, i64)>,
}

impl SelectionAccess for FakeSelection {
    fn read(&self) -> Result<SelectedText, SelectionError> {
        Ok(SelectedText { text: "ghbdtn".to_owned(), source: "ax".to_owned() })
    }
    fn selected_range(&self, _selected: &str) -> Result<Option<(i64, i64)>, SelectionError> {
        Ok(Some((0, 6)))
    }
    fn caret_range(&self) -> Option<(i64, i64)> {
        self.caret
    }
    fn replace(&self, _selected: &str, _replacement: &str) -> Result<SelectionAfter, SelectionError> {
        self.replace.lock().unwrap().take().unwrap()
    }
}

fn selecting(api: &Arc<FakeMac>, replace: Result<SelectionAfter, SelectionError>, caret: Option<(i64, i64)>) -> MacBackend {
    let cell = Mutex::new(Some((replace, caret)));
    let factory: SelectionAccessFactory = Box::new(move || {
        let (replace, caret) = cell.lock().unwrap().take().unwrap();
        Box::new(FakeSelection { replace: Mutex::new(Some(replace)), caret })
    });
    let parts = BackendParts { clock: Arc::new(StepClock::default()), selection_access_factory: factory, ..BackendParts::default() };
    MacBackend::with_parts(api.clone(), parts)
}

fn selected() -> SelectedText {
    SelectedText { text: "ghbdtn".to_owned(), source: "ax".to_owned() }
}

#[test]
fn a_selection_the_field_keeps_selected_needs_no_keys() {
    let api = FakeMac::new();
    let backend = selecting(&api, Ok(SelectionAfter::Kept), None);
    assert_eq!(backend.read_selection("TextEdit", 10).unwrap().text, "ghbdtn");
    assert_eq!(backend.replace_selection(&selected(), "привет", Some(1)), Ok(true));
    assert!(api.posted().is_empty());
    assert_eq!(api.current_input_source(), RUSSIAN);
}

#[test]
fn a_caret_left_after_the_replacement_is_selected_back_with_shift_and_left() {
    let api = FakeMac::new();
    let backend = selecting(&api, Ok(SelectionAfter::CaretAfter), None);
    assert_eq!(backend.replace_selection(&selected(), "ab", None), Ok(true));
    let posted = api.posted();
    assert_eq!(posted.len(), 2 + 4);
    assert_eq!((posted[0], posted[5]), (NativeInput::new(true, MAC_VK_SHIFT), NativeInput::new(false, MAC_VK_SHIFT)));
    assert!(posted[1..5].iter().all(|input| input.keycode == MAC_VK_LEFT_ARROW && input.shift));
}

#[test]
fn a_replacement_that_cannot_be_counted_in_steps_is_left_unselected() {
    let api = FakeMac::new();
    let backend = selecting(&api, Ok(SelectionAfter::CaretAfter), None);
    assert_eq!(backend.replace_selection(&selected(), "a😀", None), Ok(false));
    assert!(api.posted().is_empty());
}

#[test]
fn a_field_that_cannot_be_written_gets_a_paste_and_gives_the_pasteboard_back() {
    let api = FakeMac::new();
    let refusal = Err(SelectionError::Refused(SelectionRefused::new(RefusalReason::NotSettable)));
    let backend = selecting(&api, refusal, Some((0, 0)));
    assert_eq!(backend.replace_selection(&selected(), "привет", None), Ok(false));
    assert_eq!(
        api.posted(),
        vec![
            NativeInput::new(true, MAC_VK_COMMAND),
            NativeInput { command: true, ..NativeInput::new(true, MAC_VK_ANSI_V) },
            NativeInput { command: true, ..NativeInput::new(false, MAC_VK_ANSI_V) },
            NativeInput::new(false, MAC_VK_COMMAND),
        ]
    );
    assert_eq!(*api.board.text.lock().unwrap(), "привет");
    assert_eq!(api.board.contents.lock().unwrap().as_ref().unwrap().items[0][0].1, b"mine".to_vec());
}

#[test]
fn a_pasteboard_that_cannot_be_given_back_is_not_borrowed() {
    let api = FakeMac::new();
    let mut fake = Arc::try_unwrap(api).ok().unwrap();
    fake.board.complete = false;
    let api = Arc::new(fake);
    let refusal = Err(SelectionError::Refused(SelectionRefused::new(RefusalReason::NotSettable)));
    let backend = selecting(&api, refusal, None);
    assert_eq!(backend.replace_selection(&selected(), "x", None), Err(refused(RefusalReason::ClipboardUnrestorable)));
    assert!(api.posted().is_empty());
}

#[test]
fn other_refusals_pass_through_untouched() {
    let api = FakeMac::new();
    let backend = selecting(&api, Err(refused(RefusalReason::FocusChanged)), None);
    assert_eq!(backend.replace_selection(&selected(), "x", None), Err(refused(RefusalReason::FocusChanged)));
}
