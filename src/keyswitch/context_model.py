"""Small local, learned contextual action policy, separate from Layout Intent.

The model is a four-class sparse softmax classifier. It receives the recent
sentence, application/field evidence and the existing detector's lexical
evidence. The context-v1 trainer produces feature5 weights (feature2 plus typo
evidence); feature2 artifacts still load, and the context action trainer
produces feature3 weights. Probabilities are corpus scores, not a promise of
real-world correctness. Hard safety/explicit user intent live in the engine.
"""

from __future__ import annotations

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
from .constants.file_formats import MAX_CONTEXT_MODEL_BYTES as MAX_ARTIFACT_BYTES
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
    CONTEXT_FEATURE_AFTER_WORD_COUNT,
    CONTEXT_FEATURE_BEFORE_WORD_COUNT,
    CONTEXT_FEATURE_NGRAM_ORDERS,
    CONTEXT_MODEL_FEATURE_VERSION as FEATURE_VERSION,
    CONTEXT_SUPPORTED_FEATURE_VERSIONS as SUPPORTED_FEATURE_VERSIONS,
    CONTEXT_TYPO_FEATURE_VERSION,
    CONTEXT_TYPO_MIN_CHARACTERS,
    CONTEXT_V1_CONVERSION_THRESHOLD,
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
AfterOrigin = Literal["none", "field", "planned_next_conversion"]
ACTIONS: Final[tuple[ContextAction, ...]] = ("keep", "convert", "wait", "suggest")
ARTIFACT_PATH = Path(__file__).parent / "resources" / "models" / "context_policy_v1.json"
_WORDS = re.compile(r"[^\W\d_]+(?:['’\-][^\W\d_]+)*", re.UNICODE)
# Characters that mark a path, an address or an identifier inside a token.
TECHNICAL_MARKS: Final = "_/@\\=<>"


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
    """Bounded features shared verbatim by training and serving; schema 5 adds the typo evidence."""

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
    for label, text in (("before", before), ("after", after)):
        words = _WORDS.findall(text)
        words = words[-CONTEXT_FEATURE_BEFORE_WORD_COUNT:] if label == "before" else words[:CONTEXT_FEATURE_AFTER_WORD_COUNT]
        ru = sum("а" <= char <= "я" or char == "ё" for char in text)
        en = sum("a" <= char <= "z" for char in text)
        dominant = "ru" if ru > en else "en" if en > ru else "none"
        scripts.append(dominant)
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
    if feature_version == CONTEXT_TYPO_FEATURE_VERSION:
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
    return {name: value for name, value in features.items() if value}


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
        raw_weights: object = payload.get("weights")
        if not isinstance(raw_weights, dict) or not 0 < len(raw_weights) <= MAX_FEATURES:
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
    def try_load(cls) -> tuple[ContextModel | None, str]:
        try:
            model = cls.load()
        except (OSError, ValueError) as error:
            return None, str(error)
        return model, model.version

    def supports_features(self, features: Mapping[str, float]) -> bool:
        """Use the same language-support gate during calibration and inference."""

        if self.feature_version == CONTEXT_ACTION_FEATURE_VERSION:
            text = ACTION_FEATURE_CHARACTER_TEXT_FIELD_INDEX
            return all(any(
                name in self.weights for name in features
                if name.startswith(label + ":char:") and any(char.isalpha() for char in name.split(":", text)[text])
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
        if self.feature_version == CONTEXT_ACTION_FEATURE_VERSION and action == "convert" and not self.allows_automatic_conversion(features):
            action = "suggest"
        supported = self.supports_features(features)
        if self.feature_version == CONTEXT_ACTION_FEATURE_VERSION and not supported and action in {"keep", "convert"}:
            action = "suggest"
        return ContextPrediction(action, probabilities[selected], probabilities, self.version, supported)
