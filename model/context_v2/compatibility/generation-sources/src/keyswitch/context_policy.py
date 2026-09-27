"""Contextual policy orchestration, independent of keyboard injection."""

from __future__ import annotations

from dataclasses import dataclass, replace
from typing import ClassVar, Final

from .context_model import ContextEvidence, ContextModel, ContextPrediction
from .detector import DetectionDecision, LanguageDetector
from .input_context import FieldContext, FieldReader, InputContext
from .ortho_model import OrthoEvidence, OrthoModel, shape_of
from .short_words import is_short_word_override


@dataclass(frozen=True)
class ContextResult:
    decision: DetectionDecision
    prediction: ContextPrediction | None = None
    field: FieldContext | None = None
    policy_applied: bool = False
    decision_source: str = "baseline"
    fallback_reason: str = ""


INNER_MARKS: Final[str] = "-'’"


def word_shaped(token: str) -> bool:
    """Is this a word in the layout it was typed in, or a fragment?

    A character model trained on words can only speak about words. `и"ю` is a
    quotation mark caught between two letters, not Russian written badly, and
    left in the population such fragments set every threshold. A hyphen or an
    apostrophe inside a token is different: the engine joins those to the word
    before it tests for a boundary at all, so `Я-то`, `из-за` and `don't` reach
    a model whole and are ordinary words of their language.
    """

    if len(token) < 2 or not token[0].isalpha() or not token[-1].isalpha():
        return False
    marks = 0
    for character in token[1:-1]:
        if character.isalpha():
            continue
        if character not in INNER_MARKS:
            return False
        marks += 1
        if marks > 1:
            return False
    return True


class ContextPolicy:
    # The orthotactic artifact is several megabytes and immutable once loaded.
    # A running application builds one engine, but replays and tests build many,
    # so the parse is shared rather than repeated per instance.
    _shared_ortho: ClassVar[tuple[OrthoModel | None, str] | None] = None

    def __init__(self, reader: FieldReader | None = None) -> None:
        self.stream = InputContext()
        self.model, self.status = ContextModel.try_load()
        if ContextPolicy._shared_ortho is None:
            ContextPolicy._shared_ortho = OrthoModel.try_load()
        self.ortho, self.ortho_status = ContextPolicy._shared_ortho
        self.reader = reader

    def decide(
        self, baseline: DetectionDecision, alternative: str, target_group: int,
        detector: LanguageDetector, trigger: str, mode: str,
        *, after: str = "", read_field: bool = False,
        field_override: FieldContext | None = None,
        literal_tail: str = "",
    ) -> ContextResult:
        if mode not in {"assist", "shadow"} or self.model is None:
            return ContextResult(baseline, fallback_reason="mode_disabled" if mode not in {"assist", "shadow"} else "model_unavailable")
        original = baseline.original
        anchor = original + literal_tail
        field = field_override or self.stream.snapshot(anchor)
        if read_field and self.reader is not None:
            snapshot = self.reader.read(field.application, self.stream.window)
            if snapshot is not None and snapshot.application == field.application:
                snapshot = snapshot.bounded()
                # A native snapshot includes the current word and possibly a
                # boundary. Only use it if anchored to this exact suffix.
                if snapshot.sensitive or snapshot.selection:
                    return ContextResult(replace(baseline, should_convert=False, reason="защищённое поле или выделение"), field=snapshot, decision_source="safety", fallback_reason="sensitive_or_selected_field")
                before = snapshot.before
                if anchor and before.endswith(anchor):
                    field = replace(snapshot, before=before[:-len(anchor)])
                elif anchor and before[:-1].endswith(anchor):
                    field = replace(snapshot, before=before[:-len(anchor) - 1])
                else:
                    # The editor contradicts the observer: do not fall back
                    # to stale strokes and erase a different span of text.
                    return ContextResult(replace(baseline, should_convert=False, reason="текст активного поля изменился"), field=snapshot, decision_source="safety", fallback_reason="field_changed")
        if after:
            field = replace(field, after=after)
        source = detector.models[baseline.source_group].score(original)
        target = detector.models[target_group].score(alternative)
        prediction = self.model.predict(ContextEvidence(
            original, alternative, baseline.source_group, field, trigger,
            baseline.should_convert, source.known, target.known, target.value - source.value,
        ))
        if mode == "shadow":
            return ContextResult(baseline, prediction, field, fallback_reason="shadow_mode")
        # Missing context is honest uncertainty, not an instruction to guess.
        # A wait/suggestion can still describe a short ambiguous first word.
        if not prediction.supported and prediction.action not in {"wait", "suggest"}:
            return ContextResult(baseline, prediction, field, fallback_reason="unsupported_context")
        if prediction.action == "convert":
            decision = replace(
                baseline, should_convert=True, replacement=alternative,
                target_group=target_group, source_score=source, target_score=target,
                reason="решение контекстной модели", confidence=prediction.probability,
            )
        elif prediction.action != "wait" and is_short_word_override(baseline):
            # A curated, reviewed exception is an explicit rule, not a guess, so
            # neither a probabilistic `keep` nor an under-confident `convert`
            # cancels it. Only `wait` still delays it, because that is about
            # timing rather than direction and the lookahead may resolve the
            # word jointly. The model's opinion is recorded either way.
            return ContextResult(baseline, prediction, field, decision_source="short_word_override",
                                 fallback_reason="trusted_short_word")
        else:
            decision = replace(baseline, should_convert=False, reason={
                "keep": "контекстная модель оставляет текст",
                "wait": "контекстная модель ждёт продолжения",
                "suggest": "контекстная модель предлагает проверить раскладку",
            }[prediction.action])
        result = ContextResult(decision, prediction, field, policy_applied=True,
                               decision_source="context_model")
        return self._licensed(result, baseline, alternative, target_group, field)

    def _licensed(self, result: ContextResult, baseline: DetectionDecision, alternative: str,
                  target_group: int, field: FieldContext) -> ContextResult:
        """Let the orthotactic model turn a refusal into a conversion, never the reverse.

        The word models answer "is this a word"; this one answers "could this
        sequence of keys have been produced by this language at all". That is
        the only evidence available for a token no dictionary contains, which is
        why an unknown command typed in the wrong layout used to survive every
        earlier layer untouched. It never vetoes, never overrides an explicit
        rule, and `wait` still outranks it, because waiting is about timing.

        It also stays out of a contextual veto: when the detector itself wanted
        to convert and the model refused, that refusal is the context model's
        job and is left alone. This layer only adds recall where the detector
        abstained, which is exactly the population no dictionary covers.
        """

        prediction = result.prediction
        if (result.decision.should_convert or baseline.should_convert or self.ortho is None
                or prediction is None or prediction.action == "wait"):
            return result
        licensed = self._orthotactic(baseline, alternative, target_group, field)
        if licensed is None:
            return result
        return ContextResult(licensed, prediction, field, decision_source="ortho_model",
                             fallback_reason="orthotactic_licence")

    def _orthotactic(self, baseline: DetectionDecision, alternative: str,
                     target_group: int, field: FieldContext) -> DetectionDecision | None:
        """Score the physical keys under both languages and license a conversion."""

        model = self.ortho
        if baseline.source_score.known:
            # The dictionary of the layout in use knows this token. The word
            # models hold stronger evidence than any character model can, and
            # their refusal stands: `руку` is a Russian word whose keys spell
            # the English word `here`.
            return None
        if model is None or baseline.source_group not in (0, 1):
            return None
        source_script = "en" if baseline.source_group == 0 else "ru"
        # Key space is the US rendering, so it is whichever side is Latin.
        keys = baseline.original if source_script == "en" else alternative
        if len(keys) < model.minimum_length:
            return None
        if not word_shaped(baseline.original):
            # Punctuation caught between letters is not a word in the layout
            # being typed, and a character model trained on words cannot judge
            # it: `и"ю` is a quotation mark between two letters, not Russian.
            return None
        shape = shape_of(baseline.original, not field.before.strip())
        score = model.score(OrthoEvidence(keys, shape, source_script))
        if not score.supported or score.total <= model.thresholds[source_script]:
            return None
        return replace(
            baseline, should_convert=True, replacement=alternative, target_group=target_group,
            reason="последовательность клавиш не соответствует языку", confidence=score.total,
        )
