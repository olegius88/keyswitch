//! The Windows application: the window that owns the tray icon, the engine, and their lifecycle (the application part of
//! the Python's `desktop_ui.DesktopApplication`, `windows_ui.py` and `keyswitch_windows_entry.py`).
//!
//! One thread, the main one, runs the message loop and owns every window and the tray. Other threads (the engine's worker,
//! the model loader, the settings store) hand it work through `post`, which queues a closure and wakes the loop with a
//! message. The settings pages are not here yet: the window the tray opens is a small stand-in of the same class, so that
//! a second start still finds the running program.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ptr::{null, null_mut};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use ks_config::config::SettingsStore;
use ks_config::history::{HistoryStore, history_limit};
use ks_config::indicator::{layout_label, normalize_indicator_style};
use ks_config::logsetup::{self, LocalTime};
use ks_engine::context_access::ReaderFactory;
use ks_engine::engine_loader::{Build, DeferredEngine, Post};
use ks_engine::{CorrectionPlan, EngineSnapshot, KeySwitchEngine, LearningPrompt, engine::default_learning_store};
use serde_json::Value;
use windows_sys::Wdk::System::SystemServices::RtlGetVersion;
use windows_sys::Win32::Foundation::{GENERIC_WRITE, HWND, INVALID_HANDLE_VALUE, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{CreateFontW, GetDC, GetDeviceCaps, LOGPIXELSX, ReleaseDC};
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_WRITE, OPEN_EXISTING, WriteFile};
use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemInformation::{GetLocalTime, OSVERSIONINFOW};
use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
use windows_sys::Win32::System::Threading::ExitProcess;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, BS_PUSHBUTTON, ChangeWindowMessageFilterEx, CreateWindowExW, DefWindowProcW, DestroyWindow,
    DispatchMessageW, GetMessageW, IsDialogMessageW, KillTimer, LoadCursorW, MB_OK, MSG, MSGFLT_ALLOW, PostMessageW,
    PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SW_HIDE, SW_RESTORE, SetForegroundWindow, SetProcessDPIAware,
    SetTimer, SetWindowTextW, ShowWindow, TranslateMessage, WM_APP, WM_CLOSE, WM_COMMAND, WM_DESTROY, WM_LBUTTONUP, WM_RBUTTONUP,
    WM_SETFONT, WM_TIMER, WNDCLASSW, WS_CAPTION, WS_CHILD, WS_EX_WINDOWEDGE, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU,
    WS_TABSTOP, WS_VISIBLE, IDC_ARROW,
};

use crate::constants::{
    BACKEND_LABEL, BASE_DPI, ENGINE_STOP_WAIT_SECONDS, SETTINGS_BUTTON_HEIGHT_PIXELS, SETTINGS_BUTTON_WIDTH_PIXELS,
    SETTINGS_FONT_HEIGHT_PIXELS, SETTINGS_OPEN_FOLDER_LABEL, SETTINGS_WINDOW_CLASS, SETTINGS_WINDOW_CLIENT_HEIGHT_PIXELS,
    SETTINGS_WINDOW_CLIENT_WIDTH_PIXELS, SETTINGS_WINDOW_MARGIN_PIXELS, SETTINGS_WINDOW_TITLE, SMOKE_UI_QUIT_AFTER_MS,
};
use crate::tray::Tray;
use crate::tray_model::TrayAction;
use crate::windows_app::{Arguments, VERSION, resource_dir};
use crate::windows_context::WindowsFieldReader;
use crate::windows_backend::WindowsBackend;
use crate::windows_instance::WindowsSingleInstance;
use crate::windows_native::wide;
use crate::windows_system::{WindowsAutostartManager, open_directory};

const LOGGER: &str = "keyswitch.desktop_ui";
const WM_POSTED: u32 = WM_APP + 1;
const WM_TRAY: u32 = WM_APP + 2;
const ID_OPEN_FOLDER: usize = 1001;
const SMOKE_TIMER_ID: usize = 1;

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
        unsafe { PostMessageW(window as HWND, WM_POSTED, 0, 0) };
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
        GetLocalTime(&mut time);
        time
    };
    LocalTime {
        year: time.wYear as i32,
        month: time.wMonth as u32,
        day: time.wDay as u32,
        hour: time.wHour as u32,
        minute: time.wMinute as u32,
        second: time.wSecond as u32,
        millisecond: time.wMilliseconds as u32,
    }
}

/// `platform.platform()` as Python words it on Windows: `Windows-11-10.0.26100-SP0`.
fn platform_description() -> String {
    // SAFETY: RtlGetVersion fills the structure whose size it is told.
    let (major, minor, build) = unsafe {
        let mut info: OSVERSIONINFOW = std::mem::zeroed();
        info.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOW>() as u32;
        RtlGetVersion(&mut info);
        (info.dwMajorVersion, info.dwMinorVersion, info.dwBuildNumber)
    };
    let release = if major >= 10 && build >= 22000 { "11" } else if major >= 10 { "10" } else { "8" };
    format!("Windows-{release}-{major}.{minor}.{build}-SP0")
}

/// The clock and system description of the log, the log sink of this crate, and the process's DPI awareness.
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
    // SAFETY: a process-wide switch with no arguments.
    unsafe { SetProcessDPIAware() };
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
        WriteFile(handle, text.as_ptr(), text.len() as u32, &mut written, null_mut());
    }
}

/// `leave`: end the process without waiting for a thread that is still loading models. The log is closed first.
pub fn leave(status: i32) -> ! {
    logsetup::uninstall_handler();
    // SAFETY: ends the process.
    unsafe { ExitProcess(status as u32) }
}

// -------------------------------------------------------------------------------------------------------- the program

/// `main` after the arguments: one instance at a time, then the application.
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
    text: HWND,
    settings: Arc<SettingsStore>,
    history: Arc<HistoryStore>,
    engine: Arc<DeferredEngine>,
    autostart: Option<WindowsAutostartManager>,
    tray: RefCell<Option<Tray>>,
    error_text: RefCell<String>,
    snapshot: RefCell<EngineSnapshot>,
    closing: Cell<bool>,
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
        *self.error_text.borrow_mut() = text.to_owned();
        self.refresh_text();
    }

    /// The stand-in settings window: the engine's status, the version, and where the settings file is.
    fn refresh_text(&self) {
        let snapshot = self.snapshot.borrow();
        let status = if snapshot.enabled && snapshot.running {
            "Автокоррекция включена"
        } else if !snapshot.enabled {
            "Автокоррекция на паузе"
        } else {
            "Движок остановлен"
        };
        let mut lines = vec![
            status.to_owned(),
            format!("Раскладка: {}", layout_label(snapshot.current_group)),
            format!("Исправлений: {}", snapshot.correction_count),
            snapshot.last_action.clone(),
        ];
        let error = self.error_text.borrow();
        let error = if error.is_empty() { snapshot.last_error.as_str() } else { error.as_str() };
        if !error.is_empty() {
            lines.push(error.to_owned());
        }
        lines.push(String::new());
        lines.push(format!("KeySwitch {VERSION}"));
        lines.push(format!("Файл настроек: {}", self.settings.path.display()));
        lines.push("Страницы настроек пока недоступны.".to_owned());
        let text = wide(&lines.join("\r\n"));
        // SAFETY: the control is ours and the text is NUL-terminated.
        unsafe { SetWindowTextW(self.text, text.as_ptr()) };
    }

    fn apply_snapshot(&self, snapshot: EngineSnapshot) {
        if let Some(tray) = self.tray.borrow().as_ref() {
            let mut state = tray.state();
            state.group = snapshot.current_group;
            state.enabled = snapshot.enabled;
            tray.set_state(state);
        }
        *self.snapshot.borrow_mut() = snapshot;
        self.refresh_text();
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
        match path {
            "*" => {
                self.history.set_limit(history_limit(&self.settings.get("history.limit").unwrap_or(Value::Null)));
                self.sync_autostart(false);
                self.sync_tray();
                self.sync_tray_state();
            }
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
            // The pages arrive with the settings window: until then each of these opens the stand-in.
            TrayAction::ShowSettings | TrayAction::ShowHistory | TrayAction::ShowExclusions | TrayAction::ShowAbout => self.present(),
            TrayAction::SwitchLayout => {
                if !self.engine.select_alternate_group() {
                    let error = self.engine.snapshot().last_error;
                    self.set_error(&error);
                }
            }
            TrayAction::ToggleEngine => self.toggle("enabled", true),
            TrayAction::ToggleSound => self.toggle("general.sound", false),
            TrayAction::ToggleNotifications => self.toggle("general.notifications", true),
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

unsafe extern "system" fn window_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_POSTED => {
            drain_posted();
            0
        }
        WM_TRAY => {
            if lparam as u32 == WM_LBUTTONUP || lparam as u32 == WM_RBUTTONUP {
                // A primary click opens the same complete menu as the secondary one.
                with_app(|app| app.show_menu());
            }
            0
        }
        WM_COMMAND if (wparam & 0xFFFF) == ID_OPEN_FOLDER => {
            with_app(|app| {
                let directory = app.settings.path.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
                let _ = std::fs::create_dir_all(&directory);
                if let Err(error) = open_directory(&directory, None) {
                    app.set_error(&error.to_string());
                }
            });
            0
        }
        WM_TIMER if wparam == SMOKE_TIMER_ID => {
            // SAFETY: the timer belongs to this window.
            unsafe { KillTimer(window, SMOKE_TIMER_ID) };
            with_app(|app| app.shutdown());
            0
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
            0
        }
        WM_DESTROY => {
            // SAFETY: ends this thread's message loop.
            unsafe { PostQuitMessage(0) };
            0
        }
        _ if message == TASKBAR_CREATED.load(Ordering::SeqCst) && message != 0 => {
            // Explorer restarted: the shell has forgotten the icon.
            with_app(|app| {
                if let Some(tray) = app.tray.borrow().as_ref() {
                    tray.add();
                }
            });
            0
        }
        // SAFETY: the default procedure takes the arguments the system gave this one.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

fn scaled(pixels: i32, dpi: i32) -> i32 {
    pixels * dpi / BASE_DPI
}

/// The hidden window with its stand-in controls; the text control and the window.
fn create_window() -> Option<(HWND, HWND)> {
    // SAFETY: the class and the controls are made on this thread with strings that live through the calls; the screen's
    // device context is released at once.
    unsafe {
        let instance = GetModuleHandleW(null());
        let class = wide(SETTINGS_WINDOW_CLASS);
        let description = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(window_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: null_mut(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: 6 as _, // COLOR_WINDOW + 1
            lpszMenuName: null(),
            lpszClassName: class.as_ptr(),
        };
        if RegisterClassW(&description) == 0 {
            return None;
        }
        let screen = GetDC(null_mut());
        let dpi = GetDeviceCaps(screen, LOGPIXELSX as i32);
        ReleaseDC(null_mut(), screen);
        let dpi = if dpi > 0 { dpi } else { BASE_DPI };
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let mut frame = RECT {
            left: 0,
            top: 0,
            right: scaled(SETTINGS_WINDOW_CLIENT_WIDTH_PIXELS, dpi),
            bottom: scaled(SETTINGS_WINDOW_CLIENT_HEIGHT_PIXELS, dpi),
        };
        AdjustWindowRectEx(&mut frame, style, 0, WS_EX_WINDOWEDGE);
        let title = wide(SETTINGS_WINDOW_TITLE);
        let window = CreateWindowExW(
            WS_EX_WINDOWEDGE,
            class.as_ptr(),
            title.as_ptr(),
            style,
            100,
            100,
            frame.right - frame.left,
            frame.bottom - frame.top,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if window.is_null() {
            return None;
        }
        let margin = scaled(SETTINGS_WINDOW_MARGIN_PIXELS, dpi);
        let width = scaled(SETTINGS_WINDOW_CLIENT_WIDTH_PIXELS, dpi);
        let height = scaled(SETTINGS_WINDOW_CLIENT_HEIGHT_PIXELS, dpi);
        let button_height = scaled(SETTINGS_BUTTON_HEIGHT_PIXELS, dpi);
        let static_class = wide("STATIC");
        let button_class = wide("BUTTON");
        let empty = wide("");
        let text = CreateWindowExW(
            0,
            static_class.as_ptr(),
            empty.as_ptr(),
            WS_CHILD | WS_VISIBLE,
            margin,
            margin,
            width - 2 * margin,
            height - 3 * margin - button_height,
            window,
            null_mut(),
            instance,
            null(),
        );
        let label = wide(SETTINGS_OPEN_FOLDER_LABEL);
        let button = CreateWindowExW(
            0,
            button_class.as_ptr(),
            label.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32,
            margin,
            height - margin - button_height,
            scaled(SETTINGS_BUTTON_WIDTH_PIXELS, dpi),
            button_height,
            window,
            ID_OPEN_FOLDER as _,
            instance,
            null(),
        );
        let face = wide("Segoe UI");
        let font = CreateFontW(-scaled(SETTINGS_FONT_HEIGHT_PIXELS, dpi), 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 0, face.as_ptr());
        for control in [text, button] {
            windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(control, WM_SETFONT, font as usize, 1);
        }
        let created = wide("TaskbarCreated");
        TASKBAR_CREATED.store(RegisterWindowMessageW(created.as_ptr()), Ordering::SeqCst);
        // An elevated program is otherwise not told that Explorer restarted.
        ChangeWindowMessageFilterEx(window, TASKBAR_CREATED.load(Ordering::SeqCst), MSGFLT_ALLOW, null_mut());
        Some((window, text))
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
            WindowsFieldReader::new().map(|reader| Box::new(reader) as Box<dyn ks_backend::ManagedFieldReader>)
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
    let engine = DeferredEngine::new(engine_builder(settings.clone(), history.clone(), backend, learning.clone()), learning, enabled, post_fn, true);
    let autostart = match WindowsAutostartManager::new(None, None, None) {
        Ok(manager) => Some(manager),
        Err(error) => {
            logsetup::warning(LOGGER, &format!("Автозагрузка недоступна: {error}"));
            None
        }
    };
    let Some((window, text)) = create_window() else {
        logsetup::error(LOGGER, "Не удалось создать окно");
        return 1;
    };
    let app = Rc::new(App {
        window,
        text,
        settings: settings.clone(),
        history,
        engine: engine.clone(),
        autostart,
        tray: RefCell::new(None),
        error_text: RefCell::new(String::new()),
        snapshot: RefCell::new(engine.snapshot()),
        closing: Cell::new(false),
    });
    APP.with(|slot| *slot.borrow_mut() = Some(app.clone()));
    WINDOW.store(window as isize, Ordering::SeqCst);

    let _ = settings.subscribe(Arc::new(|path: &str, value: &Value| {
        let (path, value) = (path.to_owned(), value.clone());
        post(Box::new(move || with_app(|app| app.apply_setting(&path, &value))));
    }));
    engine.when_built(Box::new(|engine| {
        engine.subscribe(Arc::new(|snapshot: &EngineSnapshot| {
            let snapshot = snapshot.clone();
            post(Box::new(move || with_app(|app| app.apply_snapshot(snapshot))));
        }));
        engine.subscribe_corrections(Arc::new(|plan: &CorrectionPlan| {
            let plan = plan.clone();
            post(Box::new(move || with_app(|app| app.announce_correction(&plan))));
        }));
        // The rule window comes with the settings pages: a request for it is logged and nothing opens.
        engine.subscribe_rule_requests(Arc::new(|prompt: &LearningPrompt| {
            logsetup::info(LOGGER, &format!("Запрос окна правила (окно ещё не реализовано): {}", prompt.original));
        }));
    }));

    app.sync_autostart(false);
    app.sync_tray();
    if !no_engine {
        // Started once its models are loaded; until then the window says so.
        let _ = engine.start(Arc::new(|error: &str| {
            let error = error.to_owned();
            post(Box::new(move || with_app(|app| app.engine_start_failed(&error))));
        }));
    }
    app.refresh_text();
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
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            if IsDialogMessageW(window, &message) == 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
    APP.with(|slot| slot.borrow_mut().take());
    0
}
