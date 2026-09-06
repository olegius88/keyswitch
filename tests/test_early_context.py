"""Early-prefix switching with assist enabled, checked against an editing buffer."""

from __future__ import annotations

import unittest
from dataclasses import replace
from unittest.mock import patch

from keyswitch.backend import CONTROL_MASK
from keyswitch.context_model import ContextModel
from keyswitch.early_switch import EarlySwitchDecision
from keyswitch.input_context import FieldContext
from keyswitch.prefix_model import PrefixModel
from test_input_integrity import EditorBackend, InputIntegrityTests


class EditorReader:
    def __init__(self, backend: EditorBackend) -> None:
        self.backend = backend
        self.field_id = "editor-1"
        self.sensitive = False
        self.selection = False
        self.suffix = ""

    def read(self, application: str, window: int) -> FieldContext:
        return FieldContext(
            application, self.field_id, self.backend.text + self.suffix,
            sensitive=self.sensitive, selection=self.selection, source="test-reader",
        )


class EarlyContextTests(InputIntegrityTests):
    def setUp(self) -> None:
        super().setUp()
        self.settings.set("detection.context_policy", "assist")
        self.settings.set("detection.context_aware", True)
        self.settings.set("detection.early_switch", True)
        self.engine.prefix_model = PrefixModel(ContextModel({"bias": (0.0, 20.0, 0.0, 0.0)}, "prefix-v1-fixture"))

    def test_assist_switches_before_boundary_without_classifying_an_unfinished_word(self) -> None:
        model = self.engine.context_policy.model
        assert model is not None
        with patch.object(model, "predict", side_effect=AssertionError("prefix is not a completed word")):
            self.type("ghb")
            self.assertEqual(self.backend.text, "ghb")
            self.type("d")
            self.assertEqual(self.backend.text, "прив")
            self.assertEqual(self.backend.group, 1)
            self.assertEqual(self.engine.context_policy.stream.text, "прив")
        self.type("ет ")
        self.assertEqual(self.backend.text, "привет ")
        self.assertEqual(self.engine.snapshot.correction_count, 1)

    def test_early_setting_is_independent_of_context_modes(self) -> None:
        for mode in ("assist", "shadow", "off"):
            for aware in (True, False):
                with self.subTest(mode=mode, aware=aware):
                    self.reset_editor()
                    self.settings.set("detection.context_policy", mode)
                    self.settings.set("detection.context_aware", aware)
                    self.type("ghbd")
                    self.assertEqual(self.backend.text, "прив")
        self.reset_editor()
        self.settings.set("detection.context_policy", "assist")
        self.settings.set("detection.early_switch", False)
        self.type("ghbd")
        self.assertEqual(self.backend.text, "ghbd")

    def test_assist_rollover_waits_for_all_releases_and_preserves_phrase(self) -> None:
        self.type("я ", group=1)
        self.type("ghb", group=0)
        held = self.key("d", "d", group=0)
        rollover = self.key("t", "t", group=0)
        self.send(held)
        self.send(rollover)
        self.assertEqual(self.backend.text, "я ghbdt")
        self.send(replace(held, pressed=False))
        self.assertEqual(self.backend.text, "я ghbdt")
        self.send(replace(rollover, pressed=False))
        self.assertEqual(self.backend.text, "я приве")
        self.assertEqual(self.engine.context_policy.stream.text, "я приве")
        self.type("т  ")
        self.assertEqual(self.backend.text, "я привет  ")

    def test_excluded_words_and_learned_rejections_protect_their_prefix(self) -> None:
        self.settings.set("exclusions.words", ["GHBDTN"])
        self.type("ghbdtn ", group=0)
        self.assertEqual(self.backend.text, "ghbdtn ")
        self.reset_editor()
        self.settings.set("exclusions.words", [])
        self.engine.learning.reject(0, "ghbdtn", 1)
        self.type("ghbdtn ", group=0)
        self.assertEqual(self.backend.text, "ghbdtn ")
        self.reset_editor()
        self.settings.set("detection.learning", False)
        self.type("ghbd")
        self.assertEqual(self.backend.text, "прив")

    def test_wrong_rejection_direction_and_other_words_do_not_block_prefix(self) -> None:
        self.engine.learning.reject(1, "ghbdtn", 0)
        self.engine.learning.reject(0, "other", 1)
        self.settings.set("exclusions.words", ["other"])
        self.type("ghbd")
        self.assertEqual(self.backend.text, "прив")

    def test_undo_and_manual_choice_still_outrank_early_switching(self) -> None:
        self.type("ghbd")
        self.tap(replace(self.key("z"), state=CONTROL_MASK | 8))
        self.assertEqual(self.backend.text, "ghbd")
        self.type("tn ", group=0)
        self.assertEqual(self.backend.text, "ghbdtn ")
        self.reset_editor()
        self.settings.set("detection.respect_manual_layout", True)
        self.engine._manual_layout_group = 0
        self.type("ghbd")
        self.assertEqual(self.backend.text, "ghbd")

    def test_rollover_that_becomes_a_technical_token_cancels_early_plan(self) -> None:
        self.type("ghb")
        held = self.key("d", "d")
        self.send(held)
        self.type("_id", group=0)
        self.send(replace(held, pressed=False))
        self.assertEqual(self.backend.text, "ghbd_id")
        self.assertEqual(self.backend.injections, [])

    def test_native_field_is_rechecked_after_rollover(self) -> None:
        reader = EditorReader(self.backend)
        self.engine.context_policy.reader = reader
        self.settings.set("detection.context_read_field", True)
        self.type("ghb")
        held = self.key("d", "d")
        self.send(held)
        self.type("t", group=0)
        reader.field_id = "different-field"
        self.send(replace(held, pressed=False))
        self.assertEqual(self.backend.text, "ghbdt")
        self.assertEqual(self.backend.injections, [])

    def test_native_field_mismatch_and_selection_prevent_early_replacement(self) -> None:
        for selection in (True, False):
            with self.subTest(selection=selection):
                self.reset_editor()
                reader = EditorReader(self.backend)
                reader.selection = selection
                reader.suffix = "" if selection else "untracked suffix"
                self.engine.context_policy.reader = reader
                self.settings.set("detection.context_read_field", True)
                self.type("ghbd")
                self.assertEqual(self.backend.text, "ghbd")
        self.reset_editor()
        self.engine.context_policy.reader = EditorReader(self.backend)
        self.type("ghbd")
        self.assertEqual(self.backend.text, "прив")

    def test_model_keep_wait_missing_and_shadow_have_distinct_effects(self) -> None:
        for scores in ((20., 0., 0., 0.), (0., 1., 0., 0.)):
            self.reset_editor()
            self.engine.prefix_model = PrefixModel(ContextModel({"bias": scores}, "fixture"))
            self.type("ghbd")
            self.assertEqual(self.backend.text, "ghbd")
        self.reset_editor()
        self.settings.set("detection.context_policy", "shadow")
        self.type("ghbd")
        self.assertEqual(self.backend.text, "прив")
        for mode in ("assist", "shadow"):
            self.reset_editor()
            self.settings.set("detection.context_policy", mode)
            self.engine.prefix_model = None
            self.type("ghbd")
            self.assertEqual(self.backend.text, "ghbd" if mode == "assist" else "прив")

    def test_unvalidated_inputs_never_use_model_even_when_code_guard_is_off(self) -> None:
        self.settings.set("detection.protect_code", False)
        baseline = EarlySwitchDecision(True, 0, 1, "ghbd", "прив", "fixture")
        field = FieldContext("TestEditor", "1")
        for item, context in (
            (baseline, None), (replace(baseline, original="ghb"), field),
            (replace(baseline, original="g" * 13), field), (replace(baseline, replacement="1"), field),
            (replace(baseline, original="GhBd"), field), (replace(baseline, source_group=2), field),
            (replace(baseline, target_group=2), field),
        ):
            self.assertFalse(self.engine._decide_prefix(item, context)[0].should_switch)
        for group in (0, 1):
            for attribute in ("models", "_prefix_indexes"):
                with patch.dict(getattr(self.engine, attribute), {}, clear=True):
                    self.assertFalse(self.engine._decide_prefix(replace(baseline, source_group=group, target_group=1-group), field)[0].should_switch)
        self.settings.set("detection.early_switch_min_length", 3)
        self.type("ghb")
        self.assertEqual(self.backend.text, "ghb")
        self.type("d")
        self.assertEqual(self.backend.text, "прив")

    def test_sensitive_and_unavailable_reader_and_waiting_word_guards(self) -> None:
        reader = EditorReader(self.backend)
        self.engine.context_policy.reader = reader
        self.settings.set("detection.context_read_field", True)
        with patch.object(reader, "read", return_value=None):
            self.type("ghbd")
        self.assertEqual(self.backend.text, "прив")
        reader.sensitive = True
        self.engine._sensitive_context_window = None
        field, reason = self.engine._early_prefix_field("прив", "TestEditor")
        self.assertEqual(reason, "sensitive_field")
        self.assertEqual(field.before, "")
        self.assertEqual(self.engine.context_policy.stream.text, "")
        self.assertEqual(self.engine.snapshot.current_word, "")
        with patch.object(self.engine, "_context_waiting", object()):
            self.assertEqual(self.engine._early_prefix_protection("ghbd", 0, 1), "context_word_waiting")

    def test_release_rechecks_prediction_field_and_changed_minimum_length(self) -> None:
        for change in ("minimum", "disabled", "keep", "sensitive", "selection"):
            with self.subTest(change=change):
                self.reset_editor()
                self.settings.set("detection.early_switch", True)
                self.settings.set("detection.early_switch_min_length", 4)
                self.settings.set("detection.context_read_field", False)
                self.engine.prefix_model = PrefixModel(ContextModel({"bias": (0., 20., 0., 0.)}, "fixture"))
                self.type("ghb")
                held = self.key("d", "d")
                self.send(held)
                self.assertIsNotNone(self.engine._pending)
                if change == "minimum":
                    self.settings.set("detection.early_switch_min_length", 8)
                elif change == "disabled":
                    self.settings.set("detection.early_switch", False)
                elif change == "keep":
                    self.engine.prefix_model = PrefixModel(ContextModel({"bias": (20., 0., 0., 0.)}, "fixture"))
                else:
                    reader = EditorReader(self.backend)
                    reader.sensitive, reader.selection = change == "sensitive", change == "selection"
                    self.engine.context_policy.reader = reader
                    self.settings.set("detection.context_read_field", True)
                self.send(replace(held, pressed=False))
                self.assertEqual(self.backend.text, "ghbd")

    def test_bundled_prefix_model_corrects_partial_words_in_both_directions(self) -> None:
        self.engine.prefix_model = PrefixModel.load()
        for original, source, expected, remaining in (("ghbd", 0, "прив", "ет"), ("рудд", 1, "hell", "o")):
            self.reset_editor(source)
            self.type(original)
            self.assertEqual(self.backend.text, expected)
            self.type(remaining + "  ")
            self.assertEqual(self.backend.text, expected + remaining + "  ")

    def test_bundled_model_preserves_native_words_in_comments_and_code_context(self) -> None:
        self.engine.prefix_model = PrefixModel.load()
        for before, word, group, app in (
            ("// описание ", "hello", 0, "Code"), ("// пояснение ", "привет", 1, "Code"),
            ("# описание ", "программирование", 1, "WindowsTerminal"),
            ("const value = ", "process", 0, "Code"), ("", "reference", 0, "TestEditor"),
        ):
            with self.subTest(before=before, word=word):
                self.reset_editor(group)
                self.backend.text, self.backend.caret = before, len(before)
                with patch.object(self.backend, "active_application", return_value=app):
                    self.engine.context_policy.stream.focus(app, self.backend.window)
                    # A valid field snapshot is used, without injecting the preamble.
                    self.settings.set("detection.context_read_field", True)
                    self.engine.context_policy.reader = EditorReader(self.backend)
                    self.type(word)
                self.assertEqual(self.backend.text, before + word)


def load_tests(loader: unittest.TestLoader, tests: unittest.TestSuite, pattern: str | None) -> unittest.TestSuite:
    return unittest.TestSuite(
        EarlyContextTests(name) for name in EarlyContextTests.__dict__ if name.startswith("test_")
    )
