"""Words left as typed that a later converted word takes along.

The first words of a line have nothing before them to tell their layout by:
`hey here` typed in the Russian layout is `рун руку`, two Russian words. When
the next word, typed in the same layout, converts - `ерун` is `they` - the model
is asked about the words before it again with the converted word after them,
nearest first, and one correction takes along every word it now converts, as
long as they reach back to the start of the line. The engine only asks; the
model decides.
"""

from __future__ import annotations

import unittest

from keyswitch.context_model import ContextAction, ContextEvidence
from test_context_policy import ContextEngineTests
from test_context_wait_pairs import ScriptedModel, before_greeting
from test_inside_word import CaretReader
from fixture_values.clock import LAST_WORD_INPUT_AT_SECONDS, WAIT_PAIR_PAUSE_CHECK_SECONDS

# What each word is once the word after it is known: `руку` before `they` is `here`.
NEXT_WORD_TELLS = {"руку": "they", "рун": "here", "ура": "hey", "dc`": "тот", "t`": "мать", "kb[": "привет"}


def english_phrase(item: ContextEvidence) -> ContextAction:
    """Convert a word asked with the word that shows its layout after it, keep the rest."""

    if item.original in NEXT_WORD_TELLS:
        return "convert" if item.field.after == NEXT_WORD_TELLS[item.original] else "keep"
    return "convert" if item.original in {"ерун", "njn", "ghbdtn", "vfnm"} else "keep"


class KeptWordTests(ContextEngineTests):
    def setUp(self) -> None:
        super().setUp()
        self.settings.set("detection.context_read_field", True)
        self.engine.context_policy.reader = CaretReader(self)  # type: ignore[arg-type]

    def script(self, answer: object = english_phrase) -> ScriptedModel:
        model = ScriptedModel(answer)  # type: ignore[arg-type]
        self.engine.context_policy.model = model
        return model

    def test_the_next_word_takes_a_kept_word_along(self) -> None:
        model = self.script()
        self.reset_editor(1)
        self.type("руку ", group=1)
        self.assertEqual(self.backend.text, "руку ")
        self.type("ерун ", group=1)
        self.assertEqual(self.backend.text, "here they ")
        # The kept word was asked again with the converted word after it.
        self.assertIn(("руку", "", "they"), model.questions)

    def test_two_kept_words_are_taken_along_nearest_first(self) -> None:
        self.script()
        self.reset_editor(1)
        self.type("рун руку ерун ", group=1)
        self.assertEqual(self.backend.text, "hey here they ")

    def test_words_that_do_not_reach_the_start_of_the_line_are_left(self) -> None:
        # Only the nearest two words are remembered, and `рун` has `ура` before it on its line.
        self.script()
        self.reset_editor(1)
        self.type("ура рун руку ерун ", group=1)
        self.assertEqual(self.backend.text, "ура рун руку they ")

    def test_the_first_words_of_a_new_line_are_taken_along(self) -> None:
        self.script()
        self.reset_editor(1)
        self.backend.text = "Sounds good.\n"
        self.backend.caret = len(self.backend.text)
        self.type("руку ерун ", group=1)
        self.assertEqual(self.backend.text, "Sounds good.\nhere they ")

    def test_a_word_with_words_before_it_on_its_line_is_left(self) -> None:
        # It was decided with them: a Russian word typed as intended before a term typed
        # in the other layout (`склонируй репо пшерги`) looks just like this one.
        self.script()
        self.reset_editor(1)
        self.backend.text = "через "
        self.backend.caret = len(self.backend.text)
        self.type("руку ерун ", group=1)
        self.assertEqual(self.backend.text, "через руку they ")

    def test_a_word_whose_other_reading_is_no_word_is_not_asked(self) -> None:
        # `htop` stays at its space; `рещз` is no word, so `показывает` after it does not reopen it.
        model = self.script(lambda item: "keep" if item.original == "htop" and not item.field.after else "convert")
        self.reset_editor(0)
        self.type("htop gjrfpsdftn ")
        self.assertEqual(self.backend.text, "htop показывает ")
        self.assertNotIn(("htop", "", "показывает"), model.questions)

    def test_a_token_of_signs_is_not_taken_along(self) -> None:
        # `.` is `ю` in the other layout, but a token without a letter is never a word to convert.
        model = self.script(lambda item: "convert")
        self.reset_editor(0)
        self.type(". ghbdtn ")
        self.assertEqual(self.backend.text, ". привет ")
        self.assertNotIn((".", "", "привет"), model.questions)

    def test_a_word_the_model_still_keeps_stops_the_walk(self) -> None:
        self.script(lambda item: "convert" if item.original == "ерун" else "keep")
        self.reset_editor(1)
        self.type("рун руку ерун ", group=1)
        self.assertEqual(self.backend.text, "рун руку they ")

    def test_a_word_converted_into_the_other_language_takes_nothing_along(self) -> None:
        self.script()
        self.reset_editor(1)
        self.type("руку ", group=1)
        self.backend.group = 0
        self.type("ghbdtn ", group=0)
        self.assertEqual(self.backend.text, "руку привет ")

    def test_an_edit_between_the_words_takes_nothing_along(self) -> None:
        self.script()
        self.reset_editor(1)
        self.type("руку ф", group=1)
        self.tap(self.key("BackSpace", "", group=1))
        self.type("ерун ", group=1)
        self.assertEqual(self.backend.text, "руку they ")

    def test_a_reopened_word_is_kept_again_when_it_ends_again(self) -> None:
        # Backspace right after the space reopens the word; ending it again keeps it again.
        self.script()
        self.reset_editor(1)
        self.type("руку ", group=1)
        self.tap(self.key("BackSpace", "", group=1))
        self.type(" ерун ", group=1)
        self.assertEqual(self.backend.text, "here they ")

    def test_a_token_the_boundary_model_could_not_split_is_taken_along(self) -> None:
        model = self.script()
        self.reset_editor(0)
        self.type("dc` ")
        # Neither `всё` nor `вс` with a backtick: the token was not decided at all.
        self.assertNotIn(("dc`", "", ""), model.questions)
        self.type("njn ")
        self.assertEqual(self.backend.text, "всё тот ")

    def test_a_sign_split_off_a_letter_of_the_whole_word_is_taken_along_with_it(self) -> None:
        # The boundary model keeps the backtick of `t\`` literal; `е` alone is no word, `её` is.
        model = self.script()
        self.reset_editor(0)
        self.type("t` vfnm ")
        self.assertEqual(self.backend.text, "её мать ")
        self.assertIn(("t`", "", "мать"), model.questions)

    def test_a_split_sign_stands_when_the_word_without_it_is_a_word(self) -> None:
        # `kb[` is `ли` and a bracket to the boundary model; `ли` is a word, so `лих` is not asked about.
        model = self.script()
        self.reset_editor(0)
        self.type("kb[ ghbdtn ")
        self.assertEqual(self.backend.text, "kb[ привет ")
        self.assertNotIn(("kb[", "", "привет"), model.questions)

    def test_a_converted_word_with_a_sign_split_off_takes_a_kept_word_along(self) -> None:
        # The comma after `ерун` stays literal and ends the one correction.
        self.script()
        self.reset_editor(1)
        self.type("руку ерун, ", group=1)
        self.assertEqual(self.backend.text, "here they, ")

    def test_a_waiting_word_is_asked_with_a_neighbour_that_has_a_sign_split_off(self) -> None:
        # The comma ends the wait for `tot`, which is then asked as a kept word.
        model = self.script(before_greeting)
        self.reset_editor(0)
        self.type("tot ghbdtn, ")
        self.assertEqual(self.backend.text, "еще привет, ")
        self.assertIn(("tot", "", "привет"), model.questions)

    def test_nothing_is_taken_along_outside_assist(self) -> None:
        self.script()
        self.settings.set("detection.context_policy", "shadow")
        self.reset_editor(1)
        self.type("руку ерун ", group=1)
        self.assertTrue(self.backend.text.startswith("руку "))

    def test_a_word_converted_at_a_pause_takes_the_kept_word_along(self) -> None:
        self.script()
        self.reset_editor(1)
        self.type("руку ", group=1)
        self.type("ерун", group=1)
        self.engine._last_word_input_at = LAST_WORD_INPUT_AT_SECONDS
        self.engine._maybe_correct_after_pause(now=WAIT_PAIR_PAUSE_CHECK_SECONDS)
        self.assertEqual(self.backend.text, "here they")


class KeptWordModelTests(ContextEngineTests):
    """What the bundled model decides about the first words of a message once the next word is known."""

    def setUp(self) -> None:
        super().setUp()
        self.settings.set("detection.context_read_field", True)
        self.engine.context_policy.reader = CaretReader(self)  # type: ignore[arg-type]

    def typed(self, group: int, *words: str) -> str:
        self.reset_editor(group)
        for word in words:
            self.type(word + " ", group=self.backend.group)
        return self.backend.text

    def test_the_next_word_tells_the_first_one(self) -> None:
        # `где` is `ult`, an English token, until `будет` follows it; `here` is `руку` until `they`.
        self.assertEqual(self.typed(0, "ult", ",eltn"), "где будет ")
        self.assertEqual(self.typed(1, "руку", "ерун"), "here they ")

    def test_a_token_the_boundary_model_could_not_split_is_decided_with_the_next_word(self) -> None:
        self.assertEqual(self.typed(0, "dc`", "njn"), "всё тот ")
        self.assertEqual(self.typed(0, "levf.", "jy"), "думаю он ")

    def test_a_sign_split_off_the_first_word_is_decided_with_the_next_word(self) -> None:
        # `её` is `t\`` and a backtick to the boundary model; `е` alone is no word.
        self.assertEqual(self.typed(0, "t`", "vfnm"), "её мать ")

    def test_a_first_word_with_a_period_split_off_converts_at_its_own_space(self) -> None:
        # `jr.` is `ок` and a period: it converts before the next word, which is then typed
        # in the Russian layout already.
        self.assertEqual(self.typed(0, "jr.", "привет"), "ок. привет ")

    def test_a_clear_first_word_stays_before_a_word_of_the_other_language(self) -> None:
        self.assertEqual(self.typed(0, "ok", "ghbdtn"), "ok привет ")
        self.assertEqual(self.typed(1, "привет", "ерун"), "привет they ")


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    suite = unittest.TestSuite()
    for case in (KeptWordTests, KeptWordModelTests):
        for name in case.__dict__:
            if name.startswith("test_"):
                suite.addTest(case(name))
    return suite


if __name__ == "__main__":
    unittest.main()
