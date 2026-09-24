"""Geometry, fonts and value ranges of the GTK settings window."""

from __future__ import annotations

from typing import Final

# Window sizing.
GTK_WINDOW_DEFAULT_WIDTH_PIXELS: Final = 1040
GTK_WINDOW_DEFAULT_HEIGHT_PIXELS: Final = 720
GTK_WINDOW_MIN_WIDTH_PIXELS: Final = 850
GTK_WINDOW_MIN_HEIGHT_PIXELS: Final = 600
# The design's recurring gap sizes, reused wherever a Gtk.Box's inter-child spacing matches one of
# these two tokens.
GTK_STANDARD_SPACING_PIXELS: Final = 12
GTK_COMPACT_SPACING_PIXELS: Final = 10
GTK_SIDEBAR_WIDTH_PIXELS: Final = 250
BRAND_MARGIN_OUTER_PIXELS: Final = 20
BRAND_MARGIN_BOTTOM_PIXELS: Final = 12
BRAND_MARGIN_END_PIXELS: Final = 16
BRAND_ICON_SIZE_PIXELS: Final = 44
SIDEBAR_PRIVACY_MARGIN_START_PIXELS: Final = 22
SIDEBAR_PRIVACY_MARGIN_BOTTOM_PIXELS: Final = 18
# Every page built through _new_page shares this heading/description gap.
PAGE_CONTENT_SPACING_PIXELS: Final = 18
HERO_TOP_ROW_SPACING_PIXELS: Final = 14
HERO_COPY_SPACING_PIXELS: Final = 5
TEST_ENTRY_WIDTH_CHARACTERS: Final = 30
STAT_CARD_SPACING_PIXELS: Final = 3
# Index of "off" among the GTK window's context policy choices, used when the stored
# detection.context_policy is not a known mode.
CONTEXT_MODE_OFF_INDEX: Final = 2
# Characters of the intent model checksum shown on the languages page.
CHECKSUM_DISPLAY_LENGTH_CHARACTERS: Final = 12
HOTKEY_ENTRY_WIDTH_CHARACTERS: Final = 20
MANUAL_APP_ENTRY_WIDTH_CHARACTERS: Final = 22
WORDS_EDITOR_HEIGHT_PIXELS: Final = 100
TEXT_EDITOR_PADDING_PIXELS: Final = 10
APPLICATION_PICKER_DIALOG_SIDE_PIXELS: Final = 620
DIALOG_CONTENT_MARGIN_PIXELS: Final = 12
UPDATE_BUTTONS_SPACING_PIXELS: Final = 8
# Newest history entries the GTK history page reads.
GTK_HISTORY_PAGE_ENTRIES: Final = 200
DASHBOARD_RECENT_HISTORY_COUNT: Final = 4
# Adw.SpinRow reports floats; this is the tolerance for "unchanged" when an external settings update
# is echoed back to the control.
SPIN_VALUE_EPSILON: Final = 1e-9
