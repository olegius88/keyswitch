"""A waiting word asked once more beside a next word that stayed as typed.

`зк` amid Russian prose waits: two letters tell little, and the pair converts only when the next
word converts too. A term typed in the Russian layout has a next word that is right as typed
(`есть новые зк проверь`), so the engine asks the model about `зк` once more with `проверь` after
it, a `kept_next_word` question; a conversion replaces `зк` alone, `проверь` is typed again in its
own layout, and the layout stays there. The engine only asks; the model decides.
"""

from __future__ import annotations

import unittest
from dataclasses import replace

from keyswitch.constants.keyboard import ALT_MASK, CONTROL_MASK
from keyswitch.constants.models import CONTEXT_ACTION_FEATURE_VERSION
from keyswitch.context_model import FEATURE_VERSION, ContextAction, ContextEvidence
from test_context_policy import ContextEngineTests
from test_context_wait_pairs import ScriptedModel


def term_amid_russian(item: ContextEvidence) -> ContextAction:
    """`зк` and `зкщсуыы` wait at their space; beside a kept `проверь` they convert."""

    if item.original in {"зк", "зкщсуыы"}:
        return "convert" if item.after_origin == "kept_next_word" and item.field.after == "проверь" else "suggest"
    return "keep"


class KeptNeighbourTests(ContextEngineTests):
    def script(self, answer: object = term_amid_russian, version: int = CONTEXT_ACTION_FEATURE_VERSION) -> ScriptedModel:
        model = ScriptedModel(answer)  # type: ignore[arg-type]
        model.feature_version = version
        self.engine.context_policy.model = model
        return model

    def kept_questions(self, model: ScriptedModel) -> list[tuple[str, str, str]]:
        return [(original, before, after) for original, before, after in model.questions if after]

    def test_a_term_amid_russian_converts_alone_and_the_layout_stays(self) -> None:
        model = self.script()
        self.reset_editor(1)
        self.type("есть новые зк проверь ", group=1)
        self.assertEqual(self.backend.text, "есть новые pr проверь ")
        self.assertEqual((self.backend.group, self.engine.snapshot.current_group), (1, 1))
        self.assertEqual(self.backend.kept_tails[-1], len("проверь"))
        self.assertIn(("зк", "есть новые ", "проверь"), model.questions)
        correction = self.engine._last_correction
        assert correction is not None
        self.assertEqual((correction.mode, correction.kept_tail), ("context_word", len("проверь")))
        self.type("их ", group=1)
        self.assertEqual(self.backend.text, "есть новые pr проверь их ")

    def test_undo_restores_both_words_as_typed(self) -> None:
        self.script()
        self.reset_editor(1)
        self.type("есть зк проверь ", group=1)
        self.assertEqual(self.backend.text, "есть pr проверь ")
        self.tap(replace(self.key("z"), state=CONTROL_MASK | ALT_MASK))
        self.assertEqual(self.backend.text, "есть зк проверь ")

    def test_a_declining_model_leaves_both_words(self) -> None:
        model = self.script(lambda item: "suggest" if item.original == "зк" else "keep")
        self.reset_editor(1)
        self.type("есть зк проверь ", group=1)
        self.assertEqual(self.backend.text, "есть зк проверь ")
        self.assertIn(("зк", "есть ", "проверь"), self.kept_questions(model))

    def test_a_longer_waiting_word_is_not_asked_again(self) -> None:
        model = self.script()
        self.reset_editor(1)
        self.type("есть зкщсуыы проверь ", group=1)
        self.assertEqual(self.backend.text, "есть зкщсуыы проверь ")
        self.assertEqual(self.kept_questions(model), [])

    def test_a_model_of_an_older_schema_is_not_asked(self) -> None:
        model = self.script(version=FEATURE_VERSION)
        self.reset_editor(1)
        self.type("есть зк проверь ", group=1)
        self.assertNotIn(("зк", "есть ", "проверь"), model.questions)

    def test_a_next_word_ended_by_a_sign_is_not_a_kept_neighbour(self) -> None:
        model = self.script()
        self.reset_editor(1)
        self.type("есть зк проверь, ", group=1)
        self.assertEqual(self.backend.text, "есть зк проверь, ")
        self.assertNotIn(("зк", "есть ", "проверь"), model.questions)

    def test_an_expired_wait_is_not_asked_again(self) -> None:
        model = self.script()
        self.reset_editor(1)
        self.type("есть зк ", group=1)
        waiting = self.engine._context_waiting
        assert waiting is not None
        self.engine._context_waiting = replace(waiting, deadline=0.0)
        self.type("проверь ", group=1)
        self.assertEqual(self.backend.text, "есть зк проверь ")
        self.assertNotIn(("зк", "есть ", "проверь"), model.questions)

    def test_a_changed_text_is_not_rewritten(self) -> None:
        self.script()
        self.reset_editor(1)
        self.type("есть зк ", group=1)
        self.engine.context_policy.stream.clear()
        self.type("проверь ", group=1)
        self.assertEqual(self.backend.text, "есть зк проверь ")


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    suite = unittest.TestSuite()
    for name in KeptNeighbourTests.__dict__:
        if name.startswith("test_"):
            suite.addTest(KeptNeighbourTests(name))
    return suite


if __name__ == "__main__":
    unittest.main()
