"""Model fixtures of the tests: fake weights, dimensions, thresholds and the values tests pin."""

from __future__ import annotations

import math
from typing import Final

from keyswitch.constants.file_formats import KSLM_SCHEMA_VERSION

from .counts import (
    INTENT_OPTIMIZER_DELETION_TYPO_STRONG_COUNT,
    INTENT_OPTIMIZER_DELETION_TYPO_WEAK_COUNT,
    INTENT_OPTIMIZER_FALSE_POSITIVE_BOUND_FALSE_POSITIVES,
    INTENT_OPTIMIZER_FALSE_POSITIVE_BOUND_TOTAL_NEGATIVES,
    INTENT_OPTIMIZER_IDENTITY_TYPO_STRONG_COUNT,
    INTENT_OPTIMIZER_IDENTITY_TYPO_WEAK_COUNT,
)

# One past the only two valid context feature versions (context_model.FEATURE_VERSION and
# CONTEXT_ACTION_FEATURE_VERSION); used to probe both the payload and the constructor's own
# validation.
UNSUPPORTED_CONTEXT_FEATURE_VERSION: Final = 4
RECEIPT_ALTERED_PREFIX_FEATURE_VERSION: Final = 2
UNSUPPORTED_PREFIX_SCHEMA_VERSION: Final = 2
# An int where a str field is required, to prove the type check rejects it.
NON_STRING_FIELD_VALUE: Final = 3
ARBITRARY_INVALID_CONTEXT_FEATURE_VERSION: Final = 99
# The context model's fixed action/class count (labels 0..3); frozen context_optimizer.py and
# context_model.py name it only as len(ACTIONS).
CONTEXT_ACTION_CLASS_COUNT: Final = 4
# schema_version the loader does not understand.
UNSUPPORTED_IDENTIFIER_LEXICON_SCHEMA_VERSION: Final = 2
# model_bytes()'s own defaults, reused across most tests that only need *a* valid model.
INTENT_TEST_DIMENSION: Final = 256
# A second, smaller dimension reused across classes wherever a full-size model would just add noise.
INTENT_SMALL_TEST_DIMENSION: Final = 64
# A larger dimension for tests that want more distinct buckets to work with.
INTENT_LARGE_TEST_DIMENSION: Final = 1024
# intent_model.NGRAM_ORDERS is (1, 2, 3, 4, 5); this fixture is missing 1 and 5 and recurs wherever
# a test needs "some ngram_orders, but not the real ones".
INCOMPLETE_INTENT_NGRAM_ORDERS: Final = (2, 3, 4)
# encode_test_case()'s default dimension, small enough to keep malformed-input fixtures cheap.
INTENT_MINI_TEST_DIMENSION: Final = 8
# Indices within the tuple intent_model.HEADER.unpack_from() returns (magic, schema, flags,
# manifest_length, payload_length, crc, digest).
KSLM_HEADER_MANIFEST_LENGTH_INDEX: Final = 3
KSLM_HEADER_PAYLOAD_LENGTH_INDEX: Final = 4
KSLM_HEADER_CRC_INDEX: Final = 5
KSLM_HEADER_DIGEST_INDEX: Final = 6
# Pins of the frozen intent_model.py limits (pending reseal): a change there must fail these tests.
EXPECTED_INTENT_MINIMUM_RUNTIME_TOKEN_LENGTH: Final = 5
# intent_model.MAX_SUPPORTED_FINGERPRINTS == 1 << this
EXPECTED_KSLM_MAX_FINGERPRINTS_LOG2: Final = 20
EXPECTED_KSLM_MAX_PAYLOAD_MEBIBYTES: Final = 12
EXPECTED_KSLM_MAX_CONTAINER_MEBIBYTES: Final = 14
# A non-zero dimension that is not a valid model dimension.
INVALID_INTENT_DIMENSION: Final = 3
INTENT_MATRIX_DIMENSION: Final = 2048
INTENT_SAMPLE_FINGERPRINT: Final = 3
INTENT_SAMPLE_FINGERPRINTS: Final = (1, 2, 3)
INTENT_MUTATION_PROBE_VALUE: Final = 9
INTENT_IMMUTABLE_WRITE_PROBE: Final = 2
INTENT_SECOND_SAMPLE_FINGERPRINT: Final = 2
# src/keyswitch/intent_model.py's own bound, not yet named there (PENDING_RESEAL)
INTENT_MODEL_VERSION_MAX_CHARACTERS: Final = 128
INTENT_OVERSIZED_METADATA_INT: Final = 1 << 65
INTENT_LOADER_FIXTURE_FINGERPRINT: Final = 7
INTENT_WRONG_SCHEMA_FLOAT: Final = 3.0
INTENT_WRONG_FEATURE_VERSION_FLOAT: Final = 5.0
INTENT_WRONG_DIMENSION_FOR_PAYLOAD: Final = 16
INTENT_WRONG_FINGERPRINT_COUNT_FOR_PAYLOAD: Final = 3
INTENT_GC_TEST_FINGERPRINTS: Final = {1, 2}
INT16_MAX: Final = 32767
INTENT_SAMPLE_NEGATIVE_WEIGHT: Final = 2
INTENT_LEAKAGE_ALTERNATE_TEST_DIMENSION: Final = 1024
INTENT_LEAKAGE_SPARSE_FEATURE_TEST_DIMENSION: Final = 8
INTENT_LEAKAGE_UNNORMALIZED_SPARSE_FEATURES: Final = ((2, 1.0), (1, 2.0), (2, -0.5))
INTENT_LEAKAGE_NORMALIZED_SPARSE_FEATURES: Final = ((1, 2.0), (2, 0.5))
INTENT_LEAKAGE_OUT_OF_RANGE_SPARSE_FEATURES: Final = ((8, 1.0),)
INTENT_LEAKAGE_NON_FINITE_SPARSE_FEATURES: Final = ((1, math.inf),)
INTENT_OPTIMIZER_SECOND_FEATURE_INDEX: Final = 2
FTRL_SMALL_TEST_DIMENSION: Final = 8
FTRL_CACHED_WEIGHTS_DIMENSION: Final = 32
INTENT_NONCANONICAL_FEATURE_CASES: Final = (
        (((2, 1.0), (2, 0.5)), "unique"),
        (((3, 1.0), (2, 0.5)), "strictly increasing"),
        (((8, 1.0),), "outside"),
        (((1, math.inf),), "finite"),
    )
INTENT_EXCESSIVE_DIMENSION_EXPONENT: Final = 22
INTENT_TRAIN_ONLY_SCORER_ALGORITHM_VERSION: Final = 2
INTENT_TRAIN_ONLY_SCORER_NGRAM_ORDERS: Final = (2, 3, 4)
INTENT_EXTERNAL_POLICY_SCHEMA_VERSION: Final = 2
INTENT_UNSUPPORTED_MANIFEST_SCHEMA_VERSION: Final = 2
INTENT_SEALED_CANDIDATE_QUANTIZE_WEIGHTS: Final = {2: 1.5, 9: -0.5}
INTENT_SEALED_CANDIDATE_FINGERPRINTS: Final = frozenset({1, 2, 3})
INTENT_TAMPERED_FINGERPRINTS: Final = frozenset({1, 2, 4})
INTENT_INVALID_QUANTIZED_WEIGHT: Final = INT16_MAX + 1
UINT64_BIT_WIDTH: Final = 64
EXPECTED_INTENT_PRODUCTION_MODEL_DIMENSION: Final = 2_097_152
INTENT_QUANTIZED_FIRST_WEIGHT_INDEX: Final = 2
INTENT_QUANTIZED_SECOND_WEIGHT_INDEX: Final = 9
EXPECTED_INTENT_CONFIG_SCHEMA_VERSION: Final = 13
INTENT_UNSUPPORTED_CONFIG_SCHEMA_VERSION: Final = 2
FTRL_KERNEL_DIMENSION: Final = 512
FTRL_KERNEL_DUPLICATE_FEATURE_INDEX: Final = 5
# "unused_training_version" is not part of the lexical contract; these fixture numbers only have to
# differ from each other to exercise that indifference.
LEXICAL_GENERATION_UNUSED_TRAINING_VERSION: Final = 21
LEXICAL_CURRENT_UNUSED_TRAINING_VERSION: Final = 23
LEXICAL_FUTURE_UNUSED_TRAINING_VERSION: Final = 24
# Arbitrary fixture weights for a fabricated candidate.json; their values are never asserted on,
# only their presence and checksum.
LEXICAL_FIXTURE_CANDIDATE_WEIGHTS: Final = (1, 2, 3)
UNSUPPORTED_LEXICON_SUPPLEMENT_SCHEMA_VERSION: Final = 2
# channel()'s fixture: order-1 grams "a"/"b" share one stored logprob, the order-2 gram "ab" has its
# own, backoff off either order-1 gram costs the same, and anything unseen falls all the way to the
# uniform logprob.
ORTHO_FIXTURE_SHORT_GRAM_LOGPROB: Final = -512
ORTHO_FIXTURE_LONG_GRAM_LOGPROB: Final = -256
ORTHO_FIXTURE_BACKOFF_WEIGHT: Final = -128
ORTHO_FIXTURE_UNIFORM_LOGPROB: Final = -1024
# shape_table()'s fixture: "initial"/"inner" share a moderate penalty, "upper" a severe one ("lower"
# is the unpenalized default, 0).
ORTHO_FIXTURE_MODERATE_SHAPE_PENALTY: Final = -512
ORTHO_FIXTURE_SEVERE_SHAPE_PENALTY: Final = -2048
# minimal()'s own fixture values. ORTHO_FIXTURE_SCALE doubles as both "scale" and each language's
# raw "thresholds" entry, so the loaded, normalized threshold comes out to exactly 1.0.
ORTHO_FIXTURE_ORDER: Final = 3
ORTHO_FIXTURE_SCALE: Final = 512
ORTHO_FIXTURE_MINIMUM_LENGTH: Final = 3
UNSUPPORTED_ORTHO_SCHEMA_VERSION: Final = 2
ORTHO_OUT_OF_RANGE_ORDER: Final = 99
ORTHO_NON_INTEGER_PROBE: Final = 1.5
ORTHO_NON_INTEGER_LOGPROB: Final = 0.5
# an arbitrary int where a gram name (text) is required
ORTHO_NON_TEXT_GRAM_NAME: Final = 2
UNSUPPORTED_EXPOSURE_INVENTORY_SCHEMA_VERSION: Final = 2
UNSUPPORTED_PREFIX_FEATURE_VERSION: Final = 3
LANGUAGE_MODEL_TRIGRAM_ORDER: Final = 3
UNSUPPORTED_KSLM_SCHEMA_VERSION: Final = KSLM_SCHEMA_VERSION - 1
# One past the largest representable uint64, and the largest one itself; both recur as
# fingerprint/seed boundary fixtures.
UINT64_OVERFLOW: Final = 1 << UINT64_BIT_WIDTH
UINT64_MAX: Final = UINT64_OVERFLOW - 1
# --- test_intent_training's working TrainingConfig, config() ---
# The hard-negative development policy's schema and per-group role counts; the four roles together
# exhaust the external evaluation's minimum words per group, as TrainingConfig.validate() requires.
INTENT_HARD_NEGATIVE_POLICY_SCHEMA_VERSION: Final = 2
INTENT_HARD_NEGATIVE_TRAIN_WORDS_PER_GROUP: Final = 3_500
# One train word short, so the four roles no longer exhaust the words per group.
INTENT_HARD_NEGATIVE_UNDERFILLED_TRAIN_WORDS_PER_GROUP: Final = INTENT_HARD_NEGATIVE_TRAIN_WORDS_PER_GROUP - 1
INTENT_HARD_NEGATIVE_DEVELOPMENT_WORDS_PER_GROUP: Final = 500
INTENT_HARD_NEGATIVE_CALIBRATION_WORDS_PER_GROUP: Final = 500
INTENT_HARD_NEGATIVE_THRESHOLD_WORDS_PER_GROUP: Final = 500
# Its sealed-test recall floors; a selection recall floor below its sealed-test floor is refused.
INTENT_TEST_CONFIG_SEALED_MINIMUM_RECALL: Final = 0.8
INTENT_TEST_CONFIG_SEALED_MINIMUM_PAUSE_RECALL: Final = 0.7
INTENT_TEST_CONFIG_SEALED_MINIMUM_TYPO_RECALL: Final = 0.7
INTENT_TEST_CONFIG_SEALED_MINIMUM_PAUSE_TYPO_RECALL: Final = 0.6
# Its word-length, epoch, optimizer, calibration, threshold, veto and gate fields, keyed by
# TrainingConfig field: a short run whose selection recall floors sit just above the sealed ones.
INTENT_TEST_TRAINING_CONFIG_FIELDS: Final[dict[str, object]] = {
    "minimum_word_length": 3,
    "maximum_word_length": 18,
    "maximum_words_per_language": 0,
    "typo_augmentations": 2,
    "maximum_epochs": 8,
    "minimum_epochs": 2,
    "patience": 2,
    "ftrl_alpha": 0.2,
    "ftrl_beta": 1.0,
    "ftrl_l1": 0.0,
    "ftrl_l2": 0.01,
    "calibration_l2": 0.01,
    "calibration_max_iterations": 80,
    "threshold_precision_floor": 1.0,
    "threshold_max_false_positive_rate": 0.0,
    "pause_threshold_max_false_positive_rate": 0.0,
    "selection_maximum_false_positives_per_trigger": 0,
    "threshold_logit_margin_cap": 0.0,
    "pause_logit_margin": 0.5,
    "veto_positive_quantile": 0.001,
    "veto_logit_margin": 0.25,
    "veto_max_false_negative_rate": 0.01,
    "selection_minimum_recall": 0.81,
    "selection_minimum_pause_recall": 0.71,
    "selection_minimum_typo_recall": 0.71,
    "selection_minimum_pause_typo_recall": 0.61,
    "test_minimum_precision": 0.9,
    "test_minimum_recall": INTENT_TEST_CONFIG_SEALED_MINIMUM_RECALL,
    "test_minimum_pause_recall": INTENT_TEST_CONFIG_SEALED_MINIMUM_PAUSE_RECALL,
    "test_minimum_typo_recall": INTENT_TEST_CONFIG_SEALED_MINIMUM_TYPO_RECALL,
    "test_minimum_pause_typo_recall": INTENT_TEST_CONFIG_SEALED_MINIMUM_PAUSE_TYPO_RECALL,
    "test_minimum_specificity": 0.9,
    "safety_maximum_guard_failures": 0,
}
# Per-class sizes of the symmetric fixture ConfusionMatrix slices: with zero false positives the
# strong slice keeps even the familywise Wilson bound under INTENT_STRICT_FALSE_POSITIVE_RATE; the
# moderate slice only has to be a valid, perfectly separated one.
INTENT_STRONG_SAMPLE_SIZE: Final = 10_000
INTENT_MODERATE_SAMPLE_SIZE: Final = 100
# (true_positive, false_negative, true_negative, false_positive) of fixture ConfusionMatrix values.
# Development operating points of two epochs: the passing one clears its policy, the uncertified
# one recalls less and fails it.
INTENT_PASSING_EPOCH_CONFUSION_COUNTS: Final = (95, 5, INTENT_STRONG_SAMPLE_SIZE, 0)
INTENT_PASSING_EPOCH_TYPO_CONFUSION_COUNTS: Final = (90, 10, INTENT_STRONG_SAMPLE_SIZE, 0)
INTENT_UNCERTIFIED_EPOCH_CONFUSION_COUNTS: Final = (94, 6, INTENT_STRONG_SAMPLE_SIZE, 0)
INTENT_UNCERTIFIED_EPOCH_TYPO_CONFUSION_COUNTS: Final = (89, 11, INTENT_STRONG_SAMPLE_SIZE, 0)
INTENT_PASSING_EPOCH_POLICY_CHECKS_PASSED: Final = 10
INTENT_UNCERTIFIED_EPOCH_POLICY_CHECKS_PASSED: Final = 8
# What choose_directional_threshold selects on the INTENT_DIRECTIONAL_BUDGET_ROWS fixture, first
# with the default budget, then with no false positive allowed.
EXPECTED_INTENT_DIRECTIONAL_BUDGET_CONFUSION_COUNTS: Final = (10, 0, 19, 1)
EXPECTED_INTENT_ZERO_FALSE_POSITIVE_CONFUSION_COUNTS: Final = (5, 5, 20, 0)
# What it selects under an aggregate budget of one false positive across both directions.
EXPECTED_INTENT_AGGREGATE_BUDGET_CONFUSION_COUNTS: Final = (5, 1, 5, 1)
# The overall slice clears the strict precision floor; the typo slice clears the strict typo floor
# but not the stricter overall one.
INTENT_STRICT_OVERALL_CONFUSION_COUNTS: Final = (20_000, 0, 29_995, 5)
INTENT_STRICT_TYPO_CONFUSION_COUNTS: Final = (10_000, 0, 29_994, 6)
# Passes the ordinary Wilson bound but not the familywise one; its false positives and negatives are
# the ones false_positive_bound_payload() is given for the same familywise bound.
INTENT_WILSON_BOUND_CONFUSION_COUNTS: Final = (
    20_000,
    0,
    (INTENT_OPTIMIZER_FALSE_POSITIVE_BOUND_TOTAL_NEGATIVES
     - INTENT_OPTIMIZER_FALSE_POSITIVE_BOUND_FALSE_POSITIVES),
    INTENT_OPTIMIZER_FALSE_POSITIVE_BOUND_FALSE_POSITIVES,
)
# The full typo policy's selection at the strong score: every strong positive converts, every weak
# one is missed, and none of the (deletion-typo) negatives converts.
EXPECTED_INTENT_TYPO_POLICY_CONFUSION_COUNTS: Final = (
    INTENT_OPTIMIZER_DELETION_TYPO_STRONG_COUNT + INTENT_OPTIMIZER_IDENTITY_TYPO_STRONG_COUNT,
    INTENT_OPTIMIZER_DELETION_TYPO_WEAK_COUNT + INTENT_OPTIMIZER_IDENTITY_TYPO_WEAK_COUNT,
    INTENT_STRONG_SAMPLE_SIZE,
    0,
)
EXPECTED_INTENT_TYPO_POLICY_TYPO_CONFUSION_COUNTS: Final = (
    INTENT_OPTIMIZER_DELETION_TYPO_STRONG_COUNT,
    INTENT_OPTIMIZER_DELETION_TYPO_WEAK_COUNT,
    INTENT_STRONG_SAMPLE_SIZE,
    0,
)
INTENT_TAIL_DIAGNOSTIC_OVERALL_CONFUSION_COUNTS: Final = (1, 1, 2, 0)
# The operating point choose_directional_threshold is patched to return in the margin tests.
INTENT_INITIAL_SYMMETRIC_CONFUSION_COUNTS: Final = (2, 0, 2, 0)
