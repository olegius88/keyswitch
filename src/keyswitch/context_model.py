"""Small local, learned contextual action policy, separate from Layout Intent.

The model is a four-class sparse softmax classifier. It receives the recent
sentence, application/field evidence and the existing detector's lexical
evidence. The legacy trainer produces feature2 weights; the context action
trainer produces feature3 weights. Probabilities are corpus scores, not a promise of
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
from .language_model import WordScore
from .context_action_features import extract_action_features


ContextAction = Literal["keep", "convert", "wait", "suggest"]
AfterOrigin = Literal["none", "field", "planned_next_conversion"]
ACTIONS: Final[tuple[ContextAction, ...]] = ("keep", "convert", "wait", "suggest")
FEATURE_VERSION = 2
MAX_ARTIFACT_BYTES = 8 * 1024 * 1024
MAX_FEATURES = 50000
# A correctly typed short token unknown to the lexicon cannot be told apart from
# the same keys typed in the wrong layout when the other reading happens to be a
# word or a command name and nothing else disambiguates it: an all-uppercase
# acronym or brand in any context, or any such token standing alone. The model
# may only suggest there. Two letters are already deferred by the corpus policy.
SHORT_UNKNOWN_SOURCE_MAX_LENGTH: Final = 3
SHORT_UPPERCASE_UNKNOWN_SOURCE_MAX_LENGTH: Final = SHORT_UNKNOWN_SOURCE_MAX_LENGTH
ARTIFACT_PATH = Path(__file__).parent / "resources" / "models" / "context_policy_v1.json"
_WORDS = re.compile(r"[^\W\d_]+(?:['’\-][^\W\d_]+)*", re.UNICODE)


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


def extract_context_features(item: ContextEvidence) -> dict[str, float]:
    """Bounded features shared verbatim by training and serving."""

    original, alternative = _normalized(item.original[:64]), _normalized(item.alternative[:64])
    before, after = _normalized(item.field.before[-512:]), _normalized(item.field.after[:128])
    length = min(6, max(len(original), len(alternative)))
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
        "score_delta": max(-10.0, min(10.0, item.score_delta)) / 10.0,
    }
    for part in re.findall(r"[a-z0-9]+", item.field.application.casefold()[:128])[:4]:
        features[f"app:{part}"] = 1.0
        features[f"app:{part}:length:{length}"] = 1.0
    scripts: list[str] = []
    for label, text in (("before", before), ("after", after)):
        words = _WORDS.findall(text)
        words = words[-6:] if label == "before" else words[:3]
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
        for order in (1, 2, 3):
            scale = sign / math.sqrt(max(1, len(padded) - order + 1))
            for index in range(len(padded) - order + 1):
                feature = "char:" + padded[index:index + order]
                features[feature] = features.get(feature, 0.0) + scale
    features["token:digits"] = float(any(char.isdigit() for char in original))
    features["token:technical"] = float(any(char in original for char in "_/@\\=<>"))
    features[f"context:{scripts[0]}:{scripts[1]}:{item.field.role}:{direction}:{length}"] = 1.0
    features[f"context:{scripts[0]}:{scripts[1]}:baseline:{baseline}:length:{length}"] = 1.0
    return {name: value for name, value in features.items() if value}


def softmax(scores: list[float]) -> tuple[float, ...]:
    maximum = max(scores)
    values = [math.exp(value - maximum) for value in scores]
    total = sum(values)
    return tuple(value / total for value in values)


class ContextModel:
    def __init__(
        self, weights: Mapping[str, tuple[float, ...]], version: str,
        conversion_threshold: float = 0.985,
        *, feature_version: int = FEATURE_VERSION,
    ) -> None:
        if type(feature_version) is not int or feature_version not in (FEATURE_VERSION, 3):
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
        if type(feature_version) is not int or feature_version not in (FEATURE_VERSION, 3) or payload.get("actions") != list(ACTIONS):
            raise ValueError("incompatible context model")
        raw_weights: object = payload.get("weights")
        if not isinstance(raw_weights, dict) or not 0 < len(raw_weights) <= MAX_FEATURES:
            raise ValueError("invalid context weights")
        weights: dict[str, tuple[float, ...]] = {}
        for name, values in raw_weights.items():
            if not isinstance(name, str) or len(name) > 512 or not isinstance(values, list) or len(values) != 4:
                raise ValueError("invalid context feature")
            if any(isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or abs(value) > 1000 for value in values):
                raise ValueError("invalid context weight")
            weights[name] = tuple(float(value) for value in values)
        checksum = hashlib.sha256(json.dumps(raw_weights, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()
        if payload.get("weights_sha256") != checksum:
            raise ValueError("context model checksum mismatch")
        version: object = payload.get("version")
        threshold: object = payload.get("conversion_threshold")
        prefix = "context-v1-" if feature_version == FEATURE_VERSION else "context-v3-"
        if not isinstance(version, str) or not version.startswith(prefix) or len(version) > 80:
            raise ValueError("invalid context version")
        if isinstance(threshold, bool) or not isinstance(threshold, (int, float)) or not 0.95 <= threshold <= 1.0:
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

        if self.feature_version == 3:
            return all(any(
                name in self.weights for name in features
                if name.startswith(label + ":char:") and any(char.isalpha() for char in name.split(":", 4)[4])
            ) for label in ("source", "target"))
        return any(
            name in self.weights
            for name in features
            if name.startswith(("pair:", "before:word:", "after:word:", "app:"))
        )

    def allows_automatic_conversion(self, features: Mapping[str, float]) -> bool:
        """Shared by inference, calibration and epoch selection.

        Two ambiguous classes may only be suggested. A token of at most three
        letters whose own reading is unknown to the lexicon, when it is all
        uppercase (an acronym or a brand against the same keys in the other
        layout) or when no word stands on either side of it (a chat word against
        a command name typed in the wrong layout): with so little text both
        readings stay plausible. And a token of any length standing alone
        converts automatically only under a licence from a frozen verdict: the
        detector's baseline decision, an orthotactic margin above that model's
        own threshold, or a known other reading - a dictionary word or a command
        name - against an own reading the lexicon does not know. Without one,
        with no context, the conversion would rest on the learned character
        weights alone, which cannot tell a rare correctly typed word from the
        same keys in the wrong layout, nor one dictionary word from another. A
        word the lexicon knows is evidence for what was typed, so it withdraws
        the licence even when the other reading is a command name: `зум` is a
        chat word as much as `pev` is a program.
        """

        if self.feature_version != 3:
            return True
        isolated = any(name.startswith("before:script:none:direction:") for name in features) and any(
            name.startswith("after:script:none:direction:") for name in features)
        known_other_reading = "target:known:1" in features or "target:identifier:1" in features
        licensed = ("baseline:1" in features or features.get("ortho:margin", 0.0) > 0.0
                    or (known_other_reading and "source:known:0" in features))
        if isolated and not licensed:
            return False
        if "source:known:0" not in features:
            return True
        if not any(f"length:{length}" in features for length in range(1, SHORT_UNKNOWN_SOURCE_MAX_LENGTH + 1)):
            return True
        uppercase = features.get("source:case:upper") == 1.0
        return not (uppercase or isolated)

    def predict(self, item: ContextEvidence) -> ContextPrediction:
        if self.feature_version == 3:
            try:
                features = extract_action_features(item)
            except ValueError:
                return ContextPrediction("suggest", 0.0, (0.0, 0.0, 0.0, 1.0), self.version, False)
        else:
            features = extract_context_features(item)
        scores = [0.0] * len(ACTIONS)
        for name, value in features.items():
            weights = self.weights.get(name)
            if weights is not None:
                for index, weight in enumerate(weights):
                    scores[index] += weight * value
        if self.feature_version == 3 and not all(math.isfinite(score) for score in scores):
            return ContextPrediction("suggest", 0.0, (0.0, 0.0, 0.0, 1.0), self.version, False)
        probabilities = softmax(scores)
        selected = max(range(len(ACTIONS)), key=probabilities.__getitem__)
        action = ACTIONS[selected]
        if action == "convert" and probabilities[selected] < self.conversion_threshold:
            action = "suggest"
        if self.feature_version == 3 and action == "convert" and not self.allows_automatic_conversion(features):
            action = "suggest"
        supported = self.supports_features(features)
        if self.feature_version == 3 and not supported and action in {"keep", "convert"}:
            action = "suggest"
        return ContextPrediction(action, probabilities[selected], probabilities, self.version, supported)
