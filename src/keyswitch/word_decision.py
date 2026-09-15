"""The automatic baseline shared by the event engine and action training."""

from __future__ import annotations

from .detector import DetectionDecision, LanguageDetector
from .intent_model import CorrectionTrigger
from .short_words import natural_short_source_veto, trusted_short_word_decision


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
    if not decision.should_convert:
        override = trusted_short_word_decision(
            detector, original, alternatives, source_group,
            ignored_words=() if ignored_words is None else ignored_words,
            rejected_targets=frozenset() if rejected_targets is None else rejected_targets,
            protect_code=protect_code, context_group=context_group,
        )
        if override is not None:
            return override
    return decision
