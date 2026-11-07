//! The real thing on Windows: the low-level hook, then the installed program against an edit control of this process,
//! keys through `SendInput`, corrections through `SendInput` back. Runs with `KS_E2E_BINARY` naming the program; otherwise,
//! and on other systems, it says so and passes.

#![cfg_attr(windows, expect(unsafe_code, reason = "calls into Win32"))]
#[cfg(not(windows))]
fn main() {
    println!("the Windows end-to-end test runs on Windows only, skipped");
}

#[cfg(windows)]
fn main() {
    scenario::main();
}

#[cfg(windows)]
mod scenario {
    use ks_cast::To as _;

    use std::path::{Path, PathBuf};
    use std::process::{Child, Command};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use ks_config::config::SettingsStore;
    use ks_config::history::HistoryStore;
    use ks_config::learning::LearningStore;
    use ks_win::constants::{SETTINGS_WINDOW_CLASS, SHELL_TRAY_WINDOW_CLASSES, VK_BACK, VK_END, VK_ESCAPE, VK_HOME, VK_LSHIFT, VK_PAUSE, VK_RETURN, VK_RSHIFT, VK_SHIFT, WM_TRAY};
    use ks_win::windows_backend::{NativeInput, NativeKeyEvent, WindowsAPI};
    use ks_win::windows_native::NativeWindowsAPI;
    use serde_json::{Value, json};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardLayout, INPUT, INPUT_0, INPUT_MOUSE, LoadKeyboardLayoutW, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT, SendInput,
        SetFocus,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, FindWindowExW, GetMenuItemCount, GetMenuItemRect, GetMenuStringW, GetWindowThreadProcessId, HMENU,
        IsWindowVisible, MF_BYPOSITION, MN_GETHMENU, MenuItemFromPoint, PostMessageW, SPI_GETTHREADLOCALINPUTSETTINGS,
        SPI_SETTHREADLOCALINPUTSETTINGS, SPIF_SENDCHANGE, SendMessageW, SetCursorPos, SystemParametersInfoW, WM_RBUTTONUP,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, ES_AUTOVSCROLL, ES_MULTILINE, ES_WANTRETURN, GetClientRect, GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW, IsZoomed, MSG, MoveWindow, PM_REMOVE,
        PeekMessageW, RegisterClassW, SW_MAXIMIZE, SW_SHOW, ShowWindow, TranslateMessage, WM_SIZE, WNDCLASSW, WS_BORDER, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };

    const VK_F24: u32 = 0x87;
    const SCAN_G: u32 = 0x22;
    const SCAN_H: u32 = 0x23;
    const SCAN_B: u32 = 0x30;
    const SCAN_D: u32 = 0x20;
    const SCAN_T: u32 = 0x14;
    const SCAN_N: u32 = 0x31;
    const SCAN_E: u32 = 0x12;
    const SCAN_L: u32 = 0x26;
    const SCAN_O: u32 = 0x18;
    const SCAN_SPACE: u32 = 0x39;
    const SCAN_ENTER: u32 = 0x1C;
    const SCAN_LEFT_SHIFT: u32 = 0x2A;
    const SCAN_RIGHT_SHIFT: u32 = 0x36;
    const PRIVET: [u32; 6] = [SCAN_G, SCAN_H, SCAN_B, SCAN_D, SCAN_T, SCAN_N];
    const HELLO: [u32; 5] = [SCAN_H, SCAN_E, SCAN_L, SCAN_L, SCAN_O];
    const ENGLISH: u32 = 0x409;
    const RUSSIAN: u32 = 0x419;
    const STANDARD: u64 = 5_000;
    const LONG: u64 = 10_000;
    const PHASE_START: u64 = 100;
    const PHASE_SETTLE: u64 = 300;
    const WINDOW_OPENS: u64 = 1_000;
    const START_ATTEMPTS: u32 = 4;
    const PROBE_ATTEMPTS: u32 = 3;
    const SURFACE_CLOSES: u64 = 1_000;
    const PROGRAM_SETTLES: u64 = 1_500;
    /// The class of the popup menus of Windows (`#32768`, the menu class of About Window Classes on Microsoft Learn).
    const MENU_CLASS: &str = "#32768";
    /// The line of the tray menu that switches the layout begins so; its end names the layout it switches to.
    const SWITCH_LINE: &str = "Переключить на ";
    const MENU_LINE_CHARACTERS: usize = 128;

    /// The group of a keyboard layout by its language: 0 English, 1 Russian, -1 another.
    fn group_of_layout(layout: i64) -> i32 {
        match layout.bits::<u64>().low::<u32>() & 0xffff {
            ENGLISH => 0,
            RUSSIAN => 1,
            _ => -1,
        }
    }

    /// A key as a person's: no mark of this program on it.
    fn user(pressed: bool) -> NativeInput {
        NativeInput { synthetic: false, ..NativeInput::new(pressed) }
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain([0]).collect()
    }

    extern "system" fn window_procedure(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if message == WM_SIZE {
            let edit = EDIT.load(Ordering::SeqCst);
            if edit != 0 {
                // SAFETY: RECT is plain integers, and all zeros is a valid one.
                    let mut area = unsafe { std::mem::zeroed() };
                // SAFETY: the rectangle is ours and both windows belong to this thread.
                unsafe {
                    GetClientRect(window, &raw mut area);
                    MoveWindow(edit.pointer_mut::<std::ffi::c_void>(), 0, 0, area.right, area.bottom, 1);
                }
            }
        }
        // SAFETY: the arguments are the ones this window procedure was given.
        unsafe { DefWindowProcW(window, message, wparam, lparam) }
    }

    static EDIT: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

    /// The window typed into: a multi-line edit, so that an Enter that reaches it can be seen in its text.
    struct Editor {
        window: HWND,
        edit: HWND,
        api: Arc<NativeWindowsAPI>,
        layouts: (i64, i64),
    }

    impl Editor {
        fn new(api: Arc<NativeWindowsAPI>) -> Result<Editor, String> {
            let class = wide("KeySwitchE2E");
            let title = wide("KeySwitch E2E");
            let edit_class = wide("EDIT");
            let empty = wide("");
            // SAFETY: every pointer stays alive for its call; the handles returned are checked.
            unsafe {
                let instance = GetModuleHandleW(std::ptr::null());
                let mut description: WNDCLASSW = std::mem::zeroed();
                description.lpfnWndProc = Some(window_procedure);
                description.hInstance = instance;
                description.lpszClassName = class.as_ptr();
                RegisterClassW(&raw const description);
                let window = CreateWindowExW(0, class.as_ptr(), title.as_ptr(), WS_OVERLAPPEDWINDOW | WS_VISIBLE, 100, 100, 640, 240, std::ptr::null_mut(), std::ptr::null_mut(), instance, std::ptr::null());
                if window.is_null() {
                    return Err("the window was not created".to_owned());
                }
                let style = WS_CHILD | WS_VISIBLE | WS_BORDER | (ES_MULTILINE | ES_WANTRETURN | ES_AUTOVSCROLL).to::<u32>();
                let edit = CreateWindowExW(0, edit_class.as_ptr(), empty.as_ptr(), style, 0, 0, 620, 200, window, std::ptr::null_mut(), instance, std::ptr::null());
                if edit.is_null() {
                    return Err("the edit control was not created".to_owned());
                }
                EDIT.store(edit.to::<isize>(), Ordering::SeqCst);
                ShowWindow(window, SW_SHOW);
                let english = ((LoadKeyboardLayoutW(wide("00000409").as_ptr(), 0)).to::<isize>()).to::<i64>();
                let russian = ((LoadKeyboardLayoutW(wide("00000419").as_ptr(), 0)).to::<isize>()).to::<i64>();
                if english == 0 || russian == 0 {
                    return Err("the layouts could not be loaded".to_owned());
                }
                Ok(Editor { window, edit, api, layouts: (english, russian) })
            }
        }

        fn text(&self) -> String {
            // SAFETY: the buffer has room for the length asked of the control and its terminator.
            unsafe {
                let length = GetWindowTextLengthW(self.edit);
                let mut buffer = vec![0_u16; length.to::<usize>() + 2];
                let written = GetWindowTextW(self.edit, buffer.as_mut_ptr(), buffer.len().low::<i32>());
                String::from_utf16_lossy(&buffer[..written.max(0).to::<usize>()])
            }
        }

        fn group(&self) -> i32 {
            // SAFETY: the layout of this thread, which owns the edit control.
            group_of_layout(((unsafe { GetKeyboardLayout(0) }).to::<isize>()).to::<i64>())
        }

        /// Runs this thread's messages until the condition holds or the time is out.
        fn pump_until(&self, milliseconds: u64, mut condition: impl FnMut(&Editor) -> bool) -> bool {
            let deadline = Instant::now() + Duration::from_millis(milliseconds);
            loop {
                // SAFETY: MSG is plain data, and all zeros is a valid one.
        let mut message: MSG = unsafe { std::mem::zeroed() };
                // SAFETY: the message is ours for the length of the calls.
                unsafe {
                    while PeekMessageW(&raw mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                        TranslateMessage(&raw const message);
                        DispatchMessageW(&raw const message);
                    }
                }
                if condition(self) {
                    return true;
                }
                if Instant::now() >= deadline {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        fn pump(&self, milliseconds: u64) {
            self.pump_until(milliseconds, |_| false);
        }

        /// A system surface can hold the foreground of a fresh session (seen on the Arm runner: search, the web host):
        /// nothing typed reaches a window until Escape closes it.
        fn close_surface_over_this(&self) -> Result<(), String> {
            // SAFETY: takes nothing.
            if unsafe { GetForegroundWindow() } != self.window {
                self.virtual_key(VK_ESCAPE)?;
                self.pump(SURFACE_CLOSES);
            }
            Ok(())
        }

        fn activate(&self) {
            self.api.activate_window((self.window.to::<isize>()).to::<i64>());
            // SAFETY: the edit control belongs to this thread.
            unsafe {
                SetFocus(self.edit);
            }
        }

        fn maximize(&self) {
            // SAFETY: the window belongs to this thread.
            unsafe {
                ShowWindow(self.window, SW_MAXIMIZE);
            }
        }

        fn is_zoomed(&self) -> bool {
            // SAFETY: the window belongs to this thread.
            unsafe { IsZoomed(self.window) != 0 }
        }

        fn send(&self, inputs: &[NativeInput]) -> Result<(), String> {
            match self.api.send_inputs(inputs) {
                Ok(count) if count == inputs.len() => Ok(()),
                other => Err(format!("SendInput took {other:?} of {} events", inputs.len())),
            }
        }

        fn scans(&self, codes: &[u32]) -> Result<(), String> {
            let inputs: Vec<NativeInput> = codes.iter().flat_map(|code| [true, false].map(|pressed| NativeInput { scan_code: *code, ..user(pressed) })).collect();
            self.send(&inputs)
        }

        fn virtual_key(&self, key: u32) -> Result<(), String> {
            self.send(&[NativeInput { virtual_key: key, ..user(true) }, NativeInput { virtual_key: key, ..user(false) }])
        }

        /// Through the keyboard, so that the engine sees the edit: Home and Shift+End select the line in both layouts.
        fn clear(&self) -> Result<(), String> {
            let key = |pressed: bool, virtual_key: u32, extended: bool| NativeInput { virtual_key, extended, ..user(pressed) };
            self.send(&[key(true, VK_HOME, true), key(false, VK_HOME, true), key(true, VK_SHIFT, false), key(true, VK_END, true), key(false, VK_END, true), key(false, VK_SHIFT, false), key(true, VK_BACK, false), key(false, VK_BACK, false)])?;
            if self.pump_until(STANDARD, |editor| editor.text().is_empty()) { Ok(()) } else { Err(format!("the field did not clear: {:?}", self.text())) }
        }

        /// Foreground, an empty field, the layout: everything a phase starts from.
        fn prepare(&self, group: i32) -> Result<(), String> {
            self.activate();
            self.clear()?;
            let layout = if group == 0 { self.layouts.0 } else { self.layouts.1 };
            if !self.api.request_layout(layout) {
                return Err(format!("the layout {group} could not be requested"));
            }
            self.pump(PHASE_START);
            let ready = self.pump_until(STANDARD, |editor| {
                editor.activate();
                editor.group() == group
            });
            if ready { Ok(()) } else { Err(format!("the layout did not become {group}: it is {}", self.group())) }
        }

        fn expect(&self, text: &str, group: i32, milliseconds: u64) -> Result<(), String> {
            if !self.pump_until(milliseconds, |editor| editor.text() == text) {
                return Err(format!("waited for {text:?}, the field holds {:?}", self.text()));
            }
            if self.group() != group {
                return Err(format!("{text:?} came with the layout {}, not {group}", self.group()));
            }
            Ok(())
        }

        /// Foreground and focus can both hold while injected keys still go nowhere: a lone space proves the field receives.
        fn probe(&self) -> Result<(), String> {
            for _ in 0..PROBE_ATTEMPTS {
                self.activate();
                self.pump(PHASE_START);
                self.close_surface_over_this()?;
                self.activate();
                self.pump(PHASE_START);
                self.scans(&[SCAN_SPACE])?;
                if self.pump_until(STANDARD, |editor| editor.text() == " ") {
                    return self.clear();
                }
            }
            Err(format!("injected input never reached the field: {:?}", self.text()))
        }
    }

    impl Drop for Editor {
        fn drop(&mut self) {
            // SAFETY: the window belongs to this thread.
            unsafe {
                DestroyWindow(self.window);
            }
        }
    }

    /// The hook sees an injected key, and marks it as foreign to the program.
    fn hook(api: &Arc<NativeWindowsAPI>) -> Result<(), String> {
        let events: Arc<Mutex<Vec<NativeKeyEvent>>> = Arc::default();
        let (ready, received) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        let (hook_api, listening, marked, seen) = (api.clone(), events.clone(), ready.clone(), received.clone());
        let thread = std::thread::spawn(move || {
            let listener = Arc::new(move |event: NativeKeyEvent| {
                if event.virtual_key == VK_F24 {
                    seen.store(true, Ordering::SeqCst);
                }
                listening.lock().expect("the events").push(event);
                false
            });
            hook_api.run_keyboard_hook(listener, Box::new(move || marked.store(true, Ordering::SeqCst)))
        });
        let wait = |flag: &AtomicBool| {
            let deadline = Instant::now() + Duration::from_millis(STANDARD);
            while !flag.load(Ordering::SeqCst) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            flag.load(Ordering::SeqCst)
        };
        let outcome = (|| {
            if !wait(&ready) {
                return Err("WH_KEYBOARD_LL did not become ready".to_owned());
            }
            let key = |pressed| NativeInput { virtual_key: VK_F24, synthetic: true, ..user(pressed) };
            if api.send_inputs(&[key(true), key(false)]).ok() != Some(2) {
                return Err("SendInput did not accept the F24 sequence".to_owned());
            }
            if !wait(&received) {
                return Err("WH_KEYBOARD_LL did not observe the injected F24".to_owned());
            }
            if !events.lock().expect("the events").iter().any(|event| event.virtual_key == VK_F24 && event.injected) {
                return Err("the injected event was not marked synthetic".to_owned());
            }
            Ok(())
        })();
        api.stop_keyboard_hook();
        thread.join().map_err(|_| "the hook thread panicked".to_owned())?.map_err(|error| format!("{error:?}"))?;
        outcome
    }

    fn configure(root: &Path, extra: &[(&str, Value)]) -> Result<(), String> {
        let settings = SettingsStore::new(Some(root.join("config/config.json")));
        let base = [
            ("general.autostart", json!(false)),
            ("appearance.show_indicator", json!(true)),
            ("general.notifications", json!(false)),
            ("detection.respect_manual_layout", json!(false)),
            // Synthetic typing has no gaps between keys; prefix switching would race with the burst here.
            ("detection.early_switch", json!(false)),
            ("updates.check_automatically", json!(false)),
            // The scenario types with SendInput, which Windows marks as injected by another program; the setting
            // lets the program treat it as typing.
            ("detection.injected_input", json!(true)),
            // A scenario that fails on a runner cannot be examined afterwards: the log says what the engine saw.
            ("diagnostics.technical_logging", json!(true)),
        ];
        for (path, value) in base.iter().cloned().chain(extra.iter().cloned()) {
            settings.set(path, value, true).map_err(|error| format!("{error:?}"))?;
        }
        Ok(())
    }

    /// A copy left running holds the single instance and its own settings: this one would leave at once and nothing would
    /// answer the keys.
    fn start(binary: &Path, root: &Path) -> Result<Child, String> {
        let _ = Command::new("taskkill").args(["/IM", "KeySwitch.exe", "/F"]).output();
        let mut child = Command::new(binary).arg("--hidden").env("KEYSWITCH_CONFIG_DIR", root.join("config")).env("KEYSWITCH_DATA_DIR", root.join("data")).spawn().map_err(|error| error.to_string())?;
        std::thread::sleep(Duration::from_millis(PROGRAM_SETTLES));
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            return Err(format!("the program left at once: {status}"));
        }
        Ok(child)
    }

    fn history_of(root: &Path) -> Result<Vec<(String, String)>, String> {
        let history = HistoryStore::new(Some(root.join("data/history.jsonl")), 200);
        Ok(history.read(None).map_err(|error| error.to_string())?.into_iter().map(|item| (item.original, item.replacement)).collect())
    }

    fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items.iter().map(|(a, b)| ((*a).to_owned(), (*b).to_owned())).collect()
    }

    fn tail(items: Vec<(String, String)>, count: usize) -> Vec<(String, String)> {
        let skip = items.len().saturating_sub(count);
        items.into_iter().skip(skip).collect()
    }

    fn corrections(editor: &Editor, root: &Path) -> Result<(), String> {
        // The hook comes up after the process: the first word is typed again until the engine answers.
        let mut attempt = 0;
        loop {
            attempt += 1;
            editor.prepare(0)?;
            editor.scans(&PRIVET)?;
            match editor.expect("привет", 1, LONG) {
                Ok(()) => break,
                Err(error) if attempt >= START_ATTEMPTS => return Err(error),
                Err(_) => editor.pump(PHASE_SETTLE),
            }
        }
        println!("WINDOWS_EN_TO_RU_E2E_OK");
        editor.pump(PHASE_SETTLE);
        // Another window can take the foreground of a runner in the middle of a correction, and the engine rightly gives
        // the correction up: the word is typed again.
        let mut attempt = 0;
        loop {
            attempt += 1;
            editor.prepare(1)?;
            editor.scans(&HELLO)?;
            editor.scans(&[SCAN_SPACE])?;
            match editor.expect("hello ", 0, LONG) {
                Ok(()) => break,
                Err(error) if attempt >= START_ATTEMPTS => return Err(error),
                Err(_) => editor.pump(PHASE_SETTLE),
            }
        }
        // The engine records a correction after it has typed it.
        let wanted = pairs(&[("ghbdtn", "привет"), ("руддщ", "hello")]);
        if !editor.pump_until(STANDARD, |_| history_of(root).is_ok_and(|history| tail(history, 2) == wanted)) {
            return Err(format!("unexpected correction history: {:?}", tail(history_of(root)?, 2)));
        }
        println!("WINDOWS_RU_TO_EN_E2E_OK");
        editor.pump(PHASE_SETTLE);
        Ok(())
    }

    fn learning(editor: &Editor, root: &Path) -> Result<(), String> {
        let store = LearningStore::new(Some(root.join("data/learning.json")), None).map_err(|error| format!("{error:?}"))?;
        editor.maximize();
        editor.prepare(0)?;
        editor.scans(&HELLO)?;
        editor.expect("hello", 0, LONG)?;
        // A double press: the first converts the word, the second offers a rule.
        editor.virtual_key(VK_PAUSE)?;
        editor.virtual_key(VK_PAUSE)?;
        editor.expect("руддщ", 1, LONG)?;
        editor.pump(WINDOW_OPENS);
        // Enter on the offer opens the rule window, which takes the keyboard; nothing is stored yet.
        editor.virtual_key(VK_RETURN)?;
        // SAFETY: takes nothing.
        if !editor.pump_until(LONG, |editor| unsafe { GetForegroundWindow() } != editor.window) {
            return Err("Enter on the offer opened no rule window".to_owned());
        }
        editor.pump(WINDOW_OPENS);
        store.load().map_err(|error| format!("{error:?}"))?;
        if store.count() != 0 {
            return Err("a rule was stored before OK in the rule window".to_owned());
        }
        println!("WINDOWS_RULE_WINDOW_E2E_OK");
        // Enter in the rule window is OK.
        editor.virtual_key(VK_RETURN)?;
        let stored = editor.pump_until(STANDARD, |_| {
            let _ = store.load();
            store.forced_target(0, "hello") == Some(1)
        });
        if !stored {
            return Err(format!("OK in the rule window stored nothing: {:?}", store.rules()));
        }
        editor.pump(PHASE_SETTLE);
        if !editor.is_zoomed() {
            return Err("the rule offer changed the maximized target window".to_owned());
        }
        if editor.text() != "руддщ" {
            return Err(format!("the rule offer changed the field: {:?}", editor.text()));
        }
        println!("WINDOWS_PROMPT_ENTER_NO_LEAK_E2E_OK");
        Ok(())
    }

    /// The windows of `process` with `class`, top-level, in the order Windows lists them.
    fn windows_of(process: u32, class: &str) -> Vec<HWND> {
        let class = wide(class);
        let mut found = Vec::new();
        let mut after: HWND = std::ptr::null_mut();
        loop {
            // SAFETY: the class name is NUL-terminated and lives through the call; a null parent means top-level windows.
            after = unsafe { FindWindowExW(std::ptr::null_mut(), after, class.as_ptr(), std::ptr::null()) };
            if after.is_null() {
                return found;
            }
            let mut owner = 0_u32;
            // SAFETY: `owner` is a valid place for the id.
            unsafe { GetWindowThreadProcessId(after, &raw mut owner) };
            if owner == process {
                found.push(after);
            }
        }
    }

    /// The tray menu the program shows, once it is on the screen: its menu and the line that switches the layout.
    fn shown_menu(process: u32) -> Option<(HMENU, u32, String)> {
        for window in windows_of(process, MENU_CLASS) {
            // SAFETY: the window is a menu window of the program, which answers MN_GETHMENU with its menu.
            let menu = unsafe {
                if IsWindowVisible(window) == 0 {
                    continue;
                }
                SendMessageW(window, MN_GETHMENU, 0, 0)
            };
            let menu: HMENU = menu.pointer_mut::<std::ffi::c_void>();
            // SAFETY: the menu handle came from its window; a menu gone meanwhile answers -1 and empty lines.
            let count = unsafe { GetMenuItemCount(menu) };
            for index in 0..count.max(0).to::<u32>() {
                let mut line = [0_u16; MENU_LINE_CHARACTERS];
                // SAFETY: the buffer holds the number of characters given.
                let length = unsafe { GetMenuStringW(menu, index, line.as_mut_ptr(), line.len().low::<i32>(), MF_BYPOSITION) };
                let line = String::from_utf16_lossy(&line[..length.max(0).to::<usize>()]);
                if line.starts_with(SWITCH_LINE) {
                    return Some((menu, index, line));
                }
            }
        }
        None
    }

    /// A press and a release of the left button where the cursor is.
    fn click() -> Result<(), String> {
        let button = |flags| INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 { mi: MOUSEINPUT { dx: 0, dy: 0, mouseData: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
        };
        let inputs = [button(MOUSEEVENTF_LEFTDOWN), button(MOUSEEVENTF_LEFTUP)];
        // SAFETY: the inputs are initialised and their size is the one passed.
        let sent = unsafe { SendInput(inputs.len().low::<u32>(), inputs.as_ptr(), size_of::<INPUT>().low::<i32>()) };
        if sent.to::<usize>() == inputs.len() { Ok(()) } else { Err(format!("SendInput took {sent} of the two button events")) }
    }

    /// What is in front when the shell tells the program of the click on its icon.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Front {
        /// The field typed in.
        Field,
        /// The taskbar, which a click on the icon brings to the front a moment before.
        TaskbarClicked,
        /// The taskbar, long enough for the engine to have looked at what is in front.
        TaskbarAWhile,
    }

    fn taskbar() -> Result<HWND, String> {
        let class = wide(SHELL_TRAY_WINDOW_CLASSES[0]);
        // SAFETY: the class name is NUL-terminated and lives through the call.
        let taskbar = unsafe { FindWindowExW(std::ptr::null_mut(), std::ptr::null_mut(), class.as_ptr(), std::ptr::null()) };
        if taskbar.is_null() { Err("the runner has no taskbar".to_owned()) } else { Ok(taskbar) }
    }

    fn bring_taskbar_forward(editor: &Editor) -> Result<HWND, String> {
        let taskbar = taskbar()?;
        editor.api.activate_window((taskbar.to::<isize>()).to::<i64>());
        // SAFETY: takes nothing.
        if editor.pump_until(STANDARD, |_| unsafe { GetForegroundWindow() } == taskbar) {
            Ok(taskbar)
        } else {
            Err("the taskbar did not come to the front".to_owned())
        }
    }

    /// The layout chosen in the tray menu goes to the window the user typed in, although the menu has taken the front from
    /// it, and that window has the front back afterwards. The program's settings window gets the message the shell sends
    /// for a click on the icon, with `front` in front; the line of the menu is clicked as a person clicks it.
    fn tray_switch(editor: &Editor, process: u32, group: i32, front: Front) -> Result<(), String> {
        let wanted = if group == 0 { "(EN)" } else { "(RU)" };
        let window = *windows_of(process, SETTINGS_WINDOW_CLASS).first().ok_or("the program has no settings window")?;
        // The engine follows the layout of the window in front at its next wake.
        editor.pump(PROGRAM_SETTLES);
        // The shell lets the program of a clicked icon take the front, and so does this.
        // SAFETY: takes the process id by value.
        unsafe { AllowSetForegroundWindow(process) };
        if front != Front::Field {
            bring_taskbar_forward(editor)?;
        }
        if front == Front::TaskbarAWhile {
            editor.pump(PROGRAM_SETTLES);
            let taskbar_group = group_of_layout(editor.api.foreground_layout());
            if taskbar_group != group {
                return Err(format!("the taskbar has the layout {taskbar_group}, not the one the menu is to offer"));
            }
        }
        // SAFETY: the message is the icon's callback, which carries the mouse message in its lParam.
        if unsafe { PostMessageW(window, WM_TRAY, 1, WM_RBUTTONUP.to::<LPARAM>()) } == 0 {
            return Err("the tray message could not be posted".to_owned());
        }
        let mut shown = None;
        editor.pump_until(STANDARD, |_| {
            shown = shown_menu(process);
            shown.is_some()
        });
        let (menu, index, line) = shown.ok_or("the tray menu did not open")?;
        if !line.ends_with(wanted) {
            return Err(format!("the tray menu offers {line:?}, not the switch to {wanted}"));
        }
        let mut place = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        // SAFETY: a null window lets the call find the window of the popup menu; the rectangle is ours.
        if unsafe { GetMenuItemRect(std::ptr::null_mut(), menu, index, &raw mut place) } == 0 {
            return Err("the place of the menu line is unknown".to_owned());
        }
        let point = POINT { x: i32::midpoint(place.left, place.right), y: i32::midpoint(place.top, place.bottom) };
        // SAFETY: plain calls with values.
        unsafe {
            SetCursorPos(point.x, point.y);
            if MenuItemFromPoint(std::ptr::null_mut(), menu, point).to::<i64>() != i64::from(index) {
                return Err(format!("the cursor at {}, {} is not on the menu line", point.x, point.y));
            }
        }
        click()?;
        // SAFETY: takes nothing.
        let switched = editor.pump_until(LONG, |editor| editor.group() == group && unsafe { GetForegroundWindow() } == editor.window);
        if !switched {
            // SAFETY: takes nothing.
            let front = unsafe { GetForegroundWindow() } == editor.window;
            return Err(format!("after {line:?} the field's layout is {}, and the field is {}in front", editor.group(), if front { "" } else { "not " }));
        }
        Ok(())
    }

    /// Whether each window keeps a layout of its own («Позволить выбирать метод ввода для каждого окна приложения», «Let
    /// me use a different input method for each app window»): the per-thread scope of the input settings, against the
    /// session's (SPI_GETTHREADLOCALINPUTSETTINGS of SystemParametersInfoW on Microsoft Learn).
    fn layout_per_window() -> Result<bool, String> {
        let mut local = 0;
        // SAFETY: the call writes one BOOL into `local`.
        if unsafe { SystemParametersInfoW(SPI_GETTHREADLOCALINPUTSETTINGS, 0, (&raw mut local).cast(), 0) } == 0 {
            return Err("the scope of the input settings could not be read".to_owned());
        }
        Ok(local != 0)
    }

    fn set_layout_per_window(per_window: bool) -> Result<(), String> {
        // SAFETY: this action takes the BOOL itself in the pointer's place; nothing is written to the profile.
        let set = unsafe { SystemParametersInfoW(SPI_SETTHREADLOCALINPUTSETTINGS, 0, isize::from(per_window).pointer_mut::<std::ffi::c_void>(), SPIF_SENDCHANGE) };
        if set == 0 || layout_per_window()? != per_window {
            return Err(format!("the layout could not be made {}", if per_window { "per window" } else { "shared" }));
        }
        Ok(())
    }

    /// With a layout for each window the taskbar keeps one of its own. The field gets the other one, and the taskbar stays in
    /// front long enough for the engine to look: the program still tells the engine the field's layout, so the menu offers
    /// the field's other layout, which is the taskbar's.
    fn tray_switch_past_the_taskbar(editor: &Editor, process: u32) -> Result<(), String> {
        bring_taskbar_forward(editor)?;
        let taskbar_group = group_of_layout(editor.api.foreground_layout());
        if taskbar_group < 0 {
            return Err("the taskbar's layout is neither English nor Russian".to_owned());
        }
        editor.prepare(1 - taskbar_group)?;
        tray_switch(editor, process, taskbar_group, Front::TaskbarAWhile)
    }

    /// The tray switch with one layout shared by every window, as Windows has it by default, and with a layout for each
    /// window, where only the window that takes the request changes. The setting of the runner is put back afterwards.
    fn tray_switches(editor: &Editor, process: u32) -> Result<(), String> {
        let before = layout_per_window()?;
        let outcome = (|| {
            for per_window in [false, true] {
                set_layout_per_window(per_window)?;
                editor.prepare(0)?;
                tray_switch(editor, process, 1, Front::Field)?;
                tray_switch(editor, process, 0, Front::TaskbarClicked)?;
                println!("WINDOWS_TRAY_SWITCH_{}_E2E_OK", if per_window { "PER_WINDOW" } else { "SHARED" });
            }
            tray_switch_past_the_taskbar(editor, process)?;
            println!("WINDOWS_TRAY_SWITCH_PAST_THE_TASKBAR_E2E_OK");
            Ok(())
        })();
        set_layout_per_window(before)?;
        outcome
    }

    fn correct_before_enter(editor: &Editor) -> Result<(), String> {
        editor.prepare(0)?;
        let inputs: Vec<NativeInput> = PRIVET.iter().chain(&[SCAN_ENTER]).flat_map(|code| [true, false].map(|pressed| NativeInput { scan_code: *code, ..user(pressed) })).collect();
        editor.send(&inputs)?;
        if !editor.pump_until(LONG, |editor| editor.text().contains('\n')) {
            return Err(format!("the intercepted Enter never reached the field: {:?}", editor.text()));
        }
        editor.pump(PHASE_SETTLE);
        if editor.text() != "привет\r\n" {
            return Err(format!("Enter submitted an uncorrected or duplicate message: {:?}", editor.text()));
        }
        println!("WINDOWS_CORRECT_BEFORE_ENTER_E2E_OK");
        Ok(())
    }

    /// The layout switch key (`detection.layout_switch_key` set to Left Shift): a lone tap switches the layout of the
    /// window and back; Right Shift, and Left Shift held over a letter, switch nothing.
    fn layout_key(editor: &Editor) -> Result<(), String> {
        // The Enter of the phase before left a second, empty line, which Home and Shift+End do not select: one Backspace
        // takes it away first.
        editor.activate();
        editor.virtual_key(VK_BACK)?;
        editor.pump(PHASE_SETTLE);
        editor.prepare(0)?;
        let shift = |virtual_key: u32, scan_code: u32, pressed: bool| NativeInput { virtual_key, scan_code, ..user(pressed) };
        for wanted in [1, 0] {
            editor.send(&[shift(VK_LSHIFT, SCAN_LEFT_SHIFT, true), shift(VK_LSHIFT, SCAN_LEFT_SHIFT, false)])?;
            if !editor.pump_until(STANDARD, |editor| editor.group() == wanted) {
                return Err(format!("a tap of Left Shift left the layout {}, not {wanted}", editor.group()));
            }
            editor.pump(PHASE_SETTLE);
        }
        editor.send(&[shift(VK_RSHIFT, SCAN_RIGHT_SHIFT, true), shift(VK_RSHIFT, SCAN_RIGHT_SHIFT, false)])?;
        let letter = |pressed: bool| NativeInput { scan_code: SCAN_H, ..user(pressed) };
        editor.send(&[shift(VK_LSHIFT, SCAN_LEFT_SHIFT, true), letter(true), letter(false), shift(VK_LSHIFT, SCAN_LEFT_SHIFT, false)])?;
        editor.expect("H", 0, LONG)?;
        editor.pump(PHASE_SETTLE);
        if editor.group() != 0 {
            return Err(format!("Right Shift or Shift with a letter switched the layout to {}", editor.group()));
        }
        println!("WINDOWS_LAYOUT_KEY_E2E_OK");
        Ok(())
    }

    fn run(binary: &Path) -> Result<(), String> {
        let api = Arc::new(NativeWindowsAPI::new());
        hook(&api)?;
        println!("WINDOWS_HOOK_E2E_OK");
        let root: PathBuf = std::env::temp_dir().join(format!("keyswitch-e2e-{}", std::process::id()));
        std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        configure(&root, &[])?;
        let editor = Editor::new(api)?;
        editor.pump(PHASE_SETTLE);
        editor.probe()?;
        let mut child = start(binary, &root)?;
        let outcome = (|| -> Result<(), String> {
            corrections(&editor, &root)?;
            learning(&editor, &root)?;
            editor.prepare(0)?;
            editor.scans(&HELLO)?;
            editor.scans(&[SCAN_SPACE])?;
            editor.expect("руддщ ", 1, LONG)?;
            let wanted = pairs(&[("ghbdtn", "привет"), ("руддщ", "hello"), ("hello", "руддщ")]);
            if !editor.pump_until(STANDARD, |_| history_of(&root).is_ok_and(|history| tail(history, 3) == wanted)) {
                return Err(format!("unexpected learned history: {:?}", tail(history_of(&root)?, 3)));
            }
            println!("WINDOWS_LEARNING_PROMPT_E2E_OK");
            tray_switches(&editor, child.id())?;
            let _ = child.kill();
            let _ = child.wait();
            configure(&root, &[("detection.correct_on_pause", json!(false)), ("detection.correct_on_enter", json!(true)), ("detection.layout_switch_key", json!("Shift_L"))])?;
            child = start(binary, &root)?;
            editor.pump(WINDOW_OPENS);
            // The first word after a start waits for the hook, as above.
            let mut attempt = 0;
            loop {
                attempt += 1;
                match correct_before_enter(&editor) {
                    Ok(()) => break,
                    Err(error) if attempt >= START_ATTEMPTS => return Err(error),
                    Err(_) => {
                        editor.activate();
                        editor.send(&[NativeInput { virtual_key: VK_BACK, ..user(true) }, NativeInput { virtual_key: VK_BACK, ..user(false) }])?;
                        editor.pump(PHASE_SETTLE);
                    }
                }
            }
            layout_key(&editor)
        })();
        let _ = child.kill();
        let _ = child.wait();
        if outcome.is_err() {
            println!("field={:?} group={}", editor.text(), editor.group());
            if let Ok(log) = std::fs::read_to_string(root.join("data/keyswitch.log")) {
                println!("{}", log.lines().rev().take(30).collect::<Vec<&str>>().join("\n"));
            }
        }
        drop(editor);
        let _ = std::fs::remove_dir_all(&root);
        outcome
    }

    pub(crate) fn main() {
        let Some(binary) = std::env::var_os("KS_E2E_BINARY") else {
            println!("KS_E2E_BINARY is not set: no installed program to drive, skipped");
            return;
        };
        match run(Path::new(&binary)) {
            Ok(()) => println!("WINDOWS_E2E_OK"),
            Err(error) => {
                println!("WINDOWS_E2E_FAILED: {error}");
                std::process::exit(1);
            }
        }
    }
}
