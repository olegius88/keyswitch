"""Automatic training baselines preserve the engine's explicit exclusions."""

import unittest

from keyswitch.detector import DetectionDecision, LanguageDetector
from keyswitch.language_model import LanguageModel, WordScore
from keyswitch.short_words import ISOLATED_SHORT_WORD_REASON
from keyswitch.word_decision import automatic_word_decision, word_shape_veto

WORD_FREQUENCY = 100_000
HIGHER_WORD_FREQUENCY = 200_000


class AutomaticWordDecisionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.detector = LanguageDetector({
            0: LanguageModel("en_US", {"hello": WORD_FREQUENCY, "if": HIGHER_WORD_FREQUENCY}, "test", enable_spellcheck=False),
            1: LanguageModel("ru_RU", {"привет": WORD_FREQUENCY, "не": HIGHER_WORD_FREQUENCY}, "test", enable_spellcheck=False),
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


class WordShapeVetoTests(unittest.TestCase):
    """A word may only become a word; the reverse direction keeps working."""

    def decision(self, original: str, replacement: str, *, convert: bool = True) -> DetectionDecision:
        score = WordScore(0.0, False, 0, 0.0)
        return DetectionDecision(convert, original, replacement, 1, 0, 0.0, "fixture", score, score, None, None)

    def test_letters_may_not_become_punctuation(self) -> None:
        vetoed = word_shape_veto(self.decision("рукх", "her["))
        self.assertFalse(vetoed.should_convert)
        self.assertEqual(vetoed.reason, "другая раскладка пишет это не буквами")
        self.assertEqual((vetoed.original, vetoed.replacement), ("рукх", "her["))

    def test_the_opposite_direction_and_ordinary_pairs_are_untouched(self) -> None:
        for original, replacement in (("[jhjij", "хорошо"), ("ghbdtn", "привет"), ("don't", "вщт'е"),
                                       ("из-за", "bp-pf")):
            with self.subTest(original=original):
                self.assertTrue(word_shape_veto(self.decision(original, replacement)).should_convert)

    def test_a_decision_that_keeps_the_text_is_returned_unchanged(self) -> None:
        kept = self.decision("рукх", "her[", convert=False)
        self.assertIs(word_shape_veto(kept), kept)

class OpeningLetterTests(unittest.TestCase):
    """A lone curated letter converts with no previous word and stays after an English one."""

    def setUp(self) -> None:
        self.detector = LanguageDetector({
            0: LanguageModel("en_US", {"hello": WORD_FREQUENCY}, "test", enable_spellcheck=False),
            1: LanguageModel("ru_RU", {"привет": WORD_FREQUENCY}, "test", enable_spellcheck=False),
        })

    def decide(self, original: str, replacement: str, context_group: int | None, **options: object) -> DetectionDecision:
        return automatic_word_decision(self.detector, original, {1: replacement}, 0, context_group=context_group, **options)  # type: ignore[arg-type]

    def test_message_start_letters_convert_with_the_isolated_reason(self) -> None:
        for original, replacement in (("z", "я"), ("b", "и"), ("f", "а")):
            with self.subTest(original=original):
                decision = self.decide(original, replacement, None)
                self.assertTrue(decision.should_convert)
                self.assertEqual((decision.replacement, decision.reason), (replacement, ISOLATED_SHORT_WORD_REASON))

    def test_an_english_neighbour_a_foreign_letter_and_exclusions_keep_the_letter(self) -> None:
        self.assertFalse(self.decide("z", "я", 0).should_convert)
        self.assertFalse(self.decide("g", "п", None).should_convert)
        self.assertFalse(self.decide("z", "я", None, ignored_words=("z",)).should_convert)
        self.assertFalse(self.decide("z", "я", None, rejected_targets={1}).should_convert)
