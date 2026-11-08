//! The program in a real Wayland session (GNOME Shell, KDE Plasma, sway): a virtual USB keyboard and tablet through
//! `/dev/uinput`, the input helper's grab, typing into the window of another process. With `KS_E2E_WAYLAND_SOURCE` the
//! engine runs in this process through the typing cases, the deferred Enter, the hotkeys, the click, Caps Lock, a plugged
//! keyboard, a password field, a taught rule, a prefix and a locked session; with `KS_E2E_WAYLAND_BINARY` the packaged
//! program is driven, down to a frozen main process and a killed helper. `KS_E2E_DESKTOP` names the desktop. Without them
//! it says so and passes. The harness keeps it in the VM: nothing else should have its keyboard grabbed.

#![expect(unsafe_code, reason = "calls into the Linux kernel and system libraries")]
use ks_cast::To as _;
mod support;

use std::collections::VecDeque;
use std::ffi::CString;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gtk::{gdk, glib};
use gtk::prelude::*;
use ks_backend::InputBackend;
use ks_config::config::SettingsStore;
use ks_config::history::HistoryStore;
use ks_config::learning::LearningStore;
use ks_engine::rule_editor::{RuleDraft, build_rule, draft_from_prompt};
use ks_engine::{KeySwitchEngine, LearningPrompt};
use ks_linux::gtk_hint::{HintCalls, HintClicked, HintDriver, HintScreen};
use ks_linux::gtk_model::{HINT_WINDOW_TITLE, LEARNING_PROMPT_WINDOW_TITLE};
use ks_linux::gtk_prompt::{LearningPromptWindow, PromptAnchor, PromptCallbacks, RuleCallbacks, RuleEditorWindow};
use ks_linux::linux_app::{engine_builder, selection_factory};
use ks_linux::tray::{MENU_INTERFACE, MENU_PATH};
use ks_linux::wayland_constants::{EV_KEY, EV_LED, EV_REP, EV_SYN, EVIOCGNAME_256, EVIOCGRAB, UI_DEV_CREATE, UI_DEV_DESTROY, UI_DEV_SETUP, UI_SET_EVBIT, UI_SET_KEYBIT, UI_SET_LEDBIT, UINPUT_MAX_NAME_BYTES, WAYLAND_FORWARDED_LEDS, WAYLAND_UINPUT_MAX_KEYCODE, WAYLAND_UINPUT_VERSION};
use ks_linux::wayland_native::new_wayland_backend;
use ks_ui::hints::{EngineInput, Hints, Moment, RUNNING_PLATFORM, subscribe};
use serde_json::{Value, json};
use support::{CONTEXT_RESOLUTION, ENTRY_MARGIN, IDLE_PAUSE, WINDOW_HEIGHT, WINDOW_WIDTH, Watcher, dbus_call, pump_until};

const TARGET_ROLE: &str = "KS_E2E_WAYLAND_TARGET";
const TARGET_APPLICATION_ID: &str = "io.github.olegius88.KeySwitchWaylandTarget";
const KEYBOARD_NAME: &str = "USB HID Test Keyboard";
const BUS_USB: u16 = 0x03;
const VENDOR_ID: u16 = 0x046D;
const PRODUCT_ID: u16 = 0xC31C;
const INTER_KEY: Duration = Duration::from_millis(20);
const POLL: Duration = Duration::from_millis(20);
const GRAB_SETTLE: Duration = Duration::from_millis(450);
const SIGSTOP_SETTLE: Duration = Duration::from_millis(100);
const WATCHDOG_DEADLINE: Duration = Duration::from_millis(1500);
const FOCUS_DEADLINE: Duration = Duration::from_secs(8);
const TRAY_DEADLINE: Duration = Duration::from_secs(60);
const INTER_CASE: Duration = Duration::from_millis(200);
const SETTLE: Duration = Duration::from_millis(900);
const LATE: Duration = Duration::from_millis(4900);
const SHUTDOWN: Duration = Duration::from_secs(8);
const WARMUP_ATTEMPTS: u32 = 3;
const KEY_PRESS: i32 = 1;
const KEY_RELEASE: i32 = 0;
const SYN_REPORT: u16 = 0;
const KEY_ENTER: u16 = 28;
const KEY_LEFTCTRL: u16 = 29;
const KEY_A: u16 = 30;
const KEY_BACKSPACE: u16 = 14;
const NAME_BUFFER_BYTES: usize = 256;
const BUS_NAME_PREFIX: &str = "io.github.olegius88.KeySwitch.StatusNotifierItem.p";
const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
const HOTPLUG_NAME: &str = "USB Hotplug Keyboard";
const HOTPLUG_PRODUCT_ID: u16 = 0xC31D;
const TABLET_NAME: &str = "USB Test Tablet";
const TABLET_PRODUCT_ID: u16 = 0xC078;
const EV_ABS: u32 = 0x03;
const BTN_MOUSE: u16 = 0x110;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_MAX: i32 = 32767;
const UI_SET_ABSBIT: libc::c_ulong = 0x4004_5567;
const UI_ABS_SETUP: libc::c_ulong = 0x401C_5504;
const KEY_LEFTALT: u16 = 56;
const KEY_CAPSLOCK: u16 = 58;
const KEY_PAUSE: u16 = 119;
const KEY_Z: u16 = 44;
const KEY_P: u16 = 25;
const LED_CAPSL: u16 = 1;
const CAPS_DEADLINE: Duration = Duration::from_secs(2);
const LOCK_SETTLE: Duration = Duration::from_millis(800);
const SLOW_KEY: Duration = Duration::from_millis(75);
const NATIVE_CASE_SETTLE: u64 = 900;
const GRACE_MS: u64 = 4000;
const DOCUMENT_PORTAL: &str = "org.freedesktop.portal.Documents";
/// The longest a hint takes to appear after a correction, and to have the keyboard given back.
const HINT_APPEARS: Duration = Duration::from_secs(4);
const HINT_GIVES_KEYBOARD_BACK: Duration = Duration::from_secs(2);
const CASES: [support::Case; 15] = [
    support::Case("EN pause correction", 0, "ghbdtn", "привет", 1, IDLE_PAUSE),
    support::Case("RU keys to English", 1, "hello ", "hello ", 0, NATIVE_CASE_SETTLE),
    support::Case("punctuation key is a Russian letter", 0, ",fpf ", "база ", 1, NATIVE_CASE_SETTLE),
    support::Case("return to EN before punctuation test", 1, "hello ", "hello ", 0, NATIVE_CASE_SETTLE),
    support::Case("ambiguous punctuation keeps its glyph after idle", 0, "ghbdtn,", "привет,", 1, IDLE_PAUSE),
    support::Case("manual layout switch protects next word", 0, "ghbdtn ", "ghbdtn ", 0, NATIVE_CASE_SETTLE),
    support::Case("manual protection is consumed once", 0, "ghbdtn ", "привет ", 1, NATIVE_CASE_SETTLE),
    support::Case("short Russian word switches to English with its neighbour", 1, "if hello ", "if hello ", 0, CONTEXT_RESOLUTION),
    support::Case("manual Russian selection protects short word", 1, "if ", "ша ", 1, NATIVE_CASE_SETTLE),
    support::Case("short-word protection is consumed once", 1, "if hello ", "if hello ", 0, CONTEXT_RESOLUTION),
    support::Case("context resolves a short word with the next word", 0, "e 'njuj ", "у этого ", 1, CONTEXT_RESOLUTION),
    support::Case("return to EN before internal punctuation", 1, "hello ", "hello ", 0, NATIVE_CASE_SETTLE),
    support::Case("learned boundaries keep the internal comma key", 0, "ghj,ktvf ", "проблема ", 1, NATIVE_CASE_SETTLE),
    support::Case("return to EN before isolated punctuation", 1, "hello ", "hello ", 0, NATIVE_CASE_SETTLE),
    support::Case("isolated dot stays literal after idle", 0, ".", ".", 0, IDLE_PAUSE),
];

fn wait_until(limit: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        if condition() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(POLL);
    }
}

/// `struct uinput_setup`: the device's identity and name.
#[repr(C)]
struct UinputSetup {
    bus: u16,
    vendor: u16,
    product: u16,
    version: u16,
    name: [u8; UINPUT_MAX_NAME_BYTES],
    effects: u32,
}

/// `struct input_event`.
#[repr(C)]
struct InputEvent {
    time: libc::timeval,
    kind: u16,
    code: u16,
    value: i32,
}

/// A simulated hardware keyboard, fed into the kernel's input subsystem.
struct PhysicalKeyboard {
    descriptor: i32,
    name: &'static str,
    leds: std::cell::RefCell<std::collections::HashMap<u16, i32>>,
}

fn io_error(what: &str) -> String {
    format!("{what}: {}", std::io::Error::last_os_error())
}

impl PhysicalKeyboard {
    fn new() -> Result<PhysicalKeyboard, String> {
        Self::named(KEYBOARD_NAME, PRODUCT_ID)
    }

    fn named(device: &'static str, product: u16) -> Result<PhysicalKeyboard, String> {
        let path = CString::new("/dev/uinput").expect("a path");
        // SAFETY: the path is NUL-terminated, and the descriptor is checked.
        let descriptor = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_NONBLOCK | libc::O_CLOEXEC) };
        if descriptor < 0 {
            return Err(io_error("/dev/uinput"));
        }
        let keyboard = PhysicalKeyboard { descriptor, name: device, leds: std::cell::RefCell::default() };
        // SAFETY: each ioctl takes the integer the kernel's uinput interface documents for it, and `setup` outlives its call.
        unsafe {
            for event in [EV_KEY, EV_SYN, EV_LED, EV_REP] {
                libc::ioctl(descriptor, UI_SET_EVBIT, event);
            }
            for code in 1..=WAYLAND_UINPUT_MAX_KEYCODE {
                libc::ioctl(descriptor, UI_SET_KEYBIT, code);
            }
            for led in WAYLAND_FORWARDED_LEDS {
                libc::ioctl(descriptor, UI_SET_LEDBIT, led);
            }
            let mut name = [0_u8; UINPUT_MAX_NAME_BYTES];
            name[..device.len()].copy_from_slice(device.as_bytes());
            let setup = UinputSetup { bus: BUS_USB, vendor: VENDOR_ID, product, version: WAYLAND_UINPUT_VERSION.low::<u16>(), name, effects: 0 };
            if libc::ioctl(descriptor, UI_DEV_SETUP, &setup) < 0 || libc::ioctl(descriptor, UI_DEV_CREATE) < 0 {
                return Err(io_error("the virtual keyboard"));
            }
        }
        Ok(keyboard)
    }

    fn emit(&self, code: u16, value: i32) -> Result<(), String> {
        let now = libc::timeval { tv_sec: 0, tv_usec: 0 };
        let events = [InputEvent { time: now, kind: EV_KEY.low::<u16>(), code, value }, InputEvent { time: now, kind: EV_SYN.low::<u16>(), code: SYN_REPORT, value: 0 }];
        // SAFETY: the bytes written are those of the array, which outlives the call.
        let written = unsafe { libc::write(self.descriptor, events.as_ptr().cast(), size_of_val(&events)) };
        if written < 0 { Err(io_error("a key event")) } else { Ok(()) }
    }

    fn tap(&self, code: u16) -> Result<(), String> {
        self.emit(code, KEY_PRESS)?;
        std::thread::sleep(INTER_KEY);
        self.emit(code, KEY_RELEASE)?;
        std::thread::sleep(INTER_KEY);
        Ok(())
    }

    fn type_keys(&self, text: &str) -> Result<(), String> {
        for character in text.chars() {
            self.tap(code_of(character).ok_or_else(|| format!("no key for {character:?}"))?)?;
        }
        Ok(())
    }

    fn chord(&self, modifiers: &[u16], key: u16) -> Result<(), String> {
        for modifier in modifiers {
            self.emit(*modifier, KEY_PRESS)?;
            std::thread::sleep(INTER_KEY);
        }
        self.emit(key, KEY_PRESS)?;
        std::thread::sleep(INTER_KEY);
        self.emit(key, KEY_RELEASE)?;
        for modifier in modifiers.iter().rev() {
            std::thread::sleep(INTER_KEY);
            self.emit(*modifier, KEY_RELEASE)?;
        }
        std::thread::sleep(INTER_KEY);
        Ok(())
    }

    fn clear_field(&self) -> Result<(), String> {
        self.chord(&[KEY_LEFTCTRL], KEY_A)?;
        self.tap(KEY_BACKSPACE)
    }

    /// What the compositor last told this keyboard's lights.
    fn drain_leds(&self) -> std::collections::HashMap<u16, i32> {
        let mut leds = self.leds.borrow_mut();
        loop {
            let mut event = InputEvent { time: libc::timeval { tv_sec: 0, tv_usec: 0 }, kind: 0, code: 0, value: 0 };
            // SAFETY: the kernel writes at most the size of the event it is given.
            let read = unsafe { libc::read(self.descriptor, (&raw mut event).cast(), size_of::<InputEvent>()) };
            if read != (size_of::<InputEvent>()).bits::<isize>() {
                break;
            }
            if u32::from(event.kind) == EV_LED {
                leds.insert(event.code, event.value);
            }
        }
        leds.clone()
    }

    /// The event node of this keyboard, found by its name.
    fn event_node(&self) -> Option<PathBuf> {
        let mut nodes: Vec<PathBuf> = std::fs::read_dir("/dev/input").ok()?.flatten().map(|entry| entry.path()).filter(|path| path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with("event"))).collect();
        nodes.sort();
        nodes.into_iter().find(|node| {
            let Ok(path) = CString::new(node.as_os_str().as_encoded_bytes()) else { return false };
            // SAFETY: the path is NUL-terminated; the buffer has the size the request names; the descriptor is closed here.
            unsafe {
                let descriptor = libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC);
                if descriptor < 0 {
                    return false;
                }
                let mut buffer = [0_u8; NAME_BUFFER_BYTES];
                let answered = libc::ioctl(descriptor, EVIOCGNAME_256, buffer.as_mut_ptr()) >= 0;
                libc::close(descriptor);
                answered && std::ffi::CStr::from_bytes_until_nul(&buffer).is_ok_and(|name| name.to_bytes() == self.name.as_bytes())
            }
        })
    }

    /// Whether another process holds the keyboard: trying to grab it answers EBUSY.
    fn is_grabbed(&self) -> bool {
        let Some(node) = self.event_node() else { return false };
        let Ok(path) = CString::new(node.as_os_str().as_encoded_bytes()) else { return false };
        // SAFETY: as above; a grab taken here is released at once.
        unsafe {
            let descriptor = libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC);
            if descriptor < 0 {
                return false;
            }
            let taken = libc::ioctl(descriptor, EVIOCGRAB, 1) >= 0;
            let busy = !taken && std::io::Error::last_os_error().raw_os_error() == Some(libc::EBUSY);
            if taken {
                libc::ioctl(descriptor, EVIOCGRAB, 0);
            }
            libc::close(descriptor);
            busy
        }
    }
}

impl Drop for PhysicalKeyboard {
    fn drop(&mut self) {
        // SAFETY: the descriptor is ours and is not used after this.
        unsafe {
            libc::ioctl(self.descriptor, UI_DEV_DESTROY);
            libc::close(self.descriptor);
        }
    }
}

/// A simulated absolute pointer, as QEMU's tablet: a relative mouse would click wherever it happens to be.
struct PhysicalPointer {
    descriptor: i32,
}

/// `struct uinput_abs_setup`.
#[repr(C)]
struct AbsSetup {
    code: u16,
    info: [i32; 6],
}

impl PhysicalPointer {
    fn new() -> Result<PhysicalPointer, String> {
        let path = CString::new("/dev/uinput").expect("a path");
        // SAFETY: as for the keyboard.
        unsafe {
            let descriptor = libc::open(path.as_ptr(), libc::O_RDWR | libc::O_NONBLOCK | libc::O_CLOEXEC);
            if descriptor < 0 {
                return Err(io_error("/dev/uinput"));
            }
            let pointer = PhysicalPointer { descriptor };
            for event in [EV_KEY, EV_ABS, EV_SYN] {
                libc::ioctl(descriptor, UI_SET_EVBIT, event);
            }
            libc::ioctl(descriptor, UI_SET_KEYBIT, u32::from(BTN_MOUSE));
            for axis in [ABS_X, ABS_Y] {
                libc::ioctl(descriptor, UI_SET_ABSBIT, u32::from(axis));
                let setup = AbsSetup { code: axis, info: [0, 0, ABS_MAX, 0, 0, 0] };
                libc::ioctl(descriptor, UI_ABS_SETUP, &setup);
            }
            let mut name = [0_u8; UINPUT_MAX_NAME_BYTES];
            name[..TABLET_NAME.len()].copy_from_slice(TABLET_NAME.as_bytes());
            let setup = UinputSetup { bus: BUS_USB, vendor: VENDOR_ID, product: TABLET_PRODUCT_ID, version: WAYLAND_UINPUT_VERSION.low::<u16>(), name, effects: 0 };
            if libc::ioctl(descriptor, UI_DEV_SETUP, &setup) < 0 || libc::ioctl(descriptor, UI_DEV_CREATE) < 0 {
                return Err(io_error("the virtual pointer"));
            }
            Ok(pointer)
        }
    }

    fn emit(&self, events: &[(u32, u16, i32)]) -> Result<(), String> {
        let now = libc::timeval { tv_sec: 0, tv_usec: 0 };
        let packed: Vec<InputEvent> = events.iter().map(|(kind, code, value)| InputEvent { time: now, kind: (*kind).low::<u16>(), code: *code, value: *value }).collect();
        // SAFETY: the bytes written are those of the vector, which outlives the call.
        let written = unsafe { libc::write(self.descriptor, packed.as_ptr().cast(), packed.len() * size_of::<InputEvent>()) };
        if written < 0 { Err(io_error("a pointer event")) } else { Ok(()) }
    }

    /// Presses and releases the left button at a logical point of the screen.
    fn click_at(&self, (x, y): (i64, i64), (left, top, width, height): (i64, i64, i64, i64)) -> Result<(), String> {
        let scaled = |value: i64, origin: i64, size: i64| ((value - origin) * i64::from(ABS_MAX) / (size - 1).max(1)).low::<i32>();
        self.emit(&[(EV_ABS, ABS_X, scaled(x, left, width)), (EV_ABS, ABS_Y, scaled(y, top, height)), (EV_SYN, SYN_REPORT, 0)])?;
        std::thread::sleep(INTER_KEY);
        for value in [KEY_PRESS, KEY_RELEASE] {
            self.emit(&[(EV_KEY, BTN_MOUSE, value), (EV_SYN, SYN_REPORT, 0)])?;
            std::thread::sleep(INTER_KEY);
        }
        Ok(())
    }
}

impl Drop for PhysicalPointer {
    fn drop(&mut self) {
        // SAFETY: the descriptor is ours and is not used after this.
        unsafe {
            libc::ioctl(self.descriptor, UI_DEV_DESTROY);
            libc::close(self.descriptor);
        }
    }
}

/// The evdev code of a character on the US layout, for what this test types.
fn code_of(character: char) -> Option<u16> {
    Some(match character {
        'a' => 30,
        'b' => 48,
        'c' => 46,
        'd' => 32,
        'e' => 18,
        'f' => 33,
        'g' => 34,
        'h' => 35,
        'i' => 23,
        'j' => 36,
        'k' => 37,
        'l' => 38,
        'n' => 49,
        'o' => 24,
        'p' => 25,
        'r' => 19,
        't' => 20,
        'u' => 22,
        'v' => 47,
        ',' => 51,
        '.' => 52,
        '\'' => 40,
        ' ' => 57,
        '\n' => KEY_ENTER,
        _ => return None,
    })
}

/// The window typed into, in a process of its own: the program neither follows the focus into a window of its own process
/// nor reads its fields, so a window here would hide what a user's application goes through.
mod target {
    use super::{json, Value, Write, DisplayExt, ApplicationExt, TARGET_APPLICATION_ID, EntryExt, GtkWindowExt, WINDOW_WIDTH, WINDOW_HEIGHT, EditableExt, DeviceExt, Cast, WidgetExt, ENTRY_MARGIN, BoxExt, gdk, SeatExt, channel, BufRead, glib, Duration, ApplicationExtManual};

    fn emit(event: &Value) {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{event}");
        let _ = out.flush();
    }

    pub(crate) fn main() {
        let application = gtk::Application::builder().application_id(TARGET_APPLICATION_ID).build();
        application.connect_activate(|application| {
            let window = gtk::ApplicationWindow::builder().application(application).title("KeySwitch Wayland E2E target").default_width(WINDOW_WIDTH).default_height(WINDOW_HEIGHT).build();
            let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let entry = gtk::Entry::builder().placeholder_text("Wayland E2E input").build();
            // As GTK 4 applications ask for a password: the field tells AT-SPI and the compositor what it is.
            let password = gtk::PasswordEntry::builder().placeholder_text("Password input").build();
            entry.connect_changed(|entry| emit(&json!({"event": "text", "widget": "entry", "text": entry.text().as_str()})));
            password.connect_changed(|entry| emit(&json!({"event": "text", "widget": "password", "text": entry.text().as_str()})));
            entry.connect_activate(|entry| emit(&json!({"event": "activate", "text": entry.text().as_str()})));
            for widget in [entry.upcast_ref::<gtk::Widget>(), password.upcast_ref::<gtk::Widget>()] {
                widget.set_margin_top(ENTRY_MARGIN);
                widget.set_margin_start(ENTRY_MARGIN);
                widget.set_margin_end(ENTRY_MARGIN);
                holder.append(widget);
            }
            window.set_child(Some(&holder));
            if let Some(keyboard) = gdk::Display::default().and_then(|display| display.default_seat()).and_then(|seat| seat.keyboard()) {
                keyboard.connect_caps_lock_state_notify(|device| emit(&json!({"event": "caps", "value": device.is_caps_locked()})));
                emit(&json!({"event": "caps", "value": keyboard.is_caps_locked()}));
            }
            window.present();
            let (sender, receiver) = channel::<String>();
            std::thread::spawn(move || {
                for line in std::io::stdin().lock().lines().map_while(Result::ok) {
                    let _ = sender.send(line);
                }
            });
            let application = application.clone();
            glib::timeout_add_local(Duration::from_millis(10), move || {
                while let Ok(line) = receiver.try_recv() {
                    let command: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
                    let name = command["widget"].as_str().unwrap_or("entry");
                    let widget: &gtk::Widget = if name == "password" { password.upcast_ref() } else { entry.upcast_ref() };
                    let editable: &gtk::Editable = if name == "password" { password.upcast_ref() } else { entry.upcast_ref() };
                    match command["cmd"].as_str().unwrap_or("") {
                        "present" => window.present(),
                        "focus" => {
                            widget.grab_focus();
                            editable.set_position(-1);
                        }
                        "set_text" => {
                            editable.set_text(command["text"].as_str().unwrap_or(""));
                            editable.set_position(-1);
                        }
                        "bounds" => {
                            let bounds = widget.compute_bounds(&window);
                            let (x, y, width, height) = bounds.map_or((0.0, 0.0, 0.0, 0.0), |area| (area.x(), area.y(), area.width(), area.height()));
                            emit(&json!({"event": "bounds", "widget": name, "ok": bounds.is_some(), "x": x, "y": y, "width": width, "height": height}));
                        }
                        "quit" => application.quit(),
                        _ => {}
                    }
                    emit(&json!({"event": "done", "seq": command["seq"]}));
                }
                glib::ControlFlow::Continue
            });
        });
        application.run_with_args::<&str>(&[]);
    }
}

#[derive(Default)]
struct TargetState {
    text: String,
    password: String,
    activations: VecDeque<String>,
    done: u64,
    caps: bool,
    bounds: Option<(bool, f64, f64, f64, f64)>,
}

struct TargetApp {
    child: Child,
    state: Arc<Mutex<TargetState>>,
    sequence: u64,
}

impl TargetApp {
    fn new() -> Result<TargetApp, String> {
        let program = std::env::current_exe().map_err(|error| error.to_string())?;
        let mut child = Command::new(program).env(TARGET_ROLE, "1").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().map_err(|error| error.to_string())?;
        let stdout = child.stdout.take().ok_or("no target output")?;
        let state: Arc<Mutex<TargetState>> = Arc::default();
        let reading = state.clone();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
                let Ok(event) = serde_json::from_str::<Value>(&line) else { continue };
                let mut state = reading.lock().expect("the target's state");
                match event["event"].as_str().unwrap_or("") {
                    "text" if event["widget"] == "password" => event["text"].as_str().unwrap_or("").clone_into(&mut state.password),
                    "text" => event["text"].as_str().unwrap_or("").clone_into(&mut state.text),
                    "caps" => state.caps = event["value"] == json!(true),
                    "bounds" => state.bounds = Some((event["ok"] == json!(true), event["x"].as_f64().unwrap_or(0.0), event["y"].as_f64().unwrap_or(0.0), event["width"].as_f64().unwrap_or(0.0), event["height"].as_f64().unwrap_or(0.0))),
                    "activate" => state.activations.push_back(event["text"].as_str().unwrap_or("").to_owned()),
                    "done" => state.done = state.done.max(event["seq"].as_u64().unwrap_or(0)),
                    _ => {}
                }
            }
        });
        Ok(TargetApp { child, state, sequence: 0 })
    }

    fn command(&mut self, name: &str, text: &str) -> Result<(), String> {
        self.command_on("entry", name, text)
    }

    fn command_on(&mut self, widget: &str, name: &str, text: &str) -> Result<(), String> {
        self.sequence += 1;
        let sequence = self.sequence;
        let stdin = self.child.stdin.as_mut().ok_or("no target input")?;
        writeln!(stdin, "{}", json!({"cmd": name, "seq": sequence, "text": text, "widget": widget})).map_err(|error| error.to_string())?;
        stdin.flush().map_err(|error| error.to_string())?;
        let state = self.state.clone();
        if wait_until(FOCUS_DEADLINE, || state.lock().expect("the target's state").done >= sequence) { Ok(()) } else { Err(format!("the target application did not answer {name}")) }
    }

    fn text(&self) -> String {
        self.state.lock().expect("the target's state").text.clone()
    }

    fn password(&self) -> String {
        self.state.lock().expect("the target's state").password.clone()
    }

    fn caps_lock(&self) -> bool {
        self.state.lock().expect("the target's state").caps
    }

    fn bounds(&mut self, widget: &str) -> Result<(bool, f64, f64, f64, f64), String> {
        self.command_on(widget, "bounds", "")?;
        self.state.lock().expect("the target's state").bounds.ok_or_else(|| "the target gave no bounds".to_owned())
    }

    fn activations(&self) -> Vec<String> {
        self.state.lock().expect("the target's state").activations.iter().cloned().collect()
    }

    fn clear_activations(&self) {
        self.state.lock().expect("the target's state").activations.clear();
    }

    fn focused(&mut self, keyboard: &PhysicalKeyboard) -> Result<(), String> {
        let deadline = Instant::now() + FOCUS_DEADLINE;
        while Instant::now() < deadline {
            self.command("present", "")?;
            self.command("focus", "")?;
            std::thread::sleep(INTER_CASE);
            self.command("set_text", "")?;
            keyboard.tap(KEY_A)?;
            if wait_until(SETTLE, || !self.text().is_empty()) {
                keyboard.clear_field()?;
                wait_until(SETTLE, || self.text().is_empty());
                return Ok(());
            }
        }
        Err(format!("the entry never received the Wayland keyboard focus: {:?}", self.text()))
    }
}

impl Drop for TargetApp {
    fn drop(&mut self) {
        let _ = self.command("quit", "");
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn unprivileged() -> Result<(), String> {
    // SAFETY: takes nothing.
    if unsafe { libc::geteuid() } == 0 {
        return Err("the test must run as an unprivileged desktop user".to_owned());
    }
    let groups = String::from_utf8_lossy(&Command::new("id").arg("-Gn").output().map_err(|error| error.to_string())?.stdout).into_owned();
    if groups.split_whitespace().any(|group| group == "input") {
        return Err(format!("the user belongs to the input group ({groups}): the session's access would not be proven"));
    }
    println!("UNPRIVILEGED_UACCESS_OK");
    Ok(())
}

fn has_owner(name: &str) -> bool {
    dbus_call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus.NameHasOwner", &[name]).is_ok_and(|answer| answer.contains("true"))
}

fn diagnose(binary: &Path, desktop: &str, scratch: &Path) -> Result<(), String> {
    let output = Command::new(binary).arg("--diagnose").env("KEYSWITCH_DATA_DIR", scratch.join("diagnostic")).output().map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!("--diagnose ended with {}: {}", output.status, String::from_utf8_lossy(&output.stderr)));
    }
    let report: Value = serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())?;
    if report["available"] != json!(true) || report["session_type"] != json!("wayland") {
        return Err(format!("--diagnose did not report an available Wayland probe: {report}"));
    }
    if !desktop.is_empty() && !report["display"].as_str().unwrap_or("").to_lowercase().contains(&desktop.to_lowercase()) {
        return Err(format!("--diagnose names another desktop: {report}"));
    }
    println!("PACKAGED_DIAGNOSE_WAYLAND_OK");
    Ok(())
}

fn send_signal(child: u32, signal: i32) {
    // SAFETY: a signal to a process this test started.
    unsafe {
        libc::kill(child.bits::<i32>(), signal);
    }
}

fn packaged(binary: &Path, desktop: &str) -> Result<(), String> {
    unprivileged()?;
    let root = std::env::temp_dir().join(format!("keyswitch-e2e-wayland-{}", std::process::id()));
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    diagnose(binary, desktop, &root)?;
    let keyboard = PhysicalKeyboard::new()?;
    std::thread::sleep(GRAB_SETTLE);
    let settings = SettingsStore::new(Some(root.join("config/config.json")));
    for (path, value) in [
        ("general.autostart", json!(false)),
        ("general.notifications", json!(false)),
        ("general.sound", json!(false)),
        ("general.keep_history", json!(true)),
        ("appearance.show_indicator", json!(true)),
        ("appearance.indicator_style", json!("letters")),
        ("detection.early_switch", json!(false)),
    ] {
        settings.set(path, value, true).map_err(|error| format!("{error:?}"))?;
    }
    let watcher = if has_owner(WATCHER_NAME) { None } else { Some(Watcher::new()?) };
    let mut target = TargetApp::new()?;
    target.command("present", "")?;
    target.command("focus", "")?;
    std::thread::sleep(GRAB_SETTLE);
    let control = new_wayland_backend(None)?;
    let original = control.current_group().map_err(|error| format!("{error:?}"))?;
    control.switch_group(0).map_err(|error| format!("{error:?}"))?;
    let mut child = Command::new(binary)
        .arg("--hidden")
        .env("KEYSWITCH_CONFIG_DIR", root.join("config"))
        .env("KEYSWITCH_DATA_DIR", root.join("data"))
        .env("XDG_CONFIG_HOME", root.join("xdg-config"))
        .env("XDG_DATA_HOME", root.join("xdg-data"))
        .spawn()
        .map_err(|error| error.to_string())?;
    let history = HistoryStore::new(Some(root.join("data/history.jsonl")), 200);
    let outcome = (|| -> Result<(), String> {
        let destination = format!("{BUS_NAME_PREFIX}{}", child.id());
        if !wait_until(TRAY_DEADLINE, || child.try_wait().ok().flatten().is_none() && has_owner(&destination)) {
            return Err("the program did not register its panel icon".to_owned());
        }
        if let Some(watcher) = &watcher
            && watcher.service().is_none() {
                return Err("the watcher did not receive the icon's registration".to_owned());
            }
        let menu = dbus_call(&destination, MENU_PATH, &format!("{MENU_INTERFACE}.GetLayout"), &["0", "1", "[]"])?;
        if !menu.contains("Настройки KeySwitch") || !menu.contains("Выход") {
            return Err(format!("unexpected menu: {menu}"));
        }
        println!("PACKAGED_SNI_MENU_OK");
        if !wait_until(FOCUS_DEADLINE, || keyboard.is_grabbed()) {
            return Err("the input helper did not grab the keyboard".to_owned());
        }
        target.focused(&keyboard)?;
        let switch = |group: i32| control.switch_group(group).map_err(|error| format!("{error:?}"));
        // The first conversion, and an Enter held back until the word is corrected.
        let mut converted = false;
        for _ in 0..WARMUP_ATTEMPTS {
            switch(0)?;
            std::thread::sleep(INTER_CASE);
            keyboard.clear_field()?;
            wait_until(SETTLE, || target.text().is_empty());
            // The switch above protects one word: it is spent on `ok `.
            keyboard.type_keys("ok ")?;
            wait_until(SETTLE, || target.text() == "ok ");
            target.clear_activations();
            keyboard.type_keys("ghbdtn\n")?;
            if wait_until(LATE, || target.activations() == ["ok привет"] && control.current_group().ok() == Some(1)) {
                converted = true;
                break;
            }
        }
        if !converted {
            return Err(format!("conversion and deferred Enter failed: activated={:?} text={:?}", target.activations(), target.text()));
        }
        let entries = history.read(None).map_err(|error| error.to_string())?;
        if !entries.iter().any(|item| item.replacement == "привет") {
            return Err("the history misses the word".to_owned());
        }
        println!("PACKAGED_TYPING_AND_DEFERRED_ENTER_OK");
        // A frozen main process: the helper stays alive and the keys still pass, within its watchdog.
        switch(0)?;
        std::thread::sleep(INTER_CASE);
        keyboard.clear_field()?;
        wait_until(SETTLE, || target.text().is_empty());
        send_signal(child.id(), libc::SIGSTOP);
        let frozen = (|| {
            std::thread::sleep(SIGSTOP_SETTLE);
            let start = Instant::now();
            keyboard.type_keys("abc")?;
            if !wait_until(WATCHDOG_DEADLINE, || target.text() == "abc") {
                return Err(format!("the keyboard froze with the main process: {:?}", target.text()));
            }
            println!("SIGSTOP_WATCHDOG_PASSTHROUGH_OK elapsed_ms={}", start.elapsed().as_millis());
            Ok(())
        })();
        send_signal(child.id(), libc::SIGCONT);
        frozen?;
        std::thread::sleep(GRAB_SETTLE);
        let word_converts = |what: &str| -> Result<(), String> {
            keyboard.clear_field()?;
            wait_until(SETTLE, || target.text().is_empty());
            keyboard.type_keys("ok ")?;
            wait_until(SETTLE, || target.text() == "ok ");
            keyboard.type_keys("ghbdtn ")?;
            if wait_until(LATE, || target.text() == "ok привет ") { Ok(()) } else { Err(format!("{what}: {:?}", target.text())) }
        };
        word_converts("no recovery after the main process thawed")?;
        println!("SIGCONT_RECOVERY_OK");
        // The input helper dies: its grab ends with it, and the program starts a new one.
        let helpers = String::from_utf8_lossy(&Command::new("pgrep").args(["-P", &child.id().to_string(), "-f", "--", "wayland-input-helper"]).output().map_err(|error| error.to_string())?.stdout).into_owned();
        let helpers: Vec<&str> = helpers.split_whitespace().collect();
        let [helper] = helpers.as_slice() else { return Err(format!("expected one input helper, found {helpers:?}")) };
        send_signal(helper.parse().map_err(|_| "a process id")?, libc::SIGKILL);
        if !wait_until(FOCUS_DEADLINE, || !keyboard.is_grabbed()) {
            return Err("the keyboard stayed grabbed after the helper was killed".to_owned());
        }
        if !wait_until(FOCUS_DEADLINE, || keyboard.is_grabbed()) {
            return Err("the program did not start a new input helper".to_owned());
        }
        switch(0)?;
        std::thread::sleep(INTER_CASE);
        word_converts("no conversion after the helper was restarted")?;
        println!("HELPER_RESTART_OK");
        send_signal(child.id(), libc::SIGTERM);
        let deadline = Instant::now() + SHUTDOWN;
        let status = loop {
            if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
                break status;
            }
            if Instant::now() >= deadline {
                return Err("the program did not stop".to_owned());
            }
            std::thread::sleep(POLL);
        };
        if !status.success() {
            return Err(format!("the program ended with {status}"));
        }
        if !wait_until(FOCUS_DEADLINE, || !keyboard.is_grabbed()) {
            return Err("the helper left the keyboard grabbed after the program stopped".to_owned());
        }
        Ok(())
    })();
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = control.switch_group(original);
    drop(target);
    drop(keyboard);
    if outcome.is_err()
        && let Ok(log) = std::fs::read_to_string(root.join("data/keyswitch.log")) {
            println!("{}", log.lines().rev().take(40).collect::<Vec<&str>>().join("\n"));
        }
    let _ = std::fs::remove_dir_all(&root);
    outcome
}

enum Posted {
    Prompt(Option<(LearningPrompt, Option<PromptAnchor>)>),
    Rule(LearningPrompt),
}

fn screen_extents() -> Result<(i64, i64, i64, i64), String> {
    let display = gdk::Display::default().ok_or("no display")?;
    let monitors = display.monitors();
    let rectangles: Vec<gdk::Rectangle> = (0..monitors.n_items()).filter_map(|index| monitors.item(index)).filter_map(|item| item.downcast::<gdk::Monitor>().ok()).map(|monitor| monitor.geometry()).collect();
    let left = rectangles.iter().map(gdk::Rectangle::x).min().ok_or("no monitor")?;
    let top = rectangles.iter().map(gdk::Rectangle::y).min().ok_or("no monitor")?;
    let right = rectangles.iter().map(|area| area.x() + area.width()).max().ok_or("no monitor")?;
    let bottom = rectangles.iter().map(|area| area.y() + area.height()).max().ok_or("no monitor")?;
    Ok((i64::from(left), i64::from(top), i64::from(right - left), i64::from(bottom - top)))
}

fn set_locked_hint(locked: bool) -> bool {
    let Ok(session) = std::env::var("XDG_SESSION_ID") else { return false };
    let session = session.trim();
    if session.is_empty() {
        return false;
    }
    let path = format!("/org/freedesktop/login1/session/{session}");
    Command::new("sudo")
        .args(["-n", "gdbus", "call", "--system", "--dest", "org.freedesktop.login1", "--object-path", &path, "--method", "org.freedesktop.login1.Session.SetLockedHint", if locked { "true" } else { "false" }])
        .output()
        .is_ok_and(|output| output.status.success())
}

fn session_names() -> Result<String, String> {
    dbus_call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus.ListNames", &[])
}

/// The program's parts in this process: the engine, the input helper it starts, its rule windows, in the real session.
fn source(desktop: &str) -> Result<(), String> {
    unprivileged()?;
    gtk::init().map_err(|error| error.to_string())?;
    let portal_before = session_names()?.contains(DOCUMENT_PORTAL);
    let keyboard = PhysicalKeyboard::new()?;
    let mouse = PhysicalPointer::new()?;
    std::thread::sleep(GRAB_SETTLE);
    let root = std::env::temp_dir().join(format!("keyswitch-e2e-wayland-source-{}", std::process::id()));
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let settings = Arc::new(SettingsStore::new(Some(root.join("config.json"))));
    let set = |name: &str, value: Value| settings.set(name, value, false).map_err(|error| format!("{error:?}"));
    set("general.keep_history", json!(true))?;
    set("detection.early_switch", json!(false))?;
    set("diagnostics.technical_logging", json!(true))?;
    let history = Arc::new(HistoryStore::new(Some(root.join("history.jsonl")), 200));
    let learning = Arc::new(LearningStore::new(Some(root.join("learning.json")), None).map_err(|error| format!("{error:?}"))?);
    let backend = new_wayland_backend(Some(selection_factory()))?;
    let probe = backend.probe();
    println!("probe={probe:?}");
    if !probe.available || probe.session_type != "wayland" {
        return Err(format!("the Wayland probe is unavailable: {probe:?}"));
    }
    if !desktop.is_empty() && !probe.display.to_lowercase().contains(&desktop.to_lowercase()) {
        return Err(format!("expected {desktop}, got {}", probe.display));
    }
    let engine: Arc<KeySwitchEngine> = Arc::new(engine_builder(settings.clone(), history.clone(), backend.clone(), backend.process_for_window(), learning.clone())().map_err(|error| error.to_string())?);
    let modes: Arc<Mutex<Vec<String>>> = Arc::default();
    {
        let modes = modes.clone();
        engine.subscribe_corrections(Arc::new(move |plan: &ks_engine::CorrectionPlan| modes.lock().expect("the modes").push(plan.mode.to_string())));
    }
    let application = gtk::Application::builder().application_id("io.github.olegius88.KeySwitchWaylandE2E").build();
    application.register(None::<&gtk::gio::Cancellable>).map_err(|error| error.to_string())?;
    let mut target = TargetApp::new()?;
    let posted: Arc<Mutex<VecDeque<Posted>>> = Arc::default();
    let hint_inputs: Arc<Mutex<VecDeque<EngineInput<PromptAnchor>>>> = Arc::default();
    {
        let (inputs, anchors) = (hint_inputs.clone(), backend.clone());
        let anchor = Arc::new(move || anchors.input_anchor().map(|anchor| PromptAnchor { x: anchor.x, y: anchor.y, window: anchor.window }));
        subscribe(&engine.shared, Arc::new(std::sync::atomic::AtomicBool::new(true)), anchor, Arc::new(move |input| inputs.lock().expect("the queue").push_back(input)));
    }
    let restored = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let hints_origin = Instant::now();
    let hint_driver = {
        let (placer, restorer, restored) = (backend.clone(), backend.clone(), restored.clone());
        let clicked: HintClicked = std::rc::Rc::new(|_, _| {});
        // A version that has run before: no news and no first start to wait behind.
        std::fs::create_dir_all(root.join("hints")).map_err(|error| error.to_string())?;
        std::fs::write(root.join("hints/hints.json"), format!(r#"{{"last_run_version": "{}"}}"#, ks_config::logsetup::VERSION)).map_err(|error| error.to_string())?;
        HintDriver::new(
            Hints::start(settings.clone(), &root.join("hints"), RUNNING_PLATFORM, &Moment::now(hints_origin, 0)),
            hints_origin,
            HintScreen::new(
                &application,
                HintCalls {
                    position: Box::new(move |title, window, x, y| {
                        assert_eq!(title, HINT_WINDOW_TITLE);
                        placer.position_window(title, window, i64::from(x), i64::from(y))
                    }),
                    restore: Box::new(move |window| {
                        restored.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        restorer.restore_window(window)
                    }),
                    clicked,
                    open_page: Box::new(|_| {}),
                    hide_card: Box::new(|_| {}),
                },
            ),
        )
    };
    hint_driver.engine_built(engine.shared.clone());
    {
        let (posted, anchors) = (posted.clone(), backend.clone());
        engine.subscribe_learning_prompts(Arc::new(move |prompt: Option<&LearningPrompt>| {
            let item = prompt.cloned().map(|prompt| {
                let anchor = anchors.input_anchor().map(|anchor| PromptAnchor { x: anchor.x, y: anchor.y, window: anchor.window });
                (prompt, anchor)
            });
            posted.lock().expect("the queue").push_back(Posted::Prompt(item));
        }));
    }
    {
        let posted = posted.clone();
        engine.subscribe_rule_requests(Arc::new(move |prompt: &LearningPrompt| posted.lock().expect("the queue").push_back(Posted::Rule(prompt.clone()))));
    }
    let prompt_window = {
        let (confirm, dismiss) = (engine.clone(), engine.clone());
        let (position, restore) = (backend.clone(), backend.clone());
        LearningPromptWindow::new(
            &application,
            PromptCallbacks {
                confirm: Box::new(move |prompt| confirm.confirm_learning_prompt(Some(prompt))),
                dismiss: Box::new(move |prompt| dismiss.dismiss_learning_prompt(Some(prompt))),
                position: Box::new(move |window, x, y| position.position_window(LEARNING_PROMPT_WINDOW_TITLE, window, i64::from(x), i64::from(y))),
                restore: Box::new(move |window| restore.restore_window(window)),
            },
        )
    };
    let rule_windows: std::rc::Rc<std::cell::RefCell<Vec<RuleEditorWindow>>> = std::rc::Rc::default();
    let offer_window: std::rc::Rc<std::cell::Cell<Option<i64>>> = std::rc::Rc::default();
    let drain = || loop {
        let Some(item) = posted.lock().expect("the queue").pop_front() else { break };
        match item {
            Posted::Prompt(None) => prompt_window.hide_prompt(),
            Posted::Prompt(Some((prompt, anchor))) => prompt_window.show_prompt(prompt, anchor),
            Posted::Rule(prompt) => {
                let (accepting, focusing, restoring) = (engine.clone(), engine.clone(), backend.clone());
                let window = offer_window.get();
                let editor = RuleEditorWindow::new(
                    &application,
                    draft_from_prompt(&prompt),
                    RuleCallbacks {
                        accept: Box::new(move |draft: &RuleDraft| {
                            let built = build_rule(draft).map_err(|error| error.to_string());
                            match built.and_then(|rule| accepting.add_learning_rule(rule, None, &draft.application).map_err(|error| error.to_string())) {
                                Ok(_) => String::new(),
                                Err(error) => error,
                            }
                        }),
                        closed: Box::new(move || {
                            restoring.restore_window(window);
                        }),
                        focused: Box::new(move |opened| focusing.set_rule_editor_open(opened)),
                    },
                );
                editor.present();
                rule_windows.borrow_mut().push(editor);
            }
        }
    };
    let drain_hints = || {
        loop {
            let Some(input) = hint_inputs.lock().expect("the queue").pop_front() else { break };
            hint_driver.input(input);
        }
    };
    let wait = |limit: Duration, condition: &mut dyn FnMut() -> bool| -> bool {
        pump_until(limit.as_millis().low::<u64>(), || {
            drain();
            drain_hints();
            condition()
        })
    };
    let settle = |limit: Duration| {
        wait(limit, &mut || false);
    };
    target.command("present", "")?;
    target.command("focus", "")?;
    settle(GRAB_SETTLE);
    let original = backend.current_group().map_err(|error| format!("{error:?}"))?;
    set("detection.respect_manual_layout", json!(false))?;
    engine.start().map_err(|error| format!("{error:?}"))?;
    let outcome = (|| -> Result<(), String> {
        if !wait(FOCUS_DEADLINE, &mut || keyboard.is_grabbed()) {
            return Err("the input helper did not grab the keyboard".to_owned());
        }
        println!("EVIOCGRAB_ACQUIRED_OK");
        let switch = |group: i32| backend.switch_group(group).map_err(|error| format!("{error:?}"));
        let group = || backend.current_group().unwrap_or(-1);
        switch(0)?;
        target.focused(&keyboard)?;
        if target.caps_lock() {
            keyboard.tap(KEY_CAPSLOCK)?;
            if !wait(CAPS_DEADLINE, &mut || !target.caps_lock()) {
                return Err("Caps Lock left on by an earlier run could not be turned off".to_owned());
            }
        }
        history.clear().map_err(|error| error.to_string())?;
        set("detection.respect_manual_layout", json!(true))?;
        let focus = backend.focused_window().ok_or("no focused window")?;
        println!("focused_window={focus:?} active_app={:?}", backend.active_application());
        if focus.window <= 0 || focus.own {
            return Err(format!("the compositor's adapter did not report the target's window: {focus:?}"));
        }
        let clear = |target: &TargetApp| -> Result<(), String> {
            keyboard.clear_field()?;
            wait(SETTLE, &mut || target.text().is_empty());
            Ok(())
        };
        // The hint alone, for a quick check of the card on one desktop; the full run goes through everything.
        let only_hint = std::env::var_os("KS_E2E_WAYLAND_HINT_ONLY").is_some();
        // 1. The typing cases of X11.
        for case in CASES.iter().filter(|_| !only_hint) {
            switch(case.1)?;
            target.command("focus", "")?;
            clear(&target)?;
            keyboard.type_keys(case.2)?;
            settle(Duration::from_millis(case.5));
            wait(Duration::from_millis(GRACE_MS), &mut || target.text() == case.3 && group() == case.4);
            println!("case={:?} text={:?} group={} expected={:?}/{}", case.0, target.text(), group(), case.3, case.4);
            if target.text() != case.3 || group() != case.4 {
                return Err(format!("case {:?} failed (last error {:?})", case.0, engine.snapshot().last_error));
            }
            settle(INTER_CASE);
        }
        println!("TYPING_CASES_E2E_OK");
        // A hint over the field. A desktop may give a new window the keyboard: it comes back to the field by itself, and the
        // keys, the correction and Enter are the field's. With KS_E2E_WAYLAND_NO_HINT the same keys with no hint at all, to
        // compare a failure against.
        let show_hint = std::env::var_os("KS_E2E_WAYLAND_NO_HINT").is_none();
        if !show_hint {
            set("general.hints", json!(false))?;
            hint_driver.settings_changed();
        }
        switch(0)?;
        target.command("present", "")?;
        target.command("focus", "")?;
        settle(GRAB_SETTLE);
        clear(&target)?;
        keyboard.type_keys("ghbdtn ")?;
        if !wait(LATE, &mut || target.text() == "привет " && group() == 1) {
            return Err(format!("the correction that brings the hint: {:?} group {} engine {:?}", target.text(), group(), engine.snapshot().last_action));
        }
        let field = backend.focused_window().ok_or("no focused window under the hint")?;
        if show_hint && !wait(HINT_APPEARS, &mut || hint_driver.card_id().is_some()) {
            return Err("the hint did not appear after the correction".to_owned());
        }
        if !show_hint {
            // The time a hint takes to appear, spent with no hint.
            settle(Duration::from_millis(1500));
        }
        let appeared = Instant::now();
        let gave_back = wait(HINT_GIVES_KEYBOARD_BACK, &mut || backend.focused_window().is_some_and(|focus| focus.window == field.window));
        println!("hint: the keyboard back with the field after {} ms, restored {} times, window {:?}", appeared.elapsed().as_millis(), restored.load(std::sync::atomic::Ordering::Relaxed), backend.focused_window());
        if !gave_back {
            return Err(format!("the hint kept the keyboard: {:?}", backend.focused_window()));
        }
        settle(INTER_CASE);
        keyboard.type_keys("hello ")?;
        if !wait(LATE, &mut || target.text() == "привет hello " && group() == 0) {
            return Err(format!("typing under the hint: {:?} group {} engine {:?}", target.text(), group(), engine.snapshot().last_action));
        }
        target.clear_activations();
        keyboard.tap(KEY_ENTER)?;
        if !wait(LATE, &mut || target.activations().len() == 1) || engine.shared.prompt_key_deadline() != 0.0 {
            return Err(format!("Enter under the hint: {:?} deadline {}", target.activations(), engine.shared.prompt_key_deadline()));
        }
        println!("HINT_KEEPS_THE_KEYBOARD_E2E_OK");
        // Long enough for a look at the desktop, when somebody asks for one.
        if let Some(hold) = std::env::var("KS_E2E_WAYLAND_HINT_HOLD_MS").ok().and_then(|text| text.parse::<u64>().ok()) {
            settle(Duration::from_millis(hold));
        }
        hint_driver.close();
        if only_hint {
            return Ok(());
        }
        // The checks of single features do without the protection of a manual switch.
        set("detection.respect_manual_layout", json!(false))?;
        // 2. Enter is withheld until the replacement is done, then delivered once.
        switch(0)?;
        clear(&target)?;
        target.clear_activations();
        keyboard.type_keys("ghbdtn\n")?;
        if !wait(LATE, &mut || target.activations().len() == 1 && group() == 1) {
            return Err(format!("deferred Enter: {:?} text={:?}", target.activations(), target.text()));
        }
        if target.activations() != ["привет"] {
            return Err(format!("Enter fired before the replacement or more than once: {:?}", target.activations()));
        }
        println!("DEFERRED_ENTER_E2E_OK");
        // 3. Two lines in a burst.
        switch(0)?;
        clear(&target)?;
        target.clear_activations();
        keyboard.type_keys("ghbdtn\ngjrf\n")?;
        if !wait(LATE, &mut || target.activations() == ["привет", "приветпока"]) {
            return Err(format!("rapid Enter burst: {:?}", target.activations()));
        }
        println!("RAPID_DEFERRED_BURST_E2E_OK");
        // 4. The hotkeys are swallowed: undo without a typed z, pause and resume.
        switch(0)?;
        clear(&target)?;
        keyboard.type_keys("ghbdtn ")?;
        if !wait(LATE, &mut || target.text() == "привет ") {
            return Err(format!("before the undo hotkey: {:?}", target.text()));
        }
        keyboard.chord(&[KEY_LEFTCTRL, KEY_LEFTALT], KEY_Z)?;
        if !wait(LATE, &mut || target.text() == "ghbdtn ") {
            return Err(format!("Ctrl+Alt+Z failed or leaked a character: {:?}", target.text()));
        }
        keyboard.chord(&[KEY_LEFTCTRL, KEY_LEFTALT], KEY_P)?;
        if !wait(CAPS_DEADLINE, &mut || !engine.snapshot().enabled) {
            return Err("Ctrl+Alt+P did not pause".to_owned());
        }
        clear(&target)?;
        keyboard.type_keys("ghbdtn ")?;
        settle(SETTLE);
        if target.text() != "ghbdtn " {
            return Err(format!("the paused engine converted text: {:?}", target.text()));
        }
        keyboard.chord(&[KEY_LEFTCTRL, KEY_LEFTALT], KEY_P)?;
        if !wait(CAPS_DEADLINE, &mut || engine.snapshot().enabled) {
            return Err("Ctrl+Alt+P did not resume".to_owned());
        }
        println!("HOTKEY_SWALLOW_E2E_OK");
        // 5. A pointer click forgets the word typed so far.
        let center = |target: &mut TargetApp, widget: &str| -> Result<(i64, i64), String> {
            let origin = backend.window_origin(focus.window);
            let (ok, x, y, width, height) = target.bounds(widget)?;
            let Some((left, top)) = origin.filter(|_| ok) else { return Err(format!("cannot place the click: origin {origin:?}, bounds known {ok}")) };
            Ok((left + (x + width / 2.0).to::<i64>(), top + (y + height / 2.0).to::<i64>()))
        };
        switch(0)?;
        clear(&target)?;
        keyboard.type_keys("ghb")?;
        wait(SETTLE, &mut || target.text() == "ghb");
        let point = center(&mut target, "entry")?;
        mouse.click_at(point, screen_extents()?)?;
        settle(INTER_CASE);
        let after = backend.focused_window();
        if after.as_ref().map(|focused| focused.window) != Some(focus.window) {
            return Err(format!("the click left the test window: {after:?}"));
        }
        keyboard.type_keys("dtn ")?;
        settle(SETTLE);
        if target.text().starts_with("при") {
            return Err(format!("the click did not invalidate the word before it: {:?}", target.text()));
        }
        println!("POINTER_BUFFER_RESET_E2E_OK");
        // 6. Caps Lock, and the light on the physical keyboard.
        switch(0)?;
        clear(&target)?;
        keyboard.drain_leds();
        keyboard.tap(KEY_CAPSLOCK)?;
        let synced = wait(CAPS_DEADLINE, &mut || keyboard.drain_leds().get(&LED_CAPSL) == Some(&1));
        keyboard.type_keys("ghbdtn ")?;
        if !wait(LATE, &mut || target.text() == "ПРИВЕТ ") {
            return Err(format!("Caps Lock conversion: {:?}", target.text()));
        }
        keyboard.tap(KEY_CAPSLOCK)?;
        wait(CAPS_DEADLINE, &mut || keyboard.drain_leds().get(&LED_CAPSL) == Some(&0));
        if !synced {
            return Err(format!("the compositor's Caps Lock light did not reach the keyboard: {:?}", keyboard.drain_leds()));
        }
        println!("CAPSLOCK_AND_LED_SYNC_E2E_OK");
        // 7. A keyboard plugged in while running.
        switch(0)?;
        clear(&target)?;
        {
            let plugged = PhysicalKeyboard::named(HOTPLUG_NAME, HOTPLUG_PRODUCT_ID)?;
            if !wait(FOCUS_DEADLINE, &mut || plugged.is_grabbed()) {
                return Err("the plugged keyboard was not grabbed".to_owned());
            }
            plugged.type_keys("ghbdtn ")?;
            if !wait(LATE, &mut || target.text() == "привет ") {
                return Err(format!("the plugged keyboard's word did not convert: {:?}", target.text()));
            }
        }
        settle(INTER_CASE);
        println!("HOTPLUG_KEYBOARD_E2E_OK");
        // 8. A password field stays as typed. The focus moves as a user moves it, with a click: the engine leaves an
        // application's window alone after its password field until a click, Tab, Enter or another window.
        set("detection.context_policy", json!("assist"))?;
        switch(0)?;
        target.command_on("password", "set_text", "")?;
        let point = center(&mut target, "password")?;
        mouse.click_at(point, screen_extents()?)?;
        settle(SETTLE);
        keyboard.type_keys("ghbdtn ")?;
        settle(LATE);
        if target.password() != "ghbdtn " || group() != 0 {
            return Err(format!("the password field's text was converted, group {}", group()));
        }
        println!("ATSPI_PASSWORD_PROTECTION_E2E_OK");
        let point = center(&mut target, "entry")?;
        mouse.click_at(point, screen_extents()?)?;
        settle(INTER_CASE);
        // 9. Double Pause, the offer, Enter, the rule window, OK, the keyboard back.
        set("detection.respect_manual_layout", json!(true))?;
        switch(0)?;
        clear(&target)?;
        // The switch protects one word: it is spent on `ok `.
        keyboard.type_keys("ok ")?;
        wait(SETTLE, &mut || target.text() == "ok ");
        clear(&target)?;
        keyboard.type_keys("hello")?;
        wait(SETTLE, &mut || target.text() == "hello");
        keyboard.tap(KEY_PAUSE)?;
        keyboard.tap(KEY_PAUSE)?;
        if !wait(LATE, &mut || target.text() == "руддщ" && group() == 1 && engine.learning_prompt().is_some() && prompt_window.is_visible()) {
            return Err(format!("double Pause: text={:?} prompt={:?} visible={}", target.text(), engine.learning_prompt(), prompt_window.is_visible()));
        }
        offer_window.set(prompt_window.anchor_window());
        keyboard.tap(KEY_ENTER)?;
        if !wait(CAPS_DEADLINE, &mut || rule_windows.borrow().first().is_some_and(|window| window.current_draft().pattern == "hello")) {
            return Err("Enter on the offer did not open the rule window".to_owned());
        }
        rule_windows.borrow()[0].confirm();
        if !wait(CAPS_DEADLINE, &mut || engine.learning().forced_target(0, "hello") == Some(1)) {
            return Err("the rule window's OK stored no rule".to_owned());
        }
        switch(0)?;
        target.command("focus", "")?;
        settle(INTER_CASE);
        clear(&target)?;
        // The first word after a manual switch is protected; the second takes the learned rule.
        keyboard.type_keys("hello ")?;
        wait(SETTLE, &mut || target.text() == "hello ");
        clear(&target)?;
        keyboard.type_keys("hello ")?;
        if !wait(LATE, &mut || target.text() == "руддщ " && group() == 1) {
            return Err(format!("the learned rule was not applied: {:?}", target.text()));
        }
        println!("LEARNING_PROMPT_AND_RULE_EDITOR_E2E_OK");
        // 10. The menu's layout switch, then switching on a prefix.
        if !engine.select_alternate_group() {
            return Err("select_alternate_group failed".to_owned());
        }
        if !wait(CAPS_DEADLINE, &mut || group() == 0) {
            return Err("the menu's selection did not switch the layout".to_owned());
        }
        set("detection.early_switch", json!(true))?;
        target.command("focus", "")?;
        clear(&target)?;
        keyboard.type_keys("ok ")?;
        wait(SETTLE, &mut || target.text() == "ok ");
        modes.lock().expect("the modes").clear();
        for character in "ghbd".chars() {
            keyboard.type_keys(&character.to_string())?;
            settle(SLOW_KEY);
        }
        if !wait(LATE, &mut || target.text() == "ok прив" && group() == 1) {
            return Err(format!("prefix switch mid-word: {:?}", target.text()));
        }
        for character in "tn ".chars() {
            keyboard.type_keys(&character.to_string())?;
            settle(SLOW_KEY);
        }
        if !wait(LATE, &mut || target.text() == "ok привет " && modes.lock().expect("the modes").iter().any(|mode| mode == "early")) {
            return Err(format!("prefix completion: {:?} modes={:?}", target.text(), modes.lock().expect("the modes")));
        }
        set("detection.early_switch", json!(false))?;
        println!("EARLY_PREFIX_SWITCH_E2E_OK");
        // 11. A locked session: the grab is let go, and taken again after the unlock.
        set("detection.respect_manual_layout", json!(false))?;
        if set_locked_hint(true) {
            let locked = (|| -> Result<(), String> {
                if !wait(FOCUS_DEADLINE, &mut || !keyboard.is_grabbed()) {
                    return Err("the grab was not released when the session locked".to_owned());
                }
                switch(0)?;
                clear(&target)?;
                keyboard.type_keys("ghbdtn ")?;
                settle(LOCK_SETTLE);
                if target.text() != "ghbdtn " {
                    return Err(format!("keys were intercepted while locked: {:?}", target.text()));
                }
                Ok(())
            })();
            set_locked_hint(false);
            locked?;
            if !wait(FOCUS_DEADLINE, &mut || keyboard.is_grabbed()) {
                return Err("the grab was not taken again after the unlock".to_owned());
            }
            clear(&target)?;
            keyboard.type_keys("ghbdtn ")?;
            if !wait(LATE, &mut || target.text() == "привет ") {
                return Err(format!("no conversion after the unlock: {:?}", target.text()));
            }
            println!("SCREEN_LOCK_UNGRAB_REGRAB_E2E_OK");
        }
        // 12. The program activated no document portal.
        if !portal_before && session_names()?.contains(DOCUMENT_PORTAL) {
            return Err("the document portal was activated".to_owned());
        }
        Ok(())
    })();
    set_locked_hint(false);
    let _ = backend.switch_group(original);
    engine.stop();
    prompt_window.destroy();
    drop(target);
    let _ = std::fs::remove_dir_all(&root);
    outcome
}

fn main() {
    // The engine starts the input helper from the program it runs in, and here that is this test.
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.iter().any(|argument| argument == "--wayland-input-helper") {
        std::process::exit(ks_linux::linux_app::main(&arguments));
    }
    if std::env::var_os(TARGET_ROLE).is_some() {
        target::main();
        return;
    }
    let desktop = std::env::var("KS_E2E_DESKTOP").unwrap_or_default();
    if std::env::var_os("KS_E2E_WAYLAND_SOURCE").is_some() {
        match source(&desktop) {
            Ok(()) => println!("WAYLAND_SOURCE_E2E_OK"),
            Err(error) => {
                println!("WAYLAND_SOURCE_E2E_FAILED: {error}");
                std::process::exit(1);
            }
        }
        return;
    }
    let Some(binary) = std::env::var_os("KS_E2E_WAYLAND_BINARY") else {
        println!("KS_E2E_WAYLAND_BINARY is not set: no Wayland session to drive, skipped");
        return;
    };
    match packaged(Path::new(&binary), &desktop) {
        Ok(()) => println!("WAYLAND_PACKAGED_E2E_OK"),
        Err(error) => {
            println!("WAYLAND_PACKAGED_E2E_FAILED: {error}");
            std::process::exit(1);
        }
    }
}
