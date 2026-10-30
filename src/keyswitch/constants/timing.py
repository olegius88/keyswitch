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
# A Windows application answers the layout request from its own message loop, which a busy editor
# runs late: VS Code Insiders confirmed one about a second after it was posted (field log of
# 09.10.2026), when the correction had already been given up after half a second and the switch
# that arrived was taken for the user's own.
WINDOWS_LAYOUT_SWITCH_TIMEOUT_SECONDS: Final = 1.5
# Step at which a native backend polls for the layout it asked the operating system to switch to.
LAYOUT_SWITCH_POLL_SECONDS: Final = 0.01
# A modifier the Windows hook saw go down is checked against the keyboard when the next key comes
# this many milliseconds later or in another window: its release may have gone where the hook does
# not reach (a UAC prompt, the Ctrl+Alt+Del screen, an elevated window).
MODIFIER_RECHECK_GAP_MS: Final = 1000
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
# A second press of the manual conversion hotkey this soon after the first one, with its release in
# between and no other key, asks for a switching rule instead of converting the word back. The
# Windows default double-click time is the same half second.
DOUBLE_CONVERT_PRESS_WINDOW_SECONDS: Final = 0.5
# A press of the manual conversion hotkey this soon after the engine converted the word at a pause,
# with nothing typed since, asks for the conversion that has just been made: the user stopped typing
# to press it. In the field logs of 01-09.10.2026 every one of the eight such presses (7 ms to 1.2 s
# after the correction) undid a right correction, and the user then converted the word again.
PAUSE_CORRECTION_AGREEMENT_SECONDS: Final = 1.5
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
# How often a correction waiting for a field that has not shown its word yet looks again (detection.field_catch_up).
FIELD_CATCH_UP_RETRY_SECONDS: Final = 0.025
# Typed field context older than this is dropped, and the engine waits as long for the neighbour of a
# waiting word (input_context exports it as CONTEXT_TTL).
FIELD_CONTEXT_TTL_SECONDS: Final = 45.0
# A word waiting for its neighbour is decided at a pause no sooner than this after its space, however short
# the pause correction is set: the pause that settles it ends the neighbour's chance to decide it, and a
# thought between two words (`tot`, then `ghbdtn`) takes longer than a hesitation inside one.
CONTEXT_WAIT_SETTLE_MIN_SECONDS: Final = 1.5
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
# macOS answers Text Input Sources calls on the main thread only (keyswitch.macos_main_thread). How
# long the engine's worker waits for the main thread to read the layout before it keeps the one read
# last; the event tap's thread waits less, because a slow tap callback is what macOS switches a tap
# off for; a layout switch the engine asked for may take longer.
MACOS_MAIN_THREAD_WAIT_SECONDS: Final = 0.25
MACOS_TAP_MAIN_THREAD_WAIT_SECONDS: Final = 0.02
MACOS_LAYOUT_SWITCH_WAIT_SECONDS: Final = 1.0
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
# A key the keyboard hook sees this long after the system stamped it was held up on its way: every
# window waited for it (input_delay).
INPUT_DELAY_LATE_MS: Final = 100
# A key stamped this long before the hook saw it was not held up: Windows takes a low-level hook
# that does not answer within a second out of the chain. Some program sends keys with a time of
# its own (zero): on the owner's computer one such key every ten minutes or so read as late by the
# computer's uptime, 443 026 750 ms, and filled every report of 0.44.0 (field log, 08.10.2026).
INPUT_DELAY_UNSTAMPED_MS: Final = 60_000
# A hook callback that takes this long has held every key behind it; the callback normally takes
# well under a millisecond.
INPUT_DELAY_SLOW_CALLBACK_MS: Final = 20
# The engine logs the hook's delays at most this often, and only when a key was late.
INPUT_DELAY_REPORT_INTERVAL_SECONDS: Final = 60.0
# A key the Windows hook ran this long after its key time, or answered this slowly, kept the keys behind it waiting
# for the hook's thread, and Windows gives a key that waited about 300 ms to the window without calling any hook
# (297-313 ms between the keys it skipped, Windows 11 26200 and Windows 10 19045, 28.10.2026). In VM runs under CPU
# load every key that passed the hook lay within the wait of a key this late (20 of 20); GetTickCount steps 15.6 ms.
# The owner's worst wait in the logs is 218 ms.
HOOK_GAP_MS: Final = 280
# An input time GetLastInputInfo reports and no hook of KeySwitch got is taken as passed the hooks once this long went by
# with no later hook event either: a hook thread kept from the processor got its keys at most 4.8 s late in VM runs under
# CPU load, while a program in front that reads the keyboard as raw input and has a keyboard hook of its own keeps
# every keyboard hook silent for good.
HOOK_SILENCE_MS: Final = 10_000
# Input the hooks did not get is taken as kept away from them only when GetLastInputInfo moved on this many times with no
# hook event between: keys typed into a program that silences the hooks move it at every press and release. A single
# such input followed by events the hooks got is no program keeping keys away: in the laptop's log of 0.64.0
# (29-30.10.2026) inputs no hook got came in 14 programs, Explorer, Task Manager and KeySwitch's own window among them,
# in the middle of words the hook saw typed, and in none on the desktop computer.
UNSEEN_INPUT_MIN_TIMES: Final = 3
# How long the Wayland input helper waits for the main process to answer a physical key before
# typing it itself, so a hung process can never freeze the keyboard: it then reports every key it
# types until the process answers again. At 50 ms the answer was late while the engine's worker held
# the interpreter for a model decision (about 60 ms in a GNOME 50 VM, 27.10.2026), and keys typed
# that way were lost to the engine; a key late by this much is one stall, not a frozen keyboard.
WAYLAND_HELPER_VERDICT_TIMEOUT_MS: Final = 250
# Idle poll slice of the Wayland input helper loop so stop requests and dead devices are noticed.
WAYLAND_HELPER_POLL_TIMEOUT_MS: Final = 200
# Pause before an input helper that died is started again (WAYLAND_HELPER_RESTARTS).
WAYLAND_HELPER_RESTART_DELAY_SECONDS: Final = 1.0
# How long the Wayland backend waits for the input helper to acknowledge a synchronous command.
WAYLAND_HELPER_COMMAND_TIMEOUT_SECONDS: Final = 1.0
# Settle pause after UI_DEV_CREATE so udev and the Wayland compositor open the virtual keyboard.
WAYLAND_UINPUT_SETTLE_SECONDS: Final = 0.2
# Pause between bursts of a long injection (WAYLAND_UINPUT_BURST_TRANSITIONS), so the compositor
# reads one burst before the next lands in its event queue.
WAYLAND_UINPUT_BURST_PAUSE_SECONDS: Final = 0.003
# A Wayland compositor tells the focused client a new layout group at once, while keys typed just
# before may still be on their way through the input method (GNOME passes every key through IBus):
# switched under them, they come out in the new layout. Keys typed by a correction get this long to
# reach the client before the layout changes again. GNOME 50 in a VM, ',' then a switch: wrong 20
# of 20 at once, 3 of 20 after 20 ms, none after 40 ms (27.10.2026).
WAYLAND_TYPED_INPUT_SETTLE_SECONDS: Final = 0.15
# Maximum wait for physical keys to come up before taking EVIOCGRAB on a keyboard device.
WAYLAND_GRAB_WAIT_TIMEOUT_SECONDS: Final = 2.0
# Step at which EVIOCGKEY is polled while waiting for held keys to come up before EVIOCGRAB.
WAYLAND_GRAB_POLL_SECONDS: Final = 0.005
# Settle pause after the last held key comes up so libinput reads its release before EVIOCGRAB.
WAYLAND_GRAB_RELEASE_SETTLE_SECONDS: Final = 0.001
# Socket timeout for local Wayland compositor IPC (sway socket, D-Bus calls).
WAYLAND_IPC_TIMEOUT_SECONDS: Final = 1.0
# How long the Wayland keymap listener waits for wl_keyboard::keymap on initial connection.
WAYLAND_KEYMAP_WAIT_SECONDS: Final = 1.5
# Default client-side Wayland key-repeat delay before wl_keyboard::repeat_info arrives.
WAYLAND_REPEAT_DEFAULT_DELAY_MS: Final = 600
# D-Bus method call timeout for desktop compositor queries and layout changes.
WAYLAND_DBUS_CALL_TIMEOUT_MS: Final = 1000
