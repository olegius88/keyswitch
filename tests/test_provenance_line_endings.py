"""Every file hashed into model provenance must survive any checkout.

A digest is taken on the machine that trains a model and re-checked by the
package build on both platforms. If git is allowed to translate end-of-line
bytes for such a file, the check fails on Windows for a file nobody edited,
and the failure is invisible on Linux. `.gitattributes` therefore pins each of
them to `eol=lf` or marks it binary; this test fails as soon as a new hashed
file is added without that pin, on either platform.
"""

from __future__ import annotations

import json
import subprocess
import sys
import unittest
from collections.abc import Iterator
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TOOLS_PATH = str(ROOT / "tools")
if TOOLS_PATH not in sys.path:
    sys.path.insert(0, TOOLS_PATH)

import release_pipeline  # noqa: E402
import verify_context_model  # noqa: E402
import verify_context_v2_history  # noqa: E402

# Mappings whose keys are repository-relative paths of hashed files. Reading the
# recorded evidence instead of importing every trainer keeps future provenance
# lists covered without another edit here.
PROVENANCE_KEYS = ("provenance", "source_hashes")
# `git check-attr` prints "<path>: <attribute>: <value>"; splitting from the
# right on ": " at most twice yields exactly these three fields.
MAX_ATTR_LINE_SPLITS = 2
EXPECTED_ATTR_FIELD_COUNT = 3


def recorded_paths() -> Iterator[Path]:
    for report in sorted((ROOT / "model").rglob("*.json")):
        try:
            payload: object = json.loads(report.read_text(encoding="utf-8"))
        except (ValueError, UnicodeDecodeError):
            continue
        if not isinstance(payload, dict):
            continue
        for key in PROVENANCE_KEYS:
            mapping = payload.get(key)
            if not isinstance(mapping, dict):
                continue
            for relative in mapping:
                candidate = ROOT / str(relative).replace("\\", "/")
                if candidate.is_file() and candidate.is_relative_to(ROOT):
                    yield candidate


def hashed_files() -> list[Path]:
    paths = set(recorded_paths())
    paths |= set(verify_context_model.provenance_paths().values())
    paths |= {ROOT / relative for relative in release_pipeline.MODEL_TOOLCHAIN_PATHS.values()}
    paths |= {ROOT / verify_context_v2_history.ARCHIVE / Path(name).name for name in verify_context_v2_history.SOURCES}
    return sorted(path for path in paths if path.is_file())


def unprotected(paths: list[Path]) -> list[str]:
    relative = [path.relative_to(ROOT).as_posix() for path in paths]
    result = subprocess.run(
        ["git", "check-attr", "--stdin", "eol", "binary"],
        cwd=ROOT, input="\n".join(relative), capture_output=True, text=True, check=True,
    )
    attributes: dict[str, set[str]] = {name: set() for name in relative}
    for line in result.stdout.splitlines():
        # `git check-attr` prints `<path>: <attribute>: <value>`; only the last
        # two fields are fixed, so split from the right and keep the path whole.
        fields = line.rsplit(": ", MAX_ATTR_LINE_SPLITS)
        if len(fields) != EXPECTED_ATTR_FIELD_COUNT:
            continue
        path, attribute, value = fields
        attributes.setdefault(path, set()).add(f"{attribute}={value}")
    return sorted(
        name for name in relative
        if "eol=lf" not in attributes.get(name, set())
        and "binary=set" not in attributes.get(name, set())
    )


class ProvenanceLineEndingTests(unittest.TestCase):
    MINIMUM_HASHED_FILE_COUNT = 30

    def setUp(self) -> None:
        if not (ROOT / ".git").exists():
            self.skipTest("attributes can only be resolved inside a git checkout")

    def test_every_hashed_file_is_pinned_against_eol_conversion(self) -> None:
        paths = hashed_files()
        self.assertGreater(len(paths), self.MINIMUM_HASHED_FILE_COUNT, "provenance discovery collected almost nothing")
        self.assertIn(ROOT / "src/keyswitch/short_words.py", paths)
        self.assertEqual(
            unprotected(paths), [],
            "these hashed files are not pinned in .gitattributes; add "
            "`<path> text eol=lf` (or `binary`) or a checkout will invalidate the model",
        )

    def test_detects_a_hashed_file_without_a_pin(self) -> None:
        # git check-attr resolves unmatched paths without creating a file.
        unpinned = ROOT / "unprotected-provenance-fixture.dat"
        self.assertEqual(unprotected([unpinned]), [unpinned.relative_to(ROOT).as_posix()])


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    return unittest.TestSuite(
        ProvenanceLineEndingTests(name)
        for name in ProvenanceLineEndingTests.__dict__ if name.startswith("test_")
    )
