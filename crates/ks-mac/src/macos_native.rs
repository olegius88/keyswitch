//! The macOS half of the keyboard backend: nothing here decides anything, it only calls the system. Every decision lives
//! in `macos_backend`, which is verified on any platform.
use std::collections::HashMap;
use std::ffi::{CString, c_char, c_long, c_ulong, c_void};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;

use ks_backend::constants::{ALT_MASK, CONTROL_MASK, LOCK_MASK, SHIFT_MASK};
use ks_backend::{BackendError, ScreenAnchor};
use objc2::msg_send;

use crate::constants::*;
use crate::macos_backend::{MacAPI, MacBackendError, MacPasteboardAPI, NativeInput, NativeKeyEvent, PasteboardContents, TapListener, TapReady, modifier_pressed};
use crate::macos_context::{AccessibilityAPI, SelectionAccessibilityAPI};
use crate::macos_main_thread::MainThreadRunner;
use crate::macos_objc::{Id, frontmost_application, string, text_of};

type CFRef = *const c_void;

#[repr(C)]
#[derive(Default)]
struct CFRange {
    location: c_long,
    length: c_long,
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(reference: CFRef);
    fn CFRetain(reference: CFRef) -> CFRef;
    fn CFArrayGetCount(array: CFRef) -> c_long;
    fn CFArrayGetValueAtIndex(array: CFRef, index: c_long) -> CFRef;
    fn CFStringGetCString(string: CFRef, buffer: *mut c_char, size: c_long, encoding: u32) -> bool;
    fn CFStringGetLength(string: CFRef) -> c_long;
    fn CFStringGetMaximumSizeForEncoding(length: c_long, encoding: u32) -> c_long;
    fn CFDataGetBytePtr(data: CFRef) -> *const u8;
    fn CFDictionaryGetValue(dictionary: CFRef, key: CFRef) -> CFRef;
    fn CFNumberGetValue(number: CFRef, kind: c_long, value: *mut c_void) -> bool;
    fn CFDictionaryCreate(allocator: CFRef, keys: *const CFRef, values: *const CFRef, count: c_long, key_callbacks: *const c_void, value_callbacks: *const c_void) -> CFRef;
    fn CFStringCreateWithCString(allocator: CFRef, text: *const c_char, encoding: u32) -> CFRef;
    fn CFURLCreateWithString(allocator: CFRef, text: CFRef, base: CFRef) -> CFRef;
    fn CFMachPortCreateRunLoopSource(allocator: CFRef, port: CFRef, order: c_long) -> CFRef;
    fn CFRunLoopGetCurrent() -> CFRef;
    fn CFRunLoopAddSource(run_loop: CFRef, source: CFRef, mode: CFRef);
    fn CFRunLoopRemoveSource(run_loop: CFRef, source: CFRef, mode: CFRef);
    fn CFRunLoopRunInMode(mode: CFRef, seconds: f64, return_after_source: bool) -> i32;
    fn CFGetTypeID(reference: CFRef) -> c_ulong;
    fn CFStringGetTypeID() -> c_ulong;
    static kCFRunLoopCommonModes: CFRef;
    static kCFRunLoopDefaultMode: CFRef;
    static kCFBooleanTrue: CFRef;
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;
}

type TapCallback = extern "C" fn(CFRef, u32, *mut c_void, *mut c_void) -> *mut c_void;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(tap: u32, place: u32, options: u32, mask: u64, callback: TapCallback, info: *mut c_void) -> CFRef;
    fn CGEventTapEnable(tap: CFRef, enable: bool);
    fn CGEventGetIntegerValueField(event: CFRef, field: u32) -> i64;
    fn CGEventSetIntegerValueField(event: CFRef, field: u32, value: i64);
    fn CGEventGetFlags(event: CFRef) -> u64;
    fn CGEventSetFlags(event: CFRef, flags: u64);
    fn CGEventGetTimestamp(event: CFRef) -> u64;
    fn CGEventCreate(source: CFRef) -> CFRef;
    fn CGEventCreateKeyboardEvent(source: CFRef, keycode: u16, down: bool) -> CFRef;
    fn CGEventPost(tap: u32, event: CFRef);
    fn CGWindowListCopyWindowInfo(option: u32, window: u32) -> CFRef;
    static kCGWindowNumber: CFRef;
    static kCGWindowLayer: CFRef;
    static kCGWindowOwnerPID: CFRef;
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn TISCreateInputSourceList(properties: CFRef, include_all: bool) -> CFRef;
    fn TISCopyCurrentKeyboardInputSource() -> CFRef;
    fn TISGetInputSourceProperty(source: CFRef, key: CFRef) -> CFRef;
    fn TISSelectInputSource(source: CFRef) -> i32;
    fn LMGetKbdType() -> u8;
    fn UCKeyTranslate(layout: *const u8, keycode: u16, action: u16, modifiers: u32, keyboard: u32, options: u32, dead: *mut u32, capacity: c_ulong, length: *mut c_ulong, buffer: *mut u16) -> i32;
    static kTISPropertyInputSourceID: CFRef;
    static kTISPropertyUnicodeKeyLayoutData: CFRef;
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: CFRef) -> bool;
    fn LSOpenCFURLRef(url: CFRef, launched: *mut CFRef) -> i32;
    fn AXUIElementCreateSystemWide() -> CFRef;
    fn AXUIElementCopyAttributeValue(element: CFRef, name: CFRef, value: *mut CFRef) -> i32;
    fn AXValueGetValue(value: CFRef, kind: u32, output: *mut c_void) -> bool;
    fn AXUIElementSetAttributeValue(element: CFRef, name: CFRef, value: CFRef) -> i32;
    fn AXUIElementIsAttributeSettable(element: CFRef, name: CFRef, settable: *mut bool) -> i32;
    static kAXTrustedCheckOptionPrompt: CFRef;
}

unsafe extern "C" {
    static _dispatch_main_q: c_void;
    fn dispatch_async_f(queue: *const c_void, context: *mut c_void, work: extern "C" fn(*mut c_void));
    fn pthread_main_np() -> i32;
    fn getpid() -> i32;
}

/// The pane of System Settings that holds the switch KeySwitch needs. macOS has no call that grants the permission; the
/// most a program may do is ask, and then take the user to the place where the answer is given.
const ACCESSIBILITY_SETTINGS_URL: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";

// The Text Input Sources calls run on the main thread only: HIToolbox ends the process when the engine's worker or the
// tap's thread makes one. libdispatch carries a job there.
fn is_main_thread() -> bool {
    // SAFETY: a plain query of this thread.
    unsafe { pthread_main_np() != 0 }
}

extern "C" fn run_on_main_queue(context: *mut c_void) {
    MAIN_THREAD.run(context as u64);
}

fn post_to_main_queue(number: u64) {
    // SAFETY: the main queue lives as long as the process and the function takes the number as its context.
    unsafe { dispatch_async_f(&raw const _dispatch_main_q, number as *mut c_void, run_on_main_queue) };
}

pub static MAIN_THREAD: MainThreadRunner = MainThreadRunner::new(is_main_thread, post_to_main_queue);

/// Work for the main thread, without waiting for it: what AppKit may only be told there.
pub fn run_on_main(action: Box<dyn FnOnce() + Send>) {
    extern "C" fn trampoline(context: *mut c_void) {
        // SAFETY: the box was leaked below for this one call.
        let action = unsafe { Box::from_raw(context.cast::<Box<dyn FnOnce() + Send>>()) };
        action();
    }
    let context = Box::into_raw(Box::new(action));
    // SAFETY: the queue lives as long as the process; the trampoline takes the box back exactly once.
    unsafe { dispatch_async_f(&raw const _dispatch_main_q, context.cast(), trampoline) };
}

fn text(reference: CFRef) -> String {
    if reference.is_null() {
        return String::new();
    }
    // SAFETY: `reference` is a CFString; the buffer holds the size asked for.
    unsafe {
        // A field's text is as long as the field: a fixed buffer refused every value over about 255 Cyrillic characters.
        let needed = CFStringGetMaximumSizeForEncoding(CFStringGetLength(reference), CF_STRING_ENCODING_UTF8) + 1;
        if needed > CF_STRING_MAX_BYTES as c_long {
            return String::new();
        }
        let mut buffer = vec![0u8; needed.max(1) as usize];
        if !CFStringGetCString(reference, buffer.as_mut_ptr().cast(), buffer.len() as c_long, CF_STRING_ENCODING_UTF8) {
            return String::new();
        }
        let end = buffer.iter().position(|byte| *byte == 0).unwrap_or(buffer.len());
        String::from_utf8_lossy(&buffer[..end]).into_owned()
    }
}

fn number(dictionary: CFRef, key: CFRef) -> i32 {
    let mut result = 0i32;
    // SAFETY: the dictionary is valid for this call and the value, if any, a number.
    unsafe {
        let value = CFDictionaryGetValue(dictionary, key);
        if value.is_null() || !CFNumberGetValue(value, c_long::from(CF_NUMBER_SINT32_TYPE as i32), (&raw mut result).cast()) {
            return 0;
        }
    }
    result
}

fn translate_modifiers(state: u32) -> u32 {
    let mut modifiers = 0;
    if state & SHIFT_MASK != 0 {
        modifiers |= UC_MODIFIER_SHIFT;
    }
    // Without it a word typed under Caps Lock read as lower case, the field showed capitals, and the check that the field
    // still holds the typed word refused every such correction.
    if state & LOCK_MASK != 0 {
        modifiers |= UC_MODIFIER_ALPHA_LOCK;
    }
    if state & ALT_MASK != 0 {
        modifiers |= UC_MODIFIER_OPTION;
    }
    if state & CONTROL_MASK != 0 {
        modifiers |= UC_MODIFIER_CONTROL;
    }
    modifiers
}

#[derive(Default)]
struct Sources {
    // References kept as numbers: they are retained for the life of the table and used on the main thread.
    sources: HashMap<String, usize>,
    layouts: HashMap<String, usize>,
    order: Vec<String>,
}

#[derive(Default)]
struct State {
    sources: Mutex<Sources>,
    current_source: Mutex<String>,
    tap: AtomicUsize,
    tap_source: AtomicUsize,
    stop: AtomicBool,
    tap_thread: Mutex<Option<ThreadId>>,
}

fn locked<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl State {
    fn refresh_sources(&self) -> Result<(), MacBackendError> {
        // SAFETY: the list is released below; what is kept is retained first.
        unsafe {
            let array = TISCreateInputSourceList(std::ptr::null(), false);
            if array.is_null() {
                return Err(BackendError::Failed("macOS не вернула список источников ввода".to_owned()));
            }
            let mut found = Sources::default();
            for index in 0..CFArrayGetCount(array) {
                let source = CFArrayGetValueAtIndex(array, index);
                let layout = TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData);
                if layout.is_null() {
                    continue;
                }
                let identifier = text(TISGetInputSourceProperty(source, kTISPropertyInputSourceID));
                if identifier.is_empty() || found.sources.contains_key(&identifier) {
                    continue;
                }
                // The array owns what it holds; these have to outlive it.
                found.sources.insert(identifier.clone(), CFRetain(source) as usize);
                found.layouts.insert(identifier.clone(), CFRetain(layout) as usize);
                found.order.push(identifier);
            }
            CFRelease(array);
            let mut table = locked(&self.sources);
            for reference in table.sources.values().chain(table.layouts.values()) {
                CFRelease(*reference as CFRef);
            }
            *table = found;
        }
        Ok(())
    }
}

pub struct NativeMacAPI {
    state: Arc<State>,
}

impl Default for NativeMacAPI {
    fn default() -> Self {
        Self::new()
    }
}

impl NativeMacAPI {
    pub fn new() -> Self {
        NativeMacAPI { state: Arc::new(State::default()) }
    }

    /// The window id, its owning process and its program name.
    fn front_window(&self) -> (i64, i32, String) {
        let (focused, name) = frontmost_application();
        if focused == 0 {
            // Naming the wrong program is worse than naming none: the engine would apply another application's
            // exclusions to what is typed here.
            return (0, 0, String::new());
        }
        // SAFETY: the list is released below and its entries read only before that.
        unsafe {
            let info = CGWindowListCopyWindowInfo(WINDOW_LIST_ON_SCREEN_ONLY | WINDOW_LIST_EXCLUDE_DESKTOP, NULL_WINDOW_ID);
            if !info.is_null() {
                for index in 0..CFArrayGetCount(info) {
                    let entry = CFArrayGetValueAtIndex(info, index);
                    if number(entry, kCGWindowLayer) != NORMAL_WINDOW_LAYER as i32 {
                        continue;
                    }
                    if number(entry, kCGWindowOwnerPID) != focused {
                        continue;
                    }
                    let window = number(entry, kCGWindowNumber);
                    CFRelease(info);
                    return (i64::from(window), focused, name);
                }
                CFRelease(info);
            }
        }
        // A program with no ordinary window (a menu bar agent, a full screen view) is still the one being typed into.
        (0, focused, name)
    }
}

/// The listener of the running tap: the callback gets no closure, so it reads it here.
static TAP_LISTENER: Mutex<Option<TapListener>> = Mutex::new(None);

extern "C" fn on_event(_proxy: CFRef, event_type: u32, event: *mut c_void, _info: *mut c_void) -> *mut c_void {
    let Some(listener) = locked(&TAP_LISTENER).clone() else { return event };
    if event_type == EVENT_TAP_DISABLED_BY_TIMEOUT || event_type == EVENT_TAP_DISABLED_BY_USER_INPUT {
        listener(NativeKeyEvent { pressed: false, keycode: 0, timestamp: 0, injected: false, replayed: false, event_type, pointer: false });
        return event;
    }
    // SAFETY: the system hands a valid event for the time of this call.
    unsafe {
        let timestamp = CGEventGetTimestamp(event) as i64;
        if event_type == EVENT_LEFT_MOUSE_DOWN || event_type == EVENT_RIGHT_MOUSE_DOWN || event_type == EVENT_OTHER_MOUSE_DOWN {
            listener(NativeKeyEvent { pressed: true, keycode: 0, timestamp, injected: false, replayed: false, event_type: 0, pointer: true });
            return event;
        }
        let mark = CGEventGetIntegerValueField(event, EVENT_SOURCE_USER_DATA_FIELD);
        let keycode = CGEventGetIntegerValueField(event, KEYBOARD_EVENT_KEYCODE_FIELD) as u32;
        // A modifier comes as a flags change, which says nothing about up or down by itself.
        let pressed = if event_type == EVENT_FLAGS_CHANGED { modifier_pressed(keycode, CGEventGetFlags(event)) } else { event_type == EVENT_KEY_DOWN };
        let native = NativeKeyEvent { pressed, keycode, timestamp, injected: mark == EVENT_MARK_INJECTED, replayed: mark == EVENT_MARK_REPLAYED, event_type, pointer: false };
        if listener(native) { std::ptr::null_mut() } else { event }
    }
}

impl MacAPI for NativeMacAPI {
    fn input_sources(&self) -> Result<Vec<String>, MacBackendError> {
        let state = self.state.clone();
        MAIN_THREAD.call(
            move || {
                state.refresh_sources()?;
                Ok(locked(&state.sources).order.clone())
            },
            Ok(locked(&self.state.sources).order.clone()),
            MACOS_MAIN_THREAD_WAIT_SECONDS,
        )
    }

    fn current_input_source(&self) -> String {
        // The tap's callback must stay short: past its brief wait it takes the layout read last.
        let on_tap = *locked(&self.state.tap_thread) == Some(std::thread::current().id());
        let wait = if on_tap { MACOS_TAP_MAIN_THREAD_WAIT_SECONDS } else { MACOS_MAIN_THREAD_WAIT_SECONDS };
        let read = MAIN_THREAD.call(
            || {
                // SAFETY: the copy is released after its name is read.
                unsafe {
                    let source = TISCopyCurrentKeyboardInputSource();
                    if source.is_null() {
                        return Some(String::new());
                    }
                    let identifier = text(TISGetInputSourceProperty(source, kTISPropertyInputSourceID));
                    CFRelease(source);
                    Some(identifier)
                }
            },
            None,
            wait,
        );
        let mut current = locked(&self.state.current_source);
        if let Some(identifier) = read {
            *current = identifier;
        }
        current.clone()
    }

    fn select_input_source(&self, identifier: &str) -> bool {
        let (state, wanted) = (self.state.clone(), identifier.to_owned());
        let selected = MAIN_THREAD.call(
            move || {
                let mut source = locked(&state.sources).sources.get(&wanted).copied();
                if source.is_none() {
                    let _ = state.refresh_sources();
                    source = locked(&state.sources).sources.get(&wanted).copied();
                }
                // SAFETY: the reference is retained in the table.
                source.is_some_and(|source| unsafe { TISSelectInputSource(source as CFRef) } == 0)
            },
            false,
            MACOS_LAYOUT_SWITCH_WAIT_SECONDS,
        );
        if selected {
            *locked(&self.state.current_source) = identifier.to_owned();
        }
        selected
    }

    fn translate_key(&self, keycode: u32, state: u32, source: &str) -> String {
        let mut layout = locked(&self.state.sources).layouts.get(source).copied();
        if layout.is_none() {
            let shared = self.state.clone();
            let _ = MAIN_THREAD.call(move || shared.refresh_sources().is_ok(), false, MACOS_MAIN_THREAD_WAIT_SECONDS);
            layout = locked(&self.state.sources).layouts.get(source).copied();
        }
        let Some(layout) = layout else { return String::new() };
        // SAFETY: the layout data is retained in the table; the buffers outlive the call.
        unsafe {
            let pointer = CFDataGetBytePtr(layout as CFRef);
            if pointer.is_null() {
                return String::new();
            }
            let (mut dead, mut length) = (0u32, 0 as c_ulong);
            let mut buffer = [0u16; UC_KEY_TRANSLATE_BUFFER_CHARACTERS];
            let status = UCKeyTranslate(pointer, keycode as u16, UC_KEY_ACTION_DISPLAY, translate_modifiers(state), u32::from(LMGetKbdType()), UC_KEY_TRANSLATE_NO_DEAD_KEYS_MASK, &mut dead, buffer.len() as c_ulong, &mut length, buffer.as_mut_ptr());
            if status != 0 || length == 0 {
                return String::new();
            }
            String::from_utf16_lossy(&buffer[..length as usize])
        }
    }

    fn post_inputs(&self, inputs: &[NativeInput]) -> usize {
        let mut posted = 0;
        for item in inputs {
            // SAFETY: the event is created, posted and released here.
            unsafe {
                let event = CGEventCreateKeyboardEvent(std::ptr::null(), item.keycode as u16, item.pressed);
                if event.is_null() {
                    break;
                }
                if item.shift {
                    CGEventSetFlags(event, CGEventGetFlags(event) | EVENT_FLAG_SHIFT | EVENT_FLAG_DEVICE_LEFT_SHIFT);
                }
                if item.command {
                    CGEventSetFlags(event, CGEventGetFlags(event) | EVENT_FLAG_COMMAND | EVENT_FLAG_DEVICE_LEFT_COMMAND);
                }
                CGEventSetIntegerValueField(event, EVENT_SOURCE_USER_DATA_FIELD, if item.replayed { EVENT_MARK_REPLAYED } else { EVENT_MARK_INJECTED });
                CGEventPost(SESSION_EVENT_TAP, event);
                CFRelease(event);
            }
            posted += 1;
        }
        posted
    }

    fn active_application(&self) -> String {
        self.front_window().2
    }

    fn focused_window(&self) -> i64 {
        self.front_window().0
    }

    fn window_process_id(&self, window: i64) -> i32 {
        let (number, process, _) = self.front_window();
        if number == window { process } else { 0 }
    }

    fn current_process_id(&self) -> i32 {
        // SAFETY: a plain query of this process.
        unsafe { getpid() }
    }

    fn input_anchor(&self) -> Option<ScreenAnchor> {
        None
    }

    fn caps_lock_enabled(&self) -> bool {
        // SAFETY: the event is created, read and released here.
        unsafe {
            let event = CGEventCreate(std::ptr::null());
            if event.is_null() {
                return false;
            }
            let on = CGEventGetFlags(event) & EVENT_FLAG_ALPHA_SHIFT != 0;
            CFRelease(event);
            on
        }
    }

    fn accessibility_trusted(&self, prompt: bool) -> bool {
        // SAFETY: the options live for the call and are released after it; the callbacks are the system's own.
        unsafe {
            if !prompt {
                return AXIsProcessTrusted();
            }
            let (keys, values) = ([kAXTrustedCheckOptionPrompt], [kCFBooleanTrue]);
            let options = CFDictionaryCreate(std::ptr::null(), keys.as_ptr(), values.as_ptr(), 1, &raw const kCFTypeDictionaryKeyCallBacks, &raw const kCFTypeDictionaryValueCallBacks);
            let trusted = AXIsProcessTrustedWithOptions(options);
            if !options.is_null() {
                CFRelease(options);
            }
            trusted
        }
    }

    fn open_accessibility_settings(&self) -> bool {
        let Ok(address) = CString::new(ACCESSIBILITY_SETTINGS_URL) else { return false };
        // SAFETY: the created objects are released here.
        unsafe {
            let text = CFStringCreateWithCString(std::ptr::null(), address.as_ptr(), CF_STRING_ENCODING_UTF8);
            if text.is_null() {
                return false;
            }
            let url = CFURLCreateWithString(std::ptr::null(), text, std::ptr::null());
            CFRelease(text);
            if url.is_null() {
                return false;
            }
            let opened = LSOpenCFURLRef(url, std::ptr::null_mut()) == 0;
            CFRelease(url);
            opened
        }
    }

    fn layout_follows_window(&self) -> bool {
        false
    }

    fn run_event_tap(&self, listener: TapListener, ready: TapReady) -> Result<(), MacBackendError> {
        *locked(&TAP_LISTENER) = Some(listener);
        self.state.stop.store(false, Ordering::SeqCst);
        *locked(&self.state.tap_thread) = Some(std::thread::current().id());
        let mask = (1u64 << EVENT_KEY_DOWN) | (1u64 << EVENT_KEY_UP) | (1u64 << EVENT_FLAGS_CHANGED) | (1u64 << EVENT_LEFT_MOUSE_DOWN) | (1u64 << EVENT_RIGHT_MOUSE_DOWN) | (1u64 << EVENT_OTHER_MOUSE_DOWN);
        // SAFETY: the callback is a plain function for the life of the process; the tap and its source are released below.
        unsafe {
            let tap = CGEventTapCreate(SESSION_EVENT_TAP, HEAD_INSERT_EVENT_TAP, EVENT_TAP_OPTION_DEFAULT, mask, on_event, std::ptr::null_mut());
            if tap.is_null() {
                *locked(&TAP_LISTENER) = None;
                return Err(BackendError::Failed("macOS отказала в перехвате клавиатуры: разрешите KeySwitch в разделе «Конфиденциальность и безопасность» → «Универсальный доступ»".to_owned()));
            }
            self.state.tap.store(tap as usize, Ordering::SeqCst);
            let source = CFMachPortCreateRunLoopSource(std::ptr::null(), tap, 0);
            self.state.tap_source.store(source as usize, Ordering::SeqCst);
            CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopCommonModes);
            CGEventTapEnable(tap, true);
            ready();
            while !self.state.stop.load(Ordering::SeqCst) {
                CFRunLoopRunInMode(kCFRunLoopDefaultMode, EVENT_TAP_RUN_LOOP_SLICE_SECONDS, false);
            }
            CGEventTapEnable(tap, false);
            CFRunLoopRemoveSource(CFRunLoopGetCurrent(), source, kCFRunLoopCommonModes);
            CFRelease(source);
            self.state.tap_source.store(0, Ordering::SeqCst);
            self.state.tap.store(0, Ordering::SeqCst);
            CFRelease(tap);
        }
        *locked(&TAP_LISTENER) = None;
        Ok(())
    }

    fn enable_event_tap(&self) {
        let tap = self.state.tap.load(Ordering::SeqCst);
        if tap != 0 {
            // SAFETY: the tap is alive until `run_event_tap` clears the number.
            unsafe { CGEventTapEnable(tap as CFRef, true) };
        }
    }

    fn stop_event_tap(&self) {
        self.state.stop.store(true, Ordering::SeqCst);
    }

    fn pasteboard(&self) -> Option<&dyn MacPasteboardAPI> {
        Some(self)
    }
}

// The pasteboard of the selection hotkeys' paste. NSPasteboard is reached on the main thread like the input sources, inside
// an autorelease pool: the convenience objects it hands out would otherwise leak on the engine's thread.
fn general_pasteboard() -> Id {
    // SAFETY: a class method returning the shared object.
    unsafe { msg_send![objc2::class!(NSPasteboard), generalPasteboard] }
}

fn change_count() -> i64 {
    // SAFETY: a plain message to the shared pasteboard.
    unsafe { msg_send![general_pasteboard(), changeCount] }
}

fn objects_of(array: Id) -> Vec<Id> {
    if array.is_null() {
        return Vec::new();
    }
    // SAFETY: the object is an NSArray, read while the surrounding pool keeps it.
    unsafe {
        let count: usize = msg_send![array, count];
        (0..count).map(|index| msg_send![array, objectAtIndex: index]).collect()
    }
}

fn read_pasteboard_now(limit_bytes: usize) -> PasteboardContents {
    objc2::rc::autoreleasepool(|_| {
        let (mut items, mut total) = (Vec::new(), 0usize);
        // SAFETY: messages to objects the pool keeps; the bytes are copied before it drains.
        unsafe {
            let listed: Id = msg_send![general_pasteboard(), pasteboardItems];
            for item in objects_of(listed) {
                let mut kinds = Vec::new();
                let types: Id = msg_send![item, types];
                for kind in objects_of(types) {
                    let data: Id = msg_send![item, dataForType: kind];
                    let size: usize = if data.is_null() { 0 } else { msg_send![data, length] };
                    total += size;
                    if data.is_null() || total > limit_bytes {
                        return PasteboardContents { items, complete: false };
                    }
                    let bytes: *const u8 = msg_send![data, bytes];
                    kinds.push((text_of(kind), if size == 0 { Vec::new() } else { std::slice::from_raw_parts(bytes, size).to_vec() }));
                }
                items.push(kinds);
            }
        }
        PasteboardContents { items, complete: true }
    })
}

fn write_pasteboard_now(contents: &PasteboardContents) -> bool {
    objc2::rc::autoreleasepool(|_| {
        // SAFETY: messages to objects the pool keeps; the alloc'd items are given up once the array holds them.
        unsafe {
            let board = general_pasteboard();
            let _: i64 = msg_send![board, clearContents];
            if contents.items.is_empty() {
                return true;
            }
            let objects: Id = msg_send![objc2::class!(NSMutableArray), array];
            for kinds in &contents.items {
                let allocated: Id = msg_send![objc2::class!(NSPasteboardItem), alloc];
                let item: Id = msg_send![allocated, init];
                for (kind, data) in kinds {
                    let bytes: Id = msg_send![objc2::class!(NSData), dataWithBytes: data.as_ptr().cast::<c_void>(), length: data.len()];
                    let _: bool = msg_send![item, setData: bytes, forType: string(kind)];
                }
                let _: () = msg_send![objects, addObject: item];
                let _: () = msg_send![item, release];
            }
            msg_send![board, writeObjects: objects]
        }
    })
}

fn put_text_now(text_value: &str) -> Option<i64> {
    objc2::rc::autoreleasepool(|_| {
        // SAFETY: messages to the shared pasteboard with an autoreleased string.
        unsafe {
            let board = general_pasteboard();
            let _: i64 = msg_send![board, clearContents];
            let written: bool = msg_send![board, setString: string(text_value), forType: string("public.utf8-plain-text")];
            written.then(change_count)
        }
    })
}

impl MacPasteboardAPI for NativeMacAPI {
    fn pasteboard_change_count(&self) -> i64 {
        MAIN_THREAD.call(change_count, -1, SELECTION_CLIPBOARD_TIMEOUT_SECONDS)
    }

    fn read_pasteboard(&self, limit_bytes: usize) -> PasteboardContents {
        MAIN_THREAD.call(move || read_pasteboard_now(limit_bytes), PasteboardContents { items: Vec::new(), complete: false }, SELECTION_CLIPBOARD_TIMEOUT_SECONDS)
    }

    fn write_pasteboard(&self, contents: &PasteboardContents) -> bool {
        let contents = contents.clone();
        MAIN_THREAD.call(move || write_pasteboard_now(&contents), false, SELECTION_CLIPBOARD_TIMEOUT_SECONDS)
    }

    fn put_text(&self, text_value: &str) -> Option<i64> {
        let text_value = text_value.to_owned();
        MAIN_THREAD.call(move || put_text_now(&text_value), None, SELECTION_CLIPBOARD_TIMEOUT_SECONDS)
    }
}

/// The accessibility tree, for reading what stands before the caret. Attribute names are turned into CoreFoundation strings
/// once and kept: a read happens on every word the user finishes.
pub struct NativeAccessibility {
    system: usize,
    names: Mutex<HashMap<String, usize>>,
}

impl Default for NativeAccessibility {
    fn default() -> Self {
        Self::new()
    }
}

impl NativeAccessibility {
    pub fn new() -> Self {
        // SAFETY: creates the system-wide element, kept for the life of this object.
        NativeAccessibility { system: unsafe { AXUIElementCreateSystemWide() } as usize, names: Mutex::new(HashMap::new()) }
    }

    fn name(&self, name: &str) -> CFRef {
        let mut names = locked(&self.names);
        if let Some(known) = names.get(name) {
            return *known as CFRef;
        }
        let Ok(text) = CString::new(name) else { return std::ptr::null() };
        // SAFETY: the created string is kept in the table for the life of this object.
        let created = unsafe { CFStringCreateWithCString(std::ptr::null(), text.as_ptr(), CF_STRING_ENCODING_UTF8) };
        names.insert(name.to_owned(), created as usize);
        created
    }

    /// The attribute's value, owned by the caller, or null.
    fn attribute(&self, element: usize, name: &str) -> CFRef {
        let key = self.name(name);
        if element == 0 || key.is_null() {
            return std::ptr::null();
        }
        let mut value: CFRef = std::ptr::null();
        // SAFETY: the element and the key are valid; the value is written by the call.
        let status = unsafe { AXUIElementCopyAttributeValue(element as CFRef, key, &mut value) };
        if status == AX_SUCCESS { value } else { std::ptr::null() }
    }
}

impl AccessibilityAPI for NativeAccessibility {
    fn focused_element(&self) -> usize {
        self.attribute(self.system, "AXFocusedUIElement") as usize
    }

    fn string_attribute(&self, element: usize, name: &str) -> Option<String> {
        let value = self.attribute(element, name);
        if value.is_null() {
            return None;
        }
        // SAFETY: the value is ours to release; an attribute may hold a number or a structure, so its type is asked first.
        unsafe {
            let result = (CFGetTypeID(value) == CFStringGetTypeID()).then(|| text(value));
            CFRelease(value);
            result
        }
    }

    fn range_attribute(&self, element: usize, name: &str) -> Option<(i64, i64)> {
        let value = self.attribute(element, name);
        if value.is_null() {
            return None;
        }
        let mut span = CFRange::default();
        // SAFETY: the value is ours to release; the structure is written by the call.
        unsafe {
            let read = AXValueGetValue(value, AX_VALUE_TYPE_CF_RANGE, (&raw mut span).cast());
            CFRelease(value);
            read.then_some((span.location as i64, span.length as i64))
        }
    }

    fn release(&self, element: usize) {
        if element != 0 {
            // SAFETY: the caller owns the element it got from this object.
            unsafe { CFRelease(element as CFRef) };
        }
    }
}

impl SelectionAccessibilityAPI for NativeAccessibility {
    fn attribute_settable(&self, element: usize, name: &str) -> bool {
        let key = self.name(name);
        if element == 0 || key.is_null() {
            return false;
        }
        let mut settable = false;
        // SAFETY: the element and the key are valid; the answer is written by the call.
        let status = unsafe { AXUIElementIsAttributeSettable(element as CFRef, key, &mut settable) };
        status == AX_SUCCESS && settable
    }

    fn set_string_attribute(&self, element: usize, name: &str, value: &str) -> bool {
        let key = self.name(name);
        let Ok(content) = CString::new(value) else { return false };
        if element == 0 || key.is_null() {
            return false;
        }
        // SAFETY: the created string is released after the call.
        unsafe {
            let created = CFStringCreateWithCString(std::ptr::null(), content.as_ptr(), CF_STRING_ENCODING_UTF8);
            if created.is_null() {
                return false;
            }
            let status = AXUIElementSetAttributeValue(element as CFRef, key, created);
            CFRelease(created);
            status == AX_SUCCESS
        }
    }
}

/// The name `macos_backend` asks for on macOS.
pub type NativeAccessibilityAPI = NativeAccessibility;
