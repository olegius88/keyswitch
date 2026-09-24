"""Win32 API numbers and Windows-specific limits."""

from __future__ import annotations

from typing import Final

# Windows reports the wheel as a signed delta of this many units per notch.
WINDOWS_WHEEL_UNITS_PER_NOTCH: Final = 120
# Win32 primary language ids of the two supported layouts.
LANG_ENGLISH: Final = 0x09
LANG_RUSSIAN: Final = 0x19
# LOWORD(hkl) & PRIMARYLANGID(lgid), the Win32 macros `primary_language` inlines.
LOWORD_MASK: Final = 0xFFFF
PRIMARY_LANGID_MASK: Final = 0x03FF
# The low 32 bits of an HKL, for display; HKL is pointer-sized but only the low word carries the
# language id and the high word the layout id.
DWORD_MASK: Final = 0xFFFFFFFF
VK_BACK: Final = 0x08
VK_TAB: Final = 0x09
VK_RETURN: Final = 0x0D
VK_SHIFT: Final = 0x10
VK_CONTROL: Final = 0x11
VK_MENU: Final = 0x12
VK_PAUSE: Final = 0x13
VK_CAPITAL: Final = 0x14
VK_ESCAPE: Final = 0x1B
VK_SPACE: Final = 0x20
VK_PRIOR: Final = 0x21
VK_NEXT: Final = 0x22
VK_END: Final = 0x23
VK_HOME: Final = 0x24
VK_LEFT: Final = 0x25
VK_UP: Final = 0x26
VK_RIGHT: Final = 0x27
VK_DOWN: Final = 0x28
VK_INSERT: Final = 0x2D
VK_DELETE: Final = 0x2E
VK_LWIN: Final = 0x5B
VK_RWIN: Final = 0x5C
VK_LSHIFT: Final = 0xA0
VK_RSHIFT: Final = 0xA1
VK_LCONTROL: Final = 0xA2
VK_RCONTROL: Final = 0xA3
VK_LMENU: Final = 0xA4
VK_RMENU: Final = 0xA5
# Punctuation keys of the US layout; named after the X11 keysyms so the log reads the same on both
# platforms instead of showing "VK_BC" for a comma.
VK_OEM_1: Final = 0xBA
VK_OEM_PLUS: Final = 0xBB
VK_OEM_COMMA: Final = 0xBC
VK_OEM_MINUS: Final = 0xBD
VK_OEM_PERIOD: Final = 0xBE
VK_OEM_2: Final = 0xBF
VK_OEM_3: Final = 0xC0
VK_OEM_4: Final = 0xDB
VK_OEM_5: Final = 0xDC
VK_OEM_6: Final = 0xDD
VK_OEM_7: Final = 0xDE
# Alphanumeric keys: winuser.h defines no VK_0.. VK_9 / VK_A.. VK_Z constants because their values
# equal the ASCII digits and upper-case letters.
VK_0: Final = 0x30
VK_9: Final = 0x39
VK_A: Final = 0x41
VK_Z: Final = 0x5A
# UI Automation TextPattern id. GetCurrentPattern answers S_OK with a null pointer when the element
# does not support it: a field without text is an ordinary answer.
UIA_TEXT_PATTERN_ID: Final = 10014
ERROR_ALREADY_EXISTS: Final = 183
SW_RESTORE: Final = 9
WH_KEYBOARD_LL: Final = 13
WH_MOUSE_LL: Final = 14
HC_ACTION: Final = 0
WM_KEYDOWN: Final = 0x0100
WM_KEYUP: Final = 0x0101
WM_SYSKEYDOWN: Final = 0x0104
WM_SYSKEYUP: Final = 0x0105
WM_QUIT: Final = 0x0012
WM_USER: Final = 0x0400
WM_INPUTLANGCHANGEREQUEST: Final = 0x0050
PM_NOREMOVE: Final = 0x0000
# Mouse messages that invalidate the caret position; observed only, never suppressed (see
# run_keyboard_hook's mouse_callback).
WM_LBUTTONDOWN: Final = 0x0201
WM_RBUTTONDOWN: Final = 0x0204
WM_MBUTTONDOWN: Final = 0x0207
WM_MOUSEWHEEL: Final = 0x020A
WM_XBUTTONDOWN: Final = 0x020B
WM_MOUSEHWHEEL: Final = 0x020E
LLKHF_EXTENDED: Final = 0x01
LLKHF_INJECTED: Final = 0x10
KEYEVENTF_EXTENDEDKEY: Final = 0x0001
KEYEVENTF_KEYUP: Final = 0x0002
KEYEVENTF_SCANCODE: Final = 0x0008
INPUT_KEYBOARD: Final = 1
PROCESS_QUERY_LIMITED_INFORMATION: Final = 0x1000
# dwExtraInfo marks on the keys KeySwitch sends itself (synthetic) and replays.
KEYSWITCH_EXTRA_INFO: Final = 0x4B535743
KEYSWITCH_REPLAY_INFO: Final = 0x4B535752
GA_ROOT: Final = 2
GWL_EXSTYLE: Final = -20
WS_EX_TOOLWINDOW: Final = 0x00000080
WS_EX_NOACTIVATE: Final = 0x08000000
SWP_NOSIZE: Final = 0x0001
SWP_NOMOVE: Final = 0x0002
SWP_NOZORDER: Final = 0x0004
SWP_NOACTIVATE: Final = 0x0010
SWP_FRAMECHANGED: Final = 0x0020
# A pointer to a 256-byte array holding one keyboard-state byte per virtual key (ToUnicodeEx's
# lpKeyState / GetKeyboardState). High bit set means down.
KEYBOARD_STATE_ARRAY_SIZE: Final = 256
VK_STATE_DOWN_BIT: Final = 0x80
VIRTUAL_KEY_BYTE_MASK: Final = 0xFF
TRANSLATED_TEXT_BUFFER_CHARACTERS: Final = 8
# ToUnicodeEx wFlags bit 2 (Windows 10 1607+): keyboard state is not changed.
TO_UNICODE_KEEP_KEYBOARD_STATE_FLAG: Final = 0x04
# QueryFullProcessImageNameW's output buffer, in wide characters.
PROCESS_IMAGE_NAME_BUFFER_CHARACTERS: Final = 32768
# StartupApproved\Run's first byte: 0x02 and 0x06 mean enabled, anything else (0x03 from Task
# Manager, 0x01 from older builds) means the value is skipped.
STARTUP_APPROVAL_ENABLED_BYTES: Final = (0x02, 0x06)
SHIFT_KEYS: Final = frozenset((VK_SHIFT, VK_LSHIFT, VK_RSHIFT))
CONTROL_KEYS: Final = frozenset((VK_CONTROL, VK_LCONTROL, VK_RCONTROL))
ALT_KEYS: Final = frozenset((VK_MENU, VK_LMENU, VK_RMENU))
SUPER_KEYS: Final = frozenset((VK_LWIN, VK_RWIN))
POINTER_INVALIDATING_MESSAGES: Final = frozenset({WM_LBUTTONDOWN, WM_RBUTTONDOWN, WM_MBUTTONDOWN, WM_MOUSEWHEEL, WM_XBUTTONDOWN, WM_MOUSEHWHEEL})
