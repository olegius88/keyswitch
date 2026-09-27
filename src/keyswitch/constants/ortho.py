"""The orthotactic model: limits of its artifact, and the ortho-v1 tools that count, seal and test it."""

from __future__ import annotations

from typing import Final

from .units import BYTES_PER_MEBIBYTE

# What keyswitch.ortho_model accepts from an artifact. The module exports the first four under
# the names it has always had (MAX_ARTIFACT_BYTES, MAX_GRAMS, MIN_ORDER, MAX_ORDER).
# Largest artifact file read.
ORTHO_ARTIFACT_MAX_BYTES: Final = 24 * BYTES_PER_MEBIBYTE
# Most n-grams one language channel may carry.
ORTHO_CHANNEL_MAX_GRAMS: Final = 4_000_000
# Orders of the character model an artifact may declare.
ORTHO_MIN_ORDER: Final = 2
ORTHO_MAX_ORDER: Final = 8
# Largest quantisation scale an artifact may declare: integer steps per nat of every stored weight
# and threshold.
ORTHO_MAX_SCALE: Final = 4096
# Largest shortest-scored-token length an artifact may declare.
ORTHO_MAX_MINIMUM_LENGTH_CHARACTERS: Final = 32
# A stored backoff weight is a [context, weight] pair.
ORTHO_BACKOFF_ENTRY_FIELDS: Final = 2

# The ortho-v1 key-space corpus (tools/ortho_corpus.py): the longest whitespace token it keeps.
ORTHO_V1_MAXIMUM_TOKEN_CHARACTERS: Final = 32

# The ortho-v1 trainer (tools/train_ortho_model.py): the additive pseudo-count of every unigram and
# of every case shape, so an unseen character or shape is unlikely rather than impossible.
ORTHO_V1_UNIGRAM_PSEUDO_COUNT: Final = 0.5
ORTHO_V1_SHAPE_PSEUDO_COUNT: Final = 0.5
# Decimal places of the thresholds written into the seal and of the recall written into the report.
ORTHO_V1_SEALED_THRESHOLD_DECIMALS: Final = 6
ORTHO_V1_RECALL_DECIMALS: Final = 6
# The report quotes this many worst false conversions per direction, their scores rounded to this
# many decimal places.
ORTHO_V1_REPORTED_WORST_FALSE: Final = 10
ORTHO_V1_REPORTED_SCORE_DECIMALS: Final = 4
# Characters of the test report the trainer prints.
ORTHO_V1_PRINTED_REPORT_MAX_CHARACTERS: Final = 4000

# The ortho-v1 split (tools/ortho_corpus.py, generation 3): a phrase group whose bucket, one of
# CONTEXT_PHRASE_SPLIT_BUCKET_COUNT, lies below this ceiling is test; every other group is train.
# The groups of a salt the final candidate does not train on (buckets below this). The single test is
# an independent set (tools/ortho_independent.py), so this part is only left out: its size makes the
# final train on the same share of groups as a development draw - 45 %, nine elevenths of the 55 % a
# development salt leaves visible - so the spread of the draws' weights describes the final's.
ORTHO_V1_TEST_SPLIT_CEILING: Final = 55
# The frozen Onboard word lists enter the character counts only: a word of at least this many
# characters, weighted by the bit length of its frequency, capped at this weight.
ORTHO_V1_LEXICON_MIN_CHARACTERS: Final = 2
ORTHO_V1_MAXIMUM_LEXICON_WEIGHT: Final = 32
# Schema versions: the configuration (3: fixed thresholds and the feature fit; 4: the web vocabulary of
# the source channel; 5: the minimum support of a feature fitted on its own; 6: the stretch readings of
# a schema 4 artifact, `stretch_runs`; 7: the feature fit's prior and shared weights, `feature_prior`,
# `feature_shared`; 8: features always tied to their parent, `feature_tied`; 9: the schema 5 artifact,
# `hyphen_stretch`, and features left unfitted, `feature_omitted`; 10: columns fitted with a log-F(1,1)
# prior, `feature_separated`, and the length a compound's half must reach, `compound_min_letters`;
# 11: the schema 6 artifact, `target_plausibility`, and features counted below the plausibility centre only,
# `feature_gated`), and the corpus receipt, seal and report that describe the population the engine serves the model.
ORTHO_V1_CONFIG_SCHEMA_VERSION: Final = 11
ORTHO_V1_EVIDENCE_SCHEMA_VERSION: Final = 2

# The feature model (ortho-v1 artifact schema 2): the artifact schema that carries feature weights,
# and the number of parts a hyphenated token splits into for the compound features.
ORTHO_FEATURE_ARTIFACT_SCHEMA_VERSION: Final = 2
ORTHO_COMPOUND_PARTS: Final = 2
# Artifact schema 3: schema 2 and the weight of `extra_key` (a target reading that becomes a word
# without one of its letters).
ORTHO_EXTRA_KEY_ARTIFACT_SCHEMA_VERSION: Final = 3
# The share, in percent of the eligible train word types, that gets a synthetic variant with one stray
# key for the feature fit (tools/ortho_corpus.py `extra_key_variant`): a feature weight needs tens of
# events and this gives thousands, while the synthetic examples stay far fewer than the real positives
# that fit the weight of the character score every feature weight is relative to.
ORTHO_SYNTHETIC_EXTRA_KEY_SHARE_PERCENT: Final = 10
# The receipt of an independent test built from text the development never saw
# (tools/ortho_independent.py); the shares of the English Wikipedia sentences of Common Voice and of the
# sentences of the Wikisource stories its active set keeps (by a hash of the sentence), chosen from the
# type counts before any model saw them, so that each language gives about as many negative types as the
# larger one gave in the first set; and the share of the same English file an earlier set took, left out.
ORTHO_INDEPENDENT_RECEIPT_SCHEMA_VERSION: Final = 2
ORTHO_INDEPENDENT_ENGLISH_SAMPLE_PERCENT: Final = 7
ORTHO_INDEPENDENT_WIKISOURCE_SAMPLE_PERCENT: Final = 30
ORTHO_INDEPENDENT_TAKEN_ENGLISH_PERCENT: Final = 5
# The trainer's logistic regression over [char score, features] (tools/train_ortho_model.py): Newton
# steps at most, the step size below which it stops, and the bound on a log-odds before exp().
ORTHO_V1_LOGISTIC_ITERATIONS: Final = 25
ORTHO_V1_LOGISTIC_TOLERANCE: Final = 1e-7
ORTHO_V1_LOGISTIC_LOGIT_BOUND: Final = 30.0
# Firth's modified score adds, per sample, its leverage times (probability - this value).
ORTHO_V1_FIRTH_SCORE_CENTRE: Final = 0.5
# The shortest run of one key an artifact may collapse a longer run to (`collapse_runs`): stretched
# spellings (`каеффф`, `нееет`) are judged by the word without the stretch.
ORTHO_MIN_COLLAPSED_RUN: Final = 2
# An artifact of schema 4 is schema 3 with `stretch_runs`: a run of at least that many identical keys may
# be a stretch or a stuck key, and the model also reads it once (`ortho_model.read_once`). The shortest
# such run is two keys: one key is no run.
ORTHO_STRETCH_ARTIFACT_SCHEMA_VERSION: Final = 4
ORTHO_MIN_STRETCH_RUN: Final = 2
# An artifact of schema 5 is schema 4 that also reads a hyphen between two copies of one key as a
# drawn-out sound (`Ти-ише`) and weighs whether the replacement is a word (`target_unknown`,
# `name_unknown`).
ORTHO_REPLACEMENT_ARTIFACT_SCHEMA_VERSION: Final = 5
# An artifact of schema 6 is schema 5 that also weighs how word-like the replacement is in its own language
# (`target_plausibility`, from the centres it carries) and may count a feature only for a token less
# plausible than the centre of the layout typed (`gated_features`).
ORTHO_PLAUSIBILITY_ARTIFACT_SCHEMA_VERSION: Final = 6
# The web word forms (tools/ortho_web_lexicon.py) that the source channel of Russian counts: the schema
# of the frozen derived file; the fewest letters a form has, because shorter forms are fragments that
# make short English keys read as Russian (`гги` for `uub`); and the count per billion words of the most
# frequent word, from which the count of a rank follows by Zipf's law.
ORTHO_WEB_LEXICON_SCHEMA_VERSION: Final = 1
ORTHO_WEB_LEXICON_MIN_CHARACTERS: Final = 4
ORTHO_WEB_LEXICON_TOP_PER_BILLION: Final = 50_000_000
# The public-domain prose forms of the Russian source channel (tools/ortho_prose_lexicon.py): the format
# of the derived file; the last year of death of an author taken (public domain under a term of life plus a
# century); how deep the subcategories of an author's prose category are followed; the words a form's count
# is scaled to (its weight is the bit length of the count per this many words, as a web form's per billion);
# and the MediaWiki namespaces of the dump: articles, categories and Wikisource's author pages.
ORTHO_PROSE_LEXICON_SCHEMA_VERSION: Final = 1
ORTHO_PROSE_LAST_DEATH_YEAR: Final = 1925
ORTHO_PROSE_CATEGORY_DEPTH: Final = 3
ORTHO_PROSE_WORDS_SCALE: Final = 1_000_000_000
MEDIAWIKI_ARTICLE_NAMESPACE: Final = 0
MEDIAWIKI_CATEGORY_NAMESPACE: Final = 14
WIKISOURCE_AUTHOR_NAMESPACE: Final = 102
