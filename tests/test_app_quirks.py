"""Application conventions: a quote in front of a word that may be a mention."""

from __future__ import annotations

import tempfile
import unittest
from collections.abc import Iterator
from contextlib import contextmanager
from dataclasses import replace
from pathlib import Path
from unittest.mock import patch

from keyswitch.app_quirks import MENTION_HEADS, TELEGRAM_QUOTE_MENTION, mention_head
from keyswitch.backend import KeyEvent
from keyswitch.constants.keyboard import SHIFT_MASK
from keyswitch.config import SettingsStore
from keyswitch.constants.settings_defaults import DEFAULT_SETTINGS
from keyswitch.constants.units import MILLISECONDS_PER_SECOND
from keyswitch.engine import KeySwitchEngine
from keyswitch.history import HistoryStore
from keyswitch.windows_ui_model import ALL_SETTING_SPECS
from context_physical_keys import KEYS
from test_input_integrity import EditorBackend
from fixture_values.clock import (
    APP_QUIRKS_IDLE_PAUSE_SECONDS,
    APP_QUIRKS_SHORT_IDLE_SECONDS,
    FAKE_CLOCK_START_SECONDS,
    SIMULATED_KEY_HOLD_SECONDS,
    SIMULATED_KEY_RELEASE_GAP_SECONDS,
)
from fixture_values.keys import NAMED_KEY_PLACEHOLDER_KEYCODE, PAUSE_KEYCODE

# Shift+2 on the digit row: "@" in the English layout, the quote in the Russian one.
QUOTE_KEY = next(key for key in KEYS if key.characters == ("@", '"'))
SPACE_KEY = next(key for key in KEYS if key.characters == (" ", " "))
SLASH_KEY = next(key for key in KEYS if key.characters == ("/", "."))
BY_LATIN = {key.characters[0]: key for key in KEYS if len(key.characters[0]) == 1}
BY_RUSSIAN = {key.characters[1]: key for key in KEYS if len(key.characters[1]) == 1}


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
        applications = DEFAULT_SETTINGS["applications"]
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
    """The quote is shown as `@` at once; what follows it decides what it was.

    Telegram offers its member list only after a real `@`, and the list is how
    a mention is made: the arrow keys, Enter or a click keep the `@`. Anything
    typed after it writes the quote back, and the word then decides: `"` +
    j,o,h,n is `"ощрт` in the Russian layout and `@john` in the English one,
    while `"` + g,h,b,d,t,n is the ordinary Russian `"привет`.
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
        self.clock = [FAKE_CLOCK_START_SECONDS]
        with patch("keyswitch.engine.time.monotonic", side_effect=lambda: self.clock[0]), \
                patch.object(self.backend, "active_application", side_effect=lambda: self.application):
            self.engine = KeySwitchEngine(self.settings, HistoryStore(root / "history.jsonl"), self.backend)

    def press(self, key: object, *, key_name: str | None = None) -> None:
        characters = key.characters  # type: ignore[attr-defined]
        observed = characters[self.backend.group]
        event = KeyEvent(True, key.keycode, observed, observed, characters,  # type: ignore[attr-defined]
                         self.backend.group, SHIFT_MASK if key.shift else 0,  # type: ignore[attr-defined]
                         round(self.clock[0] * MILLISECONDS_PER_SECOND))
        if key_name is not None:
            event = replace(event, key_name=key_name)
        with patch("keyswitch.engine.time.monotonic", side_effect=lambda: self.clock[0]), \
                patch.object(self.backend, "active_application", side_effect=lambda: self.application):
            self.clock[0] += SIMULATED_KEY_HOLD_SECONDS
            self.backend.type(event)
            self.engine._handle(event)
            self.clock[0] += SIMULATED_KEY_RELEASE_GAP_SECONDS
            released = replace(event, pressed=False, timestamp=round(self.clock[0] * MILLISECONDS_PER_SECOND))
            self.backend.type(released)
            self.engine._handle(released)

    def named(self, key_name: str, keycode: int = NAMED_KEY_PLACEHOLDER_KEYCODE) -> None:
        event = replace(
            KeyEvent(
                True, keycode, "", "", ("", ""), self.backend.group, 0,
                round(self.clock[0] * MILLISECONDS_PER_SECOND),
            ),
            key_name=key_name,
        )
        with patch("keyswitch.engine.time.monotonic", side_effect=lambda: self.clock[0]), \
                patch.object(self.backend, "active_application", side_effect=lambda: self.application):
            self.clock[0] += SIMULATED_KEY_HOLD_SECONDS
            self.engine._handle(event)
            self.clock[0] += SIMULATED_KEY_RELEASE_GAP_SECONDS
            self.engine._handle(
                replace(event, pressed=False, timestamp=round(self.clock[0] * MILLISECONDS_PER_SECOND))
            )

    @contextmanager
    def patched(self) -> Iterator[None]:
        with patch("keyswitch.engine.time.monotonic", side_effect=lambda: self.clock[0]), \
                patch.object(self.backend, "active_application", side_effect=lambda: self.application):
            yield

    def down(self, key: object) -> KeyEvent:
        """Press a key and keep it down: the next key may come before its release."""

        characters = key.characters  # type: ignore[attr-defined]
        observed = characters[self.backend.group]
        self.clock[0] += SIMULATED_KEY_HOLD_SECONDS
        event = KeyEvent(True, key.keycode, observed, observed, characters,  # type: ignore[attr-defined]
                         self.backend.group, SHIFT_MASK if key.shift else 0,  # type: ignore[attr-defined]
                         round(self.clock[0] * MILLISECONDS_PER_SECOND))
        with self.patched():
            self.backend.type(event)
            self.engine._handle(event)
        return event

    def up(self, event: KeyEvent) -> None:
        self.clock[0] += SIMULATED_KEY_RELEASE_GAP_SECONDS
        released = replace(event, pressed=False, timestamp=round(self.clock[0] * MILLISECONDS_PER_SECOND))
        with self.patched():
            self.backend.type(released)
            self.engine._handle(released)

    def edit(self, key_name: str) -> None:
        """A named key the editor acts on as well (BackSpace)."""

        event = replace(
            KeyEvent(True, NAMED_KEY_PLACEHOLDER_KEYCODE, "", "", ("", ""), self.backend.group, 0,
                     round(self.clock[0] * MILLISECONDS_PER_SECOND)),
            key_name=key_name,
        )
        self.backend.type(event)
        self.named(key_name)

    def events(self, name: str) -> list[str]:
        return [line for line in self.logs.output if f'"event":"{name}"' in line]

    def word(self, latin: str) -> None:
        for character in latin:
            self.press(BY_LATIN[character])

    def idle(self, seconds: float = APP_QUIRKS_IDLE_PAUSE_SECONDS) -> None:
        self.clock[0] += seconds
        with patch("keyswitch.engine.time.monotonic", side_effect=lambda: self.clock[0]), \
                patch.object(self.backend, "active_application", side_effect=lambda: self.application):
            self.engine._maybe_correct_after_pause()

    def test_a_name_after_the_quote_turns_the_pair_into_a_mention(self) -> None:
        self.press(QUOTE_KEY)
        self.assertEqual((self.backend.text, self.backend.group), ("@", 1))
        self.word("j")
        self.assertEqual(self.backend.text, '"о')
        self.word("ohn")
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

    def test_a_quote_right_after_punctuation_closes_a_quotation(self) -> None:
        """Text right before the quote, with no space, means it closes a quotation."""

        for punctuation in (BY_RUSSIAN[","], SLASH_KEY, BY_RUSSIAN["!"]):
            with self.subTest(punctuation=punctuation.characters[1]):
                self.backend.text, self.backend.caret = "", 0
                self.word("ghbdtn")
                self.press(punctuation)
                self.press(QUOTE_KEY)
                self.assertEqual(self.backend.text, "привет" + punctuation.characters[1] + '"')
                self.assertIsNone(self.engine._mention_shown)

    def test_a_quote_after_a_space_or_a_new_line_opens_a_mention(self) -> None:
        self.word("ghbdtn")
        self.press(SPACE_KEY)
        self.press(QUOTE_KEY)
        self.assertEqual(self.backend.text, "привет @")
        self.named("Pointer")
        self.backend.text, self.backend.caret = "", 0
        self.press(QUOTE_KEY)
        self.assertEqual(self.backend.text, "@")

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
        self.backend.text, self.backend.caret = "", 0
        self.press(QUOTE_KEY)
        self.press(QUOTE_KEY)
        self.idle()
        self.assertEqual(self.backend.text, '""')

    def test_pause_on_the_shown_at_gives_the_quote_back_for_good(self) -> None:
        self.press(QUOTE_KEY)
        self.named("Pause", keycode=PAUSE_KEYCODE)
        self.idle(APP_QUIRKS_SHORT_IDLE_SECONDS)
        self.assertEqual((self.backend.text, self.backend.group), ('"', 1))
        self.word("john")
        self.press(SPACE_KEY)
        self.assertEqual(self.backend.text, '"john ')

    def test_pause_still_converts_a_lone_quote_where_no_convention_applies(self) -> None:
        self.application = "Code"
        self.press(QUOTE_KEY)
        self.named("Pause", keycode=PAUSE_KEYCODE)
        self.idle(APP_QUIRKS_SHORT_IDLE_SECONDS)
        self.assertEqual(self.backend.text, "@")

    def test_the_member_list_keys_keep_the_at(self) -> None:
        """Every `@` in the collected Telegram logs was followed by arrows, Enter or a click."""

        for keys in (("Down", "Return"), ("Pointer",), ("Escape",)):
            with self.subTest(keys=keys):
                self.backend.text, self.backend.caret = "", 0
                self.press(QUOTE_KEY)
                for key_name in keys:
                    self.named(key_name)
                self.assertEqual(self.backend.text, "@")
                self.assertIsNone(self.engine._mention_shown)
        self.assertEqual(self.backend.group, 1)

    def test_the_next_key_writes_the_quote_back(self) -> None:
        for latin, expected in (("g", '"п'), (" ", '" '), ("5", '"5'), ("-", '"-'), ("(", '"(')):
            with self.subTest(key=latin):
                self.backend.text, self.backend.caret = "", 0
                self.engine._clear_word(reason="test")
                self.press(QUOTE_KEY)
                self.press(BY_LATIN[latin])
                self.assertEqual(self.backend.text, expected)
        self.assertEqual(self.backend.group, 1)

    def test_a_key_pressed_before_the_quote_came_up_cancels_the_at(self) -> None:
        quote = self.down(QUOTE_KEY)
        letter = self.down(BY_LATIN["g"])
        self.up(quote)
        self.up(letter)
        self.assertEqual(self.backend.text, '"п')

    def test_letters_held_over_each_other_are_written_back_together(self) -> None:
        self.press(QUOTE_KEY)
        first = self.down(BY_LATIN["g"])
        second = self.down(BY_LATIN["h"])
        self.up(first)
        self.assertEqual(self.backend.text, "@пр")
        self.up(second)
        self.assertEqual(self.backend.text, '"пр')

    def held_word(self, latin: str, end: object = SPACE_KEY) -> None:
        """All keys held over each other: the word ends while the "@" is still shown."""

        self.press(QUOTE_KEY)
        held = [self.down(BY_LATIN[character]) for character in latin]
        held.append(self.down(end))
        self.assertTrue(self.backend.text.startswith("@"), self.backend.text)
        for event in held:
            self.up(event)

    def test_a_name_finished_before_the_write_back_still_becomes_a_mention(self) -> None:
        self.held_word("john")
        self.assertEqual(self.backend.text, "@john ")

    def test_a_russian_word_finished_before_the_write_back_gets_the_quote_back(self) -> None:
        self.held_word("ghbdtn")
        self.assertEqual(self.backend.text, '"привет ')

    def test_enter_right_after_a_held_word_writes_the_quote_back_first(self) -> None:
        submitted: list[str] = []

        def complete_action(deliver: bool) -> int:
            submitted.append(self.backend.text)
            return 0

        self.backend.complete_action = complete_action  # type: ignore[method-assign]
        self.press(QUOTE_KEY)
        held = [self.down(BY_LATIN[character]) for character in "ghbdtn"]
        enter = replace(
            KeyEvent(True, NAMED_KEY_PLACEHOLDER_KEYCODE, "", "", ("", ""), self.backend.group, 0,
                     round(self.clock[0] * MILLISECONDS_PER_SECOND), deferred=True),
            key_name="Return",
        )
        with self.patched():
            self.engine._handle(enter)
        for event in (*held, replace(enter, pressed=False)):
            self.up(event) if event.pressed else self.engine._handle(event)
        self.assertEqual(submitted, ['"привет'])

    def test_backspace_on_a_second_symbol_leaves_the_first(self) -> None:
        self.press(QUOTE_KEY)
        self.press(QUOTE_KEY)
        self.edit("BackSpace")
        self.assertEqual(self.backend.text, '"')
        self.assertEqual(len(self.engine._symbol_strokes), 1)

    def test_backspace_on_the_shown_at_erases_it(self) -> None:
        self.press(QUOTE_KEY)
        self.edit("BackSpace")
        self.assertEqual(self.backend.text, "")
        self.assertIsNone(self.engine._mention_shown)
        self.word("john")
        self.press(SPACE_KEY)
        self.assertEqual(self.backend.text, "john ")

    def test_backspace_before_the_write_back_keeps_the_letters_after_the_at(self) -> None:
        self.press(QUOTE_KEY)
        first = self.down(BY_LATIN["g"])
        second = self.down(BY_LATIN["h"])
        self.edit("BackSpace")
        self.up(first)
        self.up(second)
        self.assertEqual(self.backend.text, '"п')

    def test_backspace_on_the_only_letter_leaves_the_at_shown(self) -> None:
        self.press(QUOTE_KEY)
        letter = self.down(BY_LATIN["g"])
        self.edit("BackSpace")
        self.up(letter)
        self.assertEqual(self.backend.text, "@")
        self.assertIsNotNone(self.engine._mention_shown)

    def test_a_layout_switched_after_the_at_keeps_it_for_the_name(self) -> None:
        self.press(QUOTE_KEY)
        self.backend.group = 0
        self.word("john")
        self.press(SPACE_KEY)
        self.assertEqual(self.backend.text, "@john ")

    def test_pause_after_a_letter_converts_the_whole_token(self) -> None:
        self.press(QUOTE_KEY)
        letter = self.down(BY_LATIN["j"])
        self.named("Pause", keycode=PAUSE_KEYCODE)
        self.up(letter)
        self.idle(APP_QUIRKS_SHORT_IDLE_SECONDS)
        self.assertEqual(self.backend.text, "@j")
        self.assertIsNone(self.engine._mention_shown)

    def test_nothing_is_shown_while_the_engine_is_off(self) -> None:
        self.settings.set("enabled", False)
        self.press(QUOTE_KEY)
        self.assertEqual(self.backend.text, '"')

    def kept_reason(self, latin: str, end: object) -> str:
        with self.assertLogs("keyswitch.engine", level="INFO") as self.logs:
            self.press(QUOTE_KEY)
            self.word(latin)
            self.press(end)
        kept = self.events("mention_head_kept")
        self.assertEqual(len(kept), 1, kept)
        return kept[0]

    def test_a_quote_kept_before_a_russian_word_is_logged(self) -> None:
        self.assertIn('"reason":"word_kept"', self.kept_reason("ghbdtn", SPACE_KEY))

    def test_a_closed_quotation_is_logged_as_converted_without_the_head(self) -> None:
        self.assertIn('"reason":"converted_without_head"', self.kept_reason("john", QUOTE_KEY))
        self.assertEqual(self.backend.text, '"john"')

    def test_a_word_left_unanalysed_is_logged_with_its_quote(self) -> None:
        self.settings.set("detection.correct_on_space", False)
        self.assertIn('"reason":"word_not_analysed"', self.kept_reason("john", SPACE_KEY))

    def test_a_kept_head_is_logged_at_a_pause_too(self) -> None:
        with self.assertLogs("keyswitch.engine", level="INFO") as self.logs:
            self.press(QUOTE_KEY)
            self.word("ghbdtn")
            self.idle()
        self.assertEqual(self.backend.text, '"привет')
        self.assertIn('"reason":"word_kept"', self.events("mention_head_kept")[0])

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
