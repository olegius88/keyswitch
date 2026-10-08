"""The digest CI keeps the strict intent report under moves with every input and only with them."""

from __future__ import annotations

import contextlib
import io
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

import intent_strict_inputs as inputs

PROJECT_ROOT = Path(__file__).resolve().parents[1]
# A constant the history module reads (so its value is an input) and where it is defined.
USED_CONSTANT = "HISTORY_CONFIDENCE_DECIMALS"
USED_CONSTANT_MODULE = "src/keyswitch/constants/file_formats.py"
UNUSED_CONSTANT = "INTENT_STRICT_INPUTS_TEST_UNUSED"


class IntentStrictInputsTests(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        for relative in inputs.INPUT_FILES:
            self.copy(relative)
        shutil.copytree(PROJECT_ROOT / "src/keyswitch/constants", self.root / "src/keyswitch/constants")
        model = self.root / inputs.INPUT_DIRECTORIES[0]
        (model / "sources").mkdir(parents=True)
        (model / "manifest.json").write_text("{}", encoding="utf-8")
        (model / "sources" / "en_US.lm").write_bytes(b"model")

    def copy(self, relative: str) -> None:
        target = self.root / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(PROJECT_ROOT / relative, target)

    def digest(self) -> str:
        return inputs.inputs_digest(self.root)

    def define(self, line: str) -> None:
        module = self.root / USED_CONSTANT_MODULE
        module.write_text(module.read_text(encoding="utf-8") + line + "\n", encoding="utf-8")

    def test_the_digest_of_the_tree_is_stable_and_skips_bytecode(self) -> None:
        before = self.digest()
        cache = self.root / inputs.INPUT_DIRECTORIES[0] / inputs.SKIPPED_DIRECTORY
        cache.mkdir()
        (cache / "manifest.cpython.pyc").write_bytes(b"bytecode")

        self.assertEqual(before, self.digest())

    def test_a_changed_or_added_model_file_changes_the_digest(self) -> None:
        before = self.digest()
        (self.root / inputs.INPUT_DIRECTORIES[0] / "sources" / "en_US.lm").write_bytes(b"other model")
        changed = self.digest()
        (self.root / inputs.INPUT_DIRECTORIES[0] / "holdout-preseal.json").write_text("{}", encoding="utf-8")

        self.assertNotEqual(before, changed)
        self.assertNotEqual(changed, self.digest())

    def test_a_changed_toolchain_file_changes_the_digest(self) -> None:
        before = self.digest()
        detector = self.root / "src/keyswitch/detector.py"
        detector.write_text(detector.read_text(encoding="utf-8") + "\n", encoding="utf-8")

        self.assertNotEqual(before, self.digest())

    def test_only_a_constant_the_evaluation_uses_changes_the_digest(self) -> None:
        before = self.digest()
        self.define(f"{UNUSED_CONSTANT}: Final = 0")
        unused = self.digest()
        self.define(f"{USED_CONSTANT}: Final = {USED_CONSTANT} + 1")

        self.assertEqual(before, unused)
        self.assertNotEqual(unused, self.digest())

    def test_a_missing_input_is_refused(self) -> None:
        (self.root / "src/keyswitch/history.py").unlink()

        with self.assertRaisesRegex(inputs.InputsUnreadable, "history.py is unreadable"):
            self.digest()

    def test_a_missing_model_directory_is_refused(self) -> None:
        shutil.rmtree(self.root / inputs.INPUT_DIRECTORIES[0])

        with self.assertRaisesRegex(inputs.InputsUnreadable, "is not a directory"):
            self.digest()

    def test_constants_that_cannot_be_pinned_are_refused(self) -> None:
        (self.root / USED_CONSTANT_MODULE).write_text("this is not python\n", encoding="utf-8")

        with self.assertRaisesRegex(inputs.InputsUnreadable, "constants cannot be pinned"):
            self.digest()

    def test_the_command_prints_the_digest_or_names_the_unreadable_input(self) -> None:
        expected = self.digest()
        output, errors = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(output):
            printed = inputs.main(["--project-root", str(self.root)])
        shutil.rmtree(self.root / inputs.INPUT_DIRECTORIES[0])
        with contextlib.redirect_stderr(errors):
            refused = inputs.main(["--project-root", str(self.root)])

        self.assertEqual((printed, output.getvalue().strip()), (0, expected))
        self.assertEqual(refused, 1)
        self.assertIn("strict evaluation inputs unreadable", errors.getvalue())


class KeptReportWorkflowContractTests(unittest.TestCase):
    def test_ci_keys_a_kept_report_by_the_inputs_and_verifies_it_before_handing_it_on(self) -> None:
        text = (PROJECT_ROOT / ".github/workflows/tests.yml").read_text("utf-8")
        kept = text.split("- name: Verify the kept strict report against this tree")[1].split("- name:")[0]

        self.assertIn("steps.restore_intent.outputs.cache-hit == 'true'", kept)
        self.assertIn("python3 tools/verify_intent_strict_report.py", kept)
        self.assertIn("--report build/keyswitch-intent-strict.json", kept)
        self.assertEqual(text.count("steps.intent_inputs.outputs.inputs"), len(("restore", "save")))
        self.assertIn('echo "inputs=$(python3 tools/intent_strict_inputs.py)"', text)

    def test_the_release_build_evaluates_again(self) -> None:
        text = (PROJECT_ROOT / ".github/workflows/release.yml").read_text("utf-8")

        self.assertNotIn("intent_strict_inputs", text)
        self.assertIn("tools/evaluate_intent_model.py --strict", text)


if __name__ == "__main__":
    unittest.main()
