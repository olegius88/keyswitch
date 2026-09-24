"""Every value lives once, in a constants folder (AGENTS.md)."""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))

import check_named_values as checker  # noqa: E402
from fixture_values.counts import (
    NAMED_VALUES_SAMPLE_UNNAMED_LINE,
    NAMED_VALUES_SAMPLE_UNNAMED_VALUE,
)

APPLICATION_CONSTANTS = "src/keyswitch/constants"
TEST_CONSTANTS = "tests/fixture_values"
OTHER_APPLICATION_CONSTANTS = "apps/logcourier/src/logcourier/constants"
UNNAMED_SAMPLE = "def wait() -> float:\n    return 12.5\n"
NEUTRAL_SAMPLE = "def step(index: int) -> int:\n    return index + 1 - 0 * -1\n"
CONSTANTS_SAMPLE = '"""Waits."""\n\nWAIT_SECONDS = 12.5\nWAITS = (WAIT_SECONDS, 45.0)\n'
DECLARED_VALUE_SAMPLE = "WAIT_SECONDS = 12.5\n"
SECOND_NAME_SAMPLE = (
    "from keyswitch.constants.timing import WAIT_SECONDS\n\n"
    "PAUSE_SECONDS = WAIT_SECONDS\n"
    "PAUSES = (WAIT_SECONDS, -WAIT_SECONDS)\n\n\n"
    "class Timer:\n"
    "    LIMIT = WAIT_SECONDS + 1\n"
)
NOT_VALUES_SAMPLE = (
    "import enum\n"
    "from keyswitch.constants.timing import WAIT_SECONDS\n\n"
    "HANDLERS = (int, str)\n"
    "CACHE: dict[str, float] = {}\n"
    'NAMES = frozenset({"wait"})\n\n\n'
    "class Kind(enum.Enum):\n"
    "    WAIT = WAIT_SECONDS\n"
)
SAME_NAME_SAMPLE = "WAIT_SECONDS = 12.5\n"
# Rule 5: each literal below spells the values in TEXT_VALUES, in the order the checker reports them.
TEXT_VALUES_SAMPLE = '''"""A docstring may say 5 секунд, 4–12 and шесть часов."""

from keyswitch.constants.timing import WAIT_SECONDS

"Bare string statements are documentation too: 30 seconds."
DELAY = "через 3 секунды"
HALF = "на 2,5 секунды"
PACE = "50 ms before every key-down, 1.7 s idle, a 96-character clip"
SIZES = ("до 128 МиБ", "the 12 MiB bound", "99.7% of rows")
RANGE = "длиной 4–12"
DOTS = "rotations 0..20"
SPOKEN = "от 1 до 80"
SPAN = "from 2 to 128"
THROUGH = "3 through 16"
BOUND = "Поддерживается до 50 источников"
POWER = "the 2^20 bound"
WORDS = "каждые шесть часов"
PLACED = f"через {WAIT_SECONDS} секунд, {WAIT_SECONDS:.1f} МиБ"
'''
TEXT_VALUES = (
    ("3 секунды",), ("2,5 секунды",), ("50 ms", "1.7 s", "96-character"), ("128 МиБ", "до 128"), ("12 MiB",),
    ("99.7%",), ("4–12",), ("0..20",), ("от 1 до 80", "до 80"), ("from 2 to 128",), ("3 through 16",),
    ("до 50",), ("2^20",), ("шесть часов",),
)
NOT_TEXT_VALUES_SAMPLE = '''NOT_VALUES = (
    "Загружаем версию 0.16.2: 0%", "Unit tests with 100% branch coverage", "--jobs must be at least 1",
    "от 0 до 1", "24.09.2026", "2026-09-24", "SHA-256", "X11", "Win32 hook", "-1001234567890",
    "1. Создайте бота в @BotFather", "по одному слову на строку", "до 10:00", "x86_64 bytes",
    "Inno Setup 6", "[a-z]{3,16}", "group 0 or 1", "0 bytes", "early switch from four letters",
    "однобуквенное слово", "Когда ни один словарь не знает слово",
)
'''


def write(root: Path, relative: str, text: str) -> Path:
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    return path


class NamedValuesTests(unittest.TestCase):
    def test_the_repository_follows_the_rule_outside_the_files_awaiting_reseal(self) -> None:
        violations = [str(finding) for finding in checker.duplicate_names()]
        for path in checker.scanned_files():
            if path.relative_to(checker.ROOT).as_posix() not in checker.PENDING_RESEAL:
                violations.extend(str(finding) for finding in checker.findings(path))
        self.assertEqual(violations, [])

    def test_every_file_awaiting_reseal_still_breaks_the_rule(self) -> None:
        """A file that follows the rule is taken off the list, so the list only ever shrinks."""
        for name in sorted(checker.PENDING_RESEAL):
            with self.subTest(file=name):
                path = checker.ROOT / name
                self.assertTrue(path.is_file())
                self.assertTrue(checker.findings(path))

    def test_a_number_spelled_in_place_is_found(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (finding,) = checker.findings(write(root, "src/keyswitch/engine.py", UNNAMED_SAMPLE), root)
            self.assertEqual(finding.line, NAMED_VALUES_SAMPLE_UNNAMED_LINE)
            self.assertIn(f":{NAMED_VALUES_SAMPLE_UNNAMED_LINE}: {NAMED_VALUES_SAMPLE_UNNAMED_VALUE!r}", str(finding))

    def test_zero_and_one_need_no_name(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.assertEqual(checker.findings(write(root, "tools/step.py", NEUTRAL_SAMPLE), root), [])

    def test_a_constants_folder_holds_numbers_only_in_module_level_constants(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            named = write(root, f"{APPLICATION_CONSTANTS}/timing.py", CONSTANTS_SAMPLE)
            self.assertEqual(checker.findings(named, root), [])
            loose = write(root, f"{TEST_CONSTANTS}/clock.py", UNNAMED_SAMPLE)
            (finding,) = checker.findings(loose, root)
            self.assertIn("outside a module-level constant", finding.message)

    def test_a_value_declared_outside_a_constants_folder_is_found(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            messages = [finding.message for finding in
                        checker.findings(write(root, "tools/wait.py", DECLARED_VALUE_SAMPLE), root)]
            self.assertTrue(any("belongs in a constants folder" in message for message in messages))
            self.assertIn("WAIT_SECONDS is a value; declare it in a constants folder", messages)

    def test_a_second_name_for_a_value_is_found(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            messages = {finding.message.split()[0] for finding in
                        checker.findings(write(root, "tests/test_wait.py", SECOND_NAME_SAMPLE), root)}
            self.assertEqual(messages, {"PAUSE_SECONDS", "PAUSES", "LIMIT"})

    def test_tables_of_objects_caches_and_enumerations_are_not_values(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.assertEqual(checker.findings(write(root, "src/keyswitch/kinds.py", NOT_VALUES_SAMPLE), root), [])

    def test_a_name_declared_in_two_folders_of_one_application_is_found(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            write(root, f"{APPLICATION_CONSTANTS}/timing.py", SAME_NAME_SAMPLE)
            write(root, f"{OTHER_APPLICATION_CONSTANTS}/timing.py", SAME_NAME_SAMPLE)
            self.assertEqual(checker.duplicate_names(root), [])
            write(root, f"{TEST_CONSTANTS}/clock.py", SAME_NAME_SAMPLE)
            (finding,) = checker.duplicate_names(root)
            self.assertEqual(finding.path, f"{TEST_CONSTANTS}/clock.py")
            self.assertIn(f"WAIT_SECONDS is also declared in {APPLICATION_CONSTANTS}/timing.py", finding.message)

    def test_a_value_spelled_in_a_text_is_found_outside_docstrings(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            found = checker.findings(write(root, "src/keyswitch/texts.py", TEXT_VALUES_SAMPLE), root)
            self.assertEqual(
                sorted(finding.message for finding in found),
                sorted(f"text spells {', '.join(map(repr, values))}; format it from the constant it describes"
                       for values in TEXT_VALUES),
            )

    def test_paths_versions_dates_names_samples_and_neutral_amounts_are_not_values(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.assertEqual(checker.findings(write(root, "tools/texts.py", NOT_TEXT_VALUES_SAMPLE), root), [])

    def test_texts_are_read_in_the_application_tools_and_apps_but_not_in_tests_or_constants(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for relative in ("tools/texts.py", "apps/logcourier/src/logcourier/texts.py", "apps/logcourier/tools/texts.py"):
                with self.subTest(file=relative):
                    self.assertTrue(checker.findings(write(root, relative, TEXT_VALUES_SAMPLE), root))
            for relative in ("tests/test_texts.py", "apps/logcourier/tests/test_texts.py",
                             f"{APPLICATION_CONSTANTS}/texts.py", f"{OTHER_APPLICATION_CONSTANTS}/texts.py"):
                with self.subTest(file=relative):
                    self.assertEqual(checker.findings(write(root, relative, TEXT_VALUES_SAMPLE), root), [])


if __name__ == "__main__":
    unittest.main()
