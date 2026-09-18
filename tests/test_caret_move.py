"""A word typed after the caret moved has surroundings the engine has not seen."""

from __future__ import annotations

import json
import unittest
from typing import cast

from test_input_integrity import InputIntegrityTests


class Reader:
    """A field reader that answers, the way a supported application's does."""

    status = "available"

    def read(self, application: str, window: int) -> object:
        from keyswitch.input_context import FieldContext
        return FieldContext(application, str(window), "уже написано ", "", source="native")


class CaretMoveTests(InputIntegrityTests):
    def setUp(self) -> None:
        super().setUp()
        self.settings.set("detection.context_policy", "assist")

    def evaluations(self, lines: list[str]) -> list[dict[str, object]]:
        events = [json.loads(line.split("TECHNICAL ", 1)[1]) for line in lines if "TECHNICAL " in line]
        return [event for event in events if event["event"] == "word_evaluation"]

    def test_a_word_typed_after_an_arrow_key_is_left_alone(self) -> None:
        self.type("ghbdtn ")
        self.assertEqual(self.backend.text, "привет ")
        self.reset_editor()
        self.tap(self.key("Left"))
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type("ghbdtn ", group=0)
        self.assertEqual(self.backend.text, "ghbdtn ")
        evaluation = self.evaluations(logs.output)[-1]
        self.assertEqual(evaluation["skipped_reason"], "caret_moved")
        # What the detector would have said is still recorded, so a missed
        # correction can be told from a wrong verdict.
        shadow = cast(dict[str, object], evaluation["shadow_decision"])
        self.assertTrue(shadow["should_convert"])

    def test_only_the_word_right_after_the_move_is_held(self) -> None:
        self.tap(self.key("Left"))
        self.type("ghbdtn ", group=0)
        self.assertEqual(self.backend.text, "ghbdtn ")
        self.type("ghbdtn ", group=0)
        self.assertEqual(self.backend.text, "ghbdtn привет ")

    def test_erasing_after_the_move_answers_the_question_the_rule_asks(self) -> None:
        """The rule holds a word because nobody knows what precedes the caret.

        A user who moves the caret and then rubs the text out has said what precedes
        it: nothing. Holding the next word after that is caution with no question left,
        and it is what an editor's own "select all, erase, retype" does every time.
        """

        self.tap(self.key("Left"))
        self.tap(self.key("BackSpace"))
        self.type("ghbdtn ", group=0)
        self.assertEqual(self.backend.text, "привет ")
        # The shape an editor actually uses to empty a single-line field, and the one
        # the native Windows end-to-end test drives: Home, Shift+End, BackSpace.
        self.reset_editor()
        self.type("руддщ", group=1)
        self.tap(self.key("Home"))
        self.tap(self.key("End"))
        self.tap(self.key("BackSpace"))
        self.type("ghbdtn ", group=0)
        self.assertTrue(self.backend.text.endswith("привет "), self.backend.text)

    def test_pause_still_converts_the_held_word_by_hand(self) -> None:
        self.tap(self.key("Left"))
        self.type("ghbdtn ", group=0)
        self.assertEqual(self.backend.text, "ghbdtn ")
        self.tap(self.key("Pause"))
        self.assertEqual(self.backend.text, "привет ")

    def test_every_caret_key_counts_and_the_setting_turns_the_rule_off(self) -> None:
        for name in ("Left", "Right", "Home", "End", "Up", "Down", "Page_Up", "Page_Down"):
            with self.subTest(key=name):
                self.reset_editor()
                self.tap(self.key(name))
                self.type("ghbdtn ", group=0)
                self.assertEqual(self.backend.text, "ghbdtn ")
        self.settings.set("detection.hold_after_caret_move", False)
        self.reset_editor()
        self.tap(self.key("Left"))
        self.type("ghbdtn ", group=0)
        self.assertEqual(self.backend.text, "привет ")

    def test_a_readable_field_answers_the_question_and_the_rule_stands_aside(self) -> None:
        """With the real text before the caret in hand, nothing has to be guessed.

        The word is then judged on that text, whatever the judgement turns out to
        be; what this rule decides is only whether the question is asked at all.
        """

        self.settings.set("detection.context_read_field", True)
        self.engine.context_policy.reader = cast(object, Reader())  # type: ignore[assignment]
        self.tap(self.key("Left"))
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type("ghbdtn ", group=0)
        self.assertIsNone(self.evaluations(logs.output)[-1]["skipped_reason"])
        self.reset_editor()
        self.engine.context_policy.reader = None
        self.tap(self.key("Left"))
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type("ghbdtn ", group=0)
        self.assertEqual(self.evaluations(logs.output)[-1]["skipped_reason"], "caret_moved")

    def test_field_reading_turned_off_leaves_nothing_to_ask(self) -> None:
        """Without the reader there is no way to learn what precedes the caret."""

        self.settings.set("detection.context_read_field", False)
        self.engine.context_policy.reader = cast(object, Reader())  # type: ignore[assignment]
        self.tap(self.key("Left"))
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type("ghbdtn ", group=0)
        self.assertEqual(self.backend.text, "ghbdtn ")
        self.assertEqual(self.evaluations(logs.output)[-1]["skipped_reason"], "caret_moved")

    def test_an_early_switch_does_not_fire_after_the_caret_moved(self) -> None:
        self.settings.set("detection.early_switch", True)
        self.tap(self.key("Left"))
        self.type("ghbdtn", group=0)
        self.assertEqual(self.backend.text, "ghbdtn")
        self.assertIsNone(self.engine._early_switch_origin)


# Only this module's own cases; the inherited harness keeps its own module.
def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    suite = unittest.TestSuite()
    for name in CaretMoveTests.__dict__:
        if name.startswith("test_"):
            suite.addTest(CaretMoveTests(name))
    return suite


if __name__ == "__main__":
    unittest.main()
