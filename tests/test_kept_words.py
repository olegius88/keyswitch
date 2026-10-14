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

import test_input_sequence_matrix as sequences

from keyswitch.context_model import ContextAction, ContextEvidence
from keyswitch.learning import LearnedRule
from test_context_policy import ContextEngineTests
from test_context_wait_pairs import ScriptedModel, before_greeting
from test_early_switch_settling import ScriptedActionModel
from test_inside_word import CaretReader
from fixture_values.clock import LAST_WORD_INPUT_AT_SECONDS, WAIT_PAIR_PAUSE_CHECK_SECONDS
from fixture_values.keys import UNDO_TRIGGER_SENTINEL_KEYCODE
from test_engine_behaviour import plain_key
from disclosed_regressions import disclosed_schema3_pair_regression
from engine_models import share_engine_models

# What each word is once the word after it is known: `руку` before `they` is `here`.
NEXT_WORD_TELLS = {"руку": "they", "рун": "here", "ура": "hey", "dc`": "тот", "t`": "мать", "kb[": "привет"}


def setUpModule() -> None:
    share_engine_models()


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

    def test_a_sign_typed_the_same_in_either_layout_stands_between_kept_words(self) -> None:
        # `-` is `-` in either layout: asked for a wait, it has nothing to wait for, and `руку`
        # before it is asked with the converted word after the sign.
        model = self.script(lambda item: "wait" if item.original == "-" else english_phrase(item))
        self.reset_editor(1)
        self.type("руку - ерун ", group=1)
        self.assertEqual(self.backend.text, "here - they ")
        self.assertIn(("руку", "", "they"), model.questions)

    def test_a_passed_sign_stays_when_the_word_before_it_does(self) -> None:
        self.script(lambda item: "convert" if item.original == "ерун" else "keep")
        self.reset_editor(1)
        self.type("руку - ерун ", group=1)
        self.assertEqual(self.backend.text, "руку - they ")

    def test_two_signs_in_a_row_cut_the_kept_words_apart(self) -> None:
        model = self.script()
        self.reset_editor(1)
        self.type("руку - - ерун ", group=1)
        self.assertEqual(self.backend.text, "руку - - they ")
        self.assertNotIn(("руку", "", "they"), model.questions)

    def test_a_passed_sign_does_not_count_among_the_words_taken_along(self) -> None:
        self.script()
        self.reset_editor(1)
        self.type("рун - руку ерун ", group=1)
        self.assertEqual(self.backend.text, "hey - here they ")

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

    def test_a_token_the_boundary_model_could_not_split_is_taken_along_in_the_middle_of_a_line(self) -> None:
        # It was decided neither with the words before it nor at all: after `через` too,
        # `dc\`` is `всё` or `вс` with a backtick until the next word tells which.
        model = self.script()
        self.settings.set("detection.respect_manual_layout", False)
        self.reset_editor(1)
        self.type("через ", group=1)
        self.backend.group = 0
        self.type("dc` njn ")
        self.assertEqual(self.backend.text, "через всё тот ")
        self.assertIn(("dc`", "через ", "тот"), model.questions)

    def test_an_undecided_token_whose_other_reading_no_lexicon_holds_is_asked(self) -> None:
        # `мурзилках` is in no lexicon, but the orthotactic model reads `vehpbkrf[` as Russian keys:
        # unlike `htop`, it is no term typed as intended (`старческую` is in a Hunspell dictionary only).
        model = self.script(lambda item: "convert" if item.original != "vehpbkrf[" or item.field.after else "keep")
        self.reset_editor(0)
        self.type("vehpbkrf[ ghbdtn ")
        self.assertEqual(self.backend.text, "мурзилках привет ")
        self.assertIn(("vehpbkrf[", "", "привет"), model.questions)

    def test_a_token_the_code_guard_keeps_whole_is_not_asked(self) -> None:
        # `-PHP,` is undecided, as the code guard keeps it from segmentation, but not open, and
        # `-ЗРЗб` is no word. Asked with `значит` after it, the model converted it in the owner's
        # typing (25.09.2026) once undecided tokens were asked whatever they were.
        model = self.script(lambda item: "convert")
        self.reset_editor(0)
        self.type("-PHP, pyfxbn ")
        self.assertEqual(self.backend.text, "-PHP, значит ")
        self.assertFalse(any(question[0] == "-PHP," for question in model.questions))

    def test_an_open_token_whose_keys_read_as_typed_is_not_asked(self) -> None:
        # `repo,` may be `кузщ` and a comma or `кузщб`: no word, and English keys to the orthotactic model.
        model = self.script(lambda item: "convert")
        self.settings.set("diagnostics.technical_logging", True)
        self.reset_editor(0)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type("repo, ghbdtn ")
        self.assertEqual(self.backend.text, "repo, привет ")
        self.assertFalse(any(question[0] == "repo," for question in model.questions))
        self.assertTrue(any('"event":"kept_word_not_asked"' in line for line in logs.output))

    def test_a_word_the_user_excluded_is_not_taken_along(self) -> None:
        # The model is asked about an excluded word neither at its space nor after it (_decide_word).
        model = self.script()
        self.settings.set("exclusions.words", ["руку"])
        self.reset_editor(1)
        self.type("руку ерун ", group=1)
        self.assertEqual(self.backend.text, "руку they ")
        self.assertNotIn(("руку", "", "they"), model.questions)

    def test_a_word_a_rule_of_the_user_keeps_is_not_taken_along(self) -> None:
        model = self.script()
        self.settings.set("detection.learning", True)
        self.engine.learning.add_rule(LearnedRule("руку", 1, 0, "keep"))
        self.reset_editor(1)
        self.type("руку ерун ", group=1)
        self.assertEqual(self.backend.text, "руку they ")
        self.assertNotIn(("руку", "", "they"), model.questions)

    def test_an_undecided_token_too_short_for_the_orthotactic_model_is_not_asked(self) -> None:
        # `o.` is `щ` and a period or `щю`: no word, and two keys tell the orthotactic model nothing.
        model = self.script(lambda item: "convert")
        self.reset_editor(0)
        self.type("o. ghbdtn ")
        self.assertEqual(self.backend.text, "o. привет ")
        self.assertFalse(any(question[0] == "o." for question in model.questions))

    def test_an_undecided_token_is_not_asked_without_the_orthotactic_model(self) -> None:
        model = self.script(lambda item: "convert")
        self.engine.context_policy.ortho = None
        self.reset_editor(0)
        self.type("vehpbkrf[ ghbdtn ")
        self.assertEqual(self.backend.text, "vehpbkrf[ привет ")
        self.assertFalse(any(question[0] == "vehpbkrf[" for question in model.questions))

    def test_an_undecided_token_is_taken_along_on_the_model_s_own_conversion_only(self) -> None:
        # The detector, asked about `it.` now, reads the period as typed and the key of `ю` in the other
        # reading - no English word against the Russian `шею` - and would confirm a mere suggestion.
        model = ScriptedActionModel(lambda item: "suggest" if item.original == "it." else "convert")
        self.engine.context_policy.model = model
        self.reset_editor(0)
        self.type("it. ntgthm ")
        self.assertEqual(self.backend.text, "it. теперь ")
        self.assertIn(("it.", "", "теперь"), model.questions)

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

    def test_a_word_converted_at_a_pause_with_a_sign_split_off_takes_the_kept_word_along(self) -> None:
        # As at a space: the comma stays as typed after the whole correction.
        self.script()
        self.reset_editor(1)
        self.type("руку ", group=1)
        self.type("ерун,", group=1)
        self.engine._last_word_input_at = LAST_WORD_INPUT_AT_SECONDS
        self.engine._maybe_correct_after_pause(now=WAIT_PAIR_PAUSE_CHECK_SECONDS)
        self.assertEqual(self.backend.text, "here they,")

    def test_an_undecided_token_is_taken_along_by_the_word_that_ends_the_message(self) -> None:
        # The last word converts at the pause with its period, the Russian one typed on `/`; the keys
        # are the sequence session's, where `/` types that period in the Russian layout.
        with sequences.session(0) as current:
            current.engine.context_policy.model = ScriptedModel(
                lambda item: "keep" if item.original == "lbre." and not item.field.after else "convert")
            current.physical("lbre. rjire/")
            current.flush()
            current.engine._last_word_input_at = LAST_WORD_INPUT_AT_SECONDS
            current.engine._maybe_correct_after_pause(now=WAIT_PAIR_PAUSE_CHECK_SECONDS)
            current.flush()
            self.assertEqual(current.backend.text, "дикую кошку.")

    def test_undo_types_back_what_the_word_ending_the_message_took_along(self) -> None:
        # One correction took the token, its space, the word and the period: undo takes back all of it.
        with sequences.session(0) as current:
            current.engine.context_policy.model = ScriptedModel(
                lambda item: "keep" if item.original == "lbre." and not item.field.after else "convert")
            current.physical("lbre. rjire/")
            current.flush()
            current.engine._last_word_input_at = LAST_WORD_INPUT_AT_SECONDS
            current.engine._maybe_correct_after_pause(now=WAIT_PAIR_PAUSE_CHECK_SECONDS)
            current.flush()
            self.assertEqual(current.backend.text, "дикую кошку.")
            current.engine._schedule_undo(UNDO_TRIGGER_SENTINEL_KEYCODE)
            current.send(plain_key("z", UNDO_TRIGGER_SENTINEL_KEYCODE, current.backend.group, False))
            current.flush()
            self.assertEqual(current.backend.text, "lbre. rjire/")

    def test_a_word_converted_at_a_pause_after_a_literal_head_takes_nothing_along(self) -> None:
        # Only the part after the slash converts; the words before the head stay as typed.
        model = self.script()
        self.reset_editor(1)
        self.type("руку ", group=1)
        self.type("ура/ерун", group=1)
        self.engine._last_word_input_at = LAST_WORD_INPUT_AT_SECONDS
        self.engine._maybe_correct_after_pause(now=WAIT_PAIR_PAUSE_CHECK_SECONDS)
        self.assertTrue(self.backend.text.startswith("руку "))
        self.assertNotIn(("руку", "", "they"), model.questions)


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

    def test_a_first_word_is_decided_with_the_next_word_across_a_dash(self) -> None:
        # `Ytn` stayed for good once `-` had cut it off from `где`; a row of sealed test v38 lost
        # its first word the same way. The detector confirms the model's suggestion for `Ytn` at
        # its own space now (ContextPolicy.decide), and the keys of `где` go on in the layout that
        # follows, so the phrase is typed with the physical keys.
        with sequences.session(0) as current:
            current.physical("Ytn - ult ")
            current.flush()
            self.assertEqual(current.backend.text, "Нет - где ")

    def test_a_word_left_undecided_in_the_middle_of_a_sentence_is_decided_with_the_next_word(self) -> None:
        # `vbkkbjyf[` is `миллиона` and a bracket or `миллионах`: the boundary model cannot tell, and the
        # words before it on its line did not decide it either (development rows of corpus v57).
        with sequences.session(1) as current:
            current.physical("d 2 ")
            current.flush()
            current.backend.group = 0
            current.physical("vbkkbjyf[ cdtnjds[ ")
            current.flush()
            self.assertEqual(current.backend.text, "в 2 миллионах световых ")

    def test_a_sign_split_off_the_first_word_is_decided_with_the_next_word(self) -> None:
        # `её` is `t\`` and a backtick to the boundary model; `е` alone is no word.
        self.assertEqual(self.typed(0, "t`", "vfnm"), "её мать ")

    @disclosed_schema3_pair_regression
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
