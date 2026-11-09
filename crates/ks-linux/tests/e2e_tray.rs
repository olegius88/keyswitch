//! The panel icon and its menu on a real session bus: it registers, answers, and a click reaches the application. Runs
//! under `dbus-run-session`, with `KS_E2E` set; otherwise it says so and passes.
#![expect(clippy::disallowed_macros, clippy::disallowed_methods, reason = "temporary: these tests still build and read their JSON as untyped trees; step 7 of the strict-json plan types them and removes this")]

mod support;

use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use ks_linux::tray::{ITEM_INTERFACE, MENU_INTERFACE, MENU_PATH, MENU_SETTINGS, MENU_SWITCH_LAYOUT, OBJECT_PATH, StatusNotifierItem, TrayAction};
use support::{Watcher, dbus_call};

const REGISTRATION_WAIT: Duration = Duration::from_secs(10);
const EVENT_WAIT: Duration = Duration::from_secs(5);

fn run() -> Result<(), String> {
    let watcher = Watcher::new()?;
    let (sender, receiver) = channel();
    let sender = std::sync::Mutex::new(sender);
    let item = StatusNotifierItem::new(Arc::new(move |action| { let _ = sender.lock().expect("the channel").send(action); }), Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources").as_path())?;
    item.set_layout(1);
    let destination = format!("io.github.olegius88.KeySwitch.StatusNotifierItem.p{}", std::process::id());
    let deadline = Instant::now() + REGISTRATION_WAIT;
    while watcher.service(&destination).is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    if watcher.service(&destination).as_deref() != Some(OBJECT_PATH) {
        return Err(format!("the icon registered an unexpected object path: {:?}", watcher.service(&destination)));
    }
    let properties = "org.freedesktop.DBus.Properties.Get";
    let item_is_menu = dbus_call(&destination, OBJECT_PATH, properties, &[ITEM_INTERFACE, "ItemIsMenu"])?;
    let menu_path = dbus_call(&destination, OBJECT_PATH, properties, &[ITEM_INTERFACE, "Menu"])?;
    let version = dbus_call(&destination, MENU_PATH, properties, &[MENU_INTERFACE, "Version"])?;
    let layout = dbus_call(&destination, MENU_PATH, &format!("{MENU_INTERFACE}.GetLayout"), &["0", "1", "[]"])?;
    let event = format!("{MENU_INTERFACE}.Event");
    for id in [MENU_SETTINGS, MENU_SWITCH_LAYOUT] {
        dbus_call(&destination, MENU_PATH, &event, &[&id.to_string(), "clicked", "<''>", "0"])?;
    }
    // Everything else a tray host may ask: the answers are only looked at where a wrong one would show.
    let all = "org.freedesktop.DBus.Properties.GetAll";
    let properties_of_item = dbus_call(&destination, OBJECT_PATH, all, &[ITEM_INTERFACE])?;
    let properties_of_menu = dbus_call(&destination, MENU_PATH, all, &[MENU_INTERFACE])?;
    for method in ["Activate", "SecondaryActivate", "ContextMenu"] {
        dbus_call(&destination, OBJECT_PATH, &format!("{ITEM_INTERFACE}.{method}"), &["1", "2"])?;
    }
    dbus_call(&destination, OBJECT_PATH, &format!("{ITEM_INTERFACE}.Scroll"), &["1", "vertical"])?;
    let menu = |method: &str| format!("{MENU_INTERFACE}.{method}");
    dbus_call(&destination, MENU_PATH, &menu("GetGroupProperties"), &["[]", "[]"])?;
    dbus_call(&destination, MENU_PATH, &menu("GetGroupProperties"), &["[0, 1, 999]", "['label']"])?;
    dbus_call(&destination, MENU_PATH, &menu("GetProperty"), &["1", "label"])?;
    dbus_call(&destination, MENU_PATH, &menu("GetProperty"), &["0", "visible"])?;
    let refused = [
        dbus_call(&destination, MENU_PATH, &menu("GetProperty"), &["999", "label"]).is_err(),
        dbus_call(&destination, MENU_PATH, &menu("GetProperty"), &["1", "nothing"]).is_err(),
        dbus_call(&destination, MENU_PATH, &menu("GetLayout"), &["999", "1", "[]"]).is_err(),
    ];
    dbus_call(&destination, MENU_PATH, &menu("GetLayout"), &["1", "0", "['label']"])?;
    dbus_call(&destination, MENU_PATH, &menu("GetLayout"), &["0", "0", "['label']"])?;
    dbus_call(&destination, MENU_PATH, &menu("AboutToShow"), &["0"])?;
    dbus_call(&destination, MENU_PATH, &menu("AboutToShowGroup"), &["[0, 1, 999]"])?;
    dbus_call(&destination, MENU_PATH, &menu("EventGroup"), &["[(0, 'hovered', <''>, 0), (999, 'clicked', <''>, 0)]"])?;
    for id in 0..30 {
        let _ = dbus_call(&destination, MENU_PATH, &event, &[&id.to_string(), "clicked", "<''>", "0"]);
    }
    for name in ["type", "label", "icon-name", "toggle-type", "children-display", "toggle-state", "enabled", "visible"] {
        let _ = dbus_call(&destination, MENU_PATH, &menu("GetProperty"), &["0", name]);
    }
    dbus_call(&destination, MENU_PATH, &event, &["0", "clicked", "<''>", "0"])?;
    dbus_call(&destination, MENU_PATH, &event, &["999", "clicked", "<''>", "0"])?;
    dbus_call(&destination, MENU_PATH, &event, &["1", "hovered", "<''>", "0"])?;
    item.set_layout(0);
    item.set_layout(0);
    item.set_layout(-1);
    item.set_enabled(false);
    item.set_enabled(false);
    item.set_enabled(true);
    for style in ["letters", "letters", "flag", "nonsense"] {
        item.set_indicator_style(&serde_json::json!(style));
    }
    item.set_indicator_style(&serde_json::json!(5));
    // A style that changes the icon of a known layout, and nothing else.
    item.set_layout(0);
    for style in ["flags", "letters"] {
        item.set_indicator_style(&serde_json::json!(style));
    }
    for enabled in [false, false, true] {
        item.set_sound_enabled(enabled);
        item.set_notifications_enabled(enabled);
    }
    std::thread::sleep(Duration::from_millis(100));
    let mut actions = Vec::new();
    while actions.len() < 12 {
        match receiver.recv_timeout(Duration::from_millis(200).min(EVENT_WAIT)) {
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
        ("item properties", properties_of_item.contains("KeySwitch") && properties_of_item.contains("ApplicationStatus")),
        ("menu properties", properties_of_menu.contains("normal")),
        ("what the menu does not have", refused.iter().all(|refused| *refused)),
    ];
    item.close();
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
