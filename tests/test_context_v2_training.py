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


def frame(split: str, family: str = "token") -> Frame:
    return Frame(split + family, split + family, split, "eng", family, "токен", 0, "a ", "", "", "unknown", "space", "keep", family, "fixture")


class ContextV2TrainingTests(unittest.TestCase):
    @unittest.skipUnless(sys.platform != "win32" and (shutil.which("gcc") or shutil.which("cc")), "training-only Linux C kernel")
    def test_native_epoch_matches_python_reference_and_empty_prediction(self) -> None:
        data = Packed.build([({"a": 1.0, "b": -0.5}, 1, 1.4), ({"c": 4.0, "b": 0.9}, 0, 0.5), ({"a": 0.8}, 3, 2.0)], ["a", "b", "c"])
        first, second = array("d", [0.0]) * 12, array("d", [0.0]) * 12
        accum1, accum2 = array("d", [1.0]) * 12, array("d", [1.0]) * 12
        kernel = Kernel.load()
        for _ in range(4):
            python_epoch(data, first, accum1, 0.08)
            kernel.epoch(data, second, accum2, 0.08)
        for expected, actual in zip(first, second):
            self.assertAlmostEqual(expected, actual, places=14)
        for expected, actual in zip(accum1, accum2):
            self.assertAlmostEqual(expected, actual, places=14)
        scores = kernel.predict(data, second)
        for row in range(3):
            self.assertAlmostEqual(sum(scores[row * 4:row * 4 + 4]), 1.0, places=14)
        self.assertEqual(list(kernel.predict(Packed.build([], []), array("d"))), [])

    def test_packed_rejects_invalid_numeric_inputs(self) -> None:
        for label, weight in ((-1, 1.0), (4, 1.0), (0, -1.0), (0, math.inf)):
            with self.assertRaises(ValueError):
                Packed.build([({"x": 1.0}, label, weight)], ["x"])
        with self.assertRaises(ValueError):
            Packed.build([({"x": math.nan}, 0, 1.0)], ["x"])

    def test_partition_audit_and_training_profile_are_deterministic(self) -> None:
        rows = [frame("train"), frame("development"), frame("calibration"), frame("test"), frame("lexical_test", "unseen")]
        self.assertEqual(audit(rows)["source_group_overlap"], 0)
        self.assertEqual(len(samples(rows, "train")), 1)
        self.assertEqual(len(samples(rows, "test")), 2)
        self.assertEqual(samples(rows, "train"), samples(list(reversed(rows)), "train"))
        with self.assertRaisesRegex(ValueError, "source-group"):
            audit(rows + [replace(rows[0], split="test")])
        with self.assertRaisesRegex(ValueError, "focus-family"):
            audit(rows + [replace(rows[0], split="lexical_test", cluster="different")])

    def test_threshold_uses_safety_budget_and_rejects_saturation(self) -> None:
        scores = array("d", [0.001, 0.995, 0.003, 0.001, 0.0, 0.99995, 0.00005, 0.0])
        threshold, metrics = select_threshold(scores, array("B", [0, 1]), [0.99, 0.999, 1.0], 0)
        self.assertEqual(threshold, 0.999)
        self.assertEqual(metrics["converted_correctly"], 1)
        with self.assertRaises(ValueError):
            select_threshold(array("d", [0, 1, 0, 0]), array("B", [0]), [1.0], 0)

    def test_promotion_cannot_trade_more_false_conversions_for_recall(self) -> None:
        gate = config()["promotion"]
        assert isinstance(gate, dict)
        counts = {"rows": 1000, "desired_conversions": 500, "converted_correctly": 490,
            "false_conversions": 0, "baseline_false_conversions": 0}
        metrics: dict[str, object] = {"counts": counts, "categories": {}}
        self.assertEqual(promotion_failures(metrics, metrics, gate), [])
        worse: dict[str, object] = {"counts": {**counts, "converted_correctly": 499, "false_conversions": 2}, "categories": {}}
        self.assertIn("more false conversions than v1", promotion_failures(worse, metrics, gate))
        fewer: dict[str, object] = {"counts": {**counts, "converted_correctly": 480}, "categories": {}}
        self.assertIn("fewer correct conversions than v1", promotion_failures(fewer, metrics, gate))


if __name__ == "__main__":
    unittest.main()
