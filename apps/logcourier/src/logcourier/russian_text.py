"""Counts in Russian texts: the number comes from its constant, the noun agrees with it.

A number written into a sentence is the constant it describes, formatted here, so the
sentence cannot keep an old value after the constant changes. Russian makes the noun agree
with the number: `quantity(4, SECONDS)` is "4 секунды", `quantity(5, SECONDS)` is
"5 секунд", `quantity(2.5, SECONDS)` is "2,5 секунды".

The three forms of a noun are the ones after 1, after 2-4 and after 5-20, in the accusative
the sentences use ("раз в 5 секунд"); a fraction takes the genitive singular, which is the
second form.
"""

from __future__ import annotations

from typing import Final

from .constants.russian_text import (
    RUSSIAN_PLURAL_ELEVEN_EXCEPTION,
    RUSSIAN_PLURAL_FEW_LAST_DIGITS,
    RUSSIAN_PLURAL_MOD_HUNDRED,
    RUSSIAN_PLURAL_MOD_TEN,
    RUSSIAN_PLURAL_TEEN_EXCEPTIONS,
)

NounForms = tuple[str, str, str]

# Accusative: "раз в 1 секунду", "раз в 2 секунды", "раз в 5 секунд".
SECONDS: Final[NounForms] = ("секунду", "секунды", "секунд")


def russian_number(value: float) -> str:
    """A whole value without a fraction part, a fraction with a decimal comma: 3, 2,5."""
    if float(value).is_integer():
        return str(int(value))
    return repr(float(value)).replace(".", ",")


def quantity(value: float, forms: NounForms) -> str:
    """The number and the noun form it takes: 1 секунду, 2 секунды, 5 секунд, 2,5 секунды."""
    one, few, many = forms
    written = russian_number(value)
    if not float(value).is_integer():
        return f"{written} {few}"
    count = abs(int(value))
    last, last_two = count % RUSSIAN_PLURAL_MOD_TEN, count % RUSSIAN_PLURAL_MOD_HUNDRED
    if last == 1 and last_two != RUSSIAN_PLURAL_ELEVEN_EXCEPTION:
        return f"{written} {one}"
    if last in RUSSIAN_PLURAL_FEW_LAST_DIGITS and last_two not in RUSSIAN_PLURAL_TEEN_EXCEPTIONS:
        return f"{written} {few}"
    return f"{written} {many}"
