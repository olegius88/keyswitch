"""Authored physical sequences with exact visible-text and action outcomes."""

from __future__ import annotations

import queue
import tempfile
import unittest
from collections.abc import Iterable, Iterator, Sequence
from contextlib import contextmanager
from dataclasses import replace
from pathlib import Path
from unittest.mock import patch

from keyswitch.backend import KeyEvent
from keyswitch.constants.keyboard import SHIFT_MASK
from keyswitch.config import SettingsStore
from keyswitch.engine import KeySwitchEngine
from keyswitch.history import HistoryStore
from keyswitch.layouts import LayoutPair
import test_input_integrity as integrity
from fixture_values.counts import SEQUENCE_MATRIX_MAX_FLUSH_ITERATIONS
from fixture_values.keys import EDITOR_REPLAY_FIRST_KEY_SERIAL, SECOND_WINDOW_ID


class PhysicalSession:
    def __init__(self, root: Path, group: int) -> None:
        self.settings = SettingsStore(root / "settings.json")
        self.settings.set("detection.early_switch", False)
        self.settings.set("detection.respect_manual_layout", False)
        self.settings.set("detection.context_read_field", False)
        self.settings.set("detection.learning", False)
        self.settings.set("general.keep_history", False)
        self.settings.set("diagnostics.technical_logging", False)
        self.backend = integrity.EditorBackend()
        self.backend.group = group
        self.engine = KeySwitchEngine(
            self.settings, HistoryStore(root / "history.jsonl"), self.backend,
        )
        self.serial = EDITOR_REPLAY_FIRST_KEY_SERIAL
        self.pair = LayoutPair()

    def key(self, us: str, *, name: str = "") -> KeyEvent:
        self.serial += 1
        ru = {
            "/": ".", "?": ",", "@": '"', "#": "№", "$": ";", "^": ":", "&": "?",
            "~": "Ё", "{": "Х", "}": "Ъ", ":": "Ж", '"': "Э", "<": "Б", ">": "Ю", "|": "/",
        }.get(
            us, self.pair.translate(us, "us", "ru"),
        )
        group = self.backend.group
        character = (us, ru)[group]
        shift = bool(us) and (us.isupper() or us in '~!@#$%^&*()_+{}|:"<>?')
        return KeyEvent(
            True, self.serial, name or ("space" if character.isspace() else us),
            character, (us, ru), group, SHIFT_MASK if shift else 0, self.serial,
        )

    def send(self, event: KeyEvent) -> None:
        self.backend.type(event)
        self.engine._handle(event)

    def tap(self, event: KeyEvent) -> None:
        self.send(event)
        self.send(replace(event, pressed=False))

    def physical(self, keys: str) -> None:
        for key in keys:
            self.tap(self.key(key))

    def command(self, name: str) -> None:
        self.tap(self.key("", name=name))

    def flush(self) -> None:
        for _ in range(SEQUENCE_MATRIX_MAX_FLUSH_ITERATIONS):
            try:
                event = self.engine._events.get_nowait()
            except queue.Empty:
                return
            if not isinstance(event, KeyEvent):
                raise AssertionError("Unexpected control message in sequence replay")
            self.engine._handle(event)
        raise AssertionError("Sequence replay did not terminate")


@contextmanager
def session(group: int = 0) -> Iterator[PhysicalSession]:
    with tempfile.TemporaryDirectory(prefix="keyswitch-sequence-") as temporary:
        yield PhysicalSession(Path(temporary), group)


class InputSequenceMatrixTests(unittest.TestCase):
    def test_explicit_conversion_preserves_physical_letters_symbols_and_case(self) -> None:
        cases = (
            (0, "ghj,ktvf", "проблема"),
            (0, "[jhjij", "хорошо"),
            (0, "rjnjhe.", "которую"),
            (0, "Ghbdtn", "Привет"),
            (0, "`krf", "ёлка"),
            (0, "~krf", "Ёлка"),
            (0, "pm2", "зь2"),
            (0, "'", "э"),
            (0, ".", "ю"),
            (1, "hello", "hello"),
            (1, "@", "@"),
            (1, "pm2", "pm2"),
        )
        for group, keys, expected in cases:
            with self.subTest(group=group, keys=keys), session(group) as current:
                current.physical(keys)
                current.command("Pause")
                self.assertEqual(current.backend.text, expected)
                self.assertEqual(current.backend.caret, len(expected))
                self.assertEqual(current.backend.group, 1 - group)
                self.assertEqual(len(current.backend.injections), 1)

    def test_authored_automatic_sequences_preserve_each_literal_boundary(self) -> None:
        cases = (
            (0, "ghj,ktvf ", "проблема "),
            (0, ",b,kbjntrf ", "библиотека "),
            (0, "[jhjij ", "хорошо "),
            (0, "rjnjhe. ", "которую "),
            (0, "ghbdtn... ", "привет... "),
            (0, "/c,jhrb ", "/сборки "),
            (1, "hello/", "hello."),
            (1, "hello?", "hello,"),
        )
        for group, keys, expected in cases:
            with self.subTest(group=group, keys=keys), session(group) as current:
                current.physical(keys)
                self.assertEqual(current.backend.text, expected)
                self.assertEqual(current.backend.caret, len(expected))
                self.assertEqual(current.backend.group, 1 - group)

    def test_undo_preserves_original_punctuation_glyph_and_spacing(self) -> None:
        for spacing in (" ", "\u00a0", "\u202f", "\u2009"):
            for suffix in (",", ".", ";", "]", "'", "..."):
                with self.subTest(spacing=repr(spacing), suffix=suffix), session() as current:
                    current.physical("/c,jhrb" + suffix + spacing)
                    self.assertEqual(current.backend.text, "/сборки" + suffix + spacing)
                    current.command("Pause")
                    self.assertEqual(current.backend.text, "/c,jhrb" + suffix + spacing)
                    self.assertEqual(current.backend.caret, len("/c,jhrb" + suffix + spacing))
                    self.assertEqual(current.backend.group, 0)

    def test_correct_technical_and_colloquial_words_remain_literal(self) -> None:
        # `дюп` reads `l.g` in the other layout and the shipped model asks for it;
        # the replacement-shape refusal in the context policy keeps the word, so
        # this is no longer a disclosed regression of the installed pair.
        cases = (
            (0, "npm install  ", "npm install  "),
            (0, "don't stop. ", "don't stop. "),
            (0, "object.field ", "object.field "),
            (1, "akel ", "флуд "),
            (1, "ken ", "лут "),
            (1, "l.g ", "дюп "),
        )
        for group, keys, expected in cases:
            with self.subTest(group=group, keys=keys), session(group) as current:
                current.physical(keys)
                self.assertEqual(current.backend.text, expected)
                self.assertEqual(current.backend.group, group)

    def test_edit_before_boundary_release_preserves_surrounding_text(self) -> None:
        for name, expected in (
            ("Left", "hello  /c,jhrb... "),
            ("Pointer", "hello  /c,jhrb... "),
            ("BackSpace", "hello  /c,jhrb..."),
        ):
            with self.subTest(edit=name), session() as current:
                current.physical("hello  /c,jhrb...")
                boundary = current.key(" ")
                current.send(boundary)
                current.command(name)
                current.send(replace(boundary, pressed=False))
                self.assertEqual(current.backend.text, expected)
                self.assertEqual(current.backend.group, 0)
                self.assertEqual(current.backend.injections, [])

    def test_whitespace_run_disarms_old_word_undo_without_rewriting_it(self) -> None:
        for spacing in ("  ", " \u00a0", "\u202f ", "\u2009\u2009", "   "):
            with self.subTest(spacing=repr(spacing)), session() as current:
                current.physical("/c,jhrb" + spacing)
                self.assertEqual(current.backend.text, "/сборки" + spacing)
                current.command("Pause")
                self.assertEqual(current.backend.text, "/сборки" + spacing)
                self.assertEqual(current.backend.group, 0)
                self.assertEqual(len(current.backend.injections), 1)

    def test_action_boundary_releases_exact_completed_text_once(self) -> None:
        for name in ("Return", "Tab"):
            for keys, expected in (
                ("ghj,ktvf", "проблема"),
                ("ghbdtn...", "привет..."),
                ("/c,jhrb...", "/сборки..."),
            ):
                with self.subTest(name=name, keys=keys), session() as current:
                    current.physical(keys)
                    action = replace(current.key("", name=name), deferred=True)
                    delivered: list[str] = []

                    def complete(deliver: bool) -> int:
                        if deliver:
                            delivered.append(current.backend.text)
                            current.backend.type(replace(action, deferred=False))
                        return 0

                    with patch.object(current.backend, "complete_action", side_effect=complete):
                        current.engine._handle(action)
                        self.assertEqual(delivered, [])
                        current.engine._handle(replace(action, pressed=False))
                    self.assertEqual(delivered, [expected])
                    if name == "Return":
                        self.assertEqual(current.backend.submissions, [expected])
                        self.assertEqual(current.backend.text, "")
                    else:
                        self.assertEqual(current.backend.text, "another field")
                        self.assertEqual(current.backend.window, SECOND_WINDOW_ID)

    def test_manual_command_survives_release_order_with_an_ambiguous_internal_key(self) -> None:
        for pause_first in (False, True):
            with self.subTest(pause_first=pause_first), session() as current:
                current.physical("ghj,ktv")
                letter = current.key("f")
                pause = current.key("", name="Pause")
                current.send(letter)
                current.send(pause)
                self.assertEqual(current.backend.text, "ghj,ktvf")
                releases = (pause, letter) if pause_first else (letter, pause)
                current.send(replace(releases[0], pressed=False))
                self.assertEqual(current.backend.text, "ghj,ktvf")
                current.send(replace(releases[1], pressed=False))
                self.assertEqual(current.backend.text, "проблема")
                self.assertEqual(len(current.backend.injections), 1)
                self.assertEqual(current.engine._pressed, set())

    def test_safe_replayed_releases_keep_the_full_word_for_continuation_and_pause(self) -> None:
        for release_names in (("Shift_L",), ("Shift_L", "Control_L")):
            with self.subTest(releases=release_names), session() as current:
                current.physical("ghj,ktv")
                inject = current.backend.inject_correction

                def with_releases(
                    strokes: Iterable[KeyEvent], target_group: int,
                    boundary: KeyEvent | None, source_group: int | None = None,
                    late: Sequence[KeyEvent] = (), trailing: Sequence[KeyEvent] = (),
                ) -> int:
                    inject(strokes, target_group, boundary, source_group, late, trailing)
                    for name in release_names:
                        current.engine.enqueue(replace(current.key("", name=name), pressed=False, group=0))
                    return len(release_names)

                with patch.object(current.backend, "inject_correction", side_effect=with_releases):
                    current.command("Pause")
                current.flush()
                current.physical("f")
                self.assertEqual(current.backend.text, "проблема")
                current.command("Pause")
                self.assertEqual(current.backend.text, "ghj,ktvf")
                self.assertEqual(current.backend.caret, len("ghj,ktvf"))


if __name__ == "__main__":
    unittest.main()
