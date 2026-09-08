"""Loading, scoring and licensing for the key-space orthotactic model."""

from __future__ import annotations

import json
import tempfile
import unittest
from copy import deepcopy
from dataclasses import replace
from pathlib import Path

from keyswitch.context_policy import ContextPolicy, ContextResult
from keyswitch.context_model import ContextPrediction
from keyswitch.detector import DetectionDecision
from keyswitch.input_context import FieldContext
from keyswitch.language_model import WordScore
from keyswitch.ortho_model import (
    ARTIFACT_PATH, OrthoEvidence, OrthoModel, shape_of,
)

SCORE = WordScore(0.0, False, 0, 0.0)


def channel() -> dict[str, object]:
    return {"grams": "a\nb\nab", "logprob": [-512, -512, -256],
            "backoff": [["a", -128], ["b", -128]], "uniform": -1024}


def shape_table() -> dict[str, object]:
    return {"lower": 0, "initial": -512, "inner": -512, "upper": -2048}


def minimal() -> dict[str, object]:
    """A tiny but structurally complete artifact, so every guard can be tried."""

    shape = shape_table()
    return {
        "schema_version": 1, "version": "ortho-v1-000000000000", "order": 3, "scale": 512,
        "models": {"en": channel(), "ru": channel()},
        "prose_shape": {"en": dict(shape), "ru": dict(shape)},
        "acronym_shape": {"en": dict(shape), "ru": dict(shape)},
        "thresholds": {"en": 512, "ru": 512}, "minimum_length": 3,
    }


class OrthoArtifactTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.path = Path(self.temporary.name) / "ortho.json"

    def write(self, payload: object) -> Path:
        self.path.write_text(json.dumps(payload), encoding="utf-8")
        return self.path

    def test_a_complete_artifact_loads_and_scores(self) -> None:
        model = OrthoModel.load(self.write(minimal()))
        self.assertEqual((model.order, model.minimum_length), (3, 3))
        self.assertEqual(model.thresholds, {"en": 1.0, "ru": 1.0})
        score = model.score(OrthoEvidence("ab", "lower", "ru"))
        self.assertTrue(score.supported)
        # Both channels are identical here, so the languages cannot disagree.
        self.assertEqual(score.ratio, 0.0)
        self.assertEqual(OrthoModel.try_load(self.path)[1], "ortho-v1-000000000000")

    def test_scoring_walks_the_backoff_chain_and_falls_back_to_uniform(self) -> None:
        model = OrthoModel.load(self.write(minimal()))
        channel = model.channels["en"]
        self.assertEqual(channel._conditional("ab"), -0.5)          # stored directly
        self.assertEqual(channel._conditional("ba"), -0.25 - 1.0)   # backoff on "b", then "a"
        self.assertEqual(channel._conditional("z"), -2.0)           # unigram falls to uniform

    def test_an_unusable_source_or_empty_token_is_not_scored(self) -> None:
        model = OrthoModel.load(self.write(minimal()))
        for evidence in (OrthoEvidence("ab", "lower", "de"), OrthoEvidence("", "lower", "ru")):
            self.assertFalse(model.score(evidence).supported)
        # An unknown shape is read conservatively rather than rejected.
        self.assertTrue(model.score(OrthoEvidence("ab", "sideways", "ru")).supported)

    def test_every_structural_guard_refuses_its_own_defect(self) -> None:
        cases: list[tuple[str, object]] = [
            ("not an object", []),
            ("wrong schema", {**minimal(), "schema_version": 2}),
            ("order not a number", {**minimal(), "order": "3"}),
            ("order out of range", {**minimal(), "order": 99}),
            ("version not a string", {**minimal(), "version": 1}),
            ("foreign version", {**minimal(), "version": "context-v1-0"}),
            ("scale not a number", {**minimal(), "scale": "512"}),
            ("scale out of range", {**minimal(), "scale": 0}),
            ("models missing", {**minimal(), "models": {"en": channel()}}),
            ("thresholds missing", {**minimal(), "thresholds": {"en": 1}}),
            ("threshold not a number", {**minimal(), "thresholds": {"en": 1.5, "ru": 1}}),
            ("minimum length absent", {**minimal(), "minimum_length": None}),
            ("minimum length out of range", {**minimal(), "minimum_length": 0}),
            ("shape channel missing", {**minimal(), "prose_shape": {"en": {}}}),
        ]
        for label, payload in cases:
            with self.subTest(label=label):
                with self.assertRaises(ValueError):
                    OrthoModel.load(self.write(payload))

    def test_channel_and_shape_contents_are_validated(self) -> None:
        def broken(mutate: object) -> dict[str, object]:
            payload = deepcopy(minimal())
            payload["models"] = {"en": mutate, "ru": channel()}
            return payload

        base = channel()
        cases: list[tuple[str, object]] = [
            ("channel not an object", broken([])),
            ("grams not text", broken({**base, "grams": 1})),
            ("weights not a list", broken({**base, "logprob": {}})),
            ("counts disagree", broken({**base, "logprob": [-1]})),
            ("weight not a number", broken({**base, "logprob": [-1, -1, 0.5]})),
            ("gram longer than the order", broken({**base, "grams": "a\nb\nabcd"})),
            ("backoff entry malformed", broken({**base, "backoff": [["a"]]})),
            ("backoff name not text", broken({**base, "backoff": [[1, 2]]})),
            ("uniform not a number", broken({**base, "uniform": None})),
        ]
        shape = shape_table()
        cases += [
            ("shape table not an object", {**minimal(), "prose_shape": {"en": [], "ru": shape}}),
            ("shape missing a case", {**minimal(),
                                      "prose_shape": {"en": {"lower": 0}, "ru": shape}}),
            ("shape value not a number", {**minimal(),
                                          "prose_shape": {"en": {**shape, "upper": 1.5},
                                                          "ru": shape}}),
        ]
        for label, payload in cases:
            with self.subTest(label=label):
                with self.assertRaises(ValueError):
                    OrthoModel.load(self.write(payload))

    def test_an_oversized_or_missing_artifact_is_reported_not_raised(self) -> None:
        self.path.write_bytes(b"[" + b" " * (24 * 1024 * 1024))
        with self.assertRaisesRegex(ValueError, "oversized"):
            OrthoModel.load(self.path)
        model, status = OrthoModel.try_load(self.path.parent / "absent.json")
        self.assertIsNone(model)
        self.assertTrue(status.startswith("unavailable:"))

    def test_case_shape_names_the_reading_the_user_meant(self) -> None:
        self.assertEqual(shape_of("РСФСР", False), "upper")
        self.assertEqual(shape_of("Дэн", False), "inner")
        self.assertEqual(shape_of("Привет", True), "initial")
        self.assertEqual(shape_of("htop", False), "lower")
        self.assertEqual(shape_of("2", False), "lower")


class OrthoLicenceTests(unittest.TestCase):
    """The licence adds conversions where no dictionary can help, and only there."""

    def setUp(self) -> None:
        self.policy = ContextPolicy()
        if self.policy.ortho is None:  # pragma: no cover - the artifact ships with the package
            self.skipTest("orthotactic artifact unavailable")
        self.field = FieldContext("Code", "1", "", "")

    def decision(self, original: str, *, convert: bool = False, group: int = 1) -> DetectionDecision:
        return DetectionDecision(convert, original, original, group, group, 0.0,
                                 "не требуется", SCORE, SCORE)

    def licence(self, original: str, alternative: str, *, group: int = 1,
                convert: bool = False) -> DetectionDecision | None:
        baseline = self.decision(original, convert=convert, group=group)
        return self.policy._orthotactic(baseline, alternative, 1 - group, self.field)

    def test_a_command_typed_in_the_wrong_layout_is_licensed(self) -> None:
        for original, alternative in (("рещз", "htop"), ("цуиоы", "webjs"), ("ишдв", "bild")):
            with self.subTest(original=original):
                licensed = self.licence(original, alternative)
                assert licensed is not None
                self.assertTrue(licensed.should_convert)
                self.assertEqual(licensed.replacement, alternative)
                self.assertEqual(licensed.reason, "последовательность клавиш не соответствует языку")

    def test_correct_text_and_unusable_input_are_never_licensed(self) -> None:
        self.assertIsNone(self.licence("привет", "ghbdtn"))
        self.assertIsNone(self.licence("пересборку", "gthtc,jhre"))
        self.assertIsNone(self.licence("hello", "руддщ", group=0))
        self.assertIsNone(self.licence("ab", "ab"))                      # shorter than the model allows
        self.assertIsNone(self.licence("рещз", "htop", group=-1))        # layout unknown
        self.assertIsNone(self.licence("рещз", ""))                      # nothing to score

    def test_the_licence_never_overturns_a_refusal_that_belongs_to_another_layer(self) -> None:
        prediction = ContextPrediction("keep", 1.0, (1.0, 0.0, 0.0, 0.0), "ortho-test", True)
        keep = ContextResult(self.decision("рещз"), prediction, self.field)
        licensed = self.policy._licensed(keep, self.decision("рещз"), "htop", 0, self.field)
        self.assertTrue(licensed.decision.should_convert)
        self.assertEqual(licensed.decision_source, "ortho_model")
        for label, result, baseline in (
            ("already converting", replace(keep, decision=self.decision("рещз", convert=True)),
             self.decision("рещз")),
            ("detector wanted it and was vetoed", keep, self.decision("рещз", convert=True)),
            ("model is still waiting",
             replace(keep, prediction=ContextPrediction("wait", 1.0, (0.0, 0.0, 1.0, 0.0), "ortho-test", True)),
             self.decision("рещз")),
            ("no prediction at all", replace(keep, prediction=None), self.decision("рещз")),
        ):
            with self.subTest(label=label):
                self.assertIs(self.policy._licensed(result, baseline, "htop", 0, self.field), result)

    def test_without_an_artifact_the_licence_disappears_silently(self) -> None:
        self.policy.ortho = None
        prediction = ContextPrediction("keep", 1.0, (1.0, 0.0, 0.0, 0.0), "ortho-test", True)
        keep = ContextResult(self.decision("рещз"), prediction, self.field)
        self.assertIs(self.policy._licensed(keep, self.decision("рещз"), "htop", 0, self.field), keep)
        self.assertIsNone(self.policy._orthotactic(self.decision("рещз"), "htop", 0, self.field))

    def test_the_shipped_artifact_is_the_one_the_package_declares(self) -> None:
        model = OrthoModel.load(ARTIFACT_PATH)
        self.assertTrue(model.version.startswith("ortho-v1-"))
        self.assertGreaterEqual(model.minimum_length, 3)
