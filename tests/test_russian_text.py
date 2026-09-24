"""Counts in Russian texts: the number from its constant, the noun agreeing with it."""

from __future__ import annotations

import unittest

from fixture_values.counts import (
    RUSSIAN_QUANTITY_SAMPLE_COUNTS,
    RUSSIAN_QUANTITY_SAMPLE_FRACTION,
    RUSSIAN_QUANTITY_SAMPLE_WHOLE_FLOAT,
)
from keyswitch.russian_text import HOURS, SECONDS, quantity, russian_number


class RussianTextTests(unittest.TestCase):
    def test_the_noun_agrees_with_the_last_digits_of_a_count(self) -> None:
        self.assertEqual(
            [quantity(count, SECONDS) for count in RUSSIAN_QUANTITY_SAMPLE_COUNTS],
            ["1 секунду", "2 секунды", "5 секунд", "11 секунд", "12 секунд", "21 секунду", "22 секунды",
             "25 секунд", "111 секунд"],
        )
        self.assertEqual(
            [quantity(count, HOURS) for count in RUSSIAN_QUANTITY_SAMPLE_COUNTS],
            ["1 час", "2 часа", "5 часов", "11 часов", "12 часов", "21 час", "22 часа", "25 часов", "111 часов"],
        )

    def test_a_whole_float_has_no_fraction_and_a_fraction_takes_the_genitive_singular(self) -> None:
        self.assertEqual(quantity(RUSSIAN_QUANTITY_SAMPLE_WHOLE_FLOAT, SECONDS), "3 секунды")
        self.assertEqual(quantity(RUSSIAN_QUANTITY_SAMPLE_FRACTION, SECONDS), "2,5 секунды")
        self.assertEqual(quantity(RUSSIAN_QUANTITY_SAMPLE_FRACTION, HOURS), "2,5 часа")
        self.assertEqual(russian_number(RUSSIAN_QUANTITY_SAMPLE_FRACTION), "2,5")
        self.assertEqual(russian_number(RUSSIAN_QUANTITY_SAMPLE_WHOLE_FLOAT), "3")


if __name__ == "__main__":
    unittest.main()
