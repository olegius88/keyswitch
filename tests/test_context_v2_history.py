"""Historical anchors and feature-2 compatibility; no prospective corpus access."""
from __future__ import annotations

from contextlib import redirect_stdout
from dataclasses import replace
import io
from pathlib import Path
import sys
import tempfile
from typing import cast
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

from context_evidence import PROFILES, canonical, key
from context_frames import Frame
from keyswitch.context_model import ContextModel
import train_context_v2 as trainer
import verify_context_v2 as verifier
import verify_context_v2_history as history


class HistoricalContextV2Tests(unittest.TestCase):
    def test_reviewed_anchors_and_archived_sources_are_exact_without_current_engine_claim(self) -> None:
        root = history.ROOT
        current_engine = {root / name for name in history.SOURCES if not name.endswith("context_model.py")}
        checksum = history.checksum

        def digest(path: Path) -> str:
            if path in current_engine:
                raise AssertionError("historical evidence must not claim current engine identity")
            return checksum(path)

        with patch.object(history, "checksum", side_effect=digest):
            result = history.verify_anchors(root / "model/context_v2")
        self.assertTrue(result["historical_evidence_verified"])
        self.assertIs(result["current_runtime_verified"], False)
        self.assertIn("no new fit", str(result["scope"]))

    def test_each_historical_artifact_and_source_remains_a_required_pin(self) -> None:
        root = history.ROOT
        targets = [root / "model/context_v2" / name for name in history.ANCHORS]
        targets += [root / history.ARCHIVE / Path(name).name for name in history.SOURCES]
        checksum = history.checksum
        for target in targets:
            with self.subTest(path=target.name), patch.object(history, "checksum", side_effect=lambda path: "0" * 64 if path == target else checksum(path)):
                with self.assertRaisesRegex(ValueError, "historical"):
                    history.verify_anchors(root / "model/context_v2")
        with tempfile.TemporaryDirectory(prefix="keyswitch-historical-pin-") as temporary:
            root = Path(temporary)
            (root / "source.py").write_text("fixed numeric source")
            with self.assertRaisesRegex(ValueError, "source changed"):
                history.verify_sources({"source.py": "0" * 64}, root)
        with patch("verify_context_v2_history.json.loads", return_value=[]):
            with self.assertRaisesRegex(ValueError, "historical evidence object"):
                history.verify_anchors(history.ROOT / "model/context_v2")

    def test_provenance_paths_and_archived_digest_cannot_be_relabeled(self) -> None:
        digest = "a" * 64
        invalid: tuple[object, ...] = ({}, [], {1: digest}, {"tools/file.py": "invalid"}, {"/outside.py": digest},
                     {"../outside.py": digest}, {"C:\\outside.py": digest},
                     {"tools/a.py": digest, "tools\\a.py": digest})
        for pins in invalid:
            with self.subTest(pins=pins), self.assertRaises(ValueError):
                history.normalized_provenance(pins)
        with self.assertRaisesRegex(ValueError, "unapproved"):
            history.verify_sources({"src/keyswitch/context_model.py": digest})

    def test_feature_two_numerics_remain_exact_while_feature_three_branches_are_independent(self) -> None:
        root = history.ROOT
        archived = (root / history.ARCHIVE / "context_model.py").read_text()
        current = (root / "src/keyswitch/context_model.py").read_text()
        history.verify_feature_two(archived, archived)
        history.verify_feature_two(current, archived)
        without_docstring = current.replace('"""Use the same language-support gate during calibration and inference."""', "")
        self.assertNotEqual(without_docstring, current)
        history.verify_feature_two(without_docstring, archived)
        changed_three = current.replace('return ContextPrediction("suggest", 0.0,', 'return ContextPrediction("wait", 0.0,')
        self.assertNotEqual(changed_three, current)
        history.verify_feature_two(changed_three, archived)
        for before, after in (("weight * value", "weight + value"), ("FEATURE_VERSION = 2", "FEATURE_VERSION = 3"),
                ("def softmax(", "def renamed_softmax("), ("math.exp(value - maximum)", "math.exp(value)"),
                ('return any(\n            name in self.weights', 'return all(\n            name in self.weights'),
                ('return any(\n            name in self.weights', 'if False:\n            pass\n        return any(\n            name in self.weights')):
            altered = current.replace(before, after)
            with self.subTest(change=before):
                self.assertNotEqual(altered, current)
                with self.assertRaisesRegex(ValueError, "feature-2"):
                    history.verify_feature_two(altered, archived)

    def test_numeric_wrapper_matches_original_four_track_report_without_fitting_or_writing(self) -> None:
        frames = [Frame(split + action, split + action, split, "eng", action, "слово", 0, "", "", "Telegram",
                        "unknown", "space", action, split + action, "authored")
                  for split in ("test", "lexical_test") for action in ("keep", "convert")]
        cache = {key(row, profile): (False, False, False, 0.0) for row in frames for profile in PROFILES}
        model = ContextModel({"bias": (20.0, 0.0, 0.0, 0.0), "app:telegram": (0.0,) * 4}, "context-v1-fixture")
        seal = {"schema_version": 1, "model_version": model.version, "artifact_sha256": "f" * 64}
        historical = {"historical_evidence_verified": True, "current_runtime_verified": False, "promotion_passed": False}
        with tempfile.TemporaryDirectory(prefix="keyswitch-historical-numeric-") as temporary:
            directory = Path(temporary)
            (directory / trainer.SEAL).write_bytes(canonical(seal))
            with patch.object(trainer, "validate_seal", return_value=seal), \
                    patch.object(ContextModel, "load", return_value=model), redirect_stdout(io.StringIO()):
                original = trainer.evaluate(directory, frames, cache)
            self.assertEqual(set(cast(dict[str, object], original["results"])),
                             {split + ":" + profile for split in ("test", "lexical_test") for profile in PROFILES})
            before = (directory / trainer.REPORT).read_bytes()
            with patch.object(verifier, "verify", return_value=historical), \
                    patch.object(verifier, "all_frames", return_value=frames), \
                    patch.object(verifier, "load_cache", return_value=cache), \
                    patch.object(ContextModel, "load", return_value=model), \
                    patch.object(trainer, "fit", side_effect=AssertionError("historical numeric replay must not fit")):
                result = verifier.verify_frozen(directory)
                self.assertIs(result["frozen_numeric_regression"], True)
                self.assertIs(result["promotion_passed"], False)
                self.assertEqual((directory / trainer.REPORT).read_bytes(), before)
                with patch.object(verifier, "all_frames", return_value=[replace(frames[0], original="changed"), *frames[1:]]):
                    with self.assertRaises((ValueError, KeyError)):
                        verifier.verify_frozen(directory)
                with patch.object(verifier, "metrics", return_value={"counts": {"rows": 2, "desired_conversions": 1,
                        "converted_correctly": 0, "false_conversions": 1, "baseline_false_conversions": 1}, "categories": {}}):
                    with self.assertRaisesRegex(ValueError, "numeric report changed"):
                        verifier.verify_frozen(directory)
                with patch.object(ContextModel, "load", return_value=ContextModel({}, "context-v3-fixture", feature_version=3)):
                    with self.assertRaisesRegex(ValueError, "feature-2"):
                        verifier.verify_frozen(directory)

    def test_feature_two_normalization_and_weight_loading_cannot_change_silently(self) -> None:
        root = history.ROOT
        archived = (root / history.ARCHIVE / "context_model.py").read_text()
        current = (root / "src/keyswitch/context_model.py").read_text()
        for before, after in ((
                'unicodedata.normalize("NFC", text.casefold())',
                'unicodedata.normalize("NFKC", text.lower())'),
                ("self.weights = dict(weights)", "self.weights = {}"),
                ("self.conversion_threshold = conversion_threshold", "self.conversion_threshold = 1.0"),
                ("tuple(float(value) for value in values)", "tuple(float(value) * 0.5 for value in values)"),
                ("payload: object = json.loads(raw)", "payload: object = json.loads(raw[::-1])"),
                ("MAX_FEATURES = 50000", "MAX_FEATURES = 10000"),
                ("MAX_ARTIFACT_BYTES = 8 * 1024 * 1024", "MAX_ARTIFACT_BYTES = 1024")):
            altered = current.replace(before, after)
            with self.subTest(change=before):
                self.assertNotEqual(altered, current)
                with self.assertRaisesRegex(ValueError, "feature-2"):
                    history.verify_feature_two(altered, archived)

    def test_changed_history_is_rejected_before_numeric_inputs_are_read(self) -> None:
        with patch.object(verifier, "verify", side_effect=ValueError("historical anchor changed")), \
                patch.object(verifier, "all_frames") as frames, patch.object(verifier, "load_cache") as cache:
            with self.assertRaisesRegex(ValueError, "historical anchor"):
                verifier.verify_frozen()
            frames.assert_not_called()
            cache.assert_not_called()

    def test_fast_history_gate_cannot_treat_feature_three_as_the_rejected_model(self) -> None:
        with patch.object(ContextModel, "load", return_value=ContextModel({}, "context-v3-fixture", feature_version=3)):
            with self.assertRaisesRegex(ValueError, "candidate identity"):
                verifier.verify()

    def test_numeric_cli_is_explicit_and_fast_cli_does_not_replay(self) -> None:
        with patch.object(verifier, "verify", return_value={"current_runtime_verified": False}) as fast, \
                patch.object(verifier, "verify_frozen", return_value={"frozen_numeric_regression": True}) as numeric, \
                redirect_stdout(io.StringIO()):
            self.assertEqual(verifier.main([]), 0)
            fast.assert_called_once_with()
            numeric.assert_not_called()
            self.assertEqual(verifier.main(["--verify-frozen"]), 0)
            numeric.assert_called_once_with()


if __name__ == "__main__":
    unittest.main()
