"""Wayland unit and VM E2E test fixture values."""

from __future__ import annotations

from typing import Final

# Desktop compositor geometry, PIDs, window IDs, and cache limits.
WL_FIXTURE_WINDOW_ID: Final = 42
WL_FIXTURE_SECOND_WINDOW_ID: Final = 77
WL_FIXTURE_THIRD_WINDOW_ID: Final = 99
WL_FIXTURE_PROCESS_ID: Final = 4242
WL_FIXTURE_SECOND_PROCESS_ID: Final = 5151
WL_FIXTURE_OWN_PROCESS_ID: Final = 9001
WL_FIXTURE_WINDOW_X_PIXELS: Final = 120
WL_FIXTURE_WINDOW_Y_PIXELS: Final = 240
WL_FIXTURE_WINDOW_WIDTH_PIXELS: Final = 800
WL_FIXTURE_WINDOW_HEIGHT_PIXELS: Final = 600
WL_FIXTURE_POINTER_X_PIXELS: Final = 350
WL_FIXTURE_POINTER_Y_PIXELS: Final = 410
WL_FIXTURE_POPUP_X_PIXELS: Final = 310
WL_FIXTURE_POPUP_Y_PIXELS: Final = 190
WL_FIXTURE_ANCHOR_X_PIXELS: Final = 400
WL_FIXTURE_ANCHOR_Y_PIXELS: Final = 300
WL_FIXTURE_SMALL_CACHE_LIMIT: Final = 2
WL_FIXTURE_FLOAT_COORD: Final = 128.9
# A sway client inside its container: a border on the left, a title bar above.
WL_FIXTURE_BORDER_PIXELS: Final = 2
WL_FIXTURE_TITLE_BAR_PIXELS: Final = 25
WL_FIXTURE_OUT_OF_RANGE_LAYOUT_INDEX: Final = 5
WL_FIXTURE_SHORT_HEADER_BYTES: Final = 4
WL_FIXTURE_SAMPLE_IPC_TYPE: Final = 4

# Helper framing, timestamps, file descriptors, and keycodes.
WL_FIXTURE_CHUNK_SPLIT_BYTES: Final = 3
WL_FIXTURE_EVENT_SEC: Final = 12
WL_FIXTURE_EVENT_USEC: Final = 345000
WL_FIXTURE_EXPECTED_EVENT_TIMESTAMP_MS: Final = 12345
WL_FIXTURE_CHANNEL_FD: Final = 3
WL_FIXTURE_HELPER_ARG_FD: Final = 7
WL_FIXTURE_UINPUT_FD: Final = 10
WL_FIXTURE_INOTIFY_FD: Final = 11
WL_FIXTURE_FIRST_KEYBOARD_FD: Final = 20
WL_FIXTURE_SECOND_KEYBOARD_FD: Final = 21
WL_FIXTURE_THIRD_KEYBOARD_FD: Final = 22
WL_FIXTURE_POINTER_FD: Final = 30
WL_FIXTURE_GENERIC_VENDOR_ID: Final = 0x046D
WL_FIXTURE_GENERIC_PRODUCT_ID: Final = 0xC31C
WL_FIXTURE_UNKNOWN_COMMAND_ID: Final = 250
WL_FIXTURE_UNKNOWN_MESSAGE_ID: Final = 251
WL_FIXTURE_UNKNOWN_KEY_VALUE: Final = 3
WL_FIXTURE_CUSTOM_REPEAT_DELAY_MS: Final = 300
WL_FIXTURE_SHORT_HOLD_SECONDS: Final = 0.1
WL_FIXTURE_LONG_HOLD_SECONDS: Final = 0.5
WL_FIXTURE_ADVANCE_PAST_VERDICT_SECONDS: Final = 0.3
# Taps in one injection, more than one uinput burst holds.
WL_FIXTURE_LONG_BATCH_TAPS: Final = 10
# Helper key numbers of the keys in one backend scenario.
WL_FIXTURE_SEQ_FIRST: Final = 5
WL_FIXTURE_SEQ_SECOND: Final = 6
WL_FIXTURE_SEQ_THIRD: Final = 7
# Key numbers on either side of the helper's 32-bit wrap.
WL_FIXTURE_SEQ_BEFORE_WRAP: Final = 0xFFFFFFFE
WL_FIXTURE_SEQ_AFTER_WRAP: Final = 3
WL_FIXTURE_ADVANCE_PAST_GRAB_WAIT_SECONDS: Final = 1.0
WL_FIXTURE_SHORT_TIMEOUT_SECONDS: Final = 0.05
WL_FIXTURE_POLLOUT_MASK: Final = 4
WL_FIXTURE_REQ_ID_ONE: Final = 2
WL_FIXTURE_REQ_ID_UNKNOWN: Final = 999
WL_FIXTURE_SEQ_STALE: Final = 888
WL_FIXTURE_EMIT_COUNT_TWO: Final = 2
WL_FIXTURE_KEY_G: Final = 34
WL_FIXTURE_KEY_H: Final = 35
WL_FIXTURE_KEY_B: Final = 48
WL_FIXTURE_KEY_D: Final = 32
WL_FIXTURE_KEY_T: Final = 20
WL_FIXTURE_KEY_N: Final = 49
WL_FIXTURE_KEY_J: Final = 36
WL_FIXTURE_KEY_R: Final = 19
WL_FIXTURE_KEY_F: Final = 33
WL_FIXTURE_KEY_E: Final = 18
WL_FIXTURE_KEY_U: Final = 22
WL_FIXTURE_KEY_S: Final = 31
WL_FIXTURE_KEY_L: Final = 38
WL_FIXTURE_KEY_O: Final = 24
WL_FIXTURE_UNNAMED_KEYCODE: Final = 240

# WaylandBackend timestamps, layout indices, and expected counts.
WL_FIXTURE_LAYOUT_EN_INDEX: Final = 0
WL_FIXTURE_LAYOUT_RU_INDEX: Final = 1
WL_FIXTURE_LAYOUT_DE_INDEX: Final = 2
WL_FIXTURE_INVALID_GROUP: Final = 2
WL_FIXTURE_INVALID_SOURCE_GROUP: Final = 3
WL_FIXTURE_TIMESTAMP_BASE_MS: Final = 1000
WL_FIXTURE_TIMESTAMP_STEP_MS: Final = 25
WL_FIXTURE_MONOTONIC_NOW_MS: Final = 1015
WL_FIXTURE_EXPECTED_LATENCY_MS: Final = 15
WL_FIXTURE_POLL_DEADLINE_READINGS: Final = (10.0, 10.1, 20.0)
WL_FIXTURE_EXPECTED_TWO_EVENTS: Final = 2
WL_FIXTURE_EXPECTED_THREE_BATCHES: Final = 3
WL_FIXTURE_EXPECTED_FOUR_BACKSPACES: Final = 4
WL_FIXTURE_HELPER_EXIT_CODE: Final = 0

# Real Wayland VM end-to-end device IDs, ioctls, timings, and physical US keymap.
WL_E2E_BUS_USB: Final = 0x03
WL_E2E_KBD_VENDOR_ID: Final = 0x046D
WL_E2E_KBD_PRODUCT_ID: Final = 0xC31C
WL_E2E_HOTPLUG_PRODUCT_ID: Final = 0xC31D
# The clicking pointer is absolute, like QEMU's tablet: it lands where the test puts it (linux/uinput.h).
WL_E2E_TABLET_PRODUCT_ID: Final = 0xC078
WL_E2E_UI_SET_ABSBIT: Final = 0x40045567
WL_E2E_UI_ABS_SETUP: Final = 0x401C5504
WL_E2E_ABS_X: Final = 0x00
WL_E2E_ABS_Y: Final = 0x01
WL_E2E_ABS_MAX: Final = 32767
WL_E2E_KEY_P: Final = 25
WL_E2E_UDEV_GRAB_SETTLE_SECONDS: Final = 0.45
# Half the time of one key: 40 ms a key is fast typing, and an engine reading the field is not outrun by it.
WL_E2E_INTER_KEY_DELAY_SECONDS: Final = 0.02
WL_E2E_SLOW_KEY_DELAY_MS: Final = 75
WL_E2E_SIGSTOP_SETTLE_SECONDS: Final = 0.1
WL_E2E_WATCHDOG_DEADLINE_SECONDS: Final = 1.5
WL_E2E_POLL_INTERVAL_SECONDS: Final = 0.02
WL_E2E_FOCUS_DEADLINE_SECONDS: Final = 8.0
WL_E2E_CAPS_DEADLINE_SECONDS: Final = 2.0
WL_E2E_LOCK_SETTLE_SECONDS: Final = 0.8
WL_E2E_TIMEOUT_SECONDS: Final = 150
WL_E2E_DIAGNOSE_TIMEOUT_SECONDS: Final = 30
# The packaged binary loads its models before it shows the tray: 15-25 s on four cores of a VM.
WL_E2E_TRAY_READY_DEADLINE_SECONDS: Final = 60.0
WL_E2E_US_KEYMAP: Final[dict[str, tuple[int, bool]]] = {
    "1": (2, False),
    "2": (3, False),
    "3": (4, False),
    "4": (5, False),
    "5": (6, False),
    "6": (7, False),
    "7": (8, False),
    "8": (9, False),
    "9": (10, False),
    "0": (11, False),
    "-": (12, False),
    "=": (13, False),
    "q": (16, False),
    "w": (17, False),
    "e": (18, False),
    "r": (19, False),
    "t": (20, False),
    "y": (21, False),
    "u": (22, False),
    "i": (23, False),
    "o": (24, False),
    "p": (25, False),
    "[": (26, False),
    "]": (27, False),
    "a": (30, False),
    "s": (31, False),
    "d": (32, False),
    "f": (33, False),
    "g": (34, False),
    "h": (35, False),
    "j": (36, False),
    "k": (37, False),
    "l": (38, False),
    ";": (39, False),
    "'": (40, False),
    "`": (41, False),
    "\\": (43, False),
    "z": (44, False),
    "x": (45, False),
    "c": (46, False),
    "v": (47, False),
    "b": (48, False),
    "n": (49, False),
    "m": (50, False),
    ",": (51, False),
    ".": (52, False),
    "/": (53, False),
    " ": (57, False),
    "Q": (16, True),
    "W": (17, True),
    "E": (18, True),
    "R": (19, True),
    "T": (20, True),
    "Y": (21, True),
    "U": (22, True),
    "I": (23, True),
    "O": (24, True),
    "P": (25, True),
    "A": (30, True),
    "S": (31, True),
    "D": (32, True),
    "F": (33, True),
    "G": (34, True),
    "H": (35, True),
    "J": (36, True),
    "K": (37, True),
    "L": (38, True),
    "Z": (44, True),
    "X": (45, True),
    "C": (46, True),
    "V": (47, True),
    "B": (48, True),
    "N": (49, True),
    "M": (50, True),
    ":": (39, True),
    "\"": (40, True),
    "<": (51, True),
    ">": (52, True),
    "?": (53, True),
}
