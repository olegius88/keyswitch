"""The resume head (`seek() , f там` for `seek() , а там`: a short word right after Latin text asked again once its next
word is planned for conversion): its class and evidence, its routing in the trainer and the capture's messages and
frames."""

from __future__ import annotations

import sys
import unittest
from dataclasses import replace
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

import mixed_typing  # noqa: E402
import select_capture_messages as selection  # noqa: E402
import train_context_action_model as trainer  # noqa: E402
from keyswitch.constants.models import CONTEXT_ACTION_FEATURE_VERSION, LETTER_FEATURE_PREFIX, RESUME_FEATURE_PREFIX  # noqa: E402
from keyswitch.context_action_features import extract_action_features, resume_question  # noqa: E402
from keyswitch.context_model import ACTIONS, AfterOrigin, ContextAction, ContextEvidence, ContextModel  # noqa: E402
from keyswitch.language_model import WordScore  # noqa: E402
from test_context_policy import ContextEngineTests  # noqa: E402
from test_early_switch_settling import ScriptedActionModel  # noqa: E402
from test_variable_and_dotted_heads import evidence, prefixes  # noqa: E402

PLANNED: AfterOrigin = "planned_next_conversion"


class ResumeClassTests(unittest.TestCase):
    def test_the_class_is_a_short_word_after_latin_text_asked_again_before_a_planned_next_word(self) -> None:
        for original, alternative, before, after in (("f", "а", "вы вызываете seek() , ", "там"), ("z", "я", "TSet - ", "использовал"),
                                                     ("ds", "вы", "нажать Shift+Tab ", "и"), ("(b", "(и", "pthread_rwlock_rdlock ", "наверное"),
                                                     ("x", "ч", "i = 0; ", "в"), ("ч", "x", "value = ", "равно")):
            with self.subTest(original=original, before=before):
                self.assertTrue(resume_question(original, alternative, before, after, PLANNED))
        # Another question, Russian text or nothing before it, no word after it, four letters, a digit in the token, an
        # alternative that is no word, and a letter right after a Latin word and a space (the single-letter head's).
        for original, alternative, before, after, origin in (
                ("f", "а", "seek() , ", "там", "field"), ("f", "а", "seek() , ", "там", "kept_next_word"),
                ("f", "а", "вызываете ", "там", PLANNED), ("f", "а", "", "там", PLANNED), ("f", "а", "seek() , ", "1", PLANNED),
                ("ghbd", "прив", "seek ", "мир", PLANNED), ("f2", "а2", "seek ", "там", PLANNED), ("b", "и", "seek ", "там", PLANNED),
                ("ns", "ты.", "seek , ", "там", PLANNED)):
            with self.subTest(original=original, before=before, origin=origin):
                self.assertFalse(resume_question(original, alternative, before, after, origin))

    def test_the_head_reads_the_shared_features_again_and_its_own_evidence(self) -> None:
        item = replace(evidence("f", "а", 0, "вы вызываете seek() , ", "там", PLANNED), source_known=True, target_known=True,
                       source_score=WordScore(0.0, True, 0, 0.0, spell_known=True))
        features = extract_action_features(item)
        self.assertEqual(prefixes(features), {RESUME_FEATURE_PREFIX})
        shared = {name for name in features if "|" not in name}
        self.assertTrue(shared)
        self.assertTrue(all(RESUME_FEATURE_PREFIX + name in features for name in shared if features[name]))
        for name in ("bias", "gap:(),", "spell:1:0", "sign:none", "letters:1", "case:lower", "letters:1:spell:1:0"):
            self.assertIn(f"{RESUME_FEATURE_PREFIX}{name}:direction:0", features)
        self.assertTrue(any(name.startswith(RESUME_FEATURE_PREFIX + "previous_word:english:") for name in features))
        self.assertTrue(any(name.startswith(RESUME_FEATURE_PREFIX + "next_word:russian:") for name in features))
        # A space alone between them, a leading parenthesis, and the case of a word in capitals.
        spaced = extract_action_features(evidence("DS", "ВЫ", 0, "Shift+Tab ", "и", PLANNED))
        for name in ("gap:space", "case:upper", "letters:2"):
            self.assertIn(f"{RESUME_FEATURE_PREFIX}{name}:direction:0", spaced)
        self.assertIn(f"{RESUME_FEATURE_PREFIX}sign:(:direction:0",
                      extract_action_features(evidence("(b", "(и", 0, "rdlock ", "наверное", PLANNED)))
        # The single-letter head keeps its class, and the first asking of the word is the frozen model's alone.
        self.assertEqual(prefixes(extract_action_features(evidence("b", "и", 0, "nats ", "redis", PLANNED))), {LETTER_FEATURE_PREFIX})
        self.assertNotIn(RESUME_FEATURE_PREFIX, prefixes(extract_action_features(evidence("f", "а", 0, "seek() , "))))

    def test_the_head_is_fitted_onto_a_frozen_base_with_an_epoch_of_its_own(self) -> None:
        self.assertEqual(trainer.HEAD_PREFIXES["resume"], RESUME_FEATURE_PREFIX)
        self.assertIn(RESUME_FEATURE_PREFIX, trainer.OWN_EPOCH_HEADS)
        heads = frozenset({RESUME_FEATURE_PREFIX})
        self.assertTrue(trainer.head_question("f", "а", "seek() , ", "там", "space", PLANNED, heads))
        self.assertFalse(trainer.head_question("f", "а", "seek() , ", "там", "space", "field", heads))
        self.assertFalse(trainer.head_question("f", "а", "вызываете ", "там", "space", PLANNED, heads))
        try:
            import context_action_cuda as cuda
        except ImportError:  # CuPy missing, as in CI: the GPU path does not load here
            self.skipTest("CuPy is required to load the GPU path")
        self.assertTrue(cuda.heads_question("f", "seek() , ", "там", "space", "field", PLANNED))
        self.assertFalse(cuda.heads_question("f", "seek() , ", "там", "space", "field", "field"))


class ResumeCaptureTests(unittest.TestCase):
    def test_a_capture_keeps_the_questions_of_the_class(self) -> None:
        question = mixed_typing.CAPTURED_QUESTIONS["resume"]
        row = {"original": "f", "alternative": "а", "before": "seek() , ", "after": "там", "after_origin": PLANNED}
        self.assertTrue(question(row))
        self.assertFalse(question({**row, "after_origin": "field"}))
        self.assertFalse(question({**row, "before": "вызываете "}))

    def test_messages_hold_a_short_word_after_latin_text_before_a_russian_word(self) -> None:
        for text in ("вы вызываете seek() , а там ничего", "TSet - я использовал", "для x в списке", "rdlock (и наверное)"):
            with self.subTest(text=text):
                self.assertTrue(selection.resumes(text))
        for text in ("а там ничего", "seek() , а", "seek() , then there", "вызываете а там", "seek() , ничего там"):
            with self.subTest(text=text):
                self.assertFalse(selection.resumes(text))
        self.assertIn("resume", selection.MESSAGE_CLASSES)


def resumed(item: ContextEvidence) -> ContextAction:
    """`f` is kept at its space, as Hunspell had it, and converts once `nfv` (`там`) after it is planned for conversion."""

    if item.original == "f":
        return "convert" if item.after_origin == "planned_next_conversion" and item.field.after == "там" else "keep"
    return "convert" if item.original == "nfv" else "keep"


class ResumeEngineTests(ContextEngineTests):
    """A short word after Latin text that the model converts once its next word converts is taken along mid-line, by a
    model with the resume head alone."""

    def setUp(self) -> None:
        super().setUp()
        self.model = ScriptedActionModel(resumed)
        self.engine.context_policy.model = self.model

    def test_a_model_with_the_head_answers_its_class(self) -> None:
        bias = (0.0,) * len(ACTIONS)
        self.assertTrue(ContextModel({RESUME_FEATURE_PREFIX + "bias": bias}, "context-v3-head",
                                     feature_version=CONTEXT_ACTION_FEATURE_VERSION).answers_resume)
        self.assertFalse(ContextModel({"bias": bias}, "context-v3-base", feature_version=CONTEXT_ACTION_FEATURE_VERSION).answers_resume)

    def test_the_word_after_latin_text_is_taken_along_mid_line(self) -> None:
        self.model.answers_resume = True
        self.reset_editor(1)
        self.type("вызываете ", group=1)
        self.type("seek() , f nfv ", group=0)
        self.assertEqual(self.backend.text, "вызываете seek() , а там ")
        self.assertIn(("f", "вызываете seek() , ", "там"), self.model.questions)

    def test_without_the_head_the_word_stays(self) -> None:
        self.reset_editor(1)
        self.type("вызываете ", group=1)
        self.type("seek() , f nfv ", group=0)
        self.assertEqual(self.backend.text, "вызываете seek() , f там ")


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    suite = unittest.TestSuite()
    for case in (ResumeClassTests, ResumeCaptureTests, ResumeEngineTests):
        for name in case.__dict__:
            if name.startswith("test_"):
                suite.addTest(case(name))
    return suite


if __name__ == "__main__":
    unittest.main()
