"""The slip head (`cgtie` for `спешу` after `я `: a word of Russian prose typed with the English layout on, which slipped
there with no language boundary): its class and evidence, the slip typing mode of the capture and its frames."""

from __future__ import annotations

import sys
import unittest
from dataclasses import replace
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

import mixed_typing  # noqa: E402
import select_capture_messages as selection  # noqa: E402
import train_context_action_model as trainer  # noqa: E402
from fixture_values.scores import SLIP_FIXTURE_ORTHO, SLIP_FIXTURE_PROBABILITY  # noqa: E402
from keyswitch.constants.models import SLIP_FEATURE_PREFIX  # noqa: E402
from keyswitch.context_action_features import extract_action_features, slip_question  # noqa: E402
from test_context_training_captured import captured_question  # noqa: E402
from test_variable_and_dotted_heads import evidence, prefixes  # noqa: E402


class SlipClassTests(unittest.TestCase):
    def test_the_class_is_a_rare_latin_token_whose_keys_spell_russian_prose_after_a_russian_line(self) -> None:
        for original, before in (("cgtie", "я "), ("htity", "вопрос "), ("pfdsk", "волк "), ("Cgtie", "Я ")):
            with self.subTest(original=original):
                self.assertTrue(slip_question(original, before))
        # A term Russian technical text writes in Latin letters (`gtk`), an English word (`it`), a reading no Russian
        # prose uses (`hello` reads `руддщ`), English text before or a Russian line above an English one, fewer than four
        # letters (`ibp` reads `шиз`, `ck` `сл`), signs or digits in the token.
        for original, before in (("gtk", "ты "), ("it", "помой "), ("hello", "привет "), ("cgtie", "hello "),
                                 ("cgtie", "я спешу\nhello "), ("ibp", "том "), ("ck", "я "), ("cgtie/", "я "), ("x64", "я ")):
            with self.subTest(original=original, before=before):
                self.assertFalse(slip_question(original, before))

    def test_the_head_reads_counts_neighbours_the_detector_and_the_word_before_but_no_letter_or_boundary(self) -> None:
        ortho_score, ortho_threshold = SLIP_FIXTURE_ORTHO
        typed = extract_action_features(replace(evidence("cgtie", "спешу", 0, "я "), target_known=True, baseline_convert=True,
                                                model_probability=SLIP_FIXTURE_PROBABILITY, ortho_score=ortho_score,
                                                ortho_threshold=ortho_threshold))
        self.assertEqual(prefixes(typed), {SLIP_FEATURE_PREFIX})
        for name in ("bias", "known:1", "detector:1:9", "ortho:-2", "case:lower", "after:none"):
            self.assertIn(SLIP_FEATURE_PREFIX + name, typed)
        self.assertTrue(any(name.startswith(SLIP_FEATURE_PREFIX + "before:") for name in typed))
        own = [name for name in typed if name.startswith(SLIP_FEATURE_PREFIX)]
        self.assertFalse(any("cgtie" in name or "спешу" in name or "boundary" in name or "trigger" in name for name in own))
        # Without the detector's probability or the orthotactic model the bands read `na`.
        plain = extract_action_features(evidence("cgtie", "спешу", 0, "я "))
        self.assertIn(SLIP_FEATURE_PREFIX + "detector:0:na", plain)
        self.assertIn(SLIP_FEATURE_PREFIX + "ortho:na", plain)
        # A word the English lexicon knows or an identifier stays with the frozen model.
        for item in (replace(evidence("cgtie", "спешу", 0, "я "), source_known=True),
                     replace(evidence("cgtie", "спешу", 0, "я "), source_identifier=True)):
            with self.subTest(item=item):
                self.assertNotIn(SLIP_FEATURE_PREFIX + "bias", extract_action_features(item))

    def test_the_head_is_fitted_onto_a_frozen_base_with_an_epoch_of_its_own(self) -> None:
        self.assertEqual(trainer.HEAD_PREFIXES["slip"], SLIP_FEATURE_PREFIX)
        self.assertIn(SLIP_FEATURE_PREFIX, trainer.OWN_EPOCH_HEADS)
        heads = frozenset({SLIP_FEATURE_PREFIX})
        self.assertTrue(trainer.head_question("cgtie", "спешу", "я ", "", "space", "none", heads))
        self.assertFalse(trainer.head_question("cgtie", "спешу", "я ", "", "space", "none", heads, "planned_previous_reading"))
        self.assertFalse(trainer.head_question("gtk", "пел", "ты ", "", "space", "none", heads))
        try:
            import context_action_cuda as cuda
        except ImportError:  # CuPy missing, as in CI: the GPU path does not load here
            self.skipTest("CuPy is required to load the GPU path")
        self.assertTrue(cuda.heads_question("cgtie", "я ", "", "space"))


class SlipTypingTests(unittest.TestCase):
    def test_a_word_after_another_russian_word_slips_into_the_english_layout_unseen(self) -> None:
        start, steps = mixed_typing.plan("я сегодня спешу домой".split(), mixed_typing.SLIP_MODE, 1) or (0, [])
        self.assertEqual(start, 1)
        self.assertEqual(steps, [("я", 1, None, False), ("сегодня", 1, None, False), ("спешу", 1, 0, True), ("домой", 1, None, False)])
        # No two Russian words in a row: nothing to slip.
        self.assertIsNone(mixed_typing.plan("я use это".split(), mixed_typing.SLIP_MODE, 0))
        self.assertIsNone(mixed_typing.plan("Hello world".split(), mixed_typing.SLIP_MODE, 0))
        # The other modes change the layout by hand only.
        _start, correct = mixed_typing.plan("я use это".split(), "correct", 0) or (0, [])
        self.assertFalse(any(slipped for *_rest, slipped in correct))

    def test_a_capture_keeps_the_questions_of_the_class(self) -> None:
        self.assertTrue(mixed_typing.CAPTURED_QUESTIONS["slip"](captured_question("cgtie", "спешу", "я ")))
        self.assertFalse(mixed_typing.CAPTURED_QUESTIONS["slip"](
            captured_question("cgtie", "спешу", "я ", before_origin="planned_previous_reading")))
        self.assertFalse(mixed_typing.CAPTURED_QUESTIONS["slip"](captured_question("gtk", "пел", "ты ")))

    def test_messages_hold_two_russian_words_in_a_row(self) -> None:
        self.assertTrue(selection.slippable("я спешу домой"))
        self.assertTrue(selection.slippable("используй git, а потом спешу"))
        self.assertFalse(selection.slippable("я use это"))
        self.assertFalse(selection.slippable("hello world"))
        self.assertIn("slippable", selection.MESSAGE_CLASSES)


if __name__ == "__main__":
    unittest.main()
