"""Right-field text and an unexecuted next-word plan are different evidence."""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import replace
from typing import cast
import unittest
from unittest.mock import patch

from keyswitch.context_action_features import extract_action_features
from keyswitch.constants.models import (
    ALONE_COUNT_RATIO_BOUND,
    ALONE_FEATURE_PREFIX,
    CAPITALS_FEATURE_PREFIX,
    CONTEXT_ACTION_FEATURE_VERSION,
    KEPT_CONTEXT_WORD_MAX_CHARACTERS,
    KEPT_FEATURE_PREFIX,
    PLANNED_CONTEXT_AFTER_MAX_CHARACTERS,
)
from keyswitch.context_model import ACTIONS, AfterOrigin, ContextAction, ContextEvidence, ContextModel, ContextPrediction, extract_context_features
from keyswitch.context_policy import ContextPolicy, evidence_for_decision
from keyswitch.detector import LanguageDetector
from keyswitch.input_context import FieldContext
from keyswitch.language_model import LanguageModel
import test_input_sequence_matrix as sequences
from fixture_values.counts import TRANSITION_FIXTURE_PENDING_TOKEN_MAX_CHARACTERS
from fixture_values.models import NON_STRING_FIELD_VALUE
from fixture_values.scores import (
    AFTER_ORIGIN_CONVERT_BIAS_WEIGHT,
    AFTER_ORIGIN_DECISIVE_BIAS_WEIGHT,
    AFTER_ORIGIN_KEEP_BIAS_WEIGHT,
    AFTER_ORIGIN_OVERRIDE_WEIGHT,
)


class TransitionModel(ContextModel):
    """Drive a waiting transaction; record evidence before any injection."""

    def __init__(self, observe: Callable[[], tuple[str, int]], *, planned_action: ContextAction = "convert") -> None:
        super().__init__({}, "context-v3-transition", feature_version=CONTEXT_ACTION_FEATURE_VERSION)
        self.observe = observe
        self.planned_action = planned_action
        self.items: list[tuple[ContextEvidence, tuple[str, int]]] = []

    def predict(self, item: ContextEvidence) -> ContextPrediction:
        self.items.append((item, self.observe()))
        action: ContextAction = (
            self.planned_action if item.field.after
            else "wait" if len(item.original) <= TRANSITION_FIXTURE_PENDING_TOKEN_MAX_CHARACTERS else "convert"
        )
        scores = tuple(float(name == action) for name in ACTIONS)
        return ContextPrediction(action, 1.0, scores, self.version, True)


class ContextAfterOriginTests(unittest.TestCase):
    def setUp(self) -> None:
        self.item = ContextEvidence("r", "к", 0, FieldContext("Editor", "1", "", "нас"), boundary_text=" ")

    def weights(self) -> dict[str, tuple[float, ...]]:
        return {name: (0.0,) * len(ACTIONS) for name in extract_action_features(self.item)
                if name.startswith(("source:char:", "target:char:"))}

    def test_default_origin_follows_literal_field_after_and_dataclass_replacement(self) -> None:
        self.assertEqual(self.item.after_origin, "field")
        empty = replace(self.item, field=replace(self.item.field, after=""))
        self.assertEqual(empty.after_origin, "none")
        restored = replace(empty, field=replace(empty.field, after="нас"))
        self.assertEqual(restored.after_origin, "field")
        planned = replace(restored, after_origin="planned_next_conversion")
        self.assertEqual(planned.after_origin, "planned_next_conversion")

    def test_origin_is_a_learned_signal_and_never_a_conversion_licence(self) -> None:
        planned = replace(self.item, after_origin="planned_next_conversion")
        field_features, planned_features = extract_action_features(self.item), extract_action_features(planned)
        self.assertIn("after_origin:field", field_features)
        self.assertIn("after_origin:planned_next_conversion:direction:0:length:1", planned_features)
        self.assertIn("after_origin:planned_next_conversion:script:ru:direction:0:length:1", planned_features)
        self.assertNotEqual(field_features, planned_features)
        weights = self.weights()
        weights.update({"bias": (AFTER_ORIGIN_KEEP_BIAS_WEIGHT, 0.0, 0.0, 0.0), "after_origin:planned_next_conversion": (0.0, AFTER_ORIGIN_OVERRIDE_WEIGHT, 0.0, 0.0)})
        learned = ContextModel(weights, "context-v3-origin", feature_version=CONTEXT_ACTION_FEATURE_VERSION)
        self.assertEqual(learned.predict(self.item).action, "keep")
        self.assertEqual(learned.predict(planned).action, "convert")
        weights["after_origin:planned_next_conversion"] = (AFTER_ORIGIN_OVERRIDE_WEIGHT, 0.0, 0.0, 0.0)
        self.assertEqual(ContextModel(weights, "context-v3-opposite", feature_version=CONTEXT_ACTION_FEATURE_VERSION).predict(planned).action, "keep")

    def test_impossible_planned_metadata_and_unknown_origins_fail_closed_in_v3(self) -> None:
        model = ContextModel({**self.weights(), "bias": (0.0, AFTER_ORIGIN_DECISIVE_BIAS_WEIGHT, 0.0, 0.0)}, "context-v3-origin", feature_version=CONTEXT_ACTION_FEATURE_VERSION)
        planned = replace(self.item, after_origin="planned_next_conversion")
        invalid_origins: tuple[object, ...] = ("verified", "", None, NON_STRING_FIELD_VALUE, [])
        invalid = [replace(self.item, after_origin=cast(AfterOrigin, value)) for value in invalid_origins]
        invalid += [replace(planned, field=replace(planned.field, after=after)) for after in ("", "нас завтра", "нас\u00a0", "я" * (PLANNED_CONTEXT_AFTER_MAX_CHARACTERS + 1))]
        invalid += [replace(planned, trigger="enter"), replace(planned, boundary_text="\t"),
                    replace(planned, original="long"), replace(planned, original="")]
        for item in invalid:
            with self.subTest(item=item):
                result = model.predict(item)
                self.assertEqual((result.action, result.supported), ("suggest", False))

    def test_a_kept_neighbour_question_has_weights_of_its_own(self) -> None:
        # Asked beside a kept next word, every feature carries KEPT_FEATURE_PREFIX: the weights of the
        # question asked at the word's own boundary never answer it, and the reverse.
        kept = replace(self.item, after_origin="kept_next_word")
        features = extract_action_features(kept)
        self.assertTrue(features and all(name.startswith(KEPT_FEATURE_PREFIX) for name in features))
        self.assertEqual({name.removeprefix(KEPT_FEATURE_PREFIX) for name in features} - {
            "after_origin:kept_next_word", "after_origin:kept_next_word:direction:0:length:1",
            "after_origin:kept_next_word:script:ru:direction:0:length:1"},
            set(extract_action_features(self.item)) - {"after_origin:field", "after_origin:field:direction:0:length:1",
                                                         "after_origin:field:script:ru:direction:0:length:1"})
        plain = ContextModel({**self.weights(), "bias": (0.0, AFTER_ORIGIN_DECISIVE_BIAS_WEIGHT, 0.0, 0.0)}, "context-v3-plain",
                             feature_version=CONTEXT_ACTION_FEATURE_VERSION)
        self.assertEqual(plain.predict(self.item).action, "convert")
        self.assertEqual((plain.predict(kept).action, plain.predict(kept).supported), ("suggest", False))
        own = ContextModel({**{KEPT_FEATURE_PREFIX + name: weights for name, weights in self.weights().items()},
                            KEPT_FEATURE_PREFIX + "bias": (0.0, AFTER_ORIGIN_DECISIVE_BIAS_WEIGHT, 0.0, 0.0)},
                           "context-v3-kept", feature_version=CONTEXT_ACTION_FEATURE_VERSION)
        self.assertEqual(own.predict(kept).action, "convert")
        self.assertEqual(own.predict(self.item).supported, False)
        unreachable = [replace(kept, field=replace(kept.field, after=after)) for after in ("", "нас завтра", "я" * (PLANNED_CONTEXT_AFTER_MAX_CHARACTERS + 1))]
        unreachable += [replace(kept, trigger="enter"), replace(kept, boundary_text="\t"), replace(kept, original=""),
                        replace(kept, original="r" * (KEPT_CONTEXT_WORD_MAX_CHARACTERS + 1))]
        for item in unreachable:
            with self.subTest(item=item):
                self.assertEqual((own.predict(item).action, own.predict(item).supported), ("suggest", False))

    def test_a_token_in_capitals_has_its_features_once_more_for_the_capitals_head(self) -> None:
        capitals = ContextEvidence("BP", "ИЗ", 0, FieldContext("Editor", "1", "affected by the ", ""), boundary_text=" ")
        features = extract_action_features(capitals)
        shared = {name: value for name, value in features.items() if not name.startswith(CAPITALS_FEATURE_PREFIX)}
        self.assertEqual({name.removeprefix(CAPITALS_FEATURE_PREFIX): value for name, value in features.items()
                          if name.startswith(CAPITALS_FEATURE_PREFIX)}, shared)
        # After Russian prose or nothing (`я YT` is `я НЕ` typed with Caps Lock), one letter, a word longer
        # than an abbreviation, a word not all in capitals, a token with a sign and the kept-neighbour
        # question: no head.
        for item in (replace(capitals, field=replace(capitals.field, before="я ")), replace(capitals, field=replace(capitals.field, before="")),
                     replace(capitals, original="B", alternative="И"), replace(capitals, original="PFGHJC", alternative="ЗАПРОС"),
                     replace(capitals, original="Bp", alternative="Из"), replace(capitals, original="BP2", alternative="ИЗ2"),
                     replace(capitals, field=replace(capitals.field, after="нас"), after_origin="kept_next_word")):
            with self.subTest(item=item):
                self.assertFalse(any(name.startswith(CAPITALS_FEATURE_PREFIX) for name in extract_action_features(item)))
        # Without head weights the shared ones decide; a head weight answers the class alone.
        support = {name: (0.0,) * len(ACTIONS) for item in (self.item, capitals) for name in extract_action_features(item)
                   if name.startswith(("source:char:", "target:char:"))}
        plain = {**support, "bias": (0.0, AFTER_ORIGIN_DECISIVE_BIAS_WEIGHT, 0.0, 0.0)}
        head = {**plain, CAPITALS_FEATURE_PREFIX + "bias": (AFTER_ORIGIN_DECISIVE_BIAS_WEIGHT * len(ACTIONS), 0.0, 0.0, 0.0)}
        for weights, expected in ((plain, "convert"), (head, "keep")):
            model = ContextModel(weights, "context-v3-capitals", feature_version=CONTEXT_ACTION_FEATURE_VERSION)
            self.assertEqual(model.predict(capitals).action, expected)
            self.assertEqual(model.predict(self.item).action, "convert")

    def test_a_lone_two_letter_token_has_its_features_once_more_for_the_lone_word_head(self) -> None:
        lone = ContextEvidence("гш", "ui", 1, FieldContext("Telegram", "1", "", ""), trigger="enter", boundary_text="\n")
        features = extract_action_features(lone)
        shared = {name: value for name, value in features.items() if not name.startswith(ALONE_FEATURE_PREFIX)}
        own = {name.removeprefix(ALONE_FEATURE_PREFIX): value for name, value in features.items() if name.startswith(ALONE_FEATURE_PREFIX)}
        # The shared features once more, and the orders of magnitude of the two counts: `ui` is counted 4752
        # times (13 binary digits), `гш` never.
        self.assertEqual({name: value for name, value in own.items() if not name.startswith("count:")}, shared)
        self.assertEqual(sorted(name for name in own if name.startswith("count:")),
                         ["count:13:0:direction:1", "count:cyrillic:0:direction:1", "count:latin:13:direction:1",
                          f"count:ratio:{ALONE_COUNT_RATIO_BOUND}:direction:1"])
        # Signs and digits around it leave it alone; a capitalised word too.
        for item in (replace(lone, field=replace(lone.field, before="1. ", after=" :)")), replace(lone, original="Гш", alternative="Ui")):
            with self.subTest(item=item):
                self.assertTrue(any(name.startswith(ALONE_FEATURE_PREFIX) for name in extract_action_features(item)))
        # A word before or after it, a space after it (the next word may come), one letter, three letters, a
        # reading with a sign (`хз` is `[p`) and the kept-neighbour question: no head.
        for item in (replace(lone, field=replace(lone.field, before="есть ")), replace(lone, field=replace(lone.field, after="кнопка")),
                     replace(lone, trigger="space", boundary_text=" "),
                     replace(lone, original="г", alternative="u"), replace(lone, original="гшы", alternative="uis"),
                     replace(lone, original="хз", alternative="[p"),
                     replace(lone, field=replace(lone.field, after="нас"), trigger="space", boundary_text=" ",
                             after_origin="kept_next_word")):
            with self.subTest(item=item):
                self.assertFalse(any(name.startswith(ALONE_FEATURE_PREFIX) for name in extract_action_features(item)))
        # Without head weights the shared ones decide; a head weight answers the class alone.
        support = {name: (0.0,) * len(ACTIONS) for item in (self.item, lone) for name in extract_action_features(item)
                   if name.startswith(("source:char:", "target:char:"))}
        plain = {**support, "bias": (AFTER_ORIGIN_DECISIVE_BIAS_WEIGHT, 0.0, 0.0, 0.0)}
        head = {**plain, ALONE_FEATURE_PREFIX + "bias": (0.0, AFTER_ORIGIN_DECISIVE_BIAS_WEIGHT * len(ACTIONS), 0.0, 0.0)}
        for weights, expected in ((plain, "keep"), (head, "convert")):
            model = ContextModel(weights, "context-v3-alone", feature_version=CONTEXT_ACTION_FEATURE_VERSION)
            self.assertEqual(model.predict(lone).action, expected)
            self.assertEqual(model.predict(self.item).action, "keep")

    def test_v2_features_and_predictions_ignore_origin_entirely(self) -> None:
        model = ContextModel({"bias": (0.0, AFTER_ORIGIN_CONVERT_BIAS_WEIGHT, 0.0, 0.0), "app:editor": (0.0,) * len(ACTIONS)}, "context-v1-origin")
        for origin in ("none", "field", "planned_next_conversion", "invalid"):
            with self.subTest(origin=origin):
                changed = replace(self.item, after_origin=cast(AfterOrigin, origin))
                self.assertEqual(extract_context_features(changed), extract_context_features(self.item))
                self.assertEqual(model.predict(changed), model.predict(self.item))

    def test_builder_and_policy_preserve_the_declared_origin_without_overriding_keep(self) -> None:
        detector = LanguageDetector({group: LanguageModel(locale, {}, "fixture", enable_spellcheck=False)
                                     for group, locale in enumerate(("en_US", "ru_RU"))})
        baseline = detector.decide("r", {1: "к"}, 0)
        direct = evidence_for_decision(baseline, "к", 1, detector, self.item.field, "space", boundary_text=" ")
        self.assertEqual(direct.after_origin, "field")
        model = ContextModel({**self.weights(), "bias": (AFTER_ORIGIN_DECISIVE_BIAS_WEIGHT, 0.0, 0.0, 0.0)}, "context-v3-keep", feature_version=CONTEXT_ACTION_FEATURE_VERSION)
        policy = ContextPolicy()
        policy.model = model
        with patch.object(model, "predict", wraps=model.predict) as prediction:
            result = policy.decide(baseline, "к", 1, detector, "space", "assist",
                                   field_override=self.item.field, after="нас", boundary_text=" ",
                                   after_origin="planned_next_conversion")
        self.assertEqual(prediction.call_args.args[0].after_origin, "planned_next_conversion")
        self.assertFalse(result.decision.should_convert)

    def test_a_token_the_orthotactic_model_cannot_read_carries_no_such_evidence(self) -> None:
        """The character model scores keys; with no keys there is nothing to score."""

        detector = LanguageDetector({group: LanguageModel(locale, {}, "fixture", enable_spellcheck=False)
                                     for group, locale in enumerate(("en_US", "ru_RU"))})
        ortho = ContextPolicy().ortho
        assert ortho is not None
        empty = evidence_for_decision(detector.decide("", {1: ""}, 0), "", 1, detector,
                                      self.item.field, "space", ortho=ortho)
        self.assertEqual((empty.ortho_score, empty.ortho_threshold), (None, None))
        scored = evidence_for_decision(detector.decide("ghbdtn", {1: "привет"}, 0), "привет", 1,
                                       detector, self.item.field, "space", ortho=ortho)
        self.assertIsNotNone(scored.ortho_score)

    def test_native_field_after_is_not_a_planned_conversion(self) -> None:
        class Reader:
            def read(self, application: str, window: int) -> FieldContext:
                return FieldContext(application, str(window), "r ", "нас", source="native")

        with sequences.session() as current:
            model = TransitionModel(lambda: (current.backend.text, len(current.backend.injections)))
            current.engine.context_policy.model = model
            current.engine.context_policy.reader = Reader()
            current.settings.set("detection.context_read_field", True)
            current.physical("r ")
            self.assertEqual(model.items[0][0].after_origin, "field")
            self.assertEqual(model.items[0][0].field.after, "нас")

    def test_resolver_labels_the_pending_next_word_before_joint_injection(self) -> None:
        # Two letters wait as one does; a lone letter would follow only a curated rule, whatever
        # the model says (ContextPolicy.decide), and `р` is no word of that list.
        with sequences.session() as current:
            model = TransitionModel(lambda: (current.backend.text, len(current.backend.injections)))
            current.engine.context_policy.model = model
            current.physical("hs ")
            current.physical("yfc ")
            self.assertEqual([(item.original, item.after_origin, item.field.after) for item, _ in model.items],
                             [("hs", "none", ""), ("yfc", "none", ""), ("hs", "planned_next_conversion", "нас")])
            self.assertEqual(model.items[-1][1], ("hs yfc ", 0))
            self.assertEqual((current.backend.text, len(current.backend.injections)), ("ры нас ", 1))

    def test_planned_keep_preserves_first_word_and_only_executes_the_next_plan(self) -> None:
        with sequences.session() as current:
            model = TransitionModel(lambda: (current.backend.text, len(current.backend.injections)), planned_action="keep")
            current.engine.context_policy.model = model
            current.physical("h yfc ")
            self.assertEqual(model.items[-1][0].after_origin, "planned_next_conversion")
            self.assertEqual((current.backend.text, len(current.backend.injections)), ("h нас ", 1))

    def test_focus_timeout_and_suffix_invalidation_do_not_create_planned_evidence(self) -> None:
        for invalidation in ("focus", "timeout", "suffix"):
            with self.subTest(invalidation=invalidation), sequences.session() as current:
                model = TransitionModel(lambda: (current.backend.text, len(current.backend.injections)))
                current.engine.context_policy.model = model
                current.physical("h ")
                waiting = current.engine._context_waiting
                assert waiting is not None
                if invalidation == "focus":
                    current.backend.window += 1
                elif invalidation == "timeout":
                    current.engine._context_waiting = replace(waiting, deadline=0.0)
                else:
                    current.engine.context_policy.stream.text = "external "
                current.physical("yfc ")
                self.assertFalse(any(item.after_origin == "planned_next_conversion" for item, _ in model.items))
                self.assertTrue(current.backend.text.startswith("h "))
                self.assertEqual(current.backend.submissions, [])


if __name__ == "__main__":
    unittest.main()
