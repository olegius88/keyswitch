"""Bounded experimental action features shared by training and inference."""

from __future__ import annotations

import math
import re
import unicodedata
from typing import TYPE_CHECKING, Final
from .constants.models import (
    ABBREVIATION_FEATURE_PREFIX,
    ABBREVIATION_NON_INITIAL_LETTERS,
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
    CAPITALS_FEATURE_PREFIX,
    CAPITALS_HEAD_MAX_LETTERS,
    CAPITALS_HEAD_MIN_LETTERS,
    KEPT_CONTEXT_WORD_MAX_CHARACTERS,
    KEPT_FEATURE_PREFIX,
    LATIN_ABBREVIATION_FEATURE_PREFIX,
    LETTER_FEATURE_PREFIX,
    PLANNED_CONTEXT_AFTER_MAX_CHARACTERS,
    PLANNED_CONTEXT_WORD_MAX_CHARACTERS,
    START_DELTA_BAND_WIDTH,
    START_DELTA_BANDS,
    START_FEATURE_PREFIX,
    START_HEAD_MIN_LETTERS,
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

    from .context_model import term_bucket

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
    latin_bucket = term_bucket(latin, "latin")
    cyrillic_bucket = "lexicon" if cyrillic_known else term_bucket(cyrillic, "cyrillic")
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

    from .context_model import term_bucket

    letters = item.original
    if len(letters) < ACTION_FEATURE_CAPITALS_MIN_LETTERS or not letters.isalpha() or not letters.isupper():
        return
    russian = term_bucket(cyrillic, "russian")
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

    from .context_model import term_bucket

    before, after, state = surroundings
    latin, cyrillic = readings
    latin_bucket, cyrillic_bucket = buckets
    planned = item.after_origin == "planned_next_conversion"
    context = f"before:{_dominant(before)}:{'next' if planned else 'after'}:{_dominant(after)}"
    if not item.inside and (before.strip() or after.strip()):
        english_bucket = term_bucket(latin, "english")
        russian_bucket = term_bucket(cyrillic, "russian")
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
        return kept
    result = {name: value for name, value in features.items() if value}
    if capitals_question(item.original, item.field.before):
        # The class the capitals head answers: its features once more under their own names, beside
        # the shared ones, so a head fitted onto a frozen model moves no decision outside the class.
        # `BP` in English prose read `ИЗ` at p=0.997, and the frames that kept it moved the weights
        # the Latin keys of `не` and `что` share (`yt`, `xnj` stayed as typed).
        result.update({CAPITALS_FEATURE_PREFIX + name: value for name, value in list(result.items())})
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
