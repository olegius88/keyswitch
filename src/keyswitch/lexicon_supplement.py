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
SUPPLEMENT_PATTERNS: Final = {"ru_RU": r"[а-яё]{2,}", "en_US": r"[a-z][a-z'-]*"}
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
        if not isinstance(word, str) or re.fullmatch(pattern, word) is None:
            raise ValueError("invalid lexicon supplement entry")
        result.append(word)
    return tuple(result)
