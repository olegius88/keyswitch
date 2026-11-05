//! The packaged program itself, through real X11 and D-Bus: its panel icon and menu, its corrections, its history and a rule
//! taught through its own windows. Runs under `packaging/e2e.sh <deb>`, with `KS_E2E_BINARY` naming the program; otherwise
//! it says so and passes.

mod support;

use std::path::Path;
use std::process::{Child, Command, Stdio};

use gtk::prelude::*;
use ks_config::config::SettingsStore;
use ks_config::history::HistoryStore;
use ks_config::learning::LearningStore;
use ks_linux::tray::{ITEM_INTERFACE, MENU_INTERFACE, MENU_PATH, OBJECT_PATH};
use serde_json::json;
use support::{Case, IDLE_PAUSE, CONTEXT_RESOLUTION, dbus_call, Watcher, Typer, pump_until, pump, judge, INTER_CASE, GRACE, PAUSE_KEYSYM, SETTLE, RETURN_KEYSYM, LEARNING_CONFIRMATION, entry_window};

const TRAY_READY_TO_TYPING: u64 = 300;
const WARMUP_DEADLINE: u64 = 6000;
const WARMUP_ATTEMPTS: u32 = 3;
const APPLICATION_WAIT: u64 = 45_000;
const GRACEFUL_SHUTDOWN_SECONDS: u64 = 8;
const ONE_SECOND: u64 = 1000;
const NATIVE_SETTLE: u64 = 1000;
const CASES: [Case; 11] = [
    Case("EN pause correction", 0, "ghbdtn", "привет", 1, IDLE_PAUSE),
    Case("RU keys to English", 1, "hello ", "hello ", 0, NATIVE_SETTLE),
    Case("punctuation key is a Russian letter", 0, ",fpf ", "база ", 1, NATIVE_SETTLE),
    Case("return to EN before punctuation test", 1, "hello ", "hello ", 0, NATIVE_SETTLE),
    // Ambiguous punctuation waits for the idle boundary.
    Case("punctuation boundary keeps its glyph", 0, "ghbdtn,", "привет,", 1, IDLE_PAUSE),
    Case("manual layout switch protects next word", 0, "ghbdtn ", "ghbdtn ", 0, NATIVE_SETTLE),
    Case("manual protection is consumed once", 0, "ghbdtn ", "привет ", 1, NATIVE_SETTLE),
    // A lone two-letter word waits for its neighbour; the two are decided together.
    Case("short Russian word switches to English with its neighbour", 1, "if hello ", "if hello ", 0, CONTEXT_RESOLUTION),
    Case("manual Russian selection protects short word", 1, "if ", "ша ", 1, NATIVE_SETTLE),
    Case("short-word protection is consumed once", 1, "if hello ", "if hello ", 0, CONTEXT_RESOLUTION),
    Case("context resolves a short word with the next word", 0, "e 'njuj ", "у этого ", 1, CONTEXT_RESOLUTION),
];
const EXPECTED_HISTORY: [(&str, &str); 9] = [
    ("ghbdtn", "привет"),
    ("руддщ", "hello"),
    (",fpf", "база"),
    ("руддщ", "hello"),
    ("ghbdtn", "привет"),
    ("ghbdtn", "привет"),
    ("ша руддщ", "if hello"),
    ("ша руддщ", "if hello"),
    ("e 'njuj", "у этого"),
];

fn configure(root: &Path) -> Result<(), String> {
    let settings = SettingsStore::new(Some(root.join("config/config.json")));
    for (path, value) in [
        ("general.autostart", json!(false)),
        ("general.notifications", json!(false)),
        ("general.sound", json!(false)),
        ("general.keep_history", json!(true)),
        ("appearance.show_indicator", json!(true)),
        ("appearance.indicator_style", json!("letters")),
        // Synthetic typing has no gaps between keys; prefix switching is covered by the source test and would race here.
        ("detection.early_switch", json!(false)),
    ] {
        settings.set(path, value, true).map_err(|error| format!("{error:?}"))?;
    }
    Ok(())
}

fn tray_checks(destination: &str) -> Result<(), String> {
    let get = |name: &str| dbus_call(destination, OBJECT_PATH, "org.freedesktop.DBus.Properties.Get", &[ITEM_INTERFACE, name]);
    let (item_is_menu, menu_path) = (get("ItemIsMenu")?, get("Menu")?);
    let layout = dbus_call(destination, MENU_PATH, &format!("{MENU_INTERFACE}.GetLayout"), &["0", "1", "[]"])?;
    let checks = [
        ("ItemIsMenu", item_is_menu.contains("true")),
        ("Menu path", menu_path.contains(MENU_PATH)),
        ("Settings", layout.contains("Настройки KeySwitch")),
        ("Autoswitch", layout.contains("Автопереключение")),
        ("History", layout.contains("История исправлений")),
        ("Exclusions", layout.contains("Программы-исключения")),
        ("Quit", layout.contains("Выход")),
    ];
    let failed: Vec<&str> = checks.iter().filter(|(_, passed)| !passed).map(|(name, _)| *name).collect();
    if failed.is_empty() { Ok(()) } else { Err(format!("packaged tray checks failed: {}", failed.join(", "))) }
}

fn stop(child: &mut Child) -> i32 {
    let _ = Command::new("kill").arg("-TERM").arg(child.id().to_string()).status();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(GRACEFUL_SHUTDOWN_SECONDS);
    while std::time::Instant::now() < deadline {
        if let Ok(Some(status)) = child.try_wait() {
            return status.code().unwrap_or(-1);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let _ = child.kill();
    child.wait().ok().and_then(|status| status.code()).unwrap_or(-1)
}

fn history_of(root: &Path) -> Result<Vec<(String, String)>, String> {
    let history = HistoryStore::new(Some(root.join("data/history.jsonl")), 200);
    Ok(history.read(None).map_err(|error| error.to_string())?.into_iter().map(|item| (item.original, item.replacement)).collect())
}

fn wanted(extra: &[(&str, &str)]) -> Vec<(String, String)> {
    EXPECTED_HISTORY.iter().chain(extra).map(|(a, b)| ((*a).to_owned(), (*b).to_owned())).collect()
}

fn scenario(root: &Path, child: &mut Child, watcher: &Watcher, entry: &gtk::Entry, window: &gtk::Window, typer: &Typer) -> Result<(), String> {
    let destination = format!("io.github.olegius88.KeySwitch.StatusNotifierItem.p{}", child.id());
    let mut exited = None;
    let registered = pump_until(APPLICATION_WAIT, || {
        exited = child.try_wait().ok().flatten();
        exited.is_some() || watcher.service().is_some()
    });
    if let Some(status) = exited {
        return Err(format!("the program exited early with {status}"));
    }
    if !registered {
        return Err("the panel icon did not register".to_owned());
    }
    if watcher.service().as_deref() != Some(OBJECT_PATH) {
        return Err(format!("unexpected registered tray path {:?}", watcher.service()));
    }
    tray_checks(&destination)?;
    pump(TRAY_READY_TO_TYPING);
    let group = || typer.current_group();
    let type_case = |case: &Case| -> Result<(), String> {
        typer.switch_group(case.1)?;
        entry.grab_focus();
        typer.clear_field()?;
        typer.type_keys(case.2)
    };
    // The icon registers before the engine's listener has necessarily seen a key, and a loaded runner delays the pause
    // timer, so the first case is typed again when it is not corrected in time. The corrections of the earlier attempts
    // are in the history before this one types.
    let mut warmup = 0;
    for attempt in 1..=WARMUP_ATTEMPTS {
        warmup = history_of(root)?.len();
        type_case(&CASES[0])?;
        match judge(entry, &group, &CASES[0], WARMUP_DEADLINE) {
            Ok(()) => break,
            Err(error) if attempt == WARMUP_ATTEMPTS => return Err(error),
            Err(_) => pump(INTER_CASE),
        }
    }
    pump(INTER_CASE);
    for case in &CASES[1..] {
        type_case(case)?;
        judge(entry, &group, case, GRACE)?;
        pump(INTER_CASE);
    }
    let actual: Vec<(String, String)> = history_of(root)?.split_off(warmup);
    if actual != wanted(&[]) {
        return Err(format!("wrong correction history: {actual:?}"));
    }
    // A double press: the first converts the word, the second offers a rule.
    typer.switch_group(0)?;
    window.present();
    entry.grab_focus();
    typer.clear_field()?;
    typer.type_keys("hello")?;
    typer.tap_keysym(PAUSE_KEYSYM)?;
    typer.tap_keysym(PAUSE_KEYSYM)?;
    pump(SETTLE);
    if entry.text() != "руддщ" || group() != 1 {
        return Err(format!("manual learning conversion failed: text={:?} group={}", entry.text().as_str(), group()));
    }
    typer.tap_keysym(RETURN_KEYSYM)?;
    pump(SETTLE);
    // Enter on the offer only opened the rule window; nothing is stored before OK.
    let learning = LearningStore::new(Some(root.join("data/learning.json")), None).map_err(|error| format!("{error:?}"))?;
    learning.load().map_err(|error| format!("{error:?}"))?;
    if learning.count() != 0 {
        return Err("a rule was stored before OK in the packaged rule window".to_owned());
    }
    typer.tap_keysym(RETURN_KEYSYM)?;
    pump(LEARNING_CONFIRMATION);
    learning.load().map_err(|error| format!("{error:?}"))?;
    if learning.forced_target(0, "hello") != Some(1) {
        return Err("OK in the rule window did not persist the packaged rule".to_owned());
    }
    typer.switch_group(0)?;
    window.present();
    entry.grab_focus();
    typer.clear_field()?;
    typer.type_keys("hello ")?;
    pump(SETTLE);
    if entry.text() != "hello " || group() != 0 {
        return Err(format!("manual layout did not override the packaged learned rule: text={:?} group={}", entry.text().as_str(), group()));
    }
    window.present();
    entry.grab_focus();
    typer.clear_field()?;
    typer.type_keys("hello ")?;
    pump(SETTLE);
    if entry.text() != "руддщ " || group() != 1 {
        return Err(format!("packaged learned rule did not run: text={:?} group={}", entry.text().as_str(), group()));
    }
    let actual: Vec<(String, String)> = history_of(root)?.split_off(warmup);
    if actual != wanted(&[("hello", "руддщ")]) {
        return Err(format!("wrong learned correction history: {actual:?}"));
    }
    println!("NATIVE_LEARNING_PROMPT_E2E_OK");
    Ok(())
}

fn run(binary: &Path) -> Result<(), String> {
    gtk::init().map_err(|error| error.to_string())?;
    let root = std::env::temp_dir().join(format!("keyswitch-e2e-package-{}", std::process::id()));
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    configure(&root)?;
    let watcher = Watcher::new()?;
    let typer = Typer::new()?;
    let original = typer.current_group();
    let (window, entry) = entry_window("KeySwitch native package E2E");
    let mut child = Command::new(binary)
        .arg("--hidden")
        .env("KEYSWITCH_CONFIG_DIR", root.join("config"))
        .env("KEYSWITCH_DATA_DIR", root.join("data"))
        .env("XDG_CONFIG_HOME", root.join("xdg-config"))
        .env("XDG_DATA_HOME", root.join("xdg-data"))
        .env("GTK_A11Y", "none")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| error.to_string())?;
    let outcome = scenario(&root, &mut child, &watcher, &entry, &window, &typer);
    let _ = typer.switch_group(original);
    let code = stop(&mut child);
    if outcome.is_err()
        && let Ok(log) = std::fs::read_to_string(root.join("data/keyswitch.log")) {
            println!("native log: {}", log.trim());
        }
    window.destroy();
    pump(ONE_SECOND / 10);
    let _ = std::fs::remove_dir_all(&root);
    outcome?;
    if code != 0 {
        return Err(format!("native shutdown returned {code}"));
    }
    Ok(())
}

fn main() {
    let Some(binary) = std::env::var_os("KS_E2E_BINARY") else {
        println!("KS_E2E_BINARY is not set: no packaged program to drive, skipped");
        return;
    };
    match run(Path::new(&binary)) {
        Ok(()) => println!("NATIVE_E2E_OK"),
        Err(error) => {
            println!("NATIVE_E2E_FAILED: {error}");
            std::process::exit(1);
        }
    }
}
