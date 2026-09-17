"""Prefix feature schemas beyond the frozen first one, and the loader that knows them.

The installed prefix-v1 model, its frozen corpus and the lexical-compatibility gate
pin ``prefix_model.py`` byte for byte, so schema two (characters of the observed
prefix) and the schema-aware artifact loader live here instead. The engine loads
the installed artifact through :meth:`VersionedPrefixModel.default`, which reads
either schema, so an accepted schema-two pair can be installed without touching
the frozen module.
"""
from __future__ import annotations

import hashlib
import json
import math
import re
import unicodedata
from functools import lru_cache
from pathlib import Path

from .context_model import ACTIONS, ContextModel, ContextPrediction
from .early_switch import PrefixIndex
from .language_model import LanguageModel
from .prefix_model import ARTIFACT, PREFIX_FEATURE_VERSION, PrefixInput, PrefixModel, features

CURRENT_PREFIX_FEATURE_VERSION = 2


def features_for_version(item: PrefixInput, indexes: dict[int, PrefixIndex], models: dict[int, LanguageModel],
                         *, feature_version: int = PREFIX_FEATURE_VERSION) -> dict[str, float]:
    """Schema one stays frozen; schema two learns characters of observed prefixes.

    The start marker describes an observed boundary. There is no end marker:
    an incremental prefix is not evidence that the word has finished.
    """
    if type(feature_version) is not int or feature_version not in (1, 2):
        raise ValueError("unsupported prefix features")
    result = features(item, indexes, models)
    if feature_version == 1:
        return result
    for side, text, group in (("source", item.original, item.source_group),
                              ("target", item.alternative, 1 - item.source_group)):
        observed = "^" + unicodedata.normalize("NFC", text[:12]).casefold()
        for order in range(1, 5):
            for start in range(len(observed) - order + 1):
                gram = observed[start:start + order]
                name = f"{side}:prefix_char:{group}:{order}:{gram}"
                result[name] = min(2.0, result.get(name, 0.0) + 1.0)
    return result


class VersionedPrefixModel(PrefixModel):
    """A prefix model that carries the feature schema its weights were fitted on."""

    def __init__(self, model: ContextModel, *, feature_version: int = PREFIX_FEATURE_VERSION) -> None:
        if type(feature_version) is not int or feature_version not in (1, CURRENT_PREFIX_FEATURE_VERSION):
            raise ValueError("unsupported prefix features")
        super().__init__(model)
        self.feature_version = feature_version

    def predict(self, item: PrefixInput, indexes: dict[int, PrefixIndex], models: dict[int, LanguageModel]) -> ContextPrediction:
        return self.predict_features(features_for_version(item, indexes, models, feature_version=self.feature_version))

    @classmethod
    def load(cls, path: Path = ARTIFACT) -> VersionedPrefixModel:
        with path.open("rb") as stream:
            raw = stream.read(2 * 1024 * 1024 + 1)
        if len(raw) > 2 * 1024 * 1024:
            raise ValueError("oversized prefix model")
        payload: object = json.loads(raw)
        if (not isinstance(payload, dict) or payload.get("kind") != "keyswitch.prefix-policy"
                or type(payload.get("feature_version")) is not int or payload.get("feature_version") not in (1, 2)
                or payload.get("actions") != list(ACTIONS)):
            raise ValueError("unsupported prefix model")
        feature_version = int(payload["feature_version"])
        declared = payload.get("prefix_feature_version", 1 if feature_version == 1 else None)
        if type(declared) is not int or declared != feature_version:
            raise ValueError("inconsistent prefix features")
        version, threshold, stored = payload.get("version"), payload.get("conversion_threshold"), payload.get("weights")
        namespace = f"prefix-v{feature_version}-"
        if not isinstance(version, str) or not re.fullmatch(namespace + r"[a-f0-9]{12}", version):
            raise ValueError("invalid prefix version")
        if isinstance(threshold, bool) or not isinstance(threshold, (float, int)) or not 0.985 <= threshold <= 1.0:
            raise ValueError("unsafe prefix threshold")
        if not isinstance(stored, dict) or not stored or len(stored) > 10000:
            raise ValueError("invalid prefix weights")
        weights: dict[str, tuple[float, ...]] = {}
        for name, vector in stored.items():
            if (not isinstance(name, str) or not name or len(name) > 160
                    or not isinstance(vector, list) or len(vector) != len(ACTIONS)
                    or any(type(value) not in (int, float) or not math.isfinite(value) for value in vector)):
                raise ValueError("invalid prefix coefficient")
            weights[name] = tuple(float(value) for value in vector)
        digest = hashlib.sha256(json.dumps(stored, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()
        if payload.get("weights_sha256") != digest or version != namespace + digest[:12]:
            raise ValueError("prefix checksum mismatch")
        return cls(ContextModel(weights, version, float(threshold)), feature_version=feature_version)

    @staticmethod
    @lru_cache(maxsize=1)
    def default() -> VersionedPrefixModel | None:
        """The installed prefix artifact in whichever schema it was accepted, or none."""
        try:
            return VersionedPrefixModel.load()
        except (OSError, ValueError, TypeError):
            return None
