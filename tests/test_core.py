from __future__ import annotations

import tempfile
import unittest
from collections.abc import Callable, Iterable, Sequence
from pathlib import Path
from typing import ClassVar
from unittest.mock import patch

from keyswitch.config import SettingsStore
from keyswitch.detector import DetectionDecision, LanguageDetector
from keyswitch.engine import Hotkey, KeySwitchEngine, LearningPrompt
from keyswitch.history import HistoryStore
from keyswitch.history import HistoryEntry
from keyswitch.indicator import (
    alternate_layout_action_label,
    alternate_layout_group,
    layout_icon_name,
    layout_label,
    normalize_indicator_style,
)
from keyswitch.language_model import LanguageModel
from keyswitch.learning import LearningStore
from keyswitch.layouts import LayoutPair
from keyswitch.language_model import WordScore
from keyswitch.short_words import (
    CONTEXT_SHORT_WORD_REASON,
    NATURAL_SOURCE_REASON,
    natural_short_source_veto,
    trusted_short_word_decision,
)
from keyswitch.spellcheck import HunspellDictionary
from keyswitch.system import AutostartManager
from keyswitch.x11_backend import BackendProbe, KeyEvent


class FakeBackend:
    def __init__(self) -> None:
        self.injections: list[tuple[tuple[KeyEvent, ...], int, KeyEvent | None]] = []
        self.late: list[tuple[KeyEvent, ...]] = []
        self.hold_calls = 0
        self.held_count = 0
        self.group = 0
        self.listener: Callable[[KeyEvent], None] | None = None

    def active_application(self) -> str:
        return "TestEditor"

    def focused_window(self) -> None:
        return None

    def current_group(self) -> int:
        return self.group

    def switch_group(self, group: int) -> None:
        self.group = group

    def hold_input(self) -> None:
        self.hold_calls += 1

    def inject_correction(
        self,
        strokes: Iterable[KeyEvent],
        target_group: int,
        boundary: KeyEvent | None,
        source_group: int | None = None,
        late: Sequence[KeyEvent] = (),
    ) -> int:
        self.injections.append((tuple(strokes), target_group, boundary))
        self.late.append(tuple(late))
        self.group = target_group
        return self.held_count

    def set_key_filter(
        self, predicate: Callable[[KeyEvent], bool] | None
    ) -> None:
        self.key_filter = predicate

    def start(self, listener: Callable[[KeyEvent], None]) -> None:
        self.listener = listener

    def stop(self) -> None:
        pass

    def close(self) -> None:
        pass

    def probe(self) -> BackendProbe:
        return BackendProbe(True, "x11", ":test", "1", "2", "1", self.group)


def letter_event(character: str, keycode: int, group: int, pair: LayoutPair) -> KeyEvent:
    if group == 0:
        other = pair.translate(character, "us", "ru")
        characters = (character, other)
    else:
        other = pair.translate(character, "ru", "us")
        characters = (other, character)
    return KeyEvent(True, keycode, characters[0], character, characters, group, 0, keycode)


def boundary_event(
    pressed: bool,
    keycode: int = 65,
    state: int = 0,
    group: int = 0,
) -> KeyEvent:
    return KeyEvent(
        pressed,
        keycode,
        "space",
        " ",
        (" ", " "),
        group,
        state,
        1000,
    )


def release_event(event: KeyEvent) -> KeyEvent:
    return KeyEvent(
        False,
        event.keycode,
        event.key_name,
        event.character,
        event.characters,
        event.group,
        event.state,
        event.timestamp + 1,
    )


class LayoutTests(unittest.TestCase):
    def test_bidirectional_mapping_preserves_case(self) -> None:
        pair = LayoutPair()
        self.assertEqual(pair.translate("Ghbdtn", "us", "ru"), "Привет")
        self.assertEqual(pair.translate("Руддщ", "ru", "us"), "Hello")

    def test_alternate_layout_menu_mapping(self) -> None:
        self.assertEqual(alternate_layout_group(0), 1)
        self.assertEqual(alternate_layout_group(1), 0)
        self.assertIsNone(alternate_layout_group(-1))
        self.assertEqual(
            alternate_layout_action_label(0),
            "Переключить на русский (RU)",
        )
        self.assertEqual(
            alternate_layout_action_label(1),
            "Переключить на английский (EN)",
        )
        self.assertEqual(alternate_layout_action_label(7), "Переключить язык")


class SettingsTests(unittest.TestCase):
    def test_defaults_are_merged_and_changes_persist(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "config.json"
            path.write_text('{"detection": {"minimum_length": 5}}', encoding="utf-8")
            store = SettingsStore(path)
            self.assertEqual(store.get("detection.minimum_length"), 5)
            self.assertEqual(store.get("detection.confidence"), 2.0)
            self.assertTrue(store.get("general.autostart"))
            self.assertTrue(store.get("general.start_hidden"))
            self.assertEqual(store.get("appearance.indicator_style"), "letters")
            self.assertTrue(store.get("detection.context_aware"))
            self.assertTrue(store.get("detection.protect_code"))
            self.assertTrue(store.get("detection.intent_model_enabled"))
            self.assertTrue(store.get("detection.respect_manual_layout"))
            self.assertTrue(store.get("detection.correct_on_pause"))
            self.assertTrue(store.get("detection.learning"))
            self.assertEqual(store.get("detection.learning_confirmations"), 2)
            store.set("enabled", False)
            self.assertFalse(SettingsStore(path).get("enabled"))

    def test_a_single_setting_reports_and_restores_its_default(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            store = SettingsStore(Path(directory) / "config.json")
            changed: list[tuple[str, object]] = []
            store.subscribe(lambda path, value: changed.append((path, value)))

            self.assertEqual(store.default("detection.confidence"), 2.0)
            self.assertIsNone(store.default("detection.unknown"))
            self.assertEqual(store.default("detection.unknown", "—"), "—")
            self.assertTrue(store.is_default("detection.confidence"))

            store.set("detection.confidence", 3.5)
            self.assertFalse(store.is_default("detection.confidence"))
            self.assertTrue(store.restore_default("detection.confidence"))
            self.assertEqual(store.get("detection.confidence"), 2.0)
            self.assertTrue(store.is_default("detection.confidence"))
            self.assertEqual(
                changed,
                [("detection.confidence", 3.5), ("detection.confidence", 2.0)],
            )

            # A path outside the shipped schema has no default to restore.
            self.assertFalse(store.restore_default("detection.unknown"))
            self.assertTrue(store.is_default("detection.unknown"))
            store.set("detection.unknown", "kept")
            self.assertFalse(store.is_default("detection.unknown"))

            # Mutating what a lookup returned must not touch the defaults.
            layouts = store.default("detection.layouts")
            assert isinstance(layouts, list)
            layouts.append("de")
            self.assertEqual(store.default("detection.layouts"), ["us", "ru"])


class LearningTests(unittest.TestCase):
    def test_manual_confirmations_and_rejection_are_persistent(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "learning.json"
            learning = LearningStore(path)
            self.assertEqual(learning.record_manual(0, "qwerty", 1), 1)
            self.assertIsNone(learning.forced_target(0, "QWERTY", 2))
            self.assertEqual(learning.record_manual(0, "qwerty", 1), 2)
            self.assertEqual(LearningStore(path).forced_target(0, "Qwerty", 2), 1)
            learning.reject(0, "qwerty", 1)
            self.assertIsNone(learning.forced_target(0, "qwerty", 1))
            self.assertEqual(learning.rejected_targets(0, "qwerty"), {1})

    def test_layout_punctuation_is_part_of_the_learned_identity(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            learning = LearningStore(Path(directory) / "learning.json")
            learning.record_manual(0, ",fpf", 1)
            learning.record_manual(0, ",fpf", 1)
            self.assertEqual(learning.forced_target(0, ",FPF", 2), 1)
            self.assertIsNone(learning.forced_target(0, "fpf", 2))


class LearningRuleStateTests(unittest.TestCase):
    def test_a_pending_rule_is_readable_before_it_becomes_active(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            store = LearningStore(Path(directory) / "learning.json")
            self.assertEqual(store.rule_state(0, "qwerty"), (None, 0))

            store.record_manual(0, "qwerty", 1)
            self.assertEqual(store.rule_state(0, "qwerty"), (1, 1))
            # One confirmation is not yet a rule that forces a conversion.
            self.assertIsNone(store.forced_target(0, "qwerty", 2))

            store.record_manual(0, "qwerty", 1)
            self.assertEqual(store.rule_state(0, "qwerty"), (1, 2))
            self.assertEqual(store.forced_target(0, "qwerty", 2), 1)

            # A malformed rule reads as no rule at all.
            store._data["rules"][store._key(0, "qwerty")] = {"confirmations": 3}
            self.assertEqual(store.rule_state(0, "qwerty"), (None, 0))
            store._data["rules"][store._key(0, "qwerty")] = "broken"
            self.assertEqual(store.rule_state(0, "qwerty"), (None, 0))

    def test_the_settings_window_can_list_what_was_learned(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            store = LearningStore(Path(directory) / "learning.json")
            self.assertEqual(store.rules(), ())
            self.assertEqual(store.rejections(), ())

            store.record_manual(0, "ghbdtn", 1)
            store.record_manual(0, "ghbdtn", 1)
            store.record_manual(1, "руддщ", 0)
            store.reject(0, "hjrjdsq", 1)

            self.assertEqual(
                [
                    (rule.word, rule.source_group, rule.target_group,
                     rule.confirmations, rule.active)
                    for rule in store.rules(2)
                ],
                [("ghbdtn", 0, 1, 2, True), ("руддщ", 1, 0, 1, False)],
            )
            # A stricter threshold leaves the same rule waiting.
            self.assertFalse(store.rules(3)[0].active)
            self.assertEqual(
                [
                    (item.word, item.source_group, item.target_group)
                    for item in store.rejections()
                ],
                [("hjrjdsq", 0, 1)],
            )

            # Damaged entries are skipped instead of breaking the listing.
            rules = store._data["rules"]
            rules["broken"] = {"target_group": 1, "confirmations": 1}
            rules["x:word"] = {"target_group": 1, "confirmations": 1}
            rules[store._key(0, "text")] = "not a rule"
            rules[store._key(0, "other")] = {"confirmations": 1}
            rejections = store._data["rejections"]
            rejections["broken"] = [1]
            rejections[store._key(0, "word")] = "not a list"
            rejections[store._key(0, "mixed")] = [1, "nonsense"]
            self.assertEqual(len(store.rules(2)), 2)
            self.assertEqual(
                [item.word for item in store.rejections()], ["hjrjdsq", "mixed"]
            )


class SpellcheckTests(unittest.TestCase):
    def test_per_user_dictionary_root_is_discovered(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "keyswitch" / "dictionaries"
            root.mkdir(parents=True)
            affix = root / "zz_ZZ.aff"
            dictionary = root / "zz_ZZ.dic"
            affix.write_text("SET UTF-8\n", encoding="utf-8")
            dictionary.write_text("0\n", encoding="utf-8")
            with patch.dict("os.environ", {"XDG_DATA_HOME": directory}):
                self.assertEqual(
                    HunspellDictionary._find_dictionary("zz_ZZ"),
                    (affix, dictionary),
                )


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
            self.assertIn("Hidden=false", contents)
            self.assertIn("--hidden", contents)
            self.assertNotIn("OnlyShowIn", contents)
            manager.set_enabled(True, start_hidden=False)
            self.assertNotIn("--hidden", path.read_text(encoding="utf-8"))
            path.write_text(
                "[Desktop Entry]\nHidden=false\nX-GNOME-Autostart-enabled=false\n",
                encoding="utf-8",
            )
            self.assertFalse(manager.enabled())
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
    pair: ClassVar[LayoutPair]
    detector: ClassVar[LanguageDetector]

    @classmethod
    def setUpClass(cls) -> None:
        cls.pair = LayoutPair()
        cls.detector = LanguageDetector(
            {0: LanguageModel.load("en_US"), 1: LanguageModel.load("ru_RU")}
        )

    def decision(self, word: str, group: int) -> DetectionDecision:
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

    def test_trusted_short_if_bypasses_configured_minimum_length(self) -> None:
        translated = self.pair.translate("ша", "ru", "us")
        self.assertEqual(translated, "if")
        decision = trusted_short_word_decision(
            self.detector,
            "ша",
            {0: translated},
            1,
            ignored_words=(),
            rejected_targets=set(),
            protect_code=True,
        )
        self.assertIsNotNone(decision)
        assert decision is not None
        self.assertTrue(decision.should_convert)
        self.assertEqual(decision.replacement, "if")
        self.assertIn("короткое слово", decision.reason)

        for source in ("шт", "фе"):
            replacement = self.pair.translate(source, "ru", "us")
            with self.subTest(source=source, replacement=replacement):
                self.assertIsNone(
                    trusted_short_word_decision(
                        self.detector,
                        source,
                        {0: replacement},
                        1,
                        ignored_words=(),
                        rejected_targets=set(),
                        protect_code=True,
                    )
                )

    def test_russian_function_words_typed_in_latin_are_trusted(self) -> None:
        def short(original: str, replacement: str, context: int | None) -> DetectionDecision | None:
            return trusted_short_word_decision(
                self.detector,
                original,
                {1: replacement},
                0,
                ignored_words=(),
                rejected_targets=set(),
                protect_code=True,
                context_group=context,
            )

        # Frequent and unambiguous: no context needed.
        for original, replacement in (("yt", "не"), ("gj", "по"), ("yf", "на")):
            with self.subTest(original=original):
                decision = short(original, replacement, None)
                assert decision is not None
                self.assertTrue(decision.should_convert)
                self.assertEqual(decision.replacement, replacement)
                self.assertEqual(decision.reason, "частотное короткое слово из безопасного списка")
        # Real English tokens need the previous word to be Russian.
        self.assertIsNone(short("kb", "ли", None))
        with_context = short("kb", "ли", 1)
        assert with_context is not None
        self.assertEqual(with_context.reason, CONTEXT_SHORT_WORD_REASON)
        # ...and even then a more frequent English token stays.
        self.assertIsNone(short("vs", "мы", 1))
        # A single letter is a word only after a Russian word.
        self.assertIsNone(short("f", "а", None))
        self.assertIsNone(short("f", "а", 0))
        letter = short("f", "а", 1)
        assert letter is not None
        self.assertTrue(letter.should_convert)
        self.assertEqual((letter.replacement, letter.confidence), ("а", 1.0))
        self.assertEqual(letter.reason, CONTEXT_SHORT_WORD_REASON)
        # A letter outside the curated list is not a word.
        self.assertIsNone(short("g", "п", 1))

    def test_a_short_natural_word_is_not_traded_for_a_dictionary_token(self) -> None:
        def decision(
            *,
            convert: bool = True,
            reason: str = "слово найдено только в целевом частотном словаре",
            original: str = "дев",
            replacement: str = "ltd",
            ngram: float = -0.73,
        ) -> DetectionDecision:
            source = WordScore(-0.8, False, 0, 0.0, ngram_score=ngram)
            target = WordScore(4.2, True, 54906, 0.0, exact=True, ngram_score=-4.0)
            return DetectionDecision(
                convert, original, replacement, 1, 0, 4.73, reason, source, target
            )

        vetoed = natural_short_source_veto(decision(), context_group=1)
        self.assertFalse(vetoed.should_convert)
        self.assertEqual(vetoed.reason, NATURAL_SOURCE_REASON)
        # The context favouring the target, an unnatural source, a longer
        # token, another reason or no conversion at all all leave it alone.
        for kept in (
            natural_short_source_veto(decision(), context_group=0),
            natural_short_source_veto(decision(ngram=-6.3), context_group=1),
            natural_short_source_veto(
                decision(original="rfrjq", replacement="какой"), context_group=1
            ),
            natural_short_source_veto(
                decision(reason="уверенное решение линейной n-граммной модели"),
                context_group=1,
            ),
        ):
            self.assertTrue(kept.should_convert)
        untouched = decision(convert=False)
        self.assertIs(natural_short_source_veto(untouched, context_group=1), untouched)

    def test_valid_words_are_not_changed(self) -> None:
        self.assertFalse(self.decision("hello", 0).should_convert)
        self.assertFalse(self.decision("привет", 1).should_convert)

    def test_common_technical_tokens_and_code_are_protected(self) -> None:
        for word in ("xfce", "kubectl", "API", "camelCase", "user_name", "abc123"):
            with self.subTest(word=word):
                self.assertFalse(self.decision(word, 0).should_convert)

    def test_one_extra_letter_does_not_hide_the_target_layout(self) -> None:
        for source, expected in (("рудддщ", "helllo"), ("цщкддв", "worlld")):
            with self.subTest(source=source):
                decision = self.decision(source, 1)
                self.assertTrue(decision.should_convert)
                self.assertEqual(decision.replacement, expected)
                self.assertIn("опечатки", decision.reason)

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
        # These fixtures type whole words at once; the prefix-based early
        # switch has dedicated tests and would otherwise fire on "ghbd".
        self.settings.set("detection.early_switch", False)
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

    def test_word_is_corrected_after_typing_pause_without_boundary(self) -> None:
        with patch("keyswitch.engine.time.monotonic", return_value=10.0):
            for index, character in enumerate("ghbdtn", start=30):
                event = letter_event(character, index, 0, self.pair)
                self.engine._handle(event)
                self.engine._handle(release_event(event))

        self.engine._maybe_correct_after_pause(now=11.49)
        self.assertEqual(self.backend.injections, [])
        self.engine._maybe_correct_after_pause(now=11.5)

        self.assertEqual(len(self.backend.injections), 1)
        strokes, target, boundary = self.backend.injections[0]
        self.assertEqual(target, 1)
        self.assertIsNone(boundary)
        self.assertEqual(
            "".join(item.character_for(target) for item in strokes),
            "привет",
        )
        self.assertEqual(self.engine.snapshot.current_word, "")
        self.assertEqual(
            [(entry.original, entry.replacement) for entry in self.history.read()],
            [("ghbdtn", "привет")],
        )

    def test_pause_correction_can_be_disabled(self) -> None:
        self.settings.set("detection.correct_on_pause", False)
        with patch("keyswitch.engine.time.monotonic", return_value=20.0):
            for index, character in enumerate("ghbdtn", start=30):
                event = letter_event(character, index, 0, self.pair)
                self.engine._handle(event)
                self.engine._handle(release_event(event))

        self.engine._maybe_correct_after_pause(now=22.0)

        self.assertEqual(self.backend.injections, [])
        self.assertEqual(self.engine.snapshot.current_word, "ghbdtn")
        self.assertFalse(self.engine._pause_correction_pending)

    def test_layout_letter_on_punctuation_key_is_kept_inside_word(self) -> None:
        self.settings.set("detection.respect_manual_layout", False)
        for expected, physical in (("база", ",fpf"), ("общих", "j,ob[")):
            with self.subTest(expected=expected):
                self.backend.injections.clear()
                self.backend.group = 0
                for index, character in enumerate(physical, start=30):
                    event = letter_event(character, index, 0, self.pair)
                    self.engine._handle(event)
                    self.engine._handle(release_event(event))
                self.engine._handle(boundary_event(True))
                self.engine._handle(boundary_event(False))
                self.assertEqual(len(self.backend.injections), 1)
                strokes, target, _boundary = self.backend.injections[0]
                self.assertEqual(target, 1)
                self.assertEqual(
                    "".join(item.character_for(target) for item in strokes), expected
                )

    def test_punctuation_after_a_complete_word_remains_a_boundary(self) -> None:
        for index, character in enumerate("ghbdtn", start=30):
            self.engine._handle(letter_event(character, index, 0, self.pair))
        comma = letter_event(",", 59, 0, self.pair)
        self.engine._handle(comma)
        self.assertEqual(self.backend.injections, [])
        self.engine._handle(release_event(comma))
        self.assertEqual(len(self.backend.injections), 1)
        _strokes, target, boundary = self.backend.injections[0]
        self.assertEqual(target, 1)
        assert boundary is not None
        self.assertEqual(boundary.character, ",")

    def test_punctuation_after_a_protected_unknown_token_starts_a_new_word(self) -> None:
        for index, character in enumerate("kubectl", start=30):
            event = letter_event(character, index, 0, self.pair)
            self.engine._handle(event)
            self.engine._handle(release_event(event))
        comma = letter_event(",", 59, 0, self.pair)
        self.engine._handle(comma)
        self.engine._handle(release_event(comma))
        self.assertEqual(self.engine.snapshot.current_word, "")
        self.assertEqual(self.backend.injections, [])

        for index, character in enumerate("ghbdtn", start=70):
            event = letter_event(character, index, 0, self.pair)
            self.engine._handle(event)
            self.engine._handle(release_event(event))
        self.engine._handle(boundary_event(True))
        self.engine._handle(boundary_event(False))
        self.assertEqual(len(self.backend.injections), 1)
        strokes, target, _boundary = self.backend.injections[0]
        self.assertEqual(target, 1)
        self.assertEqual(
            "".join(item.character_for(target) for item in strokes), "привет"
        )

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

    def test_layout_group_is_refreshed_without_typing(self) -> None:
        self.assertEqual(self.engine.snapshot.current_group, -1)
        self.backend.group = 1
        self.engine._poll_current_group()
        self.assertEqual(self.engine.snapshot.current_group, 1)
        self.assertIsNone(self.engine._manual_layout_group)

        self.backend.group = 0
        self.engine._poll_current_group()
        self.assertEqual(self.engine.snapshot.current_group, 0)
        self.assertEqual(self.engine._manual_layout_group, 0)
        self.assertIn("Ручная смена", self.engine.snapshot.last_action)

    def test_manual_layout_switch_protects_exactly_the_next_word(self) -> None:
        self.engine._update(current_group=1)
        for index, character in enumerate("ghbdtn", start=30):
            self.engine._handle(letter_event(character, index, 0, self.pair))
        self.engine._handle(boundary_event(True))
        self.engine._handle(boundary_event(False))

        self.assertEqual(self.backend.injections, [])
        self.assertIsNone(self.engine._manual_layout_group)
        self.assertEqual(
            self.engine.snapshot.last_action,
            "Ручная раскладка сохранена: ghbdtn",
        )

        for index, character in enumerate("ghbdtn", start=70):
            self.engine._handle(letter_event(character, index, 0, self.pair))
        self.engine._handle(boundary_event(True))
        self.engine._handle(boundary_event(False))
        self.assertEqual(len(self.backend.injections), 1)
        self.assertEqual(self.backend.injections[0][1], 1)

    def test_manual_layout_protection_can_be_disabled(self) -> None:
        self.engine._manual_layout_group = 0
        self.settings.set("detection.respect_manual_layout", False)
        self.assertIsNone(self.engine._manual_layout_group)
        self.engine._update(current_group=1)

        for index, character in enumerate("ghbdtn", start=30):
            self.engine._handle(letter_event(character, index, 0, self.pair))
        self.engine._handle(boundary_event(True))
        self.engine._handle(boundary_event(False))
        self.assertEqual(len(self.backend.injections), 1)
        self.assertEqual(self.backend.injections[0][1], 1)

    def test_context_is_not_shared_when_application_is_unknown(self) -> None:
        strokes = tuple(
            letter_event(character, index, 0, self.pair)
            for index, character in enumerate("hello", start=30)
        )
        self.engine._remember_context("", 0, strokes)
        self.assertEqual(self.engine._context_for(""), ({}, None))

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

    def test_enter_confirms_manual_conversion_as_an_immediate_rule(self) -> None:
        prompts: list[LearningPrompt | None] = []
        self.engine.subscribe_learning_prompts(prompts.append)
        for index, character in enumerate("hello", start=30):
            self.engine._handle(letter_event(character, index, 0, self.pair))
        pause = KeyEvent(True, 127, "Pause", "", ("", ""), 0, 0, 2000)
        self.engine._handle(pause)
        self.engine._handle(release_event(pause))

        prompt = self.engine.learning_prompt
        self.assertIsNotNone(prompt)
        assert prompt is not None
        self.assertEqual((prompt.original, prompt.replacement), ("hello", "руддщ"))
        self.assertIsNone(self.engine.learning.forced_target(0, "hello", 2))

        enter = KeyEvent(True, 36, "Return", "\n", ("\n", "\n"), 1, 0, 2002)
        self.engine._handle(enter)
        self.assertIsNone(self.engine.learning_prompt)
        self.assertEqual(self.engine.learning.forced_target(0, "hello", 2), 1)
        self.assertEqual(prompts[-1], None)
        self.assertIn("правило выучено", self.engine.snapshot.last_action)

        self.engine._manual_layout_group = 0
        for index, character in enumerate("hello", start=70):
            self.engine._handle(letter_event(character, index, 0, self.pair))
        self.engine._handle(boundary_event(True))
        self.engine._handle(boundary_event(False))
        self.assertEqual(len(self.backend.injections), 1)
        self.assertIsNone(self.engine._manual_layout_group)
        self.assertIn("Ручная раскладка сохранена", self.engine.snapshot.last_action)

        for index, character in enumerate("hello", start=90):
            self.engine._handle(letter_event(character, index, 0, self.pair))
        self.engine._handle(boundary_event(True))
        self.engine._handle(boundary_event(False))
        self.assertEqual(len(self.backend.injections), 2)
        self.assertEqual(self.backend.injections[-1][1], 1)

    def test_manual_russian_selection_protects_short_if_on_pause_and_space(self) -> None:
        self.engine._update(current_group=0)
        self.backend.group = 1
        self.engine._poll_current_group()
        self.assertEqual(self.engine._manual_layout_group, 1)

        with patch("keyswitch.engine.time.monotonic", return_value=10.0):
            for index, character in enumerate("ша", start=30):
                event = letter_event(character, index, 1, self.pair)
                self.engine._handle(event)
                self.engine._handle(release_event(event))

        self.engine._maybe_correct_after_pause(now=12.0)
        self.assertEqual(self.backend.injections, [])
        self.assertEqual(self.engine._manual_layout_group, 1)
        self.engine._handle(boundary_event(True, group=1))
        self.engine._handle(boundary_event(False, group=1))
        self.assertEqual(self.backend.injections, [])
        self.assertIsNone(self.engine._manual_layout_group)

        for index, character in enumerate("ша", start=50):
            event = letter_event(character, index, 1, self.pair)
            self.engine._handle(event)
            self.engine._handle(release_event(event))
        self.engine._handle(boundary_event(True, group=1))
        self.engine._handle(boundary_event(False, group=1))
        self.assertEqual(len(self.backend.injections), 1)
        self.assertEqual(self.backend.injections[0][1], 0)

    def test_short_if_converts_after_pause_without_manual_intent(self) -> None:
        self.engine._update(current_group=1)
        with patch("keyswitch.engine.time.monotonic", return_value=20.0):
            for index, character in enumerate("ша", start=30):
                event = letter_event(character, index, 1, self.pair)
                self.engine._handle(event)
                self.engine._handle(release_event(event))

        self.engine._maybe_correct_after_pause(now=22.0)

        self.assertEqual(len(self.backend.injections), 1)
        self.assertEqual(self.backend.injections[0][1], 0)
        self.assertIsNone(self.backend.injections[0][2])

    def test_two_manual_conversions_create_an_automatic_rule(self) -> None:
        self.settings.set("detection.respect_manual_layout", False)
        for _attempt in range(2):
            self.backend.group = 0
            for index, character in enumerate("qwerty", start=30):
                self.engine._handle(letter_event(character, index, 0, self.pair))
            pause_press = KeyEvent(True, 127, "Pause", "", ("", ""), 0, 0, 2000)
            self.engine._handle(pause_press)
            self.engine._handle(release_event(pause_press))
        self.assertEqual(self.engine.learning.forced_target(0, "qwerty", 2), 1)
        self.backend.group = 0
        for index, character in enumerate("qwerty", start=30):
            self.engine._handle(letter_event(character, index, 0, self.pair))
        self.engine._handle(boundary_event(True))
        self.engine._handle(boundary_event(False))
        self.assertEqual(len(self.backend.injections), 3)
        self.assertEqual(self.backend.injections[-1][1], 1)

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
        self.assertEqual(self.engine.learning.rejected_targets(0, "ghbdtn"), {1})
        self.backend.group = 0
        for index, character in enumerate("ghbdtn", start=30):
            self.engine._handle(letter_event(character, index, 0, self.pair))
        self.engine._handle(boundary_event(True))
        self.engine._handle(boundary_event(False))
        self.assertEqual([item[1] for item in self.backend.injections], [1, 0])


class HotkeyTests(unittest.TestCase):
    def test_exact_modifier_match(self) -> None:
        event = KeyEvent(True, 33, "p", "", ("p", "з"), 0, (1 << 2) | (1 << 3), 1)
        self.assertTrue(Hotkey("Ctrl+Alt+P").matches(event))
        self.assertFalse(Hotkey("Ctrl+P").matches(event))


class IndicatorTests(unittest.TestCase):
    def test_letters_and_flags_map_to_layout_specific_icons(self) -> None:
        self.assertEqual(layout_icon_name("letters", 0), "keyswitch-en")
        self.assertEqual(layout_icon_name("letters", 1), "keyswitch-ru")
        self.assertEqual(layout_icon_name("flags", 0), "keyswitch-flag-us")
        self.assertEqual(layout_icon_name("flags", 1), "keyswitch-flag-ru")
        self.assertEqual(layout_icon_name("flags", -1), "keyswitch")

    def test_invalid_indicator_style_falls_back_to_letters(self) -> None:
        self.assertEqual(normalize_indicator_style("unknown"), "letters")
        self.assertEqual(layout_label(0), "EN")
        self.assertEqual(layout_label(1), "RU")
        self.assertEqual(layout_label(3), "—")


if __name__ == "__main__":
    unittest.main()
