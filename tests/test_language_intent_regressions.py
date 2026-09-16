"""Authored development regressions through the bundled models and editor.

These examples have been inspected during development. They are not a new
independent model evaluation and must not be relabelled as one after training.
"""

from dataclasses import replace
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from keyswitch.backend import KeyEvent, SHIFT_MASK
from keyswitch.config import SettingsStore
from keyswitch.engine import KeySwitchEngine
from keyswitch.history import HistoryStore
from keyswitch.layouts import LayoutPair
from test_input_integrity import EditorBackend


class LanguageIntentRegressions(unittest.TestCase):
    def replay(
        self,
        segments: tuple[tuple[str, int, int | None], ...],
        *,
        application: str = "Telegram",
    ) -> str:
        """Keep physical keys fixed; subsequent glyphs follow automatic switches.

        A segment is (intended text, intended layout, explicit initial layout).
        None continues the layout left by the preceding input. Explicit changes
        model a user choosing a layout for a legitimate language insertion.
        """

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            settings = SettingsStore(root / "settings.json")
            settings.set("detection.context_policy", "assist")
            settings.set("detection.context_aware", True)
            settings.set("detection.early_switch", False)
            settings.set("detection.respect_manual_layout", False)
            backend = EditorBackend()
            pair = LayoutPair()
            with patch.object(backend, "active_application", return_value=application):
                engine = KeySwitchEngine(
                    settings, HistoryStore(root / "history.jsonl"), backend,
                )
                serial = 100
                for text, intended, initial in segments:
                    if initial is not None:
                        backend.group = initial
                    for desired in text:
                        serial += 1
                        alternate = pair.translate(
                            desired,
                            "us" if intended == 0 else "ru",
                            "ru" if intended == 0 else "us",
                        )
                        characters = (desired, alternate) if intended == 0 else (alternate, desired)
                        observed = characters[backend.group]
                        event = KeyEvent(
                            True, serial, "space" if observed == " " else observed,
                            observed, characters, backend.group,
                            SHIFT_MASK if desired.isupper() else 0, serial,
                        )
                        backend.type(event)
                        engine._handle(event)
                        engine._handle(replace(event, pressed=False))
                self.assertEqual(backend.submissions, [])
                return backend.text

    def test_correct_russian_chat_words_and_typos_keep_their_spelling(self) -> None:
        for word in (
            "гифку", "флуд", "лут", "дюп", "ютуб", "зум", "видос", "скрин", "репост",
            "бэкап", "чатик", "созвон", "пожалуста", "превет", "севодня",
            "уведмление", "клаиватура", "повторнно",
        ):
            with self.subTest(word=word):
                self.assertEqual(self.replay(((word + " ", 1, 1),)), word + " ")

    def test_english_insertions_keep_their_spelling_inside_russian_prose(self) -> None:
        for word in ("healthcheck", "websocket", "reconnect", "backfill", "staging", "hotfix"):
            with self.subTest(word=word):
                segments = (("обсуждаем ", 1, 1), (word + " ", 0, 0), ("завтра ", 1, 1))
                self.assertEqual(self.replay(segments), "обсуждаем " + word + " завтра ")

    def test_correct_text_keeps_repeated_spaces_and_punctuation(self) -> None:
        text = "проверь  сообщение!  потом повтори. "
        self.assertEqual(self.replay(((text, 1, 1),)), text)

    def test_russian_chat_sequence_does_not_change_following_keystrokes(self) -> None:
        text = "флуд закончился  всё нормально. "
        self.assertEqual(self.replay(((text, 1, 1),)), text)

    def test_wrong_layout_russian_keeps_repeated_spaces_and_punctuation(self) -> None:
        text = "проверь  сообщение!  потом повтори. "
        self.assertEqual(self.replay(((text, 1, 0),)), text)

    def test_wrong_layout_commands_and_dotfiles_are_restored_with_spaces(self) -> None:
        for word in ("htop", "webjs", "bild", ".dist"):
            with self.subTest(word=word):
                self.assertEqual(self.replay(((word + "  ", 0, 1),)), word + "  ")

    def test_russian_comment_and_english_insertion_survive_in_code_editor(self) -> None:
        for word in ("гифку", "тыща"):
            with self.subTest(word=word):
                segments = (("// текст ", 1, 1), (word + " ", 1, None), ("preview ", 0, 0))
                self.assertEqual(
                    self.replay(segments, application="Code"), "// текст " + word + " preview ",
                )
