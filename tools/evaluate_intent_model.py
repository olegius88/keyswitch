#!/usr/bin/env python3
"""Sealed quality and provenance evaluation for the shipped KSLM model."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import multiprocessing
import statistics
import sys
import time
from collections.abc import Callable, Collection, Iterable, Mapping, Sequence
from concurrent.futures import ProcessPoolExecutor
from dataclasses import asdict, dataclass, field, replace
from pathlib import Path
from typing import Final, Literal, TypeVar, cast

tools_path = str(Path(__file__).resolve().parent)
if tools_path not in sys.path:
    sys.path.insert(0, tools_path)

from keyswitch.detector import (
    CONTEXT_DELTA_MULTIPLIER,
    CONTEXT_SCORE_MAXIMUM,
    CONTEXT_SCORE_MINIMUM,
    CONTEXT_SOURCE_GROUP_PENALTY,
    CONTEXT_TARGET_GROUP_BONUS,
    LanguageDetector,
    LanguageScorer,
)
from keyswitch.intent_model import (
    LAYOUT_DIRECTIONS,
    MAX_CONTAINER_BYTES,
    IntentModelInput,
    LinearPrediction,
    LinearNgramModel,
    MINIMUM_RUNTIME_TOKEN_LENGTH,
    TRIGGERS,
    CorrectionTrigger,
)
from keyswitch.language_model import LanguageModel, WordScore
from keyswitch.layouts import LayoutPair
from keyswitch.spellcheck import HunspellDictionary

from train_intent_model import (
    CONTEXT_STRESS_PROFILES,
    resolve_training_workers,
    DEVELOPMENT_FREEZER_PATH,
    INTENT_RUNTIME_PATH,
    LANGUAGE_MODEL_RUNTIME_PATH,
    LAYOUTS_RUNTIME_PATH,
    PRESEAL_GENERATOR_PATH,
    PRESEAL_RECEIPT_PATH,
    PROJECT_ROOT,
    PRESEALED_SPLITS,
    SELECTION_FALSE_POSITIVE_COMPARISONS,
    SELECTION_PER_COMPARISON_CONFIDENCE,
    SELECTION_WILSON_Z_SCORE,
    SEALED_TEST_SPLITS,
    SAFETY_COLLISION_MINIMUM_WORD_LENGTH,
    SPLIT_NAMES,
    SPLIT_NAMESPACE,
    UNKNOWN_TYPO_DEVELOPMENT_CHOICE_NAMESPACE,
    UNKNOWN_TYPO_DEVELOPMENT_RANK_NAMESPACE,
    UNKNOWN_TYPO_HOLDOUT_CHOICE_NAMESPACE,
    UNKNOWN_TYPO_HOLDOUT_RANK_NAMESPACE,
    TYPO_VARIANT_KINDS,
    ConfusionMatrix,
    DatasetBundle,
    GuardedSafetyAudit,
    LexicalExample,
    PreparedLexicon,
    TrainOnlyLanguageScorers,
    TrainingConfig,
    ThresholdSelection,
    VetoSelection,
    WILSON_95_Z_SCORE,
    WILSON_INTERVAL_CONFIDENCE,
    WordScorer,
    audit_guarded_safety_corpus,
    assert_no_split_leakage,
    _balanced_ranges,
    build_dataset,
    context_stress_gate_breakdown,
    dataset_fingerprint,
    gate_policy_payload,
    intent_input_for_example,
    load_onboard_unigrams,
    load_hard_negative_development_corpus,
    load_training_config,
    metrics_payload,
    merge_sealed_test_dataset,
    merge_hard_negative_development,
    physical_signature,
    presealed_candidate_counts,
    presealed_candidate_metadata_projection,
    prepare_lexicon,
    runtime_feature_extractor,
    runtime_candidate_model_parameters,
    row_physical_signature,
    sealed_candidate_sha256,
    sealed_evaluation_evidence_is_valid,
    sha256_file,
    typo_variants,
    threshold_selection_gate_breakdown,
    training_quality_gate_breakdown,
    verify_context_feature_invariance,
    verify_training_sources,
    wilson_upper_bound,
)


_T = TypeVar("_T")
_TRAINER_PATH = Path(__file__).resolve().with_name("train_intent_model.py")
_EVALUATOR_PATH = Path(__file__).resolve()
_DETECTOR_PATH = INTENT_RUNTIME_PATH.with_name("detector.py")
_PROTECTED_TOKENS_PATH = _DETECTOR_PATH.parent / "resources/protected_tokens.txt"
_TOOLCHAIN_CODE_PATHS: tuple[tuple[str, Path], ...] = (
    ("trainer_sha256", _TRAINER_PATH),
    ("runtime_sha256", INTENT_RUNTIME_PATH),
    ("detector_sha256", _DETECTOR_PATH),
    ("protected_tokens_sha256", _PROTECTED_TOKENS_PATH),
    ("layouts_sha256", LAYOUTS_RUNTIME_PATH),
    ("language_model_sha256", LANGUAGE_MODEL_RUNTIME_PATH),
    ("evaluator_sha256", _EVALUATOR_PATH),
    ("preseal_generator_sha256", PRESEAL_GENERATOR_PATH),
    ("development_freezer_sha256", DEVELOPMENT_FREEZER_PATH),
    ("preseal_receipt_sha256", PRESEAL_RECEIPT_PATH),
)
_BUILD_PROVENANCE_FIELDS = (
    "config_sha256",
    "dataset_sha256",
    "split_namespace",
    "sealed_evaluation",
    "gate_policy",
    "source_package",
    "sources",
    "toolchain",
    "variant_quarantine_sha256",
    "training_language_scorer",
)
_MANIFEST_KEYS: Final[frozenset[str]] = frozenset(
    {
        "schema_version",
        "model_id",
        "calibration_scope",
        "config_sha256",
        "toolchain",
        "dataset_sha256",
        "split_namespace",
        "sealed_evaluation",
        "source_package",
        "sources",
        "counts",
        "variant_quarantine_sha256",
        "sealed_variant_quarantine_sha256",
        "sealed_test_exclusion_signatures_sha256",
        "training_language_scorer",
        "gate_policy",
        "training",
        "quantization",
        "calibration",
        "veto",
        "thresholds",
        "threshold_selection_gate_breakdown",
        "sealed_test",
        "sealed_test_typos",
        "sealed_test_context_stress",
        "safety",
        "build_provenance_sha256",
        "quality_gate_breakdown",
        "quality_gates_passed",
        "artifact_sha256",
        "artifact_model_version",
    }
)
_MANIFEST_MAPPING_KEYS: Final[frozenset[str]] = frozenset(
    {
        "toolchain",
        "sealed_evaluation",
        "source_package",
        "counts",
        "training_language_scorer",
        "gate_policy",
        "training",
        "quantization",
        "calibration",
        "veto",
        "thresholds",
        "threshold_selection_gate_breakdown",
        "sealed_test",
        "sealed_test_typos",
        "sealed_test_context_stress",
        "safety",
        "quality_gate_breakdown",
    }
)
_MANIFEST_SHA256_KEYS: Final[frozenset[str]] = frozenset(
    {
        "config_sha256",
        "dataset_sha256",
        "variant_quarantine_sha256",
        "sealed_variant_quarantine_sha256",
        "sealed_test_exclusion_signatures_sha256",
        "build_provenance_sha256",
        "artifact_sha256",
    }
)
_PRESEALED_PROVENANCE_CHECK_NAMES: Final[frozenset[str]] = frozenset(
    {
        "artifact_sha256",
        "config_sha256",
        "split_namespace",
        "sealed_evaluation",
        "sealed_candidate_sha256",
        "presealed_candidate_metadata",
        "gate_policy",
        "threshold_selection_gate",
        "training_quality_gate",
        "runtime_decision_parameters",
        "training_language_scorer",
        "hard_negative_development",
        "english_source_sha256",
        "russian_source_sha256",
        "embedded_manifest",
        "build_provenance_sha256",
        "model_version",
        "feature_schema",
        "membership_schema",
        "calibration_scope",
        *(
            f"toolchain_{field_name}"
            for field_name, _path in _TOOLCHAIN_CODE_PATHS
        ),
    }
)
MAX_EXTERNAL_MANIFEST_BYTES = 1 << 20
MAX_HUNSPELL_AFFIX_BYTES = 1 << 20
MAX_HUNSPELL_DICTIONARY_BYTES = 1 << 26
STRICT_COMPARISON_SAMPLE: Final[int] = 5_000
STRICT_LATENCY_SAMPLE: Final[int] = 5_000
INTERNAL_SEALED_EVIDENCE_CHECK_NAMES: Final[frozenset[str]] = frozenset(
    {
        "runtime_threshold_selection",
        "sealed_test",
        "sealed_test_typos",
        "sealed_test_context_stress",
        "veto_selection",
        "veto_sealed_test",
        "safety_guard_audit",
        "safety_raw_model_diagnostics",
        "quality_gate_breakdown",
        "sealed_dataset_metadata",
    }
)
EXTERNAL_CORPUS_PROVENANCE_GATE_NAMES: Final[frozenset[str]] = frozenset(
    {
        "external_policy_schema",
        "external_minimum_corpus_policy",
        "external_trigger_expansion_policy",
        "external_hunspell_provenance",
        "hunspell_handle_snapshot_stability",
        "lexical_disjoint_size",
        "lexical_disjoint_corpus_provenance",
        "unknown_typo_development_provenance",
        "unknown_typo_disjoint_size",
        "unknown_typo_holdout_provenance",
        "unknown_typo_holdout_disjointness",
    }
)
STRICT_GATE_NAMES: Final[frozenset[str]] = frozenset(
    {
        "provenance",
        "external_policy_schema",
        "external_minimum_corpus_policy",
        "external_trigger_expansion_policy",
        "external_hunspell_provenance",
        "hunspell_handle_snapshot_stability",
        "runtime_threshold_selection_evidence",
        "sealed_test",
        "sealed_test_context_stress",
        "safety",
        "typo_unknown_recall",
        "veto",
        "fallback_regression",
        "lexical_disjoint_size",
        "lexical_disjoint_corpus_provenance",
        "hunspell_hard_guard_regression",
        "lexical_disjoint_recall",
        "unknown_typo_development_provenance",
        "unknown_typo_disjoint_size",
        "unknown_typo_holdout_provenance",
        "unknown_typo_holdout_disjointness",
        "unknown_typo_model_evaluated",
        "unknown_typo_false_positives",
        "unknown_typo_recall",
        "unknown_typo_raw_model_integrity",
        "production_context_ensemble",
        "artifact_size",
        "load_latency",
        "inference_latency",
        "deterministic_inference",
    }
)


@dataclass(frozen=True)
class EvaluationArguments:
    config: Path
    english_model: Path
    russian_model: Path
    artifact: Path
    manifest: Path
    comparison_sample: int
    latency_sample: int
    strict: bool
    provenance_only: bool
    workers: int = 1


@dataclass(frozen=True)
class VerificationCheck:
    name: str
    passed: bool
    detail: str


@dataclass(frozen=True)
class PredictionComparison:
    fallback: ConfusionMatrix
    ensemble: ConfusionMatrix
    rescued_true_positives: int
    vetoed_true_positives: int
    prevented_false_positives: int
    introduced_false_positives: int
    samples: int
    model_evaluated_samples: int
    negative_model_evaluated: int


ContextGroupSelector = Literal["none", "source", "target"]


@dataclass(frozen=True)
class ProductionContextProfile:
    """One role-relative reachable endpoint of detector context arithmetic."""

    name: str
    source_context: float
    target_context: float
    group_selector: ContextGroupSelector

    @property
    def expected_delta(self) -> float:
        delta = CONTEXT_DELTA_MULTIPLIER * (
            self.target_context - self.source_context
        )
        if self.group_selector == "target":
            delta += CONTEXT_TARGET_GROUP_BONUS
        elif self.group_selector == "source":
            delta -= CONTEXT_SOURCE_GROUP_PENALTY
        return delta

    def context_group(self, source_group: int, target_group: int) -> int | None:
        if self.group_selector == "source":
            return source_group
        if self.group_selector == "target":
            return target_group
        return None


PRODUCTION_CONTEXT_PROFILES: Final[tuple[ProductionContextProfile, ...]] = (
    ProductionContextProfile("neutral", 0.0, 0.0, "none"),
    ProductionContextProfile(
        "none_min", CONTEXT_SCORE_MAXIMUM, CONTEXT_SCORE_MINIMUM, "none"
    ),
    ProductionContextProfile(
        "none_max", CONTEXT_SCORE_MINIMUM, CONTEXT_SCORE_MAXIMUM, "none"
    ),
    ProductionContextProfile(
        "source_min", CONTEXT_SCORE_MAXIMUM, CONTEXT_SCORE_MINIMUM, "source"
    ),
    ProductionContextProfile(
        "source_max", CONTEXT_SCORE_MINIMUM, CONTEXT_SCORE_MAXIMUM, "source"
    ),
    ProductionContextProfile(
        "target_min", CONTEXT_SCORE_MAXIMUM, CONTEXT_SCORE_MINIMUM, "target"
    ),
    ProductionContextProfile(
        "target_max", CONTEXT_SCORE_MINIMUM, CONTEXT_SCORE_MAXIMUM, "target"
    ),
)

_PRODUCTION_CONTEXT_CORPORA: Final[tuple[str, ...]] = (
    "sealed_test",
    "sealed_test_typos",
    "unknown_typo",
    "safety",
    "source_known",
)
_SOURCE_CONTEXT_SENTINEL: Final[str] = "\0keyswitch-context-source\0"
_TARGET_CONTEXT_SENTINEL: Final[str] = "\0keyswitch-context-target\0"


@dataclass(frozen=True)
class ProductionPolicyRow:
    example: LexicalExample
    fallback: bool
    ensemble: bool
    model_evaluated: bool
    observed_context_delta: float


@dataclass(frozen=True)
class ProductionContextCorpusEvaluation:
    examples: tuple[LexicalExample, ...]
    negative_ensemble_decisions: bytes
    overall: PredictionComparison
    per_trigger: dict[CorrectionTrigger, PredictionComparison]


@dataclass(frozen=True)
class ProductionContextProfileEvaluation:
    profile: ProductionContextProfile
    observed_deltas_by_direction: dict[str, tuple[float, ...]]
    corpora: dict[str, ProductionContextCorpusEvaluation]


@dataclass(frozen=True)
class ProductionContextEnsembleEvaluation:
    schema_version: int
    profiles: dict[str, ProductionContextProfileEvaluation]
    unique_model_predictions: int
    model_prediction_cache_hits: int


@dataclass(frozen=True)
class FrozenExternalFile:
    """Exact bytes observed for one bounded external evaluation input."""

    path: str
    sha256: str
    bytes: int


@dataclass(frozen=True)
class HunspellDictionaryProvenance:
    locale: str
    dictionary: FrozenExternalFile
    affix: FrozenExternalFile


@dataclass(frozen=True)
class HunspellDictionarySnapshot:
    """Parsed words and provenance derived from the very same immutable bytes."""

    words: tuple[str, ...]
    provenance: HunspellDictionaryProvenance


@dataclass(frozen=True)
class FrozenExternalFilePolicy:
    sha256: str
    bytes: int


@dataclass(frozen=True)
class HunspellLocalePolicy:
    dictionary: FrozenExternalFilePolicy
    affix: FrozenExternalFilePolicy


@dataclass(frozen=True)
class ExternalEvaluationPolicy:
    """Release-pinned policy that makes the external test fail closed."""

    schema_version: int
    minimum_words_per_group: int
    trigger_expansion: tuple[str, ...]
    hunspell: dict[int, HunspellLocalePolicy]
    lexical_disjoint_corpus_sha256: str
    unknown_typo_development_corpus_sha256: str
    unknown_typo_holdout_corpus_sha256: str


@dataclass(frozen=True)
class LexicalDisjointCorpus:
    examples: tuple[LexicalExample, ...]
    words_by_group: dict[int, int]
    dictionary_sources: dict[int, str]
    minimum_words_per_group: int
    rejected_sealed_overlaps_by_group: dict[int, int] = field(default_factory=dict)
    exclusion_signature_count: int = 0
    exclusion_signature_sha256: str = ""
    selected_sealed_overlap_count: int = 0
    corpus_sha256: str = ""
    rank_namespace: str = ""
    choice_namespace: str = ""
    dictionary_provenance: dict[int, HunspellDictionaryProvenance] = field(
        default_factory=dict
    )


@dataclass(frozen=True)
class SealedSignatureIndex:
    """Exact physical sequences already exposed to any sealed dataset slice."""

    signatures: frozenset[str]
    sha256: str
    split_lexical_signature_count: int
    safety_lexical_signature_count: int
    protected_exact_signature_count: int

    @property
    def signature_count(self) -> int:
        return len(self.signatures)


@dataclass(frozen=True)
class SafetyPolicyEvaluation:
    """Production detector results and guard-path accounting for safety rows."""

    per_trigger: dict[CorrectionTrigger, ConfusionMatrix]
    samples: int
    protected_samples: int
    lexical_collision_samples: int
    pre_model_guarded_samples: int
    expected_pre_model_guard_samples: int
    expected_pre_model_guarded_samples: int
    model_evaluated_samples: int
    guard_failure_samples: int
    reason_counts: dict[str, int]


@dataclass(frozen=True)
class ModelPredictionRow:
    example: LexicalExample
    should_switch: bool
    raw_logit: float
    coverage: float


@dataclass(frozen=True)
class InternalSealedEvidence:
    """Recomputed test evidence that needs no mutable system dictionary."""

    runtime_threshold_selection: dict[str, object]
    test_predictions: tuple[ModelPredictionRow, ...]
    test_metrics: dict[CorrectionTrigger, ConfusionMatrix]
    typo_test_metrics: dict[CorrectionTrigger, ConfusionMatrix]
    context_test_metrics: dict[
        str, dict[CorrectionTrigger, ConfusionMatrix]
    ]
    context_test_typo_metrics: dict[
        str, dict[CorrectionTrigger, ConfusionMatrix]
    ]
    safety_audit: GuardedSafetyAudit
    safety_raw_predictions: tuple[ModelPredictionRow, ...]
    safety_raw_metrics: dict[CorrectionTrigger, ConfusionMatrix]
    selection_veto: VetoSelection
    test_veto: VetoSelection
    quality_gate_breakdown: dict[str, object]
    checks: tuple[VerificationCheck, ...]


@dataclass(frozen=True)
class CoverageStatistics:
    """Descriptive membership coverage for one external-corpus slice."""

    samples: int
    minimum: float
    p05: float
    p25: float
    median: float
    p75: float
    p95: float
    maximum: float
    mean: float
    zero_coverage_samples: int
    full_coverage_samples: int


def _parse_arguments(argv: Sequence[str] | None = None) -> EvaluationArguments:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--config", type=Path, default=Path("model/intent_v1/config.json")
    )
    parser.add_argument(
        "--en-model",
        type=Path,
        default=Path("model/intent_v1/sources/en_US.lm"),
    )
    parser.add_argument(
        "--ru-model",
        type=Path,
        default=Path("model/intent_v1/sources/ru_RU.lm"),
    )
    parser.add_argument(
        "--artifact",
        type=Path,
        default=Path("src/keyswitch/resources/models/layout_intent_v1.ksm"),
    )
    parser.add_argument(
        "--manifest", type=Path, default=Path("model/intent_v1/manifest.json")
    )
    parser.add_argument(
        "--comparison-sample",
        type=int,
        default=STRICT_COMPARISON_SAMPLE,
    )
    parser.add_argument(
        "--latency-sample",
        type=int,
        default=STRICT_LATENCY_SAMPLE,
    )
    parser.add_argument("--strict", action="store_true")
    parser.add_argument(
        "--workers",
        type=int,
        default=0,
        metavar="N",
        help=(
            "worker processes for row scoring; 0 (the default) uses every "
            "logical CPU available to the process; results do not depend on it"
        ),
    )
    parser.add_argument(
        "--provenance-only",
        action="store_true",
        help=(
            "recompute and validate complete sealed candidate/test evidence, "
            "then exit before Hunspell discovery or external/runtime quality "
            "scoring"
        ),
    )
    namespace = parser.parse_args(argv)
    comparison_sample = cast(int, namespace.comparison_sample)
    latency_sample = cast(int, namespace.latency_sample)
    if comparison_sample < 0 or latency_sample < 1:
        parser.error("sample sizes must be non-negative (latency must be positive)")
    if cast(bool, namespace.strict) and (
        comparison_sample != STRICT_COMPARISON_SAMPLE
        or latency_sample != STRICT_LATENCY_SAMPLE
    ):
        parser.error(
            "strict evaluation requires comparison-sample=5000 and "
            "latency-sample=5000"
        )
    return EvaluationArguments(
        cast(Path, namespace.config),
        cast(Path, namespace.en_model),
        cast(Path, namespace.ru_model),
        cast(Path, namespace.artifact),
        cast(Path, namespace.manifest),
        comparison_sample,
        latency_sample,
        cast(bool, namespace.strict),
        cast(bool, namespace.provenance_only),
        resolve_training_workers(cast(int, namespace.workers)),
    )


def _read_bounded_external_file(
    path: Path,
    maximum_bytes: int,
    *,
    label: str,
) -> bytes:
    with path.open("rb") as stream:
        raw = stream.read(maximum_bytes + 1)
    if len(raw) > maximum_bytes:
        raise ValueError(f"{label} exceeds {maximum_bytes} bytes: {path}")
    return raw


def _json_object(
    path: Path, *, label: str = "model manifest"
) -> dict[str, object]:
    raw_json = _read_bounded_external_file(
        path,
        MAX_EXTERNAL_MANIFEST_BYTES,
        label=label,
    )

    def unique_object(
        pairs: list[tuple[str, object]],
    ) -> dict[str, object]:
        result: dict[str, object] = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(f"{label} contains duplicate key {key!r}")
            result[key] = value
        return result

    try:
        decoded: object = json.loads(
            raw_json.decode("utf-8"), object_pairs_hook=unique_object
        )
    except (UnicodeDecodeError, json.JSONDecodeError, RecursionError) as error:
        raise ValueError(f"{label} must contain valid UTF-8 JSON: {path}") from error
    if not isinstance(decoded, dict):
        raise ValueError(f"{label} must contain a JSON object: {path}")
    raw = cast(dict[object, object], decoded)
    if not all(isinstance(key, str) for key in raw):
        raise ValueError(f"{label} contains a non-string key: {path}")
    return {cast(str, key): value for key, value in raw.items()}


def _string(value: object, label: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{label} must be a string")
    return value


def _sha256(value: object, label: str) -> str:
    digest = _string(value, label)
    if len(digest) != 64 or any(
        character not in "0123456789abcdef" for character in digest
    ):
        raise ValueError(f"{label} must be an exact lowercase SHA-256 digest")
    return digest


def _mapping(value: object, label: str) -> dict[str, object]:
    if not isinstance(value, dict):
        raise ValueError(f"{label} must be an object")
    raw = cast(dict[object, object], value)
    if not all(isinstance(key, str) for key in raw):
        raise ValueError(f"{label} contains a non-string key")
    return {cast(str, key): item for key, item in raw.items()}


def external_evaluation_policy_from_config(
    config: TrainingConfig,
) -> ExternalEvaluationPolicy:
    """Adapt the already validated immutable training-config snapshot."""

    frozen = config.external_evaluation

    def locale_policy(group: int) -> HunspellLocalePolicy:
        source = frozen.english if group == 0 else frozen.russian
        return HunspellLocalePolicy(
            dictionary=FrozenExternalFilePolicy(
                sha256=source.dictionary_sha256,
                bytes=source.dictionary_bytes,
            ),
            affix=FrozenExternalFilePolicy(
                sha256=source.affix_sha256,
                bytes=source.affix_bytes,
            ),
        )

    return ExternalEvaluationPolicy(
        schema_version=frozen.schema_version,
        minimum_words_per_group=frozen.minimum_words_per_group,
        trigger_expansion=tuple(frozen.trigger_expansion),
        hunspell={group: locale_policy(group) for group in (0, 1)},
        lexical_disjoint_corpus_sha256=(
            frozen.lexical_disjoint_corpus_sha256
        ),
        unknown_typo_development_corpus_sha256=(
            frozen.unknown_typo_development_corpus_sha256
        ),
        unknown_typo_holdout_corpus_sha256=(
            frozen.unknown_typo_holdout_corpus_sha256
        ),
    )


def load_external_evaluation_policy(path: Path) -> ExternalEvaluationPolicy:
    """Load once through the canonical strict training-config decoder."""

    return external_evaluation_policy_from_config(load_training_config(path))


def _sequence(value: object, label: str) -> tuple[object, ...]:
    if not isinstance(value, list):
        raise ValueError(f"{label} must be an array")
    return tuple(cast(list[object], value))


def _check(name: str, passed: bool, detail: str) -> VerificationCheck:
    return VerificationCheck(name, passed, detail)


def _canonical_json_bytes(value: object) -> bytes:
    """Compare signed evidence by its strict JSON value, not Python containers."""

    return json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
        allow_nan=False,
    ).encode("utf-8")


def _source_hashes(source_rows: object) -> dict[int, str]:
    result: dict[int, str] = {}
    for index, value in enumerate(_sequence(source_rows, "manifest.sources")):
        row = _mapping(value, f"manifest.sources[{index}]")
        group = row.get("group")
        digest = row.get("sha256")
        if isinstance(group, bool) or not isinstance(group, int):
            raise ValueError("manifest source group must be an integer")
        result[group] = _string(digest, "manifest source sha256")
    return result


def _toolchain_code_hashes(toolchain_value: object) -> dict[str, str]:
    toolchain = _mapping(toolchain_value, "manifest.toolchain")
    return {
        field_name: _sha256(
            toolchain.get(field_name),
            f"manifest.toolchain.{field_name}",
        )
        for field_name, _path in _TOOLCHAIN_CODE_PATHS
    }


def _build_provenance_sha256(manifest: Mapping[str, object]) -> str:
    _sha256(manifest.get("config_sha256"), "manifest.config_sha256")
    _sha256(manifest.get("dataset_sha256"), "manifest.dataset_sha256")
    _string(manifest.get("split_namespace"), "manifest.split_namespace")
    _mapping(manifest.get("sealed_evaluation"), "manifest.sealed_evaluation")
    _mapping(manifest.get("gate_policy"), "manifest.gate_policy")
    _mapping(manifest.get("source_package"), "manifest.source_package")
    _sequence(manifest.get("sources"), "manifest.sources")
    _mapping(manifest.get("toolchain"), "manifest.toolchain")
    _sha256(
        manifest.get("variant_quarantine_sha256"),
        "manifest.variant_quarantine_sha256",
    )
    _mapping(
        manifest.get("training_language_scorer"),
        "manifest.training_language_scorer",
    )
    payload = {
        field_name: manifest[field_name]
        for field_name in _BUILD_PROVENANCE_FIELDS
    }
    return hashlib.sha256(
        json.dumps(
            payload,
            ensure_ascii=False,
            sort_keys=True,
            separators=(",", ":"),
        ).encode("utf-8")
    ).hexdigest()


def _require_exact_keys(
    mapping: Mapping[str, object],
    expected: Collection[str],
    label: str,
) -> None:
    actual = set(mapping)
    required = set(expected)
    if actual != required:
        raise ValueError(
            f"{label} keys mismatch: "
            f"missing={sorted(required - actual)}, "
            f"extra={sorted(actual - required)}"
        )


def _exact_mapping(
    value: object,
    expected: Collection[str],
    label: str,
) -> dict[str, object]:
    mapping = _mapping(value, label)
    _require_exact_keys(mapping, expected, label)
    return mapping


def _validate_manifest_schema(
    value: Mapping[str, object],
) -> dict[str, object]:
    """Reject every unknown manifest schema before provenance is evaluated."""

    manifest = _exact_mapping(value, _MANIFEST_KEYS, "manifest")
    if type(manifest.get("schema_version")) is not int:
        raise ValueError("manifest.schema_version must be an integer")
    if manifest["schema_version"] != 1:
        raise ValueError("unsupported model manifest schema")
    if manifest.get("model_id") != "keyswitch-layout-intent-v1":
        raise ValueError("manifest.model_id is unsupported")
    for field_name in _MANIFEST_SHA256_KEYS:
        _sha256(manifest.get(field_name), f"manifest.{field_name}")
    for field_name in _MANIFEST_MAPPING_KEYS:
        _mapping(manifest.get(field_name), f"manifest.{field_name}")
    _sequence(manifest.get("sources"), "manifest.sources")
    for field_name in (
        "calibration_scope",
        "split_namespace",
        "artifact_model_version",
    ):
        value_string = _string(
            manifest.get(field_name), f"manifest.{field_name}"
        )
        if not value_string:
            raise ValueError(f"manifest.{field_name} must not be empty")
    if type(manifest.get("quality_gates_passed")) is not bool:
        raise ValueError("manifest.quality_gates_passed must be a boolean")
    return manifest


def _finite_number(value: object) -> bool:
    return (
        isinstance(value, (int, float))
        and not isinstance(value, bool)
        and math.isfinite(float(value))
    )


def _nonnegative_integer(value: object) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and value >= 0


_BINARY_GATE_KEYS = frozenset(
    {"passed", "checks", "actual", "false_positive_bound", "limits"}
)
_BINARY_CHECK_KEYS = frozenset(
    {
        "positive_samples",
        "negative_samples",
        "precision",
        "recall",
        "specificity",
        "false_positive_rate_upper_bound",
    }
)
_METRICS_PAYLOAD_KEYS = frozenset(
    {
        "true_positive",
        "false_negative",
        "true_negative",
        "false_positive",
        "precision",
        "recall",
        "specificity",
        "false_positive_rate",
        "false_positive_rate_upper_95",
        "negative_samples",
    }
)
_BINARY_LIMIT_KEYS = frozenset(
    {
        "minimum_precision",
        "minimum_recall",
        "minimum_specificity",
        "maximum_false_positive_rate_upper_bound",
    }
)
_FALSE_POSITIVE_BOUND_KEYS = frozenset(
    {
        "method",
        "multiplicity_correction",
        "familywise_confidence",
        "comparisons",
        "per_comparison_confidence",
        "z_score",
        "upper",
    }
)


def _binary_gate_evidence_passed(
    value: object,
    label: str,
    *,
    selection_familywise: bool,
) -> bool:
    gate = _exact_mapping(value, _BINARY_GATE_KEYS, label)
    checks = _exact_mapping(
        gate.get("checks"), _BINARY_CHECK_KEYS, f"{label}.checks"
    )
    actual = _exact_mapping(
        gate.get("actual"), _METRICS_PAYLOAD_KEYS, f"{label}.actual"
    )
    limits = _exact_mapping(
        gate.get("limits"), _BINARY_LIMIT_KEYS, f"{label}.limits"
    )
    bound = _exact_mapping(
        gate.get("false_positive_bound"),
        _FALSE_POSITIVE_BOUND_KEYS,
        f"{label}.false_positive_bound",
    )
    expected_comparisons = (
        SELECTION_FALSE_POSITIVE_COMPARISONS
        if selection_familywise
        else 1
    )
    expected_confidence = (
        SELECTION_PER_COMPARISON_CONFIDENCE
        if selection_familywise
        else WILSON_INTERVAL_CONFIDENCE
    )
    expected_z_score = (
        SELECTION_WILSON_Z_SCORE
        if selection_familywise
        else WILSON_95_Z_SCORE
    )
    false_positives = actual.get("false_positive")
    negative_samples = actual.get("negative_samples")
    expected_upper = (
        wilson_upper_bound(
            cast(int, false_positives),
            cast(int, negative_samples),
            expected_z_score,
        )
        if _nonnegative_integer(false_positives)
        and _nonnegative_integer(negative_samples)
        else None
    )
    return (
        gate.get("passed") is True
        and all(checks.get(key) is True for key in _BINARY_CHECK_KEYS)
        and all(
            _nonnegative_integer(actual.get(key))
            for key in (
                "true_positive",
                "false_negative",
                "true_negative",
                "false_positive",
                "negative_samples",
            )
        )
        and all(
            _finite_number(actual.get(key))
            for key in _METRICS_PAYLOAD_KEYS
            if key
            not in {
                "true_positive",
                "false_negative",
                "true_negative",
                "false_positive",
                "negative_samples",
            }
        )
        and all(_finite_number(limits.get(key)) for key in _BINARY_LIMIT_KEYS)
        and bound.get("method") == "wilson_score_upper_endpoint"
        and bound.get("multiplicity_correction")
        == ("bonferroni" if selection_familywise else "none")
        and bound.get("familywise_confidence")
        == WILSON_INTERVAL_CONFIDENCE
        and bound.get("comparisons") == expected_comparisons
        and bound.get("per_comparison_confidence") == expected_confidence
        and bound.get("z_score") == expected_z_score
        and expected_upper is not None
        and _finite_number(bound.get("upper"))
        and math.isclose(
            float(cast(float, bound.get("upper"))),
            expected_upper,
            rel_tol=0.0,
            abs_tol=1e-15,
        )
        and expected_upper
        <= float(
            cast(
                float,
                limits.get("maximum_false_positive_rate_upper_bound"),
            )
        )
    )


def _context_gate_evidence_passed(
    value: object,
    label: str,
    *,
    selection_familywise: bool,
) -> bool:
    root = _exact_mapping(
        value,
        {"passed", "all_profiles_present", "expected_profiles", "profiles"},
        label,
    )
    expected_profiles = sorted(
        profile.name for profile in CONTEXT_STRESS_PROFILES
    )
    if (
        root.get("passed") is not True
        or root.get("all_profiles_present") is not True
        or root.get("expected_profiles") != expected_profiles
    ):
        return False
    profiles = _exact_mapping(
        root.get("profiles"), expected_profiles, f"{label}.profiles"
    )
    profile_contract = {
        profile.name: profile for profile in CONTEXT_STRESS_PROFILES
    }
    for name, contract in profile_contract.items():
        profile = _exact_mapping(
            profiles.get(name),
            {
                "passed",
                "delta",
                "group_selector",
                "all_triggers_present",
                "per_trigger",
            },
            f"{label}.profiles.{name}",
        )
        if (
            profile.get("passed") is not True
            or profile.get("all_triggers_present") is not True
            or profile.get("delta") != contract.delta
            or profile.get("group_selector") != contract.group_selector
        ):
            return False
        per_trigger = _exact_mapping(
            profile.get("per_trigger"),
            TRIGGERS,
            f"{label}.profiles.{name}.per_trigger",
        )
        for trigger in TRIGGERS:
            trigger_gate = _exact_mapping(
                per_trigger.get(trigger),
                {"passed", "overall", "typos"},
                f"{label}.profiles.{name}.per_trigger.{trigger}",
            )
            if (
                trigger_gate.get("passed") is not True
                or not _binary_gate_evidence_passed(
                    trigger_gate.get("overall"),
                    f"{label}.profiles.{name}.{trigger}.overall",
                    selection_familywise=selection_familywise,
                )
                or not _binary_gate_evidence_passed(
                    trigger_gate.get("typos"),
                    f"{label}.profiles.{name}.{trigger}.typos",
                    selection_familywise=selection_familywise,
                )
            ):
                return False
    return True


def _selection_trigger_evidence_passed(
    value: object,
    label: str,
    *,
    maximum_false_positives: int,
) -> bool:
    item = _exact_mapping(
        value,
        {
            "passed",
            "logits",
            "overall",
            "typos",
            "false_positive_budget",
        },
        label,
    )
    logits = _exact_mapping(
        item.get("logits"),
        LAYOUT_DIRECTIONS,
        f"{label}.logits",
    )
    budget = _exact_mapping(
        item.get("false_positive_budget"),
        {
            "passed",
            "checks",
            "actual",
            "maximum_false_positives_per_trigger",
        },
        f"{label}.false_positive_budget",
    )
    budget_checks = _exact_mapping(
        budget.get("checks"),
        {"overall", "typos"},
        f"{label}.false_positive_budget.checks",
    )
    budget_actual = _exact_mapping(
        budget.get("actual"),
        {"overall_false_positives", "typo_false_positives"},
        f"{label}.false_positive_budget.actual",
    )
    overall = _exact_mapping(
        item.get("overall"), _BINARY_GATE_KEYS, f"{label}.overall"
    )
    overall_actual = _exact_mapping(
        overall.get("actual"), _METRICS_PAYLOAD_KEYS, f"{label}.overall.actual"
    )
    typos = _exact_mapping(
        item.get("typos"), _BINARY_GATE_KEYS, f"{label}.typos"
    )
    typo_actual = _exact_mapping(
        typos.get("actual"), _METRICS_PAYLOAD_KEYS, f"{label}.typos.actual"
    )
    overall_false_positives = budget_actual.get("overall_false_positives")
    typo_false_positives = budget_actual.get("typo_false_positives")
    return (
        item.get("passed") is True
        and all(_finite_number(logits.get(direction)) for direction in LAYOUT_DIRECTIONS)
        and _binary_gate_evidence_passed(
            item.get("overall"),
            f"{label}.overall",
            selection_familywise=True,
        )
        and _binary_gate_evidence_passed(
            item.get("typos"),
            f"{label}.typos",
            selection_familywise=True,
        )
        and budget.get("passed") is True
        and budget_checks.get("overall") is True
        and budget_checks.get("typos") is True
        and _nonnegative_integer(overall_false_positives)
        and _nonnegative_integer(typo_false_positives)
        and overall_false_positives == overall_actual.get("false_positive")
        and typo_false_positives == typo_actual.get("false_positive")
        and budget.get("maximum_false_positives_per_trigger")
        == maximum_false_positives
        and cast(int, overall_false_positives) <= maximum_false_positives
        and cast(int, typo_false_positives) <= maximum_false_positives
    )


def _threshold_gate_evidence_passed(
    value: object,
    label: str,
    *,
    maximum_false_positives: int,
) -> bool:
    try:
        root = _exact_mapping(
            value,
            {
                "passed",
                "all_triggers_present",
                "per_trigger",
                "neutral",
                "context_stress",
            },
            label,
        )
        per_trigger = _exact_mapping(
            root.get("per_trigger"), TRIGGERS, f"{label}.per_trigger"
        )
        neutral = _exact_mapping(
            root.get("neutral"),
            {"passed", "all_triggers_present", "per_trigger"},
            f"{label}.neutral",
        )
        neutral_per_trigger = _exact_mapping(
            neutral.get("per_trigger"),
            TRIGGERS,
            f"{label}.neutral.per_trigger",
        )
        return (
            root.get("passed") is True
            and root.get("all_triggers_present") is True
            and neutral.get("passed") is True
            and neutral.get("all_triggers_present") is True
            and per_trigger == neutral_per_trigger
            and all(
                _selection_trigger_evidence_passed(
                    per_trigger.get(trigger),
                    f"{label}.per_trigger.{trigger}",
                    maximum_false_positives=maximum_false_positives,
                )
                for trigger in TRIGGERS
            )
            and _context_gate_evidence_passed(
                root.get("context_stress"),
                f"{label}.context_stress",
                selection_familywise=True,
            )
        )
    except ValueError:
        return False


def _quality_gate_evidence_passed(value: object, label: str) -> bool:
    try:
        root = _exact_mapping(
            value,
            {
                "passed",
                "all_triggers_present",
                "sealed_test",
                "sealed_test_typos",
                "sealed_test_context_stress",
                "safety",
                "veto",
            },
            label,
        )

        def sealed_slice(name: str) -> bool:
            section = _exact_mapping(
                root.get(name), {"passed", "per_trigger"}, f"{label}.{name}"
            )
            per_trigger = _exact_mapping(
                section.get("per_trigger"),
                TRIGGERS,
                f"{label}.{name}.per_trigger",
            )
            return section.get("passed") is True and all(
                _binary_gate_evidence_passed(
                    per_trigger.get(trigger),
                    f"{label}.{name}.per_trigger.{trigger}",
                    selection_familywise=False,
                )
                for trigger in TRIGGERS
            )

        safety = _exact_mapping(
            root.get("safety"),
            {"passed", "actual_guard_failures", "maximum_guard_failures"},
            f"{label}.safety",
        )
        veto = _exact_mapping(
            root.get("veto"),
            {
                "passed",
                "selection",
                "sealed_test",
                "maximum_false_negative_rate",
            },
            f"{label}.veto",
        )

        def veto_slice(name: str) -> bool:
            section = _exact_mapping(
                veto.get(name),
                {
                    "passed",
                    "raw_logit",
                    "positive_samples",
                    "vetoed_positive_samples",
                    "false_negative_rate",
                },
                f"{label}.veto.{name}",
            )
            return (
                section.get("passed") is True
                and _finite_number(section.get("raw_logit"))
                and _nonnegative_integer(section.get("positive_samples"))
                and int(cast(int, section.get("positive_samples"))) > 0
                and _nonnegative_integer(
                    section.get("vetoed_positive_samples")
                )
                and _finite_number(section.get("false_negative_rate"))
            )

        return (
            root.get("passed") is True
            and root.get("all_triggers_present") is True
            and sealed_slice("sealed_test")
            and sealed_slice("sealed_test_typos")
            and _context_gate_evidence_passed(
                root.get("sealed_test_context_stress"),
                f"{label}.sealed_test_context_stress",
                selection_familywise=False,
            )
            and safety.get("passed") is True
            and _nonnegative_integer(safety.get("actual_guard_failures"))
            and _nonnegative_integer(safety.get("maximum_guard_failures"))
            and veto.get("passed") is True
            and _finite_number(veto.get("maximum_false_negative_rate"))
            and veto_slice("selection")
            and veto_slice("sealed_test")
        )
    except ValueError:
        return False


def _runtime_decision_parameters_match(
    model: LinearNgramModel,
    manifest: Mapping[str, object],
    config: TrainingConfig,
) -> bool:
    """Bind shipped thresholds/veto to every signed selection record."""

    try:
        thresholds = _exact_mapping(
            manifest.get("thresholds"), TRIGGERS, "manifest.thresholds"
        )
        selection_gate = _exact_mapping(
            manifest.get("threshold_selection_gate_breakdown"),
            {
                "passed",
                "all_triggers_present",
                "per_trigger",
                "neutral",
                "context_stress",
            },
            "manifest.threshold_selection_gate_breakdown",
        )
        selection_triggers = _exact_mapping(
            selection_gate.get("per_trigger"),
            TRIGGERS,
            "manifest.threshold_selection_gate_breakdown.per_trigger",
        )
        if set(model.threshold_logits) != set(TRIGGERS) or set(
            model.thresholds
        ) != set(TRIGGERS):
            return False
        margins: set[float] = set()
        for trigger in TRIGGERS:
            threshold = _exact_mapping(
                thresholds.get(trigger),
                {
                    "global_logit_margin",
                    "logits",
                    "confidences",
                    "selection_metrics",
                    "selection_typo_metrics",
                },
                f"manifest.thresholds.{trigger}",
            )
            margin = threshold.get("global_logit_margin")
            if not _finite_number(margin):
                return False
            margins.add(float(cast(float | int, margin)))
            selection = _exact_mapping(
                selection_triggers.get(trigger),
                {
                    "passed",
                    "logits",
                    "overall",
                    "typos",
                    "false_positive_budget",
                },
                (
                    "manifest.threshold_selection_gate_breakdown."
                    f"per_trigger.{trigger}"
                ),
            )
            logits = _exact_mapping(
                threshold.get("logits"),
                LAYOUT_DIRECTIONS,
                f"manifest.thresholds.{trigger}.logits",
            )
            confidences = _exact_mapping(
                threshold.get("confidences"),
                LAYOUT_DIRECTIONS,
                f"manifest.thresholds.{trigger}.confidences",
            )
            selection_logits = _exact_mapping(
                selection.get("logits"),
                LAYOUT_DIRECTIONS,
                (
                    "manifest.threshold_selection_gate_breakdown."
                    f"per_trigger.{trigger}.logits"
                ),
            )
            for direction in LAYOUT_DIRECTIONS:
                logit = logits.get(direction)
                confidence = confidences.get(direction)
                if (
                    not _finite_number(logit)
                    or not _finite_number(confidence)
                    or logit != selection_logits.get(direction)
                    or float(cast(float | int, logit))
                    != model.threshold_logits[trigger][direction]
                    or float(cast(float | int, confidence))
                    != model.thresholds[trigger][direction]
                ):
                    return False

        if (
            len(margins) != 1
            or next(iter(margins)) < 0.0
            or next(iter(margins)) > config.threshold_logit_margin_cap
        ):
            return False

        veto = _exact_mapping(
            manifest.get("veto"),
            {"selection", "sealed_test"},
            "manifest.veto",
        )
        veto_selection = _exact_mapping(
            veto.get("selection"),
            {
                "raw_logit",
                "positive_samples",
                "vetoed_positive_samples",
                "false_negative_rate",
            },
            "manifest.veto.selection",
        )
        quality = _exact_mapping(
            manifest.get("quality_gate_breakdown"),
            {
                "passed",
                "all_triggers_present",
                "sealed_test",
                "sealed_test_typos",
                "sealed_test_context_stress",
                "safety",
                "veto",
            },
            "manifest.quality_gate_breakdown",
        )
        quality_veto = _exact_mapping(
            quality.get("veto"),
            {
                "passed",
                "selection",
                "sealed_test",
                "maximum_false_negative_rate",
            },
            "manifest.quality_gate_breakdown.veto",
        )
        quality_selection = _exact_mapping(
            quality_veto.get("selection"),
            {
                "passed",
                "raw_logit",
                "positive_samples",
                "vetoed_positive_samples",
                "false_negative_rate",
            },
            "manifest.quality_gate_breakdown.veto.selection",
        )
        raw_logit = veto_selection.get("raw_logit")
        return (
            _finite_number(raw_logit)
            and raw_logit == quality_selection.get("raw_logit")
            and float(cast(float | int, raw_logit)) == model.veto_threshold
        )
    except (AttributeError, KeyError, TypeError, ValueError):
        return False


def verify_provenance(
    *,
    model: LinearNgramModel,
    artifact: Path,
    manifest: Mapping[str, object],
    config_path: Path,
    config: TrainingConfig,
    english_path: Path,
    russian_path: Path,
    dataset: DatasetBundle | None,
    training_language_scorer: Mapping[str, object],
    hard_negative_development: Mapping[str, object] | None = None,
    candidate_dataset: DatasetBundle | None = None,
    sealed_registry_root: Path | None = None,
) -> tuple[VerificationCheck, ...]:
    manifest = _validate_manifest_schema(manifest)
    artifact_digest = sha256_file(artifact)
    config_digest = sha256_file(config_path)
    dataset_digest = (
        dataset_fingerprint(dataset) if dataset is not None else None
    )
    candidate_source = candidate_dataset or dataset
    if candidate_source is None:
        raise ValueError("candidate dataset is required for provenance")
    candidate_dataset_digest = dataset_fingerprint(candidate_source)
    english_digest = sha256_file(english_path)
    russian_digest = sha256_file(russian_path)
    embedded = model.metadata
    sidecar_without_artifact = {
        key: value
        for key, value in manifest.items()
        if key not in {"artifact_sha256", "artifact_model_version"}
    }
    source_hashes = _source_hashes(manifest.get("sources"))
    manifest_toolchain_hashes = _toolchain_code_hashes(manifest.get("toolchain"))
    calculated_build_provenance = _build_provenance_sha256(manifest)
    recorded_build_provenance = _sha256(
        manifest.get("build_provenance_sha256"),
        "manifest.build_provenance_sha256",
    )
    artifact_model_version = _string(
        manifest.get("artifact_model_version"),
        "manifest.artifact_model_version",
    )
    expected_model_version = "intent-v1-" + calculated_build_provenance[:12]
    manifest_training_language_scorer = _mapping(
        manifest.get("training_language_scorer"),
        "manifest.training_language_scorer",
    )
    manifest_hard_negative_development: dict[str, object] | None = None
    try:
        manifest_training = _mapping(
            manifest.get("training"), "manifest.training"
        )
        manifest_hard_negative_development = _mapping(
            manifest_training.get("hard_negative_development"),
            "manifest.training.hard_negative_development",
        )
    except (TypeError, ValueError):
        pass
    threshold_gate_passed = _threshold_gate_evidence_passed(
        manifest.get("threshold_selection_gate_breakdown"),
        "manifest.threshold_selection_gate_breakdown",
        maximum_false_positives=(
            config.selection_maximum_false_positives_per_trigger
        ),
    )
    quality_gate_passed = _quality_gate_evidence_passed(
        manifest.get("quality_gate_breakdown"),
        "manifest.quality_gate_breakdown",
    )
    runtime_parameters_match = _runtime_decision_parameters_match(
        model, manifest, config
    )
    sealed_candidate_digest: str | None = None
    candidate_metadata: dict[str, object] | None = None
    try:
        manifest_veto = _mapping(manifest.get("veto"), "manifest.veto")
        candidate_metadata = presealed_candidate_metadata_projection(
            model_id=_string(manifest.get("model_id"), "manifest.model_id"),
            calibration_scope=_string(
                manifest.get("calibration_scope"),
                "manifest.calibration_scope",
            ),
            config_sha256=config_digest,
            split_namespace=config.sealed_evaluation.split_namespace,
            toolchain=_mapping(
                manifest.get("toolchain"), "manifest.toolchain"
            ),
            source_package=_mapping(
                manifest.get("source_package"), "manifest.source_package"
            ),
            sources=_sequence(manifest.get("sources"), "manifest.sources"),
            candidate_counts=presealed_candidate_counts(candidate_source),
            variant_quarantine_sha256=(
                candidate_source.variant_quarantine.sha256
            ),
            training_language_scorer=training_language_scorer,
            gate_policy=gate_policy_payload(config),
            training=_mapping(
                manifest.get("training"), "manifest.training"
            ),
            quantization=_mapping(
                manifest.get("quantization"), "manifest.quantization"
            ),
            calibration=_mapping(
                manifest.get("calibration"), "manifest.calibration"
            ),
            veto_selection=_mapping(
                manifest_veto.get("selection"),
                "manifest.veto.selection",
            ),
            thresholds=_mapping(
                manifest.get("thresholds"), "manifest.thresholds"
            ),
            selection_gate_breakdown=_mapping(
                manifest.get("threshold_selection_gate_breakdown"),
                "manifest.threshold_selection_gate_breakdown",
            ),
            safety_guard_audit=asdict(
                audit_guarded_safety_corpus(candidate_source.safety)
            ),
            model_parameters=runtime_candidate_model_parameters(model),
        )
        sealed_candidate_digest = sealed_candidate_sha256(
            split_namespace=config.sealed_evaluation.split_namespace,
            config_sha256=config_digest,
            candidate_dataset_sha256=candidate_dataset_digest,
            toolchain=_mapping(manifest.get("toolchain"), "manifest.toolchain"),
            training_language_scorer=training_language_scorer,
            model_parameters=runtime_candidate_model_parameters(model),
            selection_gate_breakdown=_mapping(
                manifest.get("threshold_selection_gate_breakdown"),
                "manifest.threshold_selection_gate_breakdown",
            ),
            candidate_metadata=candidate_metadata,
        )
    except (AttributeError, KeyError, TypeError, ValueError):
        pass
    sealed_evaluation_valid = (
        sealed_candidate_digest is not None
        and sealed_evaluation_evidence_is_valid(
            config=config,
            value=manifest.get("sealed_evaluation"),
            expected_config_sha256=config_digest,
            expected_candidate_dataset_sha256=candidate_dataset_digest,
            expected_candidate_sha256=sealed_candidate_digest,
            repository_root=sealed_registry_root,
        )
    )
    toolchain_checks: list[VerificationCheck] = []
    for field_name, path in _TOOLCHAIN_CODE_PATHS:
        current_digest = sha256_file(path)
        manifest_digest = manifest_toolchain_hashes[field_name]
        toolchain_checks.append(
            _check(
                f"toolchain_{field_name}",
                manifest_digest == current_digest,
                f"current={current_digest}, manifest={manifest_digest}",
            )
        )
    checks: list[VerificationCheck] = [
        _check(
            "artifact_sha256",
            artifact_digest == manifest.get("artifact_sha256") == model.checksum,
            artifact_digest,
        ),
        _check(
            "config_sha256",
            config_digest == manifest.get("config_sha256"),
            config_digest,
        ),
        _check(
            "split_namespace",
            manifest.get("split_namespace") == SPLIT_NAMESPACE,
            SPLIT_NAMESPACE,
        ),
        _check(
            "sealed_evaluation",
            sealed_evaluation_valid,
            "manifest receipt equals the immutable one-candidate registry",
        ),
        _check(
            "sealed_candidate_sha256",
            sealed_evaluation_valid,
            (
                "loaded KSLM runtime parameters reproduce the registry "
                f"candidate {sealed_candidate_digest}"
            ),
        ),
        _check(
            "presealed_candidate_metadata",
            candidate_metadata is not None and sealed_evaluation_valid,
            (
                "canonical training, quantization, calibration, source, "
                "count, threshold, and safety metadata reproduce the "
                "registry candidate"
            ),
        ),
        _check(
            "gate_policy",
            manifest.get("gate_policy") == gate_policy_payload(config),
            "manifest gate policy equals the loaded training config",
        ),
        _check(
            "threshold_selection_gate",
            threshold_gate_passed,
            "signed threshold-selection evidence passed",
        ),
        _check(
            "training_quality_gate",
            quality_gate_passed and manifest.get("quality_gates_passed") is True,
            "signed training quality evidence passed",
        ),
        _check(
            "runtime_decision_parameters",
            runtime_parameters_match,
            "KSLM thresholds and veto equal every signed selection record",
        ),
        _check(
            "training_language_scorer",
            manifest_training_language_scorer
            == dict(training_language_scorer),
            "manifest scorer provenance equals reconstructed train-only scorer",
        ),
        _check(
            "hard_negative_development",
            hard_negative_development is not None
            and manifest_hard_negative_development
            == dict(hard_negative_development),
            (
                "manifest hard-negative provenance equals the verified frozen "
                "development corpus"
            ),
        ),
        _check(
            "english_source_sha256",
            source_hashes.get(0) == english_digest,
            english_digest,
        ),
        _check(
            "russian_source_sha256",
            source_hashes.get(1) == russian_digest,
            russian_digest,
        ),
        _check(
            "embedded_manifest",
            embedded == sidecar_without_artifact,
            "embedded metadata equals the signed training manifest",
        ),
        _check(
            "build_provenance_sha256",
            calculated_build_provenance == recorded_build_provenance,
            (
                f"calculated={calculated_build_provenance}, "
                f"manifest={recorded_build_provenance}"
            ),
        ),
        _check(
            "model_version",
            model.model_version == artifact_model_version == expected_model_version,
            (
                f"model={model.model_version}, sidecar={artifact_model_version}, "
                f"expected={expected_model_version}"
            ),
        ),
        _check(
            "feature_schema",
            model.dimension == config.dimension
            and model.fnv_seed == config.feature_hash_seed,
            f"dimension={model.dimension}, fnv_seed={model.fnv_seed}",
        ),
        _check(
            "membership_schema",
            model.membership_seed == config.membership_hash_seed,
            (
                f"membership_seed={model.membership_seed}, "
                f"expected={config.membership_hash_seed}"
            ),
        ),
        _check(
            "calibration_scope",
            manifest.get("calibration_scope")
            == "lexical-synthetic-not-real-world-probability",
            str(manifest.get("calibration_scope")),
        ),
    ]
    if dataset_digest is not None:
        checks.insert(
            2,
            _check(
                "dataset_sha256",
                dataset_digest == manifest.get("dataset_sha256"),
                dataset_digest,
            ),
        )
    checks.extend(toolchain_checks)
    return tuple(checks)


def provenance_checks_pass(
    checks: Sequence[VerificationCheck], *, require_full_dataset: bool
) -> bool:
    """Require the exact provenance gate set before any sealed-test access."""

    expected = set(_PRESEALED_PROVENANCE_CHECK_NAMES)
    if require_full_dataset:
        expected.add("dataset_sha256")
    return (
        len(checks) == len(expected)
        and {check.name for check in checks} == expected
        and all(check.passed is True for check in checks)
    )


def _confusion(labels_and_predictions: Iterable[tuple[bool, bool]]) -> ConfusionMatrix:
    true_positive = false_negative = true_negative = false_positive = 0
    for label, predicted in labels_and_predictions:
        if label and predicted:
            true_positive += 1
        elif label:
            false_negative += 1
        elif predicted:
            false_positive += 1
        else:
            true_negative += 1
    return ConfusionMatrix(true_positive, false_negative, true_negative, false_positive)


def model_metrics(
    model: LinearNgramModel,
    examples: Iterable[LexicalExample],
    *,
    scorers: Mapping[int, WordScorer],
) -> dict[CorrectionTrigger, ConfusionMatrix]:
    return prediction_metrics(
        predict_model_examples(model, examples, scorers=scorers)
    )


def predict_model_examples(
    model: LinearNgramModel,
    examples: Iterable[LexicalExample],
    *,
    scorers: Mapping[int, WordScorer],
    workers: int | None = None,
) -> tuple[ModelPredictionRow, ...]:
    rows = tuple(examples)
    actual_workers = _effective_row_workers(workers, len(rows))
    if actual_workers > 1:
        outputs = _map_row_chunks(
            _predict_rows_worker,
            _RowWorkload(examples=rows, model=model, scorers=scorers),
            actual_workers,
        )
        return tuple(
            ModelPredictionRow(example, should_switch, logit, coverage)
            for example, (should_switch, logit, coverage) in zip(
                rows, outputs, strict=True
            )
        )
    result: list[ModelPredictionRow] = []
    for example in rows:
        prediction = model.predict(
            intent_input_for_example(example, scorers=scorers)
        )
        result.append(
            ModelPredictionRow(
                example,
                prediction.should_switch,
                prediction.logit,
                prediction.coverage,
            )
        )
    return tuple(result)


def prediction_metrics(
    predictions: Iterable[ModelPredictionRow],
) -> dict[CorrectionTrigger, ConfusionMatrix]:
    rows: dict[CorrectionTrigger, list[tuple[bool, bool]]] = {
        trigger: [] for trigger in TRIGGERS
    }
    for item in predictions:
        rows[item.example.trigger].append(
            (item.example.label, item.should_switch)
        )
    return {
        trigger: _confusion(rows[trigger])
        for trigger in TRIGGERS
    }


def evaluate_context_stress(
    model: LinearNgramModel,
    examples: Sequence[LexicalExample],
    *,
    scorers: Mapping[int, WordScorer],
    neutral_predictions: Sequence[ModelPredictionRow] | None = None,
) -> tuple[
    dict[str, dict[CorrectionTrigger, ConfusionMatrix]],
    dict[str, dict[CorrectionTrigger, ConfusionMatrix]],
]:
    """Reuse neutral predictions after proving exact runtime feature invariance."""

    verify_context_feature_invariance(
        dimension=model.dimension,
        extractor=runtime_feature_extractor(
            model.fnv_seed,
            model.membership_seed,
            scorers=scorers,
        ),
    )
    if neutral_predictions is None:
        predictions = predict_model_examples(model, examples, scorers=scorers)
    else:
        if len(neutral_predictions) != len(examples) or any(
            prediction.example != example
            for prediction, example in zip(
                neutral_predictions, examples, strict=True
            )
        ):
            raise ValueError(
                "neutral predictions do not match context-stress examples"
            )
        predictions = tuple(neutral_predictions)

    neutral_overall = prediction_metrics(predictions)
    neutral_typos = prediction_metrics(
        item
        for item in predictions
        if item.example.variant_kind in TYPO_VARIANT_KINDS
    )
    return (
        {
            profile.name: dict(neutral_overall)
            for profile in CONTEXT_STRESS_PROFILES
        },
        {
            profile.name: dict(neutral_typos)
            for profile in CONTEXT_STRESS_PROFILES
        },
    )


def evaluate_runtime_threshold_selection(
    model: LinearNgramModel,
    examples: Sequence[LexicalExample],
    *,
    config: TrainingConfig,
    scorers: Mapping[int, WordScorer],
) -> dict[str, object]:
    """Recompute the complete pre-sealed selection evidence via runtime code."""

    predictions = predict_model_examples(model, examples, scorers=scorers)
    overall = prediction_metrics(predictions)
    typos = prediction_metrics(
        item
        for item in predictions
        if item.example.variant_kind in TYPO_VARIANT_KINDS
    )
    context, context_typos = evaluate_context_stress(
        model,
        examples,
        scorers=scorers,
        neutral_predictions=predictions,
    )
    selections = {
        trigger: ThresholdSelection(
            trigger,
            max(model.threshold_logits[trigger].values()),
            overall[trigger],
            typos[trigger],
            model.threshold_logits[trigger],
        )
        for trigger in TRIGGERS
    }
    return threshold_selection_gate_breakdown(
        config,
        selections,
        context,
        context_typos,
    )


def _veto_from_predictions(
    predictions: Sequence[ModelPredictionRow], threshold: float
) -> VetoSelection:
    positives = tuple(item for item in predictions if item.example.label)
    if not positives:
        raise ValueError("veto evaluation requires positive examples")
    vetoed = sum(item.raw_logit < threshold for item in positives)
    return VetoSelection(
        threshold,
        len(positives),
        vetoed,
        vetoed / len(positives),
    )


def recompute_internal_sealed_evidence(
    *,
    model: LinearNgramModel,
    manifest: Mapping[str, object],
    config: TrainingConfig,
    prepared: PreparedLexicon,
    dataset: DatasetBundle,
    scorers: Mapping[int, WordScorer],
) -> InternalSealedEvidence:
    """Recompute and exactly compare every first-party sealed-test record."""

    runtime_threshold_selection = evaluate_runtime_threshold_selection(
        model,
        dataset.by_split["threshold"],
        config=config,
        scorers=scorers,
    )
    calibration_predictions = predict_model_examples(
        model,
        dataset.by_split["calibration"],
        scorers=scorers,
    )
    selection_veto = _veto_from_predictions(
        calibration_predictions, model.veto_threshold
    )
    test_predictions = predict_model_examples(
        model,
        dataset.by_split["test"],
        scorers=scorers,
    )
    test_metrics = prediction_metrics(test_predictions)
    typo_test_metrics = prediction_metrics(
        item
        for item in test_predictions
        if item.example.variant_kind in TYPO_VARIANT_KINDS
    )
    context_test_metrics, context_test_typo_metrics = evaluate_context_stress(
        model,
        dataset.by_split["test"],
        scorers=scorers,
        neutral_predictions=test_predictions,
    )
    safety_audit = audit_guarded_safety_corpus(dataset.safety)
    safety_raw_predictions = predict_model_examples(
        model,
        dataset.safety,
        scorers=scorers,
    )
    safety_raw_metrics = prediction_metrics(safety_raw_predictions)
    test_veto = _veto_from_predictions(
        test_predictions, model.veto_threshold
    )
    quality_gate_breakdown = training_quality_gate_breakdown(
        config,
        test_metrics,
        typo_test_metrics,
        safety_audit,
        selection_veto,
        test_veto,
        context_test_metrics,
        context_test_typo_metrics,
    )
    expected_test = {
        trigger: metrics_payload(metrics)
        for trigger, metrics in test_metrics.items()
    }
    expected_typos = {
        trigger: metrics_payload(metrics)
        for trigger, metrics in typo_test_metrics.items()
    }
    expected_context = {
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
    }
    expected_safety_guard = asdict(safety_audit)
    expected_safety_raw = {
        trigger: metrics_payload(metrics)
        for trigger, metrics in safety_raw_metrics.items()
    }
    expected_counts = {
        "lexicon_words": sum(
            len(items) for items in prepared.words_by_split.values()
        ),
        "collisions": len(prepared.collisions),
        "examples": {
            split: len(dataset.by_split[split]) for split in SPLIT_NAMES
        },
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
    }
    expected_exclusion_sha256 = hashlib.sha256(
        "\n".join(dataset.sealed_test_exclusion_signatures).encode("utf-8")
    ).hexdigest()
    manifest_veto = _mapping(manifest.get("veto"), "manifest.veto")
    manifest_safety = _mapping(manifest.get("safety"), "manifest.safety")
    veto_shape_matches = set(manifest_veto) == {"selection", "sealed_test"}
    safety_shape_matches = set(manifest_safety) == {
        "guard_audit",
        "raw_model_diagnostics",
    }
    checks = (
        _check(
            "runtime_threshold_selection",
            runtime_threshold_selection
            == manifest.get("threshold_selection_gate_breakdown"),
            "runtime threshold split exactly reproduces signed evidence",
        ),
        _check(
            "sealed_test",
            expected_test == manifest.get("sealed_test"),
            "runtime test predictions exactly reproduce signed metrics",
        ),
        _check(
            "sealed_test_typos",
            expected_typos == manifest.get("sealed_test_typos"),
            "runtime typo slice exactly reproduces signed metrics",
        ),
        _check(
            "sealed_test_context_stress",
            expected_context == manifest.get("sealed_test_context_stress"),
            "runtime context profiles exactly reproduce signed metrics",
        ),
        _check(
            "veto_selection",
            veto_shape_matches
            and asdict(selection_veto) == manifest_veto.get("selection"),
            "runtime calibration split exactly reproduces selected veto",
        ),
        _check(
            "veto_sealed_test",
            veto_shape_matches
            and asdict(test_veto) == manifest_veto.get("sealed_test"),
            "runtime test logits exactly reproduce signed veto evidence",
        ),
        _check(
            "safety_guard_audit",
            safety_shape_matches
            and _canonical_json_bytes(expected_safety_guard)
            == _canonical_json_bytes(manifest_safety.get("guard_audit")),
            "production static guards exactly reproduce signed safety audit",
        ),
        _check(
            "safety_raw_model_diagnostics",
            safety_shape_matches
            and expected_safety_raw
            == manifest_safety.get("raw_model_diagnostics"),
            "runtime raw safety predictions exactly reproduce diagnostics",
        ),
        _check(
            "quality_gate_breakdown",
            quality_gate_breakdown == manifest.get("quality_gate_breakdown")
            and manifest.get("quality_gates_passed")
            is (quality_gate_breakdown.get("passed") is True),
            "recomputed inputs exactly reproduce the complete quality gate",
        ),
        _check(
            "sealed_dataset_metadata",
            expected_counts == manifest.get("counts")
            and dataset.variant_quarantine.sha256
            == manifest.get("variant_quarantine_sha256")
            and dataset.sealed_variant_quarantine.sha256
            == manifest.get("sealed_variant_quarantine_sha256")
            and expected_exclusion_sha256
            == manifest.get("sealed_test_exclusion_signatures_sha256"),
            "reconstructed sealed counts and quarantine hashes match",
        ),
    )
    return InternalSealedEvidence(
        runtime_threshold_selection=runtime_threshold_selection,
        test_predictions=test_predictions,
        test_metrics=test_metrics,
        typo_test_metrics=typo_test_metrics,
        context_test_metrics=context_test_metrics,
        context_test_typo_metrics=context_test_typo_metrics,
        safety_audit=safety_audit,
        safety_raw_predictions=safety_raw_predictions,
        safety_raw_metrics=safety_raw_metrics,
        selection_veto=selection_veto,
        test_veto=test_veto,
        quality_gate_breakdown=quality_gate_breakdown,
        checks=checks,
    )


def internal_sealed_evidence_checks_pass(
    checks: Sequence[VerificationCheck],
) -> bool:
    """Fail closed on missing, duplicate, extra, or non-passing evidence."""

    return (
        len(checks) == len(INTERNAL_SEALED_EVIDENCE_CHECK_NAMES)
        and {check.name for check in checks}
        == set(INTERNAL_SEALED_EVIDENCE_CHECK_NAMES)
        and all(check.passed is True for check in checks)
    )


def _deterministic_sample(
    examples: Sequence[_T], requested: int
) -> tuple[_T, ...]:
    if requested == 0 or requested >= len(examples):
        return tuple(examples)
    if requested == 1:
        return (examples[0],)
    return tuple(
        examples[round(index * (len(examples) - 1) / (requested - 1))]
        for index in range(requested)
    )


_SEALED_LEXICAL_VARIANTS = frozenset(
    {"identity", "deletion", "duplication", "transposition", "lexical_collision"}
)
_SEALED_HARD_NEGATIVE_VARIANTS = frozenset(
    {
        "hunspell-unknown-deletion",
        "hunspell-unknown-duplication",
        "hunspell-unknown-transposition",
    }
)
_HARD_NEGATIVE_SIGNATURE_PREFIX = "hunspell-unknown:"


def physical_signature_set_sha256(signatures: Collection[str]) -> str:
    """Fingerprint a set without relying on iteration order or separators."""

    normalized = frozenset(signatures)
    if "" in normalized:
        raise ValueError("physical-signature provenance must not contain empty values")
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


def _rendered_physical_signature(example: LexicalExample) -> str:
    candidates = {
        signature
        for signature in (
            physical_signature(example.original, example.source_group),
            physical_signature(example.alternative, example.target_group),
        )
        if signature
    }
    if len(candidates) != 1:
        raise RuntimeError(
            "sealed lexical row has missing or contradictory physical signatures: "
            f"base={example.base_signature!r}, candidates={sorted(candidates)!r}"
        )
    return next(iter(candidates))


def _rendered_hard_negative_signature(example: LexicalExample) -> str:
    if not example.base_signature.startswith(_HARD_NEGATIVE_SIGNATURE_PREFIX):
        raise RuntimeError(
            "sealed hard-negative row has a foreign base signature: "
            f"base={example.base_signature!r}"
        )
    expected = example.base_signature.removeprefix(
        _HARD_NEGATIVE_SIGNATURE_PREFIX
    )
    signature, error = row_physical_signature(example)
    if not expected or error is not None or signature != expected:
        raise RuntimeError(
            "sealed hard-negative row does not reproduce its physical "
            f"signature: base={example.base_signature!r}, "
            f"rendered={signature!r}, error={error!r}"
        )
    return signature


def build_sealed_signature_index(dataset: DatasetBundle) -> SealedSignatureIndex:
    """Index every emitted lexical sequence plus protected exact key strings."""

    split_lexical: set[str] = set()
    safety_lexical: set[str] = set()
    protected_exact: set[str] = set()
    for split_rows in dataset.by_split.values():
        for example in split_rows:
            if example.variant_kind in _SEALED_LEXICAL_VARIANTS:
                split_lexical.add(_rendered_physical_signature(example))
            elif example.variant_kind in _SEALED_HARD_NEGATIVE_VARIANTS:
                split_lexical.add(
                    _rendered_hard_negative_signature(example)
                )
            elif example.protected and example.base_signature.startswith("hard:"):
                protected_exact.add(example.base_signature.removeprefix("hard:"))
            else:
                raise RuntimeError(
                    "cannot prove sealed-signature exclusion for unsupported "
                    f"dataset row: variant={example.variant_kind!r}, "
                    f"base={example.base_signature!r}"
                )
    for example in dataset.safety:
        if example.variant_kind in _SEALED_LEXICAL_VARIANTS:
            safety_lexical.add(_rendered_physical_signature(example))
        elif example.variant_kind in _SEALED_HARD_NEGATIVE_VARIANTS:
            safety_lexical.add(
                _rendered_hard_negative_signature(example)
            )
        elif example.protected and example.base_signature.startswith("hard:"):
            protected_exact.add(example.base_signature.removeprefix("hard:"))
        else:
            raise RuntimeError(
                "cannot prove sealed-signature exclusion for unsupported "
                f"safety row: variant={example.variant_kind!r}, "
                f"base={example.base_signature!r}"
            )
    combined = frozenset(split_lexical | safety_lexical | protected_exact)
    return SealedSignatureIndex(
        signatures=combined,
        sha256=physical_signature_set_sha256(combined),
        split_lexical_signature_count=len(split_lexical),
        safety_lexical_signature_count=len(safety_lexical),
        protected_exact_signature_count=len(protected_exact),
    )


def _missing_hunspell_snapshot(locale: str) -> HunspellDictionarySnapshot:
    missing = FrozenExternalFile(
        path="",
        sha256=hashlib.sha256(b"").hexdigest(),
        bytes=0,
    )
    return HunspellDictionarySnapshot(
        words=(),
        provenance=HunspellDictionaryProvenance(
            locale=locale,
            dictionary=missing,
            affix=missing,
        ),
    )


def _hunspell_dictionary_snapshot(
    model: LanguageModel, locale: str
) -> HunspellDictionarySnapshot:
    """Snapshot the exact paths backing an already-created runtime handle."""

    if not model.speller.available or not model.speller.source:
        return _missing_hunspell_snapshot(locale)
    dictionary_path = Path(model.speller.source)
    affix_path = dictionary_path.with_suffix(".aff")
    return _hunspell_snapshot_from_paths(
        locale,
        dictionary_path=dictionary_path,
        affix_path=affix_path,
    )


def _hunspell_snapshot_from_paths(
    locale: str,
    *,
    dictionary_path: Path,
    affix_path: Path,
) -> HunspellDictionarySnapshot:
    """Derive parsed words and provenance from one pair of bounded byte reads."""

    encoding = "utf-8"
    try:
        affix_bytes = _read_bounded_external_file(
            affix_path,
            MAX_HUNSPELL_AFFIX_BYTES,
            label="Hunspell affix file",
        )
        for raw_line in affix_bytes.splitlines()[:80]:
            if raw_line.startswith(b"SET "):
                encoding = raw_line[4:].decode("ascii", "replace")
                break
        dictionary_bytes = _read_bounded_external_file(
            dictionary_path,
            MAX_HUNSPELL_DICTIONARY_BYTES,
            label="Hunspell dictionary",
        )
        candidates = {
            LanguageModel.normalize(line.split("/", 1)[0].replace(r"\/", "/"))
            for line in dictionary_bytes.decode(
                encoding=encoding,
                errors="replace",
            ).splitlines()[1:]
        }
    except OSError:
        return _missing_hunspell_snapshot(locale)
    words = tuple(
        sorted(
            word
            for word in candidates
            if MINIMUM_RUNTIME_TOKEN_LENGTH <= len(word) <= 24
            and word.isalpha()
        )
    )
    return HunspellDictionarySnapshot(
        words=words,
        provenance=HunspellDictionaryProvenance(
            locale=locale,
            dictionary=FrozenExternalFile(
                path=str(dictionary_path),
                sha256=hashlib.sha256(dictionary_bytes).hexdigest(),
                bytes=len(dictionary_bytes),
            ),
            affix=FrozenExternalFile(
                path=str(affix_path),
                sha256=hashlib.sha256(affix_bytes).hexdigest(),
                bytes=len(affix_bytes),
            ),
        ),
    )


def _discover_hunspell_snapshot(locale: str) -> HunspellDictionarySnapshot:
    """Snapshot files before Hunspell opens them, using runtime discovery order."""

    discovered = HunspellDictionary._find_dictionary(locale)
    if discovered is None:
        return _missing_hunspell_snapshot(locale)
    affix_path, dictionary_path = discovered
    return _hunspell_snapshot_from_paths(
        locale,
        dictionary_path=dictionary_path,
        affix_path=affix_path,
    )


def _hunspell_dictionary_words(model: LanguageModel) -> tuple[str, ...]:
    """Compatibility wrapper for focused parser tests."""

    locale = str(getattr(model, "locale", "unknown"))
    return _hunspell_dictionary_snapshot(model, locale).words


def load_hunspell_dictionary_snapshots() -> dict[int, HunspellDictionarySnapshot]:
    """Take one bounded immutable snapshot per locale for the whole evaluation."""

    return {
        group: _hunspell_dictionary_snapshot(LanguageModel.load(locale), locale)
        for group, locale in ((0, "en_US"), (1, "ru_RU"))
    }


def _dictionary_snapshots(
    provided: Mapping[int, HunspellDictionarySnapshot] | None,
) -> dict[int, HunspellDictionarySnapshot]:
    snapshots = (
        load_hunspell_dictionary_snapshots()
        if provided is None
        else dict(provided)
    )
    if set(snapshots) != {0, 1}:
        raise ValueError("Hunspell snapshots must contain exactly groups 0 and 1")
    return snapshots


def external_corpus_sha256(examples: Sequence[LexicalExample]) -> str:
    """Fingerprint the exact generated external rows, independent of file paths."""

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


def build_lexical_disjoint_corpus(
    onboard_words: Mapping[int, set[str]],
    *,
    minimum_words_per_group: int = 5000,
    hunspell_snapshots: Mapping[int, HunspellDictionarySnapshot] | None = None,
) -> LexicalDisjointCorpus:
    if minimum_words_per_group < 1:
        raise ValueError("minimum_words_per_group must be positive")
    pair = LayoutPair()
    examples: list[LexicalExample] = []
    counts: dict[int, int] = {}
    sources: dict[int, str] = {}
    snapshots = _dictionary_snapshots(hunspell_snapshots)
    for group, locale in ((0, "en_US"), (1, "ru_RU")):
        snapshot = snapshots[group]
        if snapshot.provenance.locale != locale:
            raise ValueError(f"Hunspell snapshot group {group} must be {locale}")
        sources[group] = snapshot.provenance.dictionary.path
        disjoint = tuple(
            word
            for word in snapshot.words
            if word not in onboard_words.get(group, set())
            and physical_signature(word, group)
        )
        selected = _deterministic_sample(disjoint, minimum_words_per_group)
        counts[group] = len(selected)
        for word in selected:
            if group == 0:
                wrong = pair.translate(word, "us", "ru")
            else:
                wrong = pair.translate(word, "ru", "us")
            signature = "hunspell:" + physical_signature(word, group)
            examples.append(
                LexicalExample(
                    original=word,
                    alternative=wrong,
                    source_group=group,
                    target_group=1 - group,
                    trigger="space",
                    label=False,
                    weight=1.0,
                    base_signature=signature,
                    variant_kind="hunspell-disjoint",
                    source_known=False,
                    target_known=False,
                )
            )
            examples.append(
                LexicalExample(
                    original=wrong,
                    alternative=word,
                    source_group=1 - group,
                    target_group=group,
                    trigger="space",
                    label=True,
                    weight=1.0,
                    base_signature=signature,
                    variant_kind="hunspell-disjoint",
                    source_known=False,
                    target_known=False,
                )
            )
    frozen_examples = tuple(examples)
    return LexicalDisjointCorpus(
        frozen_examples,
        counts,
        sources,
        minimum_words_per_group,
        corpus_sha256=external_corpus_sha256(frozen_examples),
        dictionary_provenance={
            group: snapshots[group].provenance for group in (0, 1)
        },
    )


def select_source_known_negative_examples(
    examples: Sequence[LexicalExample],
    *,
    language_models: Mapping[int, WordScorer],
) -> tuple[LexicalExample, ...]:
    """Derive a two-direction hard-guard corpus from actual runtime answers.

    A parsed Hunspell ``.dic`` stem is not necessarily accepted by a live
    handle: affix flags can make an entry incomplete or forbidden on its own.
    Keep the frozen external corpus unchanged, but admit a row to this
    production-policy slice only after the serving scorer reports the source
    spelling as known.
    """

    if set(language_models) != {0, 1}:
        raise ValueError("source-known language models must be groups 0 and 1")
    negatives = tuple(example for example in examples if not example.label)
    if not negatives:
        raise ValueError("source-known candidates must contain negative rows")
    if any(example.source_group not in {0, 1} for example in negatives):
        raise ValueError("source-known candidates contain an invalid source group")
    selected = tuple(
        replace(example, source_known=True)
        for example in negatives
        if language_models[example.source_group].score(example.original).known
    )
    if {example.source_group for example in selected} != {0, 1}:
        raise ValueError(
            "source-known runtime subset must cover groups 0 and 1"
        )
    return selected


def _render_physical_signature(
    signature: str, group: int, pair: LayoutPair
) -> str:
    if group == 0:
        return signature
    if group == 1:
        return pair.translate(signature, "us", "ru")
    raise ValueError("only EN/RU groups 0 and 1 are supported")


def build_unknown_typo_disjoint_corpus(
    onboard_words: Mapping[int, set[str]],
    *,
    sealed_physical_signatures: Collection[str],
    minimum_words_per_group: int = 5000,
    hunspell_snapshots: Mapping[int, HunspellDictionarySnapshot] | None = None,
    language_models: Mapping[int, LanguageModel] | None = None,
    rank_namespace: str = UNKNOWN_TYPO_DEVELOPMENT_RANK_NAMESPACE,
    choice_namespace: str = UNKNOWN_TYPO_DEVELOPMENT_CHOICE_NAMESPACE,
) -> LexicalDisjointCorpus:
    """Build real serving-domain negatives where neither decoding is known.

    Each base word comes from Hunspell but not the Onboard training lexicon. A
    deterministic physical-key typo is selected only when both its correct-
    layout and wrong-layout renderings are unknown to the real runtime scorers.
    The symmetric negative/positive rows therefore reach the intent model
    instead of being intercepted by the valid-source hard guard.
    """

    if minimum_words_per_group < 1:
        raise ValueError("minimum_words_per_group must be positive")
    for label, namespace in (
        ("rank_namespace", rank_namespace),
        ("choice_namespace", choice_namespace),
    ):
        if (
            not namespace
            or len(namespace) > 128
            or "\0" in namespace
            or not namespace.isascii()
        ):
            raise ValueError(f"{label} must be non-empty bounded ASCII")
    if rank_namespace == choice_namespace:
        raise ValueError("rank and choice namespaces must be distinct")
    excluded_signatures = frozenset(sealed_physical_signatures)
    exclusion_sha256 = physical_signature_set_sha256(excluded_signatures)
    pair = LayoutPair()
    models = (
        {
            0: LanguageModel.load("en_US"),
            1: LanguageModel.load("ru_RU"),
        }
        if language_models is None
        else dict(language_models)
    )
    if set(models) != {0, 1}:
        raise ValueError("language models must contain exactly groups 0 and 1")
    snapshots = (
        {
            group: _hunspell_dictionary_snapshot(models[group], locale)
            for group, locale in ((0, "en_US"), (1, "ru_RU"))
        }
        if hunspell_snapshots is None
        else _dictionary_snapshots(hunspell_snapshots)
    )
    examples: list[LexicalExample] = []
    counts: dict[int, int] = {}
    sources: dict[int, str] = {}
    rejected_overlaps: dict[int, int] = {}
    rank_namespace_bytes = rank_namespace.encode("ascii") + b"\0"
    choice_namespace_bytes = choice_namespace.encode("ascii") + b"\0"
    used_physical_typos: set[str] = set()
    for group in (0, 1):
        model = models[group]
        expected_locale = "en_US" if group == 0 else "ru_RU"
        snapshot = snapshots[group]
        if snapshot.provenance.locale != expected_locale:
            raise ValueError(
                f"Hunspell snapshot group {group} must be {expected_locale}"
            )
        sources[group] = snapshot.provenance.dictionary.path
        candidates = tuple(
            word
            for word in snapshot.words
            if word not in onboard_words.get(group, set())
            and physical_signature(word, group)
        )
        ordered = sorted(
            candidates,
            key=lambda word: hashlib.sha256(
                rank_namespace_bytes
                + physical_signature(word, group).encode("utf-8")
            ).digest(),
        )
        selected: list[tuple[str, str, str, str, bytes]] = []
        rejected_overlap_count = 0
        for word in ordered:
            signature = physical_signature(word, group)
            digest = hashlib.sha256(
                choice_namespace_bytes + signature.encode("utf-8")
            ).digest()
            variants = list(typo_variants(signature, 3)[1:])
            if not variants:
                continue
            rotation = digest[0] % len(variants)
            variants = variants[rotation:] + variants[:rotation]
            for variant in variants:
                if variant.physical_signature in excluded_signatures:
                    rejected_overlap_count += 1
                    continue
                if variant.physical_signature in used_physical_typos:
                    continue
                correct_typo = _render_physical_signature(
                    variant.physical_signature, group, pair
                )
                wrong_typo = _render_physical_signature(
                    variant.physical_signature, 1 - group, pair
                )
                if (
                    max(
                        len(LanguageModel.normalize(correct_typo)),
                        len(LanguageModel.normalize(wrong_typo)),
                    )
                    < MINIMUM_RUNTIME_TOKEN_LENGTH
                    or LanguageDetector.is_protected_token(correct_typo)
                    or LanguageDetector.is_protected_token(wrong_typo)
                    or models[group].score(correct_typo).known
                    or models[1 - group].score(wrong_typo).known
                ):
                    continue
                selected.append(
                    (
                        variant.physical_signature,
                        correct_typo,
                        wrong_typo,
                        variant.kind,
                        digest,
                    )
                )
                used_physical_typos.add(variant.physical_signature)
                break
            if len(selected) >= minimum_words_per_group:
                break
        rejected_overlaps[group] = rejected_overlap_count
        counts[group] = len(selected)
        for physical_typo, correct_typo, wrong_typo, variant_kind, _digest in selected:
            signature = "hunspell-unknown:" + physical_typo
            for trigger in TRIGGERS:
                examples.extend(
                    (
                        LexicalExample(
                            original=correct_typo,
                            alternative=wrong_typo,
                            source_group=group,
                            target_group=1 - group,
                            trigger=trigger,
                            label=False,
                            weight=1.0,
                            base_signature=signature,
                            variant_kind=f"hunspell-unknown-{variant_kind}",
                            source_known=False,
                            target_known=False,
                        ),
                        LexicalExample(
                            original=wrong_typo,
                            alternative=correct_typo,
                            source_group=1 - group,
                            target_group=group,
                            trigger=trigger,
                            label=True,
                            weight=1.0,
                            base_signature=signature,
                            variant_kind=f"hunspell-unknown-{variant_kind}",
                            source_known=False,
                            target_known=False,
                        ),
                    )
                )
    selected_overlap_count = len(used_physical_typos & excluded_signatures)
    if selected_overlap_count:
        raise RuntimeError(
            "unknown-typo corpus contains a sealed physical signature after filtering"
        )
    frozen_examples = tuple(examples)
    return LexicalDisjointCorpus(
        examples=frozen_examples,
        words_by_group=counts,
        dictionary_sources=sources,
        minimum_words_per_group=minimum_words_per_group,
        rejected_sealed_overlaps_by_group=rejected_overlaps,
        exclusion_signature_count=len(excluded_signatures),
        exclusion_signature_sha256=exclusion_sha256,
        selected_sealed_overlap_count=selected_overlap_count,
        corpus_sha256=external_corpus_sha256(frozen_examples),
        rank_namespace=rank_namespace,
        choice_namespace=choice_namespace,
        dictionary_provenance={
            group: snapshots[group].provenance for group in (0, 1)
        },
    )


def unknown_typo_physical_signatures(
    corpus: LexicalDisjointCorpus,
) -> frozenset[str]:
    """Return the exact selected physical typo domain after shape validation."""

    prefix = "hunspell-unknown:"
    signatures: set[str] = set()
    rows_by_signature: dict[str, list[LexicalExample]] = {}
    for example in corpus.examples:
        if not example.base_signature.startswith(prefix):
            raise ValueError("unknown-typo corpus contains a foreign signature")
        physical = example.base_signature.removeprefix(prefix)
        if not physical:
            raise ValueError("unknown-typo corpus contains an empty signature")
        signatures.add(physical)
        rows_by_signature.setdefault(physical, []).append(example)
    expected_rows = len(TRIGGERS) * 2
    if not signatures or any(
        len(rows) != expected_rows for rows in rows_by_signature.values()
    ):
        raise ValueError("unknown-typo corpus is not trigger-symmetric")
    return frozenset(signatures)


def compare_with_fallback(
    model: LinearNgramModel,
    examples: Sequence[LexicalExample],
    requested: int,
    *,
    language_models: Mapping[int, LanguageModel] | None = None,
    workers: int | None = None,
) -> PredictionComparison:
    sample = _deterministic_sample(examples, requested)
    scorers = (
        {
            0: LanguageModel.load("en_US"),
            1: LanguageModel.load("ru_RU"),
        }
        if language_models is None
        else dict(language_models)
    )
    if set(scorers) != {0, 1}:
        raise ValueError("language models must contain exactly groups 0 and 1")
    detector = LanguageDetector(scorers, model)
    fallback_rows: list[tuple[bool, bool]] = []
    ensemble_rows: list[tuple[bool, bool]] = []
    rescued = vetoed = prevented_fp = introduced_fp = 0
    model_evaluated = negative_model_evaluated = 0
    actual_workers = _effective_row_workers(workers, len(sample))
    decisions: list[tuple[bool, bool, bool]]
    if actual_workers > 1:
        decisions = _map_row_chunks(
            _fallback_rows_worker,
            _RowWorkload(examples=sample, detector=detector),
            actual_workers,
        )
    else:
        decisions = []
        for example in sample:
            alternatives = {example.target_group: example.alternative}
            fallback_decision = detector.decide(
                example.original,
                alternatives,
                example.source_group,
                trigger=example.trigger,
                use_intent_model=False,
            ).should_convert
            ensemble_decision = detector.decide(
                example.original,
                alternatives,
                example.source_group,
                trigger=example.trigger,
                use_intent_model=True,
            )
            decisions.append(
                (
                    fallback_decision,
                    ensemble_decision.should_convert,
                    ensemble_decision.model_probability is not None,
                )
            )
    for example, (fallback, ensemble, evaluated) in zip(
        sample, decisions, strict=True
    ):
        if evaluated:
            model_evaluated += 1
            if not example.label:
                negative_model_evaluated += 1
        fallback_rows.append((example.label, fallback))
        ensemble_rows.append((example.label, ensemble))
        if example.label and not fallback and ensemble:
            rescued += 1
        elif example.label and fallback and not ensemble:
            vetoed += 1
        elif not example.label and fallback and not ensemble:
            prevented_fp += 1
        elif not example.label and not fallback and ensemble:
            introduced_fp += 1
    return PredictionComparison(
        _confusion(fallback_rows),
        _confusion(ensemble_rows),
        rescued,
        vetoed,
        prevented_fp,
        introduced_fp,
        len(sample),
        model_evaluated,
        negative_model_evaluated,
    )


@dataclass(frozen=True)
class _FixedContextLanguageScorer:
    """Delegate lexical evidence while forcing one reachable context profile."""

    delegate: LanguageScorer
    profile: ProductionContextProfile

    def score(self, word: str) -> WordScore:
        return self.delegate.score(word)

    def context_score(self, previous: str, word: str) -> float:
        del word
        if previous == _SOURCE_CONTEXT_SENTINEL:
            return self.profile.source_context
        if previous == _TARGET_CONTEXT_SENTINEL:
            return self.profile.target_context
        raise ValueError("production context scorer received an unknown sentinel")

    def best_single_deletion(self, word: str) -> WordScore:
        return self.delegate.best_single_deletion(word)


class _CorpusLanguageScorerCache:
    """Avoid seven-pass LanguageModel LRU thrashing on large sealed corpora."""

    def __init__(self, delegate: LanguageScorer) -> None:
        self._delegate = delegate
        self._scores: dict[str, WordScore] = {}
        self._deletions: dict[str, WordScore] = {}

    def score(self, word: str) -> WordScore:
        cached = self._scores.get(word)
        if cached is not None:
            return cached
        result = self._delegate.score(word)
        self._scores[word] = result
        return result

    def context_score(self, previous: str, word: str) -> float:
        return self._delegate.context_score(previous, word)

    def best_single_deletion(self, word: str) -> WordScore:
        cached = self._deletions.get(word)
        if cached is not None:
            return cached
        result = self._delegate.best_single_deletion(word)
        self._deletions[word] = result
        return result


class _ContextInvariantIntentModel:
    """Reuse exact feature-schema-v5 predictions after proving invariance."""

    def __init__(self, delegate: LinearNgramModel) -> None:
        self._delegate = delegate
        self._cache: dict[IntentModelInput, LinearPrediction] = {}
        self.cache_hits = 0

    @property
    def veto_threshold(self) -> float:
        return self._delegate.veto_threshold

    @property
    def unique_predictions(self) -> int:
        return len(self._cache)

    def predict(self, item: IntentModelInput) -> LinearPrediction:
        neutral = IntentModelInput(
            original=item.original,
            alternative=item.alternative,
            source_group=item.source_group,
            target_group=item.target_group,
            trigger=item.trigger,
            source_score=item.source_score,
            target_score=item.target_score,
        )
        cached = self._cache.get(neutral)
        if cached is not None:
            self.cache_hits += 1
            return cached
        prediction = self._delegate.predict(neutral)
        self._cache[neutral] = prediction
        return prediction

    def replay(self, neutral: IntentModelInput, prediction: LinearPrediction) -> None:
        """Account for a call made on a worker exactly as ``predict`` would."""

        if neutral in self._cache:
            self.cache_hits += 1
        else:
            self._cache[neutral] = prediction


# ---------------------------------------------------------------------------
# Parallel row scoring
# ---------------------------------------------------------------------------
#
# Every row decision below is a pure function of the row and of read-only
# model/scorer objects, so rows are scored on forked worker processes and
# reassembled in the original order; no decision, logit or coverage changes.
# The one stateful object, the context-invariant prediction cache, is replayed
# in the parent from the model calls each worker recorded, so its reported
# counters stay exactly what a sequential run produces.

_DEFAULT_ROW_WORKERS = 1
_R = TypeVar("_R")


def set_default_row_workers(workers: int) -> None:
    """Set the worker count used when a scoring call does not name one."""

    global _DEFAULT_ROW_WORKERS
    if workers < 1:
        raise ValueError("row scoring needs at least one worker")
    _DEFAULT_ROW_WORKERS = workers


def _effective_row_workers(requested: int | None, rows: int) -> int:
    workers = _DEFAULT_ROW_WORKERS if requested is None else requested
    if workers < 1:
        raise ValueError("row scoring needs at least one worker")
    return max(1, min(workers, rows))


class _RecordingIntentModel:
    """Forward to a classifier and record every call for parent-side replay."""

    def __init__(
        self, delegate: LinearNgramModel | _ContextInvariantIntentModel
    ) -> None:
        self._delegate = delegate
        self.calls: list[tuple[IntentModelInput, LinearPrediction]] = []

    @property
    def veto_threshold(self) -> float:
        return self._delegate.veto_threshold

    def predict(self, item: IntentModelInput) -> LinearPrediction:
        prediction = self._delegate.predict(item)
        neutral = IntentModelInput(
            original=item.original,
            alternative=item.alternative,
            source_group=item.source_group,
            target_group=item.target_group,
            trigger=item.trigger,
            source_score=item.source_score,
            target_score=item.target_score,
        )
        self.calls.append((neutral, prediction))
        return prediction


@dataclass(frozen=True)
class _RowWorkload:
    examples: Sequence[LexicalExample]
    model: LinearNgramModel | _ContextInvariantIntentModel | None = None
    scorers: Mapping[int, WordScorer] | None = None
    detector: LanguageDetector | None = None
    profile: ProductionContextProfile | None = None
    recorder: _RecordingIntentModel | None = None


_ROW_WORKLOAD: _RowWorkload | None = None


def _initialize_row_worker(workload: _RowWorkload) -> None:
    global _ROW_WORKLOAD
    _ROW_WORKLOAD = workload


def _row_workload() -> _RowWorkload:
    if _ROW_WORKLOAD is None:
        raise RuntimeError("row scoring worker was not initialised")
    return _ROW_WORKLOAD


def _map_row_chunks(
    worker: Callable[[tuple[int, int]], tuple[int, list[_R]]],
    workload: _RowWorkload,
    workers: int,
) -> list[_R]:
    """Score contiguous row ranges on worker processes, keeping row order."""

    ranges = _balanced_ranges(len(workload.examples), workers)
    start_method = (
        "fork"
        if "fork" in multiprocessing.get_all_start_methods()
        else "spawn"
    )
    context = multiprocessing.get_context(start_method)
    with ProcessPoolExecutor(
        max_workers=len(ranges),
        mp_context=context,
        initializer=_initialize_row_worker,
        initargs=(workload,),
    ) as executor:
        chunks = tuple(executor.map(worker, ranges))
    if tuple(chunk[0] for chunk in chunks) != tuple(start for start, _stop in ranges):
        raise RuntimeError("parallel row scoring changed chunk order")
    results = [item for _start, items in chunks for item in items]
    if len(results) != len(workload.examples):
        raise RuntimeError("parallel row scoring lost rows")
    return results


def _predict_rows_worker(
    bounds: tuple[int, int],
) -> tuple[int, list[tuple[bool, float, float]]]:
    workload = _row_workload()
    if workload.model is None or workload.scorers is None:
        raise RuntimeError("prediction worker lacks a model or scorers")
    start, stop = bounds
    results: list[tuple[bool, float, float]] = []
    for example in workload.examples[start:stop]:
        prediction = workload.model.predict(
            intent_input_for_example(example, scorers=workload.scorers)
        )
        results.append(
            (prediction.should_switch, prediction.logit, prediction.coverage)
        )
    return start, results


def _fallback_rows_worker(
    bounds: tuple[int, int],
) -> tuple[int, list[tuple[bool, bool, bool]]]:
    workload = _row_workload()
    detector = workload.detector
    if detector is None:
        raise RuntimeError("comparison worker lacks a detector")
    start, stop = bounds
    results: list[tuple[bool, bool, bool]] = []
    for example in workload.examples[start:stop]:
        alternatives = {example.target_group: example.alternative}
        fallback = detector.decide(
            example.original,
            alternatives,
            example.source_group,
            trigger=example.trigger,
            use_intent_model=False,
        ).should_convert
        ensemble_decision = detector.decide(
            example.original,
            alternatives,
            example.source_group,
            trigger=example.trigger,
            use_intent_model=True,
        )
        results.append(
            (
                fallback,
                ensemble_decision.should_convert,
                ensemble_decision.model_probability is not None,
            )
        )
    return start, results


_ContextRowResult = tuple[
    bool, bool, bool, float, tuple[tuple[IntentModelInput, LinearPrediction], ...]
]


def _context_profile_rows_worker(
    bounds: tuple[int, int],
) -> tuple[int, list[_ContextRowResult]]:
    workload = _row_workload()
    detector = workload.detector
    profile = workload.profile
    recorder = workload.recorder
    if detector is None or profile is None or recorder is None:
        raise RuntimeError("context worker lacks a detector, profile or recorder")
    start, stop = bounds
    results: list[_ContextRowResult] = []
    for example in workload.examples[start:stop]:
        recorder.calls.clear()
        previous_words = {
            example.source_group: _SOURCE_CONTEXT_SENTINEL,
            example.target_group: _TARGET_CONTEXT_SENTINEL,
        }
        context_group = profile.context_group(
            example.source_group, example.target_group
        )
        observed_delta = detector._context_delta(
            example.source_group,
            example.target_group,
            example.original,
            example.alternative,
            previous_words,
            context_group,
        )
        alternatives = {example.target_group: example.alternative}
        fallback = detector.decide(
            example.original,
            alternatives,
            example.source_group,
            previous_words=previous_words,
            context_group=context_group,
            trigger=example.trigger,
            use_intent_model=False,
        ).should_convert
        ensemble_decision = detector.decide(
            example.original,
            alternatives,
            example.source_group,
            previous_words=previous_words,
            context_group=context_group,
            trigger=example.trigger,
            use_intent_model=True,
        )
        results.append(
            (
                fallback,
                ensemble_decision.should_convert,
                ensemble_decision.model_probability is not None,
                observed_delta,
                tuple(recorder.calls),
            )
        )
    return start, results


def _prediction_comparison_from_policy_rows(
    rows: Sequence[ProductionPolicyRow],
) -> PredictionComparison:
    fallback_rows = tuple(
        (row.example.label, row.fallback) for row in rows
    )
    ensemble_rows = tuple(
        (row.example.label, row.ensemble) for row in rows
    )
    return PredictionComparison(
        fallback=_confusion(fallback_rows),
        ensemble=_confusion(ensemble_rows),
        rescued_true_positives=sum(
            row.example.label and not row.fallback and row.ensemble
            for row in rows
        ),
        vetoed_true_positives=sum(
            row.example.label and row.fallback and not row.ensemble
            for row in rows
        ),
        prevented_false_positives=sum(
            not row.example.label and row.fallback and not row.ensemble
            for row in rows
        ),
        introduced_false_positives=sum(
            not row.example.label and not row.fallback and row.ensemble
            for row in rows
        ),
        samples=len(rows),
        model_evaluated_samples=sum(row.model_evaluated for row in rows),
        negative_model_evaluated=sum(
            row.model_evaluated and not row.example.label for row in rows
        ),
    )


def _production_context_corpus_evaluation(
    rows: Sequence[ProductionPolicyRow],
) -> ProductionContextCorpusEvaluation:
    frozen = tuple(rows)
    return ProductionContextCorpusEvaluation(
        examples=tuple(row.example for row in frozen),
        negative_ensemble_decisions=bytes(
            row.ensemble for row in frozen if not row.example.label
        ),
        overall=_prediction_comparison_from_policy_rows(frozen),
        per_trigger={
            trigger: _prediction_comparison_from_policy_rows(
                tuple(row for row in frozen if row.example.trigger == trigger)
            )
            for trigger in TRIGGERS
            if any(row.example.trigger == trigger for row in frozen)
        },
    )


def _evaluate_production_context_profile_rows(
    model: LinearNgramModel | _ContextInvariantIntentModel,
    examples: Sequence[LexicalExample],
    profile: ProductionContextProfile,
    *,
    language_models: Mapping[int, LanguageScorer],
    workers: int | None = None,
) -> tuple[ProductionPolicyRow, ...]:
    fixed_scorers: dict[int, LanguageScorer] = {
        group: _FixedContextLanguageScorer(scorer, profile)
        for group, scorer in language_models.items()
    }
    examples = tuple(examples)
    actual_workers = _effective_row_workers(workers, len(examples))
    if actual_workers > 1:
        recorder = _RecordingIntentModel(model)
        outputs = _map_row_chunks(
            _context_profile_rows_worker,
            _RowWorkload(
                examples=examples,
                model=model,
                detector=LanguageDetector(fixed_scorers, recorder),
                profile=profile,
                recorder=recorder,
            ),
            actual_workers,
        )
        parallel_rows: list[ProductionPolicyRow] = []
        for example, (
            fallback_decision,
            ensemble_decision_value,
            evaluated,
            observed,
            calls,
        ) in zip(examples, outputs, strict=True):
            if isinstance(model, _ContextInvariantIntentModel):
                for neutral, prediction in calls:
                    model.replay(neutral, prediction)
            parallel_rows.append(
                ProductionPolicyRow(
                    example=example,
                    fallback=fallback_decision,
                    ensemble=ensemble_decision_value,
                    model_evaluated=evaluated,
                    observed_context_delta=observed,
                )
            )
        return tuple(parallel_rows)
    detector = LanguageDetector(fixed_scorers, model)
    rows: list[ProductionPolicyRow] = []
    for example in examples:
        previous_words = {
            example.source_group: _SOURCE_CONTEXT_SENTINEL,
            example.target_group: _TARGET_CONTEXT_SENTINEL,
        }
        context_group = profile.context_group(
            example.source_group, example.target_group
        )
        observed_delta = detector._context_delta(
            example.source_group,
            example.target_group,
            example.original,
            example.alternative,
            previous_words,
            context_group,
        )
        alternatives = {example.target_group: example.alternative}
        fallback = detector.decide(
            example.original,
            alternatives,
            example.source_group,
            previous_words=previous_words,
            context_group=context_group,
            trigger=example.trigger,
            use_intent_model=False,
        ).should_convert
        ensemble_decision = detector.decide(
            example.original,
            alternatives,
            example.source_group,
            previous_words=previous_words,
            context_group=context_group,
            trigger=example.trigger,
            use_intent_model=True,
        )
        rows.append(
            ProductionPolicyRow(
                example=example,
                fallback=fallback,
                ensemble=ensemble_decision.should_convert,
                model_evaluated=(
                    ensemble_decision.model_probability is not None
                ),
                observed_context_delta=observed_delta,
            )
        )
    return tuple(rows)


def evaluate_production_context_ensemble(
    model: LinearNgramModel,
    *,
    sealed_test: Sequence[LexicalExample],
    unknown_typo: Sequence[LexicalExample],
    safety: Sequence[LexicalExample],
    source_known: Sequence[LexicalExample],
    language_models: Mapping[int, LanguageModel],
    use_prediction_cache: bool = True,
) -> ProductionContextEnsembleEvaluation:
    """Exercise the real detector at every role-relative context endpoint."""

    if set(language_models) != {0, 1}:
        raise ValueError("production context language models must be groups 0 and 1")
    if any(example.label or not example.safety for example in safety):
        raise ValueError("production context safety rows must be safety negatives")
    if any(
        example.label
        or not language_models[example.source_group].score(
            example.original
        ).known
        for example in source_known
    ):
        raise ValueError(
            "production context source-known rows must be source-known negatives"
        )
    verify_context_feature_invariance(
        dimension=model.dimension,
        extractor=runtime_feature_extractor(
            model.fnv_seed,
            model.membership_seed,
            scorers=language_models,
        ),
    )
    primary_corpora: dict[str, tuple[LexicalExample, ...]] = {
        "sealed_test": tuple(sealed_test),
        "unknown_typo": tuple(unknown_typo),
        "safety": tuple(safety),
        "source_known": tuple(source_known),
    }
    profile_corpora: dict[
        str, dict[str, ProductionContextCorpusEvaluation]
    ] = {profile.name: {} for profile in PRODUCTION_CONTEXT_PROFILES}
    observed_by_profile: dict[str, dict[str, set[float]]] = {
        profile.name: {} for profile in PRODUCTION_CONTEXT_PROFILES
    }
    unique_predictions = 0
    cache_hits = 0
    for corpus_name, examples in primary_corpora.items():
        if not examples:
            raise ValueError(
                f"production context corpus {corpus_name!r} must not be empty"
            )
        cache = _ContextInvariantIntentModel(model)
        corpus_language_models: dict[int, LanguageScorer] = {
            group: _CorpusLanguageScorerCache(language_model)
            for group, language_model in language_models.items()
        }
        classifier: LinearNgramModel | _ContextInvariantIntentModel = (
            cache if use_prediction_cache else model
        )
        for profile in PRODUCTION_CONTEXT_PROFILES:
            rows = _evaluate_production_context_profile_rows(
                classifier,
                examples,
                profile,
                language_models=corpus_language_models,
            )
            profile_corpora[profile.name][corpus_name] = (
                _production_context_corpus_evaluation(rows)
            )
            for row in rows:
                direction = (
                    f"{row.example.source_group}_to_"
                    f"{row.example.target_group}"
                )
                observed_by_profile[profile.name].setdefault(
                    direction, set()
                ).add(row.observed_context_delta)
            if corpus_name == "sealed_test":
                profile_corpora[profile.name]["sealed_test_typos"] = (
                    _production_context_corpus_evaluation(
                        tuple(
                            row
                            for row in rows
                            if row.example.variant_kind in TYPO_VARIANT_KINDS
                        )
                    )
                )
        if use_prediction_cache:
            unique_predictions += cache.unique_predictions
            cache_hits += cache.cache_hits
    profiles: dict[str, ProductionContextProfileEvaluation] = {}
    for profile in PRODUCTION_CONTEXT_PROFILES:
        corpora = profile_corpora[profile.name]
        profiles[profile.name] = ProductionContextProfileEvaluation(
            profile=profile,
            observed_deltas_by_direction={
                direction: tuple(sorted(values))
                for direction, values in sorted(
                    observed_by_profile[profile.name].items()
                )
            },
            corpora=corpora,
        )
    return ProductionContextEnsembleEvaluation(
        schema_version=1,
        profiles=profiles,
        unique_model_predictions=unique_predictions,
        model_prediction_cache_hits=cache_hits,
    )


def evaluate_safety_policy(
    model: LinearNgramModel,
    examples: Sequence[LexicalExample],
) -> SafetyPolicyEvaluation:
    """Run safety rows through the same guards and ensemble as production."""

    language_models = {
        0: LanguageModel.load("en_US"),
        1: LanguageModel.load("ru_RU"),
    }
    detector = LanguageDetector(language_models, model)
    rows: dict[CorrectionTrigger, list[tuple[bool, bool]]] = {
        trigger: [] for trigger in TRIGGERS
    }
    protected_samples = 0
    lexical_collision_samples = 0
    pre_model_guarded_samples = 0
    expected_guard_samples = 0
    expected_guarded_samples = 0
    model_evaluated_samples = 0
    guard_failure_samples = 0
    reason_counts: dict[str, int] = {}
    for index, example in enumerate(examples):
        if example.label or not example.safety:
            raise ValueError(
                "safety policy corpus must contain only labelled-negative safety rows: "
                f"index={index}, base={example.base_signature!r}"
            )
        if example.variant_kind == "protected":
            if not example.protected:
                raise ValueError(
                    "protected safety row is missing its protected marker: "
                    f"index={index}, base={example.base_signature!r}"
                )
            protected_samples += 1
            protected_guard_matches = LanguageDetector.is_protected_token(
                example.original
            )
            expected_pre_model_guard = True
        elif example.variant_kind == "lexical_collision":
            if example.protected:
                raise ValueError(
                    "lexical-collision safety row cannot be marked protected: "
                    f"index={index}, base={example.base_signature!r}"
                )
            lexical_collision_samples += 1
            protected_guard_matches = True
            expected_pre_model_guard = True
        else:
            raise ValueError(
                "unsupported safety policy row kind: "
                f"index={index}, variant={example.variant_kind!r}"
            )
        decision = detector.decide(
            example.original,
            {example.target_group: example.alternative},
            example.source_group,
            trigger=example.trigger,
            context_group=example.context_group,
            use_intent_model=True,
        )
        model_evaluated = decision.model_probability is not None
        if model_evaluated:
            model_evaluated_samples += 1
        else:
            pre_model_guarded_samples += 1
        if expected_pre_model_guard:
            expected_guard_samples += 1
            if not model_evaluated:
                expected_guarded_samples += 1
        collision_guard_matches = (
            example.variant_kind != "lexical_collision"
            or decision.reason
            in {"обе раскладки дают допустимое слово", "исходное слово допустимо"}
        )
        if (
            decision.should_convert
            or (expected_pre_model_guard and model_evaluated)
            or not protected_guard_matches
            or not collision_guard_matches
        ):
            guard_failure_samples += 1
        reason_counts[decision.reason] = reason_counts.get(decision.reason, 0) + 1
        rows[example.trigger].append((False, decision.should_convert))
    return SafetyPolicyEvaluation(
        per_trigger={
            trigger: _confusion(rows[trigger])
            for trigger in TRIGGERS
        },
        samples=len(examples),
        protected_samples=protected_samples,
        lexical_collision_samples=lexical_collision_samples,
        pre_model_guarded_samples=pre_model_guarded_samples,
        expected_pre_model_guard_samples=expected_guard_samples,
        expected_pre_model_guarded_samples=expected_guarded_samples,
        model_evaluated_samples=model_evaluated_samples,
        guard_failure_samples=guard_failure_samples,
        reason_counts=dict(sorted(reason_counts.items())),
    )


def _percentile(samples: Sequence[float], quantile: float) -> float:
    if not samples:
        raise ValueError("percentile requires samples")
    ordered = sorted(samples)
    index = min(len(ordered) - 1, int(round((len(ordered) - 1) * quantile)))
    return ordered[index]


def _coverage_statistics(rows: Sequence[ModelPredictionRow]) -> CoverageStatistics:
    if not rows:
        raise ValueError("coverage statistics require at least one prediction")
    values = tuple(item.coverage for item in rows)
    if any(not math.isfinite(value) or not 0.0 <= value <= 1.0 for value in values):
        raise ValueError("prediction coverage must be finite and in [0, 1]")
    return CoverageStatistics(
        samples=len(values),
        minimum=min(values),
        p05=_percentile(values, 0.05),
        p25=_percentile(values, 0.25),
        median=statistics.median(values),
        p75=_percentile(values, 0.75),
        p95=_percentile(values, 0.95),
        maximum=max(values),
        mean=statistics.fmean(values),
        zero_coverage_samples=sum(value == 0.0 for value in values),
        full_coverage_samples=sum(value == 1.0 for value in values),
    )


def coverage_diagnostics(
    rows: Sequence[ModelPredictionRow],
) -> dict[str, CoverageStatistics]:
    """Describe OOD membership coverage without turning it into a quality gate."""

    if not rows:
        raise ValueError("coverage diagnostics require at least one prediction")
    slices: dict[str, tuple[ModelPredictionRow, ...]] = {
        "overall": tuple(rows),
        "label_negative": tuple(item for item in rows if not item.example.label),
        "label_positive": tuple(item for item in rows if item.example.label),
        "direction_0_to_1": tuple(
            item
            for item in rows
            if item.example.source_group == 0 and item.example.target_group == 1
        ),
        "direction_1_to_0": tuple(
            item
            for item in rows
            if item.example.source_group == 1 and item.example.target_group == 0
        ),
    }
    for trigger in TRIGGERS:
        slices[f"trigger_{trigger}"] = tuple(
            item for item in rows if item.example.trigger == trigger
        )
    return {
        name: _coverage_statistics(selected)
        for name, selected in slices.items()
        if selected
    }


def latency_report(
    artifact: Path,
    model: LinearNgramModel,
    examples: Sequence[LexicalExample],
    requested: int,
    *,
    scorers: Mapping[int, WordScorer],
) -> dict[str, object]:
    load_samples: list[float] = []
    for _index in range(11):
        started = time.perf_counter_ns()
        loaded = LinearNgramModel.load(artifact)
        load_samples.append((time.perf_counter_ns() - started) / 1_000_000.0)
        if loaded.checksum != model.checksum:
            raise RuntimeError("model checksum changed between repeated loads")
    sample = _deterministic_sample(examples, min(requested, len(examples)))
    evidence = tuple(
        intent_input_for_example(example, scorers=scorers)
        for example in sample
    )
    for item in evidence[: min(100, len(evidence))]:
        model.predict(item)
    inference_samples: list[float] = []
    prediction_digest = hashlib.sha256()
    for item in evidence:
        started = time.perf_counter_ns()
        prediction = model.predict(item)
        inference_samples.append((time.perf_counter_ns() - started) / 1_000_000.0)
        prediction_digest.update(repr(prediction).encode("utf-8"))
    repeated_digest = hashlib.sha256()
    for item in evidence:
        repeated_digest.update(repr(model.predict(item)).encode("utf-8"))
    deterministic = prediction_digest.digest() == repeated_digest.digest()
    return {
        "artifact_bytes": artifact.stat().st_size,
        "load_ms": {
            "median": round(statistics.median(load_samples), 6),
            "p95": round(_percentile(load_samples, 0.95), 6),
            "samples": len(load_samples),
        },
        "inference_ms": {
            "median": round(statistics.median(inference_samples), 6),
            "p95": round(_percentile(inference_samples, 0.95), 6),
            "samples": len(inference_samples),
        },
        "deterministic_predictions": deterministic,
    }


def _comparison_payload(comparison: PredictionComparison) -> dict[str, object]:
    return {
        "samples": comparison.samples,
        "fallback": metrics_payload(comparison.fallback),
        "ensemble": metrics_payload(comparison.ensemble),
        "rescued_true_positives": comparison.rescued_true_positives,
        "vetoed_true_positives": comparison.vetoed_true_positives,
        "prevented_false_positives": comparison.prevented_false_positives,
        "introduced_false_positives": comparison.introduced_false_positives,
        "model_evaluated_samples": comparison.model_evaluated_samples,
        "negative_model_evaluated": comparison.negative_model_evaluated,
    }


def _hunspell_file_matches(
    actual: FrozenExternalFile,
    expected: FrozenExternalFilePolicy,
) -> bool:
    return actual.sha256 == expected.sha256 and actual.bytes == expected.bytes


def _hunspell_provenance_matches(
    actual: Mapping[int, HunspellDictionaryProvenance],
    expected: Mapping[int, HunspellLocalePolicy],
) -> bool:
    if set(actual) != {0, 1} or set(expected) != {0, 1}:
        return False
    for group, locale in ((0, "en_US"), (1, "ru_RU")):
        observed = actual[group]
        pinned = expected[group]
        if (
            observed.locale != locale
            or not _hunspell_file_matches(observed.dictionary, pinned.dictionary)
            or not _hunspell_file_matches(observed.affix, pinned.affix)
        ):
            return False
    return True


def _unknown_typo_trigger_expansion_matches(
    corpus: LexicalDisjointCorpus,
    expected_triggers: Sequence[str],
) -> bool:
    if tuple(expected_triggers) != tuple(TRIGGERS):
        return False
    expected_pairs = {
        (trigger, label) for trigger in TRIGGERS for label in (False, True)
    }
    rows_by_signature: dict[str, list[LexicalExample]] = {}
    for example in corpus.examples:
        rows_by_signature.setdefault(example.base_signature, []).append(example)
    if not rows_by_signature:
        return False
    return all(
        len(rows) == len(expected_pairs)
        and {(row.trigger, row.label) for row in rows} == expected_pairs
        for rows in rows_by_signature.values()
    )


def _context_profile_corpus_gate(
    config: TrainingConfig,
    *,
    profile: ProductionContextProfile,
    corpus_name: str,
    corpus: ProductionContextCorpusEvaluation,
    neutral: ProductionContextCorpusEvaluation,
) -> dict[str, object]:
    expected_triggers: frozenset[CorrectionTrigger] = (
        frozenset({"space"})
        if corpus_name == "source_known"
        else frozenset(TRIGGERS)
    )
    requires_both_labels = corpus_name in {
        "sealed_test",
        "sealed_test_typos",
        "unknown_typo",
    }
    typo_recall = corpus_name in {"sealed_test_typos", "unknown_typo"}
    def shape_is_valid(value: ProductionContextCorpusEvaluation) -> bool:
        positives = sum(example.label for example in value.examples)
        negatives = len(value.examples) - positives
        aggregate = value.overall
        aggregate_valid = (
            aggregate.samples == len(value.examples)
            and aggregate.fallback.true_positive
            + aggregate.fallback.false_negative
            == positives
            and aggregate.fallback.true_negative
            + aggregate.fallback.false_positive
            == negatives
            and aggregate.ensemble.true_positive
            + aggregate.ensemble.false_negative
            == positives
            and aggregate.ensemble.true_negative
            + aggregate.ensemble.false_positive
            == negatives
            and 0
            <= aggregate.negative_model_evaluated
            <= aggregate.model_evaluated_samples
            <= aggregate.samples
            and len(value.negative_ensemble_decisions) == negatives
            and all(
                decision in (0, 1)
                for decision in value.negative_ensemble_decisions
            )
            and sum(value.negative_ensemble_decisions)
            == aggregate.ensemble.false_positive
        )
        per_trigger_samples = 0
        per_trigger_valid = True
        for trigger, comparison in value.per_trigger.items():
            trigger_examples = tuple(
                example
                for example in value.examples
                if example.trigger == trigger
            )
            trigger_positives = sum(
                example.label for example in trigger_examples
            )
            trigger_negatives = len(trigger_examples) - trigger_positives
            per_trigger_samples += comparison.samples
            per_trigger_valid = per_trigger_valid and (
                comparison.samples == len(trigger_examples)
                and comparison.fallback.true_positive
                + comparison.fallback.false_negative
                == trigger_positives
                and comparison.fallback.true_negative
                + comparison.fallback.false_positive
                == trigger_negatives
                and comparison.ensemble.true_positive
                + comparison.ensemble.false_negative
                == trigger_positives
                and comparison.ensemble.true_negative
                + comparison.ensemble.false_positive
                == trigger_negatives
                and 0
                <= comparison.negative_model_evaluated
                <= comparison.model_evaluated_samples
                <= comparison.samples
            )
        return (
            aggregate_valid
            and per_trigger_valid
            and per_trigger_samples == len(value.examples)
        )

    corpus_shape_valid = shape_is_valid(corpus)
    neutral_shape_valid = shape_is_valid(neutral)
    rows_aligned = corpus.examples == neutral.examples
    newly_converted_negative = (
        sum(
            not baseline and current
            for current, baseline in zip(
                corpus.negative_ensemble_decisions,
                neutral.negative_ensemble_decisions,
                strict=True,
            )
        )
        if rows_aligned
        and len(corpus.negative_ensemble_decisions)
        == len(neutral.negative_ensemble_decisions)
        else len(corpus.examples) + 1
    )
    strictest_fpr = min(
        config.pause_threshold_max_false_positive_rate
        if trigger == "pause"
        else config.threshold_max_false_positive_rate
        for trigger in expected_triggers
    )
    overall = corpus.overall
    neutral_overall = neutral.overall
    overall_negative_count = (
        overall.ensemble.true_negative + overall.ensemble.false_positive
    )
    overall_checks: dict[str, bool] = {
        "samples_nonempty": overall.samples > 0,
        "corpus_shape_valid": corpus_shape_valid,
        "neutral_corpus_shape_valid": neutral_shape_valid,
        "rows_aligned_with_neutral": rows_aligned,
        "absolute_precision": (
            overall.ensemble.precision >= config.test_minimum_precision
        ),
        "absolute_specificity": (
            overall.ensemble.specificity >= config.test_minimum_specificity
        ),
        "false_positive_policy": overall_negative_count > 0
        and (
            wilson_upper_bound(
                overall.ensemble.false_positive, overall_negative_count
            )
            <= strictest_fpr
            if requires_both_labels
            else overall.ensemble.false_positive == 0
        ),
        "introduced_false_positive_policy": (
            overall.ensemble.false_positive
            <= overall.fallback.false_positive
            if requires_both_labels
            else overall.introduced_false_positives == 0
        ),
        "ensemble_false_positives_no_more_than_fallback": (
            overall.ensemble.false_positive
            <= overall.fallback.false_positive
        ),
        "no_new_negative_vs_neutral": newly_converted_negative == 0,
        "ensemble_false_positives_no_more_than_neutral": (
            overall.ensemble.false_positive
            <= neutral_overall.ensemble.false_positive
        ),
    }
    trigger_keys_exact = set(corpus.per_trigger) == expected_triggers
    neutral_trigger_keys_exact = set(neutral.per_trigger) == expected_triggers
    trigger_gates: dict[str, object] = {}
    for trigger in sorted(expected_triggers):
        comparison = corpus.per_trigger.get(trigger)
        neutral_comparison = neutral.per_trigger.get(trigger)
        if comparison is None or neutral_comparison is None:
            trigger_gates[trigger] = {"passed": False, "missing": True}
            continue
        ensemble = comparison.ensemble
        positive_count = ensemble.true_positive + ensemble.false_negative
        negative_count = ensemble.true_negative + ensemble.false_positive
        maximum_fpr = (
            config.pause_threshold_max_false_positive_rate
            if trigger == "pause"
            else config.threshold_max_false_positive_rate
        )
        minimum_recall = (
            (
                config.test_minimum_pause_typo_recall
                if trigger == "pause"
                else config.test_minimum_typo_recall
            )
            if typo_recall
            else (
                config.test_minimum_pause_recall
                if trigger == "pause"
                else config.test_minimum_recall
            )
        )
        if requires_both_labels:
            if profile.expected_delta < 0.0:
                recall_passed = (
                    ensemble.recall + 0.005
                    >= comparison.fallback.recall
                )
                recall_policy = "contextual-fallback-minus-0.005"
            else:
                recall_passed = ensemble.recall >= minimum_recall
                recall_policy = "absolute-floor"
        else:
            recall_passed = positive_count == 0
            recall_policy = "negative-only"
        checks = {
            "label_support": (
                positive_count > 0 and negative_count > 0
                if requires_both_labels
                else positive_count == 0 and negative_count > 0
            ),
            "absolute_precision": (
                ensemble.precision >= config.test_minimum_precision
            ),
            "absolute_specificity": (
                ensemble.specificity >= config.test_minimum_specificity
            ),
            "false_positive_policy": negative_count > 0
            and (
                wilson_upper_bound(
                    ensemble.false_positive, negative_count
                )
                <= maximum_fpr
                if requires_both_labels
                else ensemble.false_positive == 0
            ),
            "recall": recall_passed,
            "introduced_false_positive_policy": (
                ensemble.false_positive
                <= comparison.fallback.false_positive
                if requires_both_labels
                else comparison.introduced_false_positives == 0
            ),
            "ensemble_false_positives_no_more_than_fallback": (
                ensemble.false_positive
                <= comparison.fallback.false_positive
            ),
            "ensemble_false_positives_no_more_than_neutral": (
                ensemble.false_positive
                <= neutral_comparison.ensemble.false_positive
            ),
        }
        trigger_gates[trigger] = {
            "passed": all(value is True for value in checks.values()),
            "recall_policy": recall_policy,
            "minimum_recall": minimum_recall,
            "maximum_false_positive_rate": maximum_fpr,
            "checks": checks,
            "comparison": _comparison_payload(comparison),
        }
    reachability_checks: dict[str, bool] = {}
    if corpus_name == "unknown_typo":
        reachability_checks = {
            "all_rows_reach_model": (
                overall.model_evaluated_samples == overall.samples
            ),
            "all_negative_rows_reach_model": (
                overall.negative_model_evaluated * 2 == overall.samples
            ),
        }
    elif corpus_name in {"safety", "source_known"}:
        reachability_checks = {
            "pre_model_guard_blocks_all_rows": (
                overall.model_evaluated_samples == 0
                and overall.negative_model_evaluated == 0
            )
        }
    passed = (
        trigger_keys_exact
        and neutral_trigger_keys_exact
        and all(value is True for value in overall_checks.values())
        and all(value is True for value in reachability_checks.values())
        and all(
            isinstance(value, dict) and value.get("passed") is True
            for value in trigger_gates.values()
        )
    )
    return {
        "passed": passed,
        "expected_triggers": sorted(expected_triggers),
        "all_triggers_present": (
            trigger_keys_exact and neutral_trigger_keys_exact
        ),
        "newly_converted_negative_vs_neutral": newly_converted_negative,
        "overall_checks": overall_checks,
        "reachability_checks": reachability_checks,
        "overall_comparison": _comparison_payload(overall),
        "per_trigger": trigger_gates,
    }


def production_context_ensemble_gate_breakdown(
    config: TrainingConfig,
    evaluation: ProductionContextEnsembleEvaluation,
) -> dict[str, object]:
    """Fail closed on production policy behavior at all reachable extrema."""

    expected_profiles = {
        profile.name: profile for profile in PRODUCTION_CONTEXT_PROFILES
    }
    profiles_exact = set(evaluation.profiles) == set(expected_profiles)
    profile_gates: dict[str, object] = {}
    for name, expected_profile in expected_profiles.items():
        observed = evaluation.profiles.get(name)
        if observed is None:
            profile_gates[name] = {"passed": False, "missing": True}
            continue
        profile_matches = observed.profile == expected_profile
        corpora_exact = set(observed.corpora) == set(_PRODUCTION_CONTEXT_CORPORA)
        directions_exact = set(observed.observed_deltas_by_direction) == {
            "0_to_1",
            "1_to_0",
        }
        extrema_exact = directions_exact and all(
            values == (expected_profile.expected_delta,)
            for values in observed.observed_deltas_by_direction.values()
        )
        neutral_profile = evaluation.profiles.get("neutral")
        corpus_gates: dict[str, object] = {}
        if neutral_profile is not None:
            for corpus_name in _PRODUCTION_CONTEXT_CORPORA:
                corpus = observed.corpora.get(corpus_name)
                neutral = neutral_profile.corpora.get(corpus_name)
                if corpus is None or neutral is None:
                    corpus_gates[corpus_name] = {
                        "passed": False,
                        "missing": True,
                    }
                    continue
                corpus_gates[corpus_name] = _context_profile_corpus_gate(
                    config,
                    profile=expected_profile,
                    corpus_name=corpus_name,
                    corpus=corpus,
                    neutral=neutral,
                )
        profile_passed = (
            profile_matches
            and corpora_exact
            and extrema_exact
            and set(corpus_gates) == set(_PRODUCTION_CONTEXT_CORPORA)
            and all(
                isinstance(value, dict) and value.get("passed") is True
                for value in corpus_gates.values()
            )
        )
        profile_gates[name] = {
            "passed": profile_passed,
            "profile_matches_contract": profile_matches,
            "all_corpora_present": corpora_exact,
            "reachable_extrema_exact": extrema_exact,
            "expected_delta": expected_profile.expected_delta,
            "expected_delta_hex": expected_profile.expected_delta.hex(),
            "group_selector": expected_profile.group_selector,
            "observed_deltas_by_direction": {
                direction: list(values)
                for direction, values in observed.observed_deltas_by_direction.items()
            },
            "corpora": corpus_gates,
        }
    return {
        "passed": evaluation.schema_version == 1
        and profiles_exact
        and all(
            isinstance(value, dict) and value.get("passed") is True
            for value in profile_gates.values()
        ),
        "schema_version": evaluation.schema_version,
        "all_profiles_present": profiles_exact,
        "expected_profiles": sorted(expected_profiles),
        "profiles": profile_gates,
    }


def _production_context_payload(
    evaluation: ProductionContextEnsembleEvaluation,
    gate_breakdown: Mapping[str, object],
) -> dict[str, object]:
    return {
        "schema_version": evaluation.schema_version,
        "bounds": {
            "context_score_minimum": CONTEXT_SCORE_MINIMUM,
            "context_score_maximum": CONTEXT_SCORE_MAXIMUM,
            "delta_multiplier": CONTEXT_DELTA_MULTIPLIER,
            "target_group_bonus": CONTEXT_TARGET_GROUP_BONUS,
            "source_group_penalty": CONTEXT_SOURCE_GROUP_PENALTY,
        },
        "prediction_cache": {
            "context_invariance_proved_before_use": True,
            "unique_model_predictions": evaluation.unique_model_predictions,
            "cache_hits": evaluation.model_prediction_cache_hits,
        },
        "gate_breakdown": dict(gate_breakdown),
        "profiles": {
            name: {
                "source_context": result.profile.source_context,
                "target_context": result.profile.target_context,
                "group_selector": result.profile.group_selector,
                "expected_delta": result.profile.expected_delta,
                "expected_delta_hex": result.profile.expected_delta.hex(),
                "observed_deltas_by_direction": {
                    direction: list(values)
                    for direction, values in result.observed_deltas_by_direction.items()
                },
                "corpora": {
                    corpus_name: {
                        "overall": _comparison_payload(corpus.overall),
                        "per_trigger": {
                            trigger: _comparison_payload(comparison)
                            for trigger, comparison in corpus.per_trigger.items()
                        },
                    }
                    for corpus_name, corpus in result.corpora.items()
                },
            }
            for name, result in evaluation.profiles.items()
        },
    }


def external_corpus_provenance_gate_breakdown(
    *,
    external_policy: ExternalEvaluationPolicy,
    hunspell_handle_snapshot_stable: bool,
    lexical_disjoint: LexicalDisjointCorpus,
    sealed_signature_index: SealedSignatureIndex,
    unknown_typo_development: LexicalDisjointCorpus,
    unknown_typo_disjoint: LexicalDisjointCorpus,
) -> dict[str, bool]:
    """Validate every frozen external input before model metrics are computed."""

    try:
        development_physical_signatures = unknown_typo_physical_signatures(
            unknown_typo_development
        )
        holdout_physical_signatures = unknown_typo_physical_signatures(
            unknown_typo_disjoint
        )
        external_signature_shape_valid = True
    except ValueError:
        development_physical_signatures = frozenset()
        holdout_physical_signatures = frozenset()
        external_signature_shape_valid = False
    holdout_exclusion_signatures = (
        sealed_signature_index.signatures | development_physical_signatures
    )
    hunspell_provenance_matches = (
        lexical_disjoint.dictionary_provenance
        == unknown_typo_development.dictionary_provenance
        == unknown_typo_disjoint.dictionary_provenance
        and lexical_disjoint.dictionary_sources
        == {
            group: item.dictionary.path
            for group, item in lexical_disjoint.dictionary_provenance.items()
        }
        and unknown_typo_disjoint.dictionary_sources
        == {
            group: item.dictionary.path
            for group, item in unknown_typo_disjoint.dictionary_provenance.items()
        }
        and unknown_typo_development.dictionary_sources
        == {
            group: item.dictionary.path
            for group, item in unknown_typo_development.dictionary_provenance.items()
        }
        and _hunspell_provenance_matches(
            lexical_disjoint.dictionary_provenance,
            external_policy.hunspell,
        )
    )
    gates = {
        "external_policy_schema": external_policy.schema_version == 2,
        "external_minimum_corpus_policy": (
            lexical_disjoint.minimum_words_per_group
            == external_policy.minimum_words_per_group
            and unknown_typo_development.minimum_words_per_group
            == external_policy.minimum_words_per_group
            and unknown_typo_disjoint.minimum_words_per_group
            == external_policy.minimum_words_per_group
        ),
        "external_trigger_expansion_policy": (
            _unknown_typo_trigger_expansion_matches(
                unknown_typo_disjoint,
                external_policy.trigger_expansion,
            )
        ),
        "external_hunspell_provenance": hunspell_provenance_matches,
        "hunspell_handle_snapshot_stability": (
            hunspell_handle_snapshot_stable
        ),
        "lexical_disjoint_size": all(
            lexical_disjoint.words_by_group.get(group, 0)
            >= lexical_disjoint.minimum_words_per_group
            for group in (0, 1)
        ),
        "lexical_disjoint_corpus_provenance": (
            lexical_disjoint.corpus_sha256
            == external_corpus_sha256(lexical_disjoint.examples)
            == external_policy.lexical_disjoint_corpus_sha256
        ),
        "unknown_typo_development_provenance": (
            external_signature_shape_valid
            and unknown_typo_development.corpus_sha256
            == external_corpus_sha256(unknown_typo_development.examples)
            == external_policy.unknown_typo_development_corpus_sha256
            and unknown_typo_development.rank_namespace
            == UNKNOWN_TYPO_DEVELOPMENT_RANK_NAMESPACE
            and unknown_typo_development.choice_namespace
            == UNKNOWN_TYPO_DEVELOPMENT_CHOICE_NAMESPACE
            and unknown_typo_development.exclusion_signature_count
            == sealed_signature_index.signature_count
            and unknown_typo_development.exclusion_signature_sha256
            == sealed_signature_index.sha256
            and unknown_typo_development.selected_sealed_overlap_count == 0
        ),
        "unknown_typo_disjoint_size": all(
            unknown_typo_disjoint.words_by_group.get(group, 0)
            >= unknown_typo_disjoint.minimum_words_per_group
            for group in (0, 1)
        ),
        "unknown_typo_holdout_provenance": (
            unknown_typo_disjoint.corpus_sha256
            == external_corpus_sha256(unknown_typo_disjoint.examples)
            == external_policy.unknown_typo_holdout_corpus_sha256
            and unknown_typo_disjoint.rank_namespace
            == UNKNOWN_TYPO_HOLDOUT_RANK_NAMESPACE
            and unknown_typo_disjoint.choice_namespace
            == UNKNOWN_TYPO_HOLDOUT_CHOICE_NAMESPACE
        ),
        "unknown_typo_holdout_disjointness": (
            external_signature_shape_valid
            and unknown_typo_disjoint.exclusion_signature_count
            == len(holdout_exclusion_signatures)
            and unknown_typo_disjoint.exclusion_signature_sha256
            == physical_signature_set_sha256(holdout_exclusion_signatures)
            and unknown_typo_disjoint.selected_sealed_overlap_count == 0
            and development_physical_signatures.isdisjoint(
                holdout_physical_signatures
            )
            and sealed_signature_index.signatures.isdisjoint(
                holdout_physical_signatures
            )
        ),
    }
    if set(gates) != EXTERNAL_CORPUS_PROVENANCE_GATE_NAMES:
        raise AssertionError("external provenance gate schema drifted")
    return gates


def external_corpus_provenance_gates_pass(
    gates: Mapping[str, object],
) -> bool:
    """Accept only the exact external-provenance schema and literal booleans."""

    return set(gates) == EXTERNAL_CORPUS_PROVENANCE_GATE_NAMES and all(
        gates.get(name) is True
        for name in EXTERNAL_CORPUS_PROVENANCE_GATE_NAMES
    )


def _strict_gates(
    *,
    config: TrainingConfig,
    external_policy: ExternalEvaluationPolicy,
    provenance: Sequence[VerificationCheck],
    hunspell_handle_snapshot_stable: bool,
    runtime_threshold_selection_matches: bool,
    test: Mapping[CorrectionTrigger, ConfusionMatrix],
    context_test: Mapping[
        str, Mapping[CorrectionTrigger, ConfusionMatrix]
    ],
    context_test_typos: Mapping[
        str, Mapping[CorrectionTrigger, ConfusionMatrix]
    ],
    safety: SafetyPolicyEvaluation,
    typo_unknown: Mapping[CorrectionTrigger, ConfusionMatrix],
    comparison: PredictionComparison,
    lexical_disjoint: LexicalDisjointCorpus,
    lexical_comparison: PredictionComparison,
    source_known_comparison: PredictionComparison,
    sealed_signature_index: SealedSignatureIndex,
    unknown_typo_development: LexicalDisjointCorpus,
    unknown_typo_disjoint: LexicalDisjointCorpus,
    unknown_typo_comparison: PredictionComparison,
    unknown_typo_raw_model: Mapping[CorrectionTrigger, ConfusionMatrix],
    production_context_gate: Mapping[str, object],
    latency: Mapping[str, object],
    veto_false_negative_rate: float,
) -> dict[str, bool]:
    load = _mapping(latency.get("load_ms"), "latency.load_ms")
    inference = _mapping(latency.get("inference_ms"), "latency.inference_ms")
    load_p95 = load.get("p95")
    inference_p95 = inference.get("p95")
    artifact_bytes = latency.get("artifact_bytes")
    safety_metrics = safety.per_trigger
    safety_false_positives = sum(
        safety_metrics[trigger].false_positive
        for trigger in TRIGGERS
        if trigger in safety_metrics
    )
    safety_expected_guard_bypasses = (
        safety.expected_pre_model_guard_samples
        - safety.expected_pre_model_guarded_samples
    )
    unknown_typo_expected_samples = (
        2
        * sum(unknown_typo_disjoint.words_by_group.values())
        * len(TRIGGERS)
    )
    external_provenance_gates = external_corpus_provenance_gate_breakdown(
        external_policy=external_policy,
        hunspell_handle_snapshot_stable=hunspell_handle_snapshot_stable,
        lexical_disjoint=lexical_disjoint,
        sealed_signature_index=sealed_signature_index,
        unknown_typo_development=unknown_typo_development,
        unknown_typo_disjoint=unknown_typo_disjoint,
    )
    context_gate = context_stress_gate_breakdown(
        config,
        context_test,
        context_test_typos,
        phase="sealed_test",
    )

    def binary_slice_passes(
        metrics: ConfusionMatrix,
        *,
        minimum_recall: float,
        maximum_false_positive_rate: float,
    ) -> bool:
        positive_count = metrics.true_positive + metrics.false_negative
        negative_count = metrics.true_negative + metrics.false_positive
        return (
            positive_count > 0
            and negative_count > 0
            and metrics.precision >= config.test_minimum_precision
            and metrics.recall >= minimum_recall
            and metrics.specificity >= config.test_minimum_specificity
            and wilson_upper_bound(metrics.false_positive, negative_count)
            <= maximum_false_positive_rate
        )

    return {
        "provenance": all(check.passed for check in provenance),
        **external_provenance_gates,
        "runtime_threshold_selection_evidence": (
            runtime_threshold_selection_matches
        ),
        "sealed_test": set(test) == set(TRIGGERS)
        and all(
            binary_slice_passes(
                test[trigger],
                minimum_recall=(
                    config.test_minimum_pause_recall
                    if trigger == "pause"
                    else config.test_minimum_recall
                ),
                maximum_false_positive_rate=(
                    config.pause_threshold_max_false_positive_rate
                    if trigger == "pause"
                    else config.threshold_max_false_positive_rate
                ),
            )
            for trigger in TRIGGERS
        ),
        "sealed_test_context_stress": context_gate.get("passed") is True,
        "safety": safety.samples > 0
        and safety.protected_samples > 0
        and safety.lexical_collision_samples > 0
        and safety.protected_samples + safety.lexical_collision_samples
        == safety.samples
        and safety.pre_model_guarded_samples + safety.model_evaluated_samples
        == safety.samples
        and safety.expected_pre_model_guard_samples > 0
        and safety.expected_pre_model_guard_samples == safety.samples
        and 0
        <= safety.expected_pre_model_guarded_samples
        <= safety.expected_pre_model_guard_samples
        and max(safety_false_positives, safety_expected_guard_bypasses)
        <= safety.guard_failure_samples
        <= safety_false_positives + safety_expected_guard_bypasses
        and safety.guard_failure_samples
        <= config.safety_maximum_guard_failures
        and sum(safety.reason_counts.values()) == safety.samples
        and set(safety_metrics) == set(TRIGGERS)
        and all(
            safety_metrics[trigger].true_positive
            + safety_metrics[trigger].false_negative
            == 0
            and safety_metrics[trigger].true_negative
            + safety_metrics[trigger].false_positive
            > 0
            for trigger in TRIGGERS
        )
        and sum(
            safety_metrics[trigger].true_negative
            + safety_metrics[trigger].false_positive
            for trigger in TRIGGERS
        )
        == safety.samples,
        "typo_unknown_recall": set(typo_unknown) == set(TRIGGERS)
        and all(
            binary_slice_passes(
                typo_unknown[trigger],
                minimum_recall=(
                    config.test_minimum_pause_typo_recall
                    if trigger == "pause"
                    else config.test_minimum_typo_recall
                ),
                maximum_false_positive_rate=(
                    config.pause_threshold_max_false_positive_rate
                    if trigger == "pause"
                    else config.threshold_max_false_positive_rate
                ),
            )
            for trigger in TRIGGERS
        ),
        "veto": veto_false_negative_rate <= config.veto_max_false_negative_rate,
        "fallback_regression": (
            comparison.introduced_false_positives == 0
            and comparison.ensemble.false_positive
            <= comparison.fallback.false_positive
        ),
        "hunspell_hard_guard_regression": (
            source_known_comparison.samples > 0
            and source_known_comparison.model_evaluated_samples == 0
            and source_known_comparison.negative_model_evaluated == 0
            and source_known_comparison.ensemble.false_positive == 0
            and source_known_comparison.introduced_false_positives == 0
        ),
        "lexical_disjoint_recall": (
            lexical_comparison.ensemble.recall + 0.005
            >= lexical_comparison.fallback.recall
        ),
        "unknown_typo_model_evaluated": (
            unknown_typo_comparison.samples == unknown_typo_expected_samples
            and unknown_typo_comparison.model_evaluated_samples
            == unknown_typo_comparison.samples
            and unknown_typo_comparison.negative_model_evaluated * 2
            == unknown_typo_comparison.samples
        ),
        "unknown_typo_false_positives": (
            unknown_typo_comparison.ensemble.false_positive
            <= unknown_typo_comparison.fallback.false_positive
            and unknown_typo_comparison.ensemble.specificity
            >= unknown_typo_comparison.fallback.specificity
        ),
        "unknown_typo_recall": (
            unknown_typo_comparison.ensemble.recall + 0.005
            >= unknown_typo_comparison.fallback.recall
        ),
        "unknown_typo_raw_model_integrity": set(unknown_typo_raw_model)
        == set(TRIGGERS)
        and all(
            unknown_typo_raw_model[trigger].true_positive
            + unknown_typo_raw_model[trigger].false_negative
            > 0
            and unknown_typo_raw_model[trigger].true_negative
            + unknown_typo_raw_model[trigger].false_positive
            > 0
            for trigger in TRIGGERS
        ),
        "production_context_ensemble": (
            production_context_gate.get("passed") is True
        ),
        "artifact_size": isinstance(artifact_bytes, int)
        and artifact_bytes <= MAX_CONTAINER_BYTES,
        "load_latency": isinstance(load_p95, (int, float))
        and not isinstance(load_p95, bool)
        and float(load_p95) <= 500.0,
        "inference_latency": isinstance(inference_p95, (int, float))
        and not isinstance(inference_p95, bool)
        and float(inference_p95) <= 10.0,
        "deterministic_inference": latency.get("deterministic_predictions") is True,
    }


def strict_gates_pass(gates: Mapping[str, object]) -> bool:
    """Reject missing, extra, false, and truthy-non-boolean release gates."""

    return set(gates) == STRICT_GATE_NAMES and all(
        gates.get(name) is True for name in STRICT_GATE_NAMES
    )


def main(argv: Sequence[str] | None = None) -> int:
    arguments = _parse_arguments(argv)
    previous_workers = _DEFAULT_ROW_WORKERS
    set_default_row_workers(arguments.workers)
    sys.stderr.write(
        "KeySwitch evaluator: row scoring uses "
        f"{arguments.workers} worker process(es)\n"
    )
    sys.stderr.flush()
    try:
        return _evaluate(arguments)
    finally:
        set_default_row_workers(previous_workers)


def _evaluate(arguments: EvaluationArguments) -> int:
    config = load_training_config(arguments.config)
    external_policy = external_evaluation_policy_from_config(config)
    verify_training_sources(
        config,
        arguments.english_model,
        arguments.russian_model,
    )
    english, _english_source = load_onboard_unigrams(
        arguments.english_model,
        "en_US",
        0,
        config,
        license_declaration=config.sources.license_declaration,
        license_evidence=config.sources.license_evidence.path,
        logical_path=config.sources.english.path,
        minimum_word_length=SAFETY_COLLISION_MINIMUM_WORD_LENGTH,
    )
    russian, _russian_source = load_onboard_unigrams(
        arguments.russian_model,
        "ru_RU",
        1,
        config,
        license_declaration=config.sources.license_declaration,
        license_evidence=config.sources.license_evidence.path,
        logical_path=config.sources.russian.path,
        minimum_word_length=SAFETY_COLLISION_MINIMUM_WORD_LENGTH,
    )
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
    base_candidate_dataset = build_dataset(
        prepared, config, included_splits=PRESEALED_SPLITS
    )
    hard_negative_corpus = load_hard_negative_development_corpus(
        PROJECT_ROOT / config.hard_negative_development.source.path,
        config,
    )
    candidate_dataset = merge_hard_negative_development(
        base_candidate_dataset,
        hard_negative_corpus,
    )
    assert_no_split_leakage(candidate_dataset)
    training_language_scorers = (
        TrainOnlyLanguageScorers.from_training_partition(
            prepared,
            candidate_dataset.variant_quarantine,
        )
    )
    manifest = _json_object(arguments.manifest)
    model = LinearNgramModel.load(arguments.artifact)
    presealed_provenance = verify_provenance(
        model=model,
        artifact=arguments.artifact,
        manifest=manifest,
        config_path=arguments.config,
        config=config,
        english_path=arguments.english_model,
        russian_path=arguments.russian_model,
        candidate_dataset=candidate_dataset,
        dataset=None,
        training_language_scorer=(
            training_language_scorers.provenance_payload()
        ),
        hard_negative_development=(
            hard_negative_corpus.provenance_payload()
        ),
    )
    if not provenance_checks_pass(
        presealed_provenance, require_full_dataset=False
    ):
        print(
            json.dumps(
                {
                    "schema_version": 1,
                    "phase": "presealed_provenance",
                    "sealed_test_evaluated": False,
                    "provenance": [
                        asdict(check) for check in presealed_provenance
                    ],
                },
                ensure_ascii=False,
                indent=2,
                sort_keys=True,
            )
        )
        return 1

    sealed_phase_dataset = build_dataset(
        prepared, config, included_splits=SEALED_TEST_SPLITS
    )
    assert_no_split_leakage(
        sealed_phase_dataset,
        variant_quarantine_splits=SEALED_TEST_SPLITS,
    )
    dataset = merge_sealed_test_dataset(
        candidate_dataset, sealed_phase_dataset
    )
    assert_no_split_leakage(dataset)
    base_sealed_dataset = merge_sealed_test_dataset(
        base_candidate_dataset, sealed_phase_dataset
    )
    assert_no_split_leakage(base_sealed_dataset)
    provenance = verify_provenance(
        model=model,
        artifact=arguments.artifact,
        manifest=manifest,
        config_path=arguments.config,
        config=config,
        english_path=arguments.english_model,
        russian_path=arguments.russian_model,
        candidate_dataset=candidate_dataset,
        dataset=dataset,
        training_language_scorer=(
            training_language_scorers.provenance_payload()
        ),
        hard_negative_development=(
            hard_negative_corpus.provenance_payload()
        ),
    )
    if not provenance_checks_pass(
        provenance, require_full_dataset=True
    ):
        print(
            json.dumps(
                {
                    "schema_version": 1,
                    "phase": "full_provenance",
                    "sealed_test_evaluated": False,
                    "provenance": [asdict(check) for check in provenance],
                },
                ensure_ascii=False,
                indent=2,
                sort_keys=True,
            )
        )
        return 1

    internal_evidence = recompute_internal_sealed_evidence(
        model=model,
        manifest=manifest,
        config=config,
        prepared=prepared,
        dataset=dataset,
        scorers=training_language_scorers.scorers,
    )
    if not internal_sealed_evidence_checks_pass(internal_evidence.checks):
        print(
            json.dumps(
                {
                    "schema_version": 1,
                    "phase": "internal_sealed_evidence",
                    "sealed_test_evaluated": True,
                    "provenance": [asdict(check) for check in provenance],
                    "internal_sealed_evidence": [
                        asdict(check) for check in internal_evidence.checks
                    ],
                },
                ensure_ascii=False,
                indent=2,
                sort_keys=True,
            )
        )
        return 1

    runtime_threshold_selection = (
        internal_evidence.runtime_threshold_selection
    )
    runtime_threshold_selection_matches = True
    test_prediction_rows = internal_evidence.test_predictions
    test_metrics = internal_evidence.test_metrics
    context_test_metrics = internal_evidence.context_test_metrics
    context_test_typo_metrics = (
        internal_evidence.context_test_typo_metrics
    )
    safety_raw_predictions = internal_evidence.safety_raw_predictions
    safety_raw_metrics = internal_evidence.safety_raw_metrics

    if arguments.provenance_only:
        print(
            json.dumps(
                {
                    "schema_version": 1,
                    "phase": "internal_sealed_evidence",
                    "provenance_passed": True,
                    "internal_sealed_evidence_passed": True,
                    "sealed_test_evaluated": True,
                    "provenance": [asdict(check) for check in provenance],
                    "internal_sealed_evidence": [
                        asdict(check) for check in internal_evidence.checks
                    ],
                },
                ensure_ascii=False,
                indent=2,
                sort_keys=True,
            )
        )
        return 0

    hunspell_snapshots_before_handle = {
        group: _discover_hunspell_snapshot(locale)
        for group, locale in ((0, "en_US"), (1, "ru_RU"))
    }
    runtime_language_models: dict[int, LanguageModel] = {
        0: LanguageModel.load("en_US"),
        1: LanguageModel.load("ru_RU"),
    }
    runtime_language_scorers: dict[int, WordScorer] = dict(
        runtime_language_models
    )
    # Measure the production-shaped runtime before allocating the evaluator-only
    # external corpora.  Otherwise cyclic-GC scans of those retained corpora can
    # dominate a repeated model load even though they do not exist at startup.
    latency = latency_report(
        arguments.artifact,
        model,
        dataset.by_split["test"],
        arguments.latency_sample,
        scorers=runtime_language_scorers,
    )
    hunspell_snapshots = {
        group: _hunspell_dictionary_snapshot(
            runtime_language_models[group], locale
        )
        for group, locale in ((0, "en_US"), (1, "ru_RU"))
    }
    hunspell_handle_snapshot_stable = (
        hunspell_snapshots_before_handle == hunspell_snapshots
        and all(
            runtime_language_models[group].speller.available
            and runtime_language_models[group].speller.source
            == hunspell_snapshots[group].provenance.dictionary.path
            for group in (0, 1)
        )
    )
    # The frozen hard-negative development corpus is an independently tracked
    # external domain.  Index the base lexical/safety dataset here, then add the
    # development signatures exactly once when constructing holdout exclusions.
    # Including development rows in this first index would make the corpus
    # exclude itself and could never reproduce its model-blind preseal receipt.
    sealed_signature_index = build_sealed_signature_index(base_sealed_dataset)
    safety_policy = evaluate_safety_policy(model, dataset.safety)
    slice_predictions = {
        "identity": tuple(
            item
            for item in test_prediction_rows
            if item.example.variant_kind == "identity"
        ),
        "typo_unknown": tuple(
            item
            for item in test_prediction_rows
            if item.example.variant_kind
            in {"deletion", "duplication", "transposition"}
        ),
        "direction_0_to_1": tuple(
            item for item in test_prediction_rows if item.example.target_group == 1
        ),
        "direction_1_to_0": tuple(
            item for item in test_prediction_rows if item.example.target_group == 0
        ),
        "context_none": tuple(
            item
            for item in test_prediction_rows
            if item.example.context_group is None
        ),
        "context_present": tuple(
            item
            for item in test_prediction_rows
            if item.example.context_group is not None
        ),
        "context_misleading": tuple(
            item
            for item in test_prediction_rows
            if item.example.context_group is not None
            and item.example.context_group
            == (
                item.example.source_group
                if item.example.label
                else item.example.target_group
            )
        ),
    }
    slice_metrics = {
        name: prediction_metrics(predictions)
        for name, predictions in slice_predictions.items()
    }
    veto_rows = tuple(
        _VetoRow(item.example, item.raw_logit)
        for item in test_prediction_rows
    )
    veto_result = _veto_result(veto_rows, model.veto_threshold)
    comparison = compare_with_fallback(
        model,
        dataset.by_split["test"],
        arguments.comparison_sample,
        language_models=runtime_language_models,
    )
    disjoint_corpus = build_lexical_disjoint_corpus(
        {
            0: {word.word for word in english},
            1: {word.word for word in russian},
        },
        minimum_words_per_group=external_policy.minimum_words_per_group,
        hunspell_snapshots=hunspell_snapshots,
    )
    unknown_typo_development = build_unknown_typo_disjoint_corpus(
        {
            0: {word.word for word in english},
            1: {word.word for word in russian},
        },
        sealed_physical_signatures=sealed_signature_index.signatures,
        minimum_words_per_group=external_policy.minimum_words_per_group,
        hunspell_snapshots=hunspell_snapshots,
        language_models=runtime_language_models,
        rank_namespace=UNKNOWN_TYPO_DEVELOPMENT_RANK_NAMESPACE,
        choice_namespace=UNKNOWN_TYPO_DEVELOPMENT_CHOICE_NAMESPACE,
    )
    development_physical_signatures = unknown_typo_physical_signatures(
        unknown_typo_development
    )
    unknown_typo_corpus = build_unknown_typo_disjoint_corpus(
        {
            0: {word.word for word in english},
            1: {word.word for word in russian},
        },
        sealed_physical_signatures=(
            sealed_signature_index.signatures
            | development_physical_signatures
        ),
        minimum_words_per_group=external_policy.minimum_words_per_group,
        hunspell_snapshots=hunspell_snapshots,
        language_models=runtime_language_models,
        rank_namespace=UNKNOWN_TYPO_HOLDOUT_RANK_NAMESPACE,
        choice_namespace=UNKNOWN_TYPO_HOLDOUT_CHOICE_NAMESPACE,
    )
    external_provenance_gates = external_corpus_provenance_gate_breakdown(
        external_policy=external_policy,
        hunspell_handle_snapshot_stable=hunspell_handle_snapshot_stable,
        lexical_disjoint=disjoint_corpus,
        sealed_signature_index=sealed_signature_index,
        unknown_typo_development=unknown_typo_development,
        unknown_typo_disjoint=unknown_typo_corpus,
    )
    if not external_corpus_provenance_gates_pass(
        external_provenance_gates
    ):
        print(
            json.dumps(
                {
                    "schema_version": 1,
                    "phase": "external_corpus_provenance",
                    "external_model_metrics_evaluated": False,
                    "external_provenance_gates": (
                        external_provenance_gates
                    ),
                },
                ensure_ascii=False,
                indent=2,
                sort_keys=True,
            )
        )
        return 1
    disjoint_comparison = compare_with_fallback(
        model,
        disjoint_corpus.examples,
        0,
        language_models=runtime_language_models,
    )
    unknown_typo_comparison = compare_with_fallback(
        model,
        unknown_typo_corpus.examples,
        0,
        language_models=runtime_language_models,
    )
    unknown_typo_raw_predictions = predict_model_examples(
        model,
        unknown_typo_corpus.examples,
        scorers=runtime_language_scorers,
    )
    unknown_typo_raw_metrics = prediction_metrics(
        unknown_typo_raw_predictions
    )
    source_known_examples = select_source_known_negative_examples(
        disjoint_corpus.examples,
        language_models=runtime_language_models,
    )
    source_known_comparison = compare_with_fallback(
        model,
        source_known_examples,
        0,
        language_models=runtime_language_models,
    )
    production_context = evaluate_production_context_ensemble(
        model,
        sealed_test=dataset.by_split["test"],
        unknown_typo=unknown_typo_corpus.examples,
        safety=dataset.safety,
        source_known=source_known_examples,
        language_models=runtime_language_models,
    )
    production_context_gate = production_context_ensemble_gate_breakdown(
        config,
        production_context,
    )
    gates = _strict_gates(
        config=config,
        external_policy=external_policy,
        provenance=provenance,
        hunspell_handle_snapshot_stable=(
            hunspell_handle_snapshot_stable
        ),
        runtime_threshold_selection_matches=(
            runtime_threshold_selection_matches
        ),
        test=test_metrics,
        context_test=context_test_metrics,
        context_test_typos=context_test_typo_metrics,
        safety=safety_policy,
        typo_unknown=slice_metrics["typo_unknown"],
        comparison=comparison,
        lexical_disjoint=disjoint_corpus,
        lexical_comparison=disjoint_comparison,
        source_known_comparison=source_known_comparison,
        sealed_signature_index=sealed_signature_index,
        unknown_typo_development=unknown_typo_development,
        unknown_typo_disjoint=unknown_typo_corpus,
        unknown_typo_comparison=unknown_typo_comparison,
        unknown_typo_raw_model=unknown_typo_raw_metrics,
        production_context_gate=production_context_gate,
        latency=latency,
        veto_false_negative_rate=veto_result.false_negative_rate,
    )
    strict_passed = strict_gates_pass(gates)
    payload: dict[str, object] = {
        "model": {
            "version": model.model_version,
            "checksum": model.checksum,
            "dimension": model.dimension,
            "calibration_scope": "lexical-synthetic-not-real-world-probability",
            "training_language_scorer": (
                training_language_scorers.provenance_payload()
            ),
        },
        "provenance": [asdict(check) for check in provenance],
        "runtime_threshold_selection": {
            "matches_signed_training_evidence": (
                runtime_threshold_selection_matches
            ),
            "gate_breakdown": runtime_threshold_selection,
        },
        "external_evaluation_provenance": {
            "policy": {
                "schema_version": external_policy.schema_version,
                "minimum_words_per_group": (
                    external_policy.minimum_words_per_group
                ),
                "trigger_expansion": list(
                    external_policy.trigger_expansion
                ),
                "hunspell": {
                    locale: {
                        "dictionary_sha256": (
                            external_policy.hunspell[group].dictionary.sha256
                        ),
                        "dictionary_bytes": (
                            external_policy.hunspell[group].dictionary.bytes
                        ),
                        "affix_sha256": (
                            external_policy.hunspell[group].affix.sha256
                        ),
                        "affix_bytes": (
                            external_policy.hunspell[group].affix.bytes
                        ),
                    }
                    for group, locale in ((0, "en_US"), (1, "ru_RU"))
                },
                "lexical_disjoint_corpus_sha256": (
                    external_policy.lexical_disjoint_corpus_sha256
                ),
                "unknown_typo_development_corpus_sha256": (
                    external_policy.unknown_typo_development_corpus_sha256
                ),
                "unknown_typo_holdout_corpus_sha256": (
                    external_policy.unknown_typo_holdout_corpus_sha256
                ),
            },
            "actual_hunspell": {
                snapshot.provenance.locale: asdict(snapshot.provenance)
                for snapshot in hunspell_snapshots.values()
            },
            "before_handle_hunspell": {
                snapshot.provenance.locale: asdict(snapshot.provenance)
                for snapshot in hunspell_snapshots_before_handle.values()
            },
            "handle_snapshot_stable": hunspell_handle_snapshot_stable,
            "bounded_snapshot_reads_per_file": 2,
            "words_loaded_by_group": {
                snapshot.provenance.locale: len(snapshot.words)
                for snapshot in hunspell_snapshots.values()
            },
        },
        "reproducibility": {
            "scope": "input provenance, sealed dataset fingerprint, repeated load and inference",
            "byte_identical_retraining_executed": False,
            "note": "Run the deterministic trainer twice to audit byte-identical retraining.",
        },
        "sealed_test": {
            trigger: metrics_payload(metrics)
            for trigger, metrics in test_metrics.items()
        },
        "sealed_test_context_stress": {
            "gate_breakdown": context_stress_gate_breakdown(
                config,
                context_test_metrics,
                context_test_typo_metrics,
                phase="sealed_test",
            ),
            "profiles": {
                profile.name: {
                    "overall": {
                        trigger: metrics_payload(metrics)
                        for trigger, metrics in context_test_metrics[
                            profile.name
                        ].items()
                    },
                    "typos": {
                        trigger: metrics_payload(metrics)
                        for trigger, metrics in context_test_typo_metrics[
                            profile.name
                        ].items()
                    },
                }
                for profile in CONTEXT_STRESS_PROFILES
            },
        },
        "safety": {
            "qualification": (
                "Strict safety gates use the production detector ensemble and its "
                "pre-model structural/source-known guards. Direct intent-model "
                "predictions are diagnostic only."
            ),
            "guard_audit": {
                "samples": safety_policy.samples,
                "protected_samples": safety_policy.protected_samples,
                "lexical_collision_samples": (
                    safety_policy.lexical_collision_samples
                ),
                "pre_model_guarded_samples": (
                    safety_policy.pre_model_guarded_samples
                ),
                "expected_pre_model_guard_samples": (
                    safety_policy.expected_pre_model_guard_samples
                ),
                "expected_pre_model_guarded_samples": (
                    safety_policy.expected_pre_model_guarded_samples
                ),
                "model_evaluated_samples": safety_policy.model_evaluated_samples,
                "guard_failure_samples": safety_policy.guard_failure_samples,
                "reason_counts": safety_policy.reason_counts,
                "production_policy_per_trigger": {
                    trigger: metrics_payload(metrics)
                    for trigger, metrics in safety_policy.per_trigger.items()
                },
            },
            "raw_model_diagnostics": {
                "per_trigger": {
                    trigger: metrics_payload(metrics)
                    for trigger, metrics in safety_raw_metrics.items()
                },
                "membership_coverage": {
                    name: asdict(statistics_for_slice)
                    for name, statistics_for_slice in coverage_diagnostics(
                        safety_raw_predictions
                    ).items()
                },
                "is_a_gate": False,
            },
        },
        "sealed_slices": {
            name: {
                trigger: metrics_payload(metrics)
                for trigger, metrics in per_trigger.items()
            }
            for name, per_trigger in slice_metrics.items()
        },
        "veto": asdict(veto_result),
        "model_vs_fallback": _comparison_payload(comparison),
        "lexical_disjoint_hunspell": {
            "qualification": (
                "Lexically disjoint from Onboard unigrams; not fully source-independent "
                "because Hunspell is also an input to the runtime language scorer. "
                "Its correct-layout negatives validate the source-known hard guard, "
                "not model false-positive behavior."
            ),
            "words_by_group": disjoint_corpus.words_by_group,
            "dictionary_sources": disjoint_corpus.dictionary_sources,
            "dictionary_provenance": {
                group: asdict(item)
                for group, item in disjoint_corpus.dictionary_provenance.items()
            },
            "corpus_sha256": disjoint_corpus.corpus_sha256,
            "comparison": _comparison_payload(disjoint_comparison),
            "runtime_source_known_guard_subset": {
                "samples": len(source_known_examples),
                "comparison": _comparison_payload(source_known_comparison),
            },
        },
        "lexical_disjoint_unknown_typos_development": {
            "qualification": (
                "Policy-development corpus retained byte-for-byte from v5. "
                "Its observed metrics were used to freeze the v6/v7 serving policy, "
                "so it is never accepted as independent release evidence."
            ),
            "is_release_gate": False,
            "words_by_group": unknown_typo_development.words_by_group,
            "corpus_sha256": unknown_typo_development.corpus_sha256,
            "rank_namespace": unknown_typo_development.rank_namespace,
            "choice_namespace": unknown_typo_development.choice_namespace,
            "selected_physical_signature_count": len(
                development_physical_signatures
            ),
        },
        "lexical_disjoint_unknown_typos": {
            "qualification": (
                "Previously unseen release holdout of deterministic physical-key "
                "typos of Hunspell-not-Onboard words; "
                "both decodings are verified unknown by the runtime scorers before "
                "the symmetric pair enters evaluation. Every physical signature "
                "emitted by a sealed train/development/calibration/threshold/test "
                "or safety row, plus every v5 development signature, is excluded "
                "before selection."
            ),
            "role": "independent-release-holdout",
            "rank_namespace": unknown_typo_corpus.rank_namespace,
            "choice_namespace": unknown_typo_corpus.choice_namespace,
            "serving_policy": {
                "model_first_after_hard_guards": True,
                "post_guard_decision_rule": (
                    "trigger_direction_calibrated_logit_threshold_only"
                ),
                "membership_coverage_role": "diagnostic_only",
                "target_language_score_role": "diagnostic_only",
            },
            "words_by_group": unknown_typo_corpus.words_by_group,
            "dictionary_sources": unknown_typo_corpus.dictionary_sources,
            "dictionary_provenance": {
                group: asdict(item)
                for group, item in unknown_typo_corpus.dictionary_provenance.items()
            },
            "corpus_sha256": unknown_typo_corpus.corpus_sha256,
            "sealed_signature_exclusion_provenance": {
                "schema": "keyswitch-intent-v3-holdout-exclusions",
                "signature_count": (
                    unknown_typo_corpus.exclusion_signature_count
                ),
                "sha256": unknown_typo_corpus.exclusion_signature_sha256,
                "sealed_signature_count": (
                    sealed_signature_index.signature_count
                ),
                "sealed_signature_sha256": sealed_signature_index.sha256,
                "development_signature_count": len(
                    development_physical_signatures
                ),
                "development_signature_sha256": (
                    physical_signature_set_sha256(
                        development_physical_signatures
                    )
                ),
                "split_lexical_signature_count": (
                    sealed_signature_index.split_lexical_signature_count
                ),
                "safety_lexical_signature_count": (
                    sealed_signature_index.safety_lexical_signature_count
                ),
                "protected_exact_signature_count": (
                    sealed_signature_index.protected_exact_signature_count
                ),
            },
            "rejected_sealed_overlaps_by_group": (
                unknown_typo_corpus.rejected_sealed_overlaps_by_group
            ),
            "rejected_sealed_overlaps_total": sum(
                unknown_typo_corpus.rejected_sealed_overlaps_by_group.values()
            ),
            "selected_sealed_overlap_count": (
                unknown_typo_corpus.selected_sealed_overlap_count
            ),
            "comparison": _comparison_payload(unknown_typo_comparison),
            "raw_model_per_trigger": {
                trigger: metrics_payload(metrics)
                for trigger, metrics in unknown_typo_raw_metrics.items()
            },
            "membership_coverage_diagnostics": {
                name: asdict(statistics_for_slice)
                for name, statistics_for_slice in coverage_diagnostics(
                    unknown_typo_raw_predictions
                ).items()
            },
            "membership_coverage_is_a_gate": False,
        },
        "production_context_ensemble": _production_context_payload(
            production_context,
            production_context_gate,
        ),
        "performance": latency,
        "strict_gates": gates,
        "strict_passed": strict_passed,
        "statistical_limitation": (
            "Observed false-positive rates and Wilson upper bounds describe only "
            "the sealed synthetic lexical set, not real-world user traffic."
        ),
    }
    print(json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True))
    if arguments.strict and not strict_passed:
        return 1
    return 0


@dataclass(frozen=True)
class _VetoRow:
    example: LexicalExample
    raw_logit: float


@dataclass(frozen=True)
class _VetoResult:
    raw_logit: float
    positive_samples: int
    vetoed_positive_samples: int
    false_negative_rate: float


def _veto_result(
    rows: Sequence[_VetoRow], threshold: float
) -> _VetoResult:
    positives = tuple(row for row in rows if row.example.label)
    if not positives:
        raise ValueError("veto evaluation requires positives")
    vetoed = sum(row.raw_logit < threshold for row in positives)
    return _VetoResult(threshold, len(positives), vetoed, vetoed / len(positives))


if __name__ == "__main__":
    sys.exit(main())
