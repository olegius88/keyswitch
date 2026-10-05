"""The action features say where a word stands on its line and how often each reading occurs.

context-v1 schema 7 decided the owner's mixed typing by this evidence and the action scheme had
none of it: `зк` and `pr` read alike to its character n-grams, while the term tables know that `pr`
occurs inside Russian text and `зк` never does, and that `тз` is a Russian abbreviation.
"""

from __future__ import annotations

import unittest

from keyswitch.context_action_features import extract_action_features
from keyswitch.context_model import ContextAction, ContextEvidence
from keyswitch.context_policy import russian_slang
from keyswitch.input_context import FieldContext
from test_context_policy import ContextEngineTests
from test_early_switch_settling import ScriptedActionModel


def evidence(original: str, alternative: str, group: int, before: str = "", after: str = "", *,
             target_known: bool = False, inside: bool = False) -> ContextEvidence:
    return ContextEvidence(
        original, alternative, group, FieldContext("Telegram", "1", before, after, "text"), "space",
        False, False, target_known, 0.0, inside=inside,
    )


def named(features: dict[str, float], prefix: str) -> set[str]:
    return {name for name in features if name.startswith(prefix)}


class TermFrequencyFeatureTests(unittest.TestCase):
    def test_a_term_typed_in_the_russian_layout_is_told_from_a_russian_abbreviation(self) -> None:
        term = extract_action_features(evidence("зк", "pr", 1, "сделай "))
        abbreviation = extract_action_features(evidence("тз", "np", 1, "пришли "))
        # `pr` is a term of Russian technical text, `зк` occurs there never; `тз` occurs often.
        self.assertIn("freq:cyrillic:0:direction:1", term)
        self.assertNotIn("freq:latin:0:direction:1", term)
        self.assertNotIn("freq:cyrillic:0:direction:1", abbreviation)

    def test_capitals_are_told_apart_by_how_russian_text_uses_their_cyrillic_reading(self) -> None:
        cited = extract_action_features(evidence("WBC", "ЦИС", 0, "чемпион по версии ", target_known=True))
        country = extract_action_features(evidence("CIF", "США", 0, "санкции ", target_known=True))
        self.assertEqual(named(cited, "capitals:"), {"capitals:known:1:russian:0:direction:0",
                                                     "capitals:known:1:russian:0:before:ru:direction:0"})
        self.assertTrue(named(country, "capitals:") and not named(country, "capitals:known:1:russian:0:"))
        for original, alternative in (("wbc", "цис"), ("WB", "ЦИ"), ("Wi-Fi", "Шш-Ашш"), ("Wbc", "Цис")):
            with self.subTest(original=original):
                self.assertFalse(named(extract_action_features(evidence(original, alternative, 0, "по версии ")), "capitals:"))

    def test_a_reading_the_lexicon_knows_is_bucketed_by_its_frequency_in_its_language(self) -> None:
        word = extract_action_features(evidence("dct", "все", 0, "todo ", target_known=True))
        self.assertTrue(named(word, "freq:cyrillic:lexicon:"))
        self.assertTrue(named(word, "freq:english:"))

    def test_where_the_word_stands_on_its_line(self) -> None:
        self.assertIn("line:empty:direction:0", extract_action_features(evidence("dct", "все", 0)))
        self.assertIn("line:same:direction:0", extract_action_features(evidence("dct", "все", 0, "todo ")))
        newline = extract_action_features(evidence("dct", "все", 0, "update the docs\n"))
        self.assertIn("line:newline:direction:0", newline)
        self.assertIn("line:newline:previous:en:direction:0", newline)

    def test_letters_typed_into_a_word_and_a_word_alone_have_no_language_frequencies(self) -> None:
        # The rest of the word shows its language, and a word alone in its field waits for its
        # neighbour, as schema 7 left them.
        inside = extract_action_features(evidence("зк", "pr", 1, "сделай ", inside=True))
        alone = extract_action_features(evidence("зк", "pr", 1))
        self.assertFalse(named(inside, "freq:english:") | named(alone, "freq:english:"))
        self.assertTrue(named(inside, "inside:freq:"))

    def test_a_token_neither_table_counts_has_no_frequencies(self) -> None:
        # `1С` and `2фа` in Russian prose: an absent count is no evidence, and as one shared bucket
        # with the Latin tokens with digits the corpus restores they became `1C` and `2af`.
        for original, alternative in (("1С", "1C"), ("2фа", "2af")):
            with self.subTest(original=original):
                features = extract_action_features(evidence(original, alternative, 1, "работаю в "))
                self.assertFalse(named(features, "freq:"))
        self.assertTrue(named(extract_action_features(evidence("зь2", "pm2", 1, "перезапусти ")), "line:"))

    def test_a_dot_inside_the_latin_reading_is_a_name_of_its_own(self) -> None:
        domain = extract_action_features(evidence("ыуафтюкг", "sefan.ru", 1, "открой "))
        self.assertIn("latin:dot:direction:1", domain)


def convert(item: ContextEvidence) -> ContextAction:
    return "convert"


class RussianSlangTests(ContextEngineTests):
    def setUp(self) -> None:
        super().setUp()
        self.engine.context_policy.model = ScriptedActionModel(convert)

    def typed(self, text: str) -> str:
        self.reset_editor(1)
        self.type(text, group=1)
        return self.backend.text

    def test_a_russian_abbreviation_whose_latin_keys_nobody_uses_stays(self) -> None:
        # `пдф` and `впн` occur inside Russian technical text; `gla` and `dgy` occur nowhere.
        self.assertTrue(russian_slang("пдф", "gla", 1, False))
        self.assertEqual(self.typed("пдф "), "пдф ")
        self.assertEqual(self.typed("впн "), "впн ")

    def test_a_term_typed_in_the_russian_layout_is_the_models_to_convert(self) -> None:
        # `зк` occurs nowhere in Russian text, so its keys are `pr`; `тз` is an abbreviation, but its
        # keys spell a term (`np`), and the model decides it.
        self.assertFalse(russian_slang("зк", "pr", 1, False))
        self.assertFalse(russian_slang("тз", "np", 1, False))
        self.assertEqual(self.typed("зк "), "pr ")


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    suite = unittest.TestSuite()
    for case in (TermFrequencyFeatureTests, RussianSlangTests):
        for name in case.__dict__:
            if name.startswith("test_"):
                suite.addTest(case(name))
    return suite


if __name__ == "__main__":
    unittest.main()
