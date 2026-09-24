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
    # The runtime confidence threshold almost every call below selects at, and a
    # second, higher candidate offered alongside it in the tie-break test.
    RUNTIME_THRESHOLD = .99
    ALTERNATE_THRESHOLD = .995

    # Fixture scores fed through `profile()`. KEEP_ROW_SCORE is the negligible score
    # given to a profile's one keep-labeled row so it never trips a false conversion;
    # KEEP_ROW_SCORE_LATER plays the same role with a second value so the "early" and
    # "later" epochs below are built from distinguishable data. BELOW_THRESHOLD_SCORE
    # and RUNTIME_CONFIDENCE_SCORE are convert-row scores that respectively miss and
    # reach RUNTIME_THRESHOLD; HIGH_CONFIDENCE_SCORE is comfortably past it either way
    # a row is labeled; AMBIGUOUS_SCORE is a coin-flip the model never commits on.
    KEEP_ROW_SCORE = .001
    KEEP_ROW_SCORE_LATER = .002
    BELOW_THRESHOLD_SCORE = .98
    RUNTIME_CONFIDENCE_SCORE = .995
    HIGH_CONFIDENCE_SCORE = .999
    AMBIGUOUS_SCORE = .5

    # Development loss values. LOW and HIGH are genuinely compared against each
    # other; LOSS_NOT_UNDER_TEST and VALID_LOSS are placeholders the assertions
    # never inspect, kept apart because their calls isolate different parameters.
    DEVELOPMENT_LOSS_LOW = .01
    DEVELOPMENT_LOSS_HIGH = .02
    LOSS_NOT_UNDER_TEST = .001
    VALID_LOSS = .1

    # Pinned expected results.
    EXPECTED_NET_BENEFIT = 3
    EXPECTED_MINIMUM_NET_BENEFIT = 2
    EXPECTED_MINIMUM_RECALL = .5

    # Fixture tables.
    ABSTENTION_ROW_SCORES = array("d", (.0, .99, .01, .0, .0, .1, .0, .9, .0, .01, .99, .0))
    ABSTENTION_ROW_LABELS = array("B", (1, 1, 2))
    INVALID_THRESHOLD_SETS: tuple[tuple[float, ...], ...] = ((), (float("nan"),), (-.1,), (1.1,))
    INVALID_PREDICTION_FIXTURES: tuple[dict[str, tuple[array[float], array[int]]], ...] = (
        {}, {"portable": (array("d"), array("B"))},
        {"portable": (array("d", (1, 0, 0, 0)), array("B", (0,)))},
        {"portable": (array("d", (1, 0, 0)), array("B", (1,)))},
        {"portable": (array("d", (1, 0, 0, 0)), array("B", (4,)))},
        {"portable": (array("d", (float("nan"), 0, 0, 0)), array("B", (1,)))},
        {"portable": (array("d", (.1, .1, .1, .1)), array("B", (1,)))},
    )

    def test_reaching_runtime_confidence_beats_lower_log_loss(self) -> None:
        early = assess_epoch({"portable": profile(self.KEEP_ROW_SCORE, self.BELOW_THRESHOLD_SCORE)},
                             thresholds=(self.RUNTIME_THRESHOLD,), loss=self.DEVELOPMENT_LOSS_LOW)
        later = assess_epoch({"portable": profile(self.KEEP_ROW_SCORE_LATER, self.RUNTIME_CONFIDENCE_SCORE)},
                             thresholds=(self.RUNTIME_THRESHOLD,), loss=self.DEVELOPMENT_LOSS_HIGH)
        self.assertIsNone(early)  # nothing converts at .99: no repairs, so nothing to weigh
        assert later is not None
        self.assertEqual(later.by_profile["portable"]["converted_correctly"], 1)
        self.assertEqual((later.net_benefit, later.minimum_net_benefit), (1, 1))

    def test_an_epoch_that_breaks_as_much_as_it_repairs_is_not_selected(self) -> None:
        """The balance decides, per profile: one profile in the red rejects the epoch."""
        profiles = {"portable": profile(self.KEEP_ROW_SCORE, self.HIGH_CONFIDENCE_SCORE),
                    "reference": profile(self.HIGH_CONFIDENCE_SCORE, self.HIGH_CONFIDENCE_SCORE)}
        self.assertIsNone(assess_epoch(profiles, thresholds=(self.RUNTIME_THRESHOLD,), loss=self.LOSS_NOT_UNDER_TEST))
        # A higher threshold that converts nothing nets nothing either, so it does not qualify.
        self.assertIsNone(assess_epoch(profiles, thresholds=(self.RUNTIME_THRESHOLD, 1.0), loss=self.LOSS_NOT_UNDER_TEST))
        # Two repairs against one breakage in the weaker profile is a net gain.
        ahead = assess_epoch({"portable": profile(self.KEEP_ROW_SCORE, self.HIGH_CONFIDENCE_SCORE,
                              self.HIGH_CONFIDENCE_SCORE),
                              "reference": profile(self.HIGH_CONFIDENCE_SCORE, self.HIGH_CONFIDENCE_SCORE,
                              self.HIGH_CONFIDENCE_SCORE)}, thresholds=(self.RUNTIME_THRESHOLD,),
                              loss=self.LOSS_NOT_UNDER_TEST)
        assert ahead is not None
        self.assertEqual((ahead.minimum_net_benefit, ahead.net_benefit), (1, self.EXPECTED_NET_BENEFIT))

    def test_a_model_that_repairs_more_beats_one_that_merely_never_errs(self) -> None:
        """What the removed zero-budget rule could not express."""
        cautious = assess_epoch({"portable": profile(self.KEEP_ROW_SCORE, self.HIGH_CONFIDENCE_SCORE,
                                 self.AMBIGUOUS_SCORE, self.AMBIGUOUS_SCORE, self.AMBIGUOUS_SCORE)},
                                 thresholds=(self.RUNTIME_THRESHOLD,), loss=self.DEVELOPMENT_LOSS_LOW)
        bolder = assess_epoch({"portable": profile(self.HIGH_CONFIDENCE_SCORE, self.HIGH_CONFIDENCE_SCORE,
                               self.HIGH_CONFIDENCE_SCORE, self.HIGH_CONFIDENCE_SCORE, self.HIGH_CONFIDENCE_SCORE)},
                               thresholds=(self.RUNTIME_THRESHOLD,), loss=self.DEVELOPMENT_LOSS_LOW)
        assert cautious is not None and bolder is not None
        self.assertEqual((cautious.net_benefit, bolder.net_benefit), (1, self.EXPECTED_NET_BENEFIT))
        self.assertGreater(bolder.rank, cautious.rank)

    def test_selection_protects_the_weaker_profile_before_the_pooled_balance(self) -> None:
        uneven = assess_epoch({"portable": profile(self.KEEP_ROW_SCORE, self.HIGH_CONFIDENCE_SCORE,
                               self.HIGH_CONFIDENCE_SCORE, self.HIGH_CONFIDENCE_SCORE),
                               "reference": profile(self.KEEP_ROW_SCORE, self.HIGH_CONFIDENCE_SCORE,
                               self.AMBIGUOUS_SCORE, self.AMBIGUOUS_SCORE)},
                              thresholds=(self.RUNTIME_THRESHOLD,), loss=self.DEVELOPMENT_LOSS_LOW)
        balanced = assess_epoch({"portable": profile(self.KEEP_ROW_SCORE, self.HIGH_CONFIDENCE_SCORE,
                                 self.HIGH_CONFIDENCE_SCORE, self.AMBIGUOUS_SCORE),
                                 "reference": profile(self.KEEP_ROW_SCORE, self.HIGH_CONFIDENCE_SCORE,
                                 self.HIGH_CONFIDENCE_SCORE, self.AMBIGUOUS_SCORE)},
                                thresholds=(self.RUNTIME_THRESHOLD,), loss=self.DEVELOPMENT_LOSS_HIGH)
        assert uneven is not None and balanced is not None
        self.assertEqual((uneven.minimum_net_benefit, balanced.minimum_net_benefit),
                         (1, self.EXPECTED_MINIMUM_NET_BENEFIT))
        self.assertGreater(balanced.rank, uneven.rank)

    def test_equal_runtime_outcomes_use_loss_then_lower_threshold(self) -> None:
        predictions = {"portable": profile(self.KEEP_ROW_SCORE, self.HIGH_CONFIDENCE_SCORE)}
        first = assess_epoch(predictions, thresholds=(self.ALTERNATE_THRESHOLD, self.RUNTIME_THRESHOLD),
                             loss=self.DEVELOPMENT_LOSS_HIGH)
        second = assess_epoch(predictions, thresholds=(self.RUNTIME_THRESHOLD, self.ALTERNATE_THRESHOLD),
                              loss=self.DEVELOPMENT_LOSS_LOW)
        assert first is not None and second is not None
        self.assertEqual(first.threshold, self.RUNTIME_THRESHOLD)
        self.assertGreater(second.rank, first.rank)
        self.assertEqual(second, assess_epoch(predictions, thresholds=(self.ALTERNATE_THRESHOLD, self.RUNTIME_THRESHOLD),
                         loss=self.DEVELOPMENT_LOSS_LOW))

    def test_abstention_and_exact_threshold_follow_serving_actions(self) -> None:
        selected = assess_epoch({"portable": (self.ABSTENTION_ROW_SCORES, self.ABSTENTION_ROW_LABELS)},
                                thresholds=(self.RUNTIME_THRESHOLD,), loss=self.VALID_LOSS)
        assert selected is not None
        self.assertEqual(selected.by_profile["portable"]["converted_correctly"], 1)
        self.assertEqual(selected.by_profile["portable"]["false_conversions"], 0)
        self.assertEqual(selected.minimum_recall, self.EXPECTED_MINIMUM_RECALL)

    def test_invalid_or_empty_evidence_cannot_select_an_epoch(self) -> None:
        valid = {"portable": profile(self.KEEP_ROW_SCORE, self.HIGH_CONFIDENCE_SCORE)}
        for loss in (float("nan"), float("inf"), -1.0):
            with self.subTest(loss=loss), self.assertRaises(ValueError):
                assess_epoch(valid, thresholds=(self.RUNTIME_THRESHOLD,), loss=loss)
        for thresholds in self.INVALID_THRESHOLD_SETS:
            with self.subTest(thresholds=thresholds), self.assertRaises(ValueError):
                assess_epoch(valid, thresholds=thresholds, loss=self.VALID_LOSS)
        for predictions in self.INVALID_PREDICTION_FIXTURES:
            with self.subTest(predictions=predictions), self.assertRaises(ValueError):
                assess_epoch(predictions, thresholds=(self.RUNTIME_THRESHOLD,), loss=self.VALID_LOSS)


if __name__ == "__main__":
    unittest.main()
