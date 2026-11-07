//! The notification-area icon (the Python's `windows_tray_native.py`, which drew it with pystray and Pillow): the picture
//! comes from `tray_icon`, its glyphs from GDI, the icon from `Shell_NotifyIconW`, the menu from `tray_model`.
//!
//! Everything here runs on the thread of the window that owns the icon. The menu is built each time it opens, so its
//! labels and switches follow the state, as pystray's callables did. The icon is drawn at the size the notification area
//! shows on its monitor, and drawn again when the scale of the screen changes (`refresh_size`).

#![cfg_attr(windows, expect(unsafe_code, reason = "calls into Win32"))]
use ks_cast::To as _;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{HWND, POINT, RECT, S_OK};
use windows_sys::Win32::Graphics::Gdi::{
    ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLIP_DEFAULT_PRECIS, CreateBitmap, CreateCompatibleDC,
    CreateDIBSection, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH, DIB_RGB_COLORS, DeleteDC, DeleteObject, MONITOR_DEFAULTTONEAREST,
    MonitorFromRect, OUT_DEFAULT_PRECIS, SelectObject, SetBkMode, SetTextColor, TRANSPARENT, TextOutW,
};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, GetSystemMetricsForDpi, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, NOTIFYICONIDENTIFIER,
    Shell_NotifyIconGetRect, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, DestroyIcon, DestroyMenu, GetCursorPos, HICON, ICONINFO, MF_CHECKED,
    MF_GRAYED, MF_SEPARATOR, MF_STRING, PostMessageW, SM_CXSMICON, SetForegroundWindow, SetMenuDefaultItem, TPM_BOTTOMALIGN,
    TPM_RETURNCMD, TPM_RIGHTALIGN, TrackPopupMenuEx, WM_NULL,
};

use crate::tray_icon::{Ink, SUPERSAMPLING, badge_text, fitted_ink, is_flag, render, scaled_bgra};
use crate::tray_model::{TrayAction, TrayState, menu_entries};
use crate::windows_native::wide;

const ICON_ID: u32 = 1;
const FONT_FACE: &str = "Segoe UI";
const FONT_WEIGHT_BOLD: i32 = 700;
const DEFAULT_SMALL_ICON_PIXELS: i32 = 16;
/// The glyphs are drawn on a bitmap this many canvases wide, so that a font height tried while fitting them is measured
/// whole even when its ink would not fit the canvas.
const GLYPH_BITMAP_CANVASES: usize = 2;

thread_local! {
    /// The fitted glyphs of each label at each canvas side: the layout changes with every correction, the fitting draws a
    /// label a few times.
    static INKS: RefCell<HashMap<(&'static str, usize), Option<Ink>>> = RefCell::new(HashMap::new());
}

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
    // SAFETY: BITMAPINFO is plain data, and all zeros is a valid one.
    let mut header: BITMAPINFO = unsafe { std::mem::zeroed() };
    header.bmiHeader = BITMAPINFOHEADER {
        biSize: (size_of::<BITMAPINFOHEADER>()).low::<u32>(),
        biWidth: size.low::<i32>(),
        biHeight: -size.low::<i32>(),
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
    let bitmap = unsafe { CreateDIBSection(null_mut(), &raw const header, DIB_RGB_COLORS, &raw mut bits, null_mut(), 0) };
    if bitmap.is_null() || bits.is_null() { None } else { Some((bitmap, bits.cast::<u8>())) }
}

/// The glyphs of `text` in the badge font `font_height` pixels high, white on black, on a bitmap `size` pixels square,
/// as a coverage mask cropped to the ink.
fn glyph_ink(text: &str, font_height: i32, size: usize) -> Option<Ink> {
    let units: Vec<u16> = text.encode_utf16().collect();
    // SAFETY: every handle made here is released before returning; the DIB's memory is read only while it is selected
    // into the device context that drew on it.
    unsafe {
        let (bitmap, bits) = dib_section(size)?;
        let hdc = CreateCompatibleDC(null_mut());
        let face = wide(FONT_FACE);
        let font = CreateFontW(
            -font_height,
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
        let old_bitmap = SelectObject(hdc, bitmap);
        let old_font = SelectObject(hdc, font);
        std::ptr::write_bytes(bits, 0, size * size * 4);
        SetBkMode(hdc, TRANSPARENT.bits::<i32>());
        SetTextColor(hdc, 0x00FF_FFFF);
        TextOutW(hdc, 0, 0, units.as_ptr(), units.len().low::<i32>());
        let pixels = std::slice::from_raw_parts(bits, size * size * 4);
        let coverage: Vec<u8> = pixels.as_chunks::<4>().0.iter().map(|pixel| pixel[1]).collect();
        SelectObject(hdc, old_font);
        SelectObject(hdc, old_bitmap);
        DeleteObject(font);
        DeleteObject(bitmap);
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

/// The glyphs of a badge label fitted to a canvas `side` pixels square (`tray_icon::fitted_ink`).
fn badge_ink(text: &'static str, side: usize) -> Option<Ink> {
    INKS.with(|inks| {
        inks.borrow_mut()
            .entry((text, side))
            .or_insert_with(|| fitted_ink(side, |font_height| glyph_ink(text, font_height, side * GLYPH_BITMAP_CANVASES)))
            .clone()
    })
}

/// The picture of `state` as an icon `side` pixels square shows it: straight-alpha BGRA, row by row from the top.
#[must_use]
pub fn icon_pixels(state: &TrayState, side: usize) -> Vec<u8> {
    let canvas_side = side * SUPERSAMPLING;
    let ink = if is_flag(state) { None } else { badge_ink(badge_text(state), canvas_side) };
    scaled_bgra(&render(state, ink.as_ref(), canvas_side), side)
}

/// The side of a small icon at `dpi`: what the notification area shows on a monitor of that scale.
#[must_use]
pub fn small_icon_side(dpi: u32) -> usize {
    // SAFETY: a plain metrics query.
    let metric = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, dpi) };
    (if metric > 0 { metric } else { DEFAULT_SMALL_ICON_PIXELS }).to::<usize>()
}

/// The icon of `state`, `side` pixels square.
fn create_icon(state: &TrayState, target: usize) -> Option<HICON> {
    let bytes = icon_pixels(state, target);
    // SAFETY: the colour DIB has room for `target * target * 4` bytes, which `bytes` is; the bitmaps are copied by
    // CreateIconIndirect and released here.
    unsafe {
        let (color, bits) = dib_section(target)?;
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), bits, bytes.len());
        // The mask is ignored for a 32-bit colour bitmap with alpha, but the call needs one.
        let mask = CreateBitmap(target.low::<i32>(), target.low::<i32>(), 1, 1, null());
        let info = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&raw const info);
        DeleteObject(color);
        DeleteObject(mask);
        if icon.is_null() { None } else { Some(icon) }
    }
}

/// The icon, the state it shows, and the window it reports to.
pub struct Tray {
    window: HWND,
    callback_message: u32,
    state: RefCell<TrayState>,
    icon: Cell<HICON>,
    /// The side the icon is drawn at.
    side: Cell<usize>,
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
            side: Cell::new(0),
            added: Cell::new(false),
        };
        tray.side.set(tray.wanted_side());
        tray.replace_icon();
        tray.add();
        // The shell has placed the icon now, and its monitor may have another scale than the window's.
        tray.refresh_size();
        tray
    }

    /// The side of the icon the notification area shows: the small-icon size at the scale of the monitor the icon is on,
    /// or of the window's monitor while the shell has not placed it.
    fn wanted_side(&self) -> usize {
        // SAFETY: a plain query; the window is the one this tray reports to.
        let dpi = self.icon_monitor_dpi().unwrap_or_else(|| unsafe { GetDpiForWindow(self.window) });
        small_icon_side(dpi)
    }

    fn icon_monitor_dpi(&self) -> Option<u32> {
        // SAFETY: NOTIFYICONIDENTIFIER is plain data for which all zeros is a valid value (no GUID).
        let mut identifier: NOTIFYICONIDENTIFIER = unsafe { std::mem::zeroed() };
        identifier.cbSize = (size_of::<NOTIFYICONIDENTIFIER>()).low::<u32>();
        identifier.hWnd = self.window;
        identifier.uID = ICON_ID;
        let mut place = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        // SAFETY: both structures live through the calls, which write only into `place` and the two numbers.
        unsafe {
            if Shell_NotifyIconGetRect(&raw const identifier, &raw mut place) != S_OK {
                return None;
            }
            let monitor = MonitorFromRect(&raw const place, MONITOR_DEFAULTTONEAREST);
            let (mut horizontal, mut vertical) = (0u32, 0u32);
            if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &raw mut horizontal, &raw mut vertical) != S_OK || horizontal == 0 {
                return None;
            }
            Some(horizontal)
        }
    }

    /// The icon is drawn again when the size the notification area shows it at has changed: the window's DPI changed,
    /// the displays or the taskbar moved, or Explorer restarted.
    pub fn refresh_size(&self) {
        let side = self.wanted_side();
        if side != self.side.replace(side) {
            self.replace_icon();
        }
    }

    fn data(&self, flags: u32) -> NOTIFYICONDATAW {
        // SAFETY: NOTIFYICONDATAW is plain data for which all zeros is a valid value.
        let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        data.cbSize = (size_of::<NOTIFYICONDATAW>()).low::<u32>();
        data.hWnd = self.window;
        data.uID = ICON_ID;
        data.uFlags = flags;
        data
    }

    fn replace_icon(&self) {
        let fresh = create_icon(&self.state.borrow(), self.side.get()).unwrap_or(null_mut());
        let old = self.icon.replace(fresh);
        if !old.is_null() {
            // SAFETY: the icon was made by CreateIconIndirect and the shell has been given the new one before this.
            self.notify_icon(NIM_MODIFY, NIF_ICON);
            // SAFETY: the icon was created here and nothing shows it any more.
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
        unsafe { Shell_NotifyIconW(message, &raw const data) != 0 }
    }

    /// `NIM_ADD`; the shell forgets the icon when Explorer restarts, and this puts it back (`TaskbarCreated`).
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
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &raw const data) };
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
            GetCursorPos(&raw mut point);
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

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::path::Path;

    /// The scales of the screen Windows offers and the side of the icon at each: 100 %, 125 %, 150 %, 200 %, 250 %.
    const SCALES: [(u32, usize); 5] = [(96, 16), (120, 20), (144, 24), (192, 32), (240, 40)];
    /// A pixel at least this bright in every channel is a letter's: the badges are no brighter than 105 in their darkest.
    const LETTER_CHANNEL: u8 = 160;
    /// Each preview pixel of the sheet is this many pixels square.
    const SHEET_ZOOM: usize = 8;
    const SHEET_GAP_PIXELS: usize = 8;
    /// The dark taskbar of Windows 11.
    const SHEET_BACKGROUND: [u8; 4] = [0x1C, 0x1C, 0x1C, 0xFF];
    const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    /// IHDR after the sides: 8 bits a channel, colour type 6 (RGBA), deflate, adaptive filtering, no interlace.
    const PNG_RGBA_HEADER_TAIL: [u8; 5] = [8, 6, 0, 0, 0];

    fn badges() -> [(&'static str, TrayState); 3] {
        [
            ("en", TrayState { group: 0, ..TrayState::default() }),
            ("ru", TrayState { group: 1, ..TrayState::default() }),
            ("disabled", TrayState { group: 0, enabled: false, ..TrayState::default() }),
        ]
    }

    fn chunk(out: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
        out.extend_from_slice(&data.len().low::<u32>().to_be_bytes());
        out.extend_from_slice(&kind);
        out.extend_from_slice(data);
        let mut crc = flate2::Crc::new();
        crc.update(&kind);
        crc.update(data);
        out.extend_from_slice(&crc.sum().to_be_bytes());
    }

    /// A PNG of `width` by `height` pixels of straight-alpha BGRA.
    fn png(width: usize, height: usize, bgra: &[u8]) -> Vec<u8> {
        let mut header = Vec::new();
        header.extend_from_slice(&width.low::<u32>().to_be_bytes());
        header.extend_from_slice(&height.low::<u32>().to_be_bytes());
        header.extend_from_slice(&PNG_RGBA_HEADER_TAIL);
        let mut rows = Vec::with_capacity(height * (width * 4 + 1));
        for row in bgra.chunks(width * 4) {
            rows.push(0);
            for pixel in row.chunks(4) {
                rows.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
            }
        }
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        encoder.write_all(&rows).unwrap();
        let mut out = PNG_SIGNATURE.to_vec();
        chunk(&mut out, *b"IHDR", &header);
        chunk(&mut out, *b"IDAT", &encoder.finish().unwrap());
        chunk(&mut out, *b"IEND", &[]);
        out
    }

    /// The columns and rows the letters take, from the first to the last.
    fn letter_extent(side: usize, bgra: &[u8]) -> (usize, usize) {
        let (mut columns, mut rows) = ((side, 0), (side, 0));
        for (index, pixel) in bgra.chunks(4).enumerate() {
            if pixel[..3].iter().all(|channel| *channel >= LETTER_CHANNEL) {
                let (x, y) = (index % side, index / side);
                columns = (columns.0.min(x), columns.1.max(x + 1));
                rows = (rows.0.min(y), rows.1.max(y + 1));
            }
        }
        (columns.1.saturating_sub(columns.0), rows.1.saturating_sub(rows.0))
    }

    #[test]
    fn the_icon_grows_with_the_scale_of_the_screen() {
        for (dpi, side) in SCALES {
            assert_eq!(small_icon_side(dpi), side, "at {dpi} DPI");
        }
    }

    /// The badges at every scale: the letters take most of the icon. With `KS_TRAY_ICON_DIR` set, each picture is written
    /// there as a PNG at its own size, and all of them on one enlarged sheet, a row a badge, for a look.
    #[test]
    fn the_letters_fill_the_badge_at_every_scale() {
        let folder = std::env::var_os("KS_TRAY_ICON_DIR");
        let mut sheet_rows = Vec::new();
        for (name, state) in badges() {
            let mut row = Vec::new();
            for (_, side) in SCALES {
                let pixels = icon_pixels(&state, side);
                assert_eq!(pixels.len(), side * side * 4);
                if state.enabled {
                    let (width, height) = letter_extent(side, &pixels);
                    assert!(width * 4 >= side * 3, "{name} at {side}: the letters are {width} pixels wide");
                    assert!(height * 5 >= side * 2, "{name} at {side}: the letters are {height} pixels tall");
                }
                if let Some(folder) = &folder {
                    std::fs::create_dir_all(folder).unwrap();
                    std::fs::write(Path::new(folder).join(format!("badge-{name}-{side}.png")), png(side, side, &pixels)).unwrap();
                }
                row.push((side, pixels));
            }
            sheet_rows.push(row);
        }
        let Some(folder) = folder else { return };
        let row_height = SCALES.iter().map(|(_, side)| side * SHEET_ZOOM).max().unwrap() + SHEET_GAP_PIXELS;
        let width = SCALES.iter().map(|(_, side)| side * SHEET_ZOOM + SHEET_GAP_PIXELS).sum::<usize>() + SHEET_GAP_PIXELS;
        let height = sheet_rows.len() * row_height + SHEET_GAP_PIXELS;
        let mut sheet: Vec<u8> = SHEET_BACKGROUND.iter().copied().cycle().take(width * height * 4).collect();
        for (row_index, row) in sheet_rows.iter().enumerate() {
            let mut left = SHEET_GAP_PIXELS;
            for (side, pixels) in row {
                let top = SHEET_GAP_PIXELS + row_index * row_height;
                for y in 0..side * SHEET_ZOOM {
                    for x in 0..side * SHEET_ZOOM {
                        let source = &pixels[((y / SHEET_ZOOM) * side + x / SHEET_ZOOM) * 4..][..4];
                        let target = &mut sheet[((top + y) * width + left + x) * 4..][..4];
                        let alpha = u32::from(source[3]);
                        for channel in 0..3 {
                            target[channel] = ((u32::from(source[channel]) * alpha + u32::from(target[channel]) * (255 - alpha) + 127) / 255).low::<u8>();
                        }
                    }
                }
                left += side * SHEET_ZOOM + SHEET_GAP_PIXELS;
            }
        }
        std::fs::write(Path::new(&folder).join("badges-enlarged.png"), png(width, height, &sheet)).unwrap();
    }
}
