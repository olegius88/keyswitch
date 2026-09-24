"""Windows action barriers: visible correction must precede chat submission."""

from __future__ import annotations

import queue
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path
from unittest.mock import patch

from keyswitch.backend import KeyEvent
from keyswitch.constants.keyboard import COMPLETED_ACTION_EVENT_COUNT, LOCK_MASK, SHIFT_MASK
from keyswitch.config import SettingsStore
from keyswitch.engine import KeySwitchEngine, LearningPrompt
from keyswitch.history import HistoryStore
from keyswitch.layouts import LayoutPair
from keyswitch.windows_backend import NativeInput, NativeKeyEvent, WindowsBackend
from keyswitch.constants.windows import VK_BACK, VK_RETURN, VK_SHIFT, VK_SPACE, VK_TAB
from test_windows_backend import FakeWindowsAPI
from fixture_values.platform import ENGLISH_HKL
from fixture_values.counts import (
    ACTION_BOUNDARY_MAX_FLUSH_ITERATIONS,
    ENTER_AUTOREPEAT_PRESS_COUNT,
    GHBDTN_REPLACEMENT_BACKSPACE_COUNT,
)
from fixture_values.keys import (
    SCAN_CODE_A,
    SCAN_CODE_B,
    SCAN_CODE_C,
    SCAN_CODE_D,
    SCAN_CODE_E,
    SCAN_CODE_ENTER,
    SCAN_CODE_F,
    SCAN_CODE_G,
    SCAN_CODE_H,
    SCAN_CODE_I,
    SCAN_CODE_J,
    SCAN_CODE_K,
    SCAN_CODE_L,
    SCAN_CODE_LEFT_SHIFT,
    SCAN_CODE_M,
    SCAN_CODE_N,
    SCAN_CODE_O,
    SCAN_CODE_P,
    SCAN_CODE_Q,
    SCAN_CODE_R,
    SCAN_CODE_S,
    SCAN_CODE_SPACE,
    SCAN_CODE_T,
    SCAN_CODE_TAB,
    SCAN_CODE_U,
    SCAN_CODE_V,
    SCAN_CODE_W,
    SCAN_CODE_X,
    SCAN_CODE_Y,
    SCAN_CODE_Z,
    UNRELATED_HELD_KEYCODE,
)


SCANS = {
    "q": SCAN_CODE_Q, "w": SCAN_CODE_W, "e": SCAN_CODE_E, "r": SCAN_CODE_R, "t": SCAN_CODE_T,
    "y": SCAN_CODE_Y, "u": SCAN_CODE_U, "i": SCAN_CODE_I, "o": SCAN_CODE_O, "p": SCAN_CODE_P,
    "a": SCAN_CODE_A, "s": SCAN_CODE_S, "d": SCAN_CODE_D, "f": SCAN_CODE_F, "g": SCAN_CODE_G,
    "h": SCAN_CODE_H, "j": SCAN_CODE_J, "k": SCAN_CODE_K, "l": SCAN_CODE_L, "z": SCAN_CODE_Z,
    "x": SCAN_CODE_X, "c": SCAN_CODE_C, "v": SCAN_CODE_V, "b": SCAN_CODE_B, "n": SCAN_CODE_N,
    "m": SCAN_CODE_M, " ": SCAN_CODE_SPACE,
}


class ActionEditorAPI(FakeWindowsAPI):
    def __init__(self) -> None:
        super().__init__()
        self.backend: WindowsBackend | None = None
        self.text = ""
        self.messages: list[str] = []
        self.fields: list[str] = []
        self.timeline: list[str] = []

    def translate_key(self, virtual_key: int, scan_code: int, state: int, layout: int) -> str:
        if virtual_key == VK_SPACE:
            return " "
        if not ord("A") <= virtual_key <= ord("Z"):
            return ""
        text = chr(virtual_key).lower()
        if layout != ENGLISH_HKL:
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
        for _ in range(ACTION_BOUNDARY_MAX_FLUSH_ITERATIONS):
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
        self.tap(self.key(VK_RETURN, SCAN_CODE_ENTER))
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
        self.tap(self.key(VK_TAB, SCAN_CODE_TAB))
        self.flush()
        self.assertEqual(self.api.fields, ["привет"])
        self.assertEqual(
            sum(item.pressed and item.virtual_key == VK_BACK for batch in self.api.sent for item in batch),
            GHBDTN_REPLACEMENT_BACKSPACE_COUNT,
        )

    def test_contextual_phrase_is_corrected_before_enter_is_delivered(self) -> None:
        self.type("z ctujlyz")
        self.tap(self.key(VK_RETURN, SCAN_CODE_ENTER))
        self.assertEqual(self.api.messages, [])
        self.flush()
        self.assertEqual(self.api.messages, ["я сегодня"])
        self.assertEqual(self.api.text, "")
        self.assertFalse(self.backend._holding)
        self.assertEqual(self.engine.context_policy.stream.text, "")

    def test_rapid_next_message_and_second_enter_stay_in_order(self) -> None:
        self.type("ghbdtn")
        self.tap(self.key(VK_RETURN, SCAN_CODE_ENTER))
        self.type("hello")
        self.tap(self.key(VK_RETURN, SCAN_CODE_ENTER))
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
        enter = self.key(VK_RETURN, SCAN_CODE_ENTER)
        for _ in range(ENTER_AUTOREPEAT_PRESS_COUNT):
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
        enter = self.key(VK_RETURN, SCAN_CODE_ENTER)
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
                self.tap(self.key(VK_RETURN, SCAN_CODE_ENTER))
                self.flush()
                self.assertEqual(self.api.messages[-1], word)
        self.settings.set("enabled", False)
        self.type("ghbdtn")
        self.tap(self.key(VK_RETURN, SCAN_CODE_ENTER))
        self.assertEqual(self.api.messages[-1], "ghbdtn")
        self.assertFalse(self.backend._holding)

    def test_learning_confirmation_remains_separate_from_chat_submission(self) -> None:
        self.engine._show_learning_prompt(LearningPrompt(0, 1, "hello", "руддщ", "Notepad"))
        self.api.text = "руддщ"
        self.tap(self.key(VK_RETURN, SCAN_CODE_ENTER))
        self.flush()
        self.assertEqual(self.api.messages, [])
        self.assertEqual(self.api.text, "руддщ")
        self.assertIsNone(self.engine.learning_prompt)

    def test_prompt_appearing_after_interception_does_not_leave_input_held(self) -> None:
        self.tap(self.key(VK_RETURN, SCAN_CODE_ENTER))
        self.engine._show_learning_prompt(LearningPrompt(0, 1, "hello", "руддщ", "Notepad"))
        self.flush()
        self.assertEqual(self.api.messages, [])
        self.assertFalse(self.backend._holding)
        self.assertIsNone(self.engine.learning_prompt)

    def test_partial_action_send_is_reported_without_a_duplicate_retry(self) -> None:
        self.type("hello")
        self.tap(self.key(VK_RETURN, SCAN_CODE_ENTER))
        self.api.send_count = 1
        self.flush()
        self.assertEqual(self.api.messages, ["hello"])
        self.assertIn("1 из 2", self.engine.snapshot.last_error)
        self.assertFalse(self.backend._holding)

    def test_keypad_enter_preserves_extended_scan_code(self) -> None:
        self.type("ghbdtn")
        self.tap(replace(self.key(VK_RETURN, SCAN_CODE_ENTER), extended=True))
        self.flush()
        self.assertEqual(self.api.messages, ["привет"])
        actions = [item for batch in self.api.sent for item in batch if item.virtual_key == VK_RETURN]
        self.assertEqual(len(actions), COMPLETED_ACTION_EVENT_COUNT)
        self.assertTrue(all(item.extended for item in actions))

    def test_shift_enter_is_not_intercepted(self) -> None:
        self.api.physical(self.key(VK_SHIFT, SCAN_CODE_LEFT_SHIFT))
        self.api.text = "draft"
        self.tap(self.key(VK_RETURN, SCAN_CODE_ENTER))
        self.assertEqual(self.api.messages, ["draft"])
        self.assertIsNone(self.backend._deferred_action)

    def test_failure_cancels_submission_and_restores_keyboard(self) -> None:
        self.type("ghbdtn")
        self.tap(self.key(VK_RETURN, SCAN_CODE_ENTER))
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
                self.tap(self.key(VK_RETURN, SCAN_CODE_ENTER))
                if pointer:
                    self.backend._handle_native(self.key(0, 0))
                else:
                    self.api.foreground += 1
                self.flush()
                self.assertEqual(self.api.messages, [])
                self.assertFalse(self.backend._holding)
                self.api.text = ""

    def test_a_stuck_unrelated_key_no_longer_swallows_the_enter(self) -> None:
        """A press whose release was lost must not cost the user a keystroke.

        The engine forgets such a press after three seconds and the deferred action
        gave up after two, so an Enter waiting behind an unrelated key was dropped one
        second before the obstacle would have cleared itself. Recorded on Windows
        0.24.0: twenty-three Enter presses cancelled as `action_release_timeout`,
        and the user saw a keyboard whose Enter had stopped working.
        """

        self.api.physical(self.key(ord("A"), SCANS["a"]))  # pressed, never released
        self.type("ghbdtn")
        self.api.physical(self.key(VK_RETURN, SCAN_CODE_ENTER))
        self.api.physical(self.key(VK_RETURN, SCAN_CODE_ENTER, pressed=False))
        self.flush()
        self.assertEqual(self.api.messages, [])
        with patch("keyswitch.engine.time.monotonic", return_value=self.engine._action_deadline + 1):
            self.engine._expire_deferred_action()
        self.flush()
        # The stuck key typed its own character before the word; what matters is that
        # the Enter behind it was submitted rather than swallowed.
        self.assertEqual(self.api.messages, ["фпривет"])
        self.assertFalse(self.backend._holding)

    def test_a_key_pressed_after_the_enter_keeps_the_cautious_answer(self) -> None:
        """Freeing the Enter is about obstacles that predate it, not about new typing."""
        self.type("ghbdtn")
        self.api.physical(self.key(VK_RETURN, SCAN_CODE_ENTER))
        self.flush()
        deadline = self.engine._action_deadline
        # A key held down after the Enter was withheld: still down at the deadline, and
        # young enough that nothing about it says its release was lost.
        self.engine._pressed.add(UNRELATED_HELD_KEYCODE)
        self.engine._pressed_since[UNRELATED_HELD_KEYCODE] = deadline
        with patch("keyswitch.engine.time.monotonic", return_value=deadline + 1):
            self.engine._expire_deferred_action()
        self.flush()
        self.assertEqual(self.api.messages, [])

    def test_missing_release_times_out_without_sending_and_stop_releases_capture(self) -> None:
        self.type("ghbdtn")
        self.api.physical(self.key(VK_RETURN, SCAN_CODE_ENTER))
        self.flush()
        self.engine._expire_deferred_action()
        self.assertTrue(self.backend._holding)
        with patch("keyswitch.engine.time.monotonic", return_value=self.engine._action_deadline + 1):
            self.engine._expire_deferred_action()
        self.assertFalse(self.backend._holding)
        self.assertEqual(self.api.messages, [])
        self.api.physical(self.key(VK_RETURN, SCAN_CODE_ENTER, pressed=False))
        self.api.physical(self.key(VK_RETURN, SCAN_CODE_ENTER))
        self.backend.stop()
        self.assertFalse(self.backend._holding)


if __name__ == "__main__":
    unittest.main()
