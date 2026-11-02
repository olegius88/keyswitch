"""The digit-dot head (`2ю` for `2.`: digits with the Russian letter on the key of the English dot or comma): its class and
evidence, its routing in the trainer and the capture, and the engine's sign plan, which leaves the Russian layout on."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

import mixed_typing  # noqa: E402
import select_capture_messages as selection  # noqa: E402
import train_context_action_model as trainer  # noqa: E402
from keyswitch.constants.models import CONTEXT_ACTION_FEATURE_VERSION, DIGIT_DOT_FEATURE_PREFIX  # noqa: E402
from keyswitch.context_action_features import digit_dot_question, extract_action_features  # noqa: E402
from keyswitch.context_model import ACTIONS, ContextAction, ContextEvidence, ContextModel  # noqa: E402
from test_context_policy import ContextEngineTests  # noqa: E402
from test_context_training_captured import captured_question  # noqa: E402
from test_early_switch_settling import ScriptedActionModel  # noqa: E402
from test_variable_and_dotted_heads import evidence, prefixes  # noqa: E402


class DigitDotClassTests(unittest.TestCase):
    def test_the_class_is_digits_with_the_russian_letter_on_the_dot_or_comma_key(self) -> None:
        for original, alternative in (("2ю", "2."), ("5ю2", "5.2"), ("10ю25", "10.25")):
            with self.subTest(original=original):
                self.assertTrue(digit_dot_question(original, alternative, "space"))
        # No digit before the letter, two letters, another letter (`5б` is written as meant: a class, a building), a sign
        # typed as a sign, a letter after the digits that follow, and a boundary other than a space.
        for original, alternative, trigger in (("ю2", ".2", "space"), ("2юю", "2..", "space"), ("2в", "2d", "space"),
                                               ("5б", "5,", "space"), ("3б5", "3,5", "space"),
                                               ("2.", "2/", "space"), ("2ю2ю", "2.2.", "space"), ("2ю", "2.", "pause"),
                                               ("2ю", "2.", "punctuation")):
            with self.subTest(original=original, trigger=trigger):
                self.assertFalse(digit_dot_question(original, alternative, trigger))

    def test_the_head_reads_the_shared_features_again_and_its_own_evidence(self) -> None:
        features = extract_action_features(evidence("2ю", "2.", 1, "сделай пункт "))
        self.assertEqual(prefixes(features), {DIGIT_DOT_FEATURE_PREFIX})
        shared = {name for name in features if "|" not in name}
        self.assertTrue(all(DIGIT_DOT_FEATURE_PREFIX + name in features for name in shared if features[name]))
        for name in ("bias", "sign:ю", "digits:1", "after_digits:0", "after:none:none", "code:0", "latin:0",
                     "sign:ю:digits:1"):
            self.assertIn(f"{DIGIT_DOT_FEATURE_PREFIX}{name}:direction:1", features)
        self.assertTrue(any(name.startswith(DIGIT_DOT_FEATURE_PREFIX + "previous_word:russian:") for name in features))
        version = extract_action_features(evidence("10ю25", "10.25", 1, "версии x = ", "вышла", "kept_next_word"))
        for name in ("digits:2", "after_digits:1", "code:1", "latin:1", "after:kept_next_word:ru"):
            self.assertIn(f"{DIGIT_DOT_FEATURE_PREFIX}{name}:direction:1", version)

    def test_a_reading_without_letters_needs_no_known_letters(self) -> None:
        # `2.` has no letter for the model to know: the letters of `2ю` alone decide whether the model supports the token.
        features = extract_action_features(evidence("2ю", "2.", 1, "сделай пункт "))
        source = next(name for name in features if name.startswith("source:char:") and "ю" in name)
        zero = (0.0,) * len(ACTIONS)
        self.assertTrue(ContextModel({source: zero}, "context-v3-test",
                                     feature_version=CONTEXT_ACTION_FEATURE_VERSION).supports_features(features))
        self.assertFalse(ContextModel({"bias": zero}, "context-v3-test",
                                      feature_version=CONTEXT_ACTION_FEATURE_VERSION).supports_features(features))

    def test_the_head_is_fitted_onto_a_frozen_base_with_an_epoch_of_its_own(self) -> None:
        self.assertEqual(trainer.HEAD_PREFIXES["digitdot"], DIGIT_DOT_FEATURE_PREFIX)
        self.assertIn(DIGIT_DOT_FEATURE_PREFIX, trainer.OWN_EPOCH_HEADS)
        heads = frozenset({DIGIT_DOT_FEATURE_PREFIX})
        self.assertTrue(trainer.head_question("2ю", "2.", "пункт ", "", "space", "none", heads))
        self.assertFalse(trainer.head_question("2ю", "2.", "пункт ", "", "pause", "none", heads))
        try:
            import context_action_cuda as cuda
        except ImportError:  # CuPy missing, as in CI: the GPU path does not load here
            self.skipTest("CuPy is required to load the GPU path")
        self.assertTrue(cuda.heads_question("2ю", "пункт ", "", "space"))


class DigitDotCaptureTests(unittest.TestCase):
    def test_a_capture_keeps_the_questions_of_the_class(self) -> None:
        question = mixed_typing.CAPTURED_QUESTIONS["digitdot"]
        self.assertTrue(question(captured_question("2ю", "2.", "пункт ")))
        self.assertFalse(question(captured_question("2ю", "2.", "пункт ", trigger="pause")))

    def test_messages_hold_digits_with_a_dot_or_digits_with_b(self) -> None:
        self.assertTrue(selection.digit_dot_token("версии", "5.2"))
        self.assertTrue(selection.digit_dot_token("пункт", "2."))
        self.assertFalse(selection.digit_dot_token("use", "2."))
        self.assertTrue(selection.digit_letter_token("класс", "5б"))
        self.assertFalse(selection.digit_letter_token("класс", "5в"))
        self.assertIn("digitdot", selection.CLASSES)
        self.assertIn("digitdot-letter", selection.CLASSES)


def digit_dot(item: ContextEvidence) -> ContextAction:
    return "convert" if item.original == "2ю" else "keep"


class DigitDotEngineTests(ContextEngineTests):
    """A model with the digit-dot head converts `2ю`: the sign is typed in the English layout and the Russian one stays."""

    def use(self, head: bool) -> ScriptedActionModel:
        model = ScriptedActionModel(digit_dot)
        model.answers_digit_dot = head
        self.engine.context_policy.model = model
        return model

    def test_the_sign_is_replaced_and_the_russian_layout_stays(self) -> None:
        self.use(head=True)
        self.reset_editor(1)
        self.type("сделай пункт ", group=1)
        self.type("2ю ", group=1)
        self.assertEqual(self.backend.text, "сделай пункт 2. ")
        self.assertEqual(self.backend.group, 1)
        self.assertEqual(self.engine.snapshot.current_group, 1)
        self.type("потом ", group=1)
        self.assertEqual(self.backend.text, "сделай пункт 2. потом ")

    def test_without_the_head_the_token_stays(self) -> None:
        self.use(head=False)
        self.reset_editor(1)
        self.type("сделай пункт ", group=1)
        self.type("2ю ", group=1)
        self.assertEqual(self.backend.text, "сделай пункт 2ю ")


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    suite = unittest.TestSuite()
    for case in (DigitDotClassTests, DigitDotCaptureTests, DigitDotEngineTests):
        for name in case.__dict__:
            if name.startswith("test_"):
                suite.addTest(case(name))
    return suite


if __name__ == "__main__":
    unittest.main()
