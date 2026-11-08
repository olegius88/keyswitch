//! The menu bar item, drawn through `AppKit`. The menu is rebuilt when `AppKit` announces it is about to open, rather than
//! whenever the state changes: every `AppKit` call then happens on the main thread, while the engine keeps changing the state
//! from its own.
#![cfg_attr(target_os = "macos", expect(unsafe_code, reason = "calls into the macOS system libraries"))]
#[cfg(target_os = "macos")]
pub use native::{StatusItem, status_item_frame};

#[cfg(target_os = "macos")]
mod native {
    use ks_cast::To as _;

    use std::ffi::c_void;
    use std::sync::{Arc, Mutex, OnceLock};

    use ks_ui::tray_model::{MenuEntry, TrayAction, TrayState, menu_entries};
    use objc2::runtime::{AnyClass, AnyObject, ClassBuilder, Sel};
    use objc2::{class, msg_send, sel};

    use crate::hint_place::Frame;
    use crate::macos_objc::{Id, string};
    use crate::settings_window::Rect;

    /// `NSVariableStatusItemLength`.
    const VARIABLE_STATUS_ITEM_LENGTH: f64 = -1.0;
    const ACTIVATION_POLICY_ACCESSORY: isize = 1;
    const CONTROL_STATE_ON: isize = 1;
    const CONTROL_STATE_OFF: isize = 0;

    type Handler = Arc<dyn Fn(TrayAction) + Send + Sync>;

    struct Shared {
        item: usize,
        menu: usize,
        target: usize,
        state: TrayState,
        entries: Vec<MenuEntry>,
        handler: Option<Handler>,
    }

    static SHARED: OnceLock<Mutex<Shared>> = OnceLock::new();

    fn shared() -> &'static Mutex<Shared> {
        SHARED.get_or_init(|| Mutex::new(Shared { item: 0, menu: 0, target: 0, state: TrayState::default(), entries: Vec::new(), handler: None }))
    }

    fn locked() -> std::sync::MutexGuard<'static, Shared> {
        shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    extern "C-unwind" fn menu_item_chosen(_this: &AnyObject, _selector: Sel, sender: *mut AnyObject) {
        // SAFETY: the sender is the menu item AppKit passes for the time of this call.
        let index: isize = unsafe { msg_send![sender, tag] };
        let (action, handler) = {
            let guard = locked();
            (usize::try_from(index).ok().and_then(|index| guard.entries.get(index)).and_then(|entry| entry.action), guard.handler.clone())
        };
        if let (Some(action), Some(handler)) = (action, handler) {
            handler(action);
        }
    }

    extern "C-unwind" fn menu_needs_update(_this: &AnyObject, _selector: Sel, _menu: *mut AnyObject) {
        rebuild_menu();
    }

    fn rebuild_menu() {
        let mut guard = locked();
        let entries = menu_entries(&guard.state);
        let (menu, target) = (guard.menu.pointer_mut::<AnyObject>(), guard.target.pointer_mut::<AnyObject>());
        if menu.is_null() {
            return;
        }
        objc2::rc::autoreleasepool(|_| {
            // SAFETY: messages on the main thread to the menu this module owns; each item is released once the menu holds it.
            unsafe {
                let _: () = msg_send![menu, removeAllItems];
                for (index, entry) in entries.iter().enumerate() {
                    if entry.separator {
                        let separator: Id = msg_send![class!(NSMenuItem), separatorItem];
                        let _: () = msg_send![menu, addItem: separator];
                        continue;
                    }
                    let action: Option<Sel> = entry.action.map(|_| sel!(keySwitchMenuItem:));
                    let allocated: Id = msg_send![class!(NSMenuItem), alloc];
                    let item: Id = msg_send![allocated, initWithTitle: string(&entry.label), action: action, keyEquivalent: string("")];
                    let _: () = msg_send![item, setTarget: target];
                    let _: () = msg_send![item, setTag: index.bits::<isize>()];
                    let _: () = msg_send![item, setEnabled: entry.enabled && entry.action.is_some()];
                    if let Some(checked) = entry.checked {
                        let _: () = msg_send![item, setState: if checked { CONTROL_STATE_ON } else { CONTROL_STATE_OFF }];
                    }
                    let _: () = msg_send![menu, addItem: item];
                    let _: () = msg_send![item, release];
                }
            }
        });
        guard.entries = entries;
    }

    /// Where the item stands on the bar, in `AppKit`'s coordinates: the frame of the window the bar gives its button. None
    /// without an item, as when its setting is off.
    #[must_use]
    pub fn status_item_frame() -> Option<Frame> {
        let item = locked().item.pointer_mut::<AnyObject>();
        if item.is_null() {
            return None;
        }
        objc2::rc::autoreleasepool(|_| {
            // SAFETY: messages on the main thread to the status item this module owns and its button.
            unsafe {
                let button: Id = msg_send![item, button];
                if button.is_null() {
                    return None;
                }
                let window: Id = msg_send![button, window];
                if window.is_null() {
                    return None;
                }
                let frame: Rect = msg_send![window, frame];
                Some(Frame { x: frame.origin.x, y: frame.origin.y, width: frame.size.width, height: frame.size.height })
            }
        })
    }

    /// The item of the menu bar; every method runs on the main thread.
    pub struct StatusItem;

    impl StatusItem {
        pub fn start(handler: Handler, state: TrayState) -> Result<StatusItem, String> {
            objc2::rc::autoreleasepool(|_| {
                // The item can come back after its setting was turned off and on; a class registers once.
                let target_class = if let Some(registered) = AnyClass::get(c"KeySwitchMenuTarget") { registered } else {
                    let mut builder = ClassBuilder::new(c"KeySwitchMenuTarget", class!(NSObject)).ok_or("не удалось создать класс-получатель для меню")?;
                    // SAFETY: the functions have the signatures of these selectors.
                    unsafe {
                        builder.add_method(sel!(keySwitchMenuItem:), { let method: extern "C-unwind" fn(_, _, _) = menu_item_chosen; method });
                        builder.add_method(sel!(menuNeedsUpdate:), { let method: extern "C-unwind" fn(_, _, _) = menu_needs_update; method });
                    }
                    builder.register()
                };
                // SAFETY: messages on the main thread to AppKit's shared objects and the objects made here, kept for the life of the program.
                unsafe {
                    let application: Id = msg_send![class!(NSApplication), sharedApplication];
                    let _: bool = msg_send![application, setActivationPolicy: ACTIVATION_POLICY_ACCESSORY];
                    let bar: Id = msg_send![class!(NSStatusBar), systemStatusBar];
                    let item: Id = msg_send![bar, statusItemWithLength: VARIABLE_STATUS_ITEM_LENGTH];
                    if item.is_null() {
                        return Err("macOS не выдала место в строке меню".to_owned());
                    }
                    // The status bar hands out an item it does not keep for us.
                    let item: Id = msg_send![item, retain];
                    let allocated: Id = msg_send![target_class, alloc];
                    let target: Id = msg_send![allocated, init];
                    let allocated: Id = msg_send![class!(NSMenu), alloc];
                    let menu: Id = msg_send![allocated, init];
                    // Without this AppKit greys out every item that has no target of its own.
                    let _: () = msg_send![menu, setAutoenablesItems: false];
                    let _: () = msg_send![menu, setDelegate: target];
                    let _: () = msg_send![item, setMenu: menu];
                    let mut guard = locked();
                    guard.item = item.to::<usize>();
                    guard.menu = menu.to::<usize>();
                    guard.target = target.to::<usize>();
                    guard.handler = Some(handler);
                }
                Ok(())
            })?;
            let tray = StatusItem;
            tray.update(state);
            Ok(tray)
        }

        /// Shows the layout on the bar; the menu is rebuilt when it opens.
        pub fn update(&self, state: TrayState) {
            let title = state.label().to_owned();
            let item = {
                let mut guard = locked();
                guard.state = state;
                guard.item.pointer_mut::<AnyObject>()
            };
            if item.is_null() {
                return;
            }
            objc2::rc::autoreleasepool(|_| {
                // SAFETY: messages on the main thread to the status item this module owns.
                unsafe {
                    let button: Id = msg_send![item, button];
                    if !button.is_null() {
                        let _: () = msg_send![button, setTitle: string(&title)];
                    }
                }
            });
        }

        #[must_use]
        pub fn state(&self) -> TrayState {
            locked().state.clone()
        }

        pub fn close(&self) {
            let mut guard = locked();
            let item = std::mem::take(&mut guard.item).pointer_mut::<AnyObject>();
            if item.is_null() {
                return;
            }
            // SAFETY: on the main thread; the item is removed from the bar and given up.
            unsafe {
                let bar: Id = msg_send![class!(NSStatusBar), systemStatusBar];
                let _: () = msg_send![bar, removeStatusItem: item];
                let _: () = msg_send![item, release];
            }
        }
    }

    const _: *const c_void = std::ptr::null();
}
