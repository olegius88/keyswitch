"""The abbreviation head's class and evidence (context_action_features.abbreviation_question).

After Russian prose the frozen model turned abbreviations Russian text uses into the Latin readings
their keys spell (`страны ЕС приняли` became `страны TC приняли`, 0.37-0.42). The head answers an
attested Cyrillic abbreviation there with evidence of its own: how often each reading occurs.
"""

from __future__ import annotations

import unittest

from keyswitch.constants.models import ABBREVIATION_FEATURE_PREFIX
from keyswitch.context_action_features import abbreviation_question, attested_abbreviation, extract_action_features
from keyswitch.context_model import ContextEvidence
from keyswitch.input_context import FieldContext
from test_context_policy import ContextEngineTests


def evidence(original: str, alternative: str, before: str) -> ContextEvidence:
    return ContextEvidence(original, alternative, 1, FieldContext("Telegram", "test", before, "", "unknown"), "space",
                           boundary_text=" ")


def head(original: str, alternative: str, before: str) -> dict[str, float]:
    return {name: value for name, value in extract_action_features(evidence(original, alternative, before)).items()
            if name.startswith(ABBREVIATION_FEATURE_PREFIX)}


class AbbreviationHeadTests(unittest.TestCase):
    def test_an_attested_abbreviation_is_one_russian_technical_text_counts_outside_the_lexicon(self) -> None:
        self.assertTrue(attested_abbreviation("ЕС") and attested_abbreviation("ТЗ") and attested_abbreviation("ЗЫ"))
        # A word of the lexicon in capitals (`ЛУНЫ`), a Latin token, mixed case, one letter and six letters are not.
        for token in ("ЛУНЫ", "TC", "Ес", "Е", "ЕСЕСЕС", "Е1"):
            with self.subTest(token=token):
                self.assertFalse(attested_abbreviation(token))

    def test_the_head_answers_after_russian_prose_when_the_latin_reading_is_letters(self) -> None:
        self.assertTrue(abbreviation_question("ЕС", "TC", "страны "))
        self.assertFalse(abbreviation_question("ЕС", "TC", "the countries "))
        self.assertFalse(abbreviation_question("БД", ",L", "запрос к "))
        self.assertFalse(abbreviation_question("ЛУНЫ", "KEYS", "свет "))

    def test_the_head_reads_both_counts_and_whether_a_letter_starts_no_russian_word(self) -> None:
        features = head("ЕС", "TC", "страны ")
        names = {name.removeprefix(ABBREVIATION_FEATURE_PREFIX).removesuffix(":direction:1") for name in features}
        # `ес` is counted 41 times among the Cyrillic words of Russian technical text (bit length 6), `tc` 117 times
        # among its Latin words (bit length 7).
        self.assertTrue({"bias", "count:cyrillic:6", "count:latin:7", "count:ratio:1", "non_initial:0", "letters:2",
                         "trigger:space"} <= names)
        self.assertFalse([name for name in names if ":char:" in name])
        # `ЗЫ` (`PS`) holds `Ы`, which no Russian word starts with.
        self.assertIn(ABBREVIATION_FEATURE_PREFIX + "non_initial:1:direction:1", head("ЗЫ", "PS", "в конце "))
        self.assertEqual(head("ЕС", "TC", "the countries "), {})
        self.assertEqual(head("ЛУНЫ", "KEYS", "свет "), {})


class InstalledAbbreviationTests(ContextEngineTests):
    """The installed model answers an attested abbreviation after Russian prose with its own head."""

    def test_an_abbreviation_russian_text_uses_stays_after_russian_prose(self) -> None:
        # The corpus v35 model turned each into the Latin reading its keys spell: `TC`, `AYC`, `YU`.
        for text in ("Страны ЕС приняли санкции ", "Письмо из ФНС пришло ", "Поздравляю с НГ всех "):
            with self.subTest(text=text):
                self.reset_editor(1)
                self.type(text, group=1)
                self.assertEqual(self.backend.text, text)


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    suite = unittest.TestSuite()
    for case in (AbbreviationHeadTests, InstalledAbbreviationTests):
        for name in case.__dict__:
            if name.startswith("test_"):
                suite.addTest(case(name))
    return suite


if __name__ == "__main__":
    unittest.main()
