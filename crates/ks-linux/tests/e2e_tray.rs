//! The panel icon and its menu on a real session bus: it registers, answers, and a click reaches the application. Runs
//! under `dbus-run-session`, with `KS_E2E` set; otherwise it says so and passes.

mod support;

use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use ks_linux::tray::{ITEM_INTERFACE, MENU_INTERFACE, MENU_PATH, MENU_SETTINGS, MENU_SWITCH_LAYOUT, OBJECT_PATH, StatusNotifierItem, TrayAction};
use support::*;

const REGISTRATION_WAIT: Duration = Duration::from_secs(10);
const EVENT_WAIT: Duration = Duration::from_secs(5);

fn run() -> Result<(), String> {
    let watcher = Watcher::new()?;
    let (sender, receiver) = channel();
    let sender = std::sync::Mutex::new(sender);
    let item = StatusNotifierItem::new(Arc::new(move |action| { let _ = sender.lock().expect("the channel").send(action); }), Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources").as_path())?;
    item.set_layout(1);
    let deadline = Instant::now() + REGISTRATION_WAIT;
    while watcher.service().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    if watcher.service().as_deref() != Some(OBJECT_PATH) {
        return Err(format!("the icon registered an unexpected object path: {:?}", watcher.service()));
    }
    let destination = format!("io.github.olegius88.KeySwitch.StatusNotifierItem.p{}", std::process::id());
    let properties = "org.freedesktop.DBus.Properties.Get";
    let item_is_menu = dbus_call(&destination, OBJECT_PATH, properties, &[ITEM_INTERFACE, "ItemIsMenu"])?;
    let menu_path = dbus_call(&destination, OBJECT_PATH, properties, &[ITEM_INTERFACE, "Menu"])?;
    let version = dbus_call(&destination, MENU_PATH, properties, &[MENU_INTERFACE, "Version"])?;
    let layout = dbus_call(&destination, MENU_PATH, &format!("{MENU_INTERFACE}.GetLayout"), &["0", "1", "[]"])?;
    let event = format!("{MENU_INTERFACE}.Event");
    for id in [MENU_SETTINGS, MENU_SWITCH_LAYOUT] {
        dbus_call(&destination, MENU_PATH, &event, &[&id.to_string(), "clicked", "<''>", "0"])?;
    }
    let mut actions = Vec::new();
    while actions.len() < 2 {
        match receiver.recv_timeout(EVENT_WAIT) {
            Ok(action) => actions.push(action),
            Err(_) => break,
        }
    }
    let checks = [
        ("ItemIsMenu", item_is_menu.contains("true")),
        ("Menu object path", menu_path.contains(MENU_PATH)),
        ("DBusMenu version", version.contains("uint32 3")),
        ("settings menu item", layout.contains("Настройки KeySwitch")),
        ("alternate layout menu item", layout.contains("Переключить на английский (EN)")),
        ("autoswitch menu item", layout.contains("Автопереключение")),
        ("quit menu item", layout.contains("Выход")),
        ("clicked event", actions.contains(&TrayAction::ShowSettings)),
        ("layout clicked event", actions.contains(&TrayAction::SwitchLayout)),
    ];
    let failed: Vec<&str> = checks.iter().filter(|(_, passed)| !passed).map(|(name, _)| *name).collect();
    if failed.is_empty() { Ok(()) } else { Err(format!("failed tray checks: {}", failed.join(", "))) }
}

fn main() {
    if std::env::var_os("KS_E2E").is_none() {
        println!("KS_E2E is not set: no private session bus, skipped");
        return;
    }
    match run() {
        Ok(()) => println!("TRAY_MENU_E2E_OK"),
        Err(error) => {
            println!("TRAY_MENU_E2E_FAILED: {error}");
            std::process::exit(1);
        }
    }
}
