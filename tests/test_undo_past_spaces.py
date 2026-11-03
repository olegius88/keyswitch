"""The undo hotkey reaches the last correction past the spaces typed after it.

A wrong correction is noticed once it is seen, and by then a space or more often stands after it:
the hotkey answered that nothing could be undone. It now puts the word back as it was typed and keeps
the spaces; any other key after the correction still leaves nothing to undo. `_schedule_undo` itself
keeps its old answer, which a frozen historical test pins (tests/test_input_integrity.py,
test_undo_is_unavailable_after_edit_or_pointer_move); the hotkey goes through
KeySwitchEngine._schedule_undo_hotkey. The sessions type physical keys with the context policy off.
"""

from __future__ import annotations

import json
import time
import unittest
from collections.abc import Iterator
from contextlib import contextmanager
from dataclasses import replace

from keyswitch.constants.detection import PAUSE_TRAILING_MAX_KEYS
from keyswitch.constants.keyboard import ALT_MASK, CONTROL_MASK
from keyswitch.constants.timing import UNDO_AVAILABLE_WINDOW_SECONDS
from fixture_values.clock import PAUSE_TRIGGER_OFFSET_SECONDS
from fixture_values.keys import UNDO_TRIGGER_SENTINEL_KEYCODE
from engine_models import share_engine_models
from test_engine_behaviour import plain_key
import test_input_sequence_matrix as sequences


def setUpModule() -> None:
    share_engine_models()


@contextmanager
def session(group: int) -> Iterator[sequences.PhysicalSession]:
    with sequences.session(group) as current:
        current.settings.set("detection.context_policy", "off")
        current.settings.set("diagnostics.technical_logging", True)
        yield current


def technical_events(lines: list[str]) -> list[dict[str, object]]:
    marker = "TECHNICAL "
    return [json.loads(line.split(marker, 1)[1]) for line in lines if marker in line]


def type_keys(current: sequences.PhysicalSession, keys: str) -> None:
    current.physical(keys)
    current.flush()


def undo_hotkey(current: sequences.PhysicalSession) -> None:
    """Ctrl+Alt+Z as the hook reports it: KeySwitch's own hotkey, which the editor never receives."""

    key = replace(current.key("z"), character="", characters=("", ""), state=CONTROL_MASK | ALT_MASK)
    current.engine._handle(key)
    current.engine._handle(replace(key, pressed=False, state=0))
    current.flush()


class UndoPastSpacesTests(unittest.TestCase):
    def test_the_word_comes_back_as_typed_with_the_spaces_after_it(self) -> None:
        cases = (
            (0, "ghbdtn ", " ", "привет  ", "ghbdtn  "),
            (0, "ghbdtn ", "   ", "привет    ", "ghbdtn    "),
            (1, "hello ", " ", "hello  ", "руддщ  "),
        )
        for group, word, spaces, corrected, restored in cases:
            with self.subTest(word=word, spaces=len(spaces)), session(group) as current:
                type_keys(current, word)
                type_keys(current, spaces)
                self.assertEqual(current.backend.text, corrected)
                with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                    undo_hotkey(current)
                self.assertEqual(current.backend.text, restored)
                self.assertEqual(current.backend.caret, len(restored))
                self.assertEqual(current.backend.group, group)
                reached = next(event for event in technical_events(logs.output) if event["event"] == "undo_past_spaces")
                self.assertEqual(reached["spaces"], len(spaces))
                applied = next(event for event in technical_events(logs.output) if event["event"] == "correction_applied")
                self.assertEqual(applied["mode"], "undo")

    def test_a_second_undo_puts_the_correction_back_and_the_chain_goes_on(self) -> None:
        with session(0) as current:
            type_keys(current, "ghbdtn  ")
            automatic = current.engine._last_correction
            undo_hotkey(current)
            self.assertEqual(current.backend.text, "ghbdtn  ")
            # A double press of Pause reasons about the engine's own correction, as after any undo.
            self.assertIs(current.engine._correction_origin, automatic)
            undo_hotkey(current)
            self.assertEqual(current.backend.text, "привет  ")

    def test_any_other_key_after_the_correction_leaves_nothing_to_undo(self) -> None:
        for after in ("x", " x", " BackSpace", " Tab"):
            with self.subTest(after=after), session(0) as current:
                type_keys(current, "ghbdtn ")
                for part in after.split(" "):
                    if part in {"BackSpace", "Tab"}:
                        current.command(part)
                    elif part == "":
                        current.physical(" ")
                    else:
                        current.physical(part)
                current.flush()
                visible = current.backend.text
                injections = len(current.backend.injections)
                with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                    undo_hotkey(current)
                self.assertEqual(current.backend.text, visible)
                self.assertEqual(len(current.backend.injections), injections)
                self.assertIn("undo_unavailable", [event["event"] for event in technical_events(logs.output)])

    def test_the_reach_ends_where_pause_reaches_no_further(self) -> None:
        with session(0) as current:
            type_keys(current, "ghbdtn" + " " * PAUSE_TRAILING_MAX_KEYS)
            undo_hotkey(current)
            self.assertEqual(current.backend.text, "ghbdtn" + " " * PAUSE_TRAILING_MAX_KEYS)
        beyond = "ghbdtn" + " " * (PAUSE_TRAILING_MAX_KEYS + 1)
        with session(0) as current:
            type_keys(current, beyond)
            undo_hotkey(current)
            self.assertEqual(current.backend.text, "привет" + " " * (PAUSE_TRAILING_MAX_KEYS + 1))

    def test_undo_expires_past_spaces_as_it_does_without_them(self) -> None:
        with session(0) as current:
            type_keys(current, "ghbdtn  ")
            current.engine._last_correction_time = time.monotonic() - UNDO_AVAILABLE_WINDOW_SECONDS - 1
            undo_hotkey(current)
            self.assertEqual(current.backend.text, "привет  ")
            self.assertEqual(current.engine.snapshot.last_action, "Последнее исправление уже нельзя отменить")

    def test_the_case_fix_comes_back_as_typed_past_a_space(self) -> None:
        with session(1) as current:
            type_keys(current, "GHbdtn  ")
            self.assertEqual(current.backend.text, "Привет  ")
            undo_hotkey(current)
            self.assertEqual(current.backend.text, "ПРивет  ")

    def test_a_word_converted_at_a_pause_comes_back_past_the_space_that_ended_it(self) -> None:
        with session(0) as current:
            type_keys(current, "ghbdtn")
            last_input = current.engine._last_word_input_at
            assert last_input is not None
            current.engine._maybe_correct_after_pause(now=last_input + PAUSE_TRIGGER_OFFSET_SECONDS)
            current.flush()
            self.assertEqual(current.backend.text, "привет")
            type_keys(current, " ")
            undo_hotkey(current)
            self.assertEqual(current.backend.text, "ghbdtn ")

    def test_right_after_the_correction_the_hotkey_undoes_as_before(self) -> None:
        with session(0) as current:
            type_keys(current, "ghbdtn ")
            with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                undo_hotkey(current)
            self.assertEqual(current.backend.text, "ghbdtn ")
            self.assertNotIn("undo_past_spaces", [event["event"] for event in technical_events(logs.output)])

    def test_the_old_method_keeps_its_answer_for_its_callers(self) -> None:
        # The frozen test calls `_schedule_undo` directly: a space after the correction leaves nothing to undo there.
        with session(0) as current:
            type_keys(current, "ghbdtn  ")
            current.engine._schedule_undo(UNDO_TRIGGER_SENTINEL_KEYCODE)
            current.send(plain_key("z", UNDO_TRIGGER_SENTINEL_KEYCODE, current.backend.group, False))
            current.flush()
            self.assertEqual(current.backend.text, "привет  ")


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    return loader.loadTestsFromTestCase(UndoPastSpacesTests)


if __name__ == "__main__":
    unittest.main()
