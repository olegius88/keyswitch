"""Quartz and Cocoa numbers and macOS-specific limits."""

from __future__ import annotations

from typing import Final

# Virtual key codes, read from Apple's Events.h. They name physical positions and do not move with
# the layout, which is what the engine's key codes mean.
MAC_VK_ANSI_Z: Final = 0x06
MAC_VK_ANSI_Q: Final = 0x0C
MAC_VK_PERIOD: Final = 0x2F
MAC_VK_KEYPAD_ENTER: Final = 0x4C
MAC_VK_RETURN: Final = 0x24
MAC_VK_TAB: Final = 0x30
MAC_VK_SPACE: Final = 0x31
MAC_VK_BACKSPACE: Final = 0x33
MAC_VK_ESCAPE: Final = 0x35
MAC_VK_COMMAND: Final = 0x37
MAC_VK_RIGHT_COMMAND: Final = 0x36
MAC_VK_SHIFT: Final = 0x38
MAC_VK_CAPS_LOCK: Final = 0x39
MAC_VK_OPTION: Final = 0x3A
MAC_VK_CONTROL: Final = 0x3B
MAC_VK_RIGHT_SHIFT: Final = 0x3C
MAC_VK_RIGHT_OPTION: Final = 0x3D
MAC_VK_RIGHT_CONTROL: Final = 0x3E
MAC_VK_FUNCTION: Final = 0x3F
MAC_VK_HELP: Final = 0x72
MAC_VK_HOME: Final = 0x73
MAC_VK_PAGE_UP: Final = 0x74
MAC_VK_FORWARD_DELETE: Final = 0x75
MAC_VK_END: Final = 0x77
MAC_VK_PAGE_DOWN: Final = 0x79
MAC_VK_LEFT_ARROW: Final = 0x7B
MAC_VK_RIGHT_ARROW: Final = 0x7C
MAC_VK_DOWN_ARROW: Final = 0x7D
MAC_VK_UP_ARROW: Final = 0x7E
# The event tap reports these instead of a key when the system disabled it.
EVENT_TAP_DISABLED_BY_TIMEOUT: Final = 0xFFFFFFFE
EVENT_TAP_DISABLED_BY_USER_INPUT: Final = 0xFFFFFFFF
# Event numbers, from IOLLEvent.h through CGEventTypes.h.
EVENT_LEFT_MOUSE_DOWN: Final = 1
EVENT_RIGHT_MOUSE_DOWN: Final = 3
EVENT_KEY_DOWN: Final = 10
EVENT_KEY_UP: Final = 11
EVENT_FLAGS_CHANGED: Final = 12
EVENT_SCROLL_WHEEL: Final = 22
EVENT_OTHER_MOUSE_DOWN: Final = 25
# Modifier bits of a Quartz event, from IOLLEvent.h.
EVENT_FLAG_ALPHA_SHIFT: Final = 0x00010000
EVENT_FLAG_SHIFT: Final = 0x00020000
EVENT_FLAG_CONTROL: Final = 0x00040000
EVENT_FLAG_ALTERNATE: Final = 0x00080000
EVENT_FLAG_COMMAND: Final = 0x00100000
# Tap placement and options.
SESSION_EVENT_TAP: Final = 1
HEAD_INSERT_EVENT_TAP: Final = 0
EVENT_TAP_OPTION_DEFAULT: Final = 0
KEYBOARD_EVENT_KEYCODE_FIELD: Final = 9
EVENT_SOURCE_USER_DATA_FIELD: Final = 42
# The marks the backend puts on events it posts itself. A tap sees its own injections like any other
# event, and without a mark it would answer its own corrections as though the user had typed them.
EVENT_MARK_INJECTED: Final = 0x4B53_0001
EVENT_MARK_REPLAYED: Final = 0x4B53_0002
# UCKeyTranslate arguments.
UC_KEY_ACTION_DISPLAY: Final = 3
UC_KEY_TRANSLATE_NO_DEAD_KEYS_MASK: Final = 1
# shiftKey (0x0200) >> 8, the shape UCKeyTranslate wants.
UC_MODIFIER_SHIFT: Final = 2
# alphaLock (0x0400) >> 8.
UC_MODIFIER_ALPHA_LOCK: Final = 4
# optionKey (0x0800) >> 8.
UC_MODIFIER_OPTION: Final = 8
# controlKey (0x1000) >> 8.
UC_MODIFIER_CONTROL: Final = 16
# UTF-16 units of the buffer UCKeyTranslate writes a key's text into.
UC_KEY_TRANSLATE_BUFFER_CHARACTERS: Final = 8
# Buffer a CFString is copied into as UTF-8.
CF_STRING_BUFFER_BYTES: Final = 512
# kCFStringEncodingUTF8.
CF_STRING_ENCODING_UTF8: Final = 0x08000100
CF_NUMBER_SINT32_TYPE: Final = 3
WINDOW_LIST_ON_SCREEN_ONLY: Final = 1 << 0
WINDOW_LIST_EXCLUDE_DESKTOP: Final = 1 << 4
NULL_WINDOW_ID: Final = 0
# A window the user types into sits on the normal layer; menus, the dock and overlays sit above it
# and would otherwise be mistaken for the focus.
NORMAL_WINDOW_LAYER: Final = 0
# Index of the program name within the (window id, owner pid, name) tuple
# `CtypesMacAPI._front_window` returns.
FRONT_WINDOW_NAME_INDEX: Final = 2
AX_SUCCESS: Final = 0
# AXValue wrapping a CFRange, from AXValue.h.
AX_VALUE_TYPE_CF_RANGE: Final = 4
# NSApplication presents no dock icon and no menu bar of its own.
ACTIVATION_POLICY_ACCESSORY: Final = 1
# NSStatusItem asks for the width its content needs.
VARIABLE_STATUS_ITEM_LENGTH: Final = -1.0
# NSControlStateValue of a checked and an unchecked menu item.
CONTROL_STATE_ON: Final = 1
CONTROL_STATE_OFF: Final = 0
MAC_SHIFT_KEYS: Final = frozenset({MAC_VK_SHIFT, MAC_VK_RIGHT_SHIFT})
MAC_CONTROL_KEYS: Final = frozenset({MAC_VK_CONTROL, MAC_VK_RIGHT_CONTROL})
MAC_ALT_KEYS: Final = frozenset({MAC_VK_OPTION, MAC_VK_RIGHT_OPTION})
MAC_SUPER_KEYS: Final = frozenset({MAC_VK_COMMAND, MAC_VK_RIGHT_COMMAND})
MAC_MODIFIER_KEYCODES: Final = MAC_SHIFT_KEYS | MAC_CONTROL_KEYS | MAC_ALT_KEYS | MAC_SUPER_KEYS | {MAC_VK_CAPS_LOCK, MAC_VK_FUNCTION}
# The key whose translation tells which script a layout types.
MAC_SCRIPT_PROBE_KEYCODE: Final = MAC_VK_ANSI_Q
EVENT_TAP_DISABLED_TYPES: Final = frozenset({EVENT_TAP_DISABLED_BY_TIMEOUT, EVENT_TAP_DISABLED_BY_USER_INPUT})
POINTER_EVENTS: Final = frozenset({EVENT_LEFT_MOUSE_DOWN, EVENT_RIGHT_MOUSE_DOWN, EVENT_OTHER_MOUSE_DOWN, EVENT_SCROLL_WHEEL})
KEY_EVENTS: Final = frozenset({EVENT_KEY_DOWN, EVENT_KEY_UP, EVENT_FLAGS_CHANGED})
TAP_DISABLED_EVENTS: Final = frozenset({EVENT_TAP_DISABLED_BY_TIMEOUT, EVENT_TAP_DISABLED_BY_USER_INPUT})
