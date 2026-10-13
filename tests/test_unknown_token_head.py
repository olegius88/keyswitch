"""The unknown-token head (context_action_features.unknown_question) and its curriculum.

The owner's typing in a chat input of VS Code (13.10.2026): after Russian prose the frozen model kept `цуиоы` (`webjs`)
four times, twice with `webjs` typed right earlier in the same field; `мгуоы` (`vuejs`) and `реьч` (`htmx`) the same.
"""

from __future__ import annotations

import sys
import unittest
from dataclasses import replace
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

from keyswitch.constants.models import UNKNOWN_FEATURE_PREFIX  # noqa: E402
from keyswitch.context_action_features import extract_action_features, own_bucket, unknown_question, unknown_shape  # noqa: E402
from keyswitch.context_model import ContextEvidence  # noqa: E402
from keyswitch.input_context import FieldContext  # noqa: E402
import train_context_action_model as trainer  # noqa: E402


def evidence(original: str, alternative: str, before: str, *, known: bool = False) -> ContextEvidence:
    return ContextEvidence(original, alternative, 1, FieldContext("Code", "test", before, "", "unknown"), "space",
                           source_known=known, boundary_text=" ")


def head(item: ContextEvidence) -> set[str]:
    return {name.removeprefix(UNKNOWN_FEATURE_PREFIX) for name in extract_action_features(item)
            if name.startswith(UNKNOWN_FEATURE_PREFIX)}


class UnknownTokenHeadTests(unittest.TestCase):
    def test_the_question_reads_the_token_and_the_text_before_it(self) -> None:
        # Russian letters only after their Latin reading, typed right earlier in the text: without that echo the class
        # is Russian slang and truncations as often as terms (`пукты`, `иишку`), and the frozen model decides.
        self.assertFalse(unknown_question("цуиоы", "затрагивает только "))
        self.assertTrue(unknown_question("цуиоы", "смотри webjs не дает. затрагивает только "))
        self.assertFalse(unknown_question("пукты", "все "))
        # Either layout: the term typed as meant, a Latin name, and a Russian word no table counts typed in the English
        # layout are the class too; a Russian word Russian prose uses typed there (`ghbdtn`, `привет`) is not.
        self.assertTrue(unknown_question("webjs", "затрагивает только "))
        self.assertTrue(unknown_question("Porsche", "у него "))
        self.assertFalse(unknown_question("ghbdtn", "скажи "))
        # Russian prose uses the word: the frozen model has its count.
        self.assertFalse(unknown_question("только", "затрагивает "))
        # After Latin text, in capitals, shorter than four, or with a letter whose key is a sign (`фолбек` is
        # `ajk,tr`): the shape is another head's or no term's. Three letters are slang as often as terms (`зач`).
        self.assertFalse(unknown_question("цуиоы", "use the webjs "))
        self.assertFalse(unknown_question("ЦУИОЫ", "смотри WEBJS не дает. затрагивает только "))
        self.assertFalse(unknown_question("зач", "затрагивает только "))
        self.assertFalse(unknown_shape("фолбек"))
        self.assertFalse(unknown_shape("1234"))
        self.assertFalse(unknown_shape("webцуи"))

    def test_the_head_copies_the_shared_features_and_reads_echoes_and_typos(self) -> None:
        names = head(evidence("цуиоы", "webjs", "смотри webjs не дает. затрагивает только "))
        self.assertTrue({"bias", "echo:1:0:direction:1", "echo:1:0:letters:5:direction:1", "letters:5:direction:1",
                         "typo:0:0:direction:1"} <= names)
        # The same term typed as meant reads its own echo under the other layout's names.
        latin = ContextEvidence("webjs", "цуиоы", 0, FieldContext("Code", "test", "смотри webjs не дает. только ", "", "unknown"),
                                "space", boundary_text=" ")
        self.assertIn("echo:0:1:direction:0", head(latin))
        self.assertEqual(head(evidence("цуиоы", "webjs", "затрагивает только ")), set())
        # A lexicon word has no head features: the class is the tokens the lexicon does not hold.
        self.assertEqual(head(evidence("цуиоы", "webjs", "смотри webjs. затрагивает только ", known=True)), set())

    def test_dropped_counts_read_as_uncounted_for_the_token_alone(self) -> None:
        item = evidence("ньуы", "yml", "поправь конфиг ")
        self.assertNotEqual(own_bucket(item, "yml", "latin"), "0")
        dropped = replace(item, term_counts_dropped=True)
        self.assertEqual(own_bucket(dropped, "yml", "latin"), "0")
        self.assertEqual(own_bucket(dropped, "ньуы1", "latin"), "na")
        features = extract_action_features(dropped)
        self.assertTrue([name for name in features if name.startswith("freq:latin:0")])
        self.assertFalse([name for name in features if name.startswith("freq:latin:") and not name.startswith("freq:latin:0")])

    def test_count_dropout_takes_one_long_family_in_three(self) -> None:
        self.assertFalse(trainer.term_counts_dropped("unknown-term:тщв:0", "тщв"))
        self.assertFalse(trainer.term_counts_dropped("natural:row:1", "тщвуоы"))
        families = [f"unknown-term:{word}" for word in ("тщвуоы", "куфсеоы", "тучеоы", "мгуоы", "ыуйгудшяу", "ензущкь")]
        dropped = [trainer.term_counts_dropped(family + ":0", family.split(":")[1]) for family in families]
        self.assertTrue(any(dropped) and not all(dropped))

    def test_the_trainer_routes_the_class_to_the_head(self) -> None:
        prefixes = frozenset({UNKNOWN_FEATURE_PREFIX})
        self.assertTrue(trainer.head_question("цуиоы", "webjs", "смотри webjs. затрагивает только ", "", "space", "none", prefixes))
        self.assertFalse(trainer.head_question("только", "njkmrj", "затрагивает ", "", "space", "none", prefixes))
        self.assertFalse(trainer.head_question("цуиоы", "webjs", "смотри webjs. затрагивает только ", "", "space", "none", frozenset()))


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    return loader.loadTestsFromTestCase(UnknownTokenHeadTests)


if __name__ == "__main__":
    unittest.main()
