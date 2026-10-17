"""The term head (context_action_features.term_question) and its curriculum.

The frozen model left a term Russian technical text uses, typed in the Russian layout the first time a message held it,
below the conversion threshold: `сш` in `npm ci`, `зк` for `pr` and `рвьш` for `hdmi` stayed in the owner's typing.
"""

from __future__ import annotations

import sys
import unittest
from dataclasses import replace
from typing import cast
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

from keyswitch.constants.models import TERM_FEATURE_PREFIX, TERM_HEAD_MIN_LATIN_BUCKET  # noqa: E402
from keyswitch.constants.model_protocol import CALIBRATION, DEVELOPMENT, TRAIN  # noqa: E402
from keyswitch.constants.models import TERM_HEAD_ENDING_LETTERS  # noqa: E402
from keyswitch.constants.training import (  # noqa: E402
    TERM_ALONE_WAIT_MAX_LETTERS, TERM_CAPITALS_MAX_LETTERS,
)
from keyswitch.context_action_features import (  # noqa: E402
    alone_tail, extract_action_features, russian_ending_bucket, term_question,
)
from keyswitch.context_model import ContextEvidence, term_bucket  # noqa: E402
from keyswitch.input_context import FieldContext  # noqa: E402
import train_context_action_model as trainer  # noqa: E402
from reconcile_context_action_corpus import expanded_aliases  # noqa: E402
from test_context_action_training import fixture  # noqa: E402
from fixture_values.counts import TERM_FIXTURE_ITEM_MODULUS, TERM_FIXTURE_PER_PIECE  # noqa: E402

TERMS = {"item_modulus": TERM_FIXTURE_ITEM_MODULUS, "per_piece": TERM_FIXTURE_PER_PIECE, "held_out": [], "sample_weight": 1.0}


def evidence(original: str, alternative: str, before: str) -> ContextEvidence:
    return ContextEvidence(original, alternative, 1, FieldContext("Code", "test", before, "", "unknown"), "space",
                           boundary_text=" ")


def head(item: ContextEvidence) -> set[str]:
    return {name.removeprefix(TERM_FEATURE_PREFIX) for name in extract_action_features(item)
            if name.startswith(TERM_FEATURE_PREFIX)}


class TermHeadTests(unittest.TestCase):
    def test_the_question_reads_the_latin_reading_of_russian_letters(self) -> None:
        for typed in ("сш", "зк", "ЗК", "СЗГ", "рвьш", "сымю", "сымб", "юучу", "юучую", "Зфгыу"):
            self.assertTrue(term_question(typed, "мы сделали "), typed)
        # Latin letters, words Russian text uses, abbreviations Russian technical text counts, signs inside or at the
        # front of the Latin reading (`бещ` is `,to`), a single letter with a sign (`ую` is `e.`), digits, hyphens.
        for typed in ("ci", "ты", "привет", "кз", "ТЗ", "бещ", "эты", "ую", "зь32", "юоыщт-файлы", "с"):
            self.assertFalse(term_question(typed, "мы сделали "), typed)
        # A term typed right earlier in the text makes the token the unknown-token head's.
        self.assertTrue(term_question("мысщву", "открой проект в "))
        self.assertFalse(term_question("мысщву", "открыл vscode и в "))

    def test_the_head_copies_the_shared_features_and_reads_its_evidence(self) -> None:
        counts = f"latin:{term_bucket('csv', 'latin')}:english:{term_bucket('csv', 'english')}"
        self.assertGreaterEqual(int(term_bucket("csv", "latin")), TERM_HEAD_MIN_LATIN_BUCKET)
        signed = head(evidence("сымю", "csv.", "отправь файл "))
        self.assertTrue({"bias", counts, "signed:1", "extension:0", "ending:na:letters:3", "before:ru", f"{counts}:before:ru",
                         "signed:1:before:ru"} <= signed)
        # The letters with the boundary: alone at a space, two letters wait and more convert.
        self.assertIn("before:none:boundary:space:letters:2", head(evidence("гш", "ui", "")))
        # Alone in a field a reader read empty a short term is the first-term head's, not this head's.
        read = replace(evidence("йк", "qr", ""), field=FieldContext("Code", "test", "", "", "text", source="uia"))
        self.assertEqual(head(read), set())
        self.assertTrue(head(evidence("йк", "qr", "")))
        # No copy of the shared features: the frozen model reads the context, the head its own evidence alone.
        self.assertFalse(any(name.startswith(("source:", "target:", "before:char")) for name in signed))
        self.assertTrue({"signed:0", "extension:1", "before:none", "ending:na"} <= head(evidence("юучу", ".exe", "")))
        # How many Russian words end with a short token: the tail of a word an edit left ends many, a term's keys few.
        tail, term = russian_ending_bucket("ев"), russian_ending_bucket("зк")
        self.assertGreater(int(tail), int(term))
        self.assertIn(f"ending:{tail}", head(evidence("ев", "td", "у них ")))
        self.assertEqual(russian_ending_bucket("сымю"), "na")
        self.assertEqual(head(evidence("привет", "ghbdtn", "Он сказал ")), set())
        # A word the profile's lexicon knows stays with the frozen model (`шву`, a form of `шов`, whose keys spell `ide`), a
        # tail of Russian words it knows stays the head's, to keep (`ев`).
        self.assertTrue(term_question("шву", ""))
        self.assertTrue(head(evidence("шву", "ide", "")))
        self.assertEqual(head(replace(evidence("шву", "ide", ""), source_known=True)), set())
        self.assertTrue(head(replace(evidence("ев", "td", "у них "), source_known=True)))

    def test_the_trainer_and_the_gpu_path_route_the_class_to_the_head(self) -> None:
        prefixes = frozenset({TERM_FEATURE_PREFIX})
        self.assertTrue(trainer.head_question("зк", "pr", "сделай ", "", "space", "none", prefixes))
        self.assertFalse(trainer.head_question("зк", "pr", "сделай ", "", "space", "none", frozenset()))
        self.assertIn(TERM_FEATURE_PREFIX, trainer.OWN_EPOCH_HEADS)
        try:
            import context_action_cuda as cuda
        except ImportError:  # CuPy missing, as in CI: the GPU path does not load here
            self.skipTest("CuPy is required to load the GPU path")
        self.assertTrue(cuda.heads_question("рвьш", "у усилителя один ", "", "space"))

    def test_the_curriculum_frames_terms_in_the_russian_layout_and_keeps_pieces_of_words(self) -> None:
        # `постояльцев` ends in `ев`, the keys of `td`; `зк` is the keys of `pr`.
        word = replace(fixture("t1", "постояльцев", 1), before="Он принимал ")
        rows = [word, replace(word, identifier="t2", document="other"), replace(word, identifier="t3", document="third")]
        train, report = trainer.term_curriculum(TRAIN, rows, frozenset(), TERMS)
        development = trainer.term_curriculum(DEVELOPMENT, rows, frozenset(), TERMS)[0]
        converted = [row for row in train + development if row.action in ("convert", "wait")]
        self.assertTrue(converted)
        self.assertTrue(all(term_question(row.original, row.field.before) and row.group == 1 for row in train + development))
        # No term is framed in both splits, and each is framed twice: plainly, and in one variant.
        families = {split: {row.identifier.rsplit(":", 1)[0] for row in frames if row.action == "convert"}
                    for split, frames in ((TRAIN, train), (DEVELOPMENT, development))}
        self.assertFalse(families[TRAIN] & families[DEVELOPMENT])
        variants = {row.identifier.rsplit(":", 1)[0]: row for row in converted if row.identifier.endswith(":1")}
        for row in variants.values():
            self.assertTrue(not row.field.before or row.original[-1:] in "юб" or row.original[:1].isupper())
            if row.original.isupper() and len(row.original) > 1:
                self.assertLessEqual(len(row.original), TERM_CAPITALS_MAX_LETTERS)
            if row.original[:1].isupper() and not row.original.isupper():
                self.assertTrue(row.field.before.endswith(". "))
        # Alone at the start of a message and ended by a space, in a field nobody read, a term of two letters waits for
        # its next word, as the lone-word head leaves such a word: the engine asks a waiting word again at a pause, a
        # suggestion never. Any other converts at once.
        alone = [row for row in train + development if not row.field.before and row.trigger == "space"
                 and row.field.role == "unknown" and not row.identifier.endswith(":revisit")]
        self.assertEqual({row.action for row in alone if len(row.original) <= TERM_ALONE_WAIT_MAX_LETTERS}, {"wait"})
        self.assertEqual({row.action for row in alone if len(row.original) > TERM_ALONE_WAIT_MAX_LETTERS}, {"convert"})
        # Every short term stands once more alone at a space, to wait.
        short = [row for row in train + development if row.identifier.endswith(":alone")]
        self.assertTrue(short)
        self.assertTrue(all(len(row.original) <= TERM_ALONE_WAIT_MAX_LETTERS and row.action == "wait" for row in short))
        # Alone in a field a reader read empty a short term is the first-term head's: none of its frames is here.
        self.assertFalse([row for row in train + development if ":alone-read" in row.identifier])
        self.assertFalse([row for row in train + development if row.after_origin == "kept_next_word"])
        # Each term and each kept tail is also asked again as the engine asks a converted word whose next word went back:
        # a short Russian word after it, given as a planned conversion (a token of up to three letters) or as the field.
        revisits = [row for row in train + development if row.identifier.endswith(":revisit")]
        self.assertTrue(revisits)
        self.assertTrue(all(row.field.after and row.trigger == "space" and row.boundary_text == " " for row in revisits))
        self.assertEqual({row.after_origin for row in revisits if len(row.original) <= len("abc")}, {"planned_next_conversion"})
        self.assertEqual({row.after_origin for row in revisits if len(row.original) > len("abc")}, {"field"})
        self.assertEqual({row.action for row in revisits if row.identifier.startswith("term-piece:")}, {"keep"})
        self.assertEqual({row.action for row in revisits if row.identifier.startswith("term:")}, {"convert"})
        # Each with the baseline the engine gives it: its own boundary's verdict with no conversion (ActionRow.revisit).
        self.assertTrue(all(row.revisit for row in revisits))
        self.assertFalse(any(row.revisit for row in train + development if not row.identifier.endswith("revisit")))
        # The piece is kept at most `per_piece` times, after its row's left context.
        pieces = [row for row in train if row.action == "keep" and not row.identifier.endswith(":revisit")]
        self.assertEqual([row.original for row in pieces], ["ев"] * TERM_FIXTURE_PER_PIECE)
        self.assertTrue(all(row.field.before == "Он принимал " for row in pieces))
        self.assertIn(len(pieces[0].original), TERM_HEAD_ENDING_LETTERS)
        self.assertEqual(report["pieces"], 1)
        # A word opening its row gives no piece: alone in its field, a tail stands where a term does.
        opening = trainer.term_curriculum(TRAIN, [replace(word, identifier="t0", document="first", before=""), *rows],
                                          frozenset(), TERMS)
        self.assertFalse([row for row in opening[0] if row.identifier.startswith("term-piece:t0:")])
        self.assertEqual(cast(dict[str, int], opening[1]["items"])["skipped:row-start"], 1)
        # Held-out tokens and test aliases are refused in either reading.
        held = trainer.term_curriculum(TRAIN, rows, frozenset(), {**TERMS, "held_out": ["td"]})
        self.assertFalse([row for row in held[0] if row.action == "keep"])
        self.assertTrue(cast(dict[str, int], held[1]["items"])["refused"])
        refused = trainer.term_curriculum(TRAIN, rows, frozenset(expanded_aliases("ев")), TERMS)[0]
        self.assertFalse([row for row in refused if row.action == "keep"])
        first = converted[0]
        alias = trainer.term_curriculum(TRAIN if first in train else DEVELOPMENT, rows,
                                        frozenset(expanded_aliases(first.original)), TERMS)[0]
        self.assertNotIn(first.identifier, {row.identifier for row in alias})
        self.assertEqual(trainer.term_curriculum(TRAIN, rows, frozenset(), trainer.NO_TERM_CURRICULUM),
                         ([], {"frames": 0, "scope": "not used"}))
        self.assertEqual(trainer.term_curriculum(CALIBRATION, rows, frozenset(), TERMS)[0], [])
        # Without Russian contexts no term is framed.
        english = replace(fixture("e1", "left", 0), before="He has ", document="en")
        self.assertEqual(trainer.term_curriculum(TRAIN, [english], frozenset(), TERMS)[0], [])


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    return loader.loadTestsFromTestCase(TermHeadTests)


if __name__ == "__main__":
    unittest.main()
