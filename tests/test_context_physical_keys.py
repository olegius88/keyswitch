"""Physical glyph oracles and table-wide inversion for context experiments."""

from __future__ import annotations

from dataclasses import replace
from pathlib import Path
import sys
from typing import cast
import unittest

TOOLS = str(Path(__file__).resolve().parents[1] / "tools")
if TOOLS not in sys.path:
    sys.path.insert(0, TOOLS)

from context_physical_keys import KEYS, physical_keys, translated

# translated() only accepts group 0 or 1; this is one step past the valid range.
OUT_OF_RANGE_GROUP = 2


class ContextPhysicalKeyTests(unittest.TestCase):
    def test_all_plain_and_shift_glyphs_follow_the_expected_keyboard_rows(self) -> None:
        planes = (
            ("`1234567890-=qwertyuiop[]\\asdfghjkl;'zxcvbnm,./ ",
             "ё1234567890-=йцукенгшщзхъ\\фывапролджэячсмитьбю. "),
            ('~!@#$%^&*()_+QWERTYUIOP{}|ASDFGHJKL:"ZXCVBNM<>? ',
             'Ё!"№;%:?*()_+ЙЦУКЕНГШЩЗХЪ/ФЫВАПРОЛДЖЭЯЧСМИТЬБЮ, '),
        )
        self.assertEqual(physical_keys(), KEYS)
        self.assertEqual(len(KEYS), sum(len(us) for us, _ in planes))
        for us, ru in planes:
            self.assertEqual(translated(us, 0), ru)
            self.assertEqual(translated(ru, 1), us)
        for key in KEYS:
            for group in (0, 1):
                with self.subTest(key=key.keycode, shift=key.shift, group=group):
                    source = key.characters[group]
                    self.assertEqual(translated(source, group), key.characters[1 - group])
                    self.assertEqual(translated(translated(source, group), 1 - group), source)

    def test_shift_letters_and_symbols_share_physical_positions_without_case_loss(self) -> None:
        pairs = (('"<>:{}', "ЭБЮЖХЪ"), ("@#$^&", '"№;:?'), ("|?~", "/,Ё"),
                 ("'[],.;", "эхъбюж"))
        for us, ru in pairs:
            self.assertEqual(translated(us, 0), ru)
            self.assertEqual(translated(ru, 1), us)
        plain = [key for key in KEYS if not key.shift]
        shifted = [key for key in KEYS if key.shift]
        self.assertEqual([key.keycode for key in plain], [key.keycode for key in shifted])
        quote = next(key for key in KEYS if key.characters[0] == '"')
        self.assertTrue(quote.shift)
        self.assertEqual(quote.characters[1], "Э")
        self.assertIsNone(quote.explicit_group)
        self.assertEqual(replace(quote, explicit_group=1).explicit_group, 1)

    def test_unknown_glyphs_and_invalid_groups_fail_instead_of_preserving_wrong_text(self) -> None:
        for group in (0, 1):
            self.assertEqual(translated("", group), "")
            self.assertEqual(translated(" ", group), " ")
            for character in ("\t", "\n", "\u00a0", "’", "🙂", "é"):
                with self.subTest(group=group, codepoint=ord(character)), self.assertRaisesRegex(ValueError, "unsupported physical glyph"):
                    translated(character, group)
        for invalid_group in (-1, OUT_OF_RANGE_GROUP, True, False, "0", None):
            with self.subTest(group=invalid_group), self.assertRaisesRegex(ValueError, "requires group"):
                translated("", cast(int, invalid_group))
