from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from keyswitch.config import DEFAULTS, SettingsStore
from keyswitch.detector import LanguageDetector
from keyswitch.engine import Hotkey, KeySwitchEngine
from keyswitch.history import HistoryStore
from keyswitch.history import HistoryEntry
from keyswitch.language_model import LanguageModel
from keyswitch.layouts import LayoutPair
from keyswitch.system import AutostartManager
from keyswitch.x11_backend import KeyEvent


class FakeBackend:
    def __init__(self) -> None:
        self.injections = []
        self.group = 0

    def active_application(self) -> str:
        return "TestEditor"

    def inject_correction(self, strokes, target_group, boundary) -> None:
        self.injections.append((tuple(strokes), target_group, boundary))
        self.group = target_group

    def start(self, listener) -> None:
        self.listener = listener

    def stop(self) -> None:
        pass

    def close(self) -> None:
        pass


def letter_event(character: str, keycode: int, group: int, pair: LayoutPair) -> KeyEvent:
    if group == 0:
        other = pair.translate(character, "us", "ru")
        characters = (character, other)
    else:
        other = pair.translate(character, "ru", "us")
        characters = (other, character)
    return KeyEvent(True, keycode, characters[0], character, characters, group, 0, keycode)


def boundary_event(pressed: bool, keycode: int = 65, state: int = 0) -> KeyEvent:
    return KeyEvent(pressed, keycode, "space", " ", (" ", " "), 0, state, 1000)


class LayoutTests(unittest.TestCase):
    def test_bidirectional_mapping_preserves_case(self) -> None:
        pair = LayoutPair()
        self.assertEqual(pair.translate("Ghbdtn", "us", "ru"), "Привет")
        self.assertEqual(pair.translate("Руддщ", "ru", "us"), "Hello")


class SettingsTests(unittest.TestCase):
    def test_defaults_are_merged_and_changes_persist(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "config.json"
            path.write_text('{"detection": {"minimum_length": 5}}', encoding="utf-8")
            store = SettingsStore(path)
            self.assertEqual(store.get("detection.minimum_length"), 5)
            self.assertEqual(store.get("detection.confidence"), DEFAULTS["detection"]["confidence"])
            store.set("enabled", False)
            self.assertFalse(SettingsStore(path).get("enabled"))


class DesktopIntegrationTests(unittest.TestCase):
    def test_autostart_entry_can_be_enabled_and_disabled(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "autostart" / "keyswitch.desktop"
            manager = AutostartManager(path)
            self.assertFalse(manager.enabled())
            manager.set_enabled(True)
            contents = path.read_text(encoding="utf-8")
            self.assertTrue(manager.enabled())
            self.assertIn("[Desktop Entry]", contents)
            self.assertIn("X-GNOME-Autostart-enabled=true", contents)
            self.assertNotIn("--hidden", contents)
            manager.set_enabled(False)
            self.assertFalse(path.exists())

    def test_history_contains_only_explicit_correction_fields(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "history.jsonl"
            history = HistoryStore(path, limit=2)
            history.append(HistoryEntry.create("ghbdtn", "привет", "Editor", 9.5))
            history.append(HistoryEntry.create("руддщ", "hello", "Editor", 8.5))
            history.append(HistoryEntry.create("цщкдв", "world", "Editor", 8.0))
            entries = history.read()
            self.assertEqual(len(entries), 2)
            self.assertEqual(entries[-1].replacement, "world")
            payload = path.read_text(encoding="utf-8")
            self.assertNotIn("keycode", payload)
            self.assertNotIn("keystrokes", payload)


class DetectorTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.pair = LayoutPair()
        cls.detector = LanguageDetector(
            {0: LanguageModel.load("en_US"), 1: LanguageModel.load("ru_RU")}
        )

    def decision(self, word: str, group: int):
        target = 1 - group
        source_name, target_name = ("us", "ru") if group == 0 else ("ru", "us")
        translated = self.pair.translate(word, source_name, target_name)
        return self.detector.decide(word, {target: translated}, group)

    def test_common_mistyped_words_are_detected(self) -> None:
        for source, expected, group in (
            ("ghbdtn", "привет", 0),
            ("руддщ", "hello", 1),
            ("цщкдв", "world", 1),
        ):
            with self.subTest(source=source):
                decision = self.decision(source, group)
                self.assertTrue(decision.should_convert)
                self.assertEqual(decision.replacement, expected)

    def test_valid_words_are_not_changed(self) -> None:
        self.assertFalse(self.decision("hello", 0).should_convert)
        self.assertFalse(self.decision("привет", 1).should_convert)

    def test_user_exception_wins(self) -> None:
        translated = self.pair.translate("ghbdtn", "us", "ru")
        decision = self.detector.decide(
            "ghbdtn", {1: translated}, 0, ignored_words={"ghbdtn"}
        )
        self.assertFalse(decision.should_convert)


class EngineTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        root = Path(self.temporary.name)
        self.settings = SettingsStore(root / "config.json")
        self.history = HistoryStore(root / "history.jsonl")
        self.backend = FakeBackend()
        self.engine = KeySwitchEngine(self.settings, self.history, self.backend)
        self.pair = LayoutPair()

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def test_word_is_corrected_after_space_release(self) -> None:
        for index, character in enumerate("ghbdtn", start=30):
            self.engine._handle(letter_event(character, index, 0, self.pair))
        self.engine._handle(boundary_event(True))
        self.assertEqual(self.backend.injections, [])
        self.engine._handle(boundary_event(False))
        self.assertEqual(len(self.backend.injections), 1)
        strokes, target, boundary = self.backend.injections[0]
        self.assertEqual(target, 1)
        self.assertEqual("".join(item.character_for(target) for item in strokes), "привет")
        self.assertIsNotNone(boundary)
        self.assertEqual(self.engine.snapshot.last_action, "ghbdtn → привет")

    def test_disabled_engine_observes_but_does_not_correct(self) -> None:
        self.settings.set("enabled", False)
        for index, character in enumerate("ghbdtn", start=30):
            self.engine._handle(letter_event(character, index, 0, self.pair))
        self.engine._handle(boundary_event(True))
        self.engine._handle(boundary_event(False))
        self.assertEqual(self.backend.injections, [])

    def test_excluded_application_is_not_corrected(self) -> None:
        self.settings.set("exclusions.applications", ["testeditor"])
        for index, character in enumerate("ghbdtn", start=30):
            self.engine._handle(letter_event(character, index, 0, self.pair))
        self.engine._handle(boundary_event(True))
        self.engine._handle(boundary_event(False))
        self.assertEqual(self.backend.injections, [])

    def test_pause_manually_converts_last_valid_word(self) -> None:
        for index, character in enumerate("hello", start=30):
            self.engine._handle(letter_event(character, index, 0, self.pair))
        self.engine._handle(boundary_event(True))
        self.engine._handle(boundary_event(False))
        self.assertEqual(self.backend.injections, [])
        pause_press = KeyEvent(True, 127, "Pause", "", ("", ""), 0, 0, 2000)
        pause_release = KeyEvent(False, 127, "Pause", "", ("", ""), 0, 0, 2001)
        self.engine._handle(pause_press)
        self.engine._handle(pause_release)
        self.assertEqual(len(self.backend.injections), 1)
        strokes, target, boundary = self.backend.injections[0]
        self.assertEqual(target, 1)
        self.assertEqual("".join(item.character_for(target) for item in strokes), "руддщ")
        self.assertIsNotNone(boundary)

    def test_undo_hotkey_restores_previous_layout(self) -> None:
        for index, character in enumerate("ghbdtn", start=30):
            self.engine._handle(letter_event(character, index, 0, self.pair))
        self.engine._handle(boundary_event(True))
        self.engine._handle(boundary_event(False))
        control = 1 << 2
        alt = 1 << 3
        events = (
            KeyEvent(True, 37, "Control_L", "", ("", ""), 1, 0, 3000),
            KeyEvent(True, 64, "Alt_L", "", ("", ""), 1, control, 3001),
            KeyEvent(True, 52, "z", "", ("z", "я"), 1, control | alt, 3002),
            KeyEvent(False, 52, "z", "", ("z", "я"), 1, control | alt, 3003),
            KeyEvent(False, 64, "Alt_L", "", ("", ""), 1, control | alt, 3004),
            KeyEvent(False, 37, "Control_L", "", ("", ""), 1, control, 3005),
        )
        for event in events:
            self.engine._handle(event)
        self.assertEqual([item[1] for item in self.backend.injections], [1, 0])


class HotkeyTests(unittest.TestCase):
    def test_exact_modifier_match(self) -> None:
        event = KeyEvent(True, 33, "p", "", ("p", "з"), 0, (1 << 2) | (1 << 3), 1)
        self.assertTrue(Hotkey("Ctrl+Alt+P").matches(event))
        self.assertFalse(Hotkey("Ctrl+P").matches(event))


if __name__ == "__main__":
    unittest.main()
