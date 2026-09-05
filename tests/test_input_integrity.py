"""Regression matrix with a visible text buffer, not just injection counts."""

from __future__ import annotations

import tempfile
import ctypes
import unittest
from collections.abc import Iterable, Sequence
from dataclasses import replace
from pathlib import Path
from unittest.mock import patch

from keyswitch.backend import KeyEvent, SHIFT_MASK, LOCK_MASK
from keyswitch.config import SettingsStore
from keyswitch.engine import KeySwitchEngine, LearningPrompt, MAX_WORD_STROKES, _LayoutSelection
from keyswitch.history import HistoryStore
from keyswitch.layouts import LayoutPair
from keyswitch.windows_backend import NativeInput, NativeKeyEvent, VK_RETURN, VK_SHIFT, WindowsBackend
from test_engine_behaviour import FakeBackend, plain_key
from test_windows_backend import FakeWindowsAPI, key_event
from test_x11_backend import backend_with, payload
from keyswitch.x11_backend import XkbStateRec, X11Error


class EditorBackend(FakeBackend):
    """Model editing and submission before the passive observer sees a key."""

    def __init__(self) -> None:
        super().__init__()
        self.text = ""
        self.caret = 0
        self.submissions: list[str] = []

    def type(self, event: KeyEvent) -> None:
        if not event.pressed:
            return
        if event.key_name in {"Return", "KP_Enter"}:
            self.submissions.append(self.text)
            self.text = ""
            self.caret = 0
        elif event.key_name in {"Tab", "ISO_Left_Tab"}:
            self.window += 1
            self.text = "another field"
            self.caret = len(self.text)
        elif event.key_name == "Left":
            self.caret = max(0, self.caret - 1)
        elif event.key_name == "BackSpace":
            if self.caret:
                self.text = self.text[:self.caret - 1] + self.text[self.caret:]
                self.caret -= 1
        elif event.character:
            self.text = self.text[:self.caret] + event.character + self.text[self.caret:]
            self.caret += len(event.character)

    def inject_correction(
        self, strokes: Iterable[KeyEvent], target_group: int,
        boundary: KeyEvent | None, source_group: int | None = None,
        late: Sequence[KeyEvent] = (),
    ) -> int:
        word = tuple(strokes)
        count = len(word) + int(boundary is not None) + len(late)
        start = self.caret - count
        if start < 0:
            raise AssertionError("Correction attempted to erase text outside the word")
        replacement = "".join(stroke.character_for(target_group) for stroke in word)
        replacement += boundary.character if boundary else ""
        replacement += "".join(stroke.character_for(target_group) for stroke in late)
        self.text = self.text[:start] + replacement + self.text[self.caret:]
        self.caret = start + len(replacement)
        return super().inject_correction(word, target_group, boundary, source_group, late)


class InputIntegrityTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        root = Path(self.temporary.name)
        self.settings = SettingsStore(root / "config.json")
        self.settings.set("detection.context_policy", "off")
        self.settings.set("detection.early_switch", False)
        self.settings.set("detection.respect_manual_layout", False)
        self.settings.set("diagnostics.technical_logging", True)
        self.backend = EditorBackend()
        self.engine = KeySwitchEngine(self.settings, HistoryStore(root / "history.jsonl"), self.backend)
        self.pair = LayoutPair()
        self.serial = 100

    def key(self, name: str, character: str = "", *, group: int | None = None) -> KeyEvent:
        self.serial += 1
        source = self.backend.group if group is None else group
        other = self.pair.translate(character, "us" if source == 0 else "ru", "ru" if source == 0 else "us")
        chars = (character, other) if source == 0 else (other, character)
        return KeyEvent(True, self.serial, name, character, chars, source, 0, self.serial)

    def send(self, event: KeyEvent) -> None:
        self.backend.type(event)
        self.engine._handle(event)

    def tap(self, event: KeyEvent) -> None:
        self.send(event)
        self.send(replace(event, pressed=False))

    def type(self, text: str, *, group: int | None = None) -> None:
        for character in text:
            self.tap(self.key("space" if character == " " else character, character, group=group))

    def reset_editor(self, group: int = 0) -> None:
        self.engine._clear_word()
        self.engine._untracked_token = False
        self.backend.text = ""
        self.backend.caret = 0
        self.backend.group = group

    def test_manual_digits_and_literal_spelling_are_preserved(self) -> None:
        for original, expected in (("зь2", "pm2"), ("руддщ", "hello"), ("срабатыает", "chf,fnsftn")):
            with self.subTest(original=original):
                self.reset_editor(1)
                self.type(original)
                self.tap(self.key("Pause"))
                self.assertEqual(self.backend.text, expected)
                self.assertEqual(self.backend.caret, len(expected))

    def test_case_yo_and_continuation_after_idle_correction(self) -> None:
        self.type("Ghbdtn")
        last_input = self.engine._last_word_input_at
        assert last_input is not None
        self.engine._maybe_correct_after_pause(now=last_input + 2)
        self.assertEqual(self.backend.text, "Привет")
        self.type("ик")
        self.assertEqual(self.engine.snapshot.current_word, "Приветик")
        self.tap(self.key("BackSpace"))
        self.tap(self.key("Pause"))
        self.assertEqual(self.backend.text, "Ghbdtnb")
        self.reset_editor()
        self.type("`krf")
        self.tap(self.key("Pause"))
        self.assertEqual(self.backend.text, "ёлка")
        self.reset_editor()
        shifted_yo = replace(self.key("asciitilde", "~"), characters=("~", "Ё"), state=SHIFT_MASK)
        self.tap(shifted_yo)
        self.type("krf")
        self.tap(self.key("Pause"))
        self.assertEqual(self.backend.text, "Ёлка")

    def test_a_real_at_sign_prefix_protects_the_entire_handle(self) -> None:
        self.tap(replace(self.key("at", "@"), characters=("@", '"'), state=SHIFT_MASK))
        self.type("ghbdtn ")
        self.assertEqual(self.backend.text, "@ghbdtn ")

    def test_prompt_modifiers_preserve_confirmation_but_new_text_disarms_it_immediately(self) -> None:
        self.engine._show_learning_prompt(LearningPrompt(0, 1, "hello", "руддщ", "TestEditor"))
        deadline = self.engine._prompt_key_deadline
        self.assertFalse(self.engine.consumes_key(self.key("Shift_L")))
        self.assertEqual(self.engine._prompt_key_deadline, deadline)
        self.assertTrue(self.engine.consumes_key(self.key("Return")))
        # The keyboard hook can see both keys before the worker processes either.
        self.assertFalse(self.engine.consumes_key(self.key("a", "a")))
        self.assertEqual(self.engine.consumes_key(self.key("Return")), "defer")
        self.assertIsNone(self.engine.learning.forced_target(0, "hello", 1))

    def test_another_word_committed_before_boundary_release_cancels_the_old_plan(self) -> None:
        self.type("ghbdtn")
        boundary = self.key("space", " ")
        self.send(boundary)
        self.type("hello ")
        self.send(replace(boundary, pressed=False))
        self.assertEqual(self.backend.text, "ghbdtn hello ")
        self.assertEqual(self.backend.injections, [])

    def test_queued_control_messages_and_synthetic_echo_are_not_lost(self) -> None:
        for message in (_LayoutSelection(1), None):
            self.reset_editor()
            self.type("ghbdtn")
            boundary = self.key("space", " ")
            self.send(boundary)
            self.engine._events.put_nowait(message)
            self.send(replace(boundary, pressed=False))
            self.assertEqual(self.backend.text, "ghbdtn ")
            self.assertEqual(self.engine._events.get_nowait(), message)
        self.reset_editor()
        self.type("ghbdtn")
        boundary = self.key("space", " ")
        self.send(boundary)
        echo = replace(self.key("x", "x"), synthetic=True)
        self.engine._events.put_nowait(echo)
        self.send(replace(boundary, pressed=False))
        self.assertEqual(self.backend.text, "привет ")
        self.assertEqual(self.engine._events.get_nowait(), echo)

    def test_plan_is_revalidated_for_lost_input_and_unsafe_character_mapping(self) -> None:
        self.type("ghbdtn")
        boundary = self.key("space", " ")
        self.send(boundary)
        plan = self.engine._pending
        assert plan is not None
        self.engine._input_overflow.set()
        self.engine._execute_correction(plan, None)
        self.assertEqual(self.backend.text, "ghbdtn ")
        self.engine._input_overflow.clear()
        broken = replace(plan.strokes[0], characters=("g", ""))
        self.engine._execute_correction(replace(plan, strokes=(broken,)), None)
        self.assertEqual(self.backend.injections, [])

    def test_stale_early_plan_is_rejected_even_if_it_reaches_execution(self) -> None:
        self.settings.set("detection.early_switch", True)
        self.type("ghb")
        last = self.key("d", "d")
        self.send(last)
        saved = self.engine._pending
        assert saved is not None
        self.tap(self.key("BackSpace"))
        self.engine._pending = saved  # delayed stale plan from before the edit
        self.send(replace(last, pressed=False))
        self.assertEqual(self.backend.text, "ghb")
        self.assertEqual(self.backend.injections, [])

    def test_remapped_pause_and_caps_do_not_discard_text(self) -> None:
        self.settings.set("hotkeys.convert_last", "F12")
        self.type("hello")
        self.tap(self.key("Pause"))
        self.tap(self.key("Caps_Lock"))
        self.assertEqual(self.engine.snapshot.current_word, "hello")

    def test_punctuation_and_repeated_spaces_keep_their_glyphs_and_count(self) -> None:
        for suffix in (" ", "   ", ",", ".", "!", "?", ";", ":", ")", "]", "}", "…", "—", "»", "\u00a0", "\u202f", "\u2009"):
            with self.subTest(suffix=repr(suffix)):
                self.reset_editor()
                self.type("ghbdtn", group=0)
                self.type(suffix, group=0)
                self.assertEqual(self.backend.text, "привет" + suffix)

    def test_space_needs_no_letter_mapping_in_the_other_layout(self) -> None:
        self.type("ghbdtn")
        self.tap(replace(self.key("space", " "), characters=(" ", "")))
        self.assertEqual(self.backend.text, "привет ")

    def test_hyphenated_words_and_apostrophes_are_not_split_mid_token(self) -> None:
        for word in ("кто-то", "по-русски", "Санкт-Петербург", "rock-n-roll", "don't", "we’re", "O’Neill", "mother-in-law", "слово‑слово"):
            with self.subTest(word=word):
                self.reset_editor(1 if "а" <= word[0].lower() <= "я" else 0)
                self.type(word)
                self.assertEqual(self.engine.snapshot.current_word, word)
                self.assertEqual(self.backend.text, word)
                self.tap(self.key("space", " "))
                self.assertEqual(self.backend.text, word + " ")

    def test_structured_tokens_are_not_corrected_piece_by_piece(self) -> None:
        for token in ("pm2", "2ghbdtn", "--ghbdtn", "ghbdtn_world", "@ghbdtn", "/ghbdtn", "user@ghbdtn", "path/ghbdtn", "foo=ghbdtn", "HTTP", "APIKey", "fooBar"):
            with self.subTest(token=token):
                self.reset_editor()
                self.type(token + " ", group=0)
                self.assertEqual(self.backend.text, token + " ")

    def test_action_keys_are_never_replayed_after_submission_or_focus_change(self) -> None:
        self.settings.set("detection.correct_on_enter", True)  # legacy settings cannot override safety
        self.settings.set("detection.correct_on_tab", True)
        for name in ("Return", "KP_Enter", "Tab", "ISO_Left_Tab"):
            with self.subTest(name=name):
                self.reset_editor()
                self.type("ghbdtn")
                before = len(self.backend.injections)
                self.tap(self.key(name))
                self.assertEqual(len(self.backend.injections), before)
                self.assertIsNone(self.engine._pending)
                self.assertTrue(self.engine._last_committed_stale)
                if "Enter" in name or name == "Return":
                    self.assertEqual(self.backend.submissions[-1], "ghbdtn")
                    self.assertEqual(self.backend.text, "")
                else:
                    self.assertEqual(self.backend.text, "another field")

    def test_unsafe_queued_event_aborts_without_touching_visible_text(self) -> None:
        for name in ("Return", "KP_Enter", "Tab", "Left", "BackSpace", "Pointer", "Shift_L"):
            with self.subTest(name=name):
                self.reset_editor()
                self.type("ghbdtn")
                boundary = self.key("space", " ")
                self.send(boundary)
                unsafe = self.key(name)
                self.backend.type(unsafe)  # OS has already delivered this key
                visible = self.backend.text
                self.engine.enqueue(unsafe)
                before = len(self.backend.injections)
                self.send(replace(boundary, pressed=False))
                self.assertEqual(self.backend.text, visible)
                self.assertEqual(len(self.backend.injections), before)
                self.assertEqual(self.engine._events.get_nowait(), unsafe)

    def test_undo_is_unavailable_after_edit_or_pointer_move(self) -> None:
        for edit in ("Left", "BackSpace", "Pointer", "space", "x"):
            with self.subTest(edit=edit):
                self.reset_editor()
                self.type("ghbdtn ")
                self.tap(self.key(edit, " " if edit == "space" else "x" if edit == "x" else ""))
                visible = self.backend.text
                before = len(self.backend.injections)
                self.engine._schedule_undo(999)
                self.send(plain_key("z", 999, self.backend.group, False))
                self.assertEqual(len(self.backend.injections), before)
                self.assertEqual(self.backend.text, visible)

    def test_changed_focus_cancels_pending_even_when_current_word_is_empty(self) -> None:
        self.type("ghbdtn")
        boundary = self.key("space", " ")
        self.send(boundary)
        self.backend.window += 1
        self.backend.text, self.backend.caret = "elsewhere", 9
        self.send(replace(boundary, pressed=False))
        self.assertEqual(self.backend.text, "elsewhere")
        self.assertEqual(self.backend.injections, [])
        self.assertIsNone(self.engine._pending)

    def test_complex_unicode_cannot_start_a_destructive_suffix_correction(self) -> None:
        for complex_text in ("🙂", "е\u0308", "\u0301", "\u200d", "ab"):
            with self.subTest(text=repr(complex_text)):
                self.reset_editor()
                event = self.key("Compose", complex_text)
                self.tap(event)
                self.type("ghbdtn ")
                self.assertEqual(self.backend.text, complex_text + "ghbdtn ")
                self.type("ghbdtn ")
                self.assertTrue(self.backend.text.endswith("привет "))

    def test_long_input_is_bounded_and_does_not_correct_an_untracked_suffix(self) -> None:
        text = "a" * (MAX_WORD_STROKES + 20) + "ghbdtn "
        self.type(text)
        self.assertEqual(self.backend.text, text)
        self.assertLessEqual(len(self.engine._strokes), MAX_WORD_STROKES)
        self.type("ghbdtn ")
        self.assertTrue(self.backend.text.endswith("привет "))

    def test_explicit_short_rule_is_not_ignored_by_minimum_length(self) -> None:
        self.settings.set("detection.minimum_length", 8)
        self.engine.learning.confirm_manual(0, "kb", 1, 2)
        self.type("kb ")
        self.assertEqual(self.backend.text, "ли ")
        self.reset_editor()
        self.engine.learning.reject(0, "kb", 1)
        self.type("kb ")
        self.assertEqual(self.backend.text, "kb ")

    def test_injection_failure_releases_input_and_disables_stale_undo(self) -> None:
        self.type("ghbdtn")
        with patch.object(self.backend, "hold_input", side_effect=RuntimeError("hold failed")), patch.object(self.backend, "release_input", wraps=self.backend.release_input) as release:
            self.tap(self.key("space", " "))
        release.assert_called_once()
        self.assertEqual(self.backend.text, "ghbdtn ")
        self.assertIn("hold failed", self.engine.snapshot.last_error)
        self.assertTrue(self.engine._last_committed_stale)


class NativeInputIntegrityTests(unittest.TestCase):
    def test_x11_pointer_and_unknown_keyboard_state(self) -> None:
        backend, libraries = backend_with()
        backend._control = 101
        self.assertEqual(backend.release_input(), 0)
        self.assertEqual(backend.complete_action(True), 0)
        pointer = backend._decode_event(payload(4))
        assert pointer is not None
        self.assertEqual(pointer.key_name, "Pointer")
        libraries.x11.XkbGetState.return_value = 1
        with self.assertRaisesRegex(X11Error, "Caps Lock"):
            backend.inject_correction((key_event(),), 1, None)
        libraries.xtst.XTestFakeKeyEvent.assert_not_called()

    def test_x11_replay_compensates_caps_lock(self) -> None:
        backend, libraries = backend_with()
        backend._control = 101
        def caps_on(_display: object, _device: object, pointer: ctypes._CData | ctypes._CArgObject | int) -> int:
            ctypes.cast(pointer, ctypes.POINTER(XkbStateRec)).contents.locked_mods = LOCK_MASK
            return 0
        libraries.x11.XkbGetState.side_effect = caps_on
        backend.inject_correction((key_event(),), 1, None)
        sequence = [(call.args[1], call.args[2]) for call in libraries.xtst.XTestFakeKeyEvent.call_args_list]
        self.assertEqual(sequence, [(22, 1), (22, 0), (50, 1), (30, 1), (30, 0), (50, 0)])

    def test_swallowed_enter_repeats_never_reach_the_chat(self) -> None:
        api = FakeWindowsAPI()
        backend = WindowsBackend(api)
        backend.set_key_filter(lambda event: event.key_name == "Return")
        event = NativeKeyEvent(True, VK_RETURN, 28, False, False, 1)
        self.assertTrue(backend._handle_native(event))
        backend.set_key_filter(None)  # prompt already confirmed on first press
        for _ in range(4):
            self.assertTrue(backend._handle_native(event))
        self.assertFalse(backend._handle_native(replace(event, injected=True)))
        self.assertTrue(backend._handle_native(replace(event, pressed=False)))
        self.assertFalse(backend._handle_native(event))

    def test_replayed_keys_reenter_the_hook_without_being_held_again(self) -> None:
        api = FakeWindowsAPI()
        backend = WindowsBackend(api)
        delivered: list[KeyEvent] = []
        backend._listener = delivered.append
        original_send = api.send_inputs
        def send(inputs: tuple[NativeInput, ...]) -> int:
            # Simulate the native synchronous hook call absent in the old mock.
            for item in inputs:
                backend._handle_native(NativeKeyEvent(item.pressed, item.virtual_key or 65, item.scan_code or 30, item.extended, item.synthetic, 1, replayed=not item.synthetic))
            return original_send(inputs)
        with patch.object(api, "send_inputs", side_effect=send):
            backend.hold_input()
            self.assertTrue(backend._handle_native(NativeKeyEvent(True, 68, 32, False, False, 1)))
            backend.inject_correction((key_event(),), 1, None, late=(key_event(keycode=31),))
        self.assertFalse(backend._holding)
        self.assertEqual(backend._held, [])
        self.assertEqual([event.keycode for event in delivered if not event.synthetic], [31, 31, 32])

    def test_validation_error_and_pointer_during_layout_switch_release_input(self) -> None:
        api = FakeWindowsAPI()
        backend = WindowsBackend(api)
        backend.hold_input()
        with self.assertRaisesRegex(RuntimeError, "Неизвестная группа"):
            backend.inject_correction((), 8, None)
        self.assertFalse(backend._holding)
        request = api.request_layout
        def click_during_switch(layout: int) -> bool:
            backend._handle_native(NativeKeyEvent(True, 0, 0, False, False, 1))
            return request(layout)
        with patch.object(api, "request_layout", side_effect=click_during_switch):
            backend.hold_input()
            with self.assertRaisesRegex(RuntimeError, "Место ввода изменилось"):
                backend.inject_correction((key_event(),), 1, None)
        self.assertEqual(api.sent, [])
        self.assertFalse(backend._holding)

    def test_shift_enter_is_not_a_learning_confirmation(self) -> None:
        event = replace(key_event(character=""), key_name="Return", state=SHIFT_MASK)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            engine = KeySwitchEngine(SettingsStore(root / "config.json"), HistoryStore(root / "history.jsonl"), FakeBackend())
            engine._show_learning_prompt(LearningPrompt(0, 1, "hello", "руддщ", "TestEditor"))
            self.assertFalse(engine.consumes_key(event))
            engine._handle(event)
            self.assertIsNone(engine.learning.forced_target(0, "hello", 1))

    def test_caps_state_changes_only_replay_case_not_punctuation(self) -> None:
        backend = WindowsBackend(FakeWindowsAPI())
        for caps_now in (False, True):
            backend._caps_lock = caps_now
            for caps_then in (False, True):
                for shift_then in (False, True):
                    state = (LOCK_MASK if caps_then else 0) | (SHIFT_MASK if shift_then else 0)
                    letter = replace(key_event(), state=state)
                    events = backend._stroke_inputs(letter, group=1)
                    shifted = any(item.virtual_key == VK_SHIFT for item in events)
                    self.assertEqual(shifted, shift_then ^ caps_then ^ caps_now)
                    punctuation = replace(letter, characters=(".", "."))
                    events = backend._stroke_inputs(punctuation, group=1)
                    self.assertEqual(any(item.virtual_key == VK_SHIFT for item in events), shift_then)

    def test_pointer_events_are_observed_without_being_suppressed(self) -> None:
        backend = WindowsBackend(FakeWindowsAPI())
        events: list[KeyEvent] = []
        backend._listener = events.append
        backend.hold_input()
        self.assertFalse(backend._handle_native(NativeKeyEvent(True, 0, 0, False, False, 1)))
        self.assertEqual(events[0].key_name, "Pointer")
        self.assertEqual(backend.release_input(), 0)


if __name__ == "__main__":
    unittest.main()
