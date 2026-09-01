#!/usr/bin/env python3
"""Deterministic offline trainer for the KeySwitch layout-intent model.

The module intentionally depends only on the Python standard library.  Dataset
construction, FTRL-Proximal, score calibration and threshold selection are
kept importable so focused tests can exercise every stage without the large
system lexicons.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import platform
import random
import struct
import sys
import tempfile
from collections import defaultdict
from collections.abc import Collection, Iterable, Mapping, Sequence
from dataclasses import asdict, dataclass, field, replace
from functools import lru_cache
from pathlib import Path
from types import MappingProxyType
from typing import Final, Literal, Protocol, TypeAlias, cast

from keyswitch.intent_model import (
    DEFAULT_FNV_SEED,
    DEFAULT_MEMBERSHIP_FNV_SEED,
    FEATURE_VERSION,
    MAX_DIMENSION,
    MAX_PAYLOAD_BYTES,
    MAX_SUPPORTED_FINGERPRINTS,
    MINIMUM_RUNTIME_TOKEN_LENGTH,
    NGRAM_ORDERS,
    LAYOUT_DIRECTIONS,
    TRIGGERS as MODEL_TRIGGERS,
    CorrectionTrigger,
    IntentModelInput,
    LayoutDirection,
    LinearNgramModel,
    PlattParameters,
    extract_features,
    layout_direction,
    stable_sigmoid as stable_sigmoid,
    write_model,
)
from keyswitch.detector import (
    LanguageDetector,
)
from keyswitch.language_model import LanguageModel, WordScore
from keyswitch.layouts import LayoutPair


SplitName: TypeAlias = Literal[
    "train", "development", "calibration", "threshold", "test"
]
QuarantineReason: TypeAlias = Literal[
    "cross_split", "cross_language", "cross_split+cross_language"
]
ContextGroupSelector: TypeAlias = Literal["source", "target"]
SparseFeatures: TypeAlias = tuple[tuple[int, float], ...]

SPLIT_BUCKETS: Final[tuple[tuple[SplitName, int], ...]] = (
    ("train", 26),
    ("development", 4),
    ("calibration", 4),
    ("threshold", 3),
    ("test", 3),
)
SPLIT_NAMES: Final[tuple[SplitName, ...]] = tuple(
    name for name, _width in SPLIT_BUCKETS
)
PRESEALED_SPLITS: Final[tuple[SplitName, ...]] = (
    "train",
    "development",
    "calibration",
    "threshold",
)
SEALED_TEST_SPLITS: Final[tuple[SplitName, ...]] = ("test",)
SPLIT_NAMESPACE: Final[str] = "keyswitch:intent-v14:physical-signature"
SPLIT_HASH_NAMESPACE: Final[bytes] = SPLIT_NAMESPACE.encode("ascii") + b"\0"
SEALED_REGISTRY_RELATIVE_PATH: Final[str] = (
    "model/intent_v1/seal-registry-v14.json"
)
UNKNOWN_TYPO_DEVELOPMENT_RANK_NAMESPACE: Final[str] = (
    "keyswitch:intent-v1:unknown-typo-rank"
)
UNKNOWN_TYPO_DEVELOPMENT_CHOICE_NAMESPACE: Final[str] = (
    "keyswitch:intent-v1:unknown-typo-choice"
)
UNKNOWN_TYPO_HOLDOUT_RANK_NAMESPACE: Final[str] = (
    "keyswitch:intent-v14:unknown-typo-holdout-rank"
)
UNKNOWN_TYPO_HOLDOUT_CHOICE_NAMESPACE: Final[str] = (
    "keyswitch:intent-v14:unknown-typo-holdout-choice"
)
HARD_NEGATIVE_ROLE_NAMESPACE: Final[str] = (
    "keyswitch:intent-v14:unknown-typo-development-role"
)
HARD_NEGATIVE_SOURCE_RELATIVE_PATH: Final[str] = (
    "model/intent_v1/unknown-typo-development-v14.json"
)
SAFETY_COLLISION_MINIMUM_WORD_LENGTH: Final[int] = 3
PROJECT_ROOT: Final[Path] = Path(__file__).resolve().parents[1]
INTENT_RUNTIME_PATH: Final[Path] = (
    PROJECT_ROOT / "src/keyswitch/intent_model.py"
)
LAYOUTS_RUNTIME_PATH: Final[Path] = PROJECT_ROOT / "src/keyswitch/layouts.py"
LANGUAGE_MODEL_RUNTIME_PATH: Final[Path] = (
    PROJECT_ROOT / "src/keyswitch/language_model.py"
)
DETECTOR_RUNTIME_PATH: Final[Path] = PROJECT_ROOT / "src/keyswitch/detector.py"
PROTECTED_TOKENS_RUNTIME_PATH: Final[Path] = (
    PROJECT_ROOT / "src/keyswitch/resources/protected_tokens.txt"
)
EVALUATOR_PATH: Final[Path] = PROJECT_ROOT / "tools/evaluate_intent_model.py"
PRESEAL_GENERATOR_PATH: Final[Path] = (
    PROJECT_ROOT / "tools/preseal_intent_holdout.py"
)
DEVELOPMENT_FREEZER_PATH: Final[Path] = (
    PROJECT_ROOT / "tools/freeze_intent_development_corpus.py"
)
PRESEAL_RECEIPT_PATH: Final[Path] = (
    PROJECT_ROOT / "model/intent_v1/holdout-v14-preseal.json"
)
MAX_TRAINING_CONFIG_BYTES: Final[int] = 1 << 16
MAX_FROZEN_SOURCE_BYTES: Final[int] = 1 << 26
MAX_PUBLICATION_BACKUP_BYTES: Final[int] = 1 << 26
MAX_SEAL_REGISTRY_BYTES: Final[int] = 1 << 14
WILSON_INTERVAL_CONFIDENCE: Final[float] = 0.95
WILSON_95_Z_SCORE: Final[float] = 1.959963984540054
# The primary threshold family contains one overall and one typo-tail false-
# positive gate for each trigger.  Fixed context-stress profiles repeat this
# already-defined family on deterministic perturbations of the same rows.
SELECTION_FALSE_POSITIVE_COMPARISONS: Final[int] = len(MODEL_TRIGGERS) * 2
SELECTION_PER_COMPARISON_CONFIDENCE: Final[float] = (
    1.0
    - (1.0 - WILSON_INTERVAL_CONFIDENCE)
    / SELECTION_FALSE_POSITIVE_COMPARISONS
)
# NormalDist().inv_cdf(1 - (1 - 0.95) / (2 * 12)), pinned rather than
# recomputed so signed reports remain byte-reproducible across Python builds.
SELECTION_WILSON_Z_SCORE: Final[float] = 2.8652602385321333


@dataclass(frozen=True)
class FrozenSourceFile:
    """One immutable training input pinned by path, byte size and SHA-256."""

    path: str
    sha256: str
    bytes: int

    def validate(self, label: str) -> None:
        source_path = Path(self.path)
        if (
            not self.path
            or source_path.is_absolute()
            or ".." in source_path.parts
        ):
            raise ValueError(f"{label}.path must be a repository-relative path")
        if (
            len(self.sha256) != 64
            or any(character not in "0123456789abcdef" for character in self.sha256)
        ):
            raise ValueError(f"{label}.sha256 must be lowercase hexadecimal")
        if (
            isinstance(self.bytes, bool)
            or not 1 <= self.bytes <= MAX_FROZEN_SOURCE_BYTES
        ):
            raise ValueError(
                f"{label}.bytes must be between 1 and "
                f"{MAX_FROZEN_SOURCE_BYTES}"
            )


@dataclass(frozen=True)
class FrozenLanguageSource(FrozenSourceFile):
    group: int

    def validate(self, label: str) -> None:
        super().validate(label)
        if self.group not in (0, 1):
            raise ValueError(f"{label}.group must be 0 or 1")


@dataclass(frozen=True)
class TrainingSources:
    package: str
    package_version: str
    license_declaration: str
    license_evidence: FrozenSourceFile
    english: FrozenLanguageSource
    russian: FrozenLanguageSource

    def validate(self) -> None:
        if not self.package or not self.package_version:
            raise ValueError("source package metadata must not be empty")
        if self.license_declaration != "GPL-3+":
            raise ValueError("source license declaration must match frozen evidence")
        self.license_evidence.validate("sources.license_evidence")
        self.english.validate("sources.languages.en_US")
        self.russian.validate("sources.languages.ru_RU")
        if self.english.group != 0 or self.russian.group != 1:
            raise ValueError("frozen EN/RU source groups must be 0 and 1")
        paths = {
            self.license_evidence.path,
            self.english.path,
            self.russian.path,
        }
        if len(paths) != 3:
            raise ValueError("frozen source paths must be distinct")


@dataclass(frozen=True)
class FrozenExternalLocalePolicy:
    """Hashes and sizes of one immutable Hunspell evaluation snapshot."""

    dictionary_sha256: str
    dictionary_bytes: int
    affix_sha256: str
    affix_bytes: int

    def validate(self, label: str) -> None:
        for field_name, digest in (
            ("dictionary_sha256", self.dictionary_sha256),
            ("affix_sha256", self.affix_sha256),
        ):
            if len(digest) != 64 or any(
                character not in "0123456789abcdef" for character in digest
            ):
                raise ValueError(
                    f"{label}.{field_name} must be exact lowercase SHA-256"
                )
        for field_name, size in (
            ("dictionary_bytes", self.dictionary_bytes),
            ("affix_bytes", self.affix_bytes),
        ):
            if (
                isinstance(size, bool)
                or not 1 <= size <= MAX_FROZEN_SOURCE_BYTES
            ):
                raise ValueError(
                    f"{label}.{field_name} must be between 1 and "
                    f"{MAX_FROZEN_SOURCE_BYTES}"
                )


@dataclass(frozen=True)
class FrozenExternalEvaluationPolicy:
    """Release-pinned policy for the independent Hunspell evaluation."""

    schema_version: int
    minimum_words_per_group: int
    trigger_expansion: tuple[CorrectionTrigger, ...]
    english: FrozenExternalLocalePolicy
    russian: FrozenExternalLocalePolicy
    lexical_disjoint_corpus_sha256: str
    unknown_typo_development_corpus_sha256: str
    unknown_typo_holdout_corpus_sha256: str

    def validate(self) -> None:
        if self.schema_version != 2:
            raise ValueError("unsupported external evaluation policy schema")
        if self.minimum_words_per_group < 5_000:
            raise ValueError(
                "external evaluation requires at least 5000 words per group"
            )
        if self.trigger_expansion != tuple(MODEL_TRIGGERS):
            raise ValueError(
                "external evaluation trigger_expansion must exactly match "
                "the runtime trigger order"
            )
        self.english.validate("external_evaluation.hunspell.en_US")
        self.russian.validate("external_evaluation.hunspell.ru_RU")
        for field_name, digest in (
            (
                "lexical_disjoint_corpus_sha256",
                self.lexical_disjoint_corpus_sha256,
            ),
            (
                "unknown_typo_development_corpus_sha256",
                self.unknown_typo_development_corpus_sha256,
            ),
            (
                "unknown_typo_holdout_corpus_sha256",
                self.unknown_typo_holdout_corpus_sha256,
            ),
        ):
            if len(digest) != 64 or any(
                character not in "0123456789abcdef" for character in digest
            ):
                raise ValueError(
                    f"external_evaluation.{field_name} must be exact "
                    "lowercase SHA-256"
                )


@dataclass(frozen=True)
class HardNegativeDevelopmentPolicy:
    """Signed source and disjoint roles for model-blind hard negatives."""

    schema_version: int
    source: FrozenSourceFile
    role_namespace: str
    train_words_per_group: int
    development_words_per_group: int
    calibration_words_per_group: int
    threshold_words_per_group: int
    training_example_weight: float

    def role_counts(self) -> dict[SplitName, int]:
        return {
            "train": self.train_words_per_group,
            "development": self.development_words_per_group,
            "calibration": self.calibration_words_per_group,
            "threshold": self.threshold_words_per_group,
            "test": 0,
        }

    def validate(self, *, expected_words_per_group: int) -> None:
        if self.schema_version != 2:
            raise ValueError("unsupported hard-negative development schema")
        self.source.validate("hard_negative_development.source")
        if self.source.path != HARD_NEGATIVE_SOURCE_RELATIVE_PATH:
            raise ValueError(
                "hard-negative development source must match the versioned path"
            )
        if self.role_namespace != HARD_NEGATIVE_ROLE_NAMESPACE:
            raise ValueError(
                "hard-negative development role namespace must match v14"
            )
        counts = self.role_counts()
        if any(
            isinstance(value, bool) or value < 1
            for split, value in counts.items()
            if split != "test"
        ):
            raise ValueError(
                "hard-negative pre-sealed role counts must be positive integers"
            )
        if counts["test"] != 0 or sum(counts.values()) != expected_words_per_group:
            raise ValueError(
                "hard-negative role counts must exhaust the model-blind "
                "development words without a test role"
            )
        if (
            isinstance(self.training_example_weight, bool)
            or not math.isfinite(self.training_example_weight)
            or not 0.25 <= self.training_example_weight <= 8.0
        ):
            raise ValueError(
                "hard-negative training example weight must be finite and "
                "between 0.25 and 8.0"
            )


@dataclass(frozen=True)
class TrainingToolchainSnapshot:
    """Hashes of executable inputs that determine the serialized artifact."""

    config_sha256: str
    trainer_sha256: str
    runtime_sha256: str
    layouts_sha256: str
    language_model_sha256: str
    detector_sha256: str
    protected_tokens_sha256: str
    evaluator_sha256: str
    preseal_generator_sha256: str
    development_freezer_sha256: str
    preseal_receipt_sha256: str
    python_implementation: str
    python_version: str
    python_build: str
    system: str
    machine: str
    libc: str
    byteorder: str


@dataclass(frozen=True)
class SealedEvaluationPolicy:
    """Versioned one-candidate policy for the held-out test namespace."""

    schema_version: int
    split_namespace: str
    registry_path: str

    def validate(self) -> None:
        if self.schema_version != 1:
            raise ValueError("unsupported sealed_evaluation schema")
        if self.split_namespace != SPLIT_NAMESPACE:
            raise ValueError(
                "sealed_evaluation.split_namespace must match the trainer "
                "split namespace"
            )
        if (
            not self.registry_path
            or Path(self.registry_path).is_absolute()
            or ".." in Path(self.registry_path).parts
            or Path(self.registry_path).name in {"", ".", ".."}
            or not self.registry_path.endswith(".json")
        ):
            raise ValueError(
                "sealed_evaluation.registry_path must be a repository-relative "
                "JSON path"
            )
        if self.registry_path != SEALED_REGISTRY_RELATIVE_PATH:
            raise ValueError(
                "sealed_evaluation.registry_path must match the versioned "
                "trainer registry path"
            )


@dataclass(frozen=True)
class SealedEvaluationReceipt:
    """Immutable receipt returned before the sealed test can be evaluated."""

    schema_version: int
    split_namespace: str
    candidate_sha256: str
    config_sha256: str
    candidate_dataset_sha256: str
    registry_relative_path: str
    registry_sha256: str
    registry_path: Path = field(repr=False, compare=False)
    registry_bytes: bytes = field(repr=False, compare=False)

    def payload(self) -> dict[str, object]:
        return {
            "schema_version": self.schema_version,
            "split_namespace": self.split_namespace,
            "candidate_sha256": self.candidate_sha256,
            "config_sha256": self.config_sha256,
            "candidate_dataset_sha256": self.candidate_dataset_sha256,
            "registry_path": self.registry_relative_path,
            "registry_sha256": self.registry_sha256,
        }


@dataclass(frozen=True)
class TrainingConfig:
    """Fully explicit, reproducible training configuration."""

    schema_version: int
    seed: int
    dimension: int
    feature_hash_seed: int
    membership_hash_seed: int
    sources: TrainingSources
    external_evaluation: FrozenExternalEvaluationPolicy
    hard_negative_development: HardNegativeDevelopmentPolicy
    sealed_evaluation: SealedEvaluationPolicy
    minimum_word_length: int
    maximum_word_length: int
    maximum_words_per_language: int
    typo_augmentations: int
    maximum_epochs: int
    minimum_epochs: int
    patience: int
    ftrl_alpha: float
    ftrl_beta: float
    ftrl_l1: float
    ftrl_l2: float
    calibration_l2: float
    calibration_max_iterations: int
    threshold_precision_floor: float
    threshold_max_false_positive_rate: float
    pause_threshold_max_false_positive_rate: float
    selection_maximum_false_positives_per_trigger: int
    threshold_logit_margin_cap: float
    pause_logit_margin: float
    veto_positive_quantile: float
    veto_logit_margin: float
    veto_max_false_negative_rate: float
    selection_minimum_recall: float
    selection_minimum_pause_recall: float
    selection_minimum_typo_recall: float
    selection_minimum_pause_typo_recall: float
    test_minimum_precision: float
    test_minimum_recall: float
    test_minimum_pause_recall: float
    test_minimum_typo_recall: float
    test_minimum_pause_typo_recall: float
    test_minimum_specificity: float
    safety_maximum_guard_failures: int

    def validate(self) -> None:
        if self.schema_version != 13:
            raise ValueError("unsupported training config schema")
        if (
            self.dimension < 256
            or self.dimension > MAX_DIMENSION
            or self.dimension & (self.dimension - 1)
        ):
            raise ValueError(
                "dimension must be a power of two between 256 and MAX_DIMENSION"
            )
        if not 0 <= self.feature_hash_seed <= (1 << 64) - 1:
            raise ValueError("feature_hash_seed must be an unsigned 64-bit integer")
        if not 0 <= self.membership_hash_seed <= (1 << 64) - 1:
            raise ValueError("membership_hash_seed must be an unsigned 64-bit integer")
        if self.membership_hash_seed == self.feature_hash_seed:
            raise ValueError("membership_hash_seed must differ from feature_hash_seed")
        self.sources.validate()
        self.external_evaluation.validate()
        self.hard_negative_development.validate(
            expected_words_per_group=(
                self.external_evaluation.minimum_words_per_group
            )
        )
        if self.hard_negative_development.source.path in {
            self.sources.license_evidence.path,
            self.sources.english.path,
            self.sources.russian.path,
        }:
            raise ValueError("hard-negative source path must be distinct")
        self.sealed_evaluation.validate()
        if not 2 <= self.minimum_word_length <= self.maximum_word_length:
            raise ValueError("invalid word-length bounds")
        if self.maximum_words_per_language != 0:
            raise ValueError(
                "maximum_words_per_language must be zero so held-out rows "
                "cannot affect candidate truncation"
            )
        if not 1 <= self.typo_augmentations <= 3:
            raise ValueError("typo_augmentations must be between one and three")
        if not 1 <= self.minimum_epochs <= self.maximum_epochs:
            raise ValueError("invalid epoch bounds")
        if self.patience < 1:
            raise ValueError("patience must be positive")
        if self.ftrl_alpha <= 0.0 or self.ftrl_beta < 0.0:
            raise ValueError("invalid FTRL learning-rate parameters")
        if self.ftrl_l1 < 0.0 or self.ftrl_l2 < 0.0:
            raise ValueError("FTRL regularisation cannot be negative")
        if self.calibration_l2 <= 0.0 or self.calibration_max_iterations < 1:
            raise ValueError("invalid calibration parameters")
        for name, value in (
            ("threshold_precision_floor", self.threshold_precision_floor),
            ("selection_minimum_recall", self.selection_minimum_recall),
            (
                "selection_minimum_pause_recall",
                self.selection_minimum_pause_recall,
            ),
            (
                "selection_minimum_typo_recall",
                self.selection_minimum_typo_recall,
            ),
            (
                "selection_minimum_pause_typo_recall",
                self.selection_minimum_pause_typo_recall,
            ),
            ("test_minimum_precision", self.test_minimum_precision),
            ("test_minimum_recall", self.test_minimum_recall),
            ("test_minimum_pause_recall", self.test_minimum_pause_recall),
            ("test_minimum_typo_recall", self.test_minimum_typo_recall),
            (
                "test_minimum_pause_typo_recall",
                self.test_minimum_pause_typo_recall,
            ),
            ("test_minimum_specificity", self.test_minimum_specificity),
        ):
            if not 0.0 <= value <= 1.0:
                raise ValueError(f"{name} must be in [0, 1]")
        for name, selection_value, sealed_value in (
            (
                "selection_minimum_recall",
                self.selection_minimum_recall,
                self.test_minimum_recall,
            ),
            (
                "selection_minimum_pause_recall",
                self.selection_minimum_pause_recall,
                self.test_minimum_pause_recall,
            ),
            (
                "selection_minimum_typo_recall",
                self.selection_minimum_typo_recall,
                self.test_minimum_typo_recall,
            ),
            (
                "selection_minimum_pause_typo_recall",
                self.selection_minimum_pause_typo_recall,
                self.test_minimum_pause_typo_recall,
            ),
        ):
            if selection_value < sealed_value:
                raise ValueError(
                    f"{name} must not be below its sealed-test minimum"
                )
        if not 0.0 <= self.threshold_max_false_positive_rate <= 1.0:
            raise ValueError("threshold_max_false_positive_rate must be in [0, 1]")
        if not 0.0 <= self.pause_threshold_max_false_positive_rate <= 1.0:
            raise ValueError(
                "pause_threshold_max_false_positive_rate must be in [0, 1]"
            )
        if (
            isinstance(
                self.selection_maximum_false_positives_per_trigger,
                bool,
            )
            or not 0
            <= self.selection_maximum_false_positives_per_trigger
            <= 1
        ):
            raise ValueError(
                "selection_maximum_false_positives_per_trigger must be a "
                "pre-sealed budget between zero and one"
            )
        for name, value in (
            ("threshold_logit_margin_cap", self.threshold_logit_margin_cap),
            ("pause_logit_margin", self.pause_logit_margin),
        ):
            if not math.isfinite(value) or value < 0.0:
                raise ValueError(f"{name} must be finite and non-negative")
        if not 0.0 <= self.veto_positive_quantile <= 0.1:
            raise ValueError("veto_positive_quantile must be in [0, 0.1]")
        if self.veto_logit_margin < 0.0:
            raise ValueError("veto_logit_margin cannot be negative")
        if not 0.0 <= self.veto_max_false_negative_rate <= 1.0:
            raise ValueError("veto_max_false_negative_rate must be in [0, 1]")
        if self.safety_maximum_guard_failures < 0:
            raise ValueError("safety_maximum_guard_failures cannot be negative")


def _mapping(value: object, label: str) -> dict[str, object]:
    if not isinstance(value, dict):
        raise ValueError(f"{label} must be an object")
    raw = cast(dict[object, object], value)
    if not all(isinstance(key, str) for key in raw):
        raise ValueError(f"{label} keys must be strings")
    return {cast(str, key): item for key, item in raw.items()}


def _require_keys(
    mapping: Mapping[str, object], expected: Collection[str], label: str
) -> None:
    actual = set(mapping)
    required = set(expected)
    if actual != required:
        missing = sorted(required - actual)
        extra = sorted(actual - required)
        raise ValueError(f"{label} fields mismatch; missing={missing}, extra={extra}")


def _string(mapping: Mapping[str, object], key: str) -> str:
    value = mapping.get(key)
    if not isinstance(value, str) or not value:
        raise ValueError(f"{key} must be a non-empty string")
    return value


def _integer(mapping: Mapping[str, object], key: str) -> int:
    value = mapping.get(key)
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError(f"{key} must be an integer")
    return value


def _number(mapping: Mapping[str, object], key: str) -> float:
    value = mapping.get(key)
    if not isinstance(value, (int, float)) or isinstance(value, bool):
        raise ValueError(f"{key} must be a number")
    result = float(value)
    if not math.isfinite(result):
        raise ValueError(f"{key} must be finite")
    return result


def _frozen_source_file(
    mapping: Mapping[str, object], *, label: str
) -> FrozenSourceFile:
    _require_keys(mapping, {"path", "sha256", "bytes"}, label)
    return FrozenSourceFile(
        _string(mapping, "path"),
        _string(mapping, "sha256"),
        _integer(mapping, "bytes"),
    )


def _frozen_language_source(
    mapping: Mapping[str, object], *, label: str
) -> FrozenLanguageSource:
    _require_keys(mapping, {"group", "path", "sha256", "bytes"}, label)
    return FrozenLanguageSource(
        _string(mapping, "path"),
        _string(mapping, "sha256"),
        _integer(mapping, "bytes"),
        _integer(mapping, "group"),
    )


def _frozen_external_locale_policy(
    mapping: Mapping[str, object], *, label: str
) -> FrozenExternalLocalePolicy:
    _require_keys(
        mapping,
        {
            "dictionary_sha256",
            "dictionary_bytes",
            "affix_sha256",
            "affix_bytes",
        },
        label,
    )
    return FrozenExternalLocalePolicy(
        dictionary_sha256=_string(mapping, "dictionary_sha256"),
        dictionary_bytes=_integer(mapping, "dictionary_bytes"),
        affix_sha256=_string(mapping, "affix_sha256"),
        affix_bytes=_integer(mapping, "affix_bytes"),
    )


def _external_trigger_expansion(value: object) -> tuple[CorrectionTrigger, ...]:
    if not isinstance(value, list):
        raise ValueError("external_evaluation.trigger_expansion must be an array")
    raw = cast(list[object], value)
    if not all(isinstance(item, str) for item in raw):
        raise ValueError(
            "external_evaluation.trigger_expansion entries must be strings"
        )
    return cast(tuple[CorrectionTrigger, ...], tuple(raw))


def _decode_training_config(text: str) -> TrainingConfig:
    """Decode strict JSON without consulting a mutable path twice."""

    def unique_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
        result: dict[str, object] = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(
                    f"training config contains duplicate key {key!r}"
                )
            result[key] = value
        return result

    try:
        decoded: object = json.loads(text, object_pairs_hook=unique_object)
    except (json.JSONDecodeError, RecursionError) as error:
        raise ValueError("training config must contain valid JSON") from error
    root = _mapping(decoded, "config")
    _require_keys(
        root,
        {
            "schema_version",
            "seed",
            "dimension",
            "feature_hash_seed",
            "membership_hash_seed",
            "ngram_orders",
            "split_buckets",
            "sources",
            "external_evaluation",
            "hard_negative_development",
            "sealed_evaluation",
            "dataset",
            "ftrl",
            "calibration",
            "thresholds",
            "quality_gates",
        },
        "config",
    )
    sources = _mapping(root.get("sources"), "sources")
    _require_keys(
        sources,
        {
            "package",
            "package_version",
            "license_declaration",
            "license_evidence",
            "languages",
        },
        "sources",
    )
    license_evidence = _mapping(
        sources.get("license_evidence"), "sources.license_evidence"
    )
    languages = _mapping(sources.get("languages"), "sources.languages")
    _require_keys(languages, {"en_US", "ru_RU"}, "sources.languages")
    english_source = _mapping(
        languages.get("en_US"), "sources.languages.en_US"
    )
    russian_source = _mapping(
        languages.get("ru_RU"), "sources.languages.ru_RU"
    )
    external_evaluation = _mapping(
        root.get("external_evaluation"), "external_evaluation"
    )
    _require_keys(
        external_evaluation,
        {
            "schema_version",
            "minimum_words_per_group",
            "trigger_expansion",
            "hunspell",
            "lexical_disjoint_corpus_sha256",
            "unknown_typo_development_corpus_sha256",
            "unknown_typo_holdout_corpus_sha256",
        },
        "external_evaluation",
    )
    external_hunspell = _mapping(
        external_evaluation.get("hunspell"), "external_evaluation.hunspell"
    )
    _require_keys(
        external_hunspell,
        {"en_US", "ru_RU"},
        "external_evaluation.hunspell",
    )
    external_english = _mapping(
        external_hunspell.get("en_US"),
        "external_evaluation.hunspell.en_US",
    )
    external_russian = _mapping(
        external_hunspell.get("ru_RU"),
        "external_evaluation.hunspell.ru_RU",
    )
    hard_negative_development = _mapping(
        root.get("hard_negative_development"),
        "hard_negative_development",
    )
    _require_keys(
        hard_negative_development,
        {
            "schema_version",
            "source",
            "role_namespace",
            "words_per_group",
            "training_example_weight",
        },
        "hard_negative_development",
    )
    hard_negative_source = _mapping(
        hard_negative_development.get("source"),
        "hard_negative_development.source",
    )
    hard_negative_counts = _mapping(
        hard_negative_development.get("words_per_group"),
        "hard_negative_development.words_per_group",
    )
    _require_keys(
        hard_negative_counts,
        set(PRESEALED_SPLITS),
        "hard_negative_development.words_per_group",
    )
    sealed_evaluation = _mapping(
        root.get("sealed_evaluation"), "sealed_evaluation"
    )
    _require_keys(
        sealed_evaluation,
        {"schema_version", "split_namespace", "registry_path"},
        "sealed_evaluation",
    )
    dataset = _mapping(root.get("dataset"), "dataset")
    _require_keys(
        dataset,
        {
            "minimum_word_length",
            "maximum_word_length",
            "maximum_words_per_language",
            "typo_augmentations",
        },
        "dataset",
    )
    ftrl = _mapping(root.get("ftrl"), "ftrl")
    _require_keys(
        ftrl,
        {"maximum_epochs", "minimum_epochs", "patience", "alpha", "beta", "l1", "l2"},
        "ftrl",
    )
    calibration = _mapping(root.get("calibration"), "calibration")
    _require_keys(calibration, {"l2", "maximum_iterations"}, "calibration")
    thresholds = _mapping(root.get("thresholds"), "thresholds")
    _require_keys(
        thresholds,
        {
            "precision_floor",
            "maximum_false_positive_rate",
            "pause_maximum_false_positive_rate",
            "selection_maximum_false_positives_per_trigger",
            "threshold_logit_margin_cap",
            "pause_logit_margin",
            "veto_positive_quantile",
            "veto_logit_margin",
            "veto_max_false_negative_rate",
        },
        "thresholds",
    )
    gates = _mapping(root.get("quality_gates"), "quality_gates")
    _require_keys(
        gates,
        {
            "minimum_precision",
            "minimum_recall",
            "minimum_pause_recall",
            "minimum_typo_recall",
            "minimum_pause_typo_recall",
            "selection_minimum_recall",
            "selection_minimum_pause_recall",
            "selection_minimum_typo_recall",
            "selection_minimum_pause_typo_recall",
            "minimum_specificity",
            "safety_maximum_guard_failures",
        },
        "quality_gates",
    )
    config = TrainingConfig(
        schema_version=_integer(root, "schema_version"),
        seed=_integer(root, "seed"),
        dimension=_integer(root, "dimension"),
        feature_hash_seed=_integer(root, "feature_hash_seed"),
        membership_hash_seed=_integer(root, "membership_hash_seed"),
        sources=TrainingSources(
            package=_string(sources, "package"),
            package_version=_string(sources, "package_version"),
            license_declaration=_string(sources, "license_declaration"),
            license_evidence=_frozen_source_file(
                license_evidence, label="sources.license_evidence"
            ),
            english=_frozen_language_source(
                english_source, label="sources.languages.en_US"
            ),
            russian=_frozen_language_source(
                russian_source, label="sources.languages.ru_RU"
            ),
        ),
        external_evaluation=FrozenExternalEvaluationPolicy(
            schema_version=_integer(external_evaluation, "schema_version"),
            minimum_words_per_group=_integer(
                external_evaluation, "minimum_words_per_group"
            ),
            trigger_expansion=_external_trigger_expansion(
                external_evaluation.get("trigger_expansion")
            ),
            english=_frozen_external_locale_policy(
                external_english,
                label="external_evaluation.hunspell.en_US",
            ),
            russian=_frozen_external_locale_policy(
                external_russian,
                label="external_evaluation.hunspell.ru_RU",
            ),
            lexical_disjoint_corpus_sha256=_string(
                external_evaluation, "lexical_disjoint_corpus_sha256"
            ),
            unknown_typo_development_corpus_sha256=_string(
                external_evaluation,
                "unknown_typo_development_corpus_sha256",
            ),
            unknown_typo_holdout_corpus_sha256=_string(
                external_evaluation,
                "unknown_typo_holdout_corpus_sha256",
            ),
        ),
        hard_negative_development=HardNegativeDevelopmentPolicy(
            schema_version=_integer(
                hard_negative_development, "schema_version"
            ),
            source=_frozen_source_file(
                hard_negative_source,
                label="hard_negative_development.source",
            ),
            role_namespace=_string(
                hard_negative_development, "role_namespace"
            ),
            train_words_per_group=_integer(
                hard_negative_counts, "train"
            ),
            development_words_per_group=_integer(
                hard_negative_counts, "development"
            ),
            calibration_words_per_group=_integer(
                hard_negative_counts, "calibration"
            ),
            threshold_words_per_group=_integer(
                hard_negative_counts, "threshold"
            ),
            training_example_weight=_number(
                hard_negative_development, "training_example_weight"
            ),
        ),
        sealed_evaluation=SealedEvaluationPolicy(
            schema_version=_integer(sealed_evaluation, "schema_version"),
            split_namespace=_string(
                sealed_evaluation, "split_namespace"
            ),
            registry_path=_string(sealed_evaluation, "registry_path"),
        ),
        minimum_word_length=_integer(dataset, "minimum_word_length"),
        maximum_word_length=_integer(dataset, "maximum_word_length"),
        maximum_words_per_language=_integer(
            dataset, "maximum_words_per_language"
        ),
        typo_augmentations=_integer(dataset, "typo_augmentations"),
        maximum_epochs=_integer(ftrl, "maximum_epochs"),
        minimum_epochs=_integer(ftrl, "minimum_epochs"),
        patience=_integer(ftrl, "patience"),
        ftrl_alpha=_number(ftrl, "alpha"),
        ftrl_beta=_number(ftrl, "beta"),
        ftrl_l1=_number(ftrl, "l1"),
        ftrl_l2=_number(ftrl, "l2"),
        calibration_l2=_number(calibration, "l2"),
        calibration_max_iterations=_integer(calibration, "maximum_iterations"),
        threshold_precision_floor=_number(thresholds, "precision_floor"),
        threshold_max_false_positive_rate=_number(
            thresholds, "maximum_false_positive_rate"
        ),
        pause_threshold_max_false_positive_rate=_number(
            thresholds, "pause_maximum_false_positive_rate"
        ),
        selection_maximum_false_positives_per_trigger=_integer(
            thresholds,
            "selection_maximum_false_positives_per_trigger",
        ),
        threshold_logit_margin_cap=_number(
            thresholds, "threshold_logit_margin_cap"
        ),
        pause_logit_margin=_number(thresholds, "pause_logit_margin"),
        veto_positive_quantile=_number(thresholds, "veto_positive_quantile"),
        veto_logit_margin=_number(thresholds, "veto_logit_margin"),
        veto_max_false_negative_rate=_number(
            thresholds, "veto_max_false_negative_rate"
        ),
        selection_minimum_recall=_number(
            gates, "selection_minimum_recall"
        ),
        selection_minimum_pause_recall=_number(
            gates, "selection_minimum_pause_recall"
        ),
        selection_minimum_typo_recall=_number(
            gates, "selection_minimum_typo_recall"
        ),
        selection_minimum_pause_typo_recall=_number(
            gates, "selection_minimum_pause_typo_recall"
        ),
        test_minimum_precision=_number(gates, "minimum_precision"),
        test_minimum_recall=_number(gates, "minimum_recall"),
        test_minimum_pause_recall=_number(gates, "minimum_pause_recall"),
        test_minimum_typo_recall=_number(gates, "minimum_typo_recall"),
        test_minimum_pause_typo_recall=_number(
            gates, "minimum_pause_typo_recall"
        ),
        test_minimum_specificity=_number(gates, "minimum_specificity"),
        safety_maximum_guard_failures=_integer(
            gates, "safety_maximum_guard_failures"
        ),
    )
    config.validate()
    expected: list[list[object]] = [
        [name, width] for name, width in SPLIT_BUCKETS
    ]
    configured_splits = root.get("split_buckets")
    if configured_splits != expected:
        raise ValueError(
            "split_buckets must exactly match the versioned "
            "65/10/10/7.5/7.5 split"
        )
    if root.get("ngram_orders") != list(NGRAM_ORDERS):
        raise ValueError("ngram_orders must match the KSLM v5 feature schema")
    return config


def load_training_config(path: Path) -> TrainingConfig:
    """Load a strict JSON config and reject implicit/defaulted parameters."""

    config, _digest = load_training_config_snapshot(path)
    return config


def load_training_config_snapshot(path: Path) -> tuple[TrainingConfig, str]:
    """Load one immutable byte snapshot and return its exact SHA-256."""

    try:
        with path.open("rb") as stream:
            raw = stream.read(MAX_TRAINING_CONFIG_BYTES + 1)
    except OSError as error:
        raise ValueError(f"training config is unavailable: {path}") from error
    if len(raw) > MAX_TRAINING_CONFIG_BYTES:
        raise ValueError(
            "training config exceeds the maximum size of "
            f"{MAX_TRAINING_CONFIG_BYTES} bytes"
        )
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError as error:
        raise ValueError("training config must be valid UTF-8") from error
    return _decode_training_config(text), hashlib.sha256(raw).hexdigest()


@dataclass(frozen=True)
class LexiconSource:
    locale: str
    group: int
    path: str
    sha256: str
    bytes: int
    license_declaration: str
    license_evidence: str


@dataclass(frozen=True)
class LexiconWord:
    word: str
    group: int
    frequency: int
    physical_signature: str
    split: SplitName


@dataclass(frozen=True)
class LexiconCollision:
    physical_signature: str
    words: tuple[LexiconWord, ...]


@dataclass(frozen=True)
class PreparedLexicon:
    words_by_split: dict[SplitName, tuple[LexiconWord, ...]]
    collisions: tuple[LexiconCollision, ...]


def sha256_file(
    path: Path, *, maximum_bytes: int = MAX_FROZEN_SOURCE_BYTES
) -> str:
    """Hash at most a declared number of bytes from a local input."""

    if maximum_bytes < 1:
        raise ValueError("maximum_bytes must be positive")
    digest = hashlib.sha256()
    total = 0
    with path.open("rb") as handle:
        while chunk := handle.read(min(1024 * 1024, maximum_bytes - total + 1)):
            total += len(chunk)
            if total > maximum_bytes:
                raise RuntimeError(f"file exceeds hashing size limit: {path}")
            digest.update(chunk)
    return digest.hexdigest()


def _stage_bytes(destination: Path, data: bytes) -> Path:
    """Durably stage bytes beside their final destination without replacing it."""

    destination.parent.mkdir(parents=True, exist_ok=True)
    staged: Path | None = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="wb",
            prefix=f".{destination.name}.",
            suffix=".staged",
            dir=destination.parent,
            delete=False,
        ) as handle:
            staged = Path(handle.name)
            written = handle.write(data)
            if written != len(data):
                raise OSError("short write while staging bytes")
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(staged, 0o644)
        return staged
    except BaseException:
        if staged is not None:
            try:
                staged.unlink()
            except FileNotFoundError:
                pass
        raise


def _fsync_directory(path: Path) -> None:
    flags = os.O_RDONLY | getattr(os, "O_DIRECTORY", 0)
    try:
        descriptor = os.open(path, flags)
    except OSError:
        if os.name == "nt":
            return
        raise
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def _atomic_write_bytes(destination: Path, data: bytes) -> None:
    staged = _stage_bytes(destination, data)
    try:
        os.replace(staged, destination)
        _fsync_directory(destination.parent)
    finally:
        try:
            staged.unlink()
        except FileNotFoundError:
            pass


def publish_bytes_bundle(outputs: Sequence[tuple[Path, bytes]]) -> None:
    """Publish a small bundle transactionally, rolling back process failures.

    Callers order the commit marker last.  Every payload is staged and fsynced
    before the first destination changes.  A raised exception restores exact
    pre-existing bytes (or absence) for every destination already replaced.
    """

    destinations = tuple(destination for destination, _data in outputs)
    if not outputs or len(set(destinations)) != len(destinations):
        raise ValueError("bundle destinations must be non-empty and unique")
    previous: dict[Path, bytes | None] = {}
    staged: dict[Path, Path] = {}
    replaced: list[Path] = []
    try:
        for destination, data in outputs:
            try:
                with destination.open("rb") as stream:
                    existing = stream.read(MAX_PUBLICATION_BACKUP_BYTES + 1)
                if len(existing) > MAX_PUBLICATION_BACKUP_BYTES:
                    raise ValueError(
                        "existing publication target exceeds the rollback limit: "
                        f"{destination}"
                    )
                previous[destination] = existing
            except FileNotFoundError:
                previous[destination] = None
            staged[destination] = _stage_bytes(destination, data)
        for destination in destinations:
            os.replace(staged[destination], destination)
            replaced.append(destination)
            _fsync_directory(destination.parent)
    except BaseException as publish_error:
        rollback_errors: list[str] = []
        for destination in reversed(replaced):
            try:
                old = previous[destination]
                if old is None:
                    destination.unlink(missing_ok=True)
                    _fsync_directory(destination.parent)
                else:
                    _atomic_write_bytes(destination, old)
            except OSError as rollback_error:
                rollback_errors.append(f"{destination}: {rollback_error}")
        if rollback_errors:
            raise RuntimeError(
                "bundle publication failed and rollback was incomplete: "
                + "; ".join(rollback_errors)
            ) from publish_error
        raise
    finally:
        for path in staged.values():
            try:
                path.unlink()
            except FileNotFoundError:
                pass


def _canonical_json_bytes(value: Mapping[str, object]) -> bytes:
    return (
        json.dumps(
            value,
            ensure_ascii=False,
            sort_keys=True,
            separators=(",", ":"),
            allow_nan=False,
        )
        + "\n"
    ).encode("utf-8")


def json_native_mapping(
    value: Mapping[str, object],
) -> dict[str, object]:
    """Normalize dataclass tuples to strict JSON containers, byte-exactly.

    The human-readable manifest accepts tuples because the JSON encoder emits
    them as arrays.  KSLM metadata deliberately accepts only JSON-native
    containers, so normalize at that single boundary and prove that the
    canonical representation is unchanged.
    """

    canonical = _canonical_json_bytes(value)
    decoded = json.loads(canonical.decode("utf-8"))
    if not isinstance(decoded, dict):
        raise RuntimeError("JSON-native metadata normalization was not an object")
    result = cast(dict[str, object], decoded)
    if _canonical_json_bytes(result) != canonical:
        raise RuntimeError("JSON-native metadata normalization changed bytes")
    return result


def _exact_sha256(value: str, label: str) -> str:
    if len(value) != 64 or any(
        character not in "0123456789abcdef" for character in value
    ):
        raise ValueError(f"{label} must be exact lowercase SHA-256")
    return value


def quantized_weights_sha256(weights: QuantizedWeights) -> str:
    """Hash the exact sparse int16 vector and its quantization contract."""

    digest = hashlib.sha256(b"KeySwitch sealed quantized weights v1\0")
    digest.update(struct.pack("<Q", weights.dimension))
    digest.update(weights.scale.hex().encode("ascii") + b"\0")
    digest.update(weights.maximum_absolute_error.hex().encode("ascii") + b"\0")
    digest.update(hashlib.sha256(weights.support).digest())
    for index, value in enumerate(weights.values):
        if value:
            digest.update(struct.pack("<Qh", index, value))
    return digest.hexdigest()


def supported_fingerprints_sha256(values: Collection[int]) -> str:
    """Hash the sorted exact uint64 membership set in platform-neutral form."""

    normalized = sorted(set(values))
    if len(normalized) != len(values) or any(
        isinstance(value, bool) or not 0 <= value <= (1 << 64) - 1
        for value in normalized
    ):
        raise ValueError("supported fingerprints must be unique uint64 values")
    digest = hashlib.sha256(b"KeySwitch sealed fingerprints v1\0")
    digest.update(struct.pack("<Q", len(normalized)))
    for value in normalized:
        digest.update(struct.pack("<Q", value))
    return digest.hexdigest()


def quantized_model_payload_sha256(
    weights: QuantizedWeights,
    supported_fingerprints: Collection[int],
) -> str:
    """Reproduce the exact KSLM weight+membership payload digest."""

    if len(weights.values) != weights.dimension:
        raise ValueError("quantized weight count must equal dimension")
    weight_bytes = bytearray(weights.dimension * 2)
    for index, value in enumerate(weights.values):
        if not -32767 <= value <= 32767:
            raise ValueError("quantized weight is outside signed int16 range")
        struct.pack_into("<h", weight_bytes, index * 2, value)
    fingerprints = sorted(set(supported_fingerprints))
    if len(fingerprints) != len(supported_fingerprints) or any(
        isinstance(value, bool) or not 0 <= value <= (1 << 64) - 1
        for value in fingerprints
    ):
        raise ValueError("supported fingerprints must be unique uint64 values")
    if len(fingerprints) > MAX_SUPPORTED_FINGERPRINTS:
        raise ValueError("supported fingerprints exceed the KSLM size limit")
    payload_bytes = (weights.dimension * 2) + (len(fingerprints) * 8)
    if payload_bytes > MAX_PAYLOAD_BYTES:
        raise ValueError("quantized model payload exceeds the KSLM size limit")
    fingerprint_bytes = bytearray(len(fingerprints) * 8)
    for index, value in enumerate(fingerprints):
        struct.pack_into("<Q", fingerprint_bytes, index * 8, value)
    return hashlib.sha256(weight_bytes + fingerprint_bytes).hexdigest()


def training_candidate_model_parameters(
    *,
    config: TrainingConfig,
    quantized: QuantizedWeights,
    supported_fingerprints: Collection[int],
    bias: float,
    calibration: DirectionalPlattCalibration,
    thresholds: Mapping[CorrectionTrigger, ThresholdSelection],
    veto: VetoSelection,
) -> dict[str, object]:
    """Describe only runtime parameters that the final KSLM must contain."""

    if set(thresholds) != set(MODEL_TRIGGERS):
        raise ValueError("sealed candidate thresholds are incomplete")
    calibration_parameters = calibration.runtime_parameters()
    if not all(
        math.isfinite(value)
        for value in (
            bias,
            veto.raw_logit,
            quantized.scale,
            *(
                parameter
                for direction in LAYOUT_DIRECTIONS
                for parameter in (
                    calibration_parameters[direction].scale,
                    calibration_parameters[direction].bias,
                )
            ),
        )
    ):
        raise ValueError("sealed candidate contains non-finite model values")
    return {
        "dimension": config.dimension,
        "payload_sha256": quantized_model_payload_sha256(
            quantized, supported_fingerprints
        ),
        "weight_scale_hex": quantized.scale.hex(),
        "bias_hex": bias.hex(),
        "platt_calibration_hex": {
            direction: {
                "scale": calibration_parameters[direction].scale.hex(),
                "bias": calibration_parameters[direction].bias.hex(),
            }
            for direction in LAYOUT_DIRECTIONS
        },
        "threshold_logits_hex": {
            trigger: {
                direction: thresholds[trigger].logit_for(direction).hex()
                for direction in LAYOUT_DIRECTIONS
            }
            for trigger in MODEL_TRIGGERS
        },
        "veto_threshold_hex": veto.raw_logit.hex(),
        "feature_hash_seed": config.feature_hash_seed,
        "membership_hash_seed": config.membership_hash_seed,
        "ngram_orders": list(NGRAM_ORDERS),
    }


def runtime_candidate_model_parameters(
    model: LinearNgramModel,
) -> dict[str, object]:
    """Extract the same candidate identity from a validated loaded KSLM."""

    return {
        "dimension": model.dimension,
        "payload_sha256": model.payload_sha256,
        "weight_scale_hex": model.weight_scale.hex(),
        "bias_hex": model.bias.hex(),
        "platt_calibration_hex": {
            direction: {
                "scale": model.platt_calibration[direction].scale.hex(),
                "bias": model.platt_calibration[direction].bias.hex(),
            }
            for direction in LAYOUT_DIRECTIONS
        },
        "threshold_logits_hex": {
            trigger: {
                direction: model.threshold_logits[trigger][direction].hex()
                for direction in LAYOUT_DIRECTIONS
            }
            for trigger in MODEL_TRIGGERS
        },
        "veto_threshold_hex": model.veto_threshold.hex(),
        "feature_hash_seed": model.fnv_seed,
        "membership_hash_seed": model.membership_seed,
        "ngram_orders": list(model.ngram_orders),
    }


def validate_presealed_candidate_serialization(
    *,
    config: TrainingConfig,
    quantized: QuantizedWeights,
    supported_fingerprints: Collection[int],
    bias: float,
    calibration: DirectionalPlattCalibration,
    thresholds: Mapping[CorrectionTrigger, ThresholdSelection],
    veto: VetoSelection,
    expected_parameters: Mapping[str, object],
) -> None:
    """Prove runtime serialization and quantization parity before test access."""

    with tempfile.TemporaryDirectory(
        prefix="keyswitch-intent-presealed-validation-"
    ) as temporary:
        model = write_model(
            Path(temporary) / "candidate.ksm",
            model_version="intent-v1-presealed-validation",
            dimension=config.dimension,
            weights=quantized.dequantized(),
            supported_fingerprints=supported_fingerprints,
            threshold_logits={
                trigger: item.runtime_logits()
                for trigger, item in thresholds.items()
            },
            veto_threshold=veto.raw_logit,
            bias=bias,
            platt_calibration=calibration.runtime_parameters(),
            fnv_seed=config.feature_hash_seed,
            membership_seed=config.membership_hash_seed,
            ngram_orders=NGRAM_ORDERS,
            metadata={"validation_phase": "presealed_candidate"},
        )
    if runtime_candidate_model_parameters(model) != dict(expected_parameters):
        raise RuntimeError(
            "presealed KSLM parameters differ after runtime serialization"
        )


PRESEALED_CANDIDATE_METADATA_KEYS: Final[frozenset[str]] = frozenset(
    {
        "schema_version",
        "model_id",
        "calibration_scope",
        "config_sha256",
        "split_namespace",
        "toolchain",
        "source_package",
        "sources",
        "candidate_counts",
        "variant_quarantine_sha256",
        "training_language_scorer",
        "gate_policy",
        "training",
        "quantization",
        "calibration",
        "veto_selection",
        "thresholds",
        "threshold_selection_gate_breakdown",
        "safety_guard_audit",
        "model_parameters",
    }
)


def presealed_candidate_counts(dataset: DatasetBundle) -> dict[str, object]:
    """Return only counts observable before the held-out split is built."""

    if dataset.by_split["test"]:
        raise ValueError("presealed candidate unexpectedly contains test rows")
    if (
        dataset.sealed_variant_quarantine.occurrences
        or dataset.sealed_test_exclusion_signatures
    ):
        raise ValueError("presealed candidate contains post-seal metadata")
    return {
        "examples": {
            split: len(dataset.by_split[split])
            for split in PRESEALED_SPLITS
        },
        "safety_examples": len(dataset.safety),
        "quarantined_variant_occurrences": (
            dataset.variant_quarantine.occurrence_count
        ),
        "quarantined_physical_signatures": (
            dataset.variant_quarantine.physical_signature_count
        ),
    }


def presealed_candidate_metadata_projection(
    *,
    model_id: str,
    calibration_scope: str,
    config_sha256: str,
    split_namespace: str,
    toolchain: Mapping[str, object],
    source_package: Mapping[str, object],
    sources: Sequence[object],
    candidate_counts: Mapping[str, object],
    variant_quarantine_sha256: str,
    training_language_scorer: Mapping[str, object],
    gate_policy: Mapping[str, object],
    training: Mapping[str, object],
    quantization: Mapping[str, object],
    calibration: Mapping[str, object],
    veto_selection: Mapping[str, object],
    thresholds: Mapping[str, object],
    selection_gate_breakdown: Mapping[str, object],
    safety_guard_audit: Mapping[str, object],
    model_parameters: Mapping[str, object],
) -> dict[str, object]:
    """Canonical exact projection of all candidate metadata known pre-test."""

    _exact_sha256(config_sha256, "config_sha256")
    _exact_sha256(
        variant_quarantine_sha256, "variant_quarantine_sha256"
    )
    if model_id != "keyswitch-layout-intent-v1":
        raise ValueError("sealed candidate model_id is unsupported")
    if calibration_scope != "lexical-synthetic-not-real-world-probability":
        raise ValueError("sealed candidate calibration scope is unsupported")
    if split_namespace != SPLIT_NAMESPACE:
        raise ValueError("sealed candidate split namespace is unsupported")
    projection: dict[str, object] = {
        "schema_version": 1,
        "model_id": model_id,
        "calibration_scope": calibration_scope,
        "config_sha256": config_sha256,
        "split_namespace": split_namespace,
        "toolchain": dict(toolchain),
        "source_package": dict(source_package),
        "sources": list(sources),
        "candidate_counts": dict(candidate_counts),
        "variant_quarantine_sha256": variant_quarantine_sha256,
        "training_language_scorer": dict(training_language_scorer),
        "gate_policy": dict(gate_policy),
        "training": dict(training),
        "quantization": dict(quantization),
        "calibration": dict(calibration),
        "veto_selection": dict(veto_selection),
        "thresholds": dict(thresholds),
        "threshold_selection_gate_breakdown": dict(
            selection_gate_breakdown
        ),
        "safety_guard_audit": dict(safety_guard_audit),
        "model_parameters": dict(model_parameters),
    }
    _require_keys(
        projection,
        PRESEALED_CANDIDATE_METADATA_KEYS,
        "presealed candidate metadata",
    )
    _canonical_json_bytes(projection)
    return projection


def sealed_candidate_sha256(
    *,
    split_namespace: str,
    config_sha256: str,
    candidate_dataset_sha256: str,
    toolchain: Mapping[str, object],
    training_language_scorer: Mapping[str, object],
    model_parameters: Mapping[str, object],
    selection_gate_breakdown: Mapping[str, object],
    candidate_metadata: Mapping[str, object],
) -> str:
    """Identify every candidate input fixed before the sealed test is exposed."""

    _exact_sha256(
        candidate_dataset_sha256, "candidate_dataset_sha256"
    )
    _exact_sha256(config_sha256, "config_sha256")
    if split_namespace != SPLIT_NAMESPACE:
        raise ValueError("sealed candidate split namespace is unsupported")
    metadata = dict(candidate_metadata)
    _require_keys(
        metadata,
        PRESEALED_CANDIDATE_METADATA_KEYS,
        "presealed candidate metadata",
    )
    if (
        metadata.get("schema_version") != 1
        or metadata.get("split_namespace") != split_namespace
        or metadata.get("config_sha256") != config_sha256
        or metadata.get("toolchain") != dict(toolchain)
        or metadata.get("training_language_scorer")
        != dict(training_language_scorer)
        or metadata.get("model_parameters") != dict(model_parameters)
        or metadata.get("threshold_selection_gate_breakdown")
        != dict(selection_gate_breakdown)
    ):
        raise ValueError(
            "presealed candidate metadata conflicts with candidate identity"
        )
    payload: dict[str, object] = {
        "schema_version": 2,
        "split_namespace": split_namespace,
        "config_sha256": config_sha256,
        "candidate_dataset_sha256": candidate_dataset_sha256,
        "toolchain": dict(toolchain),
        "training_language_scorer": dict(training_language_scorer),
        "model": dict(model_parameters),
        "threshold_selection_gate_breakdown": dict(
            selection_gate_breakdown
        ),
        "candidate_metadata": metadata,
    }
    return hashlib.sha256(_canonical_json_bytes(payload)).hexdigest()


def sealed_registry_path(
    config: TrainingConfig, *, repository_root: Path | None = None
) -> Path:
    """Resolve one repository-scoped ledger independent of config copies."""

    config.sealed_evaluation.validate()
    root = (PROJECT_ROOT if repository_root is None else repository_root).resolve()
    relative = Path(config.sealed_evaluation.registry_path)
    try:
        parent = (root / relative).parent.resolve(strict=True)
    except OSError as error:
        raise RuntimeError(
            "sealed evaluation registry parent is unavailable"
        ) from error
    if not parent.is_relative_to(root):
        raise RuntimeError(
            "sealed evaluation registry escapes the repository root"
        )
    return parent / relative.name


def _read_seal_registry_snapshot(path: Path) -> bytes:
    if path.is_symlink():
        raise RuntimeError(f"sealed evaluation registry cannot be a symlink: {path}")
    try:
        with path.open("rb") as stream:
            raw = stream.read(MAX_SEAL_REGISTRY_BYTES + 1)
    except OSError as error:
        raise RuntimeError(
            f"sealed evaluation registry is unavailable: {path}"
        ) from error
    if len(raw) > MAX_SEAL_REGISTRY_BYTES:
        raise RuntimeError("sealed evaluation registry exceeds its size limit")
    return raw


def claim_sealed_evaluation(
    *,
    config: TrainingConfig,
    candidate_sha256: str,
    config_sha256: str,
    candidate_dataset_sha256: str,
    repository_root: Path | None = None,
) -> SealedEvaluationReceipt:
    """Atomically consume a split namespace for exactly one candidate.

    An identical rerun is allowed for reproducibility. Any changed candidate
    is rejected before a held-out row is scored and requires an explicit split
    namespace plus registry-file rotation.
    """

    _exact_sha256(candidate_sha256, "candidate_sha256")
    _exact_sha256(config_sha256, "config_sha256")
    _exact_sha256(
        candidate_dataset_sha256, "candidate_dataset_sha256"
    )
    path = sealed_registry_path(config, repository_root=repository_root)
    record: dict[str, object] = {
        "schema_version": 1,
        "split_namespace": config.sealed_evaluation.split_namespace,
        "candidate_sha256": candidate_sha256,
        "config_sha256": config_sha256,
        "candidate_dataset_sha256": candidate_dataset_sha256,
    }
    expected = _canonical_json_bytes(record)
    if len(expected) > MAX_SEAL_REGISTRY_BYTES:
        raise AssertionError("sealed evaluation registry record is oversized")
    staged: Path | None = None
    try:
        staged = _stage_bytes(path, expected)
        os.link(staged, path, follow_symlinks=False)
    except FileExistsError:
        existing = _read_seal_registry_snapshot(path)
        if existing != expected:
            raise RuntimeError(
                "sealed test namespace is already consumed by another "
                "candidate; rotate split_namespace and registry_path before "
                "evaluating a changed candidate"
            )
    except OSError as error:
        raise RuntimeError(
            f"cannot create sealed evaluation registry: {path}"
        ) from error
    else:
        _fsync_directory(path.parent)
    finally:
        if staged is not None:
            try:
                staged.unlink()
            except FileNotFoundError:
                pass
    snapshot = _read_seal_registry_snapshot(path)
    if snapshot != expected:
        raise RuntimeError("sealed evaluation registry changed during claim")
    return SealedEvaluationReceipt(
        schema_version=1,
        split_namespace=config.sealed_evaluation.split_namespace,
        candidate_sha256=candidate_sha256,
        config_sha256=config_sha256,
        candidate_dataset_sha256=candidate_dataset_sha256,
        registry_relative_path=config.sealed_evaluation.registry_path,
        registry_sha256=hashlib.sha256(snapshot).hexdigest(),
        registry_path=path,
        registry_bytes=snapshot,
    )


def verify_sealed_evaluation_receipt(
    receipt: SealedEvaluationReceipt,
) -> None:
    """Fail if the one-shot ledger changes after test access was granted."""

    snapshot = _read_seal_registry_snapshot(receipt.registry_path)
    if (
        snapshot != receipt.registry_bytes
        or hashlib.sha256(snapshot).hexdigest() != receipt.registry_sha256
    ):
        raise RuntimeError("sealed evaluation registry changed after claim")


def sealed_evaluation_evidence_is_valid(
    *,
    config: TrainingConfig,
    value: object,
    expected_config_sha256: str,
    expected_candidate_dataset_sha256: str,
    expected_candidate_sha256: str,
    repository_root: Path | None = None,
) -> bool:
    """Independently validate a manifest receipt against the local ledger."""

    try:
        evidence = _mapping(value, "sealed_evaluation")
        _require_keys(
            evidence,
            {
                "schema_version",
                "split_namespace",
                "candidate_sha256",
                "config_sha256",
                "candidate_dataset_sha256",
                "registry_path",
                "registry_sha256",
            },
            "sealed_evaluation",
        )
        schema_version = _integer(evidence, "schema_version")
        split_namespace = _string(evidence, "split_namespace")
        candidate_sha256 = _exact_sha256(
            _string(evidence, "candidate_sha256"), "candidate_sha256"
        )
        config_sha256 = _exact_sha256(
            _string(evidence, "config_sha256"), "config_sha256"
        )
        candidate_dataset_sha256 = _exact_sha256(
            _string(evidence, "candidate_dataset_sha256"),
            "candidate_dataset_sha256",
        )
        registry_path_value = _string(evidence, "registry_path")
        registry_sha256 = _exact_sha256(
            _string(evidence, "registry_sha256"), "registry_sha256"
        )
        if (
            schema_version != 1
            or split_namespace != config.sealed_evaluation.split_namespace
            or candidate_sha256 != expected_candidate_sha256
            or registry_path_value != config.sealed_evaluation.registry_path
            or config_sha256 != expected_config_sha256
            or candidate_dataset_sha256
            != expected_candidate_dataset_sha256
        ):
            return False
        record: dict[str, object] = {
            "schema_version": schema_version,
            "split_namespace": split_namespace,
            "candidate_sha256": candidate_sha256,
            "config_sha256": config_sha256,
            "candidate_dataset_sha256": candidate_dataset_sha256,
        }
        snapshot = _read_seal_registry_snapshot(
            sealed_registry_path(config, repository_root=repository_root)
        )
        return (
            snapshot == _canonical_json_bytes(record)
            and hashlib.sha256(snapshot).hexdigest() == registry_sha256
        )
    except (OSError, RuntimeError, TypeError, ValueError):
        return False


def capture_toolchain_snapshot(
    config_sha256: str,
) -> TrainingToolchainSnapshot:
    """Capture code/runtime identity before feature extraction or training."""

    return TrainingToolchainSnapshot(
        config_sha256=config_sha256,
        trainer_sha256=sha256_file(Path(__file__).resolve()),
        runtime_sha256=sha256_file(INTENT_RUNTIME_PATH),
        layouts_sha256=sha256_file(LAYOUTS_RUNTIME_PATH),
        language_model_sha256=sha256_file(LANGUAGE_MODEL_RUNTIME_PATH),
        detector_sha256=sha256_file(DETECTOR_RUNTIME_PATH),
        protected_tokens_sha256=sha256_file(PROTECTED_TOKENS_RUNTIME_PATH),
        evaluator_sha256=sha256_file(EVALUATOR_PATH),
        preseal_generator_sha256=sha256_file(PRESEAL_GENERATOR_PATH),
        development_freezer_sha256=sha256_file(
            DEVELOPMENT_FREEZER_PATH
        ),
        preseal_receipt_sha256=sha256_file(
            PRESEAL_RECEIPT_PATH,
            maximum_bytes=MAX_TRAINING_CONFIG_BYTES,
        ),
        python_implementation=sys.implementation.name,
        python_version=platform.python_version(),
        python_build=" ".join(sys.version.split()),
        system=platform.system(),
        machine=platform.machine(),
        libc=" ".join(platform.libc_ver()),
        byteorder=sys.byteorder,
    )


def verify_toolchain_snapshot(
    snapshot: TrainingToolchainSnapshot, config_path: Path
) -> None:
    """Refuse publication when executable inputs changed during training."""

    current = capture_toolchain_snapshot(
        sha256_file(config_path, maximum_bytes=MAX_TRAINING_CONFIG_BYTES)
    )
    if current != snapshot:
        changed = tuple(
            field_name
            for field_name in snapshot.__dataclass_fields__
            if getattr(snapshot, field_name) != getattr(current, field_name)
        )
        raise RuntimeError(
            f"training inputs changed during execution: {changed!r}"
        )


def validate_training_paths(
    *,
    config: Path,
    english: Path,
    russian: Path,
    license_evidence: Path,
    hard_negative_source: Path,
    seal_registry: Path,
    artifact: Path,
    manifest: Path,
    report: Path,
    diagnostic: Path | None = None,
) -> None:
    """Prevent output aliases from overwriting each other or immutable inputs."""

    output_paths = (artifact, manifest, report) + (
        () if diagnostic is None else (diagnostic,)
    )
    outputs = tuple(path.resolve() for path in output_paths)
    registry = seal_registry.resolve()
    mutable_paths = (*outputs, registry)
    if len(set(mutable_paths)) != len(mutable_paths):
        raise ValueError(
            "artifact, manifest, report and seal registry paths must be distinct"
        )
    protected = tuple(
        path.resolve()
        for path in (
            config,
            english,
            russian,
            license_evidence,
            hard_negative_source,
            Path(__file__),
            INTENT_RUNTIME_PATH,
            LAYOUTS_RUNTIME_PATH,
            LANGUAGE_MODEL_RUNTIME_PATH,
            DETECTOR_RUNTIME_PATH,
            PROTECTED_TOKENS_RUNTIME_PATH,
            EVALUATOR_PATH,
            PRESEAL_GENERATOR_PATH,
            DEVELOPMENT_FREEZER_PATH,
            PRESEAL_RECEIPT_PATH,
        )
    )
    for output in mutable_paths:
        for source in protected:
            aliases = output == source
            if not aliases and output.exists() and source.exists():
                try:
                    aliases = output.samefile(source)
                except OSError:
                    aliases = False
            if aliases:
                raise ValueError(
                    f"output path aliases an immutable training input: {output}"
                )


def verify_frozen_file(
    path: Path, expected: FrozenSourceFile, *, label: str
) -> None:
    """Fail closed when a configured immutable source differs by one byte."""

    read_verified_frozen_file(path, expected, label=label)


def read_verified_frozen_file(
    path: Path, expected: FrozenSourceFile, *, label: str
) -> bytes:
    """Return exactly the bytes whose configured size and SHA-256 were checked."""

    if not 1 <= expected.bytes <= MAX_FROZEN_SOURCE_BYTES:
        raise RuntimeError(
            f"{label} configured size is outside the supported bounds"
        )
    try:
        with path.open("rb") as stream:
            raw = stream.read(expected.bytes + 1)
    except OSError as error:
        raise RuntimeError(f"{label} is unavailable: {path}") from error
    if len(raw) != expected.bytes:
        raise RuntimeError(
            f"{label} size mismatch: expected {expected.bytes}, found {len(raw)}"
        )
    digest = hashlib.sha256(raw).hexdigest()
    if digest != expected.sha256:
        raise RuntimeError(
            f"{label} SHA-256 mismatch: expected {expected.sha256}, found {digest}"
        )
    return raw


def verify_training_sources(
    config: TrainingConfig, english_path: Path, russian_path: Path
) -> Path:
    """Verify both corpora and their verbatim license evidence before parsing."""

    verify_frozen_file(
        english_path, config.sources.english, label="English training source"
    )
    verify_frozen_file(
        russian_path, config.sources.russian, label="Russian training source"
    )
    evidence_path = PROJECT_ROOT / config.sources.license_evidence.path
    evidence_bytes = read_verified_frozen_file(
        evidence_path,
        config.sources.license_evidence,
        label="Onboard license evidence",
    )
    try:
        evidence_lines = frozenset(
            evidence_bytes.decode("utf-8").splitlines()
        )
    except UnicodeDecodeError as error:
        raise RuntimeError(
            f"Onboard license evidence is not valid UTF-8: {evidence_path}"
        ) from error
    required_lines = {
        "Files: models/*",
        "Copyright: 2013, 2014, marmuta <marmvta@gmail.com>",
        "  2011, 2012, Francesco Fumanti <francesco.fumanti@gmx.net>",
        f"License: {config.sources.license_declaration}",
    }
    missing = sorted(required_lines - evidence_lines)
    if missing:
        raise RuntimeError(
            f"Onboard license evidence is missing required lines: {missing}"
        )
    return evidence_path


def _normalise_lexical_word(word: str, group: int) -> str:
    normalized = word.casefold().strip()
    if group == 0:
        alphabet = frozenset("abcdefghijklmnopqrstuvwxyz'-")
    elif group == 1:
        alphabet = frozenset("абвгдеёжзийклмнопрстуфхцчшщъыьэюя'-")
    else:
        raise ValueError("only EN/RU groups 0 and 1 are supported")
    return normalized if normalized and all(char in alphabet for char in normalized) else ""


def physical_signature(word: str, group: int, pair: LayoutPair | None = None) -> str:
    """Return the layout-independent physical key sequence for a word."""

    normalized = _normalise_lexical_word(word, group)
    if not normalized:
        return ""
    layout_pair = pair or LayoutPair()
    if group == 0:
        return normalized
    return layout_pair.translate(normalized, "ru", "us")


def stable_split(signature: str) -> SplitName:
    """Assign a physical signature to the immutable 40-bucket split."""

    digest = hashlib.sha256(SPLIT_HASH_NAMESPACE + signature.encode("utf-8")).digest()
    bucket = int.from_bytes(digest[:8], "big") % 40
    cursor = 0
    for name, width in SPLIT_BUCKETS:
        cursor += width
        if bucket < cursor:
            return name
    raise AssertionError("split bucket table does not cover all 40 buckets")


def deterministic_training_trigger(signature: str) -> CorrectionTrigger:
    digest = hashlib.sha256(
        b"keyswitch:intent-v1:training-trigger\0" + signature.encode("utf-8")
    ).digest()
    return MODEL_TRIGGERS[int.from_bytes(digest[:8], "big") % len(MODEL_TRIGGERS)]


def load_onboard_unigrams(
    path: Path,
    locale: str,
    group: int,
    config: TrainingConfig,
    *,
    license_declaration: str = "GPL-3+",
    license_evidence: str = "onboard-data/copyright: models/*",
    logical_path: str | None = None,
    source_bytes: bytes | None = None,
    minimum_word_length: int | None = None,
) -> tuple[tuple[LexiconWord, ...], LexiconSource]:
    """Read and deterministically filter the one-gram section of an Onboard LM."""

    effective_minimum_word_length = (
        config.minimum_word_length
        if minimum_word_length is None
        else minimum_word_length
    )
    if not 2 <= effective_minimum_word_length <= config.maximum_word_length:
        raise ValueError(
            "minimum_word_length override must be between two and the "
            "configured maximum_word_length"
        )

    if source_bytes is None:
        try:
            with path.open("rb") as stream:
                raw = stream.read(MAX_FROZEN_SOURCE_BYTES + 1)
        except OSError as error:
            raise RuntimeError(f"language source is unavailable: {path}") from error
        if len(raw) > MAX_FROZEN_SOURCE_BYTES:
            raise RuntimeError(
                "language source exceeds the maximum supported size of "
                f"{MAX_FROZEN_SOURCE_BYTES} bytes"
            )
    else:
        raw = source_bytes
    frequencies: dict[str, int] = {}
    section = False
    for line in raw.decode("utf-8", errors="replace").splitlines():
        if line == r"\1-grams:":
            section = True
            continue
        if section and line.startswith("\\"):
            break
        if not section or not line:
            continue
        count_text, separator, token = line.partition(" ")
        if not separator or token.startswith("<"):
            continue
        try:
            frequency = int(count_text)
        except ValueError:
            continue
        normalized = _normalise_lexical_word(token, group)
        if not (
            effective_minimum_word_length
            <= len(normalized)
            <= config.maximum_word_length
        ):
            continue
        signature = physical_signature(normalized, group)
        if not signature:
            continue
        frequencies[normalized] = frequencies.get(normalized, 0) + max(1, frequency)
    ranked = sorted(frequencies.items(), key=lambda item: (-item[1], item[0]))
    words = tuple(
        LexiconWord(word, group, frequency, physical_signature(word, group), stable_split(physical_signature(word, group)))
        for word, frequency in ranked
    )
    source = LexiconSource(
        locale=locale,
        group=group,
        path=logical_path or path.name,
        sha256=hashlib.sha256(raw).hexdigest(),
        bytes=len(raw),
        license_declaration=license_declaration,
        license_evidence=license_evidence,
    )
    return words, source


def prepare_lexicon(
    words: Iterable[LexiconWord],
    *,
    minimum_training_signature_length: int = 0,
) -> PreparedLexicon:
    """Deduplicate train words while retaining all safety collisions."""

    if minimum_training_signature_length < 0:
        raise ValueError("minimum_training_signature_length cannot be negative")

    grouped: dict[str, list[LexiconWord]] = defaultdict(list)
    for word in words:
        if not word.physical_signature:
            continue
        if word.split != stable_split(word.physical_signature):
            raise ValueError("lexicon word has an inconsistent split")
        grouped[word.physical_signature].append(word)

    by_split: dict[SplitName, list[LexiconWord]] = {
        name: [] for name in SPLIT_NAMES
    }
    collisions: list[LexiconCollision] = []
    for signature in sorted(grouped):
        candidates = grouped[signature]
        groups = {candidate.group for candidate in candidates}
        if len(groups) > 1:
            collisions.append(
                LexiconCollision(
                    signature,
                    tuple(
                        sorted(
                            candidates,
                            key=lambda item: (item.group, -item.frequency, item.word),
                        )
                    ),
                )
            )
            continue
        if len(signature) < minimum_training_signature_length:
            continue
        selected = min(candidates, key=lambda item: (-item.frequency, item.word))
        by_split[selected.split].append(selected)
    return PreparedLexicon(
        {
            name: tuple(
                sorted(
                    by_split[name],
                    key=lambda item: (item.physical_signature, item.group),
                )
            )
            for name in SPLIT_NAMES
        },
        tuple(collisions),
    )


@dataclass(frozen=True)
class TypoVariant:
    physical_signature: str
    kind: str


TYPO_VARIANT_KINDS: Final[frozenset[str]] = frozenset(
    {"deletion", "duplication", "transposition"}
)


def typo_variants(
    signature: str, maximum_augmentations: int
) -> tuple[TypoVariant, ...]:
    """Create symmetric physical-key typos without consulting either language."""

    variants = [TypoVariant(signature, "identity")]
    if maximum_augmentations <= 0 or len(signature) < 3:
        return tuple(variants)
    digest = hashlib.sha256(b"keyswitch:intent-v1:typo\0" + signature.encode()).digest()
    interior = 1 + digest[0] % max(1, len(signature) - 2)
    candidates = (
        TypoVariant(signature[:interior] + signature[interior + 1 :], "deletion"),
        TypoVariant(
            signature[:interior]
            + signature[interior]
            + signature[interior:],
            "duplication",
        ),
        TypoVariant(
            signature[:interior]
            + signature[interior + 1]
            + signature[interior]
            + signature[interior + 2 :],
            "transposition",
        ),
    )
    seen = {signature}
    for candidate in candidates:
        if len(variants) > maximum_augmentations or candidate.physical_signature in seen:
            continue
        seen.add(candidate.physical_signature)
        variants.append(candidate)
    return tuple(variants)


@dataclass(frozen=True)
class LexicalExample:
    original: str
    alternative: str
    source_group: int
    target_group: int
    trigger: CorrectionTrigger
    label: bool
    weight: float
    base_signature: str
    variant_kind: str
    source_known: bool
    target_known: bool
    context_delta: float = 0.0
    context_group: int | None = None
    frequency: int = 0
    protected: bool = False
    safety: bool = False


@dataclass(frozen=True)
class HardNegativeDevelopmentCorpus:
    """Verified external-development rows assigned to disjoint pre-seal roles."""

    by_split: dict[SplitName, tuple[LexicalExample, ...]]
    source_sha256: str
    source_bytes: int
    expanded_corpus_sha256: str
    physical_signatures_sha256: str
    signature_count: int
    words_by_group: dict[int, int]
    role_words_by_group: dict[SplitName, dict[int, int]]
    training_example_weight: float

    def provenance_payload(self) -> dict[str, object]:
        return {
            "schema_version": 2,
            "source": {
                "path": HARD_NEGATIVE_SOURCE_RELATIVE_PATH,
                "sha256": self.source_sha256,
                "bytes": self.source_bytes,
            },
            "role_namespace": HARD_NEGATIVE_ROLE_NAMESPACE,
            "expanded_corpus_sha256": self.expanded_corpus_sha256,
            "physical_signatures_sha256": (
                self.physical_signatures_sha256
            ),
            "signature_count": self.signature_count,
            "words_by_group": {
                str(group): count
                for group, count in sorted(self.words_by_group.items())
            },
            "role_words_by_group": {
                split: {
                    str(group): count
                    for group, count in sorted(counts.items())
                }
                for split, counts in self.role_words_by_group.items()
                if split in PRESEALED_SPLITS
            },
            "examples_by_role": {
                split: len(self.by_split[split])
                for split in PRESEALED_SPLITS
            },
            "training_example_weight": self.training_example_weight,
        }


def external_corpus_fingerprint(
    examples: Sequence[LexicalExample],
) -> str:
    """Match the evaluator's path-independent external-corpus hash."""

    digest = hashlib.sha256(b"keyswitch:intent-v1:external-corpus\0")
    canonical_rows = sorted(
        json.dumps(
            asdict(example),
            ensure_ascii=False,
            sort_keys=True,
            separators=(",", ":"),
        )
        for example in examples
    )
    for row in canonical_rows:
        digest.update(row.encode("utf-8"))
        digest.update(b"\n")
    return digest.hexdigest()


def physical_signature_set_fingerprint(
    signatures: Collection[str],
) -> str:
    """Match the evaluator's separator-safe physical-signature hash."""

    normalized = frozenset(signatures)
    if "" in normalized:
        raise ValueError("physical signatures must not contain empty values")
    digest = hashlib.sha256(b"keyswitch:intent-v2:sealed-signatures\0")
    for signature in sorted(normalized):
        digest.update(
            json.dumps(
                signature,
                ensure_ascii=False,
                separators=(",", ":"),
            ).encode("utf-8")
        )
        digest.update(b"\n")
    return digest.hexdigest()


def _hard_negative_source_provenance(
    config: TrainingConfig,
) -> dict[str, object]:
    return {
        "0": {
            "locale": "en_US",
            "dictionary_sha256": config.external_evaluation.english.dictionary_sha256,
            "dictionary_bytes": config.external_evaluation.english.dictionary_bytes,
            "affix_sha256": config.external_evaluation.english.affix_sha256,
            "affix_bytes": config.external_evaluation.english.affix_bytes,
        },
        "1": {
            "locale": "ru_RU",
            "dictionary_sha256": config.external_evaluation.russian.dictionary_sha256,
            "dictionary_bytes": config.external_evaluation.russian.dictionary_bytes,
            "affix_sha256": config.external_evaluation.russian.affix_sha256,
            "affix_bytes": config.external_evaluation.russian.affix_bytes,
        },
    }


def _bounded_corpus_string(value: object, label: str) -> str:
    if (
        not isinstance(value, str)
        or not value
        or len(value) > 256
        or any(ord(character) < 32 for character in value)
    ):
        raise ValueError(f"{label} must be non-empty bounded text")
    return value


def _hard_negative_physical_text(
    text: str,
    group: int,
    pair: LayoutPair,
) -> str:
    normalized = text.casefold().strip()
    if group == 0:
        return normalized
    if group == 1:
        return pair.translate(normalized, "ru", "us")
    raise ValueError("hard-negative language group must be 0 or 1")


def _decode_hard_negative_development_corpus(
    raw: bytes,
    config: TrainingConfig,
) -> HardNegativeDevelopmentCorpus:
    """Strictly decode, expand, repartition and fingerprint the frozen source."""

    def unique_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
        result: dict[str, object] = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(
                    f"hard-negative corpus contains duplicate key {key!r}"
                )
            result[key] = value
        return result

    def forbidden_constant(value: str) -> object:
        raise ValueError(
            "hard-negative corpus contains forbidden JSON "
            f"constant {value}"
        )

    try:
        decoded: object = json.loads(
            raw.decode("utf-8"),
            object_pairs_hook=unique_object,
            parse_constant=forbidden_constant,
        )
    except (
        UnicodeDecodeError,
        json.JSONDecodeError,
        RecursionError,
    ) as error:
        raise ValueError("hard-negative corpus must be strict UTF-8 JSON") from error
    root = _mapping(decoded, "hard-negative corpus")
    _require_keys(
        root,
        {
            "schema_version",
            "policy",
            "role_namespace",
            "rank_namespace",
            "choice_namespace",
            "expanded_corpus_sha256",
            "physical_signatures_sha256",
            "signature_count",
            "words_by_group",
            "source_provenance",
            "rows",
        },
        "hard-negative corpus",
    )
    if _integer(root, "schema_version") != 1:
        raise ValueError("unsupported frozen hard-negative corpus schema")
    if _string(root, "policy") != (
        "keyswitch-intent-v14-frozen-unknown-typo-development"
    ):
        raise ValueError("hard-negative corpus policy must match v14")
    if _string(root, "role_namespace") != HARD_NEGATIVE_ROLE_NAMESPACE:
        raise ValueError("hard-negative corpus role namespace must match v14")
    if _string(root, "rank_namespace") != (
        UNKNOWN_TYPO_DEVELOPMENT_RANK_NAMESPACE
    ) or _string(root, "choice_namespace") != (
        UNKNOWN_TYPO_DEVELOPMENT_CHOICE_NAMESPACE
    ):
        raise ValueError("hard-negative corpus generation namespace mismatch")
    expanded_digest = _string(root, "expanded_corpus_sha256")
    if expanded_digest != (
        config.external_evaluation.unknown_typo_development_corpus_sha256
    ):
        raise ValueError("hard-negative corpus expanded SHA-256 is not pinned")
    physical_digest = _string(root, "physical_signatures_sha256")
    for label, digest in (
        ("expanded_corpus_sha256", expanded_digest),
        ("physical_signatures_sha256", physical_digest),
    ):
        _exact_sha256(digest, label)
    signature_count = _integer(root, "signature_count")
    expected_signatures = (
        config.external_evaluation.minimum_words_per_group * 2
    )
    if signature_count != expected_signatures:
        raise ValueError("hard-negative corpus signature count is incomplete")
    words = _mapping(root.get("words_by_group"), "words_by_group")
    _require_keys(words, {"0", "1"}, "words_by_group")
    words_by_group = {
        group: _integer(words, str(group)) for group in (0, 1)
    }
    if any(
        count != config.external_evaluation.minimum_words_per_group
        for count in words_by_group.values()
    ):
        raise ValueError("hard-negative corpus group counts are incomplete")
    if _mapping(root.get("source_provenance"), "source_provenance") != (
        _hard_negative_source_provenance(config)
    ):
        raise ValueError("hard-negative dictionary provenance is not pinned")
    raw_rows = root.get("rows")
    if not isinstance(raw_rows, list):
        raise ValueError("hard-negative corpus rows must be an array")
    rows = cast(list[object], raw_rows)
    if len(rows) != signature_count:
        raise ValueError("hard-negative compact row count is inconsistent")

    records: dict[str, tuple[int, str, str, str]] = {}
    records_by_group: dict[int, list[str]] = {0: [], 1: []}
    pair = LayoutPair()
    allowed_variants = {
        "hunspell-unknown-deletion",
        "hunspell-unknown-duplication",
        "hunspell-unknown-transposition",
    }
    for index, value in enumerate(rows):
        item = _mapping(value, f"rows[{index}]")
        _require_keys(
            item,
            {
                "physical_signature",
                "correct_group",
                "correct_typo",
                "wrong_typo",
                "variant_kind",
            },
            f"rows[{index}]",
        )
        signature = _bounded_corpus_string(
            item.get("physical_signature"),
            f"rows[{index}].physical_signature",
        )
        correct_group = _integer(item, "correct_group")
        if correct_group not in (0, 1):
            raise ValueError("hard-negative correct_group must be 0 or 1")
        correct_typo = _bounded_corpus_string(
            item.get("correct_typo"), f"rows[{index}].correct_typo"
        )
        wrong_typo = _bounded_corpus_string(
            item.get("wrong_typo"), f"rows[{index}].wrong_typo"
        )
        variant_kind = _bounded_corpus_string(
            item.get("variant_kind"), f"rows[{index}].variant_kind"
        )
        if variant_kind not in allowed_variants:
            raise ValueError("hard-negative variant kind is unsupported")
        if not (
            config.minimum_word_length
            <= len(signature)
            <= config.maximum_word_length
        ):
            raise ValueError("hard-negative physical signature length is invalid")
        if (
            _hard_negative_physical_text(
                correct_typo, correct_group, pair
            )
            != signature
            or _hard_negative_physical_text(
                wrong_typo, 1 - correct_group, pair
            )
            != signature
        ):
            raise ValueError("hard-negative rendered physical signature changed")
        if (
            LanguageDetector.is_protected_token(correct_typo)
            or LanguageDetector.is_protected_token(wrong_typo)
        ):
            raise ValueError("hard-negative corpus contains a protected token")
        if signature in records:
            raise ValueError("hard-negative corpus contains duplicate signatures")
        records[signature] = (
            correct_group,
            correct_typo,
            wrong_typo,
            variant_kind,
        )
        records_by_group[correct_group].append(signature)
    if {
        group: len(signatures)
        for group, signatures in records_by_group.items()
    } != words_by_group:
        raise ValueError("hard-negative compact rows disagree with group counts")
    if physical_signature_set_fingerprint(records) != physical_digest:
        raise ValueError("hard-negative physical-signature SHA-256 mismatch")

    role_counts = config.hard_negative_development.role_counts()
    role_by_signature: dict[str, SplitName] = {}
    role_words_by_group: dict[SplitName, dict[int, int]] = {
        split: {0: 0, 1: 0} for split in SPLIT_NAMES
    }
    namespace = config.hard_negative_development.role_namespace.encode(
        "ascii"
    ) + b"\0"
    for group in (0, 1):
        ordered = sorted(
            records_by_group[group],
            key=lambda signature: hashlib.sha256(
                namespace + signature.encode("utf-8")
            ).digest(),
        )
        cursor = 0
        for split in PRESEALED_SPLITS:
            count = role_counts[split]
            selected = ordered[cursor : cursor + count]
            if len(selected) != count:
                raise ValueError("hard-negative role partition is incomplete")
            for signature in selected:
                role_by_signature[signature] = split
            role_words_by_group[split][group] = count
            cursor += count
        if cursor != len(ordered):
            raise ValueError("hard-negative role partition left unused rows")

    expanded: list[LexicalExample] = []
    by_split: dict[SplitName, list[LexicalExample]] = {
        split: [] for split in SPLIT_NAMES
    }
    for signature in sorted(records):
        correct_group, correct_typo, wrong_typo, variant_kind = records[
            signature
        ]
        split = role_by_signature[signature]
        base_signature = "hunspell-unknown:" + signature
        for trigger in MODEL_TRIGGERS:
            pair_rows = (
                LexicalExample(
                    original=correct_typo,
                    alternative=wrong_typo,
                    source_group=correct_group,
                    target_group=1 - correct_group,
                    trigger=trigger,
                    label=False,
                    weight=1.0,
                    base_signature=base_signature,
                    variant_kind=variant_kind,
                    source_known=False,
                    target_known=False,
                ),
                LexicalExample(
                    original=wrong_typo,
                    alternative=correct_typo,
                    source_group=1 - correct_group,
                    target_group=correct_group,
                    trigger=trigger,
                    label=True,
                    weight=1.0,
                    base_signature=base_signature,
                    variant_kind=variant_kind,
                    source_known=False,
                    target_known=False,
                ),
            )
            expanded.extend(pair_rows)
            if split == "train":
                by_split[split].extend(
                    replace(
                        row,
                        weight=(
                            config.hard_negative_development
                            .training_example_weight
                        ),
                    )
                    for row in pair_rows
                )
            else:
                by_split[split].extend(pair_rows)
    if external_corpus_fingerprint(expanded) != expanded_digest:
        raise ValueError("hard-negative expanded corpus SHA-256 mismatch")
    return HardNegativeDevelopmentCorpus(
        by_split={
            split: tuple(
                sorted(
                    split_rows,
                    key=lambda item: (
                        item.base_signature,
                        item.variant_kind,
                        item.trigger,
                        item.label,
                        item.source_group,
                    ),
                )
            )
            for split, split_rows in by_split.items()
        },
        source_sha256=hashlib.sha256(raw).hexdigest(),
        source_bytes=len(raw),
        expanded_corpus_sha256=expanded_digest,
        physical_signatures_sha256=physical_digest,
        signature_count=signature_count,
        words_by_group=words_by_group,
        role_words_by_group=role_words_by_group,
        training_example_weight=(
            config.hard_negative_development.training_example_weight
        ),
    )


def load_hard_negative_development_corpus(
    path: Path,
    config: TrainingConfig,
) -> HardNegativeDevelopmentCorpus:
    raw = read_verified_frozen_file(
        path,
        config.hard_negative_development.source,
        label="Frozen hard-negative development corpus",
    )
    corpus = _decode_hard_negative_development_corpus(raw, config)
    if (
        corpus.source_sha256
        != config.hard_negative_development.source.sha256
        or corpus.source_bytes
        != config.hard_negative_development.source.bytes
    ):
        raise RuntimeError("hard-negative source changed after verification")
    return corpus


@dataclass(frozen=True)
class ContextStressProfile:
    """One fixed, label-independent observable context perturbation."""

    name: str
    delta: float
    group_selector: ContextGroupSelector


CONTEXT_STRESS_PROFILES: Final[tuple[ContextStressProfile, ...]] = (
    ContextStressProfile("source_minimum", -6.0, "source"),
    ContextStressProfile("source_outer_negative", -1.25, "source"),
    ContextStressProfile("source_inner_negative", -0.75, "source"),
    ContextStressProfile("source_near_zero_negative", -0.125, "source"),
    ContextStressProfile("source_zero", 0.0, "source"),
    ContextStressProfile("source_near_zero_positive", 0.125, "source"),
    ContextStressProfile("source_inner_positive", 0.75, "source"),
    ContextStressProfile("source_outer_positive", 1.25, "source"),
    ContextStressProfile("source_maximum", 6.0, "source"),
    ContextStressProfile("target_minimum", -6.0, "target"),
    ContextStressProfile("target_outer_negative", -1.25, "target"),
    ContextStressProfile("target_inner_negative", -0.75, "target"),
    ContextStressProfile("target_near_zero_negative", -0.125, "target"),
    ContextStressProfile("target_zero", 0.0, "target"),
    ContextStressProfile("target_near_zero_positive", 0.125, "target"),
    ContextStressProfile("target_inner_positive", 0.75, "target"),
    ContextStressProfile("target_outer_positive", 1.25, "target"),
    ContextStressProfile("target_maximum", 6.0, "target"),
)


@dataclass(frozen=True)
class QuarantinedVariantOccurrence:
    """One generated variant removed because its physical keys cross splits."""

    physical_signature: str
    base_signature: str
    split: SplitName
    group: int
    variant_kind: str
    reason: QuarantineReason


@dataclass(frozen=True)
class _VariantOccurrenceCandidate:
    physical_signature: str
    base_signature: str
    split: SplitName
    group: int
    variant_kind: str


@dataclass(frozen=True)
class VariantQuarantine:
    """Deterministic evidence for every augmentation occurrence we removed."""

    occurrences: tuple[QuarantinedVariantOccurrence, ...]
    sha256: str

    @property
    def occurrence_count(self) -> int:
        return len(self.occurrences)

    @property
    def physical_signature_count(self) -> int:
        return len({item.physical_signature for item in self.occurrences})


class WordScorer(Protocol):
    """Score one explicitly supplied word without dataset metadata.

    Group selection happens outside the scorer.  The narrow API deliberately
    has no access to labels, split annotations, synthetic known flags or
    example frequency, making label-derived dense features impossible by
    construction.
    """

    def score(self, word: str) -> WordScore: ...


_IGNORED_CLASSIFIER_WORD_SCORE: Final[WordScore] = WordScore(
    0.0,
    False,
    0,
    0.0,
    exact=False,
    spell_known=False,
    ngram_score=0.0,
    invalid_ratio=1.0,
    raw_ngram_score=0.0,
)


class NgramOnlyWordScorer:
    """Expose only distributional character evidence from a language model.

    Exact dictionary membership of a train word is a split-membership shortcut:
    it is present for train identities and absent for every sealed identity.
    The wrapper deliberately masks lexical lookup and recomputes the score from
    n-gram naturalness and invalid-gram ratio only.  Runtime evaluation still
    uses the real unmasked language models.
    """

    def __init__(self, model: LanguageModel) -> None:
        self._model = model

    @lru_cache(maxsize=65_536)
    def score(self, word: str) -> WordScore:
        normalized = self._model.normalize(word)
        if not normalized:
            return WordScore(
                -30.0,
                False,
                0,
                0.0,
                exact=False,
                spell_known=False,
                ngram_score=-15.0,
                invalid_ratio=1.0,
                raw_ngram_score=-15.0,
            )
        structural = self._model.score(word)
        naturalness = max(
            -15.0,
            min(4.0, self._model.ngram_score(normalized)),
        )
        value = 1.15 * naturalness - 0.75 * structural.invalid_ratio
        return WordScore(
            value,
            False,
            0,
            structural.gram_ratio,
            exact=False,
            spell_known=False,
            ngram_score=naturalness,
            invalid_ratio=structural.invalid_ratio,
            raw_ngram_score=naturalness,
        )


@dataclass(frozen=True)
class TrainOnlyLanguageScorers:
    """Frozen EN/RU lexical+n-gram scorer built exclusively from train rows."""

    scorers: Mapping[int, WordScorer] = field(repr=False)
    word_counts_by_group: tuple[int, int]
    excluded_quarantined_identities: int
    source_sha256: str

    @classmethod
    def from_training_partition(
        cls,
        prepared: PreparedLexicon,
        quarantine: VariantQuarantine,
    ) -> TrainOnlyLanguageScorers:
        quarantined = {
            occurrence.physical_signature for occurrence in quarantine.occurrences
        }
        frequencies: dict[int, dict[str, int]] = {0: {}, 1: {}}
        excluded = 0
        digest = hashlib.sha256(
            b"keyswitch:intent-v1:train-only-language-scorer\0"
        )
        digest.update(
            json.dumps(
                {
                    "algorithm_version": 2,
                    "ngram_orders": list(LanguageModel.NGRAM_ORDERS),
                    "score_mode": "character-ngram-only",
                    "spellcheck_enabled": False,
                },
                sort_keys=True,
                separators=(",", ":"),
            ).encode("ascii")
        )
        digest.update(b"\n")
        for record in sorted(
            prepared.words_by_split.get("train", ()),
            key=lambda item: (
                item.group,
                item.physical_signature,
                item.word,
                item.frequency,
            ),
        ):
            if record.group not in frequencies:
                raise ValueError("training language scorer supports only groups 0 and 1")
            if record.split != "train":
                raise ValueError("training language scorer received a non-train row")
            if record.frequency < 1:
                raise ValueError("training language scorer frequencies must be positive")
            if physical_signature(record.word, record.group) != record.physical_signature:
                raise ValueError(
                    "training language scorer received an inconsistent physical signature"
                )
            if record.physical_signature in quarantined:
                excluded += 1
                continue
            if record.word in frequencies[record.group]:
                raise ValueError("training language scorer words must be unique")
            frequencies[record.group][record.word] = record.frequency
            digest.update(
                json.dumps(
                    {
                        "frequency": record.frequency,
                        "group": record.group,
                        "physical_signature": record.physical_signature,
                        "word": record.word,
                    },
                    ensure_ascii=False,
                    sort_keys=True,
                    separators=(",", ":"),
                ).encode("utf-8")
            )
            digest.update(b"\n")
        if not frequencies[0] or not frequencies[1]:
            raise ValueError(
                "train-only language scorer requires non-empty EN and RU partitions"
            )
        scorers: dict[int, WordScorer] = {
            group: NgramOnlyWordScorer(
                LanguageModel(
                    locale,
                    dict(sorted(frequencies[group].items())),
                    "sealed train partition",
                    enable_spellcheck=False,
                )
            )
            for group, locale in ((0, "en_US"), (1, "ru_RU"))
        }
        return cls(
            scorers,
            (len(frequencies[0]), len(frequencies[1])),
            excluded,
            digest.hexdigest(),
        )

    def provenance_payload(self) -> dict[str, object]:
        return {
            "kind": "train-only-character-ngram",
            "algorithm_version": 2,
            "ngram_orders": list(LanguageModel.NGRAM_ORDERS),
            "score_mode": "character-ngram-only",
            "spellcheck_enabled": False,
            "word_counts_by_group": {
                "0": self.word_counts_by_group[0],
                "1": self.word_counts_by_group[1],
            },
            "excluded_quarantined_identities": (
                self.excluded_quarantined_identities
            ),
            "source_sha256": self.source_sha256,
        }


def variant_quarantine_fingerprint(
    occurrences: Sequence[QuarantinedVariantOccurrence],
) -> str:
    """Hash a canonical, order-independent representation of a quarantine."""

    digest = hashlib.sha256(b"keyswitch:intent-v1:variant-quarantine\0")
    for occurrence in sorted(
        occurrences,
        key=lambda item: (
            item.physical_signature,
            item.split,
            item.base_signature,
            item.group,
            item.variant_kind,
            item.reason,
        ),
    ):
        digest.update(
            json.dumps(
                asdict(occurrence),
                ensure_ascii=False,
                sort_keys=True,
                separators=(",", ":"),
            ).encode("utf-8")
        )
        digest.update(b"\n")
    return digest.hexdigest()


def _empty_variant_quarantine() -> VariantQuarantine:
    return VariantQuarantine((), variant_quarantine_fingerprint(()))


@dataclass(frozen=True)
class DatasetBundle:
    by_split: dict[SplitName, tuple[LexicalExample, ...]]
    safety: tuple[LexicalExample, ...]
    variant_quarantine: VariantQuarantine = field(
        default_factory=_empty_variant_quarantine
    )
    sealed_variant_quarantine: VariantQuarantine = field(
        default_factory=_empty_variant_quarantine
    )
    sealed_test_exclusion_signatures: tuple[str, ...] = ()


def merge_hard_negative_development(
    dataset: DatasetBundle,
    hard_negatives: HardNegativeDevelopmentCorpus,
) -> DatasetBundle:
    """Add each external-development signature to exactly one pre-seal role."""

    if dataset.by_split["test"] or hard_negatives.by_split["test"]:
        raise ValueError("hard-negative development merge cannot contain test rows")
    if (
        dataset.sealed_variant_quarantine.occurrences
        or dataset.sealed_test_exclusion_signatures
    ):
        raise ValueError("hard-negative development merge must happen pre-seal")
    merged = DatasetBundle(
        {
            split: tuple(
                sorted(
                    (
                        *dataset.by_split[split],
                        *hard_negatives.by_split[split],
                    ),
                    key=lambda item: (
                        item.base_signature,
                        item.variant_kind,
                        item.trigger,
                        item.label,
                        item.source_group,
                    ),
                )
            )
            for split in SPLIT_NAMES
        },
        dataset.safety,
        dataset.variant_quarantine,
        dataset.sealed_variant_quarantine,
        dataset.sealed_test_exclusion_signatures,
    )
    for split in PRESEALED_SPLITS:
        expected = len(dataset.by_split[split]) + len(
            hard_negatives.by_split[split]
        )
        if len(merged.by_split[split]) != expected:
            raise AssertionError("hard-negative development rows were lost")
    return merged


@dataclass(frozen=True)
class DatasetSignatureAudit:
    """Result of deriving physical keys from every generated lexical row."""

    audited_rows: int
    skipped_nonlexical_rows: int
    unique_physical_signatures: int
    cross_split_signatures: tuple[tuple[str, tuple[SplitName, ...]], ...]
    cross_language_signatures: tuple[tuple[str, tuple[int, ...]], ...]
    safety_overlap_signatures: tuple[str, ...]
    safety_base_signature_overlaps: tuple[str, ...]
    malformed_rows: tuple[str, ...]
    quarantined_signatures_present: tuple[str, ...]


TRAINING_HARD_NEGATIVES: Final[tuple[str, ...]] = (
    "abc123",
    "api_v2",
    "camelCase",
    "docker-compose",
    "foo@example.com",
    "https://example.com",
    "KEYSWITCH_DEBUG",
    "localhost:8080",
    "python3.12",
    "user_name",
)
SAFETY_HARD_NEGATIVES: Final[tuple[str, ...]] = (
    "--force-with-lease",
    "/usr/bin/python3",
    "127.0.0.1",
    "C:\\Windows\\System32",
    "feature/intent-model-v1",
    "sha256:deadbeef",
)


def _render_physical(signature: str, group: int, pair: LayoutPair) -> str:
    if group == 0:
        return signature
    if group == 1:
        return pair.translate(signature, "us", "ru")
    raise ValueError("only EN/RU groups 0 and 1 are supported")


def _canonical_physical_text(
    text: str, group: int, pair: LayoutPair
) -> str:
    """Map any rendered token, including protected syntax, to US key positions."""

    normalized = text.casefold().strip()
    if group == 0:
        return normalized
    if group == 1:
        return pair.translate(normalized, "ru", "us")
    raise ValueError("only EN/RU groups 0 and 1 are supported")


def _example_weight(frequency: int) -> float:
    return 1.0 + min(2.0, math.log1p(max(1, frequency)) / 8.0)


def context_stress_examples(
    examples: Iterable[LexicalExample],
    profile: ContextStressProfile,
) -> tuple[LexicalExample, ...]:
    """Apply a fixed profile without inspecting labels or hidden metadata."""

    stressed: list[LexicalExample] = []
    for example in examples:
        if profile.group_selector == "source":
            context_group = example.source_group
        elif profile.group_selector == "target":
            context_group = example.target_group
        else:
            raise ValueError("unknown context stress group selector")
        stressed.append(
            replace(
                example,
                context_delta=profile.delta,
                context_group=context_group,
            )
        )
    return tuple(stressed)


def _hard_negative_examples(
    token: str, *, safety: bool, pair: LayoutPair
) -> tuple[LexicalExample, ...]:
    alternative = pair.translate(token, "us", "ru")
    signature = f"hard:{token.casefold()}"
    return tuple(
        LexicalExample(
            original=token,
            alternative=alternative,
            source_group=0,
            target_group=1,
            trigger=trigger,
            label=False,
            weight=2.0,
            base_signature=signature,
            variant_kind="protected",
            source_known=False,
            target_known=False,
            protected=True,
            safety=safety,
        )
        for trigger in MODEL_TRIGGERS
    )


@dataclass(frozen=True)
class GuardedSafetyAudit:
    """Evidence that every safety row is stopped before model inference."""

    samples: int
    protected_samples: int
    lexical_collision_samples: int
    triggers: tuple[CorrectionTrigger, ...]
    protected_triggers: tuple[CorrectionTrigger, ...]
    lexical_collision_triggers: tuple[CorrectionTrigger, ...]
    failures: tuple[str, ...]

    def passes(self, maximum_failures: int) -> bool:
        expected = tuple(MODEL_TRIGGERS)
        return (
            maximum_failures >= 0
            and self.samples > 0
            and self.protected_samples > 0
            and self.lexical_collision_samples > 0
            and self.triggers == expected
            and self.protected_triggers == expected
            and self.lexical_collision_triggers == expected
            and len(self.failures) <= maximum_failures
        )


def audit_guarded_safety_corpus(
    examples: Sequence[LexicalExample],
) -> GuardedSafetyAudit:
    """Verify safety examples against the actual production pre-model guards."""

    observed: set[CorrectionTrigger] = set()
    protected_triggers: set[CorrectionTrigger] = set()
    collision_triggers: set[CorrectionTrigger] = set()
    protected_samples = 0
    collision_samples = 0
    failures: list[str] = []
    for index, example in enumerate(examples):
        observed.add(example.trigger)
        prefix = f"row {index} ({example.trigger}, {example.variant_kind})"
        if example.label or not example.safety:
            failures.append(f"{prefix}: row is not a sealed safety negative")
            continue
        if example.protected:
            protected_samples += 1
            protected_triggers.add(example.trigger)
            if example.variant_kind != "protected":
                failures.append(f"{prefix}: protected row has the wrong kind")
            if example.source_known or example.target_known:
                failures.append(f"{prefix}: protected row claims lexical knowledge")
            if not LanguageDetector.is_protected_token(example.original):
                failures.append(
                    f"{prefix}: production protected-token guard does not match"
                )
            continue
        if example.variant_kind == "lexical_collision":
            collision_samples += 1
            collision_triggers.add(example.trigger)
            if not (example.source_known and example.target_known):
                failures.append(
                    f"{prefix}: collision must be known in both layouts"
                )
            continue
        failures.append(f"{prefix}: no production pre-model guard applies")
    return GuardedSafetyAudit(
        samples=len(examples),
        protected_samples=protected_samples,
        lexical_collision_samples=collision_samples,
        triggers=tuple(trigger for trigger in MODEL_TRIGGERS if trigger in observed),
        protected_triggers=tuple(
            trigger for trigger in MODEL_TRIGGERS if trigger in protected_triggers
        ),
        lexical_collision_triggers=tuple(
            trigger for trigger in MODEL_TRIGGERS if trigger in collision_triggers
        ),
        failures=tuple(failures),
    )


def build_variant_quarantine(
    prepared: PreparedLexicon,
    config: TrainingConfig,
    *,
    included_splits: Collection[SplitName],
) -> VariantQuarantine:
    """Find generated signatures owned by multiple splits or intended groups.

    The pre-pass runs before any positive/negative rows are emitted.  When a
    typo of one word is the identity (or another typo) of a word in a different
    split, every occurrence of that exact physical sequence is removed.  The
    same applies when one sequence is claimed by both intended languages, which
    would otherwise create identical directional rows with opposite labels.
    This preserves label symmetry and prevents augmented leakage or label
    contradiction.
    """

    selected_splits = frozenset(included_splits)
    if not selected_splits or not selected_splits <= frozenset(SPLIT_NAMES):
        raise ValueError("included_splits must be a non-empty split subset")
    candidates: list[_VariantOccurrenceCandidate] = []
    split_owners: dict[str, set[SplitName]] = defaultdict(set)
    group_owners: dict[str, set[int]] = defaultdict(set)
    layout_pair = LayoutPair()
    for collision in prepared.collisions:
        # A collision is emitted only into the held-out safety corpus as an
        # ambiguous negative in both directions.  Any train/evaluation typo
        # that renders to the same physical keys would otherwise give the
        # same observation contradictory labels across corpora.
        if stable_split(collision.physical_signature) in selected_splits:
            group_owners[collision.physical_signature].update((0, 1))
    for token in TRAINING_HARD_NEGATIVES:
        signature = _canonical_physical_text(token, 0, layout_pair)
        split = stable_split(f"hard:{token.casefold()}")
        if split in selected_splits:
            split_owners[signature].add(split)
            group_owners[signature].add(0)
    for token in SAFETY_HARD_NEGATIVES:
        signature = _canonical_physical_text(token, 0, layout_pair)
        group_owners[signature].update((0, 1))
    for split in SPLIT_NAMES:
        if split not in selected_splits:
            continue
        for record in prepared.words_by_split[split]:
            for variant in typo_variants(
                record.physical_signature, config.typo_augmentations
            ):
                if len(variant.physical_signature) < config.minimum_word_length:
                    continue
                occurrence = _VariantOccurrenceCandidate(
                    variant.physical_signature,
                    record.physical_signature,
                    split,
                    record.group,
                    variant.kind,
                )
                candidates.append(occurrence)
                split_owners[variant.physical_signature].add(split)
                group_owners[variant.physical_signature].add(record.group)
    reasons: dict[str, QuarantineReason] = {}
    for signature in sorted(set(split_owners) | set(group_owners)):
        reason = _variant_quarantine_reason(
            split_owners[signature], group_owners[signature]
        )
        if reason is not None:
            reasons[signature] = reason
    quarantined = tuple(
        sorted(
            (
                QuarantinedVariantOccurrence(
                    occurrence.physical_signature,
                    occurrence.base_signature,
                    occurrence.split,
                    occurrence.group,
                    occurrence.variant_kind,
                    reasons[occurrence.physical_signature],
                )
                for occurrence in candidates
                if occurrence.physical_signature in reasons
            ),
            key=lambda item: (
                item.physical_signature,
                item.split,
                item.base_signature,
                item.group,
                item.variant_kind,
                item.reason,
            ),
        )
    )
    return VariantQuarantine(
        quarantined,
        variant_quarantine_fingerprint(quarantined),
    )


def _variant_quarantine_reason(
    splits: set[SplitName], groups: set[int]
) -> QuarantineReason | None:
    crosses_split = len(splits) > 1
    crosses_language = len(groups) > 1
    if crosses_split and crosses_language:
        return "cross_split+cross_language"
    if crosses_split:
        return "cross_split"
    if crosses_language:
        return "cross_language"
    return None


def build_dataset(
    prepared: PreparedLexicon,
    config: TrainingConfig,
    pair: LayoutPair | None = None,
    *,
    included_splits: Collection[SplitName],
) -> DatasetBundle:
    """Build balanced positive/negative examples for an explicit phase."""

    selected_splits = frozenset(included_splits)
    if not selected_splits or not selected_splits <= frozenset(SPLIT_NAMES):
        raise ValueError("included_splits must be a non-empty split subset")
    layout_pair = pair or LayoutPair()
    quarantine = build_variant_quarantine(
        prepared, config, included_splits=selected_splits
    )
    quarantined_signatures = {
        occurrence.physical_signature for occurrence in quarantine.occurrences
    }
    by_split: dict[SplitName, list[LexicalExample]] = {
        name: [] for name in SPLIT_NAMES
    }
    for split in SPLIT_NAMES:
        if split not in selected_splits:
            continue
        for record in prepared.words_by_split[split]:
            for variant in typo_variants(
                record.physical_signature, config.typo_augmentations
            ):
                if len(variant.physical_signature) < config.minimum_word_length:
                    continue
                if variant.physical_signature in quarantined_signatures:
                    continue
                correct = _render_physical(
                    variant.physical_signature, record.group, layout_pair
                )
                wrong_group = 1 - record.group
                wrong = _render_physical(
                    variant.physical_signature, wrong_group, layout_pair
                )
                known = variant.kind == "identity"
                weight = _example_weight(record.frequency)
                active_triggers = (
                    (deterministic_training_trigger(record.physical_signature),)
                    if split in ("train", "development")
                    else MODEL_TRIGGERS
                )
                for trigger in active_triggers:
                    by_split[split].append(
                        LexicalExample(
                            original=correct,
                            alternative=wrong,
                            source_group=record.group,
                            target_group=wrong_group,
                            trigger=trigger,
                            label=False,
                            weight=weight,
                            base_signature=record.physical_signature,
                            variant_kind=variant.kind,
                            source_known=known,
                            target_known=False,
                            frequency=record.frequency,
                        )
                    )
                    by_split[split].append(
                        LexicalExample(
                            original=wrong,
                            alternative=correct,
                            source_group=wrong_group,
                            target_group=record.group,
                            trigger=trigger,
                            label=True,
                            weight=weight,
                            base_signature=record.physical_signature,
                            variant_kind=variant.kind,
                            source_known=False,
                            target_known=known,
                            frequency=record.frequency,
                        )
                    )

    for token in TRAINING_HARD_NEGATIVES:
        split = stable_split(f"hard:{token.casefold()}")
        if split not in selected_splits:
            continue
        by_split[split].extend(
            _hard_negative_examples(token, safety=False, pair=layout_pair)
        )

    safety_examples: list[LexicalExample] = []
    for collision in prepared.collisions:
        if stable_split(collision.physical_signature) not in selected_splits:
            continue
        for source_group in (0, 1):
            target_group = 1 - source_group
            for trigger in MODEL_TRIGGERS:
                safety_examples.append(
                    LexicalExample(
                        original=_render_physical(
                            collision.physical_signature, source_group, layout_pair
                        ),
                        alternative=_render_physical(
                            collision.physical_signature, target_group, layout_pair
                        ),
                        source_group=source_group,
                        target_group=target_group,
                        trigger=trigger,
                        label=False,
                        weight=3.0,
                        base_signature=collision.physical_signature,
                        variant_kind="lexical_collision",
                        source_known=True,
                        target_known=True,
                        safety=True,
                    )
                )
    for token in SAFETY_HARD_NEGATIVES:
        safety_examples.extend(
            _hard_negative_examples(token, safety=True, pair=layout_pair)
        )
    return DatasetBundle(
        {
            name: tuple(
                sorted(
                    by_split[name],
                    key=lambda item: (
                        item.base_signature,
                        item.variant_kind,
                        item.trigger,
                        item.label,
                        item.source_group,
                    ),
                )
            )
            for name in SPLIT_NAMES
        },
        tuple(
            sorted(
                safety_examples,
                key=lambda item: (
                    item.base_signature,
                    item.trigger,
                    item.source_group,
                ),
            )
        ),
        quarantine,
    )


def row_physical_signature(example: LexicalExample) -> tuple[str, str | None]:
    pair = LayoutPair()
    renderer = (
        _hard_negative_physical_text
        if example.base_signature.startswith("hunspell-unknown:")
        and example.variant_kind.startswith("hunspell-unknown-")
        else _canonical_physical_text
    )
    candidates = tuple(
        signature
        for signature in (
            renderer(
                example.original, example.source_group, pair
            ),
            renderer(
                example.alternative, example.target_group, pair
            ),
        )
        if signature
    )
    if not candidates:
        return "", "neither rendered side has a valid physical signature"
    unique = tuple(sorted(set(candidates)))
    if len(unique) != 1:
        return "", f"rendered sides disagree: {unique!r}"
    return unique[0], None


def merge_sealed_test_dataset(
    presealed: DatasetBundle,
    sealed: DatasetBundle,
) -> DatasetBundle:
    """Merge a post-claim test partition without changing candidate rows."""

    if presealed.by_split["test"]:
        raise ValueError("presealed dataset unexpectedly contains test rows")
    if any(sealed.by_split[split] for split in PRESEALED_SPLITS):
        raise ValueError("sealed dataset unexpectedly contains candidate rows")
    if (
        presealed.sealed_variant_quarantine.occurrences
        or presealed.sealed_test_exclusion_signatures
        or sealed.sealed_variant_quarantine.occurrences
        or sealed.sealed_test_exclusion_signatures
    ):
        raise ValueError("phase-local datasets contain unexpected sealed metadata")

    # The held-out test may not reuse any physical signature that was visible
    # before the one-candidate seal.  Quarantine records are part of that
    # pre-seal evidence even though their lexical rows were deliberately not
    # emitted into the candidate partitions.
    presealed_signatures: set[str] = {
        occurrence.physical_signature
        for occurrence in presealed.variant_quarantine.occurrences
    }
    for split in PRESEALED_SPLITS:
        for example in presealed.by_split[split]:
            signature, error = row_physical_signature(example)
            if error is not None:
                raise ValueError(
                    f"malformed presealed row while merging test: {error}"
                )
            presealed_signatures.add(signature)

    safety_rows = tuple(
        sorted(
            set((*presealed.safety, *sealed.safety)),
            key=lambda item: (
                item.base_signature,
                item.trigger,
                item.source_group,
            ),
        )
    )
    safety_signatures: set[str] = set()
    safety_bases = {example.base_signature for example in safety_rows}
    for example in safety_rows:
        signature, error = row_physical_signature(example)
        if error is not None:
            raise ValueError(
                f"malformed safety row while merging test: {error}"
            )
        safety_signatures.add(signature)

    excluded: set[str] = set()
    retained_test: list[LexicalExample] = []
    for example in sealed.by_split["test"]:
        signature, error = row_physical_signature(example)
        if error is not None:
            raise ValueError(f"malformed sealed test row: {error}")
        if (
            signature in presealed_signatures
            or signature in safety_signatures
            or example.base_signature in safety_bases
        ):
            excluded.add(signature)
            continue
        retained_test.append(example)

    merged = DatasetBundle(
        {
            **{
                split: presealed.by_split[split]
                for split in PRESEALED_SPLITS
            },
            "test": tuple(retained_test),
        },
        safety_rows,
        presealed.variant_quarantine,
        sealed.variant_quarantine,
        tuple(sorted(excluded)),
    )
    for split in PRESEALED_SPLITS:
        if merged.by_split[split] != presealed.by_split[split]:
            raise AssertionError("sealed merge changed candidate rows")
    return merged


_AUGMENTED_VARIANT_KINDS: Final[frozenset[str]] = frozenset(
    {"identity", "deletion", "duplication", "transposition"}
)


def audit_dataset_physical_signatures(
    dataset: DatasetBundle,
) -> DatasetSignatureAudit:
    """Audit actual rendered lexical rows, independently of base signatures."""

    owners: dict[str, set[SplitName]] = defaultdict(set)
    intended_groups: dict[str, set[int]] = defaultdict(set)
    base_owners: set[str] = set()
    malformed: list[str] = []
    audited_rows = 0
    skipped_rows = 0
    quarantined_occurrences = {
        (
            occurrence.physical_signature,
            occurrence.base_signature,
            occurrence.split,
            occurrence.group,
            occurrence.variant_kind,
        )
        for occurrence in (
            *dataset.variant_quarantine.occurrences,
            *dataset.sealed_variant_quarantine.occurrences,
        )
    }
    present_quarantined: set[str] = set()
    for split in SPLIT_NAMES:
        for index, example in enumerate(dataset.by_split[split]):
            base_owners.add(example.base_signature)
            audited_rows += 1
            signature, error = row_physical_signature(example)
            if error is not None:
                malformed.append(
                    f"{split}[{index}] {example.base_signature}: {error}"
                )
                continue
            owners[signature].add(split)
            intended_groups[signature].add(
                example.target_group if example.label else example.source_group
            )
            intended_group = (
                example.target_group if example.label else example.source_group
            )
            if (
                (
                    signature,
                    example.base_signature,
                    split,
                    intended_group,
                    example.variant_kind,
                )
                in quarantined_occurrences
                and example.variant_kind in _AUGMENTED_VARIANT_KINDS
            ):
                present_quarantined.add(signature)
    safety_signatures: set[str] = set()
    safety_base_signatures = {
        example.base_signature for example in dataset.safety
    }
    for index, example in enumerate(dataset.safety):
        signature, error = row_physical_signature(example)
        if error is not None:
            malformed.append(
                f"safety[{index}] {example.base_signature}: {error}"
            )
            continue
        safety_signatures.add(signature)
    cross_split = tuple(
        (signature, tuple(sorted(splits)))
        for signature, splits in sorted(owners.items())
        if len(splits) > 1
    )
    cross_language = tuple(
        (signature, tuple(sorted(groups)))
        for signature, groups in sorted(intended_groups.items())
        if len(groups) > 1
    )
    return DatasetSignatureAudit(
        audited_rows=audited_rows,
        skipped_nonlexical_rows=skipped_rows,
        unique_physical_signatures=len(owners),
        cross_split_signatures=cross_split,
        cross_language_signatures=cross_language,
        safety_overlap_signatures=tuple(sorted(set(owners) & safety_signatures)),
        safety_base_signature_overlaps=tuple(
            sorted(base_owners & safety_base_signatures)
        ),
        malformed_rows=tuple(malformed),
        quarantined_signatures_present=tuple(sorted(present_quarantined)),
    )


def _assert_variant_quarantine_integrity(
    dataset: DatasetBundle,
    quarantine: VariantQuarantine,
    *,
    label: str,
    included_splits: Collection[SplitName],
) -> None:
    """Validate one phase's quarantine against evidence from that phase only."""

    selected_splits = frozenset(included_splits)
    if not selected_splits or not selected_splits <= frozenset(SPLIT_NAMES):
        raise ValueError("quarantine split scope must be a non-empty split subset")
    expected_quarantine_hash = variant_quarantine_fingerprint(
        quarantine.occurrences
    )
    if quarantine.sha256 != expected_quarantine_hash:
        raise ValueError(
            f"{label} variant quarantine fingerprint does not match its records"
        )
    if len(set(quarantine.occurrences)) != len(quarantine.occurrences):
        raise ValueError(
            f"{label} variant quarantine contains duplicate occurrence records"
        )

    quarantine_split_owners: dict[str, set[SplitName]] = defaultdict(set)
    quarantine_group_owners: dict[str, set[int]] = defaultdict(set)
    quarantine_reasons: dict[str, set[QuarantineReason]] = defaultdict(set)
    for occurrence in quarantine.occurrences:
        if occurrence.split not in selected_splits:
            raise ValueError(
                f"{label} variant quarantine occurrence is outside its phase: "
                f"{occurrence.split}"
            )
        quarantine_split_owners[occurrence.physical_signature].add(occurrence.split)
        quarantine_group_owners[occurrence.physical_signature].add(occurrence.group)
        quarantine_reasons[occurrence.physical_signature].add(occurrence.reason)
    for split in selected_splits:
        for example in dataset.by_split[split]:
            signature, error = row_physical_signature(example)
            if error is not None:
                continue
            if example.variant_kind in _AUGMENTED_VARIANT_KINDS:
                quarantine_split_owners[signature].add(split)
                quarantine_group_owners[signature].add(
                    example.target_group
                    if example.label
                    else example.source_group
                )
            elif example.variant_kind == "protected":
                quarantine_split_owners[signature].add(split)
                quarantine_group_owners[signature].add(example.source_group)
    for example in dataset.safety:
        signature, error = row_physical_signature(example)
        if error is not None:
            continue
        if example.protected or (
            example.variant_kind == "lexical_collision"
            and stable_split(example.base_signature) in selected_splits
        ):
            quarantine_group_owners[signature].update((0, 1))
    invalid_quarantine: list[str] = []
    for signature in sorted(quarantine_reasons):
        expected_reason = _variant_quarantine_reason(
            quarantine_split_owners[signature],
            quarantine_group_owners[signature],
        )
        if expected_reason is None or quarantine_reasons[signature] != {
            expected_reason
        }:
            invalid_quarantine.append(signature)
    if invalid_quarantine:
        raise ValueError(
            f"{label} variant quarantine ownership/reason evidence is invalid: "
            f"{tuple(invalid_quarantine[:3])!r}"
        )


def assert_no_split_leakage(
    dataset: DatasetBundle,
    *,
    variant_quarantine_splits: Collection[SplitName] = PRESEALED_SPLITS,
) -> None:
    """Validate phase-local quarantine evidence and all emitted dataset rows."""

    scope_label = (
        "sealed"
        if frozenset(variant_quarantine_splits) == frozenset(SEALED_TEST_SPLITS)
        else "candidate"
    )
    _assert_variant_quarantine_integrity(
        dataset,
        dataset.variant_quarantine,
        label=scope_label,
        included_splits=variant_quarantine_splits,
    )
    _assert_variant_quarantine_integrity(
        dataset,
        dataset.sealed_variant_quarantine,
        label="sealed",
        included_splits=SEALED_TEST_SPLITS,
    )
    all_quarantine_occurrences = (
        *dataset.variant_quarantine.occurrences,
        *dataset.sealed_variant_quarantine.occurrences,
    )
    if len(set(all_quarantine_occurrences)) != len(
        all_quarantine_occurrences
    ):
        raise ValueError(
            "candidate and sealed variant quarantines contain a duplicate "
            "occurrence record"
        )

    owners: dict[str, SplitName] = {}
    for split in SPLIT_NAMES:
        for example in dataset.by_split[split]:
            previous = owners.setdefault(example.base_signature, split)
            if previous != split:
                raise ValueError(
                    f"physical signature leaked from {previous} into {split}: "
                    f"{example.base_signature}"
                )

    audit = audit_dataset_physical_signatures(dataset)
    emitted_test_signatures = {
        signature
        for example in dataset.by_split["test"]
        for signature, error in (row_physical_signature(example),)
        if error is None
    }
    if emitted_test_signatures & set(
        dataset.sealed_test_exclusion_signatures
    ):
        raise ValueError("excluded sealed-test signature was emitted")
    if audit.malformed_rows:
        raise ValueError(
            "generated lexical rows have inconsistent physical signatures: "
            f"{audit.malformed_rows[:3]!r}"
        )
    if audit.cross_split_signatures:
        raise ValueError(
            "actual augmented physical signature leaked between splits: "
            f"{audit.cross_split_signatures[:3]!r}"
        )
    if audit.cross_language_signatures:
        raise ValueError(
            "actual augmented physical signature has contradictory intended "
            f"languages: {audit.cross_language_signatures[:3]!r}"
        )
    if audit.safety_overlap_signatures:
        raise ValueError(
            "actual augmented physical signature overlaps the safety corpus: "
            f"{audit.safety_overlap_signatures[:3]!r}"
        )
    if audit.safety_base_signature_overlaps:
        raise ValueError(
            "base signature overlaps the safety corpus: "
            f"{audit.safety_base_signature_overlaps[:3]!r}"
        )
    if audit.quarantined_signatures_present:
        raise ValueError(
            "quarantined physical signatures were emitted as rows: "
            f"{audit.quarantined_signatures_present[:3]!r}"
        )


def dataset_fingerprint(dataset: DatasetBundle) -> str:
    """Hash every sealed row so evaluation can verify exact provenance."""

    digest = hashlib.sha256()
    for label, quarantine in (
        ("candidate-variant-quarantine", dataset.variant_quarantine),
        ("sealed-variant-quarantine", dataset.sealed_variant_quarantine),
    ):
        expected_quarantine_hash = variant_quarantine_fingerprint(
            quarantine.occurrences
        )
        if quarantine.sha256 != expected_quarantine_hash:
            raise ValueError(
                f"{label} fingerprint does not match its records"
            )
        digest.update(
            (
                f"{label}:"
                f"{quarantine.occurrence_count}:"
                f"{quarantine.physical_signature_count}:"
                f"{quarantine.sha256}\n"
            ).encode("utf-8")
        )
        for occurrence in quarantine.occurrences:
            digest.update(
                json.dumps(
                    asdict(occurrence),
                    ensure_ascii=False,
                    sort_keys=True,
                    separators=(",", ":"),
                ).encode("utf-8")
            )
            digest.update(b"\n")
    if tuple(sorted(set(dataset.sealed_test_exclusion_signatures))) != (
        dataset.sealed_test_exclusion_signatures
    ):
        raise ValueError("sealed test exclusions must be sorted and unique")
    digest.update(b"sealed-test-exclusions\n")
    for signature in dataset.sealed_test_exclusion_signatures:
        digest.update(signature.encode("utf-8") + b"\n")
    for split in SPLIT_NAMES:
        digest.update(f"split:{split}\n".encode())
        for example in dataset.by_split[split]:
            digest.update(
                json.dumps(
                    asdict(example),
                    ensure_ascii=False,
                    sort_keys=True,
                    separators=(",", ":"),
                ).encode("utf-8")
            )
            digest.update(b"\n")
    digest.update(b"safety\n")
    for example in dataset.safety:
        digest.update(
            json.dumps(
                asdict(example),
                ensure_ascii=False,
                sort_keys=True,
                separators=(",", ":"),
            ).encode("utf-8")
        )
        digest.update(b"\n")
    return digest.hexdigest()


def normalize_sparse_features(
    features: Iterable[tuple[int, float]], dimension: int
) -> SparseFeatures:
    combined: dict[int, float] = {}
    for index, value in features:
        if not 0 <= index < dimension:
            raise ValueError("feature index outside model dimension")
        if not math.isfinite(value):
            raise ValueError("feature value must be finite")
        combined[index] = combined.get(index, 0.0) + value
    return tuple(
        (index, value)
        for index, value in sorted(combined.items())
        if value != 0.0
    )


@dataclass(frozen=True)
class FeaturedExample:
    example: LexicalExample
    features: SparseFeatures


@dataclass(frozen=True)
class ExtractedExampleFeatures:
    values: SparseFeatures
    character_fingerprints: frozenset[int]


class ExampleFeatureExtractor(Protocol):
    def __call__(
        self, example: LexicalExample, dimension: int
    ) -> ExtractedExampleFeatures: ...


def featurize_examples(
    examples: Iterable[LexicalExample],
    dimension: int,
    extractor: ExampleFeatureExtractor,
    *,
    supported_fingerprints: set[int] | None = None,
) -> tuple[FeaturedExample, ...]:
    result: list[FeaturedExample] = []
    for example in examples:
        extracted = extractor(example, dimension)
        if supported_fingerprints is not None:
            supported_fingerprints.update(extracted.character_fingerprints)
        result.append(
            FeaturedExample(
                example,
                normalize_sparse_features(extracted.values, dimension),
            )
        )
    return tuple(result)


@dataclass(frozen=True)
class FTRLParameters:
    dimension: int
    alpha: float
    beta: float
    l1: float
    l2: float


class FTRLProximal:
    """Sparse logistic regression trained with FTRL-Proximal."""

    def __init__(self, parameters: FTRLParameters) -> None:
        if parameters.dimension < 1 or parameters.alpha <= 0.0:
            raise ValueError("invalid FTRL parameters")
        if min(parameters.beta, parameters.l1, parameters.l2) < 0.0:
            raise ValueError("FTRL regularisation cannot be negative")
        self.parameters = parameters
        self.z: dict[int, float] = {}
        self.n: dict[int, float] = {}
        self.bias_z = 0.0
        self.bias_n = 0.0

    def weight(self, index: int) -> float:
        z_value = self.z.get(index, 0.0)
        if abs(z_value) <= self.parameters.l1:
            return 0.0
        sign = -1.0 if z_value < 0.0 else 1.0
        denominator = (
            (self.parameters.beta + math.sqrt(self.n.get(index, 0.0)))
            / self.parameters.alpha
            + self.parameters.l2
        )
        return -(z_value - sign * self.parameters.l1) / denominator

    @property
    def bias(self) -> float:
        denominator = (
            (self.parameters.beta + math.sqrt(self.bias_n))
            / self.parameters.alpha
        )
        return -self.bias_z / denominator if denominator else 0.0

    def score(self, features: SparseFeatures) -> float:
        return self.bias + sum(
            self.weight(index) * value for index, value in features
        )

    def update(
        self, features: SparseFeatures, label: bool, sample_weight: float = 1.0
    ) -> float:
        if sample_weight <= 0.0 or not math.isfinite(sample_weight):
            raise ValueError("sample weight must be positive and finite")
        current_weights: list[float] = []
        previous_index = -1
        for index, value in features:
            if (
                isinstance(index, bool)
                or not 0 <= index < self.parameters.dimension
            ):
                raise ValueError("feature index outside model dimension")
            if index <= previous_index:
                raise ValueError(
                    "sparse feature indices must be unique and strictly increasing"
                )
            if not math.isfinite(value):
                raise ValueError("feature value must be finite")
            previous_index = index
            current_weights.append(self.weight(index))

        old_bias = self.bias
        prediction = stable_sigmoid(
            old_bias
            + sum(
                weight * value
                for weight, (_index, value) in zip(
                    current_weights, features, strict=True
                )
            )
        )
        residual = (prediction - float(label)) * sample_weight
        old_bias_n = self.bias_n
        new_bias_n = old_bias_n + residual * residual
        bias_sigma = (
            math.sqrt(new_bias_n) - math.sqrt(old_bias_n)
        ) / self.parameters.alpha
        self.bias_z += residual - bias_sigma * old_bias
        self.bias_n = new_bias_n
        for (index, value), weight in zip(
            features, current_weights, strict=True
        ):
            gradient = residual * value
            old_n = self.n.get(index, 0.0)
            new_n = old_n + gradient * gradient
            sigma = (math.sqrt(new_n) - math.sqrt(old_n)) / self.parameters.alpha
            self.z[index] = self.z.get(index, 0.0) + gradient - sigma * weight
            self.n[index] = new_n
        return prediction

    def sparse_weights(self) -> dict[int, float]:
        return {
            index: weight
            for index in sorted(self.z)
            if (weight := self.weight(index)) != 0.0
        }

    def nonzero_weight_count(self) -> int:
        """Count active weights without sorting or materialising a mapping."""

        return sum(self.weight(index) != 0.0 for index in self.z)

    def clone(self) -> FTRLProximal:
        cloned = FTRLProximal(self.parameters)
        cloned.z = dict(self.z)
        cloned.n = dict(self.n)
        cloned.bias_z = self.bias_z
        cloned.bias_n = self.bias_n
        return cloned


def logistic_loss(logit: float, label: bool) -> float:
    target = float(label)
    return max(logit, 0.0) - target * logit + math.log1p(math.exp(-abs(logit)))


def mean_logistic_loss(model: FTRLProximal, examples: Sequence[FeaturedExample]) -> float:
    if not examples:
        raise ValueError("cannot evaluate an empty dataset")
    weighted_loss = sum(
        logistic_loss(model.score(item.features), item.example.label)
        * item.example.weight
        for item in examples
    )
    total_weight = sum(item.example.weight for item in examples)
    return weighted_loss / total_weight


@dataclass(frozen=True)
class EpochReport:
    epoch: int
    development_log_loss: float
    development_operating_thresholds: dict[str, float]
    development_precision: float
    development_recall: float
    development_specificity: float
    development_false_positive: int
    development_false_positive_rate_upper_familywise_95: float
    development_typo_precision: float
    development_typo_recall: float
    development_typo_specificity: float
    development_typo_false_positive: int
    development_typo_false_positive_rate_upper_familywise_95: float
    development_policy_checks_passed: int
    development_policy_passed: bool
    nonzero_weights: int


@dataclass(frozen=True)
class FTRLTrainingResult:
    model: FTRLProximal
    best_epoch: int
    history: tuple[EpochReport, ...]


def fit_ftrl(
    training: Sequence[FeaturedExample],
    development: Sequence[FeaturedExample],
    config: TrainingConfig,
) -> FTRLTrainingResult:
    if not training or not development:
        raise ValueError("training and development datasets must not be empty")
    parameters = FTRLParameters(
        config.dimension,
        config.ftrl_alpha,
        config.ftrl_beta,
        config.ftrl_l1,
        config.ftrl_l2,
    )
    model = FTRLProximal(parameters)
    best_model: FTRLProximal | None = None
    best_ranking: (
        tuple[int, int, float, float, float, float, float, float, float] | None
    ) = None
    best_epoch = 0
    stale_epochs = 0
    history: list[EpochReport] = []
    indices = list(range(len(training)))
    for epoch in range(1, config.maximum_epochs + 1):
        random.Random(config.seed + epoch).shuffle(indices)
        for index in indices:
            item = training[index]
            model.update(item.features, item.example.label, item.example.weight)
        evaluation = evaluate_development_epoch(model, development, config)
        report = EpochReport(
            epoch=epoch,
            development_log_loss=evaluation.log_loss,
            development_operating_thresholds={
                direction: evaluation.operating_point.logit_for(direction)
                for direction in LAYOUT_DIRECTIONS
            },
            development_precision=evaluation.operating_point.metrics.precision,
            development_recall=evaluation.operating_point.metrics.recall,
            development_specificity=(
                evaluation.operating_point.metrics.specificity
            ),
            development_false_positive=(
                evaluation.operating_point.metrics.false_positive
            ),
            development_false_positive_rate_upper_familywise_95=(
                evaluation.false_positive_rate_upper_familywise_95
            ),
            development_typo_precision=(
                evaluation.operating_point.typo_metrics.precision
            ),
            development_typo_recall=(
                evaluation.operating_point.typo_metrics.recall
            ),
            development_typo_specificity=(
                evaluation.operating_point.typo_metrics.specificity
            ),
            development_typo_false_positive=(
                evaluation.operating_point.typo_metrics.false_positive
            ),
            development_typo_false_positive_rate_upper_familywise_95=(
                evaluation.typo_false_positive_rate_upper_familywise_95
            ),
            development_policy_checks_passed=evaluation.policy_checks_passed,
            development_policy_passed=evaluation.policy_passed,
            nonzero_weights=model.nonzero_weight_count(),
        )
        history.append(report)
        ranking = evaluation.ranking()
        if best_ranking is None or ranking > best_ranking:
            best_ranking = ranking
            best_model = model.clone()
            best_epoch = epoch
            stale_epochs = 0
        else:
            stale_epochs += 1
        if epoch >= config.minimum_epochs and stale_epochs >= config.patience:
            break
    if best_model is None:
        raise AssertionError("at least one training epoch must produce a model")
    return FTRLTrainingResult(best_model, best_epoch, tuple(history))


@dataclass(frozen=True)
class QuantizedWeights:
    dimension: int
    scale: float
    values: tuple[int, ...]
    support: bytes
    maximum_absolute_error: float

    def dequantized(self) -> tuple[float, ...]:
        return tuple(value * self.scale for value in self.values)


def quantize_weights(
    weights: Mapping[int, float], dimension: int
) -> QuantizedWeights:
    if dimension < 1:
        raise ValueError("dimension must be positive")
    if any(not 0 <= index < dimension for index in weights):
        raise ValueError("weight index outside model dimension")
    if any(not math.isfinite(value) for value in weights.values()):
        raise ValueError("weights must be finite")
    maximum = max((abs(value) for value in weights.values()), default=0.0)
    scale = maximum / 32767.0 if maximum else 1.0
    values = [0] * dimension
    support = bytearray((dimension + 7) // 8)
    maximum_error = 0.0
    for index, weight in weights.items():
        quantized = max(-32767, min(32767, int(round(weight / scale))))
        values[index] = quantized
        if quantized:
            support[index // 8] |= 1 << (index % 8)
        maximum_error = max(maximum_error, abs(weight - quantized * scale))
    return QuantizedWeights(
        dimension,
        scale,
        tuple(values),
        bytes(support),
        maximum_error,
    )


@dataclass(frozen=True)
class QuantizedLinearScorer:
    weights: QuantizedWeights
    bias: float

    def score(self, features: SparseFeatures) -> float:
        # Keep the exact accumulation order used by LinearNgramModel.predict.
        # Thresholds are selected from these logits, so even a one-ULP
        # reduction difference at equality would break train/serve parity.
        logit = self.bias
        for index, value in features:
            logit += self.weights.values[index] * self.weights.scale * value
        return logit


@dataclass(frozen=True)
class PlattCalibration:
    slope: float
    intercept: float
    sample_count: int
    positive_count: int
    provenance: str = "lexical-synthetic-not-real-world-probability"

    def transform_logit(self, raw_logit: float) -> float:
        return self.slope * raw_logit + self.intercept

    def confidence(self, raw_logit: float) -> float:
        return stable_sigmoid(self.transform_logit(raw_logit))


@dataclass(frozen=True)
class DirectionalPlattCalibration:
    """Independent monotonic calibration for both EN/RU directions."""

    zero_to_one: PlattCalibration
    one_to_zero: PlattCalibration
    provenance: str = "lexical-synthetic-not-real-world-probability"

    @property
    def sample_count(self) -> int:
        return self.zero_to_one.sample_count + self.one_to_zero.sample_count

    @property
    def positive_count(self) -> int:
        return self.zero_to_one.positive_count + self.one_to_zero.positive_count

    def for_direction(self, direction: LayoutDirection) -> PlattCalibration:
        if direction == "0>1":
            return self.zero_to_one
        if direction == "1>0":
            return self.one_to_zero
        raise ValueError("unsupported calibration direction")

    def transform_logit(
        self,
        raw_logit: float,
        source_group: int,
        target_group: int,
    ) -> float:
        calibration = self.for_direction(
            layout_direction(source_group, target_group)
        )
        return calibration.transform_logit(raw_logit)

    def runtime_parameters(
        self,
    ) -> dict[LayoutDirection, PlattParameters]:
        return {
            direction: PlattParameters(
                self.for_direction(direction).slope,
                self.for_direction(direction).intercept,
            )
            for direction in LAYOUT_DIRECTIONS
        }

    def payload(self) -> dict[str, object]:
        return {
            "schema_version": 1,
            "method": "independent-platt-by-layout-direction",
            "provenance": self.provenance,
            "sample_count": self.sample_count,
            "positive_count": self.positive_count,
            "by_direction": {
                direction: asdict(self.for_direction(direction))
                for direction in LAYOUT_DIRECTIONS
            },
        }


def _calibration_loss(
    scores: Sequence[tuple[float, bool]], slope: float, intercept: float, l2: float
) -> float:
    return (
        sum(logistic_loss(slope * score + intercept, label) for score, label in scores)
        + 0.5 * l2 * (slope * slope + intercept * intercept)
    )


def fit_platt_calibration(
    scores: Sequence[tuple[float, bool]],
    *,
    l2: float,
    maximum_iterations: int,
) -> PlattCalibration:
    """Fit a two-parameter sigmoid only on the dedicated calibration split."""

    if l2 <= 0.0 or maximum_iterations < 1:
        raise ValueError("invalid calibration parameters")
    positives = sum(label for _score, label in scores)
    negatives = len(scores) - positives
    if positives == 0 or negatives == 0:
        raise ValueError("calibration requires both labels")
    slope = 1.0
    intercept = math.log((positives + 1.0) / (negatives + 1.0))
    for _iteration in range(maximum_iterations):
        gradient_slope = l2 * slope
        gradient_intercept = l2 * intercept
        hessian_ss = l2
        hessian_si = 0.0
        hessian_ii = l2
        for score, label in scores:
            logit = slope * score + intercept
            probability = stable_sigmoid(logit)
            residual = probability - float(label)
            curvature = max(1e-12, probability * (1.0 - probability))
            gradient_slope += residual * score
            gradient_intercept += residual
            hessian_ss += curvature * score * score
            hessian_si += curvature * score
            hessian_ii += curvature
        determinant = hessian_ss * hessian_ii - hessian_si * hessian_si
        if determinant <= 1e-18:
            break
        step_slope = (
            hessian_ii * gradient_slope - hessian_si * gradient_intercept
        ) / determinant
        step_intercept = (
            hessian_ss * gradient_intercept - hessian_si * gradient_slope
        ) / determinant
        if max(abs(step_slope), abs(step_intercept)) < 1e-10:
            break
        old_loss = _calibration_loss(scores, slope, intercept, l2)
        step_scale = 1.0
        accepted = False
        for _line_search in range(30):
            candidate_slope = slope - step_scale * step_slope
            candidate_intercept = intercept - step_scale * step_intercept
            new_loss = _calibration_loss(
                scores, candidate_slope, candidate_intercept, l2
            )
            if new_loss <= old_loss:
                slope = candidate_slope
                intercept = candidate_intercept
                accepted = True
                break
            step_scale *= 0.5
        if not accepted:
            break
    return PlattCalibration(slope, intercept, len(scores), positives)


def fit_directional_platt_calibration(
    scores: Sequence[tuple[float, bool, LayoutDirection]],
    *,
    l2: float,
    maximum_iterations: int,
) -> DirectionalPlattCalibration:
    """Fit each physical direction only on its calibration partition."""

    grouped: dict[LayoutDirection, list[tuple[float, bool]]] = {
        direction: [] for direction in LAYOUT_DIRECTIONS
    }
    for score, label, direction in scores:
        if direction not in grouped:
            raise ValueError("unsupported calibration direction")
        grouped[direction].append((score, label))
    fitted = {
        direction: fit_platt_calibration(
            grouped[direction],
            l2=l2,
            maximum_iterations=maximum_iterations,
        )
        for direction in LAYOUT_DIRECTIONS
    }
    return DirectionalPlattCalibration(
        fitted["0>1"],
        fitted["1>0"],
    )


@dataclass(frozen=True)
class ScoredExample:
    example: LexicalExample
    raw_logit: float
    calibrated_logit: float


class SparseScorer(Protocol):
    def score(self, features: SparseFeatures) -> float: ...


@dataclass(frozen=True)
class ConfusionMatrix:
    true_positive: int = 0
    false_negative: int = 0
    true_negative: int = 0
    false_positive: int = 0

    @property
    def precision(self) -> float:
        denominator = self.true_positive + self.false_positive
        return self.true_positive / denominator if denominator else 1.0

    @property
    def recall(self) -> float:
        denominator = self.true_positive + self.false_negative
        return self.true_positive / denominator if denominator else 1.0

    @property
    def specificity(self) -> float:
        denominator = self.true_negative + self.false_positive
        return self.true_negative / denominator if denominator else 1.0

    @property
    def false_positive_rate(self) -> float:
        return 1.0 - self.specificity


def score_examples(
    model: SparseScorer,
    calibration: DirectionalPlattCalibration,
    examples: Iterable[FeaturedExample],
) -> tuple[ScoredExample, ...]:
    result: list[ScoredExample] = []
    for item in examples:
        raw_logit = model.score(item.features)
        result.append(
            ScoredExample(
                item.example,
                raw_logit,
                calibration.transform_logit(
                    raw_logit,
                    item.example.source_group,
                    item.example.target_group,
                ),
            )
        )
    return tuple(result)


def confusion_at_threshold(
    examples: Iterable[ScoredExample], threshold: float
) -> ConfusionMatrix:
    true_positive = false_negative = true_negative = false_positive = 0
    for item in examples:
        predicted = item.calibrated_logit >= threshold
        if item.example.label and predicted:
            true_positive += 1
        elif item.example.label:
            false_negative += 1
        elif predicted:
            false_positive += 1
        else:
            true_negative += 1
    return ConfusionMatrix(true_positive, false_negative, true_negative, false_positive)


@dataclass(frozen=True)
class ThresholdSelection:
    trigger: CorrectionTrigger
    logit: float
    metrics: ConfusionMatrix
    typo_metrics: ConfusionMatrix
    direction_logits: Mapping[LayoutDirection, float] = field(
        default_factory=dict
    )
    global_logit_margin: float = 0.0

    def __post_init__(self) -> None:
        raw = (
            {direction: self.logit for direction in LAYOUT_DIRECTIONS}
            if not self.direction_logits
            else dict(self.direction_logits)
        )
        if set(raw) != set(LAYOUT_DIRECTIONS):
            raise ValueError("threshold selection must contain both directions")
        parsed: dict[LayoutDirection, float] = {}
        for direction in LAYOUT_DIRECTIONS:
            value = raw[direction]
            if not math.isfinite(value):
                raise ValueError("threshold selection logits must be finite")
            parsed[direction] = value
        if not math.isfinite(self.logit):
            raise ValueError("threshold selection logit must be finite")
        if (
            not math.isfinite(self.global_logit_margin)
            or self.global_logit_margin < 0.0
        ):
            raise ValueError(
                "threshold selection global logit margin must be finite and "
                "non-negative"
            )
        object.__setattr__(
            self,
            "direction_logits",
            MappingProxyType(parsed),
        )

    def logit_for(self, direction: LayoutDirection) -> float:
        return self.direction_logits[direction]

    def runtime_logits(self) -> dict[LayoutDirection, float]:
        return {
            direction: self.logit_for(direction)
            for direction in LAYOUT_DIRECTIONS
        }


@dataclass(frozen=True)
class DevelopmentEpochEvaluation:
    """Leakage-safe epoch objective measured only on development rows."""

    log_loss: float
    operating_point: ThresholdSelection
    false_positive_rate_upper_familywise_95: float
    typo_false_positive_rate_upper_familywise_95: float
    policy_checks_passed: int
    policy_passed: bool

    def ranking(
        self,
    ) -> tuple[int, int, float, float, float, float, float, float, float]:
        metrics = self.operating_point.metrics
        typo_metrics = self.operating_point.typo_metrics
        return (
            int(self.policy_passed),
            self.policy_checks_passed,
            metrics.recall,
            typo_metrics.recall,
            metrics.specificity,
            typo_metrics.specificity,
            metrics.precision,
            typo_metrics.precision,
            -self.log_loss,
        )


@dataclass(frozen=True)
class VetoSelection:
    raw_logit: float
    positive_samples: int
    vetoed_positive_samples: int
    false_negative_rate: float


def veto_metrics(
    examples: Iterable[ScoredExample], raw_logit: float
) -> VetoSelection:
    positives = [item for item in examples if item.example.label]
    if not positives:
        raise ValueError("veto evaluation requires positive examples")
    vetoed = sum(item.raw_logit < raw_logit for item in positives)
    return VetoSelection(
        raw_logit,
        len(positives),
        vetoed,
        vetoed / len(positives),
    )


def choose_veto_threshold(
    examples: Sequence[ScoredExample], *, quantile: float, margin: float
) -> VetoSelection:
    positive_scores = sorted(
        item.raw_logit for item in examples if item.example.label
    )
    if not positive_scores:
        raise ValueError("veto selection requires positive examples")
    if not 0.0 <= quantile <= 1.0 or margin < 0.0:
        raise ValueError("invalid veto selection parameters")
    index = min(
        len(positive_scores) - 1,
        int(math.floor(quantile * (len(positive_scores) - 1))),
    )
    threshold = positive_scores[index] - margin
    return veto_metrics(examples, threshold)


def choose_threshold(
    examples: Sequence[ScoredExample],
    trigger: CorrectionTrigger,
    *,
    precision_floor: float,
    maximum_false_positive_rate: float,
    minimum_recall: float = 0.0,
    minimum_specificity: float = 0.0,
    typo_precision_floor: float = 0.0,
    minimum_typo_recall: float = 0.0,
    typo_minimum_specificity: float = 0.0,
    typo_maximum_false_positive_rate: float = 1.0,
    false_positive_z_score: float = WILSON_95_Z_SCORE,
    typo_false_positive_z_score: float | None = None,
    maximum_false_positives: int | None = None,
) -> ThresholdSelection:
    if not 0.0 <= precision_floor <= 1.0:
        raise ValueError("precision_floor must be in [0, 1]")
    if not 0.0 <= maximum_false_positive_rate <= 1.0:
        raise ValueError("maximum_false_positive_rate must be in [0, 1]")
    for name, value in (
        ("minimum_recall", minimum_recall),
        ("minimum_specificity", minimum_specificity),
        ("typo_precision_floor", typo_precision_floor),
        ("minimum_typo_recall", minimum_typo_recall),
        ("typo_minimum_specificity", typo_minimum_specificity),
        (
            "typo_maximum_false_positive_rate",
            typo_maximum_false_positive_rate,
        ),
    ):
        if not 0.0 <= value <= 1.0:
            raise ValueError(f"{name} must be in [0, 1]")
    effective_typo_z_score = (
        false_positive_z_score
        if typo_false_positive_z_score is None
        else typo_false_positive_z_score
    )
    for name, value in (
        ("false_positive_z_score", false_positive_z_score),
        ("typo_false_positive_z_score", effective_typo_z_score),
    ):
        if not math.isfinite(value) or value <= 0.0:
            raise ValueError(f"{name} must be finite and positive")
    if maximum_false_positives is not None and (
        isinstance(maximum_false_positives, bool)
        or maximum_false_positives < 0
    ):
        raise ValueError(
            "maximum_false_positives must be a non-negative integer or None"
        )
    selected = tuple(item for item in examples if item.example.trigger == trigger)
    if not selected:
        raise ValueError(f"no threshold examples for trigger {trigger}")
    ordered = sorted(
        selected,
        key=lambda item: item.calibrated_logit,
        reverse=True,
    )
    positives = sum(item.example.label for item in ordered)
    negatives = len(ordered) - positives
    if positives == 0 or negatives == 0:
        raise ValueError(
            f"threshold examples for {trigger} must contain both labels"
        )
    metrics = ConfusionMatrix(0, positives, negatives, 0)
    typo_items = tuple(
        item for item in ordered if item.example.variant_kind in TYPO_VARIANT_KINDS
    )
    typo_positives = sum(item.example.label for item in typo_items)
    typo_negatives = len(typo_items) - typo_positives
    if typo_positives == 0 or typo_negatives == 0:
        raise ValueError(
            f"typo threshold examples for {trigger} must contain both labels"
        )
    typo_metrics = ConfusionMatrix(0, typo_positives, typo_negatives, 0)
    maximum_score = ordered[0].calibrated_logit
    fallback: tuple[float, ConfusionMatrix, ConfusionMatrix] | None = None
    best: tuple[float, ConfusionMatrix, ConfusionMatrix] | None = None

    def ranking(
        item: tuple[float, ConfusionMatrix, ConfusionMatrix],
    ) -> tuple[float, float, float, float, float]:
        item_threshold, item_metrics, item_typo_metrics = item
        return (
            item_metrics.recall,
            item_typo_metrics.recall,
            item_metrics.specificity,
            item_metrics.precision,
            item_threshold,
        )

    def consider(
        threshold: float,
        candidate_metrics: ConfusionMatrix,
        candidate_typo_metrics: ConfusionMatrix,
    ) -> None:
        nonlocal best, fallback
        if (
            (
                maximum_false_positives is not None
                and candidate_metrics.false_positive
                > maximum_false_positives
            )
            or candidate_metrics.precision + 1e-15 < precision_floor
            or wilson_upper_bound(
                candidate_metrics.false_positive,
                candidate_metrics.true_negative
                + candidate_metrics.false_positive,
                false_positive_z_score,
            )
            > maximum_false_positive_rate + 1e-15
        ):
            return
        candidate = (threshold, candidate_metrics, candidate_typo_metrics)
        if fallback is None or ranking(candidate) > ranking(fallback):
            fallback = candidate
        typo_negative_count = (
            candidate_typo_metrics.true_negative
            + candidate_typo_metrics.false_positive
        )
        full_policy_passes = (
            candidate_metrics.recall >= minimum_recall
            and candidate_metrics.specificity >= minimum_specificity
            and candidate_typo_metrics.precision >= typo_precision_floor
            and candidate_typo_metrics.recall >= minimum_typo_recall
            and candidate_typo_metrics.specificity
            >= typo_minimum_specificity
            and wilson_upper_bound(
                candidate_typo_metrics.false_positive,
                typo_negative_count,
                effective_typo_z_score,
            )
            <= typo_maximum_false_positive_rate + 1e-15
        )
        if full_policy_passes and (
            best is None or ranking(candidate) > ranking(best)
        ):
            best = candidate

    consider(math.nextafter(maximum_score, math.inf), metrics, typo_metrics)
    index = 0
    while index < len(ordered):
        score = ordered[index].calibrated_logit
        gained_positive = 0
        gained_negative = 0
        gained_typo_positive = 0
        gained_typo_negative = 0
        while index < len(ordered) and ordered[index].calibrated_logit == score:
            item = ordered[index]
            if item.example.label:
                gained_positive += 1
            else:
                gained_negative += 1
            if item.example.variant_kind in TYPO_VARIANT_KINDS:
                if item.example.label:
                    gained_typo_positive += 1
                else:
                    gained_typo_negative += 1
            index += 1
        metrics = ConfusionMatrix(
            metrics.true_positive + gained_positive,
            metrics.false_negative - gained_positive,
            metrics.true_negative - gained_negative,
            metrics.false_positive + gained_negative,
        )
        typo_metrics = ConfusionMatrix(
            typo_metrics.true_positive + gained_typo_positive,
            typo_metrics.false_negative - gained_typo_positive,
            typo_metrics.true_negative - gained_typo_negative,
            typo_metrics.false_positive + gained_typo_negative,
        )
        consider(score, metrics, typo_metrics)
    selected_best = best or fallback
    if selected_best is None:
        raise RuntimeError(
            f"no statistically certified threshold exists for trigger {trigger}"
        )
    return ThresholdSelection(
        trigger,
        selected_best[0],
        selected_best[1],
        selected_best[2],
    )


def _sum_confusion(left: ConfusionMatrix, right: ConfusionMatrix) -> ConfusionMatrix:
    return ConfusionMatrix(
        left.true_positive + right.true_positive,
        left.false_negative + right.false_negative,
        left.true_negative + right.true_negative,
        left.false_positive + right.false_positive,
    )


def confusion_at_directional_threshold(
    examples: Iterable[ScoredExample],
    logits: Mapping[LayoutDirection, float],
) -> ConfusionMatrix:
    if set(logits) != set(LAYOUT_DIRECTIONS) or any(
        not math.isfinite(value) for value in logits.values()
    ):
        raise ValueError("directional thresholds must contain finite EN/RU logits")
    true_positive = false_negative = true_negative = false_positive = 0
    for item in examples:
        direction = layout_direction(
            item.example.source_group,
            item.example.target_group,
        )
        predicted = item.calibrated_logit >= logits[direction]
        if item.example.label and predicted:
            true_positive += 1
        elif item.example.label:
            false_negative += 1
        elif predicted:
            false_positive += 1
        else:
            true_negative += 1
    return ConfusionMatrix(
        true_positive,
        false_negative,
        true_negative,
        false_positive,
    )


def _maximum_primary_false_positives(
    *,
    positives: int,
    negatives: int,
    precision_floor: float,
    maximum_false_positive_rate: float,
    z_score: float,
) -> int:
    maximum = -1
    for false_positives in range(negatives + 1):
        precision = (
            positives / (positives + false_positives)
            if positives + false_positives
            else 1.0
        )
        if (
            precision + 1e-15 < precision_floor
            or wilson_upper_bound(false_positives, negatives, z_score)
            > maximum_false_positive_rate + 1e-15
        ):
            break
        maximum = false_positives
    if maximum < 0:
        raise RuntimeError("primary precision policy cannot accept any threshold")
    return maximum


def _direction_operating_curve(
    examples: Sequence[ScoredExample],
    direction: LayoutDirection,
    maximum_false_positives: int,
) -> tuple[tuple[float, ConfusionMatrix, ConfusionMatrix], ...]:
    selected = tuple(
        item
        for item in examples
        if layout_direction(
            item.example.source_group,
            item.example.target_group,
        )
        == direction
    )
    if not selected:
        raise ValueError(f"no threshold examples for direction {direction}")
    ordered = sorted(
        selected,
        key=lambda item: item.calibrated_logit,
        reverse=True,
    )
    positives = sum(item.example.label for item in ordered)
    negatives = len(ordered) - positives
    typo_items = tuple(
        item for item in ordered if item.example.variant_kind in TYPO_VARIANT_KINDS
    )
    typo_positives = sum(item.example.label for item in typo_items)
    typo_negatives = len(typo_items) - typo_positives
    if min(positives, negatives, typo_positives, typo_negatives) <= 0:
        raise ValueError(
            f"direction {direction} requires both labels overall and for typos"
        )
    metrics = ConfusionMatrix(0, positives, negatives, 0)
    typo_metrics = ConfusionMatrix(0, typo_positives, typo_negatives, 0)
    best_by_error_counts: dict[
        tuple[int, int],
        tuple[float, ConfusionMatrix, ConfusionMatrix],
    ] = {}

    def record(threshold: float) -> None:
        key = (metrics.false_positive, typo_metrics.false_positive)
        candidate = (threshold, metrics, typo_metrics)
        current = best_by_error_counts.get(key)
        candidate_ranking = (
            metrics.true_positive,
            typo_metrics.true_positive,
            threshold,
        )
        current_ranking = (
            current[1].true_positive,
            current[2].true_positive,
            current[0],
        ) if current is not None else None
        if current_ranking is None or candidate_ranking > current_ranking:
            best_by_error_counts[key] = candidate

    record(math.nextafter(ordered[0].calibrated_logit, math.inf))
    index = 0
    while index < len(ordered):
        score = ordered[index].calibrated_logit
        gained_positive = gained_negative = 0
        gained_typo_positive = gained_typo_negative = 0
        while index < len(ordered) and ordered[index].calibrated_logit == score:
            item = ordered[index]
            if item.example.label:
                gained_positive += 1
            else:
                gained_negative += 1
            if item.example.variant_kind in TYPO_VARIANT_KINDS:
                if item.example.label:
                    gained_typo_positive += 1
                else:
                    gained_typo_negative += 1
            index += 1
        metrics = ConfusionMatrix(
            metrics.true_positive + gained_positive,
            metrics.false_negative - gained_positive,
            metrics.true_negative - gained_negative,
            metrics.false_positive + gained_negative,
        )
        typo_metrics = ConfusionMatrix(
            typo_metrics.true_positive + gained_typo_positive,
            typo_metrics.false_negative - gained_typo_positive,
            typo_metrics.true_negative - gained_typo_negative,
            typo_metrics.false_positive + gained_typo_negative,
        )
        if metrics.false_positive > maximum_false_positives:
            break
        record(score)
    return tuple(
        best_by_error_counts[key]
        for key in sorted(best_by_error_counts)
    )


def choose_directional_threshold(
    examples: Sequence[ScoredExample],
    trigger: CorrectionTrigger,
    *,
    precision_floor: float,
    maximum_false_positive_rate: float,
    minimum_recall: float = 0.0,
    minimum_specificity: float = 0.0,
    typo_precision_floor: float = 0.0,
    minimum_typo_recall: float = 0.0,
    typo_minimum_specificity: float = 0.0,
    typo_maximum_false_positive_rate: float = 1.0,
    false_positive_z_score: float = WILSON_95_Z_SCORE,
    typo_false_positive_z_score: float | None = None,
    maximum_false_positives: int | None = None,
) -> ThresholdSelection:
    """Jointly allocate the certified false-positive budget by direction."""

    for name, value in (
        ("precision_floor", precision_floor),
        ("maximum_false_positive_rate", maximum_false_positive_rate),
        ("minimum_recall", minimum_recall),
        ("minimum_specificity", minimum_specificity),
        ("typo_precision_floor", typo_precision_floor),
        ("minimum_typo_recall", minimum_typo_recall),
        ("typo_minimum_specificity", typo_minimum_specificity),
        (
            "typo_maximum_false_positive_rate",
            typo_maximum_false_positive_rate,
        ),
    ):
        if not 0.0 <= value <= 1.0:
            raise ValueError(f"{name} must be in [0, 1]")
    effective_typo_z_score = (
        false_positive_z_score
        if typo_false_positive_z_score is None
        else typo_false_positive_z_score
    )
    for name, value in (
        ("false_positive_z_score", false_positive_z_score),
        ("typo_false_positive_z_score", effective_typo_z_score),
    ):
        if not math.isfinite(value) or value <= 0.0:
            raise ValueError(f"{name} must be finite and positive")
    if maximum_false_positives is not None and (
        isinstance(maximum_false_positives, bool)
        or maximum_false_positives < 0
    ):
        raise ValueError(
            "maximum_false_positives must be a non-negative integer or None"
        )
    selected = tuple(
        item for item in examples if item.example.trigger == trigger
    )
    if not selected:
        raise ValueError(f"no threshold examples for trigger {trigger}")
    positives = sum(item.example.label for item in selected)
    negatives = len(selected) - positives
    typo_selected = tuple(
        item for item in selected if item.example.variant_kind in TYPO_VARIANT_KINDS
    )
    typo_positives = sum(item.example.label for item in typo_selected)
    typo_negatives = len(typo_selected) - typo_positives
    if min(positives, negatives, typo_positives, typo_negatives) <= 0:
        raise ValueError(
            f"threshold examples for {trigger} require both labels overall and for typos"
        )
    certified_maximum_false_positives = _maximum_primary_false_positives(
        positives=positives,
        negatives=negatives,
        precision_floor=precision_floor,
        maximum_false_positive_rate=maximum_false_positive_rate,
        z_score=false_positive_z_score,
    )
    if maximum_false_positives is not None:
        certified_maximum_false_positives = min(
            certified_maximum_false_positives,
            maximum_false_positives,
        )
    curves = {
        direction: _direction_operating_curve(
            selected,
            direction,
            certified_maximum_false_positives,
        )
        for direction in LAYOUT_DIRECTIONS
    }
    fallback: tuple[
        dict[LayoutDirection, float],
        ConfusionMatrix,
        ConfusionMatrix,
    ] | None = None
    best: tuple[
        dict[LayoutDirection, float],
        ConfusionMatrix,
        ConfusionMatrix,
    ] | None = None

    def ranking(
        item: tuple[
            dict[LayoutDirection, float],
            ConfusionMatrix,
            ConfusionMatrix,
        ],
    ) -> tuple[float, float, float, float, float, float]:
        logits, metrics, typo_metrics = item
        return (
            metrics.recall,
            typo_metrics.recall,
            metrics.specificity,
            metrics.precision,
            logits["0>1"],
            logits["1>0"],
        )

    for forward in curves["0>1"]:
        for reverse in curves["1>0"]:
            logits: dict[LayoutDirection, float] = {
                "0>1": forward[0],
                "1>0": reverse[0],
            }
            metrics = _sum_confusion(forward[1], reverse[1])
            typo_metrics = _sum_confusion(forward[2], reverse[2])
            if (
                (
                    maximum_false_positives is not None
                    and (
                        metrics.false_positive > maximum_false_positives
                        or typo_metrics.false_positive
                        > maximum_false_positives
                    )
                )
                or metrics.precision + 1e-15 < precision_floor
                or wilson_upper_bound(
                    metrics.false_positive,
                    metrics.true_negative + metrics.false_positive,
                    false_positive_z_score,
                )
                > maximum_false_positive_rate + 1e-15
            ):
                continue
            candidate = (logits, metrics, typo_metrics)
            if fallback is None or ranking(candidate) > ranking(fallback):
                fallback = candidate
            full_policy_passes = (
                metrics.recall >= minimum_recall
                and metrics.specificity >= minimum_specificity
                and typo_metrics.precision >= typo_precision_floor
                and typo_metrics.recall >= minimum_typo_recall
                and typo_metrics.specificity >= typo_minimum_specificity
                and wilson_upper_bound(
                    typo_metrics.false_positive,
                    typo_metrics.true_negative + typo_metrics.false_positive,
                    effective_typo_z_score,
                )
                <= typo_maximum_false_positive_rate + 1e-15
            )
            if full_policy_passes and (
                best is None or ranking(candidate) > ranking(best)
            ):
                best = candidate
    selected_best = best or fallback
    if selected_best is None:
        raise RuntimeError(
            f"no statistically certified directional threshold exists for {trigger}"
        )
    return ThresholdSelection(
        trigger,
        max(selected_best[0].values()),
        selected_best[1],
        selected_best[2],
        selected_best[0],
    )


def evaluate_development_epoch(
    model: SparseScorer,
    examples: Sequence[FeaturedExample],
    config: TrainingConfig,
) -> DevelopmentEpochEvaluation:
    """Rank an epoch by its high-precision development operating point.

    The threshold split remains untouched.  Development rows form one aggregate
    operating curve: only their metadata trigger is normalized for grouping,
    while every already-extracted feature remains byte-for-byte unchanged.
    """

    if not examples:
        raise ValueError("cannot evaluate an empty development dataset")
    scored: list[ScoredExample] = []
    weighted_loss = 0.0
    total_weight = 0.0
    for item in examples:
        raw_logit = model.score(item.features)
        weighted_loss += (
            logistic_loss(raw_logit, item.example.label) * item.example.weight
        )
        total_weight += item.example.weight
        scored.append(
            ScoredExample(
                replace(item.example, trigger="space"),
                raw_logit,
                raw_logit,
            )
        )
    if total_weight <= 0.0:
        raise ValueError("development sample weights must have a positive sum")
    operating_point = choose_directional_threshold(
        scored,
        "space",
        precision_floor=config.threshold_precision_floor,
        maximum_false_positive_rate=1.0,
        typo_precision_floor=config.threshold_precision_floor,
        typo_maximum_false_positive_rate=1.0,
        false_positive_z_score=SELECTION_WILSON_Z_SCORE,
        typo_false_positive_z_score=SELECTION_WILSON_Z_SCORE,
        maximum_false_positives=(
            config.selection_maximum_false_positives_per_trigger
        ),
    )
    metrics = operating_point.metrics
    typo_metrics = operating_point.typo_metrics
    negative_samples = metrics.true_negative + metrics.false_positive
    typo_negative_samples = (
        typo_metrics.true_negative + typo_metrics.false_positive
    )
    false_positive_rate_upper_familywise_95 = wilson_upper_bound(
        metrics.false_positive,
        negative_samples,
        SELECTION_WILSON_Z_SCORE,
    )
    typo_false_positive_rate_upper_familywise_95 = wilson_upper_bound(
        typo_metrics.false_positive,
        typo_negative_samples,
        SELECTION_WILSON_Z_SCORE,
    )
    checks = (
        metrics.precision >= config.threshold_precision_floor,
        metrics.recall >= config.selection_minimum_recall,
        metrics.specificity >= config.test_minimum_specificity,
        false_positive_rate_upper_familywise_95
        <= config.threshold_max_false_positive_rate,
        typo_metrics.precision >= config.threshold_precision_floor,
        typo_metrics.recall >= config.selection_minimum_typo_recall,
        typo_metrics.specificity >= config.test_minimum_specificity,
        typo_false_positive_rate_upper_familywise_95
        <= config.threshold_max_false_positive_rate,
        metrics.false_positive
        <= config.selection_maximum_false_positives_per_trigger,
        typo_metrics.false_positive
        <= config.selection_maximum_false_positives_per_trigger,
    )
    return DevelopmentEpochEvaluation(
        log_loss=weighted_loss / total_weight,
        operating_point=operating_point,
        false_positive_rate_upper_familywise_95=(
            false_positive_rate_upper_familywise_95
        ),
        typo_false_positive_rate_upper_familywise_95=(
            typo_false_positive_rate_upper_familywise_95
        ),
        policy_checks_passed=sum(checks),
        policy_passed=all(checks),
    )


def _apply_trigger_threshold_margin(
    base: Mapping[CorrectionTrigger, ThresholdSelection],
    examples: Sequence[ScoredExample],
    config: TrainingConfig,
    margin: float,
) -> dict[CorrectionTrigger, ThresholdSelection]:
    """Apply one signed global margin and the additional Pause margin."""

    if not math.isfinite(margin) or not 0.0 <= margin <= (
        config.threshold_logit_margin_cap
    ):
        raise ValueError("global threshold margin is outside its signed cap")
    if set(base) != set(MODEL_TRIGGERS):
        raise ValueError("base threshold selection is incomplete")
    selected: dict[CorrectionTrigger, ThresholdSelection] = {}
    for trigger, selection in base.items():
        hardened_logits = {
            direction: selection.logit_for(direction) + margin
            for direction in LAYOUT_DIRECTIONS
        }
        trigger_examples = tuple(
            item for item in examples if item.example.trigger == trigger
        )
        selected[trigger] = ThresholdSelection(
            trigger,
            max(hardened_logits.values()),
            confusion_at_directional_threshold(
                trigger_examples, hardened_logits
            ),
            confusion_at_directional_threshold(
                (
                    item
                    for item in trigger_examples
                    if item.example.variant_kind in TYPO_VARIANT_KINDS
                ),
                hardened_logits,
            ),
            hardened_logits,
            margin,
        )
    pause_initial = selected["pause"]
    pause_logits: dict[LayoutDirection, float] = {
        direction: max(
            pause_initial.logit_for(direction),
            max(
                selection.logit_for(direction)
                for trigger, selection in selected.items()
                if trigger != "pause"
            )
            + config.pause_logit_margin,
        )
        for direction in LAYOUT_DIRECTIONS
    }
    pause_examples = tuple(
        item for item in examples if item.example.trigger == "pause"
    )
    selected["pause"] = ThresholdSelection(
        "pause",
        max(pause_logits.values()),
        confusion_at_directional_threshold(pause_examples, pause_logits),
        confusion_at_directional_threshold(
            (
                item
                for item in pause_examples
                if item.example.variant_kind in TYPO_VARIANT_KINDS
            ),
            pause_logits,
        ),
        pause_logits,
        margin,
    )
    return selected


def _threshold_selection_passes_for_margin(
    config: TrainingConfig,
    selections: Mapping[CorrectionTrigger, ThresholdSelection],
) -> bool:
    """Evaluate the complete context-invariant pre-seal threshold policy."""

    neutral = {
        trigger: selection.metrics
        for trigger, selection in selections.items()
    }
    neutral_typos = {
        trigger: selection.typo_metrics
        for trigger, selection in selections.items()
    }
    context = {
        profile.name: neutral for profile in CONTEXT_STRESS_PROFILES
    }
    context_typos = {
        profile.name: neutral_typos for profile in CONTEXT_STRESS_PROFILES
    }
    return threshold_selection_gate_breakdown(
        config,
        selections,
        context,
        context_typos,
    ).get("passed") is True


def _maximum_feasible_threshold_margin(
    base: Mapping[CorrectionTrigger, ThresholdSelection],
    examples: Sequence[ScoredExample],
    config: TrainingConfig,
) -> float:
    """Select the greatest stepwise margin that preserves every signed gate."""

    cap = config.threshold_logit_margin_cap
    zero = _apply_trigger_threshold_margin(base, examples, config, 0.0)
    if cap == 0.0 or not _threshold_selection_passes_for_margin(config, zero):
        return 0.0
    capped = _apply_trigger_threshold_margin(base, examples, config, cap)
    if _threshold_selection_passes_for_margin(config, capped):
        return cap

    intercepts = {
        trigger: selection.direction_logits
        for trigger, selection in zero.items()
    }
    candidates = {0.0, cap}
    for item in examples:
        if not item.example.label:
            continue
        trigger = item.example.trigger
        direction = layout_direction(
            item.example.source_group,
            item.example.target_group,
        )
        critical = (
            item.calibrated_logit - intercepts[trigger][direction]
        )
        if not math.isfinite(critical) or not 0.0 <= critical <= cap:
            continue
        candidates.add(critical)
        below = math.nextafter(critical, -math.inf)
        if below >= 0.0:
            candidates.add(below)
    ordered = sorted(candidates)
    lower = 0
    upper = len(ordered) - 1
    best = 0
    while lower <= upper:
        middle = (lower + upper) // 2
        margin = ordered[middle]
        selections = _apply_trigger_threshold_margin(
            base, examples, config, margin
        )
        if _threshold_selection_passes_for_margin(config, selections):
            best = middle
            lower = middle + 1
        else:
            upper = middle - 1
    return ordered[best]


def choose_trigger_thresholds(
    examples: Sequence[ScoredExample], config: TrainingConfig
) -> dict[CorrectionTrigger, ThresholdSelection]:
    base = {
        trigger: choose_directional_threshold(
            examples,
            trigger,
            precision_floor=config.threshold_precision_floor,
            maximum_false_positive_rate=(
                config.pause_threshold_max_false_positive_rate
                if trigger == "pause"
                else config.threshold_max_false_positive_rate
            ),
            minimum_recall=(
                config.selection_minimum_pause_recall
                if trigger == "pause"
                else config.selection_minimum_recall
            ),
            minimum_specificity=config.test_minimum_specificity,
            typo_precision_floor=config.threshold_precision_floor,
            minimum_typo_recall=(
                config.selection_minimum_pause_typo_recall
                if trigger == "pause"
                else config.selection_minimum_typo_recall
            ),
            typo_minimum_specificity=config.test_minimum_specificity,
            typo_maximum_false_positive_rate=(
                config.pause_threshold_max_false_positive_rate
                if trigger == "pause"
                else config.threshold_max_false_positive_rate
            ),
            false_positive_z_score=SELECTION_WILSON_Z_SCORE,
            typo_false_positive_z_score=SELECTION_WILSON_Z_SCORE,
            maximum_false_positives=(
                config.selection_maximum_false_positives_per_trigger
            ),
        )
        for trigger in MODEL_TRIGGERS
    }
    margin = _maximum_feasible_threshold_margin(base, examples, config)
    return _apply_trigger_threshold_margin(
        base,
        examples,
        config,
        margin,
    )


CONTEXT_INVARIANT_FEATURE_VERSION: Final[int] = 5
_CONTEXT_INVARIANCE_SENTINELS: Final[
    tuple[tuple[str, str, int, int], ...]
] = (
    ("test", "еуые", 0, 1),
    ("еуые", "test", 1, 0),
    ("keyboard", "лунищфкв", 0, 1),
    ("лунищфкв", "keyboard", 1, 0),
)


def verify_context_feature_invariance(
    *,
    dimension: int,
    extractor: ExampleFeatureExtractor,
) -> int:
    """Fail closed unless runtime schema v5 ignores the full context grid."""

    if FEATURE_VERSION != CONTEXT_INVARIANT_FEATURE_VERSION:
        raise RuntimeError(
            "fast context stress requires context-invariant feature schema v5"
        )
    comparisons = 0
    for original, alternative, source_group, target_group in (
        _CONTEXT_INVARIANCE_SENTINELS
    ):
        for trigger in MODEL_TRIGGERS:
            neutral = LexicalExample(
                original=original,
                alternative=alternative,
                source_group=source_group,
                target_group=target_group,
                trigger=trigger,
                label=False,
                weight=1.0,
                base_signature="context-invariance-sentinel",
                variant_kind="identity",
                source_known=False,
                target_known=False,
            )
            expected = extractor(neutral, dimension)
            for profile in CONTEXT_STRESS_PROFILES:
                stressed = context_stress_examples((neutral,), profile)[0]
                actual = extractor(stressed, dimension)
                comparisons += 1
                if actual != expected:
                    raise RuntimeError(
                        "feature schema is context-sensitive for sentinel "
                        f"{source_group}>{target_group}/{trigger}/{profile.name}"
                    )
    return comparisons


def score_context_stress_profiles(
    examples: Sequence[LexicalExample],
    *,
    dimension: int,
    extractor: ExampleFeatureExtractor,
    model: SparseScorer,
    calibration: DirectionalPlattCalibration,
    thresholds: Mapping[CorrectionTrigger, ThresholdSelection],
    neutral_scores: Sequence[ScoredExample] | None = None,
) -> tuple[
    dict[str, dict[CorrectionTrigger, ConfusionMatrix]],
    dict[str, dict[CorrectionTrigger, ConfusionMatrix]],
]:
    """Reuse neutral scores only after proving runtime context invariance."""

    verify_context_feature_invariance(
        dimension=dimension,
        extractor=extractor,
    )
    if neutral_scores is None:
        featured = featurize_examples(examples, dimension, extractor)
        scores = score_examples(model, calibration, featured)
    else:
        if len(neutral_scores) != len(examples) or any(
            scored.example != example
            for scored, example in zip(neutral_scores, examples, strict=True)
        ):
            raise ValueError("neutral scores do not match context-stress examples")
        scores = tuple(neutral_scores)

    overall = {
        trigger: confusion_at_directional_threshold(
            (item for item in scores if item.example.trigger == trigger),
            thresholds[trigger].direction_logits,
        )
        for trigger in MODEL_TRIGGERS
    }
    typos = {
        trigger: confusion_at_directional_threshold(
            (
                item
                for item in scores
                if item.example.trigger == trigger
                and item.example.variant_kind in TYPO_VARIANT_KINDS
            ),
            thresholds[trigger].direction_logits,
        )
        for trigger in MODEL_TRIGGERS
    }
    return (
        {profile.name: dict(overall) for profile in CONTEXT_STRESS_PROFILES},
        {profile.name: dict(typos) for profile in CONTEXT_STRESS_PROFILES},
    )


def metrics_payload(metrics: ConfusionMatrix) -> dict[str, object]:
    negative_count = metrics.true_negative + metrics.false_positive
    payload: dict[str, object] = asdict(metrics)
    payload.update(
        precision=round(metrics.precision, 9),
        recall=round(metrics.recall, 9),
        specificity=round(metrics.specificity, 9),
        false_positive_rate=round(metrics.false_positive_rate, 9),
        false_positive_rate_upper_95=round(
            wilson_upper_bound(metrics.false_positive, negative_count), 9
        ),
        negative_samples=negative_count,
    )
    return payload


def wilson_upper_bound(
    successes: int,
    samples: int,
    z_score: float = WILSON_95_Z_SCORE,
) -> float:
    """Return the upper Wilson endpoint for an explicit normal quantile."""

    if samples < 0 or successes < 0 or successes > samples:
        raise ValueError("invalid binomial counts")
    if not math.isfinite(z_score) or z_score <= 0.0:
        raise ValueError("z_score must be finite and positive")
    if samples == 0:
        return 1.0
    proportion = successes / samples
    z_squared = z_score * z_score
    denominator = 1.0 + z_squared / samples
    centre = proportion + z_squared / (2.0 * samples)
    radius = z_score * math.sqrt(
        proportion * (1.0 - proportion) / samples
        + z_squared / (4.0 * samples * samples)
    )
    return min(1.0, (centre + radius) / denominator)


def false_positive_bound_payload(
    successes: int,
    samples: int,
    *,
    selection_familywise: bool,
) -> dict[str, object]:
    """Describe the exact multiplicity policy behind one gate decision."""

    comparisons = (
        SELECTION_FALSE_POSITIVE_COMPARISONS
        if selection_familywise
        else 1
    )
    per_comparison_confidence = (
        SELECTION_PER_COMPARISON_CONFIDENCE
        if selection_familywise
        else WILSON_INTERVAL_CONFIDENCE
    )
    z_score = (
        SELECTION_WILSON_Z_SCORE
        if selection_familywise
        else WILSON_95_Z_SCORE
    )
    return {
        "method": "wilson_score_upper_endpoint",
        "multiplicity_correction": (
            "bonferroni" if selection_familywise else "none"
        ),
        "familywise_confidence": WILSON_INTERVAL_CONFIDENCE,
        "comparisons": comparisons,
        "per_comparison_confidence": per_comparison_confidence,
        "z_score": z_score,
        "upper": wilson_upper_bound(successes, samples, z_score),
    }


def gate_policy_payload(config: TrainingConfig) -> dict[str, object]:
    """Return the complete, self-contained release policy for audit reports."""

    return {
        "model_applicability": {
            "minimum_normalized_token_length": MINIMUM_RUNTIME_TOKEN_LENGTH,
            "length_comparison": "maximum_of_original_and_replacement",
            "model_first_after_hard_guards": True,
            "post_guard_decision_rule": (
                "trigger_direction_calibrated_logit_threshold_only"
            ),
            "membership_coverage_role": "diagnostic_only",
            "target_language_score_role": "diagnostic_only",
        },
        "sealed_evaluation": asdict(config.sealed_evaluation),
        "external_evaluation": {
            "schema_version": config.external_evaluation.schema_version,
            "minimum_words_per_group": (
                config.external_evaluation.minimum_words_per_group
            ),
            "trigger_expansion": list(
                config.external_evaluation.trigger_expansion
            ),
            "hunspell": {
                "en_US": asdict(config.external_evaluation.english),
                "ru_RU": asdict(config.external_evaluation.russian),
            },
            "lexical_disjoint_corpus_sha256": (
                config.external_evaluation.lexical_disjoint_corpus_sha256
            ),
            "unknown_typo_development_corpus_sha256": (
                config.external_evaluation.unknown_typo_development_corpus_sha256
            ),
            "unknown_typo_holdout_corpus_sha256": (
                config.external_evaluation.unknown_typo_holdout_corpus_sha256
            ),
        },
        "selection": {
            "context": "neutral_primary_plus_fixed_label_independent_stress",
            "context_stress_profiles": [
                asdict(profile) for profile in CONTEXT_STRESS_PROFILES
            ],
            "minimum_precision": config.threshold_precision_floor,
            "minimum_recall": config.selection_minimum_recall,
            "minimum_pause_recall": config.selection_minimum_pause_recall,
            "minimum_typo_precision": config.threshold_precision_floor,
            "minimum_typo_recall": config.selection_minimum_typo_recall,
            "minimum_pause_typo_recall": (
                config.selection_minimum_pause_typo_recall
            ),
            "minimum_specificity": config.test_minimum_specificity,
            "maximum_false_positive_rate_familywise_upper_95": (
                config.threshold_max_false_positive_rate
            ),
            "pause_maximum_false_positive_rate_familywise_upper_95": (
                config.pause_threshold_max_false_positive_rate
            ),
            "typo_maximum_false_positive_rate_familywise_upper_95": (
                config.threshold_max_false_positive_rate
            ),
            "pause_typo_maximum_false_positive_rate_familywise_upper_95": (
                config.pause_threshold_max_false_positive_rate
            ),
            "maximum_false_positives_per_trigger": (
                config.selection_maximum_false_positives_per_trigger
            ),
            "threshold_logit_margin_policy": (
                "maximum_feasible_not_above_signed_cap"
            ),
            "threshold_logit_margin_cap": (
                config.threshold_logit_margin_cap
            ),
            "pause_logit_margin": config.pause_logit_margin,
        },
        "sealed_test": {
            "context": "neutral_primary_plus_fixed_label_independent_stress",
            "context_stress_profiles": [
                asdict(profile) for profile in CONTEXT_STRESS_PROFILES
            ],
            "minimum_precision": config.test_minimum_precision,
            "minimum_recall": config.test_minimum_recall,
            "minimum_pause_recall": config.test_minimum_pause_recall,
            "minimum_typo_recall": config.test_minimum_typo_recall,
            "minimum_pause_typo_recall": (
                config.test_minimum_pause_typo_recall
            ),
            "minimum_specificity": config.test_minimum_specificity,
            "maximum_false_positive_rate_upper_95": (
                config.threshold_max_false_positive_rate
            ),
            "pause_maximum_false_positive_rate_upper_95": (
                config.pause_threshold_max_false_positive_rate
            ),
        },
        "veto": {
            "maximum_false_negative_rate": config.veto_max_false_negative_rate,
        },
        "safety": {
            "maximum_guard_failures": config.safety_maximum_guard_failures,
        },
        "statistical_bounds": {
            "selection": {
                "method": "wilson_score_upper_endpoint",
                "multiplicity_correction": "bonferroni",
                "familywise_confidence": WILSON_INTERVAL_CONFIDENCE,
                "comparisons": SELECTION_FALSE_POSITIVE_COMPARISONS,
                "per_comparison_confidence": (
                    SELECTION_PER_COMPARISON_CONFIDENCE
                ),
                "z_score": SELECTION_WILSON_Z_SCORE,
            },
            "sealed_test": {
                "method": "wilson_score_upper_endpoint",
                "multiplicity_correction": "none",
                "familywise_confidence": WILSON_INTERVAL_CONFIDENCE,
                "comparisons": 1,
                "per_comparison_confidence": WILSON_INTERVAL_CONFIDENCE,
                "z_score": WILSON_95_Z_SCORE,
            },
        },
    }


def binary_gate_breakdown(
    metrics: ConfusionMatrix,
    *,
    minimum_precision: float,
    minimum_recall: float,
    minimum_specificity: float,
    maximum_false_positive_rate: float,
    selection_familywise: bool = False,
) -> dict[str, object]:
    """Explain every non-vacuous binary quality constraint."""

    positive_count = metrics.true_positive + metrics.false_negative
    negative_count = metrics.true_negative + metrics.false_positive
    bound = false_positive_bound_payload(
        metrics.false_positive,
        negative_count,
        selection_familywise=selection_familywise,
    )
    false_positive_upper = wilson_upper_bound(
        metrics.false_positive,
        negative_count,
        (
            SELECTION_WILSON_Z_SCORE
            if selection_familywise
            else WILSON_95_Z_SCORE
        ),
    )
    checks = {
        "positive_samples": positive_count > 0,
        "negative_samples": negative_count > 0,
        "precision": metrics.precision >= minimum_precision,
        "recall": metrics.recall >= minimum_recall,
        "specificity": metrics.specificity >= minimum_specificity,
        "false_positive_rate_upper_bound": (
            false_positive_upper <= maximum_false_positive_rate
        ),
    }
    return {
        "passed": all(checks.values()),
        "checks": checks,
        "actual": metrics_payload(metrics),
        "false_positive_bound": bound,
        "limits": {
            "minimum_precision": minimum_precision,
            "minimum_recall": minimum_recall,
            "minimum_specificity": minimum_specificity,
            "maximum_false_positive_rate_upper_bound": (
                maximum_false_positive_rate
            ),
        },
    }


def context_stress_gate_breakdown(
    config: TrainingConfig,
    metrics_by_profile: Mapping[
        str, Mapping[CorrectionTrigger, ConfusionMatrix]
    ]
    | None,
    typo_metrics_by_profile: Mapping[
        str, Mapping[CorrectionTrigger, ConfusionMatrix]
    ]
    | None,
    *,
    phase: Literal["selection", "sealed_test"],
) -> dict[str, object]:
    """Require the complete fixed context matrix and fail closed if absent."""

    if phase not in {"selection", "sealed_test"}:
        raise ValueError("unknown context stress gate phase")

    expected_profiles = {
        profile.name for profile in CONTEXT_STRESS_PROFILES
    }
    if metrics_by_profile is None or typo_metrics_by_profile is None:
        return {
            "passed": False,
            "all_profiles_present": False,
            "expected_profiles": sorted(expected_profiles),
            "profiles": {},
            "reason": "context stress metrics are missing",
        }
    profiles_complete = (
        set(metrics_by_profile) == expected_profiles
        and set(typo_metrics_by_profile) == expected_profiles
    )
    profiles: dict[str, object] = {}
    for profile in CONTEXT_STRESS_PROFILES:
        overall_metrics = metrics_by_profile.get(profile.name)
        typo_metrics = typo_metrics_by_profile.get(profile.name)
        triggers_complete = (
            overall_metrics is not None
            and typo_metrics is not None
            and set(overall_metrics) == set(MODEL_TRIGGERS)
            and set(typo_metrics) == set(MODEL_TRIGGERS)
        )
        per_trigger: dict[str, object] = {}
        for trigger in MODEL_TRIGGERS:
            overall = (
                overall_metrics.get(trigger)
                if overall_metrics is not None
                else None
            )
            typos = (
                typo_metrics.get(trigger)
                if typo_metrics is not None
                else None
            )
            if overall is None or typos is None:
                per_trigger[trigger] = {
                    "passed": False,
                    "missing": True,
                }
                continue
            maximum_fpr = (
                config.pause_threshold_max_false_positive_rate
                if trigger == "pause"
                else config.threshold_max_false_positive_rate
            )
            overall_gate = binary_gate_breakdown(
                overall,
                minimum_precision=(
                    config.threshold_precision_floor
                    if phase == "selection"
                    else config.test_minimum_precision
                ),
                minimum_recall=(
                    (
                        config.selection_minimum_pause_recall
                        if trigger == "pause"
                        else config.selection_minimum_recall
                    )
                    if phase == "selection"
                    else (
                        config.test_minimum_pause_recall
                        if trigger == "pause"
                        else config.test_minimum_recall
                    )
                ),
                minimum_specificity=config.test_minimum_specificity,
                maximum_false_positive_rate=maximum_fpr,
                selection_familywise=phase == "selection",
            )
            typo_gate = binary_gate_breakdown(
                typos,
                minimum_precision=(
                    config.threshold_precision_floor
                    if phase == "selection"
                    else config.test_minimum_precision
                ),
                minimum_recall=(
                    (
                        config.selection_minimum_pause_typo_recall
                        if trigger == "pause"
                        else config.selection_minimum_typo_recall
                    )
                    if phase == "selection"
                    else (
                        config.test_minimum_pause_typo_recall
                        if trigger == "pause"
                        else config.test_minimum_typo_recall
                    )
                ),
                minimum_specificity=config.test_minimum_specificity,
                maximum_false_positive_rate=maximum_fpr,
                selection_familywise=phase == "selection",
            )
            per_trigger[trigger] = {
                "passed": overall_gate["passed"] is True
                and typo_gate["passed"] is True,
                "overall": overall_gate,
                "typos": typo_gate,
            }
        profile_passed = triggers_complete and all(
            isinstance(item, dict) and item.get("passed") is True
            for item in per_trigger.values()
        )
        profiles[profile.name] = {
            "passed": profile_passed,
            "delta": profile.delta,
            "group_selector": profile.group_selector,
            "all_triggers_present": triggers_complete,
            "per_trigger": per_trigger,
        }
    return {
        "passed": profiles_complete
        and all(
            isinstance(item, dict) and item.get("passed") is True
            for item in profiles.values()
        ),
        "all_profiles_present": profiles_complete,
        "expected_profiles": sorted(expected_profiles),
        "profiles": profiles,
    }


def selection_false_positive_budget_breakdown(
    overall: ConfusionMatrix,
    typos: ConfusionMatrix,
    maximum_false_positives: int,
) -> dict[str, object]:
    """Audit an absolute selection-tail budget independently of rates.

    A zero budget is deliberately stronger than the statistical upper-bound
    gate: the threshold must sit above every observed negative in both the
    complete and typo-only selection slices.  The sealed test keeps its
    predeclared Wilson-rate policy and is never used to choose this threshold.
    """

    if isinstance(maximum_false_positives, bool) or maximum_false_positives < 0:
        raise ValueError("maximum_false_positives must be a non-negative integer")
    checks = {
        "overall": overall.false_positive <= maximum_false_positives,
        "typos": typos.false_positive <= maximum_false_positives,
    }
    return {
        "passed": all(checks.values()),
        "checks": checks,
        "actual": {
            "overall_false_positives": overall.false_positive,
            "typo_false_positives": typos.false_positive,
        },
        "maximum_false_positives_per_trigger": maximum_false_positives,
    }


def threshold_selection_gate_breakdown(
    config: TrainingConfig,
    selections: Mapping[CorrectionTrigger, ThresholdSelection],
    context_metrics: Mapping[
        str, Mapping[CorrectionTrigger, ConfusionMatrix]
    ]
    | None = None,
    context_typo_metrics: Mapping[
        str, Mapping[CorrectionTrigger, ConfusionMatrix]
    ]
    | None = None,
) -> dict[str, object]:
    """Reject an infeasible candidate before the sealed test is materialised."""

    complete = set(selections) == set(MODEL_TRIGGERS)
    per_trigger: dict[str, object] = {}
    for trigger in MODEL_TRIGGERS:
        selection = selections.get(trigger)
        if selection is None:
            per_trigger[trigger] = {"passed": False, "missing": True}
            continue
        maximum_fpr = (
            config.pause_threshold_max_false_positive_rate
            if trigger == "pause"
            else config.threshold_max_false_positive_rate
        )
        overall = binary_gate_breakdown(
            selection.metrics,
            minimum_precision=config.threshold_precision_floor,
            minimum_recall=(
                config.selection_minimum_pause_recall
                if trigger == "pause"
                else config.selection_minimum_recall
            ),
            minimum_specificity=config.test_minimum_specificity,
            maximum_false_positive_rate=maximum_fpr,
            selection_familywise=True,
        )
        typos = binary_gate_breakdown(
            selection.typo_metrics,
            minimum_precision=config.threshold_precision_floor,
            minimum_recall=(
                config.selection_minimum_pause_typo_recall
                if trigger == "pause"
                else config.selection_minimum_typo_recall
            ),
            minimum_specificity=config.test_minimum_specificity,
            maximum_false_positive_rate=maximum_fpr,
            selection_familywise=True,
        )
        false_positive_budget = selection_false_positive_budget_breakdown(
            selection.metrics,
            selection.typo_metrics,
            config.selection_maximum_false_positives_per_trigger,
        )
        per_trigger[trigger] = {
            "passed": overall["passed"] is True
            and typos["passed"] is True
            and false_positive_budget["passed"] is True,
            "logits": selection.runtime_logits(),
            "overall": overall,
            "typos": typos,
            "false_positive_budget": false_positive_budget,
        }
    neutral_passed = complete and all(
        isinstance(item, dict) and item.get("passed") is True
        for item in per_trigger.values()
    )
    context_stress = context_stress_gate_breakdown(
        config,
        context_metrics,
        context_typo_metrics,
        phase="selection",
    )
    return {
        "passed": neutral_passed and context_stress["passed"] is True,
        "all_triggers_present": complete,
        "per_trigger": per_trigger,
        "neutral": {
            "passed": neutral_passed,
            "all_triggers_present": complete,
            "per_trigger": per_trigger,
        },
        "context_stress": context_stress,
    }


def _diagnostic_length_bucket(example: LexicalExample) -> str:
    length = max(len(example.original), len(example.alternative))
    if length <= 4:
        return "1-4"
    if length <= 7:
        return "5-7"
    if length <= 11:
        return "8-11"
    if length <= 19:
        return "12-19"
    return "20+"


def _diagnostic_frequency_bucket(frequency: int) -> str:
    if frequency <= 0:
        return "0"
    if frequency < 10:
        return "1-9"
    if frequency < 100:
        return "10-99"
    if frequency < 1_000:
        return "100-999"
    return "1000+"


def _selection_metric_slices(
    examples: Sequence[ScoredExample],
    threshold: float | Mapping[LayoutDirection, float],
) -> dict[str, object]:
    grouped: dict[str, dict[str, list[ScoredExample]]] = {
        "direction": defaultdict(list),
        "intended_group": defaultdict(list),
        "variant_kind": defaultdict(list),
        "length": defaultdict(list),
        "frequency": defaultdict(list),
    }
    for item in examples:
        example = item.example
        intended_group = (
            example.target_group if example.label else example.source_group
        )
        categories = {
            "direction": f"{example.source_group}>{example.target_group}",
            "intended_group": str(intended_group),
            "variant_kind": example.variant_kind,
            "length": _diagnostic_length_bucket(example),
            "frequency": _diagnostic_frequency_bucket(example.frequency),
        }
        for dimension, category in categories.items():
            grouped[dimension][category].append(item)
    return {
        dimension: {
            category: metrics_payload(
                confusion_at_threshold(category_examples, threshold)
                if isinstance(threshold, float)
                else confusion_at_directional_threshold(
                    category_examples,
                    threshold,
                )
            )
            for category, category_examples in sorted(categories.items())
        }
        for dimension, categories in grouped.items()
    }


def _recall_target_threshold(
    examples: Sequence[ScoredExample],
    target: float,
    *,
    typos_only: bool,
) -> float:
    if not 0.0 < target <= 1.0:
        raise ValueError("diagnostic recall target must be in (0, 1]")
    positive_scores = sorted(
        (
            item.calibrated_logit
            for item in examples
            if item.example.label
            and (
                not typos_only
                or item.example.variant_kind in TYPO_VARIANT_KINDS
            )
        ),
        reverse=True,
    )
    if not positive_scores:
        label = "typo-positive" if typos_only else "positive"
        raise ValueError(f"selection diagnostics require {label} examples")
    required = min(
        len(positive_scores),
        max(1, math.ceil(target * len(positive_scores))),
    )
    return positive_scores[required - 1]


def _selection_point_payload(
    examples: Sequence[ScoredExample],
    threshold: float,
    trigger: CorrectionTrigger,
    config: TrainingConfig,
) -> dict[str, object]:
    overall = confusion_at_threshold(examples, threshold)
    typos = confusion_at_threshold(
        (
            item
            for item in examples
            if item.example.variant_kind in TYPO_VARIANT_KINDS
        ),
        threshold,
    )
    maximum_fpr = (
        config.pause_threshold_max_false_positive_rate
        if trigger == "pause"
        else config.threshold_max_false_positive_rate
    )
    return {
        "logit": threshold,
        "overall": binary_gate_breakdown(
            overall,
            minimum_precision=config.threshold_precision_floor,
            minimum_recall=(
                config.selection_minimum_pause_recall
                if trigger == "pause"
                else config.selection_minimum_recall
            ),
            minimum_specificity=config.test_minimum_specificity,
            maximum_false_positive_rate=maximum_fpr,
            selection_familywise=True,
        ),
        "typos": binary_gate_breakdown(
            typos,
            minimum_precision=config.threshold_precision_floor,
            minimum_recall=(
                config.selection_minimum_pause_typo_recall
                if trigger == "pause"
                else config.selection_minimum_typo_recall
            ),
            minimum_specificity=config.test_minimum_specificity,
            maximum_false_positive_rate=maximum_fpr,
            selection_familywise=True,
        ),
    }


def selection_tail_diagnostics(
    scores: Sequence[ScoredExample],
    selections: Mapping[CorrectionTrigger, ThresholdSelection],
    config: TrainingConfig,
) -> dict[str, object]:
    """Explain a pre-sealed tail failure without exposing lexical tokens."""

    if set(selections) != set(MODEL_TRIGGERS):
        raise ValueError("selection diagnostics require every trigger")
    per_trigger: dict[str, object] = {}
    for trigger in MODEL_TRIGGERS:
        selected = tuple(
            item for item in scores if item.example.trigger == trigger
        )
        if not selected:
            raise ValueError(
                f"selection diagnostics require examples for {trigger}"
            )
        ordinary_target = (
            config.selection_minimum_pause_recall
            if trigger == "pause"
            else config.selection_minimum_recall
        )
        typo_target = (
            config.selection_minimum_pause_typo_recall
            if trigger == "pause"
            else config.selection_minimum_typo_recall
        )
        ordinary_target_threshold = _recall_target_threshold(
            selected,
            ordinary_target,
            typos_only=False,
        )
        typo_target_threshold = _recall_target_threshold(
            selected,
            typo_target,
            typos_only=True,
        )
        selection = selections[trigger]
        per_trigger[trigger] = {
            "selected_logits": selection.runtime_logits(),
            "selected_metric_slices": _selection_metric_slices(
                selected,
                selection.direction_logits,
            ),
            "ordinary_recall_target": ordinary_target,
            "ordinary_target_point": _selection_point_payload(
                selected,
                ordinary_target_threshold,
                trigger,
                config,
            ),
            "typo_recall_target": typo_target,
            "typo_target_point": _selection_point_payload(
                selected,
                typo_target_threshold,
                trigger,
                config,
            ),
        }
    return {
        "schema_version": 1,
        "source_split": "threshold",
        "contains_lexical_tokens": False,
        "per_trigger": per_trigger,
    }


def presealed_candidate_gate_breakdown(
    config: TrainingConfig,
    selection_gate: Mapping[str, object],
    safety_audit: GuardedSafetyAudit,
    veto_selection: VetoSelection,
) -> dict[str, object]:
    """Reject every already-known release failure before opening sealed test."""

    selection_passed = selection_gate.get("passed") is True
    safety_passed = safety_audit.passes(config.safety_maximum_guard_failures)
    veto_passed = (
        veto_selection.positive_samples > 0
        and veto_selection.false_negative_rate
        <= config.veto_max_false_negative_rate
    )
    return {
        "passed": selection_passed and safety_passed and veto_passed,
        "threshold_selection": selection_gate,
        "safety": {
            "passed": safety_passed,
            "actual_guard_failures": len(safety_audit.failures),
            "maximum_guard_failures": config.safety_maximum_guard_failures,
        },
        "veto_selection": {
            "passed": veto_passed,
            **asdict(veto_selection),
            "maximum_false_negative_rate": config.veto_max_false_negative_rate,
        },
    }


def training_quality_gate_breakdown(
    config: TrainingConfig,
    test_metrics: Mapping[CorrectionTrigger, ConfusionMatrix],
    typo_metrics: Mapping[CorrectionTrigger, ConfusionMatrix],
    safety_audit: GuardedSafetyAudit,
    veto_selection: VetoSelection,
    test_veto: VetoSelection,
    context_metrics: Mapping[
        str, Mapping[CorrectionTrigger, ConfusionMatrix]
    ]
    | None = None,
    context_typo_metrics: Mapping[
        str, Mapping[CorrectionTrigger, ConfusionMatrix]
    ]
    | None = None,
) -> dict[str, object]:
    """Return exact pass/fail evidence for every release constraint."""

    complete = (
        set(test_metrics) == set(MODEL_TRIGGERS)
        and set(typo_metrics) == set(MODEL_TRIGGERS)
    )
    sealed: dict[str, object] = {}
    typos: dict[str, object] = {}
    for trigger in MODEL_TRIGGERS:
        maximum_fpr = (
            config.pause_threshold_max_false_positive_rate
            if trigger == "pause"
            else config.threshold_max_false_positive_rate
        )
        if trigger in test_metrics:
            sealed[trigger] = binary_gate_breakdown(
                test_metrics[trigger],
                minimum_precision=config.test_minimum_precision,
                minimum_recall=(
                    config.test_minimum_pause_recall
                    if trigger == "pause"
                    else config.test_minimum_recall
                ),
                minimum_specificity=config.test_minimum_specificity,
                maximum_false_positive_rate=maximum_fpr,
            )
        if trigger in typo_metrics:
            typos[trigger] = binary_gate_breakdown(
                typo_metrics[trigger],
                minimum_precision=config.test_minimum_precision,
                minimum_recall=(
                    config.test_minimum_pause_typo_recall
                    if trigger == "pause"
                    else config.test_minimum_typo_recall
                ),
                minimum_specificity=config.test_minimum_specificity,
                maximum_false_positive_rate=maximum_fpr,
            )
    safety_passed = safety_audit.passes(config.safety_maximum_guard_failures)
    selection_veto_passed = (
        veto_selection.positive_samples > 0
        and veto_selection.false_negative_rate
        <= config.veto_max_false_negative_rate
    )
    test_veto_passed = (
        test_veto.positive_samples > 0
        and test_veto.false_negative_rate <= config.veto_max_false_negative_rate
    )
    sealed_passed = complete and all(
        isinstance(item, dict) and item.get("passed") is True
        for item in sealed.values()
    )
    typo_passed = complete and all(
        isinstance(item, dict) and item.get("passed") is True
        for item in typos.values()
    )
    context_stress = context_stress_gate_breakdown(
        config,
        context_metrics,
        context_typo_metrics,
        phase="sealed_test",
    )
    return {
        "passed": (
            sealed_passed
            and typo_passed
            and context_stress["passed"] is True
            and safety_passed
            and selection_veto_passed
            and test_veto_passed
        ),
        "all_triggers_present": complete,
        "sealed_test": {"passed": sealed_passed, "per_trigger": sealed},
        "sealed_test_typos": {"passed": typo_passed, "per_trigger": typos},
        "sealed_test_context_stress": context_stress,
        "safety": {
            "passed": safety_passed,
            "actual_guard_failures": len(safety_audit.failures),
            "maximum_guard_failures": config.safety_maximum_guard_failures,
        },
        "veto": {
            "passed": selection_veto_passed and test_veto_passed,
            "selection": {
                "passed": selection_veto_passed,
                **asdict(veto_selection),
            },
            "sealed_test": {
                "passed": test_veto_passed,
                **asdict(test_veto),
            },
            "maximum_false_negative_rate": config.veto_max_false_negative_rate,
        },
    }


def training_quality_gates_pass(
    config: TrainingConfig,
    test_metrics: Mapping[CorrectionTrigger, ConfusionMatrix],
    typo_metrics: Mapping[CorrectionTrigger, ConfusionMatrix],
    safety_audit: GuardedSafetyAudit,
    veto_selection: VetoSelection,
    test_veto: VetoSelection,
    context_metrics: Mapping[
        str, Mapping[CorrectionTrigger, ConfusionMatrix]
    ]
    | None = None,
    context_typo_metrics: Mapping[
        str, Mapping[CorrectionTrigger, ConfusionMatrix]
    ]
    | None = None,
) -> bool:
    """Evaluate non-vacuous statistical gates for every trigger."""
    return bool(
        training_quality_gate_breakdown(
            config,
            test_metrics,
            typo_metrics,
            safety_audit,
            veto_selection,
            test_veto,
            context_metrics,
            context_typo_metrics,
        )["passed"]
    )


def validate_word_scorers(scorers: Mapping[int, WordScorer]) -> None:
    if set(scorers) != {0, 1}:
        raise ValueError("training word scorers must contain exactly groups 0 and 1")


def training_word_score(
    word: str,
    group: int,
    *,
    scorers: Mapping[int, WordScorer],
) -> WordScore:
    """Score observable text only; dataset labels and annotations are absent."""

    if group not in (0, 1):
        raise ValueError("training word scorer supports only groups 0 and 1")
    try:
        scorer = scorers[group]
    except KeyError as error:
        raise ValueError(f"missing training word scorer for group {group}") from error
    return scorer.score(word)


def runtime_feature_extractor(
    hash_seed: int,
    membership_seed: int = DEFAULT_MEMBERSHIP_FNV_SEED,
    *,
    scorers: Mapping[int, WordScorer],
) -> ExampleFeatureExtractor:
    """Adapt lexical rows to the exact feature function used in production."""

    validate_word_scorers(scorers)

    def extractor(
        example: LexicalExample, dimension: int
    ) -> ExtractedExampleFeatures:
        evidence = IntentModelInput(
            original=example.original,
            alternative=example.alternative,
            source_group=example.source_group,
            target_group=example.target_group,
            trigger=example.trigger,
            source_score=_IGNORED_CLASSIFIER_WORD_SCORE,
            target_score=_IGNORED_CLASSIFIER_WORD_SCORE,
            context_delta=example.context_delta,
            context_group=example.context_group,
        )
        vector = extract_features(
            evidence,
            dimension=dimension,
            hash_seed=hash_seed,
            membership_seed=membership_seed,
            ngram_orders=NGRAM_ORDERS,
        )
        return ExtractedExampleFeatures(
            vector.values, vector.character_fingerprints
        )

    return extractor


def intent_input_for_example(
    example: LexicalExample,
    *,
    scorers: Mapping[int, WordScorer],
) -> IntentModelInput:
    return IntentModelInput(
        original=example.original,
        alternative=example.alternative,
        source_group=example.source_group,
        target_group=example.target_group,
        trigger=example.trigger,
        source_score=training_word_score(
            example.original,
            example.source_group,
            scorers=scorers,
        ),
        target_score=training_word_score(
            example.alternative,
            example.target_group,
            scorers=scorers,
        ),
        context_delta=example.context_delta,
        context_group=example.context_group,
    )


def _parse_arguments(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--config",
        type=Path,
        default=PROJECT_ROOT / "model/intent_v1/config.json",
    )
    parser.add_argument(
        "--en-model",
        type=Path,
        default=PROJECT_ROOT / "model/intent_v1/sources/en_US.lm",
    )
    parser.add_argument(
        "--ru-model",
        type=Path,
        default=PROJECT_ROOT / "model/intent_v1/sources/ru_RU.lm",
    )
    parser.add_argument(
        "--artifact",
        type=Path,
        default=(
            PROJECT_ROOT
            / "src/keyswitch/resources/models/layout_intent_v1.ksm"
        ),
    )
    parser.add_argument(
        "--manifest",
        type=Path,
        default=PROJECT_ROOT / "model/intent_v1/manifest.json",
    )
    parser.add_argument(
        "--test-report",
        type=Path,
        default=PROJECT_ROOT / "model/intent_v1/test-report.json",
    )
    parser.add_argument(
        "--diagnostic-output",
        type=Path,
        default=None,
        help=(
            "atomically write the full pre-sealed failure diagnostic to a "
            "dedicated path without publishing an artifact or consuming the seal"
        ),
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help=(
            "train and report without publishing artifacts; a passed "
            "complete pre-sealed gate still consumes the sealed-evaluation "
            "ledger"
        ),
    )
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    """Train, calibrate, select thresholds, seal test metrics and write artifacts."""

    arguments = _parse_arguments(argv)
    config, config_sha256 = load_training_config_snapshot(arguments.config)
    toolchain_snapshot = capture_toolchain_snapshot(config_sha256)
    evidence_path = verify_training_sources(
        config, arguments.en_model, arguments.ru_model
    )
    hard_negative_path = (
        PROJECT_ROOT / config.hard_negative_development.source.path
    )
    validate_training_paths(
        config=arguments.config,
        english=arguments.en_model,
        russian=arguments.ru_model,
        license_evidence=evidence_path,
        hard_negative_source=hard_negative_path,
        seal_registry=sealed_registry_path(config),
        artifact=arguments.artifact,
        manifest=arguments.manifest,
        report=arguments.test_report,
        diagnostic=arguments.diagnostic_output,
    )
    english_bytes = read_verified_frozen_file(
        arguments.en_model,
        config.sources.english,
        label="English training source",
    )
    russian_bytes = read_verified_frozen_file(
        arguments.ru_model,
        config.sources.russian,
        label="Russian training source",
    )
    hard_negative_corpus = load_hard_negative_development_corpus(
        hard_negative_path,
        config,
    )
    english, english_source = load_onboard_unigrams(
        arguments.en_model,
        "en_US",
        config.sources.english.group,
        config,
        license_declaration=config.sources.license_declaration,
        license_evidence=config.sources.license_evidence.path,
        logical_path=config.sources.english.path,
        source_bytes=english_bytes,
        minimum_word_length=SAFETY_COLLISION_MINIMUM_WORD_LENGTH,
    )
    russian, russian_source = load_onboard_unigrams(
        arguments.ru_model,
        "ru_RU",
        config.sources.russian.group,
        config,
        license_declaration=config.sources.license_declaration,
        license_evidence=config.sources.license_evidence.path,
        logical_path=config.sources.russian.path,
        source_bytes=russian_bytes,
        minimum_word_length=SAFETY_COLLISION_MINIMUM_WORD_LENGTH,
    )
    del english_bytes
    del russian_bytes
    prepared = prepare_lexicon(
        (*english, *russian),
        minimum_training_signature_length=config.minimum_word_length,
    )
    english = tuple(
        word
        for word in english
        if len(word.physical_signature) >= config.minimum_word_length
    )
    russian = tuple(
        word
        for word in russian
        if len(word.physical_signature) >= config.minimum_word_length
    )
    dataset = build_dataset(
        prepared, config, included_splits=PRESEALED_SPLITS
    )
    dataset = merge_hard_negative_development(
        dataset,
        hard_negative_corpus,
    )
    assert_no_split_leakage(dataset)
    candidate_dataset_sha256 = dataset_fingerprint(dataset)
    training_language_scorers = TrainOnlyLanguageScorers.from_training_partition(
        prepared,
        dataset.variant_quarantine,
    )
    safety_audit = audit_guarded_safety_corpus(dataset.safety)
    extractor = runtime_feature_extractor(
        config.feature_hash_seed,
        config.membership_hash_seed,
        scorers=training_language_scorers.scorers,
    )
    training_fingerprint_union: set[int] = set()
    training_featured = featurize_examples(
        dataset.by_split["train"],
        config.dimension,
        extractor,
        supported_fingerprints=training_fingerprint_union,
    )
    development_featured = featurize_examples(
        dataset.by_split["development"], config.dimension, extractor
    )
    training_result = fit_ftrl(
        training_featured, development_featured, config
    )
    final_sparse_weights = training_result.model.sparse_weights()
    quantized = quantize_weights(
        final_sparse_weights, config.dimension
    )
    quantized_scorer = QuantizedLinearScorer(
        quantized, training_result.model.bias
    )
    supported_fingerprints = frozenset(training_fingerprint_union)
    del training_fingerprint_union
    del training_featured
    del development_featured
    calibration_featured = featurize_examples(
        dataset.by_split["calibration"], config.dimension, extractor
    )
    calibration_pairs = tuple(
        (
            quantized_scorer.score(item.features),
            item.example.label,
            layout_direction(
                item.example.source_group,
                item.example.target_group,
            ),
        )
        for item in calibration_featured
    )
    calibration = fit_directional_platt_calibration(
        calibration_pairs,
        l2=config.calibration_l2,
        maximum_iterations=config.calibration_max_iterations,
    )
    if any(
        calibration.for_direction(direction).slope <= 0.0
        for direction in LAYOUT_DIRECTIONS
    ):
        raise RuntimeError("calibration reversed the model score orientation")
    calibration_scores = score_examples(
        quantized_scorer, calibration, calibration_featured
    )
    veto_selection = choose_veto_threshold(
        calibration_scores,
        quantile=config.veto_positive_quantile,
        margin=config.veto_logit_margin,
    )
    del calibration_featured
    threshold_featured = featurize_examples(
        dataset.by_split["threshold"], config.dimension, extractor
    )
    threshold_scores = score_examples(
        quantized_scorer, calibration, threshold_featured
    )
    thresholds = choose_trigger_thresholds(threshold_scores, config)
    (
        threshold_context_metrics,
        threshold_context_typo_metrics,
    ) = score_context_stress_profiles(
        dataset.by_split["threshold"],
        dimension=config.dimension,
        extractor=extractor,
        model=quantized_scorer,
        calibration=calibration,
        thresholds=thresholds,
        neutral_scores=threshold_scores,
    )
    policy = gate_policy_payload(config)
    selection_gate_breakdown = threshold_selection_gate_breakdown(
        config,
        thresholds,
        threshold_context_metrics,
        threshold_context_typo_metrics,
    )
    presealed_gate_breakdown = presealed_candidate_gate_breakdown(
        config,
        selection_gate_breakdown,
        safety_audit,
        veto_selection,
    )
    thresholds_payload: dict[str, object] = {
        trigger: {
            "global_logit_margin": selection.global_logit_margin,
            "logits": selection.runtime_logits(),
            "confidences": {
                direction: stable_sigmoid(selection.logit_for(direction))
                for direction in LAYOUT_DIRECTIONS
            },
            "selection_metrics": metrics_payload(selection.metrics),
            "selection_typo_metrics": metrics_payload(
                selection.typo_metrics
            ),
        }
        for trigger, selection in thresholds.items()
    }
    del threshold_featured
    if presealed_gate_breakdown["passed"] is not True:
        verify_training_sources(
            config, arguments.en_model, arguments.ru_model
        )
        verify_toolchain_snapshot(toolchain_snapshot, arguments.config)
        diagnostic = {
            "schema_version": 1,
            "model_id": "keyswitch-layout-intent-v1",
            "phase": "presealed_candidate",
            "quality_gates_passed": False,
            "config_sha256": toolchain_snapshot.config_sha256,
            "toolchain": asdict(toolchain_snapshot),
            "candidate_dataset_sha256": candidate_dataset_sha256,
            "split_namespace": SPLIT_NAMESPACE,
            "variant_quarantine_sha256": (
                dataset.variant_quarantine.sha256
            ),
            "training_language_scorer": (
                training_language_scorers.provenance_payload()
            ),
            "training": {
                "seed": config.seed,
                "best_epoch": training_result.best_epoch,
                "history": [
                    asdict(item) for item in training_result.history
                ],
                "nonzero_weights": len(final_sparse_weights),
                "bias": training_result.model.bias,
                "supported_character_fingerprints": len(
                    supported_fingerprints
                ),
                "hard_negative_development": (
                    hard_negative_corpus.provenance_payload()
                ),
            },
            "quantization": {
                "format": "signed-int16",
                "scale": quantized.scale,
                "maximum_absolute_error": (
                    quantized.maximum_absolute_error
                ),
                "quantized_nonzero_weights": sum(
                    byte.bit_count() for byte in quantized.support
                ),
            },
            "calibration": calibration.payload(),
            "veto": {"selection": asdict(veto_selection)},
            "gate_policy": policy,
            "thresholds": thresholds_payload,
            "threshold_selection_gate_breakdown": (
                selection_gate_breakdown
            ),
            "selection_tail_diagnostics": selection_tail_diagnostics(
                threshold_scores,
                thresholds,
                config,
            ),
            "presealed_candidate_gate_breakdown": (
                presealed_gate_breakdown
            ),
            "sealed_test_evaluated": False,
            "safety": {"guard_audit": asdict(safety_audit)},
        }
        diagnostic_bytes = (
            json.dumps(
                diagnostic, ensure_ascii=False, indent=2, sort_keys=True
            )
            + "\n"
        ).encode("utf-8")
        if arguments.diagnostic_output is not None:
            _atomic_write_bytes(arguments.diagnostic_output, diagnostic_bytes)
        print(
            diagnostic_bytes.decode("utf-8"),
            end="",
            flush=True,
        )
        return 1
    verify_training_sources(config, arguments.en_model, arguments.ru_model)
    verify_toolchain_snapshot(toolchain_snapshot, arguments.config)
    scorer_provenance = training_language_scorers.provenance_payload()
    source_rows = [asdict(english_source), asdict(russian_source)]
    source_package_payload = {
        "name": config.sources.package,
        "version": config.sources.package_version,
        "license_declaration": config.sources.license_declaration,
        "license_evidence": asdict(config.sources.license_evidence),
        "license_evidence_verified_path": config.sources.license_evidence.path,
    }
    training_payload = {
        "seed": config.seed,
        "best_epoch": training_result.best_epoch,
        "history": [asdict(item) for item in training_result.history],
        "nonzero_weights": len(final_sparse_weights),
        "bias": training_result.model.bias,
        "supported_character_fingerprints": len(supported_fingerprints),
        "hard_negative_development": (
            hard_negative_corpus.provenance_payload()
        ),
    }
    quantization_payload = {
        "format": "signed-int16",
        "scale": quantized.scale,
        "maximum_absolute_error": quantized.maximum_absolute_error,
        "quantized_nonzero_weights": sum(
            byte.bit_count() for byte in quantized.support
        ),
    }
    calibration_payload = calibration.payload()
    veto_selection_payload = asdict(veto_selection)
    candidate_model_parameters = training_candidate_model_parameters(
        config=config,
        quantized=quantized,
        supported_fingerprints=supported_fingerprints,
        bias=training_result.model.bias,
        calibration=calibration,
        thresholds=thresholds,
        veto=veto_selection,
    )
    validate_presealed_candidate_serialization(
        config=config,
        quantized=quantized,
        supported_fingerprints=supported_fingerprints,
        bias=training_result.model.bias,
        calibration=calibration,
        thresholds=thresholds,
        veto=veto_selection,
        expected_parameters=candidate_model_parameters,
    )
    candidate_metadata = presealed_candidate_metadata_projection(
        model_id="keyswitch-layout-intent-v1",
        calibration_scope=calibration.provenance,
        config_sha256=toolchain_snapshot.config_sha256,
        split_namespace=config.sealed_evaluation.split_namespace,
        toolchain=asdict(toolchain_snapshot),
        source_package=source_package_payload,
        sources=source_rows,
        candidate_counts=presealed_candidate_counts(dataset),
        variant_quarantine_sha256=dataset.variant_quarantine.sha256,
        training_language_scorer=scorer_provenance,
        gate_policy=policy,
        training=training_payload,
        quantization=quantization_payload,
        calibration=calibration_payload,
        veto_selection=veto_selection_payload,
        thresholds=thresholds_payload,
        selection_gate_breakdown=selection_gate_breakdown,
        safety_guard_audit=asdict(safety_audit),
        model_parameters=candidate_model_parameters,
    )
    candidate_sha256 = sealed_candidate_sha256(
        split_namespace=config.sealed_evaluation.split_namespace,
        config_sha256=toolchain_snapshot.config_sha256,
        candidate_dataset_sha256=candidate_dataset_sha256,
        toolchain=asdict(toolchain_snapshot),
        training_language_scorer=scorer_provenance,
        model_parameters=candidate_model_parameters,
        selection_gate_breakdown=selection_gate_breakdown,
        candidate_metadata=candidate_metadata,
    )
    sealed_receipt = claim_sealed_evaluation(
        config=config,
        candidate_sha256=candidate_sha256,
        config_sha256=toolchain_snapshot.config_sha256,
        candidate_dataset_sha256=candidate_dataset_sha256,
    )
    verify_sealed_evaluation_receipt(sealed_receipt)
    sealed_phase_dataset = build_dataset(
        prepared, config, included_splits=SEALED_TEST_SPLITS
    )
    assert_no_split_leakage(
        sealed_phase_dataset,
        variant_quarantine_splits=SEALED_TEST_SPLITS,
    )
    dataset = merge_sealed_test_dataset(dataset, sealed_phase_dataset)
    assert_no_split_leakage(dataset)
    dataset_sha256 = dataset_fingerprint(dataset)
    safety_audit = audit_guarded_safety_corpus(dataset.safety)
    test_featured = featurize_examples(
        dataset.by_split["test"], config.dimension, extractor
    )
    test_scores = score_examples(
        quantized_scorer, calibration, test_featured
    )
    del test_featured
    safety_featured = featurize_examples(
        dataset.safety, config.dimension, extractor
    )
    safety_scores = score_examples(quantized_scorer, calibration, safety_featured)
    test_veto = veto_metrics(test_scores, veto_selection.raw_logit)
    test_metrics = {
        trigger: confusion_at_directional_threshold(
            (item for item in test_scores if item.example.trigger == trigger),
            thresholds[trigger].direction_logits,
        )
        for trigger in MODEL_TRIGGERS
    }
    typo_test_metrics = {
        trigger: confusion_at_directional_threshold(
            (
                item
                for item in test_scores
                if item.example.trigger == trigger
                and item.example.variant_kind in TYPO_VARIANT_KINDS
            ),
            thresholds[trigger].direction_logits,
        )
        for trigger in MODEL_TRIGGERS
    }
    (
        context_test_metrics,
        context_test_typo_metrics,
    ) = score_context_stress_profiles(
        dataset.by_split["test"],
        dimension=config.dimension,
        extractor=extractor,
        model=quantized_scorer,
        calibration=calibration,
        thresholds=thresholds,
        neutral_scores=test_scores,
    )
    safety_metrics = {
        trigger: confusion_at_directional_threshold(
            (item for item in safety_scores if item.example.trigger == trigger),
            thresholds[trigger].direction_logits,
        )
        for trigger in MODEL_TRIGGERS
    }
    verify_training_sources(config, arguments.en_model, arguments.ru_model)
    verify_toolchain_snapshot(toolchain_snapshot, arguments.config)
    verify_sealed_evaluation_receipt(sealed_receipt)
    counts = {
        split: len(dataset.by_split[split]) for split in SPLIT_NAMES
    }
    manifest: dict[str, object] = {
        "schema_version": 1,
        "model_id": "keyswitch-layout-intent-v1",
        "calibration_scope": calibration.provenance,
        "config_sha256": toolchain_snapshot.config_sha256,
        "toolchain": asdict(toolchain_snapshot),
        "dataset_sha256": dataset_sha256,
        "split_namespace": SPLIT_NAMESPACE,
        "sealed_evaluation": sealed_receipt.payload(),
        "source_package": source_package_payload,
        "sources": source_rows,
        "counts": {
            "lexicon_words": sum(len(items) for items in prepared.words_by_split.values()),
            "collisions": len(prepared.collisions),
            "examples": counts,
            "safety_examples": len(dataset.safety),
            "quarantined_variant_occurrences": (
                dataset.variant_quarantine.occurrence_count
            ),
            "quarantined_physical_signatures": (
                dataset.variant_quarantine.physical_signature_count
            ),
            "sealed_quarantined_variant_occurrences": (
                dataset.sealed_variant_quarantine.occurrence_count
            ),
            "sealed_quarantined_physical_signatures": (
                dataset.sealed_variant_quarantine.physical_signature_count
            ),
            "sealed_test_exclusion_signatures": len(
                dataset.sealed_test_exclusion_signatures
            ),
        },
        "variant_quarantine_sha256": dataset.variant_quarantine.sha256,
        "sealed_variant_quarantine_sha256": (
            dataset.sealed_variant_quarantine.sha256
        ),
        "sealed_test_exclusion_signatures_sha256": hashlib.sha256(
            "\n".join(dataset.sealed_test_exclusion_signatures).encode("utf-8")
        ).hexdigest(),
        "training_language_scorer": (
            scorer_provenance
        ),
        "gate_policy": policy,
        "training": training_payload,
        "quantization": quantization_payload,
        "calibration": calibration_payload,
        "veto": {
            "selection": veto_selection_payload,
            "sealed_test": asdict(test_veto),
        },
        "thresholds": thresholds_payload,
        "threshold_selection_gate_breakdown": selection_gate_breakdown,
        "sealed_test": {
            trigger: metrics_payload(metrics)
            for trigger, metrics in test_metrics.items()
        },
        "sealed_test_typos": {
            trigger: metrics_payload(metrics)
            for trigger, metrics in typo_test_metrics.items()
        },
        "sealed_test_context_stress": {
            context_name: {
                "overall": {
                    trigger: metrics_payload(metrics)
                    for trigger, metrics in context_metrics.items()
                },
                "typos": {
                    trigger: metrics_payload(metrics)
                    for trigger, metrics in context_test_typo_metrics[
                        context_name
                    ].items()
                },
            }
            for context_name, context_metrics in context_test_metrics.items()
        },
        "safety": {
            "guard_audit": asdict(safety_audit),
            "raw_model_diagnostics": {
                trigger: metrics_payload(metrics)
                for trigger, metrics in safety_metrics.items()
            },
        },
    }
    build_provenance = {
        "config_sha256": manifest["config_sha256"],
        "dataset_sha256": manifest["dataset_sha256"],
        "split_namespace": manifest["split_namespace"],
        "sealed_evaluation": manifest["sealed_evaluation"],
        "gate_policy": manifest["gate_policy"],
        "source_package": manifest["source_package"],
        "sources": manifest["sources"],
        "toolchain": manifest["toolchain"],
        "variant_quarantine_sha256": manifest[
            "variant_quarantine_sha256"
        ],
        "training_language_scorer": manifest[
            "training_language_scorer"
        ],
    }
    build_provenance_sha256 = hashlib.sha256(
        json.dumps(
            build_provenance,
            ensure_ascii=False,
            sort_keys=True,
            separators=(",", ":"),
        ).encode("utf-8")
    ).hexdigest()
    manifest["build_provenance_sha256"] = build_provenance_sha256
    quality_gate_breakdown = training_quality_gate_breakdown(
        config,
        test_metrics,
        typo_test_metrics,
        safety_audit,
        veto_selection,
        test_veto,
        context_test_metrics,
        context_test_typo_metrics,
    )
    gates_pass = quality_gate_breakdown["passed"] is True
    manifest["quality_gate_breakdown"] = quality_gate_breakdown
    manifest["quality_gates_passed"] = gates_pass
    report = {
        "model_id": manifest["model_id"],
        "calibration_scope": calibration.provenance,
        "config_sha256": manifest["config_sha256"],
        "build_provenance_sha256": build_provenance_sha256,
        "split_namespace": manifest["split_namespace"],
        "sealed_evaluation": manifest["sealed_evaluation"],
        "gate_policy": policy,
        "thresholds": manifest["thresholds"],
        "threshold_selection_gate_breakdown": selection_gate_breakdown,
        "sealed_test": manifest["sealed_test"],
        "sealed_test_typos": manifest["sealed_test_typos"],
        "sealed_test_context_stress": manifest[
            "sealed_test_context_stress"
        ],
        "safety": manifest["safety"],
        "veto": manifest["veto"],
        "quality_gate_breakdown": quality_gate_breakdown,
        "quality_gates_passed": gates_pass,
    }
    if not arguments.dry_run and gates_pass:
        arguments.artifact.parent.mkdir(parents=True, exist_ok=True)
        arguments.manifest.parent.mkdir(parents=True, exist_ok=True)
        arguments.test_report.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(
            prefix=".intent-model-build-", dir=arguments.artifact.parent
        ) as temporary:
            staged_artifact = Path(temporary) / arguments.artifact.name
            model = write_model(
                staged_artifact,
                model_version=(
                    "intent-v1-" + build_provenance_sha256[:12]
                ),
                dimension=config.dimension,
                weights=quantized.dequantized(),
                supported_fingerprints=supported_fingerprints,
                threshold_logits={
                    trigger: item.runtime_logits()
                    for trigger, item in thresholds.items()
                },
                veto_threshold=veto_selection.raw_logit,
                bias=training_result.model.bias,
                platt_calibration=calibration.runtime_parameters(),
                fnv_seed=config.feature_hash_seed,
                membership_seed=config.membership_hash_seed,
                ngram_orders=NGRAM_ORDERS,
                metadata=json_native_mapping(manifest),
            )
            if runtime_candidate_model_parameters(model) != (
                candidate_model_parameters
            ):
                raise RuntimeError(
                    "serialized KSLM parameters differ from the sealed candidate"
                )
            artifact_bytes = staged_artifact.read_bytes()
            manifest["artifact_sha256"] = hashlib.sha256(
                artifact_bytes
            ).hexdigest()
            manifest["artifact_model_version"] = model.model_version
            report["artifact_sha256"] = manifest["artifact_sha256"]
            report["artifact_model_version"] = model.model_version
            manifest_bytes = (
                json.dumps(
                    manifest,
                    ensure_ascii=False,
                    indent=2,
                    sort_keys=True,
                )
                + "\n"
            ).encode("utf-8")
            report_bytes = (
                json.dumps(
                    report,
                    ensure_ascii=False,
                    indent=2,
                    sort_keys=True,
                )
                + "\n"
            ).encode("utf-8")
            verify_training_sources(
                config, arguments.en_model, arguments.ru_model
            )
            verify_toolchain_snapshot(
                toolchain_snapshot, arguments.config
            )
            verify_sealed_evaluation_receipt(sealed_receipt)
            publish_bytes_bundle(
                (
                    (arguments.test_report, report_bytes),
                    (arguments.artifact, artifact_bytes),
                    (arguments.manifest, manifest_bytes),
                )
            )
    print(json.dumps(manifest, ensure_ascii=False, indent=2, sort_keys=True))
    return 0 if gates_pass else 1


if __name__ == "__main__":
    sys.exit(main())
