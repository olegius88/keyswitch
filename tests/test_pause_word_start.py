"""A pause in the middle of a word is a pause to think, not a boundary.

`иг` of `игру` became `bu` at a pause and came back once the word was finished (0.44.0 log,
08.10.2026), as `фл` of `флагом` had become `ak` (03.10.2026). Two letters that many words of their
own language begin with are decided at the word's boundary; a short word typed in the other layout
(`gj` is `по`) still converts at a pause. A curated short-word rule at a pause waits for the word's boundary when its
two letters begin many words of their own language (`ша` of `шаблоне` is no `if` yet), and applies as before to other
tokens and to a lone letter.
"""

from __future__ import annotations

import itertools
import unittest
from dataclasses import replace

from keyswitch.constants.detection import PAUSE_WORD_START_MIN_WORDS
from keyswitch.context_model import ContextAction
from keyswitch.context_policy import ContextPolicy
from keyswitch.detector import DetectionDecision, LanguageDetector
from keyswitch.input_context import FieldContext
from keyswitch.language_model import LanguageModel
from keyswitch.short_words import ISOLATED_SHORT_WORD_REASON, begins_words, word_starts
from test_early_switch_settling import ScriptedActionModel
from test_context_policy import ContextEngineTests
from test_context_wait_pairs import ScriptedModel
from test_inside_word import whole_word
from fixture_values.clock import LAST_WORD_INPUT_AT_SECONDS, WAIT_PAIR_PAUSE_CHECK_SECONDS


class PauseWordStartTests(ContextEngineTests):
    def script(self, answer: dict[str, ContextAction]) -> ScriptedModel:
        model = ScriptedModel(whole_word(answer))
        self.engine.context_policy.model = model
        return model

    def pause(self) -> None:
        self.engine._last_word_input_at = LAST_WORD_INPUT_AT_SECONDS
        self.engine._maybe_correct_after_pause(now=WAIT_PAIR_PAUSE_CHECK_SECONDS)

    def test_the_start_of_a_word_is_not_converted_at_a_pause(self) -> None:
        model = self.script({"иг": "convert"})
        self.reset_editor(1)
        self.type("иг", group=1)
        self.pause()
        self.assertEqual(self.backend.text, "иг")
        self.assertEqual(model.questions, [])
        self.type("ру ", group=1)
        self.assertEqual(self.backend.text, "игру ")

    def test_the_fragment_is_still_decided_at_its_boundary(self) -> None:
        self.script({"иг": "convert"})
        self.reset_editor(1)
        self.type("иг", group=1)
        self.pause()
        self.type(" ", group=1)
        self.assertEqual(self.backend.text, "bu ")

    def test_an_english_word_typed_in_the_russian_layout_converts_at_a_pause(self) -> None:
        self.script({"ша": "convert"})
        self.reset_editor(1)
        self.type("ша", group=1)
        self.pause()
        self.assertEqual(self.backend.text, "if")

    def test_a_short_word_typed_in_the_other_layout_converts_at_a_pause(self) -> None:
        self.script({"gj": "convert"})
        self.reset_editor(0)
        self.type("gj", group=0)
        self.pause()
        self.assertEqual(self.backend.text, "по")

    def test_the_rule_counts_the_lexicon_of_the_fragments_own_language(self) -> None:
        english, russian = self.engine.models[0], self.engine.models[1]
        self.assertGreaterEqual(word_starts(russian)["иг"], PAUSE_WORD_START_MIN_WORDS)
        self.assertLess(word_starts(english)["gj"], PAUSE_WORD_START_MIN_WORDS)
        self.assertTrue(self.engine._word_begun("Иг", 1, "Bu", 0))
        self.assertFalse(self.engine._word_begun("игр", 1, "buh", 0))
        self.assertFalse(self.engine._word_begun("и2", 1, "b2", 0))
        self.assertFalse(self.engine._word_begun("иг", len(self.engine.models), "bu", 0))
        # `ша` typed for `if` begins many Russian words too, but English prose has `if`.
        self.assertFalse(self.engine._word_begun("ша", 1, "if", 0))
        # A layout without a prose table of its own.
        self.assertFalse(self.engine._word_begun("иг", 1, "bu", len(self.engine.models)))


FREQUENT_SHORT_WORD_REASON = "частотное короткое слово из безопасного списка"
RUSSIAN_LETTERS = "абвгдежзиклмнопрстуфхцчшщэюя"


def models() -> dict[int, LanguageModel]:
    """English with no word beginning `yf`, Russian with as many words beginning `ша` as make a word start."""

    starts = itertools.islice(("ша" + a + b for a in RUSSIAN_LETTERS for b in RUSSIAN_LETTERS), PAUSE_WORD_START_MIN_WORDS)
    russian = {word: 1 for word in starts}
    english = {"if": 1, "yes": 1}
    return {0: LanguageModel("en_US", english, "fixture", enable_spellcheck=False),
            1: LanguageModel("ru_RU", russian, "fixture", enable_spellcheck=False)}


def rule(found: LanguageDetector, original: str, alternative: str, source: int, reason: str) -> DetectionDecision:
    target = 1 - source
    return replace(found.decide(original, {target: alternative}, source), should_convert=True, replacement=alternative,
                   target_group=target, reason=reason)


class PauseRuleWordStartTests(unittest.TestCase):
    def setUp(self) -> None:
        self.models = models()
        self.detector = LanguageDetector(self.models)
        self.policy = ContextPolicy()
        self.policy.model = ScriptedActionModel(lambda item: "keep")
        self.field = FieldContext("Code", "1", "на ", "", "text")

    def decide(self, baseline: DetectionDecision, trigger: str) -> DetectionDecision:
        return self.policy.decide(baseline, baseline.replacement, baseline.target_group, self.detector, trigger, "assist",
                                  field_override=self.field).decision

    def test_two_letters_that_begin_many_words_wait_for_the_boundary_at_a_pause(self) -> None:
        self.assertTrue(begins_words(self.models[1], "ша"))
        self.assertTrue(begins_words(self.models[1], "ША"))
        baseline = rule(self.detector, "ша", "if", 1, FREQUENT_SHORT_WORD_REASON)
        self.assertFalse(self.decide(baseline, "pause").should_convert)

    def test_other_tokens_and_a_lone_letter_keep_the_rule_at_a_pause(self) -> None:
        english = self.models[0]
        self.assertFalse(begins_words(english, "yf"))
        self.assertFalse(begins_words(self.models[1], "ш"))
        self.assertFalse(begins_words(self.models[1], "ш1"))
        self.assertEqual(word_starts(english)["ye"], 1)
        for baseline in (rule(self.detector, "yf", "на", 0, FREQUENT_SHORT_WORD_REASON),
                         rule(self.detector, "d", "в", 0, ISOLATED_SHORT_WORD_REASON)):
            with self.subTest(original=baseline.original):
                self.assertTrue(self.decide(baseline, "pause").should_convert)



def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    return unittest.TestSuite([*(PauseWordStartTests(name) for name in PauseWordStartTests.__dict__ if name.startswith("test_")),
                               *(PauseRuleWordStartTests(name) for name in PauseRuleWordStartTests.__dict__
                                 if name.startswith("test_"))])


if __name__ == "__main__":
    unittest.main()
