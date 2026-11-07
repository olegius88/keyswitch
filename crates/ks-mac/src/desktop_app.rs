//! The program on macOS: `AppKit` owns the main thread, the models load in the background, the menu bar item comes up at once.
#![cfg_attr(target_os = "macos", expect(unsafe_code, reason = "calls into the macOS system libraries"))]
use ks_cast::To as _;
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;

use ks_backend::InputBackend;
use ks_config::config::SettingsStore;
use ks_config::history::{HistoryStore, history_limit};
use ks_config::indicator::normalize_indicator_style;
use ks_config::learning::LearningStore;
use ks_config::logsetup;
use ks_engine::KeySwitchEngine;
use ks_engine::context_access::ReaderFactory;
use ks_engine::engine_loader::{Build, DeferredEngine, Post};
use ks_ui::tray_model::{TrayAction, TrayState};
use ks_ui::windows_ui_model::truthy;
use objc2::{class, msg_send};
use serde_json::Value;

use crate::constants::{BACKEND_LABEL, MACOS_PERMISSION_POLL_SECONDS};
use crate::macos_app::{PERMISSION_GRANTED_MESSAGE, PERMISSION_REQUEST_MESSAGE, resource_dir};
use crate::macos_backend::MacBackend;
use crate::macos_context::MacFieldReader;
use crate::macos_native::{NativeAccessibility, run_on_main, run_on_main_after};
use crate::macos_objc::{Id, beep, notify};
use crate::macos_system::MacAutostartManager;
use crate::macos_tray_native::StatusItem;
use crate::settings_window;

const LOGGER: &str = "keyswitch.desktop_ui";
const ENGINE_STOP_WAIT_SECONDS: u64 = 5;

pub struct Services {
    pub settings: Arc<SettingsStore>,
    pub history: Arc<HistoryStore>,
    pub learning: Arc<LearningStore>,
    pub engine: Arc<DeferredEngine>,
    pub backend: Arc<dyn InputBackend>,
}

struct App {
    services: Arc<Services>,
    permission: Arc<MacBackend>,
    awaiting_permission: bool,
    tray: Option<StatusItem>,
    autostart: MacAutostartManager,
    closing: bool,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn with_app<T>(action: impl FnOnce(&mut App) -> T) -> Option<T> {
    APP.with(|app| app.try_borrow_mut().ok().and_then(|mut app| app.as_mut().map(action)))
}

pub fn setting_bool(settings: &SettingsStore, path: &str, default: bool) -> bool {
    settings.get(path).as_ref().map_or(default, truthy)
}

/// What the logger asks of the platform: the local time and the system's name.
pub fn install_platform() {
    unsafe extern "C" {
        fn localtime_r(time: *const i64, result: *mut Tm) -> *mut Tm;
    }
    #[repr(C)]
    #[derive(Default)]
    struct Tm {
        sec: i32,
        min: i32,
        hour: i32,
        mday: i32,
        mon: i32,
        year: i32,
        wday: i32,
        yday: i32,
        isdst: i32,
        gmtoff: i64,
        zone: usize,
    }
    fn local_clock() -> logsetup::LocalTime {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let seconds = now.as_secs().bits::<i64>();
        let mut tm = Tm::default();
        // SAFETY: both pointers are valid for the call.
        unsafe { localtime_r(&raw const seconds, &raw mut tm) };
        logsetup::LocalTime { year: tm.year + 1900, month: (tm.mon + 1).bits::<u32>(), day: tm.mday.bits::<u32>(), hour: tm.hour.bits::<u32>(), minute: tm.min.bits::<u32>(), second: tm.sec.bits::<u32>(), millisecond: now.subsec_millis() }
    }
    logsetup::set_local_clock(local_clock);
    logsetup::set_platform_description("macOS");
}

fn tray_state(app: &App) -> TrayState {
    let services = &app.services;
    let snapshot = services.engine.snapshot();
    TrayState {
        group: snapshot.current_group,
        enabled: snapshot.enabled,
        sound_enabled: setting_bool(&services.settings, "general.sound", false),
        notifications_enabled: setting_bool(&services.settings, "general.notifications", true),
        indicator_style: normalize_indicator_style(&services.settings.get("appearance.indicator_style").unwrap_or_else(|| Value::String("letters".into()))),
        awaiting_permission: app.awaiting_permission,
    }
}

fn sync_tray() {
    with_app(|app| {
        let state = tray_state(app);
        if let Some(tray) = &app.tray {
            tray.update(state);
        }
    });
}

const INSTANCE_LOCK_FILE: &str = "keyswitch.lock";

fn sync_autostart() {
    with_app(|app| {
        let settings = &app.services.settings;
        let requested = setting_bool(settings, "general.autostart", true);
        match app.autostart.set_enabled(requested, setting_bool(settings, "general.start_hidden", true)) {
            Ok(()) => {
                let status = app.autostart.status();
                if requested && status.target_missing {
                    logsetup::warning(LOGGER, &format!("Команда автозагрузки указывает на отсутствующий файл: {}", status.command.unwrap_or_default()));
                    run_on_main(Box::new(|| settings_window::show_error("Автозагрузка указывает на другую папку; переключите её заново.")));
                }
            }
            Err(error) => {
                logsetup::warning(LOGGER, &format!("Не удалось настроить автозагрузку: {error}"));
                run_on_main(Box::new(move || settings_window::show_error(&format!("Автозагрузка: {error}"))));
            }
        }
    });
}

/// The menu bar item follows its setting; without it the window is the only way back in.
fn sync_indicator() {
    let shown = with_app(|app| {
        let requested = setting_bool(&app.services.settings, "appearance.show_indicator", true);
        if requested && app.tray.is_none() {
            match StatusItem::start(Arc::new(tray_action), tray_state(app)) {
                Ok(tray) => app.tray = Some(tray),
                Err(error) => logsetup::error(LOGGER, &error),
            }
        } else if !requested
            && let Some(tray) = app.tray.take() {
                tray.close();
                return false;
            }
        true
    });
    if shown == Some(false) {
        settings_window::show(None);
    }
}

fn apply_setting(path: &str, value: &Value) {
    settings_window::apply_setting(path, value);
    match path {
        "*" => {
            with_app(|app| app.services.history.set_limit(history_limit(&app.services.settings.get("history.limit").unwrap_or(Value::Null))));
            sync_autostart();
            sync_indicator();
        }
        "general.autostart" | "general.start_hidden" => sync_autostart(),
        "appearance.show_indicator" => sync_indicator(),
        "history.limit" => {
            with_app(|app| app.services.history.set_limit(history_limit(value)));
        }
        _ => {}
    }
    sync_tray();
}

fn toggle(services: &Services, path: &str, default: bool) {
    let value = !setting_bool(&services.settings, path, default);
    if let Err(error) = services.settings.set(path, Value::Bool(value), true) {
        logsetup::error(LOGGER, &format!("Настройка не сохранена: {error}"));
    }
}

fn tray_action(action: TrayAction) {
    run_on_main(Box::new(move || {
        let Some(services) = with_app(|app| app.services.clone()) else { return };
        match action {
            TrayAction::ShowSettings => settings_window::show(None),
            TrayAction::ShowHistory => settings_window::show(Some("history")),
            TrayAction::ShowExclusions => settings_window::show(Some("exclusions")),
            TrayAction::ShowAbout => settings_window::show(Some("about")),
            TrayAction::SwitchLayout => {
                services.engine.select_alternate_group();
            }
            TrayAction::ToggleEngine => toggle(&services, "enabled", true),
            TrayAction::ToggleSound => toggle(&services, "general.sound", false),
            TrayAction::ToggleNotifications => toggle(&services, "general.notifications", true),
            TrayAction::OpenPermissionSettings => {
                let _ = with_app(|app| app.permission.open_permission_settings());
            }
            TrayAction::QuitApplication => quit(),
        }
        sync_tray();
    }));
}

/// `[NSApp stop:]` only takes effect once another event arrives, so a program nobody is clicking would never notice: the
/// process ends here, after a bounded wait for the engine.
pub fn quit() {
    let Some(services) = with_app(|app| {
        app.closing = true;
        app.tray.take().inspect(StatusItem::close);
        app.services.clone()
    }) else {
        std::process::exit(0)
    };
    std::thread::spawn(move || {
        services.engine.stop();
        std::process::exit(0);
    });
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(ENGINE_STOP_WAIT_SECONDS));
        std::process::exit(0);
    });
}

fn attach_engine(services: &Arc<Services>) {
    services.engine.when_built(Box::new(|engine: &KeySwitchEngine| {
        engine.subscribe(Arc::new(|_snapshot| {
            run_on_main(Box::new(|| {
                sync_tray();
                settings_window::refresh_status();
            }));
        }));
        engine.subscribe_corrections(Arc::new(|plan| {
            let title = if plan.case_fix() { "Регистр исправлен" } else { "Раскладка исправлена" };
            let message = format!("{}  →  {}", plan.original, plan.replacement);
            run_on_main(Box::new(move || {
                let Some((notifications, sound)) = with_app(|app| {
                    let settings = &app.services.settings;
                    (setting_bool(settings, "general.notifications", true), setting_bool(settings, "general.sound", false))
                }) else {
                    return;
                };
                if notifications {
                    logsetup::info(LOGGER, &format!("{title}: {message}"));
                    notify(title, &message);
                }
                if sound {
                    beep();
                }
            }));
        }));
        engine.subscribe_learning_prompts(Arc::new(|prompt| {
            let prompt = prompt.cloned();
            run_on_main(Box::new(move || settings_window::learning_prompt(prompt.as_ref())));
        }));
        engine.subscribe_rule_requests(Arc::new(|prompt| {
            let prompt = prompt.clone();
            run_on_main(Box::new(move || settings_window::rule_editor(Some(&prompt))));
        }));
    }));
}

/// Without the permission the program would look alive and correct nothing, which is the hardest kind of fault to report:
/// it asks, says in the menu bar that it waits, and carries on without a relaunch once the user grants it.
fn ask_for_permission(backend: &MacBackend) {
    logsetup::warning(LOGGER, PERMISSION_REQUEST_MESSAGE);
    println!("{PERMISSION_REQUEST_MESSAGE}");
    let _ = backend.request_permission();
}

/// Asks again once a second on the main queue, which `AppKit`'s run loop serves. As measured on macOS 13, the process
/// keeps its answer of `AXIsProcessTrusted` and asks the system again only after the main run loop has delivered the
/// distributed notification `com.apple.accessibility.api`, which comes when the switch is turned on: asked from a main
/// thread that sleeps instead, the answer stays "no" after the grant until the program is started again.
fn wait_for_permission(no_engine: bool) {
    run_on_main_after(MACOS_PERMISSION_POLL_SECONDS, Box::new(move || match with_app(|app| app.permission.permission_granted()) {
        Some(true) => carry_on(no_engine),
        Some(false) => wait_for_permission(no_engine),
        None => {}
    }));
}

fn carry_on(no_engine: bool) {
    logsetup::info(LOGGER, PERMISSION_GRANTED_MESSAGE);
    println!("{PERMISSION_GRANTED_MESSAGE}");
    let engine = with_app(|app| {
        app.awaiting_permission = false;
        app.services.engine.clone()
    });
    sync_tray();
    if let Some(engine) = engine
        && !no_engine
    {
        start_engine(&engine);
    }
}

fn start_engine(engine: &Arc<DeferredEngine>) {
    let _ = engine.start(Arc::new(|error: &str| {
        logsetup::error(LOGGER, error);
        let error = error.to_owned();
        run_on_main(Box::new(move || settings_window::show_error(&error)));
    }));
}

/// Two copies would both correct the same word and garble it, so a second one started directly (the system stops only
/// those it starts itself) ends at once. The lock goes away with the process.
fn only_copy() -> Result<Option<std::fs::File>, ()> {
    let directory = ks_config::history::data_dir();
    if std::fs::create_dir_all(&directory).is_err() {
        return Ok(None);
    }
    let Ok(file) = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(directory.join(INSTANCE_LOCK_FILE)) else {
        return Ok(None);
    };
    match file.try_lock() {
        Err(std::fs::TryLockError::WouldBlock) => Err(()),
        _ => Ok(Some(file)),
    }
}

pub fn run(hidden: bool, no_engine: bool) -> i32 {
    let Ok(_instance_lock) = only_copy() else {
        println!("KeySwitch уже запущен");
        return 0;
    };
    let settings = Arc::new(SettingsStore::new(None));
    logsetup::follow_settings(&settings, None);
    let limit = history_limit(&settings.get("history.limit").unwrap_or(Value::from(ks_config::constants::DEFAULT_HISTORY_LIMIT)));
    let history = Arc::new(HistoryStore::new(None, limit));
    let learning = match LearningStore::new(None, None) {
        Ok(learning) => Arc::new(learning),
        Err(error) => {
            logsetup::error(LOGGER, &format!("Не удалось открыть правила переключения: {error}"));
            return 1;
        }
    };
    let backend = match MacBackend::new(None) {
        Ok(backend) => Arc::new(backend),
        Err(error) => {
            logsetup::error(LOGGER, &error.to_string());
            return 1;
        }
    };
    let granted = backend.permission_granted();
    let input: Arc<dyn InputBackend> = backend.clone();
    let resources = ks_lang::Resources::new(resource_dir());
    let build: Build = {
        let (settings, history, learning, input) = (settings.clone(), history.clone(), learning.clone(), input.clone());
        Box::new(move || {
            let factory: ReaderFactory = Arc::new(|| Ok(Box::new(MacFieldReader::new(NativeAccessibility::new()))));
            KeySwitchEngine::new(settings, history, input, Some(learning), BACKEND_LABEL, None, Some(factory), &resources, ks_context::input_context::monotonic_clock())
        })
    };
    let post: Post = Arc::new(run_on_main);
    let enabled = setting_bool(&settings, "enabled", true);
    let engine = DeferredEngine::new(build, learning.clone(), enabled, post, true);
    let services = Arc::new(Services { settings: settings.clone(), history, learning, engine: engine.clone(), backend: input });
    // SAFETY: on the main thread, before anything else of AppKit.
    let application: Id = unsafe { msg_send![class!(NSApplication), sharedApplication] };
    let autostart = MacAutostartManager::new(None, None);
    APP.with(|slot| *slot.borrow_mut() = Some(App { services: services.clone(), permission: backend.clone(), awaiting_permission: !granted, tray: None, autostart, closing: false }));
    settings_window::create(services.clone());
    let showing = setting_bool(&settings, "appearance.show_indicator", true);
    if showing {
        sync_indicator();
    }
    sync_autostart();
    let _ = settings.subscribe(Arc::new(|path: &str, value: &Value| {
        let (path, value) = (path.to_owned(), value.clone());
        run_on_main(Box::new(move || apply_setting(&path, &value)));
    }));
    services.history.subscribe(Arc::new(|| run_on_main(Box::new(settings_window::refresh_history))));
    attach_engine(&services);
    if !granted {
        ask_for_permission(&backend);
        wait_for_permission(no_engine);
    } else if !no_engine {
        start_engine(&engine);
    }
    // Without the item in the menu bar the window is the only way in, whatever the arguments.
    if !hidden || !showing {
        settings_window::show(None);
    }
    // SAFETY: AppKit's run loop on the main thread; it serves the main dispatch queue the input sources are read on.
    unsafe {
        let _: () = msg_send![application, run];
    }
    0
}

#[expect(dead_code, reason = "the macOS build alone uses it")]
fn unused(_: PathBuf) {}
