"""A word left as typed follows its neighbour once the neighbour turns out converted.

Two neighbours were left out. A lone letter that waited for its next word was refused by the
model when the next word converted: no corpus row has a lone letter, so its answer about
`f` before `потом` was no opinion, and `api f потом` kept its `f`. And the words before a
word the early switch converted were never asked again: that word reaches its boundary
already in the other layout, so `dct` opening a message stayed when `ujnjdj` after it became
`готово` on its fourth letter. The scripted models below answer by the question, so each
conversion here is the engine's rule, not a model's lean.
"""

from __future__ import annotations

import unittest

from keyswitch.context_model import ContextAction, ContextEvidence
from test_context_policy import ContextEngineTests
from test_early_switch_settling import ScriptedActionModel, ScriptedPrefix


class WaitingLetterTests(ContextEngineTests):
    def phrase(self, letter: str) -> str:
        """`api`, a letter that waits, and `потом` typed in the English layout."""

        def answer(item: ContextEvidence) -> ContextAction:
            if item.original == letter:
                return "keep" if item.field.after else "wait"
            return "convert" if item.original == "gjnjv" else "keep"

        self.engine.context_policy.model = ScriptedActionModel(answer)
        self.reset_editor(0)
        self.type(f"api {letter} gjnjv ")
        return self.backend.text

    def test_a_waiting_letter_that_reads_as_a_curated_word_goes_with_its_neighbour(self) -> None:
        self.assertEqual(self.phrase("f"), "api а потом ")

    def test_a_waiting_letter_no_rule_names_stays(self) -> None:
        # `x` is `ч`, not a word of the curated list: the model's answer stands.
        self.assertEqual(self.phrase("x"), "api x потом ")


class EarlySwitchedNeighbourTests(ContextEngineTests):
    def setUp(self) -> None:
        super().setUp()
        self.settings.set("detection.early_switch", True)
        self.engine.prefix_model = ScriptedPrefix("convert")  # type: ignore[assignment]

    def script(self, first: str, converts: bool) -> ScriptedActionModel:
        """The first word stays alone and converts, if `converts`, before the switched word."""

        def answer(item: ContextEvidence) -> ContextAction:
            if item.original == first:
                return "convert" if converts and item.field.after == "готово" else "keep"
            return "keep"

        model = ScriptedActionModel(answer)
        self.engine.context_policy.model = model
        return model

    def type_keys(self, text: str) -> None:
        """Press the keys of `text` as written in the English layout, whatever layout is active."""

        for character in text:
            shown = character if self.backend.group == 0 else self.pair.translate(character, "us", "ru")
            self.tap(self.key("space" if shown == " " else shown, shown))

    def test_the_first_word_of_a_line_follows_a_word_the_early_switch_converted(self) -> None:
        model = self.script("dct", converts=True)
        self.reset_editor(0)
        self.type_keys("dct ujnjdj ")
        self.assertEqual((self.backend.text, self.backend.group), ("все готово ", 1))
        self.assertIn(("dct", "", "готово"), model.questions)
        # Pause takes the phrase back to the keys as typed, as it does any context phrase.
        self.tap(self.key("Pause"))
        self.assertEqual(self.backend.text, "dct ujnjdj ")

    def test_a_first_word_the_model_keeps_stays(self) -> None:
        self.script("dct", converts=False)
        self.reset_editor(0)
        self.type_keys("dct ujnjdj ")
        self.assertEqual(self.backend.text, "dct готово ")

    def test_a_word_with_words_before_it_on_its_line_is_not_asked_again(self) -> None:
        # It was decided with them: only the first words of a line are taken along.
        self.script("dct", converts=True)
        self.reset_editor(0)
        self.type_keys("ok dct ujnjdj ")
        self.assertEqual(self.backend.text, "ok dct готово ")


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    suite = unittest.TestSuite()
    for case in (WaitingLetterTests, EarlySwitchedNeighbourTests):
        for name in case.__dict__:
            if name.startswith("test_"):
                suite.addTest(case(name))
    return suite


if __name__ == "__main__":
    unittest.main()
