"""Small tamper fixtures for frozen lexical evidence and gate dispatch."""
from __future__ import annotations

import copy
import gzip
import io
import json
import sys
import tempfile
import unittest
from contextlib import ExitStack, redirect_stdout
from pathlib import Path
from typing import cast
from unittest.mock import patch

TOOLS = str(Path(__file__).resolve().parents[1] / "tools")
if TOOLS not in sys.path:
    sys.path.insert(0, TOOLS)
import verify_lexical_compatibility as compatibility
from model_protocol import ACTIVE_SPLITS

# "unused_training_version" is not part of the lexical contract; these fixture
# numbers only have to differ from each other to exercise that indifference.
GENERATION_UNUSED_VERSION = 21
CURRENT_UNUSED_VERSION = 23
FUTURE_UNUSED_VERSION = 24
# Arbitrary fixture weights for a fabricated candidate.json; their values are
# never asserted on, only their presence and checksum.
FIXTURE_CANDIDATE_WEIGHTS = (1, 2, 3)
SHA256_HEX_LENGTH = 64


class LexicalCompatibilityTests(unittest.TestCase):
    def setUp(self) -> None:
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory(prefix="keyswitch-lexical-fixture-")))
        languages: dict[str, object] = {}
        dictionaries: dict[str, object] = {}
        for locale in ("en_US", "ru_RU"):
            path = f"model/intent_v1/sources/{locale}.lm"
            self.write(path, (locale + " lexical fixture").encode())
            languages[locale] = {"path": path, "sha256": compatibility.checksum(self.root / path)}
            spelling: dict[str, str] = {}
            for extension, field in (("dic", "dictionary_sha256"), ("aff", "affix_sha256")):
                relative = f"model/intent_v1/sources/hunspell/{locale}.{extension}"
                self.write(relative, (locale + extension).encode())
                spelling[field] = compatibility.checksum(self.root / relative)
            dictionaries[locale] = spelling
        self.generation: dict[str, object] = {"sources": {"languages": languages},
            "external_evaluation": {"hunspell": dictionaries}, "unused_training_version": GENERATION_UNUSED_VERSION}
        self.current = {**self.generation, "unused_training_version": CURRENT_UNUSED_VERSION}
        self.write_json(compatibility.GENERATION_CONFIG, self.generation)
        self.write_json(compatibility.CONFIG, self.current)
        self.stack.enter_context(patch.object(compatibility, "GENERATION_SHA256", compatibility.checksum(self.root / compatibility.GENERATION_CONFIG)))
        self.stack.enter_context(patch.object(compatibility, "AUDITED_CONFIG_SHA256", compatibility.checksum(self.root / compatibility.CONFIG)))
        self.stack.enter_context(patch.object(compatibility, "LEXICAL_SHA256", compatibility.contract_sha256(self.generation)))
        self.write("tools/context_optimizer.c", b"frozen optimizer")
        self.write("tools/generator.py", b"frozen generator")
        anchors: dict[str, dict[str, str]] = {}
        for kind in compatibility.ANCHORS:
            self.write(compatibility.TRAINERS[kind], ("frozen trainer " + kind).encode())
            partitions: dict[str, str] = {}
            for split in ACTIVE_SPLITS:
                relative = f"model/{kind}/{split}.jsonl.gz"
                self.write(relative, gzip.compress((split + " frozen bytes").encode(), mtime=0))
                partitions[split] = compatibility.checksum(self.root / relative)
            self.write_json(f"model/{kind}/corpus.json", {"sha256": partitions, "provenance": {
                compatibility.CONFIG: compatibility.GENERATION_SHA256,
                "tools/generator.py": compatibility.checksum(self.root / "tools/generator.py")}})
            self.write_json(f"model/{kind}/candidate.json", {"weights": list(FIXTURE_CANDIDATE_WEIGHTS)})
            self.write_json(f"model/{kind}/report.json", {"accepted": True, "scope": "fixture"})
            self.write_json(f"model/{kind}/seal.json", {"provenance": {
                "corpus": compatibility.checksum(self.root / f"model/{kind}/corpus.json"),
                "trainer": compatibility.checksum(self.root / compatibility.TRAINERS[kind]),
                "optimizer": compatibility.checksum(self.root / "tools/context_optimizer.c")}})
            anchors[kind] = {name: compatibility.checksum(self.root / f"model/{kind}/{name}")
                            for name in ("corpus.json", "candidate.json", "seal.json", "report.json")}
        self.stack.enter_context(patch.object(compatibility, "ANCHORS", anchors))
        for kind in anchors:
            self.write_json(f"model/{kind}/lexical-compatibility.json", compatibility.expected_receipt(kind, self.generation))

    def write(self, relative: str, raw: bytes) -> None:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(raw)

    def write_json(self, relative: str, value: object) -> None:
        self.write(relative, (json.dumps(value, sort_keys=True) + "\n").encode())

    def test_unchanged_lexical_subset_with_changed_unused_config_is_compatible(self) -> None:
        self.assertNotEqual(compatibility.GENERATION_SHA256, compatibility.AUDITED_CONFIG_SHA256)
        for kind in compatibility.ANCHORS:
            result = compatibility.verify(kind, root=self.root)
            self.assertTrue(result["compatible"])
            self.assertEqual(result["verified_partitions"], list(ACTIVE_SPLITS))
            self.assertNotIn("accepted", result)

    def test_actual_frozen_partitions_sources_trainers_and_anchors_are_required(self) -> None:
        files = [compatibility.CONFIG, compatibility.GENERATION_CONFIG, "tools/generator.py",
                 "tools/context_optimizer.c", *compatibility.lexical_files(self.generation)]
        for kind in compatibility.ANCHORS:
            files.extend([compatibility.TRAINERS[kind], *(f"model/{kind}/{name}" for name in compatibility.ANCHORS[kind]),
                          *(f"model/{kind}/{split}.jsonl.gz" for split in ACTIVE_SPLITS)])
        for relative in files:
            with self.subTest(path=relative):
                path = self.root / relative
                original = path.read_bytes()
                kind = "boundary_v2" if "boundary_v2" in relative else "prefix_v1"
                try:
                    path.write_bytes(original + b"changed")
                    with self.assertRaises(ValueError):
                        compatibility.verify(kind, root=self.root)
                    path.unlink()
                    with self.assertRaises(FileNotFoundError):
                        compatibility.verify(kind, root=self.root)
                finally:
                    path.write_bytes(original)

    def test_receipt_cannot_reanchor_a_replaced_corpus_and_its_partition_hashes(self) -> None:
        kind = "prefix_v1"
        corpus_path = self.root / f"model/{kind}/corpus.json"
        corpus = compatibility.read_object(corpus_path)
        split_path = self.root / f"model/{kind}/test.jsonl.gz"
        split_path.write_bytes(b"replacement")
        cast(dict[str, str], corpus["sha256"])["test"] = compatibility.checksum(split_path)
        self.write_json(f"model/{kind}/corpus.json", corpus)
        receipt = compatibility.expected_receipt(kind, self.generation)
        cast(dict[str, str], receipt["frozen_artifacts"])[f"model/{kind}/corpus.json"] = compatibility.checksum(corpus_path)
        self.write_json(f"model/{kind}/lexical-compatibility.json", receipt)
        with self.assertRaisesRegex(ValueError, "unapproved"):
            compatibility.verify(kind, root=self.root)

    def test_receipt_schema_is_closed_and_duplicate_fields_are_rejected(self) -> None:
        good = compatibility.expected_receipt("prefix_v1", self.generation)
        relative = "model/prefix_v1/lexical-compatibility.json"
        for changed in ({**good, "accepted": True}, {**good, "schema_version": True},
                        {**good, "scope": "fresh evaluation passed"}, {**good, "consumed_contract_sha256": "0" * SHA256_HEX_LENGTH}, {}):
            with self.subTest(changed=changed.get("scope")):
                self.write_json(relative, changed)
                with self.assertRaises(ValueError):
                    compatibility.verify("prefix_v1", root=self.root)
        self.write(relative, b'{"schema_version":1,"schema_version":1}')
        with self.assertRaisesRegex(ValueError, "duplicate"):
            compatibility.verify("prefix_v1", root=self.root)
        with self.assertRaisesRegex(ValueError, "unsupported"):
            compatibility.verify("arbitrary", root=self.root)

    def test_changed_consumed_contract_is_rejected_even_with_reviewed_outer_digest(self) -> None:
        changed = copy.deepcopy(self.current)
        cast(dict[str, object], cast(dict[str, object], changed["sources"])["languages"])["extra_locale"] = {}
        self.write_json(compatibility.CONFIG, changed)
        with patch.object(compatibility, "AUDITED_CONFIG_SHA256", compatibility.checksum(self.root / compatibility.CONFIG)):
            with self.assertRaisesRegex(ValueError, "consumed lexical contract"):
                compatibility.verify("prefix_v1", root=self.root)

    def test_future_unused_config_change_still_requires_a_reviewed_transition(self) -> None:
        self.write_json(compatibility.CONFIG, {**self.current, "unused_training_version": FUTURE_UNUSED_VERSION})
        with self.assertRaisesRegex(ValueError, "input changed"):
            compatibility.verify("prefix_v1", root=self.root)

    def test_external_symlink_is_not_a_valid_frozen_input(self) -> None:
        outside = self.root.parent / (self.root.name + "-external")
        self.addCleanup(outside.unlink, missing_ok=True)
        original = self.root / "tools/generator.py"
        outside.write_bytes(original.read_bytes())
        original.unlink()
        try:
            original.symlink_to(outside)
        except OSError as error:
            self.skipTest(str(error))
        with self.assertRaisesRegex(ValueError, "escapes"):
            compatibility.verify("prefix_v1", root=self.root)


class FrozenGateDispatchTests(unittest.TestCase):
    # Fixture-only gate values for the promotion config and the candidate
    # below; not sourced from any application default.
    MINIMUM_DECIDED_FRACTION_PER_STRATUM = 0.8
    FIXTURE_THRESHOLD = 0.95
    FIXTURE_DECISIVE_WEIGHT = 10.0

    def test_boundary_wrapper_reproduces_original_numeric_evaluation_on_small_fixture(self) -> None:
        import train_boundary_v2 as trainer
        import verify_boundary_v2 as gate
        from keyswitch.boundary_policy import FEATURE_VERSION, BoundaryPolicy
        with ExitStack() as stack:
            directory = Path(stack.enter_context(tempfile.TemporaryDirectory()))
            config: dict[str, object] = {"promotion": {"maximum_errors": 0, "required_recall_strata": ["word_ending"],
                "minimum_decided_fraction_per_stratum": self.MINIMUM_DECIDED_FRACTION_PER_STRATUM,
                "minimum_test_rows": 1, "minimum_test_word_endings": 1}}
            candidate, seal, cfg, receipt = (directory / name for name in ("candidate.json", "seal.json", "config.json", "corpus.json"))
            candidate.write_text(json.dumps({"feature_version": FEATURE_VERSION, "version": "boundary-v2-fixture",
                                            "threshold": self.FIXTURE_THRESHOLD,
                                            "weights": {"decisive": self.FIXTURE_DECISIVE_WEIGHT}}))
            cfg.write_text(json.dumps(config))
            data: list[dict[str, object]] = [{"features": [{"decisive": 0.0}, {"decisive": 1.0}],
                                            "labels": [1], "category": "word_ending"}]
            calibration = trainer.metrics(BoundaryPolicy.load(candidate), data)
            seal.write_text(json.dumps({"candidate_sha256": compatibility.checksum(candidate), "provenance": {},
                                        "config": config, "calibration": calibration}))
            receipt.write_text('{"provenance":{}}')
            for module in (trainer, gate):
                for name, value in (("CONFIG", cfg), ("CANDIDATE", candidate), ("SEAL", seal), ("RECEIPT", receipt)):
                    stack.enter_context(patch.object(module, name, value))
                stack.enter_context(patch.object(module, "provenance", return_value={}))
                stack.enter_context(patch.object(module, "rows", side_effect=lambda split: iter(data)))
            stack.enter_context(patch.object(trainer, "corpus_provenance", return_value={}))
            stack.enter_context(patch.object(gate, "verify_compatibility", return_value={"compatible": True}))
            original = trainer.evaluate()
            self.assertEqual(gate.evaluate(), original)
            self.assertTrue(json.loads(original)["accepted"])
            self.assertEqual(json.loads(original)["test"]["counts"], {"correct": 1, "rows": 1})
            # An incorrect frozen label changes the score and fails the original quality gate.
            data[0]["labels"] = [0]
            changed = gate.evaluate()
            self.assertNotEqual(changed, original)
            self.assertFalse(json.loads(changed)["accepted"])

    def test_prefix_frozen_numeric_replay_requires_compatibility_and_exact_report(self) -> None:
        import verify_prefix_model as gate
        with tempfile.TemporaryDirectory() as temporary:
            report = Path(temporary) / "report.json"
            report.write_bytes(b'{"accepted":true}\n')
            with patch.object(gate, "REPORT", report), patch.object(gate, "verify_compatibility", return_value={"compatible": True}) as validate, \
                    patch.object(gate, "evaluate", return_value=report.read_bytes()) as evaluate:
                self.assertTrue(gate.verify_frozen()["frozen_numeric_regression"])
                validate.assert_called_once_with("prefix_v1")
                evaluate.assert_called_once_with()
                report.write_bytes(b'{"accepted":false}\n')
                with self.assertRaisesRegex(ValueError, "numeric regression"):
                    gate.verify_frozen()
            with patch.object(gate, "verify_compatibility", side_effect=ValueError("unapproved")), patch.object(gate, "evaluate") as evaluate:
                with self.assertRaisesRegex(ValueError, "unapproved"):
                    gate.verify_frozen()
                evaluate.assert_not_called()

    def test_boundary_compatibility_failure_never_opens_frozen_feature_rows(self) -> None:
        import verify_boundary_v2 as gate
        with patch.object(gate, "verify_compatibility", side_effect=ValueError("unapproved")), patch.object(gate, "rows") as rows:
            with self.assertRaisesRegex(ValueError, "unapproved"):
                gate.evaluate()
            rows.assert_not_called()

    def test_ci_preserves_engine_and_rejected_candidate_gates_without_claiming_a_fit(self) -> None:
        root = Path(__file__).resolve().parents[1]
        for workflow in ("tests.yml", "release.yml"):
            text = (root / ".github/workflows" / workflow).read_text()
            for command in ("verify_lexical_compatibility.py prefix_v1", "verify_lexical_compatibility.py boundary_v2",
                            "verify_prefix_model.py --verify-frozen", "verify_boundary_v2.py --verify-frozen",
                            "evaluate_prefix_engine.py --verify", "evaluate_boundary_engine.py --verify",
                            "train_boundary_model.py --verify", "verify_boundary_model.py"):
                self.assertIn(command, text)
            self.assertNotIn("train_boundary_v2.py --verify", text)
            self.assertNotIn("train_prefix_model.py verify", text)

    def test_engine_evaluators_refuse_inputs_that_change_during_replay(self) -> None:
        import evaluate_boundary_engine as boundary
        import evaluate_prefix_engine as prefix
        from keyswitch.boundary_model import BoundaryModel
        from keyswitch.boundary_policy import BoundaryPolicy
        from keyswitch.prefix_model import PrefixModel
        before, after = {"runtime.py": "old"}, {"runtime.py": "new"}
        with patch.object(boundary, "verify_compatibility"), patch.object(boundary, "reference_models", return_value={}), \
                patch.object(boundary, "provenance", side_effect=[before, after]), \
                patch.object(boundary, "SCENARIOS", (("input", "output"),)), \
                patch.object(boundary, "replay", return_value=("output ", 0)), \
                patch.object(BoundaryModel, "load", return_value=None), patch.object(BoundaryPolicy, "load", return_value=None):
            with self.assertRaisesRegex(ValueError, "runtime inputs changed"):
                boundary.evaluate()
        row = {"sequence": 1, "text": "input", "category": "wrong", "desired": True}
        with patch.object(prefix, "verify_compatibility"), patch.object(prefix, "reference_models", return_value={}), \
                patch.object(prefix, "provenance", side_effect=[before, after]), \
                patch.object(prefix, "select", return_value=[row]), \
                patch.object(prefix, "replay", return_value={"expected": "output ", "actual": "output ", "early_at": 1, "injections": 1}), \
                patch.object(PrefixModel, "load", return_value=None), redirect_stdout(io.StringIO()):
            with self.assertRaisesRegex(ValueError, "runtime inputs changed"):
                prefix.evaluate("development")


if __name__ == "__main__":
    unittest.main()
