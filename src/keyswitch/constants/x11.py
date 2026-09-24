"""X11, XKB and XRecord protocol numbers and keysyms."""

from __future__ import annotations

from typing import Final

# X11 reports the wheel as button presses; Windows reports a signed delta.
X11_WHEEL_UP_BUTTON: Final = 4
X11_WHEEL_DOWN_BUTTON: Final = 5
X11_KEY_PRESS: Final = 2
X11_BUTTON_PRESS: Final = 4
X11_KEY_RELEASE: Final = 3
XRECORD_FROM_SERVER: Final = 0
XRECORD_START_OF_DATA: Final = 4
XRECORD_ALL_CLIENTS: Final = 3
XKB_USE_CORE_KBD: Final = 0x0100
X11_REVERT_TO_PARENT: Final = 2
X11_CURRENT_TIME: Final = 0
XA_CARDINAL: Final = 6
MAX_XKB_GROUPS: Final = 4
# XRecord delivers each event as a fixed-size, 4-byte-unit record.
XRECORD_DATA_UNIT_BYTES: Final = 4
XRECORD_EVENT_SIZE_BYTES: Final = 32
# Bit 7 of the XRecord event type is the "send event" flag, not part of it.
XRECORD_EVENT_TYPE_MASK: Final = 0x7F
# The XKB group occupies bits 13-14 of the X11 key event state field.
XKB_GROUP_STATE_SHIFT: Final = 13
XKB_GROUP_STATE_MASK: Final = 0x3
MAX_TRACKED_OWN_WINDOWS: Final = 256
# A toolkit gives the focus to a child window with no properties of its own; the ancestor walk that
# looks for _NET_WM_PID or the class hint is bounded.
MAX_WINDOW_ANCESTOR_DEPTH: Final = 16
CARDINAL_PROPERTY_FORMAT_BITS: Final = 32
BACKSPACE_KEYSYM: Final = 0xFF08
SHIFT_L_KEYSYM: Final = 0xFFE1
