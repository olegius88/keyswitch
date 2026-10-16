"""The first-term head (context_action_features.first_question) and its curriculum.

A term of up to three letters typed in the Russian layout alone in a field a reader read empty opens a message, and the
owner asked for `йк ` (`qr`) to convert there at once (16.10.2026), where the frozen model waited for a pause.
"""

from __future__ import annotations

import sys
import unittest
from collections import Counter
from dataclasses import replace
from pathlib import Path
from typing import cast

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

from keyswitch.constants.models import FIRST_FEATURE_PREFIX, TERM_FEATURE_PREFIX, TERM_HEAD_ENDING_LETTERS  # noqa: E402
from keyswitch.constants.model_protocol import CALIBRATION, DEVELOPMENT, TRAIN  # noqa: E402
from keyswitch.context_action_features import alone_tail, extract_action_features, first_question  # noqa: E402
from keyswitch.context_model import ContextEvidence, term_bucket  # noqa: E402
from keyswitch.input_context import FieldContext  # noqa: E402
import train_context_action_model as trainer  # noqa: E402

FIRST = {"enabled": True, "held_out": ["qr"], "sample_weight": 1.0}


def evidence(original: str, alternative: str, role: str = "text", before: str = "", trigger: str = "space") -> ContextEvidence:
    field = FieldContext("Code", "test", before, "", "text" if role == "text" else "unknown", source="uia")
    return ContextEvidence(original, alternative, 1, field, trigger, boundary_text=" " if trigger == "space" else "")


def heads(item: ContextEvidence) -> dict[str, set[str]]:
    features = extract_action_features(item)
    return {prefix: {name.removeprefix(prefix) for name in features if name.startswith(prefix)}
            for prefix in (FIRST_FEATURE_PREFIX, TERM_FEATURE_PREFIX)}


class FirstTermHeadTests(unittest.TestCase):
    def test_the_question_is_a_short_term_alone_in_the_field(self) -> None:
        for typed in ("йк", "зк", "сш", "ыйд", "ев"):
            self.assertTrue(first_question(typed, "", ""), typed)
        # Letters before or after it, a longer term, or a word Russian text uses: no member.
        for typed, before, after in (("йк", "мой ", ""), ("рвьш", "", ""), ("ты", "", "")):
            self.assertFalse(first_question(typed, before, after), (typed, before, after))
        # Signs before it, or the word the engine asks it again with after it.
        self.assertTrue(first_question("йк", "1. ", ""))
        self.assertTrue(first_question("йк", "", "истек"))
        # Whether a token may still be the tail of a Russian word: two letters from a higher ending bucket than three
        # (`йк` ends 11 words and may not, `ев` 169 and may, `дые` 7 and may); a longer token ends no word as a tail.
        self.assertEqual([alone_tail(token) for token in ("йк", "зк", "ев", "дые", "сымю")], [False, False, True, True, False])

    def test_the_head_answers_a_field_read_empty_and_the_term_head_the_rest(self) -> None:
        read = heads(evidence("йк", "qr"))
        counts = f"latin:{term_bucket('qr', 'latin')}:english:{term_bucket('qr', 'english')}"
        self.assertTrue({"bias", "tail:0", "tail:0:letters:2", "tail:0:boundary:space", "tail:0:boundary:space:letters:2",
                         f"tail:0:{counts}", f"tail:0:boundary:space:{counts}", "tail:0:after:none",
                         f"tail:0:after:none:{counts}"} <= read[FIRST_FEATURE_PREFIX])
        # Asked again with the word after it, as the engine asks a converted term once a Russian word follows.
        again = replace(evidence("йк", "qr"), field=FieldContext("Code", "test", "", "истек", "text", source="uia"),
                        after_origin="planned_next_conversion")
        self.assertIn("tail:0:after:planned_next_conversion:letters:2", heads(again)[FIRST_FEATURE_PREFIX])
        self.assertEqual(read[TERM_FEATURE_PREFIX], set())
        # Nobody read the field: the text before the token is unknown, and the term head answers.
        unread = heads(evidence("йк", "qr", role="unknown"))
        self.assertEqual(unread[FIRST_FEATURE_PREFIX], set())
        self.assertTrue(unread[TERM_FEATURE_PREFIX])
        # A token that may still be the tail of a Russian word, at a pause.
        self.assertIn("tail:1:boundary:end", heads(evidence("ев", "td", trigger="pause"))[FIRST_FEATURE_PREFIX])
        # A word the profile's lexicon knows stays with the frozen model.
        known = heads(replace(evidence("шву", "ide"), source_known=True))
        self.assertEqual(known, {FIRST_FEATURE_PREFIX: set(), TERM_FEATURE_PREFIX: set()})

    def test_the_trainer_and_the_gpu_path_route_the_class_to_the_head(self) -> None:
        prefixes = frozenset({FIRST_FEATURE_PREFIX})
        self.assertTrue(trainer.head_question("йк", "qr", "", "", "space", "none", prefixes))
        self.assertFalse(trainer.head_question("йк", "qr", "мой ", "", "space", "none", prefixes))
        self.assertIn(FIRST_FEATURE_PREFIX, trainer.OWN_EPOCH_HEADS)
        self.assertEqual(trainer.HEAD_PREFIXES["first"], FIRST_FEATURE_PREFIX)
        try:
            import context_action_cuda as cuda
        except ImportError:  # CuPy missing, as in CI: the GPU path does not load here
            self.skipTest("CuPy is required to load the GPU path")
        self.assertTrue(cuda.heads_question("йк", "", "", "space"))

    def test_the_curriculum_frames_each_short_term_alone_at_a_space_and_a_pause(self) -> None:
        train, report = trainer.first_curriculum(TRAIN, [], frozenset(), FIRST)
        development, development_report = trainer.first_curriculum(DEVELOPMENT, [], frozenset(), FIRST)
        self.assertTrue(train and development)
        tokens = {split: {row.original for row in rows} for split, rows in ((TRAIN, train), (DEVELOPMENT, development))}
        self.assertFalse(tokens[TRAIN] & tokens[DEVELOPMENT])
        self.assertNotIn("йк", tokens[TRAIN] | tokens[DEVELOPMENT])
        self.assertTrue(sum(cast(dict[str, int], value["items"]).get("refused", 0) for value in (report, development_report)))
        rows = train + development
        self.assertTrue(all(first_question(row.original, row.field.before, row.field.after) and row.field.role == "text"
                            and len(row.original) <= max(TERM_HEAD_ENDING_LETTERS) for row in rows))
        self.assertEqual(set(Counter(row.original for row in rows).values()), {len(("space", "pause", "next"))})
        for row in rows:
            tail = alone_tail(row.original)
            kind = row.identifier.rsplit(":", 1)[1]
            expected = {"space": "wait" if tail else "convert", "pause": "keep" if tail else "convert",
                        "next": "keep" if tail else "convert"}[kind]
            self.assertEqual(row.action, expected, row.identifier)
            # Asked again with a Russian word after it, as the engine asks it: a converted term with the baseline the
            # engine gives the revisit, a token that waited with its planned one.
            self.assertEqual((bool(row.field.after), row.revisit), (kind == "next", kind == "next" and not tail))
            if kind == "next":
                self.assertEqual(row.after_origin, "planned_next_conversion" if len(row.original) <= len("йку") else "field")
        self.assertTrue({True, False} <= {alone_tail(row.original) for row in rows})
        self.assertEqual(trainer.first_curriculum(TRAIN, [], frozenset(), trainer.NO_FIRST_CURRICULUM), ([], {"frames": 0, "scope": "not used"}))
        self.assertEqual(trainer.first_curriculum(CALIBRATION, [], frozenset(), FIRST)[0], [])


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    return loader.loadTestsFromTestCase(FirstTermHeadTests)


if __name__ == "__main__":
    unittest.main()
