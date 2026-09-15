"""Small semantic fixtures for paired prefix data and sequence calibration."""
from __future__ import annotations

from collections import defaultdict
from array import array
from dataclasses import replace
import gzip
import hashlib
import json
import math
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from keyswitch.input_context import FieldContext
from keyswitch.language_model import LanguageModel
from keyswitch.prefix_model import PrefixInput, PrefixModel
from keyswitch.prefix_schema import VersionedPrefixModel
import sys

TOOLS = str(Path(__file__).resolve().parents[1] / "tools")
if TOOLS not in sys.path:
    sys.path.insert(0, TOOLS)
from prefix_v2_corpus import ParentWord, PrefixContext, PrefixFrame, generate_frames, load_parents
from train_prefix_v2_model import (
    SequenceScore, archive_sources, calibrate, calibration_passed, feature_vocabulary, fit, importance,
    recipe, sequence_metrics,
    artifact_payload, assess_prefix_epoch, sequence_metadata, scores_from_probabilities,
)


def lexical_fixture() -> dict[str, dict[int, LanguageModel]]:
    return {"portable": {
        0: LanguageModel("en_US", {"example": 5000, "wording": 2000}, "prefix-fixture-en", enable_spellcheck=False),
        1: LanguageModel("ru_RU", {"пример": 5000, "словарь": 2000}, "prefix-fixture-ru", enable_spellcheck=False),
    }}


def sample(*, weight: float = 1.0, values: dict[str, float] | None = None, label: int = 0) -> PrefixFrame:
    return PrefixFrame(PrefixInput("exam", "учфь", 0, FieldContext("Code", "fixture")), label,
                       "exa", "sequence", 4, False, "correct", "portable", weight,
                       {"bias": 1.0} if values is None else values, "pair", "exa", 7)


class PrefixV2TrainingTests(unittest.TestCase):
    def test_each_comment_and_application_has_correct_and_wrong_counterparts(self) -> None:
        parents = [ParentWord(word, group, family, "", family, "correct")
                   for word, group, family in (("example", 0, "exa"), ("пример", 1, "ghb"))]
        frames = list(generate_frames(parents, lexical_fixture()))
        pairs: dict[tuple[str, int], list[PrefixFrame]] = defaultdict(list)
        for row in frames:
            pairs[row.pair_id, row.length].append(row)
        self.assertTrue(any(row.item.field.application == "Code" and row.item.field.before.startswith("//") for row in frames))
        for values in pairs.values():
            self.assertEqual(len(values), 2)
            correct = next(row for row in values if not row.desired)
            wrong = next(row for row in values if row.desired)
            self.assertEqual(correct.label, 0)
            self.assertIn(wrong.label, (1, 2))
            self.assertEqual(correct.item.field, wrong.item.field)
            self.assertEqual(correct.item.original, wrong.item.alternative)
            self.assertEqual(correct.item.alternative, wrong.item.original)
            self.assertEqual(correct.sample_weight, wrong.sample_weight)

    def test_new_app_roles_and_prefixes_divide_one_family_budget(self) -> None:
        parent = ParentWord("example", 0, "exa", "", "one", "correct")
        first = PrefixContext("one", "// explanation ", "Code", "unknown")
        contexts = [first, PrefixContext("two", "// explanation ", "Code", "code"),
                    PrefixContext("three", "// explanation ", "Telegram", "text")]
        for variants in ([first], contexts, [*contexts, replace(first, identifier="duplicate")]):
            frames = list(generate_frames([parent], lexical_fixture(), contexts=variants))
            self.assertAlmostEqual(math.fsum(row.sample_weight for row in frames), 1.0)
            self.assertAlmostEqual(math.fsum(row.sample_weight for row in frames if row.desired), 0.5)
            self.assertTrue(all(math.isfinite(row.sample_weight) and row.sample_weight > 0 for row in frames))

    def test_existing_typo_is_keep_only_without_new_wrong_surface(self) -> None:
        parents = [ParentWord("example", 0, "exa", "", "original", "correct"),
                   ParentWord("exaple", 0, "exa", "", "existing-typo", "typo", "original")]
        frames = list(generate_frames(parents, lexical_fixture()))
        typos = [row for row in frames if row.category == "typo"]
        self.assertTrue(typos)
        self.assertTrue(all(row.label == 0 and not row.desired for row in typos))
        self.assertAlmostEqual(math.fsum(row.sample_weight for row in frames), 1.0)

    def test_existing_technical_surfaces_keep_their_original_context(self) -> None:
        parent = ParentWord("example_value", 0, "exa", "const value = ", "existing-code", "identifier",
                            application="Code", role="code")
        frames = list(generate_frames([parent], lexical_fixture(), contexts=[PrefixContext("other", "// ", "Chat", "text")]))
        self.assertTrue(frames)
        self.assertTrue(all(row.label == 0 and not row.desired and row.full_length == 13 for row in frames))
        self.assertTrue(all(row.item.field.before == "const value = " and row.item.field.application == "Code" for row in frames))
        self.assertAlmostEqual(math.fsum(row.sample_weight for row in frames), 1.0)

    def test_indexes_use_the_supplied_lexicons_without_system_reloading(self) -> None:
        parent = ParentWord("example", 0, "exa", "", "one", "correct")
        first = lexical_fixture()
        second = lexical_fixture()
        second["portable"][0] = LanguageModel("en_US", {"another": 5000}, "prefix-fixture-en", enable_spellcheck=False)
        with patch("keyswitch.language_model.LanguageModel.load", side_effect=AssertionError("system lexicon")):
            values = [[row.values["source:empty"] for row in generate_frames([parent], models)
                       if row.length == 4 and not row.desired] for models in (first, second)]
        self.assertTrue(all(value == 0 for value in values[0]))
        self.assertTrue(all(value == 1 for value in values[1]))

    def test_test_split_is_rejected_before_any_source_access(self) -> None:
        with patch.object(Path, "read_bytes", side_effect=AssertionError("source access")) as read:
            with self.assertRaisesRegex(ValueError, "train/development/calibration"):
                load_parents("test", 1, directory=Path("/absent"))
        read.assert_not_called()

    def test_loader_retains_only_existing_typo_and_verifies_actual_split(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            rows = [{"sequence": index, "source": 0, "text": text, "family": "exa", "before": "",
                     "category": category, "profile": "portable", "length": 1}
                    for index, (text, category) in enumerate((("example", "correct"), ("exaple", "typo"), ("exammple", "typo")))]
            raw = gzip.compress("\n".join(json.dumps(row) for row in rows).encode())
            (directory / "train.jsonl.gz").write_bytes(raw)
            (directory / "corpus.json").write_text(json.dumps({"sha256": {"train": hashlib.sha256(raw).hexdigest()}}))
            parents = load_parents("train", 1, directory=directory)
            self.assertEqual({row.original for row in parents}, {"example", "exaple"})
            (directory / "train.jsonl.gz").write_bytes(raw + b"changed")
            with self.assertRaisesRegex(ValueError, "checksum"):
                load_parents("train", 1, directory=directory)

    def test_divided_semantic_copies_do_not_change_feature_selection(self) -> None:
        other = sample(values={"other": 1.0}, weight=2.0)
        for copies in (1, 3, 10, 1000):
            duplicated = [sample(values={"context": 1.0}, weight=1.0 / copies) for _ in range(copies)]
            self.assertEqual(feature_vocabulary([*duplicated, other], 1.0, 1), ["other"])
            self.assertEqual(feature_vocabulary([*duplicated, other], 1.0, 2), ["context", "other"])
        self.assertEqual(feature_vocabulary([sample(values={"rare": 100.0}, weight=.25)], 1.0, 1), [])

    def test_class_importance_applies_once_after_sample_budget(self) -> None:
        cfg = {"keep_importance": 3.0, "wait_importance": 2.0}
        self.assertEqual(importance(sample(weight=.25), cfg), .75)
        self.assertEqual(importance(sample(weight=.25, label=2), cfg), .5)
        self.assertEqual(importance(sample(weight=.25, label=1), cfg), .25)
        for invalid in (0.0, -1.0, math.nan, math.inf):
            with self.assertRaises(ValueError):
                importance(sample(weight=invalid), cfg)

    def test_schema_two_preserves_pairs_budgets_and_export_loader_contract(self) -> None:
        parent = ParentWord("example", 0, "exa", "", "one", "correct")
        old = list(generate_frames([parent], lexical_fixture()))
        new = list(generate_frames([parent], lexical_fixture(), feature_version=2))
        self.assertEqual([(row.sequence_id, row.length, row.label, row.sample_weight) for row in old],
                         [(row.sequence_id, row.length, row.label, row.sample_weight) for row in new])
        self.assertTrue(all(row.feature_version == 2 for row in new))
        self.assertTrue(all(any("prefix_char:" in name for name in row.values) for row in new))
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "prefix.json"
            for schema in (1, 2):
                payload = artifact_payload({"bias": (0., 20., 0., 0.)}, schema, .999)
                path.write_text(json.dumps(payload))
                loaded = VersionedPrefixModel.load(path)
                self.assertEqual(loaded.feature_version, schema)
                self.assertTrue(loaded.version.startswith(f"prefix-v{schema}-"))
                self.assertEqual(loaded.predict_features({"bias": 1.}).action, "convert")

    def test_epoch_recall_requires_early_conversion_and_zero_false_in_each_profile(self) -> None:
        early = {profile + name: SequenceScore(profile, desired, 7, name, candidates)
                 for profile in ("portable", "reference_hunspell")
                 for name, desired, candidates in (("correct", False, []), ("wrong", True, [(4, .99)]))}
        late = {key: replace(value, candidates=[(7, .99999)] if value.desired else []) for key, value in early.items()}
        early_choice = assess_prefix_epoch(early, thresholds=[.985, .999], loss=.4)
        late_choice = assess_prefix_epoch(late, thresholds=[.985, .999], loss=.01)
        assert early_choice is not None and late_choice is not None
        self.assertGreater(early_choice.rank, late_choice.rank)
        self.assertEqual((early_choice.minimum_recall, late_choice.minimum_recall), (1., 0.))
        unsafe = {key: replace(value, candidates=[(4, 1.)]) if key == "reference_hunspellcorrect" else value
                  for key, value in early.items()}
        self.assertIsNone(assess_prefix_epoch(unsafe, thresholds=[.985, .999], loss=.001))
        # A better portable profile cannot compensate for the worse profile.
        weak = {key: replace(value, candidates=[]) if key == "reference_hunspellwrong" else value
                for key, value in early.items()}
        weak_choice = assess_prefix_epoch(weak, thresholds=[.985], loss=.001)
        assert weak_choice is not None
        self.assertGreater(early_choice.rank, weak_choice.rank)

    def test_development_threshold_is_not_exported_instead_of_calibration(self) -> None:
        dev = {profile + name: SequenceScore(profile, desired, 7, name, candidates)
               for profile in ("portable", "reference_hunspell")
               for name, desired, candidates in (("correct", False, []), ("wrong", True, [(4, .99)]))}
        chosen = assess_prefix_epoch(dev, thresholds=[.985, .999], loss=.1)
        assert chosen is not None
        self.assertEqual(chosen.threshold, .985)
        cal = {key: replace(value, candidates=[(4, .9995)] if value.desired else [(4, .99)])
               for key, value in dev.items()}
        threshold, passed = calibrate(cal, [.985, .999], .70)
        self.assertTrue(passed)
        self.assertEqual(artifact_payload({"bias": (0., 1., 0., 0.)}, 2, threshold)["conversion_threshold"], .999)

    def test_cached_predictions_follow_prefix_execution_support_without_extracting_again(self) -> None:
        base = replace(sample(), desired=True, label=1)
        frames = [base,
                  replace(base, sequence_id="punctuation", item=replace(base.item, original="ex.m")),
                  replace(base, sequence_id="upper", item=replace(base.item, alternative="учФь")),
                  replace(base, sequence_id="short", length=3, item=replace(base.item, original="exa", alternative="учф"))]
        metadata = [sequence_metadata(row) for row in frames]
        probabilities = array("d", [0., 1., 0., 0.]) * len(frames)
        with patch("train_prefix_v2_model.generate_frames", side_effect=AssertionError("re-extraction")):
            result = scores_from_probabilities(metadata, probabilities)
        self.assertEqual(result["portable:sequence"].candidates, [(4, 1.)])
        self.assertTrue(all(not result["portable:" + key].candidates for key in ("punctuation", "upper", "short")))
        for invalid in (array("d", [1.]), array("d", [math.nan, 0., 0., 0.]) * 4,
                        array("d", [1., 1., 0., 0.]) * 4):
            with self.assertRaises(ValueError):
                scores_from_probabilities(metadata, invalid)

    def test_epoch_selection_rejects_missing_profiles_and_nonfinite_inputs(self) -> None:
        scores = {"portable": SequenceScore("portable", True, 7, "family", [(4, .999)])}
        for thresholds, loss in (([.999], math.nan), ([math.nan], .1), ([], .1), ([.999], .1)):
            with self.assertRaises(ValueError):
                assess_prefix_epoch(scores, thresholds=thresholds, loss=loss)

    def test_one_early_false_is_a_false_whole_sequence(self) -> None:
        scores = {profile + name: SequenceScore(profile, desired, 7, name, candidates)
                  for profile in ("portable", "reference_hunspell")
                  for name, desired, candidates in (("correct", False, [(4, .9995), (5, .9994)]),
                                                    ("wrong", True, [(4, .99999)]))}
        low = sequence_metrics(scores, .999)
        self.assertEqual(low["portable"]["false"], 1)
        self.assertFalse(calibration_passed(low, .70))
        self.assertEqual(calibrate(scores, [.999, .9999], .70), (.9999, True))

    def test_zero_false_with_no_early_recall_or_missing_profile_does_not_pass(self) -> None:
        scores = {profile: SequenceScore(profile, True, 4, "family", [(4, .99999)])
                  for profile in ("portable", "reference_hunspell")}
        metrics = sequence_metrics(scores, .999)
        self.assertEqual(metrics["portable"]["converted"], 1)
        self.assertEqual(metrics["portable"]["converted_before_end"], 0)
        self.assertFalse(calibration_passed(metrics, .70))
        self.assertEqual(calibrate(scores, [.999], .70), (1.0, False))
        del metrics["reference_hunspell"]
        self.assertFalse(calibration_passed(metrics, .70))

    def test_recipe_cannot_add_test_or_disable_the_recall_requirement(self) -> None:
        original = recipe()
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "recipe.json"
            for change in ({"maximum_families": {"train": 1, "development": 1, "calibration": 1, "test": 1}},
                           {"minimum_early_recall": 0}, {"minimum_early_recall": .69},
                           {"thresholds": [1.0, .999]}, {"epochs": True}):
                path.write_text(json.dumps({**original, **change}))
                with self.assertRaises(ValueError):
                    recipe(path)

    def test_fit_never_overwrites_existing_output(self) -> None:
        with tempfile.TemporaryDirectory() as temporary, patch("train_prefix_v2_model.source_provenance") as provenance:
            with self.assertRaisesRegex(ValueError, "overwrite"):
                fit(Path(temporary))
        provenance.assert_not_called()

    def test_source_archive_preserves_exact_bytes_and_rejects_changed_input(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "fixture.py"
            raw = b"value = 1\n"
            source.write_bytes(raw)
            pins = {"fixture.py": hashlib.sha256(raw).hexdigest()}
            with patch("train_prefix_v2_model.ROOT", root):
                archive_sources(root / "snapshot", pins)
                self.assertEqual((root / "snapshot/source-snapshot/fixture.py").read_bytes(), raw)
                source.write_bytes(b"value = 2\n")
                with self.assertRaisesRegex(ValueError, "changed before archive"):
                    archive_sources(root / "second", pins)


if __name__ == "__main__":
    unittest.main()
