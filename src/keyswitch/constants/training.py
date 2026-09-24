"""Training and evaluation hyperparameters, gates and protocol numbers of the tools."""

from __future__ import annotations

from typing import Final

from .keyboard import LAYOUT_GROUP_COUNT

# Absolute tolerance for a probability vector summing to one.
PROBABILITY_SUM_TOLERANCE: Final = 1e-8
REFERENCE_LEXICAL_FILE_COUNT: Final = 6
# Hard cap on families a span capture selects.
SPAN_MAXIMUM_FAMILIES: Final = 128
# Half the hard cap: one budget half for each layout group.
SPAN_DEFAULT_MAXIMUM_FAMILIES: Final = SPAN_MAXIMUM_FAMILIES // LAYOUT_GROUP_COUNT
SPAN_ORIGINAL_MAX_CHARACTERS: Final = 64
# Length bounds of a span anchor word.
SPAN_ANCHOR_MIN_CHARACTERS: Final = 3
SPAN_ANCHOR_MAX_CHARACTERS: Final = 24
# The isolated short reading: words of one or two letters; the lookahead curriculum accepts only
# these as seeds.
SHORT_WORD_MAX_CHARACTERS: Final = 2
LOOKAHEAD_DEFAULT_MAXIMUM_FAMILIES: Final = 64
LOOKAHEAD_MAXIMUM_FAMILIES_LIMIT: Final = 4096
# Also the ceiling `seeds_per_family` may not exceed; the default sits at the cap.
LOOKAHEAD_MAXIMUM_SEEDS_PER_FAMILY: Final = 128
# Length bounds of a lookahead anchor word, shared by the trainer that builds anchors and the
# curriculum that checks them.
LOOKAHEAD_ANCHOR_MIN_CHARACTERS: Final = 3
LOOKAHEAD_ANCHOR_MAX_CHARACTERS: Final = 64
# A seed with a planned variant keeps this share of its sample weight for each reading.
PLANNED_VARIANT_MASS_DIVISOR: Final = 2.0
# Synthetic keycodes for the US/RU physical pairs; the actual hardware code never matters here, only
# that each pair gets its own stable number.
PHYSICAL_KEY_KEYCODE_BASE: Final = 20
# Keycode and timestamp of the first synthetic key of a boundary replay.
BOUNDARY_EVALUATION_FIRST_KEY_SERIAL: Final = 100
# Simulated typing in the sequence test: key held, gap after release, idle after a word.
SIMULATED_KEY_DOWN_SECONDS: Final = 0.05
SIMULATED_KEY_UP_SECONDS: Final = 0.03
SIMULATED_WORD_IDLE_SECONDS: Final = 1.7
# Permissions of the test-access ledger lock file.
TEST_LEDGER_LOCK_FILE_MODE: Final = 0o600
SIMULATED_CLOCK_START_SECONDS: Final = 1000.0
# A sequence test needs at least this many documents in each language group (GATE_POLICY
# minimum_sequence_documents_per_group).
MINIMUM_SEQUENCE_DOCUMENTS_PER_GROUP: Final = 32
# Hash-ranked documents taken per document group in the sequence test (PROTOCOL
# document_cap_per_group).
SEQUENCE_DOCUMENT_CAP_PER_GROUP: Final = 128
# Version of the context-action sequence protocol: the evaluator's PROTOCOL (hashed into
# AUDITED_SEQUENCE_PROTOCOL_SHA256) and the verifier's public summary of it carry the same number.
CONTEXT_ACTION_SEQUENCE_PROTOCOL_VERSION: Final = 3
# physical_key_period_ms of the verifier's public protocol summary. It is the 100 ms key period of
# protocol version 2; version 3 types with SIMULATED_KEY_DOWN_SECONDS + SIMULATED_KEY_UP_SECONDS.
CONTEXT_ACTION_PROTOCOL_PHYSICAL_KEY_PERIOD_MS: Final = 100
# Promotion gates of the context-action model: the evaluator seals them and the verifier re-checks
# them.
CONTEXT_ACTION_GATE_POLICY: Final[dict[str, object]] = {
    "minimum_calibration_net_benefit": 1,
    "minimum_calibration_conversion_recall": 0.0,
    "sequence_corruptions_at_most_baseline": True,
    "sequence_length_mismatches": 0,
    "sequence_net_restorations_at_least_baseline": True,
    "minimum_sequence_documents_per_group": MINIMUM_SEQUENCE_DOCUMENTS_PER_GROUP,
}
# Words shorter or longer than these bounds are outside what the detector is meant to judge, so
# evaluation samples skip them.
DETECTOR_EVALUATION_MIN_WORD_CHARACTERS: Final = 3
DETECTOR_EVALUATION_MAX_WORD_CHARACTERS: Final = 18
# How many decimal places reported precision/recall/specificity keep.
DETECTOR_METRIC_DECIMALS: Final = 6
# Per-direction cap on how many miss examples are kept for the report.
DETECTOR_MAX_REPORTED_MISSES: Final = 20
# How many leading .aff lines are scanned for the file's declared encoding.
HUNSPELL_AFFIX_HEADER_SCAN_LINES: Final = 40
# Detector evaluation sample size: the CLI default and the floor applied to what is requested.
DETECTOR_DEFAULT_SAMPLE_SIZE: Final = 5000
DETECTOR_MIN_SAMPLE_SIZE: Final = 100
# Quality gates for --strict: precision/specificity are held to the same bar for the frequency-based
# and the Hunspell-based samples; recall is allowed to be looser against the broader Hunspell
# vocabulary.
DETECTOR_GATE_MIN_PRECISION: Final = 0.999
DETECTOR_GATE_MIN_SPECIFICITY: Final = 0.999
DETECTOR_GATE_MIN_RECALL: Final = 0.985
DETECTOR_GATE_MIN_DICTIONARY_RECALL: Final = 0.90
PREFIX_EVALUATION_SEQUENCES_PER_CATEGORY: Final = 32
PREFIX_EVALUATION_MAX_RECORDED_FAILURES: Final = 24
PREFIX_EVALUATION_KEYCODE_BASE: Final = 100
PREFIX_EVALUATION_TIMESTAMP_BASE: Final = 100
# The prefix candidate must restore at least this share of its desired prefixes early; the evaluator
# applies it and the verifier re-checks it.
PREFIX_EARLY_RESTORED_MIN_FRACTION: Final = 0.7
# Leading hex digits of a digest that feed a deterministic training choice.
DETERMINISTIC_CHOICE_HEX_DIGITS: Final = 8
# Absolute tolerance for feature-mass comparisons and budgets that must sum to one.
FEATURE_MASS_TOLERANCE: Final = 1e-9
# Decimal places fitted weights and feature masses are rounded to before hashing or ranking, so
# results do not depend on the platform.
DETERMINISTIC_ROUNDING_DECIMALS: Final = 9
# Balance languages before scoring: retain at most this many contexts per family.
MAX_CONTEXTS_PER_FAMILY: Final = 2
# A fair coin flip deciding which boundary-event text (newline vs tab) is used.
BOUNDARY_EVENT_CHOICES: Final = 2
# Roughly one row in this many keeps its observed field-after context.
FIELD_AFTER_SAMPLE_MODULUS: Final = 8
# Decimal places kept in the mass-balancing report.
MASS_REPORT_DECIMALS: Final = 4
# An identifier needs this many colon-separated parts to carry a command family, which is also how
# many leading parts make up that family key.
COMMAND_FAMILY_IDENTIFIER_PARTS: Final = 3
# Trailing colon-separated segments stripped to recover a base identifier (undoing suffixes like
# ":keep", ":planned:<anchor>").
IDENTIFIER_SUFFIX_SEGMENTS: Final = 2
# Positions of "false" and "threshold" in a qualifying (minimum, net, false, threshold, report) row.
NET_BENEFIT_FALSE_INDEX: Final = 2
NET_BENEFIT_THRESHOLD_INDEX: Final = 3
# Floor preventing log(0) in a training loss.
LOG_LOSS_PROBABILITY_FLOOR: Final = 1e-15
# One command family in this many is trained without its identifier evidence.
IDENTIFIER_DROPOUT_FAMILIES: Final = 3
# Lowest minimum_early_recall a prefix-v2 training config may set.
PREFIX_MIN_EARLY_RECALL_FLOOR: Final = 0.70
PREFIX_CALIBRATION_FAILED_EXIT_CODE: Final = 2
# Boundary model promotion gate: with no errors, at least this share of test rows decided correctly;
# first declared by the frozen tools/train_boundary_model.py.
BOUNDARY_PROMOTION_MIN_DECIDED_FRACTION: Final = 0.80
# The excluded version module must hold a docstring and one assignment, nothing else.
VERSION_MODULE_STATEMENT_COUNT: Final = 2
# The document cap times the three document groups (US, RU and correct-only, which has no layout
# group).
SEQUENCE_MAXIMUM_SELECTED_ROWS: Final = (LAYOUT_GROUP_COUNT + 1) * SEQUENCE_DOCUMENT_CAP_PER_GROUP
# Positions of trimmed_left/trimmed_right in case_evidence's `inputs` tuple.
TRIMMED_LEFT_FIELD: Final = 3
TRIMMED_RIGHT_FIELD: Final = 4
CONTEXT_V1_MINIMUM_TEST_ROWS: Final = 10_000
# Decimal places of the recall the ortho-v2 verifier recomputes and compares with the sealed report.
ORTHO_V2_RECALL_DECIMALS: Final = 6
# Below this many sequence ids, the engine comparison is too small to trust.
PREFIX_MIN_SELECTED_SEQUENCES: Final = 250
