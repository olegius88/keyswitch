"""The truncation filter of the Russian lexicon supplement: its rule, its receipt, its reproducibility."""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

import filter_lexicon_supplement as supplement_filter
from keyswitch.lexicon_supplement import supplement_words


class FakeSpeller:
    """Accepts exactly the given spellings, case-sensitively, as Hunspell does."""

    def __init__(self, *spellings: str) -> None:
        self.spellings = set(spellings)

    def check(self, word: str) -> bool:
        return word in self.spellings


class FilterRuleTests(unittest.TestCase):
    def test_only_unaccepted_proper_prefixes_of_onboard_words_are_dropped(self) -> None:
        onboard = sorted(["клуб", "которых", "машина", "таких"])
        speller = FakeSpeller("таки", "Маша")
        forms = ["зум", "клу", "которы", "маша", "таки", "клуб"]
        kept, dropped = supplement_filter.split_forms(forms, onboard, speller)
        # `клу`/`которы` begin onboard words and no dictionary accepts them; `таки` is a word and
        # `маша` a name spelled capitalised; `зум` begins no onboard word; `клуб` is one itself.
        self.assertEqual(dropped, {"клу": "клуб", "которы": "которых"})
        self.assertEqual(kept, ["зум", "маша", "таки", "клуб"])

    def test_the_output_names_the_filter_and_counts_what_it_dropped(self) -> None:
        payload = {"name": "v3", "scope": "Forms.", "selection": {"selected": len(["а", "б", "в"])},
                   "words": ["а", "б", "в"]}
        built = supplement_filter.build(payload, ["а"], {"б": "ба", "в": "ва"})
        self.assertEqual(built["name"], supplement_filter.NAME)
        self.assertEqual(built["words"], ["а"])
        self.assertEqual(built["selection"], {"selected": len(["а"]), "dropped_truncations": len(["б", "в"])})
        self.assertTrue(str(built["scope"]).startswith("Forms. Filtered by tools/filter_lexicon_supplement.py"))

    def test_an_already_filtered_input_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "input.json"
            path.write_text(json.dumps({"name": supplement_filter.NAME, "locale": "ru_RU", "words": ["абв"]}))
            with self.assertRaisesRegex(ValueError, "already filtered"):
                supplement_filter.main(["--input", str(path), "--output", str(Path(temporary) / "out.json"),
                                        "--receipt", str(Path(temporary) / "receipt.json")])


class PackagedSupplementTests(unittest.TestCase):
    def test_the_packaged_supplement_is_the_receipt_output_and_follows_the_rule(self) -> None:
        result = supplement_filter.verify()
        self.assertTrue(result["verified"])
        words = set(supplement_words("ru_RU"))
        for truncated in ("клу", "которы", "други", "сво", "истори", "нову"):
            with self.subTest(truncated=truncated):
                self.assertFalse(truncated in words, truncated)
        for kept in ("зум", "окей", "вау", "гифку", "флуд", "а", "я"):
            with self.subTest(kept=kept):
                self.assertTrue(kept in words, kept)

    def test_tampering_with_the_supplement_or_its_receipt_is_found(self) -> None:
        receipt = json.loads(supplement_filter.RECEIPT.read_bytes())
        forms = receipt["dropped"]["forms"]
        with tempfile.TemporaryDirectory() as temporary:
            changed = Path(temporary) / "receipt.json"
            for broken in ({**receipt, "dropped": {**receipt["dropped"], "forms": forms[1:]}},
                           {**receipt, "references": {}},
                           {**receipt, "output": {**receipt["output"], "sha256": "0" * len(receipt["output"]["sha256"])}}):
                changed.write_text(json.dumps(broken, ensure_ascii=False))
                with self.subTest(broken=sorted(broken)), patch.object(supplement_filter, "RECEIPT", changed), \
                        self.assertRaises(ValueError):
                    supplement_filter.verify()


if __name__ == "__main__":
    unittest.main()
