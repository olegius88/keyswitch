"""What the selection hotkeys make of the selected text: layout, case and transliteration, without a platform."""

from __future__ import annotations

import unittest

from keyswitch.constants.text import CAPS_LOCK_WORD_MIN_CAPITALS
from keyswitch.layouts import LayoutPair
from keyswitch.selection_text import (
    CYRILLIC_LETTERS,
    CYRILLIC_TO_LATIN,
    LATIN_LETTERS,
    LATIN_TO_CYRILLIC,
    RU_LAYOUT,
    RU_SHIFTED_SIGNS,
    US_LAYOUT,
    US_SHIFTED_SIGNS,
    LayoutConversion,
    convert_layout,
    cycle_case,
    transliterate,
)

PAIR = LayoutPair()


class LayoutConversionTests(unittest.TestCase):

    def test_text_typed_in_the_wrong_layout_is_retyped_on_the_same_keys(self) -> None:
        self.assertEqual(convert_layout("ghbdtn vbh", US_LAYOUT), LayoutConversion("привет мир", US_LAYOUT, RU_LAYOUT))
        self.assertEqual(convert_layout("руддщ цщкдв", US_LAYOUT), LayoutConversion("hello world", RU_LAYOUT, US_LAYOUT))
        self.assertEqual(convert_layout("Ghbdtn", RU_LAYOUT).text, "Привет")
        self.assertEqual(convert_layout("ПРИВЕТ", US_LAYOUT).text, "GHBDTN")

    def test_every_key_of_both_layouts_goes_over_and_back(self) -> None:
        """Each key that types different characters in the two layouts, plain and with Shift."""

        english = "".join(PAIR.us_to_ru) + US_SHIFTED_SIGNS
        russian = "".join(PAIR.us_to_ru.values()) + RU_SHIFTED_SIGNS
        self.assertEqual(convert_layout(english, US_LAYOUT), LayoutConversion(russian, US_LAYOUT, RU_LAYOUT))
        self.assertEqual(convert_layout(russian, US_LAYOUT), LayoutConversion(english, RU_LAYOUT, US_LAYOUT))
        self.assertEqual(len(set(english)), len(english))
        self.assertEqual(len(set(russian)), len(russian))

    def test_the_layout_pair_and_the_selection_agree_on_every_key_the_pair_maps(self) -> None:
        for latin, cyrillic in PAIR.us_to_ru.items():
            with self.subTest(key=latin):
                self.assertEqual(convert_layout("ab" + latin, US_LAYOUT).text, "фи" + cyrillic)
        for cyrillic, latin in PAIR.ru_to_us.items():
            with self.subTest(key=cyrillic):
                self.assertEqual(convert_layout("фи" + cyrillic, RU_LAYOUT).text, "ab" + latin)

    def test_signs_on_shifted_keys_change_with_their_key(self) -> None:
        self.assertEqual(convert_layout("Ghbdtn? vbh/", US_LAYOUT).text, "Привет, мир.")
        self.assertEqual(convert_layout("Привет, мир.", US_LAYOUT).text, "Ghbdtn? vbh/")
        self.assertEqual(convert_layout('ab@#$^&|~{}<>:"', US_LAYOUT).text, 'фи"№;:?/ЁХЪБЮЖЭ')
        self.assertEqual(convert_layout('аб"№;:?/', US_LAYOUT).text, "f,@#$^&|")

    def test_digits_spaces_shared_signs_and_other_characters_stay(self) -> None:
        self.assertEqual(convert_layout("a1 -=!%*()_+\\", US_LAYOUT).text, "ф1 -=!%*()_+\\")
        self.assertEqual(convert_layout("ghbdtn 😀 é", US_LAYOUT).text, "привет 😀 é")

    def test_in_a_mixed_selection_only_the_runs_of_the_fewer_letters_are_retyped(self) -> None:
        self.assertEqual(convert_layout("привет vbh", US_LAYOUT), LayoutConversion("привет мир", US_LAYOUT, RU_LAYOUT))
        self.assertEqual(
            convert_layout("hello world руддщ", RU_LAYOUT), LayoutConversion("hello world hello", RU_LAYOUT, US_LAYOUT))
        self.assertEqual(convert_layout("vbhнейший", RU_LAYOUT).text, "мирнейший")
        # Signs on the keys inside a run go with it; a sign at its edge or alone between words stays.
        self.assertEqual(convert_layout("новый j,]trn в базе", US_LAYOUT).text, "новый объект в базе")
        self.assertEqual(convert_layout("привет vbh.", US_LAYOUT).text, "привет мир.")
        self.assertEqual(convert_layout("привет, vbh 12", US_LAYOUT).text, "привет, мир 12")
        self.assertEqual(convert_layout("привет мир, how", US_LAYOUT).text, "привет мир, рщц")

    def test_a_tie_of_the_scripts_is_retyped_whole_from_the_layout_on_now(self) -> None:
        self.assertEqual(convert_layout("hello руддщ", US_LAYOUT), LayoutConversion("руддщ руддщ", US_LAYOUT, RU_LAYOUT))
        self.assertEqual(convert_layout("hello руддщ", RU_LAYOUT), LayoutConversion("hello hello", RU_LAYOUT, US_LAYOUT))

    def test_a_tie_or_no_letters_follow_the_layout_on_now(self) -> None:
        self.assertEqual(convert_layout("ab аб", RU_LAYOUT), LayoutConversion("ab f,", RU_LAYOUT, US_LAYOUT))
        self.assertEqual(convert_layout("ab аб", US_LAYOUT), LayoutConversion("фи аб", US_LAYOUT, RU_LAYOUT))
        self.assertEqual(convert_layout("ab аб", "").source, US_LAYOUT)
        self.assertEqual(convert_layout("?/", US_LAYOUT).text, ",.")
        self.assertEqual(convert_layout("?/", RU_LAYOUT).text, "&|")

    def test_the_letter_sets_are_the_letters_of_each_layout(self) -> None:
        self.assertIn("Q", LATIN_LETTERS)
        self.assertNotIn("[", LATIN_LETTERS)
        self.assertIn("Ё", CYRILLIC_LETTERS)
        self.assertIn("х", CYRILLIC_LETTERS)
        self.assertNotIn("\\", CYRILLIC_LETTERS)


class CaseCycleTests(unittest.TestCase):

    def test_small_capitals_title_and_back(self) -> None:
        upper = cycle_case("привет мир")
        title = cycle_case(upper)
        self.assertEqual((upper, title, cycle_case(title)), ("ПРИВЕТ МИР", "Привет Мир", "привет мир"))

    def test_any_other_mixture_goes_to_small_letters(self) -> None:
        self.assertEqual(cycle_case("Привет мир"), "привет мир")
        self.assertEqual(cycle_case("HELLO мир"), "hello мир")
        self.assertEqual(cycle_case("iPhone"), "iphone")

    def test_text_typed_under_caps_lock_is_turned_over(self) -> None:
        self.assertEqual(cycle_case("пРИВЕТ"), "Привет")
        self.assertEqual(cycle_case("пРИВЕТ мИР"), "Привет Мир")
        self.assertEqual(cycle_case("пРИВЕТ, Я ТУТ"), "Привет, я тут")
        self.assertEqual(cycle_case("пРИВЕТ мир"), "привет мир")

    def test_names_written_with_a_few_capitals_go_through_the_plain_cycle(self) -> None:
        self.assertEqual(cycle_case("iOS"), "ios")
        self.assertEqual(cycle_case("eSIM"), "esim")
        self.assertEqual(cycle_case("мИР"), "мир")
        self.assertEqual(cycle_case("eSIM пРИВЕТ"), "Esim Привет")
        self.assertLess(len("SIM"), CAPS_LOCK_WORD_MIN_CAPITALS)
        self.assertGreaterEqual(len("РИВЕТ"), CAPS_LOCK_WORD_MIN_CAPITALS)

    def test_a_step_that_changes_nothing_is_passed_over(self) -> None:
        self.assertEqual(cycle_case("Я"), "я")
        self.assertEqual(cycle_case("я"), "Я")

    def test_title_case_keeps_words_whole(self) -> None:
        self.assertEqual(cycle_case("DON'T STOP"), "Don't Stop")
        self.assertEqual(cycle_case("X2Y СЕВЕРО-ЗАПАД"), "X2y Северо-Запад")
        self.assertEqual(cycle_case("'HELLO' ’МИР’"), "'Hello' ’Мир’")

    def test_text_without_cased_letters_stays(self) -> None:
        self.assertEqual(cycle_case("123 !"), "123 !")
        self.assertEqual(cycle_case("日本"), "日本")


class TransliterationTests(unittest.TestCase):

    def test_the_icao_table_letter_by_letter(self) -> None:
        self.assertEqual(
            transliterate("абвгдеёжзийклмнопрстуфхцчшщъыьэюя"),
            "abvgdeezhziiklmnoprstufkhtschshshchieyeiuia",
        )
        self.assertEqual(len(CYRILLIC_TO_LATIN), len("абвгдеёжзийклмнопрстуфхцчшщъыьэюя"))

    def test_capitals_follow_the_word(self) -> None:
        self.assertEqual(transliterate("Щука"), "Shchuka")
        self.assertEqual(transliterate("ЩУКА"), "SHCHUKA")
        self.assertEqual(transliterate("ЖЖ"), "ZHZH")
        self.assertEqual(transliterate("Ж."), "Zh.")
        self.assertEqual(transliterate("ВЬЮГА"), "VIUGA")

    def test_soft_and_hard_signs(self) -> None:
        self.assertEqual(transliterate("Ольга"), "Olga")
        self.assertEqual(transliterate("Вьюга"), "Viuga")
        self.assertEqual(transliterate("Объект"), "Obieekt")

    def test_only_the_dominant_script_is_written_over(self) -> None:
        self.assertEqual(transliterate("Привет, John"), "Privet, John")
        self.assertEqual(transliterate("ab вг"), "ab vg")
        self.assertEqual(transliterate("Hello мир"), "Хелло мир")
        self.assertEqual(transliterate("123 !"), "123 !")

    def test_latin_back_to_cyrillic(self) -> None:
        cases = {
            "Privet": "Привет", "Shchuka": "Щука", "SHCHUKA": "ЩУКА", "Zhuk": "Жук", "ZHUK": "ЖУК",
            "Iuliia": "Юлия", "Mariia": "Мария", "Rossiia": "Россия", "schet": "счет",
            "horosho": "хорошо", "Wladimir": "Владимир", "novye": "новые",
        }
        for latin, cyrillic in cases.items():
            with self.subTest(latin=latin):
                self.assertEqual(transliterate(latin), cyrillic)

    def test_an_i_after_a_vowel_is_the_short_i_before_a_consonant_and_at_the_end(self) -> None:
        cases = {
            "Sergei": "Сергей", "krasnyi": "красный", "moi drug": "мой друг", "voina": "война",
            "Chaikovskii": "Чайковский", "Zaitsev": "Зайцев", "quiz": "куйз",
            # After и the -ии of nouns is the commoner ending, after к, г and х the -ий of adjectives.
            "Rossii": "России", "sinii": "синии", "russkii": "русский", "dolgii": "долгий", "tikhii": "тихий",
            # Before a vowel, after a consonant, or alone it stays и.
            "taksi": "такси", "a i": "а и", "geroiam": "героям",
        }
        for latin, cyrillic in cases.items():
            with self.subTest(latin=latin):
                self.assertEqual(transliterate(latin), cyrillic)

    def test_sequences_read_by_their_place_in_the_word(self) -> None:
        cases = {
            "iavliaetsia": "является", "Nakhoditsia": "Находится", "TSIA": "ТСЯ", "stantsiia": "станция",
            "tsiar": "цяр", "sovetskii": "советский", "SOVETSKII": "СОВЕТСКИЙ", "Yelena": "Елена",
            "na Yelke": "на Елке", "novye": "новые",
        }
        for latin, cyrillic in cases.items():
            with self.subTest(latin=latin):
                self.assertEqual(transliterate(latin), cyrillic)

    def test_informal_spellings_are_read_too(self) -> None:
        cases = {
            "Yolka": "Ёлка", "Yakov": "Яков", "yubka": "юбка", "jolka": "ёлка", "Jurij": "Юрий", "Jana": "Яна",
            "Ol'ga": "Ольга", "Ol’ga": "Ольга", "'Olga'": "'Олга'", "don't": "доньт",
        }
        for latin, cyrillic in cases.items():
            with self.subTest(latin=latin):
                self.assertEqual(transliterate(latin), cyrillic)

    def test_x_is_two_letters_in_the_case_of_its_word(self) -> None:
        self.assertEqual(transliterate("Alexandr"), "Александр")
        self.assertEqual(transliterate("ALEX"), "АЛЕКС")
        self.assertEqual(transliterate("Xenia"), "Ксеня")

    def test_letters_outside_ascii_are_kept(self) -> None:
        self.assertEqual(transliterate("café"), "цафé")
        self.assertEqual(transliterate("İsh"), "İш")

    def test_words_without_ambiguous_letters_go_there_and_back(self) -> None:
        for word in ("Привет", "Щука", "Мария", "России", "красный", "Россия", "ЖУК", "является", "советский"):
            with self.subTest(word=word):
                self.assertEqual(transliterate(transliterate(word)), word)

    def test_every_sequence_the_reverse_reads_is_lower_case_ascii(self) -> None:
        for sequence in LATIN_TO_CYRILLIC:
            self.assertTrue(sequence.isascii() and sequence.islower())


if __name__ == "__main__":
    unittest.main()
