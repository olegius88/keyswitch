"""The context artifact is written one feature to a line, so that a diff of two artifacts names the features that
changed, and the program and the Windows build read it up to the same limit."""

from __future__ import annotations

import hashlib
import json
import re
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

import train_context_action_model as trainer  # noqa: E402
from keyswitch.constants.file_formats import MAX_CONTEXT_ACTION_MODEL_BYTES, VERSION_HASH_CHARACTERS  # noqa: E402
from keyswitch.constants.units import BYTES_PER_MEBIBYTE  # noqa: E402
from keyswitch.constants.models import CONTEXT_ACTION_FEATURE_VERSION, CONTEXT_LINE_FEATURE_VERSION  # noqa: E402
from keyswitch.context_model import ACTIONS, ContextModel  # noqa: E402

PROJECT_ROOT = Path(__file__).resolve().parents[1]


class ArtifactLayoutTests(unittest.TestCase):
    def test_every_feature_is_a_line_of_its_own_and_the_content_is_the_payload(self) -> None:
        payload = {"weights": {"b|x": [1.0, 0.0], "a": [0.0, -1.0]}, "version": "context-v3-test", "actions": list(ACTIONS),
                   "weights_sha256": "test-hash"}
        raw = trainer.artifact_bytes(payload)
        self.assertEqual(json.loads(raw), payload)
        lines = raw.decode().splitlines()
        self.assertEqual(lines[0], "{")
        self.assertEqual(lines[-1], "}")
        self.assertIn('  "a": [0.0,-1.0],', lines)
        self.assertIn('  "b|x": [1.0,0.0]', lines)
        self.assertIn(' "weights": {', lines)
        self.assertLess(lines.index(' "actions": ' + json.dumps(list(ACTIONS), separators=(",", ":")) + ","), lines.index(' "weights": {'))

    def test_an_action_model_may_outgrow_the_limit_of_the_other_schemas(self) -> None:
        weights = {"bias": [0.0] * len(ACTIONS)}
        digest = hashlib.sha256(json.dumps(weights, sort_keys=True, separators=(",", ":")).encode()).hexdigest()

        def payload(feature_version: int, prefix: str) -> dict[str, object]:
            return {"actions": list(ACTIONS), "feature_version": feature_version, "weights": weights, "weights_sha256": digest,
                    "version": prefix + digest[:VERSION_HASH_CHARACTERS], "conversion_threshold": 1.0}

        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "context-action.json"
            path.write_bytes(trainer.artifact_bytes(payload(CONTEXT_ACTION_FEATURE_VERSION, "context-v3-")))
            with patch("keyswitch.context_model.MAX_ARTIFACT_BYTES", 1):
                self.assertEqual(ContextModel.load(path).feature_version, CONTEXT_ACTION_FEATURE_VERSION)
                path.write_bytes(trainer.artifact_bytes(payload(CONTEXT_LINE_FEATURE_VERSION, "context-v1-")))
                with self.assertRaisesRegex(ValueError, "too large"):
                    ContextModel.load(path)
            path.write_bytes(trainer.artifact_bytes(payload(CONTEXT_ACTION_FEATURE_VERSION, "context-v3-")))
            with patch("keyswitch.context_model.MAX_CONTEXT_ACTION_MODEL_BYTES", 1), self.assertRaisesRegex(ValueError, "too large"):
                ContextModel.load(path)

    def test_the_windows_build_reads_the_context_model_up_to_the_program_s_limit(self) -> None:
        script = (PROJECT_ROOT / "packaging/build-windows.ps1").read_text(encoding="utf-8")
        match = re.search(r"-Path \$BundledContextModel -MaximumBytes (\d+)MB", script)
        assert match is not None
        self.assertEqual(int(match.group(1)) * BYTES_PER_MEBIBYTE, MAX_CONTEXT_ACTION_MODEL_BYTES)


if __name__ == "__main__":
    unittest.main()
