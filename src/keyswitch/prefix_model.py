"""A separately trained action classifier for incremental EN/RU prefixes.

Uses the validated sparse-weight container, not the completed-word extractor
or its weights. Its scores describe synthetic training evidence, not a
calibrated probability that a real user's intent has been understood.
"""
from __future__ import annotations

import math
import re
import hashlib
import json
from dataclasses import dataclass
from functools import lru_cache
from pathlib import Path

from .context_model import ACTIONS, ContextModel, ContextPrediction, softmax
from .early_switch import PrefixIndex
from .input_context import FieldContext
from .language_model import LanguageModel
from .constants.file_formats import MAX_PREFIX_MODEL_BYTES, VERSION_HASH_CHARACTERS
from .constants.models import (
    MAX_PREFIX_FEATURE_NAME_CHARACTERS,
    MAX_PREFIX_WEIGHTS,
    MIN_PREFIX_CONVERSION_THRESHOLD,
)
from .constants.prefix import (
    PREFIX_V1_APPLICATION_NAME_CHARACTERS,
    PREFIX_V1_APPLICATION_TOKEN_COUNT,
    PREFIX_V1_BEFORE_CONTEXT_CHARACTERS,
    PREFIX_V1_BEFORE_WORD_COUNT,
    PREFIX_V1_COMPLETIONS_LOG_DIVISOR,
    PREFIX_V1_FEATURE_VERSION as PREFIX_FEATURE_VERSION,
    PREFIX_V1_FREQUENCY_LOG_DIVISOR,
    PREFIX_V1_LENGTH_BUCKET_MAX_CHARACTERS,
    PREFIX_V1_LENGTH_SCALE_CHARACTERS,
    PREFIX_V1_NGRAM_SCORE_BOUND,
    PREFIX_V1_QUOTE_MARKS_PER_PAIR,
)

__all__ = ["ARTIFACT", "PREFIX_FEATURE_VERSION", "WORDS", "PrefixInput", "PrefixModel", "features"]

ARTIFACT = Path(__file__).parent / "resources/models/prefix_policy_v1.json"
WORDS = re.compile(r"[^\W\d_]+", re.UNICODE)


@dataclass(frozen=True)
class PrefixInput:
    original: str
    alternative: str
    source_group: int
    field: FieldContext


def features(item: PrefixInput, indexes: dict[int, PrefixIndex], models: dict[int, LanguageModel]) -> dict[str, float]:
    original, alternate = item.original.casefold(), item.alternative.casefold()
    result: dict[str, float] = {
        "bias": 1.0, f"direction:{item.source_group}": 1.0,
        f"length:{min(len(original), PREFIX_V1_LENGTH_BUCKET_MAX_CHARACTERS)}": 1.0,
        "length": min(len(original), PREFIX_V1_LENGTH_SCALE_CHARACTERS) / PREFIX_V1_LENGTH_SCALE_CHARACTERS,
        "source:letters": float(original.isalpha()),
        "target:letters": float(alternate.isalpha()),
        "token:capitals": float(any(char.isupper() for char in item.original[1:])),
        "token:digits": float(any(char.isdigit() for char in original)),
    }
    for name, text, group in (("source", original, item.source_group), ("target", alternate, 1 - item.source_group)):
        evidence = indexes[group].completions(text)
        score = models[group].score(text)
        result.update({
            name + ":empty": float(evidence.completions == 0),
            name + ":completions": math.log1p(evidence.completions) / PREFIX_V1_COMPLETIONS_LOG_DIVISOR,
            name + ":frequency": math.log1p(evidence.maximum_frequency) / PREFIX_V1_FREQUENCY_LOG_DIVISOR,
            name + ":known": float(evidence.known or score.known),
            name + ":invalid": score.invalid_ratio,
            name + ":ngram": max(-PREFIX_V1_NGRAM_SCORE_BOUND, min(PREFIX_V1_NGRAM_SCORE_BOUND, score.ngram_score))
            / PREFIX_V1_NGRAM_SCORE_BOUND,
        })
    before = item.field.before[-PREFIX_V1_BEFORE_CONTEXT_CHARACTERS:].casefold()
    line = before.rsplit("\n", 1)[-1]
    comment = bool(re.search(r"(?://|/\*|#).*\Z", line))
    syntax = bool(re.search(r"[=;{}()\[\]$]", line))
    quote = bool(line.count('"') % PREFIX_V1_QUOTE_MARKS_PER_PAIR or line.count("'") % PREFIX_V1_QUOTE_MARKS_PER_PAIR)
    ru = sum("а" <= char <= "я" or char == "ё" for char in before)
    en = sum("a" <= char <= "z" for char in before)
    context = {
        "comment": float(comment), "syntax": float(syntax), "quote": float(quote),
        "empty": float(not before.strip()), "ru": ru / max(1, ru + en),
        "en": en / max(1, ru + en), "technical": float(syntax and not comment and not quote),
    }
    for name, value in context.items():
        result["before:" + name] = value
        result["source_empty:before:" + name] = value * result["source:empty"]
    result[f"role:{item.field.role}"] = 1.0
    application = item.field.application.casefold()[:PREFIX_V1_APPLICATION_NAME_CHARACTERS]
    for part in re.findall(r"[a-z0-9]+", application)[:PREFIX_V1_APPLICATION_TOKEN_COUNT]:
        result["app:" + part] = 1.0
    for word in WORDS.findall(before)[-PREFIX_V1_BEFORE_WORD_COUNT:]:
        result["before:word:" + word] = 1.0
    return result


class PrefixModel:
    def __init__(self, model: ContextModel) -> None:
        self.model = model

    @property
    def version(self) -> str:
        return self.model.version

    def predict_features(self, values: dict[str, float]) -> ContextPrediction:
        scores = [0.0] * len(ACTIONS)
        for name, value in values.items():
            for index, weight in enumerate(self.model.weights.get(name, ())):
                scores[index] += weight * value
        probabilities = softmax(scores)
        best = max(range(len(ACTIONS)), key=probabilities.__getitem__)
        action = ACTIONS[best]
        probability = probabilities[best]
        if action == "convert" and probability < self.model.conversion_threshold:
            action = "wait"
        return ContextPrediction(action, probability, tuple(probabilities), self.version, True)

    def predict(self, item: PrefixInput, indexes: dict[int, PrefixIndex], models: dict[int, LanguageModel]) -> ContextPrediction:
        return self.predict_features(features(item, indexes, models))

    @classmethod
    def load(cls, path: Path = ARTIFACT) -> PrefixModel:
        with path.open("rb") as stream:
            raw = stream.read(MAX_PREFIX_MODEL_BYTES + 1)
        if len(raw) > MAX_PREFIX_MODEL_BYTES:
            raise ValueError("oversized prefix model")
        payload: object = json.loads(raw)
        if (not isinstance(payload, dict) or payload.get("kind") != "keyswitch.prefix-policy"
                or payload.get("feature_version") != PREFIX_FEATURE_VERSION
                or payload.get("actions") != list(ACTIONS)):
            raise ValueError("unsupported prefix model")
        version, threshold, stored = payload.get("version"), payload.get("conversion_threshold"), payload.get("weights")
        if not isinstance(version, str) or not re.fullmatch(r"prefix-v1-[a-f0-9]{12}", version):
            raise ValueError("invalid prefix version")
        if isinstance(threshold, bool) or not isinstance(threshold, (float, int)) or not MIN_PREFIX_CONVERSION_THRESHOLD <= threshold <= 1.0:
            raise ValueError("unsafe prefix threshold")
        if not isinstance(stored, dict) or not stored or len(stored) > MAX_PREFIX_WEIGHTS:
            raise ValueError("invalid prefix weights")
        weights: dict[str, tuple[float, ...]] = {}
        for name, vector in stored.items():
            if (not isinstance(name, str) or not name or len(name) > MAX_PREFIX_FEATURE_NAME_CHARACTERS
                    or not isinstance(vector, list) or len(vector) != len(ACTIONS)
                    or any(type(value) not in (int, float) or not math.isfinite(value) for value in vector)):
                raise ValueError("invalid prefix coefficient")
            weights[name] = tuple(float(value) for value in vector)
        digest = hashlib.sha256(json.dumps(stored, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()
        if payload.get("weights_sha256") != digest or version != "prefix-v1-" + digest[:VERSION_HASH_CHARACTERS]:
            raise ValueError("prefix checksum mismatch")
        return cls(ContextModel(weights, version, float(threshold)))

    @staticmethod
    @lru_cache(maxsize=1)
    def default() -> PrefixModel | None:
        try:
            return PrefixModel.load()
        except (OSError, ValueError, TypeError):
            return None
