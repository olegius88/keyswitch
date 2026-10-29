"""Bounded experimental action features shared by training and inference."""

from __future__ import annotations

import bisect
import functools
import itertools
import math
import re
import unicodedata
from typing import TYPE_CHECKING, Final
from .constants.training import TERM_FRAGMENT_MIN_ENDING_BUCKET
from .constants.models import (
    ABBREVIATION_FEATURE_PREFIX,
    ABBREVIATION_NON_INITIAL_LETTERS,
    ALNUM_FEATURE_PREFIX,
    ALNUM_HEAD_MAX_CHARACTERS,
    ALNUM_RATIO_BOUND,
    ALNUM_RUN_LENGTH_MAX,
    ALNUM_SHAPE_MAX_RUNS,
    ACTION_FEATURE_AFTER_CONTEXT_CHARACTERS,
    ACTION_FEATURE_APPLICATION_NAME_CHARACTERS,
    ACTION_FEATURE_APPLICATION_TOKEN_COUNT,
    ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS,
    ACTION_FEATURE_CAPITALS_MIN_LETTERS,
    ACTION_FEATURE_CHARACTER_WEIGHT_CAP,
    ACTION_FEATURE_FIELD_LABEL_MAX_CHARACTERS,
    ACTION_FEATURE_FREQUENCY_CAP,
    ACTION_FEATURE_LENGTH_BUCKET_MAX_CHARACTERS,
    ACTION_FEATURE_NEIGHBOUR_WORD_COUNT,
    ACTION_FEATURE_NEIGHBOUR_WORD_MAX_CHARACTERS,
    ACTION_FEATURE_NGRAM_ORDERS,
    ACTION_FEATURE_ORTHO_SCORE_BOUND,
    ACTION_FEATURE_RAW_NGRAM_SCORE_BOUND,
    ACTION_FEATURE_SHORT_TEXT_CHARACTERS,
    ACTION_FEATURE_WHITESPACE_MAX_CHARACTERS,
    ACTION_FEATURE_WORD_MAX_CHARACTERS,
    ACTION_FEATURE_WORD_SCORE_BOUND,
    ALONE_COUNT_RATIO_BOUND,
    ALONE_FEATURE_PREFIX,
    ALONE_HEAD_LETTERS,
    BRIEF_FEATURE_PREFIX,
    BRIEF_HEAD_LETTERS,
    BRIEF_RATIO_BOUND,
    BRIEF_SETTLED_DOMINANCE,
    BRIEF_SETTLED_MIN_COUNT,
    BRIEF_SETTLED_OTHER_MAXIMUM,
    CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS,
    DOTTED_FEATURE_PREFIX,
    DOTTED_PART_MAX_LETTERS,
    FIRST_FEATURE_PREFIX,
    HYPOTHESIS_FEATURE_PREFIX,
    HYPOTHESIS_MAGNITUDE_MAX,
    HYPOTHESIS_RATIO_BOUND,
    CAPITALS_FEATURE_PREFIX,
    CAPITALS_HEAD_MAX_LETTERS,
    CAPITALS_HEAD_MIN_LETTERS,
    JOINED_FEATURE_PREFIX,
    KEPT_CONTEXT_WORD_MAX_CHARACTERS,
    KEPT_FEATURE_PREFIX,
    LATIN_ABBREVIATION_FEATURE_PREFIX,
    LETTER_FEATURE_PREFIX,
    OPENING_SHORT_MAX_LETTERS,
    PLANNED_CONTEXT_AFTER_MAX_CHARACTERS,
    PLANNED_CONTEXT_WORD_MAX_CHARACTERS,
    RESUME_FEATURE_PREFIX,
    RESUME_LEADING_SIGNS,
    START_DELTA_BAND_WIDTH,
    START_DELTA_BANDS,
    START_FEATURE_PREFIX,
    START_HEAD_MIN_LETTERS,
    SHORT_FEATURE_PREFIX,
    SHORT_HEAD_LETTERS,
    SLIP_BEFORE_MAGNITUDE_STEP,
    SLIP_FEATURE_PREFIX,
    SLIP_HEAD_MAX_LATIN_BUCKET,
    SLIP_HEAD_MIN_LETTERS,
    SLIP_ORTHO_BAND_WIDTH,
    SLIP_ORTHO_BANDS,
    SLIP_PROBABILITY_BANDS,
    SHORT_HEAD_MAX_LATIN_BUCKET,
    SHORT_HEAD_MIN_RUSSIAN_BUCKET,
    SHORT_MAGNITUDE_MAX,
    SHORT_RATIO_BOUND,
    SIGN_DASH,
    SIGN_FEATURE_PREFIX,
    SIGN_OPERATOR_SIGNS,
    SIGN_TOKEN_MAX_CHARACTERS,
    TERM_FEATURE_PREFIX,
    VARIABLE_CODE_CONTEXT_CHARACTERS,
    VARIABLE_CODE_SIGNS,
    VARIABLE_FEATURE_PREFIX,
    VARIABLE_HEAD_TRIGGERS,
    TERM_HEAD_ENDING_BUCKET_MAX,
    TERM_HEAD_ENDING_LETTERS,
    TERM_HEAD_LEADING_SIGN,
    TERM_HEAD_MIN_LATIN_BUCKET,
    TERM_HEAD_MIN_LETTERS,
    TERM_HEAD_TRAILING_SIGNS,
    UNKNOWN_FEATURE_PREFIX,
    UNKNOWN_HEAD_MIN_CHARACTERS,
    UNKNOWN_HEAD_SIGN_KEY_LETTERS,
)

if TYPE_CHECKING:
    from .context_model import ContextEvidence
    from .language_model import WordScore


_WORDS = re.compile(r"[^\W\d_]+(?:['’\-][^\W\d_]+)*", re.UNICODE)


def _bounded(value: float, lower: float = -1.0, upper: float = 1.0) -> float:
    if not math.isfinite(value):
        raise ValueError("nonfinite context action evidence")
    return max(lower, min(upper, value))


def _script(text: str) -> str:
    ru = any("а" <= char <= "я" or char == "ё" for char in text)
    en = any("a" <= char <= "z" for char in text)
    return "mixed" if ru and en else "ru" if ru else "en" if en else "none"


def _characters(features: dict[str, float], label: str, text: str, direction: str) -> None:
    if not text:
        return
    padded = "^" + text + "$"
    for order in ACTION_FEATURE_NGRAM_ORDERS:
        count = len(padded) - order + 1
        for index in range(max(0, count)):
            name = f"{label}:char:{direction}:{order}:{padded[index:index + order]}"
            features[name] = min(
                ACTION_FEATURE_CHARACTER_WEIGHT_CAP, features.get(name, 0.0) + 1.0 / math.sqrt(count)
            )


def _word_score(features: dict[str, float], label: str, score: WordScore | None, known: bool) -> None:
    features[f"{label}:known:{int(known)}"] = 1.0
    if score is None:
        features[f"{label}:score_missing"] = 1.0
        return
    if type(score.frequency) is not int or score.frequency < 0:
        raise ValueError("invalid context action frequency")
    features.update({
        f"{label}:value": _bounded(score.value, -ACTION_FEATURE_WORD_SCORE_BOUND, ACTION_FEATURE_WORD_SCORE_BOUND) / ACTION_FEATURE_WORD_SCORE_BOUND,
        f"{label}:gram_ratio": _bounded(score.gram_ratio, 0.0, 1.0),
        f"{label}:ngram_score": (
            _bounded(score.ngram_score, -ACTION_FEATURE_WORD_SCORE_BOUND, ACTION_FEATURE_WORD_SCORE_BOUND) / ACTION_FEATURE_WORD_SCORE_BOUND
        ),
        f"{label}:invalid_ratio": _bounded(score.invalid_ratio, 0.0, 1.0),
        f"{label}:raw_ngram_score": (
            _bounded(score.raw_ngram_score, -ACTION_FEATURE_RAW_NGRAM_SCORE_BOUND, 0.0) / ACTION_FEATURE_RAW_NGRAM_SCORE_BOUND
        ),
        f"{label}:logfrequency": (
            math.log1p(max(0, min(ACTION_FEATURE_FREQUENCY_CAP, score.frequency))) / math.log1p(ACTION_FEATURE_FREQUENCY_CAP)
        ),
        f"{label}:exact:{int(score.exact)}": 1.0,
        f"{label}:spell_known:{int(score.spell_known)}": 1.0,
    })


# A dot between two Latin letters: the Latin reading is a domain or a file name (`sefan.ru`).
_LATIN_DOT: Final = re.compile(r"[A-Za-z]\.[A-Za-z]")


def _dominant(text: str) -> str:
    """The script with more letters in `text`, as the line and context evidence names it."""

    ru = sum("а" <= char <= "я" or char == "ё" for char in text)
    en = sum("a" <= char <= "z" for char in text)
    return "ru" if ru > en else "en" if en > ru else "none"


def own_bucket(item: ContextEvidence, text: str, alphabet: str) -> str:
    """How often a reading of the decided token occurs in one term table (context_model.term_bucket).

    A training frame drawn with its counts dropped (ContextEvidence.term_counts_dropped) reads as uncounted: a term
    no table holds (`webjs`) is what such a frame stands for, and its shape and context must decide it. The counts
    of the words around the token are never dropped.
    """

    from .context_model import term_bucket

    bucket = term_bucket(text, alphabet)
    return "0" if item.term_counts_dropped and bucket != "na" else bucket


def joined_parts(text: str) -> list[str] | None:
    """The words of a token of letters joined by hyphens, its edge signs aside (`ку-ку`, `pre-order`); None otherwise."""

    from .context_model import _TERM_EDGE_SIGNS

    parts = text.strip(_TERM_EDGE_SIGNS).split("-")
    return parts if len(parts) > 1 and all(part.isalpha() for part in parts) else None


def _term_features(features: dict[str, float], item: ContextEvidence, direction: str) -> None:
    """Where the word stands on its line, whether a reading opens sentences, how often each occurs.

    context-v1 schema 7 decided by this evidence (context_model.extract_context_features), and the
    action scheme had none of it. Replayed through the engine, the owner's own typing (field logs
    of 0.26-0.36.3) showed what that cost: 0.37.0 left words in the other layout about twice as
    often as 0.36.3, and nearly all of them were what the term tables tell apart - an English term
    typed in the Russian layout amid Russian prose (`зк` is never seen inside Russian text, `pr`
    often), a common Russian word typed in the English layout after an English term (`dct` is no
    term, `все` the commonest word), a Russian abbreviation that stays (`тз` is frequent in Russian
    text). The tables are the packaged ones schema 7 reads (context-term-frequency.json), counted on
    public text; an artifact trained before these features carries no weight for them and scores as
    it did.
    """

    before = item.field.before[-ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS:].casefold()
    after = item.field.after[:ACTION_FEATURE_AFTER_CONTEXT_CHARACTERS].casefold()
    known = f"known:{int(item.source_known)}:{int(item.target_known)}"
    lines = item.field.before.rsplit("\n", 1)
    state = "same" if any(char.isalpha() for char in lines[-1]) else "newline" if len(lines) > 1 else "empty"
    features[f"line:{state}:direction:{direction}"] = 1.0
    features[f"line:{state}:{known}:direction:{direction}"] = 1.0
    if state == "newline":
        previous = lines[0].rsplit("\n", 1)[-1].casefold()
        features[f"line:newline:previous:{_dominant(previous)}:direction:{direction}"] = 1.0
    if item.source_opening or item.target_opening:
        opening = f"opening:{int(item.source_opening)}:{int(item.target_opening)}"
        alone = int(not before.strip() and not after.strip())
        features[f"{opening}:direction:{direction}"] = 1.0
        features[f"{opening}:alone:{alone}:direction:{direction}"] = 1.0
        features[f"{opening}:line:{state}:direction:{direction}"] = 1.0
    latin = item.original if item.source_group == 0 else item.alternative
    cyrillic = item.alternative if item.source_group == 0 else item.original
    features[f"latin:dot:direction:{direction}"] = float(bool(_LATIN_DOT.search(latin)))
    features[f"latin:underscore:direction:{direction}"] = float("_" in latin.strip("_"))
    cyrillic_known = item.source_known if item.source_group == 1 else item.target_known
    latin_bucket = own_bucket(item, latin, "latin")
    cyrillic_bucket = "lexicon" if cyrillic_known else own_bucket(item, cyrillic, "cyrillic")
    _capitals_features(features, item, direction, before, cyrillic, cyrillic_known)
    if latin_bucket != "na" or cyrillic_bucket != "na":
        _frequency_features(features, item, direction, (before, after, state), (latin, cyrillic),
                            (latin_bucket, cyrillic_bucket), cyrillic_known)
    if not any(char.isalpha() for char in item.original):
        digits = int(any(char.isdigit() for char in item.original))
        kind = "letters" if any(char.isalpha() for char in item.alternative) else "only"
        features[f"signs:{kind}:direction:{direction}"] = 1.0
        features[f"signs:{kind}:digits:{digits}:direction:{direction}"] = 1.0


def _capitals_features(features: dict[str, float], item: ContextEvidence, direction: str, before: str,
                       cyrillic: str, cyrillic_known: bool) -> None:
    """A token of letters only, all capitals, three or more: whether its Cyrillic reading is a known word,
    how often Russian text uses it, and the script of the text before it.

    `WBC` after Russian prose reads `ЦИС`, which the lexicon knows (the OpenSubtitles supplement) and
    Russian text never uses; `CIF` reads `США`, which Russian text uses all the time. Without a feature
    of their own, the frames teaching that such a citation stays moved the weights every Latin token
    after Russian text shares, and short Latin keys of Russian words (`f` for `а`) stayed with them.
    """

    letters = item.original
    if len(letters) < ACTION_FEATURE_CAPITALS_MIN_LETTERS or not letters.isalpha() or not letters.isupper():
        return
    russian = own_bucket(item, cyrillic, "russian")
    features[f"capitals:known:{int(cyrillic_known)}:russian:{russian}:direction:{direction}"] = 1.0
    features[f"capitals:known:{int(cyrillic_known)}:russian:{russian}:before:{_dominant(before)}:direction:{direction}"] = 1.0


def _frequency_features(features: dict[str, float], item: ContextEvidence, direction: str,
                        surroundings: tuple[str, str, str], readings: tuple[str, str], buckets: tuple[str, str],
                        cyrillic_known: bool) -> None:
    """How often each reading occurs inside Russian text and in text of its own language.

    Only for a token at least one of whose readings is a word the tables could count. A token with
    digits or inner signs in both readings has no entry in either, and as one shared bucket such
    tokens all read like the Latin ones with digits the corpus restores (`зь2` is `pm2`): `1С`, `2фа`
    and `а1` in Russian prose became `1C`, `2af` and `f1`. An absent count is no evidence.
    """

    before, after, state = surroundings
    latin, cyrillic = readings
    latin_bucket, cyrillic_bucket = buckets
    planned = item.after_origin == "planned_next_conversion"
    context = f"before:{_dominant(before)}:{'next' if planned else 'after'}:{_dominant(after)}"
    if not item.inside and (before.strip() or after.strip()):
        english_bucket = own_bucket(item, latin, "english")
        russian_bucket = own_bucket(item, cyrillic, "russian")
        if latin_bucket == "0":
            latin_bucket = f"0:{english_bucket}"
        if cyrillic_known:
            cyrillic_bucket = f"lexicon:{russian_bucket}"
        languages = f"freq:english:{english_bucket}:russian:{russian_bucket}"
        features[f"freq:english:{english_bucket}:direction:{direction}"] = 1.0
        features[f"freq:russian:{russian_bucket}:direction:{direction}"] = 1.0
        features[f"{languages}:direction:{direction}"] = 1.0
        features[f"{languages}:{context}:direction:{direction}"] = 1.0
        features[f"{languages}:line:{state}:direction:{direction}"] = 1.0
    frequency = "inside:freq" if item.inside else "freq"
    features[f"{frequency}:latin:{latin_bucket}:direction:{direction}"] = 1.0
    features[f"{frequency}:cyrillic:{cyrillic_bucket}:direction:{direction}"] = 1.0
    features[f"{frequency}:{latin_bucket}:{cyrillic_bucket}:direction:{direction}"] = 1.0
    features[f"{frequency}:{latin_bucket}:{cyrillic_bucket}:{context}:direction:{direction}"] = 1.0


def extract_action_features(item: ContextEvidence) -> dict[str, float]:
    """Describe each reading separately; no token-specific conversion rules."""

    if type(item.source_group) is not int or item.source_group not in (0, 1):
        raise ValueError("invalid context action direction")
    origin = item.after_origin
    if origin not in ("none", "field", "planned_next_conversion", "kept_next_word"):
        raise ValueError("invalid right-context origin")
    if item.before_origin not in ("field", "planned_previous_reading") or (
            item.before_origin == "planned_previous_reading" and origin not in ("none", "field")):
        raise ValueError("invalid left-context origin")
    if origin == "kept_next_word" and (
        not 0 < len(item.original) <= KEPT_CONTEXT_WORD_MAX_CHARACTERS
        or item.trigger != "space" or item.boundary_text != " "
        or not item.field.after or len(item.field.after) > PLANNED_CONTEXT_AFTER_MAX_CHARACTERS
        or any(char.isspace() for char in item.field.after)
    ):
        raise ValueError("unreachable kept right context")
    if origin == "planned_next_conversion" and (
        not 0 < len(item.original) <= PLANNED_CONTEXT_WORD_MAX_CHARACTERS
        or item.trigger != "space" or item.boundary_text != " "
        or not item.field.after or len(item.field.after) > PLANNED_CONTEXT_AFTER_MAX_CHARACTERS
        or any(char.isspace() for char in item.field.after)
    ):
        raise ValueError("unreachable planned right context")
    direction = str(item.source_group)
    length = min(ACTION_FEATURE_LENGTH_BUCKET_MAX_CHARACTERS, max(len(item.original), len(item.alternative)))
    features: dict[str, float] = {
        "bias": 1.0, f"direction:{direction}": 1.0,
        f"baseline:{int(item.baseline_convert)}": 1.0,
        f"length:{length}": 1.0,
        f"baseline:{int(item.baseline_convert)}:length:{length}": 1.0,
        f"known:{int(item.source_known)}:{int(item.target_known)}:length:{length}": 1.0,
        f"identifier:{int(item.source_identifier)}:{int(item.target_identifier)}": 1.0,
        f"role:{item.field.role[:ACTION_FEATURE_FIELD_LABEL_MAX_CHARACTERS]}": 1.0,
        f"trigger:{item.trigger[:ACTION_FEATURE_FIELD_LABEL_MAX_CHARACTERS]}": 1.0,
        "score_delta": _bounded(item.score_delta, -ACTION_FEATURE_WORD_SCORE_BOUND, ACTION_FEATURE_WORD_SCORE_BOUND) / ACTION_FEATURE_WORD_SCORE_BOUND,
        f"after_origin:{origin}": 1.0,
        f"after_origin:{origin}:direction:{direction}:length:{length}": 1.0,
        f"after_origin:{origin}:script:{_script(item.field.after[:ACTION_FEATURE_AFTER_CONTEXT_CHARACTERS].casefold())}"
        f":direction:{direction}:length:{length}": 1.0,
    }
    if item.source_identifier or item.target_identifier:
        # Only an identifier reading carries new information; without one these
        # combinations would merely duplicate the length, direction and
        # left-context evidence and double their learned weight.
        prefix = f"identifier:{int(item.source_identifier)}:{int(item.target_identifier)}"
        features[f"{prefix}:direction:{direction}"] = 1.0
        features[f"{prefix}:length:{length}"] = 1.0
        features[f"{prefix}:before:{_script(item.field.before[-ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS:].casefold())}"] = 1.0
    source_case = "none"
    for label, raw, score, known, identifier in (
        ("source", item.original, item.source_score, item.source_known, item.source_identifier),
        ("target", item.alternative, item.target_score, item.target_known, item.target_identifier),
    ):
        raw = raw[:ACTION_FEATURE_WORD_MAX_CHARACTERS]
        text = unicodedata.normalize("NFC", raw.casefold())[:ACTION_FEATURE_WORD_MAX_CHARACTERS]
        _characters(features, label, text, direction)
        _word_score(features, label, score, known)
        features[f"{label}:identifier:{int(identifier)}"] = 1.0
        letters = "".join(char for char in raw if char.isalpha())
        case = "none" if not letters else "upper" if letters.isupper() else "lower" if letters.islower() else "title" if letters.istitle() else "mixed"
        if label == "source":
            source_case = case
        features[f"{label}:case:{case}"] = 1.0
        features[f"{label}:script:{_script(text)}"] = 1.0
        features[f"{label}:length"] = len(text) / ACTION_FEATURE_WORD_MAX_CHARACTERS
        features[f"{label}:digits"] = sum(char.isdigit() for char in raw) / max(1, len(raw))
        for mark in ",.;[]'’`-_/@\\=<>\"":
            features[f"{label}:mark:{ord(mark):04x}"] = raw.count(mark) / max(1, len(raw))
    for label, raw in (
        ("before", item.field.before[-ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS:]),
        ("after", item.field.after[:ACTION_FEATURE_AFTER_CONTEXT_CHARACTERS]),
    ):
        text = unicodedata.normalize("NFC", raw.casefold())
        script = _script(text)
        features[f"{label}:script:{script}:direction:{direction}"] = 1.0
        features[f"{label}:script:{script}:direction:{direction}:length:{length}"] = 1.0
        words = _WORDS.findall(text)
        neighbours = words[-ACTION_FEATURE_NEIGHBOUR_WORD_COUNT:] if label == "before" else words[:ACTION_FEATURE_NEIGHBOUR_WORD_COUNT]
        for word in neighbours:
            _characters(features, label, word[:ACTION_FEATURE_NEIGHBOUR_WORD_MAX_CHARACTERS], direction)
        whitespace = raw[len(raw.rstrip()):] if label == "before" else raw[:len(raw) - len(raw.lstrip())]
        whitespace = whitespace[:ACTION_FEATURE_WHITESPACE_MAX_CHARACTERS]
        features[f"{label}:space_count"] = len(whitespace) / ACTION_FEATURE_WHITESPACE_MAX_CHARACTERS
        for char in sorted(set(whitespace)):
            features[f"{label}:space:{ord(char):04x}"] = whitespace.count(char) / ACTION_FEATURE_WHITESPACE_MAX_CHARACTERS
    tail = item.literal_tail[:ACTION_FEATURE_SHORT_TEXT_CHARACTERS]
    features["tail:length"] = len(tail) / ACTION_FEATURE_SHORT_TEXT_CHARACTERS
    features["tail:overflow"] = float(len(item.literal_tail) > ACTION_FEATURE_SHORT_TEXT_CHARACTERS)
    for char in sorted(set(tail)):
        features[f"tail:char:{ord(char):04x}"] = tail.count(char) / ACTION_FEATURE_SHORT_TEXT_CHARACTERS
    boundary = item.boundary_text[:ACTION_FEATURE_SHORT_TEXT_CHARACTERS]
    features["boundary:length"] = len(boundary) / ACTION_FEATURE_SHORT_TEXT_CHARACTERS
    features["boundary:overflow"] = float(len(item.boundary_text) > ACTION_FEATURE_SHORT_TEXT_CHARACTERS)
    features["boundary:isspace"] = float(boundary.isspace())
    for char in sorted(set(boundary)):
        features[f"boundary:char:{ord(char):04x}"] = boundary.count(char) / ACTION_FEATURE_SHORT_TEXT_CHARACTERS
    application = item.field.application.casefold()[:ACTION_FEATURE_APPLICATION_NAME_CHARACTERS]
    for part in re.findall(r"[a-z0-9]+", application)[:ACTION_FEATURE_APPLICATION_TOKEN_COUNT]:
        features[f"app:{part}"] = 1.0
    for label, value in (("baseline:probability", item.model_probability), ("baseline:threshold", item.model_threshold)):
        if value is not None:
            features[label] = _bounded(value, 0.0, 1.0)
            features[label + ":present"] = 1.0
    if item.model_probability is not None and item.model_threshold is not None:
        features["baseline:margin"] = _bounded(item.model_probability - item.model_threshold)
    for label, value in (("ortho:score", item.ortho_score), ("ortho:threshold", item.ortho_threshold)):
        if value is not None:
            features[label] = _bounded(value, -ACTION_FEATURE_ORTHO_SCORE_BOUND, ACTION_FEATURE_ORTHO_SCORE_BOUND) / ACTION_FEATURE_ORTHO_SCORE_BOUND
            features[label + ":present"] = 1.0
    if item.ortho_score is not None and item.ortho_threshold is not None:
        margin = (
            _bounded(item.ortho_score - item.ortho_threshold, -ACTION_FEATURE_ORTHO_SCORE_BOUND, ACTION_FEATURE_ORTHO_SCORE_BOUND)
            / ACTION_FEATURE_ORTHO_SCORE_BOUND
        )
        features["ortho:margin"] = margin
        # The threshold belongs to the frozen orthographic model. Its side is
        # evidence with learned coefficients, alongside the continuous margin.
        side = "positive" if margin > 0 else "negative" if margin < 0 else "zero"
        support = f"direction:{direction}:source_known:{int(item.source_known)}"
        features[f"ortho:side:{side}:{support}"] = 1.0
        features[f"ortho:side:{side}:{support}:case:{source_case}"] = 1.0
        context_script = _script(item.field.before[-ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS:].casefold())
        category = f"direction:{direction}:source_known:{int(item.source_known)}:context:{context_script}"
        # Orthographic evidence may differ by language and lexical support.
        # Both slopes are learned; neither licenses a conversion by itself.
        for sign, value in (("positive", max(0.0, margin)), ("negative", min(0.0, margin))):
            features[f"ortho:{sign}:direction:{direction}"] = value
            features[f"ortho:{sign}:{category}"] = value
    _term_features(features, item, direction)
    letter = letter_question(item.original, item.alternative, item.field.before)
    if origin == "kept_next_word":
        # A waiting word whose next word stayed as typed is a question of its own: the same
        # evidence, its own weights (KEPT_FEATURE_PREFIX). Sharing them, the question took the
        # weights of the converted-neighbour frames and turned `еще` before `поищу` into `tot`.
        kept = {KEPT_FEATURE_PREFIX + name: value for name, value in features.items() if value}
        if letter:
            # The kept-neighbour frames hold no word of one letter, and the question answered `keep`
            # at p=1.00 for `b` before `redis` in `nats b redis` (`nats и redis`, the owner's typing).
            kept.update({LETTER_FEATURE_PREFIX + name: value for name, value in list(kept.items())})
            kept.update(_letter_evidence(item, direction))
        if variable_question(item.original, item.alternative, item.field.before, item.trigger):
            # Nor a letter after a word of Cyrillic letters: the variable head answers there too.
            kept.update(_variable_evidence(item))
        if sign_question(item.original, item.alternative, item.field.before, item.field.after, item.trigger, origin):
            # And a sign after such a letter is evidence the variable head reads as no script at all: asked with `=` after
            # it, `ч` of `для ч = 0` was kept at p=0.95 (the sign head).
            kept.update(_sign_evidence(item))
        if alnum_question(item.original, item.alternative, item.field.before):
            # Nor a token of letters and digits: asked again with `как` kept after it, the question kept `м67` at p=1.00
            # where `v67` was meant (the owner's typing, 19.10.2026).
            kept.update(_alnum_evidence(item))
        return kept
    result = {name: value for name, value in features.items() if value}
    if capitals_question(item.original, item.field.before):
        # The class the capitals head answers: its features once more under their own names, beside
        # the shared ones, so a head fitted onto a frozen model moves no decision outside the class.
        # `BP` in English prose read `ИЗ` at p=0.997, and the frames that kept it moved the weights
        # the Latin keys of `не` and `что` share (`yt`, `xnj` stayed as typed).
        result.update({CAPITALS_FEATURE_PREFIX + name: value for name, value in list(result.items())})
    if resume_question(item.original, item.alternative, item.field.before, item.field.after, origin):
        # A short word right after Latin text asked again once its next word is planned for conversion: no head answered
        # it, and with Hunspell, which knows `f` and `z` as English words, the frozen model kept `а` in `seek() , f там`
        # and `я` in `TSet - z использовал` (the resume head).
        result.update(_resume_evidence(item, direction, features))
    if alone_question(item.original, item.alternative, item.field.before, item.field.after, item.trigger):
        # The class the lone-word head answers, in the same way: every corpus frame of it was deferred,
        # so the frozen model waits on `гш` alone and never converts it to `ui`. The head also reads how
        # often each reading occurs, which the shared features leave out for a word with nothing
        # around it: `ой` (Russian prose, 1298 times) and `гш` (never) were the same to it.
        result.update({ALONE_FEATURE_PREFIX + name: value for name, value in list(result.items())})
        result.update(_alone_counts(item, direction))
    if start_question(item.original, item.alternative, item.field.before, item.field.after):
        # The class the message-start head answers, in the same way. With nothing around the word the
        # shared features leave out how often each reading occurs in prose, and a rare Russian word
        # opening a message was converted on its letters alone; the head reads both prose counts and the
        # score delta beyond the shared feature's bound.
        result.update({START_FEATURE_PREFIX + name: value for name, value in list(result.items())})
        result.update(_start_evidence(item, direction))
    if letter:
        # The class the single-letter head answers, in the same way. After a word of Latin letters the
        # frozen model kept `f` in `lid f номер` at p=1.00 with the next word converted (`lid а номер`,
        # the owner's typing): no frame of the corpus puts a Russian word of one letter there.
        result.update({LETTER_FEATURE_PREFIX + name: value for name, value in list(result.items())})
        result.update(_letter_evidence(item, direction))
    if abbreviation_question(item.original, item.alternative, item.field.before):
        # The class the abbreviation head answers. After Russian prose the frozen model turned abbreviations
        # Russian text uses into the Latin readings they spell (`страны ЕС приняли` became `страны TC
        # приняли`). Unlike the other heads it reads no copy of the shared features: the corpus holds a few
        # hundred kinds of abbreviation, and a head reading their letter n-grams learned those kinds by heart
        # and turned `ТЗ` into `NP`. It reads what tells the kinds apart instead: how often each reading
        # occurs.
        result.update(_abbreviation_evidence(item, direction))
    if latin_abbreviation_question(item.original, item.alternative, item.field.before):
        # The same abbreviation after Latin text, the Latin-context abbreviation head's class: the same counts,
        # and how often the word before it occurs in English prose and in Russian technical text, which tell a
        # sentence of English prose from a command or an identifier (`/designv2 ТЗ`, the owner's typing).
        result.update(_latin_abbreviation_evidence(item, direction))
    if unknown_question(item.original, item.field.before) and not item.source_known:
        # The class the unknown-token head answers: its features once more under their own names, beside the shared
        # ones, and evidence of its own. After Russian prose the frozen model kept a token no table counts as Russian
        # slang or a Latin citation, and the words typed in the wrong layout with it: `цуиоы` (`webjs`) at p=0.94 with
        # `webjs` typed right earlier in the same field, `мгуоы` (`vuejs`), `реьч` (`htmx`) (the owner's typing,
        # 13.10.2026), and the last word of a short Russian message, a word no table counts, typed in the English
        # layout (test v56).
        result.update({UNKNOWN_FEATURE_PREFIX + name: value for name, value in features.items() if value})
        result.update(_unknown_evidence(item))
    if joined_question(item.original):
        # The class the joined-token head answers: its features once more under their own names, beside the shared
        # ones, and evidence of its own. No term table counts such a token, so the shared features read no count of
        # it at all, and the frozen model kept `Re-re!` after a Russian sentence, meant as `Ку-ку!`, at p=0.99
        # (sealed test v58, both pairs); the head reads the counts of its words.
        result.update({JOINED_FEATURE_PREFIX + name: value for name, value in features.items() if value})
        result.update(_joined_evidence(item))
    if (brief_question(item.original, item.field.before, item.field.after) and not item.source_known
            and brief_settled(item.original, item.alternative)):
        # The class the short-message head answers: a short reply typed in the English layout, which the frozen model
        # kept or left waiting for a next word that never came (`ytn` for `нет`). The head reads how often each reading
        # occurs, the boundary and the case, and no copy of the shared features: with one it learned that a known Latin
        # word stays, and kept `vs` sent alone for `мы` (experiment E6). It answers only a token whose counts settle it,
        # one no lexicon of the profile knows as typed: a pair both readings count (`ij` and `шо`, `ds` and `вы`) stays
        # with the frozen model, which turned no Russian reply into Latin (candidate v63e).
        result.update(_brief_evidence(item))
    if variable_question(item.original, item.alternative, item.field.before, item.trigger):
        # The class the variable head answers: a letter of the Russian layout after a word of Cyrillic letters that is no
        # Russian word of one letter. Every single-letter row of the corpus sits in quarantine, and the frozen model's
        # verdict on such a letter is no opinion: it turned `ч` into `x` in the owner's chat and kept `т` for `n` in
        # `результат n - 1`; the lone-letter refusal (context_policy) kept the conversions it got right too.
        result.update(_variable_evidence(item))
    if dotted_question(item.original, item.alternative):
        # The class the dotted-abbreviation head answers: no frame of the corpus types `т.к.` in the English layout, and
        # the frozen model kept `n/r/` at p=1.00 as a path would be kept (the developer-chat development set).
        result.update(_dotted_evidence(item))
    if alnum_question(item.original, item.alternative, item.field.before):
        # The class the letters-and-digits head answers: the shared features read every digit as one class, and the frozen
        # model left `м67` for `v67` a suggestion, kept `x64` typed in the Russian layout as `ч64` and turned `100мс` into
        # `100vc`; its conversions of a token of one letter the lone-letter refusal kept as well (context_policy).
        result.update(_alnum_evidence(item))
    if hypothesis_question(item):
        # The class the hypothesis head answers: the next word of a waiting word asked with that word's other reading
        # before it. The frozen model read the reading as typed text, and a word that is no word of either layout went the
        # reading's way: `тыв` (a typo of `ты`) after `tot ` became `nsd` at p=1.00, and `еще тыв` became `tot nsd` (the
        # owner's typing, 22.10.2026).
        result.update(_hypothesis_evidence(item))
    elif slip_question(item.original, item.field.before) and not item.source_known and not item.source_identifier:
        # The class the slip head answers: a word of Russian prose typed with the English layout on after a line of Russian
        # text, whose Latin reading no lexicon, identifier index or table of Latin text supports. The frozen model left such
        # a word a suggestion just under the threshold, which no pause settles (`я cgtie` for `я спешу`), or kept it
        # (`волк pfdsk` for `волк завыл`): its keep weights for unknown Latin tokens after Russian prose pulled them down.
        result.update(_slip_evidence(item))
    elif short_question(item.original, item.field.before) and not item.source_known:
        # The class the short-token head answers: a token of two or three Russian letters after Russian text that no table
        # of Russian text counted, whose Latin reading the English lexicon knows. The frozen model turned `кз` after
        # `говорит он ` into `rp` at p=0.997 (the owner's typing, 22.10.2026), and `дз` and `ср` into `lp` and `ch` in
        # Russian phrases; the head reads how often Russian web and news text writes the token.
        result.update(_short_evidence(item))
    known = item.source_known and not _tail_like(item.original)
    if first_question(item.original, item.field.before, item.field.after) and item.field.role != "unknown" and not known:
        # The class the first-term head answers: a term of up to three letters alone in a field a reader read empty,
        # where it opens a message (`йк ` for `qr`: the owner's requirement of 16.10.2026 that it convert at once, where
        # the frozen model waited for a pause). The term head, which chooses an early epoch for its other frames, stopped
        # short of the threshold there: `зк` and `йк` became suggestions at p 0.98, which no pause settles (candidates
        # v64b to v64e). The tail of a Russian word is no token of this class: an edit that leaves one as a token leaves
        # letters before it, or reopens the word and asks the whole of it.
        result.update(_first_evidence(item))
    elif term_question(item.original, item.field.before) and not known:
        # The class the term head answers: a term typed in the Russian layout, which the frozen model left at p 0.92-0.98
        # the first time a message held it (`сымю` for `csv.`, `сш` for `ci`). The head reads evidence of its own alone:
        # with a copy of the shared features it learned that two letters and a known Latin reading stay, and kept `зк`
        # for `pr` after `принимай все` (experiment E6); the frozen model reads the context. A word the profile's lexicon
        # knows is a Russian word however rare in the tables, and stays with the frozen model: a word of four letters
        # opening a correct Russian sentence, known to Hunspell, went to its Latin reading, a term of 58 counts (sealed
        # test v62, the reference profile). A tail of Russian words the lexicon knows (`ев`, `ась`) stays the head's: the
        # head learns to keep tails from such frames, and without them it stopped learning after its first epoch, as it
        # did reading whether the lexicon knows the token (corpora v62 and v63).
        result.update(_term_evidence(item))
    return result


def attested_abbreviation(original: str) -> bool:
    """A token of letters only, all capitals, of CAPITALS_HEAD_MIN_LETTERS to CAPITALS_HEAD_MAX_LETTERS, in
    Cyrillic, that Russian technical text uses and the lexicon does not hold: counted among the Cyrillic words
    of Russian technical text, which the term table keeps for words outside the lexicon (`ЕС`, `ТЗ`, `ИД`). A
    lexicon word in capitals (`ЛУНЫ`) is a word typed with Caps Lock, which the frozen model keeps."""

    if not (CAPITALS_HEAD_MIN_LETTERS <= len(original) <= CAPITALS_HEAD_MAX_LETTERS and original.isalpha()
            and original.isupper() and not any(char.isascii() for char in original)):
        return False
    from .context_model import _term_frequency

    table = _term_frequency()
    return bool(table["cyrillic"].get(original.casefold()))


def abbreviation_question(original: str, alternative: str, before: str) -> bool:
    """An attested Cyrillic abbreviation (attested_abbreviation) whose Latin reading is letters too, after text
    with more Cyrillic letters than Latin ones: the abbreviation as written (`страны ЕС`) or a Latin one typed
    in the Russian layout whose keys spell it (`TC`). After Latin text the capitals head answers the same
    shape (capitals_question); an abbreviation Russian text does not use is the frozen model's."""

    return (attested_abbreviation(original) and alternative.isalpha()
            and _dominant(before[-ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS:].casefold()) == "ru")


def _abbreviation_evidence(item: ContextEvidence, direction: str) -> dict[str, float]:
    """The abbreviation head's evidence: the binary order of magnitude of how often the Latin reading occurs
    among the Latin words of Russian technical text and in English prose, and the Cyrillic one in Russian
    prose and among the Cyrillic words of Russian technical text, and of their ratio; whether the lexicons
    and the identifier index know each reading; the length; whether the Cyrillic reading has a letter no
    Russian word starts with (ABBREVIATION_NON_INITIAL_LETTERS), which no abbreviation of Russian words
    has; the boundary. No letter of its own: a head reading them learned the corpus's kinds of
    abbreviation and turned `ДТП` into `LNG`."""

    return {f"{ABBREVIATION_FEATURE_PREFIX}{name}:direction:{direction}": 1.0 for name in _abbreviation_names(item)}


def _abbreviation_names(item: ContextEvidence) -> list[str]:
    """The evidence both abbreviation heads read (_abbreviation_evidence), without their prefix."""

    from .context_model import _term_frequency

    table = _term_frequency()
    latin, cyrillic = item.alternative.casefold(), item.original.casefold()
    magnitudes = {"latin": table["latin"].get(latin, 0).bit_length(), "english": table["english"].get(latin, 0).bit_length(),
                  "russian": table["russian"].get(cyrillic, 0).bit_length(), "cyrillic": table["cyrillic"].get(cyrillic, 0).bit_length()}
    latin_side = max(magnitudes["latin"], magnitudes["english"])
    cyrillic_side = max(magnitudes["russian"], magnitudes["cyrillic"])
    ratio = max(-ALONE_COUNT_RATIO_BOUND, min(ALONE_COUNT_RATIO_BOUND, latin_side - cyrillic_side))
    return ["bias", *(f"count:{name}:{value}" for name, value in magnitudes.items()), f"count:ratio:{ratio}",
            f"count:{latin_side}:{cyrillic_side}", f"letters:{len(item.original)}",
            f"known:{int(item.source_known)}:{int(item.target_known)}",
            f"identifier:{int(item.source_identifier)}:{int(item.target_identifier)}",
            f"non_initial:{int(any(char in ABBREVIATION_NON_INITIAL_LETTERS for char in item.original))}",
            f"trigger:{item.trigger[:ACTION_FEATURE_FIELD_LABEL_MAX_CHARACTERS]}"]


def latin_abbreviation_question(original: str, alternative: str, before: str) -> bool:
    """An attested Cyrillic abbreviation (attested_abbreviation) whose Latin reading is letters too, after text with
    more Latin letters than Cyrillic ones: `ТЗ` after `/designv2 `, or `US` after English prose typed in the Russian
    layout (`ГЫ`). The capitals head answers the same shape with a copy of the shared features, and both turned
    abbreviations the Latin reading of which nobody writes into it (`ТЗ` into `NP`, `НГ` into `YU`)."""

    return (attested_abbreviation(original) and alternative.isalpha()
            and _dominant(before[-ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS:].casefold()) == "en")


def _latin_abbreviation_evidence(item: ContextEvidence, direction: str) -> dict[str, float]:
    """The Latin-context abbreviation head's evidence: the abbreviation head's (_abbreviation_names), and the binary
    order of magnitude of how often the word before it occurs in English prose and among the Latin words of Russian
    technical text (_neighbour_names)."""

    from .context_model import _term_frequency

    words = item.field.before.split()
    names = [*_abbreviation_names(item), *_neighbour_names("previous_word", words[-1] if words else "", _term_frequency())]
    return {f"{LATIN_ABBREVIATION_FEATURE_PREFIX}{name}:direction:{direction}": 1.0 for name in names}


def alone_question(original: str, alternative: str, before: str, after: str, trigger: str) -> bool:
    """A token of ALONE_HEAD_LETTERS letters, letters in both layouts, with no letter before or after it
    in the field, at a boundary that ends it for good: a message of one such word (`гш` for `ui`) sent,
    or left at a pause. At a space the next word may still come and decide it with its neighbour, so
    the frozen model's wait stands there. `хз` is no member: its Latin keys `[p` are no word."""

    return (len(original) == ALONE_HEAD_LETTERS and original.isalpha() and alternative.isalpha()
            and trigger != "space"
            and not any(char.isalpha() for char in before) and not any(char.isalpha() for char in after))


def _alone_counts(item: ContextEvidence, direction: str) -> dict[str, float]:
    """The lone-word head's own count evidence: the order of magnitude of each reading's count in the
    term table (`latin`; `cyrillic` or `russian`, whichever is larger) and of their ratio."""

    from .context_model import _term_frequency

    table = _term_frequency()
    latin, cyrillic = (item.original, item.alternative) if item.source_group == 0 else (item.alternative, item.original)
    latin_count = table["latin"].get(latin.casefold(), 0)
    cyrillic_count = max(table["cyrillic"].get(cyrillic.casefold(), 0), table["russian"].get(cyrillic.casefold(), 0))
    magnitudes = (latin_count.bit_length(), cyrillic_count.bit_length())
    ratio = max(-ALONE_COUNT_RATIO_BOUND, min(ALONE_COUNT_RATIO_BOUND, magnitudes[0] - magnitudes[1]))
    names = (f"count:latin:{magnitudes[0]}", f"count:cyrillic:{magnitudes[1]}", f"count:ratio:{ratio}",
             f"count:{magnitudes[0]}:{magnitudes[1]}")
    return {f"{ALONE_FEATURE_PREFIX}{name}:direction:{direction}": 1.0 for name in names}


def start_question(original: str, alternative: str, before: str, after: str) -> bool:
    """A word of at least START_HEAD_MIN_LETTERS letters, letters in both layouts, with no letter before or
    after it in the field: the first word of a message, at any boundary. Two letters are the lone-word
    head's (alone_question) or wait for the next word; a word whose other reading has signs (`J,thyed`)
    is no member."""

    return (len(original) >= START_HEAD_MIN_LETTERS and original.isalpha() and alternative.isalpha()
            and not any(char.isalpha() for char in before) and not any(char.isalpha() for char in after))


def _start_evidence(item: ContextEvidence, direction: str) -> dict[str, float]:
    """The message-start head's own evidence: the binary order of magnitude of how often the Latin reading
    occurs in English prose and the Cyrillic one in Russian prose, and of their ratio; and the band of the
    detector's score delta (START_DELTA_BAND_WIDTH wide, START_DELTA_BANDS either way)."""

    from .context_model import _term_frequency

    table = _term_frequency()
    latin, cyrillic = (item.original, item.alternative) if item.source_group == 0 else (item.alternative, item.original)
    magnitudes = (table["english"].get(latin.casefold(), 0).bit_length(), table["russian"].get(cyrillic.casefold(), 0).bit_length())
    ratio = max(-ALONE_COUNT_RATIO_BOUND, min(ALONE_COUNT_RATIO_BOUND, magnitudes[0] - magnitudes[1]))
    band = max(-START_DELTA_BANDS, min(START_DELTA_BANDS, math.floor(item.score_delta / START_DELTA_BAND_WIDTH)))
    names = (f"count:english:{magnitudes[0]}", f"count:russian:{magnitudes[1]}", f"count:ratio:{ratio}",
             f"count:{magnitudes[0]}:{magnitudes[1]}", f"delta:{band}")
    return {f"{START_FEATURE_PREFIX}{name}:direction:{direction}": 1.0 for name in names}


def resume_question(original: str, alternative: str, before: str, after: str, origin: str) -> bool:
    """A short word right after Latin text asked again once the word after it is planned for conversion (after_origin
    planned_next_conversion): the nearest letter before it is Latin, a word follows it, and in either reading it has at
    most OPENING_SHORT_MAX_LETTERS letters once one leading sign of RESUME_LEADING_SIGNS is set aside (`f`/`а` in
    `seek() , f там`, `ds`/`вы` in `Shift+Tab ds`, `(b`/`(и`). That is where a person who forgot to switch back to Russian
    after a Latin token resumes Russian text, and where a Latin token is followed by Russian text typed in the English
    layout. A word of one letter right after a Latin word and a space is the single-letter head's (letter_question). The
    question reads only the token, the text around it and the origin, as the trainer and the GPU path route frames by
    them."""

    letters = [char for char in before[-ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS:] if char.isalpha()]
    return (origin == "planned_next_conversion" and bool(letters) and letters[-1].isascii()
            and any(char.isalpha() for char in after) and _resume_word(original) != "" and _resume_word(alternative) != ""
            and not letter_question(original, alternative, before))


def _resume_word(token: str) -> str:
    """The letters of a short word without its one leading sign, or "" when it is no such word."""

    word = token[1:] if token[:1] in RESUME_LEADING_SIGNS else token
    return word if word.isalpha() and len(word) <= OPENING_SHORT_MAX_LETTERS else ""


def _resume_evidence(item: ContextEvidence, direction: str, features: dict[str, float]) -> dict[str, float]:
    """The resume head's evidence: the shared features once more under its own names, so that it weighs the lexicons'
    and Hunspell's verdicts in its class apart (Hunspell knows `f`, and the detector's score delta, which converted `f`
    without it, is gone), and evidence of its own: how often the Latin word before it occurs in English prose and among
    the Latin words of Russian technical text, how often the next word as planned occurs, the signs between the Latin
    word and it, whether Hunspell knows each reading, the leading sign, the number of letters and their case."""

    from .context_model import _term_frequency

    table = _term_frequency()
    before = item.field.before[-ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS:]
    last = max(index for index, char in enumerate(before) if char.isalpha())
    gap = "".join(sorted(set(before[last + 1:]) - {" "})) or "space"
    previous = re.findall(r"[A-Za-z]+", before)[-1]
    word = _resume_word(item.original)
    case = "upper" if word.isupper() and len(word) > 1 else "title" if word[:1].isupper() else "lower"
    sign = item.original[:1] if item.original[:1] in RESUME_LEADING_SIGNS else "none"
    spelled = (int(bool(item.source_score and item.source_score.spell_known)),
               int(bool(item.target_score and item.target_score.spell_known)))
    names = ("bias", f"gap:{gap}", f"spell:{spelled[0]}:{spelled[1]}", f"sign:{sign}", f"letters:{len(word)}",
             f"case:{case}", f"letters:{len(word)}:spell:{spelled[0]}:{spelled[1]}",
             *_neighbour_names("previous_word", previous, table),
             *_neighbour_names("next_word", (item.field.after.split() or [""])[0], table))
    return {**{RESUME_FEATURE_PREFIX + name: value for name, value in features.items() if value},
            **{f"{RESUME_FEATURE_PREFIX}{name}:direction:{direction}": 1.0 for name in names}}


def letter_question(original: str, alternative: str, before: str) -> bool:
    """A word of one letter, a letter in both layouts, right after a word whose letters are all Latin:
    `b` after `nats ` (`nats и redis`), `f` after `lid ` (`lid а номер`), `a` after `use `. The engine
    asks it at the letter's boundary and again beside the next word, converted or kept as typed. After
    Cyrillic, after a sign or with nothing before it the frozen model decides alone."""

    if len(original) != 1 or not original.isalpha() or not alternative.isalpha() or not before[-1:].isspace():
        return False
    words = before.split()
    letters = [char for char in words[-1] if char.isalpha()] if words else []
    return bool(letters) and all(char.isascii() for char in letters)


def _letter_evidence(item: ContextEvidence, direction: str) -> dict[str, float]:
    """The single-letter head's own evidence: the binary order of magnitude of how often the word before
    the letter, and the word after it, occur in English prose and among the Latin words of Russian
    technical text (a Cyrillic word after it: in Russian prose). `nats` and `redis` are counted in Russian
    technical text alone, `plan` and `then` in English prose."""

    from .context_model import _term_frequency

    table = _term_frequency()
    names = [*_neighbour_names("previous_word", item.field.before.split()[-1], table),
             *_neighbour_names("next_word", (item.field.after.split() or [""])[0], table)]
    return {f"{LETTER_FEATURE_PREFIX}{name}:direction:{direction}": 1.0 for name in names}


def _neighbour_names(side: str, text: str, table: dict[str, dict[str, int]]) -> list[str]:
    """How often a neighbouring word occurs: a Latin one in English prose and among the Latin words of Russian
    technical text, a Cyrillic one in Russian prose, each as a binary order of magnitude; `none` without letters."""

    word = "".join(char for char in text if char.isalpha()).casefold()
    if not word:
        return [f"{side}:none"]
    if all(char.isascii() for char in word):
        english, latin = table["english"].get(word, 0).bit_length(), table["latin"].get(word, 0).bit_length()
        return [f"{side}:english:{english}", f"{side}:latin:{latin}", f"{side}:{english}:{latin}"]
    return [f"{side}:russian:{table['russian'].get(word, 0).bit_length()}"]


def capitals_question(original: str, before: str) -> bool:
    """A token of letters only, all capitals, of CAPITALS_HEAD_MIN_LETTERS to CAPITALS_HEAD_MAX_LETTERS,
    after text with more Latin letters than Cyrillic ones (`according to the BP`).

    After Russian prose the same shape is as often a word typed with Caps Lock (`я YT` for `я НЕ`) as a
    cited abbreviation (`по версии WBC`), and a head fitted there kept 54 of 72 such words the corpus
    v26 pair converts, or converted the abbreviations; there the frozen model decides alone.
    """

    return (CAPITALS_HEAD_MIN_LETTERS <= len(original) <= CAPITALS_HEAD_MAX_LETTERS and original.isalpha() and original.isupper()
            and _dominant(before[-ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS:].casefold()) == "en")


_TOKENS: Final = re.compile(r"[^\W_]+", re.UNICODE)


def unknown_shape(original: str) -> bool:
    """A token the unknown-token head's class may hold (unknown_question): UNKNOWN_HEAD_MIN_CHARACTERS long or more, not
    all capitals, of digits and of letters of one script - Russian letters whose keys are letters too, or Latin ones.
    Capitals are the abbreviation heads' shapes; a Russian letter whose key is a sign (UNKNOWN_HEAD_SIGN_KEY_LETTERS)
    makes the Latin reading no term (`фолбек` is `ajk,tr`), and a Latin token's keys are such letters in no case."""

    if len(original) < UNKNOWN_HEAD_MIN_CHARACTERS or not original.isalnum() or original.isupper():
        return False
    letters = [char.casefold() for char in original if char.isalpha()]
    return bool(letters) and (all("a" <= char <= "z" for char in letters)
                              or all("а" <= char <= "я" and char not in UNKNOWN_HEAD_SIGN_KEY_LETTERS for char in letters))


def unknown_cyrillic(original: str) -> str:
    """The Cyrillic reading of a token of the head's shape: the token itself, or the Russian letters of its keys."""

    from .layouts import LayoutPair

    return original if any("а" <= char.casefold() <= "я" for char in original) else LayoutPair().translate(original, "us", "ru")


def _earlier_words(before: str) -> set[str]:
    """The words of the text before a token, as the unknown-token head compares readings with them."""

    return {word.casefold() for word in _TOKENS.findall(before[-ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS:])}


def unknown_question(original: str, before: str) -> bool:
    """A token of the unknown-token head's shape (unknown_shape) whose Cyrillic reading Russian prose does not use, after
    text with more Cyrillic letters than Latin ones: Latin letters typed in either layout (a Russian word no table
    counts typed in the English layout, which test v56 found kept, or a Latin term typed as meant), or Russian letters
    whose Latin reading the text before already holds as a word (`цуиоы` after `webjs` typed right earlier in the same
    message). With its typed reading no lexicon word of its layout it is the head's class (extract_action_features).
    Russian letters without that echo stay with the frozen model: there the class is Russian slang, truncations and
    brands in Cyrillic as often as terms, and the head fitted on them converted `пукты` and `иишку` in the owner's
    typing. The question reads only the typed token, the text before it and the packaged term table, as the trainer
    and the GPU path route frames by them (head_question, context_action_cuda.heads_question)."""

    from .context_model import term_bucket
    from .layouts import LayoutPair

    if not (unknown_shape(original) and term_bucket(unknown_cyrillic(original), "russian") in ("0", "na")
            and _dominant(before[-ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS:].casefold()) == "ru"):
        return False
    latin = unknown_cyrillic(original) != original
    return latin or LayoutPair().translate(original, "ru", "us").casefold() in _earlier_words(before)


def _unknown_evidence(item: ContextEvidence) -> dict[str, float]:
    """The unknown-token head's evidence beside its copy of the shared features.

    Whether either reading occurs as a whole word earlier in the field: a term typed right earlier in the same message
    is the term typed again (`webjs не дает информацию о номерах ... дело в цуиоы`), while the shared features read only
    the two words next to the token. Whether a reading is no word but one typo from one (`вляется`), which the shared
    features do not read either. And the token's length.
    """

    words = _earlier_words(item.field.before)
    echo = f"echo:{int(item.alternative.casefold() in words)}:{int(item.original.casefold() in words)}"
    letters = f"letters:{min(len(item.original), ACTION_FEATURE_LENGTH_BUCKET_MAX_CHARACTERS)}"
    names = (echo, f"typo:{int(item.source_typo)}:{int(item.target_typo)}", letters, f"{echo}:{letters}")
    # Each layout learns its own weights: an echo converts `цуиоы` after `webjs`, while `vscode` typed right after its
    # keys left as `мысщву` by a miss stays.
    return {f"{UNKNOWN_FEATURE_PREFIX}{name}:direction:{item.source_group}": 1.0 for name in names}


def joined_question(original: str) -> bool:
    """A token one of whose readings is words joined by hyphens (joined_parts): the joined-token head's class.

    The other reading is the same keys in the other layout: `j,sxyj-nfr` is `обычно-так`. The question reads only the
    typed token, as the trainer and the GPU path route frames by it (head_question, context_action_cuda.heads_question).
    """

    from .layouts import LayoutPair

    if joined_parts(original) is not None:
        return True
    latin = any("a" <= char.casefold() <= "z" for char in original)
    return joined_parts(LayoutPair().translate(original, "us", "ru") if latin
                        else LayoutPair().translate(original, "ru", "us")) is not None


def _joined_evidence(item: ContextEvidence) -> dict[str, float]:
    """The joined-token head's evidence beside its copy of the shared features: how often the most and the least
    frequent word of each reading occurs in its language's prose, and whether the token repeats one word.

    No term table counts a token of words joined by hyphens, and the shared features read no count of it, while its
    words tell the language: `ку` occurs in Russian prose and `re` nowhere in English (`Re-re!` is `Ку-ку!`), and
    `pre-order` is English by `order` though `pre` is no word of either prose. The shared features stay as the frozen
    model read them: fed these counts, its weights for single words turned `bye-bye` typed as meant into `ину-ину`.
    """

    from .context_model import term_bucket

    latin = item.original if item.source_group == 0 else item.alternative
    cyrillic = item.alternative if item.source_group == 0 else item.original
    names: list[str] = []
    for reading, alphabet in ((latin, "english"), (cyrillic, "russian")):
        parts = joined_parts(reading)
        buckets = ["na"] if parts is None else sorted((term_bucket(part, alphabet) for part in parts), key=int)
        if item.term_counts_dropped:
            buckets = ["0" if bucket != "na" else bucket for bucket in buckets]
        names.append(f"{alphabet}:most:{buckets[-1]}:least:{buckets[0]}")
    words = joined_parts(item.original) or joined_parts(item.alternative) or []
    repeat = f"repeat:{int(len({word.casefold() for word in words}) == 1)}"
    counts = f"{names[0]}:{names[1]}"
    return {f"{JOINED_FEATURE_PREFIX}{name}:direction:{item.source_group}": 1.0
            for name in (*names, counts, repeat, f"{repeat}:{counts}")}


def term_question(original: str, before: str) -> bool:
    """A token of Russian letters whose Latin reading is Latin letters, a trailing sign (TERM_HEAD_TRAILING_SIGNS) and
    the leading dot of an extension aside, that Russian technical text uses at least in bucket
    TERM_HEAD_MIN_LATIN_BUCKET of the term table, while its Cyrillic reading occurs in no Russian text the tables count
    (`сымю` is `csv.`, `сш` is `ci`, `рвьш` is `hdmi`): the term head's class. A token the unknown-token head answers
    (a term typed right earlier in the text) is that head's, words joined by hyphens are the joined-token head's, and
    an attested abbreviation is counted among Cyrillic words. The question reads only the typed token, the text before
    it and the packaged term table, as the trainer and the GPU path route frames by them (head_question,
    context_action_cuda.heads_question)."""

    from .context_model import term_bucket
    from .layouts import LayoutPair

    if len(original) < TERM_HEAD_MIN_LETTERS or not all("а" <= char.casefold() <= "я" or char.casefold() == "ё"
                                                         for char in original):
        return False
    latin_reading = LayoutPair().translate(original, "ru", "us")
    core = latin_reading.rstrip(TERM_HEAD_TRAILING_SIGNS).removeprefix(TERM_HEAD_LEADING_SIGN)
    if len(core) < TERM_HEAD_MIN_LETTERS or not (core.isascii() and core.isalpha()):
        return False
    latin = term_bucket(core, "latin")
    return (int(latin) >= TERM_HEAD_MIN_LATIN_BUCKET and term_bucket(original, "cyrillic") == "0"
            and term_bucket(original, "russian") == "0" and not unknown_question(original, before))


def _term_evidence(item: ContextEvidence) -> dict[str, float]:
    """The term head's evidence, all it reads (it has no copy of the shared features): how often Russian technical text
    and English text use the Latin reading, whether it ends in a sign typed on a Russian letter's key (`сымю` is `csv.`) or opens
    with the dot of an extension (`юучу` is `.exe`), how many Russian words end with it (russian_ending_bucket) with the
    token's length, which script the text before it is in, the band of the detector's score delta (as the message-start
    head reads it), how the text after it was given (asked again with its converted neighbour after it, or not) and, for
    a token alone in the field, whether a space or an end (Enter, a pause, a sign) ended it, with its letters: a term of
    two letters alone at a space waits, a longer one converts, and read without the letters the wait pulled the longer
    ones and the convert the shorter (`гш` became a suggestion, experiment E9f). A term of up to three letters alone in a
    field a reader read empty is the first-term head's (first_question), not this head's.
    Without the endings the head could tell the tail of a Russian word an edit left (`ев`) from a term's keys (`зк`) by
    the length alone, and it kept short terms; the length is read with the ending only, for the same reason. Alone at a
    space the frozen model waits, and a wait stands until a pause of one and a half seconds settles it: the head
    converts there by its own weights (`йк` for `qr`, the owner's typing of 15.10.2026). Asked again once the Russian
    word after it went back to Russian, the term is the same term: `зк` became `pr` in `займись зк и тз`, the layout
    followed, `и` came out as `b`, and asked again with `и` after it the frozen model leaned to keeping `зк`, and the
    engine took both back.
    """

    from .context_model import term_bucket

    latin = item.alternative
    counts = f"latin:{term_bucket(latin, 'latin')}:english:{term_bucket(latin, 'english')}"
    signed = f"signed:{int(latin.endswith(tuple(TERM_HEAD_TRAILING_SIGNS)))}"
    extension = f"extension:{int(latin.startswith(TERM_HEAD_LEADING_SIGN))}"
    letters = f"letters:{min(sum(char.isalpha() for char in latin), ACTION_FEATURE_LENGTH_BUCKET_MAX_CHARACTERS)}"
    before = f"before:{_dominant(item.field.before[-ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS:].casefold())}"
    ending = f"ending:{russian_ending_bucket(item.original)}"
    band = max(-START_DELTA_BANDS, min(START_DELTA_BANDS, math.floor(item.score_delta / START_DELTA_BAND_WIDTH)))
    delta = f"delta:{band}"
    boundary = f"{before}:boundary:{'space' if item.trigger == 'space' else 'end'}"
    after = f"after:{item.after_origin}"
    names = ("bias", counts, signed, extension, before, ending, delta, boundary, after, f"{counts}:{before}", f"{signed}:{before}",
             f"{ending}:{letters}", f"{ending}:{before}", f"{ending}:{counts}", f"{ending}:{letters}:{counts}",
             f"{delta}:{before}", f"{boundary}:{ending}:{letters}", f"{boundary}:{letters}", f"{after}:{ending}",
             f"{after}:{counts}")
    return {f"{TERM_FEATURE_PREFIX}{name}": 1.0 for name in names}


def russian_ending_bucket(token: str) -> str:
    """How many words of Russian prose end with the token, as the binary order of magnitude of their number
    (TERM_HEAD_ENDING_BUCKET_MAX at most); `na` for a token of another length than TERM_HEAD_ENDING_LETTERS."""

    word = token.casefold()
    if len(word) not in TERM_HEAD_ENDING_LETTERS:
        return "na"
    return str(min(_russian_endings().get(word, 0).bit_length(), TERM_HEAD_ENDING_BUCKET_MAX))


def russian_start_bucket(token: str) -> str:
    """How many words of Russian prose begin with the token, as the binary order of magnitude of their number
    (TERM_HEAD_ENDING_BUCKET_MAX at most); `na` for a token of another length than TERM_HEAD_ENDING_LETTERS."""

    word = token.casefold()
    if len(word) not in TERM_HEAD_ENDING_LETTERS:
        return "na"
    return str(min(_russian_starts().get(word, 0).bit_length(), TERM_HEAD_ENDING_BUCKET_MAX))


def _tail_like(token: str) -> bool:
    """Whether a token ends as many Russian words as a tail the term head keeps does (TERM_FRAGMENT_MIN_ENDING_BUCKET)."""

    ending = russian_ending_bucket(token)
    return ending != "na" and int(ending) >= TERM_FRAGMENT_MIN_ENDING_BUCKET


def first_question(original: str, before: str, after: str) -> bool:
    """A token of the term head's class (term_question) of up to three letters, the most a term's ending is read for
    (TERM_HEAD_ENDING_LETTERS), with no letter before it in the field: the first-term head's class when a field reader
    read the field (the role, which the trainer's and the GPU path's routing do not see, is checked where the features
    are made). The word after it is the one the engine asks it again with: a term converted there once a Russian word
    followed, a token that waited once its next word was converted. The question reads only the typed token and the
    text before it."""

    return (len(original) <= max(TERM_HEAD_ENDING_LETTERS) and term_question(original, before)
            and not any(char.isalpha() for char in before))


def _first_evidence(item: ContextEvidence) -> dict[str, float]:
    """The first-term head's evidence, every piece read with the boundary that ended the token (a space, or an end such
    as a pause): its letters, how often Russian technical text and English prose use its Latin reading, how many Russian
    words end and begin with it, and how the word after it was given (none, or the next word it is asked again with).
    Ended by a pause, a token many Russian words begin with may be a word stopped in the middle (`уче` of `учеб`), and
    that answer must not reach the space: evidence shared by both took the pause's answer to the space too and left
    `ци`, `пя` and `мц` suggestions there at p 0.95-0.985 (candidate v67b). Asked again by the term head, a term converted
    there went back with the Russian word after it on one corpus and stayed on another (`йк истек`, p 0.981 and 0.995,
    candidates v65 and v64f)."""

    from .context_model import term_bucket

    latin = item.alternative
    letters = f"letters:{sum(char.isalpha() for char in item.original)}"
    boundary = f"boundary:{'space' if item.trigger == 'space' else 'end'}"
    counts = f"latin:{term_bucket(latin, 'latin')}:english:{term_bucket(latin, 'english')}"
    ending = f"ending:{russian_ending_bucket(item.original)}"
    start = f"start:{russian_start_bucket(item.original)}"
    after = f"after:{item.after_origin}"
    names = ("", letters, counts, f"{ending}:{letters}", start, f"{start}:{letters}", after, f"{after}:{letters}",
             f"{after}:{counts}")
    return {f"{FIRST_FEATURE_PREFIX}{boundary}{':' if name else ''}{name}": 1.0 for name in names}


def _cyrillic_letter(char: str) -> bool:
    return "а" <= char.casefold() <= "я" or char.casefold() == "ё"


def variable_question(original: str, alternative: str, before: str, trigger: str) -> bool:
    """One letter typed in the Russian layout that is no Russian word of one letter (short_words.TRUSTED_SINGLE_LETTER_WORDS),
    whose Latin reading is a letter too, right after a word of Cyrillic letters and a space, ended by a space or a sign
    (VARIABLE_HEAD_TRIGGERS): a Latin variable typed in the Russian layout in technical prose (`т` for `n` in
    `результат n - 1`, `ч` for `x`), or a letter meant as written (`т.` of `и т. д.`, a word broken off in chat). After a
    word of Latin letters the single-letter head answers (letter_question); with no word before it, or at a pause, the
    frozen model."""

    from .short_words import TRUSTED_SINGLE_LETTER_WORDS

    if (len(original) != 1 or not _cyrillic_letter(original) or len(alternative) != 1 or not alternative.isalpha()
            or original.casefold() in TRUSTED_SINGLE_LETTER_WORDS or not before[-1:].isspace()
            or trigger not in VARIABLE_HEAD_TRIGGERS):
        return False
    words = before.split()
    return bool(words) and all(_cyrillic_letter(char) for char in words[-1])


def _variable_evidence(item: ContextEvidence) -> dict[str, float]:
    """The variable head's evidence: the letter and its case, the boundary (a space, or the sign that ended it), the word
    before it and how often Russian prose uses that word, whether the text before holds signs of code
    (VARIABLE_CODE_SIGNS), Latin letters or digits, and how the word after it was given; the letter and the boundary
    are each read with the rest. In chat such a letter is meant as written (`да я про то ч`), in a question about code it
    is a variable (`для ч = 0.3`)."""

    from .context_model import term_bucket

    tail = item.field.before[-VARIABLE_CODE_CONTEXT_CHARACTERS:]
    previous = item.field.before.split()[-1].casefold()
    following = (item.field.after.split() or [""])[0].casefold()
    letter = f"letter:{item.original.casefold()}"
    boundary = f"boundary:{item.trigger}:{item.boundary_text.strip() if item.trigger == 'punctuation' else ''}"
    rest = (f"case:{'upper' if item.original.isupper() else 'lower'}", f"previous:{term_bucket(previous, 'russian')}",
            f"code:{int(any(char in VARIABLE_CODE_SIGNS for char in tail))}",
            f"latin:{int(any('a' <= char.casefold() <= 'z' for char in tail))}",
            f"digit:{int(any(char.isdigit() for char in tail))}", f"after:{item.after_origin}:{_script(following)}")
    names = ("bias", letter, boundary, f"word:{previous}", *rest, f"{letter}:{boundary}",
             *(f"{head}:{name}" for head in (letter, boundary) for name in rest))
    return {f"{VARIABLE_FEATURE_PREFIX}{name}": 1.0 for name in names}


def sign_question(original: str, alternative: str, before: str, after: str, trigger: str, origin: str) -> bool:
    """A letter of the variable head's class (variable_question) asked once more with the token after it given as kept
    text (after_origin kept_next_word), that token holding no letter and no dash alone (sign_token_asked): a sign, signs or
    digits (`ч =`, `м +`, `ч 2`). The engine asks a kept letter so when a token that reads alike in both layouts follows it
    (_decide_letter_before_sign), and a waiting one before any token kept as typed (_decide_with_kept_neighbour). In a
    question about code the letter is a Latin variable; before a number in Russian prose it is meant as written (`ГОСТ Р
    54960`). The question reads only the letter, the text around it and the origin, as the trainer and the GPU path route
    frames by them."""

    return (origin == "kept_next_word" and sign_token_asked((after.split() or [""])[0])
            and variable_question(original, alternative, before, trigger))


def sign_token_asked(token: str) -> bool:
    """Whether a letter is asked again with this token after it: one holding no letter that is no dash alone (SIGN_DASH)."""

    return bool(token.strip(SIGN_DASH)) and not any(char.isalpha() for char in token)


def _sign_token(after: str) -> str:
    token = (after.split() or [""])[0]
    if token.isdigit():
        return "digits"
    return token if len(token) <= SIGN_TOKEN_MAX_CHARACTERS else "other"


def _sign_kind(after: str) -> str:
    token = (after.split() or [""])[0]
    if token.isdigit():
        return "digits"
    return "operator" if all(char in SIGN_OPERATOR_SIGNS for char in token) else "other"


def _sign_evidence(item: ContextEvidence) -> dict[str, float]:
    """The sign head's evidence, all it reads beside the kept-neighbour question's: the token after the letter itself (up
    to SIGN_TOKEN_MAX_CHARACTERS signs, or digits, or longer) and its kind (an operator, digits, other), the letter, its
    case, how often Russian prose uses the
    word before it, and whether the text before holds signs of code (VARIABLE_CODE_SIGNS), Latin letters or digits, each
    also read with the token and with its kind; the letter alone, as the two Russian `м +` of the Leipzig sentences (`50 кв
    м + балкон`) outweighed every `м` before a formula sign once read with the sign (candidate v76 e3). The variable head
    reads every such token as the same script: `=` after `для ч` went the way of the dash in `е - все то, что`."""

    from .context_model import term_bucket

    tail = item.field.before[-VARIABLE_CODE_CONTEXT_CHARACTERS:]
    previous = item.field.before.split()[-1].casefold()
    token, kind = f"token:{_sign_token(item.field.after)}", f"kind:{_sign_kind(item.field.after)}"
    letter = f"letter:{item.original.casefold()}"
    rest = (f"case:{'upper' if item.original.isupper() else 'lower'}",
            f"previous:{term_bucket(previous, 'russian')}", f"code:{int(any(char in VARIABLE_CODE_SIGNS for char in tail))}",
            f"latin:{int(any('a' <= char.casefold() <= 'z' for char in tail))}",
            f"digit:{int(any(char.isdigit() for char in tail))}")
    names = ("bias", token, kind, letter, *rest, *(f"{head}:{name}" for head in (token, kind) for name in rest))
    return {f"{SIGN_FEATURE_PREFIX}{name}": 1.0 for name in names}


_DOTTED_READING: Final = re.compile(
    rf"[а-яё]{{1,{DOTTED_PART_MAX_LETTERS}}}(?:\.[а-яё]{{1,{DOTTED_PART_MAX_LETTERS}}})+\.?", re.IGNORECASE)


def dotted_question(original: str, alternative: str) -> bool:
    """A token typed in the English layout with `/`, the key of the Russian dot, whose Cyrillic reading is two or more
    groups of up to DOTTED_PART_MAX_LETTERS letters joined by dots: a Russian abbreviation typed in the wrong layout
    (`n/r/` for `т.к.`, `l/,/` for `д.б.`), or a Latin token of short parts meant as typed (`n/a`, `I/O`, `usr/bin`). A
    reading of one group (`cv/` for `см.`, `dev/` for `вум.`) is no member: a word with the key of a dot after it is as
    often a part of a path as a short Russian word ending a sentence, and the frozen model decides it."""

    return original.isascii() and "/" in original and _DOTTED_READING.fullmatch(alternative) is not None


def _rarest_part(parts: list[str], alphabet: str) -> str:
    """The term-table bucket (term_bucket) of the rarest part of a token in one table; a part that is no word at all
    (`,` of `l/,/`, typed for `д.б.`) is rarer than any word: `na`."""

    from .context_model import term_bucket

    buckets = [term_bucket(part, alphabet) for part in parts if part]
    return "na" if "na" in buckets else min(buckets, key=int)


def _dotted_evidence(item: ContextEvidence) -> dict[str, float]:
    """The dotted-abbreviation head's evidence: the Cyrillic reading itself (the abbreviations are few: `т.к.`, `т.е.`,
    `т.д.`, `т.п.`, `д.б.`), the lengths of its parts and its final dot, its case, the script of the word before and
    after it, and how often the parts occur as words: the rarest part of the reading in Russian prose and the rarest
    part typed among the Latin words of Russian technical text (term_bucket). Each part of `т.к.` is a Russian word of
    the table, no part of `гык.ишт` (`usr/bin`) is. The reading, its shape and the two counts are read with each of the
    rest."""

    reading = item.alternative.casefold()
    words = item.field.before.split()
    following = item.field.after.split()
    shape = f"shape:{'-'.join(str(len(part)) for part in reading.split('.') if part)}:{int(reading.endswith('.'))}"
    case = f"case:{'upper' if item.original[:1].isupper() else 'lower'}"
    before = f"before:{_script(words[-1].casefold()) if words else 'start'}"
    after = f"after:{item.after_origin}:{_script(following[0].casefold()) if following else 'none'}"
    russian = _rarest_part(reading.split("."), "russian")
    latin = _rarest_part(item.original.casefold().split("/"), "latin")
    counts = f"russian:{russian}:latin:{latin}"
    names = ("bias", f"reading:{reading}", shape, case, before, after, f"russian:{russian}", f"latin:{latin}", counts,
             *(f"{head}:{name}" for head in (f"reading:{reading}", shape, counts) for name in (case, before, after)))
    return {f"{DOTTED_FEATURE_PREFIX}{name}": 1.0 for name in names}


def letters_and_digits(token: str) -> bool:
    """Letters and ASCII digits alone, at least one of each, the letters all Latin or all Cyrillic."""

    letters = [char for char in token if char.isalpha()]
    return (bool(letters) and any(char.isascii() and char.isdigit() for char in token)
            and all(char.isalpha() or (char.isascii() and char.isdigit()) for char in token)
            and (all(char.isascii() for char in letters) or all(_cyrillic_letter(char) for char in letters)))


def alnum_question(original: str, alternative: str, before: str) -> bool:
    """A token typed in Russian letters and digits, of at most ALNUM_HEAD_MAX_CHARACTERS, whose keys in the English layout
    are Latin letters and digits, that Russian text is known to write as a whole token one way or the other
    (context-alnum-frequency.json counts it as typed or its Latin reading), after text that is Russian or none: a Latin
    token typed in the Russian layout in Russian text (`шзм6` for `ipv6`, `зь2` for `pm2`, `ч64` for `x64`, `2аф` for
    `2fa`), or a Russian one meant as typed (`1с`, `3д`, `100мс`): the letters-and-digits head's class. A reading with a
    sign is none (`2х` reads `2[`), nor a token with a sign inside (`-v2`, `5ю2`): the frozen model decides those, and the
    lone-letter refusal (context_policy) a token of one letter. After English text the frozen model decides too, and for
    a token typed in Latin letters, whose typed questions label `1c` of Russian prose a conversion to `1с`. A token the
    table counts in neither script is the frozen model's as well: fitted on the 2023 dump, where such shapes are Russian
    (`2х`, `ГОСТ2012`), the head kept model names typed in the Russian layout in Russian sentences that the frozen model
    had converted (`В7000` for `D7000`, `К2В2` for `R2D2`, `220ф` for `220a`; corpus v72, candidates v72 and v72b). The
    question reads only the token, its other reading, the text before it and the packaged table, as the trainer and the
    GPU path route frames by them."""

    from .context_model import _alnum_frequency

    if not (len(original) <= ALNUM_HEAD_MAX_CHARACTERS and not original.isascii() and letters_and_digits(original)
            and alternative.isascii() and letters_and_digits(alternative)
            and _dominant(before[-ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS:].casefold()) != "en"):
        return False
    tokens = _alnum_frequency()
    return bool(tokens["cyrillic"].get(original.casefold()) or tokens["latin"].get(alternative.casefold()))


def _alnum_shape(token: str) -> str:
    """The runs of letters (`L`) and digits (`D`) of a token, each with its length up to ALNUM_RUN_LENGTH_MAX: `м67` is
    `L1D2`, `ц4и` `L1D1L1`, `100мс` `D3L2`; a token of more than ALNUM_SHAPE_MAX_RUNS runs is `many`."""

    runs = [(kind, len(list(group))) for kind, group in itertools.groupby(token, key=lambda char: "D" if char.isdigit() else "L")]
    if len(runs) > ALNUM_SHAPE_MAX_RUNS:
        return "many"
    return "".join(f"{kind}{min(size, ALNUM_RUN_LENGTH_MAX)}" for kind, size in runs)


def _ratio(cyrillic: int, latin: int) -> int:
    """The binary order of magnitude by which a Cyrillic count outnumbers a Latin one, ALNUM_RATIO_BOUND at most either way."""

    return max(-ALNUM_RATIO_BOUND, min(ALNUM_RATIO_BOUND, cyrillic.bit_length() - latin.bit_length()))


def _alnum_evidence(item: ContextEvidence) -> dict[str, float]:
    """The letters-and-digits head's evidence: how often Russian text writes the token as typed and its Latin reading
    as whole tokens (context-alnum-frequency.json: `1с` 1 363 times and `1c` 134, `ipv6` 351 and `шзм6` never), each as a
    bucket of the term tables' bounds, with the binary order of magnitude by which one outnumbers the other
    (ALNUM_RATIO_BOUND at most either way); the same order for the longest run of letters of each reading in the term
    tables (Russian text for the Cyrillic run, English prose or Russian technical text for the Latin one) and whether
    either occurs; its shape (runs of letters and digits), the case of its letters, the script of the text and of the word
    before it, whether that text holds signs of code (VARIABLE_CODE_SIGNS) or digits, the boundary and how the word after
    it was given; the shape and both orders are read with the rest. The shared features read every digit as one class:
    `1С` became `1C` in Russian prose and `2фа` `2af`, while `м67` for `v67` stayed a suggestion and `x64` typed in the
    Russian layout stayed `ч64` (the owner's typing, ru.stackoverflow messages of 2023)."""

    from .context_model import _alnum_frequency, _term_frequency

    table, tokens = _term_frequency(), _alnum_frequency()
    cyrillic, latin = item.original.casefold(), item.alternative.casefold()
    typed, other = tokens["cyrillic"].get(cyrillic, 0), tokens["latin"].get(latin, 0)
    token = (f"token:{bisect.bisect_right(CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS, typed)}:"
             f"{bisect.bisect_right(CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS, other)}")
    whole = f"whole:{_ratio(typed, other)}"
    cyrillic_letters = max(re.findall("[а-яё]+", cyrillic), key=len)
    latin_letters = max(re.findall("[a-z]+", latin), key=len)
    russian = max(table["russian"].get(cyrillic_letters, 0), table["cyrillic"].get(cyrillic_letters, 0))
    english = max(table["english"].get(latin_letters, 0), table["latin"].get(latin_letters, 0))
    ratio = f"ratio:{_ratio(russian, english)}"
    seen = f"seen:{int(russian > 0)}:{int(english > 0)}"
    shape = f"shape:{_alnum_shape(item.original)}"
    letters = [char for char in item.original if char.isalpha()]
    case = f"case:{'upper' if all(char.isupper() for char in letters) else 'lower' if not any(char.isupper() for char in letters) else 'mixed'}"
    tail = item.field.before[-VARIABLE_CODE_CONTEXT_CHARACTERS:]
    words = item.field.before.split()
    following = (item.field.after.split() or [""])[0].casefold()
    before = f"before:{_dominant(tail.casefold())}:{_script(words[-1].casefold()) if words else 'start'}"
    code = f"code:{int(any(char in VARIABLE_CODE_SIGNS for char in tail))}:digit:{int(any(char.isdigit() for char in tail))}"
    boundary = f"boundary:{item.trigger}:{item.boundary_text.strip() if item.trigger == 'punctuation' else ''}"
    after = f"after:{item.after_origin}:{_script(following)}"
    rest = (case, before, code, boundary, after)
    names = ("bias", token, whole, shape, ratio, seen, *rest, f"{shape}:{whole}", f"{shape}:{ratio}", f"{whole}:{ratio}",
             f"{ratio}:{seen}", *(f"{head}:{name}" for head in (shape, whole, ratio) for name in rest))
    return {f"{ALNUM_FEATURE_PREFIX}{name}": 1.0 for name in names}


@functools.cache
def _russian_endings() -> dict[str, int]:
    """The endings of TERM_HEAD_ENDING_LETTERS letters of the words of Russian prose the term table counts, with how many
    words end with each (a word longer than its ending); built once, on the first term the head is asked about."""

    from .context_model import _term_frequency

    endings: dict[str, int] = {}
    for word in _term_frequency()["russian"]:
        for size in TERM_HEAD_ENDING_LETTERS:
            if len(word) > size:
                endings[word[-size:]] = endings.get(word[-size:], 0) + 1
    return endings


@functools.cache
def _russian_starts() -> dict[str, int]:
    """The beginnings of TERM_HEAD_ENDING_LETTERS letters of the words of Russian prose the term table counts, with how
    many words begin with each (a word longer than its beginning); built once, on the first token asked about."""

    from .context_model import _term_frequency

    starts: dict[str, int] = {}
    for word in _term_frequency()["russian"]:
        for size in TERM_HEAD_ENDING_LETTERS:
            if len(word) > size:
                starts[word[:size]] = starts.get(word[:size], 0) + 1
    return starts


def brief_question(original: str, before: str, after: str) -> bool:
    """A token of BRIEF_HEAD_LETTERS Latin letters with no letter before or after it in the field, whose keys in the
    Russian layout are Russian letters: a short message typed in the English layout, `ytn` for `нет`, `jr` for `ок`,
    `tot` for `еще`. The message-start head starts at four letters, because at three it turned Russian words typed as
    meant into Latin (`чук`, `дес`): this head answers Latin letters alone, and a Russian token stays with the frozen
    model. The question reads only the typed token and the text around it, as the trainer and the GPU path route frames
    by them (head_question, context_action_cuda.heads_question)."""

    from .layouts import LayoutPair

    if not (len(original) in BRIEF_HEAD_LETTERS and original.isascii() and original.isalpha()):
        return False
    cyrillic = LayoutPair().translate(original, "us", "ru")
    return (cyrillic.isalpha() and not any(char.isascii() for char in cyrillic)
            and not any(char.isalpha() for char in before) and not any(char.isalpha() for char in after))


def brief_settled(latin: str, cyrillic: str) -> bool:
    """Whether the counts settle a short Latin token as the keys of a Russian word: its Cyrillic reading counted in
    Russian text at least BRIEF_SETTLED_MIN_COUNT times and BRIEF_SETTLED_DOMINANCE times as often as the token in
    English prose or Russian technical text, where it occurs fewer than BRIEF_SETTLED_OTHER_MAXIMUM times (`ytn` and
    `нет`, `gy` and `пн`), as context_deferral.counted_reading settles the calibration's lone words with the Latin count
    of English prose added. `ds` (`вы`, 108 Latin counts), `ij` (`шо`, 84 against 29) are not settled."""

    from .context_model import _term_frequency

    table = _term_frequency()
    latin, cyrillic = latin.casefold(), cyrillic.casefold()
    russian = max(table["russian"].get(cyrillic, 0), table["cyrillic"].get(cyrillic, 0))
    other = max(table["english"].get(latin, 0), table["latin"].get(latin, 0))
    return (russian >= BRIEF_SETTLED_MIN_COUNT and other < BRIEF_SETTLED_OTHER_MAXIMUM
            and russian >= BRIEF_SETTLED_DOMINANCE * other)


def _brief_evidence(item: ContextEvidence) -> dict[str, float]:
    """The short-message head's evidence: the binary order of magnitude by which how often Russian prose uses the
    Cyrillic reading outnumbers how often English prose or Russian technical text uses the Latin one (BRIEF_RATIO_BOUND
    at most either way), whether either reading occurs at all, the boundary that ended the token and its case. The
    orders of each count on their own told few tokens apart, and the head learnt them by heart: its development loss
    rose from the second epoch on, and it turned `ye` sent alone, which the frozen model converted to `ну`, into a
    suggestion (experiment E9)."""

    from .context_model import _term_frequency

    table = _term_frequency()
    latin, cyrillic = item.original.casefold(), item.alternative.casefold()
    russian = table["russian"].get(cyrillic, 0).bit_length()
    other = max(table["english"].get(latin, 0), table["latin"].get(latin, 0)).bit_length()
    ratio = max(-BRIEF_RATIO_BOUND, min(BRIEF_RATIO_BOUND, russian - other))
    seen = f"seen:{int(russian > 0)}:{int(other > 0)}"
    case = "upper" if item.original.isupper() else "title" if item.original[:1].isupper() else "lower"
    names = ("bias", f"ratio:{ratio}", seen, f"trigger:{item.trigger}", f"case:{case}", f"ratio:{ratio}:trigger:{item.trigger}",
             f"ratio:{ratio}:case:{case}", f"{seen}:trigger:{item.trigger}")
    return {f"{BRIEF_FEATURE_PREFIX}{name}": 1.0 for name in names}


_RUSSIAN_LETTERS: Final = "абвгдеёжзийклмнопрстуфхцчшщъыьэюя"
_ENGLISH_LETTERS: Final = "abcdefghijklmnopqrstuvwxyz"
# Signs stripped off both ends of a reading before it is looked up, as the term tables were counted.
_EDGE_SIGNS: Final = ".,!?:;\"'()[]{}<>«»-"


def _magnitude(count: int, bound: int) -> int:
    return min(count.bit_length(), bound)


def nearest_count(word: str, language: str) -> int:
    """How often the commonest word one edit from `word` occurs in the prose of its language (the term table's `russian`
    or `english`): a letter missed, one extra, one wrong, or two neighbours swapped; 0 for a word of anything but letters.
    `тыв` is `ты` with a letter extra, which Russian prose uses 99 523 times. The words a word of three letters or more is
    one typo from are read for the readings of the hypothesis question only: almost every short string has some neighbour,
    and how common the commonest is tells a typo of a common word from a string of keys."""

    from .context_model import _term_frequency

    table = _term_frequency()[language]
    letters = _RUSSIAN_LETTERS if language == "russian" else _ENGLISH_LETTERS
    word = word.strip(_EDGE_SIGNS).casefold()
    if not word or not all(char in letters for char in word):
        return 0
    best = 0
    for index in range(len(word) + 1):
        head, tail = word[:index], word[index:]
        candidates = [head + letter + tail for letter in letters]
        if tail:
            candidates.append(head + tail[1:])
            candidates += [head + letter + tail[1:] for letter in letters if letter != tail[0]]
            swapped = tail[1:][:1] + tail[:1]
            if len(swapped) > 1 and swapped != tail[:len(swapped)]:
                candidates.append(head + swapped + tail[len(swapped):])
        best = max(best, max(table.get(candidate, 0) for candidate in candidates))
    return best


def _prose_count(word: str, language: str) -> int:
    from .context_model import _term_frequency

    return _term_frequency()[language].get(word.strip(_EDGE_SIGNS).casefold(), 0)


def hypothesis_question(item: ContextEvidence) -> bool:
    """The question the engine asks about the next word of a waiting word with that word's other reading before it
    (engine._decide_after_waiting_word): the hypothesis head's class. Only the question says so (before_origin); the same
    text before a word is typed when the layout was switched by hand after a word of the other script, and that word is
    the frozen model's."""

    return item.before_origin == "planned_previous_reading"


def _hypothesis_evidence(item: ContextEvidence) -> dict[str, float]:
    """The hypothesis head's evidence, all it reads (no copy of the shared features, which read the reading before the
    word as typed text: `before:script:en` and the identifier `nsd` turned `тыв` after `tot ` into `nsd`): by how many
    binary orders of magnitude the prose of each reading's language uses the word as typed more than the other reading,
    and the commonest word one edit from each (nearest_count) more than the other's; whether either reading is a known
    identifier; by how much the waiting word as typed is commoner in its language than the reading put before the word
    (`еще` against `tot`); the token's letters, the band of the detector's score delta and the boundary."""

    from .layouts import LayoutPair

    typed_language, other_language = ("russian", "english") if item.source_group == 1 else ("english", "russian")
    typed_count = _prose_count(item.original, typed_language)
    other_count = _prose_count(item.alternative, other_language)
    typed_near = max(typed_count, nearest_count(item.original, typed_language))
    other_near = max(other_count, nearest_count(item.alternative, other_language))
    words = _WORDS.findall(item.field.before)
    reading = words[-1] if words else ""
    waiting = LayoutPair().translate(reading, "us", "ru") if item.source_group == 1 else LayoutPair().translate(reading, "ru", "us")

    def ratio(first: int, second: int) -> int:
        difference = _magnitude(first, HYPOTHESIS_MAGNITUDE_MAX) - _magnitude(second, HYPOTHESIS_MAGNITUDE_MAX)
        return max(-HYPOTHESIS_RATIO_BOUND, min(HYPOTHESIS_RATIO_BOUND, difference))

    word = f"word:{ratio(typed_count, other_count)}"
    near = f"near:{ratio(typed_near, other_near)}"
    seen = f"seen:{int(typed_count > 0)}:{int(other_count > 0)}"
    identifier = f"identifier:{int(item.source_identifier)}:{int(item.target_identifier)}"
    held = f"waiting:{ratio(_prose_count(waiting, typed_language), _prose_count(reading, other_language))}"
    letters = f"letters:{min(sum(char.isalpha() for char in item.original), ACTION_FEATURE_LENGTH_BUCKET_MAX_CHARACTERS)}"
    band = max(-START_DELTA_BANDS, min(START_DELTA_BANDS, math.floor(item.score_delta / START_DELTA_BAND_WIDTH)))
    boundary = f"boundary:{'space' if item.trigger == 'space' else 'end'}"
    names = ("bias", word, near, seen, identifier, held, letters, f"delta:{band}", boundary, f"{word}:{near}",
             f"{near}:{identifier}", f"{near}:{held}", f"{near}:{letters}", f"{seen}:{identifier}")
    return {f"{HYPOTHESIS_FEATURE_PREFIX}{name}:direction:{item.source_group}": 1.0 for name in names}


def short_question(original: str, before: str) -> bool:
    """A token of SHORT_HEAD_LETTERS Russian letters, not all capitals, whose keys are Latin letters, after text with more
    Cyrillic letters than Latin ones, that Russian text uses at least in bucket SHORT_HEAD_MIN_RUSSIAN_BUCKET (web and news
    text, prose or technical text) while English prose and Russian technical text use its Latin reading at most in bucket
    SHORT_HEAD_MAX_LATIN_BUCKET: the short-token head's class (the lexicon's own words are left out where the features
    are made). `кз` (68 in web and news text, `rp` 23 in technical text) and `дз` are; `ьк` (`mr`), `ин` (`by`), `сс`
    (`cc`) and `ср` (`ch`), whose Latin readings are common, are the frozen model's, which converted them as meant, and so
    is a token Russian text hardly uses (`ге`, `сщ`). Capitals are the abbreviation head's, a word of one letter the
    letter heads', a term of Russian technical text the term head's. The question reads only the typed token, the text
    before it and the packaged tables, as the trainer and the GPU path route frames by them (head_question,
    context_action_cuda.heads_question)."""

    from .context_model import _short_frequency, _term_frequency
    from .layouts import LayoutPair

    if not (len(original) in SHORT_HEAD_LETTERS and all(_cyrillic_letter(char) for char in original) and not original.isupper()):
        return False
    latin = LayoutPair().translate(original, "ru", "us")
    if not (latin.isascii() and latin.isalpha()
            and _dominant(before[-ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS:].casefold()) == "ru"):
        return False
    table, cyrillic = _term_frequency(), original.casefold()
    russian = max(_short_frequency()["cyrillic"].get(cyrillic, 0), table["russian"].get(cyrillic, 0),
                  table["cyrillic"].get(cyrillic, 0))
    other = max(table["latin"].get(latin.casefold(), 0), table["english"].get(latin.casefold(), 0))
    return (bisect.bisect_right(CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS, russian) >= SHORT_HEAD_MIN_RUSSIAN_BUCKET
            and bisect.bisect_right(CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS, other) <= SHORT_HEAD_MAX_LATIN_BUCKET
            and not term_question(original, before))


def _short_evidence(item: ContextEvidence) -> dict[str, float]:
    """The short-token head's evidence, all it reads: the binary order of magnitude of how often Russian web and news
    text writes the token (a table of its own, context-short-frequency.json), Russian prose and technical text write it,
    Russian technical text writes its Latin reading and English prose uses it, and by how much the larger Russian count
    outnumbers the larger Latin one; whether the English lexicon knows the Latin reading and the identifier index holds it
    (`rp` is a word to it); the token's letters, the boundary, how the text after it was given and the band of the
    detector's score delta. No letter of its own: an abbreviation head reading the letters learned the corpus's tokens by
    heart and turned `ТЗ` into `NP`."""

    from .context_model import _short_frequency, _term_frequency

    table = _term_frequency()
    cyrillic, latin = item.original.casefold(), item.alternative.casefold()
    web = _short_frequency()["cyrillic"].get(cyrillic, 0)
    russian = max(table["russian"].get(cyrillic, 0), table["cyrillic"].get(cyrillic, 0))
    technical, english = table["latin"].get(latin, 0), table["english"].get(latin, 0)
    magnitudes = {name: _magnitude(count, SHORT_MAGNITUDE_MAX)
                  for name, count in (("web", web), ("russian", russian), ("latin", technical), ("english", english))}
    difference = max(magnitudes["web"], magnitudes["russian"]) - max(magnitudes["latin"], magnitudes["english"])
    ratio = f"ratio:{max(-SHORT_RATIO_BOUND, min(SHORT_RATIO_BOUND, difference))}"
    known = f"known:{int(item.target_known)}:identifier:{int(item.target_identifier)}"
    letters = f"letters:{len(item.original)}"
    seen = "seen:" + ":".join(str(int(count > 0)) for count in (web, russian, technical, english))
    band = max(-START_DELTA_BANDS, min(START_DELTA_BANDS, math.floor(item.score_delta / START_DELTA_BAND_WIDTH)))
    boundary = f"boundary:{'space' if item.trigger == 'space' else 'end'}"
    after = f"after:{item.after_origin}"
    names = ("bias", *(f"{name}:{value}" for name, value in magnitudes.items()), ratio, seen, known, letters, boundary,
             after, f"delta:{band}", f"{ratio}:{known}", f"{ratio}:{letters}", f"{ratio}:{boundary}",
             f"web:{magnitudes['web']}:latin:{magnitudes['latin']}", f"{seen}:{known}")
    return {f"{SHORT_FEATURE_PREFIX}{name}": 1.0 for name in names}


def slip_question(original: str, before: str) -> bool:
    """A token of at least SLIP_HEAD_MIN_LETTERS Latin letters after a line with more Cyrillic letters than Latin ones,
    whose Latin reading English prose and Russian technical text use at most in bucket SLIP_HEAD_MAX_LATIN_BUCKET and
    whose Cyrillic reading Russian prose uses: the slip head's class (a word the lexicon knows or the identifier index
    holds is left out where the features are made). It is a word of Russian prose typed with the English layout on, the
    layout having slipped with no language boundary (`cgtie` for `спешу` after `я `); a term Russian technical text
    writes in Latin letters (`gtk`, `env`) stays with the frozen model. The question reads only the typed token, the text
    before it and the packaged term table, as the trainer and the GPU path route frames by them (head_question,
    context_action_cuda.heads_question)."""

    from .context_model import _term_frequency, term_bucket
    from .layouts import LayoutPair

    if not (len(original) >= SLIP_HEAD_MIN_LETTERS and original.isascii() and original.isalpha()):
        return False
    line = before.rsplit("\n", 1)[-1]
    if _dominant(line[-ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS:].casefold()) != "ru":
        return False
    cyrillic = LayoutPair().translate(original, "us", "ru").casefold()
    return (all(_cyrillic_letter(char) for char in cyrillic) and _term_frequency()["russian"].get(cyrillic, 0) > 0
            and int(term_bucket(original, "latin")) <= SLIP_HEAD_MAX_LATIN_BUCKET
            and int(term_bucket(original, "english")) <= SLIP_HEAD_MAX_LATIN_BUCKET)


def _slip_evidence(item: ContextEvidence) -> dict[str, float]:
    """The slip head's evidence, all it reads (no copy of the shared features, which hold the frozen model's keep weights
    for unknown Latin tokens after Russian prose): by how many binary orders of magnitude Russian prose uses the Cyrillic
    reading more than English prose or Russian technical text use the Latin one, and the commonest word one edit from each
    more than the other's (nearest_count); whether the Russian lexicon knows the Cyrillic reading; the detector's verdict
    and the band of its probability; the band of the orthotactic margin; how often Russian prose uses the word before the
    token (`я` against a technical word); the token's letters and case, the band of the score delta and how the text after
    it was given. Nothing of the boundary: the same word asked at a pause and at its space gets the same answer."""

    from .context_model import _term_frequency

    table = _term_frequency()
    latin, cyrillic = item.original.casefold(), item.alternative.casefold()
    russian = table["russian"].get(cyrillic, 0)
    english = max(table["english"].get(latin, 0), table["latin"].get(latin, 0))
    russian_near = max(russian, nearest_count(cyrillic, "russian"))
    english_near = max(english, nearest_count(latin, "english"))

    def ratio(first: int, second: int) -> int:
        difference = _magnitude(first, HYPOTHESIS_MAGNITUDE_MAX) - _magnitude(second, HYPOTHESIS_MAGNITUDE_MAX)
        return max(-HYPOTHESIS_RATIO_BOUND, min(HYPOTHESIS_RATIO_BOUND, difference))

    probability = ("na" if item.model_probability is None
                   else str(min(SLIP_PROBABILITY_BANDS - 1, max(0, math.floor(item.model_probability * SLIP_PROBABILITY_BANDS)))))
    margin = ("na" if item.ortho_score is None or item.ortho_threshold is None
              else str(max(-SLIP_ORTHO_BANDS, min(SLIP_ORTHO_BANDS,
                                                  math.floor((item.ortho_score - item.ortho_threshold) / SLIP_ORTHO_BAND_WIDTH)))))
    words = _WORDS.findall(item.field.before.rsplit("\n", 1)[-1])
    previous = _magnitude(table["russian"].get(words[-1].casefold(), 0), HYPOTHESIS_MAGNITUDE_MAX) if words else 0
    word = f"word:{ratio(russian, english)}"
    near = f"near:{ratio(russian_near, english_near)}"
    known = f"known:{int(item.target_known)}"
    detector = f"detector:{int(item.baseline_convert)}:{probability}"
    ortho = f"ortho:{margin}"
    before = f"before:{previous // SLIP_BEFORE_MAGNITUDE_STEP}"
    letters = f"letters:{min(len(item.original), ACTION_FEATURE_LENGTH_BUCKET_MAX_CHARACTERS)}"
    band = max(-START_DELTA_BANDS, min(START_DELTA_BANDS, math.floor(item.score_delta / START_DELTA_BAND_WIDTH)))
    case = "case:" + ("upper" if item.original.isupper() else "title" if item.original[:1].isupper() else "lower")
    names = ("bias", word, near, known, detector, ortho, before, letters, f"delta:{band}", case, f"after:{item.after_origin}",
             f"{word}:{near}", f"{word}:{known}", f"{near}:{detector}", f"{word}:{before}", f"{near}:{letters}",
             f"{ortho}:{known}")
    return {f"{SLIP_FEATURE_PREFIX}{name}": 1.0 for name in names}
