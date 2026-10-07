"""The promotion tool reads no sealed test before every check of the pinned files holds."""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from collections.abc import Sequence
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

import promote_context_action_pair as promotion


class Recorder:
    """A runner that records each command and fails those whose text holds one of `failing`."""

    def __init__(self, *failing: str) -> None:
        self.failing = failing
        self.commands: list[list[str]] = []

    def __call__(self, command: Sequence[str]) -> int:
        self.commands.append(list(command))
        return 1 if any(part in " ".join(command) for part in self.failing) else 0


class PromotionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        root = Path(self.directory.name)
        self.candidate, self.prefix, self.corpus = root / "ctx", root / "prefix", root / "fit"
        for path in (self.candidate, self.prefix, self.corpus):
            path.mkdir()
        seal = {"provenance": {"tests/test_context_policy.py": "a", "tests/fixture_values/models.py": "b",
                               "tools/train_context_action_model.py": "c", "tests/test_core.py": "d"}}
        (self.candidate / promotion.CONTEXT_SEAL).write_text(json.dumps(seal), encoding="utf-8")
        for name in (promotion.CONTEXT_ARTIFACT,):
            (self.candidate / name).write_text("{}", encoding="utf-8")
        for name in (promotion.PREFIX_ARTIFACT, promotion.PREFIX_SEAL):
            (self.prefix / name).write_text("{}", encoding="utf-8")

    def tearDown(self) -> None:
        self.directory.cleanup()

    def checks(self, runner: Recorder, changed: list[str] | None = None) -> list[str]:
        with patch("evaluate_context_action_sequences.validate_candidate_seal", return_value={}):
            return promotion.preflight(self.candidate, self.prefix, self.corpus, runner, lambda: changed or [])

    def test_the_tests_the_seal_pins_are_run_by_module(self) -> None:
        seal = json.loads((self.candidate / promotion.CONTEXT_SEAL).read_bytes())
        self.assertEqual(promotion.pinned_test_modules(seal), ["test_context_policy", "test_core"])
        runner = Recorder()
        self.assertEqual(self.checks(runner), [])
        self.assertEqual(runner.commands[-1][-len(["test_context_policy", "test_core"]):], ["test_context_policy", "test_core"])
        self.assertTrue(any("tools/typecheck.sh" in command for command in runner.commands))
        self.assertTrue(any(part.endswith("check_named_values.py") for command in runner.commands for part in command))

    def test_every_failing_check_is_named_and_no_test_is_read(self) -> None:
        runner = Recorder("typecheck.sh", "check_named_values.py")
        failures = self.checks(runner, ["tests/test_core.py"])
        self.assertEqual(failures, ["uncommitted tracked files: tests/test_core.py", "strict typing", "named values"])
        self.assertFalse(any("evaluate_context_action_sequences.py" in " ".join(command) for command in runner.commands))
        with patch.object(promotion, "preflight", return_value=failures), patch.object(promotion, "promote") as promote:
            self.assertEqual(promotion.main(["--candidate", str(self.candidate), "--prefix-candidate", str(self.prefix),
                                             "--corpus", str(self.corpus), "--report", str(self.corpus / "r.json")]), 1)
        promote.assert_not_called()

    def test_a_seal_the_corpus_refuses_and_a_missing_prefix_file_fail_the_checks(self) -> None:
        (self.prefix / promotion.PREFIX_SEAL).unlink()
        with patch("evaluate_context_action_sequences.validate_candidate_seal", side_effect=ValueError("corpus mismatch")):
            failures = promotion.preflight(self.candidate, self.prefix, self.corpus, Recorder(), lambda: [])
        self.assertEqual(failures, ["candidate seal: corpus mismatch", f"missing {self.prefix / promotion.PREFIX_SEAL}"])

    def test_a_failed_sealed_test_installs_nothing_and_a_read_report_is_not_read_again(self) -> None:
        report = self.corpus / "sequences-test.json"
        runner = Recorder("evaluate_context_action_sequences.py")
        with patch("shutil.copyfile") as copy:
            self.assertNotEqual(promotion.promote(self.candidate, self.prefix, self.corpus, report, runner), 0)
        copy.assert_not_called()
        report.write_text("{}", encoding="utf-8")
        runner = Recorder()
        with patch("shutil.copyfile") as copy, patch.object(Path, "unlink"):
            self.assertEqual(promotion.promote(self.candidate, self.prefix, self.corpus, report, runner), 0)
        self.assertFalse(any("evaluate_context_action_sequences.py" in " ".join(command) for command in runner.commands))
        self.assertEqual(len(copy.call_args_list), len(promotion.INSTALLED))
        self.assertEqual([command[1] for command in runner.commands],
                         ["tools/verify_context_action_model.py", "tools/verify_context_action_model.py",
                          "tools/evaluate_boundary_engine.py", "tools/evaluate_prefix_engine.py",
                          "tools/verify_context_model.py", "tools/verify_prefix_model.py"])


if __name__ == "__main__":
    unittest.main()
