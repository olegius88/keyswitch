"""Reproducibility, family split and artifact provenance regression tests."""

from __future__ import annotations

import copy
from contextlib import redirect_stdout
import hashlib
import io
import json
import os
import subprocess
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
import verify_context_action_model as action_verifier
import verify_context_model as verifier
from verify_context_model import LEGACY_FEATURE_VERSION
from keyswitch.constants.models import (
    CONTEXT_ACTION_FEATURE_VERSION,
    CONTEXT_V1_CONVERSION_THRESHOLD,
)
from fixture_values.corpora import HISTORICAL_CONTEXT_V1_TEST_COUNTS
from fixture_values.counts import (
    CONTEXT_ACTION_REPLAY_COMMAND_COUNT,
    CONTEXT_TRAINING_TEST_EPOCHS,
    VERIFY_CALLS_DURING_REPLAY,
)
from fixture_values.models import (
    ARBITRARY_INVALID_CONTEXT_FEATURE_VERSION,
    UNSUPPORTED_CONTEXT_FEATURE_VERSION,
)
from fixture_values.platform import FAILED_PROCESS_RETURN_CODE
from keyswitch.constants.file_formats import (
    METADATA_JSON_LIMIT_BYTES,
    SHA256_HEX_CHARACTERS,
    VERSION_HASH_CHARACTERS,
)
from keyswitch.constants.training import CONTEXT_V1_MINIMUM_TEST_ROWS


def write_fixture_artifact(path: Path, feature_version: int = LEGACY_FEATURE_VERSION) -> ContextModel:
    weights = {"bias": [1.0, 0.0, 0.0, 0.0]}
    fingerprint = hashlib.sha256(json.dumps(weights, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps({"feature_version": feature_version, "actions": list(ACTIONS),
        "weights": weights, "weights_sha256": fingerprint, "conversion_threshold": CONTEXT_V1_CONVERSION_THRESHOLD,
        "version": ("context-v1-" if feature_version == LEGACY_FEATURE_VERSION else "context-v3-")
        + fingerprint[:VERSION_HASH_CHARACTERS]}))
    return ContextModel.load(path)


class ContextTrainingTests(unittest.TestCase):
    def test_family_split_keeps_variants_and_reserves_unseen_test_tokens(self) -> None:
        development = trainer.build_corpus()
        held_out = trainer.build_corpus(trainer.HOLDOUT, held_out=True)
        seen: dict[str, str] = {}
        for row in development:
            self.assertEqual(seen.setdefault(row.family, row.split), row.split)
            self.assertIn(row.split, {"train", "development"})
        self.assertGreater(len(held_out), CONTEXT_V1_MINIMUM_TEST_ROWS)
        self.assertFalse(set(seen) & {row.family for row in held_out})
        self.assertEqual({row.split for row in held_out}, {"test"})
        self.assertEqual(trainer.family_split("test"), trainer.family_split("test"))

    def test_optimizer_is_reproducible_and_never_trains_on_test_rows(self) -> None:
        rows = [
            trainer.Row(ContextEvidence(action, action, 0, FieldContext("test", "1", action)), action, action, split, "fixture")
            for action in ACTIONS for split in ("train", "development")
        ]
        with patch.object(trainer, "EPOCHS", CONTEXT_TRAINING_TEST_EPOCHS):
            first = trainer.train(rows)
            second = trainer.train(rows + [replace(rows[0], split="test", action="convert")])
        self.assertEqual(first, second)
        self.assertTrue(first[0])
        with self.assertRaises(ValueError):
            trainer.train([])
        model = ContextModel({name: tuple(values) for name, values in first[0].items()}, "test")
        metrics = trainer.evaluate(model, rows, "development")
        self.assertEqual(cast(dict[str, int], metrics["counts"])["rows"], len(ACTIONS))

    def test_historical_report_fixture_is_bound_to_artifact_and_quality_counts(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            artifact = root / "artifact.json"
            model = write_fixture_artifact(artifact)
            for path in verifier.provenance_paths(root, artifact).values():
                if path != artifact:
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_text("authored historical provenance fixture\n")
            original: dict[str, object] = {"schema_version": 1, "quality_gates_passed": True,
                "test_overlap": 0, "model_version": model.version, "evidence_scope": "synthetic fixture",
                **{name: hashlib.sha256(path.read_bytes()).hexdigest()
                   for name, path in verifier.provenance_paths(root, artifact).items()},
                "test": {"counts": HISTORICAL_CONTEXT_V1_TEST_COUNTS}}
            report_path = root / "report.json"
            report_path.write_text(json.dumps(original))
            valid = verifier.verify_legacy(root, report_path, artifact)
            self.assertEqual(valid["model_version"], model.version)
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
                    verifier.verify_legacy(root, report_path, artifact)
            report_path.write_bytes(b" " * (METADATA_JSON_LIMIT_BYTES + 1))
            with self.assertRaisesRegex(ValueError, "oversized"):
                verifier.verify_legacy(root, report_path, artifact)

    def test_training_rejects_invalid_scenarios_and_missing_baseline(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "scenarios.json"
            path.write_text("[]", encoding="utf-8")
            with self.assertRaises(ValueError):
                trainer.build_corpus(path)
        with patch("train_context_model.LinearNgramModel.try_load_default", return_value=(None, None)):
            with self.assertRaises(ValueError):
                trainer.build_corpus()


class ActiveContextGateTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix="keyswitch-active-context-fixture-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.artifact = self.root / "artifact.json"
        self.report = self.root / "old-report.json"
        self.receipt = self.root / "receipt.json"
        self.model = write_fixture_artifact(self.artifact)
        self.identity: dict[str, object] = {"model_version": self.model.version,
            "artifact_sha256": hashlib.sha256(self.artifact.read_bytes()).hexdigest(),
            "evidence_scope": "authored fixture"}

    def verify(self) -> dict[str, object]:
        return verifier.verify(self.root, self.report, self.artifact, self.receipt)

    def test_active_shipping_schema_uses_its_declared_model_generation(self) -> None:
        model = ContextModel.load()
        self.assertIn(model.feature_version, (LEGACY_FEATURE_VERSION, CONTEXT_ACTION_FEATURE_VERSION))
        prefix = "context-v1-" if model.feature_version == LEGACY_FEATURE_VERSION else "context-v3-"
        self.assertTrue(model.version.startswith(prefix))
        # Acceptance is an explicit CLI gate, never inferred from this loader test.

    def test_feature_two_keeps_both_historical_evidence_checks(self) -> None:
        with patch.object(verifier, "verify_legacy", return_value=self.identity) as legacy, \
                patch("verify_context_v2.verify") as research, \
                patch.object(action_verifier, "verify", side_effect=AssertionError("v3 receipt must not be used")):
            result = self.verify()
        legacy.assert_called_once_with(self.root, self.report, self.artifact)
        research.assert_called_once_with(self.root / "model/context_v2", self.artifact)
        self.assertEqual(
            result, {**self.identity, "feature_version": LEGACY_FEATURE_VERSION, "quality_gates_passed": True}
        )
        with patch.object(verifier, "verify_legacy", return_value=self.identity), \
                patch("verify_context_v2.verify", side_effect=ValueError("research seal mutated")):
            with self.assertRaisesRegex(ValueError, "research seal"):
                self.verify()

    def test_feature_three_requires_its_receipt_without_historical_fallback(self) -> None:
        model = write_fixture_artifact(self.artifact, CONTEXT_ACTION_FEATURE_VERSION)
        with patch.object(verifier, "verify_legacy", side_effect=AssertionError("historical fallback forbidden")), \
                patch("verify_context_v2.verify", side_effect=AssertionError("historical replay forbidden")):
            with self.assertRaises(FileNotFoundError):
                self.verify()
            self.receipt.write_text('{"quality_gates_passed":true}')
            with self.assertRaises(ValueError):
                self.verify()
            accepted = {"model_version": model.version, "artifact_sha256": action_verifier.checksum(self.artifact),
                        "scope": "receipt fixture delegated to its separately tested validator"}
            with patch.object(action_verifier, "verify", return_value=accepted) as validate:
                result = self.verify()
            validate.assert_called_once_with(root=self.root, receipt_path=self.receipt, artifact=self.artifact)
            self.assertEqual(result["feature_version"], CONTEXT_ACTION_FEATURE_VERSION)
            self.assertEqual(result["artifact_sha256"], accepted["artifact_sha256"])

    def test_rejected_artifact_and_changes_during_verification_fail_closed(self) -> None:
        with patch.object(action_verifier, "checksum", return_value=verifier.REJECTED_ARTIFACT), \
                patch.object(verifier, "verify_legacy", side_effect=AssertionError("rejected bytes reached report")):
            with self.assertRaisesRegex(ValueError, "rejected context-v2"):
                self.verify()
        with patch.object(
            action_verifier, "checksum",
            side_effect=["a" * SHA256_HEX_CHARACTERS, "b" * SHA256_HEX_CHARACTERS],
        ), \
                patch.object(verifier, "verify_legacy", return_value=self.identity), patch("verify_context_v2.verify"):
            with self.assertRaisesRegex(ValueError, "changed during"):
                self.verify()
        payload = cast(dict[str, object], json.loads(self.artifact.read_bytes()))
        payload["feature_version"] = ARBITRARY_INVALID_CONTEXT_FEATURE_VERSION
        self.artifact.write_text(json.dumps(payload))
        with self.assertRaises(ValueError):
            self.verify()

    def test_actual_rejected_research_bytes_cannot_enter_either_acceptance_branch(self) -> None:
        rejected = verifier.ROOT / "model/context_v2/candidate.json"
        self.assertEqual(hashlib.sha256(rejected.read_bytes()).hexdigest(), verifier.REJECTED_ARTIFACT)
        with patch.object(verifier, "verify_legacy", side_effect=AssertionError("legacy acceptance reached")), \
                patch.object(action_verifier, "verify", side_effect=AssertionError("receipt acceptance reached")):
            with self.assertRaisesRegex(ValueError, "rejected context-v2"):
                verifier.verify(artifact=rejected)

    def test_replay_dispatch_keeps_exact_protocols_and_propagates_failure(self) -> None:
        self.assertEqual(verifier.replay_commands(LEGACY_FEATURE_VERSION), (
            ("tools/train_context_model.py", "--verify"),))
        with self.assertRaises(ValueError):
            verifier.replay_commands(UNSUPPORTED_CONTEXT_FEATURE_VERSION)
        with patch("verify_context_model.subprocess.run") as run:
            verifier.replay(self.root, CONTEXT_ACTION_FEATURE_VERSION)
        self.assertEqual(run.call_count, CONTEXT_ACTION_REPLAY_COMMAND_COUNT)
        for call, target in zip(run.call_args_list, (
                "test_language_intent_regressions.LanguageIntentRegressions",
                "test_input_sequence_matrix.InputSequenceMatrixTests",
                "test_context_policy.ContextEngineTests.test_bundled_trained_model_resolves_user_phrase_and_retains_code",
                "test_default_input_sequences.DefaultInputSequenceTests")):
            self.assertEqual(call.args[0], [sys.executable, "-m", "unittest", "-v", target])
            self.assertIs(call.kwargs["check"], True)
            self.assertEqual(call.kwargs["cwd"], self.root)
            self.assertEqual(call.kwargs["env"]["PYTHONPATH"], os.pathsep.join(str(self.root / name) for name in ("src", "tools", "tests")))
        with patch(
            "verify_context_model.subprocess.run",
            side_effect=subprocess.CalledProcessError(FAILED_PROCESS_RETURN_CODE, ["fixture"]),
        ) as run:
            with self.assertRaises(subprocess.CalledProcessError):
                verifier.replay(self.root, LEGACY_FEATURE_VERSION)
        self.assertEqual(run.call_count, 1)
        with patch("verify_context_model.subprocess.run", side_effect=[
                None, None, None, subprocess.CalledProcessError(1, ["default input regressions"])]) as run:
            with self.assertRaises(subprocess.CalledProcessError):
                verifier.replay(self.root, CONTEXT_ACTION_FEATURE_VERSION)
        self.assertEqual(run.call_count, CONTEXT_ACTION_REPLAY_COMMAND_COUNT)

    def test_cli_rechecks_evidence_after_replay_and_prints_only_verified_identity(self) -> None:
        identity = {**self.identity, "feature_version": CONTEXT_ACTION_FEATURE_VERSION, "quality_gates_passed": True}
        output = io.StringIO()
        with patch.object(verifier, "verify", return_value=identity) as verify, \
                patch.object(verifier, "replay") as replay, redirect_stdout(output):
            self.assertEqual(verifier.main(["--replay"]), 0)
        self.assertEqual(verify.call_count, VERIFY_CALLS_DURING_REPLAY)
        replay.assert_called_once_with(verifier.ROOT, CONTEXT_ACTION_FEATURE_VERSION)
        self.assertEqual(json.loads(output.getvalue()), identity)
        with patch.object(verifier, "verify", side_effect=[identity, {**identity, "artifact_sha256": "mutated"}]), \
                patch.object(verifier, "replay"):
            with self.assertRaisesRegex(ValueError, "changed during replay"):
                verifier.main(["--replay"])


if __name__ == "__main__":
    unittest.main()
