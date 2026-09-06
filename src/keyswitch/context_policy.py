"""Contextual policy orchestration, independent of keyboard injection."""

from __future__ import annotations

from dataclasses import dataclass, replace

from .context_model import ContextEvidence, ContextModel, ContextPrediction
from .detector import DetectionDecision, LanguageDetector
from .input_context import FieldContext, FieldReader, InputContext


@dataclass(frozen=True)
class ContextResult:
    decision: DetectionDecision
    prediction: ContextPrediction | None = None
    field: FieldContext | None = None
    policy_applied: bool = False
    decision_source: str = "baseline"
    fallback_reason: str = ""


class ContextPolicy:
    def __init__(self, reader: FieldReader | None = None) -> None:
        self.stream = InputContext()
        self.model, self.status = ContextModel.try_load()
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
        else:
            decision = replace(baseline, should_convert=False, reason={
                "keep": "контекстная модель оставляет текст",
                "wait": "контекстная модель ждёт продолжения",
                "suggest": "контекстная модель предлагает проверить раскладку",
            }[prediction.action])
        return ContextResult(decision, prediction, field, policy_applied=True, decision_source="context_model")
