"""Small local, learned contextual action policy, separate from Layout Intent.

The model is a four-class sparse softmax classifier. It receives the recent
sentence, application/field evidence and the existing detector's lexical
evidence. Weights are produced by tools/train_context_model.py, not rules in
the runtime. Probabilities are synthetic-corpus scores, not a promise of
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


ContextAction = Literal["keep", "convert", "wait", "suggest"]
ACTIONS: Final[tuple[ContextAction, ...]] = ("keep", "convert", "wait", "suggest")
FEATURE_VERSION = 2
MAX_ARTIFACT_BYTES = 8 * 1024 * 1024
MAX_FEATURES = 50000
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
    ) -> None:
        self.weights = dict(weights)
        self.version = version
        self.conversion_threshold = conversion_threshold

    @classmethod
    def load(cls, path: Path = ARTIFACT_PATH) -> ContextModel:
        with path.open("rb") as handle:
            raw = handle.read(MAX_ARTIFACT_BYTES + 1)
        if len(raw) > MAX_ARTIFACT_BYTES:
            raise ValueError("context model is too large")
        payload: object = json.loads(raw)
        if not isinstance(payload, dict):
            raise ValueError("context model must be an object")
        if payload.get("feature_version") != FEATURE_VERSION or payload.get("actions") != list(ACTIONS):
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
        if not isinstance(version, str) or not version.startswith("context-v1-") or len(version) > 80:
            raise ValueError("invalid context version")
        if isinstance(threshold, bool) or not isinstance(threshold, (int, float)) or not 0.95 <= threshold <= 1.0:
            raise ValueError("unsafe context threshold")
        return cls(weights, version, float(threshold))

    @classmethod
    def try_load(cls) -> tuple[ContextModel | None, str]:
        try:
            model = cls.load()
        except (OSError, ValueError) as error:
            return None, str(error)
        return model, model.version

    def predict(self, item: ContextEvidence) -> ContextPrediction:
        features = extract_context_features(item)
        scores = [0.0] * len(ACTIONS)
        for name, value in features.items():
            weights = self.weights.get(name)
            if weights is not None:
                for index, weight in enumerate(weights):
                    scores[index] += weight * value
        probabilities = softmax(scores)
        selected = max(range(len(ACTIONS)), key=probabilities.__getitem__)
        action = ACTIONS[selected]
        if action == "convert" and probabilities[selected] < self.conversion_threshold:
            action = "suggest"
        supported = any(
            name in self.weights
            for name in features
            if name.startswith(("pair:", "before:word:", "after:word:", "app:"))
        )
        return ContextPrediction(action, probabilities[selected], probabilities, self.version, supported)
