"""Per-application input conventions that a language model cannot learn.

One physical key carries ``@`` in the English layout and the quote in the
Russian one, and in a chat client a word that starts with it is usually a
mention: ``@name``. That is a fact about the application, not about either
language, so it lives here.

What the head means is not decided here. The engine analyses the word that
follows on its own - ``ощрт`` is nothing in Russian while ``john`` is a name,
``привет`` is an ordinary Russian word - and the head simply follows that
decision: it becomes ``@`` when the word turns out to be the other layout, and
stays a quote when the word is left alone. A quotation therefore survives, and
so does a quote typed after the user selected the layout by hand or moved the
caret, because those already stop the engine from touching the word.

Every convention is one setting the user can turn off, and every condition is
observable in the engine's own state: which application has focus, which symbol
is pending and which word grew after it.
"""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass


@dataclass(frozen=True)
class MentionHead:
    """A pending symbol that may be the start of a mention in this application."""

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


TELEGRAM_QUOTE_MENTION = MentionHead(
    setting="applications.telegram_quote_mention",
    title="Telegram: кавычка перед словом может быть @",
    description=(
        "Кавычка, набранная в русской раскладке перед словом, становится «@», только если "
        "само слово оказалось набранным не в той раскладке: «ощрт» → «@john». Русское слово "
        "после кавычки остаётся цитатой, и кавычка остаётся кавычкой."
    ),
    applications=("telegram", "kotatogram", "ayugram", "unigram", "forkgram"),
    typed='"',
    meant="@",
)

MENTION_HEADS: tuple[MentionHead, ...] = (TELEGRAM_QUOTE_MENTION,)


def mention_head(
    application: str,
    typed: str,
    meant: str,
    enabled: Callable[[str], bool],
) -> MentionHead | None:
    """The convention that explains this pending symbol, or ``None``."""

    for head in MENTION_HEADS:
        if head.applies(application, typed, meant) and enabled(head.setting):
            return head
    return None
