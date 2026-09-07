"""V2 learned ranking of literal suffixes; no source words stored in weights.

The old rejected boundary-v1 experiment remains separately loadable for
regression. Only a separately verified v2 artifact enables this policy.
"""
from __future__ import annotations

import json
import math
from functools import lru_cache
from pathlib import Path

from .boundary_model import BoundaryModel, features as legacy_features
from .language_model import LanguageModel


ARTIFACT = Path(__file__).parent / "resources/models/boundary-v2.json"
FEATURE_VERSION = 2


def features(original: str, alternative: str, suffix: int,
             source: LanguageModel, target: LanguageModel) -> dict[str, float]:
    values = legacy_features(original, alternative, suffix, source, target)
    # A candidate can be an English literal or a Russian word. Learning must
    # compare both, not inherit the old ranker's mostly-English calibration.
    known = max(values["source_word:known"], values["target_word:known"])
    values.update({
        "word:known_either": known,
        "word:known_both": min(values["source_word:known"], values["target_word:known"]),
        "word:known_length": known * values["length"],
        "word:best_ngram": max(values["source_word:ngram"], values["target_word:ngram"]),
        "word:least_invalid": min(values["source_word:invalid"], values["target_word:invalid"]),
    })
    removed = original[-suffix:] if suffix else ""
    for char in ",.;[]'`":
        values["tail:has:" + char] = float(char in removed)
    tail = len(original) - len(original.rstrip(",.;[]'`"))
    known_candidates = 0
    for length in range(min(tail, 8, len(original) - 1) + 1):
        end = len(original) - length
        left, right = original[:end], alternative[:end]
        known_candidates += bool(left.isalpha() and source.score(left).exact or right.isalpha() and target.score(right).exact)
    competing = float(known_candidates > 1)
    # Learned interactions distinguish a strong sole candidate from several
    # legitimate interpretations. No threshold selects a spelling by itself.
    values.update({name + ":competition": value * competing for name, value in tuple(values.items())})
    return values


class BoundaryPolicy(BoundaryModel):
    @classmethod
    def load(cls, path: Path = ARTIFACT) -> BoundaryPolicy:
        with path.open("rb") as stream:
            raw = stream.read(65537)
        if len(raw) > 65536:
            raise ValueError("oversized boundary policy")
        value: object = json.loads(raw)
        if not isinstance(value, dict) or value.get("feature_version") != FEATURE_VERSION:
            raise ValueError("unsupported boundary policy")
        weights, threshold, version = value.get("weights"), value.get("threshold"), value.get("version")
        if (not isinstance(weights, dict) or not weights
                or any(not isinstance(key, str) or type(weight) not in (int, float) or not math.isfinite(weight) for key, weight in weights.items())
                or not isinstance(threshold, (float, int)) or isinstance(threshold, bool) or not 0.5 < threshold <= 1.0
                or not isinstance(version, str) or not version.startswith("boundary-v2-")):
            raise ValueError("invalid boundary policy")
        return cls(weights, threshold, version)

    @staticmethod
    @lru_cache(maxsize=1)
    def default() -> BoundaryPolicy | None:
        try:
            return BoundaryPolicy.load()
        except (OSError, ValueError, TypeError):
            return None
