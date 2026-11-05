//! What the end-to-end tests share: physical keys through XTEST, a GTK entry to type into, the main loop run from straight
//! code, and a watcher for the panel icon.
#![allow(dead_code)]

use ks_cast::To as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gtk::glib;
use gtk::prelude::{EditableExt as _, GtkWindowExt as _, WidgetExt as _};
use ks_linux::x11_backend::X11Api;
use ks_linux::x11_native;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt as _, KEY_PRESS_EVENT, KEY_RELEASE_EVENT};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

pub(crate) const INTER_CASE: u64 = 200;
pub(crate) const SETTLE: u64 = 900;
pub(crate) const GRACE: u64 = 4000;
pub(crate) const POLL: u64 = 100;
pub(crate) const LEARNING_CONFIRMATION: u64 = 500;
pub(crate) const IDLE_PAUSE: u64 = 2300;
pub(crate) const CONTEXT_RESOLUTION: u64 = 1200;
pub(crate) const MENU_LAYOUT: u64 = 300;
pub(crate) const SLOW_CHARACTER: u64 = 150;
pub(crate) const EARLY_SETUP: u64 = 600;
pub(crate) const STARTUP: u64 = 450;
pub(crate) const KEY_PRESS_DELAY_MS: u32 = 18;
pub(crate) const KEY_RELEASE_DELAY_MS: u32 = 8;
pub(crate) const WINDOW_WIDTH: i32 = 520;
pub(crate) const WINDOW_HEIGHT: i32 = 120;
pub(crate) const ENTRY_MARGIN: i32 = 24;
pub(crate) const PAUSE_KEYSYM: u32 = 0xff13;
pub(crate) const RETURN_KEYSYM: u32 = 0xff0d;
pub(crate) const CONTROL_L_KEYSYM: u32 = 0xffe3;
pub(crate) const BACKSPACE_KEYSYM: u32 = 0xff08;

/// Physical US-layout keys, independent of any input method.
pub(crate) struct Typer {
    connection: RustConnection,
    root: u32,
    api: Arc<dyn X11Api>,
}

impl Typer {
    pub(crate) fn new() -> Result<Typer, String> {
        let (connection, screen) = x11rb::connect(None).map_err(|error| format!("cannot open DISPLAY: {error}"))?;
        let root = connection.setup().roots[screen].root;
        let api = x11_native::connect().map_err(|error| format!("{error:?}"))?;
        Ok(Typer { connection, root, api })
    }

    fn event(&self, kind: u8, keycode: u8, delay: u32) -> Result<(), String> {
        self.connection.xtest_fake_input(kind, keycode, delay, self.root, 0, 0, 0).map_err(|error| error.to_string())?;
        Ok(())
    }

    fn sync(&self) -> Result<(), String> {
        self.connection.flush().map_err(|error| error.to_string())?;
        self.connection.get_input_focus().map_err(|error| error.to_string())?.reply().map_err(|error| error.to_string())?;
        Ok(())
    }

    fn keycode(&self, keysym: u32) -> Result<u8, String> {
        match self.api.keycode_for_keysym(keysym) {
            0 => Err(format!("no X11 keycode for keysym {keysym:#x}")),
            keycode => Ok(keycode),
        }
    }

    pub(crate) fn tap_keysym(&self, keysym: u32) -> Result<(), String> {
        let keycode = self.keycode(keysym)?;
        self.event(KEY_PRESS_EVENT, keycode, KEY_PRESS_DELAY_MS)?;
        self.event(KEY_RELEASE_EVENT, keycode, KEY_RELEASE_DELAY_MS)?;
        self.sync()
    }

    pub(crate) fn type_keys(&self, physical: &str) -> Result<(), String> {
        for character in physical.chars() {
            let keycode = self.keycode(character.to::<u32>())?;
            self.event(KEY_PRESS_EVENT, keycode, KEY_PRESS_DELAY_MS)?;
            self.event(KEY_RELEASE_EVENT, keycode, KEY_RELEASE_DELAY_MS)?;
        }
        self.sync()
    }

    /// Through real Ctrl+A and Backspace, so that the observer sees the edit too.
    pub(crate) fn clear_field(&self) -> Result<(), String> {
        let (control, letter) = (self.keycode(CONTROL_L_KEYSYM)?, self.keycode(('a').to::<u32>())?);
        for (kind, keycode) in [(KEY_PRESS_EVENT, control), (KEY_PRESS_EVENT, letter), (KEY_RELEASE_EVENT, letter), (KEY_RELEASE_EVENT, control)] {
            self.event(kind, keycode, KEY_RELEASE_DELAY_MS)?;
        }
        self.tap_keysym(BACKSPACE_KEYSYM)
    }

    pub(crate) fn current_group(&self) -> i32 {
        self.api.xkb_state().map_or(-1, |state| i32::from(state.group))
    }

    pub(crate) fn switch_group(&self, group: i32) -> Result<(), String> {
        if !self.api.lock_group(group.low::<u8>()) {
            return Err(format!("cannot switch the XKB group to {group}"));
        }
        self.api.sync();
        Ok(())
    }
}

/// Runs the main loop for this long.
pub(crate) fn pump(milliseconds: u64) {
    pump_until(milliseconds, || false);
}

/// Runs the main loop until the condition holds or the time is out; whether it held.
pub(crate) fn pump_until(milliseconds: u64, mut condition: impl FnMut() -> bool) -> bool {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_millis(milliseconds);
    loop {
        while context.iteration(false) {}
        if condition() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

pub(crate) fn entry_window(title: &str) -> (gtk::Window, gtk::Entry) {
    let window = gtk::Window::builder().title(title).default_width(WINDOW_WIDTH).default_height(WINDOW_HEIGHT).build();
    let entry = gtk::Entry::builder().placeholder_text("E2E input").build();
    entry.set_margin_top(ENTRY_MARGIN);
    entry.set_margin_bottom(ENTRY_MARGIN);
    entry.set_margin_start(ENTRY_MARGIN);
    entry.set_margin_end(ENTRY_MARGIN);
    window.set_child(Some(&entry));
    window.present();
    entry.grab_focus();
    (window, entry)
}

/// One typed case: the layout selected before, the keys, the text and layout expected, the earliest moment to judge them.
pub(crate) struct Case(pub &'static str, pub i32, pub &'static str, pub &'static str, pub i32, pub u64);

/// The settle delay is the earliest moment the text may be judged (a kept word must still be as typed then); a late
/// correction is polled for until the deadline.
pub(crate) fn judge(entry: &gtk::Entry, group: &dyn Fn() -> i32, case: &Case, grace: u64) -> Result<(), String> {
    pump(case.5);
    let matches = || entry.text() == case.3 && group() == case.4;
    let reached = pump_until(grace, matches);
    println!("case={:?} text={:?} group={} expected={:?}/{}", case.0, entry.text().as_str(), group(), case.3, case.4);
    if reached { Ok(()) } else { Err(format!("wrong correction in {:?}", case.0)) }
}

/// The minimal host a panel icon registers with.
pub(crate) struct Watcher {
    pub registered: Arc<Mutex<Option<String>>>,
    _connection: zbus::blocking::Connection,
}

struct WatcherObject {
    registered: Arc<Mutex<Option<String>>>,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl WatcherObject {
    #[zbus(name = "RegisterStatusNotifierItem")]
    fn register_status_notifier_item(&self, service: String) {
        *self.registered.lock().expect("the watcher's state") = Some(service);
    }
}

impl Watcher {
    pub(crate) fn new() -> Result<Watcher, String> {
        let registered = Arc::new(Mutex::new(None));
        let object = WatcherObject { registered: registered.clone() };
        let connection = zbus::blocking::connection::Builder::session()
            .and_then(|builder| builder.name("org.kde.StatusNotifierWatcher"))
            .and_then(|builder| builder.serve_at("/StatusNotifierWatcher", object))
            .and_then(zbus::blocking::connection::Builder::build)
            .map_err(|error| error.to_string())?;
        Ok(Watcher { registered, _connection: connection })
    }

    pub(crate) fn service(&self) -> Option<String> {
        self.registered.lock().expect("the watcher's state").clone()
    }
}

/// `gdbus call`: the answer as text.
pub(crate) fn dbus_call(destination: &str, path: &str, method: &str, arguments: &[&str]) -> Result<String, String> {
    let output = std::process::Command::new("gdbus")
        .args(["call", "--session", "--dest", destination, "--object-path", path, "--method", method])
        .args(arguments)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
