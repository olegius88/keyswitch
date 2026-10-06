"""A field that reports its caret a character or two short of what was typed is still that field.

VS Code Insiders (06.10.2026) put the last typed letter of its chat box after the caret it reported
through UI Automation: every word there was refused as a changed field, and every new word read as
typed into the middle of another. The text itself was exactly the one typed - the manual conversion
of the same words went through.

These cases live outside `test_context_policy` on purpose: that module's bytes are pinned by the
context-action release receipt.
"""

from __future__ import annotations

import unittest
from collections.abc import Callable

from keyswitch.constants.text import FIELD_CARET_LAG_MAX_CHARACTERS
from keyswitch.context_policy import caret_lag
from keyswitch.input_context import FieldContext
import test_input_sequence_matrix as sequences
from test_reopened_word import ConvertingModel, evaluated_words, technical_events


class LaggingReader:
    """A field reader whose caret stands `lag` characters short of the end of the text."""

    def __init__(self, text: Callable[[], str], lag: int) -> None:
        self.text = text
        self.lag = lag

    def read(self, application: str, window: int) -> FieldContext:
        text = self.text()
        cut = max(0, len(text) - self.lag)
        return FieldContext(application, "1", text[:cut], text[cut:], source="uia")


class CaretLagTests(unittest.TestCase):
    def test_the_lag_is_the_typed_text_found_just_after_the_caret(self) -> None:
        self.assertEqual(caret_lag("мир ghbdtn", "", "ghbdtn"), 0)
        self.assertEqual(caret_lag("мир ghbdtn", " ", "ghbdtn"), 0)
        self.assertEqual(caret_lag("мир ghbdt", "n", "ghbdtn"), 1)
        self.assertEqual(caret_lag("мир ghbd", "tn", "ghbdtn"), FIELD_CARET_LAG_MAX_CHARACTERS)
        # The boundary may have reached the editor or not; exactly, it has to be there.
        self.assertEqual(caret_lag("мир ghbdt", "n ", "ghbdtn"), 1)
        self.assertEqual(caret_lag("мир ghbdtn", " ", "ghbdtn ", exact=True), 1)
        self.assertIsNone(caret_lag("мир ghbdt", "n", "ghbdtn ", exact=True))
        # Further than that, or another text altogether, is a changed field.
        self.assertIsNone(caret_lag("мир gh", "bdtn", "ghbdtn"))
        self.assertIsNone(caret_lag("something else", "", "ghbdtn"))
        self.assertIsNone(caret_lag("мир ghbdtn x", "", "ghbdtn ", exact=True))


class LaggingFieldTests(unittest.TestCase):
    def converted(self, lag: int) -> tuple[str, list[dict[str, object]]]:
        with sequences.session(0) as current:
            current.settings.set("detection.context_read_field", True)
            current.settings.set("diagnostics.technical_logging", True)
            current.engine.context_policy.reader = LaggingReader(lambda: current.backend.text, lag)
            current.engine.context_policy.model = ConvertingModel()
            with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                current.physical("ghbdtn ")
            return current.backend.text, technical_events(logs.output)

    def test_a_word_converts_where_the_caret_is_reported_short_of_it(self) -> None:
        for lag in range(FIELD_CARET_LAG_MAX_CHARACTERS + 1):
            with self.subTest(lag=lag):
                text, events = self.converted(lag)
                self.assertEqual(text, "привет ")
                self.assertEqual(evaluated_words(events), ["ghbdtn"])
                self.assertFalse([event for event in events
                                  if event["event"] in ("field_contradiction", "correction_aborted", "word_started_inside_text")])

    def test_a_caret_further_off_is_a_changed_field_and_says_where_the_word_stood(self) -> None:
        # One character further the word is decided with its space after the caret, and the check
        # before the correction runs, which wants the space too, holds the line.
        text, events = self.converted(FIELD_CARET_LAG_MAX_CHARACTERS + 1)
        self.assertEqual(text, "ghbdtn ")
        [aborted] = [event for event in events if event["event"] == "correction_aborted"]
        self.assertEqual(aborted["reason"], "context_field_changed")
        # Two further the decision itself is refused, and the log says where the word ended: the
        # space has reached the editor, so the word ends one short of the lag after the caret.
        lag = FIELD_CARET_LAG_MAX_CHARACTERS + len("  ")
        text, events = self.converted(lag)
        self.assertEqual(text, "ghbdtn ")
        [contradiction] = [event for event in events if event["event"] == "field_contradiction"]
        self.assertEqual((contradiction["typed_characters"], contradiction["typed_end_from_caret"]),
                         (len("ghbdtn"), lag - len(" ")))

    def test_the_early_switch_reads_a_lagging_field_as_the_typed_prefix(self) -> None:
        with sequences.session(0) as current:
            current.settings.set("detection.context_read_field", True)
            application = current.backend.active_application()
            for lag, refusal in ((0, ""), (1, ""), (FIELD_CARET_LAG_MAX_CHARACTERS + 1, "context_field_changed")):
                with self.subTest(lag=lag):
                    current.engine.context_policy.reader = LaggingReader(lambda: "мир ghbd", lag)
                    field, reason = current.engine._early_prefix_field("ghbd", application)
                    self.assertEqual(reason, refusal)
                    if not refusal:
                        self.assertEqual((field.before, field.after), ("мир ", ""))

    def test_another_text_altogether_is_still_refused(self) -> None:
        with sequences.session(0) as current:
            current.settings.set("detection.context_read_field", True)
            current.settings.set("diagnostics.technical_logging", True)
            current.engine.context_policy.reader = LaggingReader(lambda: "something else ", 0)
            current.engine.context_policy.model = ConvertingModel()
            with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                current.physical("ghbdtn ")
            self.assertEqual(current.backend.text, "ghbdtn ")
            [contradiction] = [event for event in technical_events(logs.output) if event["event"] == "field_contradiction"]
            self.assertIsNone(contradiction["typed_end_from_caret"])
            self.assertNotIn("something", str(contradiction))


if __name__ == "__main__":
    unittest.main()
