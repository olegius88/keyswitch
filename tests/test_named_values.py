"""Every number in the code, the tests and the tools has a name (AGENTS.md)."""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))

import check_named_values as checker  # noqa: E402

UNNAMED_SAMPLE = "def wait() -> float:\n    return 12.5\n"
NAMED_SAMPLE = "WAIT_SECONDS = 12.5\nNEUTRAL = (0, 1, -1)\n\n\nclass Timer:\n    LIMIT: float = 45.0\n"
SAMPLE_UNNAMED_VALUE = 12.5
SAMPLE_UNNAMED_LINE = 2


class NamedValuesTests(unittest.TestCase):
    def test_the_repository_names_every_number_outside_the_files_awaiting_reseal(self) -> None:
        violations: list[str] = []
        for path in checker.scanned_files():
            if path.relative_to(checker.ROOT).as_posix() not in checker.PENDING_RESEAL:
                violations.extend(str(finding) for finding in checker.findings(path))
        self.assertEqual(violations, [])

    def test_every_file_awaiting_reseal_still_holds_an_unnamed_number(self) -> None:
        """A cleaned file is taken off the list, so the list only ever shrinks."""
        for name in sorted(checker.PENDING_RESEAL):
            with self.subTest(file=name):
                path = checker.ROOT / name
                self.assertTrue(path.is_file())
                self.assertTrue(checker.findings(path))

    def test_a_number_spelled_in_place_is_found_and_a_named_one_is_not(self) -> None:
        with tempfile.TemporaryDirectory(dir=checker.ROOT) as temporary:
            unnamed = Path(temporary) / "unnamed.py"
            named = Path(temporary) / "named.py"
            unnamed.write_text(UNNAMED_SAMPLE, encoding="utf-8")
            named.write_text(NAMED_SAMPLE, encoding="utf-8")
            (finding,) = checker.findings(unnamed)
            self.assertEqual((finding.line, finding.value), (SAMPLE_UNNAMED_LINE, SAMPLE_UNNAMED_VALUE))
            self.assertIn(f":{SAMPLE_UNNAMED_LINE}: {SAMPLE_UNNAMED_VALUE!r}", str(finding))
            self.assertEqual(checker.findings(named), [])


if __name__ == "__main__":
    unittest.main()
