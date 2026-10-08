"""A keep the context model is unsure of is a suggestion (ContextModel.predict, CONTEXT_DOUBTFUL_KEEP_PROBABILITY).

The owner's typing of 08.10.2026: `зк` after `принимай ` was kept at p=0.51 against 0.49 for `pr` and stayed as typed,
while the same model asked beside the next word converts it at p=1.00. A suggestion at a space makes the engine wait
for the next word and ask once more.
"""

from __future__ import annotations

import math
import unittest

from keyswitch.constants.models import CONTEXT_ACTION_FEATURE_VERSION
from keyswitch.context_model import ACTIONS, ContextEvidence, ContextModel
from keyswitch.input_context import FieldContext
from fixture_values.scores import DOMINANT_BIAS_WEIGHT, DOUBTFUL_KEEP_FIXTURE_PROBABILITIES, SURE_KEEP_FIXTURE_PROBABILITIES
from test_context_policy import ContextEngineTests


def model(keep: float, convert: float) -> ContextModel:
    """A schema-3 model whose keep and convert scores give these probabilities, waiting and suggesting never."""
    scores = (math.log(keep), math.log(convert), -DOMINANT_BIAS_WEIGHT, -DOMINANT_BIAS_WEIGHT)
    zero = (0.0,) * len(ACTIONS)
    return ContextModel({"bias": scores, "source:char:1:1:з": zero, "target:char:1:1:p": zero}, "context-v3-fixture",
                        feature_version=CONTEXT_ACTION_FEATURE_VERSION)


class DoubtfulKeepTests(unittest.TestCase):
    def test_a_keep_the_model_is_unsure_of_is_a_suggestion(self) -> None:
        item = ContextEvidence("зк", "pr", 1, FieldContext("Telegram", "test", "принимай ", "", "unknown"), "space",
                               boundary_text=" ")
        unsure = model(*DOUBTFUL_KEEP_FIXTURE_PROBABILITIES).predict(item)
        self.assertEqual(unsure.action, "suggest")
        self.assertTrue(unsure.supported)
        self.assertGreater(unsure.probabilities[ACTIONS.index("keep")], unsure.probabilities[ACTIONS.index("convert")])
        self.assertEqual(model(*SURE_KEEP_FIXTURE_PROBABILITIES).predict(item).action, "keep")


class InstalledDoubtfulKeepTests(ContextEngineTests):
    """The installed pair, typing on: the term waits for its next word and converts beside it."""

    def test_a_term_the_model_was_unsure_of_converts_beside_its_next_word(self) -> None:
        self.reset_editor(1)
        self.type("принимай зк и продолжай ", group=1)
        self.assertEqual(self.backend.text, "принимай pr и продолжай ")


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    suite = unittest.TestSuite()
    for case in (DoubtfulKeepTests, InstalledDoubtfulKeepTests):
        for name in case.__dict__:
            if name.startswith("test_"):
                suite.addTest(case(name))
    return suite


if __name__ == "__main__":
    unittest.main()
