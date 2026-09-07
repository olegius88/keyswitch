"""Frozen prefix evidence and fail-closed package gates (no retraining here)."""
from __future__ import annotations

import copy
import json
import os
import subprocess
import sys
import tempfile
import unittest
from fnmatch import fnmatchcase
from pathlib import Path
from typing import cast
from unittest.mock import patch

from keyswitch.layouts import LayoutPair
from keyswitch.prefix_model import ARTIFACT, PrefixModel

TOOLS = str(Path(__file__).resolve().parents[1] / "tools")
if TOOLS not in sys.path:
    sys.path.insert(0, TOOLS)
import prefix_corpus as corpus
import train_prefix_model as trainer
import verify_prefix_model as verifier


class PrefixEvidenceTests(unittest.TestCase):
    def test_verifier_cli_preserves_unicode_report_on_legacy_stdout(self) -> None:
        root = Path(TOOLS).parent
        expected = verifier.verify()
        self.assertFalse(json.dumps(expected, ensure_ascii=False).isascii())
        for encoding in ("cp1252", "ascii", "utf-8"):
            with self.subTest(encoding=encoding):
                completed = subprocess.run(
                    [sys.executable, str(Path(TOOLS) / "verify_prefix_model.py")],
                    cwd=root,
                    env={**os.environ, "PYTHONPATH": str(root / "src"),
                         "PYTHONIOENCODING": encoding},
                    capture_output=True,
                    timeout=30,
                    check=False,
                )
                self.assertEqual(completed.returncode, 0, completed.stderr.decode("utf-8", "replace"))
                self.assertTrue(completed.stdout.isascii())
                self.assertEqual(json.loads(completed.stdout), expected)

    def test_hashed_prefix_text_inputs_pin_checkout_line_endings(self) -> None:
        engine = json.loads((corpus.DIRECTORY / "engine-report.json").read_bytes())
        paths = set(corpus.provenance()) | set(trainer.provenance()) | set(engine["provenance"])
        root = Path(TOOLS).parent
        paths.add(ARTIFACT.relative_to(root).as_posix())
        attributes = (root / ".gitattributes").read_text(encoding="utf-8")
        patterns = [parts[0] for line in attributes.splitlines()
                    if (parts := line.split())[1:] == ["text", "eol=lf"]]
        for path in sorted(paths):
            if Path(path).suffix in {".py", ".json", ".c", ".txt"}:
                self.assertTrue(any(fnmatchcase(path, pattern) for pattern in patterns), path)

    def test_physical_prefix_families_stay_together_and_frozen_test_cannot_be_replaced(self) -> None:
        pair = LayoutPair()
        self.assertEqual(corpus.family("привет", 1, pair), corpus.family("ghbdtn", 0, pair))
        self.assertEqual(corpus.family("ghbdtn_value", 0, pair), corpus.family("ghbdtn", 0, pair))
        namespace = str(corpus.config()["namespace"])
        self.assertEqual({corpus.split_for(str(index), namespace) for index in range(1000)}, set(corpus.SPLITS))
        with self.assertRaisesRegex(ValueError, "overwrite"):
            corpus.freeze()
        with self.assertRaisesRegex(ValueError, "already observed"):
            trainer.main(["fit"])
        with self.assertRaisesRegex(ValueError, "already observed"):
            trainer.main(["test"])
        with self.assertRaisesRegex(ValueError, "invalid prefix split"):
            next(corpus.rows("private"))

    def test_metrics_count_the_first_conversion_per_sequence_not_each_prefix(self) -> None:
        examples = [
            trainer.SequenceResult("ghbdtn", "wrong", True, "portable", [(4, .9999), (5, .99999)], 5),
            trainer.SequenceResult("function", "identifier", False, "portable", [(4, .995)], None),
            trainer.SequenceResult("hello", "correct", False, "portable", [], None),
        ]
        counts = cast(dict[str, int], trainer.metrics(examples, .999)["counts"])
        self.assertEqual((counts["converted"], counts["false"], counts["characters_before_conversion"]), (1, 0, 4))
        counts = cast(dict[str, int], trainer.metrics(examples, .99)["counts"])
        self.assertEqual((counts["false"], counts["technical_false"]), (1, 1))
        self.assertFalse(trainer.accepted({"profiles": {}}))
        report = json.loads(trainer.REPORT.read_bytes())
        self.assertTrue(trainer.accepted(report["test"]))
        for key, value in (("sequences", True), ("desired", 0), ("false", -1),
                           ("converted", 10**9), ("technical_false", 1), ("converted_before_end", 0)):
            modified = copy.deepcopy(report["test"])
            modified["profiles"]["portable"][key] = value
            self.assertFalse(trainer.accepted(modified))

    def test_package_gate_binds_weights_test_counts_engine_and_runtime(self) -> None:
        self.assertTrue(verifier.verify()["accepted"])
        self.assertEqual(PrefixModel.load().version, PrefixModel.load(trainer.CANDIDATE).version)
        report = json.loads(trainer.REPORT.read_bytes())
        engine = json.loads((corpus.DIRECTORY / "engine-report.json").read_bytes())
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "evidence.json"
            modifications: list[dict[str, object]] = [{"accepted": False}, {"test": {}}, {"candidate_sha256": "changed"}]
            for modification in modifications:
                path.write_text(json.dumps({**report, **modification}), encoding="utf-8")
                with self.assertRaises(ValueError):
                    verifier.verify(report_path=path)
            variants = [{**engine, "provenance": {}}, {**engine, "passed": False}, {**engine, "sequence_ids": []}]
            modified = copy.deepcopy(engine)
            modified["results"]["portable/observed"]["candidate"]["length_mismatches"] = 1
            variants.append(modified)
            for modified in variants:
                path.write_text(json.dumps(modified), encoding="utf-8")
                with self.assertRaises(ValueError):
                    verifier.verify(engine_path=path)
            path.write_bytes(b"different weights")
            with self.assertRaises(ValueError):
                verifier.verify(artifact=path)
            with patch.object(verifier, "checksum", return_value="changed"):
                with self.assertRaises(ValueError):
                    verifier.verify()
            self.assertTrue(ARTIFACT.is_file())


if __name__ == "__main__":
    unittest.main()
