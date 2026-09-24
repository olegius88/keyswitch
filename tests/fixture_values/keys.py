"""Fake keycodes, keysyms, scan codes and window or process ids used by tests."""

from __future__ import annotations

from typing import Final

# X11 keysyms the E2E drivers tap directly; BackSpace is BACKSPACE_KEYSYM of
# keyswitch.constants.x11.
CONTROL_L_KEYSYM: Final = 0xFFE3
# The default "convert_last" hotkey (see config.py).
PAUSE_KEYSYM: Final = 0xFF13
RETURN_KEYSYM: Final = 0xFF0D
# Win32 virtual key F24: the Windows E2E probe key, which no layout maps to text.
VK_F24: Final = 0x87
# Physical (set 1) scan codes; together they spell "ghbdtn" (-> привет) or "hello" (-> руддщ) in the
# layout under test, plus Enter and Space.
SCAN_CODE_G: Final = 0x22
SCAN_CODE_H: Final = 0x23
SCAN_CODE_B: Final = 0x30
SCAN_CODE_D: Final = 0x20
SCAN_CODE_T: Final = 0x14
SCAN_CODE_N: Final = 0x31
SCAN_CODE_ENTER: Final = 0x1C
SCAN_CODE_E: Final = 0x12
SCAN_CODE_L: Final = 0x26
SCAN_CODE_O: Final = 0x18
SCAN_CODE_SPACE: Final = 0x39
# Standard PC/AT (set 1) scan codes the Windows backend tests put on fake key events.
SCAN_CODE_TAB: Final = 15
SCAN_CODE_LEFT_SHIFT: Final = 42
# A virtual-key/scan code that does not correspond to anything typed; it only needs to be tracked as
# an unrelated key still held down.
UNRELATED_HELD_KEYCODE: Final = 99
# Placeholder physical keycode for synthetic named-key events (Pause, etc).
NAMED_KEY_PLACEHOLDER_KEYCODE: Final = 200
# Physical keycode used when the test specifically plays the Pause key.
PAUSE_KEYCODE: Final = 127
# An arbitrary keycode used only as a sentinel to match a scheduled undo with the synthetic key-up
# event that should trigger it.
UNDO_TRIGGER_SENTINEL_KEYCODE: Final = 999
# A source group that is never in self.models, forcing the early guard.
BOUNDARY_UNMODELLED_SOURCE_GROUP: Final = 20
# A keyboard-layout group outside what the fixtures define.
BOUNDARY_OUT_OF_FIXTURE_LAYOUT_GROUP: Final = 99
# An arbitrary window id fed to read() wherever the id itself carries no meaning.
CONTEXT_READER_WINDOW_ID: Final = 5
# The field's process id equals the window's process id, and one that differs from it.
FIELD_READER_MATCHING_PROCESS_ID: Final = 42
FIELD_READER_MISMATCHED_PROCESS_ID: Final = 43
# The first layout group past the two supported ones (0 = EN, 1 = RU).
UNSUPPORTED_LAYOUT_GROUP: Final = 2
RECEIPT_INVALID_ACTUAL_GROUP: Final = 5
SPAN_CAPTURE_EVENT_SERIAL: Final = 100
# An arbitrary non-zero/non-one keycode for a synthesized key fixture.
CONTEXT_POLICY_FIXTURE_KEYCODE: Final = 10
# A window other than the fake backend's first window (1).
SECOND_WINDOW_ID: Final = 2
# Set 1 scan codes of the rest of the US-QWERTY letters (SCAN_CODE_A, SCAN_CODE_D, SCAN_CODE_E,
# SCAN_CODE_G, SCAN_CODE_H, SCAN_CODE_B, SCAN_CODE_T, SCAN_CODE_N, SCAN_CODE_L, SCAN_CODE_O and
# SCAN_CODE_SPACE are named above); test_action_boundary.py combines all of them into one
# physical-key-by-character map to type whole words as fake Windows key events.
SCAN_CODE_Q: Final = 16
SCAN_CODE_W: Final = 17
SCAN_CODE_R: Final = 19
SCAN_CODE_Y: Final = 21
SCAN_CODE_U: Final = 22
SCAN_CODE_I: Final = 23
SCAN_CODE_P: Final = 25
SCAN_CODE_S: Final = 31
SCAN_CODE_F: Final = 33
SCAN_CODE_J: Final = 36
SCAN_CODE_K: Final = 37
SCAN_CODE_Z: Final = 44
SCAN_CODE_X: Final = 45
SCAN_CODE_C: Final = 46
SCAN_CODE_V: Final = 47
SCAN_CODE_M: Final = 50
# X11 (evdev) keycodes of the keys the engine tests play: Space, the default word boundary.
SPACE_KEYCODE: Final = 65
# First fake keycode of a typed fixture word; each following letter takes the next keycode, so every
# letter of the word has a distinct keycode.
SYNTHETIC_KEYCODE_BASE: Final = 30
CORE_SECOND_WORD_KEYCODE_BASE: Final = 70
CORE_THIRD_WORD_KEYCODE_BASE: Final = 90
CORE_FOURTH_WORD_KEYCODE_BASE: Final = 50
COMMA_KEYCODE: Final = 59
CONTROL_L_KEYCODE: Final = 37
ALT_L_KEYCODE: Final = 64
Z_KEYCODE: Final = 52
P_KEYCODE: Final = 33
RETURN_KEYCODE: Final = 36
UNLABELLED_LAYOUT_GROUP: Final = 7
OUT_OF_RANGE_LAYOUT_GROUP: Final = 3
# A learning target group other than the two layouts a rule usually names; the store keeps it apart
# from them.
ALTERNATE_TARGET_GROUP: Final = 2
DEFAULT_SEQUENCE_RETURN_KEYCODE: Final = 104
# A layout group no fixture model or backend defines.
NONEXISTENT_LAYOUT_GROUP: Final = 9
THIRD_LANGUAGE_GROUP: Final = 2
DETECTOR_DEFAULT_NAMED_KEY_KEYCODE: Final = 65
# The keycode of the second letter of a word typed from SYNTHETIC_KEYCODE_BASE.
SECOND_LETTER_KEYCODE: Final = SYNTHETIC_KEYCODE_BASE + 1
# -- fixture keycodes for the X11 keys these tests press directly ----------- The engine dispatches
# on `KeyEvent.key_name`, not on the numeric keycode, so a keycode here only has to (a) match the
# real X11 code for well-known keys where a test cares about realism and (b) stay consistent between
# a press
BACKSPACE_KEYCODE: Final = 22
ESCAPE_KEYCODE: Final = 9
A_KEYCODE: Final = 38
APOSTROPHE_KEYCODE: Final = 48
ALTERNATE_COMMA_KEYCODE: Final = 60
ARBITRARY_BACKEND_GROUP: Final = 7
# Unicode surrogate-range sweep over the Basic Multilingual Plane.
BMP_SWEEP_CODEPOINT_LIMIT: Final = 0x10000
UTF16_SURROGATE_FIRST_CODEPOINT: Final = 0xD800
UTF16_SURROGATE_LAST_CODEPOINT: Final = 0xDFFF
# Group ids absent from _indexes()/_scorers(), used to exercise "no other layout": one collides with
# neither key, the other differs from it too.
EARLY_SWITCH_UNKNOWN_SOURCE_GROUP: Final = 5
EARLY_SWITCH_UNKNOWN_TARGET_GROUP: Final = 7
SYNTHETIC_LEFT_ARROW_KEYCODE: Final = 100
X11_LEFT_ARROW_KEYCODE: Final = 113
LETTER_TYPED_DURING_INJECTION_KEYCODE: Final = 99
# the physical "2" key: "2" unshifted, a quote shifted
DIGIT_TWO_KEYCODE: Final = 11
# schedule_undo's keycode, matched by the "z" release
SCHEDULED_UNDO_KEYCODE: Final = 200
# never matches a real release; forces a guard
UNMATCHED_TRIGGER_KEYCODE: Final = 999
REPLACED_TRIGGER_KEYCODE: Final = 128
EXCLUDED_APP_TRIGGER_KEYCODE: Final = 129
# Keycodes of later letters and later words typed after a word that starts at
# SYNTHETIC_KEYCODE_BASE: each word gets its own block of ten, so a new batch of key events never
# collides with keys still held from an earlier one. The values carry no meaning beyond
# distinguishing the events.
FOURTH_LETTER_KEYCODE: Final = SYNTHETIC_KEYCODE_BASE + 3
FIFTH_LETTER_KEYCODE: Final = SYNTHETIC_KEYCODE_BASE + 4
SIXTH_LETTER_KEYCODE: Final = SYNTHETIC_KEYCODE_BASE + 5
SECOND_WORD_KEYCODE_BASE: Final = SYNTHETIC_KEYCODE_BASE + 10
SECOND_WORD_SECOND_LETTER_KEYCODE: Final = SECOND_WORD_KEYCODE_BASE + 1
SECOND_WORD_FOURTH_LETTER_KEYCODE: Final = SECOND_WORD_KEYCODE_BASE + 3
SECOND_WORD_FIFTH_LETTER_KEYCODE: Final = SECOND_WORD_KEYCODE_BASE + 4
THIRD_WORD_KEYCODE_BASE: Final = SECOND_WORD_KEYCODE_BASE + 10
THIRD_WORD_FOURTH_LETTER_KEYCODE: Final = THIRD_WORD_KEYCODE_BASE + 3
THIRD_WORD_FIFTH_LETTER_KEYCODE: Final = THIRD_WORD_KEYCODE_BASE + 4
THIRD_WORD_SIXTH_LETTER_KEYCODE: Final = THIRD_WORD_KEYCODE_BASE + 5
FOURTH_WORD_KEYCODE_BASE: Final = THIRD_WORD_KEYCODE_BASE + 10
FIFTH_WORD_KEYCODE_BASE: Final = FOURTH_WORD_KEYCODE_BASE + 10
SIXTH_WORD_KEYCODE_BASE: Final = FIFTH_WORD_KEYCODE_BASE + 10
SEVENTH_WORD_KEYCODE_BASE: Final = SIXTH_WORD_KEYCODE_BASE + 10
# Engine bookkeeping fixtures (_pressed, _modifier_keycodes) that are not built from a real
# KeyEvent.
STALE_MODIFIER_KEYCODE: Final = 99
ACTIVELY_PRESSED_KEYCODE: Final = 50
HELD_MODIFIER_KEYCODE: Final = 60
THIRD_WINDOW_ID: Final = 3
FOURTH_WINDOW_ID: Final = 4
OWN_WINDOW_ID: Final = 9
ENGINE_INVALID_SOURCE_GROUP: Final = 5
# Starts well above any real key serial so fixture events are never mistaken for the engine's own
# bookkeeping; each replayed key takes the next serial.
EDITOR_REPLAY_FIRST_KEY_SERIAL: Final = 100
INTENT_EXTREME_GROUP: Final = 100
INTENT_FEATURE_ARBITRARY_CONTEXT_GROUP: Final = 42
INTENT_GOLDEN_CONTEXT_GROUP: Final = 1
INTENT_MATRIX_UNKNOWN_CONTEXT_GROUP: Final = 7
INTENT_MAX_PLAUSIBLE_CONTEXT_GROUP: Final = 63
INTENT_LEAKAGE_ARBITRARY_CONTEXT_GROUP: Final = 7
LEARNING_PROMPT_FIXTURE_WINDOW_ID: Final = 123
# Fake identifiers the substitute macOS API hands back, distinctive enough that a mix-up between
# them would show up immediately.
MACOS_FAKE_WINDOW_ID: Final = 101
# Process id a fake platform API reports for the foreground window, distinct from KeySwitch's own.
FAKE_FOREGROUND_PROCESS_ID: Final = 4242
MACOS_OWN_PROCESS_ID: Final = 7
# A keycode with no name in KEY_NAMES, to exercise the VK_<hex> fallback.
MACOS_UNNAMED_VIRTUAL_KEYCODE: Final = 0x5A
# Window ids distinct from MACOS_FAKE_WINDOW_ID, standing in for the focus having moved somewhere
# else.
MACOS_WINDOW_ID_CHANGED_DURING_DEFERRAL: Final = 999
MACOS_WINDOW_ID_CHANGED_DURING_HOLD: Final = 555
MACOS_WINDOW_ID_DURING_SECOND_HOLD: Final = 888
# Group indices outside the {0, 1} pair, used to provoke a refusal.
MACOS_UNKNOWN_TARGET_GROUP: Final = 5
MACOS_GROUP_OUTSIDE_PAIR: Final = 7
MACOS_UNKNOWN_SOURCE_GROUP: Final = 9
# Arbitrary window ids: the backend ignores them on macOS, so any value proves the point.
MACOS_RESTORE_WINDOW_ID: Final = 123
MACOS_INACTIVE_WINDOW_ID: Final = 456
MACOS_CONTEXT_WINDOW_ID: Final = 42
# Layout groups of the two supported languages.
ENGLISH_LAYOUT_GROUP: Final = 0
RUSSIAN_LAYOUT_GROUP: Final = 1
# PC/AT Set 1 scan codes the Windows fixtures inject; the backend only has to keep the same physical
# key across a press/release.
SCAN_CODE_A: Final = 30
SCAN_CODE_SEMICOLON: Final = 39
# 'S'; only has to differ from SCAN_CODE_A
LATE_KEY_SCAN_CODE: Final = 31
# 'D'; the key held back during a correction
HELD_KEY_SCAN_CODE: Final = 32
# 'X'; stands in for the backend's own injection
INJECTED_KEY_SCAN_CODE: Final = 45
# A fake HWND and another one, reused wherever a test needs some window identity and not a
# particular one.
WINDOWS_FAKE_HWND: Final = 300
WINDOWS_OTHER_HWND: Final = 301
WINDOWS_OWN_PROCESS_ID: Final = 7777
# no KEY_NAMES entry; exercises the VK_xx fallback
WINDOWS_UNNAMED_VIRTUAL_KEY: Final = 0xFE
# the top-level ancestor GetAncestor(GA_ROOT) hands back
WINDOWS_ROOT_HWND: Final = 900
WINDOWS_INACTIVE_HWND: Final = 55
# only groups 0 and 1 exist
WINDOWS_INVALID_SOURCE_GROUP: Final = 7
SHIFT_L_KEYCODE: Final = 50
# An arbitrary keycode distinct from A_KEYCODE, to see it does not match.
X11_UNMATCHED_KEYCODE: Final = 39
ASCII_SPACE_CODEPOINT: Final = 32
# Fake window ids reused across the focus/ancestry fixtures.
X11_ROOT_WINDOW: Final = 99
X11_OWN_WINDOW: Final = 777
X11_OWN_CHILD_WINDOW: Final = 778
# a window that is its own parent, acting as a root
X11_OTHER_ROOT_WINDOW: Final = 779
X11_FOREIGN_WINDOW: Final = 555
# examined while its ancestry walk is still in flight
X11_LATER_WINDOW: Final = 780
X11_POSITION_TARGET_WINDOW: Final = 55
X11_POINTER_CHILD_WINDOW: Final = 100
X11_FOCUS_WINDOW: Final = 10
X11_PARENT_WINDOW: Final = 20
X11_DEEP_TOWER_END_WINDOW: Final = 900
X11_WINDOW_CACHE_TEST_START: Final = 1000
X11_XKB_REPORTED_GROUP: Final = 3
