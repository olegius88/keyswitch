"""The installed artifacts never fall below the quality the shipped ones reached.

The model protocol accepts a candidate pair that beats the frozen context-v1 + prefix-v1
pair; it does not compare a candidate with the pair it replaces, so a retrained model
could ship worse than the installed one and still pass every gate. These tests read the
shipped evidence (the release receipt, the frozen engine replays) and hold it to the
floors in `fixture_values/quality_floors.py`, which only move up.
"""

from __future__ import annotations

import json
import re
import unittest
from pathlib import Path
from typing import ClassVar, cast

from fixture_values.quality_floors import (
    BOUNDARY_MAX_CHANGED_CORRECT,
    BOUNDARY_MAX_INJECTIONS_BEFORE_HARD_BOUNDARY,
    BOUNDARY_MAX_LENGTH_MISMATCHES,
    BOUNDARY_MIN_EXACT,
    MAX_DISCLOSED_EXPECTED_FAILURES,
    PREFIX_MAX_CHANGED_CORRECT,
    PREFIX_MAX_LENGTH_MISMATCHES,
    PREFIX_MIN_EARLY_RESTORED,
    PREFIX_MIN_EXACT,
    PREFIX_MIN_RESTORED,
    RECEIPT_MAX_CALIBRATION_FALSE_CONVERSIONS,
    RECEIPT_MAX_CORRECTION_LAYOUT_MISMATCHES,
    RECEIPT_MAX_CORRUPTIONS,
    RECEIPT_MAX_EXECUTION_ERRORS,
    RECEIPT_MAX_LENGTH_MISMATCHES,
    RECEIPT_MIN_CALIBRATION_RECALL,
    RECEIPT_MIN_CONVERSION_THRESHOLD,
    RECEIPT_MIN_PREFIX_CONVERSION_THRESHOLD,
    RECEIPT_MIN_RESTORATION_MARGIN,
)

ROOT = Path(__file__).resolve().parents[1]
RECEIPT_PATH = ROOT / "model" / "context_v3" / "release-receipt.json"
PREFIX_REPORT_PATH = ROOT / "model" / "prefix_v1" / "engine-report.json"
BOUNDARY_REPORT_PATH = ROOT / "model" / "boundary_v1" / "engine-regression.json"
TESTS_DIR = ROOT / "tests"
DISCLOSURE_DECORATOR = re.compile(r"^\s*@disclosed_schema3_pair_regression\s*$", re.MULTILINE)


Section = dict[str, object]


def load(path: Path) -> Section:
    return cast(Section, json.loads(path.read_text(encoding="utf-8")))


def section(data: Section, *keys: str) -> Section:
    for key in keys:
        data = cast(Section, data[key])
    return data


def number(data: Section, key: str) -> float:
    value = data[key]
    assert isinstance(value, (int, float)) and not isinstance(value, bool), (key, value)
    return value


def counts(data: Section, *keys: str) -> dict[str, int]:
    return {name: int(number(section(data, *keys), name)) for name in section(data, *keys)}


def mode_counts(profile: Section) -> dict[str, dict[str, dict[str, int]]]:
    """The candidate and baseline counts of a profile, by settings mode."""

    return {
        "default": {side: counts(profile, "counts", side) for side in ("candidate", "baseline")},
        "early_off": {side: counts(profile, "early_off", "counts", side) for side in ("candidate", "baseline")},
    }


class ReleaseReceiptFloorTests(unittest.TestCase):
    receipt: ClassVar[Section]

    @classmethod
    def setUpClass(cls) -> None:
        cls.receipt = load(RECEIPT_PATH)

    def test_the_receipt_passed_its_own_gates(self) -> None:
        self.assertIs(self.receipt["quality_gates_passed"], True)
        self.assertGreaterEqual(number(self.receipt, "conversion_threshold"), RECEIPT_MIN_CONVERSION_THRESHOLD)
        self.assertGreaterEqual(
            number(self.receipt, "prefix_conversion_threshold"), RECEIPT_MIN_PREFIX_CONVERSION_THRESHOLD)

    def test_the_installed_pair_corrupts_nothing_and_executes_every_row(self) -> None:
        for name, profile in section(self.receipt, "test", "profiles").items():
            for mode, modal in mode_counts(cast(Section, profile)).items():
                candidate = modal["candidate"]
                with self.subTest(profile=name, mode=mode):
                    self.assertLessEqual(candidate["correct_text_corruptions"], RECEIPT_MAX_CORRUPTIONS)
                    self.assertEqual(candidate["preserved_correct"], candidate["initially_correct"])
                    self.assertLessEqual(candidate["execution_errors"], RECEIPT_MAX_EXECUTION_ERRORS)
                    self.assertLessEqual(candidate["length_mismatches"], RECEIPT_MAX_LENGTH_MISMATCHES)
                    self.assertLessEqual(
                        candidate["correction_layout_mismatches"], RECEIPT_MAX_CORRECTION_LAYOUT_MISMATCHES)

    def test_the_installed_pair_keeps_its_margin_over_the_frozen_baseline(self) -> None:
        for name, profile in section(self.receipt, "test", "profiles").items():
            for mode, modal in mode_counts(cast(Section, profile)).items():
                margin = modal["candidate"]["exactly_restored"] - modal["baseline"]["exactly_restored"]
                with self.subTest(profile=name, mode=mode):
                    self.assertGreaterEqual(margin, RECEIPT_MIN_RESTORATION_MARGIN[mode])

    def test_calibration_recall_and_false_conversions_hold(self) -> None:
        calibration = section(self.receipt, "calibration")
        self.assertGreaterEqual(number(calibration, "conversion_recall"), RECEIPT_MIN_CALIBRATION_RECALL)
        for name, limit in RECEIPT_MAX_CALIBRATION_FALSE_CONVERSIONS.items():
            with self.subTest(profile=name):
                self.assertLessEqual(number(section(calibration, "by_profile", name), "false_conversions"), limit)


class EngineReplayFloorTests(unittest.TestCase):
    def test_the_prefix_replay_restores_at_least_as_much_as_shipped(self) -> None:
        report = load(PREFIX_REPORT_PATH)
        self.assertIs(report["passed"], True)
        for name in section(report, "results"):
            candidate = counts(report, "results", name, "candidate")
            without = counts(report, "results", name, "shipping_no_prefix")
            with self.subTest(profile=name):
                self.assertGreaterEqual(candidate["exact"], PREFIX_MIN_EXACT)
                self.assertGreaterEqual(candidate["restored"], PREFIX_MIN_RESTORED)
                self.assertGreaterEqual(candidate["early_restored"], PREFIX_MIN_EARLY_RESTORED)
                self.assertLessEqual(candidate["changed_correct"], PREFIX_MAX_CHANGED_CORRECT)
                self.assertLessEqual(candidate["length_mismatches"], PREFIX_MAX_LENGTH_MISMATCHES)
                # The prefix model may never cost a restoration the engine made without it.
                self.assertGreaterEqual(candidate["exact"], without["exact"])
                self.assertGreaterEqual(candidate["restored"], without["restored"])
                self.assertLessEqual(candidate["changed_correct"], without["changed_correct"])

    def test_the_boundary_replay_holds_every_hard_boundary(self) -> None:
        active = section(load(BOUNDARY_REPORT_PATH), "results", "active_v2")
        rows = active["rows"]
        assert isinstance(rows, list)
        self.assertGreaterEqual(number(active, "exact"), BOUNDARY_MIN_EXACT)
        self.assertLessEqual(number(active, "changed_correct"), BOUNDARY_MAX_CHANGED_CORRECT)
        self.assertLessEqual(
            number(active, "injections_before_hard_boundary"), BOUNDARY_MAX_INJECTIONS_BEFORE_HARD_BOUNDARY)
        self.assertLessEqual(number(active, "length_mismatches"), BOUNDARY_MAX_LENGTH_MISMATCHES)
        self.assertEqual(number(active, "exact"), len(rows))


class DisclosedExpectationFloorTests(unittest.TestCase):
    def test_disclosed_expected_failures_do_not_grow_quietly(self) -> None:
        disclosed = sum(
            len(DISCLOSURE_DECORATOR.findall(path.read_text(encoding="utf-8")))
            for path in TESTS_DIR.glob("test_*.py")
        )
        self.assertLessEqual(disclosed, MAX_DISCLOSED_EXPECTED_FAILURES)


if __name__ == "__main__":
    unittest.main()
