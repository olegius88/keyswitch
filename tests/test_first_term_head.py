"""The first-term head (context_action_features.first_question) and its curriculum.

A term of up to three letters typed in the Russian layout alone in a field a reader read empty opens a message, and
converts there at once (`йк ` for `qr`), where the frozen model waited for a pause.
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
from keyswitch.constants.training import FIRST_PAUSE_WORD_START_MIN_BUCKET  # noqa: E402
from keyswitch.constants.model_protocol import CALIBRATION, DEVELOPMENT, TRAIN  # noqa: E402
from keyswitch import context_action_features as features  # noqa: E402
from keyswitch.context_action_features import extract_action_features, first_question, russian_start_bucket  # noqa: E402
from keyswitch.context_model import ContextEvidence, term_bucket  # noqa: E402
from keyswitch.input_context import FieldContext  # noqa: E402
import train_context_action_model as trainer  # noqa: E402
from fixture_values.clock import DEFAULT_SEQUENCE_EXTENDED_IDLE_SECONDS  # noqa: E402
from test_default_input_sequences import DefaultSequenceHarness, _CaretReader  # noqa: E402

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
        # The tail of a Russian word is a member alone, and none with the letters an edit leaves before it.
        self.assertTrue(first_question("дые", "", ""))
        self.assertFalse(first_question("дые", "где то д", ""))
        # The tables of Russian endings and beginnings the head reads are built once: built for every token, they read
        # the whole Russian table for each frame and each word typed.
        self.assertIs(features._russian_endings(), features._russian_endings())
        self.assertIs(features._russian_starts(), features._russian_starts())
        # How many Russian words begin with a token: `уче` begins 77 (bucket 7), the keys of `qr` none.
        self.assertEqual([russian_start_bucket(token) for token in ("уче", "йк", "сымю")], ["7", "0", "na"])

    def test_the_head_answers_a_field_read_empty_and_the_term_head_the_rest(self) -> None:
        read = heads(evidence("йк", "qr"))
        counts = f"latin:{term_bucket('qr', 'latin')}:english:{term_bucket('qr', 'english')}"
        self.assertTrue({"boundary:space", "boundary:space:letters:2", f"boundary:space:{counts}", "boundary:space:start:0",
                         "boundary:space:start:0:letters:2", "boundary:space:after:none",
                         f"boundary:space:after:none:{counts}"} <= read[FIRST_FEATURE_PREFIX])
        # Every piece is read with the boundary: what a pause teaches never reaches the space.
        self.assertTrue(all(name.startswith("boundary:space") for name in read[FIRST_FEATURE_PREFIX]))
        # Asked again with the word after it, as the engine asks a converted term once a Russian word follows.
        again = replace(evidence("йк", "qr"), field=FieldContext("Code", "test", "", "истек", "text", source="uia"),
                        after_origin="planned_next_conversion")
        self.assertIn("boundary:space:after:planned_next_conversion:letters:2", heads(again)[FIRST_FEATURE_PREFIX])
        self.assertEqual(read[TERM_FEATURE_PREFIX], set())
        # Nobody read the field: the text before the token is unknown, and the term head answers.
        unread = heads(evidence("йк", "qr", role="unknown"))
        self.assertEqual(unread[FIRST_FEATURE_PREFIX], set())
        self.assertTrue(unread[TERM_FEATURE_PREFIX])
        # The tail of a Russian word alone, at a pause: the same evidence as any term of the class.
        self.assertIn("boundary:end:letters:2", heads(evidence("ев", "td", trigger="pause"))[FIRST_FEATURE_PREFIX])
        # A word the profile's lexicon knows stays with the frozen model.
        known = heads(replace(evidence("шву", "ide"), source_known=True))
        self.assertEqual(known, {FIRST_FEATURE_PREFIX: set(), TERM_FEATURE_PREFIX: set()})

    def test_the_head_is_fitted_on_its_own_curriculum_alone(self) -> None:
        # A frame of the class another curriculum framed leaves TRAIN (frame_chain); unread, it is the term head's.
        field = FieldContext("Code", "test", "", "", "text", source="uia")
        other = trainer.ActionRow("legacy-train:щт", "щт", 1, field, "space", "", "wait", "legacy", " ", 1.0)
        self.assertTrue(trainer.outside_first_curriculum(other))
        self.assertFalse(trainer.outside_first_curriculum(replace(other, identifier="first:щт:space")))
        self.assertFalse(trainer.outside_first_curriculum(replace(other, field=replace(field, role="unknown"))))
        self.assertFalse(trainer.outside_first_curriculum(replace(other, field=replace(field, before="мой "))))

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
            kind = row.identifier.rsplit(":", 1)[1]
            # At the pause a token many Russian words begin with may be a word stopped in the middle: kept.
            started = int(russian_start_bucket(row.original)) >= FIRST_PAUSE_WORD_START_MIN_BUCKET
            self.assertEqual(row.action, "keep" if kind == "pause" and started else "convert", row.identifier)
            # Asked again with a Russian word after it, as the engine asks a converted term: the baseline the engine
            # gives the revisit.
            self.assertEqual((bool(row.field.after), row.revisit), (kind == "next", kind == "next"))
            if kind == "next":
                self.assertEqual(row.after_origin, "planned_next_conversion" if len(row.original) <= len("йку") else "field")
        # The tails of Russian words among them (`ев`, `ые`) convert as every term of the class does, and some tokens
        # are kept at the pause.
        self.assertTrue({"ев", "ые"} & {row.original for row in rows})
        self.assertEqual({row.action for row in rows if row.identifier.endswith(":pause")}, {"keep", "convert"})
        self.assertEqual(trainer.first_curriculum(TRAIN, [], frozenset(), trainer.NO_FIRST_CURRICULUM), ([], {"frames": 0, "scope": "not used"}))
        self.assertEqual(trainer.first_curriculum(CALIBRATION, [], frozenset(), FIRST)[0], [])


class FirstTermEngineTests(DefaultSequenceHarness):
    """The installed pair answers the class in the engine. Not a module the pair's seal pins: the pair it replaces
    reads these tokens otherwise, and the seal's modules pass with both."""

    def test_a_short_term_opening_a_message_in_a_field_read_empty_converts_at_its_space(self) -> None:
        # `шь` and `уче`, which many Russian words end with, too: the tail of a Russian word never stands there alone.
        for typed, meant in (("йк", "qr"), ("шь", "im"), ("уче", "ext")):
            with self.subTest(typed=typed), self.session() as session:
                session.engine.context_policy.reader = _CaretReader(session.backend)
                session.type(typed + " ", 1, idle_after_words=False)
                self.assertEqual(session.backend.text, meant + " ")

    def test_a_pause_after_letters_many_russian_words_begin_with_is_a_pause_inside_a_word(self) -> None:
        # `уче`, a pause, `ба` stays `учеба`: the pause in the middle of a word that opens a message is no boundary.
        with self.session() as session:
            session.engine.context_policy.reader = _CaretReader(session.backend)
            session.type("уче", 1, idle_after_words=False)
            session.idle(DEFAULT_SEQUENCE_EXTENDED_IDLE_SECONDS)
            session.type("ба ", 1, idle_after_words=False)
            self.assertEqual(session.backend.text, "учеба ")


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    return unittest.TestSuite(loader.loadTestsFromTestCase(case) for case in (FirstTermHeadTests, FirstTermEngineTests))


if __name__ == "__main__":
    unittest.main()

