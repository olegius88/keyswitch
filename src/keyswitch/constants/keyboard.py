"""Platform-neutral facts about key events."""

from __future__ import annotations

from typing import Final

# Internal modifier bits intentionally match the X11 core masks. Platform backends normalize their
# native state into these values before emitting an event, keeping the engine independent from an
# operating system API.
SHIFT_MASK: Final = 1 << 0
LOCK_MASK: Final = 1 << 1
CONTROL_MASK: Final = 1 << 2
ALT_MASK: Final = 1 << 3
SUPER_MASK: Final = 1 << 6
# The keyboard event pair (down, up) `complete_action` sends for one key.
COMPLETED_ACTION_EVENT_COUNT: Final = 2
# Language layout groups the application works with: 0 (EN) and 1 (RU). A third language would need
# a third physical layout group, which nothing supports yet.
LAYOUT_GROUP_COUNT: Final = 2
# Short label of each language layout group.
LAYOUT_LABELS: Final = {0: "EN", 1: "RU"}
# Readable name of each language layout group.
LAYOUT_NAMES: Final = {0: "английский (EN)", 1: "русский (RU)"}
# Layout group a synthetic pointer event carries: it belongs to no layout.
POINTER_EVENT_GROUP: Final = -1
