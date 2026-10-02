"""The key translation hears Caps Lock (macOS only: the module loads the system frameworks)."""

from __future__ import annotations

import sys
import unittest

from keyswitch.constants.keyboard import ALT_MASK, CONTROL_MASK, LOCK_MASK, SHIFT_MASK
from keyswitch.constants.macos import (
    UC_MODIFIER_ALPHA_LOCK,
    UC_MODIFIER_CONTROL,
    UC_MODIFIER_OPTION,
    UC_MODIFIER_SHIFT,
)


@unittest.skipUnless(sys.platform == "darwin", "keyswitch.macos_native loads macOS frameworks")
class TranslateModifiersTests(unittest.TestCase):
    def test_caps_lock_reaches_uckeytranslate_with_the_other_modifiers(self) -> None:
        # Without alphaLock a word typed under Caps Lock read as lower case while the field
        # showed capitals, and the field check refused every such correction (02.10.2026).
        from keyswitch.macos_native import _translate_modifiers

        self.assertEqual(_translate_modifiers(LOCK_MASK), UC_MODIFIER_ALPHA_LOCK)
        self.assertEqual(
            _translate_modifiers(SHIFT_MASK | LOCK_MASK | ALT_MASK | CONTROL_MASK),
            UC_MODIFIER_SHIFT | UC_MODIFIER_ALPHA_LOCK | UC_MODIFIER_OPTION | UC_MODIFIER_CONTROL,
        )
        self.assertEqual(_translate_modifiers(0), 0)


if __name__ == "__main__":
    unittest.main()
