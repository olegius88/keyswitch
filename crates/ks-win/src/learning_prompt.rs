//! The learning prompt (the Python's `WindowsLearningPrompt`): a small card above the caret that offers a switching rule for
//! the last word. It never takes the foreground, because every Windows window has its own keyboard layout, so an activated
//! prompt would both read as a manual layout switch and swallow the next typed key; Enter and Esc reach the engine through
//! the global keyboard hook, and the window's own key handling below is only a second way to the same calls.

#![cfg_attr(windows, expect(unsafe_code, reason = "calls into Win32"))]
use ks_cast::To as _;
use std::cell::RefCell;
use std::ptr::{null, null_mut};
use std::rc::Rc;

use ks_backend::backend::ScreenAnchor;
use ks_engine::LearningPrompt;
use ks_engine::rule_editor::{PROMPT_HINT, PROMPT_TITLE, prompt_word_line};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, HDC, MONITOR_DEFAULTTONEAREST, MonitorFromPoint, PAINTSTRUCT,
};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, DestroyWindow, GetCursorPos, GetSystemMetrics, HWND_TOPMOST, IDC_ARROW, LoadCursorW, MA_NOACTIVATE, RegisterClassW,
    SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetWindowPos, WM_ERASEBKGND,
    WM_KEYDOWN, WM_MOUSEACTIVATE, WM_PAINT, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::constants::{LEARNING_PROMPT_CLASS, BASE_DPI, LEARNING_PROMPT_CARD_PADDING_X_PIXELS, LEARNING_PROMPT_CARD_PADDING_Y_PIXELS, LEARNING_PROMPT_MINIMUM_WIDTH_PIXELS, LEARNING_PROMPT_WORD_PADDING_TOP_PIXELS, LEARNING_PROMPT_WORD_PADDING_BOTTOM_PIXELS, LEARNING_PROMPT_ANCHOR_GAP_PIXELS, LEARNING_PROMPT_SCREEN_MARGIN_PIXELS};
use crate::win32_controls::{self as w32, Fonts, scaled};
use crate::windows_native::wide;
use crate::windows_ui_model::{PROMPT_BORDER, PROMPT_CARD, PROMPT_HINT_COLOR, PROMPT_TITLE_COLOR, PROMPT_WORD_COLOR, prompt_position};

const VK_RETURN: usize = 0x0D;
const VK_ESCAPE: usize = 0x1B;

/// What the prompt asks of the program around it.
pub struct Callbacks {
    pub confirm: Box<dyn Fn(&LearningPrompt) -> bool>,
    pub dismiss: Box<dyn Fn(&LearningPrompt) -> bool>,
    /// `keep_window_inactive`.
    pub keep_inactive: Box<dyn Fn(i64) -> bool>,
    /// `restore_window`.
    pub restore: Box<dyn Fn(Option<i64>) -> bool>,
}

struct Shown {
    window: HWND,
    fonts: Fonts,
    prompt: LearningPrompt,
    anchor: Option<ScreenAnchor>,
    word_line: String,
    dpi: i32,
    callbacks: Rc<Callbacks>,
}

thread_local! {
    static SHOWN: RefCell<Option<Shown>> = const { RefCell::new(None) };
    static REGISTERED: RefCell<bool> = const { RefCell::new(false) };
}

fn register() -> bool {
    if REGISTERED.with(|flag| *flag.borrow()) {
        return true;
    }
    let class = wide(LEARNING_PROMPT_CLASS);
    // SAFETY: the class description points at strings that live through RegisterClassW, which copies them.
    let registered = unsafe {
        let description = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(prompt_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: w32::hinstance(),
            hIcon: null_mut(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: class.as_ptr(),
        };
        RegisterClassW(&raw const description) != 0
    };
    REGISTERED.with(|flag| *flag.borrow_mut() = registered);
    registered
}

fn monitor_dpi(point: POINT) -> i32 {
    let (mut x, mut y) = (0u32, 0u32);
    // SAFETY: the monitor is looked up by value; the call fills two numbers.
    let result = unsafe {
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &raw mut x, &raw mut y)
    };
    if result == 0 && x > 0 { x.bits::<i32>() } else { BASE_DPI }
}

fn virtual_screen() -> (i32, i32, i32, i32) {
    // SAFETY: plain metrics queries.
    unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}

/// `show_prompt`: the card above the anchor, or above the pointer when there is no caret to be found.
pub fn show(prompt: &LearningPrompt, anchor: Option<ScreenAnchor>, callbacks: Rc<Callbacks>) {
    if !register() {
        return;
    }
    destroy_window();
    let anchor = anchor.or_else(|| {
        let mut point = POINT { x: 0, y: 0 };
        // SAFETY: fills the point it is given.
        unsafe { GetCursorPos(&raw mut point) };
        Some(ScreenAnchor { x: point.x, y: point.y, window: None })
    });
    let Some(place) = anchor else { return };
    let dpi = monitor_dpi(POINT { x: place.x, y: place.y });
    let fonts = Fonts::new(dpi);
    let word_line = prompt_word_line(prompt);
    let widest = [(fonts.prompt_title, PROMPT_TITLE), (fonts.prompt_word, word_line.as_str()), (fonts.prompt_hint, PROMPT_HINT)]
        .iter()
        .map(|(font, text)| w32::text_width(*font, text))
        .max()
        .unwrap_or(0);
    let pad_x = scaled(LEARNING_PROMPT_CARD_PADDING_X_PIXELS, dpi);
    let pad_y = scaled(LEARNING_PROMPT_CARD_PADDING_Y_PIXELS, dpi);
    let width = scaled(LEARNING_PROMPT_MINIMUM_WIDTH_PIXELS, dpi).max(widest + 2 * pad_x + 2);
    let height = 2
        + 2 * pad_y
        + w32::line_height(fonts.prompt_title)
        + scaled(LEARNING_PROMPT_WORD_PADDING_TOP_PIXELS, dpi)
        + w32::line_height(fonts.prompt_word)
        + scaled(LEARNING_PROMPT_WORD_PADDING_BOTTOM_PIXELS, dpi)
        + w32::line_height(fonts.prompt_hint);
    let (x, y) = prompt_position(
        (place.x, place.y),
        (width, height),
        virtual_screen(),
        scaled(LEARNING_PROMPT_ANCHOR_GAP_PIXELS, dpi),
        scaled(LEARNING_PROMPT_SCREEN_MARGIN_PIXELS, dpi),
    );
    let window = w32::create_window(
        LEARNING_PROMPT_CLASS,
        "KeySwitch",
        WS_POPUP,
        WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
        null_mut(),
        0,
        (x, y, width, height),
    );
    if window.is_null() {
        fonts.destroy();
        return;
    }
    let restore = anchor.and_then(|place| place.window);
    // The prompt must never take the foreground.
    (callbacks.keep_inactive)((window.to::<isize>()).to::<i64>());
    SHOWN.with(|slot| {
        *slot.borrow_mut() = Some(Shown { window, fonts, prompt: prompt.clone(), anchor, word_line, dpi, callbacks: callbacks.clone() });
    });
    // SAFETY: the window is ours; NOACTIVATE keeps the foreground where it is.
    unsafe { SetWindowPos(window, HWND_TOPMOST, x, y, width, height, SWP_NOACTIVATE | SWP_SHOWWINDOW) };
    (callbacks.keep_inactive)((window.to::<isize>()).to::<i64>());
    if restore.is_some() {
        (callbacks.restore)(restore);
    }
}

fn destroy_window() -> Option<Shown> {
    let shown = SHOWN.with(|slot| slot.borrow_mut().take())?;
    // SAFETY: the window belongs to this thread.
    unsafe { DestroyWindow(shown.window) };
    shown.fonts.destroy();
    Some(shown)
}

/// `hide_prompt`: the card goes away, and the window it was shown over gets the keyboard back.
#[must_use]
pub fn hide() -> Option<(Option<i64>, Rc<Callbacks>)> {
    destroy_window().map(|shown| (shown.anchor.and_then(|anchor| anchor.window), shown.callbacks))
}

#[must_use]
pub fn is_shown() -> bool {
    SHOWN.with(|slot| slot.borrow().is_some())
}

fn key_down(key: usize) {
    let target = SHOWN.with(|slot| slot.borrow().as_ref().map(|shown| (shown.prompt.clone(), shown.callbacks.clone())));
    if let Some((prompt, callbacks)) = target {
        match key {
            VK_RETURN => {
                (callbacks.confirm)(&prompt);
            }
            VK_ESCAPE => {
                (callbacks.dismiss)(&prompt);
            }
            _ => {}
        }
    }
}

fn paint(window: HWND) {
    // SAFETY: PAINTSTRUCT is plain data filled by BeginPaint.
    let mut paint: PAINTSTRUCT = unsafe { std::mem::zeroed() };
    // SAFETY: balanced with EndPaint below on the same window.
    let hdc: HDC = unsafe { BeginPaint(window, &raw mut paint) };
    SHOWN.with(|slot| {
        if let Some(shown) = slot.borrow().as_ref() {
            let client = w32::client_rect(window);
            let dpi = shown.dpi;
            let border = w32::solid_brush(PROMPT_BORDER);
            let card = w32::solid_brush(PROMPT_CARD);
            w32::fill_rect(hdc, &client, border);
            w32::fill_rect(hdc, &RECT { left: 1, top: 1, right: client.right - 1, bottom: client.bottom - 1 }, card);
            w32::delete_object(border);
            w32::delete_object(card);
            let left = 1 + scaled(LEARNING_PROMPT_CARD_PADDING_X_PIXELS, dpi);
            let mut top = 1 + scaled(LEARNING_PROMPT_CARD_PADDING_Y_PIXELS, dpi);
            let mut line = |text: &str, font, color: u32, height: i32, gap: i32| {
                let mut rect = RECT { left, top, right: client.right - left, bottom: top + height };
                w32::draw_label_text(hdc, text, &mut rect, font, color);
                top += height + gap;
            };
            line(PROMPT_TITLE, shown.fonts.prompt_title, PROMPT_TITLE_COLOR.colorref(), w32::line_height(shown.fonts.prompt_title), scaled(LEARNING_PROMPT_WORD_PADDING_TOP_PIXELS, dpi));
            line(&shown.word_line, shown.fonts.prompt_word, PROMPT_WORD_COLOR.colorref(), w32::line_height(shown.fonts.prompt_word), scaled(LEARNING_PROMPT_WORD_PADDING_BOTTOM_PIXELS, dpi));
            line(PROMPT_HINT, shown.fonts.prompt_hint, PROMPT_HINT_COLOR.colorref(), w32::line_height(shown.fonts.prompt_hint), 0);
        }
    });
    // SAFETY: ends the paint begun above.
    unsafe { EndPaint(window, &raw const paint) };
}

unsafe extern "system" fn prompt_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint(window);
            0
        }
        WM_MOUSEACTIVATE => MA_NOACTIVATE.to::<LRESULT>(),
        WM_KEYDOWN => {
            key_down(wparam);
            0
        }
        // SAFETY: the default procedure takes the arguments the system gave this one.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}
