"""Values of the packed gram tables (packed_grams)."""

from __future__ import annotations

from typing import Final

# A gram's characters become one byte each, 1 to 255; a zero byte marks a character the table never holds.
PACKED_GRAM_LAST_CODE: Final = 255
PACKED_GRAM_BITS_PER_CODE: Final = 8
# The bytes of a gram are read as one signed 64-bit key, so a gram may have at most this many characters: the key's
# first byte stays zero and the key positive.
PACKED_GRAM_KEY_BYTES: Final = 8
PACKED_GRAM_MAX_CHARACTERS: Final = PACKED_GRAM_KEY_BYTES - 1
# Below this many entries a table is left as it is: packing saves nothing worth the slower lookups.
PACKED_GRAM_MIN_ENTRIES: Final = 4096
