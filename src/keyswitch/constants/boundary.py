"""Completed-token boundaries: the boundary-v1/v2 features and artifacts, the v2 corpus, both trainers.

The corpus receipt and the seal of the installed boundary-v2 model pin the values their files
import from here (`keyswitch.value_provenance`) next to the bytes of those files, so a changed
value fails the boundary-v2 checks until the model is refit and sealed again. The rejected
boundary-v1 experiment is replayed with the same live code (`verify_boundary_model.py
--verify-frozen`), so a changed value it reads fails that replay as well.
"""

from __future__ import annotations

from typing import Final

from .units import BYTES_PER_KIBIBYTE

# Keys that are punctuation in the US layout and letters in the Russian one. A completed token's
# trailing run of them is its ambiguous tail; the v2 features list them in this order.
BOUNDARY_AMBIGUOUS_PUNCTUATION: Final = ",.;[]'`"
# A completed token keeps at most this many literal trailing characters; a longer ambiguous tail is
# not segmented (boundary_model.MAX_SUFFIX).
BOUNDARY_MAX_LITERAL_SUFFIX_CHARACTERS: Final = 8

# feature_version of the boundary-v1 features and artifact (boundary_model) and of the boundary-v2
# ones (boundary_policy).
BOUNDARY_V1_FEATURE_VERSION: Final = 1
BOUNDARY_V2_FEATURE_VERSION: Final = 2
# Feature version 3 adds, for each reading, whether it is a known word with one letter missing (not the
# last one): a misspelling in the sense the boundary-v2 corpus generates them. The policy loads both
# versions and extracts the features of the version its artifact was trained on.
BOUNDARY_V3_FEATURE_VERSION: Final = 3
BOUNDARY_SUPPORTED_FEATURE_VERSIONS: Final = (BOUNDARY_V2_FEATURE_VERSION, BOUNDARY_V3_FEATURE_VERSION)
# Letters a misspelled reading may be missing, by locale of the word model that reads it.
BOUNDARY_MISSING_LETTERS: Final = {
    "en_US": "abcdefghijklmnopqrstuvwxyz",
    "ru_RU": "абвгдеёжзийклмнопрстуфхцчшщъыьэюя",
}
# Every boundary-v2 artifact version starts with this; the trainer appends a digest of the weights.
BOUNDARY_V2_VERSION_PREFIX: Final = "boundary-v2-"
# Candidate features: the word length is capped at and divided by the scale; log1p of the word
# frequency is divided by the divisor; the n-gram score is clamped to the bound and divided by it.
BOUNDARY_FEATURE_LENGTH_SCALE_CHARACTERS: Final = 24
BOUNDARY_FEATURE_FREQUENCY_LOG_DIVISOR: Final = 20
BOUNDARY_FEATURE_NGRAM_SCORE_BOUND: Final = 3.0
# An artifact's threshold must exceed one half: a decided span then outweighs all others together.
BOUNDARY_THRESHOLD_EXCLUSIVE_MIN: Final = 0.5
# A boundary-v2 artifact larger than this is refused unread.
BOUNDARY_POLICY_MAX_BYTES: Final = 64 * BYTES_PER_KIBIBYTE
# Fitted boundary weights (v1 and v2) are rounded to this many decimal places before hashing.
BOUNDARY_WEIGHT_DECIMALS: Final = 12

# The boundary-v2 corpus (tools/boundary_v2_corpus.py). A family is the first this many physical
# keys of a word; the family's hash picks one of the buckets: the first ones are train, the next one
# development, the one after calibration, the rest test.
BOUNDARY_V2_FAMILY_KEY_CHARACTERS: Final = 4
BOUNDARY_V2_SPLIT_BUCKET_COUNT: Final = 10
BOUNDARY_V2_TRAIN_SPLIT_BUCKETS: Final = 6
# Lexicon words of these lengths become examples.
BOUNDARY_V2_WORD_MIN_CHARACTERS: Final = 3
BOUNDARY_V2_WORD_MAX_CHARACTERS: Final = 24
# Literal suffixes typed after a word: every word gets each of them, a misspelled word one of them.
BOUNDARY_V2_LITERAL_SUFFIXES: Final = (",", ".", ";", "[", "]", "...", "'", "`")
# A word at least this long also gets a typo variant when the leading hex digits of its hash, read
# as a number, divide by the modulus; the typo drops the key at a position its hash picks.
BOUNDARY_V2_TYPO_MIN_CHARACTERS: Final = 7
BOUNDARY_V2_TYPO_CHOICE_HEX_DIGITS: Final = 2
BOUNDARY_V2_TYPO_SAMPLE_MODULUS: Final = 7
# The boundary-v2 trainer prints the development loss every this many epochs.
BOUNDARY_V2_TRAINING_PROGRESS_EPOCHS: Final = 10

# The rejected boundary-v1 experiment (tools/train_boundary_model.py), kept replayable: its family
# key and split (the first buckets train, then one development, one calibration, the rest test),
# word lengths, AdaGrad epochs and step, and the calibration thresholds tried in order.
BOUNDARY_V1_FAMILY_KEY_CHARACTERS: Final = 4
BOUNDARY_V1_SPLIT_BUCKET_COUNT: Final = 10
BOUNDARY_V1_TRAIN_SPLIT_BUCKETS: Final = 7
BOUNDARY_V1_WORD_MIN_CHARACTERS: Final = 3
BOUNDARY_V1_WORD_MAX_CHARACTERS: Final = 24
BOUNDARY_V1_EPOCHS: Final = 35
BOUNDARY_V1_LEARNING_RATE: Final = 0.10
BOUNDARY_V1_THRESHOLDS: Final = (0.9, 0.95, 0.98, 0.99, 0.995, 0.999, 0.9995)
# Row importance balances the two label classes: the whole word and a literal suffix.
BOUNDARY_V1_LABEL_CLASSES: Final = 2
# Threshold of the models that only compute training and development probabilities;
# BoundaryModel.probabilities() never reads it.
BOUNDARY_V1_LOSS_MODEL_THRESHOLD: Final = 0.9
