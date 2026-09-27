"""Prefix-v1: the legacy early-switch rule, the schema-one prefix features, its corpus and trainer.

The corpus receipt and the seal of the installed prefix-v1 model pin the values their files
import from here (`keyswitch.value_provenance`) next to the bytes of those files, so a
changed value fails the prefix checks until the model is refit and sealed again.
"""

from __future__ import annotations

from typing import Final

from .units import BYTES_PER_MEBIBYTE

# The legacy early-switch rule (early_switch.EarlySwitchPolicy): a prefix impossible in its own
# language switches when its rendering in the other layout starts at least this many words, one of
# them at least this frequent, or one word dominant by frequency on its own.
EARLY_SWITCH_MIN_TARGET_COMPLETIONS: Final = 10
EARLY_SWITCH_MIN_TARGET_FREQUENCY: Final = 2000
EARLY_SWITCH_DOMINANT_TARGET_FREQUENCY: Final = 100_000
# A prefix index scans at most this many matching words for their best frequency; the count of
# matches is exact regardless.
PREFIX_INDEX_COMPLETION_SCAN_LIMIT: Final = 4096
# Prefix indexes built from a lexicon and its Hunspell stems that stay cached.
PREFIX_INDEX_CACHE_SIZE: Final = 8
# A Hunspell .dic larger than this adds no stems to a prefix index.
PREFIX_INDEX_DICTIONARY_MAX_BYTES: Final = 16 * BYTES_PER_MEBIBYTE

# feature_version of the schema-one prefix features (prefix_model.features) and of their artifact.
PREFIX_V1_FEATURE_VERSION: Final = 1
# Schema-one features: the one-hot length feature buckets every length from here up together; the
# continuous length feature is the length capped at and divided by the scale.
PREFIX_V1_LENGTH_BUCKET_MAX_CHARACTERS: Final = 12
PREFIX_V1_LENGTH_SCALE_CHARACTERS: Final = 24
# log1p of the completion count and of the best completion frequency is divided by these.
PREFIX_V1_COMPLETIONS_LOG_DIVISOR: Final = 10
PREFIX_V1_FREQUENCY_LOG_DIVISOR: Final = 20
# The word n-gram score is clamped to this magnitude and divided by it.
PREFIX_V1_NGRAM_SCORE_BOUND: Final = 3.0
# Text before the prefix that the context features read.
PREFIX_V1_BEFORE_CONTEXT_CHARACTERS: Final = 512
# Quote marks pair up: an odd count on the line leaves one open.
PREFIX_V1_QUOTE_MARKS_PER_PAIR: Final = 2
# Characters of the application identifier, and its alphanumeric tokens, that become "app:" features.
PREFIX_V1_APPLICATION_NAME_CHARACTERS: Final = 128
PREFIX_V1_APPLICATION_TOKEN_COUNT: Final = 4
# Last words of the text before the prefix that become "before:word:" features.
PREFIX_V1_BEFORE_WORD_COUNT: Final = 2

# The prefix-v1 corpus (tools/prefix_corpus.py). A family is the first this many physical keys of a
# word; the family's hash picks one of the buckets, the first ones are train and each later one is
# the next split (development, calibration, test).
PREFIX_FAMILY_KEY_CHARACTERS: Final = 3
PREFIX_SPLIT_BUCKET_COUNT: Final = 10
PREFIX_TRAIN_SPLIT_BUCKETS: Final = 7
# Source text before a word kept as its context.
PREFIX_CORPUS_BEFORE_CHARACTERS: Final = 160
# Prefixes up to this length are shared ambiguous `wait` examples, never actionable.
PREFIX_SHARED_WAIT_MAX_CHARACTERS: Final = 2

# The prefix-v1 trainer (tools/train_prefix_model.py): a feature enters the model when it occurs in
# at least this many train rows; progress is printed every this many epochs; a report quotes at most
# this many false or missed sequences.
PREFIX_MIN_FEATURE_OCCURRENCES: Final = 2
PREFIX_TRAINING_PROGRESS_EPOCHS: Final = 5
PREFIX_REPORT_MAX_EXAMPLES: Final = 15
