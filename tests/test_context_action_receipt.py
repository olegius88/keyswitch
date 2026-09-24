"""Public receipt fixtures never load a real corpus or promote real weights."""
from __future__ import annotations

from copy import deepcopy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from typing import cast
import unittest
from unittest.mock import patch

TOOLS = str(Path(__file__).resolve().parents[1] / "tools")
if TOOLS not in sys.path:
    sys.path.insert(0, TOOLS)

import evaluate_context_action_sequences as evaluator
import verify_context_action_model as verifier
from keyswitch.context_model import ContextModel, FEATURE_VERSION as BASELINE_FEATURE_VERSION
from model_protocol import PROFILES

CONTEXT_FEATURE_VERSION = verifier.CONTEXT_FEATURE_VERSION
DEFAULT_RESTORED = 50
FIXTURE_ROWS = 128
IDENTIFIER_COUNT = FIXTURE_ROWS // 2  # also "initially_correct" and "initially_wrong"
HALF_IDENTIFIER_COUNT = IDENTIFIER_COUNT // 2  # rows per layout group
BASELINE_RESTORED = 48
SHA256_HEX_CHARACTERS = 64
VERSION_HASH_CHARACTERS = 12
CALIBRATION_PROFILE_ROWS = 20
CALIBRATION_PROFILE_CONVERT_ROWS = 10
CALIBRATION_PROFILE_CONVERTED_CORRECTLY = 9
SAMPLE_CONVERSION_RECALL = 0.9
CALIBRATION_PROFILE_CORRECT_CLASSES = 19
ARTIFACT_CONVERSION_THRESHOLD = 0.99
SEAL_CALIBRATION_ROWS = 40
SEAL_CALIBRATION_CONVERT_ROWS = 20
SEAL_CALIBRATION_CONVERTED_CORRECTLY = 18
PREFIX_CONVERSION_THRESHOLD = 0.999
ADDED_IDENTIFIER_COUNT = 2
MIXED_CASE_TRIMMED_LEFT = 2
INVALID_ERROR_VALUE = 7
INVALID_GROUP_VALUE = 2
INVALID_ACTUAL_GROUP_VALUE = 5
INVALID_CARET_POSITION = 999
ALTERED_PREFIX_WEIGHT = 2.0
ALTERED_PREFIX_FEATURE_VERSION = 2
ALTERED_PREFIX_CONVERSION_THRESHOLD = 0.995
DUPLICATE_ENTRY_COUNT = 2


def counts(restored: int = DEFAULT_RESTORED, corruptions: int = 0) -> dict[str, int]:
    return {"rows": FIXTURE_ROWS, "initially_correct": IDENTIFIER_COUNT, "initially_wrong": IDENTIFIER_COUNT,
            "preserved_correct": IDENTIFIER_COUNT - corruptions,
            "exactly_restored": restored, "correct_text_corruptions": corruptions, "length_mismatches": 0,
            "injections": restored + corruptions, "execution_errors": 0, "correction_layout_mismatches": 0,
            "final_layout_mismatches": IDENTIFIER_COUNT - restored + corruptions}


class ContextActionReceiptTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix="keyswitch-public-receipt-fixture-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        models = self.root / "src/keyswitch/resources/models"
        models.mkdir(parents=True)
        (models / "layout_intent_v1.ksm").write_text("authored intent bytes")
        # The version module is the one source the receipt does not fingerprint, so the
        # fixture has to carry it in the shape the verifier insists on.
        (self.root / verifier.VERSION_MODULE).write_text('"""Fixture."""\n\n__version__ = "0.0.0"\n',
                                                         encoding="utf-8")
        for name in verifier.required_provenance(self.root):
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            if not path.exists():
                path.write_text("# authored source identity\n")
        self.artifact = models / "context_policy_v1.json"
        self.write_artifact(self.artifact, CONTEXT_FEATURE_VERSION)
        self.write_artifact(self.root / verifier.BASELINE, BASELINE_FEATURE_VERSION)
        self.prefix_artifact = models / "prefix_policy_v1.json"
        self.write_prefix(self.prefix_artifact)
        self.recipe = {"schema_version": 1, "feature_version": CONTEXT_FEATURE_VERSION, "gate_policy": verifier.GATE_POLICY,
                       "profiles": list(PROFILES)}
        (self.root / verifier.RECIPE).write_bytes(verifier.canonical(self.recipe))
        self.hashes = {name: verifier.checksum(self.root / name) for name in verifier.required_provenance(self.root)}
        self.corpus = self.root / ".t/corpus"
        self.corpus.mkdir(parents=True)
        self.secret = "PRIVATE-FIXTURE-MUST-NOT-BE-PUBLISHED"
        self.identifiers = [self.secret + ":" + str(index) for index in range(IDENTIFIER_COUNT)]
        membership = {"row_ids_sha256": sorted(hashlib.sha256(name.encode()).hexdigest() for name in self.identifiers),
                      "document_ids_sha256": sorted(hashlib.sha256(("document:" + str(index)).encode()).hexdigest() for index in range(IDENTIFIER_COUNT)),
                      "family_ids_sha256": [hashlib.sha256(b"authored-family").hexdigest()]}
        (self.corpus / "test-membership.json").write_bytes(verifier.canonical(membership))
        (self.corpus / "manifest.json").write_bytes(verifier.canonical({"private_note": self.secret,
            "test_membership_sha256": verifier.checksum(self.corpus / "test-membership.json")}))
        self.seal_path = self.corpus / "candidate-seal.json"
        self.report_path = self.corpus / "full-report.json"
        self.receipt_path = self.root / "public-receipt.json"
        self.ledger = self.root / ".t/ledger"
        self.ledger.mkdir()
        for module, name, value in (
            (verifier, "ROOT", self.root), (evaluator, "ROOT", self.root),
            (evaluator, "RECIPE", self.root / verifier.RECIPE), (evaluator, "BASELINE", self.root / verifier.BASELINE),
            (evaluator, "REQUIRED_PROVENANCE", frozenset(self.hashes)), (evaluator, "LEDGER_ROOT", self.ledger),
            (evaluator, "PREFIX_BASELINE", self.root / verifier.PREFIX_BASELINE),
        ):
            patcher = patch.object(module, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)
        runtime_patcher = patch.object(evaluator, "runtime_provenance", return_value=self.hashes)
        runtime_patcher.start()
        self.addCleanup(runtime_patcher.stop)
        profile = {"rows": CALIBRATION_PROFILE_ROWS, "convert_rows": CALIBRATION_PROFILE_CONVERT_ROWS,
                   "converted_correctly": CALIBRATION_PROFILE_CONVERTED_CORRECTLY, "false_conversions": 0,
                   "conversion_recall": SAMPLE_CONVERSION_RECALL,
                   "correct_classes": CALIBRATION_PROFILE_CORRECT_CLASSES, "private_note": self.secret}
        self.seal: dict[str, object] = {"schema_version": 1, "stage": "sealed-before-test", "test_accessed": False,
            "artifact_sha256": verifier.checksum(self.artifact), "model_version": ContextModel.load(self.artifact).version,
            "conversion_threshold": ARTIFACT_CONVERSION_THRESHOLD,
            "corpus_manifest_sha256": verifier.checksum(self.corpus / "manifest.json"),
            "provenance": self.hashes, "recipe": self.recipe, "gate_policy": verifier.GATE_POLICY,
            "calibration": {"rows": SEAL_CALIBRATION_ROWS, "convert_rows": SEAL_CALIBRATION_CONVERT_ROWS,
                "converted_correctly": SEAL_CALIBRATION_CONVERTED_CORRECTLY, "false_conversions": 0,
                "conversion_recall": SAMPLE_CONVERSION_RECALL, "by_profile": {name: dict(profile) for name in PROFILES}},
            "private_note": self.secret}
        self.seal_path.write_bytes(verifier.canonical(self.seal))
        self.prefix_seal_path = self.corpus / "prefix-seal.json"
        self.write_prefix_seal()
        identity = evaluator.evaluation_identity(self.artifact, self.seal_path, self.corpus, self.prefix_artifact, self.prefix_seal_path)
        documents = {"0": HALF_IDENTIFIER_COUNT, "1": HALF_IDENTIFIER_COUNT, "None": 0}

        def block() -> dict[str, object]:
            return {"counts": {"candidate": counts(), "baseline": counts(BASELINE_RESTORED, 1)},
                    "gates": evaluator.profile_gates(counts(), counts(BASELINE_RESTORED, 1), documents),
                    "cases": {"candidate": self.cases(), "baseline": self.cases(BASELINE_RESTORED, 1)}}

        self.report: dict[str, object] = {"schema_version": 1, "split": "test", "identity": identity,
            "protocol": evaluator.PROTOCOL, "gate_policy": evaluator.GATE_POLICY, "promotion_passed": True,
            "selection": {"replayed_documents": documents, "selected_rows": IDENTIFIER_COUNT, "trimmed_rows": 0,
                "unsupported": [], "source_ids": self.identifiers},
            "profiles": {name: {**block(), "early_off": block(),
                                "ablation": {"candidate_context_baseline_prefix": {"counts": counts(), "cases": self.cases()}}}
                         for name in PROFILES}, "private_note": self.secret}
        self.key = evaluator.membership_key(identity)
        (self.ledger / (self.key + ".access.json")).write_bytes(verifier.canonical(identity))
        self.save_report()

    def cases(self, restored: int = DEFAULT_RESTORED, corruptions: int = 0) -> list[dict[str, object]]:
        cases: list[dict[str, object]] = []
        for index, identifier in enumerate(self.identifiers):
            group = int(index >= HALF_IDENTIFIER_COUNT)
            expected = "hello " if group == 0 else "привет "
            other = ("р" if group == 0 else "g") + expected[1:]
            for wrong in (False, True):
                exact = index < restored if wrong else index >= corruptions
                actual = expected if exact else other
                changed = wrong and exact or not wrong and not exact
                final = group if exact else 1 - group
                corrections = [{"event": len(expected), "before": other if wrong else expected, "after": actual,
                    "source_group": 1 - group if wrong else group, "target_group": final, "actual_group": final,
                    "caret_before": len(expected), "caret_after": len(actual)}] if changed else []
                cases.append({"identifier": identifier, "document": "document:" + str(index), "group": group,
                    "initially_wrong": wrong, "expected": expected, "actual": actual, "exact": exact,
                    "trimmed_left": 0, "trimmed_right": 0, "length_mismatch": False,
                    "final_group": final, "expected_final_group": group, "final_layout_matches": final == group,
                    "corrections": corrections, "error": None,
                    "intervention": {"event": 1, "expected": expected[0], "observed": other[0],
                                     "source_group": 1 - group, "target_group": group} if wrong else None})
        return cases

    def write_prefix(self, path: Path, weight: float = 1.0) -> None:
        weights = {"bias": [weight, 0.0, 0.0, 0.0]}
        fingerprint = hashlib.sha256(json.dumps(weights, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()
        path.write_bytes(verifier.canonical({"kind": "keyswitch.prefix-policy", "feature_version": 1, "prefix_feature_version": 1,
            "actions": ["keep", "convert", "wait", "suggest"], "version": "prefix-v1-" + fingerprint[:VERSION_HASH_CHARACTERS],
            "conversion_threshold": PREFIX_CONVERSION_THRESHOLD, "weights_sha256": fingerprint, "weights": weights}))

    def write_prefix_seal(self, **overrides: object) -> None:
        payload = json.loads(self.prefix_artifact.read_bytes())
        value: dict[str, object] = {"schema_version": 1, "candidate_sha256": verifier.checksum(self.prefix_artifact),
            "model_version": payload["version"], "threshold": payload["conversion_threshold"], "provenance": {},
            "recipe": {"fixture": True}, "calibration_passed": True, "promotion_accepted": False,
            "independent_test_evaluated": False, "private_note": self.secret}
        value.update(overrides)
        self.prefix_seal_path.write_bytes(verifier.canonical(value))

    def write_artifact(self, path: Path, feature_version: int) -> None:
        weights = {"bias": [1.0, 0.0, 0.0, 0.0]}
        fingerprint = hashlib.sha256(json.dumps(weights, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
        path.write_bytes(verifier.canonical({"actions": ["keep", "convert", "wait", "suggest"],
            "feature_version": feature_version, "weights": weights, "weights_sha256": fingerprint,
            "version": ("context-v3-" if feature_version == CONTEXT_FEATURE_VERSION else "context-v1-")
            + fingerprint[:VERSION_HASH_CHARACTERS],
            "conversion_threshold": ARTIFACT_CONVERSION_THRESHOLD}))

    def save_report(self) -> None:
        raw = verifier.canonical(self.report)
        self.report_path.write_bytes(raw)
        (self.ledger / (self.key + ".outcome.json")).write_bytes(raw)

    def exported(self) -> dict[str, object]:
        with patch.object(evaluator, "load_split", side_effect=AssertionError("test corpus must not be read")):
            return verifier.export_receipt(self.artifact, self.seal_path, self.report_path, self.corpus, self.receipt_path,
                                           prefix_artifact=self.prefix_artifact, prefix_seal_path=self.prefix_seal_path)

    def verify(self, value: dict[str, object]) -> dict[str, object]:
        return verifier.validate_receipt(value, self.artifact, self.root, self.prefix_artifact)

    def test_export_is_whitelisted_and_installed_artifact_verifies(self) -> None:
        receipt = self.exported()
        self.assertEqual(self.verify(receipt), receipt)
        self.assertEqual(verifier.verify(self.root, self.receipt_path, self.artifact, self.prefix_artifact), receipt)
        self.assertEqual(self.exported(), receipt)
        encoded = self.receipt_path.read_text()
        for private in (self.secret, str(self.root), '"cases"', '"source_ids"', '"expected"', '"actual"', '"private_note"', ".t/"):
            self.assertNotIn(private, encoded)
        self.assertIs(cast(dict[str, object], receipt["protocol"])["fresh_fit_reproducibility_verified"], False)
        self.assertEqual(cast(dict[str, object], receipt["protocol"])["version"], evaluator.PROTOCOL["version"])
        self.assertEqual(receipt["prefix_artifact_sha256"], verifier.checksum(self.prefix_artifact))
        self.assertEqual(receipt["prefix_baseline_sha256"], verifier.checksum(self.root / verifier.PREFIX_BASELINE))
        self.assertEqual(receipt["prefix_baseline_path"], verifier.PREFIX_BASELINE)
        profiles = cast(dict[str, dict[str, object]], cast(dict[str, object], receipt["test"])["profiles"])
        self.assertEqual(set(profiles["portable"]), {"counts", "gates", "early_off"})
        self.assertEqual(set(cast(dict[str, object], profiles["portable"]["early_off"])), {"counts", "gates"})
        self.assertEqual(cast(dict[str, object], receipt["protocol"])["source_protocol_sha256"],
                         hashlib.sha256(verifier.canonical(evaluator.PROTOCOL)).hexdigest())

    def test_every_top_level_field_and_every_gate_is_required(self) -> None:
        original = self.exported()
        for field in verifier.FIELDS:
            with self.subTest(field=field):
                changed = deepcopy(original)
                del changed[field]
                with self.assertRaises(ValueError):
                    self.verify(changed)
        for profile in PROFILES:
            profiles = cast(dict[str, dict[str, object]], cast(dict[str, object], original["test"])["profiles"])
            for gate in cast(dict[str, object], profiles[profile]["gates"]):
                with self.subTest(profile=profile, gate=gate):
                    changed = deepcopy(original)
                    changed_profiles = cast(dict[str, dict[str, object]], cast(dict[str, object], changed["test"])["profiles"])
                    del cast(dict[str, object], changed_profiles[profile]["gates"])[gate]
                    with self.assertRaisesRegex(ValueError, "gates"):
                        self.verify(changed)

    def test_claimed_acceptance_cannot_override_bad_counts_in_either_profile(self) -> None:
        original = self.exported()
        for profile in PROFILES:
            for field in ("correct_text_corruptions", "length_mismatches", "execution_errors", "correction_layout_mismatches",
                          "exactly_restored", "rows", "initially_wrong"):
                with self.subTest(profile=profile, field=field):
                    changed = deepcopy(original)
                    profiles = cast(dict[str, dict[str, object]], cast(dict[str, object], changed["test"])["profiles"])
                    counts = cast(dict[str, dict[str, int]], profiles[profile]["counts"])
                    current, baseline = counts["candidate"], counts["baseline"]
                    current[field] = 0 if field == "exactly_restored" else current[field] + 1
                    if field == "correct_text_corruptions":
                        # The gate is relative: breaking more correct text than the pair
                        # already shipped is what it refuses, not breaking any at all.
                        current[field] = baseline[field] + 1
                        current["preserved_correct"] = current["initially_correct"] - current[field]
                    with self.assertRaises(ValueError):
                        self.verify(changed)
        changed = deepcopy(original)
        cast(dict[str, object], cast(dict[str, object], changed["test"])["documents"])["0"] = HALF_IDENTIFIER_COUNT - 1
        with self.assertRaises(ValueError):
            self.verify(changed)

    def test_calibration_profile_and_aggregate_consistency_are_required(self) -> None:
        original = self.exported()
        for target in ("portable", "reference_hunspell", "aggregate"):
            for field, value in (("false_conversions", 1), ("conversion_recall", 1.0), ("rows", -1), ("convert_rows", True)):
                with self.subTest(target=target, field=field):
                    changed = deepcopy(original)
                    aggregate = cast(dict[str, object], changed["calibration"])
                    current = aggregate if target == "aggregate" else cast(dict[str, dict[str, object]], aggregate["by_profile"])[target]
                    current[field] = value
                    with self.assertRaises(ValueError):
                        self.verify(changed)

    def test_artifact_recipe_and_provenance_tampering_fail_closed(self) -> None:
        original = self.exported()
        for name in ("artifact_sha256", "weights_sha256", "baseline_sha256", "recipe_sha256", "model_version",
                     "prefix_artifact_sha256", "prefix_weights_sha256", "prefix_baseline_sha256", "prefix_model_version"):
            with self.subTest(name=name):
                changed = deepcopy(original)
                changed[name] = "0" * SHA256_HEX_CHARACTERS
                with self.assertRaises(ValueError):
                    self.verify(changed)
        for name, value in (("prefix_feature_version", ALTERED_PREFIX_FEATURE_VERSION),
                             ("prefix_conversion_threshold", ALTERED_PREFIX_CONVERSION_THRESHOLD),
                             ("prefix_baseline_path", "model/prefix_v2/other.json")):
            with self.subTest(name=name):
                changed = deepcopy(original)
                changed[name] = value
                with self.assertRaises(ValueError):
                    self.verify(changed)
        for name in self.hashes:
            with self.subTest(omitted=name):
                changed = deepcopy(original)
                del cast(dict[str, object], changed["provenance"])[name]
                with self.assertRaisesRegex(ValueError, "provenance"):
                    self.verify(changed)
        path = self.root / "tools/train_context_action_model.py"
        path.write_text("# modified trainer\n")
        with self.assertRaisesRegex(ValueError, "provenance"):
            self.verify(original)

    def test_private_or_unexpected_fields_and_paths_are_rejected(self) -> None:
        original = self.exported()
        for name in (".t/private.json", "/tmp/private.json", "src/../private.json", "model/.hidden/file.json"):
            with self.subTest(path=name):
                changed = deepcopy(original)
                cast(dict[str, object], changed["provenance"])[name] = "0" * SHA256_HEX_CHARACTERS
                with self.assertRaisesRegex(ValueError, "nonpublic"):
                    self.verify(changed)
        changed = deepcopy(original)
        changed["cases"] = [{"expected": self.secret}]
        with self.assertRaisesRegex(ValueError, "extra fields"):
            self.verify(changed)
        changed = deepcopy(original)
        cast(dict[str, object], changed["protocol"])["native_execution_verified"] = True
        with self.assertRaises(ValueError):
            self.verify(changed)

    def test_recipe_schema_cannot_be_changed_by_updating_only_claimed_hash(self) -> None:
        receipt = self.exported()
        self.recipe["feature_version"] = BASELINE_FEATURE_VERSION
        path = self.root / verifier.RECIPE
        path.write_bytes(verifier.canonical(self.recipe))
        receipt["recipe_sha256"] = verifier.checksum(path)
        cast(dict[str, object], receipt["provenance"])[verifier.RECIPE] = verifier.checksum(path)
        with self.assertRaisesRegex(ValueError, "recipe"):
            self.verify(receipt)

    def test_export_requires_exact_existing_access_and_outcome(self) -> None:
        for suffix in ("access", "outcome"):
            path = self.ledger / (self.key + "." + suffix + ".json")
            old = path.read_bytes()
            path.write_bytes(b"{}\n")
            with self.subTest(suffix=suffix), self.assertRaisesRegex(ValueError, "immutable"):
                self.exported()
            path.write_bytes(old)
            self.assertFalse(self.receipt_path.exists())

    @unittest.skipIf(os.name == "nt", "symlink creation requires separate Windows privileges")
    def test_public_alias_cannot_export_a_private_file_fingerprint(self) -> None:
        receipt = self.exported()
        target = self.corpus / "private.py"
        target.write_text(self.secret)
        alias = self.root / "tools/public.py"
        alias.symlink_to(target)
        cast(dict[str, object], receipt["provenance"])["tools/public.py"] = verifier.checksum(target)
        with self.assertRaisesRegex(ValueError, "nonpublic"):
            self.verify(receipt)

    def test_changed_evaluator_protocol_cannot_reuse_public_protocol_summary(self) -> None:
        changed = {**evaluator.PROTOCOL, "timing": "changed simulation semantics"}
        with patch.object(evaluator, "PROTOCOL", changed), patch.object(evaluator, "load_split") as loader:
            self.report["protocol"] = changed
            self.report["identity"] = evaluator.evaluation_identity(self.artifact, self.seal_path, self.corpus,
                                                                    self.prefix_artifact, self.prefix_seal_path)
            self.save_report()
            with self.assertRaisesRegex(ValueError, "protocol changed"):
                self.exported()
            loader.assert_not_called()
        self.assertFalse(self.receipt_path.exists())

    def test_export_recomputes_gates_even_for_matching_immutable_claim(self) -> None:
        profiles = cast(dict[str, dict[str, object]], self.report["profiles"])
        candidate = cast(dict[str, dict[str, int]], profiles["portable"]["counts"])["candidate"]
        candidate.update(counts(0))
        cast(dict[str, object], profiles["portable"]["cases"])["candidate"] = self.cases(0)
        self.save_report()
        with self.assertRaisesRegex(ValueError, "gates"):
            self.exported()
        self.assertFalse(self.receipt_path.exists())

    def test_full_cases_must_cover_selection_and_belong_to_immutable_membership(self) -> None:
        original = deepcopy(self.report)
        for fault in ("selected_count", "selected_duplicate", "selected_outside", "unsupported_duplicate",
                      "unsupported_shape", "case_rows_shape", "trim_count", "missing_cases", "duplicate_case",
                      "outside_document", "different_inputs", "false_document_count"):
            with self.subTest(fault=fault):
                self.report = deepcopy(original)
                selection = cast(dict[str, object], self.report["selection"])
                identifiers = cast(list[str], selection["source_ids"])
                profiles = cast(dict[str, dict[str, object]], self.report["profiles"])
                cases = cast(dict[str, list[dict[str, object]]], profiles["portable"]["cases"])["candidate"]
                if fault == "selected_count":
                    selection["selected_rows"] = IDENTIFIER_COUNT - 1
                elif fault == "selected_duplicate":
                    identifiers[1] = identifiers[0]
                elif fault == "selected_outside":
                    identifiers[0] = "outside-membership"
                elif fault == "unsupported_duplicate":
                    selection["unsupported"] = [{"identifier": identifiers[0]}] * DUPLICATE_ENTRY_COUNT
                elif fault == "unsupported_shape":
                    selection["unsupported"] = None
                elif fault == "case_rows_shape":
                    cast(dict[str, object], profiles["portable"]["cases"])["candidate"] = {}
                elif fault == "trim_count":
                    selection["trimmed_rows"] = 1
                elif fault == "missing_cases":
                    cases.clear()
                elif fault == "duplicate_case":
                    cases[1] = cases[0]
                elif fault == "outside_document":
                    cases[0]["document"] = "outside-membership"
                elif fault == "different_inputs":
                    cases[0]["trimmed_left"] = 1
                else:
                    selection["replayed_documents"] = {
                        "0": HALF_IDENTIFIER_COUNT + 1, "1": HALF_IDENTIFIER_COUNT - 1, "None": 0
                    }
                self.save_report()
                with self.assertRaisesRegex(ValueError, "case"):
                    self.exported()
                self.assertFalse(self.receipt_path.exists())

    def test_full_report_accepts_mixed_correct_only_and_declared_unsupported_cases(self) -> None:
        identity = cast(dict[str, object], self.report["identity"])
        membership = cast(dict[str, list[str]], identity["test_membership"])
        selection = cast(dict[str, object], self.report["selection"])
        mixed_id, unsupported_id, document = "mixed-authored", "unsupported-authored", "mixed-document"
        cast(list[str], selection["source_ids"]).extend((mixed_id, unsupported_id))
        membership["row_ids_sha256"].extend(hashlib.sha256(name.encode()).hexdigest() for name in (mixed_id, unsupported_id))
        membership["document_ids_sha256"].append(hashlib.sha256(document.encode()).hexdigest())
        selection["selected_rows"], selection["trimmed_rows"] = IDENTIFIER_COUNT + ADDED_IDENTIFIER_COUNT, ADDED_IDENTIFIER_COUNT
        selection["unsupported"] = [{"identifier": unsupported_id, "codepoints": ["U+00EF"]}]
        cast(dict[str, int], selection["replayed_documents"])["None"] = 1
        mixed = {**self.cases()[0], "identifier": mixed_id, "document": document, "group": None,
                 "trimmed_left": MIXED_CASE_TRIMMED_LEFT}
        for profile in cast(dict[str, dict[str, object]], self.report["profiles"]).values():
            for block in (profile, cast(dict[str, object], profile["early_off"])):
                for model, cases in cast(dict[str, list[dict[str, object]]], block["cases"]).items():
                    cases.append(dict(mixed))
                    total = cast(dict[str, dict[str, int]], block["counts"])[model]
                    for field in ("rows", "initially_correct", "preserved_correct"):
                        total[field] += 1
        verifier.validate_full_report(self.report, identity)

    def test_case_claims_cannot_override_text_layout_intervention_or_injection(self) -> None:
        original = self.cases()
        for index, field, value in ((0, "exact", False), (0, "length_mismatch", True),
                (0, "final_layout_matches", False), (0, "error", INVALID_ERROR_VALUE), (0, "group", True),
                (0, "initially_wrong", 0), (0, "final_group", INVALID_GROUP_VALUE), (0, "actual", None),
                (0, "trimmed_left", -1), (0, "intervention", {}), (1, "intervention", None),
                (1, "corrections", []), (1, "corrections", "claimed")):
            with self.subTest(field=field, value=value):
                case = deepcopy(original[index])
                case[field] = value
                with self.assertRaisesRegex(ValueError, "case"):
                    verifier.case_evidence(case)
        for field, value in (("event", 0), ("expected", "z"), ("observed", "h"), ("source_group", 0)):
            case = deepcopy(original[1])
            cast(dict[str, object], case["intervention"])[field] = value
            with self.subTest(intervention=field), self.assertRaisesRegex(ValueError, "case"):
                verifier.case_evidence(case)
        for field, value in (("actual_group", INVALID_ACTUAL_GROUP_VALUE), ("event", 0),
                             ("caret_after", INVALID_CARET_POSITION), ("before", None)):
            case = deepcopy(original[1])
            cast(list[dict[str, object]], case["corrections"])[0][field] = value
            with self.subTest(correction=field), self.assertRaisesRegex(ValueError, "case"):
                verifier.case_evidence(case)

    def test_correct_and_wrong_cases_must_share_the_same_intended_window(self) -> None:
        profiles = cast(dict[str, dict[str, object]], self.report["profiles"])
        for profile in profiles.values():
            for block in (profile, cast(dict[str, object], profile["early_off"])):
                for cases in cast(dict[str, list[dict[str, object]]], block["cases"]).values():
                    cases[1]["expected"] = str(cases[1]["expected"]) + "!"
                    cases[1]["actual"] = str(cases[1]["actual"]) + "!"
        self.save_report()
        with self.assertRaisesRegex(ValueError, "paired"):
            self.exported()

    def test_early_off_control_is_required_gated_and_bound_to_the_same_cases(self) -> None:
        receipt = self.exported()
        profiles = cast(dict[str, dict[str, object]], cast(dict[str, object], receipt["test"])["profiles"])
        changed = deepcopy(receipt)
        del cast(dict[str, dict[str, object]], cast(dict[str, object], changed["test"])["profiles"])["portable"]["early_off"]
        with self.assertRaisesRegex(ValueError, "missing or extra fields"):
            self.verify(changed)
        changed = deepcopy(receipt)
        control = cast(dict[str, dict[str, object]], cast(dict[str, dict[str, object]], cast(dict[str, object], changed["test"])["profiles"])["portable"]["early_off"])
        cast(dict[str, dict[str, int]], control["counts"])["candidate"]["exactly_restored"] = 0
        with self.assertRaisesRegex(ValueError, "gates"):
            self.verify(changed)
        self.assertEqual(profiles["portable"]["early_off"], {
            "counts": {"candidate": counts(), "baseline": counts(BASELINE_RESTORED, 1)},
            "gates": evaluator.profile_gates(
                counts(), counts(BASELINE_RESTORED, 1),
                {"0": HALF_IDENTIFIER_COUNT, "1": HALF_IDENTIFIER_COUNT, "None": 0},
            ),
        })
        report_profiles = cast(dict[str, dict[str, object]], self.report["profiles"])
        control_cases = cast(dict[str, list[dict[str, object]]], cast(dict[str, object], report_profiles["portable"]["early_off"])["cases"])
        control_cases["candidate"][0]["actual"] = "jello "
        control_cases["candidate"][0]["exact"] = False
        self.save_report()
        with self.assertRaisesRegex(ValueError, "case"):
            self.exported()

    def test_early_off_control_must_replay_exactly_the_same_cases(self) -> None:
        profiles = cast(dict[str, dict[str, object]], self.report["profiles"])
        control_cases = cast(dict[str, list[dict[str, object]]], cast(dict[str, object], profiles["portable"]["early_off"])["cases"])
        for cases in control_cases.values():
            cases[0]["trimmed_left"] = 1
        self.save_report()
        with self.assertRaisesRegex(ValueError, "different case inputs"):
            self.exported()
        self.assertFalse(self.receipt_path.exists())

    def test_installed_prefix_and_prefix_seal_are_bound_to_the_receipt(self) -> None:
        receipt = self.exported()
        self.write_prefix(self.prefix_artifact, weight=ALTERED_PREFIX_WEIGHT)
        with self.assertRaisesRegex(ValueError, "prefix"):
            self.verify(receipt)
        self.write_prefix(self.prefix_artifact)
        self.assertEqual(self.verify(receipt), receipt)
        self.write_prefix_seal(calibration_passed=False)
        with self.assertRaisesRegex(ValueError, "calibration"):
            self.exported()
        self.write_prefix_seal()
        changed = deepcopy(receipt)
        cast(dict[str, object], changed["provenance"])[verifier.PREFIX_BASELINE] = "0" * SHA256_HEX_CHARACTERS
        with self.assertRaisesRegex(ValueError, "provenance"):
            self.verify(changed)
        (self.root / verifier.PREFIX_BASELINE).write_text("# replaced baseline prefix\n")
        with self.assertRaisesRegex(ValueError, "provenance"):
            self.verify(receipt)

    def test_public_counts_cannot_claim_restoration_without_injection(self) -> None:
        receipt = self.exported()
        profiles = cast(dict[str, dict[str, object]], cast(dict[str, object], receipt["test"])["profiles"])
        cast(dict[str, dict[str, int]], profiles["portable"]["counts"])["candidate"]["injections"] = 0
        with self.assertRaisesRegex(ValueError, "inconsistent sequence counts"):
            self.verify(receipt)

    def test_export_rejects_claimed_success_when_immutable_cases_show_damage(self) -> None:
        profiles = cast(dict[str, dict[str, object]], self.report["profiles"])
        cases = cast(dict[str, list[dict[str, object]]], profiles["portable"]["cases"])["candidate"]
        cases[0]["actual"] = "jello "
        cases[0]["exact"] = False
        self.save_report()
        with self.assertRaisesRegex(ValueError, "case"):
            self.exported()
        self.assertFalse(self.receipt_path.exists())

    def test_development_or_changed_identity_cannot_be_exported(self) -> None:
        original = deepcopy(self.report)
        for fault in ("split", "identity", "promotion_passed", "schema_version"):
            self.report = deepcopy(original)
            self.report[fault] = "development" if fault == "split" else {} if fault == "identity" else True if fault == "schema_version" else False
            self.save_report()
            with self.subTest(fault=fault), self.assertRaisesRegex(ValueError, "accepted current"):
                self.exported()

    def test_known_rejected_artifact_is_explicitly_blocked(self) -> None:
        receipt = self.exported()
        receipt["artifact_sha256"] = verifier.REJECTED_ARTIFACT
        with patch.object(verifier, "checksum", return_value=verifier.REJECTED_ARTIFACT):
            with self.assertRaisesRegex(ValueError, "unaccepted"):
                self.verify(receipt)

    def test_the_version_module_is_unpinned_but_may_hold_nothing_else(self) -> None:
        """A release rewrites the version; a model's evidence must not depend on it.

        Pinning the one file the release process rewrites by construction would tie a
        pair to the release number it was sealed under, and no pair could ship in a
        later version without spending a sealed test on a string. It is left out of the
        fingerprints, so its shape is checked instead: a docstring and one string
        assignment, nothing that could hide behaviour where the receipt does not look.
        """

        root = Path(__file__).resolve().parents[1]
        self.assertNotIn(verifier.VERSION_MODULE, verifier.required_provenance(root))
        self.assertTrue(verifier.version_module_holds_only_a_version(root))
        with tempfile.TemporaryDirectory() as temporary:
            other = Path(temporary)
            module = other / verifier.VERSION_MODULE
            module.parent.mkdir(parents=True)
            for body, allowed in (('"""Doc."""\n\n__version__ = "1.2.3"\n', True),
                                  ('"""Doc."""\n\n__version__ = "1.2.3"\nimport os\n', False),
                                  ('__version__ = "1.2.3"\n', False),
                                  ('"""Doc."""\n\nVERSION = "1.2.3"\n', False),
                                  ('"""Doc."""\n\n__version__ = 3\n', False),
                                  ('"""Doc."""\n\n__version__ = version()\n', False),
                                  ('"""Doc."""\n\nx = y = "1.2.3"\n', False),
                                  ('"""Doc."""\n\nthis is not python\n', False)):
                with self.subTest(body=body):
                    module.write_text(body, encoding="utf-8")
                    self.assertIs(verifier.version_module_holds_only_a_version(other), allowed)
            module.unlink()
            self.assertFalse(verifier.version_module_holds_only_a_version(other))

    def test_public_verify_import_does_not_import_editor_or_evaluator(self) -> None:
        root = Path(__file__).resolve().parents[1]
        result = subprocess.run([sys.executable, "-c", "import sys; import verify_context_action_model; "
                                 "assert 'evaluate_context_action_sequences' not in sys.modules; "
                                 "assert 'test_input_integrity' not in sys.modules"],
                                cwd=root, env={**os.environ, "PYTHONPATH": str(root / "src") + os.pathsep + TOOLS},
                                text=True, capture_output=True, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_duplicate_json_fields_are_rejected(self) -> None:
        path = self.root / "duplicate.json"
        path.write_text('{"schema_version":1,"schema_version":1}')
        with self.assertRaisesRegex(ValueError, "duplicate"):
            verifier.read_object(path)


if __name__ == "__main__":
    unittest.main()
