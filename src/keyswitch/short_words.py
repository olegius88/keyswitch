"""Conservative runtime exceptions for frequent two-letter function words.

This policy intentionally lives outside the frozen intent-model detector.  It
does not change the certified classifier toolchain and only admits reviewed,
exact lexicon hits with a large corpus-frequency advantage.
"""

from __future__ import annotations

import math
from collections.abc import Collection, Mapping, Set
from typing import Final

from .detector import DetectionDecision, LanguageDetector
from .language_model import LanguageModel


TRUSTED_SHORT_WORDS: Final[Mapping[int, frozenset[str]]] = {
    # Start with the reported, manually reviewed collision only. Broad lists of
    # two-letter words would make legitimate abbreviations such as RU ``шт``
    # candidates for EN ``in`` and would defeat the precision-first
    # minimum-length policy.
    0: frozenset({"if"}),
}
TRUSTED_SHORT_WORD_MAX_LENGTH: Final[int] = 2
TRUSTED_SHORT_WORD_MINIMUM_FREQUENCY: Final[int] = 10_000
TRUSTED_SHORT_WORD_MINIMUM_RATIO: Final[float] = 100.0


def trusted_short_word_decision(
    detector: LanguageDetector,
    original: str,
    alternatives: Mapping[int, str],
    source_group: int,
    *,
    ignored_words: Collection[str],
    rejected_targets: Set[int],
    protect_code: bool,
) -> DetectionDecision | None:
    """Return a high-precision short-word override, or leave policy unchanged."""

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
        target_score = target_model.score(replacement)
        if (
            not target_score.exact
            or target_score.frequency < TRUSTED_SHORT_WORD_MINIMUM_FREQUENCY
        ):
            continue
        ratio = (target_score.frequency + 1) / (source_score.frequency + 1)
        if ratio < TRUSTED_SHORT_WORD_MINIMUM_RATIO:
            continue
        candidates.append(
            DetectionDecision(
                True,
                original,
                replacement,
                source_group,
                target_group,
                math.log10(ratio),
                "частотное короткое слово из безопасного списка",
                source_score,
                target_score,
            )
        )
    return max(candidates, key=lambda item: item.confidence, default=None)
