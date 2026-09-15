"""Planned context preserves existing labels, family provenance and mass."""

from __future__ import annotations

from dataclasses import replace
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

from context_lookahead_curriculum import LookaheadAnchor, LookaheadSeed, build_lookahead_curriculum, provenance
from keyswitch.context_model import ContextEvidence
from keyswitch.detector import LanguageDetector
from keyswitch.input_context import FieldContext
from keyswitch.language_model import LanguageModel


class LookaheadCurriculumTests(unittest.TestCase):
    def setUp(self) -> None:
        self.detector = LanguageDetector({
            0: LanguageModel("en_US", {"hello": 5000, "world": 5000}, "fixture", enable_spellcheck=False),
            1: LanguageModel("ru_RU", {"привет": 5000, "работа": 5000}, "fixture", enable_spellcheck=False),
        })
        self.positive = LookaheadSeed("positive", "short-physical-family", ContextEvidence(
            "r", "к", 0, FieldContext("Editor", "stream", "", "привет дальше"), boundary_text=" "),
            "convert", "legacy_short_lookahead", 0.25)
        self.negative = LookaheadSeed("negative", "short-physical-family", ContextEvidence(
            "r", "к", 0, FieldContext("Editor", "stream", "const value = ", "", "code"), boundary_text=" "),
            "keep", "legacy_technical", 0.75)
        self.anchor = LookaheadAnchor("anchor", "russian-anchor-family", "привет", 1)

    def test_planned_positive_and_negative_keep_their_labels_before_and_parent_mass(self) -> None:
        result = build_lookahead_curriculum([self.positive, self.negative], [self.anchor], self.detector, profile="portable")
        planned = [frame for frame in result.frames if frame.kind == "planned"]
        self.assertEqual({frame.action for frame in planned}, {"keep", "convert"})
        self.assertEqual({frame.next_original for frame in planned}, {"ghbdtn"})
        self.assertEqual({frame.evidence.field.after for frame in planned}, {"привет"})
        self.assertEqual({frame.evidence.after_origin for frame in planned}, {"planned_next_conversion"})
        self.assertEqual({frame.anchor_family for frame in planned}, {self.anchor.parent_family})
        for seed in (self.positive, self.negative):
            variants = [frame for frame in result.frames if frame.source_identifier == seed.identifier]
            self.assertEqual(len(variants), 2)
            self.assertAlmostEqual(sum(frame.sample_weight for frame in variants), seed.sample_weight)
            for frame in variants:
                self.assertEqual((frame.action, frame.parent_family, frame.evidence.original,
                                  frame.evidence.alternative, frame.evidence.field.before),
                                 (seed.action, seed.parent_family, seed.evidence.original,
                                  seed.evidence.alternative, seed.evidence.field.before))
        self.assertEqual(result.mass_by_origin_action, {"field:convert": .125, "planned_next_conversion:convert": .125,
                                                      "none:keep": .375, "planned_next_conversion:keep": .375})

    def test_literal_right_context_does_not_become_a_negative_planned_label(self) -> None:
        negative = replace(self.negative, evidence=replace(self.negative.evidence,
                           field=replace(self.negative.evidence.field, after="привет")))
        empty = replace(self.negative, identifier="empty", evidence=replace(self.negative.evidence,
                        field=replace(self.negative.evidence.field, before="")))
        result = build_lookahead_curriculum([negative, empty], [self.anchor], self.detector, profile="portable")
        self.assertEqual([frame.kind for frame in result.frames], ["original", "original"])
        self.assertEqual(sum(frame.sample_weight for frame in result.frames), 1.5)

    def test_missing_train_anchor_and_nonproposed_next_word_do_not_invent_labels(self) -> None:
        for anchors in ([], [replace(self.anchor, text="неизвестно")]):
            result = build_lookahead_curriculum([self.positive], anchors, self.detector, profile="portable")
            self.assertEqual(result.counts["skipped_missing_anchor"], 1)
            self.assertEqual(len(result.frames), 1)
        detector = LanguageDetector({group: LanguageModel(locale, {}, "empty", enable_spellcheck=False)
                                     for group, locale in enumerate(("en_US", "ru_RU"))})
        result = build_lookahead_curriculum([self.negative], [self.anchor], detector, profile="portable")
        self.assertEqual(result.counts["skipped_next_not_proposed"], 1)
        self.assertEqual(result.frames[0].sample_weight, self.negative.sample_weight)

    def test_same_observable_evidence_never_acquires_opposite_generated_labels(self) -> None:
        # A pre-existing planned frame with the same observable evidence as the generated
        # one, including the baseline verdict the planned next word gives the short word.
        conflicting = replace(self.positive, identifier="conflict", action="keep", category="legacy_technical",
                              evidence=replace(self.positive.evidence, after_origin="planned_next_conversion",
                                               field=replace(self.positive.evidence.field, after="привет"),
                                               baseline_convert=True))
        result = build_lookahead_curriculum([self.positive, conflicting], [self.anchor], self.detector, profile="portable")
        self.assertEqual(result.counts["skipped_label_conflict"], 1)
        self.assertEqual(len(result.frames), 2)

    def test_sampling_is_order_independent_and_bounded_per_family(self) -> None:
        seeds = [replace(self.negative, identifier=f"seed-{index}", parent_family=f"family-{index // 4}")
                 for index in range(12)]
        anchors = [self.anchor, replace(self.anchor, identifier="second", text="работа")]
        first = build_lookahead_curriculum(seeds, anchors, self.detector, profile="portable", maximum_families=2, seeds_per_family=2)
        second = build_lookahead_curriculum(list(reversed(seeds)), list(reversed(anchors)), self.detector,
                                            profile="portable", maximum_families=2, seeds_per_family=2)
        self.assertEqual(first, second)
        self.assertEqual(first.counts["selected_families"], 2)
        self.assertEqual(first.counts["selected_seeds"], 4)
        self.assertEqual(first.counts["planned_keep"], 4)
        self.assertEqual(sum(frame.sample_weight for frame in first.frames), sum(seed.sample_weight for seed in seeds))

    def test_nontrain_and_unsafe_inputs_are_rejected_before_generation(self) -> None:
        for split in ("test", "development", "calibration", ""):
            with self.subTest(split=split), self.assertRaisesRegex(ValueError, "declared split"):
                build_lookahead_curriculum([replace(self.negative, split=split)], [self.anchor], self.detector, profile="portable")
            with self.assertRaisesRegex(ValueError, "declared split"):
                build_lookahead_curriculum([self.negative], [replace(self.anchor, split=split)], self.detector, profile="portable")
        for field in (replace(self.negative.evidence.field, sensitive=True),
                      replace(self.negative.evidence.field, selection=True),
                      replace(self.negative.evidence.field, role="password")):
            with self.assertRaisesRegex(ValueError, "unsafe"):
                build_lookahead_curriculum([replace(self.negative, evidence=replace(self.negative.evidence, field=field))],
                                           [self.anchor], self.detector, profile="portable")

    def test_unreachable_focus_and_unsupported_anchors_are_not_supervised(self) -> None:
        variants = [replace(self.negative, identifier=str(index), evidence=item) for index, item in enumerate((
            replace(self.negative.evidence, original="long"), replace(self.negative.evidence, trigger="enter"),
            replace(self.negative.evidence, boundary_text="\t"), replace(self.negative.evidence, literal_tail="!"),
            replace(self.negative.evidence, original="é"), replace(self.negative.evidence, alternative="не тот"),
            replace(self.negative.evidence, alternative="кк"),
            replace(self.negative.evidence, original=";", alternative="ж"),
            replace(self.negative.evidence, original="ж", alternative=";", source_group=1),
        ))]
        result = build_lookahead_curriculum(variants, [self.anchor], self.detector, profile="portable")
        self.assertEqual(result.counts["selected_seeds"], 0)
        anchors = [replace(self.anchor, identifier=str(index), text=text) for index, text in enumerate(("", "x", "café", "a b", "x" * 65))]
        result = build_lookahead_curriculum([self.negative], anchors, self.detector, profile="portable")
        self.assertEqual(result.counts["skipped_anchor"], 5)
        self.assertEqual(len(result.frames), 1)

    def test_configuration_weights_identity_and_provenance_are_explicit(self) -> None:
        for maximum in (0, 4097, True):
            with self.assertRaises(ValueError):
                build_lookahead_curriculum([], [], self.detector, profile="portable", maximum_families=maximum)
        for maximum in (0, 129, True):
            with self.assertRaises(ValueError):
                build_lookahead_curriculum([], [], self.detector, profile="portable", seeds_per_family=maximum)
        with self.assertRaises(ValueError):
            build_lookahead_curriculum([], [], self.detector, profile="implicit")
        for weight in (0.0, -1.0, float("nan"), float("inf"), True):
            with self.assertRaises(ValueError):
                build_lookahead_curriculum([replace(self.negative, sample_weight=weight)], [], self.detector, profile="portable")
        for seed in (replace(self.negative, parent_family=""), replace(self.negative, identifier="")):
            with self.assertRaises(ValueError):
                build_lookahead_curriculum([seed], [], self.detector, profile="portable")
        with self.assertRaises(ValueError):
            build_lookahead_curriculum([self.negative, self.negative], [], self.detector, profile="portable")
        self.assertIn("src/keyswitch/engine.py", provenance())
        self.assertTrue(all(len(value) == 64 for value in provenance().values()))


if __name__ == "__main__":
    unittest.main()


class NaturalLookaheadSeedTests(unittest.TestCase):
    def setUp(self) -> None:
        self.detector = LanguageDetector({
            0: LanguageModel("en_US", {"hello": 5000, "world": 5000}, "fixture", enable_spellcheck=False),
            1: LanguageModel("ru_RU", {"привет": 5000, "этого": 5000, "работа": 5000}, "fixture", enable_spellcheck=False),
        })
        # A field-start frame: `у` typed as `e`, the sentence continues with `этого`.
        self.start = LookaheadSeed("natural:1:empty:wrong", "natural:doc-1", ContextEvidence(
            "e", "у", 0, FieldContext("Telegram", "public-training", "", ""), boundary_text=" "),
            "convert", "natural_short_lookahead", 1.0, next_words="этого не касается")
        self.anchor = LookaheadAnchor("natural:1:first-after", "natural-after:x", "этого", 1)

    def test_natural_seed_with_empty_left_context_gains_a_planned_variant_and_keeps_its_original(self) -> None:
        result = build_lookahead_curriculum([self.start], [self.anchor], self.detector, profile="portable")
        kinds = {frame.kind: frame for frame in result.frames}
        self.assertEqual(set(kinds), {"original", "planned"})
        self.assertEqual(kinds["original"].evidence.field.after, "")
        self.assertEqual(kinds["original"].evidence.after_origin, "none")
        self.assertEqual(kinds["planned"].evidence.field.after, "этого")
        self.assertEqual(kinds["planned"].evidence.after_origin, "planned_next_conversion")
        self.assertEqual(kinds["planned"].evidence.field.before, "")
        self.assertEqual({frame.action for frame in result.frames}, {"convert"})
        self.assertAlmostEqual(sum(frame.sample_weight for frame in result.frames), 1.0)
        self.assertEqual(result.mass_by_origin_action, {"none:convert": 0.5, "planned_next_conversion:convert": 0.5})

    def test_natural_seed_needs_its_own_continuation_and_the_matching_anchor(self) -> None:
        silent = replace(self.start, identifier="silent", next_words="")
        other = replace(self.start, identifier="other", next_words="работа")
        result = build_lookahead_curriculum([silent, other], [self.anchor], self.detector, profile="portable")
        self.assertEqual([frame.kind for frame in result.frames], ["original", "original"])
        self.assertEqual(result.counts.get("skipped_missing_anchor", 0), 1)

    def test_development_curriculum_uses_only_development_members(self) -> None:
        seed = replace(self.start, split="development")
        anchor = replace(self.anchor, split="development")
        result = build_lookahead_curriculum([seed], [anchor], self.detector, profile="portable", split="development")
        self.assertEqual({frame.kind for frame in result.frames}, {"original", "planned"})
        with self.assertRaisesRegex(ValueError, "declared split"):
            build_lookahead_curriculum([seed], [self.anchor], self.detector, profile="portable", split="development")
        with self.assertRaisesRegex(ValueError, "declared split"):
            build_lookahead_curriculum([self.start], [self.anchor], self.detector, profile="portable", split="development")
        with self.assertRaisesRegex(ValueError, "train, development or calibration"):
            build_lookahead_curriculum([replace(seed, split="test")], [replace(anchor, split="test")], self.detector, profile="portable", split="test")

    def test_deferred_seed_keeps_its_own_label_and_its_planned_variant_takes_the_declared_one(self) -> None:
        deferred = replace(self.start, action="wait", planned_action="convert")
        result = build_lookahead_curriculum([deferred], [self.anchor], self.detector, profile="portable")
        kinds = {frame.kind: frame for frame in result.frames}
        self.assertEqual((kinds["original"].action, kinds["planned"].action), ("wait", "convert"))
        self.assertEqual(result.mass_by_origin_action, {"none:wait": 0.5, "planned_next_conversion:convert": 0.5})
        self.assertEqual(result.counts["planned_convert"], 1)

    def test_planned_frame_carries_the_baseline_verdict_given_its_next_word(self) -> None:
        # Alone, `r` is a short word the detector declines; after a target-language
        # neighbour the reviewed safe-short-word rule converts it. The engine re-decides
        # the waiting word with its planned next word, so the planned frame says so too.
        seed = replace(self.start, evidence=ContextEvidence("r", "к", 0, FieldContext("Telegram", "public-training", "", ""), boundary_text=" ", baseline_convert=False))
        anchor = LookaheadAnchor("natural:2:first-after", "natural-after:y", "привет", 1)
        seed = replace(seed, next_words="привет дальше")
        result = build_lookahead_curriculum([seed], [anchor], self.detector, profile="portable")
        kinds = {frame.kind: frame for frame in result.frames}
        self.assertFalse(kinds["original"].evidence.baseline_convert)
        self.assertTrue(kinds["planned"].evidence.baseline_convert)
        self.assertEqual(kinds["planned"].evidence.field.after, "привет")
