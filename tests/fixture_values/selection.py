"""Values the selection hotkey tests use: fake windows, keys and clipboard contents."""

from __future__ import annotations

from typing import Final

# The window a selection is read in, and another one the focus moves to.
SELECTION_FIXTURE_WINDOW: Final = 7
SELECTION_FIXTURE_OTHER_WINDOW: Final = 8
# A fake X11 keyboard: the slash key (`/` `?` in the English group, `.` `,` in the Russian one).
SELECTION_FIXTURE_SLASH_KEYCODE: Final = 61
# A fake clipboard format the selection hotkeys keep and put back, and its bytes.
SELECTION_FIXTURE_CLIPBOARD_FORMAT: Final = 0xC0DE
SELECTION_FIXTURE_CLIPBOARD_BYTES: Final = b"user data"
# The clipboard sequence number before a copy.
SELECTION_FIXTURE_SEQUENCE_BEFORE: Final = 40
# A UTF-16 range in a macOS field: where the selection starts and how long the replacement is.
SELECTION_FIXTURE_RANGE_START: Final = 5
SELECTION_FIXTURE_REPLACEMENT_UNITS: Final = 6
# Key events of one tap and of a tap with Shift around it, for counting what a backend typed.
SELECTION_FIXTURE_EVENTS_PER_TAP: Final = 2
SELECTION_FIXTURE_EVENTS_PER_SHIFTED_TAP: Final = 4
# The Scroll Lock key of an X11 keyboard, for pressing Alt+Scroll Lock.
SELECTION_FIXTURE_SCROLL_LOCK_KEYCODE: Final = 78
# More than one selection in a field (AT-SPI counts them).
SELECTION_FIXTURE_SEVERAL_SELECTIONS: Final = 2
# Step of the fake clock of the Windows paste tests, and clipboard reads that answer busy for longer than any wait.
SELECTION_FIXTURE_CLOCK_STEP_SECONDS: Final = 0.25
SELECTION_FIXTURE_ALWAYS_BUSY_READS: Final = 256
