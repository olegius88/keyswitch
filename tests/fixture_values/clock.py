"""Fake timestamps and durations used by tests."""

from __future__ import annotations

from typing import Final

from keyswitch.constants.settings_defaults import DEFAULT_PAUSE_DELAY_SECONDS

# The AT-SPI-unavailable probe subprocess must finish within this long.
ATSPI_UNAVAILABLE_PROBE_TIMEOUT_SECONDS: Final = 10
CONTEXT_ACCESS_E2E_READ_DEADLINE_SECONDS: Final = 3
CONTEXT_ACCESS_E2E_PREPARE_SETTLE_DELAY_MS: Final = 300
CONTEXT_ACCESS_E2E_START_READ_DELAY_MS: Final = 100
CONTEXT_ACCESS_E2E_INITIAL_PREPARE_DELAY_MS: Final = 600
CONTEXT_ACCESS_E2E_TIMEOUT_SECONDS: Final = 15
CONTEXT_ACCESS_E2E_WORKER_JOIN_TIMEOUT_SECONDS: Final = 4
CONTEXT_ACCESS_E2E_POLL_SECONDS: Final = 0.05
# XTestFakeKeyEvent's last argument is a delay in milliseconds before the event plays; the X11 E2E
# drivers use these for presses and releases.
XTEST_KEY_PRESS_DELAY_MS: Final = 18
XTEST_KEY_RELEASE_DELAY_MS: Final = 8
# A `gdbus call` subprocess of an E2E driver must finish within this long.
GDBUS_CALL_TIMEOUT_SECONDS: Final = 10
NATIVE_PACKAGE_E2E_GRACEFUL_SHUTDOWN_TIMEOUT_SECONDS: Final = 8
NATIVE_PACKAGE_E2E_FORCED_SHUTDOWN_TIMEOUT_SECONDS: Final = 3
NATIVE_PACKAGE_E2E_TIMEOUT_SECONDS: Final = 50
NATIVE_PACKAGE_E2E_APPLICATION_POLL_MS: Final = 100
NATIVE_PACKAGE_E2E_TRAY_READY_TO_TYPING_DELAY_MS: Final = 300
# Pause of the X11 E2E drivers between one scripted typing case and the next.
E2E_INTER_CASE_DELAY_MS: Final = 200
# Time the X11 E2E drivers let a correction or learning prompt settle before the next check.
E2E_VERIFY_SETTLE_DELAY_MS: Final = 900
E2E_LEARNING_CONFIRMATION_VERIFY_DELAY_MS: Final = 500
TRAY_MENU_E2E_READ_LINE_TIMEOUT_SECONDS: Final = 5
TRAY_MENU_E2E_EVENT_WAIT_TIMEOUT_SECONDS: Final = 2
TRAY_MENU_E2E_LOOP_JOIN_TIMEOUT_SECONDS: Final = 2
TRAY_MENU_E2E_WATCHER_EXIT_TIMEOUT_SECONDS: Final = 3
# A UI or hook step gets this long before the scenario reports it as timed out.
WINDOWS_E2E_STANDARD_DEADLINE_SECONDS: Final = 5.0
WINDOWS_E2E_LONG_DEADLINE_SECONDS: Final = 10.0
WINDOWS_E2E_ENTER_SUBMIT_TIMEOUT_SECONDS: Final = 10
# Short retry poll interval, in milliseconds, for the async wait_for_* steps.
WINDOWS_E2E_POLL_MS: Final = 50
# Slightly longer delay before starting the next phase of a scenario.
WINDOWS_E2E_PHASE_START_DELAY_MS: Final = 100
# UI settle delay, in milliseconds, after a phase completes.
WINDOWS_E2E_PHASE_SETTLE_DELAY_MS: Final = 300
WINDOWS_E2E_ENTER_PREPARE_DELAY_MS: Final = 200
WINDOWS_E2E_PROBE_TIMEOUT_SECONDS: Final = 3.0
WINDOWS_E2E_WATCHDOG_SECONDS: Final = 90
# Overall watchdog for the whole scripted run.
X11_E2E_TIMEOUT_SECONDS: Final = 45
# Pacing between scripted actions and their verification, all in milliseconds.
X11_E2E_INITIAL_STARTUP_DELAY_MS: Final = 450
X11_E2E_IDLE_PAUSE_VERIFY_DELAY_MS: Final = 2300
X11_E2E_CONTEXT_RESOLUTION_VERIFY_DELAY_MS: Final = 1200
X11_E2E_MENU_LAYOUT_VERIFY_DELAY_MS: Final = 300
X11_E2E_SLOW_TYPING_CHARACTER_DELAY_MS: Final = 150
X11_E2E_EARLY_SWITCH_SETUP_DELAY_MS: Final = 600
# A synchronous session-bus call (ListNames) in a test gets this long.
SESSION_BUS_CALL_TIMEOUT_MS: Final = 3000
# Arbitrary, non-zero starting point of a test's fake monotonic clock.
FAKE_CLOCK_START_SECONDS: Final = 1000.0
# How long a simulated key stays down, and the gap before the next event.
SIMULATED_KEY_HOLD_SECONDS: Final = 0.05
SIMULATED_KEY_RELEASE_GAP_SECONDS: Final = 0.03
# Default and short idle gaps fed to idle(), comfortably either side of the real pause-correction
# delay (DEFAULT_PAUSE_DELAY_SECONDS = 1.5).
APP_QUIRKS_IDLE_PAUSE_SECONDS: Final = 2.0
APP_QUIRKS_SHORT_IDLE_SECONDS: Final = 0.5
# A file's mtime is moved forward by this much so a cache keyed on it must reload.
MTIME_BUMP_NANOSECONDS: Final = 1_000_000
# Seconds past the last word input `idle()` reports as elapsed, enough to count as a pause
# regardless of the exact idle gate.
PAUSE_TRIGGER_OFFSET_SECONDS: Final = 2
# Monotonic-clock fixture instants shared by the manual-release watchdog tests.
MANUAL_RELEASE_KEY_PRESS_SECONDS: Final = 100.0
MANUAL_RELEASE_KEY_HELD_SINCE_SECONDS: Final = 90.0
MANUAL_RELEASE_WITHIN_DEADLINE_SECONDS: Final = 101.0
MANUAL_RELEASE_AFTER_DEADLINE_SECONDS: Final = 104.0
WAIT_PAIR_FIRST_TYPING_SECONDS: Final = 1000.0
WAIT_PAIR_SECOND_TYPING_WITHIN_TTL_SECONDS: Final = 1012.0
# Fixture instant of the engine's last word input (_last_word_input_at) in the pause-correction
# tests.
LAST_WORD_INPUT_AT_SECONDS: Final = 100.0
WAIT_PAIR_PAUSE_CHECK_SECONDS: Final = 102.0
# How far past (or short of) the pause-correction delay the fixture "now" sits.
PAST_PAUSE_THRESHOLD_MARGIN_SECONDS: Final = 0.5
BEFORE_PAUSE_THRESHOLD_MARGIN_SECONDS: Final = 0.1
# KeyEvent.timestamp is never read by the engine; these only satisfy the dataclass field, so a name
# just needs to say which fixture event it is.
BOUNDARY_EVENT_TIMESTAMP: Final = 1000
CORE_PAUSE_PRESS_TIMESTAMP: Final = 2000
CORE_PAUSE_RELEASE_TIMESTAMP: Final = 2001
CORE_ENTER_TIMESTAMP: Final = 2002
# Timestamps of the Control+Alt+Z undo chord: Control, Alt and Z down, then up in reverse.
UNDO_CHORD_CONTROL_DOWN_TIMESTAMP: Final = 3000
UNDO_CHORD_ALT_DOWN_TIMESTAMP: Final = 3001
UNDO_CHORD_Z_DOWN_TIMESTAMP: Final = 3002
UNDO_CHORD_Z_UP_TIMESTAMP: Final = 3003
UNDO_CHORD_ALT_UP_TIMESTAMP: Final = 3004
UNDO_CHORD_CONTROL_UP_TIMESTAMP: Final = 3005
CORE_FIRST_PAUSE_START_SECONDS: Final = 10.0
CORE_PAUSE_CHECK_JUST_BEFORE_SECONDS: Final = 11.49
CORE_PAUSE_CHECK_AT_THRESHOLD_SECONDS: Final = 11.5
CORE_SECOND_PAUSE_START_SECONDS: Final = 20.0
CORE_SECOND_PAUSE_CHECK_SECONDS: Final = 22.0
CORE_MANUAL_LAYOUT_PAUSE_CHECK_SECONDS: Final = 12.0
# Longer than DEFAULT_PAUSE_DELAY_SECONDS so a plain idle() always crosses it.
DEFAULT_SEQUENCE_IDLE_SECONDS: Final = 1.7
DEFAULT_SEQUENCE_SHORT_IDLE_SECONDS: Final = 1.3
DEFAULT_SEQUENCE_REMAINING_IDLE_SECONDS: Final = 0.4
DEFAULT_SEQUENCE_EXTENDED_IDLE_SECONDS: Final = 12.0
LEARNING_PROMPT_EARLY_NOW_SECONDS: Final = 10.0
JUST_BEFORE_DEADLINE_MARGIN_SECONDS: Final = 0.01
MANUAL_LAYOUT_PAUSE_NOW_SECONDS: Final = 3.0
PAUSE_GUARD_INPUT_AT_SECONDS: Final = 10.0
PAUSE_GUARD_NOW_SECONDS: Final = 12.0
QUOTE_EVENT_TIMESTAMP: Final = 500
PLAIN_KEY_TIMESTAMP: Final = 700
PROMPT_ENTER_TIMESTAMP: Final = 4000
PROMPT_ESCAPE_TIMESTAMP: Final = 4001
PROMPT_ENTER_RELEASE_TIMESTAMP: Final = 4004
PROMPT_ENTER_SHORTCUT_TIMESTAMP: Final = 4005
MODIFIER_SHORTCUT_EVENT_TIMESTAMP: Final = 800
EXCLUDED_APP_SHORTCUT_EVENT_TIMESTAMP: Final = 900
EXPIRED_PROMPT_OFFSET_SECONDS: Final = 0.1
PROMPT_DEADLINE_OFFSET_SECONDS: Final = 5
NON_DEFAULT_PAUSE_DELAY_SECONDS: Final = 0.5
JUST_BEFORE_NON_DEFAULT_PAUSE_DELAY_SECONDS: Final = 0.49
OVERSIZED_PAUSE_DELAY_SECONDS: Final = 50
UNDERSIZED_PAUSE_DELAY_SECONDS: Final = 0.01
STALE_PRESS_AGE_SECONDS: Final = 30.0
FIRST_PRUNE_CHECK_OFFSET_SECONDS: Final = 2.0
SECOND_PRUNE_CHECK_OFFSET_SECONDS: Final = 2.1
MODIFIER_DEFERRAL_CHECK_OFFSET_SECONDS: Final = 2.2
PENDING_CORRECTION_CHECK_OFFSET_SECONDS: Final = 2.3
SUCCESSFUL_CORRECTION_CHECK_OFFSET_SECONDS: Final = 2.4
IDLE_LOWER_BOUND_MS: Final = 2000
EARLY_SWITCH_STALE_AGE_SECONDS: Final = 5.0
# Safety timeout for the subprocess calls below, in seconds.
GUI_TEST_SCRIPT_TIMEOUT_SECONDS: Final = 10
INTENT_LOADER_WAIT_TIMEOUT_SECONDS: Final = 5.0
INTENT_LOADER_SHORT_POLL_SECONDS: Final = 0.1
INTENT_BARRIER_WAIT_TIMEOUT_SECONDS: Final = 5.0
# The timestamp carried by every synthetic key event; the backend passes it through unlooked-at, so
# any fixed value will do.
MACOS_FAKE_EVENT_TIMESTAMP: Final = 1000
# How long run_event_tap's stand-in blocks before giving up if stop_event is never set, so a stuck
# test cannot hang the whole suite.
MACOS_STOP_WAIT_SAFETY_TIMEOUT_SECONDS: Final = 5.0
PREFIX_TRAINING_SUBPROCESS_TIMEOUT_SECONDS: Final = 30
READER_RETRY_START_SECONDS: Final = 100.0
READER_JUST_BEFORE_FIRST_RETRY_SECONDS: Final = 104.999
READER_FIRST_RETRY_DUE_SECONDS: Final = 105.0
READER_SECOND_RETRY_DUE_SECONDS: Final = 120.0
READER_THIRD_RETRY_DUE_SECONDS: Final = 180.0
READER_CAPPED_RETRY_DUE_SECONDS: Final = 240.0
READER_FAR_FUTURE_SECONDS: Final = 5000.0
READER_OVERDUE_SECONDS: Final = 1000.0
SECONDS_JUST_UNDER_A_MINUTE: Final = 59
SECONDS_JUST_OVER_A_MINUTE: Final = 61
RELEASE_SCRIPT_CI_TIMEOUT_SECONDS: Final = 60.0
# Comfortably longer than any possible pause delay (clamped to 10s at most), so the pause always
# fires regardless of the configured delay.
PAUSE_ALWAYS_ELAPSED_SECONDS: Final = 60
UPDATE_SAMPLE_TIMEOUT_SECONDS: Final = 7.0
UPDATE_CLIENT_SAMPLE_TIMEOUT_SECONDS: Final = 9
WINDOWS_FAKE_KEY_TIMESTAMP: Final = 100
# Patched onto the keyboard listener start timeout to keep the timeout test fast.
SHORT_LISTENER_START_TIMEOUT_SECONDS: Final = 0.01
WINDOWS_CAPS_LOCK_REPEAT_TIMESTAMP: Final = 2
WINDOWS_MODIFIER_RELEASE_TIMESTAMP: Final = 3
WINDOWS_UNKNOWN_LAYOUT_PRESS_TIMESTAMP: Final = 4
WINDOWS_NO_LISTENER_RELEASE_TIMESTAMP: Final = 5
WINDOWS_INJECTED_PRESS_TIMESTAMP: Final = 123
WINDOWS_NO_FILTER_PRESS_TIMESTAMP: Final = 124
WINDOWS_FILTERED_PRESS_TIMESTAMP: Final = 125
WINDOWS_FILTERED_RELEASE_TIMESTAMP: Final = 126
WINDOWS_UNMATCHED_RELEASE_TIMESTAMP: Final = 127
WINDOWS_UNFILTERED_KEY_PRESS_TIMESTAMP: Final = 128
WINDOWS_FILTER_CLEARED_PRESS_TIMESTAMP: Final = 129
WINDOWS_HELD_KEY_PRESS_TIMESTAMP: Final = 10
WINDOWS_HELD_KEY_RELEASE_TIMESTAMP: Final = 11
WINDOWS_INJECTED_KEY_TIMESTAMP: Final = 12
WINDOWS_POST_HOLD_PRESS_TIMESTAMP: Final = 13
# time.monotonic() readings that make the poll loop see the deadline pass: the start, one reading
# still inside it, one past it.
LAYOUT_SWITCH_POLL_MONOTONIC_READINGS: Final = (0.0, 0.1, 0.6)
X11_FIXTURE_EVENT_TIMESTAMP: Final = 77
X11_EXPECTED_DEADLINE_MARGIN_SECONDS: Final = 5
# A fixture "now" just past the pause-correction delay after the last word.
SETTLED_NOW_SECONDS: Final = LAST_WORD_INPUT_AT_SECONDS + DEFAULT_PAUSE_DELAY_SECONDS + PAST_PAUSE_THRESHOLD_MARGIN_SECONDS
# A fixture "now" well short of the pause-correction delay after the last word.
TOO_SOON_NOW_SECONDS: Final = LAST_WORD_INPUT_AT_SECONDS + BEFORE_PAUSE_THRESHOLD_MARGIN_SECONDS
