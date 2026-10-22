"""The letters-and-digits head (`м67` for `v67`, `ч64` for `x64`, `1с` meant as typed): its class, its evidence and the
table of whole tokens it reads, the lone-letter refusal it lifts, and its frames from captured typing."""

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
from fixture_values.counts import (  # noqa: E402
    ALNUM_FIXTURE_CONVERT_WEIGHT, ALNUM_FIXTURE_TOKEN_COUNT, CAPTURE_FIXTURE_LEFT_OUT_QUESTION, CAPTURE_FIXTURE_QUESTIONS,
)
from keyswitch.constants.model_protocol import DEVELOPMENT, TRAIN  # noqa: E402
from keyswitch.constants.models import (  # noqa: E402
    ALNUM_FEATURE_PREFIX, CONTEXT_ACTION_FEATURE_VERSION, KEPT_FEATURE_PREFIX,
)
from keyswitch.context_action_features import _alnum_shape, alnum_question, extract_action_features  # noqa: E402
from keyswitch.context_model import (  # noqa: E402
    ACTIONS, ALNUM_ALPHABETS, ALNUM_FREQUENCY_PATH, ContextAction, ContextEvidence, ContextModel, _alnum_frequency,
)
from test_context_policy import ContextEngineTests  # noqa: E402
from test_context_training_captured import captured_line  # noqa: E402
from test_early_switch_settling import ScriptedActionModel  # noqa: E402
from test_variable_and_dotted_heads import evidence, prefixes  # noqa: E402


def alnum_converted(item: ContextEvidence) -> ContextAction:
    """The model converts every token of the head's class and keeps every word."""

    return "convert" if alnum_question(item.original, item.alternative, item.field.before) else "keep"


class AlnumClassTests(unittest.TestCase):
    def test_the_class_is_a_counted_token_typed_in_russian_letters_and_digits_after_russian_text(self) -> None:
        for original, alternative, before in (("ч64", "x64", "собрал под "), ("зь2", "pm2", "настроить "), ("2аф", "2fa", ""),
                                              ("1с", "1c", "работаю в "), ("100мс", "100vc", "прошло "), ("3д", "3l", "сделал ")):
            with self.subTest(original=original):
                self.assertTrue(alnum_question(original, alternative, before))
        # Typed in Latin letters, after English text, counted in neither script (`v67`, `e17`), a reading with a sign
        # (`2х` reads `2[`), a sign inside the token, no digit, no letter, letters of both scripts, a token longer than the
        # class's.
        for original, alternative, before in (("x64", "ч64", "под "), ("ч64", "x64", "the price is "), ("м67", "v67", "или "),
                                              ("У17", "E17", ""), ("2х", "2[", "в "),
                                              ("-м2", "-v2", "в "), ("5ю2", "5.2", "в "), ("м", "v", "в "), ("12", "12", "в "),
                                              ("мv2", "vм2", "в "), ("ыйдшеу3ч1", "sqlite3x1", "в "), ("м²", "v²", "в ")):
            with self.subTest(original=original):
                self.assertFalse(alnum_question(original, alternative, before))

    def test_the_shape_is_the_runs_of_letters_and_digits(self) -> None:
        self.assertEqual([_alnum_shape(token) for token in ("м67", "ц4и", "100мс", "ГОСТ2012", "a1b2c3", "1с")],
                         ["L1D2", "L1D1L1", "D3L2", "L3D3", "many", "D1L1"])

    def test_the_table_counts_whole_tokens_in_both_scripts(self) -> None:
        table = _alnum_frequency()
        self.assertEqual(sorted(table), list(ALNUM_ALPHABETS))
        self.assertGreater(table["latin"]["ipv6"], table["cyrillic"].get("шзм6", 0))
        self.assertGreater(table["cyrillic"]["1с"], table["latin"]["1c"])
        self.assertTrue(ALNUM_FREQUENCY_PATH.is_file())

    def test_the_head_reads_the_whole_token_the_shape_and_the_text_before(self) -> None:
        typed = extract_action_features(evidence("зь2", "pm2", 1, "настроить "))
        self.assertEqual(prefixes(typed), {ALNUM_FEATURE_PREFIX})
        for name in ("bias", "token:0:2", "whole:-8", "shape:L2D1", "ratio:-7", "seen:0:1", "case:lower", "before:ru:ru",
                     "code:0:digit:0", "boundary:space:", "after:none:none", "shape:L2D1:whole:-8", "shape:L2D1:before:ru:ru"):
            self.assertIn(ALNUM_FEATURE_PREFIX + name, typed)
        # `ipv6` is written 351 times in Russian text, `шзм6` never; `1с` far more often than `1c`.
        self.assertIn(ALNUM_FEATURE_PREFIX + "whole:-8", extract_action_features(evidence("шзм6", "ipv6", 1, "прокси ")))
        self.assertIn(ALNUM_FEATURE_PREFIX + "token:3:2", extract_action_features(evidence("1с", "1c", 1, "работаю в ")))
        code = extract_action_features(evidence("Ч64", "X64", 1, "x = 1; собрал под ", trigger="punctuation", boundary="."))
        for name in ("case:upper", "code:1:digit:1", "boundary:punctuation:."):
            self.assertIn(ALNUM_FEATURE_PREFIX + name, code)
        self.assertIn(ALNUM_FEATURE_PREFIX + "before:none:start", extract_action_features(evidence("зь2", "pm2", 1, "")))
        # After English text, and for a token the table counts in neither script, the frozen model decides.
        self.assertEqual(prefixes(extract_action_features(evidence("ч64", "x64", 1, "richest the "))), set())
        self.assertEqual(prefixes(extract_action_features(evidence("м67", "v67", 1, "правильную или "))), set())
        # Asked again beside a next word that stayed as typed, the head reads it too (`2аф ввел`, the owner's typing).
        kept = extract_action_features(evidence("2аф", "2fa", 1, "я ", "ввел", "kept_next_word"))
        self.assertIn(ALNUM_FEATURE_PREFIX + "after:kept_next_word:ru", kept)
        self.assertTrue(any(name.startswith(KEPT_FEATURE_PREFIX) for name in kept))

    def test_a_model_with_the_head_answers_its_class(self) -> None:
        bias = (0.0,) * len(ACTIONS)
        self.assertTrue(ContextModel({ALNUM_FEATURE_PREFIX + "bias": bias}, "context-v3-head",
                                     feature_version=CONTEXT_ACTION_FEATURE_VERSION).answers_alnum)
        self.assertFalse(ContextModel({"bias": bias}, "context-v3-base", feature_version=CONTEXT_ACTION_FEATURE_VERSION).answers_alnum)


class AlnumHeadEngineTests(ContextEngineTests):
    """A model with the letters-and-digits head has learned a token of letters and digits: there its verdict stands."""

    def setUp(self) -> None:
        super().setUp()
        self.model = ScriptedActionModel(alnum_converted)
        self.engine.context_policy.model = self.model

    def test_a_token_of_one_letter_and_digits_follows_a_model_that_learned_it(self) -> None:
        self.model.answers_alnum = True
        self.reset_editor(1)
        self.type("собрал под ч64 ", group=1)
        self.assertEqual(self.backend.text, "собрал под x64 ")

    def test_without_the_head_the_lone_letter_refusal_keeps_it(self) -> None:
        self.reset_editor(1)
        self.type("собрал под ч64 ", group=1)
        self.assertEqual(self.backend.text, "собрал под ч64 ")


class AlnumCurriculumTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        data = self.root / "questions.jsonl.xz"
        data.write_bytes(lzma.compress("".join((
            captured_line(1, "ч64", "x64", 1, "convert", "собрал под "), captured_line(1, "1с", "1c", 1, "keep", "работаю в "),
            captured_line(1, "1с", "1c", 1, "keep", "версия "), captured_line(1, "ghbdtn", "привет", 0, "convert", "я "),
            captured_line(1, "м67", "v67", 1, "wait", "или "), captured_line(1, "x64", "ч64", 0, "keep", "под "),
            captured_line(1, "У17", "E17", 1, "convert", "richest the "))).encode()))
        manifest = self.root / "manifest.json"
        manifest.write_text(json.dumps({"schema_version": 1, "columns": list(CAPTURED_COLUMNS),
                                        "sources": [{"file": data.name, "sha256": hashlib.sha256(data.read_bytes()).hexdigest(),
                                                     "weight": 1.0}]}), encoding="utf-8")
        self.options = {"captured": {TRAIN: [[str(manifest), data.name]]}, "sample_weight": 1.0}

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def test_the_head_takes_the_captured_questions_of_its_class(self) -> None:
        rows, report = trainer.alnum_curriculum(TRAIN, [], frozenset(), self.options)
        self.assertEqual([(row.original, row.action, row.category) for row in rows],
                         [("ч64", "convert", "alnum"), ("1с", "keep", "alnum"), ("1с", "keep", "alnum")])
        self.assertEqual(report["frames"], len(rows))
        self.assertNotIn("capped", report)

    def test_a_recipe_caps_the_frames_of_one_token_and_weighs_the_converted_ones(self) -> None:
        options = {**self.options, "token_cap": 1, "convert_weight": ALNUM_FIXTURE_CONVERT_WEIGHT}
        rows, report = trainer.alnum_curriculum(TRAIN, [], frozenset(), options)
        self.assertEqual(sorted((row.original, row.action, row.sample_weight) for row in rows),
                         [("1с", "keep", 1.0), ("ч64", "convert", ALNUM_FIXTURE_CONVERT_WEIGHT)])
        self.assertEqual((report["frames"], report["capped"], report["token_cap"], report["convert_weight"]),
                         (len(rows), 1, 1, ALNUM_FIXTURE_CONVERT_WEIGHT))
        self.assertEqual(trainer.alnum_curriculum(DEVELOPMENT, [], frozenset(), self.options), ([], {"frames": 0, "scope": "not used"}))

    def test_the_head_is_fitted_onto_a_frozen_base_with_an_epoch_of_its_own(self) -> None:
        self.assertEqual(trainer.HEAD_PREFIXES["alnum"], ALNUM_FEATURE_PREFIX)
        self.assertIn(ALNUM_FEATURE_PREFIX, trainer.OWN_EPOCH_HEADS)
        heads = frozenset({ALNUM_FEATURE_PREFIX})
        self.assertTrue(trainer.head_question("зь2", "pm2", "или ", "", "space", "none", heads))
        self.assertTrue(trainer.head_question("зь2", "pm2", "или ", "как", "space", "kept_next_word", heads))
        self.assertFalse(trainer.head_question("зь2", "pm2", "или ", "", "space", "none", frozenset()))
        self.assertFalse(trainer.head_question("м67", "v67", "или ", "", "space", "none", heads))
        self.assertFalse(trainer.head_question("2х", "2[", "или ", "", "space", "none", heads))
        self.assertFalse(trainer.head_question("v67", "м67", "или ", "", "space", "none", heads))
        self.assertFalse(trainer.head_question("У17", "E17", "richest the ", "", "space", "none", heads))
        try:
            import context_action_cuda as cuda
        except ImportError:  # CuPy missing, as in CI: the GPU path does not load here
            self.skipTest("CuPy is required to load the GPU path")
        self.assertTrue(cuda.heads_question("зь2", "или ", "", "space"))
        self.assertFalse(cuda.heads_question("У17", "richest the ", "", "space"))


class AlnumCaptureSelectionTests(unittest.TestCase):
    def test_a_capture_keeps_the_questions_of_the_class(self) -> None:
        self.assertTrue(mixed_typing.CAPTURED_QUESTIONS["alnum"]("ч64", "x64", "под ", "space"))
        self.assertFalse(mixed_typing.CAPTURED_QUESTIONS["alnum"]("2х", "2[", "в ", "space"))
        self.assertFalse(mixed_typing.CAPTURED_QUESTIONS["alnum"]("x64", "ч64", "под ", "space"))

    def test_the_table_of_whole_tokens_counts_the_training_messages_but_the_developer_chat_questions(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            members = root / "devchat-dev.members.jsonl.gz"
            texts = {CAPTURE_FIXTURE_LEFT_OUT_QUESTION: "версия 1с и x64", CAPTURE_FIXTURE_QUESTIONS[0]: "в 1с, x64 (и v2)",
                     CAPTURE_FIXTURE_QUESTIONS[1]: "работает 2х слово"}
            with gzip.open(root / "so-train.jsonl.gz", "wt", encoding="utf-8") as handle:
                for question, text in texts.items():
                    for _copy in range(ALNUM_FIXTURE_TOKEN_COUNT):
                        handle.write(json.dumps({"qid": question, "kind": "comment", "mix": "mixed", "text": text}, ensure_ascii=False) + "\n")
            with gzip.open(members, "wt", encoding="utf-8") as handle:
                handle.write(json.dumps({"text": "", "question": CAPTURE_FIXTURE_LEFT_OUT_QUESTION, "mix": "mixed"}) + "\n")
            table = mixed_typing.build_alnum(root, [members])
            self.assertEqual(table, {"cyrillic": {"1с": ALNUM_FIXTURE_TOKEN_COUNT, "2х": ALNUM_FIXTURE_TOKEN_COUNT},
                                     "latin": {"v2": ALNUM_FIXTURE_TOKEN_COUNT, "x64": ALNUM_FIXTURE_TOKEN_COUNT}})

    def test_messages_are_chosen_by_a_token_of_the_class_in_either_script(self) -> None:
        for token in ("x64,", "(v2)", "«1с»", "3д.", "100мс"):
            with self.subTest(token=token):
                self.assertTrue(selection.alnum_token("", token))
        for token in ("2х", "-v2", "x", "64", "utf-8"):
            with self.subTest(token=token):
                self.assertFalse(selection.alnum_token("", token))
        self.assertTrue(selection.latin_alnum_token("", "x64") and selection.russian_alnum_token("", "1с"))
        self.assertFalse(selection.latin_alnum_token("", "1с") or selection.russian_alnum_token("", "x64"))
        self.assertEqual(selection.EXCLUDED_BY["alnum-russian"], "alnum-latin")


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    # Each class's own tests alone: the engine tests' base holds tests of its own, run in its module.
    return unittest.TestSuite(case(name) for case in (
        AlnumClassTests, AlnumHeadEngineTests, AlnumCurriculumTests, AlnumCaptureSelectionTests)
        for name in case.__dict__ if name.startswith("test_"))
