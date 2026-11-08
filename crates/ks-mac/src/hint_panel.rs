//! The one-time hints on a Mac: a floating panel at the caret or under the menu bar item that never takes the keyboard.
//!
//! The panel is the rule prompt's kind, non-activating and floating (`settings_window::learning_prompt`), and one thing
//! more: it refuses to become the key window, so that a key typed while it is on screen, Enter and Esc included, stays with
//! the field it was going to. What goes where is decided in `hint_place`; `desktop_app` feeds this from the engine.
#![cfg_attr(target_os = "macos", expect(unsafe_code, reason = "calls into the macOS system libraries"))]
use ks_cast::To as _;
use std::sync::{Arc, OnceLock};

use ks_config::logsetup;
use ks_ui::constants::HINT_CARD_WIDTH_PIXELS;
use ks_ui::hints::{Click, HintPresenter, HintView, Opens};
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
use objc2::{class, msg_send, sel};

use crate::constants::{HINT_BUTTON_HEIGHT_POINTS, HINT_CARD_PADDING_POINTS, HINT_CARD_SPACING_POINTS, HINT_CARD_TITLE_FONT_POINTS, HINT_LABEL_UNLIMITED_HEIGHT_POINTS, PANEL_SLOT, WINDOW_COLLECTION_ALL_SPACES, WINDOW_STYLE_TITLED};
use crate::desktop_app;
use crate::hint_place::{CaretAnchor, Display, Frame, appkit_frame, caret_anchor, card_layout, display_at, origin_above_caret, origin_in_corner, origin_under_item, tag_of};
use crate::macos_native::{NativeAccessibility, run_on_main};
use crate::macos_objc::{Id, string};
use crate::macos_tray_native::status_item_frame;
use crate::settings_window::{self, BUFFERED, Point, Rect, STYLE_PANEL_NONACTIVATING, Size, WINDOW_LEVEL_FLOATING, rect};

const LOGGER: &str = "keyswitch.hints";

extern "C-unwind" fn refuses(_this: &AnyObject, _selector: Sel) -> Bool {
    Bool::NO
}

/// A panel that cannot become the key window or the main one, whatever is clicked in it.
fn panel_class() -> Option<&'static AnyClass> {
    if let Some(registered) = AnyClass::get(c"KeySwitchHintPanel") {
        return Some(registered);
    }
    let mut builder = ClassBuilder::new(c"KeySwitchHintPanel", class!(NSPanel))?;
    // SAFETY: the function has the signature of these selectors.
    unsafe {
        builder.add_method(sel!(canBecomeKeyWindow), { let method: extern "C-unwind" fn(_, _) -> _ = refuses; method });
        builder.add_method(sel!(canBecomeMainWindow), { let method: extern "C-unwind" fn(_, _) -> _ = refuses; method });
    }
    Some(builder.register())
}

extern "C-unwind" fn hint_clicked(_this: &AnyObject, _selector: Sel, sender: *mut AnyObject) {
    // SAFETY: the sender is the button AppKit passes for the time of this call.
    let tag: isize = unsafe { msg_send![sender, tag] };
    run_on_main(Box::new(move || desktop_app::hint_clicked(tag)));
}

/// The one object every button of a hint sends its action to; the buttons say by their tags whose they are (`hint_place`).
pub fn hint_target() -> Option<Id> {
    static TARGET: OnceLock<Option<usize>> = OnceLock::new();
    let target = *TARGET.get_or_init(|| {
        let mut builder = ClassBuilder::new(c"KeySwitchHintTarget", class!(NSObject))?;
        // SAFETY: the function has the signature of this selector.
        unsafe {
            builder.add_method(sel!(hintClicked:), { let method: extern "C-unwind" fn(_, _, _) = hint_clicked; method });
        }
        let class = builder.register();
        // SAFETY: an object kept for the life of the program.
        let target: Id = unsafe {
            let allocated: Id = msg_send![class, alloc];
            msg_send![allocated, init]
        };
        Some(target.to::<usize>())
    });
    target.map(ks_cast::To::pointer_mut::<AnyObject>)
}

fn add(parent: Id, child: Id) {
    // SAFETY: messages on the main thread to live views.
    unsafe {
        let _: () = msg_send![parent, addSubview: child];
    }
}

/// A button placed at `x` in the row at `y`, as wide as its title; says where the next one starts.
fn add_button(parent: Id, target: Id, title: &str, tag: isize, x: f64, y: f64) -> f64 {
    // SAFETY: messages on the main thread; the button stays alive as a subview of its parent.
    unsafe {
        let button: Id = msg_send![class!(NSButton), buttonWithTitle: string(title), target: target, action: sel!(hintClicked:)];
        let _: () = msg_send![button, setTag: tag];
        let _: () = msg_send![button, sizeToFit];
        let fitted: Rect = msg_send![button, frame];
        let _: () = msg_send![button, setFrame: rect(x, y, fitted.size.width, HINT_BUTTON_HEIGHT_POINTS)];
        add(parent, button);
        x + fitted.size.width + HINT_CARD_SPACING_POINTS
    }
}

/// A card of width `width`: the text (under a bold title when one is given), «Понятно» and «Больше не показывать
/// подсказки» under it. The text, when it leads anywhere, is a button of its own, drawn by nothing. An autoreleased view and
/// its height.
pub fn card_view(view: &HintView, width: f64, slot: isize, target: Id, title: Option<&str>) -> (Id, f64) {
    let inner = width - 2.0 * HINT_CARD_PADDING_POINTS;
    // SAFETY: messages on the main thread; the labels stay alive as subviews of the card and the card is autoreleased.
    unsafe {
        let label: Id = msg_send![class!(NSTextField), wrappingLabelWithString: string(&view.text)];
        let _: () = msg_send![label, setSelectable: false];
        let fitted: Size = msg_send![label, sizeThatFits: Size { width: inner, height: HINT_LABEL_UNLIMITED_HEIGHT_POINTS }];
        let mut text_height = fitted.height;
        let heading: Id = match title {
            Some(title) => {
                let heading: Id = msg_send![class!(NSTextField), labelWithString: string(title)];
                let font: Id = msg_send![class!(NSFont), boldSystemFontOfSize: HINT_CARD_TITLE_FONT_POINTS];
                let _: () = msg_send![heading, setFont: font];
                let _: () = msg_send![heading, sizeToFit];
                let size: Rect = msg_send![heading, frame];
                text_height += size.size.height + HINT_CARD_SPACING_POINTS / 2.0;
                heading
            }
            None => std::ptr::null_mut(),
        };
        let layout = card_layout(text_height);
        let allocated: Id = msg_send![class!(NSView), alloc];
        let card: Id = msg_send![allocated, initWithFrame: rect(0.0, 0.0, width, layout.height)];
        let _: Id = msg_send![card, autorelease];
        let _: () = msg_send![label, setFrame: rect(HINT_CARD_PADDING_POINTS, layout.text_y, inner, fitted.height)];
        add(card, label);
        if !heading.is_null() {
            let _: () = msg_send![heading, setFrameOrigin: Point { x: HINT_CARD_PADDING_POINTS, y: layout.text_y + fitted.height + HINT_CARD_SPACING_POINTS / 2.0 }];
            add(card, heading);
        }
        let next = add_button(card, target, view.ok, tag_of(slot, Click::Ok), HINT_CARD_PADDING_POINTS, layout.buttons_y);
        add_button(card, target, view.never, tag_of(slot, Click::Never), next, layout.buttons_y);
        if view.opens.is_some() {
            let over: Id = msg_send![class!(NSButton), buttonWithTitle: string(""), target: target, action: sel!(hintClicked:)];
            let _: () = msg_send![over, setTag: tag_of(slot, Click::Text)];
            let _: () = msg_send![over, setTransparent: true];
            let _: () = msg_send![over, setFrame: rect(HINT_CARD_PADDING_POINTS, layout.text_y, inner, fitted.height)];
            add(card, over);
        }
        (card, layout.height)
    }
}

fn frame_of(rectangle: Rect) -> Frame {
    Frame { x: rectangle.origin.x, y: rectangle.origin.y, width: rectangle.size.width, height: rectangle.size.height }
}

/// Every display: the first is the primary one, the one with the menu bar.
fn displays() -> Vec<Display> {
    // SAFETY: messages on the main thread; the array and its screens are the system's, read within this call.
    unsafe {
        let screens: Id = msg_send![class!(NSScreen), screens];
        let count: usize = msg_send![screens, count];
        (0..count)
            .map(|index| {
                let screen: Id = msg_send![screens, objectAtIndex: index];
                let (frame, visible): (Rect, Rect) = (msg_send![screen, frame], msg_send![screen, visibleFrame]);
                Display { frame: frame_of(frame), visible: frame_of(visible) }
            })
            .collect()
    }
}

/// The panel of the hints at the caret and under the menu bar item.
pub struct HintPanel {
    panel: Id,
    shown: Option<&'static str>,
    caret: Arc<NativeAccessibility>,
}

impl HintPanel {
    #[must_use]
    pub fn new() -> HintPanel {
        HintPanel { panel: std::ptr::null_mut(), shown: None, caret: Arc::new(NativeAccessibility::new()) }
    }

    /// The hint on screen, or about to be, as asked for by the caller last.
    #[must_use]
    pub fn shown(&self) -> Option<&'static str> {
        self.shown
    }

    /// The panel, made the first time it is needed.
    fn panel(&mut self) -> Option<Id> {
        if !self.panel.is_null() {
            return Some(self.panel);
        }
        let class = panel_class()?;
        // SAFETY: an object kept for the life of the program; messages on the main thread.
        unsafe {
            let allocated: Id = msg_send![class, alloc];
            let panel: Id = msg_send![allocated, initWithContentRect: rect(0.0, 0.0, f64::from(HINT_CARD_WIDTH_PIXELS), 100.0), styleMask: (WINDOW_STYLE_TITLED | STYLE_PANEL_NONACTIVATING), backing: BUFFERED, defer: true];
            let _: () = msg_send![panel, setReleasedWhenClosed: false];
            let _: () = msg_send![panel, setLevel: WINDOW_LEVEL_FLOATING];
            let _: () = msg_send![panel, setFloatingPanel: true];
            // A panel hides when its program stops being the active one, and this program is hardly ever active.
            let _: () = msg_send![panel, setHidesOnDeactivate: false];
            let _: () = msg_send![panel, setCollectionBehavior: WINDOW_COLLECTION_ALL_SPACES];
            self.panel = panel;
        }
        Some(self.panel)
    }

    /// Shows the hint asked for by `show_caret`, once the caret has been found; a hint taken down meanwhile stays down.
    pub fn place_at_caret(&mut self, view: &HintView, anchor: CaretAnchor) {
        if self.shown == Some(view.id) {
            let caret = match anchor {
                CaretAnchor::At(frame) => Some(frame),
                CaretAnchor::Unknown => None,
            };
            self.present(view, caret);
        }
    }

    /// Shows `view` at `caret` in `AppKit`'s coordinates, or under the menu bar item when there is none. False when it
    /// could not be drawn: the hint then stays unseen.
    fn present(&mut self, view: &HintView, caret: Option<Frame>) -> bool {
        let (Some(panel), Some(target)) = (self.panel(), hint_target()) else { return false };
        let width = f64::from(HINT_CARD_WIDTH_PIXELS);
        objc2::rc::autoreleasepool(|_| {
            let displays = displays();
            let Some(primary) = displays.first().copied() else { return false };
            let (card, height) = card_view(view, width, PANEL_SLOT, target, None);
            // SAFETY: messages on the main thread to the panel this object owns.
            unsafe {
                let _: () = msg_send![panel, setTitle: string(view.title)];
                let _: () = msg_send![panel, setContentView: card];
                let _: () = msg_send![panel, setContentSize: Size { width, height }];
                let frame: Rect = msg_send![panel, frame];
                let size = (frame.size.width, frame.size.height);
                let (x, y) = if let Some(caret) = caret {
                    let caret = appkit_frame(caret, primary.frame.height);
                    let display = display_at(&displays, caret.x, caret.y).unwrap_or(primary);
                    origin_above_caret(caret, size, display.visible)
                } else if let Some(item) = status_item_frame() {
                    let display = display_at(&displays, item.x, item.y).unwrap_or(primary);
                    origin_under_item(item, size, display.visible)
                } else {
                    origin_in_corner(size, primary.visible)
                };
                let _: () = msg_send![panel, setFrameOrigin: Point { x, y }];
                let _: () = msg_send![panel, orderFrontRegardless];
            }
            self.shown = Some(view.id);
            true
        })
    }
}

impl Default for HintPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl HintPresenter<()> for HintPanel {
    /// The caret is looked for now, when the typing has stopped and the text has settled, not when the correction was made:
    /// `TextEdit` answers wrongly for a range of text it is still rewriting. A field's program can be slow to answer, so a
    /// thread of its own asks and the main thread draws what it finds.
    fn show_caret(&mut self, view: &HintView, _anchor: &()) -> bool {
        self.shown = Some(view.id);
        let (view, caret) = (view.clone(), self.caret.clone());
        std::thread::spawn(move || {
            let anchor = caret_anchor(&*caret);
            run_on_main(Box::new(move || desktop_app::place_caret_hint(&view, anchor)));
        });
        true
    }

    fn show_tray(&mut self, view: &HintView) -> bool {
        self.present(view, None)
    }

    fn hide(&mut self, id: &str) {
        if self.shown == Some(id) {
            self.shown = None;
            // SAFETY: a message on the main thread to the panel this object owns.
            unsafe {
                let _: () = msg_send![self.panel, orderOut: std::ptr::null_mut::<AnyObject>()];
            }
        }
        settings_window::remove_hint_card(id);
    }

    fn open(&mut self, opens: &Opens) {
        match opens.clone() {
            Opens::Settings { page, setting } => run_on_main(Box::new(move || settings_window::show_setting(page.window_page(), setting))),
            Opens::Url(address) => {
                if let Err(error) = std::process::Command::new("open").arg(&address).status() {
                    logsetup::warning(LOGGER, &format!("Не удалось открыть {address}: {error}"));
                }
            }
        }
    }
}
