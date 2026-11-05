//! Linux evdev, uinput, Wayland XKB and compositor IPC protocol numbers (the Python's `constants/wayland.py`), with the
//! timings of the Wayland layer (`constants/timing.py`).


pub const EV_SYN: u32 = 0x00;
pub const EV_KEY: u32 = 0x01;
pub const EV_REL: u32 = 0x02;
pub const EV_ABS: u32 = 0x03;
pub const EV_MSC: u32 = 0x04;
pub const EV_LED: u32 = 0x11;
pub const EV_REP: u32 = 0x14;
pub const SYN_REPORT: u32 = 0;
pub const KEY_VALUE_RELEASE: i32 = 0;
pub const KEY_VALUE_PRESS: i32 = 1;
pub const KEY_VALUE_REPEAT: i32 = 2;
pub const LED_NUML: u32 = 0x00;
pub const LED_CAPSL: u32 = 0x01;
pub const LED_SCROLLL: u32 = 0x02;
pub const BUS_I8042: u32 = 0x11;
pub const WAYLAND_UINPUT_VENDOR_ID: u32 = 0x1209;
pub const WAYLAND_UINPUT_PRODUCT_ID: u32 = 0x4B53;
pub const WAYLAND_UINPUT_VERSION: u32 = 1;
pub const WAYLAND_UINPUT_MAX_KEYCODE: u32 = 248;
pub const UI_SET_EVBIT: libc::c_ulong = 0x4004_5564;
pub const UI_SET_KEYBIT: libc::c_ulong = 0x4004_5565;
pub const UI_SET_LEDBIT: libc::c_ulong = 0x4004_5569;
pub const UI_DEV_SETUP: libc::c_ulong = 0x405C_5503;
pub const UI_DEV_CREATE: libc::c_ulong = 0x5501;
pub const UI_DEV_DESTROY: libc::c_ulong = 0x5502;
pub const EVIOCGID: libc::c_ulong = 0x8008_4502;
pub const EVIOCGNAME_256: libc::c_ulong = 0x8100_4506;
pub const EVIOCGPHYS_256: libc::c_ulong = 0x8100_4507;
pub const EVIOCGKEY_96: libc::c_ulong = 0x8060_4518;
pub const EVIOCGLED_8: libc::c_ulong = 0x8008_4519;
pub const EVIOCGBIT_EV_8: libc::c_ulong = 0x8008_4520;
pub const EVIOCGBIT_KEY_96: libc::c_ulong = 0x8060_4521;
pub const EVIOCGBIT_REL_8: libc::c_ulong = 0x8008_4522;
pub const EVIOCGBIT_ABS_8: libc::c_ulong = 0x8008_4523;
pub const EVIOCGRAB: libc::c_ulong = 0x4004_4590;
pub const EVIOCSCLOCKID: libc::c_ulong = 0x4004_45A0;
pub const EVDEV_NAME_BUFFER_BYTES: u32 = 256;
pub const EVDEV_KEY_BITMAP_BYTES: u32 = 96;
pub const EVDEV_SMALL_BITMAP_BYTES: u32 = 8;
pub const EVDEV_READ_BATCH_EVENTS: u32 = 64;
pub const UINPUT_MAX_NAME_BYTES: usize = 80;
pub const UINT32_MODULO_MASK: u32 = 0xFFFF_FFFF;
pub const EVDEV_KEY_ESC: u32 = 1;
pub const EVDEV_KEY_1: u32 = 2;
pub const EVDEV_KEY_0: u32 = 11;
pub const EVDEV_KEY_MINUS: u32 = 12;
pub const EVDEV_KEY_EQUAL: u32 = 13;
pub const EVDEV_KEY_BACKSPACE: u32 = 14;
pub const EVDEV_KEY_TAB: u32 = 15;
pub const EVDEV_KEY_Q: u32 = 16;
pub const EVDEV_KEY_LEFTBRACE: u32 = 26;
pub const EVDEV_KEY_RIGHTBRACE: u32 = 27;
pub const EVDEV_KEY_ENTER: u32 = 28;
pub const EVDEV_KEY_LEFTCTRL: u32 = 29;
pub const EVDEV_KEY_A: u32 = 30;
pub const EVDEV_KEY_SEMICOLON: u32 = 39;
pub const EVDEV_KEY_APOSTROPHE: u32 = 40;
pub const EVDEV_KEY_GRAVE: u32 = 41;
pub const EVDEV_KEY_LEFTSHIFT: u32 = 42;
pub const EVDEV_KEY_BACKSLASH: u32 = 43;
pub const EVDEV_KEY_Z: u32 = 44;
pub const EVDEV_KEY_COMMA: u32 = 51;
pub const EVDEV_KEY_DOT: u32 = 52;
pub const EVDEV_KEY_SLASH: u32 = 53;
pub const EVDEV_KEY_RIGHTSHIFT: u32 = 54;
pub const EVDEV_KEY_LEFTALT: u32 = 56;
pub const EVDEV_KEY_SPACE: u32 = 57;
pub const EVDEV_KEY_CAPSLOCK: u32 = 58;
pub const EVDEV_KEY_KPENTER: u32 = 96;
pub const EVDEV_KEY_RIGHTCTRL: u32 = 97;
pub const EVDEV_KEY_RIGHTALT: u32 = 100;
pub const EVDEV_KEY_HOME: u32 = 102;
pub const EVDEV_KEY_UP: u32 = 103;
pub const EVDEV_KEY_PAGEUP: u32 = 104;
pub const EVDEV_KEY_LEFT: u32 = 105;
pub const EVDEV_KEY_RIGHT: u32 = 106;
pub const EVDEV_KEY_END: u32 = 107;
pub const EVDEV_KEY_DOWN: u32 = 108;
pub const EVDEV_KEY_PAGEDOWN: u32 = 109;
pub const EVDEV_KEY_INSERT: u32 = 110;
pub const EVDEV_KEY_DELETE: u32 = 111;
pub const EVDEV_KEY_PAUSE: u32 = 119;
pub const EVDEV_KEY_LEFTMETA: u32 = 125;
pub const EVDEV_KEY_RIGHTMETA: u32 = 126;
pub const EVDEV_REL_X: u32 = 0x00;
pub const EVDEV_REL_Y: u32 = 0x01;
pub const EVDEV_ABS_MT_POSITION_X: u32 = 0x35;
pub const EVDEV_BTN_MOUSE: u32 = 0x110;
pub const EVDEV_BTN_TASK: u32 = 0x117;
pub const EVDEV_BTN_GAMEPAD: u32 = 0x130;
pub const EVDEV_BTN_STYLUS: u32 = 0x14B;
pub const WAYLAND_XKB_KEYCODE_OFFSET: u32 = 8;
pub const WL_SEAT_CAPABILITY_KEYBOARD: u32 = 2;
pub const WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1: u32 = 1;
pub const WAYLAND_SEAT_INTERFACE_MAX_VERSION: u32 = 7;
pub const WAYLAND_KEYMAP_ROUNDTRIPS: u32 = 3;
pub const XKB_KEYSYM_NAME_BUFFER_BYTES: usize = 64;
pub const IN_ATTRIB: u32 = 0x0000_0004;
pub const IN_CREATE: u32 = 0x0000_0100;
pub const IN_DELETE: u32 = 0x0000_0200;
pub const IN_NONBLOCK: u32 = 0x0000_0800;
pub const IN_CLOEXEC: u32 = 0x0008_0000;
pub const INOTIFY_EVENT_HEADER_BYTES: u32 = 16;
pub const INOTIFY_READ_BUFFER_BYTES: u32 = 4096;
pub const HELPER_FRAME_HEADER_BYTES: u32 = 5;
pub const HELPER_MAX_FRAME_BYTES: u32 = 65536;
pub const HELPER_EMIT_ITEM_BYTES: u32 = 3;
pub const HELPER_SOCKET_CHUNK_BYTES: u32 = 4096;
// Key transitions written to uinput in one go. Every reader of a keyboard device gets a queue of 64 events
// (EVDEV_MIN_BUFFER_SIZE in drivers/input/evdev.c: a key-only device estimates 8 events a packet, times
// EVDEV_BUF_PACKETS 8); one more and the kernel drops all it holds for SYN_DROPPED. A transition is two events (the
// key and SYN_REPORT): a burst of 8 fills a quarter of it. A correction written whole (72 events) vanished in GNOME 50.
pub const WAYLAND_UINPUT_BURST_TRANSITIONS: u32 = 8;
pub const TRANSITION_EVENTS: u32 = 2;
pub const HELPER_MSG_READY: u32 = 1;
pub const HELPER_MSG_ERROR: u32 = 2;
pub const HELPER_MSG_KEY: u32 = 3;
pub const HELPER_MSG_POINTER: u32 = 4;
pub const HELPER_MSG_CAPS_LOCK: u32 = 5;
pub const HELPER_MSG_EMIT_ACK: u32 = 6;
pub const HELPER_MSG_KEY_DOWN: u32 = 7;
pub const HELPER_MSG_REPEAT_RESET: u32 = 8;
pub const HELPER_MSG_INPUT_RESET: u32 = 9;
pub const HELPER_CMD_VERDICT: u32 = 101;
pub const HELPER_CMD_EMIT_BATCH: u32 = 102;
pub const HELPER_CMD_SET_GRAB: u32 = 103;
pub const HELPER_CMD_QUERY_KEY_DOWN: u32 = 104;
pub const HELPER_CMD_STOP: u32 = 105;
pub const HELPER_CMD_SEQ_ACK: u32 = 106;
pub const HELPER_KEY_NEEDS_VERDICT: u32 = 1;
pub const HELPER_KEY_FORCED: u32 = 2;
pub const HELPER_EMIT_UNCHECKED: u32 = 0;
pub const HELPER_EMIT_TEXT: u32 = 1;
pub const HELPER_EMIT_ACTION: u32 = 2;
pub const WAYLAND_HELPER_RESTARTS: u32 = 5;
pub const HELPER_VERDICT_PASS: u32 = 0;
pub const HELPER_VERDICT_SWALLOW: u32 = 1;
pub const SWAY_IPC_HEADER_BYTES: u32 = 14;
pub const SWAY_IPC_READ_CHUNK_BYTES: u32 = 4096;
pub const SWAY_IPC_RUN_COMMAND: u32 = 0;
pub const SWAY_IPC_SUBSCRIBE: u32 = 2;
pub const SWAY_IPC_GET_TREE: u32 = 4;
pub const SWAY_IPC_GET_INPUTS: u32 = 100;
pub const SWAY_IPC_EVENT_WINDOW: u32 = 0x8000_0003;
pub const SWAY_IPC_EVENT_INPUT: u32 = 0x8000_0015;
pub const KDE_WINDOW_ID_CACHE_LIMIT: u32 = 512;
pub const KDE_WINDOW_ID_START: u32 = 1000;
pub const WAYLAND_ACCESS_FILE_MODE: u32 = 0o644;

pub const WAYLAND_FORWARDED_LEDS: [u32; 3] = [LED_NUML, LED_CAPSL, LED_SCROLLL];
pub const EVDEV_SHIFT_KEYS: [u32; 2] = [EVDEV_KEY_LEFTSHIFT, EVDEV_KEY_RIGHTSHIFT];
pub const EVDEV_CONTROL_KEYS: [u32; 2] = [EVDEV_KEY_LEFTCTRL, EVDEV_KEY_RIGHTCTRL];
pub const EVDEV_ALT_KEYS: [u32; 2] = [EVDEV_KEY_LEFTALT, EVDEV_KEY_RIGHTALT];
pub const EVDEV_SUPER_KEYS: [u32; 2] = [EVDEV_KEY_LEFTMETA, EVDEV_KEY_RIGHTMETA];
pub const EVDEV_MODIFIER_KEYS: [u32; 8] = [
    EVDEV_KEY_LEFTSHIFT,
    EVDEV_KEY_RIGHTSHIFT,
    EVDEV_KEY_LEFTCTRL,
    EVDEV_KEY_RIGHTCTRL,
    EVDEV_KEY_LEFTALT,
    EVDEV_KEY_RIGHTALT,
    EVDEV_KEY_LEFTMETA,
    EVDEV_KEY_RIGHTMETA,
];
pub const EVDEV_REQUIRED_KEYBOARD_KEYS: [u32; 4] = [EVDEV_KEY_A, EVDEV_KEY_Z, EVDEV_KEY_SPACE, EVDEV_KEY_ENTER];

// Timings of the Wayland layer (`constants/timing.py`), seconds unless the name says milliseconds.
pub const LAYOUT_SWITCH_TIMEOUT_SECONDS: f64 = 0.5;
pub const LAYOUT_SWITCH_POLL_SECONDS: f64 = 0.01;
pub const MODIFIER_RECHECK_GAP_MS: u32 = 1000;
pub const WAYLAND_TYPED_INPUT_SETTLE_SECONDS: f64 = 0.15;
pub const WAYLAND_HELPER_VERDICT_TIMEOUT_MS: u32 = 250;
pub const WAYLAND_HELPER_POLL_TIMEOUT_MS: i32 = 200;
/// Pause before an input helper that died is started again (`WAYLAND_HELPER_RESTARTS`).
pub const WAYLAND_HELPER_RESTART_DELAY_SECONDS: f64 = 1.0;
pub const WAYLAND_HELPER_COMMAND_TIMEOUT_SECONDS: f64 = 1.0;
pub const WAYLAND_UINPUT_SETTLE_SECONDS: f64 = 0.2;
/// Pause between bursts of a long injection (`WAYLAND_UINPUT_BURST_TRANSITIONS`).
pub const WAYLAND_UINPUT_BURST_PAUSE_SECONDS: f64 = 0.003;
pub const WAYLAND_GRAB_WAIT_TIMEOUT_SECONDS: f64 = 2.0;
pub const WAYLAND_GRAB_POLL_SECONDS: f64 = 0.005;
pub const WAYLAND_GRAB_RELEASE_SETTLE_SECONDS: f64 = 0.001;
pub const WAYLAND_REPEAT_DEFAULT_DELAY_MS: u32 = 600;
pub const WAYLAND_IPC_TIMEOUT_SECONDS: f64 = 1.0;
pub const WAYLAND_KEYMAP_WAIT_SECONDS: f64 = 1.5;
pub const WAYLAND_DBUS_CALL_TIMEOUT_MS: u64 = 1000;
pub const LEARNING_PROMPT_WINDOW_TITLE: &str = "Обучение KeySwitch";
pub const MICROSECONDS_PER_MILLISECOND: i64 = 1000;
