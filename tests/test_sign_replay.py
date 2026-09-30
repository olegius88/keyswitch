"""Signs typed on the keys of the layout a word was meant in are replayed with the word.

A person typing Russian in the English layout presses the keys of the Russian layout
for signs as well: the comma is Shift+/, so `ghbdtn?` is `привет,`. And `hello,` typed
in the Russian layout is `руддщб`: the comma is the key of `б`. Physical keys only; the
installed models decide whether the word converts.
"""

from __future__ import annotations

import unittest
from unittest.mock import patch

import test_input_sequence_matrix as sequences


def typed(group: int, keys: str) -> str:
    with sequences.session(group) as current:
        current.physical(keys)
        current.flush()
        return current.backend.text


class SignReplayTests(unittest.TestCase):
    def test_a_sign_after_a_converted_word_is_replayed_in_its_layout(self) -> None:
        self.assertEqual(typed(0, "ghbdtn? rfr ltkf/ "), "привет, как дела. ")
        self.assertEqual(typed(0, "Lf? z gjyzk "), "Да, я понял ")
        self.assertEqual(typed(0, "ltkf& ult "), "дела? где ")

    def test_a_sign_after_an_english_word_typed_in_the_russian_layout(self) -> None:
        # Shift+/ is `,` in the Russian layout and `?` in the English one; `/` is `.` and `/`.
        # Until 0.35.0 the sign stayed as the Russian layout typed it: `hello,`, `hello.`.
        self.assertEqual(typed(1, "length? ok "), "length? ok ")
        self.assertEqual(typed(1, "hello?"), "hello?")
        self.assertEqual(typed(1, "hello/"), "hello/")

    def test_a_sign_the_same_in_both_layouts_stays(self) -> None:
        self.assertEqual(typed(0, "ghbdtn! rfr "), "привет! как ")

    def test_a_quote_closes_the_quotation_it_opened_and_is_an_at_sign_otherwise(self) -> None:
        # Shift+2 is `"` in the Russian layout and `@` in the English one. After an English word
        # typed in the Russian layout it is the `@` of an address, unless the same sign opened
        # the word: then it closes a quotation.
        self.assertEqual(typed(1, "support@mail.ru "), "support@mail.ru ")
        self.assertEqual(typed(1, "@hello@ "), '"hello" ')
        self.assertEqual(typed(0, "ghbdtn@ "), 'привет" ')

    def test_a_sign_after_a_word_that_stays_is_left_as_typed(self) -> None:
        self.assertEqual(typed(0, "hello? world "), "hello? world ")

    def test_the_next_word_after_a_replayed_sign_is_a_word_of_its_own(self) -> None:
        # The correction ends the word: typing on does not reopen `привет,`.
        self.assertEqual(typed(0, "ghbdtn?rfr "), "привет,как ")


class RussianLayoutSignTailTests(unittest.TestCase):
    def test_a_word_with_a_sign_typed_in_the_russian_layout_converts_whole(self) -> None:
        self.assertEqual(typed(1, "hello, world "), "hello, world ")
        self.assertEqual(typed(1, "HTML. ok "), "HTML. ok ")

    def test_a_russian_word_ending_in_those_letters_stays(self) -> None:
        # `хлеб` ends with `б`, the key of the comma, and is a word as typed.
        self.assertEqual(typed(1, "[kt, ntcn "), "хлеб тест ")

    def test_a_token_with_signs_inside_is_judged_whole(self) -> None:
        # `з+1ю` is `p+1.`: the tail is not split off a token with signs or digits inside.
        with sequences.session(1) as current:
            count = current.engine._replayed_signs(tuple(current.key(key) for key in "p+1."), 1)
        self.assertEqual(count, 0)

    def test_nothing_is_split_off_without_a_model_of_the_other_layout(self) -> None:
        with sequences.session(1) as current:
            strokes = tuple(current.key(key) for key in "hello,")
            with patch.dict(current.engine.models, {1: current.engine.models[1]}, clear=True):
                self.assertEqual(current.engine._replayed_signs(strokes, 1), 0)

    def test_a_single_letter_is_not_split_off_its_tail(self) -> None:
        # `чё` without `ё` would be a lone `ч` - `x` with a backtick after it - and one letter is no word.
        with sequences.session(1) as current:
            self.assertEqual(current.engine._replayed_signs(tuple(current.key(key) for key in "x`"), 1), 0)
            self.assertEqual(current.engine._replayed_signs(tuple(current.key(key) for key in "ok."), 1), 1)

    def test_nothing_is_split_off_a_token_of_tail_letters_only(self) -> None:
        with sequences.session(1) as current:
            self.assertEqual(current.engine._replayed_signs(tuple(current.key(key) for key in ",."), 1), 0)
            self.assertEqual(current.engine._replayed_signs(tuple(current.key(key) for key in "ok"), 1), 0)


if __name__ == "__main__":
    unittest.main()
