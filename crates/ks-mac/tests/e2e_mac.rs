//! The real thing on a Mac: the program, `TextEdit` in front, keys posted at the HID level, the document read back through
//! the accessibility tree. Started from a Terminal that holds the Accessibility permission, by `packaging/e2e-mac.command`,
//! with `KS_E2E_BINARY` naming the program; otherwise, and on other systems, it says so and passes.

#![cfg_attr(target_os = "macos", expect(unsafe_code, reason = "calls into the macOS system libraries"))]
#[cfg(not(target_os = "macos"))]
fn main() {
    println!("the Mac end-to-end test runs on macOS only, skipped");
}

#[cfg(target_os = "macos")]
fn main() {
    scenario::main();
}

#[cfg(target_os = "macos")]
mod scenario {
    use ks_cast::To as _;
    use std::cell::Cell;
    use std::ffi::{CString, c_char, c_long, c_ulong, c_void};
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use ks_backend::InputBackend;
    use ks_config::config::SettingsStore;
    use ks_config::constants::HINTS_SETTING;
    use ks_mac::constants::{CF_STRING_ENCODING_UTF8, HINT_PANEL_GAP_POINTS};
    use ks_mac::hint_place::{CaretAnchor, Frame, caret_anchor};
    use ks_mac::macos_app::VERSION;
    use ks_mac::macos_backend::MacBackend;
    use ks_mac::macos_context::{AccessibilityAPI, SelectionAccessibilityAPI};
    use ks_mac::macos_native::{NativeAccessibility, NativeMacAPI};
    use ks_mac::macos_objc::frontmost_application;
    use ks_ui::constants::HINT_CARD_WIDTH_PIXELS;
    use ks_ui::hints::{HINT_NEVER_LABEL, HINT_OK_LABEL};
    use ks_ui::windows_ui_model::HINTS_SETTING_TITLE;
    use serde_json::{Value, json};

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFRunLoopDefaultMode: *const c_void;
        fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after_source: bool) -> i32;
        fn CFRelease(object: *const c_void);
        fn CFArrayGetCount(array: *const c_void) -> c_long;
        fn CFArrayGetValueAtIndex(array: *const c_void, index: c_long) -> *const c_void;
        fn CFDictionaryGetValue(dictionary: *const c_void, key: *const c_void) -> *const c_void;
        fn CFNumberGetValue(number: *const c_void, kind: c_long, value: *mut c_void) -> bool;
        fn CFStringCreateWithCString(allocator: *const c_void, text: *const c_char, encoding: u32) -> *const c_void;
        fn CFGetTypeID(object: *const c_void) -> c_ulong;
        fn CFArrayGetTypeID() -> c_ulong;
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGRect {
        origin: CGPoint,
        width: f64,
        height: f64,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        static kCGWindowOwnerPID: *const c_void;
        static kCGWindowLayer: *const c_void;
        static kCGWindowBounds: *const c_void;
        fn CGEventSourceCreate(state: i32) -> *const c_void;
        fn CGEventCreateKeyboardEvent(source: *const c_void, keycode: u16, down: bool) -> *const c_void;
        fn CGEventCreateMouseEvent(source: *const c_void, kind: u32, position: CGPoint, button: u32) -> *const c_void;
        fn CGEventPost(tap: u32, event: *const c_void);
        fn CGWindowListCopyWindowInfo(option: u32, window: u32) -> *const c_void;
        fn CGRectMakeWithDictionaryRepresentation(dictionary: *const c_void, rectangle: *mut CGRect) -> bool;
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXUIElementCreateApplication(process: i32) -> *const c_void;
        fn AXUIElementCopyAttributeValue(element: *const c_void, name: *const c_void, value: *mut *const c_void) -> i32;
        fn AXUIElementPerformAction(element: *const c_void, action: *const c_void) -> i32;
        fn AXValueGetValue(value: *const c_void, kind: u32, output: *mut c_void) -> bool;
    }

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOServiceMatching(name: *const c_char) -> *const c_void;
        fn IOServiceGetMatchingService(port: u32, matching: *const c_void) -> u32;
        fn IOServiceOpen(service: u32, task: u32, kind: u32, connect: *mut u32) -> i32;
        fn IOHIDSetModifierLockState(connect: u32, selector: i32, state: bool) -> i32;
        fn IOServiceClose(connect: u32) -> i32;
        static mach_task_self_: u32;
    }

    const HID_TAP: u32 = 0;
    const HID_SYSTEM_STATE: i32 = 1;
    const CAPS_LOCK_SELECTOR: i32 = 1;
    const IOHID_PARAM_CONNECT: u32 = 1;
    const SHIFT: u16 = 0x38;
    const RIGHT_SHIFT: u16 = 0x3C;
    const CONTROL: u16 = 0x3B;
    const OPTION: u16 = 0x3A;
    const COMMAND: u16 = 0x37;
    const BACKSPACE: u16 = 0x33;
    const KEY_PAUSE: Duration = Duration::from_millis(50);
    const AFTER_KEY: Duration = Duration::from_millis(60);
    const SETTLED: Duration = Duration::from_millis(2500);
    const CHANGE_DEADLINE: Duration = Duration::from_secs(4);
    const PROGRAM_STARTS: Duration = Duration::from_secs(15);
    const DOCUMENT_OPENS: Duration = Duration::from_secs(5);
    const FRONT_ATTEMPTS: u32 = 3;
    const RUSSIAN_KEYS: [(char, char); 26] = [
        ('й', 'q'), ('ц', 'w'), ('у', 'e'), ('к', 'r'), ('е', 't'), ('н', 'y'), ('г', 'u'), ('ш', 'i'), ('щ', 'o'), ('з', 'p'), ('ф', 'a'), ('ы', 's'), ('в', 'd'),
        ('а', 'f'), ('п', 'g'), ('р', 'h'), ('о', 'j'), ('л', 'k'), ('д', 'l'), ('я', 'z'), ('ч', 'x'), ('с', 'c'), ('м', 'v'), ('и', 'b'), ('т', 'n'), ('ь', 'm'),
    ];
    const SENTINEL: &str = "KSProbe-pasteboard-sentinel";
    const RETURN: u16 = 0x24;
    /// `kCGEventLeftMouseDown`, `kCGEventLeftMouseUp`, `kCGMouseButtonLeft`.
    const MOUSE_DOWN: u32 = 1;
    const MOUSE_UP: u32 = 2;
    const LEFT_BUTTON: u32 = 0;
    /// `kCGWindowListOptionOnScreenOnly`, and the layer `NSFloatingWindowLevel` and the menu bar items are drawn on.
    const ON_SCREEN_ONLY: u32 = 1;
    const FLOATING_LAYER: i32 = 3;
    const STATUS_ITEM_LAYER: i32 = 25;
    /// `kCFNumberSInt32Type`, `kAXValueCGPointType`, `kAXValueCGSizeType`.
    const NUMBER_SINT32: c_long = 3;
    const AX_POINT: u32 = 1;
    const AX_SIZE: u32 = 2;
    const AX_SEARCH_DEPTH: usize = 12;
    /// A hint at the caret is due a second after the typing stops, and the tick that draws it comes a quarter of a second later.
    const CARET_HINT_APPEARS: Duration = Duration::from_secs(4);
    /// The first-start hint waits five seconds after the engine runs, and the models load first.
    const TRAY_HINT_APPEARS: Duration = Duration::from_secs(40);
    const WINDOW_APPEARS: Duration = Duration::from_secs(20);
    const CARD_APPEARS: Duration = Duration::from_secs(3);
    const HINT_GOES: Duration = Duration::from_secs(2);

    fn keycode(name: char) -> Option<u16> {
        Some(match name {
            'a' => 0x00,
            's' => 0x01,
            'd' => 0x02,
            'f' => 0x03,
            'h' => 0x04,
            'g' => 0x05,
            'z' => 0x06,
            'x' => 0x07,
            'c' => 0x08,
            'v' => 0x09,
            'b' => 0x0B,
            'q' => 0x0C,
            'w' => 0x0D,
            'e' => 0x0E,
            'r' => 0x0F,
            'y' => 0x10,
            't' => 0x11,
            'o' => 0x1F,
            'u' => 0x20,
            'i' => 0x22,
            'p' => 0x23,
            'l' => 0x25,
            'j' => 0x26,
            'k' => 0x28,
            'n' => 0x2D,
            'm' => 0x2E,
            ' ' => 0x31,
            _ => return None,
        })
    }

    /// Lets `HIToolbox` hear about layout changes: without a run loop this process keeps a stale one.
    fn pump(seconds: f64) {
        // SAFETY: the mode is the framework's constant.
        unsafe {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, seconds, false);
        }
    }

    fn real_caps_lock(on: bool) -> Result<(), String> {
        let name = CString::new("IOHIDSystem").expect("a name");
        // SAFETY: the name is NUL-terminated; the matching dictionary is consumed by the lookup; the connection is closed.
        unsafe {
            let service = IOServiceGetMatchingService(0, IOServiceMatching(name.as_ptr()));
            let mut connect = 0_u32;
            let status = IOServiceOpen(service, mach_task_self_, IOHID_PARAM_CONNECT, &raw mut connect);
            if status != 0 {
                return Err(format!("IOServiceOpen: {status}"));
            }
            let status = IOHIDSetModifierLockState(connect, CAPS_LOCK_SELECTOR, on);
            IOServiceClose(connect);
            if status == 0 { Ok(()) } else { Err(format!("IOHIDSetModifierLockState: {status}")) }
        }
    }

    struct Driver {
        source: *const c_void,
        ax: NativeAccessibility,
        backend: Arc<MacBackend>,
        document: String,
        failures: std::cell::RefCell<Vec<String>>,
        /// The process of the program under test, and its data folder.
        program: Cell<i32>,
        data: std::cell::RefCell<PathBuf>,
    }

    impl Driver {
        /// Stops the run unless `TextEdit` is in front: no key may reach another program.
        fn guard(&self) -> Result<(), String> {
            for _ in 0..FRONT_ATTEMPTS {
                if frontmost_application().1 == "TextEdit" {
                    return Ok(());
                }
                let _ = Command::new("open").args(["-a", "TextEdit", &self.document]).status();
                std::thread::sleep(Duration::from_secs(2));
            }
            Err(format!("TextEdit is not in front: {:?}", frontmost_application()))
        }

        fn key(&self, code: u16, down: bool) -> Result<(), String> {
            if down {
                self.guard()?;
            }
            // SAFETY: the source lives as long as the driver; the event is released after it is posted.
            unsafe {
                let event = CGEventCreateKeyboardEvent(self.source, code, down);
                CGEventPost(HID_TAP, event);
                CFRelease(event);
            }
            std::thread::sleep(KEY_PAUSE);
            Ok(())
        }

        fn release_modifiers(&self) -> Result<(), String> {
            for code in [SHIFT, CONTROL, OPTION, COMMAND] {
                self.key(code, false)?;
            }
            Ok(())
        }

        fn chord(&self, name: char, modifiers: &[u16]) -> Result<(), String> {
            let code = keycode(name).ok_or_else(|| format!("no key for {name:?}"))?;
            for modifier in modifiers {
                self.key(*modifier, true)?;
            }
            self.key(code, true)?;
            self.key(code, false)?;
            for modifier in modifiers.iter().rev() {
                self.key(*modifier, false)?;
            }
            std::thread::sleep(AFTER_KEY);
            Ok(())
        }

        /// Keys, in physical terms; `russian` reads the text by the Russian layout's keys.
        fn type_keys(&self, text: &str, russian: bool) -> Result<(), String> {
            for character in text.chars() {
                let lower = character.to_lowercase().next().unwrap_or(character);
                let name = if russian { RUSSIAN_KEYS.iter().find(|(letter, _)| *letter == lower).map_or(lower, |(_, key)| *key) } else { lower };
                if character == lower {
                    let code = keycode(name).ok_or_else(|| format!("no key for {name:?}"))?;
                    self.key(code, true)?;
                    self.key(code, false)?;
                } else {
                    self.chord(name, &[SHIFT])?;
                }
                std::thread::sleep(AFTER_KEY);
            }
            Ok(())
        }

        fn attribute(&self, name: &str) -> Option<String> {
            let element = self.ax.focused_element();
            let value = self.ax.string_attribute(element, name);
            self.ax.release(element);
            value
        }

        fn value(&self) -> String {
            self.attribute("AXValue").unwrap_or_default()
        }

        fn put_text(&self, text: &str) -> Result<(), String> {
            self.chord('a', &[COMMAND])?;
            let element = self.ax.focused_element();
            if !self.value().is_empty() || !text.is_empty() {
                self.ax.set_string_attribute(element, "AXSelectedText", text);
            }
            self.ax.release(element);
            std::thread::sleep(Duration::from_millis(300));
            Ok(())
        }

        fn select(&self, group: i32) -> Result<(), String> {
            pump(0.3);
            if self.backend.current_group().map_err(|error| format!("{error:?}"))? != group {
                self.backend.switch_group(group).map_err(|error| format!("{error:?}"))?;
                pump(1.0);
            }
            Ok(())
        }

        fn group(&self) -> i32 {
            pump(0.3);
            self.backend.current_group().unwrap_or(-1)
        }

        fn wait_change(&self, before: &str) -> String {
            let deadline = Instant::now() + CHANGE_DEADLINE;
            while self.value() == before && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(100));
            }
            std::thread::sleep(Duration::from_millis(400));
            self.value()
        }

        fn check(&self, name: &str, holds: bool, detail: &str) {
            println!("{} {name}: {detail}", if holds { "OK  " } else { "FAIL" });
            if !holds {
                self.failures.borrow_mut().push(name.to_owned());
            }
        }
    }

    fn wait_for<T>(deadline: Duration, ask: impl Fn() -> Option<T>) -> Option<T> {
        let end = Instant::now() + deadline;
        loop {
            let answer = ask();
            if answer.is_some() || Instant::now() >= end {
                return answer;
            }
            pump(0.1);
        }
    }

    /// A window as the window server lists it: the corner is the top left one, in the coordinates of Quartz.
    #[derive(Clone, Copy, Debug)]
    struct Window {
        layer: i32,
        frame: Frame,
    }

    fn windows_of(process: i32) -> Vec<Window> {
        let mut found = Vec::new();
        // SAFETY: the list is released below and its entries read only before that; the numbers and the rectangle are
        // written by the calls.
        unsafe {
            let list = CGWindowListCopyWindowInfo(ON_SCREEN_ONLY, 0);
            if list.is_null() {
                return found;
            }
            for index in 0..CFArrayGetCount(list) {
                let entry = CFArrayGetValueAtIndex(list, index);
                let (mut owner, mut layer) = (0_i32, 0_i32);
                CFNumberGetValue(CFDictionaryGetValue(entry, kCGWindowOwnerPID), NUMBER_SINT32, (&raw mut owner).cast());
                if owner != process {
                    continue;
                }
                CFNumberGetValue(CFDictionaryGetValue(entry, kCGWindowLayer), NUMBER_SINT32, (&raw mut layer).cast());
                let mut bounds = CGRect::default();
                if CGRectMakeWithDictionaryRepresentation(CFDictionaryGetValue(entry, kCGWindowBounds), &raw mut bounds) {
                    found.push(Window { layer, frame: Frame { x: bounds.origin.x, y: bounds.origin.y, width: bounds.width, height: bounds.height } });
                }
            }
            CFRelease(list);
        }
        found
    }

    /// The hint panel on screen: the program's floating window of the card's width.
    fn hint_window(process: i32) -> Option<Frame> {
        windows_of(process).into_iter().find(|window| window.layer == FLOATING_LAYER && (window.frame.width - f64::from(HINT_CARD_WIDTH_PIXELS)).abs() < 2.0).map(|window| window.frame)
    }

    fn status_item_window(process: i32) -> Option<Frame> {
        windows_of(process).into_iter().find(|window| window.layer == STATUS_ITEM_LAYER).map(|window| window.frame)
    }

    /// Over the caret's character, right above its line or, with no room left there, right under it.
    fn stands_at_the_caret(panel: Frame, caret: Frame) -> bool {
        let reach = HINT_PANEL_GAP_POINTS + 2.0;
        let (above, under) = (panel.top() <= caret.y && caret.y - panel.top() <= reach, panel.y >= caret.top() && panel.y - caret.top() <= reach);
        (above || under) && panel.x <= caret.x && caret.x <= panel.right()
    }

    /// Under the item and over it in the width of the panel, at the gap: not in the middle of the screen.
    fn stands_under_the_item(panel: Frame, item: Frame) -> bool {
        let center = item.x + item.width / 2.0;
        panel.y >= item.top() && panel.y <= item.top() + HINT_PANEL_GAP_POINTS + 2.0 && panel.x <= center && center <= panel.right()
    }

    fn cf_string(text: &str) -> *const c_void {
        let text = CString::new(text).expect("a name without NUL");
        // SAFETY: the text outlives the call, which copies it; the result is kept for the short life of this test.
        unsafe { CFStringCreateWithCString(std::ptr::null(), text.as_ptr(), CF_STRING_ENCODING_UTF8) }
    }

    /// The accessibility tree of the program under test, read the way an assistive program reads it. Elements are usize as in
    /// `AccessibilityAPI`, and kept for the short life of this test.
    fn ax_attribute(element: usize, name: &str) -> *const c_void {
        let mut value: *const c_void = std::ptr::null();
        // SAFETY: the element is valid and the name lives through the call; the value is written by the call.
        unsafe { AXUIElementCopyAttributeValue(element.pointer::<c_void>(), cf_string(name), &raw mut value) };
        value
    }

    fn ax_elements(element: usize, name: &str) -> Vec<usize> {
        let value = ax_attribute(element, name);
        if value.is_null() {
            return Vec::new();
        }
        // SAFETY: the value is an array of elements, which is kept alive for the rest of the test.
        unsafe {
            if CFGetTypeID(value) != CFArrayGetTypeID() {
                return Vec::new();
            }
            (0..CFArrayGetCount(value)).map(|index| CFArrayGetValueAtIndex(value, index).to::<usize>()).collect()
        }
    }

    fn ax_frame(element: usize) -> Option<Frame> {
        let (position, size) = (ax_attribute(element, "AXPosition"), ax_attribute(element, "AXSize"));
        if position.is_null() || size.is_null() {
            return None;
        }
        let (mut origin, mut extent) = (CGPoint::default(), CGPoint::default());
        // SAFETY: the values wrap a point and a size, written by the calls.
        unsafe {
            AXValueGetValue(position, AX_POINT, (&raw mut origin).cast());
            AXValueGetValue(size, AX_SIZE, (&raw mut extent).cast());
        }
        Some(Frame { x: origin.x, y: origin.y, width: extent.x, height: extent.y })
    }

    fn ax_press(element: usize) -> bool {
        // SAFETY: the element is valid and the name lives through the call.
        unsafe { AXUIElementPerformAction(element.pointer::<c_void>(), cf_string("AXPress")) == 0 }
    }

    fn ax_windows(process: i32) -> Vec<usize> {
        // SAFETY: creates a reference kept for the short life of this test.
        let application = unsafe { AXUIElementCreateApplication(process) }.to::<usize>();
        ax_elements(application, "AXWindows")
    }

    impl Driver {
        fn string(&self, element: usize, name: &str) -> String {
            self.ax.string_attribute(element, name).unwrap_or_default()
        }

        /// The first element under `element`, depth first, of this role and whose title and value hold `text`.
        fn ax_find(&self, element: usize, role: &str, text: &str, depth: usize) -> Option<usize> {
            let (found_role, title, value) = (self.string(element, "AXRole"), self.string(element, "AXTitle"), self.string(element, "AXValue"));
            if found_role == role && (title.contains(text) || value.contains(text)) {
                return Some(element);
            }
            if depth == 0 {
                return None;
            }
            ax_elements(element, "AXChildren").into_iter().find_map(|child| self.ax_find(child, role, text, depth - 1))
        }

        /// The element of the program's windows of this role and text.
        fn on_screen(&self, role: &str, text: &str) -> Option<usize> {
            ax_windows(self.program.get()).into_iter().find_map(|window| self.ax_find(window, role, text, AX_SEARCH_DEPTH))
        }

        /// The first button under `element` with no title: the one laid over a text.
        fn ax_untitled_button(&self, element: usize, depth: usize) -> Option<usize> {
            if self.string(element, "AXRole") == "AXButton" && self.string(element, "AXTitle").is_empty() && !self.has_close_action(element) {
                return Some(element);
            }
            if depth == 0 {
                return None;
            }
            ax_elements(element, "AXChildren").into_iter().find_map(|child| self.ax_untitled_button(child, depth - 1))
        }

        /// The standard buttons of a window have no title either: those carry a subrole.
        fn has_close_action(&self, element: usize) -> bool {
            !self.string(element, "AXSubrole").is_empty()
        }

        /// Whether some group under `element` surrounds `frame`: the marker of a setting.
        fn ax_group_around(&self, element: usize, frame: Frame, depth: usize) -> bool {
            if self.string(element, "AXRole") == "AXGroup"
                && ax_frame(element).is_some_and(|group| group.x < frame.x && group.right() > frame.right() && group.y < frame.y && group.top() > frame.top())
            {
                return true;
            }
            depth > 0 && ax_elements(element, "AXChildren").into_iter().any(|child| self.ax_group_around(child, frame, depth - 1))
        }

        /// A real click, through the window server, on the middle of an element.
        fn click(&self, frame: Frame) {
            let point = CGPoint { x: frame.x + frame.width / 2.0, y: frame.y + frame.height / 2.0 };
            for kind in [MOUSE_DOWN, MOUSE_UP] {
                // SAFETY: the source lives as long as the driver; the event is released after it is posted.
                unsafe {
                    let event = CGEventCreateMouseEvent(self.source, kind, point, LEFT_BUTTON);
                    CGEventPost(HID_TAP, event);
                    CFRelease(event);
                }
                std::thread::sleep(KEY_PAUSE);
            }
        }

        fn front(&self) -> String {
            frontmost_application().1
        }

        fn hints_file(&self) -> Value {
            std::fs::read_to_string(self.data.borrow().join("hints.json")).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or(Value::Null)
        }

        /// How the hint `id` went away, by the file the hints keep.
        fn how(&self, id: &str) -> String {
            self.hints_file()["seen"][id]["how"].as_str().unwrap_or("").to_owned()
        }

        /// Whether the panel went away and the file of the hints says how.
        fn went_away(&self, id: &str, how: &str) -> bool {
            wait_for(HINT_GOES, || (hint_window(self.program.get()).is_none() && self.how(id) == how).then_some(())).is_some()
        }
    }

    /// Writes the file of the hints as an earlier run would have left it.
    fn seed_hints(data: &Path, content: &Value) -> Result<(), String> {
        std::fs::create_dir_all(data).map_err(|error| error.to_string())?;
        std::fs::write(data.join("hints.json"), content.to_string()).map_err(|error| error.to_string())
    }

    /// The first correction brought its hint at the caret. It takes the focus from nothing: the editor stays in front, the
    /// undo hotkey and Return work as before, and a real click on «Понятно» leaves the editor in front.
    fn hint_at_the_caret(driver: &Driver) -> Result<(), String> {
        let process = driver.program.get();
        let Some(panel) = wait_for(CARET_HINT_APPEARS, || hint_window(process)) else {
            driver.check("a hint after the first correction", false, &format!("windows {:?}", windows_of(process)));
            return Ok(());
        };
        driver.check("a hint after the first correction", true, &format!("{panel:?}"));
        driver.check("the editor stays in front of a hint", driver.front() == "TextEdit", &driver.front());
        match caret_anchor(&driver.ax) {
            CaretAnchor::At(caret) => driver.check("the hint stands at the caret", stands_at_the_caret(panel, caret), &format!("panel {panel:?}, caret {caret:?}")),
            CaretAnchor::Unknown => {
                let item = status_item_window(process).ok_or("no menu bar item")?;
                driver.check("the hint stands under the menu bar item", stands_under_the_item(panel, item), &format!("panel {panel:?}, item {item:?}"));
            }
        }
        let corrected = driver.value();
        driver.chord('z', &[CONTROL, OPTION])?;
        let undone = driver.wait_change(&corrected);
        driver.check("undo hotkey with a hint on screen", corrected == "the привет " && undone == "the ghbdtn ", &format!("{corrected:?} then {undone:?}"));
        driver.key(RETURN, true)?;
        driver.key(RETURN, false)?;
        let entered = driver.wait_change(&undone);
        driver.check("Return reaches the field past a hint", entered == "the ghbdtn \n", &format!("{entered:?}"));
        let Some(button) = driver.on_screen("AXButton", HINT_OK_LABEL).and_then(ax_frame) else {
            driver.check("the button of a hint", false, "no button in the accessibility tree");
            return Ok(());
        };
        driver.click(button);
        driver.check("a click takes the hint away", driver.went_away("first_correction_undo", "ok"), &format!("{:?}", driver.how("first_correction_undo")));
        driver.check("a click on a hint leaves the editor in front", driver.front() == "TextEdit", &driver.front());
        driver.type_keys("x", false)?;
        std::thread::sleep(Duration::from_millis(400));
        driver.check("the field still takes keys", driver.value() == "the ghbdtn \nx", &driver.value());
        Ok(())
    }

    /// A fresh start of the program; everything but `root`'s hints file is new.
    fn start_program(binary: &Path, folder: &Path, hidden: bool) -> Result<Child, String> {
        let settings = SettingsStore::new(Some(folder.join("config/config.json")));
        for (path, value) in [("general.autostart", json!(false)), ("updates.check_automatically", json!(false)), ("diagnostics.technical_logging", json!(true))] {
            settings.set(path, value, true).map_err(|error| format!("{error:?}"))?;
        }
        let mut command = Command::new(binary);
        if hidden {
            command.arg("--hidden");
        }
        command.env("KEYSWITCH_CONFIG_DIR", folder.join("config")).env("KEYSWITCH_DATA_DIR", folder.join("data")).spawn().map_err(|error| error.to_string())
    }

    fn program_id(child: &Child) -> i32 {
        i32::try_from(child.id()).expect("a process id fits")
    }

    /// The hint for a new installation: under the menu bar item, the editor keeps the front, and a real click on the second
    /// button turns the hints off where the settings show them.
    fn hint_at_the_item(driver: &Driver, binary: &Path, folder: &Path) -> Result<(), String> {
        let data = folder.join("data");
        seed_hints(&data, &json!({"last_run_version": VERSION, "fresh_install": true}))?;
        let mut child = start_program(binary, folder, true)?;
        driver.program.set(program_id(&child));
        *driver.data.borrow_mut() = data;
        let outcome = at_the_item(driver, folder);
        let _ = child.kill();
        let _ = child.wait();
        outcome
    }

    fn at_the_item(driver: &Driver, folder: &Path) -> Result<(), String> {
        let process = driver.program.get();
        let Some(panel) = wait_for(TRAY_HINT_APPEARS, || hint_window(process)) else {
            driver.check("the hint of a new installation", false, &format!("windows {:?}", windows_of(process)));
            return Ok(());
        };
        driver.check("the hint of a new installation", driver.how("first_start") == "shown", &format!("{panel:?}, how {:?}", driver.how("first_start")));
        let item = status_item_window(process).ok_or("no menu bar item")?;
        driver.check("the hint stands under the menu bar item", stands_under_the_item(panel, item), &format!("panel {panel:?}, item {item:?}"));
        driver.check("the editor stays in front of a hint at the item", driver.front() == "TextEdit", &driver.front());
        let Some(never) = driver.on_screen("AXButton", HINT_NEVER_LABEL).and_then(ax_frame) else {
            driver.check("the second button of a hint", false, "no button in the accessibility tree");
            return Ok(());
        };
        driver.click(never);
        let turned_off = wait_for(HINT_GOES, || (SettingsStore::new(Some(folder.join("config/config.json"))).get(HINTS_SETTING) == Some(Value::Bool(false))).then_some(()));
        driver.check("«Больше не показывать» turns the hints off", driver.went_away("first_start", "forever") && turned_off.is_some(), &format!("{:?}", driver.how("first_start")));
        driver.check("a click on a hint at the item leaves the editor in front", driver.front() == "TextEdit", &driver.front());
        Ok(())
    }

    /// The cards of the settings window: one on the page in front, one on the page of a setting, the setting it is about
    /// marked, and the checkbox that turns the hints off. Everything through the accessibility tree.
    fn hints_in_the_settings_window(driver: &Driver, binary: &Path, folder: &Path) -> Result<(), String> {
        let data = folder.join("data");
        // An update from before the switch key: a card of news waits on the page of hotkeys.
        seed_hints(&data, &json!({"last_run_version": "0.69.0"}))?;
        let mut child = start_program(binary, folder, false)?;
        driver.program.set(program_id(&child));
        *driver.data.borrow_mut() = data;
        in_the_window(driver, folder);
        let _ = child.kill();
        let _ = child.wait();
        Ok(())
    }

    fn in_the_window(driver: &Driver, folder: &Path) {
        let process = driver.program.get();
        let card = wait_for(WINDOW_APPEARS, || driver.on_screen("AXStaticText", "короткие подсказки"));
        driver.check("the card of the first page", card.is_some(), &format!("how {:?}", driver.how("hints_intro")));
        // The text of the card leads to the setting it names: the transparent button over it is the one with no title.
        let over_text = ax_windows(process).into_iter().find_map(|window| driver.ax_untitled_button(window, AX_SEARCH_DEPTH));
        driver.check("the text of the card is a button", over_text.is_some(), "");
        if let Some(over_text) = over_text {
            driver.check("the button over the text answers", ax_press(over_text), "");
        }
        let gone = wait_for(CARD_APPEARS, || (driver.on_screen("AXStaticText", "короткие подсказки").is_none() && driver.how("hints_intro") == "click").then_some(()));
        driver.check("the card goes away and its setting comes up", gone.is_some(), &driver.how("hints_intro"));
        let checkbox = wait_for(CARD_APPEARS, || driver.on_screen("AXCheckBox", HINTS_SETTING_TITLE));
        driver.check("the text of the card leads to the page of the setting", checkbox.is_some(), "");
        let marked = checkbox.and_then(ax_frame).is_some_and(|control| ax_windows(process).into_iter().any(|window| driver.ax_group_around(window, control, AX_SEARCH_DEPTH)));
        driver.check("the setting the card led to is marked", marked, "");
        // Other pages, one by one: the hotkeys page shows its card.
        let tab = driver.on_screen("AXRadioButton", "Горячие клавиши");
        driver.check("the tab of the hotkeys", tab.is_some(), "");
        if let Some(tab) = tab {
            ax_press(tab);
        }
        let news = wait_for(CARD_APPEARS, || driver.on_screen("AXStaticText", "одной клавишей — "));
        driver.check("the card of the hotkeys page", news.is_some(), &driver.how("new_0_70_switch_layout_key"));
        // The marker lies around the control the card is about: it is a group whose frame holds the control's.
        let control = driver.on_screen("AXPopUpButton", "Выключено").or_else(|| driver.on_screen("AXPopUpButton", "")).and_then(ax_frame);
        let marked = control.is_some_and(|control| {
            ax_windows(process).into_iter().any(|window| driver.ax_group_around(window, control, AX_SEARCH_DEPTH))
        });
        driver.check("the setting of the card is marked", marked, &format!("{control:?}"));
        if let Some(ok) = driver.on_screen("AXButton", HINT_OK_LABEL) {
            ax_press(ok);
        }
        let tab = driver.on_screen("AXRadioButton", "Внешний вид");
        if let Some(tab) = tab {
            ax_press(tab);
        }
        let checkbox = wait_for(CARD_APPEARS, || driver.on_screen("AXCheckBox", HINTS_SETTING_TITLE));
        driver.check("the setting of the hints in the window", checkbox.is_some(), "");
        if let Some(checkbox) = checkbox {
            ax_press(checkbox);
            let off = wait_for(HINT_GOES, || (SettingsStore::new(Some(folder.join("config/config.json"))).get(HINTS_SETTING) == Some(Value::Bool(false))).then_some(()));
            driver.check("the checkbox turns the hints off", off.is_some(), "");
        }
    }

    fn pasteboard() -> String {
        Command::new("pbpaste").output().map_or_else(|_| String::new(), |output| String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn set_pasteboard(text: &str) -> Result<(), String> {
        use std::io::Write;
        let mut child = Command::new("pbcopy").stdin(Stdio::piped()).spawn().map_err(|error| error.to_string())?;
        child.stdin.as_mut().ok_or("no input")?.write_all(text.as_bytes()).map_err(|error| error.to_string())?;
        child.wait().map_err(|error| error.to_string())?;
        Ok(())
    }

    fn typing(driver: &Driver) -> Result<(), String> {
        // A correction in the middle of a sentence.
        driver.put_text("")?;
        driver.select(0)?;
        driver.type_keys("the ghbdtn ", false)?;
        std::thread::sleep(SETTLED);
        driver.check("correction", driver.value() == "the привет ", &driver.value());
        hint_at_the_caret(driver)?;
        // Shift held for the first letter.
        driver.put_text("")?;
        driver.select(0)?;
        driver.type_keys("the Ghbdtn ", false)?;
        std::thread::sleep(SETTLED);
        driver.check("shift on the first letter", driver.value() == "the Привет ", &driver.value());
        // The real Caps Lock engaged while the word is typed.
        driver.put_text("")?;
        driver.select(0)?;
        driver.type_keys("the ", false)?;
        real_caps_lock(true)?;
        pump(0.5);
        driver.type_keys("ghbdtn ", false)?;
        std::thread::sleep(SETTLED);
        let locked = driver.value();
        real_caps_lock(false)?;
        pump(1.0);
        // The first key after the lock goes off may still carry it: a throwaway key and its erasure take that.
        driver.type_keys("x", false)?;
        driver.key(BACKSPACE, true)?;
        driver.key(BACKSPACE, false)?;
        std::thread::sleep(Duration::from_secs(1));
        driver.check("Caps Lock", locked == "the ПРИВЕТ ", &locked);
        // A correction, then the undo hotkey, twice over.
        for attempt in ["first", "second"] {
            driver.put_text("")?;
            driver.select(0)?;
            driver.type_keys("the ghbdtn ", false)?;
            std::thread::sleep(SETTLED);
            let corrected = driver.value();
            driver.chord('z', &[CONTROL, OPTION])?;
            std::thread::sleep(SETTLED);
            let undone = driver.value();
            driver.check(&format!("undo hotkey, {attempt}"), corrected == "the привет " && undone == "the ghbdtn ", &format!("{corrected:?} then {undone:?}"));
        }
        Ok(())
    }

    fn selection(driver: &Driver) -> Result<(), String> {
        const CASES: [(&str, Option<&str>, char, &str, Option<i32>); 7] = [
            ("layout", Some("ghbdtn vbh"), 'l', "привет мир", Some(1)),
            ("layout mixed", Some("привет vbh"), 'l', "привет мир", None),
            ("layout back", Some("руддщ цщкдв"), 'l', "hello world", Some(0)),
            ("case 1", Some("привет мир"), 'u', "ПРИВЕТ МИР", None),
            ("case 2 (reselected)", None, 'u', "Привет Мир", None),
            ("translit", Some("Щука"), 't', "Shchuka", None),
            ("translit back (reselected)", None, 't', "Щука", None),
        ];
        for (name, text, letter, want, want_group) in CASES {
            if let Some(text) = text {
                driver.select(0)?;
                driver.put_text(text)?;
                driver.chord('a', &[COMMAND])?;
            }
            set_pasteboard(SENTINEL)?;
            let before = driver.value();
            driver.chord(letter, &[CONTROL, OPTION])?;
            let after = driver.wait_change(&before);
            let kept = pasteboard() == SENTINEL;
            let group = driver.group();
            driver.check(&format!("selection {name}"), after == want && kept && want_group.is_none_or(|wanted| wanted == group), &format!("{before:?} became {after:?}, pasteboard kept: {kept}, group {group}"));
        }
        Ok(())
    }

    fn words(driver: &Driver) -> Result<(), String> {
        for (name, group, keys, want) in [("russian typo", 1, "нромально ", "нормально "), ("double capitals", 1, "ПРивет ", "Привет "), ("english typo off by default", 0, "recieve ", "recieve ")] {
            driver.put_text("")?;
            driver.select(group)?;
            driver.type_keys(keys, group == 1)?;
            std::thread::sleep(Duration::from_secs(2));
            driver.check(&format!("typing {name}"), driver.value() == want, &driver.value());
        }
        driver.put_text("")?;
        driver.select(1)?;
        driver.type_keys("нромально ", true)?;
        std::thread::sleep(Duration::from_millis(1500));
        let corrected = driver.value();
        driver.chord('z', &[CONTROL, OPTION])?;
        let undone = driver.wait_change(&corrected);
        driver.type_keys("нромально ", true)?;
        std::thread::sleep(Duration::from_secs(2));
        let again = driver.value();
        driver.check("the undo of a typo correction, then the word typed again stays", corrected == "нормально " && undone == "нромально " && again == "нромально нромально ", &format!("{corrected:?}, {undone:?}, {again:?}"));
        Ok(())
    }

    /// Pause has no key on a Mac: the conversion hotkey is Control+Option+K there. The word is one no correction at a pause
    /// would touch, so only the hotkey converts it.
    fn conversion(driver: &Driver) -> Result<(), String> {
        driver.put_text("")?;
        driver.select(0)?;
        driver.type_keys("the hello", false)?;
        let typed = driver.value();
        driver.chord('k', &[CONTROL, OPTION])?;
        let converted = driver.wait_change(&typed);
        driver.check("conversion hotkey", converted == "the руддщ" && driver.group() == 1, &format!("{typed:?} became {converted:?}"));
        Ok(())
    }

    /// The layout switch key (Left Shift here): a lone tap switches the layout and keeps the next word as typed; Right
    /// Shift and Shift held over a letter switch nothing.
    fn layout_key(driver: &Driver) -> Result<(), String> {
        driver.put_text("")?;
        driver.select(0)?;
        driver.key(SHIFT, true)?;
        driver.key(SHIFT, false)?;
        std::thread::sleep(Duration::from_millis(500));
        let tapped = driver.group();
        driver.check("layout key tap", tapped == 1, &format!("group {tapped}"));
        driver.type_keys("hello ", false)?;
        std::thread::sleep(SETTLED);
        driver.check("the word after the tap stays as typed", driver.value() == "руддщ ", &driver.value());
        driver.key(RIGHT_SHIFT, true)?;
        driver.key(RIGHT_SHIFT, false)?;
        driver.chord('a', &[SHIFT])?;
        std::thread::sleep(Duration::from_millis(500));
        let kept = driver.group();
        driver.check("right shift and shift with a letter", kept == 1 && driver.value() == "руддщ Ф", &format!("group {kept}, {:?}", driver.value()));
        driver.key(SHIFT, true)?;
        driver.key(SHIFT, false)?;
        std::thread::sleep(Duration::from_millis(500));
        let back = driver.group();
        driver.check("layout key tap back", back == 0, &format!("group {back}"));
        Ok(())
    }

    fn run(binary: &Path) -> Result<(), String> {
        let root = std::env::temp_dir().join(format!("keyswitch-e2e-{}", std::process::id()));
        std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        // Seen before: the first correction of this run brings the hint at the caret, and a start of this version no news.
        seed_hints(&root.join("data"), &json!({"last_run_version": VERSION}))?;
        let settings = SettingsStore::new(Some(root.join("config/config.json")));
        for (path, value) in [("general.autostart", json!(false)), ("updates.check_automatically", json!(false)), ("diagnostics.technical_logging", json!(true)), ("detection.early_switch", json!(false)), ("detection.layout_switch_key", json!("Shift_L"))] {
            settings.set(path, value, true).map_err(|error| format!("{error:?}"))?;
        }
        let mut child: Child = Command::new(binary).arg("--hidden").env("KEYSWITCH_CONFIG_DIR", root.join("config")).env("KEYSWITCH_DATA_DIR", root.join("data")).spawn().map_err(|error| error.to_string())?;
        std::thread::sleep(PROGRAM_STARTS);
        let document = root.join("document.txt").to_string_lossy().into_owned();
        std::fs::write(&document, "").map_err(|error| error.to_string())?;
        // The system's own corrections would change the text under the program's.
        let _ = Command::new("open").args(["-a", "TextEdit", &document, "--args", "-CorrectSpellingAutomatically", "NO", "-NSAutomaticCapitalizationEnabled", "NO", "-NSAutomaticSpellingCorrectionEnabled", "NO", "-NSAutomaticQuoteSubstitutionEnabled", "NO", "-NSAutomaticDashSubstitutionEnabled", "NO", "-NSAutomaticPeriodSubstitutionEnabled", "NO", "-NSAutomaticTextReplacementEnabled", "NO"]).status();
        std::thread::sleep(DOCUMENT_OPENS);
        // SAFETY: takes the state by value.
        let source = unsafe { CGEventSourceCreate(HID_SYSTEM_STATE) };
        let backend = Arc::new(MacBackend::new(Some(Arc::new(NativeMacAPI::new()))).map_err(|error| format!("{error:?}"))?);
        let driver = Driver { source, ax: NativeAccessibility::new(), backend, document, failures: std::cell::RefCell::default(), program: Cell::new(program_id(&child)), data: std::cell::RefCell::new(root.join("data")) };
        let outcome = (|| -> Result<(), String> {
            driver.release_modifiers()?;
            typing(&driver)?;
            selection(&driver)?;
            words(&driver)?;
            conversion(&driver)?;
            layout_key(&driver)?;
            driver.release_modifiers()?;
            let _ = child.kill();
            let _ = child.wait();
            hint_at_the_item(&driver, binary, &root.join("tray"))?;
            hints_in_the_settings_window(&driver, binary, &root.join("settings"))
        })();
        let _ = child.kill();
        let _ = child.wait();
        let _ = Command::new("pkill").args(["-x", "TextEdit"]).status();
        let failures = driver.failures.borrow().clone();
        if (!failures.is_empty() || outcome.is_err())
            && let Ok(log) = std::fs::read_to_string(root.join("data/keyswitch.log")) {
                println!("{}", log.lines().rev().take(30).collect::<Vec<&str>>().join("\n"));
            }
        let _ = std::fs::remove_dir_all(&root);
        outcome?;
        if failures.is_empty() { Ok(()) } else { Err(failures.join(", ")) }
    }

    pub(crate) fn main() {
        let Some(binary) = std::env::var_os("KS_E2E_BINARY") else {
            println!("KS_E2E_BINARY is not set: no program to drive, skipped");
            return;
        };
        match run(Path::new(&binary)) {
            Ok(()) => println!("MAC_E2E_OK"),
            Err(error) => {
                println!("MAC_E2E_FAILED: {error}");
                std::process::exit(1);
            }
        }
    }
}
