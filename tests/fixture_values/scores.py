"""Sample probabilities, scores, weights and losses fed to the code under test."""

from __future__ import annotations

import math
from typing import Final

from keyswitch.constants.training import FEATURE_MASS_TOLERANCE
from keyswitch.intent_model import CorrectionTrigger, LayoutDirection

from .models import (
    INTENT_TEST_CONFIG_SEALED_MINIMUM_PAUSE_RECALL,
    INTENT_TEST_CONFIG_SEALED_MINIMUM_PAUSE_TYPO_RECALL,
    INTENT_TEST_CONFIG_SEALED_MINIMUM_RECALL,
    INTENT_TEST_CONFIG_SEALED_MINIMUM_TYPO_RECALL,
)

# The runtime confidence threshold almost every call selects at, and a second, higher candidate
# offered alongside it in the tie-break test.
EPOCH_SELECTION_RUNTIME_THRESHOLD: Final = .99
EPOCH_SELECTION_ALTERNATE_THRESHOLD: Final = .995
# Fixture scores fed through profile(): the keep-row scores are negligible scores for a profile's
# one keep-labelled row, a second value keeping the early and later epochs distinguishable; the
# below-threshold and runtime-confidence scores are convert-row scores that respectively miss and
# reach EPOCH_SELECTION_RUNTIME_THRESHOLD; the high-confidence score is comfortably past it either
# way; the ambiguous score is a coin flip the model never commits on.
EPOCH_SELECTION_KEEP_ROW_SCORE: Final = .001
EPOCH_SELECTION_KEEP_ROW_SCORE_LATER: Final = .002
EPOCH_SELECTION_BELOW_THRESHOLD_SCORE: Final = .98
EPOCH_SELECTION_RUNTIME_CONFIDENCE_SCORE: Final = .995
EPOCH_SELECTION_HIGH_CONFIDENCE_SCORE: Final = .999
EPOCH_SELECTION_AMBIGUOUS_SCORE: Final = .5
# Development loss values: the low and high losses are genuinely compared against each other;
# EPOCH_SELECTION_LOSS_NOT_UNDER_TEST and EPOCH_SELECTION_VALID_LOSS are placeholders the assertions
# never inspect, kept apart because their calls isolate different parameters.
EPOCH_SELECTION_DEVELOPMENT_LOSS_LOW: Final = .01
EPOCH_SELECTION_DEVELOPMENT_LOSS_HIGH: Final = .02
EPOCH_SELECTION_LOSS_NOT_UNDER_TEST: Final = .001
EPOCH_SELECTION_VALID_LOSS: Final = .1
EPOCH_SELECTION_EXPECTED_MINIMUM_RECALL: Final = .5
# (keep, convert, wait, suggest) rows of the abstention fixture, labelled convert, convert, wait:
# the first converts at exactly EPOCH_SELECTION_RUNTIME_THRESHOLD, the second abstains because
# suggest outweighs convert, the third is a confident wait.
EPOCH_SELECTION_ABSTENTION_ROW_SCORES: Final = (
    0.0, EPOCH_SELECTION_RUNTIME_THRESHOLD, .01, 0.0,
    0.0, .1, 0.0, .9,
    0.0, .01, .99, 0.0,
)
# Development thresholds just outside the [0, 1] probability range.
EPOCH_SELECTION_THRESHOLD_BELOW_RANGE: Final = -.1
EPOCH_SELECTION_THRESHOLD_ABOVE_RANGE: Final = 1.1
# Every probability of a row that does not sum to one.
EPOCH_SELECTION_UNNORMALIZED_ROW_SCORE: Final = .1
# A weight and threshold picked so predict() lands decisively on the weighted candidate, well above
# any threshold checked here.
BOUNDARY_DECISIVE_FEATURE_WEIGHT: Final = 20
BOUNDARY_CONSTRUCTED_MODEL_THRESHOLD: Final = .9
# A threshold inside the model's accepted (0.5, 1.0] range, and one below it.
BOUNDARY_MODEL_VALID_THRESHOLD: Final = .99
BOUNDARY_MODEL_THRESHOLD_BELOW_RANGE: Final = .4
# A probability paired with an abstaining (suffix_length=None) prediction.
BOUNDARY_ABSTAIN_PROBABILITY: Final = .51
BOUNDARY_EXCLUDED_TOKEN_PROBABILITY: Final = .5
# Frozen tools/train_boundary_v2.py and src/keyswitch/boundary_policy.py pin these directly
# (PENDING_RESEAL), so they are mirrored here rather than imported.
BOUNDARY_POLICY_VALID_THRESHOLD: Final = 0.99
BOUNDARY_POLICY_THRESHOLD_LOWER_BOUND: Final = 0.5
# A per_mille sampling rate chosen for this test, distinct from the application's own
# TATOEBA_SAMPLE_PER_MILLE default.
TATOEBA_TEST_SAMPLE_PER_MILLE: Final = 100
# One weight per action (keep, convert, wait, suggest), all zero.
NEUTRAL_ACTION_WEIGHT_ROW: Final = (0.0, 0.0, 0.0, 0.0)
# A bias large enough that its class wins the softmax regardless of whatever else is in the weights
# - the "make this action win" fixture trick.
DOMINANT_BIAS_WEIGHT: Final = 20.0
# A smaller bias used the same way against a fixture with no competing features, where it dominates
# without needing DOMINANT_BIAS_WEIGHT's margin.
MODERATE_BIAS_WEIGHT: Final = 8.0
ACTION_FEATURES_RICH_MODEL_PROBABILITY: Final = 0.6
ACTION_FEATURES_RICH_MODEL_THRESHOLD: Final = 0.99
ACTION_FEATURES_RICH_ORTHO_SCORE: Final = 100.0
ACTION_FEATURES_SOURCE_NGRAM_SCORE: Final = 0.3
ACTION_FEATURES_STRONG_FEATURE_WEIGHT: Final = 30.0
ACTION_FEATURES_NGRAM_SCORE_PROBE: Final = 5.0
# A sample model_probability below the sample model_threshold of a decision fixture.
SAMPLE_MODEL_PROBABILITY: Final = 0.8
SAMPLE_MODEL_THRESHOLD: Final = 0.9
ACTION_FEATURES_ORTHO_SCORE: Final = 20
ACTION_FEATURES_ORTHO_THRESHOLD: Final = 4
ACTION_FEATURES_EXPECTED_BASELINE_MARGIN: Final = -0.1
ACTION_FEATURES_EXTREME_MAGNITUDE: Final = 1000
ACTION_FEATURES_OUT_OF_RANGE_RATIO: Final = 10
ACTION_FEATURES_EXTREME_INVALID_RATIO_MAGNITUDE: Final = 3
ACTION_FEATURES_EXTREME_RAW_NGRAM_SCORE: Final = 2
ACTION_FEATURES_EXTREME_FREQUENCY: Final = 10**100
ACTION_FEATURES_ORTHO_EVIDENCE_SCORE: Final = 20.0
ACTION_FEATURES_ORTHO_EVIDENCE_THRESHOLD: Final = 4.0
ACTION_FEATURES_ORTHO_EVIDENCE_BASE_BIAS: Final = 2.0
ACTION_FEATURES_ORTHO_SIDE_THRESHOLD: Final = 10.0
ACTION_FEATURES_ORTHO_SIDE_SCORE: Final = 12.0
ACTION_FEATURES_ORTHO_SIDE_BASE_BIAS: Final = 4.0
ACTION_FEATURES_ORTHO_SIDE_MARGIN_PROBES: Final = (0.001, 2.0, 32.0)
ACTION_FEATURES_ORTHO_SIDE_BELOW_SCORE: Final = 9.0
ACTION_FEATURES_SHORT_WORD_BASE_BIAS: Final = 5.0
# The WordScore both sides of the action-feature fixtures start from: an unknown, fairly plausible
# reading (frequency is the corpus count it reports).
ACTION_FEATURES_SAMPLE_WORD_VALUE: Final = 2.0
ACTION_FEATURES_SAMPLE_WORD_FREQUENCY: Final = 20
ACTION_FEATURES_SAMPLE_GRAM_RATIO: Final = 0.8
ACTION_FEATURES_SAMPLE_NGRAM_SCORE: Final = 3.0
ACTION_FEATURES_SAMPLE_RAW_NGRAM_SCORE: Final = -2.0
SEAL_FIXTURE_CONVERSION_RECALL: Final = 0.9
# conversion_threshold of the authored context-action artifact the evaluator and verifier tests
# write.
AUTHORED_ARTIFACT_CONVERSION_THRESHOLD: Final = 0.99
# conversion_threshold of the authored prefix artifact the evaluator and verifier tests write.
AUTHORED_PREFIX_CONVERSION_THRESHOLD: Final = 0.999
# A weight that differs from the authored one, so a rewritten artifact no longer matches its seal.
ALTERED_ARTIFACT_WEIGHT: Final = 2.0
# A prefix threshold that differs from the authored one, so it contradicts the seal.
MISMATCHED_PREFIX_CONVERSION_THRESHOLD: Final = 0.995
# Transparent fixture weights (see authored_model's own comment): large enough to force a specific,
# executable decision, not tuned for model quality.
AUTHORED_SEQUENCE_BIAS_WEIGHT: Final = 20.0
AUTHORED_SEQUENCE_DIRECTION_WEIGHT: Final = 40.0
AUTHORED_PREFIX_WEIGHT: Final = 10.0
AUTHORED_BASELINE_PREFIX_WEIGHT: Final = 5.0
ALTERED_BASELINE_PREFIX_WEIGHT: Final = 7.0
MISMATCHED_ARTIFACT_CONVERSION_THRESHOLD: Final = 0.98
ACTION_TRAINING_CONVERT_PROBABILITY: Final = 0.999
ACTION_TRAINING_RESIDUAL_PROBABILITY: Final = 0.001
# FeatureMass / select_features fixtures
FEATURE_MASS_FAMILY_A_MASS: Final = 8.0
FEATURE_MASS_SHARED_LOW_MASS: Final = 0.01
FEATURE_MASS_CUTOFF: Final = 0.25
FEATURE_MASS_COMPARISON_DELTA: Final = 1e-12
FEATURE_MASS_NEW_CONTEXT_MASS: Final = 100.0
FEATURE_MASS_LOOSER_CUTOFF: Final = 0.3
FEATURE_MASS_BASE_MASS: Final = 3.0
FEATURE_MASS_EXCLUDED_MASS: Final = FEATURE_MASS_BASE_MASS - 1.1 * FEATURE_MASS_TOLERANCE
FEATURE_MASS_INCLUDED_MASS: Final = FEATURE_MASS_BASE_MASS - 0.9 * FEATURE_MASS_TOLERANCE
FEATURE_MASS_NEAR_TIE_MASS: Final = FEATURE_MASS_BASE_MASS - 1e-12
FEATURE_MASS_OTHER_MASS: Final = 2.9
FEATURE_MASS_OVERFLOW_WEIGHT: Final = 1e308
HISTORICAL_CURRICULUM_CONVERT_MASS_SHARE: Final = 0.5
HISTORICAL_CURRICULUM_COMBINED_INDEPENDENT_MASS: Final = 2.0
# threshold / calibration fixtures
CHOOSE_THRESHOLD_PORTABLE_FALSE_SCORE: Final = 0.98
CHOOSE_THRESHOLD_REFERENCE_FALSE_SCORE: Final = 0.995
CHOOSE_THRESHOLD_HIGH_CONVERT_CONFIDENCE: Final = 0.9999
CHOOSE_THRESHOLD_MINIMUM_RECALL_FLOOR: Final = 0.9
CHOOSE_THRESHOLD_LOW_CANDIDATE: Final = 0.95
CHOOSE_THRESHOLD_MID_CANDIDATE: Final = 0.99
CHOOSE_THRESHOLD_HIGH_CANDIDATE: Final = 0.999
CHOOSE_THRESHOLD_CEILING_CANDIDATE: Final = 0.9995
CHOOSE_THRESHOLD_LOW_PROBABILITY: Final = 0.96
CHOOSE_THRESHOLD_NET_BENEFIT_TOLERANCE: Final = 0.05
EXPECTED_RECIPE_NET_BENEFIT_TOLERANCE: Final = 0.01
CHOOSE_THRESHOLD_AUTHORED_FLOOR: Final = 0.96
CHOOSE_THRESHOLD_NO_CONVERSION_FALSE_SCORE: Final = 0.99
CHOOSE_THRESHOLD_NO_CONVERSION_TRUE_SCORE: Final = 0.98
CHOOSE_THRESHOLD_VERY_LOW_FALSE_SCORE: Final = 0.1
CHOOSE_THRESHOLD_PARTIAL_RECALL: Final = 0.5
CHOOSE_THRESHOLD_LOW_RECALL_FLOOR: Final = 0.4
LOOKAHEAD_POSITIVE_SEED_WEIGHT: Final = 0.25
LOOKAHEAD_NEGATIVE_SEED_WEIGHT: Final = 0.75
LEGACY_LOOKAHEAD_TOTAL_SEED_MASS: Final = 3.0
PLANNED_BALANCE_ISOLATED_SUGGEST_WEIGHT: Final = 2.0
PLANNED_BALANCE_CONTEXT_CONVERT_WEIGHT: Final = 5.0
PLANNED_BALANCE_FIRST_CONVERT_WEIGHT: Final = 0.01
PLANNED_BALANCE_SECOND_CONVERT_WEIGHT: Final = 0.03
PLANNED_BALANCE_KEEP_WEIGHT: Final = 0.5
PLANNED_BALANCE_ISOLATED_MASS_TOTAL: Final = 4.0
PLANNED_BALANCE_SCALE_FACTOR: Final = 100.0
# Fixture bias/weight magnitudes for the small four-class model: the keep bias is a modest default
# preference for "keep"; the override weight is large enough to flip the argmax away from it via a
# single feature; the decisive bias wins outright.
AFTER_ORIGIN_KEEP_BIAS_WEIGHT: Final = 4.0
AFTER_ORIGIN_OVERRIDE_WEIGHT: Final = 20.0
AFTER_ORIGIN_DECISIVE_BIAS_WEIGHT: Final = 30.0
AFTER_ORIGIN_CONVERT_BIAS_WEIGHT: Final = 10.0
INVALID_CONTEXT_CONVERSION_THRESHOLD: Final = 0.5
SOFTMAX_HIGH_LOGIT: Final = 10000.0
SOFTMAX_NEAR_HIGH_LOGIT: Final = 9999.0
CONTEXT_V2_KERNEL_LEARNING_RATE: Final = 0.08
CONTEXT_V2_LOW_THRESHOLD_CANDIDATE: Final = 0.99
CONTEXT_V2_EXPECTED_THRESHOLD: Final = 0.999
CONTEXT_V2_THRESHOLD_CANDIDATES: Final = [CONTEXT_V2_LOW_THRESHOLD_CANDIDATE, CONTEXT_V2_EXPECTED_THRESHOLD, 1.0]
# (keep, convert, wait, suggest) rows given to select_threshold, labelled keep then convert: the
# keep row is falsely converted between CONTEXT_V2_LOW_THRESHOLD_CANDIDATE and
# CONTEXT_V2_EXPECTED_THRESHOLD, the convert row clears both.
CONTEXT_V2_THRESHOLD_ROW_SCORES: Final = (0.001, 0.995, 0.003, 0.001, 0.0, 0.99995, 0.00005, 0.0)
# A detection.confidence a test configures instead of the default.
NON_DEFAULT_CONFIDENCE_THRESHOLD: Final = 3.5
HISTORY_FIRST_ENTRY_CONFIDENCE: Final = 9.5
HISTORY_SECOND_ENTRY_CONFIDENCE: Final = 8.5
HISTORY_THIRD_ENTRY_CONFIDENCE: Final = 8.0
SHORT_SOURCE_VETO_NGRAM_SCORE: Final = 0.73
SHORT_SOURCE_VETO_SOURCE_WORD_SCORE: Final = 0.8
SHORT_SOURCE_VETO_TARGET_WORD_SCORE: Final = 4.2
SHORT_SOURCE_VETO_TARGET_NGRAM_SCORE: Final = 4.0
SHORT_SOURCE_VETO_DECISION_CONFIDENCE: Final = 4.73
SHORT_SOURCE_VETO_STRONGER_NGRAM_SCORE: Final = 6.3
# score() helper fixture internals.
DETECTOR_FIXTURE_DEFAULT_NGRAM_SCORE: Final = -2.0
DETECTOR_FIXTURE_GRAM_RATIO: Final = 0.5
DETECTOR_FIXTURE_INVALID_RATIO: Final = 0.5
DETECTOR_UNKNOWN_WORD_SCORE: Final = -5.0
DETECTOR_DEFAULT_VETO_THRESHOLD: Final = -3.0
# --- test_early_guards_and_forced_rules ---
DETECTOR_WEAK_SOURCE_SCORE: Final = -5
DETECTOR_DOMINANT_KNOWN_SCORE: Final = 8
DETECTOR_HIGHER_DOMINANT_KNOWN_SCORE: Final = 9
# Mirrors detector.decide's own forced-group floor: max(20.0, forced_delta).
DETECTOR_FORCED_CONFIDENCE_FLOOR: Final = 20.0
# --- test_trusted_short_words_require_curated_exact_dominant_target ---
DETECTOR_CURATED_SOURCE_SCORE: Final = 5.0
DETECTOR_NOT_EXACT_SHORT_WORD_SCORE: Final = -4.0
DETECTOR_TRUSTED_TARGET_SCORE: Final = 4.5
DETECTOR_NON_EXACT_TARGET_SCORE: Final = 4.0
DETECTOR_HIGH_CONFIDENCE_SCORE: Final = 8.0
# --- test_valid_source_guards_ambiguity ---
DETECTOR_SOURCE_ONLY_TARGET_SCORE: Final = -2
# --- test_linear_model_is_residual_and_cannot_bypass_hard_guards ---
DETECTOR_CONFIDENT_SWITCH_LOGIT: Final = 8.0
DETECTOR_CONFIDENT_SWITCH_PROBABILITY: Final = 0.999
DETECTOR_HIGH_MODEL_THRESHOLD: Final = 0.99
DETECTOR_STANDARD_MODEL_THRESHOLD: Final = 0.98
# --- test_linear_model_is_authoritative_after_hard_guards ---
DETECTOR_VETOED_LOGIT: Final = -6.0
DETECTOR_VETOED_PROBABILITY: Final = 0.002
DETECTOR_STRICT_VETO_THRESHOLD: Final = -4.0
DETECTOR_UNCERTAIN_LOGIT: Final = 0.0
DETECTOR_UNCERTAIN_PROBABILITY: Final = 0.5
DETECTOR_RESCUE_LOGIT: Final = 6.0
DETECTOR_RESCUE_PROBABILITY: Final = 0.998
DETECTOR_RESCUE_SOURCE_SCORE: Final = 2
DETECTOR_RESCUE_SOURCE_NGRAM: Final = -2
DETECTOR_RESCUE_TARGET_SCORE: Final = 4
DETECTOR_RESCUE_CONFIDENCE_THRESHOLD: Final = 3.0
# --- test_linear_model_threshold_is_not_overridden_by_secondary_scores ---
DETECTOR_LOOSE_VETO_THRESHOLD: Final = -5.0
DETECTOR_UNSUPPORTED_SOURCE_SCORE: Final = -8
DETECTOR_UNSUPPORTED_SOURCE_NGRAM: Final = -2
DETECTOR_UNSUPPORTED_TARGET_SCORE: Final = 2
DETECTOR_LOW_COVERAGE: Final = 0.2
DETECTOR_SOURCEX_SCORE: Final = -2.5
DETECTOR_SOURCEX_NGRAM: Final = -1.0
DETECTOR_TARGETX_PLAUSIBLE_NGRAM: Final = -1.5
DETECTOR_TARGETX_NGRAM_BELOW_FLOOR: Final = -6.01
DETECTOR_TARGETX_NGRAM_AT_FLOOR: Final = -6.0
# --- test_exact_and_morphological_target_outcomes ---
DETECTOR_EXACT_CASE_SOURCE_SCORE: Final = -8
DETECTOR_EXACT_CASE_SOURCE_NGRAM: Final = -4
DETECTOR_EXACT_CASE_CONFIDENCE_THRESHOLD: Final = 9.0
DETECTOR_MORPHOLOGICAL_SOURCE_SCORE: Final = -3
DETECTOR_MORPHOLOGICAL_SOURCE_NGRAM: Final = -2
DETECTOR_MORPHOLOGICAL_TARGET_SCORE: Final = 4
DETECTOR_PLAUSIBLE_TARGET_NGRAM: Final = -4
DETECTOR_IMPLAUSIBLE_TARGET_NGRAM: Final = -5
DETECTOR_LOW_MARGIN_SOURCE_SCORE: Final = 3.5
DETECTOR_LOW_MARGIN_CONFIDENCE_THRESHOLD: Final = 3.0
# --- test_typo_and_unknown_ngram_outcomes ---
DETECTOR_TYPO_SOURCE_SCORE: Final = -8
DETECTOR_TYPO_SOURCE_NGRAM: Final = -3
DETECTOR_TYPO_TARGET_SCORE: Final = -4
DETECTOR_SOURCE_DELETION_SCORE: Final = -6
DETECTOR_TARGET_DELETION_SCORE: Final = 4
DETECTOR_CONVERTING_SOURCE_NGRAM: Final = -2
DETECTOR_CONVERTING_TARGET_SCORE: Final = 2
DETECTOR_UNNATURAL_SOURCE_SCORE: Final = -4
DETECTOR_UNNATURAL_SOURCE_NGRAM: Final = -2
DETECTOR_BAD_TARGET_NGRAM: Final = -3
DETECTOR_LOW_TARGET_SCORE: Final = -3
DETECTOR_AGGRESSIVE_SOURCE_NGRAM: Final = -0.8
DETECTOR_AGGRESSIVE_TARGET_NGRAM: Final = -1.8
# --- test_context_scoring_and_best_of_multiple_candidates ---
DETECTOR_MODERATE_NEGATIVE_SOURCE_SCORE: Final = -2
DETECTOR_CONTEXT_BONUS_SCORE: Final = 2.0
DETECTOR_CONTEXT_CONFIDENCE_FLOOR: Final = 3.0
CORRECTION_PLAN_CONFIDENCE: Final = 99
INJECTION_ERROR_PLAN_CONFIDENCE: Final = 4
# An arbitrary double used to probe ULP distance.
ULP_PROBE_DOUBLE_VALUE: Final = 0.1
INTENT_THRESHOLD_LOGITS: Final[dict[CorrectionTrigger, float]] = {
    "boundary_probe": 0.80,
    "pause": 0.70,
    "space": 0.60,
    "enter": 0.75,
    "tab": 0.77,
    "punctuation": 0.72,
}
# evidence()'s default source/target WordScore fixtures: an implausible source reading against a
# plausible, known, frequent target reading.
INTENT_EVIDENCE_SOURCE_WORD_VALUE: Final = -4.0
INTENT_EVIDENCE_SOURCE_NGRAM_SCORE: Final = -3.0
INTENT_EVIDENCE_TARGET_WORD_VALUE: Final = 5.0
INTENT_EVIDENCE_TARGET_GRAM_RATIO: Final = 0.9
INTENT_TEST_VETO_THRESHOLD: Final = -0.25
INTENT_TEST_BIAS: Final = 0.5
SIGMOID_OF_ZERO: Final = 0.5
SATURATING_LOGIT_MAGNITUDE: Final = 1000.0
INTENT_SOURCE_CHARACTER_GRAM_RATIO: Final = 0.2
INTENT_SOURCE_CHARACTER_INVALID_RATIO: Final = 0.8
INTENT_SOURCE_CHARACTER_RAW_NGRAM_SCORE: Final = -3.5
INTENT_TARGET_CHARACTER_GRAM_RATIO: Final = 0.8
INTENT_TARGET_CHARACTER_NGRAM_SCORE: Final = 0.5
INTENT_TARGET_CHARACTER_INVALID_RATIO: Final = 0.2
INTENT_TARGET_CHARACTER_RAW_NGRAM_SCORE: Final = 0.75
INTENT_SCORER_ONLY_MAGNITUDE: Final = 1_000.0
INTENT_SCORER_ONLY_HIGH_RATIO: Final = 0.99
INTENT_SCORER_ONLY_LOW_RATIO: Final = 0.01
INTENT_SCORER_ONLY_NGRAM_MAGNITUDE: Final = 4.0
INTENT_SCORER_ONLY_TARGET_RAW_NGRAM_SCORE: Final = -15.0
INTENT_GOLDEN_CONTEXT_DELTA: Final = 0.75
INTENT_MATRIX_CONTEXT_DELTAS: Final = (-6.0, -1.0, -0.25, 0.0, 0.25, 1.0, 6.0, math.nan, math.inf)
INTENT_RUNTIME_MATRIX_WEIGHT_SCALE: Final = 8.0
INTENT_RUNTIME_MATRIX_BIAS: Final = 0.125
INTENT_RUNTIME_MATRIX_PLATT_SCALE: Final = 0.75
INTENT_RUNTIME_MATRIX_PLATT_BIAS: Final = -0.25
INTENT_RUNTIME_MATRIX_CONTEXT_DELTAS: Final = (
        -math.inf,
        -6.0,
        -1.25,
        -1.0,
        -0.75,
        -0.25,
        -0.125,
        -0.0,
        0.125,
        0.25,
        0.75,
        1.0,
        1.25,
        6.0,
        math.inf,
        math.nan,
    )
INTENT_DETERMINISTIC_WEIGHT_SCALE: Final = 10.0
INTENT_METADATA_LOSS: Final = 0.125
INTENT_STEEP_PLATT_SCALE: Final = 2.0
INTENT_BIAS_MAGNITUDE: Final = 2.0
# A Platt scale that moves a raw logit across the threshold: raw above but calibrated below, and the
# reverse.
INTENT_THRESHOLD_CROSSING_PLATT_SCALE: Final = 0.25
INTENT_EXACT_BOUNDARY_PLATT_SCALE: Final = 0.5
INTENT_SATURATED_BIAS: Final = 40.0
INTENT_SATURATED_LOGIT: Final = 50.0
INTENT_DIRECTIONAL_PLATT_BIAS: Final = 2.0
INTENT_THRESHOLD_WRITE_PROBE: Final = 0.1
INTENT_ARBITRARY_THRESHOLD_LOGIT: Final = 0.5
INTENT_HARD_NEGATIVE_TRAINING_EXAMPLE_WEIGHT: Final = 3.0
INTENT_EVALUATION_LM_KNOWN_WORD_SCORE: Final = 6.0
INTENT_EVALUATION_LM_IMPLAUSIBLE_WORD_SCORE: Final = -4.0
INTENT_EVALUATION_LM_PLAUSIBLE_GRAM_RATIO: Final = 0.9
INTENT_EVALUATION_LM_IMPLAUSIBLE_GRAM_RATIO: Final = 0.05
INTENT_EVALUATION_LM_IMPLAUSIBLE_NGRAM_SCORE: Final = -3.0
INTENT_EVALUATION_LM_IMPLAUSIBLE_INVALID_RATIO: Final = 0.9
INTENT_SPY_VETO_THRESHOLD: Final = -999.0
INTENT_SPY_LOGIT: Final = 10.0
INTENT_SPY_PROBABILITY: Final = 0.99999
INTENT_SPY_THRESHOLD: Final = 0.9
INTENT_DATASET_EXPECTED_CONTEXT_STRESS_DELTAS: Final = {-6.0, -1.25, -0.75, -0.125, 0.0, 0.125, 0.75, 1.25, 6.0}
INTENT_DATASET_INVALID_TRAINING_EXAMPLE_WEIGHT: Final = 8.1
INTENT_LEAKAGE_LARGE_CONTEXT_DELTA_MAGNITUDE: Final = 6.0
INTENT_LEAKAGE_FRACTIONAL_CONTEXT_DELTA: Final = 0.25
INTENT_LEAKAGE_POISONED_WEIGHT: Final = 99.0
INTENT_OPTIMIZER_SECOND_FEATURE_VALUE: Final = 0.25
FTRL_TEST_ALPHA: Final = 0.1
FTRL_SPARSE_TEST_L1: Final = 10.0
FTRL_TEST_L2: Final = 0.01
INTENT_OPTIMIZER_CALIBRATION_SCALE: Final = 1.25
INTENT_OPTIMIZER_CALIBRATION_BIAS_MAGNITUDE: Final = 0.5
FTRL_CACHED_WEIGHTS_ALPHA: Final = 0.08
FTRL_CACHED_WEIGHTS_L1: Final = 0.001
FTRL_CACHED_WEIGHTS_L2: Final = 0.05
# Updates replayed through FTRLProximal and its bit-exact reference: the first update's raw sparse
# features are unsorted and repeat index 7, so the test normalizes them; the others are already
# canonical (features, label, sample_weight) rows.
FTRL_CACHED_WEIGHTS_FIRST_RAW_FEATURES: Final = ((7, 0.25), (2, -0.5), (7, 0.125), (11, 1.0))
FTRL_CACHED_WEIGHTS_FIRST_SAMPLE_WEIGHT: Final = 1.75
FTRL_CACHED_WEIGHTS_CANONICAL_UPDATES: Final = (
    (((2, 0.5), (7, -0.25), (19, 0.875)), False, 1.0),
    (((1, -1.0), (11, 0.5), (31, 0.125)), True, 2.25),
    (((2, -0.75), (19, 0.25)), False, 0.5),
)
# Development log losses of the two fixture epochs: the uncertified epoch's is lower, so only its
# failed policy can rank it below the passing one.
INTENT_PASSING_EPOCH_LOG_LOSS: Final = 0.2
INTENT_UNCERTIFIED_EPOCH_LOG_LOSS: Final = 0.01
INTENT_UNCERTIFIED_EPOCH_THRESHOLD_LOGIT: Final = 1.1
# Familywise Wilson upper bound of zero false positives among INTENT_STRONG_SAMPLE_SIZE development
# negatives, rounded up; both fixture epochs report it for ordinary and typo rows alike.
INTENT_EPOCH_FALSE_POSITIVE_RATE_UPPER_BOUND: Final = 0.000821
INTENT_OPTIMIZER_PLATT_THRESHOLD_SAMPLES: Final = ((-3.0, False), (-2.0, False), (2.0, True), (3.0, True))
INTENT_PLATT_TEST_L2: Final = 0.01
INTENT_OPTIMIZER_EXTREME_SAMPLE_LOGIT: Final = 3.0
INTENT_OPTIMIZER_SIGMOID_EDGE_LOGITS: Final = (
        -math.inf,
        -1000.0,
        -746.0,
        -745.0,
        -0.0,
        0.0,
        745.0,
        1000.0,
        math.inf,
    )
INTENT_OPTIMIZER_DIRECTIONAL_PLATT_SAMPLES: Final[tuple[tuple[float, bool, LayoutDirection], ...]] = (
        (-2.0, False, "0>1"),
        (-1.0, False, "0>1"),
        (1.0, True, "0>1"),
        (2.0, True, "0>1"),
        (-6.0, False, "1>0"),
        (-5.0, False, "1>0"),
        (-3.0, True, "1>0"),
        (-2.0, True, "1>0"),
    )
INTENT_OPTIMIZER_PROBE_LOGIT: Final = 2.0
INTENT_OPTIMIZER_HIGH_SCORE: Final = 2.0
INTENT_OPTIMIZER_NEGATIVE_LOGIT_STEP_DIVISOR: Final = 100.0
INTENT_OPTIMIZER_PAUSE_LOGIT_MARGIN_PROBE: Final = 0.5
INTENT_STRICT_FALSE_POSITIVE_RATE: Final = 0.001
INTENT_OPTIMIZER_VETO_MARGIN_PROBE: Final = 0.25
INTENT_DIRECTIONAL_BUDGET_ROWS: Final = (
        ("0>1", (10.0, 9.0, 8.0, 7.0, 6.0), 5.0),
        ("1>0", (4.0, 3.0, 2.0, 1.0, 0.0), 100.0),
    )
INTENT_OPTIMIZER_NEGATIVE_TAIL_BASE: Final = 20.0
INTENT_OPTIMIZER_STANDARD_TEST_PRECISION_FLOOR: Final = 0.9
INTENT_OPTIMIZER_EXPECTED_DIRECTIONAL_RUNTIME_LOGIT: Final = 6.0
INTENT_OPTIMIZER_AGGREGATE_POSITIVE_SCORES: Final = (10.0, 9.0, 5.0)
INTENT_OPTIMIZER_AGGREGATE_NEGATIVE_SCORES: Final = (6.0, -10.0, -11.0)
INTENT_OPTIMIZER_STRICT_PRECISION_FLOOR: Final = 0.9995
INTENT_OPTIMIZER_STRICT_TYPO_PRECISION_FLOOR: Final = 0.999
INTENT_STRICTER_TYPO_CONFIG_OVERRIDES: Final = {
        "threshold_precision_floor": 0.9995,
        "threshold_max_false_positive_rate": 0.001,
        "pause_threshold_max_false_positive_rate": 0.001,
        "test_minimum_precision": 0.999,
        "test_minimum_recall": 0.90,
        "test_minimum_pause_recall": 0.90,
        "test_minimum_typo_recall": 0.90,
        "test_minimum_pause_typo_recall": 0.85,
        "test_minimum_specificity": 0.999,
        "selection_maximum_false_positives_per_trigger": 10,
    }
INTENT_WILSON_BOUND_GATE_KWARGS: Final = {
        "minimum_precision": 0.9995,
        "minimum_recall": 0.95,
        "minimum_specificity": 0.999,
        "maximum_false_positive_rate": 0.001,
    }
INTENT_FULL_TYPO_POLICY_KWARGS: Final = {
        "precision_floor": 0.9995,
        "maximum_false_positive_rate": 0.001,
        "minimum_recall": 0.95,
        "minimum_specificity": 0.999,
        "typo_precision_floor": 0.999,
        "minimum_typo_recall": 0.90,
        "typo_minimum_specificity": 0.999,
        "typo_maximum_false_positive_rate": 0.001,
    }
INTENT_WEAK_TYPO_POLICY_KWARGS: Final = {
        "precision_floor": 0.9995,
        "maximum_false_positive_rate": 0.001,
        "minimum_recall": 0.90,
        "minimum_specificity": 0.999,
        "typo_precision_floor": 0.999,
        "minimum_typo_recall": 0.90,
        "typo_minimum_specificity": 0.999,
        "typo_maximum_false_positive_rate": 0.001,
    }
INTENT_WEAK_TRAINING_CONFIG_OVERRIDES: Final = {
        "threshold_precision_floor": 0.9995,
        "threshold_max_false_positive_rate": 0.001,
        "pause_threshold_max_false_positive_rate": 0.001,
        "test_minimum_precision": 0.999,
        "test_minimum_recall": 0.90,
        "test_minimum_pause_recall": 0.90,
        "test_minimum_typo_recall": 0.90,
        "test_minimum_pause_typo_recall": 0.90,
        "test_minimum_specificity": 0.999,
    }
INTENT_TAIL_DIAGNOSTIC_TEMPLATES: Final = (
        (True, "deletion", 3.0, 5),
        (True, "identity", 2.0, 500),
        (False, "deletion", 1.0, 5),
        (False, "identity", 0.0, 500),
    )
INTENT_OPTIMIZER_TAIL_DIAGNOSTIC_LOGIT: Final = 2.5
INTENT_OPTIMIZER_EXPECTED_ORDINARY_TAIL_LOGIT: Final = 2.0
INTENT_OPTIMIZER_EXPECTED_TYPO_TAIL_LOGIT: Final = 3.0
INTENT_OPTIMIZER_PAUSE_POSITIVE_SCORES: Final = (2.6, 2.8)
INTENT_OPTIMIZER_NON_PAUSE_POSITIVE_SCORES: Final = (3.0, 3.1)
INTENT_PAUSE_MARGIN_CONFIG_OVERRIDES: Final = {
        "threshold_max_false_positive_rate": 0.001,
        "pause_threshold_max_false_positive_rate": 0.001,
        "pause_logit_margin": 0.5,
        "test_minimum_recall": 1.0,
        "test_minimum_pause_recall": 1.0,
        "test_minimum_typo_recall": 1.0,
        "test_minimum_pause_typo_recall": 1.0,
    }
INTENT_OPTIMIZER_PAUSE_LOGIT_LOWER_BOUND: Final = 3.1
INTENT_OPTIMIZER_POSITIVE_ROW_SCORE: Final = 4.0
INTENT_OPTIMIZER_LOGIT_MARGIN_CAP: Final = 2.0
INTENT_OPTIMIZER_EXPECTED_MARGIN_BASE_LOGIT: Final = 3.0
INTENT_OPTIMIZER_BACKOFF_NON_PAUSE_SCORE: Final = 2.5
INTENT_BACKOFF_CONFIG_OVERRIDES: Final = {
        "threshold_max_false_positive_rate": 1.0,
        "pause_threshold_max_false_positive_rate": 1.0,
        "threshold_logit_margin_cap": 2.0,
        "pause_logit_margin": 0.5,
        "test_minimum_recall": 1.0,
        "test_minimum_pause_recall": 1.0,
        "test_minimum_typo_recall": 1.0,
        "test_minimum_pause_typo_recall": 1.0,
    }
INTENT_OPTIMIZER_BACKOFF_EXPECTED_MARGIN: Final = 1.5
INTENT_OPTIMIZER_INVALID_FALSE_POSITIVE_RATE: Final = 1.1
# Each selection recall floor of config() with the sealed-test floor it may not fall below.
INTENT_OPTIMIZER_SEALED_VALUE_BY_FIELD: Final = (
        ("selection_minimum_recall", INTENT_TEST_CONFIG_SEALED_MINIMUM_RECALL),
        ("selection_minimum_pause_recall", INTENT_TEST_CONFIG_SEALED_MINIMUM_PAUSE_RECALL),
        ("selection_minimum_typo_recall", INTENT_TEST_CONFIG_SEALED_MINIMUM_TYPO_RECALL),
        (
            "selection_minimum_pause_typo_recall",
            INTENT_TEST_CONFIG_SEALED_MINIMUM_PAUSE_TYPO_RECALL,
        ),
    )
INTENT_OPTIMIZER_RECALL_UNDER_SEALED_MARGIN: Final = 0.01
# Pins of the frozen train_intent_model.py context constants: a change there must fail this test.
EXPECTED_INTENT_CONTEXT_DELTA_MULTIPLIER: Final = 1.75
EXPECTED_INTENT_CONTEXT_TARGET_GROUP_BONUS: Final = 0.55
EXPECTED_INTENT_CONTEXT_SOURCE_GROUP_PENALTY: Final = 0.3
INTENT_EXTERNAL_FIXTURE_THRESHOLD_LOGIT: Final = 0.6
INTENT_EXTERNAL_THRESHOLD_TAMPER_DELTA: Final = 0.25
INTENT_EXTERNAL_VETO_TAMPER_DELTA: Final = 0.5
INTENT_TAMPERED_METADATA_VALUE: Final = 999.0
INTENT_EXTERNAL_EXPECTED_COVERAGE_P25: Final = 0.25
INTENT_EXTERNAL_EXPECTED_COVERAGE_MEDIAN: Final = 0.5
INTENT_EXTERNAL_EXPECTED_COVERAGE_P75: Final = 0.75
# Logits and coverages of the coverage-diagnostics rows: two confident rows at plus or minus the
# confident logit and two marginal ones at plus or minus one; coverages 0, low, high and 1.
INTENT_EXTERNAL_COVERAGE_CONFIDENT_LOGIT: Final = 2.0
INTENT_EXTERNAL_COVERAGE_LOW_PARTIAL: Final = 0.25
INTENT_EXTERNAL_COVERAGE_HIGH_PARTIAL: Final = 0.75
INTENT_EXTERNAL_NEUTRAL_PREDICTION_LOGIT: Final = 10.0
INTENT_ARTIFACT_FIXTURE_VETO_THRESHOLD: Final = 2.0
INTENT_ARTIFACT_NEGATIVE_MODEL_LOGIT: Final = 2.0
INTENT_ARTIFACT_SEALED_CANDIDATE_THRESHOLD_LOGIT: Final = 0.25
INTENT_ARTIFACT_SEALED_CANDIDATE_CALIBRATION_SCALE: Final = 1.2
INTENT_ARTIFACT_SEALED_CANDIDATE_CALIBRATION_BIAS_MAGNITUDE: Final = 0.3
INTENT_ARTIFACT_SEALED_CANDIDATE_BIAS: Final = 0.1
INTENT_ARTIFACT_PRECISE_TAMPERED_LOGIT: Final = 0.25000000000000006
INTENT_ARTIFACT_TAMPERED_VETO_FALSE_NEGATIVE_RATE: Final = 0.02
INTENT_ARTIFACT_TAMPERED_BIAS_HEX_VALUE: Final = 0.2
INTENT_ARTIFACT_UNBOUNDED_LOGIT: Final = 1_000_001.0
INTENT_ARTIFACT_SCALE_HALVING_DIVISOR: Final = 2.0
INTENT_ARTIFACT_QUANTIZED_SCORER_BIAS: Final = 0.2
INTENT_ARTIFACT_QUANTIZATION_TEST_THRESHOLD_LOGIT: Final = 0.6
INTENT_ARTIFACT_QUANTIZATION_VETO_THRESHOLD: Final = 3.0
INTENT_ARTIFACT_QUANTIZATION_PLATT_SCALE: Final = 1.1
INTENT_ARTIFACT_QUANTIZATION_PLATT_BIAS_MAGNITUDE: Final = 0.1
EXPECTED_INTENT_PRODUCTION_TRAINING_EXAMPLE_WEIGHT: Final = 3.0
EXPECTED_INTENT_PRODUCTION_SELECTION_RECALLS: Final = (0.956, 0.91, 0.91, 0.86)
EXPECTED_INTENT_PRODUCTION_LOGIT_MARGIN_CAP: Final = 2.0
EXPECTED_WILSON_BOUND_AT_ZERO_FALSE_POSITIVES: Final = 0.003826759
FTRL_KERNEL_FEATURE_VALUE_SMALL: Final = 0.5
FTRL_KERNEL_FEATURE_VALUE_LARGE: Final = 2.0
FTRL_KERNEL_FEATURE_VALUE_NEGATIVE_SMALL: Final = 0.25
FTRL_KERNEL_RANDOM_FEATURE_MAGNITUDE: Final = 3.0
FTRL_KERNEL_LABEL_PROBABILITY_THRESHOLD: Final = 0.5
FTRL_KERNEL_WEIGHT_VALUE_LARGE: Final = 3.0
FTRL_KERNEL_WEIGHT_UNIFORM_LOWER_BOUND: Final = 0.5
FTRL_KERNEL_NATIVE_TEST_ALPHA: Final = 0.05
FTRL_KERNEL_NATIVE_TEST_L1: Final = 0.3
FTRL_KERNEL_NATIVE_TEST_L2: Final = 0.1
FTRL_KERNEL_PARTIAL_ORDER_ALPHA: Final = 0.5
INTENT_DETERMINISTIC_VETO_THRESHOLD: Final = 999.0
INTENT_DETERMINISTIC_LOGIT_OFFSET: Final = 0.25
INTENT_DETERMINISTIC_PREDICTION_COVERAGE: Final = 0.75
# Fixture-only gate values for the promotion config and the candidate; not sourced from any
# application default.
FROZEN_GATE_MINIMUM_DECIDED_FRACTION_PER_STRATUM: Final = 0.8
FROZEN_GATE_FIXTURE_THRESHOLD: Final = 0.95
FROZEN_GATE_FIXTURE_DECISIVE_WEIGHT: Final = 10.0
# Bias magnitudes for the fixture ContextModel weight vectors: enough to force a single action to
# win regardless of the other (zero) weights.
PREFIX_FIXTURE_CONVERT_BIAS: Final = 10.0
PREFIX_FIXTURE_UNCERTAIN_BIAS: Final = 2.0
PREFIX_THRESHOLD_BELOW_MINIMUM: Final = 0.9
PREFIX_THRESHOLD_ABOVE_MAXIMUM: Final = 1.1
PREFIX_METRICS_STRICT_THRESHOLD: Final = 0.999
PREFIX_METRICS_LENIENT_THRESHOLD: Final = 0.99
# Candidate probabilities of the prefix metrics fixture: the wrong-layout sequence converts at its
# first and next candidate prefixes above PREFIX_METRICS_STRICT_THRESHOLD, the identifier only above
# PREFIX_METRICS_LENIENT_THRESHOLD.
PREFIX_METRICS_WRONG_FIRST_CANDIDATE_PROBABILITY: Final = .9999
PREFIX_METRICS_WRONG_NEXT_CANDIDATE_PROBABILITY: Final = .99999
PREFIX_METRICS_IDENTIFIER_CANDIDATE_PROBABILITY: Final = .995
PREFIX_POLICY_SAMPLE_FEATURE_WEIGHT: Final = 0.5
PREFIX_POLICY_SAMPLE_CONVERSION_THRESHOLD: Final = 0.985
PREFIX_POLICY_ALTERED_CONVERSION_THRESHOLD: Final = 0.99
# A plausible serving threshold above MIN_PREFIX_CONVERSION_THRESHOLD, reused throughout as the
# calibration/development grid's upper rung.
PREFIX_V2_SERVING_THRESHOLD: Final = 0.999
PREFIX_V2_VERY_HIGH_THRESHOLD: Final = 0.9999
PREFIX_V2_NEAR_CERTAIN_PROBABILITY: Final = 0.99999
PREFIX_V2_MODERATE_PROBABILITY: Final = 0.99
# Two probabilities for the same wrongly-considered "correct" sequence: high confidence at the first
# candidate position, slightly lower at the next one.
PREFIX_V2_FIRST_CANDIDATE_HIGH_PROBABILITY: Final = 0.9995
PREFIX_V2_NEXT_CANDIDATE_HIGH_PROBABILITY: Final = 0.9994
# Loss values that stand in where the assertion is not about the loss itself.
PREFIX_V2_EARLY_EPOCH_LOSS: Final = 0.4
PREFIX_V2_LATE_EPOCH_LOSS: Final = 0.01
PREFIX_V2_ARBITRARY_LOSS: Final = 0.001
PREFIX_V2_PLACEHOLDER_LOSS: Final = 0.1
# Just under PREFIX_MIN_EARLY_RECALL_FLOOR, to see the recipe reject it.
PREFIX_V2_BELOW_EARLY_RECALL_FLOOR: Final = 0.69
# Each pair splits its total mass (1.0) evenly between correct and wrong.
PREFIX_V2_DESIRED_MASS_SHARE: Final = 0.5
# A weight distinguishing an undivided singleton row from its divided duplicates.
PREFIX_V2_SINGLETON_WEIGHT: Final = 2.0
# An arbitrary feature magnitude; irrelevant to selection, which ranks by sample weight, not by this
# value.
PREFIX_V2_RARE_FEATURE_VALUE: Final = 100.0
# A sample weight below the minimum feature mass (1.0), reused wherever a row should be too light to
# count.
PREFIX_V2_LIGHT_SAMPLE_WEIGHT: Final = 0.25
PREFIX_V2_KEEP_IMPORTANCE: Final = 3.0
PREFIX_V2_WAIT_IMPORTANCE: Final = 2.0
SEALED_TEST_TRIGGER_SCORE: Final = 0.99
SEAL_ENVIRONMENT_VETO_THRESHOLD: Final = 1.25
HISTORY_BRANCH_CONFIDENCE_OFFSET: Final = 0.126
LANGUAGE_MODEL_PUNCTUATION_ONLY_SCORE: Final = -30.0
LANGUAGE_MODEL_NEAR_ZERO_DEVIATION: Final = 0.01
TRAY_SAMPLE_CORRECTION_CONFIDENCE: Final = 5.0
UI_SAMPLE_HISTORY_CONFIDENCE: Final = 4.256
UI_SECOND_HISTORY_CONFIDENCE: Final = 2.0
# Threshold candidates offered to choose_threshold, lowest first.
CHOOSE_THRESHOLD_CANDIDATES: Final = [CHOOSE_THRESHOLD_LOW_CANDIDATE, CHOOSE_THRESHOLD_MID_CANDIDATE, CHOOSE_THRESHOLD_HIGH_CANDIDATE]
CHOOSE_THRESHOLD_CANDIDATES_WITH_CEILING: Final = [CHOOSE_THRESHOLD_LOW_CANDIDATE, CHOOSE_THRESHOLD_MID_CANDIDATE, CHOOSE_THRESHOLD_HIGH_CANDIDATE, CHOOSE_THRESHOLD_CEILING_CANDIDATE]
CHOOSE_THRESHOLD_NARROW_CANDIDATES: Final = [CHOOSE_THRESHOLD_LOW_CANDIDATE, CHOOSE_THRESHOLD_MID_CANDIDATE]
