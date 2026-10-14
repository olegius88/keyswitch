"""What Pause converts: the last word past the spaces and signs after it, and the last part of a joined token.

The owner's log of 13.10.2026: Pause right after `цуиоы` (meant `webjs`) with a few spaces and signs typed after
it only switched the layout, and the word was erased and typed again by hand; Pause on the missed `lid-'ne` made
`дшв-эту`, pressed again it came back, and the part was typed anew. The sessions type physical keys with the
context policy off, so that no automatic decision moves the text the Pause is pressed on.
"""

from __future__ import annotations

import unittest
from collections.abc import Iterator
from contextlib import contextmanager

from keyswitch.constants.detection import PAUSE_TRAILING_MAX_KEYS
import test_input_sequence_matrix as sequences


@contextmanager
def session(group: int) -> Iterator[sequences.PhysicalSession]:
    with sequences.session(group) as current:
        current.settings.set("detection.context_policy", "off")
        yield current


class PauseReachTests(unittest.TestCase):
    def pause(self, group: int, *keys: str) -> str:
        with session(group) as current:
            for part in keys:
                if part == "BackSpace":
                    current.command("BackSpace")
                else:
                    current.physical(part)
                current.flush()
            current.command("Pause")
            current.flush()
            return current.backend.text

    def test_pause_reaches_the_last_word_past_spaces_and_signs(self) -> None:
        cases = (
            ("hello - ", "руддщ - "),
            ("hello  ", "руддщ  "),
            ("hello + ", "руддщ + "),
            ("hello -- ", "руддщ -- "),
            # A sign that is another one in the other layout was typed for the text it stands in: `ЗК? ` is `PR? `.
            ("hello? ", "руддщ? "),
        )
        for typed, expected in cases:
            with self.subTest(typed=typed):
                self.assertEqual(self.pause(0, typed), expected)

    def test_letters_erasing_and_too_many_keys_end_the_reach(self) -> None:
        # A letter begins the next word, and Pause converts that one.
        self.assertEqual(self.pause(0, "hello - b"), "hello - и")
        # After an erased key, or past the limit, Pause only switches the layout.
        self.assertEqual(self.pause(0, "hello - ", "BackSpace"), "hello -")
        self.assertEqual(self.pause(0, "hello" + " " * PAUSE_TRAILING_MAX_KEYS), "руддщ" + " " * PAUSE_TRAILING_MAX_KEYS)
        beyond = "hello" + " " * (PAUSE_TRAILING_MAX_KEYS + 1)
        self.assertEqual(self.pause(0, beyond), beyond)
        # A token whose sign is another one in the other layout is converted itself, as before.
        self.assertEqual(self.pause(0, "hello -> "), "hello -Ю ")

    def test_pause_converts_the_part_after_a_joining_sign(self) -> None:
        # The head is settled as typed: only the part after the hyphen converts, at a boundary and inside the word.
        self.assertEqual(self.pause(0, "lid-'ne "), "lid-эту ")
        self.assertEqual(self.pause(0, "lid-'n"), "lid-эт")
        self.assertEqual(self.pause(0, "mongo-cthdbcf - "), "mongo-сервиса - ")
        # A head that is itself a word in the other layout is no settled part: the token converts whole.
        self.assertEqual(self.pause(0, "ghbdtn-vbh"), "привет-мир")

    def test_pause_on_a_token_corrected_whole_takes_all_of_it_back(self) -> None:
        with sequences.session(1) as current:
            current.physical("mongo-service ")
            current.flush()
            # The context model converts the whole token typed in the Russian layout (`ьщтпщ-ыукмшсу`).
            self.assertEqual(current.backend.text, "mongo-service ")
            current.command("Pause")
            current.flush()
            self.assertEqual(current.backend.text, "ьщтпщ-ыукмшсу ")


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    return loader.loadTestsFromTestCase(PauseReachTests)


if __name__ == "__main__":
    unittest.main()
