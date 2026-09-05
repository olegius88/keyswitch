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
from unittest.mock import patch

from keyswitch.backend import CONTROL_MASK, KeyEvent
from keyswitch.context_model import (
    ACTIONS, FEATURE_VERSION, ContextAction, ContextEvidence, ContextModel,
    extract_context_features, softmax,
)
from keyswitch.context_policy import ContextPolicy, ContextResult
from keyswitch.engine import KeySwitchEngine
from keyswitch.input_context import CONTEXT_LIMIT, FieldContext, InputContext
from test_input_integrity import InputIntegrityTests


class InputContextTests(unittest.TestCase):
    def setUp(self) -> None:
        self.stream = InputContext()
        self.stream.focus("chat", 1)

    @staticmethod
    def event(character: str = "", name: str = "") -> KeyEvent:
        return KeyEvent(True, 10, name or character, character, (character, character), 0, 0, 1)

    def type(self, text: str) -> None:
        for character in text:
            self.stream.observe(self.event(character))

    def test_bounded_field_history_and_suffix_anchoring(self) -> None:
        self.type("а" * 700 + " привет")
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
        self.stream.focus("chat", 2)
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
        self.stream.updated_at = time.monotonic() - 46
        self.assertEqual(self.stream.before_word("stale"), "")
        self.type("new")
        self.assertEqual(self.stream.text, "new")

    def test_native_context_never_returns_sensitive_text(self) -> None:
        for field in (FieldContext("app", "field", "secret", "suffix", sensitive=True),
                      FieldContext("app", "field", "secret", "suffix", role="password")):
            bounded = field.bounded()
            self.assertTrue(bounded.sensitive)
            self.assertEqual((bounded.before, bounded.after), ("", ""))
        field = FieldContext("x" * 200, "y" * 200, "z" * 900, "w" * 900).bounded()
        self.assertEqual(len(field.before), CONTEXT_LIMIT)
        self.assertEqual(len(field.application), 128)


class ContextModelTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.path = Path(self.temporary.name) / "model.json"
        self.item = ContextEvidence("ghbdtn", "привет", 0, FieldContext("Telegram", "1", "я думаю ", "", "text"), baseline_convert=True)

    def payload(self) -> dict[str, object]:
        weights = {"bias": [0.0, 8.0, 0.0, 0.0]}
        checksum = hashlib.sha256(json.dumps(weights, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()
        return {"feature_version": FEATURE_VERSION, "actions": list(ACTIONS), "weights": weights, "weights_sha256": checksum, "version": "context-v1-test", "conversion_threshold": 0.985}

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
            {"weights_sha256": "bad"}, {"version": 3},
            {"conversion_threshold": 0.5}, {"conversion_threshold": True},
        )
        for updates in updates_list:
            with self.subTest(updates=updates):
                self.save({**self.payload(), **updates})
                with self.assertRaises(ValueError):
                    ContextModel.load(self.path)
        self.save([])
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
        self.assertAlmostEqual(sum(softmax([10000.0, 9999.0])), 1.0)
        self.assertEqual(len(extract_context_features(replace(self.item, original="", alternative=""))) > 0, True)
        rich = replace(self.item, original="a_2", alternative="ф_2", field=FieldContext("test", "1", "// привет =", "hello"))
        self.assertEqual(extract_context_features(rich)["token:digits"], 1.0)


class ContextEngineTests(InputIntegrityTests):
    """Inherited physical-editor harness; only context tests are collected."""

    def setUp(self) -> None:
        super().setUp()
        self.settings.set("detection.context_policy", "assist")

    def choose(self, action: ContextAction) -> None:
        scores = [0.0] * 4
        scores[list(ACTIONS).index(action)] = 20.0
        self.engine.context_policy.model = ContextModel({"bias": tuple(scores), "app:testeditor": (0.0,) * 4}, "context-v1-fixture")

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
        self.assertTrue(model.version.startswith("context-v1-"))
        self.type("e ")
        self.assertEqual(self.backend.text, "e ")
        self.assertEqual(self.engine.snapshot.context_action, "wait")
        self.type("'njuj ")
        self.assertEqual(self.backend.text, "у этого ")
        self.reset_editor()
        self.type("const value = e ")
        self.assertEqual(self.backend.text, "const value = e ")

    def test_explicit_rules_exclusions_and_manual_layout_are_above_model(self) -> None:
        self.choose("convert")
        self.settings.set("exclusions.words", ["ghbdtn"])
        self.type("ghbdtn ")
        self.assertEqual(self.backend.text, "ghbdtn ")
        self.reset_editor()
        self.settings.set("exclusions.words", [])
        self.type("pm2 ")
        self.assertEqual(self.backend.text, "pm2 ")
        self.reset_editor()
        self.engine.learning.reject(0, "ghbdtn", 1)
        self.type("ghbdtn ")
        self.assertEqual(self.backend.text, "ghbdtn ")
        self.reset_editor()
        self.choose("keep")
        self.engine.learning.confirm_manual(0, "ghbdtn", 1, 2)
        self.type("ghbdtn ")
        self.assertEqual(self.backend.text, "привет ")

    def test_no_cross_field_or_disabled_context(self) -> None:
        self.choose("keep")
        self.type("hello ")
        self.assertEqual(self.engine.context_policy.stream.text, "hello ")
        self.backend.window = 2
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
        self.type("e ")
        self.assertIsNotNone(self.engine._context_waiting)
        self.choose("convert")
        self.type("'njuj ")
        self.assertEqual(self.backend.text, "у этого ")
        correction = self.engine._last_correction
        assert correction is not None
        self.assertEqual(correction.mode, "context_phrase")
        self.tap(replace(self.key("z"), state=CONTROL_MASK | 8))
        self.assertEqual(self.backend.text, "e 'njuj ")

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
        self.type("e ")
        waiting = self.engine._context_waiting
        assert waiting is not None
        next_word = self.engine.detector.decide("'njuj", {1: "этого"}, 0)
        results = [ContextResult(next_word), ContextResult(replace(waiting.decision, should_convert=False))]
        with patch.object(self.engine.context_policy, "decide", side_effect=results):
            self.type("'njuj ")
        self.assertEqual(self.backend.text, "e этого ")


# Avoid re-running the inherited historical matrix here; that matrix keeps
# its own class and source module. The methods remain available as a harness.
def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    suite = unittest.TestSuite()
    for case in (InputContextTests, ContextModelTests, ContextEngineTests):
        for name in case.__dict__:
            if name.startswith("test_"):
                suite.addTest(case(name))
    return suite
