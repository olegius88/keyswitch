"""The context-v1 trainer's captured questions: pinned files, reserved families and the kernel fallback."""

from __future__ import annotations

import hashlib
import json
import lzma
import shutil
import sys
import tempfile
import unittest
from array import array
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

import train_context_model as trainer
from context_optimizer import Kernel, Packed
from keyswitch.context_model import ACTIONS, ContextEvidence
from keyswitch.input_context import FieldContext

from fixture_values.counts import CONTEXT_TRAINING_TEST_EPOCHS
from fixture_values.scores import PACKED_FIXTURE_OWN_VALUE, PACKED_FIXTURE_SHARED_VALUE


def captured_line(count: int, original: str, alternative: str, group: int, label: str, before: str = "") -> str:
    values = {"count": count, "original": original, "alternative": alternative, "source_group": group,
              "trigger": "space", "baseline_convert": False, "source_known": False, "target_known": True,
              "score_delta": 0.0, "literal_tail": "", "boundary_text": " ", "after_origin": "none",
              "source_typo": False, "target_typo": False, "source_opening": False, "target_opening": False,
              "inside": False, "before": before, "after": "", "role": "text", "label": label}
    return json.dumps([values[name] for name in trainer.CAPTURED_COLUMNS], ensure_ascii=False) + "\n"


class CapturedQuestionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.data = self.root / "questions.jsonl.xz"
        self.data.write_bytes(lzma.compress((captured_line(1, "ghbdtn", "привет", 0, "convert", "я ")
                                             + captured_line(1, "vs", "мы", 0, "keep", "React ")).encode()))

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def manifest(self, **changes: object) -> Path:
        source: dict[str, object] = {"file": self.data.name, "sha256": hashlib.sha256(self.data.read_bytes()).hexdigest(), "weight": 1.0}
        source.update(changes)
        payload = {"schema_version": 1, "columns": list(trainer.CAPTURED_COLUMNS), "sources": [source]}
        path = self.root / "manifest.json"
        path.write_text(json.dumps(payload), encoding="utf-8")
        return path

    def test_sources_are_read_with_their_weights_and_pinned_bytes(self) -> None:
        self.assertEqual(trainer.captured_sources(self.manifest()), [trainer.CapturedSource(self.data, 1.0, True)])
        self.assertEqual(trainer.captured_sources(self.manifest(balance=False)), [trainer.CapturedSource(self.data, 1.0, False)])
        for changes in ({"sha256": "0" * len(hashlib.sha256().hexdigest())}, {"weight": 0.0}, {"weight": float("nan")}, {"balance": "no"}):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                trainer.captured_sources(self.manifest(**changes))
        path = self.manifest()
        payload = json.loads(path.read_text(encoding="utf-8"))
        payload["columns"] = payload["columns"][1:]
        path.write_text(json.dumps(payload), encoding="utf-8")
        with self.assertRaises(ValueError):
            trainer.captured_sources(path)

    def test_rows_carry_no_application_and_keep_their_counts(self) -> None:
        rows = list(trainer.captured_rows(self.data))
        self.assertEqual([(item.original, ACTIONS[label], count) for item, label, count in rows],
                         [("ghbdtn", "convert", 1), ("vs", "keep", 1)])
        self.assertEqual({item.field.application for item, _label, _count in rows}, {""})
        self.assertEqual(rows[1][0].field.before, "React ")

    def test_a_family_is_its_readings_as_typed_and_without_edge_signs(self) -> None:
        self.assertEqual(trainer.signatures("пссб", "gcc,"), {"gcc,", "gcc"})
        self.assertEqual(trainer.signatures("Швы", "IDS"), {"ids"})

    def rows(self) -> list[trainer.Row]:
        return [trainer.Row(ContextEvidence(action, action, 0, FieldContext("test", "1", action)), action, action, split, "fixture")
                for action in ACTIONS for split in ("train", "development")]

    def test_questions_about_reserved_families_are_not_trained_on(self) -> None:
        sources = [trainer.CapturedSource(self.data, 1.0)]
        with patch.object(trainer, "EPOCHS", CONTEXT_TRAINING_TEST_EPOCHS):
            taught, _epoch, _loss = trainer.train(self.rows(), sources)
            reserved, _epoch, _loss = trainer.train(self.rows(), sources, frozenset({"vs"}))
        self.assertIn("before:word:react", taught)
        self.assertNotIn("before:word:react", reserved)

    def test_an_unbalanced_source_leaves_the_balance_of_actions_alone(self) -> None:
        # Only `keep` rows, not counted in the balance: the scenario rows keep their importance.
        with patch.object(trainer, "EPOCHS", CONTEXT_TRAINING_TEST_EPOCHS):
            counted = trainer.train(self.rows(), [trainer.CapturedSource(self.data, 1.0, True)])
            apart = trainer.train(self.rows(), [trainer.CapturedSource(self.data, 1.0, False)])
        self.assertNotEqual(counted[0], apart[0])

    def test_the_python_fallback_trains_what_the_kernel_trains(self) -> None:
        sources = [trainer.CapturedSource(self.data, 1.0)]
        with patch.object(trainer, "EPOCHS", CONTEXT_TRAINING_TEST_EPOCHS):
            native = trainer.train(self.rows(), sources)
            with patch.object(Kernel, "load", side_effect=RuntimeError("no compiler")):
                python = trainer.train(self.rows(), sources)
        self.assertEqual(native, python)

    @unittest.skipUnless(sys.platform != "win32" and (shutil.which("gcc") or shutil.which("cc")), "training-only Linux C kernel")
    def test_the_python_prediction_matches_the_kernel(self) -> None:
        names = ["a", "b"]
        data = Packed.build([({"a": 1.0}, 0, 1.0), ({"a": PACKED_FIXTURE_SHARED_VALUE, "b": PACKED_FIXTURE_OWN_VALUE}, 1, 1.0)], names)
        weights = array("d", [float(index) / len(ACTIONS) for index in range(len(names) * len(ACTIONS))])
        self.assertEqual(list(trainer._python_predict(data, weights)), list(Kernel.load().predict(data, weights)))


if __name__ == "__main__":
    unittest.main()
