"""Numbers used to build and freeze corpora."""

from __future__ import annotations

from typing import Final

# A Debian Contents index line is a path and its package list.
CONTENTS_INDEX_FIELDS_PER_LINE: Final = 2
# Commands retained per alias family, and the same figure reported in the summary.
COMMANDS_PER_ALIAS_FAMILY: Final = 2
# Lengths of the lowercase ASCII command names the technical corpora take from package indexes.
COMMAND_NAME_MIN_CHARACTERS: Final = 3
COMMAND_NAME_MAX_CHARACTERS: Final = 16
# Shortest word that gets internal-edit typo variants (delete, duplicate, transpose).
TYPO_SOURCE_MIN_CHARACTERS: Final = 4
# Longest word the internal-edit typo expansion covers (the old context-v1 edit-expansion bound).
TYPO_SOURCE_MAX_CHARACTERS: Final = 64
# Characters left at the word's tail by the transposition loop so a two-character swap stays in
# bounds.
TRANSPOSE_TAIL_MARGIN_CHARACTERS: Final = 2
HTTP_OK_STATUS: Final = 200
# Leading hex digits (64 bits) of a SHA-256 digest that the corpus freezers turn into a
# deterministic bucket, rank or sample draw.
DETERMINISTIC_DRAW_HEX_DIGITS: Final = 16
# typo_variants(): a delete or duplicate needs one character on each side.
TYPO_DELETE_DUPLICATE_MARGIN_CHARACTERS: Final = 2
# typo_variants(): a transposition needs the character and the next two in bounds.
TYPO_TRANSPOSE_MARGIN_CHARACTERS: Final = 3
# characters consumed by one transposed pair
TRANSPOSE_PAIR_CHARACTERS: Final = 2
# CoNLL-U column: ID, FORM, LEMMA, ...
CONLLU_LEMMA_COLUMN_INDEX: Final = 2
CONLLU_COLUMN_COUNT: Final = 10
# The frozen sentence window: how much surrounding text each row keeps.
CORPUS_BEFORE_WINDOW_CHARACTERS: Final = 96
CORPUS_AFTER_WINDOW_CHARACTERS: Final = 64
CORPUS_LITERAL_TAIL_MAX_CHARACTERS: Final = 64
# Hash buckets a split is drawn from: a digest modulo this, compared with the cumulative split
# ceilings.
SPLIT_BUCKET_COUNT: Final = 100
# Cumulative bucket ceilings of the base corpus splits: train, development, calibration; test takes
# the rest.
TRAIN_SPLIT_CEILING: Final = 70
DEVELOPMENT_SPLIT_CEILING: Final = 80
CALIBRATION_SPLIT_CEILING: Final = 90
# a "[family, text]" pair needs at least two elements
FAMILY_KEY_MIN_PARTS: Final = 2
# Fields of a prior context TSV line, and the one holding the text.
PRIOR_CONTEXT_TSV_FIELD_COUNT: Final = 4
PRIOR_CONTEXT_TSV_TEXT_FIELD_INDEX: Final = 2
# A document holding more than half of all documents' rows is a conflict.
MAJORITY_DIVISOR: Final = 2
# Documents sampled per source by default when a corpus or holdout is frozen.
DEFAULT_MAX_DOCUMENTS_PER_SOURCE: Final = 2000
# Sentences taken per sampled document by default when a context-action corpus, fitting set or
# holdout is frozen.
DEFAULT_MAX_SENTENCES_PER_DOCUMENT: Final = 12
# The base corpus uses 70/10/10/10 over four splits; the extension has no test share, so the same
# proportions renormalise to these cumulative buckets over a hundred.
FITTING_SHARES: Final = ((78, "train"), (89, "development"), (SPLIT_BUCKET_COUNT, "calibration"))
# Documents sampled per source by default when the fitting extension is frozen.
FITTING_DEFAULT_MAX_DOCUMENTS_PER_SOURCE: Final = 12000
TATOEBA_SAMPLE_PER_MILLE: Final = 40
TATOEBA_EXPORT_COLUMNS: Final = 3
MAX_TATOEBA_SENTENCE_CHARACTERS: Final = 400
# Bounds on a prefix-v1 parent row the prefix-v2 corpus reuses.
PREFIX_V2_PARENT_TEXT_MAX_CHARACTERS: Final = 32
PREFIX_V2_PARENT_FAMILY_MAX_CHARACTERS: Final = 3
PREFIX_V2_PARENT_BEFORE_MAX_CHARACTERS: Final = 160
# Longest application name a prefix-v2 parent or context may carry.
PREFIX_V2_APPLICATION_NAME_MAX_CHARACTERS: Final = 128
PREFIX_V2_DEFAULT_WORDS_PER_FAMILY: Final = 2
PREFIX_V2_MAXIMUM_FAMILIES_LIMIT: Final = 4096
PREFIX_V2_MAXIMUM_WORDS_PER_FAMILY: Final = 4
PREFIX_V2_PARENT_IDENTIFIER_LIMIT: Final = 32768
# Position of the fixed deletion typo the old prefix corpus already exposed.
PREFIX_V2_LEGACY_DELETION_INDEX: Final = 3
# Longest before-context a prefix-v2 context may carry.
PREFIX_V2_CONTEXT_BEFORE_MAX_CHARACTERS: Final = 512
PREFIX_V2_MAX_CONTEXTS_PER_PARENT: Final = 64
PREFIX_V2_INDEX_CACHE_SIZE: Final = 4
# A desired prefix shorter than this is labelled ambiguous-positive.
PREFIX_V2_SHORT_PREFIX_CHARACTERS: Final = 4
PREFIX_V2_AMBIGUOUS_POSITIVE_LABEL: Final = 2
PHYSICAL_ALIAS_CACHE_SIZE: Final = 131072
# Positional arguments of add()/unknown_family() calls in the frozen train_context_model.py:
# add(word, group, before, after, ...), unknown_family(name, group, contexts, ...).
ADD_CALL_MIN_ARGS: Final = 4
ADD_CALL_BEFORE_ARG_INDEX: Final = 2
ADD_CALL_AFTER_ARG_INDEX: Final = 3
UNKNOWN_FAMILY_CALL_MIN_ARGS: Final = 3
UNKNOWN_FAMILY_CALL_CONTEXTS_ARG_INDEX: Final = 2
# Split shares of the base context-action corpus, derived from its bucket ceilings for the report.
TRAIN_SPLIT_PROBABILITY: Final = TRAIN_SPLIT_CEILING / SPLIT_BUCKET_COUNT
DEVELOPMENT_SPLIT_PROBABILITY: Final = (DEVELOPMENT_SPLIT_CEILING - TRAIN_SPLIT_CEILING) / SPLIT_BUCKET_COUNT
CALIBRATION_SPLIT_PROBABILITY: Final = (CALIBRATION_SPLIT_CEILING - DEVELOPMENT_SPLIT_CEILING) / SPLIT_BUCKET_COUNT
TEST_SPLIT_PROBABILITY: Final = (SPLIT_BUCKET_COUNT - CALIBRATION_SPLIT_CEILING) / SPLIT_BUCKET_COUNT
