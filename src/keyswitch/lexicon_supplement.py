"""Packaged lexicon supplements: extra known forms outside the certified language-model toolchain.

The frozen ``language_model.py`` and the onboard ``.lm`` files are pinned by the
Layout Intent certification and must not change. A supplement adds forms the
onboard vocabulary lacks (colloquial words, interjections, transliterated names)
through the existing ``extra_words`` hook of ``LanguageModel.load``. The same
file feeds the engine, the early-switch models, the trainers and the evaluators,
so lexical evidence never differs between corpus construction and serving.
"""

from __future__ import annotations

import json
import re
from functools import lru_cache
from pathlib import Path
from typing import Final

SUPPLEMENT_ROOT: Final = Path(__file__).parent / "resources"
SUPPLEMENT_PATTERNS: Final = {"ru_RU": r"[а-яё]{3,}", "en_US": r"[a-z][a-z'-]*"}
# Two-letter Cyrillic forms are refused: the onboard vocabulary already holds every real
# two-letter Russian word, so a frequency list adds only what its count floor misses at that
# length - doubled letters, transliteration fragments and subtitle noise. Each one disguises a
# short Latin command as a Russian word, which is how `pm2` came to be rewritten as `зь2`.
# Russian has exactly eight single-letter words, and "я" is the most frequent token of the
# frequency list this supplement is built from; the {2,} shape dropped all of them, so the
# engine could only learn them from a spell checker, which calls every lone Latin letter a
# word as well and therefore never told `z` from `я` apart. Any other single letter is a
# letter, not a word, and is still refused.
SUPPLEMENT_SINGLE_LETTERS: Final = {"ru_RU": frozenset("аисвкуоя"), "en_US": frozenset("ai")}
MAX_SUPPLEMENT_BYTES: Final = 8 * 1024 * 1024


@lru_cache(maxsize=8)
def supplement_words(locale: str) -> tuple[str, ...]:
    """Sorted extra forms for a locale; empty when no supplement is packaged.

    A malformed supplement is an error, never a silent gap.
    """

    path = SUPPLEMENT_ROOT / f"lexicon-supplement-{locale}.json"
    if not path.is_file():
        return ()
    raw = path.read_bytes()
    if len(raw) > MAX_SUPPLEMENT_BYTES:
        raise ValueError("lexicon supplement is too large")
    payload: object = json.loads(raw)
    if (not isinstance(payload, dict) or payload.get("schema_version") != 1 or payload.get("locale") != locale
            or not isinstance(payload.get("words"), list)):
        raise ValueError("invalid lexicon supplement: " + path.name)
    pattern = SUPPLEMENT_PATTERNS.get(locale)
    words: list[object] = payload["words"]
    if pattern is None or not words or words != sorted(set(words), key=str):
        raise ValueError("lexicon supplement must be a sorted set for a supported locale")
    result: list[str] = []
    for word in words:
        allowed = SUPPLEMENT_SINGLE_LETTERS.get(locale, frozenset())
        if not isinstance(word, str) or (re.fullmatch(pattern, word) is None and word not in allowed):
            raise ValueError("invalid lexicon supplement entry")
        result.append(word)
    return tuple(result)
