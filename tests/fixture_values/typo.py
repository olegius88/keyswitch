"""Data of the typo correction tests: small counted word lists and a key timestamp."""

from __future__ import annotations

from typing import Final

# A fake timestamp of the key presses the typo tests build.
TYPO_FIXTURE_KEY_TIMESTAMP: Final = 4242
# Russian forms with counts, as the source list has them: two frequent forms one key apart (`клавиатура`,
# `клавиатуры`), a form below the decision's least count (`редкий`), forms too rare to be stored (`клавиатурка`,
# `зеленого`, which occurs as `зелёного` too), a short word, and a form written in Latin letters the build skips.
TYPO_FIXTURE_LINES_RU: Final = (
    "клавиатура 5000",
    "клавиатуры 3000",
    "редкий 10",
    "клавиатурка 2",
    "зеленого 3",
    "и 100000",
    "keyboard 50",
    "клавиатура 7",
)
# English forms: `hello`, the reading of `руддщ` typed in the Russian layout, and `keyboard`.
TYPO_FIXTURE_LINES_EN: Final = (
    "hello 10000",
    "keyboard 2000",
)
