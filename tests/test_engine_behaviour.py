"""Engine behaviour: Pause semantics, pause timing, early switching, logging."""

from __future__ import annotations

import json
import tempfile
import time
import unittest
from collections.abc import Callable, Iterable
from pathlib import Path
from unittest.mock import patch

from keyswitch.config import SettingsStore
from keyswitch.engine import (
    ENGINE_SWITCH_GRACE_SECONDS,
    KeySwitchEngine,
    CorrectionPlan,
)
from keyswitch.history import HistoryStore
from keyswitch.indicator import layout_label
from keyswitch.layouts import LayoutPair
from keyswitch.x11_backend import BackendProbe, KeyEvent


class FakeBackend:
    def __init__(self) -> None:
        self.injections: list[tuple[tuple[KeyEvent, ...], int, KeyEvent | None]] = []
        self.group = 0

    def active_application(self) -> str:
        return "TestEditor"

    def current_group(self) -> int:
        return self.group

    def switch_group(self, group: int) -> None:
        self.group = group

    def inject_correction(
        self,
        strokes: Iterable[KeyEvent],
        target_group: int,
        boundary: KeyEvent | None,
        source_group: int | None = None,
    ) -> None:
        self.injections.append((tuple(strokes), target_group, boundary))
        self.group = target_group

    def start(self, listener: Callable[[KeyEvent], None]) -> None:
        pass

    def stop(self) -> None:
        pass

    def close(self) -> None:
        pass

    def probe(self) -> BackendProbe:
        return BackendProbe(True, "x11", ":test", "1", "2", "1", self.group)


def letter_event(character: str, keycode: int, group: int, pair: LayoutPair) -> KeyEvent:
    if group == 0:
        characters = (character, pair.translate(character, "us", "ru"))
    else:
        characters = (pair.translate(character, "ru", "us"), character)
    return KeyEvent(True, keycode, characters[0], character, characters, group, 0, keycode)


def boundary_event(pressed: bool, keycode: int = 65, group: int = 0) -> KeyEvent:
    return KeyEvent(pressed, keycode, "space", " ", (" ", " "), group, 0, 1000)


def quote_event(group: int = 1, keycode: int = 11) -> KeyEvent:
    """Shift+2: "@" in the US layout, a quote in the Russian one."""

    return KeyEvent(True, keycode, "quotedbl", ('@', '"')[group], ('@', '"'), group, 1, 500)


def plain_key(name: str, keycode: int, group: int, pressed: bool = True) -> KeyEvent:
    return KeyEvent(pressed, keycode, name, "", ("", ""), group, 0, 700)


class EngineBehaviourTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        root = Path(self.temporary.name)
        self.settings = SettingsStore(root / "config.json")
        self.settings.set("detection.early_switch", False)
        self.settings.set("diagnostics.technical_logging", True)
        self.history = HistoryStore(root / "history.jsonl")
        self.backend = FakeBackend()
        self.engine = KeySwitchEngine(self.settings, self.history, self.backend)
        self.pair = LayoutPair()

    def tearDown(self) -> None:
        self.temporary.cleanup()

    # -- helpers ---------------------------------------------------------
    def type_word(self, text: str, group: int = 0, start: int = 30) -> None:
        for index, character in enumerate(text, start):
            event = letter_event(character, index, group, self.pair)
            self.engine._handle(event)
            self.engine._handle(
                KeyEvent(False, index, event.key_name, event.character, event.characters, group, 0, index)
            )

    def press_space(self, group: int | None = None) -> None:
        current = self.engine.snapshot.current_group if group is None else group
        self.engine._handle(boundary_event(True, group=current))
        self.engine._handle(boundary_event(False, group=current))

    def press_pause(self, keycode: int = 127) -> None:
        self.engine._schedule_manual_conversion(keycode)
        self.engine._handle(
            plain_key("Pause", keycode, self.engine.snapshot.current_group, pressed=False)
        )

    def technical_events(self, logs: list[str]) -> list[dict[str, object]]:
        events: list[dict[str, object]] = []
        for line in logs:
            marker = "TECHNICAL "
            if marker in line:
                payload = json.loads(line.split(marker, 1)[1])
                assert isinstance(payload, dict)
                events.append(payload)
        return events

    def correct_hello(self) -> None:
        """Type ghbdtn + space so the engine converts it to привет."""

        before = len(self.backend.injections)
        self.type_word("ghbdtn")
        self.press_space(0)
        self.assertEqual(len(self.backend.injections), before + 1)
        self.assertEqual(self.engine.snapshot.current_group, 1)

    # -- Pause semantics -------------------------------------------------
    def test_pause_converts_only_the_symbols_typed_after_a_boundary(self) -> None:
        self.correct_hello()
        seen: list[CorrectionPlan] = []
        self.engine.subscribe_corrections(seen.append)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._handle(quote_event())
            self.engine._handle(plain_key("BackSpace", 22, 1))
            self.assertEqual(self.engine._symbol_strokes, [])
            quote = quote_event()
            self.engine._handle(quote)
            self.press_pause()
        self.assertEqual(len(self.backend.injections), 2)
        strokes, target, boundary = self.backend.injections[-1]
        self.assertEqual((strokes, target, boundary), ((quote,), 0, None))
        self.assertEqual(self.engine.snapshot.last_action, '" → @')
        self.assertEqual(self.engine.snapshot.current_group, 0)
        self.assertEqual([plan.mode for plan in seen], ["symbols"])
        self.assertIsNone(self.engine.learning_prompt)
        self.assertTrue(self.engine._last_committed_stale)
        self.assertEqual(len(self.history.read()), 1)
        events = {event["event"]: event for event in self.technical_events(logs.output)}
        scheduled = events["manual_conversion_scheduled"]
        self.assertEqual(scheduled["source"], "symbols")
        self.assertEqual((scheduled["original"], scheduled["replacement"]), ('"', "@"))
        applied = events["correction_applied"]
        self.assertEqual(applied["mode"], "symbols")
        self.assertEqual(applied["deleted_characters"], 1)
        self.assertTrue(applied["layout_switched"])

    def test_pause_converts_symbols_together_with_the_current_word(self) -> None:
        self.engine._handle(boundary_event(True, group=1))
        self.engine._handle(boundary_event(False, group=1))
        quote = quote_event()
        self.engine._handle(quote)
        self.type_word("руддщ", group=1)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.press_pause()
        strokes, target, _boundary = self.backend.injections[-1]
        self.assertEqual(strokes[0], quote)
        self.assertEqual(len(strokes), 6)
        self.assertEqual(target, 0)
        self.assertEqual(self.engine.snapshot.last_action, '"руддщ → @hello')
        scheduled = next(
            event
            for event in self.technical_events(logs.output)
            if event["event"] == "manual_conversion_scheduled"
        )
        self.assertEqual(scheduled["source"], "symbols_and_word")
        self.assertEqual(scheduled["symbol_count"], 1)

    def test_pause_with_nothing_new_only_switches_the_layout(self) -> None:
        self.correct_hello()
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.press_space()  # second space: the word is now stale
            self.press_pause()
        self.assertEqual(len(self.backend.injections), 1)
        self.assertEqual(self.backend.group, 0)
        self.assertEqual(self.engine.snapshot.current_group, 0)
        self.assertEqual(self.engine._manual_layout_group, 0)
        self.assertIn(layout_label(0), self.engine.snapshot.last_action)
        switched = next(
            event
            for event in self.technical_events(logs.output)
            if event["event"] == "layout_switched_without_word"
        )
        self.assertEqual(switched["previous_group"], 1)
        self.assertTrue(switched["last_committed_stale"])
        self.assertTrue(switched["protects_next_word"])

        with patch.object(self.backend, "switch_group", side_effect=RuntimeError("boom")):
            self.press_pause()
        self.assertEqual(self.engine.snapshot.last_error, "boom")
        self.assertEqual(self.engine.snapshot.last_action, "Раскладка не переключена")

    def test_backspace_and_navigation_after_a_word_make_it_stale(self) -> None:
        self.correct_hello()
        self.engine._handle(plain_key("BackSpace", 22, 1))
        self.assertTrue(self.engine._last_committed_stale)
        self.press_pause()
        self.assertEqual(len(self.backend.injections), 1)
        self.assertEqual(self.backend.group, 0)

        self.engine._manual_layout_group = None
        self.correct_hello()
        self.engine._handle(plain_key("Left", 100, 1))
        self.assertTrue(self.engine._last_committed_stale)

    def test_pause_without_any_word_reports_nothing_to_convert(self) -> None:
        self.press_pause()
        self.assertEqual(self.engine.snapshot.last_action, "Нет слова для ручного преобразования")
        self.assertEqual(self.backend.injections, [])

    def test_pause_right_after_a_correction_converts_the_word_back(self) -> None:
        self.correct_hello()
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.press_pause()
        strokes, target, boundary = self.backend.injections[-1]
        self.assertEqual(len(strokes), 6)
        self.assertEqual(target, 0)
        self.assertIsNotNone(boundary)
        scheduled = next(
            event
            for event in self.technical_events(logs.output)
            if event["event"] == "manual_conversion_scheduled"
        )
        self.assertEqual(scheduled["source"], "last_committed")
        self.assertEqual((scheduled["original"], scheduled["replacement"]), ("привет", "ghbdtn"))

    def test_layout_dependent_symbols_only_are_remembered(self) -> None:
        dot = KeyEvent(True, 60, "period", ".", (".", "."), 0, 0, 1)
        self.assertFalse(self.engine._layout_dependent(dot))
        self.engine._handle(dot)
        self.assertEqual(self.engine._symbol_strokes, [])
        self.engine._handle(quote_event(group=0))
        self.assertEqual(len(self.engine._symbol_strokes), 1)
        self.press_space(0)
        self.assertEqual(self.engine._symbol_strokes, [])
        self.engine._handle(quote_event(group=0))
        self.engine._handle(plain_key("Left", 100, 0))
        self.assertEqual(self.engine._symbol_strokes, [])

    # -- pause timing ----------------------------------------------------
    def test_pause_delay_setting_controls_the_timer(self) -> None:
        self.settings.set("detection.pause_delay_seconds", 0.5)
        self.assertEqual(self.engine._loop_timeout(), 0.5)
        self.type_word("ghbdtn")
        last_input = self.engine._last_word_input_at
        assert last_input is not None
        timeout = self.engine._loop_timeout()
        self.assertGreater(timeout, 0.0)
        self.assertLessEqual(timeout, 0.5)
        self.engine._maybe_correct_after_pause(now=last_input + 0.49)
        self.assertEqual(self.backend.injections, [])
        self.engine._maybe_correct_after_pause(now=last_input + 0.5)
        self.assertEqual(len(self.backend.injections), 1)
        self.assertEqual(self.engine._loop_timeout(), 0.5)

        self.settings.set("detection.pause_delay_seconds", "soon")
        self.assertEqual(self.engine._pause_delay(), 1.5)
        self.settings.set("detection.pause_delay_seconds", 50)
        self.assertEqual(self.engine._pause_delay(), 10.0)
        self.settings.set("detection.pause_delay_seconds", 0.01)
        self.assertEqual(self.engine._pause_delay(), 0.2)

    def test_stale_presses_are_pruned_and_deferrals_logged_once(self) -> None:
        self.type_word("ghbdtn")
        now = time.monotonic()
        self.engine._pressed.update({99, 50})
        self.engine._modifier_keycodes.add(99)
        self.engine._pressed_since[99] = now - 30.0
        self.engine._pressed_since[50] = now
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._maybe_correct_after_pause(now=now + 2.0)
            self.engine._maybe_correct_after_pause(now=now + 2.1)
        events = self.technical_events(logs.output)
        self.assertEqual(
            [event["event"] for event in events],
            ["stale_presses_pruned", "pause_correction_deferred"],
        )
        self.assertEqual(events[0]["keycodes"], [99])
        self.assertEqual(events[1]["reason"], "keys_pressed")
        self.assertEqual(events[1]["pressed_keycodes"], [50])
        self.assertGreaterEqual(int(str(events[1]["idle_ms"])), 2000)
        self.assertEqual(self.engine._modifier_keycodes, set())
        self.assertEqual(self.backend.injections, [])

        self.engine._pressed.clear()
        self.engine._pressed_since.clear()
        self.engine._modifier_keycodes.add(60)
        self.engine._pause_deferral_logged = False
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._maybe_correct_after_pause(now=now + 2.2)
        self.assertEqual(self.technical_events(logs.output)[0]["reason"], "modifiers_pressed")
        self.engine._modifier_keycodes.clear()
        self.engine._pending = CorrectionPlan((), None, 0, 1, "", "", 0.0, "", False)
        self.engine._pause_deferral_logged = False
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._maybe_correct_after_pause(now=now + 2.3)
        self.assertEqual(self.technical_events(logs.output)[0]["reason"], "correction_pending")
        self.engine._pending = None
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._maybe_correct_after_pause(now=now + 2.4)
        self.assertEqual(len(self.backend.injections), 1)
        evaluation = next(
            event for event in self.technical_events(logs.output) if event["event"] == "word_evaluation"
        )
        self.assertEqual(evaluation["trigger"], "pause")
        self.assertGreaterEqual(int(str(evaluation["idle_ms"])), 2000)
        self.assertEqual(evaluation["source_group"], 0)

    # -- early switching -------------------------------------------------
    def test_early_switch_rewrites_the_prefix_and_finishes_the_word(self) -> None:
        self.settings.set("detection.early_switch", True)
        seen: list[CorrectionPlan] = []
        self.engine.subscribe_corrections(seen.append)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type_word("ghbd")
        self.assertEqual(len(self.backend.injections), 1)
        strokes, target, boundary = self.backend.injections[0]
        self.assertEqual((len(strokes), target, boundary), (4, 1, None))
        self.assertEqual(self.engine.snapshot.current_group, 1)
        self.assertEqual(self.engine.snapshot.current_word, "прив")
        self.assertEqual(self.engine.snapshot.correction_count, 0)
        self.assertEqual(self.engine._early_switch_origin, 0)
        events = self.technical_events(logs.output)
        evaluation = next(event for event in events if event["event"] == "early_switch_evaluation")
        decision = evaluation["decision"]
        assert isinstance(decision, dict)
        self.assertTrue(decision["should_switch"])
        self.assertEqual(decision["replacement"], "прив")
        self.assertEqual(evaluation["prefix_length"], 4)
        applied = next(event for event in events if event["event"] == "correction_applied")
        self.assertEqual(applied["mode"], "early")

        self.type_word("ет", group=1, start=34)
        self.assertEqual(self.engine.snapshot.current_word, "привет")
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.press_space(1)
        self.assertEqual(len(self.backend.injections), 1)
        self.assertEqual(self.engine.snapshot.correction_count, 1)
        self.assertEqual(self.engine.snapshot.last_action, "ghbdtn → привет")
        self.assertEqual([plan.mode for plan in seen], ["early"])
        self.assertEqual((seen[0].original, seen[0].replacement), ("ghbdtn", "привет"))
        entries = self.history.read()
        self.assertEqual(len(entries), 1)
        self.assertEqual((entries[0].original, entries[0].replacement), ("ghbdtn", "привет"))
        self.assertIsNone(self.engine._early_switch_origin)
        assert self.engine._last_correction is not None
        self.assertEqual(self.engine._last_correction.mode, "early")
        events = self.technical_events(logs.output)
        completed = next(event for event in events if event["event"] == "early_switch_completed")
        self.assertEqual(completed["word_length"], 6)
        evaluation = next(event for event in events if event["event"] == "word_evaluation")
        self.assertEqual(evaluation["early_switch_origin"], 0)

        self.engine._schedule_undo(200)
        self.engine._handle(plain_key("z", 200, 1, pressed=False))
        strokes, target, boundary = self.backend.injections[-1]
        self.assertEqual((len(strokes), target), (6, 0))
        self.assertIsNotNone(boundary)
        self.assertEqual(self.engine.snapshot.last_action, "привет → ghbdtn · ложное срабатывание запомнено")

    def test_late_stroke_after_early_switch_is_converted_on_its_own(self) -> None:
        self.settings.set("detection.early_switch", True)
        self.type_word("ghbd")
        late = letter_event("t", 34, 0, self.pair)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._handle(late)
        self.assertEqual(len(self.backend.injections), 2)
        self.assertEqual(self.backend.injections[-1], ((late,), 1, None))
        self.assertEqual(self.engine.snapshot.current_word, "приве")
        self.assertEqual(self.engine._source_group, 1)
        self.assertEqual(self.engine._strokes[-1].group, 1)
        converted = next(
            event for event in self.technical_events(logs.output) if event["event"] == "late_stroke_converted"
        )
        self.assertEqual((converted["source_group"], converted["target_group"]), (0, 1))

        with patch.object(self.backend, "inject_correction", side_effect=RuntimeError("xtest")):
            with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                self.engine._handle(letter_event("n", 35, 0, self.pair))
        failed = next(
            event
            for event in self.technical_events(logs.output)
            if event["event"] == "late_stroke_conversion_failed"
        )
        self.assertEqual(failed["error"], "xtest")
        self.assertEqual(self.engine.snapshot.current_word, "n")
        self.assertIsNone(self.engine._early_switch_origin)

        self.engine._clear_word()
        self.type_word("ghbd", start=40)
        self.assertEqual(len(self.backend.injections), 3)
        self.engine._early_switch_at = time.monotonic() - 5.0
        self.engine._handle(letter_event("t", 44, 0, self.pair))
        self.assertEqual(self.engine.snapshot.current_word, "t")

    def test_early_switch_guards(self) -> None:
        self.settings.set("detection.early_switch", True)
        self.settings.set("exclusions.applications", ["TestEditor"])
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type_word("ghbd")
        self.assertEqual(self.backend.injections, [])
        evaluation = next(
            event for event in self.technical_events(logs.output) if event["event"] == "early_switch_evaluation"
        )
        self.assertEqual(evaluation["original"], "<redacted>")
        self.assertTrue(evaluation["application_excluded"])
        decision = evaluation["decision"]
        assert isinstance(decision, dict)
        self.assertEqual(decision["replacement"], "<redacted>")
        self.settings.set("exclusions.applications", [])

        self.engine._clear_word()
        self.engine._manual_layout_group = 0
        self.type_word("ghbd")
        self.assertEqual(self.backend.injections, [])
        self.engine._manual_layout_group = None

        self.engine._clear_word()
        self.settings.set("enabled", False)
        self.type_word("ghbd")
        self.assertEqual(self.backend.injections, [])
        self.settings.set("enabled", True)

        self.engine._clear_word()
        self.engine._pending = CorrectionPlan((), None, 0, 1, "", "", 0.0, "", False)
        self.engine._pending_trigger_keycode = 999
        self.type_word("ghbd")
        self.assertEqual(self.backend.injections, [])
        self.engine._clear_word()

        self.engine._strokes = [letter_event(character, 30, 0, self.pair) for character in "ghbd"]
        self.engine._source_group = 5
        self.engine._maybe_early_switch()
        self.assertEqual(self.backend.injections, [])
        self.engine._clear_word()

        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type_word("hello")
        evaluations = [
            event for event in self.technical_events(logs.output) if event["event"] == "early_switch_evaluation"
        ]
        self.assertEqual(len(evaluations), 1)
        decision = evaluations[0]["decision"]
        assert isinstance(decision, dict)
        self.assertFalse(decision["should_switch"])
        self.assertEqual(self.backend.injections, [])
        self.press_space(0)

        self.settings.set("detection.early_switch_min_length", 3)
        self.type_word("ghb", start=50)
        self.assertEqual(len(self.backend.injections), 1)
        self.settings.set("detection.early_switch_min_length", "four")
        self.assertEqual(self.engine._early_switch_policy().minimum_length, 4)
        self.settings.set("detection.early_switch_min_length", 1)
        self.assertEqual(self.engine._early_switch_policy().minimum_length, 3)
        self.settings.set("detection.early_switch_min_length", 40)
        self.assertEqual(self.engine._early_switch_policy().minimum_length, 8)

    def test_boundary_detector_can_still_override_an_early_switch(self) -> None:
        self.settings.set("detection.early_switch", True)
        for _ in range(2):
            self.engine.learning.record_manual(1, "привет", 0)
        self.type_word("ghbd")
        self.type_word("ет", group=1, start=34)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.press_space(1)
        self.assertEqual(len(self.backend.injections), 2)
        self.assertEqual(self.backend.injections[-1][1], 0)
        self.assertIsNone(self.engine._early_switch_origin)
        self.assertEqual(self.engine.snapshot.correction_count, 1)
        names = [event["event"] for event in self.technical_events(logs.output)]
        self.assertNotIn("early_switch_completed", names)

    def test_early_switch_without_history_still_counts(self) -> None:
        self.settings.set("detection.early_switch", True)
        self.settings.set("general.keep_history", False)
        self.type_word("ghbd")
        self.type_word("ет", group=1, start=34)
        self.press_space(1)
        self.assertEqual(self.engine.snapshot.correction_count, 1)
        self.assertEqual(self.history.read(), [])

    def test_early_switch_waits_for_the_key_release_and_absorbs_rollover(self) -> None:
        self.settings.set("detection.early_switch", True)
        self.type_word("ghb")
        held = letter_event("d", 33, 0, self.pair)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._handle(held)
        self.assertEqual(self.backend.injections, [])
        assert self.engine._pending is not None
        self.assertEqual(self.engine._pending.mode, "early")
        names = [event["event"] for event in self.technical_events(logs.output)]
        self.assertIn("early_switch_scheduled", names)
        rollover = letter_event("t", 34, 0, self.pair)
        self.engine._handle(rollover)
        self.assertEqual(self.backend.injections, [])
        self.engine._handle(KeyEvent(False, 33, "d", "d", held.characters, 0, 0, 40))
        self.assertEqual(len(self.backend.injections), 1)
        strokes, target, _boundary = self.backend.injections[0]
        self.assertEqual((len(strokes), target), (5, 1))
        self.assertEqual(self.engine.snapshot.current_word, "приве")
        self.assertEqual(self.engine._source_group, 1)
        self.assertEqual(self.engine._early_switch_origin, 0)
        self.engine._handle(KeyEvent(False, 34, "t", "t", rollover.characters, 1, 0, 41))
        self.assertEqual(len(self.backend.injections), 1)

    def test_early_switch_is_dropped_when_the_word_changes_before_release(self) -> None:
        self.settings.set("detection.early_switch", True)
        self.type_word("ghb")
        held = letter_event("d", 33, 0, self.pair)
        self.engine._handle(held)
        self.engine._handle(plain_key("BackSpace", 22, 0))
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._handle(KeyEvent(False, 33, "d", "d", held.characters, 0, 0, 40))
        self.assertEqual(self.backend.injections, [])
        self.assertIsNone(self.engine._pending)
        self.assertIsNone(self.engine._early_switch_origin)
        dropped = next(
            event for event in self.technical_events(logs.output) if event["event"] == "early_switch_dropped"
        )
        self.assertEqual(dropped["current_word_length"], 3)

        self.engine._clear_word()
        self.type_word("ghb", start=50)
        held = letter_event("d", 53, 0, self.pair)
        self.engine._handle(held)
        self.engine._handle(letter_event("t", 54, 0, self.pair))
        self.engine._handle(letter_event("n", 55, 0, self.pair))
        self.press_space(0)  # the boundary correction takes over the whole word
        self.assertEqual(len(self.backend.injections), 1)
        strokes, target, boundary = self.backend.injections[0]
        self.assertEqual((len(strokes), target), (6, 1))
        self.assertIsNotNone(boundary)
        self.engine._handle(KeyEvent(False, 53, "d", "d", held.characters, 1, 0, 60))
        self.assertEqual(len(self.backend.injections), 1)

    def test_backspacing_the_whole_prefix_forgets_the_early_switch(self) -> None:
        self.settings.set("detection.early_switch", True)
        self.type_word("ghbd")
        for _ in range(4):
            self.engine._handle(plain_key("BackSpace", 22, 1))
        self.assertIsNone(self.engine._early_switch_origin)
        self.assertEqual(self.engine.snapshot.current_word, "")

    # -- logging ---------------------------------------------------------
    def test_word_evaluation_logs_skip_reasons_shadow_decisions_and_context(self) -> None:
        self.settings.set("enabled", False)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type_word("ghbdtn")
            self.press_space(0)
        evaluation = next(
            event for event in self.technical_events(logs.output) if event["event"] == "word_evaluation"
        )
        self.assertEqual(evaluation["skipped_reason"], "disabled")
        shadow = evaluation["shadow_decision"]
        assert isinstance(shadow, dict)
        self.assertTrue(shadow["should_convert"])
        self.assertIsNone(evaluation["decision"])
        self.assertIsNone(evaluation["protection"])
        context = evaluation["context"]
        assert isinstance(context, dict)
        self.assertIsNone(context["group"])
        self.settings.set("enabled", True)

        self.settings.set("detection.correct_on_space", False)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type_word("ghbdtn")
            self.press_space(0)
        evaluation = next(
            event for event in self.technical_events(logs.output) if event["event"] == "word_evaluation"
        )
        self.assertEqual(evaluation["skipped_reason"], "trigger_disabled")
        context = evaluation["context"]
        assert isinstance(context, dict)
        self.assertEqual(context["group"], 0)
        self.settings.set("detection.correct_on_space", True)

        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._observe_group(1, source="keystroke")
            self.type_word("руддщ", group=1)
            self.press_space(1)
        events = self.technical_events(logs.output)
        observed = next(event for event in events if event["event"] == "manual_layout_observed")
        self.assertEqual(observed["source"], "keystroke")
        self.assertFalse(observed["initiated_by_engine"])
        evaluation = next(event for event in events if event["event"] == "word_evaluation")
        self.assertEqual(evaluation["skipped_reason"], "manual_layout_protected")
        protection = evaluation["protection"]
        assert isinstance(protection, dict)
        self.assertEqual(protection["source"], "keystroke")
        self.assertEqual(protection["group"], 1)
        self.assertIsInstance(protection["observed_ms_ago"], int)
        assert isinstance(evaluation["shadow_decision"], dict)

        self.settings.set("exclusions.applications", ["TestEditor"])
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type_word("ghbdtn", group=0)
            self.press_space(0)
        evaluation = next(
            event for event in self.technical_events(logs.output) if event["event"] == "word_evaluation"
        )
        self.assertEqual(evaluation["skipped_reason"], "application_excluded")
        self.assertIsNone(evaluation["shadow_decision"])
        self.assertEqual(evaluation["original"], "<redacted>")

    def test_word_evaluation_is_skipped_entirely_without_technical_logging(self) -> None:
        self.settings.set("diagnostics.technical_logging", False)
        with patch.object(self.engine, "_decide_word", wraps=self.engine._decide_word) as decide:
            self.settings.set("enabled", False)
            self.type_word("ghbdtn")
            self.press_space(0)
        decide.assert_not_called()

    def test_layout_change_right_after_an_engine_switch_is_not_manual(self) -> None:
        self.correct_hello()
        # A stale poll reporting the previous layout, then the engine's own
        # target arriving: neither is a manual switch.
        self.engine._update(current_group=0)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._observe_group(1, source="poll")
        observed = self.technical_events(logs.output)[0]
        self.assertEqual(observed["event"], "layout_change_observed")
        self.assertTrue(observed["initiated_by_engine"])
        self.assertFalse(observed["protects_next_word"])
        self.assertEqual(observed["source"], "poll")
        self.assertIsInstance(observed["engine_switch_ms_ago"], int)
        self.assertIsNone(self.engine._manual_layout_group)
        self.assertEqual(self.engine.snapshot.current_group, 1)

        # The user switching away right after the correction is manual even
        # inside the grace period.
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._observe_group(0, source="keystroke")
        observed = self.technical_events(logs.output)[0]
        self.assertEqual(observed["event"], "manual_layout_observed")
        self.assertFalse(observed["initiated_by_engine"])
        self.assertTrue(observed["protects_next_word"])
        self.assertEqual(self.engine._manual_layout_group, 0)

        # After the grace period even the engine's target counts as manual.
        self.engine._manual_layout_group = None
        self.engine._engine_switch_at = time.monotonic() - ENGINE_SWITCH_GRACE_SECONDS - 1.0
        self.backend.group = 1
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._poll_current_group()
        observed = self.technical_events(logs.output)[0]
        self.assertEqual(observed["event"], "manual_layout_observed")
        self.assertEqual(self.engine._manual_layout_group, 1)

    def test_learning_prompt_lifecycle_is_logged(self) -> None:
        self.settings.set("detection.learning_confirmations", 5)

        def manual_conversion() -> None:
            self.type_word("ghbdtn")
            self.press_pause()
            self.assertIsNotNone(self.engine.learning_prompt)

        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            manual_conversion()
            self.engine._handle(plain_key("Escape", 9, 1))
        names = [event["event"] for event in self.technical_events(logs.output)]
        self.assertIn("learning_prompt_shown", names)
        dismissed = next(
            event for event in self.technical_events(logs.output) if event["event"] == "learning_prompt_dismissed"
        )
        self.assertEqual(dismissed["reason"], "escape")

        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            manual_conversion()
            self.engine._handle(letter_event("a", 38, 1, self.pair))
        reasons = [
            event["reason"] for event in self.technical_events(logs.output) if event["event"] == "learning_prompt_dismissed"
        ]
        self.assertEqual(reasons, ["other_key"])
        self.engine._clear_word()

        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            manual_conversion()
            deadline = self.engine._learning_prompt_deadline
            assert deadline is not None
            self.assertTrue(self.engine._expire_learning_prompt(now=deadline + 1.0))
        reasons = [
            event["reason"] for event in self.technical_events(logs.output) if event["event"] == "learning_prompt_dismissed"
        ]
        self.assertEqual(reasons, ["timeout"])

        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            manual_conversion()
            self.assertTrue(self.engine.confirm_learning_prompt())
        confirmed = next(
            event for event in self.technical_events(logs.output) if event["event"] == "learning_prompt_confirmed"
        )
        self.assertEqual(confirmed["required_confirmations"], 5)

    def test_setting_changes_are_logged_with_loggable_values(self) -> None:
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.settings.set("detection.confidence", 3.5)
            self.settings.set("exclusions.applications", ["one"])
            self.settings.set("hotkeys.undo", "x" * 100)
        changes = [
            event for event in self.technical_events(logs.output) if event["event"] == "setting_changed"
        ]
        self.assertEqual(changes[0]["value"], 3.5)
        self.assertEqual(changes[1]["value"], {"type": "list", "items": 1})
        self.assertEqual(len(str(changes[2]["value"])), 80)
        self.assertEqual(self.engine._loggable_setting("*", None), "<all>")
        self.assertEqual(self.engine._loggable_setting("x", object()), "object")
        self.assertIsNone(self.engine._loggable_setting("x", None))

    def test_session_event_lists_the_new_settings(self) -> None:
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._technical_session_event("probe")
        session = self.technical_events(logs.output)[0]
        detection = session["detection_settings"]
        assert isinstance(detection, dict)
        self.assertEqual(detection["pause_delay_seconds"], 1.5)
        self.assertFalse(detection["early_switch"])
        hotkeys = session["hotkeys"]
        assert isinstance(hotkeys, dict)
        self.assertEqual(hotkeys["convert_last"], "Pause")


if __name__ == "__main__":
    unittest.main()
