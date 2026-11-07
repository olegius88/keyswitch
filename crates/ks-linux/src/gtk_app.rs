//! The application with a window (the Python's `app.py`): its life cycle, the tray, the notifications and the rule windows.
//!
//! GTK runs on the main thread, the engine on its worker: whatever the engine, the settings or the updater report from
//! another thread is posted to the main loop with `glib::idle_add_once`, and runs there against the application kept in a
//! thread-local, as the Windows application does.

use ks_cast::To as _;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use ks_backend::ScreenAnchor;
use ks_config::config::SettingsStore;
use ks_config::history::{HistoryStore, history_limit};
use ks_config::learning::{LearnedRule, LearningStore};
use ks_config::logsetup;
use ks_engine::engine::default_learning_store;
use ks_engine::engine_loader::{Build, DeferredEngine, Post};
use ks_engine::rule_editor::{RuleDraft, build_rule, draft_from_prompt};
use ks_engine::{EngineSnapshot, KeySwitchEngine, LearningPrompt};
use ks_updates::constants::{UPDATE_CHECK_INITIAL_DELAY_SECONDS, UPDATE_CHECK_INTERVAL_SECONDS};
use ks_updates::{UpdateController, UpdateManager, UpdatePhase, UpdateSnapshot};
use serde_json::{Value, json};

use crate::atspi_bus;
use crate::atspi_caret::focused_caret_anchor;
use crate::constants::APP_ID;
use crate::desktop_backend::{DesktopBackend, new_desktop_backend};
use crate::gtk_prompt::{LearningPromptWindow, PromptAnchor, PromptCallbacks, RuleCallbacks, RuleEditorWindow};
use crate::gtk_window::{MainWindow, ModelInfo, Services, SharedModelInfo, apply_theme};
use crate::linux_app::{VERSION, engine_builder, resource_dir, setting_bool};
use crate::system::AutostartManager;
use crate::tray::{StatusNotifierItem, TrayAction};

const LOGGER: &str = "keyswitch.app";

thread_local! {
    static APP: RefCell<Option<Rc<App>>> = const { RefCell::new(None) };
}

/// The application and what it keeps; it lives on the main thread.
pub struct App {
    pub application: adw::Application,
    pub services: Rc<Services>,
    pub desktop: DesktopBackend,
    learning: Arc<LearningStore>,
    autostart: AutostartManager,
    close_updates: Box<dyn Fn()>,
    hidden: bool,
    no_engine: bool,
    window: RefCell<Option<Rc<MainWindow>>>,
    tray: RefCell<Option<StatusNotifierItem>>,
    pub prompt: RefCell<Option<LearningPromptWindow>>,
    pub rule_editor: RefCell<Option<RuleEditorWindow>>,
    /// The window the user typed in when the last rule offer closed, and the one the open rule window gives the keyboard
    /// back to.
    offer_window: Cell<Option<i64>>,
    rule_return_window: Cell<Option<i64>>,
    initialized: Cell<bool>,
    hold: RefCell<Option<gio::ApplicationHoldGuard>>,
    update_initial: RefCell<Option<glib::SourceId>>,
    update_periodic: RefCell<Option<glib::SourceId>>,
    last_update_notification: RefCell<String>,
    /// The switch of the autostart row being put back after a failed change.
    pub reverting_autostart: Cell<bool>,
}

fn app() -> Option<Rc<App>> {
    APP.with(|slot| slot.borrow().clone())
}

pub fn with_app(action: impl FnOnce(&Rc<App>)) {
    if let Some(app) = app() {
        action(&app);
    }
}

/// Runs `action` on the settings window if there is one.
/// The window as a plain widget, for a test to walk through.
#[must_use]
pub fn with_app_window() -> Option<gtk::Widget> {
    let mut found = None;
    with_window(|window| found = Some(window.window.clone().upcast::<gtk::Widget>()));
    found
}

/// What a call on the window answers, when there is a window.
pub fn with_window_result<T>(action: impl FnOnce(&MainWindow) -> T) -> Option<T> {
    let mut found = None;
    with_window(|window| found = Some(action(window)));
    found
}

pub fn with_window(action: impl FnOnce(&MainWindow)) {
    if let Some(window) = app().and_then(|app| app.window.borrow().clone()) {
        action(&window);
    }
}

fn post(work: impl FnOnce() + Send + 'static) {
    glib::idle_add_once(work);
}

fn post_app(action: impl FnOnce(&Rc<App>) + Send + 'static) {
    post(move || with_app(action));
}

fn toast(message: &str) {
    with_window(|window| window.toast(message));
}

// ------------------------------------------------------------------------------------------------ what the window calls

pub fn refresh_current(page: &str) {
    with_window(|window| match page {
        "history" => window.refresh_history(),
        "automation" => window.refresh_rules(),
        _ => {}
    });
}

pub fn set_theme(theme: &str) {
    with_app(|app| {
        let _ = app.services.settings.set("appearance.theme", json!(theme), true);
    });
    apply_theme(theme);
}

pub fn clear_history() {
    with_app(|app| {
        let _ = app.services.history.clear();
    });
    toast("История очищена");
}

pub fn clear_rules() {
    with_app(|app| {
        let _ = app.learning.clear();
    });
    with_window(MainWindow::refresh_rules);
    toast("Правила удалены");
}

pub fn remove_rule(rule: &LearnedRule) {
    with_app(|app| {
        let _ = app.learning.remove_rule(rule);
    });
    with_window(MainWindow::refresh_rules);
    toast("Правило удалено");
}

pub fn reset_settings() {
    with_app(|app| {
        let _ = app.services.settings.reset();
        app.sync_autostart();
    });
    toast("Настройки сброшены; перезапустите окно для обновления всех полей");
}

pub fn add_application_exclusion(identifier: &str, name: &str) {
    with_window(|window| window.add_application_exclusion(identifier, name));
}

pub fn remove_application_exclusion(identifier: &str) {
    with_window(|window| window.remove_application_exclusion(identifier));
}

pub fn finish_application_capture() {
    with_window(MainWindow::finish_active_application_capture);
}

pub fn open_rule_editor_for(draft: RuleDraft) {
    with_app(|app| app.open_rule_editor(draft, None));
}

/// `_autostart_toggled`: the XDG autostart entry follows the row; a failure puts the row back.
pub fn autostart_toggled(active: bool, row: &adw::SwitchRow) {
    let Some(app) = app() else { return };
    if app.reverting_autostart.get() {
        return;
    }
    let hidden = setting_bool(&app.services.settings, "general.start_hidden", true);
    match app.autostart.set_enabled(active, hidden) {
        Ok(()) => {
            let _ = app.services.settings.set("general.autostart", json!(active), true);
            toast(if active { "Автозапуск включён" } else { "Автозапуск выключен" });
        }
        Err(error) => {
            app.reverting_autostart.set(true);
            row.set_active(!active);
            app.reverting_autostart.set(false);
            toast(&format!("Не удалось изменить автозапуск: {error}"));
        }
    }
}

/// The window's close button: to the tray when the tray is there and the setting says so, else the end of the program.
#[must_use]
pub fn window_close_requested() -> glib::Propagation {
    with_app(|app| {
        if setting_bool(&app.services.settings, "general.close_to_tray", true) && app.tray.borrow().is_some() {
            // Destroyed once this signal has finished, so that the window is not taken apart inside its own handler.
            glib::idle_add_local_once(|| with_app(|app| app.drop_window()));
        } else {
            app.application.quit();
        }
    });
    glib::Propagation::Stop
}

// ------------------------------------------------------------------------------------------------------ the application

fn json_text(value: Option<Value>, default: &str) -> String {
    match value {
        Some(Value::String(text)) => text,
        _ => default.to_owned(),
    }
}

/// What the engine reports of its models, kept for the languages page and the copied report.
pub fn record_models(engine: &KeySwitchEngine, info: &SharedModelInfo) {
    let guard = engine.core();
    let Some(core) = guard.as_ref() else { return };
    let status = &core.intent_model_status;
    let recorded = ModelInfo {
        loaded: true,
        languages: core.models.iter().map(|model| (model.frequencies.len(), model.source.clone())).collect(),
        intent_available: status.available,
        intent_version: status.version.clone().unwrap_or_default(),
        intent_checksum: status.checksum.clone().unwrap_or_default(),
        intent_error: status.error.clone().unwrap_or_default(),
        intent_summary: status.summary(),
        context_status: core.context_policy.status.clone(),
    };
    *info.lock().unwrap_or_else(PoisonError::into_inner) = recorded;
}

/// The caret the offer goes above, searched on the engine's thread while the window the user typed in is still the active
/// one; the pointer is the fallback.
#[must_use]
pub fn prompt_anchor(desktop: &DesktopBackend) -> Option<PromptAnchor> {
    let fallback: Option<ScreenAnchor> = desktop.input.input_anchor();
    let process = anchor_process(fallback.and_then(|anchor| anchor.window), desktop.wayland, &*desktop.process_for_window);
    let caret = atspi_bus::connect().ok().and_then(|api| focused_caret_anchor(&*api, process));
    anchor_of(caret, fallback)
}

/// Whose fields to search: a backend that knows where windows are searches the application of the window the user typed in.
fn anchor_process(window: Option<i64>, wayland: bool, process_of: &dyn Fn(i64) -> u32) -> u32 {
    match window {
        Some(window) if wayland => process_of(window),
        _ => 0,
    }
}

/// The caret if it was found, else the pointer.
fn anchor_of(caret: Option<(i32, i32)>, fallback: Option<ScreenAnchor>) -> Option<PromptAnchor> {
    let window = fallback.and_then(|anchor| anchor.window);
    match (caret, fallback) {
        (Some((x, y)), _) => Some(PromptAnchor { x, y, window }),
        (None, Some(anchor)) => Some(PromptAnchor { x: anchor.x, y: anchor.y, window: anchor.window }),
        (None, None) => None,
    }
}

/// `main`'s GTK half: the status the process ends with.
#[must_use]
pub fn run(hidden: bool, no_engine: bool) -> i32 {
    glib::set_prgname(Some("keyswitch"));
    glib::set_application_name("KeySwitch");
    let application = adw::Application::builder().application_id(APP_ID).flags(gio::ApplicationFlags::FLAGS_NONE).build();
    application.connect_startup(move |application| startup(application, hidden, no_engine));
    application.connect_activate(|_| activate());
    application.connect_shutdown(|_| shutdown());
    i32::from(application.run_with_args(&["keyswitch"]).get())
}

fn startup(application: &adw::Application, hidden: bool, no_engine: bool) {
    let settings = Arc::new(SettingsStore::new(None));
    logsetup::follow_settings(&settings, None);
    let history = Arc::new(HistoryStore::new(
        None,
        history_limit(&settings.get("history.limit").unwrap_or(Value::from(ks_config::constants::DEFAULT_HISTORY_LIMIT))),
    ));
    let learning = match default_learning_store(&settings, &history) {
        Ok(store) => Arc::new(store),
        Err(error) => {
            logsetup::error(LOGGER, &format!("Не удалось открыть правила переключения: {error}"));
            application.quit();
            return;
        }
    };
    let desktop = new_desktop_backend();
    let models: SharedModelInfo = Arc::new(Mutex::new(ModelInfo::default()));
    let build: Build = {
        let inner = engine_builder(settings.clone(), history.clone(), desktop.input.clone(), desktop.process_for_window.clone(), learning.clone());
        let models = models.clone();
        Box::new(move || {
            let engine = inner()?;
            record_models(&engine, &models);
            Ok(engine)
        })
    };
    let enabled = setting_bool(&settings, "enabled", true);
    let post_fn: Post = Arc::new(post);
    // The engine loads its models on a background thread while the window and the tray come up.
    let engine = DeferredEngine::new(build, learning.clone(), enabled, post_fn, true);
    let manager = UpdateManager::with_opener(VERSION, ks_config::history::data_dir().join("updates"), Arc::new(crate::http_curl::CurlOpener)).expect("the program's own version is a version");
    let closing = manager.clone();
    let (updates, close_updates): (Arc<dyn UpdateController>, Box<dyn Fn()>) = (Arc::new(manager), Box::new(move || closing.close()));
    let services = Rc::new(Services {
        settings: settings.clone(),
        history: history.clone(),
        engine: engine.clone(),
        updates: updates.clone(),
        backend: desktop.input.clone(),
        models,
        resource_dir: resource_dir(),
        version: VERSION,
    });
    let this = Rc::new(App {
        application: application.clone(),
        services,
        desktop,
        learning,
        autostart: AutostartManager::new(None),
        close_updates,
        hidden,
        no_engine,
        window: RefCell::new(None),
        tray: RefCell::new(None),
        prompt: RefCell::new(None),
        rule_editor: RefCell::new(None),
        offer_window: Cell::new(None),
        rule_return_window: Cell::new(None),
        initialized: Cell::new(false),
        hold: RefCell::new(None),
        update_initial: RefCell::new(None),
        update_periodic: RefCell::new(None),
        last_update_notification: RefCell::new(String::new()),
        reverting_autostart: Cell::new(false),
    });
    APP.with(|slot| *slot.borrow_mut() = Some(this.clone()));

    for (name, action) in [("show", 0), ("toggle", 1), ("quit", 2)] {
        let simple = gio::SimpleAction::new(name, None);
        simple.connect_activate(move |_, _| {
            with_app(|app| match action {
                0 => app.show_window(),
                1 => app.toggle("enabled", true),
                _ => app.application.quit(),
            });
        });
        application.add_action(&simple);
    }
    apply_theme(&json_text(settings.get("appearance.theme"), "system"));
    this.sync_autostart();
    let _ = settings.subscribe(Arc::new(|path: &str, value: &Value| {
        let (path, value) = (path.to_owned(), value.clone());
        // Applied as the setting stands when the main loop gets to it. When a second change came first, the window, set to
        // the first value, stored it again, and that came back here in turn, back and forth for good. A path the settings
        // no longer hold ("*", or one a reset took away) is applied with the value it was changed to.
        post_app(move |app| {
            let current = app.settings().get(&path).unwrap_or(value);
            app.apply_setting(&path, &current);
        });
    }));
    history.subscribe(Arc::new(|| post_app(|app| app.with_window(MainWindow::refresh_history))));
    updates.subscribe(Arc::new(|snapshot: &UpdateSnapshot| {
        let snapshot = snapshot.clone();
        post_app(move |app| app.apply_update_snapshot(&snapshot));
    }));
    for signal in [libc::SIGINT, libc::SIGTERM] {
        glib_unix::unix_signal_add_local(signal, || {
            with_app(|app| app.application.quit());
            glib::ControlFlow::Break
        });
    }
}

fn activate() {
    with_app(|app| {
        if app.initialized.replace(true) {
            app.show_window();
            return;
        }
        *app.hold.borrow_mut() = Some(app.application.hold());
        app.attach_engine();
        if !app.no_engine {
            let _ = app.services.engine.start(Arc::new(|error: &str| {
                let error = error.to_owned();
                post_app(move |app| app.engine_start_failed(&error));
            }));
        }
        app.sync_tray();
        app.schedule_update_checks();
        if !app.hidden {
            app.show_window();
        }
    });
}

fn shutdown() {
    with_app(|app| {
        app.cancel_update_checks();
        (app.close_updates)();
        app.services.engine.stop();
        if let Some(tray) = app.tray.borrow_mut().take() {
            tray.close();
        }
        if let Some(prompt) = app.prompt.borrow_mut().take() {
            prompt.destroy();
        }
        if let Some(editor) = app.rule_editor.borrow_mut().take() {
            editor.destroy();
        }
        app.drop_window();
        app.hold.borrow_mut().take();
    });
    APP.with(|slot| slot.borrow_mut().take());
}

/// The sound of a correction, on a display if there is one.
pub fn beep_on(display: Option<gdk::Display>) {
    if let Some(display) = display {
        display.beep();
    }
}

/// The check that comes round by itself.
#[must_use]
pub fn periodic_update_check() -> glib::ControlFlow {
    with_app(|app| {
        app.services.updates.check(true, false);
    });
    glib::ControlFlow::Continue
}

impl App {
    fn with_window(&self, action: impl FnOnce(&MainWindow)) {
        if let Some(window) = self.window.borrow().clone() {
            action(&window);
        }
    }

    fn settings(&self) -> &Arc<SettingsStore> {
        &self.services.settings
    }

    fn flag(&self, path: &str, default: bool) -> bool {
        setting_bool(self.settings(), path, default)
    }

    fn ensure_window(&self) -> Rc<MainWindow> {
        if let Some(window) = self.window.borrow().clone() {
            return window;
        }
        let window = MainWindow::new(&self.application, self.services.clone());
        *self.window.borrow_mut() = Some(window.clone());
        window
    }

    pub fn show_window(&self) {
        self.ensure_window().window.present();
    }

    pub fn show_page(&self, page: &str) {
        let window = self.ensure_window();
        window.show_page(page);
        window.window.present();
    }

    /// The window is taken apart so that a program in the tray holds none of its widgets.
    pub fn drop_window(&self) {
        let window = self.window.borrow_mut().take();
        if let Some(window) = window {
            window.flush_pending_saves();
            window.window.destroy();
            drop(window);
            ks_config::memory_release::release_free_memory();
        }
    }

    pub fn toggle(&self, path: &str, default: bool) {
        let _ = self.settings().set(path, json!(!self.flag(path, default)), true);
    }

    pub fn engine_start_failed(&self, error: &str) {
        logsetup::error(LOGGER, &format!("Не удалось запустить движок\n{error}"));
        let window = self.ensure_window();
        window.window.present();
        window.toast(&format!("Глобальная автокоррекция не запущена: {error}"));
    }

    /// The subscriptions to the engine, made once it is built, on the main thread.
    fn attach_engine(&self) {
        let desktop = self.desktop.clone();
        self.services.engine.when_built(Box::new(move |engine: &KeySwitchEngine| {
            engine.subscribe(Arc::new(|snapshot: &EngineSnapshot| {
                let snapshot = snapshot.clone();
                post_app(move |app| app.apply_engine_snapshot(&snapshot));
            }));
            engine.subscribe_corrections(Arc::new(|plan| {
                let (case_fix, original, replacement) = (plan.case_fix(), plan.original.clone(), plan.replacement.clone());
                post_app(move |app| app.announce_correction(case_fix, &original, &replacement));
            }));
            engine.subscribe_learning_prompts(Arc::new(move |prompt: Option<&LearningPrompt>| match prompt.cloned() {
                None => post_app(|app| app.apply_learning_prompt(None, None)),
                Some(prompt) => {
                    let anchor = prompt_anchor(&desktop);
                    post_app(move |app| app.apply_learning_prompt(Some(prompt), anchor));
                }
            }));
            engine.subscribe_rule_requests(Arc::new(|prompt: &LearningPrompt| {
                let draft = draft_from_prompt(prompt);
                // The offer has just closed: the window under it is where the user typed.
                post_app(move |app| {
                    let window = app.offer_window.get();
                    app.open_rule_editor(draft, window);
                });
            }));
            post_app(|app| {
                app.with_window(|window| {
                    window.refresh_models();
                    window.apply_engine_snapshot(&app.services.engine.snapshot());
                });
            });
        }));
    }

    // ------------------------------------------------------------------------------------------------------ the tray

    fn sync_tray(&self) {
        let requested = self.flag("appearance.show_indicator", true);
        let present = self.tray.borrow().is_some();
        if requested && !present {
            let handler: crate::tray::ActionHandler = Arc::new(|action| post_app(move |app| app.tray_action(action)));
            match StatusNotifierItem::new(handler, &self.services.resource_dir) {
                Ok(tray) => {
                    tray.set_indicator_style(&self.settings().get("appearance.indicator_style").unwrap_or_else(|| json!("letters")));
                    let snapshot = self.services.engine.snapshot();
                    tray.set_layout(snapshot.current_group);
                    tray.set_enabled(snapshot.enabled);
                    tray.set_sound_enabled(self.flag("general.sound", false));
                    tray.set_notifications_enabled(self.flag("general.notifications", true));
                    *self.tray.borrow_mut() = Some(tray);
                }
                Err(error) => {
                    logsetup::warning(LOGGER, &format!("Системный индикатор недоступен: {error}"));
                    self.with_window(|window| window.toast(&format!("Системный индикатор недоступен: {error}")));
                }
            }
        } else if !requested && let Some(tray) = self.tray.borrow_mut().take() {
            tray.close();
        }
    }

    pub fn tray_action(&self, action: TrayAction) {
        match action {
            TrayAction::ShowSettings => self.show_window(),
            TrayAction::ToggleEngine => self.toggle("enabled", true),
            TrayAction::SwitchLayout => {
                if !self.services.engine.select_alternate_group() {
                    let message = self.services.engine.snapshot().last_error;
                    self.with_window(|window| window.toast(&message));
                }
            }
            TrayAction::ToggleSound => self.toggle("general.sound", false),
            TrayAction::ToggleNotifications => self.toggle("general.notifications", true),
            TrayAction::ShowHistory => self.show_page("history"),
            TrayAction::ShowExceptions => self.show_page("exceptions"),
            TrayAction::ShowAbout => self.show_page("diagnostics"),
            TrayAction::Quit => self.application.quit(),
        }
    }

    // --------------------------------------------------------------------------------------------------------- settings

    pub fn apply_setting(&self, path: &str, value: &Value) {
        self.with_window(|window| window.apply_setting_update(path, value));
        let truth = crate::linux_app::truthy(value);
        match path {
            "enabled" => self.tray.borrow().iter().for_each(|tray| tray.set_enabled(truth)),
            "general.sound" => self.tray.borrow().iter().for_each(|tray| tray.set_sound_enabled(truth)),
            "general.notifications" => self.tray.borrow().iter().for_each(|tray| tray.set_notifications_enabled(truth)),
            "appearance.show_indicator" => self.sync_tray(),
            "appearance.indicator_style" => self.tray.borrow().iter().for_each(|tray| tray.set_indicator_style(value)),
            "appearance.theme" => apply_theme(value.as_str().unwrap_or("system")),
            "general.autostart" | "general.start_hidden" => self.sync_autostart(),
            "updates.check_automatically" => {
                if truth {
                    self.schedule_update_checks();
                } else {
                    self.cancel_update_checks();
                }
            }
            "*" => {
                // Every setting went back to its default at once: apply each one this handler applies alone.
                for each in [
                    "appearance.show_indicator",
                    "enabled",
                    "general.sound",
                    "general.notifications",
                    "appearance.indicator_style",
                    "appearance.theme",
                    "general.autostart",
                    "updates.check_automatically",
                ] {
                    self.apply_setting(each, &self.settings().get(each).unwrap_or(Value::Null));
                }
            }
            _ => {}
        }
    }

    pub fn sync_autostart(&self) {
        let (enabled, hidden) = (self.flag("general.autostart", true), self.flag("general.start_hidden", true));
        if let Err(error) = self.autostart.set_enabled(enabled, hidden) {
            logsetup::warning(LOGGER, &format!("Не удалось синхронизировать XDG Autostart: {error}"));
        }
    }

    // --------------------------------------------------------------------------------------------------------- updates

    pub fn schedule_update_checks(&self) {
        if self.no_engine
            || !self.flag("updates.check_automatically", true)
            || self.update_initial.borrow().is_some()
            || self.update_periodic.borrow().is_some()
        {
            return;
        }
        let source = glib::timeout_add_seconds_local_once(UPDATE_CHECK_INITIAL_DELAY_SECONDS.low::<u32>(), || with_app(|app| app.first_update_check()));
        *self.update_initial.borrow_mut() = Some(source);
    }

    /// The check a while after the start; from then on it comes round by itself, if it should.
    pub fn first_update_check(&self) {
        self.update_initial.borrow_mut().take();
        self.services.updates.check(true, false);
        if self.flag("updates.check_automatically", true) {
            let periodic = glib::timeout_add_seconds_local(UPDATE_CHECK_INTERVAL_SECONDS.low::<u32>(), periodic_update_check);
            *self.update_periodic.borrow_mut() = Some(periodic);
        }
    }

    pub fn cancel_update_checks(&self) {
        for slot in [&self.update_initial, &self.update_periodic] {
            if let Some(source) = slot.borrow_mut().take() {
                source.remove();
            }
        }
    }

    pub fn apply_update_snapshot(&self, snapshot: &UpdateSnapshot) {
        self.with_window(|window| window.apply_update_snapshot(snapshot));
        if snapshot.phase == UpdatePhase::Available && *self.last_update_notification.borrow() != snapshot.available_version {
            (*self.last_update_notification.borrow_mut()).clone_from(&snapshot.available_version);
            if self.flag("general.notifications", true) {
                let notification = gio::Notification::new("Доступно обновление KeySwitch");
                notification.set_body(Some(&snapshot.message));
                notification.set_icon(&gio::ThemedIcon::new("keyswitch"));
                notification.set_default_action("app.show");
                self.application.send_notification(Some("keyswitch-update"), &notification);
            }
        }
    }

    // ----------------------------------------------------------------------------------------------------- the engine

    pub fn apply_engine_snapshot(&self, snapshot: &EngineSnapshot) {
        if let Some(tray) = self.tray.borrow().as_ref() {
            tray.set_layout(snapshot.current_group);
            tray.set_enabled(snapshot.enabled);
        }
        self.with_window(|window| window.apply_engine_snapshot(snapshot));
    }

    pub fn announce_correction(&self, case_fix: bool, original: &str, replacement: &str) {
        if self.flag("general.notifications", true) {
            let notification = gio::Notification::new(if case_fix { "Регистр исправлен" } else { "Раскладка исправлена" });
            notification.set_body(Some(&format!("{original}  →  {replacement}")));
            notification.set_icon(&gio::ThemedIcon::new("keyswitch"));
            self.application.send_notification(None, &notification);
        }
        if self.flag("general.sound", false) {
            beep_on(gdk::Display::default());
        }
        if let Some(window) = self.window.borrow().as_ref()
            && window.window.is_visible()
        {
            window.toast(&format!("Исправлено: {original} → {replacement}"));
        }
    }

    // ------------------------------------------------------------------------------------------------ the rule windows

    pub fn apply_learning_prompt(&self, prompt: Option<LearningPrompt>, anchor: Option<PromptAnchor>) {
        let Some(prompt) = prompt else {
            if let Some(window) = self.prompt.borrow().as_ref() {
                self.offer_window.set(window.anchor_window());
                window.hide_prompt();
            }
            return;
        };
        if self.prompt.borrow().is_none() {
            let (confirm, dismiss) = (self.services.engine.clone(), self.services.engine.clone());
            let placer = self.desktop.position_window.clone();
            let restorer = self.desktop.input.clone();
            let callbacks = PromptCallbacks {
                confirm: Box::new(move |prompt| confirm.confirm_learning_prompt(Some(prompt))),
                dismiss: Box::new(move |prompt| dismiss.dismiss_learning_prompt(Some(prompt))),
                position: Box::new(move |window, x, y| placer(window, x, y)),
                restore: Box::new(move |window| restorer.restore_window(window)),
            };
            *self.prompt.borrow_mut() = Some(LearningPromptWindow::new(self.application.upcast_ref(), callbacks));
        }
        self.prompt.borrow().iter().for_each(|window| window.show_prompt(prompt.clone(), anchor));
    }

    /// Shows the rule window filled in with `draft`; one at a time. `return_window` gets the keyboard back when it closes.
    pub fn open_rule_editor(&self, draft: RuleDraft, return_window: Option<i64>) {
        if let Some(editor) = self.rule_editor.borrow().as_ref() {
            editor.present();
            return;
        }
        self.rule_return_window.set(return_window);
        let engine = self.services.engine.clone();
        let flagged = self.services.engine.clone();
        let callbacks = RuleCallbacks {
            accept: Box::new(move |draft| accept_rule(&engine, draft)),
            closed: Box::new(|| post_app(|app| app.rule_editor_closed())),
            focused: Box::new(move |focused| flagged.set_rule_editor_open(focused)),
        };
        let editor = RuleEditorWindow::new(self.application.upcast_ref(), draft, callbacks);
        editor.present();
        *self.rule_editor.borrow_mut() = Some(editor);
    }

    pub fn rule_editor_closed(&self) {
        self.rule_editor.borrow_mut().take();
        if let Some(window) = self.rule_return_window.take() {
            self.desktop.input.restore_window(Some(window));
        }
    }
}

/// `_accept_rule`: an empty answer when the rule was stored, else what the window shows instead of closing.
fn accept_rule(engine: &DeferredEngine, draft: &RuleDraft) -> String {
    let rule = match build_rule(draft) {
        Ok(rule) => rule,
        Err(error) => return error.to_string(),
    };
    if let Err(error) = engine.add_learning_rule(rule, draft.editing.as_ref(), &draft.application) {
        return format!("Правило не сохранено: {error}");
    }
    with_window(MainWindow::refresh_rules);
    String::new()
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn a_setting_that_is_no_text_reads_as_the_default() {
        assert_eq!(json_text(Some(json!("dark")), "system"), "dark");
        assert_eq!(json_text(Some(json!(5)), "system"), "system");
        assert_eq!(json_text(None, "system"), "system");
    }

    #[test]
    fn the_offer_goes_above_the_caret_or_else_the_pointer() {
        let at = |window| ScreenAnchor { x: 1, y: 2, window };
        assert_eq!(anchor_process(Some(7), true, &|window| u32::try_from(window).unwrap() + 1), 8);
        assert_eq!(anchor_process(Some(7), false, &|_| 9), 0);
        assert_eq!(anchor_process(None, true, &|_| 9), 0);
        let caret = anchor_of(Some((5, 6)), Some(at(Some(3)))).unwrap();
        assert_eq!((caret.x, caret.y, caret.window), (5, 6, Some(3)));
        let pointer = anchor_of(None, Some(at(None))).unwrap();
        assert_eq!((pointer.x, pointer.y, pointer.window), (1, 2, None));
        assert!(anchor_of(None, None).is_none());
        assert!(anchor_of(Some((5, 6)), None).is_some());
    }
}
