"""The training readiness check names what is missing instead of passing it silently."""

from __future__ import annotations

import hashlib
import io
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))

import training_doctor as doctor  # noqa: E402


class TrainingDoctorTest(unittest.TestCase):
    def test_matching_reports_missing_changed_and_pinned_files(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "en_US.lm"
            self.assertIn("missing", doctor.matching({path: "pin"}).detail)
            path.write_bytes(b"lexicon")
            self.assertFalse(doctor.matching({path: "pin"}).passed)
            self.assertIn("differs", doctor.matching({path: "pin"}).detail)
            self.assertTrue(doctor.matching({path: hashlib.sha256(b"lexicon").hexdigest()}).passed)

    def test_stages_name_every_requirement_they_use(self) -> None:
        checks = {probe.__name__.removeprefix("check_") for probe in doctor.CHECKS}
        for stage in doctor.STAGES:
            self.assertTrue(set(stage.requires) <= checks, stage.name)

    def test_requested_blocked_stage_fails_and_ready_stage_passes(self) -> None:
        def passing(name: str) -> doctor.Check:
            return doctor.Check(name, name != "context_action_corpus", "probe")

        probes = tuple((lambda name=name: passing(name)) for name in (
            "compiler", "system_lexicons", "system_hunspell", "reference_models",
            "prefix_v1_data", "context_action_corpus", "context_action_ledger"))
        output = io.StringIO()
        with patch.object(doctor, "CHECKS", probes), redirect_stdout(output), redirect_stderr(io.StringIO()):
            self.assertEqual(doctor.main(["--stage", "train-prefix-v2"]), 0)
            self.assertEqual(doctor.main(["--stage", "train-context-v3"]), 1)
        self.assertIn("[blocked] train-context-v3", output.getvalue())
        self.assertIn("[ready]   train-prefix-v2", output.getvalue())


if __name__ == "__main__":
    unittest.main()
