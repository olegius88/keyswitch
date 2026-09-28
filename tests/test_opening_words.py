"""A short word converted at the space, and the next word that may take it back.

`ns` typed alone into an empty field is `ты` far more often than anything else: 10.1% of
Russian sentences open with a curated two-letter word, while their Latin readings open almost
no English one (UD Taiga and UD EWT, 28.09.2026). The model decides it at the space. If the
next word then goes back the other way - `vs` and `code` come out as `мы сщву` once the layout
followed - the engine asks the model about the first word again with the converted next word
after it, and when the model no longer converts it, one correction returns both words.
"""

from __future__ import annotations

import unittest

from keyswitch.context_model import ContextAction, ContextEvidence
from test_context_policy import ContextEngineTests
from test_context_wait_pairs import ScriptedModel
from test_inside_word import CaretReader
from fixture_values.clock import LAST_WORD_INPUT_AT_SECONDS, WAIT_PAIR_PAUSE_CHECK_SECONDS


def english_after(item: ContextEvidence) -> ContextAction:
    """`ns` converts unless an English word follows it, `сщву` is `code`, the rest stays."""

    if item.original == "ns":
        return "keep" if item.field.after == "code" else "convert"
    return "convert" if item.original == "сщву" else "keep"


class ConvertedWordRevisitTests(ContextEngineTests):
    """The engine's part, with a scripted model: it asks again, the model answers."""

    def setUp(self) -> None:
        super().setUp()
        self.settings.set("detection.context_read_field", True)
        self.engine.context_policy.reader = CaretReader(self)  # type: ignore[arg-type]

    def script(self, answer: object) -> ScriptedModel:
        model = ScriptedModel(answer)  # type: ignore[arg-type]
        self.engine.context_policy.model = model
        return model

    def pause(self) -> None:
        self.engine._last_word_input_at = LAST_WORD_INPUT_AT_SECONDS
        self.engine._maybe_correct_after_pause(now=WAIT_PAIR_PAUSE_CHECK_SECONDS)

    def test_the_model_takes_a_word_back_when_the_next_one_goes_the_other_way(self) -> None:
        model = self.script(english_after)
        self.reset_editor(0)
        self.type("ns ")
        self.assertEqual((self.backend.text, self.backend.group), ("ты ", 1))
        self.type("сщву ", group=1)
        self.assertEqual((self.backend.text, self.backend.group), ("ns code ", 0))
        # The first word was asked about again with the converted next word after it.
        self.assertIn(("ns", "", "code"), model.questions)

    def test_a_word_the_model_still_converts_stays_converted(self) -> None:
        self.script(lambda item: "convert" if item.original in {"ns", "сщву"} else "keep")
        self.reset_editor(0)
        self.type("ns ")
        self.type("сщву ", group=1)
        self.assertEqual(self.backend.text, "ты code ")

    def test_a_next_word_in_the_same_language_asks_nothing(self) -> None:
        model = self.script(english_after)
        self.reset_editor(0)
        self.type("ns ")
        self.type("привет ", group=1)
        self.assertEqual(self.backend.text, "ты привет ")
        self.assertNotIn(("ns", "", "привет"), model.questions)

    def test_a_next_word_decided_at_a_pause_takes_it_back_too(self) -> None:
        self.script(english_after)
        self.reset_editor(0)
        self.type("ns ")
        self.type("сщву", group=1)
        self.pause()
        self.assertEqual(self.backend.text, "ns code")

    def test_a_word_that_does_not_follow_it_directly_asks_nothing(self) -> None:
        model = self.script(english_after)
        self.reset_editor(0)
        self.type("ns ")
        self.type(" сщву ", group=1)
        self.assertEqual(self.backend.text, "ты  code ")
        self.assertNotIn(("ns", "", "code"), model.questions)

    def test_backspace_ends_the_claim_of_the_next_word(self) -> None:
        self.script(english_after)
        self.reset_editor(0)
        self.type("ns ")
        self.tap(self.key("BackSpace", "", group=1))
        self.type(" сщву ", group=1)
        self.assertTrue(self.backend.text.startswith("ты"))


class ShortWordModelTests(ContextEngineTests):
    """What the bundled model decides about short words, read from the field as on Windows."""

    def setUp(self) -> None:
        super().setUp()
        self.settings.set("detection.context_read_field", True)
        self.engine.context_policy.reader = CaretReader(self)  # type: ignore[arg-type]

    def test_ns_alone_converts_at_the_space(self) -> None:
        self.reset_editor(0)
        self.type("ns ")
        self.assertEqual((self.backend.text, self.backend.group), ("ты ", 1))

    def test_ns_then_a_russian_word(self) -> None:
        self.reset_editor(0)
        self.type("ns ")
        self.type("привет ", group=1)
        self.assertEqual(self.backend.text, "ты привет ")

    def test_ns_then_code_stays_english(self) -> None:
        self.reset_editor(0)
        self.type("ns ")
        self.type("сщву ", group=1)
        self.assertEqual(self.backend.text, "ns code ")

    def test_vs_then_code_stays_english(self) -> None:
        self.reset_editor(0)
        self.type("vs ")
        self.type("сщву ", group=1)
        self.assertEqual(self.backend.text, "vs code ")

    def test_a_clear_word_stays_converted_when_the_next_one_goes_back(self) -> None:
        for first, group, second, expected in (("cgfcb,j ", 0, "еуфь ", "спасибо team "),
                                               ("ghbdtn ", 0, "руддщ ", "привет hello "),
                                               ("руддщ ", 1, "ghbdtn ", "hello привет ")):
            with self.subTest(first=first):
                self.reset_editor(group)
                self.type(first, group=group)
                self.type(second, group=self.backend.group)
                self.assertEqual(self.backend.text, expected)

    def test_ns_after_russian_text_converts(self) -> None:
        for before in ("привет ", "я думаю, что ", "мы решили, что "):
            with self.subTest(before=before):
                self.reset_editor(0)
                self.backend.text, self.backend.caret = before, len(before)
                self.type("ns ")
                self.assertEqual(self.backend.text, before + "ты ")

    def test_ty_typed_as_intended_stays_with_text_after_the_caret(self) -> None:
        for before, after in (("", ""), ("на ", " и так понятно"), ("я знаю, что ", " прав")):
            with self.subTest(before=before, after=after):
                self.reset_editor(1)
                self.backend.text, self.backend.caret = before + after, len(before)
                self.type("ты ", group=1)
                self.assertEqual(self.backend.text, before + "ты " + after)


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    suite = unittest.TestSuite()
    for case in (ConvertedWordRevisitTests, ShortWordModelTests):
        for name in case.__dict__:
            if name.startswith("test_"):
                suite.addTest(case(name))
    return suite


if __name__ == "__main__":
    unittest.main()
