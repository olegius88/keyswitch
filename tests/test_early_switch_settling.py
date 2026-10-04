"""An early switch is provisional: the completed word, judged as typed, settles it.

The prefix model switches the layout on a prefix. At the word's boundary the completed word
is asked the question the switch answered too soon - does what was typed need the other
layout. The scripted models below answer by the reading they are asked about, so each test
states which judgement the engine must follow.
"""

from __future__ import annotations

import json
import unittest

from collections.abc import Callable

from keyswitch.constants.models import CONTEXT_ACTION_FEATURE_VERSION
from keyswitch.constants.settings_defaults import DEFAULT_EARLY_SWITCH_MIN_LENGTH
from keyswitch.context_model import ACTIONS, ContextAction, ContextEvidence, ContextModel, ContextPrediction
from keyswitch.input_context import FieldContext
from keyswitch.language_model import LanguageModel
from keyswitch.early_switch import PrefixIndex
from keyswitch.prefix_model import PrefixInput
from test_context_policy import ContextEngineTests

# A Russian-looking word no lexicon holds, the shape of a brand or slang word.
INVENTED_WORD = "шупшуп"


class ScriptedActionModel(ContextModel):
    """A feature-version-3 model, as shipped, that answers by the reading it is asked about."""

    def __init__(self, answer: Callable[[ContextEvidence], ContextAction]) -> None:
        super().__init__({"bias": (0.0,) * len(ACTIONS)}, "context-v3-scripted", feature_version=CONTEXT_ACTION_FEATURE_VERSION)
        self.answer = answer
        self.questions: list[tuple[str, str, str]] = []

    def predict(self, item: ContextEvidence) -> ContextPrediction:
        action = self.answer(item)
        self.questions.append((item.original, item.field.before, item.field.after))
        return ContextPrediction(action, 1.0, tuple(1.0 if name == action else 0.0 for name in ACTIONS), self.version, True)


class ScriptedPrefix:
    """A prefix model that answers every prefix the same way and records what it saw."""

    version = "prefix-v2-scripted"

    def __init__(self, action: ContextAction) -> None:
        self.action = action
        self.prefixes: list[str] = []

    def predict(self, item: PrefixInput, indexes: dict[int, PrefixIndex], models: dict[int, LanguageModel]) -> ContextPrediction:
        self.prefixes.append(item.original)
        return ContextPrediction(self.action, 1.0, tuple(1.0 if name == self.action else 0.0 for name in ACTIONS), self.version, True)


class EarlySwitchSettlingTests(ContextEngineTests):
    def setUp(self) -> None:
        super().setUp()
        self.settings.set("detection.early_switch", True)
        self.prefix = ScriptedPrefix("convert")
        self.engine.prefix_model = self.prefix  # type: ignore[assignment]

    def judge(self, typed: ContextAction, shown: ContextAction = "keep") -> ScriptedActionModel:
        """The context model's answer about the typed (Russian) and the shown (Latin) reading."""

        def answer(item: ContextEvidence) -> ContextAction:
            return typed if item.source_group == 1 else shown

        model = ScriptedActionModel(answer)
        self.engine.context_policy.model = model
        return model

    def type_keys(self, text: str, group: int) -> None:
        """Press the keys of `text` as written in `group`, whatever layout is active by then."""

        for character in text:
            shown = character if self.backend.group == group else self.pair.translate(
                character, "ru" if group == 1 else "us", "us" if group == 1 else "ru")
            self.tap(self.key("space" if shown == " " else shown, shown))

    def settled(self, logs: list[str]) -> list[dict[str, object]]:
        events = [json.loads(line.split("TECHNICAL ", 1)[1]) for line in logs if "TECHNICAL " in line]
        return [event for event in events if event["event"] == "early_switch_settled"]

    def test_a_word_the_model_keeps_as_typed_takes_the_early_switch_back(self) -> None:
        self.reset_editor(1)
        model = self.judge("keep")
        self.type_keys(INVENTED_WORD[:DEFAULT_EARLY_SWITCH_MIN_LENGTH], 1)
        # The prefix model switched at the shortest prefix it is asked about: it shows in Latin.
        self.assertEqual((self.backend.text, self.backend.group), ("iegi", 0))
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type_keys(INVENTED_WORD[DEFAULT_EARLY_SWITCH_MIN_LENGTH:] + " ", 1)
        self.assertEqual((self.backend.text, self.backend.group), (INVENTED_WORD + " ", 1))
        # Asked about the shown word first, then about the word as typed, in the same field.
        *_, shown, typed = model.questions
        self.assertEqual((shown[0], typed[0]), ("iegieg", INVENTED_WORD))
        self.assertEqual(typed[1:], shown[1:])
        [settled] = self.settled(logs.output)
        self.assertEqual((settled["verdict"], settled["origin_group"]), ("keep", 1))
        evaluation = next(json.loads(line.split("TECHNICAL ", 1)[1]) for line in logs.output
                          if '"event":"word_evaluation"' in line.replace(" ", ""))
        assert isinstance(evaluation["decision"], dict)
        self.assertEqual(evaluation["decision"]["reason"], "ранняя смена отменена: целое слово верно в набранной раскладке")
        self.assertIsNone(self.engine._early_switch_origin)

    def test_a_switch_that_produced_a_word_is_not_taken_back(self) -> None:
        # `руддщ` is `hello` typed in the Russian layout: a word of the lexicon stands even when
        # the context model, asked about the unknown typed reading alone, keeps it.
        self.reset_editor(1)
        self.judge("keep")
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type_keys("руддщ ", 1)
        self.assertEqual((self.backend.text, self.backend.group), ("hello ", 0))
        self.assertEqual(self.settled(logs.output), [])

    def test_a_word_the_model_converts_as_typed_keeps_the_early_switch(self) -> None:
        self.reset_editor(1)
        self.judge("convert")
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type_keys(INVENTED_WORD + " ", 1)
        self.assertEqual((self.backend.text, self.backend.group), ("iegieg ", 0))
        [settled] = self.settled(logs.output)
        self.assertEqual(settled["verdict"], "convert")

    def test_a_model_in_doubt_leaves_the_prefix_models_choice(self) -> None:
        for doubt in ("wait", "suggest"):
            with self.subTest(doubt=doubt):
                self.reset_editor(1)
                self.judge(doubt)
                with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                    self.type_keys(INVENTED_WORD + " ", 1)
                self.assertEqual(self.backend.text, "iegieg ")
                self.assertEqual(self.settled(logs.output)[0]["verdict"], doubt)

    def test_a_shown_word_the_model_converts_back_needs_no_second_question(self) -> None:
        self.reset_editor(1)
        model = self.judge("keep", shown="convert")
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type_keys(INVENTED_WORD + " ", 1)
        self.assertEqual((self.backend.text, self.backend.group), (INVENTED_WORD + " ", 1))
        self.assertEqual(self.settled(logs.output), [])
        self.assertEqual(model.questions[-1][0], "iegieg")

    def test_a_word_without_a_model_answer_is_not_settled(self) -> None:
        # The field changed behind the observer after the switch: the safety refusal has no
        # model prediction to settle with, and the engine edits nothing more.
        self.reset_editor(1)
        self.judge("keep")
        self.type_keys(INVENTED_WORD[:-1], 1)
        self.assertEqual(self.backend.group, 0)

        class StaleReader:
            status = "available"

            def read(self, application: str, window: int) -> FieldContext:
                return FieldContext(application, "1", "something else entirely", "", role="text", source="uia")

        self.settings.set("detection.context_read_field", True)
        self.engine.context_policy.reader = StaleReader()
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type_keys(INVENTED_WORD[-1] + " ", 1)
        self.assertEqual((self.backend.text, self.backend.group), ("iegieg ", 0))
        self.assertEqual(self.settled(logs.output), [])
        self.assertIn("текст активного поля изменился", "\n".join(logs.output))

    def test_a_sign_key_inside_the_prefix_no_longer_blocks_the_early_switch(self) -> None:
        # `,` is `б` in the Russian layout: `ghj,` may still end in a comma, `ghj,k` is a word.
        self.reset_editor(0)
        self.judge("convert")
        self.type_keys("ghj,", 0)
        self.assertEqual((self.backend.text, self.backend.group), ("ghj,", 0))
        self.assertNotIn("ghj,", self.prefix.prefixes)
        self.type_keys("k", 0)
        self.assertEqual((self.backend.text, self.backend.group), ("пробл", 1))
        self.assertEqual(self.prefix.prefixes[-1], "ghj,k")


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    suite = unittest.TestSuite()
    for name in EarlySwitchSettlingTests.__dict__:
        if name.startswith("test_"):
            suite.addTest(EarlySwitchSettlingTests(name))
    return suite


if __name__ == "__main__":
    unittest.main()
