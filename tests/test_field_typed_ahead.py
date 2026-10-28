"""A field read at a word's boundary may already show the keys typed after it, and that is no change.

The hook lets a key through to the window as it queues it for the engine, and the engine decides on a
word a moment later, after a field read and the model. Typed fast, the window shows the boundary and
the next key or two by then, and the decision refused the word as typed into a changed field: `e 'njuj `
typed at 40 ms a key into a GTK window of KDE Plasma kept its `e` in five of ten runs (27.10.2026).
"""

from __future__ import annotations

import logging
import unittest
from dataclasses import replace

from keyswitch.context_policy import shown_ahead
from keyswitch.input_context import FieldContext
import test_input_sequence_matrix as sequences
from test_reopened_word import technical_events


class FieldReader:
    """The window's text as the fake backend holds it."""

    def __init__(self, session: sequences.PhysicalSession) -> None:
        self.session = session

    def read(self, application: str, window: int) -> FieldContext:
        return FieldContext(application, "1", self.session.backend.text, "", source="atspi")


class ShownAheadTests(unittest.TestCase):
    def test_the_text_after_a_word_counts_only_when_it_is_what_was_typed_after_it(self) -> None:
        self.assertEqual(shown_ahead("мир e '", "e", " 'n"), len(" '"))
        self.assertEqual(shown_ahead("мир e 'n", "e", " 'n"), len(" 'n"))
        # One character after the word is the boundary, which the caller accepts on its own.
        self.assertEqual(shown_ahead("мир e ", "e", " 'n"), 0)
        # Other text after the word, or the word not there: a changed field.
        self.assertEqual(shown_ahead("мир e xy", "e", " 'n"), 0)
        self.assertEqual(shown_ahead("мир x '", "e", " 'n"), 0)
        self.assertEqual(shown_ahead("мир e '", "e", ""), 0)


class TypedAheadDecisionTests(unittest.TestCase):
    def test_a_word_decided_after_the_window_took_the_next_keys_is_still_decided(self) -> None:
        with sequences.session(0) as current:
            current.settings.set("detection.context_read_field", True)
            current.settings.set("diagnostics.technical_logging", True)
            current.engine.context_policy.reader = FieldReader(current)
            with self.assertLogs("keyswitch.engine", level=logging.INFO) as logs:
                current.physical("e")
                space, quote = current.key(" "), current.key("'")
                # The window has the space and the next key; the engine has the next key queued, and
                # handles the space only now.
                current.backend.type(space)
                current.backend.type(quote)
                current.engine._events.put_nowait(quote)
                current.engine._handle(space)
                current.engine._handle(replace(space, pressed=False))
                current.flush()
                current.engine._handle(replace(quote, pressed=False))
                current.physical("njuj ")
                current.flush()
            events = technical_events(logs.output)
            self.assertFalse([event for event in events if event["event"] == "field_contradiction"])
            self.assertEqual(current.backend.text, "у этого ")


class TypedAfterTests(unittest.TestCase):
    def test_the_word_in_progress_is_typed_after_a_word_decided_later_but_not_after_itself(self) -> None:
        with sequences.session(0) as current:
            current.physical("ok gh")
            current.engine._events.put_nowait(current.key("b"))
            # The word in progress is `gh`: deciding it, only the queued key comes after it.
            self.assertEqual(current.engine._typed_after("gh"), "b")
            # Deciding `ok` only now, the letters of `gh` were typed after it as well.
            self.assertEqual(current.engine._typed_after("ok"), "ghb")


if __name__ == "__main__":
    unittest.main()
