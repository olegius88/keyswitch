//! The settings window and the tray icon on Windows, in plain Win32. The worker and short tasks run on threads of their
//! own and hand their results to this thread through the window's message queue.
use ks_cast::To as _;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{COLOR_WINDOW, DEFAULT_GUI_FONT, GetStockObject, GetSysColorBrush, HBRUSH, HFONT, UpdateWindow};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY, OPENFILENAMEW};
use windows_sys::Win32::UI::Controls::{
    EM_LINESCROLL, ICC_LISTVIEW_CLASSES, ICC_TAB_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx, LVCF_TEXT, LVCF_WIDTH, LVCOLUMNW, LVIF_TEXT, LVITEMW, LVM_DELETEALLITEMS,
    LVM_GETNEXTITEM, LVM_INSERTCOLUMNW, LVM_INSERTITEMW, LVM_SETEXTENDEDLISTVIEWSTYLE, LVM_SETITEMTEXTW, LVNI_SELECTED, LVS_EX_FULLROWSELECT, LVS_REPORT, LVS_SHOWSELALWAYS,
    NMHDR, TCIF_TEXT, TCITEMW, TCM_ADJUSTRECT, TCM_GETCURSEL, TCM_INSERTITEMW, TCM_SETCURSEL, TCN_SELCHANGE,
};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows_sys::Win32::UI::Shell::{NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::catalog::{current_catalog, verify_connection};
use crate::config::{Config, Source, data_directory, load_config, private_directory, save_config};
use crate::constants::*;
use crate::error::{Error, Result};
use crate::gui_model::*;
use crate::secrets::{read_token, redact, store_token, token_bot_id};
use crate::service::Service;
use crate::store::Store;
use crate::telegram::Telegram;

const CLASS: &str = "LogCourierWindow";
const PAGE_CLASS: &str = "LogCourierPage";
const SOURCE_CLASS: &str = "LogCourierSource";
const WM_POSTED: u32 = WM_APP + 1;
const WM_TRAY: u32 = WM_APP + 2;
const SHUTDOWN_TIMER: usize = 1;
const SHUTDOWN_POLL_MS: u32 = 100;
const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE_NAME: &str = "LogCourier";
const WINDOW_SIZE: (i32, i32) = (850, 640);

// Control and menu identifiers.
const ID_TABS: i32 = 100;
const ID_TOKEN: i32 = 101;
const ID_PERSIST: i32 = 102;
const ID_CHAT: i32 = 103;
const ID_DEVICE: i32 = 104;
const ID_GROUPS: i32 = 105;
const ID_FIND: i32 = 106;
const ID_CHECK: i32 = 107;
const ID_SAVE: i32 = 108;
const ID_ADOPT: i32 = 109;
const ID_TABLE: i32 = 110;
const ID_ADD: i32 = 111;
const ID_REMOVE: i32 = 112;
const ID_CONSENT: i32 = 113;
const ID_AUTO: i32 = 114;
const ID_AUTOSTART: i32 = 115;
const ID_INTERVAL: i32 = 116;
const ID_SEND: i32 = 117;
const ID_PAUSE: i32 = 118;
const ID_RESET: i32 = 119;
const ID_MENU_OPEN: i32 = 200;
const ID_MENU_SEND: i32 = 201;
const ID_MENU_PAUSE: i32 = 202;
const ID_MENU_EXIT: i32 = 203;
const ID_LABEL: i32 = 301;
const ID_ROTATIONS: i32 = 302;
const ID_EXISTING: i32 = 303;
const ID_OK: i32 = 304;
const ID_CANCEL: i32 = 305;

type Posted = Box<dyn FnOnce() + Send>;

static POSTED: Mutex<VecDeque<Posted>> = Mutex::new(VecDeque::new());
static WINDOW: AtomicIsize = AtomicIsize::new(0);
static TASKS: AtomicUsize = AtomicUsize::new(0);
static TASKBAR_CREATED: AtomicUsize = AtomicUsize::new(0);

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// Runs a closure on the window's thread.
fn post(action: Posted) {
    POSTED.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push_back(action);
    let window = WINDOW.load(Ordering::SeqCst) as HWND;
    // SAFETY: posting to a window that has gone is reported by the return value, which does not matter while closing.
    unsafe { PostMessageW(window, WM_POSTED, 0, 0) };
}

struct App {
    root: PathBuf,
    config: Config,
    sources: Vec<Source>,
    service: Option<Service>,
    window: HWND,
    tabs: HWND,
    pages: [HWND; 4],
    status: HWND,
    summary: HWND,
    events: HWND,
    autostart_note: HWND,
    groups: Vec<Value>,
    event_lines: VecDeque<String>,
    last_message: String,
    exiting: bool,
    source_result: Option<Option<Source>>,
    source_path: String,
    font: HFONT,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn with_app<T>(action: impl FnOnce(&mut App) -> T) -> Option<T> {
    APP.with(|app| app.try_borrow_mut().ok().and_then(|mut app| app.as_mut().map(action)))
}

fn instance() -> HINSTANCE {
    // SAFETY: asks for this process's own module.
    unsafe { GetModuleHandleW(null()) }
}

fn scale(window: HWND, value: i32) -> i32 {
    // SAFETY: a plain query about a window of this thread.
    let dpi = unsafe { GetDpiForWindow(window) };
    value * if dpi == 0 { 96 } else { dpi as i32 } / 96
}

struct Spot {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

fn control(parent: HWND, class: &str, text: &str, style: u32, extra: u32, id: i32, spot: Spot, font: HFONT) -> HWND {
    let (class, text) = (wide(class), wide(text));
    let at = |value| scale(parent, value);
    // SAFETY: the strings are NUL-terminated and outlive the call; the parent is a window of this thread.
    unsafe {
        let made = CreateWindowExW(extra, class.as_ptr(), text.as_ptr(), WS_CHILD | WS_VISIBLE | style, at(spot.x), at(spot.y), at(spot.w), at(spot.h), parent, id as usize as HMENU, instance(), null());
        SendMessageW(made, WM_SETFONT, font as usize, 1);
        made
    }
}

fn label(parent: HWND, text: &str, spot: Spot, font: HFONT) -> HWND {
    control(parent, "STATIC", text, 0, 0, 0, spot, font)
}

fn button(parent: HWND, text: &str, id: i32, spot: Spot, font: HFONT) -> HWND {
    control(parent, "BUTTON", text, WS_TABSTOP | BS_PUSHBUTTON as u32, 0, id, spot, font)
}

fn check(parent: HWND, text: &str, id: i32, on: bool, spot: Spot, font: HFONT) -> HWND {
    let made = control(parent, "BUTTON", text, WS_TABSTOP | BS_AUTOCHECKBOX as u32, 0, id, spot, font);
    // SAFETY: a message to the control just made.
    unsafe { SendMessageW(made, BM_SETCHECK, usize::from(on), 0) };
    made
}

fn edit(parent: HWND, text: &str, style: u32, id: i32, spot: Spot, font: HFONT) -> HWND {
    control(parent, "EDIT", text, WS_TABSTOP | ES_AUTOHSCROLL as u32 | style, WS_EX_CLIENTEDGE, id, spot, font)
}

fn item(page: HWND, id: i32) -> HWND {
    // SAFETY: a plain query about a window of this thread.
    unsafe { GetDlgItem(page, id) }
}

fn text_of(window: HWND) -> String {
    // SAFETY: the buffer holds the length asked for plus the terminator.
    unsafe {
        let length = GetWindowTextLengthW(window);
        let mut buffer = vec![0u16; length as usize + 1];
        let copied = GetWindowTextW(window, buffer.as_mut_ptr(), buffer.len() as i32);
        String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
    }
}

fn set_text(window: HWND, text: &str) {
    let text = wide(text);
    // SAFETY: the string is NUL-terminated and outlives the call.
    unsafe { SetWindowTextW(window, text.as_ptr()) };
}

fn is_checked(window: HWND) -> bool {
    // SAFETY: a plain message to a control of this thread.
    unsafe { SendMessageW(window, BM_GETCHECK, 0, 0) == 1 }
}

fn set_checked(window: HWND, on: bool) {
    // SAFETY: a plain message to a control of this thread.
    unsafe { SendMessageW(window, BM_SETCHECK, usize::from(on), 0) };
}

fn message_box(window: HWND, text: &str, title: &str, flags: u32) -> i32 {
    let (text, title) = (wide(text), wide(title));
    // SAFETY: the strings are NUL-terminated and outlive the call.
    unsafe { MessageBoxW(window, text.as_ptr(), title.as_ptr(), flags) }
}

impl App {
    fn telegram_page(&self) -> HWND {
        self.pages[0]
    }

    fn delivery_page(&self) -> HWND {
        self.pages[2]
    }

    fn token(&self) -> String {
        text_of(item(self.telegram_page(), ID_TOKEN)).trim().to_owned()
    }

    fn chat(&self) -> String {
        text_of(item(self.telegram_page(), ID_CHAT)).trim().to_owned()
    }

    fn error(&self, message: &str) {
        message_box(self.window, &redact(message), "LogCourier", MB_ICONWARNING | MB_OK);
    }

    fn say(&self, message: &str) {
        set_text(self.status, message);
    }

    fn draft(&self) -> Result<(Config, String)> {
        let delivery = self.delivery_page();
        draft(
            &self.config,
            &Form {
                token: &text_of(item(self.telegram_page(), ID_TOKEN)),
                chat: &text_of(item(self.telegram_page(), ID_CHAT)),
                device: &text_of(item(self.telegram_page(), ID_DEVICE)),
                sources: &self.sources,
                consent: is_checked(item(delivery, ID_CONSENT)),
                auto: is_checked(item(delivery, ID_AUTO)),
                interval: &text_of(item(delivery, ID_INTERVAL)),
            },
        )
    }

    fn save(&mut self) -> bool {
        let attempt = (|| -> Result<(Config, String)> {
            let (config, token) = self.draft()?;
            if !token.is_empty() && is_checked(item(self.telegram_page(), ID_PERSIST)) {
                store_token(&token)?;
            }
            save_config(&self.root, &config)?;
            Ok((config, token))
        })();
        match attempt {
            Ok((config, token)) => {
                self.config = config.clone();
                if let Some(service) = &self.service {
                    service.update(config, token);
                }
                self.say("Настройки сохранены.");
                true
            }
            Err(Error::Os(_)) => {
                self.error("Не удалось сохранить настройки.");
                false
            }
            Err(error) => {
                self.error(&error.to_string());
                false
            }
        }
    }

    fn pause(&mut self) {
        set_checked(item(self.delivery_page(), ID_AUTO), false);
        // Pausing never depends on availability of the system keyring.
        self.config.auto_send = false;
        if let Some(service) = &self.service {
            service.update(self.config.clone(), self.token());
        }
        if save_config(&self.root, &self.config).is_err() {
            self.error("Сбор приостановлен, но настройка не сохранена на диск.");
        }
        self.say("Автосбор приостановлен. Текущий сетевой запрос может завершиться.");
    }

    fn send_now(&mut self) {
        if !is_checked(item(self.delivery_page(), ID_CONSENT)) {
            self.error("Сначала разрешите передачу текста логов на вкладке «Отправка».");
            return;
        }
        if self.save() {
            if let Some(service) = &self.service {
                service.send_now();
            }
        }
    }

    fn reset_group(&mut self) {
        if self.config.chat_id.is_empty() || self.token().is_empty() {
            self.error("Сначала укажите токен и группу на вкладке «Telegram» и сохраните.");
            return;
        }
        if self.chat() != self.config.chat_id {
            self.error("ID группы изменён, но не сохранён. Сохраните настройки и повторите.");
            return;
        }
        let choice = message_box(self.window, &reset_warning(&self.config.chat_id), "Очистить группу", MB_YESNOCANCEL | MB_ICONWARNING | MB_DEFBUTTON3);
        if choice != IDYES && choice != IDNO {
            return;
        }
        if let Some(service) = &self.service {
            service.reset(choice == IDYES);
        }
        self.show_page(3);
        self.say("Очистка группы запущена; ход — на вкладке «Состояние».");
    }

    fn show_page(&self, index: usize) {
        for (position, page) in self.pages.iter().enumerate() {
            // SAFETY: plain calls on windows of this thread.
            unsafe { ShowWindow(*page, if position == index { SW_SHOW } else { SW_HIDE }) };
        }
        // SAFETY: a plain message to a control of this thread.
        unsafe { SendMessageW(self.tabs, TCM_SETCURSEL, index, 0) };
    }

    fn layout(&self) {
        // SAFETY: the rectangles are written by the calls; the windows belong to this thread.
        unsafe {
            let mut client: RECT = std::mem::zeroed();
            GetClientRect(self.window, &mut client);
            let bar = scale(self.window, 24);
            MoveWindow(self.status, 8, client.bottom - bar, client.right - 16, bar, 1);
            MoveWindow(self.tabs, 0, 0, client.right, client.bottom - bar, 1);
            let mut area = RECT { left: 0, top: 0, right: client.right, bottom: client.bottom - bar };
            SendMessageW(self.tabs, TCM_ADJUSTRECT, 0, &mut area as *mut RECT as isize);
            for page in self.pages {
                MoveWindow(page, area.left, area.top, area.right - area.left, area.bottom - area.top, 1);
            }
        }
    }

    fn refresh_sources(&self) {
        let table = item(self.pages[1], ID_TABLE);
        // SAFETY: the item structures and strings outlive each message.
        unsafe {
            SendMessageW(table, LVM_DELETEALLITEMS, 0, 0);
            for (row, source) in self.sources.iter().enumerate() {
                let cells = [source.label.clone(), source.path.clone(), source.rotations.to_string(), if source.include_existing { "Да" } else { "Нет" }.to_owned()];
                for (column, cell) in cells.iter().enumerate() {
                    let mut text = wide(cell);
                    let mut entry: LVITEMW = std::mem::zeroed();
                    entry.mask = LVIF_TEXT;
                    entry.iItem = row as i32;
                    entry.iSubItem = column as i32;
                    entry.pszText = text.as_mut_ptr();
                    let message = if column == 0 { LVM_INSERTITEMW } else { LVM_SETITEMTEXTW };
                    SendMessageW(table, message, row, &entry as *const LVITEMW as isize);
                }
            }
        }
    }

    fn remove_source(&mut self) {
        let table = item(self.pages[1], ID_TABLE);
        let mut selected = Vec::new();
        let mut from = -1isize;
        loop {
            // SAFETY: a plain message to a control of this thread; the wide index -1 starts the search.
            let next = unsafe { SendMessageW(table, LVM_GETNEXTITEM, from as usize, LVNI_SELECTED as isize) };
            if next < 0 {
                break;
            }
            selected.push(next as usize);
            from = next;
        }
        for row in selected.into_iter().rev() {
            if row < self.sources.len() {
                self.sources.remove(row);
            }
        }
        self.refresh_sources();
    }

    fn on_status(&mut self, message: &str, stats: &Value) {
        let stamp = stats["last_success"].as_f64().map(local_stamp);
        set_text(self.summary, &summary(message, stats, &self.sources, stamp.as_deref()));
        if message != self.last_message {
            self.event_lines.push_back(format!("{} {}", local_clock(), redact(message)));
            while self.event_lines.len() > EVENTS_LOG_MAX_LINES {
                self.event_lines.pop_front();
            }
            let all: Vec<&str> = self.event_lines.iter().map(String::as_str).collect();
            set_text(self.events, &all.join("\r\n"));
            // SAFETY: plain messages scrolling the edit control of this thread to its end.
            unsafe { SendMessageW(self.events, EM_LINESCROLL, 0, self.event_lines.len() as isize) };
            self.last_message = message.to_owned();
        }
        tray(self.window, NIM_MODIFY, Some(&tray_tooltip(message)));
    }

    fn reveal(&self) {
        // SAFETY: plain calls on the main window of this thread.
        unsafe {
            ShowWindow(self.window, SW_SHOWNORMAL);
            SetForegroundWindow(self.window);
        }
    }

    fn shutdown(&mut self) {
        self.exiting = true;
        if let Some(service) = &self.service {
            service.stop();
        }
        self.say("Завершение текущего запроса…");
        // SAFETY: plain calls on the main window of this thread.
        unsafe {
            EnableWindow(self.window, 0);
            SetTimer(self.window, SHUTDOWN_TIMER, SHUTDOWN_POLL_MS, None);
        }
    }

    fn finish_shutdown(&self) {
        let idle = self.service.as_ref().is_none_or(Service::is_finished) && TASKS.load(Ordering::SeqCst) == 0;
        if idle {
            tray(self.window, NIM_DELETE, None);
            // SAFETY: ends this thread's message loop.
            unsafe {
                KillTimer(self.window, SHUTDOWN_TIMER);
                PostQuitMessage(0);
            }
        }
    }

    fn change_autostart(&mut self) {
        let box_ = item(self.delivery_page(), ID_AUTOSTART);
        let enabled = is_checked(box_);
        match set_autostart(enabled) {
            Ok(()) => set_text(self.autostart_note, if enabled { "Автозапуск включён: при входе в Windows программа будет в трее." } else { "Автозапуск выключен." }),
            Err(error) => {
                set_checked(box_, !enabled);
                self.error(if let Error::Os(_) = error { "Не удалось изменить автозапуск Windows.".to_owned() } else { error.to_string() }.as_str());
            }
        }
    }

    fn command(&mut self, id: i32, code: u32) -> bool {
        match id {
            ID_SAVE if code == BN_CLICKED => {
                self.save();
            }
            ID_FIND => find_groups(self),
            ID_CHECK => check_connection(self),
            ID_ADOPT => adopt_catalog(self),
            ID_ADD => return false,
            ID_REMOVE => self.remove_source(),
            ID_SEND | ID_MENU_SEND => self.send_now(),
            ID_PAUSE | ID_MENU_PAUSE => self.pause(),
            ID_RESET => self.reset_group(),
            ID_AUTOSTART => self.change_autostart(),
            ID_MENU_OPEN => self.reveal(),
            ID_MENU_EXIT => self.shutdown(),
            ID_GROUPS if code == CBN_SELCHANGE => {
                // SAFETY: a plain message to a control of this thread.
                let chosen = unsafe { SendMessageW(item(self.telegram_page(), ID_GROUPS), CB_GETCURSEL, 0, 0) };
                if let Some(group) = usize::try_from(chosen).ok().and_then(|index| self.groups.get(index)) {
                    set_text(item(self.telegram_page(), ID_CHAT), &group["id"].to_string());
                }
            }
            // Another group or token: what was allowed for the old one is no longer allowed.
            ID_CHAT | ID_TOKEN if code == EN_CHANGE as u32 => set_checked(item(self.delivery_page(), ID_CONSENT), false),
            _ => {}
        }
        true
    }
}

fn local_parts(seconds: f64) -> (u16, u16, u16, u16, u16, u16) {
    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    let ticks = ((seconds + 11_644_473_600.0) * 10_000_000.0) as u64;
    let file = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    // SAFETY: the structures are written by the calls.
    unsafe {
        let (mut utc, mut local): (SYSTEMTIME, SYSTEMTIME) = (std::mem::zeroed(), std::mem::zeroed());
        FileTimeToSystemTime(&file, &mut utc);
        SystemTimeToTzSpecificLocalTime(null(), &utc, &mut local);
        (local.wDay, local.wMonth, local.wYear, local.wHour, local.wMinute, local.wSecond)
    }
}

fn local_stamp(seconds: f64) -> String {
    let (d, m, y, hh, mm, ss) = local_parts(seconds);
    format!("{d:02}.{m:02}.{y} {hh:02}:{mm:02}:{ss:02}")
}

fn local_clock() -> String {
    let (_, _, _, hh, mm, ss) = local_parts(crate::timeutil::now_seconds());
    format!("{hh:02}:{mm:02}:{ss:02}")
}

/// A network job off the window's thread; its answer comes back to it.
fn task<T: Send + 'static>(action: impl FnOnce() -> Result<T> + Send + 'static, done: impl FnOnce(&mut App, T) + Send + 'static) {
    TASKS.fetch_add(1, Ordering::SeqCst);
    std::thread::spawn(move || {
        let result = action();
        post(Box::new(move || {
            TASKS.fetch_sub(1, Ordering::SeqCst);
            with_app(|app| match result {
                Ok(value) => done(app, value),
                Err(error) if error.has_user_text() => app.error(&error.to_string()),
                Err(error) => app.error(&format!("Операция не выполнена ({}).", error.type_name())),
            });
        }));
    });
}

fn find_groups(app: &mut App) {
    let client = match Telegram::new(&app.token()) {
        Ok(client) => client,
        Err(error) => return app.error(&error.to_string()),
    };
    task(move || client.groups(), |app, groups: Vec<Value>| {
        if groups.is_empty() {
            return app.error("Группы не найдены. Добавьте бота и отправьте /start в группе, затем повторите. Либо введите Chat ID вручную.");
        }
        let combo = item(app.telegram_page(), ID_GROUPS);
        // SAFETY: the strings outlive each message to the combo box of this thread.
        unsafe {
            SendMessageW(combo, CB_RESETCONTENT, 0, 0);
            for group in &groups {
                let text = wide(&group_label(group));
                SendMessageW(combo, CB_ADDSTRING, 0, text.as_ptr() as isize);
            }
            SendMessageW(combo, CB_SETCURSEL, 0, 0);
        }
        set_text(item(app.telegram_page(), ID_CHAT), &groups[0]["id"].to_string());
        app.groups = groups;
        app.say("Выберите группу в списке.");
    });
}

fn check_connection(app: &mut App) {
    let chat = app.chat();
    let client = match Telegram::new(&app.token()).and_then(|client| if chat.is_empty() { Err(Error::value("Сначала выберите группу.")) } else { Ok(client) }) {
        Ok(client) => client,
        Err(error) => return app.error(&error.to_string()),
    };
    task(move || verify_connection(&client, &chat), |app, result: String| {
        message_box(app.window, &result, "Подключение проверено", MB_ICONINFORMATION | MB_OK);
    });
}

fn adopt_catalog(app: &mut App) {
    let chat = app.chat();
    let client = match Telegram::new(&app.token()).and_then(|client| if chat.is_empty() { Err(Error::value("Укажите ID группы.")) } else { Ok(client) }) {
        Ok(client) => client,
        Err(error) => return app.error(&error.to_string()),
    };
    let bot_id = crate::telegram::Client::bot_id(&client).to_owned();
    let asked = chat.clone();
    task(
        move || {
            verify_connection(&client, &chat)?;
            current_catalog(&client, &chat)
        },
        move |app, head: Option<Value>| confirm_catalog(app, head, &bot_id, &asked),
    );
}

fn confirm_catalog(app: &mut App, head: Option<Value>, bot_id: &str, chat: &str) {
    if app.chat() != chat || token_bot_id(&app.token()).unwrap_or_default() != bot_id {
        return app.error("Настройки подключения изменились. Повторите проверку.");
    }
    let Some(head) = head else {
        return app.error("В группе нет закреплённого каталога LogCourier.");
    };
    let device = head["index"]["device_id"].as_str().unwrap_or_default().to_owned();
    if device == app.config.device_id {
        message_box(app.window, "Этот каталог уже относится к вашему профилю.", "LogCourier", MB_ICONINFORMATION | MB_OK);
        return;
    }
    let question = "Сначала остановите LogCourier на прежнем компьютере.\r\nПродолжить его каталог на этом компьютере?\r\nСтарые логи не скачиваются. Автоотправку потребуется включить отдельно.";
    if message_box(app.window, question, "Перенос каталога", MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2) != IDYES {
        return;
    }
    app.pause();
    let queued = Store::open(&app.root).and_then(|store| store.stats(&format!("{bot_id}:{chat}")));
    match queued {
        Ok(stats) if stats["queued"].as_i64().unwrap_or(0) != 0 || stats["unindexed"].as_i64().unwrap_or(0) != 0 => {
            app.error("Сначала завершите отправку локальной очереди в эту группу.");
        }
        Ok(_) => {
            let old = std::mem::replace(&mut app.config.device_id, device);
            set_checked(item(app.delivery_page(), ID_CONSENT), false);
            if !app.save() {
                app.config.device_id = old;
            }
        }
        Err(error) => app.error(&error.to_string()),
    }
}

fn autostart_enabled() -> bool {
    let key = wide(RUN_KEY);
    let name = wide(VALUE_NAME);
    // SAFETY: the buffers are written by the calls and the key is closed before returning.
    unsafe {
        let mut opened: HKEY = null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, key.as_ptr(), 0, KEY_READ, &mut opened) != 0 {
            return false;
        }
        let (mut kind, mut size) = (0u32, 0u32);
        let found = RegQueryValueExW(opened, name.as_ptr(), null(), &mut kind, null_mut(), &mut size);
        RegCloseKey(opened);
        found == 0 && kind == REG_SZ && size > 2
    }
}

fn set_autostart(enabled: bool) -> Result<()> {
    let key = wide(RUN_KEY);
    let name = wide(VALUE_NAME);
    let value = if enabled {
        let executable = std::env::current_exe()?;
        Some(wide(&autostart_command(&executable.display().to_string())?))
    } else {
        None
    };
    // SAFETY: the strings are NUL-terminated, the sizes are theirs, and the key is closed before returning.
    unsafe {
        let mut opened: HKEY = null_mut();
        if RegCreateKeyExW(HKEY_CURRENT_USER, key.as_ptr(), 0, null(), 0, KEY_SET_VALUE, null(), &mut opened, null_mut()) != 0 {
            return Err(Error::Os("OSError"));
        }
        let code = match &value {
            Some(value) => RegSetValueExW(opened, name.as_ptr(), 0, REG_SZ, value.as_ptr().cast(), (value.len() * 2) as u32),
            None => {
                let code = RegDeleteValueW(opened, name.as_ptr());
                if code == 2 { 0 } else { code }
            }
        };
        RegCloseKey(opened);
        if code != 0 {
            return Err(Error::Os("OSError"));
        }
    }
    Ok(())
}

fn tray(window: HWND, action: u32, tip: Option<&str>) {
    // SAFETY: the structure is filled for this window and read during the call; the icon is this program's own resource.
    unsafe {
        let mut data: NOTIFYICONDATAW = std::mem::zeroed();
        data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = window;
        data.uID = 1;
        data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        data.uCallbackMessage = WM_TRAY;
        data.hIcon = LoadIconW(instance(), 1 as *const u16);
        let mut text = data.szTip;
        for (slot, unit) in text.iter_mut().zip(tip.unwrap_or("LogCourier — личный сборщик логов").encode_utf16().take(120)) {
            *slot = unit;
        }
        data.szTip = text;
        Shell_NotifyIconW(action, &data);
    }
}

fn tray_menu(window: HWND) {
    // SAFETY: the menu lives for this call; the strings outlive each append.
    unsafe {
        let menu = CreatePopupMenu();
        for (id, title) in [(ID_MENU_OPEN, "Открыть настройки"), (ID_MENU_SEND, "Отправить сейчас"), (ID_MENU_PAUSE, "Приостановить автосбор"), (ID_MENU_EXIT, "Выход")] {
            let text = wide(title);
            AppendMenuW(menu, MF_STRING, id as usize, text.as_ptr());
        }
        let mut point = POINT { x: 0, y: 0 };
        GetCursorPos(&mut point);
        SetForegroundWindow(window);
        TrackPopupMenu(menu, TPM_RIGHTBUTTON, point.x, point.y, 0, window, null());
        PostMessageW(window, WM_NULL, 0, 0);
        DestroyMenu(menu);
    }
}

fn choose_file(window: HWND) -> Option<String> {
    let filter: Vec<u16> = "Все файлы (*)\0*\0\0".encode_utf16().collect();
    let title = wide("Выберите лог");
    let mut buffer = vec![0u16; 4096];
    // SAFETY: the structure points to buffers that outlive the call.
    unsafe {
        let mut dialog: OPENFILENAMEW = std::mem::zeroed();
        dialog.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
        dialog.hwndOwner = window;
        dialog.lpstrFilter = filter.as_ptr();
        dialog.lpstrFile = buffer.as_mut_ptr();
        dialog.nMaxFile = buffer.len() as u32;
        dialog.lpstrTitle = title.as_ptr();
        dialog.Flags = OFN_FILEMUSTEXIST | OFN_HIDEREADONLY;
        if GetOpenFileNameW(&mut dialog) == 0 {
            return None;
        }
    }
    let end = buffer.iter().position(|unit| *unit == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..end]))
}

/// The dialog of a new source, as a window of its own with a message loop until it is answered.
fn add_source(window: HWND) {
    let Some(path) = choose_file(window) else { return };
    let name = Path::new(&path).file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let font = with_app(|app| {
        app.source_result = None;
        app.source_path.clone_from(&path);
        app.font
    })
    .unwrap_or(null_mut());
    let (class, title) = (wide(SOURCE_CLASS), wide("Добавить источник"));
    // SAFETY: the strings outlive the call; the class is registered at start; the owner is disabled while the dialog runs.
    let dialog = unsafe {
        let dialog = CreateWindowExW(WS_EX_DLGMODALFRAME, class.as_ptr(), title.as_ptr(), WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_VISIBLE, 200, 200, scale(window, 520), scale(window, 270), window, null_mut(), instance(), null());
        EnableWindow(window, 0);
        dialog
    };
    label(dialog, "Файл", Spot { x: 12, y: 14, w: 80, h: 20 }, font);
    label(dialog, &path, Spot { x: 100, y: 14, w: 400, h: 20 }, font);
    label(dialog, "Название", Spot { x: 12, y: 44, w: 80, h: 20 }, font);
    edit(dialog, &name, 0, ID_LABEL, Spot { x: 100, y: 42, w: 400, h: 24 }, font);
    label(dialog, "Ротаций", Spot { x: 12, y: 76, w: 80, h: 20 }, font);
    edit(dialog, &DEFAULT_ROTATIONS.to_string(), ES_NUMBER as u32, ID_ROTATIONS, Spot { x: 100, y: 74, w: 80, h: 24 }, font);
    check(dialog, "Включить уже существующие записи и ротации", ID_EXISTING, false, Spot { x: 100, y: 104, w: 400, h: 22 }, font);
    label(dialog, SOURCE_NOTE, Spot { x: 12, y: 132, w: 490, h: 48 }, font);
    button(dialog, "OK", ID_OK, Spot { x: 320, y: 190, w: 86, h: 28 }, font);
    button(dialog, "Отмена", ID_CANCEL, Spot { x: 414, y: 190, w: 86, h: 28 }, font);
    // SAFETY: a standard message loop for the dialog; the owner is enabled again before returning.
    unsafe {
        let mut message: MSG = std::mem::zeroed();
        while with_app(|app| app.source_result.is_none()).unwrap_or(false) && GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            if IsDialogMessageW(dialog, &message) == 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        EnableWindow(window, 1);
        DestroyWindow(dialog);
        SetForegroundWindow(window);
    }
    with_app(|app| {
        if let Some(Some(source)) = app.source_result.take() {
            app.sources.push(source);
            app.refresh_sources();
        }
    });
}

unsafe extern "system" fn source_proc(dialog: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_COMMAND => {
            let id = (wparam & 0xFFFF) as i32;
            if id == ID_OK {
                let label = text_of(item(dialog, ID_LABEL));
                let rotations = text_of(item(dialog, ID_ROTATIONS));
                let existing = is_checked(item(dialog, ID_EXISTING));
                with_app(|app| match source_from(&app.source_path, &label, &rotations, existing, &app.sources) {
                    Ok(source) => app.source_result = Some(Some(source)),
                    Err(error) => {
                        message_box(dialog, &error.to_string(), "LogCourier", MB_ICONWARNING | MB_OK);
                    }
                });
            } else if id == ID_CANCEL {
                with_app(|app| app.source_result = Some(None));
            }
            0
        }
        WM_CLOSE => {
            with_app(|app| app.source_result = Some(None));
            0
        }
        // SAFETY: the default handling of a message of this window.
        _ => unsafe { DefWindowProcW(dialog, message, wparam, lparam) },
    }
}

/// The pages only pass their controls' commands on to the main window.
unsafe extern "system" fn page_proc(page: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if message == WM_COMMAND {
        let window = WINDOW.load(Ordering::SeqCst) as HWND;
        // SAFETY: the main window outlives its pages.
        return unsafe { SendMessageW(window, WM_COMMAND, wparam, lparam) };
    }
    if message == WM_CTLCOLORSTATIC {
        // SAFETY: the system colour brush needs no release.
        return unsafe { GetSysColorBrush(COLOR_WINDOW as i32) } as LRESULT;
    }
    // SAFETY: the default handling of a message of this window.
    unsafe { DefWindowProcW(page, message, wparam, lparam) }
}

unsafe extern "system" fn window_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_POSTED => {
            let action = POSTED.lock().unwrap_or_else(std::sync::PoisonError::into_inner).pop_front();
            if let Some(action) = action {
                action();
            }
            0
        }
        WM_SIZE => {
            with_app(|app| app.layout());
            0
        }
        WM_COMMAND => {
            let (id, code) = ((wparam & 0xFFFF) as i32, ((wparam >> 16) & 0xFFFF) as u32);
            if with_app(|app| app.command(id, code)) == Some(false) {
                add_source(window);
            }
            0
        }
        WM_NOTIFY => {
            // SAFETY: for WM_NOTIFY the parameter points to a header valid during the message.
            let header = unsafe { &*lparam.pointer::<NMHDR>() };
            if header.idFrom == ID_TABS as usize && header.code == TCN_SELCHANGE {
                with_app(|app| {
                    // SAFETY: a plain message to a control of this thread.
                    let index = unsafe { SendMessageW(app.tabs, TCM_GETCURSEL, 0, 0) };
                    app.show_page(index.max(0) as usize);
                });
            }
            0
        }
        WM_TRAY => {
            match lparam as u32 {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => {
                    with_app(|app| app.reveal());
                }
                WM_RBUTTONUP | WM_CONTEXTMENU => tray_menu(window),
                _ => {}
            }
            0
        }
        WM_TIMER if wparam == SHUTDOWN_TIMER => {
            with_app(|app| app.finish_shutdown());
            0
        }
        WM_CLOSE => {
            // The program goes on in the tray; only its menu's «Выход» ends it.
            // SAFETY: hides the main window of this thread.
            unsafe { ShowWindow(window, SW_HIDE) };
            0
        }
        WM_DESTROY => {
            // SAFETY: ends this thread's message loop.
            unsafe { PostQuitMessage(0) };
            0
        }
        _ => {
            if message == TASKBAR_CREATED.load(Ordering::SeqCst) as u32 && message != 0 {
                tray(window, NIM_ADD, None);
                return 0;
            }
            // SAFETY: the default handling of a message of this window.
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
    }
}

fn register_class(name: &str, procedure: WNDPROC) {
    let name = wide(name);
    // SAFETY: the structure is read during the call, which copies the name.
    unsafe {
        let mut class: WNDCLASSW = std::mem::zeroed();
        class.lpfnWndProc = procedure;
        class.hInstance = instance();
        class.hIcon = LoadIconW(instance(), 1 as *const u16);
        class.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
        class.hbrBackground = (COLOR_WINDOW + 1) as usize as HBRUSH;
        class.lpszClassName = name.as_ptr();
        RegisterClassW(&class);
    }
}

fn make_page(window: HWND, font: HFONT) -> HWND {
    let class = wide(PAGE_CLASS);
    let _ = font;
    // SAFETY: the class is registered at start; the parent is the main window of this thread.
    unsafe { CreateWindowExW(WS_EX_CONTROLPARENT, class.as_ptr(), null(), WS_CHILD, 0, 0, 10, 10, window, null_mut(), instance(), null()) }
}

fn insert_tab(tabs: HWND, index: usize, title: &str) {
    let mut text = wide(title);
    // SAFETY: the structure and string outlive the message.
    unsafe {
        let mut entry: TCITEMW = std::mem::zeroed();
        entry.mask = TCIF_TEXT;
        entry.pszText = text.as_mut_ptr();
        SendMessageW(tabs, TCM_INSERTITEMW, index, &entry as *const TCITEMW as isize);
    }
}

fn build(root: PathBuf, config: Config, window: HWND) -> App {
    // SAFETY: a stock object needs no release.
    let font = unsafe { GetStockObject(DEFAULT_GUI_FONT) } as HFONT;
    let tabs = control(window, "SysTabControl32", "", WS_CLIPSIBLINGS | WS_TABSTOP, 0, ID_TABS, Spot { x: 0, y: 0, w: 10, h: 10 }, font);
    for (index, title) in ["Telegram", "Файлы", "Отправка", "Состояние"].iter().enumerate() {
        insert_tab(tabs, index, title);
    }
    let pages = [make_page(window, font), make_page(window, font), make_page(window, font), make_page(window, font)];

    let page = pages[0];
    label(page, CONNECTION_INTRO, Spot { x: 12, y: 10, w: 780, h: 110 }, font);
    label(page, "Токен", Spot { x: 12, y: 130, w: 150, h: 20 }, font);
    edit(page, "", ES_PASSWORD as u32, ID_TOKEN, Spot { x: 170, y: 128, w: 620, h: 24 }, font);
    check(page, "Сохранить токен в системном хранилище секретов", ID_PERSIST, true, Spot { x: 170, y: 156, w: 620, h: 22 }, font);
    label(page, "ID группы (Chat ID)", Spot { x: 12, y: 186, w: 150, h: 20 }, font);
    edit(page, &config.chat_id, 0, ID_CHAT, Spot { x: 170, y: 184, w: 240, h: 24 }, font);
    control(page, "COMBOBOX", "", WS_TABSTOP | WS_VSCROLL | CBS_DROPDOWNLIST as u32, 0, ID_GROUPS, Spot { x: 420, y: 184, w: 370, h: 200 }, font);
    label(page, "Имя устройства", Spot { x: 12, y: 218, w: 150, h: 20 }, font);
    edit(page, &config.device_name, 0, ID_DEVICE, Spot { x: 170, y: 216, w: 620, h: 24 }, font);
    button(page, "Найти группы", ID_FIND, Spot { x: 12, y: 254, w: 180, h: 30 }, font);
    button(page, "Проверить подключение", ID_CHECK, Spot { x: 200, y: 254, w: 200, h: 30 }, font);
    button(page, "Сохранить", ID_SAVE, Spot { x: 408, y: 254, w: 140, h: 30 }, font);
    button(page, "Подключить существующий каталог (перенос с другого компьютера)", ID_ADOPT, Spot { x: 12, y: 292, w: 536, h: 30 }, font);

    let page = pages[1];
    label(page, SOURCES_NOTE, Spot { x: 12, y: 10, w: 780, h: 56 }, font);
    let table = control(page, "SysListView32", "", WS_TABSTOP | LVS_REPORT | LVS_SHOWSELALWAYS, WS_EX_CLIENTEDGE, ID_TABLE, Spot { x: 12, y: 70, w: 780, h: 330 }, font);
    // SAFETY: the column structures and strings outlive each message.
    unsafe {
        SendMessageW(table, LVM_SETEXTENDEDLISTVIEWSTYLE, 0, LVS_EX_FULLROWSELECT as isize);
        for (index, (title, width)) in [("Название", 180), ("Файл", 380), ("Ротаций", 70), ("Старые записи", 110)].iter().enumerate() {
            let mut text = wide(title);
            let mut column: LVCOLUMNW = std::mem::zeroed();
            column.mask = LVCF_TEXT | LVCF_WIDTH;
            column.cx = scale(window, *width);
            column.pszText = text.as_mut_ptr();
            SendMessageW(table, LVM_INSERTCOLUMNW, index, &column as *const LVCOLUMNW as isize);
        }
    }
    button(page, "Добавить файл", ID_ADD, Spot { x: 12, y: 410, w: 160, h: 30 }, font);
    button(page, "Удалить из списка", ID_REMOVE, Spot { x: 180, y: 410, w: 170, h: 30 }, font);
    button(page, "Сохранить", ID_SAVE, Spot { x: 358, y: 410, w: 140, h: 30 }, font);

    let page = pages[2];
    check(page, "Разрешаю отправлять содержимое выбранных логов в эту группу Telegram", ID_CONSENT, config.consent, Spot { x: 12, y: 10, w: 780, h: 22 }, font);
    check(page, "Автоматически собирать и отправлять логи", ID_AUTO, config.auto_send, Spot { x: 12, y: 34, w: 780, h: 22 }, font);
    check(page, "Запускать при входе в Windows, сразу в трее", ID_AUTOSTART, autostart_enabled(), Spot { x: 12, y: 58, w: 780, h: 22 }, font);
    let autostart_note = label(page, "Применяется сразу. Полный выход — через меню трея → «Выход».", Spot { x: 32, y: 82, w: 760, h: 20 }, font);
    label(page, "Интервал отправки, мин", Spot { x: 12, y: 110, w: 180, h: 20 }, font);
    edit(page, &config.interval_minutes.to_string(), ES_NUMBER as u32, ID_INTERVAL, Spot { x: 200, y: 108, w: 90, h: 24 }, font);
    label(page, &delivery_note(), Spot { x: 12, y: 140, w: 780, h: 250 }, font);
    button(page, "Сохранить", ID_SAVE, Spot { x: 12, y: 396, w: 140, h: 30 }, font);
    button(page, "Отправить сейчас", ID_SEND, Spot { x: 160, y: 396, w: 170, h: 30 }, font);
    button(page, "Пауза", ID_PAUSE, Spot { x: 338, y: 396, w: 120, h: 30 }, font);
    button(page, "Очистить группу и начать заново…", ID_RESET, Spot { x: 12, y: 432, w: 446, h: 30 }, font);

    let page = pages[3];
    let summary_label = label(page, "Нет отправленных архивов.", Spot { x: 12, y: 10, w: 780, h: 104 }, font);
    let events = control(page, "EDIT", "", WS_VSCROLL | ES_MULTILINE as u32 | ES_READONLY as u32 | ES_AUTOVSCROLL as u32, WS_EX_CLIENTEDGE, 0, Spot { x: 12, y: 120, w: 780, h: 300 }, font);
    label(page, &format!("Локальные настройки и очередь: {}\r\nЖурнал состояния не содержит токенов и фрагментов исходных логов.", root.display()), Spot { x: 12, y: 426, w: 780, h: 40 }, font);

    let status = label(window, "Отправка только после вашего разрешения", Spot { x: 8, y: 0, w: 10, h: 20 }, font);
    let sources = config.sources.clone();
    App {
        root, config, sources, service: None, window, tabs, pages, status, summary: summary_label, events, autostart_note, groups: Vec::new(),
        event_lines: VecDeque::new(), last_message: String::new(), exiting: false, source_result: None, source_path: String::new(), font,
    }
}

fn message_loop(window: HWND) -> i32 {
    // SAFETY: a standard message loop of this thread.
    unsafe {
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            let page = with_app(|app| {
                // SAFETY: a plain message to a control of this thread.
                let index = SendMessageW(app.tabs, TCM_GETCURSEL, 0, 0).max(0) as usize;
                app.pages.get(index).copied().unwrap_or(window)
            })
            .unwrap_or(window);
            if IsDialogMessageW(page, &message) == 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        message.wParam as i32
    }
}

fn start_ui() {
    // SAFETY: the structure is read during the call.
    unsafe {
        let controls = INITCOMMONCONTROLSEX { dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32, dwICC: ICC_TAB_CLASSES | ICC_LISTVIEW_CLASSES };
        InitCommonControlsEx(&controls);
    }
    register_class(CLASS, Some(window_proc));
    register_class(PAGE_CLASS, Some(page_proc));
    register_class(SOURCE_CLASS, Some(source_proc));
}

fn new_window() -> HWND {
    let (class, title) = (wide(CLASS), wide(&format!("{WINDOW_TITLE_PREFIX} {} — сборщик логов", crate::VERSION)));
    // SAFETY: the class is registered and the strings outlive the call.
    unsafe {
        let window = CreateWindowExW(0, class.as_ptr(), title.as_ptr(), WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN, CW_USEDEFAULT, CW_USEDEFAULT, WINDOW_SIZE.0, WINDOW_SIZE.1, null_mut(), null_mut(), instance(), null());
        let scaled = (scale(window, WINDOW_SIZE.0), scale(window, WINDOW_SIZE.1));
        SetWindowPos(window, null_mut(), 0, 0, scaled.0, scaled.1, SWP_NOMOVE | SWP_NOZORDER);
        window
    }
}

pub fn run(minimized: bool) -> i32 {
    let root = data_directory();
    let _ = private_directory(&root);
    let name = wide("Local\\LogCourier.instance");
    // SAFETY: the name is NUL-terminated; the handle stays open for the life of the process.
    unsafe {
        CreateMutexW(null(), 0, name.as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            message_box(null_mut(), "Сборщик уже запущен. Откройте его через значок в трее.", "LogCourier", MB_ICONINFORMATION | MB_OK);
            return 0;
        }
    }
    let config = match load_config(&root) {
        Ok(config) => config,
        Err(_) => {
            message_box(null_mut(), &format!("Не удалось прочитать {}.\r\nФайл оставлен без изменений; автоматическая отправка не запущена.", root.join("config.json").display()), "LogCourier", MB_ICONERROR | MB_OK);
            return 1;
        }
    };
    start_ui();
    let window = new_window();
    WINDOW.store(window as isize, Ordering::SeqCst);
    let mut app = build(root.clone(), config.clone(), window);
    let mut token = std::env::var("LOGCOURIER_BOT_TOKEN").unwrap_or_default();
    if token.is_empty() && !config.bot_id.is_empty() {
        match read_token(&config.bot_id) {
            Ok(found) => token = found,
            Err(error) => app.event_lines.push_back(error.to_string()),
        }
    }
    set_text(item(app.pages[0], ID_TOKEN), &token);
    // Reading the stored values into the fields is not the user changing them.
    set_checked(item(app.pages[2], ID_CONSENT), config.consent);
    let notify: crate::service::Notify = Arc::new(|message, stats| {
        post(Box::new(move || {
            with_app(|app| app.on_status(&message, &stats));
        }));
    });
    app.service = Some(Service::start(root, config.clone(), token, notify));
    app.refresh_sources();
    app.layout();
    app.show_page(0);
    APP.with(|slot| *slot.borrow_mut() = Some(app));
    tray(window, NIM_ADD, None);
    let created = wide("TaskbarCreated");
    // SAFETY: the name is NUL-terminated.
    TASKBAR_CREATED.store(unsafe { RegisterWindowMessageW(created.as_ptr()) } as usize, Ordering::SeqCst);
    if !minimized || config.bot_id.is_empty() {
        // SAFETY: plain calls on the main window of this thread.
        unsafe {
            ShowWindow(window, SW_SHOW);
            UpdateWindow(window);
            SetFocus(window);
        }
    }
    message_loop(window)
}

/// The window on an isolated profile, built and closed: what an installer's check needs to know.
pub fn smoke_test(output: Option<&Path>) -> Result<i32> {
    start_ui();
    let window = new_window();
    WINDOW.store(window as isize, Ordering::SeqCst);
    let app = build(std::env::temp_dir().join("logcourier-self-test"), Config::default(), window);
    app.layout();
    app.show_page(3);
    let pages = app.pages.len();
    // SAFETY: closes the window just made.
    unsafe { DestroyWindow(window) };
    if let Some(output) = output {
        std::fs::write(output, json!({"version": crate::VERSION, "pages": pages, "ok": true}).to_string())?;
    }
    let _ = (&TASKS, MAX_INTERVAL_MINUTES, CONFIG_MAX_BYTES);
    Ok(0)
}
