"""Physical span labels, parent-family closure and bounded sampling."""
from __future__ import annotations

from collections import defaultdict
from dataclasses import replace
from pathlib import Path
import sys
from typing import cast
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

from context_action_spans import _Capture, _body, _capture, _keys, _label, _select, build_span_curriculum, provenance
from context_physical_keys import translated
from evaluate_context_action_sequences import SequencePlan
from freeze_context_action_corpus import CorpusRow
from keyswitch.context_model import ContextEvidence
from keyswitch.input_context import FieldContext
from keyswitch.language_model import LanguageModel


def row(word: str, group: int, family: str = "") -> CorpusRow:
    return CorpusRow(word, word, group, "", "", word, family or word, "doc:" + word,
                     "ru" if group else "en", "authored", "fixture", "1", "1", " ", "", "",
                     "NOUN", "", "", "authored", True, "train")


class SpanCurriculumTests(unittest.TestCase):
    def models(self) -> dict[int, LanguageModel]:
        return {0: LanguageModel("en_US", {"hello": 5000, "world": 5000}, "fixture", enable_spellcheck=False),
                1: LanguageModel("ru_RU", {"привет": 5000, "работа": 5000}, "fixture", enable_spellcheck=False)}

    def test_real_engine_produces_correct_and_wrong_frames_with_actual_tails_and_spacing(self) -> None:
        rows = [row("hello", 0), row("world", 0), row("привет", 1), row("работа", 1)]
        actual = build_span_curriculum(rows, self.models(), profile="portable", maximum_families=4)
        self.assertEqual(actual.counts["selected_families"], 4)
        self.assertEqual(actual.counts["sequences"], 48)
        self.assertEqual({frame.action for frame in actual.frames}, {"keep", "convert"})
        self.assertGreater(actual.counts["tail_nonempty"], 0)
        self.assertGreater(actual.counts["repeated_space_before"], 0)
        self.assertTrue(any(frame.evidence.literal_tail == "..." for frame in actual.frames))
        mass: defaultdict[str, float] = defaultdict(float)
        originals = {item.identifier: item for item in rows}
        for frame in actual.frames:
            parent = originals[frame.source_identifier]
            item = frame.evidence
            if frame.action == "convert":
                self.assertEqual(item.alternative, parent.original)
                self.assertEqual(item.original, translated(parent.original, cast(int, parent.group)))
            else:
                self.assertEqual(item.original, parent.original)
            # The control mode now runs the engine's idle callbacks, so a word can also be
            # decided on pause, before its boundary key arrives; such a frame carries no
            # boundary character, exactly as the engine passes it at runtime.
            self.assertIn(item.boundary_text, {"", " ", ".", ",", ";", "!"})
            self.assertEqual(item.trigger, "pause" if item.boundary_text == "" else "space"
                             if item.boundary_text == " " else item.trigger)
            self.assertTrue(item.field.before.rstrip() in originals)
            self.assertEqual(frame.parent_family, parent.family)
            mass[frame.parent_family] += frame.sample_weight
        self.assertEqual(set(mass), {item.family for item in rows})
        for value in mass.values():
            self.assertAlmostEqual(value, 1.0)

    def test_internal_slash_fragments_never_become_new_supervised_families(self) -> None:
        rows = [row("hello", 0), row("world", 0), row("hello/world", 0, "slash")]
        actual = build_span_curriculum(rows, self.models(), profile="portable", maximum_families=8)
        self.assertGreater(actual.counts.get("skipped_nonparent_body", 0), 0)
        self.assertNotIn("slash", {frame.parent_family for frame in actual.frames})

    def test_counterfactual_requires_exact_whole_prefix_and_preserves_literal_tail(self) -> None:
        item = ContextEvidence("ghbdtn", "привет", 0, FieldContext("editor", "stream", "hello  ", "", "unknown"),
                               literal_tail="...", boundary_text=" ")
        capture = _Capture(item, "hello  ghbdtn... ", 17)
        self.assertEqual(_label(capture, "hello  привет... "), "convert")
        self.assertEqual(_label(capture, capture.observed), "keep")
        self.assertIsNone(_label(capture, "hello привет... "))
        self.assertIsNone(_label(replace(capture, observed="changed selection"), "hello  привет... "))
        self.assertIsNone(_label(replace(capture, evidence=replace(item, original="", literal_tail="", boundary_text="")), "other"))

    def test_inexpressible_whole_prefix_is_counted_without_inventing_an_action(self) -> None:
        item = ContextEvidence("hello", "руддщ", 0, FieldContext("editor", "stream", "", "", "unknown"), boundary_text=" ")
        capture = _Capture(item, "externally changed hello ", 100)
        with patch("context_action_spans._capture", return_value=[capture]):
            actual = build_span_curriculum([row("hello", 0)], self.models(), profile="portable", maximum_families=2)
        self.assertEqual(actual.counts["skipped_inexpressible"], 12)
        self.assertEqual(actual.frames, ())
        self.assertEqual(actual.counts["retained_families"], 0)

    def test_sampling_is_deterministic_group_capped_and_rejects_unrepresentable_sources(self) -> None:
        rows = [row("hello", 0), row("world", 0), row("привет", 1), row("работа", 1),
                row("café", 0), row("123", 0), row("x" * 65, 0),
                replace(row("mixed", 0), group=None), replace(row("hidden", 0), layout_representable=False)]
        selected, _ = _select(rows, 2)
        reverse, _ = _select(list(reversed(rows)), 2)
        self.assertEqual(selected, reverse)
        self.assertEqual({item.group for item in selected}, {0, 1})
        self.assertEqual(len(selected), 2)
        self.assertEqual(_select([row("x", 0)], 2)[0], [])
        self.assertEqual(_body("ПРИВЕТ", 1), _body("привет", 1))
        self.assertNotEqual(_body("привет", 1), _body("приве", 1))
        with self.assertRaisesRegex(ValueError, "unrepresentable"):
            _keys("\u00a0", 0)

    def test_split_and_budget_guards_run_before_replay(self) -> None:
        with patch("context_action_spans._capture") as capture:
            for split in ("test", "calibration", "development", "quarantine"):
                with self.assertRaisesRegex(ValueError, "declared split"):
                    build_span_curriculum([replace(row("hello", 0), split=split)], self.models(), profile="portable")
            with self.assertRaisesRegex(ValueError, "declared split"):
                build_span_curriculum([replace(row("hello", 0), quarantine_reasons=("exposed",))], self.models(), profile="portable")
            for invalid in (0, 1, 3, 130, True):
                with self.assertRaisesRegex(ValueError, "family budget"):
                    build_span_curriculum([], self.models(), profile="portable", maximum_families=invalid)
            with self.assertRaisesRegex(ValueError, "explicit lexical profile"):
                build_span_curriculum([], {}, profile="portable")
            with self.assertRaisesRegex(ValueError, "explicit lexical profile"):
                build_span_curriculum([], self.models(), profile="unknown")
            with self.assertRaisesRegex(ValueError, "never accepts the test"):
                build_span_curriculum([], self.models(), profile="portable", expected_split="test")
            for split in ("development", "calibration"):
                result = build_span_curriculum([], self.models(), profile="portable", expected_split=split)
                self.assertEqual(result.split, split)
                self.assertEqual(result.frames, ())
            capture.assert_not_called()

    def test_capture_rejects_execution_errors_and_any_unexpected_correction(self) -> None:
        plan = SequencePlan("fixture", "fixture", 0, False, "", (), (), 0, 0, 0, None)
        outcomes: tuple[dict[str, object], ...] = ({"error": "failed", "corrections": []}, {"error": None, "corrections": [{}]})
        for result in outcomes:
            with patch("context_action_spans.replay", return_value=result), self.assertRaisesRegex(ValueError, "without corrections"):
                _capture(plan, self.models())
        hashes = provenance()
        self.assertIn("tools/context_action_spans.py", hashes)
        self.assertIn("src/keyswitch/engine.py", hashes)
        self.assertTrue(all(len(value) == 64 for value in hashes.values()))
