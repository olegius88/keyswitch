"""The double-capitals fix: `ПРивет` and `HEllo`, typed with Shift held one letter too long.

A word kept in its layout gets its second letter lowercase at its boundary when the lexicon of
its layout knows the word so written; no other shape of word is touched. The sessions type
physical keys with the context policy off unless a test turns it on, so that no model decision
moves the text.
"""

from __future__ import annotations

import json
import unittest
from collections.abc import Iterator, Mapping
from contextlib import contextmanager
from dataclasses import replace
from unittest.mock import Mock, patch

from keyswitch.constants.detection import DOUBLE_CAPITALS_MIN_LETTERS
from keyswitch.constants.keyboard import LOCK_MASK, SHIFT_MASK
from keyswitch.engine import CorrectionPlan, InsertionPoint
from keyswitch.learning import LearnedRule
from fixture_values.clock import PAUSE_TRIGGER_OFFSET_SECONDS
from fixture_values.keys import UNDO_TRIGGER_SENTINEL_KEYCODE
from engine_models import share_engine_models
from test_context_wait_pairs import ScriptedModel
from test_engine_behaviour import plain_key
import test_input_sequence_matrix as sequences


def setUpModule() -> None:
    share_engine_models()


@contextmanager
def session(group: int, settings: Mapping[str, object] | None = None) -> Iterator[sequences.PhysicalSession]:
    with sequences.session(group) as current:
        current.settings.set("detection.context_policy", "off")
        current.settings.set("diagnostics.technical_logging", True)
        for path, value in (settings or {}).items():
            current.settings.set(path, value)
        yield current


def technical_events(lines: list[str]) -> list[dict[str, object]]:
    marker = "TECHNICAL "
    return [json.loads(line.split(marker, 1)[1]) for line in lines if marker in line]


def typed(group: int, keys: str, settings: Mapping[str, object] | None = None) -> str:
    with session(group, settings) as current:
        current.physical(keys)
        current.flush()
        return current.backend.text


def undo(current: sequences.PhysicalSession) -> None:
    current.engine._schedule_undo(UNDO_TRIGGER_SENTINEL_KEYCODE)
    current.send(plain_key("z", UNDO_TRIGGER_SENTINEL_KEYCODE, current.backend.group, False))
    current.flush()


class DoubleCapitalsTests(unittest.TestCase):
    def test_a_word_typed_with_two_capitals_gets_its_second_letter_lowercase(self) -> None:
        cases = (
            (1, "GHbdtn ", "Привет "),
            (0, "HEllo ", "Hello "),
            (1, "PLhfdcndeqnt ", "Здравствуйте "),
            # The second letter's key prints a sign in the other layout: the key is typed again without Shift.
            (1, "J<sxyj ", "Обычно "),
            # The sign that ends the word is typed again as it was.
            (1, "GHbdtn? ", "Привет, "),
        )
        for group, keys, expected in cases:
            with self.subTest(keys=keys):
                self.assertEqual(typed(group, keys), expected)

    def test_the_setting_turns_the_fix_off(self) -> None:
        self.assertEqual(typed(1, "GHbdtn ", {"detection.fix_double_capitals": False}), "ПРивет ")

    def test_other_shapes_are_never_touched(self) -> None:
        cases = (
            # Abbreviations, words in capitals and words too short to tell a slip from an abbreviation.
            (1, "GR ", "ПК "),
            (0, "IT ", "IT "),
            (0, "HELLO ", "HELLO "),
            (0, "IDs ", "IDs "),
            (1, "GHBdtn ", "ПРИвет "),
            # CamelCase, a capital later in the word, and tokens with digits or joining signs.
            (0, "HelloWorld ", "HelloWorld "),
            (0, "HEllO ", "HEllO "),
            (0, "HEllo2 ", "HEllo2 "),
            (0, "HE-llo ", "HE-llo "),
        )
        for group, keys, expected in cases:
            with self.subTest(keys=keys):
                self.assertEqual(typed(group, keys), expected)
        self.assertEqual(len("Hell"), DOUBLE_CAPITALS_MIN_LETTERS)
        self.assertEqual(typed(0, "HEll "), "Hell ")

    def test_terms_of_this_shape_are_no_words_of_the_lexicon(self) -> None:
        for term in ("VMware", "OAuth", "JUnit", "MBps", "IPsec", "ETag", "XPath", "IList"):
            with self.subTest(term=term):
                with session(0) as current, self.assertLogs("keyswitch.engine", level="INFO") as logs:
                    current.physical(term + " ")
                    current.flush()
                self.assertEqual(current.backend.text, term + " ")
                kept = [event for event in technical_events(logs.output) if event["event"] == "double_capitals_kept"]
                self.assertEqual([event["reason"] for event in kept], ["fixed_word_unknown"])

    def test_exclusions_and_rules_of_the_user_stand(self) -> None:
        self.assertEqual(typed(0, "HEllo ", {"exclusions.words": ["hello"]}), "HEllo ")
        with session(1, {"detection.learning": True}) as current, \
                self.assertLogs("keyswitch.engine", level="INFO") as logs:
            current.engine.learning.add_rule(LearnedRule("привет", 1, 0, "keep"))
            current.physical("GHbdtn ")
            current.flush()
        self.assertEqual(current.backend.text, "ПРивет ")
        self.assertIn("user_rule", [event.get("reason") for event in technical_events(logs.output)])
        with session(0, {"detection.learning": True}) as current:
            current.engine.learning.add_rule(LearnedRule("hello", 0, 1, "convert"))
            self.assertIsNotNone(current.engine._forced_target_group(0, "HEllo"))
            plan = current.engine._fix_double_capitals(self.committed(current, "HEllo"), current.key(" "))
        self.assertEqual(plan.original, "HEllo")
        self.assertIsNone(current.engine._pending)

    def test_a_word_a_case_aware_dictionary_knows_as_typed_stays(self) -> None:
        # Hunspell knows a few words so written (`HBase`, `XEmacs`). The word is asked directly: the
        # language model caches its scores, and a score taken under a patched dictionary would outlive the test.
        with session(0) as current:
            committed = self.committed(current, "HEllo")
            speller = current.engine.models[0].speller
            with patch.object(speller, "check", side_effect=lambda word: word == "HEllo"), \
                    self.assertLogs("keyswitch.engine", level="INFO") as logs:
                self.assertIs(current.engine._fix_double_capitals(committed, current.key(" ")), committed)
        self.assertIn("known_as_typed", [event.get("reason") for event in technical_events(logs.output)])
        self.assertIsNone(current.engine._pending)

    def test_a_word_typed_under_caps_lock_stays(self) -> None:
        with session(1) as current, self.assertLogs("keyswitch.engine", level="INFO") as logs:
            for key in "GHbdtn ":
                event = current.key(key)
                current.tap(replace(event, state=event.state | LOCK_MASK))
            current.flush()
        self.assertEqual(current.backend.text, "ПРивет ")
        self.assertIn("caps_lock", [event.get("reason") for event in technical_events(logs.output)])

    def test_the_fix_is_logged_applied_counted_and_announced(self) -> None:
        with session(1, {"general.keep_history": True}) as current, \
                self.assertLogs("keyswitch.engine", level="INFO") as logs:
            seen: list[CorrectionPlan] = []
            current.engine.subscribe_corrections(seen.append)
            current.physical("GHbdtn ")
            current.flush()
            history = current.engine.history.read()
        events = technical_events(logs.output)
        scheduled = next(event for event in events if event["event"] == "double_capitals_scheduled")
        self.assertEqual((scheduled["source_group"], scheduled["letters"], scheduled["boundary"]), (1, len("ПРивет"), "space"))
        applied = next(event for event in events if event["event"] == "correction_applied")
        self.assertEqual((applied["mode"], applied["layout_switched"], applied["automatic"]), ("case", False, True))
        self.assertEqual(current.engine.snapshot.correction_count, 1)
        self.assertEqual(current.engine.snapshot.last_action, "ПРивет → Привет")
        self.assertEqual(current.backend.group, 1)
        self.assertEqual([(entry.original, entry.replacement) for entry in history], [("ПРивет", "Привет")])
        self.assertEqual(len(seen), 1)
        self.assertTrue(seen[0].case_fix)
        self.assertEqual((seen[0].source_group, seen[0].target_group, seen[0].mode), (1, 1, "case"))
        # The replayed key no longer carries Shift, and reads lowercase in both layouts.
        second = seen[0].strokes[1]
        self.assertFalse(second.state & SHIFT_MASK)
        self.assertEqual((second.character, second.characters), ("р", ("h", "р")))

    def test_undo_brings_back_the_word_as_typed_and_a_second_undo_the_fix(self) -> None:
        with session(1) as current:
            current.physical("GHbdtn ")
            current.flush()
            self.assertEqual(current.backend.text, "Привет ")
            undo(current)
            self.assertEqual(current.backend.text, "ПРивет ")
            assert current.engine._last_correction is not None
            self.assertTrue(current.engine._last_correction.case_fix)
            undo(current)
            self.assertEqual(current.backend.text, "Привет ")
            self.assertEqual(current.backend.caret, len("Привет "))

    def test_pause_after_the_fix_converts_the_word_as_it_stands(self) -> None:
        with session(1) as current:
            current.physical("GHbdtn ")
            current.flush()
            current.command("Pause")
            current.flush()
            self.assertEqual(current.backend.text, "Ghbdtn ")
            self.assertEqual(current.backend.group, 0)

    def test_a_converted_word_keeps_the_case_it_was_typed_with(self) -> None:
        # The code guard off, `GHbdtn` typed in the English layout converts; a conversion types the keys
        # as they were typed, so its case stays and undo and Pause find the keys they expect.
        self.assertEqual(typed(0, "GHbdtn ", {"detection.protect_code": False}), "ПРивет ")
        with session(0, {"detection.protect_code": False}) as current, \
                self.assertLogs("keyswitch.engine", level="INFO") as logs:
            current.physical("GHbdtn")
            last_input = current.engine._last_word_input_at
            assert last_input is not None
            current.engine._maybe_correct_after_pause(now=last_input + PAUSE_TRIGGER_OFFSET_SECONDS)
            current.flush()
            self.assertEqual(current.backend.text, "ПРивет")
            current.physical(" ")
            current.flush()
        self.assertEqual(current.backend.text, "ПРивет ")
        self.assertIn("converted_while_typed", [event.get("reason") for event in technical_events(logs.output)])

    def test_enter_is_delivered_once_after_the_fix(self) -> None:
        with session(1) as current:
            current.backend.complete_action = Mock(return_value=0)  # type: ignore[method-assign]
            current.physical("GHbdtn")
            enter = replace(current.key("", name="Return"), deferred=True)
            # The hook holds a deferred Enter back: the editor does not see it until the fix is typed.
            current.engine._handle(enter)
            current.engine._handle(replace(enter, pressed=False))
            current.flush()
        self.assertEqual(current.backend.text, "Привет")
        current.backend.complete_action.assert_called_once_with(True)

    def test_letters_typed_before_the_space_came_up_are_typed_again_after_the_fix(self) -> None:
        with session(1) as current:
            current.physical("GHbdtn")
            space = current.key(" ")
            current.send(space)
            current.tap(current.key("r"))
            current.send(replace(space, pressed=False))
            current.flush()
            self.assertEqual(current.backend.text, "Привет к")

    def test_a_word_waiting_for_its_neighbour_is_left_as_typed(self) -> None:
        # The model may still convert it with the next word, and a conversion types the keys as they were typed.
        with session(1, {"detection.context_policy": "assist"}) as current, \
                self.assertLogs("keyswitch.engine", level="INFO") as logs:
            model = ScriptedModel(lambda item: "wait" if item.original == "ПРивет" else "keep")
            current.engine.context_policy.model = model
            current.physical("GHbdtn ")
            current.flush()
            self.assertEqual(current.backend.text, "ПРивет ")
            current.physical("rfr ")
            current.flush()
        self.assertEqual(current.backend.text, "ПРивет как ")
        self.assertIn(("ПРивет", "", ""), model.questions)
        self.assertNotIn("double_capitals_scheduled", [event["event"] for event in technical_events(logs.output)])

    def test_a_word_a_conversion_or_a_wait_is_still_deciding_is_left_alone(self) -> None:
        with session(1) as current:
            committed = self.committed(current, "ПРивет")
            boundary = current.key(" ")
            # Letters typed into a word: the word is longer than the keys the engine holds.
            current.engine._insertion = InsertionPoint("", "ик")
            self.assertIs(current.engine._fix_double_capitals(committed, boundary), committed)
            current.engine._insertion = None
            # A layout the engine has no key table for.
            current.settings.set("detection.layouts", ["us", "de"])
            self.assertIs(current.engine._fix_double_capitals(committed, boundary), committed)
            current.settings.set("detection.layouts", ["us", "ru"])
            # A capital that came without Shift (another program typed it).
            unshifted = replace(committed.strokes[1], state=0)
            self.assertIsNone(current.engine._unshifted((unshifted,), 1))
            # A group without a lexicon.
            elsewhere = replace(committed, source_group=len(current.engine.models))
            self.assertIs(current.engine._fix_double_capitals(elsewhere, boundary), elsewhere)
            self.assertIsNone(current.engine._pending)

    @staticmethod
    def committed(current: sequences.PhysicalSession, word: str) -> CorrectionPlan:
        """The plan _commit_word makes for `word` typed in the session's layout and kept there."""

        group = current.backend.group
        keys = current.pair.translate(word, "ru", "us") if group == 1 else word
        strokes = tuple(current.key(key) for key in keys)
        text = "".join(stroke.character_for(group) for stroke in strokes)
        other = "".join(stroke.character_for(1 - group) for stroke in strokes)
        return CorrectionPlan(strokes, current.key(" "), group, 1 - group, text, other, 0.0, "TestEditor", False)


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    return loader.loadTestsFromTestCase(DoubleCapitalsTests)


if __name__ == "__main__":
    unittest.main()
