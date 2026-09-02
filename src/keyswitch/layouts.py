"""Keyboard-layout transformations used by the detector and tests."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Final


US_KEYS = "`qwertyuiop[]\\asdfghjkl;'zxcvbnm,."
RU_KEYS = "ёйцукенгшщзхъ\\фывапролджэячсмитьбю"


def _case_aware_map(source: str, target: str) -> dict[str, str]:
    mapping = dict(zip(source, target, strict=True))
    for left, right in zip(source, target, strict=True):
        if left.isalpha() and right.isalpha():
            mapping[left.upper()] = right.upper()
    return mapping


# The mappings are pure functions of the two key rows, so they are built once.
# ``str.translate`` with these tables replaces exactly the characters the
# mapping contains and leaves every other code point untouched, which is the
# same result as substituting character by character.
_US_TO_RU: Final[dict[str, str]] = _case_aware_map(US_KEYS, RU_KEYS)
_RU_TO_US: Final[dict[str, str]] = _case_aware_map(RU_KEYS, US_KEYS)
_US_TO_RU_TABLE: Final[dict[int, str]] = str.maketrans(_US_TO_RU)
_RU_TO_US_TABLE: Final[dict[int, str]] = str.maketrans(_RU_TO_US)


@dataclass(frozen=True)
class LayoutPair:
    first: str = "us"
    second: str = "ru"

    def __post_init__(self) -> None:
        if {self.first, self.second} != {"us", "ru"}:
            raise ValueError("The static mapper currently supports the us/ru pair")

    @property
    def us_to_ru(self) -> dict[str, str]:
        return dict(_US_TO_RU)

    @property
    def ru_to_us(self) -> dict[str, str]:
        return dict(_RU_TO_US)

    def translate(self, text: str, source: str, target: str) -> str:
        if source == target:
            return text
        if (source, target) == ("us", "ru"):
            table = _US_TO_RU_TABLE
        elif (source, target) == ("ru", "us"):
            table = _RU_TO_US_TABLE
        else:
            raise ValueError(f"Unsupported layout conversion: {source} -> {target}")
        return text.translate(table)
