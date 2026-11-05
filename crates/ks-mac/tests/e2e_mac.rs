//! The real thing on a Mac: the program, TextEdit in front, keys posted at the HID level, the document read back through
//! the accessibility tree. Started from a Terminal that holds the Accessibility permission, by `packaging/e2e-mac.command`,
//! with `KS_E2E_BINARY` naming the program; otherwise, and on other systems, it says so and passes.

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
    use std::ffi::{CString, c_void};
    use std::path::Path;
    use std::process::{Child, Command, Stdio};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use ks_backend::InputBackend;
    use ks_config::config::SettingsStore;
    use ks_mac::macos_backend::MacBackend;
    use ks_mac::macos_context::{AccessibilityAPI, SelectionAccessibilityAPI};
    use ks_mac::macos_native::{NativeAccessibility, NativeMacAPI};
    use ks_mac::macos_objc::frontmost_application;
    use serde_json::json;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFRunLoopDefaultMode: *const c_void;
        fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after_source: bool) -> i32;
        fn CFRelease(object: *const c_void);
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceCreate(state: i32) -> *const c_void;
        fn CGEventCreateKeyboardEvent(source: *const c_void, keycode: u16, down: bool) -> *const c_void;
        fn CGEventPost(tap: u32, event: *const c_void);
    }

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOServiceMatching(name: *const std::ffi::c_char) -> *const c_void;
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

    /// Lets HIToolbox hear about layout changes: without a run loop this process keeps a stale one.
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
            let status = IOServiceOpen(service, mach_task_self_, IOHID_PARAM_CONNECT, &mut connect);
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
    }

    impl Driver {
        /// Stops the run unless TextEdit is in front: no key may reach another program.
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

    fn run(binary: &Path) -> Result<(), String> {
        let root = std::env::temp_dir().join(format!("keyswitch-e2e-{}", std::process::id()));
        std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        let settings = SettingsStore::new(Some(root.join("config/config.json")));
        for (path, value) in [("general.autostart", json!(false)), ("updates.check_automatically", json!(false)), ("diagnostics.technical_logging", json!(true)), ("detection.early_switch", json!(false))] {
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
        let driver = Driver { source, ax: NativeAccessibility::new(), backend, document, failures: Default::default() };
        let outcome = (|| -> Result<(), String> {
            driver.release_modifiers()?;
            driver.check("the document can be read", driver.attribute("AXValue").is_some(), "");
            typing(&driver)?;
            selection(&driver)?;
            words(&driver)?;
            driver.release_modifiers()
        })();
        let _ = child.kill();
        let _ = child.wait();
        let _ = Command::new("pkill").args(["-x", "TextEdit"]).status();
        let failures = driver.failures.borrow().clone();
        if !failures.is_empty() || outcome.is_err() {
            if let Ok(log) = std::fs::read_to_string(root.join("data/keyswitch.log")) {
                println!("{}", log.lines().rev().take(30).collect::<Vec<&str>>().join("\n"));
            }
        }
        let _ = std::fs::remove_dir_all(&root);
        outcome?;
        if failures.is_empty() { Ok(()) } else { Err(failures.join(", ")) }
    }

    pub fn main() {
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
