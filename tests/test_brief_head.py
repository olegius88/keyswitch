"""The short-message head (context_action_features.brief_question) and its curriculum.

A whole message typed in the English layout stayed: `ytn` for `нет` was kept at p 0.94, `jr` for `ок` and `tot` for `еще`
waited for a next word that never came.
"""

from __future__ import annotations

import math
import sys
import unittest
from collections import defaultdict
from pathlib import Path
from typing import cast

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

from keyswitch.constants.models import BRIEF_FEATURE_PREFIX, BRIEF_HEAD_LETTERS, BRIEF_RATIO_BOUND  # noqa: E402
from keyswitch.constants.model_protocol import CALIBRATION, DEVELOPMENT, TRAIN  # noqa: E402
from keyswitch.constants.training import BRIEF_BOUNDARIES  # noqa: E402
from keyswitch.context_action_features import brief_question, extract_action_features  # noqa: E402
from keyswitch.context_model import ContextEvidence  # noqa: E402
from keyswitch.input_context import FieldContext  # noqa: E402
import train_context_action_model as trainer  # noqa: E402
from fixture_values.counts import BRIEF_FIXTURE_MINIMUM_COUNT  # noqa: E402

BRIEF = {"minimum_count": BRIEF_FIXTURE_MINIMUM_COUNT, "held_out": ["ytn"], "sample_weight": 1.0}


def head(original: str, alternative: str, before: str = "", after: str = "", trigger: str = "enter") -> set[str]:
    item = ContextEvidence(original, alternative, 0, FieldContext("Telegram", "1", before, after), trigger, boundary_text="\n")
    return {name.removeprefix(BRIEF_FEATURE_PREFIX) for name in extract_action_features(item) if name.startswith(BRIEF_FEATURE_PREFIX)}


class BriefHeadTests(unittest.TestCase):
    def test_the_question_is_a_short_latin_token_alone_in_the_field(self) -> None:
        for token in ("ytn", "jr", "tot", "Ytn", "ok", "in"):
            self.assertTrue(brief_question(token, "", ""), token)
        # A Russian token, a longer one, one with words around it or a sign: no member.
        for token, before, after in (("нет", "", ""), ("ytns", "", ""), ("ytn", "мы ", ""), ("ytn", "", "yflj"),
                                     ("j,", "", ""), ("y", "", "")):
            self.assertFalse(brief_question(token, before, after), (token, before, after))

    def test_the_head_reads_how_much_more_often_one_reading_occurs(self) -> None:
        reply = head("ytn", "нет")
        self.assertIn("trigger:enter", reply)
        self.assertIn("case:lower", reply)
        # `нет` is a word of Russian prose and `ytn` no word at all: the ratio is at its bound, and only the Russian
        # reading is seen. `ok` is the other way round.
        self.assertIn(f"ratio:{BRIEF_RATIO_BOUND}", reply)
        self.assertIn("seen:1:0", reply)
        english = head("ok", "щл")
        self.assertIn(f"ratio:{-BRIEF_RATIO_BOUND}", english)
        self.assertIn("seen:0:1", english)
        # The counts on their own are no evidence: the head learnt the tokens by heart from them.
        self.assertFalse([name for name in reply if name.startswith(("russian:", "english:", "latin:"))])
        self.assertIn("case:title", head("Ytn", "Нет"))
        self.assertEqual(head("ytn", "нет", before="ну "), set())

    def test_the_trainer_routes_the_class_to_the_head(self) -> None:
        prefixes = frozenset({BRIEF_FEATURE_PREFIX})
        self.assertTrue(trainer.head_question("ytn", "нет", "", "", "enter", "none", prefixes))
        self.assertFalse(trainer.head_question("ytn", "нет", "", "", "enter", "none", frozenset()))
        self.assertIn(BRIEF_FEATURE_PREFIX, trainer.OWN_EPOCH_HEADS)

    def test_the_curriculum_frames_counted_words_alone_both_ways_by_their_counts(self) -> None:
        train, report = trainer.brief_curriculum(TRAIN, [], frozenset(), BRIEF)
        development, development_report = trainer.brief_curriculum(DEVELOPMENT, [], frozenset(), BRIEF)
        self.assertTrue(train and development)
        families = {split: {row.identifier.split(":")[1] for row in rows} for split, rows in ((TRAIN, train), (DEVELOPMENT, development))}
        self.assertFalse(families[TRAIN] & families[DEVELOPMENT])
        self.assertTrue(all(brief_question(row.original, row.field.before, "") and not row.field.before for row in train))
        self.assertTrue(all(len(row.original) in BRIEF_HEAD_LETTERS and row.group == 0 for row in train))
        # Every token at every boundary, in two cases; `ytn` is held out.
        self.assertEqual({(row.trigger, row.boundary_text) for row in train}, set(BRIEF_BOUNDARIES))
        self.assertNotIn("ytn", families[TRAIN] | families[DEVELOPMENT])
        self.assertTrue(sum(cast(dict[str, int], value["tokens"]).get("refused", 0) for value in (report, development_report)))
        # A token that is both a Russian word's keys and a Latin word is framed both ways, its weights summing to one frame.
        mass: defaultdict[str, float] = defaultdict(float)
        actions: dict[str, set[str]] = {}
        for row in train + development:
            mass[row.identifier.rsplit(":", 1)[0]] += row.sample_weight
            actions.setdefault(row.identifier.split(":")[1], set()).add(row.action)
        self.assertTrue(all(math.isclose(value, 1.0) for value in mass.values()))
        self.assertTrue(any(found == {"keep", "convert"} for found in actions.values()))
        self.assertEqual(trainer.brief_curriculum(TRAIN, [], frozenset(), trainer.NO_BRIEF_CURRICULUM), ([], {"frames": 0, "scope": "not used"}))
        self.assertEqual(trainer.brief_curriculum(CALIBRATION, [], frozenset(), BRIEF)[0], [])


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    return loader.loadTestsFromTestCase(BriefHeadTests)


if __name__ == "__main__":
    unittest.main()
