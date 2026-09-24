"""Repository layout and the rules its source follows."""

from __future__ import annotations

from typing import Final

# src/keyswitch/system.py -> src/keyswitch -> src -> repository root
SOURCE_ROOT_PARENT_LEVELS: Final = 2
# The additive and multiplicative identities: an empty count, a first index, a single step.
# Everything else says something and has a name.
NEUTRAL_NUMBERS: Final = frozenset({0, 1})
# In a text, a share of none or of all ("0%" before a download starts, "100% branch coverage").
NEUTRAL_PERCENTS: Final = frozenset({0, 100})
