"""Per-application input conventions that a language model cannot learn.

A quirk is a rule about one application's own syntax, not about Russian or
English. In Telegram a message that begins with the Russian quote is almost
always a mention: the same physical key carries ``@`` in the English layout, and
``@name`` is how Telegram addresses somebody. The rule fires on the keystroke, so
the nickname can follow immediately; pressing the same key again writes the two
quotes after all, which is what a user who meant a quotation does.

Every quirk is one setting the user can turn off, and every condition is
observable in the engine's own state: which application has focus, which symbol
is pending, and whether anything followed it.
"""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass


@dataclass(frozen=True)
class SymbolQuirk:
    """One pending symbol that means another character in this application."""

    setting: str
    title: str
    description: str
    applications: tuple[str, ...]
    typed: str
    meant: str

    def applies(self, application: str, typed: str, meant: str) -> bool:
        if typed != self.typed or meant != self.meant:
            return False
        name = application.casefold()
        return any(candidate in name for candidate in self.applications)


TELEGRAM_QUOTE_MENTION = SymbolQuirk(
    setting="applications.telegram_quote_mention",
    title="Telegram: одинокая кавычка — это @",
    description=(
        "Кавычка, набранная в русской раскладке в начале слова, сразу заменяется на «@» "
        "для упоминания, и ник печатается следом без паузы. Нужна настоящая кавычка — "
        "нажмите ту же клавишу ещё раз: получится «\"\"» и раскладка вернётся."
    ),
    applications=("telegram", "kotatogram", "ayugram", "unigram", "forkgram"),
    typed='"',
    meant="@",
)

SYMBOL_QUIRKS: tuple[SymbolQuirk, ...] = (TELEGRAM_QUOTE_MENTION,)


def symbol_quirk(
    application: str,
    typed: str,
    meant: str,
    enabled: Callable[[str], bool],
) -> SymbolQuirk | None:
    """The quirk that explains this pending symbol, or ``None``."""

    for quirk in SYMBOL_QUIRKS:
        if quirk.applies(application, typed, meant) and enabled(quirk.setting):
            return quirk
    return None
