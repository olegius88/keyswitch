"""Fake operating-system values used by tests."""

from __future__ import annotations

from typing import Final

# Positional argument index of the recorded fake-call fields under test: XTestFakeKeyEvent(display,
# keycode, is_press, ...) and XkbLockGroup(display, device_spec, group, ...).
XTEST_FAKE_KEY_EVENT_IS_PRESS_ARG_INDEX: Final = 2
# The positional index of the "group" argument in an XkbLockGroup call.
XKB_LOCK_GROUP_GROUP_ARG_INDEX: Final = 2
# An arbitrary HRESULT used wherever a fixture provider failure needs a code.
FIXTURE_PROVIDER_FAILURE_HRESULT: Final = -2147220991
UIA_ELEMENT_RUNTIME_ID: Final = [1, 2, 3]
UIA_OTHER_ELEMENT_RUNTIME_ID: Final = [99]
PREEXISTING_COINIT_FLAGS: Final = 2
ATSPI_FAILED_INIT_STATUS: Final = 2
FAILED_PROCESS_RETURN_CODE: Final = 2
# run-gui-test.sh's usage-error exit code when given no command.
RUN_GUI_TEST_USAGE_ERROR_EXIT_CODE: Final = 2
FILE_PERMISSION_BITS_MASK: Final = 0o777
EXPECTED_INTENT_MODEL_FILE_MODE: Final = 0o644
FAKE_FILE_DESCRIPTOR: Final = 42
WRONG_ARRAY_ITEM_SIZE: Final = 4
FAKE_TRAINER_RETURN_CODE: Final = 7
FAKE_AFFINITY_CORE_IDS: Final = {2, 4, 6, 8}
FAKE_CPU_COUNT: Final = 6
# Fake exit codes, one per mocked entry point, so a test failure names which call's return value
# went astray instead of pointing at a bare number.
FAKE_MACOS_MAIN_EXIT_CODE: Final = 11
FAKE_GTK_MAIN_EXIT_CODE: Final = 3
FAKE_WINDOW_RUN_EXIT_CODE: Final = 5
# arbitrary, only has to round-trip through main()
FAKE_DIAGNOSE_EXIT_CODE: Final = 7
MACOS_FAKE_AX_ELEMENT: Final = 0xF0C5
NON_MAPPING_PLIST: Final = [1, 2, 3]
# `git check-attr` prints "<path>: <attribute>: <value>"; splitting from the right on ": " at most
# twice yields exactly these three fields.
GIT_CHECK_ATTR_MAX_SPLITS: Final = 2
GIT_CHECK_ATTR_FIELD_COUNT: Final = 3
FAKE_HUNSPELL_HANDLE: Final = 123
# Fake GLib event-source ids of the update-check timers.
STALE_INITIAL_UPDATE_SOURCE_ID: Final = 8
STALE_PERIODIC_UPDATE_SOURCE_ID: Final = 9
SCHEDULED_INITIAL_UPDATE_SOURCE_ID: Final = 10
SCHEDULED_PERIODIC_UPDATE_SOURCE_ID: Final = 11
DISABLED_INITIAL_UPDATE_SOURCE_ID: Final = 12
CANCELLED_INITIAL_UPDATE_SOURCE_ID: Final = 13
CANCELLED_PERIODIC_UPDATE_SOURCE_ID: Final = 14
FAKE_LAUNCHER_EXIT_CODE: Final = 17
FAKE_LINUX_MAIN_EXIT_CODE: Final = 5
FAKE_WINDOWS_MAIN_EXIT_CODE: Final = 6
FAKE_APPLICATION_RUN_EXIT_CODE: Final = 3
# Standard Windows message codes, reused by the fake pystray win32 module and by the
# primary/secondary click test.
WM_LBUTTONUP: Final = 0x0202
WM_RBUTTONUP: Final = 0x0205
GTK_STALE_TEXT_SAVE_SOURCE_ID: Final = 999
GTK_FAKE_TEXT_SAVE_SOURCE_ID: Final = 123
GTK_FRESH_TEXT_SAVE_SOURCE_ID: Final = 124
# Win32 keyboard layout handles (HKL) of the two installed fake layouts: en-US and ru-RU.
ENGLISH_HKL: Final = 0x00000409
RUSSIAN_HKL: Final = 0x00000419
# An HKL with a nonzero device-handle high word but the English low word; primary_language() must
# mask that off and still read LANG_ENGLISH.
ENGLISH_HKL_WITH_DEVICE_FLAGS: Final = 0xF0010409
# HKLs with a nonzero device-handle high word, to check that current_group() keys off the low word
# (the language id) alone, like primary_language() does.
ENGLISH_HKL_WITH_HANDLE: Final = 0x12340409
RUSSIAN_HKL_WITH_HANDLE: Final = 0x12340419
# German; matches neither installed layout
GERMAN_HKL: Final = 0x00000407
# The Task Manager Startup-tab "StartupApproved" registry value: a state byte followed by padding
# whose length this fixture pins.
STARTUP_APPROVAL_PADDING_BYTES: Final = 11
# any byte outside the enabled set
STARTUP_APPROVAL_DISABLED_BYTE: Final = 0x03
# Abstract message ids for menu_activation_message(); real code passes pystray's
# win32.WM_LBUTTONUP/WM_RBUTTONUP, but the mapping logic only needs distinct values.
ABSTRACT_PRIMARY_CLICK_MESSAGE: Final = 10
ABSTRACT_MENU_CLICK_MESSAGE: Final = 20
ABSTRACT_OTHER_MESSAGE: Final = 30
# the fake run_windows_application()'s return value
FAKE_WINDOWS_UI_EXIT_CODE: Final = 9
PYTHON_SERIES_VERSION_COMPONENTS: Final = 2
# Fake connection handles returned by XOpenDisplay for the control and record X11 connections.
X11_CONTROL_DISPLAY: Final = 101
X11_RECORD_DISPLAY: Final = 102
# A smaller, generic pair of fake handles used once a backend is already "open", where the exact
# values do not matter.
X11_FAKE_CONTROL_HANDLE: Final = 1
X11_FAKE_RECORD_HANDLE: Final = 2
X11_FAKE_RECORD_CONTEXT: Final = 42
X11_SCREEN_NUMBER: Final = 2
X11_NET_WM_PID_ATOM: Final = 5
# Deliberately not CARDINAL_PROPERTY_FORMAT_BITS, to see the property is ignored.
X11_WRONG_PROPERTY_FORMAT_BITS: Final = 8
X11_XKB_VERSION_MINOR: Final = 2
X11_XTEST_VERSION_COMPONENT: Final = 2
X11_RECORD_VERSION_MINOR: Final = 13
# A non-keyboard, non-button event type, arbitrary and unrecognised.
X11_UNRECOGNIZED_EVENT_TYPE: Final = 12
