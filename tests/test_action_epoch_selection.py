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
        assert early is not None and later is not None
        self.assertGreater(later.rank, early.rank)
        self.assertEqual(later.by_profile["portable"]["converted_correctly"], 1)

    def test_any_profile_exceeding_false_conversion_budget_rejects_an_epoch(self) -> None:
        profiles = {"portable": profile(.001, .999), "reference": profile(.999, .999)}
        self.assertIsNone(assess_epoch(profiles, thresholds=(.99,), loss=.001))
        conservative = assess_epoch(profiles, thresholds=(.99, 1.0), loss=.001)
        assert conservative is not None
        self.assertEqual(conservative.threshold, 1.0)
        self.assertEqual(conservative.minimum_recall, 0.0)

    def test_selection_protects_the_weaker_profile_before_pooled_recall(self) -> None:
        uneven = assess_epoch({"portable": profile(.001, .999, .999, .999),
                               "reference": profile(.001, .999, .5, .5)},
                              thresholds=(.99,), loss=.01)
        balanced = assess_epoch({"portable": profile(.001, .999, .999, .5),
                                 "reference": profile(.001, .999, .999, .5)},
                                thresholds=(.99,), loss=.02)
        assert uneven is not None and balanced is not None
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
