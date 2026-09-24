"""Diagnostic traces explain edits without recording keys or adjacent text."""

from __future__ import annotations

import json
import unittest
from dataclasses import replace
from typing import cast
from unittest.mock import patch

from keyswitch.backend import CONTROL_MASK
from keyswitch.context_access import PlatformFieldReader, RETRY_DELAYS
from keyswitch.context_model import ACTIONS, ContextModel
from test_input_integrity import InputIntegrityTests

# Heavily biases the "wait" action (ACTIONS[2]) so short words wait for context.
BIAS_WEIGHT_TOWARD_WAIT = 20.0
EXPECTED_EDIT_COUNT = 2
FIRST_EDIT_CHARACTERS_BEFORE = 2


class InputDiagnosticsTests(InputIntegrityTests):
    def setUp(self) -> None:
        super().setUp()
        self.settings.set("detection.context_policy", "assist")
        self.engine.context_policy.model = ContextModel(
            {"bias": (0.0, 0.0, BIAS_WEIGHT_TOWARD_WAIT, 0.0), "app:testeditor": (0.0,) * len(ACTIONS)},
            "context-v1-diagnostics-fixture",
        )

    @staticmethod
    def events(lines: list[str]) -> list[dict[str, object]]:
        return [cast(dict[str, object], json.loads(line.split("TECHNICAL ", 1)[1])) for line in lines]

    def test_wait_start_and_backspace_cancel_share_id_without_word_text(self) -> None:
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type("g ")
            self.tap(self.key("BackSpace"))
            self.tap(self.key("BackSpace"))
        events = self.events(logs.output)
        started = next(e for e in events if e["event"] == "context_wait_started")
        cancelled = next(e for e in events if e["event"] == "context_wait_cancelled")
        self.assertEqual(started["wait_id"], cancelled["wait_id"])
        self.assertEqual(cancelled["reason"], "backspace")
        edits = [e for e in events if e["event"] == "input_edit_observed"]
        self.assertEqual(len(edits), EXPECTED_EDIT_COUNT)  # Releases are not extra edits.
        self.assertEqual(edits[0]["context_characters_before"], FIRST_EDIT_CHARACTERS_BEFORE)
        self.assertEqual(edits[1]["context_characters_before"], 1)
        self.assertEqual(edits[0]["wait_id"], started["wait_id"])
        self.assertEqual(edits[1]["wait_id"], None)
        for event in (started, cancelled, *edits):
            self.assertNotIn("original", event)
            self.assertNotIn("character", event)
            self.assertNotIn("before", event)
        self.assertFalse(edits[0]["text_verified"])
        self.assertEqual(self.backend.text, "")

    def test_pointer_navigation_shortcut_and_settings_explain_wait_cancellation(self) -> None:
        for key, reason in (("Pointer", "pointer_activity"), ("Left", "navigation"), ("shortcut", "modifier_shortcut"), ("settings", "settings_changed")):
            with self.subTest(key=key):
                self.reset_editor()
                # The previous round left the caret somewhere unseen; this one
                # starts in a field of its own.
                self.engine._caret_moved = False
                self.type("g ")
                with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                    if key == "settings":
                        self.settings.set("detection.context_aware", False)
                        self.settings.set("detection.context_aware", True)
                    elif key == "shortcut":
                        self.tap(replace(self.key("a", "a"), state=CONTROL_MASK))
                    else:
                        self.tap(self.key(key))
                cancelled = next(e for e in self.events(logs.output) if e["event"] == "context_wait_cancelled")
                self.assertEqual(cancelled["reason"], reason)

    def test_keys_held_across_a_focus_change_are_forgotten_at_once(self) -> None:
        """A press whose window is gone must not outlive it by three seconds.

        Its release goes to whatever took the focus, so the engine kept the key in its
        books until the stale timer ran - and a withheld Enter, which gives up a second
        earlier, was dropped waiting for a key that was never coming up. Recorded on
        Windows 0.24.0 with `Escape`, right `Shift` and `NumLock`.
        """

        held = self.key("a", "a")
        self.send(held)
        self.assertIn(held.keycode, self.engine._pressed)
        self.backend.window += 1
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._poll_current_group()
        self.assertEqual((self.engine._pressed, self.engine._modifier_keycodes, self.engine._pressed_since),
                         (set(), set(), {}))
        change = next(e for e in self.events(logs.output) if e["event"] == "focus_changed")
        self.assertEqual(change["released_keycodes"], [held.keycode])
        # A release that arrives after all finds nothing to clear and changes nothing.
        self.send(replace(held, pressed=False))
        self.assertEqual(self.engine._pressed, set())

    def test_edit_diagnostics_are_opt_in_and_suppress_excluded_and_sensitive_fields(self) -> None:
        self.engine._focus_window = self.backend.window
        for mode in ("off", "disabled", "excluded", "sensitive"):
            with self.subTest(mode=mode):
                self.settings.set("diagnostics.technical_logging", mode != "off")
                self.settings.set("enabled", mode != "disabled")
                self.settings.set("exclusions.applications", ["TestEditor"] if mode == "excluded" else [])
                self.engine._sensitive_context_window = self.backend.window if mode == "sensitive" else None
                with self.assertNoLogs("keyswitch.engine", level="INFO"):
                    self.engine._log_input_edit(self.key("BackSpace"), "TestEditor")

    def test_printable_keys_are_not_logged_but_delete_is_distinguished(self) -> None:
        self.engine.context_policy.stream.text = "private adjacent text"
        with self.assertNoLogs("keyswitch.engine", level="INFO"):
            self.engine._log_input_edit(self.key("s", "s"), "TestEditor")
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._log_input_edit(self.key("Delete"), "TestEditor")
        event = self.events(logs.output)[0]
        self.assertEqual(event["edit"], "delete")
        self.assertNotIn("private", logs.output[0])

    def test_manual_conversion_reports_pending_wait_without_claiming_editor_verification(self) -> None:
        self.type("g ")
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.tap(self.key("Pause"))
        scheduled = next(e for e in self.events(logs.output) if e["event"] == "manual_conversion_scheduled")
        self.assertIsNotNone(scheduled["wait_id"])
        self.assertEqual(scheduled["source"], "last_committed")
        self.assertEqual(self.backend.text, "п ")
        self.assertIsNone(self.engine._context_waiting)
        cancelled = next(e for e in self.events(logs.output) if e["event"] == "context_wait_cancelled")
        self.assertEqual(cancelled["wait_id"], scheduled["wait_id"])
        self.assertEqual(cancelled["reason"], "manual_conversion")

    def test_manual_intent_cancels_wait_even_while_waiting_for_key_release(self) -> None:
        self.type("g ")
        self.engine._pressed.add(1)
        self.tap(self.key("Pause"))
        self.assertIsNone(self.engine._context_waiting)
        self.assertIsNotNone(self.engine._pending)
        self.assertEqual(self.backend.text, "g ")
        self.assertEqual(self.backend.injections, [])

    def test_delete_switch_layout_and_retype_are_distinct_from_pause(self) -> None:
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type("g ")
            self.tap(self.key("BackSpace"))
            self.tap(self.key("BackSpace"))
            self.backend.group = 1
            self.engine._poll_current_group()
            self.type("п ")
        events = self.events(logs.output)
        names = [e["event"] for e in events]
        self.assertNotIn("manual_conversion_scheduled", names)
        self.assertNotIn("correction_applied", names)
        switched_index = next(i for i, e in enumerate(events) if e["event"] == "layout_change_observed" and e["selected_group"] == 1)
        self.assertLess(names.index("input_edit_observed"), switched_index)
        switched = events[switched_index]
        self.assertEqual(switched["selected_group"], 1)
        self.assertFalse(switched["initiated_by_engine"])
        self.assertEqual([e["original"] for e in events if e["event"] == "word_evaluation"], ["g", "п"])
        self.assertEqual(self.backend.text, "п ")

    def test_provider_failure_is_visible_in_decision_without_exception_text(self) -> None:
        reader = PlatformFieldReader()
        with patch("keyswitch.context_access.sys.platform", "win32"), patch(
            "keyswitch.windows_context.WindowsFieldReader", side_effect=ImportError("private field content"),
        ):
            self.assertIsNone(reader.read("TestEditor", 1))
        self.engine.context_policy.reader = reader
        self.settings.set("detection.context_read_field", True)
        # "g" is outside the curated single-letter list, so the decision is an
        # ordinary short-word keep and the diagnostics are what this test is about.
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type("g ")
        decision = next(e for e in self.events(logs.output) if e["event"] == "context_decision")
        self.assertEqual(decision["field_reader_details"], {
            "status": "unavailable", "failure_stage": "initialization", "failure_type": "import_error",
            # The class name says which call failed; the message never appears.
            "failure_name": "ImportError",
            "retry": {"attempts": 0, "limit": len(RETRY_DELAYS), "after_ms": None},
        })
        self.assertEqual(decision["baseline_reason"], "короткое слово")
        self.assertNotIn("private field content", "\n".join(logs.output))

    def test_pointer_invalidates_committed_word_before_pause_without_recording_text(self) -> None:
        self.type("hello ")
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.tap(self.key("Pointer"))
            self.tap(self.key("Pause"))
        events = self.events(logs.output)
        edits = [e for e in events if e["event"] == "input_edit_observed"]
        self.assertEqual(len(edits), 1)
        self.assertEqual(edits[0]["edit"], "pointer")
        self.assertTrue(edits[0]["last_committed_available"])
        self.assertIn("layout_switched_without_word", [e["event"] for e in events])
        self.assertNotIn("hello", "\n".join(logs.output))
        self.assertEqual(self.backend.text, "hello ")


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    return unittest.TestSuite(InputDiagnosticsTests(name) for name in InputDiagnosticsTests.__dict__ if name.startswith("test_"))
