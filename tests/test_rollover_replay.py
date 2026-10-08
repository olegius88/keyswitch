"""Letters typed before the space comes up go with the correction and come back as the next word.

A fast typist presses the next letter before the space comes up. The correction of the word before
it deletes that letter with the word and types it again in the new layout, and it comes back through
the hook as the start of the next word (WindowsBackend.inject_correction). The engine still held the
old layout for that word: the letter coming back read as a layout change in the middle of a word and
cleared the observed text, and the pair it starts was no longer decided as when typed after the space.
"""

from __future__ import annotations

import logging
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path

from keyswitch.backend import KeyEvent
from keyswitch.config import SettingsStore
from keyswitch.engine import KeySwitchEngine
from keyswitch.history import HistoryStore
from keyswitch.input_context import InputContext
from keyswitch.constants.windows import VK_SPACE
from keyswitch.windows_backend import NativeKeyEvent, WindowsBackend
from fixture_values.platform import ENGLISH_HKL, RUSSIAN_HKL
from test_action_boundary import SCANS, ActionEditorAPI
from test_reopened_word import technical_events


class RolloverReplayTests(unittest.TestCase):
    def typed(self, keys: str, rolled: bool) -> tuple[str, list[dict[str, object]]]:
        """Type the keys from the Russian layout, the letter after each space down before it comes up."""

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            settings = SettingsStore(root / "config.json")
            settings.set("detection.early_switch", False)
            settings.set("detection.respect_manual_layout", False)
            settings.set("detection.context_read_field", False)
            settings.set("diagnostics.technical_logging", True)
            api = ActionEditorAPI()
            api.current_layout = RUSSIAN_HKL
            backend = WindowsBackend(api)
            api.backend = backend
            engine = KeySwitchEngine(settings, HistoryStore(root / "history.jsonl"), backend)
            backend.set_key_filter(engine.consumes_key)
            backend._listener = engine.enqueue

            def flush() -> None:
                while not engine._events.empty():
                    event = engine._events.get_nowait()
                    assert event is not None
                    engine._handle(event)  # type: ignore[arg-type]

            def key(char: str) -> NativeKeyEvent:
                return NativeKeyEvent(True, ord(char.upper()) if char != " " else VK_SPACE, SCANS[char], False, False, 1)

            with self.assertLogs("keyswitch.engine", level=logging.INFO) as logs:
                held: NativeKeyEvent | None = None
                for char in keys:
                    event = key(char)
                    api.physical(event)
                    flush()
                    if held is not None:
                        api.physical(replace(held, pressed=False))
                        flush()
                        held = None
                    if rolled and char == " ":
                        held = event
                    else:
                        api.physical(replace(event, pressed=False))
                        flush()
                if held is not None:
                    api.physical(replace(held, pressed=False))
                    flush()
            self.assertIn(api.current_layout, (ENGLISH_HKL, RUSSIAN_HKL))
            return api.text, technical_events(logs.output)

    def test_a_pair_begun_before_the_space_came_up_is_decided_as_when_typed_after_it(self) -> None:
        # `тфеы` is `nats` typed in the Russian layout; the `b` key after it is `и` there and `b` after the switch.
        keys = "nats b redis "
        plain, _events = self.typed(keys, rolled=False)
        rolled, events = self.typed(keys, rolled=True)
        self.assertEqual(rolled, plain)
        self.assertTrue(rolled.startswith("nats "))
        self.assertIn(1, [event.get("late_keys") for event in events if event["event"] == "correction_applied"])
        self.assertNotIn("layout_changed_mid_word", [event.get("reason") for event in events])


    def test_the_observed_text_gives_back_only_what_it_ends_with(self) -> None:
        stream = InputContext()
        for char in "nats b":
            stream.observe(KeyEvent(True, 0, char, char, (char, char), 0, 0, 0))
        self.assertFalse(stream.withdraw("x"))
        self.assertEqual(stream.text, "nats b")
        self.assertTrue(stream.withdraw("b"))
        self.assertEqual(stream.text, "nats ")


if __name__ == "__main__":
    unittest.main()
