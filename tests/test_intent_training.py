"""Focused tests for deterministic offline intent-model training."""

from __future__ import annotations

import copy
import hashlib
import json
import math
import os
import random
import shutil
import struct
import sys
import tempfile
import unittest
from collections.abc import Collection, Mapping, Sequence
from concurrent.futures import ThreadPoolExecutor
from contextlib import ExitStack
from dataclasses import asdict, replace
from itertools import product
from pathlib import Path
from types import SimpleNamespace
from threading import Barrier
from typing import BinaryIO, cast
from unittest.mock import patch

TOOLS_PATH = str(Path(__file__).resolve().parents[1] / "tools")
if TOOLS_PATH not in sys.path:
    sys.path.insert(0, TOOLS_PATH)

import evaluate_intent_model as eim  # noqa: E402
import train_intent_model as tim  # noqa: E402

from keyswitch.intent_model import (  # noqa: E402
    DEFAULT_FNV_SEED,
    DEFAULT_MEMBERSHIP_FNV_SEED,
    CorrectionTrigger,
    IntentModelInput,
    LayoutDirection,
    LinearPrediction,
    MAX_CONTAINER_BYTES,
    NGRAM_ORDERS,
    TRIGGERS,
    LinearNgramModel,
    PlattParameters,
    extract_features,
    stable_sigmoid as runtime_stable_sigmoid,
    write_model,
)
from keyswitch.detector import LanguageDetector  # noqa: E402
from keyswitch.detector import (  # noqa: E402
    CONTEXT_DELTA_MULTIPLIER,
    CONTEXT_SCORE_MAXIMUM,
    CONTEXT_SCORE_MINIMUM,
    CONTEXT_SOURCE_GROUP_PENALTY,
    CONTEXT_TARGET_GROUP_BONUS,
)
from keyswitch.language_model import LanguageModel, WordScore  # noqa: E402
from keyswitch.layouts import LayoutPair  # noqa: E402
from evaluate_intent_model import (  # noqa: E402
    ExternalEvaluationPolicy,
    FrozenExternalFile,
    FrozenExternalFilePolicy,
    HunspellDictionaryProvenance,
    HunspellDictionarySnapshot,
    HunspellLocalePolicy,
    LexicalDisjointCorpus,
    ModelPredictionRow,
    INTERNAL_SEALED_EVIDENCE_CHECK_NAMES,
    PredictionComparison,
    PRODUCTION_CONTEXT_PROFILES,
    ProductionContextCorpusEvaluation,
    ProductionContextEnsembleEvaluation,
    ProductionContextProfileEvaluation,
    ProductionPolicyRow,
    SafetyPolicyEvaluation,
    SealedSignatureIndex,
    EXTERNAL_CORPUS_PROVENANCE_GATE_NAMES,
    STRICT_GATE_NAMES,
    VerificationCheck,
    _strict_gates,
    build_lexical_disjoint_corpus,
    build_sealed_signature_index,
    build_unknown_typo_disjoint_corpus,
    compare_with_fallback,
    coverage_diagnostics,
    evaluate_context_stress,
    evaluate_production_context_ensemble,
    evaluate_safety_policy,
    external_corpus_sha256,
    external_corpus_provenance_gates_pass,
    load_external_evaluation_policy,
    main as evaluate_main,
    model_metrics,
    physical_signature_set_sha256,
    predict_model_examples,
    prediction_metrics,
    production_context_ensemble_gate_breakdown,
    provenance_checks_pass,
    recompute_internal_sealed_evidence,
    select_source_known_negative_examples,
    strict_gates_pass,
    unknown_typo_physical_signatures,
    verify_provenance,
)
from train_intent_model import (  # noqa: E402
    CONTEXT_STRESS_PROFILES,
    ConfusionMatrix,
    DatasetBundle,
    DevelopmentEpochEvaluation,
    DirectionalPlattCalibration,
    ExtractedExampleFeatures,
    FTRLProximal,
    FTRLParameters,
    FeaturedExample,
    FrozenExternalEvaluationPolicy,
    FrozenExternalLocalePolicy,
    FrozenLanguageSource,
    FrozenSourceFile,
    GuardedSafetyAudit,
    HARD_NEGATIVE_ROLE_NAMESPACE,
    HARD_NEGATIVE_SOURCE_RELATIVE_PATH,
    HardNegativeDevelopmentCorpus,
    HardNegativeDevelopmentPolicy,
    LexicalExample,
    LexiconSource,
    LexiconWord,
    PlattCalibration,
    PreparedLexicon,
    QuantizedLinearScorer,
    QuarantinedVariantOccurrence,
    PRESEALED_SPLITS,
    SELECTION_FALSE_POSITIVE_COMPARISONS,
    SELECTION_PER_COMPARISON_CONFIDENCE,
    SELECTION_WILSON_Z_SCORE,
    SEALED_TEST_SPLITS,
    ScoredExample,
    SealedEvaluationPolicy,
    SealedEvaluationReceipt,
    SPLIT_NAMESPACE,
    UNKNOWN_TYPO_DEVELOPMENT_CHOICE_NAMESPACE,
    UNKNOWN_TYPO_DEVELOPMENT_RANK_NAMESPACE,
    UNKNOWN_TYPO_HOLDOUT_CHOICE_NAMESPACE,
    UNKNOWN_TYPO_HOLDOUT_RANK_NAMESPACE,
    SplitName,
    TrainingConfig,
    TrainingSources,
    ThresholdSelection,
    TRAINING_HARD_NEGATIVES,
    TrainOnlyLanguageScorers,
    VariantQuarantine,
    VetoSelection,
    WILSON_95_Z_SCORE,
    WILSON_INTERVAL_CONFIDENCE,
    WordScorer,
    audit_guarded_safety_corpus,
    audit_dataset_physical_signatures,
    assert_no_split_leakage,
    binary_gate_breakdown,
    build_dataset,
    build_variant_quarantine,
    capture_toolchain_snapshot,
    claim_sealed_evaluation,
    choose_threshold,
    choose_directional_threshold,
    choose_trigger_thresholds,
    choose_veto_threshold,
    context_stress_examples,
    context_stress_gate_breakdown,
    dataset_fingerprint,
    deterministic_training_trigger,
    evaluate_development_epoch,
    fit_ftrl,
    fit_directional_platt_calibration,
    fit_platt_calibration,
    false_positive_bound_payload,
    featurize_examples,
    gate_policy_payload,
    intent_input_for_example,
    load_onboard_unigrams,
    load_hard_negative_development_corpus,
    load_training_config,
    load_training_config_snapshot,
    main as train_main,
    metrics_payload,
    merge_hard_negative_development,
    merge_sealed_test_dataset,
    normalize_sparse_features,
    physical_signature,
    prepare_lexicon,
    publish_bytes_bundle,
    presealed_candidate_gate_breakdown,
    quantize_weights,
    quantized_model_payload_sha256,
    runtime_feature_extractor,
    runtime_candidate_model_parameters,
    score_context_stress_profiles,
    selection_tail_diagnostics,
    sealed_candidate_sha256,
    sealed_evaluation_evidence_is_valid,
    sha256_file,
    stable_sigmoid,
    stable_split,
    supported_fingerprints_sha256,
    training_word_score,
    training_quality_gates_pass,
    training_quality_gate_breakdown,
    threshold_selection_gate_breakdown,
    training_candidate_model_parameters,
    presealed_candidate_counts,
    presealed_candidate_metadata_projection,
    typo_variants,
    variant_quarantine_fingerprint,
    validate_training_paths,
    validate_presealed_candidate_serialization,
    ENVIRONMENT_PROBE_PATH,
    verify_frozen_file,
    verify_context_feature_invariance,
    verify_sealed_evaluation_receipt,
    verify_training_sources,
    verify_toolchain_snapshot,
    veto_metrics,
    wilson_upper_bound,
)
from fixture_values.clock import INTENT_BARRIER_WAIT_TIMEOUT_SECONDS
from fixture_values.corpora import (
    HARD_NEGATIVE_CORPUS_JSON_INDENT,
    INTENT_DATASET_EXPECTED_HELLO_FREQUENCY,
    INTENT_DATASET_EXPECTED_KEY_FREQUENCY,
    INTENT_DATASET_HIGH_FREQUENCY,
    INTENT_DATASET_LOWER_MID_FREQUENCY,
    INTENT_DATASET_LOW_FREQUENCY,
    INTENT_DATASET_MID_FREQUENCY,
    INTENT_DATASET_UPPER_MID_FREQUENCY,
    INTENT_EVALUATION_LM_KNOWN_WORD_FREQUENCY,
    INTENT_LEAKAGE_EXCLUDED_ENTRY_FREQUENCY,
    INTENT_LEAKAGE_INCLUDED_ENTRY_FREQUENCY,
    INTENT_LEAKAGE_MUTATED_NON_TRAIN_EN_FREQUENCY,
    INTENT_LEAKAGE_MUTATED_NON_TRAIN_RU_FREQUENCY,
    INTENT_LEAKAGE_NON_TRAIN_ENTRY_FREQUENCY,
    INTENT_LEAKAGE_NON_TRAIN_EN_RECORD_INDEX,
    INTENT_LEAKAGE_NON_TRAIN_RU_RECORD_INDEX,
    INTENT_LEAKAGE_POISONED_FREQUENCY,
    INTENT_LEAKAGE_TRAIN_RECORD_COUNT,
)
from fixture_values.counts import (
    EXPECTED_COMBINED_EXCLUSION_SIGNATURE_COUNT,
    EXPECTED_HARD_NEGATIVE_PRESEAL_SIGNATURE_COUNT,
    EXPECTED_INTENT_CONFIG_MINIMUM_WORDS_PER_GROUP,
    EXPECTED_INTENT_CONFIG_MINIMUM_WORD_LENGTH,
    EXPECTED_INTENT_PRODUCTION_ROLE_COUNTS,
    EXPECTED_PRESEAL_EXAMPLES_BY_ROLE,
    EXPECTED_PRESEAL_ROLE_WORDS_BY_GROUP,
    EXPECTED_PRESEAL_WORDS_BY_GROUP,
    EXPECTED_SEALED_EXCLUSION_SIGNATURE_COUNT,
    FTRL_CACHED_WEIGHTS_EPOCHS,
    FTRL_KERNEL_COMPARISON_DEV_ROW_COUNT,
    FTRL_KERNEL_COMPARISON_DEV_SEED,
    FTRL_KERNEL_COMPARISON_MAX_EPOCHS,
    FTRL_KERNEL_COMPARISON_ROW_COUNT,
    FTRL_KERNEL_COMPARISON_SEED,
    FTRL_KERNEL_DIVERGENCE_TEST_ROW_COUNT,
    FTRL_KERNEL_DIVERGENCE_TEST_SEED,
    FTRL_KERNEL_EXPECTED_INDPTR_LENGTH,
    FTRL_KERNEL_FALLBACK_TEST_DEV_ROW_COUNT,
    FTRL_KERNEL_FALLBACK_TEST_DEV_SEED,
    FTRL_KERNEL_FALLBACK_TEST_ROW_COUNT,
    FTRL_KERNEL_FALLBACK_TEST_SEED,
    FTRL_KERNEL_MAX_ROW_FEATURE_WIDTH,
    FTRL_KERNEL_NATIVE_TEST_EPOCH_RANGE_END,
    FTRL_KERNEL_NATIVE_TEST_ROW_COUNT,
    FTRL_KERNEL_NATIVE_TEST_SEED,
    FTRL_KERNEL_PACK_VALIDATION_ROW_COUNT,
    FTRL_KERNEL_PARTIAL_ORDER_ROW_COUNT,
    FTRL_KERNEL_PARTIAL_ORDER_SEED,
    FTRL_KERNEL_PARTIAL_ORDER_SKIP_MODULUS,
    FTRL_KERNEL_POSITION_PARITY_MODULUS,
    INTENT_ARTIFACT_EXPECTED_DIAGNOSTIC_COUNT,
    INTENT_ARTIFACT_FULL_VERIFY_CALL_NUMBER,
    INTENT_ARTIFACT_PASSED_FULL_VERIFY_CALL_NUMBER,
    INTENT_ARTIFACT_QUANTIZATION_TEST_METRIC_COUNT,
    INTENT_ARTIFACT_SEALED_CANDIDATE_POSITIVES_PER_DIRECTION,
    INTENT_ARTIFACT_SEALED_CANDIDATE_SAMPLES_PER_DIRECTION,
    INTENT_ARTIFACT_TAMPERED_VETOED_POSITIVE_SAMPLES,
    INTENT_CONCURRENT_CLAIMANTS,
    INTENT_DATASET_DIRECTION_PAIR_COUNT,
    INTENT_DATASET_EXPECTED_CONTEXT_STRESS_PROFILE_COUNT,
    INTENT_DATASET_EXPECTED_HARD_NEGATIVE_ROLE_SIGNATURES,
    INTENT_DATASET_EXPECTED_HARD_NEGATIVE_SIGNATURE_COUNT,
    INTENT_DATASET_EXPECTED_HARD_NEGATIVE_SPLIT_ROWS,
    INTENT_DATASET_EXPECTED_HARD_NEGATIVE_WORDS_BY_GROUP,
    INTENT_DATASET_EXPECTED_MERGED_HARD_NEGATIVE_AUDITED_ROWS,
    INTENT_DATASET_EXPECTED_PREPARED_WORD_COUNT,
    INTENT_DATASET_EXPECTED_STRESSED_EXAMPLE_COUNT,
    INTENT_DATASET_FUNCTION_MINIMUM_WORD_LENGTH,
    INTENT_DATASET_MINIMUM_WORD_LENGTH,
    INTENT_DEFAULT_POSITIVES_PER_DIRECTION,
    INTENT_DEFAULT_SAMPLES_PER_DIRECTION,
    INTENT_EXCESSIVE_REQUESTED_WORKERS,
    INTENT_EXPECTED_AFFINITY_WORKER_COUNT,
    INTENT_EXTERNAL_AFFIX_BYTES,
    INTENT_EXTERNAL_DICTIONARY_BYTES,
    INTENT_EXTERNAL_EXPECTED_COVERAGE_PER_LABEL_COUNT,
    INTENT_EXTERNAL_EXPECTED_COVERAGE_SAMPLE_COUNT,
    INTENT_EXTERNAL_EXPECTED_HARD_GUARD_MODEL_EVALUATED,
    INTENT_EXTERNAL_EXPECTED_HARD_GUARD_SAMPLES,
    INTENT_EXTERNAL_EXPECTED_NEGATIVE_ROWS,
    INTENT_EXTERNAL_EXPECTED_RAW_METRICS_COUNT,
    INTENT_EXTERNAL_EXPECTED_SAFETY_POLICY_SAMPLES,
    INTENT_EXTERNAL_EXPECTED_STRICT_SAMPLE_SIZE,
    INTENT_EXTERNAL_INVALID_AFFIX_BYTES_FLOAT,
    INTENT_EXTERNAL_MINIMUM_APPLICABLE_DELETION_LENGTH,
    INTENT_EXTERNAL_MINIMUM_WORDS_PER_GROUP,
    INTENT_EXTERNAL_SAFETY_TRIGGER_SAMPLE_COUNT,
    INTENT_EXTERNAL_SOURCE_KNOWN_SAMPLE_COUNT,
    INTENT_EXTERNAL_THIRD_CANDIDATE_INDEX,
    INTENT_EXTERNAL_UNDERSIZED_MINIMUM_WORDS_PER_GROUP,
    INTENT_EXTERNAL_UNKNOWN_TYPO_EXAMPLES_PER_TRIGGER,
    INTENT_EXTERNAL_UNKNOWN_TYPO_NEGATIVE_EXAMPLES_PER_TRIGGER,
    INTENT_HARD_NEGATIVE_INPUT_INDEX,
    INTENT_LABEL_PARITY_MODULUS,
    INTENT_LEAKAGE_EXCLUDED_WORD_LENGTH,
    INTENT_LEAKAGE_INCLUDED_WORD_LENGTH,
    INTENT_LEAKAGE_PARALLEL_FEATURIZATION_EXAMPLE_COUNT,
    INTENT_LEAKAGE_PARALLEL_FEATURIZATION_WORKER_COUNT,
    INTENT_LEAKAGE_RUSSIAN_WORD_SIGNATURE_LENGTH,
    INTENT_LICENSE_INPUT_INDEX,
    INTENT_MAX_PRESEAL_BYTES,
    INTENT_MINIMUM_LENGTH_BEFORE_DELETION,
    INTENT_MINIMUM_SIGNATURE_LENGTH,
    INTENT_OPTIMIZER_COMPARISONS_PER_TRIGGER,
    INTENT_OPTIMIZER_CONTEXT_STRESS_SCORE_COUNT_PER_EXAMPLE,
    INTENT_OPTIMIZER_DELETION_TYPO_STRONG_COUNT,
    INTENT_OPTIMIZER_DELETION_TYPO_WEAK_COUNT,
    INTENT_OPTIMIZER_DIRECTIONAL_SAMPLE_DIRECTION_INDEX,
    INTENT_OPTIMIZER_DIRECTION_GROUP_MODULUS,
    INTENT_OPTIMIZER_DIRECTION_SWITCH_THRESHOLD,
    INTENT_OPTIMIZER_EXAMPLES_PER_DIRECTION_DIVISOR,
    INTENT_OPTIMIZER_EXPECTED_DIRECTIONAL_POSITIVE_COUNT,
    INTENT_OPTIMIZER_EXPECTED_TRUE_POSITIVE_COUNT,
    INTENT_OPTIMIZER_FALSE_POSITIVE_BOUND_FALSE_POSITIVES,
    INTENT_OPTIMIZER_FALSE_POSITIVE_BOUND_TOTAL_NEGATIVES,
    INTENT_OPTIMIZER_FEATURED_EXAMPLE_COUNT,
    INTENT_OPTIMIZER_IDENTITY_TYPO_STRONG_COUNT,
    INTENT_OPTIMIZER_IDENTITY_TYPO_WEAK_COUNT,
    INTENT_OPTIMIZER_INVALID_SELECTION_BUDGET,
    INTENT_OPTIMIZER_LABEL_VARIANT_COMBINATION_COUNT,
    INTENT_OPTIMIZER_MAXIMUM_EPOCHS,
    INTENT_OPTIMIZER_NEGATIVE_FIXTURE_COUNT,
    INTENT_OPTIMIZER_NEGATIVE_TAIL_COUNT,
    INTENT_OPTIMIZER_PARALLEL_WORKER_COUNT,
    INTENT_OPTIMIZER_PATIENCE,
    INTENT_OPTIMIZER_POSITIVES_PER_DIRECTION_OVERRIDE,
    INTENT_OPTIMIZER_SELECTION_FALSE_POSITIVE_BUDGET,
    INTENT_OPTIMIZER_SINGLE_EXAMPLE_MAX_ITERATIONS,
    INTENT_OPTIMIZER_TRAINING_ROW_COUNT,
    INTENT_PATCHED_BYTE_LIMIT,
    INTENT_PATCHED_MAX_PAYLOAD_BYTES,
    INTENT_PATCHED_MAX_SUPPORTED_FINGERPRINTS,
    INTENT_PERFECT_CONFUSION_COUNT,
    INTENT_PLATT_TEST_MAX_ITERATIONS,
    INTENT_PRESEALED_VERIFY_CALL_NUMBERS,
    INTENT_ROW_SCORING_AVAILABLE_ROW_WORKERS_CAP,
    INTENT_ROW_SCORING_CONTEXT_PROFILE_SAMPLE_COUNT,
    INTENT_ROW_SCORING_EFFECTIVE_ROW_WORKERS_CAP,
    INTENT_ROW_SCORING_MISORDERED_CHUNK_COUNT,
    INTENT_ROW_SCORING_MODERATE_PARALLEL_TEST_WORKERS,
    INTENT_ROW_SCORING_PARALLEL_TEST_WORKERS,
    INTENT_ROW_SCORING_REQUESTED_ROW_WORKERS,
    INTENT_ROW_SCORING_SECONDARY_LABEL_MODULUS,
    INTENT_ROW_SCORING_WIDE_PARALLEL_TEST_WORKERS,
    INTENT_RUSSIAN_INPUT_INDEX,
    INTENT_SAFETY_SAMPLE_MULTIPLIER,
    INTENT_SIGNATURE_LENGTH_SEARCH_BOUND,
    INTENT_TAMPERED_TRAINING_SEED,
    INTENT_TEST_TRAINING_SEED,
    INTENT_TYPO_AUGMENTATION_FIXTURE_LIMIT,
    INTENT_UNDERSIZED_CONFIG_MINIMUM_WORDS_PER_GROUP,
    INTENT_VETO_SELECTION_SAMPLE_COUNT,
    INTENT_WEAK_SAMPLE_SIZE,
    WILSON_BOUND_COMPARISON_PLACES,
    WILSON_BOUND_TEST_FALSE_POSITIVES,
    WILSON_BOUND_TEST_SAMPLE_SIZE,
)
from fixture_values.models import (
    EXPECTED_INTENT_AGGREGATE_BUDGET_CONFUSION_COUNTS,
    EXPECTED_INTENT_CONFIG_SCHEMA_VERSION,
    EXPECTED_INTENT_DIRECTIONAL_BUDGET_CONFUSION_COUNTS,
    EXPECTED_INTENT_MINIMUM_RUNTIME_TOKEN_LENGTH,
    EXPECTED_INTENT_PRODUCTION_MODEL_DIMENSION,
    EXPECTED_INTENT_TYPO_POLICY_CONFUSION_COUNTS,
    EXPECTED_INTENT_TYPO_POLICY_TYPO_CONFUSION_COUNTS,
    EXPECTED_INTENT_ZERO_FALSE_POSITIVE_CONFUSION_COUNTS,
    FTRL_CACHED_WEIGHTS_DIMENSION,
    FTRL_KERNEL_DIMENSION,
    FTRL_KERNEL_DUPLICATE_FEATURE_INDEX,
    FTRL_SMALL_TEST_DIMENSION,
    INTENT_EXCESSIVE_DIMENSION_EXPONENT,
    INTENT_EXTERNAL_POLICY_SCHEMA_VERSION,
    INTENT_HARD_NEGATIVE_CALIBRATION_WORDS_PER_GROUP,
    INTENT_HARD_NEGATIVE_DEVELOPMENT_WORDS_PER_GROUP,
    INTENT_HARD_NEGATIVE_POLICY_SCHEMA_VERSION,
    INTENT_HARD_NEGATIVE_THRESHOLD_WORDS_PER_GROUP,
    INTENT_HARD_NEGATIVE_TRAIN_WORDS_PER_GROUP,
    INTENT_HARD_NEGATIVE_UNDERFILLED_TRAIN_WORDS_PER_GROUP,
    INTENT_INITIAL_SYMMETRIC_CONFUSION_COUNTS,
    INTENT_INVALID_QUANTIZED_WEIGHT,
    INTENT_LEAKAGE_ALTERNATE_TEST_DIMENSION,
    INTENT_LEAKAGE_NON_FINITE_SPARSE_FEATURES,
    INTENT_LEAKAGE_NORMALIZED_SPARSE_FEATURES,
    INTENT_LEAKAGE_OUT_OF_RANGE_SPARSE_FEATURES,
    INTENT_LEAKAGE_SPARSE_FEATURE_TEST_DIMENSION,
    INTENT_LEAKAGE_UNNORMALIZED_SPARSE_FEATURES,
    INTENT_MODERATE_SAMPLE_SIZE,
    INTENT_NONCANONICAL_FEATURE_CASES,
    INTENT_OPTIMIZER_SECOND_FEATURE_INDEX,
    INTENT_PASSING_EPOCH_CONFUSION_COUNTS,
    INTENT_PASSING_EPOCH_POLICY_CHECKS_PASSED,
    INTENT_PASSING_EPOCH_TYPO_CONFUSION_COUNTS,
    INTENT_QUANTIZED_FIRST_WEIGHT_INDEX,
    INTENT_QUANTIZED_SECOND_WEIGHT_INDEX,
    INTENT_SEALED_CANDIDATE_FINGERPRINTS,
    INTENT_SEALED_CANDIDATE_QUANTIZE_WEIGHTS,
    INTENT_STRICT_OVERALL_CONFUSION_COUNTS,
    INTENT_STRICT_TYPO_CONFUSION_COUNTS,
    INTENT_STRONG_SAMPLE_SIZE,
    INTENT_TAIL_DIAGNOSTIC_OVERALL_CONFUSION_COUNTS,
    INTENT_TAMPERED_FINGERPRINTS,
    INTENT_TEST_DIMENSION,
    INTENT_TEST_TRAINING_CONFIG_FIELDS,
    INTENT_TRAIN_ONLY_SCORER_ALGORITHM_VERSION,
    INTENT_TRAIN_ONLY_SCORER_NGRAM_ORDERS,
    INTENT_UNCERTIFIED_EPOCH_CONFUSION_COUNTS,
    INTENT_UNCERTIFIED_EPOCH_POLICY_CHECKS_PASSED,
    INTENT_UNCERTIFIED_EPOCH_TYPO_CONFUSION_COUNTS,
    INTENT_UNSUPPORTED_CONFIG_SCHEMA_VERSION,
    INTENT_UNSUPPORTED_MANIFEST_SCHEMA_VERSION,
    INTENT_WILSON_BOUND_CONFUSION_COUNTS,
    UINT64_BIT_WIDTH,
)
from fixture_values.hashes import (
    INTENT_HASH_SEED_XOR_PERTURBATION,
    INTENT_MEMBERSHIP_SEED_XOR_PERTURBATION,
)
from fixture_values.keys import INTENT_LEAKAGE_ARBITRARY_CONTEXT_GROUP, UNSUPPORTED_LAYOUT_GROUP
from fixture_values.platform import FAKE_AFFINITY_CORE_IDS, FAKE_CPU_COUNT
from fixture_values.scores import (
    EXPECTED_INTENT_CONTEXT_DELTA_MULTIPLIER,
    EXPECTED_INTENT_CONTEXT_SOURCE_GROUP_PENALTY,
    EXPECTED_INTENT_CONTEXT_TARGET_GROUP_BONUS,
    EXPECTED_INTENT_PRODUCTION_LOGIT_MARGIN_CAP,
    EXPECTED_INTENT_PRODUCTION_SELECTION_RECALLS,
    EXPECTED_INTENT_PRODUCTION_TRAINING_EXAMPLE_WEIGHT,
    EXPECTED_WILSON_BOUND_AT_ZERO_FALSE_POSITIVES,
    FTRL_CACHED_WEIGHTS_ALPHA,
    FTRL_CACHED_WEIGHTS_CANONICAL_UPDATES,
    FTRL_CACHED_WEIGHTS_FIRST_RAW_FEATURES,
    FTRL_CACHED_WEIGHTS_FIRST_SAMPLE_WEIGHT,
    FTRL_CACHED_WEIGHTS_L1,
    FTRL_CACHED_WEIGHTS_L2,
    FTRL_KERNEL_FEATURE_VALUE_LARGE,
    FTRL_KERNEL_FEATURE_VALUE_NEGATIVE_SMALL,
    FTRL_KERNEL_FEATURE_VALUE_SMALL,
    FTRL_KERNEL_LABEL_PROBABILITY_THRESHOLD,
    FTRL_KERNEL_NATIVE_TEST_ALPHA,
    FTRL_KERNEL_NATIVE_TEST_L1,
    FTRL_KERNEL_NATIVE_TEST_L2,
    FTRL_KERNEL_PARTIAL_ORDER_ALPHA,
    FTRL_KERNEL_RANDOM_FEATURE_MAGNITUDE,
    FTRL_KERNEL_WEIGHT_UNIFORM_LOWER_BOUND,
    FTRL_KERNEL_WEIGHT_VALUE_LARGE,
    FTRL_SPARSE_TEST_L1,
    FTRL_TEST_ALPHA,
    FTRL_TEST_L2,
    INTENT_ARTIFACT_FIXTURE_VETO_THRESHOLD,
    INTENT_ARTIFACT_NEGATIVE_MODEL_LOGIT,
    INTENT_ARTIFACT_PRECISE_TAMPERED_LOGIT,
    INTENT_ARTIFACT_QUANTIZATION_PLATT_BIAS_MAGNITUDE,
    INTENT_ARTIFACT_QUANTIZATION_PLATT_SCALE,
    INTENT_ARTIFACT_QUANTIZATION_TEST_THRESHOLD_LOGIT,
    INTENT_ARTIFACT_QUANTIZATION_VETO_THRESHOLD,
    INTENT_ARTIFACT_QUANTIZED_SCORER_BIAS,
    INTENT_ARTIFACT_SCALE_HALVING_DIVISOR,
    INTENT_ARTIFACT_SEALED_CANDIDATE_BIAS,
    INTENT_ARTIFACT_SEALED_CANDIDATE_CALIBRATION_BIAS_MAGNITUDE,
    INTENT_ARTIFACT_SEALED_CANDIDATE_CALIBRATION_SCALE,
    INTENT_ARTIFACT_SEALED_CANDIDATE_THRESHOLD_LOGIT,
    INTENT_ARTIFACT_TAMPERED_BIAS_HEX_VALUE,
    INTENT_ARTIFACT_TAMPERED_VETO_FALSE_NEGATIVE_RATE,
    INTENT_ARTIFACT_UNBOUNDED_LOGIT,
    INTENT_BACKOFF_CONFIG_OVERRIDES,
    INTENT_DATASET_EXPECTED_CONTEXT_STRESS_DELTAS,
    INTENT_DATASET_INVALID_TRAINING_EXAMPLE_WEIGHT,
    INTENT_DETERMINISTIC_LOGIT_OFFSET,
    INTENT_DETERMINISTIC_PREDICTION_COVERAGE,
    INTENT_DETERMINISTIC_VETO_THRESHOLD,
    INTENT_DIRECTIONAL_BUDGET_ROWS,
    INTENT_EPOCH_FALSE_POSITIVE_RATE_UPPER_BOUND,
    INTENT_EVALUATION_LM_IMPLAUSIBLE_GRAM_RATIO,
    INTENT_EVALUATION_LM_IMPLAUSIBLE_INVALID_RATIO,
    INTENT_EVALUATION_LM_IMPLAUSIBLE_NGRAM_SCORE,
    INTENT_EVALUATION_LM_IMPLAUSIBLE_WORD_SCORE,
    INTENT_EVALUATION_LM_KNOWN_WORD_SCORE,
    INTENT_EVALUATION_LM_PLAUSIBLE_GRAM_RATIO,
    INTENT_EXTERNAL_COVERAGE_CONFIDENT_LOGIT,
    INTENT_EXTERNAL_COVERAGE_HIGH_PARTIAL,
    INTENT_EXTERNAL_COVERAGE_LOW_PARTIAL,
    INTENT_EXTERNAL_EXPECTED_COVERAGE_MEDIAN,
    INTENT_EXTERNAL_EXPECTED_COVERAGE_P25,
    INTENT_EXTERNAL_EXPECTED_COVERAGE_P75,
    INTENT_EXTERNAL_FIXTURE_THRESHOLD_LOGIT,
    INTENT_EXTERNAL_NEUTRAL_PREDICTION_LOGIT,
    INTENT_EXTERNAL_THRESHOLD_TAMPER_DELTA,
    INTENT_EXTERNAL_VETO_TAMPER_DELTA,
    INTENT_FULL_TYPO_POLICY_KWARGS,
    INTENT_HARD_NEGATIVE_TRAINING_EXAMPLE_WEIGHT,
    INTENT_LEAKAGE_FRACTIONAL_CONTEXT_DELTA,
    INTENT_LEAKAGE_LARGE_CONTEXT_DELTA_MAGNITUDE,
    INTENT_LEAKAGE_POISONED_WEIGHT,
    INTENT_OPTIMIZER_AGGREGATE_NEGATIVE_SCORES,
    INTENT_OPTIMIZER_AGGREGATE_POSITIVE_SCORES,
    INTENT_OPTIMIZER_BACKOFF_EXPECTED_MARGIN,
    INTENT_OPTIMIZER_BACKOFF_NON_PAUSE_SCORE,
    INTENT_OPTIMIZER_CALIBRATION_BIAS_MAGNITUDE,
    INTENT_OPTIMIZER_CALIBRATION_SCALE,
    INTENT_OPTIMIZER_DIRECTIONAL_PLATT_SAMPLES,
    INTENT_OPTIMIZER_EXPECTED_DIRECTIONAL_RUNTIME_LOGIT,
    INTENT_OPTIMIZER_EXPECTED_MARGIN_BASE_LOGIT,
    INTENT_OPTIMIZER_EXPECTED_ORDINARY_TAIL_LOGIT,
    INTENT_OPTIMIZER_EXPECTED_TYPO_TAIL_LOGIT,
    INTENT_OPTIMIZER_EXTREME_SAMPLE_LOGIT,
    INTENT_OPTIMIZER_HIGH_SCORE,
    INTENT_OPTIMIZER_INVALID_FALSE_POSITIVE_RATE,
    INTENT_OPTIMIZER_LOGIT_MARGIN_CAP,
    INTENT_OPTIMIZER_NEGATIVE_LOGIT_STEP_DIVISOR,
    INTENT_OPTIMIZER_NEGATIVE_TAIL_BASE,
    INTENT_OPTIMIZER_NON_PAUSE_POSITIVE_SCORES,
    INTENT_OPTIMIZER_PAUSE_LOGIT_LOWER_BOUND,
    INTENT_OPTIMIZER_PAUSE_LOGIT_MARGIN_PROBE,
    INTENT_OPTIMIZER_PAUSE_POSITIVE_SCORES,
    INTENT_OPTIMIZER_PLATT_THRESHOLD_SAMPLES,
    INTENT_OPTIMIZER_POSITIVE_ROW_SCORE,
    INTENT_OPTIMIZER_PROBE_LOGIT,
    INTENT_OPTIMIZER_RECALL_UNDER_SEALED_MARGIN,
    INTENT_OPTIMIZER_SEALED_VALUE_BY_FIELD,
    INTENT_OPTIMIZER_SECOND_FEATURE_VALUE,
    INTENT_OPTIMIZER_SIGMOID_EDGE_LOGITS,
    INTENT_OPTIMIZER_STANDARD_TEST_PRECISION_FLOOR,
    INTENT_OPTIMIZER_STRICT_PRECISION_FLOOR,
    INTENT_OPTIMIZER_STRICT_TYPO_PRECISION_FLOOR,
    INTENT_OPTIMIZER_TAIL_DIAGNOSTIC_LOGIT,
    INTENT_OPTIMIZER_VETO_MARGIN_PROBE,
    INTENT_PASSING_EPOCH_LOG_LOSS,
    INTENT_PAUSE_MARGIN_CONFIG_OVERRIDES,
    INTENT_PLATT_TEST_L2,
    INTENT_SPY_LOGIT,
    INTENT_SPY_PROBABILITY,
    INTENT_SPY_THRESHOLD,
    INTENT_SPY_VETO_THRESHOLD,
    INTENT_STRICTER_TYPO_CONFIG_OVERRIDES,
    INTENT_STRICT_FALSE_POSITIVE_RATE,
    INTENT_TAIL_DIAGNOSTIC_TEMPLATES,
    INTENT_TAMPERED_METADATA_VALUE,
    INTENT_UNCERTIFIED_EPOCH_LOG_LOSS,
    INTENT_UNCERTIFIED_EPOCH_THRESHOLD_LOGIT,
    INTENT_WEAK_TRAINING_CONFIG_OVERRIDES,
    INTENT_WEAK_TYPO_POLICY_KWARGS,
    INTENT_WILSON_BOUND_GATE_KWARGS,
)
from keyswitch.constants.file_formats import SHA256_HEX_CHARACTERS, VERSION_HASH_CHARACTERS


def directional_calibration(
    scale: float = 1.0,
    bias: float = 0.0,
    *,
    samples_per_direction: int = INTENT_DEFAULT_SAMPLES_PER_DIRECTION,
    positives_per_direction: int = INTENT_DEFAULT_POSITIVES_PER_DIRECTION,
) -> DirectionalPlattCalibration:
    return DirectionalPlattCalibration(
        PlattCalibration(
            scale,
            bias,
            samples_per_direction,
            positives_per_direction,
        ),
        PlattCalibration(
            scale,
            bias,
            samples_per_direction,
            positives_per_direction,
        ),
    )


def runtime_threshold_logits(
    values: Mapping[CorrectionTrigger, float],
) -> dict[CorrectionTrigger, dict[LayoutDirection, float]]:
    return {
        trigger: {
            direction: value for direction in ("0>1", "1>0")
        }
        for trigger, value in values.items()
    }


EXPECTED_PRESEALED_PROVENANCE_CHECK_NAMES: frozenset[str] = frozenset(
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
        "english_source_sha256",
        "russian_source_sha256",
        "embedded_manifest",
        "build_provenance_sha256",
        "model_version",
        "feature_schema",
        "membership_schema",
        "calibration_scope",
        "toolchain_trainer_sha256",
        "toolchain_runtime_sha256",
        "toolchain_detector_sha256",
        "toolchain_protected_tokens_sha256",
        "toolchain_layouts_sha256",
        "toolchain_language_model_sha256",
        "toolchain_evaluator_sha256",
        "toolchain_environment_probe_sha256",
        "toolchain_preseal_generator_sha256",
        "toolchain_development_freezer_sha256",
        "toolchain_preseal_receipt_sha256",
        "hard_negative_development",
    }
)


# Every field of a working TrainingConfig, as one fixture table; config()
# copies it and layers **changes on top so each test can override just the
# fields it cares about.
DEFAULT_TRAINING_CONFIG_VALUES: dict[str, object] = {
    "schema_version": EXPECTED_INTENT_CONFIG_SCHEMA_VERSION,
    "seed": INTENT_TEST_TRAINING_SEED,
    "dimension": INTENT_TEST_DIMENSION,
    "feature_hash_seed": DEFAULT_FNV_SEED,
    "membership_hash_seed": DEFAULT_MEMBERSHIP_FNV_SEED,
    "sources": TrainingSources(
        package="onboard-data",
        package_version="test-version",
        license_declaration="GPL-3+",
        license_evidence=FrozenSourceFile(
            "model/intent_v1/sources/COPYRIGHT.onboard-data",
            "0" * SHA256_HEX_CHARACTERS,
            1,
        ),
        english=FrozenLanguageSource(
            "model/intent_v1/sources/en_US.lm",
            "1" * SHA256_HEX_CHARACTERS,
            1,
            0,
        ),
        russian=FrozenLanguageSource(
            "model/intent_v1/sources/ru_RU.lm",
            "2" * SHA256_HEX_CHARACTERS,
            1,
            1,
        ),
    ),
    "external_evaluation": FrozenExternalEvaluationPolicy(
        schema_version=INTENT_EXTERNAL_POLICY_SCHEMA_VERSION,
        minimum_words_per_group=INTENT_EXTERNAL_MINIMUM_WORDS_PER_GROUP,
        trigger_expansion=TRIGGERS,
        english=FrozenExternalLocalePolicy(
            dictionary_sha256="3" * SHA256_HEX_CHARACTERS,
            dictionary_bytes=1,
            affix_sha256="4" * SHA256_HEX_CHARACTERS,
            affix_bytes=1,
        ),
        russian=FrozenExternalLocalePolicy(
            dictionary_sha256="5" * SHA256_HEX_CHARACTERS,
            dictionary_bytes=1,
            affix_sha256="6" * SHA256_HEX_CHARACTERS,
            affix_bytes=1,
        ),
        lexical_disjoint_corpus_sha256="7" * SHA256_HEX_CHARACTERS,
        unknown_typo_development_corpus_sha256="8" * SHA256_HEX_CHARACTERS,
        unknown_typo_holdout_corpus_sha256="9" * SHA256_HEX_CHARACTERS,
    ),
    "hard_negative_development": HardNegativeDevelopmentPolicy(
        schema_version=INTENT_HARD_NEGATIVE_POLICY_SCHEMA_VERSION,
        source=FrozenSourceFile(
            HARD_NEGATIVE_SOURCE_RELATIVE_PATH,
            "a" * SHA256_HEX_CHARACTERS,
            1,
        ),
        role_namespace=HARD_NEGATIVE_ROLE_NAMESPACE,
        train_words_per_group=INTENT_HARD_NEGATIVE_TRAIN_WORDS_PER_GROUP,
        development_words_per_group=INTENT_HARD_NEGATIVE_DEVELOPMENT_WORDS_PER_GROUP,
        calibration_words_per_group=INTENT_HARD_NEGATIVE_CALIBRATION_WORDS_PER_GROUP,
        threshold_words_per_group=INTENT_HARD_NEGATIVE_THRESHOLD_WORDS_PER_GROUP,
        training_example_weight=INTENT_HARD_NEGATIVE_TRAINING_EXAMPLE_WEIGHT,
    ),
    "sealed_evaluation": SealedEvaluationPolicy(
        schema_version=1,
        split_namespace=SPLIT_NAMESPACE,
        registry_path="model/intent_v1/seal-registry-v23.json",
    ),
    **INTENT_TEST_TRAINING_CONFIG_FIELDS,
}


def config(**changes: object) -> TrainingConfig:
    values: dict[str, object] = dict(DEFAULT_TRAINING_CONFIG_VALUES)
    values.update(changes)
    return TrainingConfig(**values)  # type: ignore[arg-type]


def empty_hard_negative_corpus() -> HardNegativeDevelopmentCorpus:
    rows: dict[SplitName, tuple[LexicalExample, ...]] = {
        split: () for split in (*PRESEALED_SPLITS, *SEALED_TEST_SPLITS)
    }
    role_counts = {
        split: {0: 0, 1: 0}
        for split in (*PRESEALED_SPLITS, *SEALED_TEST_SPLITS)
    }
    return HardNegativeDevelopmentCorpus(
        by_split=rows,
        source_sha256="a" * SHA256_HEX_CHARACTERS,
        source_bytes=1,
        expanded_corpus_sha256="8" * SHA256_HEX_CHARACTERS,
        physical_signatures_sha256="b" * SHA256_HEX_CHARACTERS,
        signature_count=0,
        words_by_group={0: 0, 1: 0},
        role_words_by_group=role_counts,
        training_example_weight=INTENT_HARD_NEGATIVE_TRAINING_EXAMPLE_WEIGHT,
    )


def alphabetic_signature_for_split(
    split: SplitName,
    *,
    minimum_length: int = INTENT_MINIMUM_SIGNATURE_LENGTH,
) -> str:
    """Find a deterministic keyboard-safe fixture for the active namespace."""

    for length in range(max(INTENT_MINIMUM_SIGNATURE_LENGTH, minimum_length), INTENT_SIGNATURE_LENGTH_SEARCH_BOUND):
        for characters in product("abcde", repeat=length):
            signature = "".join(characters)
            if stable_split(signature) == split:
                return signature
    raise AssertionError(f"no alphabetic fixture found for split {split}")


def cross_split_deletion_fixture() -> tuple[str, str]:
    """Find an identity/deletion collision across active pre-sealed splits."""

    for length in range(INTENT_MINIMUM_LENGTH_BEFORE_DELETION, INTENT_SIGNATURE_LENGTH_SEARCH_BOUND):
        for characters in product("abcde", repeat=length):
            base = "".join(characters)
            base_split = stable_split(base)
            if base_split not in PRESEALED_SPLITS:
                continue
            for variant in typo_variants(base, INTENT_TYPO_AUGMENTATION_FIXTURE_LIMIT):
                collision = variant.physical_signature
                collision_split = stable_split(collision)
                if (
                    variant.kind == "deletion"
                    and collision_split in PRESEALED_SPLITS
                    and collision_split != base_split
                ):
                    return base, collision
    raise AssertionError("no cross-split deletion fixture found")


def sealed_to_candidate_variant_fixture() -> tuple[str, SplitName, str, str]:
    """Find a sealed identity whose typo collides with a candidate identity."""

    for length in range(INTENT_MINIMUM_SIGNATURE_LENGTH, INTENT_SIGNATURE_LENGTH_SEARCH_BOUND):
        for characters in product("abcde", repeat=length):
            sealed = "".join(characters)
            if stable_split(sealed) != "test":
                continue
            for variant in typo_variants(sealed, INTENT_TYPO_AUGMENTATION_FIXTURE_LIMIT):
                candidate = variant.physical_signature
                candidate_split = stable_split(candidate)
                if (
                    len(candidate) >= INTENT_MINIMUM_SIGNATURE_LENGTH
                    and candidate_split in PRESEALED_SPLITS
                    and candidate != sealed
                ):
                    return candidate, candidate_split, sealed, variant.kind
    raise AssertionError("no sealed-to-candidate variant fixture found")


def same_split_variant_collision_fixture() -> tuple[str, str, str, SplitName]:
    """Find a typo and identity collision owned by different languages."""

    for length in range(INTENT_MINIMUM_SIGNATURE_LENGTH, INTENT_SIGNATURE_LENGTH_SEARCH_BOUND):
        for characters in product("abcde", repeat=length):
            base = "".join(characters)
            split = stable_split(base)
            for variant in typo_variants(base, INTENT_TYPO_AUGMENTATION_FIXTURE_LIMIT):
                identity = variant.physical_signature
                if (
                    identity != base
                    and len(identity) >= INTENT_MINIMUM_SIGNATURE_LENGTH
                    and stable_split(identity) == split
                ):
                    return base, identity, variant.kind, split
    raise AssertionError("no same-split variant collision fixture found")


def manifest_schema_fixture() -> dict[str, object]:
    """Small top-level sidecar with every versioned field and correct type."""

    return {
        "schema_version": 1,
        "model_id": "keyswitch-layout-intent-v1",
        "calibration_scope": "lexical-synthetic-not-real-world-probability",
        "config_sha256": "0" * SHA256_HEX_CHARACTERS,
        "toolchain": {},
        "dataset_sha256": "1" * SHA256_HEX_CHARACTERS,
        "split_namespace": SPLIT_NAMESPACE,
        "sealed_evaluation": {},
        "source_package": {},
        "sources": [],
        "counts": {},
        "variant_quarantine_sha256": "2" * SHA256_HEX_CHARACTERS,
        "sealed_variant_quarantine_sha256": "3" * SHA256_HEX_CHARACTERS,
        "sealed_test_exclusion_signatures_sha256": "4" * SHA256_HEX_CHARACTERS,
        "training_language_scorer": {},
        "gate_policy": {},
        "training": {},
        "quantization": {},
        "calibration": {},
        "veto": {},
        "thresholds": {},
        "threshold_selection_gate_breakdown": {},
        "sealed_test": {},
        "sealed_test_typos": {},
        "sealed_test_context_stress": {},
        "safety": {},
        "build_provenance_sha256": "5" * SHA256_HEX_CHARACTERS,
        "quality_gate_breakdown": {},
        "quality_gates_passed": True,
        "artifact_sha256": "6" * SHA256_HEX_CHARACTERS,
        "artifact_model_version": "intent-v1-test",
    }


def lexical_example(
    label: bool,
    *,
    trigger: str = "space",
    signature: str = "hello",
    variant: str = "deletion",
    direction: LayoutDirection | None = None,
) -> LexicalExample:
    if trigger not in TRIGGERS:
        raise ValueError("invalid test trigger")
    typed_trigger = next(item for item in TRIGGERS if item == trigger)
    source_group, target_group = (
        (1 if label else 0, 0 if label else 1)
        if direction is None
        else ((0, 1) if direction == "0>1" else (1, 0))
    )
    return LexicalExample(
        original="руддщ" if label else "hello",
        alternative="hello" if label else "руддщ",
        source_group=source_group,
        target_group=target_group,
        trigger=typed_trigger,
        label=label,
        weight=1.0,
        base_signature=signature,
        variant_kind=variant,
        source_known=False,
        target_known=False,
    )


def release_dataset(
    prepared: PreparedLexicon, training_config: TrainingConfig
) -> DatasetBundle:
    candidate = build_dataset(
        prepared,
        training_config,
        included_splits=PRESEALED_SPLITS,
    )
    sealed = build_dataset(
        prepared,
        training_config,
        included_splits=SEALED_TEST_SPLITS,
    )
    return merge_sealed_test_dataset(candidate, sealed)


class EvaluationLanguageModel:
    """Tiny real-policy scorer used to prove model reachability in tests."""


    def __init__(self, locale: str, known_words: set[str]) -> None:
        self.locale = locale
        self.known_words = known_words
        self.speller = SimpleNamespace(
            available=True,
            source=f"/{locale}.dic",
        )

    def score(self, word: str) -> WordScore:
        known = word in self.known_words
        if self.locale == "en_US":
            plausible = bool(word) and all("a" <= character <= "z" for character in word)
        else:
            plausible = bool(word) and all(
                character in "абвгдеёжзийклмнопрстуфхцчшщъыьэюя"
                for character in word
            )
        return WordScore(
            value=INTENT_EVALUATION_LM_KNOWN_WORD_SCORE if known else (0.0 if plausible else INTENT_EVALUATION_LM_IMPLAUSIBLE_WORD_SCORE),
            known=known,
            frequency=INTENT_EVALUATION_LM_KNOWN_WORD_FREQUENCY if known else 0,
            gram_ratio=INTENT_EVALUATION_LM_PLAUSIBLE_GRAM_RATIO if plausible else INTENT_EVALUATION_LM_IMPLAUSIBLE_GRAM_RATIO,
            exact=known,
            spell_known=known,
            ngram_score=0.0 if plausible else INTENT_EVALUATION_LM_IMPLAUSIBLE_NGRAM_SCORE,
            invalid_ratio=0.0 if plausible else INTENT_EVALUATION_LM_IMPLAUSIBLE_INVALID_RATIO,
            raw_ngram_score=0.0 if plausible else INTENT_EVALUATION_LM_IMPLAUSIBLE_NGRAM_SCORE,
        )

    def context_score(self, previous: str, word: str) -> float:
        return 0.0

    def best_single_deletion(self, word: str) -> WordScore:
        return WordScore(
            INTENT_EVALUATION_LM_IMPLAUSIBLE_WORD_SCORE,
            False,
            0,
            0.0,
            ngram_score=INTENT_EVALUATION_LM_IMPLAUSIBLE_NGRAM_SCORE,
            raw_ngram_score=INTENT_EVALUATION_LM_IMPLAUSIBLE_NGRAM_SCORE,
        )


def test_word_scorers() -> dict[int, WordScorer]:
    return {
        0: EvaluationLanguageModel("en_US", {"hello", "keyboard"}),
        1: EvaluationLanguageModel("ru_RU", {"привет", "слово"}),
    }


class SpyIntentModel:

    veto_threshold = INTENT_SPY_VETO_THRESHOLD
    dimension = INTENT_TEST_DIMENSION
    fnv_seed = DEFAULT_FNV_SEED
    membership_seed = DEFAULT_MEMBERSHIP_FNV_SEED

    def __init__(self) -> None:
        self.inputs: list[IntentModelInput] = []

    def predict(self, item: IntentModelInput) -> LinearPrediction:
        self.inputs.append(item)
        return LinearPrediction(
            INTENT_SPY_LOGIT, INTENT_SPY_PROBABILITY, INTENT_SPY_THRESHOLD, 1.0, True, "spy"
        )


class DatasetConstructionTests(unittest.TestCase):

    def test_generated_variants_respect_configured_minimum_length(self) -> None:
        signature = "short"
        split = stable_split(signature)
        prepared = prepare_lexicon(
            (
                LexiconWord(
                    signature,
                    0,
                    INTENT_DATASET_HIGH_FREQUENCY,
                    signature,
                    split,
                ),
            )
        )
        training_config = config(
            minimum_word_length=INTENT_DATASET_MINIMUM_WORD_LENGTH,
            typo_augmentations=INTENT_TYPO_AUGMENTATION_FIXTURE_LIMIT,
        )

        quarantine = build_variant_quarantine(
            prepared,
            training_config,
            included_splits=(split,),
        )
        dataset = build_dataset(
            prepared,
            training_config,
            included_splits=(split,),
        )

        self.assertFalse(
            any(
                item.base_signature == signature
                and item.variant_kind == "deletion"
                for item in quarantine.occurrences
            )
        )
        lexical_rows = tuple(
            item
            for item in dataset.by_split[split]
            if item.base_signature == signature
        )
        self.assertTrue(lexical_rows)
        self.assertNotIn(
            "deletion", {item.variant_kind for item in lexical_rows}
        )
        self.assertTrue(
            all(
                len(physical_signature(item.original, item.source_group))
                >= INTENT_DATASET_MINIMUM_WORD_LENGTH
                for item in lexical_rows
            )
        )

    def test_sealed_words_cannot_change_candidate_quarantine_or_rows(self) -> None:
        candidate_signature, candidate_split, sealed_signature, variant_kind = (
            sealed_to_candidate_variant_fixture()
        )
        self.assertIn(candidate_split, PRESEALED_SPLITS)
        self.assertEqual(stable_split(sealed_signature), "test")
        self.assertIn(
            (candidate_signature, variant_kind),
            tuple(
                (item.physical_signature, item.kind)
                for item in typo_variants(sealed_signature, INTENT_TYPO_AUGMENTATION_FIXTURE_LIMIT)
            ),
        )
        train_word = LexiconWord(
            candidate_signature,
            0,
            INTENT_DATASET_HIGH_FREQUENCY,
            candidate_signature,
            candidate_split,
        )
        test_word = LexiconWord(
            sealed_signature,
            0,
            INTENT_DATASET_UPPER_MID_FREQUENCY,
            sealed_signature,
            "test",
        )
        training_config = config(typo_augmentations=INTENT_TYPO_AUGMENTATION_FIXTURE_LIMIT)
        baseline_prepared = prepare_lexicon((train_word,))
        changed_prepared = prepare_lexicon((train_word, test_word))
        baseline = build_dataset(
            baseline_prepared,
            training_config,
            included_splits=PRESEALED_SPLITS,
        )
        changed = build_dataset(
            changed_prepared,
            training_config,
            included_splits=PRESEALED_SPLITS,
        )
        self.assertEqual(baseline, changed)
        self.assertEqual(
            dataset_fingerprint(baseline), dataset_fingerprint(changed)
        )
        self.assertEqual(
            sum(
                item.base_signature == candidate_signature
                and item.variant_kind == "identity"
                for item in changed.by_split[candidate_split]
            ),
            (
                INTENT_DATASET_DIRECTION_PAIR_COUNT
                if candidate_split in ("train", "development")
                else INTENT_DATASET_DIRECTION_PAIR_COUNT * len(TRIGGERS)
            ),
        )

        sealed = build_dataset(
            changed_prepared,
            training_config,
            included_splits=SEALED_TEST_SPLITS,
        )
        merged = merge_sealed_test_dataset(changed, sealed)
        assert_no_split_leakage(merged)
        self.assertIn(
            candidate_signature,
            merged.sealed_test_exclusion_signatures,
        )
        self.assertTrue(
            all(
                physical_signature(item.original, item.source_group)
                != candidate_signature
                for item in merged.by_split["test"]
            )
        )

    def test_sealed_merge_rejects_phase_contamination_and_malformed_rows(
        self,
    ) -> None:
        empty_rows: dict[SplitName, tuple[LexicalExample, ...]] = {
            split: ()
            for split in (*PRESEALED_SPLITS, *SEALED_TEST_SPLITS)
        }
        presealed = DatasetBundle(dict(empty_rows), ())
        sealed = DatasetBundle(dict(empty_rows), ())
        row = lexical_example(False)

        presealed_with_test = DatasetBundle(
            {**empty_rows, "test": (row,)},
            (),
        )
        with self.assertRaisesRegex(ValueError, "contains test rows"):
            merge_sealed_test_dataset(presealed_with_test, sealed)

        sealed_with_candidate = DatasetBundle(
            {**empty_rows, "train": (row,)},
            (),
        )
        with self.assertRaisesRegex(ValueError, "contains candidate rows"):
            merge_sealed_test_dataset(presealed, sealed_with_candidate)

        for name, left, right in (
            (
                "presealed",
                replace(
                    presealed,
                    sealed_test_exclusion_signatures=("hello",),
                ),
                sealed,
            ),
            (
                "sealed",
                presealed,
                replace(
                    sealed,
                    sealed_test_exclusion_signatures=("hello",),
                ),
            ),
        ):
            with self.subTest(metadata=name):
                with self.assertRaisesRegex(
                    ValueError, "unexpected sealed metadata"
                ):
                    merge_sealed_test_dataset(left, right)

        malformed = replace(row, alternative="цщкдв")
        malformed_cases = (
            (
                "presealed",
                DatasetBundle({**empty_rows, "train": (malformed,)}, ()),
                sealed,
                "malformed presealed row",
            ),
            (
                "sealed-test",
                presealed,
                DatasetBundle({**empty_rows, "test": (malformed,)}, ()),
                "malformed sealed test row",
            ),
            (
                "safety",
                replace(presealed, safety=(malformed,)),
                sealed,
                "malformed safety row",
            ),
        )
        for name, left, right, message in malformed_cases:
            with self.subTest(malformed=name):
                with self.assertRaisesRegex(ValueError, message):
                    merge_sealed_test_dataset(left, right)

    def test_sealed_merge_excludes_candidate_quarantine_signatures(self) -> None:
        rows: dict[SplitName, tuple[LexicalExample, ...]] = {
            split: () for split in (*PRESEALED_SPLITS, *SEALED_TEST_SPLITS)
        }
        occurrences = (
            QuarantinedVariantOccurrence(
                "hello", "candidate-a", "train", 0, "deletion", "cross_split"
            ),
            QuarantinedVariantOccurrence(
                "hello",
                "candidate-b",
                "development",
                0,
                "identity",
                "cross_split",
            ),
        )
        quarantine = VariantQuarantine(
            occurrences,
            variant_quarantine_fingerprint(occurrences),
        )
        presealed = DatasetBundle(rows, (), quarantine)
        sealed = DatasetBundle(
            {**rows, "test": (lexical_example(False, signature="sealed"),)},
            (),
        )

        merged = merge_sealed_test_dataset(presealed, sealed)

        self.assertEqual(merged.by_split["test"], ())
        self.assertEqual(merged.sealed_test_exclusion_signatures, ("hello",))
        assert_no_split_leakage(merged)

    def test_merged_audit_keeps_sealed_quarantine_evidence_phase_local(
        self,
    ) -> None:
        rows: dict[SplitName, tuple[LexicalExample, ...]] = {
            split: () for split in (*PRESEALED_SPLITS, *SEALED_TEST_SPLITS)
        }
        candidate = DatasetBundle(
            {**rows, "train": (lexical_example(False, signature="candidate"),)},
            (),
        )
        occurrences = (
            QuarantinedVariantOccurrence(
                "hello", "sealed-en", "test", 0, "identity", "cross_language"
            ),
            QuarantinedVariantOccurrence(
                "hello", "sealed-ru", "test", 1, "deletion", "cross_language"
            ),
        )
        sealed = DatasetBundle(
            rows,
            (),
            VariantQuarantine(
                occurrences,
                variant_quarantine_fingerprint(occurrences),
            ),
        )
        assert_no_split_leakage(
            sealed,
            variant_quarantine_splits=SEALED_TEST_SPLITS,
        )

        merged = merge_sealed_test_dataset(candidate, sealed)

        assert_no_split_leakage(merged)

    def test_physical_split_dedup_collision_and_no_leakage(self) -> None:
        pair = LayoutPair()
        russian_collision = pair.translate("test", "us", "ru")
        words = (
            LexiconWord("test", 0, INTENT_DATASET_HIGH_FREQUENCY, "test", stable_split("test")),
            LexiconWord("test", 0, INTENT_DATASET_LOW_FREQUENCY, "test", stable_split("test")),
            LexiconWord(
                russian_collision,
                1,
                INTENT_DATASET_MID_FREQUENCY,
                physical_signature(russian_collision, 1),
                stable_split("test"),
            ),
            LexiconWord("hello", 0, INTENT_DATASET_UPPER_MID_FREQUENCY, "hello", stable_split("hello")),
            LexiconWord(
                "привет",
                1,
                INTENT_DATASET_LOWER_MID_FREQUENCY,
                physical_signature("привет", 1),
                stable_split(physical_signature("привет", 1)),
            ),
        )
        prepared = prepare_lexicon(words)
        self.assertEqual(len(prepared.collisions), 1)
        self.assertEqual(prepared.collisions[0].physical_signature, "test")
        self.assertEqual(
            sum(len(items) for items in prepared.words_by_split.values()),
            INTENT_DATASET_EXPECTED_PREPARED_WORD_COUNT,
        )
        dataset = release_dataset(prepared, config())
        assert_no_split_leakage(dataset)
        self.assertTrue(dataset.safety)
        self.assertTrue(all(not item.label and item.safety for item in dataset.safety))
        self.assertEqual(dataset_fingerprint(dataset), dataset_fingerprint(dataset))

        owner = stable_split("hello")
        leaked = next(
            item
            for item in dataset.by_split[owner]
            if item.base_signature == "hello"
        )
        other = next(name for name in dataset.by_split if name != owner)
        broken = {
            name: items + ((leaked,) if name == other else ())
            for name, items in dataset.by_split.items()
        }
        with self.assertRaisesRegex(ValueError, "leaked"):
            assert_no_split_leakage(DatasetBundle(broken, dataset.safety))

    def test_short_frozen_collisions_remain_only_in_safety_corpus(self) -> None:
        pair = LayoutPair()
        collision_signature = "keys"
        collision = (
            LexiconWord(
                collision_signature,
                0,
                INTENT_DATASET_HIGH_FREQUENCY,
                collision_signature,
                stable_split(collision_signature),
            ),
            LexiconWord(
                pair.translate(collision_signature, "us", "ru"),
                1,
                INTENT_DATASET_UPPER_MID_FREQUENCY,
                collision_signature,
                stable_split(collision_signature),
            ),
        )
        long_signature = "keyboard"
        prepared = prepare_lexicon(
            (
                *collision,
                LexiconWord(
                    long_signature,
                    0,
                    INTENT_DATASET_MID_FREQUENCY,
                    long_signature,
                    stable_split(long_signature),
                ),
            ),
            minimum_training_signature_length=INTENT_DATASET_MINIMUM_WORD_LENGTH,
        )

        self.assertEqual(
            tuple(item.physical_signature for item in prepared.collisions),
            (collision_signature,),
        )
        self.assertEqual(
            tuple(
                item.physical_signature
                for rows in prepared.words_by_split.values()
                for item in rows
            ),
            (long_signature,),
        )
        dataset = release_dataset(
            prepared,
            config(minimum_word_length=INTENT_DATASET_MINIMUM_WORD_LENGTH),
        )
        matching = tuple(
            item
            for item in dataset.safety
            if item.base_signature == collision_signature
        )
        self.assertEqual(len(matching), INTENT_DATASET_DIRECTION_PAIR_COUNT * len(TRIGGERS))
        audit = audit_guarded_safety_corpus(dataset.safety)
        self.assertGreater(audit.lexical_collision_samples, 0)
        self.assertEqual(audit.lexical_collision_triggers, TRIGGERS)

    def test_cross_split_augmented_signatures_are_quarantined_before_rows(self) -> None:
        base, collision = cross_split_deletion_fixture()
        base_split = stable_split(base)
        collision_split = stable_split(collision)
        self.assertNotEqual(base_split, collision_split)
        words = (
            LexiconWord(base, 0, INTENT_DATASET_HIGH_FREQUENCY, base, base_split),
            LexiconWord(collision, 0, INTENT_DATASET_UPPER_MID_FREQUENCY, collision, collision_split),
        )
        prepared = prepare_lexicon(words)
        training_config = config(typo_augmentations=INTENT_TYPO_AUGMENTATION_FIXTURE_LIMIT)
        quarantine = build_variant_quarantine(
            prepared,
            training_config,
            included_splits=PRESEALED_SPLITS,
        )
        matching = tuple(
            item
            for item in quarantine.occurrences
            if item.physical_signature == collision
        )
        self.assertEqual(
            {(item.base_signature, item.variant_kind) for item in matching},
            {(base, "deletion"), (collision, "identity")},
        )
        self.assertEqual(
            {item.split for item in matching}, {base_split, collision_split}
        )
        self.assertEqual(
            quarantine.sha256,
            variant_quarantine_fingerprint(quarantine.occurrences),
        )

        first = release_dataset(prepared, training_config)
        second = release_dataset(prepared, training_config)
        self.assertEqual(first.variant_quarantine, quarantine)
        self.assertEqual(first, second)
        emitted_signatures = {
            physical_signature(item.original, item.source_group)
            or physical_signature(item.alternative, item.target_group)
            for rows in first.by_split.values()
            for item in rows
            if item.variant_kind in {
                "identity",
                "deletion",
                "duplication",
                "transposition",
            }
        }
        self.assertNotIn(collision, emitted_signatures)
        self.assertFalse(
            any(
                item.base_signature == "aaac" and item.variant_kind == "deletion"
                for rows in first.by_split.values()
                for item in rows
            )
        )
        self.assertFalse(
            any(
                item.base_signature == "aad" and item.variant_kind == "identity"
                for rows in first.by_split.values()
                for item in rows
            )
        )
        audit = audit_dataset_physical_signatures(first)
        self.assertGreater(audit.audited_rows, 0)
        self.assertEqual(audit.cross_split_signatures, ())
        self.assertEqual(audit.cross_language_signatures, ())
        self.assertEqual(audit.safety_overlap_signatures, ())
        self.assertEqual(audit.safety_base_signature_overlaps, ())
        self.assertEqual(audit.malformed_rows, ())
        self.assertEqual(audit.quarantined_signatures_present, ())
        assert_no_split_leakage(first)
        self.assertNotEqual(
            dataset_fingerprint(first),
            dataset_fingerprint(DatasetBundle(first.by_split, first.safety)),
        )

    def test_full_row_audit_detects_leak_hidden_by_distinct_base_signatures(self) -> None:
        first = lexical_example(False, signature="base-in-train")
        second = lexical_example(False, signature="base-in-test")
        rows: dict[SplitName, tuple[LexicalExample, ...]] = {
            "train": (first,),
            "development": (),
            "calibration": (),
            "threshold": (),
            "test": (second,),
        }
        broken = DatasetBundle(rows, ())
        audit = audit_dataset_physical_signatures(broken)
        self.assertEqual(
            audit.cross_split_signatures,
            (("hello", ("test", "train")),),
        )
        with self.assertRaisesRegex(ValueError, "actual augmented physical signature"):
            assert_no_split_leakage(broken)

    def test_same_split_cross_language_variant_collision_is_quarantined(self) -> None:
        pair = LayoutPair()
        base, identity, variant_kind, split = (
            same_split_variant_collision_fixture()
        )
        russian_word = pair.translate(identity, "us", "ru")
        self.assertEqual(stable_split(base), split)
        self.assertEqual(stable_split(identity), split)
        prepared = prepare_lexicon(
            (
                LexiconWord(base, 0, INTENT_DATASET_HIGH_FREQUENCY, base, split),
                LexiconWord(russian_word, 1, INTENT_DATASET_UPPER_MID_FREQUENCY, identity, split),
            )
        )
        training_config = config(typo_augmentations=INTENT_TYPO_AUGMENTATION_FIXTURE_LIMIT)
        dataset = release_dataset(prepared, training_config)
        matching = tuple(
            item
            for item in dataset.variant_quarantine.occurrences
            if item.physical_signature == identity
        )
        self.assertEqual(
            {(item.base_signature, item.variant_kind, item.group) for item in matching},
            {(base, variant_kind, 0), (identity, "identity", 1)},
        )
        self.assertEqual({item.reason for item in matching}, {"cross_language"})
        self.assertFalse(
            any(
                (
                    physical_signature(item.original, item.source_group)
                    or physical_signature(item.alternative, item.target_group)
                )
                == identity
                for rows_for_split in dataset.by_split.values()
                for item in rows_for_split
                if item.variant_kind
                in {"identity", "deletion", "duplication", "transposition"}
            )
        )
        assert_no_split_leakage(dataset)

        english_claim = LexicalExample(
            original=identity,
            alternative=russian_word,
            source_group=0,
            target_group=1,
            trigger="space",
            label=False,
            weight=1.0,
            base_signature=base,
            variant_kind=variant_kind,
            source_known=False,
            target_known=False,
        )
        russian_claim = LexicalExample(
            original=russian_word,
            alternative=identity,
            source_group=1,
            target_group=0,
            trigger="space",
            label=False,
            weight=1.0,
            base_signature=identity,
            variant_kind="identity",
            source_known=False,
            target_known=False,
        )
        contradictory_rows: dict[SplitName, tuple[LexicalExample, ...]] = {
            "train": (),
            "development": (),
            "calibration": (),
            "threshold": (),
            "test": (),
        }
        contradictory_rows[split] = (english_claim, russian_claim)
        contradictory = DatasetBundle(contradictory_rows, ())
        audit = audit_dataset_physical_signatures(contradictory)
        self.assertEqual(
            audit.cross_language_signatures,
            ((identity, (0, 1)),),
        )
        with self.assertRaisesRegex(ValueError, "contradictory intended languages"):
            assert_no_split_leakage(contradictory)

    def test_augmented_signature_cannot_overlap_safety_collision(self) -> None:
        pair = LayoutPair()
        base_signature, collision_signature = cross_split_deletion_fixture()
        russian_collision = pair.translate(collision_signature, "us", "ru")
        prepared = prepare_lexicon(
            (
                LexiconWord(
                    base_signature,
                    0,
                    INTENT_DATASET_HIGH_FREQUENCY,
                    base_signature,
                    stable_split(base_signature),
                ),
                LexiconWord(
                    collision_signature,
                    0,
                    INTENT_DATASET_UPPER_MID_FREQUENCY,
                    collision_signature,
                    stable_split(collision_signature),
                ),
                LexiconWord(
                    russian_collision,
                    1,
                    INTENT_DATASET_MID_FREQUENCY,
                    collision_signature,
                    stable_split(collision_signature),
                ),
            )
        )
        self.assertEqual(
            {item.physical_signature for item in prepared.collisions},
            {collision_signature},
        )
        dataset = release_dataset(
            prepared, config(typo_augmentations=INTENT_TYPO_AUGMENTATION_FIXTURE_LIMIT)
        )
        quarantined = tuple(
            item
            for item in dataset.variant_quarantine.occurrences
            if item.physical_signature == collision_signature
        )
        self.assertEqual(
            {(item.base_signature, item.variant_kind, item.reason) for item in quarantined},
            {(base_signature, "deletion", "cross_language")},
        )
        self.assertFalse(
            any(
                item.base_signature == base_signature
                and item.variant_kind == "deletion"
                for rows in dataset.by_split.values()
                for item in rows
            )
        )
        assert_no_split_leakage(dataset)

        leaked = LexicalExample(
            original=collision_signature,
            alternative=russian_collision,
            source_group=0,
            target_group=1,
            trigger="space",
            label=False,
            weight=1.0,
            base_signature=base_signature,
            variant_kind="deletion",
            source_known=False,
            target_known=False,
        )
        broken_rows = dict(dataset.by_split)
        broken_rows[stable_split(base_signature)] += (leaked,)
        broken = DatasetBundle(
            broken_rows,
            dataset.safety,
            dataset.variant_quarantine,
        )
        audit = audit_dataset_physical_signatures(broken)
        self.assertEqual(
            audit.safety_overlap_signatures, (collision_signature,)
        )
        with self.assertRaisesRegex(ValueError, "overlaps the safety corpus"):
            assert_no_split_leakage(broken)

    def test_curated_hard_negative_cannot_leak_into_lexical_split(self) -> None:
        token = next(
            candidate
            for candidate in TRAINING_HARD_NEGATIVES
            if stable_split(candidate.casefold()) in PRESEALED_SPLITS
            and stable_split(f"hard:{candidate.casefold()}")
            in PRESEALED_SPLITS
            and stable_split(candidate.casefold())
            != stable_split(f"hard:{candidate.casefold()}")
        )
        signature = token.casefold()
        lexical_split = stable_split(signature)
        hard_negative_split = stable_split(f"hard:{signature}")
        self.assertNotEqual(lexical_split, hard_negative_split)
        prepared = prepare_lexicon(
            (
                LexiconWord(signature, 0, INTENT_DATASET_HIGH_FREQUENCY, signature, lexical_split),
            )
        )
        dataset = release_dataset(
            prepared, config(typo_augmentations=INTENT_TYPO_AUGMENTATION_FIXTURE_LIMIT)
        )
        matching = tuple(
            item
            for item in dataset.variant_quarantine.occurrences
            if item.physical_signature == signature
        )
        self.assertEqual(
            {(item.base_signature, item.variant_kind, item.reason) for item in matching},
            {(signature, "identity", "cross_split")},
        )
        self.assertTrue(
            any(
                item.base_signature == f"hard:{signature}"
                and item.variant_kind == "protected"
                for item in dataset.by_split[hard_negative_split]
            )
        )
        self.assertFalse(
            any(
                item.base_signature == signature
                and item.variant_kind == "identity"
                for rows in dataset.by_split.values()
                for item in rows
            )
        )
        assert_no_split_leakage(dataset)

    def test_augmentation_and_trigger_are_stable(self) -> None:
        variants = typo_variants("keyboard", INTENT_TYPO_AUGMENTATION_FIXTURE_LIMIT)
        self.assertEqual(variants, typo_variants("keyboard", INTENT_TYPO_AUGMENTATION_FIXTURE_LIMIT))
        self.assertEqual(
            {item.kind for item in variants},
            {"identity", "deletion", "duplication", "transposition"},
        )
        self.assertEqual(len(typo_variants("ab", INTENT_TYPO_AUGMENTATION_FIXTURE_LIMIT)), 1)
        self.assertIn(deterministic_training_trigger("keyboard"), TRIGGERS)

    def test_context_stress_profiles_are_fixed_nonempty_and_label_independent(
        self,
    ) -> None:
        self.assertEqual(len(CONTEXT_STRESS_PROFILES), INTENT_DATASET_EXPECTED_CONTEXT_STRESS_PROFILE_COUNT)
        expected_deltas = INTENT_DATASET_EXPECTED_CONTEXT_STRESS_DELTAS
        self.assertEqual(
            {
                profile.delta
                for profile in CONTEXT_STRESS_PROFILES
                if profile.group_selector == "source"
            },
            expected_deltas,
        )
        self.assertEqual(
            {
                profile.delta
                for profile in CONTEXT_STRESS_PROFILES
                if profile.group_selector == "target"
            },
            expected_deltas,
        )
        self.assertEqual(
            len({profile.name for profile in CONTEXT_STRESS_PROFILES}),
            len(CONTEXT_STRESS_PROFILES),
        )
        positive = lexical_example(True)
        negative = replace(positive, label=False)
        for profile in CONTEXT_STRESS_PROFILES:
            stressed = context_stress_examples(
                (positive, negative), profile
            )
            self.assertEqual(len(stressed), INTENT_DATASET_EXPECTED_STRESSED_EXAMPLE_COUNT)
            self.assertEqual(stressed[0].context_delta, profile.delta)
            self.assertNotEqual(stressed[0].context_group, None)
            self.assertEqual(
                (
                    stressed[0].context_delta,
                    stressed[0].context_group,
                ),
                (
                    stressed[1].context_delta,
                    stressed[1].context_group,
                ),
            )

    def test_every_dataset_split_has_neutral_context(self) -> None:
        signatures: dict[SplitName, str] = {
            split: alphabetic_signature_for_split(split)
            for split in tim.SPLIT_NAMES
        }
        for split, signature in signatures.items():
            self.assertEqual(stable_split(signature), split)
            prepared = prepare_lexicon(
                (
                    LexiconWord(
                        signature,
                        0,
                        INTENT_DATASET_HIGH_FREQUENCY,
                        signature,
                        split,
                    ),
                )
            )
            dataset = release_dataset(
                prepared, config(typo_augmentations=1)
            )
            matching = tuple(
                item
                for item in dataset.by_split[split]
                if item.base_signature == signature
            )
            self.assertTrue(matching)
            self.assertTrue(
                all(
                    item.context_delta == 0.0
                    and item.context_group is None
                    for item in dataset.by_split[split]
                )
            )

    def test_arpa_loading_filters_and_hashes_tiny_lexicon(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "en_US.lm"
            path.write_text(
                "\\data\\\n\\1-grams:\n10 Hello\n5 hello\n7 key\n4 a\n3 <s>\n"
                "bad world\n8 abc123\n\\2-grams:\n9 hello world\n",
                encoding="utf-8",
            )
            words, source = load_onboard_unigrams(
                path,
                "en_US",
                0,
                config(minimum_word_length=INTENT_DATASET_MINIMUM_WORD_LENGTH),
            )
            self.assertEqual(
                [(word.word, word.frequency) for word in words],
                [("hello", INTENT_DATASET_EXPECTED_HELLO_FREQUENCY)],
            )
            expanded, _ = load_onboard_unigrams(
                path,
                "en_US",
                0,
                config(minimum_word_length=INTENT_DATASET_MINIMUM_WORD_LENGTH),
                minimum_word_length=INTENT_DATASET_FUNCTION_MINIMUM_WORD_LENGTH,
            )
            self.assertEqual(
                [(word.word, word.frequency) for word in expanded],
                [("hello", INTENT_DATASET_EXPECTED_HELLO_FREQUENCY), ("key", INTENT_DATASET_EXPECTED_KEY_FREQUENCY)],
            )
            with self.assertRaisesRegex(ValueError, "between two"):
                load_onboard_unigrams(
                    path,
                    "en_US",
                    0,
                    config(),
                    minimum_word_length=1,
                )
            self.assertEqual(source.sha256, __import__("hashlib").sha256(path.read_bytes()).hexdigest())
            self.assertEqual(source.license_declaration, "GPL-3+")

    def test_frozen_hard_negative_corpus_is_complete_disjoint_and_audited(
        self,
    ) -> None:
        repository = Path(__file__).resolve().parents[1]
        training_config = load_training_config(
            repository / "model/intent_v1/config.json"
        )
        source = (
            repository
            / training_config.hard_negative_development.source.path
        )
        corpus = load_hard_negative_development_corpus(
            source, training_config
        )
        expected_rows = INTENT_DATASET_EXPECTED_HARD_NEGATIVE_SPLIT_ROWS
        self.assertEqual(
            {
                split: len(rows)
                for split, rows in corpus.by_split.items()
            },
            expected_rows,
        )
        self.assertEqual(corpus.signature_count, INTENT_DATASET_EXPECTED_HARD_NEGATIVE_SIGNATURE_COUNT)
        self.assertEqual(corpus.words_by_group, INTENT_DATASET_EXPECTED_HARD_NEGATIVE_WORDS_BY_GROUP)
        self.assertEqual(
            corpus.expanded_corpus_sha256,
            training_config.external_evaluation
            .unknown_typo_development_corpus_sha256,
        )
        role_signatures = {
            split: {
                row.base_signature.removeprefix("hunspell-unknown:")
                for row in rows
            }
            for split, rows in corpus.by_split.items()
        }
        self.assertEqual(
            {split: len(values) for split, values in role_signatures.items()},
            INTENT_DATASET_EXPECTED_HARD_NEGATIVE_ROLE_SIGNATURES,
        )
        observed: set[str] = set()
        for split in PRESEALED_SPLITS:
            self.assertTrue(observed.isdisjoint(role_signatures[split]))
            observed.update(role_signatures[split])
        self.assertEqual(len(observed), INTENT_DATASET_EXPECTED_HARD_NEGATIVE_SIGNATURE_COUNT)
        self.assertEqual(
            tim.physical_signature_set_fingerprint(observed),
            corpus.physical_signatures_sha256,
        )
        flattened = tuple(
            replace(row, weight=1.0)
            for split in PRESEALED_SPLITS
            for row in corpus.by_split[split]
        )
        self.assertEqual(
            tim.external_corpus_fingerprint(flattened),
            corpus.expanded_corpus_sha256,
        )
        self.assertEqual(corpus.training_example_weight, INTENT_HARD_NEGATIVE_TRAINING_EXAMPLE_WEIGHT)
        self.assertEqual(
            {row.weight for row in corpus.by_split["train"]}, {INTENT_HARD_NEGATIVE_TRAINING_EXAMPLE_WEIGHT}
        )
        for split in ("development", "calibration", "threshold"):
            self.assertEqual(
                {row.weight for row in corpus.by_split[split]}, {1.0}
            )
        empty = DatasetBundle(
            {
                split: ()
                for split in (*PRESEALED_SPLITS, *SEALED_TEST_SPLITS)
            },
            (),
        )
        merged = merge_hard_negative_development(empty, corpus)
        assert_no_split_leakage(merged)
        audit = audit_dataset_physical_signatures(merged)
        self.assertEqual(audit.audited_rows, INTENT_DATASET_EXPECTED_MERGED_HARD_NEGATIVE_AUDITED_ROWS)
        self.assertEqual(audit.cross_split_signatures, ())
        self.assertEqual(audit.cross_language_signatures, ())
        self.assertEqual(audit.malformed_rows, ())

    def test_frozen_hard_negative_corpus_rejects_tampering_and_non_json_values(
        self,
    ) -> None:
        repository = Path(__file__).resolve().parents[1]
        training_config = load_training_config(
            repository / "model/intent_v1/config.json"
        )
        source = (
            repository
            / training_config.hard_negative_development.source.path
        )

        def config_for(raw: bytes) -> TrainingConfig:
            policy = replace(
                training_config.hard_negative_development,
                source=FrozenSourceFile(
                    HARD_NEGATIVE_SOURCE_RELATIVE_PATH,
                    hashlib.sha256(raw).hexdigest(),
                    len(raw),
                ),
            )
            return replace(
                training_config,
                hard_negative_development=policy,
            )

        decoded = cast(
            dict[str, object], json.loads(source.read_text(encoding="utf-8"))
        )
        rows = cast(list[object], decoded["rows"])
        first = dict(cast(dict[str, object], rows[0]))
        first["correct_typo"] = cast(str, first["correct_typo"]) + "x"
        tampered = {**decoded, "rows": [first, *rows[1:]]}
        tampered_bytes = (
            json.dumps(tampered, ensure_ascii=False, indent=HARD_NEGATIVE_CORPUS_JSON_INDENT) + "\n"
        ).encode("utf-8")
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "hard-negative.json"
            path.write_bytes(tampered_bytes)
            with self.assertRaisesRegex(
                ValueError, "rendered physical signature changed"
            ):
                load_hard_negative_development_corpus(
                    path, config_for(tampered_bytes)
                )

            invalid_cases = (
                (
                    b'{"schema_version":NaN}',
                    "forbidden JSON constant NaN",
                ),
                (
                    b'{"schema_version":1,"schema_version":1}',
                    "duplicate key 'schema_version'",
                ),
            )
            for raw, message in invalid_cases:
                with self.subTest(message=message):
                    path.write_bytes(raw)
                    with self.assertRaisesRegex(ValueError, message):
                        load_hard_negative_development_corpus(
                            path, config_for(raw)
                        )

    def test_hard_negative_policy_fails_closed(self) -> None:
        baseline = config().hard_negative_development
        invalid = (
            (
                replace(baseline, schema_version=0),
                "unsupported hard-negative development schema",
            ),
            (
                replace(
                    baseline,
                    source=replace(baseline.source, path="other.json"),
                ),
                "source must match the versioned path",
            ),
            (
                replace(baseline, role_namespace="other"),
                "role namespace must match v23",
            ),
            (
                replace(baseline, train_words_per_group=0),
                "role counts must be positive integers",
            ),
            (
                replace(baseline, train_words_per_group=INTENT_HARD_NEGATIVE_UNDERFILLED_TRAIN_WORDS_PER_GROUP),
                "role counts must exhaust",
            ),
            (
                replace(baseline, training_example_weight=float("nan")),
                "training example weight must be finite",
            ),
            (
                replace(baseline, training_example_weight=INTENT_DATASET_INVALID_TRAINING_EXAMPLE_WEIGHT),
                "training example weight must be finite",
            ),
        )
        for policy, message in invalid:
            with self.subTest(message=message):
                with self.assertRaisesRegex(ValueError, message):
                    policy.validate(expected_words_per_group=INTENT_EXTERNAL_MINIMUM_WORDS_PER_GROUP)


class LeakageAndFeatureTests(unittest.TestCase):

    def test_training_adapter_exactly_matches_runtime_feature_contract(self) -> None:
        scorers = test_word_scorers()
        hash_seed = DEFAULT_FNV_SEED ^ INTENT_HASH_SEED_XOR_PERTURBATION
        membership_seed = DEFAULT_MEMBERSHIP_FNV_SEED ^ INTENT_MEMBERSHIP_SEED_XOR_PERTURBATION
        adapter = runtime_feature_extractor(
            hash_seed,
            membership_seed,
            scorers=scorers,
        )
        base = lexical_example(True)
        for source_group, target_group in ((0, 1), (1, 0)):
            for trigger in TRIGGERS:
                for context_delta, context_group in (
                    (-INTENT_LEAKAGE_LARGE_CONTEXT_DELTA_MAGNITUDE, None),
                    (-1.0, source_group),
                    (0.0, target_group),
                    (INTENT_LEAKAGE_FRACTIONAL_CONTEXT_DELTA, INTENT_LEAKAGE_ARBITRARY_CONTEXT_GROUP),
                    (INTENT_LEAKAGE_LARGE_CONTEXT_DELTA_MAGNITUDE, None),
                ):
                    row = replace(
                        base,
                        original="ghbdtn" if source_group == 0 else "руддщ",
                        alternative="привет" if target_group == 1 else "hello",
                        source_group=source_group,
                        target_group=target_group,
                        trigger=trigger,
                        context_delta=context_delta,
                        context_group=context_group,
                    )
                    direct_input = IntentModelInput(
                        original=row.original,
                        alternative=row.alternative,
                        source_group=row.source_group,
                        target_group=row.target_group,
                        trigger=row.trigger,
                        source_score=scorers[row.source_group].score(row.original),
                        target_score=scorers[row.target_group].score(row.alternative),
                        context_delta=row.context_delta,
                        context_group=row.context_group,
                    )
                    direct = extract_features(
                        direct_input,
                        dimension=INTENT_LEAKAGE_ALTERNATE_TEST_DIMENSION,
                        hash_seed=hash_seed,
                        membership_seed=membership_seed,
                        ngram_orders=NGRAM_ORDERS,
                    )
                    self.assertEqual(
                        adapter(row, INTENT_LEAKAGE_ALTERNATE_TEST_DIMENSION),
                        ExtractedExampleFeatures(
                            direct.values,
                            direct.character_fingerprints,
                        ),
                    )

    def test_unknown_pairs_have_no_label_derived_dense_or_state_signal(self) -> None:
        scorers = test_word_scorers()
        positive = lexical_example(True)
        poisoned_metadata = replace(
            positive,
            label=False,
            weight=INTENT_LEAKAGE_POISONED_WEIGHT,
            base_signature="poisoned",
            variant_kind="identity",
            source_known=True,
            target_known=True,
            frequency=INTENT_LEAKAGE_POISONED_FREQUENCY,
            protected=True,
            safety=True,
        )
        source_positive = training_word_score(
            positive.original,
            positive.source_group,
            scorers=scorers,
        )
        target_positive = training_word_score(
            positive.alternative,
            positive.target_group,
            scorers=scorers,
        )
        self.assertNotEqual(source_positive, target_positive)
        self.assertEqual(
            intent_input_for_example(positive, scorers=scorers),
            intent_input_for_example(poisoned_metadata, scorers=scorers),
        )
        extractor = runtime_feature_extractor(
            DEFAULT_FNV_SEED,
            scorers=scorers,
        )
        features_positive = extractor(positive, INTENT_TEST_DIMENSION)
        features_inverted = extractor(poisoned_metadata, INTENT_TEST_DIMENSION)
        self.assertEqual(features_positive, features_inverted)
        self.assertTrue(features_positive.character_fingerprints)

        class RaisingScorer:
            def score(self, _word: str) -> WordScore:
                raise AssertionError("classifier extraction must not call scorers")

        scorer_free = runtime_feature_extractor(
            DEFAULT_FNV_SEED,
            scorers={0: RaisingScorer(), 1: RaisingScorer()},
        )
        self.assertEqual(
            scorer_free(positive, INTENT_TEST_DIMENSION),
            features_positive,
        )

    def test_train_only_scorer_is_quarantined_and_non_train_invariant(self) -> None:
        pair = LayoutPair()
        train_en_excluded = alphabetic_signature_for_split(
            "train", minimum_length=INTENT_LEAKAGE_EXCLUDED_WORD_LENGTH
        )
        train_en_included = alphabetic_signature_for_split(
            "train", minimum_length=INTENT_LEAKAGE_INCLUDED_WORD_LENGTH
        )
        train_ru_signature = alphabetic_signature_for_split(
            "train", minimum_length=INTENT_LEAKAGE_RUSSIAN_WORD_SIGNATURE_LENGTH
        )
        train_ru = pair.translate(train_ru_signature, "us", "ru")
        non_train_en = alphabetic_signature_for_split(
            "development", minimum_length=INTENT_LEAKAGE_EXCLUDED_WORD_LENGTH
        )
        non_train_ru_signature = alphabetic_signature_for_split(
            "calibration", minimum_length=INTENT_LEAKAGE_EXCLUDED_WORD_LENGTH
        )
        non_train_ru = pair.translate(non_train_ru_signature, "us", "ru")
        records = (
            LexiconWord(
                train_en_excluded,
                0,
                INTENT_LEAKAGE_EXCLUDED_ENTRY_FREQUENCY,
                train_en_excluded,
                "train",
            ),
            LexiconWord(
                train_en_included,
                0,
                INTENT_LEAKAGE_INCLUDED_ENTRY_FREQUENCY,
                train_en_included,
                "train",
            ),
            LexiconWord(
                train_ru,
                1,
                INTENT_LEAKAGE_EXCLUDED_ENTRY_FREQUENCY,
                train_ru_signature,
                "train",
            ),
            LexiconWord(
                non_train_en,
                0,
                INTENT_LEAKAGE_NON_TRAIN_ENTRY_FREQUENCY,
                non_train_en,
                "development",
            ),
            LexiconWord(
                non_train_ru,
                1,
                INTENT_LEAKAGE_NON_TRAIN_ENTRY_FREQUENCY,
                non_train_ru_signature,
                "calibration",
            ),
        )
        prepared = prepare_lexicon(records)
        occurrence = QuarantinedVariantOccurrence(
            train_en_excluded,
            train_en_excluded,
            "train",
            0,
            "identity",
            "cross_split",
        )
        quarantine = VariantQuarantine(
            (occurrence,),
            variant_quarantine_fingerprint((occurrence,)),
        )
        first = TrainOnlyLanguageScorers.from_training_partition(
            prepared,
            quarantine,
        )
        mutated_non_train = prepare_lexicon(
            (
                *records[: INTENT_LEAKAGE_TRAIN_RECORD_COUNT],
                replace(
                    records[INTENT_LEAKAGE_NON_TRAIN_EN_RECORD_INDEX],
                    frequency=INTENT_LEAKAGE_MUTATED_NON_TRAIN_EN_FREQUENCY,
                ),
                replace(
                    records[INTENT_LEAKAGE_NON_TRAIN_RU_RECORD_INDEX],
                    frequency=INTENT_LEAKAGE_MUTATED_NON_TRAIN_RU_FREQUENCY,
                ),
            )
        )
        second = TrainOnlyLanguageScorers.from_training_partition(
            mutated_non_train,
            quarantine,
        )
        self.assertEqual(first.source_sha256, second.source_sha256)
        self.assertEqual(first.provenance_payload(), second.provenance_payload())
        self.assertEqual(first.word_counts_by_group, (1, 1))
        self.assertEqual(first.excluded_quarantined_identities, 1)
        self.assertFalse(first.scorers[0].score(train_en_excluded).exact)
        window_score = first.scorers[0].score("window")
        self.assertFalse(window_score.exact)
        self.assertFalse(window_score.known)
        self.assertEqual(window_score.frequency, 0)
        self.assertFalse(first.scorers[0].score(non_train_en).exact)
        self.assertEqual(
            first.scorers[0].score(non_train_en),
            second.scorers[0].score(non_train_en),
        )

    def test_guarded_safety_audit_uses_real_production_guards(self) -> None:
        rows = tuple(
            LexicalExample(
                original="--force-with-lease",
                alternative="--ащксуюцшер-дфыув",
                source_group=0,
                target_group=1,
                trigger=trigger,
                label=False,
                weight=1.0,
                base_signature="hard:--force-with-lease",
                variant_kind="protected",
                source_known=False,
                target_known=False,
                protected=True,
                safety=True,
            )
            for trigger in TRIGGERS
        ) + tuple(
            LexicalExample(
                original="test",
                alternative="еуые",
                source_group=0,
                target_group=1,
                trigger=trigger,
                label=False,
                weight=1.0,
                base_signature="test",
                variant_kind="lexical_collision",
                source_known=True,
                target_known=True,
                safety=True,
            )
            for trigger in TRIGGERS
        )
        audit = audit_guarded_safety_corpus(rows)
        self.assertTrue(audit.passes(0))
        broken = audit_guarded_safety_corpus(
            (replace(rows[0], original="ordinary"), *rows[1:])
        )
        self.assertFalse(broken.passes(0))
        self.assertTrue(broken.failures)

    def test_sparse_normalization_and_runtime_input(self) -> None:
        normalized = normalize_sparse_features(
            INTENT_LEAKAGE_UNNORMALIZED_SPARSE_FEATURES, INTENT_LEAKAGE_SPARSE_FEATURE_TEST_DIMENSION
        )
        self.assertEqual(normalized, INTENT_LEAKAGE_NORMALIZED_SPARSE_FEATURES)
        with self.assertRaisesRegex(ValueError, "outside"):
            normalize_sparse_features(
                INTENT_LEAKAGE_OUT_OF_RANGE_SPARSE_FEATURES, INTENT_LEAKAGE_SPARSE_FEATURE_TEST_DIMENSION
            )
        with self.assertRaisesRegex(ValueError, "finite"):
            normalize_sparse_features(
                INTENT_LEAKAGE_NON_FINITE_SPARSE_FEATURES, INTENT_LEAKAGE_SPARSE_FEATURE_TEST_DIMENSION
            )
        evidence = intent_input_for_example(
            lexical_example(True),
            scorers=test_word_scorers(),
        )
        self.assertEqual(evidence.context_delta, 0.0)
        self.assertEqual(evidence.trigger, "space")

    def test_training_support_union_is_collected_without_per_row_sets(self) -> None:
        examples = (lexical_example(True), lexical_example(False, signature="world"))
        extractor = runtime_feature_extractor(
            DEFAULT_FNV_SEED,
            DEFAULT_MEMBERSHIP_FNV_SEED,
            scorers=test_word_scorers(),
        )
        expected: set[int] = set()
        for example in examples:
            expected.update(
                extractor(example, INTENT_TEST_DIMENSION).character_fingerprints
            )
        supported: set[int] = set()
        featured = featurize_examples(
            examples,
            INTENT_TEST_DIMENSION,
            extractor,
            supported_fingerprints=supported,
        )
        self.assertEqual(supported, expected)
        self.assertTrue(featured)
        self.assertTrue(all(not hasattr(row, "character_fingerprints") for row in featured))

    def test_process_featurization_is_ordered_and_bit_exact(self) -> None:
        examples = tuple(
            lexical_example(
                index % INTENT_LABEL_PARITY_MODULUS == 0,
                trigger=TRIGGERS[index % len(TRIGGERS)],
                signature=f"parallel-feature-{index}",
            )
            for index in range(INTENT_LEAKAGE_PARALLEL_FEATURIZATION_EXAMPLE_COUNT)
        )
        extractor = runtime_feature_extractor(
            DEFAULT_FNV_SEED,
            DEFAULT_MEMBERSHIP_FNV_SEED,
            scorers=test_word_scorers(),
        )
        sequential_support: set[int] = set()
        parallel_support: set[int] = set()
        sequential = featurize_examples(
            examples,
            INTENT_TEST_DIMENSION,
            extractor,
            supported_fingerprints=sequential_support,
        )
        parallel = tim.featurize_examples_parallel(
            examples,
            INTENT_TEST_DIMENSION,
            extractor,
            workers=INTENT_LEAKAGE_PARALLEL_FEATURIZATION_WORKER_COUNT,
            supported_fingerprints=parallel_support,
        )

        self.assertEqual(parallel, sequential)
        self.assertTrue(
            all(
                featured.example is example
                for featured, example in zip(parallel, examples, strict=True)
            )
        )
        self.assertEqual(parallel_support, sequential_support)
        self.assertEqual(tim.resolve_training_workers(1), 1)
        with patch(
            "train_intent_model.os.sched_getaffinity",
            return_value=FAKE_AFFINITY_CORE_IDS,
        ):
            self.assertEqual(tim.available_training_workers(), INTENT_EXPECTED_AFFINITY_WORKER_COUNT)
            self.assertEqual(tim.resolve_training_workers(0), INTENT_EXPECTED_AFFINITY_WORKER_COUNT)
            self.assertEqual(
                tim.resolve_training_workers(INTENT_EXCESSIVE_REQUESTED_WORKERS),
                INTENT_EXPECTED_AFFINITY_WORKER_COUNT,
            )
        with (
            patch(
                "train_intent_model.os.sched_getaffinity",
                side_effect=OSError("unsupported"),
            ),
            patch("train_intent_model.os.cpu_count", return_value=FAKE_CPU_COUNT),
        ):
            self.assertEqual(tim.available_training_workers(), FAKE_CPU_COUNT)
        with self.assertRaisesRegex(ValueError, "at least one worker"):
            tim.featurize_examples_parallel(
                examples,
                INTENT_TEST_DIMENSION,
                extractor,
                workers=0,
            )
        with self.assertRaisesRegex(ValueError, "cannot be negative"):
            tim.resolve_training_workers(-1)


class OptimizerAndCalibrationTests(unittest.TestCase):
    PASSING_EPOCH_EVALUATION = DevelopmentEpochEvaluation(
        log_loss=INTENT_PASSING_EPOCH_LOG_LOSS,
        operating_point=ThresholdSelection(
            "space",
            1.0,
            ConfusionMatrix(*INTENT_PASSING_EPOCH_CONFUSION_COUNTS),
            ConfusionMatrix(*INTENT_PASSING_EPOCH_TYPO_CONFUSION_COUNTS),
        ),
        false_positive_rate_upper_familywise_95=INTENT_EPOCH_FALSE_POSITIVE_RATE_UPPER_BOUND,
        typo_false_positive_rate_upper_familywise_95=INTENT_EPOCH_FALSE_POSITIVE_RATE_UPPER_BOUND,
        policy_checks_passed=INTENT_PASSING_EPOCH_POLICY_CHECKS_PASSED,
        policy_passed=True,
    )
    LOWER_LOSS_UNCERTIFIED_EPOCH_EVALUATION = DevelopmentEpochEvaluation(
        log_loss=INTENT_UNCERTIFIED_EPOCH_LOG_LOSS,
        operating_point=ThresholdSelection(
            "space",
            INTENT_UNCERTIFIED_EPOCH_THRESHOLD_LOGIT,
            ConfusionMatrix(*INTENT_UNCERTIFIED_EPOCH_CONFUSION_COUNTS),
            ConfusionMatrix(*INTENT_UNCERTIFIED_EPOCH_TYPO_CONFUSION_COUNTS),
        ),
        false_positive_rate_upper_familywise_95=INTENT_EPOCH_FALSE_POSITIVE_RATE_UPPER_BOUND,
        typo_false_positive_rate_upper_familywise_95=INTENT_EPOCH_FALSE_POSITIVE_RATE_UPPER_BOUND,
        policy_checks_passed=INTENT_UNCERTIFIED_EPOCH_POLICY_CHECKS_PASSED,
        policy_passed=False,
    )
    FTRL_CACHED_WEIGHTS_UPDATES = (
        (
            normalize_sparse_features(
                FTRL_CACHED_WEIGHTS_FIRST_RAW_FEATURES,
                FTRL_CACHED_WEIGHTS_DIMENSION,
            ),
            True,
            FTRL_CACHED_WEIGHTS_FIRST_SAMPLE_WEIGHT,
        ),
        *FTRL_CACHED_WEIGHTS_CANONICAL_UPDATES,
    )
    EXPECTED_DIRECTIONAL_METRICS = ConfusionMatrix(*EXPECTED_INTENT_DIRECTIONAL_BUDGET_CONFUSION_COUNTS)
    EXPECTED_ZERO_FALSE_POSITIVE_METRICS = ConfusionMatrix(*EXPECTED_INTENT_ZERO_FALSE_POSITIVE_CONFUSION_COUNTS)
    EXPECTED_AGGREGATE_BUDGET_METRICS = ConfusionMatrix(*EXPECTED_INTENT_AGGREGATE_BUDGET_CONFUSION_COUNTS)
    OVERALL_STRICT_METRICS = ConfusionMatrix(*INTENT_STRICT_OVERALL_CONFUSION_COUNTS)
    TYPO_STRICT_METRICS = ConfusionMatrix(*INTENT_STRICT_TYPO_CONFUSION_COUNTS)
    WILSON_BOUND_METRICS = ConfusionMatrix(*INTENT_WILSON_BOUND_CONFUSION_COUNTS)
    EXPECTED_TYPO_POLICY_METRICS = ConfusionMatrix(*EXPECTED_INTENT_TYPO_POLICY_CONFUSION_COUNTS)
    EXPECTED_TYPO_POLICY_TYPO_METRICS = ConfusionMatrix(*EXPECTED_INTENT_TYPO_POLICY_TYPO_CONFUSION_COUNTS)
    TAIL_DIAGNOSTIC_OVERALL_METRICS = ConfusionMatrix(*INTENT_TAIL_DIAGNOSTIC_OVERALL_CONFUSION_COUNTS)
    INITIAL_SYMMETRIC_SELECTION = ThresholdSelection(
        "space",
        1.0,
        ConfusionMatrix(*INTENT_INITIAL_SYMMETRIC_CONFUSION_COUNTS),
        ConfusionMatrix(*INTENT_INITIAL_SYMMETRIC_CONFUSION_COUNTS),
        {"0>1": 1.0, "1>0": 1.0},
    )
    STRONG_SYMMETRIC_METRICS = ConfusionMatrix(INTENT_STRONG_SAMPLE_SIZE, 0, INTENT_STRONG_SAMPLE_SIZE, 0)
    EMPTY_TRIGGER_METRICS = ConfusionMatrix(0, 0, INTENT_STRONG_SAMPLE_SIZE, 0)
    WEAK_SAMPLE_METRICS = ConfusionMatrix(INTENT_WEAK_SAMPLE_SIZE, 0, INTENT_WEAK_SAMPLE_SIZE, 0)
    @staticmethod
    def _reference_ftrl_update(
        model: FTRLProximal,
        features: tuple[tuple[int, float], ...],
        label: bool,
        sample_weight: float,
    ) -> float:
        """Original two-weight-lookup update used as a bit-exact oracle."""

        prediction = stable_sigmoid(model.score(features))
        residual = (prediction - float(label)) * sample_weight
        old_bias_n = model.bias_n
        old_bias = model.bias
        new_bias_n = old_bias_n + residual * residual
        bias_sigma = (
            math.sqrt(new_bias_n) - math.sqrt(old_bias_n)
        ) / model.parameters.alpha
        model.bias_z += residual - bias_sigma * old_bias
        model.bias_n = new_bias_n
        for index, value in features:
            gradient = residual * value
            old_n = model.n.get(index, 0.0)
            weight = model.weight(index)
            new_n = old_n + gradient * gradient
            sigma = (
                math.sqrt(new_n) - math.sqrt(old_n)
            ) / model.parameters.alpha
            model.z[index] = (
                model.z.get(index, 0.0) + gradient - sigma * weight
            )
            model.n[index] = new_n
        return prediction

    def test_context_stress_scoring_covers_every_profile_and_trigger(
        self,
    ) -> None:
        examples = tuple(
            lexical_example(
                label,
                trigger=trigger,
                variant=variant,
                signature=f"{trigger}-{label}-{variant}",
            )
            for trigger in TRIGGERS
            for label in (False, True)
            for variant in ("identity", "deletion")
        )
        observed_contexts: list[tuple[float, int | None]] = []

        class RecordingExtractor:
            def __call__(
                self, example: LexicalExample, dimension: int
            ) -> ExtractedExampleFeatures:
                del dimension
                observed_contexts.append(
                    (example.context_delta, example.context_group)
                )
                return ExtractedExampleFeatures((), frozenset())

        class ConstantScorer:
            def score(
                self, features: tuple[tuple[int, float], ...]
            ) -> float:
                del features
                raise AssertionError("neutral scores must be reused")

        predicted_positive = ConfusionMatrix(
            INTENT_PERFECT_CONFUSION_COUNT, 0, 0, INTENT_PERFECT_CONFUSION_COUNT
        )
        predicted_positive_typo = ConfusionMatrix(1, 0, 0, 1)
        thresholds = {
            trigger: ThresholdSelection(
                trigger,
                0.0,
                predicted_positive,
                predicted_positive,
            )
            for trigger in TRIGGERS
        }
        overall, typos = score_context_stress_profiles(
            examples,
            dimension=INTENT_TEST_DIMENSION,
            extractor=RecordingExtractor(),
            model=ConstantScorer(),
            calibration=directional_calibration(
                samples_per_direction=len(examples) // INTENT_OPTIMIZER_EXAMPLES_PER_DIRECTION_DIVISOR,
                positives_per_direction=INTENT_OPTIMIZER_POSITIVES_PER_DIRECTION_OVERRIDE,
            ),
            thresholds=thresholds,
            neutral_scores=tuple(
                ScoredExample(example, 1.0, 1.0) for example in examples
            ),
        )
        self.assertEqual(
            set(overall),
            {profile.name for profile in CONTEXT_STRESS_PROFILES},
        )
        self.assertNotEqual(overall, typos)
        self.assertTrue(
            all(
                metrics == predicted_positive
                for profile_metrics in overall.values()
                for metrics in profile_metrics.values()
            )
        )
        self.assertTrue(
            all(
                metrics == predicted_positive_typo
                for profile_metrics in typos.values()
                for metrics in profile_metrics.values()
            )
        )
        self.assertEqual(
            len(observed_contexts),
            INTENT_OPTIMIZER_LABEL_VARIANT_COMBINATION_COUNT
            * len(TRIGGERS)
            * INTENT_OPTIMIZER_CONTEXT_STRESS_SCORE_COUNT_PER_EXAMPLE,
        )
        self.assertIn((0.0, None), observed_contexts)

    def test_context_invariance_proof_rejects_sensitive_extractors(self) -> None:
        class ContextSensitiveExtractor:
            def __call__(
                self, example: LexicalExample, dimension: int
            ) -> ExtractedExampleFeatures:
                del dimension
                return ExtractedExampleFeatures(
                    ((1, example.context_delta),), frozenset()
                )

        with self.assertRaisesRegex(RuntimeError, "context-sensitive"):
            verify_context_feature_invariance(
                dimension=INTENT_TEST_DIMENSION,
                extractor=ContextSensitiveExtractor(),
            )

    def _featured(self) -> tuple[tuple[FeaturedExample, ...], tuple[FeaturedExample, ...]]:
        rows: list[FeaturedExample] = []
        for index in range(INTENT_OPTIMIZER_FEATURED_EXAMPLE_COUNT):
            label = index % INTENT_LABEL_PARITY_MODULUS == 0
            direction: LayoutDirection = (
                "0>1" if index % INTENT_OPTIMIZER_DIRECTION_GROUP_MODULUS < INTENT_OPTIMIZER_DIRECTION_SWITCH_THRESHOLD else "1>0"
            )
            example = lexical_example(
                label,
                signature=f"s{index}",
                direction=direction,
            )
            features = (
                (1, 1.0 if label else -1.0),
                (INTENT_OPTIMIZER_SECOND_FEATURE_INDEX, INTENT_OPTIMIZER_SECOND_FEATURE_VALUE),
            )
            rows.append(FeaturedExample(example, features))
        return (
            tuple(rows[: INTENT_OPTIMIZER_TRAINING_ROW_COUNT]),
            tuple(rows[INTENT_OPTIMIZER_TRAINING_ROW_COUNT :]),
        )

    def test_ftrl_is_deterministic_sparse_and_separates(self) -> None:
        training, development = self._featured()
        progress: list[tim.EpochReport] = []
        first = fit_ftrl(
            training,
            development,
            config(),
            progress=progress.append,
        )
        second = fit_ftrl(training, development, config())
        self.assertEqual(tuple(progress), first.history)
        self.assertEqual(first.best_epoch, second.best_epoch)
        self.assertEqual(first.history, second.history)
        self.assertEqual(first.model.z, second.model.z)
        self.assertEqual(first.model.n, second.model.n)
        self.assertEqual(first.model.bias, second.model.bias)
        self.assertGreater(first.model.score(((1, 1.0),)), 0.0)
        self.assertLess(first.model.score(((1, -1.0),)), 0.0)

        sparse = FTRLProximal(
            FTRLParameters(FTRL_SMALL_TEST_DIMENSION, FTRL_TEST_ALPHA, 1.0, FTRL_SPARSE_TEST_L1, 0.0)
        )
        sparse.update(((1, 1.0),), True)
        self.assertEqual(sparse.sparse_weights(), {})
        with self.assertRaisesRegex(ValueError, "sample weight"):
            sparse.update(((1, 1.0),), True, 0.0)

    def test_parallel_development_scoring_is_bit_exact(self) -> None:
        training, development = self._featured()
        parameters = FTRLParameters(INTENT_TEST_DIMENSION, FTRL_TEST_ALPHA, 1.0, 0.0, FTRL_TEST_L2)
        model = FTRLProximal(parameters)
        for item in training:
            model.update(
                item.features,
                item.example.label,
                item.example.weight,
            )

        expected = evaluate_development_epoch(
            model,
            development,
            config(dimension=INTENT_TEST_DIMENSION),
        )
        actual = tim.evaluate_development_epoch_parallel(
            model,
            development,
            config(dimension=INTENT_TEST_DIMENSION),
            workers=INTENT_OPTIMIZER_PARALLEL_WORKER_COUNT,
        )

        self.assertEqual(actual, expected)
        calibration = directional_calibration(
            scale=INTENT_OPTIMIZER_CALIBRATION_SCALE, bias=-INTENT_OPTIMIZER_CALIBRATION_BIAS_MAGNITUDE
        )
        self.assertEqual(
            tim.score_examples_parallel(
                model,
                calibration,
                development,
                workers=INTENT_OPTIMIZER_PARALLEL_WORKER_COUNT,
            ),
            tim.score_examples(model, calibration, development),
        )
        with self.assertRaisesRegex(ValueError, "do not match"):
            tim.calibrate_raw_logits(development, (), calibration)
        with self.assertRaisesRegex(ValueError, "at least one worker"):
            tim.evaluate_development_epoch_parallel(
                model,
                development,
                config(dimension=INTENT_TEST_DIMENSION),
                workers=0,
            )
        with self.assertRaisesRegex(ValueError, "at least one worker"):
            fit_ftrl(
                training,
                development,
                config(dimension=INTENT_TEST_DIMENSION),
                evaluation_workers=0,
            )

    def test_epoch_selection_prefers_certified_tail_over_lower_log_loss(
        self,
    ) -> None:
        training, development = self._featured()
        passing = self.PASSING_EPOCH_EVALUATION
        lower_loss_but_uncertified = self.LOWER_LOSS_UNCERTIFIED_EPOCH_EVALUATION
        with patch.object(
            tim,
            "evaluate_development_epoch",
            side_effect=(passing, lower_loss_but_uncertified),
        ):
            result = fit_ftrl(
                training,
                development,
                config(
                    maximum_epochs=INTENT_OPTIMIZER_MAXIMUM_EPOCHS,
                    minimum_epochs=1,
                    patience=INTENT_OPTIMIZER_PATIENCE,
                ),
            )

        self.assertEqual(result.best_epoch, 1)
        self.assertEqual(
            [item.development_log_loss for item in result.history],
            [
                self.PASSING_EPOCH_EVALUATION.log_loss,
                self.LOWER_LOSS_UNCERTIFIED_EPOCH_EVALUATION.log_loss,
            ],
        )
        self.assertTrue(
            all(
                item.development_false_positive == 0
                and item.development_typo_false_positive == 0
                for item in result.history
            )
        )

    def test_development_epoch_rejects_empty_or_zero_weight_data(self) -> None:
        parameters = FTRLParameters(FTRL_SMALL_TEST_DIMENSION, FTRL_TEST_ALPHA, 1.0, 0.0, 0.0)
        model = FTRLProximal(parameters)
        with self.assertRaisesRegex(ValueError, "empty development"):
            evaluate_development_epoch(
                model, (), config(dimension=FTRL_SMALL_TEST_DIMENSION)
            )

        _training, development = self._featured()
        zero_weight = tuple(
            replace(
                item,
                example=replace(item.example, weight=0.0),
            )
            for item in development
        )
        with self.assertRaisesRegex(ValueError, "positive sum"):
            evaluate_development_epoch(
                model,
                zero_weight,
                config(dimension=FTRL_SMALL_TEST_DIMENSION),
            )

    def test_ftrl_cached_weights_are_bit_exact_against_reference(self) -> None:
        parameters = FTRLParameters(
            FTRL_CACHED_WEIGHTS_DIMENSION,
            FTRL_CACHED_WEIGHTS_ALPHA,
            1.0,
            FTRL_CACHED_WEIGHTS_L1,
            FTRL_CACHED_WEIGHTS_L2,
        )
        optimized = FTRLProximal(parameters)
        reference = FTRLProximal(parameters)
        updates = self.FTRL_CACHED_WEIGHTS_UPDATES
        for _epoch in range(FTRL_CACHED_WEIGHTS_EPOCHS):
            for features, label, sample_weight in updates:
                expected = self._reference_ftrl_update(
                    reference, features, label, sample_weight
                )
                actual = optimized.update(
                    features, label, sample_weight
                )
                self.assertEqual(actual.hex(), expected.hex())
                self.assertEqual(optimized.z, reference.z)
                self.assertEqual(optimized.n, reference.n)
                self.assertEqual(optimized.bias_z.hex(), reference.bias_z.hex())
                self.assertEqual(optimized.bias_n.hex(), reference.bias_n.hex())

        self.assertEqual(
            optimized.nonzero_weight_count(),
            len(optimized.sparse_weights()),
        )

    def test_ftrl_rejects_noncanonical_sparse_vectors(self) -> None:
        model = FTRLProximal(
            FTRLParameters(FTRL_SMALL_TEST_DIMENSION, FTRL_TEST_ALPHA, 1.0, 0.0, 0.0)
        )
        for features, message in INTENT_NONCANONICAL_FEATURE_CASES:
            with self.subTest(features=features):
                with self.assertRaisesRegex(ValueError, message):
                    model.update(features, True)

    def test_platt_thresholds_veto_and_numeric_edges(self) -> None:
        samples = INTENT_OPTIMIZER_PLATT_THRESHOLD_SAMPLES
        calibration = fit_platt_calibration(
            samples, l2=INTENT_PLATT_TEST_L2, maximum_iterations=INTENT_PLATT_TEST_MAX_ITERATIONS
        )
        self.assertGreater(calibration.slope, 0.0)
        self.assertLess(
            calibration.confidence(-INTENT_OPTIMIZER_EXTREME_SAMPLE_LOGIT),
            calibration.confidence(INTENT_OPTIMIZER_EXTREME_SAMPLE_LOGIT),
        )
        for logit in INTENT_OPTIMIZER_SIGMOID_EDGE_LOGITS:
            with self.subTest(logit=logit):
                self.assertEqual(
                    stable_sigmoid(logit).hex(),
                    runtime_stable_sigmoid(logit).hex(),
                )
        for sigmoid in (stable_sigmoid, runtime_stable_sigmoid):
            with self.assertRaisesRegex(ValueError, "NaN"):
                sigmoid(math.nan)
        with self.assertRaisesRegex(ValueError, "both labels"):
            fit_platt_calibration(
                ((1.0, True),),
                l2=INTENT_PLATT_TEST_L2,
                maximum_iterations=INTENT_OPTIMIZER_SINGLE_EXAMPLE_MAX_ITERATIONS,
            )

        directional_samples: tuple[
            tuple[float, bool, LayoutDirection], ...
        ] = INTENT_OPTIMIZER_DIRECTIONAL_PLATT_SAMPLES
        directional = fit_directional_platt_calibration(
            directional_samples,
            l2=INTENT_PLATT_TEST_L2,
            maximum_iterations=INTENT_PLATT_TEST_MAX_ITERATIONS,
        )
        self.assertEqual(directional.sample_count, len(INTENT_OPTIMIZER_DIRECTIONAL_PLATT_SAMPLES))
        self.assertEqual(directional.positive_count, INTENT_OPTIMIZER_EXPECTED_DIRECTIONAL_POSITIVE_COUNT)
        self.assertGreater(
            directional.for_direction("1>0").intercept,
            directional.for_direction("0>1").intercept,
        )
        self.assertGreater(
            directional.transform_logit(-INTENT_OPTIMIZER_PROBE_LOGIT, 1, 0),
            directional.transform_logit(-INTENT_OPTIMIZER_PROBE_LOGIT, 0, 1),
        )
        self.assertEqual(
            set(directional.runtime_parameters()),
            {"0>1", "1>0"},
        )
        self.assertEqual(
            directional.payload()["method"],
            "independent-platt-by-layout-direction",
        )
        with self.assertRaisesRegex(ValueError, "both labels"):
            fit_directional_platt_calibration(
                tuple(
                    item
                    for item in directional_samples
                    if item[INTENT_OPTIMIZER_DIRECTIONAL_SAMPLE_DIRECTION_INDEX] == "0>1" or item[1]
                ),
                l2=INTENT_PLATT_TEST_L2,
                maximum_iterations=INTENT_OPTIMIZER_SINGLE_EXAMPLE_MAX_ITERATIONS,
            )

        scored: list[ScoredExample] = []
        for trigger in TRIGGERS:
            scored.extend(
                ScoredExample(
                    lexical_example(
                        False,
                        trigger=trigger,
                        signature=f"n{index}",
                        direction="0>1" if index % INTENT_LABEL_PARITY_MODULUS == 0 else "1>0",
                    ),
                    -INTENT_OPTIMIZER_HIGH_SCORE - index / INTENT_OPTIMIZER_NEGATIVE_LOGIT_STEP_DIVISOR,
                    -INTENT_OPTIMIZER_HIGH_SCORE - index / INTENT_OPTIMIZER_NEGATIVE_LOGIT_STEP_DIVISOR,
                )
                for index in range(INTENT_OPTIMIZER_NEGATIVE_FIXTURE_COUNT)
            )
            scored.extend(
                (
                    ScoredExample(
                        lexical_example(True, trigger=trigger, direction="0>1"),
                        1.0,
                        1.0,
                    ),
                    ScoredExample(
                        lexical_example(True, trigger=trigger, direction="1>0"),
                        INTENT_OPTIMIZER_HIGH_SCORE,
                        INTENT_OPTIMIZER_HIGH_SCORE,
                    ),
                )
            )
        selected = choose_trigger_thresholds(
            tuple(scored),
            config(
                threshold_max_false_positive_rate=1.0,
                pause_threshold_max_false_positive_rate=1.0,
            ),
        )
        self.assertEqual(set(selected), set(TRIGGERS))
        self.assertGreaterEqual(
            selected["pause"].logit,
            max(item.logit for key, item in selected.items() if key != "pause")
            + INTENT_OPTIMIZER_PAUSE_LOGIT_MARGIN_PROBE,
        )
        direct = choose_threshold(
            tuple(scored),
            "space",
            precision_floor=1.0,
            maximum_false_positive_rate=1.0,
        )
        self.assertEqual(direct.metrics.false_positive, 0)
        statistically_weak = tuple(
            ScoredExample(
                lexical_example(False, signature=f"weak{index}"),
                -1.0,
                -1.0,
            )
            for index in range(INTENT_WEAK_SAMPLE_SIZE)
        ) + (ScoredExample(lexical_example(True), 1.0, 1.0),)
        with self.assertRaisesRegex(RuntimeError, "statistically certified"):
            choose_threshold(
                statistically_weak,
                "space",
                precision_floor=1.0,
                maximum_false_positive_rate=INTENT_STRICT_FALSE_POSITIVE_RATE,
            )
        veto = choose_veto_threshold(
            tuple(scored), quantile=0.0, margin=INTENT_OPTIMIZER_VETO_MARGIN_PROBE
        )
        self.assertLess(veto.raw_logit, 1.0)
        self.assertEqual(veto.vetoed_positive_samples, 0)
        self.assertEqual(veto_metrics(tuple(scored), veto.raw_logit), veto)

    def test_directional_threshold_jointly_allocates_false_positive_budget(
        self,
    ) -> None:
        rows: list[ScoredExample] = []
        for direction, positive_scores, leading_negative in INTENT_DIRECTIONAL_BUDGET_ROWS:
            typed_direction = cast(LayoutDirection, direction)
            for index, score in enumerate(positive_scores):
                example = lexical_example(
                    True,
                    variant="deletion",
                    signature=f"{direction}-p{index}",
                    direction=typed_direction,
                )
                rows.append(ScoredExample(example, score, score))
            for index in range(INTENT_OPTIMIZER_NEGATIVE_TAIL_COUNT):
                score = (
                    leading_negative if index == 0 else -INTENT_OPTIMIZER_NEGATIVE_TAIL_BASE - index
                )
                example = lexical_example(
                    False,
                    variant="deletion",
                    signature=f"{direction}-n{index}",
                    direction=typed_direction,
                )
                rows.append(ScoredExample(example, score, score))

        scalar = choose_threshold(
            tuple(rows),
            "space",
            precision_floor=INTENT_OPTIMIZER_STANDARD_TEST_PRECISION_FLOOR,
            maximum_false_positive_rate=1.0,
            minimum_recall=INTENT_OPTIMIZER_STANDARD_TEST_PRECISION_FLOOR,
            minimum_specificity=0.0,
            typo_precision_floor=INTENT_OPTIMIZER_STANDARD_TEST_PRECISION_FLOOR,
            minimum_typo_recall=INTENT_OPTIMIZER_STANDARD_TEST_PRECISION_FLOOR,
            typo_minimum_specificity=0.0,
            typo_maximum_false_positive_rate=1.0,
        )
        directional = choose_directional_threshold(
            tuple(rows),
            "space",
            precision_floor=INTENT_OPTIMIZER_STANDARD_TEST_PRECISION_FLOOR,
            maximum_false_positive_rate=1.0,
            minimum_recall=INTENT_OPTIMIZER_STANDARD_TEST_PRECISION_FLOOR,
            minimum_specificity=0.0,
            typo_precision_floor=INTENT_OPTIMIZER_STANDARD_TEST_PRECISION_FLOOR,
            minimum_typo_recall=INTENT_OPTIMIZER_STANDARD_TEST_PRECISION_FLOOR,
            typo_minimum_specificity=0.0,
            typo_maximum_false_positive_rate=1.0,
        )
        self.assertEqual(scalar.metrics.true_positive, 0)
        self.assertEqual(directional.metrics, self.EXPECTED_DIRECTIONAL_METRICS)
        self.assertEqual(
            directional.typo_metrics,
            directional.metrics,
        )
        self.assertGreater(
            directional.logit_for("0>1"),
            directional.logit_for("1>0"),
        )
        self.assertEqual(
            directional.runtime_logits(),
            {"0>1": INTENT_OPTIMIZER_EXPECTED_DIRECTIONAL_RUNTIME_LOGIT, "1>0": 0.0},
        )
        zero_false_positive = choose_directional_threshold(
            tuple(rows),
            "space",
            precision_floor=INTENT_OPTIMIZER_STANDARD_TEST_PRECISION_FLOOR,
            maximum_false_positive_rate=1.0,
            minimum_recall=INTENT_OPTIMIZER_STANDARD_TEST_PRECISION_FLOOR,
            minimum_specificity=0.0,
            typo_precision_floor=INTENT_OPTIMIZER_STANDARD_TEST_PRECISION_FLOOR,
            minimum_typo_recall=INTENT_OPTIMIZER_STANDARD_TEST_PRECISION_FLOOR,
            typo_minimum_specificity=0.0,
            typo_maximum_false_positive_rate=1.0,
            maximum_false_positives=0,
        )
        self.assertEqual(
            zero_false_positive.metrics,
            self.EXPECTED_ZERO_FALSE_POSITIVE_METRICS,
        )
        self.assertEqual(
            zero_false_positive.typo_metrics,
            zero_false_positive.metrics,
        )
        with self.assertRaisesRegex(ValueError, "non-negative integer"):
            choose_directional_threshold(
                tuple(rows),
                "space",
                precision_floor=INTENT_OPTIMIZER_STANDARD_TEST_PRECISION_FLOOR,
                maximum_false_positive_rate=1.0,
                minimum_recall=INTENT_OPTIMIZER_STANDARD_TEST_PRECISION_FLOOR,
                minimum_specificity=0.0,
                typo_precision_floor=INTENT_OPTIMIZER_STANDARD_TEST_PRECISION_FLOOR,
                minimum_typo_recall=INTENT_OPTIMIZER_STANDARD_TEST_PRECISION_FLOOR,
                typo_minimum_specificity=0.0,
                typo_maximum_false_positive_rate=1.0,
                maximum_false_positives=-1,
            )

    def test_directional_threshold_enforces_aggregate_false_positive_budget(
        self,
    ) -> None:
        rows: list[ScoredExample] = []
        for direction in ("0>1", "1>0"):
            for index, score in enumerate(INTENT_OPTIMIZER_AGGREGATE_POSITIVE_SCORES):
                rows.append(
                    ScoredExample(
                        lexical_example(
                            True,
                            variant="deletion",
                            signature=f"{direction}-aggregate-p{index}",
                            direction=direction,
                        ),
                        score,
                        score,
                    )
                )
            for index, score in enumerate(INTENT_OPTIMIZER_AGGREGATE_NEGATIVE_SCORES):
                rows.append(
                    ScoredExample(
                        lexical_example(
                            False,
                            variant="deletion",
                            signature=f"{direction}-aggregate-n{index}",
                            direction=direction,
                        ),
                        score,
                        score,
                    )
                )

        selected = choose_directional_threshold(
            tuple(rows),
            "space",
            precision_floor=0.0,
            maximum_false_positive_rate=1.0,
            typo_precision_floor=0.0,
            typo_maximum_false_positive_rate=1.0,
            maximum_false_positives=1,
        )

        self.assertEqual(selected.metrics, self.EXPECTED_AGGREGATE_BUDGET_METRICS)
        self.assertEqual(selected.typo_metrics, selected.metrics)

    def test_selection_uses_stricter_typo_precision_than_sealed_test(self) -> None:
        training_config = config(**INTENT_STRICTER_TYPO_CONFIG_OVERRIDES)
        overall = self.OVERALL_STRICT_METRICS
        typos = self.TYPO_STRICT_METRICS
        self.assertGreaterEqual(overall.precision, INTENT_OPTIMIZER_STRICT_PRECISION_FLOOR)
        self.assertGreaterEqual(typos.precision, INTENT_OPTIMIZER_STRICT_TYPO_PRECISION_FLOOR)
        self.assertLess(typos.precision, INTENT_OPTIMIZER_STRICT_PRECISION_FLOOR)
        selection = ThresholdSelection("space", 1.0, overall, typos)
        selections = {
            trigger: replace(selection, trigger=trigger) for trigger in TRIGGERS
        }

        breakdown = threshold_selection_gate_breakdown(
            training_config,
            selections,
        )
        per_trigger = cast(
            dict[str, object],
            cast(dict[str, object], breakdown["neutral"])["per_trigger"],
        )
        space = cast(dict[str, object], per_trigger["space"])
        self.assertIs(
            cast(dict[str, object], space["overall"])["passed"],
            True,
        )

        typo_gate = cast(dict[str, object], space["typos"])
        self.assertIs(typo_gate["passed"], False)
        self.assertIs(
            cast(dict[str, bool], typo_gate["checks"])["precision"],
            False,
        )
        self.assertEqual(
            cast(dict[str, float], typo_gate["limits"])["minimum_precision"],
            INTENT_OPTIMIZER_STRICT_PRECISION_FLOOR,
        )

        policy = gate_policy_payload(training_config)
        self.assertEqual(
            cast(dict[str, object], policy["selection"])[
                "minimum_typo_precision"
            ],
            INTENT_OPTIMIZER_STRICT_PRECISION_FLOOR,
        )
        self.assertEqual(
            cast(dict[str, object], policy["selection"])[
                "maximum_false_positives_per_trigger"
            ],
            INTENT_OPTIMIZER_SELECTION_FALSE_POSITIVE_BUDGET,
        )
        self.assertEqual(
            cast(dict[str, object], policy["sealed_test"])["minimum_precision"],
            INTENT_OPTIMIZER_STRICT_TYPO_PRECISION_FLOOR,
        )
        with patch.object(
            tim,
            "choose_directional_threshold",
            return_value=selection,
        ) as choose:
            choose_trigger_thresholds((), training_config)
        self.assertEqual(choose.call_count, len(TRIGGERS))
        self.assertTrue(
            all(
                call.kwargs["typo_precision_floor"] == INTENT_OPTIMIZER_STRICT_PRECISION_FLOOR
                for call in choose.call_args_list
            )
        )
        self.assertTrue(
            all(
                call.kwargs["false_positive_z_score"]
                == SELECTION_WILSON_Z_SCORE
                and call.kwargs["typo_false_positive_z_score"]
                == SELECTION_WILSON_Z_SCORE
                and call.kwargs["maximum_false_positives"]
                == INTENT_OPTIMIZER_SELECTION_FALSE_POSITIVE_BUDGET
                for call in choose.call_args_list
            )
        )

    def test_threshold_selection_rejects_invalid_runtime_parameters(self) -> None:
        metrics = ConfusionMatrix(1, 0, 1, 0)
        with self.assertRaisesRegex(ValueError, "both directions"):
            ThresholdSelection(
                "space",
                1.0,
                metrics,
                metrics,
                {"0>1": 1.0},
            )
        with self.assertRaisesRegex(ValueError, "logits must be finite"):
            ThresholdSelection(
                "space",
                1.0,
                metrics,
                metrics,
                {"0>1": math.inf, "1>0": 1.0},
            )
        with self.assertRaisesRegex(ValueError, "logit must be finite"):
            ThresholdSelection(
                "space",
                math.inf,
                metrics,
                metrics,
                {"0>1": 1.0, "1>0": 1.0},
            )
        for invalid_margin in (-1.0, math.inf, math.nan):
            with self.subTest(global_logit_margin=invalid_margin):
                with self.assertRaisesRegex(ValueError, "global logit margin"):
                    ThresholdSelection(
                        "space",
                        1.0,
                        metrics,
                        metrics,
                        global_logit_margin=invalid_margin,
                    )

    def test_selection_uses_auditable_familywise_wilson_bound(self) -> None:
        metrics = self.WILSON_BOUND_METRICS
        ordinary = binary_gate_breakdown(
            metrics,
            minimum_precision=INTENT_WILSON_BOUND_GATE_KWARGS["minimum_precision"],
            minimum_recall=INTENT_WILSON_BOUND_GATE_KWARGS["minimum_recall"],
            minimum_specificity=INTENT_WILSON_BOUND_GATE_KWARGS["minimum_specificity"],
            maximum_false_positive_rate=INTENT_WILSON_BOUND_GATE_KWARGS["maximum_false_positive_rate"],
        )
        familywise = binary_gate_breakdown(
            metrics,
            minimum_precision=INTENT_WILSON_BOUND_GATE_KWARGS["minimum_precision"],
            minimum_recall=INTENT_WILSON_BOUND_GATE_KWARGS["minimum_recall"],
            minimum_specificity=INTENT_WILSON_BOUND_GATE_KWARGS["minimum_specificity"],
            maximum_false_positive_rate=INTENT_WILSON_BOUND_GATE_KWARGS["maximum_false_positive_rate"],
            selection_familywise=True,
        )
        self.assertIs(ordinary["passed"], True)
        self.assertIs(familywise["passed"], False)
        ordinary_bound = cast(
            dict[str, object], ordinary["false_positive_bound"]
        )
        familywise_bound = cast(
            dict[str, object], familywise["false_positive_bound"]
        )
        self.assertEqual(ordinary_bound["multiplicity_correction"], "none")
        self.assertEqual(ordinary_bound["comparisons"], 1)
        self.assertEqual(ordinary_bound["z_score"], WILSON_95_Z_SCORE)
        self.assertEqual(
            familywise_bound["multiplicity_correction"], "bonferroni"
        )
        self.assertEqual(
            familywise_bound["comparisons"],
            SELECTION_FALSE_POSITIVE_COMPARISONS,
        )
        self.assertEqual(
            familywise_bound["per_comparison_confidence"],
            SELECTION_PER_COMPARISON_CONFIDENCE,
        )
        self.assertEqual(
            familywise_bound["z_score"], SELECTION_WILSON_Z_SCORE
        )
        self.assertGreater(
            cast(float, familywise_bound["upper"]),
            cast(float, ordinary_bound["upper"]),
        )
        self.assertEqual(
            false_positive_bound_payload(
                INTENT_OPTIMIZER_FALSE_POSITIVE_BOUND_FALSE_POSITIVES,
                INTENT_OPTIMIZER_FALSE_POSITIVE_BOUND_TOTAL_NEGATIVES,
                selection_familywise=True,
            ),
            familywise_bound,
        )
        policy = gate_policy_payload(config())
        applicability = cast(
            dict[str, object], policy["model_applicability"]
        )
        self.assertEqual(
            applicability["minimum_normalized_token_length"],
            EXPECTED_INTENT_MINIMUM_RUNTIME_TOKEN_LENGTH,
        )
        self.assertEqual(
            applicability["length_comparison"],
            "maximum_of_original_and_replacement",
        )
        self.assertEqual(
            applicability["post_guard_decision_rule"],
            "trigger_direction_calibrated_logit_threshold_only",
        )
        self.assertEqual(
            applicability["membership_coverage_role"],
            "diagnostic_only",
        )
        self.assertEqual(
            applicability["target_language_score_role"],
            "diagnostic_only",
        )
        bounds = cast(dict[str, object], policy["statistical_bounds"])
        selection_policy = cast(dict[str, object], bounds["selection"])
        sealed_policy = cast(dict[str, object], bounds["sealed_test"])
        self.assertEqual(
            selection_policy["comparisons"], len(TRIGGERS) * INTENT_OPTIMIZER_COMPARISONS_PER_TRIGGER
        )
        self.assertEqual(
            selection_policy["familywise_confidence"],
            WILSON_INTERVAL_CONFIDENCE,
        )
        self.assertEqual(sealed_policy["multiplicity_correction"], "none")

    def test_threshold_search_considers_full_typo_policy(self) -> None:
        rows: list[ScoredExample] = []
        rows.extend(
            ScoredExample(
                lexical_example(True, variant="deletion", signature=f"tp{index}"),
                score,
                score,
            )
            for index, score in enumerate(
                (INTENT_OPTIMIZER_HIGH_SCORE,) * INTENT_OPTIMIZER_DELETION_TYPO_STRONG_COUNT
                + (1.0,) * INTENT_OPTIMIZER_DELETION_TYPO_WEAK_COUNT
            )
        )
        rows.extend(
            ScoredExample(
                lexical_example(True, variant="identity", signature=f"ip{index}"),
                score,
                score,
            )
            for index, score in enumerate(
                (INTENT_OPTIMIZER_HIGH_SCORE,) * INTENT_OPTIMIZER_IDENTITY_TYPO_STRONG_COUNT
                + (1.0,) * INTENT_OPTIMIZER_IDENTITY_TYPO_WEAK_COUNT
            )
        )
        rows.extend(
            ScoredExample(
                lexical_example(False, variant="deletion", signature=f"n{index}"),
                1.0 if index == 0 else 0.0,
                1.0 if index == 0 else 0.0,
            )
            for index in range(INTENT_STRONG_SAMPLE_SIZE)
        )
        selected = choose_threshold(
            tuple(rows),
            "space",
            precision_floor=INTENT_FULL_TYPO_POLICY_KWARGS["precision_floor"],
            maximum_false_positive_rate=INTENT_FULL_TYPO_POLICY_KWARGS["maximum_false_positive_rate"],
            minimum_recall=INTENT_FULL_TYPO_POLICY_KWARGS["minimum_recall"],
            minimum_specificity=INTENT_FULL_TYPO_POLICY_KWARGS["minimum_specificity"],
            typo_precision_floor=INTENT_FULL_TYPO_POLICY_KWARGS["typo_precision_floor"],
            minimum_typo_recall=INTENT_FULL_TYPO_POLICY_KWARGS["minimum_typo_recall"],
            typo_minimum_specificity=INTENT_FULL_TYPO_POLICY_KWARGS["typo_minimum_specificity"],
            typo_maximum_false_positive_rate=(
                INTENT_FULL_TYPO_POLICY_KWARGS["typo_maximum_false_positive_rate"]
            ),
        )
        self.assertEqual(selected.logit, INTENT_OPTIMIZER_HIGH_SCORE)
        self.assertEqual(selected.metrics, self.EXPECTED_TYPO_POLICY_METRICS)
        self.assertEqual(selected.typo_metrics, self.EXPECTED_TYPO_POLICY_TYPO_METRICS)

        statistically_weak_typos = tuple(
            ScoredExample(
                lexical_example(
                    False,
                    variant=(
                        "deletion" if index < INTENT_WEAK_SAMPLE_SIZE else "identity"
                    ),
                    signature=f"wn{index}",
                ),
                0.0,
                0.0,
            )
            for index in range(INTENT_STRONG_SAMPLE_SIZE)
        ) + tuple(
            ScoredExample(
                lexical_example(True, variant="deletion", signature=f"wp{index}"),
                INTENT_OPTIMIZER_HIGH_SCORE,
                INTENT_OPTIMIZER_HIGH_SCORE,
            )
            for index in range(INTENT_WEAK_SAMPLE_SIZE)
        )
        self.assertLess(
            wilson_upper_bound(0, INTENT_STRONG_SAMPLE_SIZE), INTENT_STRICT_FALSE_POSITIVE_RATE
        )
        self.assertGreater(
            wilson_upper_bound(0, INTENT_WEAK_SAMPLE_SIZE), INTENT_STRICT_FALSE_POSITIVE_RATE
        )
        weak_selection = choose_threshold(
            statistically_weak_typos,
            "space",
            precision_floor=INTENT_WEAK_TYPO_POLICY_KWARGS["precision_floor"],
            maximum_false_positive_rate=INTENT_WEAK_TYPO_POLICY_KWARGS["maximum_false_positive_rate"],
            minimum_recall=INTENT_WEAK_TYPO_POLICY_KWARGS["minimum_recall"],
            minimum_specificity=INTENT_WEAK_TYPO_POLICY_KWARGS["minimum_specificity"],
            typo_precision_floor=INTENT_WEAK_TYPO_POLICY_KWARGS["typo_precision_floor"],
            minimum_typo_recall=INTENT_WEAK_TYPO_POLICY_KWARGS["minimum_typo_recall"],
            typo_minimum_specificity=INTENT_WEAK_TYPO_POLICY_KWARGS["typo_minimum_specificity"],
            typo_maximum_false_positive_rate=(
                INTENT_WEAK_TYPO_POLICY_KWARGS["typo_maximum_false_positive_rate"]
            ),
        )
        weak_config = config(**INTENT_WEAK_TRAINING_CONFIG_OVERRIDES)
        weak_selections = {
            trigger: replace(weak_selection, trigger=trigger)
            for trigger in TRIGGERS
        }
        weak_breakdown = threshold_selection_gate_breakdown(
            weak_config,
            weak_selections,
        )
        self.assertIs(weak_breakdown["passed"], False)

    def test_selection_tail_diagnostics_explain_targets_without_tokens(self) -> None:
        rows: list[ScoredExample] = []
        for trigger in TRIGGERS:
            templates = INTENT_TAIL_DIAGNOSTIC_TEMPLATES
            for index, (label, variant, score, frequency) in enumerate(
                templates
            ):
                example = replace(
                    lexical_example(
                        label,
                        trigger=trigger,
                        variant=variant,
                        signature=f"{trigger}-{index}",
                    ),
                    frequency=frequency,
                )
                rows.append(ScoredExample(example, score, score))
        selections = {
            trigger: ThresholdSelection(
                trigger,
                INTENT_OPTIMIZER_TAIL_DIAGNOSTIC_LOGIT,
                self.TAIL_DIAGNOSTIC_OVERALL_METRICS,
                ConfusionMatrix(1, 0, 1, 0),
            )
            for trigger in TRIGGERS
        }

        diagnostics = selection_tail_diagnostics(
            tuple(rows),
            selections,
            config(),
        )

        self.assertEqual(diagnostics["schema_version"], 1)
        self.assertEqual(diagnostics["source_split"], "threshold")
        self.assertIs(diagnostics["contains_lexical_tokens"], False)
        per_trigger = cast(dict[str, object], diagnostics["per_trigger"])
        space = cast(dict[str, object], per_trigger["space"])
        self.assertEqual(
            space["ordinary_recall_target"],
            DEFAULT_TRAINING_CONFIG_VALUES["selection_minimum_recall"],
        )
        ordinary = cast(dict[str, object], space["ordinary_target_point"])
        self.assertEqual(ordinary["logit"], INTENT_OPTIMIZER_EXPECTED_ORDINARY_TAIL_LOGIT)
        typo = cast(dict[str, object], space["typo_target_point"])
        self.assertEqual(typo["logit"], INTENT_OPTIMIZER_EXPECTED_TYPO_TAIL_LOGIT)
        slices = cast(dict[str, object], space["selected_metric_slices"])
        frequencies = cast(dict[str, object], slices["frequency"])
        self.assertEqual(set(frequencies), {"1-9", "100-999"})
        encoded = json.dumps(diagnostics, ensure_ascii=False, sort_keys=True)
        self.assertNotIn("hello", encoded)
        self.assertNotIn("руддщ", encoded)

        incomplete = dict(selections)
        del incomplete["tab"]
        with self.assertRaisesRegex(ValueError, "every trigger"):
            selection_tail_diagnostics(tuple(rows), incomplete, config())

    def test_pause_margin_is_rechecked_by_presealed_selection_gate(self) -> None:
        rows: list[ScoredExample] = []
        for trigger in TRIGGERS:
            negative_score = 0.0 if trigger == "pause" else INTENT_OPTIMIZER_HIGH_SCORE
            positive_scores = (
                INTENT_OPTIMIZER_PAUSE_POSITIVE_SCORES if trigger == "pause" else INTENT_OPTIMIZER_NON_PAUSE_POSITIVE_SCORES
            )
            rows.extend(
                ScoredExample(
                    lexical_example(
                        False,
                        trigger=trigger,
                        variant="deletion",
                        signature=f"{trigger}-n{index}",
                        direction="0>1" if index % INTENT_LABEL_PARITY_MODULUS == 0 else "1>0",
                    ),
                    negative_score,
                    negative_score,
                )
                for index in range(INTENT_STRONG_SAMPLE_SIZE)
            )
            rows.extend(
                ScoredExample(
                    lexical_example(
                        True,
                        trigger=trigger,
                        variant="deletion",
                        signature=f"{trigger}-p{index}",
                        direction="0>1" if index == 0 else "1>0",
                    ),
                    score,
                    score,
                )
                for index, score in enumerate(positive_scores)
            )
        gate_config = config(**INTENT_PAUSE_MARGIN_CONFIG_OVERRIDES)
        selected = choose_trigger_thresholds(tuple(rows), gate_config)
        self.assertGreater(selected["pause"].logit, INTENT_OPTIMIZER_PAUSE_LOGIT_LOWER_BOUND)
        self.assertEqual(selected["pause"].metrics.true_positive, 0)
        breakdown = threshold_selection_gate_breakdown(gate_config, selected)
        self.assertIs(breakdown["passed"], False)
        pause = cast(
            dict[str, object],
            cast(dict[str, object], breakdown["per_trigger"])["pause"],
        )
        self.assertIs(pause["passed"], False)

    def test_global_threshold_margin_hardens_every_direction_before_pause(self) -> None:
        rows: list[ScoredExample] = []
        directions: tuple[LayoutDirection, LayoutDirection] = ("0>1", "1>0")
        for trigger in TRIGGERS:
            for direction in directions:
                rows.append(
                    ScoredExample(
                        lexical_example(
                            False,
                            trigger=trigger,
                            variant="deletion",
                            signature=f"{trigger}-{direction}-negative",
                            direction=direction,
                        ),
                        0.0,
                        0.0,
                    )
                )
                rows.append(
                    ScoredExample(
                        lexical_example(
                            True,
                            trigger=trigger,
                            variant="deletion",
                            signature=f"{trigger}-{direction}-positive",
                            direction=direction,
                        ),
                        INTENT_OPTIMIZER_POSITIVE_ROW_SCORE,
                        INTENT_OPTIMIZER_POSITIVE_ROW_SCORE,
                    )
                )
        gate_config = config(
            threshold_max_false_positive_rate=1.0,
            pause_threshold_max_false_positive_rate=1.0,
            threshold_logit_margin_cap=INTENT_OPTIMIZER_LOGIT_MARGIN_CAP,
            pause_logit_margin=INTENT_OPTIMIZER_PAUSE_LOGIT_MARGIN_PROBE,
        )
        initial = self.INITIAL_SYMMETRIC_SELECTION
        with patch.object(
            tim,
            "choose_directional_threshold",
            return_value=initial,
        ):
            selected = choose_trigger_thresholds(tuple(rows), gate_config)
        base = INTENT_OPTIMIZER_EXPECTED_MARGIN_BASE_LOGIT
        for trigger in TRIGGERS:
            expected = base + (INTENT_OPTIMIZER_PAUSE_LOGIT_MARGIN_PROBE if trigger == "pause" else 0.0)
            self.assertEqual(selected[trigger].global_logit_margin, INTENT_OPTIMIZER_LOGIT_MARGIN_CAP)
            for direction in directions:
                self.assertEqual(
                    selected[trigger].logit_for(direction), expected
                )
            self.assertEqual(selected[trigger].metrics.false_positive, 0)
            self.assertEqual(
                selected[trigger].metrics.true_positive, INTENT_OPTIMIZER_EXPECTED_TRUE_POSITIVE_COUNT
            )
            self.assertEqual(
                selected[trigger].typo_metrics.true_positive, INTENT_OPTIMIZER_EXPECTED_TRUE_POSITIVE_COUNT
            )

    def test_global_threshold_margin_backs_off_to_largest_feasible_value(
        self,
    ) -> None:
        rows: list[ScoredExample] = []
        directions: tuple[LayoutDirection, LayoutDirection] = ("0>1", "1>0")
        for trigger in TRIGGERS:
            positive_score = (
                INTENT_OPTIMIZER_EXPECTED_MARGIN_BASE_LOGIT if trigger == "pause" else INTENT_OPTIMIZER_BACKOFF_NON_PAUSE_SCORE
            )
            for direction in directions:
                rows.append(
                    ScoredExample(
                        lexical_example(
                            False,
                            trigger=trigger,
                            variant="deletion",
                            signature=f"{trigger}-{direction}-negative",
                            direction=direction,
                        ),
                        0.0,
                        0.0,
                    )
                )
                rows.append(
                    ScoredExample(
                        lexical_example(
                            True,
                            trigger=trigger,
                            variant="deletion",
                            signature=f"{trigger}-{direction}-positive",
                            direction=direction,
                        ),
                        positive_score,
                        positive_score,
                    )
                )
        gate_config = config(**INTENT_BACKOFF_CONFIG_OVERRIDES)
        initial = self.INITIAL_SYMMETRIC_SELECTION
        with patch.object(
            tim,
            "choose_directional_threshold",
            return_value=initial,
        ):
            selected = choose_trigger_thresholds(tuple(rows), gate_config)

        for trigger in TRIGGERS:
            expected = (
                INTENT_OPTIMIZER_EXPECTED_MARGIN_BASE_LOGIT if trigger == "pause" else INTENT_OPTIMIZER_BACKOFF_NON_PAUSE_SCORE
            )
            self.assertEqual(
                selected[trigger].global_logit_margin, INTENT_OPTIMIZER_BACKOFF_EXPECTED_MARGIN
            )
            for direction in directions:
                self.assertEqual(
                    selected[trigger].logit_for(direction), expected
                )
            self.assertEqual(selected[trigger].metrics.false_negative, 0)
            self.assertEqual(selected[trigger].typo_metrics.false_negative, 0)
        overall = {
            trigger: selection.metrics
            for trigger, selection in selected.items()
        }
        typos = {
            trigger: selection.typo_metrics
            for trigger, selection in selected.items()
        }
        context = {
            profile.name: overall for profile in CONTEXT_STRESS_PROFILES
        }
        context_typos = {
            profile.name: typos for profile in CONTEXT_STRESS_PROFILES
        }
        self.assertIs(
            threshold_selection_gate_breakdown(
                gate_config,
                selected,
                context,
                context_typos,
            )["passed"],
            True,
        )

    def test_context_stress_gates_require_every_profile_trigger_and_label(
        self,
    ) -> None:
        gate_config = config(
            threshold_max_false_positive_rate=INTENT_STRICT_FALSE_POSITIVE_RATE,
            pause_threshold_max_false_positive_rate=INTENT_STRICT_FALSE_POSITIVE_RATE,
        )
        strong = self.STRONG_SYMMETRIC_METRICS
        per_trigger = {trigger: strong for trigger in TRIGGERS}
        context_metrics = {
            profile.name: per_trigger
            for profile in CONTEXT_STRESS_PROFILES
        }
        selections = {
            trigger: ThresholdSelection(trigger, 0.0, strong, strong)
            for trigger in TRIGGERS
        }
        complete = threshold_selection_gate_breakdown(
            gate_config,
            selections,
            context_metrics,
            context_metrics,
        )
        self.assertIs(complete["passed"], True)
        context_breakdown = context_stress_gate_breakdown(
            gate_config,
            context_metrics,
            context_metrics,
            phase="sealed_test",
        )
        self.assertIs(context_breakdown["passed"], True)

        missing_profile = dict(context_metrics)
        del missing_profile[CONTEXT_STRESS_PROFILES[0].name]
        self.assertIs(
            context_stress_gate_breakdown(
                gate_config,
                missing_profile,
                context_metrics,
                phase="selection",
            )["passed"],
            False,
        )

        empty_label_slice = dict(context_metrics)
        empty_trigger_map = dict(per_trigger)
        empty_trigger_map["boundary_probe"] = self.EMPTY_TRIGGER_METRICS
        empty_label_slice[CONTEXT_STRESS_PROFILES[0].name] = (
            empty_trigger_map
        )
        failed = context_stress_gate_breakdown(
            gate_config,
            empty_label_slice,
            context_metrics,
            phase="sealed_test",
        )
        self.assertIs(failed["passed"], False)

    def test_quality_gates_reject_empty_slices_and_weak_sample_bounds(self) -> None:
        gate_config = config(
            threshold_max_false_positive_rate=INTENT_STRICT_FALSE_POSITIVE_RATE,
            pause_threshold_max_false_positive_rate=INTENT_STRICT_FALSE_POSITIVE_RATE,
        )
        strong = self.STRONG_SYMMETRIC_METRICS
        per_trigger = {trigger: strong for trigger in TRIGGERS}
        context_metrics = {
            profile.name: per_trigger
            for profile in CONTEXT_STRESS_PROFILES
        }
        safety_audit = GuardedSafetyAudit(
            samples=len(TRIGGERS) * INTENT_SAFETY_SAMPLE_MULTIPLIER,
            protected_samples=len(TRIGGERS),
            lexical_collision_samples=len(TRIGGERS),
            triggers=tuple(TRIGGERS),
            protected_triggers=tuple(TRIGGERS),
            lexical_collision_triggers=tuple(TRIGGERS),
            failures=(),
        )
        veto = VetoSelection(-1.0, INTENT_VETO_SELECTION_SAMPLE_COUNT, 0, 0.0)
        self.assertTrue(
            training_quality_gates_pass(
                gate_config,
                per_trigger,
                per_trigger,
                safety_audit,
                veto,
                veto,
                context_metrics,
                context_metrics,
            )
        )

        self.assertFalse(
            training_quality_gates_pass(
                gate_config,
                per_trigger,
                per_trigger,
                safety_audit,
                veto,
                veto,
            )
        )

        no_positive = dict(per_trigger)
        no_positive["pause"] = self.EMPTY_TRIGGER_METRICS
        self.assertFalse(
            training_quality_gates_pass(
                gate_config,
                per_trigger,
                no_positive,
                safety_audit,
                veto,
                veto,
                context_metrics,
                context_metrics,
            )
        )
        weak_sample_bound = dict(per_trigger)
        weak_sample_bound["space"] = self.WEAK_SAMPLE_METRICS
        self.assertGreater(
            wilson_upper_bound(0, INTENT_WEAK_SAMPLE_SIZE), INTENT_STRICT_FALSE_POSITIVE_RATE
        )
        self.assertFalse(
            training_quality_gates_pass(
                gate_config,
                weak_sample_bound,
                per_trigger,
                safety_audit,
                veto,
                veto,
                context_metrics,
                context_metrics,
            )
        )
        self.assertFalse(
            training_quality_gates_pass(
                gate_config,
                per_trigger,
                per_trigger,
                replace(safety_audit, failures=("unguarded",)),
                veto,
                veto,
                context_metrics,
                context_metrics,
            )
        )
        with self.assertRaisesRegex(ValueError, "between one and three"):
            config(typo_augmentations=0).validate()
        with self.assertRaisesRegex(ValueError, "MAX_DIMENSION"):
            config(dimension=1 << INTENT_EXCESSIVE_DIMENSION_EXPONENT).validate()
        with self.assertRaisesRegex(
            ValueError, "pause_threshold_max_false_positive_rate"
        ):
            config(
                pause_threshold_max_false_positive_rate=INTENT_OPTIMIZER_INVALID_FALSE_POSITIVE_RATE
            ).validate()
        for invalid_budget in (-1, INTENT_OPTIMIZER_INVALID_SELECTION_BUDGET, True):
            with self.subTest(selection_budget=invalid_budget):
                with self.assertRaisesRegex(
                    ValueError,
                    "selection_maximum_false_positives_per_trigger",
                ):
                    config(
                        selection_maximum_false_positives_per_trigger=(
                            invalid_budget
                        )
                    ).validate()
        for field_name, sealed_value in INTENT_OPTIMIZER_SEALED_VALUE_BY_FIELD:
            with self.subTest(selection_recall=field_name):
                with self.assertRaisesRegex(
                    ValueError, "must not be below its sealed-test minimum"
                ):
                    config(
                        **{field_name: sealed_value - INTENT_OPTIMIZER_RECALL_UNDER_SEALED_MARGIN}
                    ).validate()
        for field_name in ("threshold_logit_margin_cap", "pause_logit_margin"):
            for invalid_margin in (-1.0, math.inf, math.nan):
                with self.subTest(
                    margin_field=field_name,
                    invalid_margin=invalid_margin,
                ):
                    with self.assertRaisesRegex(
                        ValueError, f"{field_name} must be finite"
                    ):
                        config(**{field_name: invalid_margin}).validate()


class ExternalEvaluationTests(unittest.TestCase):
    EMPTY_TRIGGER_METRICS = ConfusionMatrix(0, 0, INTENT_STRONG_SAMPLE_SIZE, 0)
    WEAK_SAMPLE_METRICS = ConfusionMatrix(INTENT_WEAK_SAMPLE_SIZE, 0, INTENT_WEAK_SAMPLE_SIZE, 0)
    STRONG_SYMMETRIC_METRICS = ConfusionMatrix(INTENT_STRONG_SAMPLE_SIZE, 0, INTENT_STRONG_SAMPLE_SIZE, 0)
    COVERAGE_DIAGNOSTIC_ROWS = (
        ModelPredictionRow(lexical_example(False), False, -INTENT_EXTERNAL_COVERAGE_CONFIDENT_LOGIT, 0.0),
        ModelPredictionRow(
            lexical_example(True), True, INTENT_EXTERNAL_COVERAGE_CONFIDENT_LOGIT, INTENT_EXTERNAL_COVERAGE_LOW_PARTIAL
        ),
        ModelPredictionRow(lexical_example(False), False, -1.0, INTENT_EXTERNAL_COVERAGE_HIGH_PARTIAL),
        ModelPredictionRow(lexical_example(True), True, 1.0, 1.0),
    )

    def test_external_provenance_gate_schema_is_exact_and_type_safe(self) -> None:
        passed = {
            name: True for name in EXTERNAL_CORPUS_PROVENANCE_GATE_NAMES
        }
        self.assertTrue(external_corpus_provenance_gates_pass(passed))
        missing = dict(passed)
        missing.pop(next(iter(EXTERNAL_CORPUS_PROVENANCE_GATE_NAMES)))
        self.assertFalse(external_corpus_provenance_gates_pass(missing))
        self.assertFalse(
            external_corpus_provenance_gates_pass(
                {**passed, "unexpected": True}
            )
        )
        truthy: dict[str, object] = dict(passed)
        truthy[next(iter(EXTERNAL_CORPUS_PROVENANCE_GATE_NAMES))] = 1
        self.assertFalse(external_corpus_provenance_gates_pass(truthy))

    def test_strict_evaluator_sample_sizes_are_canonical(self) -> None:
        canonical = eim._parse_arguments(("--strict",))
        self.assertEqual(canonical.comparison_sample, INTENT_EXTERNAL_EXPECTED_STRICT_SAMPLE_SIZE)
        self.assertEqual(canonical.latency_sample, INTENT_EXTERNAL_EXPECTED_STRICT_SAMPLE_SIZE)
        for option, value in (
            ("--comparison-sample", "1"),
            ("--comparison-sample", "5001"),
            ("--latency-sample", "1"),
            ("--latency-sample", "5001"),
        ):
            with self.subTest(option=option, value=value):
                with self.assertRaises(SystemExit):
                    eim._parse_arguments(("--strict", option, value))

        diagnostic = eim._parse_arguments(
            ("--comparison-sample", "1", "--latency-sample", "1")
        )
        self.assertEqual(diagnostic.comparison_sample, 1)
        self.assertEqual(diagnostic.latency_sample, 1)

    def setUp(self) -> None:
        self.english = EvaluationLanguageModel("en_US", {"keyboard"})
        self.russian = EvaluationLanguageModel("ru_RU", {"привет"})

    def language_model(self, locale: str) -> EvaluationLanguageModel:
        return self.english if locale == "en_US" else self.russian

    @staticmethod
    def external_policy_payload() -> dict[str, object]:
        locale = {
            "dictionary_sha256": "1" * SHA256_HEX_CHARACTERS,
            "dictionary_bytes": INTENT_EXTERNAL_DICTIONARY_BYTES,
            "affix_sha256": "2" * SHA256_HEX_CHARACTERS,
            "affix_bytes": INTENT_EXTERNAL_AFFIX_BYTES,
        }
        return {
            "schema_version": INTENT_EXTERNAL_POLICY_SCHEMA_VERSION,
            "minimum_words_per_group": INTENT_EXTERNAL_MINIMUM_WORDS_PER_GROUP,
            "trigger_expansion": list(TRIGGERS),
            "hunspell": {
                "en_US": dict(locale),
                "ru_RU": {
                    **locale,
                    "dictionary_sha256": "3" * SHA256_HEX_CHARACTERS,
                    "affix_sha256": "4" * SHA256_HEX_CHARACTERS,
                },
            },
            "lexical_disjoint_corpus_sha256": "5" * SHA256_HEX_CHARACTERS,
            "unknown_typo_development_corpus_sha256": "6" * SHA256_HEX_CHARACTERS,
            "unknown_typo_holdout_corpus_sha256": "7" * SHA256_HEX_CHARACTERS,
        }

    def dictionary_words(
        self, model: EvaluationLanguageModel
    ) -> tuple[str, ...]:
        return tuple(sorted(model.known_words))

    def dictionary_snapshots(self) -> dict[int, HunspellDictionarySnapshot]:
        result: dict[int, HunspellDictionarySnapshot] = {}
        for group, model in ((0, self.english), (1, self.russian)):
            locale = model.locale
            dictionary_bytes = (
                "1\n" + "\n".join(self.dictionary_words(model)) + "\n"
            ).encode()
            affix_bytes = b"SET UTF-8\n"
            result[group] = HunspellDictionarySnapshot(
                words=self.dictionary_words(model),
                provenance=HunspellDictionaryProvenance(
                    locale=locale,
                    dictionary=FrozenExternalFile(
                        path=f"/{locale}.dic",
                        sha256=hashlib.sha256(dictionary_bytes).hexdigest(),
                        bytes=len(dictionary_bytes),
                    ),
                    affix=FrozenExternalFile(
                        path=f"/{locale}.aff",
                        sha256=hashlib.sha256(affix_bytes).hexdigest(),
                        bytes=len(affix_bytes),
                    ),
                ),
            )
        return result

    def external_policy(
        self,
        lexical_corpus: LexicalDisjointCorpus,
        unknown_corpus: LexicalDisjointCorpus,
        development_corpus: LexicalDisjointCorpus | None = None,
        *,
        minimum_words_per_group: int = 1,
        trigger_expansion: tuple[str, ...] = tuple(TRIGGERS),
    ) -> ExternalEvaluationPolicy:
        hunspell = {
            group: HunspellLocalePolicy(
                dictionary=FrozenExternalFilePolicy(
                    sha256=item.dictionary.sha256,
                    bytes=item.dictionary.bytes,
                ),
                affix=FrozenExternalFilePolicy(
                    sha256=item.affix.sha256,
                    bytes=item.affix.bytes,
                ),
            )
            for group, item in lexical_corpus.dictionary_provenance.items()
        }
        development = development_corpus or unknown_corpus
        return ExternalEvaluationPolicy(
            schema_version=INTENT_EXTERNAL_POLICY_SCHEMA_VERSION,
            minimum_words_per_group=minimum_words_per_group,
            trigger_expansion=trigger_expansion,
            hunspell=hunspell,
            lexical_disjoint_corpus_sha256=lexical_corpus.corpus_sha256,
            unknown_typo_development_corpus_sha256=(
                development.corpus_sha256
            ),
            unknown_typo_holdout_corpus_sha256=unknown_corpus.corpus_sha256,
        )

    def test_external_policy_parser_is_strict_and_type_safe(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "config.json"
            repository = Path(__file__).resolve().parents[1]
            base_config = cast(
                dict[str, object],
                json.loads(
                    (repository / "model/intent_v1/config.json").read_text(
                        encoding="utf-8"
                    )
                ),
            )

            def write(policy: object) -> None:
                complete = dict(base_config)
                complete["external_evaluation"] = policy
                path.write_text(
                    json.dumps(complete),
                    encoding="utf-8",
                )

            payload = self.external_policy_payload()
            write(payload)
            parsed = load_external_evaluation_policy(path)
            self.assertEqual(parsed.schema_version, INTENT_EXTERNAL_POLICY_SCHEMA_VERSION)
            self.assertEqual(
                parsed.minimum_words_per_group, INTENT_EXTERNAL_MINIMUM_WORDS_PER_GROUP
            )
            self.assertEqual(parsed.trigger_expansion, tuple(TRIGGERS))
            self.assertEqual(
                parsed.hunspell[0].dictionary.bytes, INTENT_EXTERNAL_DICTIONARY_BYTES
            )

            missing_root = dict(base_config)
            missing_root.pop("external_evaluation")
            path.write_text(json.dumps(missing_root), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "config fields mismatch"):
                load_external_evaluation_policy(path)

            missing = dict(payload)
            missing.pop("unknown_typo_holdout_corpus_sha256")
            write(missing)
            with self.assertRaisesRegex(ValueError, "fields mismatch"):
                load_external_evaluation_policy(path)

            boolean_size = json.loads(json.dumps(payload))
            boolean_size["hunspell"]["en_US"]["dictionary_bytes"] = True
            write(boolean_size)
            with self.assertRaisesRegex(ValueError, "must be an integer"):
                load_external_evaluation_policy(path)

            float_size = json.loads(json.dumps(payload))
            float_size["hunspell"]["en_US"]["affix_bytes"] = INTENT_EXTERNAL_INVALID_AFFIX_BYTES_FLOAT
            write(float_size)
            with self.assertRaisesRegex(ValueError, "must be an integer"):
                load_external_evaluation_policy(path)

            bad_digest = json.loads(json.dumps(payload))
            bad_digest["hunspell"]["ru_RU"]["affix_sha256"] = "A" * SHA256_HEX_CHARACTERS
            write(bad_digest)
            with self.assertRaisesRegex(ValueError, "exact lowercase SHA-256"):
                load_external_evaluation_policy(path)

            bad_trigger = json.loads(json.dumps(payload))
            bad_trigger["trigger_expansion"][0] = 1
            write(bad_trigger)
            with self.assertRaisesRegex(ValueError, "must be strings"):
                load_external_evaluation_policy(path)

    def test_hunspell_snapshot_hashes_the_same_single_bounded_reads(self) -> None:
        model = self.english
        dictionary_bytes = b"2\nkeyboard\nlayout\n"
        affix_bytes = b"SET UTF-8\n"
        reads: list[Path] = []

        def read_once(path: Path, maximum_bytes: int, *, label: str) -> bytes:
            del maximum_bytes, label
            reads.append(path)
            return affix_bytes if path.suffix == ".aff" else dictionary_bytes

        with patch(
            "evaluate_intent_model._read_bounded_external_file",
            side_effect=read_once,
        ):
            snapshot = eim._hunspell_dictionary_snapshot(
                cast(LanguageModel, model), "en_US"
            )
        self.assertEqual(
            reads,
            [Path("/en_US.aff"), Path("/en_US.dic")],
        )
        self.assertEqual(snapshot.words, ("keyboard", "layout"))
        self.assertEqual(
            snapshot.provenance.dictionary.sha256,
            hashlib.sha256(dictionary_bytes).hexdigest(),
        )
        self.assertEqual(snapshot.provenance.dictionary.bytes, len(dictionary_bytes))
        self.assertEqual(
            snapshot.provenance.affix.sha256,
            hashlib.sha256(affix_bytes).hexdigest(),
        )
        self.assertEqual(snapshot.provenance.affix.bytes, len(affix_bytes))

    def test_external_coverage_diagnostics_are_descriptive_and_sliced(self) -> None:
        rows = self.COVERAGE_DIAGNOSTIC_ROWS
        diagnostics = coverage_diagnostics(rows)
        overall = diagnostics["overall"]
        self.assertEqual(overall.samples, INTENT_EXTERNAL_EXPECTED_COVERAGE_SAMPLE_COUNT)
        self.assertEqual(overall.minimum, 0.0)
        self.assertEqual(overall.p05, 0.0)
        self.assertEqual(overall.p25, INTENT_EXTERNAL_EXPECTED_COVERAGE_P25)
        self.assertEqual(overall.median, INTENT_EXTERNAL_EXPECTED_COVERAGE_MEDIAN)
        self.assertEqual(overall.p75, INTENT_EXTERNAL_EXPECTED_COVERAGE_P75)
        self.assertEqual(overall.p95, 1.0)
        self.assertEqual(overall.maximum, 1.0)
        self.assertEqual(overall.mean, INTENT_EXTERNAL_EXPECTED_COVERAGE_MEDIAN)
        self.assertEqual(overall.zero_coverage_samples, 1)
        self.assertEqual(overall.full_coverage_samples, 1)
        self.assertEqual(diagnostics["label_negative"].samples, INTENT_EXTERNAL_EXPECTED_COVERAGE_PER_LABEL_COUNT)
        self.assertEqual(diagnostics["label_positive"].samples, INTENT_EXTERNAL_EXPECTED_COVERAGE_PER_LABEL_COUNT)
        self.assertEqual(
            diagnostics["direction_0_to_1"].samples, INTENT_EXTERNAL_EXPECTED_COVERAGE_PER_LABEL_COUNT
        )
        self.assertEqual(
            diagnostics["direction_1_to_0"].samples, INTENT_EXTERNAL_EXPECTED_COVERAGE_PER_LABEL_COUNT
        )
        self.assertEqual(diagnostics["trigger_space"].samples, INTENT_EXTERNAL_EXPECTED_COVERAGE_SAMPLE_COUNT)
        self.assertNotIn("trigger_pause", diagnostics)
        with self.assertRaisesRegex(ValueError, "finite"):
            coverage_diagnostics((replace(rows[0], coverage=math.nan),))
        with self.assertRaisesRegex(ValueError, "at least one"):
            coverage_diagnostics(())

    def test_external_raw_predictions_use_runtime_language_scorers(self) -> None:
        example = lexical_example(False)
        runtime_scorers: dict[int, WordScorer] = {
            0: EvaluationLanguageModel("en_US", {example.original}),
            1: EvaluationLanguageModel("ru_RU", {example.alternative}),
        }
        train_only_scorers: dict[int, WordScorer] = {
            0: EvaluationLanguageModel("en_US", set()),
            1: EvaluationLanguageModel("ru_RU", set()),
        }
        spy = SpyIntentModel()

        predict_model_examples(
            cast(LinearNgramModel, spy),
            (example,),
            scorers=runtime_scorers,
        )

        self.assertEqual(len(spy.inputs), 1)
        observed = spy.inputs[0]
        expected_source = runtime_scorers[0].score(example.original)
        expected_target = runtime_scorers[1].score(example.alternative)
        train_source = train_only_scorers[0].score(example.original)
        train_target = train_only_scorers[1].score(example.alternative)
        self.assertEqual(observed.source_score, expected_source)
        self.assertEqual(observed.target_score, expected_target)
        self.assertNotEqual(observed.source_score, train_source)
        self.assertNotEqual(observed.target_score, train_target)

    def test_evaluator_proves_context_invariance_then_reuses_neutral_runtime_predictions(
        self,
    ) -> None:
        examples = tuple(
            lexical_example(label, trigger=trigger, variant=variant)
            for trigger in TRIGGERS
            for label in (False, True)
            for variant in ("identity", "deletion")
        )
        spy = SpyIntentModel()
        overall, typos = evaluate_context_stress(
            cast(LinearNgramModel, spy),
            examples,
            scorers=test_word_scorers(),
        )

        self.assertEqual(
            set(overall),
            {profile.name for profile in CONTEXT_STRESS_PROFILES},
        )
        self.assertEqual(set(typos), set(overall))
        self.assertEqual(len(spy.inputs), len(examples))
        self.assertTrue(
            all(
                item.context_delta == 0.0 and item.context_group is None
                for item in spy.inputs
            )
        )
        for profile in CONTEXT_STRESS_PROFILES:
            for trigger in TRIGGERS:
                self.assertEqual(
                    overall[profile.name][trigger],
                    ConfusionMatrix(
                        INTENT_PERFECT_CONFUSION_COUNT, 0, 0, INTENT_PERFECT_CONFUSION_COUNT
                    ),
                )
                self.assertEqual(
                    typos[profile.name][trigger],
                    ConfusionMatrix(1, 0, 0, 1),
                )

        neutral_predictions = tuple(
            ModelPredictionRow(example, True, INTENT_EXTERNAL_NEUTRAL_PREDICTION_LOGIT, 1.0)
            for example in examples
        )
        reused_spy = SpyIntentModel()
        reused_overall, reused_typos = evaluate_context_stress(
            cast(LinearNgramModel, reused_spy),
            examples,
            scorers=test_word_scorers(),
            neutral_predictions=neutral_predictions,
        )
        self.assertEqual(reused_overall, overall)
        self.assertEqual(reused_typos, typos)
        self.assertEqual(reused_spy.inputs, [])
        with self.assertRaisesRegex(ValueError, "neutral predictions"):
            evaluate_context_stress(
                cast(LinearNgramModel, reused_spy),
                examples,
                scorers=test_word_scorers(),
                neutral_predictions=neutral_predictions[:-1],
            )
        with self.assertRaisesRegex(ValueError, "neutral predictions"):
            evaluate_context_stress(
                cast(LinearNgramModel, reused_spy),
                examples,
                scorers=test_word_scorers(),
                neutral_predictions=(
                    replace(
                        neutral_predictions[0],
                        example=replace(examples[0], original="different"),
                    ),
                    *neutral_predictions[1:],
                ),
            )

    def test_evaluator_verifies_frozen_sources_before_loading_with_logical_metadata(self) -> None:
        repository = Path(__file__).resolve().parents[1]
        config_path = repository / "model/intent_v1/config.json"
        loaded_config = load_training_config(config_path)
        events: list[str] = []
        calls: list[tuple[str, Path, int, dict[str, object]]] = []

        class StopAfterSourceCalls(RuntimeError):
            pass

        def verify_sources(
            config_value: TrainingConfig,
            english_path: Path,
            russian_path: Path,
        ) -> Path:
            self.assertEqual(config_value, loaded_config)
            self.assertEqual(english_path, repository / loaded_config.sources.english.path)
            self.assertEqual(russian_path, repository / loaded_config.sources.russian.path)
            events.append("verify")
            return repository / loaded_config.sources.license_evidence.path

        def load_source(
            path: Path,
            locale: str,
            group: int,
            config_value: TrainingConfig,
            **metadata: object,
        ) -> tuple[tuple[LexiconWord, ...], LexiconSource]:
            self.assertEqual(events[0], "verify")
            self.assertEqual(config_value, loaded_config)
            events.append(f"load:{locale}")
            calls.append((locale, path, group, dict(metadata)))
            if locale == "ru_RU":
                raise StopAfterSourceCalls
            return (), LexiconSource(
                locale,
                group,
                str(metadata["logical_path"]),
                "0" * SHA256_HEX_CHARACTERS,
                1,
                str(metadata["license_declaration"]),
                str(metadata["license_evidence"]),
            )

        english_path = repository / loaded_config.sources.english.path
        russian_path = repository / loaded_config.sources.russian.path
        with patch(
            "evaluate_intent_model.verify_training_sources",
            side_effect=verify_sources,
        ), patch(
            "evaluate_intent_model.load_onboard_unigrams",
            side_effect=load_source,
        ):
            with self.assertRaises(StopAfterSourceCalls):
                evaluate_main(
                    (
                        "--config",
                        str(config_path),
                        "--en-model",
                        str(english_path),
                        "--ru-model",
                        str(russian_path),
                    )
                )
        self.assertEqual(events, ["verify", "load:en_US", "load:ru_RU"])
        self.assertEqual([group for _locale, _path, group, _metadata in calls], [0, 1])
        for call, frozen_source in zip(
            calls,
            (loaded_config.sources.english, loaded_config.sources.russian),
            strict=True,
        ):
            _call_locale, call_path, _call_group, call_metadata = call
            self.assertEqual(call_path, repository / frozen_source.path)
            self.assertEqual(
                call_metadata["logical_path"], frozen_source.path
            )
            self.assertEqual(
                call_metadata["license_declaration"],
                loaded_config.sources.license_declaration,
            )
            self.assertEqual(
                call_metadata["license_evidence"],
                loaded_config.sources.license_evidence.path,
            )

    def test_provenance_includes_the_independent_membership_seed(self) -> None:
        repository = Path(__file__).resolve().parents[1]
        config_path = repository / "model/intent_v1/config.json"
        config_value = load_training_config(config_path)
        english_path = repository / config_value.sources.english.path
        russian_path = repository / config_value.sources.russian.path
        empty_rows: dict[SplitName, tuple[LexicalExample, ...]] = {
            "train": (),
            "development": (),
            "calibration": (),
            "threshold": (),
            "test": (),
        }
        dataset = DatasetBundle(empty_rows, ())
        with tempfile.TemporaryDirectory() as temporary:
            (Path(temporary) / "model/intent_v1").mkdir(parents=True)
            verification_config_path = Path(temporary) / "config.json"
            verification_config_path.write_bytes(config_path.read_bytes())
            config_path = verification_config_path
            artifact = Path(temporary) / "model.ksm"
            artifact.write_bytes(b"synthetic-artifact")
            artifact_digest = hashlib.sha256(artifact.read_bytes()).hexdigest()
            embedded = {
                key: value
                for key, value in manifest_schema_fixture().items()
                if key not in {"artifact_sha256", "artifact_model_version"}
            }
            embedded.update({
                "config_sha256": hashlib.sha256(config_path.read_bytes()).hexdigest(),
                "dataset_sha256": dataset_fingerprint(dataset),
                "split_namespace": SPLIT_NAMESPACE,
                "source_package": {
                    "name": "onboard-data",
                    "version": "test",
                    "license_declaration": "GPL-3+",
                },
                "sources": [
                    {"group": 0, "sha256": sha256_file(english_path)},
                    {"group": 1, "sha256": sha256_file(russian_path)},
                ],
                "variant_quarantine_sha256": dataset.variant_quarantine.sha256,
                "sealed_variant_quarantine_sha256": (
                    dataset.sealed_variant_quarantine.sha256
                ),
                "sealed_test_exclusion_signatures_sha256": hashlib.sha256(
                    b""
                ).hexdigest(),
            })
            embedded["toolchain"] = asdict(
                capture_toolchain_snapshot(cast(str, embedded["config_sha256"]))
            )
            cast(dict[str, object], embedded["toolchain"])["detector_sha256"] = (
                sha256_file(repository / "src/keyswitch/detector.py")
            )
            cast(dict[str, object], embedded["toolchain"])[
                "protected_tokens_sha256"
            ] = sha256_file(
                repository / "src/keyswitch/resources/protected_tokens.txt"
            )
            training_language_scorer: dict[str, object] = {
                "kind": "train-only-character-ngram",
                "algorithm_version": INTENT_TRAIN_ONLY_SCORER_ALGORITHM_VERSION,
                "ngram_orders": list(INTENT_TRAIN_ONLY_SCORER_NGRAM_ORDERS),
                "score_mode": "character-ngram-only",
                "spellcheck_enabled": False,
                "word_counts_by_group": {"0": 1, "1": 1},
                "excluded_quarantined_identities": 0,
                "source_sha256": "3" * SHA256_HEX_CHARACTERS,
            }
            embedded["training_language_scorer"] = training_language_scorer
            hard_negative_provenance = (
                empty_hard_negative_corpus().provenance_payload()
            )
            embedded["training"] = {
                "hard_negative_development": hard_negative_provenance
            }
            embedded["gate_policy"] = gate_policy_payload(config_value)
            strong = self.STRONG_SYMMETRIC_METRICS
            per_trigger = {trigger: strong for trigger in TRIGGERS}
            context_metrics = {
                profile.name: per_trigger
                for profile in CONTEXT_STRESS_PROFILES
            }
            threshold_logits = {trigger: INTENT_EXTERNAL_FIXTURE_THRESHOLD_LOGIT for trigger in TRIGGERS}
            selections = {
                trigger: ThresholdSelection(
                    trigger,
                    threshold_logits[trigger],
                    strong,
                    strong,
                )
                for trigger in TRIGGERS
            }
            safety_audit = GuardedSafetyAudit(
                samples=len(TRIGGERS) * INTENT_SAFETY_SAMPLE_MULTIPLIER,
                protected_samples=len(TRIGGERS),
                lexical_collision_samples=len(TRIGGERS),
                triggers=TRIGGERS,
                protected_triggers=TRIGGERS,
                lexical_collision_triggers=TRIGGERS,
                failures=(),
            )
            veto = VetoSelection(-1.0, INTENT_VETO_SELECTION_SAMPLE_COUNT, 0, 0.0)
            embedded["thresholds"] = {
                trigger: {
                    "global_logit_margin": 0.0,
                    "logits": {
                        direction: threshold_logits[trigger]
                        for direction in ("0>1", "1>0")
                    },
                    "confidences": {
                        direction: stable_sigmoid(
                            threshold_logits[trigger]
                        )
                        for direction in ("0>1", "1>0")
                    },
                    "selection_metrics": metrics_payload(strong),
                    "selection_typo_metrics": metrics_payload(strong),
                }
                for trigger in TRIGGERS
            }
            embedded["threshold_selection_gate_breakdown"] = (
                threshold_selection_gate_breakdown(
                    config_value,
                    selections,
                    context_metrics,
                    context_metrics,
                )
            )
            embedded["veto"] = {
                "selection": asdict(veto),
                "sealed_test": asdict(veto),
            }
            embedded["quality_gate_breakdown"] = (
                training_quality_gate_breakdown(
                    config_value,
                    per_trigger,
                    per_trigger,
                    safety_audit,
                    veto,
                    veto,
                    context_metrics,
                    context_metrics,
                )
            )
            embedded["quality_gates_passed"] = True
            fixture_model_parameters: dict[str, object] = {
                "dimension": config_value.dimension,
                "payload_sha256": "1" * SHA256_HEX_CHARACTERS,
                "weight_scale_hex": (1.0).hex(),
                "bias_hex": (0.0).hex(),
                "platt_calibration_hex": {
                    direction: {
                        "scale": (1.0).hex(),
                        "bias": (0.0).hex(),
                    }
                    for direction in ("0>1", "1>0")
                },
                "threshold_logits_hex": {
                    trigger: {
                        direction: threshold_logits[trigger].hex()
                        for direction in ("0>1", "1>0")
                    }
                    for trigger in TRIGGERS
                },
                "veto_threshold_hex": veto.raw_logit.hex(),
                "feature_hash_seed": config_value.feature_hash_seed,
                "membership_hash_seed": config_value.membership_hash_seed,
                "ngram_orders": list(NGRAM_ORDERS),
            }
            fixture_candidate_metadata = (
                presealed_candidate_metadata_projection(
                    model_id="keyswitch-layout-intent-v1",
                    calibration_scope=cast(
                        str, embedded["calibration_scope"]
                    ),
                    config_sha256=cast(str, embedded["config_sha256"]),
                    split_namespace=SPLIT_NAMESPACE,
                    toolchain=cast(
                        dict[str, object], embedded["toolchain"]
                    ),
                    source_package=cast(
                        dict[str, object], embedded["source_package"]
                    ),
                    sources=cast(list[object], embedded["sources"]),
                    candidate_counts=presealed_candidate_counts(dataset),
                    variant_quarantine_sha256=(
                        dataset.variant_quarantine.sha256
                    ),
                    training_language_scorer=training_language_scorer,
                    gate_policy=cast(
                        dict[str, object], embedded["gate_policy"]
                    ),
                    training=cast(
                        dict[str, object], embedded["training"]
                    ),
                    quantization=cast(
                        dict[str, object], embedded["quantization"]
                    ),
                    calibration=cast(
                        dict[str, object], embedded["calibration"]
                    ),
                    veto_selection=asdict(veto),
                    thresholds=cast(
                        dict[str, object], embedded["thresholds"]
                    ),
                    selection_gate_breakdown=cast(
                        dict[str, object],
                        embedded["threshold_selection_gate_breakdown"],
                    ),
                    safety_guard_audit=asdict(
                        audit_guarded_safety_corpus(dataset.safety)
                    ),
                    model_parameters=fixture_model_parameters,
                )
            )
            fixture_candidate_sha256 = sealed_candidate_sha256(
                split_namespace=SPLIT_NAMESPACE,
                config_sha256=cast(str, embedded["config_sha256"]),
                candidate_dataset_sha256=cast(
                    str, embedded["dataset_sha256"]
                ),
                toolchain=cast(
                    dict[str, object], embedded["toolchain"]
                ),
                training_language_scorer=training_language_scorer,
                model_parameters=fixture_model_parameters,
                selection_gate_breakdown=cast(
                    dict[str, object],
                    embedded["threshold_selection_gate_breakdown"],
                ),
                candidate_metadata=fixture_candidate_metadata,
            )
            sealed_receipt = claim_sealed_evaluation(
                config=config_value,
                candidate_sha256=fixture_candidate_sha256,
                config_sha256=cast(str, embedded["config_sha256"]),
                candidate_dataset_sha256=cast(
                    str, embedded["dataset_sha256"]
                ),
                repository_root=Path(temporary),
            )
            embedded["sealed_evaluation"] = sealed_receipt.payload()
            build_provenance = {
                field_name: embedded[field_name]
                for field_name in (
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
            }
            build_provenance_sha256 = hashlib.sha256(
                json.dumps(
                    build_provenance,
                    ensure_ascii=False,
                    sort_keys=True,
                    separators=(",", ":"),
                ).encode("utf-8")
            ).hexdigest()
            embedded["build_provenance_sha256"] = build_provenance_sha256
            model_version = (
                "intent-v1-" + build_provenance_sha256[: VERSION_HASH_CHARACTERS]
            )
            manifest = {
                **embedded,
                "artifact_sha256": artifact_digest,
                "artifact_model_version": model_version,
            }
            common = {
                "metadata": embedded,
                "checksum": artifact_digest,
                "model_version": model_version,
                "dimension": config_value.dimension,
                "payload_sha256": "1" * SHA256_HEX_CHARACTERS,
                "weight_scale": 1.0,
                "bias": 0.0,
                "platt_calibration": {
                    direction: PlattParameters(1.0, 0.0)
                    for direction in ("0>1", "1>0")
                },
                "fnv_seed": config_value.feature_hash_seed,
                "threshold_logits": runtime_threshold_logits(
                    threshold_logits
                ),
                "thresholds": {
                    trigger: {
                        direction: stable_sigmoid(logit)
                        for direction in ("0>1", "1>0")
                    }
                    for trigger, logit in threshold_logits.items()
                },
                "veto_threshold": veto.raw_logit,
                "ngram_orders": NGRAM_ORDERS,
            }
            matching = cast(
                LinearNgramModel,
                SimpleNamespace(
                    **common,
                    membership_seed=config_value.membership_hash_seed,
                ),
            )
            checks = verify_provenance(
                model=matching,
                artifact=artifact,
                manifest=manifest,
                config_path=config_path,
                config=config_value,
                english_path=english_path,
                russian_path=russian_path,
                dataset=dataset,
                training_language_scorer=training_language_scorer,
                hard_negative_development=hard_negative_provenance,
                sealed_registry_root=Path(temporary),
            )
            by_name = {check.name: check for check in checks}
            self.assertTrue(by_name["membership_schema"].passed)
            for field_name in (
                "trainer_sha256",
                "runtime_sha256",
                "detector_sha256",
                "protected_tokens_sha256",
                "layouts_sha256",
                "language_model_sha256",
                "evaluator_sha256",
                "preseal_generator_sha256",
                "development_freezer_sha256",
                "preseal_receipt_sha256",
            ):
                self.assertTrue(by_name[f"toolchain_{field_name}"].passed)
            self.assertNotIn("toolchain_python_version", by_name)
            self.assertNotIn("toolchain_system", by_name)
            self.assertTrue(by_name["build_provenance_sha256"].passed)
            self.assertTrue(by_name["model_version"].passed)
            self.assertTrue(by_name["training_language_scorer"].passed)
            self.assertTrue(by_name["hard_negative_development"].passed)
            self.assertTrue(by_name["gate_policy"].passed)
            self.assertTrue(by_name["split_namespace"].passed)
            self.assertTrue(by_name["sealed_evaluation"].passed)
            self.assertTrue(
                by_name["threshold_selection_gate"].passed,
                by_name["threshold_selection_gate"].detail,
            )
            self.assertTrue(by_name["training_quality_gate"].passed)
            self.assertTrue(by_name["runtime_decision_parameters"].passed)

            hard_negative_checks = verify_provenance(
                model=matching,
                artifact=artifact,
                manifest=manifest,
                config_path=config_path,
                config=config_value,
                english_path=english_path,
                russian_path=russian_path,
                dataset=dataset,
                training_language_scorer=training_language_scorer,
                hard_negative_development={
                    **hard_negative_provenance,
                    "signature_count": 1,
                },
                sealed_registry_root=Path(temporary),
            )
            self.assertFalse(
                {
                    check.name: check for check in hard_negative_checks
                }["hard_negative_development"].passed
            )

            tampered_thresholds = dict(threshold_logits)
            tampered_thresholds["space"] += INTENT_EXTERNAL_THRESHOLD_TAMPER_DELTA
            tampered_model = cast(
                LinearNgramModel,
                SimpleNamespace(
                    **{
                        **common,
                        "threshold_logits": tampered_thresholds,
                    },
                    membership_seed=config_value.membership_hash_seed,
                ),
            )
            tampered_checks = verify_provenance(
                model=tampered_model,
                artifact=artifact,
                manifest=manifest,
                config_path=config_path,
                config=config_value,
                english_path=english_path,
                russian_path=russian_path,
                dataset=dataset,
                training_language_scorer=training_language_scorer,
                sealed_registry_root=Path(temporary),
            )
            self.assertFalse(
                {
                    check.name: check for check in tampered_checks
                }["runtime_decision_parameters"].passed
            )

            tampered_payload_model = cast(
                LinearNgramModel,
                SimpleNamespace(
                    **{**common, "payload_sha256": "2" * SHA256_HEX_CHARACTERS},
                    membership_seed=config_value.membership_hash_seed,
                ),
            )
            payload_checks = verify_provenance(
                model=tampered_payload_model,
                artifact=artifact,
                manifest=manifest,
                config_path=config_path,
                config=config_value,
                english_path=english_path,
                russian_path=russian_path,
                dataset=dataset,
                training_language_scorer=training_language_scorer,
                sealed_registry_root=Path(temporary),
            )
            payload_by_name = {
                check.name: check for check in payload_checks
            }
            self.assertTrue(
                payload_by_name["runtime_decision_parameters"].passed
            )
            self.assertFalse(payload_by_name["sealed_evaluation"].passed)
            self.assertFalse(
                payload_by_name["sealed_candidate_sha256"].passed
            )

            tampered_veto_model = cast(
                LinearNgramModel,
                SimpleNamespace(
                    **{**common, "veto_threshold": veto.raw_logit - INTENT_EXTERNAL_VETO_TAMPER_DELTA},
                    membership_seed=config_value.membership_hash_seed,
                ),
            )
            veto_checks = verify_provenance(
                model=tampered_veto_model,
                artifact=artifact,
                manifest=manifest,
                config_path=config_path,
                config=config_value,
                english_path=english_path,
                russian_path=russian_path,
                dataset=dataset,
                training_language_scorer=training_language_scorer,
                sealed_registry_root=Path(temporary),
            )
            self.assertFalse(
                {check.name: check for check in veto_checks}[
                    "runtime_decision_parameters"
                ].passed
            )

            def checks_for_gate_evidence(
                changes: dict[str, object],
            ) -> dict[str, VerificationCheck]:
                changed_embedded = {**embedded, **changes}
                changed_manifest = {
                    **changed_embedded,
                    "artifact_sha256": artifact_digest,
                    "artifact_model_version": model_version,
                }
                changed_model = cast(
                    LinearNgramModel,
                    SimpleNamespace(
                        **{**common, "metadata": changed_embedded},
                        membership_seed=config_value.membership_hash_seed,
                    ),
                )
                return {
                    check.name: check
                    for check in verify_provenance(
                        model=changed_model,
                        artifact=artifact,
                        manifest=changed_manifest,
                        config_path=config_path,
                        config=config_value,
                        english_path=english_path,
                        russian_path=russian_path,
                        dataset=dataset,
                        training_language_scorer=training_language_scorer,
                        sealed_registry_root=Path(temporary),
                    )
                }

            for invalid_threshold_evidence in (
                {"passed": False},
                {"passed": 1},
                {"passed": True},
                {"passed": True, "context_stress": {}},
                {
                    "passed": True,
                    "per_trigger": {"space": {"passed": False}},
                },
                {
                    "passed": True,
                    "per_trigger": {
                        "space": {
                            "passed": True,
                            "checks": {"recall": False},
                        }
                    },
                },
            ):
                invalid_checks = checks_for_gate_evidence(
                    {
                        "threshold_selection_gate_breakdown": (
                            invalid_threshold_evidence
                        )
                    }
                )
                self.assertFalse(
                    invalid_checks["threshold_selection_gate"].passed
                )

            for invalid_quality_evidence in (
                {"passed": False},
                {"passed": 1},
                {"passed": True},
                {"passed": True, "sealed_test_context_stress": {}},
                {"passed": True, "veto": {"passed": False}},
            ):
                invalid_checks = checks_for_gate_evidence(
                    {"quality_gate_breakdown": invalid_quality_evidence}
                )
                self.assertFalse(invalid_checks["training_quality_gate"].passed)
            invalid_checks = checks_for_gate_evidence(
                {"quality_gates_passed": False}
            )
            self.assertFalse(invalid_checks["training_quality_gate"].passed)
            with self.assertRaisesRegex(ValueError, "must be a boolean"):
                checks_for_gate_evidence({"quality_gates_passed": 1})

            for metadata_field, tampered_value in (
                ("training", {"bias": INTENT_TAMPERED_METADATA_VALUE}),
                ("quantization", {"format": "tampered"}),
                ("calibration", {"slope": INTENT_TAMPERED_METADATA_VALUE}),
            ):
                with self.subTest(sealed_metadata=metadata_field):
                    metadata_checks = checks_for_gate_evidence(
                        {metadata_field: tampered_value}
                    )
                    self.assertFalse(
                        metadata_checks["sealed_evaluation"].passed
                    )
                    self.assertFalse(
                        metadata_checks[
                            "presealed_candidate_metadata"
                        ].passed
                    )

            missing_gate_embedded = dict(embedded)
            del missing_gate_embedded["threshold_selection_gate_breakdown"]
            missing_gate_manifest = {
                **missing_gate_embedded,
                "artifact_sha256": artifact_digest,
                "artifact_model_version": model_version,
            }
            missing_gate_model = cast(
                LinearNgramModel,
                SimpleNamespace(
                    **{**common, "metadata": missing_gate_embedded},
                    membership_seed=config_value.membership_hash_seed,
                ),
            )
            with self.assertRaisesRegex(ValueError, "manifest keys mismatch"):
                verify_provenance(
                    model=missing_gate_model,
                    artifact=artifact,
                    manifest=missing_gate_manifest,
                    config_path=config_path,
                    config=config_value,
                    english_path=english_path,
                    russian_path=russian_path,
                    dataset=dataset,
                    training_language_scorer=training_language_scorer,
                    sealed_registry_root=Path(temporary),
                )

            mismatching_scorer = {
                **training_language_scorer,
                "source_sha256": "4" * SHA256_HEX_CHARACTERS,
            }
            scorer_checks = verify_provenance(
                model=matching,
                artifact=artifact,
                manifest=manifest,
                config_path=config_path,
                config=config_value,
                english_path=english_path,
                russian_path=russian_path,
                dataset=dataset,
                training_language_scorer=mismatching_scorer,
                sealed_registry_root=Path(temporary),
            )
            scorer_by_name = {check.name: check for check in scorer_checks}
            self.assertTrue(scorer_by_name["embedded_manifest"].passed)
            self.assertTrue(scorer_by_name["build_provenance_sha256"].passed)
            self.assertFalse(scorer_by_name["training_language_scorer"].passed)
            mismatching = cast(
                LinearNgramModel,
                SimpleNamespace(
                    **common,
                    membership_seed=config_value.membership_hash_seed + 1,
                ),
            )
            failed_checks = verify_provenance(
                model=mismatching,
                artifact=artifact,
                manifest=manifest,
                config_path=config_path,
                config=config_value,
                english_path=english_path,
                russian_path=russian_path,
                dataset=dataset,
                training_language_scorer=training_language_scorer,
                sealed_registry_root=Path(temporary),
            )
            failed_by_name = {check.name: check for check in failed_checks}
            self.assertFalse(failed_by_name["membership_schema"].passed)

            stale_toolchain = {
                **cast(dict[str, object], embedded["toolchain"]),
                "trainer_sha256": "0" * SHA256_HEX_CHARACTERS,
            }
            stale_embedded = {**embedded, "toolchain": stale_toolchain}
            stale_manifest = {
                **stale_embedded,
                "artifact_sha256": artifact_digest,
                "artifact_model_version": model_version,
            }
            stale_model = cast(
                LinearNgramModel,
                SimpleNamespace(
                    **{**common, "metadata": stale_embedded},
                    membership_seed=config_value.membership_hash_seed,
                ),
            )
            stale_checks = verify_provenance(
                model=stale_model,
                artifact=artifact,
                manifest=stale_manifest,
                config_path=config_path,
                config=config_value,
                english_path=english_path,
                russian_path=russian_path,
                dataset=dataset,
                training_language_scorer=training_language_scorer,
                sealed_registry_root=Path(temporary),
            )
            stale_by_name = {check.name: check for check in stale_checks}
            self.assertTrue(stale_by_name["embedded_manifest"].passed)
            self.assertFalse(stale_by_name["toolchain_trainer_sha256"].passed)
            self.assertTrue(stale_by_name["toolchain_runtime_sha256"].passed)
            self.assertTrue(stale_by_name["toolchain_layouts_sha256"].passed)
            self.assertTrue(
                stale_by_name["toolchain_language_model_sha256"].passed
            )

            tampered_embedded = {
                **embedded,
                "source_package": {
                    "name": "onboard-data",
                    "version": "tampered",
                    "license_declaration": "GPL-3+",
                },
            }
            tampered_manifest = {
                **tampered_embedded,
                "artifact_sha256": artifact_digest,
                "artifact_model_version": model_version,
            }
            tampered_model = cast(
                LinearNgramModel,
                SimpleNamespace(
                    **{**common, "metadata": tampered_embedded},
                    membership_seed=config_value.membership_hash_seed,
                ),
            )
            tampered_checks = verify_provenance(
                model=tampered_model,
                artifact=artifact,
                manifest=tampered_manifest,
                config_path=config_path,
                config=config_value,
                english_path=english_path,
                russian_path=russian_path,
                dataset=dataset,
                training_language_scorer=training_language_scorer,
                sealed_registry_root=Path(temporary),
            )
            tampered_by_name = {check.name: check for check in tampered_checks}
            self.assertTrue(tampered_by_name["embedded_manifest"].passed)
            self.assertFalse(tampered_by_name["build_provenance_sha256"].passed)
            self.assertFalse(tampered_by_name["model_version"].passed)
            self.assertFalse(tampered_by_name["sealed_evaluation"].passed)
            self.assertFalse(
                tampered_by_name["presealed_candidate_metadata"].passed
            )

            malformed_toolchain = {**stale_toolchain, "trainer_sha256": "ABC"}
            malformed_manifest = {
                **manifest,
                "toolchain": malformed_toolchain,
            }
            with self.assertRaisesRegex(
                ValueError,
                r"manifest\.toolchain\.trainer_sha256 must be an exact lowercase",
            ):
                verify_provenance(
                    model=matching,
                    artifact=artifact,
                    manifest=malformed_manifest,
                    config_path=config_path,
                    config=config_value,
                    english_path=english_path,
                    russian_path=russian_path,
                    dataset=dataset,
                    training_language_scorer=training_language_scorer,
                    sealed_registry_root=Path(temporary),
                )

    def test_hunspell_negatives_are_hard_guarded_but_unknown_typos_reach_model(self) -> None:
        snapshots = self.dictionary_snapshots()
        with patch(
            "evaluate_intent_model.LanguageModel.load",
            side_effect=self.language_model,
        ):
            hard_guard = build_lexical_disjoint_corpus(
                {0: set(), 1: set()},
                minimum_words_per_group=1,
                hunspell_snapshots=snapshots,
            )
            hard_guard_spy = SpyIntentModel()
            hard_guard_result = compare_with_fallback(
                cast(LinearNgramModel, hard_guard_spy),
                hard_guard.examples,
                0,
            )
            self.assertEqual(hard_guard_result.samples, INTENT_EXTERNAL_EXPECTED_HARD_GUARD_SAMPLES)
            self.assertEqual(
                hard_guard_result.model_evaluated_samples, INTENT_EXTERNAL_EXPECTED_HARD_GUARD_MODEL_EVALUATED
            )
            self.assertEqual(hard_guard_result.negative_model_evaluated, 0)
            self.assertEqual(len(hard_guard_spy.inputs), INTENT_EXTERNAL_EXPECTED_HARD_GUARD_MODEL_EVALUATED)

            unknown_first = build_unknown_typo_disjoint_corpus(
                {0: set(), 1: set()},
                sealed_physical_signatures=frozenset(),
                minimum_words_per_group=1,
                hunspell_snapshots=snapshots,
            )
            unknown_second = build_unknown_typo_disjoint_corpus(
                {0: set(), 1: set()},
                sealed_physical_signatures=frozenset(),
                minimum_words_per_group=1,
                hunspell_snapshots=snapshots,
            )
            self.assertEqual(unknown_first, unknown_second)
            self.assertEqual(
                unknown_first.corpus_sha256,
                external_corpus_sha256(unknown_first.examples),
            )
            self.assertEqual(unknown_first.words_by_group, {0: 1, 1: 1})
            self.assertEqual(
                len(unknown_first.examples), INTENT_EXTERNAL_UNKNOWN_TYPO_EXAMPLES_PER_TRIGGER * len(TRIGGERS)
            )
            self.assertEqual(
                {item.label for item in unknown_first.examples},
                {False, True},
            )
            self.assertEqual(
                {item.trigger for item in unknown_first.examples},
                set(TRIGGERS),
            )
            self.assertTrue(
                all(
                    item.variant_kind.startswith("hunspell-unknown-")
                    for item in unknown_first.examples
                )
            )

            unknown_spy = SpyIntentModel()
            unknown_result = compare_with_fallback(
                cast(LinearNgramModel, unknown_spy),
                unknown_first.examples,
                0,
            )
            self.assertEqual(
                unknown_result.model_evaluated_samples,
                INTENT_EXTERNAL_UNKNOWN_TYPO_EXAMPLES_PER_TRIGGER * len(TRIGGERS),
            )
            self.assertEqual(
                unknown_result.negative_model_evaluated,
                INTENT_EXTERNAL_UNKNOWN_TYPO_NEGATIVE_EXAMPLES_PER_TRIGGER * len(TRIGGERS),
            )
            self.assertEqual(
                len(unknown_spy.inputs), INTENT_EXTERNAL_UNKNOWN_TYPO_EXAMPLES_PER_TRIGGER * len(TRIGGERS)
            )
            raw_metrics = prediction_metrics(
                predict_model_examples(
                    cast(LinearNgramModel, unknown_spy),
                    unknown_first.examples,
                    scorers={0: self.english, 1: self.russian},
                )
            )
            for metrics in raw_metrics.values():
                self.assertEqual(metrics.false_positive, INTENT_EXTERNAL_EXPECTED_RAW_METRICS_COUNT)
                self.assertEqual(metrics.true_positive, INTENT_EXTERNAL_EXPECTED_RAW_METRICS_COUNT)

    def test_unknown_typo_corpus_excludes_runtime_inapplicable_deletions(
        self,
    ) -> None:
        english = EvaluationLanguageModel("en_US", {"short"})
        russian = EvaluationLanguageModel("ru_RU", {"слово"})
        previous = (self.english, self.russian)
        self.english, self.russian = english, russian
        try:
            snapshots = self.dictionary_snapshots()
        finally:
            self.english, self.russian = previous

        corpus = build_unknown_typo_disjoint_corpus(
            {0: set(), 1: set()},
            sealed_physical_signatures=frozenset(),
            minimum_words_per_group=1,
            hunspell_snapshots=snapshots,
            language_models={
                0: cast(LanguageModel, english),
                1: cast(LanguageModel, russian),
            },
        )

        self.assertEqual(corpus.words_by_group, {0: 1, 1: 1})
        self.assertTrue(
            all(
                max(
                    len(LanguageModel.normalize(item.original)),
                    len(LanguageModel.normalize(item.alternative)),
                )
                >= INTENT_EXTERNAL_MINIMUM_APPLICABLE_DELETION_LENGTH
                for item in corpus.examples
            )
        )
        english_rows = tuple(
            item
            for item in corpus.examples
            if item.source_group == 0 and not item.label
        )
        self.assertTrue(english_rows)
        self.assertEqual(
            {item.variant_kind for item in english_rows},
            {"hunspell-unknown-duplication"},
        )

    def test_unknown_typo_holdout_has_a_distinct_domain_and_namespaces(
        self,
    ) -> None:
        snapshots = self.dictionary_snapshots()
        models = {
            0: cast(LanguageModel, self.english),
            1: cast(LanguageModel, self.russian),
        }
        development = build_unknown_typo_disjoint_corpus(
            {0: set(), 1: set()},
            sealed_physical_signatures=frozenset(),
            minimum_words_per_group=1,
            hunspell_snapshots=snapshots,
            language_models=models,
            rank_namespace=UNKNOWN_TYPO_DEVELOPMENT_RANK_NAMESPACE,
            choice_namespace=UNKNOWN_TYPO_DEVELOPMENT_CHOICE_NAMESPACE,
        )
        development_signatures = unknown_typo_physical_signatures(
            development
        )
        holdout = build_unknown_typo_disjoint_corpus(
            {0: set(), 1: set()},
            sealed_physical_signatures=development_signatures,
            minimum_words_per_group=1,
            hunspell_snapshots=snapshots,
            language_models=models,
            rank_namespace=UNKNOWN_TYPO_HOLDOUT_RANK_NAMESPACE,
            choice_namespace=UNKNOWN_TYPO_HOLDOUT_CHOICE_NAMESPACE,
        )
        repeated = build_unknown_typo_disjoint_corpus(
            {0: set(), 1: set()},
            sealed_physical_signatures=development_signatures,
            minimum_words_per_group=1,
            hunspell_snapshots=snapshots,
            language_models=models,
            rank_namespace=UNKNOWN_TYPO_HOLDOUT_RANK_NAMESPACE,
            choice_namespace=UNKNOWN_TYPO_HOLDOUT_CHOICE_NAMESPACE,
        )
        holdout_signatures = unknown_typo_physical_signatures(holdout)
        self.assertEqual(holdout, repeated)
        self.assertNotEqual(development.corpus_sha256, holdout.corpus_sha256)
        self.assertTrue(
            development_signatures.isdisjoint(holdout_signatures)
        )
        self.assertEqual(
            holdout.exclusion_signature_sha256,
            physical_signature_set_sha256(development_signatures),
        )
        self.assertEqual(
            holdout.rank_namespace,
            UNKNOWN_TYPO_HOLDOUT_RANK_NAMESPACE,
        )
        self.assertEqual(
            holdout.choice_namespace,
            UNKNOWN_TYPO_HOLDOUT_CHOICE_NAMESPACE,
        )
        for rank_namespace, choice_namespace in (
            ("", "valid"),
            ("valid", "valid"),
            ("bad\0namespace", "valid"),
            ("не-ascii", "valid"),
        ):
            with self.subTest(
                rank_namespace=rank_namespace,
                choice_namespace=choice_namespace,
            ):
                with self.assertRaisesRegex(ValueError, "namespace"):
                    build_unknown_typo_disjoint_corpus(
                        {0: set(), 1: set()},
                        sealed_physical_signatures=frozenset(),
                        minimum_words_per_group=1,
                        hunspell_snapshots=snapshots,
                        language_models=models,
                        rank_namespace=rank_namespace,
                        choice_namespace=choice_namespace,
                    )

    def test_safety_policy_uses_real_guards_while_raw_model_is_diagnostic(self) -> None:
        pair = LayoutPair()
        collision_alternative = pair.translate("test", "us", "ru")
        protected_token = "--force-with-lease"
        protected_alternative = pair.translate(protected_token, "us", "ru")
        collision = LexicalExample(
            original="test",
            alternative=collision_alternative,
            source_group=0,
            target_group=1,
            trigger="space",
            label=False,
            weight=1.0,
            base_signature="test",
            variant_kind="lexical_collision",
            source_known=True,
            target_known=True,
            safety=True,
        )
        protected = LexicalExample(
            original=protected_token,
            alternative=protected_alternative,
            source_group=0,
            target_group=1,
            trigger="space",
            label=False,
            weight=1.0,
            base_signature="hard:--force-with-lease",
            variant_kind="protected",
            source_known=False,
            target_known=False,
            protected=True,
            safety=True,
        )
        english = EvaluationLanguageModel("en_US", {"test"})
        russian = EvaluationLanguageModel("ru_RU", {collision_alternative})
        model = SpyIntentModel()
        with patch(
            "evaluate_intent_model.LanguageModel.load",
            side_effect=lambda locale: english if locale == "en_US" else russian,
        ):
            policy = evaluate_safety_policy(
                cast(LinearNgramModel, model),
                (collision, protected),
            )
            with self.assertRaisesRegex(ValueError, "labelled-negative safety"):
                evaluate_safety_policy(
                    cast(LinearNgramModel, model),
                    (replace(protected, label=True),),
                )
        self.assertTrue(LanguageDetector.is_protected_token(protected_token))
        self.assertEqual(policy.samples, INTENT_EXTERNAL_EXPECTED_SAFETY_POLICY_SAMPLES)
        self.assertEqual(policy.protected_samples, 1)
        self.assertEqual(policy.lexical_collision_samples, 1)
        self.assertEqual(policy.pre_model_guarded_samples, INTENT_EXTERNAL_EXPECTED_SAFETY_POLICY_SAMPLES)
        self.assertEqual(
            policy.expected_pre_model_guard_samples, INTENT_EXTERNAL_EXPECTED_SAFETY_POLICY_SAMPLES
        )
        self.assertEqual(
            policy.expected_pre_model_guarded_samples, INTENT_EXTERNAL_EXPECTED_SAFETY_POLICY_SAMPLES
        )
        self.assertEqual(policy.model_evaluated_samples, 0)
        self.assertEqual(policy.guard_failure_samples, 0)
        self.assertEqual(policy.per_trigger["space"].false_positive, 0)
        self.assertEqual(len(model.inputs), 0)

        raw_metrics = model_metrics(
            cast(LinearNgramModel, model),
            (collision, protected),
            scorers={0: english, 1: russian},
        )
        self.assertEqual(raw_metrics["space"].false_positive, INTENT_EXTERNAL_EXPECTED_SAFETY_POLICY_SAMPLES)
        self.assertEqual(len(model.inputs), INTENT_EXTERNAL_EXPECTED_SAFETY_POLICY_SAMPLES)

    def test_production_context_profiles_are_exact_and_cache_is_policy_invariant(
        self,
    ) -> None:
        expected_hex = {
            "neutral": "0x0.0p+0",
            "none_min": "-0x1.c000000000000p+0",
            "none_max": "0x1.c000000000000p+0",
            "source_min": "-0x1.0666666666666p+1",
            "source_max": "0x1.7333333333333p+0",
            "target_min": "-0x1.3333333333333p+0",
            "target_max": "0x1.2666666666666p+1",
        }
        self.assertEqual(CONTEXT_SCORE_MINIMUM, 0.0)
        self.assertEqual(CONTEXT_SCORE_MAXIMUM, 1.0)
        self.assertEqual(CONTEXT_DELTA_MULTIPLIER, EXPECTED_INTENT_CONTEXT_DELTA_MULTIPLIER)
        self.assertEqual(CONTEXT_TARGET_GROUP_BONUS, EXPECTED_INTENT_CONTEXT_TARGET_GROUP_BONUS)
        self.assertEqual(CONTEXT_SOURCE_GROUP_PENALTY, EXPECTED_INTENT_CONTEXT_SOURCE_GROUP_PENALTY)
        self.assertEqual(
            {profile.name: profile.expected_delta.hex() for profile in PRODUCTION_CONTEXT_PROFILES},
            expected_hex,
        )

        pair = LayoutPair()
        sealed = tuple(
            lexical_example(
                label,
                trigger=trigger,
                signature=f"sealed-{trigger}-{label}",
            )
            for trigger in TRIGGERS
            for label in (False, True)
        )
        unknown = tuple(
            replace(row, base_signature="unknown:" + row.base_signature)
            for row in sealed
        )
        safety = (
            replace(
                lexical_example(False),
                original="https://example.invalid",
                alternative="реезыЖ..",
                base_signature="hard:https",
                variant_kind="protected",
                protected=True,
                safety=True,
            ),
        )
        source_known = (
            replace(
                lexical_example(False),
                original="keyboard",
                alternative=pair.translate("keyboard", "us", "ru"),
                base_signature="source-known-en",
            ),
            LexicalExample(
                original="привет",
                alternative=pair.translate("привет", "ru", "us"),
                source_group=1,
                target_group=0,
                trigger="space",
                label=False,
                weight=1.0,
                base_signature="source-known-ru",
                variant_kind="hunspell-disjoint",
                source_known=False,
                target_known=False,
            ),
        )
        language_models = cast(
            dict[int, LanguageModel],
            {0: self.english, 1: self.russian},
        )
        cached_model = SpyIntentModel()
        cached = evaluate_production_context_ensemble(
            cast(LinearNgramModel, cached_model),
            sealed_test=sealed,
            unknown_typo=unknown,
            safety=safety,
            source_known=source_known,
            language_models=language_models,
        )
        uncached_model = SpyIntentModel()
        uncached = evaluate_production_context_ensemble(
            cast(LinearNgramModel, uncached_model),
            sealed_test=sealed,
            unknown_typo=unknown,
            safety=safety,
            source_known=source_known,
            language_models=language_models,
            use_prediction_cache=False,
        )
        self.assertEqual(cached.schema_version, 1)
        self.assertEqual(cached.profiles, uncached.profiles)
        self.assertGreater(cached.unique_model_predictions, 0)
        self.assertGreater(cached.model_prediction_cache_hits, 0)
        self.assertEqual(uncached.unique_model_predictions, 0)
        self.assertEqual(uncached.model_prediction_cache_hits, 0)
        self.assertLess(len(cached_model.inputs), len(uncached_model.inputs))
        for name, result in cached.profiles.items():
            self.assertEqual(
                result.observed_deltas_by_direction,
                {
                    "0_to_1": (float.fromhex(expected_hex[name]),),
                    "1_to_0": (float.fromhex(expected_hex[name]),),
                },
            )
            self.assertEqual(
                set(result.corpora),
                {
                    "sealed_test",
                    "sealed_test_typos",
                    "unknown_typo",
                    "safety",
                    "source_known",
                },
            )

        fixed = eim._FixedContextLanguageScorer(
            self.english,
            next(
                profile
                for profile in PRODUCTION_CONTEXT_PROFILES
                if profile.name == "target_max"
            ),
        )
        self.assertEqual(fixed.score("keyboard"), self.english.score("keyboard"))
        self.assertEqual(
            fixed.best_single_deletion("keyboard"),
            self.english.best_single_deletion("keyboard"),
        )
        self.assertEqual(
            fixed.context_score(eim._SOURCE_CONTEXT_SENTINEL, "keyboard"),
            0.0,
        )
        self.assertEqual(
            fixed.context_score(eim._TARGET_CONTEXT_SENTINEL, "keyboard"),
            1.0,
        )
        with self.assertRaisesRegex(ValueError, "unknown sentinel"):
            fixed.context_score("", "keyboard")

    def test_source_known_corpus_uses_live_runtime_answers(self) -> None:
        scorers = test_word_scorers()
        candidates = (
            replace(
                lexical_example(False, signature="known-en"),
                original="keyboard",
                source_group=0,
                target_group=1,
            ),
            replace(
                lexical_example(False, signature="unknown-en"),
                original="aalborg",
                source_group=0,
                target_group=1,
            ),
            replace(
                lexical_example(False, signature="known-ru"),
                original="привет",
                source_group=1,
                target_group=0,
            ),
            replace(
                lexical_example(True, signature="positive-ignored"),
                original="keyboard",
                source_group=0,
                target_group=1,
            ),
        )

        selected = select_source_known_negative_examples(
            candidates,
            language_models=scorers,
        )

        self.assertEqual(
            tuple(example.original for example in selected),
            ("keyboard", "привет"),
        )
        self.assertTrue(all(example.source_known for example in selected))
        self.assertTrue(all(not example.label for example in selected))
        with self.assertRaisesRegex(ValueError, "groups 0 and 1"):
            select_source_known_negative_examples(
                candidates,
                language_models={0: scorers[0]},
            )
        with self.assertRaisesRegex(ValueError, "negative rows"):
            select_source_known_negative_examples(
                (candidates[-1],),
                language_models=scorers,
            )
        invalid_group = replace(candidates[0], source_group=UNSUPPORTED_LAYOUT_GROUP)
        with self.assertRaisesRegex(ValueError, "invalid source group"):
            select_source_known_negative_examples(
                (invalid_group, candidates[INTENT_EXTERNAL_THIRD_CANDIDATE_INDEX]),
                language_models=scorers,
            )
        with self.assertRaisesRegex(ValueError, "cover groups 0 and 1"):
            select_source_known_negative_examples(
                (candidates[0], candidates[1]),
                language_models=scorers,
            )

    def test_production_context_gate_is_exact_asymmetric_and_fail_closed(self) -> None:
        def policy_rows(
            *,
            triggers: Sequence[CorrectionTrigger],
            both_labels: bool,
            model_evaluated: bool,
            safety: bool = False,
        ) -> tuple[ProductionPolicyRow, ...]:
            result: list[ProductionPolicyRow] = []
            for trigger in triggers:
                labels = (False, True) if both_labels else (False,)
                for label in labels:
                    example = replace(
                        lexical_example(
                            label,
                            trigger=trigger,
                            signature=f"{trigger}-{label}",
                        ),
                        safety=safety,
                    )
                    result.append(
                        ProductionPolicyRow(
                            example,
                            fallback=label,
                            ensemble=label,
                            model_evaluated=model_evaluated,
                            observed_context_delta=0.0,
                        )
                    )
            return tuple(result)

        base_rows = {
            "sealed_test": policy_rows(
                triggers=TRIGGERS,
                both_labels=True,
                model_evaluated=True,
            ),
            "sealed_test_typos": policy_rows(
                triggers=TRIGGERS,
                both_labels=True,
                model_evaluated=True,
            ),
            "unknown_typo": policy_rows(
                triggers=TRIGGERS,
                both_labels=True,
                model_evaluated=True,
            ),
            "safety": policy_rows(
                triggers=TRIGGERS,
                both_labels=False,
                model_evaluated=False,
                safety=True,
            ),
            "source_known": policy_rows(
                triggers=("space",),
                both_labels=False,
                model_evaluated=False,
            ),
        }

        def build_evaluation() -> ProductionContextEnsembleEvaluation:
            return ProductionContextEnsembleEvaluation(
                schema_version=1,
                profiles={
                    profile.name: ProductionContextProfileEvaluation(
                        profile=profile,
                        observed_deltas_by_direction={
                            "0_to_1": (profile.expected_delta,),
                            "1_to_0": (profile.expected_delta,),
                        },
                        corpora={
                            name: eim._production_context_corpus_evaluation(
                                tuple(
                                    replace(
                                        row,
                                        observed_context_delta=profile.expected_delta,
                                    )
                                    for row in rows
                                )
                            )
                            for name, rows in base_rows.items()
                        },
                    )
                    for profile in PRODUCTION_CONTEXT_PROFILES
                },
                unique_model_predictions=1,
                model_prediction_cache_hits=1,
            )

        permissive = config(
            threshold_max_false_positive_rate=1.0,
            pause_threshold_max_false_positive_rate=1.0,
            test_minimum_precision=0.0,
            test_minimum_specificity=0.0,
        )
        baseline = build_evaluation()
        gate = production_context_ensemble_gate_breakdown(
            permissive, baseline
        )
        self.assertIs(gate["passed"], True)
        self.assertEqual(
            set(cast(dict[str, object], gate["profiles"])),
            {profile.name for profile in PRODUCTION_CONTEXT_PROFILES},
        )

        missing_profiles = dict(baseline.profiles)
        missing_profiles.pop("target_max")
        missing = production_context_ensemble_gate_breakdown(
            permissive,
            replace(baseline, profiles=missing_profiles),
        )
        self.assertIs(missing["passed"], False)
        self.assertIs(missing["all_profiles_present"], False)
        extra = production_context_ensemble_gate_breakdown(
            permissive,
            replace(
                baseline,
                profiles={**baseline.profiles, "unexpected": baseline.profiles["neutral"]},
            ),
        )
        self.assertIs(extra["passed"], False)

        missing_trigger_profiles = dict(baseline.profiles)
        target_max = missing_trigger_profiles["target_max"]
        target_corpora = dict(target_max.corpora)
        target_sealed = target_corpora["sealed_test"]
        target_corpora["sealed_test"] = replace(
            target_sealed,
            per_trigger={
                trigger: value
                for trigger, value in target_sealed.per_trigger.items()
                if trigger != "pause"
            },
        )
        missing_trigger_profiles["target_max"] = replace(
            target_max, corpora=target_corpora
        )
        missing_trigger = production_context_ensemble_gate_breakdown(
            permissive,
            replace(baseline, profiles=missing_trigger_profiles),
        )
        self.assertIs(missing_trigger["passed"], False)

        missing_label_profiles: dict[
            str, ProductionContextProfileEvaluation
        ] = {}
        for name, result in baseline.profiles.items():
            corpora = dict(result.corpora)
            corpora["sealed_test_typos"] = (
                eim._production_context_corpus_evaluation(
                    tuple(
                        replace(
                            row,
                            observed_context_delta=result.profile.expected_delta,
                        )
                        for row in base_rows["sealed_test_typos"]
                        if not row.example.label
                    )
                )
            )
            missing_label_profiles[name] = replace(result, corpora=corpora)
        missing_label = production_context_ensemble_gate_breakdown(
            permissive,
            replace(baseline, profiles=missing_label_profiles),
        )
        self.assertIs(missing_label["passed"], False)
        missing_label_gates = cast(
            dict[str, object], missing_label["profiles"]
        )
        neutral_missing_label = cast(
            dict[str, object], missing_label_gates["neutral"]
        )
        missing_label_corpora = cast(
            dict[str, object], neutral_missing_label["corpora"]
        )
        missing_label_typos = cast(
            dict[str, object], missing_label_corpora["sealed_test_typos"]
        )
        missing_label_triggers = cast(
            dict[str, object], missing_label_typos["per_trigger"]
        )
        missing_label_space = cast(
            dict[str, object], missing_label_triggers["space"]
        )
        missing_label_checks = cast(
            dict[str, bool], missing_label_space["checks"]
        )
        self.assertIs(missing_label_checks["label_support"], False)

        wrong_extrema_profiles = dict(baseline.profiles)
        wrong_extrema_profiles["target_max"] = replace(
            wrong_extrema_profiles["target_max"],
            observed_deltas_by_direction={
                "0_to_1": (0.0,),
                "1_to_0": (0.0,),
            },
        )
        wrong_extrema = production_context_ensemble_gate_breakdown(
            permissive,
            replace(baseline, profiles=wrong_extrema_profiles),
        )
        self.assertIs(wrong_extrema["passed"], False)

        weak_wilson = production_context_ensemble_gate_breakdown(
            config(
                threshold_max_false_positive_rate=INTENT_STRICT_FALSE_POSITIVE_RATE,
                pause_threshold_max_false_positive_rate=INTENT_STRICT_FALSE_POSITIVE_RATE,
                test_minimum_precision=0.0,
                test_minimum_specificity=0.0,
            ),
            baseline,
        )
        self.assertIs(weak_wilson["passed"], False)
        weak_profiles = cast(dict[str, object], weak_wilson["profiles"])
        weak_neutral = cast(dict[str, object], weak_profiles["neutral"])
        weak_corpora = cast(dict[str, object], weak_neutral["corpora"])
        weak_sealed = cast(dict[str, object], weak_corpora["sealed_test"])
        weak_triggers = cast(dict[str, object], weak_sealed["per_trigger"])
        weak_space = cast(dict[str, object], weak_triggers["space"])
        weak_checks = cast(dict[str, bool], weak_space["checks"])
        self.assertIs(weak_checks["false_positive_policy"], False)

        false_positive_profiles = dict(baseline.profiles)
        fp_profile = false_positive_profiles["target_max"]
        fp_corpora = dict(fp_profile.corpora)
        fp_unknown_rows = [
            replace(
                row,
                observed_context_delta=fp_profile.profile.expected_delta,
            )
            for row in base_rows["unknown_typo"]
        ]
        negative_index = next(
            index
            for index, row in enumerate(fp_unknown_rows)
            if not row.example.label
        )
        poisoned_rows = list(fp_unknown_rows)
        poisoned_rows[negative_index] = replace(
            poisoned_rows[negative_index],
            fallback=True,
            ensemble=True,
        )
        fp_corpora["unknown_typo"] = eim._production_context_corpus_evaluation(
            poisoned_rows
        )
        false_positive_profiles["target_max"] = replace(
            fp_profile, corpora=fp_corpora
        )
        false_positive = production_context_ensemble_gate_breakdown(
            permissive,
            replace(baseline, profiles=false_positive_profiles),
        )
        self.assertIs(false_positive["passed"], False)
        fp_profiles = cast(dict[str, object], false_positive["profiles"])
        fp_target = cast(dict[str, object], fp_profiles["target_max"])
        fp_gate_corpora = cast(dict[str, object], fp_target["corpora"])
        fp_unknown_gate = cast(
            dict[str, object], fp_gate_corpora["unknown_typo"]
        )
        self.assertEqual(
            fp_unknown_gate["newly_converted_negative_vs_neutral"], 1
        )

        introduced_profiles = dict(baseline.profiles)
        introduced_profile = introduced_profiles["target_max"]
        introduced_corpora = dict(introduced_profile.corpora)
        introduced_rows = [
            replace(
                row,
                observed_context_delta=introduced_profile.profile.expected_delta,
            )
            for row in base_rows["unknown_typo"]
        ]
        introduced_rows[negative_index] = replace(
            introduced_rows[negative_index],
            fallback=False,
            ensemble=True,
        )
        introduced_corpora["unknown_typo"] = (
            eim._production_context_corpus_evaluation(introduced_rows)
        )
        introduced_profiles["target_max"] = replace(
            introduced_profile, corpora=introduced_corpora
        )
        introduced = production_context_ensemble_gate_breakdown(
            permissive,
            replace(baseline, profiles=introduced_profiles),
        )
        self.assertIs(introduced["passed"], False)
        introduced_profile_gates = cast(
            dict[str, object], introduced["profiles"]
        )
        introduced_target = cast(
            dict[str, object], introduced_profile_gates["target_max"]
        )
        introduced_gate_corpora = cast(
            dict[str, object], introduced_target["corpora"]
        )
        introduced_unknown_gate = cast(
            dict[str, object], introduced_gate_corpora["unknown_typo"]
        )
        introduced_checks = cast(
            dict[str, bool], introduced_unknown_gate["overall_checks"]
        )
        self.assertIs(
            introduced_checks["introduced_false_positive_policy"],
            False,
        )

        unreachable_profiles = dict(baseline.profiles)
        unreachable_profile = unreachable_profiles["none_min"]
        unreachable_corpora = dict(unreachable_profile.corpora)
        unreachable_rows = [
            replace(
                row,
                observed_context_delta=unreachable_profile.profile.expected_delta,
            )
            for row in base_rows["unknown_typo"]
        ]
        unreachable_rows[0] = replace(
            unreachable_rows[0], model_evaluated=False
        )
        unreachable_corpora["unknown_typo"] = (
            eim._production_context_corpus_evaluation(unreachable_rows)
        )
        unreachable_profiles["none_min"] = replace(
            unreachable_profile, corpora=unreachable_corpora
        )
        unreachable = production_context_ensemble_gate_breakdown(
            permissive,
            replace(baseline, profiles=unreachable_profiles),
        )
        self.assertIs(unreachable["passed"], False)

        recall_profiles = dict(baseline.profiles)
        source_min = recall_profiles["source_min"]
        recall_corpora = dict(source_min.corpora)
        adverse_rows = tuple(
            replace(
                row,
                ensemble=False,
                observed_context_delta=source_min.profile.expected_delta,
            )
            if row.example.label and row.example.trigger == "space"
            else replace(
                row,
                observed_context_delta=source_min.profile.expected_delta,
            )
            for row in base_rows["sealed_test"]
        )
        recall_corpora["sealed_test"] = eim._production_context_corpus_evaluation(
            adverse_rows
        )
        recall_profiles["source_min"] = replace(
            source_min, corpora=recall_corpora
        )
        recall_gate = production_context_ensemble_gate_breakdown(
            permissive,
            replace(baseline, profiles=recall_profiles),
        )
        self.assertIs(recall_gate["passed"], False)
        recall_profile_gates = cast(dict[str, object], recall_gate["profiles"])
        source_min_gate = cast(
            dict[str, object], recall_profile_gates["source_min"]
        )
        source_min_corpora = cast(
            dict[str, object], source_min_gate["corpora"]
        )
        adverse_gate = cast(
            dict[str, object], source_min_corpora["sealed_test"]
        )
        adverse_triggers = cast(
            dict[str, object], adverse_gate["per_trigger"]
        )
        adverse_space = cast(dict[str, object], adverse_triggers["space"])
        self.assertEqual(
            adverse_space["recall_policy"],
            "contextual-fallback-minus-0.005",
        )
        adverse_checks = cast(dict[str, bool], adverse_space["checks"])
        self.assertIs(adverse_checks["recall"], False)

        payload = eim._production_context_payload(baseline, gate)
        self.assertEqual(payload["schema_version"], 1)
        self.assertEqual(
            set(cast(dict[str, object], payload["profiles"])),
            {profile.name for profile in PRODUCTION_CONTEXT_PROFILES},
        )
        cache_payload = cast(dict[str, object], payload["prediction_cache"])
        self.assertIs(
            cache_payload["context_invariance_proved_before_use"], True
        )

    def test_unknown_typo_excludes_train_safety_and_protected_signatures(self) -> None:
        snapshots = self.dictionary_snapshots()
        with patch(
            "evaluate_intent_model.LanguageModel.load",
            side_effect=self.language_model,
        ):
            baseline = build_unknown_typo_disjoint_corpus(
                {0: set(), 1: set()},
                sealed_physical_signatures=frozenset(),
                minimum_words_per_group=1,
                hunspell_snapshots=snapshots,
            )
            negative_by_signature = {
                item.base_signature: item
                for item in baseline.examples
                if not item.label
            }
            negative_rows = tuple(negative_by_signature.values())
            self.assertEqual(len(negative_rows), INTENT_EXTERNAL_EXPECTED_NEGATIVE_ROWS)
            train_claim = replace(negative_rows[0], safety=False)
            safety_claim = replace(negative_rows[1], safety=True)
            pair = LayoutPair()
            protected = LexicalExample(
                original="camelCase",
                alternative=pair.translate("camelCase", "us", "ru"),
                source_group=0,
                target_group=1,
                trigger="space",
                label=False,
                weight=1.0,
                base_signature="hard:camelcase",
                variant_kind="protected",
                source_known=False,
                target_known=False,
                protected=True,
                safety=True,
            )
            rows: dict[SplitName, tuple[LexicalExample, ...]] = {
                "train": (train_claim,),
                "development": (),
                "calibration": (),
                "threshold": (),
                "test": (),
            }
            sealed_dataset = DatasetBundle(rows, (safety_claim, protected))
            sealed_index = build_sealed_signature_index(sealed_dataset)
            baseline_signatures = {
                item.base_signature.removeprefix("hunspell-unknown:")
                for item in negative_rows
            }
            self.assertTrue(baseline_signatures <= sealed_index.signatures)
            self.assertIn("camelcase", sealed_index.signatures)
            self.assertEqual(sealed_index.split_lexical_signature_count, 1)
            self.assertEqual(sealed_index.safety_lexical_signature_count, 1)
            self.assertEqual(sealed_index.protected_exact_signature_count, 1)
            self.assertEqual(
                sealed_index.sha256,
                physical_signature_set_sha256(sealed_index.signatures),
            )
            malformed_rows = dict(rows)
            malformed_rows["train"] = (
                replace(
                    train_claim,
                    base_signature="hunspell-unknown:wrong-signature",
                ),
            )
            with self.assertRaisesRegex(
                RuntimeError,
                "does not reproduce its physical signature",
            ):
                build_sealed_signature_index(
                    DatasetBundle(malformed_rows, (safety_claim, protected))
                )

            filtered = build_unknown_typo_disjoint_corpus(
                {0: set(), 1: set()},
                sealed_physical_signatures=sealed_index.signatures,
                minimum_words_per_group=1,
                hunspell_snapshots=snapshots,
            )
            selected_signatures = {
                item.base_signature.removeprefix("hunspell-unknown:")
                for item in filtered.examples
            }
            self.assertTrue(selected_signatures.isdisjoint(sealed_index.signatures))
            self.assertGreaterEqual(
                sum(filtered.rejected_sealed_overlaps_by_group.values()),
                len(baseline_signatures),
            )
            self.assertEqual(filtered.selected_sealed_overlap_count, 0)
            self.assertEqual(
                filtered.exclusion_signature_count,
                sealed_index.signature_count,
            )
            self.assertEqual(
                filtered.exclusion_signature_sha256,
                sealed_index.sha256,
            )

    def test_unknown_typo_strict_gate_requires_every_symmetric_row_to_reach_model(self) -> None:
        perfect = ConfusionMatrix(1, 0, 1, 0)
        strong = self.STRONG_SYMMETRIC_METRICS
        metrics = {trigger: strong for trigger in TRIGGERS}
        safety_metrics = {
            trigger: ConfusionMatrix(0, 0, INTENT_EXTERNAL_SAFETY_TRIGGER_SAMPLE_COUNT, 0)
            for trigger in TRIGGERS
        }
        safety_sample_count = INTENT_EXTERNAL_SAFETY_TRIGGER_SAMPLE_COUNT * len(TRIGGERS)
        safety_policy = SafetyPolicyEvaluation(
            per_trigger=safety_metrics,
            samples=safety_sample_count,
            protected_samples=INTENT_EXTERNAL_SAFETY_TRIGGER_SAMPLE_COUNT,
            lexical_collision_samples=safety_sample_count - INTENT_EXTERNAL_SAFETY_TRIGGER_SAMPLE_COUNT,
            pre_model_guarded_samples=safety_sample_count,
            expected_pre_model_guard_samples=safety_sample_count,
            expected_pre_model_guarded_samples=safety_sample_count,
            model_evaluated_samples=0,
            guard_failure_samples=0,
            reason_counts={"guarded": safety_sample_count},
        )
        sealed_index = SealedSignatureIndex(
            frozenset(),
            physical_signature_set_sha256(()),
            0,
            0,
            0,
        )
        snapshots = self.dictionary_snapshots()
        dictionary_provenance = {
            group: snapshot.provenance for group, snapshot in snapshots.items()
        }
        dictionary_sources = {
            group: item.dictionary.path
            for group, item in dictionary_provenance.items()
        }
        lexical_corpus = LexicalDisjointCorpus(
            examples=(),
            words_by_group={0: 1, 1: 1},
            dictionary_sources=dictionary_sources,
            minimum_words_per_group=1,
            exclusion_signature_sha256=sealed_index.sha256,
            corpus_sha256=external_corpus_sha256(()),
            dictionary_provenance=dictionary_provenance,
        )
        development_rows = tuple(
            lexical_example(
                label,
                trigger=trigger,
                signature=f"hunspell-unknown:{signature}",
            )
            for signature in ("development-0", "development-1")
            for trigger in TRIGGERS
            for label in (False, True)
        )
        development_corpus = LexicalDisjointCorpus(
            examples=development_rows,
            words_by_group={0: 1, 1: 1},
            dictionary_sources=dictionary_sources,
            minimum_words_per_group=1,
            exclusion_signature_sha256=sealed_index.sha256,
            corpus_sha256=external_corpus_sha256(development_rows),
            rank_namespace=UNKNOWN_TYPO_DEVELOPMENT_RANK_NAMESPACE,
            choice_namespace=UNKNOWN_TYPO_DEVELOPMENT_CHOICE_NAMESPACE,
            dictionary_provenance=dictionary_provenance,
        )
        development_signatures = unknown_typo_physical_signatures(
            development_corpus
        )
        unknown_rows = tuple(
            lexical_example(
                label,
                trigger=trigger,
                signature=f"hunspell-unknown:{signature}",
            )
            for signature in ("holdout-0", "holdout-1")
            for trigger in TRIGGERS
            for label in (False, True)
        )
        unknown_corpus = LexicalDisjointCorpus(
            examples=unknown_rows,
            words_by_group={0: 1, 1: 1},
            dictionary_sources=dictionary_sources,
            minimum_words_per_group=1,
            exclusion_signature_count=len(development_signatures),
            exclusion_signature_sha256=physical_signature_set_sha256(
                development_signatures
            ),
            corpus_sha256=external_corpus_sha256(unknown_rows),
            rank_namespace=UNKNOWN_TYPO_HOLDOUT_RANK_NAMESPACE,
            choice_namespace=UNKNOWN_TYPO_HOLDOUT_CHOICE_NAMESPACE,
            dictionary_provenance=dictionary_provenance,
        )
        hard_guard = PredictionComparison(
            perfect,
            perfect,
            0,
            0,
            0,
            0,
            INTENT_EXTERNAL_EXPECTED_HARD_GUARD_SAMPLES,
            INTENT_EXTERNAL_EXPECTED_HARD_GUARD_MODEL_EVALUATED,
            0,
        )
        unknown = PredictionComparison(
            perfect,
            perfect,
            0,
            0,
            0,
            0,
            INTENT_EXTERNAL_UNKNOWN_TYPO_EXAMPLES_PER_TRIGGER * len(TRIGGERS),
            INTENT_EXTERNAL_UNKNOWN_TYPO_EXAMPLES_PER_TRIGGER * len(TRIGGERS),
            INTENT_EXTERNAL_UNKNOWN_TYPO_NEGATIVE_EXAMPLES_PER_TRIGGER * len(TRIGGERS),
        )
        source_known = PredictionComparison(
            ConfusionMatrix(0, 0, INTENT_EXTERNAL_SOURCE_KNOWN_SAMPLE_COUNT, 0),
            ConfusionMatrix(0, 0, INTENT_EXTERNAL_SOURCE_KNOWN_SAMPLE_COUNT, 0),
            0,
            0,
            0,
            0,
            INTENT_EXTERNAL_SOURCE_KNOWN_SAMPLE_COUNT,
            0,
            0,
        )
        arguments: dict[str, object] = {
            "config": config(
                threshold_max_false_positive_rate=INTENT_STRICT_FALSE_POSITIVE_RATE,
                pause_threshold_max_false_positive_rate=INTENT_STRICT_FALSE_POSITIVE_RATE,
            ),
            "external_policy": self.external_policy(
                lexical_corpus,
                unknown_corpus,
                development_corpus,
            ),
            "provenance": (VerificationCheck("ok", True, "ok"),),
            "hunspell_handle_snapshot_stable": True,
            "runtime_threshold_selection_matches": True,
            "test": metrics,
            "context_test": {
                profile.name: metrics for profile in CONTEXT_STRESS_PROFILES
            },
            "context_test_typos": {
                profile.name: metrics for profile in CONTEXT_STRESS_PROFILES
            },
            "safety": safety_policy,
            "typo_unknown": metrics,
            "comparison": unknown,
            "lexical_disjoint": lexical_corpus,
            "lexical_comparison": hard_guard,
            "source_known_comparison": source_known,
            "sealed_signature_index": sealed_index,
            "unknown_typo_development": development_corpus,
            "unknown_typo_disjoint": unknown_corpus,
            "unknown_typo_comparison": unknown,
            "unknown_typo_raw_model": metrics,
            "latency": {
                "load_ms": {"p95": 1.0},
                "inference_ms": {"p95": 1.0},
                "artifact_bytes": 1,
                "deterministic_predictions": True,
            },
            "veto_false_negative_rate": 0.0,
            "production_context_gate": {"passed": True},
        }
        gates = _strict_gates(**arguments)  # type: ignore[arg-type]
        self.assertEqual(set(gates), STRICT_GATE_NAMES)
        self.assertTrue(strict_gates_pass(gates))
        self.assertTrue(gates["hunspell_hard_guard_regression"])
        self.assertTrue(gates["lexical_disjoint_corpus_provenance"])
        self.assertTrue(gates["unknown_typo_model_evaluated"])
        self.assertTrue(gates["unknown_typo_development_provenance"])
        self.assertTrue(gates["unknown_typo_holdout_provenance"])
        self.assertTrue(gates["unknown_typo_holdout_disjointness"])
        self.assertTrue(gates["sealed_test"])
        self.assertTrue(gates["sealed_test_context_stress"])
        self.assertTrue(gates["safety"])
        self.assertTrue(gates["typo_unknown_recall"])
        self.assertTrue(gates["unknown_typo_raw_model_integrity"])
        self.assertTrue(gates["external_policy_schema"])
        self.assertTrue(gates["external_minimum_corpus_policy"])
        self.assertTrue(gates["external_trigger_expansion_policy"])
        self.assertTrue(gates["external_hunspell_provenance"])
        self.assertTrue(gates["hunspell_handle_snapshot_stability"])

        arguments["hunspell_handle_snapshot_stable"] = False
        unstable_handle = _strict_gates(**arguments)  # type: ignore[arg-type]
        self.assertFalse(
            unstable_handle["hunspell_handle_snapshot_stability"]
        )
        arguments["hunspell_handle_snapshot_stable"] = True

        arguments["runtime_threshold_selection_matches"] = False
        runtime_mismatch = _strict_gates(**arguments)  # type: ignore[arg-type]
        self.assertFalse(
            runtime_mismatch["runtime_threshold_selection_evidence"]
        )
        arguments["runtime_threshold_selection_matches"] = True

        missing_gate = dict(gates)
        missing_gate.pop("artifact_size")
        self.assertFalse(strict_gates_pass(missing_gate))
        self.assertFalse(strict_gates_pass({**gates, "unexpected": True}))
        truthy_gate: dict[str, object] = dict(gates)
        truthy_gate["artifact_size"] = 1
        self.assertFalse(strict_gates_pass(truthy_gate))

        arguments["production_context_gate"] = {"passed": 1}
        truthy_production_context = _strict_gates(
            **arguments  # type: ignore[arg-type]
        )
        self.assertIs(
            truthy_production_context["production_context_ensemble"],
            False,
        )
        arguments["production_context_gate"] = {"passed": True}

        complete_context = cast(
            dict[str, dict[CorrectionTrigger, ConfusionMatrix]],
            arguments["context_test"],
        )
        arguments["context_test"] = {
            name: values
            for name, values in complete_context.items()
            if name != CONTEXT_STRESS_PROFILES[0].name
        }
        missing_context = _strict_gates(**arguments)  # type: ignore[arg-type]
        self.assertFalse(missing_context["sealed_test_context_stress"])
        arguments["context_test"] = complete_context

        baseline_policy = cast(
            ExternalEvaluationPolicy, arguments["external_policy"]
        )
        arguments["external_policy"] = replace(
            baseline_policy, schema_version=1
        )
        schema_gates = _strict_gates(**arguments)  # type: ignore[arg-type]
        self.assertFalse(schema_gates["external_policy_schema"])
        arguments["external_policy"] = replace(
            baseline_policy,
            minimum_words_per_group=INTENT_EXTERNAL_UNDERSIZED_MINIMUM_WORDS_PER_GROUP,
        )
        self.assertFalse(
            _strict_gates(**arguments)[  # type: ignore[arg-type]
                "external_minimum_corpus_policy"
            ]
        )
        arguments["external_policy"] = replace(
            baseline_policy,
            trigger_expansion=tuple(reversed(TRIGGERS)),
        )
        self.assertFalse(
            _strict_gates(**arguments)[  # type: ignore[arg-type]
                "external_trigger_expansion_policy"
            ]
        )
        wrong_hunspell = dict(baseline_policy.hunspell)
        wrong_hunspell[0] = replace(
            wrong_hunspell[0],
            dictionary=replace(
                wrong_hunspell[0].dictionary,
                sha256="f" * SHA256_HEX_CHARACTERS,
            ),
        )
        arguments["external_policy"] = replace(
            baseline_policy,
            hunspell=wrong_hunspell,
        )
        self.assertFalse(
            _strict_gates(**arguments)[  # type: ignore[arg-type]
                "external_hunspell_provenance"
            ]
        )
        wrong_size_hunspell = dict(baseline_policy.hunspell)
        wrong_size_hunspell[1] = replace(
            wrong_size_hunspell[1],
            affix=replace(
                wrong_size_hunspell[1].affix,
                bytes=wrong_size_hunspell[1].affix.bytes + 1,
            ),
        )
        arguments["external_policy"] = replace(
            baseline_policy,
            hunspell=wrong_size_hunspell,
        )
        self.assertFalse(
            _strict_gates(**arguments)[  # type: ignore[arg-type]
                "external_hunspell_provenance"
            ]
        )
        arguments["external_policy"] = replace(
            baseline_policy,
            lexical_disjoint_corpus_sha256="a" * SHA256_HEX_CHARACTERS,
        )
        self.assertFalse(
            _strict_gates(**arguments)[  # type: ignore[arg-type]
                "lexical_disjoint_corpus_provenance"
            ]
        )
        arguments["external_policy"] = replace(
            baseline_policy,
            unknown_typo_holdout_corpus_sha256="b" * SHA256_HEX_CHARACTERS,
        )
        self.assertFalse(
            _strict_gates(**arguments)[  # type: ignore[arg-type]
                "unknown_typo_holdout_provenance"
            ]
        )
        arguments["external_policy"] = baseline_policy

        arguments["test"] = {}
        empty_test = _strict_gates(**arguments)  # type: ignore[arg-type]
        self.assertFalse(empty_test["sealed_test"])
        arguments["test"] = metrics

        no_safety_negatives = dict(safety_metrics)
        no_safety_negatives["pause"] = ConfusionMatrix(0, 0, 0, 0)
        arguments["safety"] = replace(
            safety_policy,
            per_trigger=no_safety_negatives,
            samples=safety_sample_count - INTENT_EXTERNAL_SAFETY_TRIGGER_SAMPLE_COUNT,
            protected_samples=INTENT_EXTERNAL_SAFETY_TRIGGER_SAMPLE_COUNT,
            lexical_collision_samples=(
                safety_sample_count
                - INTENT_EXTERNAL_SAFETY_TRIGGER_SAMPLE_COUNT
                - INTENT_EXTERNAL_SAFETY_TRIGGER_SAMPLE_COUNT
            ),
            pre_model_guarded_samples=safety_sample_count - INTENT_EXTERNAL_SAFETY_TRIGGER_SAMPLE_COUNT,
            expected_pre_model_guard_samples=safety_sample_count - INTENT_EXTERNAL_SAFETY_TRIGGER_SAMPLE_COUNT,
            expected_pre_model_guarded_samples=(
                safety_sample_count - INTENT_EXTERNAL_SAFETY_TRIGGER_SAMPLE_COUNT
            ),
            guard_failure_samples=0,
            reason_counts={"guarded": safety_sample_count - INTENT_EXTERNAL_SAFETY_TRIGGER_SAMPLE_COUNT},
        )
        empty_safety = _strict_gates(**arguments)  # type: ignore[arg-type]
        self.assertFalse(empty_safety["safety"])
        arguments["safety"] = replace(
            safety_policy,
            expected_pre_model_guarded_samples=safety_sample_count - 1,
            guard_failure_samples=1,
        )
        bypassed_guard = _strict_gates(**arguments)  # type: ignore[arg-type]
        self.assertFalse(bypassed_guard["safety"])
        arguments["safety"] = safety_policy

        no_typo_positives = dict(metrics)
        no_typo_positives["pause"] = self.EMPTY_TRIGGER_METRICS
        arguments["typo_unknown"] = no_typo_positives
        empty_typo = _strict_gates(**arguments)  # type: ignore[arg-type]
        self.assertFalse(empty_typo["typo_unknown_recall"])
        arguments["typo_unknown"] = metrics

        weak_sample = dict(metrics)
        weak_sample["space"] = self.WEAK_SAMPLE_METRICS
        self.assertGreater(
            wilson_upper_bound(0, INTENT_WEAK_SAMPLE_SIZE), INTENT_STRICT_FALSE_POSITIVE_RATE
        )
        arguments["test"] = weak_sample
        weak_sealed = _strict_gates(**arguments)  # type: ignore[arg-type]
        self.assertFalse(weak_sealed["sealed_test"])
        arguments["test"] = metrics

        arguments["unknown_typo_disjoint"] = replace(
            unknown_corpus,
            exclusion_signature_sha256="0" * SHA256_HEX_CHARACTERS,
        )
        wrong_provenance = _strict_gates(**arguments)  # type: ignore[arg-type]
        self.assertFalse(
            wrong_provenance["unknown_typo_holdout_disjointness"]
        )
        arguments["unknown_typo_disjoint"] = unknown_corpus
        incomplete = replace(unknown, negative_model_evaluated=1)
        arguments["unknown_typo_comparison"] = incomplete
        failed = _strict_gates(**arguments)  # type: ignore[arg-type]
        self.assertFalse(failed["unknown_typo_model_evaluated"])

        arguments["unknown_typo_comparison"] = unknown
        poisoned_metrics = {
            trigger: ConfusionMatrix(1, 0, 0, 0) for trigger in TRIGGERS
        }
        arguments["unknown_typo_raw_model"] = poisoned_metrics
        poisoned = _strict_gates(**arguments)  # type: ignore[arg-type]
        self.assertTrue(poisoned["unknown_typo_model_evaluated"])
        self.assertTrue(poisoned["unknown_typo_false_positives"])
        self.assertFalse(poisoned["unknown_typo_raw_model_integrity"])

        arguments["unknown_typo_raw_model"] = metrics
        latency = cast(dict[str, object], arguments["latency"])
        latency["artifact_bytes"] = MAX_CONTAINER_BYTES
        at_limit = _strict_gates(**arguments)  # type: ignore[arg-type]
        self.assertTrue(at_limit["artifact_size"])
        latency["artifact_bytes"] = MAX_CONTAINER_BYTES + 1
        above_limit = _strict_gates(**arguments)  # type: ignore[arg-type]
        self.assertFalse(above_limit["artifact_size"])


class ArtifactAndStatisticsTests(unittest.TestCase):
    STRONG_SYMMETRIC_METRICS = ConfusionMatrix(INTENT_STRONG_SAMPLE_SIZE, 0, INTENT_STRONG_SAMPLE_SIZE, 0)
    MODERATE_SYMMETRIC_METRICS = ConfusionMatrix(INTENT_MODERATE_SAMPLE_SIZE, 0, INTENT_MODERATE_SAMPLE_SIZE, 0)

    def test_v6_rejection_receipt_is_complete_and_self_consistent(self) -> None:
        repository = Path(__file__).resolve().parents[1]
        receipt = cast(
            dict[str, object],
            json.loads(
                (
                    repository / "model/intent_v1/rejection-v6.json"
                ).read_text(encoding="utf-8")
            ),
        )
        registry = cast(
            dict[str, object],
            json.loads(
                (
                    repository / "model/intent_v1/seal-registry-v6.json"
                ).read_text(encoding="utf-8")
            ),
        )
        self.assertEqual(receipt["schema_version"], 1)
        self.assertEqual(receipt["status"], "rejected")
        self.assertIs(receipt["quality_gates_passed"], False)
        self.assertIs(receipt["artifact_published"], False)
        for field_name in (
            "split_namespace",
            "candidate_sha256",
            "candidate_dataset_sha256",
            "config_sha256",
        ):
            self.assertEqual(receipt[field_name], registry[field_name])
        slice_results: dict[str, bool] = {}
        for name in (
            "ordinary_non_pause",
            "typo_non_pause",
            "ordinary_pause",
            "typo_pause",
        ):
            evidence = cast(dict[str, object], receipt[name])
            false_positives = cast(int, evidence["false_positives"])
            negative_samples = cast(int, evidence["negative_samples"])
            expected_upper = wilson_upper_bound(
                false_positives,
                negative_samples,
            )
            self.assertEqual(
                evidence["false_positive_rate_upper_95"],
                expected_upper,
            )
            passed = expected_upper <= cast(
                float, evidence["maximum_false_positive_rate_upper_95"]
            )
            self.assertIs(evidence["passed"], passed)
            slice_results[name] = passed
        self.assertEqual(
            slice_results,
            {
                "ordinary_non_pause": True,
                "typo_non_pause": False,
                "ordinary_pause": True,
                "typo_pause": True,
            },
        )

    def _run_successful_trainer_fixture(
        self,
        root: Path,
        *,
        dry_run: bool,
        mismatch_serialized_candidate: bool,
        events: list[str],
        publications: list[tuple[tuple[Path, bytes], ...]],
    ) -> int:
        training_config = config()
        config_path = root / "config.json"
        config_bytes = b"{}"
        config_path.write_bytes(config_bytes)
        config_digest = hashlib.sha256(config_bytes).hexdigest()
        artifact = root / "model.ksm"
        manifest = root / "manifest.json"
        report = root / "report.json"
        empty_rows: dict[SplitName, tuple[LexicalExample, ...]] = {
            split: ()
            for split in (*PRESEALED_SPLITS, *SEALED_TEST_SPLITS)
        }
        empty_lexicon_rows: dict[SplitName, tuple[LexiconWord, ...]] = {
            split: ()
            for split in (*PRESEALED_SPLITS, *SEALED_TEST_SPLITS)
        }
        candidate = DatasetBundle(dict(empty_rows), ())
        sealed_rows = {
            **empty_rows,
            "test": (
                lexical_example(True, signature="sealed-only"),
                lexical_example(False, signature="sealed-only"),
            ),
        }
        sealed = DatasetBundle(sealed_rows, ())
        prepared = PreparedLexicon(empty_lexicon_rows, ())
        strong = self.STRONG_SYMMETRIC_METRICS
        per_trigger = {trigger: strong for trigger in TRIGGERS}
        context_metrics = {
            profile.name: per_trigger for profile in CONTEXT_STRESS_PROFILES
        }
        thresholds = {
            trigger: ThresholdSelection(trigger, 0.0, strong, strong)
            for trigger in TRIGGERS
        }
        veto = VetoSelection(-INTENT_ARTIFACT_FIXTURE_VETO_THRESHOLD, INTENT_VETO_SELECTION_SAMPLE_COUNT, 0, 0.0)
        safety_audit = GuardedSafetyAudit(
            INTENT_SAFETY_SAMPLE_MULTIPLIER * len(TRIGGERS),
            len(TRIGGERS),
            len(TRIGGERS),
            TRIGGERS,
            TRIGGERS,
            TRIGGERS,
            (),
        )
        scorer_bundle = SimpleNamespace(
            scorers=test_word_scorers(),
            provenance_payload=lambda: {"kind": "test"},
        )
        training_model = SimpleNamespace(
            sparse_weights=lambda: {},
            bias=0.0,
        )
        training_result = SimpleNamespace(
            model=training_model,
            best_epoch=1,
            history=(),
        )
        source_arguments = (
            LexiconSource(
                "en_US", 0, "en.lm", "0" * SHA256_HEX_CHARACTERS, 1, "GPL-3+", "copyright"
            ),
            LexiconSource(
                "ru_RU", 1, "ru.lm", "1" * SHA256_HEX_CHARACTERS, 1, "GPL-3+", "copyright"
            ),
        )
        registry_path = root / training_config.sealed_evaluation.registry_path
        receipt = SealedEvaluationReceipt(
            1,
            SPLIT_NAMESPACE,
            "a" * SHA256_HEX_CHARACTERS,
            config_digest,
            "b" * SHA256_HEX_CHARACTERS,
            training_config.sealed_evaluation.registry_path,
            "c" * SHA256_HEX_CHARACTERS,
            registry_path,
            b"receipt",
        )

        def build_phase(
            _prepared: PreparedLexicon,
            _config: TrainingConfig,
            *,
            included_splits: Collection[SplitName],
        ) -> DatasetBundle:
            selected = tuple(included_splits)
            if selected == PRESEALED_SPLITS:
                events.append("build:presealed")
                return candidate
            if selected == SEALED_TEST_SPLITS:
                events.append("build:sealed-test")
                return sealed
            raise AssertionError(f"unexpected dataset phase: {selected!r}")

        def claim_phase(**_arguments: object) -> SealedEvaluationReceipt:
            events.append("claim")
            return receipt

        def claim_outcome_phase(**_arguments: object) -> None:
            # The real ledger lives in the repository, so the fixture records
            # the call instead of making it. The order is what matters: the
            # sealed answer must be fixed before the gates decide anything.
            events.append("claim-outcome")

        def merge_phase(
            presealed_value: DatasetBundle,
            sealed_value: DatasetBundle,
        ) -> DatasetBundle:
            events.append("merge")
            return merge_sealed_test_dataset(presealed_value, sealed_value)

        def featured(
            examples: tuple[LexicalExample, ...],
            _dimension: int,
            _extractor: object,
            *,
            supported_fingerprints: set[int] | None = None,
        ) -> tuple[FeaturedExample, ...]:
            del supported_fingerprints
            if examples == sealed.by_split["test"]:
                events.append("featurize:sealed-test")
            return tuple(FeaturedExample(example, ()) for example in examples)

        def write_mismatched_model(
            path: Path, **_arguments: object
        ) -> LinearNgramModel:
            events.append("write:kslm")
            path.write_bytes(b"mismatching-artifact")
            return cast(
                LinearNgramModel,
                SimpleNamespace(model_version="intent-v1-mismatch"),
            )

        def record_publication(
            outputs: Sequence[tuple[Path, bytes]],
        ) -> None:
            publications.append(tuple(outputs))

        evidence = (
            Path(__file__).resolve().parents[1]
            / training_config.sources.license_evidence.path
        )
        with ExitStack() as stack:
            stack.enter_context(
                patch.object(
                    tim,
                    "load_training_config_snapshot",
                    return_value=(training_config, config_digest),
                )
            )
            stack.enter_context(
                patch.object(
                    tim, "verify_training_sources", return_value=evidence
                )
            )
            stack.enter_context(patch.object(tim, "validate_training_paths"))
            stack.enter_context(
                patch.object(tim, "read_verified_frozen_file", return_value=b"x")
            )
            stack.enter_context(
                patch.object(
                    tim,
                    "load_hard_negative_development_corpus",
                    return_value=empty_hard_negative_corpus(),
                )
            )
            stack.enter_context(
                patch.object(
                    tim,
                    "merge_hard_negative_development",
                    side_effect=lambda dataset, _corpus: dataset,
                )
            )
            stack.enter_context(
                patch.object(
                    tim,
                    "load_onboard_unigrams",
                    side_effect=(((), source_arguments[0]), ((), source_arguments[1])),
                )
            )
            stack.enter_context(
                patch.object(tim, "prepare_lexicon", return_value=prepared)
            )
            stack.enter_context(
                patch.object(tim, "build_dataset", side_effect=build_phase)
            )
            stack.enter_context(
                patch.object(
                    tim,
                    "merge_sealed_test_dataset",
                    side_effect=merge_phase,
                )
            )
            stack.enter_context(patch.object(tim, "assert_no_split_leakage"))
            stack.enter_context(
                patch.object(
                    tim.TrainOnlyLanguageScorers,
                    "from_training_partition",
                    return_value=scorer_bundle,
                )
            )
            stack.enter_context(
                patch.object(
                    tim,
                    "audit_guarded_safety_corpus",
                    return_value=safety_audit,
                )
            )
            stack.enter_context(
                patch.object(tim, "runtime_feature_extractor", return_value=object())
            )
            stack.enter_context(
                patch.object(tim, "featurize_examples", side_effect=featured)
            )
            stack.enter_context(
                patch.object(tim, "fit_ftrl", return_value=training_result)
            )
            stack.enter_context(
                patch.object(
                    tim,
                    "fit_directional_platt_calibration",
                    return_value=directional_calibration(
                        samples_per_direction=1,
                        positives_per_direction=1,
                    ),
                )
            )
            stack.enter_context(
                patch.object(tim, "choose_veto_threshold", return_value=veto)
            )
            stack.enter_context(
                patch.object(
                    tim, "choose_trigger_thresholds", return_value=thresholds
                )
            )
            stack.enter_context(
                patch.object(
                    tim,
                    "score_context_stress_profiles",
                    return_value=(context_metrics, context_metrics),
                )
            )
            stack.enter_context(
                patch.object(
                    tim,
                    "threshold_selection_gate_breakdown",
                    return_value={"passed": True},
                )
            )
            stack.enter_context(
                patch.object(
                    tim, "claim_sealed_evaluation", side_effect=claim_phase
                )
            )
            stack.enter_context(
                patch.object(
                    tim, "claim_sealed_outcome", side_effect=claim_outcome_phase
                )
            )
            stack.enter_context(
                patch.object(tim, "verify_sealed_evaluation_receipt")
            )
            stack.enter_context(
                patch.object(tim, "veto_metrics", return_value=veto)
            )
            stack.enter_context(
                patch.object(tim, "confusion_at_threshold", return_value=strong)
            )
            stack.enter_context(
                patch.object(
                    tim,
                    "training_quality_gate_breakdown",
                    return_value={"passed": True},
                )
            )
            stack.enter_context(patch.object(tim, "verify_toolchain_snapshot"))
            stack.enter_context(
                patch.object(
                    tim, "publish_bytes_bundle", side_effect=record_publication
                )
            )
            if mismatch_serialized_candidate:
                stack.enter_context(
                    patch.object(
                        tim, "write_model", side_effect=write_mismatched_model
                    )
                )
                stack.enter_context(
                    patch.object(
                        tim,
                        "runtime_candidate_model_parameters",
                        return_value={"mismatch": True},
                    )
                )
            stack.enter_context(patch("builtins.print"))
            arguments = [
                "--config",
                str(config_path),
                "--en-model",
                str(root / "en.lm"),
                "--ru-model",
                str(root / "ru.lm"),
                "--artifact",
                str(artifact),
                "--manifest",
                str(manifest),
                "--test-report",
                str(report),
                "--build-environment",
                str(root / "build-environment.json"),
                "--workers",
                "1",
            ]
            if dry_run:
                arguments.append("--dry-run")
            return train_main(tuple(arguments))

    def test_trainer_materializes_sealed_phase_only_after_claim(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            events: list[str] = []
            publications: list[tuple[tuple[Path, bytes], ...]] = []
            exit_code = self._run_successful_trainer_fixture(
                Path(temporary),
                dry_run=True,
                mismatch_serialized_candidate=False,
                events=events,
                publications=publications,
            )
        self.assertEqual(exit_code, 0)
        self.assertEqual(
            events,
            [
                "build:presealed",
                "claim",
                "build:sealed-test",
                "merge",
                "featurize:sealed-test",
                "claim-outcome",
            ],
        )
        self.assertEqual(publications, [])
        # The ticket is claimed before a sealed row exists, and the answer is
        # claimed after the sealed rows have been scored. Both bracket the
        # sealed phase, and this dry run reached the second one without
        # publishing anything - which is the point: a run that never publishes
        # still consumes the answer, so a failing run cannot be repeated with a
        # tweak until the sealed test gives a nicer number.
        self.assertLess(events.index("claim"), events.index("build:sealed-test"))
        self.assertLess(
            events.index("featurize:sealed-test"), events.index("claim-outcome")
        )

    def test_serialized_candidate_mismatch_is_rejected_before_seal(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            events: list[str] = []
            publications: list[tuple[tuple[Path, bytes], ...]] = []
            with self.assertRaisesRegex(
                RuntimeError,
                "presealed KSLM parameters differ after runtime serialization",
            ):
                self._run_successful_trainer_fixture(
                    Path(temporary),
                    dry_run=False,
                    mismatch_serialized_candidate=True,
                    events=events,
                    publications=publications,
                )
        self.assertIn("write:kslm", events)
        self.assertNotIn("claim", events)
        self.assertNotIn("build:sealed-test", events)
        self.assertEqual(publications, [])

    def test_model_manifest_top_level_schema_is_exact_and_type_safe(self) -> None:
        baseline = manifest_schema_fixture()
        self.assertEqual(eim._validate_manifest_schema(baseline), baseline)

        invalid: tuple[tuple[str, dict[str, object], str], ...] = (
            (
                "missing",
                {key: value for key, value in baseline.items() if key != "counts"},
                "keys mismatch",
            ),
            ("extra", {**baseline, "unexpected": True}, "keys mismatch"),
            (
                "boolean-schema",
                {**baseline, "schema_version": True},
                "must be an integer",
            ),
            (
                "float-schema",
                {**baseline, "schema_version": 1.0},
                "must be an integer",
            ),
            (
                "future-schema",
                {**baseline, "schema_version": INTENT_UNSUPPORTED_MANIFEST_SCHEMA_VERSION},
                "unsupported model manifest schema",
            ),
            (
                "wrong-model",
                {**baseline, "model_id": "other"},
                "model_id is unsupported",
            ),
            (
                "mapping-type",
                {**baseline, "toolchain": []},
                "manifest.toolchain must be an object",
            ),
            (
                "sources-type",
                {**baseline, "sources": {}},
                "manifest.sources must be an array",
            ),
            (
                "digest-type",
                {**baseline, "artifact_sha256": 1},
                "manifest.artifact_sha256 must be a string",
            ),
            (
                "string-type",
                {**baseline, "calibration_scope": 1},
                "manifest.calibration_scope must be a string",
            ),
            (
                "boolean-type",
                {**baseline, "quality_gates_passed": 1},
                "quality_gates_passed must be a boolean",
            ),
        )
        for name, value, message in invalid:
            with self.subTest(name=name):
                with self.assertRaisesRegex(ValueError, message):
                    eim._validate_manifest_schema(value)

    def test_presealed_provenance_gate_is_exact_and_fail_closed(self) -> None:
        self.assertEqual(
            eim._PRESEALED_PROVENANCE_CHECK_NAMES,
            EXPECTED_PRESEALED_PROVENANCE_CHECK_NAMES,
        )
        presealed = tuple(
            VerificationCheck(name, True, "test")
            for name in sorted(EXPECTED_PRESEALED_PROVENANCE_CHECK_NAMES)
        )
        self.assertTrue(
            provenance_checks_pass(
                presealed, require_full_dataset=False
            )
        )
        self.assertFalse(
            provenance_checks_pass(
                presealed, require_full_dataset=True
            )
        )
        full = (*presealed, VerificationCheck("dataset_sha256", True, "test"))
        self.assertTrue(
            provenance_checks_pass(full, require_full_dataset=True)
        )
        self.assertFalse(
            provenance_checks_pass(full[:-1], require_full_dataset=True)
        )
        self.assertFalse(
            provenance_checks_pass(
                (*presealed, VerificationCheck("unexpected", True, "test")),
                require_full_dataset=False,
            )
        )
        failed = list(presealed)
        failed[0] = replace(failed[0], passed=False)
        self.assertFalse(
            provenance_checks_pass(
                failed, require_full_dataset=False
            )
        )

    def test_evaluator_never_builds_sealed_partition_after_failed_receipt(self) -> None:
        empty_rows: dict[SplitName, tuple[LexicalExample, ...]] = {
            split: ()
            for split in (*PRESEALED_SPLITS, *SEALED_TEST_SPLITS)
        }
        candidate = DatasetBundle(empty_rows, ())
        scorer_bundle = SimpleNamespace(
            scorers=test_word_scorers(),
            provenance_payload=lambda: {"kind": "test"},
        )
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            build = patch.object(
                eim, "build_dataset", return_value=candidate
            )
            with ExitStack() as stack:
                stack.enter_context(
                    patch.object(
                        eim, "load_training_config", return_value=config()
                    )
                )
                stack.enter_context(
                    patch.object(
                        eim,
                        "verify_training_sources",
                        return_value=root / "copyright",
                    )
                )
                stack.enter_context(
                    patch.object(eim, "load_onboard_unigrams", return_value=((), object()))
                )
                stack.enter_context(
                    patch.object(eim, "prepare_lexicon", return_value=SimpleNamespace())
                )
                stack.enter_context(
                    patch.object(
                        eim,
                        "load_hard_negative_development_corpus",
                        return_value=empty_hard_negative_corpus(),
                    )
                )
                stack.enter_context(
                    patch.object(
                        eim,
                        "merge_hard_negative_development",
                        side_effect=lambda dataset, _corpus: dataset,
                    )
                )
                build_mock = stack.enter_context(build)
                stack.enter_context(patch.object(eim, "assert_no_split_leakage"))
                stack.enter_context(
                    patch.object(
                        TrainOnlyLanguageScorers,
                        "from_training_partition",
                        return_value=scorer_bundle,
                    )
                )
                stack.enter_context(
                    patch.object(eim, "_json_object", return_value={})
                )
                stack.enter_context(
                    patch.object(
                        LinearNgramModel,
                        "load",
                        return_value=SimpleNamespace(),
                    )
                )
                stack.enter_context(
                    patch.object(
                        eim,
                        "verify_provenance",
                        return_value=(
                            VerificationCheck(
                                "sealed_evaluation", False, "missing registry"
                            ),
                        ),
                    )
                )
                printer = stack.enter_context(patch("builtins.print"))
                exit_code = evaluate_main(
                    (
                        "--config",
                        str(root / "config.json"),
                        "--en-model",
                        str(root / "en.lm"),
                        "--ru-model",
                        str(root / "ru.lm"),
                        "--artifact",
                        str(root / "model.ksm"),
                        "--manifest",
                        str(root / "manifest.json"),
                    )
                )
            self.assertEqual(exit_code, 1)
            build_mock.assert_called_once()
            diagnostic = json.loads(cast(str, printer.call_args.args[0]))
            self.assertEqual(diagnostic["phase"], "presealed_provenance")
            self.assertIs(diagnostic["sealed_test_evaluated"], False)
            self.assertNotIn("sealed_test", diagnostic)

    def test_evaluator_full_provenance_failure_stops_before_scoring(self) -> None:
        empty_rows: dict[SplitName, tuple[LexicalExample, ...]] = {
            split: ()
            for split in (*PRESEALED_SPLITS, *SEALED_TEST_SPLITS)
        }
        empty_lexicon_rows: dict[SplitName, tuple[LexiconWord, ...]] = {
            split: ()
            for split in (*PRESEALED_SPLITS, *SEALED_TEST_SPLITS)
        }
        candidate = DatasetBundle(dict(empty_rows), ())
        candidate_with_hard_negatives = DatasetBundle(dict(empty_rows), ())
        sealed = DatasetBundle(dict(empty_rows), ())
        scorer_bundle = SimpleNamespace(
            scorers=test_word_scorers(),
            provenance_payload=lambda: {"kind": "test"},
        )
        events: list[str] = []

        def build_phase(
            _prepared: PreparedLexicon,
            _config: TrainingConfig,
            *,
            included_splits: Collection[SplitName],
        ) -> DatasetBundle:
            selected = tuple(included_splits)
            if selected == PRESEALED_SPLITS:
                events.append("build:presealed")
                return candidate
            if selected == SEALED_TEST_SPLITS:
                events.append("build:sealed-test")
                return sealed
            raise AssertionError(f"unexpected dataset phase: {selected!r}")

        def merge_phase(
            presealed_value: DatasetBundle,
            sealed_value: DatasetBundle,
        ) -> DatasetBundle:
            if presealed_value is candidate_with_hard_negatives:
                events.append("merge:serving")
            elif presealed_value is candidate:
                events.append("merge:base-exclusions")
            else:
                raise AssertionError("unexpected presealed dataset")
            return merge_sealed_test_dataset(presealed_value, sealed_value)

        presealed_checks = tuple(
            VerificationCheck(name, True, "test")
            for name in sorted(EXPECTED_PRESEALED_PROVENANCE_CHECK_NAMES)
        )
        full_checks = (
            *presealed_checks,
            VerificationCheck("dataset_sha256", False, "mismatch"),
        )
        passed_full_checks = (
            *presealed_checks,
            VerificationCheck("dataset_sha256", True, "test"),
        )
        internal_checks = tuple(
            VerificationCheck(name, True, "test")
            for name in sorted(INTERNAL_SEALED_EVIDENCE_CHECK_NAMES)
        )
        internal_evidence = SimpleNamespace(
            checks=internal_checks,
            runtime_threshold_selection={},
            test_predictions=(),
            test_metrics={},
            context_test_metrics={},
            context_test_typo_metrics={},
            safety_raw_predictions=(),
            safety_raw_metrics={},
        )
        verification_calls = 0

        def verify_phase(**_arguments: object) -> tuple[VerificationCheck, ...]:
            nonlocal verification_calls
            verification_calls += 1
            if verification_calls in INTENT_PRESEALED_VERIFY_CALL_NUMBERS:
                events.append("verify:presealed")
                return presealed_checks
            if verification_calls == INTENT_ARTIFACT_FULL_VERIFY_CALL_NUMBER:
                events.append("verify:full")
                return full_checks
            if verification_calls == INTENT_ARTIFACT_PASSED_FULL_VERIFY_CALL_NUMBER:
                events.append("verify:full")
                return passed_full_checks
            raise AssertionError("unexpected extra provenance verification")

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with ExitStack() as stack:
                stack.enter_context(
                    patch.object(eim, "load_training_config", return_value=config())
                )
                stack.enter_context(
                    patch.object(
                        eim,
                        "verify_training_sources",
                        return_value=root / "copyright",
                    )
                )
                stack.enter_context(
                    patch.object(
                        eim, "load_onboard_unigrams", return_value=((), object())
                    )
                )
                stack.enter_context(
                    patch.object(
                        eim,
                        "prepare_lexicon",
                        return_value=PreparedLexicon(empty_lexicon_rows, ()),
                    )
                )
                stack.enter_context(
                    patch.object(
                        eim,
                        "load_hard_negative_development_corpus",
                        return_value=empty_hard_negative_corpus(),
                    )
                )
                stack.enter_context(
                    patch.object(
                        eim,
                        "merge_hard_negative_development",
                        return_value=candidate_with_hard_negatives,
                    )
                )
                stack.enter_context(
                    patch.object(eim, "build_dataset", side_effect=build_phase)
                )
                stack.enter_context(
                    patch.object(
                        eim,
                        "merge_sealed_test_dataset",
                        side_effect=merge_phase,
                    )
                )
                stack.enter_context(patch.object(eim, "assert_no_split_leakage"))
                stack.enter_context(
                    patch.object(
                        TrainOnlyLanguageScorers,
                        "from_training_partition",
                        return_value=scorer_bundle,
                    )
                )
                stack.enter_context(patch.object(eim, "_json_object", return_value={}))
                stack.enter_context(
                    patch.object(
                        LinearNgramModel,
                        "load",
                        return_value=SimpleNamespace(),
                    )
                )
                stack.enter_context(
                    patch.object(eim, "verify_provenance", side_effect=verify_phase)
                )
                internal = stack.enter_context(
                    patch.object(
                        eim,
                        "recompute_internal_sealed_evidence",
                        return_value=internal_evidence,
                    )
                )
                language_load = stack.enter_context(
                    patch.object(LanguageModel, "load")
                )
                predict = stack.enter_context(
                    patch.object(eim, "predict_model_examples")
                )
                context = stack.enter_context(
                    patch.object(eim, "evaluate_context_stress")
                )
                external = stack.enter_context(
                    patch.object(eim, "build_lexical_disjoint_corpus")
                )
                production_context = stack.enter_context(
                    patch.object(eim, "evaluate_production_context_ensemble")
                )
                latency = stack.enter_context(patch.object(eim, "latency_report"))
                printer = stack.enter_context(patch("builtins.print"))
                exit_code = evaluate_main(
                    (
                        "--config",
                        str(root / "config.json"),
                        "--en-model",
                        str(root / "en.lm"),
                        "--ru-model",
                        str(root / "ru.lm"),
                        "--artifact",
                        str(root / "model.ksm"),
                        "--manifest",
                        str(root / "manifest.json"),
                    )
                )
                provenance_only_exit_code = evaluate_main(
                    (
                        "--config",
                        str(root / "config.json"),
                        "--en-model",
                        str(root / "en.lm"),
                        "--ru-model",
                        str(root / "ru.lm"),
                        "--artifact",
                        str(root / "model.ksm"),
                        "--manifest",
                        str(root / "manifest.json"),
                        "--provenance-only",
                    )
                )
            self.assertEqual(exit_code, 1)
            self.assertEqual(provenance_only_exit_code, 0)
            internal.assert_called_once()
            self.assertEqual(
                events,
                [
                    "build:presealed",
                    "verify:presealed",
                    "build:sealed-test",
                    "merge:serving",
                    "merge:base-exclusions",
                    "verify:full",
                    "build:presealed",
                    "verify:presealed",
                    "build:sealed-test",
                    "merge:serving",
                    "merge:base-exclusions",
                    "verify:full",
                ],
            )
            language_load.assert_not_called()
            predict.assert_not_called()
            context.assert_not_called()
            external.assert_not_called()
            production_context.assert_not_called()
            latency.assert_not_called()
            diagnostics = tuple(
                json.loads(cast(str, call.args[0]))
                for call in printer.call_args_list
            )
            self.assertEqual(len(diagnostics), INTENT_ARTIFACT_EXPECTED_DIAGNOSTIC_COUNT)
            self.assertEqual(diagnostics[0]["phase"], "full_provenance")
            self.assertIs(diagnostics[0]["sealed_test_evaluated"], False)
            self.assertEqual(
                diagnostics[1]["phase"], "internal_sealed_evidence"
            )
            self.assertIs(diagnostics[1]["provenance_passed"], True)
            self.assertIs(
                diagnostics[1]["internal_sealed_evidence_passed"], True
            )
            self.assertIs(diagnostics[1]["sealed_test_evaluated"], True)

    def test_internal_sealed_evidence_is_exact_and_hunspell_free(self) -> None:
        rows: dict[SplitName, tuple[LexicalExample, ...]] = {
            split: () for split in (*PRESEALED_SPLITS, *SEALED_TEST_SPLITS)
        }
        rows["calibration"] = tuple(
            lexical_example(
                label,
                trigger=trigger,
                signature=f"calibration-{trigger}-{label}",
            )
            for trigger in TRIGGERS
            for label in (True, False)
        )
        rows["test"] = tuple(
            lexical_example(
                label,
                trigger=trigger,
                signature=f"test-{trigger}-{label}",
            )
            for trigger in TRIGGERS
            for label in (True, False)
        )
        safety_rows = tuple(
            lexical_example(
                False,
                trigger=trigger,
                signature=f"safety-{trigger}",
            )
            for trigger in TRIGGERS
        )
        dataset = DatasetBundle(rows, safety_rows)
        prepared = PreparedLexicon(
            {split: () for split in (*PRESEALED_SPLITS, *SEALED_TEST_SPLITS)},
            (),
        )
        runtime_selection: dict[str, object] = {"passed": True}
        test_predictions = tuple(
            ModelPredictionRow(
                item, item.label, 0.0 if item.label else -INTENT_ARTIFACT_NEGATIVE_MODEL_LOGIT, 1.0
            )
            for item in rows["test"]
        )
        calibration_predictions = tuple(
            ModelPredictionRow(
                item, item.label, 0.0 if item.label else -INTENT_ARTIFACT_NEGATIVE_MODEL_LOGIT, 1.0
            )
            for item in rows["calibration"]
        )
        safety_predictions = tuple(
            ModelPredictionRow(item, False, -INTENT_ARTIFACT_NEGATIVE_MODEL_LOGIT, 1.0)
            for item in safety_rows
        )
        strong = prediction_metrics(test_predictions)
        context_metrics = {
            profile.name: dict(strong)
            for profile in CONTEXT_STRESS_PROFILES
        }
        safety_audit = GuardedSafetyAudit(
            samples=len(safety_rows),
            protected_samples=len(TRIGGERS),
            lexical_collision_samples=len(TRIGGERS),
            triggers=TRIGGERS,
            protected_triggers=TRIGGERS,
            lexical_collision_triggers=TRIGGERS,
            failures=(),
        )
        model = cast(
            LinearNgramModel,
            SimpleNamespace(veto_threshold=-1.0),
        )

        def run(value: dict[str, object]) -> eim.InternalSealedEvidence:
            with ExitStack() as stack:
                stack.enter_context(
                    patch.object(
                        eim,
                        "evaluate_runtime_threshold_selection",
                        return_value=runtime_selection,
                    )
                )
                stack.enter_context(
                    patch.object(
                        eim,
                        "predict_model_examples",
                        side_effect=(
                            calibration_predictions,
                            test_predictions,
                            safety_predictions,
                        ),
                    )
                )
                stack.enter_context(
                    patch.object(
                        eim,
                        "evaluate_context_stress",
                        return_value=(context_metrics, context_metrics),
                    )
                )
                stack.enter_context(
                    patch.object(
                        eim,
                        "audit_guarded_safety_corpus",
                        return_value=safety_audit,
                    )
                )
                hunspell = stack.enter_context(
                    patch.object(
                        LanguageModel,
                        "load",
                        side_effect=AssertionError("Hunspell must not load"),
                    )
                )
                result = recompute_internal_sealed_evidence(
                    model=model,
                    manifest=value,
                    config=config(),
                    prepared=prepared,
                    dataset=dataset,
                    scorers=test_word_scorers(),
                )
                hunspell.assert_not_called()
                return result

        seed = run({"veto": {}, "safety": {}})
        expected_test = {
            trigger: metrics_payload(metrics)
            for trigger, metrics in seed.test_metrics.items()
        }
        expected_typos = {
            trigger: metrics_payload(metrics)
            for trigger, metrics in seed.typo_test_metrics.items()
        }
        expected_context = {
            name: {
                "overall": {
                    trigger: metrics_payload(metrics)
                    for trigger, metrics in values.items()
                },
                "typos": {
                    trigger: metrics_payload(metrics)
                    for trigger, metrics in seed.context_test_typo_metrics[
                        name
                    ].items()
                },
            }
            for name, values in seed.context_test_metrics.items()
        }
        expected_safety_raw = {
            trigger: metrics_payload(metrics)
            for trigger, metrics in seed.safety_raw_metrics.items()
        }
        expected_counts = {
            "lexicon_words": 0,
            "collisions": 0,
            "examples": {
                split: len(dataset.by_split[split])
                for split in (*PRESEALED_SPLITS, *SEALED_TEST_SPLITS)
            },
            "safety_examples": len(dataset.safety),
            "quarantined_variant_occurrences": 0,
            "quarantined_physical_signatures": 0,
            "sealed_quarantined_variant_occurrences": 0,
            "sealed_quarantined_physical_signatures": 0,
            "sealed_test_exclusion_signatures": 0,
        }
        manifest: dict[str, object] = {
            "threshold_selection_gate_breakdown": runtime_selection,
            "sealed_test": expected_test,
            "sealed_test_typos": expected_typos,
            "sealed_test_context_stress": expected_context,
            "veto": {
                "selection": asdict(seed.selection_veto),
                "sealed_test": asdict(seed.test_veto),
            },
            "safety": {
                "guard_audit": asdict(seed.safety_audit),
                "raw_model_diagnostics": expected_safety_raw,
            },
            "quality_gate_breakdown": seed.quality_gate_breakdown,
            "quality_gates_passed": (
                seed.quality_gate_breakdown.get("passed") is True
            ),
            "counts": expected_counts,
            "variant_quarantine_sha256": dataset.variant_quarantine.sha256,
            "sealed_variant_quarantine_sha256": (
                dataset.sealed_variant_quarantine.sha256
            ),
            "sealed_test_exclusion_signatures_sha256": hashlib.sha256(
                b""
            ).hexdigest(),
        }
        manifest = cast(
            dict[str, object],
            json.loads(json.dumps(manifest, allow_nan=False)),
        )
        verified = run(manifest)
        self.assertTrue(
            eim.internal_sealed_evidence_checks_pass(verified.checks)
        )

        tamper_cases = (
            ("runtime_threshold_selection", "threshold_selection_gate_breakdown"),
            ("sealed_test", "sealed_test"),
            ("sealed_test_typos", "sealed_test_typos"),
            ("sealed_test_context_stress", "sealed_test_context_stress"),
            ("quality_gate_breakdown", "quality_gate_breakdown"),
            ("sealed_dataset_metadata", "counts"),
        )
        for check_name, field_name in tamper_cases:
            with self.subTest(check=check_name):
                changed = copy.deepcopy(manifest)
                changed[field_name] = {}
                by_name = {check.name: check for check in run(changed).checks}
                self.assertFalse(by_name[check_name].passed)

        for check_name, field_name in (
            ("veto_selection", "selection"),
            ("veto_sealed_test", "sealed_test"),
        ):
            changed = copy.deepcopy(manifest)
            cast(dict[str, object], changed["veto"])[field_name] = {}
            by_name = {check.name: check for check in run(changed).checks}
            self.assertFalse(by_name[check_name].passed)

        for check_name, field_name in (
            ("safety_guard_audit", "guard_audit"),
            ("safety_raw_model_diagnostics", "raw_model_diagnostics"),
        ):
            changed = copy.deepcopy(manifest)
            cast(dict[str, object], changed["safety"])[field_name] = {}
            by_name = {check.name: check for check in run(changed).checks}
            self.assertFalse(by_name[check_name].passed)

    def test_sealed_registry_is_one_candidate_fail_closed_and_reproducible(self) -> None:
        config_value = config()
        config_bytes = b'{"test":"sealed-registry"}\n'
        config_digest = hashlib.sha256(config_bytes).hexdigest()
        dataset_digest = "a" * SHA256_HEX_CHARACTERS
        with tempfile.TemporaryDirectory() as temporary:
            (Path(temporary) / "model/intent_v1").mkdir(parents=True)
            config_path = Path(temporary) / "config.json"
            config_path.write_bytes(config_bytes)
            first = claim_sealed_evaluation(
                config=config_value,
                candidate_sha256="b" * SHA256_HEX_CHARACTERS,
                config_sha256=config_digest,
                candidate_dataset_sha256=dataset_digest,
                repository_root=Path(temporary),
            )
            repeated = claim_sealed_evaluation(
                config=config_value,
                candidate_sha256="b" * SHA256_HEX_CHARACTERS,
                config_sha256=config_digest,
                candidate_dataset_sha256=dataset_digest,
                repository_root=Path(temporary),
            )
            self.assertEqual(first.payload(), repeated.payload())
            self.assertTrue(
                sealed_evaluation_evidence_is_valid(
                    config=config_value,
                    value=first.payload(),
                    expected_config_sha256=config_digest,
                    expected_candidate_dataset_sha256=dataset_digest,
                    expected_candidate_sha256="b" * SHA256_HEX_CHARACTERS,
                    repository_root=Path(temporary),
                )
            )
            # Same config, same dataset, different candidate hash. Since v21
            # the refusal names that case specifically, because after the
            # environment left the identity it is also how a machine that
            # computes different weights announces itself.
            with self.assertRaisesRegex(
                RuntimeError,
                "same config and the same dataset but a different candidate",
            ):
                claim_sealed_evaluation(
                    config=config_value,
                    candidate_sha256="c" * SHA256_HEX_CHARACTERS,
                    config_sha256=config_digest,
                    candidate_dataset_sha256=dataset_digest,
                    repository_root=Path(temporary),
                )
            first.registry_path.write_bytes(b"tampered\n")
            with self.assertRaisesRegex(RuntimeError, "changed after claim"):
                verify_sealed_evaluation_receipt(first)
            self.assertFalse(
                sealed_evaluation_evidence_is_valid(
                    config=config_value,
                    value=first.payload(),
                    expected_config_sha256=config_digest,
                    expected_candidate_dataset_sha256=dataset_digest,
                    expected_candidate_sha256="b" * SHA256_HEX_CHARACTERS,
                    repository_root=Path(temporary),
                )
            )

    def test_sealed_registry_publication_is_atomic_for_concurrent_claims(self) -> None:
        config_value = config()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "model/intent_v1").mkdir(parents=True)
            barrier = Barrier(INTENT_CONCURRENT_CLAIMANTS)
            original_stage = tim._stage_bytes

            def staged_together(destination: Path, data: bytes) -> Path:
                staged = original_stage(destination, data)
                barrier.wait(timeout=INTENT_BARRIER_WAIT_TIMEOUT_SECONDS)
                return staged

            def claim() -> SealedEvaluationReceipt:
                return claim_sealed_evaluation(
                    config=config_value,
                    candidate_sha256="f" * SHA256_HEX_CHARACTERS,
                    config_sha256="e" * SHA256_HEX_CHARACTERS,
                    candidate_dataset_sha256="d" * SHA256_HEX_CHARACTERS,
                    repository_root=root,
                )

            with patch.object(
                tim, "_stage_bytes", side_effect=staged_together
            ):
                with ThreadPoolExecutor(max_workers=INTENT_CONCURRENT_CLAIMANTS) as executor:
                    receipts = tuple(
                        executor.map(lambda _index: claim(), range(INTENT_CONCURRENT_CLAIMANTS))
                    )
            self.assertEqual(receipts[0].payload(), receipts[1].payload())
            self.assertFalse(
                tuple((root / "model/intent_v1").glob("*.staged"))
            )

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "model/intent_v1").mkdir(parents=True)
            barrier = Barrier(INTENT_CONCURRENT_CLAIMANTS)
            original_stage = tim._stage_bytes

            def stage_competitors(destination: Path, data: bytes) -> Path:
                staged = original_stage(destination, data)
                barrier.wait(timeout=INTENT_BARRIER_WAIT_TIMEOUT_SECONDS)
                return staged

            def competing_claim(
                candidate: str,
            ) -> SealedEvaluationReceipt | RuntimeError:
                try:
                    return claim_sealed_evaluation(
                        config=config_value,
                        candidate_sha256=candidate,
                        config_sha256="e" * SHA256_HEX_CHARACTERS,
                        candidate_dataset_sha256="d" * SHA256_HEX_CHARACTERS,
                        repository_root=root,
                    )
                except RuntimeError as error:
                    return error

            with patch.object(
                tim, "_stage_bytes", side_effect=stage_competitors
            ):
                with ThreadPoolExecutor(max_workers=INTENT_CONCURRENT_CLAIMANTS) as executor:
                    results = tuple(
                        executor.map(
                            competing_claim,
                            ("a" * SHA256_HEX_CHARACTERS, "b" * SHA256_HEX_CHARACTERS),
                        )
                    )
            winners = tuple(
                result
                for result in results
                if isinstance(result, SealedEvaluationReceipt)
            )
            losers = tuple(
                result for result in results if isinstance(result, RuntimeError)
            )
            self.assertEqual(len(winners), 1)
            self.assertEqual(len(losers), 1)
            self.assertIn("already consumed", str(losers[0]))
            replay = claim_sealed_evaluation(
                config=config_value,
                candidate_sha256=winners[0].candidate_sha256,
                config_sha256="e" * SHA256_HEX_CHARACTERS,
                candidate_dataset_sha256="d" * SHA256_HEX_CHARACTERS,
                repository_root=root,
            )
            self.assertEqual(replay.payload(), winners[0].payload())
            self.assertFalse(
                tuple((root / "model/intent_v1").glob("*.staged"))
            )

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "model/intent_v1").mkdir(parents=True)
            registry = root / config_value.sealed_evaluation.registry_path

            staged_path = registry.parent / ".registry.partial.staged"

            class ShortWritingTemporaryFile:
                def __init__(self, path: Path) -> None:
                    self.name = str(path)
                    self._stream = cast(BinaryIO, path.open("wb"))

                def __enter__(self) -> ShortWritingTemporaryFile:
                    return self

                def __exit__(
                    self,
                    _exception_type: object,
                    _exception: object,
                    _traceback: object,
                ) -> None:
                    self._stream.close()

                def write(self, data: bytes) -> int:
                    partial = data[:-1]
                    self._stream.write(partial)
                    return len(partial)

                def flush(self) -> None:
                    self._stream.flush()

                def fileno(self) -> int:
                    return self._stream.fileno()

            with patch(
                "train_intent_model.tempfile.NamedTemporaryFile",
                return_value=ShortWritingTemporaryFile(staged_path),
            ):
                with self.assertRaisesRegex(
                    RuntimeError, "cannot create sealed evaluation registry"
                ):
                    claim_sealed_evaluation(
                        config=config_value,
                        candidate_sha256="f" * SHA256_HEX_CHARACTERS,
                        config_sha256="e" * SHA256_HEX_CHARACTERS,
                        candidate_dataset_sha256="d" * SHA256_HEX_CHARACTERS,
                        repository_root=root,
                    )
            self.assertFalse(registry.exists())
            self.assertFalse(staged_path.exists())

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            registry = root / config_value.sealed_evaluation.registry_path
            registry.parent.mkdir(parents=True)
            partial = b'{"schema_version":1'
            registry.write_bytes(partial)
            # A truncated ledger is refused, as before; the message now says
            # what is actually wrong instead of guessing at another candidate.
            with self.assertRaisesRegex(RuntimeError, "unreadable"):
                claim_sealed_evaluation(
                    config=config_value,
                    candidate_sha256="f" * SHA256_HEX_CHARACTERS,
                    config_sha256="e" * SHA256_HEX_CHARACTERS,
                    candidate_dataset_sha256="d" * SHA256_HEX_CHARACTERS,
                    repository_root=root,
                )
            self.assertEqual(registry.read_bytes(), partial)
            self.assertFalse(tuple(registry.parent.glob("*.staged")))

    def test_sealed_candidate_digest_binds_runtime_parameters(self) -> None:
        config_value = config()
        quantized = quantize_weights(INTENT_SEALED_CANDIDATE_QUANTIZE_WEIGHTS, INTENT_TEST_DIMENSION)
        toolchain = capture_toolchain_snapshot("d" * SHA256_HEX_CHARACTERS)
        strong = self.MODERATE_SYMMETRIC_METRICS
        thresholds = {
            trigger: ThresholdSelection(
                trigger, INTENT_ARTIFACT_SEALED_CANDIDATE_THRESHOLD_LOGIT, strong, strong
            )
            for trigger in TRIGGERS
        }
        calibration = directional_calibration(
            INTENT_ARTIFACT_SEALED_CANDIDATE_CALIBRATION_SCALE,
            -INTENT_ARTIFACT_SEALED_CANDIDATE_CALIBRATION_BIAS_MAGNITUDE,
            samples_per_direction=INTENT_ARTIFACT_SEALED_CANDIDATE_SAMPLES_PER_DIRECTION,
            positives_per_direction=INTENT_ARTIFACT_SEALED_CANDIDATE_POSITIVES_PER_DIRECTION,
        )
        veto = VetoSelection(-INTENT_ARTIFACT_FIXTURE_VETO_THRESHOLD, INTENT_VETO_SELECTION_SAMPLE_COUNT, 0, 0.0)
        model_parameters = training_candidate_model_parameters(
            config=config_value,
            quantized=quantized,
            supported_fingerprints=INTENT_SEALED_CANDIDATE_FINGERPRINTS,
            bias=INTENT_ARTIFACT_SEALED_CANDIDATE_BIAS,
            calibration=calibration,
            thresholds=thresholds,
            veto=veto,
        )
        def candidate_metadata(
            parameters: dict[str, object],
            *,
            training: dict[str, object] | None = None,
        ) -> dict[str, object]:
            return presealed_candidate_metadata_projection(
                model_id="keyswitch-layout-intent-v1",
                calibration_scope=(
                    "lexical-synthetic-not-real-world-probability"
                ),
                config_sha256=toolchain.config_sha256,
                split_namespace=SPLIT_NAMESPACE,
                toolchain=asdict(toolchain),
                source_package={"name": "test"},
                sources=(),
                candidate_counts={"examples": {}},
                variant_quarantine_sha256="f" * SHA256_HEX_CHARACTERS,
                training_language_scorer={"kind": "test"},
                gate_policy={"policy": "test"},
                training=(
                    {"seed": INTENT_TEST_TRAINING_SEED}
                    if training is None
                    else training
                ),
                quantization={"format": "signed-int16"},
                calibration=calibration.payload(),
                veto_selection=asdict(veto),
                thresholds={"test": True},
                selection_gate_breakdown={"passed": True},
                safety_guard_audit={"failures": []},
                model_parameters=parameters,
            )
        arguments: dict[str, object] = {
            "split_namespace": SPLIT_NAMESPACE,
            "config_sha256": toolchain.config_sha256,
            "candidate_dataset_sha256": "e" * SHA256_HEX_CHARACTERS,
            "toolchain": asdict(toolchain),
            "training_language_scorer": {"kind": "test"},
            "model_parameters": model_parameters,
            "selection_gate_breakdown": {"passed": True},
            "candidate_metadata": candidate_metadata(model_parameters),
        }
        baseline = sealed_candidate_sha256(**arguments)  # type: ignore[arg-type]
        self.assertEqual(
            baseline,
            sealed_candidate_sha256(**arguments),  # type: ignore[arg-type]
        )
        baseline_metadata = candidate_metadata(model_parameters)
        for field_name, changed_value in (
            ("source_package", {"name": "tampered"}),
            ("sources", [{"sha256": "0" * SHA256_HEX_CHARACTERS}]),
            ("candidate_counts", {"examples": {"train": 1}}),
            ("gate_policy", {"policy": "tampered"}),
            ("training", {"seed": INTENT_TAMPERED_TRAINING_SEED}),
            ("quantization", {"format": "tampered"}),
            ("calibration", {"slope": INTENT_TAMPERED_METADATA_VALUE}),
            ("veto_selection", {"raw_logit": INTENT_TAMPERED_METADATA_VALUE}),
            ("thresholds", {"tampered": True}),
            ("safety_guard_audit", {"failures": ["tampered"]}),
        ):
            with self.subTest(candidate_metadata=field_name):
                changed_metadata = copy.deepcopy(baseline_metadata)
                changed_metadata[field_name] = changed_value
                arguments["candidate_metadata"] = changed_metadata
                self.assertNotEqual(
                    baseline,
                    sealed_candidate_sha256(**arguments),  # type: ignore[arg-type]
                )
        arguments["candidate_metadata"] = baseline_metadata
        changed_thresholds = dict(thresholds)
        changed_thresholds["space"] = replace(
            changed_thresholds["space"],
            direction_logits={
                **changed_thresholds["space"].runtime_logits(),
                "0>1": INTENT_ARTIFACT_PRECISE_TAMPERED_LOGIT,
            },
        )
        changed_model_parameters = training_candidate_model_parameters(
            config=config_value,
            quantized=quantized,
            supported_fingerprints=INTENT_SEALED_CANDIDATE_FINGERPRINTS,
            bias=INTENT_ARTIFACT_SEALED_CANDIDATE_BIAS,
            calibration=calibration,
            thresholds=changed_thresholds,
            veto=veto,
        )
        arguments["model_parameters"] = changed_model_parameters
        arguments["candidate_metadata"] = candidate_metadata(
            changed_model_parameters
        )
        self.assertNotEqual(
            baseline,
            sealed_candidate_sha256(**arguments),  # type: ignore[arg-type]
        )
        arguments["model_parameters"] = model_parameters
        arguments["candidate_metadata"] = candidate_metadata(
            model_parameters,
            training={"seed": INTENT_TAMPERED_TRAINING_SEED},
        )
        self.assertNotEqual(
            baseline,
            sealed_candidate_sha256(**arguments),  # type: ignore[arg-type]
        )
        self.assertNotEqual(
            supported_fingerprints_sha256(INTENT_SEALED_CANDIDATE_FINGERPRINTS),
            supported_fingerprints_sha256(INTENT_TAMPERED_FINGERPRINTS),
        )

    def test_presealed_candidate_gate_rejects_known_failures(self) -> None:
        config_value = config()
        safety = GuardedSafetyAudit(
            INTENT_SAFETY_SAMPLE_MULTIPLIER * len(TRIGGERS),
            len(TRIGGERS),
            len(TRIGGERS),
            TRIGGERS,
            TRIGGERS,
            TRIGGERS,
            (),
        )
        veto = VetoSelection(-1.0, INTENT_VETO_SELECTION_SAMPLE_COUNT, 0, 0.0)
        passed = presealed_candidate_gate_breakdown(
            config_value, {"passed": True}, safety, veto
        )
        self.assertIs(passed["passed"], True)

        failed_selection = presealed_candidate_gate_breakdown(
            config_value, {"passed": False}, safety, veto
        )
        self.assertIs(failed_selection["passed"], False)
        failed_safety = presealed_candidate_gate_breakdown(
            config_value,
            {"passed": True},
            replace(safety, failures=("guard failure",)),
            veto,
        )
        self.assertIs(failed_safety["passed"], False)
        self.assertIs(failed_safety["safety"]["passed"], False)  # type: ignore[index]
        for failed_veto in (
            VetoSelection(-1.0, 0, 0, 0.0),
            VetoSelection(
                -1.0,
                INTENT_VETO_SELECTION_SAMPLE_COUNT,
                INTENT_ARTIFACT_TAMPERED_VETOED_POSITIVE_SAMPLES,
                INTENT_ARTIFACT_TAMPERED_VETO_FALSE_NEGATIVE_RATE,
            ),
        ):
            with self.subTest(veto=failed_veto):
                breakdown = presealed_candidate_gate_breakdown(
                    config_value, {"passed": True}, safety, failed_veto
                )
                self.assertIs(breakdown["passed"], False)
                self.assertIs(
                    breakdown["veto_selection"]["passed"],  # type: ignore[index]
                    False,
                )

    def test_presealed_runtime_serialization_and_payload_caps(self) -> None:
        config_value = config()
        quantized = quantize_weights(INTENT_SEALED_CANDIDATE_QUANTIZE_WEIGHTS, INTENT_TEST_DIMENSION)
        strong = self.MODERATE_SYMMETRIC_METRICS
        thresholds = {
            trigger: ThresholdSelection(
                trigger, INTENT_ARTIFACT_SEALED_CANDIDATE_THRESHOLD_LOGIT, strong, strong
            )
            for trigger in TRIGGERS
        }
        calibration = directional_calibration(
            INTENT_ARTIFACT_SEALED_CANDIDATE_CALIBRATION_SCALE,
            -INTENT_ARTIFACT_SEALED_CANDIDATE_CALIBRATION_BIAS_MAGNITUDE,
            samples_per_direction=INTENT_ARTIFACT_SEALED_CANDIDATE_SAMPLES_PER_DIRECTION,
            positives_per_direction=INTENT_ARTIFACT_SEALED_CANDIDATE_POSITIVES_PER_DIRECTION,
        )
        veto = VetoSelection(-INTENT_ARTIFACT_FIXTURE_VETO_THRESHOLD, INTENT_VETO_SELECTION_SAMPLE_COUNT, 0, 0.0)
        fingerprints = INTENT_SEALED_CANDIDATE_FINGERPRINTS
        expected = training_candidate_model_parameters(
            config=config_value,
            quantized=quantized,
            supported_fingerprints=fingerprints,
            bias=INTENT_ARTIFACT_SEALED_CANDIDATE_BIAS,
            calibration=calibration,
            thresholds=thresholds,
            veto=veto,
        )
        validate_presealed_candidate_serialization(
            config=config_value,
            quantized=quantized,
            supported_fingerprints=fingerprints,
            bias=INTENT_ARTIFACT_SEALED_CANDIDATE_BIAS,
            calibration=calibration,
            thresholds=thresholds,
            veto=veto,
            expected_parameters=expected,
        )
        with self.assertRaisesRegex(RuntimeError, "differ after runtime"):
            validate_presealed_candidate_serialization(
                config=config_value,
                quantized=quantized,
                supported_fingerprints=fingerprints,
                bias=INTENT_ARTIFACT_SEALED_CANDIDATE_BIAS,
                calibration=calibration,
                thresholds=thresholds,
                veto=veto,
                expected_parameters={
                    **expected, "bias_hex": INTENT_ARTIFACT_TAMPERED_BIAS_HEX_VALUE.hex()
                },
            )
        unbounded_thresholds = dict(thresholds)
        unbounded_thresholds["space"] = replace(
            unbounded_thresholds["space"],
            direction_logits={
                **unbounded_thresholds["space"].runtime_logits(),
                "0>1": INTENT_ARTIFACT_UNBOUNDED_LOGIT,
            },
        )
        with self.assertRaisesRegex(ValueError, "finite and bounded"):
            validate_presealed_candidate_serialization(
                config=config_value,
                quantized=quantized,
                supported_fingerprints=fingerprints,
                bias=INTENT_ARTIFACT_SEALED_CANDIDATE_BIAS,
                calibration=calibration,
                thresholds=unbounded_thresholds,
                veto=veto,
                expected_parameters=expected,
            )

        with self.assertRaisesRegex(ValueError, "weight count"):
            quantized_model_payload_sha256(
                replace(quantized, values=quantized.values[:-1]), fingerprints
            )
        invalid_values = list(quantized.values)
        invalid_values[0] = INTENT_INVALID_QUANTIZED_WEIGHT
        with self.assertRaisesRegex(ValueError, "signed int16"):
            quantized_model_payload_sha256(
                replace(quantized, values=tuple(invalid_values)), fingerprints
            )
        for invalid_fingerprints in ([1, 1], [True], [1 << UINT64_BIT_WIDTH]):
            with self.subTest(fingerprints=invalid_fingerprints):
                with self.assertRaisesRegex(ValueError, "unique uint64"):
                    quantized_model_payload_sha256(
                        quantized, invalid_fingerprints
                    )
        with patch.object(
            tim, "MAX_SUPPORTED_FINGERPRINTS", INTENT_PATCHED_MAX_SUPPORTED_FINGERPRINTS
        ):
            with self.assertRaisesRegex(ValueError, "fingerprints exceed"):
                quantized_model_payload_sha256(quantized, fingerprints)
        with patch.object(tim, "MAX_PAYLOAD_BYTES", INTENT_PATCHED_MAX_PAYLOAD_BYTES):
            with self.assertRaisesRegex(ValueError, "payload exceeds"):
                quantized_model_payload_sha256(quantized, {1})

    def test_failed_selection_gate_never_materializes_sealed_test(self) -> None:
        split_rows: dict[SplitName, tuple[LexicalExample, ...]] = {
            split: (
                lexical_example(True, signature=f"{split}-positive"),
                lexical_example(False, signature=f"{split}-negative"),
            )
            for split in PRESEALED_SPLITS
        }
        split_rows["test"] = ()
        dataset = DatasetBundle(split_rows, ())
        weak = ConfusionMatrix(0, 1, 1, 0)
        selections = {
            trigger: ThresholdSelection(trigger, 0.0, weak, weak)
            for trigger in TRIGGERS
        }
        scorer_bundle = SimpleNamespace(
            scorers=test_word_scorers(),
            provenance_payload=lambda: {"kind": "test"},
        )
        model_stub = SimpleNamespace(
            sparse_weights=lambda: {},
            bias=0.0,
        )
        training_result = SimpleNamespace(
            model=model_stub,
            best_epoch=1,
            history=(),
        )
        safety_audit = GuardedSafetyAudit(
            INTENT_SAFETY_SAMPLE_MULTIPLIER * len(TRIGGERS),
            len(TRIGGERS),
            len(TRIGGERS),
            TRIGGERS,
            TRIGGERS,
            TRIGGERS,
            (),
        )
        source = LexiconSource(
            "en_US",
            0,
            "source.lm",
            "0" * SHA256_HEX_CHARACTERS,
            1,
            "GPL-3+",
            "copyright",
        )
        featurized_splits: list[tuple[LexicalExample, ...]] = []

        def fake_featurize(
            examples: tuple[LexicalExample, ...],
            _dimension: int,
            _extractor: object,
            *,
            supported_fingerprints: set[int] | None = None,
        ) -> tuple[FeaturedExample, ...]:
            del supported_fingerprints
            if examples is dataset.by_split["test"]:
                raise AssertionError("sealed test was materialized before gate pass")
            featurized_splits.append(examples)
            return tuple(FeaturedExample(item, ()) for item in examples)

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            config_path = root / "config.json"
            config_path.write_bytes(b"{}")
            digest = hashlib.sha256(b"{}").hexdigest()
            artifact = root / "model.ksm"
            manifest = root / "manifest.json"
            report = root / "report.json"
            diagnostic_path = root / "presealed-diagnostic.json"
            with ExitStack() as stack:
                stack.enter_context(
                    patch.object(
                        tim,
                        "load_training_config_snapshot",
                        return_value=(config(), digest),
                    )
                )
                stack.enter_context(
                    patch.object(
                        tim,
                        "verify_training_sources",
                        return_value=root / "copyright",
                    )
                )
                stack.enter_context(patch.object(tim, "validate_training_paths"))
                stack.enter_context(
                    patch.object(tim, "read_verified_frozen_file", return_value=b"x")
                )
                stack.enter_context(
                    patch.object(
                        tim,
                        "load_hard_negative_development_corpus",
                        return_value=empty_hard_negative_corpus(),
                    )
                )
                stack.enter_context(
                    patch.object(
                        tim,
                        "merge_hard_negative_development",
                        side_effect=lambda dataset, _corpus: dataset,
                    )
                )
                stack.enter_context(
                    patch.object(
                        tim,
                        "load_onboard_unigrams",
                        return_value=((), source),
                    )
                )
                stack.enter_context(
                    patch.object(tim, "prepare_lexicon", return_value=SimpleNamespace())
                )
                stack.enter_context(
                    patch.object(tim, "build_dataset", return_value=dataset)
                )
                stack.enter_context(patch.object(tim, "assert_no_split_leakage"))
                stack.enter_context(
                    patch.object(
                        tim.TrainOnlyLanguageScorers,
                        "from_training_partition",
                        return_value=scorer_bundle,
                    )
                )
                stack.enter_context(
                    patch.object(
                        tim,
                        "audit_guarded_safety_corpus",
                        return_value=safety_audit,
                    )
                )
                stack.enter_context(
                    patch.object(
                        tim,
                        "runtime_feature_extractor",
                        return_value=object(),
                    )
                )
                stack.enter_context(
                    patch.object(tim, "featurize_examples", side_effect=fake_featurize)
                )
                stack.enter_context(
                    patch.object(tim, "fit_ftrl", return_value=training_result)
                )
                stack.enter_context(
                    patch.object(
                        tim,
                        "fit_directional_platt_calibration",
                        return_value=directional_calibration(
                            samples_per_direction=1,
                            positives_per_direction=1,
                        ),
                    )
                )
                stack.enter_context(
                    patch.object(
                        tim,
                        "choose_trigger_thresholds",
                        return_value=selections,
                    )
                )
                stack.enter_context(
                    patch.object(
                        tim,
                        "score_context_stress_profiles",
                        return_value=({}, {}),
                    )
                )
                stack.enter_context(
                    patch.object(
                        tim,
                        "selection_tail_diagnostics",
                        return_value={
                            "schema_version": 1,
                            "source_split": "threshold",
                            "contains_lexical_tokens": False,
                            "per_trigger": {},
                        },
                    )
                )
                threshold_gate = stack.enter_context(
                    patch.object(
                        tim,
                        "threshold_selection_gate_breakdown",
                        return_value={
                            "passed": False,
                            "neutral": {"passed": True},
                            "context_stress": {
                                "passed": False,
                                "reason": "test",
                            },
                        },
                    )
                )
                claim_gate = stack.enter_context(
                    patch.object(
                        tim,
                        "claim_sealed_evaluation",
                        side_effect=RuntimeError(
                            "sealed test namespace is already consumed"
                        ),
                    )
                )
                printer = stack.enter_context(patch("builtins.print"))
                exit_code = train_main(
                    (
                        "--config",
                        str(config_path),
                        "--en-model",
                        str(root / "en.lm"),
                        "--ru-model",
                        str(root / "ru.lm"),
                        "--artifact",
                        str(artifact),
                        "--manifest",
                        str(manifest),
                        "--test-report",
                        str(report),
                        "--build-environment",
                        str(root / "build-environment.json"),
                        "--diagnostic-output",
                        str(diagnostic_path),
                        "--workers",
                        "1",
                    )
                )
                threshold_gate.return_value = {"passed": True}
                with self.assertRaisesRegex(
                    RuntimeError, "namespace is already consumed"
                ):
                    train_main(
                        (
                            "--config",
                            str(config_path),
                            "--en-model",
                            str(root / "en.lm"),
                            "--ru-model",
                            str(root / "ru.lm"),
                            "--artifact",
                            str(artifact),
                            "--manifest",
                            str(manifest),
                            "--test-report",
                            str(report),
                            "--build-environment",
                            str(root / "build-environment.json"),
                            "--workers",
                            "1",
                        )
                    )
                claim_gate.assert_called_once()
            self.assertEqual(exit_code, 1)
            self.assertNotIn(dataset.by_split["test"], featurized_splits)
            diagnostic = json.loads(cast(str, printer.call_args.args[0]))
            self.assertEqual(
                json.loads(diagnostic_path.read_text(encoding="utf-8")),
                diagnostic,
            )
            self.assertIs(diagnostic["sealed_test_evaluated"], False)
            self.assertEqual(diagnostic["phase"], "presealed_candidate")
            self.assertIs(
                diagnostic["presealed_candidate_gate_breakdown"]["passed"],
                False,
            )
            self.assertIs(
                diagnostic["threshold_selection_gate_breakdown"][
                    "context_stress"
                ]["passed"],
                False,
            )
            self.assertFalse(artifact.exists())
            self.assertFalse(manifest.exists())
            self.assertFalse(report.exists())
            self.assertFalse(tuple(root.glob(".presealed-diagnostic.json.*.staged")))

    def test_offline_file_reads_are_explicitly_bounded(self) -> None:
        class RecordingReader:
            def __init__(self, data: bytes) -> None:
                self.data = data
                self.requested = 0

            def __enter__(self) -> RecordingReader:
                return self

            def __exit__(
                self,
                _exception_type: object,
                _exception: object,
                _traceback: object,
            ) -> None:
                return None

            def read(self, size: int) -> bytes:
                self.requested = size
                return self.data

        expected = FrozenSourceFile(
            "sample.lm",
            hashlib.sha256(b"x").hexdigest(),
            1,
        )
        source_reader = RecordingReader(b"x")
        with patch.object(Path, "open", return_value=source_reader):
            self.assertEqual(
                tim.read_verified_frozen_file(
                    Path("sample.lm"),
                    expected,
                    label="sample",
                ),
                b"x",
            )
        self.assertEqual(source_reader.requested, expected.bytes + 1)

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            oversized_config = root / "oversized.json"
            oversized_config.write_bytes(b"12345")
            with patch.object(tim, "MAX_TRAINING_CONFIG_BYTES", INTENT_PATCHED_BYTE_LIMIT):
                with self.assertRaisesRegex(ValueError, "exceeds"):
                    load_training_config_snapshot(oversized_config)

            destination = root / "existing.ksm"
            destination.write_bytes(b"12345")
            with patch.object(tim, "MAX_PUBLICATION_BACKUP_BYTES", INTENT_PATCHED_BYTE_LIMIT):
                with self.assertRaisesRegex(ValueError, "rollback limit"):
                    publish_bytes_bundle(((destination, b"new"),))
            self.assertEqual(destination.read_bytes(), b"12345")

            oversized_manifest = root / "manifest.json"
            oversized_manifest.write_bytes(b"12345")
            with patch.object(eim, "MAX_EXTERNAL_MANIFEST_BYTES", INTENT_PATCHED_BYTE_LIMIT):
                with self.assertRaisesRegex(ValueError, "model manifest exceeds"):
                    eim._json_object(oversized_manifest)

            dictionary = root / "test.dic"
            affix = root / "test.aff"
            dictionary.write_bytes(b"12345")
            affix.write_bytes(b"SET UTF-8\n")
            model = cast(
                LanguageModel,
                SimpleNamespace(
                    speller=SimpleNamespace(
                        available=True,
                        source=str(dictionary),
                    )
                ),
            )
            with patch.object(eim, "MAX_HUNSPELL_DICTIONARY_BYTES", INTENT_PATCHED_BYTE_LIMIT):
                with self.assertRaisesRegex(ValueError, "dictionary exceeds"):
                    eim._hunspell_dictionary_words(model)

    def test_output_paths_cannot_alias_each_other_or_inputs(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            inputs = tuple(
                root / name
                for name in ("config", "en", "ru", "license", "hard-negative")
            )
            for path in inputs:
                path.write_bytes(path.name.encode())
            artifact = root / "model.ksm"
            manifest = root / "manifest.json"
            report = root / "report.json"
            diagnostic = root / "diagnostic.json"
            seal_registry = root / "seal-registry.json"
            seal_outcome = root / "seal-outcome.json"
            build_environment = root / "build-environment.json"
            keyword_arguments = {
                "config": inputs[0],
                "english": inputs[1],
                "russian": inputs[INTENT_RUSSIAN_INPUT_INDEX],
                "license_evidence": inputs[INTENT_LICENSE_INPUT_INDEX],
                "hard_negative_source": inputs[INTENT_HARD_NEGATIVE_INPUT_INDEX],
                "seal_registry": seal_registry,
                "seal_outcome": seal_outcome,
                "artifact": artifact,
                "manifest": manifest,
                "report": report,
                "build_environment": build_environment,
                "diagnostic": diagnostic,
            }
            validate_training_paths(**keyword_arguments)
            with self.assertRaisesRegex(ValueError, "must be distinct"):
                validate_training_paths(
                    **{
                        **keyword_arguments,
                        "manifest": artifact,
                    }
                )
            with self.assertRaisesRegex(ValueError, "must be distinct"):
                validate_training_paths(
                    **{
                        **keyword_arguments,
                        "seal_registry": manifest,
                    }
                )
            with self.assertRaisesRegex(ValueError, "must be distinct"):
                validate_training_paths(
                    **{
                        **keyword_arguments,
                        "diagnostic": report,
                    }
                )
            # The outcome ledger and the environment sidecar joined the set of
            # mutable paths in v21; neither may alias anything else.
            with self.assertRaisesRegex(ValueError, "must be distinct"):
                validate_training_paths(
                    **{
                        **keyword_arguments,
                        "seal_outcome": seal_registry,
                    }
                )
            with self.assertRaisesRegex(ValueError, "must be distinct"):
                validate_training_paths(
                    **{
                        **keyword_arguments,
                        "build_environment": manifest,
                    }
                )
            with self.assertRaisesRegex(ValueError, "immutable training input"):
                validate_training_paths(
                    **{
                        **keyword_arguments,
                        "build_environment": ENVIRONMENT_PROBE_PATH,
                    }
                )
            with self.assertRaisesRegex(ValueError, "immutable training input"):
                validate_training_paths(
                    **{
                        **keyword_arguments,
                        "artifact": inputs[0],
                    }
                )
            hardlink = root / "hardlink.ksm"
            os.link(inputs[0], hardlink)
            with self.assertRaisesRegex(ValueError, "immutable training input"):
                validate_training_paths(
                    **{
                        **keyword_arguments,
                        "artifact": hardlink,
                    }
                )

    def test_bundle_publication_is_atomic_and_rolls_back_each_failure(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            report = root / "report.json"
            artifact = root / "model.ksm"
            manifest = root / "manifest.json"
            outputs = (
                (report, b"new-report"),
                (artifact, b"new-artifact"),
                (manifest, b"new-manifest"),
            )
            previous = {
                report: b"old-report",
                artifact: b"old-artifact",
                manifest: b"old-manifest",
            }
            for path, data in previous.items():
                path.write_bytes(data)

            real_replace = os.replace
            for failed_destination in (report, artifact, manifest):
                with self.subTest(failed_destination=failed_destination.name):
                    for path, data in previous.items():
                        path.write_bytes(data)
                    failed = False

                    def flaky_replace(
                        source: str | bytes | os.PathLike[str] | os.PathLike[bytes],
                        destination: str | bytes | os.PathLike[str] | os.PathLike[bytes],
                    ) -> None:
                        nonlocal failed
                        destination_path = Path(
                            cast(str | os.PathLike[str], destination)
                        )
                        if destination_path == failed_destination and not failed:
                            failed = True
                            raise OSError("injected publication failure")
                        real_replace(source, destination)

                    with patch(
                        "train_intent_model.os.replace",
                        side_effect=flaky_replace,
                    ):
                        with self.assertRaisesRegex(
                            OSError, "injected publication failure"
                        ):
                            publish_bytes_bundle(outputs)
                    self.assertEqual(
                        {path: path.read_bytes() for path in previous},
                        previous,
                    )
                    self.assertFalse(tuple(root.glob("*.staged")))
                    self.assertFalse(tuple(root.glob(".*.staged")))

            publish_bytes_bundle(outputs)
            self.assertEqual(
                {path: path.read_bytes() for path, _data in outputs},
                {path: data for path, data in outputs},
            )
            with self.assertRaisesRegex(ValueError, "non-empty and unique"):
                publish_bytes_bundle(())
            with self.assertRaisesRegex(ValueError, "non-empty and unique"):
                publish_bytes_bundle(((artifact, b"a"), (artifact, b"b")))

    def test_frozen_sources_and_toolchain_snapshots_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            sample = root / "sample.lm"
            sample.write_bytes(b"x")
            self.assertEqual(
                sha256_file(sample, maximum_bytes=1),
                hashlib.sha256(b"x").hexdigest(),
            )
            with self.assertRaisesRegex(ValueError, "must be positive"):
                sha256_file(sample, maximum_bytes=0)
            sample.write_bytes(b"xy")
            with self.assertRaisesRegex(RuntimeError, "hashing size limit"):
                sha256_file(sample, maximum_bytes=1)
            sample.write_bytes(b"x")
            expected = FrozenSourceFile(
                "sample.lm", hashlib.sha256(b"x").hexdigest(), 1
            )
            verify_frozen_file(sample, expected, label="sample")
            sample.write_bytes(b"xy")
            with self.assertRaisesRegex(RuntimeError, "size mismatch"):
                verify_frozen_file(sample, expected, label="sample")
            sample.write_bytes(b"y")
            with self.assertRaisesRegex(RuntimeError, "SHA-256 mismatch"):
                verify_frozen_file(sample, expected, label="sample")

            repository_config = (
                Path(__file__).resolve().parents[1]
                / "model/intent_v1/config.json"
            )
            config_copy = root / "config.json"
            config_copy.write_bytes(repository_config.read_bytes())
            loaded, digest = load_training_config_snapshot(config_copy)
            self.assertEqual(loaded.dimension, EXPECTED_INTENT_PRODUCTION_MODEL_DIMENSION)
            self.assertEqual(
                digest, hashlib.sha256(config_copy.read_bytes()).hexdigest()
            )
            snapshot = capture_toolchain_snapshot(digest)
            verify_toolchain_snapshot(snapshot, config_copy)
            config_copy.write_bytes(config_copy.read_bytes() + b"\n")
            with self.assertRaisesRegex(RuntimeError, "config_sha256"):
                verify_toolchain_snapshot(snapshot, config_copy)

            invalid_utf8 = root / "invalid.json"
            invalid_utf8.write_bytes(b"\xff")
            with self.assertRaisesRegex(ValueError, "UTF-8"):
                load_training_config_snapshot(invalid_utf8)

    def test_license_evidence_is_verified_after_hashes(self) -> None:
        required = (
            "Files: models/*\n"
            "Copyright: 2013, 2014, marmuta <marmvta@gmail.com>\n"
            "  2011, 2012, Francesco Fumanti <francesco.fumanti@gmx.net>\n"
            "License: GPL-3+\n"
        ).encode()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            english = root / "en.lm"
            russian = root / "ru.lm"
            evidence = root / "copyright"
            english.write_bytes(b"en")
            russian.write_bytes(b"ru")
            evidence.write_bytes(required)

            def frozen(path: Path) -> FrozenSourceFile:
                raw = path.read_bytes()
                return FrozenSourceFile(
                    path.name, hashlib.sha256(raw).hexdigest(), len(raw)
                )

            sources = TrainingSources(
                package="onboard-data",
                package_version="test",
                license_declaration="GPL-3+",
                license_evidence=frozen(evidence),
                english=FrozenLanguageSource(
                    english.name,
                    frozen(english).sha256,
                    english.stat().st_size,
                    0,
                ),
                russian=FrozenLanguageSource(
                    russian.name,
                    frozen(russian).sha256,
                    russian.stat().st_size,
                    1,
                ),
            )
            with patch("train_intent_model.PROJECT_ROOT", root):
                self.assertEqual(
                    verify_training_sources(
                        config(sources=sources), english, russian
                    ),
                    evidence,
                )

                bad_evidence = b"Files: models/*\nLicense: GPL-3+\n"
                evidence.write_bytes(bad_evidence)
                bad_sources = replace(
                    sources,
                    license_evidence=FrozenSourceFile(
                        evidence.name,
                        hashlib.sha256(bad_evidence).hexdigest(),
                        len(bad_evidence),
                    ),
                )
                with self.assertRaisesRegex(RuntimeError, "missing required lines"):
                    verify_training_sources(
                        config(sources=bad_sources), english, russian
                    )

    def test_quantization_support_roundtrip_and_deterministic_writer(self) -> None:
        quantized = quantize_weights(INTENT_SEALED_CANDIDATE_QUANTIZE_WEIGHTS, INTENT_TEST_DIMENSION)
        self.assertEqual(len(quantized.values), INTENT_TEST_DIMENSION)
        self.assertTrue(quantized.support[0] & (1 << INTENT_QUANTIZED_FIRST_WEIGHT_INDEX))
        self.assertTrue(quantized.support[1] & (1 << 1))
        self.assertLessEqual(
            quantized.maximum_absolute_error, quantized.scale / INTENT_ARTIFACT_SCALE_HALVING_DIVISOR
        )
        scorer = QuantizedLinearScorer(quantized, INTENT_ARTIFACT_QUANTIZED_SCORER_BIAS)
        self.assertGreater(
            scorer.score(((INTENT_QUANTIZED_FIRST_WEIGHT_INDEX, 1.0),)), INTENT_ARTIFACT_QUANTIZED_SCORER_BIAS
        )

        threshold_logits = {trigger: INTENT_ARTIFACT_QUANTIZATION_TEST_THRESHOLD_LOGIT for trigger in TRIGGERS}
        with tempfile.TemporaryDirectory() as temporary:
            first_path = Path(temporary) / "first.ksm"
            second_path = Path(temporary) / "second.ksm"
            keyword_arguments = {
                "model_version": "test-intent-v1",
                "dimension": INTENT_TEST_DIMENSION,
                "weights": quantized.dequantized(),
                "supported_fingerprints": {
                    INTENT_QUANTIZED_FIRST_WEIGHT_INDEX, INTENT_QUANTIZED_SECOND_WEIGHT_INDEX
                },
                "threshold_logits": runtime_threshold_logits(
                    threshold_logits
                ),
                "veto_threshold": -INTENT_ARTIFACT_QUANTIZATION_VETO_THRESHOLD,
                "bias": INTENT_ARTIFACT_QUANTIZED_SCORER_BIAS,
                "platt_calibration": {
                    direction: PlattParameters(
                        INTENT_ARTIFACT_QUANTIZATION_PLATT_SCALE, -INTENT_ARTIFACT_QUANTIZATION_PLATT_BIAS_MAGNITUDE
                    )
                    for direction in ("0>1", "1>0")
                },
                "fnv_seed": DEFAULT_FNV_SEED,
                "ngram_orders": NGRAM_ORDERS,
                "metadata": {"calibration_scope": "lexical-synthetic-not-real-world-probability"},
            }
            first = write_model(first_path, **keyword_arguments)  # type: ignore[arg-type]
            second = write_model(second_path, **keyword_arguments)  # type: ignore[arg-type]
            self.assertEqual(first_path.read_bytes(), second_path.read_bytes())
            loaded = LinearNgramModel.load(first_path)
            self.assertEqual(loaded.checksum, first.checksum)
            self.assertEqual(loaded.checksum, second.checksum)
            strong = ConfusionMatrix(
                INTENT_ARTIFACT_QUANTIZATION_TEST_METRIC_COUNT, 0, INTENT_ARTIFACT_QUANTIZATION_TEST_METRIC_COUNT, 0
            )
            selections = {
                trigger: ThresholdSelection(
                    trigger, threshold_logits[trigger], strong, strong
                )
                for trigger in TRIGGERS
            }
            expected_candidate_parameters = training_candidate_model_parameters(
                config=config(),
                quantized=quantized,
                supported_fingerprints={
                    INTENT_QUANTIZED_FIRST_WEIGHT_INDEX, INTENT_QUANTIZED_SECOND_WEIGHT_INDEX
                },
                bias=INTENT_ARTIFACT_QUANTIZED_SCORER_BIAS,
                calibration=directional_calibration(
                    INTENT_ARTIFACT_QUANTIZATION_PLATT_SCALE, -INTENT_ARTIFACT_QUANTIZATION_PLATT_BIAS_MAGNITUDE
                ),
                thresholds=selections,
                veto=VetoSelection(
                    -INTENT_ARTIFACT_QUANTIZATION_VETO_THRESHOLD, INTENT_ARTIFACT_QUANTIZATION_TEST_METRIC_COUNT, 0, 0.0
                ),
            )
            self.assertEqual(
                runtime_candidate_model_parameters(loaded),
                expected_candidate_parameters,
            )
            model_input = intent_input_for_example(
                lexical_example(True),
                scorers=test_word_scorers(),
            )
            prediction = loaded.predict(model_input)
            feature_vector = extract_features(
                model_input,
                dimension=quantized.dimension,
                hash_seed=DEFAULT_FNV_SEED,
                membership_seed=DEFAULT_MEMBERSHIP_FNV_SEED,
                ngram_orders=NGRAM_ORDERS,
            )
            training_logit = scorer.score(feature_vector.values)
            self.assertEqual(prediction.logit.hex(), training_logit.hex())
            self.assertEqual(
                prediction.probability.hex(),
                stable_sigmoid(
                    (INTENT_ARTIFACT_QUANTIZATION_PLATT_SCALE * training_logit)
                    - INTENT_ARTIFACT_QUANTIZATION_PLATT_BIAS_MAGNITUDE
                ).hex(),
            )
            self.assertEqual(
                prediction.should_switch,
                (
                    (INTENT_ARTIFACT_QUANTIZATION_PLATT_SCALE * training_logit)
                    - INTENT_ARTIFACT_QUANTIZATION_PLATT_BIAS_MAGNITUDE
                )
                >= threshold_logits["space"],
            )
            self.assertTrue(math.isfinite(prediction.logit))

    def test_config_schema_and_statistical_upper_bound(self) -> None:
        repository = Path(__file__).resolve().parents[1]
        config_path = repository / "model/intent_v1/config.json"
        loaded = load_training_config(config_path)
        self.assertEqual(loaded.schema_version, EXPECTED_INTENT_CONFIG_SCHEMA_VERSION)
        self.assertEqual(loaded.dimension, EXPECTED_INTENT_PRODUCTION_MODEL_DIMENSION)
        self.assertEqual(loaded.minimum_word_length, EXPECTED_INTENT_CONFIG_MINIMUM_WORD_LENGTH)
        self.assertEqual(loaded.maximum_words_per_language, 0)
        self.assertEqual(
            loaded.external_evaluation.trigger_expansion,
            TRIGGERS,
        )
        self.assertEqual(
            loaded.external_evaluation.minimum_words_per_group,
            EXPECTED_INTENT_CONFIG_MINIMUM_WORDS_PER_GROUP,
        )
        self.assertEqual(
            loaded.external_evaluation.english.dictionary_sha256,
            "829a043cf078d1e80e886289a13823454977f442a239a859d2133ea61944aa60",
        )
        self.assertEqual(
            loaded.external_evaluation.lexical_disjoint_corpus_sha256,
            "57c83b5a8005ebdaf3b676ef381cd4b4f59a045fe64e52c47f3295a2738f1a7f",
        )
        self.assertEqual(
            loaded.external_evaluation.unknown_typo_development_corpus_sha256,
            "0a84144da4259e43bf645c3c3d6a6b39a999e8d1a82aabe8b0a9b1a03d47e8c2",
        )
        self.assertEqual(
            loaded.external_evaluation.unknown_typo_holdout_corpus_sha256,
            "806bb9a5d1711bca0d5957cd8c119b3c5fa504068b95c5cc65655b6e02b0d75c",
        )
        self.assertEqual(
            loaded.sealed_evaluation.split_namespace, SPLIT_NAMESPACE
        )
        self.assertEqual(
            loaded.sealed_evaluation.registry_path,
            "model/intent_v1/seal-registry-v23.json",
        )
        self.assertEqual(
            loaded.hard_negative_development.source.path,
            "model/intent_v1/unknown-typo-development-v23.json",
        )
        self.assertEqual(
            loaded.hard_negative_development.source.sha256,
            "95ec8affaf49392862ab982e38e3b16cad67256423768a7dcb9e20940527dce8",
        )
        self.assertEqual(
            loaded.hard_negative_development.role_counts(),
            EXPECTED_INTENT_PRODUCTION_ROLE_COUNTS,
        )
        self.assertEqual(
            loaded.hard_negative_development.training_example_weight,
            EXPECTED_INTENT_PRODUCTION_TRAINING_EXAMPLE_WEIGHT,
        )
        self.assertEqual(
            loaded.selection_maximum_false_positives_per_trigger,
            0,
        )
        self.assertEqual(
            (
                loaded.selection_minimum_recall,
                loaded.selection_minimum_pause_recall,
                loaded.selection_minimum_typo_recall,
                loaded.selection_minimum_pause_typo_recall,
            ),
            EXPECTED_INTENT_PRODUCTION_SELECTION_RECALLS,
        )
        self.assertEqual(
            loaded.threshold_logit_margin_cap, EXPECTED_INTENT_PRODUCTION_LOGIT_MARGIN_CAP
        )
        preseal_path = repository / "model/intent_v1/holdout-v23-preseal.json"
        preseal_bytes = preseal_path.read_bytes()
        self.assertLessEqual(len(preseal_bytes), INTENT_MAX_PRESEAL_BYTES)
        preseal = cast(
            dict[str, object],
            json.loads(preseal_bytes.decode("utf-8")),
        )
        self.assertEqual(
            set(preseal),
            {
                "schema_version",
                "policy",
                "model_loaded",
                "metrics_evaluated",
                "development",
                "holdout",
                "sealed_dataset_exclusions",
                "combined_holdout_exclusions",
                "overlap_counts",
            },
        )
        self.assertEqual(preseal["schema_version"], 1)
        self.assertEqual(
            preseal["policy"],
            "keyswitch-intent-v23-preseal-holdout",
        )
        self.assertIs(preseal["model_loaded"], False)
        self.assertIs(preseal["metrics_evaluated"], False)
        development = cast(Mapping[str, object], preseal["development"])
        holdout = cast(Mapping[str, object], preseal["holdout"])
        sealed_exclusions = cast(
            Mapping[str, object],
            preseal["sealed_dataset_exclusions"],
        )
        combined_exclusions = cast(
            Mapping[str, object],
            preseal["combined_holdout_exclusions"],
        )
        overlaps = cast(Mapping[str, object], preseal["overlap_counts"])
        self.assertEqual(
            development,
            {
                "role": "hard-negative-presealed-roles",
                "rank_namespace": UNKNOWN_TYPO_DEVELOPMENT_RANK_NAMESPACE,
                "choice_namespace": UNKNOWN_TYPO_DEVELOPMENT_CHOICE_NAMESPACE,
                "corpus_sha256": (
                    loaded.external_evaluation
                    .unknown_typo_development_corpus_sha256
                ),
                "signature_count": EXPECTED_HARD_NEGATIVE_PRESEAL_SIGNATURE_COUNT,
                "words_by_group": EXPECTED_PRESEAL_WORDS_BY_GROUP,
                "frozen_source": {
                    "path": loaded.hard_negative_development.source.path,
                    "sha256": loaded.hard_negative_development.source.sha256,
                    "bytes": loaded.hard_negative_development.source.bytes,
                },
                "role_namespace": HARD_NEGATIVE_ROLE_NAMESPACE,
                "role_words_by_group": EXPECTED_PRESEAL_ROLE_WORDS_BY_GROUP,
                "examples_by_role": EXPECTED_PRESEAL_EXAMPLES_BY_ROLE,
                "training_example_weight": EXPECTED_INTENT_PRODUCTION_TRAINING_EXAMPLE_WEIGHT,
            },
        )
        self.assertEqual(
            holdout,
            {
                "role": "independent-release-holdout",
                "rank_namespace": UNKNOWN_TYPO_HOLDOUT_RANK_NAMESPACE,
                "choice_namespace": UNKNOWN_TYPO_HOLDOUT_CHOICE_NAMESPACE,
                "corpus_sha256": (
                    loaded.external_evaluation
                    .unknown_typo_holdout_corpus_sha256
                ),
                "signature_count": EXPECTED_HARD_NEGATIVE_PRESEAL_SIGNATURE_COUNT,
                "words_by_group": EXPECTED_PRESEAL_WORDS_BY_GROUP,
            },
        )
        self.assertEqual(
            sealed_exclusions,
            {
                "signature_count": EXPECTED_SEALED_EXCLUSION_SIGNATURE_COUNT,
                "sha256": (
                    "5d8d55c6277901a33cc72c5a3a526e33deff2b154e15e816eb8889161b1079fa"
                ),
            },
        )
        self.assertEqual(
            combined_exclusions,
            {
                "signature_count": EXPECTED_COMBINED_EXCLUSION_SIGNATURE_COUNT,
                "sha256": (
                    "99553887249602e5fee72bac59491b89be58882f383820e9cc0a80cd5b0e8204"
                ),
            },
        )
        self.assertEqual(
            overlaps,
            {"development_holdout": 0, "sealed_holdout": 0},
        )
        self.assertAlmostEqual(
            wilson_upper_bound(0, WILSON_BOUND_TEST_SAMPLE_SIZE),
            EXPECTED_WILSON_BOUND_AT_ZERO_FALSE_POSITIVES,
            places=WILSON_BOUND_COMPARISON_PLACES,
        )
        self.assertGreater(
            wilson_upper_bound(WILSON_BOUND_TEST_FALSE_POSITIVES, WILSON_BOUND_TEST_SAMPLE_SIZE),
            0.0,
        )
        self.assertEqual(wilson_upper_bound(0, 0), 1.0)
        with self.assertRaisesRegex(ValueError, "binomial"):
            wilson_upper_bound(WILSON_BOUND_TEST_FALSE_POSITIVES, 1)
        for invalid_z_score in (0.0, -1.0, math.inf, math.nan):
            with self.subTest(z_score=invalid_z_score):
                with self.assertRaisesRegex(ValueError, "z_score"):
                    wilson_upper_bound(0, WILSON_BOUND_TEST_SAMPLE_SIZE, invalid_z_score)

        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "bad.json"
            path.write_text(json.dumps({"schema_version": 1}), encoding="utf-8")
            with self.assertRaises(ValueError):
                load_training_config(path)
            duplicate_path = Path(temporary) / "duplicate.json"
            duplicate_path.write_text(
                '{"schema_version":3,"schema_version":3}',
                encoding="utf-8",
            )
            with self.assertRaisesRegex(ValueError, "duplicate key"):
                load_training_config(duplicate_path)

            baseline = cast(
                dict[str, object], json.loads(config_path.read_text(encoding="utf-8"))
            )
            invalid_cases: tuple[tuple[str, dict[str, object], str], ...] = (
                (
                    "old-schema",
                    {**baseline, "schema_version": INTENT_UNSUPPORTED_CONFIG_SCHEMA_VERSION},
                    "unsupported training config schema",
                ),
                (
                    "missing-policy",
                    {
                        key: value
                        for key, value in baseline.items()
                        if key != "external_evaluation"
                    },
                    "fields mismatch",
                ),
                (
                    "missing-sealed-policy",
                    {
                        key: value
                        for key, value in baseline.items()
                        if key != "sealed_evaluation"
                    },
                    "fields mismatch",
                ),
                (
                    "pre-split-truncation",
                    {
                        **baseline,
                        "dataset": {
                            **cast(dict[str, object], baseline["dataset"]),
                            "maximum_words_per_language": 1,
                        },
                    },
                    "maximum_words_per_language must be zero",
                ),
            )
            for name, payload, message in invalid_cases:
                invalid_path = Path(temporary) / f"{name}.json"
                invalid_path.write_text(
                    json.dumps(payload),
                    encoding="utf-8",
                )
                with self.assertRaisesRegex(ValueError, message):
                    load_training_config(invalid_path)

            sealed_policy = cast(
                dict[str, object], baseline["sealed_evaluation"]
            )
            sealed_mutations: tuple[tuple[str, object, str], ...] = (
                (
                    "schema_version",
                    INTENT_UNSUPPORTED_CONFIG_SCHEMA_VERSION,
                    "unsupported sealed_evaluation schema",
                ),
                (
                    "split_namespace",
                    "keyswitch:intent-v999:physical-signature",
                    "must match the trainer split namespace",
                ),
                (
                    "registry_path",
                    "../seal.json",
                    "must be a repository-relative JSON path",
                ),
                (
                    "registry_path",
                    "model/intent_v1/alternate-v2.json",
                    "must match the versioned trainer registry path",
                ),
            )
            for key, value, message in sealed_mutations:
                invalid_path = Path(temporary) / f"bad-sealed-{key}.json"
                invalid_path.write_text(
                    json.dumps(
                        {
                            **baseline,
                            "sealed_evaluation": {
                                **sealed_policy,
                                key: value,
                            },
                        }
                    ),
                    encoding="utf-8",
                )
                with self.assertRaisesRegex(ValueError, message):
                    load_training_config(invalid_path)

            policy = cast(
                dict[str, object], baseline["external_evaluation"]
            )
            policy_mutations: tuple[tuple[str, object, str], ...] = (
                (
                    "minimum_words_per_group",
                    INTENT_UNDERSIZED_CONFIG_MINIMUM_WORDS_PER_GROUP,
                    "at least 5000 words",
                ),
                (
                    "trigger_expansion",
                    list(reversed(TRIGGERS)),
                    "exactly match the runtime trigger order",
                ),
                (
                    "lexical_disjoint_corpus_sha256",
                    "not-a-digest",
                    "exact lowercase SHA-256",
                ),
            )
            for key, value, message in policy_mutations:
                mutated_policy = {**policy, key: value}
                invalid_path = Path(temporary) / f"bad-{key}.json"
                invalid_path.write_text(
                    json.dumps(
                        {**baseline, "external_evaluation": mutated_policy}
                    ),
                    encoding="utf-8",
                )
                with self.assertRaisesRegex(ValueError, message):
                    load_training_config(invalid_path)


class NativeFTRLKernelTests(unittest.TestCase):
    """The compiled epoch kernel must be indistinguishable from the reference."""


    def _rows(self, count: int, seed: int) -> tuple[FeaturedExample, ...]:
        rng = random.Random(seed)
        rows: list[FeaturedExample] = []
        for position in range(count):
            width = rng.randint(1, FTRL_KERNEL_MAX_ROW_FEATURE_WIDTH)
            features = tuple(
                (
                    index,
                    rng.choice((
                        1.0,
                        FTRL_KERNEL_FEATURE_VALUE_SMALL,
                        FTRL_KERNEL_FEATURE_VALUE_LARGE,
                        -FTRL_KERNEL_FEATURE_VALUE_NEGATIVE_SMALL,
                        rng.uniform(-FTRL_KERNEL_RANDOM_FEATURE_MAGNITUDE, FTRL_KERNEL_RANDOM_FEATURE_MAGNITUDE),
                    )),
                )
                for index in sorted(rng.sample(range(FTRL_KERNEL_DIMENSION), width))
            )
            direction: LayoutDirection = (
                "0>1" if position % FTRL_KERNEL_POSITION_PARITY_MODULUS == 0 else "1>0"
            )
            example = replace(
                lexical_example(
                    rng.random() < FTRL_KERNEL_LABEL_PROBABILITY_THRESHOLD,
                    signature=f"native-{seed}-{position}",
                    direction=direction,
                ),
                weight=rng.choice((
                    1.0,
                    FTRL_KERNEL_WEIGHT_VALUE_LARGE,
                    rng.uniform(FTRL_KERNEL_WEIGHT_UNIFORM_LOWER_BOUND, FTRL_KERNEL_WEIGHT_VALUE_LARGE),
                )),
            )
            rows.append(FeaturedExample(example, features))
        return tuple(rows)

    @staticmethod
    def _same_state(left: FTRLProximal, right: FTRLProximal) -> bool:
        pack = struct.Struct("<d").pack
        return (
            set(left.z) == set(right.z)
            and set(left.n) == set(right.n)
            and all(pack(value) == pack(right.z[index]) for index, value in left.z.items())
            and all(pack(value) == pack(right.n[index]) for index, value in left.n.items())
            and pack(left.bias_z) == pack(right.bias_z)
            and pack(left.bias_n) == pack(right.bias_n)
        )

    def _kernel(self) -> tim.NativeFTRLKernel:
        kernel = tim.NativeFTRLKernel.load()
        if kernel is None:
            self.skipTest("no C compiler for the native FTRL kernel")
        return kernel

    def test_native_epochs_are_bit_exact_against_the_reference_update(self) -> None:
        kernel = self._kernel()
        rows = self._rows(FTRL_KERNEL_NATIVE_TEST_ROW_COUNT, FTRL_KERNEL_NATIVE_TEST_SEED)
        parameters = FTRLParameters(
            FTRL_KERNEL_DIMENSION, FTRL_KERNEL_NATIVE_TEST_ALPHA, 1.0, FTRL_KERNEL_NATIVE_TEST_L1, FTRL_KERNEL_NATIVE_TEST_L2
        )
        packed = tim.pack_training_rows(rows, FTRL_KERNEL_DIMENSION)
        reference = FTRLProximal(parameters)
        candidate = FTRLProximal(parameters)
        order = list(range(len(rows)))
        for epoch in range(1, FTRL_KERNEL_NATIVE_TEST_EPOCH_RANGE_END):
            random.Random(epoch).shuffle(order)
            for index in order:
                item = rows[index]
                reference.update(item.features, item.example.label, item.example.weight)
            kernel.run_epoch(candidate, packed, order)
            self.assertTrue(self._same_state(reference, candidate), f"epoch {epoch}")
        self.assertEqual(reference.sparse_weights(), candidate.sparse_weights())
        self.assertEqual(reference.nonzero_weight_count(), candidate.nonzero_weight_count())

    def test_zero_regularisation_and_partial_orders_stay_bit_exact(self) -> None:
        kernel = self._kernel()
        rows = self._rows(FTRL_KERNEL_PARTIAL_ORDER_ROW_COUNT, FTRL_KERNEL_PARTIAL_ORDER_SEED)
        parameters = FTRLParameters(FTRL_KERNEL_DIMENSION, FTRL_KERNEL_PARTIAL_ORDER_ALPHA, 0.0, 0.0, 0.0)
        packed = tim.pack_training_rows(rows, FTRL_KERNEL_DIMENSION)
        reference = FTRLProximal(parameters)
        candidate = FTRLProximal(parameters)
        order = [
            index
            for index in range(len(rows))
            if index % FTRL_KERNEL_PARTIAL_ORDER_SKIP_MODULUS != 0
        ]
        for index in order:
            item = rows[index]
            reference.update(item.features, item.example.label, item.example.weight)
        kernel.run_epoch(candidate, packed, order)
        self.assertTrue(self._same_state(reference, candidate))

    def test_self_check_accepts_the_real_kernel_and_rejects_a_divergent_one(self) -> None:
        kernel = self._kernel()
        rows = self._rows(FTRL_KERNEL_DIVERGENCE_TEST_ROW_COUNT, FTRL_KERNEL_DIVERGENCE_TEST_SEED)
        parameters = FTRLParameters(
            FTRL_KERNEL_DIMENSION, FTRL_KERNEL_NATIVE_TEST_ALPHA, 1.0, FTRL_KERNEL_NATIVE_TEST_L1, FTRL_KERNEL_NATIVE_TEST_L2
        )
        packed = tim.pack_training_rows(rows, FTRL_KERNEL_DIMENSION)
        tim.verify_native_ftrl_kernel(kernel, packed, rows, parameters, range(len(rows)))

        class DivergentKernel(tim.NativeFTRLKernel):
            def __init__(self) -> None:  # noqa: D401 - test double
                self.library_path = kernel.library_path
                self._function = kernel._function

            def run_epoch(
                self,
                model: FTRLProximal,
                packed_rows: tim.PackedTrainingRows,
                order: Sequence[int],
            ) -> None:
                super().run_epoch(model, packed_rows, order)
                first = min(model.z)
                model.z[first] = math.nextafter(model.z[first], math.inf)

        with self.assertRaisesRegex(RuntimeError, "diverged in z"):
            tim.verify_native_ftrl_kernel(
                DivergentKernel(), packed, rows, parameters, range(len(rows))
            )

    def test_pack_training_rows_validates_like_update(self) -> None:
        good = self._rows(FTRL_KERNEL_PACK_VALIDATION_ROW_COUNT, 1)
        bad_weight = FeaturedExample(replace(good[0].example, weight=0.0), good[0].features)
        with self.assertRaisesRegex(ValueError, "sample weight"):
            tim.pack_training_rows((bad_weight,), FTRL_KERNEL_DIMENSION)
        unordered = FeaturedExample(
            good[0].example,
            ((FTRL_KERNEL_DUPLICATE_FEATURE_INDEX, 1.0), (FTRL_KERNEL_DUPLICATE_FEATURE_INDEX, 1.0)),
        )
        with self.assertRaisesRegex(ValueError, "strictly increasing"):
            tim.pack_training_rows((unordered,), FTRL_KERNEL_DIMENSION)
        outside = FeaturedExample(good[0].example, ((FTRL_KERNEL_DIMENSION, 1.0),))
        with self.assertRaisesRegex(ValueError, "outside model dimension"):
            tim.pack_training_rows((outside,), FTRL_KERNEL_DIMENSION)
        packed = tim.pack_training_rows(good, FTRL_KERNEL_DIMENSION)
        self.assertEqual(packed.row_count, FTRL_KERNEL_PACK_VALIDATION_ROW_COUNT)
        self.assertEqual(len(packed.indptr), FTRL_KERNEL_EXPECTED_INDPTR_LENGTH)
        self.assertEqual(len(packed.indices), sum(len(row.features) for row in good))

    def test_fit_ftrl_kernels_produce_identical_models(self) -> None:
        self._kernel()
        rows = self._rows(FTRL_KERNEL_COMPARISON_ROW_COUNT, FTRL_KERNEL_COMPARISON_SEED)
        development = self._rows(
            FTRL_KERNEL_COMPARISON_DEV_ROW_COUNT, FTRL_KERNEL_COMPARISON_DEV_SEED
        )
        settings = config(
            dimension=FTRL_KERNEL_DIMENSION,
            maximum_epochs=FTRL_KERNEL_COMPARISON_MAX_EPOCHS,
            minimum_epochs=1,
            patience=FTRL_KERNEL_COMPARISON_MAX_EPOCHS,
        )
        native = fit_ftrl(rows, development, settings, kernel="native")
        python = fit_ftrl(rows, development, settings, kernel="python")
        self.assertEqual(native.best_epoch, python.best_epoch)
        self.assertEqual(native.history, python.history)
        self.assertTrue(self._same_state(native.model, python.model))

    def test_missing_compiler_falls_back_to_python_unless_native_is_required(self) -> None:
        rows = self._rows(FTRL_KERNEL_FALLBACK_TEST_ROW_COUNT, FTRL_KERNEL_FALLBACK_TEST_SEED)
        development = self._rows(FTRL_KERNEL_FALLBACK_TEST_DEV_ROW_COUNT, FTRL_KERNEL_FALLBACK_TEST_DEV_SEED)
        settings = config(
            dimension=FTRL_KERNEL_DIMENSION, maximum_epochs=1, minimum_epochs=1, patience=1
        )
        with patch.object(shutil, "which", return_value=None):
            fallback = fit_ftrl(rows, development, settings, kernel="auto")
            with self.assertRaisesRegex(RuntimeError, "unavailable"):
                fit_ftrl(rows, development, settings, kernel="native")
        python = fit_ftrl(rows, development, settings, kernel="python")
        self.assertTrue(self._same_state(fallback.model, python.model))
        with self.assertRaisesRegex(ValueError, "unsupported FTRL kernel"):
            tim.ftrl_kernel_choice("gpu")
        self.assertEqual(tim.ftrl_kernel_choice("native"), "native")


class DeterministicIntentModel:
    """Picklable stand-in whose predictions depend only on the input."""


    veto_threshold = -INTENT_DETERMINISTIC_VETO_THRESHOLD
    dimension = INTENT_TEST_DIMENSION
    fnv_seed = DEFAULT_FNV_SEED
    membership_seed = DEFAULT_MEMBERSHIP_FNV_SEED

    def predict(self, item: IntentModelInput) -> LinearPrediction:
        logit = float(len(item.original) - len(item.alternative)) + INTENT_DETERMINISTIC_LOGIT_OFFSET
        return LinearPrediction(
            logit,
            stable_sigmoid(logit),
            0.0,
            INTENT_DETERMINISTIC_PREDICTION_COVERAGE,
            logit > 0.0,
            "deterministic",
        )


def _misordered_worker(bounds: tuple[int, int]) -> tuple[int, list[int]]:
    start, stop = bounds
    return start + 1, list(range(start, stop))


class ParallelRowScoringTests(unittest.TestCase):
    """Worker processes must reproduce sequential scoring row for row."""


    def setUp(self) -> None:
        pair = LayoutPair()
        words = ("hello", "world", "python", "layout", "switch", "typing", "kernel")
        rows: list[LexicalExample] = []
        for index, word in enumerate(words):
            rows.append(
                LexicalExample(
                    original=word,
                    alternative=pair.translate(word, "us", "ru"),
                    source_group=0,
                    target_group=1,
                    trigger="space",
                    label=index % INTENT_LABEL_PARITY_MODULUS == 0,
                    weight=1.0,
                    base_signature=f"parallel-{index}",
                    variant_kind="identity",
                    source_known=False,
                    target_known=False,
                )
            )
            russian = pair.translate(word, "us", "ru")
            rows.append(
                LexicalExample(
                    original=russian,
                    alternative=word,
                    source_group=1,
                    target_group=0,
                    trigger="pause",
                    label=index % INTENT_ROW_SCORING_SECONDARY_LABEL_MODULUS == 0,
                    weight=1.0,
                    base_signature=f"parallel-ru-{index}",
                    variant_kind="identity",
                    source_known=False,
                    target_known=False,
                )
            )
        self.rows = tuple(rows)
        self.language_models = cast(
            dict[int, LanguageModel],
            {
                0: EvaluationLanguageModel("en_US", {"hello", "world"}),
                1: EvaluationLanguageModel("ru_RU", {"руддщ"}),
            },
        )
        self.model = cast(LinearNgramModel, DeterministicIntentModel())

    def test_prediction_and_comparison_match_sequential_scoring(self) -> None:
        sequential = predict_model_examples(
            self.model, self.rows, scorers=self.language_models, workers=1
        )
        parallel = predict_model_examples(
            self.model, self.rows, scorers=self.language_models, workers=INTENT_ROW_SCORING_PARALLEL_TEST_WORKERS
        )
        self.assertEqual(sequential, parallel)
        self.assertEqual(
            compare_with_fallback(
                self.model, self.rows, 0, language_models=self.language_models, workers=1
            ),
            compare_with_fallback(
                self.model,
                self.rows,
                0,
                language_models=self.language_models,
                workers=INTENT_ROW_SCORING_PARALLEL_TEST_WORKERS,
            ),
        )

    def test_context_rows_match_sequential_scoring_and_replay_the_cache(self) -> None:
        scorers = {
            group: eim._CorpusLanguageScorerCache(model)
            for group, model in self.language_models.items()
        }
        sequential_cache = eim._ContextInvariantIntentModel(self.model)
        parallel_cache = eim._ContextInvariantIntentModel(self.model)
        for profile in eim.PRODUCTION_CONTEXT_PROFILES[: INTENT_ROW_SCORING_CONTEXT_PROFILE_SAMPLE_COUNT]:
            sequential = eim._evaluate_production_context_profile_rows(
                sequential_cache, self.rows, profile, language_models=scorers, workers=1
            )
            parallel = eim._evaluate_production_context_profile_rows(
                parallel_cache,
                self.rows,
                profile,
                language_models=scorers,
                workers=INTENT_ROW_SCORING_WIDE_PARALLEL_TEST_WORKERS,
            )
            self.assertEqual(sequential, parallel)
            self.assertEqual(
                (sequential_cache.unique_predictions, sequential_cache.cache_hits),
                (parallel_cache.unique_predictions, parallel_cache.cache_hits),
            )
        self.assertGreater(parallel_cache.cache_hits, 0)
        raw = eim._evaluate_production_context_profile_rows(
            self.model, self.rows, eim.PRODUCTION_CONTEXT_PROFILES[0],
            language_models=scorers, workers=INTENT_ROW_SCORING_MODERATE_PARALLEL_TEST_WORKERS,
        )
        self.assertEqual(
            raw,
            eim._evaluate_production_context_profile_rows(
                self.model, self.rows, eim.PRODUCTION_CONTEXT_PROFILES[0],
                language_models=scorers, workers=1,
            ),
        )

    def test_worker_bookkeeping_is_validated(self) -> None:
        with self.assertRaisesRegex(ValueError, "at least one worker"):
            eim.set_default_row_workers(0)
        with self.assertRaisesRegex(ValueError, "at least one worker"):
            eim._effective_row_workers(0, INTENT_ROW_SCORING_EFFECTIVE_ROW_WORKERS_CAP)
        self.assertEqual(
            eim._effective_row_workers(INTENT_ROW_SCORING_REQUESTED_ROW_WORKERS, INTENT_ROW_SCORING_AVAILABLE_ROW_WORKERS_CAP),
            INTENT_ROW_SCORING_AVAILABLE_ROW_WORKERS_CAP,
        )
        self.assertEqual(eim._effective_row_workers(None, INTENT_ROW_SCORING_AVAILABLE_ROW_WORKERS_CAP), 1)
        with self.assertRaisesRegex(RuntimeError, "not initialised"):
            eim._row_workload()
        with self.assertRaisesRegex(RuntimeError, "changed chunk order"):
            eim._map_row_chunks(
                _misordered_worker, eim._RowWorkload(examples=self.rows), INTENT_ROW_SCORING_MISORDERED_CHUNK_COUNT
            )

    def test_main_restores_the_default_worker_count(self) -> None:
        before = eim._DEFAULT_ROW_WORKERS
        with tempfile.TemporaryDirectory() as directory:
            missing = Path(directory) / "missing-config.json"
            with self.assertRaises(Exception):
                eim.main(["--config", str(missing), "--workers", "3"])
        self.assertEqual(eim._DEFAULT_ROW_WORKERS, before)


if __name__ == "__main__":
    unittest.main()
