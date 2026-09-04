"""Conservative runtime exceptions for frequent two-letter function words.

This policy intentionally lives outside the frozen intent-model detector.  It
does not change the certified classifier toolchain and only admits reviewed,
exact lexicon hits with a large corpus-frequency advantage.
"""

from __future__ import annotations

import math
from dataclasses import replace
from collections.abc import Collection, Mapping, Set
from typing import Final

from .detector import DetectionDecision, LanguageDetector
from .language_model import LanguageModel


# Words a person types in the wrong layout every day and has to fix by hand.
# The list is curated, not generated: every entry is a frequent function word
# whose other-layout rendering is not a word anyone types on purpose, and the
# frequency and ratio gates below still have to agree.
TRUSTED_SINGLE_LETTER_WORDS: Final[frozenset[str]] = frozenset(
    {"а", "и", "с", "в", "к", "у", "о", "я"}
)
TRUSTED_SHORT_WORDS: Final[Mapping[int, frozenset[str]]] = {
    # EN: only the reported, manually reviewed collision. Broad lists of
    # two-letter words would make legitimate abbreviations such as RU ``шт``
    # candidates for EN ``in`` and would defeat the precision-first policy.
    0: frozenset({"if"}),
    1: frozenset(
        {
            "не", "ли", "на", "по", "то", "мы", "вы", "ты", "он", "но", "же",
            "за", "до", "из", "от", "об", "ну", "их", "им", "да", "ни", "во",
            "ко", "со", "бы", "уж", "ей", "её",
        }
    )
    | TRUSTED_SINGLE_LETTER_WORDS,
}
TRUSTED_SHORT_WORD_MAX_LENGTH: Final[int] = 2
# A short token found only in the other language's frequency list is thin
# evidence: "дев" reads as ordinary Russian (n-gram z-score -0.7) yet "ltd" is
# a frequent English token, and the intent model never sees tokens this short.
# Up to this length the source must also read unnaturally — the bar the
# detector's unknown-word branch already applies — unless the recent context
# favours the target language. This lives here, not in the detector, because
# the detector is part of the certified model toolchain.
NATURAL_SOURCE_MAX_LENGTH: Final[int] = 4
NATURAL_SOURCE_NGRAM_SCORE: Final[float] = -1.1
DICTIONARY_ONLY_REASONS: Final[frozenset[str]] = frozenset(
    {
        "слово найдено только в целевом частотном словаре",
        "словоформа подтверждена морфологическим словарём",
    }
)
NATURAL_SOURCE_REASON: Final[str] = (
    "короткое слово читается естественно в исходном языке"
)


def natural_short_source_veto(
    decision: DetectionDecision,
    *,
    context_group: int | None,
) -> DetectionDecision:
    """Withhold a dictionary-only conversion of a short, natural-looking word."""

    if not decision.should_convert or decision.reason not in DICTIONARY_ONLY_REASONS:
        return decision
    length = max(
        len(LanguageModel.normalize(decision.original)),
        len(LanguageModel.normalize(decision.replacement)),
    )
    if (
        length > NATURAL_SOURCE_MAX_LENGTH
        or decision.source_score.ngram_score <= NATURAL_SOURCE_NGRAM_SCORE
        or context_group == decision.target_group
    ):
        return decision
    return replace(decision, should_convert=False, reason=NATURAL_SOURCE_REASON)
TRUSTED_SHORT_WORD_MINIMUM_FREQUENCY: Final[int] = 10_000
TRUSTED_SHORT_WORD_MINIMUM_RATIO: Final[float] = 100.0
# With the previous word already in the target language, a two-letter entry
# only has to be at least as frequent as the token it replaces: ``kb`` and
# ``nj`` are real English tokens, but after a Russian word they are ``ли`` and
# ``то``. The frequency lists hold no single letters at all, so a one-letter
# entry relies on the curated list and that context alone.
TRUSTED_SHORT_WORD_CONTEXT_RATIO: Final[float] = 1.0
SINGLE_LETTER_CONFIDENCE: Final[float] = 1.0
CONTEXT_SHORT_WORD_REASON: Final[str] = (
    "короткое слово из безопасного списка после слова на целевом языке"
)


SHORT_WORD_REASONS: Final[frozenset[str]] = frozenset(
    {"частотное короткое слово из безопасного списка", CONTEXT_SHORT_WORD_REASON}
)


def is_short_word_override(decision: DetectionDecision) -> bool:
    """Whether the decision came from the trusted short-word list."""

    return decision.should_convert and decision.reason in SHORT_WORD_REASONS


def trusted_short_word_decision(
    detector: LanguageDetector,
    original: str,
    alternatives: Mapping[int, str],
    source_group: int,
    *,
    ignored_words: Collection[str],
    rejected_targets: Set[int],
    protect_code: bool,
    context_group: int | None = None,
) -> DetectionDecision | None:
    """Return a high-precision short-word override, or leave policy unchanged.

    A single letter is a word only in a sentence: ``f`` alone may be a variable
    name, so one-letter entries need the previous word to be in the target
    language (``context_group``); two-letter entries stand on their own.
    """

    ignored_keys = {detector.token_key(word) for word in ignored_words}
    if detector.token_key(original) in ignored_keys:
        return None
    if protect_code and detector.is_protected_token(original):
        return None

    source_model = detector.models[source_group]
    source_score = source_model.score(original)
    normalized_original = LanguageModel.normalize(original)
    candidates: list[DetectionDecision] = []
    for target_group, replacement in alternatives.items():
        target_model = detector.models.get(target_group)
        if (
            target_group == source_group
            or target_model is None
            or replacement == original
            or target_group in rejected_targets
        ):
            continue
        normalized_replacement = LanguageModel.normalize(replacement)
        if (
            max(len(normalized_original), len(normalized_replacement))
            > TRUSTED_SHORT_WORD_MAX_LENGTH
            or normalized_replacement
            not in TRUSTED_SHORT_WORDS.get(target_group, frozenset())
        ):
            continue
        supported = context_group == target_group
        target_score = target_model.score(replacement)
        if normalized_replacement in TRUSTED_SINGLE_LETTER_WORDS:
            if not supported:
                continue
            candidates.append(
                DetectionDecision(
                    True,
                    original,
                    replacement,
                    source_group,
                    target_group,
                    SINGLE_LETTER_CONFIDENCE,
                    CONTEXT_SHORT_WORD_REASON,
                    source_score,
                    target_score,
                )
            )
            continue
        if (
            not target_score.exact
            or target_score.frequency < TRUSTED_SHORT_WORD_MINIMUM_FREQUENCY
        ):
            continue
        ratio = (target_score.frequency + 1) / (source_score.frequency + 1)
        if ratio < TRUSTED_SHORT_WORD_MINIMUM_RATIO and not (
            supported and ratio >= TRUSTED_SHORT_WORD_CONTEXT_RATIO
        ):
            continue
        candidates.append(
            DetectionDecision(
                True,
                original,
                replacement,
                source_group,
                target_group,
                math.log10(ratio),
                (
                    "частотное короткое слово из безопасного списка"
                    if ratio >= TRUSTED_SHORT_WORD_MINIMUM_RATIO
                    else CONTEXT_SHORT_WORD_REASON
                ),
                source_score,
                target_score,
            )
        )
    return max(candidates, key=lambda item: item.confidence, default=None)
