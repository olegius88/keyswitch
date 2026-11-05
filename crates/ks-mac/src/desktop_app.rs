//! The program on macOS: AppKit owns the main thread, the models load in the background, the menu bar item comes up at once.
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;

use ks_backend::InputBackend;
use ks_config::config::SettingsStore;
use ks_config::history::{HistoryStore, data_dir, history_limit};
use ks_config::indicator::normalize_indicator_style;
use ks_config::learning::LearningStore;
use ks_config::logsetup;
use ks_engine::KeySwitchEngine;
use ks_engine::context_access::ReaderFactory;
use ks_engine::engine_loader::{Build, DeferredEngine, Post};
use ks_ui::tray_model::{TrayAction, TrayState};
use ks_ui::windows_ui_model::{NoUpdates, truthy};
use ks_updates::{UpdateController, UpdateManager};
use objc2::{class, msg_send};
use serde_json::Value;

use crate::constants::{BACKEND_LABEL, MACOS_PERMISSION_POLL_SECONDS};
use crate::macos_app::{PERMISSION_GRANTED_MESSAGE, PERMISSION_REQUEST_MESSAGE, VERSION, resource_dir};
use crate::macos_backend::MacBackend;
use crate::macos_context::MacFieldReader;
use crate::macos_native::{NativeAccessibility, run_on_main};
use crate::macos_objc::{Id, beep};
use crate::macos_tray_native::StatusItem;
use crate::settings_window;

const LOGGER: &str = "keyswitch.desktop_ui";
const ENGINE_STOP_WAIT_SECONDS: u64 = 5;
const UPDATE_CHECK_INITIAL_DELAY_SECONDS: u64 = 30;
const UPDATE_CHECK_INTERVAL_SECONDS: u64 = 6 * 3600;

pub struct Services {
    pub settings: Arc<SettingsStore>,
    pub history: Arc<HistoryStore>,
    pub learning: Arc<LearningStore>,
    pub engine: Arc<DeferredEngine>,
    pub updates: Arc<dyn UpdateController>,
    pub backend: Arc<dyn InputBackend>,
}

struct App {
    services: Arc<Services>,
    tray: Option<StatusItem>,
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
        let seconds = now.as_secs() as i64;
        let mut tm = Tm::default();
        // SAFETY: both pointers are valid for the call.
        unsafe { localtime_r(&seconds, &mut tm) };
        logsetup::LocalTime { year: tm.year + 1900, month: (tm.mon + 1) as u32, day: tm.mday as u32, hour: tm.hour as u32, minute: tm.min as u32, second: tm.sec as u32, millisecond: now.subsec_millis() }
    }
    logsetup::set_local_clock(local_clock);
    logsetup::set_platform_description("macOS");
}

fn tray_state(services: &Services) -> TrayState {
    let snapshot = services.engine.snapshot();
    TrayState {
        group: snapshot.current_group,
        enabled: snapshot.enabled,
        sound_enabled: setting_bool(&services.settings, "general.sound", false),
        notifications_enabled: setting_bool(&services.settings, "general.notifications", true),
        indicator_style: normalize_indicator_style(&services.settings.get("appearance.indicator_style").unwrap_or_else(|| Value::String("letters".into()))),
    }
}

fn sync_tray() {
    with_app(|app| {
        let state = tray_state(&app.services);
        if let Some(tray) = &app.tray {
            tray.update(state);
        }
    });
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
        engine.subscribe_corrections(Arc::new(|_plan| {
            run_on_main(Box::new(|| {
                let sound = with_app(|app| setting_bool(&app.services.settings, "general.sound", false)).unwrap_or(false);
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
/// it asks and waits, and carries on without a relaunch once the user grants it.
fn ensure_permission(backend: &MacBackend) {
    if backend.permission_granted() {
        return;
    }
    logsetup::warning(LOGGER, PERMISSION_REQUEST_MESSAGE);
    println!("{PERMISSION_REQUEST_MESSAGE}");
    backend.request_permission();
    while !backend.permission_granted() {
        std::thread::sleep(std::time::Duration::from_secs_f64(MACOS_PERMISSION_POLL_SECONDS));
    }
    logsetup::info(LOGGER, PERMISSION_GRANTED_MESSAGE);
    println!("{PERMISSION_GRANTED_MESSAGE}");
}

fn update_checks(services: Arc<Services>) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(UPDATE_CHECK_INITIAL_DELAY_SECONDS));
        loop {
            if setting_bool(&services.settings, "updates.check_automatically", true) {
                services.updates.check(true, false);
            }
            std::thread::sleep(std::time::Duration::from_secs(UPDATE_CHECK_INTERVAL_SECONDS));
        }
    });
}

pub fn run(hidden: bool, no_engine: bool) -> i32 {
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
    ensure_permission(&backend);
    let input: Arc<dyn InputBackend> = backend;
    let resources = ks_lang::Resources::new(resource_dir());
    let build: Build = {
        let (settings, history, learning, input) = (settings.clone(), history.clone(), learning.clone(), input.clone());
        Box::new(move || {
            let factory: ReaderFactory = Arc::new(|| Ok(Box::new(MacFieldReader::new(NativeAccessibility::new())) as Box<dyn ks_backend::ManagedFieldReader>));
            KeySwitchEngine::new(settings, history, input, Some(learning), BACKEND_LABEL, None, Some(factory), &resources, ks_context::input_context::monotonic_clock())
        })
    };
    let post: Post = Arc::new(|action| run_on_main(action));
    let enabled = setting_bool(&settings, "enabled", true);
    let engine = DeferredEngine::new(build, learning.clone(), enabled, post, true);
    let updates: Arc<dyn UpdateController> = match UpdateManager::with_opener(VERSION, data_dir().join("updates"), Arc::new(ks_updates::http_curl::CurlOpener)) {
        Ok(manager) => Arc::new(manager),
        Err(error) => {
            logsetup::warning(LOGGER, &format!("Обновления недоступны: {error}"));
            Arc::new(NoUpdates { version: VERSION.to_owned() })
        }
    };
    let services = Arc::new(Services { settings: settings.clone(), history, learning, engine: engine.clone(), updates, backend: input });
    // SAFETY: on the main thread, before anything else of AppKit.
    let application: Id = unsafe { msg_send![class!(NSApplication), sharedApplication] };
    let handler = Arc::new(tray_action);
    let tray = match StatusItem::start(handler, tray_state(&services)) {
        Ok(tray) => Some(tray),
        Err(error) => {
            logsetup::error(LOGGER, &error);
            None
        }
    };
    APP.with(|slot| *slot.borrow_mut() = Some(App { services: services.clone(), tray, closing: false }));
    settings_window::create(services.clone());
    let _ = settings.subscribe(Arc::new(|_path: &str, _value: &Value| {
        run_on_main(Box::new(sync_tray));
    }));
    attach_engine(&services);
    if !no_engine {
        let _ = engine.start(Arc::new(|error: &str| {
            logsetup::error(LOGGER, error);
            let error = error.to_owned();
            run_on_main(Box::new(move || settings_window::show_error(&error)));
        }));
    }
    update_checks(services);
    if !hidden {
        settings_window::show(None);
    }
    // SAFETY: AppKit's run loop on the main thread; it serves the main dispatch queue the input sources are read on.
    unsafe {
        let _: () = msg_send![application, run];
    }
    0
}

#[allow(dead_code)]
fn unused(_: PathBuf) {}
