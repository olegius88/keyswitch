"""Unit conversions."""

from __future__ import annotations

from typing import Final

MILLISECONDS_PER_SECOND: Final = 1000
BYTES_PER_KIBIBYTE: Final[int] = 1024
BYTES_PER_MEBIBYTE: Final = BYTES_PER_KIBIBYTE * BYTES_PER_KIBIBYTE
PER_MILLE_SCALE: Final = 1000
SECONDS_PER_MINUTE: Final[int] = 60
SECONDS_PER_HOUR: Final[int] = 3600
