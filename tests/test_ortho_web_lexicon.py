"""The web word forms the orthotactic source channel counts (tools/ortho_web_lexicon.py)."""
from __future__ import annotations

import gzip
import hashlib
import io
import json
import runpy
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
TOOLS_PATH = str(ROOT / "tools")
if TOOLS_PATH not in sys.path:
    sys.path.insert(0, TOOLS_PATH)

import ortho_web_lexicon as web  # noqa: E402

from keyswitch.constants.ortho import (  # noqa: E402
    ORTHO_V1_MAXIMUM_LEXICON_WEIGHT,
    ORTHO_WEB_LEXICON_SCHEMA_VERSION,
    ORTHO_WEB_LEXICON_TOP_PER_BILLION,
)
from fixture_values.ortho import WEB_FIXTURE_BEYOND_RANK, WEB_FIXTURE_FAR_RANK, WEB_FIXTURE_NEAR_RANK  # noqa: E402

# A vocabulary in fastText order: proper names, abbreviations in capitals and short forms are skipped;
# lowercase Cyrillic words and lowercase compounds with hyphens are kept.
WORDS = ["и", "в", "Москва", "мацуи", "Мацуи", "фнаф", "http", "йцукен", "ёлка", "кот", "а-бв", "РЕН-ТВ", "рен-тв",
         "МЦУИС", "фнафер", "тв-", "-тв"]
KEPT = ["мацуи", "фнаф", "йцукен", "ёлка", "а-бв", "рен-тв", "фнафер"]


def vectors(words: list[str]) -> bytes:
    lines = [f"{len(words)} 2"] + [f"{word} 0.5 0.25" for word in words]
    return gzip.compress(("\n".join(lines) + "\n").encode())


class WebLexiconTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / "cc.ru.300.vec.gz"
        self.source.write_bytes(vectors(WORDS))
        digest = hashlib.sha256(self.source.read_bytes()).hexdigest()
        for name, value in (("SOURCE_SHA256", digest), ("FORMS", self.root / "forms.json.gz"),
                            ("RECEIPT", self.root / "receipt.json")):
            patcher = patch.object(web, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)

    def run_main(self, *arguments: str) -> dict[str, object]:
        output = io.StringIO()
        with redirect_stdout(output):
            self.assertEqual(web.main(list(arguments)), 0)
        return dict(json.loads(output.getvalue()))

    def test_the_weight_follows_the_rank_like_an_onboard_frequency(self) -> None:
        top = min(ORTHO_V1_MAXIMUM_LEXICON_WEIGHT, ORTHO_WEB_LEXICON_TOP_PER_BILLION.bit_length())
        self.assertEqual(web.web_weight(0), top)
        self.assertGreater(web.web_weight(WEB_FIXTURE_NEAR_RANK), web.web_weight(WEB_FIXTURE_FAR_RANK))
        self.assertEqual(web.web_weight(WEB_FIXTURE_BEYOND_RANK), 1)

    def test_only_lowercase_cyrillic_forms_long_enough_are_taken_with_their_rank(self) -> None:
        self.assertEqual(web.derive(self.source), [[word, WORDS.index(word)] for word in KEPT])

    def test_a_source_that_is_not_the_pinned_file_is_refused(self) -> None:
        other = self.root / "other.vec.gz"
        other.write_bytes(vectors(KEPT))
        with self.assertRaises(ValueError):
            web.derive(other)

    def test_the_frozen_file_is_byte_stable_and_loads_as_weights(self) -> None:
        forms = web.derive(self.source)
        digest = web.write(forms, self.root / "a.json.gz")
        self.assertEqual(digest, web.write(forms, self.root / "b.json.gz"))
        self.assertEqual(web.load(self.root / "a.json.gz"), {word: web.web_weight(WORDS.index(word)) for word in KEPT})
        broken = self.root / "broken.json.gz"
        broken.write_bytes(gzip.compress(json.dumps({"schema_version": 0, "forms": []}).encode()))
        with self.assertRaises(ValueError):
            web.load(broken)

    def test_freeze_writes_the_forms_and_a_receipt_that_pins_source_licence_and_forms(self) -> None:
        recorded = self.run_main("--freeze", str(self.source))
        self.assertEqual(recorded["schema_version"], ORTHO_WEB_LEXICON_SCHEMA_VERSION)
        self.assertEqual(recorded["forms"], len(KEPT))
        self.assertEqual(recorded["forms_sha256"], hashlib.sha256(web.FORMS.read_bytes()).hexdigest())
        source = dict(recorded["source"])  # type: ignore[call-overload]
        self.assertEqual((source["sha256"], source["license"]), (web.SOURCE_SHA256, web.LICENSE))
        self.assertIn("no lawyer has reviewed", str(source["license_note"]))
        with self.assertRaises(ValueError):                     # a frozen lexicon is never overwritten
            web.main(["--freeze", str(self.source)])
        self.assertEqual(self.run_main(), recorded)              # a plain run checks and prints the receipt
        self.assertEqual(self.run_main("--source", str(self.source)), recorded)   # and replays from the source
        self.assertFalse(web.FORMS.with_name(".web-lexicon-check.json.gz").exists())

    def test_the_command_line_entry_point_runs_main(self) -> None:
        with patch.object(sys, "argv", ["ortho_web_lexicon.py", "--help"]), redirect_stdout(io.StringIO()) as output:
            with self.assertRaises(SystemExit) as stopped:
                runpy.run_path(TOOLS_PATH + "/ortho_web_lexicon.py", run_name="__main__")
        self.assertEqual(stopped.exception.code, 0)
        self.assertIn("--freeze", output.getvalue())

    def test_a_changed_file_or_source_is_refused(self) -> None:
        self.run_main("--freeze", str(self.source))
        forms = web.derive(self.source)
        with patch.object(web, "derive", return_value=forms[:-1]):
            with self.assertRaises(ValueError):
                web.main(["--source", str(self.source)])
        self.assertFalse(web.FORMS.with_name(".web-lexicon-check.json.gz").exists())
        web.write(forms[:-1], web.FORMS)
        with self.assertRaises(ValueError):
            web.main([])


if __name__ == "__main__":
    unittest.main()
