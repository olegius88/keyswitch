"""Prefix extraction and independently validated model loading/inference."""
from __future__ import annotations

import hashlib
import json
import math
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path
from unittest.mock import patch

from keyswitch.context_model import ACTIONS, ContextModel
from keyswitch.early_switch import PrefixIndex
from keyswitch.input_context import FieldContext
from keyswitch.language_model import LanguageModel
from keyswitch.prefix_model import PrefixInput, PrefixModel, features
from keyswitch.prefix_schema import (
    CURRENT_PREFIX_FEATURE_VERSION, MAX_PREFIX_FEATURE_NAME_CHARACTERS, MAX_PREFIX_MODEL_BYTES,
    MAX_PREFIX_WEIGHTS, OBSERVED_PREFIX_MAX_CHARACTERS, VERSION_HASH_CHARACTERS,
    VersionedPrefixModel, features_for_version,
)

HIGH_WORD_FREQUENCY = 100
LOW_WORD_FREQUENCY = 50
# Bias magnitudes for the fixture ContextModel weight vectors below: enough to
# force a single action to win regardless of the other (zero) weights.
BASE_FIXTURE_BIAS = 10.0
UNCERTAIN_BIAS = 2.0
DOMINANT_BIAS = 20.0
FIXTURE_CONVERSION_THRESHOLD = 0.999
BELOW_MINIMUM_THRESHOLD = 0.9
ABOVE_MAXIMUM_THRESHOLD = 1.1
UNSUPPORTED_FEATURE_VERSION = 3
PREFIX_CHAR_FEATURE_VALUE_CEILING = 2
SUBTEST_PAYLOAD_PREVIEW_CHARACTERS = 150
OVER_LENGTH_CAP_CHARACTERS = 30
LONG_APPLICATION_NAME_REPEATS = 100
# Trailing filler past OBSERVED_PREFIX_MAX_CHARACTERS: must never surface in features.
TRAILING_FILLER_CHARACTERS = 100
# Mirrors the frozen src/keyswitch/prefix_model.py before[-512:] context window.
FIELD_BEFORE_WINDOW_CHARACTERS = 512
SHA256_HEX_LENGTH = 64


class PrefixModelTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.path = Path(temporary.name) / "prefix.json"
        self.models = {
            group: LanguageModel(locale, words, "fixture", enable_spellcheck=False)
            for group, locale, words in ((0, "en_US", {"hello": HIGH_WORD_FREQUENCY, "world": LOW_WORD_FREQUENCY}),
                                         (1, "ru_RU", {"привет": HIGH_WORD_FREQUENCY, "привод": LOW_WORD_FREQUENCY}))
        }
        self.indexes = {group: PrefixIndex(model.frequencies, model.frequencies) for group, model in self.models.items()}
        self.item = PrefixInput("ghbd", "прив", 0, FieldContext("Code", "1", "// проверим ", role="code"))
        PrefixModel.default.cache_clear()
        self.addCleanup(PrefixModel.default.cache_clear)

    @staticmethod
    def payload() -> dict[str, object]:
        weights = {"bias": [0.0, BASE_FIXTURE_BIAS, 0.0, 0.0]}
        digest = hashlib.sha256(json.dumps(weights, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()
        return {"kind": "keyswitch.prefix-policy", "feature_version": 1, "actions": list(ACTIONS),
                "version": "prefix-v1-" + digest[:VERSION_HASH_CHARACTERS], "conversion_threshold": FIXTURE_CONVERSION_THRESHOLD,
                "weights": weights, "weights_sha256": digest}

    def test_load_predict_and_cache_without_completed_word_extractor(self) -> None:
        self.path.write_text(json.dumps(self.payload()), encoding="utf-8")
        model = VersionedPrefixModel.load(self.path)
        with patch.object(ContextModel, "predict", side_effect=AssertionError("wrong extractor")):
            prediction = model.predict(self.item, self.indexes, self.models)
        self.assertEqual(prediction.action, "convert")
        self.assertEqual(prediction.model_version, self.payload()["version"])
        self.assertTrue(prediction.supported)
        with patch.object(PrefixModel, "load", return_value=model) as loader:
            self.assertIs(PrefixModel.default(), model)
            self.assertIs(PrefixModel.default(), model)
            loader.assert_called_once()

    def test_uncertainty_waits_and_keep_is_not_a_conversion(self) -> None:
        uncertain = PrefixModel(ContextModel({"bias": (0., UNCERTAIN_BIAS, 0., 0.)}, "fixture"))
        prediction = uncertain.predict_features({"bias": 1., "unknown": 1.})
        self.assertEqual(prediction.action, "wait")
        self.assertAlmostEqual(sum(prediction.probabilities), 1.)
        keep = PrefixModel(ContextModel({"bias": (DOMINANT_BIAS, 0., 0., 0.)}, "fixture"))
        self.assertEqual(keep.predict_features({"bias": 1.}).action, "keep")

    def test_schema_two_adds_observed_prefix_characters_without_word_end(self) -> None:
        legacy = features(self.item, self.indexes, self.models)
        self.assertEqual(features_for_version(self.item, self.indexes, self.models), legacy)
        current = features_for_version(self.item, self.indexes, self.models, feature_version=CURRENT_PREFIX_FEATURE_VERSION)
        self.assertEqual({name: current[name] for name in legacy}, legacy)
        self.assertIn("source:prefix_char:0:4:ghbd", current)
        self.assertIn("target:prefix_char:1:4:прив", current)
        self.assertIn("source:prefix_char:0:2:^g", current)
        self.assertFalse(any("$" in name for name in current if "prefix_char:" in name))
        other = features_for_version(replace(self.item, original="abcd", alternative="фисв"),
                                     self.indexes, self.models, feature_version=CURRENT_PREFIX_FEATURE_VERSION)
        self.assertNotIn("source:prefix_char:0:4:ghbd", other)
        long = features_for_version(replace(self.item, original="a" * OBSERVED_PREFIX_MAX_CHARACTERS + "z" * TRAILING_FILLER_CHARACTERS,
                                            alternative="ф" * OBSERVED_PREFIX_MAX_CHARACTERS + "я" * TRAILING_FILLER_CHARACTERS),
                                    self.indexes, self.models, feature_version=CURRENT_PREFIX_FEATURE_VERSION)
        chars = {name: value for name, value in long.items() if "prefix_char:" in name}
        self.assertTrue(chars)
        self.assertTrue(all("z" not in name and "я" not in name and value <= PREFIX_CHAR_FEATURE_VALUE_CEILING for name, value in chars.items()))
        for invalid in (0, UNSUPPORTED_FEATURE_VERSION, True):
            with self.assertRaises(ValueError):
                features_for_version(self.item, self.indexes, self.models, feature_version=invalid)

    def test_schema_two_roundtrip_keeps_prefix_and_completed_models_separate(self) -> None:
        weights = {"source:prefix_char:0:4:ghbd": [0.0, DOMINANT_BIAS, 0.0, 0.0]}
        digest = hashlib.sha256(json.dumps(weights, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()
        payload = {**self.payload(), "feature_version": CURRENT_PREFIX_FEATURE_VERSION, "prefix_feature_version": CURRENT_PREFIX_FEATURE_VERSION,
                   "version": "prefix-v2-" + digest[:VERSION_HASH_CHARACTERS], "weights": weights, "weights_sha256": digest}
        self.path.write_text(json.dumps(payload), encoding="utf-8")
        with self.assertRaises(ValueError):
            PrefixModel.load(self.path)  # the frozen loader knows schema one only
        model = VersionedPrefixModel.load(self.path)
        self.assertEqual(model.feature_version, CURRENT_PREFIX_FEATURE_VERSION)
        with patch.object(ContextModel, "predict", side_effect=AssertionError("completed word extractor")):
            self.assertEqual(model.predict(self.item, self.indexes, self.models).action, "convert")
            changed = replace(self.item, original="abcd", alternative="фисв")
            self.assertEqual(model.predict(changed, self.indexes, self.models).action, "keep")
        for change in ({"feature_version": 1}, {"feature_version": True}, {"prefix_feature_version": 1},
                       {"prefix_feature_version": True}, {"prefix_feature_version": None},
                       {"version": "prefix-v1-" + digest[:VERSION_HASH_CHARACTERS]}, {"weights_sha256": "0" * SHA256_HEX_LENGTH}):
            self.path.write_text(json.dumps({**payload, **change}), encoding="utf-8")
            with self.subTest(change=change), self.assertRaises(ValueError):
                VersionedPrefixModel.load(self.path)

    def test_schema_one_keeps_original_values_and_probabilities(self) -> None:
        self.path.write_text(json.dumps(self.payload()), encoding="utf-8")
        model = VersionedPrefixModel.load(self.path)
        self.assertEqual(model.feature_version, 1)
        self.assertEqual(model.predict(self.item, self.indexes, self.models),
                         model.predict_features(features(self.item, self.indexes, self.models)))
        with self.assertRaises(ValueError):
            VersionedPrefixModel(model.model, feature_version=UNSUPPORTED_FEATURE_VERSION)

    def test_prefix_and_context_features_are_bounded_and_distinguish_code_from_comments(self) -> None:
        values = features(self.item, self.indexes, self.models)
        self.assertEqual(values["source:empty"], 1.)
        self.assertEqual(values["target:empty"], 0.)
        self.assertEqual(values["before:comment"], 1.)
        self.assertEqual(values["before:technical"], 0.)
        self.assertIn("before:word:проверим", values)
        self.assertIn("app:code", values)
        for before, technical, quote in (("const x = ", 1., 0.), ('const x = "', 0., 1.),
                                         ("# comment $", 0., 0.), ("", 0., 0.)):
            result = features(replace(self.item, field=FieldContext("", "1", before)), self.indexes, self.models)
            self.assertEqual(result["before:technical"], technical)
            self.assertEqual(result["before:quote"], quote)
        empty = features(replace(self.item, original="", alternative=""), self.indexes, self.models)
        self.assertEqual(empty["source:letters"], 0.)
        rich = features(replace(self.item, original="hE1", alternative="рУ1"), self.indexes, self.models)
        self.assertEqual((rich["token:capitals"], rich["token:digits"]), (1., 1.))
        reverse = features(PrefixInput("привет", "ghbdtn", 1, FieldContext("Terminal", "1", "English ё")), self.indexes, self.models)
        self.assertEqual(reverse["source:known"], 1.)
        long = features(replace(self.item, original="x" * OVER_LENGTH_CAP_CHARACTERS, field=FieldContext("A " * LONG_APPLICATION_NAME_REPEATS, "1", "secret " + "x" * FIELD_BEFORE_WINDOW_CHARACTERS)), self.indexes, self.models)
        self.assertEqual(long["length"], 1.)
        self.assertNotIn("before:word:secret", long)

    def test_invalid_artifacts_are_rejected_and_default_fails_closed(self) -> None:
        variants: list[object] = [[], {**self.payload(), "kind": "completed-word"},
            {**self.payload(), "feature_version": 0}, {**self.payload(), "actions": []}]
        updates: list[dict[str, object]] = [
            {"version": 0}, {"version": "prefix-v2-other"},
            {"weights_sha256": "bad"}, {"version": "prefix-v1-" + "0" * VERSION_HASH_CHARACTERS},
        ]
        thresholds: list[object] = [True, "1", BELOW_MINIMUM_THRESHOLD, ABOVE_MAXIMUM_THRESHOLD, math.nan]
        weights: list[object] = [[], {}, {str(i): [0.] * len(ACTIONS) for i in range(MAX_PREFIX_WEIGHTS + 1)}]
        coefficients: list[tuple[str, object]] = [
            ("", [0.] * len(ACTIONS)), ("x" * (MAX_PREFIX_FEATURE_NAME_CHARACTERS + 1), [0.] * len(ACTIONS)), ("x", "vector"),
            ("x", [1.]), ("x", [True, 0., 0., 0.]), ("x", [math.inf, 0., 0., 0.]),
        ]
        updates.extend({"conversion_threshold": value} for value in thresholds)
        updates.extend({"weights": value} for value in weights)
        updates.extend({"weights": {name: vector}} for name, vector in coefficients)
        variants.extend({**self.payload(), **update} for update in updates)
        loaders: tuple[type[PrefixModel], ...] = (PrefixModel, VersionedPrefixModel)
        for payload in variants:
            self.path.write_text(json.dumps(payload), encoding="utf-8")
            for loader in loaders:
                with self.subTest(loader=loader.__name__, payload=str(payload)[:SUBTEST_PAYLOAD_PREVIEW_CHARACTERS]), self.assertRaises(ValueError):
                    loader.load(self.path)
        self.path.write_bytes(b" " * (MAX_PREFIX_MODEL_BYTES + 1))
        for loader in loaders:
            with self.subTest(loader=loader.__name__), self.assertRaises(ValueError):
                loader.load(self.path)
        self.path.write_text("{}", encoding="utf-8")
        for loader in loaders:
            with patch("keyswitch.prefix_model.json.loads", return_value={**self.payload(), "weights": {1: [0.] * len(ACTIONS)}}), \
                    self.subTest(loader=loader.__name__), self.assertRaises(ValueError):
                loader.load(self.path)
        for error in (OSError, ValueError, TypeError):
            PrefixModel.default.cache_clear()
            with patch.object(PrefixModel, "load", side_effect=error("unavailable")):
                self.assertIsNone(PrefixModel.default())


class VersionedDefaultTests(unittest.TestCase):
    def test_the_schema_aware_default_answers_none_when_the_artifact_cannot_be_read(self) -> None:
        from keyswitch.prefix_schema import VersionedPrefixModel

        VersionedPrefixModel.default.cache_clear()
        self.addCleanup(VersionedPrefixModel.default.cache_clear)
        for error in (OSError("no such file"), ValueError("unsupported prefix model"), TypeError("bad payload")):
            with self.subTest(error=type(error).__name__):
                VersionedPrefixModel.default.cache_clear()
                with patch.object(VersionedPrefixModel, "load", side_effect=error):
                    self.assertIsNone(VersionedPrefixModel.default())
        VersionedPrefixModel.default.cache_clear()
        self.assertIsNotNone(VersionedPrefixModel.default())


if __name__ == "__main__":
    unittest.main()
