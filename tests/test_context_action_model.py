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

from keyswitch.context_action_features import (
    CHARACTER_FEATURE_WEIGHT_CAP, ORTHO_SCORE_CLAMP_BOUND, SHORT_TEXT_FEATURE_CHARACTERS, WHITESPACE_MAX_CHARACTERS,
    extract_action_features,
)
from keyswitch.context_model import ACTIONS, FEATURE_VERSION, ContextEvidence, ContextModel, ContextPrediction, extract_context_features
from keyswitch.short_words import ISOLATED_SHORT_WORD_REASON
from keyswitch.context_policy import ContextPolicy, evidence_for_decision
from keyswitch.detector import DetectionDecision, LanguageDetector
from keyswitch.input_context import FieldContext
from keyswitch.language_model import LanguageModel, WordScore
from keyswitch.ortho_model import OrthoEvidence, OrthoModel, OrthoScore
import test_input_sequence_matrix as sequences


ZERO = (0.0, 0.0, 0.0, 0.0)
SCORE = WordScore(2.0, False, 20, 0.8, False, False, 3.0, 0.0, -2.0)
# context_model.py accepts this experimental feature schema (frozen, pending
# reseal) but names only the legacy one (FEATURE_VERSION); every fixture here
# that opts into the v3 features shares this one name for the other one.
ACTION_FEATURE_VERSION = 3
# A bias large enough that its class wins the softmax regardless of whatever
# else is in the weights - the "make this action win" fixture trick.
DOMINANT_BIAS = 20.0
# A smaller bias used the same way against a fixture with no competing
# features, where it dominates without needing DOMINANT_BIAS's margin.
SAMPLE_BIAS_WEIGHT = 8.0
# ContextModel.__init__'s default conversion_threshold is hardcoded (0.985)
# with no name of its own; mirrored here for the calibration behaviour it
# drives and the artifact fixture that pins it.
DEFAULT_CONVERSION_THRESHOLD = 0.985


def supported_weights(item: ContextEvidence) -> dict[str, tuple[float, ...]]:
    return {
        name: ZERO for name in extract_action_features(item)
        if name.startswith(("source:char:", "target:char:"))
    }


class RecordModel(ContextModel):
    def __init__(self) -> None:
        super().__init__({"bias": (DOMINANT_BIAS, 0.0, 0.0, 0.0)}, "context-v3-record", feature_version=ACTION_FEATURE_VERSION)
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
                current.physical("yt ")
                self.assertIsNotNone(current.engine._context_waiting)
                predict.return_value = ContextPrediction("convert", 1.0, (0.0, 1.0, 0.0, 0.0), model.version, True)
                current.physical("'njuj\u00a0")
            self.assertEqual([(item.original, item.boundary_text, item.field.after) for item in model.items],
                             [("yt", " ", ""), ("'njuj", "\u00a0", ""), ("yt", " ", "этого")])
            self.assertEqual(current.backend.text, "не этого\u00a0")
            self.assertEqual(len(current.backend.injections), 1)


class ContextActionFeatureTests(unittest.TestCase):
    # Fixture values a single test pins, named here rather than inline:
    # check_named_values.py only recognises a name assigned at module or
    # class level, not inside a method body.
    TARGET_KNOWN_FREQUENCY = 100
    RICH_MODEL_PROBABILITY = 0.6
    RICH_MODEL_THRESHOLD = 0.99
    RICH_ORTHO_SCORE = 100.0
    OVERSIZED_TEXT_LENGTH = 100
    SOURCE_NGRAM_SCORE_FEATURE = 0.3
    STRONG_FEATURE_WEIGHT = 30.0
    NGRAM_SCORE_PROBE = 5.0
    MODEL_PROBABILITY_FIXTURE = 0.8
    MODEL_THRESHOLD_FIXTURE = 0.9
    ORTHO_SCORE_TEST_VALUE = 20
    ORTHO_THRESHOLD_TEST_VALUE = 4
    BEFORE_WHITESPACE_COUNT = 3
    TAIL_DOT_COUNT = 3
    EXPECTED_BASELINE_MARGIN = -0.1
    LONG_TEXT_LENGTH = 5000
    OVERSIZED_FIELD_LENGTH = 500
    BEFORE_TEXT_BLOCK_LENGTH = 300
    BEFORE_TEXT_TRAILING_SPACES = 50
    EXTREME_MAGNITUDE = 1000
    OUT_OF_RANGE_RATIO = 10
    EXTREME_INVALID_RATIO_MAGNITUDE = 3
    EXTREME_RAW_NGRAM_SCORE = 2
    EXTREME_FREQUENCY = 10**100
    SUBTEST_LABEL_LENGTH = 10
    EXPECTED_FEATURE_COUNT_CEILING = 3000
    EXPECTED_FEATURE_NAME_LENGTH_CEILING = 160
    OVERSIZED_TEXT_VARIANT_INDEX = 2
    ORTHO_EVIDENCE_SCORE = 20.0
    ORTHO_EVIDENCE_THRESHOLD = 4.0
    ORTHO_EVIDENCE_BASE_BIAS = 2.0
    ORTHO_SIDE_THRESHOLD = 10.0
    ORTHO_SIDE_SCORE = 12.0
    ORTHO_SIDE_BASE_BIAS = 4.0
    ORTHO_SIDE_MARGIN_PROBES = (0.001, 2.0, 32.0)
    ORTHO_SIDE_BELOW_SCORE = 9.0
    SHORT_WORD_BASE_BIAS = 5.0
    INVALID_SOURCE_GROUP = 2

    def setUp(self) -> None:
        self.item = ContextEvidence(
            "флуд", "akel", 1, FieldContext("Editor", "1", "обсудим ", "", "text"),
            source_score=SCORE,
            target_score=replace(SCORE, known=True, exact=True, frequency=self.TARGET_KNOWN_FREQUENCY),
            target_known=True,
        )

    def test_v2_feature_golden_is_unchanged_and_new_evidence_is_ignored(self) -> None:
        item = ContextEvidence("gh", "пр", 0, FieldContext("Telegram", "1", "я ", "", "text"))
        features = extract_context_features(item)
        digest = hashlib.sha256(json.dumps(features, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode()).hexdigest()
        self.assertEqual(digest, "1eabba3a049f1d3f6552b04c7b98bcbb454fd03f662f8925ee7a43363c4c928b")
        model = ContextModel({"bias": (0.0, SAMPLE_BIAS_WEIGHT, 0.0, 0.0), "app:telegram": ZERO}, "context-v1-fixture")
        rich = replace(item, source_score=SCORE, target_score=SCORE, model_probability=self.RICH_MODEL_PROBABILITY,
                       model_threshold=self.RICH_MODEL_THRESHOLD, literal_tail="...", ortho_score=self.RICH_ORTHO_SCORE,
                       ortho_threshold=1.0, boundary_text="\u00a0")
        self.assertEqual(model.predict(item), model.predict(rich))
        self.assertEqual(model.predict(rich).action, "convert")
        self.assertEqual(model.feature_version, FEATURE_VERSION)

    def test_current_boundary_is_distinct_from_neighbor_whitespace_and_literal_tail(self) -> None:
        for glyph in (" ", "\u00a0", "\t", "!", ""):
            with self.subTest(glyph=repr(glyph)):
                features = extract_action_features(replace(self.item, boundary_text=glyph))
                self.assertEqual(features.get("boundary:length", 0.0), len(glyph) / SHORT_TEXT_FEATURE_CHARACTERS)
                self.assertEqual(features.get("boundary:isspace", 0.0), float(glyph.isspace()))
                for candidate in (" ", "\u00a0", "\t", "!"):
                    self.assertEqual(
                        features.get(f"boundary:char:{ord(candidate):04x}", 0.0),
                        float(glyph == candidate) / SHORT_TEXT_FEATURE_CHARACTERS,
                    )
        oversized = extract_action_features(replace(self.item, boundary_text="\t" * self.OVERSIZED_TEXT_LENGTH))
        self.assertEqual(oversized["boundary:length"], 1.0)
        self.assertEqual(oversized["boundary:overflow"], 1.0)
        self.assertEqual(oversized["boundary:char:0009"], 1.0)

    def test_unknown_source_has_its_own_lexical_evidence_despite_a_known_alternative(self) -> None:
        features = extract_action_features(self.item)
        self.assertEqual(features["source:known:0"], 1.0)
        self.assertEqual(features["target:known:1"], 1.0)
        self.assertEqual(features["source:ngram_score"], self.SOURCE_NGRAM_SCORE_FEATURE)
        self.assertEqual(features["target:exact:1"], 1.0)
        self.assertGreater(features["target:logfrequency"], features["source:logfrequency"])
        weights = supported_weights(self.item)
        weights.update({"source:ngram_score": (self.STRONG_FEATURE_WEIGHT, 0.0, 0.0, 0.0), "target:known:1": (0.0, 1.0, 0.0, 0.0)})
        model = ContextModel(weights, "context-v3-fixture", feature_version=ACTION_FEATURE_VERSION)
        self.assertEqual(model.predict(self.item).action, "keep")

    def test_source_and_target_polarity_can_reverse_the_action_without_changing_the_words(self) -> None:
        weights = supported_weights(self.item)
        weights.update({"source:ngram_score": (self.STRONG_FEATURE_WEIGHT, 0.0, 0.0, 0.0),
                        "target:ngram_score": (0.0, self.STRONG_FEATURE_WEIGHT, 0.0, 0.0)})
        model = ContextModel(weights, "context-v3-fixture", feature_version=ACTION_FEATURE_VERSION)
        source = replace(SCORE, ngram_score=self.NGRAM_SCORE_PROBE)
        target = replace(SCORE, ngram_score=-self.NGRAM_SCORE_PROBE)
        self.assertEqual(model.predict(replace(self.item, source_score=source, target_score=target)).action, "keep")
        self.assertEqual(model.predict(replace(self.item, source_score=target, target_score=source)).action, "convert")
        identical = extract_action_features(replace(self.item, original="abc", alternative="abc"))
        self.assertGreater(identical["source:char:1:3:abc"], 0)
        self.assertGreater(identical["target:char:1:3:abc"], 0)

    def test_case_whitespace_neighbors_tail_and_numeric_margins_are_distinct(self) -> None:
        item = replace(self.item, original="Флуд", alternative="AkEl", literal_tail=",...",
                       field=replace(self.item.field, before="one  \u00a0", after="\t два"),
                       model_probability=self.MODEL_PROBABILITY_FIXTURE, model_threshold=self.MODEL_THRESHOLD_FIXTURE,
                       ortho_score=self.ORTHO_SCORE_TEST_VALUE, ortho_threshold=self.ORTHO_THRESHOLD_TEST_VALUE)
        features = extract_action_features(item)
        self.assertIn("source:case:title", features)
        self.assertIn("target:case:mixed", features)
        self.assertEqual(features["before:space_count"], self.BEFORE_WHITESPACE_COUNT / WHITESPACE_MAX_CHARACTERS)
        self.assertEqual(features["before:space:00a0"], 1 / WHITESPACE_MAX_CHARACTERS)
        self.assertEqual(features["after:space:0009"], 1 / WHITESPACE_MAX_CHARACTERS)
        self.assertIn("before:char:1:3:one", features)
        self.assertIn("after:char:1:3:два", features)
        self.assertEqual(features["tail:char:002e"], self.TAIL_DOT_COUNT / SHORT_TEXT_FEATURE_CHARACTERS)
        self.assertAlmostEqual(features["baseline:margin"], self.EXPECTED_BASELINE_MARGIN)
        self.assertEqual(
            features["ortho:margin"],
            (self.ORTHO_SCORE_TEST_VALUE - self.ORTHO_THRESHOLD_TEST_VALUE) / ORTHO_SCORE_CLAMP_BOUND,
        )
        self.assertNotEqual(features, extract_action_features(self.item))

    def test_extreme_and_empty_text_produce_bounded_finite_features(self) -> None:
        variants = (
            replace(self.item, original="", alternative="", source_score=None, target_score=None, field=FieldContext("", "1")),
            replace(self.item, original="1", alternative="!", literal_tail="." * self.OVERSIZED_TEXT_LENGTH),
            replace(self.item, original="A" * self.LONG_TEXT_LENGTH, alternative="Аb" * self.LONG_TEXT_LENGTH,
                    field=FieldContext("EDITOR" * self.OVERSIZED_FIELD_LENGTH, "1",
                                       "a" * self.BEFORE_TEXT_BLOCK_LENGTH + " " + "a" * self.BEFORE_TEXT_BLOCK_LENGTH
                                       + " " * self.BEFORE_TEXT_TRAILING_SPACES,
                                       "\n" * self.OVERSIZED_FIELD_LENGTH)),
            replace(self.item, original="ё", alternative="a", field=FieldContext("", "1", "🙂", "xя"),
                    source_score=replace(SCORE, value=-self.EXTREME_MAGNITUDE, gram_ratio=self.OUT_OF_RANGE_RATIO,
                                          ngram_score=self.EXTREME_MAGNITUDE,
                                          invalid_ratio=-self.EXTREME_INVALID_RATIO_MAGNITUDE,
                                          raw_ngram_score=self.EXTREME_RAW_NGRAM_SCORE, frequency=self.EXTREME_FREQUENCY),
                    model_probability=self.EXTREME_MAGNITUDE, model_threshold=-self.EXTREME_MAGNITUDE,
                    ortho_score=-self.EXTREME_MAGNITUDE, ortho_threshold=self.EXTREME_MAGNITUDE),
        )
        for item in variants:
            with self.subTest(original=item.original[:self.SUBTEST_LABEL_LENGTH]):
                features = extract_action_features(item)
                self.assertLess(len(features), self.EXPECTED_FEATURE_COUNT_CEILING)
                self.assertTrue(all(
                    math.isfinite(value) and abs(value) <= CHARACTER_FEATURE_WEIGHT_CAP for value in features.values()
                ))
                self.assertTrue(all(len(name) <= self.EXPECTED_FEATURE_NAME_LENGTH_CEILING for name in features))
        self.assertIn("source:case:none", extract_action_features(variants[0]))
        self.assertIn("source:case:upper", extract_action_features(variants[self.OVERSIZED_TEXT_VARIANT_INDEX]))
        self.assertIn("target:script:mixed", extract_action_features(variants[self.OVERSIZED_TEXT_VARIANT_INDEX]))

    def test_orthographic_evidence_is_conditioned_on_language_and_source_support(self) -> None:
        item = replace(self.item, ortho_score=self.ORTHO_EVIDENCE_SCORE, ortho_threshold=self.ORTHO_EVIDENCE_THRESHOLD,
                       source_known=False, field=FieldContext("Editor", "1", "обсуждаем "))
        name = "ortho:positive:direction:1:source_known:0:context:ru"
        variants = (replace(item, source_known=True), replace(item, source_group=0),
                    replace(item, field=FieldContext("Editor", "1", "discuss ")),
                    replace(item, ortho_score=0.0))
        weights = supported_weights(item)
        for changed in variants:
            weights.update(supported_weights(changed))
        weights.update({"bias": (self.ORTHO_EVIDENCE_BASE_BIAS, 0.0, 0.0, 0.0), name: (0.0, ORTHO_SCORE_CLAMP_BOUND, 0.0, 0.0)})
        model = ContextModel(weights, "context-v3-fixture", feature_version=ACTION_FEATURE_VERSION)
        self.assertEqual(model.predict(item).action, "convert")
        for changed in variants:
            with self.subTest(item=changed):
                self.assertEqual(model.predict(changed).action, "keep")
        negative = extract_action_features(replace(item, ortho_score=0.0))
        self.assertEqual(negative["ortho:negative:direction:1"], -self.ORTHO_EVIDENCE_THRESHOLD / ORTHO_SCORE_CLAMP_BOUND)
        self.assertNotIn(name, negative)

    def test_orthographic_threshold_side_is_learned_and_does_not_license_conversion(self) -> None:
        item = replace(self.item, original="рещз", alternative="htop", source_known=False,
                       ortho_score=self.ORTHO_SIDE_SCORE, ortho_threshold=self.ORTHO_SIDE_THRESHOLD)
        name = "ortho:side:positive:direction:1:source_known:0:case:lower"
        weights = supported_weights(item)
        weights.update({"bias": (self.ORTHO_SIDE_BASE_BIAS, 0.0, 0.0, 0.0), name: (0.0, DOMINANT_BIAS, 0.0, 0.0)})
        model = ContextModel(weights, "context-v3-fixture", feature_version=ACTION_FEATURE_VERSION)
        # A coefficient can express the same policy close to and far from the
        # pre-existing learned threshold; the raw linear margin remains separate.
        for margin in self.ORTHO_SIDE_MARGIN_PROBES:
            with self.subTest(margin=margin):
                self.assertEqual(model.predict(replace(item, ortho_score=self.ORTHO_SIDE_THRESHOLD + margin)).action, "convert")
        for changed in (replace(item, ortho_score=self.ORTHO_SIDE_THRESHOLD), replace(item, ortho_score=self.ORTHO_SIDE_BELOW_SCORE),
                        replace(item, source_known=True), replace(item, original="РЕЩЗ", alternative="HTOP")):
            with self.subTest(item=changed):
                self.assertEqual(model.predict(changed).action, "keep")
        weights[name] = (DOMINANT_BIAS, 0.0, 0.0, 0.0)
        keep = ContextModel(weights, "context-v3-fixture", feature_version=ACTION_FEATURE_VERSION)
        self.assertEqual(keep.predict(item).action, "keep")

    def test_context_script_can_resolve_short_words_without_rewriting_long_insertions(self) -> None:
        short = ContextEvidence("y", "н", 0, FieldContext("Editor", "1", "", "пример"))
        long = replace(short, original="pipeline", alternative="зшзудшту")
        weights = supported_weights(short) | supported_weights(long)
        weights.update({"bias": (self.SHORT_WORD_BASE_BIAS, 0.0, 0.0, 0.0),
                        "after:script:ru:direction:0:length:1": (0.0, DOMINANT_BIAS, 0.0, 0.0)})
        model = ContextModel(weights, "context-v3-fixture", feature_version=ACTION_FEATURE_VERSION)
        self.assertEqual(model.predict(short).action, "convert")
        self.assertEqual(model.predict(long).action, "keep")
        self.assertEqual(model.predict(replace(short, field=replace(short.field, after=""))).action, "keep")

    def test_nonfinite_or_invalid_numeric_evidence_abstains_without_softmax(self) -> None:
        model = ContextModel({"bias": (0.0, self.STRONG_FEATURE_WEIGHT, 0.0, 0.0)}, "context-v3-fixture",
                              feature_version=ACTION_FEATURE_VERSION)
        variants = [replace(self.item, score_delta=math.nan), replace(self.item, source_group=self.INVALID_SOURCE_GROUP),
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
        broken = ContextModel({"bias": (math.inf, 0.0, 0.0, 0.0)}, "context-v3-fixture", feature_version=ACTION_FEATURE_VERSION)
        self.assertEqual(broken.predict(self.item).action, "suggest")

    def test_application_or_one_reading_cannot_supply_language_support(self) -> None:
        for action_index in range(len(ACTIONS)):
            bias = tuple(DOMINANT_BIAS if index == action_index else 0.0 for index in range(len(ACTIONS)))
            for partial in ({}, {"source:char:1:1:ф": ZERO}, {"target:char:1:1:a": ZERO},
                            {"source:char:1:1:^": ZERO, "target:char:1:1:$": ZERO}):
                weights = {"bias": bias, "app:editor": ZERO, **partial}
                model = ContextModel(weights, "context-v3-fixture", feature_version=ACTION_FEATURE_VERSION)
                prediction = model.predict(self.item)
                self.assertFalse(prediction.supported)
                self.assertFalse(model.supports_features(extract_action_features(self.item)))
                self.assertEqual(prediction.action, "wait" if action_index == ACTIONS.index("wait") else "suggest")
        weights = supported_weights(self.item)
        weights["bias"] = (0.0, 1.0, 0.0, 0.0)
        prediction = ContextModel(weights, "context-v3-fixture", feature_version=ACTION_FEATURE_VERSION).predict(self.item)
        self.assertTrue(prediction.supported)
        self.assertEqual(prediction.action, "suggest")


class ModelVerdictStandsTests(unittest.TestCase):
    """The model decides; nothing in this layer overrules it for a whole class of input.

    Until 17.09.2026 two class-wide vetoes lived here - a licence for isolated tokens and a
    ban on short unknown readings standing alone or written in capitals. They overruled the
    model on the very input it was trained to tell apart, cost 14 of 300 restorations on
    chat-like first words and prevented no corruption
    (.t/reliable-release-2026-09-12/SHORT-ISOLATED-CURRICULUM.md). What may still refuse a
    conversion is an exception the user can see: an excluded word or application, their own
    settings, a rule they taught the engine - and those live in the engine, not in the model.
    """

    def test_no_class_of_input_is_refused_by_this_layer(self) -> None:
        field = FieldContext("Telegram", "1", "", "", "text")
        cases = (
            ("KFC", "ЛАС", False, True, False),      # uppercase, isolated
            ("зум", "pev", True, False, True),       # a command name as the other reading
            ("афиши", "fabib", False, False, False),  # neither reading known, no verdict
            ("Нщг", "You", False, True, False),      # the chat-like first word
        )
        for original, alternative, source_known, target_known, target_identifier in cases:
            with self.subTest(original=original):
                group = 0 if original.isascii() else 1
                item = ContextEvidence(original, alternative, group, field, source_known=source_known,
                                       target_known=target_known, target_identifier=target_identifier)
                weights = supported_weights(item)
                weights["bias"] = (0.0, DOMINANT_BIAS, 0.0, 0.0)
                model = ContextModel(weights, "context-v3-fixture", feature_version=ACTION_FEATURE_VERSION)
                self.assertTrue(model.allows_automatic_conversion(extract_action_features(item)))
                prediction = model.predict(item)
                self.assertEqual((prediction.action, prediction.supported), ("convert", True))

    def test_a_refusal_from_the_conversion_policy_is_honoured_when_serving(self) -> None:
        """The seam is empty today, and serving must still obey it if it is ever filled.

        Calibration counts conversions through the same method, so a refusal that the
        served prediction ignored would make the threshold describe a model nobody runs -
        the corpus-against-serving gap this project has already paid for once.
        """

        field = FieldContext("Telegram", "1", "", "", "text")
        item = ContextEvidence("Нщг", "You", 0, field, target_known=True)
        weights = supported_weights(item)
        weights["bias"] = (0.0, DOMINANT_BIAS, 0.0, 0.0)
        model = ContextModel(weights, "context-v3-fixture", feature_version=ACTION_FEATURE_VERSION)
        self.assertEqual(model.predict(item).action, "convert")
        with patch.object(ContextModel, "allows_automatic_conversion", return_value=False):
            self.assertEqual(model.predict(item).action, "suggest")

    def test_the_model_still_decides_against_a_conversion_on_its_own(self) -> None:
        """Refusing is the model's own verdict, not a rule applied on top of it."""
        field = FieldContext("Telegram", "1", "", "", "text")
        item = ContextEvidence("KFC", "ЛАС", 0, field, target_known=True)
        weights = supported_weights(item)
        weights["bias"] = (DOMINANT_BIAS, 0.0, 0.0, 0.0)
        self.assertEqual(ContextModel(weights, "context-v3-fixture", feature_version=ACTION_FEATURE_VERSION).predict(item).action, "keep")

    def test_an_underconfident_conversion_is_offered_rather_than_made(self) -> None:
        """The threshold is the model's own calibration, not a class-wide ban."""
        field = FieldContext("Telegram", "1", "", "", "text")
        item = ContextEvidence("hfyj", "рано", 0, field, target_known=True)
        weights = supported_weights(item)
        weights["bias"] = (0.0, 1.0, 0.0, 0.0)
        prediction = ContextModel(weights, "context-v3-fixture", feature_version=ACTION_FEATURE_VERSION).predict(item)
        self.assertEqual(prediction.action, "suggest")
        self.assertLess(prediction.probability, DEFAULT_CONVERSION_THRESHOLD)


class ArtifactIdentityTests(unittest.TestCase):
    # One past the only two valid feature versions (FEATURE_VERSION and
    # ACTION_FEATURE_VERSION); used to probe both the payload and the
    # constructor's own validation.
    INVALID_FEATURE_VERSION = 4

    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.path = Path(directory.name) / "context-action.json"

    def test_artifact_feature_versions_require_their_own_identity(self) -> None:
        weights = {"bias": [0.0, SAMPLE_BIAS_WEIGHT, 0.0, 0.0]}
        digest = hashlib.sha256(json.dumps(weights, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()
        payload = {"feature_version": ACTION_FEATURE_VERSION, "actions": list(ACTIONS), "weights": weights,
                   "weights_sha256": digest, "version": "context-v3-fixture", "conversion_threshold": DEFAULT_CONVERSION_THRESHOLD}
        for feature_version, version in ((FEATURE_VERSION, "context-v1-fixture"), (ACTION_FEATURE_VERSION, "context-v3-fixture")):
            self.path.write_text(json.dumps({**payload, "feature_version": feature_version, "version": version}))
            self.assertEqual(ContextModel.load(self.path).feature_version, feature_version)
        for changes in ({"feature_version": True}, {"feature_version": float(ACTION_FEATURE_VERSION)},
                        {"feature_version": self.INVALID_FEATURE_VERSION},
                        {"version": "context-v1-fixture"}, {"feature_version": FEATURE_VERSION},
                        {"conversion_threshold": math.nan}, {"conversion_threshold": math.inf}):
            self.path.write_text(json.dumps({**payload, **changes}))
            with self.assertRaises(ValueError):
                ContextModel.load(self.path)
        for feature_version in (0, self.INVALID_FEATURE_VERSION, True):
            with self.assertRaises(ValueError):
                ContextModel({}, "fixture", feature_version=feature_version)
