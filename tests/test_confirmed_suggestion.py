"""A word the action model only suggests converting is converted when the detector converts it too.

The model's `suggest` leans to the other layout without the certainty of its threshold. A word no
lexicon holds, typed in the wrong layout (`byzpt` for `инязе`, `ntby` for `теин`), was left as typed
there, while the detector converted it and the baseline pair did too. Two verdicts of the same
direction settle it; a lone letter still needs a curated rule, and a keep or a wait stands.
"""

from __future__ import annotations

import unittest

from keyswitch.context_model import ContextAction, ContextEvidence
from keyswitch.context_policy import SUGGESTION_CONFIRMED_REASON
from test_context_policy import ContextEngineTests
from test_early_switch_settling import ScriptedActionModel


def suggest(item: ContextEvidence) -> ContextAction:
    return "suggest"


class ConfirmedSuggestionTests(ContextEngineTests):
    def setUp(self) -> None:
        super().setUp()
        self.engine.context_policy.model = ScriptedActionModel(suggest)

    def test_a_suggestion_the_detector_confirms_converts(self) -> None:
        self.reset_editor(0)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type("ghbdtn ")
        self.assertEqual(self.backend.text, "привет ")
        self.assertTrue(any(SUGGESTION_CONFIRMED_REASON in line for line in logs.output))

    def test_a_suggestion_the_detector_does_not_confirm_stays(self) -> None:
        # `hello` is a word as typed: the detector keeps it, and the suggestion changes nothing.
        self.reset_editor(0)
        self.type("hello ")
        self.assertEqual(self.backend.text, "hello ")

    def test_a_lone_letter_still_needs_a_curated_rule(self) -> None:
        self.reset_editor(1)
        self.type("привет ", group=1)
        self.backend.group = 0
        self.type("x ")
        self.assertEqual(self.backend.text, "привет x ")

    def test_a_keep_or_a_wait_stands(self) -> None:
        answers: tuple[ContextAction, ...] = ("keep", "wait")
        for action in answers:
            with self.subTest(action=action):
                def answer(item: ContextEvidence, chosen: ContextAction = action) -> ContextAction:
                    return chosen

                self.engine.context_policy.model = ScriptedActionModel(answer)
                self.reset_editor(0)
                self.type("ghbdtn ")
                self.assertEqual(self.backend.text, "ghbdtn ")


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    return unittest.TestSuite(ConfirmedSuggestionTests(name) for name in ConfirmedSuggestionTests.__dict__ if name.startswith("test_"))


if __name__ == "__main__":
    unittest.main()
