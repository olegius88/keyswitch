"""Automatic training baselines preserve the engine's explicit exclusions."""

import unittest

from keyswitch.detector import LanguageDetector
from keyswitch.language_model import LanguageModel
from keyswitch.word_decision import automatic_word_decision


class AutomaticWordDecisionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.detector = LanguageDetector({
            0: LanguageModel("en_US", {"hello": 100_000, "if": 200_000}, "test", enable_spellcheck=False),
            1: LanguageModel("ru_RU", {"привет": 100_000, "не": 200_000}, "test", enable_spellcheck=False),
        })

    def test_default_and_engine_options_preserve_the_same_conversion(self) -> None:
        default = automatic_word_decision(self.detector, "ghbdtn", {1: "привет"}, 0)
        configured = automatic_word_decision(
            self.detector, "ghbdtn", {1: "привет"}, 0,
            ignored_words=set(), rejected_targets=set(), previous_words={},
        )
        self.assertTrue(default.should_convert)
        self.assertEqual(default, configured)
        self.assertEqual(default.replacement, "привет")

    def test_short_override_obeys_exclusions_and_rejected_direction(self) -> None:
        default = automatic_word_decision(self.detector, "ша", {0: "if"}, 1)
        self.assertTrue(default.should_convert)
        self.assertEqual(default.replacement, "if")
        ignored = automatic_word_decision(
            self.detector, "ша", {0: "if"}, 1, ignored_words={"ша"},
        )
        rejected = automatic_word_decision(
            self.detector, "ша", {0: "if"}, 1, rejected_targets={0},
        )
        self.assertFalse(ignored.should_convert)
        self.assertFalse(rejected.should_convert)

    def test_a_correct_word_and_an_unexplained_token_are_preserved(self) -> None:
        for original, alternate in (("hello", "руддщ"), ("zz", "яя")):
            with self.subTest(original=original):
                self.assertFalse(automatic_word_decision(
                    self.detector, original, {1: alternate}, 0,
                ).should_convert)
