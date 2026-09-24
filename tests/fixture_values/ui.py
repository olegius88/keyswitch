"""Fake geometry used by tests."""

from __future__ import annotations

from typing import Final

# Geometry of the scratch GTK window the X11 E2E drivers type into.
E2E_TEST_WINDOW_WIDTH_PIXELS: Final = 520
E2E_TEST_WINDOW_HEIGHT_PIXELS: Final = 120
E2E_TEST_ENTRY_MARGIN_PIXELS: Final = 24
# English flag: near-black top-left corner, red bottom-right stripe. Russian flag: white top-left
# stripe, red bottom-right stripe.
WINDOWS_TRAY_FLAG_PIXEL_EXPECTATIONS: Final = {
    0: ((60, 59, 110, 255), (178, 34, 52, 255)),
    1: ((255, 255, 255, 255), (213, 43, 30, 255)),
}
# A Tk scrollregion is (x1, y1, x2, y2); the bottom edge is its last field.
TK_SCROLLREGION_FIELD_COUNT: Final = 4
TK_SCROLLREGION_BOTTOM_INDEX: Final = 3
WINDOWS_E2E_WHEEL_EVENT_OFFSET_PIXELS: Final = 20
LEARNING_PROMPT_FIXTURE_WINDOW_HEIGHT_PIXELS: Final = 80
# The caret anchor the substitute API reports, asserted back against verbatim.
MACOS_FAKE_CARET_ANCHOR_X: Final = 10
MACOS_FAKE_CARET_ANCHOR_Y: Final = 20
INVALID_MENU_ITEM_ID: Final = 999
GTK_HISTORY_NAVIGATION_INDEX: Final = 7
X11_POINTER_ROOT_X: Final = 640
X11_POINTER_ROOT_Y: Final = 480
X11_POINTER_WINDOW_X: Final = 12
X11_POINTER_WINDOW_Y: Final = 34
X11_POSITION_TARGET_X: Final = 10
X11_POSITION_TARGET_Y: Final = 20
# A coordinate value that does not matter for the assertion using it.
PLACEHOLDER_COORDINATE: Final = 2
# Accessible-text character-extents fixtures (Atspi get_character_extents) the caret-anchor tests
# use; expected screen anchors are derived from these, the same way focused_caret_anchor() derives
# them from the real ones.
LEARNING_PROMPT_FOCUSED_RECT_X: Final = 100
LEARNING_PROMPT_FOCUSED_RECT_Y: Final = 200
LEARNING_PROMPT_FOCUSED_RECT_WIDTH: Final = 9
LEARNING_PROMPT_FOCUSED_RECT_HEIGHT: Final = 18
LEARNING_PROMPT_CHILD_RECT_X: Final = 10
LEARNING_PROMPT_CHILD_RECT_Y: Final = 20
LEARNING_PROMPT_CHILD_RECT_WIDTH: Final = 5
LEARNING_PROMPT_CHILD_RECT_HEIGHT: Final = 10
# Fake ScreenAnchor fixtures for the learning-prompt window-positioning tests.
LEARNING_PROMPT_FIRST_SCREEN_ANCHOR_X: Final = 500
LEARNING_PROMPT_FIRST_SCREEN_ANCHOR_Y: Final = 400
LEARNING_PROMPT_FIRST_SCREEN_ANCHOR_WINDOW: Final = 77
LEARNING_PROMPT_CARET_OVERRIDE_ANCHOR_X: Final = 700
LEARNING_PROMPT_CARET_OVERRIDE_ANCHOR_Y: Final = 300
LEARNING_PROMPT_CARET_ONLY_ANCHOR_X: Final = 10
LEARNING_PROMPT_CARET_ONLY_ANCHOR_Y: Final = 20
LEARNING_PROMPT_FALLBACK_ANCHOR_X: Final = 30
LEARNING_PROMPT_FALLBACK_ANCHOR_Y: Final = 40
LEARNING_PROMPT_FALLBACK_ANCHOR_WINDOW: Final = 88
# The fake input anchor Windows backend tests attach to a window: an arbitrary screen position.
WINDOWS_FAKE_ANCHOR_X: Final = 100
WINDOWS_FAKE_ANCHOR_Y: Final = 200
# Row of "off" in the GTK context policy selector.
EXPECTED_CONTEXT_MODE_OFF_INDEX: Final = 2
