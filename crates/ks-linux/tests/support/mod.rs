//! What the end-to-end tests share: physical keys through XTEST, a GTK entry to type into, the main loop run from straight
//! code, and a watcher for the panel icon.
#![allow(dead_code, reason = "each test program uses a different part of this module")]

use ks_cast::To as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gtk::glib;
use gtk::prelude::{EditableExt as _, GtkWindowExt as _, WidgetExt as _};
use ks_linux::cinnamon_native::CinnamonLayouts;
use ks_linux::x11_backend::{DesktopLayouts, X11Api, is_cinnamon_desktop};
use ks_linux::x11_native;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT, ConnectionExt as _, KEY_PRESS_EVENT, KEY_RELEASE_EVENT, MOTION_NOTIFY_EVENT};
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
pub(crate) const CONTROL_R_KEYSYM: u32 = 0xffe4;
pub(crate) const BACKSPACE_KEYSYM: u32 = 0xff08;
pub(crate) const SHIFT_L_KEYSYM: u32 = 0xffe1;
pub(crate) const SHIFT_R_KEYSYM: u32 = 0xffe2;
pub(crate) const ESCAPE_KEYSYM: u32 = 0xff1b;
const LEFT_BUTTON: u8 = 1;
/// The window manager the keyboard is tried against, and how long it takes to start.
pub(crate) const WINDOW_MANAGER: &str = "xfwm4";
const WINDOW_MANAGER_STARTS: u64 = 10_000;
/// A group asked of a desktop that keeps it (Cinnamon) is waited for this long. Cinnamon took more than a second on a busy
/// machine; a group locked past it meanwhile is put back at once, and the keys typed then went in the old group.
const DESKTOP_GROUP_SHOWN: u64 = 5000;
const STATUS_NOTIFIER_WATCHER: &str = "org.kde.StatusNotifierWatcher";
const STATUS_NOTIFIER_WATCHER_PATH: &str = "/StatusNotifierWatcher";

/// Physical US-layout keys, independent of any input method.
pub(crate) struct Typer {
    connection: RustConnection,
    root: u32,
    api: Arc<dyn X11Api>,
    /// On a desktop that keeps the layout group itself (Cinnamon), the group is asked of it, as the program asks it.
    desktop: Option<CinnamonLayouts>,
}

impl Typer {
    pub(crate) fn new() -> Result<Typer, String> {
        let (connection, screen) = x11rb::connect(None).map_err(|error| format!("cannot open DISPLAY: {error}"))?;
        let root = connection.setup().roots[screen].root;
        let api = x11_native::connect().map_err(|error| format!("{error:?}"))?;
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktop| is_cinnamon_desktop(&desktop)).then(CinnamonLayouts::default);
        Ok(Typer { connection, root, api, desktop })
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

    /// The key goes down and stays down, or comes up.
    pub(crate) fn hold_keysym(&self, keysym: u32, pressed: bool) -> Result<(), String> {
        let keycode = self.keycode(keysym)?;
        self.event(if pressed { KEY_PRESS_EVENT } else { KEY_RELEASE_EVENT }, keycode, KEY_PRESS_DELAY_MS)?;
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

    /// A press and a release of the left button at a point of the screen.
    pub(crate) fn click_at(&self, x: i32, y: i32) -> Result<(), String> {
        let (x, y) = (x.low::<i16>(), y.low::<i16>());
        self.connection.xtest_fake_input(MOTION_NOTIFY_EVENT, 0, 0, self.root, x, y, 0).map_err(|error| error.to_string())?;
        for (kind, delay) in [(BUTTON_PRESS_EVENT, KEY_PRESS_DELAY_MS), (BUTTON_RELEASE_EVENT, KEY_PRESS_DELAY_MS)] {
            self.connection.xtest_fake_input(kind, LEFT_BUTTON, delay, self.root, 0, 0, 0).map_err(|error| error.to_string())?;
        }
        self.sync()
    }

    /// The window that has the keyboard now, by X11 itself: the one a window manager or a call gave it to.
    pub(crate) fn input_focus(&self) -> Result<u32, String> {
        Ok(self.connection.get_input_focus().map_err(|error| error.to_string())?.reply().map_err(|error| error.to_string())?.focus)
    }

    /// The top left corner of a window on the screen.
    pub(crate) fn window_origin(&self, window: u32) -> Result<(i32, i32), String> {
        let reply = self.connection.translate_coordinates(window, self.root, 0, 0).map_err(|error| error.to_string())?.reply().map_err(|error| error.to_string())?;
        Ok((i32::from(reply.dst_x), i32::from(reply.dst_y)))
    }

    /// Whether a window manager has said it runs on this screen (`_NET_SUPPORTING_WM_CHECK` on the root window).
    pub(crate) fn window_manager_runs(&self) -> Result<bool, String> {
        let atom = self.connection.intern_atom(false, b"_NET_SUPPORTING_WM_CHECK").map_err(|error| error.to_string())?.reply().map_err(|error| error.to_string())?.atom;
        let property = self.connection.get_property(false, self.root, atom, AtomEnum::WINDOW, 0, 1).map_err(|error| error.to_string())?.reply().map_err(|error| error.to_string())?;
        Ok(property.value_len != 0)
    }

    pub(crate) fn current_group(&self) -> i32 {
        self.api.xkb_state().map_or(-1, |state| i32::from(state.group))
    }

    pub(crate) fn switch_group(&self, group: i32) -> Result<(), String> {
        if self.desktop.as_ref().is_some_and(|desktop| desktop.activate(group.low::<u8>())) {
            // The desktop took the request: it is waited for, as locking the group past it would only be put back.
            return if pump_until(DESKTOP_GROUP_SHOWN, || self.current_group() == group) {
                Ok(())
            } else {
                Err(format!("the desktop did not switch to the XKB group {group}"))
            };
        }
        if !self.api.lock_group(group.low::<u8>()) {
            return Err(format!("cannot switch the XKB group to {group}"));
        }
        self.api.sync();
        Ok(())
    }
}

/// A window manager on the private screen, and the moment it says it runs.
pub(crate) fn start_window_manager(typer: &Typer) -> Result<std::process::Child, String> {
    let child = std::process::Command::new(WINDOW_MANAGER)
        .args(["--compositor=off", "--sm-client-disable"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| format!("{WINDOW_MANAGER} is needed for the test that a hint keeps its hands off the keyboard: {error}"))?;
    if !pump_until(WINDOW_MANAGER_STARTS, || typer.window_manager_runs().unwrap_or(false)) {
        return Err(format!("{WINDOW_MANAGER} did not start"));
    }
    Ok(child)
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

/// A click in the middle of the window, which gives it the keyboard where a window manager keeps a new window of a program
/// in the background from taking it (Cinnamon's Muffin).
pub(crate) fn click_into(typer: &Typer, window: &gtk::Window) -> Result<(), String> {
    use gtk::prelude::{Cast as _, NativeExt as _};
    let xid = window
        .surface()
        .and_then(|surface| surface.downcast::<gdk_x11::X11Surface>().ok())
        .and_then(|surface| u32::try_from(surface.xid()).ok())
        .ok_or("the window has no X11 window")?;
    let (x, y) = typer.window_origin(xid)?;
    typer.click_at(x + WINDOW_WIDTH / 2, y + WINDOW_HEIGHT / 2)
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

/// The host a panel icon registers with: a minimal one of the test's own on a private bus, or the desktop's (Cinnamon's
/// `xapp-sn-watcher`) when a desktop session runs one already.
pub(crate) struct Watcher {
    pub registered: Arc<Mutex<Option<String>>>,
    connection: Option<zbus::blocking::Connection>,
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
        let owned = dbus_call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus.NameHasOwner", &[STATUS_NOTIFIER_WATCHER])?;
        if owned.contains("true") {
            return Ok(Watcher { registered, connection: None });
        }
        let object = WatcherObject { registered: registered.clone() };
        let connection = zbus::blocking::connection::Builder::session()
            .and_then(|builder| builder.name(STATUS_NOTIFIER_WATCHER))
            .and_then(|builder| builder.serve_at(STATUS_NOTIFIER_WATCHER_PATH, object))
            .and_then(zbus::blocking::connection::Builder::build)
            .map_err(|error| error.to_string())?;
        Ok(Watcher { registered, connection: Some(connection) })
    }

    /// What the icon of `destination` registered: the object path it gave the test's own host, or the path the desktop's
    /// host lists after the icon's connection.
    pub(crate) fn service(&self, destination: &str) -> Option<String> {
        if self.connection.is_some() {
            return self.registered.lock().expect("the watcher's state").clone();
        }
        let owner = dbus_call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus.GetNameOwner", &[destination]).ok()?;
        let owner = owner.split('\'').nth(1)?.to_owned();
        let items = dbus_call(
            STATUS_NOTIFIER_WATCHER,
            STATUS_NOTIFIER_WATCHER_PATH,
            "org.freedesktop.DBus.Properties.Get",
            &[STATUS_NOTIFIER_WATCHER, "RegisteredStatusNotifierItems"],
        )
        .ok()?;
        items.split('\'').find_map(|item| item.strip_prefix(&owner).filter(|path| path.starts_with('/')).map(str::to_owned))
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
