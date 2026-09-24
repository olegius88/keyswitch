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
from keyswitch.context_policy import word_shaped
from keyswitch.ortho_model import ARTIFACT_PATH, MAX_ARTIFACT_BYTES, OrthoEvidence, OrthoModel, shape_of

SCORE = WordScore(0.0, False, 0, 0.0)
KNOWN = WordScore(0.0, True, 0, 0.0)

# channel()'s fixture: order-1 grams "a"/"b" share one stored logprob, the
# order-2 gram "ab" has its own, backoff off either order-1 gram costs the
# same, and anything unseen falls all the way to the uniform logprob.
GRAM_LOGPROB_SHORT = -512
GRAM_LOGPROB_LONG = -256
BACKOFF_WEIGHT = -128
UNIFORM_LOGPROB = -1024
# shape_table()'s fixture: "initial"/"inner" share a moderate penalty, "upper"
# a severe one ("lower" is the unpenalized default, 0).
SHAPE_PENALTY_MODERATE = -512
SHAPE_PENALTY_SEVERE = -2048
# minimal()'s own fixture values. FIXTURE_SCALE doubles as both "scale" and
# each language's raw "thresholds" entry, so the loaded, normalized threshold
# comes out to exactly 1.0.
FIXTURE_ORDER = 3
FIXTURE_SCALE = 512
FIXTURE_MINIMUM_LENGTH = 3
WRONG_SCHEMA_VERSION = 2
OUT_OF_RANGE_ORDER = 99
NON_INTEGER_PROBE = 1.5
NON_INTEGER_LOGPROB = 0.5
PLACEHOLDER_INT = 2  # an arbitrary int where a gram name (text) is required
# A floor the shipped production artifact (not the minimal() fixture) is
# expected to clear.
SHIPPED_MINIMUM_LENGTH_FLOOR = 3


def channel() -> dict[str, object]:
    return {"grams": "a\nb\nab", "logprob": [GRAM_LOGPROB_SHORT, GRAM_LOGPROB_SHORT, GRAM_LOGPROB_LONG],
            "backoff": [["a", BACKOFF_WEIGHT], ["b", BACKOFF_WEIGHT]], "uniform": UNIFORM_LOGPROB}


def shape_table() -> dict[str, object]:
    return {"lower": 0, "initial": SHAPE_PENALTY_MODERATE, "inner": SHAPE_PENALTY_MODERATE, "upper": SHAPE_PENALTY_SEVERE}


def minimal() -> dict[str, object]:
    """A tiny but structurally complete artifact, so every guard can be tried."""

    shape = shape_table()
    return {
        "schema_version": 1, "version": "ortho-v1-000000000000", "order": FIXTURE_ORDER, "scale": FIXTURE_SCALE,
        "models": {"en": channel(), "ru": channel()},
        "prose_shape": {"en": dict(shape), "ru": dict(shape)},
        "acronym_shape": {"en": dict(shape), "ru": dict(shape)},
        "thresholds": {"en": FIXTURE_SCALE, "ru": FIXTURE_SCALE}, "minimum_length": FIXTURE_MINIMUM_LENGTH,
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
        self.assertEqual((model.order, model.minimum_length), (FIXTURE_ORDER, FIXTURE_MINIMUM_LENGTH))
        self.assertEqual(model.thresholds, {"en": 1.0, "ru": 1.0})
        score = model.score(OrthoEvidence("ab", "lower", "ru"))
        self.assertTrue(score.supported)
        # Both channels are identical here, so the languages cannot disagree.
        self.assertEqual(score.ratio, 0.0)
        self.assertEqual(OrthoModel.try_load(self.path)[1], "ortho-v1-000000000000")

    def test_scoring_walks_the_backoff_chain_and_falls_back_to_uniform(self) -> None:
        model = OrthoModel.load(self.write(minimal()))
        channel = model.channels["en"]
        self.assertEqual(channel._conditional("ab"), GRAM_LOGPROB_LONG / FIXTURE_SCALE)          # stored directly
        self.assertEqual(channel._conditional("ba"), BACKOFF_WEIGHT / FIXTURE_SCALE + GRAM_LOGPROB_SHORT / FIXTURE_SCALE)   # backoff on "b", then "a"
        self.assertEqual(channel._conditional("z"), UNIFORM_LOGPROB / FIXTURE_SCALE)           # unigram falls to uniform

    def test_an_unusable_source_or_empty_token_is_not_scored(self) -> None:
        model = OrthoModel.load(self.write(minimal()))
        for evidence in (OrthoEvidence("ab", "lower", "de"), OrthoEvidence("", "lower", "ru")):
            self.assertFalse(model.score(evidence).supported)
        # An unknown shape is read conservatively rather than rejected.
        self.assertTrue(model.score(OrthoEvidence("ab", "sideways", "ru")).supported)

    def test_every_structural_guard_refuses_its_own_defect(self) -> None:
        cases: list[tuple[str, object]] = [
            ("not an object", []),
            ("wrong schema", {**minimal(), "schema_version": WRONG_SCHEMA_VERSION}),
            ("order not a number", {**minimal(), "order": "3"}),
            ("order out of range", {**minimal(), "order": OUT_OF_RANGE_ORDER}),
            ("version not a string", {**minimal(), "version": 1}),
            ("foreign version", {**minimal(), "version": "context-v1-0"}),
            ("scale not a number", {**minimal(), "scale": "512"}),
            ("scale out of range", {**minimal(), "scale": 0}),
            ("models missing", {**minimal(), "models": {"en": channel()}}),
            ("thresholds missing", {**minimal(), "thresholds": {"en": 1}}),
            ("threshold not a number", {**minimal(), "thresholds": {"en": NON_INTEGER_PROBE, "ru": 1}}),
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
            ("weight not a number", broken({**base, "logprob": [-1, -1, NON_INTEGER_LOGPROB]})),
            ("gram longer than the order", broken({**base, "grams": "a\nb\nabcd"})),
            ("backoff entry malformed", broken({**base, "backoff": [["a"]]})),
            ("backoff name not text", broken({**base, "backoff": [[1, PLACEHOLDER_INT]]})),
            ("uniform not a number", broken({**base, "uniform": None})),
        ]
        shape = shape_table()
        cases += [
            ("shape table not an object", {**minimal(), "prose_shape": {"en": [], "ru": shape}}),
            ("shape missing a case", {**minimal(),
                                      "prose_shape": {"en": {"lower": 0}, "ru": shape}}),
            ("shape value not a number", {**minimal(),
                                          "prose_shape": {"en": {**shape, "upper": NON_INTEGER_PROBE},
                                                          "ru": shape}}),
        ]
        for label, payload in cases:
            with self.subTest(label=label):
                with self.assertRaises(ValueError):
                    OrthoModel.load(self.write(payload))

    def test_an_oversized_or_missing_artifact_is_reported_not_raised(self) -> None:
        self.path.write_bytes(b"[" + b" " * MAX_ARTIFACT_BYTES)
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
        for original, alternative in (("рещз", "htop"), ("тпште", "nginx"), ("ишдв", "bild")):
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

    def test_a_hyphenated_word_typed_in_the_wrong_layout_is_licensed(self) -> None:
        """`Я-то` mistyped reads `Z-nj`; the hyphen must not disqualify it."""

        licensed = self.licence("Z-nj", "Я-то", group=0)
        assert licensed is not None
        self.assertEqual(licensed.replacement, "Я-то")

    def test_the_corpus_boundary_set_matches_the_engine(self) -> None:
        """The corpus copies the engine's punctuation set; drift would silently
        change which tokens the evidence is measured on."""

        import sys
        sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
        import ortho_v2_corpus  # noqa: PLC0415
        from keyswitch.engine import PUNCTUATION  # noqa: PLC0415

        self.assertEqual(ortho_v2_corpus.BOUNDARY_PUNCTUATION, frozenset(PUNCTUATION))

    def test_punctuation_between_letters_is_not_a_word_in_any_layout(self) -> None:
        """One inner hyphen is a word; a quotation mark or a second hyphen is not."""

        self.assertIsNone(self.licence('и"ю', 'b".'))
        self.assertIsNone(self.licence("зы-ы-ырк", "ps-s-shr"))
        self.assertIsNone(self.licence("-рещз", "-htop"))
        self.assertTrue(word_shaped("Я-то"))
        self.assertTrue(word_shaped("don't"))
        for token in ('и"ю', "зы-ы-ырк", "-htop", "htop-", "a", ""):
            self.assertFalse(word_shaped(token), token)

    def test_a_token_the_dictionary_knows_is_never_licensed(self) -> None:
        """`руку` is a Russian word whose keys spell the English word `here`."""

        baseline = DetectionDecision(False, "руку", "руку", 1, 1, 0.0, "не требуется",
                                     KNOWN, SCORE)
        self.assertIsNone(self.policy._orthotactic(baseline, "here", 0, self.field))

    def test_the_shipped_artifact_is_the_one_the_package_declares(self) -> None:
        model = OrthoModel.load(ARTIFACT_PATH)
        self.assertTrue(model.version.startswith("ortho-v1-"))
        self.assertGreaterEqual(model.minimum_length, SHIPPED_MINIMUM_LENGTH_FLOOR)
        self.assertEqual(set(model.thresholds), {"en", "ru"})
