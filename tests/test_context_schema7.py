"""Feature schema 7: where on its line a word stands, its case, how often its readings occur, and tokens of signs."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from keyswitch import context_model
from keyswitch.constants.models import CONTEXT_LINE_FEATURE_VERSION, CONTEXT_OPENING_FEATURE_VERSION
from keyswitch.context_model import (
    TERM_ALPHABETS, TERM_FREQUENCY_PATH, ContextEvidence, ContextModel, extract_context_features, load_term_frequency, term_bucket,
)
from keyswitch.input_context import FieldContext


def features(original: str, alternative: str, group: int, before: str = "", after: str = "", *,
             source_known: bool = False, target_known: bool = False, inside: bool = False) -> dict[str, float]:
    item = ContextEvidence(original, alternative, group, FieldContext("", "1", before, after, "text"),
                           source_known=source_known, target_known=target_known, inside=inside)
    return extract_context_features(item, CONTEXT_LINE_FEATURE_VERSION)


def names(prefix: str, found: dict[str, float]) -> set[str]:
    return {name for name in found if name.startswith(prefix)}


class LineTests(unittest.TestCase):
    def test_the_first_word_of_a_message(self) -> None:
        self.assertIn("line:empty", features("vs", "мы", 0))

    def test_the_first_word_of_a_new_line_under_english_text(self) -> None:
        found = features("vs", "мы", 0, "Sounds good, the build is green.\n")
        self.assertIn("line:newline", found)
        self.assertIn("line:newline:previous:en:direction:0", found)

    def test_the_first_word_of_a_new_line_under_russian_text_or_signs(self) -> None:
        self.assertIn("line:newline:previous:ru:direction:0", features("vs", "мы", 0, "Всё понятно.\n"))
        self.assertIn("line:newline:previous:none:direction:0", features("vs", "мы", 0, "---\n"))
        # A comment marker before the word has no letters: still the start of its line.
        self.assertIn("line:newline", features("vs", "мы", 0, "code();\n    // "))

    def test_a_word_with_words_before_it_on_its_line(self) -> None:
        self.assertIn("line:same", features("vs", "мы", 0, "React "))


class CaseTests(unittest.TestCase):
    def test_case_patterns_of_the_token(self) -> None:
        self.assertIn("case:inner:direction:1", features("ЦштАщкьы", "WinForms", 1))
        self.assertIn("case:upper:direction:1", features("ЦЗА", "WPF", 1))
        self.assertIn("case:title:direction:0", features("Ghbdtn", "Привет", 0))
        self.assertIn("case:lower:direction:0", features("ghbdtn", "привет", 0))

    def test_a_single_letter_has_no_case_pattern(self) -> None:
        self.assertFalse(names("case:", features("D", "В", 0)))


class LatinReadingTests(unittest.TestCase):
    def test_signs_inside_the_latin_reading(self) -> None:
        found = features("ЫуеештпыюВуафгде", "Settings.Default", 1)
        self.assertIn("latin:dot:direction:1", found)
        self.assertNotIn("latin:underscore:direction:1", found)
        self.assertIn("latin:underscore:direction:0", features("file_get_contents", "аш|у_пуе_сщтеутеы", 0))
        # A leading or trailing underscore is not inside the reading.
        self.assertNotIn("latin:underscore:direction:0", features("_init", "_штше", 0))


class TermFrequencyTests(unittest.TestCase):
    def test_terms_and_slang_that_occur_in_russian_text(self) -> None:
        self.assertEqual(term_bucket("id", "latin"), "4")
        self.assertEqual(term_bucket("gla", "latin"), "0")
        self.assertEqual(term_bucket("пдф", "cyrillic"), "2")
        self.assertEqual(term_bucket("дге", "cyrillic"), "0")

    def test_signs_around_a_word_are_stripped_and_anything_else_is_not_a_word(self) -> None:
        self.assertEqual(term_bucket("(id),", "latin"), term_bucket("id", "latin"))
        self.assertEqual(term_bucket("a.b", "latin"), "na")
        self.assertEqual(term_bucket("...", "latin"), "na")

    def test_the_buckets_of_both_readings_are_features(self) -> None:
        found = features("пдф", "gla", 1, "я отправил ")
        self.assertIn("freq:latin:0:0:direction:1", found)
        self.assertIn("freq:cyrillic:2:direction:1", found)
        self.assertIn("freq:0:0:2:direction:1", found)

    def test_words_are_counted_in_text_of_their_own_language_too(self) -> None:
        self.assertEqual(term_bucket("uh", "english"), "1")
        self.assertEqual(term_bucket("gla", "english"), "0")
        self.assertEqual(term_bucket("на", "russian"), "4")
        self.assertEqual(term_bucket("гр", "russian"), "1")

    def test_a_reading_never_seen_inside_russian_text_is_told_by_english_text(self) -> None:
        # `uh` and `gla` both never occur inside Russian text; only `uh` is an English word.
        self.assertIn("freq:latin:0:1:direction:1", features("гр", "uh", 1, after="why"))
        self.assertIn("freq:latin:0:0:direction:1", features("пдф", "gla", 1, "я отправил "))

    def test_a_lexicon_word_is_told_by_how_common_it_is_in_russian_text(self) -> None:
        found = features("гр", "uh", 1, after="why", source_known=True, target_known=True)
        self.assertIn("freq:cyrillic:lexicon:1:direction:1", found)
        self.assertIn("freq:english:1:russian:1:direction:1", found)
        self.assertIn("freq:english:1:russian:1:en:direction:1", found)
        self.assertIn("freq:english:1:russian:1:line:empty:direction:1", found)
        self.assertIn("freq:cyrillic:lexicon:4:direction:0", features("yf", "на", 0, "я ", target_known=True))

    def test_letters_typed_into_a_word_are_not_told_by_the_language_tables(self) -> None:
        found = features("руку", "here", 1, "to bathe ", source_known=True, target_known=True, inside=True)
        self.assertIn("inside:freq:cyrillic:lexicon:direction:1", found)
        self.assertFalse(names("freq:", found))

    def test_a_word_alone_in_its_field_is_not_told_by_the_language_tables(self) -> None:
        found = features("шы", "is", 1, source_known=True, target_known=True)
        self.assertIn("freq:cyrillic:lexicon:direction:1", found)
        self.assertFalse(names("freq:english:", found) | names("freq:russian:", found))
        self.assertTrue(names("freq:english:", features("шы", "is", 1, after="it", source_known=True, target_known=True)))


class SignTokenTests(unittest.TestCase):
    def test_a_token_of_signs_in_both_readings(self) -> None:
        found = features("=/", "=|", 1, "эти строки мне нужны ")
        self.assertIn("signs:only:direction:1", found)
        self.assertIn("signs:only:digits:0:direction:1", found)
        self.assertIn("signs:only:digits:1:direction:0", features("1/", "1.", 0))

    def test_a_token_of_signs_whose_other_reading_has_letters(self) -> None:
        found = features(":/", "Ж.", 0)
        self.assertIn("signs:letters:direction:0", found)
        self.assertIn("signs:letters:digits:0:direction:0", found)

    def test_a_token_with_letters_has_none_of_them(self) -> None:
        self.assertFalse(names("signs:", features("[jhjij", "хорошо", 0)))


class TermFrequencyTableTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.path = Path(self.temporary.name) / "table.json"

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def write(self, payload: object) -> Path:
        self.path.write_text(json.dumps(payload), encoding="utf-8")
        return self.path

    def test_the_bundled_table_has_every_part(self) -> None:
        self.assertEqual(sorted(load_term_frequency(TERM_FREQUENCY_PATH)), list(TERM_ALPHABETS))

    def test_a_malformed_table_is_refused(self) -> None:
        empty: dict[str, object] = {name: {} for name in TERM_ALPHABETS}
        malformed: tuple[object, ...] = ([], {"latin": {}, "cyrillic": {}}, {**empty, "cyrillic": []},
                                         {**empty, "latin": {"id": 0}}, {**empty, "latin": {"id": True}})
        for payload in malformed:
            with self.subTest(payload=payload), self.assertRaises(ValueError):
                load_term_frequency(self.write(payload))

    def test_an_oversized_table_is_refused(self) -> None:
        with patch.object(context_model, "MAX_CONTEXT_TERM_FREQUENCY_BYTES", 1), self.assertRaises(ValueError):
            load_term_frequency(self.write({name: {} for name in TERM_ALPHABETS}))

    def test_a_schema_seven_model_without_its_table_does_not_load(self) -> None:
        missing = Path(self.temporary.name) / "missing.json"
        with patch.object(context_model, "_TERM_FREQUENCY", None), patch.object(context_model, "TERM_FREQUENCY_PATH", missing):
            model, reason = ContextModel.try_load()
        self.assertIsNone(model)
        self.assertIn(missing.name, reason)


class SchemaSixTests(unittest.TestCase):
    def test_schema_six_has_none_of_them(self) -> None:
        item = ContextEvidence("ЦЗА", "WPF", 1, FieldContext("", "1", "text\n", "", "text"))
        found = extract_context_features(item, CONTEXT_OPENING_FEATURE_VERSION)
        for prefix in ("line:", "case:", "latin:", "freq:", "signs:"):
            self.assertFalse(names(prefix, found), prefix)


if __name__ == "__main__":
    unittest.main()
