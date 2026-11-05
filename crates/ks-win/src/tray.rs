//! The notification-area icon (the Python's `windows_tray_native.py`, which drew it with pystray and Pillow): the picture
//! comes from `tray_icon`, its glyphs from GDI, the icon from `Shell_NotifyIconW`, the menu from `tray_model`.
//!
//! Everything here runs on the thread of the window that owns the icon. The menu is built each time it opens, so its
//! labels and switches follow the state, as pystray's callables did.

use ks_cast::To as _;
use std::cell::{Cell, RefCell};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{HWND, POINT};
use windows_sys::Win32::Graphics::Gdi::{
    ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLIP_DEFAULT_PRECIS, CreateBitmap, CreateCompatibleDC,
    CreateDIBSection, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH, DIB_RGB_COLORS, DeleteDC, DeleteObject, OUT_DEFAULT_PRECIS,
    SelectObject, SetBkMode, SetTextColor, TRANSPARENT, TextOutW,
};
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, DestroyIcon, DestroyMenu, GetCursorPos, GetSystemMetrics, HICON,
    ICONINFO, MF_CHECKED, MF_GRAYED, MF_SEPARATOR, MF_STRING, PostMessageW, SM_CXSMICON, SetForegroundWindow,
    SetMenuDefaultItem, TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTALIGN, TrackPopupMenuEx, WM_NULL,
};

use crate::tray_icon::{BADGE_FONT_SIZE_PIXELS, Ink, TRAY_ICON_SIZE_PIXELS, badge_text, is_flag, render, scaled_bgra};
use crate::tray_model::{TrayAction, TrayState, menu_entries};
use crate::windows_native::wide;

const ICON_ID: u32 = 1;
const FONT_FACE: &str = "Segoe UI";
const FONT_WEIGHT_BOLD: i32 = 700;
const DEFAULT_SMALL_ICON_PIXELS: i32 = 16;

fn copy_text(target: &mut [u16], text: &str) {
    let limit = target.len() - 1;
    for (slot, unit) in target.iter_mut().zip(text.encode_utf16().take(limit)) {
        *slot = unit;
    }
}

/// A 32-bit top-down DIB section of `size` pixels square and the pointer to its pixels.
///
/// SAFETY of callers: the pointer is valid for `size * size * 4` bytes while the returned bitmap lives.
unsafe fn dib_section(size: usize) -> Option<(windows_sys::Win32::Graphics::Gdi::HBITMAP, *mut u8)> {
    let mut header: BITMAPINFO = unsafe { std::mem::zeroed() };
    header.bmiHeader = BITMAPINFOHEADER {
        biSize: (std::mem::size_of::<BITMAPINFOHEADER>()).low::<u32>(),
        biWidth: size.low::<i32>(),
        biHeight: -(size as i32),
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        biSizeImage: 0,
        biXPelsPerMeter: 0,
        biYPelsPerMeter: 0,
        biClrUsed: 0,
        biClrImportant: 0,
    };
    let mut bits: *mut core::ffi::c_void = null_mut();
    // SAFETY: the header describes a 32-bit RGB bitmap; with DIB_RGB_COLORS no device context is read.
    let bitmap = unsafe { CreateDIBSection(null_mut(), &header, DIB_RGB_COLORS, &mut bits, null_mut(), 0) };
    if bitmap.is_null() || bits.is_null() { None } else { Some((bitmap, bits as *mut u8)) }
}

/// The glyphs of `text` in the badge font, white on black, as a coverage mask cropped to the ink.
fn glyph_ink(text: &str) -> Option<Ink> {
    let size = TRAY_ICON_SIZE_PIXELS;
    let units: Vec<u16> = text.encode_utf16().collect();
    // SAFETY: every handle made here is released before returning; the DIB's memory is read only while it is selected
    // into the device context that drew on it.
    unsafe {
        let (bitmap, bits) = dib_section(size)?;
        let hdc = CreateCompatibleDC(null_mut());
        let face = wide(FONT_FACE);
        let font = CreateFontW(
            -BADGE_FONT_SIZE_PIXELS,
            0,
            0,
            0,
            FONT_WEIGHT_BOLD,
            0,
            0,
            0,
            DEFAULT_CHARSET.to::<u32>(),
            OUT_DEFAULT_PRECIS.to::<u32>(),
            CLIP_DEFAULT_PRECIS.to::<u32>(),
            ANTIALIASED_QUALITY.to::<u32>(),
            DEFAULT_PITCH.to::<u32>(),
            face.as_ptr(),
        );
        let old_bitmap = SelectObject(hdc, bitmap as _);
        let old_font = SelectObject(hdc, font as _);
        std::ptr::write_bytes(bits, 0, size * size * 4);
        SetBkMode(hdc, TRANSPARENT.bits::<i32>());
        SetTextColor(hdc, 0x00FF_FFFF);
        TextOutW(hdc, 0, 0, units.as_ptr(), units.len().low::<i32>());
        let pixels = std::slice::from_raw_parts(bits, size * size * 4);
        let coverage: Vec<u8> = pixels.chunks_exact(4).map(|pixel| pixel[1]).collect();
        SelectObject(hdc, old_font);
        SelectObject(hdc, old_bitmap);
        DeleteObject(font as _);
        DeleteObject(bitmap as _);
        DeleteDC(hdc);
        crop_ink(&coverage, size)
    }
}

fn crop_ink(coverage: &[u8], size: usize) -> Option<Ink> {
    let (mut left, mut top, mut right, mut bottom) = (size, size, 0, 0);
    for y in 0..size {
        for x in 0..size {
            if coverage[y * size + x] > 0 {
                left = left.min(x);
                top = top.min(y);
                right = right.max(x + 1);
                bottom = bottom.max(y + 1);
            }
        }
    }
    if right <= left || bottom <= top {
        return None;
    }
    let (width, height) = (right - left, bottom - top);
    let mut cropped = Vec::with_capacity(width * height);
    for y in top..bottom {
        cropped.extend_from_slice(&coverage[y * size + left..y * size + right]);
    }
    Some(Ink { width, height, coverage: cropped })
}

/// The icon of `state` at the size the notification area uses.
fn create_icon(state: &TrayState) -> Option<HICON> {
    let ink = if is_flag(state) { None } else { glyph_ink(badge_text(state)) };
    let canvas = render(state, ink.as_ref());
    // SAFETY: a plain metrics query.
    let metric = unsafe { GetSystemMetrics(SM_CXSMICON) };
    let target = (if metric > 0 { metric } else { DEFAULT_SMALL_ICON_PIXELS }).to::<usize>();
    let bytes = scaled_bgra(&canvas, target);
    // SAFETY: the colour DIB has room for `target * target * 4` bytes, which `bytes` is; the bitmaps are copied by
    // CreateIconIndirect and released here.
    unsafe {
        let (color, bits) = dib_section(target)?;
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), bits, bytes.len());
        // The mask is ignored for a 32-bit colour bitmap with alpha, but the call needs one.
        let mask = CreateBitmap(target.low::<i32>(), target.low::<i32>(), 1, 1, null());
        let info = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info);
        DeleteObject(color as _);
        DeleteObject(mask as _);
        if icon.is_null() { None } else { Some(icon) }
    }
}

/// The icon, the state it shows, and the window it reports to.
pub struct Tray {
    window: HWND,
    callback_message: u32,
    state: RefCell<TrayState>,
    icon: Cell<HICON>,
    added: Cell<bool>,
}

impl Tray {
    /// The icon comes up at once; `callback_message` is what the window is sent for mouse events on it.
    pub fn new(window: HWND, callback_message: u32) -> Tray {
        let tray = Tray {
            window,
            callback_message,
            state: RefCell::new(TrayState::default()),
            icon: Cell::new(null_mut()),
            added: Cell::new(false),
        };
        tray.replace_icon();
        tray.add();
        tray
    }

    fn data(&self, flags: u32) -> NOTIFYICONDATAW {
        // SAFETY: NOTIFYICONDATAW is plain data for which all zeros is a valid value.
        let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        data.cbSize = (std::mem::size_of::<NOTIFYICONDATAW>()).low::<u32>();
        data.hWnd = self.window;
        data.uID = ICON_ID;
        data.uFlags = flags;
        data
    }

    fn replace_icon(&self) {
        let fresh = create_icon(&self.state.borrow()).unwrap_or(null_mut());
        let old = self.icon.replace(fresh);
        if !old.is_null() {
            // SAFETY: the icon was made by CreateIconIndirect and the shell has been given the new one before this.
            self.notify_icon(NIM_MODIFY, NIF_ICON);
            unsafe { DestroyIcon(old) };
        }
    }

    fn notify_icon(&self, message: u32, flags: u32) -> bool {
        let mut data = self.data(flags);
        data.hIcon = self.icon.get();
        if flags & NIF_TIP != 0 {
            // A copy: on 32-bit Windows the structure is packed and its fields cannot be borrowed.
            let mut tip = data.szTip;
            copy_text(&mut tip, &self.state.borrow().title());
            data.szTip = tip;
        }
        if flags & NIF_MESSAGE != 0 {
            data.uCallbackMessage = self.callback_message;
        }
        // SAFETY: `data` is a fully initialised structure of the size it states.
        unsafe { Shell_NotifyIconW(message, &data) != 0 }
    }

    /// NIM_ADD; the shell forgets the icon when Explorer restarts, and this puts it back (TaskbarCreated).
    pub fn add(&self) {
        let added = self.notify_icon(NIM_ADD, NIF_MESSAGE | NIF_ICON | NIF_TIP);
        self.added.set(added);
    }

    /// `update`: the picture and the tooltip of `state`.
    pub fn set_state(&self, state: TrayState) {
        if *self.state.borrow() == state {
            return;
        }
        let picture_changed = {
            let current = self.state.borrow();
            (current.group, current.enabled, current.indicator_style) != (state.group, state.enabled, state.indicator_style)
        };
        let tip_changed = self.state.borrow().label() != state.label();
        *self.state.borrow_mut() = state;
        if picture_changed {
            self.replace_icon();
        }
        if tip_changed {
            self.notify_icon(NIM_MODIFY, NIF_TIP);
        }
    }

    pub fn state(&self) -> TrayState {
        self.state.borrow().clone()
    }

    /// A balloon notification; the text is cut to what the shell takes.
    pub fn notify(&self, title: &str, message: &str) {
        let mut data = self.data(NIF_INFO);
        let (mut info, mut info_title) = (data.szInfo, data.szInfoTitle);
        copy_text(&mut info, message);
        copy_text(&mut info_title, title);
        (data.szInfo, data.szInfoTitle) = (info, info_title);
        // SAFETY: as in `notify_icon`.
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
    }

    /// The menu at the cursor; the line chosen, if one was.
    pub fn popup_menu(&self) -> Option<TrayAction> {
        let entries = menu_entries(&self.state.borrow());
        // SAFETY: the menu is made and destroyed here; the window is the one that owns the icon, which has to be in front
        // for the menu to close when the user clicks elsewhere (as pystray did).
        let chosen = unsafe {
            let menu = CreatePopupMenu();
            if menu.is_null() {
                return None;
            }
            for (index, entry) in entries.iter().enumerate() {
                if entry.separator {
                    AppendMenuW(menu, MF_SEPARATOR, 0, null());
                    continue;
                }
                let mut flags = MF_STRING;
                if !entry.enabled {
                    flags |= MF_GRAYED;
                }
                if entry.checked == Some(true) {
                    flags |= MF_CHECKED;
                }
                AppendMenuW(menu, flags, index + 1, wide(&entry.label).as_ptr());
                if entry.default {
                    SetMenuDefaultItem(menu, index.low::<u32>(), 1);
                }
            }
            let mut point = POINT { x: 0, y: 0 };
            GetCursorPos(&mut point);
            SetForegroundWindow(self.window);
            let chosen = TrackPopupMenuEx(
                menu,
                TPM_RIGHTALIGN | TPM_BOTTOMALIGN | TPM_RETURNCMD,
                point.x,
                point.y,
                self.window,
                null(),
            );
            PostMessageW(self.window, WM_NULL, 0, 0);
            DestroyMenu(menu);
            chosen
        };
        let index = usize::try_from(chosen).ok()?.checked_sub(1)?;
        entries.get(index)?.action
    }

    /// `close`: the icon leaves the notification area.
    pub fn close(&self) {
        if self.added.replace(false) {
            self.notify_icon(NIM_DELETE, 0);
        }
        let icon = self.icon.replace(null_mut());
        if !icon.is_null() {
            // SAFETY: the icon was made by CreateIconIndirect and the shell no longer shows it.
            unsafe { DestroyIcon(icon) };
        }
    }
}
