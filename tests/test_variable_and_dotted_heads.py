"""The variable head (a letter of the Russian layout after a Cyrillic word) and the dotted-abbreviation head (`n/r/` for
`т.к.`): their classes, their evidence, the lone-letter refusal they lift, and their frames from captured typing."""

from __future__ import annotations

import gzip
import hashlib
import json
import lzma
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

import mixed_typing  # noqa: E402
import select_capture_messages as selection  # noqa: E402
import train_context_action_model as trainer  # noqa: E402
from train_context_model import CAPTURED_COLUMNS  # noqa: E402
from fixture_values.counts import CAPTURE_FIXTURE_LEFT_OUT_QUESTION, CAPTURE_FIXTURE_QUESTIONS  # noqa: E402
from fixture_values.clock import PAUSE_TRIGGER_OFFSET_SECONDS  # noqa: E402
from keyswitch.constants.model_protocol import DEVELOPMENT, TRAIN  # noqa: E402
from keyswitch.constants.models import (  # noqa: E402
    CONTEXT_ACTION_FEATURE_VERSION, DOTTED_FEATURE_PREFIX, KEPT_FEATURE_PREFIX, VARIABLE_FEATURE_PREFIX,
)
from keyswitch.context_action_features import dotted_question, extract_action_features, variable_question  # noqa: E402
from keyswitch.context_model import ACTIONS, AfterOrigin, ContextEvidence, ContextModel  # noqa: E402
from keyswitch.input_context import FieldContext  # noqa: E402
from test_context_policy import ContextEngineTests  # noqa: E402
from test_context_training_captured import captured_line, captured_question  # noqa: E402
from test_early_switch_settling import ScriptedActionModel  # noqa: E402
from test_stranded_letters import letters  # noqa: E402


def evidence(original: str, alternative: str, group: int, before: str, after: str = "", origin: AfterOrigin = "none",
             trigger: str = "space", boundary: str = " ") -> ContextEvidence:
    return ContextEvidence(original, alternative, group, FieldContext("Code", "test", before, after, "text", source="uia"),
                           trigger, boundary_text=boundary, after_origin=origin)


def prefixes(features: dict[str, float]) -> set[str]:
    return {name.split("|", 1)[0] + "|" for name in features if "|" in name}


class VariableClassTests(unittest.TestCase):
    def test_the_class_is_a_letter_of_the_russian_layout_after_a_word_of_cyrillic_letters(self) -> None:
        for original, alternative, before, trigger in (("ч", "x", "для ", "space"), ("Т", "N", "время ", "punctuation"),
                                                       ("т", "n", "и ", "space")):
            with self.subTest(original=original, before=before):
                self.assertTrue(variable_question(original, alternative, before, trigger))
        # A Russian word of one letter, a word with a digit or Latin letters before it, no space before it, a letter
        # whose other reading is a sign, two letters, nothing before it.
        for original, alternative, before in (("и", "b", "для "), ("ч", "x", "2 "), ("ч", "x", "value "), ("ч", "x", "для"),
                                              ("б", ",", "если "), ("чт", "xn", "для "), ("ч", "x", "")):
            with self.subTest(original=original, before=before):
                self.assertFalse(variable_question(original, alternative, before, "space"))
        # At a pause the letter may open a word still being typed (`Ф` of `AmoCRM_v2`): no frame of the head's is one.
        self.assertFalse(variable_question("Ф", "A", "для ", "pause"))
        self.assertNotIn(VARIABLE_FEATURE_PREFIX, prefixes(extract_action_features(evidence("Ф", "A", 1, "для ", trigger="pause"))))

    def test_the_head_reads_the_letter_the_boundary_and_the_signs_of_code_before_it(self) -> None:
        plain = extract_action_features(evidence("ч", "x", 1, "для "))
        self.assertEqual(prefixes(plain), {VARIABLE_FEATURE_PREFIX})
        for name in ("letter:ч", "boundary:space:", "word:для", "code:0", "latin:0", "digit:0", "letter:ч:boundary:space:",
                     "letter:ч:code:0", "after:none:none"):
            self.assertIn(VARIABLE_FEATURE_PREFIX + name, plain)
        code = extract_action_features(evidence("Ч", "X", 1, "x = 1; для ", trigger="punctuation", boundary="."))
        for name in ("code:1", "latin:1", "digit:1", "case:upper", "boundary:punctuation:."):
            self.assertIn(VARIABLE_FEATURE_PREFIX + name, code)
        # Asked again beside a next word that stayed as typed, the head reads it too.
        kept = extract_action_features(evidence("ч", "x", 1, "для ", "равно", "kept_next_word"))
        self.assertIn(VARIABLE_FEATURE_PREFIX + "after:kept_next_word:ru", kept)
        self.assertTrue(any(name.startswith(KEPT_FEATURE_PREFIX) for name in kept))

    def test_a_model_with_the_head_answers_its_class(self) -> None:
        bias = (0.0,) * len(ACTIONS)
        self.assertTrue(ContextModel({VARIABLE_FEATURE_PREFIX + "bias": bias}, "context-v3-head",
                                     feature_version=CONTEXT_ACTION_FEATURE_VERSION).answers_variables)
        self.assertFalse(ContextModel({"bias": bias}, "context-v3-base", feature_version=CONTEXT_ACTION_FEATURE_VERSION).answers_variables)


class DottedClassTests(unittest.TestCase):
    def test_the_class_is_a_token_typed_with_a_slash_that_reads_as_letter_groups_joined_by_dots(self) -> None:
        for original, alternative in (("n/r/", "т.к."), ("n/a", "т.ф"), ("l/,/", "д.б."), ("N/t/", "Т.е."), ("usr/bin", "гык.ишт")):
            with self.subTest(original=original):
                self.assertTrue(dotted_question(original, alternative))
        # One group of letters is no member: `dev/` of a path (the owner's typing) reads `вум.` as `cv/` reads `см.`.
        for original, alternative in (("src/main", "ыкс.ьфшт"), ("1/2", "1.2"), ("т.к.", "n/r/"), ("nr", "тк"), ("cv/", "см."),
                                      ("dev/", "вум.")):
            with self.subTest(original=original):
                self.assertFalse(dotted_question(original, alternative))

    def test_the_head_reads_the_reading_its_shape_and_the_words_around_it(self) -> None:
        features = extract_action_features(evidence("n/r/", "т.к.", 0, "IDisposable ", "это", "field"))
        self.assertEqual(prefixes(features), {DOTTED_FEATURE_PREFIX})
        for name in ("reading:т.к.", "shape:1-1:1", "case:lower", "before:en", "after:field:ru", "reading:т.к.:before:en",
                     "shape:1-1:1:after:field:ru", "russian:4", "latin:3", "russian:4:latin:3:before:en"):
            self.assertIn(DOTTED_FEATURE_PREFIX + name, features)
        self.assertIn(DOTTED_FEATURE_PREFIX + "before:start", extract_action_features(evidence("N/t/", "Т.е.", 0, "")))
        # The parts of a path are no Russian words; a part that is no word at all is rarer than any.
        path = extract_action_features(evidence("usr/bin", "гык.ишт", 0, "в "))
        self.assertIn(DOTTED_FEATURE_PREFIX + "russian:0:latin:1", path)
        self.assertIn(DOTTED_FEATURE_PREFIX + "latin:na", extract_action_features(evidence("l/,/", "д.б.", 0, "")))


class VariableHeadEngineTests(ContextEngineTests):
    """A model with the variable head has learned a letter after a Cyrillic word: there its verdict stands."""

    def setUp(self) -> None:
        super().setUp()
        model = ScriptedActionModel(letters)
        model.answers_variables = True
        self.engine.context_policy.model = model

    def test_a_letter_after_a_cyrillic_word_follows_a_model_that_learned_it(self) -> None:
        self.reset_editor(1)
        self.type("подробнее ч ", group=1)
        self.assertEqual(self.backend.text, "подробнее x ")

    def test_a_letter_left_at_a_pause_may_open_a_word_and_still_needs_a_rule(self) -> None:
        # `Ф` of `AmoCRM_v2`, a pause after it: the head has no frame of a letter at a pause (the owner's typing, 14.10).
        self.reset_editor(1)
        self.type("для Ф", group=1)
        last = self.engine._last_word_input_at
        assert last is not None
        self.engine._maybe_correct_after_pause(now=last + PAUSE_TRIGGER_OFFSET_SECONDS)
        self.assertEqual(self.backend.text, "для Ф")

    def test_outside_the_heads_class_a_lone_letter_still_needs_a_rule(self) -> None:
        self.reset_editor(1)
        self.type("работаю в 1С ", group=1)
        self.assertEqual(self.backend.text, "работаю в 1С ")
        self.reset_editor(0)
        self.type("nats b ")
        self.assertEqual(self.backend.text, "nats b ")


class CapturedHeadCurriculumTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        data = self.root / "questions.jsonl.xz"
        data.write_bytes(lzma.compress("".join((
            captured_line(1, "ч", "x", 1, "convert", "для "), captured_line(1, "т", "n", 1, "keep", "и "),
            captured_line(1, "ghbdtn", "привет", 0, "convert", "я "), captured_line(1, "n/r/", "т.к.", 0, "convert", "IDisposable "),
            captured_line(1, "т", "n", 1, "wait", "и "),
            captured_line(1, "ш", "i", 1, "convert", "индекс ").replace('"none"', '"planned_next_conversion"'))).encode()))
        manifest = self.root / "manifest.json"
        manifest.write_text(json.dumps({"schema_version": 1, "columns": list(CAPTURED_COLUMNS),
                                        "sources": [{"file": data.name, "sha256": hashlib.sha256(data.read_bytes()).hexdigest(),
                                                     "weight": 1.0}]}), encoding="utf-8")
        self.options = {"captured": {TRAIN: [[str(manifest), data.name]]}, "sample_weight": 1.0}

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def test_each_head_takes_the_captured_questions_of_its_class_for_the_splits_named(self) -> None:
        rows, report = trainer.variable_curriculum(TRAIN, [], frozenset(), self.options)
        self.assertEqual([(row.original, row.action, row.category) for row in rows], [("ч", "convert", "variable"), ("т", "keep", "variable")])
        self.assertEqual(report["frames"], len(rows))
        # A question planned beside a next word with none after it was never asked: it is left out.
        self.assertEqual(report["by_file"], {"questions.jsonl.xz:convert": 1, "questions.jsonl.xz:keep": 1,
                                             "skipped_planned_without_word": 1})
        rows, _report = trainer.dotted_curriculum(TRAIN, [], frozenset(), self.options)
        self.assertEqual([(row.original, row.action, row.field.before) for row in rows], [("n/r/", "convert", "IDisposable ")])
        self.assertEqual(trainer.variable_curriculum(DEVELOPMENT, [], frozenset(), self.options), ([], {"frames": 0, "scope": "not used"}))

    def test_the_heads_are_fitted_onto_a_frozen_base_with_epochs_of_their_own(self) -> None:
        self.assertEqual((trainer.HEAD_PREFIXES["variable"], trainer.HEAD_PREFIXES["dotted"]), (VARIABLE_FEATURE_PREFIX, DOTTED_FEATURE_PREFIX))
        self.assertIn(VARIABLE_FEATURE_PREFIX, trainer.OWN_EPOCH_HEADS)
        self.assertIn(DOTTED_FEATURE_PREFIX, trainer.OWN_EPOCH_HEADS)
        heads = frozenset({VARIABLE_FEATURE_PREFIX, DOTTED_FEATURE_PREFIX})
        self.assertTrue(trainer.head_question("ч", "x", "для ", "", "space", "none", heads))
        self.assertTrue(trainer.head_question("ч", "x", "для ", "равно", "space", "kept_next_word", heads))
        self.assertTrue(trainer.head_question("n/r/", "т.к.", "IDisposable ", "", "space", "none", heads))
        self.assertFalse(trainer.head_question("ч", "x", "для ", "", "space", "none", frozenset()))
        self.assertFalse(trainer.head_question("ч", "x", "для ", "", "pause", "none", heads))
        self.assertFalse(trainer.head_question("cv/", "см.", "", "", "space", "none", heads))


class CaptureSelectionTests(unittest.TestCase):
    def test_a_capture_keeps_the_questions_of_one_heads_class(self) -> None:
        self.assertTrue(mixed_typing.CAPTURED_QUESTIONS["variable"](captured_question("ч", "x", "для ", "space")))
        self.assertFalse(mixed_typing.CAPTURED_QUESTIONS["variable"](captured_question("и", "b", "для ", "space")))
        self.assertFalse(mixed_typing.CAPTURED_QUESTIONS["variable"](captured_question("ч", "x", "для ", "pause")))
        self.assertTrue(mixed_typing.CAPTURED_QUESTIONS["dotted"](captured_question("n/r/", "т.к.", "", "space")))

    def test_messages_with_a_token_of_the_class_are_chosen_by_question(self) -> None:
        self.assertTrue(selection.variable_token("для", "x,"))
        self.assertTrue(selection.variable_token("для", "ч"))
        self.assertFalse(selection.variable_token("для", "и") or selection.variable_token("value", "x") or selection.variable_token("", "x"))
        # The head's own predicate chooses: Russian tokens by their keys in the English layout, Latin ones as written.
        for token in ("(т.к.", "n/a,", "д.б.", "usr/bin", "src/app/"):
            with self.subTest(token=token):
                self.assertTrue(selection.dotted_token("", token))
        for token in ("все.", "src/main", "dev/", "1/2"):
            with self.subTest(token=token):
                self.assertFalse(selection.dotted_token("", token))
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            messages, members = root / "so-train.jsonl.gz", root / "devchat-dev.members.jsonl.gz"
            texts = {CAPTURE_FIXTURE_LEFT_OUT_QUESTION: "значение x равно", CAPTURE_FIXTURE_QUESTIONS[0]: "значение x равно",
                     CAPTURE_FIXTURE_QUESTIONS[1]: "обычная фраза"}
            with gzip.open(messages, "wt", encoding="utf-8") as handle:
                for question, text in texts.items():
                    handle.write(json.dumps({"qid": question, "kind": "comment", "mix": "mixed", "text": text}, ensure_ascii=False) + "\n")
            with gzip.open(members, "wt", encoding="utf-8") as handle:
                handle.write(json.dumps({"text": "", "question": CAPTURE_FIXTURE_LEFT_OUT_QUESTION, "mix": "mixed"}) + "\n")
            report = selection.select(messages, [members], "variable", 0.0, root / "out")
            self.assertEqual(report["messages"], {"train": 1, "dev": 0, "left_out": 1})
            with gzip.open(root / "out/so-variable-train.jsonl.gz", "rt", encoding="utf-8") as handle:
                self.assertEqual([json.loads(line)["qid"] for line in handle], [CAPTURE_FIXTURE_QUESTIONS[0]])
            self.assertEqual(selection.split_of(CAPTURE_FIXTURE_QUESTIONS[0], 1.0), "dev")


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    # Each class's own tests alone: the engine tests' base holds tests of its own, run in its module.
    return unittest.TestSuite(case(name) for case in (
        VariableClassTests, DottedClassTests, VariableHeadEngineTests, CapturedHeadCurriculumTests, CaptureSelectionTests)
        for name in case.__dict__ if name.startswith("test_"))
