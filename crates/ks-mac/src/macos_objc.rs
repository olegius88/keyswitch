//! The few Objective-C runtime calls the macOS layers share. `AppKit` has no C interface, so reaching it means sending
//! messages, and those are written once here.
use std::ffi::{CStr, CString, c_char};

use objc2::runtime::{AnyClass, AnyObject};
use objc2::{class, msg_send};

pub type Id = *mut AnyObject;

/// An autoreleased `NSString`.
pub fn string(text: &str) -> Id {
    let text = CString::new(text.replace('\0', "")).unwrap_or_default();
    // SAFETY: the C string outlives the message, which copies it.
    unsafe { msg_send![class!(NSString), stringWithUTF8String: text.as_ptr()] }
}

/// The characters of an `NSString`.
pub fn text_of(reference: Id) -> String {
    if reference.is_null() {
        return String::new();
    }
    // SAFETY: the object is an NSString; the bytes stay valid for this conversion.
    unsafe {
        let characters: *const c_char = msg_send![reference, UTF8String];
        if characters.is_null() { String::new() } else { CStr::from_ptr(characters).to_string_lossy().into_owned() }
    }
}

/// The process the user is working in, and its name. `NSWorkspace` answers directly; the accessibility tree answers
/// only once its connection is established, which it is not when a program has just started.
#[must_use]
pub fn frontmost_application() -> (i32, String) {
    objc2::rc::autoreleasepool(|_| {
        // SAFETY: plain messages to shared objects of AppKit.
        unsafe {
            let workspace: Id = msg_send![class!(NSWorkspace), sharedWorkspace];
            let application: Id = msg_send![workspace, frontmostApplication];
            if application.is_null() {
                return (0, String::new());
            }
            let process: i32 = msg_send![application, processIdentifier];
            let name: Id = msg_send![application, localizedName];
            (process, text_of(name))
        }
    })
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    fn NSBeep();
}

/// The system's own alert sound.
pub fn beep() {
    // SAFETY: no arguments and no state of ours.
    unsafe { NSBeep() };
}

/// The global `AppleInterfaceStyle` default: `Dark` in the dark appearance, empty when it is not set.
#[must_use]
pub fn interface_style() -> String {
    objc2::rc::autoreleasepool(|_| {
        // SAFETY: plain messages to the shared defaults object.
        unsafe {
            let defaults: Id = msg_send![class!(NSUserDefaults), standardUserDefaults];
            let value: Id = msg_send![defaults, stringForKey: string("AppleInterfaceStyle")];
            text_of(value)
        }
    })
}

/// Show a file or a folder in the Finder.
#[must_use]
pub fn open_path(path: &str) -> bool {
    objc2::rc::autoreleasepool(|_| {
        // SAFETY: plain messages to shared objects of AppKit with autoreleased arguments.
        unsafe {
            let workspace: Id = msg_send![class!(NSWorkspace), sharedWorkspace];
            let url: Id = msg_send![class!(NSURL), fileURLWithPath: string(path)];
            if url.is_null() {
                return false;
            }
            let opened: bool = msg_send![workspace, openURL: url];
            opened
        }
    })
}

/// A correction shown in Notification Center. Older than its replacement, which would ask for a permission of its own
/// first; where the system no longer has it nothing is shown.
pub fn notify(title: &str, message: &str) {
    let (Some(kind), Some(center_kind)) = (AnyClass::get(c"NSUserNotification"), AnyClass::get(c"NSUserNotificationCenter")) else {
        return;
    };
    objc2::rc::autoreleasepool(|_| {
        // SAFETY: messages on the main thread; the notification is released once the centre holds it.
        unsafe {
            let notification: Id = msg_send![kind, new];
            let _: () = msg_send![notification, setTitle: string(title)];
            let _: () = msg_send![notification, setInformativeText: string(message)];
            let center: Id = msg_send![center_kind, defaultUserNotificationCenter];
            if !center.is_null() {
                let _: () = msg_send![center, deliverNotification: notification];
            }
            let _: () = msg_send![notification, release];
        }
    });
}
