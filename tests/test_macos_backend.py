"""Platform-independent verification of the macOS keyboard backend.

The backend reaches the system only through :class:`MacAPI`, so everything it
decides is checked here, on any platform, with a substitute in its place.
"""

from __future__ import annotations

import sys
import threading
import types
import unittest
from collections.abc import Callable
from unittest.mock import patch

from keyswitch.backend import (
    COMPLETED_ACTION_EVENT_COUNT,
    KeyDisposition,
    KeyEvent,
    LOCK_MASK,
    SHIFT_MASK,
    ScreenAnchor,
)
from keyswitch.macos_backend import (
    MacBackend,
    MacBackendError,
    NativeInput,
    NativeKeyEvent,
    TAP_DISABLED_BY_TIMEOUT,
    VK_ANSI_Q,
    VK_ANSI_Z,
    VK_BACKSPACE,
    VK_COMMAND,
    VK_CONTROL,
    VK_LEFT_ARROW,
    VK_OPTION,
    VK_PERIOD,
    VK_RETURN,
    VK_SHIFT,
    key_name,
    select_source_pair,
)

ENGLISH = "com.apple.keylayout.ABC"
RUSSIAN = "com.apple.keylayout.Russian"
GERMAN = "com.apple.keylayout.German"

# What the two layouts make of the keys the tests press, taken from the probe
# run on a real Mac rather than invented.
CHARACTERS = {
    (VK_ANSI_Q, ENGLISH): ("q", "Q"),
    (VK_ANSI_Q, RUSSIAN): ("й", "Й"),
    (VK_ANSI_Z, ENGLISH): ("z", "Z"),
    (VK_ANSI_Z, RUSSIAN): ("я", "Я"),
    (VK_ANSI_Q, GERMAN): ("q", "Q"),
    (VK_RETURN, ENGLISH): ("\r", "\r"),
    (VK_RETURN, RUSSIAN): ("\r", "\r"),
    (VK_BACKSPACE, ENGLISH): ("", ""),
    (VK_BACKSPACE, RUSSIAN): ("", ""),
    (VK_SHIFT, ENGLISH): ("", ""),
    (VK_SHIFT, RUSSIAN): ("", ""),
    (VK_LEFT_ARROW, ENGLISH): ("", ""),
    (VK_LEFT_ARROW, RUSSIAN): ("", ""),
    (VK_PERIOD, ENGLISH): (".", ">"),
    (VK_PERIOD, RUSSIAN): (".", ">"),
}

# Fake identifiers the substitute API hands back, distinctive enough that a
# mix-up between them would show up immediately.
FAKE_WINDOW_ID = 101
FAKE_PROCESS_ID = 4242
OWN_PROCESS_ID = 7

# The caret anchor the substitute API reports, asserted back against verbatim.
FAKE_ANCHOR_X = 10
FAKE_ANCHOR_Y = 20

# The timestamp carried by every synthetic key event; the backend passes it
# through unlooked-at, so any fixed value will do.
FAKE_EVENT_TIMESTAMP = 1000

# How long run_event_tap's stand-in blocks before giving up if stop_event is
# never set, so a stuck test cannot hang the whole suite.
STOP_WAIT_SAFETY_TIMEOUT_SECONDS = 5.0

# A keycode with no name in KEY_NAMES, to exercise the VK_<hex> fallback.
UNKNOWN_VK_KEYCODE = 0x5A

# Window ids distinct from FAKE_WINDOW_ID, standing in for the focus having
# moved somewhere else.
WINDOW_ID_CHANGED_DURING_DEFERRAL = 999
WINDOW_ID_CHANGED_DURING_HOLD = 555
WINDOW_ID_DURING_SECOND_HOLD = 888

# Group indices outside the {0, 1} pair, used to provoke a refusal.
UNKNOWN_TARGET_GROUP = 5
GROUP_OUTSIDE_PAIR = 7
UNKNOWN_SOURCE_GROUP = 9

# Counts of native events a correction is expected to post.
PRESS_RELEASE_EVENT_COUNT = 2
ERASE_EVENT_COUNT = 4
SHIFTED_KEYSTROKE_EVENT_COUNT = 4
BACKSPACE_PRESS_COUNT = 2

# How many calls into post_inputs succeed before CountingAPI starts refusing.
FAIL_AFTER_CALLS = 2

# Arbitrary window ids: the backend ignores them on macOS, so any value proves
# the point.
RESTORE_WINDOW_ID = 123
INACTIVE_WINDOW_ID = 456


class FakeMacAPI:
    def __init__(self, sources: tuple[str, ...] = (ENGLISH, RUSSIAN)) -> None:
        self.sources = sources
        self.current = sources[0]
        self.posted: list[NativeInput] = []
        self.window = FAKE_WINDOW_ID
        self.own_window = 0
        self.process = FAKE_PROCESS_ID
        self.caps = False
        self.per_window_layout = False
        self.tap_enabled = True
        self.enable_calls = 0
        self.stopped = False
        self.accept_posts = True
        self.select_refusals: set[str] = set()
        self.trusted = True
        self.prompts = 0
        self.settings_opened = 0

    def input_sources(self) -> tuple[str, ...]:
        return self.sources

    def current_input_source(self) -> str:
        return self.current

    def select_input_source(self, identifier: str) -> bool:
        if identifier in self.select_refusals:
            return False
        self.current = identifier
        return True

    def translate_key(self, keycode: int, state: int, source: str) -> str:
        pair = CHARACTERS.get((keycode, source), ("", ""))
        return pair[1] if state & SHIFT_MASK else pair[0]

    def post_inputs(self, inputs: tuple[NativeInput, ...]) -> int:
        if not self.accept_posts:
            return 0
        self.posted.extend(inputs)
        return len(inputs)

    def active_application(self) -> str:
        return "com.apple.TextEdit"

    def focused_window(self) -> int:
        return self.window

    def window_process_id(self, window: int) -> int:
        return self.process if window != self.own_window else OWN_PROCESS_ID

    def current_process_id(self) -> int:
        return OWN_PROCESS_ID

    def input_anchor(self) -> ScreenAnchor | None:
        return ScreenAnchor(FAKE_ANCHOR_X, FAKE_ANCHOR_Y, self.window)

    def caps_lock_enabled(self) -> bool:
        return self.caps

    def accessibility_trusted(self, *, prompt: bool = False) -> bool:
        if prompt:
            self.prompts += 1
        return self.trusted

    def open_accessibility_settings(self) -> bool:
        self.settings_opened += 1
        return True

    def layout_follows_window(self) -> bool:
        return self.per_window_layout

    def run_event_tap(
        self,
        listener: Callable[[NativeKeyEvent], bool],
        ready: Callable[[], None],
    ) -> None:
        self.listener = listener
        ready()
        self.stop_event = threading.Event()
        self.stop_event.wait(timeout=STOP_WAIT_SAFETY_TIMEOUT_SECONDS)

    def enable_event_tap(self) -> None:
        self.enable_calls += 1
        self.tap_enabled = True

    def stop_event_tap(self) -> None:
        self.stopped = True
        event = getattr(self, "stop_event", None)
        if event is not None:
            event.set()


def press(keycode: int, pressed: bool = True, **extra: object) -> NativeKeyEvent:
    return NativeKeyEvent(pressed, keycode, FAKE_EVENT_TIMESTAMP, **extra)  # type: ignore[arg-type]


class SourcePairTests(unittest.TestCase):
    def test_the_pair_is_chosen_by_script_not_by_name(self) -> None:
        """A layout's identifier is not a promise about the letters it types."""

        api = FakeMacAPI((GERMAN, RUSSIAN))
        self.assertEqual(select_source_pair(api.input_sources(), api.translate_key), (GERMAN, RUSSIAN))

    def test_a_missing_script_is_refused_rather_than_guessed(self) -> None:
        api = FakeMacAPI((ENGLISH, GERMAN))
        with self.assertRaises(MacBackendError):
            select_source_pair(api.input_sources(), api.translate_key)

    def test_key_names_follow_the_vocabulary_the_engine_speaks(self) -> None:
        self.assertEqual(key_name(VK_BACKSPACE, ""), "BackSpace")
        self.assertEqual(key_name(VK_RETURN, "\r"), "Return")
        self.assertEqual(key_name(VK_LEFT_ARROW, ""), "Left")
        self.assertEqual(key_name(VK_ANSI_Q, "q"), "q")
        self.assertEqual(key_name(UNKNOWN_VK_KEYCODE, ""), "VK_5A")


class BackendTests(unittest.TestCase):
    def setUp(self) -> None:
        self.api = FakeMacAPI()
        self.backend = MacBackend(self.api)
        self.seen: list[KeyEvent] = []
        self.backend._listener = self.seen.append

    def test_a_key_carries_a_character_for_every_layout(self) -> None:
        self.backend._handle_native(press(VK_ANSI_Q))
        event = self.seen[-1]
        self.assertEqual(event.characters, ("q", "й"))
        self.assertEqual(event.character, "q")
        self.assertEqual(event.group, 0)
        self.assertEqual(event.key_name, "q")

    def test_the_character_follows_the_layout_in_use(self) -> None:
        self.api.current = RUSSIAN
        self.backend._handle_native(press(VK_ANSI_Q))
        self.assertEqual(self.seen[-1].character, "й")
        self.assertEqual(self.seen[-1].group, 1)

    def test_shift_and_caps_lock_reach_the_engine_as_state(self) -> None:
        self.api.caps = True
        self.backend._handle_native(press(VK_SHIFT))
        self.backend._handle_native(press(VK_ANSI_Q))
        event = self.seen[-1]
        self.assertTrue(event.shift)
        self.assertTrue(event.caps_lock)
        self.assertEqual(event.characters, ("Q", "Й"))
        self.assertEqual(event.state & (SHIFT_MASK | LOCK_MASK), SHIFT_MASK | LOCK_MASK)

    def test_a_pointer_event_is_named_and_counted(self) -> None:
        self.backend._handle_native(NativeKeyEvent(True, 0, FAKE_EVENT_TIMESTAMP, pointer=True))
        self.assertEqual(self.seen[-1].key_name, "Pointer")
        self.assertEqual(self.backend._pointer_epoch, 1)

    def test_a_tap_the_system_switched_off_is_revived(self) -> None:
        """Ignoring this message leaves a dead tap and a program gone deaf."""

        swallowed = self.backend._handle_native(
            NativeKeyEvent(True, 0, FAKE_EVENT_TIMESTAMP, event_type=TAP_DISABLED_BY_TIMEOUT))
        self.assertFalse(swallowed)
        self.assertEqual(self.api.enable_calls, 1)
        self.assertEqual(self.backend.tap_revivals, 1)
        self.assertEqual(self.seen, [])

    def test_a_swallowed_press_takes_its_release_with_it(self) -> None:
        self.backend.set_key_filter(lambda event: event.keycode == VK_ANSI_Z)
        self.assertTrue(self.backend._handle_native(press(VK_ANSI_Z)))
        self.assertTrue(self.backend._handle_native(press(VK_ANSI_Z, False)))
        self.assertFalse(self.backend._handle_native(press(VK_ANSI_Q)))

    def test_a_deferred_key_is_held_and_then_delivered(self) -> None:
        self.backend.set_key_filter(
            lambda event: "defer" if event.keycode == VK_RETURN and event.pressed else False)
        self.assertTrue(self.backend._handle_native(press(VK_RETURN)))
        self.assertTrue(self.seen[-1].deferred)
        self.assertEqual(self.backend.complete_action(True), COMPLETED_ACTION_EVENT_COUNT)
        self.assertEqual([(item.pressed, item.keycode) for item in self.api.posted],
                         [(True, VK_RETURN), (False, VK_RETURN)])

    def test_a_deferred_key_dropped_after_the_window_changed_is_not_delivered(self) -> None:
        self.backend.set_key_filter(
            lambda event: "defer" if event.keycode == VK_RETURN and event.pressed else False)
        self.backend._handle_native(press(VK_RETURN))
        self.api.window = WINDOW_ID_CHANGED_DURING_DEFERRAL
        with self.assertRaises(MacBackendError):
            self.backend.complete_action(True)
        self.assertEqual(self.api.posted, [])

    def test_keys_typed_during_a_hold_are_posted_again_afterwards(self) -> None:
        self.backend.hold_input()
        self.assertTrue(self.backend._handle_native(press(VK_ANSI_Z)))
        self.assertTrue(self.backend._handle_native(press(VK_ANSI_Z, False)))
        self.assertEqual(self.backend.release_input(), PRESS_RELEASE_EVENT_COUNT)
        self.assertEqual([(item.pressed, item.keycode, item.replayed) for item in self.api.posted],
                         [(True, VK_ANSI_Z, True), (False, VK_ANSI_Z, True)])

    def test_an_event_the_backend_posted_is_not_held_again(self) -> None:
        self.backend.hold_input()
        self.assertFalse(self.backend._handle_native(press(VK_ANSI_Z, replayed=True)))
        self.assertEqual(self.backend._held, [])

    def test_switching_the_layout_reports_a_refusal(self) -> None:
        self.api.select_refusals = {RUSSIAN}
        with self.assertRaises(MacBackendError):
            self.backend.switch_group(1)

    def test_the_focused_window_says_whether_it_is_our_own(self) -> None:
        info = self.backend.focused_window()
        assert info is not None
        self.assertFalse(info.own)
        self.api.own_window = self.api.window
        info = self.backend.focused_window()
        assert info is not None
        self.assertTrue(info.own)

    def test_the_probe_names_the_pair_and_the_current_group(self) -> None:
        self.api.current = RUSSIAN
        probe = self.backend.probe()
        self.assertTrue(probe.available)
        self.assertEqual(probe.current_group, 1)
        self.assertIn(RUSSIAN, probe.xkb_version)

    def test_a_probe_without_both_scripts_reports_the_reason(self) -> None:
        backend = MacBackend(FakeMacAPI((ENGLISH, GERMAN)))
        probe = backend.probe()
        self.assertFalse(probe.available)
        self.assertIn("раскладки", probe.error)


class InjectionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.api = FakeMacAPI()
        self.backend = MacBackend(self.api)

    def stroke(self, keycode: int, group: int = 0, **extra: object) -> KeyEvent:
        characters = (
            CHARACTERS[(keycode, ENGLISH)][0],
            CHARACTERS[(keycode, RUSSIAN)][0],
        )
        return KeyEvent(
            True, keycode, key_name(keycode, characters[0]), characters[group],
            characters, group, 0, FAKE_EVENT_TIMESTAMP, **extra)  # type: ignore[arg-type]

    def test_a_word_is_erased_and_typed_again_in_the_other_layout(self) -> None:
        strokes = [self.stroke(VK_ANSI_Q), self.stroke(VK_ANSI_Z)]
        self.backend.inject_correction(strokes, 1, None)
        codes = [(item.pressed, item.keycode) for item in self.api.posted]
        self.assertEqual(codes[:ERASE_EVENT_COUNT], [(True, VK_BACKSPACE), (False, VK_BACKSPACE),
                                     (True, VK_BACKSPACE), (False, VK_BACKSPACE)])
        self.assertEqual(codes[ERASE_EVENT_COUNT:], [(True, VK_ANSI_Q), (False, VK_ANSI_Q),
                                     (True, VK_ANSI_Z), (False, VK_ANSI_Z)])
        self.assertEqual(self.api.current, RUSSIAN)

    def test_an_unknown_target_layout_is_refused_before_anything_is_erased(self) -> None:
        with self.assertRaises(MacBackendError):
            self.backend.inject_correction([self.stroke(VK_ANSI_Q)], UNKNOWN_TARGET_GROUP, None)
        self.assertEqual(self.api.posted, [])

    def test_a_refused_post_does_not_leave_the_keyboard_captured(self) -> None:
        self.backend.hold_input()
        self.api.accept_posts = False
        with self.assertRaises(MacBackendError):
            self.backend.inject_correction([self.stroke(VK_ANSI_Q)], 1, None)
        self.assertFalse(self.backend._holding)


if __name__ == "__main__":
    unittest.main()


class LifecycleTests(unittest.TestCase):
    def setUp(self) -> None:
        self.api = FakeMacAPI()
        self.backend = MacBackend(self.api)

    def test_the_tap_runs_on_its_own_thread_and_stops_on_request(self) -> None:
        seen: list[KeyEvent] = []
        self.backend.start(seen.append)
        self.assertTrue(self.backend.running)
        self.backend.start(seen.append)  # A second call is not a second tap.
        self.backend.close()
        self.assertFalse(self.backend.running)
        self.assertTrue(self.api.stopped)

    def test_stopping_a_backend_that_never_started_does_nothing(self) -> None:
        self.backend.stop()
        self.assertFalse(self.api.stopped)

    def test_a_tap_the_system_refuses_is_reported_to_the_caller(self) -> None:
        """The refusal is what a missing Accessibility permission looks like."""

        def refuse(listener: object, ready: Callable[[], None]) -> None:
            raise MacBackendError("нет разрешения")

        self.api.run_event_tap = refuse  # type: ignore[method-assign]
        with self.assertRaises(MacBackendError):
            self.backend.start(lambda event: None)
        self.assertFalse(self.backend.running)

    def test_the_names_the_engine_asks_for_come_from_the_system(self) -> None:
        self.assertEqual(self.backend.active_application(), "com.apple.TextEdit")
        anchor = self.backend.input_anchor()
        assert anchor is not None
        self.assertEqual((anchor.x, anchor.y), (FAKE_ANCHOR_X, FAKE_ANCHOR_Y))

    def test_a_group_outside_the_pair_is_refused(self) -> None:
        with self.assertRaises(MacBackendError):
            self.backend.switch_group(GROUP_OUTSIDE_PAIR)

    def test_an_unknown_current_source_reads_as_the_first_group(self) -> None:
        self.api.current = GERMAN
        self.assertEqual(self.backend.current_group(), 0)

    def test_no_focused_window_is_reported_as_none(self) -> None:
        self.api.window = 0
        self.assertIsNone(self.backend.focused_window())

    def test_a_window_carrying_its_own_layout_says_so(self) -> None:
        self.api.per_window_layout = True
        info = self.backend.focused_window()
        assert info is not None
        self.assertTrue(info.isolated_layout)

    def test_the_real_system_layer_is_used_when_none_is_supplied(self) -> None:
        """Only the wiring is checked here; the layer itself needs a Mac."""

        module = types.ModuleType("keyswitch.macos_native")
        sentinel = FakeMacAPI()
        module.CtypesMacAPI = lambda: sentinel  # type: ignore[attr-defined]
        with patch.dict(sys.modules, {"keyswitch.macos_native": module}):
            backend = MacBackend()
        self.assertIs(backend._api, sentinel)


class HoldTests(unittest.TestCase):
    def setUp(self) -> None:
        self.api = FakeMacAPI()
        self.backend = MacBackend(self.api)
        self.seen: list[KeyEvent] = []
        self.backend._listener = self.seen.append

    def defer_return(self) -> None:
        self.backend.set_key_filter(
            lambda event: "defer" if event.keycode == VK_RETURN and event.pressed else False)

    def test_nothing_is_replayed_while_an_action_is_still_waiting(self) -> None:
        self.defer_return()
        self.backend._handle_native(press(VK_RETURN))
        self.assertEqual(self.backend.release_input(), 0)
        self.assertEqual(self.api.posted, [])

    def test_a_release_of_a_key_pressed_before_the_action_reaches_the_engine(self) -> None:
        """Those releases must not be withheld, or the action waits for ever."""

        self.backend._handle_native(press(VK_SHIFT))
        self.defer_return()
        self.backend._handle_native(press(VK_RETURN))
        self.assertFalse(self.backend._handle_native(press(VK_SHIFT, False)))
        self.assertEqual(self.seen[-1].key_name, "Shift_L")

    def test_text_typed_after_the_action_waits_for_it(self) -> None:
        self.defer_return()
        self.backend._handle_native(press(VK_RETURN))
        self.assertTrue(self.backend._handle_native(press(VK_ANSI_Z)))
        self.assertEqual(self.backend.release_input(), 0)
        self.assertEqual(self.backend.complete_action(False), 0)
        self.assertEqual([(item.pressed, item.keycode) for item in self.api.posted],
                         [(True, VK_ANSI_Z)])

    def test_a_repeat_of_the_action_key_follows_its_own_replay(self) -> None:
        self.defer_return()
        self.backend._handle_native(press(VK_SHIFT))
        self.backend._handle_native(press(VK_RETURN))
        self.backend._held.append(press(VK_SHIFT))
        self.backend._handle_native(press(VK_SHIFT, False))
        self.assertEqual([(item.pressed, item.keycode) for item in self.backend._held],
                         [(True, VK_SHIFT), (False, VK_SHIFT)])

    def test_completing_an_action_nobody_deferred_changes_nothing(self) -> None:
        self.assertEqual(self.backend.complete_action(True), 0)
        self.assertEqual(self.api.posted, [])

    def test_a_refused_post_of_a_held_key_still_frees_the_keyboard(self) -> None:
        self.backend.hold_input()
        self.backend._handle_native(press(VK_ANSI_Z))
        self.api.accept_posts = False
        with self.assertRaises(MacBackendError):
            self.backend.release_input()
        self.assertFalse(self.backend._holding)

    def test_an_injected_key_is_never_answered_by_the_filter(self) -> None:
        self.backend.set_key_filter(lambda event: True)
        self.assertFalse(self.backend._handle_native(press(VK_ANSI_Z, injected=True)))

    def test_a_key_already_being_swallowed_is_not_asked_about_twice(self) -> None:
        answers: list[KeyEvent] = []

        def filter_once(event: KeyEvent) -> KeyDisposition:
            answers.append(event)
            return True

        self.backend.set_key_filter(filter_once)
        self.backend._handle_native(press(VK_ANSI_Z))
        self.backend._handle_native(press(VK_ANSI_Z))
        self.assertEqual(len(answers), 1)
        self.assertEqual(len(self.seen), 1)


class InjectionDetailTests(unittest.TestCase):
    def setUp(self) -> None:
        self.api = FakeMacAPI()
        self.backend = MacBackend(self.api)

    def stroke(self, keycode: int, group: int = 0, **extra: object) -> KeyEvent:
        characters = (CHARACTERS[(keycode, ENGLISH)][0], CHARACTERS[(keycode, RUSSIAN)][0])
        return KeyEvent(True, keycode, key_name(keycode, characters[0]), characters[group],
                        characters, group, 0, FAKE_EVENT_TIMESTAMP, **extra)  # type: ignore[arg-type]

    def test_an_unknown_source_layout_is_refused(self) -> None:
        with self.assertRaises(MacBackendError):
            self.backend.inject_correction(
                [self.stroke(VK_ANSI_Q)], 1, None, source_group=UNKNOWN_SOURCE_GROUP)

    def test_punctuation_typed_in_a_mixture_of_layouts_is_refused(self) -> None:
        first = self.stroke(VK_ANSI_Q, 0)
        second = self.stroke(VK_ANSI_Z, 1)
        with self.assertRaises(MacBackendError):
            self.backend.inject_correction([], 1, None, trailing=(first, second))
        self.assertEqual(self.api.posted, [])

    def test_a_boundary_that_would_change_letter_keeps_its_own_layout(self) -> None:
        """Typing it in the new layout would put a different character there."""

        boundary = self.stroke(VK_ANSI_Z, 0)
        self.backend.inject_correction([self.stroke(VK_ANSI_Q)], 1, boundary)
        self.assertEqual(self.api.current, RUSSIAN)
        codes = [(item.pressed, item.keycode) for item in self.api.posted]
        self.assertIn((True, VK_ANSI_Z), codes)

    def test_late_keys_are_typed_again_after_the_replacement(self) -> None:
        late = self.stroke(VK_ANSI_Z)
        self.backend.inject_correction([self.stroke(VK_ANSI_Q)], 1, None, late=(late,))
        posted = [(item.pressed, item.keycode, item.replayed) for item in self.api.posted]
        self.assertEqual(posted[-PRESS_RELEASE_EVENT_COUNT:],
                         [(True, VK_ANSI_Z, True), (False, VK_ANSI_Z, True)])
        self.assertEqual(
            sum(1 for item in posted if item[1] == VK_BACKSPACE and item[0]), BACKSPACE_PRESS_COUNT)

    def test_a_shifted_stroke_is_typed_with_shift_around_it(self) -> None:
        stroke = KeyEvent(True, VK_ANSI_Q, "q", "Q", ("Q", "Й"), 0, SHIFT_MASK, FAKE_EVENT_TIMESTAMP)
        self.backend.inject_correction([stroke], 1, None)
        codes = [(item.pressed, item.keycode) for item in self.api.posted]
        self.assertEqual(codes[-SHIFTED_KEYSTROKE_EVENT_COUNT:],
                         [(True, VK_SHIFT), (True, VK_ANSI_Q),
                          (False, VK_ANSI_Q), (False, VK_SHIFT)])

    def test_caps_lock_alone_supplies_the_shift_a_letter_needs(self) -> None:
        self.api.caps = True
        stroke = KeyEvent(True, VK_ANSI_Q, "q", "q", ("q", "й"), 0, 0, FAKE_EVENT_TIMESTAMP)
        self.backend.inject_correction([stroke], 1, None)
        codes = [(item.pressed, item.keycode) for item in self.api.posted]
        self.assertEqual(codes[-SHIFTED_KEYSTROKE_EVENT_COUNT:],
                         [(True, VK_SHIFT), (True, VK_ANSI_Q),
                          (False, VK_ANSI_Q), (False, VK_SHIFT)])

    def test_a_layout_that_never_arrives_stops_the_replacement(self) -> None:
        class StubbornAPI(FakeMacAPI):
            def select_input_source(self, identifier: str) -> bool:
                return True  # Accepted, but the layout never becomes the current one.

        api = StubbornAPI()
        backend = MacBackend(api)
        with self.assertRaises(MacBackendError):
            backend.inject_correction([self.stroke(VK_ANSI_Q)], 1, None)

    def test_a_window_change_during_a_hold_cancels_the_replacement(self) -> None:
        self.backend.hold_input()
        self.api.window = WINDOW_ID_CHANGED_DURING_HOLD
        with self.assertRaises(MacBackendError):
            self.backend.inject_correction([self.stroke(VK_ANSI_Q)], 1, None)

    def test_a_correction_into_the_layout_already_in_use_switches_nothing(self) -> None:
        self.backend.inject_correction([self.stroke(VK_ANSI_Q)], 0, None)
        self.assertEqual(self.api.current, ENGLISH)


class UncommonPathTests(unittest.TestCase):
    """The paths that only appear when something goes wrong or races."""

    def setUp(self) -> None:
        self.api = FakeMacAPI()
        self.backend = MacBackend(self.api)

    def stroke(self, keycode: int, group: int = 0) -> KeyEvent:
        characters = (CHARACTERS[(keycode, ENGLISH)][0], CHARACTERS[(keycode, RUSSIAN)][0])
        return KeyEvent(True, keycode, key_name(keycode, characters[0]), characters[group],
                        characters, group, 0, FAKE_EVENT_TIMESTAMP)

    def test_stopping_from_the_tap_thread_does_not_wait_for_itself(self) -> None:
        self.backend._running.set()
        self.backend._thread = None
        self.backend.stop()
        self.assertTrue(self.api.stopped)

    def test_holding_twice_keeps_the_place_the_first_hold_recorded(self) -> None:
        self.backend.hold_input()
        self.api.window = WINDOW_ID_DURING_SECOND_HOLD
        self.backend.hold_input()
        self.assertEqual(self.backend._hold_window, FAKE_WINDOW_ID)

    def test_every_modifier_reaches_the_engine_as_its_own_bit(self) -> None:
        seen: list[KeyEvent] = []
        self.backend._listener = seen.append
        for keycode in (VK_CONTROL, VK_OPTION, VK_COMMAND):
            self.backend._handle_native(press(keycode))
        self.backend._handle_native(press(VK_ANSI_Q))
        event = seen[-1]
        self.assertTrue(event.control and event.alt and event.super_key)

    def test_a_pointer_event_without_a_listener_is_still_counted(self) -> None:
        self.backend._handle_native(NativeKeyEvent(True, 0, FAKE_EVENT_TIMESTAMP, pointer=True))
        self.assertEqual(self.backend._pointer_epoch, 1)

    def test_an_action_appearing_during_a_replay_holds_back_the_rest(self) -> None:
        """Reposting a key calls the tap again, which may defer the next Enter."""

        backend = self.backend
        deferred = press(VK_RETURN)

        def post_then_defer(inputs: tuple[NativeInput, ...]) -> int:
            if backend._deferred_action is None:
                backend._deferred_action = deferred
            return FakeMacAPI.post_inputs(self.api, inputs)

        backend._holding = True
        backend._held = [press(VK_ANSI_Z), press(VK_SHIFT, False)]
        backend._action_prior_keys = {VK_SHIFT}
        self.api.post_inputs = post_then_defer  # type: ignore[method-assign]
        self.assertEqual(backend.release_input(), PRESS_RELEASE_EVENT_COUNT)
        self.assertEqual([(item.pressed, item.keycode) for item in self.api.posted],
                         [(True, VK_ANSI_Z), (False, VK_SHIFT)])

    def test_punctuation_is_typed_without_a_shift_it_does_not_need(self) -> None:
        self.api.caps = True
        self.backend.inject_correction([], 1, self.stroke(VK_PERIOD))
        codes = [(item.pressed, item.keycode) for item in self.api.posted]
        self.assertNotIn((True, VK_SHIFT), codes)
        self.assertIn((True, VK_PERIOD), codes)

    def test_a_failure_to_restore_does_not_hide_the_failure_that_caused_it(self) -> None:
        class CountingAPI(FakeMacAPI):
            def __init__(self) -> None:
                super().__init__()
                self.calls = 0
                self.fail_from = FAIL_AFTER_CALLS

            def post_inputs(self, inputs: tuple[NativeInput, ...]) -> int:
                self.calls += 1
                if self.calls >= self.fail_from:
                    return 0
                return super().post_inputs(inputs)

        api = CountingAPI()
        backend = MacBackend(api)
        boundary = self.stroke(VK_ANSI_Z)
        late = self.stroke(VK_ANSI_Q)
        with self.assertRaises(MacBackendError):
            backend.inject_correction([self.stroke(VK_ANSI_Q)], 1, boundary, late=(late,))


class PermissionTests(unittest.TestCase):
    """Without the permission there is no keyboard to watch, so it is reported."""

    def setUp(self) -> None:
        self.api = FakeMacAPI()
        self.backend = MacBackend(self.api)

    def test_a_granted_permission_is_reported_plainly(self) -> None:
        self.assertTrue(self.backend.permission_granted())
        self.assertEqual(self.api.prompts, 0)

    def test_the_probe_says_what_is_missing_rather_than_naming_layouts(self) -> None:
        self.api.trusted = False
        probe = self.backend.probe()
        self.assertFalse(probe.available)
        self.assertIn("Универсальный доступ", probe.error)
        self.assertEqual(probe.current_group, -1)

    def test_asking_shows_the_request_and_opens_the_pane_when_refused(self) -> None:
        self.api.trusted = False
        self.assertFalse(self.backend.request_permission())
        self.assertEqual(self.api.prompts, 1)
        self.assertEqual(self.api.settings_opened, 1)

    def test_a_permission_already_given_opens_nothing(self) -> None:
        self.assertTrue(self.backend.request_permission())
        self.assertEqual(self.api.settings_opened, 0)


class PromptWindowTests(unittest.TestCase):
    """What the learning prompt asks of the backend, and why macOS says no."""

    def test_the_prompt_needs_no_help_keeping_the_focus_in_place(self) -> None:
        backend = MacBackend(FakeMacAPI())
        self.assertFalse(backend.restore_window(RESTORE_WINDOW_ID))
        self.assertFalse(backend.keep_window_inactive(INACTIVE_WINDOW_ID))
