"""Shipped segmentation policy: learned spans, exact buffer and fail-closed IO."""
from __future__ import annotations

import json
import sys
import tempfile
import time
import unittest
from dataclasses import replace
from pathlib import Path
from unittest.mock import patch

from keyswitch.boundary_policy import BoundaryPolicy, features
from test_input_integrity import InputIntegrityTests


ROOT = Path(__file__).resolve().parents[1]
CANDIDATE = ROOT / "model/boundary_v2/candidate.json"
if str(ROOT / "tools") not in sys.path:
    sys.path.insert(0, str(ROOT / "tools"))

import verify_boundary_v2 as verifier
from boundary_v2_corpus import RECEIPT
from evaluate_boundary_engine import REPORT as ENGINE_REPORT
from verify_context_v2 import read_object


class BoundaryPolicyArtifactTests(unittest.TestCase):
    def test_package_requires_approved_weights_and_exact_engine_evidence(self) -> None:
        self.assertTrue(verifier.verify()["accepted"])
        corpus, engine = read_object(RECEIPT), read_object(ENGINE_REPORT)
        altered_corpora = ({**corpus, "family_overlap": 1}, {**corpus, "prior_phrase_test_family_overlap": 1},
                           {**corpus, "sha256": {}})
        for changed in altered_corpora:
            with patch.object(verifier, "read_object", return_value=changed), self.assertRaises(ValueError):
                verifier.verify()
        invalid_engines: tuple[dict[str, object], ...] = ({"provenance": {}}, {"results": {}}, {"results": []})
        for change in invalid_engines:
            with patch.object(verifier, "read_object", side_effect=[corpus, {**engine, **change}]), self.assertRaises(ValueError):
                verifier.verify()
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "artifact.json"
            with self.assertRaisesRegex(ValueError, "missing"):
                verifier.verify(artifact=path)
            path.write_text("{}")
            with self.assertRaisesRegex(ValueError, "changed"):
                verifier.verify(artifact=path)
            with self.assertRaisesRegex(ValueError, "sealed"):
                verifier.verify(report_path=path)
        results = engine["results"]
        assert isinstance(results, dict)
        active = results["active_v2"]
        assert isinstance(active, dict)
        invalid_results: tuple[dict[str, object], ...] = ({"exact": 0}, {"changed_correct": 1}, {"rows": []}, {"rows": [{}] * 18})
        for changes in invalid_results:
            changed_engine = {**engine, "results": {**results, "active_v2": {**active, **changes}}}
            with patch.object(verifier, "read_object", side_effect=[corpus, changed_engine]), self.assertRaises(ValueError):
                verifier.verify()

    def test_version_size_and_numeric_validation(self) -> None:
        good = {"feature_version": 2, "version": "boundary-v2-test", "threshold": .99, "weights": {"bias": 0.0}}
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "model.json"
            invalid_cases: tuple[object, ...] = ([], {}, {**good, "feature_version": 1}, {**good, "version": "boundary-v1-test"},
                            {**good, "version": None}, {**good, "threshold": True}, {**good, "threshold": .5},
                            {**good, "threshold": None}, {**good, "weights": {}},
                            {**good, "weights": {"a": float("inf")}}, {**good, "weights": {"a": True}})
            for invalid in invalid_cases:
                path.write_text(json.dumps(invalid))
                with self.assertRaises(ValueError):
                    BoundaryPolicy.load(path)
            path.write_bytes(b" " * 65537)
            with self.assertRaisesRegex(ValueError, "oversized"):
                BoundaryPolicy.load(path)
            path.write_text(json.dumps(good))
            self.assertEqual(BoundaryPolicy.load(path).version, "boundary-v2-test")
        BoundaryPolicy.default.cache_clear()
        with patch.object(BoundaryPolicy, "load", side_effect=OSError("missing")):
            self.assertIsNone(BoundaryPolicy.default())
        BoundaryPolicy.default.cache_clear()
        with patch.object(BoundaryPolicy, "load", return_value=BoundaryPolicy({}, .99, "test")) as load:
            self.assertIs(BoundaryPolicy.default(), load.return_value)
        BoundaryPolicy.default.cache_clear()


class BoundaryPolicyEngineTests(InputIntegrityTests):
    def setUp(self) -> None:
        super().setUp()
        self.settings.set("detection.context_policy", "assist")
        self.engine.boundary_model = BoundaryPolicy.load(CANDIDATE)

    def physical(self, text: str) -> None:
        for key in text:
            group = self.backend.group
            char = key if group == 0 else self.pair.translate(key, "us", "ru")
            self.tap(self.key("space" if char == " " else char, char, group=group))

    def test_internal_ambiguous_keys_are_not_committed_as_literal_punctuation(self) -> None:
        for keys, expected in (("ghj,ktvf", "проблема"), ("ghtlkj;bk", "предложил"), (",b,kbjntrf", "библиотека")):
            with self.subTest(keys=keys):
                self.reset_editor()
                self.physical(keys)
                self.assertEqual(self.backend.text, keys)
                self.physical(" ")
                self.assertEqual(self.backend.text, expected + " ")

    def test_standalone_punctuation_does_not_become_a_russian_letter_on_idle(self) -> None:
        for key in (".", ",", ";", "[", "]", "...", "'"):
            with self.subTest(key=key):
                self.reset_editor()
                before = "проверить модуль parser "
                self.engine._focus_window = self.backend.window
                self.engine.context_policy.stream.focus("TestEditor", self.backend.window)
                self.engine.context_policy.stream.text = before
                self.engine.context_policy.stream.updated_at = time.monotonic()
                self.backend.text = before
                self.backend.caret = len(before)
                self.physical(key)
                last = self.engine._last_word_input_at
                assert last is not None
                self.engine._maybe_correct_after_pause(now=last + 2)
                self.assertEqual(self.backend.text, before + key)
                self.assertEqual(self.backend.group, 0)

    def test_authored_literal_suffixes_words_and_technical_negatives(self) -> None:
        for keys, expected in (("rjnjhe.", "которую"), ("ghbdtn,", "привет,"),
                               ("ghbdtn...", "привет..."), ("hello,", "hello,"),
                               ("hello,world", "hello,world"), ("don't", "don't"),
                               ("object.field", "object.field"), ("folder/file.py", "folder/file.py")):
            with self.subTest(keys=keys):
                self.reset_editor()
                self.physical(keys + " ")
                self.assertEqual(self.backend.text, expected + " ")

    def test_features_distinguish_competition_without_whole_word_identifiers(self) -> None:
        values = features("rjnjhe.", "которую", 0, self.engine.models[0], self.engine.models[1])
        self.assertIn("word:known_either:competition", values)
        self.assertEqual(values["tail:has:."], 0.0)
        self.assertFalse(any("которую" in name for name in values))

    def test_real_policy_preserves_literal_suffix_and_spaces_during_undo(self) -> None:
        for suffix in (",", ".", ";", "]", "...", "'"):
            for space in (" ", "   ", "\u00a0", "\u202f", "\u2009"):
                with self.subTest(suffix=suffix, space=repr(space)):
                    self.reset_editor()
                    self.engine.learning.clear()
                    self.physical("ghbdtn" + suffix + space)
                    self.assertEqual(self.backend.text, "привет" + suffix + space)
                    self.assertEqual(self.backend.caret, len(self.backend.text))
                    if len(space) == 1:
                        self.tap(self.key("Pause"))
                        self.assertEqual(self.backend.text, "ghbdtn" + suffix + space)

    def test_manual_pause_can_convert_an_isolated_symbol_despite_automatic_abstention(self) -> None:
        self.physical(".")
        last = self.engine._last_word_input_at
        assert last is not None
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.engine._maybe_correct_after_pause(now=last + 2)
        guards = [json.loads(line.split("TECHNICAL ", 1)[1]) for line in logs.output if "TECHNICAL " in line]
        self.assertTrue(any(event["event"] == "boundary_guard" and event["reason"] == "no_word" for event in guards))
        self.assertFalse(any(event["event"] == "context_decision" for event in guards))
        self.tap(self.key("Pause"))
        self.assertEqual(self.backend.text, "ю")

    def test_real_policy_corrects_before_deferred_enter_and_tab_exactly_once(self) -> None:
        for key in ("Return", "Tab"):
            with self.subTest(key=key):
                self.reset_editor()
                self.engine.learning.clear()
                self.physical("ghbdtn,")
                event = replace(self.key(key), deferred=True)
                delivered: list[str] = []
                def complete_action(deliver: bool) -> int:
                    if deliver:
                        delivered.append(self.backend.text)
                        self.backend.type(replace(event, deferred=False))
                    return 0
                self.engine._handle(event)
                with patch.object(self.backend, "complete_action", side_effect=complete_action) as complete:
                    self.engine._handle(replace(event, pressed=False))
                complete.assert_called_once_with(True)
                self.assertEqual(delivered, ["привет,"])

    def test_missing_artifact_keeps_legacy_boundaries_and_protected_tokens(self) -> None:
        self.engine.boundary_model = None
        self.settings.set("detection.context_policy", "off")
        self.assertFalse(self.engine._ambiguous_key_is_boundary(self.key(",", ",")))
        for physical, expected in (("ghbdtn,", "привет,"), (",fpf ", "база "), ("j,ob[ ", "общих "),
                                   ("kubectl,", "kubectl,"), ("hello,", "hello,")):
            with self.subTest(physical=physical):
                self.reset_editor()
                self.physical(physical)
                self.assertEqual(self.backend.text, expected)
        self.reset_editor()
        self.physical("a")
        self.assertFalse(self.engine._ambiguous_key_is_boundary(self.key("'", "'")))


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    suite = loader.loadTestsFromTestCase(BoundaryPolicyArtifactTests)
    suite.addTests(BoundaryPolicyEngineTests(name) for name in BoundaryPolicyEngineTests.__dict__ if name.startswith("test_"))
    return suite
