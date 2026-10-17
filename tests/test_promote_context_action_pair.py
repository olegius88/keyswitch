"""The promotion tool reads no sealed test before every check of the pinned files holds."""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from collections.abc import Iterator, Sequence
from contextlib import contextmanager
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

import promote_context_action_pair as promotion


class Recorder:
    """A runner that records each command and the tree it runs in, and fails those whose text holds one
    of `failing` (or that run in a tree of `failing_trees`)."""

    def __init__(self, *failing: str, failing_trees: Sequence[Path] = ()) -> None:
        self.failing = failing
        self.failing_trees = list(failing_trees)
        self.commands: list[list[str]] = []
        self.trees: list[Path] = []

    def __call__(self, command: Sequence[str], cwd: Path = promotion.ROOT) -> int:
        self.commands.append(list(command))
        self.trees.append(cwd)
        failed = any(part in " ".join(command) for part in self.failing) or cwd in self.failing_trees
        return 1 if failed else 0


SCRATCH_TREE = Path("/scratch/candidate-tree")


@contextmanager
def scratch_tree(candidate: Path, prefix_candidate: Path) -> Iterator[Path]:
    yield SCRATCH_TREE


def sealed_report(baseline_restored: int) -> str:
    """A sealed test report whose candidate restores the one row typed in the wrong layout in every profile and mode."""
    sides = {"candidate": {"exactly_restored": 1, "initially_wrong": 1},
             "baseline": {"exactly_restored": baseline_restored, "initially_wrong": 1}}
    return json.dumps({"profiles": {name: {"counts": sides, "early_off": {"counts": sides}} for name in ("portable", "hunspell")}})


def git(root: Path, *arguments: str) -> str:
    return subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@t", *arguments], cwd=root, check=True,
                          capture_output=True, text=True).stdout


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

    def checks(self, runner: Recorder, changed: list[str] | None = None, gain: bool = True,
               shortfall: Sequence[str] = ()) -> list[str]:
        with (patch("evaluate_context_action_sequences.validate_candidate_seal", return_value={}),
              patch("evaluate_context_action_sequences.validate_prefix_seal", return_value={}),
              patch.object(promotion, "development_gain", return_value=gain),
              patch.object(promotion, "calibration_shortfall", return_value=list(shortfall)) as calibration):
            failures = promotion.preflight(self.candidate, self.prefix, self.corpus, runner, lambda: changed or [], scratch_tree)
        # The calibration is compared once every other check holds.
        self.assertEqual(calibration.called, not [failure for failure in failures if "calibration" not in failure])
        return failures

    def test_a_candidate_netting_less_calibration_than_the_installed_pair_reads_no_sealed_test(self) -> None:
        runner = Recorder()
        self.assertEqual(self.checks(runner, shortfall=["hunspell"]), [
            "the calibration nets less than the installed pair's in the hunspell profile "
            "(tools/compare_context_candidates.py): the quality ratchet would refuse the install"])
        self.assertFalse(any("evaluate_context_action_sequences.py" in " ".join(command) for command in runner.commands))
        self.assertEqual(self.checks(Recorder(), gain=False), [
            "no developer-chat development result of the candidate holds both floors (tools/evaluate_devchat_test.py --split dev)"])

    def test_the_calibration_is_compared_with_the_installed_pair_on_the_corpus(self) -> None:
        output = self.candidate / promotion.CALIBRATION_COMPARISON
        commands: list[list[str]] = []

        def comparing(nets: dict[str, tuple[int, int]], status: int = 0) -> promotion.Runner:
            def runner(command: Sequence[str], cwd: Path = promotion.ROOT) -> int:
                commands.append(list(command))
                report = {profile: {name: {"by_class": {"all": {"net": net}}} for name, net in zip(("installed", "candidate"), pair)}
                          for profile, pair in nets.items()}
                output.write_text(json.dumps({"calibration": report, "development": {}}), encoding="utf-8")
                return status
            return runner

        self.assertEqual(promotion.calibration_shortfall(self.candidate, self.corpus, comparing({"portable": (1, 1), "hunspell": (0, 1)})), [])
        command = commands[-1]
        self.assertEqual(command[1], "tools/compare_context_candidates.py")
        self.assertIn(f"installed={promotion.ROOT / promotion.INSTALLED[promotion.CONTEXT_ARTIFACT]}", command)
        self.assertIn(f"candidate={self.candidate / promotion.CONTEXT_ARTIFACT}", command)
        self.assertEqual(command[command.index("--corpus") + 1], str(self.corpus))
        self.assertEqual(promotion.calibration_shortfall(self.candidate, self.corpus, comparing({"portable": (1, 0), "hunspell": (1, 0)})),
                         ["hunspell", "portable"])
        # A comparison that did not run, or left no report, holds in no profile.
        self.assertEqual(promotion.calibration_shortfall(self.candidate, self.corpus, comparing({}, status=1)), ["every"])
        self.assertEqual(promotion.calibration_shortfall(self.candidate, self.corpus, Recorder()), ["every"])
        self.assertFalse(output.exists())

    def test_the_tests_the_seal_pins_are_run_by_module(self) -> None:
        seal = json.loads((self.candidate / promotion.CONTEXT_SEAL).read_bytes())
        self.assertEqual(promotion.pinned_test_modules(seal), ["test_context_policy", "test_core"])
        runner = Recorder()
        self.assertEqual(self.checks(runner), [])
        command = next(command for command in runner.commands if "tools/run_test_modules.py" in command)
        self.assertEqual(command[1:command.index("--")], ["tools/run_test_modules.py", "test_context_policy.py", "test_core.py"])
        # The same modules run in the repository and in a tree with the candidate pair installed.
        self.assertEqual([tree for ran, tree in zip(runner.commands, runner.trees) if ran == command], [promotion.ROOT, SCRATCH_TREE])
        self.assertTrue(any("tools/typecheck.sh" in command for command in runner.commands))
        self.assertTrue(any(part.endswith("check_named_values.py") for command in runner.commands for part in command))

    def test_every_failing_check_is_named_and_no_test_is_read(self) -> None:
        runner = Recorder("typecheck.sh", "check_named_values.py")
        failures = self.checks(runner, ["tests/test_core.py"])
        self.assertEqual(failures, ["uncommitted tracked files: tests/test_core.py", "strict typing", "named values"])
        self.assertFalse(any("evaluate_context_action_sequences.py" in " ".join(command) for command in runner.commands))
        # No candidate tree is built while anything else fails.
        self.assertNotIn(SCRATCH_TREE, runner.trees)
        with (patch.object(promotion, "preflight", return_value=failures), patch.object(promotion, "promote") as promote,
              patch.object(sys, "path", list(sys.path))):
            self.assertEqual(promotion.main(["--candidate", str(self.candidate), "--prefix-candidate", str(self.prefix),
                                             "--corpus", str(self.corpus), "--report", str(self.corpus / "r.json")]), 1)
            # The seal is checked against this tree's runtime, whatever PYTHONPATH the tool was started with.
            self.assertEqual(sys.path[0], str(promotion.ROOT / "src"))
        promote.assert_not_called()

    def test_a_seal_the_corpus_refuses_and_a_prefix_seal_this_tree_refuses_fail_the_checks(self) -> None:
        (self.prefix / promotion.PREFIX_SEAL).unlink()
        with (patch("evaluate_context_action_sequences.validate_candidate_seal", side_effect=ValueError("corpus mismatch")),
              patch.object(promotion, "development_gain", return_value=True)):
            failures = promotion.preflight(self.candidate, self.prefix, self.corpus, Recorder(), lambda: [])
        self.assertEqual(failures[0], "candidate seal: corpus mismatch")
        self.assertTrue(failures[1].startswith("prefix seal: "))
        # A prefix seal whose pinned files changed since it was made (the context candidate's features, say).
        with (patch("evaluate_context_action_sequences.validate_candidate_seal", return_value={}),
              patch("evaluate_context_action_sequences.validate_prefix_seal",
                    side_effect=ValueError("prefix provenance changed: src/keyswitch/context_action_features.py")),
              patch.object(promotion, "development_gain", return_value=True)):
            failures = promotion.preflight(self.candidate, self.prefix, self.corpus, Recorder(), lambda: [])
        self.assertEqual(failures[0], "prefix seal: prefix provenance changed: src/keyswitch/context_action_features.py")

    def test_a_candidate_without_a_recorded_development_gain_reads_no_sealed_test(self) -> None:
        runner = Recorder()
        self.assertEqual(self.checks(runner, gain=False), [
            "no developer-chat development result of the candidate holds both floors (tools/evaluate_devchat_test.py --split dev)"])
        self.assertFalse(any("evaluate_context_action_sequences.py" in " ".join(command) for command in runner.commands))

    def test_the_development_gain_is_read_from_the_record_of_the_candidates_artifact(self) -> None:
        import evaluate_devchat_test as devchat
        ledger = Path(self.directory.name) / "devchat-ledger"
        ledger.mkdir()
        artifact = self.candidate / promotion.CONTEXT_ARTIFACT
        record = ledger / f"{devchat.checksum(artifact)}.dev.json"
        with patch.object(devchat, "ledger_root", return_value=ledger):
            self.assertFalse(promotion.development_gain(artifact))
            record.write_text(json.dumps({"floors": {"f1_better_than_baseline": True, "f3_not_worse_than_released": False}}),
                              encoding="utf-8")
            self.assertFalse(promotion.development_gain(artifact))
            record.write_text(json.dumps({"floors": {"f1_better_than_baseline": True, "f3_not_worse_than_released": True}}),
                              encoding="utf-8")
            self.assertTrue(promotion.development_gain(artifact))

    def test_a_failed_sealed_test_installs_nothing_and_a_read_report_is_not_read_again(self) -> None:
        report = self.corpus / "sequences-test.json"
        runner = Recorder("evaluate_context_action_sequences.py")
        with patch("shutil.copyfile") as copy:
            self.assertNotEqual(promotion.promote(self.candidate, self.prefix, self.corpus, report, runner), 0)
        copy.assert_not_called()
        # The developer-chat test is read only after the Tatoeba test passed.
        self.assertFalse(any("evaluate_devchat_test.py" in " ".join(command) for command in runner.commands))
        report.write_text(sealed_report(0), encoding="utf-8")
        runner = Recorder()
        with patch("shutil.copyfile") as copy, patch.object(Path, "unlink"):
            self.assertEqual(promotion.promote(self.candidate, self.prefix, self.corpus, report, runner), 0)
        self.assertFalse(any("evaluate_context_action_sequences.py" in " ".join(command) for command in runner.commands))
        self.assertEqual(len(copy.call_args_list), len(promotion.INSTALLED))
        self.assertEqual([command[1] for command in runner.commands],
                         ["tools/evaluate_devchat_test.py", "tools/verify_context_action_model.py",
                          "tools/verify_context_action_model.py", "tools/evaluate_boundary_engine.py",
                          "tools/evaluate_prefix_engine.py", "tools/verify_context_model.py", "tools/verify_prefix_model.py"])
        devchat = runner.commands[0]
        # Read against the pair still installed, before the install.
        self.assertEqual(devchat[devchat.index("--released") + 1], promotion.INSTALLED[promotion.CONTEXT_ARTIFACT])
        self.assertEqual(devchat[devchat.index("--split") + 1], "test")

    def test_a_candidate_restoring_no_more_than_the_baseline_pair_installs_nothing_and_reads_no_developer_chat_test(self) -> None:
        report = self.corpus / "sequences-test.json"
        report.write_text(sealed_report(1), encoding="utf-8")
        runner = Recorder()
        with patch("shutil.copyfile") as copy, patch("builtins.print"):
            self.assertNotEqual(promotion.promote(self.candidate, self.prefix, self.corpus, report, runner), 0)
        copy.assert_not_called()
        self.assertEqual(runner.commands, [])
        self.assertEqual(promotion.restoration_shortfall(report), ["hunspell early_off: 1 restored against the baseline pair's 1",
                                                                   "portable early_off: 1 restored against the baseline pair's 1"])
        # The share of the rows typed in the wrong layout counts too.
        sides = {"candidate": {"exactly_restored": 0, "initially_wrong": 1}, "baseline": {"exactly_restored": 0, "initially_wrong": 1}}
        report.write_text(json.dumps({"profiles": {"portable": {"counts": sides, "early_off": {"counts": sides}}}}), encoding="utf-8")
        self.assertIn("portable early_off: 0 of 1 restored", promotion.restoration_shortfall(report))

    def test_a_failed_developer_chat_test_installs_nothing(self) -> None:
        report = self.corpus / "sequences-test.json"
        report.write_text(sealed_report(0), encoding="utf-8")
        runner = Recorder("evaluate_devchat_test.py")
        with patch("shutil.copyfile") as copy:
            self.assertNotEqual(promotion.promote(self.candidate, self.prefix, self.corpus, report, runner), 0)
        copy.assert_not_called()
        self.assertEqual(len(runner.commands), 1)

    def test_the_owners_typing_is_replayed_with_the_candidate_pair_installed_before_any_test_is_read(self) -> None:
        runner = Recorder()
        self.assertEqual(self.checks(runner), [])
        gate = runner.commands[-1]
        self.assertEqual(gate[1:], ["tools/field_regression_gate.py", "--candidate-root", str(SCRATCH_TREE)])
        self.assertEqual(runner.trees[-1], promotion.ROOT)
        runner = Recorder("field_regression_gate.py")
        self.assertEqual(self.checks(runner), [
            "the owner's typing: a word the last release got right comes out wrong, or the field regression gate could "
            "not run (tools/field_regression_gate.py)"])
        self.assertFalse(any("evaluate_context_action_sequences.py" in " ".join(command) for command in runner.commands))

    def test_a_test_module_the_installed_candidate_breaks_fails_the_checks(self) -> None:
        runner = Recorder(failing_trees=[SCRATCH_TREE])
        self.assertEqual(self.checks(runner), ["tests the seal pins, with the candidate pair installed"])
        self.assertEqual(runner.trees.count(SCRATCH_TREE), 1)

    def test_the_candidate_tree_is_the_commit_with_the_pair_installed_and_is_removed_after(self) -> None:
        repository = Path(self.directory.name) / "repository"
        repository.mkdir()
        git(repository, "init", "-q")
        for target in promotion.INSTALLED.values():
            (repository / target).parent.mkdir(parents=True, exist_ok=True)
            (repository / target).write_text("installed", encoding="utf-8")
        git(repository, "add", "-A")
        git(repository, "commit", "-q", "-m", "installed pair")
        for source, _ in promotion.installed_files(self.candidate, self.prefix):
            source.write_text(f"candidate {source.name}", encoding="utf-8")
        with promotion.candidate_tree(self.candidate, self.prefix, repository) as tree:
            for source, target in promotion.installed_files(self.candidate, self.prefix):
                self.assertEqual((tree / target).read_text(encoding="utf-8"), f"candidate {source.name}")
            self.assertIn(str(tree.resolve()), git(repository, "worktree", "list"))
        self.assertFalse(tree.exists())
        self.assertNotIn(str(tree.resolve()), git(repository, "worktree", "list"))
        # The repository's own files are untouched.
        self.assertEqual((repository / promotion.INSTALLED[promotion.CONTEXT_ARTIFACT]).read_text(encoding="utf-8"), "installed")

    def test_a_command_runs_with_the_sources_of_its_own_tree(self) -> None:
        probe = "import os, sys; sys.exit(0 if os.environ['PYTHONPATH'] == 'src:tools' and os.getcwd() == sys.argv[1] else 3)"
        tree = Path(self.directory.name).resolve()
        with patch("builtins.print"):
            self.assertEqual(promotion.run([sys.executable, "-c", probe, str(tree)], tree), 0)


if __name__ == "__main__":
    unittest.main()
