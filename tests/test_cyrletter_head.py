"""The Cyrillic-letter head (`d` for `в` in `разрабатывал и d документ`: a Latin letter whose Cyrillic reading is a Russian
word of one letter, typed after Russian text): its class and evidence, its routing in the trainer, the capture's messages,
typing modes and frames, and the engine's use of its verdict."""

from __future__ import annotations

import gzip
import json
import sys
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

import mixed_typing  # noqa: E402
import select_capture_messages as selection  # noqa: E402
import train_context_action_model as trainer  # noqa: E402
from keyswitch.constants.models import (  # noqa: E402
    CONTEXT_ACTION_FEATURE_VERSION, CYRLETTER_FEATURE_PREFIX, KEPT_FEATURE_PREFIX, LETTER_FEATURE_PREFIX,
)
from keyswitch.context_action_features import cyrletter_question, extract_action_features  # noqa: E402
from keyswitch.context_model import ACTIONS, AfterOrigin, ContextAction, ContextEvidence, ContextModel  # noqa: E402
from keyswitch.language_model import WordScore  # noqa: E402
from test_context_policy import ContextEngineTests  # noqa: E402
from test_context_training_captured import captured_question  # noqa: E402
from test_early_switch_settling import ScriptedActionModel  # noqa: E402
from test_variable_and_dotted_heads import evidence, prefixes  # noqa: E402

PLANNED: AfterOrigin = "planned_next_conversion"
KEPT: AfterOrigin = "kept_next_word"


class CyrletterClassTests(unittest.TestCase):
    def test_the_class_is_a_latin_letter_read_as_a_russian_word_of_one_letter_after_russian_text(self) -> None:
        for original, alternative, before, trigger in (("d", "в", "я долго разрабатывал и ", "space"),
                                                       ("b", "и", "посмотри ", "space"), ("C", "С", "язык ", "space"),
                                                       ("f", "а", "Привет, ", "punctuation"), ("z", "я", "ответ: 2 ", "space")):
            with self.subTest(original=original, before=before):
                self.assertTrue(cyrletter_question(original, alternative, before, trigger))
        # Nothing or Latin text before it, a letter that is no Russian word of one letter, a Cyrillic letter, two letters,
        # a digit, and a pause, where the letter may open a Latin word still being typed (`d` of `docker`).
        for original, alternative, before, trigger in (("d", "в", "", "space"), ("d", "в", "nats ", "space"),
                                                       ("x", "ч", "для ", "space"), ("в", "d", "и ", "space"),
                                                       ("db", "ви", "и ", "space"), ("2", "2", "и ", "space"),
                                                       ("d", "в", "и ", "pause")):
            with self.subTest(original=original, before=before, trigger=trigger):
                self.assertFalse(cyrletter_question(original, alternative, before, trigger))

    def test_the_head_reads_the_shared_features_again_and_its_own_evidence(self) -> None:
        item = replace(evidence("d", "в", 0, "я долго разрабатывал и ", "документ", PLANNED), source_known=True,
                       target_known=True, source_score=WordScore(0.0, True, 0, 0.0, spell_known=True))
        features = extract_action_features(item)
        self.assertEqual(prefixes(features), {CYRLETTER_FEATURE_PREFIX})
        shared = {name for name in features if "|" not in name}
        self.assertTrue(shared)
        self.assertTrue(all(CYRLETTER_FEATURE_PREFIX + name in features for name in shared if features[name]))
        for name in ("bias", "letter:d", "case:lower", f"after:{PLANNED}:ru", "boundary:space:", "code:0", "latin:0",
                     "digit:0", "spell:1:0", f"letter:d:after:{PLANNED}:ru", "case:lower:code:0", "word:и", "letter:d:word:и"):
            self.assertIn(f"{CYRLETTER_FEATURE_PREFIX}{name}:direction:0", features)
        for side in ("previous_word", "next_word"):
            self.assertTrue(any(name.startswith(f"{CYRLETTER_FEATURE_PREFIX}{side}:russian:") for name in features))
        # A capital after code-flavoured text ended by a sign, with nothing after it.
        capital = extract_action_features(evidence("C", "С", 0, "размер = 2 и язык=2 ", trigger="punctuation", boundary=","))
        for name in ("case:upper", "code:1", "latin:0", "digit:1", "boundary:punctuation:,", "after:none:none",
                     "next_word:none", "letter:c:case:upper"):
            self.assertIn(f"{CYRLETTER_FEATURE_PREFIX}{name}:direction:0", capital)
        # Asked beside a next word kept as typed, it reads the kept-neighbour features under its own names as well.
        kept = extract_action_features(evidence("d", "в", 0, "разрабатывал и ", "когда", KEPT))
        self.assertEqual(prefixes(kept), {KEPT_FEATURE_PREFIX, CYRLETTER_FEATURE_PREFIX})
        self.assertIn(f"{CYRLETTER_FEATURE_PREFIX}after:{KEPT}:ru:direction:0", kept)
        self.assertTrue(any(name.startswith(CYRLETTER_FEATURE_PREFIX + KEPT_FEATURE_PREFIX) for name in kept))
        # After a Latin word the single-letter head keeps its class.
        self.assertEqual(prefixes(extract_action_features(evidence("b", "и", 0, "nats ", "redis"))), {LETTER_FEATURE_PREFIX})

    def test_the_head_is_fitted_onto_a_frozen_base_with_an_epoch_of_its_own(self) -> None:
        self.assertEqual(trainer.HEAD_PREFIXES["cyrletter"], CYRLETTER_FEATURE_PREFIX)
        self.assertIn(CYRLETTER_FEATURE_PREFIX, trainer.OWN_EPOCH_HEADS)
        heads = frozenset({CYRLETTER_FEATURE_PREFIX})
        self.assertTrue(trainer.head_question("d", "в", "разрабатывал и ", "", "space", "none", heads))
        self.assertTrue(trainer.head_question("d", "в", "разрабатывал и ", "документ", "space", PLANNED, heads))
        self.assertTrue(trainer.head_question("d", "в", "разрабатывал и ", "когда", "space", KEPT, heads))
        self.assertFalse(trainer.head_question("d", "в", "nats ", "", "space", "none", heads))
        self.assertFalse(trainer.head_question("d", "в", "разрабатывал и ", "", "pause", "none", heads))
        try:
            import context_action_cuda as cuda
        except ImportError:  # CuPy missing, as in CI: the GPU path does not load here
            self.skipTest("CuPy is required to load the GPU path")
        self.assertTrue(cuda.heads_question("D", "витамин ", "", "space"))
        self.assertFalse(cuda.heads_question("q", "и ", "", "space"))


class CyrletterCaptureTests(unittest.TestCase):
    def test_a_capture_keeps_the_questions_of_the_class(self) -> None:
        question = mixed_typing.CAPTURED_QUESTIONS["cyrletter"]
        self.assertTrue(question(captured_question("d", "в", "разрабатывал и ")))
        self.assertFalse(question(captured_question("d", "в", "разрабатывал и ", trigger="pause")))
        self.assertFalse(question(captured_question("d", "в", "nats ")))

    def test_messages_hold_a_latin_letter_or_a_russian_word_of_one_letter_after_a_russian_word(self) -> None:
        for previous, token in (("язык", "C"), ("витамин", "D,"), ("функция", "f")):
            with self.subTest(token=token):
                self.assertTrue(selection.cyrletter_latin_token(previous, token))
        # A letter that reads as no Russian word, one after a Latin word, a Cyrillic letter.
        for previous, token in (("язык", "x"), ("use", "C"), ("язык", "с")):
            with self.subTest(previous=previous, token=token):
                self.assertFalse(selection.cyrletter_latin_token(previous, token))
        self.assertTrue(selection.letter_slippable("я долго думал и в итоге"))
        for text in ("и всё", "Hello и world", "я долго думал"):
            with self.subTest(text=text):
                self.assertFalse(selection.letter_slippable(text))
        self.assertIn("cyrletter-latin", selection.CLASSES)
        self.assertIn("cyrletter-russian", selection.MESSAGE_CLASSES)

    def test_the_letter_slip_lands_on_a_russian_word_of_one_letter(self) -> None:
        start, steps = mixed_typing.plan("я долго думал и в итоге".split(), mixed_typing.LETTER_SLIP_MODE, 1) or (0, [])
        self.assertEqual(start, 1)
        self.assertEqual([token for token, _intended, _selected, slipped in steps if slipped], ["в"])
        self.assertIsNone(mixed_typing.plan("я сегодня спешу домой".split(), mixed_typing.LETTER_SLIP_MODE, 0))
        # Slipped and switched back: the word after the letter is typed in the Russian layout, selected by hand.
        _start, back = mixed_typing.plan("я думал и в итоге".split(), mixed_typing.LETTER_SLIP_BACK_MODE, 1) or (0, [])
        self.assertEqual([(token, selected, slipped) for token, _intended, selected, slipped in back if token in ("и", "в", "итоге")],
                         [("и", None, False), ("в", 0, True), ("итоге", 1, False)])
        # A letter that ends the message has no word to switch back for.
        self.assertIsNone(mixed_typing.plan("я думал в".split(), mixed_typing.LETTER_SLIP_BACK_MODE, 0))
        # The unprotected mode makes every change by hand, as the correct one does.
        _start, unprotected = mixed_typing.plan("язык C это".split(), mixed_typing.UNPROTECTED_MODE, 0) or (0, [])
        self.assertEqual([selected for _token, _intended, selected, _slipped in unprotected], [None, 0, 1])

    def test_typing_jobs_take_the_new_modes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            messages = Path(directory) / "messages.jsonl.gz"
            with gzip.open(messages, "wt", encoding="utf-8") as handle:
                handle.write(json.dumps({"kind": "tatoeba", "text": "я думал и в итоге", "mix": "russian", "qid": 1},
                                        ensure_ascii=False) + "\n")
            recipe = {"seed": "s", "kinds": ["tatoeba"], "count": 1, "russian_share": 1.0, "messages": messages.name,
                      "modes": [mixed_typing.LETTER_SLIP_MODE, mixed_typing.UNPROTECTED_MODE]}
            jobs = mixed_typing.jobs_for(messages, recipe, None)
            self.assertEqual([mode for _index, _text, mode, _choice, _prefix in jobs],
                             [mixed_typing.LETTER_SLIP_MODE, mixed_typing.UNPROTECTED_MODE])
            with self.assertRaises(ValueError):
                mixed_typing.jobs_for(messages, {**recipe, "modes": ["sideways"]}, None)


def cyrletter(item: ContextEvidence) -> ContextAction:
    """`d` is kept at its space and converts once `ljrevtyn` (`документ`) after it is planned for conversion."""

    if item.original == "d":
        return "convert" if item.after_origin == "planned_next_conversion" and item.field.after == "документ" else "keep"
    return "convert" if item.original == "ljrevtyn" else "keep"


def letter_only(item: ContextEvidence) -> ContextAction:
    """`d` converts wherever it is asked, every other word stays."""

    return "convert" if item.original == "d" else "keep"


class CyrletterEngineTests(ContextEngineTests):
    """A Latin letter after Russian text that the model converts is converted by a model with the Cyrillic-letter head
    alone: at its space, and taken along mid-line once its next word converts."""

    def setUp(self) -> None:
        super().setUp()
        self.settings.set("detection.early_switch", False)

    def use(self, answer: object, *, head: bool) -> ScriptedActionModel:
        model = ScriptedActionModel(answer)  # type: ignore[arg-type]
        model.answers_cyrletter = head
        self.engine.context_policy.model = model
        return model

    def test_a_model_with_the_head_answers_its_class(self) -> None:
        bias = (0.0,) * len(ACTIONS)
        self.assertTrue(ContextModel({CYRLETTER_FEATURE_PREFIX + "bias": bias}, "context-v3-head",
                                     feature_version=CONTEXT_ACTION_FEATURE_VERSION).answers_cyrletter)
        self.assertFalse(ContextModel({"bias": bias}, "context-v3-base",
                                      feature_version=CONTEXT_ACTION_FEATURE_VERSION).answers_cyrletter)

    def type_into_russian_text(self) -> None:
        """`d ` typed into a field of Russian text the engine did not see typed, its last word in the application
        Latin: neither the message-start rule nor the context rule of one-letter words speaks (the owner's typing,
        30.10.2026)."""

        self.settings.set("detection.context_read_field", True)
        self.engine.context_policy.reader = mixed_typing.Reader(self.backend)
        self.reset_editor(0)
        self.type("ok ", group=0)
        self.backend.text = "я долго думал и "
        self.backend.caret = len(self.backend.text)
        self.type("d ", group=0)

    def test_the_letter_converts_at_its_space_on_the_heads_verdict(self) -> None:
        self.use(letter_only, head=True)
        self.type_into_russian_text()
        self.assertEqual(self.backend.text, "я долго думал и в ")

    def test_a_letter_the_head_keeps_at_its_space_waits_for_the_next_word(self) -> None:
        # Kept at its own space, the letter is asked again beside its next word: there the head converts it.
        model = self.use(cyrletter, head=True)
        self.type_into_russian_text()
        self.assertEqual(self.backend.text, "я долго думал и d ")
        self.assertIsNotNone(self.engine._context_waiting)
        self.assertEqual(self.engine._context_result.prediction.action if self.engine._context_result
                         and self.engine._context_result.prediction else None, "wait")
        self.assertIn(("d", "я долго думал и ", ""), model.questions)

    def test_without_the_head_a_lone_letter_stays(self) -> None:
        self.use(letter_only, head=False)
        self.type_into_russian_text()
        self.assertEqual(self.backend.text, "я долго думал и d ")

    def test_the_letter_is_taken_along_mid_line_once_its_next_word_converts(self) -> None:
        model = self.use(cyrletter, head=True)
        self.reset_editor(1)
        self.type("я долго думал и ", group=1)
        self.type("d ljrevtyn ", group=0)
        self.assertEqual(self.backend.text, "я долго думал и в документ ")
        self.assertIn(("d", "я долго думал и ", "документ"), model.questions)

    def test_without_the_head_the_letter_stays_beside_its_converted_word(self) -> None:
        self.use(cyrletter, head=False)
        self.reset_editor(1)
        self.type("я долго думал и ", group=1)
        self.type("d ljrevtyn ", group=0)
        self.assertEqual(self.backend.text, "я долго думал и d документ ")


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    suite = unittest.TestSuite()
    for case in (CyrletterClassTests, CyrletterCaptureTests, CyrletterEngineTests):
        for name in case.__dict__:
            if name.startswith("test_"):
                suite.addTest(case(name))
    return suite


if __name__ == "__main__":
    unittest.main()
