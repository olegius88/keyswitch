"""A literal ``/`` head keeps its place while the word after it is analysed."""

from __future__ import annotations

import json
import unittest
from unittest.mock import patch

from test_input_integrity import InputIntegrityTests


class LiteralHeadTests(InputIntegrityTests):
    def setUp(self) -> None:
        super().setUp()
        self.settings.set("detection.context_policy", "assist")
        self.settings.set("detection.context_aware", True)
        self.settings.set("detection.correct_on_pause", True)
        patcher = patch.object(self.backend, "active_application", return_value="Telegram")
        patcher.start()
        self.addCleanup(patcher.stop)

    def evaluations(self, lines: list[str]) -> list[dict[str, object]]:
        events = [json.loads(line.split("TECHNICAL ", 1)[1]) for line in lines if "TECHNICAL " in line]
        return [event for event in events if event["event"] == "word_evaluation"]

    def test_word_after_a_slash_is_converted_and_the_slash_stays(self) -> None:
        for typed, expected in (("/c,jhrb ", "/сборки "), ("bild/c,jhrb ", "bild/сборки "), ("v2/c,jhrb ", "v2/сборки "), ("//c,jhrb ", "//сборки ")):
            with self.subTest(typed=typed):
                self.reset_editor()
                with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                    self.type(typed, group=0)
                self.assertEqual(self.backend.text, expected)
                self.assertEqual(self.backend.group, 1)
                evaluation = self.evaluations(logs.output)[-1]
                self.assertEqual(evaluation["original"], "c,jhrb")
                self.assertEqual(evaluation["literal_head"], typed[: typed.index("c")])
                decision = evaluation["decision"]
                assert isinstance(decision, dict)
                self.assertTrue(decision["should_convert"])
                self.assertEqual(decision["replacement"], "сборки")

    def test_pause_after_the_automatic_correction_undoes_only_the_word(self) -> None:
        self.type("bild/c,jhrb ", group=0)
        self.assertEqual(self.backend.text, "bild/сборки ")
        self.tap(self.key("Pause"))
        self.assertEqual(self.backend.text, "bild/c,jhrb ")
        self.assertEqual(self.backend.group, 0)

    def test_idle_pause_trigger_converts_the_word_after_the_head(self) -> None:
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type("/c,jhrb", group=0)
            last = self.engine._last_word_input_at
            assert last is not None
            self.engine._maybe_correct_after_pause(now=last + 2)
        self.assertEqual(self.backend.text, "/сборки")
        evaluation = self.evaluations(logs.output)[-1]
        self.assertEqual((evaluation["trigger"], evaluation["original"], evaluation["literal_head"]), ("pause", "c,jhrb", "/"))

    def test_heads_that_may_be_wrong_layout_words_abstain_and_pause_converts_everything(self) -> None:
        for typed in ("c,jhrb/ntcns ", "b/bkb ", "rhtp/c,jhrb "):
            with self.subTest(typed=typed):
                self.reset_editor()
                with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                    self.type(typed, group=0)
                self.assertEqual(self.backend.text, typed)
                evaluation = self.evaluations(logs.output)[-1]
                self.assertEqual((evaluation["original"], evaluation["literal_head"]), (typed.strip(), ""))
        self.reset_editor()
        self.type("c,jhrb/ntcns ", group=0)
        self.tap(self.key("Pause"))
        self.assertEqual(self.backend.text, "сборки/тесты ")

    def test_paths_commands_and_correct_words_around_slashes_stay(self) -> None:
        for typed, group in (("/usr/local/bin ", 0), ("src/keyswitch/engine ", 0), ("/start ", 0), ("user@ghbdtn ", 0), ("abc/ ", 0), ("/foo=ghbdtn ", 0), ("/c,jhrb2 ", 0), ("да/нет ", 1), ("и/или ", 1)):
            with self.subTest(typed=typed, group=group):
                self.reset_editor(group)
                self.type(typed, group=group)
                self.assertEqual(self.backend.text, typed)
                self.assertEqual(self.backend.group, group)

    def test_head_requires_the_contextual_policy_and_code_protection(self) -> None:
        for setting, value in (("detection.context_policy", "shadow"), ("detection.context_aware", False), ("detection.protect_code", False)):
            with self.subTest(setting=setting):
                self.reset_editor()
                self.settings.set(setting, value)
                try:
                    with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                        self.type("/c,jhrb ", group=0)
                finally:
                    self.settings.set(setting, "assist" if setting == "detection.context_policy" else True)
                evaluation = self.evaluations(logs.output)[-1]
                self.assertEqual((evaluation["original"], evaluation["literal_head"]), ("/c,jhrb", ""))
        self.reset_editor()
        self.engine.context_policy.model = None
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type("/c,jhrb ", group=0)
        self.assertEqual(self.backend.text, "/c,jhrb ")
        self.assertEqual(self.evaluations(logs.output)[-1]["literal_head"], "")

    def test_a_head_does_not_join_a_waiting_short_word(self) -> None:
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type("e /c,jhrb ", group=0)
        events = [json.loads(line.split("TECHNICAL ", 1)[1]) for line in logs.output if "TECHNICAL " in line]
        cancelled = [event for event in events if event["event"] == "context_wait_cancelled"]
        self.assertEqual([event["reason"] for event in cancelled][-1:], ["literal_head"])
        self.assertIsNone(self.engine._context_waiting)
        self.assertEqual(self.backend.text, "e /сборки ")

    def test_observed_slash_tokens_are_never_half_converted(self) -> None:
        """Real slash tokens from field logs: the head stays, the tail is whole."""

        for typed, group in (("js/ts ", 1), ("js/ts ", 0), ("/go ", 1), ("3/ ", 0),
                             ("vt[fybpvs/jgthfwbb ", 1), ("cthdbcf/gh ", 1), ("bild/c,jhrb ", 0)):
            with self.subTest(typed=typed, group=group):
                self.reset_editor(group)
                shown = typed if group == 0 else self.pair.translate(typed, "us", "ru")
                self.type(shown, group=group)
                head, _, tail = shown.rstrip().rpartition("/")
                other = self.pair.translate(tail, "ru" if group else "us", "us" if group else "ru")
                result_head, _, result_tail = self.backend.text.rstrip().rpartition("/")
                self.assertEqual(result_head, head)
                self.assertIn(result_tail, {tail, other})

    def test_excluded_application_never_records_the_literal_head(self) -> None:
        self.settings.set("exclusions.applications", ["Telegram"])
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type("bild/c,jhrb ", group=0)
        self.assertEqual(self.backend.text, "bild/c,jhrb ")
        evaluation = self.evaluations(logs.output)[-1]
        self.assertEqual(evaluation["original"], "<redacted>")
        self.assertEqual(evaluation["literal_head"], "<redacted>")
        self.assertEqual(evaluation["alternatives"], {})


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    return unittest.TestSuite(LiteralHeadTests(name) for name in LiteralHeadTests.__dict__ if name.startswith("test_"))
