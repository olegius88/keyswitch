"""Letters typed ahead of a correction stand in the field the correction checks, and that is no change.

A correction runs once the key that triggered it is up. A user who presses the next letter before
the space comes up (rollover typing) has that letter on screen by then, and the hook passes every
key to the window as it queues it. The late input of the correction deletes such letters with the
word and types them again; the check of the field before it refused them as a changed field. With
the field reader on (the Windows default), `ghbdtn vbh` typed that way stayed as typed, its next
word with it: in VS Code `l` stayed for `в` and `gj` for `по` (field logs, 08.10.2026).
"""

from __future__ import annotations

import unittest
from dataclasses import replace

from keyswitch.backend import KeyEvent
from keyswitch.constants.keyboard import CONTROL_MASK
from keyswitch.engine import CorrectionPlan, KeySwitchEngine
from keyswitch.input_context import FieldContext
import test_input_sequence_matrix as sequences
from test_field_caret_lag import LaggingReader
from test_reopened_word import ConvertingModel, technical_events


def rolled(current: sequences.PhysicalSession, words: list[str], *, queued: bool = False) -> None:
    """Type the words, each next one begun before the space ahead of it comes up.

    `queued` leaves that first letter in the engine's queue when the space comes up, as when
    the worker has not yet taken it.
    """

    for index, word in enumerate(words):
        for character in word[1:] if index else word:
            current.tap(current.key(character))
        if index == len(words) - 1:
            break
        space = current.key(" ")
        current.send(space)
        letter = current.key(words[index + 1][0])
        if queued:
            current.backend.type(letter)
            current.engine.enqueue(letter)
        else:
            current.send(letter)
        current.send(replace(space, pressed=False))
        current.flush()
        current.send(replace(letter, pressed=False))
    current.tap(current.key(" "))
    current.flush()


class TypedAheadTests(unittest.TestCase):
    def test_a_phrase_typed_with_rollover_is_corrected_with_the_field_reader_on(self) -> None:
        for words, expected in ((["ghbdtn", "vbh"], "привет мир "), (["z", "ctujlyz"], "я сегодня "),
                                (["rfr", "ltkf"], "как дела ")):
            for queued in (False, True):
                for lag in (0, 1):
                    with self.subTest(words=words, queued=queued, lag=lag), sequences.session(0) as current:
                        current.settings.set("detection.context_read_field", True)
                        current.settings.set("diagnostics.technical_logging", True)
                        current.engine.context_policy.reader = LaggingReader(lambda: current.backend.text, lag)
                        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                            rolled(current, list(words), queued=queued)
                        self.assertEqual(current.backend.text, expected)
                        self.assertFalse([event for event in technical_events(logs.output)
                                          if event["event"] == "correction_aborted"])

    def test_a_field_that_changed_is_still_refused_and_the_log_names_the_check(self) -> None:
        with sequences.session(0) as current:
            current.settings.set("detection.context_read_field", True)
            current.settings.set("diagnostics.technical_logging", True)
            current.engine.context_policy.model = ConvertingModel()
            rewritten: list[str] = []
            # The decision reads the word; by the correction another program has rewritten the line.
            current.engine.context_policy.reader = LaggingReader(
                lambda: rewritten[0] if rewritten else current.backend.text, 0)
            with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                current.physical("ghbdtn")
                space = current.key(" ")
                current.send(space)
                letter = current.key("v")
                current.send(letter)
                rewritten.append("ghbdtn, v")
                current.send(replace(space, pressed=False))
                current.send(replace(letter, pressed=False))
            [aborted] = [event for event in technical_events(logs.output) if event["event"] == "correction_aborted"]
            self.assertEqual((aborted["reason"], aborted["field_check"], aborted["typed_ahead"]),
                             ("context_field_changed", "other_text", len("v")))
            self.assertEqual(current.backend.text, "ghbdtn v")

    def test_typed_ahead_is_the_plain_text_after_the_word_up_to_the_first_other_key(self) -> None:
        with sequences.session(0) as current:
            engine = current.engine
            current.physical("ghbdtn")
            word = tuple(engine._strokes)
            plan = CorrectionPlan(word, None, 0, 1, "ghbdtn", "привет", 1.0, "", True, "early")
            self.assertEqual(engine._typed_ahead(plan), "")
            shift = current.key("", name="Shift_L")
            for event in (current.key("V"), replace(current.key("V"), pressed=False), shift, current.key("b"),
                          current.key("", name="Left"), current.key("h")):
                engine.enqueue(event)
            # The Shift typed nothing, the key-up neither; the caret move ends what can be told.
            self.assertEqual(engine._typed_ahead(plan), "Vb")
            current.engine._events.queue.clear()
            engine.enqueue(replace(current.key("c"), state=CONTROL_MASK))
            self.assertEqual(engine._typed_ahead(plan), "")
            current.engine._events.queue.clear()
            engine._events.put_nowait(None)
            self.assertEqual(engine._typed_ahead(plan), "")

    def test_the_refusal_names_what_differs(self) -> None:
        stroke = KeyEvent(True, 1, "g", "g", ("g", "п"), 0, 0, 1)
        plan = CorrectionPlan((stroke,), None, 0, 1, "g", "п", 1.0, "editor", True, "early", context_field="1")
        refusal = KeySwitchEngine._field_refusal
        field = FieldContext("editor", "1", "мир g", "", source="uia")
        self.assertEqual(refusal(plan, None, "g", ""), "unread")
        self.assertEqual(refusal(plan, replace(field, field_id="2"), "g", ""), "other_field")
        self.assertEqual(refusal(plan, replace(field, application="other"), "g", ""), "other_field")
        self.assertEqual(refusal(plan, replace(field, sensitive=True), "g", ""), "sensitive_or_selected")
        self.assertEqual(refusal(plan, replace(field, selection=True), "g", ""), "sensitive_or_selected")
        self.assertEqual(refusal(plan, field, "g", ""), "")
        # As much of the text typed ahead as the window has taken in, the caret where it reports it.
        for before, after in (("мир g", ""), ("мир g ", ""), ("мир g d", ""), ("мир g ", "d")):
            with self.subTest(before=before, after=after):
                self.assertEqual(refusal(plan, replace(field, before=before, after=after), "g", " d"), "")
        self.assertEqual(refusal(plan, replace(field, before="мир g dx"), "g", " d"), "other_text")
        self.assertEqual(refusal(plan, replace(field, before="мир g d"), "g", ""), "other_text")


if __name__ == "__main__":
    unittest.main()
