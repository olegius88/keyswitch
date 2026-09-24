"""Facts about the runtime model artifacts: feature versions, weight caps, thresholds."""

from __future__ import annotations

from typing import Final

# Character n-gram orders extracted around each text span.
ACTION_FEATURE_NGRAM_ORDERS: Final = (1, 2, 3, 4)
# Cap on one accumulated character n-gram feature's weight.
ACTION_FEATURE_CHARACTER_WEIGHT_CAP: Final = 8.0
# A WordScore's value, ngram_score and the value delta between two readings share the same
# log-probability-like scale; clamped to this range and divided by it so the feature stays in [-1,
# 1].
ACTION_FEATURE_WORD_SCORE_BOUND: Final = 10.0
# Magnitude a raw n-gram score is clamped to and divided by.
ACTION_FEATURE_RAW_NGRAM_SCORE_BOUND: Final = 32.0
# Frequencies are log-scaled against this cap so one very common word cannot dominate the feature.
ACTION_FEATURE_FREQUENCY_CAP: Final = 10**12
# `after_origin == "planned_next_conversion"` is only reachable for a short waiting word with a
# short, single-line planned right context.
PLANNED_CONTEXT_WORD_MAX_CHARACTERS: Final = 2
# Longest single-line planned right context a planned_next_conversion origin allows.
PLANNED_CONTEXT_AFTER_MAX_CHARACTERS: Final = 64
# The "length" feature buckets every word length from here up together.
ACTION_FEATURE_LENGTH_BUCKET_MAX_CHARACTERS: Final = 6
# Short categorical field labels (role, trigger) truncated before they reach a feature name.
ACTION_FEATURE_FIELD_LABEL_MAX_CHARACTERS: Final = 32
# Right-context ("after") text considered for scripting/lexical features.
ACTION_FEATURE_AFTER_CONTEXT_CHARACTERS: Final = 128
# Left-context ("before") text considered for scripting/lexical features.
ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS: Final = 512
# Source/target token text truncated before character and script features.
ACTION_FEATURE_WORD_MAX_CHARACTERS: Final = 64
# Neighbouring words taken from the left or right context.
ACTION_FEATURE_NEIGHBOUR_WORD_COUNT: Final = 2
# Characters of a neighbouring word used for its character features.
ACTION_FEATURE_NEIGHBOUR_WORD_MAX_CHARACTERS: Final = 24
# Whitespace run between the word and its neighbour, truncated and used to normalise the
# whitespace-shape features.
ACTION_FEATURE_WHITESPACE_MAX_CHARACTERS: Final = 16
# The literal tail and the boundary character(s) are both short trailing punctuation buffers
# normalised the same way.
ACTION_FEATURE_SHORT_TEXT_CHARACTERS: Final = 8
# Application identifier considered for the "app:" features.
ACTION_FEATURE_APPLICATION_NAME_CHARACTERS: Final = 128
# Alphanumeric tokens of the application identifier turned into features.
ACTION_FEATURE_APPLICATION_TOKEN_COUNT: Final = 4
# The orthotactic model's score and threshold share this magnitude range.
ACTION_FEATURE_ORTHO_SCORE_BOUND: Final = 128.0
# ContextModel feature_version of the context-action (orthotactic-aware) scheme; the frozen
# context_model.py spells it as a bare 3, so it is named here.
CONTEXT_ACTION_FEATURE_VERSION: Final[int] = 3
# Shortest prefix the engine offers the prefix model; prefix training mirrors this support.
PREFIX_MIN_CHARACTERS: Final = 4
# Longest prefix the engine offers the prefix model, the characters prefix features (schema two)
# encode, and the longest prefix a prefix corpus may generate.
PREFIX_MAX_CHARACTERS: Final = 12
IDENTIFIER_LEXICON_CACHE_SIZE: Final = 4
LEXICON_SUPPLEMENT_CACHE_SIZE: Final = 8
# Newest prefix feature schema; schema one stays frozen in prefix_model.py.
CURRENT_PREFIX_FEATURE_VERSION: Final = 2
# Longest character n-gram of an observed prefix (schema two).
MAX_PREFIX_CHARACTER_NGRAM_ORDER: Final = 4
PREFIX_CHARACTER_FEATURE_WEIGHT_CAP: Final = 2.0
# Lowest conversion threshold a prefix model artifact may carry.
MIN_PREFIX_CONVERSION_THRESHOLD: Final = 0.985
MAX_PREFIX_WEIGHTS: Final = 10000
MAX_PREFIX_FEATURE_NAME_CHARACTERS: Final = 160
# Mirrors the synthetic-frequency fallback in keyswitch.language_model so a lexicon without
# frequency data still ranks below any real observed word.
SYNTHETIC_FREQUENCY_FLOOR: Final = 1000
SYNTHETIC_FREQUENCY_DIVISOR: Final = 20
# Conversion threshold of the context-v1 model; mirrors the frozen ContextModel.__init__ default.
CONTEXT_V1_CONVERSION_THRESHOLD: Final = 0.985
# ContextModel feature_version of the context-v1/v2 scheme; mirrors FEATURE_VERSION of the frozen
# context_model.py.
CONTEXT_MODEL_FEATURE_VERSION: Final = 2
