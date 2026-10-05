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
# these as seeds. The context-v1 corpus defers such a token standing alone and teaches no conversion
# of one in a Latin field.
SHORT_WORD_MAX_CHARACTERS: Final = 2
# The context-action model (schema 3) has its own threshold so that the context-v1 corpus above keeps
# its frozen evidence whatever this one measures. Two: with two, the corpus v10 and v11 candidates
# (01.10.2026) turned `зум` alone into `pev` and left `tot` before `привет`; with three, the corpus
# v12 candidate kept `зум` but still left `tot привет`, no longer corrected `rjn` by the pause and
# turned `pm2` into `зь2` - the trade the author measured on 17.09.2026, confirmed.
ACTION_SHORT_WORD_MAX_CHARACTERS: Final = 2
# An isolated reading of three letters is deferred only when its intent is not observable: both
# readings are plausible, a lexicon word or a shipped identifier each (`tot`/`еще`, `зум`/`pev` - the
# Debian command `pev` is why the v10 and v11 candidates converted `зум`). One plausible reading
# decides at once, which is what keeps `rjn` converting by the pause, `три` standing and `зь2`
# becoming `pm2` (neither reading is plausible, the model's own evidence decides). Measured on
# corpus v12 (02.10.2026): 64 of the three-letter Debian commands read as a known Russian form.
ACTION_DEFERRED_WORD_MAX_CHARACTERS: Final = 3
# The lexical short-pair curriculum (train_context_action_model.lexical_short_pair_rows): natural
# text rarely holds a three-letter word whose other reading is a word too (corpus v12 TRAIN: 21 rows
# in 45 544, almost all Debian commands), so every such pair of the pinned lexicons and the shipped
# identifier index enters TRAIN with this many planned-neighbour variants per member, each carrying
# an equal share of the member's unit mass.
LEXICAL_PAIR_ANCHOR_VARIANTS: Final = 3
# Opening signs of a quoted Latin citation, typed in the Latin layout, whose keys are letters
# in the Russian layout: `` is ёё, ' is э, " is Э. A citation that carries one is still the
# citation, and its Cyrillic reading still no word.
CITATION_SIGN_HEADS: Final = ("``", "'", '"')
# A Russian word closing a quotation, typed with its quote in the layout of the word: the Russian
# `"` is the `@` key, so `привет"` typed in the Latin layout is `ghbdtn@`. One Russian row in this
# many gets the closing-quote pair, chosen by hash.
QUOTE_TAIL_MODULUS: Final = 4
QUOTE_TAIL: Final = '"'
# Letters of a previous word left in the wrong layout that the stranded-previous frames take: a
# lone letter waits for its neighbour and is no word left behind.
STRANDED_PREVIOUS_MIN_LETTERS: Final = 2
# Sample weight of a stranded-previous frame: natural text holds few of them (corpus v23 TRAIN:
# 384 rows), and at unit weight the authored `руку ерун` stayed under the threshold (p=0.987).
STRANDED_PREVIOUS_WEIGHT: Final = 2.0
# Kept-neighbour frames (train_context_action_model.kept_neighbour_curriculum): the shortest waiting
# word they frame (a lone letter is decided by the curated rules, not by the model), the share of
# terms framed in capitals (`ДДЬ` for `LLM`), one in this many by hash, and the most frames one term,
# abbreviation or misspelt word gets, each after a different Russian row.
KEPT_NEIGHBOUR_MIN_LETTERS: Final = 2
KEPT_NEIGHBOUR_CAPITALS_MODULUS: Final = 4
KEPT_NEIGHBOUR_FRAMES_PER_WORD: Final = 3
# Letters of a Latin abbreviation cited in capitals inside Russian prose whose Cyrillic reading is a
# rare word (`WBC` is `ЦИС`, `IBF` is `ШИА`): three to five, as abbreviations are written. Two
# letters stay with the deferral rules and the owner's own Cyrillic abbreviations (`ГА`).
CAPITAL_CITATION_MIN_LETTERS: Final = 3
CAPITAL_CITATION_MAX_LETTERS: Final = 5
# The frequency bucket of a Russian word counted fewer than the first bound in Russian text
# (CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS): such a word written in capitals is no abbreviation of it.
CAPITAL_CITATION_RUSSIAN_BUCKET: Final = "0"
# The marks a punctuation-triggered capital citation ends with.
CAPITAL_CITATION_PUNCTUATION: Final = ",."
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
# Decimal places fitted weights, feature masses and a reported development loss are rounded to
# before hashing, ranking or reporting, so results do not depend on the platform.
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
# The context-v1 trainer (tools/train_context_model.py): AdaGrad epochs and step, and the importance
# of `keep` rows on top of the inverse class frequency. The epochs are the earliest at which the
# development budget (no false conversion) is met and every authored engine test passes, chosen on
# disclosed data: for feature schema 6 (28.09.2026) development converts a correct `if` after
# English text through the 8th epoch and nothing correct from the 9th. Before schema 6 saw a slash, each
# further epoch converted more correctly typed package names after a path (npm names: 128 of 23,664
# at the 8th against 166 at the 14th for schema 5), and the holdout rejected a 14-epoch schema-5
# model and a 10-epoch schema-6 corpus without the slash for three such words after a slash.
CONTEXT_V1_EPOCHS: Final = 9
CONTEXT_V1_LEARNING_RATE: Final = 0.2
CONTEXT_V1_KEEP_IMPORTANCE: Final = 1.0
# The features a context-v1 model trained with the engine's captured questions keeps: the most
# frequent ones across its training rows, below the loader's limit (MAX_CONTEXT_MODEL_FEATURES). The
# captured questions bring in hundreds of thousands of rare letter n-grams; keeping the 48 000 most
# frequent cost nothing measurable against keeping all of them (research-2026-09-29, M3 against M1).
CONTEXT_V1_MAX_TRAINED_FEATURES: Final = 48000
# Its family split: a signature's hash picks one of the buckets; the first ones are train, the rest
# development.
CONTEXT_V1_SPLIT_BUCKET_COUNT: Final = 10
CONTEXT_V1_TRAIN_SPLIT_BUCKETS: Final = 8
# Russian and English words its corpus types after every slash head (technical
# terms are all typed there). With ten, a command name after a Russian path
# segment (`код/dpkg`) read as a Russian word typed in the wrong layout.
CONTEXT_V1_SLASH_WORDS_PER_FAMILY: Final = 30
# Words its corpus types with letters put into their middle in the other layout (feature schema 6
# tells the model so): a word at least this long has a letter or two between a head and a tail, as
# the mid-word edits of the package replay have.
CONTEXT_V1_INSIDE_WORD_MIN_CHARACTERS: Final = 4
# Mismatched rows its report quotes for a split.
CONTEXT_V1_MAX_REPORTED_FAILURES: Final = 30
# Decimal places of the recall the ortho-v2 verifier recomputes and compares with the sealed report.
ORTHO_V2_RECALL_DECIMALS: Final = 6
# Below this many sequence ids, the engine comparison is too small to trust.
PREFIX_MIN_SELECTED_SEQUENCES: Final = 250
# The intent corpus split: a physical signature's hash picks one of the bucket count, and the splits
# take consecutive runs of this many buckets each (train, development, calibration, threshold,
# test).
INTENT_TRAIN_SPLIT_BUCKETS: Final[int] = 26
INTENT_DEVELOPMENT_SPLIT_BUCKETS: Final[int] = 4
INTENT_CALIBRATION_SPLIT_BUCKETS: Final[int] = 4
INTENT_THRESHOLD_SPLIT_BUCKETS: Final[int] = 3
INTENT_TEST_SPLIT_BUCKETS: Final[int] = 3
INTENT_SPLIT_BUCKET_COUNT: Final[int] = (
    INTENT_TRAIN_SPLIT_BUCKETS
    + INTENT_DEVELOPMENT_SPLIT_BUCKETS
    + INTENT_CALIBRATION_SPLIT_BUCKETS
    + INTENT_THRESHOLD_SPLIT_BUCKETS
    + INTENT_TEST_SPLIT_BUCKETS
)
# Leading SHA-256 bytes (64 bits) the intent trainer turns into a split bucket or a trigger choice.
INTENT_DRAW_DIGEST_BYTES: Final[int] = 8
# Words shorter than this stay out of the intent safety-collision corpus.
SAFETY_COLLISION_MIN_WORD_CHARACTERS: Final[int] = 3
# Wilson score interval of a false-positive rate: the confidence of its upper bound and the
# two-sided z-score of that confidence. The selection z-score splits the error over its
# false-positive gates, NormalDist().inv_cdf(1 - (1 - 0.95) / (2 * 12)), pinned rather than
# recomputed so signed reports remain byte-reproducible across Python builds.
WILSON_INTERVAL_CONFIDENCE: Final[float] = 0.95
WILSON_95_Z_SCORE: Final[float] = 1.959963984540054
SELECTION_WILSON_Z_SCORE: Final[float] = 2.8652602385321333
# The Wilson interval's centre is p + z^2 / (2n) and its radius z * sqrt(p(1 - p) / n + z^2 / (4n^2)).
WILSON_CENTRE_DENOMINATOR_FACTOR: Final[float] = 2.0
WILSON_RADIUS_DENOMINATOR_FACTOR: Final[float] = 4.0
# Threshold selection has one overall and one typo-tail false-positive gate for each trigger.
FALSE_POSITIVE_GATES_PER_TRIGGER: Final[int] = 2
# The intent training sources: the license evidence and the EN and RU lexicons, three distinct files.
INTENT_FROZEN_SOURCE_FILE_COUNT: Final[int] = 3
# An external evaluation corpus has at least this many words per language group; the corpus
# builders default to it.
INTENT_EXTERNAL_MIN_WORDS_PER_GROUP: Final[int] = 5_000
# Allowed range of hard_negative_development.training_example_weight.
HARD_NEGATIVE_MIN_EXAMPLE_WEIGHT: Final[float] = 0.25
HARD_NEGATIVE_MAX_EXAMPLE_WEIGHT: Final[float] = 8.0
# Longest string field of the frozen hard-negative corpus.
HARD_NEGATIVE_CORPUS_STRING_MAX_CHARACTERS: Final[int] = 256
# Smallest feature-hash dimension an intent training config may choose.
INTENT_MIN_TRAINING_DIMENSION: Final[int] = 256
# The intent config's minimum_word_length, or its override, may not go below this.
INTENT_MIN_WORD_LENGTH_FLOOR: Final[int] = 2
# typo_augmentations asks for up to this many physical-key typos (deletion, duplication,
# transposition) per word; the evaluator's unknown-typo corpus takes all of them.
INTENT_MAX_TYPO_AUGMENTATIONS: Final[int] = 3
# Upper bound of the intent config's veto_positive_quantile.
INTENT_MAX_VETO_POSITIVE_QUANTILE: Final[float] = 0.1
# The intent trainer's physical-key typos edit an interior key: a signature needs at least this many
# keys, and this many edge keys (the first and the last) are never edited.
INTENT_TYPO_MIN_SIGNATURE_CHARACTERS: Final[int] = 3
INTENT_TYPO_EDGE_CHARACTERS: Final[int] = 2
# How long the C compiler may take to report its version for the native FTRL kernel's provenance.
COMPILER_VERSION_TIMEOUT_SECONDS: Final[int] = 30
# Context stress profiles shift the context delta by these magnitudes, each in both signs, for the
# source and for the target group.
CONTEXT_STRESS_EXTREME_DELTA: Final[float] = 6.0
CONTEXT_STRESS_OUTER_DELTA: Final[float] = 1.25
CONTEXT_STRESS_INNER_DELTA: Final[float] = 0.75
CONTEXT_STRESS_NEAR_ZERO_DELTA: Final[float] = 0.125
# algorithm_version of the intent trainer's train-only character-n-gram scorer.
TRAIN_ONLY_SCORER_ALGORITHM_VERSION: Final[int] = 2
# A lexical example weighs one plus its log frequency over the divisor, the addition capped.
EXAMPLE_FREQUENCY_WEIGHT_CAP: Final[float] = 2.0
EXAMPLE_FREQUENCY_LOG_DIVISOR: Final[float] = 8.0
# Weights of the synthetic protected-token negatives and of the lexical-collision negatives.
PROTECTED_TOKEN_EXAMPLE_WEIGHT: Final[float] = 2.0
LEXICAL_COLLISION_EXAMPLE_WEIGHT: Final[float] = 3.0
# Offending signatures quoted in a split-leakage or quarantine error.
LEAKAGE_ERROR_EXAMPLE_COUNT: Final[int] = 3
# Rows of the native FTRL kernel's self-check against the Python kernel.
FTRL_NATIVE_SELF_CHECK_ROWS: Final[int] = 4096
# The native FTRL kernel takes int64 row offsets, int32 feature indices and double state; rows and
# features stay below 2 ** FTRL_NATIVE_INDEX_LIMIT_LOG2.
FTRL_NATIVE_OFFSET_BYTES: Final[int] = 8
FTRL_NATIVE_INDEX_BYTES: Final[int] = 4
FTRL_NATIVE_DOUBLE_BYTES: Final[int] = 8
FTRL_NATIVE_INDEX_LIMIT_LOG2: Final[int] = 31
FTRL_NATIVE_INDEX_LIMIT: Final[int] = 2**FTRL_NATIVE_INDEX_LIMIT_LOG2
# Hex characters of the kernel source digest that name its build cache directory.
FTRL_NATIVE_CACHE_DIGEST_CHARACTERS: Final[int] = 16
# Platt calibration by damped Newton steps: the L2 penalty is this factor times l2 times the squared
# parameters; curvature and determinant have floors; a step below the tolerance ends the fit; the
# line search shrinks the step by the factor at most this many times.
CALIBRATION_L2_PENALTY_FACTOR: Final[float] = 0.5
CALIBRATION_MIN_CURVATURE: Final[float] = 1e-12
CALIBRATION_MIN_DETERMINANT: Final[float] = 1e-18
CALIBRATION_STEP_TOLERANCE: Final[float] = 1e-10
CALIBRATION_LINE_SEARCH_STEPS: Final[int] = 30
CALIBRATION_LINE_SEARCH_SHRINK: Final[float] = 0.5
# Slack a rate is allowed against its precision floor or false-positive bound, so a rate equal to its
# bound up to rounding passes.
RATE_BOUND_TOLERANCE: Final[float] = 1e-15
# Position of the typo-tail metrics in a (threshold, metrics, typo metrics) candidate.
THRESHOLD_CANDIDATE_TYPO_METRICS_INDEX: Final = 2
# A bisection halves its interval.
BISECTION_DIVISOR: Final[int] = 2
# Decimal places of reported intent precision, recall, specificity and rates.
INTENT_METRIC_DECIMALS: Final[int] = 9
# Word-frequency bounds of the intent trainer's diagnostic slices.
DIAGNOSTIC_FREQUENCY_FIRST_BOUND: Final[int] = 10
DIAGNOSTIC_FREQUENCY_SECOND_BOUND: Final[int] = 100
DIAGNOSTIC_FREQUENCY_THIRD_BOUND: Final[int] = 1_000
# Sample sizes of the intent strict evaluation's fallback comparison and latency measurement.
STRICT_COMPARISON_SAMPLE: Final[int] = 5_000
STRICT_LATENCY_SAMPLE: Final[int] = 5_000
# The dataset_sha256 check follows the artifact and config checks in the provenance report.
PROVENANCE_DATASET_CHECK_POSITION: Final[int] = 2
# Leading .aff lines the intent evaluator scans for the declared encoding. Same purpose as
# HUNSPELL_AFFIX_HEADER_SCAN_LINES of the detector evaluation but a different number; the sealed
# intent evaluator keeps its own until the two are reconciled.
INTENT_HUNSPELL_AFFIX_HEADER_SCAN_LINES: Final[int] = 80
# Longest Hunspell word an external evaluation corpus takes.
EXTERNAL_WORD_MAX_CHARACTERS: Final[int] = 24
# Longest rank or choice namespace of the unknown-typo corpus.
CORPUS_NAMESPACE_MAX_CHARACTERS: Final[int] = 128
# Each physical signature of a paired external corpus yields, for every trigger, a kept (negative)
# and a converted (positive) row.
PAIRED_ROWS_PER_TRIGGER: Final[int] = 2
# Percentiles of the coverage statistics; latency reports the ninety-fifth.
FIFTH_PERCENTILE: Final[float] = 0.05
FIRST_QUARTILE: Final[float] = 0.25
THIRD_QUARTILE: Final[float] = 0.75
NINETY_FIFTH_PERCENTILE: Final[float] = 0.95
# Latency measurement: model loads timed, warm-up predictions, decimals kept, and the strict gates
# on the load and inference p95.
LATENCY_LOAD_REPETITIONS: Final[int] = 11
LATENCY_WARMUP_PREDICTIONS: Final[int] = 100
LATENCY_MS_DECIMALS: Final[int] = 6
STRICT_LOAD_P95_MAX_MS: Final[float] = 500.0
STRICT_INFERENCE_P95_MAX_MS: Final[float] = 10.0
# The model's recall may trail the contextual fallback's by at most this much.
CONTEXTUAL_RECALL_TOLERANCE: Final[float] = 0.005
# Each Hunspell snapshot file is read, bounded, this many times: before the dictionary handle opens
# and after.
HUNSPELL_SNAPSHOT_READS_PER_FILE: Final[int] = 2
# The rejected context-v2 trainer (tools/train_context_v2.py) and its native kernel
# (tools/context_optimizer.py): hex characters of the kernel digest naming its build cache, and
# false conversions its report quotes per track.
CONTEXT_OPTIMIZER_CACHE_DIGEST_CHARACTERS: Final[int] = 16
CONTEXT_V2_MAX_REPORTED_FALSE_CONVERSIONS: Final = 12
# Its engine replay (tools/evaluate_context_engine.py): distinct test phrase groups per locale, the
# phrase length window, the serial of the first synthetic key, and failures the report quotes per
# variant.
CONTEXT_V2_ENGINE_ROWS_PER_LOCALE: Final = 64
CONTEXT_V2_ENGINE_PHRASE_MIN_CHARACTERS: Final = 8
CONTEXT_V2_ENGINE_PHRASE_MAX_CHARACTERS: Final = 96
CONTEXT_V2_ENGINE_FIRST_KEY_SERIAL: Final = 100
CONTEXT_V2_ENGINE_MAX_REPORTED_FAILURES: Final = 12
# The rejected ortho-v2 trainer (tools/train_ortho_v2.py): additive smoothing of the unigram and of
# the case-shape channels, rounding of sealed thresholds, of the scores and of the false-conversion
# rates it reports, the scale of those rates (false conversions per this many negative
# occurrences), the worst false conversions it quotes, and how much of its printed report it shows.
ORTHO_V2_UNIGRAM_PSEUDO_COUNT: Final = 0.5
ORTHO_V2_SHAPE_PSEUDO_COUNT: Final = 0.5
ORTHO_V2_SEALED_THRESHOLD_DECIMALS: Final = 6
ORTHO_V2_REPORTED_SCORE_DECIMALS: Final = 4
ORTHO_V2_FALSE_RATE_DECIMALS: Final = 4
ORTHO_V2_FALSE_RATE_SCALE: Final = 10_000
ORTHO_V2_REPORTED_WORST_FALSE: Final = 10
ORTHO_V2_PRINTED_REPORT_MAX_CHARACTERS: Final = 4000
# Share of wanted conversions the rejected ortho-v2 weights separate at an ideal threshold, as its
# report states it.
ORTHO_V2_IDEAL_SEPARATION_PERCENT: Final = 99.7
