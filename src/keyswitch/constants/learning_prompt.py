"""Geometry of the learning prompt window."""

from __future__ import annotations

from typing import Final

LEARNING_PROMPT_CARD_PADDING_X_PIXELS: Final = 16
LEARNING_PROMPT_CARD_PADDING_Y_PIXELS: Final = 12
LEARNING_PROMPT_TITLE_FONT_SIZE_POINTS: Final = 10
LEARNING_PROMPT_WORD_FONT_SIZE_POINTS: Final = 11
LEARNING_PROMPT_WORD_PADDING_TOP_PIXELS: Final = 5
LEARNING_PROMPT_WORD_PADDING_BOTTOM_PIXELS: Final = 2
LEARNING_PROMPT_HINT_FONT_SIZE_POINTS: Final = 9
# Narrowest the desktop (Tk) learning prompt is laid out.
LEARNING_PROMPT_MINIMUM_WIDTH_PIXELS: Final = 390
# The learning prompt floats this many pixels above the caret.
LEARNING_PROMPT_ANCHOR_GAP_PIXELS: Final = 12
# The desktop learning prompt keeps this far inside the virtual screen.
LEARNING_PROMPT_SCREEN_MARGIN_PIXELS: Final = 8
# Bounded traversal of the accessibility tree while looking for the focused caret: a total node
# budget and a per-node child budget.
MAX_CARET_SEARCH_NODES: Final = 4096
MAX_CARET_SEARCH_CHILDREN: Final = 512
# Default width of the GTK learning prompt.
LEARNING_PROMPT_GTK_WIDTH_PIXELS: Final = 410
LEARNING_PROMPT_GTK_SPACING_PIXELS: Final = 5
LEARNING_PROMPT_GTK_MARGIN_Y_PIXELS: Final = 14
LEARNING_PROMPT_GTK_MARGIN_X_PIXELS: Final = 18
