"""Application conventions: a quote in front of a word that may be a mention."""

from __future__ import annotations

import tempfile
import unittest
from dataclasses import replace
from pathlib import Path
from unittest.mock import patch

from keyswitch.app_quirks import MENTION_HEADS, TELEGRAM_QUOTE_MENTION, mention_head
from keyswitch.backend import KeyEvent, SHIFT_MASK
from keyswitch.config import DEFAULTS, SettingsStore
from keyswitch.engine import KeySwitchEngine
from keyswitch.history import HistoryStore
from keyswitch.windows_ui_model import ALL_SETTING_SPECS
from context_physical_keys import KEYS
from test_input_integrity import EditorBackend

# Shift+2 on the digit row: "@" in the English layout, the quote in the Russian one.
QUOTE_KEY = next(key for key in KEYS if key.characters == ("@", '"'))
SPACE_KEY = next(key for key in KEYS if key.characters == (" ", " "))
SLASH_KEY = next(key for key in KEYS if key.characters == ("/", "."))
BY_LATIN = {key.characters[0]: key for key in KEYS if len(key.characters[0]) == 1}


class MentionHeadTableTests(unittest.TestCase):
    def test_the_telegram_head_matches_its_clients_and_nothing_else(self) -> None:
        def always(_path: str) -> bool:
            return True

        for application in ("Telegram", "telegram-desktop", "AyuGram", "org.telegram.desktop"):
            with self.subTest(application=application):
                self.assertIs(mention_head(application, '"', "@", always), TELEGRAM_QUOTE_MENTION)
        for application, typed, meant in (("Firefox", '"', "@"), ("Telegram", "@", '"'),
                                          ("Telegram", '"', "2"), ("Code", '"', "@")):
            with self.subTest(application=application, typed=typed):
                self.assertIsNone(mention_head(application, typed, meant, always))
        self.assertIsNone(mention_head("Telegram", '"', "@", lambda _path: False))

    def test_every_convention_is_a_setting_the_user_can_see_and_turn_off(self) -> None:
        applications = DEFAULTS["applications"]
        assert isinstance(applications, dict)
        paths = {spec.path for spec in ALL_SETTING_SPECS}
        for head in MENTION_HEADS:
            with self.subTest(head=head.setting):
                section, name = head.setting.split(".", 1)
                self.assertEqual(section, "applications")
                self.assertIn(name, applications)
                self.assertIn(head.setting, paths)
                self.assertTrue(head.title and head.description)


class TelegramQuoteMentionTests(unittest.TestCase):
    """The word after the quote decides what the quote was.

    Both readings are typed with the same keys: `"` + j,o,h,n is `"ощрт` in the
    Russian layout and `@john` in the English one, while `"` + g,h,b,d,t,n is
    the ordinary Russian `"привет`. Nothing is rewritten while the word grows.
    """

    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory(prefix="keyswitch-mention-")
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        self.settings = SettingsStore(root / "settings.json")
        self.settings.set("detection.context_policy", "off")
        self.settings.set("detection.early_switch", False)
        self.settings.set("diagnostics.technical_logging", True)
        self.backend = EditorBackend()
        self.backend.group = 1
        self.application = "Telegram"
        self.clock = [1000.0]
        with patch("keyswitch.engine.time.monotonic", side_effect=lambda: self.clock[0]), \
                patch.object(self.backend, "active_application", side_effect=lambda: self.application):
            self.engine = KeySwitchEngine(self.settings, HistoryStore(root / "history.jsonl"), self.backend)

    def press(self, key: object, *, key_name: str | None = None) -> None:
        characters = key.characters  # type: ignore[attr-defined]
        observed = characters[self.backend.group]
        event = KeyEvent(True, key.keycode, observed, observed, characters,  # type: ignore[attr-defined]
                         self.backend.group, SHIFT_MASK if key.shift else 0,  # type: ignore[attr-defined]
                         round(self.clock[0] * 1000))
        if key_name is not None:
            event = replace(event, key_name=key_name)
        with patch("keyswitch.engine.time.monotonic", side_effect=lambda: self.clock[0]), \
                patch.object(self.backend, "active_application", side_effect=lambda: self.application):
            self.clock[0] += 0.05
            self.backend.type(event)
            self.engine._handle(event)
            self.clock[0] += 0.03
            released = replace(event, pressed=False, timestamp=round(self.clock[0] * 1000))
            self.backend.type(released)
            self.engine._handle(released)

    def named(self, key_name: str, keycode: int = 200) -> None:
        event = replace(
            KeyEvent(True, keycode, "", "", ("", ""), self.backend.group, 0, round(self.clock[0] * 1000)),
            key_name=key_name,
        )
        with patch("keyswitch.engine.time.monotonic", side_effect=lambda: self.clock[0]), \
                patch.object(self.backend, "active_application", side_effect=lambda: self.application):
            self.clock[0] += 0.05
            self.engine._handle(event)
            self.clock[0] += 0.03
            self.engine._handle(replace(event, pressed=False, timestamp=round(self.clock[0] * 1000)))

    def word(self, latin: str) -> None:
        for character in latin:
            self.press(BY_LATIN[character])

    def idle(self, seconds: float = 2.0) -> None:
        self.clock[0] += seconds
        with patch("keyswitch.engine.time.monotonic", side_effect=lambda: self.clock[0]), \
                patch.object(self.backend, "active_application", side_effect=lambda: self.application):
            self.engine._maybe_correct_after_pause()

    def test_a_name_after_the_quote_turns_the_pair_into_a_mention(self) -> None:
        self.press(QUOTE_KEY)
        self.assertEqual(self.backend.text, '"')
        self.word("john")
        self.press(SPACE_KEY)
        self.assertEqual(self.backend.text, "@john ")
        self.assertEqual(self.backend.group, 0)

    def test_the_mention_is_logged_with_the_word_it_followed(self) -> None:
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.press(QUOTE_KEY)
            self.word("john")
            self.press(SPACE_KEY)
        applied = [line for line in logs.output if '"mention_head_applied"' in line]
        self.assertEqual(len(applied), 1)
        self.assertIn('"mode":"boundary"', applied[0])

    def test_a_russian_word_after_the_quote_keeps_the_quotation(self) -> None:
        self.press(QUOTE_KEY)
        self.word("ghbdtn")
        self.press(SPACE_KEY)
        self.assertEqual(self.backend.text, '"привет ')
        self.assertEqual(self.backend.group, 1)

    def test_a_closing_quote_keeps_the_quotation_around_a_name(self) -> None:
        """`"john"` is a quoted name: the word is corrected, both quotes stay."""

        self.press(QUOTE_KEY)
        self.word("john")
        self.press(QUOTE_KEY)
        self.assertEqual(self.backend.text, '"john"')
        self.press(SPACE_KEY)
        self.assertEqual(self.backend.text, '"john" ')

    def test_a_closing_quote_after_a_russian_word_changes_nothing(self) -> None:
        self.press(QUOTE_KEY)
        self.word("ghbdtn")
        self.press(QUOTE_KEY)
        self.assertEqual(self.backend.text, '"привет"')

    def test_a_quote_separated_by_a_space_is_not_a_head(self) -> None:
        self.press(QUOTE_KEY)
        self.press(SPACE_KEY)
        self.word("john")
        self.press(SPACE_KEY)
        self.assertEqual(self.backend.text, '" john ')

    def test_a_closing_quote_does_not_carry_over_to_the_next_word(self) -> None:
        self.word("ghbdtn")
        self.press(QUOTE_KEY)
        self.word("john")
        self.press(SPACE_KEY)
        self.assertEqual(self.backend.text, 'привет"john ')

    def test_a_pause_in_the_middle_decides_the_same_way(self) -> None:
        self.press(QUOTE_KEY)
        self.word("john")
        self.idle()
        self.assertEqual(self.backend.text, "@john")

    def test_a_lone_quote_and_a_pair_of_quotes_stay_as_typed(self) -> None:
        self.press(QUOTE_KEY)
        self.press(SPACE_KEY)
        self.idle()
        self.assertEqual(self.backend.text, '" ')
        self.backend.text = ""
        self.press(QUOTE_KEY)
        self.press(QUOTE_KEY)
        self.idle()
        self.assertEqual(self.backend.text, '""')

    def test_pause_still_converts_a_lone_quote_on_command(self) -> None:
        self.press(QUOTE_KEY)
        self.named("Pause", keycode=127)
        self.idle(0.5)
        self.assertEqual(self.backend.text, "@")

    def test_a_caret_move_before_the_quote_leaves_the_word_alone(self) -> None:
        self.word("ghbdtn")
        self.press(SPACE_KEY)
        self.named("Left")
        self.press(QUOTE_KEY)
        self.word("john")
        self.press(SPACE_KEY)
        self.assertIn('"ощрт', self.backend.text)

    def test_a_layout_chosen_by_hand_before_the_quote_is_respected(self) -> None:
        self.backend.group = 0
        self.press(BY_LATIN["a"])
        self.press(SPACE_KEY)
        self.backend.group = 1
        self.press(QUOTE_KEY)
        self.word("john")
        self.press(SPACE_KEY)
        self.assertEqual(self.backend.text, 'a "ощрт ')

    def test_another_application_converts_the_word_but_not_the_quote(self) -> None:
        self.application = "Code"
        self.press(QUOTE_KEY)
        self.word("john")
        self.press(SPACE_KEY)
        self.assertEqual(self.backend.text, '"john ')

    def test_a_disabled_convention_leaves_the_quote_alone(self) -> None:
        self.settings.set(TELEGRAM_QUOTE_MENTION.setting, False)
        self.press(QUOTE_KEY)
        self.word("john")
        self.press(SPACE_KEY)
        self.assertEqual(self.backend.text, '"john ')

    def test_an_excluded_application_is_never_touched(self) -> None:
        self.settings.set("exclusions.applications", ["telegram"])
        self.press(QUOTE_KEY)
        self.word("john")
        self.press(SPACE_KEY)
        self.idle()
        self.assertEqual(self.backend.text, '"ощрт ')

    def test_a_symbol_the_engine_cannot_translate_is_left_alone(self) -> None:
        """One layout group in the engine: there is no other reading to offer."""
        self.engine.models.pop(0, None)
        self.press(QUOTE_KEY)
        self.word("john")
        self.assertEqual(self.engine._mention_head(self.application), ())
        self.idle()
        self.assertEqual(self.backend.text, '"ощрт')

    def test_a_pause_with_nothing_typed_does_nothing(self) -> None:
        """The word was cleared between the last keystroke and the idle callback."""
        self.press(BY_LATIN["a"])
        self.engine._strokes = []
        self.idle()
        self.assertEqual(self.backend.text, "ф")

    def test_only_a_single_pending_symbol_can_be_a_mention_head(self) -> None:
        self.press(SLASH_KEY)
        self.press(QUOTE_KEY)
        self.word("john")
        self.press(SPACE_KEY)
        self.assertTrue(self.backend.text.startswith('."'), self.backend.text)

    def test_the_early_switch_takes_the_quote_with_it(self) -> None:
        """The prefix already proves the layout, so the head moves with it."""

        self.settings.set("detection.early_switch", True)
        self.press(QUOTE_KEY)
        self.word("john")
        self.assertEqual(self.backend.text, "@john")
        self.assertEqual(self.backend.group, 0)


if __name__ == "__main__":
    unittest.main()
