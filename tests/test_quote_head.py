"""The quote head (`@Z` for `"Я`: a Russian message begun in the other layout with a quotation mark): its class and evidence,
its routing in the trainer and the capture's messages and frames."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

import mixed_typing  # noqa: E402
import select_capture_messages as selection  # noqa: E402
import train_context_action_model as trainer  # noqa: E402
from keyswitch.constants.models import QUOTE_FEATURE_PREFIX  # noqa: E402
from keyswitch.context_action_features import extract_action_features, quote_question  # noqa: E402
from test_context_training_captured import captured_question  # noqa: E402
from test_variable_and_dotted_heads import evidence, prefixes  # noqa: E402


class QuoteClassTests(unittest.TestCase):
    def test_the_class_is_one_sign_of_the_shared_key_and_letters_in_either_reading(self) -> None:
        for original, alternative, trigger in (("@Z", '"Я', "space"), ('"Я', "@Z", "space"), ("@media", '"ьувшф', "space"),
                                               ("@Ds", '"Вы', "punctuation")):
            with self.subTest(original=original):
                self.assertTrue(quote_question(original, alternative, trigger))
        # The sign alone, the same sign in both readings, a digit, letters of two scripts, and a pause.
        for original, alternative, trigger in (("@", '"', "space"), ("@Z", "@Я", "space"), ("@Z1", '"Я1', "space"),
                                               ("@ZЯ", '"ЯZ', "space"), ("@Z", '"Я', "pause")):
            with self.subTest(original=original, trigger=trigger):
                self.assertFalse(quote_question(original, alternative, trigger))

    def test_the_head_reads_the_shared_features_again_and_its_own_evidence(self) -> None:
        features = extract_action_features(evidence("@Z", '"Я', 0, "", "ctqxfc", "planned_next_conversion"))
        self.assertEqual(prefixes(features), {QUOTE_FEATURE_PREFIX})
        shared = {name for name in features if "|" not in name}
        self.assertTrue(all(QUOTE_FEATURE_PREFIX + name in features for name in shared if features[name]))
        for name in ("bias", "sign:@", "letters:1", "case:title", "start:1", "after:planned_next_conversion:en", "code:0",
                     "sign:@:start:1"):
            self.assertIn(f"{QUOTE_FEATURE_PREFIX}{name}:direction:0", features)
        for side in ("letters:english:", "reading:russian:"):
            self.assertTrue(any(name.startswith(QUOTE_FEATURE_PREFIX + side) for name in features), side)
        code = extract_action_features(evidence("@media", '"ьувшф', 0, "} { ", "(max-width"))
        for name in ("letters:4", "case:lower", "start:1", "code:1"):
            self.assertIn(f"{QUOTE_FEATURE_PREFIX}{name}:direction:0", code)

    def test_the_head_is_fitted_onto_a_frozen_base_with_an_epoch_of_its_own(self) -> None:
        self.assertEqual(trainer.HEAD_PREFIXES["quote"], QUOTE_FEATURE_PREFIX)
        self.assertIn(QUOTE_FEATURE_PREFIX, trainer.OWN_EPOCH_HEADS)
        heads = frozenset({QUOTE_FEATURE_PREFIX})
        self.assertTrue(trainer.head_question("@Z", '"Я', "", "", "space", "none", heads))
        self.assertFalse(trainer.head_question("@Z", '"Я', "", "", "pause", "none", heads))
        try:
            import context_action_cuda as cuda
        except ImportError:  # CuPy missing, as in CI: the GPU path does not load here
            self.skipTest("CuPy is required to load the GPU path")
        self.assertTrue(cuda.heads_question("@Z", "", "", "space"))
        self.assertTrue(cuda.heads_question('"Я', "", "", "space"))


class QuoteCaptureTests(unittest.TestCase):
    def test_a_capture_keeps_the_questions_of_the_class(self) -> None:
        question = mixed_typing.CAPTURED_QUESTIONS["quote"]
        self.assertTrue(question(captured_question("@Z", '"Я', "")))
        self.assertFalse(question(captured_question("@Z", '"Я', "", trigger="pause")))

    def test_messages_open_with_a_quotation_mark_and_a_russian_word(self) -> None:
        self.assertTrue(selection.quote_opener('"Я сейчас не могу". - "А когда сможешь?"'))
        for text in ("Я сейчас", '"I can not"', '" Я', ""):
            with self.subTest(text=text):
                self.assertFalse(selection.quote_opener(text))
        self.assertIn("quote-opener", selection.MESSAGE_CLASSES)


if __name__ == "__main__":
    unittest.main()
