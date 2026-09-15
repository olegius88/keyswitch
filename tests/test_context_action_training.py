"""Pre-test action labels and calibration must preserve the declared risk budget."""

from __future__ import annotations

from array import array
from dataclasses import replace
from pathlib import Path
import json
import sys
from types import SimpleNamespace
from typing import cast
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

from freeze_context_action_corpus import CorpusRow, typo_variants
from keyswitch.context_model import SHORT_UNKNOWN_SOURCE_MAX_LENGTH, SHORT_UPPERCASE_UNKNOWN_SOURCE_MAX_LENGTH, ContextEvidence, ContextModel
from train_context_action_model import BLIND_IDENTIFIERS, IDENTIFIER_DROPOUT_FAMILIES, ROOT, ActionRow, FeatureMass, action_rows, apply_runtime_support, apply_support_mask, balance_planned_mass, choose_threshold, identifier_evidence_dropped, identifier_family, natural_lookahead_rows, evidence, historical_curriculum, legacy_lookahead_rows, metrics, previous_context, select_features, select_rows, training_order, translated
from keyswitch.detector import LanguageDetector
from keyswitch.intent_model import LinearNgramModel
from keyswitch.language_model import LanguageModel
from keyswitch.input_context import FieldContext
from train_context_model import Row as HistoricalRow


def fixture(identifier: str, original: str, group: int, *, family: str = "") -> CorpusRow:
    return CorpusRow(
        identifier=identifier, original=original, group=group,
        before="обсуждаем  ", after="!  потом", lemma=original,
        family=family or identifier, document="doc:" + identifier,
        language="ru" if group == 1 else "en", source="authored-fixture",
        source_file="fixture.conllu", source_sentence=identifier, source_token="1",
        spacing="", space_before="  ", literal_tail="!  ", upos="NOUN",
        features="_", misc="SpaceAfter=No", alignment="fixture",
        layout_representable=True, split="development",
    )


def scores(negative: float, positive: float) -> tuple[array[float], array[int]]:
    return array("d", [1 - negative, negative, 0, 0, 1 - positive, positive, 0, 0]), array("B", [0, 1])


class ActionTrainingTests(unittest.TestCase):
    def test_fractional_variants_preserve_vocabulary_ranking_and_context_mass(self) -> None:
        original = FeatureMass()
        original.add({"family-a": 8.0, "shared": .01}, 1.0)
        for _ in range(2):
            original.add({"family-b": 1.0, "shared": 1.0}, 1.0)
        expected = select_features(original.values, .25, 2)
        self.assertEqual(expected, ["family-b", "shared"])
        for count in (3, 10, 1000):
            expanded = FeatureMass()
            for _ in range(count):
                expanded.add({"family-a": 8.0, "shared": .01}, 1.0 / count)
            for _ in range(2):
                expanded.add({"family-b": 1.0, "shared": 1.0}, 1.0)
            self.assertEqual(select_features(expanded.values, .25, 2), expected)
            for name, value in original.values.items():
                self.assertAlmostEqual(expanded.values[name], value, delta=1e-12)
        contexts = FeatureMass()
        for index in range(4):
            features = {"family-a": 1.0, "shared": 1.0}
            if index == 0:
                features["new-context"] = 100.0
            contexts.add(features, .25)
        for _ in range(2):
            contexts.add({"family-b": 1.0, "shared": 1.0}, 1.0)
        self.assertEqual(contexts.values["new-context"], .25)
        self.assertEqual(select_features(contexts.values, .25, 2), expected)
        self.assertNotIn("new-context", select_features(contexts.values, .3, 10))

    def test_feature_mass_cutoff_tolerance_and_rank_ties_are_deterministic(self) -> None:
        masses = {"under": 3.0 - 1.1e-9, "near": 3.0 - .9e-9, "exact": 3.0}
        self.assertEqual(select_features(masses, 3.0, 10), ["exact", "near"])
        tied = {"z": 3.0, "a": 3.0 - 1e-12, "other": 2.9}
        self.assertEqual(select_features(tied, 3.0, 1), ["a"])
        self.assertEqual(select_features(dict(reversed(list(tied.items()))), 3.0, 1), ["a"])

    def test_feature_mass_rejects_invalid_weights_limits_and_overflow(self) -> None:
        for value in (0.0, -1.0, float("nan"), float("inf"), True):
            with self.subTest(value=value), self.assertRaises(ValueError):
                FeatureMass().add({"name": 1.0}, value)
            with self.subTest(minimum=value), self.assertRaises(ValueError):
                select_features({"name": 1.0}, value, 1)
        for value in (-1.0, float("nan"), float("inf"), True):
            with self.subTest(accumulated=value), self.assertRaises(ValueError):
                select_features({"name": value}, 1.0, 1)
        for maximum in (0, -1, True):
            with self.subTest(maximum=maximum), self.assertRaises(ValueError):
                select_features({}, 1.0, maximum)
        overflow = FeatureMass()
        overflow.add({"name": 1.0}, 1e308)
        with self.assertRaisesRegex(ValueError, "overflow"):
            overflow.add({"name": 1.0}, 1e308)

    def historical_fixture(self, rows: list[HistoricalRow]) -> list[ActionRow]:
        intent = cast(LinearNgramModel, SimpleNamespace(model_version="fixture", checksum="fixture-sha"))
        with patch("train_context_action_model.reference_models", return_value={}), \
                patch("train_context_model.build_corpus", return_value=rows):
            return historical_curriculum(intent)

    def test_historical_reduction_keeps_different_context_actions_and_application_roles(self) -> None:
        item = ContextEvidence("r", "к", 0, FieldContext("Telegram", "fixture", "", "", "text"), "space", False, False, True, 0.0)
        rows = [HistoricalRow(item, "wait", "r", "train", "short_russian_wrong"),
                HistoricalRow(replace(item, field=replace(item.field, before="я думаю что ")), "convert", "r", "train", "short_russian_wrong"),
                HistoricalRow(replace(item, field=replace(item.field, application="Code", role="unknown", before="я думаю что ")), "convert", "r", "train", "short_russian_wrong"),
                HistoricalRow(replace(item, original="x", alternative="ч"), "keep", "x", "development", "short_russian_correct")]
        actual = self.historical_fixture(rows + rows[:1])
        self.assertEqual(len(actual), 3)
        self.assertEqual({(row.action, row.field.before, row.field.application, row.field.role) for row in actual}, {
            ("wait", "", "Telegram", "text"), ("convert", "я думаю что ", "Telegram", "text"),
            ("convert", "я думаю что ", "Code", "unknown"),
        })
        self.assertEqual({row.boundary_text for row in actual}, {" "})

    def test_historical_lookahead_contains_the_single_completed_word_the_engine_passes(self) -> None:
        item = ContextEvidence("r", "к", 0, FieldContext("Telegram", "fixture", "", "меня всё устраивает", "unknown"), "space", False, False, True, 0.0)
        rows = [HistoricalRow(item, "convert", "r", "train", "short_lookahead"),
                HistoricalRow(replace(item, field=replace(item.field, after="нас это не касается")), "convert", "r", "train", "short_lookahead")]
        actual = self.historical_fixture(rows)
        self.assertEqual({row.field.after for row in actual}, {"меня", "меня всё устраивает", "нас", "нас это не касается"})
        self.assertEqual({row.boundary_text for row in actual}, {" "})
        self.assertTrue(all(row.action == "convert" and row.literal_tail == "" for row in actual))

    def test_historical_trigger_boundaries_cover_passive_and_deferred_input(self) -> None:
        item = ContextEvidence("r", "к", 0, FieldContext("Telegram", "fixture", "", "", "unknown"), "space", False, False, True, 0.0)
        rows = [HistoricalRow(replace(item, trigger=trigger), "wait", "r", "train", "short_russian_wrong")
                for trigger in ("space", "pause", "enter", "tab", "punctuation")]
        actual = self.historical_fixture(rows)
        self.assertEqual({(row.trigger, row.boundary_text) for row in actual}, {
            ("space", " "), ("pause", ""), ("enter", ""), ("enter", "\n"),
            ("tab", ""), ("tab", "\t"), ("punctuation", "."),
        })

    def test_legacy_variants_share_the_original_family_category_trigger_budget(self) -> None:
        item = ContextEvidence("r", "к", 0, FieldContext("Telegram", "fixture", "", "", "unknown"), "space", False, False, True, 0.0)
        rows = [HistoricalRow(item, "wait", "r", "train", "short_russian_wrong"),
                HistoricalRow(replace(item, field=replace(item.field, before="я думаю что ")), "convert", "r", "train", "short_russian_wrong")]
        expanded = rows + [replace(row, evidence=replace(row.evidence,
                    field=replace(row.evidence.field, application="Code", role="text"))) for row in rows]
        for source in (rows, expanded, expanded + expanded):
            actual = self.historical_fixture(source)
            self.assertAlmostEqual(sum(row.sample_weight for row in actual), 1.0)
            self.assertEqual({row.action for row in actual}, {"wait", "convert"})
            self.assertAlmostEqual(sum(row.sample_weight for row in actual if row.action == "convert"), .5)
        independent = self.historical_fixture(expanded + [replace(rows[0], category="trusted_short_wrong")])
        self.assertAlmostEqual(sum(row.sample_weight for row in independent), 2.0)

    def test_sample_weights_are_positive_finite_and_natural_rows_keep_unit_weight(self) -> None:
        rows = action_rows([fixture("weight", "пример", 1)])
        self.assertTrue(all(row.sample_weight == 1.0 for row in rows))
        for value in (0.0, -1.0, float("nan"), float("inf"), float("-inf"), True):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, "positive and finite"):
                replace(rows[0], sample_weight=value)

    def test_training_sources_are_hash_interleaved_independently_of_input_order(self) -> None:
        natural = action_rows([fixture("first", "пример", 1), fixture("second", "example", 0)])
        legacy = [replace(row, identifier="legacy-train:" + row.identifier, category="legacy_fixture") for row in natural]
        ordered = training_order(natural + legacy)
        self.assertEqual(ordered, training_order(list(reversed(legacy + natural))))
        self.assertCountEqual(ordered, natural + legacy)
        self.assertNotEqual(ordered, natural + legacy)
        self.assertGreater(sum(ordered[i].category.startswith("legacy_") != ordered[i + 1].category.startswith("legacy_")
                               for i in range(len(ordered) - 1)), 1)

    def test_historical_labels_use_pinned_models_even_with_environment_overrides(self) -> None:
        intent = cast(LinearNgramModel, SimpleNamespace(model_version="fixture", checksum="fixture-sha"))
        models = {group: LanguageModel(locale, {}, "fixture", enable_spellcheck=False)
                  for group, locale in enumerate(("en_US", "ru_RU"))}

        def build() -> list[object]:
            selected, status = LinearNgramModel.try_load_default()
            self.assertIs(selected, intent)
            self.assertTrue(status.available)
            self.assertIs(LanguageModel.load("en_US"), models[0])
            self.assertIs(LanguageModel.load("ru_RU"), models[1])
            return []

        with patch.dict("os.environ", {"KEYSWITCH_INTENT_MODEL_PATH": "/unavailable-override.ksm",
                                      "KEYSWITCH_MODEL_PATH": "/unavailable-lexicon"}), \
                patch("train_context_action_model.LinearNgramModel.load", return_value=intent) as load, \
                patch("train_context_action_model.reference_models", return_value=models) as lexical, \
                patch("train_context_model.build_corpus", side_effect=build):
            self.assertEqual(historical_curriculum(), [])
            load.assert_called_once_with(ROOT / "src/keyswitch/resources/models/layout_intent_v1.ksm")
            lexical.assert_called_once_with(True)

    def test_calibration_uses_the_same_language_support_as_runtime(self) -> None:
        features = {"source:char:0:1:a": 1.0, "target:char:0:1:ф": 1.0}
        model = ContextModel({name: (0.0,) * 4 for name in features}, "context-v3-test", feature_version=3)
        probabilities = array("d", [0.001, 0.999, 0, 0, 0.001, 0.999, 0, 0])
        rows = [(features, 1, 1.0), ({"app:telegram": 1.0}, 1, 1.0)]
        guarded = apply_runtime_support(probabilities, rows, model)
        self.assertEqual(guarded[:4], probabilities[:4])
        self.assertEqual(list(guarded[4:]), [0.0, 0.0, 0.0, 1.0])
        self.assertEqual(probabilities[5], 0.999)
        with self.assertRaises(ValueError):
            apply_runtime_support(probabilities, rows[:1], model)
        with self.assertRaises(ValueError):
            apply_runtime_support(array("d"), rows, model)

    def test_original_surface_and_typos_are_keep_independent_of_lexical_scores(self) -> None:
        source = fixture("word", "обсуждение", 1)
        rows = action_rows([source])
        naturals = [row for row in rows if row.category == "natural_surface"]
        typos = [row for row in rows if row.category == "spelling_intervention"]
        self.assertEqual({row.original for row in naturals}, {source.original})
        self.assertEqual({row.action for row in naturals + typos}, {"keep"})
        self.assertEqual({row.original for row in typos}, set(typo_variants(source.original, source.identifier)))
        self.assertTrue(typos)
        wrong = [row for row in rows if row.category == "layout_intervention"]
        self.assertEqual({row.group for row in wrong}, {0})
        self.assertEqual({translated(row.original, row.group) for row in wrong}, {source.original})

    def test_short_wrong_layout_without_neighbours_is_an_abstention(self) -> None:
        rows = action_rows([replace(fixture("short", "он", 1), before="", after="")])
        wrong = [row for row in rows if row.category == "layout_intervention"]
        self.assertEqual(len(wrong), 2)
        self.assertTrue(all(row.action in {"wait", "suggest"} for row in wrong))
        natural = [row for row in rows if row.category == "natural_surface"]
        self.assertEqual([row.action for row in natural], [row.action for row in wrong])
        self.assertTrue(all(row.action == "keep" for row in rows
                            if row.category == "mixed_language_insertion"))

    def test_contextless_short_pairs_share_the_same_deferred_action(self) -> None:
        expected = {"space": {"wait"}, "pause": {"wait"},
                    "enter": {"suggest"}, "tab": {"suggest"}, "punctuation": {"suggest"}}
        actual: dict[str, set[str]] = {trigger: set() for trigger in expected}
        for original, group in (("a", 0), ("we", 0), ("я", 1), ("мы", 1)):
            sources = [replace(fixture(f"short-pair:{index}", original, group), before="", after="")
                       for index in range(32)]
            for row in action_rows(sources):
                if row.category in {"natural_surface", "layout_intervention"}:
                    # Intended-language provenance is absent at inference.
                    # Both readings preserve text and allow the same next step.
                    actual[row.trigger].add(row.action)
        self.assertEqual(actual, expected)

    def test_digits_and_punctuation_do_not_license_a_short_layout_conversion(self) -> None:
        expected = {"space": {"wait"}, "pause": {"wait"},
                    "enter": {"suggest"}, "tab": {"suggest"}, "punctuation": {"suggest"}}
        actual: dict[str, set[str]] = {trigger: set() for trigger in expected}
        for before, after in (("(", ""), ("8-10", "?")):
            sources = [replace(fixture(f"nonlexical-short:{index}", "мы", 1), before=before, after=after)
                       for index in range(32)]
            for row in action_rows(sources):
                if row.category == "layout_intervention":
                    actual[row.trigger].add(row.action)
        self.assertEqual(actual, expected)

    def test_punctuation_alone_does_not_label_a_short_layout_conversion(self) -> None:
        for context in ("---", "...", "123", "{}", " / "):
            with self.subTest(context=context):
                rows = action_rows([replace(fixture("symbol-context", "мы", 1),
                                            before=context, after=context)])
                direct = [row for row in rows
                          if row.category in {"natural_surface", "layout_intervention"}]
                self.assertTrue(all(row.action in {"wait", "suggest"} for row in direct))

    def test_contextless_short_legacy_keep_shares_the_deferred_action(self) -> None:
        item = ContextEvidence("хз", "[p", 1, FieldContext("chrome", "fixture", "", "", "unknown"), "pause", False, False, False, 0.0)
        rows = [HistoricalRow(item, "keep", "хз", "train", "russian_unknown_correct"),
                HistoricalRow(replace(item, trigger="enter"), "keep", "хз", "train", "russian_unknown_correct"),
                HistoricalRow(replace(item, field=replace(item.field, before="я думаю ")), "keep", "хз", "train", "russian_unknown_correct"),
                HistoricalRow(replace(item, field=replace(item.field, before="8-10 ")), "keep", "хз", "train", "russian_unknown_correct"),
                HistoricalRow(replace(item, original="yf", alternative="на", source_group=0), "convert", "yf", "train", "trusted_short_wrong"),
                HistoricalRow(replace(item, original="три"), "keep", "три", "train", "russian_unknown_correct")]
        actual = {(row.trigger, row.field.before, row.category, row.original): row.action for row in self.historical_fixture(rows)}
        self.assertEqual(actual[("pause", "", "legacy_russian_unknown_correct", "хз")], "wait")
        self.assertEqual(actual[("enter", "", "legacy_russian_unknown_correct", "хз")], "suggest")
        self.assertEqual(actual[("pause", "я думаю ", "legacy_russian_unknown_correct", "хз")], "keep")
        self.assertEqual(actual[("pause", "8-10 ", "legacy_russian_unknown_correct", "хз")], "wait")
        self.assertEqual(actual[("pause", "", "legacy_trusted_short_wrong", "yf")], "convert")
        self.assertEqual(actual[("pause", "", "legacy_russian_unknown_correct", "три")], "keep")
        relabeled = self.historical_fixture(rows[:1] + rows[2:3])
        self.assertAlmostEqual(sum(row.sample_weight for row in relabeled), 1.0)
        self.assertEqual({row.action for row in relabeled}, {"wait", "keep"})

    def test_more_context_variants_do_not_dilute_another_legacy_action(self) -> None:
        item = ContextEvidence("r", "к", 0, FieldContext("Telegram", "fixture", "", "", "unknown"),
                               "space", False, False, True, 0.0)
        waiting = HistoricalRow(item, "wait", "r", "train", "short_russian_wrong")
        positive = HistoricalRow(replace(item, field=replace(item.field, before="мы говорили ")),
                                 "convert", "r", "train", "short_russian_wrong")
        for copies in (1, 3, 9):
            variants = [replace(positive, evidence=replace(positive.evidence,
                        field=replace(positive.evidence.field, application=f"Editor{index}")))
                        for index in range(copies)]
            rows = self.historical_fixture([waiting, *variants])
            self.assertAlmostEqual(sum(row.sample_weight for row in rows), 1.0)
            self.assertAlmostEqual(sum(row.sample_weight for row in rows if row.action == "wait"), .5)

    def test_foreign_insertion_context_does_not_determine_layout_label(self) -> None:
        for original, group in (("deployment", 0), ("обсуждение", 1)):
            with self.subTest(group=group):
                rows = action_rows([fixture("insertion", original, group)])
                pair = [row for row in rows if row.category.startswith("mixed_language")]
                self.assertEqual(len(pair), 2)
                keep, wrong = pair
                self.assertEqual((keep.action, wrong.action), ("keep", "convert"))
                self.assertEqual(keep.field, wrong.field)
                self.assertEqual((keep.group, wrong.group), (group, 1 - group))
                self.assertEqual(translated(wrong.original, wrong.group), keep.original)

    def test_boundary_application_and_optional_context_vary_independently_of_trigger(self) -> None:
        rows = [row for number in range(256)
                for row in action_rows([fixture("variant:" + str(number), "пример", 1)])
                if row.category == "natural_surface" and row.identifier.endswith(":observed:keep")]
        for trigger in ("space", "enter", "punctuation", "tab", "pause"):
            with self.subTest(trigger=trigger):
                selected = [row for row in rows if row.trigger == trigger]
                self.assertEqual({row.field.application for row in selected},
                                 {"Telegram", "Code", "chrome", "UnseenEditor"})
                self.assertEqual({row.literal_tail for row in selected}, {""})
                self.assertEqual({bool(row.field.after) for row in selected}, {False, True})
                if trigger in ("enter", "tab"):
                    self.assertEqual({row.boundary_text for row in selected},
                                     {"", "\n" if trigger == "enter" else "\t"})

    def test_training_evidence_preserves_shift_letters_on_punctuation_keys(self) -> None:
        detector = LanguageDetector({
            0: LanguageModel("en_US", {"an": 1}, "fixture", enable_spellcheck=False),
            1: LanguageModel("ru_RU", {"он": 1}, "fixture", enable_spellcheck=False),
        })
        for original, physical in (("Жук", ":er"), ("Буря", "<ehz"), ("Хлопок", "{kjgjr")):
            with self.subTest(original=original):
                rows = action_rows([fixture("shift", original, 1)])
                wrong = next(row for row in rows if row.category == "layout_intervention")
                item = evidence(wrong, detector, None)
                self.assertEqual((item.original, item.alternative), (physical, original))
                neighbour = replace(wrong, field=replace(wrong.field, before=original + " "))
                self.assertEqual(previous_context(neighbour), ({1: original, 0: physical}, 1))
                unsupported = replace(wrong, field=replace(wrong.field, before="don’t "))
                self.assertEqual(previous_context(unsupported), ({}, None))

    def test_selection_balances_languages_and_caps_repeated_families_before_scoring(self) -> None:
        rows = [fixture(str(index), "слово", 1, family="repeated") for index in range(12)]
        rows += [fixture("en" + str(index), "word", 0) for index in range(8)]
        rows.append(replace(fixture("unsupported", "日本語", 0), layout_representable=False))
        rows.append(fixture("coarse-only", "can’t", 0))
        selected = select_rows(rows, 8)
        self.assertEqual(sum(row.group == 1 for row in selected), 2)
        self.assertEqual(sum(row.group == 0 for row in selected), 4)
        self.assertEqual(selected, select_rows(list(reversed(rows)), 8))

    def test_threshold_rejects_false_intervention_in_either_profile(self) -> None:
        threshold, report, passed = choose_threshold(
            {"portable": scores(0.98, 0.9999), "reference_hunspell": scores(0.995, 0.9999)},
            [0.95, 0.99, 0.999], 0, 0.9,
        )
        self.assertTrue(passed)
        self.assertEqual(threshold, 0.999)
        self.assertEqual(report["false_conversions"], 0)
        self.assertEqual(report["conversion_recall"], 1.0)

    def test_serving_threshold_never_drops_below_the_development_operating_threshold(self) -> None:
        predictions = {"portable": scores(0.98, 0.9999), "reference_hunspell": scores(0.98, 0.9999)}
        threshold, report, passed = choose_threshold(predictions, [0.95, 0.99, 0.999, 0.9995], 0, 0.9)
        self.assertEqual((threshold, passed), (0.99, True))
        threshold, report, passed = choose_threshold(predictions, [0.95, 0.99, 0.999, 0.9995], 0, 0.9, minimum_threshold=0.9995)
        self.assertEqual((threshold, passed), (0.9995, True))
        self.assertEqual(report["false_conversions"], 0)
        with self.assertRaises(ValueError):
            choose_threshold(predictions, [0.95, 0.99], 0, 0.9, minimum_threshold=0.999)
        recipe = json.loads((ROOT / "model/context_v3/recipe.json").read_bytes())
        self.assertEqual(recipe["epoch_selection"]["serving_threshold_floor"], "development operating threshold of the selected epoch")

    def test_no_conversion_is_not_a_successful_calibration(self) -> None:
        _, report, passed = choose_threshold({"portable": scores(0.99, 0.98)}, [1.0], 0, 0.9)
        self.assertFalse(passed)
        self.assertEqual(report["false_conversions"], 0)
        self.assertEqual(report["conversion_recall"], 0.0)
        self.assertEqual(metrics(array("d"), array("B"), 0.95)["conversion_recall"], 0.0)

    def test_a_good_profile_cannot_mask_a_failed_profile(self) -> None:
        _, report, passed = choose_threshold(
            {"portable": scores(0.1, 0.9999), "reference_hunspell": scores(0.1, 0.5)},
            [0.95], 0, 0.4,
        )
        self.assertEqual(report["conversion_recall"], 0.5)
        self.assertFalse(passed)
        with self.assertRaises(ValueError):
            choose_threshold({}, [0.95], 0, 0.9)
        with self.assertRaises(ValueError):
            choose_threshold({"portable": scores(0.1, 0.9)}, [], 0, 0.9)


class LegacyLookaheadIntegrationTests(unittest.TestCase):
    """Bounded old TRAIN seeds gain planned variants without new mass, labels or anchors."""

    def setUp(self) -> None:
        self.detector = LanguageDetector({
            0: LanguageModel("en_US", {"hello": 5000, "world": 5000}, "fixture", enable_spellcheck=False),
            1: LanguageModel("ru_RU", {"привет": 5000, "работа": 5000}, "fixture", enable_spellcheck=False),
        })
        self.convert = ActionRow("legacy-train:convert", "r", 0, FieldContext("Telegram", "public-training", "", "привет дальше"),
                                 "space", "", "convert", "legacy_short_lookahead", " ", 0.25, parent_family="family-a")
        self.keep = ActionRow("legacy-train:keep", "r", 0, FieldContext("Code", "public-training", "const value = ", ""),
                              "space", "", "keep", "legacy_technical", " ", 0.75, parent_family="family-a")
        self.long = ActionRow("legacy-train:long", "hello", 0, FieldContext("Telegram", "public-training", "", ""),
                              "space", "", "keep", "legacy_english_correct", " ", 1.0, parent_family="family-b")
        self.natural = ActionRow("natural:keep", "r", 0, FieldContext("Telegram", "public-training", "", "работа"),
                                 "space", "", "keep", "natural_surface", " ")
        self.rows = [self.natural, self.long, self.keep, self.convert]

    def lookahead(self, rows: list[ActionRow]) -> tuple[list[ActionRow], dict[str, object]]:
        return legacy_lookahead_rows(rows, self.detector, None, profile="portable", maximum_families=64, seeds_per_family=128)

    def test_short_legacy_seeds_gain_planned_variants_and_keep_their_mass(self) -> None:
        result, report = self.lookahead(self.rows)
        by_id = {row.identifier: row for row in result}
        self.assertEqual(by_id["natural:keep"], self.natural)
        self.assertEqual(by_id["legacy-train:long"], self.long)
        planned = [row for row in result if row.after_origin == "planned_next_conversion"]
        self.assertEqual({row.identifier for row in planned},
                         {"legacy-train:convert:planned:legacy-train:convert:first-after",
                          "legacy-train:keep:planned:legacy-train:convert:first-after"})
        self.assertEqual({row.field.after for row in planned}, {"привет"})
        for seed in (self.convert, self.keep):
            variants = [row for row in result if row.identifier.split(":planned:")[0] == seed.identifier]
            self.assertEqual(len(variants), 2)
            self.assertAlmostEqual(sum(row.sample_weight for row in variants), seed.sample_weight)
            for row in variants:
                self.assertEqual((row.action, row.category, row.parent_family, row.original, row.group, row.trigger,
                                  row.boundary_text, row.literal_tail, row.field.before, row.field.application),
                                 (seed.action, seed.category, seed.parent_family, seed.original, seed.group, seed.trigger,
                                  seed.boundary_text, seed.literal_tail, seed.field.before, seed.field.application))
        self.assertEqual(by_id["legacy-train:convert"].field.after, "привет дальше")
        self.assertEqual(by_id["legacy-train:keep"].field.after, "")
        self.assertAlmostEqual(cast(float, report["input_mass"]), 3.0)
        self.assertAlmostEqual(cast(float, report["output_mass"]), 3.0)
        self.assertAlmostEqual(sum(row.sample_weight for row in result), 3.0)
        counts = cast(dict[str, int], report["counts"])
        self.assertEqual((counts["input_seeds"], counts["input_anchors"], counts["planned_convert"], counts["planned_keep"]), (2, 1, 1, 1))
        self.assertEqual(report["mass_by_origin_action"], {"field:convert": .125, "planned_next_conversion:convert": .125,
                                                           "none:keep": .375, "planned_next_conversion:keep": .375})

    def test_only_legacy_right_contexts_supply_anchors(self) -> None:
        result, report = self.lookahead([self.natural, self.keep])
        self.assertEqual({row.identifier for row in result}, {"natural:keep", "legacy-train:keep"})
        counts = cast(dict[str, int], report["counts"])
        self.assertEqual((counts["input_anchors"], counts["skipped_missing_anchor"]), (0, 1))
        self.assertAlmostEqual(sum(row.sample_weight for row in result), 1.75)

    def test_unsafe_and_unreachable_legacy_rows_are_left_untouched(self) -> None:
        variants = [
            replace(self.keep, identifier="legacy-train:sensitive", field=replace(self.keep.field, sensitive=True)),
            replace(self.keep, identifier="legacy-train:selection", field=replace(self.keep.field, selection=True)),
            replace(self.keep, identifier="legacy-train:password", field=replace(self.keep.field, role="password")),
            replace(self.keep, identifier="legacy-train:enter", trigger="enter", boundary_text="\n"),
            replace(self.keep, identifier="legacy-train:pause", trigger="pause", boundary_text=""),
            replace(self.keep, identifier="legacy-train:tail", literal_tail="!"),
            replace(self.keep, identifier="legacy-train:long-word", original="hello"),
        ]
        result, report = self.lookahead([*variants, self.convert])
        for row in variants:
            self.assertIn(row, result)
        planned = [row for row in result if row.after_origin == "planned_next_conversion"]
        self.assertEqual({row.identifier.split(":planned:")[0] for row in planned}, {"legacy-train:convert"})
        self.assertEqual(cast(dict[str, int], report["counts"])["input_seeds"], 1)
        self.assertAlmostEqual(sum(row.sample_weight for row in result), sum(row.sample_weight for row in [*variants, self.convert]))

    def test_result_is_independent_of_input_order(self) -> None:
        first, first_report = self.lookahead(self.rows)
        second, second_report = self.lookahead(list(reversed(self.rows)))
        self.assertEqual(sorted(first, key=lambda row: row.identifier), sorted(second, key=lambda row: row.identifier))
        self.assertEqual(first_report, second_report)


class RuntimePolicyMaskTests(unittest.TestCase):
    def test_calibration_applies_the_short_uppercase_unknown_source_policy_like_runtime(self) -> None:
        vocabulary = {"source:char:0:1:k": 1.0, "target:char:0:1:л": 1.0}
        model = ContextModel({name: (0.0,) * 4 for name in vocabulary}, "context-v3-test", feature_version=3)
        blocked = {**vocabulary, "source:case:upper": 1.0, "source:known:0": 1.0, "length:3": 1.0}
        longer = {**vocabulary, "source:case:upper": 1.0, "source:known:0": 1.0, "length:4": 1.0}
        known = {**vocabulary, "source:case:upper": 1.0, "source:known:1": 1.0, "length:3": 1.0}
        lower = {**vocabulary, "source:case:lower": 1.0, "source:known:0": 1.0, "length:3": 1.0,
                 "before:script:ru:direction:1": 1.0, "after:script:none:direction:1": 1.0}
        rows = [(features, 1, 1.0) for features in (blocked, longer, known, lower)]
        isolated = {**lower, "before:script:none:direction:1": 1.0}
        del isolated["before:script:ru:direction:1"]
        self.assertFalse(model.allows_automatic_conversion(isolated))
        convert = [0.001, 0.999, 0.0, 0.0]
        probabilities = array("d", convert * 4)
        guarded = apply_runtime_support(probabilities, rows, model)
        self.assertEqual(list(guarded[:4]), [0.0, 0.0, 0.0, 1.0])
        for index in (1, 2, 3):
            with self.subTest(index=index):
                self.assertEqual(list(guarded[index * 4:index * 4 + 4]), convert)
        keep = array("d", [0.999, 0.001, 0.0, 0.0] * 4)
        self.assertEqual(list(apply_runtime_support(keep, rows, model)), list(keep))
        with self.assertRaises(ValueError):
            apply_support_mask(probabilities, [True] * 4, [True] * 3)

    def test_identifier_evidence_dropout_is_deterministic_per_family_and_train_only(self) -> None:
        family = "debian-trixie-main-amd64:command:nthash"
        variants = [family + ":spelling:1", family + ":observed", family]
        decisions = {identifier_evidence_dropped(name) for name in variants}
        self.assertEqual(len(decisions), 1)
        self.assertEqual(identifier_family(family + ":spelling:1"), family)
        self.assertEqual(identifier_family("UD_Russian-Taiga:x:y"), "UD_Russian-Taiga:x:y")
        commands = [f"debian-trixie-main-amd64:command:{name}" for name in ("abc", "def", "ghi", "jkl", "mno", "pqr", "stu", "vwx", "yza", "bcd", "efg", "hij")]
        dropped = sum(identifier_evidence_dropped(name) for name in commands)
        self.assertTrue(0 < dropped < len(commands))
        self.assertEqual(IDENTIFIER_DROPOUT_FAMILIES, 3)
        self.assertEqual(BLIND_IDENTIFIERS.identifiers, frozenset())
        recipe = json.loads((ROOT / "model/context_v3/recipe.json").read_bytes())
        self.assertEqual(recipe["identifier_lexicon"]["training_dropout"]["families"], IDENTIFIER_DROPOUT_FAMILIES)

    def test_recipe_documents_the_runtime_policy_bound_the_model_enforces(self) -> None:
        recipe = json.loads((ROOT / "model/context_v3/recipe.json").read_bytes())
        policy = recipe["runtime_policy"]["short_unknown_source"]
        self.assertEqual(policy["maximum_length"], SHORT_UPPERCASE_UNKNOWN_SOURCE_MAX_LENGTH)
        self.assertEqual(policy["maximum_length"], SHORT_UNKNOWN_SOURCE_MAX_LENGTH)
        self.assertEqual(policy["action"], "suggest")
        self.assertEqual(recipe["gate_policy"]["sequence_net_restorations_at_least_baseline"], True)
        self.assertNotIn("sequence_restored_at_least_baseline", recipe["gate_policy"])


class NaturalLookaheadIntegrationTests(unittest.TestCase):
    """Natural short typing frames gain planned variants from their own sentence, per split."""

    def setUp(self) -> None:
        self.detector = LanguageDetector({
            0: LanguageModel("en_US", {"hello": 5000, "world": 5000}, "fixture", enable_spellcheck=False),
            1: LanguageModel("ru_RU", {"привет": 5000, "этого": 5000, "работа": 5000}, "fixture", enable_spellcheck=False),
        })

    def corpus_row(self, identifier: str, original: str, group: int, before: str, after: str, split: str = "train") -> CorpusRow:
        return CorpusRow(identifier, original, group, before, after, original, "family-" + identifier, "doc-" + identifier.split(":")[0],
                         "ru" if group == 1 else "en", "fixture", "fixture.conllu", "1", "1", " ", "", "", "ADP", "", "", "", True, split)

    def test_field_start_one_letter_word_gains_a_planned_variant_from_its_own_sentence(self) -> None:
        source = self.corpus_row("d1:1", "у", 1, "", "этого не касается")
        rows = [row for row in action_rows([source]) if row.trigger == "space"]
        result, report = natural_lookahead_rows(rows, [source], self.detector, None, profile="portable", split="train",
                                                maximum_families=64, seeds_per_family=32)
        planned = [row for row in result if row.after_origin == "planned_next_conversion"]
        self.assertTrue(planned)
        convert = [row for row in planned if row.action == "convert"]
        self.assertTrue(convert)
        for row in convert:
            self.assertEqual((row.original, row.group, row.field.after, row.category, row.trigger, row.boundary_text),
                             ("e", 0, "этого", "layout_intervention", "space", " "))
        self.assertIn("", {row.field.before for row in convert})
        self.assertAlmostEqual(cast(float, report["input_mass"]), cast(float, report["output_mass"]))
        self.assertAlmostEqual(sum(row.sample_weight for row in rows), sum(row.sample_weight for row in result))
        originals = {row.identifier: row for row in result if row.after_origin != "planned_next_conversion"}
        for row in convert:
            seed = originals[row.identifier.split(":planned:")[0]]
            self.assertEqual(seed.field.after, "")
            self.assertAlmostEqual(seed.sample_weight, row.sample_weight)
        counts = cast(dict[str, int], report["counts"])
        self.assertGreaterEqual(counts["planned_convert"], 1)

    def test_development_frames_use_development_anchors_only(self) -> None:
        # The same identifier as the train fixture: triggers are chosen by identifier hash.
        source = self.corpus_row("d1:1", "у", 1, "", "этого не касается", split="development")
        rows = [row for row in action_rows([source]) if row.trigger == "space"]
        result, report = natural_lookahead_rows(rows, [source], self.detector, None, profile="portable", split="development",
                                                maximum_families=64, seeds_per_family=32)
        self.assertTrue([row for row in result if row.after_origin == "planned_next_conversion"])
        with self.assertRaisesRegex(ValueError, "declared split"):
            natural_lookahead_rows(rows, [source], self.detector, None, profile="portable", split="train",
                                   maximum_families=64, seeds_per_family=32)

    def test_long_words_and_missing_continuations_are_left_untouched(self) -> None:
        long_word = self.corpus_row("d3:1", "привет", 1, "", "этого")
        silent = self.corpus_row("d3:2", "у", 1, "он ", "")
        rows = [row for row in action_rows([long_word, silent]) if row.trigger == "space"]
        result, report = natural_lookahead_rows(rows, [long_word, silent], self.detector, None, profile="portable", split="train",
                                                maximum_families=64, seeds_per_family=32)
        self.assertEqual([row for row in result if row.after_origin == "planned_next_conversion"], [])
        self.assertEqual(sorted(row.identifier for row in result), sorted(row.identifier for row in rows))


class PlannedMassBalanceTests(unittest.TestCase):
    def test_planned_frames_take_the_isolated_deferred_mass_of_their_class(self) -> None:
        alone = FieldContext("Telegram", "public-training", "", "")
        planned_field = FieldContext("Telegram", "public-training", "", "этого")
        rows = [
            ActionRow("iso:1", "e", 0, alone, "space", "", "wait", "layout_intervention", " ", 1.0),
            ActionRow("iso:2", "r", 0, alone, "space", "", "wait", "layout_intervention", " ", 1.0),
            ActionRow("iso:3", "e", 0, FieldContext("Telegram", "public-training", "12 ", "!"), "enter", "", "suggest", "natural_surface", "\\n", 2.0),
            ActionRow("ctx:1", "e", 0, FieldContext("Telegram", "public-training", "слово ", ""), "space", "", "convert", "layout_intervention", " ", 5.0),
            ActionRow("planned:1", "e", 0, planned_field, "space", "", "convert", "legacy_short_lookahead", " ", 0.01, after_origin="planned_next_conversion"),
            ActionRow("planned:2", "r", 0, planned_field, "space", "", "convert", "legacy_short_lookahead", " ", 0.03, after_origin="planned_next_conversion"),
            ActionRow("planned:ru", "у", 1, planned_field, "space", "", "keep", "natural_surface", " ", 0.5, after_origin="planned_next_conversion"),
            ActionRow("long", "hello", 0, alone, "space", "", "keep", "natural_surface", " ", 1.0),
        ]
        result, report = balance_planned_mass(rows)
        by_id = {row.identifier: row for row in result}
        self.assertAlmostEqual(by_id["planned:1"].sample_weight + by_id["planned:2"].sample_weight, 4.0)
        self.assertAlmostEqual(by_id["planned:1"].sample_weight / by_id["planned:2"].sample_weight, 1 / 3)
        self.assertEqual(by_id["planned:ru"].sample_weight, 0.5)
        for name in ("iso:1", "iso:2", "iso:3", "ctx:1", "long"):
            self.assertEqual(by_id[name].sample_weight, {"iso:3": 2.0, "ctx:1": 5.0}.get(name, 1.0))
        self.assertEqual(cast(dict[str, float], report["scale"]), {"0:1": 100.0})
        self.assertEqual(cast(dict[str, float], report["isolated_mass"]), {"0:1": 4.0})
        self.assertAlmostEqual(cast(float, report["output_mass"]), cast(float, report["input_mass"]) - 0.04 + 4.0)
        self.assertEqual([row.identifier for row in result], [row.identifier for row in rows])
        again, _ = balance_planned_mass(list(reversed(rows)))
        self.assertEqual({row.identifier: row.sample_weight for row in again}, {row.identifier: row.sample_weight for row in result})
        recipe = json.loads((ROOT / "model/context_v3/recipe.json").read_bytes())
        self.assertIn("planned_mass_policy", recipe)


class PlannedEvidenceTests(unittest.TestCase):
    def test_planned_frame_evidence_uses_the_next_word_as_its_only_context(self) -> None:
        detector = LanguageDetector({
            0: LanguageModel("en_US", {"hello": 5000}, "fixture", enable_spellcheck=False),
            1: LanguageModel("ru_RU", {"этого": 5000, "привет": 5000}, "fixture", enable_spellcheck=False),
        })
        waiting = ActionRow("planned:seed", "e", 0, FieldContext("Telegram", "public-training", "", ""), "space", "", "convert", "layout_intervention", " ")
        planned = replace(waiting, identifier="planned:frame", field=FieldContext("Telegram", "public-training", "", "этого"), after_origin="planned_next_conversion")
        alone = evidence(waiting, detector, None)
        with_next = evidence(planned, detector, None)
        self.assertFalse(alone.baseline_convert)
        self.assertTrue(with_next.baseline_convert)
        self.assertEqual((with_next.after_origin, with_next.field.after), ("planned_next_conversion", "этого"))
        with_context = replace(planned, field=FieldContext("Telegram", "public-training", "hello ", "этого"))
        self.assertTrue(evidence(with_context, detector, None).baseline_convert)
        with self.assertRaises(ValueError):
            evidence(replace(planned, field=FieldContext("Telegram", "public-training", "", "...")), detector, None)
