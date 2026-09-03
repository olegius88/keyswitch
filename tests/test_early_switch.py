"""Tests for prefix-based early layout switching."""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from keyswitch import early_switch  # noqa: E402
from keyswitch.early_switch import (  # noqa: E402
    EarlySwitchPolicy,
    PrefixIndex,
    early_switch_decision,
)
from keyswitch.language_model import LanguageModel, WordScore  # noqa: E402
from keyswitch.layouts import LayoutPair  # noqa: E402


class _Scorer:
    def __init__(self, known: set[str]) -> None:
        self.known = known

    def score(self, word: str) -> WordScore:
        known = word.casefold() in self.known
        return WordScore(1.0 if known else 0.0, known, 1 if known else 0, 0.0)


EN_WORDS = {"hello": 500_000, "help": 200_000, "held": 50_000, "xfce": 10}
RU_WORDS = {
    "привет": 900_000,
    "приветствие": 5_000,
    "привал": 4_000,
    "приз": 3_000,
    "почему": 800_000,
    "почесать": 1_500,
}


def _indexes() -> dict[int, PrefixIndex]:
    return {0: PrefixIndex(EN_WORDS, EN_WORDS), 1: PrefixIndex(RU_WORDS, RU_WORDS)}


def _scorers() -> dict[int, _Scorer]:
    return {0: _Scorer(set(EN_WORDS)), 1: _Scorer(set(RU_WORDS))}


class PrefixIndexTests(unittest.TestCase):
    def test_completions_count_words_and_best_frequency(self) -> None:
        index = PrefixIndex(RU_WORDS, RU_WORDS)
        evidence = index.completions("прив")
        self.assertEqual(evidence.completions, 3)
        self.assertEqual(evidence.maximum_frequency, 900_000)
        self.assertFalse(evidence.known)
        self.assertEqual(index.completions("").completions, 0)
        self.assertTrue(index.completions("ПРИВЕТ").known)
        self.assertEqual(len(index), len(RU_WORDS))
        # The frequency scan is bounded; the count is not.
        self.assertEqual(
            index.completions("прив", limit=1).as_dict(),
            {"completions": 3, "maximum_frequency": 4_000, "known": False},
        )

    def test_words_without_frequency_are_known_but_zero(self) -> None:
        index = PrefixIndex({"stem"}, {})
        evidence = index.completions("stem")
        self.assertTrue(evidence.known)
        self.assertEqual(evidence.maximum_frequency, 0)

    def test_language_model_index_is_cached_and_includes_hunspell(self) -> None:
        model = LanguageModel.load("en_US")
        first = PrefixIndex.for_language_model(model)
        second = PrefixIndex.for_language_model(model)
        self.assertIs(first, second)
        self.assertGreaterEqual(len(first), len(model.frequencies))
        lexicon_only = early_switch._cached_index("en_US", "lexicon only", 0)
        self.assertEqual(len(lexicon_only), len(model.frequencies))


class HunspellHelpersTests(unittest.TestCase):
    def test_dictionary_path_is_taken_from_source_description(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            dictionary = Path(directory) / "en_US.dic"
            dictionary.write_text("3\nhello/S\nworld\tpos\n123\nnaïve\n", encoding="utf-8")
            self.assertIsNone(early_switch._hunspell_dictionary_path("lexicon only"))
            self.assertIsNone(
                early_switch._hunspell_dictionary_path(f"Hunspell: {directory}/missing.dic")
            )
            self.assertIsNone(
                early_switch._hunspell_dictionary_path(f"Hunspell: {Path(directory) / 'en_US.aff'}")
            )
            self.assertEqual(
                early_switch._hunspell_dictionary_path(
                    f"lexicon; Hunspell: {dictionary}"
                ),
                dictionary,
            )
            self.assertEqual(
                early_switch._dictionary_stems(dictionary),
                {"hello", "world", "naïve"},
            )
            self.assertEqual(early_switch._dictionary_stems(dictionary, limit=4), set())
            self.assertEqual(
                early_switch._dictionary_stems(Path(directory) / "missing.dic"), set()
            )


class EarlySwitchDecisionTests(unittest.TestCase):
    def decide(
        self, original: str, group: int = 0, policy: EarlySwitchPolicy = EarlySwitchPolicy()
    ) -> early_switch.EarlySwitchDecision:
        pair = LayoutPair()
        alternative = (
            pair.translate(original, "us", "ru")
            if group == 0
            else pair.translate(original, "ru", "us")
        )
        return early_switch_decision(
            _indexes(), _scorers(), original, {1 - group: alternative}, group, policy=policy
        )

    def test_dominant_target_prefix_switches(self) -> None:
        decision = self.decide("ghbd")
        self.assertTrue(decision.should_switch)
        self.assertEqual((decision.source_group, decision.target_group), (0, 1))
        self.assertEqual(decision.replacement, "прив")
        self.assertEqual(decision.reason, "целевой префикс доминирует по частоте")
        payload = decision.as_dict()
        self.assertEqual(payload["source_evidence"], {"completions": 0, "maximum_frequency": 0, "known": False})
        self.assertEqual(payload["target_evidence"], {"completions": 3, "maximum_frequency": 900_000, "known": False})

    def test_many_frequent_completions_switch_without_dominance(self) -> None:
        policy = EarlySwitchPolicy(minimum_completions=2, minimum_frequency=1_000, dominant_frequency=10**9)
        decision = self.decide("gjxt", policy=policy)
        self.assertTrue(decision.should_switch)
        self.assertEqual(decision.reason, "исходный префикс невозможен, целевой начинает частотные слова")
        self.assertEqual(policy.as_dict()["dominant_frequency"], 10**9)

    def test_rejections_carry_reasons(self) -> None:
        self.assertEqual(self.decide("ghb").reason, "слишком короткий префикс")
        self.assertEqual(self.decide("hell").reason, "префикс продолжается в исходном языке")
        self.assertEqual(self.decide("GhBd").reason, "похоже на сокращение или camelCase")
        self.assertEqual(self.decide("gh1d").reason, "префикс содержит не буквы")
        weak = early_switch_decision(
            _indexes(), _scorers(), "zzzz", {1: "яяяя"}, 0
        )
        self.assertEqual(weak.reason, "целевой префикс не начинает частотных слов")
        self.assertIsNotNone(weak.target_evidence)
        self.assertFalse(weak.as_dict()["should_switch"])

    def test_known_scorer_word_without_index_entry_is_rejected(self) -> None:
        scorers = {0: _Scorer({"ghbd"}), 1: _Scorer(set())}
        decision = early_switch_decision(_indexes(), scorers, "ghbd", {1: "прив"}, 0)
        self.assertEqual(decision.reason, "префикс сам является словом исходного языка")

    def test_missing_layout_information_is_rejected(self) -> None:
        indexes = _indexes()
        no_target = early_switch_decision(indexes, _scorers(), "ghbd", {}, 0)
        self.assertEqual(no_target.reason, "нет другой раскладки")
        self.assertIsNone(no_target.as_dict()["source_evidence"])
        unknown_source = early_switch_decision(indexes, _scorers(), "ghbd", {1: "прив"}, 5)
        self.assertEqual(unknown_source.reason, "нет другой раскладки")
        unknown_target = early_switch_decision(indexes, _scorers(), "ghbd", {7: "прив"}, 0)
        self.assertEqual(unknown_target.reason, "нет другой раскладки")

    def test_real_lexicons_switch_common_words_and_keep_english(self) -> None:
        models = {0: LanguageModel.load("en_US"), 1: LanguageModel.load("ru_RU")}
        indexes = {group: PrefixIndex.for_language_model(model) for group, model in models.items()}
        pair = LayoutPair()
        for word, group in (("ghbd", 0), ("cgfc", 0), ("рудд", 1), ("фвьшт", 1)):
            alternative = (
                pair.translate(word, "us", "ru") if group == 0 else pair.translate(word, "ru", "us")
            )
            decision = early_switch_decision(indexes, models, word, {1 - group: alternative}, group)
            self.assertTrue(decision.should_switch, word)
        for word in ("hell", "json", "yaml", "xfce", "prod"):
            decision = early_switch_decision(
                indexes, models, word, {1: pair.translate(word, "us", "ru")}, 0
            )
            self.assertFalse(decision.should_switch, word)


if __name__ == "__main__":
    unittest.main()
