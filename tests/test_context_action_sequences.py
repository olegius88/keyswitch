"""Authored physical replay and pre-read holdout enforcement regressions."""
from __future__ import annotations

from collections import Counter
from dataclasses import replace
import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile
from typing import cast
import unittest
from unittest.mock import Mock, patch

TOOLS = str(Path(__file__).resolve().parents[1] / "tools")
if TOOLS not in sys.path:
    sys.path.insert(0, TOOLS)

from keyswitch.context_model import ACTIONS, ContextModel
from keyswitch.backend import KeyEvent
from keyswitch.config import SettingsStore
from keyswitch.history import HistoryStore
from keyswitch.intent_model import IntentModelStatus, LinearNgramModel
from keyswitch.language_model import LanguageModel
from keyswitch.context_model import ContextPrediction
from keyswitch.early_switch import PrefixIndex
from keyswitch.prefix_model import PrefixInput, PrefixModel
from keyswitch.prefix_schema import VersionedPrefixModel
import evaluate_context_action_sequences as evaluator
from freeze_context_action_corpus import CorpusRow, canonical, checksum, digest


def row(original: str = "hello", group: int | None = 0, before: str = "", after: str = " ",
        *, identifier: str = "authored:1", document: str = "authored:document") -> CorpusRow:
    return CorpusRow(identifier, original, group, before, after, original.casefold(), digest(original),
                     document, "eng" if group == 0 else "rus", "authored", "fixtures", "sentence", "1",
                     " ", " ", " ", "NOUN", "_", "_", "authored", True, "development")


def authored_model(convert_russian: bool = False) -> ContextModel:
    # These transparent fixture weights exercise execution, not model quality.
    weights = {f"{label}:char:{group}:1:{character}": (0.0, 0.0, 0.0, 0.0)
               for label in ("source", "target") for group in (0, 1)
               for character in "abcdefghijklmnopqrstuvwxyzабвгдеёжзийклмнопрстуфхцчшщъыьэюя"}
    weights["bias"] = (20.0, 0.0, 0.0, 0.0)
    if convert_russian:
        weights["direction:1"] = (-40.0, 40.0, 0.0, 0.0)
    return ContextModel(weights, "context-v3-authored", feature_version=3)


def write_prefix_artifact(path: Path, *, weight: float = 10.0, threshold: float = 0.999, feature_version: int = 1) -> PrefixModel:
    weights = {"bias": [weight, 0.0, 0.0, 0.0]}
    fingerprint = hashlib.sha256(json.dumps(weights, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()
    path.write_bytes(canonical({"kind": "keyswitch.prefix-policy", "feature_version": feature_version,
                               "prefix_feature_version": feature_version, "actions": list(ACTIONS),
                               "version": f"prefix-v{feature_version}-" + fingerprint[:12], "conversion_threshold": threshold,
                               "weights_sha256": fingerprint, "weights": weights}))
    return PrefixModel.load(path)


def write_prefix_seal(path: Path, artifact: Path, provenance: dict[str, str], *, calibration_passed: bool = True) -> dict[str, object]:
    model = PrefixModel.load(artifact)
    value: dict[str, object] = {"schema_version": 1, "candidate_sha256": checksum(artifact), "model_version": model.version,
                                "threshold": model.model.conversion_threshold, "provenance": provenance, "recipe": {"fixture": True},
                                "calibration_passed": calibration_passed, "promotion_accepted": False, "independent_test_evaluated": False}
    path.write_bytes(canonical(value))
    return value


def authored_prefix(weight: float = 10.0) -> PrefixModel:
    return VersionedPrefixModel(ContextModel({"bias": (weight, 0.0, 0.0, 0.0)}, "prefix-v1-authored0000"), feature_version=1)


class PhysicalSequenceTests(unittest.TestCase):
    def test_replay_pins_packaged_intent_and_restores_discovery_scope(self) -> None:
        intent = cast(LinearNgramModel, Mock(spec=LinearNgramModel,
                      model_version="authored-intent", checksum="f" * 64))
        observed: list[tuple[LinearNgramModel | None, IntentModelStatus]] = []
        environments: list[str | None] = []

        class RecordingEngine:
            def __init__(self, settings: SettingsStore, history: HistoryStore,
                         backend: evaluator.TracedEditor) -> None:
                observed.append(LinearNgramModel.try_load_default())
                environments.append(os.environ.get("KEYSWITCH_INTENT_MODEL_PATH"))

            def _handle(self, event: KeyEvent) -> None:
                pass

            # Both settings modes drive the engine's idle callbacks.
            def _expire_deferred_action(self) -> None:
                pass

            def _expire_manual_correction(self) -> None:
                pass

            def _poll_current_group(self) -> None:
                pass

            def _maybe_correct_after_pause(self) -> None:
                pass

            def _expire_learning_prompt(self) -> None:
                pass

        previous = os.environ.get("KEYSWITCH_INTENT_MODEL_PATH")
        with tempfile.TemporaryDirectory(prefix="keyswitch-pinned-intent-") as temporary:
            root = Path(temporary)
            packaged = root / "src/keyswitch/resources/models/layout_intent_v1.ksm"
            packaged.parent.mkdir(parents=True)
            packaged.write_bytes(b"authored file, decoded by the fixture loader")
            outside = (None, IntentModelStatus(False, root / "override.ksm", None, None, "fixture"))
            with patch.object(evaluator, "ROOT", root), \
                    patch.object(evaluator, "KeySwitchEngine", RecordingEngine), \
                    patch.object(LinearNgramModel, "load", return_value=intent) as load, \
                    patch.object(LinearNgramModel, "try_load_default", return_value=outside), \
                    patch.dict(os.environ, {"KEYSWITCH_INTENT_MODEL_PATH": "external-one.ksm"}):
                plan = evaluator.sequence_plan(row("a", after=""), False)
                first = evaluator.replay(plan, authored_model(), {})
                self.assertEqual(LinearNgramModel.try_load_default(), outside)
                os.environ["KEYSWITCH_INTENT_MODEL_PATH"] = "external-two.ksm"
                second = evaluator.replay(plan, authored_model(), {})
                self.assertEqual(LinearNgramModel.try_load_default(), outside)
                self.assertEqual(first, second)
                self.assertEqual(first["actual"], "a")
                expected = (intent, IntentModelStatus(True, packaged, "authored-intent", "f" * 64, None))
                self.assertEqual(observed, [expected, expected])
                self.assertEqual(environments, ["external-one.ksm", "external-two.ksm"])
                load.assert_called_once_with(packaged)
        self.assertEqual(os.environ.get("KEYSWITCH_INTENT_MODEL_PATH"), previous)

    def test_replay_rejects_broken_packaged_intent_without_external_fallback(self) -> None:
        with tempfile.TemporaryDirectory(prefix="keyswitch-broken-intent-") as temporary:
            root = Path(temporary)
            packaged = root / "src/keyswitch/resources/models/layout_intent_v1.ksm"
            packaged.parent.mkdir(parents=True)
            packaged.write_bytes(b"invalid fixture")
            with patch.object(evaluator, "ROOT", root), \
                    patch.object(LinearNgramModel, "load", side_effect=ValueError("invalid packaged KSLM")), \
                    patch.object(LinearNgramModel, "try_load_default") as discovery, \
                    patch.object(evaluator, "KeySwitchEngine") as engine:
                with self.assertRaisesRegex(ValueError, "invalid packaged KSLM"):
                    evaluator.replay(evaluator.sequence_plan(row(), False), authored_model(), {})
                discovery.assert_not_called()
                engine.assert_not_called()

    def test_shift_pairs_and_layout_specific_punctuation(self) -> None:
        for us, ru in (("@", '"'), ("#", "№"), ("$", ";"), ("^", ":"), ("&", "?"),
                       ("?", ","), ("{", "Х"), ("}", "Ъ"), ("~", "Ё"), ("|", "/")):
            with self.subTest(us=us):
                self.assertIn((us, ru), [key.characters for key in evaluator.KEYS if key.shift])
        plan = evaluator.sequence_plan(row("Hello", 0, '"', ',  мир! '), False)
        self.assertFalse(plan.unsupported)
        group = 0
        visible = ""
        for key in plan.keys:
            if key.explicit_group is not None:
                group = key.explicit_group
            visible += key.characters[group]
        self.assertEqual(visible, '"Hello,  мир! ')

    def test_metadata_does_not_duplicate_spaces_or_literal_tail(self) -> None:
        source = replace(row("hello", before="Он  ", after=",  world! "),
                         spacing="  ", space_before="  ", literal_tail=",  ")
        plan = evaluator.sequence_plan(source, False)
        self.assertEqual(plan.expected, "Он  hello,  world! ")
        self.assertEqual(len(plan.keys), len(plan.expected))

    def test_clipped_outer_fragments_are_trimmed_before_replay(self) -> None:
        source = row(before="fragment " + "a " * 43 + "x", after=" z" + "a" * 62)
        self.assertEqual(len(source.before), 96)
        self.assertEqual(len(source.after), 64)
        plan = evaluator.sequence_plan(source, False)
        self.assertEqual((plan.trimmed_left, plan.trimmed_right), (8, 63))
        self.assertEqual(plan.expected, source.before[8:] + "hello ")
        unbounded = evaluator.sequence_plan(row(before="fragment ", after=" tail"), False)
        self.assertEqual((unbounded.trimmed_left, unbounded.trimmed_right), (0, 0))

    def test_wrong_focus_is_not_reset_on_each_character_or_same_script_suffix(self) -> None:
        plan = evaluator.sequence_plan(row("hello", after=" world "), True)
        self.assertEqual(plan.keys[0].explicit_group, 1)
        self.assertTrue(all(key.explicit_group is None for key in plan.keys[1:]))
        mixed = evaluator.sequence_plan(row("hello", after=" мир "), True)
        self.assertEqual([key.explicit_group for key in mixed.keys if key.explicit_group is not None], [1, 1])
        with self.assertRaisesRegex(ValueError, "unique wrong"):
            evaluator.sequence_plan(row("123", None), True)

    def test_wrong_focus_preserves_literal_prefix_and_corrupts_first_letter(self) -> None:
        for original, group in (("№hello", 0), ('"hello', 0), ("Эхо", 1), ('"Эхо', 1),
                                (".dist", 0), ("don't", 0), ("h№ello", 0)):
            with self.subTest(original=original):
                source = row(original, group)
                actual: dict[bool, str] = {}
                for wrong in (False, True):
                    plan = evaluator.sequence_plan(source, wrong)
                    self.assertFalse(plan.unsupported)
                    current, text = 0, ""
                    for key in plan.keys:
                        if key.explicit_group is not None:
                            current = key.explicit_group
                        text += key.characters[current]
                    actual[wrong] = text
                first_letter = next(index for index, character in enumerate(original)
                                    if evaluator.script_group(character) == group)
                self.assertEqual(actual[False], original + " ")
                self.assertEqual(actual[True][:first_letter], original[:first_letter])
                self.assertNotEqual(actual[True][first_letter], original[first_letter])
                self.assertEqual(evaluator.sequence_plan(source, True).intervention_event, first_letter + 1)

    def test_nonletters_have_no_wrong_letter_intervention(self) -> None:
        for original in ("123", '"№.', ""):
            with self.subTest(original=original):
                with self.assertRaisesRegex(ValueError, "focus letter"):
                    evaluator.sequence_plan(row(original, 0), True)
                self.assertFalse(evaluator.sequence_plan(row(original, None), False).unsupported)

    def test_live_editor_observes_effective_wrong_intervention_after_prefix(self) -> None:
        models = {
            0: LanguageModel("en_US", {"hello": 5000, "dist": 4000}, "authored", enable_spellcheck=False),
            1: LanguageModel("ru_RU", {"эхо": 5000}, "authored", enable_spellcheck=False),
        }
        for original, group in (("№hello", 0), ('"hello', 0), ("Э", 1), ('"Эхо', 1), (".dist", 0), ("don't", 0)):
            with self.subTest(original=original):
                plan = evaluator.sequence_plan(row(original, group), True)
                result = evaluator.replay(plan, authored_model(), models)
                intervention = cast(dict[str, object], result["intervention"])
                self.assertIsNone(result["error"])
                self.assertEqual(intervention["event"], plan.intervention_event)
                self.assertEqual(intervention["target_group"], group)
                self.assertEqual(intervention["source_group"], 1 - group)
                self.assertNotEqual(intervention["observed"], intervention["expected"])
        plan = evaluator.sequence_plan(row(), True)
        changed = replace(plan, keys=(replace(plan.keys[0], explicit_group=0), *plan.keys[1:]))
        result = evaluator.replay(changed, authored_model(), models)
        self.assertIn("did not change", str(result["error"]))
        for invalid in (replace(plan, intervention_event=None), replace(plan, intervention_event=999)):
            with self.assertRaisesRegex(ValueError, "intervention"):
                evaluator.replay(invalid, authored_model(), models)

    def test_default_mode_reaches_the_injected_prefix_model_and_early_off_does_not(self) -> None:
        models = {
            0: LanguageModel("en_US", {"hello": 5000, "world": 4000}, "authored", enable_spellcheck=False),
            1: LanguageModel("ru_RU", {"привет": 5000, "мир": 4000}, "authored", enable_spellcheck=False),
        }
        calls: list[int] = []

        class Recording(VersionedPrefixModel):
            def predict(self, item: PrefixInput, indexes: dict[int, PrefixIndex],
                        language_models: dict[int, LanguageModel]) -> ContextPrediction:
                calls.append(len(item.original))
                return super().predict(item, indexes, language_models)

        prefix = Recording(ContextModel({"bias": (10.0, 0.0, 0.0, 0.0)}, "prefix-v1-authored0000"), feature_version=1)
        plan = evaluator.sequence_plan(row("привет", 1, before="", after=" мир "), True)
        with patch.object(ContextModel, "load", side_effect=AssertionError("installed context must not be read")), \
                patch.object(PrefixModel, "load", side_effect=AssertionError("installed prefix must not be read")):
            early = evaluator.replay(plan, authored_model(), models, prefix=prefix)
            self.assertEqual(calls, [])
            default = evaluator.replay(plan, authored_model(), models, prefix=prefix, mode="default")
        self.assertIsNone(early["error"])
        self.assertIsNone(default["error"])
        self.assertTrue(calls)
        self.assertGreaterEqual(min(calls), 4)
        self.assertEqual(default["mode"], "default")

    def test_default_mode_requires_one_key_per_expected_character(self) -> None:
        plan = evaluator.sequence_plan(row("hi", after=""), False)
        mismatched = replace(plan, expected=plan.expected + "!")
        with patch.object(evaluator, "KeySwitchEngine") as engine:
            with self.assertRaisesRegex(ValueError, "one key per expected character"):
                evaluator.replay(mismatched, authored_model(), {}, prefix=authored_prefix(), mode="default")
            engine.assert_not_called()

    def test_unsupported_characters_are_reported_without_replacement_sampling(self) -> None:
        rows = [row("hello", document="a"), row("naïve", document="b")]
        selected = evaluator.select_rows(rows)
        self.assertEqual({item.document for item in selected}, {"a", "b"})
        plan = evaluator.sequence_plan(rows[1], False)
        self.assertEqual(plan.unsupported, ("U+00EF",))
        with self.assertRaisesRegex(ValueError, "unsupported"):
            evaluator.replay(plan, ContextModel({}, "fixture"), {})

    def test_document_selection_is_stable_and_one_focus_per_document_group(self) -> None:
        rows = [row(identifier=f"focus:{index}", document=f"document:{index // 3}") for index in range(420)]
        chosen = evaluator.select_rows(rows)
        reversed_chosen = evaluator.select_rows(list(reversed(rows)))
        self.assertEqual(chosen, reversed_chosen)
        self.assertEqual(len(chosen), 128)
        self.assertEqual(len({item.document for item in chosen}), 128)

    def test_live_engine_preserves_correct_mixed_text_and_repeated_spaces(self) -> None:
        models = {
            0: LanguageModel("en_US", {"hello": 5000, "world": 4000, "api": 3000}, "authored", enable_spellcheck=False),
            1: LanguageModel("ru_RU", {"привет": 5000, "мир": 4000}, "authored", enable_spellcheck=False),
        }
        keep = authored_model()
        plan = evaluator.sequence_plan(row("hello", before="Привет!  ", after=",  API  мир! "), False)
        with patch.object(ContextModel, "load", side_effect=AssertionError("installed context must not be read")):
            result = evaluator.replay(plan, keep, models)
        self.assertEqual(result["actual"], "Привет!  hello,  API  мир! ")
        self.assertIs(result["exact"], True)
        self.assertIs(result["length_mismatch"], False)
        self.assertEqual(result["final_group"], 1)
        self.assertIsNone(result["error"])

    def test_injection_changes_subsequent_physical_glyphs_and_records_layout(self) -> None:
        # Authored direction weights force one correction through real engine
        # replacement; subsequent keys must use its new layout.
        models = {
            0: LanguageModel("en_US", {"hello": 5000, "world": 4000}, "authored", enable_spellcheck=False),
            1: LanguageModel("ru_RU", {"привет": 5000, "мир": 4000}, "authored", enable_spellcheck=False),
        }
        candidate = authored_model(convert_russian=True)
        plan = evaluator.sequence_plan(row("hello", after=" world "), True)
        result = evaluator.replay(plan, candidate, models)
        self.assertEqual(result["actual"], "hello world ")
        corrections = cast(list[dict[str, object]], result["corrections"])
        self.assertTrue(corrections)
        self.assertEqual(corrections[0]["source_group"], 1)
        self.assertEqual(corrections[0]["actual_group"], 0)
        self.assertEqual(corrections[0]["target_group"], 0)
        self.assertEqual(result["final_group"], 0)

    def test_gates_require_zero_damage_sufficient_documents_and_baseline_recall(self) -> None:
        counts = Counter({"correct_text_corruptions": 0, "length_mismatches": 0, "exactly_restored": 4,
                          "execution_errors": 0, "correction_layout_mismatches": 0})
        self.assertTrue(all(evaluator.profile_gates(counts, counts, {"0": 32, "1": 32}).values()))
        for key in ("correct_text_corruptions", "length_mismatches", "execution_errors", "correction_layout_mismatches"):
            with self.subTest(key=key):
                changed = counts.copy()
                changed[key] = 1
                self.assertFalse(all(evaluator.profile_gates(changed, counts, {"0": 32, "1": 32}).values()))
        changed = counts.copy()
        changed["exactly_restored"] = 3
        self.assertFalse(all(evaluator.profile_gates(changed, counts, {"0": 32, "1": 32}).values()))
        self.assertFalse(all(evaluator.profile_gates(counts, counts, {"0": 31, "1": 32}).values()))
        # The bar is the baseline's net outcome: restorations bought with
        # corruptions of correct text do not count against the candidate.
        damaged_baseline = counts.copy()
        damaged_baseline["correct_text_corruptions"] = 1
        self.assertTrue(all(evaluator.profile_gates(changed, damaged_baseline, {"0": 32, "1": 32}).values()))
        changed["exactly_restored"] = 2
        self.assertFalse(all(evaluator.profile_gates(changed, damaged_baseline, {"0": 32, "1": 32}).values()))
        gates = evaluator.profile_gates(counts, counts, {"0": 32, "1": 32})
        self.assertIn("net_restorations_at_least_baseline", gates)
        self.assertNotIn("restored_at_least_baseline", gates)

    def test_runtime_identity_excludes_only_unused_installed_context_and_prefix(self) -> None:
        with tempfile.TemporaryDirectory(prefix="keyswitch-runtime-identity-") as temporary:
            root = Path(temporary)
            models = root / "src/keyswitch/resources/models"
            models.mkdir(parents=True)
            tests = root / "tests"
            tests.mkdir()
            for name in ("test_input_integrity.py", "test_engine_behaviour.py", "test_windows_backend.py", "test_x11_backend.py"):
                (tests / name).write_text("# authored harness\n")
            installed = models / "context_policy_v1.json"
            installed.write_text("old installed context")
            installed_prefix = models / "prefix_policy_v1.json"
            installed_prefix.write_text("old installed prefix")
            (models / "layout_intent_v1.ksm").write_text("baseline intent")
            (root / "baseline.json").write_text("preserved context baseline")
            with patch.object(evaluator, "ROOT", root), patch.object(evaluator, "REQUIRED_PROVENANCE", frozenset({"baseline.json"})):
                before = evaluator.runtime_provenance()
                installed.write_text("promoted context")
                installed_prefix.write_text("promoted prefix")
                self.assertEqual(evaluator.runtime_provenance(), before)
                (root / "baseline.json").write_text("changed baseline")
                self.assertNotEqual(evaluator.runtime_provenance(), before)
                self.assertNotIn("src/keyswitch/resources/models/context_policy_v1.json", before)
                self.assertNotIn("src/keyswitch/resources/models/prefix_policy_v1.json", before)
                self.assertIn("src/keyswitch/resources/models/layout_intent_v1.ksm", before)

    def test_profile_aggregation_cannot_hide_damage_or_replace_unsupported_rows(self) -> None:
        rows = [row("hello" if group == 0 else "привет", group,
                    identifier=f"source:{group}:{index}", document=f"document:{group}:{index}")
                for group in (0, 1) for index in range(32)]
        rows.append(row("naïve", identifier="unsupported", document="unsupported-document"))
        candidate, baseline = authored_model(), authored_model()
        active_spelling = [False]

        def models(spelling: bool) -> dict[int, LanguageModel]:
            active_spelling[0] = spelling
            return {}

        prefix_candidate, prefix_baseline = authored_prefix(), authored_prefix(5.0)
        seen: list[tuple[str, PrefixModel | None, bool]] = []

        def replay(plan: evaluator.SequencePlan, model: ContextModel, language_models: dict[int, LanguageModel], *,
                   prefix: PrefixModel | None = None, mode: str = "early_off") -> dict[str, object]:
            seen.append((mode, prefix, model is candidate))
            damaged = active_spelling[0] and model is candidate and not plan.initially_wrong and plan.identifier == "source:0:0"
            return {"mode": mode, "actual": "corrupted" if damaged else plan.expected, "exact": not damaged,
                    "length_mismatch": False, "final_group": plan.expected_final_group,
                    "expected_final_group": plan.expected_final_group, "final_layout_matches": True,
                    "corrections": [], "error": None}

        with patch.object(evaluator, "reference_models", side_effect=models), \
                patch.object(evaluator, "replay", side_effect=replay) as replayed:
            report = evaluator.score_sequences(rows, candidate, baseline,
                                               prefix_candidate=prefix_candidate, prefix_baseline=prefix_baseline)
        profiles = cast(dict[str, dict[str, object]], report["profiles"])
        self.assertTrue(all(cast(dict[str, bool], profiles["portable"]["gates"]).values()))
        self.assertFalse(cast(dict[str, bool], profiles["reference_hunspell"]["gates"])["correct_text_preserved"])
        control = cast(dict[str, object], profiles["reference_hunspell"]["early_off"])
        self.assertFalse(cast(dict[str, bool], control["gates"])["correct_text_preserved"])
        self.assertEqual(set(cast(dict[str, object], control["counts"])), {"baseline", "candidate"})
        ablation = cast(dict[str, dict[str, object]], profiles["portable"]["ablation"])
        self.assertEqual(set(ablation), {"candidate_context_baseline_prefix"})
        self.assertNotIn("ablation", control)
        self.assertIs(report["promotion_passed"], False)
        selection = cast(dict[str, object], report["selection"])
        self.assertEqual(selection["selected_rows"], 65)
        self.assertEqual(selection["unsupported"], [{"identifier": "unsupported", "codepoints": ("U+00EF",)}])
        self.assertEqual(replayed.call_count, 64 * 2 * 2 * 5)
        self.assertEqual({(mode, prefix is prefix_candidate, is_candidate) for mode, prefix, is_candidate in seen}, {
            ("early_off", False, False), ("early_off", True, True),
            ("default", False, False), ("default", True, True), ("default", False, True)})

    def test_gates_apply_in_both_settings_modes(self) -> None:
        rows = [row("hello" if group == 0 else "привет", group,
                    identifier=f"source:{group}:{index}", document=f"document:{group}:{index}")
                for group in (0, 1) for index in range(32)]
        candidate, baseline = authored_model(), authored_model()

        def replay(plan: evaluator.SequencePlan, model: ContextModel, language_models: dict[int, LanguageModel], *,
                   prefix: PrefixModel | None = None, mode: str = "early_off") -> dict[str, object]:
            restored = plan.initially_wrong and (model is baseline or mode == "default")
            actual = plan.expected if restored or not plan.initially_wrong else "wrong"
            corrections = [{"event": 1, "before": "wrong", "after": actual, "source_group": 1 - (plan.group or 0),
                            "target_group": plan.group, "actual_group": plan.group, "caret_before": 0, "caret_after": 1}] if restored else []
            return {"mode": mode, "actual": actual, "exact": actual == plan.expected, "length_mismatch": False,
                    "final_group": plan.expected_final_group, "expected_final_group": plan.expected_final_group,
                    "final_layout_matches": True, "corrections": corrections, "error": None}

        with patch.object(evaluator, "reference_models", return_value={}), patch.object(evaluator, "replay", side_effect=replay):
            report = evaluator.score_sequences(rows, candidate, baseline,
                                               prefix_candidate=authored_prefix(), prefix_baseline=authored_prefix())
        profiles = cast(dict[str, dict[str, object]], report["profiles"])
        for profile in evaluator.PROFILES:
            self.assertTrue(all(cast(dict[str, bool], profiles[profile]["gates"]).values()))
            control = cast(dict[str, object], profiles[profile]["early_off"])
            self.assertFalse(cast(dict[str, bool], control["gates"])["net_restorations_at_least_baseline"])
        self.assertIs(report["promotion_passed"], False)

    def test_replay_injects_explicit_prefix_and_both_modes_run_engine_timers(self) -> None:
        intent = cast(LinearNgramModel, Mock(spec=LinearNgramModel, model_version="authored-intent", checksum="f" * 64))
        observed: list[tuple[PrefixModel | None, bool, bool]] = []
        timers: list[int] = []

        class RecordingEngine:
            def __init__(self, settings: SettingsStore, history: HistoryStore, backend: evaluator.TracedEditor) -> None:
                observed.append((PrefixModel.default(), bool(settings.get("detection.early_switch", True)),
                                 bool(settings.get("detection.correct_on_pause", True))))
                timers.append(0)

            def _handle(self, event: KeyEvent) -> None:
                pass

            def _expire_deferred_action(self) -> None:
                timers[-1] += 1

            def _expire_manual_correction(self) -> None:
                pass

            def _poll_current_group(self) -> None:
                pass

            def _maybe_correct_after_pause(self) -> None:
                pass

            def _expire_learning_prompt(self) -> None:
                pass

        prefix = authored_prefix()
        with tempfile.TemporaryDirectory(prefix="keyswitch-prefix-injection-") as temporary:
            root = Path(temporary)
            packaged = root / "src/keyswitch/resources/models/layout_intent_v1.ksm"
            packaged.parent.mkdir(parents=True)
            packaged.write_bytes(b"authored intent bytes")
            plan = evaluator.sequence_plan(row("hi", after=" there"), False)
            with patch.object(evaluator, "ROOT", root), patch.object(evaluator, "KeySwitchEngine", RecordingEngine), \
                    patch.object(LinearNgramModel, "load", return_value=intent):
                early = evaluator.replay(plan, authored_model(), {})
                default = evaluator.replay(plan, authored_model(), {}, prefix=prefix, mode="default")
                with self.assertRaisesRegex(ValueError, "explicit prefix"):
                    evaluator.replay(plan, authored_model(), {}, mode="default")
                with self.assertRaisesRegex(ValueError, "settings mode"):
                    evaluator.replay(plan, authored_model(), {}, prefix=prefix, mode="learning")
        self.assertEqual(observed, [(None, False, True), (prefix, True, True)])
        self.assertEqual((early["mode"], default["mode"]), ("early_off", "default"))
        # "hi there" has eight keys and two completed words: a timer pass after every key plus one
        # per word. Both modes run them, so the control mode differs from the default one only in
        # its settings: without the callbacks a document that ends without a boundary key would
        # never reach the completed-word decision, and the control would measure the early switch.
        self.assertEqual(timers, [8 + 2, 8 + 2])


class SealedAccessTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix="keyswitch-seal-fixture-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.corpus = self.root / "corpus"
        self.corpus.mkdir()
        self.artifact = self.root / "context-action.json"
        self.seal = self.root / "candidate-seal.json"
        self.output = self.root / "report.json"
        self.recipe = {"schema_version": 1, "feature_version": 3, "gate_policy": evaluator.GATE_POLICY,
                       "profiles": list(evaluator.PROFILES)}
        (self.root / "recipe.json").write_bytes(canonical(self.recipe))
        (self.root / "runtime.py").write_text("# authored runtime identity\n")
        self.write_artifact(self.artifact)
        self.write_artifact(self.root / "baseline.json", feature_version=2)
        self.prefix = self.root / "prefix-candidate.json"
        self.prefix_seal = self.root / "prefix-seal.json"
        write_prefix_artifact(self.prefix)
        write_prefix_artifact(self.root / "baseline-prefix.json", weight=5.0)
        self.membership = {key: [digest(key)] for key in ("row_ids_sha256", "family_ids_sha256", "document_ids_sha256")}
        (self.corpus / "test-membership.json").write_bytes(canonical(self.membership))
        self.manifest: dict[str, object] = {"test_membership_sha256": checksum(self.corpus / "test-membership.json"),
                                         "generator_sha256": "diagnostic-a"}
        (self.corpus / "manifest.json").write_bytes(canonical(self.manifest))
        for name, value in {
            "ROOT": self.root, "RECIPE": self.root / "recipe.json", "BASELINE": self.root / "baseline.json",
            "PREFIX_BASELINE": self.root / "baseline-prefix.json",
            "LEDGER_ROOT": self.root / "ledger", "REQUIRED_PROVENANCE": frozenset({"runtime.py", "recipe.json"}),
        }.items():
            patcher = patch.object(evaluator, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)
        runtime_patcher = patch.object(evaluator, "runtime_provenance", side_effect=lambda: {"runtime.py": checksum(self.root / "runtime.py")})
        runtime_patcher.start()
        self.addCleanup(runtime_patcher.stop)
        self.write_seal()
        self.write_prefix_seal()

    def write_prefix_seal(self, **overrides: object) -> dict[str, object]:
        value = write_prefix_seal(self.prefix_seal, self.prefix, {"runtime.py": checksum(self.root / "runtime.py")})
        if overrides:
            value.update(overrides)
            self.prefix_seal.write_bytes(canonical(value))
        return value

    def write_artifact(self, path: Path, feature_version: int = 3, weight: float = 1.0) -> None:
        weights = {"bias": [weight, 0.0, 0.0, 0.0]}
        fingerprint = hashlib.sha256(json.dumps(weights, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
        path.write_bytes(canonical({"actions": ["keep", "convert", "wait", "suggest"], "feature_version": feature_version,
                                   "weights": weights, "weights_sha256": fingerprint,
                                   "version": ("context-v3-" if feature_version == 3 else "context-v1-") + fingerprint[:12],
                                   "conversion_threshold": 0.99}))

    def write_seal(self) -> dict[str, object]:
        profile = {"rows": 20, "convert_rows": 10, "converted_correctly": 9, "false_conversions": 0, "conversion_recall": 0.9}
        value: dict[str, object] = {
            "schema_version": 1, "stage": "sealed-before-test", "test_accessed": False,
            "artifact_sha256": checksum(self.artifact), "model_version": ContextModel.load(self.artifact).version,
            "conversion_threshold": 0.99, "corpus_manifest_sha256": checksum(self.corpus / "manifest.json"),
            "provenance": {name: checksum(self.root / name) for name in ("runtime.py", "recipe.json")},
            "recipe": self.recipe, "gate_policy": evaluator.GATE_POLICY,
            "calibration": {"rows": 40, "convert_rows": 20, "converted_correctly": 18, "false_conversions": 0,
                            "conversion_recall": 0.9, "by_profile": {name: profile.copy() for name in evaluator.PROFILES}},
        }
        self.seal.write_bytes(canonical(value))
        return value

    def evaluate(self, split: str = "test") -> dict[str, object]:
        return evaluator.evaluate(self.artifact, self.seal, self.corpus, split, self.output,
                                  prefix=self.prefix, prefix_seal=self.prefix_seal)

    def test_prefix_seal_must_be_calibrated_for_test_but_may_be_diagnosed_on_development(self) -> None:
        self.write_prefix_seal(calibration_passed=False)
        with patch.object(evaluator, "load_split") as loader:
            with self.assertRaisesRegex(ValueError, "calibration"):
                self.evaluate()
            loader.assert_not_called()
        with patch.object(evaluator, "load_split", return_value=[row()]), \
                patch.object(evaluator, "score_sequences", return_value={"promotion_passed": True}):
            report = self.evaluate("development")
        self.assertIs(report["promotion_passed"], False)
        self.assertEqual(cast(dict[str, object], report["identity"])["prefix_candidate_sha256"], checksum(self.prefix))
        self.assertEqual(cast(dict[str, object], report["identity"])["prefix_baseline_sha256"], checksum(self.root / "baseline-prefix.json"))
        self.assertEqual(list((self.root / "ledger").glob("*")) if (self.root / "ledger").exists() else [], [])

    def test_prefix_seal_tampering_never_reads_test(self) -> None:
        faults: dict[str, dict[str, object]] = {
            "sha": {"candidate_sha256": "0" * 64}, "promotion": {"promotion_accepted": True},
            "tested": {"independent_test_evaluated": True},
            "provenance": {"provenance": {"runtime.py": "0" * 64}}, "version": {"model_version": "prefix-v1-000000000000"},
            "threshold": {"threshold": 0.995}, "schema": {"schema_version": 2}, "recipe": {"recipe": None},
        }
        for fault, overrides in faults.items():
            with self.subTest(fault=fault), patch.object(evaluator, "load_split") as loader:
                self.write_prefix_seal(**overrides)
                with self.assertRaises(ValueError):
                    self.evaluate()
                loader.assert_not_called()

    def test_prefix_seal_provenance_must_be_repository_relative(self) -> None:
        self.write_prefix_seal(provenance={str(self.root / "runtime.py"): checksum(self.root / "runtime.py")})
        with patch.object(evaluator, "load_split") as loader:
            with self.assertRaisesRegex(ValueError, "repository-relative"):
                self.evaluate()
            loader.assert_not_called()

    def test_changed_prefix_candidate_or_baseline_is_blocked_before_reading_observed_test(self) -> None:
        with patch.object(evaluator, "load_split", return_value=[row()]), \
                patch.object(evaluator, "score_sequences", return_value={"promotion_passed": False}):
            self.evaluate()
        write_prefix_artifact(self.prefix, weight=2.0)
        self.write_prefix_seal()
        with patch.object(evaluator, "load_split") as loader:
            with self.assertRaisesRegex(ValueError, "immutable"):
                self.evaluate()
            loader.assert_not_called()
        write_prefix_artifact(self.prefix)
        self.write_prefix_seal()
        write_prefix_artifact(self.root / "baseline-prefix.json", weight=7.0)
        with patch.object(evaluator, "load_split") as loader:
            with self.assertRaisesRegex(ValueError, "immutable"):
                self.evaluate()
            loader.assert_not_called()

    def test_valid_seal_checks_both_profiles_and_exact_policy_without_reading_test(self) -> None:
        self.assertEqual(evaluator.validate_candidate_seal(self.artifact, self.seal, self.corpus)["stage"], "sealed-before-test")
        for field in evaluator.GATE_POLICY:
            with self.subTest(field=field), patch.object(evaluator, "load_split") as loader:
                value = self.write_seal()
                policy = dict(evaluator.GATE_POLICY)
                del policy[field]
                value["gate_policy"] = policy
                self.seal.write_bytes(canonical(value))
                with self.assertRaisesRegex(ValueError, "gate policy"):
                    self.evaluate()
                loader.assert_not_called()

    def test_rejected_calibration_or_missing_provenance_never_reads_test(self) -> None:
        for fault in ("stage", "artifact", "provenance", "profile", "aggregate", "recall", "threshold", "version"):
            with self.subTest(fault=fault), patch.object(evaluator, "load_split") as loader:
                value = self.write_seal()
                if fault == "stage":
                    value["stage"] = "rejected-before-test"
                elif fault == "artifact":
                    value["artifact_sha256"] = "0" * 64
                elif fault == "provenance":
                    value["provenance"] = {"runtime.py": checksum(self.root / "runtime.py")}
                elif fault in {"profile", "aggregate", "recall"}:
                    calibration = cast(dict[str, object], value["calibration"])
                    if fault == "aggregate":
                        calibration["rows"] = 41
                    elif fault == "recall":
                        calibration["conversion_recall"] = 1.0
                    else:
                        profiles = cast(dict[str, dict[str, object]], calibration["by_profile"])
                        profiles["portable"]["false_conversions"] = 1
                elif fault == "threshold":
                    value["conversion_threshold"] = 0.98
                else:
                    value["model_version"] = "context-v3-incorrect"
                self.seal.write_bytes(canonical(value))
                with self.assertRaises(ValueError):
                    self.evaluate()
                loader.assert_not_called()

    def test_failed_result_is_immutable_and_identical_replay_is_allowed(self) -> None:
        result = {"promotion_passed": False, "fixture_failure": "correct-text-damage"}
        with patch.object(evaluator, "load_split", return_value=[row()]) as loader, \
                patch.object(evaluator, "score_sequences", return_value=result):
            report = self.evaluate()
            self.assertFalse(report["promotion_passed"])
            self.assertEqual(self.evaluate(), report)
            self.assertEqual(loader.call_count, 2)
        outcomes = list((self.root / "ledger").glob("*.outcome.json"))
        self.assertEqual(len(outcomes), 1)
        original = outcomes[0].read_bytes()
        with patch.object(evaluator, "load_split", return_value=[row()]), \
                patch.object(evaluator, "score_sequences", return_value={"promotion_passed": True}):
            with self.assertRaisesRegex(ValueError, "immutable"):
                self.evaluate()
        self.assertEqual(outcomes[0].read_bytes(), original)

    def test_new_candidate_is_blocked_before_reading_previously_observed_test(self) -> None:
        with patch.object(evaluator, "load_split", return_value=[row()]), \
                patch.object(evaluator, "score_sequences", return_value={"promotion_passed": False}):
            self.evaluate()
        self.write_artifact(self.artifact, weight=2.0)
        self.write_seal()
        with patch.object(evaluator, "load_split") as loader:
            with self.assertRaisesRegex(ValueError, "immutable"):
                self.evaluate()
            loader.assert_not_called()

    def test_same_membership_cannot_be_reset_by_manifest_metadata_or_namespace(self) -> None:
        with patch.object(evaluator, "load_split", return_value=[row()]), \
                patch.object(evaluator, "score_sequences", return_value={"promotion_passed": False}):
            self.evaluate()
        self.manifest["generator_sha256"] = "diagnostic-b"
        self.manifest["namespace"] = "attempt-to-relabel-test"
        (self.corpus / "manifest.json").write_bytes(canonical(self.manifest))
        self.write_artifact(self.artifact, weight=2.0)
        self.write_seal()
        with patch.object(evaluator, "load_split") as loader:
            with self.assertRaisesRegex(ValueError, "immutable"):
                self.evaluate()
            loader.assert_not_called()

    def test_runtime_change_and_overlapping_documents_are_blocked_before_read(self) -> None:
        with patch.object(evaluator, "load_split", return_value=[row()]), \
                patch.object(evaluator, "score_sequences", return_value={"promotion_passed": False}):
            self.evaluate()
        (self.root / "runtime.py").write_text("# changed runtime\n")
        with patch.object(evaluator, "load_split") as loader:
            with self.assertRaisesRegex(ValueError, "provenance changed"):
                self.evaluate()
            loader.assert_not_called()
        self.write_seal()
        with patch.object(evaluator, "load_split") as loader:
            with self.assertRaisesRegex(ValueError, "prefix provenance changed"):
                self.evaluate()
            loader.assert_not_called()
        self.write_prefix_seal()
        with patch.object(evaluator, "load_split") as loader:
            with self.assertRaisesRegex(ValueError, "immutable"):
                self.evaluate()
            loader.assert_not_called()
        self.membership["row_ids_sha256"] = [digest("different-row")]
        self.membership["family_ids_sha256"] = [digest("different-family")]
        (self.corpus / "test-membership.json").write_bytes(canonical(self.membership))
        self.manifest["test_membership_sha256"] = checksum(self.corpus / "test-membership.json")
        (self.corpus / "manifest.json").write_bytes(canonical(self.manifest))
        self.write_seal()
        with patch.object(evaluator, "load_split") as loader:
            with self.assertRaisesRegex(ValueError, "overlaps"):
                self.evaluate()
            loader.assert_not_called()

    def test_exception_after_access_preserves_failed_outcome_and_rethrows(self) -> None:
        with patch.object(evaluator, "load_split", side_effect=ValueError("authored broken split")):
            with self.assertRaisesRegex(ValueError, "authored broken"):
                self.evaluate()
        outcome = evaluator.read_object(next((self.root / "ledger").glob("*.outcome.json")))
        self.assertIs(outcome["promotion_passed"], False)
        self.assertEqual(outcome["execution_error"], "ValueError: authored broken split")
        self.assertTrue(self.output.is_file())

    def test_report_cannot_overwrite_test_ledger_or_sealed_inputs(self) -> None:
        for path in (self.seal, self.artifact, self.prefix, self.prefix_seal, self.corpus / "manifest.json",
                     self.root / "ledger" / "report.json"):
            with self.subTest(path=path), patch.object(evaluator, "load_split") as loader:
                with self.assertRaisesRegex(ValueError, "overwrite"):
                    evaluator.evaluate(self.artifact, self.seal, self.corpus, "test", path,
                                       prefix=self.prefix, prefix_seal=self.prefix_seal)
                loader.assert_not_called()


if __name__ == "__main__":
    unittest.main()
