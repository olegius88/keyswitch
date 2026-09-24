"""Disclosed authored acceptance sequences with unmodified application defaults.

Portable lexical inputs and packaged models make the observations reproducible.
EditorBackend models a passive editor, not native Windows/X11 interception.
These examples are development regressions, never a new independent holdout.
"""

from __future__ import annotations


from collections.abc import Iterable, Iterator
from contextlib import contextmanager
from dataclasses import replace
from pathlib import Path
import sys
import tempfile
from typing import ClassVar
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

from auxiliary_runtime_evidence import packaged_intent
from reference_lexicon import reference_models
from context_physical_keys import KEYS
from keyswitch.backend import KeyEvent
from keyswitch.constants.keyboard import SHIFT_MASK
from keyswitch.config import SettingsStore
from keyswitch.constants.settings_defaults import (
    DEFAULT_EARLY_SWITCH_MIN_LENGTH,
    DEFAULT_PAUSE_DELAY_SECONDS,
)
from keyswitch.engine import KeySwitchEngine
from keyswitch.history import HistoryStore
from keyswitch.intent_model import IntentModelStatus, LinearNgramModel
from keyswitch.language_model import LanguageModel
from test_input_integrity import EditorBackend
from fixture_values.clock import (
    DEFAULT_SEQUENCE_EXTENDED_IDLE_SECONDS,
    DEFAULT_SEQUENCE_IDLE_SECONDS,
    DEFAULT_SEQUENCE_REMAINING_IDLE_SECONDS,
    DEFAULT_SEQUENCE_SHORT_IDLE_SECONDS,
    FAKE_CLOCK_START_SECONDS,
    SIMULATED_KEY_HOLD_SECONDS,
    SIMULATED_KEY_RELEASE_GAP_SECONDS,
)
from fixture_values.counts import LOOKAHEAD_SLICE_END_OFFSET
from fixture_values.keys import DEFAULT_SEQUENCE_RETURN_KEYCODE
from keyswitch.constants.units import MILLISECONDS_PER_SECOND


class _Clock:
    def __init__(self) -> None:
        self.now = FAKE_CLOCK_START_SECONDS

    def read(self) -> float:
        return self.now


class _Session:
    def __init__(self, engine: KeySwitchEngine, backend: EditorBackend, clock: _Clock) -> None:
        self.engine, self.backend, self.clock = engine, backend, clock

    def timers(self) -> None:
        self.engine._expire_deferred_action()
        self.engine._expire_manual_correction()
        self.engine._poll_current_group()
        self.engine._maybe_correct_after_pause()
        self.engine._expire_learning_prompt()

    def idle(self, seconds: float = DEFAULT_SEQUENCE_IDLE_SECONDS) -> None:
        self.clock.now += seconds
        self.timers()

    def select_layout(self, group: int) -> None:
        self.backend.group = group
        self.engine._poll_current_group()

    def tap(self, event: KeyEvent) -> None:
        self.clock.now += SIMULATED_KEY_HOLD_SECONDS
        pressed = replace(event, timestamp=round(self.clock.now * MILLISECONDS_PER_SECOND))
        self.backend.type(pressed)
        self.engine._handle(pressed)
        self.clock.now += SIMULATED_KEY_RELEASE_GAP_SECONDS
        released = replace(pressed, pressed=False, timestamp=round(self.clock.now * MILLISECONDS_PER_SECOND))
        self.backend.type(released)
        self.engine._handle(released)
        self.timers()

    def type(self, intended: str, group: int, *, idle_after_words: bool = True) -> None:
        # Freeze physical keys before automatic layout changes affect glyphs.
        keys = [next(key for key in KEYS if key.characters[group] == char) for char in intended]
        for index, key in enumerate(keys):
            observed = key.characters[self.backend.group]
            self.tap(KeyEvent(
                True, key.keycode, "space" if observed == " " else observed,
                observed, key.characters, self.backend.group,
                SHIFT_MASK if key.shift else 0, 0,
            ))
            following = intended[index + 1:index + LOOKAHEAD_SLICE_END_OFFSET]
            if idle_after_words and not intended[index].isspace() and (not following or following.isspace()):
                self.idle()

    def submit(self) -> None:
        self.tap(KeyEvent(True, DEFAULT_SEQUENCE_RETURN_KEYCODE, "Return", "", ("", ""), self.backend.group, 0, 0))
        self.idle()


class DefaultInputSequenceTests(unittest.TestCase):
    models: ClassVar[dict[int, LanguageModel]]
    intent: ClassVar[tuple[LinearNgramModel, IntentModelStatus]]

    @classmethod
    def setUpClass(cls) -> None:
        cls.models = reference_models(False)
        cls.intent = packaged_intent(Path(__file__).resolve().parents[1])

    @contextmanager
    def session(self, group: int = 1, application: str = "Telegram") -> Iterator[_Session]:
        with tempfile.TemporaryDirectory(prefix="keyswitch-default-sequence-") as directory:
            root = Path(directory)
            settings = SettingsStore(root / "settings.json")
            expected = {
                "early_switch": False, "early_switch_min_length": DEFAULT_EARLY_SWITCH_MIN_LENGTH,
                "correct_on_pause": True, "pause_delay_seconds": DEFAULT_PAUSE_DELAY_SECONDS,
                "respect_manual_layout": True, "learning": True,
                "context_policy": "assist", "context_aware": True,
                "context_read_field": True, "aggressive": True,
            }
            self.assertEqual({name: settings.get("detection." + name) for name in expected}, expected)
            backend, clock = EditorBackend(), _Clock()
            backend.group = group

            def load(locale: str, extra_words: Iterable[str] = ()) -> LanguageModel:
                # Mirrors what the engine loads: the onboard lexicon plus the packaged supplement.
                return self.models[{"en_US": 0, "ru_RU": 1}[locale]]

            with patch.object(backend, "active_application", return_value=application), \
                 patch("keyswitch.engine.LanguageModel.load", side_effect=load), \
                 patch("keyswitch.engine.LinearNgramModel.try_load_default", return_value=self.intent), \
                 patch("keyswitch.engine.time.monotonic", side_effect=clock.read):
                engine = KeySwitchEngine(settings, HistoryStore(root / "history.jsonl"), backend)
                self.assertIsNotNone(engine.prefix_model)
                self.assertIsNotNone(engine.context_policy.model)
                yield _Session(engine, backend, clock)

    def assert_sent_once(self, session: _Session, expected: str) -> None:
        before = session.backend.text
        session.submit()
        self.assertEqual(
            (before, session.backend.text, session.backend.submissions),
            (expected, "", [expected]),
        )

    def test_russian_chat_keeps_spelling_spaces_and_one_submission(self) -> None:
        for text in ("пришли гифку  позже. ", "этот флуд  закончился. ", "севодня  будет созвон. "):
            with self.subTest(text=text), self.session() as session:
                session.type(text, 1)
                self.assert_sent_once(session, text)

    def test_code_editor_keeps_russian_comment_before_manual_english_insert(self) -> None:
        with self.session(application="Code") as session:
            session.type("// сохрани гифку  ", 1)
            session.select_layout(0)
            session.type("preview ", 0)
            self.assertEqual(
                (session.backend.text, session.backend.submissions),
                ("// сохрани гифку  preview ", []),
            )

    def test_manual_layout_changes_preserve_mixed_chat_and_spacing(self) -> None:
        with self.session() as session:
            session.type("готовим  ", 1)
            session.select_layout(0)
            session.type("websocket  ", 0)
            session.select_layout(1)
            session.type("завтра. ", 1)
            self.assert_sent_once(session, "готовим  websocket  завтра. ")

    def test_idle_corrects_short_wrong_layout_before_spaces_and_send(self) -> None:
        with self.session(group=0) as session:
            session.type("кот", 1, idle_after_words=False)
            self.assertEqual(session.backend.text, "rjn")
            self.assertEqual(session.backend.submissions, [])
            session.idle(DEFAULT_SEQUENCE_SHORT_IDLE_SECONDS)
            self.assertEqual(session.backend.text, "rjn")
            session.idle(DEFAULT_SEQUENCE_REMAINING_IDLE_SECONDS)
            session.type("  ", 1)
            self.assert_sent_once(session, "кот  ")

    def test_wrong_layout_continues_with_actual_layout_after_early_switch(self) -> None:
        with self.session(group=0) as session:
            session.type("привет  ", 1)
            self.assert_sent_once(session, "привет  ")

    def test_a_word_in_doubt_follows_its_converted_neighbour_in_any_application(self) -> None:
        """`tot привет` stayed in Firefox, whose name the context model does not know, and
        after a pause before the space anywhere (0.31.0 and 0.31.1 logs, 24.09.2026)."""
        for application in ("firefox", "Telegram"):
            for pause in (True, False):
                with self.subTest(application=application, pause=pause), \
                        self.session(group=0, application=application) as session:
                    session.type("еще привет ", 1, idle_after_words=pause)
                    self.assertEqual(session.backend.text, "еще привет ")

    def test_punctuation_and_repeated_spaces_never_submit_without_return(self) -> None:
        with self.session() as session:
            session.type("проверь  письмо!  ответь потом. ", 1)
            session.idle(DEFAULT_SEQUENCE_EXTENDED_IDLE_SECONDS)
            self.assertEqual(session.backend.text, "проверь  письмо!  ответь потом. ")
            self.assertEqual(session.backend.submissions, [])

    def test_two_return_presses_submit_two_messages_without_release_duplicates(self) -> None:
        with self.session() as session:
            session.type("готово  ", 1)
            session.submit()
            session.type("спасибо ", 1)
            session.submit()
            self.assertEqual((session.backend.text, session.backend.submissions), ("", ["готово  ", "спасибо "]))


if __name__ == "__main__":
    unittest.main()
