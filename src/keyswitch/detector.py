"""Conservative decision logic for automatic layout correction."""

from __future__ import annotations

from dataclasses import dataclass

from .language_model import LanguageModel, WordScore


@dataclass(frozen=True)
class DetectionDecision:
    should_convert: bool
    original: str
    replacement: str
    source_group: int
    target_group: int
    confidence: float
    reason: str
    source_score: WordScore
    target_score: WordScore


class LanguageDetector:
    def __init__(self, models: dict[int, LanguageModel]) -> None:
        if len(models) < 2:
            raise ValueError("At least two language models are required")
        self.models = models

    def decide(
        self,
        original: str,
        alternatives: dict[int, str],
        source_group: int,
        *,
        minimum_length: int = 3,
        confidence_threshold: float = 2.0,
        ignored_words: set[str] | None = None,
        aggressive: bool = False,
    ) -> DetectionDecision:
        ignored = {word.casefold() for word in (ignored_words or set())}
        source_model = self.models[source_group]
        source_score = source_model.score(original)
        rejected = DetectionDecision(
            False,
            original,
            original,
            source_group,
            source_group,
            0.0,
            "не требуется",
            source_score,
            source_score,
        )
        normalized = LanguageModel.normalize(original)
        if len(normalized) < minimum_length:
            return DetectionDecision(**{**rejected.__dict__, "reason": "короткое слово"})
        if normalized in ignored:
            return DetectionDecision(**{**rejected.__dict__, "reason": "исключение пользователя"})
        if any(character.isdigit() for character in original) or "_" in original:
            return DetectionDecision(**{**rejected.__dict__, "reason": "код или число"})
        candidates: list[tuple[float, int, str, WordScore]] = []
        for group, candidate in alternatives.items():
            if group == source_group or group not in self.models or candidate == original:
                continue
            score = self.models[group].score(candidate)
            candidates.append((score.value - source_score.value, group, candidate, score))
        if not candidates:
            return DetectionDecision(**{**rejected.__dict__, "reason": "нет другой раскладки"})
        delta, group, replacement, target_score = max(candidates, key=lambda item: item[0])
        safe_candidate = target_score.known and not source_score.known
        both_known_but_clear = (
            target_score.known
            and source_score.known
            and delta >= confidence_threshold + 2.5
        )
        pattern_candidate = aggressive and target_score.gram_ratio >= 0.82 and delta >= confidence_threshold
        should_convert = delta >= confidence_threshold and (
            safe_candidate or both_known_but_clear or pattern_candidate
        )
        if safe_candidate:
            reason = "слово найдено только в целевом языке"
        elif both_known_but_clear:
            reason = "целевая форма существенно вероятнее"
        elif pattern_candidate:
            reason = "характерная последовательность букв"
        elif source_score.known:
            reason = "исходное слово допустимо"
        elif not target_score.known:
            reason = "целевая форма не найдена"
        else:
            reason = "недостаточная уверенность"
        return DetectionDecision(
            should_convert,
            original,
            replacement,
            source_group,
            group,
            delta,
            reason,
            source_score,
            target_score,
        )
