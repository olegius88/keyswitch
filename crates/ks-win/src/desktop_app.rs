//! The Windows application: the window that owns the tray icon, the engine, and their lifecycle (the application part of
//! the Python's `desktop_ui.DesktopApplication`, `windows_ui.py` and `keyswitch_windows_entry.py`).
//!
//! One thread, the main one, runs the message loop and owns every window and the tray. Other threads (the engine's worker,
//! the model loader, the settings store) hand it work through `post`, which queues a closure and wakes the loop with a
//! message. The window itself, its pages, the learning prompt and the rule window are in `settings_window`,
//! `learning_prompt` and `rule_editor_dialog`; this file wires them to the engine, the stores and the tray.

#![cfg_attr(windows, expect(unsafe_code, reason = "calls into Win32"))]
use ks_cast::To as _;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ptr::{null, null_mut};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use ks_config::config::SettingsStore;
use ks_config::history::{HistoryStore, history_limit};
use ks_config::indicator::normalize_indicator_style;
use ks_config::logsetup::{self, LocalTime};
use ks_engine::context_access::ReaderFactory;
use ks_engine::engine_loader::{Build, DeferredEngine, Post};
use ks_backend::backend::{InputBackend, ScreenAnchor};
use ks_engine::rule_editor::draft_from_prompt;
use ks_engine::{CorrectionPlan, EngineSnapshot, KeySwitchEngine, LearningPrompt, engine::default_learning_store};
use serde_json::Value;
use windows_sys::Wdk::System::SystemServices::RtlGetVersion;
use windows_sys::Win32::Foundation::{GENERIC_WRITE, HWND, INVALID_HANDLE_VALUE, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_WRITE, OPEN_EXISTING, WriteFile};
use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE};
use windows_sys::Win32::System::SystemInformation::{GetLocalTime, OSVERSIONINFOW};
use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
use windows_sys::Win32::System::Threading::ExitProcess;
use windows_sys::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    ChangeWindowMessageFilterEx, DestroyWindow, DispatchMessageW, GetMessageW, KillTimer, MB_OK, MSG, MSGFLT_ALLOW,
    PostMessageW, PostQuitMessage, RegisterWindowMessageW, SW_HIDE, SW_RESTORE, SetForegroundWindow, SetTimer, ShowWindow,
    TranslateMessage, WM_CLOSE, WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_LBUTTONUP, WM_RBUTTONUP, WM_SETTINGCHANGE,
    WM_TIMER,
};

use crate::constants::{
    BACKEND_LABEL, ENGINE_STOP_WAIT_SECONDS, LEARNING_PROMPT_SHOW_DELAY_MS, SMOKE_UI_QUIT_AFTER_MS, UPDATE_CHECK_INITIAL_DELAY_MS,
    UPDATE_CHECK_INTERVAL_MS, WM_APP, WM_TRAY,
};
use crate::learning_prompt;
use crate::settings_window::{self, Services};
use crate::windows_ui_model::{NoUpdates, UpdateController, UpdatePhase, UpdateSnapshot};
use crate::tray::Tray;
use crate::tray_model::TrayAction;
use crate::windows_app::{Arguments, VERSION, resource_dir};
use crate::windows_context::WindowsFieldReader;
use crate::windows_backend::WindowsBackend;
use crate::windows_instance::WindowsSingleInstance;
use crate::windows_native::wide;
use crate::windows_system::{WindowsApplicationCatalog, WindowsAutostartManager};

const LOGGER: &str = "keyswitch.desktop_ui";
const WM_POSTED: u32 = WM_APP + 1;
const SMOKE_TIMER_ID: usize = 1;
const PROMPT_TIMER_ID: usize = 2;
const UPDATE_TIMER_ID: usize = 3;

type Posted = Box<dyn FnOnce() + Send>;

static POSTED: Mutex<VecDeque<Posted>> = Mutex::new(VecDeque::new());
static WINDOW: AtomicIsize = AtomicIsize::new(0);
static CLOSING: AtomicBool = AtomicBool::new(false);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

thread_local! {
    static APP: RefCell<Option<Rc<App>>> = const { RefCell::new(None) };
}

/// The application, from the thread that owns it; none before it is built and after it is gone.
fn with_app(action: impl FnOnce(&Rc<App>)) {
    let app = APP.with(|slot| slot.borrow().clone());
    if let Some(app) = app {
        action(&app);
    }
}

/// `_post`: run `action` on the window's thread. Nothing is queued once the program is quitting.
fn post(action: Posted) {
    if CLOSING.load(Ordering::SeqCst) {
        return;
    }
    POSTED.lock().unwrap_or_else(PoisonError::into_inner).push_back(action);
    let window = WINDOW.load(Ordering::SeqCst);
    if window != 0 {
        // SAFETY: PostMessageW may be called from any thread; a window that is gone just fails the call.
        unsafe { PostMessageW(window.pointer_mut::<std::ffi::c_void>(), WM_POSTED, 0, 0) };
    }
}

fn drain_posted() {
    loop {
        let next = POSTED.lock().unwrap_or_else(PoisonError::into_inner).pop_front();
        match next {
            Some(action) => action(),
            None => break,
        }
    }
}

// ------------------------------------------------------------------------------------------------------ the platform

fn local_time() -> LocalTime {
    // SAFETY: GetLocalTime fills the structure it is given.
    let time = unsafe {
        let mut time = std::mem::zeroed();
        GetLocalTime(&raw mut time);
        time
    };
    LocalTime {
        year: time.wYear.to::<i32>(),
        month: time.wMonth.to::<u32>(),
        day: time.wDay.to::<u32>(),
        hour: time.wHour.to::<u32>(),
        minute: time.wMinute.to::<u32>(),
        second: time.wSecond.to::<u32>(),
        millisecond: time.wMilliseconds.to::<u32>(),
    }
}

/// `platform.platform()` as Python words it on Windows: `Windows-11-10.0.26100-SP0`.
fn platform_description() -> String {
    // SAFETY: RtlGetVersion fills the structure whose size it is told.
    let (major, minor, build) = unsafe {
        let mut info: OSVERSIONINFOW = std::mem::zeroed();
        info.dwOSVersionInfoSize = (size_of::<OSVERSIONINFOW>()).low::<u32>();
        RtlGetVersion(&raw mut info);
        (info.dwMajorVersion, info.dwMinorVersion, info.dwBuildNumber)
    };
    let release = if major >= 10 && build >= 22000 { "11" } else if major >= 10 { "10" } else { "8" };
    format!("Windows-{release}-{major}.{minor}.{build}-SP0")
}

/// The clock and system description of the log, the log sink of this crate, and the process's DPI awareness (per monitor).
pub fn install_platform() {
    logsetup::set_local_clock(local_time);
    logsetup::set_platform_description(&platform_description());
    crate::log::set_sink(|level, logger, message| {
        let level = match level {
            crate::log::Level::Info => logsetup::Level::Info,
            crate::log::Level::Error => logsetup::Level::Error,
        };
        logsetup::log(level, logger, message);
    });
    // SAFETY: a process-wide switch; the manifest asks for the same awareness, and a refusal because it is already set is
    // harmless.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
}

/// Standard output or error. A windowed program has none of its own unless it was started with them redirected, so a
/// console it was started from is attached to, and its screen buffer written to.
pub fn write_standard(error: bool, text: &str) {
    let which = if error { STD_ERROR_HANDLE } else { STD_OUTPUT_HANDLE };
    // SAFETY: the handles are used for one write and the console one is closed by the process's end; the buffer outlives
    // the call.
    unsafe {
        let mut handle = GetStdHandle(which);
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            AttachConsole(ATTACH_PARENT_PROCESS);
            let name = wide("CONOUT$");
            handle = CreateFileW(name.as_ptr(), GENERIC_WRITE, FILE_SHARE_WRITE, null(), OPEN_EXISTING, 0, null_mut());
            if handle == INVALID_HANDLE_VALUE {
                return;
            }
        }
        let mut written = 0u32;
        WriteFile(handle, text.as_ptr(), text.len().low::<u32>(), &raw mut written, null_mut());
    }
}

/// `leave`: end the process without waiting for a thread that is still loading models. The log is closed first.
pub fn leave(status: i32) -> ! {
    logsetup::uninstall_handler();
    // SAFETY: ends the process.
    unsafe { ExitProcess(status.bits::<u32>()) }
}

// -------------------------------------------------------------------------------------------------------- the program

/// `main` after the arguments: one instance at a time, then the application.
#[must_use]
pub fn run_with_instance(arguments: Arguments) -> i32 {
    let mut instance = match WindowsSingleInstance::new(None) {
        Ok(instance) => instance,
        Err(error) => {
            logsetup::error(LOGGER, &format!("Не удалось создать single-instance: {error}"));
            return 1;
        }
    };
    match instance.acquire() {
        Ok(true) => {}
        Ok(false) => {
            instance.activate_existing();
            instance.close();
            return 0;
        }
        Err(error) => {
            logsetup::error(LOGGER, &format!("Не удалось занять single-instance: {error}"));
            return 1;
        }
    }
    let status = if arguments.smoke_ui {
        run_application(false, true, Some(SMOKE_UI_QUIT_AFTER_MS))
    } else {
        run_application(arguments.hidden, arguments.no_engine, None)
    };
    instance.close();
    status
}

struct App {
    window: HWND,
    backend: Arc<WindowsBackend>,
    settings: Arc<SettingsStore>,
    history: Arc<HistoryStore>,
    engine: Arc<DeferredEngine>,
    updates: Arc<dyn UpdateController>,
    enable_updates: bool,
    autostart: Option<WindowsAutostartManager>,
    tray: RefCell<Option<Tray>>,
    closing: Cell<bool>,
    /// The offer waiting for its short delay before it is shown.
    pending_prompt: RefCell<Option<(LearningPrompt, Option<ScreenAnchor>)>>,
    prompt_callbacks: Rc<learning_prompt::Callbacks>,
    last_update_notification: RefCell<String>,
}

fn setting_bool(settings: &SettingsStore, path: &str, default: bool) -> bool {
    settings.get(path).map_or(default, |value| match value {
        Value::Bool(flag) => flag,
        Value::Null => false,
        Value::Number(number) => number.as_f64().is_some_and(|number| number != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
    })
}

impl App {
    fn present(&self) {
        // Windows does not tell a window that the system went light or dark: one that follows it asks again when shown.
        settings_window::refresh_theme();
        // SAFETY: the window is ours; SW_RESTORE shows a hidden or minimised window and activates it.
        unsafe {
            ShowWindow(self.window, SW_RESTORE);
            SetForegroundWindow(self.window);
        }
    }

    fn hide(&self) {
        // SAFETY: as above.
        unsafe { ShowWindow(self.window, SW_HIDE) };
    }

    fn set_error(&self, text: &str) {
        settings_window::set_error(text);
    }

    fn apply_snapshot(&self, snapshot: EngineSnapshot) {
        if let Some(tray) = self.tray.borrow().as_ref() {
            let mut state = tray.state();
            state.group = snapshot.current_group;
            state.enabled = snapshot.enabled;
            tray.set_state(state);
        }
        settings_window::apply_snapshot(&snapshot);
    }

    fn show_page_and_present(&self, name: &str) {
        settings_window::show_page(name);
        self.present();
    }

    /// `_apply_update_snapshot`'s part that is not drawing: a found update is announced once through the tray.
    fn apply_update_snapshot(&self, snapshot: UpdateSnapshot) {
        settings_window::apply_update_snapshot(&snapshot);
        if snapshot.phase == UpdatePhase::Available && *self.last_update_notification.borrow() != snapshot.available_version {
            (*self.last_update_notification.borrow_mut()).clone_from(&snapshot.available_version);
            if setting_bool(&self.settings, "general.notifications", true)
                && let Some(tray) = self.tray.borrow().as_ref()
            {
                tray.notify("Доступно обновление KeySwitch", &snapshot.message);
            }
        }
    }

    /// `_sync_update_schedule`: one timer, for the first check after the start and then for each one after it.
    fn sync_update_schedule(&self, initial: bool) {
        // SAFETY: the timer belongs to the window made by this thread; killing one that is not set is harmless.
        unsafe { KillTimer(self.window, UPDATE_TIMER_ID) };
        let requested = self.enable_updates && !self.closing.get() && setting_bool(&self.settings, "updates.check_automatically", true);
        if requested {
            let delay = if initial { UPDATE_CHECK_INITIAL_DELAY_MS } else { UPDATE_CHECK_INTERVAL_MS };
            // SAFETY: as above.
            unsafe { SetTimer(self.window, UPDATE_TIMER_ID, delay, None) };
        }
    }

    fn automatic_update_check(&self) {
        // SAFETY: as above.
        unsafe { KillTimer(self.window, UPDATE_TIMER_ID) };
        self.updates.check(true, setting_bool(&self.settings, "updates.install_automatically", true));
        self.sync_update_schedule(false);
    }

    /// `_schedule_learning_prompt`: the offer is shown a moment after the correction that raised it.
    fn schedule_learning_prompt(&self, prompt: LearningPrompt, anchor: Option<ScreenAnchor>) {
        *self.pending_prompt.borrow_mut() = Some((prompt, anchor));
        // SAFETY: the timer belongs to the window made by this thread; setting it again restarts it.
        unsafe { SetTimer(self.window, PROMPT_TIMER_ID, LEARNING_PROMPT_SHOW_DELAY_MS, None) };
    }

    /// `_apply_learning_prompt`: `None` takes the card away; an offer the engine no longer holds is not shown.
    fn apply_learning_prompt(&self, prompt: Option<(LearningPrompt, Option<ScreenAnchor>)>) {
        let Some((prompt, anchor)) = prompt else {
            // SAFETY: as above.
            unsafe { KillTimer(self.window, PROMPT_TIMER_ID) };
            self.pending_prompt.borrow_mut().take();
            if let Some((window, callbacks)) = learning_prompt::hide()
                && window.is_some()
            {
                (callbacks.restore)(window);
            }
            return;
        };
        if self.engine.learning_prompt().as_ref() != Some(&prompt) {
            return;
        }
        learning_prompt::show(&prompt, anchor, self.prompt_callbacks.clone());
    }

    /// The installer was started by the update manager's request: the program ends so that it can replace it, or the
    /// manager is told it failed.
    fn install_launched(&self, result: Result<(), String>) {
        match result {
            Ok(()) => self.shutdown(),
            Err(error) => {
                logsetup::error(LOGGER, &format!("Не удалось запустить установщик KeySwitch: {error}"));
                self.updates.installation_failed(&error);
            }
        }
    }

    fn announce_correction(&self, plan: &CorrectionPlan) {
        if setting_bool(&self.settings, "general.notifications", true)
            && let Some(tray) = self.tray.borrow().as_ref()
        {
            let title = if plan.case_fix() { "Регистр исправлен" } else { "Раскладка исправлена" };
            tray.notify(title, &format!("{}  →  {}", plan.original, plan.replacement));
        }
        if setting_bool(&self.settings, "general.sound", false) {
            // SAFETY: plays the system alert sound.
            unsafe { MessageBeep(MB_OK) };
        }
    }

    fn sync_autostart(&self, user_request: bool) {
        let Some(manager) = self.autostart.as_ref() else { return };
        let requested = setting_bool(&self.settings, "general.autostart", true);
        let result = manager
            .set_enabled(requested, setting_bool(&self.settings, "general.start_hidden", true), user_request)
            .and_then(|()| manager.status());
        let status = match result {
            Ok(status) => status,
            Err(error) => {
                logsetup::warning(LOGGER, &format!("Не удалось синхронизировать автозагрузку Windows: {error}"));
                self.set_error(&format!("Автозагрузка: {error}"));
                return;
            }
        };
        if requested && status.blocked_by_windows {
            logsetup::warning(LOGGER, "Автозагрузка отключена в параметрах автозапуска Windows");
            self.set_error(
                "Автозагрузка выключена в Windows: включите KeySwitch в «Диспетчер задач → Автозагрузка приложений» или переключите здесь ещё раз.",
            );
        } else if requested && status.target_missing {
            logsetup::warning(
                LOGGER,
                &format!("Команда автозагрузки указывает на отсутствующий файл: {}", status.command.clone().unwrap_or_default()),
            );
            self.set_error("Автозагрузка указывает на другой каталог установки; переключите её заново.");
        }
    }

    fn sync_tray_state(&self) {
        let tray = self.tray.borrow();
        let Some(tray) = tray.as_ref() else { return };
        let mut state = tray.state();
        let snapshot = self.engine.snapshot();
        state.group = snapshot.current_group;
        state.enabled = snapshot.enabled;
        state.sound_enabled = setting_bool(&self.settings, "general.sound", false);
        state.notifications_enabled = setting_bool(&self.settings, "general.notifications", true);
        state.indicator_style = normalize_indicator_style(
            &self.settings.get("appearance.indicator_style").unwrap_or_else(|| Value::String("letters".into())),
        );
        tray.set_state(state);
    }

    fn sync_tray(&self) {
        let requested = setting_bool(&self.settings, "appearance.show_indicator", true);
        let exists = self.tray.borrow().is_some();
        if requested && !exists {
            *self.tray.borrow_mut() = Some(Tray::new(self.window, WM_TRAY));
            self.sync_tray_state();
        } else if !requested && exists {
            if let Some(tray) = self.tray.borrow_mut().take() {
                tray.close();
            }
            // Without the icon the window is the only way back in.
            self.present();
        }
    }

    fn apply_setting(&self, path: &str, value: &Value) {
        settings_window::apply_setting(path, value);
        match path {
            "*" => {
                self.history.set_limit(history_limit(&self.settings.get("history.limit").unwrap_or(Value::Null)));
                self.sync_autostart(false);
                self.sync_tray();
                self.sync_tray_state();
                self.sync_update_schedule(true);
            }
            "updates.check_automatically" => self.sync_update_schedule(true),
            "general.autostart" | "general.start_hidden" => self.sync_autostart(true),
            "appearance.show_indicator" => self.sync_tray(),
            "appearance.indicator_style" | "enabled" | "general.sound" | "general.notifications" => self.sync_tray_state(),
            "history.limit" => self.history.set_limit(history_limit(value)),
            _ => {}
        }
    }

    fn toggle(&self, path: &str, default: bool) {
        let flag = !setting_bool(&self.settings, path, default);
        if let Err(error) = self.settings.set(path, Value::Bool(flag), true) {
            logsetup::warning(LOGGER, &format!("Не удалось сохранить настройку {path}: {error}"));
        }
    }

    fn tray_action(&self, action: TrayAction) {
        match action {
            TrayAction::ShowSettings => self.present(),
            TrayAction::ShowHistory => self.show_page_and_present("history"),
            TrayAction::ShowExclusions => self.show_page_and_present("exclusions"),
            TrayAction::ShowAbout => self.show_page_and_present("about"),
            TrayAction::SwitchLayout => {
                // The menu has taken the front from the window the user types in: the layout goes to that window.
                self.backend.aim_next_switch_at_work_window(true);
                if !self.engine.select_alternate_group() {
                    self.backend.aim_next_switch_at_work_window(false);
                    let error = self.engine.snapshot().last_error;
                    self.set_error(&error);
                }
            }
            TrayAction::ToggleEngine => self.toggle("enabled", true),
            TrayAction::ToggleSound => self.toggle("general.sound", false),
            TrayAction::ToggleNotifications => self.toggle("general.notifications", true),
            // Only macOS waits for a permission; the menu of Windows never offers its pane.
            TrayAction::OpenPermissionSettings => {}
            TrayAction::QuitApplication => self.shutdown(),
        }
    }

    fn show_menu(&self) {
        let action = {
            let tray = self.tray.borrow();
            tray.as_ref().and_then(Tray::popup_menu)
        };
        if let Some(action) = action {
            self.tray_action(action);
        }
    }

    fn window_close(&self) {
        if setting_bool(&self.settings, "general.close_to_tray", true) && self.tray.borrow().is_some() {
            self.hide();
        } else {
            self.shutdown();
        }
    }

    fn engine_start_failed(&self, error: &str) {
        logsetup::error(LOGGER, &format!("Не удалось запустить клавиатурный слой\n{error}"));
        self.set_error(error);
        self.present();
    }

    /// `shutdown`: stop the engine (waiting a bounded time for it), take the icon away, close the window.
    fn shutdown(&self) {
        if self.closing.replace(true) {
            return;
        }
        CLOSING.store(true, Ordering::SeqCst);
        self.sync_update_schedule(false);
        let (done, finished) = std::sync::mpsc::channel();
        let engine = self.engine.clone();
        let stopper = std::thread::Builder::new().name("KeySwitchEngineStop".to_owned()).spawn(move || {
            engine.stop();
            let _ = done.send(());
        });
        if stopper.is_err() || finished.recv_timeout(Duration::from_secs_f64(ENGINE_STOP_WAIT_SECONDS)).is_err() {
            logsetup::error(LOGGER, "Ошибка остановки клавиатурного слоя: движок не остановился вовремя");
        }
        if let Some(tray) = self.tray.borrow_mut().take() {
            tray.close();
        }
        // SAFETY: the window is ours and this is its thread.
        unsafe { DestroyWindow(self.window) };
    }
}

// ---------------------------------------------------------------------------------------------------------- the window

/// The messages of the frame that are the application's: the queue of posted work, the tray, the timers, closing, and the
/// shell's notice that Explorer restarted. The rest is the settings window's own.
fn host_message(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    match message {
        WM_POSTED => {
            drain_posted();
            Some(0)
        }
        WM_TRAY => {
            if lparam.low::<u32>() == WM_LBUTTONUP || lparam.low::<u32>() == WM_RBUTTONUP {
                // A primary click opens the same complete menu as the secondary one.
                with_app(|app| app.show_menu());
            }
            Some(0)
        }
        WM_TIMER if wparam == SMOKE_TIMER_ID => {
            // SAFETY: the timer belongs to this window.
            unsafe { KillTimer(window, SMOKE_TIMER_ID) };
            with_app(|app| app.shutdown());
            Some(0)
        }
        WM_TIMER if wparam == PROMPT_TIMER_ID => {
            // SAFETY: as above.
            unsafe { KillTimer(window, PROMPT_TIMER_ID) };
            with_app(|app| {
                let pending = app.pending_prompt.borrow_mut().take();
                if pending.is_some() {
                    app.apply_learning_prompt(pending);
                }
            });
            Some(0)
        }
        WM_TIMER if wparam == UPDATE_TIMER_ID => {
            with_app(|app| app.automatic_update_check());
            Some(0)
        }
        WM_CLOSE => {
            let mut handled = false;
            with_app(|app| {
                app.window_close();
                handled = true;
            });
            if !handled {
                // SAFETY: the window is ours.
                unsafe { DestroyWindow(window) };
            }
            Some(0)
        }
        WM_DESTROY => {
            settings_window::destroy_state();
            // SAFETY: ends this thread's message loop.
            unsafe { PostQuitMessage(0) };
            Some(0)
        }
        WM_DPICHANGED | WM_DISPLAYCHANGE | WM_SETTINGCHANGE => {
            // The scale of the icon's monitor may have changed, or the taskbar moved to another monitor. The window's own
            // handling goes on after this.
            with_app(|app| {
                if let Some(tray) = app.tray.borrow().as_ref() {
                    tray.refresh_size();
                }
            });
            None
        }
        _ if message == TASKBAR_CREATED.load(Ordering::SeqCst) && message != 0 => {
            // Explorer restarted: the shell has forgotten the icon, and the taskbar may be on a monitor of another scale.
            with_app(|app| {
                if let Some(tray) = app.tray.borrow().as_ref() {
                    tray.add();
                    tray.refresh_size();
                }
            });
            Some(0)
        }
        _ => None,
    }
}

/// The window the tray reports to and the update installer's request go through: once the update manager has started the
/// installer (`launch`), the program ends so that it can replace it.
pub fn request_install(launch: Box<dyn FnOnce() -> Result<(), String> + Send>) {
    post(Box::new(move || {
        let result = launch();
        with_app(|app| app.install_launched(result));
    }));
}

fn register_taskbar_message(window: HWND) {
    let created = wide("TaskbarCreated");
    // SAFETY: the name is NUL-terminated; the filter call takes the window and the message by value.
    unsafe {
        TASKBAR_CREATED.store(RegisterWindowMessageW(created.as_ptr()), Ordering::SeqCst);
        // An elevated program is otherwise not told that Explorer restarted.
        ChangeWindowMessageFilterEx(window, TASKBAR_CREATED.load(Ordering::SeqCst), MSGFLT_ALLOW, null_mut());
    }
}

// ---------------------------------------------------------------------------------------------------------- the wiring

fn engine_builder(
    settings: Arc<SettingsStore>,
    history: Arc<HistoryStore>,
    backend: Arc<WindowsBackend>,
    learning: Arc<ks_config::learning::LearningStore>,
) -> Build {
    let resources = ks_lang::Resources::new(resource_dir());
    Box::new(move || {
        let factory: ReaderFactory = Arc::new(|| {
            WindowsFieldReader::new().map(|reader| -> Box<dyn ks_backend::ManagedFieldReader> { Box::new(reader) })
        });
        KeySwitchEngine::new(
            settings,
            history,
            backend,
            Some(learning),
            BACKEND_LABEL,
            None,
            Some(factory),
            &resources,
            ks_context::input_context::monotonic_clock(),
        )
    })
}

/// `run_windows_application`: build everything, run the loop until the window is destroyed, and return the status.
fn run_application(hidden: bool, no_engine: bool, quit_after_ms: Option<u32>) -> i32 {
    let settings = Arc::new(SettingsStore::new(None));
    logsetup::follow_settings(&settings, None);
    let history = Arc::new(HistoryStore::new(
        None,
        history_limit(&settings.get("history.limit").unwrap_or(Value::from(ks_config::constants::DEFAULT_HISTORY_LIMIT))),
    ));
    let backend = match WindowsBackend::new(None) {
        Ok(backend) => Arc::new(backend),
        Err(error) => {
            logsetup::error(LOGGER, &format!("Клавиатурный слой недоступен: {error}"));
            return 1;
        }
    };
    let learning = match default_learning_store(&settings, &history) {
        Ok(store) => Arc::new(store),
        Err(error) => {
            logsetup::error(LOGGER, &format!("Не удалось открыть правила переключения: {error}"));
            return 1;
        }
    };
    let enabled = setting_bool(&settings, "enabled", true);
    let post_fn: Post = Arc::new(post);
    // The engine loads its models on a background thread while the window and the tray come up.
    let engine = DeferredEngine::new(engine_builder(settings.clone(), history.clone(), backend.clone(), learning.clone()), learning, enabled, post_fn, true);
    let autostart = match WindowsAutostartManager::new(None, None, None) {
        Ok(manager) => Some(manager),
        Err(error) => {
            logsetup::warning(LOGGER, &format!("Автозагрузка недоступна: {error}"));
            None
        }
    };
    // The update manager is the updater's; until it is wired in, nothing is ever found.
    let updates: Arc<dyn UpdateController> = match ks_updates::UpdateManager::with_opener(
        VERSION,
        ks_config::history::data_dir().join("updates"),
        Arc::new(crate::winhttp::WinHttpOpener),
    ) {
        Ok(manager) => Arc::new(manager.with_install_request(Arc::new(|path: &std::path::Path| {
            let path = path.to_path_buf();
            request_install(Box::new(move || {
                crate::winhttp::launch_windows_installer(&path).map_err(|error| error.to_string())
            }));
            Ok(())
        }))),
        Err(error) => {
            logsetup::warning(LOGGER, &format!("Обновления недоступны: {error}"));
            Arc::new(NoUpdates { version: VERSION.to_owned() })
        }
    };
    let input_backend: Arc<dyn InputBackend> = backend.clone();
    let services = Services {
        settings: settings.clone(),
        history: history.clone(),
        engine: engine.clone(),
        backend: input_backend.clone(),
        updates: updates.clone(),
        catalog: WindowsApplicationCatalog::new(None).ok(),
        autostart_report: Box::new(crate::windows_app::autostart_status),
        host: host_message,
    };
    let Some(window) = settings_window::create(services) else {
        logsetup::error(LOGGER, "Не удалось создать окно");
        return 1;
    };
    register_taskbar_message(window);
    let prompt_engine = engine.clone();
    let dismiss_engine = engine.clone();
    let keep_backend = input_backend.clone();
    let restore_backend = input_backend.clone();
    let prompt_callbacks = Rc::new(learning_prompt::Callbacks {
        confirm: Box::new(move |prompt| prompt_engine.confirm_learning_prompt(Some(prompt))),
        dismiss: Box::new(move |prompt| dismiss_engine.dismiss_learning_prompt(Some(prompt))),
        keep_inactive: Box::new(move |handle| keep_backend.keep_window_inactive(handle)),
        restore: Box::new(move |handle| restore_backend.restore_window(handle)),
    });
    let app = Rc::new(App {
        window,
        backend,
        settings: settings.clone(),
        history: history.clone(),
        engine: engine.clone(),
        updates: updates.clone(),
        enable_updates: quit_after_ms.is_none(),
        autostart,
        tray: RefCell::new(None),
        closing: Cell::new(false),
        pending_prompt: RefCell::new(None),
        prompt_callbacks,
        last_update_notification: RefCell::new(String::new()),
    });
    APP.with(|slot| *slot.borrow_mut() = Some(app.clone()));
    WINDOW.store(window.to::<isize>(), Ordering::SeqCst);

    let _ = settings.subscribe(Arc::new(|path: &str, value: &Value| {
        let (path, value) = (path.to_owned(), value.clone());
        post(Box::new(move || with_app(|app| app.apply_setting(&path, &value))));
    }));
    history.subscribe(Arc::new(|| post(Box::new(settings_window::refresh_history))));
    updates.subscribe(Arc::new(|snapshot: &UpdateSnapshot| {
        let snapshot = snapshot.clone();
        post(Box::new(move || with_app(|app| app.apply_update_snapshot(snapshot))));
    }));
    let anchor_backend = input_backend.clone();
    let request_backend = input_backend;
    engine.when_built(Box::new(move |engine| {
        engine.subscribe(Arc::new(|snapshot: &EngineSnapshot| {
            let snapshot = snapshot.clone();
            post(Box::new(move || with_app(|app| app.apply_snapshot(snapshot))));
        }));
        engine.subscribe_corrections(Arc::new(|plan: &CorrectionPlan| {
            let plan = plan.clone();
            post(Box::new(move || with_app(|app| app.announce_correction(&plan))));
        }));
        engine.subscribe_learning_prompts(Arc::new(move |prompt: Option<&LearningPrompt>| {
            // The caret is looked for on the engine's thread, while the window the user typed in is still the active one.
            match prompt.cloned() {
                None => post(Box::new(|| with_app(|app| app.apply_learning_prompt(None)))),
                Some(prompt) => {
                    let anchor = anchor_backend.input_anchor();
                    post(Box::new(move || with_app(|app| app.schedule_learning_prompt(prompt, anchor))));
                }
            }
        }));
        engine.subscribe_rule_requests(Arc::new(move |prompt: &LearningPrompt| {
            // The prompt never took the keyboard, so the window in front is the one the user typed in: it gets the keyboard
            // back when the rule window closes.
            let anchor = request_backend.input_anchor();
            let draft = draft_from_prompt(prompt);
            post(Box::new(move || settings_window::open_rule_editor(draft, anchor.and_then(|anchor| anchor.window))));
        }));
    }));

    app.sync_autostart(false);
    app.sync_tray();
    app.sync_update_schedule(true);
    if !no_engine {
        // Started once its models are loaded; until then the window says so.
        let _ = engine.start(Arc::new(|error: &str| {
            let error = error.to_owned();
            post(Box::new(move || with_app(|app| app.engine_start_failed(&error))));
        }));
    }
    if let Some(milliseconds) = quit_after_ms {
        // SAFETY: the timer belongs to the window made above.
        unsafe { SetTimer(window, SMOKE_TIMER_ID, milliseconds, None) };
    }
    if hidden && app.tray.borrow().is_some() && app.engine.snapshot().last_error.is_empty() {
        app.hide();
    } else {
        app.present();
    }
    // Anything posted before the window existed.
    // SAFETY: the window is ours.
    unsafe { PostMessageW(window, WM_POSTED, 0, 0) };

    // SAFETY: the usual loop on the thread that made the window; `message` is plain data the calls fill in.
    unsafe {
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&raw mut message, null_mut(), 0, 0) > 0 {
            if !settings_window::is_dialog_message(&message) {
                TranslateMessage(&raw const message);
                DispatchMessageW(&raw const message);
            }
        }
    }
    APP.with(|slot| slot.borrow_mut().take());
    0
}
