"""English terms inside Russian sentences of TRAIN, and the Russian word typed after them.

The owner's typing left two classes in the wrong layout more than any other: a term typed in the
Russian layout amid Russian prose, and the Russian word typed in the English layout right after the
term. The trainer builds both from real TRAIN sentences (train_context_action_model.term_insertion_curriculum).
"""

from __future__ import annotations

import unittest

from context_deferral import plausible_reading
from context_physical_keys import translated
from freeze_context_action_corpus import CorpusRow
from train_context_action_model import ActionRow, term_insertion_curriculum

OPTIONS = {"maximum_contexts": 10, "maximum_terms": 1500, "minimum_term_count": 100, "weight": 0.5,
           "dominance": 3, "maximum_abbreviation_contexts": 0}


def row(identifier: str, original: str, before: str, group: int = 1) -> CorpusRow:
    return CorpusRow(identifier, original, group, before, "", original, "family-" + identifier, "doc-" + identifier,
                     "ru" if group == 1 else "en", "fixture", "fixture.conllu", "1", "1", " ", "", "", "NOUN", "", "", "",
                     True, "train")


SENTENCE = row("d1:3", "сегодня", "мы обновили сервер и ")


class TermInsertionTests(unittest.TestCase):
    def test_no_budget_adds_nothing(self) -> None:
        frames, report = term_insertion_curriculum([SENTENCE], {**OPTIONS, "maximum_contexts": 0})
        self.assertEqual((frames, report), ([], {"maximum_contexts": 0}))

    def test_both_readings_of_the_term_and_of_the_word_after_it_are_labelled(self) -> None:
        frames, report = term_insertion_curriculum([SENTENCE], OPTIONS)
        by_category = {frame.category: frame for frame in frames}
        term = by_category["term_insertion"]
        wrong = by_category["term_insertion_layout_intervention"]
        self.assertEqual((term.group, term.action, wrong.group, wrong.action), (0, "keep", 1, "convert"))
        self.assertEqual(wrong.original, translated(term.original, 0))
        self.assertEqual(term.field.before, SENTENCE.before)
        after = by_category["word_after_term"]
        after_wrong = by_category["word_after_term_layout_intervention"]
        self.assertEqual((after.original, after.action), ("сегодня", "keep"))
        self.assertEqual((after_wrong.original, after_wrong.group, after_wrong.action), (translated("сегодня", 1), 0, "convert"))
        self.assertEqual(after.field.before, SENTENCE.before + term.original + " ")
        self.assertEqual({frame.sample_weight for frame in frames}, {0.5})
        self.assertEqual(report["counts"], {"term": 1, "after_term": 1})

    def test_a_term_whose_cyrillic_reading_is_a_word_is_never_inserted(self) -> None:
        frames, report = term_insertion_curriculum([SENTENCE], {**OPTIONS, "maximum_terms": 100000})
        self.assertGreater(cast_int(report["terms"]), 100)
        for frame in frames:
            if frame.category == "term_insertion_layout_intervention":
                self.assertFalse(plausible_reading(frame.original, 1))

    def test_the_term_list_stops_at_its_budget(self) -> None:
        _frames, report = term_insertion_curriculum([SENTENCE], {**OPTIONS, "maximum_terms": 2})
        self.assertEqual(report["terms"], 2)

    def test_a_russian_word_whose_latin_keys_spell_a_word_has_no_frame_after_the_term(self) -> None:
        # `руддщ` is `hello` typed in the Russian layout: after an English term that is English.
        frames, report = term_insertion_curriculum([row("d2:4", "руддщ", "надо будет сказать им ")], OPTIONS)
        self.assertEqual({frame.category for frame in frames}, {"term_insertion", "term_insertion_layout_intervention"})
        self.assertEqual(report["counts"], {"term": 1, "after_term_skipped_plausible_latin": 1})

    def test_only_russian_words_with_russian_words_before_them_are_contexts(self) -> None:
        rows = [row("d3:1", "сегодня", ""), row("d4:1", "today", "we have updated ", 0), row("d5:2", "и", "мы обновили ")]
        frames, report = term_insertion_curriculum(rows, OPTIONS)
        self.assertEqual((frames, report["contexts"]), ([], 0))

    def test_a_context_without_a_closing_space_gets_one_before_the_term(self) -> None:
        frames, _report = term_insertion_curriculum([row("d6:3", "сегодня", "мы обновили сервер,")], OPTIONS)
        self.assertTrue(all(frame.field.before.startswith("мы обновили сервер, ") for frame in frames))


class AbbreviationInsertionTests(unittest.TestCase):
    def frames(self) -> list[ActionRow]:
        rows = [SENTENCE, row("d7:2", "сегодня", "надо будет обновить ")]
        frames, _report = term_insertion_curriculum(rows, {**OPTIONS, "maximum_contexts": 1, "maximum_abbreviation_contexts": 5})
        return [frame for frame in frames if frame.category.startswith("abbreviation_insertion")]

    def test_a_russian_abbreviation_stays_and_its_latin_keys_convert(self) -> None:
        frames = self.frames()
        keep = [frame for frame in frames if frame.category == "abbreviation_insertion"]
        wrong = [frame for frame in frames if frame.category == "abbreviation_insertion_layout_intervention"]
        self.assertEqual(len(keep), 1)
        self.assertEqual((keep[0].group, keep[0].action, wrong[0].group, wrong[0].action), (1, "keep", 0, "convert"))
        self.assertEqual(wrong[0].original, translated(keep[0].original, 1))
        self.assertEqual(keep[0].field.before, "надо будет обновить ")

    def test_the_reading_russian_technical_text_uses_more_decides_the_label(self) -> None:
        # `тз` occurs 744 times in Russian technical text and its keys `np` 80 times: an abbreviation.
        # `бд` has a comma among its Latin keys and is no pair of words at all.
        _frames, report = term_insertion_curriculum([SENTENCE], {**OPTIONS, "maximum_abbreviation_contexts": 1})
        self.assertGreater(cast_int(report["abbreviations"]), 100)
        frames, _ = term_insertion_curriculum([row(f"d{i}:2", "сегодня", "надо будет обновить ") for i in range(400)],
                                              {**OPTIONS, "maximum_contexts": 0, "maximum_abbreviation_contexts": 400})
        self.assertEqual(frames, [])
        frames, _ = term_insertion_curriculum([row(f"d{i}:2", "сегодня", "надо будет обновить ") for i in range(400)],
                                              {**OPTIONS, "maximum_contexts": 1, "maximum_abbreviation_contexts": 399})
        inserted = {frame.original for frame in frames if frame.category == "abbreviation_insertion"}
        self.assertIn("тз", inserted)
        self.assertNotIn("бд", inserted)
        self.assertNotIn("зк", inserted)


def cast_int(value: object) -> int:
    assert isinstance(value, int)
    return value


if __name__ == "__main__":
    unittest.main()
