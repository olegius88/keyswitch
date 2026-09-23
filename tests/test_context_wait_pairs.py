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

from keyswitch.context_model import ACTIONS, ContextAction, ContextEvidence, ContextModel, ContextPrediction
from test_context_policy import ContextEngineTests


class ScriptedModel(ContextModel):
    """Answers each question by what it is asked about, and records the questions."""

    def __init__(self, answer: Callable[[ContextEvidence], ContextAction]) -> None:
        super().__init__({"bias": (0.0, 0.0, 0.0, 0.0)}, "context-v1-scripted")
        self.answer = answer
        self.questions: list[tuple[str, str, str]] = []

    def predict(self, item: ContextEvidence) -> ContextPrediction:
        action = self.answer(item)
        self.questions.append((item.original, item.field.before, item.field.after))
        probabilities = tuple(1.0 if name == action else 0.0 for name in ACTIONS)
        return ContextPrediction(action, 1.0, probabilities, self.version, True)


def after_reading(item: ContextEvidence) -> ContextAction:
    """The expected judgement: `tot` waits and follows a Russian neighbour; `d` is `в` after `еще`."""

    if item.original == "tot":
        return "convert" if item.field.after == "в" else "wait"
    return "convert" if item.field.before.endswith("еще ") else "keep"


class ContextWaitPairTests(ContextEngineTests):
    def script(self, answer: Callable[[ContextEvidence], ContextAction]) -> ScriptedModel:
        model = ScriptedModel(answer)
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
        asked = [(original, before[-4:], after) for original, before, after in model.questions]
        # As typed, then after `еще`, then the waiting word with its converted neighbour.
        self.assertEqual(asked[1:], [("d", "tot ", ""), ("d", "еще ", ""), ("tot", "", "в")])

    def test_a_next_word_kept_after_either_reading_leaves_both_words(self) -> None:
        """`tot is` stays English: `is` is not converted after `еще` either."""
        model = self.script(lambda item: "wait" if item.original == "tot" else "keep")
        self.type("tot is ")
        self.assertEqual(self.backend.text, "tot is ")
        self.assertIsNone(self.engine._context_waiting)
        self.assertIsNone(self.engine._pending)
        self.assertIn(("is", "еще "), [(original, before[-4:]) for original, before, _after in model.questions])
        # The waiting word is not asked again: nothing converted after it.
        self.assertEqual([original for original, _before, _after in model.questions].count("tot"), 1)

    def test_the_waiting_word_still_has_the_last_say(self) -> None:
        """The next word converting after `еще` is not enough: `tot` itself must follow it."""
        self.script(lambda item: "keep" if item.original == "tot" and item.field.after else after_reading(item))
        self.type("tot d ")
        self.assertEqual(self.backend.text, "tot d ")
        self.assertIsNone(self.engine._pending)

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
        self.engine._last_word_input_at = 100.0
        self.choose("convert")
        self.engine._maybe_correct_after_pause(now=102.0)
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
