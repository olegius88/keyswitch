"""Exact US/RU key pairs shared by context training and physical replay."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Final


@dataclass(frozen=True)
class PhysicalKey:
    characters: tuple[str, str]
    shift: bool
    keycode: int
    explicit_group: int | None = None


def physical_keys() -> tuple[PhysicalKey, ...]:
    plain_us = "`1234567890-=qwertyuiop[]\\asdfghjkl;'zxcvbnm,./ "
    plain_ru = "ё1234567890-=йцукенгшщзхъ\\фывапролджэячсмитьбю. "
    shift_us = '~!@#$%^&*()_+QWERTYUIOP{}|ASDFGHJKL:"ZXCVBNM<>? '
    shift_ru = 'Ё!"№;%:?*()_+ЙЦУКЕНГШЩЗХЪ/ФЫВАПРОЛДЖЭЯЧСМИТЬБЮ, '
    return tuple(PhysicalKey((left, right), shift, index + 20)
                 for shift, us, ru in ((False, plain_us, plain_ru), (True, shift_us, shift_ru))
                 for index, (left, right) in enumerate(zip(us, ru, strict=True)))


KEYS: Final[tuple[PhysicalKey, ...]] = physical_keys()
_BY_CHARACTER: Final[tuple[dict[str, PhysicalKey], dict[str, PhysicalKey]]] = ({}, {})
for _key in KEYS:
    for _group in (0, 1):
        _BY_CHARACTER[_group].setdefault(_key.characters[_group], _key)


def translated(text: str, group: int) -> str:
    """Read identical key presses in the other layout; reject unknown glyphs."""

    if type(group) is not int or group not in (0, 1):
        raise ValueError("physical translation requires group 0 or 1")
    result: list[str] = []
    for character in text:
        key = _BY_CHARACTER[group].get(character)
        if key is None:
            raise ValueError(f"unsupported physical glyph U+{ord(character):04X} in group {group}")
        result.append(key.characters[1 - group])
    return "".join(result)
