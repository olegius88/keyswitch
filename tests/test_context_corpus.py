"""Model-blind source grouping, immutable text and reproducible snapshots."""
from __future__ import annotations

import tempfile
import unittest
import sys
from pathlib import Path

TOOLS_PATH = str(Path(__file__).resolve().parents[1] / "tools")
if TOOLS_PATH not in sys.path:
    sys.path.insert(0, TOOLS_PATH)

from context_corpus import Locale, Phrase, assign, canonical_tokens, load_archive, write_source


class CorpusTests(unittest.TestCase):
    def test_orthography_and_whitespace_group_without_changing_text(self) -> None:
        texts = ["Ещё  раз попробуем!", "Еще раз попробуем.", "ЕЩЕ РАЗ ПОПРОБУЕМ"]
        records = [Phrase(index + 1, "rus", text, "2026-01-01") for index, text in enumerate(texts)]
        assigned, report = assign(records)
        self.assertEqual({row.phrase.text for row in assigned}, set(texts))
        self.assertEqual(len({row.group for row in assigned}), 1)
        self.assertEqual(report["groups"], 1)
        self.assertEqual(canonical_tokens("We don’t know."), canonical_tokens("we don't know"))

    def test_single_token_variants_and_disagreeing_language_tags_do_not_leak(self) -> None:
        fixtures: list[tuple[Locale, str]] = [
            ("eng", "I would like some tea"),
            ("eng", "I would like some coffee"),
            ("eng", "I would like tea"),
            ("rus", "I would like some tea!"),
        ]
        records = [Phrase(index + 1, locale, text, "") for index, (locale, text) in enumerate(fixtures)]
        rows, _ = assign(records)
        self.assertEqual(len({row.group for row in rows}), 1)
        self.assertEqual(len({row.split for row in rows}), 1)

    # More source records than PER_GROUP_CAP allows, to see the cap and the
    # order-independent tie-break both apply.
    FIXTURE_PHRASE_COUNT = 20
    PER_GROUP_CAP = 3

    def test_source_order_cap_and_empty_input(self) -> None:
        records = [Phrase(index + 1, "eng", f"I would like some item{index}", "")
                   for index in range(self.FIXTURE_PHRASE_COUNT)]
        rows, report = assign(records, per_group=self.PER_GROUP_CAP)
        self.assertEqual(assign(list(reversed(records)), per_group=self.PER_GROUP_CAP), (rows, report))
        self.assertEqual(report["maximum_group_size"], self.FIXTURE_PHRASE_COUNT)
        self.assertEqual(len(rows), self.PER_GROUP_CAP)
        self.assertEqual(assign([])[1]["groups"], 0)
        with self.assertRaises(ValueError):
            assign(records, per_group=0)

    def test_frozen_source_is_deterministic_and_unreviewed_archive_is_rejected(self) -> None:
        records = [Phrase(1, "rus", "вот  так", "2026-01-01")]
        with tempfile.TemporaryDirectory() as directory:
            first, second = Path(directory) / "a.gz", Path(directory) / "b.gz"
            self.assertEqual(write_source(records, first), write_source(records, second))
            self.assertEqual(first.read_bytes(), second.read_bytes())
            with self.assertRaisesRegex(ValueError, "checksum"):
                load_archive(first)


if __name__ == "__main__":
    unittest.main()
