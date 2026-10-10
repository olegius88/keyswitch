"""A field read too short to hold the word just typed is not the text the word went into.

VS Code with its screen-reader support off takes keys through an input it keeps empty, and UI Automation
reads that input: on the owner's laptop every read after `ghbdtn` held one character around the caret
(10.10.2026). The word was refused as typed into a changed field, or into another word after a click,
and only `Pause` converted it.

These cases live outside `test_context_policy` and `test_engine_behaviour` on purpose: those modules'
bytes are pinned by the context-action release receipt.
"""

from __future__ import annotations

import unittest
from collections.abc import Callable

from keyswitch.input_context import FieldContext
import test_input_sequence_matrix as sequences
from test_reopened_word import ConvertingModel, evaluated_words, technical_events


class InputBufferReader:
    """A field reader that sees only an editor's input buffer, never the document."""

    def __init__(self, shown: Callable[[str], tuple[str, str]], text: Callable[[], str]) -> None:
        self.shown = shown
        self.text = text

    def read(self, application: str, window: int) -> FieldContext:
        before, after = self.shown(self.text())
        return FieldContext(application, "1", before, after, source="uia")


# What the buffer holds: nothing, the key still in flight, or a letter of no word typed here.
BUFFERS: dict[str, Callable[[str], tuple[str, str]]] = {
    "empty": lambda text: ("", ""),
    "key_in_flight": lambda text: ("", text[-1:]),
    "foreign_letter": lambda text: ("", "x"),
}


class TooShortFieldTests(unittest.TestCase):
    def typed(self, shown: Callable[[str], tuple[str, str]], keys: str, *, click: bool = False,
              ) -> tuple[str, list[dict[str, object]]]:
        with sequences.session(0) as current:
            current.settings.set("detection.context_read_field", True)
            current.settings.set("diagnostics.technical_logging", True)
            current.engine.context_policy.reader = InputBufferReader(shown, lambda: current.backend.text)
            current.engine.context_policy.model = ConvertingModel()
            with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                if click:
                    current.command("Pointer")
                current.physical(keys)
            return current.backend.text, technical_events(logs.output)

    def test_a_word_converts_where_the_field_shows_only_an_input_buffer(self) -> None:
        for name, shown in BUFFERS.items():
            with self.subTest(buffer=name):
                text, events = self.typed(shown, "ghbdtn ")
                self.assertEqual(text, "привет ")
                self.assertEqual(evaluated_words(events), ["ghbdtn"])
                self.assertFalse([event for event in events
                                  if event["event"] in ("field_contradiction", "correction_aborted")])
                [decision] = [event for event in events if event["event"] == "context_decision"]
                self.assertEqual((decision["context_source"], decision["fallback_reason"]),
                                 ("observed", "field_too_short"))

    def test_a_word_begun_after_a_click_is_not_read_as_typed_into_another(self) -> None:
        # The first read, one letter after the caret, says the word began inside a word; the read
        # at its space cannot hold the word and drops that.
        text, events = self.typed(BUFFERS["foreign_letter"], "ghbdtn ", click=True)
        self.assertEqual(text, "привет ")
        [started] = [event for event in events if event["event"] == "word_started_inside_text"]
        self.assertEqual((started["head_letters"], started["tail_letters"]), (0, 1))
        dropped = [event for event in events if event["event"] == "insertion_dropped"]
        self.assertTrue(dropped)
        self.assertTrue(all(event["reason"] == "field_too_short" for event in dropped))
        self.assertFalse([event for event in events if event["event"] == "inside_word_decision"])

    def test_the_early_switch_keeps_the_observed_prefix(self) -> None:
        for name, shown in BUFFERS.items():
            with self.subTest(buffer=name), sequences.session(0) as current:
                current.settings.set("detection.context_read_field", True)
                current.engine.context_policy.reader = InputBufferReader(shown, lambda: "мир ghbd")
                field, reason = current.engine._early_prefix_field("ghbd", current.backend.active_application())
                self.assertEqual((reason, field.source), ("", "observed"))

    def test_a_field_long_enough_for_the_word_and_without_it_is_still_a_changed_field(self) -> None:
        text, events = self.typed(lambda text: ("other text ", ""), "ghbdtn ")
        self.assertEqual(text, "ghbdtn ")
        self.assertTrue([event for event in events if event["event"] == "field_contradiction"])
        with sequences.session(0) as current:
            current.settings.set("detection.context_read_field", True)
            current.engine.context_policy.reader = InputBufferReader(lambda text: ("x" * len("ghbd"), ""), lambda: "")
            _field, reason = current.engine._early_prefix_field("ghbd", current.backend.active_application())
            self.assertEqual(reason, "context_field_changed")


if __name__ == "__main__":
    unittest.main()
