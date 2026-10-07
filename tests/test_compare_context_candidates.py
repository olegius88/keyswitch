"""The candidate comparison scores frames as the trainer does and tells the classes apart."""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from array import array
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

import compare_context_candidates as comparison
from fixture_values.scores import DOMINANT_BIAS_WEIGHT, EPOCH_SELECTION_BELOW_THRESHOLD_SCORE, EPOCH_SELECTION_RUNTIME_THRESHOLD
from keyswitch.constants.model_protocol import CALIBRATION, DEVELOPMENT
from keyswitch.constants.models import CONTEXT_ACTION_FEATURE_VERSION
from keyswitch.context_model import ACTIONS, ContextModel

KEEP, CONVERT = ACTIONS.index("keep"), ACTIONS.index("convert")
# The letters the runtime's support check reads (ContextModel.supports_features).
LETTERS = {"source:char:0:1:a": 1.0, "target:char:0:1:b": 1.0}


def model(action: str, version: str) -> ContextModel:
    """A model that answers every supported frame with `action`, sure of it."""
    bias = tuple(DOMINANT_BIAS_WEIGHT if name == action else 0.0 for name in ACTIONS)
    weights = {"bias": bias, **{name: (0.0,) * len(ACTIONS) for name in LETTERS}}
    return ContextModel(weights, version, EPOCH_SELECTION_RUNTIME_THRESHOLD, feature_version=CONTEXT_ACTION_FEATURE_VERSION)


def row(label: int, *extra: str, supported: bool = True) -> tuple[dict[str, float], int, float]:
    features = {"bias": 1.0, **(LETTERS if supported else {}), **{name: 1.0 for name in extra}}
    return features, label, 1.0


class CandidateComparisonTests(unittest.TestCase):
    def test_a_frame_belongs_to_the_first_head_whose_prefix_it_carries(self) -> None:
        self.assertEqual(comparison.head_class(["bias", "abbr|bias:direction:1", "alone|bias"]), "abbreviation")
        self.assertEqual(comparison.head_class(["letter|bias"]), "letter")
        self.assertEqual(comparison.head_class(["kept|bias"]), "kept")
        self.assertEqual(comparison.head_class(["bias", "source:char:0:1:a"]), comparison.BASE_CLASS)

    def test_frames_are_converted_as_the_runtime_does_and_counted_by_class(self) -> None:
        rows = [row(CONVERT), row(KEEP, "abbr|bias"), row(CONVERT, supported=False), row(CONVERT, "letter|bias")]
        scored = comparison.score_split({"keeps": model("keep", "context-v3-keeps"),
                                         "converts": model("convert", "context-v3-converts")}, rows)
        convert_rows = sum(label == CONVERT for _features, label, _importance in rows)
        # A frame the runtime cannot support becomes a suggestion, never a conversion.
        right = sum(label == CONVERT and LETTERS.keys() <= features.keys() for features, label, _importance in rows)
        false = sum(label != CONVERT and LETTERS.keys() <= features.keys() for features, label, _importance in rows)
        self.assertEqual(scored["keeps"].by_class[comparison.ALL_FRAMES],
                         {"rows": len(rows), "convert_rows": convert_rows, "right": 0, "false": 0, "net": 0})
        self.assertEqual(scored["keeps"].differences, {})
        converts = scored["converts"]
        self.assertEqual(converts.by_class[comparison.ALL_FRAMES],
                         {"rows": len(rows), "convert_rows": convert_rows, "right": right, "false": false, "net": right - false})
        self.assertEqual(converts.by_class["abbreviation"], {"rows": 1, "convert_rows": 0, "right": 0, "false": 1, "net": -1})
        self.assertEqual(converts.by_class["letter"]["right"], 1)
        self.assertEqual(converts.differences[comparison.ALL_FRAMES], {"gained_right": right, "gained_false": false})
        self.assertEqual(converts.differences["abbreviation"], {"gained_false": 1})
        self.assertEqual((converts.model_version, converts.threshold), ("context-v3-converts", EPOCH_SELECTION_RUNTIME_THRESHOLD))

    def test_a_conversion_the_first_artifact_makes_and_the_other_does_not_is_lost(self) -> None:
        self.assertEqual(comparison.differences([True, True, False], [False, False, False], [CONVERT, KEEP, KEEP],
                                                ["base", "start", "base"]),
                         {comparison.ALL_FRAMES: {"lost_right": 1, "lost_false": 1}, "base": {"lost_right": 1},
                          "start": {"lost_false": 1}})

    def test_a_conversion_below_the_threshold_is_none(self) -> None:
        scores = [0.0] * len(ACTIONS)
        scores[CONVERT] = EPOCH_SELECTION_BELOW_THRESHOLD_SCORE
        scores[KEEP] = 1.0 - EPOCH_SELECTION_BELOW_THRESHOLD_SCORE
        self.assertEqual(comparison.converted(array("d", scores), EPOCH_SELECTION_RUNTIME_THRESHOLD), [False])
        self.assertEqual(comparison.converted(array("d", scores), EPOCH_SELECTION_BELOW_THRESHOLD_SCORE), [True])

    def test_the_report_names_every_split_profile_and_artifact_and_reads_no_test(self) -> None:
        rows = {(profile, split): [row(CONVERT), row(KEEP, "start|bias")]
                for profile in ("portable", "reference_hunspell") for split in (CALIBRATION, DEVELOPMENT)}
        features = SimpleNamespace(rows=lambda profile, split: iter(rows[profile, split]))
        inputs = SimpleNamespace(profiles=["portable", "reference_hunspell"])
        models = {"first.json": model("keep", "context-v3-first"), "second.json": model("convert", "context-v3-second")}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / "report.json"
            with (patch.object(ContextModel, "load", side_effect=lambda path: models[path.name]),
                  patch("train_context_action_model.recipe", return_value={}),
                  patch("train_context_action_model.load_inputs", return_value=inputs),
                  patch("context_action_pipeline.choose_backend", return_value="cpu"),
                  patch("context_action_pipeline.choose_jobs", return_value=1),
                  patch("context_action_pipeline.build_features", return_value=features) as build,
                  patch("builtins.print") as printed):
                self.assertEqual(comparison.main(["--corpus", str(root), "--artifact", f"first={root / 'first.json'}",
                                                  "--artifact", f"second={root / 'second.json'}", "--output", str(output)]), 0)
            build.assert_called_once()
            report = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(sorted(report), sorted([CALIBRATION, DEVELOPMENT]))
            second = report[CALIBRATION]["portable"]["second"]
            self.assertEqual(second["by_class"]["all"], {"rows": len(rows["portable", CALIBRATION]), "convert_rows": 1,
                                                         "right": 1, "false": 1, "net": 0})
            self.assertEqual(second["differences"]["start"], {"gained_false": 1})
            lines = [call.args[0] for call in printed.call_args_list]
            # One line per split, profile and artifact.
            self.assertEqual(len(lines), len(report) * len(inputs.profiles) * len(models))
            self.assertIn("differs from the first", lines[1])
            self.assertNotIn("differs", lines[0])

    def test_an_artifact_needs_a_name_and_a_path_and_a_name_of_its_own(self) -> None:
        self.assertEqual(comparison.parse_artifact("installed=a/b.json"), ("installed", Path("a/b.json")))
        for text in ("a/b.json", "=a.json", "name="):
            with self.subTest(text=text), self.assertRaises(Exception):
                comparison.parse_artifact(text)
        with patch("sys.stderr"), self.assertRaises(SystemExit):
            comparison.main(["--corpus", "c", "--artifact", "same=a.json", "--artifact", "same=b.json"])


if __name__ == "__main__":
    unittest.main()
