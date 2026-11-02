"""The dash head (`-v` after `git`, `-м` typed for it with the Russian layout on: a command-line flag in either layout):
its class and evidence, its routing in the trainer and the capture's messages and frames."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

import mixed_typing  # noqa: E402
import select_capture_messages as selection  # noqa: E402
import train_context_action_model as trainer  # noqa: E402
from keyswitch.constants.models import DASH_FEATURE_PREFIX  # noqa: E402
from keyswitch.context_action_features import dash_question, extract_action_features  # noqa: E402
from test_context_training_captured import captured_question  # noqa: E402
from test_variable_and_dotted_heads import evidence, prefixes  # noqa: E402


class DashClassTests(unittest.TestCase):
    def test_the_class_is_a_flag_of_one_or_two_dashes_and_letters_in_either_reading(self) -> None:
        for original, alternative, trigger in (("-v", "-м", "space"), ("-м", "-v", "space"), ("--force", "--ащксу", "enter"),
                                               ("-v2", "-м2", "punctuation"), ("-цфеср", "-watch", "space")):
            with self.subTest(original=original, trigger=trigger):
                self.assertTrue(dash_question(original, alternative, trigger))
        # Three dashes, a dash alone, a dash inside a word, digits before letters, a reading with a sign, mixed scripts,
        # and a pause, where the flag may still be being typed.
        for original, alternative, trigger in (("---v", "---м", "space"), ("-", "-", "space"), ("a-b", "ф-и", "space"),
                                               ("-2v", "-2м", "space"), ("-b", "-и", "pause"), ("-vм", "-мv", "space"),
                                               ("-,", "-б", "space")):
            with self.subTest(original=original, trigger=trigger):
                self.assertFalse(dash_question(original, alternative, trigger))

    def test_the_head_reads_the_shared_features_again_and_its_own_evidence(self) -> None:
        features = extract_action_features(evidence("-v", "-м", 0, "запусти git "))
        self.assertEqual(prefixes(features), {DASH_FEATURE_PREFIX})
        shared = {name for name in features if "|" not in name}
        self.assertTrue(all(DASH_FEATURE_PREFIX + name in features for name in shared if features[name]))
        for name in ("bias", "dashes:1", "letters:1", "case:lower", "digits:0", "code:0", "dashes:1:case:lower"):
            self.assertIn(f"{DASH_FEATURE_PREFIX}{name}:direction:0", features)
        for side in ("letters:english:", "reading:russian:", "previous_word:english:"):
            self.assertTrue(any(name.startswith(DASH_FEATURE_PREFIX + side) for name in features), side)
        flag = extract_action_features(evidence("--Force2", "--Ащксу2", 0, "x = 1; npm i "))
        for name in ("dashes:2", "letters:4", "case:title", "digits:1", "code:1"):
            self.assertIn(f"{DASH_FEATURE_PREFIX}{name}:direction:0", flag)

    def test_the_head_is_fitted_onto_a_frozen_base_with_an_epoch_of_its_own(self) -> None:
        self.assertEqual(trainer.HEAD_PREFIXES["dash"], DASH_FEATURE_PREFIX)
        self.assertIn(DASH_FEATURE_PREFIX, trainer.OWN_EPOCH_HEADS)
        heads = frozenset({DASH_FEATURE_PREFIX})
        self.assertTrue(trainer.head_question("-v", "-м", "git ", "", "space", "none", heads))
        self.assertFalse(trainer.head_question("-v", "-м", "git ", "", "pause", "none", heads))
        try:
            import context_action_cuda as cuda
        except ImportError:  # CuPy missing, as in CI: the GPU path does not load here
            self.skipTest("CuPy is required to load the GPU path")
        self.assertTrue(cuda.heads_question("-v", "git ", "", "space"))
        self.assertTrue(cuda.heads_question("-м", "git ", "", "space"))


class DashCaptureTests(unittest.TestCase):
    def test_a_capture_keeps_the_questions_of_the_class(self) -> None:
        question = mixed_typing.CAPTURED_QUESTIONS["dash"]
        self.assertTrue(question(captured_question("-v", "-м", "git ")))
        self.assertFalse(question(captured_question("-v", "-м", "git ", trigger="pause")))

    def test_messages_hold_a_flag(self) -> None:
        for token in ("-v", "--force", "-v2", "-цфеср", "-rf,"):
            with self.subTest(token=token):
                self.assertTrue(selection.dash_token("", token))
        for token in ("-", "--", "-1", "a-b", "---v"):
            with self.subTest(token=token):
                self.assertFalse(selection.dash_token("", token))
        self.assertIn("dash", selection.CLASSES)


if __name__ == "__main__":
    unittest.main()
