"""A field read as a key goes down may not show that key yet, and that is no change.

The early switch reads the field when the fourth letter of a word goes down. A browser or an
Electron editor reports its text a moment after the key, and the read was refused as a changed
field: at the fourth letter in 175 of 210 words in Firefox, 174 of 416 in the Claude app and 532
of 2 275 in VS Code (Windows logs, 05.09-08.10.2026). The early switch hardly ever worked there.
"""

from __future__ import annotations

import logging
import unittest
from collections.abc import Callable
from dataclasses import replace

from keyswitch.constants.text import FIELD_PENDING_MAX_CHARACTERS
from keyswitch.engine import KeySwitchEngine
from keyswitch.input_context import FieldContext
import test_input_sequence_matrix as sequences
from test_reopened_word import technical_events


class BehindReader:
    """A field reader that shows the text `behind` characters short of what was typed."""

    def __init__(self, text: Callable[[], str]) -> None:
        self.text = text
        self.behind = 0

    def read(self, application: str, window: int) -> FieldContext:
        text = self.text()
        return FieldContext(application, "1", text[:len(text) - self.behind], "", source="uia")


class FieldBehindTests(unittest.TestCase):
    def test_the_typed_word_is_found_whole_short_of_its_last_letters_or_with_more_after_it(self) -> None:
        located = KeySwitchEngine._typed_in_field
        field = FieldContext("editor", "1", "мир ghbd", " after", source="uia")
        self.assertEqual(located(field, "ghbd", ""), ("мир ", " after"))
        for pending in range(1, FIELD_PENDING_MAX_CHARACTERS + 1):
            with self.subTest(pending=pending):
                self.assertEqual(located(field, "ghbd" + "tn"[:pending], ""), ("мир ", " after"))
        self.assertIsNone(located(field, "ghbd" + "tn "[:FIELD_PENDING_MAX_CHARACTERS + 1], ""))
        # At least one letter of the word has to be there.
        self.assertEqual(located(replace(field, before="мир g"), "gh", ""), ("мир ", " after"))
        self.assertIsNone(located(replace(field, before="мир "), "gh", ""))
        # Text typed ahead and a caret reported short of it.
        self.assertEqual(located(replace(field, before="мир ghbdt", after=""), "ghbd", "tn"), ("мир ", ""))
        self.assertEqual(located(replace(field, before="мир ghbd", after="t"), "ghbd", "t"), ("мир ", ""))
        self.assertIsNone(located(replace(field, before="мир ghbdx"), "ghbd", "t"))

    def test_the_early_switch_happens_while_the_field_is_a_letter_behind(self) -> None:
        with sequences.session(0) as current:
            current.settings.set("detection.early_switch", True)
            current.settings.set("detection.context_read_field", True)
            current.settings.set("diagnostics.technical_logging", True)
            reader = BehindReader(lambda: current.backend.text)
            current.engine.context_policy.reader = reader
            with self.assertLogs("keyswitch.engine", level=logging.INFO) as logs:
                for character in "ghbdtn ":
                    key = current.key(character)
                    # The field takes the key in only after the engine has read it.
                    reader.behind = 1
                    current.send(key)
                    reader.behind = 0
                    current.send(replace(key, pressed=False))
                current.flush()
            events = technical_events(logs.output)
            self.assertEqual(current.backend.text, "привет ")
            self.assertEqual([event["mode"] for event in events if event["event"] == "correction_applied"], ["early"])
            self.assertFalse([event for event in events if event["event"] == "early_switch_evaluation"
                              and "context_field_changed" in str(event["decision"])])


if __name__ == "__main__":
    unittest.main()
