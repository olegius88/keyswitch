"""Fixture rows and tables used by tests."""

from __future__ import annotations

from typing import Final

from keyswitch.short_words import TRUSTED_SHORT_WORD_MINIMUM_FREQUENCY

# (name, source group, physical keys, expected text, expected group, delay before verifying, in ms)
# for each scripted native-package typing scenario.
NATIVE_PACKAGE_TYPING_CASES: Final = (
    ("EN pause correction", 0, "ghbdtn", "привет", 1, 2300),
    ("RU keys to English", 1, "hello ", "hello ", 0, 1000),
    ("punctuation key is a Russian letter", 0, ",fpf ", "база ", 1, 1000),
    ("return to EN before punctuation test", 1, "hello ", "hello ", 0, 1000),
    # Ambiguous punctuation now waits for the default 1.5-second idle boundary.
    ("punctuation boundary keeps its glyph", 0, "ghbdtn,", "привет,", 1, 2300),
    ("manual layout switch protects next word", 0, "ghbdtn ", "ghbdtn ", 0, 1000),
    ("manual protection is consumed once", 0, "ghbdtn ", "привет ", 1, 1000),
    ("short Russian word switches to English", 1, "if ", "if ", 0, 1000),
    ("manual Russian selection protects short word", 1, "if ", "ша ", 1, 1000),
    ("short-word protection is consumed once", 1, "if ", "if ", 0, 1000),
    ("context resolves a short word with the next word", 0, "e 'njuj ", "у этого ", 1, 1200),
)
# "wait" is the third row of "I can't  wait!" (I, can't, wait, !)
WAIT_TOKEN_ROW_INDEX: Final = 2
# s3, the sentence carrying "# newdoc_id = named"
NAMED_DOCUMENT_SENTENCE_INDEX: Final = 2
# s4, the sentence after a bare "# newdoc"
ANONYMOUS_DOCUMENT_SENTENCE_INDEX: Final = 3
# Fixture values the action-feature tests feed to extract_action_features() and ContextModel.
ACTION_FEATURES_TARGET_KNOWN_FREQUENCY: Final = 100
# Ordered word frequencies of the authored sequence-test lexicons.
SEQUENCE_LEXICON_HIGH_FREQUENCY: Final = 5000
SEQUENCE_LEXICON_MID_FREQUENCY: Final = 4000
SEQUENCE_LEXICON_LOW_FREQUENCY: Final = 3000
# Frequency every word of a small authored lexicon gets when only its presence matters.
FIXTURE_WORD_FREQUENCY: Final = 5000
PLANNED_EVIDENCE_DOMINANT_WORD_FREQUENCY: Final = 20000
CONTEXT_FRAMES_SECOND_PHRASE_ID: Final = 2
HELD_KEY_CONTEXT_REPLAY_CASES: Final = (
        (0, 0, False, "контекст привет"),
        (1, 1, False, "контекст привет"),
        (2, 2, False, "контекст привет"),
        (1, 0, False, ""),
        (1, 1, True, ""),
    )
HISTORICAL_CONTEXT_V1_TEST_COUNTS: Final = {
    "rows": 10000, "desired_conversions": 5000,
    "converted_correctly": 4900, "baseline_converted_correctly": 4800, "false_conversions": 0,
}
# Index of the "test" entry within the (train, development, calibration, test, reserve) splits tuple
# built below; the only split select_phrases both reads and can pick.
CONTEXT_V2_TEST_SPLIT_INDEX: Final = 3
CONTEXT_V2_KERNEL_TRAINING_ROWS: Final = [
    ({"a": 1.0, "b": -0.5}, 1, 1.4),
    ({"c": 4.0, "b": 0.9}, 0, 0.5),
    ({"a": 0.8}, 3, 2.0),
]
CONTEXT_V2_BASE_PROMOTION_COUNTS: Final = {
    "rows": 1000,
    "desired_conversions": 500,
    "converted_correctly": 490,
    "false_conversions": 0,
    "baseline_false_conversions": 0,
}
SHORT_SOURCE_VETO_TARGET_WORD_FREQUENCY: Final = 54906
DETECTOR_FIXTURE_EXACT_WORD_FREQUENCY: Final = 10
DETECTOR_CURATED_SOURCE_FREQUENCY: Final = 531
DETECTOR_TRUSTED_TARGET_FREQUENCY: Final = 464_324
DETECTOR_VERY_HIGH_FREQUENCY: Final = 1_000_000
# One below the curated frequency floor; deliberately below the frequency floor a candidate must
# clear before the ratio gate is even considered.
BELOW_TRUSTED_SHORT_WORD_MINIMUM_FREQUENCY: Final = TRUSTED_SHORT_WORD_MINIMUM_FREQUENCY - 1
# Tuned so (frequency + 1) / (DETECTOR_SOURCE_FREQUENCY_BELOW_RATIO + 1) falls just under
# TRUSTED_SHORT_WORD_MINIMUM_RATIO when the target sits at the minimum.
DETECTOR_SOURCE_FREQUENCY_BELOW_RATIO: Final = 100
EARLY_SWITCH_EN_WORD_FREQUENCIES: Final = {"hello": 500_000, "help": 200_000, "held": 50_000, "xfce": 10}
EARLY_SWITCH_RU_WORD_FREQUENCIES: Final = {
    "привет": 900_000,
    "приветствие": 5_000,
    "привал": 4_000,
    "приз": 3_000,
    "почему": 800_000,
    "почесать": 1_500,
    "тебя": 600_000,
}
EARLY_SWITCH_POLICY_LOW_MINIMUM_FREQUENCY: Final = 1_000
EARLY_SWITCH_POLICY_UNREACHABLE_DOMINANT_FREQUENCY: Final = 10**9
# Frequency fed to the fixture LanguageModel entries below.
IDENTIFIER_LEXICON_WORD_FREQUENCY: Final = 20
INTENT_EVIDENCE_TARGET_FREQUENCY: Final = 100
INTENT_IMPLAUSIBLE_FREQUENCY_MAGNITUDE: Final = 10
INTENT_HUGE_FREQUENCY: Final = 2_000_000_000
INTENT_SCORER_ONLY_SOURCE_FREQUENCY: Final = 1_000_000_000
INTENT_SCORER_ONLY_TARGET_FREQUENCY: Final = 999_999_999
INTENT_EVALUATION_LM_KNOWN_WORD_FREQUENCY: Final = 100
INTENT_DATASET_HIGH_FREQUENCY: Final = 100
INTENT_DATASET_UPPER_MID_FREQUENCY: Final = 90
INTENT_DATASET_MID_FREQUENCY: Final = 80
INTENT_DATASET_LOWER_MID_FREQUENCY: Final = 70
INTENT_DATASET_LOW_FREQUENCY: Final = 10
# Indentation the frozen hard-negative corpus writer uses; the tampered copy is re-serialised the
# same way.
HARD_NEGATIVE_CORPUS_JSON_INDENT: Final = 2
INTENT_DATASET_EXPECTED_HELLO_FREQUENCY: Final = 15
INTENT_DATASET_EXPECTED_KEY_FREQUENCY: Final = 7
INTENT_LEAKAGE_POISONED_FREQUENCY: Final = 10**9
INTENT_LEAKAGE_EXCLUDED_ENTRY_FREQUENCY: Final = 100
INTENT_LEAKAGE_INCLUDED_ENTRY_FREQUENCY: Final = 80
INTENT_LEAKAGE_NON_TRAIN_ENTRY_FREQUENCY: Final = 50
INTENT_LEAKAGE_MUTATED_NON_TRAIN_EN_FREQUENCY: Final = 5_000
INTENT_LEAKAGE_MUTATED_NON_TRAIN_RU_FREQUENCY: Final = 7_000
INTENT_LEAKAGE_TRAIN_RECORD_COUNT: Final = 3
INTENT_LEAKAGE_NON_TRAIN_EN_RECORD_INDEX: Final = 3
INTENT_LEAKAGE_NON_TRAIN_RU_RECORD_INDEX: Final = 4
PREFIX_LEXICON_HIGH_FREQUENCY: Final = 100
PREFIX_LEXICON_LOW_FREQUENCY: Final = 50
# Fixture word frequencies: a common word outweighs a rarer one.
PREFIX_V2_COMMON_WORD_FREQUENCY: Final = 5000
PREFIX_V2_RARE_WORD_FREQUENCY: Final = 2000
DEEP_MERGE_BASE_CONFIG: Final[dict[str, object]] = {"a": {"b": 1, "c": 2}, "d": 3}
DEEP_MERGE_OVERRIDE_CONFIG: Final[dict[str, object]] = {"a": {"b": 4}, "d": {"e": 5}}
DEEP_MERGE_EXPECTED_CONFIG: Final = {"a": {"b": 4, "c": 2}, "d": {"e": 5}}
SETTINGS_CHILD_LIST: Final = [1, 2]
SETTINGS_APPENDED_CHILD_VALUE: Final = 3
LANGUAGE_MODEL_HIGH_FREQUENCY: Final = 100
LANGUAGE_MODEL_LOW_FREQUENCY: Final = 20
LANGUAGE_MODEL_LOWEST_FREQUENCY: Final = 10
LANGUAGE_MODEL_HELPER_WORLD_FREQUENCY: Final = 50
LANGUAGE_MODEL_EXPECTED_HELLO_UNIGRAM_COUNT: Final = 15
LANGUAGE_MODEL_EXPECTED_HELLO_WORLD_BIGRAM_COUNT: Final = 7
LANGUAGE_MODEL_UNIFORM_WORD_FREQUENCY: Final = 7
LANGUAGE_MODEL_OVERFLOW_TRIGRAM_FREQUENCY: Final = 2**40
LANGUAGE_MODEL_BUILT_GRAM_FREQUENCY: Final = 10
LANGUAGE_MODEL_SINGLE_WORD_FREQUENCY: Final = 2
UI_FAKE_WORD_FREQUENCY: Final = 10
WORD_DECISION_WORD_FREQUENCY: Final = 100_000
WORD_DECISION_HIGHER_WORD_FREQUENCY: Final = 200_000
