"""Context semantics, real editing, privacy and trained-policy regressions."""

from __future__ import annotations

import hashlib
import json
import math
import tempfile
import time
import unittest
from dataclasses import replace
from pathlib import Path
from types import SimpleNamespace
from typing import cast
from unittest.mock import patch

from keyswitch.backend import KeyEvent
from keyswitch.constants.keyboard import ALT_MASK, CONTROL_MASK
from keyswitch.context_model import (
    ACTIONS, FEATURE_VERSION, ContextAction, ContextEvidence, ContextModel,
    extract_context_features, one_typo_from_word, softmax,
)
from keyswitch.context_policy import ContextPolicy, ContextResult, evidence_for_decision
from keyswitch.short_words import opens_sentences
from keyswitch.detector import LanguageDetector, LanguageScorer
from keyswitch.language_model import LanguageModel
from keyswitch.engine import KeySwitchEngine
from keyswitch.learning import LearnedRule
from keyswitch.input_context import CONTEXT_LIMIT, CONTEXT_TTL, FieldContext, InputContext
from test_input_integrity import InputIntegrityTests
from keyswitch.constants.text import FIELD_CONTEXT_APPLICATION_MAX_CHARACTERS
from fixture_values.counts import (
    CONTEXT_FILLER_CHARACTERS,
    FIELD_CONTEXT_OVERLONG_NAME_CHARACTERS,
    FIELD_CONTEXT_OVERLONG_TEXT_CHARACTERS,
)
from fixture_values.keys import CONTEXT_POLICY_FIXTURE_KEYCODE, SECOND_WINDOW_ID
from fixture_values.models import NON_STRING_FIELD_VALUE
from fixture_values.scores import (
    DOMINANT_BIAS_WEIGHT,
    INVALID_CONTEXT_CONVERSION_THRESHOLD,
    MODERATE_BIAS_WEIGHT,
    SOFTMAX_HIGH_LOGIT,
    SOFTMAX_NEAR_HIGH_LOGIT,
)
from keyswitch.constants.models import (
    CONTEXT_ACTION_FEATURE_VERSION,
    CONTEXT_OPENING_FEATURE_VERSION,
    CONTEXT_TYPO_FEATURE_VERSION,
    CONTEXT_V1_CONVERSION_THRESHOLD,
    CONTEXT_V1_FEATURE_VERSIONS,
)


class InputContextTests(unittest.TestCase):

    def setUp(self) -> None:
        self.stream = InputContext()
        self.stream.focus("chat", 1)

    @staticmethod
    def event(character: str = "", name: str = "") -> KeyEvent:
        return KeyEvent(True, CONTEXT_POLICY_FIXTURE_KEYCODE, name or character, character, (character, character), 0, 0, 1)

    def type(self, text: str) -> None:
        for character in text:
            self.stream.observe(self.event(character))

    def test_bounded_field_history_and_suffix_anchoring(self) -> None:
        self.type("а" * CONTEXT_FILLER_CHARACTERS + " привет")
        self.assertEqual(len(self.stream.text), CONTEXT_LIMIT)
        prefix = self.stream.before_word("привет")
        self.assertTrue(prefix.endswith(" "))
        self.type("\u00a0")
        self.assertEqual(self.stream.before_word("привет"), prefix[1:])
        self.assertEqual(self.stream.before_word("another"), "")
        self.assertEqual(self.stream.before_word(""), "")
        snapshot = self.stream.snapshot("привет")
        self.assertEqual(snapshot.application, "chat")
        self.assertEqual(snapshot.field_id, "1")
        self.stream.focus("chat", 1)
        self.assertNotEqual(self.stream.text, "")
        self.stream.focus("chat", SECOND_WINDOW_ID)
        self.assertEqual(self.stream.text, "")

    def test_edit_invalidation_and_ignored_events(self) -> None:
        self.type("hello")
        self.stream.observe(replace(self.event("x"), pressed=False))
        self.stream.observe(replace(self.event("x"), synthetic=True))
        self.stream.observe(self.event(name="Shift_L"))
        self.assertEqual(self.stream.text, "hello")
        self.stream.observe(self.event(name="BackSpace"))
        self.assertEqual(self.stream.text, "hell")
        self.stream.observe(replace(self.event(name="Return"), deferred=True))
        self.assertEqual(self.stream.text, "hell")
        for key in ("Return", "Tab", "Pointer", "Home", "Delete"):
            self.type("text")
            self.stream.observe(self.event(name=key))
            self.assertEqual(self.stream.text, "")
        self.type("text")
        self.stream.observe(replace(self.event("v"), state=CONTROL_MASK))
        self.assertEqual(self.stream.text, "")

    def test_ttl_and_exact_correction(self) -> None:
        self.type("проверим ghbdtn ")
        self.stream.replace_suffix("ghbdtn", "привет", " ")
        self.assertEqual(self.stream.text, "проверим привет ")
        self.stream.replace_suffix("old", "new", " ")
        self.assertEqual(self.stream.text, "")
        self.type("stale")
        self.stream.updated_at = time.monotonic() - (CONTEXT_TTL + 1)
        self.assertEqual(self.stream.before_word("stale"), "")
        self.type("new")
        self.assertEqual(self.stream.text, "new")

    def test_native_context_never_returns_sensitive_text(self) -> None:
        for field in (FieldContext("app", "field", "secret", "suffix", sensitive=True),
                      FieldContext("app", "field", "secret", "suffix", role="password")):
            bounded = field.bounded()
            self.assertTrue(bounded.sensitive)
            self.assertEqual((bounded.before, bounded.after), ("", ""))
        field = FieldContext(
            "x" * FIELD_CONTEXT_OVERLONG_NAME_CHARACTERS, "y" * FIELD_CONTEXT_OVERLONG_NAME_CHARACTERS,
            "z" * FIELD_CONTEXT_OVERLONG_TEXT_CHARACTERS, "w" * FIELD_CONTEXT_OVERLONG_TEXT_CHARACTERS,
        ).bounded()
        self.assertEqual(len(field.before), CONTEXT_LIMIT)
        self.assertEqual(len(field.application), FIELD_CONTEXT_APPLICATION_MAX_CHARACTERS)


class ContextModelTests(unittest.TestCase):

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.path = Path(self.temporary.name) / "model.json"
        self.item = ContextEvidence("ghbdtn", "привет", 0, FieldContext("Telegram", "1", "я думаю ", "", "text"), baseline_convert=True)

    def payload(self) -> dict[str, object]:
        weights = {"bias": [0.0, MODERATE_BIAS_WEIGHT, 0.0, 0.0]}
        checksum = hashlib.sha256(json.dumps(weights, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()
        return {"feature_version": FEATURE_VERSION, "actions": list(ACTIONS), "weights": weights, "weights_sha256": checksum, "version": "context-v1-test", "conversion_threshold": CONTEXT_V1_CONVERSION_THRESHOLD}

    def save(self, value: object) -> None:
        self.path.write_text(json.dumps(value), encoding="utf-8")

    def test_strict_artifact_validation_and_fallback(self) -> None:
        self.save(self.payload())
        model = ContextModel.load(self.path)
        self.assertEqual(model.predict(self.item).action, "convert")
        updates_list: tuple[dict[str, object], ...] = (
            {"feature_version": -1}, {"actions": []}, {"weights": []}, {"weights": {}},
            {"weights": {"x": [1]}}, {"weights": {"x": [True, 0, 0, 0]}},
            {"weights": {"x": [math.inf, 0, 0, 0]}},
            {"weights_sha256": "bad"}, {"version": NON_STRING_FIELD_VALUE},
            {"conversion_threshold": INVALID_CONTEXT_CONVERSION_THRESHOLD}, {"conversion_threshold": True},
        )
        for updates in updates_list:
            with self.subTest(updates=updates):
                self.save({**self.payload(), **updates})
                with self.assertRaises(ValueError):
                    ContextModel.load(self.path)
        self.save([])
        with self.assertRaises(ValueError):
            ContextModel.load(self.path)
        # An action model has a feature budget of its own, larger than the older schemas' one.
        action = {**self.payload(), "feature_version": CONTEXT_ACTION_FEATURE_VERSION, "version": "context-v3-test"}
        self.save(action)
        self.assertEqual(ContextModel.load(self.path).feature_version, CONTEXT_ACTION_FEATURE_VERSION)
        for limit, refused, accepted in (("MAX_CONTEXT_ACTION_MODEL_FEATURES", action, self.payload()),
                                         ("MAX_FEATURES", self.payload(), action)):
            with self.subTest(limit=limit), patch(f"keyswitch.context_model.{limit}", 0):
                self.save(refused)
                with self.assertRaises(ValueError):
                    ContextModel.load(self.path)
                self.save(accepted)
                ContextModel.load(self.path)
        self.save({**action, "weights": {}})
        with self.assertRaises(ValueError):
            ContextModel.load(self.path)
        with patch("keyswitch.context_model.MAX_ARTIFACT_BYTES", 1):
            with self.assertRaises(ValueError):
                ContextModel.load(self.path)
        with patch.object(ContextModel, "load", side_effect=OSError("missing")):
            model_or_none, status = ContextModel.try_load()
            self.assertIsNone(model_or_none)
            self.assertEqual(status, "missing")
        with patch.object(ContextModel, "load", return_value=model):
            self.assertEqual(ContextModel.try_load(), (model, model.version))

    def test_prediction_uncertainty_and_context_features(self) -> None:
        model = ContextModel({"bias": (0.0, 1.0, 0.0, 0.0)}, "test")
        prediction = model.predict(self.item)
        self.assertEqual(prediction.action, "suggest")
        self.assertFalse(prediction.supported)
        features = extract_context_features(self.item)
        self.assertIn("before:word:думаю", features)
        other = replace(self.item, field=FieldContext("Code", "1", "const a = ", "hello", "code"))
        self.assertNotEqual(features, extract_context_features(other))
        self.assertNotEqual(features, extract_context_features(replace(self.item, source_group=1)))
        self.assertAlmostEqual(sum(softmax([SOFTMAX_HIGH_LOGIT, SOFTMAX_NEAR_HIGH_LOGIT])), 1.0)
        self.assertEqual(len(extract_context_features(replace(self.item, original="", alternative=""))) > 0, True)
        rich = replace(self.item, original="a_2", alternative="ф_2", field=FieldContext("test", "1", "// привет =", "hello"))
        self.assertEqual(extract_context_features(rich)["token:digits"], 1.0)

    def test_typo_evidence_and_digit_direction_belong_to_schema_five(self) -> None:
        stray = replace(self.item, original="aghbdtn", alternative="фпривет", target_typo=True)
        legacy = extract_context_features(stray)
        self.assertFalse([name for name in legacy if name.startswith(("typo:", "token:digits:direction"))])
        features = extract_context_features(stray, CONTEXT_TYPO_FEATURE_VERSION)
        self.assertEqual(features["typo:0:1:known:0:0"], 1.0)
        self.assertEqual(features["typo:0:1:known:0:0:baseline:1"], 1.0)
        self.assertNotIn("token:digits:direction:0", features)
        command = replace(self.item, original="зь2", alternative="pm2", source_group=1)
        self.assertEqual(extract_context_features(command, CONTEXT_TYPO_FEATURE_VERSION)["token:digits:direction:1"], 1.0)
        weights = {"typo:0:1:known:0:0": (0.0, DOMINANT_BIAS_WEIGHT, 0.0, 0.0)}
        typo_model = ContextModel(weights, "context-v1-typo", feature_version=CONTEXT_TYPO_FEATURE_VERSION)
        self.assertEqual(typo_model.predict(stray).action, "convert")
        self.assertNotEqual(ContextModel(weights, "context-v1-legacy").predict(stray).action, "convert")
        self.save({**self.payload(), "feature_version": CONTEXT_TYPO_FEATURE_VERSION})
        self.assertEqual(ContextModel.load(self.path).feature_version, CONTEXT_TYPO_FEATURE_VERSION)
        self.save({**self.payload(), "feature_version": CONTEXT_TYPO_FEATURE_VERSION, "version": "context-v3-test"})
        with self.assertRaises(ValueError):
            ContextModel.load(self.path)

    def test_one_typo_from_word_reads_the_frequency_lexicon(self) -> None:
        russian = LanguageModel("ru_RU", {"привет": 1}, "fixture", enable_spellcheck=False)
        for typed in ("фпривет", "пирвет", "прибет", "приет", "ПРИВЕТТ"):
            with self.subTest(typed=typed):
                self.assertTrue(one_typo_from_word(typed, russian))
        for typed in ("привет", "прив", "при-вет", "тучеоы"):
            with self.subTest(typed=typed):
                self.assertFalse(one_typo_from_word(typed, russian))
        # Without a known alphabet only extra and swapped letters can be found.
        other = LanguageModel("xx_XX", {"hello": 1}, "fixture", enable_spellcheck=False)
        self.assertTrue(one_typo_from_word("helloo", other))
        self.assertFalse(one_typo_from_word("hallo", other))

    def test_the_evidence_builder_marks_readings_one_typo_from_a_word(self) -> None:
        detector = LanguageDetector({0: LanguageModel("en_US", {"hello": 1}, "fixture", enable_spellcheck=False),
                                     1: LanguageModel("ru_RU", {"привет": 1}, "fixture", enable_spellcheck=False)})
        field = FieldContext("editor", "1", "", "", "unknown")
        stray = evidence_for_decision(detector.decide("aghbdtn", {1: "фпривет"}, 0), "фпривет", 1, detector, field, "enter")
        self.assertEqual((stray.source_typo, stray.target_typo), (False, True))
        typed = evidence_for_decision(detector.decide("helloo", {1: "руддщщ"}, 0), "руддщщ", 1, detector, field, "space")
        self.assertEqual((typed.source_typo, typed.target_typo), (True, False))
        # A scorer without a frequency lexicon (a test double, say) reports no typo.
        scorer = detector.models[1]
        bare = LanguageDetector({0: detector.models[0], 1: cast(LanguageScorer, SimpleNamespace(
            score=scorer.score, context_score=scorer.context_score, best_single_deletion=scorer.best_single_deletion))})
        without = evidence_for_decision(bare.decide("aghbdtn", {1: "фпривет"}, 0), "фпривет", 1, bare, field, "enter")
        self.assertFalse(without.target_typo)

    def test_opening_evidence_and_whether_the_word_stands_alone_belong_to_schema_six(self) -> None:
        lone = replace(self.item, original="ns", alternative="ты", field=FieldContext("editor", "1", "", "", "text"),
                       target_opening=True)
        typo_features = extract_context_features(lone, CONTEXT_TYPO_FEATURE_VERSION)
        self.assertFalse([name for name in typo_features if name.startswith("opening:")])
        features = extract_context_features(lone, CONTEXT_OPENING_FEATURE_VERSION)
        self.assertEqual(features["opening:0:1"], 1.0)
        self.assertEqual(features["opening:0:1:alone:1:direction:0"], 1.0)
        # Schema 6 keeps the typo evidence of schema 5.
        self.assertIn("typo:0:0:known:0:0", features)
        after = replace(lone, field=FieldContext("editor", "1", "привет ", "", "text"))
        self.assertEqual(extract_context_features(after, CONTEXT_OPENING_FEATURE_VERSION)["opening:0:1:alone:0:direction:0"], 1.0)
        self.save({**self.payload(), "feature_version": CONTEXT_OPENING_FEATURE_VERSION})
        self.assertEqual(ContextModel.load(self.path).feature_version, CONTEXT_OPENING_FEATURE_VERSION)

    def test_the_planned_next_word_a_path_and_an_edit_in_place_belong_to_schema_six(self) -> None:
        planned = replace(self.item, field=FieldContext("editor", "1", "", "code", "text"),
                          after_origin="planned_next_conversion")
        features = extract_context_features(planned, CONTEXT_OPENING_FEATURE_VERSION)
        self.assertEqual(features["next:word:code"], 1.0)
        self.assertEqual(features["next:script:en:direction:0"], 1.0)
        self.assertFalse([name for name in features if name.startswith("after:")])
        # The same word already in the field is text after the caret.
        field = replace(planned, after_origin="field")
        self.assertEqual(extract_context_features(field, CONTEXT_OPENING_FEATURE_VERSION)["after:word:code"], 1.0)
        # Schema 5 reads a planned word as text after the caret, as it was trained.
        self.assertEqual(extract_context_features(planned, CONTEXT_TYPO_FEATURE_VERSION)["after:word:code"], 1.0)
        path = replace(self.item, field=FieldContext("editor", "1", "код/", "", "text"))
        self.assertEqual(extract_context_features(path, CONTEXT_OPENING_FEATURE_VERSION)["before:slash"], 1.0)
        self.assertEqual(extract_context_features(path, CONTEXT_OPENING_FEATURE_VERSION)["before:slash:known:0:0:direction:0"], 1.0)
        self.assertNotIn("before:slash", extract_context_features(path, CONTEXT_TYPO_FEATURE_VERSION))
        inside = replace(self.item, inside=True)
        self.assertEqual(extract_context_features(inside, CONTEXT_OPENING_FEATURE_VERSION)["inside"], 1.0)
        self.assertNotIn("inside", extract_context_features(inside, CONTEXT_TYPO_FEATURE_VERSION))
        self.assertNotIn("inside", extract_context_features(self.item, CONTEXT_OPENING_FEATURE_VERSION))

    def test_the_evidence_builder_marks_readings_that_open_sentences(self) -> None:
        self.assertTrue(opens_sentences("ты", 1))
        self.assertTrue(opens_sentences("Мы", 1))
        self.assertFalse(opens_sentences("же", 1))
        self.assertFalse(opens_sentences("ns", 0))
        detector = LanguageDetector({0: LanguageModel("en_US", {"ns": 1}, "fixture", enable_spellcheck=False),
                                     1: LanguageModel("ru_RU", {"ты": 1}, "fixture", enable_spellcheck=False)})
        field = FieldContext("editor", "1", "", "", "text")
        evidence = evidence_for_decision(detector.decide("ns", {1: "ты"}, 0), "ты", 1, detector, field, "space")
        self.assertEqual((evidence.source_opening, evidence.target_opening), (False, True))
        typed = evidence_for_decision(detector.decide("ты", {0: "ns"}, 1), "ns", 0, detector, field, "space")
        self.assertEqual((typed.source_opening, typed.target_opening), (True, False))


class ContextEngineTests(InputIntegrityTests):
    """Inherited physical-editor harness; only context tests are collected."""


    def setUp(self) -> None:
        super().setUp()
        self.settings.set("detection.context_policy", "assist")

    def choose(self, action: ContextAction) -> None:
        scores = [0.0] * len(ACTIONS)
        scores[list(ACTIONS).index(action)] = DOMINANT_BIAS_WEIGHT
        self.engine.context_policy.model = ContextModel({"bias": tuple(scores), "app:testeditor": (0.0,) * len(ACTIONS)}, "context-v1-fixture")

    def test_context_keep_convert_shadow_and_off(self) -> None:
        self.choose("keep")
        self.type("ghbdtn ")
        self.assertEqual(self.backend.text, "ghbdtn ")
        self.assertEqual(self.engine.snapshot.context_action, "keep")
        for mode in ("shadow", "off"):
            self.reset_editor()
            self.settings.set("detection.context_policy", mode)
            self.type("ghbdtn ")
            self.assertEqual(self.backend.text, "привет ")
        self.settings.set("detection.context_policy", "assist")
        self.reset_editor()
        self.choose("convert")
        self.type("ghbdtn ")
        self.assertEqual(self.backend.text, "привет ")

    def test_bundled_trained_model_resolves_user_phrase_and_retains_code(self) -> None:
        model = self.engine.context_policy.model
        assert model is not None
        self.assertIn(model.feature_version, (*CONTEXT_V1_FEATURE_VERSIONS, CONTEXT_ACTION_FEATURE_VERSION))
        prefix = "context-v1-" if model.feature_version in CONTEXT_V1_FEATURE_VERSIONS else "context-v3-"
        self.assertTrue(model.version.startswith(prefix))
        # A lone curated letter converts at the start of a message on its own and
        # the layout follows, so the same physical keys now type the Russian word.
        self.type("e ")
        self.assertEqual(self.backend.text, "у ")
        self.assertEqual(self.backend.group, 1)
        self.type("этого ")
        self.assertEqual(self.backend.text, "у этого ")
        self.reset_editor()
        self.type("const value = e ")
        self.assertEqual(self.backend.text, "const value = e ")
        # A command name carries digits, and the model - not a class-wide veto - decides
        # both readings of it: the Russian keys are the command, the command stays itself.
        self.reset_editor()
        self.type("pm2 ")
        self.assertEqual(self.backend.text, "pm2 ")
        self.reset_editor(1)
        self.type("зь2 ")
        self.assertEqual(self.backend.text, "pm2 ")

    def test_model_conversion_must_still_spell_a_word(self) -> None:
        """Whichever layer asks, a word is never replaced by something that is not one.

        Six Russian letters sit on punctuation keys, so the Latin reading of a
        Russian abbreviation such as ``збс`` is ``p,c``. The detector's own
        conversions already pass ``word_shape_veto``; the model's did not, and
        the bundled model converts ``збс`` with p=0.997 in every application it
        knows. The refusal is logged as a safety decision, not as the model's.
        """

        self.reset_editor(1)
        self.type("збс ")
        self.assertEqual(self.backend.text, "збс ")
        self.choose("convert")
        self.reset_editor(1)
        self.type("збс ")
        self.assertEqual(self.backend.text, "збс ")
        result = self.engine._context_result
        assert result is not None
        self.assertEqual(result.decision_source, "safety")
        self.assertEqual(result.fallback_reason, "replacement_not_a_word")
        self.assertFalse(result.decision.should_convert)
        # Letters joined by single dots are a name the model may restore: a domain or an address.
        self.reset_editor(1)
        self.type("учфьздуюсщь ")
        self.assertEqual(self.backend.text, "example.com ")
        self.reset_editor(1)
        self.type("ыфьздуююсщь ")
        self.assertEqual(self.backend.text, "ыфьздуююсщь ")

    def test_explicit_rules_exclusions_and_manual_layout_are_above_model(self) -> None:
        self.choose("convert")
        self.settings.set("exclusions.words", ["ghbdtn"])
        self.type("ghbdtn ")
        self.assertEqual(self.backend.text, "ghbdtn ")
        self.reset_editor()
        self.settings.set("exclusions.words", [])
        self.engine.learning.add_rule(LearnedRule("ghbdtn", 0, 1, "keep"))
        self.type("ghbdtn ")
        self.assertEqual(self.backend.text, "ghbdtn ")
        self.reset_editor()
        self.choose("keep")
        self.engine.learning.add_rule(LearnedRule("ghbdtn", 0, 1))
        self.type("ghbdtn ")
        self.assertEqual(self.backend.text, "привет ")

    def test_no_cross_field_or_disabled_context(self) -> None:
        self.choose("keep")
        self.type("hello ")
        self.assertEqual(self.engine.context_policy.stream.text, "hello ")
        self.backend.window = SECOND_WINDOW_ID
        self.type("next")
        self.assertEqual(self.engine.context_policy.stream.text, "next")
        self.settings.set("detection.context_policy", "off")
        self.assertEqual(self.engine.context_policy.stream.text, "")
        self.type(" word")
        self.assertEqual(self.engine.context_policy.stream.text, "")
        self.settings.set("detection.context_policy", "assist")
        self.settings.set("exclusions.applications", ["TestEditor"])
        self.type("secret")
        self.assertEqual(self.engine.context_policy.stream.text, "")

    def test_worker_with_injected_reader_leaves_its_lifecycle_to_owner(self) -> None:
        self.engine.context_policy.reader = None
        self.engine._run()

    def test_wait_uses_next_word_without_losing_spaces_or_undo(self) -> None:
        self.choose("wait")
        self.type("yt ")
        self.assertIsNotNone(self.engine._context_waiting)
        self.choose("convert")
        self.type("'njuj ")
        self.assertEqual(self.backend.text, "не этого ")
        correction = self.engine._last_correction
        assert correction is not None
        self.assertEqual(correction.mode, "context_phrase")
        self.tap(replace(self.key("z"), state=CONTROL_MASK | ALT_MASK))
        self.assertEqual(self.backend.text, "yt 'njuj ")

    def test_wait_is_cancelled_on_edit_navigation_and_timeout(self) -> None:
        for reason in ("BackSpace", "Left", "Pointer", "timeout", "space"):
            self.reset_editor()
            self.choose("wait")
            self.type("e ")
            if reason == "timeout":
                waiting = self.engine._context_waiting
                assert waiting is not None
                self.engine._context_waiting = replace(waiting, deadline=0.0)
            else:
                self.tap(self.key(reason, " " if reason == "space" else ""))
            self.choose("convert")
            self.type("'njuj ")
            last = self.engine._last_correction
            self.assertTrue(last is None or last.mode != "context_phrase")

    def test_suggestion_does_not_change_text_and_context_log_is_redacted(self) -> None:
        self.choose("keep")
        self.type("private sentence ")
        self.choose("suggest")
        with self.assertLogs("keyswitch.engine", level="INFO") as output:
            self.type("ghbdtn ")
        context_lines = [line for line in output.output if '"context_decision"' in line]
        self.assertTrue(context_lines)
        self.assertNotIn("private", "\n".join(context_lines))
        self.assertNotIn("sentence", "\n".join(context_lines))
        self.assertTrue(self.backend.text.endswith("ghbdtn "))
        self.assertIn("Pause", self.engine.snapshot.last_action)

    def test_wait_keeps_previous_word_when_second_inference_is_uncertain(self) -> None:
        self.choose("wait")
        self.type("yt ")
        waiting = self.engine._context_waiting
        assert waiting is not None
        next_word = self.engine.detector.decide("'njuj", {1: "этого"}, 0)
        results = [ContextResult(next_word), ContextResult(replace(waiting.decision, should_convert=False))]
        with patch.object(self.engine.context_policy, "decide", side_effect=results):
            self.type("'njuj ")
        self.assertEqual(self.backend.text, "yt этого ")


# Avoid re-running the inherited historical matrix here; that matrix keeps
# its own class and source module. The methods remain available as a harness.
def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    suite = unittest.TestSuite()
    for case in (InputContextTests, ContextModelTests, ContextEngineTests):
        for name in case.__dict__:
            if name.startswith("test_"):
                suite.addTest(case(name))
    return suite
