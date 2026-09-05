"""Model-blind source grouping, immutable text and reproducible snapshots."""
from __future__ import annotations

import tempfile
import unittest
import sys
from pathlib import Path

TOOLS_PATH = str(Path(__file__).resolve().parents[1] / "tools")
if TOOLS_PATH not in sys.path:
    sys.path.insert(0, TOOLS_PATH)

from context_corpus import Phrase, assign, canonical_tokens, load_archive, write_source


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
        records = [
            Phrase(1, "eng", "I would like some tea", ""),
            Phrase(2, "eng", "I would like some coffee", ""),
            Phrase(3, "eng", "I would like tea", ""),
            Phrase(4, "rus", "I would like some tea!", ""),
        ]
        rows, _ = assign(records)
        self.assertEqual(len({row.group for row in rows}), 1)
        self.assertEqual(len({row.split for row in rows}), 1)

    def test_source_order_cap_and_empty_input(self) -> None:
        records = [Phrase(index + 1, "eng", f"I would like some item{index}", "") for index in range(20)]
        rows, report = assign(records, per_group=3)
        self.assertEqual(assign(list(reversed(records)), per_group=3), (rows, report))
        self.assertEqual(report["maximum_group_size"], 20)
        self.assertEqual(len(rows), 3)
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
