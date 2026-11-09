//! Quartz and Cocoa numbers and macOS-specific limits, each value once, as in the Python's `constants/macos.py`, and from
//! `timing.py`, `text.py` and `units.py` what these modules use.

// ------------------------------------------------------------------------------------------------ constants/timing.py

/// How long a layout change may take to become visible. Selecting an input source is asynchronous: the call returns
/// before the window server has told the focused application, and typing into the old layout in between is exactly the
/// error the correction exists to undo.
pub const LAYOUT_SWITCH_TIMEOUT_SECONDS: f64 = 0.5;
pub const LAYOUT_SWITCH_POLL_SECONDS: f64 = 0.01;
/// How long `start()` waits for the event tap to confirm it runs.
pub const KEYBOARD_LISTENER_START_TIMEOUT_SECONDS: f64 = 5.0;
/// How long `stop()` waits for the tap's thread to finish.
pub const KEYBOARD_LISTENER_STOP_TIMEOUT_SECONDS: f64 = 2.0;
/// The tap's run loop is run in slices of this length, so a stop request is seen within one.
pub const EVENT_TAP_RUN_LOOP_SLICE_SECONDS: f64 = 0.2;
/// How often the program asks again, while it waits, whether the user has granted the permission.
pub const MACOS_PERMISSION_POLL_SECONDS: f64 = 1.0;
/// How long a caller waits for the main thread to answer, and the shorter wait of the tap's own callback.
pub const MACOS_MAIN_THREAD_WAIT_SECONDS: f64 = 0.25;
pub const MACOS_TAP_MAIN_THREAD_WAIT_SECONDS: f64 = 0.02;
pub const MACOS_LAYOUT_SWITCH_WAIT_SECONDS: f64 = 1.0;
pub const SELECTION_CLIPBOARD_TIMEOUT_SECONDS: f64 = 1.0;
/// The user's pasteboard is given back this long after the paste keys.
pub const MACOS_PASTE_RESTORE_DELAY_SECONDS: f64 = 0.5;

// -------------------------------------------------------------------------------------- constants/text.py and units.py

pub const BYTES_PER_MEBIBYTE: usize = 1024 * 1024;
/// The most a pasteboard may hold to be borrowed for a paste.
pub const CLIPBOARD_SNAPSHOT_MAX_BYTES: usize = 64 * BYTES_PER_MEBIBYTE;
/// The code points of the Cyrillic block.
pub const CYRILLIC_CODEPOINT_RANGE: (u32, u32) = (0x0400, 0x04FF);

// ------------------------------------------------------------------------------------------------ constants/macos.py

// Virtual key codes, read from Apple's Events.h. They name physical positions and do not move with the layout, which is
// what the engine's key codes mean.
pub const MAC_VK_ANSI_Q: u32 = 0x0C;
/// `kVK_ANSI_V`, the key Cmd+V pastes with.
pub const MAC_VK_ANSI_V: u32 = 0x09;
pub const MAC_VK_KEYPAD_ENTER: u32 = 0x4C;
pub const MAC_VK_RETURN: u32 = 0x24;
pub const MAC_VK_TAB: u32 = 0x30;
pub const MAC_VK_SPACE: u32 = 0x31;
pub const MAC_VK_BACKSPACE: u32 = 0x33;
pub const MAC_VK_ESCAPE: u32 = 0x35;
pub const MAC_VK_COMMAND: u32 = 0x37;
pub const MAC_VK_RIGHT_COMMAND: u32 = 0x36;
pub const MAC_VK_SHIFT: u32 = 0x38;
pub const MAC_VK_CAPS_LOCK: u32 = 0x39;
pub const MAC_VK_OPTION: u32 = 0x3A;
pub const MAC_VK_CONTROL: u32 = 0x3B;
pub const MAC_VK_RIGHT_SHIFT: u32 = 0x3C;
pub const MAC_VK_RIGHT_OPTION: u32 = 0x3D;
pub const MAC_VK_RIGHT_CONTROL: u32 = 0x3E;
pub const MAC_VK_FUNCTION: u32 = 0x3F;
pub const MAC_VK_HELP: u32 = 0x72;
pub const MAC_VK_HOME: u32 = 0x73;
pub const MAC_VK_PAGE_UP: u32 = 0x74;
pub const MAC_VK_FORWARD_DELETE: u32 = 0x75;
pub const MAC_VK_END: u32 = 0x77;
pub const MAC_VK_PAGE_DOWN: u32 = 0x79;
pub const MAC_VK_LEFT_ARROW: u32 = 0x7B;
pub const MAC_VK_RIGHT_ARROW: u32 = 0x7C;
pub const MAC_VK_DOWN_ARROW: u32 = 0x7D;
pub const MAC_VK_UP_ARROW: u32 = 0x7E;
/// The function keys (`kVK_F1` .. `kVK_F20` of HIToolbox's Events.h): their codes follow no order.
pub const MAC_VK_F1: u32 = 0x7A;
pub const MAC_VK_F2: u32 = 0x78;
pub const MAC_VK_F3: u32 = 0x63;
pub const MAC_VK_F4: u32 = 0x76;
pub const MAC_VK_F5: u32 = 0x60;
pub const MAC_VK_F6: u32 = 0x61;
pub const MAC_VK_F7: u32 = 0x62;
pub const MAC_VK_F8: u32 = 0x64;
pub const MAC_VK_F9: u32 = 0x65;
pub const MAC_VK_F10: u32 = 0x6D;
pub const MAC_VK_F11: u32 = 0x67;
pub const MAC_VK_F12: u32 = 0x6F;
pub const MAC_VK_F13: u32 = 0x69;
pub const MAC_VK_F14: u32 = 0x6B;
pub const MAC_VK_F15: u32 = 0x71;
pub const MAC_VK_F16: u32 = 0x6A;
pub const MAC_VK_F17: u32 = 0x40;
pub const MAC_VK_F18: u32 = 0x4F;
pub const MAC_VK_F19: u32 = 0x50;
pub const MAC_VK_F20: u32 = 0x5A;
/// A key `key_name` has no name for, as a hotkey of the settings holds it: `vk_` and its code in two hexadecimal digits.
pub const UNNAMED_KEY_PREFIX: &str = "vk_";
pub const UNNAMED_KEY_CODE_DIGITS: usize = 2;
pub const HEXADECIMAL: u32 = 16;

/// The event tap reports these instead of a key when the system disabled it.
pub const EVENT_TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
pub const EVENT_TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;

// Event numbers, from IOLLEvent.h through CGEventTypes.h.
pub const EVENT_LEFT_MOUSE_DOWN: u32 = 1;
pub const EVENT_RIGHT_MOUSE_DOWN: u32 = 3;
pub const EVENT_KEY_DOWN: u32 = 10;
pub const EVENT_KEY_UP: u32 = 11;
pub const EVENT_FLAGS_CHANGED: u32 = 12;
pub const EVENT_OTHER_MOUSE_DOWN: u32 = 25;
/// A turn of the wheel or a scroll on the trackpad: reported only as a turn, which ends a lone press of the layout switch
/// key (Control with the wheel zooms where Accessibility is set so, Shift with it scrolls sideways).
pub const EVENT_SCROLL_WHEEL: u32 = 22;

// Modifier bits of a Quartz event, from IOLLEvent.h.
pub const EVENT_FLAG_ALPHA_SHIFT: u64 = 0x0001_0000;
pub const EVENT_FLAG_SHIFT: u64 = 0x0002_0000;
pub const EVENT_FLAG_CONTROL: u64 = 0x0004_0000;
pub const EVENT_FLAG_ALTERNATE: u64 = 0x0008_0000;
pub const EVENT_FLAG_COMMAND: u64 = 0x0010_0000;

// Side-specific modifier bits of the same flags, from IOLLEvent.h (NX_DEVICE*KEYMASK). A flags-changed event carries no
// "down" of its own: the bit of its key says it, and with both Shift keys held, releasing one clears only its own bit.
pub const EVENT_FLAG_DEVICE_LEFT_CONTROL: u64 = 0x0000_0001;
pub const EVENT_FLAG_DEVICE_LEFT_SHIFT: u64 = 0x0000_0002;
pub const EVENT_FLAG_DEVICE_RIGHT_SHIFT: u64 = 0x0000_0004;
pub const EVENT_FLAG_DEVICE_LEFT_COMMAND: u64 = 0x0000_0008;
pub const EVENT_FLAG_DEVICE_RIGHT_COMMAND: u64 = 0x0000_0010;
pub const EVENT_FLAG_DEVICE_LEFT_ALTERNATE: u64 = 0x0000_0020;
pub const EVENT_FLAG_DEVICE_RIGHT_ALTERNATE: u64 = 0x0000_0040;
pub const EVENT_FLAG_DEVICE_RIGHT_CONTROL: u64 = 0x0000_2000;

// Tap placement and options.
pub const SESSION_EVENT_TAP: u32 = 1;
pub const HEAD_INSERT_EVENT_TAP: u32 = 0;
pub const EVENT_TAP_OPTION_DEFAULT: u32 = 0;
pub const KEYBOARD_EVENT_KEYCODE_FIELD: u32 = 9;
pub const EVENT_SOURCE_USER_DATA_FIELD: u32 = 42;

/// The marks the backend puts on events it posts itself. A tap sees its own injections like any other event, and without
/// a mark it would answer its own corrections as though the user had typed them.
pub const EVENT_MARK_INJECTED: i64 = 0x4B53_0001;
pub const EVENT_MARK_REPLAYED: i64 = 0x4B53_0002;

// UCKeyTranslate arguments.
pub const UC_KEY_ACTION_DISPLAY: u16 = 3;
pub const UC_KEY_TRANSLATE_NO_DEAD_KEYS_MASK: u32 = 1;
/// shiftKey (0x0200) >> 8, the shape `UCKeyTranslate` wants.
pub const UC_MODIFIER_SHIFT: u32 = 2;
/// alphaLock (0x0400) >> 8.
pub const UC_MODIFIER_ALPHA_LOCK: u32 = 4;
/// optionKey (0x0800) >> 8.
pub const UC_MODIFIER_OPTION: u32 = 8;
/// controlKey (0x1000) >> 8.
pub const UC_MODIFIER_CONTROL: u32 = 16;
/// UTF-16 units of the buffer `UCKeyTranslate` writes a key's text into.
pub const UC_KEY_TRANSLATE_BUFFER_CHARACTERS: usize = 8;
/// Largest text an accessibility attribute is copied out at: a field holding more is not read, rather than copying a
/// whole document into memory for a few hundred characters of context.
pub const CF_STRING_MAX_BYTES: usize = 4 * BYTES_PER_MEBIBYTE;
/// kCFStringEncodingUTF8.
pub const CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
pub const CF_NUMBER_SINT32_TYPE: isize = 3;
pub const WINDOW_LIST_ON_SCREEN_ONLY: u32 = 1 << 0;
pub const WINDOW_LIST_EXCLUDE_DESKTOP: u32 = 1 << 4;
pub const NULL_WINDOW_ID: u32 = 0;
/// A window the user types into sits on the normal layer; menus, the dock and overlays sit above it and would otherwise
/// be mistaken for the focus.
pub const NORMAL_WINDOW_LAYER: i32 = 0;
pub const AX_SUCCESS: i32 = 0;
/// `DISPATCH_TIME_NOW` of dispatch/time.h: a delay given to `dispatch_time` counts from now.
pub const DISPATCH_TIME_NOW: u64 = 0;
/// `AXValue` wrapping a `CFRange`, from AXValue.h.
pub const AX_VALUE_TYPE_CF_RANGE: u32 = 4;
/// `kAXValueCGRectType` of `AXValue.h`.
pub const AX_VALUE_TYPE_CG_RECT: u32 = 3;
/// `NSApplication` presents no dock icon and no menu bar of its own.
pub const ACTIVATION_POLICY_ACCESSORY: isize = 1;
/// `NSApplication` with a dock icon and a menu bar, for as long as the settings window is open.
pub const ACTIVATION_POLICY_REGULAR: isize = 0;
/// `NSControlStateValue` of a checked and an unchecked menu item.
pub const CONTROL_STATE_ON: isize = 1;
pub const CONTROL_STATE_OFF: isize = 0;

pub const MAC_SHIFT_KEYS: [u32; 2] = [MAC_VK_SHIFT, MAC_VK_RIGHT_SHIFT];
pub const MAC_CONTROL_KEYS: [u32; 2] = [MAC_VK_CONTROL, MAC_VK_RIGHT_CONTROL];
pub const MAC_ALT_KEYS: [u32; 2] = [MAC_VK_OPTION, MAC_VK_RIGHT_OPTION];
pub const MAC_SUPER_KEYS: [u32; 2] = [MAC_VK_COMMAND, MAC_VK_RIGHT_COMMAND];

/// The flag bit that is set while a modifier key is down, by keycode, where the event carries a side-specific one.
/// Caps Lock has none: its bit holds the lock, not the key (see `macos_backend::modifier_pressed`).
pub const MAC_MODIFIER_DEVICE_FLAGS: [(u32, u64); 8] = [
    (MAC_VK_CONTROL, EVENT_FLAG_DEVICE_LEFT_CONTROL),
    (MAC_VK_RIGHT_CONTROL, EVENT_FLAG_DEVICE_RIGHT_CONTROL),
    (MAC_VK_SHIFT, EVENT_FLAG_DEVICE_LEFT_SHIFT),
    (MAC_VK_RIGHT_SHIFT, EVENT_FLAG_DEVICE_RIGHT_SHIFT),
    (MAC_VK_OPTION, EVENT_FLAG_DEVICE_LEFT_ALTERNATE),
    (MAC_VK_RIGHT_OPTION, EVENT_FLAG_DEVICE_RIGHT_ALTERNATE),
    (MAC_VK_COMMAND, EVENT_FLAG_DEVICE_LEFT_COMMAND),
    (MAC_VK_RIGHT_COMMAND, EVENT_FLAG_DEVICE_RIGHT_COMMAND),
];
pub const MAC_DEVICE_FLAGS_MASK: u64 = EVENT_FLAG_DEVICE_LEFT_CONTROL
    | EVENT_FLAG_DEVICE_RIGHT_CONTROL
    | EVENT_FLAG_DEVICE_LEFT_SHIFT
    | EVENT_FLAG_DEVICE_RIGHT_SHIFT
    | EVENT_FLAG_DEVICE_LEFT_ALTERNATE
    | EVENT_FLAG_DEVICE_RIGHT_ALTERNATE
    | EVENT_FLAG_DEVICE_LEFT_COMMAND
    | EVENT_FLAG_DEVICE_RIGHT_COMMAND;
/// The shared bit of each modifier, for an event that carries no side-specific bits.
pub const MAC_MODIFIER_SHARED_FLAGS: [(u32, u64); 8] = [
    (MAC_VK_SHIFT, EVENT_FLAG_SHIFT),
    (MAC_VK_RIGHT_SHIFT, EVENT_FLAG_SHIFT),
    (MAC_VK_CONTROL, EVENT_FLAG_CONTROL),
    (MAC_VK_RIGHT_CONTROL, EVENT_FLAG_CONTROL),
    (MAC_VK_OPTION, EVENT_FLAG_ALTERNATE),
    (MAC_VK_RIGHT_OPTION, EVENT_FLAG_ALTERNATE),
    (MAC_VK_COMMAND, EVENT_FLAG_COMMAND),
    (MAC_VK_RIGHT_COMMAND, EVENT_FLAG_COMMAND),
];
/// The key whose translation tells which script a layout types.
pub const MAC_SCRIPT_PROBE_KEYCODE: u32 = MAC_VK_ANSI_Q;

// ------------------------------------------------------------------------------------ the field reader and the system

/// What the engine calls the source of a field read through the accessibility tree.
pub const SOURCE_AX: &str = "ax";
/// The pane of System Settings that holds the switch `KeySwitch` needs. macOS has no call that grants the permission; the
/// most a program may do is ask, and then take the user to the place where the answer is given.
pub const ACCESSIBILITY_SETTINGS_URL: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";
/// Where programs live on macOS. The first two are the system's own, the third is a user's private one.
pub const APPLICATION_DIRECTORIES: [&str; 3] = ["/Applications", "/System/Applications", "~/Applications"];
/// Apple keeps its smaller programs one level down, and installers use the same place.
pub const NESTED_DIRECTORIES: [&str; 1] = ["Utilities"];
/// launchd reads every property list in this directory when the user logs in.
pub const LAUNCH_AGENTS_DIRECTORY: &str = "Library/LaunchAgents";
/// The bundle identifier of the application, which also labels its launch agent.
pub const APP_ID: &str = "io.github.olegius88.KeySwitch";
pub const BACKEND_LABEL: &str = "CGEventTap + CGEventPost";

// ---- the one-time hints (`hint_place`, `hint_panel`): points of the screen
/// Between the caret, or the menu bar item, and the panel of a hint.
pub const HINT_PANEL_GAP_POINTS: f64 = 8.0;
/// How far a panel keeps from the edge of a display.
pub const HINT_SCREEN_MARGIN_POINTS: f64 = 8.0;
/// Around the rows of a card.
pub const HINT_CARD_PADDING_POINTS: f64 = 12.0;
pub const HINT_BUTTON_HEIGHT_POINTS: f64 = 28.0;
/// Between the text of a card and its buttons.
pub const HINT_BUTTON_ROW_GAP_POINTS: f64 = 10.0;
/// Between the two buttons of a card, and under its title.
pub const HINT_CARD_SPACING_POINTS: f64 = 8.0;
pub const HINT_CARD_TITLE_FONT_POINTS: f64 = 13.0;
/// How tall a label may ask to be: its width limits it first.
pub const HINT_LABEL_UNLIMITED_HEIGHT_POINTS: f64 = 10000.0;
/// How far the frame of a marked control stands from it, how wide it is and how round its corners are.
pub const HINT_MARKER_INSET_POINTS: f64 = 3.0;
pub const HINT_MARKER_BORDER_POINTS: f64 = 2.0;
pub const HINT_MARKER_CORNER_POINTS: f64 = 4.0;
pub const HINT_CARD_CORNER_POINTS: f64 = 6.0;
/// The slots of the buttons of a hint (`hint_place`): the panel, then the settings window's pages by index. Tags count the
/// clicks of one slot before the next slot starts, as one target serves them all.
pub const PANEL_SLOT: isize = 0;
pub const HINT_FIRST_PAGE_SLOT: isize = 1;
pub const HINT_CLICKS_PER_SLOT: isize = 3;
pub const HINT_CLICK_OK: isize = 0;
pub const HINT_CLICK_NEVER: isize = 1;
pub const HINT_CLICK_TEXT: isize = 2;
/// `NSWindowStyleMaskTitled`.
pub const WINDOW_STYLE_TITLED: u64 = 1;
/// `NSWindowCollectionBehaviorCanJoinAllSpaces` and `NSWindowCollectionBehaviorFullScreenAuxiliary`: the panel shows wherever
/// the user is typing, a full screen program included.
pub const WINDOW_COLLECTION_ALL_SPACES: u64 = (1 << 0) | (1 << 8);
/// `NSBoxCustom` and `NSNoTitle`: a card or a marker draws only what is set on it.
pub const BOX_TYPE_CUSTOM: u64 = 4;
pub const BOX_TITLE_NONE: u64 = 0;
/// `NSWindowBelow`: a marker lies under its control, so that the control keeps its clicks.
pub const WINDOW_ORDER_BELOW: isize = -1;
