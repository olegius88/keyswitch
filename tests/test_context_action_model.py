"""Experimental v3 feature polarity, abstention and policy integration."""

from __future__ import annotations

import hashlib
import json
import math
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path
from typing import cast
from unittest.mock import patch

from keyswitch.context_action_features import extract_action_features
from keyswitch.context_model import ACTIONS, ContextEvidence, ContextModel, ContextPrediction, extract_context_features
from keyswitch.context_policy import ContextPolicy, evidence_for_decision
from keyswitch.detector import DetectionDecision, LanguageDetector
from keyswitch.input_context import FieldContext
from keyswitch.language_model import LanguageModel, WordScore
from keyswitch.ortho_model import OrthoEvidence, OrthoModel, OrthoScore
import test_input_sequence_matrix as sequences


ZERO = (0.0, 0.0, 0.0, 0.0)
SCORE = WordScore(2.0, False, 20, 0.8, False, False, 3.0, 0.0, -2.0)


def supported_weights(item: ContextEvidence) -> dict[str, tuple[float, ...]]:
    return {
        name: ZERO for name in extract_action_features(item)
        if name.startswith(("source:char:", "target:char:"))
    }


class RecordModel(ContextModel):
    def __init__(self) -> None:
        super().__init__({"bias": (20.0, 0.0, 0.0, 0.0)}, "context-v3-record", feature_version=3)
        self.items: list[ContextEvidence] = []

    def predict(self, item: ContextEvidence) -> ContextPrediction:
        self.items.append(item)
        return super().predict(item)


class ContextActionEngineTests(unittest.TestCase):
    def test_engine_passes_literal_completing_glyph_without_changing_text(self) -> None:
        for glyph in (" ", "\u00a0", "\t", "!"):
            with self.subTest(glyph=repr(glyph)), sequences.session() as current:
                model = RecordModel()
                current.engine.context_policy.model = model
                current.physical("ghbdtn" + glyph)
                self.assertEqual(len(model.items), 1)
                self.assertEqual(model.items[0].boundary_text, glyph)
                self.assertEqual(model.items[0].original, "ghbdtn")
                self.assertEqual(model.items[0].alternative, "привет")
                self.assertEqual(current.backend.text, "ghbdtn" + glyph)
                self.assertEqual(current.backend.injections, [])

    def test_lookahead_keeps_the_previous_boundary_when_the_next_boundary_differs(self) -> None:
        with sequences.session() as current:
            model = RecordModel()
            current.engine.context_policy.model = model
            with patch.object(ContextModel, "predict", return_value=ContextPrediction("wait", 1.0, (0.0, 0.0, 1.0, 0.0), model.version, True)) as predict:
                current.physical("e ")
                self.assertIsNotNone(current.engine._context_waiting)
                predict.return_value = ContextPrediction("convert", 1.0, (0.0, 1.0, 0.0, 0.0), model.version, True)
                current.physical("'njuj\u00a0")
            self.assertEqual([(item.original, item.boundary_text, item.field.after) for item in model.items],
                             [("e", " ", ""), ("'njuj", "\u00a0", ""), ("e", " ", "этого")])
            self.assertEqual(current.backend.text, "у этого\u00a0")
            self.assertEqual(len(current.backend.injections), 1)


class ContextActionFeatureTests(unittest.TestCase):
    def setUp(self) -> None:
        self.item = ContextEvidence(
            "флуд", "akel", 1, FieldContext("Editor", "1", "обсудим ", "", "text"),
            source_score=SCORE, target_score=replace(SCORE, known=True, exact=True, frequency=100),
            target_known=True,
        )

    def test_v2_feature_golden_is_unchanged_and_new_evidence_is_ignored(self) -> None:
        item = ContextEvidence("gh", "пр", 0, FieldContext("Telegram", "1", "я ", "", "text"))
        features = extract_context_features(item)
        digest = hashlib.sha256(json.dumps(features, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode()).hexdigest()
        self.assertEqual(digest, "1eabba3a049f1d3f6552b04c7b98bcbb454fd03f662f8925ee7a43363c4c928b")
        model = ContextModel({"bias": (0.0, 8.0, 0.0, 0.0), "app:telegram": ZERO}, "context-v1-fixture")
        rich = replace(item, source_score=SCORE, target_score=SCORE, model_probability=0.6,
                       model_threshold=0.99, literal_tail="...", ortho_score=100.0, ortho_threshold=1.0,
                       boundary_text="\u00a0")
        self.assertEqual(model.predict(item), model.predict(rich))
        self.assertEqual(model.predict(rich).action, "convert")
        self.assertEqual(model.feature_version, 2)

    def test_current_boundary_is_distinct_from_neighbor_whitespace_and_literal_tail(self) -> None:
        for glyph in (" ", "\u00a0", "\t", "!", ""):
            with self.subTest(glyph=repr(glyph)):
                features = extract_action_features(replace(self.item, boundary_text=glyph))
                self.assertEqual(features.get("boundary:length", 0.0), len(glyph) / 8)
                self.assertEqual(features.get("boundary:isspace", 0.0), float(glyph.isspace()))
                for candidate in (" ", "\u00a0", "\t", "!"):
                    self.assertEqual(features.get(f"boundary:char:{ord(candidate):04x}", 0.0), float(glyph == candidate) / 8)
        oversized = extract_action_features(replace(self.item, boundary_text="\t" * 100))
        self.assertEqual(oversized["boundary:length"], 1.0)
        self.assertEqual(oversized["boundary:overflow"], 1.0)
        self.assertEqual(oversized["boundary:char:0009"], 1.0)

    def test_unknown_source_has_its_own_lexical_evidence_despite_a_known_alternative(self) -> None:
        features = extract_action_features(self.item)
        self.assertEqual(features["source:known:0"], 1.0)
        self.assertEqual(features["target:known:1"], 1.0)
        self.assertEqual(features["source:ngram_score"], 0.3)
        self.assertEqual(features["target:exact:1"], 1.0)
        self.assertGreater(features["target:logfrequency"], features["source:logfrequency"])
        weights = supported_weights(self.item)
        weights.update({"source:ngram_score": (30.0, 0.0, 0.0, 0.0), "target:known:1": (0.0, 1.0, 0.0, 0.0)})
        model = ContextModel(weights, "context-v3-fixture", feature_version=3)
        self.assertEqual(model.predict(self.item).action, "keep")

    def test_source_and_target_polarity_can_reverse_the_action_without_changing_the_words(self) -> None:
        weights = supported_weights(self.item)
        weights.update({"source:ngram_score": (30.0, 0.0, 0.0, 0.0), "target:ngram_score": (0.0, 30.0, 0.0, 0.0)})
        model = ContextModel(weights, "context-v3-fixture", feature_version=3)
        source = replace(SCORE, ngram_score=5.0)
        target = replace(SCORE, ngram_score=-5.0)
        self.assertEqual(model.predict(replace(self.item, source_score=source, target_score=target)).action, "keep")
        self.assertEqual(model.predict(replace(self.item, source_score=target, target_score=source)).action, "convert")
        identical = extract_action_features(replace(self.item, original="abc", alternative="abc"))
        self.assertGreater(identical["source:char:1:3:abc"], 0)
        self.assertGreater(identical["target:char:1:3:abc"], 0)

    def test_case_whitespace_neighbors_tail_and_numeric_margins_are_distinct(self) -> None:
        item = replace(self.item, original="Флуд", alternative="AkEl", literal_tail=",...",
                       field=replace(self.item.field, before="one  \u00a0", after="\t два"),
                       model_probability=0.8, model_threshold=0.9, ortho_score=20, ortho_threshold=4)
        features = extract_action_features(item)
        self.assertIn("source:case:title", features)
        self.assertIn("target:case:mixed", features)
        self.assertEqual(features["before:space_count"], 3 / 16)
        self.assertEqual(features["before:space:00a0"], 1 / 16)
        self.assertEqual(features["after:space:0009"], 1 / 16)
        self.assertIn("before:char:1:3:one", features)
        self.assertIn("after:char:1:3:два", features)
        self.assertEqual(features["tail:char:002e"], 3 / 8)
        self.assertAlmostEqual(features["baseline:margin"], -0.1)
        self.assertEqual(features["ortho:margin"], 16 / 128)
        self.assertNotEqual(features, extract_action_features(self.item))

    def test_extreme_and_empty_text_produce_bounded_finite_features(self) -> None:
        variants = (
            replace(self.item, original="", alternative="", source_score=None, target_score=None, field=FieldContext("", "1")),
            replace(self.item, original="1", alternative="!", literal_tail="." * 100),
            replace(self.item, original="A" * 5000, alternative="Аb" * 5000,
                    field=FieldContext("EDITOR" * 500, "1", "a" * 300 + " " + "a" * 300 + " " * 50, "\n" * 500)),
            replace(self.item, original="ё", alternative="a", field=FieldContext("", "1", "🙂", "xя"),
                    source_score=replace(SCORE, value=-1000, gram_ratio=10, ngram_score=1000, invalid_ratio=-3, raw_ngram_score=2, frequency=10**100),
                    model_probability=1000, model_threshold=-1000, ortho_score=-1000, ortho_threshold=1000),
        )
        for item in variants:
            with self.subTest(original=item.original[:10]):
                features = extract_action_features(item)
                self.assertLess(len(features), 3000)
                self.assertTrue(all(math.isfinite(value) and abs(value) <= 8 for value in features.values()))
                self.assertTrue(all(len(name) <= 160 for name in features))
        self.assertIn("source:case:none", extract_action_features(variants[0]))
        self.assertIn("source:case:upper", extract_action_features(variants[2]))
        self.assertIn("target:script:mixed", extract_action_features(variants[2]))

    def test_orthographic_evidence_is_conditioned_on_language_and_source_support(self) -> None:
        item = replace(self.item, ortho_score=20.0, ortho_threshold=4.0,
                       source_known=False, field=FieldContext("Editor", "1", "обсуждаем "))
        name = "ortho:positive:direction:1:source_known:0:context:ru"
        variants = (replace(item, source_known=True), replace(item, source_group=0),
                    replace(item, field=FieldContext("Editor", "1", "discuss ")),
                    replace(item, ortho_score=0.0))
        weights = supported_weights(item)
        for changed in variants:
            weights.update(supported_weights(changed))
        weights.update({"bias": (2.0, 0.0, 0.0, 0.0), name: (0.0, 128.0, 0.0, 0.0)})
        model = ContextModel(weights, "context-v3-fixture", feature_version=3)
        self.assertEqual(model.predict(item).action, "convert")
        for changed in variants:
            with self.subTest(item=changed):
                self.assertEqual(model.predict(changed).action, "keep")
        negative = extract_action_features(replace(item, ortho_score=0.0))
        self.assertEqual(negative["ortho:negative:direction:1"], -4 / 128)
        self.assertNotIn(name, negative)

    def test_orthographic_threshold_side_is_learned_and_does_not_license_conversion(self) -> None:
        item = replace(self.item, original="рещз", alternative="htop", source_known=False,
                       ortho_score=12.0, ortho_threshold=10.0)
        name = "ortho:side:positive:direction:1:source_known:0:case:lower"
        weights = supported_weights(item)
        weights.update({"bias": (4.0, 0.0, 0.0, 0.0), name: (0.0, 20.0, 0.0, 0.0)})
        model = ContextModel(weights, "context-v3-fixture", feature_version=3)
        # A coefficient can express the same policy close to and far from the
        # pre-existing learned threshold; the raw linear margin remains separate.
        for margin in (0.001, 2.0, 32.0):
            with self.subTest(margin=margin):
                self.assertEqual(model.predict(replace(item, ortho_score=10.0 + margin)).action, "convert")
        for changed in (replace(item, ortho_score=10.0), replace(item, ortho_score=9.0),
                        replace(item, source_known=True), replace(item, original="РЕЩЗ", alternative="HTOP")):
            with self.subTest(item=changed):
                self.assertEqual(model.predict(changed).action, "keep")
        weights[name] = (20.0, 0.0, 0.0, 0.0)
        keep = ContextModel(weights, "context-v3-fixture", feature_version=3)
        self.assertEqual(keep.predict(item).action, "keep")

    def test_context_script_can_resolve_short_words_without_rewriting_long_insertions(self) -> None:
        short = ContextEvidence("y", "н", 0, FieldContext("Editor", "1", "", "пример"))
        long = replace(short, original="pipeline", alternative="зшзудшту")
        weights = supported_weights(short) | supported_weights(long)
        weights.update({"bias": (5.0, 0.0, 0.0, 0.0),
                        "after:script:ru:direction:0:length:1": (0.0, 20.0, 0.0, 0.0)})
        model = ContextModel(weights, "context-v3-fixture", feature_version=3)
        self.assertEqual(model.predict(short).action, "convert")
        self.assertEqual(model.predict(long).action, "keep")
        self.assertEqual(model.predict(replace(short, field=replace(short.field, after=""))).action, "keep")

    def test_nonfinite_or_invalid_numeric_evidence_abstains_without_softmax(self) -> None:
        model = ContextModel({"bias": (0.0, 30.0, 0.0, 0.0)}, "context-v3-fixture", feature_version=3)
        variants = [replace(self.item, score_delta=math.nan), replace(self.item, source_group=2),
                    replace(self.item, source_group=True)]
        for score in (replace(SCORE, value=math.inf), replace(SCORE, gram_ratio=math.inf),
                      replace(SCORE, ngram_score=math.inf), replace(SCORE, invalid_ratio=math.inf),
                      replace(SCORE, raw_ngram_score=math.inf)):
            variants.append(replace(self.item, source_score=score))
        variants.extend((replace(self.item, source_score=replace(SCORE, frequency=-1)),
                         replace(self.item, source_score=replace(SCORE, frequency=cast(int, math.nan)))))
        variants.extend((replace(self.item, model_probability=math.inf), replace(self.item, model_threshold=math.inf),
                         replace(self.item, ortho_score=math.inf), replace(self.item, ortho_threshold=math.inf)))
        for item in variants:
            with self.subTest(item=item):
                with self.assertRaises(ValueError):
                    extract_action_features(item)
                prediction = model.predict(item)
                self.assertEqual(prediction.action, "suggest")
                self.assertFalse(prediction.supported)
        broken = ContextModel({"bias": (math.inf, 0.0, 0.0, 0.0)}, "context-v3-fixture", feature_version=3)
        self.assertEqual(broken.predict(self.item).action, "suggest")

    def test_application_or_one_reading_cannot_supply_language_support(self) -> None:
        for action_index in range(4):
            bias = tuple(20.0 if index == action_index else 0.0 for index in range(4))
            for partial in ({}, {"source:char:1:1:ф": ZERO}, {"target:char:1:1:a": ZERO},
                            {"source:char:1:1:^": ZERO, "target:char:1:1:$": ZERO}):
                weights = {"bias": bias, "app:editor": ZERO, **partial}
                model = ContextModel(weights, "context-v3-fixture", feature_version=3)
                prediction = model.predict(self.item)
                self.assertFalse(prediction.supported)
                self.assertFalse(model.supports_features(extract_action_features(self.item)))
                self.assertEqual(prediction.action, "wait" if action_index == 2 else "suggest")
        weights = supported_weights(self.item)
        weights["bias"] = (0.0, 1.0, 0.0, 0.0)
        prediction = ContextModel(weights, "context-v3-fixture", feature_version=3).predict(self.item)
        self.assertTrue(prediction.supported)
        self.assertEqual(prediction.action, "suggest")


class ShortUppercaseUnknownSourcePolicyTests(unittest.TestCase):
    def test_short_uppercase_unknown_source_may_only_be_suggested(self) -> None:
        field = FieldContext("Telegram", "1", "Мой сын обожает ", "!", "text")
        cases = (
            ("KFC", "ЛАС", False, "suggest"),      # brand acronym vs. dictionary reading: ambiguous
            ("EYX", "УНЧ", False, "suggest"),      # the same keys typed in the wrong layout: also only suggested
            ("ЫЙД", "SQL", False, "suggest"),      # both directions
            ("KFCN", "ЛАСТ", False, "convert"),    # four letters: outside the policy
            ("KFC", "ЛАС", True, "convert"),       # a known source reading is not the ambiguous class
            ("kfc", "лас", False, "convert"),      # lowercase tokens keep the learned decision
            ("Kfc", "Лас", False, "convert"),
        )
        for original, alternative, source_known, expected in cases:
            with self.subTest(original=original, source_known=source_known):
                group = 0 if original.isascii() else 1
                item = ContextEvidence(original, alternative, group, field, source_known=source_known, target_known=True)
                weights = supported_weights(item)
                weights["bias"] = (0.0, 20.0, 0.0, 0.0)
                model = ContextModel(weights, "context-v3-fixture", feature_version=3)
                features = extract_action_features(item)
                self.assertEqual(model.allows_automatic_conversion(features), expected == "convert")
                prediction = model.predict(item)
                self.assertTrue(prediction.supported)
                self.assertEqual(prediction.action, expected)
                self.assertGreater(prediction.probability, 0.985)
        keep = supported_weights(ContextEvidence("KFC", "ЛАС", 0, field, target_known=True))
        keep["bias"] = (20.0, 0.0, 0.0, 0.0)
        self.assertEqual(ContextModel(keep, "context-v3-fixture", feature_version=3).predict(
            ContextEvidence("KFC", "ЛАС", 0, field, target_known=True)).action, "keep")
        legacy = ContextModel({"bias": (0.0, 20.0, 0.0, 0.0)}, "context-v1-fixture")
        self.assertTrue(legacy.allows_automatic_conversion({"source:case:upper": 1.0, "source:known:0": 1.0, "length:3": 1.0}))

    def test_isolated_short_unknown_source_may_only_be_suggested(self) -> None:
        alone = FieldContext("Telegram", "1", "", "", "text")
        punctuation = FieldContext("Telegram", "1", "12 ", "!", "text")
        worded = FieldContext("Telegram", "1", "включи ", "", "text")
        cases = (
            ("зум", "pev", alone, False, False, "suggest"),        # chat word vs. a command typed in the wrong layout
            ("зцв", "pwd", alone, False, False, "suggest"),        # the same shape from the other side: also only suggested
            ("ghb", "при", alone, False, False, "suggest"),
            ("зум", "pev", punctuation, False, False, "suggest"),  # digits and punctuation are not words
            ("зум", "pev", worded, False, False, "convert"),       # a neighbouring word makes it a context decision
            ("зумм", "pevv", alone, False, False, "convert"),      # four letters: outside the short policy; unknown own reading licenses
            ("зум", "pev", alone, True, True, "convert"),          # a known source reading is not the ambiguous class; the baseline licenses
            ("зум", "pev", alone, True, False, "suggest"),         # ... but alone, between two dictionary readings, it needs a licence
        )
        for original, alternative, field, source_known, baseline, expected in cases:
            with self.subTest(original=original, before=field.before, source_known=source_known, baseline=baseline):
                group = 0 if original.isascii() else 1
                item = ContextEvidence(original, alternative, group, field, baseline_convert=baseline,
                                       source_known=source_known, target_known=True)
                weights = supported_weights(item)
                weights["bias"] = (0.0, 20.0, 0.0, 0.0)
                model = ContextModel(weights, "context-v3-fixture", feature_version=3)
                self.assertEqual(model.allows_automatic_conversion(extract_action_features(item)), expected == "convert")
                prediction = model.predict(item)
                self.assertTrue(prediction.supported)
                self.assertEqual(prediction.action, expected)

    def test_isolated_token_without_a_frozen_licence_may_only_be_suggested(self) -> None:
        alone = FieldContext("Telegram", "1", "", "", "text")
        worded = FieldContext("Telegram", "1", "в городе ", "", "text")
        unsupported = {"source_known": False, "target_known": False, "baseline_convert": False,
                       "ortho_score": -6.7, "ortho_threshold": 10.4}
        cases: tuple[tuple[str, str, FieldContext, dict[str, object], str], ...] = (
            ("афиши", "fabib", alone, {}, "suggest"),                            # no lexicon, no index, no frozen verdict
            ("k.andfaat", "люфтваффе", alone, {}, "suggest"),                    # both directions
            ("афиши", "fabib", alone, {"source_known": True}, "suggest"),        # a known own reading changes nothing here
            ("Еще", "Tot", alone, {"source_known": True, "target_known": True}, "suggest"),  # two dictionary words: no licence
            ("hfyj", "рано", alone, {"target_known": True}, "convert"),         # a dictionary reading against an unknown own one
            ("афиши", "fabib", alone, {"baseline_convert": True}, "convert"),   # the detector's own verdict licenses
            ("афиши", "fabib", alone, {"ortho_score": 12.0}, "convert"),        # orthotactics above its own threshold licenses
            ("афиши", "fabib", alone, {"ortho_score": 10.4}, "suggest"),        # exactly at the threshold: no margin
            ("афиши", "fabib", alone, {"ortho_score": None, "ortho_threshold": None}, "suggest"),  # unsupported: no licence
            ("афиши", "fabib", alone, {"target_known": True}, "convert"),       # a dictionary reading licenses
            ("афиши", "fabib", alone, {"target_identifier": True}, "convert"),  # a command name licenses an unknown own reading
            ("зум", "pev", alone, {"source_known": True, "target_identifier": True}, "suggest"),  # ... but not a known one
            ("афиши", "fabib", worded, {}, "convert"),                          # a neighbouring word: a context decision
        )
        for original, alternative, field, change, expected in cases:
            with self.subTest(original=original, before=field.before, change=change):
                group = 0 if original.isascii() else 1
                item = ContextEvidence(original, alternative, group, field, **{**unsupported, **change})  # type: ignore[arg-type]
                weights = supported_weights(item)
                weights["bias"] = (0.0, 20.0, 0.0, 0.0)
                model = ContextModel(weights, "context-v3-fixture", feature_version=3)
                self.assertEqual(model.allows_automatic_conversion(extract_action_features(item)), expected == "convert")
                prediction = model.predict(item)
                self.assertTrue(prediction.supported)
                self.assertEqual(prediction.action, expected)
        legacy = ContextModel({"bias": (0.0, 20.0, 0.0, 0.0)}, "context-v1-fixture")
        self.assertTrue(legacy.allows_automatic_conversion({"target:known:0": 1.0, "target:identifier:0": 1.0, "baseline:0": 1.0,
                                                            "before:script:none:direction:1": 1.0, "after:script:none:direction:1": 1.0}))


class ContextActionPolicyTests(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.path = Path(directory.name) / "model.json"
        self.ortho = OrthoModel(2, {}, {}, {}, "ortho-v1-fixture", {"en": 1.0, "ru": 1.0}, 3)
        en = LanguageModel("en_US", {"hello": 20}, "test", enable_spellcheck=False)
        ru = LanguageModel("ru_RU", {"привет": 20}, "test", enable_spellcheck=False)
        self.detector = LanguageDetector({0: en, 1: ru})
        self.baseline = DetectionDecision(False, "ghbdtn", "ghbdtn", 0, 0, 0, "test", SCORE, SCORE, 0.8, 0.9)
        self.field = FieldContext("Editor", "1", "", "", "text")
        with patch.object(ContextModel, "try_load", return_value=(None, "fixture")), patch.object(ContextPolicy, "_shared_ortho", (None, "fixture")):
            self.policy = ContextPolicy()

    def test_shared_evidence_uses_each_actual_scorer_and_direction(self) -> None:
        outcome = OrthoScore(2, 3, 5, -10, -5, True)
        for group, original, alternative in ((0, "ghbdtn", "привет"), (1, "руддщ", "hello")):
            baseline = replace(self.baseline, original=original, source_group=group)
            with patch.object(self.ortho, "score", return_value=outcome) as score:
                item = evidence_for_decision(baseline, alternative, 1 - group, self.detector, self.field, "enter", literal_tail="...", boundary_text="\n", ortho=self.ortho)
            self.assertEqual(item.source_score, self.detector.models[group].score(original))
            self.assertEqual(item.target_score, self.detector.models[1 - group].score(alternative))
            self.assertTrue(item.target_known)
            self.assertEqual((item.model_probability, item.model_threshold, item.literal_tail, item.ortho_score, item.ortho_threshold), (0.8, 0.9, "...", 5, 1.0))
            self.assertEqual(item.boundary_text, "\n")
            score.assert_called_once_with(OrthoEvidence("ghbdtn" if group == 0 else "hello", "lower", "en" if group == 0 else "ru"))
        with patch.object(self.ortho, "score", return_value=replace(outcome, supported=False)):
            item = evidence_for_decision(self.baseline, "привет", 1, self.detector, self.field, "space", ortho=self.ortho)
        self.assertIsNone(item.ortho_score)
        self.detector.models[2] = self.detector.models[0]
        with patch.object(self.ortho, "score") as score:
            evidence_for_decision(replace(self.baseline, source_group=2), "привет", 1, self.detector, self.field, "space", ortho=self.ortho)
        score.assert_not_called()

    def test_v3_keep_is_not_overridden_by_orthotactic_or_automatic_short_word_rules(self) -> None:
        item = evidence_for_decision(self.baseline, "привет", 1, self.detector, self.field, "space")
        weights = supported_weights(item)
        weights["bias"] = (20.0, 0.0, 0.0, 0.0)
        self.policy.model = ContextModel(weights, "context-v3-fixture", feature_version=3)
        self.policy.ortho = self.ortho
        with patch.object(self.ortho, "score", return_value=OrthoScore(50, 0, 50, -60, -10, True)), patch.object(self.policy, "_licensed") as licensed, patch("keyswitch.context_policy.is_short_word_override", return_value=True):
            result = self.policy.decide(self.baseline, "привет", 1, self.detector, "space", "assist", field_override=self.field, literal_tail=",")
        self.assertEqual(result.prediction.action if result.prediction else None, "keep")
        self.assertFalse(result.decision.should_convert)
        self.assertTrue(result.policy_applied)
        licensed.assert_not_called()

    def test_v3_unsupported_baseline_conversion_abstains_and_v2_still_uses_licensing(self) -> None:
        self.policy.model = ContextModel({"bias": (20.0, 0.0, 0.0, 0.0), "app:editor": ZERO}, "context-v3-fixture", feature_version=3)
        result = self.policy.decide(replace(self.baseline, should_convert=True), "привет", 1, self.detector, "space", "assist", field_override=self.field)
        self.assertFalse(result.decision.should_convert)
        self.assertEqual(result.prediction.action if result.prediction else None, "suggest")
        self.policy.model = ContextModel({"bias": (20.0, 0.0, 0.0, 0.0), "app:editor": ZERO}, "context-v1-fixture")
        with patch("keyswitch.context_policy.evidence_for_decision", wraps=evidence_for_decision) as build, patch.object(self.policy, "_licensed", wraps=self.policy._licensed) as licensed:
            self.policy.decide(self.baseline, "привет", 1, self.detector, "space", "assist", field_override=self.field)
        self.assertIsNone(build.call_args.kwargs["ortho"])
        licensed.assert_called_once()

    def test_v2_curated_rule_remains_authoritative_only_for_the_old_policy(self) -> None:
        baseline = replace(self.baseline, should_convert=True)
        item = evidence_for_decision(baseline, "привет", 1, self.detector, self.field, "space")
        for feature_version, expected in ((2, True), (3, False)):
            weights = supported_weights(item)
            weights.update({"bias": (20.0, 0.0, 0.0, 0.0), "app:editor": ZERO})
            self.policy.model = ContextModel(weights, "fixture", feature_version=feature_version)
            with patch("keyswitch.context_policy.is_short_word_override", return_value=True):
                result = self.policy.decide(baseline, "привет", 1, self.detector, "space", "assist", field_override=self.field)
            self.assertEqual(result.decision.should_convert, expected)

    def test_artifact_feature_versions_require_their_own_identity(self) -> None:
        weights = {"bias": [0.0, 8.0, 0.0, 0.0]}
        digest = hashlib.sha256(json.dumps(weights, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()
        payload = {"feature_version": 3, "actions": list(ACTIONS), "weights": weights,
                   "weights_sha256": digest, "version": "context-v3-fixture", "conversion_threshold": 0.985}
        for feature_version, version in ((2, "context-v1-fixture"), (3, "context-v3-fixture")):
            self.path.write_text(json.dumps({**payload, "feature_version": feature_version, "version": version}))
            self.assertEqual(ContextModel.load(self.path).feature_version, feature_version)
        for changes in ({"feature_version": True}, {"feature_version": 3.0}, {"feature_version": 4},
                        {"version": "context-v1-fixture"}, {"feature_version": 2},
                        {"conversion_threshold": math.nan}, {"conversion_threshold": math.inf}):
            self.path.write_text(json.dumps({**payload, **changes}))
            with self.assertRaises(ValueError):
                ContextModel.load(self.path)
        for feature_version in (0, 4, True):
            with self.assertRaises(ValueError):
                ContextModel({}, "fixture", feature_version=feature_version)
