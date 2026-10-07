"""Lone letters: the curated rules convert them, the action model alone does not.

A one-letter Russian word typed in the English layout right after an English term in a Russian
phrase: the term switched the layout, or was typed in the English layout on purpose, and the
Russian phrase goes on: `поправь env b` is `поправь env и`. The engine converts such a curated
letter by rule; the scripted model below keeps everything, so each conversion here is the rule's.
And a letter no rule names stays whatever the model says: it never saw a lone letter.
"""

from __future__ import annotations

import unittest

from keyswitch.context_model import ContextAction, ContextEvidence
from test_context_policy import ContextEngineTests
from test_early_switch_settling import ScriptedActionModel


def keep(item: ContextEvidence) -> ContextAction:
    return "keep"


class StrandedLetterTests(ContextEngineTests):
    def setUp(self) -> None:
        super().setUp()
        self.engine.context_policy.model = ScriptedActionModel(keep)

    def phrase(self, russian: str, english: str) -> str:
        """Type a Russian phrase, then the rest in the English layout, and return the text."""

        self.reset_editor(1)
        self.type(russian, group=1)
        self.backend.group = 0
        self.type(english)
        return self.backend.text

    def test_a_letter_after_a_term_in_a_russian_phrase_is_the_russian_word(self) -> None:
        self.assertEqual(self.phrase("поправь ", "env b "), "поправь env и ")
        self.assertEqual(self.phrase("что-то вроде как в ", "git diff d "), "что-то вроде как в git diff в ")

    def test_a_letter_after_latin_text_alone_stays(self) -> None:
        self.reset_editor(0)
        self.type("git diff d ")
        self.assertEqual(self.backend.text, "git diff d ")
        # Past the longest term the rule reads, the text before the letter is English.
        self.assertEqual(self.phrase("это ", "one two three four b "), "это one two three four b ")

    def test_a_capital_letter_a_sentence_end_and_a_letter_no_rule_names_stay(self) -> None:
        self.assertEqual(self.phrase("поправь ", "env B "), "поправь env B ")
        self.assertEqual(self.phrase("поправь ", "env. b "), "поправь env. b ")
        self.assertEqual(self.phrase("поправь. ", "env b "), "поправь. env b ")
        self.assertEqual(self.phrase("поправь ", "env x "), "поправь env x ")


def letters(item: ContextEvidence) -> ContextAction:
    """The model converts every token with a single letter and keeps every word."""

    return "convert" if sum(char.isalpha() for char in item.original) == 1 else "keep"


class LoneLetterModelTests(ContextEngineTests):
    def setUp(self) -> None:
        super().setUp()
        self.engine.context_policy.model = ScriptedActionModel(letters)

    def test_the_model_alone_does_not_convert_a_lone_letter(self) -> None:
        # `ч` is `x`, a letter no curated rule names, and the model was never taught one.
        self.reset_editor(1)
        self.type("подробнее ч ", group=1)
        self.assertEqual(self.backend.text, "подробнее ч ")

    def test_a_single_letter_with_digits_stays_whatever_the_model_says(self) -> None:
        # `1С` is a product name, `а1` a cell; their Latin keys `1C` and `f1` mean nothing here.
        self.reset_editor(1)
        self.type("работаю в 1С и а1 ", group=1)
        self.assertEqual(self.backend.text, "работаю в 1С и а1 ")

    def test_a_lone_letter_converts_when_a_curated_rule_agrees_with_the_model(self) -> None:
        self.reset_editor(1)
        self.type("привет ", group=1)
        self.backend.group = 0
        self.type("f ")
        self.assertEqual(self.backend.text, "привет а ")


class LetterHeadTests(ContextEngineTests):
    """A model with the single-letter head has learned the letter right after a Latin word: there its
    verdict stands, and elsewhere a lone letter still needs a curated rule."""

    def setUp(self) -> None:
        super().setUp()
        model = ScriptedActionModel(letters)
        model.answers_letters = True
        self.engine.context_policy.model = model

    def test_a_letter_after_a_latin_word_follows_a_model_that_learned_it(self) -> None:
        # `nats b redis` is `nats и redis`: no curated rule reads a letter after Latin text alone.
        self.reset_editor(0)
        self.type("nats b ")
        self.assertEqual(self.backend.text, "nats и ")

    def test_outside_the_heads_class_a_lone_letter_still_needs_a_rule(self) -> None:
        self.reset_editor(1)
        self.type("подробнее ч ", group=1)
        self.assertEqual(self.backend.text, "подробнее ч ")
        self.reset_editor(1)
        self.type("работаю в 1С и а1 ", group=1)
        self.assertEqual(self.backend.text, "работаю в 1С и а1 ")


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    suite = unittest.TestSuite()
    for case in (StrandedLetterTests, LoneLetterModelTests, LetterHeadTests):
        for name in case.__dict__:
            if name.startswith("test_"):
                suite.addTest(case(name))
    return suite


if __name__ == "__main__":
    unittest.main()
