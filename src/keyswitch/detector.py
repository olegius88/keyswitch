"""Precision-first ensemble for automatic keyboard-layout correction."""

from __future__ import annotations

import unicodedata
from collections.abc import Mapping
from dataclasses import dataclass, replace
from pathlib import Path
from typing import Final, Protocol

from .intent_model import (
    CorrectionTrigger,
    IntentModelInput,
    LinearPrediction,
    MINIMUM_RUNTIME_TOKEN_LENGTH,
    normalize_token,
)
from .language_model import LanguageModel, WordScore


def _load_protected_tokens() -> frozenset[str]:
    path = Path(__file__).resolve().parent / "resources" / "protected_tokens.txt"
    try:
        return frozenset(
            line.strip().casefold()
            for line in path.read_text(encoding="utf-8").splitlines()
            if line.strip() and not line.lstrip().startswith("#")
        )
    except OSError:
        return frozenset()


PROTECTED_TOKENS = _load_protected_tokens()

# Public policy constants keep offline production-context evaluation tied to
# the exact arithmetic used by the serving detector.  LanguageModel's
# context_score contract is bounded to [0, 1].
CONTEXT_SCORE_MINIMUM: Final[float] = 0.0
CONTEXT_SCORE_MAXIMUM: Final[float] = 1.0
CONTEXT_DELTA_MULTIPLIER: Final[float] = 1.75
CONTEXT_TARGET_GROUP_BONUS: Final[float] = 0.55
CONTEXT_SOURCE_GROUP_PENALTY: Final[float] = 0.3

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
    model_probability: float | None = None
    model_threshold: float | None = None
    model_version: str = ""


class LanguageScorer(Protocol):
    """Structural contract consumed by the detector."""

    def score(self, word: str) -> WordScore: ...

    def context_score(self, previous: str, word: str) -> float: ...

    def best_single_deletion(self, word: str) -> WordScore: ...


class IntentClassifier(Protocol):
    """Minimal inference contract used by the policy layer."""

    @property
    def veto_threshold(self) -> float: ...

    def predict(self, item: IntentModelInput) -> LinearPrediction: ...


class LanguageDetector:
    """Fuse lexicons, morphology, character statistics and recent context.

    False corrections are deliberately more expensive than missed ones: an
    ambiguous token stays untouched and is always available to the manual
    conversion hotkey. Explicit learned rules are evaluated before statistical
    heuristics.
    """

    def __init__(
        self,
        models: Mapping[int, LanguageScorer],
        intent_model: IntentClassifier | None = None,
    ) -> None:
        if len(models) < 2:
            raise ValueError("At least two language models are required")
        self.models = dict(models)
        self.intent_model = intent_model

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
        protect_code: bool = True,
        previous_words: dict[int, str] | None = None,
        context_group: int | None = None,
        forced_target_group: int | None = None,
        rejected_targets: set[int] | None = None,
        trigger: CorrectionTrigger = "space",
        use_intent_model: bool = True,
    ) -> DetectionDecision:
        ignored = {self.token_key(word) for word in (ignored_words or set())}
        rejected_groups = rejected_targets or set()
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
        scored: list[tuple[float, int, str, WordScore, float]] = []
        previous = previous_words or {}
        for group, candidate in alternatives.items():
            if group == source_group or group not in self.models or candidate == original:
                continue
            target_score = self.models[group].score(candidate)
            delta = target_score.value - source_score.value
            context_delta = self._context_delta(
                source_group,
                group,
                original,
                candidate,
                previous,
                context_group,
            )
            delta += context_delta
            scored.append((delta, group, candidate, target_score, context_delta))
        if not scored:
            return replace(rejected, reason="нет другой раскладки")

        delta, group, replacement, target_score, context_delta = max(
            scored, key=lambda item: item[0]
        )
        normalized = LanguageModel.normalize(original)
        original_key = self.token_key(original)
        replacement_normalized = LanguageModel.normalize(replacement)
        effective_length = max(
            [len(normalized), len(replacement_normalized)]
            + [len(LanguageModel.normalize(candidate)) for candidate in alternatives.values()]
        )
        if effective_length < minimum_length:
            return replace(rejected, reason="короткое слово")
        if original_key in ignored:
            return replace(rejected, reason="исключение пользователя")
        if group in rejected_groups:
            return replace(rejected, reason="отклонённое пользователем исправление")

        if forced_target_group is not None:
            forced = next(
                (item for item in scored if item[1] == forced_target_group),
                None,
            )
            if forced is not None:
                forced_delta, forced_group, forced_text, forced_score, _context = forced
                return DetectionDecision(
                    True,
                    original,
                    forced_text,
                    source_group,
                    forced_group,
                    max(20.0, forced_delta),
                    "подтверждённое правило пользователя",
                    source_score,
                    forced_score,
                )

        if protect_code and self._looks_like_protected_token(original):
            return replace(rejected, reason="код, адрес или аббревиатура")

        # A valid source token is the strongest false-positive guard. When
        # both decodings are valid, leave the inherently ambiguous word alone.
        if source_score.known:
            return replace(
                rejected,
                reason=(
                    "обе раскладки дают допустимое слово"
                    if target_score.known
                    else "исходное слово допустимо"
                ),
            )

        prediction = self._intent_prediction(
            original,
            replacement,
            source_group,
            group,
            trigger,
            source_score,
            target_score,
            context_delta,
            context_group,
            use_intent_model,
        )

        # Once a supported model has observed a token, its calibrated,
        # trigger-specific decision is the sole statistical switching signal.
        # Applying feature-coverage or language-score heuristics afterwards is
        # a second statistical veto: it invalidates the recall certified for
        # the selected model threshold.  Coverage and lexical scores remain
        # diagnostics only.  The heuristic ensemble below is available for
        # short tokens and installations where the artifact is absent or
        # explicitly disabled.
        if prediction is not None:
            should_convert = prediction.should_switch
            if should_convert:
                reason = "уверенное решение линейной n-граммной модели"
            else:
                reason = "линейная модель не достигла безопасного порога"
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
                *(self._prediction_fields(prediction)),
            )

        if target_score.known:
            length_relief = min(1.0, max(0, effective_length - 3) * 0.18)
            required = max(0.65, confidence_threshold - length_relief)
            if target_score.spell_known and not target_score.exact:
                required += 0.15
            context_supports_target = context_group == group
            # A Hunspell hit is already strong morphological evidence. The
            # n-gram score is clamped at -4 for valid but very rare forms, so
            # accept that floor while still requiring an invalid source and a
            # clear total-score margin.
            morphology_is_plausible = (
                target_score.ngram_score >= -4.0 or context_supports_target
            )
            heuristic_should_convert = delta >= required and morphology_is_plausible
            should_convert = heuristic_should_convert
            if target_score.exact:
                reason = "слово найдено только в целевом частотном словаре"
            elif should_convert:
                reason = "словоформа подтверждена морфологическим словарём"
            elif not morphology_is_plausible:
                reason = "редкая словоформа без достаточного контекста"
            else:
                reason = "недостаточный перевес целевой словоформы"
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
                *(self._prediction_fields(prediction)),
            )

        # One accidental extra character should not erase otherwise decisive
        # dictionary evidence. This does not correct the typo itself; it only
        # chooses the layout of the original physical sequence.
        if effective_length >= 5:
            target_without_one = self.models[group].best_single_deletion(replacement)
            source_without_one = source_model.best_single_deletion(original)
            typo_delta = target_without_one.value - max(
                source_score.value, source_without_one.value
            )
            typo_supported = (
                target_without_one.known
                and not source_without_one.known
                and typo_delta >= confidence_threshold + 0.5
            )
            if typo_supported:
                return DetectionDecision(
                    True,
                    original,
                    replacement,
                    source_group,
                    group,
                    max(delta, typo_delta),
                    "целевая раскладка подтверждается после удаления одной опечатки",
                    source_score,
                    target_score,
                    *(self._prediction_fields(prediction)),
                )

        # Unknown words can still be recognised by smoothed character n-grams,
        # but only when the source looks distinctly unnatural. The required
        # margin decreases with length because longer sequences carry more
        # independent evidence.
        length_relief = min(2.0, max(0, effective_length - 4) * 0.32)
        required = confidence_threshold + 2.4 - length_relief
        if aggressive:
            required -= 0.75
        required = max(confidence_threshold + 0.25, required)
        source_is_unlikely = source_score.ngram_score <= (-0.65 if aggressive else -1.1)
        target_is_plausible = target_score.ngram_score >= (-2.0 if aggressive else -1.25)
        heuristic_should_convert = (
            effective_length >= (4 if aggressive else 5)
            and source_is_unlikely
            and target_is_plausible
            and delta >= required
        )
        should_convert = heuristic_should_convert
        if should_convert:
            reason = "устойчивый перевес символьной языковой модели"
        elif not source_is_unlikely:
            reason = "исходная последовательность похожа на допустимое слово"
        elif not target_is_plausible:
            reason = "целевая последовательность тоже нетипична"
        else:
            reason = "недостаточная уверенность статистической модели"
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
            *(self._prediction_fields(prediction)),
        )

    def _intent_prediction(
        self,
        original: str,
        replacement: str,
        source_group: int,
        target_group: int,
        trigger: CorrectionTrigger,
        source_score: WordScore,
        target_score: WordScore,
        context_delta: float,
        context_group: int | None,
        use_intent_model: bool,
    ) -> LinearPrediction | None:
        if (
            not use_intent_model
            or self.intent_model is None
            or max(
                len(normalize_token(original)),
                len(normalize_token(replacement)),
            )
            < MINIMUM_RUNTIME_TOKEN_LENGTH
        ):
            return None
        return self.intent_model.predict(
            IntentModelInput(
                original=original,
                alternative=replacement,
                source_group=source_group,
                target_group=target_group,
                trigger=trigger,
                source_score=source_score,
                target_score=target_score,
                context_delta=context_delta,
                context_group=context_group,
            )
        )

    @staticmethod
    def _prediction_fields(
        prediction: LinearPrediction | None,
    ) -> tuple[float | None, float | None, str]:
        if prediction is None:
            return None, None, ""
        return prediction.probability, prediction.threshold, prediction.model_version

    def _context_delta(
        self,
        source_group: int,
        target_group: int,
        source_word: str,
        target_word: str,
        previous_words: dict[int, str],
        context_group: int | None,
    ) -> float:
        source_context = self.models[source_group].context_score(
            previous_words.get(source_group, ""), source_word
        )
        target_context = self.models[target_group].context_score(
            previous_words.get(target_group, ""), target_word
        )
        delta = CONTEXT_DELTA_MULTIPLIER * (
            target_context - source_context
        )
        if context_group == target_group:
            delta += CONTEXT_TARGET_GROUP_BONUS
        elif context_group == source_group:
            delta -= CONTEXT_SOURCE_GROUP_PENALTY
        return delta

    @staticmethod
    def token_key(token: str) -> str:
        """Case-insensitive identity that keeps layout-significant punctuation."""

        return token.strip().casefold()

    @staticmethod
    def is_protected_token(token: str) -> bool:
        """Expose the structural guard to the physical-token boundary logic."""

        return LanguageDetector._looks_like_protected_token(token)

    @staticmethod
    def _looks_like_protected_token(token: str) -> bool:
        if len(token) > 64:
            return True
        lowered = token.casefold()
        if lowered in PROTECTED_TOKENS:
            return True
        if lowered.startswith("-"):
            return True
        if any(marker in lowered for marker in ("http://", "https://", "www.", "@")):
            return True
        if any(character.isdigit() for character in token):
            return True
        if any(character in "_/\\=:" for character in token):
            return True
        letters = [character for character in token if character.isalpha()]
        if len(letters) >= 2 and all(character.isupper() for character in letters):
            return True
        if any(character.isupper() for character in letters[1:]):
            return True
        if any(lowered[index : index + 4] == lowered[index] * 4 for index in range(max(0, len(lowered) - 3))):
            return True
        scripts = {
            "CYRILLIC" if "CYRILLIC" in unicodedata.name(character, "") else "LATIN"
            for character in letters
            if "CYRILLIC" in unicodedata.name(character, "")
            or "LATIN" in unicodedata.name(character, "")
        }
        return len(scripts) > 1
