from __future__ import annotations

import unittest
from array import array
import sys
from pathlib import Path


TOOLS = str(Path(__file__).resolve().parents[1] / "tools")
if TOOLS not in sys.path:
    sys.path.insert(0, TOOLS)
from action_epoch_selection import assess_epoch
from keyswitch.context_model import ACTIONS
from fixture_values.counts import (
    EPOCH_SELECTION_EXPECTED_MINIMUM_NET_BENEFIT,
    EPOCH_SELECTION_EXPECTED_NET_BENEFIT,
)
from fixture_values.scores import (
    EPOCH_SELECTION_ABSTENTION_ROW_SCORES,
    EPOCH_SELECTION_ALTERNATE_THRESHOLD,
    EPOCH_SELECTION_AMBIGUOUS_SCORE,
    EPOCH_SELECTION_BELOW_THRESHOLD_SCORE,
    EPOCH_SELECTION_DEVELOPMENT_LOSS_HIGH,
    EPOCH_SELECTION_DEVELOPMENT_LOSS_LOW,
    EPOCH_SELECTION_EXPECTED_MINIMUM_RECALL,
    EPOCH_SELECTION_HIGH_CONFIDENCE_SCORE,
    EPOCH_SELECTION_KEEP_ROW_SCORE,
    EPOCH_SELECTION_KEEP_ROW_SCORE_LATER,
    EPOCH_SELECTION_LOSS_NOT_UNDER_TEST,
    EPOCH_SELECTION_RUNTIME_CONFIDENCE_SCORE,
    EPOCH_SELECTION_RUNTIME_THRESHOLD,
    EPOCH_SELECTION_THRESHOLD_ABOVE_RANGE,
    EPOCH_SELECTION_THRESHOLD_BELOW_RANGE,
    EPOCH_SELECTION_UNNORMALIZED_ROW_SCORE,
    EPOCH_SELECTION_VALID_LOSS,
)


def profile(keep: float, *converts: float) -> tuple[array[float], array[int]]:
    values = array("d")
    for score in (keep, *converts):
        values.extend((1 - score, score, 0.0, 0.0))
    return values, array("B", (0, *(1 for _ in converts)))


class EpochSelectionTests(unittest.TestCase):


    # Fixture tables.
    ABSTENTION_ROW_SCORES = array("d", EPOCH_SELECTION_ABSTENTION_ROW_SCORES)
    ABSTENTION_ROW_LABELS = array("B", (1, 1, ACTIONS.index("wait")))
    INVALID_THRESHOLD_SETS: tuple[tuple[float, ...], ...] = (
        (), (float("nan"),), (EPOCH_SELECTION_THRESHOLD_BELOW_RANGE,), (EPOCH_SELECTION_THRESHOLD_ABOVE_RANGE,))
    INVALID_PREDICTION_FIXTURES: tuple[dict[str, tuple[array[float], array[int]]], ...] = (
        {}, {"portable": (array("d"), array("B"))},
        {"portable": (array("d", (1, 0, 0, 0)), array("B", (0,)))},
        {"portable": (array("d", (1, 0, 0)), array("B", (1,)))},
        {"portable": (array("d", (1, 0, 0, 0)), array("B", (len(ACTIONS),)))},
        {"portable": (array("d", (float("nan"), 0, 0, 0)), array("B", (1,)))},
        {"portable": (array("d", (EPOCH_SELECTION_UNNORMALIZED_ROW_SCORE,) * len(ACTIONS)), array("B", (1,)))},
    )

    def test_reaching_runtime_confidence_beats_lower_log_loss(self) -> None:
        early = assess_epoch({"portable": profile(EPOCH_SELECTION_KEEP_ROW_SCORE, EPOCH_SELECTION_BELOW_THRESHOLD_SCORE)},
                             thresholds=(EPOCH_SELECTION_RUNTIME_THRESHOLD,), loss=EPOCH_SELECTION_DEVELOPMENT_LOSS_LOW)
        later = assess_epoch({"portable": profile(EPOCH_SELECTION_KEEP_ROW_SCORE_LATER, EPOCH_SELECTION_RUNTIME_CONFIDENCE_SCORE)},
                             thresholds=(EPOCH_SELECTION_RUNTIME_THRESHOLD,), loss=EPOCH_SELECTION_DEVELOPMENT_LOSS_HIGH)
        self.assertIsNone(early)  # nothing converts at .99: no repairs, so nothing to weigh
        assert later is not None
        self.assertEqual(later.by_profile["portable"]["converted_correctly"], 1)
        self.assertEqual((later.net_benefit, later.minimum_net_benefit), (1, 1))

    def test_an_epoch_that_breaks_as_much_as_it_repairs_is_not_selected(self) -> None:
        """The balance decides, per profile: one profile in the red rejects the epoch."""
        profiles = {"portable": profile(EPOCH_SELECTION_KEEP_ROW_SCORE, EPOCH_SELECTION_HIGH_CONFIDENCE_SCORE),
                    "reference": profile(EPOCH_SELECTION_HIGH_CONFIDENCE_SCORE, EPOCH_SELECTION_HIGH_CONFIDENCE_SCORE)}
        self.assertIsNone(assess_epoch(profiles, thresholds=(EPOCH_SELECTION_RUNTIME_THRESHOLD,), loss=EPOCH_SELECTION_LOSS_NOT_UNDER_TEST))
        # A higher threshold that converts nothing nets nothing either, so it does not qualify.
        self.assertIsNone(assess_epoch(profiles, thresholds=(EPOCH_SELECTION_RUNTIME_THRESHOLD, 1.0), loss=EPOCH_SELECTION_LOSS_NOT_UNDER_TEST))
        # Two repairs against one breakage in the weaker profile is a net gain.
        ahead = assess_epoch({"portable": profile(EPOCH_SELECTION_KEEP_ROW_SCORE, EPOCH_SELECTION_HIGH_CONFIDENCE_SCORE,
                              EPOCH_SELECTION_HIGH_CONFIDENCE_SCORE),
                              "reference": profile(EPOCH_SELECTION_HIGH_CONFIDENCE_SCORE, EPOCH_SELECTION_HIGH_CONFIDENCE_SCORE,
                              EPOCH_SELECTION_HIGH_CONFIDENCE_SCORE)}, thresholds=(EPOCH_SELECTION_RUNTIME_THRESHOLD,),
                              loss=EPOCH_SELECTION_LOSS_NOT_UNDER_TEST)
        assert ahead is not None
        self.assertEqual((ahead.minimum_net_benefit, ahead.net_benefit), (1, EPOCH_SELECTION_EXPECTED_NET_BENEFIT))

    def test_a_model_that_repairs_more_beats_one_that_merely_never_errs(self) -> None:
        """What the removed zero-budget rule could not express."""
        cautious = assess_epoch({"portable": profile(EPOCH_SELECTION_KEEP_ROW_SCORE, EPOCH_SELECTION_HIGH_CONFIDENCE_SCORE,
                                 EPOCH_SELECTION_AMBIGUOUS_SCORE, EPOCH_SELECTION_AMBIGUOUS_SCORE, EPOCH_SELECTION_AMBIGUOUS_SCORE)},
                                 thresholds=(EPOCH_SELECTION_RUNTIME_THRESHOLD,), loss=EPOCH_SELECTION_DEVELOPMENT_LOSS_LOW)
        bolder = assess_epoch({"portable": profile(EPOCH_SELECTION_HIGH_CONFIDENCE_SCORE, EPOCH_SELECTION_HIGH_CONFIDENCE_SCORE,
                               EPOCH_SELECTION_HIGH_CONFIDENCE_SCORE, EPOCH_SELECTION_HIGH_CONFIDENCE_SCORE, EPOCH_SELECTION_HIGH_CONFIDENCE_SCORE)},
                               thresholds=(EPOCH_SELECTION_RUNTIME_THRESHOLD,), loss=EPOCH_SELECTION_DEVELOPMENT_LOSS_LOW)
        assert cautious is not None and bolder is not None
        self.assertEqual((cautious.net_benefit, bolder.net_benefit), (1, EPOCH_SELECTION_EXPECTED_NET_BENEFIT))
        self.assertGreater(bolder.rank, cautious.rank)

    def test_selection_protects_the_weaker_profile_before_the_pooled_balance(self) -> None:
        uneven = assess_epoch({"portable": profile(EPOCH_SELECTION_KEEP_ROW_SCORE, EPOCH_SELECTION_HIGH_CONFIDENCE_SCORE,
                               EPOCH_SELECTION_HIGH_CONFIDENCE_SCORE, EPOCH_SELECTION_HIGH_CONFIDENCE_SCORE),
                               "reference": profile(EPOCH_SELECTION_KEEP_ROW_SCORE, EPOCH_SELECTION_HIGH_CONFIDENCE_SCORE,
                               EPOCH_SELECTION_AMBIGUOUS_SCORE, EPOCH_SELECTION_AMBIGUOUS_SCORE)},
                              thresholds=(EPOCH_SELECTION_RUNTIME_THRESHOLD,), loss=EPOCH_SELECTION_DEVELOPMENT_LOSS_LOW)
        balanced = assess_epoch({"portable": profile(EPOCH_SELECTION_KEEP_ROW_SCORE, EPOCH_SELECTION_HIGH_CONFIDENCE_SCORE,
                                 EPOCH_SELECTION_HIGH_CONFIDENCE_SCORE, EPOCH_SELECTION_AMBIGUOUS_SCORE),
                                 "reference": profile(EPOCH_SELECTION_KEEP_ROW_SCORE, EPOCH_SELECTION_HIGH_CONFIDENCE_SCORE,
                                 EPOCH_SELECTION_HIGH_CONFIDENCE_SCORE, EPOCH_SELECTION_AMBIGUOUS_SCORE)},
                                thresholds=(EPOCH_SELECTION_RUNTIME_THRESHOLD,), loss=EPOCH_SELECTION_DEVELOPMENT_LOSS_HIGH)
        assert uneven is not None and balanced is not None
        self.assertEqual((uneven.minimum_net_benefit, balanced.minimum_net_benefit),
                         (1, EPOCH_SELECTION_EXPECTED_MINIMUM_NET_BENEFIT))
        self.assertGreater(balanced.rank, uneven.rank)

    def test_equal_runtime_outcomes_use_loss_then_lower_threshold(self) -> None:
        predictions = {"portable": profile(EPOCH_SELECTION_KEEP_ROW_SCORE, EPOCH_SELECTION_HIGH_CONFIDENCE_SCORE)}
        first = assess_epoch(predictions, thresholds=(EPOCH_SELECTION_ALTERNATE_THRESHOLD, EPOCH_SELECTION_RUNTIME_THRESHOLD),
                             loss=EPOCH_SELECTION_DEVELOPMENT_LOSS_HIGH)
        second = assess_epoch(predictions, thresholds=(EPOCH_SELECTION_RUNTIME_THRESHOLD, EPOCH_SELECTION_ALTERNATE_THRESHOLD),
                              loss=EPOCH_SELECTION_DEVELOPMENT_LOSS_LOW)
        assert first is not None and second is not None
        self.assertEqual(first.threshold, EPOCH_SELECTION_RUNTIME_THRESHOLD)
        self.assertGreater(second.rank, first.rank)
        self.assertEqual(second, assess_epoch(predictions, thresholds=(EPOCH_SELECTION_ALTERNATE_THRESHOLD, EPOCH_SELECTION_RUNTIME_THRESHOLD),
                         loss=EPOCH_SELECTION_DEVELOPMENT_LOSS_LOW))

    def test_abstention_and_exact_threshold_follow_serving_actions(self) -> None:
        selected = assess_epoch({"portable": (self.ABSTENTION_ROW_SCORES, self.ABSTENTION_ROW_LABELS)},
                                thresholds=(EPOCH_SELECTION_RUNTIME_THRESHOLD,), loss=EPOCH_SELECTION_VALID_LOSS)
        assert selected is not None
        self.assertEqual(selected.by_profile["portable"]["converted_correctly"], 1)
        self.assertEqual(selected.by_profile["portable"]["false_conversions"], 0)
        self.assertEqual(selected.minimum_recall, EPOCH_SELECTION_EXPECTED_MINIMUM_RECALL)

    def test_invalid_or_empty_evidence_cannot_select_an_epoch(self) -> None:
        valid = {"portable": profile(EPOCH_SELECTION_KEEP_ROW_SCORE, EPOCH_SELECTION_HIGH_CONFIDENCE_SCORE)}
        for loss in (float("nan"), float("inf"), -1.0):
            with self.subTest(loss=loss), self.assertRaises(ValueError):
                assess_epoch(valid, thresholds=(EPOCH_SELECTION_RUNTIME_THRESHOLD,), loss=loss)
        for thresholds in self.INVALID_THRESHOLD_SETS:
            with self.subTest(thresholds=thresholds), self.assertRaises(ValueError):
                assess_epoch(valid, thresholds=thresholds, loss=EPOCH_SELECTION_VALID_LOSS)
        for predictions in self.INVALID_PREDICTION_FIXTURES:
            with self.subTest(predictions=predictions), self.assertRaises(ValueError):
                assess_epoch(predictions, thresholds=(EPOCH_SELECTION_RUNTIME_THRESHOLD,), loss=EPOCH_SELECTION_VALID_LOSS)


if __name__ == "__main__":
    unittest.main()
