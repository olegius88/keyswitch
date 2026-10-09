"""The parallel test runner: which modules it runs, in which order, and when the run fails."""

from __future__ import annotations

import io
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

import run_test_modules as runner

ROOT = Path(__file__).resolve().parents[1]


class RunTestModulesTests(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        self.tests = Path(self.directory.name)
        for name, body in (("test_small.py", "x\n"), ("test_large.py", "x\n" * len("large")),
                           ("test_ui.py", "x\n" * len("ui")), ("helper.py", "x\n")):
            (self.tests / name).write_text(body, encoding="utf-8")

    def tearDown(self) -> None:
        self.directory.cleanup()

    def test_patterns_name_each_module_once_and_one_that_names_none_is_an_error(self) -> None:
        self.assertEqual(runner.modules(["test_s*.py", "test_*.py"], self.tests), ["test_small.py", "test_large.py", "test_ui.py"])
        with self.assertRaisesRegex(ValueError, "no test module matches test_missing"):
            runner.modules(["test_missing*.py"], self.tests)

    def test_the_serial_lane_starts_first_and_the_others_largest_first(self) -> None:
        names = runner.modules(["test_*.py"], self.tests)
        self.assertEqual(runner.lanes(names, [], self.tests), [["test_large.py"], ["test_ui.py"], ["test_small.py"]])
        self.assertEqual(runner.lanes(names, ["test_ui.py"], self.tests), [["test_ui.py"], ["test_large.py"], ["test_small.py"]])
        self.assertEqual(runner.lanes(names, ["test_ui.py", "test_s*.py"], self.tests), [["test_small.py", "test_ui.py"], ["test_large.py"]])

    def test_every_module_runs_and_one_failure_fails_the_run(self) -> None:
        command = [sys.executable, "-c", "import sys; print('ran', sys.argv[1]); sys.exit(sys.argv[1] == 'test_large.py')"]
        output = io.StringIO()
        buffer = io.BytesIO()
        with patch.object(runner, "TESTS", self.tests), redirect_stdout(output), patch.object(sys.stdout, "buffer", buffer, create=True):
            status = runner.main(["--serial", "test_ui.py", "test_*.py", "--", *command])
        self.assertEqual(status, 1)
        printed = output.getvalue()
        for name in ("test_small.py", "test_ui.py"):
            self.assertIn(f"==== {name}: passed", printed)
            self.assertIn(f"ran {name}".encode(), buffer.getvalue())
        self.assertIn("==== test_large.py: FAILED", printed)
        self.assertIn("failed: test_large.py", printed)
        with patch.object(runner, "TESTS", self.tests), redirect_stdout(io.StringIO()), patch.object(sys.stdout, "buffer", io.BytesIO(), create=True):
            self.assertEqual(runner.main(["test_small.py", "--", *command]), 0)

    def test_parts_are_dealt_from_the_largest_down_in_snake_order_and_hold_every_module_once(self) -> None:
        for size, name in enumerate(("test_e.py", "test_d.py", "test_c.py", "test_b.py", "test_a.py")):
            (self.tests / name).write_text("x\n" * (size + len("large")), encoding="utf-8")
        names = runner.modules(["test_*.py"], self.tests)
        ordered = sorted(names, key=lambda name: (-(self.tests / name).stat().st_size, name))
        # From the largest down: the first to part 1, the next two to part 2, the next two to part 1, ...
        owners = "12211221"
        for owner in "12":
            with self.subTest(part=owner):
                self.assertEqual(runner.part(names, int(owner), len(set(owners)), self.tests),
                                 [name for name, dealt in zip(ordered, owners, strict=True) if dealt == owner])
        self.assertEqual(runner.part(names, 1, 1, self.tests), ordered)

    def test_a_part_is_k_of_n_with_k_between_one_and_n(self) -> None:
        self.assertEqual(runner.parse_part("2/3"), (len("ab"), len("abc")))
        for text in ("0/2", "3/2", "2", "a/b", "1/", "/2", "-1/2"):
            with self.subTest(text=text), self.assertRaises(Exception):
                runner.parse_part(text)

    def test_a_run_of_one_part_runs_only_its_modules(self) -> None:
        command = [sys.executable, "-c", "import sys; print('ran', sys.argv[1])"]
        output = io.StringIO()
        with patch.object(runner, "TESTS", self.tests), redirect_stdout(output), patch.object(sys.stdout, "buffer", io.BytesIO(), create=True):
            self.assertEqual(runner.main(["--part", "2/2", "test_*.py", "--", *command]), 0)
        expected = runner.part(runner.modules(["test_*.py"], self.tests), len("ab"), len("ab"), self.tests)
        ran = [line.split()[1].rstrip(":") for line in output.getvalue().splitlines() if line.startswith("==== test_")]
        self.assertEqual(sorted(ran), sorted(expected))

    def test_both_workflows_run_the_suite_in_parts_and_report_coverage_of_their_union(self) -> None:
        script = (ROOT / "tests/run_coverage.sh").read_text(encoding="utf-8")
        self.assertIn('${part:+--part "$part"}', script)
        for workflow in ("tests.yml", "release.yml"):
            text = (ROOT / ".github/workflows" / workflow).read_text(encoding="utf-8")
            with self.subTest(workflow=workflow):
                self.assertIn("        part: [1, 2]", text)
                self.assertIn("KEYSWITCH_TEST_PART: ${{ matrix.part }}/2", text)
                self.assertIn("name: keyswitch-coverage-part-${{ matrix.part }}", text)
                self.assertIn("include-hidden-files: true", text)
                coverage = text[text.index("\n  coverage:\n"):text.index("\n  replays:\n")]
                self.assertIn("needs: [verify]", coverage)
                self.assertIn("pattern: keyswitch-coverage-part-*", coverage)
                self.assertIn("python3 -m coverage combine", coverage)
                self.assertIn("python3 -m coverage report", coverage)
        release = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
        # Nothing is published before the joined report passes.
        for job in ("\n  apt-repository:\n", "\n  release:\n"):
            needs = next(line for line in release[release.index(job):].splitlines() if line.strip().startswith("needs:"))
            self.assertIn("coverage", needs)

    def test_a_run_without_a_command_is_refused(self) -> None:
        for arguments in (["test_*.py"], ["test_*.py", "--"]):
            with self.assertRaises(SystemExit), patch("sys.stderr", io.StringIO()):
                runner.main(arguments)


if __name__ == "__main__":
    unittest.main()
