"""Small local, learned contextual action policy, separate from Layout Intent.

The model is a four-class sparse softmax classifier. It receives the recent
sentence, application/field evidence and the existing detector's lexical
evidence. The context-v1 trainer produces feature7 weights; feature2, feature5
and feature6 artifacts still load, and the context action trainer produces
feature3 weights. Probabilities are corpus scores, not a promise of
real-world correctness. Hard safety/explicit user intent live in the engine.
"""

from __future__ import annotations

import bisect
import hashlib
import json
import math
import re
import unicodedata
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Final, Literal, cast

from .input_context import FieldContext
from .language_model import LanguageModel, WordScore
from .context_action_features import extract_action_features
from .constants.boundary import BOUNDARY_MISSING_LETTERS as ALPHABET_LETTERS
from .constants.file_formats import MAX_CONTEXT_MODEL_BYTES as MAX_ARTIFACT_BYTES, MAX_CONTEXT_TERM_FREQUENCY_BYTES
from .constants.models import (
    ACTION_FEATURE_AFTER_CONTEXT_CHARACTERS,
    ACTION_FEATURE_APPLICATION_NAME_CHARACTERS,
    ACTION_FEATURE_APPLICATION_TOKEN_COUNT,
    ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS,
    ACTION_FEATURE_CHARACTER_TEXT_FIELD_INDEX,
    ACTION_FEATURE_LENGTH_BUCKET_MAX_CHARACTERS,
    ACTION_FEATURE_WORD_MAX_CHARACTERS,
    ACTION_FEATURE_WORD_SCORE_BOUND,
    CONTEXT_ACTION_FEATURE_VERSION,
    CONTEXT_DOUBTFUL_KEEP_PROBABILITY,
    CONTEXT_FEATURE_AFTER_WORD_COUNT,
    CONTEXT_FEATURE_BEFORE_WORD_COUNT,
    CONTEXT_FEATURE_NGRAM_ORDERS,
    CONTEXT_MODEL_FEATURE_VERSION as FEATURE_VERSION,
    CONTEXT_SUPPORTED_FEATURE_VERSIONS as SUPPORTED_FEATURE_VERSIONS,
    CONTEXT_OPENING_FEATURE_VERSION,
    CONTEXT_LINE_FEATURE_VERSION,
    CONTEXT_OPENING_SCHEMAS,
    CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS,
    CONTEXT_TYPO_FEATURE_VERSION,
    CONTEXT_TYPO_MIN_CHARACTERS,
    CONTEXT_V1_CONVERSION_THRESHOLD,
    KEPT_FEATURE_PREFIX,
    LETTER_FEATURE_PREFIX,
    MAX_CONTEXT_ACTION_MODEL_FEATURES,
    MAX_CONTEXT_FEATURE_NAME_CHARACTERS,
    MAX_CONTEXT_MODEL_FEATURES as MAX_FEATURES,
    MAX_CONTEXT_MODEL_VERSION_CHARACTERS,
    MAX_CONTEXT_WEIGHT_MAGNITUDE,
    MIN_CONTEXT_CONVERSION_THRESHOLD,
)

# FEATURE_VERSION, MAX_ARTIFACT_BYTES and MAX_FEATURES keep the names this module has always
# exported (the historical context-v2 comparison reads them); their values live in the constants.
__all__ = [
    "ACTIONS", "ARTIFACT_PATH", "FEATURE_VERSION", "MAX_ARTIFACT_BYTES", "MAX_FEATURES",
    "AfterOrigin", "ContextAction", "ContextEvidence", "ContextModel", "ContextPrediction",
    "TECHNICAL_MARKS", "extract_context_features", "one_typo_from_word", "softmax",
]

ContextAction = Literal["keep", "convert", "wait", "suggest"]
AfterOrigin = Literal["none", "field", "planned_next_conversion", "kept_next_word"]
ACTIONS: Final[tuple[ContextAction, ...]] = ("keep", "convert", "wait", "suggest")
ARTIFACT_PATH = Path(__file__).parent / "resources" / "models" / "context_policy_v1.json"
_WORDS = re.compile(r"[^\W\d_]+(?:['’\-][^\W\d_]+)*", re.UNICODE)
# Characters that mark a path, an address or an identifier inside a token.
TECHNICAL_MARKS: Final = "_/@\\=<>"
_LATIN_DOT: Final = re.compile(r"[A-Za-z]\.[A-Za-z]")
# Signs stripped off both ends of a token before its frequency is looked up, as when it was counted.
_TERM_EDGE_SIGNS: Final = ".,!?:;\"'()[]{}<>«»-"
# How often plain words occur (schema 7), read on first use: `latin` and `cyrillic` count words inside
# Russian text, `english` and `russian` words in text of their own language.
TERM_FREQUENCY_PATH: Final = Path(__file__).with_name("resources") / "models" / "context-term-frequency.json"
TERM_ALPHABETS: Final = ("cyrillic", "english", "latin", "russian")
_TERM_FREQUENCY: dict[str, dict[str, int]] | None = None


@dataclass(frozen=True)
class ContextEvidence:
    original: str
    alternative: str
    source_group: int
    field: FieldContext
    trigger: str = "space"
    baseline_convert: bool = False
    source_known: bool = False
    target_known: bool = False
    score_delta: float = 0.0
    source_score: WordScore | None = None
    target_score: WordScore | None = None
    model_probability: float | None = None
    model_threshold: float | None = None
    literal_tail: str = ""
    ortho_score: float | None = None
    ortho_threshold: float | None = None
    boundary_text: str = ""
    after_origin: AfterOrigin = "none"
    source_identifier: bool = False
    target_identifier: bool = False
    # Whether the reading is no word itself but one typo away from one (`one_typo_from_word`).
    source_typo: bool = False
    target_typo: bool = False
    # Whether the reading is a word that opens sentences of its language (short_words.opens_sentences).
    source_opening: bool = False
    target_opening: bool = False
    # Whether letters were typed into the middle of a word whose other letters are all in the
    # other layout (engine._decide_inside_word): the question is then what the whole word is.
    inside: bool = False

    def __post_init__(self) -> None:
        # Replacing literal field contents must not retain a stale empty flag.
        # A planned conversion is explicit and remains subject to validation.
        if self.after_origin in ("none", "field"):
            object.__setattr__(self, "after_origin", "field" if self.field.after else "none")


@dataclass(frozen=True)
class ContextPrediction:
    action: ContextAction
    probability: float
    probabilities: tuple[float, ...]
    model_version: str
    supported: bool


def _normalized(text: str) -> str:
    return unicodedata.normalize("NFC", text.casefold())


def one_typo_from_word(text: str, model: LanguageModel) -> bool:
    """A reading that is no word of the lexicon but one typo away from one.

    One letter extra, missing or wrong, or two neighbours swapped: `фпривет` is
    `привет` with a stray key in front. Only the frequency lexicon is read, as
    for the boundary model's misspellings, so the check costs dictionary
    lookups and no spell checker calls. Short readings are not checked: almost
    every short string has a neighbour in a lexicon this large.
    """

    word = LanguageModel.normalize(text)
    if (len(word) < CONTEXT_TYPO_MIN_CHARACTERS or word != text.casefold() or not word.isalpha()
            or model.frequencies.get(word, 0)):
        return False
    known = model.frequencies
    letters = ALPHABET_LETTERS.get(model.locale, "")
    for index, current in enumerate(word):
        head, tail = word[:index], word[index + 1:]
        if known.get(head + tail, 0):
            return True
        if tail and known.get(head + tail[:1] + current + tail[1:], 0):
            return True
        if any(letter != current and known.get(head + letter + tail, 0) for letter in letters):
            return True
    return any(known.get(word[:index] + letter + word[index:], 0)
               for index in range(len(word) + 1) for letter in letters)


def extract_context_features(item: ContextEvidence, feature_version: int = FEATURE_VERSION) -> dict[str, float]:
    """Bounded features shared verbatim by training and serving.

    Schema 5 adds the typo evidence; schema 6 adds whether a reading opens sentences, the
    next word the engine plans to convert as text of its own kind, a path separator right
    before the token, and whether the word is being edited in place; schema 7 adds where
    the word stands on its line, its case, a dot or underscore inside the Latin reading,
    how often each reading occurs, and whether the token has letters at all.
    """

    original = _normalized(item.original[:ACTION_FEATURE_WORD_MAX_CHARACTERS])
    alternative = _normalized(item.alternative[:ACTION_FEATURE_WORD_MAX_CHARACTERS])
    before = _normalized(item.field.before[-ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS:])
    after = _normalized(item.field.after[:ACTION_FEATURE_AFTER_CONTEXT_CHARACTERS])
    length = min(ACTION_FEATURE_LENGTH_BUCKET_MAX_CHARACTERS, max(len(original), len(alternative)))
    direction = str(item.source_group)
    baseline = str(int(item.baseline_convert))
    features: dict[str, float] = {
        "bias": 1.0, f"baseline:{baseline}": 1.0,
        f"direction:{direction}": 1.0,
        f"length:{length}": 1.0,
        f"baseline:{baseline}:length:{length}": 1.0,
        f"role:{item.field.role}": 1.0,
        f"trigger:{item.trigger}": 1.0,
        f"known:{int(item.source_known)}:{int(item.target_known)}": 1.0,
        "score_delta": max(-ACTION_FEATURE_WORD_SCORE_BOUND, min(ACTION_FEATURE_WORD_SCORE_BOUND, item.score_delta))
        / ACTION_FEATURE_WORD_SCORE_BOUND,
    }
    application = item.field.application.casefold()[:ACTION_FEATURE_APPLICATION_NAME_CHARACTERS]
    for part in re.findall(r"[a-z0-9]+", application)[:ACTION_FEATURE_APPLICATION_TOKEN_COUNT]:
        features[f"app:{part}"] = 1.0
        features[f"app:{part}:length:{length}"] = 1.0
    scripts: list[str] = []
    # Schema 6 tells the text after the caret from the next word the engine plans to
    # convert (a waiting word asked with its neighbour, a converted word asked again):
    # the two are different evidence, and as one feature family the rows where `ns`
    # before an English next word stays taught every Latin token with nothing after it
    # to lean towards converting - a correct `if` after English text reached 0.98
    # (28.09.2026).
    planned = feature_version in CONTEXT_OPENING_SCHEMAS and item.after_origin == "planned_next_conversion"
    for label, text in (("before", before), ("next" if planned else "after", after)):
        words = _WORDS.findall(text)
        words = words[-CONTEXT_FEATURE_BEFORE_WORD_COUNT:] if label == "before" else words[:CONTEXT_FEATURE_AFTER_WORD_COUNT]
        ru = sum("а" <= char <= "я" or char == "ё" for char in text)
        en = sum("a" <= char <= "z" for char in text)
        dominant = "ru" if ru > en else "en" if en > ru else "none"
        scripts.append(("next-" if label == "next" else "") + dominant)
        features[f"{label}:script:{dominant}:length:{length}"] = 1.0
        features[f"{label}:script:{dominant}:direction:{direction}"] = 1.0
        features[f"{label}:script:{dominant}:baseline:{baseline}"] = 1.0
        for word in words:
            features[f"{label}:word:{word}"] = 1.0
        neighbour = words[-1] if label == "before" and words else words[0] if words else ""
        for token, sign in ((original, -1.0), (alternative, 1.0)):
            if neighbour:
                pair = f"pair:{label}:{neighbour}:{token}"
                features[pair] = features.get(pair, 0.0) + sign
        if label == "before":
            features["before:code"] = float(any(mark in text for mark in ("=", "(`", "::", "=>", "{", "```")))
            features["before:comment"] = float(any(mark in text for mark in ("//", "#", "/*")))
    for token, sign in ((original, -1.0), (alternative, 1.0)):
        padded = "^" + token + "$"
        for order in CONTEXT_FEATURE_NGRAM_ORDERS:
            scale = sign / math.sqrt(max(1, len(padded) - order + 1))
            for index in range(len(padded) - order + 1):
                feature = "char:" + padded[index:index + order]
                features[feature] = features.get(feature, 0.0) + scale
    features["token:digits"] = float(any(char.isdigit() for char in original))
    features["token:technical"] = float(any(char in original for char in TECHNICAL_MARKS))
    features[f"context:{scripts[0]}:{scripts[1]}:{item.field.role}:{direction}:{length}"] = 1.0
    features[f"context:{scripts[0]}:{scripts[1]}:baseline:{baseline}:length:{length}"] = 1.0
    if feature_version in (CONTEXT_TYPO_FEATURE_VERSION, *CONTEXT_OPENING_SCHEMAS):
        # Only a reading outside the lexicon can be a typo, and a typo of a word
        # means something else when the typed reading is itself a word: `лучше`
        # stays although `kexit` is one letter from `exit`.
        typo = (f"typo:{int(item.source_typo)}:{int(item.target_typo)}"
                f":known:{int(item.source_known)}:{int(item.target_known)}")
        features[typo] = 1.0
        features[f"{typo}:baseline:{baseline}"] = 1.0
        # A digit reads the same in both layouts, so its n-grams cancel out of
        # the two readings. What it says depends on the layout of the letters
        # around it: `pm2` and `/c,jhrb2` stay, `зь2` is `pm2`.
        features[f"token:digits:direction:{direction}"] = features["token:digits"]
    if feature_version in CONTEXT_OPENING_SCHEMAS and (item.source_opening or item.target_opening):
        # Whether a reading opens sentences, and whether the word stands alone: `ns` alone
        # opens a message as `ты` does in Russian, and nothing in English opens with `ns`.
        # Nothing is said about the words neither of whose readings opens a sentence -
        # nearly all of them: as a feature of its own, `opening:0:0` in a sentence became
        # a general lean to keep for every word typed in the other layout, and letters
        # typed into the middle of `сука` in the English layout stayed (28.09.2026).
        opening = f"opening:{int(item.source_opening)}:{int(item.target_opening)}"
        alone = int(not before.strip() and not after.strip())
        features[opening] = 1.0
        features[f"{opening}:alone:{alone}:direction:{direction}"] = 1.0
    if feature_version in CONTEXT_OPENING_SCHEMAS and before.endswith("/"):
        # A path separator right before the token: `код/dpkg` read as `dpkg` after a
        # Russian word, so a package name typed as intended after a Russian path segment
        # was converted like a Russian word typed in the wrong layout.
        known = f"known:{int(item.source_known)}:{int(item.target_known)}:direction:{direction}"
        features["before:slash"] = 1.0
        features[f"before:slash:{known}"] = 1.0
    if feature_version in CONTEXT_OPENING_SCHEMAS and item.inside:
        # Letters typed into the middle of a word whose other letters are in the other
        # layout: the model is asked about the whole word, and without this it saw a word
        # like any other - letters typed into `шмидт` under a line of code stayed Latin at
        # 0.97, and a longer training that carried them over converted more package names
        # after a path (28.09.2026).
        features["inside"] = 1.0
        features[f"inside:known:{int(item.source_known)}:{int(item.target_known)}:direction:{direction}"] = 1.0
    if feature_version == CONTEXT_LINE_FEATURE_VERSION:
        known = f"known:{int(item.source_known)}:{int(item.target_known)}"
        # Where on its line the word stands. In Russian technical text `vs` is a word between
        # two terms (`React vs Vue`); at the start of a new line under English text it is `мы`
        # typed in the other layout - schema 6 saw the same last words either way.
        lines = item.field.before.rsplit("\n", 1)
        state = "same" if any(char.isalpha() for char in lines[-1]) else "newline" if len(lines) > 1 else "empty"
        features[f"line:{state}"] = 1.0
        features[f"line:{state}:direction:{direction}"] = 1.0
        features[f"line:{state}:{known}"] = 1.0
        if state == "newline":
            previous = lines[0].rsplit("\n", 1)[-1].casefold()
            ru = sum("а" <= char <= "я" or char == "ё" for char in previous)
            en = sum("a" <= char <= "z" for char in previous)
            features[f"line:newline:previous:{'ru' if ru > en else 'en' if en > ru else 'none'}:direction:{direction}"] = 1.0
        # Both readings have the same case: an inner capital (`WinForms`, `ЦштАщкьы`) or all
        # capitals (`WPF`, `ЦЗА`) say which one was meant, and the n-grams read casefolded text.
        letters = [char for char in item.original if char.isalpha()]
        if len(letters) > 1:
            shape = ("upper" if all(char.isupper() for char in letters) else "inner" if any(char.isupper() for char in letters[1:])
                     else "title" if letters[0].isupper() else "lower")
            features[f"case:{shape}:direction:{direction}"] = 1.0
            features[f"case:{shape}:{known}"] = 1.0
        latin = item.original if item.source_group == 0 else item.alternative
        cyrillic = item.alternative if item.source_group == 0 else item.original
        features[f"latin:dot:direction:{direction}"] = float(bool(_LATIN_DOT.search(latin)))
        features[f"latin:underscore:direction:{direction}"] = float("_" in latin.strip("_"))
        # How often each reading occurs inside Russian text: a Latin term (`id`, `wpf`), or
        # Cyrillic outside the lexicon (slang and abbreviations: `пдф`, `тп`). `gla`, `lut` and
        # a typo like `дге` do not occur, and the lexicon knows none of them. A Cyrillic reading
        # the lexicon knows is a bucket of its own: `швы` is a word, not a token never seen.
        cyrillic_known = item.source_known if item.source_group == 1 else item.target_known
        latin_bucket = term_bucket(latin, "latin")
        cyrillic_bucket = "lexicon" if cyrillic_known else term_bucket(cyrillic, "cyrillic")
        if not item.inside and (before.strip() or after.strip()):
            # How often each reading occurs in text of its own language splits the two buckets
            # that said the least: `uh` never occurs inside Russian technical text, yet it is an
            # English word and `gla` is not; `гр` is in the lexicon as `на` is, but only `на` is
            # common Russian. Without them an English line typed in the Russian layout kept its
            # first word when the next one converted, `гр why`: a Latin reading never seen inside
            # Russian text and a Cyrillic one the lexicon knows had both learned to mean a Russian
            # word (30.09.2026). Letters typed into a word are left out: the rest of the word
            # shows its language, and how rare the word is does not - with the tables a name
            # stayed half typed, `lyтn` and `даuе`, and English misses inside a word went from
            # 135 to 294 on the subtitle development sample. So is a word alone in its field: a
            # short one waits there for its neighbour, and the tables made `шы` so plainly `is`
            # that it converted alone (two false conversions on the holdout); asked again with
            # the neighbour, it has the tables.
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
            features[f"{languages}:{scripts[1]}:direction:{direction}"] = 1.0
            features[f"{languages}:line:{state}:direction:{direction}"] = 1.0
        # Letters typed into a word learn their own weights for the buckets: with a word alone in
        # its field sharing them, the Russian first words that stay drew them to `keep` and English
        # misses inside a word went from 151 to 233.
        frequency = "inside:freq" if item.inside else "freq"
        features[f"{frequency}:latin:{latin_bucket}:direction:{direction}"] = 1.0
        features[f"{frequency}:cyrillic:{cyrillic_bucket}:direction:{direction}"] = 1.0
        features[f"{frequency}:{latin_bucket}:{cyrillic_bucket}:direction:{direction}"] = 1.0
        # A token without letters. `=/`, `000?` and `"4` typed in the Russian layout are signs in
        # both readings and stay, while `1/` typed in the English layout is the Russian `1.`.
        # Their frequency buckets are `na`, as those of `don't` typed in the Russian layout are,
        # and that one converts: a Russian `=/` went over to `=|` (0.990). `:/` typed in the
        # English layout reads `Ж.`, and `ж` is a common word: an emoticon became a letter
        # (1.000), though `2.7`, `<=` and `:=` of the same shape all stay (30.09.2026).
        if not any(char.isalpha() for char in item.original):
            digits = int(any(char.isdigit() for char in item.original))
            kind = "letters" if any(char.isalpha() for char in item.alternative) else "only"
            features[f"signs:{kind}:direction:{direction}"] = 1.0
            features[f"signs:{kind}:digits:{digits}:direction:{direction}"] = 1.0
    return {name: value for name, value in features.items() if value}


def load_term_frequency(path: Path) -> dict[str, dict[str, int]]:
    """The table schema 7 reads, checked like the artifact: without it the model does not load."""

    with path.open("rb") as handle:
        raw = handle.read(MAX_CONTEXT_TERM_FREQUENCY_BYTES + 1)
    if len(raw) > MAX_CONTEXT_TERM_FREQUENCY_BYTES:
        raise ValueError("term frequency table is too large")
    payload: object = json.loads(raw)
    if not isinstance(payload, dict) or sorted(payload) != list(TERM_ALPHABETS):
        raise ValueError("invalid term frequency table")
    table: dict[str, dict[str, int]] = {}
    for alphabet, counts in payload.items():
        if not isinstance(counts, dict) or not all(type(count) is int and count > 0 for count in counts.values()):
            raise ValueError("invalid term frequency table")
        table[alphabet] = counts
    return table


def _term_frequency() -> dict[str, dict[str, int]]:
    global _TERM_FREQUENCY
    if _TERM_FREQUENCY is None:
        _TERM_FREQUENCY = load_term_frequency(TERM_FREQUENCY_PATH)
    return _TERM_FREQUENCY


def term_bucket(text: str, alphabet: str) -> str:
    """The bucket of how often a plain word occurs in the text one table counts; `na` for anything else."""

    core = text.strip(_TERM_EDGE_SIGNS).casefold()
    if not core or not core.isalpha():
        return "na"
    return str(bisect.bisect_right(CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS, _term_frequency()[alphabet].get(core, 0)))


def softmax(scores: list[float]) -> tuple[float, ...]:
    maximum = max(scores)
    values = [math.exp(value - maximum) for value in scores]
    total = sum(values)
    return tuple(value / total for value in values)


class ContextModel:
    def __init__(
        self, weights: Mapping[str, tuple[float, ...]], version: str,
        conversion_threshold: float = CONTEXT_V1_CONVERSION_THRESHOLD,
        *, feature_version: int = FEATURE_VERSION,
    ) -> None:
        if type(feature_version) is not int or feature_version not in SUPPORTED_FEATURE_VERSIONS:
            raise ValueError("incompatible context feature version")
        self.weights = dict(weights)
        self.version = version
        self.conversion_threshold = conversion_threshold
        self.feature_version = feature_version
        # Whether the single-letter head has weights here: its class is the one place a verdict on a letter
        # is the model's own opinion (context_policy).
        self.answers_letters = any(name.startswith(LETTER_FEATURE_PREFIX) for name in self.weights)

    @classmethod
    def load(cls, path: Path = ARTIFACT_PATH) -> ContextModel:
        with path.open("rb") as handle:
            raw = handle.read(MAX_ARTIFACT_BYTES + 1)
        if len(raw) > MAX_ARTIFACT_BYTES:
            raise ValueError("context model is too large")
        payload: object = json.loads(raw)
        if not isinstance(payload, dict):
            raise ValueError("context model must be an object")
        feature_version = payload.get("feature_version")
        if (type(feature_version) is not int or feature_version not in SUPPORTED_FEATURE_VERSIONS
                or payload.get("actions") != list(ACTIONS)):
            raise ValueError("incompatible context model")
        if feature_version == CONTEXT_LINE_FEATURE_VERSION:
            # Schema 7 reads how often each reading occurs in Russian text: the model is not whole without it.
            _term_frequency()
        raw_weights: object = payload.get("weights")
        if feature_version == CONTEXT_ACTION_FEATURE_VERSION:
            # Up to six feature spaces, each with its own budget in the recipe (MAX_CONTEXT_ACTION_MODEL_FEATURES).
            if not isinstance(raw_weights, dict) or not 0 < len(raw_weights) <= MAX_CONTEXT_ACTION_MODEL_FEATURES:
                raise ValueError("invalid context weights")
        elif not isinstance(raw_weights, dict) or not 0 < len(raw_weights) <= MAX_FEATURES:
            raise ValueError("invalid context weights")
        weights: dict[str, tuple[float, ...]] = {}
        for name, values in raw_weights.items():
            if (not isinstance(name, str) or len(name) > MAX_CONTEXT_FEATURE_NAME_CHARACTERS or not isinstance(values, list)
                    or len(values) != len(ACTIONS)):
                raise ValueError("invalid context feature")
            if any(isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value)
                   or abs(value) > MAX_CONTEXT_WEIGHT_MAGNITUDE for value in values):
                raise ValueError("invalid context weight")
            weights[name] = tuple(float(value) for value in values)
        checksum = hashlib.sha256(json.dumps(raw_weights, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()
        if payload.get("weights_sha256") != checksum:
            raise ValueError("context model checksum mismatch")
        version: object = payload.get("version")
        threshold: object = payload.get("conversion_threshold")
        prefix = "context-v3-" if feature_version == CONTEXT_ACTION_FEATURE_VERSION else "context-v1-"
        if not isinstance(version, str) or not version.startswith(prefix) or len(version) > MAX_CONTEXT_MODEL_VERSION_CHARACTERS:
            raise ValueError("invalid context version")
        if (isinstance(threshold, bool) or not isinstance(threshold, (int, float))
                or not MIN_CONTEXT_CONVERSION_THRESHOLD <= threshold <= 1.0):
            raise ValueError("unsafe context threshold")
        return cls(weights, version, float(threshold), feature_version=feature_version)

    @classmethod
    def try_load(cls, path: Path = ARTIFACT_PATH) -> tuple[ContextModel | None, str]:
        try:
            model = cls.load(path)
        except (OSError, ValueError) as error:
            return None, str(error)
        return model, model.version

    def supports_features(self, features: Mapping[str, float]) -> bool:
        """Use the same language-support gate during calibration and inference."""

        if self.feature_version == CONTEXT_ACTION_FEATURE_VERSION:
            text = ACTION_FEATURE_CHARACTER_TEXT_FIELD_INDEX
            # The kept-neighbour question names its letters under its own prefix, and the single-letter head
            # once more under its own: the kept-neighbour frames hold no word of one letter, so only the head
            # knows the letters of `b` before a kept `redis`.
            return all(any(
                name in self.weights for name in features
                if (bare := name.removeprefix(LETTER_FEATURE_PREFIX).removeprefix(KEPT_FEATURE_PREFIX)).startswith(label + ":char:")
                and any(char.isalpha() for char in bare.split(":", text)[text])
            ) for label in ("source", "target"))
        return any(
            name in self.weights
            for name in features
            if name.startswith(("pair:", "before:word:", "after:word:", "app:"))
        )

    def allows_automatic_conversion(self, features: Mapping[str, float]) -> bool:
        """The model's verdict stands; this is where an explicit exception would refuse it.

        KeySwitch exists because rule-based switching hit a wall: the rules multiplied
        until they argued with each other over the nuances of two languages. So when the
        model says convert, the product converts. What may still refuse a conversion is
        an exception the user can see and name - an excluded word or application, their
        own switching settings, a rule they taught the engine, or a case analysed one by
        one - and those live in the engine and the settings, not here.

        Until 17.09.2026 this method carried two class-wide vetoes instead: an isolated
        token needed a licence from a frozen verdict, and a token of at most three letters
        with an unknown own reading could only be suggested when it stood alone or was
        uppercase. They overruled the model on whole classes of input - exactly the input
        the model was trained to tell apart - and cost 14 of 300 restorations on chat-like
        first words while preventing no corruption at all
        (.t/reliable-release-2026-09-12/SHORT-ISOLATED-CURRICULUM.md). A weak class is a
        training problem; it is answered with the corpus, the labels and the features, and
        proved by measurement.
        """

        return True

    def predict(self, item: ContextEvidence) -> ContextPrediction:
        if self.feature_version == CONTEXT_ACTION_FEATURE_VERSION:
            try:
                features = extract_action_features(item)
            except ValueError:
                return ContextPrediction("suggest", 0.0, (0.0, 0.0, 0.0, 1.0), self.version, False)
        else:
            features = extract_context_features(item, self.feature_version)
        scores = [0.0] * len(ACTIONS)
        for name, value in features.items():
            weights = self.weights.get(name)
            if weights is not None:
                for index, weight in enumerate(weights):
                    scores[index] += weight * value
        if self.feature_version == CONTEXT_ACTION_FEATURE_VERSION and not all(math.isfinite(score) for score in scores):
            return ContextPrediction("suggest", 0.0, (0.0, 0.0, 0.0, 1.0), self.version, False)
        probabilities = softmax(scores)
        selected = max(range(len(ACTIONS)), key=probabilities.__getitem__)
        action = ACTIONS[selected]
        if action == "convert" and probabilities[selected] < self.conversion_threshold:
            action = "suggest"
        if (self.feature_version == CONTEXT_ACTION_FEATURE_VERSION and action == "keep"
                and probabilities[ACTIONS.index("convert")] >= CONTEXT_DOUBTFUL_KEEP_PROBABILITY):
            # A keep the model is unsure of, as sure as not of the other layout, is a suggestion too: nothing is
            # converted, and at a space the engine waits for the next word and asks again with it, the question
            # the model is trained on (the kept-neighbour question). `зк` after `принимай ` was kept at p=0.51
            # against 0.49 for `pr` and stayed; asked beside `и` the same model converts it at p=1.00.
            action = "suggest"
        if self.feature_version == CONTEXT_ACTION_FEATURE_VERSION and action == "convert" and not self.allows_automatic_conversion(features):
            action = "suggest"
        supported = self.supports_features(features)
        if self.feature_version == CONTEXT_ACTION_FEATURE_VERSION and not supported and action in {"keep", "convert"}:
            action = "suggest"
        return ContextPrediction(action, probabilities[selected], probabilities, self.version, supported)
