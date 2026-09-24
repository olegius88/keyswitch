"""Runtime contracts around learned decisions, independent of model accuracy."""

from __future__ import annotations

import json
import unittest
from collections.abc import Iterable, Sequence
from dataclasses import replace
from typing import cast
from unittest.mock import patch

from keyswitch.backend import KeyEvent
from keyswitch.context_model import ACTIONS, ContextModel
from keyswitch.engine import MANUAL_RELEASE_TIMEOUT_SECONDS
from test_input_integrity import InputIntegrityTests

# Monotonic-clock fixture instants shared by the manual-release watchdog tests below.
KEY_PRESS_TIME = 100.0
KEY_HELD_SINCE_TIME = 90.0
WITHIN_DEADLINE_TIME = 101.0
AFTER_DEADLINE_TIME = 104.0
# A bias large enough that its action wins regardless of the other weights.
DOMINANT_BIAS = 20.0


class ContextRuntimeSafetyTests(InputIntegrityTests):
    REPLAY_CASES = (
        (0, 0, False, "контекст привет"),
        (1, 1, False, "контекст привет"),
        (2, 2, False, "контекст привет"),
        (1, 0, False, ""),
        (1, 1, True, ""),
    )

    @staticmethod
    def events(lines: list[str]) -> list[dict[str, object]]:
        return [cast(dict[str, object], json.loads(line.split("TECHNICAL ", 1)[1])) for line in lines]

    def test_replayed_releases_preserve_context_but_unknown_or_pressed_events_do_not(self) -> None:
        self.settings.set("detection.context_policy", "shadow")
        for held, observed, pressed, expected in self.REPLAY_CASES:
            with self.subTest(held=held, observed=observed, pressed=pressed):
                self.reset_editor()
                self.backend.held_count = held
                self.type("ghbdtn")
                self.backend.text = "контекст " + self.backend.text
                self.backend.caret = len(self.backend.text)
                self.engine.context_policy.stream.text = "контекст ghbdtn"
                original_inject = self.backend.inject_correction

                def inject(
                    strokes: Iterable[KeyEvent], target_group: int,
                    boundary: KeyEvent | None, source_group: int | None = None,
                    late: Sequence[KeyEvent] = (),
                ) -> int:
                    result = original_inject(strokes, target_group, boundary, source_group, late)
                    for _ in range(observed):
                        self.engine.enqueue(replace(self.key("Shift_L"), pressed=pressed))
                    return result

                with patch.object(self.backend, "inject_correction", side_effect=inject):
                    self.tap(self.key("Pause"))
                self.assertEqual(self.engine.context_policy.stream.text, expected)
                self.assertEqual(self.engine._last_committed_stale, not bool(expected))
                while not self.engine._events.empty():
                    self.engine._events.get_nowait()
                if expected:
                    self.type("ик")
                    self.assertEqual(self.engine.snapshot.current_word, "приветик")
                    self.assertEqual(self.backend.text, "контекст приветик")

    def test_repeat_pause_does_not_switch_layout_or_replace_waiting_command(self) -> None:
        with patch("keyswitch.engine.time.monotonic", return_value=KEY_PRESS_TIME):
            self.type("ghbdtn")
            self.engine._pressed.add(1)
            self.engine._pressed_since[1] = KEY_HELD_SINCE_TIME
            self.tap(self.key("Pause"))
        pending = self.engine._pending
        assert pending is not None
        self.assertEqual(self.engine._manual_release_deadline, KEY_PRESS_TIME + MANUAL_RELEASE_TIMEOUT_SECONDS)
        with patch("keyswitch.engine.time.monotonic", return_value=WITHIN_DEADLINE_TIME):
            self.tap(self.key("Pause"))
        self.assertIs(self.engine._pending, pending)
        self.assertEqual(self.backend.group, 0)
        self.assertEqual(self.backend.text, "ghbdtn")
        self.assertEqual(self.backend.injections, [])
        with patch("keyswitch.engine.time.monotonic", return_value=AFTER_DEADLINE_TIME):
            with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                self.engine._expire_manual_correction()
        self.assertIsNone(self.engine._pending)
        self.assertEqual(self.engine._manual_release_deadline, 0.0)
        self.assertEqual(self.backend.text, "ghbdtn")
        self.assertIn(1, self.engine._pressed)  # Timeout cannot assert a real key-up.
        self.assertIn("не получено отпускание", self.engine.snapshot.last_action)
        self.assertIn("manual_conversion_timeout", [e["event"] for e in self.events(logs.output)])
        self.assertEqual(self.engine.learning.rejected_targets(0, "ghbdtn"), set())

    def test_expiry_runs_before_new_input_and_does_not_delete_editor_text(self) -> None:
        with patch("keyswitch.engine.time.monotonic", return_value=KEY_PRESS_TIME):
            self.type("ghbdtn")
            self.engine._pressed.add(1)
            self.tap(self.key("Pause"))
        with patch("keyswitch.engine.time.monotonic", return_value=AFTER_DEADLINE_TIME):
            self.type("x")
        self.assertIsNone(self.engine._pending)
        self.assertEqual(self.backend.text, "ghbdtnx")

    def test_manual_release_before_deadline_executes_once_and_resets_watchdog(self) -> None:
        with patch("keyswitch.engine.time.monotonic", return_value=KEY_PRESS_TIME):
            self.type("ghbdtn")
            pause = self.key("Pause")
            self.send(pause)
            self.engine._expire_manual_correction()
            self.send(replace(pause, pressed=False))
        self.assertEqual(self.backend.text, "привет")
        self.assertEqual(self.engine._manual_release_deadline, 0.0)
        self.assertEqual(len(self.backend.injections), 1)

    def test_context_logging_distinguishes_unsupported_prediction_from_model_policy(self) -> None:
        self.settings.set("detection.context_policy", "assist")
        for supported in (False, True):
            self.reset_editor()
            weights: dict[str, tuple[float, ...]] = {"bias": (0.0, DOMINANT_BIAS, 0.0, 0.0)}
            if supported:
                weights["app:testeditor"] = (0.0,) * len(ACTIONS)
            self.engine.context_policy.model = ContextModel(weights, "context-v1-fixture")
            with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                self.type("ghbdtn ")
            event = next(e for e in self.events(logs.output) if e["event"] == "context_decision")
            self.assertEqual(event["model_supported"], supported)
            self.assertEqual(event["policy_applied"], supported)
            self.assertEqual(event["decision_source"], "context_model" if supported else "baseline")
            self.assertEqual(event["fallback_reason"], "" if supported else "unsupported_context")
            self.assertEqual(event["final_action"], "convert")
            self.assertEqual(event["applied"], True)  # Compatibility: this is NOT attribution.

    def test_context_logging_records_shadow_and_keep_policy(self) -> None:
        self.engine.context_policy.model = ContextModel(
            {"bias": (DOMINANT_BIAS, 0.0, 0.0, 0.0), "app:testeditor": (0.0,) * len(ACTIONS)}, "context-v1-fixture",
        )
        for mode in ("shadow", "assist"):
            self.reset_editor()
            self.settings.set("detection.context_policy", mode)
            with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                self.type("ghbdtn ")
            event = next(e for e in self.events(logs.output) if e["event"] == "context_decision")
            self.assertEqual(event["policy_applied"], mode == "assist")
            self.assertEqual(event["final_action"], "keep" if mode == "assist" else "convert")
            self.assertEqual(event["fallback_reason"], "" if mode == "assist" else "shadow_mode")

    def test_session_reports_context_configuration_without_field_contents(self) -> None:
        # Field reading ships on, so the override that proves the session reports it is off.
        self.settings.set("detection.context_read_field", False)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._technical_session_event("audit")
        event = self.events(logs.output)[0]
        settings = event["settings"]
        assert isinstance(settings, dict)
        overrides = settings["overrides"]
        assert isinstance(overrides, dict)
        self.assertEqual(overrides["detection.context_read_field"], False)
        self.assertEqual(event["field_reader_status"], "not_requested")
        self.engine.context_policy.reader = None
        self.assertEqual(self.engine._field_reader_status(), "not_configured")


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    return unittest.TestSuite(ContextRuntimeSafetyTests(name) for name in ContextRuntimeSafetyTests.__dict__ if name.startswith("test_"))
