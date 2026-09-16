"""Application quirks: one pending symbol that an application's own syntax explains."""

from __future__ import annotations

import tempfile
import unittest
from dataclasses import replace
from pathlib import Path
from unittest.mock import patch

from keyswitch.app_quirks import SYMBOL_QUIRKS, TELEGRAM_QUOTE_MENTION, symbol_quirk
from keyswitch.backend import KeyEvent, SHIFT_MASK
from keyswitch.config import DEFAULTS, SettingsStore
from keyswitch.engine import KeySwitchEngine
from keyswitch.history import HistoryStore
from keyswitch.windows_ui_model import ALL_SETTING_SPECS
from context_physical_keys import KEYS
from test_input_integrity import EditorBackend

# Shift+2 on the digit row: "@" in the English layout, the quote in the Russian one.
QUOTE_KEY = next(key for key in KEYS if key.characters == ("@", '"'))
LETTER_KEY = next(key for key in KEYS if key.characters == ("h", "р"))
SPACE_KEY = next(key for key in KEYS if key.characters == (" ", " "))


class SymbolQuirkTableTests(unittest.TestCase):
    def test_the_telegram_quirk_matches_its_clients_and_nothing_else(self) -> None:
        def always(_path: str) -> bool:
            return True

        for application in ("Telegram", "telegram-desktop", "AyuGram", "org.telegram.desktop"):
            with self.subTest(application=application):
                self.assertIs(symbol_quirk(application, '"', "@", always), TELEGRAM_QUOTE_MENTION)
        for application, typed, meant in (("Firefox", '"', "@"), ("Telegram", "@", '"'),
                                          ("Telegram", '"', "2"), ("Code", '"', "@")):
            with self.subTest(application=application, typed=typed):
                self.assertIsNone(symbol_quirk(application, typed, meant, always))
        self.assertIsNone(symbol_quirk("Telegram", '"', "@", lambda _path: False))

    def test_every_quirk_is_a_setting_the_user_can_see_and_turn_off(self) -> None:
        applications = DEFAULTS["applications"]
        assert isinstance(applications, dict)
        paths = {spec.path for spec in ALL_SETTING_SPECS}
        for quirk in SYMBOL_QUIRKS:
            with self.subTest(quirk=quirk.setting):
                section, name = quirk.setting.split(".", 1)
                self.assertEqual(section, "applications")
                self.assertIn(name, applications)
                self.assertIn(quirk.setting, paths)
                self.assertTrue(quirk.title and quirk.description)


class TelegramQuoteMentionTests(unittest.TestCase):
    """A quote typed in the Russian layout and left alone is the "@" of a mention."""

    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory(prefix="keyswitch-quirk-")
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        self.settings = SettingsStore(root / "settings.json")
        self.settings.set("detection.context_policy", "off")
        self.settings.set("detection.early_switch", False)
        self.backend = EditorBackend()
        self.backend.group = 1
        self.application = "Telegram"
        self.clock = [1000.0]
        with patch("keyswitch.engine.time.monotonic", side_effect=lambda: self.clock[0]), \
                patch.object(self.backend, "active_application", side_effect=lambda: self.application):
            self.engine = KeySwitchEngine(self.settings, HistoryStore(root / "history.jsonl"), self.backend)

    def press(self, key: object) -> None:
        characters = key.characters  # type: ignore[attr-defined]
        observed = characters[self.backend.group]
        event = KeyEvent(True, key.keycode, observed, observed, characters,  # type: ignore[attr-defined]
                         self.backend.group, SHIFT_MASK if key.shift else 0,  # type: ignore[attr-defined]
                         round(self.clock[0] * 1000))
        with patch("keyswitch.engine.time.monotonic", side_effect=lambda: self.clock[0]), \
                patch.object(self.backend, "active_application", side_effect=lambda: self.application):
            self.clock[0] += 0.05
            self.backend.type(event)
            self.engine._handle(event)
            self.clock[0] += 0.03
            released = replace(event, pressed=False, timestamp=round(self.clock[0] * 1000))
            self.backend.type(released)
            self.engine._handle(released)

    def idle(self, seconds: float = 2.0) -> None:
        self.clock[0] += seconds
        with patch("keyswitch.engine.time.monotonic", side_effect=lambda: self.clock[0]), \
                patch.object(self.backend, "active_application", side_effect=lambda: self.application):
            self.engine._maybe_correct_after_pause()

    def test_a_quote_left_alone_in_telegram_becomes_a_mention(self) -> None:
        self.press(QUOTE_KEY)
        self.assertEqual(self.backend.text, '"')
        self.idle()
        self.assertEqual(self.backend.text, "@")

    def test_a_quote_after_a_finished_word_is_a_mention_too(self) -> None:
        for _ in range(3):
            self.press(LETTER_KEY)
        self.press(SPACE_KEY)
        self.press(QUOTE_KEY)
        self.assertEqual(self.backend.text, 'ррр "')
        self.idle()
        self.assertEqual(self.backend.text, "ррр @")

    def test_a_doubled_quote_and_a_quote_the_user_kept_typing_stay_quotes(self) -> None:
        self.press(QUOTE_KEY)
        self.press(QUOTE_KEY)
        self.idle()
        self.assertEqual(self.backend.text, '""')
        self.backend.text = ""
        self.press(QUOTE_KEY)
        self.press(LETTER_KEY)
        self.idle()
        self.assertEqual(self.backend.text, '"р')

    def test_another_application_and_a_disabled_quirk_change_nothing(self) -> None:
        self.application = "Firefox"
        self.press(QUOTE_KEY)
        self.idle()
        self.assertEqual(self.backend.text, '"')
        self.backend.text = ""
        self.application = "Telegram"
        self.settings.set(TELEGRAM_QUOTE_MENTION.setting, False)
        self.press(QUOTE_KEY)
        self.idle()
        self.assertEqual(self.backend.text, '"')

    def test_a_symbol_the_engine_cannot_translate_is_left_alone(self) -> None:
        """One layout group in the engine: there is no other reading to offer."""
        self.engine.models.pop(0, None)
        self.press(QUOTE_KEY)
        self.idle()
        self.assertEqual(self.backend.text, '"')

    def test_an_excluded_application_is_never_touched(self) -> None:
        self.settings.set("exclusions.applications", ["telegram"])
        self.press(QUOTE_KEY)
        self.idle()
        self.assertEqual(self.backend.text, '"')


if __name__ == "__main__":
    unittest.main()
