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
# FIELD_CONTEXT_MAX_CHARACTERS, a different limit.
FIELD_AFTER_CARET_MAX_CHARACTERS: Final = 128
# The most characters a field reader may report its caret short of the end of what was typed and
# still be taken as the same text: VS Code Insiders (06.10.2026) reported the caret of its chat box
# one character early, the last typed letter after it, and every word there was refused.
FIELD_CARET_LAG_MAX_CHARACTERS: Final = 2
# The most of the last typed letters a field read as their key goes down may not show yet: a browser
# or an Electron editor reports the text a moment after the key (engine._typed_in_field).
FIELD_PENDING_MAX_CHARACTERS: Final = 2
# The most an editor's input buffer shows when it is what UI Automation reads instead of the text: VS Code with its
# screen-reader support off keeps the input it takes keys through empty, and a read shows at most the key in flight
# (FieldContext.input_buffer_only).
FIELD_INPUT_BUFFER_MAX_CHARACTERS: Final = 1
# Field text kept on each side of the caret, typed or read (input_context exports it as
# CONTEXT_LIMIT), and the longest application name, field identifier and source label a field
# context carries.
FIELD_CONTEXT_MAX_CHARACTERS: Final = 512
FIELD_CONTEXT_APPLICATION_MAX_CHARACTERS: Final = 128
FIELD_CONTEXT_FIELD_ID_MAX_CHARACTERS: Final = 128
FIELD_CONTEXT_SOURCE_MAX_CHARACTERS: Final = 32
# The engine backs up simple single-key text edits; anything above the Basic Multilingual Plane is
# composed text a backspace cannot safely undo alone.
BASIC_MULTILINGUAL_PLANE_MAX_CODEPOINT: Final = 0xFFFF
# The Unicode Cyrillic block, first and last code point.
CYRILLIC_CODEPOINT_RANGE: Final = (0x0400, 0x04FF)
# A string setting value longer than this is shortened (with "...") before it is logged.
LOGGED_SETTING_VALUE_MAX_CHARACTERS: Final = 80
MAX_UNICODE_CODEPOINT: Final = 0x10FFFF
# Code points below this are C0 control characters.
ASCII_CONTROL_CHARACTER_LIMIT: Final = 0x20
# DEL, the first code point after the printable ASCII characters.
ASCII_DELETE_CODEPOINT: Final = 0x7F
# Longest word handed to Hunspell; a longer token is not a dictionary word and is refused.
HUNSPELL_MAX_CHECK_CHARACTERS: Final = 128
