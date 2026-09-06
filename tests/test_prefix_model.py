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


class PrefixModelTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.path = Path(temporary.name) / "prefix.json"
        self.models = {
            group: LanguageModel(locale, words, "fixture", enable_spellcheck=False)
            for group, locale, words in ((0, "en_US", {"hello": 100, "world": 50}),
                                         (1, "ru_RU", {"привет": 100, "привод": 50}))
        }
        self.indexes = {group: PrefixIndex(model.frequencies, model.frequencies) for group, model in self.models.items()}
        self.item = PrefixInput("ghbd", "прив", 0, FieldContext("Code", "1", "// проверим ", role="code"))
        PrefixModel.default.cache_clear()
        self.addCleanup(PrefixModel.default.cache_clear)

    @staticmethod
    def payload() -> dict[str, object]:
        weights = {"bias": [0.0, 10.0, 0.0, 0.0]}
        digest = hashlib.sha256(json.dumps(weights, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()
        return {"kind": "keyswitch.prefix-policy", "feature_version": 1, "actions": list(ACTIONS),
                "version": "prefix-v1-" + digest[:12], "conversion_threshold": 0.999,
                "weights": weights, "weights_sha256": digest}

    def test_load_predict_and_cache_without_completed_word_extractor(self) -> None:
        self.path.write_text(json.dumps(self.payload()), encoding="utf-8")
        model = PrefixModel.load(self.path)
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
        uncertain = PrefixModel(ContextModel({"bias": (0., 2., 0., 0.)}, "fixture"))
        prediction = uncertain.predict_features({"bias": 1., "unknown": 1.})
        self.assertEqual(prediction.action, "wait")
        self.assertAlmostEqual(sum(prediction.probabilities), 1.)
        keep = PrefixModel(ContextModel({"bias": (20., 0., 0., 0.)}, "fixture"))
        self.assertEqual(keep.predict_features({"bias": 1.}).action, "keep")

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
        long = features(replace(self.item, original="x" * 30, field=FieldContext("A " * 100, "1", "secret " + "x" * 512)), self.indexes, self.models)
        self.assertEqual(long["length"], 1.)
        self.assertNotIn("before:word:secret", long)

    def test_invalid_artifacts_are_rejected_and_default_fails_closed(self) -> None:
        variants: list[object] = [[], {**self.payload(), "kind": "completed-word"},
            {**self.payload(), "feature_version": 0}, {**self.payload(), "actions": []}]
        updates: list[dict[str, object]] = [
            {"version": 0}, {"version": "prefix-v2-other"},
            {"weights_sha256": "bad"}, {"version": "prefix-v1-" + "0" * 12},
        ]
        thresholds: list[object] = [True, "1", .9, 1.1, math.nan]
        weights: list[object] = [[], {}, {str(i): [0.] * 4 for i in range(10001)}]
        coefficients: list[tuple[str, object]] = [
            ("", [0.] * 4), ("x" * 161, [0.] * 4), ("x", "vector"),
            ("x", [1.]), ("x", [True, 0., 0., 0.]), ("x", [math.inf, 0., 0., 0.]),
        ]
        updates.extend({"conversion_threshold": value} for value in thresholds)
        updates.extend({"weights": value} for value in weights)
        updates.extend({"weights": {name: vector}} for name, vector in coefficients)
        variants.extend({**self.payload(), **update} for update in updates)
        for payload in variants:
            self.path.write_text(json.dumps(payload), encoding="utf-8")
            with self.subTest(payload=str(payload)[:150]), self.assertRaises(ValueError):
                PrefixModel.load(self.path)
        self.path.write_bytes(b" " * (2 * 1024 * 1024 + 1))
        with self.assertRaises(ValueError):
            PrefixModel.load(self.path)
        self.path.write_text("{}", encoding="utf-8")
        with patch("keyswitch.prefix_model.json.loads", return_value={**self.payload(), "weights": {1: [0.] * 4}}), self.assertRaises(ValueError):
            PrefixModel.load(self.path)
        for error in (OSError, ValueError, TypeError):
            PrefixModel.default.cache_clear()
            with patch.object(PrefixModel, "load", side_effect=error("unavailable")):
                self.assertIsNone(PrefixModel.default())


if __name__ == "__main__":
    unittest.main()
