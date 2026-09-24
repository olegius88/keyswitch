"""Engine behaviour: Pause semantics, pause timing, early switching, logging."""

from __future__ import annotations

import json
import tempfile
import time
import unittest
from collections.abc import Callable, Iterable, Sequence
from dataclasses import replace
from pathlib import Path
from unittest.mock import patch

from keyswitch.backend import ALT_MASK, CONTROL_MASK, FocusInfo, KeyDisposition
from keyswitch.config import (
    DEFAULT_EARLY_SWITCH_MIN_LENGTH,
    DEFAULT_LEARNING_CONFIRMATIONS,
    DEFAULT_PAUSE_DELAY_SECONDS,
    SettingsStore,
)
from keyswitch.engine import (
    EARLY_SWITCH_MIN_LENGTH_CEILING,
    EARLY_SWITCH_MIN_LENGTH_FLOOR,
    ENGINE_SWITCH_GRACE_SECONDS,
    PAUSE_DELAY_MAXIMUM_SECONDS,
    PAUSE_DELAY_MINIMUM_SECONDS,
    KeySwitchEngine,
    CorrectionPlan,
)
from keyswitch.history import HistoryStore
from keyswitch.indicator import layout_label
from keyswitch.layouts import LayoutPair
from keyswitch.settings_diagnostics import _LOGGABLE_STRING_MAX_CHARACTERS
from keyswitch.x11_backend import BackendProbe, KeyEvent

# -- fixture keycodes for the X11 keys these tests press directly -----------
# The engine dispatches on `KeyEvent.key_name`, not on the numeric keycode, so
# a keycode here only has to (a) match the real X11 code for well-known keys
# where a test cares about realism and (b) stay consistent between a press and
# the release or trigger lookup that must recognize it again.
BACKSPACE_KEYCODE = 22
ESCAPE_KEYCODE = 9
RETURN_KEYCODE = 36
LEFT_ARROW_KEYCODE = 100
LEFT_ARROW_KEYCODE_ALT = 113
CONTROL_L_KEYCODE = 37
ALT_L_KEYCODE = 64
UNDO_Z_KEYCODE = 52
LETTER_A_KEYCODE = 38
LETTER_X_KEYCODE = 99
BOUNDARY_KEYCODE = 65  # the physical Space key; boundary_event's default
DIGIT_TWO_KEYCODE = 11  # the physical "2" key: "2" unshifted, a quote shifted
PAUSE_KEYCODE = 127  # the physical Pause key; press_pause's default
TYPE_WORD_START_KEYCODE = 30  # type_word's default starting synthetic keycode
EARLY_UNDO_KEYCODE = 200  # schedule_undo's keycode, matched by the "z" release
IMPOSSIBLE_TRIGGER_KEYCODE = 999  # never matches a real release; forces a guard
REPLACED_TRIGGER_KEYCODE = 128
EXCLUDED_APP_TRIGGER_KEYCODE = 129

# Beyond a first typed word (which starts at TYPE_WORD_START_KEYCODE), each of
# these is just the next free synthetic keycode a test reaches for so a second
# batch of key events does not collide with keys still held from an earlier
# one. The exact value carries no meaning beyond distinguishing the event.
KEYCODE_33 = 33
KEYCODE_34 = 34
KEYCODE_35 = 35
KEYCODE_40 = 40
KEYCODE_41 = 41
KEYCODE_43 = 43
KEYCODE_44 = 44
KEYCODE_50 = 50
KEYCODE_53 = 53
KEYCODE_54 = 54
KEYCODE_55 = 55
KEYCODE_60 = 60
KEYCODE_70 = 70
KEYCODE_80 = 80
KEYCODE_90 = 90

# `self.engine._pressed`/`_modifier_keycodes` bookkeeping fixtures that are
# not built from a real KeyEvent, so they get their own names instead of the
# generic KEYCODE_* pool above.
STALE_MODIFIER_KEYCODE = 99
ACTIVELY_PRESSED_KEYCODE = 50
HELD_MODIFIER_KEYCODE = 60

# -- fixture timestamps -------------------------------------------------
# KeyEvent.timestamp is never read by the engine; these only satisfy the
# dataclass field, so a name just needs to say which fixture event it is.
BOUNDARY_EVENT_TIMESTAMP = 1000
QUOTE_EVENT_TIMESTAMP = 500
PLAIN_KEY_TIMESTAMP = 700
UNDO_CONTROL_DOWN_TIMESTAMP = 3000
UNDO_ALT_DOWN_TIMESTAMP = 3001
UNDO_Z_DOWN_TIMESTAMP = 3002
UNDO_Z_UP_TIMESTAMP = 3003
UNDO_ALT_UP_TIMESTAMP = 3004
UNDO_CONTROL_UP_TIMESTAMP = 3005
PROMPT_ENTER_TIMESTAMP = 4000
PROMPT_ESCAPE_TIMESTAMP = 4001
PROMPT_ENTER_RELEASE_TIMESTAMP = 4004
PROMPT_ENTER_SHORTCUT_TIMESTAMP = 4005
MODIFIER_SHORTCUT_EVENT_TIMESTAMP = 800
EXCLUDED_APP_SHORTCUT_EVENT_TIMESTAMP = 900

# -- fixture window ids ---------------------------------------------------
SECOND_WINDOW_ID = 2
THIRD_WINDOW_ID = 3
FOURTH_WINDOW_ID = 4
OWN_WINDOW_ID = 9

# -- test-pinned expectations, grouped by the test that pins them ---------
INJECTIONS_AFTER_QUOTE_PAUSE = 2
SYMBOL_AND_WORD_STROKE_COUNT = 6
REOPENED_WORD_CHARACTER_COUNT = 6
INJECTIONS_AFTER_REOPENED_PAUSE = 2
REPLAYED_STROKE_COUNT = 6
REVERSED_CORRECTION_STROKE_COUNT = 6
DIGIT_WORD_STROKE_COUNT = 3
DISCARDED_WORD_LENGTH = 2
EXPIRED_PROMPT_OFFSET_SECONDS = 0.1
PROMPT_DEADLINE_OFFSET_SECONDS = 5
INJECTIONS_AFTER_REPEATED_REJECTION = 2
TOGGLE_REPEAT_COUNT = 3
INJECTIONS_AFTER_TOGGLES = 4
HELD_KEYS_DURING_INJECTION = 2
LATE_KEYS_DURING_INJECTION = 2
PAUSE_DELAY_OVERRIDE_SECONDS = 0.5
JUST_BEFORE_PAUSE_DELAY_SECONDS = 0.49
OVERSIZED_PAUSE_DELAY_SECONDS = 50
UNDERSIZED_PAUSE_DELAY_SECONDS = 0.01
STALE_PRESS_AGE_SECONDS = 30.0
FIRST_PRUNE_CHECK_OFFSET_SECONDS = 2.0
SECOND_PRUNE_CHECK_OFFSET_SECONDS = 2.1
MODIFIER_DEFERRAL_CHECK_OFFSET_SECONDS = 2.2
PENDING_CORRECTION_CHECK_OFFSET_SECONDS = 2.3
SUCCESSFUL_CORRECTION_CHECK_OFFSET_SECONDS = 2.4
IDLE_MS_LOWER_BOUND = 2000
EARLY_SWITCH_PREFIX_STROKE_COUNT = 4
EARLY_SWITCH_WORD_LENGTH = 6
INJECTIONS_AFTER_LATE_STROKE = 2
LATE_STROKE_INJECTION_LENGTH = 5
INJECTIONS_AFTER_FAILED_LATE_STROKE = 3
EARLY_SWITCH_STALE_AGE_SECONDS = 5.0
INVALID_SOURCE_GROUP = 5
OVERSIZED_EARLY_SWITCH_MIN_LENGTH = 40
INJECTIONS_AFTER_OVERRIDE = 2
ROLLOVER_INJECTION_STROKE_COUNT = 5
DROPPED_WORD_LENGTH = 3
INJECTIONS_AFTER_UNDO_REVERT = 2
INJECTIONS_AFTER_SECOND_EARLY_SWITCH = 3
REQUIRED_CONFIRMATIONS = 2
LEARNING_PROMPT_REQUIRED_CONFIRMATIONS = 5
THRESHOLD_CONFIRMATIONS = 3
CONFIDENCE_OVERRIDE = 3.5
OVERSIZED_HOTKEY_CHARACTERS = 100
HOTKEY_CHANGE_INDEX = 2
MINIMUM_LENGTH_OVERRIDE = 5


class FakeBackend:
    def __init__(self) -> None:
        self.injections: list[tuple[tuple[KeyEvent, ...], int, KeyEvent | None]] = []
        self.late: list[tuple[KeyEvent, ...]] = []
        self.hold_calls = 0
        self.held_count = 0
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

    def hold_input(self) -> None:
        self.hold_calls += 1

    def release_input(self) -> int:
        return 0

    def complete_action(self, deliver: bool) -> int:
        return 0

    def inject_correction(
        self,
        strokes: Iterable[KeyEvent],
        target_group: int,
        boundary: KeyEvent | None,
        source_group: int | None = None,
        late: Sequence[KeyEvent] = (),
        trailing: Sequence[KeyEvent] = (),
    ) -> int:
        self.injections.append((tuple(strokes), target_group, boundary))
        self.late.append(tuple(late))
        self.group = target_group
        return self.held_count

    def set_key_filter(
        self, predicate: Callable[[KeyEvent], KeyDisposition] | None
    ) -> None:
        self.key_filter: Callable[[KeyEvent], KeyDisposition] | None = predicate

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


def boundary_event(pressed: bool, keycode: int = BOUNDARY_KEYCODE, group: int = 0) -> KeyEvent:
    return KeyEvent(pressed, keycode, "space", " ", (" ", " "), group, 0, BOUNDARY_EVENT_TIMESTAMP)


def quote_event(group: int = 1, keycode: int = DIGIT_TWO_KEYCODE) -> KeyEvent:
    """Shift+2: "@" in the US layout, a quote in the Russian one."""

    return KeyEvent(True, keycode, "quotedbl", ('@', '"')[group], ('@', '"'), group, 1, QUOTE_EVENT_TIMESTAMP)


def plain_key(name: str, keycode: int, group: int, pressed: bool = True) -> KeyEvent:
    return KeyEvent(pressed, keycode, name, "", ("", ""), group, 0, PLAIN_KEY_TIMESTAMP)


def digit_event(digit: str, group: int, keycode: int = DIGIT_TWO_KEYCODE) -> KeyEvent:
    """A digit row key: the same character in both layouts."""

    return KeyEvent(True, keycode, digit, digit, (digit, digit), group, 0, keycode)


class EngineBehaviourTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        root = Path(self.temporary.name)
        self.settings = SettingsStore(root / "config.json")
        # Exercise legacy prefix/rule behaviour separately from the learned
        # context-policy matrix, which explicitly enables assist mode.
        self.settings.set("detection.context_policy", "off")
        self.settings.set("detection.early_switch", False)
        self.settings.set("diagnostics.technical_logging", True)
        self.history = HistoryStore(root / "history.jsonl")
        self.backend = FakeBackend()
        self.engine = KeySwitchEngine(self.settings, self.history, self.backend)
        self.pair = LayoutPair()

    def tearDown(self) -> None:
        self.temporary.cleanup()

    # -- helpers ---------------------------------------------------------
    def type_word(self, text: str, group: int = 0, start: int = TYPE_WORD_START_KEYCODE) -> None:
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

    def press_pause(self, keycode: int = PAUSE_KEYCODE) -> None:
        self.release_keys()
        self.engine._schedule_manual_conversion(keycode)
        self.engine._handle(
            plain_key("Pause", keycode, self.engine.snapshot.current_group, pressed=False)
        )

    def release_keys(self) -> None:
        for keycode in tuple(self.engine._pressed):
            self.engine._handle(plain_key("a", keycode, self.backend.group, pressed=False))

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
            KeyEvent(True, CONTROL_L_KEYCODE, "Control_L", "", ("", ""), group, 0, UNDO_CONTROL_DOWN_TIMESTAMP),
            KeyEvent(True, ALT_L_KEYCODE, "Alt_L", "", ("", ""), group, control, UNDO_ALT_DOWN_TIMESTAMP),
            KeyEvent(True, UNDO_Z_KEYCODE, "z", "", ("z", "я"), group, control | alt, UNDO_Z_DOWN_TIMESTAMP),
            KeyEvent(False, UNDO_Z_KEYCODE, "z", "", ("z", "я"), group, control | alt, UNDO_Z_UP_TIMESTAMP),
            KeyEvent(False, ALT_L_KEYCODE, "Alt_L", "", ("", ""), group, control | alt, UNDO_ALT_UP_TIMESTAMP),
            KeyEvent(False, CONTROL_L_KEYCODE, "Control_L", "", ("", ""), group, control, UNDO_CONTROL_UP_TIMESTAMP),
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
            self.engine._handle(plain_key("BackSpace", BACKSPACE_KEYCODE, 1))
            self.assertEqual(self.engine._symbol_strokes, [])
            quote = quote_event()
            self.engine._handle(quote)
            self.press_pause()
        self.assertEqual(len(self.backend.injections), INJECTIONS_AFTER_QUOTE_PAUSE)
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
        self.assertEqual(len(strokes), SYMBOL_AND_WORD_STROKE_COUNT)
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

    def test_backspace_over_the_boundary_reopens_the_word_and_navigation_makes_it_stale(self) -> None:
        # Pause on the reopened word reverses the correction; with learning on
        # that would also record a rejection and block the second conversion.
        self.settings.set("detection.learning", False)
        self.correct_hello()
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._handle(plain_key("BackSpace", BACKSPACE_KEYCODE, 1))
        self.assertTrue(self.engine._last_committed_stale)
        self.assertEqual(self.engine.snapshot.current_word, "привет")
        self.assertEqual(self.engine._source_group, 1)
        reopened = next(
            event
            for event in self.technical_events(logs.output)
            if event["event"] == "committed_word_reopened"
        )
        self.assertEqual(reopened["characters"], REOPENED_WORD_CHARACTER_COUNT)
        self.assertEqual(reopened["boundary"], "space")
        # The reopened word is the current word again, so Pause converts it back.
        self.press_pause()
        self.assertEqual(len(self.backend.injections), INJECTIONS_AFTER_REOPENED_PAUSE)
        self.assertEqual(self.backend.group, 0)

        self.engine._manual_layout_group = None
        self.correct_hello()
        self.engine._handle(plain_key("Left", LEFT_ARROW_KEYCODE, 1))
        self.assertTrue(self.engine._last_committed_stale)
        # A moved caret leaves nothing to reopen: the Backspace lands elsewhere.
        self.engine._handle(plain_key("BackSpace", BACKSPACE_KEYCODE, 1))
        self.assertEqual(self.engine._strokes, [])

    def test_backspace_after_a_second_boundary_does_not_reopen_the_word(self) -> None:
        self.correct_hello()
        self.press_space(1)
        self.engine._handle(plain_key("BackSpace", BACKSPACE_KEYCODE, 1))
        self.assertEqual(self.engine._strokes, [])
        self.assertTrue(self.engine._last_committed_stale)
        self.press_pause()
        self.assertEqual(len(self.backend.injections), 1)

    def test_pause_without_any_word_reports_nothing_to_convert(self) -> None:
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.press_pause()
        self.assertEqual(self.engine.snapshot.last_action, "Нет слова для ручного преобразования")
        self.assertEqual(self.backend.injections, [])
        impossible = next(
            event
            for event in self.technical_events(logs.output)
            if event["event"] == "manual_conversion_impossible"
        )
        self.assertEqual(impossible["reason"], "no_alternate_layout")
        self.assertEqual(impossible["current_group"], -1)

    def test_typing_into_an_injection_is_counted_in_the_log(self) -> None:
        self.settings.set("detection.respect_manual_layout", False)
        self.type_word("ghbdtn")
        inject = self.backend.inject_correction

        def typing_user(*arguments: object, **keywords: object) -> int:
            # The user keeps typing while the correction is being injected.
            self.engine.enqueue(letter_event("x", LETTER_X_KEYCODE, 0, self.pair))
            return inject(*arguments, **keywords)  # type: ignore[arg-type]

        with (
            patch.object(self.backend, "inject_correction", side_effect=typing_user),
            self.assertLogs("keyswitch.engine", level="INFO") as logs,
        ):
            self.press_space(0)
        applied = next(
            event
            for event in self.technical_events(logs.output)
            if event["event"] == "correction_applied"
        )
        self.assertEqual(applied["keys_during_injection"], 1)
        self.assertEqual(applied["queued_events"], 1)
        self.assertEqual(applied["replayed_strokes"], REPLAYED_STROKE_COUNT)
        self.assertTrue(applied["boundary_replayed"])

        # A failed injection reports the same count.
        self.backend.group = 0
        self.type_word("ghbdtn", start=KEYCODE_60)
        with (
            patch.object(
                self.backend, "inject_correction", side_effect=RuntimeError("boom")
            ),
            self.assertLogs("keyswitch.engine", level="INFO") as logs,
        ):
            self.press_space(0)
        failed = next(
            event
            for event in self.technical_events(logs.output)
            if event["event"] == "correction_failed"
        )
        self.assertEqual(failed["keys_during_injection"], 0)

    def test_a_scheduled_conversion_that_never_runs_is_logged(self) -> None:
        self.type_word("ghbdtn")
        self.engine._schedule_manual_conversion(PAUSE_KEYCODE)
        self.assertIsNotNone(self.engine._pending)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._handle(
                KeyEvent(True, LETTER_A_KEYCODE, "a", "a", ("a", "ф"), 0, CONTROL_MASK, MODIFIER_SHORTCUT_EVENT_TIMESTAMP)
            )
        self.assertIsNone(self.engine._pending)
        self.assertEqual(self.backend.injections, [])
        dropped = next(
            event
            for event in self.technical_events(logs.output)
            if event["event"] == "pending_correction_dropped"
        )
        self.assertEqual(dropped["reason"], "modifier_shortcut")
        self.assertEqual((dropped["original"], dropped["replacement"]), ("ghbdtn", "привет"))
        self.assertEqual(dropped["mode"], "manual")

        # A second Pause replaces an unfinished plan; the first one says so.
        self.type_word("ghbdtn")
        self.engine._schedule_manual_conversion(PAUSE_KEYCODE)
        self.type_word("ghbdtn", start=KEYCODE_60)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._schedule_manual_conversion(REPLACED_TRIGGER_KEYCODE)
        self.assertEqual(
            next(
                event["reason"]
                for event in self.technical_events(logs.output)
                if event["event"] == "pending_correction_dropped"
            ),
            "replaced_by_manual_conversion",
        )

        # Text of an excluded application never reaches the log.
        self.settings.set("exclusions.applications", ["TestEditor"])
        self.type_word("ghbdtn", start=KEYCODE_90)
        self.engine._schedule_manual_conversion(EXCLUDED_APP_TRIGGER_KEYCODE)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._handle(
                KeyEvent(True, LETTER_A_KEYCODE, "a", "a", ("a", "ф"), 0, CONTROL_MASK, EXCLUDED_APP_SHORTCUT_EVENT_TIMESTAMP)
            )
        redacted = next(
            event
            for event in self.technical_events(logs.output)
            if event["event"] == "pending_correction_dropped"
        )
        self.assertEqual(
            (redacted["original"], redacted["replacement"]),
            ("<redacted>", "<redacted>"),
        )
        self.settings.set("exclusions.applications", [])

    def test_pause_right_after_a_correction_converts_the_word_back(self) -> None:
        self.correct_hello()
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.press_pause()
        strokes, target, boundary = self.backend.injections[-1]
        self.assertEqual(len(strokes), REVERSED_CORRECTION_STROKE_COUNT)
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
        self.assertEqual(len(strokes), DIGIT_WORD_STROKE_COUNT)
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
                KeyEvent(True, LETTER_A_KEYCODE, "a", "a", ("a", "ф"), 1, CONTROL_MASK, MODIFIER_SHORTCUT_EVENT_TIMESTAMP)
            )
        self.assertEqual(self.engine._strokes, [])
        discarded = next(
            event
            for event in self.technical_events(logs.output)
            if event["event"] == "word_discarded"
        )
        self.assertEqual(discarded["reason"], "modifier_shortcut")
        self.assertEqual(discarded["original"], "зь")
        self.assertEqual(discarded["length"], DISCARDED_WORD_LENGTH)

        # Navigation keys still drop the word, and say so.
        self.type_word("зь", group=1)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._handle(plain_key("Left", LEFT_ARROW_KEYCODE, 1))
        self.assertEqual(
            next(
                event["reason"]
                for event in self.technical_events(logs.output)
                if event["event"] == "word_discarded"
            ),
            "navigation",
        )

        # A leading digit stays in the token for manual conversion and code protection.
        self.engine._handle(quote_event())
        self.engine._handle(digit_event("2", 1))
        self.assertEqual(len(self.engine._symbol_strokes), 1)
        self.assertEqual(self.engine.snapshot.current_word, "2")

    def test_the_prompt_answer_is_kept_away_from_the_window(self) -> None:
        self.settings.set("detection.correct_on_enter", False)
        enter = KeyEvent(True, RETURN_KEYCODE, "Return", "\r", ("\r", "\r"), 1, 0, PROMPT_ENTER_TIMESTAMP)
        escape = KeyEvent(True, ESCAPE_KEYCODE, "Escape", "", ("", ""), 1, 0, PROMPT_ESCAPE_TIMESTAMP)
        letter = letter_event("a", LETTER_A_KEYCODE, 1, self.pair)

        # No prompt: every key belongs to the application.
        self.assertFalse(self.engine.consumes_key(enter))

        self.type_word("qwerty")
        self.press_pause()
        self.assertIsNotNone(self.engine.learning_prompt)

        # While the prompt is shown Enter and Esc answer it, and only them.
        self.assertTrue(self.engine.consumes_key(enter))
        self.assertTrue(self.engine.consumes_key(escape))
        self.assertFalse(self.engine.consumes_key(letter))
        self.assertFalse(
            self.engine.consumes_key(
                KeyEvent(False, RETURN_KEYCODE, "Return", "\r", ("\r", "\r"), 1, 0, PROMPT_ENTER_RELEASE_TIMESTAMP)
            )
        )
        # A shortcut with Enter is the application's, not the prompt's.
        self.assertFalse(
            self.engine.consumes_key(
                KeyEvent(True, RETURN_KEYCODE, "Return", "\r", ("\r", "\r"), 1, CONTROL_MASK, PROMPT_ENTER_SHORTCUT_TIMESTAMP)
            )
        )

        # An expired prompt releases the key even before the sweep runs.
        self.engine._prompt_key_deadline = time.monotonic() - EXPIRED_PROMPT_OFFSET_SECONDS
        self.assertFalse(self.engine.consumes_key(enter))

        # Answering the prompt releases it too.
        self.engine._prompt_key_deadline = time.monotonic() + PROMPT_DEADLINE_OFFSET_SECONDS
        prompt = self.engine.learning_prompt
        assert prompt is not None
        self.engine.confirm_learning_prompt(prompt)
        self.assertFalse(self.engine.consumes_key(enter))
        self.assertIsNone(self.engine.learning_prompt)

    def test_the_backend_learns_the_filter_when_the_engine_runs(self) -> None:
        self.engine.start()
        try:
            # A bound method is a fresh object per lookup, so compare by value.
            self.assertEqual(self.backend.key_filter, self.engine.consumes_key)
        finally:
            self.engine.stop()
        self.assertIsNone(self.backend.key_filter)

    def test_pause_after_an_automatic_correction_rejects_it(self) -> None:
        self.settings.set("detection.respect_manual_layout", False)
        self.correct_hello()
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.press_pause()
        events = self.technical_events(logs.output)
        scheduled = next(e for e in events if e["event"] == "manual_conversion_scheduled")
        self.assertEqual(scheduled["reversal"], "automatic")
        self.assertFalse(scheduled["learnable"])
        rejection = next(e for e in events if e["event"] == "learning_rejection_recorded")
        self.assertEqual(
            (rejection["word"], rejection["source_group"], rejection["target_group"]),
            ("ghbdtn", 0, 1),
        )
        self.assertNotIn("learning_rule_recorded", [e["event"] for e in events])
        self.assertEqual(self.engine.learning.rejected_targets(0, "ghbdtn"), {1})
        self.assertEqual(self.engine.learning.rule_state(1, "привет"), (None, 0))
        self.assertIsNone(self.engine.learning_prompt)

        # The same word is left alone from now on.
        self.backend.group = 0
        self.type_word("ghbdtn")
        self.press_space(0)
        self.assertEqual(len(self.backend.injections), INJECTIONS_AFTER_REPEATED_REJECTION)

    def test_toggling_a_manual_conversion_is_not_a_confirmation(self) -> None:
        self.settings.set("detection.respect_manual_layout", False)
        self.type_word("qwerty")
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.press_pause()
        first = next(
            e for e in self.technical_events(logs.output)
            if e["event"] == "manual_conversion_scheduled"
        )
        self.assertIsNone(first["reversal"])
        # The conversion only offers the rule; nothing is written before Enter.
        self.assertEqual(self.engine.learning.rule_state(0, "qwerty"), (None, 0))
        for _ in range(TOGGLE_REPEAT_COUNT):
            with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                self.press_pause()
            events = self.technical_events(logs.output)
            scheduled = next(e for e in events if e["event"] == "manual_conversion_scheduled")
            self.assertEqual(scheduled["reversal"], "manual")
            self.assertFalse(scheduled["learnable"])
            self.assertNotIn("learning_rule_recorded", [e["event"] for e in events])
        self.assertEqual(self.engine.learning.rule_state(0, "qwerty"), (None, 0))
        self.assertEqual(len(self.backend.injections), INJECTIONS_AFTER_TOGGLES)

    def test_a_boundary_that_takes_over_an_early_plan_says_so(self) -> None:
        self.settings.set("detection.early_switch", True)
        self.type_word("ghb")
        self.engine._handle(letter_event("d", KEYCODE_33, 0, self.pair))
        assert self.engine._pending is not None
        self.assertEqual(self.engine._pending.mode, "early")
        for index, character in enumerate("tn", start=KEYCODE_34):
            self.engine._handle(letter_event(character, index, 0, self.pair))
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._handle(boundary_event(True, group=0))
        dropped = next(
            e for e in self.technical_events(logs.output)
            if e["event"] == "pending_correction_dropped"
        )
        self.assertEqual((dropped["reason"], dropped["mode"]), ("superseded_by_boundary", "early"))
        assert self.engine._pending is not None
        self.assertEqual(self.engine._pending.mode, "boundary")

        # An undo hotkey pressed while a plan waits replaces it as well.
        self.engine._pending_trigger_keycode = -1
        self.engine._handle(boundary_event(False, group=0))
        self.release_keys()
        self.assertEqual(len(self.backend.injections), 1)
        self.type_word("руд", group=1, start=KEYCODE_40)
        self.engine._handle(letter_event("д", KEYCODE_43, 1, self.pair))
        assert self.engine._pending is not None
        self.assertEqual(self.engine._pending.mode, "early")
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._schedule_undo(UNDO_Z_KEYCODE)
        self.assertEqual(
            next(
                e["reason"] for e in self.technical_events(logs.output)
                if e["event"] == "pending_correction_dropped"
            ),
            "replaced_by_undo",
        )

    def test_keys_typed_into_a_correction_are_deleted_and_typed_again(self) -> None:
        self.settings.set("detection.respect_manual_layout", False)
        self.backend.held_count = HELD_KEYS_DURING_INJECTION
        self.type_word("ghbdtn")
        self.engine._handle(boundary_event(True, group=0))
        assert self.engine._pending is not None
        # Rollover: the next word starts before the space is released.
        rollover = letter_event("d", KEYCODE_40, 0, self.pair)
        self.engine._handle(rollover)
        self.engine._handle(replace(rollover, pressed=False))
        # Queued: more keys arrived before the engine got to them.
        queued = letter_event("t", KEYCODE_41, 0, self.pair)
        self.engine._events.put_nowait(queued)
        self.engine._events.put_nowait(
            KeyEvent(False, KEYCODE_41, "t", "t", ("t", "е"), 0, 0, KEYCODE_41)
        )
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._handle(boundary_event(False, group=0))
        self.assertEqual(self.backend.hold_calls, 1)
        self.assertEqual(self.backend.late, [(rollover, queued)])
        self.assertEqual(self.engine._strokes, [])
        # Release bookkeeping stays queued; typed keys come back through the hook.
        remaining = self.engine._events.get_nowait()
        assert isinstance(remaining, KeyEvent)
        self.assertEqual((remaining.key_name, remaining.pressed), ("t", False))
        self.assertTrue(self.engine._events.empty())
        applied = next(
            e for e in self.technical_events(logs.output) if e["event"] == "correction_applied"
        )
        self.assertEqual((applied["late_keys"], applied["held_keys"]), (LATE_KEYS_DURING_INJECTION, HELD_KEYS_DURING_INJECTION))

    def test_late_input_is_left_alone_when_it_is_not_plain_text(self) -> None:
        self.settings.set("detection.respect_manual_layout", False)
        self.type_word("ghbdtn")
        self.engine._handle(boundary_event(True, group=0))
        rollover = letter_event("d", KEYCODE_40, 0, self.pair)
        self.engine._handle(rollover)
        self.engine._handle(replace(rollover, pressed=False))
        enter = KeyEvent(True, RETURN_KEYCODE, "Return", "\r", ("\r", "\r"), 0, 0, KEYCODE_41)
        self.engine._events.put_nowait(enter)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._handle(boundary_event(False, group=0))
        self.assertEqual(self.backend.late, [])
        self.assertEqual(self.engine._strokes, [rollover])
        self.assertIs(self.engine._events.get_nowait(), enter)
        applied = next(
            e for e in self.technical_events(logs.output) if e["event"] == "correction_aborted"
        )
        self.assertEqual(applied["reason"], "unsafe_input_after_word")

        # A failed injection reports the late keys it was given.
        self.backend.group = 0
        self.engine._strokes = []
        self.type_word("ghbdtn", start=KEYCODE_60)
        self.engine._handle(boundary_event(True, group=0))
        self.engine._handle(letter_event("d", KEYCODE_70, 0, self.pair))
        self.engine._handle(replace(letter_event("d", KEYCODE_70, 0, self.pair), pressed=False))
        with (
            patch.object(self.backend, "inject_correction", side_effect=RuntimeError("boom")),
            self.assertLogs("keyswitch.engine", level="INFO") as logs,
        ):
            self.engine._handle(boundary_event(False, group=0))
        failed = next(
            e for e in self.technical_events(logs.output) if e["event"] == "correction_failed"
        )
        self.assertEqual(failed["late_keys"], 1)

    def test_layout_dependent_symbols_only_are_remembered(self) -> None:
        dot = KeyEvent(True, KEYCODE_60, "period", ".", (".", "."), 0, 0, 1)
        self.assertFalse(self.engine._layout_dependent(dot))
        self.engine._handle(dot)
        self.assertEqual(self.engine._symbol_strokes, [])
        self.engine._handle(quote_event(group=0))
        self.assertEqual(len(self.engine._symbol_strokes), 1)
        self.press_space(0)
        self.assertEqual(self.engine._symbol_strokes, [])
        self.engine._handle(quote_event(group=0))
        self.engine._handle(plain_key("Left", LEFT_ARROW_KEYCODE, 0))
        self.assertEqual(self.engine._symbol_strokes, [])

    # -- pause timing ----------------------------------------------------
    def test_pause_delay_setting_controls_the_timer(self) -> None:
        self.settings.set("detection.pause_delay_seconds", PAUSE_DELAY_OVERRIDE_SECONDS)
        self.assertEqual(self.engine._loop_timeout(), PAUSE_DELAY_OVERRIDE_SECONDS)
        self.type_word("ghbdtn")
        last_input = self.engine._last_word_input_at
        assert last_input is not None
        timeout = self.engine._loop_timeout()
        self.assertGreater(timeout, 0.0)
        self.assertLessEqual(timeout, PAUSE_DELAY_OVERRIDE_SECONDS)
        self.engine._maybe_correct_after_pause(now=last_input + JUST_BEFORE_PAUSE_DELAY_SECONDS)
        self.assertEqual(self.backend.injections, [])
        self.engine._maybe_correct_after_pause(now=last_input + PAUSE_DELAY_OVERRIDE_SECONDS)
        self.assertEqual(len(self.backend.injections), 1)
        self.assertEqual(self.engine._loop_timeout(), PAUSE_DELAY_OVERRIDE_SECONDS)

        self.settings.set("detection.pause_delay_seconds", "soon")
        self.assertEqual(self.engine._pause_delay(), DEFAULT_PAUSE_DELAY_SECONDS)
        self.settings.set("detection.pause_delay_seconds", OVERSIZED_PAUSE_DELAY_SECONDS)
        self.assertEqual(self.engine._pause_delay(), PAUSE_DELAY_MAXIMUM_SECONDS)
        self.settings.set("detection.pause_delay_seconds", UNDERSIZED_PAUSE_DELAY_SECONDS)
        self.assertEqual(self.engine._pause_delay(), PAUSE_DELAY_MINIMUM_SECONDS)

    def test_stale_presses_are_pruned_and_deferrals_logged_once(self) -> None:
        self.type_word("ghbdtn")
        now = time.monotonic()
        self.engine._pressed.update({STALE_MODIFIER_KEYCODE, ACTIVELY_PRESSED_KEYCODE})
        self.engine._modifier_keycodes.add(STALE_MODIFIER_KEYCODE)
        self.engine._pressed_since[STALE_MODIFIER_KEYCODE] = now - STALE_PRESS_AGE_SECONDS
        self.engine._pressed_since[ACTIVELY_PRESSED_KEYCODE] = now
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._maybe_correct_after_pause(now=now + FIRST_PRUNE_CHECK_OFFSET_SECONDS)
            self.engine._maybe_correct_after_pause(now=now + SECOND_PRUNE_CHECK_OFFSET_SECONDS)
        events = self.technical_events(logs.output)
        self.assertEqual(
            [event["event"] for event in events],
            ["stale_presses_pruned", "pause_correction_deferred"],
        )
        self.assertEqual(events[0]["keycodes"], [STALE_MODIFIER_KEYCODE])
        self.assertEqual(events[1]["reason"], "keys_pressed")
        self.assertEqual(events[1]["pressed_keycodes"], [ACTIVELY_PRESSED_KEYCODE])
        self.assertGreaterEqual(int(str(events[1]["idle_ms"])), IDLE_MS_LOWER_BOUND)
        self.assertEqual(self.engine._modifier_keycodes, set())
        self.assertEqual(self.backend.injections, [])

        self.engine._pressed.clear()
        self.engine._pressed_since.clear()
        self.engine._modifier_keycodes.add(HELD_MODIFIER_KEYCODE)
        self.engine._pause_deferral_logged = False
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._maybe_correct_after_pause(now=now + MODIFIER_DEFERRAL_CHECK_OFFSET_SECONDS)
        self.assertEqual(self.technical_events(logs.output)[0]["reason"], "modifiers_pressed")
        self.engine._modifier_keycodes.clear()
        self.engine._pending = CorrectionPlan((), None, 0, 1, "", "", 0.0, "", False)
        self.engine._pause_deferral_logged = False
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._maybe_correct_after_pause(now=now + PENDING_CORRECTION_CHECK_OFFSET_SECONDS)
        self.assertEqual(self.technical_events(logs.output)[0]["reason"], "correction_pending")
        self.engine._pending = None
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._maybe_correct_after_pause(now=now + SUCCESSFUL_CORRECTION_CHECK_OFFSET_SECONDS)
        self.assertEqual(len(self.backend.injections), 1)
        evaluation = next(
            event for event in self.technical_events(logs.output) if event["event"] == "word_evaluation"
        )
        self.assertEqual(evaluation["trigger"], "pause")
        self.assertGreaterEqual(int(str(evaluation["idle_ms"])), IDLE_MS_LOWER_BOUND)
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
        self.assertEqual((len(strokes), target, boundary), (EARLY_SWITCH_PREFIX_STROKE_COUNT, 1, None))
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
        self.assertEqual(evaluation["prefix_length"], EARLY_SWITCH_PREFIX_STROKE_COUNT)
        applied = next(event for event in events if event["event"] == "correction_applied")
        self.assertEqual(applied["mode"], "early")

        self.type_word("ет", group=1, start=KEYCODE_34)
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
        self.assertEqual(completed["word_length"], EARLY_SWITCH_WORD_LENGTH)
        evaluation = next(event for event in events if event["event"] == "word_evaluation")
        self.assertEqual(evaluation["early_switch_origin"], 0)

        self.engine._schedule_undo(EARLY_UNDO_KEYCODE)
        self.engine._handle(plain_key("z", EARLY_UNDO_KEYCODE, 1, pressed=False))
        strokes, target, boundary = self.backend.injections[-1]
        self.assertEqual((len(strokes), target), (EARLY_SWITCH_WORD_LENGTH, 0))
        self.assertIsNotNone(boundary)
        self.assertEqual(self.engine.snapshot.last_action, "привет → ghbdtn · ложное срабатывание запомнено")

    def test_late_stroke_after_early_switch_waits_for_release(self) -> None:
        self.settings.set("detection.respect_manual_layout", False)
        self.settings.set("detection.early_switch", True)
        self.type_word("ghbd")
        late = letter_event("t", KEYCODE_34, 0, self.pair)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._handle(late)
        self.assertEqual(len(self.backend.injections), 1)
        self.engine._handle(replace(late, pressed=False))
        self.assertEqual(len(self.backend.injections), INJECTIONS_AFTER_LATE_STROKE)
        self.assertEqual(len(self.backend.injections[-1][0]), LATE_STROKE_INJECTION_LENGTH)
        self.assertEqual(self.engine.snapshot.current_word, "приве")
        self.assertEqual(self.engine._source_group, 1)
        self.assertEqual(self.engine._strokes[-1].group, 1)
        converted = next(
            event for event in self.technical_events(logs.output) if event["event"] == "late_stroke_scheduled"
        )
        self.assertEqual((converted["source_group"], converted["target_group"]), (0, 1))

        with patch.object(self.backend, "inject_correction", side_effect=RuntimeError("xtest")):
            with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                event = letter_event("n", KEYCODE_35, 0, self.pair)
                self.engine._handle(event)
                self.engine._handle(replace(event, pressed=False))
        failed = next(
            event
            for event in self.technical_events(logs.output)
            if event["event"] == "correction_failed"
        )
        self.assertEqual(failed["error"], "xtest")
        self.assertEqual(self.engine.snapshot.current_word, "")
        self.assertIsNone(self.engine._early_switch_origin)

        self.engine._clear_word()
        self.type_word("ghbd", start=KEYCODE_40)
        self.assertEqual(len(self.backend.injections), INJECTIONS_AFTER_FAILED_LATE_STROKE)
        self.engine._early_switch_at = time.monotonic() - EARLY_SWITCH_STALE_AGE_SECONDS
        self.engine._handle(letter_event("t", KEYCODE_44, 0, self.pair))
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
        self.engine._pending_trigger_keycode = IMPOSSIBLE_TRIGGER_KEYCODE
        self.type_word("ghbd")
        self.assertEqual(self.backend.injections, [])
        self.engine._clear_word()

        self.engine._strokes = [letter_event(character, TYPE_WORD_START_KEYCODE, 0, self.pair) for character in "ghbd"]
        self.engine._source_group = INVALID_SOURCE_GROUP
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

        self.settings.set("detection.early_switch_min_length", EARLY_SWITCH_MIN_LENGTH_FLOOR)
        self.type_word("ghb", start=KEYCODE_50)
        self.assertEqual(len(self.backend.injections), 1)
        self.settings.set("detection.early_switch_min_length", "four")
        self.assertEqual(self.engine._early_switch_policy().minimum_length, DEFAULT_EARLY_SWITCH_MIN_LENGTH)
        self.settings.set("detection.early_switch_min_length", 1)
        self.assertEqual(self.engine._early_switch_policy().minimum_length, EARLY_SWITCH_MIN_LENGTH_FLOOR)
        self.settings.set("detection.early_switch_min_length", OVERSIZED_EARLY_SWITCH_MIN_LENGTH)
        self.assertEqual(self.engine._early_switch_policy().minimum_length, EARLY_SWITCH_MIN_LENGTH_CEILING)

    def test_boundary_detector_can_still_override_an_early_switch(self) -> None:
        self.settings.set("detection.early_switch", True)
        for _ in range(DEFAULT_LEARNING_CONFIRMATIONS):
            self.engine.learning.record_manual(1, "привет", 0)
        self.type_word("ghbd")
        self.type_word("ет", group=1, start=KEYCODE_34)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.press_space(1)
        self.assertEqual(len(self.backend.injections), INJECTIONS_AFTER_OVERRIDE)
        self.assertEqual(self.backend.injections[-1][1], 0)
        self.assertIsNone(self.engine._early_switch_origin)
        self.assertEqual(self.engine.snapshot.correction_count, 1)
        names = [event["event"] for event in self.technical_events(logs.output)]
        self.assertNotIn("early_switch_completed", names)

    def test_early_switch_without_history_still_counts(self) -> None:
        self.settings.set("detection.early_switch", True)
        self.settings.set("general.keep_history", False)
        self.type_word("ghbd")
        self.type_word("ет", group=1, start=KEYCODE_34)
        self.press_space(1)
        self.assertEqual(self.engine.snapshot.correction_count, 1)
        self.assertEqual(self.history.read(), [])

    def test_early_switch_waits_for_the_key_release_and_absorbs_rollover(self) -> None:
        self.settings.set("detection.early_switch", True)
        self.type_word("ghb")
        held = letter_event("d", KEYCODE_33, 0, self.pair)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._handle(held)
        self.assertEqual(self.backend.injections, [])
        assert self.engine._pending is not None
        self.assertEqual(self.engine._pending.mode, "early")
        names = [event["event"] for event in self.technical_events(logs.output)]
        self.assertIn("early_switch_scheduled", names)
        rollover = letter_event("t", KEYCODE_34, 0, self.pair)
        self.engine._handle(rollover)
        self.assertEqual(self.backend.injections, [])
        self.engine._handle(KeyEvent(False, KEYCODE_33, "d", "d", held.characters, 0, 0, KEYCODE_40))
        self.assertEqual(self.backend.injections, [])
        self.engine._handle(replace(rollover, pressed=False))
        self.assertEqual(len(self.backend.injections), 1)
        strokes, target, _boundary = self.backend.injections[0]
        self.assertEqual((len(strokes), target), (ROLLOVER_INJECTION_STROKE_COUNT, 1))
        self.assertEqual(self.engine.snapshot.current_word, "приве")
        self.assertEqual(self.engine._source_group, 1)
        self.assertEqual(self.engine._early_switch_origin, 0)
        self.engine._handle(KeyEvent(False, KEYCODE_34, "t", "t", rollover.characters, 1, 0, KEYCODE_41))
        self.assertEqual(len(self.backend.injections), 1)

    def test_early_switch_is_dropped_when_the_word_changes_before_release(self) -> None:
        self.settings.set("detection.early_switch", True)
        self.type_word("ghb")
        held = letter_event("d", KEYCODE_33, 0, self.pair)
        self.engine._handle(held)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._handle(plain_key("BackSpace", BACKSPACE_KEYCODE, 0))
            self.engine._handle(KeyEvent(False, KEYCODE_33, "d", "d", held.characters, 0, 0, KEYCODE_40))
        self.assertEqual(self.backend.injections, [])
        self.assertIsNone(self.engine._pending)
        self.assertIsNone(self.engine._early_switch_origin)
        dropped = next(
            event for event in self.technical_events(logs.output) if event["event"] == "pending_correction_dropped"
        )
        self.assertEqual(dropped["reason"], "backspace")

        self.engine._clear_word()
        self.type_word("ghb", start=KEYCODE_50)
        held = letter_event("d", KEYCODE_53, 0, self.pair)
        self.engine._handle(held)
        self.engine._handle(letter_event("t", KEYCODE_54, 0, self.pair))
        self.engine._handle(letter_event("n", KEYCODE_55, 0, self.pair))
        self.press_space(0)  # the boundary correction takes over the whole word
        self.release_keys()
        self.assertEqual(len(self.backend.injections), 1)
        strokes, target, boundary = self.backend.injections[0]
        self.assertEqual((len(strokes), target), (EARLY_SWITCH_WORD_LENGTH, 1))
        self.assertIsNotNone(boundary)
        self.engine._handle(KeyEvent(False, KEYCODE_53, "d", "d", held.characters, 1, 0, KEYCODE_60))
        self.assertEqual(len(self.backend.injections), 1)

    def test_backspacing_the_whole_prefix_forgets_the_early_switch(self) -> None:
        self.settings.set("detection.early_switch", True)
        self.type_word("ghbd")
        for _ in range(EARLY_SWITCH_PREFIX_STROKE_COUNT):
            self.engine._handle(plain_key("BackSpace", BACKSPACE_KEYCODE, 1))
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
        # Two confirmations make the intermediate state visible in the log.
        self.settings.set("detection.learning_confirmations", REQUIRED_CONFIRMATIONS)

        # No rule yet: the word is evaluated by the model alone.
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type_word("ghbdtn")
            self.press_space(0)
        learning = self.learning_field(logs.output, "ghbdtn")
        self.assertEqual(
            learning,
            {
                "enabled": True,
                "required_confirmations": REQUIRED_CONFIRMATIONS,
                "rule_target": None,
                "confirmations": 0,
                "forced_target": None,
                "rejected_targets": [],
            },
        )

        # A rule half-confirmed by an older version stays inactive on its own.
        self.engine.learning.record_manual(0, "qwerty", 1)
        self.assertEqual(self.engine.learning.rule_state(0, "qwerty"), (1, 1))

        # One manual conversion offers the rule again; it teaches nothing by itself.
        self.backend.group = 0
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type_word("qwerty")
            self.press_pause()
            self.assertEqual(self.engine.learning.rule_state(0, "qwerty"), (1, 1))
            self.assertIsNotNone(self.engine.learning_prompt)
            self.engine.dismiss_learning_prompt(reason="escape")
        self.assertNotIn(
            "learning_rule_recorded",
            [event["event"] for event in self.technical_events(logs.output)],
        )
        self.assertEqual(self.engine.learning.rule_state(0, "qwerty"), (1, 1))

        self.backend.group = 0
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type_word("qwerty")
            self.press_space(0)
        pending = self.learning_field(logs.output, "qwerty")
        self.assertEqual(pending["confirmations"], 1)
        self.assertEqual(pending["rule_target"], 1)
        self.assertIsNone(pending["forced_target"])

        # Enter on the prompt is what turns it into an active rule.
        self.backend.group = 0
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type_word("qwerty")
            self.press_pause()
            self.assertTrue(self.engine.confirm_learning_prompt())
        recorded = next(
            event
            for event in self.technical_events(logs.output)
            if event["event"] == "learning_rule_recorded"
        )
        self.assertEqual(
            (recorded["word"], recorded["confirmations"], recorded["active"]),
            ("qwerty", REQUIRED_CONFIRMATIONS, True),
        )
        self.assertEqual(recorded["required_confirmations"], REQUIRED_CONFIRMATIONS)

        self.backend.group = 0
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type_word("qwerty")
            self.press_space(0)
        forced = self.learning_field(logs.output, "qwerty")
        self.assertEqual(forced["forced_target"], 1)
        self.assertEqual(forced["confirmations"], REQUIRED_CONFIRMATIONS)
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
        self.backend.window = SECOND_WINDOW_ID
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
        self.backend.window = THIRD_WINDOW_ID
        self.backend.group = 1
        self.engine._poll_current_group()
        self.assertIsNone(self.engine._manual_layout_group)
        self.backend.window = FOURTH_WINDOW_ID
        self.backend.group = 0
        self.engine._poll_current_group()
        self.correct_hello()

    def test_own_window_layout_is_ignored(self) -> None:
        self.engine._poll_current_group()
        self.backend.own_window = True
        self.backend.isolated_layout = True
        self.backend.window = OWN_WINDOW_ID
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
        # Every poll repeats the observation; the log records it once.
        with self.assertNoLogs("keyswitch.engine", level="INFO"):
            self.engine._poll_current_group()
            self.engine._poll_current_group()
        # Back in the editor nothing happened: no focus change, no protection.
        self.backend.own_window = False
        self.backend.isolated_layout = False
        self.backend.window = 1
        self.backend.group = 0
        self.engine._poll_current_group()
        self.assertEqual(self.engine._focus_window, 1)
        self.assertFalse(self.engine._last_committed_stale)
        self.correct_hello()
        # A new visit to an own window is a new episode and is logged again.
        self.backend.own_window = True
        self.backend.isolated_layout = True
        self.backend.window = OWN_WINDOW_ID
        self.backend.group = 0
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._poll_current_group()
        self.assertIn(
            "layout_change_ignored",
            [e["event"] for e in self.technical_events(logs.output)],
        )

    def test_own_window_with_a_global_layout_still_sees_manual_switches(self) -> None:
        # X11: the layout is global, so a switch made while a KeySwitch window
        # (or the E2E's own entry) is focused is the user's own choice.
        self.engine._poll_current_group()
        self.backend.own_window = True
        self.backend.window = OWN_WINDOW_ID
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
            self.type_word("привет", group=1, start=KEYCODE_50)
            self.press_space(1)
        evaluation = next(
            event for event in self.technical_events(logs.output) if event["event"] == "word_evaluation"
        )
        self.assertIsNone(evaluation["skipped_reason"])

    def test_moving_to_another_window_drops_the_unfinished_word(self) -> None:
        self.correct_hello()
        self.assertFalse(self.engine._last_committed_stale)
        self.type_word("руд", group=1, start=KEYCODE_60)
        self.assertEqual(self.engine.snapshot.current_word, "руд")
        self.backend.window = SECOND_WINDOW_ID
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._poll_current_group()
        changed = next(
            event for event in self.technical_events(logs.output) if event["event"] == "focus_changed"
        )
        self.assertEqual((changed["previous_window"], changed["window"]), (1, SECOND_WINDOW_ID))
        self.assertEqual(changed["dropped_word_length"], DROPPED_WORD_LENGTH)
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
            self.engine._schedule_undo(EARLY_UNDO_KEYCODE)
            self.engine._handle(plain_key("z", EARLY_UNDO_KEYCODE, 1, pressed=False))
        strokes, target, boundary = self.backend.injections[-1]
        self.assertEqual((len(strokes), target, boundary), (EARLY_SWITCH_PREFIX_STROKE_COUNT, 0, None))
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
        self.assertEqual(applied["deleted_characters"], EARLY_SWITCH_PREFIX_STROKE_COUNT)
        self.assertFalse(applied["automatic"])
        # The rest of the word is neither switched early again nor corrected.
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type_word("tn", group=0, start=KEYCODE_34)
            self.press_space(0)
        self.assertEqual(len(self.backend.injections), INJECTIONS_AFTER_UNDO_REVERT)
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
        self.type_word("ghbd", start=KEYCODE_90)
        self.assertEqual(len(self.backend.injections), INJECTIONS_AFTER_SECOND_EARLY_SWITCH)
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
        self.type_word("ет", group=1, start=KEYCODE_34)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._handle(plain_key("Left", LEFT_ARROW_KEYCODE_ALT, 1))
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
        # Navigation invalidated the position, so undo must not delete elsewhere.
        before = len(self.backend.injections)
        self.engine._schedule_undo(EARLY_UNDO_KEYCODE)
        self.engine._handle(plain_key("z", EARLY_UNDO_KEYCODE, 1, pressed=False))
        self.assertEqual(len(self.backend.injections), before)

    def test_learning_is_not_offered_for_a_lone_letter_or_symbols(self) -> None:
        self.settings.set("detection.learning_confirmations", 1)
        self.engine._handle(boundary_event(True, group=1))
        self.engine._handle(boundary_event(False, group=1))
        self.type_word("б", group=1, start=KEYCODE_70)
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
        self.type_word("yj", group=0, start=KEYCODE_80)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.press_pause()
        self.assertEqual(self.engine.snapshot.last_action, "yj → но")
        self.assertEqual(self.engine.learning.counts(), (0, 0))
        self.assertIsNotNone(self.engine.learning_prompt)
        self.assertTrue(self.engine.confirm_learning_prompt())
        self.assertEqual(self.engine.snapshot.last_action, "yj → но · правило выучено")
        self.assertEqual(self.engine.learning.counts(), (1, 0))
        scheduled = next(
            event for event in self.technical_events(logs.output) if event["event"] == "manual_conversion_scheduled"
        )
        self.assertTrue(scheduled["learnable"])

    def test_learning_prompt_lifecycle_is_logged(self) -> None:
        self.settings.set("detection.learning_confirmations", LEARNING_PROMPT_REQUIRED_CONFIRMATIONS)

        def manual_conversion() -> None:
            self.type_word("ghbdtn")
            self.press_pause()
            self.assertIsNotNone(self.engine.learning_prompt)

        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            manual_conversion()
            self.engine._handle(plain_key("Escape", ESCAPE_KEYCODE, 1))
        names = [event["event"] for event in self.technical_events(logs.output)]
        self.assertIn("learning_prompt_shown", names)
        dismissed = next(
            event for event in self.technical_events(logs.output) if event["event"] == "learning_prompt_dismissed"
        )
        self.assertEqual(dismissed["reason"], "escape")

        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            manual_conversion()
            self.engine._handle(letter_event("a", LETTER_A_KEYCODE, 1, self.pair))
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
        self.assertEqual(confirmed["required_confirmations"], LEARNING_PROMPT_REQUIRED_CONFIRMATIONS)

    def test_only_enter_records_what_local_learning_keeps(self) -> None:
        """Every answer except Enter leaves the rules as they were.

        A manual conversion is a correction, not a lesson: the user may just
        carry on typing, click elsewhere or leave the prompt alone, and none of
        that is a decision about the word.
        """

        self.settings.set("detection.learning_confirmations", 1)

        def offer() -> None:
            self.backend.group = 0
            self.type_word("qwerty")
            self.press_pause()
            self.assertIsNotNone(self.engine.learning_prompt)
            self.assertEqual(self.engine.learning.rule_state(0, "qwerty"), (None, 0))

        offer()
        self.engine._handle(plain_key("Escape", ESCAPE_KEYCODE, 1))
        self.assertEqual(self.engine.learning.counts(), (0, 0))

        offer()
        self.engine._handle(letter_event("a", LETTER_A_KEYCODE, 1, self.pair))
        self.assertIsNone(self.engine.learning_prompt)
        self.assertEqual(self.engine.learning.counts(), (0, 0))
        self.engine._clear_word()

        offer()
        self.engine._handle(plain_key("Pointer", 1, 0))
        self.assertIsNone(self.engine.learning_prompt)
        self.assertEqual(self.engine.learning.counts(), (0, 0))

        offer()
        deadline = self.engine._learning_prompt_deadline
        assert deadline is not None
        self.assertTrue(self.engine._expire_learning_prompt(now=deadline + 1.0))
        self.assertEqual(self.engine.learning.counts(), (0, 0))
        self.engine._clear_word()

        offer()
        self.assertTrue(self.engine.confirm_learning_prompt())
        self.assertEqual(self.engine.learning.rule_state(0, "qwerty"), (1, 1))
        self.assertEqual(self.engine.learning.forced_target(0, "qwerty", 1), 1)
        self.assertEqual(self.engine.learning.counts(), (1, 0))
        self.assertEqual(
            self.engine.snapshot.last_action, "qwerty → йцукен · правило выучено"
        )

    def test_a_word_that_already_has_a_rule_is_not_offered_again(self) -> None:
        """Nothing left to learn: the conversion just happens, without a prompt."""

        self.settings.set("detection.learning_confirmations", 1)
        self.backend.group = 0
        self.type_word("qwerty")
        self.press_pause()
        self.assertTrue(self.engine.confirm_learning_prompt())
        self.assertEqual(self.engine.learning.rule_state(0, "qwerty"), (1, 1))

        self.settings.set("detection.respect_manual_layout", False)
        self.backend.group = 0
        self.type_word("qwerty")
        self.press_pause()
        self.assertIsNone(self.engine.learning_prompt)
        self.assertEqual(self.engine.learning.rule_state(0, "qwerty"), (1, 1))

    def test_enter_reaches_the_threshold_whatever_it_is(self) -> None:
        """A rule costs one Enter; the threshold is what half-confirmed rules need."""

        self.settings.set("detection.learning_confirmations", THRESHOLD_CONFIRMATIONS)
        self.backend.group = 0
        self.type_word("qwerty")
        self.press_pause()
        self.assertTrue(self.engine.confirm_learning_prompt())
        self.assertEqual(
            self.engine.snapshot.last_action, "qwerty → йцукен · правило выучено"
        )
        self.assertEqual(self.engine.learning.rule_state(0, "qwerty"), (1, THRESHOLD_CONFIRMATIONS))
        self.assertEqual(self.engine.learning.forced_target(0, "qwerty", THRESHOLD_CONFIRMATIONS), 1)

    def test_setting_changes_are_logged_with_loggable_values(self) -> None:
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.settings.set("detection.confidence", CONFIDENCE_OVERRIDE)
            self.settings.set("exclusions.applications", ["one"])
            self.settings.set("hotkeys.undo", "x" * OVERSIZED_HOTKEY_CHARACTERS)
        changes = [
            event for event in self.technical_events(logs.output) if event["event"] == "setting_changed"
        ]
        self.assertEqual(changes[0]["value"], CONFIDENCE_OVERRIDE)
        self.assertEqual(changes[1]["value"], {"type": "list", "items": 1})
        self.assertEqual(len(str(changes[HOTKEY_CHANGE_INDEX]["value"])), _LOGGABLE_STRING_MAX_CHARACTERS)
        self.assertTrue(all(event["operation"] == "set" for event in changes))

    def test_session_event_lists_the_new_settings(self) -> None:
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._technical_session_event("probe")
        session = self.technical_events(logs.output)[0]
        settings = session["settings"]
        assert isinstance(settings, dict)
        overrides = settings["overrides"]
        assert isinstance(overrides, dict)
        self.assertNotIn("detection.pause_delay_seconds", overrides)
        # The early switch is off by default, so turning it off is no longer an override.
        self.assertNotIn("detection.early_switch", overrides)
        self.settings.set("detection.early_switch", True)
        with self.assertLogs("keyswitch.engine", level="INFO") as switched:
            self.engine._technical_session_event("probe")
        enabled = self.technical_events(switched.output)[0]["settings"]
        assert isinstance(enabled, dict)
        overridden = enabled["overrides"]
        assert isinstance(overridden, dict)
        self.assertTrue(overridden["detection.early_switch"])
        self.assertNotIn("hotkeys.convert_last", overrides)
        self.assertNotIn("detection_settings", session)
        self.assertNotIn("hotkeys", session)

    def test_settings_delta_logs_reset_and_ignores_unknown_keys(self) -> None:
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.settings.set("detection.confidence", CONFIDENCE_OVERRIDE)
            self.settings.restore_default("detection.confidence")
        changes = [e for e in self.technical_events(logs.output) if e["event"] == "setting_changed"]
        self.assertEqual([e["operation"] for e in changes], ["set", "reset"])
        self.assertNotIn("value", changes[1])
        self.assertEqual(changes[1]["path"], "detection.confidence")
        with self.assertNoLogs("keyswitch.engine", level="INFO"):
            self.settings.set("future.private_value", "private", persist=False)

    def test_reload_logs_replacement_snapshot_and_disabling_logging_is_respected(self) -> None:
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._settings_changed("*", self.settings.snapshot())
        change = next(e for e in self.technical_events(logs.output) if e["event"] == "setting_changed")
        self.assertEqual(change["operation"], "snapshot")
        self.assertEqual(change["path"], "*")
        self.assertNotIn("value", change)
        with self.assertNoLogs("keyswitch.engine", level="INFO"):
            self.settings.set("diagnostics.technical_logging", False)
            self.settings.restore_default("detection.early_switch")
            self.settings.set("detection.minimum_length", MINIMUM_LENGTH_OVERRIDE)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.settings.set("diagnostics.technical_logging", True)
        session = next(e for e in self.technical_events(logs.output) if e["event"] == "technical_logging_enabled")
        settings = session["settings"]
        assert isinstance(settings, dict)
        overrides = settings["overrides"]
        assert isinstance(overrides, dict)
        self.assertNotIn("detection.early_switch", overrides)
        self.assertEqual(overrides["detection.minimum_length"], MINIMUM_LENGTH_OVERRIDE)


if __name__ == "__main__":
    unittest.main()
