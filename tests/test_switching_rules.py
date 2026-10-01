"""Switching rules: the store, its conversion of old learning files and the rule window's form."""

from __future__ import annotations

import json
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path
from unittest.mock import patch

from keyswitch.constants.detection import MAX_WORD_STROKES
from keyswitch.constants.file_formats import LEARNING_STORE_BACKUP_SUFFIX, LEARNING_STORE_SCHEMA_VERSION
from keyswitch.config import SettingsStore
from keyswitch.engine import CorrectionPlan, KeySwitchEngine, LearningPrompt
from keyswitch.history import HistoryStore
from keyswitch.learning import InvalidRule, LearnedRule, LearningStore, migrate_legacy, validate_rule
from keyswitch.rule_editor import (
    PROMPT_EMPTY,
    RuleDraft,
    action_text,
    build_rule,
    condition_text,
    draft_from_prompt,
    draft_from_rule,
    draft_problem,
    layout_hint,
    other_layout_text,
    pattern_group,
    prompt_word_line,
)
from test_engine_behaviour import FakeBackend
from fixture_values.keys import PAUSE_KEYCODE
from fixture_values.counts import (
    LEGACY_LEARNING_STORE_SCHEMA_VERSION,
    LEGACY_RULE_CONFIRMATIONS_BELOW,
    LEGACY_RULE_CONFIRMATIONS_REACHED,
)


class LearnedRuleTests(unittest.TestCase):
    def test_each_condition_matches_as_the_rule_window_says(self) -> None:
        exact = LearnedRule("tot", 0, 1, "keep")
        self.assertTrue(exact.matches("TOT"))
        self.assertFalse(exact.matches("tota"))
        prefix = replace(exact, match="prefix")
        self.assertTrue(prefix.matches("Totally"))
        self.assertFalse(prefix.matches("atot"))
        part = replace(exact, match="contains")
        self.assertTrue(part.matches("atotb"))
        self.assertFalse(part.matches("to"))
        cased = replace(exact, case_sensitive=True)
        self.assertTrue(cased.matches("tot"))
        self.assertFalse(cased.matches("Tot"))

    def test_a_word_still_being_typed_may_meet_a_rule(self) -> None:
        exact = LearnedRule("ghbdtn", 0, 1, "keep")
        self.assertTrue(exact.may_match_continuation("GHB"))
        self.assertFalse(exact.may_match_continuation("ghbx"))
        prefix = replace(exact, match="prefix")
        self.assertTrue(prefix.may_match_continuation("ghb"))
        self.assertTrue(prefix.may_match_continuation("ghbdtnbr"))
        self.assertFalse(prefix.may_match_continuation("hb"))
        part = replace(exact, match="contains", pattern="dtn", case_sensitive=True)
        self.assertTrue(part.may_match_continuation("ghbdtn"))
        self.assertTrue(part.may_match_continuation("ghbd"))
        self.assertFalse(part.may_match_continuation("ghb"))
        self.assertFalse(part.may_match_continuation("ghbD"))

    def test_a_rule_that_cannot_work_is_refused(self) -> None:
        good = LearnedRule("tot", 0, 1)
        validate_rule(good)
        for broken in (
            replace(good, pattern=""),
            replace(good, pattern=" tot"),
            replace(good, pattern="t ot"),
            replace(good, pattern="123"),
            replace(good, pattern="t" * (MAX_WORD_STROKES + 1)),
            replace(good, target_group=0),
            replace(good, source_group=-1),
            replace(good, action="maybe"),  # type: ignore[arg-type]
            replace(good, match="regex"),  # type: ignore[arg-type]
        ):
            with self.subTest(rule=broken), self.assertRaises(InvalidRule):
                validate_rule(broken)


class LearningStoreTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.path = Path(self.temporary.name) / "learning.json"

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def test_rules_are_kept_replaced_and_forgotten(self) -> None:
        store = LearningStore(self.path)
        self.assertEqual((store.rules(), store.count()), ((), 0))
        convert = store.add_rule(LearnedRule("ghbdtn", 0, 1))
        keep = store.add_rule(LearnedRule("tot", 0, 1, "keep"))
        # The same letters and condition again replace the rule, whatever the case.
        store.add_rule(LearnedRule("TOT", 0, 1, "convert"))
        self.assertEqual(store.count(), len({"ghbdtn", "tot"}))
        reread = LearningStore(self.path)
        self.assertEqual(reread.rules(), (convert, LearnedRule("TOT", 0, 1, "convert")))
        payload = json.loads(self.path.read_text(encoding="utf-8"))
        self.assertEqual(payload["schema_version"], LEARNING_STORE_SCHEMA_VERSION)
        edited = store.replace_rule(convert, replace(convert, match="prefix"))
        self.assertEqual(store.rules()[0], edited)
        self.assertFalse(store.remove_rule(keep))
        self.assertTrue(store.remove_rule(edited))
        with self.assertRaises(InvalidRule):
            store.add_rule(LearnedRule("", 0, 1))
        with self.assertRaises(InvalidRule):
            store.replace_rule(edited, LearnedRule("x y", 0, 1))
        store.clear()
        self.assertEqual(LearningStore(self.path).count(), 0)

    def test_the_most_specific_rule_decides(self) -> None:
        store = LearningStore(self.path)
        self.assertIsNone(store.match(0, ""))
        store.add_rule(LearnedRule("tot", 0, 1, "convert", "contains"))
        self.assertEqual(store.forced_target(0, "atotb"), 1)
        self.assertFalse(store.keeps(0, "atotb"))
        store.add_rule(LearnedRule("to", 0, 1, "keep", "prefix"))
        # A beginning outranks a part, a whole word outranks a beginning.
        self.assertTrue(store.keeps(0, "totem"))
        self.assertIsNone(store.forced_target(0, "totem"))
        store.add_rule(LearnedRule("totem", 0, 1, "convert"))
        self.assertEqual(store.forced_target(0, "totem"), 1)
        # Of two equal conditions the longer letters decide, and on a tie "keep" does.
        store.add_rule(LearnedRule("tote", 0, 1, "convert", "prefix"))
        self.assertEqual(store.forced_target(0, "totes"), 1)
        store.add_rule(LearnedRule("tote", 0, 1, "keep", "prefix", case_sensitive=True))
        self.assertTrue(store.keeps(0, "totes"))
        # Another layout's typing is another word.
        self.assertIsNone(store.match(1, "totem"))
        self.assertTrue(store.keeps_continuation(0, "t"))
        self.assertFalse(store.keeps_continuation(1, "t"))
        self.assertFalse(store.keeps_continuation(0, "x"))

    def test_an_unreadable_or_foreign_file_is_an_empty_store(self) -> None:
        for payload in ("{broken", "[]", '{"rules": "no"}', json.dumps({"rules": [
            {"pattern": "ok", "source_group": 0, "target_group": 1, "action": "keep", "match": "exact"},
            {"pattern": "", "source_group": 0, "target_group": 1, "action": "keep", "match": "exact",
             "case_sensitive": False},
            {"pattern": "bad", "source_group": "0", "target_group": 1, "action": "keep", "match": "exact"},
            "not a rule",
        ]})):
            with self.subTest(payload=payload):
                self.path.write_text(payload, encoding="utf-8")
                store = LearningStore(self.path)
                self.assertLessEqual(store.count(), 1)
        self.assertEqual(LearningStore(self.path).rules(), (LearnedRule("ok", 0, 1, "keep"),))
        with patch.object(Path, "read_bytes", side_effect=OSError("denied")):
            self.assertEqual(LearningStore(self.path).count(), 0)

    def test_an_old_learning_file_becomes_rules_and_is_kept_aside(self) -> None:
        legacy = {
            "schema_version": LEGACY_LEARNING_STORE_SCHEMA_VERSION,
            "rules": {
                "0:ghbdtn": {"target_group": 1, "confirmations": LEGACY_RULE_CONFIRMATIONS_REACHED},
                "0:half": {"target_group": 1, "confirmations": LEGACY_RULE_CONFIRMATIONS_BELOW},
                "1:руддщ": {"target_group": 0, "confirmations": LEGACY_RULE_CONFIRMATIONS_REACHED},
                "0:both": {"target_group": 1, "confirmations": LEGACY_RULE_CONFIRMATIONS_REACHED},
                "x:broken": {"target_group": 1, "confirmations": LEGACY_RULE_CONFIRMATIONS_REACHED},
                "nocolon": {"target_group": 1, "confirmations": LEGACY_RULE_CONFIRMATIONS_REACHED},
                "0:scalar": "nonsense",
                "0:text": {"target_group": "1", "confirmations": LEGACY_RULE_CONFIRMATIONS_REACHED},
                "0:123": {"target_group": 1, "confirmations": LEGACY_RULE_CONFIRMATIONS_REACHED},
            },
            "rejections": {
                "0:tot": [1, "nonsense"],
                "0:both": [1],
                "0:list": "nonsense",
                "x:broken": [1],
            },
        }
        raw = json.dumps(legacy, ensure_ascii=False)
        self.path.write_text(raw, encoding="utf-8")
        store = LearningStore(self.path)
        self.assertEqual(store.migrated, store.count())
        self.assertEqual(
            {(rule.pattern, rule.source_group, rule.action) for rule in store.rules()},
            {("ghbdtn", 0, "convert"), ("руддщ", 1, "convert"), ("both", 0, "keep"), ("tot", 0, "keep")},
        )
        backup = self.path.with_name(self.path.name + LEARNING_STORE_BACKUP_SUFFIX)
        self.assertEqual(backup.read_text(encoding="utf-8"), raw)
        # Converted once: the next start reads the new schema and leaves the copy alone.
        self.assertEqual(LearningStore(self.path).migrated, 0)
        # A stricter old threshold carried fewer counters over; an existing copy is not overwritten.
        self.path.write_text(raw, encoding="utf-8")
        strict = LearningStore(self.path, legacy_confirmations=LEGACY_RULE_CONFIRMATIONS_REACHED + 1)
        self.assertEqual({rule.pattern for rule in strict.rules()}, {"both", "tot"})
        self.assertEqual(backup.read_text(encoding="utf-8"), raw)
        # A copy that cannot be written leaves the old file as it was.
        self.path.write_text(raw, encoding="utf-8")
        backup.unlink()
        with patch.object(Path, "write_bytes", side_effect=OSError("read-only")):
            LearningStore(self.path)
        self.assertEqual(self.path.read_text(encoding="utf-8"), raw)
        self.assertEqual(migrate_legacy({"rules": [], "rejections": None}, 1), [])
        # Keys that are not text never came from a file; two spellings of one word keep the first.
        self.assertEqual(
            migrate_legacy({"rules": {LEGACY_RULE_CONFIRMATIONS_REACHED: {"target_group": 1}}}, 1), []
        )
        twins = migrate_legacy({"rules": {
            "0:Ghbdtn": {"target_group": 1, "confirmations": LEGACY_RULE_CONFIRMATIONS_REACHED},
            "0:ghbdtn": {"target_group": 1, "confirmations": LEGACY_RULE_CONFIRMATIONS_REACHED},
        }}, LEGACY_RULE_CONFIRMATIONS_REACHED)
        self.assertEqual(twins, [LearnedRule("Ghbdtn", 0, 1)])

    def test_the_engine_reports_rules_carried_over_from_an_old_file(self) -> None:
        self.path.write_text(json.dumps({"rules": {
            "0:ghbdtn": {"target_group": 1, "confirmations": LEGACY_RULE_CONFIRMATIONS_REACHED},
        }}), encoding="utf-8")
        settings = SettingsStore(self.path.with_name("config.json"))
        settings.set("diagnostics.technical_logging", True)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            KeySwitchEngine(
                settings, HistoryStore(self.path.with_name("history.jsonl")), FakeBackend(),
                learning=LearningStore(self.path),
            )
        self.assertTrue(any('"event":"learning_rules_migrated"' in line and '"rules":1' in line for line in logs.output))


class DoublePressOfferTests(unittest.TestCase):
    def test_an_offer_shows_at_once_when_the_fix_cannot_be_put_back(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            engine = KeySwitchEngine(
                SettingsStore(root / "config.json"), HistoryStore(root / "history.jsonl"), FakeBackend(),
            )
            origin = CorrectionPlan((), None, 0, 1, "ghbdtn", "привет", 1.0, "Editor", True, "boundary")
            first = CorrectionPlan((), None, 0, 1, "ghbdtn", "привет", 1.0, "Editor", False, "manual")
            engine._correction_origin = origin
            engine._last_requested_plan = first
            engine._convert_outcome = first
            # The first press left the word converted, the user wants it as typed, and the
            # second conversion cannot be scheduled: the offer is shown without it.
            with patch.object(engine, "_schedule_manual_conversion"):
                engine._request_rule_after_double_press(PAUSE_KEYCODE)
            prompt = engine.learning_prompt
            assert prompt is not None
            self.assertEqual((prompt.original, prompt.action), ("ghbdtn", "keep"))


class RuleEditorTests(unittest.TestCase):
    def test_the_offer_fills_the_form(self) -> None:
        keep = LearningPrompt(0, 1, "tot", "еще", "Telegram", "keep")
        self.assertEqual(draft_from_prompt(keep), RuleDraft("tot", action="keep", application="Telegram"))
        self.assertEqual(prompt_word_line(keep), "«tot»: не переводить")
        convert = replace(keep, original="ghbdtn", replacement="привет", action="convert")
        self.assertEqual(prompt_word_line(convert), "«ghbdtn» → «привет»: переводить")
        self.assertEqual(prompt_word_line(replace(keep, original="")), PROMPT_EMPTY)
        rule = LearnedRule("tot", 0, 1, "keep", "prefix", True)
        self.assertEqual(draft_from_rule(rule), RuleDraft("tot", "prefix", True, "keep", editing=rule))

    def test_the_letters_say_which_layout_they_were_typed_in(self) -> None:
        self.assertEqual(pattern_group(",fpf"), 0)
        self.assertEqual(pattern_group("Ёлка"), 1)
        self.assertIsNone(pattern_group("12"))
        self.assertIsNone(pattern_group("totеще"))
        self.assertIsNone(pattern_group("café"))
        self.assertEqual(other_layout_text("ghbdtn", 0), "привет")
        self.assertEqual(other_layout_text("руддщ", 1), "hello")
        self.assertEqual(layout_hint(RuleDraft(" tot ")), "Набрано в раскладке EN; в RU это «еще»")
        self.assertEqual(layout_hint(RuleDraft("12")), "")

    def test_the_form_becomes_a_rule_or_says_what_to_fix(self) -> None:
        self.assertEqual(
            build_rule(RuleDraft(" еще ", "contains", True, "keep")),
            LearnedRule("еще", 1, 0, "keep", "contains", True),
        )
        self.assertEqual(draft_problem(RuleDraft("tot")), "")
        self.assertEqual(draft_problem(RuleDraft("12")), "В сочетании нужна хотя бы одна буква")
        self.assertIn("одной раскладки", draft_problem(RuleDraft("totеще")))
        self.assertIn("пробелов", draft_problem(RuleDraft("to t")))

    def test_the_settings_list_names_condition_and_action(self) -> None:
        rule = LearnedRule("tot", 0, 1, "keep", "prefix", True)
        self.assertEqual(condition_text(rule), "начинается с, с учётом регистра")
        self.assertEqual(action_text(rule), "не переводить")
        self.assertEqual(condition_text(replace(rule, case_sensitive=False)), "начинается с")
        self.assertEqual(action_text(replace(rule, action="convert")), "переводить в RU")


if __name__ == "__main__":
    unittest.main()
