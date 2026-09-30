"""The mixed-typing tool: a person's layout choices, drawn typos, edit points and recipes.

The captured questions the context model trains on are re-created from these draws
(`tools/mixed_typing.py capture --verify`), so a change here shows up first as a test.
"""

from __future__ import annotations

import gzip
import json
import tempfile
import unittest
from pathlib import Path

import mixed_typing as tool
from keyswitch.constants.corpus import MIXED_EDIT_WORD_MIN_LETTERS

from fixture_values.counts import MIXED_SPLIT_SAMPLE_IDS, MIXED_TYPING_TEST_MESSAGES
from fixture_values.scores import MIXED_TYPING_TEST_RUSSIAN_SHARE, MIXED_TYPING_TEST_TYPO_RATE

TOKENS = ["вот", "React", "hooks", "работают"]


def selections(mode: str) -> tuple[int, list[int | None]]:
    planned = tool.plan(TOKENS, mode, 0)
    assert planned is not None
    start, steps = planned
    return start, [selected for _token, _intended, selected in steps]


class PlanTests(unittest.TestCase):
    def test_every_change_of_language_is_selected_by_hand(self) -> None:
        self.assertEqual(selections("correct"), (1, [None, 0, None, 1]))

    def test_a_forgotten_change_is_not_selected(self) -> None:
        self.assertEqual(selections("forget_ru"), (1, [None, 0, None, None]))
        self.assertEqual(selections("forget_en"), (1, [None, None, None, 1]))

    def test_a_message_started_in_the_other_layout(self) -> None:
        self.assertEqual(selections("start_wrong"), (0, [None, 0, None, 1]))

    def test_a_mode_with_nothing_to_forget_does_not_apply(self) -> None:
        self.assertIsNone(tool.plan(["только", "русский"], "forget_en", 0))
        self.assertIsNone(tool.plan(["123", "..."], "correct", 0))


class DrawTests(unittest.TestCase):
    def test_typos_are_the_same_every_time(self) -> None:
        text = "проверь конфиг nginx перед деплоем в kubernetes"
        typed = tool.with_typos(text, MIXED_TYPING_TEST_TYPO_RATE, "fixture")
        self.assertEqual(typed, "проверь конфин nginx пеед деплоем в kubernetes")
        self.assertEqual(tool.with_typos(text, 0, "fixture"), text)

    def test_one_typo_per_word_and_none_in_short_words(self) -> None:
        self.assertEqual(tool.typo("конфигурация", "fixture:1"), "конигурация")
        self.assertEqual(tool.typo("deployment", "fixture:2"), "seployment")
        self.assertEqual(tool.typo("vm", "fixture:3"), "vm")

    def test_an_edit_point_lies_inside_a_long_word(self) -> None:
        text = "исправь конфигурацию сервера"
        point = tool.edit_point(text)
        assert point is not None
        start, length, group = point
        word_start = text.rfind(" ", 0, start) + 1
        word = text[word_start:].split(" ")[0]
        self.assertGreaterEqual(len(word), MIXED_EDIT_WORD_MIN_LETTERS)
        self.assertTrue(word_start < start and start + length < word_start + len(word))
        self.assertEqual(group, 1)
        self.assertIsNone(tool.edit_point("ok ну да"))

    def test_messages_fall_into_all_three_splits_by_identifier(self) -> None:
        splits = [tool.split_of("fixture", str(identifier)) for identifier in range(MIXED_SPLIT_SAMPLE_IDS)]
        self.assertEqual(set(splits), {"train", "dev", "test"})
        self.assertGreater(splits.count("train"), splits.count("dev") + splits.count("test"))
        self.assertEqual(splits, [tool.split_of("fixture", str(identifier)) for identifier in range(MIXED_SPLIT_SAMPLE_IDS)])


class RecipeTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.messages = Path(self.temporary.name) / "messages.jsonl.gz"
        rows = [("comment", "mixed", "запусти docker compose"), ("comment", "mixed", "обнови npm пакеты"),
                ("comment", "mixed", "почини ci pipeline"), ("comment", "russian", "всё работает"),
                ("comment", "russian", "спасибо большое"), ("title", "mixed", "ошибка в webpack")]
        with gzip.open(self.messages, "wt", encoding="utf-8") as handle:
            for index, (kind, mix, text) in enumerate(rows):
                handle.write(json.dumps({"qid": index, "kind": kind, "mix": mix, "text": text}, ensure_ascii=False) + "\n")

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def test_a_recipe_types_its_share_of_each_mix_in_every_mode(self) -> None:
        recipe: tool.Recipe = {"seed": "fixture", "kinds": ["comment"], "count": MIXED_TYPING_TEST_MESSAGES,
                               "russian_share": MIXED_TYPING_TEST_RUSSIAN_SHARE}
        jobs = tool.jobs_for(self.messages, recipe, None)
        texts = {text for _index, text, _mode, _choice, _prefix in jobs}
        self.assertEqual(len(texts), MIXED_TYPING_TEST_MESSAGES + int(MIXED_TYPING_TEST_MESSAGES * MIXED_TYPING_TEST_RUSSIAN_SHARE))
        self.assertNotIn("ошибка в webpack", texts)
        self.assertEqual([mode for _index, _text, mode, _choice, _prefix in jobs], list(tool.TYPING_MODES) * len(texts))
        self.assertEqual(jobs, tool.jobs_for(self.messages, recipe, None))

    def test_an_edit_recipe_has_one_job_per_message_under_a_previous_line(self) -> None:
        recipe: tool.Recipe = {"seed": "fixture", "kinds": ["comment"], "count": MIXED_TYPING_TEST_MESSAGES,
                               "russian_share": MIXED_TYPING_TEST_RUSSIAN_SHARE, "prefix_rate": 1, "edit": True}
        jobs = tool.jobs_for(self.messages, recipe, None)
        self.assertEqual({mode for _index, _text, mode, _choice, _prefix in jobs}, {tool.EDIT_MODE})
        messages = [text for _index, text, _mode, _choice, _prefix in jobs]
        # Without code lines, the line above a message is another message of the recipe (or, by the draw, itself).
        for _index, _text, _mode, _choice, prefix in jobs:
            self.assertIn(prefix[:-1], messages)
            self.assertTrue(prefix.endswith("\n"))


if __name__ == "__main__":
    unittest.main()
