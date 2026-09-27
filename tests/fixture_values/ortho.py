"""Values only the orthotactic population and trainer tests use."""

from __future__ import annotations

from typing import Final

# A tiny training configuration: order, discount, scale and the shortest scored token, with fixed
# thresholds in nats and the promotion gate the configuration file must carry.
POPULATION_FIXTURE_ORDER: Final = 3
POPULATION_FIXTURE_DISCOUNT: Final = 0.75
POPULATION_FIXTURE_SCALE: Final = 512
POPULATION_FIXTURE_MINIMUM_LENGTH: Final = 3
POPULATION_FIXTURE_THRESHOLD_EN_NATS: Final = 26.0
POPULATION_FIXTURE_THRESHOLD_RU_NATS: Final = 13.3
POPULATION_FIXTURE_MINIMUM_TEST_NEGATIVES: Final = 5000
POPULATION_FIXTURE_MINIMUM_RECALL: Final = 0.75
# Weight of one word-list row in the counting fixture, and occurrences of one corpus token.
POPULATION_FIXTURE_LEXICON_WEIGHT: Final = 7
POPULATION_FIXTURE_TOKEN_OCCURRENCES: Final = 2
# Share of phrase groups the split sends to test, as a range the draw of many groups falls into.
POPULATION_FIXTURE_TEST_SHARE_MIN: Final = 0.5
POPULATION_FIXTURE_TEST_SHARE_MAX: Final = 0.6
POPULATION_FIXTURE_GROUPS: Final = 4000
# The feature-model fixture: weights in scale units, and a threshold low enough that only the
# features decide.
FEATURE_FIXTURE_IDENT_WEIGHT: Final = 1024
FEATURE_FIXTURE_REPEAT_WEIGHT: Final = -2048
FEATURE_FIXTURE_PARTS_WEIGHT: Final = 1536
FEATURE_FIXTURE_DOT_WEIGHT: Final = 3072
FEATURE_FIXTURE_UNSUPPORTED_SCHEMA: Final = 5
FEATURE_FIXTURE_NON_INTEGER_WEIGHT: Final = 0.5
# Feature folds and the L2 penalty of the population fixture's configuration.
POPULATION_FIXTURE_FOLDS: Final = 5
POPULATION_FIXTURE_L2: Final = 0.1
# The run a stretched letter is cut to in the feature fixture.
FEATURE_FIXTURE_COLLAPSE: Final = 2
# The run a schema 4 fixture reads once as a stretch.
FEATURE_FIXTURE_STRETCH: Final = 3
# The length the longer half of an English compound must reach in the schema 5 fixture.
FEATURE_FIXTURE_COMPOUND_MIN_LETTERS: Final = 3
# The web lexicon fixture: ranks near the top of a vocabulary, far down it and beyond any real one; and
# the weights a word-list row and a web form of the same key carry when they are merged.
WEB_FIXTURE_NEAR_RANK: Final = 10
WEB_FIXTURE_FAR_RANK: Final = 100_000
WEB_FIXTURE_BEYOND_RANK: Final = 10 ** 12
WEB_FIXTURE_LIST_WEIGHT: Final = 3
WEB_FIXTURE_WEB_WEIGHT: Final = 5
# The support below which a feature of the tie fixture is not fitted on its own.
FEATURE_FIXTURE_MIN_TYPES: Final = 2
# The share of words that get a synthetic stray-key variant, as a range many generated words fall into.
SYNTHETIC_FIXTURE_SHARE_MIN: Final = 0.07
SYNTHETIC_FIXTURE_SHARE_MAX: Final = 0.13
# The share of fixture sentences an independent set keeps by a hash of the sentence, the number of
# sentences drawn, and the bounds the kept count falls in.
INDEPENDENT_FIXTURE_SAMPLE_PERCENT: Final = 50
INDEPENDENT_FIXTURE_SAMPLE_SENTENCES: Final = 1000
INDEPENDENT_FIXTURE_SAMPLE_MIN: Final = 400
INDEPENDENT_FIXTURE_SAMPLE_MAX: Final = 600
# A fixture column that fires on every this-many-th positive only, so it separates the labels.
FIRTH_FIXTURE_MARK_EVERY: Final = 7
# The fixture labels alternate between the two classes.
FIRTH_FIXTURE_CLASSES: Final = 2
