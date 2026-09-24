"""A waiting word is decided together with the word after it, whichever of them is in doubt.

`tot` at the start of a message may be the English word or `еще` typed in the English
layout, and the model asks the engine to wait for the next word. When that word converts,
the waiting word is decided with it. When it does not, it may look English only because
of the word in doubt in front of it: `d` after `tot` is a lone letter, after `еще` it is
the preposition `в`. The model is then asked about the next word once more after the
waiting word's other reading, and the pair converts only if it says so for both words.

These cases live outside `test_context_policy` on purpose: that module's bytes are pinned by
the context-action release receipt.
"""

from __future__ import annotations

import unittest
from collections.abc import Callable
from unittest.mock import patch

from keyswitch.context_model import ACTIONS, ContextAction, ContextEvidence, ContextModel, ContextPrediction
from keyswitch.input_context import CONTEXT_TTL, FieldContext
from test_context_policy import ContextEngineTests
from fixture_values.clock import (
    LAST_WORD_INPUT_AT_SECONDS,
    WAIT_PAIR_FIRST_TYPING_SECONDS,
    WAIT_PAIR_PAUSE_CHECK_SECONDS,
    WAIT_PAIR_SECOND_TYPING_WITHIN_TTL_SECONDS,
)
from fixture_values.counts import WAIT_PAIR_CONTEXT_SUFFIX_CHARACTERS


class ScriptedModel(ContextModel):
    """Answers each question by what it is asked about, and records the questions."""

    def __init__(self, answer: Callable[[ContextEvidence], ContextAction], *, supported: bool = True) -> None:
        super().__init__({"bias": (0.0, 0.0, 0.0, 0.0)}, "context-v1-scripted")
        self.answer = answer
        # False: the application and the neighbour words are outside the model's vocabulary.
        self.supported = supported
        self.questions: list[tuple[str, str, str]] = []

    def predict(self, item: ContextEvidence) -> ContextPrediction:
        action = self.answer(item)
        self.questions.append((item.original, item.field.before, item.field.after))
        probabilities = tuple(1.0 if name == action else 0.0 for name in ACTIONS)
        return ContextPrediction(action, 1.0, probabilities, self.version, self.supported)


def before_greeting(item: ContextEvidence) -> ContextAction:
    """`tot` waits and follows `привет`, which converts on its own."""

    if item.original == "tot":
        return "convert" if item.field.after == "привет" else "wait"
    return "convert"


def after_reading(item: ContextEvidence) -> ContextAction:
    """The expected judgement: `tot` waits and follows a Russian neighbour; `d` is `в` after `еще`."""

    if item.original == "tot":
        return "convert" if item.field.after == "в" else "wait"
    return "convert" if item.field.before.endswith("еще ") else "keep"


class ContextWaitPairTests(ContextEngineTests):
    def script(self, answer: Callable[[ContextEvidence], ContextAction], *, supported: bool = True) -> ScriptedModel:
        model = ScriptedModel(answer, supported=supported)
        self.engine.context_policy.model = model
        return model

    def test_a_word_longer_than_two_letters_waits_when_the_model_asks(self) -> None:
        self.script(lambda _item: "wait")
        self.type("tot ")
        waiting = self.engine._context_waiting
        assert waiting is not None
        self.assertEqual(waiting.plan.original, "tot")
        self.assertEqual(self.backend.text, "tot ")

    def test_the_next_word_is_asked_again_after_the_waiting_words_other_reading(self) -> None:
        model = self.script(after_reading)
        self.type("tot d ")
        self.assertEqual(self.backend.text, "еще в ")
        self.assertIsNone(self.engine._context_waiting)
        asked = [
            (original, before[-WAIT_PAIR_CONTEXT_SUFFIX_CHARACTERS:], after)
            for original, before, after in model.questions
        ]
        # As typed, then after `еще`, then the waiting word with its converted neighbour.
        self.assertEqual(asked[1:], [("d", "tot ", ""), ("d", "еще ", ""), ("tot", "", "в")])

    def test_a_next_word_kept_after_either_reading_leaves_both_words(self) -> None:
        """`tot is` stays English: `is` is not converted after `еще` either."""
        model = self.script(lambda item: "wait" if item.original == "tot" else "keep")
        self.type("tot is ")
        self.assertEqual(self.backend.text, "tot is ")
        self.assertIsNone(self.engine._context_waiting)
        self.assertIsNone(self.engine._pending)
        self.assertIn(
            ("is", "еще "),
            [(original, before[-WAIT_PAIR_CONTEXT_SUFFIX_CHARACTERS:]) for original, before, _after in model.questions],
        )
        # The waiting word is not asked again: nothing converted after it.
        self.assertEqual([original for original, _before, _after in model.questions].count("tot"), 1)

    def test_the_waiting_word_still_has_the_last_say(self) -> None:
        """The next word converting after `еще` is not enough: `tot` itself must follow it."""
        self.script(lambda item: "keep" if item.original == "tot" and item.field.after else after_reading(item))
        self.type("tot d ")
        self.assertEqual(self.backend.text, "tot d ")
        self.assertIsNone(self.engine._pending)

    def test_a_wait_lasts_as_long_as_the_context_it_waits_in(self) -> None:
        """`tot`, twelve seconds of thought, then `d`: still one phrase (0.31.0 log, 24.09.2026)."""
        self.script(after_reading)
        with patch("keyswitch.engine.time.monotonic", return_value=WAIT_PAIR_FIRST_TYPING_SECONDS):
            self.type("tot ")
        with patch(
            "keyswitch.engine.time.monotonic", return_value=WAIT_PAIR_SECOND_TYPING_WITHIN_TTL_SECONDS
        ):
            self.type("d ")
        self.assertEqual(self.backend.text, "еще в ")

    def test_a_wait_ends_with_the_context_it_waits_in(self) -> None:
        """Once the context has lapsed `tot` stays, and `d` is judged as a letter on its own."""
        model = self.script(after_reading)
        with patch("keyswitch.engine.time.monotonic", return_value=WAIT_PAIR_FIRST_TYPING_SECONDS):
            self.type("tot ")
        with patch(
            "keyswitch.engine.time.monotonic", return_value=WAIT_PAIR_FIRST_TYPING_SECONDS + CONTEXT_TTL + 1.0
        ):
            self.type("d ")
        self.assertEqual(self.backend.text, "tot в ")
        self.assertNotIn(
            "еще ", [before[-WAIT_PAIR_CONTEXT_SUFFIX_CHARACTERS:] for _original, before, _after in model.questions]
        )

    def test_the_models_verdict_on_the_pair_stands_outside_the_applications_it_knows(self) -> None:
        """Firefox is not in the model's vocabulary: it still waited on `tot`, then threw its
        own verdict away and `tot привет` stayed (0.31.0 and 0.31.1 logs, 24.09.2026)."""
        self.script(before_greeting, supported=False)
        self.type("tot ghbdtn ")
        self.assertEqual(self.backend.text, "еще привет ")

    def test_outside_its_vocabulary_the_model_decides_only_with_a_planned_neighbour(self) -> None:
        """Asked about `tot` before the `привет` the engine converted, the model's verdict
        stands; asked about the same word with no such neighbour, the baseline decides."""
        self.script(lambda _item: "convert", supported=False)
        baseline = self.engine._planned_baseline("tot", {1: "еще"}, 0, "", 1)
        self.assertFalse(baseline.should_convert)
        field = FieldContext("firefox", "1", "", "")

        def decide(planned: bool) -> bool:
            return self.engine.context_policy.decide(
                baseline, "еще", 1, self.engine.detector, "space", "assist",
                after="привет", field_override=field, planned_context=planned,
            ).decision.should_convert

        self.assertEqual((decide(True), decide(False)), (True, False))

    def test_a_next_word_converted_at_a_pause_takes_the_waiting_word_with_it(self) -> None:
        """`tot`, then `ghbdtn` and a pause before the space: `привет` used to convert alone,
        switch the layout and leave `tot` behind when the space ended the wait."""
        self.script(before_greeting)
        self.type("tot ghbdtn")
        self.engine._last_word_input_at = LAST_WORD_INPUT_AT_SECONDS
        self.engine._maybe_correct_after_pause(now=WAIT_PAIR_PAUSE_CHECK_SECONDS)
        self.assertEqual(self.backend.text, "еще привет")
        self.assertIsNone(self.engine._context_waiting)

    def test_a_next_word_with_literal_punctuation_converts_alone_at_a_pause(self) -> None:
        """As at a space, punctuation typed against the next word ends the wait: the pair is
        not rewritten across a literal the user put there."""
        self.script(before_greeting)
        self.type("tot ghbdtn.")
        self.engine._last_word_input_at = LAST_WORD_INPUT_AT_SECONDS
        self.engine._maybe_correct_after_pause(now=WAIT_PAIR_PAUSE_CHECK_SECONDS)
        self.assertEqual(self.backend.text, "tot привет.")
        self.assertIsNone(self.engine._context_waiting)

    def test_a_suggested_word_follows_its_converted_neighbour(self) -> None:
        """`vs` the model was not sure of becomes `мы` once `хотим` is typed in the same layout."""
        def answer(item: ContextEvidence) -> ContextAction:
            if item.original == "vs":
                return "convert" if item.field.after == "хотим" else "suggest"
            return "convert"
        self.script(answer)
        self.type("vs [jnbv ")
        self.assertEqual(self.backend.text, "мы хотим ")

    def test_a_suggested_word_is_not_decided_again_at_a_pause(self) -> None:
        """A pause brings no new context, so the model's hesitation stands."""
        self.script(lambda _item: "suggest")
        self.type("vs ")
        waiting = self.engine._context_waiting
        assert waiting is not None
        self.assertFalse(waiting.settles_on_pause)
        self.engine._last_word_input_at = LAST_WORD_INPUT_AT_SECONDS
        self.choose("convert")
        self.engine._maybe_correct_after_pause(now=WAIT_PAIR_PAUSE_CHECK_SECONDS)
        self.assertIsNone(self.engine._context_waiting)
        self.assertIsNone(self.engine._pending)
        self.assertEqual(self.backend.text, "vs ")


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    suite = unittest.TestSuite()
    for name in ContextWaitPairTests.__dict__:
        if name.startswith("test_"):
            suite.addTest(ContextWaitPairTests(name))
    return suite


if __name__ == "__main__":
    unittest.main()
