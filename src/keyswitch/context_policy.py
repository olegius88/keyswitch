"""Contextual policy orchestration, independent of keyboard injection."""

from __future__ import annotations

import re
from collections.abc import Callable
from dataclasses import dataclass, replace
from typing import ClassVar, Final

from .context_action_features import alnum_question, letter_question, resume_question, variable_question
from .context_model import (
    AfterOrigin, BeforeOrigin, ContextEvidence, ContextModel, ContextPrediction, one_typo_from_word, term_bucket,
)
from .identifier_lexicon import IdentifierLexicon
from .detector import DetectionDecision, LanguageDetector, LanguageScorer
from .input_context import FieldContext, FieldReader, InputContext
from .language_model import LanguageModel
from .ortho_model import OrthoEvidence, OrthoModel, shape_of
from .short_words import ISOLATED_SHORT_WORD_REASON, is_short_word_override, opens_sentences
from .word_decision import NOT_A_WORD_REASON
from .constants.detection import MINIMUM_SHAPED_TOKEN_CHARACTERS, QUOTED_WORD_CLOSING_SIGNS, WORD_QUOTES
from .constants.models import CONTEXT_ACTION_FEATURE_VERSION, RUSSIAN_SLANG_MIN_TERM_BUCKET
from .constants.text import FIELD_CARET_LAG_MAX_CHARACTERS, FIELD_PENDING_MAX_CHARACTERS

# A curated one-letter Russian word typed in the English layout right after an English term inside
# a Russian phrase (KeySwitchEngine._stranded_letter): an explicit rule like the message-start one.
STRANDED_SHORT_WORD_REASON: Final = "однобуквенное слово из безопасного списка после английского термина в русской фразе"
# A lone letter the action model alone would convert: it has never seen one (ContextPolicy.decide).
LONE_LETTER_REASON: Final = "одиночную букву переводят только правила из безопасного списка"
# A Russian abbreviation or slang word the action model would convert into Latin keys nobody uses.
RUSSIAN_SLANG_REASON: Final = "русское сокращение или сленг: латинское прочтение не встречается"
# A word the action model only suggests converting while the detector converts it (ContextPolicy.decide).
SUGGESTION_CONFIRMED_REASON: Final = "контекстная модель предлагает другую раскладку, детектор её подтверждает"


def russian_slang(original: str, alternative: str, source_group: int, source_known: bool) -> bool:
    """A Russian abbreviation or slang word typed as intended: Russian text uses it, nothing uses its Latin keys.

    `пдф` and `впн` are in no lexicon, so they read as gibberish both ways and the language models
    lean to the Latin reading; the action model learned the term tables (context_action_features)
    but on the owner's typing still turned them into `gla` and `dgy` (field logs, 03.10.2026). The
    tables (context-term-frequency.json) say what decides: the Cyrillic form occurs inside Russian
    technical text again and again, and its Latin keys occur neither there nor in English text.
    A form whose Latin keys are a term (`тз` and `np`) is the model's to decide.
    """

    bucket = term_bucket(original, "cyrillic")
    return (
        source_group == 1 and not source_known and bucket.isdigit() and int(bucket) >= RUSSIAN_SLANG_MIN_TERM_BUCKET
        and term_bucket(alternative, "latin") == "0" and term_bucket(alternative, "english") == "0"
    )


@dataclass(frozen=True)
class ContextResult:
    decision: DetectionDecision
    prediction: ContextPrediction | None = None
    field: FieldContext | None = None
    policy_applied: bool = False
    decision_source: str = "baseline"
    fallback_reason: str = ""
    # Characters the field reader showed right of the caret; the model is not given them.
    withheld_after_characters: int = 0


INNER_MARKS: Final[str] = "-'’"


def quoted_or_scoped(replacement: str, original: str) -> bool:
    """A Latin reading that is a quoted word or parts joined by colons, the shapes of code and of a cited word.

    A quote opens it (WORD_QUOTES) and may close it, with one sign after the closing quote (`"legacy",`); the parts
    between dots, or between colons (`std::swap`, `x:Name`), are letters. A colon is a part's edge only when the token
    was typed with a small letter: the `Ж` of a Russian word in capitals is a colon too (`ОЖОГ` is `J:JU`).
    """

    core = replacement
    if core[:1] in WORD_QUOTES:
        quote, core = core[0], core[1:]
        if core[-1:] in QUOTED_WORD_CLOSING_SIGNS and core[:-1].endswith(quote):
            core = core[:-1]
        core = core.removesuffix(quote)
    separators = r"\." if original.isupper() else r"::?|\."
    return bool(core) and all(part.isalpha() for part in re.split(separators, core))


def word_shaped(token: str) -> bool:
    """Is this a word in the layout it was typed in, or a fragment?

    A character model trained on words can only speak about words. `и"ю` is a
    quotation mark caught between two letters, not Russian written badly, and
    left in the population such fragments set every threshold. A hyphen or an
    apostrophe inside a token is different: the engine joins those to the word
    before it tests for a boundary at all, so `Я-то`, `из-за` and `don't` reach
    a model whole and are ordinary words of their language.
    """

    if len(token) < MINIMUM_SHAPED_TOKEN_CHARACTERS or not token[0].isalpha() or not token[-1].isalpha():
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


def _one_typo_from_word(text: str, scorer: LanguageScorer) -> bool:
    """The typo check reads a frequency lexicon; a scorer without one never reports a typo."""

    return isinstance(scorer, LanguageModel) and one_typo_from_word(text, scorer)


def ends_with_typed(text: str, typed: str) -> bool:
    """Whether ``text`` ends with what was ``typed``, its first letter in either case.

    An editor may capitalise the first word of a sentence as it is finished, as macOS automatic
    capitalisation does. The letters are still the ones the user typed, so the word is the one
    the observer saw: TextEdit showed `Ghbdtn` where `ghbdtn` was typed, and the field checks
    refused every first word of a sentence (02.10.2026).
    """

    if text.endswith(typed):
        return True
    tail = text[len(text) - len(typed):] if len(typed) <= len(text) else ""
    return bool(typed) and len(tail) == len(typed) and tail[1:] == typed[1:] and (
        tail[:1].casefold() == typed[:1].casefold())


def shown_ahead(before: str, typed: str, ahead: str) -> int:
    """How many characters typed after ``typed`` a field already shows past it, from two on; 0 if none.

    ``ahead`` is the boundary and the keys the engine has queued behind it: the window takes a key
    in as soon as the hook lets it through, while the engine decides on the word a moment later. One
    character after the word is the boundary (ends_with_typed on ``before[:-1]``); more, and every
    such word was refused as typed into a changed field: `e 'njuj ` typed at 40 ms a key into a
    GTK window of KDE Plasma kept its `e` in five of ten runs (27.10.2026).
    """

    for shown in range(len(ahead), 1, -1):
        if before.endswith(ahead[:shown]) and ends_with_typed(before[:-shown], typed):
            return shown
    return 0


def typed_short_of(before: str, after: str, typed: str) -> str | None:
    """What a field shows of ``typed`` before its caret when it has not taken the last letters in yet, or None.

    A browser or an Electron editor reports its text a moment after the key: up to FIELD_PENDING_MAX_CHARACTERS of
    the last letters may be missing, and at least one letter is there. Missing, not shifted: letters standing right
    after the caret are a caret reported short of them (caret_lag), and further off than it allows, a changed field.
    The early switch reads a word so (engine._typed_in_field), and so does the decision at its boundary.
    """

    for pending in range(1, min(FIELD_PENDING_MAX_CHARACTERS, len(typed) - 1) + 1):
        shown = typed[:-pending]
        if before.endswith(shown) and not after.startswith(typed[-pending:]):
            return shown
    return None


def caret_lag(before: str, after: str, typed: str, *, exact: bool = False) -> int | None:
    """How many characters a field reports its caret short of the end of ``typed``, or None.

    A native snapshot normally ends its text before the caret with what was typed, the boundary
    after it or not yet (ends_with_typed). VS Code Insiders (06.10.2026) reported the caret of its
    chat box one character early: the last typed letter stood after the caret, a word just begun
    read as typed into another one, and every word was refused as a changed field although the
    text was exactly the one typed, as the manual conversion of the same words showed. Up to
    FIELD_CARET_LAG_MAX_CHARACTERS characters after the reported caret are taken as typed before
    it when that is what makes the text end with the typed word; text that does not hold it
    there is still a changed field. `exact` asks for the text to end with ``typed`` itself, the
    boundary already part of it and nothing after it.
    """

    for lag in range(min(FIELD_CARET_LAG_MAX_CHARACTERS, len(after)) + 1):
        head = before + after[:lag]
        if ends_with_typed(head, typed) or (not exact and ends_with_typed(head[:-1], typed)):
            return lag
    return None


def evidence_for_decision(
    baseline: DetectionDecision, alternative: str, target_group: int,
    detector: LanguageDetector, field: FieldContext, trigger: str,
    *, literal_tail: str = "", boundary_text: str = "", ortho: OrthoModel | None = None,
    after_origin: AfterOrigin = "none", identifiers: IdentifierLexicon | None = None,
    inside: bool = False, before_origin: BeforeOrigin = "field",
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
        source_typo=_one_typo_from_word(baseline.original, detector.models[baseline.source_group]),
        target_typo=_one_typo_from_word(alternative, detector.models[target_group]),
        source_opening=opens_sentences(baseline.original, baseline.source_group),
        target_opening=opens_sentences(alternative, target_group),
        inside=inside, before_origin=before_origin,
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
        if self.model is not None:
            # Every question to the model asks whether a reading is a known identifier, and scores it with the
            # orthotactic model, which binds the engine's dictionaries at its first score: finding them
            # normalises the Russian supplement once more (0.6 s, its sort holding the interpreter). Both are
            # done here, before the keyboard hook starts, and not on the first word typed (ContextModel.load).
            shared_identifiers()
            if self.ortho is not None:
                self.ortho.score(OrthoEvidence("ghbdtn", "lower", "en"))
        self.reader = reader

    def decide(
        self, baseline: DetectionDecision, alternative: str, target_group: int,
        detector: LanguageDetector, trigger: str, mode: str,
        *, after: str = "", read_field: bool = False,
        field_override: FieldContext | None = None,
        literal_tail: str = "", boundary_text: str = "",
        after_origin: AfterOrigin = "none", inside: bool = False, before_origin: BeforeOrigin = "field",
        typed_ahead: Callable[[], str] | None = None,
    ) -> ContextResult:
        if mode not in {"assist", "shadow"} or self.model is None:
            return ContextResult(baseline, fallback_reason="mode_disabled" if mode not in {"assist", "shadow"} else "model_unavailable")
        original = baseline.original
        anchor = original + literal_tail
        field = field_override or self.stream.snapshot(anchor)
        ignored_read = ""
        if read_field and self.reader is not None:
            snapshot = self.reader.read(field.application, self.stream.window)
            if snapshot is not None and snapshot.application == field.application:
                snapshot = snapshot.bounded()
                # A native snapshot includes the current word and possibly a
                # boundary. Only use it if anchored to this exact suffix.
                if snapshot.sensitive or snapshot.selection:
                    return ContextResult(replace(baseline, should_convert=False, reason="защищённое поле или выделение"), field=snapshot, decision_source="safety", fallback_reason="sensitive_or_selected_field")
                if snapshot.input_buffer_only(anchor):
                    # Not the text the word went into: the observed keys stand, as for a field
                    # that cannot be read at all.
                    ignored_read = "field_input_buffer"
                else:
                    lag = caret_lag(snapshot.before, snapshot.after, anchor)
                    if lag:
                        snapshot = replace(snapshot, before=snapshot.before + snapshot.after[:lag], after=snapshot.after[lag:])
                    before = snapshot.before
                    if anchor and ends_with_typed(before, anchor):
                        field = replace(snapshot, before=before[:-len(anchor)])
                    elif anchor and ends_with_typed(before[:-1], anchor):
                        field = replace(snapshot, before=before[:-len(anchor) - 1])
                    elif anchor and typed_ahead is not None and (
                            ahead := shown_ahead(before, anchor, boundary_text + typed_ahead())):
                        # Asked after the read: every key the field shows was queued before the window got it.
                        field = replace(snapshot, before=before[:-len(anchor) - ahead])
                    elif anchor and (shown := typed_short_of(before, snapshot.after, anchor)) is not None:
                        # The editor has not shown the last letters yet: the word is the one typed and is decided on
                        # its keys, and its correction waits for the field to show it (engine._waits_for_field).
                        # Refused as a changed field, `ашкуащч` stayed for `firefox` in Antigravity (the owner's
                        # typing, 19.10.2026), and a word in such an editor was never put to the model at its space.
                        field = replace(snapshot, before=before[:-len(shown)])
                        ignored_read = "field_behind"
                    else:
                        # The editor contradicts the observer: do not fall back
                        # to stale strokes and erase a different span of text.
                        return ContextResult(replace(baseline, should_convert=False, reason="текст активного поля изменился"), field=snapshot, decision_source="safety", fallback_reason="field_changed")
        # The text right of the caret is the model's evidence only when the engine knows where it
        # came from: the neighbour it is about to convert or the word it kept. What a field reader
        # shows there is not given to the model. The certified protocol never reads the field,
        # and the corpus has right text only as a sentence's own continuation, so the model took
        # its script for the word's language: `мне` typed in front of English text already in a
        # VS Code chat box became `vyt` at p=0.9998, and `vyt` stayed (the owner's laptop, 0.38.1
        # and 0.52.0: 7 false conversions and 6 missed words in 102 such words, 12.10.2026). The
        # owner's PC showed no gain from the text in 470 words read with it.
        withheld = len(field.after)
        field = replace(field, after=after)
        evidence = evidence_for_decision(
            baseline, alternative, target_group, detector, field, trigger,
            literal_tail=literal_tail, boundary_text=boundary_text,
            ortho=self.ortho if self.model.feature_version == CONTEXT_ACTION_FEATURE_VERSION else None,
            after_origin=after_origin, inside=inside, before_origin=before_origin,
        )
        source, target = evidence.source_score, evidence.target_score
        assert source is not None and target is not None
        prediction = self.model.predict(evidence)
        if mode == "shadow":
            return ContextResult(baseline, prediction, field, fallback_reason="shadow_mode", withheld_after_characters=withheld)
        # The model's verdict stands whatever its support flag says. The flag only
        # tells whether some weight names this application or these exact
        # neighbour words; the model knows a handful of each, so in Firefox, Edge
        # or a terminal a confident verdict used to be thrown away for the
        # baseline - `tot привет` stayed (0.31.x logs, 24.09.2026) - while the
        # trainer evaluates and certifies the model on its own verdict, with no
        # such fallback. The flag stays in the log as a diagnostic.
        # A lone letter is outside what the action model learned: every single-letter row of its
        # corpus sits in quarantine, so its verdict on one is no opinion, and on its own it turned
        # `ч` into `x` and `ы` into `s` at p=0.99 (field logs, 03.10.2026). It converts a lone
        # letter only together with a curated rule.
        # The same holds for a single letter with digits (`1С`, `а1`): a product name or a cell, and in
        # Russian prose the model turned `1С` into `1C`.
        # A model with the single-letter head has learned the letter right after a Latin word
        # (context_action_features.letter_question): there its verdict on a letter is an opinion, and
        # `nats b redis` is `nats и redis` (the owner's typing, 0.32-0.41). One with the variable head has
        # learned a letter of the Russian layout after a word of Cyrillic letters (variable_question): a Latin
        # variable typed there (`ч` for `x`) or a letter meant as written (`да я про то ч`). One with the
        # letters-and-digits head has learned a token of Russian letters and digits whose keys are Latin letters and
        # digits, after Russian text (alnum_question): `м67` for `v67` and `ч64` for `x64`, and `1с` or `3д` meant as typed.
        # One with the resume head has learned a short word right after Latin text asked again once its next word is
        # planned for conversion (resume_question): `а` typed `f` in `seek() , f там`, and a Latin `x` before Russian text.
        action_model = self.model.feature_version == CONTEXT_ACTION_FEATURE_VERSION
        learned = ((self.model.answers_letters and letter_question(baseline.original, alternative, field.before))
                   or (self.model.answers_variables
                       and variable_question(baseline.original, alternative, field.before, trigger))
                   or (self.model.answers_alnum and alnum_question(baseline.original, alternative, field.before))
                   or (self.model.answers_resume
                       and resume_question(baseline.original, alternative, field.before, field.after, after_origin)))
        lone = (action_model and sum(char.isalpha() for char in baseline.original) == 1
                and all(char.isalpha() or char.isdigit() for char in baseline.original) and not learned)
        slang = action_model and russian_slang(baseline.original, alternative, baseline.source_group, source.known)
        if prediction.action == "convert" and not slang and (not lone or is_short_word_override(baseline)):
            decision = replace(
                baseline, should_convert=True, replacement=alternative,
                target_group=target_group, source_score=source, target_score=target,
                reason="решение контекстной модели", confidence=prediction.probability,
            )
        elif prediction.action == "suggest" and baseline.should_convert and action_model and not slang and not lone:
            # The model leans to the other layout without its threshold's certainty, and the detector,
            # which the model was asked to overrule, converts: two verdicts of the same direction. A word
            # no lexicon holds (`byzpt` for `инязе`, `ntby` for `теин`) was left as typed at a suggestion
            # while the baseline pair converted it, and in the owner's typing (0.44-0.53) such words were
            # meant converted in 6 of the 7 cases of more than one letter. Measured on the owner's field
            # batteries: 15 more words converted right, 20 fewer missed, no more converted wrongly; the
            # development and calibration sequences lost no restoration and corrupted nothing. One live word
            # of the seven was meant as typed (`lokim`). A curated short word converts on a suggestion too: left
            # to wait for its neighbour, `b[` after `the.hoster` stayed where `их` was meant. The model's wait
            # still delays it below.
            decision = replace(
                baseline, should_convert=True, replacement=alternative,
                target_group=target_group, source_score=source, target_score=target,
                reason=SUGGESTION_CONFIRMED_REASON, confidence=prediction.probability,
            )
        elif ((is_short_word_override(baseline)
               and (baseline.reason == ISOLATED_SHORT_WORD_REASON or trigger == "pause"
                    or (prediction.action != "wait" and self.model.feature_version != CONTEXT_ACTION_FEATURE_VERSION)))
              or baseline.reason == STRANDED_SHORT_WORD_REASON):
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
            # a name, visible in the model card and this comment. The second named
            # exception is the same letter right after an English term inside a
            # Russian phrase (`поправь env b`): the term switched the layout,
            # and the model has no single-letter row to know the letter by.
            return ContextResult(baseline, prediction, field, decision_source="short_word_override",
                                 fallback_reason="trusted_short_word", withheld_after_characters=withheld)
        else:
            refusal = RUSSIAN_SLANG_REASON if slang else LONE_LETTER_REASON
            decision = replace(baseline, should_convert=False, reason=refusal if prediction.action == "convert" else {
                "keep": "контекстная модель оставляет текст",
                "wait": "контекстная модель ждёт продолжения",
                "suggest": "контекстная модель предлагает проверить раскладку",
            }[prediction.action])
        result = ContextResult(decision, prediction, field, policy_applied=True,
                               decision_source="context_model", fallback_reason=ignored_read)
        if self.model.feature_version != CONTEXT_ACTION_FEATURE_VERSION or prediction.action == "suggest":
            # A feature-version-3 model decides alone, but when it only suggests the other layout, the
            # orthotactic model may settle it: the keys of a word no lexicon holds (`byzpt` for `инязе`,
            # `ntby` for `теин`) were left as typed at a suggestion, while the baseline pair converted them
            # on that licence. Its keep and wait stand.
            result = self._licensed(result, baseline, alternative, target_group, field, detector)
        return replace(self._spelling_a_word(result), withheld_after_characters=withheld)

    @staticmethod
    def _spelling_a_word(result: ContextResult) -> ContextResult:
        """A word is never replaced by something that is not one, whichever layer asked.

        Six Russian letters sit on punctuation keys, so the Latin reading of a
        Russian abbreviation such as ``збс`` is ``p,c``. The detector's own
        conversions pass :func:`word_shape_veto` in ``automatic_word_decision``;
        a conversion the model or the orthotactic licence asked for did not, and
        the shipped model asked for exactly that, with p=0.997, in every
        application it knows (found 20.09.2026 while scoring a candidate). The
        model is, however, trained to restore what the detector is not asked
        about: a dotfile (``ювшые`` is ``.dist``), a contraction (``вщтэе`` is
        ``don't``) and a dotted name - a domain, an address or a file
        (``учфьздуюсщь`` is ``example.com``) - so a leading dot, one inner
        apostrophe and letters joined by single dots are the shapes a Latin
        reading may keep. Until 04.10.2026 a dot inside was refused as well (`дюп`
        is ``l.g``), and on the owner's own typing that left every domain and
        address typed in the Russian layout as it was, with the model sure of each
        at p=1.0; the model of 0.38.0 keeps `дюп` itself. A quoted token and parts
        joined by colons are shapes too (quoted_or_scoped): `"Boot` typed in the
        Russian layout is `ЭИщще`, `std::swap` is `ыевЖЖыцфз`, and the model converts
        them, most at p=1.0, which this refusal overruled in 146 messages of the
        developer-chat development set (19.10.2026); a sign after a word without a
        quote stays refused, since `б` and `ю` end Russian words (`учеб` is `ext,`).
        The refusal is recorded as a safety decision, not as the model's opinion.
        """

        decision = result.decision
        if not decision.should_convert or not decision.original.isalpha():
            return result
        body = decision.replacement.removeprefix(".")
        if (body.isalpha() or word_shaped(body) or all(part.isalpha() for part in body.split("."))
                or quoted_or_scoped(body, decision.original)):
            return result
        vetoed = replace(decision, should_convert=False, reason=NOT_A_WORD_REASON)
        return ContextResult(vetoed, result.prediction, result.field, policy_applied=result.policy_applied,
                             decision_source="safety", fallback_reason="replacement_not_a_word")

    def _licensed(self, result: ContextResult, baseline: DetectionDecision, alternative: str,
                  target_group: int, field: FieldContext,
                  detector: LanguageDetector | None = None) -> ContextResult:
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
        licensed = self._orthotactic(baseline, alternative, target_group, field, detector)
        if licensed is None:
            return result
        return ContextResult(licensed, prediction, field, decision_source="ortho_model",
                             fallback_reason="orthotactic_licence")

    def _orthotactic(self, baseline: DetectionDecision, alternative: str,
                     target_group: int, field: FieldContext,
                     detector: LanguageDetector | None = None) -> DetectionDecision | None:
        """Score the physical keys under both languages and license a conversion.

        The features of a schema 2 artifact ask dictionaries about parts of the token; the
        detector's own models answer, so the model never loads a second copy of them.
        """

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
        known = None if detector is None else (
            lambda script, word: detector.models[0 if script == "en" else 1].score(word).known)
        score = model.score(OrthoEvidence(keys, shape, source_script), known=known)
        if not score.supported or score.total <= model.thresholds[source_script]:
            return None
        return replace(
            baseline, should_convert=True, replacement=alternative, target_group=target_group,
            reason="последовательность клавиш не соответствует языку", confidence=score.total,
        )
