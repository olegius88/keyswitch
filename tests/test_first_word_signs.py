r"""The first word of a message typed in the wrong layout is settled by the words after it: a word whose last key is a
letter of the other layout (`T\`` is `Её`, `dc.` is `всю`), a waiting word whose neighbour converts with a sign split
off (`Ша тщеб` is `If not,`), and an interjection the shipped lexicon does not hold (`jq` is `ой`). The development
sentences of the Tatoeba dump (22.10.2026) lost these to the frozen baseline pair."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

from keyswitch.engine import CorrectionPlan  # noqa: E402
from test_default_input_sequences import DefaultSequenceHarness  # noqa: E402


class FirstWordSignTests(DefaultSequenceHarness):
    def typed(self, application: str, group: int, steps: tuple[tuple[str, int], ...]) -> str:
        with self.session(group, application) as session:
            for text, typed_group in steps:
                session.type(text, typed_group, idle_after_words=False)
            session.idle()
            return session.backend.text

    def test_a_first_word_whose_last_key_is_a_letter_converts_with_its_converted_neighbour(self) -> None:
        # The backtick is the key of `ё` and `.` that of `ю`: the boundary model splits the sign off or cannot tell, the
        # token waits for its neighbour, and the model's suggestion is confirmed by the orthotactic model.
        self.assertEqual(self.typed("Telegram", 0, (("Её ", 1), ("платье ", 1), ("попало ", 1))), "Её платье попало ")
        self.assertEqual(self.typed("Telegram", 0, (("всю ", 1), ("неделю ", 1), ("светит ", 1))), "всю неделю светит ")
        self.assertEqual(self.typed("Telegram", 0, (("всё ", 1), ("хорошо ", 1))), "всё хорошо ")

    def test_an_english_word_with_a_sign_stays_before_a_converted_russian_word(self) -> None:
        for first in ("ok. ", "it. ", "So, "):
            with self.subTest(first=first):
                self.assertEqual(self.typed("Telegram", 0, ((first, 0), ("привет ", 1))), first + "привет ")

    def test_a_waiting_first_word_converts_with_a_neighbour_whose_sign_was_split_off(self) -> None:
        self.assertEqual(self.typed("Telegram", 1, (("If ", 0), ("not, ", 0), ("you ", 0))), "If not, you ")

    def test_a_kept_word_is_asked_when_prose_uses_its_other_reading(self) -> None:
        # The shipped lexicon holds no `ой`; Russian prose uses it 1 298 times.
        self.assertEqual(self.typed("Telegram", 0, (("ой ", 1), ("я ", 1), ("разве ", 1))), "ой я разве ")

    def test_without_an_orthotactic_model_no_suggestion_is_confirmed(self) -> None:
        with self.session(0, "Telegram") as session:
            plan = CorrectionPlan((), None, 0, 1, "t`", "её", 1.0, "Telegram", True)
            self.assertTrue(session.engine._keys_lean_to_target_language(plan, ""))
            with patch.object(session.engine.context_policy, "ortho", None):
                self.assertFalse(session.engine._keys_lean_to_target_language(plan, ""))


if __name__ == "__main__":
    unittest.main()
