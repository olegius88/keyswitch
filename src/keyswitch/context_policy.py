"""Contextual policy orchestration, independent of keyboard injection."""

from __future__ import annotations

from dataclasses import dataclass, replace
from typing import ClassVar, Final

from .context_model import FEATURE_VERSION, AfterOrigin, ContextEvidence, ContextModel, ContextPrediction
from .identifier_lexicon import IdentifierLexicon
from .detector import DetectionDecision, LanguageDetector
from .input_context import FieldContext, FieldReader, InputContext
from .ortho_model import OrthoEvidence, OrthoModel, shape_of
from .short_words import ISOLATED_SHORT_WORD_REASON, is_short_word_override
from .word_decision import NOT_A_WORD_REASON


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


_SHARED_IDENTIFIERS: tuple[IdentifierLexicon | None, str] | None = None


def shared_identifiers() -> IdentifierLexicon | None:
    """One packaged identifier lexicon for the engine, the trainer and the evaluator."""

    global _SHARED_IDENTIFIERS
    if _SHARED_IDENTIFIERS is None:
        _SHARED_IDENTIFIERS = IdentifierLexicon.try_load()
    return _SHARED_IDENTIFIERS[0]


def evidence_for_decision(
    baseline: DetectionDecision, alternative: str, target_group: int,
    detector: LanguageDetector, field: FieldContext, trigger: str,
    *, literal_tail: str = "", boundary_text: str = "", ortho: OrthoModel | None = None,
    after_origin: AfterOrigin = "none", identifiers: IdentifierLexicon | None = None,
) -> ContextEvidence:
    """Build the same lexical and optional orthotactic evidence for any caller."""

    source = detector.models[baseline.source_group].score(baseline.original)
    target = detector.models[target_group].score(alternative)
    lexicon = shared_identifiers() if identifiers is None else identifiers
    source_identifier = lexicon is not None and lexicon.contains(baseline.original)
    target_identifier = lexicon is not None and lexicon.contains(alternative)
    ortho_score: float | None = None
    ortho_threshold: float | None = None
    if ortho is not None and baseline.source_group in (0, 1):
        script = "en" if baseline.source_group == 0 else "ru"
        keys = baseline.original if baseline.source_group == 0 else alternative
        scored = ortho.score(OrthoEvidence(keys, shape_of(baseline.original, not field.before.strip()), script))
        if scored.supported:
            ortho_score = scored.total
            ortho_threshold = ortho.thresholds[script]
    return ContextEvidence(
        baseline.original, alternative, baseline.source_group, field, trigger,
        baseline.should_convert, source.known, target.known, target.value - source.value,
        source, target, baseline.model_probability, baseline.model_threshold,
        literal_tail, ortho_score, ortho_threshold, boundary_text, after_origin,
        source_identifier=source_identifier, target_identifier=target_identifier,
    )


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
        literal_tail: str = "", boundary_text: str = "",
        after_origin: AfterOrigin = "none",
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
        evidence = evidence_for_decision(
            baseline, alternative, target_group, detector, field, trigger,
            literal_tail=literal_tail, boundary_text=boundary_text,
            ortho=self.ortho if self.model.feature_version == 3 else None,
            after_origin=after_origin,
        )
        source, target = evidence.source_score, evidence.target_score
        assert source is not None and target is not None
        prediction = self.model.predict(evidence)
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
        elif (is_short_word_override(baseline)
              and (baseline.reason == ISOLATED_SHORT_WORD_REASON or trigger == "pause"
                   or (prediction.action != "wait" and self.model.feature_version == FEATURE_VERSION))):
            # A curated, reviewed exception is an explicit rule, not a guess, so
            # neither a probabilistic `keep` nor an under-confident `convert`
            # cancels it. `wait` still delays it at a boundary, because that is
            # about timing rather than direction and the lookahead may resolve the
            # word jointly - thirty-six of the thirty-seven curated entries are
            # Russian function words, exactly the words a phrase decides.
            # On the pause trigger the timing has already happened: the user
            # stopped, no next word is coming, and a delay with nothing left to
            # wait for is a refusal wearing another name. Letting `wait` lose
            # everywhere was measured on 18.09.2026 and took the lookahead with
            # it. The model's opinion is recorded either way.
            # A feature-version-3 model is the arbiter of the other curated
            # rules. A lone letter opening a message is the one named exception,
            # and it beats a "wait" as well: the model cannot have an opinion here,
            # because every single-letter row of the corpus sits in quarantine -
            # their families are held by other frozen evidence - so it has never
            # seen one. Waiting for a next word that may never come would leave
            # `z ` standing in a sent message. The rule itself is measured: a
            # Russian utterance opens with one of а/и/с/в/к/у/о/я once in seven
            # sentences, an English one never opens with a lone f/b/c/d/r/e/j/z
            # (UD Taiga and UD EWT, 17.09.2026). Teaching this to the model needs
            # a corpus that keeps those rows; until then it is an exception with
            # a name, visible in the model card and this comment.
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
        if self.model.feature_version != 3:
            result = self._licensed(result, baseline, alternative, target_group, field)
        return self._spelling_a_word(result)

    @staticmethod
    def _spelling_a_word(result: ContextResult) -> ContextResult:
        """A word is never replaced by something that is not one, whichever layer asked.

        Six Russian letters sit on punctuation keys, so the Latin reading of a
        Russian abbreviation such as ``збс`` is ``p,c`` and of ``дюп`` it is
        ``l.g``. The detector's own conversions pass :func:`word_shape_veto` in
        ``automatic_word_decision``; a conversion the model or the orthotactic
        licence asked for did not, and the shipped model asked for exactly those,
        with p=0.997, in every application it knows (found 20.09.2026 while
        scoring a candidate). The model is, however, trained to restore what the
        detector is not asked about: a dotfile (``ювшые`` is ``.dist``) and a
        contraction (``вщтэе`` is ``don't``), so a leading dot and one inner
        apostrophe are the two shapes a Latin reading may keep; a name with a
        second dot, such as ``.env.local``, is the accepted cost. The refusal is
        recorded as a safety decision, not as the model's opinion.
        """

        decision = result.decision
        if not decision.should_convert or not decision.original.isalpha():
            return result
        body = decision.replacement.removeprefix(".")
        if body.isalpha() or word_shaped(body):
            return result
        vetoed = replace(decision, should_convert=False, reason=NOT_A_WORD_REASON)
        return ContextResult(vetoed, result.prediction, result.field, policy_applied=result.policy_applied,
                             decision_source="safety", fallback_reason="replacement_not_a_word")

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
