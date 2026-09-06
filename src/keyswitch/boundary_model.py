"""Learned segmentation of a completed token with a layout-ambiguous suffix.

This is not an intent classifier for unfinished prefixes. The observer keeps
physical keys until a hard boundary/idle; this model then compares the whole
word with a word followed by literal punctuation. An uncertain result must
not authorize an automatic rewrite.
"""
from __future__ import annotations

import json
import math
from collections.abc import Mapping
from dataclasses import dataclass
from functools import lru_cache
from pathlib import Path

from .language_model import LanguageModel


ARTIFACT = Path(__file__).parent / "resources/models/boundary-v1.json"
FEATURE_VERSION = 1
MAX_SUFFIX = 8


def features(original: str, alternative: str, suffix_length: int,
             source: LanguageModel, target: LanguageModel) -> dict[str, float]:
    """Numeric evidence only: no word IDs, apps or private context memorized."""
    end = len(original) - suffix_length
    result = {"bias": 1.0, "suffix": min(suffix_length, MAX_SUFFIX) / MAX_SUFFIX,
              "length": min(end, 24) / 24, "whole": float(suffix_length == 0)}
    for name, text, model in (
        ("source_word", original[:end], source),
        ("target_word", alternative[:end], target),
    ):
        score = model.score(text)
        result[name + ":letters"] = float(text.isalpha())
        result[name + ":known"] = float(text.isalpha() and score.exact)
        result[name + ":frequency"] = math.log1p(score.frequency) / 20
        result[name + ":ngram"] = max(-3.0, min(3.0, score.ngram_score)) / 3
        result[name + ":invalid"] = score.invalid_ratio
    return result


@dataclass(frozen=True)
class BoundaryPrediction:
    suffix_length: int | None  # None = abstain, 0 = whole word
    probability: float  # softmax score, not a real-user correctness probability
    version: str


class BoundaryModel:
    def __init__(self, weights: Mapping[str, float], threshold: float, version: str) -> None:
        self.weights = dict(weights)
        self.threshold = threshold
        self.version = version

    def probabilities(self, candidates: tuple[dict[str, float], ...]) -> tuple[float, ...]:
        scores = [sum(self.weights.get(name, 0.0) * value for name, value in values.items()) for values in candidates]
        maximum = max(scores)
        exp = [math.exp(value - maximum) for value in scores]
        total = sum(exp)
        return tuple(value / total for value in exp)

    def predict(self, candidates: tuple[dict[str, float], ...]) -> BoundaryPrediction:
        probabilities = self.probabilities(candidates)
        best = max(range(len(probabilities)), key=probabilities.__getitem__)
        probability = probabilities[best]
        return BoundaryPrediction(best if probability >= self.threshold else None, probability, self.version)

    @classmethod
    def load(cls, path: Path = ARTIFACT) -> BoundaryModel:
        value: object = json.loads(path.read_bytes())
        if not isinstance(value, dict) or value.get("feature_version") != FEATURE_VERSION:
            raise ValueError("unsupported boundary model")
        weights, threshold, version = value.get("weights"), value.get("threshold"), value.get("version")
        if (not isinstance(weights, dict) or not weights
                or any(not isinstance(key, str) or type(weight) not in (int, float) or not math.isfinite(weight) for key, weight in weights.items())
                or not isinstance(threshold, (int, float)) or isinstance(threshold, bool) or not 0.5 < threshold <= 1.0
                or not isinstance(version, str) or not version):
            raise ValueError("invalid boundary model")
        return cls(weights, threshold, version)

    @staticmethod
    @lru_cache(maxsize=1)
    def default() -> BoundaryModel | None:
        try:
            return BoundaryModel.load()
        except (OSError, ValueError, TypeError):
            return None
