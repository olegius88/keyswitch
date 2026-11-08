//! The one-time hint (`ks_ui::hints`): a small card above the caret or the tray icon. Like the rule prompt
//! (`learning_prompt`) it never takes the foreground, and unlike the prompt it has no keys at all: Enter, Esc and every
//! other key go where they would have gone without it. Its two links are painted and hit-tested by the window itself: a
//! child button takes the keyboard focus when it is pressed, and with it the activation of its parent. The arithmetic is
//! in `hint_layout`.

#![cfg_attr(windows, expect(unsafe_code, reason = "calls into Win32"))]
use ks_cast::To as _;
use std::cell::RefCell;
use std::ptr::{null, null_mut};
use std::rc::Rc;

use ks_ui::constants::HINT_CARD_WIDTH_PIXELS;
use ks_ui::hints::{Click, HintView};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{BeginPaint, EndPaint, HDC, PAINTSTRUCT, ScreenToClient};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, DestroyWindow, GetCursorPos, HWND_TOPMOST, IDC_ARROW, IDC_HAND, LoadCursorW, MA_NOACTIVATE, RegisterClassW, SWP_NOACTIVATE,
    SWP_SHOWWINDOW, SetCursor, SetWindowPos, WM_ERASEBKGND, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_PAINT, WM_SETCURSOR, WNDCLASSW, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::constants::{
    HINT_CARD_BORDER_PIXELS, HINT_CARD_CARET_GAP_PIXELS, HINT_CARD_CLASS, HINT_CARD_LINE_GAP_PIXELS, HINT_CARD_LINK_GAP_PIXELS,
    LEARNING_PROMPT_ANCHOR_GAP_PIXELS, LEARNING_PROMPT_CARD_PADDING_X_PIXELS, LEARNING_PROMPT_CARD_PADDING_Y_PIXELS,
    LEARNING_PROMPT_SCREEN_MARGIN_PIXELS,
};
use crate::hint_layout::{Area, CardLayout, CardMetrics, card_hit, card_layout, card_position, card_text_width, link_row};
use crate::learning_prompt::{monitor_dpi, virtual_screen};
use crate::win32_controls::{self as w32, Fonts, scaled};
use crate::windows_native::wide;
use crate::windows_ui_model::{PROMPT_BORDER, PROMPT_CARD, PROMPT_HINT_COLOR, PROMPT_TITLE_COLOR, PROMPT_WORD_COLOR};

/// What the card asks of the program around it.
pub struct Callbacks {
    /// What the user did: said from the card's own message, so it only posts what it does.
    pub clicked: Box<dyn Fn(&'static str, Click)>,
    /// `keep_window_inactive`.
    pub keep_inactive: Box<dyn Fn(i64) -> bool>,
}

struct Shown {
    window: HWND,
    fonts: Fonts,
    id: &'static str,
    title: &'static str,
    text: String,
    ok: &'static str,
    never: &'static str,
    /// Whether a click on the text leads somewhere.
    text_opens: bool,
    layout: CardLayout,
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
    let class = wide(HINT_CARD_CLASS);
    // SAFETY: the class description points at strings that live through RegisterClassW, which copies them.
    let registered = unsafe {
        let description = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(card_proc),
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

fn metrics(fonts: &Fonts, dpi: i32, text: &str, ok: &str, never: &str) -> CardMetrics {
    let width = scaled(HINT_CARD_WIDTH_PIXELS, dpi);
    let pad_x = scaled(LEARNING_PROMPT_CARD_PADDING_X_PIXELS, dpi);
    CardMetrics {
        width,
        pad_x,
        pad_y: scaled(LEARNING_PROMPT_CARD_PADDING_Y_PIXELS, dpi),
        line_gap: scaled(HINT_CARD_LINE_GAP_PIXELS, dpi),
        link_gap: scaled(HINT_CARD_LINK_GAP_PIXELS, dpi),
        title_height: w32::line_height(fonts.prompt_title),
        text_height: w32::wrapped_height(fonts.prompt_hint, text, card_text_width(width, pad_x)),
        link_height: w32::line_height(fonts.prompt_hint),
        ok_width: w32::text_width(fonts.prompt_hint, ok),
        never_width: w32::text_width(fonts.prompt_hint, never),
    }
}

/// Where the two links of a card of `size` are, as `show` lays them out at `dpi`, in its client area: for the live test,
/// which clicks them as a person does.
#[must_use]
pub fn link_areas(dpi: i32, size: (i32, i32), ok: &str, never: &str) -> (Area, Area) {
    let fonts = Fonts::new(dpi);
    let row = link_row(&metrics(&fonts, dpi, "", ok, never), size.1);
    fonts.destroy();
    row
}

/// The card above the caret, whose anchor is `(x, y)`: the bottom of the caret or the mouse pointer.
#[must_use]
pub fn show_at_caret(view: &HintView, x: i32, y: i32, callbacks: &Rc<Callbacks>) -> bool {
    show(view, Area { left: x, top: y, right: x, bottom: y }, HINT_CARD_CARET_GAP_PIXELS, callbacks)
}

/// The card above the tray icon.
#[must_use]
pub fn show_at_icon(view: &HintView, icon: Area, callbacks: &Rc<Callbacks>) -> bool {
    show(view, icon, LEARNING_PROMPT_ANCHOR_GAP_PIXELS, callbacks)
}

fn show(view: &HintView, target: Area, gap: i32, callbacks: &Rc<Callbacks>) -> bool {
    if !register() {
        return false;
    }
    destroy_window();
    let dpi = monitor_dpi(POINT { x: i32::midpoint(target.left, target.right), y: target.top });
    let fonts = Fonts::new(dpi);
    let layout = card_layout(&metrics(&fonts, dpi, &view.text, view.ok, view.never));
    let (x, y) = card_position(target, layout.size, virtual_screen(), scaled(gap, dpi), scaled(LEARNING_PROMPT_SCREEN_MARGIN_PIXELS, dpi));
    let (width, height) = layout.size;
    let window = w32::create_window(
        HINT_CARD_CLASS,
        view.title,
        WS_POPUP,
        WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
        null_mut(),
        0,
        (x, y, width, height),
    );
    if window.is_null() {
        fonts.destroy();
        return false;
    }
    // The card must never take the foreground, and nothing here gives it back: what is in front stays in front.
    (callbacks.keep_inactive)(window.to::<isize>().to::<i64>());
    SHOWN.with(|slot| {
        *slot.borrow_mut() = Some(Shown {
            window,
            fonts,
            id: view.id,
            title: view.title,
            text: view.text.clone(),
            ok: view.ok,
            never: view.never,
            text_opens: view.opens.is_some(),
            layout,
            callbacks: callbacks.clone(),
        });
    });
    // SAFETY: the window is ours; NOACTIVATE keeps the foreground where it is.
    unsafe { SetWindowPos(window, HWND_TOPMOST, x, y, width, height, SWP_NOACTIVATE | SWP_SHOWWINDOW) };
    (callbacks.keep_inactive)(window.to::<isize>().to::<i64>());
    true
}

fn destroy_window() {
    if let Some(shown) = SHOWN.with(|slot| slot.borrow_mut().take()) {
        // SAFETY: the window belongs to this thread.
        unsafe { DestroyWindow(shown.window) };
        shown.fonts.destroy();
    }
}

/// The card goes away if it is the one that shows the hint `id`.
pub fn hide(id: &str) {
    if SHOWN.with(|slot| slot.borrow().as_ref().is_some_and(|shown| shown.id == id)) {
        destroy_window();
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
            let border = w32::solid_brush(PROMPT_BORDER);
            let card = w32::solid_brush(PROMPT_CARD);
            w32::fill_rect(hdc, &client, border);
            w32::fill_rect(hdc, &RECT { left: HINT_CARD_BORDER_PIXELS, top: HINT_CARD_BORDER_PIXELS, right: client.right - HINT_CARD_BORDER_PIXELS, bottom: client.bottom - HINT_CARD_BORDER_PIXELS }, card);
            w32::delete_object(border);
            w32::delete_object(card);
            let rect = |area: Area| RECT { left: area.left, top: area.top, right: area.right, bottom: area.bottom };
            let fonts = &shown.fonts;
            w32::draw_label_text(hdc, shown.title, &mut rect(shown.layout.title), fonts.prompt_title, PROMPT_TITLE_COLOR.colorref());
            w32::draw_wrapped_text(hdc, &shown.text, &mut rect(shown.layout.text), fonts.prompt_hint, PROMPT_HINT_COLOR.colorref());
            w32::draw_label_text(hdc, shown.ok, &mut rect(shown.layout.ok), fonts.prompt_hint, PROMPT_WORD_COLOR.colorref());
            w32::draw_label_text(hdc, shown.never, &mut rect(shown.layout.never), fonts.prompt_hint, PROMPT_WORD_COLOR.colorref());
        }
    });
    // SAFETY: ends the paint begun above.
    unsafe { EndPaint(window, &raw const paint) };
}

/// What a click at this point of the client area means.
fn hit_at(x: i32, y: i32) -> Option<(&'static str, Click, Rc<Callbacks>)> {
    SHOWN.with(|slot| {
        let shown = slot.borrow();
        let shown = shown.as_ref()?;
        card_hit(&shown.layout, x, y, shown.text_opens).map(|click| (shown.id, click, shown.callbacks.clone()))
    })
}

fn pointer_on_link(window: HWND) -> bool {
    let mut point = POINT { x: 0, y: 0 };
    // SAFETY: both calls write only into the point.
    let known = unsafe { GetCursorPos(&raw mut point) != 0 && ScreenToClient(window, &raw mut point) != 0 };
    known && hit_at(point.x, point.y).is_some()
}

fn signed_word(value: usize) -> i32 {
    value.low::<u16>().bits::<i16>().to::<i32>()
}

unsafe extern "system" fn card_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint(window);
            0
        }
        WM_MOUSEACTIVATE => MA_NOACTIVATE.to::<LRESULT>(),
        WM_LBUTTONUP => {
            let point = lparam.bits::<usize>();
            if let Some((id, click, callbacks)) = hit_at(signed_word(point), signed_word(point >> 16)) {
                (callbacks.clicked)(id, click);
            }
            0
        }
        WM_SETCURSOR if pointer_on_link(window) => {
            // SAFETY: a shared system cursor.
            unsafe { SetCursor(LoadCursorW(null_mut(), IDC_HAND)) };
            1
        }
        // SAFETY: the default procedure takes the arguments the system gave this one.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}
