//! The settings window (the Python's `DesktopApplication` window): a page list on the left, one scrolling panel per page
//! on the right, in plain Win32.
//!
//! The frame window is made at start and stays hidden; the controls are made when the window is first shown and every
//! page when it is first opened, and all of it is destroyed again when the window is hidden, so a program that lives in
//! the tray holds no window resources. The window's state is the settings, the history and the rules, not its controls:
//! each page reads them when it is built.
//!
//! One thread, the message loop's, owns everything here. State sits in one `Ui` kept in a thread-local; no `RefCell` of it
//! is held across a call that can send a message back into the window.

#![cfg_attr(windows, expect(unsafe_code, reason = "calls into Win32"))]
use ks_cast::To as _;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ptr::{null, null_mut};
use std::rc::Rc;
use std::sync::Arc;

use ks_backend::backend::InputBackend;
use ks_config::config::SettingsStore;
use ks_config::history::{HistoryStore, data_dir};
use ks_config::indicator::layout_label;
use ks_config::logsetup::{self, log_directory, log_path, log_status, rotation_summary};
use ks_engine::EngineSnapshot;
use ks_engine::engine_loader::{DeferredEngine, LOADING_ACTION};
use ks_engine::rule_editor::{RuleDraft, build_rule, draft_from_rule};
use serde_json::{Map, Value, json};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{BeginPaint, EndPaint, HDC, PAINTSTRUCT};
use windows_sys::Win32::System::SystemServices::{SS_ENDELLIPSIS, SS_LEFT, SS_NOPREFIX};
use windows_sys::Win32::UI::Controls::{
    DRAWITEMSTRUCT, SetScrollInfo,
    ICC_STANDARD_CLASSES, ICC_UPDOWN_CLASS, ICC_WIN95_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx, NM_DBLCLK, NMHDR, NMUPDOWN,
    TTF_IDISHWND, TTF_SUBCLASS, TTM_ADDTOOLW, TTM_SETTIPBKCOLOR, TTM_SETTIPTEXTCOLOR, TTS_ALWAYSTIP, TTS_NOPREFIX, TTTOOLINFOW,
    UDM_SETPOS32, UDM_SETRANGE32, UDN_DELTAPOS, UDS_ARROWKEYS, UDS_NOTHOUSANDS,
};
use windows_sys::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForSystem, GetDpiForWindow};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BS_OWNERDRAW, BS_PUSHBUTTON, BeginDeferWindowPos, CBN_SELCHANGE, CBS_DROPDOWNLIST, CW_USEDEFAULT, SCROLLINFO, SIF_ALL, SIF_PAGE, SIF_POS, SIF_RANGE,
    DefWindowProcW, DeferWindowPos, EN_KILLFOCUS, EN_SETFOCUS, ES_AUTOHSCROLL, ES_AUTOVSCROLL, ES_MULTILINE, ES_READONLY,
    EndDeferWindowPos, GetWindowRect, IDC_ARROW, IsWindowVisible, KillTimer, LBS_NOTIFY, LoadCursorW, LoadIconW,
    MINMAXINFO, MoveWindow, RegisterClassW, SB_BOTTOM, SB_LINEDOWN, SB_LINEUP, SB_PAGEDOWN, SB_PAGEUP, SB_THUMBTRACK, SB_TOP, SB_VERT,
    SW_HIDE, SW_SCROLLCHILDREN, SW_SHOW,
    SWP_NOACTIVATE, SWP_NOZORDER, ScrollWindowEx, SetTimer, SetWindowPos, ShowWindow, WM_COMMAND,
    WM_CTLCOLORBTN, WM_CTLCOLOREDIT, WM_CTLCOLORLISTBOX, WM_CTLCOLORSTATIC, WM_DPICHANGED, WM_DRAWITEM, WM_ERASEBKGND, WM_GETMINMAXINFO,
    WM_MOUSEWHEEL, WM_NOTIFY, WM_PAINT, WM_SHOWWINDOW, WM_SIZE, WM_TIMER, WM_VSCROLL, WNDCLASSW, WS_BORDER, WS_CHILD,
    WS_CLIPCHILDREN, WS_EX_CLIENTEDGE, WS_EX_CONTROLPARENT, WS_OVERLAPPEDWINDOW, WS_POPUP, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

use crate::constants::{SETTINGS_WINDOW_CLASS, PAGE_CLASS, MAIN_WINDOW_WIDTH_PIXELS, MAIN_WINDOW_HEIGHT_PIXELS, SETTINGS_WINDOW_TITLE, BASE_DPI, MAIN_WINDOW_MIN_WIDTH_PIXELS, MAIN_WINDOW_MIN_HEIGHT_PIXELS, DESKTOP_SIDEBAR_WIDTH_PIXELS, SIDEBAR_PADDING_X_PIXELS, SIDEBAR_PADDING_Y_PIXELS, SIDEBAR_ITEM_PADDING_X_PIXELS, SIDEBAR_BRAND_PADDING_BOTTOM_PIXELS, SIDEBAR_SUBTITLE_PADDING_BOTTOM_PIXELS, NAVIGATION_BUTTON_HEIGHT_PIXELS, NAVIGATION_BUTTON_SPACING_PIXELS, SIDEBAR_SEPARATOR_PADDING_Y_PIXELS, SIDEBAR_STATUS_PADDING_TOP_PIXELS, PAGE_CAPTION_PADDING_TOP_PIXELS, PAGE_PADDING_X_PIXELS, PAGE_CONTENT_PADDING_Y_PIXELS, SECTION_BOTTOM_MARGIN_PIXELS, PAGE_SCROLL_STEP_PIXELS, WHEEL_STEPS_PER_NOTCH, WHEEL_DELTA, APPLICATION_CAPTURE_DELAY_MS, SECTION_PADDING_X_PIXELS, SECTION_PADDING_Y_PIXELS, AUTOCORRECTION_COLUMN_GAP_PIXELS, AUTOCORRECTION_PRIMARY_COLUMN_WEIGHT, AUTOCORRECTION_SECONDARY_COLUMN_WEIGHT, SETTING_ROW_PADDING_PIXELS, MODIFIED_MARKER_COLUMN_PIXELS, RESET_BUTTON_COLUMN_PIXELS, DESCRIPTION_INDENT_PIXELS, CHOICE_CONTROL_WIDTH_PIXELS, NUMBER_CONTROL_WIDTH_PIXELS, TEXT_CONTROL_WIDTH_PIXELS, SETTING_CONTROL_PADDING_LEFT_PIXELS, ROW_HEIGHT_PIXELS, UPDOWN_WIDTH_PIXELS, MODIFIED_MARKER_WIDTH_PIXELS, BUTTON_PADDING_X_PIXELS, BUTTON_HEIGHT_PIXELS, CARD_TITLE_GAP_PIXELS, STATUS_LINE_GAP_PIXELS, TEST_ENTRY_PADDING_TOP_PIXELS, TEST_ENTRY_PADDING_BOTTOM_PIXELS, LAYOUT_LANGUAGE_LABEL_PADDING_TOP_PIXELS, LAYOUT_LANGUAGE_LABEL_PADDING_BOTTOM_PIXELS, MANUAL_SWITCH_NOTE_PADDING_TOP_PIXELS, STANDARD_GAP_PIXELS, APPLICATION_LIST_HEIGHT_ROWS, GROUP_GAP_PIXELS, WORD_LIST_HEIGHT_ROWS, HISTORY_TIME_COLUMN_WIDTH_PIXELS, HISTORY_WORD_COLUMN_WIDTH_PIXELS, HISTORY_APPLICATION_COLUMN_WIDTH_PIXELS, HISTORY_CONFIDENCE_COLUMN_WIDTH_PIXELS, HISTORY_TREE_MIN_ROWS, LEARNING_WORD_COLUMN_WIDTH_PIXELS, LEARNING_CONDITION_COLUMN_WIDTH_PIXELS, LEARNING_ACTION_COLUMN_WIDTH_PIXELS, LEARNING_TREE_HEIGHT_ROWS, DIAGNOSTICS_TEXT_HEIGHT_ROWS};
use crate::win32_controls::{self as w32, Brushes, Fonts, MessageKind, Themed, scaled};
use crate::windows_native::wide;
use crate::windows_system::{WindowsApplicationCatalog, open_directory};
use crate::windows_ui_model::{UpdateController, SettingSpec, ControlKind, UpdateView, dark_from_apps_use_light_theme, update_view, PAGE_NAMES, UpdateSnapshot, py_str, truthy, Palette, palette, dark_theme, stepped_text, choice_value, parse_number, control_text, status_line, string_list, sorted_casefold, with_application, capture_hint_text, catalog_label, with_word, without, history_row, learning_count_text, rule_row, log_state_text, DiagnosticsInput, diagnostics_text, RESET_BUTTON_LABEL, RESET_BUTTON_HINT, autocorrection_settings, trigger_settings, application_settings, manual_switch_settings, hotkey_settings, system_settings, update_settings, diagnostic_settings, locations_text};

const LOGGER: &str = "keyswitch.desktop_ui";
const ID_BASE: usize = 1000;
const IDOK: usize = 1;
const BN_CLICKED: u32 = 0;
const TIMER_CAPTURE: usize = 4;
const WM_APP_REFRESH_RULES: u32 = 0x8000 + 40;

// ------------------------------------------------------------------------------------------------------- the model

/// What the window needs from the rest of the program.
pub struct Services {
    pub settings: Arc<SettingsStore>,
    pub history: Arc<HistoryStore>,
    pub engine: Arc<DeferredEngine>,
    pub backend: Arc<dyn InputBackend>,
    pub updates: Arc<dyn UpdateController>,
    pub catalog: Option<WindowsApplicationCatalog>,
    /// `_autostart_diagnostics`: what the next logon will do, for the report.
    pub autostart_report: Box<dyn Fn() -> Value>,
    /// Messages the frame does not handle itself (the tray, the timers of the application, closing).
    pub host: fn(HWND, u32, WPARAM, LPARAM) -> Option<LRESULT>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Button {
    RefreshDiagnostics,
    CopyDiagnostics,
    AddActive,
    PickExecutable,
    RemoveApplication,
    LoadCatalog,
    AddCatalog,
    AddManual,
    AddWord,
    RemoveWord,
    RefreshHistory,
    ClearHistory,
    CheckUpdates,
    InstallUpdate,
    OpenRelease,
    AddRule,
    EditRule,
    DeleteRules,
    ClearRules,
    ResetSettings,
    OpenLogDirectory,
}

#[derive(Clone, Copy)]
enum Action {
    Navigate(usize),
    Setting(usize),
    Reset(usize),
    Button(Button),
    TestField,
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LabelStyle {
    Normal,
    Muted,
    Error,
    CardTitle,
    CardTitleModified,
    PageTitle,
    PageSubtitle,
}

struct SettingControl {
    spec: SettingSpec,
    title: HWND,
    control: HWND,
    updown: HWND,
    reset: HWND,
}

struct Label {
    hwnd: HWND,
    style: LabelStyle,
    indent: i32,
    top: i32,
}

struct RowCell {
    hwnd: HWND,
    /// Pixels; zero fills what the other cells leave.
    width: i32,
    height: i32,
    /// What the window adds below the field for a drop-down list.
    extra: i32,
    gap: i32,
}

struct SettingNode {
    index: usize,
    kind: ControlKind,
    title: HWND,
    control: HWND,
    updown: HWND,
    reset: HWND,
    description: HWND,
    marker: RECT,
}

enum Node {
    Label(Label),
    Row { cells: Vec<RowCell>, top: i32 },
    Setting(SettingNode),
    Section { title: String, children: Vec<Node>, rect: RECT },
    Columns { left: Vec<Node>, right: Vec<Node> },
    /// A list with a fixed height, or one that takes what the viewport leaves (`fill`) but not less than `height`.
    List { hwnd: HWND, height: i32, top: i32, fill: bool },
}

struct Page {
    panel: HWND,
    nodes: RefCell<Vec<Node>>,
    scroll: Cell<i32>,
    content_height: Cell<i32>,
    styles: RefCell<HashMap<isize, LabelStyle>>,
}

#[derive(Default)]
struct Controls {
    sidebar_layout: HWND,
    sidebar_status: HWND,
    navigation: Vec<HWND>,
    overview_status: HWND,
    overview_layout: HWND,
    overview_count: HWND,
    overview_last: HWND,
    overview_error: HWND,
    application_list: HWND,
    word_list: HWND,
    catalog_combo: HWND,
    manual_application: HWND,
    manual_word: HWND,
    history_list: HWND,
    learning_list: HWND,
    learning_text: HWND,
    update_version: HWND,
    update_status: HWND,
    update_progress: HWND,
    update_check: HWND,
    update_install: HWND,
    update_release: HWND,
    log_state: HWND,
    diagnostics: HWND,
    tooltip: HWND,
    sidebar_labels: Vec<HWND>,
}

struct Resources {
    fonts: Fonts,
    brushes: Brushes,
}

struct State {
    status: String,
    layout: String,
    count: String,
    last_action: String,
    error: String,
    updates: UpdateView,
    catalog: Vec<(String, String)>,
    rules: Vec<ks_config::learning::LearnedRule>,
    snapshot: EngineSnapshot,
}

struct Ui {
    window: HWND,
    services: Services,
    resources: RefCell<Option<Resources>>,
    dark: Cell<Option<bool>>,
    dpi: Cell<i32>,
    active: Cell<usize>,
    pages: RefCell<Vec<Option<Rc<Page>>>>,
    controls: RefCell<Controls>,
    settings_controls: RefCell<Vec<SettingControl>>,
    actions: RefCell<Vec<Action>>,
    state: RefCell<State>,
    built: Cell<bool>,
    test_focused: Cell<bool>,
}

thread_local! {
    static UI: RefCell<Option<Rc<Ui>>> = const { RefCell::new(None) };
}

fn ui() -> Option<Rc<Ui>> {
    UI.with(|slot| slot.borrow().clone())
}

fn with_ui(action: impl FnOnce(&Rc<Ui>)) {
    if let Some(ui) = ui() {
        action(&ui);
    }
}

// ---------------------------------------------------------------------------------------------------- entry points

fn system_dark() -> bool {
    dark_from_apps_use_light_theme(crate::windows_registry::NativeWindowsRegistry.read_apps_use_light_theme())
}

/// Registers the classes and makes the hidden frame window.
pub fn create(services: Services) -> Option<HWND> {
    let controls = INITCOMMONCONTROLSEX {
        dwSize: (size_of::<INITCOMMONCONTROLSEX>()).low::<u32>(),
        dwICC: ICC_WIN95_CLASSES | ICC_STANDARD_CLASSES | ICC_UPDOWN_CLASS,
    };
    // SAFETY: the structure is fully initialised and lives through the call; the class descriptions point at strings that
    // live through RegisterClassW, which copies them; the cursor and icon loads take system or module resources.
    unsafe {
        InitCommonControlsEx(&raw const controls);
        let instance = w32::hinstance();
        let class = wide(SETTINGS_WINDOW_CLASS);
        let frame = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(frame_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: LoadIconW(instance, std::ptr::without_provenance::<u16>(1)),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: class.as_ptr(),
        };
        if RegisterClassW(&raw const frame) == 0 {
            return None;
        }
        let page_class = wide(PAGE_CLASS);
        let page = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(page_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: null_mut(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: page_class.as_ptr(),
        };
        if RegisterClassW(&raw const page) == 0 {
            return None;
        }
    }
    let dpi = unsafe_system_dpi();
    let style = WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN;
    let mut frame = RECT { left: 0, top: 0, right: scaled(MAIN_WINDOW_WIDTH_PIXELS, dpi), bottom: scaled(MAIN_WINDOW_HEIGHT_PIXELS, dpi) };
    // SAFETY: fills the rectangle it is given.
    unsafe { AdjustWindowRectExForDpi(&raw mut frame, style, 0, 0, dpi.bits::<u32>()) };
    let window = w32::create_window(
        SETTINGS_WINDOW_CLASS,
        SETTINGS_WINDOW_TITLE,
        style,
        0,
        null_mut(),
        0,
        (CW_USEDEFAULT, CW_USEDEFAULT, frame.right - frame.left, frame.bottom - frame.top),
    );
    if window.is_null() {
        return None;
    }
    // SAFETY: a plain query for the window just made.
    let window_dpi = (unsafe { GetDpiForWindow(window) }).bits::<i32>();
    let snapshot = services.engine.snapshot();
    let updates = update_view(&services.updates.snapshot());
    let state = State {
        status: "Подготовка движка…".to_owned(),
        layout: "Раскладка: —".to_owned(),
        count: "Исправлений: 0".to_owned(),
        last_action: "Ожидание ввода".to_owned(),
        error: String::new(),
        updates,
        catalog: Vec::new(),
        rules: Vec::new(),
        snapshot,
    };
    let instance = Rc::new(Ui {
        window,
        services,
        resources: RefCell::new(None),
        dark: Cell::new(None),
        dpi: Cell::new(if window_dpi > 0 { window_dpi } else { dpi }),
        active: Cell::new(0),
        pages: RefCell::new((0..PAGE_NAMES.len()).map(|_| None).collect()),
        controls: RefCell::new(Controls::default()),
        settings_controls: RefCell::new(Vec::new()),
        actions: RefCell::new(Vec::new()),
        state: RefCell::new(state),
        built: Cell::new(false),
        test_focused: Cell::new(false),
    });
    UI.with(|slot| *slot.borrow_mut() = Some(instance.clone()));
    let theme = instance.theme_setting();
    instance.apply_theme(&theme, false);
    Some(window)
}

fn unsafe_system_dpi() -> i32 {
    // SAFETY: a plain query.
    let dpi = (unsafe { GetDpiForSystem() }).bits::<i32>();
    if dpi > 0 { dpi } else { BASE_DPI }
}

/// Messages the application's own code sends or posts to the frame; true when the frame has taken it.
#[must_use]
pub fn is_dialog_message(message: &windows_sys::Win32::UI::WindowsAndMessaging::MSG) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::IsDialogMessageW;
    if crate::rule_editor_dialog::is_dialog_message(message) {
        return true;
    }
    let Some(ui) = ui() else { return false };
    // SAFETY: the window is ours and the message is the one the loop just read.
    ui.built.get() && unsafe { IsDialogMessageW(ui.window, message) } != 0
}

/// `show_page` (the page opens when the window is next shown if it is hidden now).
pub fn show_page(name: &str) {
    with_ui(|ui| {
        if let Some(index) = PAGE_NAMES.iter().position(|(page, _)| *page == name) {
            ui.show_page(index);
        }
    });
}

pub fn apply_snapshot(snapshot: &EngineSnapshot) {
    with_ui(|ui| ui.apply_snapshot(snapshot));
}

pub fn set_error(text: &str) {
    with_ui(|ui| {
        text.clone_into(&mut ui.state.borrow_mut().error);
        ui.refresh_status_labels();
    });
}

pub fn set_status_override(text: &str) {
    with_ui(|ui| {
        text.clone_into(&mut ui.state.borrow_mut().status);
        ui.refresh_status_labels();
    });
}

/// The settings store told of a change (`_apply_setting`): the controls follow, the lists and the theme are refreshed.
pub fn apply_setting(path: &str, value: &Value) {
    with_ui(|ui| ui.apply_setting(path, value));
}

pub fn apply_update_snapshot(snapshot: &UpdateSnapshot) {
    with_ui(|ui| ui.apply_update_snapshot(snapshot));
}

pub fn refresh_history() {
    with_ui(|ui| ui.refresh_history());
}

pub fn refresh_learning() {
    with_ui(|ui| ui.refresh_learning());
}

/// The rule window, for a request of the engine (the window the user typed in gets the keyboard back when it closes).
pub fn open_rule_editor(draft: RuleDraft, return_window: Option<i64>) {
    with_ui(|ui| ui.open_rule_editor(draft, return_window));
}

#[must_use]
pub fn dark() -> bool {
    ui().and_then(|ui| ui.dark.get()).unwrap_or(false)
}

/// The window the user is looking at is the frame itself.
#[must_use]
pub fn frame_window() -> Option<HWND> {
    ui().map(|ui| ui.window)
}

/// `present`'s theme step: a window that follows the system asks again each time it is brought forward.
pub fn refresh_theme() {
    with_ui(|ui| {
        let theme = ui.theme_setting();
        ui.apply_theme(&theme, true);
    });
}

/// The test field leaves the keyboard when the window goes away, so the engine does not wait for it.
pub fn destroy_state() {
    with_ui(|ui| ui.release());
    UI.with(|slot| slot.borrow_mut().take());
}

// ------------------------------------------------------------------------------------------------------- window procs

unsafe extern "system" fn frame_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(ui) = ui() else {
        // SAFETY: the default procedure takes the arguments the system gave this one.
        return unsafe { DefWindowProcW(window, message, wparam, lparam) };
    };
    if let Some(result) = ui.frame_message(message, wparam, lparam) {
        return result;
    }
    if let Some(result) = (ui.services.host)(window, message, wparam, lparam) {
        return result;
    }
    // SAFETY: as above.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

unsafe extern "system" fn page_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if let Some(ui) = ui()
        && let Some(result) = ui.page_message(window, message, wparam, lparam)
    {
        return result;
    }
    // SAFETY: the default procedure takes the arguments the system gave this one.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

fn low_word(value: usize) -> usize {
    value & 0xFFFF
}

fn high_word(value: usize) -> u32 {
    ((value >> 16) & 0xFFFF).low::<u32>()
}

impl Ui {
    fn settings(&self) -> &SettingsStore {
        &self.services.settings
    }

    fn theme_setting(&self) -> String {
        self.settings().get("appearance.theme").map_or_else(|| "system".to_owned(), |value| py_str(&value))
    }

    fn setting_bool(&self, path: &str, default: bool) -> bool {
        self.settings().get(path).map_or(default, |value| truthy(&value))
    }

    fn palette(&self) -> Palette {
        palette(self.dark.get().unwrap_or(false))
    }

    // ------------------------------------------------------------------------------------------------- the frame

    fn frame_message(&self, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match message {
            WM_SHOWWINDOW => {
                if wparam != 0 && lparam == 0 {
                    let theme = self.theme_setting();
                    self.apply_theme(&theme, true);
                    self.build();
                } else if wparam == 0 && lparam == 0 {
                    self.release();
                }
                None
            }
            WM_SIZE => {
                self.layout_frame();
                None
            }
            WM_GETMINMAXINFO => {
                let dpi = self.dpi.get();
                let mut outer = RECT { left: 0, top: 0, right: scaled(MAIN_WINDOW_MIN_WIDTH_PIXELS, dpi), bottom: scaled(MAIN_WINDOW_MIN_HEIGHT_PIXELS, dpi) };
                // SAFETY: fills the rectangle; `lparam` points at the structure the system gave for this message.
                unsafe {
                    AdjustWindowRectExForDpi(&raw mut outer, WS_OVERLAPPEDWINDOW, 0, 0, dpi.bits::<u32>());
                    let info = &mut *lparam.pointer_mut::<MINMAXINFO>();
                    info.ptMinTrackSize = POINT { x: outer.right - outer.left, y: outer.bottom - outer.top };
                }
                Some(0)
            }
            WM_DPICHANGED => {
                let dpi = low_word(wparam).low::<i32>();
                // SAFETY: for this message `lparam` points at the suggested rectangle.
                let suggested = unsafe { *lparam.pointer::<RECT>() };
                self.dpi.set(dpi);
                self.release();
                // SAFETY: the window is ours.
                unsafe {
                    SetWindowPos(
                        self.window,
                        null_mut(),
                        suggested.left,
                        suggested.top,
                        suggested.right - suggested.left,
                        suggested.bottom - suggested.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                // SAFETY: a plain query.
                if unsafe { IsWindowVisible(self.window) } != 0 {
                    self.build();
                }
                Some(0)
            }
            WM_ERASEBKGND => {
                self.erase_frame(wparam.pointer_mut::<std::ffi::c_void>());
                Some(1)
            }
            WM_CTLCOLORSTATIC => self.frame_ctlcolor(wparam.pointer_mut::<std::ffi::c_void>(), lparam.pointer_mut::<std::ffi::c_void>()),
            WM_DRAWITEM => {
                // SAFETY: for this message `lparam` points at the structure the system gave.
                let item = unsafe { &*lparam.pointer::<DRAWITEMSTRUCT>() };
                self.draw_item(item);
                Some(1)
            }
            WM_COMMAND => {
                self.command(low_word(wparam), high_word(wparam), lparam.pointer_mut::<std::ffi::c_void>());
                Some(0)
            }
            WM_TIMER if wparam == TIMER_CAPTURE => {
                // SAFETY: the timer belongs to this window.
                unsafe { KillTimer(self.window, TIMER_CAPTURE) };
                self.capture_active_application();
                Some(0)
            }
            WM_APP_REFRESH_RULES => {
                self.refresh_learning();
                Some(0)
            }
            _ => None,
        }
    }

    fn apply_theme(&self, theme: &str, when_changed: bool) {
        let dark = dark_theme(theme, system_dark);
        if when_changed && self.dark.get() == Some(dark) {
            return;
        }
        let rebuild = self.built.get();
        if rebuild {
            self.release();
        }
        self.dark.set(Some(dark));
        w32::set_title_bar_dark(self.window, dark);
        if rebuild {
            self.build();
        }
    }

    /// Makes the controls of the frame and of the page that is open.
    fn build(&self) {
        if self.built.get() {
            return;
        }
        self.built.set(true);
        let dark = self.dark.get().unwrap_or(false);
        let dpi = self.dpi.get();
        *self.resources.borrow_mut() = Some(Resources { fonts: Fonts::new(dpi), brushes: Brushes::new(palette(dark)) });
        self.actions.borrow_mut().clear();
        self.build_sidebar();
        let active = self.active.get();
        self.show_page(active);
        self.layout_frame();
    }

    /// Destroys every control and releases the fonts and brushes.
    fn release(&self) {
        if !self.built.get() {
            return;
        }
        self.built.set(false);
        if self.test_focused.replace(false) {
            self.services.engine.set_test_field_focused(false);
        }
        let pages: Vec<Option<Rc<Page>>> = self.pages.borrow_mut().iter_mut().map(Option::take).collect();
        for page in pages.into_iter().flatten() {
            w32::destroy(page.panel);
        }
        let controls = std::mem::take(&mut *self.controls.borrow_mut());
        for hwnd in controls.navigation.iter().chain(controls.sidebar_labels.iter()) {
            w32::destroy(*hwnd);
        }
        w32::destroy(controls.tooltip);
        self.settings_controls.borrow_mut().clear();
        self.actions.borrow_mut().clear();
        if let Some(resources) = self.resources.borrow_mut().take() {
            resources.fonts.destroy();
            resources.brushes.destroy();
        }
    }

    fn new_id(&self, action: Action) -> usize {
        let mut actions = self.actions.borrow_mut();
        actions.push(action);
        ID_BASE + actions.len() - 1
    }

    fn fonts(&self) -> std::cell::Ref<'_, Fonts> {
        std::cell::Ref::map(self.resources.borrow(), |resources| &resources.as_ref().expect("built").fonts)
    }

    fn build_sidebar(&self) {
        let dpi = self.dpi.get();
        let mut controls = Controls::default();
        let label = |text: &str, font: windows_sys::Win32::Graphics::Gdi::HFONT| {
            let hwnd = w32::create_window("STATIC", text, WS_CHILD | WS_VISIBLE | SS_LEFT | SS_NOPREFIX | SS_ENDELLIPSIS, 0, self.window, 0, (0, 0, 10, 10));
            w32::set_font(hwnd, font);
            hwnd
        };
        let fonts = self.fonts();
        controls.sidebar_labels.push(label("KeySwitch", fonts.brand));
        controls.sidebar_labels.push(label("умная раскладка EN / RU", fonts.base));
        for (index, (_, title)) in PAGE_NAMES.iter().enumerate() {
            let id = self.new_id(Action::Navigate(index));
            let button = w32::create_window("BUTTON", title, WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW.to::<u32>(), 0, self.window, id, (0, 0, 10, 10));
            controls.navigation.push(button);
        }
        let state = self.state.borrow();
        controls.sidebar_layout = label(&state.layout, fonts.base);
        controls.sidebar_status = label(&state.status, fonts.base);
        controls.sidebar_labels.push(controls.sidebar_layout);
        controls.sidebar_labels.push(controls.sidebar_status);
        drop(state);
        // The tooltip of the reset buttons.
        let tooltip = w32::create_window("tooltips_class32", "", WS_POPUP | TTS_ALWAYSTIP | TTS_NOPREFIX, 0, self.window, 0, (CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT));
        let _ = dpi;
        w32::set_font(tooltip, fonts.base);
        // The visual style would ignore the colours: the Python's hint is white on #2b3040 in both schemes.
        let empty = wide("");
        // SAFETY: the strings are NUL-terminated and live through the call.
        unsafe { windows_sys::Win32::UI::Controls::SetWindowTheme(tooltip, empty.as_ptr(), empty.as_ptr()) };
        w32::send(tooltip, TTM_SETTIPBKCOLOR, 0x0040_302b, 0);
        w32::send(tooltip, TTM_SETTIPTEXTCOLOR, 0x00ff_ffff, 0);
        controls.tooltip = tooltip;
        drop(fonts);
        *self.controls.borrow_mut() = controls;
    }

    /// Places the sidebar and the page panels.
    fn layout_frame(&self) {
        if !self.built.get() {
            return;
        }
        let dpi = self.dpi.get();
        let client = w32::client_rect(self.window);
        let sidebar_width = scaled(DESKTOP_SIDEBAR_WIDTH_PIXELS, dpi);
        let (pad_x, pad_y, item_x) =
            (scaled(SIDEBAR_PADDING_X_PIXELS, dpi), scaled(SIDEBAR_PADDING_Y_PIXELS, dpi), scaled(SIDEBAR_ITEM_PADDING_X_PIXELS, dpi));
        let controls = self.controls.borrow();
        let fonts = self.fonts();
        let inner = sidebar_width - 2 * pad_x;
        let mut y = pad_y;
        let place = |hwnd: HWND, x: i32, y: i32, w: i32, h: i32| {
            // SAFETY: the control is ours.
            unsafe { MoveWindow(hwnd, x, y, w, h, 1) };
        };
        let brand_height = w32::line_height(fonts.brand);
        place(controls.sidebar_labels[0], pad_x + item_x, y, inner - item_x, brand_height);
        y += brand_height + scaled(SIDEBAR_BRAND_PADDING_BOTTOM_PIXELS, dpi);
        let line = w32::line_height(fonts.base);
        place(controls.sidebar_labels[1], pad_x + item_x, y, inner - item_x, line);
        y += line + scaled(SIDEBAR_SUBTITLE_PADDING_BOTTOM_PIXELS, dpi);
        for button in &controls.navigation {
            let height = scaled(NAVIGATION_BUTTON_HEIGHT_PIXELS, dpi);
            place(*button, pad_x, y, inner, height);
            y += height + scaled(NAVIGATION_BUTTON_SPACING_PIXELS, dpi);
        }
        y += scaled(SIDEBAR_SEPARATOR_PADDING_Y_PIXELS, dpi) * 2 + 1;
        place(controls.sidebar_layout, pad_x + item_x, y, inner - item_x, line);
        y += line + scaled(SIDEBAR_STATUS_PADDING_TOP_PIXELS, dpi);
        place(controls.sidebar_status, pad_x + item_x, y, inner - item_x, line);
        drop(fonts);
        drop(controls);
        let panels: Vec<HWND> = self.pages.borrow().iter().flatten().map(|page| page.panel).collect();
        for panel in panels {
            place(panel, sidebar_width, 0, (client.right - sidebar_width).max(0), client.bottom);
        }
        w32::invalidate(self.window);
    }

    fn erase_frame(&self, hdc: HDC) {
        let Some(resources) = self.resources.borrow().as_ref().map(|resources| (resources.brushes.sidebar, resources.brushes.background, resources.brushes.palette)) else {
            return;
        };
        let (sidebar, background, palette) = resources;
        let dpi = self.dpi.get();
        let client = w32::client_rect(self.window);
        let width = scaled(DESKTOP_SIDEBAR_WIDTH_PIXELS, dpi);
        w32::fill_rect(hdc, &RECT { left: 0, top: 0, right: width, bottom: client.bottom }, sidebar);
        w32::fill_rect(hdc, &RECT { left: width, top: 0, right: client.right, bottom: client.bottom }, background);
        // The separator under the navigation.
        if let Some(button) = self.controls.borrow().navigation.last().copied() {
            let rect = window_rect_in(self.window, button);
            let y = rect.bottom + scaled(SIDEBAR_SEPARATOR_PADDING_Y_PIXELS, dpi);
            let pad = scaled(SIDEBAR_PADDING_X_PIXELS, dpi);
            let brush = w32::solid_brush(palette.sidebar_muted);
            w32::fill_rect(hdc, &RECT { left: pad, top: y, right: width - pad, bottom: y + 1 }, brush);
            w32::delete_object(brush);
        }
    }

    fn frame_ctlcolor(&self, hdc: HDC, control: HWND) -> Option<LRESULT> {
        let resources = self.resources.borrow();
        let resources = resources.as_ref()?;
        let palette = resources.brushes.palette;
        let controls = self.controls.borrow();
        let muted = controls.sidebar_labels.get(1) == Some(&control) || controls.sidebar_status == control;
        w32::set_label_colors(hdc, if muted { palette.sidebar_muted } else { palette.sidebar_text }.colorref(), palette.sidebar.colorref());
        Some(resources.brushes.sidebar.to::<LRESULT>())
    }

    fn draw_item(&self, item: &DRAWITEMSTRUCT) {
        let id = item.CtlID.to::<usize>();
        let action = id.checked_sub(ID_BASE).and_then(|index| self.actions.borrow().get(index).copied());
        let resources = self.resources.borrow();
        let Some(resources) = resources.as_ref() else { return };
        match action {
            Some(Action::Navigate(index)) => {
                let selected = index == self.active.get();
                let palette = resources.brushes.palette;
                let color = if selected { palette.accent } else { palette.sidebar };
                let brush = w32::solid_brush(color);
                w32::draw_navigation(item, brush, palette.sidebar_text.colorref(), resources.fonts.base, self.dpi.get());
                w32::delete_object(brush);
            }
            Some(Action::Setting(_)) => w32::draw_choice(item, &resources.brushes, resources.fonts.base, self.dpi.get(), false),
            _ => {}
        }
    }

    // ----------------------------------------------------------------------------------------------- the pages

    fn show_page(&self, index: usize) {
        self.active.set(index);
        if !self.built.get() {
            return;
        }
        let page = self.ensure_page(index);
        for (other, slot) in self.pages.borrow().iter().enumerate() {
            if let Some(slot) = slot
                && other != index
            {
                // SAFETY: the panel is ours.
                unsafe { ShowWindow(slot.panel, SW_HIDE) };
            }
        }
        // SAFETY: the panel is ours.
        unsafe { ShowWindow(page.panel, SW_SHOW) };
        for button in &self.controls.borrow().navigation {
            w32::invalidate(*button);
        }
        match PAGE_NAMES[index].0 {
            "history" => self.refresh_history(),
            "exclusions" => {
                self.refresh_exclusion_lists();
                if self.state.borrow().catalog.is_empty() {
                    self.load_catalog();
                }
            }
            "updates" => self.show_update_view(&update_view(&self.services.updates.snapshot())),
            "maintenance" => {
                self.refresh_learning();
                self.refresh_log_state();
            }
            "about" => self.refresh_diagnostics(),
            _ => {}
        }
        self.relayout(&page);
    }

    fn ensure_page(&self, index: usize) -> Rc<Page> {
        if let Some(page) = self.pages.borrow()[index].clone() {
            return page;
        }
        let client = w32::client_rect(self.window);
        let sidebar_width = scaled(DESKTOP_SIDEBAR_WIDTH_PIXELS, self.dpi.get());
        let panel = w32::create_window(
            PAGE_CLASS,
            "",
            WS_CHILD | WS_CLIPCHILDREN | WS_VSCROLL,
            WS_EX_CONTROLPARENT,
            self.window,
            0,
            (sidebar_width, 0, (client.right - sidebar_width).max(0), client.bottom),
        );
        w32::apply_control_theme(panel, self.dark.get().unwrap_or(false), Themed::Explorer);
        let page = Rc::new(Page {
            panel,
            nodes: RefCell::new(Vec::new()),
            scroll: Cell::new(0),
            content_height: Cell::new(0),
            styles: RefCell::new(HashMap::new()),
        });
        self.pages.borrow_mut()[index] = Some(page.clone());
        let nodes = {
            let mut builder = Builder { ui: self, page: &page, panel };
            let (title, subtitle) = self.page_texts(index);
            let mut nodes = vec![
                builder.label(&title, LabelStyle::PageTitle, 0, 0),
                builder.label(&subtitle, LabelStyle::PageSubtitle, 0, scaled(PAGE_CAPTION_PADDING_TOP_PIXELS, self.dpi.get())),
            ];
            nodes.extend(match PAGE_NAMES[index].0 {
                "overview" => builder.overview(),
                "autocorrection" => builder.autocorrection(),
                "layouts" => builder.layouts(),
                "hotkeys" => builder.hotkeys(),
                "exclusions" => builder.exclusions(),
                "appearance" => builder.appearance(),
                "history" => builder.history(),
                "updates" => builder.updates(),
                "maintenance" => builder.maintenance(),
                _ => builder.about(),
            });
            nodes
        };
        *page.nodes.borrow_mut() = nodes;
        page
    }

    fn page_texts(&self, index: usize) -> (String, String) {
        let (title, subtitle) = match PAGE_NAMES[index].0 {
            "overview" => ("KeySwitch работает рядом с вами", "Автоматическая замена раскладки выполняется локально после завершения слова."),
            "autocorrection" => ("Автокоррекция", "Настройте точность распознавания, обучение и события завершения слова."),
            "layouts" => ("Раскладки EN / RU", "KeySwitch находит установленные английскую и русскую HKL и сохраняет выбранный пользователем язык."),
            "hotkeys" => ("Горячие клавиши", "Комбинации работают глобально. Используйте названия Ctrl, Alt, Shift, Super, Pause и букв."),
            "exclusions" => ("Исключения", "В исключённых программах KeySwitch наблюдает раскладку, но не заменяет введённые слова."),
            "appearance" => ("Внешний вид и система", "Настройте автозагрузку после входа в Windows, трей, уведомления и локальную историю."),
            "history" => ("История исправлений", "Сохраняются только пары слов, время, приложение и уверенность — не полный поток клавиатуры."),
            "updates" => ("Обновления", "KeySwitch проверяет стабильные выпуски, сверяет SHA-256 и может тихо обновиться с автоматическим перезапуском."),
            "maintenance" => ("Обслуживание", "Управляйте локальными правилами, настройками и расположением данных KeySwitch."),
            _ => return (format!("KeySwitch {}", crate::windows_app::VERSION), "Локальное приложение автоматического исправления раскладки для Windows, Linux и macOS.".to_owned()),
        };
        (title.to_owned(), subtitle.to_owned())
    }

    fn page_of(&self, panel: HWND) -> Option<Rc<Page>> {
        self.pages.borrow().iter().flatten().find(|page| page.panel == panel).cloned()
    }

    // ------------------------------------------------------------------------------------------ page layout

    fn relayout(&self, page: &Rc<Page>) {
        let Ok(mut nodes) = page.nodes.try_borrow_mut() else { return };
        if !self.built.get() {
            return;
        }
        let dpi = self.dpi.get();
        let mut client = w32::client_rect(page.panel);
        for pass in 0..2 {
            let width = client.right - 2 * scaled(PAGE_PADDING_X_PIXELS, dpi);
            let mut context = Layout { dpi, fonts: &self.fonts(), moves: Vec::new(), viewport: client.bottom, page };
            let end = layout_nodes(&mut nodes, scaled(PAGE_PADDING_X_PIXELS, dpi), width.max(scaled(120, dpi)), scaled(PAGE_CONTENT_PADDING_Y_PIXELS, dpi), &mut context);
            let content = end + scaled(PAGE_CONTENT_PADDING_Y_PIXELS, dpi) - scaled(SECTION_BOTTOM_MARGIN_PIXELS, dpi).min(end);
            page.content_height.set(content);
            let maximum = (content - client.bottom).max(0);
            let scroll = page.scroll.get().clamp(0, maximum);
            let moves = std::mem::take(&mut context.moves);
            drop(context);
            if scroll != page.scroll.get() {
                page.scroll.set(scroll);
                continue;
            }
            apply_moves(&moves, scroll);
            let info = SCROLLINFO { cbSize: (size_of::<SCROLLINFO>()).low::<u32>(), fMask: SIF_RANGE | SIF_PAGE | SIF_POS, nMin: 0, nMax: content.max(1) - 1, nPage: client.bottom.max(0).bits::<u32>(), nPos: scroll, nTrackPos: 0 };
            // SAFETY: the structure is fully initialised and lives through the call.
            unsafe { SetScrollInfo(page.panel, SB_VERT, &raw const info, 1) };
            // A scroll bar that appeared or went changes the width the page has: lay out once more.
            let after = w32::client_rect(page.panel);
            if after.right == client.right || pass == 1 {
                break;
            }
            client = after;
        }
        w32::invalidate(page.panel);
    }

    fn scroll_to(&self, page: &Rc<Page>, target: i32) {
        let client = w32::client_rect(page.panel);
        let maximum = (page.content_height.get() - client.bottom).max(0);
        let target = target.clamp(0, maximum);
        let delta = target - page.scroll.get();
        if delta == 0 {
            return;
        }
        page.scroll.set(target);
        let info = SCROLLINFO { cbSize: (size_of::<SCROLLINFO>()).low::<u32>(), fMask: SIF_POS, nMin: 0, nMax: 0, nPage: 0, nPos: target, nTrackPos: 0 };
        // SAFETY: the structure is fully initialised; the panel is ours; scrolling moves its children with it.
        unsafe {
            SetScrollInfo(page.panel, SB_VERT, &raw const info, 1);
            ScrollWindowEx(page.panel, 0, -delta, null(), null(), null_mut(), null_mut(), SW_SCROLLCHILDREN | 0x0002 | 0x0004);
        }
    }

    fn page_message(&self, panel: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match message {
            WM_ERASEBKGND => {
                let brush = self.resources.borrow().as_ref().map(|resources| resources.brushes.content)?;
                let rect = w32::client_rect(panel);
                w32::fill_rect(wparam.pointer_mut::<std::ffi::c_void>(), &rect, brush);
                Some(1)
            }
            WM_PAINT => {
                self.paint_page(panel);
                Some(0)
            }
            WM_SIZE => {
                if let Some(page) = self.page_of(panel) {
                    self.relayout(&page);
                }
                Some(0)
            }
            WM_VSCROLL => {
                let page = self.page_of(panel)?;
                let client = w32::client_rect(panel);
                let step = scaled(PAGE_SCROLL_STEP_PIXELS, self.dpi.get());
                let mut info = SCROLLINFO { cbSize: (size_of::<SCROLLINFO>()).low::<u32>(), fMask: SIF_ALL, nMin: 0, nMax: 0, nPage: 0, nPos: 0, nTrackPos: 0 };
                // SAFETY: the structure is initialised and lives through the call.
                unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetScrollInfo(panel, SB_VERT, &raw mut info) };
                let current = page.scroll.get();
                let target = match low_word(wparam).low::<i32>() {
                    code if code == SB_LINEUP => current - step,
                    code if code == SB_LINEDOWN => current + step,
                    code if code == SB_PAGEUP => current - client.bottom,
                    code if code == SB_PAGEDOWN => current + client.bottom,
                    code if code == SB_TOP => 0,
                    code if code == SB_BOTTOM => page.content_height.get(),
                    code if code == SB_THUMBTRACK => info.nTrackPos,
                    _ => current,
                };
                self.scroll_to(&page, target);
                Some(0)
            }
            WM_MOUSEWHEEL => {
                let page = self.page_of(panel)?;
                let notches = ((wparam >> 16).low::<u16>()).bits::<i16>().to::<i32>();
                let lines = -notches * WHEEL_STEPS_PER_NOTCH * scaled(PAGE_SCROLL_STEP_PIXELS, self.dpi.get()) / WHEEL_DELTA;
                self.scroll_to(&page, page.scroll.get() + lines);
                Some(0)
            }
            WM_COMMAND => {
                self.command(low_word(wparam), high_word(wparam), lparam.pointer_mut::<std::ffi::c_void>());
                Some(0)
            }
            WM_NOTIFY => {
                // SAFETY: for this message `lparam` points at a notification structure that starts with an NMHDR.
                let header = unsafe { &*lparam.pointer::<NMHDR>() };
                Some(self.notify(header, lparam))
            }
            WM_DRAWITEM => {
                // SAFETY: for this message `lparam` points at the structure the system gave.
                let item = unsafe { &*lparam.pointer::<DRAWITEMSTRUCT>() };
                self.draw_item(item);
                Some(1)
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN => {
                let page = self.page_of(panel)?;
                let resources = self.resources.borrow();
                let resources = resources.as_ref()?;
                let palette = resources.brushes.palette;
                let hdc = wparam.pointer_mut::<std::ffi::c_void>();
                let control = lparam.pointer_mut::<std::ffi::c_void>();
                if message == WM_CTLCOLORSTATIC {
                    let style = page.styles.borrow().get(&control.to::<isize>()).copied().unwrap_or(LabelStyle::Normal);
                    let color = match style {
                        LabelStyle::Normal | LabelStyle::CardTitle | LabelStyle::PageTitle => palette.foreground,
                        LabelStyle::Muted | LabelStyle::PageSubtitle => palette.muted,
                        LabelStyle::Error => palette.error,
                        LabelStyle::CardTitleModified => palette.accent,
                    };
                    w32::set_label_colors(hdc, color.colorref(), palette.content.colorref());
                    return Some(resources.brushes.content.to::<LRESULT>());
                }
                w32::set_label_colors(hdc, palette.foreground.colorref(), palette.field.colorref());
                Some(resources.brushes.field.to::<LRESULT>())
            }
            _ => None,
        }
    }

    fn paint_page(&self, panel: HWND) {
        // SAFETY: PAINTSTRUCT is plain data filled by BeginPaint.
        let mut paint: PAINTSTRUCT = unsafe { std::mem::zeroed() };
        // SAFETY: balanced with EndPaint below on the same window.
        let hdc = unsafe { BeginPaint(panel, &raw mut paint) };
        if let (Some(page), Some(resources)) = (self.page_of(panel), self.resources.borrow().as_ref()) {
            let palette = resources.brushes.palette;
            let scroll = page.scroll.get();
            let dpi = self.dpi.get();
            if let Ok(nodes) = page.nodes.try_borrow() {
                paint_nodes(&nodes, hdc, &PaintContext { scroll, dpi, palette, frame_font: resources.fonts.frame, settings: self.settings(), controls: &self.settings_controls.borrow() });
            }
        }
        // SAFETY: ends the paint begun above.
        unsafe { EndPaint(panel, &raw const paint) };
    }

    // ------------------------------------------------------------------------------------------- the commands

    fn command(&self, id: usize, code: u32, control: HWND) {
        if !self.built.get() {
            return;
        }
        if id == IDOK {
            // Enter in a text or number box saves it, as the Python's <Return> binding does.
            // SAFETY: a plain query.
            let focus = unsafe { GetFocus() };
            let index = self.settings_controls.borrow().iter().position(|setting| setting.control == focus);
            if let Some(index) = index {
                self.save_control(index);
            }
            return;
        }
        let Some(action) = id.checked_sub(ID_BASE).and_then(|index| self.actions.borrow().get(index).copied()) else { return };
        match action {
            Action::Navigate(index) if code == BN_CLICKED => self.show_page(index),
            Action::Setting(index) => {
                let kind = self.settings_controls.borrow().get(index).map(|setting| setting.spec.kind);
                match kind {
                    Some(ControlKind::Bool) if code == BN_CLICKED => {
                        let checked = !w32::is_checked(control);
                        w32::set_checked(control, checked);
                        self.save_control(index);
                    }
                    Some(ControlKind::Choice) if code == CBN_SELCHANGE => self.save_control(index),
                    Some(ControlKind::Int | ControlKind::Float | ControlKind::Text) if code == EN_KILLFOCUS => self.save_control(index),
                    _ => {}
                }
            }
            Action::Reset(index) if code == BN_CLICKED => self.restore_default(index),
            Action::Button(button) if code == BN_CLICKED => self.button(button),
            Action::TestField => match code {
                EN_SETFOCUS => {
                    self.test_focused.set(true);
                    self.services.engine.set_test_field_focused(true);
                }
                EN_KILLFOCUS => {
                    self.test_focused.set(false);
                    self.services.engine.set_test_field_focused(false);
                }
                _ => {}
            },
            Action::Navigate(_) | Action::Reset(_) | Action::Button(_) | Action::Other => {}
        }
    }

    fn notify(&self, header: &NMHDR, lparam: LPARAM) -> LRESULT {
        if header.code == NM_DBLCLK && header.hwndFrom == self.controls.borrow().learning_list {
            self.edit_selected_rule();
            return 0;
        }
        if header.code == UDN_DELTAPOS {
            // SAFETY: for this notification `lparam` points at an NMUPDOWN.
            let delta = unsafe { (*lparam.pointer::<NMUPDOWN>()).iDelta };
            let index = self.settings_controls.borrow().iter().position(|setting| setting.updown == header.hwndFrom);
            if let Some(index) = index {
                // The arrow up (a negative delta in the control's terms, positions run the other way) raises the value.
                let (spec, edit) = {
                    let settings = self.settings_controls.borrow();
                    (settings[index].spec.clone(), settings[index].control)
                };
                let shown = stepped_text(&spec, &w32::text_of(edit), -delta);
                w32::set_text(edit, &shown);
                self.save_control(index);
            }
            // Keep the arrows' own position where it is: the value is the text.
            return 1;
        }
        0
    }

    fn button(&self, button: Button) {
        match button {
            Button::RefreshDiagnostics | Button::CopyDiagnostics => {
                if button == Button::CopyDiagnostics {
                    w32::set_clipboard_text(self.window, &self.diagnostics());
                } else {
                    self.refresh_diagnostics();
                }
            }
            Button::AddActive => self.add_active_application(),
            Button::PickExecutable => self.pick_executable(),
            Button::RemoveApplication => self.remove_listed("exclusions.applications", self.controls.borrow().application_list),
            Button::LoadCatalog => self.load_catalog(),
            Button::AddCatalog => self.add_catalog_application(),
            Button::AddManual => self.add_manual_application(),
            Button::AddWord => self.add_word(),
            Button::RemoveWord => self.remove_listed("exclusions.words", self.controls.borrow().word_list),
            Button::RefreshHistory => self.refresh_history(),
            Button::ClearHistory => {
                if w32::message_box(self.window, "Очистить историю", "Удалить все сохранённые пары исправлений?", MessageKind::Question)
                    && let Err(error) = self.services.history.clear()
                {
                    logsetup::warning(LOGGER, &format!("Не удалось очистить историю: {error}"));
                }
            }
            Button::CheckUpdates => {
                if !self.services.updates.check(false, false) {
                    self.show_update_status("Проверка обновлений уже выполняется");
                }
            }
            Button::InstallUpdate => {
                if !self.services.updates.install_available() {
                    self.show_update_status("Сначала проверьте наличие новой версии");
                }
            }
            Button::OpenRelease => self.open_update_release(),
            Button::AddRule => self.open_rule_editor(RuleDraft::default(), None),
            Button::EditRule => self.edit_selected_rule(),
            Button::DeleteRules => {
                for rule in self.selected_rules() {
                    let _ = self.services.engine.learning.remove_rule(&rule);
                }
                self.refresh_learning();
            }
            Button::ClearRules => {
                if w32::message_box(self.window, "Удалить все правила", "Удалить все правила переключения? Отменить это нельзя.", MessageKind::Question) {
                    let _ = self.services.engine.learning.clear();
                    self.refresh_learning();
                }
            }
            Button::ResetSettings => {
                if w32::message_box(self.window, "Сбросить настройки", "Вернуть все параметры KeySwitch по умолчанию?", MessageKind::Question)
                    && let Err(error) = self.settings().reset()
                {
                    logsetup::warning(LOGGER, &format!("Не удалось сбросить настройки: {error}"));
                }
            }
            Button::OpenLogDirectory => {
                if let Err(error) = open_directory(&log_directory(), None) {
                    logsetup::error(LOGGER, &format!("Не удалось открыть папку журнала: {error}"));
                    w32::message_box(self.window, "Папка журнала", &format!("Не удалось открыть папку журнала: {error}"), MessageKind::Error);
                }
            }
        }
    }

    // ----------------------------------------------------------------------------------------------- settings

    /// `_save_boolean`, `_save_choice`, `_save_number` and `_save_text`: the control's value goes to the store.
    fn save_control(&self, index: usize) {
        let (spec, control) = {
            let settings = self.settings_controls.borrow();
            let Some(setting) = settings.get(index) else { return };
            (setting.spec.clone(), setting.control)
        };
        let title = self.settings_controls.borrow()[index].title;
        let stored = match spec.kind {
            ControlKind::Bool => Some(Value::Bool(w32::is_checked(title))),
            ControlKind::Choice => w32::combo_selected_text(control).map(|label| Value::String(choice_value(&spec, &label))),
            ControlKind::Int | ControlKind::Float => if let Some((value, shown)) = parse_number(&spec, &w32::text_of(control)) {
                w32::set_text(control, &shown);
                Some(value)
            } else {
                let stored = self.settings().get(&spec.path).map_or_else(|| py_str(&json!(spec.minimum)), |value| py_str(&value));
                w32::set_text(control, &stored);
                None
            },
            ControlKind::Text => {
                let value = w32::text_of(control);
                let value = ks_text::pystr::strip(&value);
                if value.is_empty() { None } else { Some(Value::String(value.to_owned())) }
            }
        };
        if let Some(value) = stored {
            if let Err(error) = self.settings().set(&spec.path, value, true) {
                logsetup::warning(LOGGER, &format!("Не удалось сохранить настройку {}: {error}", spec.path));
            }
            self.update_modified(&spec.path);
            self.relayout_all();
        }
    }

    fn restore_default(&self, index: usize) {
        let path = self.settings_controls.borrow()[index].spec.path.clone();
        if !matches!(self.settings().restore_default(&path), Ok(true)) {
            return;
        }
        // The controls follow the store through the queue; showing the restored value now keeps the button honest.
        let value = self.settings().get(&path);
        self.apply_control_value(&path, value.as_ref());
        self.update_modified(&path);
        self.relayout_all();
    }

    /// `_apply_control_value`: every control of this setting shows the stored value.
    fn apply_control_value(&self, path: &str, value: Option<&Value>) {
        for setting in self.settings_controls.borrow().iter().filter(|setting| setting.spec.path == path) {
            match setting.spec.kind {
                ControlKind::Bool => w32::set_checked(setting.title, value.is_some_and(truthy)),
                ControlKind::Choice => {
                    w32::combo_select_text(setting.control, &control_text(&setting.spec, value));
                }
                ControlKind::Int | ControlKind::Float | ControlKind::Text => w32::set_text(setting.control, &control_text(&setting.spec, value)),
            }
        }
    }

    /// `_update_modified_indicator`: the marker, the reset button and the label colour of a setting changed from its default.
    fn update_modified(&self, path: &str) {
        let modified = !self.settings().is_default(path);
        for setting in self.settings_controls.borrow().iter().filter(|setting| setting.spec.path == path) {
            // SAFETY: the reset button is ours.
            unsafe { ShowWindow(setting.reset, if modified { SW_SHOW } else { SW_HIDE }) };
            if setting.spec.kind == ControlKind::Bool {
                w32::set_modified(setting.title, modified);
            } else if let Some(page) = self.pages.borrow().iter().flatten().find(|page| page.styles.borrow().contains_key(&setting.title.to::<isize>())) {
                page.styles.borrow_mut().insert(
                    setting.title.to::<isize>(),
                    if modified { LabelStyle::CardTitleModified } else { LabelStyle::CardTitle },
                );
                w32::invalidate(setting.title);
            }
        }
    }

    /// A reset button that came back from hidden sits where the last scroll left it: every page is laid out again.
    fn relayout_all(&self) {
        let pages: Vec<Rc<Page>> = self.pages.borrow().iter().flatten().cloned().collect();
        for page in pages {
            self.relayout(&page);
        }
    }

    fn apply_setting(&self, path: &str, value: &Value) {
        if path == "*" {
            let paths: Vec<String> = self.settings_controls.borrow().iter().map(|setting| setting.spec.path.clone()).collect();
            for path in paths {
                let value = self.settings().get(&path);
                self.apply_control_value(&path, value.as_ref());
                self.update_modified(&path);
            }
            self.relayout_all();
            let theme = self.theme_setting();
            self.apply_theme(&theme, false);
            self.refresh_exclusion_lists();
            return;
        }
        self.update_modified(path);
        self.apply_control_value(path, Some(value));
        self.relayout_all();
        match path {
            "appearance.theme" => self.apply_theme(&py_str(value), false),
            "exclusions.applications" | "exclusions.words" => self.refresh_exclusion_lists(),
            _ => {}
        }
    }

    // --------------------------------------------------------------------------------------------- the status

    fn apply_snapshot(&self, snapshot: &EngineSnapshot) {
        {
            let mut state = self.state.borrow_mut();
            status_line(snapshot.enabled, snapshot.running).clone_into(&mut state.status);
            state.layout = format!("Раскладка: {}", layout_label(snapshot.current_group));
            state.count = format!("Исправлений: {}", snapshot.correction_count);
            state.last_action.clone_from(&snapshot.last_action);
            state.error.clone_from(&snapshot.last_error);
            state.snapshot = snapshot.clone();
        }
        self.refresh_status_labels();
    }

    fn refresh_status_labels(&self) {
        if !self.built.get() {
            return;
        }
        let state = self.state.borrow();
        let controls = self.controls.borrow();
        let pairs = [
            (controls.sidebar_layout, &state.layout),
            (controls.sidebar_status, &state.status),
            (controls.overview_status, &state.status),
            (controls.overview_layout, &state.layout),
            (controls.overview_count, &state.count),
            (controls.overview_last, &state.last_action),
            (controls.overview_error, &state.error),
        ];
        for (hwnd, text) in pairs {
            if !hwnd.is_null() && w32::text_of(hwnd) != *text {
                w32::set_text(hwnd, text);
            }
        }
        drop(controls);
        drop(state);
        let overview = self.pages.borrow()[0].clone();
        if let Some(page) = overview {
            self.relayout(&page);
        }
    }

    fn apply_update_snapshot(&self, snapshot: &UpdateSnapshot) {
        let view = update_view(snapshot);
        self.show_update_view(&view);
    }

    fn show_update_view(&self, view: &UpdateView) {
        self.state.borrow_mut().updates = view.clone();
        let controls = self.controls.borrow();
        if controls.update_status.is_null() {
            return;
        }
        w32::set_text(controls.update_version, &view.version);
        w32::set_text(controls.update_status, &view.status);
        w32::set_text(controls.update_progress, &view.progress);
        for (button, enabled) in [(controls.update_check, view.check_enabled), (controls.update_install, view.install_enabled), (controls.update_release, view.release_enabled)] {
            // SAFETY: the button is ours.
            unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow(button, i32::from(enabled)) };
        }
        drop(controls);
        let page = self.pages.borrow()[PAGE_NAMES.iter().position(|(name, _)| *name == "updates").unwrap_or(0)].clone();
        if let Some(page) = page {
            self.relayout(&page);
        }
    }

    fn show_update_status(&self, text: &str) {
        let status = self.controls.borrow().update_status;
        if !status.is_null() {
            w32::set_text(status, text);
        }
    }

    fn open_update_release(&self) {
        let url = self.services.updates.snapshot().release_url;
        if url.is_empty() {
            self.show_update_status("Сначала проверьте наличие новой версии");
            return;
        }
        let verb = wide("open");
        let target = wide(&url);
        // SAFETY: both strings are NUL-terminated and outlive the call.
        let result = unsafe { windows_sys::Win32::UI::Shell::ShellExecuteW(self.window, verb.as_ptr(), target.as_ptr(), null(), null(), 1) };
        if result.to::<isize>() <= 32 {
            self.show_update_status("Не удалось открыть страницу выпуска");
        }
    }

    // ----------------------------------------------------------------------------------------------- exclusions

    fn string_list(&self, path: &str) -> Vec<String> {
        string_list(self.settings().get(path).as_ref())
    }

    fn refresh_exclusion_lists(&self) {
        let controls = self.controls.borrow();
        if !controls.application_list.is_null() {
            w32::listbox_reset(controls.application_list, &sorted_casefold(self.string_list("exclusions.applications")));
        }
        if !controls.word_list.is_null() {
            w32::listbox_reset(controls.word_list, &sorted_casefold(self.string_list("exclusions.words")));
        }
    }

    fn store_list(&self, path: &str, list: Vec<String>) {
        let value = Value::Array(list.into_iter().map(Value::String).collect());
        if let Err(error) = self.settings().set(path, value, true) {
            logsetup::warning(LOGGER, &format!("Не удалось сохранить настройку {path}: {error}"));
        }
    }

    fn add_application(&self, identifier: &str) {
        if let Some(list) = with_application(self.string_list("exclusions.applications"), identifier) {
            self.store_list("exclusions.applications", list);
        }
    }

    fn add_active_application(&self) {
        w32::message_box(self.window, "Выбор окна", &capture_hint_text(), MessageKind::Information);
        {
            "Выберите приложение-исключение…".clone_into(&mut self.state.borrow_mut().status);
        }
        self.refresh_status_labels();
        // SAFETY: the window and the timer are ours.
        unsafe {
            ShowWindow(self.window, SW_HIDE);
            SetTimer(self.window, TIMER_CAPTURE, APPLICATION_CAPTURE_DELAY_MS, None);
        }
    }

    fn capture_active_application(&self) {
        let identifier = self.services.backend.active_application();
        if identifier.is_empty() {
            w32::message_box(self.window, "KeySwitch", "Не удалось определить выбранное приложение.", MessageKind::Information);
        } else {
            self.add_application(&identifier);
            self.state.borrow_mut().status = format!("Добавлено исключение: {identifier}");
        }
        self.show_page(PAGE_NAMES.iter().position(|(name, _)| *name == "exclusions").unwrap_or(0));
        self.present();
    }

    /// `present`: the window comes forward, in the colours the system has now.
    fn present(&self) {
        // SAFETY: the window is ours; SW_RESTORE shows a hidden or minimised window.
        unsafe {
            ShowWindow(self.window, windows_sys::Win32::UI::WindowsAndMessaging::SW_RESTORE);
            windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow(self.window);
        }
    }

    fn pick_executable(&self) {
        if let Some(path) = w32::pick_executable(self.window, "Выберите приложение-исключение")
            && let Some(application) = WindowsApplicationCatalog::from_executable(&path)
        {
            self.add_application(&application.identifier);
        }
    }

    fn load_catalog(&self) {
        let Some(catalog) = self.services.catalog.as_ref() else { return };
        match catalog.installed() {
            Ok(applications) => {
                let items: Vec<(String, String)> = applications
                    .into_iter()
                    .map(|application| (catalog_label(&application.name, &application.identifier), application.identifier))
                    .collect();
                let labels: Vec<String> = items.iter().map(|(label, _)| label.clone()).collect();
                let combo = self.controls.borrow().catalog_combo;
                if !combo.is_null() {
                    w32::combo_reset(combo, &labels);
                    if let Some(first) = labels.first() {
                        w32::combo_select_text(combo, first);
                    }
                }
                self.state.borrow_mut().catalog = items;
            }
            Err(error) => {
                self.state.borrow_mut().error = format!("Каталог приложений: {error}");
                self.refresh_status_labels();
            }
        }
    }

    fn add_catalog_application(&self) {
        let combo = self.controls.borrow().catalog_combo;
        let Some(index) = w32::combo_selected_index(combo) else { return };
        let identifier = self.state.borrow().catalog.get(index).map(|(_, identifier)| identifier.clone());
        if let Some(identifier) = identifier {
            self.add_application(&identifier);
        }
    }

    fn add_manual_application(&self) {
        let entry = self.controls.borrow().manual_application;
        if let Some(application) = WindowsApplicationCatalog::from_executable(&w32::text_of(entry)) {
            self.add_application(&application.identifier);
            w32::set_text(entry, "");
        }
    }

    fn add_word(&self) {
        let entry = self.controls.borrow().manual_word;
        let word = w32::text_of(entry);
        if ks_text::pystr::strip(&word).is_empty() {
            return;
        }
        if let Some(list) = with_word(self.string_list("exclusions.words"), &word) {
            self.store_list("exclusions.words", list);
        }
        w32::set_text(entry, "");
    }

    fn remove_listed(&self, path: &str, list: HWND) {
        if let Some(value) = w32::listbox_selected(list) {
            self.store_list(path, without(self.string_list(path), &value));
        }
    }

    // ------------------------------------------------------------------------------------- history and rules

    fn refresh_history(&self) {
        let list = self.controls.borrow().history_list;
        if list.is_null() {
            return;
        }
        let rows: Vec<Vec<String>> = match self.services.history.read(None) {
            Ok(entries) => entries.iter().rev().map(|entry| history_row(entry).to_vec()).collect(),
            Err(_) => Vec::new(),
        };
        w32::list_view_set_rows(list, &rows);
    }

    fn refresh_learning(&self) {
        let rules = self.services.engine.learning.rules();
        let count = self.services.engine.learning.count();
        let (list, text) = {
            let controls = self.controls.borrow();
            (controls.learning_list, controls.learning_text)
        };
        if !text.is_null() {
            w32::set_text(text, &learning_count_text(count));
        }
        if !list.is_null() {
            let rows: Vec<Vec<String>> = rules.iter().map(|rule| rule_row(rule).to_vec()).collect();
            w32::list_view_set_rows(list, &rows);
        }
        self.state.borrow_mut().rules = rules;
    }

    fn selected_rules(&self) -> Vec<ks_config::learning::LearnedRule> {
        let list = self.controls.borrow().learning_list;
        let state = self.state.borrow();
        w32::list_view_selected(list).into_iter().filter_map(|index| state.rules.get(index).cloned()).collect()
    }

    fn edit_selected_rule(&self) {
        if let Some(rule) = self.selected_rules().first() {
            self.open_rule_editor(draft_from_rule(rule), None);
        }
    }

    fn open_rule_editor(&self, draft: RuleDraft, return_window: Option<i64>) {
        if crate::rule_editor_dialog::is_open() {
            crate::rule_editor_dialog::lift();
            return;
        }
        let engine = self.services.engine.clone();
        let backend = self.services.backend.clone();
        let accept_engine = engine.clone();
        let window = self.window;
        let accept = Box::new(move |draft: &RuleDraft| -> String {
            let rule = match build_rule(draft) {
                Ok(rule) => rule,
                Err(error) => return error.0,
            };
            match accept_engine.add_learning_rule(rule, draft.editing.as_ref(), &draft.application) {
                Ok(_) => {
                    // The rules list is refreshed once the dialog is done with its own state.
                    // SAFETY: posting to a window is allowed from any code of this thread; a stale handle fails the call.
                    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(window, WM_APP_REFRESH_RULES, 0, 0) };
                    String::new()
                }
                Err(error) => match error {
                    ks_config::learning::LearningError::Invalid(invalid) => invalid.0,
                    other @ (ks_config::learning::LearningError::Io(_) |
ks_config::learning::LearningError::Value(_)) => format!("Правило не сохранено: {other}"),
                },
            }
        });
        let focused_engine = engine;
        let closed_backend = backend.clone();
        let closed = Box::new(move || {
            if let Some(window) = return_window {
                // The window the user typed in gets the keyboard back.
                closed_backend.restore_window(Some(window));
            }
        });
        let activate_backend = backend;
        crate::rule_editor_dialog::open(crate::rule_editor_dialog::Arguments {
            draft,
            palette: self.palette(),
            dpi: self.dpi.get(),
            accept,
            closed,
            focused: Box::new(move |focused| focused_engine.set_rule_editor_open(focused)),
            activate: Box::new(move |handle| {
                activate_backend.restore_window(Some(handle));
            }),
        });
    }

    // -------------------------------------------------------------------------------------------- the report

    fn refresh_log_state(&self) {
        let control = self.controls.borrow().log_state;
        if control.is_null() {
            return;
        }
        let text = log_state_text(&log_status(), &log_path().display().to_string(), &rotation_summary(true));
        w32::set_text(control, &text);
        let page = self.pages.borrow()[PAGE_NAMES.iter().position(|(name, _)| *name == "maintenance").unwrap_or(0)].clone();
        if let Some(page) = page {
            self.relayout(&page);
        }
    }

    fn diagnostics(&self) -> String {
        let probe = self.services.backend.probe();
        let ready = self.services.engine.ready();
        let snapshot = self.services.engine.snapshot();
        let built = if ready { self.services.engine.engine().ok() } else { None };
        let guard = built.as_ref().map(|engine| engine.core());
        let (intent, context, prefix) = if let Some(core) = guard.as_ref().and_then(|guard| guard.as_ref()) { (
            core.intent_model_status.as_dict(),
            Map::from_iter([("available".to_owned(), json!(core.context_policy.model.is_some())), ("status".to_owned(), json!(core.context_policy.status))]),
            Map::from_iter([("available".to_owned(), json!(core.prefix_model.is_some())), ("status".to_owned(), json!(core.prefix_model.as_ref().map_or_else(String::new, |model| model.version().to_owned())))]),
        ) } else {
            let status = if snapshot.last_error.is_empty() { LOADING_ACTION.to_owned() } else { snapshot.last_error.clone() };
            (
                json!({"status": status}),
                Map::from_iter([("available".to_owned(), json!(false)), ("status".to_owned(), json!(status))]),
                Map::from_iter([("available".to_owned(), json!(false)), ("status".to_owned(), json!(status))]),
            )
        };
        drop(guard);
        let input = DiagnosticsInput {
            version: crate::windows_app::VERSION.to_owned(),
            available: probe.available,
            session_type: probe.session_type,
            display: probe.display,
            record_version: probe.record_version,
            xtest_version: probe.xtest_version,
            xkb_version: probe.xkb_version,
            current_group: i64::from(probe.current_group),
            current_layout: layout_label(i64::from(probe.current_group)).to_owned(),
            intent_model: intent,
            context_model: context,
            prefix_model: prefix,
            autostart: (self.services.autostart_report)(),
            technical_logging: self.setting_bool("diagnostics.technical_logging", false),
            technical_log: log_path().display().to_string(),
            log: log_status(),
            settings: self.settings().path.display().to_string(),
            data: data_dir().display().to_string(),
            error: probe.error,
        };
        diagnostics_text(
            &input,
            self.settings().get("detection.context_policy").unwrap_or_else(|| json!("assist")),
            self.settings().get("detection.context_read_field").unwrap_or_else(|| json!(false)),
            &snapshot.context_action,
            self.settings().get("detection.early_switch").unwrap_or_else(|| json!(true)),
        )
    }

    fn refresh_diagnostics(&self) {
        let control = self.controls.borrow().diagnostics;
        if control.is_null() {
            return;
        }
        w32::set_text(control, &self.diagnostics().replace('\n', "\r\n"));
    }
}

fn window_rect_in(parent: HWND, child: HWND) -> RECT {
    let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    // SAFETY: fills the rectangle; the mapping converts screen coordinates to the parent's client ones.
    unsafe {
        GetWindowRect(child, &raw mut rect);
        let mut points = [POINT { x: rect.left, y: rect.top }, POINT { x: rect.right, y: rect.bottom }];
        windows_sys::Win32::Graphics::Gdi::MapWindowPoints(null_mut(), parent, points.as_mut_ptr(), 2);
        RECT { left: points[0].x, top: points[0].y, right: points[1].x, bottom: points[1].y }
    }
}

// ------------------------------------------------------------------------------------------------ layout of a page

struct Layout<'a> {
    dpi: i32,
    fonts: &'a Fonts,
    moves: Vec<(HWND, i32, i32, i32, i32)>,
    viewport: i32,
    page: &'a Page,
}

fn apply_moves(moves: &[(HWND, i32, i32, i32, i32)], scroll: i32) {
    // SAFETY: the windows are ours; a window that failed to move leaves the batch intact.
    unsafe {
        let mut batch = BeginDeferWindowPos(moves.len().low::<i32>());
        for (hwnd, x, y, w, h) in moves {
            if batch.is_null() {
                break;
            }
            batch = DeferWindowPos(batch, *hwnd, null_mut(), *x, *y - scroll, *w, *h, SWP_NOZORDER | SWP_NOACTIVATE);
        }
        if !batch.is_null() {
            EndDeferWindowPos(batch);
        }
    }
}

fn label_font(fonts: &Fonts, style: LabelStyle) -> windows_sys::Win32::Graphics::Gdi::HFONT {
    match style {
        LabelStyle::PageTitle => fonts.title,
        LabelStyle::CardTitle | LabelStyle::CardTitleModified => fonts.semibold,
        LabelStyle::Normal | LabelStyle::Muted | LabelStyle::Error | LabelStyle::PageSubtitle => fonts.base,
    }
}

fn layout_nodes(nodes: &mut [Node], x: i32, width: i32, mut y: i32, context: &mut Layout<'_>) -> i32 {
    let dpi = context.dpi;
    for node in nodes {
        match node {
            Node::Label(label) => {
                y += label.top;
                let text = w32::text_of(label.hwnd);
                let style = context.page.styles.borrow().get(&label.hwnd.to::<isize>()).copied().unwrap_or(label.style);
                let height = w32::wrapped_height(label_font(context.fonts, style), &text, width - label.indent);
                context.moves.push((label.hwnd, x + label.indent, y, width - label.indent, height));
                y += height;
            }
            Node::Row { cells, top } => {
                y += *top;
                let fixed: i32 = cells.iter().filter(|cell| cell.width > 0).map(|cell| cell.width + cell.gap).sum::<i32>()
                    + cells.iter().filter(|cell| cell.width == 0).map(|cell| cell.gap).sum::<i32>();
                let fill = (width - fixed).max(scaled(40, dpi));
                let height = cells.iter().map(|cell| cell.height).max().unwrap_or(0);
                let mut cursor = x;
                for cell in cells.iter() {
                    cursor += cell.gap;
                    let cell_width = if cell.width == 0 { fill } else { cell.width };
                    if !cell.hwnd.is_null() {
                        context.moves.push((cell.hwnd, cursor, y + (height - cell.height) / 2, cell_width, cell.height + cell.extra));
                    }
                    cursor += cell_width;
                }
                y += height;
            }
            Node::Setting(setting) => y = layout_setting(setting, x, width, y, context),
            Node::Section { title, children, rect } => {
                let _ = title;
                let title_height = w32::line_height(context.fonts.frame);
                let inner_x = x + scaled(SECTION_PADDING_X_PIXELS, dpi);
                let inner_width = width - 2 * scaled(SECTION_PADDING_X_PIXELS, dpi);
                let start = y;
                let end = layout_nodes(children, inner_x, inner_width, y + title_height + scaled(SECTION_PADDING_Y_PIXELS, dpi) / 2, context);
                *rect = RECT { left: x, top: start + title_height / 2, right: x + width, bottom: end + scaled(SECTION_PADDING_Y_PIXELS, dpi) };
                y = rect.bottom + scaled(SECTION_BOTTOM_MARGIN_PIXELS, dpi);
            }
            Node::Columns { left, right } => {
                let gap = scaled(AUTOCORRECTION_COLUMN_GAP_PIXELS, dpi);
                let total = AUTOCORRECTION_PRIMARY_COLUMN_WEIGHT + AUTOCORRECTION_SECONDARY_COLUMN_WEIGHT;
                let left_width = (width - gap) * AUTOCORRECTION_PRIMARY_COLUMN_WEIGHT / total;
                let right_width = width - gap - left_width;
                let left_end = layout_nodes(left, x, left_width, y, context);
                let right_end = layout_nodes(right, x + left_width + gap, right_width, y, context);
                y = left_end.max(right_end);
            }
            Node::List { hwnd, height, top, fill } => {
                y += *top;
                let height = if *fill { (*height).max(context.viewport - y - scaled(PAGE_CONTENT_PADDING_Y_PIXELS, dpi)) } else { *height };
                context.moves.push((*hwnd, x, y, width, height));
                y += height;
            }
        }
    }
    y
}

fn layout_setting(setting: &mut SettingNode, x: i32, width: i32, mut y: i32, context: &mut Layout<'_>) -> i32 {
    let dpi = context.dpi;
    let fonts = context.fonts;
    let pad = scaled(SETTING_ROW_PADDING_PIXELS, dpi);
    y += pad;
    let row_top = y;
    let marker_column = scaled(MODIFIED_MARKER_COLUMN_PIXELS, dpi);
    let reset_column = scaled(RESET_BUTTON_COLUMN_PIXELS, dpi);
    let body_x = x + marker_column;
    let body_width = (width - marker_column - reset_column).max(scaled(120, dpi));
    
    
    let (after, indent) = if setting.kind == ControlKind::Bool {
        let box_width = scaled(16 + 8, dpi);
        let text = w32::text_of(setting.title);
        let height = w32::wrapped_height(fonts.base, &text, body_width - box_width).max(scaled(18, dpi));
        context.moves.push((setting.title, body_x, y, body_width, height));
        (y + height, scaled(DESCRIPTION_INDENT_PIXELS, dpi))
    } else {
        let control_width = scaled(
            match setting.kind {
                ControlKind::Choice => CHOICE_CONTROL_WIDTH_PIXELS,
                ControlKind::Int | ControlKind::Float => NUMBER_CONTROL_WIDTH_PIXELS,
                ControlKind::Bool | ControlKind::Text => TEXT_CONTROL_WIDTH_PIXELS,
            },
            dpi,
        );
        let gap = scaled(SETTING_CONTROL_PADDING_LEFT_PIXELS, dpi);
        let title_width = (body_width - control_width - gap).max(scaled(48, dpi));
        let title = w32::text_of(setting.title);
        let title_height = w32::wrapped_height(fonts.semibold, &title, title_width);
        let control_height = scaled(ROW_HEIGHT_PIXELS, dpi);
        let row_height = title_height.max(control_height);
        context.moves.push((setting.title, body_x, y + (row_height - title_height) / 2, title_width, title_height));
        let control_x = body_x + body_width - control_width;
        let control_y = y + (row_height - control_height) / 2;
        match setting.kind {
            ControlKind::Choice => context.moves.push((setting.control, control_x, control_y, control_width, control_height * 7)),
            ControlKind::Int | ControlKind::Float => {
                let arrows = scaled(UPDOWN_WIDTH_PIXELS, dpi);
                context.moves.push((setting.control, control_x, control_y, control_width - arrows, control_height));
                context.moves.push((setting.updown, control_x + control_width - arrows, control_y, arrows, control_height));
            }
            ControlKind::Bool | ControlKind::Text => context.moves.push((setting.control, control_x, control_y, control_width, control_height)),
        }
        (y + row_height, 0)
    };
    let description = w32::text_of(setting.description);
    let description_height = w32::wrapped_height(fonts.base, &description, body_width - indent);
    context.moves.push((setting.description, body_x + indent, after, body_width - indent, description_height));
    y = after + description_height;
    context.moves.push((setting.reset, x + width - reset_column + scaled(4, dpi), row_top, reset_column - scaled(4, dpi), scaled(22, dpi)));
    setting.marker = RECT { left: x, top: row_top, right: x + scaled(MODIFIED_MARKER_WIDTH_PIXELS, dpi), bottom: y };
    y + pad
}

struct PaintContext<'a> {
    scroll: i32,
    dpi: i32,
    palette: Palette,
    frame_font: windows_sys::Win32::Graphics::Gdi::HFONT,
    settings: &'a SettingsStore,
    controls: &'a [SettingControl],
}

fn paint_nodes(nodes: &[Node], hdc: HDC, context: &PaintContext<'_>) {
    for node in nodes {
        match node {
            Node::Section { title, children, rect } => {
                let shifted = RECT { left: rect.left, top: rect.top - context.scroll, right: rect.right, bottom: rect.bottom - context.scroll };
                w32::frame_rect(hdc, &shifted, context.palette.line.colorref());
                let title_text = format!(" {title} ");
                let mut text_rect = RECT {
                    left: shifted.left + scaled(SECTION_PADDING_X_PIXELS - 6, context.dpi),
                    top: shifted.top - w32::line_height(context.frame_font) / 2,
                    right: shifted.right,
                    bottom: shifted.top + w32::line_height(context.frame_font),
                };
                let width = w32::text_width(context.frame_font, &title_text);
                let brush = w32::solid_brush(context.palette.content);
                w32::fill_rect(hdc, &RECT { left: text_rect.left, top: text_rect.top, right: text_rect.left + width, bottom: text_rect.bottom }, brush);
                w32::delete_object(brush);
                w32::draw_label_text(hdc, &title_text, &mut text_rect, context.frame_font, context.palette.foreground.colorref());
                paint_nodes(children, hdc, context);
            }
            Node::Columns { left, right } => {
                paint_nodes(left, hdc, context);
                paint_nodes(right, hdc, context);
            }
            Node::Setting(setting) => {
                let modified = context.controls.get(setting.index).is_some_and(|control| !context.settings.is_default(&control.spec.path));
                if modified {
                    let brush = w32::solid_brush(context.palette.accent);
                    let marker = RECT { left: setting.marker.left, top: setting.marker.top - context.scroll, right: setting.marker.right, bottom: setting.marker.bottom - context.scroll };
                    w32::fill_rect(hdc, &marker, brush);
                    w32::delete_object(brush);
                }
            }
            Node::Label(_) | Node::Row { .. } | Node::List { .. } => {}
        }
    }
}

// ------------------------------------------------------------------------------------------------------- builders

struct Builder<'a> {
    ui: &'a Ui,
    page: &'a Page,
    panel: HWND,
}

fn flow_row(cells: Vec<RowCell>, top: i32) -> Node {
    Node::Row { cells, top }
}

impl Builder<'_> {
    fn dpi(&self) -> i32 {
        self.ui.dpi.get()
    }

    fn px(&self, pixels: i32) -> i32 {
        scaled(pixels, self.dpi())
    }

    fn label_window(&self, text: &str, style: LabelStyle) -> HWND {
        let hwnd = w32::create_window("STATIC", text, WS_CHILD | WS_VISIBLE | SS_LEFT | SS_NOPREFIX, 0, self.panel, 0, (0, 0, 10, 10));
        w32::set_font(hwnd, label_font(&self.ui.fonts(), style));
        self.page.styles.borrow_mut().insert(hwnd.to::<isize>(), style);
        hwnd
    }

    fn label(&mut self, text: &str, style: LabelStyle, indent: i32, top: i32) -> Node {
        Node::Label(Label { hwnd: self.label_window(text, style), style, indent, top })
    }

    /// A label whose text the window changes later; its handle comes back with it.
    fn dynamic_label(&mut self, text: &str, style: LabelStyle, top: i32) -> (Node, HWND) {
        let hwnd = self.label_window(text, style);
        (Node::Label(Label { hwnd, style, indent: 0, top: self.px(top) }), hwnd)
    }

    fn id(&self, action: Action) -> usize {
        self.ui.new_id(action)
    }

    fn button_cell(&self, text: &str, button: Button, gap: i32) -> (RowCell, HWND) {
        let fonts = self.ui.fonts();
        let id = self.id(Action::Button(button));
        let hwnd = w32::create_window("BUTTON", text, WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON.to::<u32>(), 0, self.panel, id, (0, 0, 10, 10));
        w32::set_font(hwnd, fonts.base);
        w32::apply_control_theme(hwnd, self.ui.dark.get().unwrap_or(false), Themed::Explorer);
        let width = w32::text_width(fonts.base, text) + 2 * self.px(BUTTON_PADDING_X_PIXELS);
        (RowCell { hwnd, width, height: self.px(BUTTON_HEIGHT_PIXELS), extra: 0, gap: self.px(gap) }, hwnd)
    }

    fn spacer(&self) -> RowCell {
        RowCell { hwnd: null_mut(), width: 0, height: 0, extra: 0, gap: 0 }
    }

    fn edit_window(&self, style: u32, text: &str) -> HWND {
        let hwnd = w32::create_window("EDIT", text, WS_CHILD | WS_VISIBLE | WS_TABSTOP | style, WS_EX_CLIENTEDGE, self.panel, 0, (0, 0, 10, 10));
        w32::set_font(hwnd, self.ui.fonts().base);
        w32::apply_control_theme(hwnd, self.ui.dark.get().unwrap_or(false), Themed::Field);
        hwnd
    }

    fn edit_cell(&self, gap: i32) -> (RowCell, HWND) {
        let hwnd = self.edit_window(ES_AUTOHSCROLL.to::<u32>(), "");
        (RowCell { hwnd, width: 0, height: self.px(ROW_HEIGHT_PIXELS), extra: 0, gap: self.px(gap) }, hwnd)
    }

    fn list_box(&self, rows: i32) -> (Node, HWND) {
        let hwnd = w32::create_window(
            "LISTBOX",
            "",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | WS_BORDER | LBS_NOTIFY.to::<u32>(),
            0,
            self.panel,
            0,
            (0, 0, 10, 10),
        );
        w32::set_font(hwnd, self.ui.fonts().base);
        w32::apply_control_theme(hwnd, self.ui.dark.get().unwrap_or(false), Themed::Explorer);
        let height = rows * w32::line_height(self.ui.fonts().base) + self.px(6);
        (Node::List { hwnd, height, top: 0, fill: false }, hwnd)
    }

    fn list_view(&self, columns: &[(&str, i32)], rows: i32, top: i32, fill: bool) -> (Node, HWND) {
        let hwnd = w32::create_window(w32::LIST_VIEW_CLASS, "", WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | w32::list_view_styles(), 0, self.panel, 0, (0, 0, 10, 10));
        w32::set_font(hwnd, self.ui.fonts().base);
        w32::apply_control_theme(hwnd, self.ui.dark.get().unwrap_or(false), Themed::Explorer);
        w32::list_view_setup(hwnd, columns, self.dpi());
        let palette = self.ui.palette();
        w32::list_view_colors(hwnd, palette.field.colorref(), palette.foreground.colorref());
        let height = rows * (w32::line_height(self.ui.fonts().base) + self.px(6)) + self.px(28);
        (Node::List { hwnd, height, top: self.px(top), fill }, hwnd)
    }

    fn section(&self, title: &str, children: Vec<Node>) -> Node {
        Node::Section { title: title.to_owned(), children, rect: RECT { left: 0, top: 0, right: 0, bottom: 0 } }
    }

    /// `_add_setting`: marker, label, control, description and the reset button of one setting.
    fn setting(&mut self, spec: &SettingSpec) -> Node {
        let ui = self.ui;
        let stored = ui.settings().get(&spec.path);
        let dark = ui.dark.get().unwrap_or(false);
        let index = ui.settings_controls.borrow().len();
        let id = self.id(Action::Setting(index));
        let (title, control, updown);
        let fonts = ui.fonts();
        if spec.kind == ControlKind::Bool {
            title = w32::create_window("BUTTON", &spec.title, WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW.to::<u32>(), 0, self.panel, id, (0, 0, 10, 10));
            w32::set_checked(title, stored.as_ref().is_some_and(truthy));
            control = null_mut();
            updown = null_mut();
        } else {
            title = self.label_window(&spec.title, LabelStyle::CardTitle);
            updown = null_mut();
            match spec.kind {
                ControlKind::Choice => {
                    control = w32::create_window("COMBOBOX", "", WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | CBS_DROPDOWNLIST.to::<u32>(), 0, self.panel, id, (0, 0, 10, 10));
                    w32::set_font(control, fonts.base);
                    w32::apply_control_theme(control, dark, Themed::Field);
                    w32::combo_reset(control, &spec.choices.iter().map(|(_, label)| label.clone()).collect::<Vec<_>>());
                    w32::combo_select_text(control, &control_text(spec, stored.as_ref()));
                    w32::combo_set_field_height(control, self.px(ROW_HEIGHT_PIXELS) - 6);
                    w32::guard_wheel(control);
                }
                ControlKind::Int | ControlKind::Float => {
                    control = w32::create_window("EDIT", &control_text(spec, stored.as_ref()), WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL.to::<u32>(), WS_EX_CLIENTEDGE, self.panel, id, (0, 0, 10, 10));
                    w32::set_font(control, fonts.base);
                    w32::apply_control_theme(control, dark, Themed::Field);
                    w32::guard_wheel(control);
                }
                ControlKind::Bool | ControlKind::Text => {
                    control = w32::create_window("EDIT", &control_text(spec, stored.as_ref()), WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL.to::<u32>(), WS_EX_CLIENTEDGE, self.panel, id, (0, 0, 10, 10));
                    w32::set_font(control, fonts.base);
                    w32::apply_control_theme(control, dark, Themed::Field);
                }
            }
        }
        let updown = if matches!(spec.kind, ControlKind::Int | ControlKind::Float) {
            let arrows = w32::create_window("msctls_updown32", "", WS_CHILD | WS_VISIBLE | UDS_ARROWKEYS | UDS_NOTHOUSANDS, 0, self.panel, self.id(Action::Other), (0, 0, 10, 10));
            w32::send(arrows, UDM_SETRANGE32, 0, 100);
            w32::send(arrows, UDM_SETPOS32, 0, 50);
            w32::apply_control_theme(arrows, dark, Themed::Explorer);
            arrows
        } else {
            updown
        };
        let description = self.label_window(&spec.description, LabelStyle::Muted);
        let reset_id = self.id(Action::Reset(index));
        let reset = w32::create_window("BUTTON", RESET_BUTTON_LABEL, WS_CHILD | WS_TABSTOP | BS_PUSHBUTTON.to::<u32>(), 0, self.panel, reset_id, (0, 0, 10, 10));
        w32::set_font(reset, fonts.small);
        w32::apply_control_theme(reset, dark, Themed::Explorer);
        self.add_tooltip(reset);
        drop(fonts);
        ui.settings_controls.borrow_mut().push(SettingControl { spec: spec.clone(), title, control, updown, reset });
        let modified = !ui.settings().is_default(&spec.path);
        if modified {
            // SAFETY: the reset button is ours.
            unsafe { ShowWindow(reset, SW_SHOW) };
            if spec.kind == ControlKind::Bool {
                w32::set_modified(title, true);
            } else {
                self.page.styles.borrow_mut().insert(title.to::<isize>(), LabelStyle::CardTitleModified);
            }
        }
        Node::Setting(SettingNode { index, kind: spec.kind, title, control, updown, reset, description, marker: RECT { left: 0, top: 0, right: 0, bottom: 0 } })
    }

    fn add_tooltip(&self, tool: HWND) {
        let tooltip = self.ui.controls.borrow().tooltip;
        if tooltip.is_null() {
            return;
        }
        let text = wide(RESET_BUTTON_HINT);
        // SAFETY: TTTOOLINFOW is plain data; the text lives through the call, which copies it.
        let mut info: TTTOOLINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = (size_of::<TTTOOLINFOW>()).low::<u32>();
        info.uFlags = TTF_IDISHWND | TTF_SUBCLASS;
        info.hwnd = self.panel;
        info.uId = tool.to::<usize>();
        info.lpszText = text.as_ptr().cast_mut();
        w32::send(tooltip, TTM_ADDTOOLW, 0, std::ptr::from_ref(&info).to::<LPARAM>());
    }

    fn settings_section(&mut self, title: &str, specs: &[SettingSpec]) -> Node {
        let children = specs.iter().map(|spec| self.setting(spec)).collect();
        self.section(title, children)
    }

    fn overview(&mut self) -> Vec<Node> {
        let state_texts = {
            let state = self.ui.state.borrow();
            (state.status.clone(), state.layout.clone(), state.count.clone(), state.last_action.clone(), state.error.clone())
        };
        let (status_node, status) = self.dynamic_label(&state_texts.0, LabelStyle::CardTitle, 0);
        let (layout_node, layout) = self.dynamic_label(&state_texts.1, LabelStyle::Normal, CARD_TITLE_GAP_PIXELS);
        let (count_node, count) = self.dynamic_label(&state_texts.2, LabelStyle::Normal, STATUS_LINE_GAP_PIXELS);
        let (last_node, last) = self.dynamic_label(&state_texts.3, LabelStyle::Normal, STATUS_LINE_GAP_PIXELS);
        let (error_node, error) = self.dynamic_label(&state_texts.4, LabelStyle::Error, STATUS_LINE_GAP_PIXELS);
        {
            let mut controls = self.ui.controls.borrow_mut();
            controls.overview_status = status;
            controls.overview_layout = layout;
            controls.overview_count = count;
            controls.overview_last = last;
            controls.overview_error = error;
        }
        let enabled = self.setting(&autocorrection_settings()[0]);
        let state = self.section("Состояние", vec![status_node, layout_node, count_node, last_node, error_node, enabled]);
        let hint = self.label(
            "Выберите EN и напечатайте ghbdtn: после паузы появится «привет». Можно также нажать пробел. Потом проверьте обратное направление.",
            LabelStyle::Normal,
            0,
            0,
        );
        // The field is typed in as any other program is, and the engine has to be told when it has the keyboard.
        let id = self.id(Action::TestField);
        let entry = w32::create_window("EDIT", "", WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL.to::<u32>(), WS_EX_CLIENTEDGE, self.panel, id, (0, 0, 10, 10));
        w32::set_font(entry, self.ui.fonts().test);
        w32::apply_control_theme(entry, self.ui.dark.get().unwrap_or(false), Themed::Field);
        let height = w32::line_height(self.ui.fonts().test) + self.px(8);
        let row = flow_row(vec![RowCell { hwnd: entry, width: 0, height, extra: 0, gap: 0 }], self.px(TEST_ENTRY_PADDING_TOP_PIXELS));
        let note = self.label("Поле обрабатывается тем же глобальным Win32 backend, что и другие приложения.", LabelStyle::Muted, 0, self.px(TEST_ENTRY_PADDING_BOTTOM_PIXELS));
        vec![state, self.section("Проверка в реальном поле", vec![hint, row, note])]
    }

    fn autocorrection(&mut self) -> Vec<Node> {
        let behavior = self.settings_section("Распознавание", &autocorrection_settings());
        let triggers = self.settings_section("Когда проверять слово", &trigger_settings());
        let quirks = self.settings_section("Особенности программ", &application_settings());
        vec![Node::Columns { left: vec![behavior], right: vec![triggers] }, quirks]
    }

    fn layouts(&mut self) -> Vec<Node> {
        let top = self.px(LAYOUT_LANGUAGE_LABEL_PADDING_TOP_PIXELS);
        let bottom = self.px(LAYOUT_LANGUAGE_LABEL_PADDING_BOTTOM_PIXELS);
        let children = vec![
            self.label("Английская раскладка", LabelStyle::CardTitle, 0, 0),
            self.label("EN — primary language 0x09", LabelStyle::Normal, 0, top),
            self.label("Русская раскладка", LabelStyle::CardTitle, 0, bottom),
            self.label("RU — primary language 0x19", LabelStyle::Normal, 0, top),
            {
                let (cell, _) = self.button_cell("Обновить диагностику", Button::RefreshDiagnostics, 0);
                flow_row(vec![cell, self.spacer()], bottom)
            },
        ];
        let system = self.section("Системная пара", children);
        let mut manual: Vec<Node> = manual_switch_settings().iter().map(|spec| self.setting(spec)).collect();
        manual.push(self.label(
            "Когда вы сами меняете язык перед вводом, первое завершённое слово остаётся в выбранной раскладке. Следующие слова снова анализируются автоматически.",
            LabelStyle::Normal,
            0,
            self.px(MANUAL_SWITCH_NOTE_PADDING_TOP_PIXELS),
        ));
        vec![system, self.section("Ручное переключение", manual)]
    }

    fn hotkeys(&mut self) -> Vec<Node> {
        vec![self.settings_section("Глобальные команды", &hotkey_settings())]
    }

    fn appearance(&mut self) -> Vec<Node> {
        vec![self.settings_section("Интеграция с Windows", &system_settings())]
    }

    fn exclusions(&mut self) -> Vec<Node> {
        let gap = STANDARD_GAP_PIXELS;
        let (applications, application_list) = self.list_box(APPLICATION_LIST_HEIGHT_ROWS);
        let (active, _) = self.button_cell("Активное окно", Button::AddActive, 0);
        let (pick, _) = self.button_cell("Выбрать .exe…", Button::PickExecutable, gap);
        let (remove, _) = self.button_cell("Удалить", Button::RemoveApplication, 0);
        let buttons = flow_row(vec![active, pick, self.spacer(), remove], self.px(GROUP_GAP_PIXELS));
        let combo_id = self.id(Action::Other);
        let combo = w32::create_window("COMBOBOX", "", WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | CBS_DROPDOWNLIST.to::<u32>(), 0, self.panel, combo_id, (0, 0, 10, 10));
        w32::set_font(combo, self.ui.fonts().base);
        w32::apply_control_theme(combo, self.ui.dark.get().unwrap_or(false), Themed::Field);
        w32::combo_set_field_height(combo, self.px(ROW_HEIGHT_PIXELS) - 6);
        w32::guard_wheel(combo);
        let (refresh, _) = self.button_cell("Обновить список", Button::LoadCatalog, gap);
        let (from_list, _) = self.button_cell("Добавить из списка", Button::AddCatalog, gap);
        let catalog = flow_row(
            vec![RowCell { hwnd: combo, width: 0, height: self.px(ROW_HEIGHT_PIXELS), extra: self.px(ROW_HEIGHT_PIXELS) * 8, gap: 0 }, refresh, from_list],
            self.px(gap),
        );
        let (entry, manual_application) = self.edit_cell(0);
        let (add_manual, _) = self.button_cell("Добавить вручную", Button::AddManual, gap);
        let manual = flow_row(vec![entry, add_manual], self.px(gap));
        let programs = self.section("Программы", vec![applications, buttons, catalog, manual]);
        let (words, word_list) = self.list_box(WORD_LIST_HEIGHT_ROWS);
        let (word_entry, manual_word) = self.edit_cell(0);
        let (add, _) = self.button_cell("Добавить", Button::AddWord, gap);
        let (delete, _) = self.button_cell("Удалить", Button::RemoveWord, gap);
        let word_row = flow_row(vec![word_entry, add, delete], self.px(gap));
        let words = self.section("Слова", vec![words, word_row]);
        {
            let mut controls = self.ui.controls.borrow_mut();
            controls.application_list = application_list;
            controls.word_list = word_list;
            controls.catalog_combo = combo;
            controls.manual_application = manual_application;
            controls.manual_word = manual_word;
        }
        // The catalog combo's entries come from the last listing.
        let labels: Vec<String> = self.ui.state.borrow().catalog.iter().map(|(label, _)| label.clone()).collect();
        w32::combo_reset(combo, &labels);
        if let Some(first) = labels.first() {
            w32::combo_select_text(combo, first);
        }
        vec![programs, words]
    }

    fn history(&mut self) -> Vec<Node> {
        let (refresh, _) = self.button_cell("Обновить", Button::RefreshHistory, 0);
        let (clear, _) = self.button_cell("Очистить историю", Button::ClearHistory, 0);
        let toolbar = flow_row(vec![refresh, self.spacer(), clear], 0);
        let columns = [
            ("Время UTC", HISTORY_TIME_COLUMN_WIDTH_PIXELS),
            ("Было", HISTORY_WORD_COLUMN_WIDTH_PIXELS),
            ("Стало", HISTORY_WORD_COLUMN_WIDTH_PIXELS),
            ("Приложение", HISTORY_APPLICATION_COLUMN_WIDTH_PIXELS),
            ("Уверенность", HISTORY_CONFIDENCE_COLUMN_WIDTH_PIXELS),
        ];
        let (list, hwnd) = self.list_view(&columns, HISTORY_TREE_MIN_ROWS, GROUP_GAP_PIXELS, true);
        self.ui.controls.borrow_mut().history_list = hwnd;
        vec![toolbar, list]
    }

    fn updates(&mut self) -> Vec<Node> {
        let view = self.ui.state.borrow().updates.clone();
        let (version_node, version) = self.dynamic_label(&view.version, LabelStyle::CardTitle, 0);
        let (status_node, status) = self.dynamic_label(&view.status, LabelStyle::Normal, CARD_TITLE_GAP_PIXELS);
        let (progress_node, progress) = self.dynamic_label(&view.progress, LabelStyle::Muted, STATUS_LINE_GAP_PIXELS);
        let state = self.section("Состояние", vec![version_node, status_node, progress_node]);
        let policy = self.settings_section("Политика", &update_settings());
        let gap = STANDARD_GAP_PIXELS;
        let (check, check_button) = self.button_cell("Проверить сейчас", Button::CheckUpdates, 0);
        let (install, install_button) = self.button_cell("Установить сейчас", Button::InstallUpdate, gap);
        let (release, release_button) = self.button_cell("Открыть выпуск", Button::OpenRelease, gap);
        let row = flow_row(vec![check, install, release, self.spacer()], 0);
        let note = self.label(
            "Автоустановка работает для Setup EXE текущего пользователя и не требует прав администратора. При обновлении окно закроется и KeySwitch запустится снова.",
            LabelStyle::Muted,
            0,
            self.px(GROUP_GAP_PIXELS),
        );
        {
            let mut controls = self.ui.controls.borrow_mut();
            controls.update_version = version;
            controls.update_status = status;
            controls.update_progress = progress;
            controls.update_check = check_button;
            controls.update_install = install_button;
            controls.update_release = release_button;
        }
        let actions = self.section("Действия", vec![row, note]);
        vec![state, policy, actions]
    }

    fn maintenance(&mut self) -> Vec<Node> {
        let (count_node, count) = self.dynamic_label(&learning_count_text(self.ui.services.engine.learning.count()), LabelStyle::Normal, 0);
        let columns = [
            ("Сочетание", LEARNING_WORD_COLUMN_WIDTH_PIXELS),
            ("В другой раскладке", LEARNING_WORD_COLUMN_WIDTH_PIXELS),
            ("Условие", LEARNING_CONDITION_COLUMN_WIDTH_PIXELS),
            ("Действие", LEARNING_ACTION_COLUMN_WIDTH_PIXELS),
        ];
        let (list, rules) = self.list_view(&columns, LEARNING_TREE_HEIGHT_ROWS, GROUP_GAP_PIXELS, false);
        let note = self.label(
            "Правило говорит, что делать со словом, которое совпадает с сочетанием букв, начинается с него или содержит его: переводить в другую раскладку или нет. Правило важнее решения модели. Чтобы добавить правило для последнего слова, дважды нажмите клавишу ручного преобразования (Pause), затем Enter: окно правила откроется уже заполненным. Правило появляется только по кнопке OK.",
            LabelStyle::Muted,
            0,
            self.px(STANDARD_GAP_PIXELS),
        );
        let gap = STANDARD_GAP_PIXELS;
        let mut cells = Vec::new();
        for (index, (text, button)) in
            [("Добавить…", Button::AddRule), ("Изменить…", Button::EditRule), ("Удалить выбранное", Button::DeleteRules), ("Удалить все", Button::ClearRules)].into_iter().enumerate()
        {
            cells.push(self.button_cell(text, button, if index == 0 { 0 } else { gap }).0);
        }
        cells.push(self.spacer());
        let actions = flow_row(cells, self.px(GROUP_GAP_PIXELS));
        let learning = self.section("Правила переключения", vec![count_node, list, note, actions]);
        let reset_note = self.label("Вернуть все параметры по умолчанию. История и правила переключения сохранятся.", LabelStyle::Normal, 0, 0);
        let (reset, _) = self.button_cell("Сбросить настройки", Button::ResetSettings, 0);
        let settings = self.section("Настройки", vec![reset_note, flow_row(vec![reset, self.spacer()], self.px(GROUP_GAP_PIXELS))]);
        let mut diagnostics: Vec<Node> = diagnostic_settings().iter().map(|spec| self.setting(spec)).collect();
        let (log_node, log_state) = self.dynamic_label("", LabelStyle::Normal, STANDARD_GAP_PIXELS);
        diagnostics.push(log_node);
        let (open, _) = self.button_cell("Открыть папку журнала", Button::OpenLogDirectory, 0);
        diagnostics.push(flow_row(vec![open, self.spacer()], self.px(STANDARD_GAP_PIXELS)));
        let diagnostics = self.section("Технический журнал", diagnostics);
        let locations = locations_text(
            &self.ui.settings().path.display().to_string(),
            &self.ui.services.history.path.display().to_string(),
            &self.ui.services.engine.learning.path.display().to_string(),
            &log_path().display().to_string(),
        );
        let locations_label = self.label(&locations, LabelStyle::Normal, 0, 0);
        let locations = self.section("Локальные файлы", vec![locations_label]);
        {
            let mut controls = self.ui.controls.borrow_mut();
            controls.learning_list = rules;
            controls.learning_text = count;
            controls.log_state = log_state;
        }
        vec![learning, settings, diagnostics, locations]
    }

    fn about(&mut self) -> Vec<Node> {
        let text = w32::create_window(
            "EDIT",
            "",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | WS_BORDER | (ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL).to::<u32>(),
            0,
            self.panel,
            0,
            (0, 0, 10, 10),
        );
        w32::set_font(text, self.ui.fonts().base);
        w32::apply_control_theme(text, self.ui.dark.get().unwrap_or(false), Themed::Field);
        let height = DIAGNOSTICS_TEXT_HEIGHT_ROWS * w32::line_height(self.ui.fonts().base) + self.px(6);
        let (refresh, _) = self.button_cell("Обновить", Button::RefreshDiagnostics, 0);
        let (copy, _) = self.button_cell("Копировать", Button::CopyDiagnostics, STANDARD_GAP_PIXELS);
        let buttons = flow_row(vec![refresh, copy, self.spacer()], self.px(GROUP_GAP_PIXELS));
        self.ui.controls.borrow_mut().diagnostics = text;
        let details = self.section("Диагностика Win32 backend", vec![Node::List { hwnd: text, height, top: 0, fill: false }, buttons]);
        let license = self.label("Лицензия: GNU GPL-3.0-or-later · https://github.com/olegius88/keyswitch", LabelStyle::Muted, 0, 0);
        vec![details, license]
    }
}
