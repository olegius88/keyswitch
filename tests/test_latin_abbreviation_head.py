"""The Latin-context abbreviation head (context_action_features.latin_abbreviation_question).

The owner's typing of 08.10.2026: after `/designv2 ` the frozen model turned `ТЗ` into `NP`. The head reads how often
each reading occurs instead of a copy of the shared features.
"""

from __future__ import annotations

import unittest

from keyswitch.constants.models import LATIN_ABBREVIATION_FEATURE_PREFIX
from keyswitch.context_action_features import (
    abbreviation_question, capitals_question, extract_action_features, latin_abbreviation_question,
)
from keyswitch.context_model import ContextEvidence
from keyswitch.input_context import FieldContext


def evidence(original: str, alternative: str, before: str) -> ContextEvidence:
    return ContextEvidence(original, alternative, 1, FieldContext("Telegram", "test", before, "", "unknown"), "space",
                           boundary_text=" ")


def head(prefix: str, item: ContextEvidence) -> set[str]:
    return {name.removeprefix(prefix).removesuffix(":direction:1") for name in extract_action_features(item)
            if name.startswith(prefix)}


class LatinAbbreviationHeadTests(unittest.TestCase):
    def test_the_head_answers_an_attested_abbreviation_after_latin_text(self) -> None:
        self.assertTrue(latin_abbreviation_question("ТЗ", "NP", "/designv2 "))
        self.assertTrue(latin_abbreviation_question("ГЫ", "US", "I moved to the "))
        # After Russian prose the abbreviation head answers; a token Russian text does not use, a lexicon word in
        # capitals and keys that are no letters are the frozen model's.
        self.assertFalse(latin_abbreviation_question("ТЗ", "NP", "пришли "))
        self.assertTrue(abbreviation_question("ТЗ", "NP", "пришли "))
        self.assertFalse(latin_abbreviation_question("ЙКЕ", "QRT", "/designv2 "))
        self.assertFalse(latin_abbreviation_question("ЛУНЫ", "KEYS", "/designv2 "))
        self.assertFalse(latin_abbreviation_question("БД", ",L", "/designv2 "))
        # The capitals head answers the same shape with its copy of the shared features, beside this head.
        self.assertTrue(capitals_question("ТЗ", "/designv2 "))

    def test_the_head_reads_both_counts_and_the_word_before(self) -> None:
        names = head(LATIN_ABBREVIATION_FEATURE_PREFIX, evidence("ТЗ", "NP", "/designv2 "))
        # `тз` is counted 744 times among the Cyrillic words of Russian technical text (bit length 10), `np` 80 times
        # among its Latin words (bit length 7); `designv2` occurs in neither English prose nor that text.
        self.assertTrue({"bias", "count:cyrillic:10", "count:latin:7", "count:ratio:-3", "letters:2", "trigger:space",
                         "previous_word:english:0", "previous_word:latin:0"} <= names)
        self.assertFalse([name for name in names if ":char:" in name])
        # `the` is English prose's commonest word.
        prose = head(LATIN_ABBREVIATION_FEATURE_PREFIX, evidence("ГЫ", "US", "I moved to the "))
        self.assertTrue([name for name in prose if name.startswith("previous_word:english:")])
        self.assertNotIn("previous_word:english:0", prose)
        self.assertEqual(head(LATIN_ABBREVIATION_FEATURE_PREFIX, evidence("ТЗ", "NP", "пришли ")), set())


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    suite = unittest.TestSuite()
    for case in (LatinAbbreviationHeadTests,):
        for name in case.__dict__:
            if name.startswith("test_"):
                suite.addTest(case(name))
    return suite


if __name__ == "__main__":
    unittest.main()
