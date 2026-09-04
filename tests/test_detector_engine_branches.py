"""Branch-heavy tests for decisions and the keyboard state machine."""

from __future__ import annotations

import json
import logging
import queue
import tempfile
import threading
import time
import unittest
from collections.abc import Callable, Iterable
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, patch

from keyswitch import detector as detector_module
from keyswitch import layouts
from keyswitch.config import SettingsStore
from keyswitch.detector import DetectionDecision, LanguageDetector
from keyswitch.engine import (
    CorrectionPlan,
    EngineSnapshot,
    Hotkey,
    KeySwitchEngine,
    LanguageContext,
    LearningPrompt,
)
from keyswitch.history import HistoryStore
from keyswitch.intent_model import CorrectionTrigger, IntentModelInput, LinearPrediction
from keyswitch.language_model import WordScore
from keyswitch.layouts import LayoutPair
from keyswitch.short_words import trusted_short_word_decision
from keyswitch.x11_backend import (
    CONTROL_MASK,
    LOCK_MASK,
    MOD1_MASK,
    MOD4_MASK,
    SHIFT_MASK,
    BackendProbe,
    KeyEvent,
)


def score(
    value: float,
    *,
    known: bool = False,
    exact: bool = False,
    spell: bool = False,
    ngram: float = -2.0,
    frequency: int | None = None,
) -> WordScore:
    return WordScore(
        value,
        known,
        (10 if exact else 0) if frequency is None else frequency,
        0.5,
        exact,
        spell,
        ngram,
        0.5,
        ngram,
    )


class StubModel:
    def __init__(self, values: dict[str, WordScore], *, context: dict[tuple[str, str], float] | None = None) -> None:
        self.values = values
        self.context = context or {}
        self.deletions: dict[str, WordScore] = {}

    def score(self, word: str) -> WordScore:
        return self.values.get(word, score(-5.0))

    def context_score(self, previous: str, word: str) -> float:
        return self.context.get((previous, word), 0.0)

    def best_single_deletion(self, word: str) -> WordScore:
        return self.deletions.get(word, score(-5.0))


class StubIntentClassifier:
    def __init__(
        self,
        prediction: LinearPrediction,
        *,
        veto_threshold: float = -3.0,
    ) -> None:
        self.prediction = prediction
        self.veto_threshold = veto_threshold
        self.inputs: list[IntentModelInput] = []

    def predict(self, item: IntentModelInput) -> LinearPrediction:
        self.inputs.append(item)
        return self.prediction


class DetectorBranchTests(unittest.TestCase):
    def test_protected_token_resource_read_failure_is_safe(self) -> None:
        with patch.object(Path, "read_text", side_effect=OSError("missing resource")):
            self.assertEqual(detector_module._load_protected_tokens(), frozenset())

    def detector(self, source: WordScore, target: WordScore) -> tuple[LanguageDetector, StubModel, StubModel]:
        left = StubModel({"source": source})
        right = StubModel({"target": target})
        return LanguageDetector({0: left, 1: right}), left, right

    def decide(
        self,
        detector: LanguageDetector,
        *,
        original: str = "source",
        alternatives: dict[int, str] | None = None,
        source_group: int = 0,
        minimum_length: int = 3,
        confidence_threshold: float = 2.0,
        ignored_words: set[str] | None = None,
        aggressive: bool = False,
        context_group: int | None = None,
        forced_target_group: int | None = None,
        rejected_targets: set[int] | None = None,
        trigger: CorrectionTrigger = "space",
    ) -> DetectionDecision:
        return detector.decide(
            original,
            {1: "target"} if alternatives is None else alternatives,
            source_group,
            minimum_length=minimum_length,
            confidence_threshold=confidence_threshold,
            ignored_words=ignored_words,
            aggressive=aggressive,
            context_group=context_group,
            forced_target_group=forced_target_group,
            rejected_targets=rejected_targets,
            trigger=trigger,
        )

    def test_requires_two_models_and_handles_no_alternative(self) -> None:
        with self.assertRaises(ValueError):
            LanguageDetector({0: StubModel({})})
        detector, _left, _right = self.detector(score(0), score(1))
        decision = self.decide(detector, alternatives={0: "source", 2: "other", 1: "source"})
        self.assertEqual(decision.reason, "нет другой раскладки")

    def test_early_guards_and_forced_rules(self) -> None:
        detector, _left, _right = self.detector(score(-5), score(8, known=True, exact=True, ngram=1))
        self.assertEqual(self.decide(detector, minimum_length=20).reason, "короткое слово")
        self.assertEqual(self.decide(detector, ignored_words={" Source "}).reason, "исключение пользователя")
        self.assertEqual(self.decide(detector, rejected_targets={1}).reason, "отклонённое пользователем исправление")
        forced = self.decide(detector, forced_target_group=1)
        self.assertTrue(forced.should_convert)
        self.assertGreaterEqual(forced.confidence, 20.0)
        not_found = self.decide(detector, forced_target_group=9)
        self.assertTrue(not_found.should_convert)
        protected = self.decide(detector, original="https://host", alternatives={1: "target"})
        self.assertEqual(protected.reason, "код, адрес или аббревиатура")

    def test_trusted_short_words_require_curated_exact_dominant_target(self) -> None:
        source = StubModel(
            {
                "ша": score(5.0, known=True, exact=True, frequency=531),
                "щл": score(-4.0),
                "аб": score(-4.0),
            }
        )
        target = StubModel(
            {
                "if": score(
                    4.5,
                    known=True,
                    exact=True,
                    spell=True,
                    frequency=464_324,
                ),
                "ok": score(4.0, known=True, spell=True),
                "zz": score(8.0, known=True, exact=True, frequency=1_000_000),
            }
        )
        detector = LanguageDetector({0: target, 1: source})

        trusted = trusted_short_word_decision(
            detector,
            "ша",
            {1: "ша", 2: "if", 0: "if"},
            1,
            ignored_words=(),
            rejected_targets=set(),
            protect_code=True,
        )
        self.assertIsNotNone(trusted)
        assert trusted is not None
        self.assertTrue(trusted.should_convert)
        self.assertEqual(trusted.replacement, "if")
        self.assertIn("безопасного списка", trusted.reason)

        protected = trusted_short_word_decision(
            detector,
            "ША",
            {0: "if"},
            1,
            ignored_words=(),
            rejected_targets=set(),
            protect_code=True,
        )
        self.assertIsNone(protected)

        ignored = trusted_short_word_decision(
            detector,
            "ша",
            {0: "if"},
            1,
            ignored_words=(" ША ",),
            rejected_targets=set(),
            protect_code=False,
        )
        self.assertIsNone(ignored)

        rejected = trusted_short_word_decision(
            detector,
            "ша",
            {0: "if"},
            1,
            ignored_words=(),
            rejected_targets={0},
            protect_code=True,
        )
        self.assertIsNone(rejected)

        not_exact = trusted_short_word_decision(
            detector,
            "щл",
            {0: "ok"},
            1,
            ignored_words=(),
            rejected_targets=set(),
            protect_code=True,
        )
        self.assertIsNone(not_exact)

        not_curated = trusted_short_word_decision(
            detector,
            "аб",
            {0: "zz"},
            1,
            ignored_words=(),
            rejected_targets=set(),
            protect_code=True,
        )
        self.assertIsNone(not_curated)

        target.values["if"] = score(
            8.0, known=True, exact=True, frequency=9_999
        )
        self.assertIsNone(
            trusted_short_word_decision(
                detector,
                "ша",
                {0: "if"},
                1,
                ignored_words=(),
                rejected_targets=set(),
                protect_code=True,
            )
        )
        target.values["if"] = score(
            8.0, known=True, exact=True, frequency=10_000
        )
        source.values["ша"] = score(
            5.0, known=True, exact=True, frequency=100
        )
        self.assertIsNone(
            trusted_short_word_decision(
                detector,
                "ша",
                {0: "if"},
                1,
                ignored_words=(),
                rejected_targets=set(),
                protect_code=True,
            )
        )
        target.values["if"] = score(
            8.0, known=True, exact=True, frequency=1_000_000
        )
        source.values["шаш"] = score(-4.0)
        self.assertIsNone(
            trusted_short_word_decision(
                detector,
                "шаш",
                {0: "if"},
                1,
                ignored_words=(),
                rejected_targets=set(),
                protect_code=True,
            )
        )
        self.assertIsNone(
            trusted_short_word_decision(
                detector,
                "ша",
                {0: "ша"},
                1,
                ignored_words=(),
                rejected_targets=set(),
                protect_code=True,
            )
        )

    def test_valid_source_guards_ambiguity(self) -> None:
        both, _left, _right = self.detector(
            score(8, known=True, exact=True, ngram=1), score(9, known=True, exact=True, ngram=1)
        )
        self.assertIn("обе раскладки", self.decide(both).reason)
        source_only, _left, _right = self.detector(
            score(8, known=True, exact=True, ngram=1), score(-2)
        )
        self.assertEqual(self.decide(source_only).reason, "исходное слово допустимо")

    def test_linear_model_is_residual_and_cannot_bypass_hard_guards(self) -> None:
        prediction = LinearPrediction(8.0, 0.999, 0.99, 1.0, True, "test-v1")
        model = StubIntentClassifier(prediction)
        left = StubModel({"source": score(8, known=True, exact=True, ngram=1)})
        right = StubModel({"target": score(9, known=True, exact=True, ngram=1)})
        detector = LanguageDetector({0: left, 1: right}, model)
        self.assertFalse(self.decide(detector).should_convert)
        self.assertEqual(model.inputs, [])
        protected = self.decide(
            detector,
            original="user_name",
            alternatives={1: "target"},
        )
        self.assertFalse(protected.should_convert)
        self.assertIn("код", protected.reason)
        self.assertEqual(model.inputs, [])

        short = LanguageDetector(
            {
                0: StubModel({"abcd": score(-5)}),
                1: StubModel({"фисв": score(-5)}),
            },
            model,
        ).decide(
            "abcd",
            {1: "фисв"},
            0,
            minimum_length=3,
            confidence_threshold=20.0,
        )
        self.assertFalse(short.should_convert)
        self.assertIsNone(short.model_probability)
        self.assertEqual(model.inputs, [])

    def test_linear_model_is_authoritative_after_hard_guards(self) -> None:
        veto = StubIntentClassifier(
            LinearPrediction(-6.0, 0.002, 0.98, 1.0, False, "test-v1"),
            veto_threshold=-4.0,
        )
        left = StubModel({"source": score(-8, ngram=-3)})
        right = StubModel({"target": score(8, known=True, exact=True, ngram=1)})
        detector = LanguageDetector({0: left, 1: right}, veto)
        rejected = self.decide(detector)
        self.assertFalse(rejected.should_convert)
        self.assertIn("безопасного порога", rejected.reason)
        self.assertEqual(rejected.model_probability, 0.002)
        self.assertEqual(rejected.model_threshold, 0.98)
        self.assertEqual(rejected.model_version, "test-v1")
        self.assertEqual(veto.inputs[0].trigger, "space")

        pause_gate = StubIntentClassifier(
            LinearPrediction(0.0, 0.5, 0.98, 1.0, False, "test-v1"),
            veto_threshold=-4.0,
        )
        detector = LanguageDetector({0: left, 1: right}, pause_gate)
        paused = self.decide(detector, trigger="pause")
        self.assertFalse(paused.should_convert)
        self.assertIn("безопасного порога", paused.reason)

        rescue = StubIntentClassifier(
            LinearPrediction(6.0, 0.998, 0.98, 1.0, True, "test-v1")
        )
        left = StubModel({"source": score(2, ngram=-2)})
        right = StubModel({"target": score(4, known=True, spell=True, ngram=0)})
        detector = LanguageDetector({0: left, 1: right}, rescue)
        recovered = self.decide(detector, confidence_threshold=3.0)
        self.assertTrue(recovered.should_convert)
        self.assertIn("линейной", recovered.reason)

    def test_linear_model_threshold_is_not_overridden_by_secondary_scores(self) -> None:
        unsupported = StubIntentClassifier(
            LinearPrediction(0.0, 0.5, 0.98, 1.0, False, "test-v1"),
            veto_threshold=-5.0,
        )
        left = StubModel({"source": score(-8, ngram=-2)})
        right = StubModel({"target": score(2, ngram=0)})
        detector = LanguageDetector({0: left, 1: right}, unsupported)
        abstained = self.decide(detector)
        self.assertFalse(abstained.should_convert)
        self.assertIn("безопасного порога", abstained.reason)

        low_coverage = StubIntentClassifier(
            LinearPrediction(8.0, 0.999, 0.98, 0.2, True, "test-v1")
        )
        fallback = LanguageDetector({0: left, 1: right}, low_coverage)
        low_coverage_result = self.decide(fallback)
        self.assertTrue(low_coverage_result.should_convert)
        self.assertIn("линейной", low_coverage_result.reason)

        rescue = StubIntentClassifier(
            LinearPrediction(8.0, 0.999, 0.98, 1.0, True, "test-v1")
        )
        left = StubModel({"sourcex": score(-2.5, ngram=-1.0)})
        right = StubModel({"targetx": score(0.0, ngram=-1.5)})
        detector = LanguageDetector({0: left, 1: right}, rescue)
        rescued = self.decide(
            detector,
            original="sourcex",
            alternatives={1: "targetx"},
        )
        self.assertTrue(rescued.should_convert)
        self.assertIn("линейной", rescued.reason)

        implausible_right = StubModel(
            {"targetx": score(0.0, ngram=-6.01)}
        )
        implausible = LanguageDetector(
            {0: left, 1: implausible_right}, rescue
        )
        rejected_target = self.decide(
            implausible,
            original="sourcex",
            alternatives={1: "targetx"},
        )
        self.assertTrue(rejected_target.should_convert)
        self.assertIn("линейной", rejected_target.reason)

        floor_right = StubModel({"targetx": score(0.0, ngram=-6.0)})
        at_floor = LanguageDetector({0: left, 1: floor_right}, rescue)
        self.assertTrue(
            self.decide(
                at_floor,
                original="sourcex",
                alternatives={1: "targetx"},
            ).should_convert
        )

        disabled = detector.decide(
            "sourcex",
            {1: "targetx"},
            0,
            use_intent_model=False,
        )
        self.assertIsNone(disabled.model_probability)
        self.assertEqual(len(rescue.inputs), 3)

    def test_exact_and_morphological_target_outcomes(self) -> None:
        exact, _left, _right = self.detector(score(-8, ngram=-4), score(8, known=True, exact=True, ngram=1))
        decision = self.decide(exact, confidence_threshold=9.0)
        self.assertTrue(decision.should_convert)
        self.assertIn("частотном", decision.reason)

        plausible, _left, _right = self.detector(score(-3, ngram=-2), score(4, known=True, spell=True, ngram=-4))
        decision = self.decide(plausible, confidence_threshold=2.0)
        self.assertTrue(decision.should_convert)
        self.assertIn("морфологическим", decision.reason)

        implausible, _left, _right = self.detector(score(-3, ngram=-2), score(4, known=True, spell=True, ngram=-5))
        decision = self.decide(implausible)
        self.assertFalse(decision.should_convert)
        self.assertIn("Редкая".casefold(), decision.reason.casefold())
        contextual = self.decide(implausible, context_group=1)
        self.assertTrue(contextual.should_convert)

        low_margin, _left, _right = self.detector(score(3.5, ngram=-2), score(4, known=True, spell=True, ngram=0))
        decision = self.decide(low_margin, confidence_threshold=3.0)
        self.assertFalse(decision.should_convert)
        self.assertIn("недостаточный", decision.reason)

    def test_typo_and_unknown_ngram_outcomes(self) -> None:
        detector, left, right = self.detector(score(-8, ngram=-3), score(-4, ngram=-1))
        left.deletions["source"] = score(-6)
        right.deletions["target"] = score(4, known=True, exact=True, ngram=1)
        typo = self.decide(detector)
        self.assertTrue(typo.should_convert)
        self.assertIn("опечатки", typo.reason)

        converting, _left, _right = self.detector(score(-8, ngram=-2), score(2, ngram=0))
        result = self.decide(converting)
        self.assertTrue(result.should_convert)
        self.assertIn("символьной", result.reason)

        source_natural, _left, _right = self.detector(score(0, ngram=0), score(2, ngram=0))
        self.assertIn("исходная", self.decide(source_natural).reason)
        target_bad, _left, _right = self.detector(score(-4, ngram=-2), score(2, ngram=-3))
        self.assertIn("целевая", self.decide(target_bad).reason)
        low, _left, _right = self.detector(score(-4, ngram=-2), score(-3, ngram=0))
        self.assertIn("недостаточная", self.decide(low).reason)

        aggressive = LanguageDetector(
            {
                0: StubModel({"sour": score(-4, ngram=-0.8)}),
                1: StubModel({"targ": score(2, ngram=-1.8)}),
            }
        )
        self.assertTrue(self.decide(aggressive, original="sour", alternatives={1: "targ"}, aggressive=True).should_convert)

    def test_context_scoring_and_best_of_multiple_candidates(self) -> None:
        left = StubModel({"source": score(-2)}, context={("before", "source"): 1.0})
        right = StubModel(
            {"weak": score(-1), "target": score(1, known=True, exact=True, ngram=1)},
            context={("prior", "target"): 2.0},
        )
        third = StubModel({"other": score(0)})
        detector = LanguageDetector({0: left, 1: right, 2: third})
        decision = detector.decide(
            "source",
            {1: "target", 2: "other"},
            0,
            previous_words={0: "before", 1: "prior"},
            context_group=1,
        )
        self.assertEqual(decision.target_group, 1)
        self.assertGreater(decision.confidence, 3.0)
        penalized = detector._context_delta(0, 1, "source", "target", {}, 0)
        self.assertEqual(penalized, -0.3)

    def test_structural_token_protection(self) -> None:
        with patch("keyswitch.detector.PROTECTED_TOKENS", frozenset({"reserved"})):
            protected = (
                "x" * 65,
                "reserved",
                "--force-with-lease",
                "-x",
                "www.example.org",
                "mail@example.org",
                "version2",
                "some/path",
                "ALLCAPS",
                "camelCase",
                "aaaa",
                "mixЖ",
            )
            for token in protected:
                with self.subTest(token=token):
                    self.assertTrue(LanguageDetector.is_protected_token(token))
        self.assertFalse(LanguageDetector.is_protected_token("ordinary"))
        self.assertEqual(LanguageDetector.token_key("  MiXeD, "), "mixed,")


class FakeBackend:
    def __init__(self) -> None:
        self.injections: list[tuple[tuple[KeyEvent, ...], int, KeyEvent | None, int | None]] = []
        self.group = 0
        self.application = "TestEditor"
        self.started = 0
        self.stopped = 0
        self.closed = 0
        self.start_error: Exception | None = None
        self.inject_error: Exception | None = None
        self.switch_error: Exception | None = None
        self.switches: list[int] = []

    def active_application(self) -> str:
        return self.application

    def focused_window(self) -> None:
        return None

    def current_group(self) -> int:
        return self.group

    def switch_group(self, group: int) -> None:
        if self.switch_error:
            raise self.switch_error
        self.switches.append(group)
        self.group = group

    def inject_correction(
        self,
        strokes: Iterable[KeyEvent],
        target_group: int,
        boundary: KeyEvent | None,
        source_group: int | None = None,
    ) -> None:
        if self.inject_error:
            raise self.inject_error
        self.injections.append((tuple(strokes), target_group, boundary, source_group))
        self.group = target_group

    def set_key_filter(
        self, predicate: Callable[[KeyEvent], bool] | None
    ) -> None:
        self.key_filter = predicate

    def start(self, listener: Callable[[KeyEvent], None]) -> None:
        self.started += 1
        self.listener = listener
        if self.start_error:
            raise self.start_error

    def stop(self) -> None:
        self.stopped += 1

    def close(self) -> None:
        self.closed += 1

    def probe(self) -> BackendProbe:
        return BackendProbe(True, "x11", ":test", "1", "2", "1", self.group)


PAIR = LayoutPair()


def letter(character: str, keycode: int = 30, group: int = 0, *, pressed: bool = True, state: int = 0) -> KeyEvent:
    if group == 0:
        characters = (character, PAIR.translate(character, "us", "ru"))
    else:
        characters = (PAIR.translate(character, "ru", "us"), character)
    return KeyEvent(pressed, keycode, characters[group], character, characters, group, state, keycode)


def key(name: str, keycode: int = 65, *, pressed: bool = True, character: str = "", state: int = 0, group: int = 0) -> KeyEvent:
    return KeyEvent(pressed, keycode, name, character, (character, character), group, state, keycode)


def released(event: KeyEvent) -> KeyEvent:
    return KeyEvent(False, event.keycode, event.key_name, event.character, event.characters, event.group, event.state, event.timestamp + 1)


class EngineBranchTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        root = Path(self.temporary.name)
        self.settings = SettingsStore(root / "config.json")
        # These fixtures type whole words at once; the prefix-based early
        # switch has dedicated tests and would otherwise fire on "ghbd".
        self.settings.set("detection.early_switch", False)
        self.history = HistoryStore(root / "history.jsonl")
        self.backend = FakeBackend()
        self.engine = KeySwitchEngine(self.settings, self.history, self.backend)

    def tearDown(self) -> None:
        if self.engine._running.is_set():
            self.engine.stop()
        self.temporary.cleanup()

    def type_word(self, text: str, group: int = 0) -> None:
        for index, character in enumerate(text, 30):
            event = letter(character, index, group)
            self.engine._handle(event)
            self.engine._handle(released(event))

    def test_subscriptions_settings_updates_and_snapshot_callback(self) -> None:
        snapshots: list[EngineSnapshot] = []
        corrections: list[CorrectionPlan] = []
        self.engine.subscribe(snapshots.append)
        self.engine.subscribe_corrections(corrections.append)
        self.assertEqual(len(snapshots), 1)
        self.settings.set("enabled", False)
        self.settings.set("appearance.theme", "dark")
        self.assertFalse(self.engine.snapshot.enabled)
        self.engine._manual_layout_group = 1
        self.settings.reset()
        self.assertTrue(self.engine.snapshot.enabled)
        self.assertIsNone(self.engine._manual_layout_group)
        self.engine._update(last_action="updated")
        self.assertEqual(snapshots[-1].last_action, "updated")

    def test_engine_model_toggle_and_trigger_are_applied_without_restart(self) -> None:
        classifier = StubIntentClassifier(
            LinearPrediction(8.0, 0.999, 0.98, 1.0, True, "test-v1")
        )
        self.engine.detector.intent_model = classifier
        self.settings.set("detection.intent_model_enabled", False)
        disabled = self.engine._decide_word(
            "ghbdtn", {1: "привет"}, 0, "Editor", "pause"
        )
        self.assertTrue(disabled.should_convert)
        self.assertEqual(classifier.inputs, [])
        self.settings.set("detection.intent_model_enabled", True)
        enabled = self.engine._decide_word(
            "ghbdtn", {1: "привет"}, 0, "Editor", "pause"
        )
        self.assertTrue(enabled.should_convert)
        self.assertEqual(classifier.inputs[0].trigger, "pause")

    def test_technical_logging_is_opt_in_structured_and_redacts_exclusions(self) -> None:
        with self.assertLogs("keyswitch.engine", logging.INFO) as captured:
            self.settings.set("diagnostics.technical_logging", True)
            self.type_word("ghbdtn")
            boundary = key("space", character=" ")
            self.engine._handle(boundary)
            self.engine._handle(released(boundary))

            self.settings.set("exclusions.applications", ["testeditor"])
            self.backend.group = 0
            self.engine._update(current_group=0)
            self.type_word("ghbdtn")
            self.engine._handle(boundary)
            self.engine._handle(released(boundary))
            excluded_plan = self.engine._last_committed
            self.assertIsNotNone(excluded_plan)
            assert excluded_plan is not None
            redacted_decision = self.engine._decide_word(
                "ghbdtn", {1: "привет"}, 0, "TestEditor"
            )
            self.engine._log_word_evaluation(
                trigger="pause",
                original="ghbdtn",
                alternatives={1: "привет"},
                application="TestEditor",
                enabled=True,
                trigger_enabled=True,
                manual_layout_protected=False,
                application_excluded=True,
                decision=redacted_decision,
            )
            self.engine._execute_correction(excluded_plan, None)

        payloads = [
            json.loads(record.getMessage().removeprefix("TECHNICAL "))
            for record in captured.records
        ]
        events = [str(payload["event"]) for payload in payloads]
        self.assertIn("setting_changed", events)
        self.assertIn("technical_logging_enabled", events)
        self.assertIn("word_evaluation", events)
        self.assertIn("correction_applied", events)
        evaluated = [
            payload for payload in payloads if payload["event"] == "word_evaluation"
        ]
        decision = evaluated[0]["decision"]
        self.assertIsInstance(decision, dict)
        assert isinstance(decision, dict)
        self.assertTrue(decision["should_convert"])
        self.assertIn("source_score", decision)
        self.assertEqual(evaluated[-1]["original"], "<redacted>")
        self.assertEqual(evaluated[-1]["alternatives"], {})
        redacted_decision_payload = evaluated[-1]["decision"]
        self.assertIsInstance(redacted_decision_payload, dict)
        assert isinstance(redacted_decision_payload, dict)
        self.assertEqual(
            redacted_decision_payload["replacement"], "<redacted>"
        )
        excluded_corrections = [
            payload
            for payload in payloads
            if payload["event"] == "correction_applied"
            and payload["application_excluded"] is True
        ]
        self.assertEqual(excluded_corrections[0]["original"], "<redacted>")
        self.assertEqual(excluded_corrections[0]["replacement"], "<redacted>")

        root = Path(self.temporary.name) / "logging-enabled"
        settings = SettingsStore(root / "config.json")
        settings.set("diagnostics.technical_logging", True)
        with self.assertLogs("keyswitch.engine", logging.INFO) as initialized:
            KeySwitchEngine(settings, HistoryStore(root / "history.jsonl"), FakeBackend())
        initial_payload = json.loads(
            initialized.records[0].getMessage().removeprefix("TECHNICAL ")
        )
        self.assertEqual(initial_payload["event"], "engine_initialized")
        self.assertEqual(initial_payload["keyswitch_version"], "0.12.0")
        self.assertIn("minimum_length", initial_payload["detection_settings"])

    def test_start_stop_idempotence_and_backend_failure(self) -> None:
        self.engine.start()
        self.engine.start()
        self.assertEqual(self.backend.started, 1)
        self.assertTrue(self.engine.snapshot.running)
        self.engine.stop()
        self.assertEqual((self.backend.stopped, self.backend.closed), (1, 1))
        self.engine.stop()
        self.assertEqual(self.backend.closed, 2)

        broken = FakeBackend()
        broken.start_error = RuntimeError("record unavailable")
        other = KeySwitchEngine(self.settings, self.history, broken)
        with self.assertRaisesRegex(RuntimeError, "record unavailable"):
            other.start()
        self.assertIn("record unavailable", other.snapshot.last_error)
        other.stop()

    def test_enqueue_filters_synthetic_and_reports_overflow(self) -> None:
        synthetic = KeyEvent(True, 1, "a", "a", ("a", "ф"), 0, 0, 0, True)
        self.engine.enqueue(synthetic)
        self.assertTrue(self.engine._events.empty())
        with patch.object(self.engine._events, "put_nowait", side_effect=queue.Full):
            self.engine.enqueue(letter("a"))
        self.assertEqual(self.engine.snapshot.last_action, "Очередь ввода переполнена")

    def test_alternate_layout_selection_guards_queue_and_applies(self) -> None:
        self.assertFalse(self.engine.select_alternate_group())
        self.assertIn("не запущен", self.engine.snapshot.last_error)

        self.engine._running.set()
        self.engine._update(current_group=-1)
        self.assertFalse(self.engine.select_alternate_group())
        self.assertIn("не определена", self.engine.snapshot.last_error)

        self.engine._update(current_group=0)
        original_models = self.engine.models
        self.engine.models = {0: original_models[0]}
        self.assertFalse(self.engine.select_alternate_group())
        self.engine.models = original_models

        with patch.object(self.engine._events, "put_nowait", side_effect=queue.Full):
            self.assertFalse(self.engine.select_alternate_group())
        self.assertIn("переполнена", self.engine.snapshot.last_error)

        self.engine._handle(letter("a"))
        self.assertTrue(self.engine.select_alternate_group())
        self.engine._events.put_nowait(None)
        self.engine._run()
        self.assertEqual(self.backend.switches, [1])
        self.assertEqual(self.engine.snapshot.current_group, 1)
        self.assertEqual(self.engine.snapshot.current_word, "")
        self.assertEqual(self.engine._manual_layout_group, 1)
        self.assertIn("выбран из меню: RU", self.engine.snapshot.last_action)

    def test_alternate_layout_backend_error_and_disabled_manual_protection(self) -> None:
        self.backend.switch_error = RuntimeError("смена отклонена")
        self.engine._apply_layout_selection(1)
        self.assertIn("смена отклонена", self.engine.snapshot.last_error)
        self.assertIn("не переключён", self.engine.snapshot.last_action)

        self.backend.switch_error = None
        self.settings.set("detection.respect_manual_layout", False)
        self.engine._apply_layout_selection(0)
        self.assertEqual(self.backend.switches, [0])
        self.assertIsNone(self.engine._manual_layout_group)
        self.assertEqual(self.engine.snapshot.current_group, 0)
        self.assertEqual(self.engine.snapshot.last_error, "")

    def test_worker_polls_handles_errors_and_stops_at_sentinel(self) -> None:
        event = letter("a")
        self.engine._running.set()
        with (
            patch.object(self.engine._events, "get", side_effect=[queue.Empty, event, None]),
            patch.object(self.engine, "_poll_current_group") as poll,
            patch.object(self.engine, "_handle", side_effect=RuntimeError("bad event")),
        ):
            self.engine._run()
        poll.assert_called_once()
        self.assertIn("bad event", self.engine.snapshot.last_error)
        self.engine._running.clear()
        self.engine._run()

    def test_stop_survives_full_queue_and_current_worker(self) -> None:
        self.engine._running.set()
        self.engine._worker = threading.current_thread()
        with patch.object(self.engine._events, "put_nowait", side_effect=queue.Full):
            self.engine.stop()
        self.assertEqual(self.backend.stopped, 1)

    def test_hotkeys_modifiers_backspace_navigation_and_group_change(self) -> None:
        self.engine._handle(letter("a", group=0))
        self.engine._handle(letter("b", keycode=31, group=0))
        self.engine._handle(key("BackSpace", keycode=22))
        self.assertEqual(self.engine.snapshot.current_word, "a")
        self.engine._handle(key("BackSpace", keycode=22))
        self.assertEqual(self.engine.snapshot.current_word, "")
        self.engine._handle(key("BackSpace", keycode=22))

        self.engine._handle(letter("a", group=0))
        self.engine._handle(letter("ф", group=1))
        self.assertEqual(self.engine.snapshot.current_group, 1)
        self.engine._handle(key("Left", character=""))
        self.assertEqual(self.engine.snapshot.current_word, "")
        self.engine._handle(letter("a"))
        self.engine._handle(key("x", character="x", state=CONTROL_MASK))
        self.assertEqual(self.engine.snapshot.current_word, "")
        self.engine._handle(key("F1", character=""))

        toggle = key("p", state=CONTROL_MASK | MOD1_MASK)
        self.engine._handle(toggle)
        self.assertFalse(self.settings.get("enabled"))
        self.engine._handle(toggle)
        self.assertTrue(self.settings.get("enabled"))

    def test_pause_correction_guards_and_valid_word_path(self) -> None:
        def arm(text: str = "ghbdtn") -> None:
            self.engine._clear_word()
            self.type_word(text)
            self.engine._last_word_input_at = 10.0

        arm()
        self.settings.set("enabled", False)
        self.engine._maybe_correct_after_pause(now=12.0)
        self.assertFalse(self.engine._pause_correction_pending)
        self.settings.set("enabled", True)

        self.engine._pause_correction_pending = True
        self.engine._last_word_input_at = None
        self.engine._maybe_correct_after_pause(now=12.0)
        self.assertFalse(self.engine._pause_correction_pending)

        arm()
        self.engine._pressed.add(30)
        self.engine._maybe_correct_after_pause(now=12.0)
        self.assertTrue(self.engine._pause_correction_pending)
        self.engine._pressed.clear()

        self.engine._modifier_keycodes.add(37)
        self.engine._maybe_correct_after_pause(now=12.0)
        self.assertTrue(self.engine._pause_correction_pending)
        self.engine._modifier_keycodes.clear()

        self.engine._pending = CorrectionPlan(
            (letter("a"),), None, 0, 1, "a", "ф", 99, "Editor", False
        )
        self.engine._maybe_correct_after_pause(now=12.0)
        self.assertTrue(self.engine._pause_correction_pending)
        self.engine._pending = None

        self.engine._manual_layout_group = 0
        self.engine._maybe_correct_after_pause(now=12.0)
        self.assertFalse(self.engine._pause_correction_pending)
        self.engine._manual_layout_group = None

        arm()
        self.settings.set("exclusions.applications", ["testeditor"])
        self.engine._maybe_correct_after_pause(now=12.0)
        self.assertFalse(self.engine._pause_correction_pending)
        self.settings.set("exclusions.applications", [])

        arm("hello")
        with patch("keyswitch.engine.time.monotonic", return_value=12.0):
            self.engine._maybe_correct_after_pause()
        self.assertFalse(self.engine._pause_correction_pending)
        self.assertEqual(self.backend.injections, [])

    def test_modifier_release_defers_and_then_executes_pending(self) -> None:
        plan = CorrectionPlan((letter("a"),), None, 0, 1, "a", "ф", 99, "Editor", False)
        self.engine._pending = plan
        self.engine._pending_trigger_keycode = 127
        self.engine._modifier_keycodes.add(37)
        self.engine._maybe_execute_pending(key("Pause", 127, pressed=True))
        self.assertFalse(self.backend.injections)
        self.engine._maybe_execute_pending(key("Pause", 127, pressed=False))
        self.assertFalse(self.backend.injections)
        self.engine._handle(key("Control_L", 37, pressed=False))
        self.assertEqual(len(self.backend.injections), 1)

    def test_manual_conversion_without_word_and_without_target(self) -> None:
        self.engine._schedule_manual_conversion(127)
        self.assertIn("Нет слова", self.engine.snapshot.last_action)
        self.engine._strokes = [letter("a")]
        self.engine._source_group = 0
        original_models = self.engine.models
        self.engine.models = {0: original_models[0]}
        self.engine._schedule_manual_conversion(127)
        self.assertIsNone(self.engine._pending)
        self.engine.models = original_models

    def test_stale_undo_and_manual_undo_do_not_record_rejection(self) -> None:
        self.engine._schedule_undo(52)
        self.assertIn("нельзя отменить", self.engine.snapshot.last_action)
        plan = CorrectionPlan((letter("a"),), None, 0, 1, "a", "ф", 99, "Editor", False)
        self.engine._last_correction = plan
        self.engine._last_correction_time = time.monotonic()
        self.engine._schedule_undo(52)
        self.assertIsNone(self.engine._pending_learning_action)

    def test_injection_error_and_disabled_history_learning(self) -> None:
        callback = Mock()
        self.engine.subscribe_corrections(callback)
        plan = CorrectionPlan((letter("a"),), None, 0, 1, "a", "ф", 4, "Editor", True)
        self.engine._pending = plan
        self.engine._pending_trigger_keycode = -1
        self.backend.inject_error = RuntimeError("xtest failed")
        self.engine._maybe_execute_pending(key("x"))
        self.assertIn("xtest failed", self.engine.snapshot.last_error)
        callback.assert_not_called()

        self.backend.inject_error = None
        self.settings.set("general.keep_history", False)
        self.settings.set("detection.learning", False)
        self.engine._pending = plan
        self.engine._pending_learning_action = ("manual", 0, "a", 1)
        self.engine._pending_trigger_keycode = -1
        self.engine._maybe_execute_pending(key("x"))
        self.assertEqual(self.history.read(), [])
        callback.assert_called_once_with(plan)

    def test_learning_action_labels_manual_and_reject(self) -> None:
        self.settings.set("detection.learning_confirmations", 1)
        manual = CorrectionPlan((letter("q"),), None, 0, 1, "q", "й", 99, "Editor", False)
        self.engine._pending = manual
        self.engine._pending_learning_action = ("manual", 0, "q", 1)
        self.engine._pending_trigger_keycode = -1
        self.engine._maybe_execute_pending(key("x"))
        self.assertIn("правило выучено", self.engine.snapshot.last_action)

        automatic = CorrectionPlan((letter("a"),), None, 1, 0, "ф", "a", 99, "Editor", False)
        self.engine._pending = automatic
        self.engine._pending_learning_action = ("reject", 0, "source", 1)
        self.engine._pending_trigger_keycode = -1
        self.engine._maybe_execute_pending(key("x"))
        self.assertIn("ложное срабатывание", self.engine.snapshot.last_action)

        self.engine._pending = automatic
        self.engine._pending_learning_action = ("unknown", 0, "source", 1)
        self.engine._pending_trigger_keycode = -1
        self.engine._maybe_execute_pending(key("x"))

    def test_learning_prompt_confirmation_dismissal_and_expiry(self) -> None:
        callbacks: list[LearningPrompt | None] = []
        self.engine.subscribe_learning_prompts(callbacks.append)
        self.assertEqual(callbacks, [None])
        self.assertFalse(self.engine.confirm_learning_prompt())
        self.assertFalse(self.engine.dismiss_learning_prompt())
        self.assertFalse(self.engine._expire_learning_prompt(now=10.0))

        prompt = LearningPrompt(0, 1, "hello", "руддщ", "Editor")
        stale = LearningPrompt(0, 1, "world", "цщкдв", "Editor")
        self.engine._show_learning_prompt(prompt)
        self.assertIs(self.engine.learning_prompt, prompt)
        self.assertFalse(self.engine.confirm_learning_prompt(stale))
        self.assertFalse(self.engine.dismiss_learning_prompt(stale))
        deadline = self.engine._learning_prompt_deadline
        assert deadline is not None
        self.assertFalse(self.engine._expire_learning_prompt(now=deadline - 0.01))
        self.assertTrue(self.engine._expire_learning_prompt(now=deadline))
        self.assertEqual(callbacks[-1], None)

        self.engine._show_learning_prompt(prompt)
        self.engine._handle(key("Escape", keycode=9))
        self.assertIsNone(self.engine.learning_prompt)
        self.engine._show_learning_prompt(prompt)
        self.engine._handle(key("a", keycode=38, character="a"))
        self.assertIsNone(self.engine.learning_prompt)

        self.engine._show_learning_prompt(prompt)
        modifier = key("Control_L", keycode=37)
        self.engine._handle(modifier)
        self.assertIs(self.engine.learning_prompt, prompt)
        self.settings.set("detection.learning", False)
        self.assertIsNone(self.engine.learning_prompt)
        self.assertIsNone(self.engine._forced_target_group(0, "hello"))

    def test_manual_layout_protection_overrides_learned_rule_on_pause(self) -> None:
        self.engine.learning.confirm_manual(0, "hello", 1, 2)
        self.engine._manual_layout_group = 0
        self.engine._strokes = [
            letter(character, keycode)
            for keycode, character in enumerate("hello", start=30)
        ]
        self.engine._source_group = 0
        self.engine._pause_correction_pending = True
        self.engine._last_word_input_at = 1.0

        self.engine._maybe_correct_after_pause(now=3.0)

        self.assertEqual(self.backend.injections, [])
        self.assertEqual(self.engine._manual_layout_group, 0)
        self.assertFalse(self.engine._pause_correction_pending)

    def test_context_expiry_copy_and_lru_limit(self) -> None:
        strokes = (letter("a"),)
        self.engine._remember_context("", 0, strokes)
        self.engine._remember_context("Editor", 9, strokes)
        self.engine._remember_context("Editor", 0, ())
        self.assertEqual(self.engine._contexts, {})
        self.engine._remember_context("Editor", 0, strokes)
        words, group = self.engine._context_for("editor")
        words[0] = "mutated"
        self.assertEqual(group, 0)
        self.assertNotEqual(self.engine._contexts["editor"].words[0], "mutated")
        self.engine._contexts["editor"] = LanguageContext(0, {0: "a"}, time.monotonic() - 46)
        self.assertEqual(self.engine._context_for("Editor"), ({}, None))
        for index in range(34):
            self.engine._remember_context(f"App{index}", 0, strokes)
        self.assertEqual(len(self.engine._contexts), 32)

    def test_boundaries_exclusions_polling_and_clear_action(self) -> None:
        cases = (
            ("space", "detection.correct_on_space", "space"),
            ("Return", "detection.correct_on_enter", "enter"),
            ("Tab", "detection.correct_on_tab", "tab"),
            ("ISO_Left_Tab", "detection.correct_on_tab", "tab"),
            ("period", "detection.correct_on_punctuation", "punctuation"),
        )
        for name, setting, trigger in cases:
            event = key(name, character="." if name == "period" else "")
            self.assertTrue(self.engine._is_boundary(event))
            self.assertEqual(self.engine._trigger_for_boundary(event), trigger)
            self.settings.set(setting, False)
            self.assertFalse(self.engine._boundary_enabled(event))
            self.settings.set(setting, True)
        self.settings.set("exclusions.applications", ["", "secret"])
        self.assertTrue(self.engine._application_excluded("My Secret Editor"))
        self.assertFalse(self.engine._application_excluded("Terminal"))
        self.backend.group = 7
        self.engine._poll_current_group()
        self.backend.group = self.engine.snapshot.current_group
        self.engine._poll_current_group()
        self.engine._clear_word("cleared")
        self.assertEqual(self.engine.snapshot.last_action, "cleared")

    def test_commit_empty_and_ambiguous_apostrophe_paths(self) -> None:
        self.engine._commit_word(key("space", character=" "))
        event = letter("'", 48)
        self.engine._strokes = [letter("a")]
        self.engine._source_group = 0
        self.assertFalse(self.engine._ambiguous_key_is_boundary(event))
        self.engine._strokes.insert(0, letter(",", 59))
        self.assertFalse(self.engine._ambiguous_key_is_boundary(letter(",", 60)))


class HotkeyAndKeyEventBranchTests(unittest.TestCase):
    def test_no_key_release_aliases_and_all_modifiers(self) -> None:
        self.assertFalse(Hotkey("Ctrl").matches(key("Control_L")))
        self.assertFalse(Hotkey("Pause").matches(key("Pause", pressed=False)))
        self.assertTrue(Hotkey("Pause").matches(key("Break")))
        all_modifiers = SHIFT_MASK | CONTROL_MASK | MOD1_MASK | MOD4_MASK
        event = key("x", state=all_modifiers)
        self.assertTrue(Hotkey("Control+Alt+Shift+Meta+X").matches(event))
        self.assertTrue(event.shift and event.control and event.alt and event.super_key)
        locked = key("x", state=LOCK_MASK)
        self.assertTrue(locked.caps_lock)
        self.assertEqual(locked.character_for(9), "")

    def test_layout_validation_identity_and_unsupported_pair(self) -> None:
        with self.assertRaises(ValueError):
            LayoutPair("us", "de")
        pair = LayoutPair()
        self.assertEqual(pair.translate("text", "us", "us"), "text")
        with self.assertRaises(ValueError):
            pair.translate("text", "de", "ru")
        self.assertEqual(pair.translate("1🙂", "us", "ru"), "1🙂")

    def test_layout_tables_equal_character_by_character_mapping(self) -> None:
        """The precomputed tables must behave exactly like per-character lookup."""

        pair = LayoutPair()
        forward = pair.us_to_ru
        backward = pair.ru_to_us
        self.assertEqual(forward, layouts._case_aware_map(layouts.US_KEYS, layouts.RU_KEYS))
        self.assertEqual(backward, layouts._case_aware_map(layouts.RU_KEYS, layouts.US_KEYS))
        forward["x"] = "changed"
        self.assertNotEqual(pair.us_to_ru.get("x"), "changed")
        everything = "".join(
            chr(code) for code in range(0x10000) if not 0xD800 <= code <= 0xDFFF
        )
        for source, target, mapping in (
            ("us", "ru", pair.us_to_ru),
            ("ru", "us", pair.ru_to_us),
        ):
            expected = "".join(mapping.get(character, character) for character in everything)
            self.assertEqual(pair.translate(everything, source, target), expected)
        self.assertEqual(pair.translate("Ghbdtn", "us", "ru"), "Привет")
        self.assertEqual(pair.translate("Привет", "ru", "us"), "Ghbdtn")
        self.assertEqual(pair.translate("ß İ ~", "us", "ru"), "ß İ ~")


if __name__ == "__main__":
    unittest.main()
