//! Direct Win32 API binding used by `windows_backend`.
//!
//! The first part holds what the binding decides without calling Windows, so that it is tested on every platform: how a
//! hook's arguments become a key event, how an input becomes a KEYBDINPUT, the keyboard state of a translation, and the
//! ring of input times. The second part, on Windows only, makes the calls (`CtypesWindowsAPI` of the Python is
//! `NativeWindowsAPI` here).
//!
//! The hook procedures get no pointer of their own from the system, so what they need lives in process-wide statics:
//! the ring, written before anything else at each event, and the listener, set before the hook is installed and taken
//! away after it is removed. There is one keyboard hook per process.

#![cfg_attr(windows, expect(unsafe_code, reason = "calls into Win32"))]
use ks_cast::To as _;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use ks_backend::constants::{ALT_MASK, CONTROL_MASK, LOCK_MASK, SHIFT_MASK, SUPER_MASK};

use crate::constants::{
    HOOK_INPUT_TIMES_KEPT, KEYBOARD_STATE_ARRAY_SIZE, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE,
    KEYEVENTF_UNICODE, KEYSWITCH_EXTRA_INFO, KEYSWITCH_FOREIGN_REPLAY_INFO, KEYSWITCH_REPLAY_INFO, LLKHF_EXTENDED,
    LLKHF_INJECTED, VIRTUAL_KEY_BYTE_MASK, VK_CAPITAL, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_MENU, VK_RCONTROL,
    VK_RMENU, VK_RSHIFT, VK_SHIFT, VK_STATE_DOWN_BIT, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};
use crate::windows_backend::{NativeInput, NativeKeyEvent};

/// The time of every key and pointer event the hooks got, newest last (`hook_input_times`).
///
/// The hook's thread is the only writer and must never wait, so there is no lock: a copy on the engine's thread can run
/// while a key comes. `next` is published after its slot, so every slot below it holds a time. A slot the copy reaches
/// late may already hold a newer time than the one it replaced; that only touches the oldest entries, which would have
/// dropped out of the next copy anyway, and `take_unseen_input` asks of them nothing but whether a time is among them.
/// The newest entry of a copy is overwritten only after `HOOK_INPUT_TIMES_KEPT` more events during one copy.
pub struct InputTimes {
    slots: [AtomicU32; HOOK_INPUT_TIMES_KEPT],
    next: AtomicU64,
}

impl InputTimes {
    #[must_use]
    pub const fn new() -> Self {
        InputTimes { slots: [const { AtomicU32::new(0) }; HOOK_INPUT_TIMES_KEPT], next: AtomicU64::new(0) }
    }

    /// One writer only: the thread of the hooks.
    pub fn append(&self, time: u32) {
        let next = self.next.load(Ordering::Relaxed);
        if let Some(slot) = self.slots.get(next.to::<usize>() % HOOK_INPUT_TIMES_KEPT) {
            slot.store(time, Ordering::Relaxed);
        }
        self.next.store(next.wrapping_add(1), Ordering::Release);
    }

    pub fn copy(&self) -> Vec<i64> {
        let end = self.next.load(Ordering::Acquire);
        let count = end.min(HOOK_INPUT_TIMES_KEPT.to::<u64>());
        (end - count..end)
            .filter_map(|index| self.slots.get(index.to::<usize>() % HOOK_INPUT_TIMES_KEPT))
            .map(|slot| i64::from(slot.load(Ordering::Relaxed)))
            .collect()
    }
}

impl Default for InputTimes {
    fn default() -> Self {
        Self::new()
    }
}

/// The key event of a message a keyboard hook got, or None for a message that is no key.
#[must_use]
pub fn native_key_event(
    message: u32,
    vk_code: u32,
    scan_code: u32,
    flags: u32,
    time: u32,
    extra: usize,
) -> Option<NativeKeyEvent> {
    if !matches!(message, WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP) {
        return None;
    }
    let injected = flags & LLKHF_INJECTED != 0;
    Some(NativeKeyEvent {
        pressed: matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN),
        virtual_key: vk_code,
        scan_code,
        extended: flags & LLKHF_EXTENDED != 0,
        injected: injected && extra == KEYSWITCH_EXTRA_INFO,
        timestamp: i64::from(time),
        replayed: injected && (extra == KEYSWITCH_REPLAY_INFO || extra == KEYSWITCH_FOREIGN_REPLAY_INFO),
        // Any other injected key comes from another program: remote control, a macro, an on-screen keyboard.
        foreign: injected && extra != KEYSWITCH_EXTRA_INFO && extra != KEYSWITCH_REPLAY_INFO,
        wheel: false,
    })
}

/// The fields of the KEYBDINPUT that sends `item`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyboardFields {
    pub virtual_key: u16,
    pub scan_code: u16,
    pub flags: u32,
    pub extra: usize,
}

#[must_use]
pub fn keyboard_fields(item: &NativeInput) -> KeyboardFields {
    let mut flags = 0;
    if !item.pressed {
        flags |= KEYEVENTF_KEYUP;
    }
    if item.unicode {
        // The character itself travels in wScan; there is no key position.
        flags |= KEYEVENTF_UNICODE;
    } else {
        if item.extended {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        if item.scan_code != 0 {
            flags |= KEYEVENTF_SCANCODE;
        }
    }
    let extra = if item.synthetic {
        KEYSWITCH_EXTRA_INFO
    } else if item.replayed {
        if item.foreign { KEYSWITCH_FOREIGN_REPLAY_INFO } else { KEYSWITCH_REPLAY_INFO }
    } else {
        0
    };
    KeyboardFields {
        virtual_key: if item.unicode { 0 } else { item.virtual_key.low::<u16>() },
        scan_code: item.scan_code.low::<u16>(),
        flags,
        extra,
    }
}

/// The keyboard state `ToUnicodeEx` translates `virtual_key` in, or None when `state` holds Win, which types nothing.
#[must_use]
pub fn keyboard_state(virtual_key: u32, state: u32) -> Option<[u8; KEYBOARD_STATE_ARRAY_SIZE]> {
    let mut keys = [0u8; KEYBOARD_STATE_ARRAY_SIZE];
    let mut press = |pressed: [u32; 3]| {
        for key in pressed {
            if let Some(entry) = keys.get_mut(key.to::<usize>()) {
                *entry = VK_STATE_DOWN_BIT;
            }
        }
    };
    if state & SHIFT_MASK != 0 {
        press([VK_SHIFT, VK_LSHIFT, VK_RSHIFT]);
    }
    if state & CONTROL_MASK != 0 {
        press([VK_CONTROL, VK_LCONTROL, VK_RCONTROL]);
    }
    if state & ALT_MASK != 0 {
        press([VK_MENU, VK_LMENU, VK_RMENU]);
    }
    if state & LOCK_MASK != 0
        && let Some(entry) = keys.get_mut(VK_CAPITAL.to::<usize>())
    {
        *entry = 0x01;
    }
    if state & SUPER_MASK != 0 {
        return None;
    }
    if let Some(entry) = keys.get_mut((virtual_key & VIRTUAL_KEY_BYTE_MASK).to::<usize>()) {
        *entry |= VK_STATE_DOWN_BIT;
    }
    Some(keys)
}

/// `bytes.decode("utf-16-le", "replace").split("\0", 1)[0]`.
#[must_use]
pub fn clipboard_bytes_to_text(data: &[u8]) -> String {
    let (pairs, odd_byte) = data.as_chunks::<2>();
    let units = pairs.iter().map(|pair| u16::from_le_bytes(*pair));
    let mut text: String =
        char::decode_utf16(units).map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER)).collect();
    if !odd_byte.is_empty() {
        text.push(char::REPLACEMENT_CHARACTER);
    }
    match text.find('\0') {
        Some(end) => {
            text.truncate(end);
            text
        }
        None => text,
    }
}

#[cfg(windows)]
pub use win32::NativeWindowsAPI;
#[cfg(windows)]
pub(crate) use win32::wide;

#[cfg(windows)]
mod win32 {
    use ks_cast::To as _;

    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};
    use std::ptr::{null, null_mut};
    use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};

    use ks_backend::ScreenAnchor;
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, GlobalFree, HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, POINT, WPARAM,
    };
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardSequenceNumber, OpenClipboard,
        RegisterClipboardFormatW, SetClipboardData,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
    use windows_sys::Win32::System::SystemInformation::GetTickCount;
    use windows_sys::Win32::System::Threading::{
        AttachThreadInput, GetCurrentProcessId, GetCurrentThread, GetCurrentThreadId, OpenProcess,
        QueryFullProcessImageNameW, SetThreadPriority,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, GetKeyState, GetKeyboardLayout, GetKeyboardLayoutList, GetLastInputInfo, INPUT, INPUT_0, KEYBDINPUT,
        LASTINPUTINFO, SendInput, ToUnicodeEx,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, CallNextHookEx, CreateWindowExW, DestroyWindow, DispatchMessageW, GUITHREADINFO, GetAncestor,
        GetClassNameW, GetCursorPos, GetForegroundWindow, GetGUIThreadInfo, GetMessageW, GetWindowLongW,
        GetWindowThreadProcessId, HHOOK, KBDLLHOOKSTRUCT, MSG, MSLLHOOKSTRUCT, PeekMessageW, PostMessageW, PostThreadMessageW,
        SetForegroundWindow, SetWindowLongW, SetWindowPos, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx,
    };

    use super::{InputTimes, clipboard_bytes_to_text, keyboard_fields, keyboard_state, native_key_event};
    use crate::constants::{
        ASYNC_KEY_DOWN_BIT, CF_PRIVATEFIRST, CF_PRIVATELAST, CF_UNICODETEXT, CLIPBOARD_DIB_FORMATS,
        CLIPBOARD_FORMATS_SYNTHESIZED_FROM_DIB, CLIPBOARD_HANDLE_FORMATS, CLIPBOARD_HISTORY_FORMAT_NAME, GA_ROOT, GMEM_MOVEABLE,
        GWL_EXSTYLE, HC_ACTION, INPUT_KEYBOARD, PM_NOREMOVE, POINTER_INVALIDATING_MESSAGES, WHEEL_MESSAGES, PROCESS_IMAGE_NAME_BUFFER_CHARACTERS,
        PROCESS_QUERY_LIMITED_INFORMATION, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
        THREAD_PRIORITY_ABOVE_NORMAL, TO_UNICODE_KEEP_KEYBOARD_STATE_FLAG, TRANSLATED_TEXT_BUFFER_CHARACTERS, UTF16_CODE_UNIT_BYTES,
        VK_CAPITAL, WH_KEYBOARD_LL, WH_MOUSE_LL, WINDOW_CLASS_NAME_MAX_CHARACTERS, WM_INPUTLANGCHANGEREQUEST, WM_QUIT, WM_USER,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    };
    use crate::windows_backend::{
        ClipboardContents, HookListener, HookReady, NativeInput, NativeKeyEvent, WindowsAPI, WindowsBackendError, WindowsSelectionAPI,
    };
    use crate::windows_system::{ntpath_basename, ntpath_splitext};

    /// See `InputTimes`: the hooks write it at each event, and it outlives a restart of the hook.
    static INPUT_TIMES: InputTimes = InputTimes::new();
    /// The listener of the running hook: a leaked box, set on the hook's thread before the first hook is installed and
    /// taken back on that thread after the last one is removed. The system calls the hook procedures on that same thread,
    /// so they can never see it freed; the atomic is what lets it be a safe static at all.
    static HOOK_LISTENER: AtomicPtr<HookListener> = AtomicPtr::new(null_mut());
    /// The thread of the running hook, 0 when none: `stop_keyboard_hook` posts to it from another thread.
    static HOOK_THREAD_ID: AtomicU32 = AtomicU32::new(0);

    /// A NUL-terminated wide string.
    pub(crate) fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn to_hwnd(window: i64) -> HWND {
        window.to::<isize>().pointer_mut::<c_void>()
    }

    fn from_handle(handle: *mut c_void) -> i64 {
        (handle.to::<isize>()).to::<i64>()
    }

    fn last_error_code() -> u32 {
        // SAFETY: GetLastError takes nothing and only reads the thread's last-error value.
        unsafe { GetLastError() }
    }

    fn error(message: &str) -> WindowsBackendError {
        WindowsBackendError::Failed(format!("{message} (Win32 error {})", last_error_code()))
    }

    fn thread_of(window: HWND) -> u32 {
        // SAFETY: a null process pointer is allowed; an invalid window handle is answered with 0.
        unsafe { GetWindowThreadProcessId(window, null_mut()) }
    }

    fn gui_thread_info(thread_id: u32) -> Option<GUITHREADINFO> {
        // SAFETY: GUITHREADINFO is plain data, valid when zeroed; cbSize is set before the call, and the system writes only
        // into this local.
        unsafe {
            let mut info: GUITHREADINFO = zeroed();
            info.cbSize = (size_of::<GUITHREADINFO>()).low::<u32>();
            (GetGUIThreadInfo(thread_id, &raw mut info) != 0).then_some(info)
        }
    }

    /// The first `limit` bytes of a clipboard memory block, or None if it cannot be locked.
    fn global_bytes(handle: HGLOBAL, limit: usize) -> Option<Vec<u8>> {
        // SAFETY: `handle` came from GetClipboardData for formats that are memory blocks, and the clipboard is open, so
        // the block stays valid. The copy stays within GlobalSize bytes while the block is locked.
        unsafe {
            let size = GlobalSize(handle).min(limit);
            let pointer = GlobalLock(handle);
            if pointer.is_null() {
                return None;
            }
            let data = std::slice::from_raw_parts(pointer.cast::<u8>(), size).to_vec();
            GlobalUnlock(handle);
            Some(data)
        }
    }

    /// The window typed keys go to, and its thread, which owns the layout they are read in.
    ///
    /// Windows keeps a keyboard layout per thread. The text control of Windows 11 Notepad lives in a thread of its own:
    /// that thread's layout is the one the user types with and switches, while the thread of the window in front keeps its
    /// old layout. When the focused control shares the thread of the window in front, as in nearly every program, the
    /// window in front is asked, as before.
    fn input_window() -> (HWND, u32) {
        // SAFETY: GetForegroundWindow takes nothing and returns a handle or null.
        let window = unsafe { GetForegroundWindow() };
        if window.is_null() {
            return (null_mut(), 0);
        }
        let thread_id = thread_of(window);
        let focus = gui_thread_info(0).map_or(null_mut(), |info| info.hwndFocus);
        if !focus.is_null() && focus != window {
            let focus_thread = thread_of(focus);
            if focus_thread != 0 && focus_thread != thread_id {
                return (focus, focus_thread);
            }
        }
        (window, thread_id)
    }

    /// The thread whose layout the keys of `window` are read in, as `input_window` finds it for the window in front:
    /// the thread of its focused control when that is another one.
    fn input_thread(window: HWND) -> u32 {
        let thread_id = thread_of(window);
        let focus = if thread_id == 0 { null_mut() } else { gui_thread_info(thread_id).map_or(null_mut(), |info| info.hwndFocus) };
        if !focus.is_null() && focus != window {
            let focus_thread = thread_of(focus);
            if focus_thread != 0 && focus_thread != thread_id {
                return focus_thread;
            }
        }
        thread_id
    }

    fn append_time(time: u32) {
        INPUT_TIMES.append(time);
    }

    /// Calls the listener of the running hook.
    fn call_listener(event: NativeKeyEvent) -> bool {
        let listener = HOOK_LISTENER.load(Ordering::Acquire);
        if listener.is_null() {
            return false;
        }
        // A panic cannot unwind into the system's call of the hook: it would end the program. It ends here instead, and the
        // key passes on to the window, as a key did when the Python's callback raised.
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // SAFETY: set before the hooks are installed and freed only after they are removed, on this thread, which is
            // the thread the system calls the hook procedures on (see HOOK_LISTENER).
            unsafe { (*listener)(event) }
        }))
        .unwrap_or(false)
    }

    /// The low-level mouse hook: observe only, never suppress the user's pointer action.
    extern "system" fn mouse_callback(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
        if code == HC_ACTION {
            // SAFETY: for HC_ACTION the system passes a MSLLHOOKSTRUCT in lParam, valid for this call.
            let time = unsafe { (*data.pointer::<MSLLHOOKSTRUCT>()).time };
            append_time(time);
            // A button press invalidates the caret position; a scroll does not, it only ends a lone press of a modifier.
            if POINTER_INVALIDATING_MESSAGES.contains(&message.low::<u32>()) {
                call_listener(NativeKeyEvent::new(true, 0, 0, false, false, 0));
            } else if WHEEL_MESSAGES.contains(&message.low::<u32>()) {
                call_listener(NativeKeyEvent { wheel: true, ..NativeKeyEvent::new(false, 0, 0, false, false, i64::from(time)) });
            }
        }
        // SAFETY: the arguments are the ones the system passed in, and the hook handle is ignored by CallNextHookEx.
        unsafe { CallNextHookEx(null_mut(), code, message, data) }
    }

    /// The low-level keyboard hook.
    extern "system" fn keyboard_callback(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
        if code == HC_ACTION {
            // SAFETY: for HC_ACTION the system passes a KBDLLHOOKSTRUCT in lParam, valid for this call.
            let native = unsafe { &*data.pointer::<KBDLLHOOKSTRUCT>() };
            if let Some(event) =
                native_key_event(message.low::<u32>(), native.vkCode, native.scanCode, native.flags, native.time, native.dwExtraInfo)
            {
                append_time(native.time);
                if call_listener(event) {
                    // A non-zero result keeps the key out of the hook chain and away from the focused window: the key
                    // answered KeySwitch itself and must not also reach the text being typed.
                    return 1;
                }
            }
        }
        // SAFETY: the arguments are the ones the system passed in, and the hook handle is ignored by CallNextHookEx.
        unsafe { CallNextHookEx(null_mut(), code, message, data) }
    }

    /// Removes both hooks and the listener, whichever way the message loop ended.
    fn hook_cleanup(mouse_hook: HHOOK, keyboard_hook: HHOOK) {
        // SAFETY: both handles came from SetWindowsHookExW on this thread and are removed once, the mouse hook first.
        unsafe {
            UnhookWindowsHookEx(mouse_hook);
            UnhookWindowsHookEx(keyboard_hook);
        }
        HOOK_THREAD_ID.store(0, Ordering::Release);
        free_listener();
    }

    /// Takes the listener back. Only with no hook installed: nothing can be running it then.
    fn free_listener() {
        let listener = HOOK_LISTENER.swap(null_mut(), Ordering::AcqRel);
        if !listener.is_null() {
            // SAFETY: the pointer came from Box::into_raw in run_keyboard_hook, and the swap above took the only copy.
            drop(unsafe { Box::from_raw(listener) });
        }
    }

    /// Small owner of the User32/Kernel32 calls required by `KeySwitch`.
    pub struct NativeWindowsAPI;

    impl NativeWindowsAPI {
        #[must_use]
        pub fn new() -> Self {
            NativeWindowsAPI
        }
    }

    impl Default for NativeWindowsAPI {
        fn default() -> Self {
            Self::new()
        }
    }

    impl WindowsAPI for NativeWindowsAPI {
        fn loaded_layouts(&self) -> Result<Vec<i64>, WindowsBackendError> {
            // SAFETY: a zero count with a null list asks for the number of layouts and writes nothing.
            let count = unsafe { GetKeyboardLayoutList(0, null_mut()) };
            if count <= 0 {
                return Err(error("Windows не вернула список раскладок"));
            }
            let mut values = vec![null_mut::<c_void>(); count.to::<usize>()];
            // SAFETY: the list has room for `count` handles, the number passed.
            let actual = unsafe { GetKeyboardLayoutList(count, values.as_mut_ptr()) };
            if actual <= 0 {
                return Err(error("Windows не заполнила список раскладок"));
            }
            values.truncate(actual.to::<usize>());
            Ok(values.into_iter().map(from_handle).collect())
        }

        fn foreground_layout(&self) -> i64 {
            let (_window, thread_id) = input_window();
            if thread_id == 0 {
                return 0;
            }
            // SAFETY: GetKeyboardLayout takes a thread id by value and returns a handle or null.
            from_handle(unsafe { GetKeyboardLayout(thread_id) })
        }

        fn window_layout(&self, window: i64) -> i64 {
            let thread_id = input_thread(to_hwnd(window));
            if thread_id == 0 {
                return 0;
            }
            // SAFETY: GetKeyboardLayout takes a thread id by value and returns a handle or null.
            from_handle(unsafe { GetKeyboardLayout(thread_id) })
        }

        fn request_layout(&self, layout: i64) -> bool {
            // The request goes to the thread whose layout foreground_layout reads.
            let (window, _thread_id) = input_window();
            // SAFETY: PostMessageW takes its arguments by value; a window that is gone makes it fail.
            !window.is_null() && unsafe { PostMessageW(window, WM_INPUTLANGCHANGEREQUEST, 0, (layout.to::<isize>()).to::<LPARAM>()) } != 0
        }

        fn translate_key(&self, virtual_key: u32, scan_code: u32, state: u32, layout: i64) -> String {
            let Some(keys) = keyboard_state(virtual_key, state) else {
                return String::new();
            };
            let mut buffer = [0u16; TRANSLATED_TEXT_BUFFER_CHARACTERS];
            // SAFETY: `keys` is the 256 bytes the call reads, and `buffer` has room for the count passed.
            let written = unsafe {
                ToUnicodeEx(
                    virtual_key,
                    scan_code,
                    keys.as_ptr(),
                    buffer.as_mut_ptr(),
                    buffer.len().low::<i32>(),
                    TO_UNICODE_KEEP_KEYBOARD_STATE_FLAG,
                    layout.to::<isize>().pointer_mut::<c_void>(),
                )
            };
            if written <= 0 {
                return String::new();
            }
            // Up to the first zero, as a wide buffer's text, and no further than the count.
            let written = buffer.iter().take(written.to::<usize>()).take_while(|&&unit| unit != 0).count();
            let Some(units) = buffer.get(..written) else {
                return String::new();
            };
            // A lone surrogate is not printable either.
            match String::from_utf16(units) {
                Ok(result) if ks_text::pystr::isprintable(&result) => result,
                _ => String::new(),
            }
        }

        fn send_inputs(&self, inputs: &[NativeInput]) -> Result<usize, WindowsBackendError> {
            if inputs.is_empty() {
                return Ok(0);
            }
            let native: Vec<INPUT> = inputs
                .iter()
                .map(|item| {
                    let fields = keyboard_fields(item);
                    INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: INPUT_0 {
                            ki: KEYBDINPUT {
                                wVk: fields.virtual_key,
                                wScan: fields.scan_code,
                                dwFlags: fields.flags,
                                time: 0,
                                dwExtraInfo: fields.extra,
                            },
                        },
                    }
                })
                .collect();
            // SAFETY: the array holds the number of INPUT structures passed, of the size passed.
            let sent = unsafe { SendInput(native.len().low::<u32>(), native.as_ptr(), (size_of::<INPUT>()).low::<i32>()) };
            Ok(sent.to::<usize>())
        }

        fn active_application(&self) -> String {
            // SAFETY: GetForegroundWindow takes nothing and returns a handle or null.
            let window = unsafe { GetForegroundWindow() };
            if window.is_null() {
                return String::new();
            }
            let mut process_id = 0u32;
            // SAFETY: `process_id` is a valid place for the id; a stale window makes the call answer 0.
            if unsafe { GetWindowThreadProcessId(window, &raw mut process_id) } == 0 {
                return String::new();
            }
            // SAFETY: OpenProcess takes its arguments by value and returns a handle or null, closed below.
            let process: HANDLE = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
            if process.is_null() {
                return String::new();
            }
            let mut size = PROCESS_IMAGE_NAME_BUFFER_CHARACTERS.low::<u32>();
            let mut buffer = vec![0u16; PROCESS_IMAGE_NAME_BUFFER_CHARACTERS];
            // SAFETY: `size` says how many characters `buffer` holds; `process` is the open handle above.
            let queried = unsafe { QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &raw mut size) };
            // SAFETY: `process` is open and closed once.
            unsafe { CloseHandle(process) };
            if queried == 0 {
                return String::new();
            }
            buffer.truncate(size.to::<usize>());
            let path = String::from_utf16_lossy(&buffer);
            ntpath_splitext(ntpath_basename(&path)).0.to_owned()
        }

        fn foreground_window(&self) -> i64 {
            // SAFETY: GetForegroundWindow takes nothing and returns a handle or null.
            from_handle(unsafe { GetForegroundWindow() })
        }

        fn focused_control(&self) -> i64 {
            gui_thread_info(0).map_or(0, |info| from_handle(info.hwndFocus))
        }

        fn window_process_id(&self, window: i64) -> i64 {
            if window == 0 {
                return 0;
            }
            let mut process_id = 0u32;
            // SAFETY: `process_id` is a valid place for the id; a stale window makes the call answer 0.
            if unsafe { GetWindowThreadProcessId(to_hwnd(window), &raw mut process_id) } == 0 {
                return 0;
            }
            i64::from(process_id)
        }

        fn current_process_id(&self) -> i64 {
            // SAFETY: GetCurrentProcessId takes nothing.
            i64::from(unsafe { GetCurrentProcessId() })
        }

        fn input_anchor(&self) -> Option<ScreenAnchor> {
            // SAFETY: GetForegroundWindow takes nothing and returns a handle or null.
            let window = unsafe { GetForegroundWindow() };
            let anchor_window = (!window.is_null()).then(|| from_handle(window));
            let thread_id = if window.is_null() { 0 } else { thread_of(window) };
            if thread_id != 0
                && let Some(info) = gui_thread_info(thread_id)
                && !info.hwndCaret.is_null()
            {
                let mut point = POINT { x: info.rcCaret.left, y: info.rcCaret.bottom };
                // SAFETY: `point` is a valid POINT; a stale window makes the call fail.
                if unsafe { ClientToScreen(info.hwndCaret, &raw mut point) } != 0 {
                    return Some(ScreenAnchor { x: point.x, y: point.y, window: anchor_window });
                }
            }
            let mut point = POINT { x: 0, y: 0 };
            // SAFETY: `point` is a valid POINT.
            if unsafe { GetCursorPos(&raw mut point) } != 0 {
                return Some(ScreenAnchor { x: point.x, y: point.y, window: anchor_window });
            }
            None
        }

        /// Bring a window to the foreground without changing its state.
        ///
        /// Windows lets only the process in front move the foreground. The rule window opens from a hotkey while another
        /// program is in front, and would only flash in the taskbar; attaching to the input of the thread in front for the
        /// moment of the switch lifts that restriction.
        fn activate_window(&self, window: i64) -> bool {
            let handle = to_hwnd(window);
            // SAFETY: each of these takes handles and ids by value and fails on a stale window. The attachment is undone
            // below with the same two threads.
            unsafe {
                let ancestor = GetAncestor(handle, GA_ROOT);
                let root = if ancestor.is_null() { handle } else { ancestor };
                let foreground = GetForegroundWindow();
                let current = GetCurrentThreadId();
                let other = if foreground.is_null() { 0 } else { thread_of(foreground) };
                let attached = other != 0 && other != current && AttachThreadInput(current, other, 1) != 0;
                BringWindowToTop(root);
                let activated = SetForegroundWindow(root) != 0;
                if attached {
                    AttachThreadInput(current, other, 0);
                }
                activated
            }
        }

        /// Mark a popup so that showing or clicking it never takes the foreground.
        ///
        /// `WS_EX_NOACTIVATE` keeps the caret (and the per-window keyboard layout) in the editor while the learning prompt
        /// is visible.
        fn keep_window_inactive(&self, window: i64) -> bool {
            if window == 0 {
                return false;
            }
            // SAFETY: GetAncestor takes a handle by value and fails on a stale window.
            let ancestor = unsafe { GetAncestor(to_hwnd(window), GA_ROOT) };
            let root = if ancestor.is_null() { window } else { from_handle(ancestor) };
            let mut targets = vec![window, root];
            targets.sort_unstable();
            targets.dedup();
            for target in targets {
                let pointer = to_hwnd(target);
                // SAFETY: these take handles and numbers by value and fail on a stale window.
                unsafe {
                    let style = GetWindowLongW(pointer, GWL_EXSTYLE);
                    let wanted = style | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW;
                    if wanted != style {
                        SetWindowLongW(pointer, GWL_EXSTYLE, wanted);
                    }
                    SetWindowPos(
                        pointer,
                        null_mut(),
                        0,
                        0,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                    );
                }
            }
            true
        }

        fn caps_lock_enabled(&self) -> bool {
            // SAFETY: GetKeyState takes a key number by value.
            i32::from(unsafe { GetKeyState(VK_CAPITAL.bits::<i32>()) }) & 1 != 0
        }

        fn key_down(&self, virtual_key: u32) -> bool {
            // SAFETY: GetAsyncKeyState takes a key number by value.
            i32::from(unsafe { GetAsyncKeyState(virtual_key.bits::<i32>()) }) & ASYNC_KEY_DOWN_BIT != 0
        }

        fn tick_count(&self) -> i64 {
            // SAFETY: GetTickCount takes nothing.
            i64::from(unsafe { GetTickCount() })
        }

        fn last_input_time(&self) -> Result<i64, WindowsBackendError> {
            let mut info = LASTINPUTINFO { cbSize: (size_of::<LASTINPUTINFO>()).low::<u32>(), dwTime: 0 };
            // SAFETY: `info` is valid and its cbSize is set.
            if unsafe { GetLastInputInfo(&raw mut info) } == 0 {
                return Err(error("GetLastInputInfo завершился ошибкой"));
            }
            Ok(i64::from(info.dwTime))
        }

        fn hook_input_times(&self) -> Vec<i64> {
            INPUT_TIMES.copy()
        }

        fn run_keyboard_hook(&self, listener: HookListener, ready: HookReady) -> Result<(), WindowsBackendError> {
            // Every key of the system waits for the callback's answer. Microsoft documents that a hook answering too late
            // is removed without a word; Windows 11 build 26200 waited 11 s and kept it. A thread of normal priority waits
            // behind every busy thread of the same priority.
            // SAFETY: GetCurrentThread returns a pseudo handle that is always valid for this thread.
            unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL) };
            let boxed = Box::into_raw(Box::new(listener));
            if HOOK_LISTENER.compare_exchange(null_mut(), boxed, Ordering::AcqRel, Ordering::Acquire).is_err() {
                // SAFETY: `boxed` came from Box::into_raw above and was not published.
                drop(unsafe { Box::from_raw(boxed) });
                return Err(WindowsBackendError::Failed("Перехватчик клавиатуры уже запущен".to_owned()));
            }
            // PostThreadMessageW fails when the target thread has no message queue. Create it before publishing readiness
            // so even an immediate stop is guaranteed to deliver WM_QUIT.
            // SAFETY: MSG is plain data, valid when zeroed, and the call writes only into this local.
            unsafe {
                let mut queue_message: MSG = zeroed();
                PeekMessageW(&raw mut queue_message, null_mut(), WM_USER, WM_USER, PM_NOREMOVE);
            }
            // SAFETY: a null name asks for the handle of this process's own executable.
            let module = unsafe { GetModuleHandleW(null()) };
            // SAFETY: the procedure has the signature of HOOKPROC and is never unloaded; the listener it reads is set.
            let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_callback), module, 0) };
            if hook.is_null() {
                let failure = error("Не удалось установить WH_KEYBOARD_LL");
                free_listener();
                return Err(failure);
            }
            // SAFETY: as for the keyboard hook above.
            let mouse_hook = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_callback), module, 0) };
            if mouse_hook.is_null() {
                let failure = error("Не удалось установить WH_MOUSE_LL");
                // SAFETY: `hook` came from SetWindowsHookExW above and is removed once.
                unsafe { UnhookWindowsHookEx(hook) };
                free_listener();
                return Err(failure);
            }
            // SAFETY: GetCurrentThreadId takes nothing.
            HOOK_THREAD_ID.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
            // The moment the hooks start counts as an event they got: input made before it was never theirs to get.
            // SAFETY: GetTickCount takes nothing.
            INPUT_TIMES.append(unsafe { GetTickCount() });
            ready();
            // SAFETY: MSG is plain data, valid when zeroed.
            let mut message: MSG = unsafe { zeroed() };
            let result = loop {
                // SAFETY: `message` is a valid MSG this thread owns; the loop is the one that serves the hooks.
                let got = unsafe { GetMessageW(&raw mut message, null_mut(), 0, 0) };
                if got == 0 {
                    break Ok(());
                }
                if got < 0 {
                    break Err(error("GetMessageW завершился ошибкой"));
                }
                // SAFETY: `message` was just filled by GetMessageW.
                unsafe {
                    TranslateMessage(&raw const message);
                    DispatchMessageW(&raw const message);
                }
            };
            hook_cleanup(mouse_hook, hook);
            result
        }

        fn stop_keyboard_hook(&self) {
            let thread_id = HOOK_THREAD_ID.load(Ordering::Acquire);
            if thread_id != 0 {
                // SAFETY: PostThreadMessageW takes its arguments by value; a thread that has ended makes it fail.
                unsafe { PostThreadMessageW(thread_id, WM_QUIT, 0, 0) };
            }
        }

        fn selection_api(&self) -> Option<&dyn WindowsSelectionAPI> {
            Some(self)
        }
    }

    impl WindowsSelectionAPI for NativeWindowsAPI {
        fn window_class(&self, window: i64) -> String {
            if window == 0 {
                return String::new();
            }
            let mut buffer = [0u16; WINDOW_CLASS_NAME_MAX_CHARACTERS + 1];
            // SAFETY: `buffer` has room for the count passed.
            let written = unsafe { GetClassNameW(to_hwnd(window), buffer.as_mut_ptr(), buffer.len().low::<i32>()) };
            let written = usize::try_from(written).unwrap_or(0).min(buffer.len());
            buffer.get(..written).map_or_else(String::new, String::from_utf16_lossy)
        }

        fn text_caret(&self, window: i64) -> bool {
            let thread_id = if window == 0 { 0 } else { thread_of(to_hwnd(window)) };
            if thread_id == 0 {
                return false;
            }
            gui_thread_info(thread_id)
                .is_some_and(|info| !info.hwndCaret.is_null() && info.rcCaret.bottom > info.rcCaret.top)
        }

        fn clipboard_sequence(&self) -> i64 {
            // SAFETY: GetClipboardSequenceNumber takes nothing.
            i64::from(unsafe { GetClipboardSequenceNumber() })
        }

        /// Every format on the clipboard as bytes; incomplete when one of them cannot be kept that way.
        ///
        /// Opened with no window: reading needs none, and only a window that empties the clipboard becomes its owner.
        fn read_clipboard(&self, limit_bytes: usize) -> Option<ClipboardContents> {
            // SAFETY: a null owner is allowed for reading.
            if unsafe { OpenClipboard(null_mut()) } == 0 {
                return None;
            }
            let contents = read_opened_clipboard(limit_bytes);
            // SAFETY: this thread opened the clipboard above.
            unsafe { CloseClipboard() };
            Some(contents)
        }

        fn clipboard_text(&self, limit_characters: usize) -> Option<String> {
            // SAFETY: a null owner is allowed for reading.
            if unsafe { OpenClipboard(null_mut()) } == 0 {
                return None;
            }
            // SAFETY: the clipboard is open on this thread; the handle is checked before use.
            let handle = unsafe { GetClipboardData(CF_UNICODETEXT) };
            // One unit more than the characters asked for, for the terminating zero.
            let limit = UTF16_CODE_UNIT_BYTES * (limit_characters + 1);
            let text = if handle.is_null() {
                String::new()
            } else {
                global_bytes(handle, limit).map_or_else(String::new, |data| clipboard_bytes_to_text(&data))
            };
            // SAFETY: this thread opened the clipboard above.
            unsafe { CloseClipboard() };
            Some(text)
        }

        /// Put `contents` back as the clipboard's only data, kept out of the clipboard history.
        ///
        /// `SetClipboardData` fails for a clipboard opened without a window ("If an application calls `OpenClipboard` with hwnd
        /// set to NULL, `EmptyClipboard` sets the clipboard owner to NULL; this causes `SetClipboardData` to fail",
        /// `OpenClipboard` on Microsoft Learn), so a hidden window of this thread owns it for the call. The window is
        /// destroyed at once: it would not answer the messages a clipboard owner is sent, and nothing here is rendered
        /// late, so no owner is needed afterwards.
        fn write_clipboard(&self, contents: &ClipboardContents) -> Result<bool, WindowsBackendError> {
            let class = wide("STATIC");
            // SAFETY: `class` is a NUL-terminated name of a system class; the other pointers may be null.
            let window = unsafe {
                CreateWindowExW(0, class.as_ptr(), null(), 0, 0, 0, 0, 0, null_mut(), null_mut(), null_mut(), null())
            };
            if window.is_null() {
                return Err(error("Не удалось создать окно для буфера обмена"));
            }
            // SAFETY: `window` belongs to this thread.
            let opened = unsafe { OpenClipboard(window) } != 0;
            if opened {
                write_opened_clipboard(contents);
                // SAFETY: this thread opened the clipboard above.
                unsafe { CloseClipboard() };
            }
            // SAFETY: `window` belongs to this thread and is destroyed once.
            unsafe { DestroyWindow(window) };
            Ok(opened)
        }
    }

    fn read_opened_clipboard(limit_bytes: usize) -> ClipboardContents {
        let mut present = Vec::new();
        // SAFETY: the clipboard is open on this thread; the call takes a format number by value.
        let mut item = unsafe { EnumClipboardFormats(0) };
        while item != 0 {
            present.push(item);
            // SAFETY: as above.
            item = unsafe { EnumClipboardFormats(item) };
        }
        let with_dib = present.iter().any(|item| CLIPBOARD_DIB_FORMATS.contains(item));
        let mut kept = Vec::new();
        let mut total = 0usize;
        for item in present {
            if with_dib && CLIPBOARD_FORMATS_SYNTHESIZED_FROM_DIB.contains(&item) {
                continue;
            }
            if CLIPBOARD_HANDLE_FORMATS.contains(&item) || (CF_PRIVATEFIRST..=CF_PRIVATELAST).contains(&item) {
                return ClipboardContents { formats: kept, complete: false };
            }
            // SAFETY: the clipboard is open on this thread; handle formats were refused above, so what comes back is a
            // memory block GlobalSize can measure.
            let handle = unsafe { GetClipboardData(item) };
            // SAFETY: as above; the handle is checked first.
            let size = if handle.is_null() { 0 } else { unsafe { GlobalSize(handle) } };
            total = total.saturating_add(size);
            let data = if !handle.is_null() && total <= limit_bytes { global_bytes(handle, size) } else { None };
            let Some(data) = data else {
                return ClipboardContents { formats: kept, complete: false };
            };
            kept.push((item, data));
        }
        ClipboardContents { formats: kept, complete: true }
    }

    fn write_opened_clipboard(contents: &ClipboardContents) {
        let name = wide(CLIPBOARD_HISTORY_FORMAT_NAME);
        // SAFETY: the clipboard is open on this thread and owned by its window; `name` is NUL-terminated.
        let history = unsafe {
            EmptyClipboard();
            RegisterClipboardFormatW(name.as_ptr())
        };
        let marker = vec![0u8; size_of::<u32>()];
        let mut items: Vec<(u32, &[u8])> = contents.formats.iter().map(|(item, data)| (*item, data.as_slice())).collect();
        // An empty clipboard is put back empty, without even this marker.
        if !items.is_empty() && history != 0 && items.iter().all(|(item, _data)| *item != history) {
            items.push((history, &marker));
        }
        for (item, data) in items {
            // SAFETY: GMEM_MOVEABLE memory of at least one byte is what SetClipboardData takes; the copy stays inside the
            // block while it is locked. The system owns the block once it is set; one it refused is still ours to free.
            unsafe {
                let handle = GlobalAlloc(GMEM_MOVEABLE, data.len().max(1));
                if handle.is_null() {
                    continue;
                }
                let pointer = GlobalLock(handle);
                if !pointer.is_null() {
                    std::ptr::copy_nonoverlapping(data.as_ptr(), pointer.cast::<u8>(), data.len());
                    GlobalUnlock(handle);
                }
                if pointer.is_null() || SetClipboardData(item, handle).is_null() {
                    GlobalFree(handle);
                }
            }
        }
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::constants::{VK_BACK, VK_MENU_MASK, WM_LBUTTONDOWN};

    #[test]
    fn the_ring_keeps_the_newest_times_in_order() {
        let times = InputTimes::new();
        assert!(times.copy().is_empty());
        times.append(5);
        times.append(u32::MAX);
        assert_eq!(times.copy(), vec![5, i64::from(u32::MAX)]);
        for time in 0..HOOK_INPUT_TIMES_KEPT.low::<u32>() + 2 {
            times.append(time);
        }
        let copy = times.copy();
        assert_eq!(copy.len(), HOOK_INPUT_TIMES_KEPT);
        assert_eq!(copy.first(), Some(&2));
        assert_eq!(copy.last(), Some(&(HOOK_INPUT_TIMES_KEPT.to::<i64>() + 1)));
    }

    #[test]
    fn the_ring_can_be_copied_while_the_hook_appends() {
        let times = std::sync::Arc::new(InputTimes::new());
        let writer = {
            let times = std::sync::Arc::clone(&times);
            std::thread::spawn(move || {
                for time in 1..=200_000u32 {
                    times.append(time);
                }
            })
        };
        let mut newest = 0;
        while !writer.is_finished() {
            let copy = times.copy();
            assert!(copy.len() <= HOOK_INPUT_TIMES_KEPT);
            let last = copy.last().copied().unwrap_or(0);
            assert!(last >= newest, "the newest time went back: {last} after {newest}");
            newest = last;
        }
        writer.join().unwrap();
        assert_eq!(times.copy().last(), Some(&200_000));
    }

    #[test]
    fn a_keyboard_message_becomes_a_key_event() {
        assert_eq!(native_key_event(WM_LBUTTONDOWN, 1, 1, 0, 1, 0), None);
        let typed = native_key_event(WM_KEYDOWN, 0x41, 30, 0, 77, 0).unwrap();
        assert_eq!(typed, NativeKeyEvent::new(true, 0x41, 30, false, false, 77));
        let released = native_key_event(WM_SYSKEYUP, 0x41, 30, LLKHF_EXTENDED, 77, 0).unwrap();
        assert!(!released.pressed && released.extended);
        assert!(native_key_event(WM_SYSKEYDOWN, 0x41, 30, 0, 77, 0).unwrap().pressed);
        // A mark means nothing on a key the system did not call injected.
        let marked = native_key_event(WM_KEYDOWN, 0x41, 30, 0, 77, KEYSWITCH_EXTRA_INFO).unwrap();
        assert!(!marked.injected && !marked.replayed && !marked.foreign);
    }

    #[test]
    fn the_mark_of_an_injected_key_says_whose_it_is() {
        let injected = |extra| native_key_event(WM_KEYDOWN, 0x41, 30, LLKHF_INJECTED, 77, extra).unwrap();
        let own = injected(KEYSWITCH_EXTRA_INFO);
        assert!(own.injected && !own.replayed && !own.foreign);
        let replay = injected(KEYSWITCH_REPLAY_INFO);
        assert!(!replay.injected && replay.replayed && !replay.foreign);
        let foreign_replay = injected(KEYSWITCH_FOREIGN_REPLAY_INFO);
        assert!(!foreign_replay.injected && foreign_replay.replayed && foreign_replay.foreign);
        let foreign = injected(0);
        assert!(!foreign.injected && !foreign.replayed && foreign.foreign);
    }

    #[test]
    fn an_input_becomes_the_fields_of_keybdinput() {
        let fields = keyboard_fields(&NativeInput::key(true, VK_BACK));
        assert_eq!(
            fields,
            KeyboardFields { virtual_key: VK_BACK.low::<u16>(), scan_code: 0, flags: 0, extra: KEYSWITCH_EXTRA_INFO }
        );
        let up = keyboard_fields(&NativeInput::extended_key(false, VK_MENU_MASK));
        assert_eq!(up.flags, KEYEVENTF_KEYUP | KEYEVENTF_EXTENDEDKEY);
        let scan = keyboard_fields(&NativeInput::scan(true, 30));
        assert_eq!((scan.virtual_key, scan.scan_code, scan.flags), (0, 30, KEYEVENTF_SCANCODE));
        // The character travels in wScan; neither the key nor its flags go with it.
        let packet = NativeInput {
            virtual_key: 0x41,
            scan_code: 0x0430,
            extended: true,
            synthetic: false,
            replayed: true,
            unicode: true,
            foreign: true,
            ..NativeInput::new(false)
        };
        assert_eq!(
            keyboard_fields(&packet),
            KeyboardFields {
                virtual_key: 0,
                scan_code: 0x0430,
                flags: KEYEVENTF_KEYUP | KEYEVENTF_UNICODE,
                extra: KEYSWITCH_FOREIGN_REPLAY_INFO
            }
        );
        let replay = NativeInput { synthetic: false, replayed: true, ..NativeInput::new(true) };
        assert_eq!(keyboard_fields(&replay).extra, KEYSWITCH_REPLAY_INFO);
        let plain = NativeInput { synthetic: false, ..NativeInput::new(true) };
        assert_eq!(keyboard_fields(&plain).extra, 0);
    }

    #[test]
    fn the_keyboard_state_of_a_translation_holds_the_modifiers_and_the_key() {
        assert!(keyboard_state(0x41, SUPER_MASK).is_none());
        let plain = keyboard_state(0x41, 0).unwrap();
        assert_eq!(plain[0x41], VK_STATE_DOWN_BIT);
        assert_eq!(plain.iter().filter(|&&byte| byte != 0).count(), 1);
        let shifted = keyboard_state(0x41, SHIFT_MASK | LOCK_MASK).unwrap();
        for key in [VK_SHIFT, VK_LSHIFT, VK_RSHIFT] {
            assert_eq!(shifted[key.to::<usize>()], VK_STATE_DOWN_BIT);
        }
        assert_eq!(shifted[VK_CAPITAL.to::<usize>()], 0x01);
        // The key is Caps Lock itself, with Caps Lock on: down and locked.
        assert_eq!(keyboard_state(VK_CAPITAL, LOCK_MASK).unwrap()[VK_CAPITAL.to::<usize>()], 0x81);
        // A number past a byte is taken modulo 256, as the Python's mask does.
        assert_eq!(keyboard_state(0x141, 0).unwrap()[0x41], VK_STATE_DOWN_BIT);
    }

    #[test]
    fn clipboard_bytes_decode_as_the_python_decodes_them() {
        let bytes = |text: &str| text.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>();
        assert_eq!(clipboard_bytes_to_text(&bytes("привет\0хвост")), "привет");
        assert_eq!(clipboard_bytes_to_text(&bytes("")), "");
        // A pair cut in two by the limit, and an odd last byte, are replacement characters.
        let pair = bytes("😀");
        assert_eq!(clipboard_bytes_to_text(&pair[..2]), "\u{FFFD}");
        assert_eq!(clipboard_bytes_to_text(&[0x41, 0x00, 0x42]), "A\u{FFFD}");
    }
}
