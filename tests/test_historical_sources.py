"""Archived bytes of rejected generations: every pin resolves, nothing falls back to a live file."""
from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
TOOLS_PATH = str(ROOT / "tools")
if TOOLS_PATH not in sys.path:
    sys.path.insert(0, TOOLS_PATH)

import historical_sources  # noqa: E402
from historical_sources import ARCHIVES, BOUNDARY_V1, CONTEXT_V2, ORTHO_V2, SourceArchive, checksum  # noqa: E402
from keyswitch.constants.file_formats import SHA256_HEX_CHARACTERS  # noqa: E402

# The evidence each archive serves: the fields that map repository paths to digests, and the
# receipt fields that record the digest of one tool.
PINS = {
    CONTEXT_V2.name: (("model/context_v2/candidate-seal.json", "provenance"),
                      ("model/context_v2/lexical-receipt.json", "source_hashes"),
                      ("model/context_v2/engine-report.json", "provenance")),
    BOUNDARY_V1.name: (("model/boundary_v1/seal.json", "provenance"),),
    ORTHO_V2.name: (("model/ortho_v2/seal.json", "provenance"), ("model/ortho_v2/corpus-receipt.json", "provenance")),
}
TOOL_DIGESTS = {
    CONTEXT_V2.name: (("model/context_v2/corpus-receipt.json", "builder_sha256", "tools/context_corpus.py"),),
    BOUNDARY_V1.name: (),
    ORTHO_V2.name: (("model/ortho_v2/sources/known-receipt.json", "generator_sha256", "tools/ortho_v2_known.py"),
                    ("model/ortho_v2/sources/verified-receipt.json", "generator_sha256", "tools/ortho_v2_verified.py"),
                    ("model/ortho_v2/sources/verified-receipt.json", "corpus_tool_sha256", "tools/ortho_v2_corpus.py")),
}


def field(name: str, key: str) -> object:
    payload: object = json.loads((ROOT / name).read_bytes())
    assert isinstance(payload, dict)
    return payload[key]


class SourceArchiveTests(unittest.TestCase):
    def test_every_archived_copy_has_its_reviewed_digest_and_nothing_else_is_archived(self) -> None:
        for archive in ARCHIVES:
            with self.subTest(archive=archive.name):
                self.assertEqual(archive.verify(archive.sources), dict(archive.sources))
                directory = ROOT / archive.directory
                stored = {path.relative_to(directory).as_posix() for path in directory.rglob("*") if path.is_file()}
                self.assertEqual(stored, set(archive.sources))

    def test_every_pin_of_the_evidence_resolves_without_reading_a_live_file(self) -> None:
        for archive in ARCHIVES:
            live = {ROOT / name for name in archive.sources}

            def digest(path: Path, live: set[Path] = live) -> str:
                if path in live:
                    raise AssertionError("historical evidence read a live file: " + str(path))
                return checksum(path)

            with self.subTest(archive=archive.name), patch.object(historical_sources, "checksum", side_effect=digest):
                served: set[str] = set()
                for name, key in PINS[archive.name]:
                    served |= set(archive.verify(field(name, key)))
                for name, key, tool in TOOL_DIGESTS[archive.name]:
                    archive.recorded_digest(tool, field(name, key))
                    served.add(tool)
                # No archived copy is kept for nothing.
                self.assertEqual(set(archive.sources) - served, set())

    def test_frozen_directories_are_read_in_place_and_every_other_pin_needs_its_archived_copy(self) -> None:
        with tempfile.TemporaryDirectory(prefix="keyswitch-historical-sources-") as temporary:
            root = Path(temporary)
            (root / "own").mkdir()
            (root / "own/evidence.json").write_text("{}", encoding="utf-8")
            (root / "archive/tools").mkdir(parents=True)
            (root / "archive/tools/tool.py").write_text("old tool", encoding="utf-8")
            (root / "tools").mkdir()
            (root / "tools/tool.py").write_text("new tool", encoding="utf-8")
            old, own = checksum(root / "archive/tools/tool.py"), checksum(root / "own/evidence.json")
            archive = SourceArchive("fixture", "archive", ("own",), {"tools/tool.py": old})
            self.assertEqual(archive.path("tools/tool.py", root), root / "archive/tools/tool.py")
            self.assertEqual(archive.verify({"own/evidence.json": own, "tools\\tool.py": old}, root),
                             {"own/evidence.json": own, "tools/tool.py": old})
            # The live bytes are never an alternative to the archived ones.
            with self.assertRaisesRegex(ValueError, "unapproved"):
                archive.verify({"tools/tool.py": checksum(root / "tools/tool.py")}, root)
            with self.assertRaisesRegex(ValueError, "not archived"):
                archive.verify({"tools/other.py": old}, root)
            for pins in ({"own/evidence.json": old}, {"own/missing.json": own}):
                with self.subTest(pins=pins), self.assertRaisesRegex(ValueError, "source changed"):
                    archive.verify(pins, root)
            with self.assertRaisesRegex(ValueError, "other files"):
                archive.recorded({"tools/tool.py": old}, ("tools/tool.py", "own/evidence.json"), root)
            self.assertEqual(archive.recorded({"tools/tool.py": old}, ("tools/tool.py",), root), {"tools/tool.py": old})
            self.assertEqual(archive.recorded_digest("tools/tool.py", old, root), old)
            for digest in (None, "x", old.upper()):
                with self.subTest(digest=digest), self.assertRaisesRegex(ValueError, "invalid historical source pin"):
                    archive.recorded_digest("tools/tool.py", digest, root)
            (root / "archive/tools/tool.py").write_text("tampered", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "source changed"):
                archive.verify({"tools/tool.py": old}, root)

    def test_pins_outside_the_repository_are_refused(self) -> None:
        digest = "a" * SHA256_HEX_CHARACTERS
        invalid: tuple[object, ...] = ({}, [], {"/outside.py": digest}, {"../outside.py": digest}, {"C:\\outside.py": digest})
        for pins in invalid:
            with self.subTest(pins=pins), self.assertRaises(ValueError):
                CONTEXT_V2.verify(pins)


if __name__ == "__main__":
    unittest.main()
