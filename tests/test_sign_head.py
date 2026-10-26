"""The sign head (`м + ` for `v + `: a Latin variable typed in the Russian layout before a formula sign): its class and
evidence, the engine's question about a letter kept before such a sign, and the capture's messages and frames."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

import mixed_typing  # noqa: E402
import select_capture_messages as selection  # noqa: E402
import train_context_action_model as trainer  # noqa: E402
from keyswitch.backend import KeyEvent  # noqa: E402
from keyswitch.constants.models import (  # noqa: E402
    CONTEXT_ACTION_FEATURE_VERSION, KEPT_FEATURE_PREFIX, SIGN_FEATURE_PREFIX, VARIABLE_FEATURE_PREFIX,
)
from keyswitch.context_action_features import extract_action_features, sign_question  # noqa: E402
from keyswitch.context_model import FEATURE_VERSION, ContextAction, ContextEvidence  # noqa: E402
from test_context_policy import ContextEngineTests  # noqa: E402
from test_context_wait_pairs import ScriptedModel  # noqa: E402
from test_variable_and_dotted_heads import evidence, prefixes  # noqa: E402


class SignClassTests(unittest.TestCase):
    def test_the_class_is_a_variable_letter_asked_again_with_a_letterless_token_after_it(self) -> None:
        for after in ("+", "=", "==", "-=", "25", "+="):
            with self.subTest(after=after):
                self.assertTrue(sign_question("м", "v", "прохождения ", after, "space", "kept_next_word"))
        # A word after it, a dash alone (a minus or the dash of a sentence: only what follows tells), nothing after it,
        # another question, or a letter out of the variable class (a Russian word of one letter, after a Latin word).
        for original, alternative, before, after, origin in (
                ("м", "v", "прохождения ", "проверь", "kept_next_word"), ("м", "v", "прохождения ", "-", "kept_next_word"),
                ("м", "v", "прохождения ", "--", "kept_next_word"), ("м", "v", "прохождения ", "", "kept_next_word"),
                ("м", "v", "прохождения ", "+", "none"), ("м", "v", "прохождения ", "+", "planned_next_conversion"),
                ("и", "b", "прохождения ", "+", "kept_next_word"), ("м", "v", "value ", "+", "kept_next_word")):
            with self.subTest(original=original, before=before, after=after, origin=origin):
                self.assertFalse(sign_question(original, alternative, before, after, "space", origin))

    def test_the_head_reads_the_token_beside_the_kept_and_variable_evidence(self) -> None:
        features = extract_action_features(evidence("м", "v", 1, "добивайся прохождения ", "+", "kept_next_word"))
        self.assertEqual(prefixes(features), {KEPT_FEATURE_PREFIX, VARIABLE_FEATURE_PREFIX, SIGN_FEATURE_PREFIX})
        for name in ("bias", "token:+", "kind:operator", "letter:м", "case:lower", "code:0", "latin:0", "digit:0",
                     "token:+:case:lower", "kind:operator:code:0"):
            self.assertIn(SIGN_FEATURE_PREFIX + name, features)
        # Digits read as digits, a longer token as one kind, and code before the letter is read with the token.
        for after, before, name in (("25", "для ", "token:digits"), ("25", "для ", "kind:digits"), ("+++", "для ", "token:other"),
                                    ("()", "для ", "kind:other"), ("==", "if (a) для ", "token:==:code:1")):
            with self.subTest(after=after):
                self.assertIn(SIGN_FEATURE_PREFIX + name,
                              extract_action_features(evidence("ч", "x", 1, before, after, "kept_next_word")))
        self.assertFalse(any(name.startswith(SIGN_FEATURE_PREFIX + "token:+:letter") for name in features))
        # The letter's own question at its space is the variable head's alone.
        self.assertNotIn(SIGN_FEATURE_PREFIX + "bias", extract_action_features(evidence("м", "v", 1, "прохождения ")))

    def test_the_head_is_fitted_onto_a_frozen_base_with_an_epoch_of_its_own(self) -> None:
        self.assertEqual(trainer.HEAD_PREFIXES["sign"], SIGN_FEATURE_PREFIX)
        self.assertIn(SIGN_FEATURE_PREFIX, trainer.OWN_EPOCH_HEADS)
        heads = frozenset({SIGN_FEATURE_PREFIX})
        self.assertTrue(trainer.head_question("м", "v", "прохождения ", "+", "space", "kept_next_word", heads))
        self.assertFalse(trainer.head_question("м", "v", "прохождения ", "проверь", "space", "kept_next_word", heads))


class SignCaptureTests(unittest.TestCase):
    def test_a_capture_keeps_the_questions_of_the_class(self) -> None:
        question = mixed_typing.CAPTURED_QUESTIONS["sign"]
        row = {"original": "м", "alternative": "v", "before": "прохождения ", "after": "+", "trigger": "space",
               "after_origin": "kept_next_word"}
        self.assertTrue(question(row))
        self.assertFalse(question({**row, "after_origin": "none"}))
        self.assertFalse(question({**row, "after": "проверь"}))

    def test_messages_hold_a_variable_letter_before_a_token_both_layouts_type_alike(self) -> None:
        for text in ("если x = 0", "прохождения v + s", "согласно ГОСТ Р 54960", "для x 25", "где a == b"):
            with self.subTest(text=text):
                self.assertTrue(selection.formula(text))
        # A sign the Russian layout types otherwise, a dash alone, the dump's markdown escape, a sign stuck to the letter, a
        # word after the letter, a Russian word of one letter.
        for text in ("где a < 2", "пункт б - да", "а \\- б", "для x:", "для x и y", "и + да"):
            with self.subTest(text=text):
                self.assertFalse(selection.formula(text))
        self.assertIn("formula", selection.MESSAGE_CLASSES)


def formula_variable(item: ContextEvidence) -> ContextAction:
    """`м` is kept at its space; asked with `+` or `=` after it, it converts."""

    if item.original == "м" and item.after_origin == "kept_next_word":
        return "convert" if item.field.after in {"+", "="} else "keep"
    return "keep"


class LetterBeforeSignTests(ContextEngineTests):
    def script(self, answer: object = formula_variable, version: int = CONTEXT_ACTION_FEATURE_VERSION) -> ScriptedModel:
        model = ScriptedModel(answer)  # type: ignore[arg-type]
        model.feature_version = version
        # The variable head's weights make the model's verdict on a lone letter of its class an opinion (context_policy).
        model.answers_variables = True
        self.engine.context_policy.model = model
        return model

    def test_a_kept_letter_converts_with_the_sign_and_the_layout_follows(self) -> None:
        model = self.script()
        self.reset_editor(1)
        self.type("прохождения м + ", group=1)
        self.assertEqual(self.backend.text, "прохождения v + ")
        self.assertIn(("м", "прохождения ", "+"), model.questions)
        self.assertEqual(self.backend.kept_tails[-1], 0)
        self.assertEqual((self.backend.group, self.engine.snapshot.current_group), (0, 0))
        correction = self.engine._last_correction
        assert correction is not None
        self.assertEqual((correction.mode, correction.original, correction.replacement), ("context_word", "м +", "v +"))
        # The next letter of the formula is typed in the layout that followed, and nothing is taken along twice.
        self.type("s ")
        self.assertEqual(self.backend.text, "прохождения v + s ")
        self.assertEqual([word.plan.original for word in self.engine._kept], ["s"])

    def test_a_waiting_letter_before_a_sign_converts_with_it_too(self) -> None:
        def waits(item: ContextEvidence) -> ContextAction:
            return "suggest" if item.after_origin == "none" and item.original == "м" else formula_variable(item)

        self.script(waits)
        self.reset_editor(1)
        self.type("если м = ", group=1)
        self.assertEqual(self.backend.text, "если v = ")
        self.assertEqual(self.backend.kept_tails[-1], 0)
        self.assertEqual(self.backend.group, 0)

    def test_a_declining_model_leaves_the_letter(self) -> None:
        model = self.script(lambda item: "keep")
        self.reset_editor(1)
        self.type("прохождения м + ", group=1)
        self.assertEqual(self.backend.text, "прохождения м + ")
        self.assertIn(("м", "прохождения ", "+"), model.questions)

    def test_only_a_letter_of_the_class_before_a_token_both_layouts_type_alike_is_asked(self) -> None:
        for text in ("прохождения м проверь ", "прохождения м - ", "и + ", "прохождения мм + "):
            with self.subTest(text=text):
                model = self.script()
                self.reset_editor(1)
                self.type(text, group=1)
                self.assertEqual(self.backend.text, text)
                self.assertFalse(any(after for _original, _before, after in model.questions))
        # `.` of the Russian layout is `/` in the English one.
        model = self.script()
        self.reset_editor(1)
        self.type("прохождения м ", group=1)
        self.serial += 1
        self.tap(KeyEvent(True, self.serial, "slash", ".", ("/", "."), 1, 0, self.serial))
        self.type(" ", group=1)
        self.assertEqual(self.backend.text, "прохождения м . ")
        self.assertFalse(any(after for _original, _before, after in model.questions))

    def test_a_model_of_an_older_schema_or_a_changed_text_is_not_asked(self) -> None:
        model = self.script(version=FEATURE_VERSION)
        self.reset_editor(1)
        self.type("прохождения м + ", group=1)
        self.assertNotIn(("м", "прохождения ", "+"), model.questions)
        model = self.script()
        self.reset_editor(1)
        self.type("прохождения м ", group=1)
        self.engine.context_policy.stream.clear()
        self.type("+ ", group=1)
        self.assertEqual(self.backend.text, "прохождения м + ")
        self.assertNotIn(("м", "прохождения ", "+"), model.questions)


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    suite = unittest.TestSuite()
    for case in (SignClassTests, SignCaptureTests, LetterBeforeSignTests):
        for name in case.__dict__:
            if name.startswith("test_"):
                suite.addTest(case(name))
    return suite


if __name__ == "__main__":
    unittest.main()
