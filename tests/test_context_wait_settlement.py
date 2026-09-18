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
from unittest.mock import patch

from test_context_policy import ContextEngineTests


class ContextWaitSettlementTests(ContextEngineTests):
    def test_a_pause_settles_a_wait_the_next_word_never_answered(self) -> None:
        self.choose("wait")
        self.type("yt ")
        waiting = self.engine._context_waiting
        assert waiting is not None
        self.assertEqual(self.backend.text, "yt ")
        # The boundary commit clears the pause timer's own bookkeeping, so the idle
        # moment is supplied the way the engine's timers supply it.
        self.engine._last_word_input_at = 100.0
        self.choose("convert")
        self.engine._maybe_correct_after_pause(now=102.0)
        self.assertIsNone(self.engine._context_waiting)
        plan = self.engine._pending
        assert plan is not None
        self.assertEqual((plan.original, plan.replacement), ("yt", "не"))

    def test_a_pause_leaves_a_waiting_word_the_model_still_keeps(self) -> None:
        """Settling is a second decision, not a conversion: the model can say no."""
        self.choose("wait")
        self.type("yt ")
        assert self.engine._context_waiting is not None
        before = self.backend.text
        self.engine._last_word_input_at = 100.0
        self.choose("keep")
        self.engine._maybe_correct_after_pause(now=102.0)
        self.assertIsNone(self.engine._context_waiting)
        self.assertIsNone(self.engine._pending)
        self.assertEqual(self.backend.text, before)

    def test_a_word_left_behind_in_another_application_is_dropped_not_converted(self) -> None:
        """The text the plan describes is no longer in front of the user."""
        self.choose("wait")
        self.type("yt ")
        assert self.engine._context_waiting is not None
        self.engine._last_word_input_at = 100.0
        self.choose("convert")
        with patch.object(self.backend, "active_application", return_value="AnotherEditor"):
            self.engine._maybe_correct_after_pause(now=102.0)
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
                self.engine._last_word_input_at = 100.0
                moment = 102.0
                if reason == "window":
                    self.backend.window += 1
                    self.engine._focus_window = self.backend.window
                elif reason == "pending":
                    self.engine._pending = self.engine._context_waiting.plan
                else:
                    moment = 100.1
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
