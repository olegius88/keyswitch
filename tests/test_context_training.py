"""Reproducibility, family split and artifact provenance regression tests."""

from __future__ import annotations

import copy
import json
import sys
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path
from typing import cast
from unittest.mock import patch

from keyswitch.context_model import ACTIONS, ContextEvidence, ContextModel
from keyswitch.input_context import FieldContext

TOOLS_PATH = str(Path(__file__).resolve().parents[1] / "tools")
if TOOLS_PATH not in sys.path:
    sys.path.insert(0, TOOLS_PATH)

import train_context_model as trainer
import verify_context_model as verifier


class ContextTrainingTests(unittest.TestCase):
    def test_family_split_keeps_variants_and_reserves_unseen_test_tokens(self) -> None:
        development = trainer.build_corpus()
        held_out = trainer.build_corpus(trainer.HOLDOUT, held_out=True)
        seen: dict[str, str] = {}
        for row in development:
            self.assertEqual(seen.setdefault(row.family, row.split), row.split)
            self.assertIn(row.split, {"train", "development"})
        self.assertGreater(len(held_out), 10000)
        self.assertFalse(set(seen) & {row.family for row in held_out})
        self.assertEqual({row.split for row in held_out}, {"test"})
        self.assertEqual(trainer.family_split("test"), trainer.family_split("test"))

    def test_optimizer_is_reproducible_and_never_trains_on_test_rows(self) -> None:
        rows = [
            trainer.Row(ContextEvidence(action, action, 0, FieldContext("test", "1", action)), action, action, split, "fixture")
            for action in ACTIONS for split in ("train", "development")
        ]
        with patch.object(trainer, "EPOCHS", 2):
            first = trainer.train(rows)
            second = trainer.train(rows + [replace(rows[0], split="test", action="convert")])
        self.assertEqual(first, second)
        self.assertTrue(first[0])
        with self.assertRaises(ValueError):
            trainer.train([])
        model = ContextModel({name: tuple(values) for name, values in first[0].items()}, "test")
        metrics = trainer.evaluate(model, rows, "development")
        self.assertEqual(cast(dict[str, int], metrics["counts"])["rows"], 4)

    def test_bundled_report_is_bound_to_artifact_and_quality_counts(self) -> None:
        valid = verifier.verify()
        self.assertTrue(str(valid["model_version"]).startswith("context-v1-"))
        original = cast(dict[str, object], json.loads(verifier.REPORT.read_bytes()))
        with tempfile.TemporaryDirectory() as temporary:
            report_path = Path(temporary) / "report.json"
            variants: list[object] = [[], {**original, "quality_gates_passed": False},
                {**original, "test_overlap": 1}, {**original, "artifact_sha256": "tampered"},
                {**original, "model_version": "other"}, {**original, "test": None}]
            for field, bad_value in (("rows", 0), ("false_conversions", 1), ("converted_correctly", -1), ("rows", True)):
                variant = copy.deepcopy(original)
                counts = cast(dict[str, object], cast(dict[str, object], variant["test"])["counts"])
                counts[field] = bad_value
                variants.append(variant)
            for payload in variants:
                report_path.write_text(json.dumps(payload), encoding="utf-8")
                with self.assertRaises(ValueError):
                    verifier.verify(report_path=report_path)

    def test_training_rejects_invalid_scenarios_and_missing_baseline(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "scenarios.json"
            path.write_text("[]", encoding="utf-8")
            with self.assertRaises(ValueError):
                trainer.build_corpus(path)
        with patch("train_context_model.LinearNgramModel.try_load_default", return_value=(None, None)):
            with self.assertRaises(ValueError):
                trainer.build_corpus()


if __name__ == "__main__":
    unittest.main()
