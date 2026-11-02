"""The known-reply head (context_action_features.known_reply_question) and its curriculum.

`jr` sent alone in Telegram for `ок` stayed: the English lexicon knows `Jr`, the short-message head did not answer such a
token, and the frozen model only suggested a look at the layout (the laptop, 02.11.2026).
"""

from __future__ import annotations

import sys
import unittest
from dataclasses import replace
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

from keyswitch.backend import KeyEvent  # noqa: E402
from keyswitch.constants.models import KNOWN_REPLY_FEATURE_PREFIX  # noqa: E402
from keyswitch.constants.model_protocol import DEVELOPMENT, TRAIN  # noqa: E402
from keyswitch.context_action_features import brief_dominant, extract_action_features, known_reply_question  # noqa: E402
from keyswitch.context_model import ContextAction, ContextEvidence  # noqa: E402
from keyswitch.input_context import FieldContext  # noqa: E402
import train_context_action_model as trainer  # noqa: E402
from fixture_values.counts import BRIEF_FIXTURE_MINIMUM_COUNT  # noqa: E402
from test_context_policy import ContextEngineTests  # noqa: E402
from fixture_values.keys import DEFAULT_SEQUENCE_RETURN_KEYCODE  # noqa: E402
from test_default_input_sequences import DefaultSequenceHarness, _Session  # noqa: E402
from test_early_switch_settling import ScriptedActionModel  # noqa: E402

BRIEF = {"minimum_count": BRIEF_FIXTURE_MINIMUM_COUNT, "held_out": ["jr"], "sample_weight": 1.0}


def head(original: str, alternative: str, *, known: bool = True, before: str = "", application: str = "Telegram") -> set[str]:
    item = ContextEvidence(original, alternative, 0, FieldContext(application, "1", before, ""), "enter",
                           source_known=known, boundary_text="\n")
    return {name.removeprefix(KNOWN_REPLY_FEATURE_PREFIX) for name in extract_action_features(item)
            if name.startswith(KNOWN_REPLY_FEATURE_PREFIX)}


class KnownReplyHeadTests(unittest.TestCase):
    def test_the_question_is_a_short_message_its_russian_reading_outnumbers_by_far(self) -> None:
        for latin, cyrillic in (("jr", "ок"), ("ytn", "нет"), ("tot", "еще"), ("Jr", "Ок")):
            self.assertTrue(known_reply_question(latin, cyrillic, "", ""), latin)
        # `ha` is counted in English prose, `ns` in Russian technical text, `ok` is no Russian word; a token with text
        # around it is no message of its own.
        for latin, cyrillic, before in (("ha", "рф", ""), ("ns", "ты", ""), ("ok", "щл", ""), ("jr", "ок", "это ")):
            self.assertFalse(known_reply_question(latin, cyrillic, before, ""), latin)
        self.assertFalse(brief_dominant("ns", "ты"))

    def test_the_head_reads_the_lexicon_the_boundary_the_case_and_the_application(self) -> None:
        reply = head("jr", "ок")
        self.assertTrue({"bias", "known:1", "trigger:enter", "case:lower", "app:telegram", "known:1:app:telegram"} <= reply)
        self.assertIn("known:0", head("ytn", "нет", known=False))
        self.assertIn("case:title", head("Jr", "Ок"))
        self.assertIn("app:none", head("jr", "ок", application=""))
        self.assertEqual(head("jr", "ок", before="это "), set())
        # No count: every token of the class is counted far more often in its Russian reading.
        self.assertFalse([name for name in reply if name.startswith(("ratio:", "seen:", "russian:", "english:"))])

    def test_the_trainer_routes_the_class_to_the_head(self) -> None:
        prefixes = frozenset({KNOWN_REPLY_FEATURE_PREFIX})
        self.assertTrue(trainer.head_question("jr", "ок", "", "", "enter", "none", prefixes))
        self.assertFalse(trainer.head_question("jr", "ок", "", "", "enter", "none", frozenset()))
        self.assertFalse(trainer.head_question("ha", "рф", "", "", "enter", "none", prefixes))
        self.assertEqual(trainer.OWN_EPOCH_HEADS[KNOWN_REPLY_FEATURE_PREFIX][1], "brief_curriculum")

    def test_the_curriculum_is_the_short_message_frames_of_the_class(self) -> None:
        for split in (TRAIN, DEVELOPMENT):
            rows, report = trainer.known_reply_curriculum(split, [], frozenset(), BRIEF)
            brief = trainer.brief_curriculum(split, [], frozenset(), BRIEF)[0]
            self.assertTrue(rows)
            self.assertEqual(report["frames"], len(rows))
            self.assertTrue({row.identifier for row in rows} < {row.identifier for row in brief})
            self.assertTrue(all(known_reply_question(row.original, trainer.translated(row.original, 0), "", "") for row in rows))
            self.assertNotIn("jr", {row.original.casefold() for row in rows})


class EnterSettlesTheWaitingWordTests(ContextEngineTests):
    """`jr`, a space and Enter: the word waits at its space for a next word, and Enter, which ends the message, asks the
    model about it once more with the Enter trigger before the message goes."""

    def use(self, at_enter: ContextAction, at_space: ContextAction = "wait") -> ScriptedActionModel:
        model = ScriptedActionModel(lambda item: at_enter if item.trigger == "enter" else at_space)
        self.engine.context_policy.model = model
        return model

    def send_message(self, text: str) -> list[str]:
        """Types `text` and sends it with Enter held as the Windows hook holds it; returns the messages sent."""

        self.type(text, group=0)
        action = replace(self.key("Return"), deferred=True)

        def complete(deliver: bool) -> int:
            if deliver:
                self.backend.type(replace(action, deferred=False))
            return 0

        with patch.object(self.backend, "complete_action", side_effect=complete):
            self.engine._handle(action)
            self.engine._handle(replace(action, pressed=False))
        return self.backend.submissions

    def test_a_word_waiting_at_its_space_is_asked_again_at_enter(self) -> None:
        self.use("convert")
        self.assertEqual(self.send_message("jr "), ["ок "])

    def test_a_word_the_model_keeps_at_enter_is_sent_as_typed(self) -> None:
        self.use("keep")
        self.assertEqual(self.send_message("jr "), ["jr "])

    def test_a_suggestion_at_the_space_is_asked_again_at_enter_too(self) -> None:
        self.use("convert", at_space="suggest")
        self.assertEqual(self.send_message("jr "), ["ок "])

    def test_a_word_converted_at_its_space_is_not_converted_again(self) -> None:
        self.use("convert", at_space="convert")
        self.assertEqual(self.send_message("lf "), ["да "])

    def test_enter_in_another_application_leaves_the_word(self) -> None:
        self.use("convert")
        self.type("jr ", group=0)
        with patch.object(self.backend, "active_application", return_value="Code"):
            self.assertEqual(self.send_message(""), ["jr "])


class PackagedPairShortReplyTests(DefaultSequenceHarness):
    """The packaged pair in Telegram with the default settings: `jr` sent alone, with or without a space before Enter, is
    sent as `ок`; `да` converted at its space is not converted again; English replies typed as meant stay."""

    def test_a_short_reply_sent_with_enter_is_corrected_with_or_without_a_space(self) -> None:
        for text, group in (("ок", 1), ("ок ", 1), ("да ", 1), ("ok", 0), ("ok ", 0), ("ha ", 0)):
            with self.subTest(text=text), self.session(group=0) as session:
                session.type(text, group, idle_after_words=False)
                self.assertEqual(self.send_with_held_enter(session), [text])
        # Enter in another application than the word's is no end of that word's message.
        with self.session(group=0) as session:
            session.type("ок ", 1, idle_after_words=False)
            with patch.object(session.backend, "active_application", return_value="Code"):
                self.assertEqual(self.send_with_held_enter(session), ["jr "])

    def send_with_held_enter(self, session: _Session) -> list[str]:
        """Enter as the Windows hook passes it: held until the engine is done with the word before it. Returns the
        messages sent."""

        action = KeyEvent(True, DEFAULT_SEQUENCE_RETURN_KEYCODE, "Return", "", ("", ""), session.backend.group, 0, 0,
                          deferred=True)

        def complete(deliver: bool) -> int:
            if deliver:
                session.backend.type(replace(action, deferred=False))
            return 0

        with patch.object(session.backend, "complete_action", side_effect=complete):
            session.engine._handle(action)
            session.engine._handle(replace(action, pressed=False))
            session.idle()
        return session.backend.submissions


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    suite = unittest.TestSuite()
    for case in (KnownReplyHeadTests, EnterSettlesTheWaitingWordTests, PackagedPairShortReplyTests):
        for name in case.__dict__:
            if name.startswith("test_"):
                suite.addTest(case(name))
    return suite


if __name__ == "__main__":
    unittest.main()
