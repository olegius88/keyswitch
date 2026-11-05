//! Thin helpers over the standard Win32 controls: creating them, fonts and brushes at the monitor's DPI, the dark
//! scheme, the owner-drawn check boxes, radio buttons and navigation buttons, lists and combo boxes. Everything here is
//! a call or two of the API behind a safe signature; the windows that use it keep the layout and the behaviour.

// The raw pointers these functions take are window, font and brush handles: opaque numbers the system looks up, never
// dereferenced here, so a stale one fails the call and does nothing worse.
#![allow(clippy::not_unsafe_ptr_arg_deref)]
use ks_cast::To as _;

use std::ffi::c_void;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows_sys::Win32::Graphics::Gdi::{
    InvalidateRect,
    CreateFontIndirectW, CreateFontW, CreatePen, CreateSolidBrush, DT_CALCRECT, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
    DT_WORDBREAK, DeleteObject, DrawFocusRect, DrawTextW, Ellipse, FillRect, GetDC, GetDeviceCaps, GetTextExtentPoint32W, HBRUSH,
    HFONT, HGDIOBJ, LOGPIXELSY, LineTo, MoveToEx, PS_SOLID, Rectangle, ReleaseDC, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows_sys::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY, OFN_PATHMUSTEXIST, OPENFILENAMEW};
use windows_sys::Win32::UI::Controls::{
    DRAWITEMSTRUCT, ODS_FOCUS,
    LVCF_TEXT, LVCF_WIDTH, LVCOLUMNW, LVIF_TEXT, LVITEMW, LVM_DELETEALLITEMS, LVM_GETNEXTITEM,
    LVM_INSERTCOLUMNW, LVM_INSERTITEMW, LVM_SETEXTENDEDLISTVIEWSTYLE, LVM_SETITEMCOUNT, LVM_SETITEMTEXTW, LVM_SETBKCOLOR,
    LVM_SETTEXTBKCOLOR, LVM_SETTEXTCOLOR, LVNI_SELECTED, LVS_EX_DOUBLEBUFFER, LVS_EX_FULLROWSELECT, LVS_REPORT,
    LVS_SHOWSELALWAYS, SetWindowTheme,
};
use windows_sys::Win32::UI::HiDpi::SystemParametersInfoForDpi;
use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CB_ADDSTRING, CB_FINDSTRINGEXACT, CB_GETCURSEL, CB_GETLBTEXT, CB_GETLBTEXTLEN, CB_RESETCONTENT, CB_SETCURSEL, CB_SETITEMHEIGHT,
    CreateWindowExW, DestroyWindow, GWLP_USERDATA, GetClientRect, GetParent, GetWindowLongPtrW, GetWindowTextLengthW,
    GetWindowTextW, LB_ADDSTRING, LB_GETCURSEL, LB_GETTEXT, LB_GETTEXTLEN, LB_RESETCONTENT, MB_ICONERROR,
    MB_ICONINFORMATION, MB_ICONQUESTION, MB_OK, MB_YESNO, MessageBoxW, NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS,
    SendMessageW, SetWindowLongPtrW, SetWindowTextW, WM_MOUSEWHEEL, WM_SETFONT,
};

use crate::windows_native::wide;
use crate::windows_ui_model::Palette;

pub fn scaled(pixels: i32, dpi: i32) -> i32 {
    // Round half up, as MulDiv does.
    (pixels * dpi + crate::constants::BASE_DPI / 2) / crate::constants::BASE_DPI
}

pub fn hinstance() -> *mut c_void {
    // SAFETY: a null name asks for the module of the running executable.
    unsafe { GetModuleHandleW(null()) }
}

pub fn send(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: the message is sent to a window of this thread (or a stale handle, which the call rejects); the callers
    // pass pointers that live through the call.
    unsafe { SendMessageW(window, message, wparam, lparam) }
}

pub fn destroy(window: HWND) {
    if !window.is_null() {
        // SAFETY: the window belongs to this thread; a stale handle fails the call.
        unsafe { DestroyWindow(window) };
    }
}

pub fn parent_of(window: HWND) -> HWND {
    // SAFETY: takes a handle by value.
    unsafe { GetParent(window) }
}

pub fn client_rect(window: HWND) -> RECT {
    let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    // SAFETY: fills the structure it is given.
    unsafe { GetClientRect(window, &mut rect) };
    rect
}

pub fn invalidate(window: HWND) {
    // SAFETY: takes a handle by value; a null rectangle is the whole client area.
    unsafe { InvalidateRect(window, null(), 1) };
}

/// `CreateWindowExW` with a class given by name; the id goes in the menu parameter, as for a child control.
pub fn create_window(class: &str, text: &str, style: u32, ex_style: u32, parent: HWND, id: usize, rect: (i32, i32, i32, i32)) -> HWND {
    let class = wide(class);
    let text = wide(text);
    // SAFETY: both strings are NUL-terminated and outlive the call; the parent is a window of this thread.
    unsafe {
        CreateWindowExW(
            ex_style,
            class.as_ptr(),
            text.as_ptr(),
            style,
            rect.0,
            rect.1,
            rect.2,
            rect.3,
            parent,
            id.pointer_mut::<c_void>(),
            hinstance(),
            null(),
        )
    }
}

pub fn set_font(window: HWND, font: HFONT) {
    send(window, WM_SETFONT, font.to::<WPARAM>(), 1);
}

pub fn set_text(window: HWND, text: &str) {
    let text = wide(text);
    // SAFETY: the text is NUL-terminated and outlives the call.
    unsafe { SetWindowTextW(window, text.as_ptr()) };
}

pub fn text_of(window: HWND) -> String {
    // SAFETY: the buffer is as long as the length the control reported, plus the terminator.
    unsafe {
        let length = GetWindowTextLengthW(window).max(0).to::<usize>();
        let mut buffer = vec![0u16; length + 1];
        let copied = (GetWindowTextW(window, buffer.as_mut_ptr(), buffer.len().low::<i32>()).max(0)).to::<usize>();
        String::from_utf16_lossy(&buffer[..copied])
    }
}

// ----------------------------------------------------------------------------------------------------------- fonts

/// The fonts of one DPI: the system's message font (Segoe UI at the user's size) and the sizes the Python's styles use.
pub struct Fonts {
    pub dpi: i32,
    pub base: HFONT,
    pub semibold: HFONT,
    pub title: HFONT,
    pub brand: HFONT,
    pub frame: HFONT,
    pub small: HFONT,
    pub test: HFONT,
    pub prompt_title: HFONT,
    pub prompt_word: HFONT,
    pub prompt_hint: HFONT,
    pub mono: HFONT,
}

fn face_font(face: &str, points: i32, dpi: i32) -> HFONT {
    let face = wide(face);
    // SAFETY: the face name is NUL-terminated; the font is a GDI object this module's owner deletes.
    unsafe { CreateFontW(-(points * dpi + 36) / 72, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 0, face.as_ptr()) }
}

impl Fonts {
    pub fn new(dpi: i32) -> Fonts {
        // SAFETY: NONCLIENTMETRICSW is plain data; the call fills it for the size it is told.
        let metrics = unsafe {
            let mut metrics: NONCLIENTMETRICSW = std::mem::zeroed();
            metrics.cbSize = (std::mem::size_of::<NONCLIENTMETRICSW>()).low::<u32>();
            let filled = SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS, metrics.cbSize, std::ptr::from_mut(&mut metrics).cast::<c_void>(), 0, dpi.bits::<u32>());
            if filled == 0 { None } else { Some(metrics) }
        };
        let (base, semibold) = match metrics {
            Some(metrics) => {
                let mut regular = metrics.lfMessageFont;
                regular.lfWeight = 400;
                let mut heavy = regular;
                let face: Vec<u16> = "Segoe UI Semibold".encode_utf16().collect();
                heavy.lfFaceName = [0; 32];
                heavy.lfFaceName[..face.len()].copy_from_slice(&face);
                // SAFETY: the structures are fully initialised LOGFONTs; the fonts are deleted by `destroy`.
                unsafe { (CreateFontIndirectW(&regular), CreateFontIndirectW(&heavy)) }
            }
            None => (face_font("Segoe UI", 9, dpi), face_font("Segoe UI Semibold", 9, dpi)),
        };
        let mono = face_font("Consolas", 9, dpi);
        Fonts {
            dpi,
            base,
            semibold,
            title: face_font("Segoe UI Semibold", crate::constants::PAGE_TITLE_FONT_SIZE_POINTS, dpi),
            brand: face_font("Segoe UI Semibold", crate::constants::BRAND_LABEL_FONT_SIZE_POINTS, dpi),
            frame: face_font("Segoe UI Semibold", crate::constants::LABELFRAME_TITLE_FONT_SIZE_POINTS, dpi),
            small: face_font("Segoe UI", crate::constants::RESET_BUTTON_FONT_SIZE_POINTS, dpi),
            test: face_font("Segoe UI", crate::constants::TEST_ENTRY_FONT_SIZE_POINTS, dpi),
            prompt_title: face_font("Segoe UI Semibold", crate::constants::LEARNING_PROMPT_TITLE_FONT_SIZE_POINTS, dpi),
            prompt_word: face_font("Segoe UI Semibold", crate::constants::LEARNING_PROMPT_WORD_FONT_SIZE_POINTS, dpi),
            prompt_hint: face_font("Segoe UI", crate::constants::LEARNING_PROMPT_HINT_FONT_SIZE_POINTS, dpi),
            mono,
        }
    }

    pub fn destroy(&self) {
        for font in [
            self.base, self.semibold, self.title, self.brand, self.frame, self.small, self.test, self.prompt_title, self.prompt_word,
            self.prompt_hint, self.mono,
        ] {
            delete_object(font);
        }
    }
}

pub fn delete_object(object: HGDIOBJ) {
    if !object.is_null() {
        // SAFETY: the object was created by this program and is no longer selected into a device context.
        unsafe { DeleteObject(object) };
    }
}

/// The brushes of one scheme.
pub struct Brushes {
    pub palette: Palette,
    pub content: HBRUSH,
    pub background: HBRUSH,
    pub sidebar: HBRUSH,
    pub field: HBRUSH,
}

impl Brushes {
    pub fn new(palette: Palette) -> Brushes {
        // SAFETY: plain colour brushes, deleted by `destroy`.
        unsafe {
            Brushes {
                palette,
                content: CreateSolidBrush(palette.content.colorref()),
                background: CreateSolidBrush(palette.background.colorref()),
                sidebar: CreateSolidBrush(palette.sidebar.colorref()),
                field: CreateSolidBrush(palette.field.colorref()),
            }
        }
    }

    pub fn destroy(&self) {
        for brush in [self.content, self.background, self.sidebar, self.field] {
            delete_object(brush);
        }
    }
}

// ------------------------------------------------------------------------------------------------------ measuring

/// The width of one line of `text` in `font`.
pub fn text_width(font: HFONT, text: &str) -> i32 {
    let wide_text: Vec<u16> = text.encode_utf16().collect();
    // SAFETY: the device context is the screen's, released at once; the font is selected and put back.
    unsafe {
        let hdc = GetDC(null_mut());
        let old = SelectObject(hdc, font);
        let mut size = windows_sys::Win32::Foundation::SIZE { cx: 0, cy: 0 };
        GetTextExtentPoint32W(hdc, wide_text.as_ptr(), wide_text.len().low::<i32>(), &mut size);
        SelectObject(hdc, old);
        ReleaseDC(null_mut(), hdc);
        size.cx
    }
}

/// The height `text` takes when it wraps at `width`.
pub fn wrapped_height(font: HFONT, text: &str, width: i32) -> i32 {
    let mut wide_text: Vec<u16> = text.encode_utf16().collect();
    if wide_text.is_empty() {
        wide_text.push((b' ').to::<u16>());
    }
    let mut rect = RECT { left: 0, top: 0, right: width.max(1), bottom: 0 };
    // SAFETY: as in `text_width`; DT_CALCRECT only measures.
    unsafe {
        let hdc = GetDC(null_mut());
        let old = SelectObject(hdc, font);
        DrawTextW(hdc, wide_text.as_ptr(), wide_text.len().low::<i32>(), &mut rect, DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX | DT_LEFT);
        SelectObject(hdc, old);
        ReleaseDC(null_mut(), hdc);
    }
    rect.bottom - rect.top
}

/// The height of one line of `font`.
pub fn line_height(font: HFONT) -> i32 {
    wrapped_height(font, "Ag", 10_000)
}

pub fn pixels_per_inch() -> i32 {
    // SAFETY: the screen's device context, released at once.
    unsafe {
        let hdc = GetDC(null_mut());
        let dpi = GetDeviceCaps(hdc, LOGPIXELSY.bits::<i32>());
        ReleaseDC(null_mut(), hdc);
        if dpi > 0 { dpi } else { crate::constants::BASE_DPI }
    }
}

// ------------------------------------------------------------------------------------------------- the dark scheme

/// What `SetWindowTheme` should be told for a control of this kind in the dark scheme.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Themed {
    /// Push buttons, list views, scroll bars.
    Explorer,
    /// Edit boxes and combo boxes.
    Field,
}

pub fn apply_control_theme(window: HWND, dark: bool, kind: Themed) {
    let name = match (dark, kind) {
        (false, _) => "Explorer",
        (true, Themed::Explorer) => "DarkMode_Explorer",
        (true, Themed::Field) => "DarkMode_CFD",
    };
    let name = wide(name);
    // SAFETY: the name is NUL-terminated and a null class list keeps the control's own.
    unsafe { SetWindowTheme(window, name.as_ptr(), null()) };
}

/// The title bar of the window in the dark scheme (Windows 10 1809 and later; an older Windows keeps its light one).
pub fn set_title_bar_dark(window: HWND, dark: bool) {
    let value: i32 = i32::from(dark);
    // SAFETY: the attribute's value is a BOOL of four bytes that lives through the call.
    unsafe {
        DwmSetWindowAttribute(window, DWMWA_USE_IMMERSIVE_DARK_MODE.to::<u32>(), std::ptr::from_ref(&value).cast::<c_void>(), 4);
    }
}

// -------------------------------------------------------------------------------------- owner-drawn buttons

const STATE_CHECKED: isize = 1;
const STATE_MODIFIED: isize = 2;

pub fn is_checked(window: HWND) -> bool {
    // SAFETY: reads the word this module stores in the window's user data.
    unsafe { GetWindowLongPtrW(window, GWLP_USERDATA).to::<isize>() & STATE_CHECKED != 0 }
}

pub fn set_checked(window: HWND, checked: bool) {
    // SAFETY: as above, and a repaint request.
    unsafe {
        let state = GetWindowLongPtrW(window, GWLP_USERDATA).to::<isize>() & !STATE_CHECKED;
        SetWindowLongPtrW(window, GWLP_USERDATA, (state | if checked { STATE_CHECKED } else { 0 }).to::<isize>());
    }
    invalidate(window);
}

/// A setting changed away from its default shows its label in the accent colour.
pub fn set_modified(window: HWND, modified: bool) {
    // SAFETY: as above.
    unsafe {
        let state = GetWindowLongPtrW(window, GWLP_USERDATA).to::<isize>() & !STATE_MODIFIED;
        SetWindowLongPtrW(window, GWLP_USERDATA, (state | if modified { STATE_MODIFIED } else { 0 }).to::<isize>());
    }
    invalidate(window);
}

fn rgb(color: crate::windows_ui_model::Rgb) -> u32 {
    color.colorref()
}

fn with_pen_and_brush(hdc: *mut c_void, pen_color: u32, pen_width: i32, brush: HBRUSH, draw: impl FnOnce()) {
    // SAFETY: the pen is made, selected, used and deleted here; the previous objects are selected back.
    unsafe {
        let pen = CreatePen(PS_SOLID, pen_width, pen_color);
        let old_pen = SelectObject(hdc, pen);
        let old_brush = SelectObject(hdc, brush);
        draw();
        SelectObject(hdc, old_pen);
        SelectObject(hdc, old_brush);
        DeleteObject(pen);
    }
}

fn draw_text(hdc: *mut c_void, text: &str, rect: &mut RECT, flags: u32) {
    let text: Vec<u16> = text.encode_utf16().collect();
    // SAFETY: the buffer and the rectangle live through the call.
    unsafe { DrawTextW(hdc, text.as_ptr(), text.len().low::<i32>(), rect, flags) };
}

/// Draws a check box or a radio button (the owner-drawn kind keeps its colours in both schemes).
pub fn draw_choice(item: &DRAWITEMSTRUCT, brushes: &Brushes, font: HFONT, dpi: i32, radio: bool) {
    let palette = &brushes.palette;
    let hdc = item.hDC;
    let rect = item.rcItem;
    // SAFETY: reads the state word of the item's window.
    let state = (unsafe { GetWindowLongPtrW(item.hwndItem, GWLP_USERDATA) }).to::<isize>();
    let (checked, modified) = (state & STATE_CHECKED != 0, state & STATE_MODIFIED != 0);
    let size = scaled(16, dpi);
    let top = rect.top + scaled(1, dpi);
    // SAFETY: draws into the device context the system gave for this paint; the font is selected and put back.
    unsafe {
        FillRect(hdc, &rect, brushes.content);
        let old_font = SelectObject(hdc, font);
        let (left, right, bottom) = (rect.left + 1, rect.left + 1 + size, top + size);
        let face = if checked && !radio { CreateSolidBrush(rgb(palette.accent)) } else { CreateSolidBrush(rgb(palette.field)) };
        let outline = if checked { rgb(palette.accent) } else { rgb(palette.muted) };
        with_pen_and_brush(hdc, outline, 1, face, || {
            if radio {
                Ellipse(hdc, left, top, right, bottom);
            } else {
                Rectangle(hdc, left, top, right, bottom);
            }
        });
        DeleteObject(face);
        if checked {
            if radio {
                let dot = CreateSolidBrush(rgb(palette.accent));
                let inset = scaled(4, dpi);
                with_pen_and_brush(hdc, rgb(palette.accent), 1, dot, || {
                    Ellipse(hdc, left + inset, top + inset, right - inset, bottom - inset);
                });
                DeleteObject(dot);
            } else {
                let pen = CreatePen(PS_SOLID, scaled(2, dpi).max(2), 0x00ff_ffff);
                let old_pen = SelectObject(hdc, pen);
                let unit = size.to::<f32>() / 16.0;
                let at = |x: f32, y: f32| ((left.to::<f32>() + x * unit).to::<i32>(), (top.to::<f32>() + y * unit).to::<i32>());
                let (x1, y1) = at(3.5, 8.5);
                let (x2, y2) = at(6.5, 11.5);
                let (x3, y3) = at(12.5, 4.5);
                MoveToEx(hdc, x1, y1, null_mut());
                LineTo(hdc, x2, y2);
                LineTo(hdc, x3, y3);
                SelectObject(hdc, old_pen);
                DeleteObject(pen);
            }
        }
        SetBkMode(hdc, TRANSPARENT.bits::<i32>());
        SetTextColor(hdc, if modified { rgb(palette.accent) } else { rgb(palette.foreground) });
        let mut text_rect = RECT { left: right + scaled(8, dpi), top: rect.top, right: rect.right, bottom: rect.bottom };
        let text = text_of(item.hwndItem);
        draw_text(hdc, &text, &mut text_rect, DT_WORDBREAK | DT_NOPREFIX | DT_LEFT);
        if item.itemState & ODS_FOCUS != 0 {
            DrawFocusRect(hdc, &RECT { left: text_rect.left - 2, top: rect.top, right: rect.right, bottom: rect.bottom });
        }
        SelectObject(hdc, old_font);
    }
}

/// Draws one of the sidebar's navigation buttons: flat, white text, the accent colour for the page that is open.
pub fn draw_navigation(item: &DRAWITEMSTRUCT, fill: HBRUSH, text_color: u32, font: HFONT, dpi: i32) {
    let hdc = item.hDC;
    let mut rect = item.rcItem;
    // SAFETY: draws into the device context of this paint; the font is selected and put back.
    unsafe {
        FillRect(hdc, &rect, fill);
        let old_font = SelectObject(hdc, font);
        SetBkMode(hdc, TRANSPARENT.bits::<i32>());
        SetTextColor(hdc, text_color);
        rect.left += scaled(crate::constants::NAVIGATION_BUTTON_PADDING_X_PIXELS, dpi);
        draw_text(hdc, &text_of(item.hwndItem), &mut rect, DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_NOPREFIX);
        if item.itemState & ODS_FOCUS != 0 {
            let mut inner = item.rcItem;
            inner.left += 2;
            inner.top += 2;
            inner.right -= 2;
            inner.bottom -= 2;
            DrawFocusRect(hdc, &inner);
        }
        SelectObject(hdc, old_font);
    }
}

pub fn fill_rect(hdc: *mut c_void, rect: &RECT, brush: HBRUSH) {
    // SAFETY: draws into a device context of this thread.
    unsafe { FillRect(hdc, rect, brush) };
}

pub fn draw_label_text(hdc: *mut c_void, text: &str, rect: &mut RECT, font: HFONT, color: u32) {
    // SAFETY: the font is selected into the device context of this paint and put back.
    unsafe {
        let old = SelectObject(hdc, font);
        SetBkMode(hdc, TRANSPARENT.bits::<i32>());
        SetTextColor(hdc, color);
        draw_text(hdc, text, rect, DT_SINGLELINE | DT_LEFT | DT_NOPREFIX);
        SelectObject(hdc, old);
    }
}

/// The outline of a section: a one-pixel rectangle.
pub fn frame_rect(hdc: *mut c_void, rect: &RECT, color: u32) {
    // SAFETY: a pen made and deleted here; a null brush leaves the inside as it is.
    unsafe {
        let pen = CreatePen(PS_SOLID, 1, color);
        let old_pen = SelectObject(hdc, pen);
        let old_brush = SelectObject(hdc, windows_sys::Win32::Graphics::Gdi::GetStockObject(windows_sys::Win32::Graphics::Gdi::NULL_BRUSH));
        Rectangle(hdc, rect.left, rect.top, rect.right, rect.bottom);
        SelectObject(hdc, old_pen);
        SelectObject(hdc, old_brush);
        DeleteObject(pen);
    }
}

// ------------------------------------------------------------------------------------------------------ the wheel

unsafe extern "system" fn wheel_guard(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
    if message == WM_MOUSEWHEEL {
        // The wheel scrolls the page; it never changes the value of a combo box or a number box the pointer is over.
        send(parent_of(window), WM_MOUSEWHEEL, wparam, lparam);
        return 0;
    }
    // SAFETY: passes the message on with the arguments the system gave.
    unsafe { DefSubclassProc(window, message, wparam, lparam) }
}

pub fn guard_wheel(window: HWND) {
    // SAFETY: the procedure is a plain function that outlives the window.
    unsafe { SetWindowSubclass(window, Some(wheel_guard), 1, 0) };
}

// ------------------------------------------------------------------------------------------------- list boxes

pub fn listbox_reset(window: HWND, items: &[String]) {
    send(window, LB_RESETCONTENT, 0, 0);
    for item in items {
        let text = wide(item);
        send(window, LB_ADDSTRING, 0, text.as_ptr().to::<LPARAM>());
    }
}

pub fn listbox_selected(window: HWND) -> Option<String> {
    let index = send(window, LB_GETCURSEL, 0, 0);
    if index < 0 {
        return None;
    }
    let length = (send(window, LB_GETTEXTLEN, index.bits::<WPARAM>(), 0).max(0)).bits::<usize>();
    let mut buffer = vec![0u16; length + 1];
    send(window, LB_GETTEXT, index.bits::<WPARAM>(), buffer.as_mut_ptr().to::<LPARAM>());
    Some(String::from_utf16_lossy(&buffer[..length]))
}

// ------------------------------------------------------------------------------------------------------ combos

/// A drop-down list with its selection field `field_height` tall and room for `rows` entries.
pub fn combo_set_field_height(window: HWND, field_height: i32) {
    send(window, CB_SETITEMHEIGHT, usize::MAX, field_height.to::<LPARAM>());
}

pub fn combo_reset(window: HWND, items: &[String]) {
    send(window, CB_RESETCONTENT, 0, 0);
    for item in items {
        let text = wide(item);
        send(window, CB_ADDSTRING, 0, text.as_ptr().to::<LPARAM>());
    }
}

/// Selects the entry with exactly this text; false when there is none.
pub fn combo_select_text(window: HWND, text: &str) -> bool {
    let wide_text = wide(text);
    let found = send(window, CB_FINDSTRINGEXACT, usize::MAX, wide_text.as_ptr().to::<LPARAM>());
    if found < 0 {
        send(window, CB_SETCURSEL, usize::MAX, 0);
        return false;
    }
    send(window, CB_SETCURSEL, found.bits::<WPARAM>(), 0);
    true
}

pub fn combo_selected_text(window: HWND) -> Option<String> {
    let index = send(window, CB_GETCURSEL, 0, 0);
    if index < 0 {
        return None;
    }
    let length = (send(window, CB_GETLBTEXTLEN, index.bits::<WPARAM>(), 0).max(0)).bits::<usize>();
    let mut buffer = vec![0u16; length + 1];
    send(window, CB_GETLBTEXT, index.bits::<WPARAM>(), buffer.as_mut_ptr().to::<LPARAM>());
    Some(String::from_utf16_lossy(&buffer[..length]))
}

pub fn combo_selected_index(window: HWND) -> Option<usize> {
    usize::try_from(send(window, CB_GETCURSEL, 0, 0)).ok()
}

// ----------------------------------------------------------------------------------------------------- list views

pub const LIST_VIEW_CLASS: &str = "SysListView32";

pub fn list_view_styles() -> u32 {
    LVS_REPORT | LVS_SHOWSELALWAYS
}

/// Report-style list with whole-row selection and the given columns (title, width at 96 dpi).
pub fn list_view_setup(window: HWND, columns: &[(&str, i32)], dpi: i32) {
    send(window, LVM_SETEXTENDEDLISTVIEWSTYLE, 0, (LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER).to::<LPARAM>());
    for (index, (title, width)) in columns.iter().enumerate() {
        let mut text = wide(title);
        // SAFETY: LVCOLUMNW is plain data; the text pointer lives through the SendMessage call.
        let mut column: LVCOLUMNW = unsafe { std::mem::zeroed() };
        column.mask = LVCF_TEXT | LVCF_WIDTH;
        column.cx = scaled(*width, dpi);
        column.pszText = text.as_mut_ptr();
        send(window, LVM_INSERTCOLUMNW, index, std::ptr::from_ref(&column).to::<LPARAM>());
    }
}

pub fn list_view_colors(window: HWND, background: u32, text: u32) {
    send(window, LVM_SETBKCOLOR, 0, background.to::<LPARAM>());
    send(window, LVM_SETTEXTBKCOLOR, 0, background.to::<LPARAM>());
    send(window, LVM_SETTEXTCOLOR, 0, text.to::<LPARAM>());
}

pub fn list_view_set_rows(window: HWND, rows: &[Vec<String>]) {
    send(window, LVM_DELETEALLITEMS, 0, 0);
    send(window, LVM_SETITEMCOUNT, rows.len(), 0);
    for (index, row) in rows.iter().enumerate() {
        for (column, cell) in row.iter().enumerate() {
            let mut text = wide(cell);
            // SAFETY: LVITEMW is plain data; the text pointer lives through the SendMessage call.
            let mut item: LVITEMW = unsafe { std::mem::zeroed() };
            item.mask = LVIF_TEXT;
            item.iItem = index.low::<i32>();
            item.iSubItem = column.low::<i32>();
            item.pszText = text.as_mut_ptr();
            if column == 0 {
                send(window, LVM_INSERTITEMW, 0, std::ptr::from_ref(&item).to::<LPARAM>());
            } else {
                send(window, LVM_SETITEMTEXTW, index, std::ptr::from_ref(&item).to::<LPARAM>());
            }
        }
    }
}

pub fn list_view_selected(window: HWND) -> Vec<usize> {
    let mut selected = Vec::new();
    let mut from: isize = -1;
    loop {
        let next = send(window, LVM_GETNEXTITEM, from.bits::<WPARAM>(), LVNI_SELECTED.to::<LPARAM>());
        if next < 0 {
            break;
        }
        selected.push(next.bits::<usize>());
        from = next;
    }
    selected
}


// ------------------------------------------------------------------------------------------------- the dialogs

/// A message box over `owner`: an information box, an error, or a question answered yes or no.
pub fn message_box(owner: HWND, title: &str, text: &str, kind: MessageKind) -> bool {
    let (title, text) = (wide(title), wide(text));
    let style = match kind {
        MessageKind::Information => MB_OK | MB_ICONINFORMATION,
        MessageKind::Error => MB_OK | MB_ICONERROR,
        MessageKind::Question => MB_YESNO | MB_ICONQUESTION,
    };
    // SAFETY: both strings are NUL-terminated and outlive the call.
    let answer = unsafe { MessageBoxW(owner, text.as_ptr(), title.as_ptr(), style) };
    answer == windows_sys::Win32::UI::WindowsAndMessaging::IDYES || kind != MessageKind::Question
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    Information,
    Error,
    Question,
}

/// The open-file dialog for programs: `*.exe` first, then every file. None when the user cancels.
pub fn pick_executable(owner: HWND, title: &str) -> Option<String> {
    let title = wide(title);
    let mut filter: Vec<u16> = "Приложения Windows\0*.exe\0Все файлы\0*.*\0".encode_utf16().collect();
    filter.push(0);
    let mut buffer = vec![0u16; 1024];
    // SAFETY: OPENFILENAMEW is plain data; the buffers and strings live through the call.
    unsafe {
        let mut dialog: OPENFILENAMEW = std::mem::zeroed();
        dialog.lStructSize = (std::mem::size_of::<OPENFILENAMEW>()).low::<u32>();
        dialog.hwndOwner = owner;
        dialog.lpstrFilter = filter.as_ptr();
        dialog.lpstrFile = buffer.as_mut_ptr();
        dialog.nMaxFile = buffer.len().low::<u32>();
        dialog.lpstrTitle = title.as_ptr();
        dialog.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_HIDEREADONLY;
        if GetOpenFileNameW(&mut dialog) == 0 {
            return None;
        }
    }
    let end = buffer.iter().position(|unit| *unit == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..end]))
}

/// Puts text on the clipboard as Unicode text; false when the clipboard is held by another program.
pub fn set_clipboard_text(owner: HWND, text: &str) -> bool {
    let units = wide(text);
    let bytes = units.len() * 2;
    // SAFETY: the memory block is sized for the text and handed to the clipboard, which owns it after SetClipboardData; the
    // clipboard is closed on every path that opened it.
    unsafe {
        if OpenClipboard(owner) == 0 {
            return false;
        }
        EmptyClipboard();
        let block = GlobalAlloc(GMEM_MOVEABLE, bytes);
        let mut stored = false;
        if !block.is_null() {
            let target = GlobalLock(block).cast::<u16>();
            if !target.is_null() {
                std::ptr::copy_nonoverlapping(units.as_ptr(), target, units.len());
                GlobalUnlock(block);
                // CF_UNICODETEXT
                stored = !SetClipboardData(13, block).is_null();
            }
        }
        CloseClipboard();
        stored
    }
}

/// A brush of one colour; the caller deletes it with `delete_object`.
pub fn solid_brush(color: crate::windows_ui_model::Rgb) -> HBRUSH {
    // SAFETY: a plain colour brush.
    unsafe { CreateSolidBrush(color.colorref()) }
}

/// The colours a control's paint uses when the system asks the parent (`WM_CTLCOLOR*`).
pub fn set_label_colors(hdc: *mut c_void, foreground: u32, background: u32) {
    // SAFETY: sets colours of the device context the system gave for this message.
    unsafe {
        SetTextColor(hdc, foreground);
        windows_sys::Win32::Graphics::Gdi::SetBkColor(hdc, background);
    }
}

pub fn control_with_id(parent: HWND, id: usize) -> HWND {
    // SAFETY: takes a handle and a number by value.
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetDlgItem(parent, id.low::<i32>()) }
}
