"""Linux evdev, uinput, Wayland XKB and compositor IPC protocol numbers."""

from __future__ import annotations

from typing import Final

# Linux input event types (linux/input-event-codes.h).
EV_SYN: Final = 0x00
EV_KEY: Final = 0x01
EV_REL: Final = 0x02
EV_ABS: Final = 0x03
EV_MSC: Final = 0x04
EV_LED: Final = 0x11
EV_REP: Final = 0x14

SYN_REPORT: Final = 0
KEY_VALUE_RELEASE: Final = 0
KEY_VALUE_PRESS: Final = 1
KEY_VALUE_REPEAT: Final = 2

LED_NUML: Final = 0x00
LED_CAPSL: Final = 0x01
LED_SCROLLL: Final = 0x02
WAYLAND_FORWARDED_LEDS: Final = (LED_NUML, LED_CAPSL, LED_SCROLLL)

# Virtual keyboard identity (BUS_I8042 keeps laptop Disable-While-Typing paired in libinput;
# vendor/product tag our virtual device so the input helper never grabs itself).
BUS_I8042: Final = 0x11
WAYLAND_UINPUT_VENDOR_ID: Final = 0x1209
WAYLAND_UINPUT_PRODUCT_ID: Final = 0x4B53
WAYLAND_UINPUT_VERSION: Final = 1
WAYLAND_UINPUT_MAX_KEYCODE: Final = 248

# Kernel uinput and evdev ioctl request numbers on Linux (linux/uinput.h, linux/input.h).
UI_SET_EVBIT: Final = 0x40045564
UI_SET_KEYBIT: Final = 0x40045565
UI_SET_LEDBIT: Final = 0x40045569
UI_DEV_SETUP: Final = 0x405C5503
UI_DEV_CREATE: Final = 0x5501
UI_DEV_DESTROY: Final = 0x5502

EVIOCGID: Final = 0x80084502
EVIOCGNAME_256: Final = 0x81004506
EVIOCGPHYS_256: Final = 0x81004507
EVIOCGKEY_96: Final = 0x80604518
EVIOCGLED_8: Final = 0x80084519
EVIOCGBIT_EV_8: Final = 0x80084520
EVIOCGBIT_KEY_96: Final = 0x80604521
EVIOCGBIT_REL_8: Final = 0x80084522
EVIOCGBIT_ABS_8: Final = 0x80084523
EVIOCGRAB: Final = 0x40044590
EVIOCSCLOCKID: Final = 0x400445A0

EVDEV_NAME_BUFFER_BYTES: Final = 256
EVDEV_KEY_BITMAP_BYTES: Final = 96
EVDEV_SMALL_BITMAP_BYTES: Final = 8
EVDEV_READ_BATCH_EVENTS: Final = 64
UINPUT_MAX_NAME_BYTES: Final = 80
UINT32_MODULO_MASK: Final = 0xFFFFFFFF

# Evdev keycodes for keyboard classification, modifier tracking, and fallback naming.
EVDEV_KEY_ESC: Final = 1
EVDEV_KEY_1: Final = 2
EVDEV_KEY_0: Final = 11
EVDEV_KEY_MINUS: Final = 12
EVDEV_KEY_EQUAL: Final = 13
EVDEV_KEY_BACKSPACE: Final = 14
EVDEV_KEY_TAB: Final = 15
EVDEV_KEY_Q: Final = 16
EVDEV_KEY_LEFTBRACE: Final = 26
EVDEV_KEY_RIGHTBRACE: Final = 27
EVDEV_KEY_ENTER: Final = 28
EVDEV_KEY_LEFTCTRL: Final = 29
EVDEV_KEY_A: Final = 30
EVDEV_KEY_SEMICOLON: Final = 39
EVDEV_KEY_APOSTROPHE: Final = 40
EVDEV_KEY_GRAVE: Final = 41
EVDEV_KEY_LEFTSHIFT: Final = 42
EVDEV_KEY_BACKSLASH: Final = 43
EVDEV_KEY_Z: Final = 44
EVDEV_KEY_COMMA: Final = 51
EVDEV_KEY_DOT: Final = 52
EVDEV_KEY_SLASH: Final = 53
EVDEV_KEY_RIGHTSHIFT: Final = 54
EVDEV_KEY_LEFTALT: Final = 56
EVDEV_KEY_SPACE: Final = 57
EVDEV_KEY_CAPSLOCK: Final = 58
EVDEV_KEY_KPENTER: Final = 96
EVDEV_KEY_RIGHTCTRL: Final = 97
EVDEV_KEY_RIGHTALT: Final = 100
EVDEV_KEY_HOME: Final = 102
EVDEV_KEY_UP: Final = 103
EVDEV_KEY_PAGEUP: Final = 104
EVDEV_KEY_LEFT: Final = 105
EVDEV_KEY_RIGHT: Final = 106
EVDEV_KEY_END: Final = 107
EVDEV_KEY_DOWN: Final = 108
EVDEV_KEY_PAGEDOWN: Final = 109
EVDEV_KEY_INSERT: Final = 110
EVDEV_KEY_DELETE: Final = 111
EVDEV_KEY_PAUSE: Final = 119
EVDEV_KEY_LEFTMETA: Final = 125
EVDEV_KEY_RIGHTMETA: Final = 126

EVDEV_SHIFT_KEYS: Final = frozenset({EVDEV_KEY_LEFTSHIFT, EVDEV_KEY_RIGHTSHIFT})
EVDEV_CONTROL_KEYS: Final = frozenset({EVDEV_KEY_LEFTCTRL, EVDEV_KEY_RIGHTCTRL})
EVDEV_ALT_KEYS: Final = frozenset({EVDEV_KEY_LEFTALT, EVDEV_KEY_RIGHTALT})
EVDEV_SUPER_KEYS: Final = frozenset({EVDEV_KEY_LEFTMETA, EVDEV_KEY_RIGHTMETA})
EVDEV_MODIFIER_KEYS: Final = (
    EVDEV_SHIFT_KEYS | EVDEV_CONTROL_KEYS | EVDEV_ALT_KEYS | EVDEV_SUPER_KEYS
)
EVDEV_REQUIRED_KEYBOARD_KEYS: Final = frozenset({
    EVDEV_KEY_A,
    EVDEV_KEY_Z,
    EVDEV_KEY_SPACE,
    EVDEV_KEY_ENTER,
})

# Pointer / non-keyboard axes and buttons (linux/input-event-codes.h).
EVDEV_REL_X: Final = 0x00
EVDEV_REL_Y: Final = 0x01
EVDEV_ABS_MT_POSITION_X: Final = 0x35
EVDEV_BTN_MOUSE: Final = 0x110
EVDEV_BTN_TASK: Final = 0x117
EVDEV_BTN_GAMEPAD: Final = 0x130
EVDEV_BTN_STYLUS: Final = 0x14B

# Wayland client & libxkbcommon constants.
WAYLAND_XKB_KEYCODE_OFFSET: Final = 8
WL_SEAT_CAPABILITY_KEYBOARD: Final = 2
WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1: Final = 1
WAYLAND_SEAT_INTERFACE_MAX_VERSION: Final = 7
# Round trips to the compositor until wl_keyboard sends its keymap: registry, seat, keyboard.
WAYLAND_KEYMAP_ROUNDTRIPS: Final = 3
XKB_KEYSYM_NAME_BUFFER_BYTES: Final = 64

# Linux inotify flags and buffer bounds (sys/inotify.h).
IN_ATTRIB: Final = 0x00000004
IN_CREATE: Final = 0x00000100
IN_DELETE: Final = 0x00000200
IN_NONBLOCK: Final = 0x00000800
IN_CLOEXEC: Final = 0x00080000
INOTIFY_EVENT_HEADER_BYTES: Final = 16
INOTIFY_READ_BUFFER_BYTES: Final = 4096

# Binary framing between WaylandBackend and the input helper process.
HELPER_FRAME_HEADER_BYTES: Final = 5
HELPER_MAX_FRAME_BYTES: Final = 65536
HELPER_EMIT_ITEM_BYTES: Final = 3
HELPER_SOCKET_CHUNK_BYTES: Final = 4096
# Key transitions written to uinput in one go. Every reader of a keyboard device gets a queue of
# 64 events (EVDEV_MIN_BUFFER_SIZE in drivers/input/evdev.c: a key-only device estimates 8 events
# a packet, times EVDEV_BUF_PACKETS 8); one more and the kernel drops all it holds for SYN_DROPPED.
# A transition is two events (the key and SYN_REPORT): a burst of 8 fills a quarter of it.
# A correction of `проблема` written whole (72 events) vanished in GNOME 50 (27.10.2026).
WAYLAND_UINPUT_BURST_TRANSITIONS: Final = 8
# Kernel events a key transition takes in uinput: EV_KEY and its SYN_REPORT.
TRANSITION_EVENTS: Final = 2

HELPER_MSG_READY: Final = 1
HELPER_MSG_ERROR: Final = 2
HELPER_MSG_KEY: Final = 3
HELPER_MSG_POINTER: Final = 4
HELPER_MSG_CAPS_LOCK: Final = 5
HELPER_MSG_EMIT_ACK: Final = 6
HELPER_MSG_KEY_DOWN: Final = 7
HELPER_MSG_REPEAT_RESET: Final = 8
# The keys the helper's grab no longer accounts for (the grab was dropped or taken again): the parent
# forgets which keys it believes are held down.
HELPER_MSG_INPUT_RESET: Final = 9

HELPER_CMD_VERDICT: Final = 101
HELPER_CMD_EMIT_BATCH: Final = 102
HELPER_CMD_SET_GRAB: Final = 103
HELPER_CMD_QUERY_KEY_DOWN: Final = 104
HELPER_CMD_STOP: Final = 105
# The parent has handled every key up to this seq: a pass-through after a late verdict ends here.
HELPER_CMD_SEQ_ACK: Final = 106

# Flags of a key the helper reports: it waits for the parent's verdict, or it has already typed it
# itself (the verdict came too late). A key with neither flag was typed without asking.
HELPER_KEY_NEEDS_VERDICT: Final = 1
HELPER_KEY_FORCED: Final = 2

# How the helper checks an injection against the keys it typed without a verdict: not at all (a
# replay), as text decided once the parent had handled the given seq, or as the withheld key of
# that seq itself (Enter/Tab), which must not reach the window twice.
HELPER_EMIT_UNCHECKED: Final = 0
HELPER_EMIT_TEXT: Final = 1
HELPER_EMIT_ACTION: Final = 2

# Times an input helper that died on its own is started again before KeySwitch gives up.
WAYLAND_HELPER_RESTARTS: Final = 5

HELPER_VERDICT_PASS: Final = 0
HELPER_VERDICT_SWALLOW: Final = 1

# sway / i3-ipc binary wire protocol numbers (sway-ipc(7)).
SWAY_IPC_HEADER_BYTES: Final = 14
SWAY_IPC_READ_CHUNK_BYTES: Final = 4096
SWAY_IPC_RUN_COMMAND: Final = 0
SWAY_IPC_SUBSCRIBE: Final = 2
SWAY_IPC_GET_TREE: Final = 4
SWAY_IPC_GET_INPUTS: Final = 100
SWAY_IPC_EVENT_WINDOW: Final = 0x80000003
SWAY_IPC_EVENT_INPUT: Final = 0x80000015

# Bounded window handle table for UUID-identified compositor windows (KWin 6).
KDE_WINDOW_ID_CACHE_LIMIT: Final = 512
KDE_WINDOW_ID_START: Final = 1000
