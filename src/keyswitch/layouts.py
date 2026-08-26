"""Keyboard-layout transformations used by the detector and tests."""

from __future__ import annotations

from dataclasses import dataclass


US_KEYS = "`qwertyuiop[]\\asdfghjkl;'zxcvbnm,."
RU_KEYS = "ёйцукенгшщзхъ\\фывапролджэячсмитьбю"


def _case_aware_map(source: str, target: str) -> dict[str, str]:
    mapping = dict(zip(source, target, strict=True))
    for left, right in zip(source, target, strict=True):
        if left.isalpha() and right.isalpha():
            mapping[left.upper()] = right.upper()
    return mapping


@dataclass(frozen=True)
class LayoutPair:
    first: str = "us"
    second: str = "ru"

    def __post_init__(self) -> None:
        if {self.first, self.second} != {"us", "ru"}:
            raise ValueError("The static mapper currently supports the us/ru pair")

    @property
    def us_to_ru(self) -> dict[str, str]:
        return _case_aware_map(US_KEYS, RU_KEYS)

    @property
    def ru_to_us(self) -> dict[str, str]:
        return _case_aware_map(RU_KEYS, US_KEYS)

    def translate(self, text: str, source: str, target: str) -> str:
        if source == target:
            return text
        if (source, target) == ("us", "ru"):
            mapping = self.us_to_ru
        elif (source, target) == ("ru", "us"):
            mapping = self.ru_to_us
        else:
            raise ValueError(f"Unsupported layout conversion: {source} -> {target}")
        return "".join(mapping.get(character, character) for character in text)
