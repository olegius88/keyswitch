"""Numbers of Russian grammar used when a count is written into a text."""

from __future__ import annotations

from typing import Final

# Russian pluralisation of a count (1 секунду / 2 секунды / 5 секунд): the standard "count % 10,
# with a % 100 teens exception" rule.
RUSSIAN_PLURAL_MOD_TEN: Final = 10
RUSSIAN_PLURAL_MOD_HUNDRED: Final = 100
RUSSIAN_PLURAL_ELEVEN_EXCEPTION: Final = 11
RUSSIAN_PLURAL_FEW_LAST_DIGITS: Final = (2, 3, 4)
RUSSIAN_PLURAL_TEEN_EXCEPTIONS: Final = (12, 13, 14)
