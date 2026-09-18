"""Authored isolation cases for blind corpus reconciliation, without UD data."""

from __future__ import annotations

from dataclasses import asdict, replace
from contextlib import redirect_stdout
import gzip
import hashlib
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

TOOLS = str(Path(__file__).resolve().parents[1] / "tools")
if TOOLS not in sys.path:
    sys.path.insert(0, TOOLS)

import reconcile_context_action_corpus as repair
from freeze_context_action_corpus import CorpusRow, Sentence, SurfaceToken, canonical, checksum, digest, row_identifier, typo_variants
from model_protocol import ALL_SPLITS


def fixture(original: str, split: str, *, group: int = 0, identifier: str = "1", lemma: str | None = None,
            before: str = "") -> tuple[Sentence, CorpusRow]:
    token = SurfaceToken("1", original, (original if lemma is None else lemma,), "NOUN", "_", "_")
    sentence = Sentence(identifier, "document:" + identifier, "authored", "fixture", original, (token,))
    row = CorpusRow(row_identifier(sentence, token), original, group, before, "", token.lemmas[0],
                    digest("old-root:" + identifier), sentence.document, "en", "authored", "fixture", identifier,
                    "1", "", "", "", "NOUN", "_", "_", "exact-text-comment", True, split)
    return sentence, row


def run(*items: tuple[Sentence, CorpusRow], historical: tuple[str, ...] = ()) -> repair.Reconciled:
    return repair.reconcile_rows([sentence for sentence, _ in items], [row for _, row in items], historical)


class PhysicalClosureTests(unittest.TestCase):
    def test_shift_intervention_finds_a_test_alias_that_is_not_its_old_root(self) -> None:
        train = fixture("Жук", "train", group=1, identifier="train")
        test = fixture(":er", "test", identifier="test")
        self.assertNotEqual(digest(":er"), test[1].family)
        result = run(train, test)
        self.assertEqual(result.summary["test_contaminated_rows"], 1)
        self.assertEqual({row.split for row in result.rows}, {"quarantine"})
        self.assertEqual(result.summary["remaining_test_prior_exposure_overlap"], 0)
        self.assertEqual(result.summary["replacements"], 0)

    def test_lemma_and_declared_typo_are_part_of_transitive_closure(self) -> None:
        source = fixture("Жук", "train", group=1, identifier="source", lemma="Жука")
        test = fixture(":erf", "test", identifier="test")
        self.assertEqual(run(source, test).summary["test_contaminated_rows"], 1)
        source = fixture("abcdef", "train", identifier="typo-source")
        spelling = typo_variants(source[1].original, source[1].identifier)[0]
        calibration = fixture(spelling, "calibration", identifier="calibration")
        self.assertEqual({row.split for row in run(source, calibration).rows}, {"quarantine"})

    def test_prior_context_and_historical_exposure_survive_future_training_drops(self) -> None:
        train = fixture("orange", "train", identifier="train", before="черника ")
        test = fixture("черника", "test", group=1, identifier="test")
        result = run(train, test)
        self.assertEqual(result.rows[0].split, "train")
        self.assertEqual(result.rows[1].split, "quarantine")
        self.assertEqual(result.summary["test_contaminated_rows"], 1)
        result = run(test, historical=("черника",))
        self.assertEqual(result.summary["test_contaminated_rows"], 1)
        self.assertIn("shift-closure-historical-exposure", result.rows[0].quarantine_reasons)

    def test_unrelated_test_bytes_and_membership_are_preserved(self) -> None:
        train = fixture("orange", "train", identifier="train")
        test = fixture("черника", "test", group=1, identifier="test")
        result = run(train, test)
        self.assertEqual(result.summary["test_contaminated_rows"], 0)
        self.assertEqual(canonical(asdict(result.rows[1])), canonical(asdict(test[1])))
        self.assertNotEqual(result.rows[0].family, train[1].family)
        self.assertEqual(repair.test_membership(result.rows, "same"), repair.test_membership([train[1], test[1]], "same"))

    def test_unknown_glyph_quarantine_and_cross_split_exclusion_are_fail_closed(self) -> None:
        unsupported = fixture("a\u00a0b", "train", identifier="unsupported")
        old = fixture("киви", "quarantine", group=1, identifier="old")
        old = old[0], replace(old[1], quarantine_reasons=("old-reason",))
        result = run(unsupported, old)
        self.assertIn("unsupported-exact-physical-glyph", result.rows[0].quarantine_reasons)
        self.assertEqual(result.rows[1], old[1])
        self.assertTrue(repair.physical_aliases("🙂"))
        missing = fixture("orange", "train")
        with self.assertRaisesRegex(ValueError, "inventory"):
            repair.reconcile_rows([], [missing[1]], ())

    def test_historical_literal_inventory_excludes_docstrings_and_does_not_execute_code(self) -> None:
        source = '''
"""UNUSED_DOCUMENTATION"""
contexts: tuple[str, ...] = ("actual before", "другой контекст")
contexts += ("continued context",)
phrases = ("declared phrase",)
for following in ("future word",):
    add(word, group, "explicit before", following, app, role, trigger, action, category)
unknown_family("source", 0, ("family context",))
raise RuntimeError("MUST_NOT_EXECUTE")
'''
        forms = repair.historical_code_forms(source)
        self.assertTrue({"actual", "контекст", "continued", "declared", "future", "explicit", "family"} <= forms)
        self.assertNotIn("UNUSED_DOCUMENTATION", forms)
        self.assertNotIn("MUST_NOT_EXECUTE", forms)

    def test_screenability_counts_do_not_call_models(self) -> None:
        _, supported = fixture("hello", "test", identifier="supported")
        _, unsupported = fixture("🙂", "test", identifier="unsupported")
        with patch("keyswitch.context_model.ContextModel.predict", side_effect=AssertionError("must not score")):
            counts = repair.sequence_document_counts([supported, unsupported])
        self.assertEqual(counts["screenable_documents_by_group"], {"0": 1, "1": 0, "None": 0})
        self.assertEqual(counts["unsupported_selected_by_group"], {"0": 1})


class ReconciliationIOTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.parent = self.root / "original"
        self.parent.mkdir()
        self.items = (fixture("orange", "train", identifier="train"), fixture("черника", "test", group=1, identifier="test"))
        self.rows = [row for _, row in self.items]
        records: dict[str, dict[str, object]] = {}
        for split in ALL_SPLITS:
            raw = b"".join(canonical(asdict(row)) for row in self.rows if row.split == split)
            path = self.parent / (split + ".jsonl.gz")
            path.write_bytes(gzip.compress(raw, mtime=0))
            records[split] = {"path": path.name, "sha256": checksum(path), "content_sha256": hashlib.sha256(raw).hexdigest(),
                              "rows": sum(row.split == split for row in self.rows)}
        (self.parent / "test-membership.json").write_bytes(canonical(repair.test_membership(self.rows, "unchanged")))
        (self.parent / "generator-source.py").write_text("# historical fixture\n")
        self.manifest: dict[str, object] = {"splits": records, "namespace": "unchanged", "generator_sha256": checksum(self.parent / "generator-source.py"),
                                           "test_membership_sha256": checksum(self.parent / "test-membership.json")}
        (self.parent / "manifest.json").write_bytes(canonical(self.manifest))

    def test_validation_checks_original_content_membership_and_compressed_bytes(self) -> None:
        repair.validate_original(self.parent, self.manifest, self.rows)
        with self.assertRaisesRegex(ValueError, "reconstructed original"):
            repair.validate_original(self.parent, self.manifest, [replace(self.rows[0], original="tampered"), self.rows[1]])
        (self.parent / "test-membership.json").write_bytes(b"{}")
        with self.assertRaisesRegex(ValueError, "membership changed"):
            repair.validate_original(self.parent, self.manifest, self.rows)
        (self.parent / "train.jsonl.gz").write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "compressed split changed"):
            repair.validate_original(self.parent, self.manifest, self.rows)

    def test_derivative_preserves_parent_and_unchanged_test_exact_bytes(self) -> None:
        before = {path.name: path.read_bytes() for path in self.parent.iterdir()}
        result = run(*self.items)
        result.summary["parent_manifest_sha256"] = checksum(self.parent / "manifest.json")
        output = self.root / "derivative"
        repair.write_derivative(self.parent, output, result, self.manifest)
        self.assertEqual({path.name: path.read_bytes() for path in self.parent.iterdir()}, before)
        self.assertEqual((output / "test.jsonl.gz").read_bytes(), before["test.jsonl.gz"])
        self.assertEqual((output / "test-membership.json").read_bytes(), before["test-membership.json"])
        current = json.loads((output / "manifest.json").read_bytes())
        self.assertEqual(current["namespace"], "unchanged")
        self.assertEqual(current["parent_manifest_sha256"], checksum(self.parent / "manifest.json"))
        self.assertEqual(current["reconciliation"]["summary"]["replacements"], 0)
        with self.assertRaisesRegex(ValueError, "overwrite"):
            repair.write_derivative(self.parent, output, result, self.manifest)
        with self.assertRaisesRegex(ValueError, "inside original"):
            repair.write_derivative(self.parent, self.parent / "nested", result, self.manifest)

    def test_changed_parent_and_invalid_generator_are_rejected(self) -> None:
        result = run(*self.items)
        result.summary["parent_manifest_sha256"] = "wrong"
        with self.assertRaisesRegex(ValueError, "parent changed"):
            repair.write_derivative(self.parent, self.root / "derivative", result, self.manifest)
        with self.assertRaisesRegex(ValueError, "generator checksum"):
            repair.frozen_api(self.parent, {"generator_sha256": "wrong"})
        with patch("reconcile_context_action_corpus.importlib.util.spec_from_file_location", return_value=None), self.assertRaisesRegex(ValueError, "generator unavailable"):
            repair.frozen_api(self.parent, self.manifest)
        (self.parent / "object.json").write_text("[]")
        with self.assertRaisesRegex(ValueError, "JSON object"):
            repair.read_object(self.parent / "object.json")

    def test_access_denial_is_explicit_and_other_errors_do_not_reveal_source_text(self) -> None:
        arguments = ["--corpus", str(self.parent), "--source-root", str(self.root / "sources"),
                     "--report", str(self.root / "summary.json")]
        output = io.StringIO()
        with patch.object(repair, "audit", side_effect=PermissionError(13, "permission denied", "/fixture/data")), redirect_stdout(output):
            self.assertEqual(repair.main(arguments), 1)
        self.assertEqual(json.loads(output.getvalue())["status"], "access-denied")
        output = io.StringIO()
        with patch.object(repair, "audit", side_effect=ValueError("PRIVATE_SOURCE_EXAMPLE")), redirect_stdout(output):
            self.assertEqual(repair.main(arguments), 1)
        self.assertNotIn("PRIVATE_SOURCE_EXAMPLE", output.getvalue())
        with patch.object(repair, "audit") as audit, redirect_stdout(io.StringIO()):
            self.assertEqual(repair.main([*arguments[:-1], str(self.parent / "test.jsonl.gz")]), 1)
        audit.assert_not_called()
