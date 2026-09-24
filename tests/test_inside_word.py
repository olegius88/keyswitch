"""Letters typed into the middle of a written word are judged as that whole word.

Clicking into written text and typing is how a word is fixed in place: `сд|лать` gets
its missing `е`. The engine had seen none of that text, so the typed letters were
judged as a word of their own; a correct `е` became `t` at the pause and a mistyped
`t` stayed whenever the letter alone looked like something. The field is now read as
the word begins, and a fragment inside a word is decided as the whole word.
"""

from __future__ import annotations

import unittest
from collections.abc import Callable
from dataclasses import replace

from keyswitch.backend import KeyEvent
from keyswitch.context_model import ContextAction, ContextEvidence
from keyswitch.engine import InsertionPoint
from keyswitch.input_context import FieldContext
from test_context_policy import ContextEngineTests
from test_context_wait_pairs import ScriptedModel
from fixture_values.clock import LAST_WORD_INPUT_AT_SECONDS, WAIT_PAIR_PAUSE_CHECK_SECONDS


class CaretReader:
    """Reads the editor the way UI Automation does: the text before and after the caret."""

    status = "available"

    def __init__(self, test: InsideWordTests) -> None:
        self.test = test
        self.override: Callable[[FieldContext], FieldContext | None] | None = None

    def read(self, application: str, window: int) -> FieldContext | None:
        backend = self.test.backend
        field = FieldContext(
            backend.active_application(), "1", backend.text[:backend.caret], backend.text[backend.caret:],
            role="text", source="uia",
        )
        return field if self.override is None else self.override(field)


def whole_word(answer: dict[str, ContextAction]) -> Callable[[ContextEvidence], ContextAction]:
    """The model's answer for each whole word it is asked about; anything else is kept."""

    return lambda item: answer.get(item.original, "keep")


class InsideWordTests(ContextEngineTests):
    def setUp(self) -> None:
        super().setUp()
        self.application = self.backend.active_application()
        self.settings.set("detection.context_read_field", True)
        self.reader = CaretReader(self)
        self.engine.context_policy.reader = self.reader

    def script(self, answer: dict[str, ContextAction]) -> ScriptedModel:
        model = ScriptedModel(whole_word(answer))
        self.engine.context_policy.model = model
        return model

    def click_into(self, text: str, caret: int, group: int) -> None:
        self.reset_editor(group)
        self.backend.text, self.backend.caret = text, caret
        pointer = KeyEvent(True, 0, "Pointer", "", ("", ""), group, 0, 0)
        self.engine._handle(pointer)
        self.engine._handle(replace(pointer, pressed=False))

    def pause(self) -> None:
        self.engine._last_word_input_at = LAST_WORD_INPUT_AT_SECONDS
        self.engine._maybe_correct_after_pause(now=WAIT_PAIR_PAUSE_CHECK_SECONDS)

    def test_a_letter_in_the_other_layout_is_decided_as_the_whole_word(self) -> None:
        model = self.script({"cltkfnm": "convert"})
        self.click_into("мы хотим сдлать это", len("мы хотим сд"), 0)
        self.type("t")
        self.pause()
        self.assertEqual(self.backend.text, "мы хотим сделать это")
        # The model saw the whole word and the text around it, not the letter.
        self.assertIn(("cltkfnm", "мы хотим ", " это"), model.questions)

    def test_a_letter_in_its_own_layout_is_left_alone(self) -> None:
        model = self.script({"е": "convert"})
        self.click_into("мы хотим сдлать это", len("мы хотим сд"), 1)
        self.type("е", group=1)
        self.pause()
        self.assertEqual(self.backend.text, "мы хотим сделать это")
        self.assertEqual(model.questions, [])

    def test_a_punctuation_key_inside_a_word_is_its_letter(self) -> None:
        """`,` between `те` and `е` is the `б` of `тебе`, not punctuation."""

        self.script({"nt,t": "convert"})
        self.click_into("я говорю тее это", len("я говорю те"), 0)
        self.type(",")
        self.pause()
        self.assertEqual(self.backend.text, "я говорю тебе это")

    def test_a_whole_word_the_model_keeps_leaves_the_fragment(self) -> None:
        self.script({})
        self.click_into("мы хотим сдлать это", len("мы хотим сд"), 0)
        self.type("t")
        self.pause()
        self.assertEqual(self.backend.text, "мы хотим сдtлать это")
        self.assertIsNone(self.engine._context_result)

    def test_mixed_surroundings_leave_the_fragment_alone(self) -> None:
        model = self.script({"cltkfnm": "convert"})
        self.click_into("мы хотим сдlать это", len("мы хотим сд"), 0)
        self.type("t")
        self.pause()
        self.assertEqual(self.backend.text, "мы хотим сдtlать это")
        self.assertEqual(model.questions, [])

    def test_a_field_that_changed_before_the_decision_leaves_the_fragment(self) -> None:
        self.script({"cltkfnm": "convert"})
        for changed in (lambda field: replace(field, before=field.before + "!"), lambda _field: None):
            with self.subTest(changed=changed):
                self.reader.override = None
                self.click_into("мы хотим сдлать это", len("мы хотим сд"), 0)
                self.type("t")
                self.reader.override = changed
                self.pause()
                self.assertEqual(self.backend.text, "мы хотим сдtлать это")
                self.assertEqual(self.engine._insertion, InsertionPoint("сд", "лать"))

    def test_a_space_typed_into_a_word_ends_the_word_there(self) -> None:
        """Only the letters before the caret belong to the word the space ends."""

        model = self.script({"clt": "convert"})
        self.click_into("мы хотим сдлать это", len("мы хотим сд"), 0)
        self.type("t ")
        self.assertEqual(self.backend.text, "мы хотим сде лать это")
        self.assertIn(("clt", "мы хотим ", "лать это"), model.questions)

    def test_a_word_typed_between_words_is_a_word_of_its_own(self) -> None:
        self.script({"jxtym": "convert"})
        self.click_into("мы хотим это", len("мы "), 0)
        self.type("jxtym ")
        self.assertEqual(self.backend.text, "мы очень хотим это")
        self.click_into("мы  хотим это", len("мы "), 0)
        self.assertIsNone(self.engine._insertion)
        self.type("j")
        self.assertEqual(self.engine._insertion, InsertionPoint("", ""))

    def test_the_early_switch_waits_inside_a_word(self) -> None:
        self.click_into("мы хотим сдлать это", len("мы хотим сд"), 0)
        self.type("t")
        self.assertEqual(self.engine._early_prefix_protection("t", 0, 1), "inside_word")

    def test_no_insertion_point_without_a_readable_field(self) -> None:
        event = self.key("t", "t")
        cases: tuple[Callable[[], None], ...] = (
            lambda: setattr(self.engine.context_policy, "reader", None),
            lambda: self.settings.set("detection.context_read_field", False),
            lambda: self.settings.set("exclusions.applications", [self.application]),
            lambda: setattr(self.reader, "override", lambda _field: None),
            lambda: setattr(self.reader, "override", lambda field: replace(field, sensitive=True)),
            lambda: setattr(self.reader, "override", lambda field: replace(field, selection=True)),
            lambda: setattr(self.reader, "override", lambda field: replace(field, application="other")),
        )
        for prepare in cases:
            with self.subTest(prepare=prepare):
                self.setUp()
                prepare()
                self.assertIsNone(self.engine._read_insertion(event))

    def test_the_first_key_counts_whether_or_not_it_reached_the_editor(self) -> None:
        self.click_into("сдлать", len("сд"), 0)
        event = self.key("t", "t")
        self.assertEqual(self.engine._read_insertion(event), InsertionPoint("сд", "лать"))
        self.backend.text, self.backend.caret = "сдtлать", len("сдt")
        self.assertEqual(self.engine._read_insertion(event), InsertionPoint("сд", "лать"))

    def test_an_unknown_layout_has_no_letters(self) -> None:
        self.assertEqual(self.engine._layout_name(len(self.engine.models)), "")
        self.assertEqual(self.engine._layout_letters(len(self.engine.models)), frozenset())

    def test_a_decision_outside_a_word_is_not_taken_here(self) -> None:
        self.engine._insertion = InsertionPoint("", "")
        self.assertIsNone(self.engine._decide_inside_word((), 0, {1: ""}, self.application, "pause", None))
        self.engine._insertion = InsertionPoint("сд", "")
        self.assertIsNone(self.engine._decide_inside_word((), 0, {}, self.application, "pause", None))
        # The first read saw letters that are gone by the decision: an ordinary word.
        self.backend.text, self.backend.caret = "мы t", len("мы t")
        self.assertIsNone(self.engine._decide_inside_word(
            (self.key("t", "t"),), 0, {1: "е"}, self.application, "pause", None))


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    suite = unittest.TestSuite()
    for name in InsideWordTests.__dict__:
        if name.startswith("test_"):
            suite.addTest(InsideWordTests(name))
    return suite


if __name__ == "__main__":
    unittest.main()
