"""Boundary inference and exact execution are deliberately tested separately."""
from __future__ import annotations

import json
import sys
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path
from unittest.mock import patch

from keyswitch.backend import KeyEvent
from keyswitch.boundary_model import BoundaryModel, BoundaryPrediction
from keyswitch.input_context import FieldContext
from keyswitch.windows_backend import VK_BACK, WindowsBackend, WindowsBackendError
from keyswitch.x11_backend import X11Error
from test_input_integrity import InputIntegrityTests
from test_windows_backend import FakeWindowsAPI
from test_x11_backend import backend_with


ROOT = Path(__file__).resolve().parents[1]
CANDIDATE = ROOT / "model/boundary_v1/candidate.json"
if str(ROOT / "tools") not in sys.path:
    sys.path.insert(0, str(ROOT / "tools"))

from verify_boundary_model import verify
from verify_context_v2 import read_object


class BoundaryArtifactTests(unittest.TestCase):
    def test_rejected_weights_cannot_ship_and_metrics_cannot_be_relabelled(self) -> None:
        self.assertEqual(verify()["accepted"], False)
        with self.assertRaisesRegex(ValueError, "must not ship"):
            verify(active=CANDIDATE)
        report = read_object(CANDIDATE.parent / "report.json")
        changes: list[dict[str, object]] = [{"accepted": True}, {"test": {}}, {"test": {"rows": -1}}, {"test": []}, {"seal_sha256": "changed"}]
        for change in changes:
            def altered(path: Path) -> dict[str, object]:
                return {**report, **change} if path.name == "report.json" else read_object(path)
            with patch("verify_boundary_model.read_object", side_effect=altered), self.assertRaises(ValueError):
                verify()

    def test_load_predict_and_fail_closed(self) -> None:
        model = BoundaryModel.load(CANDIDATE)
        self.assertIsNone(model.predict(({}, {})).suffix_length)
        self.assertEqual(model.predict(({},)).suffix_length, 0)
        self.assertEqual(BoundaryModel({"a": 20}, .9, "test").predict(({}, {"a": 1})).suffix_length, 1)
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "model.json"
            good = {"feature_version": 1, "version": "test", "threshold": .99, "weights": {"bias": 0}}
            cases: list[object] = [[], {}, {**good, "weights": {}}, {**good, "weights": {"a": "bad"}},
                                  {**good, "weights": {"a": float("inf")}}, {**good, "threshold": None},
                                  {**good, "threshold": True}, {**good, "threshold": .4},
                                  {**good, "version": ""}, {**good, "version": None}]
            for value in cases:
                path.write_text(json.dumps(value))
                with self.assertRaises(ValueError):
                    BoundaryModel.load(path)
            path.write_text(json.dumps(good))
            self.assertEqual(BoundaryModel.load(path).version, "test")
        BoundaryModel.default.cache_clear()
        with patch.object(BoundaryModel, "load", return_value=model):
            self.assertIs(BoundaryModel.default(), model)
        BoundaryModel.default.cache_clear()
        with patch.object(BoundaryModel, "load", side_effect=ValueError("invalid")):
            self.assertIsNone(BoundaryModel.default())
        BoundaryModel.default.cache_clear()


class BoundaryExecutionTests(InputIntegrityTests):
    def setUp(self) -> None:
        super().setUp()
        # Explicit experimental injection, never an implicit production rollout.
        self.model = BoundaryModel.load(CANDIDATE)
        self.engine.boundary_model = self.model

    def idle(self) -> None:
        last = self.engine._last_word_input_at
        assert last is not None
        self.engine._maybe_correct_after_pause(now=last + 2)

    def test_inner_punctuation_no_longer_cuts_a_prefix(self) -> None:
        for original, expected in (("ghj,ktvf", "проблема"), ("ghtlkj;bk", "предложил"), (",b,kbjntrf", "библиотека")):
            with self.subTest(original=original):
                self.reset_editor()
                self.type(original, group=0)
                self.assertEqual(self.backend.text, original)
                self.assertEqual(self.backend.injections[-1:] if self.backend.injections else [], [])
                self.type(" ")
                self.assertEqual(self.backend.text, expected + " ")
                self.backend.injections.clear()

    def test_uncertainty_keeps_whole_token_and_manual_command_still_works(self) -> None:
        with patch.object(self.model, "predict", return_value=BoundaryPrediction(None, .51, "uncertain")):
            self.type("rjnjhe.")
            self.assertEqual(self.engine.snapshot.current_word, "rjnjhe.")
            self.idle()
            self.assertEqual(self.backend.text, "rjnjhe.")
            self.type(" ")
            self.assertEqual(self.backend.text, "rjnjhe. ")
            self.tap(self.key("Pause"))
            self.assertEqual(self.backend.text, "которую ")

    def test_literal_suffix_space_and_undo_are_exact_with_a_selected_span(self) -> None:
        # This checks the executor contract, NOT learned model accuracy.
        for punctuation in (",", ".", ";", "]", "'", "...", ",;"):
            for space in (" ", "   ", "\u00a0", "\u202f", "\u2009"):
                with self.subTest(punctuation=punctuation, space=repr(space)):
                    self.reset_editor()
                    self.engine.learning.clear()
                    with patch.object(self.model, "predict", return_value=BoundaryPrediction(len(punctuation), 1, "test-span")):
                        self.type("ghbdtn" + punctuation, group=0)
                        self.type(space, group=0)
                    self.assertEqual(self.backend.text, "привет" + punctuation + space)
                    self.assertEqual(self.backend.caret, len(self.backend.text))
                    if len(space) == 1:
                        self.tap(self.key("Pause"))
                        self.assertEqual(self.backend.text, "ghbdtn" + punctuation + space)

    def test_idle_literal_suffix_preserves_context_and_does_not_become_a_word(self) -> None:
        self.settings.set("detection.context_policy", "shadow")
        self.type("ghbdtn,")
        with patch.object(self.model, "predict", return_value=BoundaryPrediction(1, 1, "test-span")):
            self.idle()
        self.assertEqual(self.backend.text, "привет,")
        self.assertEqual(self.engine.context_policy.stream.text, "привет,")
        self.assertEqual(self.engine._strokes, [])
        self.engine._schedule_undo(999)
        self.tap(replace(self.key("Pause"), keycode=999, pressed=False))
        self.assertEqual(self.backend.text, "ghbdtn,")

    def test_deferred_enter_converts_word_and_preserves_punctuation_before_submit(self) -> None:
        self.type("ghbdtn,")
        event = replace(self.key("Return"), deferred=True)
        def complete_action(deliver: bool) -> int:
            if deliver:
                self.backend.type(replace(event, deferred=False))
            return 0
        with patch.object(self.model, "predict", return_value=BoundaryPrediction(1, 1, "test-span")):
            self.engine._handle(event)
            with patch.object(self.backend, "complete_action", side_effect=complete_action) as complete:
                self.engine._handle(replace(event, pressed=False))
        complete.assert_called_once_with(True)
        self.assertEqual(self.backend.submissions, ["привет,"])

    def test_suffix_anchor_native_reader_rejects_changed_text(self) -> None:
        self.settings.set("detection.context_policy", "assist")
        self.settings.set("detection.context_read_field", True)
        for changed in (False, True):
            self.reset_editor()
            self.type("ghbdtn,")
            field = FieldContext("TestEditor", "field", before="previous ghbdtn, ", source="native")
            reader = self.engine.context_policy.reader
            assert reader is not None
            with patch.object(self.model, "predict", return_value=BoundaryPrediction(1, 1, "test-span")), patch.object(reader, "read", return_value=field) as read:
                boundary = self.key("space", " ")
                self.send(boundary)
                plan = self.engine._pending
                assert plan is not None
                self.assertEqual(plan.context_field, "field")
                self.assertIsNotNone(self.engine._context_result)
                if changed:
                    self.backend.text = "ghbdtn; "
                    read.return_value = replace(field, before="previous ghbdtn; ")
                self.send(replace(boundary, pressed=False))
            self.assertEqual(self.backend.text, "ghbdtn; " if changed else "привет, ")

    def test_candidate_whole_word_and_shape_limits(self) -> None:
        with patch.object(self.model, "predict", return_value=BoundaryPrediction(0, 1, "test-span")):
            self.type("rjnjhe. ")
        self.assertEqual(self.backend.text, "которую ")
        for token in ("...", "a" + "." * 9, "path/word,", "ghbdtn,", "hello,world", "don’t", "unknown.xyz", "a" * 270):
            self.reset_editor()
            self.settings.set("exclusions.words", ["ghbdtn,"])
            with patch.object(self.model, "predict", return_value=BoundaryPrediction(None, .5, "uncertain")):
                self.type(token + " ", group=0)
            self.assertEqual(self.backend.text, token + " ")
        strokes = (self.key("a", "a"), self.key("comma", ","))
        self.assertTrue(self.engine._completed_word(strokes, 20)[2])
        with patch.object(self.engine, "models", {0: self.engine.models[0]}):
            self.assertFalse(self.engine._completed_word(strokes, 0)[2])

    def test_explicit_full_token_rule_and_rejection_outrank_segmentation(self) -> None:
        self.engine.learning.confirm_manual(0, "rjnjhe.", 1, 2)
        with patch.object(self.model, "predict", side_effect=AssertionError("must not override user rule")):
            self.type("rjnjhe. ")
        self.assertEqual(self.backend.text, "которую ")
        self.reset_editor()
        self.engine.learning.clear()
        self.engine.learning.reject(0, "ghbdtn,", 1)
        with patch.object(self.model, "predict", side_effect=AssertionError("must not bypass full-token rejection")):
            self.type("ghbdtn, ")
        self.assertEqual(self.backend.text, "ghbdtn, ")

    def test_edit_focus_change_and_late_input_do_not_corrupt_a_selected_suffix(self) -> None:
        self.type("ghbdtn,")
        self.tap(self.key("BackSpace"))
        self.assertEqual(self.engine.snapshot.current_word, "ghbdtn")
        self.type(",")
        self.tap(self.key("Left"))
        self.tap(self.key("space", " "))
        self.assertEqual(self.backend.text, "ghbdtn ,")
        self.assertEqual(self.backend.injections, [])
        self.reset_editor()
        self.type("ghbdtn,")
        boundary = self.key("space", " ")
        with patch.object(self.model, "predict", return_value=BoundaryPrediction(1, 1, "test-span")):
            self.send(boundary)
        self.backend.window += 1
        self.send(replace(boundary, pressed=False))
        self.assertEqual(self.backend.text, "ghbdtn, ")
        self.reset_editor()
        self.type("ghbdtn,")
        boundary = self.key("space", " ")
        with patch.object(self.model, "predict", return_value=BoundaryPrediction(1, 1, "test-span")):
            self.send(boundary)
        late = self.key("x", "x")
        self.backend.type(late)
        self.engine.enqueue(late)
        self.send(replace(boundary, pressed=False))
        self.assertEqual(self.backend.text, "привет, ч")

    def test_native_backends_replay_literal_tail_in_original_layout_even_during_undo(self) -> None:
        word = self.key("a", "a")
        comma = self.key("comma", ",")
        space = self.key("space", " ")
        for source, target in ((0, 1), (1, 0)):
            api = FakeWindowsAPI()
            backend = WindowsBackend(api)
            backend.inject_correction((word,), target, space, source, trailing=(comma,))
            sent = [event for batch in api.sent for event in batch]
            self.assertEqual(sum(event.pressed and event.virtual_key == VK_BACK for event in sent), 3)
            self.assertEqual([event.scan_code for event in sent if event.pressed and event.scan_code], [word.keycode, comma.keycode, space.keycode])
            x11, libraries = backend_with()
            x11._control = 1
            x11.inject_correction((word,), target, space, source, trailing=(comma,))
            taps = [call.args[1] for call in libraries.xtst.XTestFakeKeyEvent.call_args_list if call.args[2]]
            self.assertEqual(taps, [22, 22, 22, word.keycode, comma.keycode, space.keycode])
            self.assertEqual([call.args[2] for call in libraries.x11.XkbLockGroup.call_args_list], [1, 0, 1] if target else [0])
        for group in (1, 99):
            api = FakeWindowsAPI()
            backend = WindowsBackend(api)
            with self.assertRaises(WindowsBackendError):
                backend.inject_correction((word,), 1, space, 0, trailing=(replace(comma, group=group),))
            self.assertFalse(api.sent)
            x11, libraries = backend_with()
            x11._control = 1
            with self.assertRaises(X11Error):
                x11.inject_correction((word,), 1, space, 0, trailing=(replace(comma, group=group),))
            libraries.xtst.XTestFakeKeyEvent.assert_not_called()


def load_tests(loader: unittest.TestLoader, _tests: unittest.TestSuite, _pattern: str | None) -> unittest.TestSuite:
    suite = loader.loadTestsFromTestCase(BoundaryArtifactTests)
    suite.addTests(BoundaryExecutionTests(name) for name in BoundaryExecutionTests.__dict__ if name.startswith("test_"))
    return suite
