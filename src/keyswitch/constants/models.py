"""Facts about the runtime model artifacts: feature versions, weight caps, thresholds."""

from __future__ import annotations

from typing import Final

# The context model's features. Feature schema 3 (context_action_features) and the frozen feature
# schema 2 (context_model.extract_context_features, read by the installed context-v1 model and
# compared with its archived source by tools/verify_context_v2_history.py) share the values marked
# "schemas 2 and 3": changing one of them changes the shipped model's features too.
# Character n-gram orders extracted around each text span.
ACTION_FEATURE_NGRAM_ORDERS: Final = (1, 2, 3, 4)
# Cap on one accumulated character n-gram feature's weight.
ACTION_FEATURE_CHARACTER_WEIGHT_CAP: Final = 8.0
# A character feature name is `label:char:direction:order:text`: its n-gram text follows this many
# separators, so a split at most this many times leaves it whole at this index.
ACTION_FEATURE_CHARACTER_TEXT_FIELD_INDEX: Final = 4
# A WordScore's value, ngram_score and the value delta between two readings share the same
# log-probability-like scale; clamped to this range and divided by it so the feature stays in [-1,
# 1]. Schemas 2 and 3 (schema 2 bounds only the score delta).
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
# The "length" feature buckets every word length from here up together (schemas 2 and 3).
ACTION_FEATURE_LENGTH_BUCKET_MAX_CHARACTERS: Final = 6
# Short categorical field labels (role, trigger) truncated before they reach a feature name.
ACTION_FEATURE_FIELD_LABEL_MAX_CHARACTERS: Final = 32
# Right-context ("after") text considered for scripting/lexical features (schemas 2 and 3).
ACTION_FEATURE_AFTER_CONTEXT_CHARACTERS: Final = 128
# Left-context ("before") text considered for scripting/lexical features (schemas 2 and 3).
ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS: Final = 512
# Source/target token text truncated before character and script features (schemas 2 and 3).
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
# Application identifier considered for the "app:" features (schemas 2 and 3).
ACTION_FEATURE_APPLICATION_NAME_CHARACTERS: Final = 128
# Alphanumeric tokens of the application identifier turned into features (schemas 2 and 3).
ACTION_FEATURE_APPLICATION_TOKEN_COUNT: Final = 4
# The orthotactic model's score and threshold share this magnitude range.
ACTION_FEATURE_ORTHO_SCORE_BOUND: Final = 128.0
# ContextModel feature_version of the context-action (orthotactic-aware) scheme, feature schema 3.
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
# A built-in fallback or extra word gets the lexicon's top frequency divided by the divisor, but at
# least the floor (the floor alone for a lexicon without frequency data).
SYNTHETIC_FREQUENCY_FLOOR: Final = 1000
SYNTHETIC_FREQUENCY_DIVISOR: Final = 20
# Conversion threshold of the context-v1 model: ContextModel's default, and the threshold the
# context-v1 trainer selects its epoch with and writes into the artifact.
CONTEXT_V1_CONVERSION_THRESHOLD: Final = 0.985
# ContextModel feature_version of the context-v1/v2 scheme, feature schema 2; context_model.py
# exports it as FEATURE_VERSION.
CONTEXT_MODEL_FEATURE_VERSION: Final = 2
# ContextModel feature_version of the context-v1 scheme with typo evidence, feature schema 5: schema 2
# plus whether each reading is one typo away from a word the lexicon knows, and in which layout a
# token with a digit was typed. Schema 4 was an experiment that never shipped.
CONTEXT_TYPO_FEATURE_VERSION: Final = 5
# Schema 5: the shortest reading checked for being one typo away from a word; nearly every shorter
# string has a neighbour in a lexicon of this size.
CONTEXT_TYPO_MIN_CHARACTERS: Final = 5
# ContextModel feature_version of the context-v1 scheme with opening evidence, feature schema 6:
# schema 5 plus whether each reading is a word that opens sentences of its language
# (short_words.OPENING_WORDS). The frequency lexicons are encyclopedic and rank the conversational
# `ты` next to the English token `ns`, so without it a model could tell them apart only by letters.
CONTEXT_OPENING_FEATURE_VERSION: Final = 6
# ContextModel feature_version of the context-v1 scheme with line, case and term evidence, feature
# schema 7: schema 6 plus where on its line the word stands (the first word of a message or of a new
# line against a word with words before it), the case pattern of the token (an inner capital, all
# capitals), a dot or underscore inside its Latin reading, how often each reading occurs inside
# Russian text (a Latin term like `id`, Cyrillic slang like `пдф`) and in text of its own language
# (`uh` is an English word, `гр` a rare Russian one), and whether the token has letters at all - the
# questions the engine asks while real mixed Russian and English text is typed made these the
# evidence that decides.
CONTEXT_LINE_FEATURE_VERSION: Final = 7
# Schemas of the context-v1 trainer: feature 2, 5, 6 and 7 artifacts share its report, provenance and
# replay.
CONTEXT_V1_FEATURE_VERSIONS: Final = (
    CONTEXT_MODEL_FEATURE_VERSION, CONTEXT_TYPO_FEATURE_VERSION, CONTEXT_OPENING_FEATURE_VERSION,
    CONTEXT_LINE_FEATURE_VERSION,
)
# Schemas that carry every schema 6 feature (schema 7 adds its own on top).
CONTEXT_OPENING_SCHEMAS: Final = (CONTEXT_OPENING_FEATURE_VERSION, CONTEXT_LINE_FEATURE_VERSION)
# Schema 7: the occurrence counts that open the term frequency buckets above the first, in every
# table; a reading seen fewer times than the first bound is in bucket 0, as one never seen.
CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS: Final = (5, 50, 500, 5000)
# Feature schemas a context artifact may carry: context-v1's and the context action scheme's.
CONTEXT_SUPPORTED_FEATURE_VERSIONS: Final = (*CONTEXT_V1_FEATURE_VERSIONS, CONTEXT_ACTION_FEATURE_VERSION)
# Feature schema 2 only: words of the left and of the right context that become word features, and
# the character n-gram orders of the two readings.
CONTEXT_FEATURE_BEFORE_WORD_COUNT: Final = 6
CONTEXT_FEATURE_AFTER_WORD_COUNT: Final = 3
CONTEXT_FEATURE_NGRAM_ORDERS: Final = (1, 2, 3)
# What the context model loader accepts (schemas 2 and 3): at most this many features, feature names
# and version strings of at most these many characters, weights of at most this magnitude, and a
# conversion threshold from this floor up to one.
MAX_CONTEXT_MODEL_FEATURES: Final = 50000
MAX_CONTEXT_FEATURE_NAME_CHARACTERS: Final = 512
MAX_CONTEXT_MODEL_VERSION_CHARACTERS: Final = 80
MAX_CONTEXT_WEIGHT_MAGNITUDE: Final = 1000
MIN_CONTEXT_CONVERSION_THRESHOLD: Final = 0.95
# Feature schema of the intent model (the KSLM manifest's feature_version).
INTENT_FEATURE_VERSION: Final[int] = 5
# The last intent feature schema verified to ignore the context grid; the trainer's fast context
# stress relies on it and refuses any other schema.
INTENT_CONTEXT_INVARIANT_FEATURE_VERSION: Final[int] = 5
# FNV-1a, 64-bit: the offset basis (also the intent model's default feature-hash seed) and the prime.
FNV1A64_OFFSET_BASIS: Final[int] = 0xCBF29CE484222325
FNV1A64_PRIME: Final[int] = 0x100000001B3
# Default seed of the intent model's exact-name membership hash; it must differ from the feature
# seed.
INTENT_MEMBERSHIP_FNV_SEED: Final[int] = 0x9E3779B97F4A7C15
# The top bit of a 64-bit feature hash picks the feature's collision sign.
FEATURE_HASH_SIGN_BIT: Final[int] = 1 << 63
# Character n-gram orders of the intent model's features; KSLM schema 4 requires exactly these.
INTENT_NGRAM_ORDERS: Final[tuple[int, ...]] = (1, 2, 3, 4, 5)
# A token is cut to this many characters before normalisation, bounding CPU and memory.
INTENT_RAW_TOKEN_MAX_CHARACTERS: Final[int] = 64
# The intent model decides only when the longer normalised reading has at least this many
# characters.
INTENT_MIN_RUNTIME_TOKEN_CHARACTERS: Final[int] = 5
# Largest magnitude of a number a KSLM manifest carries (weight scale, calibration, thresholds).
INTENT_MAX_MODEL_FLOAT_MAGNITUDE: Final[float] = 1_000_000.0
INTENT_MODEL_VERSION_MAX_CHARACTERS: Final[int] = 128
# Deepest nesting of KSLM manifest metadata.
INTENT_METADATA_MAX_DEPTH: Final[int] = 16
# Layout groups are clamped into -1 through this before they reach a feature name.
INTENT_MAX_FEATURE_GROUP: Final[int] = 63
# The intent model's word-length feature: every length up to the first bound is its own bucket, then
# one bucket ends at each further bound, and longer words share the last one. The trainer's
# diagnostic slices use the same bounds.
INTENT_LENGTH_EXACT_MAX_CHARACTERS: Final[int] = 4
INTENT_LENGTH_SHORT_MAX_CHARACTERS: Final[int] = 7
INTENT_LENGTH_MEDIUM_MAX_CHARACTERS: Final[int] = 11
INTENT_LENGTH_LONG_MAX_CHARACTERS: Final[int] = 19
# Character n-gram orders of the language model's smoothed naturalness score.
LANGUAGE_MODEL_NGRAM_ORDERS: Final[tuple[int, ...]] = (2, 3, 4)
# The n-gram order whose coverage gives a WordScore's gram_ratio and invalid_ratio.
LANGUAGE_MODEL_TRIGRAM_ORDER: Final[int] = 3
# Shortest word a language model reads from its lexicon or counts into its n-grams.
LANGUAGE_MODEL_MIN_WORD_CHARACTERS: Final[int] = 2
# Naturalness is normalised against the most frequent words: at most this many, each at least this
# long. Without any, this mean is assumed; a deviation below the floor counts as one.
LANGUAGE_MODEL_CALIBRATION_WORD_LIMIT: Final[int] = 12_000
LANGUAGE_MODEL_CALIBRATION_MIN_CHARACTERS: Final[int] = 3
LANGUAGE_MODEL_DEFAULT_NGRAM_MEAN: Final[float] = -10.0
LANGUAGE_MODEL_MIN_NGRAM_DEVIATION: Final[float] = 0.05
# Loaded language models kept, one per locale and extra-word set.
LANGUAGE_MODEL_CACHE_SIZE: Final[int] = 16
# Word scores a language model caches; the intent trainer's train-only scorer caches as many.
LANGUAGE_MODEL_SCORE_CACHE_MAXSIZE: Final[int] = 65_536
# Cap on the log-scaled weight one word adds to its character n-gram counts.
LANGUAGE_MODEL_MAX_GRAM_WEIGHT: Final[int] = 32
# Additive smoothing of n-gram probabilities: the pseudo-count of a gram and the unseen grams added
# to the vocabulary.
LANGUAGE_MODEL_GRAM_SMOOTHING: Final[float] = 0.2
LANGUAGE_MODEL_UNSEEN_GRAM_VOCABULARY: Final[int] = 2048
# Score of a token with no letters: its WordScore value and its raw n-gram score.
LANGUAGE_MODEL_EMPTY_TOKEN_SCORE: Final[float] = -30.0
# Naturalness (the normalised n-gram score) is clamped to this range. A known word's naturalness is
# raised to at least the floor, and the detector accepts a target at the floor as a rare valid form.
LANGUAGE_MODEL_NATURALNESS_MIN: Final[float] = -15.0
LANGUAGE_MODEL_NATURALNESS_MAX: Final[float] = 4.0
LANGUAGE_MODEL_KNOWN_WORD_NATURALNESS_FLOOR: Final[float] = -4.0
# A WordScore's value: a lexical score, plus the naturalness weight times naturalness, minus the
# invalid-ratio weight times the invalid ratio. The lexical score is the base plus the popularity
# weight times popularity for a frequency-lexicon word, or the Hunspell-only score.
LANGUAGE_MODEL_EXACT_WORD_BASE_SCORE: Final[float] = 7.0
LANGUAGE_MODEL_POPULARITY_WEIGHT: Final[float] = 3.0
LANGUAGE_MODEL_SPELL_KNOWN_SCORE: Final[float] = 6.5
LANGUAGE_MODEL_NATURALNESS_WEIGHT: Final[float] = 1.15
LANGUAGE_MODEL_INVALID_RATIO_WEIGHT: Final[float] = 0.75
# best_single_deletion() leaves words shorter than this alone and tries at most this many evenly
# spaced positions.
LANGUAGE_MODEL_DELETION_MIN_CHARACTERS: Final[int] = 4
LANGUAGE_MODEL_DELETION_POSITIONS: Final[int] = 12
