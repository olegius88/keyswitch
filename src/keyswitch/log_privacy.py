"""Words that look like a password never reach the technical log as typed.

The technical log holds every word typed, and it is what goes with a report (LogCourier delivers it to a chat). A word
typed right after «пароль» or «password» (in either layout) and every long token that mixes letters and digits go into
it as stars, as many as the word has characters, in every text a record carries: the word evaluated, its readings and
replacements, and the words around it. The owner's decision of 15.10.2026; the passwords that reached the log until then
were such tokens or the words after such a marker.
"""

from __future__ import annotations

import re
from collections import deque
from collections.abc import Iterator
from typing import Final

from .constants.log_files import (
    LOG_SECRET_MARKERS,
    LOG_SECRET_MIN_CHARACTERS,
    LOG_SECRET_REMEMBERED_WORDS,
    LOG_STRUCTURE_KEY_SUFFIXES,
    LOG_STRUCTURE_KEYS,
    LOG_WORD_KEYS,
)
from .layouts import LayoutPair

_TOKEN: Final = re.compile(r"\S+")
# Signs around a word that are no part of it (`пароль:`, `"секрет"`).
_EDGE_SIGNS: Final = ".,!?:;\"'()[]{}<>«»"
MASK: Final = "*"


def _markers() -> frozenset[str]:
    pair = LayoutPair()
    words = {word.casefold() for word in LOG_SECRET_MARKERS}
    # The marker typed in the other layout is the same word on the same keys (`gfhjkm` is `пароль`).
    keys = {pair.translate(word, "ru", "us") for word in words} | {pair.translate(word, "us", "ru") for word in words}
    return frozenset(_core(word) for word in words | keys)


def _core(token: str) -> str:
    return token.strip(_EDGE_SIGNS).casefold()


MARKERS: Final = _markers()


def secret_token(token: str) -> bool:
    """A token of LOG_SECRET_MIN_CHARACTERS characters or more, edge signs aside, with letters and digits in it: a
    password or a key looks like it (`parol123`)."""

    core = token.strip(_EDGE_SIGNS)
    return (len(core) >= LOG_SECRET_MIN_CHARACTERS and any(char.isalpha() for char in core)
            and any(char.isdigit() for char in core))


def masked_text(text: str, secrets: frozenset[str] = frozenset()) -> str:
    """`text` with each secret word as stars: a long token of letters and digits, the word right after a password
    marker, and each word of `secrets` (casefolded, edge signs aside)."""

    parts: list[str] = []
    last = 0
    after_marker = False
    for match in _TOKEN.finditer(text):
        token = match.group()
        core = _core(token)
        hidden = after_marker or secret_token(token) or core in secrets
        parts.append(text[last:match.start()])
        parts.append(MASK * len(token) if hidden else token)
        last = match.end()
        after_marker = core in MARKERS
    parts.append(text[last:])
    return "".join(parts)


def _structure(key: object) -> bool:
    return isinstance(key, str) and (key in LOG_STRUCTURE_KEYS or key.endswith(LOG_STRUCTURE_KEY_SUFFIXES))


def _masked(value: object, secrets: frozenset[str]) -> object:
    if isinstance(value, str):
        return masked_text(value, secrets)
    if isinstance(value, dict):
        return {key: item if _structure(key) else _masked(item, secrets) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [_masked(item, secrets) for item in value]
    return value


def _words(value: object) -> Iterator[str]:
    """The words of the texts under a record's word keys (LOG_WORD_KEYS), at any depth."""

    if isinstance(value, dict):
        for key, item in value.items():
            if key in LOG_WORD_KEYS:
                yield from _texts(item)
            elif not _structure(key):
                yield from _words(item)
    elif isinstance(value, (list, tuple)):
        for item in value:
            yield from _words(item)


def _texts(value: object) -> Iterator[str]:
    if isinstance(value, str):
        yield from (_core(token) for token in _TOKEN.findall(value))
    elif isinstance(value, dict):
        for item in value.values():
            yield from _texts(item)
    elif isinstance(value, (list, tuple)):
        for item in value:
            yield from _texts(item)


class LogRedactor:
    """Masks the secret words of each technical record (masked_text), and remembers the last words typed right after a
    password marker: a later record names such a word again without the marker (a correction, an edit)."""

    def __init__(self) -> None:
        self._secrets: deque[str] = deque(maxlen=LOG_SECRET_REMEMBERED_WORDS)

    def record(self, payload: dict[str, object]) -> dict[str, object]:
        context = payload.get("context")
        before = context.get("words") if isinstance(context, dict) else None
        previous = [_core(text.split()[-1]) for text in (before.values() if isinstance(before, dict) else ())
                    if isinstance(text, str) and text.split()]
        if any(word in MARKERS for word in previous):
            self._secrets.extend(word for word in _words(payload) if word and word not in MARKERS)
        masked = _masked(payload, frozenset(self._secrets))
        assert isinstance(masked, dict)
        return masked
