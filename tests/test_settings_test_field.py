"""The test field of the settings window is typed in as any other program is.

The windows of KeySwitch itself are no place the user types in: on Windows each window
keeps a layout of its own, so a layout seen while the settings window or the learning
prompt is in front says nothing of the text and is ignored. The test field is there to try
KeySwitch out, and a layout picked by hand while it has the keyboard must protect the next
word there as it would in an editor. The window reports when the field gains and loses the
keyboard; the engine is driven here as the Windows backend reports a KeySwitch window.
"""

from __future__ import annotations

import json
import unittest
from collections.abc import Iterator
from contextlib import contextmanager

from fixture_values.keys import OWN_WINDOW_ID
from engine_models import share_engine_models
import test_input_sequence_matrix as sequences


def setUpModule() -> None:
    share_engine_models()


def technical_events(lines: list[str]) -> list[dict[str, object]]:
    marker = "TECHNICAL "
    return [json.loads(line.split(marker, 1)[1]) for line in lines if marker in line]


@contextmanager
def settings_window() -> Iterator[sequences.PhysicalSession]:
    """An editor in English first, then the settings window of KeySwitch in front with its own layout."""

    with sequences.session(0) as current:
        current.settings.set("detection.context_policy", "off")
        current.settings.set("detection.respect_manual_layout", True)
        current.settings.set("diagnostics.technical_logging", True)
        current.engine._poll_current_group()
        current.backend.own_window = True
        current.backend.isolated_layout = True
        current.backend.window = OWN_WINDOW_ID
        yield current


def type_word(current: sequences.PhysicalSession, keys: str) -> None:
    current.physical(keys)
    current.flush()


class SettingsTestFieldTests(unittest.TestCase):
    def test_a_layout_picked_in_the_test_field_protects_the_next_word(self) -> None:
        with settings_window() as current:
            with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                current.engine.set_test_field_focused(True)
                # The field takes the window's own layout with it: that is no choice of the user.
                current.backend.group = 1
                current.engine._poll_current_group()
            events = technical_events(logs.output)
            self.assertEqual(events[0]["event"], "test_field_focused")
            arrived = next(event for event in events if event["event"] == "layout_change_observed")
            self.assertTrue(arrived["focus_changed"])
            self.assertIsNone(current.engine._manual_layout_group)
            self.assertEqual(current.engine._focus_window, OWN_WINDOW_ID)
            self.assertEqual(current.engine.snapshot.current_group, 1)
            # The user picks English by hand in the field: the next word stays as typed, the one after is
            # corrected again.
            current.backend.group = 0
            with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                current.engine._poll_current_group()
            picked = technical_events(logs.output)[0]
            self.assertEqual((picked["event"], picked["protects_next_word"]), ("manual_layout_observed", True))
            self.assertEqual(current.engine._manual_layout_group, 0)
            type_word(current, "ghbdtn ")
            self.assertEqual(current.backend.text, "ghbdtn ")
            type_word(current, "ghbdtn ")
            self.assertEqual(current.backend.text, "ghbdtn привет ")

    def test_the_rest_of_the_window_and_the_prompt_are_still_ignored(self) -> None:
        with settings_window() as current:
            current.backend.group = 1
            with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                current.engine._poll_current_group()
            ignored = technical_events(logs.output)[0]
            self.assertEqual((ignored["event"], ignored["reason"]), ("layout_change_ignored", "own_window"))
            self.assertIsNone(current.engine._manual_layout_group)
            self.assertEqual(current.engine.snapshot.current_group, 0)
            # Moving to the settings window is no move to another program.
            self.assertEqual(current.engine._focus_window, 1)
            # The field had the keyboard and lost it again: a layout picked now is ignored as before.
            current.engine.set_test_field_focused(True)
            with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                current.engine.set_test_field_focused(False)
                current.engine.set_test_field_focused(False)
            self.assertEqual([event["event"] for event in technical_events(logs.output)], ["test_field_left"])
            current.backend.group = 0
            current.engine._poll_current_group()
            current.backend.group = 1
            current.engine._poll_current_group()
            self.assertIsNone(current.engine._manual_layout_group)


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    return loader.loadTestsFromTestCase(SettingsTestFieldTests)


if __name__ == "__main__":
    unittest.main()
