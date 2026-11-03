"""Typo correction: the counted lists, the noisy-channel choice, the keys a correction types and the engine's plan."""

from __future__ import annotations

import hashlib
import json
import math
import re
import sys
import tempfile
import unittest
from collections.abc import Iterable, Sequence
from dataclasses import replace
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

from build_typo_frequency import SOURCES, Source, build, read_counts  # noqa: E402

from keyswitch.backend import KeyEvent, SHIFT_MASK, erasing_injection  # noqa: E402
from keyswitch.config import SettingsStore  # noqa: E402
from keyswitch.constants.keyboard import ALT_MASK, CONTROL_MASK  # noqa: E402
from keyswitch.constants.typo import (  # noqa: E402
    TYPO_ALPHABETS,
    TYPO_COUNT_LOG_SCALE,
    TYPO_FREQUENCY_MAX_BYTES,
    TYPO_FREQUENCY_SCHEMA_VERSION,
    TYPO_MIN_LETTERS,
    TYPO_SECTIONS,
    TYPO_STORED_MIN_COUNT,
    PERSONAL_WORD_HASH_HEX_CHARACTERS,
    PERSONAL_WORD_SALT_BYTES,
    PERSONAL_WORDS_SCHEMA_VERSION,
)
from keyswitch.engine import CorrectionPlan, InsertionPoint, KeySwitchEngine  # noqa: E402
from keyswitch.history import HistoryStore  # noqa: E402
from keyswitch.identifier_lexicon import IdentifierLexicon  # noqa: E402
from keyswitch.layouts import RU_KEYS, US_KEYS, LayoutPair  # noqa: E402
from keyswitch.learning import LearnedRule  # noqa: E402
from keyswitch.personal_words import PersonalWords  # noqa: E402
from keyswitch.typo_correction import (  # noqa: E402
    CORRECT_TYPOS_DESCRIPTION, CORRECT_TYPOS_ENGLISH_DESCRIPTION,
    Candidate,
    Edit,
    KeyTemplates,
    TypoChoice,
    TypoCorrector,
    TypoJudgement,
    TypoLexicon,
    TypoPolicy,
    corrected_keys,
    edits,
    keyboard_neighbours,
    known_elsewhere,
    packaged_lexicon,
    typo_resource_path,
    word_shape,
)
from keyswitch.windows_ui_model import ALL_SETTING_SPECS  # noqa: E402
from keyswitch.constants.macos import MAC_VK_BACKSPACE  # noqa: E402
from keyswitch.constants.units import BYTES_PER_MEBIBYTE  # noqa: E402
from keyswitch.constants.wayland import EVDEV_KEY_BACKSPACE  # noqa: E402
from keyswitch.constants.windows import VK_BACK  # noqa: E402
from keyswitch.macos_backend import MacBackend  # noqa: E402
from keyswitch.wayland_backend import WaylandBackend  # noqa: E402
from keyswitch.windows_backend import WindowsBackend  # noqa: E402
from engine_models import share_engine_models  # noqa: E402
from test_macos_backend import FakeMacAPI  # noqa: E402
from test_wayland_backend import FakeWaylandAPI, stroke_event  # noqa: E402
from test_windows_backend import FakeWindowsAPI, key_event  # noqa: E402
from test_x11_backend import backend_with  # noqa: E402
from test_engine_behaviour import FakeBackend, plain_key  # noqa: E402
from test_input_integrity import EditorBackend  # noqa: E402
from fixture_values.keys import BACKSPACE_KEYCODE, PAUSE_KEYCODE, SYNTHETIC_KEYCODE_BASE, Z_KEYCODE  # noqa: E402
from fixture_values.platform import XTEST_FAKE_KEY_EVENT_IS_PRESS_ARG_INDEX  # noqa: E402
from fixture_values.typo import (  # noqa: E402
    TYPO_FIXTURE_KEY_TIMESTAMP,
    TYPO_FIXTURE_LINES_EN,
    TYPO_FIXTURE_LINES_RU,
)


def setUpModule() -> None:
    share_engine_models()


def lexicon_file(directory: Path, locale: str, lines: Iterable[str]) -> Path:
    source = next(item for item in SOURCES if item.locale == locale)
    built = build(Source(locale, source.path_in_repository, source.default, "0" * len(source.sha256)), lines, "test")
    path = directory / f"typo-frequency-{locale}.bin"
    path.write_bytes(built.file)
    return path


def small_corrector(directory: Path, policy: TypoPolicy = TypoPolicy()) -> TypoCorrector:
    lexicons = {
        "ru_RU": TypoLexicon.load(lexicon_file(directory, "ru_RU", TYPO_FIXTURE_LINES_RU), "ru_RU"),
        "en_US": TypoLexicon.load(lexicon_file(directory, "en_US", TYPO_FIXTURE_LINES_EN), "en_US"),
    }
    return TypoCorrector(lexicons, policy)


def press(character: str, group: int, keycode: int, state: int = 0) -> KeyEvent:
    pair = LayoutPair()
    lower = character.casefold()
    other = pair.translate(lower, "ru" if group else "us", "us" if group else "ru")
    other = other.upper() if character != lower else other
    characters = (other, character) if group else (character, other)
    return KeyEvent(True, keycode, characters[0].casefold(), character, characters, group, state,
                    TYPO_FIXTURE_KEY_TIMESTAMP)


def word_keys(text: str, group: int) -> tuple[KeyEvent, ...]:
    return tuple(press(character, group, keycode, SHIFT_MASK if character.isupper() else 0)
                 for keycode, character in enumerate(text, SYNTHETIC_KEYCODE_BASE))


class KeyboardTests(unittest.TestCase):
    def test_neighbours_follow_the_staggered_rows_of_both_layouts(self) -> None:
        russian, english = keyboard_neighbours(RU_KEYS), keyboard_neighbours(US_KEYS)
        self.assertEqual(russian["е"], "акнп")
        self.assertEqual(english["g"], "bfhtvy")
        # Keys that are signs in one layout are letters in the other: `х` touches `з`, `ъ` and `ж`, `э`.
        self.assertEqual(set(russian["х"]), set("зъжэ"))
        self.assertNotIn("[", english["p"])
        # The key left of `1` touches no letter.
        self.assertNotIn("ё", russian)
        self.assertNotIn("ё", "".join(russian.values()))
        with self.assertRaisesRegex(ValueError, "key rows"):
            keyboard_neighbours(US_KEYS + "x")


class LexiconTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)

    def test_stored_forms_weigh_their_counts_and_rare_forms_occur(self) -> None:
        path = lexicon_file(self.directory, "ru_RU", TYPO_FIXTURE_LINES_RU)
        lexicon = TypoLexicon.load(path, "ru_RU")
        counts, _ = read_counts(TYPO_FIXTURE_LINES_RU, TYPO_ALPHABETS["ru_RU"])
        self.assertEqual(lexicon.weight("клавиатура"), round(math.log(counts["клавиатура"]) * TYPO_COUNT_LOG_SCALE))
        self.assertIsNone(lexicon.weight("клавиатурка"))
        self.assertIsNone(lexicon.weight("abc"))
        english = TypoLexicon.load(lexicon_file(self.directory, "en_US", TYPO_FIXTURE_LINES_EN), "en_US")
        self.assertIsNone(english.weight("клавиатура"))
        self.assertFalse(english.occurs("клавиатура"))
        self.assertIsNone(lexicon.weight(""))
        # A form too rare to store occurs all the same, and so does its spelling with `е` for `ё`.
        self.assertTrue(lexicon.occurs("клавиатурка"))
        self.assertTrue(lexicon.occurs("зелёного"))
        self.assertFalse(lexicon.occurs("клавиаутра"))
        self.assertFalse(lexicon.occurs("keyboard"))
        self.assertNotIn("keyboard", counts)
        self.assertEqual(lexicon.size, len([form for form, count in counts.items()
                                            if count >= TYPO_STORED_MIN_COUNT and len(form) >= TYPO_MIN_LETTERS - 1]))
        self.assertTrue(lexicon.version.startswith("typo-frequency-"))
        header = json.loads(path.read_bytes().partition(b"\n")[0])
        self.assertEqual(header["source"]["license"].split(" (")[0], "CC BY-SA 4.0")

    def test_a_damaged_file_does_not_load(self) -> None:
        path = lexicon_file(self.directory, "ru_RU", TYPO_FIXTURE_LINES_RU)
        raw = path.read_bytes()
        header_line, _, payload = raw.partition(b"\n")
        header = json.loads(header_line)

        def written(changed: dict[str, object], body: bytes = payload) -> Path:
            target = self.directory / f"damaged-{len(list(self.directory.iterdir()))}.bin"
            target.write_bytes(json.dumps({**header, **changed}).encode() + b"\n" + body)
            return target

        cases: list[tuple[Path, str]] = [
            (written({"schema_version": TYPO_FREQUENCY_SCHEMA_VERSION + 1}), "header"),
            (written({}, payload + b"x"), "header"),
            (written({"sections": []}), "header"),
            (written({"sections": {name: -1 for name in TYPO_SECTIONS}}), "sections"),
            (written({"seen": {"bits": 0, "hashes": 1}}), "sections"),
        ]
        no_header = self.directory / "no-header.bin"
        no_header.write_bytes(b"{}")
        cases.append((no_header, "no header"))
        short = header["sections"]["seen"]
        cut = written({"sections": {**header["sections"], "seen": 0},
                       "payload_sha256": hashlib.sha256(payload[:len(payload) - short]).hexdigest()},
                      payload[:len(payload) - short])
        cases.append((cut, "payload"))
        for target, message in cases:
            with self.subTest(message=message), self.assertRaisesRegex(ValueError, message):
                TypoLexicon.load(target, "ru_RU")
        with patch("keyswitch.typo_correction.TYPO_FREQUENCY_MAX_BYTES", 0), \
                self.assertRaisesRegex(ValueError, "too large"):
            TypoLexicon.load(path, "ru_RU")

    def test_packaged_lists_load_once_and_a_missing_or_damaged_one_is_none(self) -> None:
        self.assertIs(packaged_lexicon("ru_RU"), packaged_lexicon("ru_RU"))
        self.assertIsNotNone(packaged_lexicon("en_US"))
        self.assertIsNone(packaged_lexicon("de_DE"))
        packaged_lexicon.cache_clear()
        try:
            with patch("keyswitch.typo_correction.typo_resource_path", return_value=self.directory / "none.bin"):
                self.assertIsNone(packaged_lexicon("ru_RU"))
            damaged = self.directory / "damaged.bin"
            damaged.write_bytes(b"{}\n")
            with patch("keyswitch.typo_correction.typo_resource_path", return_value=damaged):
                self.assertIsNone(packaged_lexicon("en_US"))
        finally:
            packaged_lexicon.cache_clear()
        self.assertEqual(typo_resource_path("ru_RU").name, "typo-frequency-ru_RU.bin")

    def test_the_shipped_lists_are_the_receipt_s_derivation(self) -> None:
        receipt = json.loads((typo_resource_path("ru_RU").parent / "typo-frequency.receipt.json").read_bytes())
        for locale in TYPO_ALPHABETS:
            with self.subTest(locale=locale):
                raw = typo_resource_path(locale).read_bytes()
                entry = receipt["locales"][locale]
                self.assertEqual(hashlib.sha256(raw).hexdigest(), entry["file"]["sha256"])
                header = json.loads(raw.partition(b"\n")[0])
                self.assertEqual(header["selection"], entry["selection"])
                self.assertEqual(header["source"]["sha256"], entry["source"]["sha256"])
                lexicon = packaged_lexicon(locale)
                assert lexicon is not None
                self.assertEqual(lexicon.size, entry["selection"]["stored"])


class ChannelTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.corrector = small_corrector(Path(self.temporary.name))

    def test_each_edit_undoes_its_slip_and_names_the_typed_letters_it_keeps(self) -> None:
        typed = "клавиаутра"
        cases = (
            (Edit("transposition", typed.index("у")), "клавиатура"),
            (Edit("deletion", typed.index("у")), "клавиатра"),
            (Edit("insertion", 0, "ы"), "ыклавиаутра"),
            (Edit("substitution", 0, "р"), "рлавиаутра"),
        )
        for edit, corrected in cases:
            with self.subTest(operation=edit.operation):
                self.assertEqual(edit.apply(typed), corrected)
                sources = edit.sources(len(typed))
                # Every letter of the corrected word is a typed one, but the one the edit brings.
                self.assertEqual("".join(edit.letter if source is None else typed[source] for source in sources),
                                 corrected)
                self.assertEqual(sources.count(None), int(bool(edit.letter)))

    def test_every_slip_is_undone_once(self) -> None:
        found = list(edits("aab", "ab", {"a": "b"}))
        self.assertEqual([edit for edit in found if edit.operation == "transposition"], [Edit("transposition", 1)])
        self.assertEqual(sum(edit.operation == "deletion" for edit in found), len("aab"))
        # A letter in excess repeats a letter beside it or lies on a key next to one: `c` between `a` and `b` is no slip.
        self.assertFalse(any(edit.operation == "deletion" for edit in edits("acb", "abc", {"a": "b"})))
        self.assertEqual(sum(edit.operation == "insertion" for edit in found), (len("aab") + 1) * len("ab"))
        self.assertEqual(sum(edit.operation == "substitution" for edit in found), len("aa"))

    def test_shape_wants_enough_letters_of_the_alphabet_and_at_most_a_leading_capital(self) -> None:
        alphabet = TYPO_ALPHABETS["ru_RU"]
        self.assertTrue(word_shape("Клавиаутра", alphabet))
        self.assertTrue(word_shape("клавиаутра", alphabet))
        self.assertFalse(word_shape("клАвиаутра", alphabet))
        self.assertFalse(word_shape("клави2тра", alphabet))
        self.assertFalse(word_shape("к" * TYPO_MIN_LETTERS, alphabet.replace("к", "")))
        self.assertFalse(word_shape("к" * (TYPO_MIN_LETTERS - 1), alphabet))

    def judge(self, word: str, locale: str = "ru_RU", *, capital_allowed: bool = True,
              known: Iterable[str] = ()) -> TypoJudgement:
        layouts = {"ru_RU": "ru", "en_US": "us"}
        other = "en_US" if locale == "ru_RU" else "ru_RU"
        reading = LayoutPair().translate(word, layouts[locale], layouts[other])
        return self.corrector.judge(word, locale, {other: reading}, capital_allowed=capital_allowed,
                                    known=set(known).__contains__)

    def test_a_word_is_corrected_only_where_nothing_reads_it_as_a_word(self) -> None:
        self.assertEqual(self.judge("клав").reason, "shape")
        self.assertEqual(self.corrector.judge("klaviatura", "de_DE", {}, capital_allowed=True, known=bool).reason,
                         "shape")
        self.assertEqual(self.judge("Клавиаутра", capital_allowed=False).reason, "capital")
        self.assertEqual(self.judge("клавиатурка").reason, "occurs")
        # `руддщ` typed in the Russian layout is `hello`: a layout question, not a typo.
        self.assertEqual(self.judge("руддщ").reason, "other_reading")
        self.assertEqual(self.judge("клавиаутра", known=("клавиаутра",)).reason, "known")
        self.assertEqual(self.judge("rkfdbfenhf", "en_US", known=("клавиаутра",)).reason, "known")
        self.assertEqual(self.judge("щщщщщщ").reason, "no_candidate")
        corrected = self.judge("Клавиаутра")
        self.assertEqual(corrected.reason, "corrected")
        assert corrected.choice is not None
        self.assertEqual((corrected.choice.word, corrected.choice.edit.operation), ("Клавиатура", "transposition"))
        self.assertEqual(corrected.choice.margin, math.inf)
        self.assertEqual(corrected.count_order, math.floor(math.log10(math.exp(
            corrected.choice.weight / TYPO_COUNT_LOG_SCALE))))
        self.assertEqual(self.judge("keybaord", "en_US").reason, "corrected")

    def test_a_rare_or_unclear_best_candidate_is_left(self) -> None:
        rare = self.judge("ркдкий")
        self.assertEqual(rare.reason, "rare")
        assert rare.choice is not None
        self.assertEqual(rare.choice.word, "редкий")
        # `клавиатурв` is `клавиатура` or `клавиатуры`: both one key away.
        unclear = self.judge("клавиатурв")
        self.assertEqual(unclear.reason, "ambiguous")
        self.assertEqual(unclear.candidates, len(("клавиатура", "клавиатуры")))
        self.assertEqual(TypoJudgement("occurs").count_order, -1)

    def test_the_slips_of_one_candidate_add_up_and_the_likeliest_names_it(self) -> None:
        both = Candidate("кот", round(math.log(len("ab")) * TYPO_COUNT_LOG_SCALE),
                         (Edit("deletion", 1), Edit("transposition", 0)))
        judgement = self.corrector.decide([both], False)
        assert judgement.choice is not None
        self.assertEqual(judgement.choice.edit.operation, "transposition")
        self.assertEqual(self.corrector.decide([], False).reason, "no_candidate")


class KnowledgeTests(unittest.TestCase):
    def test_every_source_the_engine_knows_a_word_from_counts(self) -> None:
        lexicons = [{"привет": 1}]
        terms = {"latin": {"kubectl": 1}}

        def known(text: str, excluded: Iterable[str] = ()) -> bool:
            return known_elsewhere(text, lexicons=lexicons, terms=terms, identifiers=None, excluded=set(excluded))

        self.assertTrue(known("Привет"))
        self.assertTrue(known("kubectl"))
        self.assertTrue(known("слово", ("слово",)))
        self.assertFalse(known("слово"))
        identifiers = IdentifierLexicon(frozenset({"usestate"}), "test", "test")
        self.assertTrue(known_elsewhere("useState", lexicons=(), terms={}, identifiers=identifiers, excluded=()))
        with patch("keyswitch.typo_correction.LanguageDetector.is_protected_token", return_value=True):
            self.assertTrue(known("слово"))


class KeyTests(unittest.TestCase):
    def test_templates_learn_the_letters_of_every_layout_from_plain_presses(self) -> None:
        templates = KeyTemplates()
        key = press("и", 1, SYNTHETIC_KEYCODE_BASE)
        for ignored in (replace(key, pressed=False), replace(key, synthetic=True), replace(key, state=CONTROL_MASK),
                        replace(key, state=ALT_MASK), replace(key, character="", characters=("", ""))):
            templates.observe(ignored)
        self.assertIsNone(templates.key(1, "и", 1))
        templates.observe(key)
        learned = templates.key(0, "b", 1)
        assert learned is not None
        self.assertEqual((learned.keycode, learned.group, learned.character), (SYNTHETIC_KEYCODE_BASE, 1, "и"))
        self.assertIsNot(learned, key)

    def test_corrected_keys_reuse_typed_presses_and_take_templates_for_new_letters(self) -> None:
        typed = word_keys("Клавматура", 1)
        templates = KeyTemplates()
        choice = TypoChoice("Клавиатура", Edit("substitution", len("клав"), "и"), 0, 1, math.inf)
        self.assertIsNone(corrected_keys(typed, 1, choice, templates))
        templates.observe(press("и", 1, Z_KEYCODE))
        keys = corrected_keys(typed, 1, choice, templates)
        assert keys is not None
        self.assertEqual("".join(key.character_for(1) for key in keys), "Клавиатура")
        self.assertIs(keys[0], typed[0])
        self.assertEqual(keys[len("клав")].keycode, Z_KEYCODE)
        # A swap that moves the capital takes a template for the letter whose case changes.
        swapped = word_keys("Лкавиатура", 1)
        swap = TypoChoice("Клавиатура", Edit("transposition", 0), 0, 1, math.inf)
        self.assertIsNone(corrected_keys(swapped, 1, swap, templates))
        templates.observe(press("К", 1, PAUSE_KEYCODE, SHIFT_MASK))
        templates.observe(press("л", 1, SYNTHETIC_KEYCODE_BASE))
        moved = corrected_keys(swapped, 1, swap, templates)
        assert moved is not None
        self.assertEqual("".join(key.character_for(1) for key in moved), "Клавиатура")


class TypoEditor(EditorBackend):
    """The editor of the integrity tests, erasing as many characters as a typo correction says the word has."""

    def __init__(self) -> None:
        super().__init__()
        self.erased: list[int | None] = []

    def inject_correction(
        self, strokes: Iterable[KeyEvent], target_group: int,
        boundary: KeyEvent | None, source_group: int | None = None,
        late: Sequence[KeyEvent] = (),
        trailing: Sequence[KeyEvent] = (),
        kept_tail: int = 0,
        erased: int | None = None,
    ) -> int:
        word = tuple(strokes)
        self.erased.append(erased)
        count = (len(word) if erased is None else erased) + len(trailing) + int(boundary is not None) + len(late)
        start = self.caret - count
        if start < 0:
            raise AssertionError("Correction attempted to erase text outside the field")
        replacement = ("".join(stroke.character_for(target_group) for stroke in word)
                       + "".join(stroke.character for stroke in trailing) + (boundary.character if boundary else "")
                       + "".join(stroke.character_for(target_group) for stroke in late))
        self.text = self.text[:start] + replacement + self.text[self.caret:]
        self.caret = start + len(replacement)
        return FakeBackend.inject_correction(self, word, target_group, boundary, source_group, late, trailing,
                                             kept_tail)


class EngineTypoTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        root = Path(self.temporary.name)
        self.settings = SettingsStore(root / "config.json")
        # The assistant keeps the observed text a capital is judged by; the early switch would convert prefixes.
        self.settings.set("detection.early_switch", False)
        self.settings.set("detection.respect_manual_layout", False)
        self.settings.set("diagnostics.technical_logging", True)
        self.editor = TypoEditor()
        self.backend: EditorBackend = self.editor
        self.backend.group = 1
        self.history = HistoryStore(root / "history.jsonl")
        self.engine = KeySwitchEngine(self.settings, self.history, self.backend)
        self.pair = LayoutPair()
        self.serial = SYNTHETIC_KEYCODE_BASE

    def tap(self, character: str, group: int | None = None) -> None:
        self.serial += 1
        source = self.backend.group if group is None else group
        name = "space" if character == " " else character.casefold()
        event = press(character, source, self.serial, SHIFT_MASK if character.isupper() else 0)
        event = replace(event, key_name=name)
        for item in (event, replace(event, pressed=False)):
            self.backend.type(item)
            self.engine._handle(item)

    def type(self, text: str, group: int | None = None) -> None:
        for character in text:
            self.tap(character, group)

    def events(self, logs: list[str], name: str) -> list[dict[str, object]]:
        found: list[dict[str, object]] = []
        for line in logs:
            if "TECHNICAL " in line:
                payload = json.loads(line.split("TECHNICAL ", 1)[1])
                if payload["event"] == name:
                    found.append(payload)
        return found

    def test_a_misspelt_word_kept_as_typed_is_corrected_at_its_space_and_undo_brings_it_back(self) -> None:
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type("клавиаутра ")
        self.assertEqual(self.backend.text, "клавиатура ")
        self.assertEqual(self.engine.snapshot.last_action, "клавиаутра → клавиатура")
        self.assertEqual([(entry.original, entry.replacement) for entry in self.history.read()],
                         [("клавиаутра", "клавиатура")])
        planned = self.events(logs.output, "typo_correction_planned")
        self.assertEqual(len(planned), 1)
        self.assertEqual((planned[0]["operation"], planned[0]["letters"]), ("transposition", len("клавиаутра")))
        self.assertIsNone(planned[0]["margin"])
        self.assertNotIn("клав", json.dumps(planned))
        assert self.engine._last_correction is not None
        self.assertEqual(self.engine._last_correction.mode, "typo")
        self.assertGreater(self.engine._last_correction.confidence, 0)
        self.assertEqual(self.editor.erased, [None])
        self.assertEqual(self.engine._kept, ())
        self.assertIsNone(self.engine._provisional)

        self.engine._schedule_undo(Z_KEYCODE)
        self.engine._handle(plain_key("z", Z_KEYCODE, 1, pressed=False))
        self.assertEqual(self.backend.text, "клавиаутра ")
        # Taken back once, the word is the user's own and stays as typed, in this session and the next.
        self.assertIn("клавиаутра", self.engine._personal)
        self.type("клавиаутра ")
        self.assertEqual(self.backend.text, "клавиаутра клавиаутра ")
        self.assertIn("клавиаутра", PersonalWords(self.engine._personal.path))
        self.assertNotIn("клавиаутра".encode(), self.engine._personal.path.read_bytes())

    def test_pause_right_after_a_correction_puts_the_word_back_and_the_next_press_converts(self) -> None:
        self.type("клавиатуура ")
        self.assertEqual(self.backend.text, "клавиатура ")
        self.assertEqual(self.editor.erased, [len("клавиатуура")])
        for pressed in (True, False):
            self.engine._handle(plain_key("Pause", PAUSE_KEYCODE, 1, pressed=pressed))
        self.assertEqual(self.backend.text, "клавиатуура ")
        self.assertEqual(self.editor.erased[-1], len("клавиатура"))
        for pressed in (True, False):
            self.engine._handle(plain_key("Pause", PAUSE_KEYCODE, 1, pressed=pressed))
        self.assertEqual(self.backend.text, self.pair.translate("клавиатуура", "ru", "us") + " ")

    def test_a_letter_no_key_has_typed_yet_cannot_be_typed(self) -> None:
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type("клавматура ")
        self.assertEqual(self.backend.text, "клавматура ")
        declined = self.events(logs.output, "typo_correction_declined")
        self.assertEqual([event["reason"] for event in declined], ["no_key"])
        self.type("и клавматура ")
        self.assertEqual(self.backend.text, "клавматура и клавиатура ")

    def test_a_left_out_letter_is_typed_and_unclear_words_stay(self) -> None:
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type("нромально клвиатура клавиатурв ")
        self.assertEqual(self.backend.text, "нормально клавиатура клавиатурв ")
        self.assertEqual(self.editor.erased, [None, len("клвиатура")])
        self.assertEqual([event["reason"] for event in self.events(logs.output, "typo_correction_declined")],
                         ["ambiguous"])
        planned = self.events(logs.output, "typo_correction_planned")
        self.assertIsInstance(planned[0]["margin"], float)

    def test_a_capital_is_corrected_only_where_a_sentence_begins(self) -> None:
        self.type("Клавиаутра и Клавиаутра. Клавиаутра ")
        self.assertEqual(self.backend.text, "Клавиатура и Клавиаутра. Клавиатура ")

    def test_the_setting_the_exclusions_and_a_word_typed_into_another_are_respected(self) -> None:
        self.settings.set("detection.correct_typos", False)
        self.type("клавиаутра ")
        self.settings.set("detection.correct_typos", True)
        self.settings.set("exclusions.words", ["Клавиатуура"])
        self.type("клавиатуура ")
        self.settings.set("exclusions.words", [])
        self.engine.learning.add_rule(LearnedRule("клавматура", 1, 0, "keep", "exact", False))
        self.type("и клавматура ")
        for character in "клавиаутра":
            self.tap(character)
            self.engine._insertion = InsertionPoint("а", "")
        self.tap(" ")
        self.assertEqual(self.backend.text, "клавиаутра клавиатуура и клавматура клавиаутра ")

    def test_a_word_converted_and_misspelt_is_corrected_in_one_injection(self) -> None:
        keys = self.pair.translate("клавиаутра", "ru", "us")
        self.engine.learning.add_rule(LearnedRule(keys, 0, 1, "convert", "exact", False))
        self.backend.group = 0
        self.type(keys + " ")
        self.assertEqual(self.backend.text, "клавиатура ")
        self.assertEqual(len(self.backend.injections), 1)
        word, target, _ = self.backend.injections[0]
        self.assertEqual((target, "".join(key.character_for(1) for key in word)), (1, "клавиатура"))
        assert self.engine._last_correction is not None
        self.assertEqual((self.engine._last_correction.source_group, self.engine._last_correction.target_group),
                         (0, 1))
        self.assertIsNone(self.engine._last_correction.revisit)
        # Undo types the keys as they were typed, in their own layout.
        self.engine._schedule_undo(Z_KEYCODE)
        self.engine._handle(plain_key("z", Z_KEYCODE, 1, pressed=False))
        self.assertEqual(self.backend.text, keys + " ")

    def test_a_word_converted_without_a_typo_and_its_undo_go_as_before(self) -> None:
        keys = self.pair.translate("привет", "ru", "us")
        self.backend.group = 0
        self.type(keys + " ")
        self.assertEqual(self.backend.text, "привет ")
        self.engine._schedule_undo(Z_KEYCODE)
        self.engine._handle(plain_key("z", Z_KEYCODE, 1, pressed=False))
        self.assertEqual(self.backend.text, keys + " ")
        self.assertEqual(self.editor.erased, [None, None])

    def test_a_backend_that_erases_what_it_types_gets_corrections_of_the_word_s_length_only(self) -> None:
        backend = EditorBackend()
        backend.group = 1
        self.assertIsNone(erasing_injection(backend))
        self.assertIsNotNone(erasing_injection(self.backend))
        self.backend, self.engine = backend, KeySwitchEngine(self.settings, self.history, backend)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type("клавиатуура клавиаутра ")
        self.assertEqual(backend.text, "клавиатуура клавиатура ")
        self.assertEqual([event["reason"] for event in self.events(logs.output, "typo_correction_declined")],
                         ["backend_erases_typed_length"])
        # A plan that would erase another count than it types fails there, and nothing is typed.
        typed = word_keys("клавиатуура", 1)
        plan = CorrectionPlan(typed[:-1], None, 1, 1, "клавиатуура", "клавиатура", 1.0, "TestEditor", True, "typo",
                              replaced_strokes=typed)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.assertFalse(self.engine._execute_correction(plan, None))
        self.assertEqual(len(self.events(logs.output, "correction_failed")), 1)

    def test_a_word_left_as_typed_twice_is_the_user_s_own_and_going_back_to_it_does_not_count(self) -> None:
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            # No key has typed `и` yet: the word stays as typed and counts once the next word begins.
            self.type("клавматура ")
            self.assertEqual(self.engine._personal_pending, "клавматура")
            erase = KeyEvent(True, Z_KEYCODE, "BackSpace", "", ("", ""), 1, 0, TYPO_FIXTURE_KEY_TIMESTAMP)
            for item in (erase, replace(erase, pressed=False)):
                self.backend.type(item)
                self.engine._handle(item)
            # Backspace went back into it: it is not counted.
            self.assertEqual(self.engine._personal_pending, "")
            self.type(" клавматура и клавматура ")
        self.assertEqual(self.backend.text, "клавматура клавматура и клавматура ")
        self.assertEqual([event["reason"] for event in self.events(logs.output, "personal_word_added")],
                         ["left_as_typed"])
        self.assertIn("клавматура", self.engine._personal)
        self.engine.clear_personal_words()
        self.type("клавматура ")
        self.assertEqual(self.backend.text, "клавматура клавматура и клавматура клавиатура ")

    def test_editing_the_next_word_leaves_the_count_of_the_word_before_it(self) -> None:
        self.type("клавматура кл")
        erase = KeyEvent(True, Z_KEYCODE, "BackSpace", "", ("", ""), 1, 0, TYPO_FIXTURE_KEY_TIMESTAMP)
        for item in (erase, replace(erase, pressed=False)):
            self.backend.type(item)
            self.engine._handle(item)
        # `клавматура` was counted once `кл` began; Backspace and Pause inside `кл` leave that count.
        self.engine._schedule_manual_conversion(PAUSE_KEYCODE)
        self.assertEqual(list(self.engine._personal._counts.values()), [1])

    def test_without_the_history_the_user_s_words_are_not_written(self) -> None:
        self.settings.set("general.keep_history", False)
        self.type("клавиаутра ")
        self.engine._schedule_undo(Z_KEYCODE)
        self.engine._handle(plain_key("z", Z_KEYCODE, 1, pressed=False))
        self.assertIn("клавиаутра", self.engine._personal)
        self.assertFalse(self.engine._personal.path.exists())

    def test_presses_in_an_excluded_application_teach_no_key(self) -> None:
        self.settings.set("exclusions.applications", [self.backend.active_application()])
        self.type("и ")
        self.settings.set("exclusions.applications", [])
        self.assertIsNone(self.engine._typo_keys.key(1, "и", 1))

    def test_english_words_are_corrected_only_with_their_setting_on(self) -> None:
        self.backend.group = 0
        self.type("hlelo ")
        self.assertEqual(self.backend.text, "hlelo ")
        self.settings.set("detection.correct_typos_english", True)
        self.type("hlelo ")
        self.assertEqual(self.backend.text, "hlelo hello ")
        spec = next(item for item in ALL_SETTING_SPECS if item.path == "detection.correct_typos_english")
        self.assertEqual(spec.description, CORRECT_TYPOS_ENGLISH_DESCRIPTION + ".")
        self.assertFalse(SettingsStore(Path(self.temporary.name) / "fresh.json").get("detection.correct_typos_english"))

    def test_both_settings_windows_offer_the_setting(self) -> None:
        spec = next(item for item in ALL_SETTING_SPECS if item.path == "detection.correct_typos")
        self.assertEqual(spec.description, CORRECT_TYPOS_DESCRIPTION + ".")
        self.assertIn(f"{TYPO_MIN_LETTERS} букв", CORRECT_TYPOS_DESCRIPTION)
        self.assertTrue(self.settings.get("detection.correct_typos"))


class BackendErasureTests(unittest.TestCase):
    """Every backend erases as many characters as a typo correction says the word has, not as it types."""

    def test_x11_erases_the_given_count(self) -> None:
        backend, libraries = backend_with()
        backend._control = 1
        strokes = word_keys("ab", 0)
        for erased in (len(strokes) + 1, len(strokes) - 1):
            libraries.xtst.XTestFakeKeyEvent.reset_mock()
            backend.inject_correction(strokes, 0, None, erased=erased)
            presses = [call.args for call in libraries.xtst.XTestFakeKeyEvent.call_args_list
                       if call.args[1] == BACKSPACE_KEYCODE and call.args[XTEST_FAKE_KEY_EVENT_IS_PRESS_ARG_INDEX]]
            self.assertEqual(len(presses), erased)

    def test_windows_erases_the_given_count(self) -> None:
        api = FakeWindowsAPI()
        backend = WindowsBackend(api)
        strokes = (key_event(), key_event())
        backend.inject_correction(strokes, 0, None, erased=len(strokes) + 1)
        self.assertEqual(sum(item.pressed and item.virtual_key == VK_BACK for item in api.sent[0]), len(strokes) + 1)

    def test_macos_erases_the_given_count(self) -> None:
        api = FakeMacAPI()
        backend = MacBackend(api)
        strokes = word_keys("ab", 0)
        backend.inject_correction(strokes, 0, None, erased=len(strokes) - 1)
        self.assertEqual(sum(item.pressed and item.keycode == MAC_VK_BACKSPACE for item in api.posted),
                         len(strokes) - 1)

    def test_wayland_erases_the_given_count(self) -> None:
        api = FakeWaylandAPI()
        backend = WaylandBackend(api)
        strokes = (stroke_event(), stroke_event())
        backend.inject_correction(strokes, 0, None, erased=len(strokes) + 1)
        self.assertEqual(sum(item == (True, EVDEV_KEY_BACKSPACE) for batch in api.posted for item in batch),
                         len(strokes) + 1)


class PackagingTests(unittest.TestCase):
    def test_both_builds_ship_the_typo_lists_as_they_are(self) -> None:
        # Without its lists the correction is off: each build compares the bundled bytes with the project's.
        root = Path(__file__).resolve().parents[1]
        script = (root / "packaging/build-windows.ps1").read_text(encoding="utf-8")
        deb_script = (root / "packaging/build-deb.sh").read_text(encoding="utf-8")
        match = re.search(r"-MaximumBytes (\d+)MB -MinimumBytes \d+ -Label \"bundled typo frequency list", script)
        assert match is not None
        self.assertEqual(int(match.group(1)) * BYTES_PER_MEBIBYTE, TYPO_FREQUENCY_MAX_BYTES)
        for locale in TYPO_ALPHABETS:
            self.assertLessEqual(typo_resource_path(locale).stat().st_size, TYPO_FREQUENCY_MAX_BYTES)
        self.assertIn("Native distribution contains a different typo frequency list", script)
        self.assertIn('"$native_dist/keyswitch/resources/typo-frequency-$typo_locale.bin"; then', deb_script)


class PersonalWordsTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.path = Path(self.temporary.name) / "personal-words.json"

    def test_a_word_left_as_typed_twice_is_the_user_s_and_only_salted_hashes_are_stored(self) -> None:
        words = PersonalWords(self.path)
        self.assertNotIn("комить", words)
        self.assertNotIn(None, words)
        self.assertFalse(self.path.exists())
        self.assertFalse(words.keep("комить"))
        self.assertNotIn("комить", words)
        self.assertTrue(words.keep("Комить"))
        self.assertIn("комить", words)
        self.assertTrue(words.keep("комить"))
        words.add("митап")
        self.assertEqual(len(words), len(("комить", "митап")))
        raw = self.path.read_text(encoding="utf-8")
        self.assertNotIn("комить", raw)
        self.assertNotIn("митап", raw)
        payload = json.loads(raw)
        self.assertEqual(len(payload["salt"]), PERSONAL_WORD_SALT_BYTES + PERSONAL_WORD_SALT_BYTES)
        self.assertTrue(all(len(key) == PERSONAL_WORD_HASH_HEX_CHARACTERS for key in payload["personal"]))
        self.assertEqual(payload["counts"], {})
        again = PersonalWords(self.path)
        self.assertIn("комить", again)
        self.assertIn("МИТАП", again)
        # Another installation's salt makes other hashes of the same words.
        self.assertNotIn("комить", PersonalWords(Path(self.temporary.name) / "other.json"))

    def test_both_lists_are_bounded_and_the_oldest_entry_goes(self) -> None:
        words = PersonalWords(self.path)
        with patch("keyswitch.personal_words.PERSONAL_WORD_COUNTS_MAX", 1), \
                patch("keyswitch.personal_words.PERSONAL_WORDS_MAX", 1):
            words.keep("первое")
            words.keep("второе")
            # The first count went: `первое` starts again from one.
            self.assertFalse(words.keep("первое"))
            words.add("третье")
            words.add("четвёртое")
        self.assertNotIn("третье", words)
        self.assertIn("четвёртое", words)

    def test_nothing_is_written_while_the_history_is_off_and_a_failed_write_is_survived(self) -> None:
        keep_history = [False]
        words = PersonalWords(self.path, persist=lambda: keep_history[0])
        words.add("комить")
        self.assertIn("комить", words)
        self.assertFalse(self.path.exists())
        keep_history[0] = True
        with patch("pathlib.Path.replace", side_effect=OSError("disk full")):
            words.add("митап")
        self.assertIn("митап", words)
        self.assertFalse(self.path.exists())

    def test_an_unreadable_file_is_set_aside_and_bad_entries_are_dropped(self) -> None:
        self.path.write_text("{", encoding="utf-8")
        words = PersonalWords(self.path)
        self.assertIsNotNone(words.unreadable)
        self.assertEqual(len(words), 0)
        salt = "ab"
        key = hashlib.sha256((salt + "комить").encode()).hexdigest()[:PERSONAL_WORD_HASH_HEX_CHARACTERS]
        self.path.write_text(json.dumps({"schema_version": PERSONAL_WORDS_SCHEMA_VERSION, "salt": salt,
                                         "personal": [key, 1], "counts": {key: 1, "x": 0, "y": "1"}}),
                             encoding="utf-8")
        loaded = PersonalWords(self.path)
        self.assertIsNone(loaded.unreadable)
        self.assertIn("комить", loaded)
        self.assertEqual(len(loaded), 1)

    def test_clearing_forgets_every_word_with_a_new_salt(self) -> None:
        words = PersonalWords(self.path)
        words.add("комить")
        salt = json.loads(self.path.read_text(encoding="utf-8"))["salt"]
        words.clear()
        self.assertNotIn("комить", words)
        self.assertEqual(json.loads(self.path.read_text(encoding="utf-8"))["salt"], "")
        words.add("комить")
        self.assertNotEqual(json.loads(self.path.read_text(encoding="utf-8"))["salt"], salt)
