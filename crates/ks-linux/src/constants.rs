//! X11, XKB and `XRecord` protocol numbers, keysyms and the timing of the backend.

pub const X11_WHEEL_BUTTONS: [u8; 4] = [4, 5, 6, 7];
pub const X11_KEY_PRESS: u8 = 2;
pub const X11_BUTTON_PRESS: u8 = 4;
pub const X11_KEY_RELEASE: u8 = 3;
pub const XRECORD_FROM_SERVER: u8 = 0;
pub const XRECORD_START_OF_DATA: u8 = 4;
pub const XRECORD_ALL_CLIENTS: u32 = 3;
pub const XKB_USE_CORE_KBD: u16 = 0x0100;
pub const MAX_XKB_GROUPS: usize = 4;
pub const XRECORD_EVENT_SIZE_BYTES: usize = 32;
pub const XRECORD_EVENT_TYPE_MASK: u8 = 0x7F;
pub const XKB_GROUP_STATE_SHIFT: u32 = 13;
pub const XKB_GROUP_STATE_MASK: u16 = 0x3;
pub const MAX_TRACKED_OWN_WINDOWS: usize = 256;
pub const MAX_WINDOW_ANCESTOR_DEPTH: usize = 16;
pub const BACKSPACE_KEYSYM: u32 = 0xFF08;
pub const SHIFT_L_KEYSYM: u32 = 0xFFE1;
pub const LEFT_KEYSYM: u32 = 0xFF51;
pub const X11_MIN_KEYCODE: u8 = 8;
pub const X11_MAX_KEYCODE: u8 = 255;

pub const KEYBOARD_LISTENER_START_TIMEOUT_SECONDS: f64 = 5.0;
pub const KEYBOARD_LISTENER_STOP_TIMEOUT_SECONDS: f64 = 2.0;
/// An X11 injection batch waits at least this long, plus `INJECTION_SECONDS_PER_EVENT` per event, for the keys to come back.
pub const MIN_INJECTION_DEADLINE_SECONDS: f64 = 1.0;
pub const INJECTION_SECONDS_PER_EVENT: f64 = 0.02;
/// A group asked of the desktop is waited for this long before XKB is locked directly.
pub const DESKTOP_GROUP_WAIT_MS: u64 = 300;
pub const DESKTOP_GROUP_POLL_MS: u64 = 5;
/// Keys sent less than this long before a group is asked of the desktop are given the rest of it first: Cinnamon
/// switches the IBus engine with the layout, and keys still on their way were lost or came out in the next layout (no loss
/// from 150 ms on in 25 switches on Linux Mint 22.3, losses below 100 ms).
pub const DESKTOP_SWITCH_SETTLE_MS: u64 = 250;

/// Cinnamon keeps the layout group itself and switches it through its own service.
pub const CINNAMON_DESKTOP_NAMES: [&str; 2] = ["X-Cinnamon", "Cinnamon"];
pub const CINNAMON_BUS_NAME: &str = "org.Cinnamon";
pub const CINNAMON_OBJECT_PATH: &str = "/org/Cinnamon";
pub const CINNAMON_INTERFACE: &str = "org.Cinnamon";
pub const CINNAMON_GET_INPUT_SOURCES: &str = "GetInputSources";
pub const CINNAMON_ACTIVATE_INPUT_SOURCE: &str = "ActivateInputSourceIndex";
/// The kind of a Cinnamon input source that is an XKB layout, and how many of them one keymap holds (`keyboardManager.js`
/// leaves one of the four XKB groups free).
pub const CINNAMON_XKB_SOURCE: &str = "xkb";
pub const CINNAMON_LAYOUTS_PER_KEYMAP: usize = 3;
pub const CINNAMON_DBUS_CALL_TIMEOUT_MS: u64 = 1000;

pub const ATSPI_MAX_TRAVERSED_CHILDREN: usize = 64;
pub const ATSPI_MAX_VISITED_NODES: usize = 128;
pub const FIELD_AFTER_CARET_MAX_CHARACTERS: i64 = 128;
pub const ATSPI_CALL_TIMEOUT_MS: u64 = 50;
pub const ATSPI_FIELD_READ_DEADLINE_SECONDS: f64 = 0.15;
/// What GTK4 shows for each character of a hidden entry exposed as ordinary text.
pub const PASSWORD_MASKS: [char; 3] = ['•', '●', '*'];

/// `AtspiStateType` and `AtspiRole` numbers (Atspi 2.0 typelib).
pub const ATSPI_STATE_EDITABLE: u32 = 7;
pub const ATSPI_STATE_FOCUSED: u32 = 12;
pub const ATSPI_ROLE_PASSWORD_TEXT: u32 = 40;


pub const APP_ID: &str = "io.github.olegius88.KeySwitch";
pub const APP_NAME: &str = "KeySwitch";
pub const BACKEND_LABEL: &str = "X11 RECORD + XTEST";
