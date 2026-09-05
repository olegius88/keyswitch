"""Frozen-source validation and prevention of accidental rejected-model rollout."""
from __future__ import annotations

import copy
import gzip
import hashlib
import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

TOOLS_PATH = str(Path(__file__).resolve().parents[1] / "tools")
if TOOLS_PATH not in sys.path:
    sys.path.insert(0, TOOLS_PATH)

from context_corpus import CORPUS_ROOT, AssignedPhrase, Phrase, Split, load_source, read_phrases
from context_evidence import load_cache
from evaluate_context_engine import select_phrases
from train_context_v2 import provenance
from verify_context_v2 import read_object, validate_metrics, verify


class ContextV2EvidenceTests(unittest.TestCase):
    def test_frozen_public_source_and_lexical_cache_are_readable(self) -> None:
        source = load_source()
        self.assertEqual(len(source), 64537)
        self.assertEqual(sum(row.locale == "rus" for row in source), 23034)
        self.assertEqual(len(load_cache()), 148138)

    def test_tsv_rejects_missing_columns_duplicate_and_invalid_ids(self) -> None:
        for rows in (["1\trus\ttext"], ["0\trus\ttext\ttime"], ["one\trus\ttext\ttime"], ["1\trus\ttext\ttime"] * 2):
            with self.assertRaises(ValueError):
                read_phrases(rows)
        self.assertEqual(read_phrases(["1\tdeu\tHallo Welt\ttime"]), [])
        self.assertEqual(read_phrases(["2\teng\tA word\ttime", "1\trus\tДва слова\ttime"])[0].identifier, 1)

    def test_source_requires_reviewed_hash(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "source.gz"
            path.write_bytes(gzip.compress(b"1\teng\tHello world\ttime\n"))
            with self.assertRaisesRegex(ValueError, "checksum"):
                load_source(path)
            digest = hashlib.sha256(path.read_bytes()).hexdigest()
            with patch("context_corpus.SOURCE_SHA", digest):
                self.assertEqual(load_source(path)[0].text, "Hello world")

    def test_engine_selection_never_reads_training_or_reserve_and_deduplicates_groups(self) -> None:
        splits: tuple[Split, ...] = ("train", "development", "calibration", "test", "reserve")
        source = [AssignedPhrase(Phrase(index, "eng", "A simple sentence.", ""), str(index), split) for index, split in enumerate(splits, 1)]
        selected = select_phrases(source + [source[3]])
        self.assertEqual(selected, [source[3]])
        self.assertEqual(select_phrases(list(reversed(source))), selected)

    def test_shipping_gate_retains_v1_and_rejects_candidate_installation(self) -> None:
        result = verify()
        self.assertTrue(result["active_model_unchanged"])
        self.assertFalse(result["promotion_passed"])
        with self.assertRaisesRegex(ValueError, "must not replace"):
            verify(active=CORPUS_ROOT / "candidate.json")

    def test_windows_path_separators_do_not_change_sealed_identity(self) -> None:
        windows = {relative.replace("/", "\\"): digest for relative, digest in provenance().items()}
        with patch("verify_context_v2.provenance", return_value=windows):
            self.assertTrue(verify()["active_model_unchanged"])

    def test_report_tampering_and_missing_tracks_fail_closed(self) -> None:
        original = read_object(CORPUS_ROOT / "report.json")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for filename in ("candidate.json", "candidate-seal.json", "engine-report.json"):
                shutil.copyfile(CORPUS_ROOT / filename, root / filename)
            variants = [{**original, "reserve_used": True}, {**original, "seal_sha256": "changed"},
                {**original, "promotion_passed": True}, {**original, "results": {}},
                {**original, "promotion_failures": {}}]
            for variant in variants:
                (root / "report.json").write_text(json.dumps(variant), encoding="utf-8")
                with self.assertRaises(ValueError):
                    verify(directory=root)

    def test_metadata_limits_and_numeric_validation(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "report.json"
            for content in ("[]", "{}", " " * (1024 * 1024 + 1)):
                path.write_text(content, encoding="utf-8")
                with self.assertRaises(ValueError):
                    read_object(path)
        valid: dict[str, object] = {"counts": {"rows": 2, "desired_conversions": 1, "converted_correctly": 1, "false_conversions": 0, "baseline_false_conversions": 0}, "categories": {}}
        self.assertEqual(validate_metrics(valid), valid)
        for field, value in (("rows", True), ("rows", -1), ("desired_conversions", 3), ("converted_correctly", 2)):
            bad = copy.deepcopy(valid)
            counts = bad["counts"]
            assert isinstance(counts, dict)
            counts[field] = value
            with self.assertRaises(ValueError):
                validate_metrics(bad)


if __name__ == "__main__":
    unittest.main()
