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

    def test_a_run_without_a_command_is_refused(self) -> None:
        for arguments in (["test_*.py"], ["test_*.py", "--"]):
            with self.assertRaises(SystemExit), patch("sys.stderr", io.StringIO()):
                runner.main(arguments)


if __name__ == "__main__":
    unittest.main()
