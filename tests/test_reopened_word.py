"""Letters added to a finished word belong to it, not to a word of their own.

Two ways a token used to fall apart, both taken from collected logs: a
Backspace over the space after "создаш" followed by "ь" was judged as the lone
letter "ь", and the "в" typed while a correction was being abandoned left "се"
to be judged alone. Neither fragment should ever reach the models.
"""

from __future__ import annotations

import json
import time
import unittest
from collections.abc import Callable
from dataclasses import replace

from keyswitch.context_model import ACTIONS, ContextEvidence, ContextModel, ContextPrediction
from keyswitch.input_context import FieldContext
import test_input_sequence_matrix as sequences
from fixture_values.clock import PAUSE_ALWAYS_ELAPSED_SECONDS
from fixture_values.counts import STOL_REOPENED_WORD_CHARACTERS
from keyswitch.constants.models import CONTEXT_ACTION_FEATURE_VERSION


def technical_events(lines: list[str]) -> list[dict[str, object]]:
    events: list[dict[str, object]] = []
    for line in lines:
        marker = "TECHNICAL "
        if marker in line:
            payload = json.loads(line.split(marker, 1)[1])
            assert isinstance(payload, dict)
            events.append(payload)
    return events


def evaluated_words(events: list[dict[str, object]]) -> list[object]:
    return [event["original"] for event in events if event["event"] == "word_evaluation"]


class ConvertingModel(ContextModel):
    """A context model that converts every finished word it is shown."""

    def __init__(self) -> None:
        super().__init__({}, "context-v3-convert", feature_version=CONTEXT_ACTION_FEATURE_VERSION)

    def predict(self, item: ContextEvidence) -> ContextPrediction:
        scores = tuple(float(name == "convert") for name in ACTIONS)
        return ContextPrediction("convert", 1.0, scores, self.version, True)


class MovingReader:
    """A field reader whose field can change identity between two reads."""

    def __init__(self, text: Callable[[], str]) -> None:
        self.field_id = "1"
        self.text = text

    def read(self, application: str, window: int) -> FieldContext:
        return FieldContext(application, self.field_id, self.text(), "", source="native")


class ReopenedWordTests(unittest.TestCase):
    def test_letters_typed_after_deleting_the_space_continue_the_word(self) -> None:
        with sequences.session(1) as current:
            current.settings.set("detection.context_policy", "off")
            current.settings.set("diagnostics.technical_logging", True)
            with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                current.physical("cnjk ")
                current.command("BackSpace")
                current.physical("s?")
            self.assertEqual(current.backend.text, "столы,")
            self.assertEqual(current.backend.injections, [])
            events = technical_events(logs.output)
            self.assertEqual(evaluated_words(events), ["стол", "столы"])
            reopened = next(event for event in events if event["event"] == "committed_word_reopened")
            self.assertEqual(reopened["characters"], STOL_REOPENED_WORD_CHARACTERS)

    def test_reopening_alone_asks_for_no_second_judgement(self) -> None:
        with sequences.session(1) as current:
            current.settings.set("detection.context_policy", "off")
            current.settings.set("diagnostics.technical_logging", True)
            current.physical("cnjk ")
            with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                current.command("BackSpace")
                # The user stops to think: the word judged at its space is
                # not judged again just because the space went away.
                current.engine._maybe_correct_after_pause(now=time.monotonic() + PAUSE_ALWAYS_ELAPSED_SECONDS)
                current.physical("s")
                current.engine._maybe_correct_after_pause(now=time.monotonic() + PAUSE_ALWAYS_ELAPSED_SECONDS)
            judged = [
                (event["original"], event["trigger"])
                for event in technical_events(logs.output)
                if event["event"] == "word_evaluation"
            ]
            self.assertEqual(judged, [("столы", "pause")])
            self.assertEqual(current.backend.text, "столы")

    def test_a_second_backspace_keeps_erasing_the_reopened_word(self) -> None:
        with sequences.session(1) as current:
            current.settings.set("detection.context_policy", "off")
            current.physical("cnjk ")
            current.command("BackSpace")
            current.command("BackSpace")
            current.physical("s ")
            self.assertEqual(current.backend.text, "стоы ")
            self.assertEqual(current.engine.snapshot.current_word, "")
            self.assertEqual(current.backend.injections, [])

    def test_letters_typed_before_an_abandoned_correction_are_not_judged_alone(self) -> None:
        with sequences.session(0) as current:
            current.settings.set("detection.context_read_field", True)
            current.settings.set("diagnostics.technical_logging", True)
            reader = MovingReader(lambda: current.backend.text)
            current.engine.context_policy.reader = reader
            current.engine.context_policy.model = ConvertingModel()
            with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                current.physical("ghbdtn")
                space = current.key(" ")
                current.send(space)
                # The next word starts before the space comes up, and the field
                # changes identity before the correction can run.
                rollover = current.key("d")
                current.send(rollover)
                reader.field_id = "2"
                current.send(replace(space, pressed=False))
                current.send(replace(rollover, pressed=False))
                current.physical("ct ")
            events = technical_events(logs.output)
            # What reached the screen and the models comes first: "ct" must
            # neither be judged nor rewritten.
            self.assertEqual(current.backend.text, "ghbdtn dct ")
            self.assertEqual(evaluated_words(events), ["ghbdtn"])
            aborted = next(event for event in events if event["event"] == "correction_aborted")
            self.assertEqual(aborted["reason"], "context_field_changed")
            self.assertTrue(aborted["letters_untracked"])


if __name__ == "__main__":
    unittest.main()
