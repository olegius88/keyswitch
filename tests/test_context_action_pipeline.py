"""The context-v3 trainer's back ends give what the serial fit gives, frame by frame and byte by byte."""

from __future__ import annotations

from collections.abc import Iterable, Mapping, Sequence
from dataclasses import replace
from pathlib import Path
import json
import sys
import tempfile
from typing import cast
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
if not sys.platform.startswith("linux"):
    raise unittest.SkipTest("the context-v3 trainer runs on Linux: it forks its workers and needs NumPy")

import context_action_pipeline as pipeline
import train_context_action_model as trainer
from context_action_spans import SpanCurriculum, SpanFrame
from context_optimizer import Packed
from fixture_values.counts import (
    BACK_END_FIXTURE_FRAMES,
    BACK_END_FIXTURE_FRAMES_PER_TASK,
    BACK_END_FIXTURE_JOBS,
    BACK_END_FIXTURE_NAMES,
    BACK_END_FIXTURE_ROWS,
)
from fixture_values.scores import BACK_END_FIXTURE_FEATURE_VALUE, BACK_END_FIXTURE_KEEP_IMPORTANCE, FEATURE_MASS_OVERFLOW_WEIGHT
from keyswitch.constants.model_protocol import CALIBRATION, FITTING_SPLITS as SPLITS, PORTABLE, PROFILES, REFERENCE_HUNSPELL, TRAIN
from keyswitch.constants.training import (
    CONTEXT_ACTION_BACK_END_SOURCES,
    CONTEXT_ACTION_BACKEND_AUTO,
    CONTEXT_ACTION_BACKEND_CPU,
    CONTEXT_ACTION_BACKEND_GPU,
)
from keyswitch.context_model import ACTIONS, ContextAction, ContextEvidence
from keyswitch.detector import LanguageDetector
from keyswitch.input_context import FieldContext
from keyswitch.intent_model import LinearNgramModel
from keyswitch.language_model import LanguageModel
from keyswitch.ortho_model import OrthoModel
from train_context_model import CapturedSource

FIELD = FieldContext("Telegram", "fixture", "", "", "text")
SOURCES = [CapturedSource(Path("first.tsv.xz"), 1.0), CapturedSource(Path("second.tsv.xz"), 1.0)]


def frame(identifier: str, action: str = "keep", weight: float = 1.0) -> trainer.ActionRow:
    return trainer.ActionRow(identifier, "word", 0, FIELD, "space", "", cast(ContextAction, action), "fixture", " ", weight)


# Stand-ins for the curricula and the features: fast, deterministic, and different per profile and split.
def fake_historical(intent: object, models: object = None) -> list[trainer.ActionRow]:
    return [frame(f"history:{number}", ACTIONS[number % len(ACTIONS)]) for number in range(BACK_END_FIXTURE_FRAMES)]


def fake_captured(source: CapturedSource, options: Mapping[str, object]) -> tuple[list[trainer.ActionRow], dict[str, object]]:
    return [frame("captured:" + source.path.name, "convert")], {"file": source.path.name}


def fake_capitals(rows: Sequence[object], refused: frozenset[str], options: Mapping[str, object],
                  models: object = None) -> tuple[list[trainer.ActionRow], dict[str, object]]:
    return [frame("capital", weight=BACK_END_FIXTURE_KEEP_IMPORTANCE)], {"rows": len(rows)}


def fake_chain(inputs: trainer.FitInputs, profile: str, split: str,
               rows: Sequence[trainer.ActionRow]) -> tuple[list[trainer.ActionRow], dict[str, object]]:
    """Keeps most frames, changes the first one and adds one: the rest of a fit sees all three kinds."""
    result = list(rows)
    result[0] = replace(result[0], sample_weight=BACK_END_FIXTURE_FEATURE_VALUE)
    result.append(frame(f"planned:{profile}:{split}", "wait"))
    return result, {"profile": profile, "split": split, "rows": len(result)}


def fake_spans(rows: Sequence[object], models: object, *, profile: str, maximum_families: int,
               expected_split: str) -> SpanCurriculum:
    evidence = ContextEvidence(profile, expected_split, 0, FIELD, "space", False, False, True, -0.0)
    return SpanCurriculum((SpanFrame(evidence, "convert", "family", "source", f"{profile}:{expected_split}", 1.0),),
                          {"frames": 1, "families": maximum_families}, profile, maximum_families, expected_split)


def fake_evidence(row: trainer.ActionRow, split: str, detector: object, ortho: object) -> str:
    return f"{row.identifier}|{split}|{detector}|{row.sample_weight}"


def fake_features(item: object) -> dict[str, float]:
    text = item if isinstance(item, str) else repr(item)
    return {"shared": BACK_END_FIXTURE_FEATURE_VALUE, "frame:" + text: 1.0, f"parity:{len(text) % len(PROFILES)}": -0.0}


def square(value: int) -> int:
    return value * value


def fixture_inputs() -> trainer.FitInputs:
    inputs = trainer.FitInputs.__new__(trainer.FitInputs)
    inputs.options = {"keep_importance": BACK_END_FIXTURE_KEEP_IMPORTANCE, "captured_curriculum": {"manifest": "model/context_v1/captured/manifest.json"},
                      "capital_citation_curriculum": {}, "span_maximum_families": {split: len(PROFILES) for split in SPLITS}}
    inputs.source_rows = {split: [] for split in SPLITS}
    inputs.frames = {split: [frame(f"{split}:{number}", ACTIONS[number % len(ACTIONS)]) for number in range(BACK_END_FIXTURE_FRAMES)]
                     for split in SPLITS}
    inputs.profiles = list(PROFILES)
    inputs.refused = frozenset()
    inputs.intent = cast(LinearNgramModel, None)
    inputs.ortho = cast(OrthoModel, None)
    inputs.lexicons = {False: {}, True: {}}
    inputs.lexical_models = {profile: {} for profile in PROFILES}
    inputs.detectors = cast(dict[str, LanguageDetector], {profile: profile for profile in PROFILES})
    return inputs


def serial(inputs: trainer.FitInputs) -> dict[tuple[str, str], list[tuple[dict[str, float], int, float]]]:
    """The serial fit's frames and features, written out with the same stand-ins."""
    extra = [*fake_historical(None), *(row for source in SOURCES for row in fake_captured(source, {})[0]),
             *fake_capitals([], frozenset(), {})[0]]
    result: dict[tuple[str, str], list[tuple[dict[str, float], int, float]]] = {}
    for profile in PROFILES:
        for split in SPLITS:
            rows = inputs.frames[split] if split != TRAIN else trainer.training_order([*inputs.frames[TRAIN], *extra])
            chained, _report = fake_chain(inputs, profile, split, rows)
            frames = trainer.prepared_frames(chained, fake_spans([], None, profile=profile, maximum_families=len(PROFILES),
                                                                   expected_split=split), split)
            rows_out = []
            for _identifier, item in frames:
                label = ACTIONS.index(item.action)
                features = fake_features(fake_evidence(item, split, profile, None) if isinstance(item, trainer.ActionRow) else item.evidence)
                rows_out.append((features, label, item.sample_weight * (BACK_END_FIXTURE_KEEP_IMPORTANCE if label == 0 else 1.0)))
            result[profile, split] = rows_out
    return result


def exact(rows: Iterable[tuple[dict[str, float], int, float]]) -> list[tuple[list[tuple[str, str]], int, str]]:
    """Rows compared by the repr of every value: -0.0 and 0.0 differ."""
    return [(sorted((name, repr(value)) for name, value in features.items()), label, repr(importance))
            for features, label, importance in rows]


class BuildTest(unittest.TestCase):
    def build(self, jobs: int) -> pipeline.FeatureSet:
        inputs = fixture_inputs()
        with patch("train_context_action_model.historical_curriculum", fake_historical), \
                patch("train_context_action_model.captured_source_curriculum", fake_captured), \
                patch("train_context_action_model.capital_citation_curriculum", fake_capitals), \
                patch("train_context_action_model.frame_chain", fake_chain), \
                patch("train_context_action_model.frame_evidence", fake_evidence), \
                patch("context_action_pipeline.build_span_curriculum", fake_spans), \
                patch("context_action_pipeline.extract_action_features", fake_features), \
                patch("context_action_pipeline.CONTEXT_ACTION_FRAMES_PER_TASK", BACK_END_FIXTURE_FRAMES_PER_TASK), \
                patch("context_action_pipeline.STATE", pipeline.State(inputs, BACK_END_FIXTURE_KEEP_IMPORTANCE, SOURCES)):
            return pipeline.Build(inputs, CONTEXT_ACTION_BACKEND_CPU, jobs).run()

    def test_one_and_several_processes_give_the_serial_frames_features_and_reports(self) -> None:
        expected = serial(fixture_inputs())
        for jobs in (1, BACK_END_FIXTURE_JOBS):
            with self.subTest(jobs=jobs):
                built = self.build(jobs)
                for key, rows in expected.items():
                    self.assertEqual(exact(built.rows(*key)), exact(rows), key)
                self.assertEqual(built.captured, {source.path.name: {"file": source.path.name} for source in SOURCES})
                self.assertEqual(built.capitals, {"rows": 0})
                self.assertEqual(built.spans[PORTABLE, TRAIN], {"frames": 1, "families": len(PROFILES)})
                self.assertEqual(built.chains[REFERENCE_HUNSPELL, CALIBRATION]["split"], CALIBRATION)
                self.assertEqual(built.columns[PORTABLE, TRAIN].importance.tolist(), [importance for _features, _label, importance
                                                                                     in expected[PORTABLE, TRAIN]])

    def test_a_frame_the_serial_fit_rejects_stops_the_fit(self) -> None:
        import numpy as np

        only = [frame("only")]
        columns = pipeline.Columns.of(np.zeros(0, np.uint32), np.zeros(0), np.zeros(1, np.int64), only, float("nan"))
        with self.assertRaisesRegex(ValueError, "importance"):
            columns.check(TRAIN, only)
        columns = pipeline.Columns.of(np.zeros(1, np.uint32), np.array([float("inf")]), np.ones(1, np.int64), only, 1.0)
        with self.assertRaisesRegex(ValueError, "non-finite"):
            columns.check(TRAIN, only)


class SchedulerTest(unittest.TestCase):
    def test_tasks_start_by_priority_and_callbacks_may_add_tasks(self) -> None:
        for jobs in (1, BACK_END_FIXTURE_JOBS):
            with self.subTest(jobs=jobs):
                executor = pipeline.Executor(jobs)
                scheduler = pipeline.Scheduler(executor, 1)
                seen: list[int] = []

                def later(value: int) -> None:
                    seen.append(value)
                    scheduler.add(pipeline.CRITICAL, square, value + 1, seen.append)

                scheduler.add(pipeline.AHEAD, square, BACK_END_FIXTURE_JOBS, seen.append)
                scheduler.add(pipeline.CRITICAL, square, 1, later)
                scheduler.run()
                executor.close()
                self.assertEqual(seen, [1, square(1 + 1), square(BACK_END_FIXTURE_JOBS)])


class PackingTest(unittest.TestCase):
    def rows(self) -> list[tuple[dict[str, float], int, float]]:
        names = [f"name:{number}" for number in range(BACK_END_FIXTURE_NAMES)]
        return [({names[(row * position) % len(names)]: (BACK_END_FIXTURE_FEATURE_VALUE * position if position % len(PROFILES) else -0.0)
                  for position in range(1, row % len(names) + 1)}, row % len(ACTIONS), 1.0 + row)
                for row in range(BACK_END_FIXTURE_ROWS)]

    def columns(self, rows: Sequence[tuple[dict[str, float], int, float]], reverse: bool) -> tuple[pipeline.Columns, list[str]]:
        import numpy as np

        vocabulary = pipeline.Vocabulary()
        ids: list[int] = []
        values: list[float] = []
        for features, _label, _importance in rows:
            entries = sorted(features.items(), reverse=reverse)
            ids.extend(vocabulary.ids([name for name, _ in entries]).tolist())
            values.extend(value for _, value in entries)
        columns = pipeline.Columns(np.array(ids, np.uint32), np.array(values, np.float64),
                                   np.array([len(features) for features, _, _ in rows], np.int64),
                                   np.array([label for _, label, _ in rows], np.uint8),
                                   np.array([importance for _, _, importance in rows], np.float64),
                                   np.array([importance for _, _, importance in rows], np.float64))
        return columns, vocabulary.names

    def assert_packed_equal(self, actual: Packed, expected: Packed) -> None:
        for name in ("offsets", "indices", "values", "labels", "importance"):
            self.assertEqual(getattr(actual, name).tobytes(), getattr(expected, name).tobytes(), name)

    def test_packing_columns_equals_packed_build_in_any_entry_order(self) -> None:
        import numpy as np

        rows = self.rows()
        selected = sorted({name for features, _, _ in rows for name in features})[::len(PROFILES)]
        for reverse in (False, True):
            with self.subTest(reverse=reverse):
                columns, names = self.columns(rows, reverse)
                column = np.full(len(names), -1, dtype=np.int64)
                for position, name in enumerate(selected):
                    column[names.index(name)] = position
                # The serial fit packs rows read back from canonical JSON lines, whose keys are sorted.
                read_back = [(dict(sorted(features.items())), label, importance) for features, label, importance in rows]
                self.assert_packed_equal(pipeline.packed([columns], column), Packed.build(read_back, selected))
                self.assertEqual(exact(columns.rows(names)), exact(rows))

    def test_masses_follow_feature_mass_in_the_serial_order(self) -> None:
        rows = self.rows()
        columns, names = self.columns(rows, False)
        feature_set = pipeline.FeatureSet(names, {(profile, TRAIN): columns for profile in PROFILES}, list(PROFILES), {}, {}, {}, {})
        reference = trainer.FeatureMass()
        for _profile in PROFILES:
            for features, _label, importance in rows:
                reference.add(features, importance)
        self.assertEqual({name: repr(value) for name, value in feature_set.masses().items()},
                         {name: repr(value) for name, value in reference.values.items()})
        columns.weights[:] = FEATURE_MASS_OVERFLOW_WEIGHT
        with self.assertRaisesRegex(ValueError, "overflow"):
            feature_set.masses()

    def test_gather_index_reads_rows_in_the_order_asked(self) -> None:
        import numpy as np

        starts = np.array([len(PROFILES), 0], np.int64)
        counts = np.array([1, len(PROFILES)], np.int64)
        self.assertEqual(pipeline.gather_index(starts, counts).tolist(), [len(PROFILES), 0, 1])


class ChoiceTest(unittest.TestCase):
    def test_backends_and_jobs(self) -> None:
        self.assertEqual(pipeline.choose_backend(CONTEXT_ACTION_BACKEND_CPU), CONTEXT_ACTION_BACKEND_CPU)
        with self.assertRaises(ValueError):
            pipeline.choose_backend("tpu")
        with patch("context_action_pipeline.cuda_status", return_value=(False, "fixture")):
            self.assertEqual(pipeline.choose_backend(CONTEXT_ACTION_BACKEND_AUTO), CONTEXT_ACTION_BACKEND_CPU)
            with self.assertRaisesRegex(RuntimeError, "fixture"):
                pipeline.choose_backend(CONTEXT_ACTION_BACKEND_GPU)
        self.assertEqual(pipeline.choose_jobs(BACK_END_FIXTURE_JOBS), BACK_END_FIXTURE_JOBS)
        self.assertGreaterEqual(pipeline.choose_jobs(None), 1)
        with self.assertRaises(ValueError):
            pipeline.choose_jobs(0)
        with patch("context_action_pipeline.available_memory", return_value=0):
            self.assertEqual(pipeline.choose_jobs(None), 1)

    def test_the_parity_record_names_every_changed_source(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            record = Path(directory) / "parity.json"
            self.assertIsNone(pipeline.parity_record(record))
            pipeline.write_parity_record(record)
            recorded = pipeline.parity_record(record)
            self.assertEqual(recorded, pipeline.mirrored_hashes())
            stale = dict(cast(dict[str, str], recorded), **{"src/keyswitch/detector.py": "0"})
            record.write_text(json.dumps({"sources": stale}), encoding="utf-8")
            with patch("context_action_pipeline.ROOT", Path(directory)), \
                    patch("context_action_pipeline.mirrored_hashes", return_value=recorded):
                (Path(directory) / "tools/context_action_kernels").mkdir(parents=True)
                (Path(directory) / "tools/context_action_kernels/parity.json").write_text(json.dumps({"sources": stale}), encoding="utf-8")
                available, reason = pipeline.cuda_status()
        self.assertFalse(available)
        self.assertIn("src/keyswitch/detector.py", reason)

    def test_every_kernel_file_is_pinned(self) -> None:
        root = Path(__file__).resolve().parents[1]
        kernels = {path.relative_to(root).as_posix() for path in (root / "tools/context_action_kernels").iterdir()}
        self.assertEqual(set(CONTEXT_ACTION_BACK_END_SOURCES),
                         kernels | {"tools/context_action_pipeline.py", "tools/context_action_cuda.py", "tools/context_action_mass.c"})
        self.assertLessEqual(set(CONTEXT_ACTION_BACK_END_SOURCES), set(trainer.provenance()))


class ProcessStateTest(unittest.TestCase):
    def test_dictionaries_are_bound_as_a_serial_fit_leaves_them_and_everything_is_restored(self) -> None:
        import keyswitch.context_policy as policy
        import keyswitch.intent_model as intent_module
        from keyswitch.context_policy import ContextPolicy
        from keyswitch.ortho_model import OrthoEvidence

        inputs = fixture_inputs()
        first = {group: LanguageModel(locale, {"word": 1}, "fixture", enable_spellcheck=False)
                 for group, locale in enumerate(("en_US", "ru_RU"))}
        inputs.lexical_models = {profile: first for profile in PROFILES}
        inputs.ortho = OrthoModel.load(trainer.ROOT / "src/keyswitch/resources/models/ortho_v1.json")
        def engine(script: str, word: str) -> bool:
            return word == "engine"

        inputs.ortho._known = engine
        typo, score, fnv, shared_before = (getattr(policy, "one_typo_from_word"), OrthoModel.score, intent_module.fnv1a64,
                                           ContextPolicy._shared_ortho)
        evidence = OrthoEvidence("ghbdtn", "lower", "en")
        plain = OrthoModel.score(inputs.ortho, evidence)
        with patch("train_context_model.captured_sources", return_value=SOURCES), pipeline.bound_process_state(inputs):
            shared = cast(tuple[OrthoModel, str], ContextPolicy._shared_ortho)[0]
            known = shared._known
            self.assertTrue(known is not None and known("en", "word") and not known("en", "engine"))
            self.assertIs(inputs.ortho._known, engine)
            self.assertIsNot(getattr(policy, "one_typo_from_word"), typo)
            self.assertEqual(OrthoModel.score(inputs.ortho, evidence), plain)
            self.assertEqual(OrthoModel.score(inputs.ortho, evidence), plain)
            self.assertEqual(intent_module.fnv1a64("word"), fnv("word"))
            self.assertIs(pipeline.state().inputs, inputs)
        self.assertEqual((getattr(policy, "one_typo_from_word"), OrthoModel.score, intent_module.fnv1a64,
                          ContextPolicy._shared_ortho), (typo, score, fnv, shared_before))
        self.assertIsNone(pipeline.STATE)
        with self.assertRaises(RuntimeError):
            pipeline.state()


if __name__ == "__main__":
    unittest.main()
