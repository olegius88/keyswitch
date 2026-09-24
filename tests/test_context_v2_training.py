"""Large-corpus fitting boundaries, optimizer parity and fail-closed gates."""
from __future__ import annotations

import math
import shutil
import sys
import unittest
from array import array
from pathlib import Path
from dataclasses import replace

TOOLS_PATH = str(Path(__file__).resolve().parents[1] / "tools")
if TOOLS_PATH not in sys.path:
    sys.path.insert(0, TOOLS_PATH)

from context_frames import Frame
from context_optimizer import Kernel, Packed, python_epoch
from train_context_v2 import audit, config, promotion_failures, samples, select_threshold
from fixture_values.corpora import CONTEXT_V2_BASE_PROMOTION_COUNTS, CONTEXT_V2_KERNEL_TRAINING_ROWS
from fixture_values.counts import (
    CONTEXT_V2_EXPECTED_TEST_SAMPLES,
    CONTEXT_V2_FEWER_CONVERTED_CORRECTLY,
    CONTEXT_V2_KERNEL_COMPARISON_PLACES,
    CONTEXT_V2_KERNEL_EPOCHS,
    CONTEXT_V2_KERNEL_WEIGHT_COUNT,
    CONTEXT_V2_WORSE_CONVERTED_CORRECTLY,
    CONTEXT_V2_WORSE_FALSE_CONVERSIONS,
)
from fixture_values.models import CONTEXT_ACTION_CLASS_COUNT
from fixture_values.scores import (
    CONTEXT_V2_EXPECTED_THRESHOLD,
    CONTEXT_V2_KERNEL_LEARNING_RATE,
    CONTEXT_V2_LOW_THRESHOLD_CANDIDATE,
    CONTEXT_V2_THRESHOLD_CANDIDATES,
    CONTEXT_V2_THRESHOLD_ROW_SCORES,
)

FEATURE_NAMES = ["a", "b", "c"]
ROW_COUNT = len(CONTEXT_V2_KERNEL_TRAINING_ROWS)
CANDIDATE_SCORES = array("d", CONTEXT_V2_THRESHOLD_ROW_SCORES)


def frame(split: str, family: str = "token") -> Frame:
    return Frame(split + family, split + family, split, "eng", family, "токен", 0, "a ", "", "", "unknown", "space", "keep", family, "fixture")


class ContextV2TrainingTests(unittest.TestCase):
    @unittest.skipUnless(sys.platform != "win32" and (shutil.which("gcc") or shutil.which("cc")), "training-only Linux C kernel")
    def test_native_epoch_matches_python_reference_and_empty_prediction(self) -> None:
        data = Packed.build(CONTEXT_V2_KERNEL_TRAINING_ROWS, FEATURE_NAMES)
        first, second = array("d", [0.0]) * CONTEXT_V2_KERNEL_WEIGHT_COUNT, array("d", [0.0]) * CONTEXT_V2_KERNEL_WEIGHT_COUNT
        accum1, accum2 = array("d", [1.0]) * CONTEXT_V2_KERNEL_WEIGHT_COUNT, array("d", [1.0]) * CONTEXT_V2_KERNEL_WEIGHT_COUNT
        kernel = Kernel.load()
        for _ in range(CONTEXT_V2_KERNEL_EPOCHS):
            python_epoch(data, first, accum1, CONTEXT_V2_KERNEL_LEARNING_RATE)
            kernel.epoch(data, second, accum2, CONTEXT_V2_KERNEL_LEARNING_RATE)
        for expected, actual in zip(first, second):
            self.assertAlmostEqual(expected, actual, places=CONTEXT_V2_KERNEL_COMPARISON_PLACES)
        for expected, actual in zip(accum1, accum2):
            self.assertAlmostEqual(expected, actual, places=CONTEXT_V2_KERNEL_COMPARISON_PLACES)
        scores = kernel.predict(data, second)
        for row in range(ROW_COUNT):
            self.assertAlmostEqual(
                sum(scores[row * CONTEXT_ACTION_CLASS_COUNT:row * CONTEXT_ACTION_CLASS_COUNT + CONTEXT_ACTION_CLASS_COUNT]),
                1.0,
                places=CONTEXT_V2_KERNEL_COMPARISON_PLACES,
            )
        self.assertEqual(list(kernel.predict(Packed.build([], []), array("d"))), [])

    def test_packed_rejects_invalid_numeric_inputs(self) -> None:
        for label, weight in ((-1, 1.0), (CONTEXT_ACTION_CLASS_COUNT, 1.0), (0, -1.0), (0, math.inf)):
            with self.assertRaises(ValueError):
                Packed.build([({"x": 1.0}, label, weight)], ["x"])
        with self.assertRaises(ValueError):
            Packed.build([({"x": math.nan}, 0, 1.0)], ["x"])

    def test_partition_audit_and_training_profile_are_deterministic(self) -> None:
        rows = [frame("train"), frame("development"), frame("calibration"), frame("test"), frame("lexical_test", "unseen")]
        self.assertEqual(audit(rows)["source_group_overlap"], 0)
        self.assertEqual(len(samples(rows, "train")), 1)
        self.assertEqual(len(samples(rows, "test")), CONTEXT_V2_EXPECTED_TEST_SAMPLES)
        self.assertEqual(samples(rows, "train"), samples(list(reversed(rows)), "train"))
        with self.assertRaisesRegex(ValueError, "source-group"):
            audit(rows + [replace(rows[0], split="test")])
        with self.assertRaisesRegex(ValueError, "focus-family"):
            audit(rows + [replace(rows[0], split="lexical_test", cluster="different")])

    def test_threshold_uses_safety_budget_and_rejects_saturation(self) -> None:
        scores = CANDIDATE_SCORES
        threshold, metrics = select_threshold(scores, array("B", [0, 1]), CONTEXT_V2_THRESHOLD_CANDIDATES, 0)
        self.assertEqual(threshold, CONTEXT_V2_EXPECTED_THRESHOLD)
        self.assertEqual(metrics["converted_correctly"], 1)
        with self.assertRaises(ValueError):
            select_threshold(array("d", [0, 1, 0, 0]), array("B", [0]), [1.0], 0)

    def test_promotion_cannot_trade_more_false_conversions_for_recall(self) -> None:
        gate = config()["promotion"]
        assert isinstance(gate, dict)
        counts = CONTEXT_V2_BASE_PROMOTION_COUNTS
        metrics: dict[str, object] = {"counts": counts, "categories": {}}
        self.assertEqual(promotion_failures(metrics, metrics, gate), [])
        worse: dict[str, object] = {
            "counts": {
                **counts,
                "converted_correctly": CONTEXT_V2_WORSE_CONVERTED_CORRECTLY,
                "false_conversions": CONTEXT_V2_WORSE_FALSE_CONVERSIONS,
            },
            "categories": {},
        }
        self.assertIn("more false conversions than v1", promotion_failures(worse, metrics, gate))
        fewer: dict[str, object] = {"counts": {**counts, "converted_correctly": CONTEXT_V2_FEWER_CONVERTED_CORRECTLY}, "categories": {}}
        self.assertIn("fewer correct conversions than v1", promotion_failures(fewer, metrics, gate))


if __name__ == "__main__":
    unittest.main()
