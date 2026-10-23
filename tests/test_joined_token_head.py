"""The joined-token head (context_action_features.joined_question) and its curriculum.

Sealed test v58: `Re-re!` after a Russian sentence, meant as `Ку-ку!`, stayed with both pairs. No term table counts a token
of words joined by hyphens, so the shared features read no count of it at all, while its words tell the language: `ку`
is counted in Russian prose and `re` nowhere in English.
"""

from __future__ import annotations

import sys
import unittest
from dataclasses import replace
from typing import cast
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

from keyswitch.constants.keyboard import LAYOUT_GROUP_COUNT  # noqa: E402
from keyswitch.constants.models import JOINED_FEATURE_PREFIX  # noqa: E402
from keyswitch.constants.model_protocol import DEVELOPMENT, TRAIN  # noqa: E402
from keyswitch.context_action_features import extract_action_features, joined_parts, joined_question, own_bucket  # noqa: E402
from keyswitch.context_model import ContextEvidence, term_bucket  # noqa: E402
from keyswitch.input_context import FieldContext  # noqa: E402
import train_context_action_model as trainer  # noqa: E402
from test_context_action_training import fixture  # noqa: E402

JOINED = {"item_modulus": 1, "per_token": 1, "held_out": ["ну-ну"], "sample_weight": 1.0}


def evidence(original: str, alternative: str, group: int, before: str) -> ContextEvidence:
    return ContextEvidence(original, alternative, group, FieldContext("Code", "test", before, "", "unknown"), "space",
                           boundary_text=" ")


def head(item: ContextEvidence) -> set[str]:
    return {name.removeprefix(JOINED_FEATURE_PREFIX) for name in extract_action_features(item)
            if name.startswith(JOINED_FEATURE_PREFIX)}


class JoinedTokenHeadTests(unittest.TestCase):
    def test_the_question_reads_either_reading_of_the_typed_token(self) -> None:
        self.assertEqual(joined_parts("Re-re!"), ["Re", "re"])
        self.assertTrue(joined_question("Re-re"))
        self.assertTrue(joined_question("pre-order"))
        # The keys of `обычно-так` type a comma in the English layout: the other reading is the class.
        self.assertIsNone(joined_parts("j,sxyj-nfr"))
        self.assertTrue(joined_question("j,sxyj-nfr"))
        # A sign at the edge is no joint, and digits are no words.
        self.assertFalse(joined_question("-PHP,"))
        self.assertFalse(joined_question("webjs"))
        self.assertFalse(joined_question("1-2"))
        self.assertFalse(joined_question("--"))

    def test_the_shared_features_read_no_count_of_a_joined_token(self) -> None:
        # The frozen model's weights for single words, fed the counts of its words, turned `bye-bye` typed as meant into
        # `ину-ину`: the counts are the head's evidence alone.
        item = evidence("Re-re", "Ку-ку", 0, "Он ушёл. ")
        self.assertEqual(own_bucket(item, "Ку-ку", "russian"), "na")
        self.assertFalse([name for name in extract_action_features(item) if name.startswith("freq:")])

    def test_the_head_copies_the_shared_features_and_reads_its_words(self) -> None:
        english = f"english:most:{term_bucket('re', 'english')}:least:{term_bucket('re', 'english')}"
        russian = f"russian:most:{term_bucket('ку', 'russian')}:least:{term_bucket('ку', 'russian')}"
        self.assertTrue({"bias", f"{english}:direction:0", f"{russian}:direction:0", "repeat:1:direction:0",
                         f"repeat:1:{english}:{russian}:direction:0"} <= head(evidence("Re-re", "Ку-ку", 0, "Он ушёл. ")))
        words = sorted((term_bucket("pre", "english"), term_bucket("order", "english")), key=int)
        self.assertIn(f"english:most:{words[-1]}:least:{words[0]}:direction:0",
                      head(evidence("pre-order", "зку-щквук", 0, "I placed a ")))
        # The keys of `обычно-так` are no words joined by hyphens: their counts are none.
        self.assertIn("english:most:na:least:na:direction:0", head(evidence("j,sxyj-nfr", "обычно-так", 0, "Он сказал ")))
        self.assertIn("repeat:0:direction:1", head(evidence("что-то", "xnj-nj", 1, "Он сказал ")))
        dropped = head(replace(evidence("Re-re", "Ку-ку", 0, "Он ушёл. "), term_counts_dropped=True))
        self.assertIn("russian:most:0:least:0:direction:0", dropped)
        self.assertEqual(head(evidence("webjs", "цуиоы", 0, "только ")), set())

    def test_the_trainer_routes_the_class_to_the_head(self) -> None:
        prefixes = frozenset({JOINED_FEATURE_PREFIX})
        self.assertTrue(trainer.head_question("Re-re", "Ку-ку", "Он ушёл. ", "", "space", "none", prefixes))
        self.assertFalse(trainer.head_question("Re-re", "Ку-ку", "Он ушёл. ", "", "space", "none", frozenset()))

    def test_the_gpu_path_routes_the_class_to_the_head(self) -> None:
        try:
            import context_action_cuda as cuda
        except ImportError:  # CuPy missing, as in CI: the GPU path does not load here
            self.skipTest("CuPy is required to load the GPU path")
        self.assertTrue(cuda.heads_question("j,sxyj-nfr", "Он сказал ", "", "space"))
        # A plain word typed in the other layout after English text (after Russian text it is the slip head's).
        self.assertFalse(cuda.heads_question("ghbdtn", "He said ", "", "space"))

    def test_the_curriculum_frames_each_occurrence_once_in_both_layouts(self) -> None:
        sentence = replace(fixture("j1", "ушёл", 1), before="Он ", after=". Ку-ку! Ну-ну, что-то и pre-order")
        # A second row of the same sentence holds the same occurrences.
        again = replace(sentence, identifier="j2", original="Он", before="", after=" ушёл. Ку-ку! Ну-ну, что-то и pre-order")
        rows, report = trainer.joined_token_curriculum(TRAIN, [sentence, again], frozenset(), JOINED)
        framed = {(row.original, row.group, row.action) for row in rows}
        self.assertEqual(framed, {("Ку-ку", 1, "keep"), ("Re-re", 0, "convert"), ("что-то", 1, "keep"),
                                  ("xnj-nj", 0, "convert"), ("pre-order", 0, "keep"), ("зку-щквук", 1, "convert")})
        # `ну-ну` is held out; one occurrence of a token at most (per_token).
        tokens = {"ку-ку", "что-то", "pre-order"}
        self.assertEqual(cast(dict[str, int], report["items"])["refused"], 1)
        self.assertEqual(report["tokens"], len(tokens))
        self.assertTrue(all(row.field.before.endswith(" ") or not row.field.before for row in rows))
        self.assertTrue(all(joined_question(row.original) for row in rows))
        capped, capped_report = trainer.joined_token_curriculum(
            DEVELOPMENT, [sentence, replace(sentence, identifier="j3", document="other")], frozenset(), JOINED)
        self.assertEqual(cast(dict[str, int], capped_report["items"])["capped"], len(tokens))
        self.assertEqual(len(capped), len(rows))
        # A row whose text no layout types, and a token of two scripts, are no frames.
        unrepresentable = replace(sentence, identifier="j4", document="third", layout_representable=False)
        mixed = replace(fixture("j5", "сервиса", 1), before="поправь mongo-", after="", document="fourth")
        self.assertEqual(trainer.joined_token_curriculum(TRAIN, [unrepresentable, mixed], frozenset(), JOINED)[0], [])
        # One occurrence in `item_modulus` is drawn.
        sampled = trainer.joined_token_curriculum(TRAIN, [sentence], frozenset(),
                                                  {**JOINED, "item_modulus": len(tokens) + 1, "per_token": len(tokens)})[0]
        self.assertLess(len(sampled), len(rows))
        self.assertEqual(trainer.joined_token_curriculum(TRAIN, [sentence], frozenset(), trainer.NO_JOINED_TOKEN_CURRICULUM),
                         ([], {"frames": 0, "scope": "not used"}))
        self.assertEqual(trainer.joined_token_curriculum("calibration", [sentence], frozenset(), JOINED)[0], [])


    def test_the_curriculum_joins_counted_words_to_themselves_in_both_splits(self) -> None:
        russian = replace(fixture("r1", "ушёл", 1), before="Он вчера ", after="", document="ru")
        english = replace(fixture("e1", "left", 0), before="He has ", after="", document="en")
        options = {**JOINED, "repeat_words": len(JOINED), "repeat_minimum_count": 1}
        train = trainer.joined_token_curriculum(TRAIN, [russian, english], frozenset(), options)[0]
        development = trainer.joined_token_curriculum(DEVELOPMENT, [russian, english], frozenset(), options)[0]
        synthetic = [row for row in train + development if row.identifier.startswith("joined-repeat:")]
        # Each language's words go to one split each, and every token is typed as written and in the other layout.
        # Two languages, and each token in both layouts.
        self.assertEqual(len(synthetic), LAYOUT_GROUP_COUNT * LAYOUT_GROUP_COUNT * len(JOINED))
        for keep, convert in zip(synthetic[::LAYOUT_GROUP_COUNT], synthetic[1::LAYOUT_GROUP_COUNT]):
            words = {part.casefold() for part in keep.original.split("-")}
            self.assertEqual((len(words), keep.action, convert.action, convert.group), (1, "keep", "convert", 1 - keep.group))
            self.assertEqual(convert.original, trainer.translated(keep.original, keep.group))
            self.assertTrue(joined_question(convert.original))
        # A capital opens a sentence.
        self.assertTrue(all(row.field.before.endswith(". ") for row in synthetic if row.original[:1].isupper()))
        held_out = {**options, "held_out": [row.original.casefold() for row in synthetic]}
        refused = trainer.joined_token_curriculum(TRAIN, [russian, english], frozenset(), held_out)
        self.assertFalse([row for row in refused[0] if row.identifier.startswith("joined-repeat:")])
        self.assertTrue(cast(dict[str, int], refused[1]["items"])["refused"])
        # Without a context of its language a word is never framed.
        alone = trainer.joined_token_curriculum(TRAIN, [russian], frozenset(), options)[0]
        self.assertFalse([row for row in alone if row.identifier.startswith("joined-repeat:english:")])


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    return loader.loadTestsFromTestCase(JoinedTokenHeadTests)


if __name__ == "__main__":
    unittest.main()
