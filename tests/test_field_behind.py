"""A field read as a key goes down may not show that key yet, and that is no change.

The early switch reads the field when the fourth letter of a word goes down. A browser or an
Electron editor reports its text a moment after the key, and the read was refused as a changed
field: at the fourth letter in 175 of 210 words in Firefox, 174 of 416 in the Claude app and 532
of 2 275 in VS Code (Windows logs, 05.09-08.10.2026). The early switch hardly ever worked there.
"""

from __future__ import annotations

import logging
import unittest
from collections.abc import Callable, Iterator
from contextlib import contextmanager
from dataclasses import replace
from unittest.mock import patch

from keyswitch.constants.settings_defaults import DEFAULT_FIELD_CATCH_UP, FIELD_CATCH_UP_OFF
from keyswitch.constants.text import FIELD_PENDING_MAX_CHARACTERS
from keyswitch.constants.timing import FIELD_CATCH_UP_RETRY_SECONDS
from keyswitch.constants.units import MILLISECONDS_PER_SECOND
from keyswitch.engine import KeySwitchEngine
from keyswitch.input_context import FieldContext
from fixture_values.clock import LAST_WORD_INPUT_AT_SECONDS
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

    def test_a_field_still_behind_when_the_switch_would_run_leaves_the_word_to_its_boundary(self) -> None:
        with sequences.session(0) as current:
            current.settings.set("detection.early_switch", True)
            current.settings.set("detection.context_read_field", True)
            current.settings.set("diagnostics.technical_logging", True)
            reader = BehindReader(lambda: current.backend.text)
            current.engine.context_policy.reader = reader
            with self.assertLogs("keyswitch.engine", level=logging.INFO) as logs:
                # A slow editor: a letter behind whenever it is read, until the space.
                reader.behind = 1
                current.physical("ghbdtn")
                reader.behind = 0
                current.physical(" ")
                current.flush()
            events = technical_events(logs.output)
            self.assertEqual(current.backend.text, "привет ")
            self.assertEqual([event["mode"] for event in events if event["event"] == "correction_applied"], ["boundary"])
            self.assertIn("field_behind", [event["reason"] for event in events if event["event"] == "early_switch_dropped"])
            self.assertFalse([event for event in events if event["event"] == "correction_aborted"])


class BoundaryFieldBehindTests(unittest.TestCase):
    """At its space a word is decided on its keys while the field is short of its last letters, and the correction
    waits for the field to show it (detection.field_catch_up): `ашкуащч` stayed for `firefox` in Antigravity, refused as
    a changed field (the owner's typing, 19.10.2026)."""

    @contextmanager
    def behind(self, catch_up: str = DEFAULT_FIELD_CATCH_UP) -> Iterator[tuple[sequences.PhysicalSession, BehindReader,
                                                                                list[float]]]:
        clock = [LAST_WORD_INPUT_AT_SECONDS]
        with sequences.session(0) as current, patch("keyswitch.engine.time.monotonic", side_effect=lambda: clock[0]):
            current.settings.set("detection.context_read_field", True)
            current.settings.set("diagnostics.technical_logging", True)
            current.settings.set("detection.field_catch_up", catch_up)
            reader = BehindReader(lambda: current.backend.text)
            current.engine.context_policy.reader = reader
            # The last letter and the space are not shown yet when the word is decided and when its correction runs.
            reader.behind = len("n ")
            yield current, reader, clock

    def tick(self, current: sequences.PhysicalSession, clock: list[float], seconds: float) -> None:
        clock[0] += seconds
        current.engine._run_timers()
        current.flush()

    def test_a_word_converts_once_the_field_shows_it(self) -> None:
        with self.behind() as (current, reader, clock), self.assertLogs("keyswitch.engine", level=logging.INFO) as logs:
            current.physical("ghbdtn ")
            current.flush()
            self.assertEqual(current.backend.text, "ghbdtn ")
            self.assertEqual(current.engine._loop_timeout(), FIELD_CATCH_UP_RETRY_SECONDS)
            self.tick(current, clock, FIELD_CATCH_UP_RETRY_SECONDS)
            self.assertEqual(current.backend.text, "ghbdtn ")
            reader.behind = 0
            self.tick(current, clock, FIELD_CATCH_UP_RETRY_SECONDS)
            self.assertEqual(current.backend.text, "привет ")
        events = technical_events(logs.output)
        self.assertEqual([event["event"] for event in events if event["event"] in (
            "field_contradiction", "correction_awaiting_field", "correction_applied", "correction_aborted")],
            ["correction_awaiting_field", "correction_applied"])
        self.assertEqual([event["fallback_reason"] for event in events if event["event"] == "context_decision"], ["field_behind"])

    def test_a_value_the_setting_does_not_offer_is_its_default(self) -> None:
        with self.behind("250") as (current, _reader, _clock):
            self.assertEqual(current.engine._field_catch_up_seconds(),
                             int(DEFAULT_FIELD_CATCH_UP) / MILLISECONDS_PER_SECOND)

    def test_with_the_setting_off_the_correction_is_given_up_as_before(self) -> None:
        with self.behind(FIELD_CATCH_UP_OFF) as (current, _reader, _clock), \
                self.assertLogs("keyswitch.engine", level=logging.INFO) as logs:
            current.physical("ghbdtn ")
            current.flush()
        self.assertEqual(current.backend.text, "ghbdtn ")
        self.assertIn("correction_aborted", [event["event"] for event in technical_events(logs.output)])

    def test_a_field_that_never_shows_the_word_is_given_up_at_the_deadline(self) -> None:
        deadline = int(DEFAULT_FIELD_CATCH_UP) / MILLISECONDS_PER_SECOND
        with self.behind() as (current, _reader, clock), self.assertLogs("keyswitch.engine", level=logging.INFO) as logs:
            current.physical("ghbdtn ")
            current.flush()
            self.tick(current, clock, deadline + FIELD_CATCH_UP_RETRY_SECONDS)
            self.assertIsNone(current.engine._field_wait)
        self.assertEqual(current.backend.text, "ghbdtn ")
        names = [event["event"] for event in technical_events(logs.output)]
        self.assertIn("field_wait_expired", names)
        self.assertIn("correction_aborted", names)

    def test_a_click_while_waiting_drops_the_correction(self) -> None:
        with self.behind() as (current, reader, clock), self.assertLogs("keyswitch.engine", level=logging.INFO) as logs:
            current.physical("ghbdtn ")
            current.flush()
            current.command("Pointer")
            reader.behind = 0
            self.tick(current, clock, FIELD_CATCH_UP_RETRY_SECONDS)
        self.assertEqual(current.backend.text, "ghbdtn ")
        self.assertIn("field_wait_dropped", [event["event"] for event in technical_events(logs.output)])

    def test_a_correction_planned_meanwhile_supersedes_the_waiting_one(self) -> None:
        with self.behind() as (current, _reader, _clock):
            current.physical("ghbdtn ")
            current.flush()
            waiting = current.engine._field_wait
            assert waiting is not None
            current.engine._pending = waiting.plan
            current.engine._retry_field_wait()
            self.assertIsNone(current.engine._field_wait)
            current.engine._pending = None
        self.assertEqual(current.backend.text, "ghbdtn ")


if __name__ == "__main__":
    unittest.main()
