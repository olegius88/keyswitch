"""Engine behaviour: Pause semantics, pause timing, early switching, logging."""

from __future__ import annotations

import json
import tempfile
import time
import unittest
from collections.abc import Callable, Iterable
from pathlib import Path
from unittest.mock import patch

from keyswitch.backend import ALT_MASK, CONTROL_MASK, FocusInfo
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
        self.window = 1
        self.own_window = False
        self.isolated_layout = False

    def active_application(self) -> str:
        return "TestEditor"

    def focused_window(self) -> FocusInfo | None:
        return FocusInfo(self.window, self.own_window, self.isolated_layout)

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


def digit_event(digit: str, group: int, keycode: int = 11) -> KeyEvent:
    """A digit row key: the same character in both layouts."""

    return KeyEvent(True, keycode, digit, digit, (digit, digit), group, 0, keycode)


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

    def learning_field(self, logs: list[str], word: str) -> dict[str, object]:
        evaluation = next(
            event
            for event in self.technical_events(logs)
            if event["event"] == "word_evaluation" and event["original"] == word
        )
        field = evaluation["learning"]
        assert isinstance(field, dict)
        return field

    def press_undo(self) -> None:
        control, alt = CONTROL_MASK, ALT_MASK
        group = self.engine.snapshot.current_group
        for event in (
            KeyEvent(True, 37, "Control_L", "", ("", ""), group, 0, 3000),
            KeyEvent(True, 64, "Alt_L", "", ("", ""), group, control, 3001),
            KeyEvent(True, 52, "z", "", ("z", "я"), group, control | alt, 3002),
            KeyEvent(False, 52, "z", "", ("z", "я"), group, control | alt, 3003),
            KeyEvent(False, 64, "Alt_L", "", ("", ""), group, control | alt, 3004),
            KeyEvent(False, 37, "Control_L", "", ("", ""), group, control, 3005),
        ):
            self.engine._handle(event)

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

    def test_a_digit_stays_part_of_the_word_so_pause_converts_it(self) -> None:
        self.type_word("зь", group=1)
        digit = digit_event("2", 1)
        self.engine._handle(digit)
        self.engine._handle(
            KeyEvent(False, digit.keycode, "2", "2", ("2", "2"), 1, 0, digit.keycode)
        )
        self.assertEqual(self.engine._strokes[-1], digit)
        self.assertEqual(self.engine.snapshot.current_word, "зь2")
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.press_pause()
        strokes, target, boundary = self.backend.injections[-1]
        self.assertEqual(len(strokes), 3)
        self.assertEqual((target, boundary), (0, None))
        self.assertEqual(self.engine.snapshot.last_action, "зь2 → pm2")
        self.assertEqual(self.engine.snapshot.current_group, 0)
        scheduled = next(
            event
            for event in self.technical_events(logs.output)
            if event["event"] == "manual_conversion_scheduled"
        )
        self.assertEqual((scheduled["original"], scheduled["replacement"]), ("зь2", "pm2"))
        # A token carrying a digit is code: nothing is learned from it and the
        # automatic path leaves it alone.
        self.assertFalse(scheduled["learnable"])

    def test_a_word_with_a_digit_is_never_corrected_automatically(self) -> None:
        self.type_word("ыекштп", group=1)
        digit = digit_event("2", 1)
        self.engine._handle(digit)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.press_space(1)
        self.assertEqual(self.backend.injections, [])
        evaluation = next(
            event
            for event in self.technical_events(logs.output)
            if event["event"] == "word_evaluation"
        )
        self.assertEqual(evaluation["original"], "ыекштп2")
        decision = evaluation["decision"]
        assert isinstance(decision, dict)
        self.assertFalse(decision["should_convert"])
        self.assertEqual(decision["reason"], "код, адрес или аббревиатура")

    def test_a_discarded_word_says_why_in_the_log(self) -> None:
        self.type_word("зь", group=1)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._handle(
                KeyEvent(True, 38, "a", "a", ("a", "ф"), 1, CONTROL_MASK, 800)
            )
        self.assertEqual(self.engine._strokes, [])
        discarded = next(
            event
            for event in self.technical_events(logs.output)
            if event["event"] == "word_discarded"
        )
        self.assertEqual(discarded["reason"], "modifier_shortcut")
        self.assertEqual(discarded["original"], "зь")
        self.assertEqual(discarded["length"], 2)

        # Navigation keys still drop the word, and say so.
        self.type_word("зь", group=1)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._handle(plain_key("Left", 100, 1))
        self.assertEqual(
            next(
                event["reason"]
                for event in self.technical_events(logs.output)
                if event["event"] == "word_discarded"
            ),
            "navigation",
        )

        # A digit with no word in progress drops the remembered symbols.
        self.engine._handle(quote_event())
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._handle(digit_event("2", 1))
        self.assertEqual(self.engine._symbol_strokes, [])
        discarded = next(
            event
            for event in self.technical_events(logs.output)
            if event["event"] == "word_discarded"
        )
        self.assertEqual(discarded["reason"], "non_word_key")
        self.assertEqual(discarded["symbol_count"], 1)

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

    def test_the_log_shows_what_local_learning_knows_about_the_word(self) -> None:
        self.settings.set("detection.respect_manual_layout", False)

        # No rule yet: the word is evaluated by the model alone.
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type_word("ghbdtn")
            self.press_space(0)
        learning = self.learning_field(logs.output, "ghbdtn")
        self.assertEqual(
            learning,
            {
                "enabled": True,
                "required_confirmations": 2,
                "rule_target": None,
                "confirmations": 0,
                "forced_target": None,
                "rejected_targets": [],
            },
        )

        # One manual conversion: a rule exists but does not force anything yet.
        self.backend.group = 0
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type_word("qwerty")
            self.press_pause()
        recorded = next(
            event
            for event in self.technical_events(logs.output)
            if event["event"] == "learning_rule_recorded"
        )
        self.assertEqual(
            (recorded["word"], recorded["confirmations"], recorded["active"]),
            ("qwerty", 1, False),
        )
        self.assertEqual(recorded["required_confirmations"], 2)
        self.assertEqual(self.engine.learning.rule_state(0, "qwerty"), (1, 1))

        self.backend.group = 0
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type_word("qwerty")
            self.press_space(0)
        pending = self.learning_field(logs.output, "qwerty")
        self.assertEqual(pending["confirmations"], 1)
        self.assertEqual(pending["rule_target"], 1)
        self.assertIsNone(pending["forced_target"])

        # The second manual conversion turns it into an active rule.
        self.backend.group = 0
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type_word("qwerty")
            self.press_pause()
        recorded = next(
            event
            for event in self.technical_events(logs.output)
            if event["event"] == "learning_rule_recorded"
        )
        self.assertEqual((recorded["confirmations"], recorded["active"]), (2, True))

        self.backend.group = 0
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type_word("qwerty")
            self.press_space(0)
        forced = self.learning_field(logs.output, "qwerty")
        self.assertEqual(forced["forced_target"], 1)
        self.assertEqual(forced["confirmations"], 2)
        evaluation = next(
            event
            for event in self.technical_events(logs.output)
            if event["event"] == "word_evaluation" and event["original"] == "qwerty"
        )
        decision = evaluation["decision"]
        assert isinstance(decision, dict)
        self.assertEqual(decision["reason"], "подтверждённое правило пользователя")

    def test_the_log_shows_a_rejection_and_the_word_it_blocks(self) -> None:
        self.settings.set("detection.respect_manual_layout", False)
        self.correct_hello()
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.press_undo()
        rejection = next(
            event
            for event in self.technical_events(logs.output)
            if event["event"] == "learning_rejection_recorded"
        )
        self.assertEqual(
            (rejection["word"], rejection["source_group"], rejection["target_group"]),
            ("ghbdtn", 0, 1),
        )

        self.backend.group = 0
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type_word("ghbdtn")
            self.press_space(0)
        blocked = self.learning_field(logs.output, "ghbdtn")
        self.assertEqual(blocked["rejected_targets"], [1])
        evaluation = next(
            event
            for event in self.technical_events(logs.output)
            if event["event"] == "word_evaluation" and event["original"] == "ghbdtn"
        )
        decision = evaluation["decision"]
        assert isinstance(decision, dict)
        self.assertEqual(decision["reason"], "отклонённое пользователем исправление")

    def test_the_learning_field_reports_the_switch_when_the_word_is_unknown(self) -> None:
        self.settings.set("detection.learning", False)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._log_word_evaluation(
                trigger="space",
                original="ghbdtn",
                alternatives={1: "привет"},
                source_group=None,
                application="TestEditor",
                application_excluded=False,
                enabled=True,
                trigger_enabled=True,
                manual_layout_protected=False,
                decision=None,
            )
        evaluation = next(
            event
            for event in self.technical_events(logs.output)
            if event["event"] == "word_evaluation"
        )
        self.assertEqual(evaluation["learning"], {"enabled": False})

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

    def test_layout_arriving_with_another_window_is_not_manual(self) -> None:
        # The first observation only records which window has the focus.
        self.engine._poll_current_group()
        self.assertEqual(self.engine._focus_window, 1)
        self.backend.window = 2
        self.backend.group = 1
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._poll_current_group()
        events = self.technical_events(logs.output)
        self.assertEqual([event["event"] for event in events], ["focus_changed", "layout_change_observed"])
        observed = events[1]
        self.assertTrue(observed["focus_changed"])
        self.assertFalse(observed["protects_next_word"])
        self.assertFalse(observed["initiated_by_engine"])
        self.assertIsNone(self.engine._manual_layout_group)
        self.assertEqual(self.engine.snapshot.current_group, 1)

        # The same change inside one window is the user's own switch.
        self.backend.group = 0
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._observe_group(0, source="keystroke")
        observed = self.technical_events(logs.output)[0]
        self.assertEqual(observed["event"], "manual_layout_observed")
        self.assertFalse(observed["focus_changed"])
        self.assertEqual(self.engine._manual_layout_group, 0)

        # Another window with yet another layout drops that manual pick, and
        # the word typed there is corrected as usual.
        self.backend.window = 3
        self.backend.group = 1
        self.engine._poll_current_group()
        self.assertIsNone(self.engine._manual_layout_group)
        self.backend.window = 4
        self.backend.group = 0
        self.engine._poll_current_group()
        self.correct_hello()

    def test_own_window_layout_is_ignored(self) -> None:
        self.engine._poll_current_group()
        self.backend.own_window = True
        self.backend.isolated_layout = True
        self.backend.window = 9
        self.backend.group = 1
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._poll_current_group()
        ignored = self.technical_events(logs.output)[0]
        self.assertEqual(ignored["event"], "layout_change_ignored")
        self.assertEqual(ignored["reason"], "own_window")
        self.assertEqual((ignored["previous_group"], ignored["selected_group"]), (0, 1))
        self.assertEqual(self.engine.snapshot.current_group, 0)
        self.assertIsNone(self.engine._manual_layout_group)
        self.assertEqual(self.engine._focus_window, 1)
        # Back in the editor nothing happened: no focus change, no protection.
        self.backend.own_window = False
        self.backend.isolated_layout = False
        self.backend.window = 1
        self.backend.group = 0
        self.engine._poll_current_group()
        self.assertEqual(self.engine._focus_window, 1)
        self.assertFalse(self.engine._last_committed_stale)
        self.correct_hello()

    def test_own_window_with_a_global_layout_still_sees_manual_switches(self) -> None:
        # X11: the layout is global, so a switch made while a KeySwitch window
        # (or the E2E's own entry) is focused is the user's own choice.
        self.engine._poll_current_group()
        self.backend.own_window = True
        self.backend.window = 9
        self.backend.group = 1
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._poll_current_group()
        observed = self.technical_events(logs.output)[0]
        self.assertEqual(observed["event"], "manual_layout_observed")
        self.assertFalse(observed["focus_changed"])
        self.assertEqual(self.engine._manual_layout_group, 1)
        self.assertEqual(self.engine.snapshot.current_group, 1)
        self.assertEqual(self.engine._focus_window, 1)

    def test_engine_switch_drops_an_older_manual_pick(self) -> None:
        self.engine._manual_layout_group = 1
        self.engine._manual_layout_observed_at = time.monotonic()
        self.correct_hello()
        self.assertIsNone(self.engine._manual_layout_group)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type_word("привет", group=1, start=50)
            self.press_space(1)
        evaluation = next(
            event for event in self.technical_events(logs.output) if event["event"] == "word_evaluation"
        )
        self.assertIsNone(evaluation["skipped_reason"])

    def test_moving_to_another_window_drops_the_unfinished_word(self) -> None:
        self.correct_hello()
        self.assertFalse(self.engine._last_committed_stale)
        self.type_word("руд", group=1, start=60)
        self.assertEqual(self.engine.snapshot.current_word, "руд")
        self.backend.window = 2
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._poll_current_group()
        changed = next(
            event for event in self.technical_events(logs.output) if event["event"] == "focus_changed"
        )
        self.assertEqual((changed["previous_window"], changed["window"]), (1, 2))
        self.assertEqual(changed["dropped_word_length"], 3)
        self.assertEqual(self.engine._strokes, [])
        self.assertEqual(self.engine.snapshot.current_word, "")
        self.assertTrue(self.engine._last_committed_stale)
        # Pause in the new window only switches the layout: the word behind
        # the previous correction lives in the other window.
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.press_pause()
        names = [event["event"] for event in self.technical_events(logs.output)]
        self.assertIn("layout_switched_without_word", names)
        self.assertEqual(len(self.backend.injections), 1)

    def _undo_early_switch(self) -> None:
        self.settings.set("detection.early_switch", True)
        self.type_word("ghbd")
        self.assertEqual(self.engine._early_switch_origin, 0)
        self.assertEqual(len(self.backend.injections), 1)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._schedule_undo(200)
            self.engine._handle(plain_key("z", 200, 1, pressed=False))
        strokes, target, boundary = self.backend.injections[-1]
        self.assertEqual((len(strokes), target, boundary), (4, 0, None))
        self.assertEqual(self.engine.snapshot.current_group, 0)
        self.assertEqual(self.engine.snapshot.current_word, "ghbd")
        self.assertEqual(
            self.engine.snapshot.last_action, "прив → ghbd · раннее переключение отменено"
        )
        self.assertIsNone(self.engine._early_switch_origin)
        self.assertTrue(self.engine._early_switch_undone)
        events = self.technical_events(logs.output)
        scheduled = next(event for event in events if event["event"] == "early_switch_undo_scheduled")
        self.assertEqual((scheduled["source_group"], scheduled["target_group"]), (1, 0))
        applied = next(event for event in events if event["event"] == "correction_applied")
        self.assertEqual(applied["mode"], "early_undo")
        self.assertEqual(applied["deleted_characters"], 4)
        self.assertFalse(applied["automatic"])
        # The rest of the word is neither switched early again nor corrected.
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type_word("tn", group=0, start=34)
            self.press_space(0)
        self.assertEqual(len(self.backend.injections), 2)
        evaluation = next(
            event for event in self.technical_events(logs.output) if event["event"] == "word_evaluation"
        )
        self.assertEqual(evaluation["skipped_reason"], "manual_layout_protected")
        protection = evaluation["protection"]
        assert isinstance(protection, dict)
        self.assertEqual(protection["reason"], "early_switch_undone")
        self.assertFalse(self.engine._early_switch_undone)
        self.assertIsNone(self.engine._manual_layout_group)
        self.assertEqual(self.engine.snapshot.correction_count, 0)
        self.assertEqual(self.history.read(), [])
        # The next wrong-layout word is switched early again.
        self.type_word("ghbd", start=90)
        self.assertEqual(len(self.backend.injections), 3)
        self.assertEqual(self.engine._early_switch_origin, 0)
        self.assertEqual(self.engine.snapshot.current_word, "прив")

    def test_undo_during_an_early_switch_reverts_the_prefix(self) -> None:
        self._undo_early_switch()

    def test_undo_during_an_early_switch_holds_without_manual_layout_respect(self) -> None:
        self.settings.set("detection.respect_manual_layout", False)
        self._undo_early_switch()

    def test_clearing_an_early_switched_word_records_the_correction(self) -> None:
        self.settings.set("detection.early_switch", True)
        self.type_word("ghbd")
        self.type_word("ет", group=1, start=34)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._handle(plain_key("Left", 113, 1))
        completed = next(
            event for event in self.technical_events(logs.output) if event["event"] == "early_switch_completed"
        )
        self.assertEqual((completed["original"], completed["replacement"]), ("ghbdtn", "привет"))
        self.assertEqual(self.engine.snapshot.correction_count, 1)
        self.assertEqual(len(self.history.read()), 1)
        self.assertTrue(self.engine._last_committed_stale)
        assert self.engine._last_correction is not None
        self.assertEqual(self.engine._last_correction.mode, "early")
        self.assertIsNone(self.engine._early_switch_origin)
        self.assertEqual(self.engine._strokes, [])
        # A generic undo now reverts exactly that prefix.
        self.engine._schedule_undo(200)
        self.engine._handle(plain_key("z", 200, 1, pressed=False))
        strokes, target, boundary = self.backend.injections[-1]
        self.assertEqual((len(strokes), target, boundary), (6, 0, None))

    def test_learning_is_not_offered_for_a_lone_letter_or_symbols(self) -> None:
        self.settings.set("detection.learning_confirmations", 1)
        self.engine._handle(boundary_event(True, group=1))
        self.engine._handle(boundary_event(False, group=1))
        self.type_word("б", group=1, start=70)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.press_pause()
        strokes, target, boundary = self.backend.injections[-1]
        self.assertEqual((len(strokes), target, boundary), (1, 0, None))
        self.assertEqual(self.engine.snapshot.last_action, "б → ,")
        self.assertIsNone(self.engine.learning_prompt)
        self.assertEqual(self.engine.learning.counts(), (0, 0))
        scheduled = next(
            event for event in self.technical_events(logs.output) if event["event"] == "manual_conversion_scheduled"
        )
        self.assertFalse(scheduled["learnable"])
        # Two letters still read as a word and become a rule at once.
        self.type_word("yj", group=0, start=80)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.press_pause()
        self.assertEqual(self.engine.snapshot.last_action, "yj → но · правило выучено")
        self.assertEqual(self.engine.learning.counts(), (1, 0))
        scheduled = next(
            event for event in self.technical_events(logs.output) if event["event"] == "manual_conversion_scheduled"
        )
        self.assertTrue(scheduled["learnable"])

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
