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
# The context-v2 phrase split (tools/context_corpus.py), frozen with the rejected context-v2
# generation and reused by corpora built from its reserve. A row of the CC0 phrase snapshot is
# identifier, language, text and modification time.
CC0_PHRASE_TSV_FIELDS: Final = 4
# Phrases kept per near-duplicate group by default, so a large template family cannot dominate.
CONTEXT_PHRASES_PER_GROUP: Final = 4
# A phrase enters the split with this many characters and at least this many words.
CONTEXT_PHRASE_MIN_CHARACTERS: Final = 4
CONTEXT_PHRASE_MAX_CHARACTERS: Final = 512
CONTEXT_PHRASE_MIN_WORDS: Final = 2
# A phrase of this many words also groups by each of its one-word deletions.
CONTEXT_NEAR_DUPLICATE_MIN_WORDS: Final = 4
CONTEXT_NEAR_DUPLICATE_MAX_WORDS: Final = 40
# A phrase group's hash picks one of the buckets; cumulative ceilings of train, development,
# calibration and test, and the rest is the reserve.
CONTEXT_PHRASE_SPLIT_BUCKET_COUNT: Final = 100
CONTEXT_PHRASE_TRAIN_SPLIT_CEILING: Final = 60
CONTEXT_PHRASE_DEVELOPMENT_SPLIT_CEILING: Final = 70
CONTEXT_PHRASE_CALIBRATION_SPLIT_CEILING: Final = 80
CONTEXT_PHRASE_TEST_SPLIT_CEILING: Final = 90
# Context-v2 phrase frames (tools/context_frames.py). A focus word has at most this many letters;
# the first word of a phrase is always a focus when it is short, and at most this many positions
# of a phrase become frames.
CONTEXT_FRAME_WORD_MAX_CHARACTERS: Final = 32
CONTEXT_FRAME_SHORT_WORD_MAX_CHARACTERS: Final = 2
CONTEXT_FRAME_FOCUS_POSITIONS_PER_PHRASE: Final = 2
# Left and right context a frame keeps around its focus word.
CONTEXT_FRAME_BEFORE_MAX_CHARACTERS: Final = 512
CONTEXT_FRAME_AFTER_MAX_CHARACTERS: Final = 128
# One frame in this many has its left context typed in the other layout; one in this many keeps
# its right context and the role of a text field.
CONTEXT_FRAME_WRONG_LAYOUT_BEFORE_MODULUS: Final = 5
CONTEXT_FRAME_TEXT_FIELD_MODULUS: Final = 7
# One frame in this many whose focus word has at least this many letters also gets a spelling
# variant: one inner letter deleted, never one of the edge letters (the first and the last).
CONTEXT_FRAME_SPELLING_VARIANT_MODULUS: Final = 5
CONTEXT_FRAME_SPELLING_VARIANT_MIN_CHARACTERS: Final = 4
CONTEXT_FRAME_SPELLING_EDGE_CHARACTERS: Final = 2
# One focus family in this many is held out of every fitting split for the lexical test track.
CONTEXT_FRAME_LEXICAL_HOLDOUT_MODULUS: Final = 10
# Authored technical tokens: a family's hash picks one of the buckets; the first ones train, then
# one development and one calibration bucket, the rest test.
CONTEXT_FRAME_SAFETY_SPLIT_BUCKET_COUNT: Final = 10
CONTEXT_FRAME_SAFETY_TRAIN_BUCKETS: Final = 6
CONTEXT_FRAME_SAFETY_DEVELOPMENT_BUCKET: Final = 6
CONTEXT_FRAME_SAFETY_CALIBRATION_BUCKET: Final = 7
# Corpus revision the context-v2 seal records.
CONTEXT_V2_CORPUS_REVISION: Final = 2
# The key-space corpus of the rejected ortho-v2 generation (tools/ortho_v2_corpus.py): the percent
# of key-sequence families reserved for the one-shot gate, the longest token taken, the cap on the
# weight of a lexicon row, and how many letters make an all-capitals token an abbreviation.
ORTHO_V2_GATE_SHARE_PERCENT: Final = 16
ORTHO_V2_MAXIMUM_TOKEN_CHARACTERS: Final = 32
ORTHO_V2_MAXIMUM_LEXICON_WEIGHT: Final = 32
ORTHO_V2_ABBREVIATION_MIN_LETTERS: Final = 2
# Shortest lexicon word the ortho-v2 corpus counts.
ORTHO_V2_LEXICON_MIN_CHARACTERS: Final = 2
# tools/ortho_v2_verified.py: a sentence vouches for its tokens' labels when it has at least this
# many word-shaped tokens and at least this share of them are known to its language.
ORTHO_V2_VERIFIED_MIN_TOKENS: Final = 5
ORTHO_V2_VERIFIED_MIN_KNOWN_SHARE: Final = 0.75
# Dictionary verdicts that tool caches while it reads the snapshot.
ORTHO_V2_DICTIONARY_CACHE_SIZE: Final = 1 << 20
# tools/mixed_typing.py - public text typed the way a person switches layouts, through the engine.
# Messages from ru.stackoverflow: whitespace tokens per message, and the split of question threads by
# the first hex digits of a SHA-256 over a hundred buckets (the first ten test, the next ten
# development, the rest train).
MIXED_MESSAGE_MIN_TOKENS: Final = 3
MIXED_MESSAGE_MAX_TOKENS: Final = 24
MIXED_SPLIT_HEX_DIGITS: Final = 8
MIXED_TEST_SPLIT_BUCKETS: Final = 10
MIXED_DEVELOPMENT_SPLIT_BUCKETS: Final = 20
# Tatoeba sentences taken: whitespace tokens per sentence.
TATOEBA_SENTENCE_MIN_TOKENS: Final = 2
TATOEBA_SENTENCE_MAX_TOKENS: Final = 12
# Single lines of code from the posts, the text of a previous line in the field: their length, and
# how many of the first ones a run draws from.
MIXED_CODE_LINE_MIN_CHARACTERS: Final = 10
MIXED_CODE_LINE_MAX_CHARACTERS: Final = 100
MIXED_PREFIX_CODE_LINES: Final = 200000
# Deterministic draws: leading hex digits of a SHA-256, and the resolution of a share compared with
# a draw.
MIXED_DRAW_HEX_DIGITS: Final = 8
MIXED_DRAW_RESOLUTION: Final = 10000
# A typo: leading hex digits of its draw, its four kinds (a letter missed, two swapped, one pressed
# twice, the key next to it), the bits that pick the kind and the place, and the fewest letters a
# token needs for one (a letter is only missed from a longer token).
MIXED_TYPO_HEX_DIGITS: Final = 12
MIXED_TYPO_PLACE_SHIFT: Final = 4
MIXED_TYPO_NEIGHBOUR_SHIFT: Final = 16
MIXED_TYPO_MIN_LETTERS: Final = 3
# A mid-word edit, as the insert package makes it: a word of at least four letters loses one letter,
# or two if it has five or more, at a place drawn from the given bits; the letters are typed back one
# key at a time, then the pause that corrects them.
MIXED_EDIT_WORD_MIN_LETTERS: Final = 4
MIXED_EDIT_TWO_LETTERS_FROM: Final = 5
MIXED_EDIT_MAX_REMOVED_LETTERS: Final = 2
MIXED_EDIT_LENGTH_SHIFT: Final = 8
MIXED_EDIT_PLACE_SHIFT: Final = 12
MIXED_EDIT_CLOCK_START_SECONDS: Final = 1000.0
MIXED_EDIT_KEY_INTERVAL_SECONDS: Final = 0.08
MIXED_EDIT_PAUSE_SECONDS: Final = 2.0
# Worker processes and the jobs each takes at a time.
MIXED_TYPING_DEFAULT_WORKERS: Final = 10
MIXED_TYPING_CHUNK_JOBS: Final = 8
# Russian-only messages typed per mixed one when `mixed_typing.py evaluate` samples a set, and the xz
# preset of the captured question files.
MIXED_EVALUATE_RUSSIAN_SHARE: Final = 0.25
MIXED_CAPTURE_XZ_PRESET: Final = 9
# The text column of a Tatoeba sentence export (id, language, text), and the two kinds of previous
# line a typed message is put under: a line of code or another message.
TATOEBA_TEXT_COLUMN: Final = 2
# Column of the author's user name in a Tatoeba `*_sentences_detailed.tsv` export.
TATOEBA_AUTHOR_COLUMN: Final = 3
MIXED_PREFIX_KINDS: Final = 2
