"""The part after a joining sign is decided as a word of its own when the token is not right whole.

An English term and a Russian word joined by a hyphen are common in Russian prose (`mongo-сервиса`,
`docker-образ`), and the layout the term was typed in often types the word after it. Judged whole,
`mongo-cthdbcf` was kept: its whole other reading `ьщтпщ-сервиса` is no word either. The installed
models decide; these tests hold the engine to asking them about the right span.
"""

from __future__ import annotations

import json
import unittest
from unittest.mock import patch

import test_input_sequence_matrix as sequences
from test_input_integrity import InputIntegrityTests
from fixture_values.clock import PAUSE_TRIGGER_OFFSET_SECONDS
from engine_models import share_engine_models


def setUpModule() -> None:
    share_engine_models()


class JoinedPartTests(InputIntegrityTests):
    def setUp(self) -> None:
        super().setUp()
        self.settings.set("detection.context_policy", "assist")
        self.settings.set("detection.context_aware", True)
        self.settings.set("detection.correct_on_pause", True)
        patcher = patch.object(self.backend, "active_application", return_value="Telegram")
        patcher.start()
        self.addCleanup(patcher.stop)

    def events(self, lines: list[str], name: str) -> list[dict[str, object]]:
        events = [json.loads(line.split("TECHNICAL ", 1)[1]) for line in lines if "TECHNICAL " in line]
        return [event for event in events if event["event"] == name]

    def test_the_part_after_a_hyphen_is_converted_and_the_head_stays(self) -> None:
        # Kept whole (`mongo-cthdbcf`, `lid-ht;bvt` in the owner's logs), or converted whole although the head is
        # a word as typed (`docker-j,hfp` became `вщслук-образ`).
        for typed, part, expected in (
            ("mongo-cthdbcf ", "cthdbcf", "mongo-сервиса "),
            ("lid-ht;bvt ", "ht;bvt", "lid-режиме "),
            ("docker-j,hfp ", "j,hfp", "docker-образ "),
            ("redis+rkfcnth ", "rkfcnth", "redis+кластер "),
        ):
            with self.subTest(typed=typed):
                self.reset_editor()
                with self.assertLogs("keyswitch.engine", level="INFO") as logs:
                    self.type(typed, group=0)
                self.assertEqual(self.backend.text, expected)
                self.assertEqual(self.backend.group, 1)
                evaluation = self.events(logs.output, "word_evaluation")[-1]
                self.assertEqual(evaluation["original"], part)
                self.assertEqual(evaluation["literal_head"], typed[: typed.index(part)])
                self.assertIs(self.events(logs.output, "joined_part_decision")[-1]["taken"], True)

    def test_a_sign_after_the_part_is_replayed_with_it(self) -> None:
        # The `/` key is the period of the Russian layout: `mongo-сервиса.` typed in the English one. This
        # editor's keys type one sign in both layouts; the physical session types the real keys.
        with sequences.session(0) as current:
            current.physical("mongo-cthdbcf/ ")
            current.flush()
            self.assertEqual(current.backend.text, "mongo-сервиса. ")

    def test_pause_after_the_correction_undoes_only_the_part(self) -> None:
        self.type("mongo-cthdbcf ", group=0)
        self.assertEqual(self.backend.text, "mongo-сервиса ")
        self.tap(self.key("Pause"))
        self.assertEqual(self.backend.text, "mongo-cthdbcf ")
        self.assertEqual(self.backend.group, 0)

    def test_idle_pause_converts_the_part_after_the_hyphen(self) -> None:
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type("mongo-cthdbcf", group=0)
            last = self.engine._last_word_input_at
            assert last is not None
            self.engine._maybe_correct_after_pause(now=last + PAUSE_TRIGGER_OFFSET_SECONDS)
        self.assertEqual(self.backend.text, "mongo-сервиса")
        evaluation = self.events(logs.output, "word_evaluation")[-1]
        self.assertEqual((evaluation["trigger"], evaluation["literal_head"]), ("pause", "mongo-"))

    def test_a_pause_in_the_middle_of_the_part_is_a_pause_to_think(self) -> None:
        # `sh` starts many English words: the part is still being typed, as a word would be.
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.type("mongo-sh", group=0)
            last = self.engine._last_word_input_at
            assert last is not None
            self.engine._maybe_correct_after_pause(now=last + PAUSE_TRIGGER_OFFSET_SECONDS)
        self.assertEqual(self.backend.text, "mongo-sh")
        self.assertEqual(self.events(logs.output, "joined_part_decision"), [])

    def test_signs_in_a_row_join_as_one(self) -> None:
        self.type("redis--rkfcnth ", group=0)
        self.assertEqual(self.backend.text, "redis--кластер ")

    def test_tokens_right_or_wrong_as_a_whole_are_decided_whole(self) -> None:
        # A token typed in one layout, right as typed or converted whole; a head the model only suggests to
        # check (`BB` of `ИИ-ассистент`) leaves the whole conversion standing.
        for group, typed, expected in (
            (0, "rfrjq-nj ", "какой-то "), (0, "BB-fccbcntyn ", "ИИ-ассистент "), (1, "ыыр-фпуте ", "ssh-agent "),
            (1, "что-то ", "что-то "), (1, "по-разному ", "по-разному "), (0, "e-mail ", "e-mail "),
            (0, "node-telegram ", "node-telegram "), (0, "state-of-the-art ", "state-of-the-art "),
            (0, "10-chat-export ", "10-chat-export "), (0, "foo=bar ", "foo=bar "), (0, "a--b ", "a--b "),
        ):
            with self.subTest(typed=typed, group=group):
                self.reset_editor(group)
                self.type(typed, group=group)
                self.assertEqual(self.backend.text, expected)

    def test_one_letter_after_a_joining_sign_is_part_of_the_term(self) -> None:
        # `usb-c` after a Russian word: neither the curated one-letter words nor the letter after a
        # Latin term in Russian prose (`поправь env b`) apply to a letter joined to the term.
        self.reset_editor(1)
        self.type("поставь ", group=1)
        self.backend.group = 0
        self.type("usb-c plan-b env b ", group=0)
        self.assertEqual(self.backend.text, "поставь usb-c plan-b env и ")

    def test_identifiers_assignments_and_addresses_are_judged_whole(self) -> None:
        # The code guard's tokens are no prose: `user_cthdbcf` is an identifier as typed.
        for typed in ("user_cthdbcf ", "foo=ghbdtn ", "user@ghbdtn "):
            with self.subTest(typed=typed):
                self.reset_editor()
                self.type(typed, group=0)
                self.assertEqual(self.backend.text, typed)

    def test_a_head_that_may_be_a_wrong_layout_word_leaves_the_token_whole(self) -> None:
        # `ьщтпщ` is `mongo` in the other layout: one correction cannot fix two words, and the token
        # stays as the whole of it was judged.
        self.reset_editor(1)
        self.type("ьщтпщ-сервиса ", group=1)
        self.assertEqual(self.backend.text, "ьщтпщ-сервиса ")

    def test_without_the_contextual_model_the_token_is_judged_whole(self) -> None:
        self.settings.set("detection.context_policy", "off")
        self.type("mongo-cthdbcf ", group=0)
        self.assertEqual(self.backend.text, "mongo-cthdbcf ")


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    return unittest.TestSuite(JoinedPartTests(name) for name in JoinedPartTests.__dict__ if name.startswith("test_"))


if __name__ == "__main__":
    unittest.main()
