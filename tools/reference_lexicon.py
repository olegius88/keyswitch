"""Reference language models for the current pipeline: the frozen onboard lexicons plus the
packaged supplements, exactly as the engine loads them.

``context_evidence.reference_models`` stays byte-identical for the historical context-v2
gate; every live trainer, evaluator and diagnostic uses this module instead, so lexical
evidence never differs between corpus construction and serving.
"""
from __future__ import annotations

import json
from pathlib import Path
from typing import cast

from keyswitch.language_model import LOCALE_FALLBACKS, LanguageModel
from keyswitch.lexicon_supplement import supplement_words

from freeze_context_action_corpus import checksum

ROOT = Path(__file__).resolve().parents[1]
# Mirrors the synthetic-frequency fallback in keyswitch.language_model so a
# lexicon without frequency data still ranks below any real observed word.
SYNTHETIC_FREQUENCY_FLOOR = 1000
SYNTHETIC_FREQUENCY_DIVISOR = 20


def reference_models(spelling: bool) -> dict[int, LanguageModel]:
    """Pinned onboard lexicons plus packaged supplements; optional reference morphology."""
    config = cast(dict[str, object], json.loads((ROOT / "model/intent_v1/config.json").read_bytes()))
    sources = cast(dict[str, object], config["sources"])
    languages = cast(dict[str, dict[str, object]], sources["languages"])
    external = cast(dict[str, object], config["external_evaluation"])
    dictionaries = cast(dict[str, dict[str, object]], external["hunspell"])
    models: dict[int, LanguageModel] = {}
    for group, locale in enumerate(("en_US", "ru_RU")):
        spec = languages[locale]
        path = ROOT / str(spec["path"])
        if checksum(path) != spec["sha256"]:
            raise ValueError("reference lexicon checksum mismatch")
        frequencies, bigrams = LanguageModel._read_arpa(path)
        frequency = max(
            max(frequencies.values(), default=SYNTHETIC_FREQUENCY_FLOOR) // SYNTHETIC_FREQUENCY_DIVISOR,
            SYNTHETIC_FREQUENCY_FLOOR,
        )
        for word in (*LOCALE_FALLBACKS[locale], *supplement_words(locale)):
            normalized = LanguageModel.normalize(word)
            if normalized:
                frequencies[normalized] = max(frequencies.get(normalized, 0), frequency)
        model = LanguageModel(locale, frequencies, str(path), bigrams, enable_spellcheck=spelling)
        if spelling:
            dictionary = Path(model.speller.source)
            if (not model.speller.available or checksum(dictionary) != dictionaries[locale]["dictionary_sha256"]
                    or checksum(dictionary.with_suffix(".aff")) != dictionaries[locale]["affix_sha256"]):
                raise ValueError("reference morphology unavailable or changed")
        models[group] = model
    return models
