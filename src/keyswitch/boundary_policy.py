"""V2 learned ranking of literal suffixes; no source words stored in weights.

The old rejected boundary-v1 experiment remains separately loadable for
regression. Only a separately verified v2 artifact enables this policy.
An artifact carries the feature version it was trained on (2 or 3); the policy
extracts exactly those features, so an installed version-2 artifact decides as
before while a version-3 one also sees which readings are misspellings.
"""
from __future__ import annotations

import json
import math
from collections.abc import Mapping
from functools import lru_cache
from pathlib import Path

from .boundary_model import MAX_SUFFIX, BoundaryModel, features as legacy_features
from .language_model import LanguageModel
from .constants.boundary import (
    BOUNDARY_AMBIGUOUS_PUNCTUATION,
    BOUNDARY_MISSING_LETTERS,
    BOUNDARY_POLICY_MAX_BYTES,
    BOUNDARY_SUPPORTED_FEATURE_VERSIONS as SUPPORTED_FEATURE_VERSIONS,
    BOUNDARY_THRESHOLD_EXCLUSIVE_MIN,
    BOUNDARY_V2_FEATURE_VERSION,
    BOUNDARY_V2_VERSION_PREFIX,
    BOUNDARY_V3_FEATURE_VERSION as FEATURE_VERSION,
)

__all__ = ["ARTIFACT", "FEATURE_VERSION", "SUPPORTED_FEATURE_VERSIONS", "BoundaryPolicy", "features", "misspelled"]

ARTIFACT = Path(__file__).parent / "resources/models/boundary-v2.json"


def misspelled(text: str, model: LanguageModel) -> bool:
    """A reading that is no word itself but a known word with one letter missing, not its last."""
    word = LanguageModel.normalize(text)
    if not word or word != text.casefold() or not word.isalpha() or model.frequencies.get(word, 0):
        return False
    letters = BOUNDARY_MISSING_LETTERS.get(model.locale, "")
    return any(model.frequencies.get(word[:index] + letter + word[index:], 0)
               for index in range(len(word)) for letter in letters)


def features(original: str, alternative: str, suffix: int,
             source: LanguageModel, target: LanguageModel, version: int = FEATURE_VERSION) -> dict[str, float]:
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
    for char in BOUNDARY_AMBIGUOUS_PUNCTUATION:
        values["tail:has:" + char] = float(char in removed)
    if version != BOUNDARY_V2_FEATURE_VERSION:
        # Misspellings: which reading could be a known word typed with one key missed.
        end = len(original) - suffix
        source_misspelled = float(misspelled(original[:end], source))
        target_misspelled = float(misspelled(alternative[:end], target))
        values.update({"source_word:misspelling": source_misspelled, "target_word:misspelling": target_misspelled,
                       "word:misspelling_either": max(source_misspelled, target_misspelled)})
    tail = len(original) - len(original.rstrip(BOUNDARY_AMBIGUOUS_PUNCTUATION))
    known_candidates = 0
    for length in range(min(tail, MAX_SUFFIX, len(original) - 1) + 1):
        end = len(original) - length
        left, right = original[:end], alternative[:end]
        known_candidates += bool(left.isalpha() and source.score(left).exact or right.isalpha() and target.score(right).exact)
    competing = float(known_candidates > 1)
    base = tuple(values.items())
    # Learned interactions distinguish a strong sole candidate from several
    # legitimate interpretations. No threshold selects a spelling by itself.
    values.update({name + ":competition": value * competing for name, value in base})
    if version != BOUNDARY_V2_FEATURE_VERSION and not known_candidates:
        # With no reading a known word, the legitimate interpretations are the misspellings of known
        # words; several of them are the same rivalry the known words have above, learned apart.
        rivals = float(sum(misspelled(original[:len(original) - length], source)
                           or misspelled(alternative[:len(original) - length], target)
                           for length in range(min(tail, MAX_SUFFIX, len(original) - 1) + 1)) > 1)
        values.update({name + ":misspelled_rivals": value * rivals for name, value in base})
    elif version != BOUNDARY_V2_FEATURE_VERSION:
        values.update({name + ":misspelled_rivals": 0.0 for name, _ in base})
    return values


class BoundaryPolicy(BoundaryModel):
    def __init__(self, weights: Mapping[str, float], threshold: float, version: str,
                 feature_version: int = FEATURE_VERSION) -> None:
        super().__init__(weights, threshold, version)
        self.feature_version = feature_version

    def extract(self, original: str, alternative: str, suffix: int,
                source: LanguageModel, target: LanguageModel) -> dict[str, float]:
        """The features of the version this artifact was trained on."""
        return features(original, alternative, suffix, source, target, self.feature_version)

    @classmethod
    def load(cls, path: Path = ARTIFACT) -> BoundaryPolicy:
        with path.open("rb") as stream:
            raw = stream.read(BOUNDARY_POLICY_MAX_BYTES + 1)
        if len(raw) > BOUNDARY_POLICY_MAX_BYTES:
            raise ValueError("oversized boundary policy")
        value: object = json.loads(raw)
        if not isinstance(value, dict) or value.get("feature_version") not in SUPPORTED_FEATURE_VERSIONS:
            raise ValueError("unsupported boundary policy")
        feature_version = value["feature_version"]
        weights, threshold, version = value.get("weights"), value.get("threshold"), value.get("version")
        if (not isinstance(weights, dict) or not weights
                or any(not isinstance(key, str) or type(weight) not in (int, float) or not math.isfinite(weight) for key, weight in weights.items())
                or not isinstance(threshold, (float, int)) or isinstance(threshold, bool)
                or not BOUNDARY_THRESHOLD_EXCLUSIVE_MIN < threshold <= 1.0
                or not isinstance(version, str) or not version.startswith(BOUNDARY_V2_VERSION_PREFIX)):
            raise ValueError("invalid boundary policy")
        return cls(weights, threshold, version, feature_version)

    @staticmethod
    @lru_cache(maxsize=1)
    def default() -> BoundaryPolicy | None:
        try:
            return BoundaryPolicy.load()
        except (OSError, ValueError, TypeError):
            return None
