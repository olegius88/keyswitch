"""The sentences of a Wikisource page, from its wikitext: what the independent sets and the prose forms read.

Templates, notes, tables, headings, back matter and markup are dropped; a line splits into sentences after
a final stop followed by a dash, a quotation mark, a bracket or a capital letter. Nothing here depends on the
orthotactic model, so both `ortho_independent` and `ortho_prose_lexicon` read pages the same way.
"""

from __future__ import annotations

import html
import re
from typing import Final

# Wikitext: what is not the text of the work, and where a sentence ends.
_COMMENT: Final = re.compile(r"<!--.*?-->", re.DOTALL)
_REFERENCE: Final = re.compile(r"<ref[^>/]*/>|<ref[^>]*>.*?</ref>", re.DOTALL | re.IGNORECASE)
_TEMPLATE: Final = re.compile(r"\{\{[^{}]*\}\}")
_TABLE: Final = re.compile(r"\{\|.*?\|\}", re.DOTALL)
_BACK_MATTER: Final = re.compile(r"^==+\s*(Примечания|Комментарии|См\. также|Ссылки)\s*==+|\[\[Категория:", re.MULTILINE)
_HEADING: Final = re.compile(r"^=+.*=+$")
_MAGIC_WORD: Final = re.compile(r"__[A-Z]+__")
_LINK: Final = re.compile(r"\[\[(?:[^|\]]*\|)?([^\]]*)\]\]")
_EXTERNAL_LINK: Final = re.compile(r"\[(?:https?:)?//\S+\s*([^\]]*)\]")
_TAG: Final = re.compile(r"<[^>]+>")
_SENTENCE_END: Final = re.compile(r"(?<=[.!?…])\s+(?=[—–«\"(\[A-ZА-ЯЁ])")
_LEADING: Final[str] = "—–-«»\"'„“” "


def wikitext_sentences(content: str) -> list[str]:
    """The sentences of a Wikisource page: its text without templates, notes, headings and back matter."""

    text = _REFERENCE.sub("", _COMMENT.sub("", content))
    previous = None
    while previous != text:
        previous, text = text, _TEMPLATE.sub("", text)
    text = _TABLE.sub("", text)
    end = _BACK_MATTER.search(text)
    text = text[:end.start()] if end is not None else text
    text = _EXTERNAL_LINK.sub(r"\1", _LINK.sub(r"\1", _MAGIC_WORD.sub("", text)))
    text = html.unescape(_TAG.sub("", text.replace("'''", "").replace("''", "")))
    sentences: list[str] = []
    for line in text.splitlines():
        line = line.strip().lstrip("*#:;").strip()
        if not line or _HEADING.match(line):
            continue
        sentences.extend(sentence for part in _SENTENCE_END.split(line) if (sentence := part.strip(_LEADING)))
    return sentences
