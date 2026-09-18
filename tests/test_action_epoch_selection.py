from __future__ import annotations

import unittest
from array import array
import sys
from pathlib import Path


TOOLS = str(Path(__file__).resolve().parents[1] / "tools")
if TOOLS not in sys.path:
    sys.path.insert(0, TOOLS)
from action_epoch_selection import assess_epoch


def profile(keep: float, *converts: float) -> tuple[array[float], array[int]]:
    values = array("d")
    for score in (keep, *converts):
        values.extend((1 - score, score, 0.0, 0.0))
    return values, array("B", (0, *(1 for _ in converts)))


class EpochSelectionTests(unittest.TestCase):
    def test_reaching_runtime_confidence_beats_lower_log_loss(self) -> None:
        early = assess_epoch({"portable": profile(.001, .98)}, thresholds=(.99,), loss=.01)
        later = assess_epoch({"portable": profile(.002, .995)}, thresholds=(.99,), loss=.02)
        self.assertIsNone(early)  # nothing converts at .99: no repairs, so nothing to weigh
        assert later is not None
        self.assertEqual(later.by_profile["portable"]["converted_correctly"], 1)
        self.assertEqual((later.net_benefit, later.minimum_net_benefit), (1, 1))

    def test_an_epoch_that_breaks_as_much_as_it_repairs_is_not_selected(self) -> None:
        """The balance decides, per profile: one profile in the red rejects the epoch."""
        profiles = {"portable": profile(.001, .999), "reference": profile(.999, .999)}
        self.assertIsNone(assess_epoch(profiles, thresholds=(.99,), loss=.001))
        # A higher threshold that converts nothing nets nothing either, so it does not qualify.
        self.assertIsNone(assess_epoch(profiles, thresholds=(.99, 1.0), loss=.001))
        # Two repairs against one breakage in the weaker profile is a net gain.
        ahead = assess_epoch({"portable": profile(.001, .999, .999),
                              "reference": profile(.999, .999, .999)}, thresholds=(.99,), loss=.001)
        assert ahead is not None
        self.assertEqual((ahead.minimum_net_benefit, ahead.net_benefit), (1, 3))

    def test_a_model_that_repairs_more_beats_one_that_merely_never_errs(self) -> None:
        """What the removed zero-budget rule could not express."""
        cautious = assess_epoch({"portable": profile(.001, .999, .5, .5, .5)}, thresholds=(.99,), loss=.01)
        bolder = assess_epoch({"portable": profile(.999, .999, .999, .999, .999)}, thresholds=(.99,), loss=.01)
        assert cautious is not None and bolder is not None
        self.assertEqual((cautious.net_benefit, bolder.net_benefit), (1, 3))
        self.assertGreater(bolder.rank, cautious.rank)

    def test_selection_protects_the_weaker_profile_before_the_pooled_balance(self) -> None:
        uneven = assess_epoch({"portable": profile(.001, .999, .999, .999),
                               "reference": profile(.001, .999, .5, .5)},
                              thresholds=(.99,), loss=.01)
        balanced = assess_epoch({"portable": profile(.001, .999, .999, .5),
                                 "reference": profile(.001, .999, .999, .5)},
                                thresholds=(.99,), loss=.02)
        assert uneven is not None and balanced is not None
        self.assertEqual((uneven.minimum_net_benefit, balanced.minimum_net_benefit), (1, 2))
        self.assertGreater(balanced.rank, uneven.rank)

    def test_equal_runtime_outcomes_use_loss_then_lower_threshold(self) -> None:
        predictions = {"portable": profile(.001, .999)}
        first = assess_epoch(predictions, thresholds=(.995, .99), loss=.02)
        second = assess_epoch(predictions, thresholds=(.99, .995), loss=.01)
        assert first is not None and second is not None
        self.assertEqual(first.threshold, .99)
        self.assertGreater(second.rank, first.rank)
        self.assertEqual(second, assess_epoch(predictions, thresholds=(.995, .99), loss=.01))

    def test_abstention_and_exact_threshold_follow_serving_actions(self) -> None:
        values = array("d", (.0, .99, .01, .0, .0, .1, .0, .9, .0, .01, .99, .0))
        selected = assess_epoch({"portable": (values, array("B", (1, 1, 2)))},
                                thresholds=(.99,), loss=.1)
        assert selected is not None
        self.assertEqual(selected.by_profile["portable"]["converted_correctly"], 1)
        self.assertEqual(selected.by_profile["portable"]["false_conversions"], 0)
        self.assertEqual(selected.minimum_recall, .5)

    def test_invalid_or_empty_evidence_cannot_select_an_epoch(self) -> None:
        valid = {"portable": profile(.001, .999)}
        for loss in (float("nan"), float("inf"), -1.0):
            with self.subTest(loss=loss), self.assertRaises(ValueError):
                assess_epoch(valid, thresholds=(.99,), loss=loss)
        for thresholds in ((), (float("nan"),), (-.1,), (1.1,)):
            with self.subTest(thresholds=thresholds), self.assertRaises(ValueError):
                assess_epoch(valid, thresholds=thresholds, loss=.1)
        invalid: tuple[dict[str, tuple[array[float], array[int]]], ...] = ({}, {"portable": (array("d"), array("B"))},
                   {"portable": (array("d", (1, 0, 0, 0)), array("B", (0,)))},
                   {"portable": (array("d", (1, 0, 0)), array("B", (1,)))},
                   {"portable": (array("d", (1, 0, 0, 0)), array("B", (4,)))},
                   {"portable": (array("d", (float("nan"), 0, 0, 0)), array("B", (1,)))},
                   {"portable": (array("d", (.1, .1, .1, .1)), array("B", (1,)))})
        for predictions in invalid:
            with self.subTest(predictions=predictions), self.assertRaises(ValueError):
                assess_epoch(predictions, thresholds=(.99,), loss=.1)


if __name__ == "__main__":
    unittest.main()
