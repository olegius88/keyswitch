"""Delays, timeouts, polling intervals and grace periods of the running application."""

from __future__ import annotations

from typing import Final

# AT-SPI IPC calls are given this many milliseconds before libatspi gives up.
ATSPI_CALL_TIMEOUT_MS: Final = 50
# The whole read() call, across every IPC round trip, is bounded by this wall-clock budget so a
# stuck accessibility bus cannot stall the engine.
ATSPI_FIELD_READ_DEADLINE_SECONDS: Final = 0.15
# A layout switch requested from the operating system is asynchronous: a native backend polls for
# the new layout at this step, and gives up after the timeout.
LAYOUT_SWITCH_TIMEOUT_SECONDS: Final = 0.5
# Step at which a native backend polls for the layout it asked the operating system to switch to.
LAYOUT_SWITCH_POLL_SECONDS: Final = 0.01
# Back-off before the field reader is tried again after successive failures.
FIELD_READER_RETRY_DELAYS_SECONDS: Final = (5.0, 15.0, 60.0)
# The desktop window shows the learning prompt this long after the engine asks for it.
LEARNING_PROMPT_SHOW_DELAY_MS: Final = 200
# The desktop window drains the engine's event queue this often.
EVENT_DRAIN_INTERVAL_MS: Final = 50
EVENT_DRAIN_INITIAL_DELAY_MS: Final = 25
# A settings window reads the active application this long after hiding, so the user can switch to
# it.
APPLICATION_CAPTURE_DELAY_MS: Final = 3000
# The learning prompt waits this long for an answer.
LEARNING_PROMPT_TIMEOUT_SECONDS: Final = 8.0
# A layout change observed this soon after the engine switched the layout itself (correction, menu
# action) is the engine's own switch, not the user's.
ENGINE_SWITCH_GRACE_SECONDS: Final = 1.5
# A key without a release for this long is treated as a lost key-up so a stuck entry can never block
# pause correction forever.
STALE_PRESS_SECONDS: Final = 3.0
# A letter arriving in the old layout this soon after an early switch was pressed before the switch
# took effect and is converted on its own.
LATE_STROKE_GRACE_SECONDS: Final = 0.5
# A correction the backend has not confirmed within this long is abandoned.
ACTION_TIMEOUT_SECONDS: Final = 2.0
# After a manual command the engine waits this long for its keys to be released.
MANUAL_RELEASE_TIMEOUT_SECONDS: Final = 3.0
# How long stopping the engine waits for its worker thread.
ENGINE_WORKER_JOIN_TIMEOUT_SECONDS: Final = 2.0
# The main loop wakes on its own at least this often even with nothing pending, and never sleeps for
# less than this even when a deadline is closer.
ENGINE_LOOP_MAX_WAKE_SECONDS: Final = 0.5
ENGINE_LOOP_MIN_WAKE_SECONDS: Final = 0.01
# Undo stays available for this long after a correction.
UNDO_AVAILABLE_WINDOW_SECONDS: Final = 10.0
# The macOS app re-checks the accessibility permission this often while it waits for it.
MACOS_PERMISSION_POLL_SECONDS: Final = 1.0
# How long start() waits for the platform keyboard listener (Win32 hook, Quartz event tap, XRecord
# capture) to confirm it runs.
KEYBOARD_LISTENER_START_TIMEOUT_SECONDS: Final = 5.0
# How long stop() waits for the platform keyboard listener thread to finish.
KEYBOARD_LISTENER_STOP_TIMEOUT_SECONDS: Final = 2.0
# The event tap's run loop runs in slices this long, so a stop request is noticed.
EVENT_TAP_RUN_LOOP_SLICE_SECONDS: Final = 0.2
# The GTK window saves an edited text setting this long after the last keystroke.
TEXT_SAVE_DEBOUNCE_MS: Final = 450
# The Windows smoke test closes its window this long after it opens.
SMOKE_UI_QUIT_AFTER_MS: Final = 300
# Milliseconds UI Automation may take to reach a provider and to finish one request.
UIA_CONNECTION_TIMEOUT_MS: Final = 200
UIA_TRANSACTION_TIMEOUT_MS: Final = 200
# An X11 injection batch waits at least this long, plus INJECTION_SECONDS_PER_EVENT per event, for
# the server to echo it.
MIN_INJECTION_DEADLINE_SECONDS: Final = 1.0
INJECTION_SECONDS_PER_EVENT: Final = 0.02
