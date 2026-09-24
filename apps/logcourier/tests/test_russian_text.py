from fixture_values.counts import (
    RUSSIAN_QUANTITY_SAMPLE_COUNTS,
    RUSSIAN_QUANTITY_SAMPLE_FRACTION,
    RUSSIAN_QUANTITY_SAMPLE_WHOLE_FLOAT,
)

from logcourier.russian_text import SECONDS, quantity, russian_number


def test_the_noun_agrees_with_the_last_digits_of_a_count():
    assert [quantity(count, SECONDS) for count in RUSSIAN_QUANTITY_SAMPLE_COUNTS] == [
        "1 секунду",
        "2 секунды",
        "5 секунд",
        "11 секунд",
        "12 секунд",
        "21 секунду",
        "22 секунды",
        "25 секунд",
        "111 секунд",
    ]


def test_a_whole_float_has_no_fraction_and_a_fraction_takes_the_genitive_singular():
    assert quantity(RUSSIAN_QUANTITY_SAMPLE_WHOLE_FLOAT, SECONDS) == "3 секунды"
    assert quantity(RUSSIAN_QUANTITY_SAMPLE_FRACTION, SECONDS) == "2,5 секунды"
    assert russian_number(RUSSIAN_QUANTITY_SAMPLE_FRACTION) == "2,5"
    assert russian_number(RUSSIAN_QUANTITY_SAMPLE_WHOLE_FLOAT) == "3"
