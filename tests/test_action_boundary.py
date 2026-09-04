"""Windows action barriers: visible correction must precede chat submission."""

from __future__ import annotations

import queue
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path
from unittest.mock import patch

from keyswitch.backend import KeyEvent, LOCK_MASK, SHIFT_MASK
from keyswitch.config import SettingsStore
from keyswitch.engine import KeySwitchEngine, LearningPrompt
from keyswitch.history import HistoryStore
from keyswitch.layouts import LayoutPair
from keyswitch.windows_backend import (
    NativeInput, NativeKeyEvent, VK_BACK, VK_RETURN, VK_SHIFT, VK_TAB, WindowsBackend,
)
from test_windows_backend import ENGLISH_LAYOUT, FakeWindowsAPI


SCANS = dict(zip("qwertyuiopasdfghjklzxcvbnm", (
    16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    30, 31, 32, 33, 34, 35, 36, 37, 38, 44, 45, 46, 47, 48, 49, 50,
)))


class ActionEditorAPI(FakeWindowsAPI):
    def __init__(self) -> None:
        super().__init__()
        self.backend: WindowsBackend | None = None
        self.text = ""
        self.messages: list[str] = []
        self.fields: list[str] = []
        self.timeline: list[str] = []

    def translate_key(self, virtual_key: int, scan_code: int, state: int, layout: int) -> str:
        if not 65 <= virtual_key <= 90:
            return ""
        text = chr(virtual_key).lower()
        if layout != ENGLISH_LAYOUT:
            text = LayoutPair().translate(text, "us", "ru")
        return text.upper() if bool(state & SHIFT_MASK) ^ bool(state & LOCK_MASK) else text

    def physical(self, event: NativeKeyEvent) -> bool:
        assert self.backend is not None
        consumed = self.backend._handle_native(event)
        if not consumed and event.pressed:
            if event.virtual_key == VK_RETURN:
                self.messages.append(self.text)
                self.timeline.append("submit:" + self.text)
                self.text = ""
            elif event.virtual_key == VK_TAB:
                self.fields.append(self.text)
                self.timeline.append("tab:" + self.text)
                self.text = ""
            elif event.virtual_key == VK_BACK:
                self.text = self.text[:-1]
            else:
                self.text += self.translate_key(event.virtual_key, event.scan_code, self.backend._normalized_state(), self.current_layout)
        return consumed

    def send_inputs(self, inputs: tuple[NativeInput, ...]) -> int:
        self.sent.append(inputs)
        count = len(inputs) if self.send_count is None else self.send_count
        for item in inputs[:count]:
            virtual_key = item.virtual_key or next((ord(char.upper()) for char, scan in SCANS.items() if scan == item.scan_code), 0)
            self.physical(NativeKeyEvent(item.pressed, virtual_key, item.scan_code, item.extended, item.synthetic, 1, replayed=item.replayed))
        return count


class ActionBoundaryTests(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        self.settings = SettingsStore(root / "config.json")
        self.settings.set("detection.early_switch", False)
        self.settings.set("detection.respect_manual_layout", False)
        self.settings.set("diagnostics.technical_logging", True)
        self.api = ActionEditorAPI()
        self.backend = WindowsBackend(self.api)
        self.api.backend = self.backend
        self.engine = KeySwitchEngine(self.settings, HistoryStore(root / "history.jsonl"), self.backend)
        self.backend.set_key_filter(self.engine.consumes_key)
        self.backend._listener = self.engine.enqueue

    def flush(self) -> None:
        for _ in range(1000):
            try:
                event = self.engine._events.get_nowait()
            except queue.Empty:
                return
            assert isinstance(event, KeyEvent)
            self.engine._handle(event)
        self.fail("Input replay did not terminate")

    def key(self, virtual_key: int, scan: int, *, pressed: bool = True) -> NativeKeyEvent:
        return NativeKeyEvent(pressed, virtual_key, scan, False, False, 1)

    def tap(self, event: NativeKeyEvent) -> None:
        self.api.physical(event)
        self.api.physical(replace(event, pressed=False))

    def type(self, text: str) -> None:
        for char in text:
            self.tap(self.key(ord(char.upper()), SCANS[char]))

    def test_wrong_layout_word_is_corrected_then_submitted_once(self) -> None:
        self.type("ghbdtn")
        self.tap(self.key(VK_RETURN, 28))
        self.assertEqual(self.api.text, "ghbdtn")
        self.assertEqual(self.api.messages, [])
        self.flush()
        self.assertEqual(self.api.messages, ["привет"])
        self.assertEqual(self.api.text, "")
        self.assertFalse(self.backend._holding)
        self.assertTrue(self.engine._last_committed_stale)
        self.assertEqual(self.engine.snapshot.current_word, "")

    def test_tab_is_delivered_after_correction_without_deleting_a_boundary(self) -> None:
        self.type("ghbdtn")
        self.tap(self.key(VK_TAB, 15))
        self.flush()
        self.assertEqual(self.api.fields, ["привет"])
        self.assertEqual(sum(item.pressed and item.virtual_key == VK_BACK for batch in self.api.sent for item in batch), 6)

    def test_rapid_next_message_and_second_enter_stay_in_order(self) -> None:
        self.type("ghbdtn")
        self.tap(self.key(VK_RETURN, 28))
        self.type("hello")
        self.tap(self.key(VK_RETURN, 28))
        self.type("a")
        self.flush()
        self.assertEqual(self.api.messages, ["привет", "hello"])
        self.assertEqual(self.api.text, "a")
        self.assertEqual(self.backend._held, [])
        self.assertFalse(self.backend._holding)

    def test_repeat_and_rollover_wait_for_prior_key_up(self) -> None:
        self.type("ghbdt")
        letter = self.key(ord("N"), SCANS["n"])
        self.api.physical(letter)
        enter = self.key(VK_RETURN, 28)
        for _ in range(4):
            self.api.physical(enter)
        self.api.physical(replace(enter, pressed=False))
        self.flush()
        self.assertEqual(self.api.messages, [])
        self.api.physical(replace(letter, pressed=False))
        self.flush()
        self.assertEqual(self.api.messages, ["привет"])

    def test_letter_autorepeat_after_enter_retains_its_balancing_release(self) -> None:
        self.type("ghbdt")
        letter = self.key(ord("N"), SCANS["n"])
        enter = self.key(VK_RETURN, 28)
        self.api.physical(letter)
        self.api.physical(enter)
        self.api.physical(letter)
        self.api.physical(replace(enter, pressed=False))
        self.api.physical(replace(letter, pressed=False))
        self.flush()
        self.assertEqual(self.api.messages, ["привет"])
        self.assertEqual(self.api.text, "т")
        self.assertEqual(self.backend._pressed, set())
        self.assertEqual(self.engine._pressed, set())

    def test_known_empty_protected_and_disabled_inputs_still_submit(self) -> None:
        for word in ("", "hello", "qwerty"):
            with self.subTest(word=word):
                self.type(word)
                self.tap(self.key(VK_RETURN, 28))
                self.flush()
                self.assertEqual(self.api.messages[-1], word)
        self.settings.set("enabled", False)
        self.type("ghbdtn")
        self.tap(self.key(VK_RETURN, 28))
        self.assertEqual(self.api.messages[-1], "ghbdtn")
        self.assertFalse(self.backend._holding)

    def test_learning_confirmation_remains_separate_from_chat_submission(self) -> None:
        self.engine._show_learning_prompt(LearningPrompt(0, 1, "hello", "руддщ", "Notepad"))
        self.api.text = "руддщ"
        self.tap(self.key(VK_RETURN, 28))
        self.flush()
        self.assertEqual(self.api.messages, [])
        self.assertEqual(self.api.text, "руддщ")
        self.assertIsNone(self.engine.learning_prompt)

    def test_prompt_appearing_after_interception_does_not_leave_input_held(self) -> None:
        self.tap(self.key(VK_RETURN, 28))
        self.engine._show_learning_prompt(LearningPrompt(0, 1, "hello", "руддщ", "Notepad"))
        self.flush()
        self.assertEqual(self.api.messages, [])
        self.assertFalse(self.backend._holding)
        self.assertIsNone(self.engine.learning_prompt)

    def test_partial_action_send_is_reported_without_a_duplicate_retry(self) -> None:
        self.type("hello")
        self.tap(self.key(VK_RETURN, 28))
        self.api.send_count = 1
        self.flush()
        self.assertEqual(self.api.messages, ["hello"])
        self.assertIn("1 из 2", self.engine.snapshot.last_error)
        self.assertFalse(self.backend._holding)

    def test_keypad_enter_preserves_extended_scan_code(self) -> None:
        self.type("ghbdtn")
        self.tap(replace(self.key(VK_RETURN, 28), extended=True))
        self.flush()
        self.assertEqual(self.api.messages, ["привет"])
        actions = [item for batch in self.api.sent for item in batch if item.virtual_key == VK_RETURN]
        self.assertEqual(len(actions), 2)
        self.assertTrue(all(item.extended for item in actions))

    def test_shift_enter_is_not_intercepted(self) -> None:
        self.api.physical(self.key(VK_SHIFT, 42))
        self.api.text = "draft"
        self.tap(self.key(VK_RETURN, 28))
        self.assertEqual(self.api.messages, ["draft"])
        self.assertIsNone(self.backend._deferred_action)

    def test_failure_cancels_submission_and_restores_keyboard(self) -> None:
        self.type("ghbdtn")
        self.tap(self.key(VK_RETURN, 28))
        self.api.accept_switch = False
        self.flush()
        self.assertEqual(self.api.messages, [])
        self.assertEqual(self.api.text, "ghbdtn")
        self.assertFalse(self.backend._holding)
        self.assertIsNone(self.engine._deferred_action)

    def test_pointer_and_focus_change_cancel_without_submitting_to_another_field(self) -> None:
        for pointer in (False, True):
            with self.subTest(pointer=pointer):
                self.type("ghbdtn")
                self.tap(self.key(VK_RETURN, 28))
                if pointer:
                    self.backend._handle_native(self.key(0, 0))
                else:
                    self.api.foreground += 1
                self.flush()
                self.assertEqual(self.api.messages, [])
                self.assertFalse(self.backend._holding)
                self.api.text = ""

    def test_missing_release_times_out_without_sending_and_stop_releases_capture(self) -> None:
        self.type("ghbdtn")
        self.api.physical(self.key(VK_RETURN, 28))
        self.flush()
        self.engine._expire_deferred_action()
        self.assertTrue(self.backend._holding)
        with patch("keyswitch.engine.time.monotonic", return_value=self.engine._action_deadline + 1):
            self.engine._expire_deferred_action()
        self.assertFalse(self.backend._holding)
        self.assertEqual(self.api.messages, [])
        self.api.physical(self.key(VK_RETURN, 28, pressed=False))
        self.api.physical(self.key(VK_RETURN, 28))
        self.backend.stop()
        self.assertFalse(self.backend._holding)


if __name__ == "__main__":
    unittest.main()
