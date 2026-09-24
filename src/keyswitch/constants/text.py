"""Character and word limits, context window sizes and truncation lengths."""

from __future__ import annotations

from typing import Final

# Bounded traversal width: at most this many children are enumerated at any one level of the
# accessibility tree, whether under the desktop root or under a node being searched for the focused
# field.
ATSPI_MAX_TRAVERSED_CHILDREN: Final = 64
# Accessibility nodes one AT-SPI field read visits at most.
ATSPI_MAX_VISITED_NODES: Final = 128
# Characters a platform field reader reads after the caret; the before side is bounded by
# input_context.CONTEXT_LIMIT, a different limit.
FIELD_AFTER_CARET_MAX_CHARACTERS: Final = 128
# The engine backs up simple single-key text edits; anything above the Basic Multilingual Plane is
# composed text a backspace cannot safely undo alone.
BASIC_MULTILINGUAL_PLANE_MAX_CODEPOINT: Final = 0xFFFF
# The Unicode Cyrillic block, first and last code point.
CYRILLIC_CODEPOINT_RANGE: Final = (0x0400, 0x04FF)
# A string setting value longer than this is shortened (with "...") before it is logged.
LOGGED_SETTING_VALUE_MAX_CHARACTERS: Final = 80
MAX_UNICODE_CODEPOINT: Final = 0x10FFFF
