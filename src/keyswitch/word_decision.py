"""The automatic baseline shared by the event engine and action training."""

from __future__ import annotations

from collections.abc import Collection, Mapping, Set
from dataclasses import replace

from .detector import DetectionDecision, LanguageDetector
from .intent_model import CorrectionTrigger
from .language_model import LanguageModel
from .short_words import (
    ISOLATED_SHORT_WORD_REASON, SINGLE_LETTER_CONFIDENCE, TRUSTED_SINGLE_LETTER_WORDS,
    natural_short_source_veto, trusted_short_word_decision,
)


def automatic_word_decision(
    detector: LanguageDetector, original: str, alternatives: dict[int, str],
    source_group: int, *, minimum_length: int = 3,
    confidence_threshold: float = 2.0, ignored_words: set[str] | None = None,
    aggressive: bool = False, protect_code: bool = True,
    previous_words: dict[int, str] | None = None, context_group: int | None = None,
    forced_target_group: int | None = None, rejected_targets: set[int] | None = None,
    trigger: CorrectionTrigger = "space", use_intent_model: bool = True,
) -> DetectionDecision:
    decision = detector.decide(
        original, alternatives, source_group, minimum_length=minimum_length,
        confidence_threshold=confidence_threshold, ignored_words=ignored_words,
        aggressive=aggressive, protect_code=protect_code, previous_words=previous_words,
        context_group=context_group, forced_target_group=forced_target_group,
        rejected_targets=rejected_targets, trigger=trigger, use_intent_model=use_intent_model,
    )
    decision = natural_short_source_veto(decision, context_group=context_group)
    decision = word_shape_veto(decision)
    if not decision.should_convert:
        override = trusted_short_word_decision(
            detector, original, alternatives, source_group,
            ignored_words=() if ignored_words is None else ignored_words,
            rejected_targets=frozenset() if rejected_targets is None else rejected_targets,
            protect_code=protect_code, context_group=context_group,
        )
        if override is not None:
            return override
        opening = opening_letter_decision(
            detector, original, alternatives, source_group, context_group=context_group,
            ignored_words=() if ignored_words is None else ignored_words,
            rejected_targets=frozenset() if rejected_targets is None else rejected_targets,
            protect_code=protect_code,
        )
        if opening is not None:
            return opening
    return decision


def opening_letter_decision(
    detector: LanguageDetector, original: str, alternatives: Mapping[int, str], source_group: int, *,
    context_group: int | None, ignored_words: Collection[str], rejected_targets: Set[int], protect_code: bool,
) -> DetectionDecision | None:
    """A lone curated letter with no previous word converts; after an English word it stays.

    With no previous word at all - the start of a message, a fresh field, or a
    pause the engine no longer bridges - the evidence points the other way from
    the middle of a sentence. In the Russian training treebank (UD Taiga, 121 967
    sentences) 14% of sentences open with one of the curated single-letter words
    (в 5405, и 3852, а 3239, я 2187, с 821, у 821, к 457, о 301); in the English
    one (UD EWT train, 12 544 sentences) no sentence opens with a lone
    f/b/c/d/r/e/j/z, while inside sentences those letters do occur (b 40, r 18,
    c 15, f 15, d 11, e 7, j 3). Measured 16.09.2026 on the base corpus sources
    in .t/reliable-release-2026-09-12/corpus-sources. The rule sits here, in the
    layer the engine and the context-action corpus share, and not in the curated
    table that generates the frozen context-v1 training scenarios.
    """

    if context_group is not None or len(LanguageModel.normalize(original)) != 1:
        return None
    ignored_keys = {detector.token_key(word) for word in ignored_words}
    if detector.token_key(original) in ignored_keys or (protect_code and detector.is_protected_token(original)):
        return None
    source_model = detector.models[source_group]
    for target_group, replacement in alternatives.items():
        target_model = detector.models.get(target_group)
        if (target_group == source_group or target_model is None or replacement == original
                or target_group in rejected_targets
                or LanguageModel.normalize(replacement) not in TRUSTED_SINGLE_LETTER_WORDS):
            continue
        return DetectionDecision(
            True, original, replacement, source_group, target_group, SINGLE_LETTER_CONFIDENCE,
            ISOLATED_SHORT_WORD_REASON, source_model.score(original), target_model.score(replacement),
        )
    return None


def word_shape_veto(decision: DetectionDecision) -> DetectionDecision:
    """A word may not be replaced by something that is not a word.

    Six Russian letters sit on punctuation keys, so the other reading of a token
    like ``рукх`` is ``her[``. The character models score such a reading on its
    letters alone and can prefer it, which is how a mistyped Russian word turned
    into a bracket. The engine is asked about a word, and the keys it would type
    instead must spell one too; nothing is vetoed in the opposite direction,
    where ``[jhjij`` becomes the ordinary ``хорошо``, nor for a source that
    already carries a hyphen or an apostrophe.
    """

    if not decision.should_convert or not decision.original.isalpha():
        return decision
    if decision.replacement.isalpha():
        return decision
    return replace(
        decision,
        should_convert=False,
        reason="другая раскладка пишет это не буквами",
    )
