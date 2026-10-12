"""A wait the next word never answered is settled by the pause, not left to lapse.

The engine postpones a short word whose direction its neighbour should decide. When the
neighbour arrives, the pair is decided together. When it does not - the user stopped, sent
the message, walked away - the postponement used to end in silence and the word stood as
typed, which is a refusal nobody asked for. The pause is where that possibility runs out,
and the word is put to the model once more as the word it is.

These cases live outside `test_context_policy` on purpose: that module's bytes are pinned by
the context-action release receipt, so a test added to it after a candidate is sealed would
invalidate the receipt of the pair it is meant to describe.
"""

from __future__ import annotations

import unittest
from dataclasses import replace
from unittest.mock import patch

from keyswitch.constants.timing import CONTEXT_WAIT_SETTLE_MIN_SECONDS
from test_context_policy import ContextEngineTests
from fixture_values.clock import (
    BEFORE_PAUSE_THRESHOLD_MARGIN_SECONDS,
    LAST_WORD_INPUT_AT_SECONDS,
    NON_DEFAULT_PAUSE_DELAY_SECONDS,
    SETTLED_NOW_SECONDS,
    TOO_SOON_NOW_SECONDS,
)


class ContextWaitSettlementTests(ContextEngineTests):
    def test_a_pause_settles_a_wait_the_next_word_never_answered(self) -> None:
        self.choose("wait")
        self.type("yt ")
        waiting = self.engine._context_waiting
        assert waiting is not None
        self.assertEqual(self.backend.text, "yt ")
        # The boundary commit clears the pause timer's own bookkeeping, so the idle
        # moment is supplied the way the engine's timers supply it.
        self.engine._last_word_input_at = LAST_WORD_INPUT_AT_SECONDS
        self.choose("convert")
        self.engine._maybe_correct_after_pause(now=SETTLED_NOW_SECONDS)
        self.assertIsNone(self.engine._context_waiting)
        # The correction is made at the pause, not left for the next key to set off.
        self.assertIsNone(self.engine._pending)
        self.assertEqual(self.backend.text, "не ")

    def test_the_engine_itself_times_the_pause_of_a_waiting_word(self) -> None:
        # Without any help from the test: the wait records its own boundary moment.
        self.choose("wait")
        self.type("yt ")
        waiting = self.engine._context_waiting
        assert waiting is not None
        started = self.engine._last_word_input_at
        assert started is not None
        self.assertFalse(self.engine._pause_correction_pending)
        self.choose("convert")
        self.engine._maybe_correct_after_pause(now=started + CONTEXT_WAIT_SETTLE_MIN_SECONDS)
        self.assertIsNone(self.engine._pending)
        self.assertEqual(self.backend.text, "не ")

    def test_the_next_word_after_a_settled_wait_is_typed_as_it_is(self) -> None:
        """The correction used to wait for the next key and then type that key again in the other
        layout: `в` typed after the pause came out as `d`, so `yt ` + `вход` gave `не dход`."""
        self.choose("wait")
        self.type("yt ")
        self.engine._last_word_input_at = LAST_WORD_INPUT_AT_SECONDS
        self.choose("convert")
        self.engine._maybe_correct_after_pause(now=SETTLED_NOW_SECONDS)
        self.assertEqual(self.backend.group, 1)
        self.choose("keep")
        self.type("вход ")
        self.assertEqual(self.backend.text, "не вход ")

    def test_a_held_key_puts_off_the_pause_that_settles_a_wait(self) -> None:
        """A key still down at the pause is not a finished thought: the wait stays and is settled
        at a later timer, once the key is up."""
        self.choose("wait")
        self.type("yt ")
        self.choose("convert")
        shift = self.key("Shift_L")
        self.send(shift)
        self.engine._last_word_input_at = LAST_WORD_INPUT_AT_SECONDS
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            # Two timer ticks with the key still down: the deferral is logged once.
            self.engine._maybe_correct_after_pause(now=SETTLED_NOW_SECONDS)
            self.engine._maybe_correct_after_pause(now=SETTLED_NOW_SECONDS)
        self.assertEqual(self.backend.text, "yt ")
        self.assertIsNotNone(self.engine._context_waiting)
        deferred = [line for line in logs.output if '"event":"context_wait_deferred"' in line]
        self.assertEqual(len(deferred), 1)
        self.assertIn('"reason":"modifiers_pressed"', deferred[0])
        self.send(replace(shift, pressed=False))
        self.engine._last_word_input_at = LAST_WORD_INPUT_AT_SECONDS
        self.engine._maybe_correct_after_pause(now=SETTLED_NOW_SECONDS)
        self.assertEqual(self.backend.text, "не ")

    def test_a_wait_is_not_settled_where_a_pause_may_not_correct(self) -> None:
        """Settling is a correction at a pause: with pause corrections off it is not made, and the word
        waits on for its neighbour."""
        self.settings.set("detection.correct_on_pause", False)
        self.choose("wait")
        self.type("yt ")
        self.choose("convert")
        self.engine._last_word_input_at = LAST_WORD_INPUT_AT_SECONDS
        self.engine._maybe_correct_after_pause(now=SETTLED_NOW_SECONDS)
        self.assertEqual(self.backend.text, "yt ")
        self.assertIsNotNone(self.engine._context_waiting)

    def test_a_short_pause_correction_leaves_a_waiting_word_its_neighbour(self) -> None:
        """The pause correction may be set short, but a waiting word is settled no sooner than
        CONTEXT_WAIT_SETTLE_MIN_SECONDS after its space: a thought between two words is no hesitation
        inside one, and the next word may still decide it."""
        self.settings.set("detection.pause_delay_seconds", NON_DEFAULT_PAUSE_DELAY_SECONDS)
        self.choose("wait")
        self.type("yt ")
        self.choose("convert")
        self.engine._last_word_input_at = LAST_WORD_INPUT_AT_SECONDS
        self.engine._maybe_correct_after_pause(now=LAST_WORD_INPUT_AT_SECONDS + NON_DEFAULT_PAUSE_DELAY_SECONDS
                                              + BEFORE_PAUSE_THRESHOLD_MARGIN_SECONDS)
        self.assertEqual(self.backend.text, "yt ")
        self.assertIsNotNone(self.engine._context_waiting)
        self.engine._maybe_correct_after_pause(now=SETTLED_NOW_SECONDS)
        self.assertEqual(self.backend.text, "не ")

    def test_a_pause_leaves_a_waiting_word_the_model_still_keeps(self) -> None:
        """Settling is a second decision, not a conversion: the model can say no."""
        self.choose("wait")
        self.type("yt ")
        assert self.engine._context_waiting is not None
        before = self.backend.text
        self.engine._last_word_input_at = LAST_WORD_INPUT_AT_SECONDS
        self.choose("keep")
        self.engine._maybe_correct_after_pause(now=SETTLED_NOW_SECONDS)
        self.assertIsNone(self.engine._context_waiting)
        self.assertIsNone(self.engine._pending)
        self.assertEqual(self.backend.text, before)

    def test_a_word_left_behind_in_another_application_is_dropped_not_converted(self) -> None:
        """The text the plan describes is no longer in front of the user."""
        self.choose("wait")
        self.type("yt ")
        assert self.engine._context_waiting is not None
        self.engine._last_word_input_at = LAST_WORD_INPUT_AT_SECONDS
        self.choose("convert")
        with patch.object(self.backend, "active_application", return_value="AnotherEditor"):
            self.engine._maybe_correct_after_pause(now=SETTLED_NOW_SECONDS)
        self.assertIsNone(self.engine._context_waiting)
        self.assertIsNone(self.engine._pending)
        self.assertEqual(self.backend.text, "yt ")

    def test_a_wait_in_another_window_or_with_a_correction_running_is_dropped(self) -> None:
        for reason in ("window", "pending", "too_soon"):
            with self.subTest(reason=reason):
                self.reset_editor()
                self.choose("wait")
                self.type("yt ")
                assert self.engine._context_waiting is not None
                self.engine._last_word_input_at = LAST_WORD_INPUT_AT_SECONDS
                moment = SETTLED_NOW_SECONDS
                if reason == "window":
                    self.backend.window += 1
                    self.engine._focus_window = self.backend.window
                elif reason == "pending":
                    self.engine._pending = self.engine._context_waiting.plan
                else:
                    moment = TOO_SOON_NOW_SECONDS
                self.choose("convert")
                self.engine._maybe_correct_after_pause(now=moment)
                if reason == "too_soon":
                    self.assertIsNotNone(self.engine._context_waiting)
                self.engine._pending = None


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    suite = unittest.TestSuite()
    for name in ContextWaitSettlementTests.__dict__:
        if name.startswith("test_"):
            suite.addTest(ContextWaitSettlementTests(name))
    return suite


if __name__ == "__main__":
    unittest.main()
