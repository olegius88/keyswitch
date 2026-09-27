"""Schema-2 orthotactic artifacts: token features computed by the runtime from the keys alone."""
from __future__ import annotations

import json
import tempfile
import unittest
from copy import deepcopy
from pathlib import Path
from typing import cast
from unittest.mock import patch

from keyswitch.constants.models import CONTEXT_TYPO_MIN_CHARACTERS
from keyswitch.constants.ortho import (
    ORTHO_EXTRA_KEY_ARTIFACT_SCHEMA_VERSION,
    ORTHO_FEATURE_ARTIFACT_SCHEMA_VERSION,
    ORTHO_MIN_STRETCH_RUN,
    ORTHO_PLAUSIBILITY_ARTIFACT_SCHEMA_VERSION,
    ORTHO_REPLACEMENT_ARTIFACT_SCHEMA_VERSION,
    ORTHO_STRETCH_ARTIFACT_SCHEMA_VERSION,
)
from keyswitch.context_policy import ContextPolicy
from keyswitch.detector import DetectionDecision, LanguageDetector
from keyswitch.input_context import FieldContext
from keyswitch.identifier_lexicon import IdentifierLexicon
from keyswitch.language_model import LanguageModel, WordScore
from keyswitch.ortho_model import (
    FEATURES, FEATURES_SCHEMA_2, FEATURES_SCHEMA_3, FEATURES_SCHEMA_4, FEATURES_SCHEMA_5, LOCALES, OrthoEvidence, OrthoModel, collapse_runs,
    drop_hyphen_stretches, one_key_extra, read_once, readings, reduplicated, stretch_readings, token_features,
)
from fixture_values.models import ORTHO_FIXTURE_LONG_GRAM_LOGPROB, ORTHO_FIXTURE_SCALE
from fixture_values.ortho import (
    FEATURE_FIXTURE_COLLAPSE,
    FEATURE_FIXTURE_COMPOUND_MIN_LETTERS,
    FEATURE_FIXTURE_DOT_WEIGHT,
    FEATURE_FIXTURE_IDENT_WEIGHT,
    FEATURE_FIXTURE_NON_INTEGER_WEIGHT,
    FEATURE_FIXTURE_PARTS_WEIGHT,
    FEATURE_FIXTURE_REPEAT_WEIGHT,
    FEATURE_FIXTURE_STRETCH,
    FEATURE_FIXTURE_UNSUPPORTED_SCHEMA,
)
from test_ortho_model import channel, minimal

KNOWN = {"en": {"dist", "e", "mail"}, "ru": {"я", "то", "у", "привет"}}


def known(script: str, word: str) -> bool:
    return word in KNOWN[script]


def identifier(token: str) -> bool:
    return token in {"htop", "nginx"}


def weights() -> dict[str, dict[str, int]]:
    table = {"ident": FEATURE_FIXTURE_IDENT_WEIGHT, "repeat": FEATURE_FIXTURE_REPEAT_WEIGHT,
             "parts_source": -FEATURE_FIXTURE_PARTS_WEIGHT, "parts_target": FEATURE_FIXTURE_PARTS_WEIGHT,
             "dot_word": FEATURE_FIXTURE_DOT_WEIGHT}
    return {"en": dict(table), "ru": dict(table)}


def featured() -> dict[str, object]:
    return {**minimal(), "schema_version": ORTHO_FEATURE_ARTIFACT_SCHEMA_VERSION, "features": weights()}


def keyed() -> dict[str, object]:
    """Schema 3: the schema 2 weights and `extra_key`."""
    table = {script: {**row, "extra_key": FEATURE_FIXTURE_DOT_WEIGHT} for script, row in weights().items()}
    return {**minimal(), "schema_version": ORTHO_EXTRA_KEY_ARTIFACT_SCHEMA_VERSION, "features": table}


def stretching(stretch: int | None) -> dict[str, object]:
    """Schema 3 (or 4 with `stretch`) whose English channel likes a double `bb` and whose Russian one does not."""
    doubled = channel()
    doubled["grams"] = f"{doubled['grams']}\nbb"
    doubled["logprob"] = [*cast(list[int], doubled["logprob"]), ORTHO_FIXTURE_LONG_GRAM_LOGPROB]
    payload = {**keyed(), "models": {"en": doubled, "ru": channel()}, "collapse_runs": FEATURE_FIXTURE_COLLAPSE}
    if stretch is not None:
        worded = {script: {**row, "source_word": -FEATURE_FIXTURE_PARTS_WEIGHT}
                  for script, row in cast(dict[str, dict[str, int]], payload["features"]).items()}
        payload.update({"schema_version": ORTHO_STRETCH_ARTIFACT_SCHEMA_VERSION, "stretch_runs": stretch, "features": worded})
    return payload


def replacing() -> dict[str, object]:
    """Schema 5: schema 4 with the replacement features, source plausibility and its centres."""
    payload = stretching(FEATURE_FIXTURE_STRETCH)
    table = {script: {**row, "name_unknown": 0, "target_unknown": 0, "source_plausibility": 0}
             for script, row in cast(dict[str, dict[str, int]], payload["features"]).items()}
    payload.update({"schema_version": ORTHO_REPLACEMENT_ARTIFACT_SCHEMA_VERSION, "features": table,
                    "feature_centres": {"en": 0, "ru": 0}})
    return payload


def weighing() -> dict[str, object]:
    """Schema 6: schema 5 with the plausibility of the replacement and its centres."""
    payload = replacing()
    table = {script: {**row, "target_plausibility": 0} for script, row in cast(dict[str, dict[str, int]], payload["features"]).items()}
    payload.update({"schema_version": ORTHO_PLAUSIBILITY_ARTIFACT_SCHEMA_VERSION, "features": table,
                    "target_centres": {"en": 0, "ru": 0}})
    return payload


class TokenFeatureTests(unittest.TestCase):
    def test_readings_follow_the_layout_typed_in(self) -> None:
        self.assertEqual(readings("htop", "ru"), ("рещз", "htop"))
        self.assertEqual(readings("z-nj", "en"), ("z-nj", "я-то"))

    def test_a_stretched_letter_is_cut_to_the_run_the_artifact_allows(self) -> None:
        self.assertEqual(collapse_runs("rftaaa", FEATURE_FIXTURE_COLLAPSE), "rftaa")
        self.assertEqual(collapse_runs("нееееет", FEATURE_FIXTURE_COLLAPSE), "неет")
        self.assertEqual(collapse_runs("abab", FEATURE_FIXTURE_COLLAPSE), "abab")
        self.assertEqual(collapse_runs("", FEATURE_FIXTURE_COLLAPSE), "")

    def test_a_run_long_enough_to_be_a_stretch_is_read_once(self) -> None:
        self.assertEqual(read_once("муууу", FEATURE_FIXTURE_STRETCH), "му")
        self.assertEqual(read_once("каеффф", FEATURE_FIXTURE_STRETCH), "каеф")
        self.assertEqual(read_once("класс", FEATURE_FIXTURE_STRETCH), "класс")          # a double is spelling here
        self.assertEqual(read_once("шттук", ORTHO_MIN_STRETCH_RUN), "штук")             # and a stuck key here
        self.assertEqual(read_once("abab", ORTHO_MIN_STRETCH_RUN), "abab")
        self.assertEqual(read_once("", ORTHO_MIN_STRETCH_RUN), "")

    def test_compound_evidence_on_both_sides_cancels_in_schema_5(self) -> None:
        both = {"en": {"e", "mail"}, "ru": {"у", "ьфшд"}}
        lookup = lambda script, word: word in both[script]
        loose = token_features("e-mail", "en", identifier=identifier, known=lookup)
        self.assertEqual((loose["parts_source"], loose["parts_target"]), (1, 1))
        strict = token_features("e-mail", "en", identifier=identifier, known=lookup, symmetric=True)
        self.assertEqual((strict["parts_source"], strict["parts_target"]), (0, 0))
        self.assertEqual(token_features("z-nj", "en", identifier=identifier, known=known, symmetric=True)["parts_target"], 1)

    def test_a_compound_needs_a_half_as_long_as_its_script_requires(self) -> None:
        short = {"en": {"r", "ut", "e", "mail"}, "ru": {"я", "то"}}
        lookup = lambda script, word: word in short[script]
        minimum = {"en": FEATURE_FIXTURE_COMPOUND_MIN_LETTERS}

        def strict(keys: str, layout: str) -> dict[str, int]:
            return token_features(keys, layout, identifier=identifier, known=lookup, symmetric=True, compound_min_letters=minimum)

        # `к-ге` reads `r-ut`: two strings the English dictionary knows as a letter and an abbreviation.
        self.assertEqual(token_features("r-ut", "ru", identifier=identifier, known=lookup)["parts_target"], 1)
        self.assertEqual(strict("r-ut", "ru")["parts_target"], 0)
        self.assertEqual(strict("r-ut", "en")["parts_source"], 0)
        self.assertEqual(token_features("r-ut", "en", identifier=identifier, known=lookup)["parts_source"], 1)
        self.assertEqual(strict("e-mail", "ru")["parts_target"], 1)                     # a half of four letters
        self.assertEqual(strict("z-nj", "en")["parts_target"], 1)                       # Russian needs no length

    def test_a_hyphen_between_two_copies_of_a_key_is_a_drawn_out_sound(self) -> None:
        self.assertEqual(drop_hyphen_stretches("ти-ише"), "тише")
        self.assertEqual(drop_hyphen_stretches("ура-а-а"), "ура")
        for text in ("e-mail", "я-то", "-а", "а-", "а--а", ""):
            with self.subTest(text=text):
                self.assertEqual(drop_hyphen_stretches(text), text)
        self.assertEqual(stretch_readings("nb-bit", FEATURE_FIXTURE_COLLAPSE, FEATURE_FIXTURE_STRETCH, hyphens=True), ("nb-bit", "nbit"))
        self.assertEqual(stretch_readings("nb-bit", FEATURE_FIXTURE_COLLAPSE, FEATURE_FIXTURE_STRETCH), ("nb-bit",))

    def test_reduplication_around_one_hyphen(self) -> None:
        for text in ("у-у", "мда-а", "ну-ну", "ааа-ба"):
            with self.subTest(text=text):
                self.assertTrue(reduplicated(text))
        for text in ("я-то", "у", "-у", "у-", "а-б-в"):
            with self.subTest(text=text):
                self.assertFalse(reduplicated(text))

    def test_each_feature_reads_the_token(self) -> None:
        self.assertEqual(token_features("htop", "ru", identifier=identifier, known=known),
                         {"ident": 1, "repeat": 0, "parts_source": 0, "parts_target": 0, "dot_word": 0, "extra_key": 0,
                          "source_word": 0, "name_unknown": 0, "target_unknown": 1})
        # The replacement `mail` is a word; `Цфшд` read in English is `Wail`... not in this dictionary.
        self.assertEqual(token_features("mail", "ru", identifier=identifier, known=known)["target_unknown"], 0)
        self.assertEqual(token_features("htop", "ru", identifier=identifier, known=known, shape="inner")["name_unknown"], 1)
        self.assertEqual(token_features("htop", "ru", identifier=identifier, known=known, shape="upper")["name_unknown"], 0)
        self.assertEqual(token_features(".dist", "ru", identifier=identifier, known=known)["target_unknown"], 0)   # not a word shape
        self.assertEqual(token_features("ghbdtn", "ru", identifier=identifier, known=known)["source_word"], 1)
        self.assertEqual(token_features("aghbdtn", "en", identifier=identifier, known=known)["extra_key"], 1)
        self.assertEqual(token_features("z-nj", "en", identifier=identifier, known=known)["parts_target"], 1)
        self.assertEqual(token_features("e-mail", "en", identifier=identifier, known=known)["parts_source"], 1)
        self.assertEqual(token_features("e-e", "ru", identifier=identifier, known=known)["repeat"], 1)
        self.assertEqual(token_features(".dist", "ru", identifier=identifier, known=known)["dot_word"], 1)
        self.assertEqual(token_features(".dist", "en", identifier=identifier, known=known)["dot_word"], 0)
        self.assertEqual(token_features(".", "ru", identifier=identifier, known=known)["dot_word"], 0)
        self.assertEqual(token_features(".xyz", "ru", identifier=identifier, known=known)["dot_word"], 0)
        self.assertEqual(token_features("z-", "en", identifier=identifier, known=known)["parts_target"], 0)
        self.assertEqual(token_features("q-nj", "en", identifier=identifier, known=known)["parts_target"], 0)


class ExtraKeyTests(unittest.TestCase):
    def test_a_reading_one_stray_key_away_from_a_word(self) -> None:
        self.assertTrue(one_key_extra("фпривет", "ru", known))            # a stray key in front
        self.assertTrue(one_key_extra("приввет", "ru", known))            # or inside
        self.assertFalse(one_key_extra("привет", "ru", known))            # a word itself is not
        self.assertFalse(one_key_extra("фпревет", "ru", known))           # two keys away is not
        self.assertFalse(one_key_extra("ф-привет", "ru", known))          # letters only
        short = "привет"[:CONTEXT_TYPO_MIN_CHARACTERS - 1]
        self.assertFalse(one_key_extra("ф" + short[:-1], "ru", lambda _script, word: word == short[:-1]))


class FeatureArtifactTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.path = Path(self.temporary.name) / "ortho.json"

    def write(self, payload: object) -> Path:
        self.path.write_text(json.dumps(payload), encoding="utf-8")
        return self.path

    def test_a_featured_artifact_adds_its_weights_to_the_score(self) -> None:
        model = OrthoModel.load(self.write(featured()), identifier=identifier, known=known)
        self.assertEqual(model.features["ru"]["ident"], FEATURE_FIXTURE_IDENT_WEIGHT / ORTHO_FIXTURE_SCALE)
        plain = OrthoModel.load(self.write(minimal()))
        with_ident = model.score(OrthoEvidence("htop", "lower", "ru"))
        without = plain.score(OrthoEvidence("htop", "lower", "ru"))
        self.assertEqual(with_ident.features, FEATURE_FIXTURE_IDENT_WEIGHT / ORTHO_FIXTURE_SCALE)
        self.assertEqual(with_ident.total, without.total + with_ident.features)
        self.assertEqual(without.features, 0.0)

    def test_every_feature_guard_refuses_its_own_defect(self) -> None:
        broken_names = deepcopy(featured())
        broken_names["features"]["ru"] = {name: 0 for name in FEATURES_SCHEMA_2[:-1]}  # type: ignore[index]
        broken_weight = deepcopy(featured())
        broken_weight["features"]["en"]["ident"] = FEATURE_FIXTURE_NON_INTEGER_WEIGHT  # type: ignore[index]
        cases: list[tuple[str, object]] = [
            ("unsupported schema", {**featured(), "schema_version": FEATURE_FIXTURE_UNSUPPORTED_SCHEMA}),
            ("schema 2 without features", {**minimal(), "schema_version": ORTHO_FEATURE_ARTIFACT_SCHEMA_VERSION}),
            ("schema 1 with features", {**minimal(), "features": weights()}),
            ("features for one script", {**featured(), "features": {"en": weights()["en"]}}),
            ("features table not an object", {**featured(), "features": {"en": [], "ru": weights()["ru"]}}),
            ("a feature missing", broken_names),
            ("schema 2 weighing extra_key", {**featured(), "features": keyed()["features"]}),
            ("schema 3 without extra_key", {**keyed(), "features": weights()}),
            ("a weight not an integer", broken_weight),
            ("schema 1 with a scope", {**minimal(), "unscored_shapes": ["upper"]}),
            ("a collapse below two", {**featured(), "collapse_runs": FEATURE_FIXTURE_COLLAPSE - 1}),
            ("a collapse not an integer", {**featured(), "collapse_runs": True}),
            ("an unknown shape in scope", {**featured(), "unscored_shapes": ["sideways"]}),
            ("a scope not a list", {**featured(), "unscored_shapes": "upper"}),
            ("schema 1 with source channels", {**minimal(), "source_models": {"ru": channel()}}),
            ("source channels not an object", {**featured(), "source_models": []}),
            ("a source channel for an unknown script", {**featured(), "source_models": {"de": channel()}}),
            ("an invalid source channel", {**featured(), "source_models": {"ru": {"grams": True}}}),
            ("schema 4 without a stretch", {**keyed(), "schema_version": ORTHO_STRETCH_ARTIFACT_SCHEMA_VERSION}),
            ("schema 3 with a stretch", {**keyed(), "stretch_runs": FEATURE_FIXTURE_STRETCH}),
            ("schema 4 without source_word", {**keyed(), "schema_version": ORTHO_STRETCH_ARTIFACT_SCHEMA_VERSION,
                                              "stretch_runs": FEATURE_FIXTURE_STRETCH}),
            ("schema 4 with feature centres", {**stretching(FEATURE_FIXTURE_STRETCH), "feature_centres": {"en": 0, "ru": 0}}),
            ("schema 5 without feature centres", {**replacing(), "feature_centres": None}),
            ("feature centres for one script", {**replacing(), "feature_centres": {"en": 0}}),
            ("feature centres not integers", {**replacing(), "feature_centres": {"en": FEATURE_FIXTURE_NON_INTEGER_WEIGHT, "ru": 0}}),
            ("schema 6 without target centres", {**weighing(), "target_centres": None}),
            ("schema 5 with target centres", {**replacing(), "target_centres": {"en": 0, "ru": 0}}),
            ("target centres for one script", {**weighing(), "target_centres": {"en": 0}}),
            ("gated features not a list", {**weighing(), "gated_features": "extra_key"}),
            ("an unknown gated feature", {**weighing(), "gated_features": ["source_plausibility"]}),
            ("schema 5 with gated features", {**replacing(), "gated_features": ["extra_key"]}),
            ("a compound minimum not an object", {**replacing(), "compound_min_letters": FEATURE_FIXTURE_COMPOUND_MIN_LETTERS}),
            ("a compound minimum for an unknown script", {**replacing(), "compound_min_letters": {"de": FEATURE_FIXTURE_COMPOUND_MIN_LETTERS}}),
            ("a compound minimum below one letter", {**replacing(), "compound_min_letters": {"en": 0}}),
            ("a compound minimum not an integer", {**replacing(), "compound_min_letters": {"en": True}}),
            ("schema 4 with a compound minimum", {**stretching(FEATURE_FIXTURE_STRETCH),
                                                  "compound_min_letters": {"en": FEATURE_FIXTURE_COMPOUND_MIN_LETTERS}}),
            ("schema 1 with a stretch", {**minimal(), "stretch_runs": FEATURE_FIXTURE_STRETCH}),
            ("a stretch of one key", stretching(ORTHO_MIN_STRETCH_RUN - 1)),
            ("a stretch not an integer", {**stretching(FEATURE_FIXTURE_STRETCH), "stretch_runs": True}),
        ]
        for label, payload in cases:
            with self.subTest(label=label):
                with self.assertRaises(ValueError):
                    OrthoModel.load(self.write(payload))

    def test_without_injected_evidence_the_runtime_resources_answer(self) -> None:
        class Score:
            def __init__(self, known: bool) -> None:
                self.known = known

        class Dictionary:
            loads = 0

            def __init__(self, locale: str) -> None:
                self.locale = locale

            def score(self, word: str) -> Score:
                return Score(word in {"dist"})

        def load(locale: str, _extra: object) -> Dictionary:
            Dictionary.loads += 1
            return Dictionary(locale)

        with patch.object(IdentifierLexicon, "try_load", return_value=(None, "absent")), \
                patch.object(LanguageModel, "load", side_effect=load):
            model = OrthoModel.load(self.write(featured()))
            self.assertEqual(Dictionary.loads, 0)                  # loading the artifact loads no dictionary
            passed = model.feature_values(".dist", "ru", known)
            self.assertEqual(Dictionary.loads, 0)                  # the caller's dictionaries answer
            values = model.feature_values(".dist", "ru")
            model.feature_values(".dist", "ru")
        self.assertEqual(Dictionary.loads, len(LOCALES))           # its own copy, once, only when not passed
        self.assertEqual(values, passed)
        self.assertEqual(values["dot_word"], 1)
        self.assertEqual(values["ident"], 0)
        packaged = OrthoModel.load(self.write(featured()), known=known)
        self.assertEqual(packaged.feature_values("htop", "ru")["ident"], 1)

    def test_the_scope_and_the_collapse_are_the_artifacts(self) -> None:
        scoped = {**featured(), "collapse_runs": FEATURE_FIXTURE_COLLAPSE, "unscored_shapes": ["upper"]}
        model = OrthoModel.load(self.write(scoped), identifier=identifier, known=known)
        self.assertFalse(model.score(OrthoEvidence("hcach", "upper", "ru")).supported)
        self.assertTrue(model.score(OrthoEvidence("hcach", "lower", "ru")).supported)
        stretched = model.score(OrthoEvidence("abbb", "lower", "ru"))
        cut = model.score(OrthoEvidence("abb", "lower", "ru"))
        self.assertEqual((stretched.source_logprob, stretched.target_logprob), (cut.source_logprob, cut.target_logprob))
        plain = OrthoModel.load(self.write(featured()), identifier=identifier, known=known)
        self.assertNotEqual(plain.score(OrthoEvidence("abbb", "lower", "ru")).source_logprob, cut.source_logprob)
        self.assertEqual((plain.collapse, plain.unscored_shapes), (None, frozenset()))

    def test_a_stretch_keeps_the_reading_that_argues_less_for_the_other_layout(self) -> None:
        cut = OrthoModel.load(self.write(stretching(None)), identifier=identifier, known=known)
        read = OrthoModel.load(self.write(stretching(FEATURE_FIXTURE_STRETCH)), identifier=identifier, known=known)
        self.assertEqual(read.stretch, FEATURE_FIXTURE_STRETCH)
        self.assertIsNone(cut.stretch)
        # Cut to two, `abbb` reads `abb`, a double the English channel likes: evidence for English.
        stretched = OrthoEvidence("abbb", "lower", "ru")
        self.assertEqual(cut.score(stretched).ratio, cut.score(OrthoEvidence("abb", "lower", "ru")).ratio)
        self.assertGreater(cut.score(stretched).ratio, 0.0)
        # Read once, it is `ab`, on which the two channels agree; the smaller evidence wins.
        once = read.score(stretched)
        plain = read.score(OrthoEvidence("ab", "lower", "ru"))
        self.assertEqual((once.ratio, once.source_logprob, once.target_logprob),
                         (plain.ratio, plain.source_logprob, plain.target_logprob))
        # A double is not a stretch of three, and a reading already against the other layout stays.
        self.assertEqual(read.score(OrthoEvidence("abb", "lower", "ru")).ratio, cut.score(OrthoEvidence("abb", "lower", "ru")).ratio)
        towards_russian = OrthoEvidence("abbb", "lower", "en")
        self.assertEqual(read.score(towards_russian).ratio, cut.score(towards_russian).ratio)
        self.assertLess(read.score(towards_russian).ratio, 0.0)
        # With the shortest stretch, a double is read once too.
        doubles = OrthoModel.load(self.write(stretching(ORTHO_MIN_STRETCH_RUN)), identifier=identifier, known=known)
        self.assertEqual(doubles.score(OrthoEvidence("abb", "lower", "ru")).ratio, plain.ratio)
        uncollapsed = {**stretching(FEATURE_FIXTURE_STRETCH)}
        del uncollapsed["collapse_runs"]
        free = OrthoModel.load(self.write(uncollapsed), identifier=identifier, known=known)
        self.assertEqual(free.score(stretched).ratio, plain.ratio)

    def test_each_stretch_reading_is_judged_with_its_own_features(self) -> None:
        read = OrthoModel.load(self.write(stretching(FEATURE_FIXTURE_STRETCH)), identifier=identifier, known=known)
        # `maiil` is `mail` with one key too many; `maiiil` is `mail` stretched, and a stretch is no stray key.
        stray = read.score(OrthoEvidence("maiil", "lower", "ru"))
        self.assertEqual((stray.reading, stray.features), ("maiil", FEATURE_FIXTURE_DOT_WEIGHT / ORTHO_FIXTURE_SCALE))
        stretched = read.score(OrthoEvidence("maiiil", "lower", "ru"))
        self.assertEqual((stretched.reading, stretched.features), ("mail", 0.0))
        typed = OrthoModel.load(self.write(keyed()), identifier=identifier, known=known)
        self.assertEqual(typed.score(OrthoEvidence("maiiil", "lower", "ru")).reading, "maiiil")
        # A reading without its stretch that is a word of the layout typed is that word: `приииивет` is `привет`.
        greeting = read.score(OrthoEvidence("ghbbbdtn", "lower", "ru"))
        self.assertEqual((greeting.reading, greeting.features), ("ghbdtn", -FEATURE_FIXTURE_PARTS_WEIGHT / ORTHO_FIXTURE_SCALE))
        self.assertEqual(set(read.features["ru"]), set(FEATURES_SCHEMA_4))

    def test_schema_5_reads_hyphen_stretches_and_weighs_the_replacement(self) -> None:
        payload = stretching(FEATURE_FIXTURE_STRETCH)
        replaced = {script: {**row, "name_unknown": -FEATURE_FIXTURE_DOT_WEIGHT, "target_unknown": -FEATURE_FIXTURE_IDENT_WEIGHT,
                             "source_plausibility": 0}
                    for script, row in cast(dict[str, dict[str, int]], payload["features"]).items()}
        payload.update({"schema_version": ORTHO_REPLACEMENT_ARTIFACT_SCHEMA_VERSION, "features": replaced,
                        "feature_centres": {"en": 0, "ru": 0}})
        schema_5 = OrthoModel.load(self.write(payload), identifier=identifier, known=known)
        self.assertTrue(schema_5.hyphens)
        self.assertEqual(set(schema_5.features["ru"]), set(FEATURES_SCHEMA_5))
        # `b-b` is a drawn-out `b`: read once, it is `b`, and the reading kept is the one against the other layout.
        stretched = schema_5.score(OrthoEvidence("ab-bb", "lower", "ru"))
        self.assertIn(stretched.reading, ("ab-bb", "ab"))
        schema_4 = OrthoModel.load(self.write(stretching(FEATURE_FIXTURE_STRETCH)), identifier=identifier, known=known)
        self.assertFalse(schema_4.hyphens)
        # A capitalised token whose replacement is no word weighs both penalties; a lowercase one only one.
        inner = schema_5.score(OrthoEvidence("abab", "inner", "ru"))
        lower = schema_5.score(OrthoEvidence("abab", "lower", "ru"))
        self.assertEqual(inner.features - lower.features, -FEATURE_FIXTURE_DOT_WEIGHT / ORTHO_FIXTURE_SCALE)
        self.assertEqual(lower.features, -FEATURE_FIXTURE_IDENT_WEIGHT / ORTHO_FIXTURE_SCALE)
        # Source plausibility: the source log-probability per scored position, from the artifact's centre.
        plausible = {**payload, "features": {script: {**row, "source_plausibility": ORTHO_FIXTURE_SCALE}
                                             for script, row in replaced.items()},
                     "feature_centres": {"en": 0, "ru": -ORTHO_FIXTURE_SCALE}}
        weighed = OrthoModel.load(self.write(plausible), identifier=identifier, known=known)
        self.assertEqual(weighed.centres, {"en": 0.0, "ru": -1.0})
        scored = weighed.score(OrthoEvidence("abab", "lower", "ru"))
        self.assertAlmostEqual(scored.features - lower.features, scored.source_logprob / (len("abab") + 1) + 1.0)
        # A compound minimum: the artifact's, applied to the features the model computes.
        self.assertEqual(schema_5.compound_min_letters, {})
        minimum = {"en": FEATURE_FIXTURE_COMPOUND_MIN_LETTERS}
        strict = OrthoModel.load(self.write({**payload, "compound_min_letters": minimum}), identifier=identifier, known=known)
        self.assertEqual(strict.compound_min_letters, minimum)
        letters = lambda script, word: word in {"en": {"r", "ut"}, "ru": set()}[script]
        self.assertEqual(schema_5.feature_values("r-ut", "ru", known=letters)["parts_target"], 1)
        self.assertEqual(strict.feature_values("r-ut", "ru", known=letters)["parts_target"], 0)

    def test_schema_6_weighs_the_replacement_and_gates_a_stray_key_by_plausibility(self) -> None:
        payload = weighing()
        payload["features"] = {script: {**row, "extra_key": FEATURE_FIXTURE_DOT_WEIGHT, "target_plausibility": ORTHO_FIXTURE_SCALE}
                               for script, row in cast(dict[str, dict[str, int]], payload["features"]).items()}
        payload["compound_min_letters"] = {"en": FEATURE_FIXTURE_COMPOUND_MIN_LETTERS}
        model = OrthoModel.load(self.write(payload), identifier=identifier, known=known)
        self.assertEqual(set(model.features["en"]), set(FEATURES))
        self.assertTrue(model.hyphens)
        self.assertEqual((model.target_centres, model.gated), ({"en": 0.0, "ru": 0.0}, ()))
        self.assertEqual(model.compound_min_letters, {"en": FEATURE_FIXTURE_COMPOUND_MIN_LETTERS})
        stray = OrthoEvidence("aghbdtn", "lower", "en")        # `фпривет` is `привет` with a stray key
        scored = model.score(stray)
        self.assertAlmostEqual(scored.features, FEATURE_FIXTURE_DOT_WEIGHT / ORTHO_FIXTURE_SCALE
                               + scored.target_logprob / (len(scored.reading) + 1))
        # Gated: the stray key counts while the reading is less plausible than the centre of its layout ...
        gated = {**payload, "gated_features": ["extra_key"], "feature_centres": {"en": 0, "ru": 0}}
        noisy = OrthoModel.load(self.write(gated), identifier=identifier, known=known)
        self.assertEqual(noisy.gated, ("extra_key",))
        self.assertEqual(noisy.score(stray).features, scored.features)
        # ... and not once it is more plausible than that centre.
        gated["feature_centres"] = {"en": -FEATURE_FIXTURE_DOT_WEIGHT * ORTHO_FIXTURE_SCALE, "ru": 0}
        plausible = OrthoModel.load(self.write(gated), identifier=identifier, known=known)
        self.assertAlmostEqual(plausible.score(stray).features, scored.target_logprob / (len(scored.reading) + 1))

    def test_schema_3_weighs_the_stray_key_and_schema_2_does_not_ask(self) -> None:
        schema_2 = OrthoModel.load(self.write(featured()), identifier=identifier, known=known)
        schema_3 = OrthoModel.load(self.write(keyed()), identifier=identifier, known=known)
        self.assertEqual(set(schema_2.features["en"]), set(FEATURES_SCHEMA_2))
        self.assertEqual(set(schema_3.features["en"]), set(FEATURES_SCHEMA_3))
        stray = OrthoEvidence("aghbdtn", "lower", "en")
        self.assertEqual(schema_2.score(stray).features, 0.0)
        self.assertEqual(schema_3.score(stray).features, FEATURE_FIXTURE_DOT_WEIGHT / ORTHO_FIXTURE_SCALE)

    def test_a_source_channel_answers_only_for_its_own_script(self) -> None:
        # The Russian source channel finds every gram one nat less likely than the Russian channel.
        other = channel()
        other["logprob"] = [value - ORTHO_FIXTURE_SCALE for value in cast(list[int], other["logprob"])]
        plain = OrthoModel.load(self.write(featured()), identifier=identifier, known=known)
        sourced = OrthoModel.load(self.write({**featured(), "source_models": {"ru": other}}),
                                  identifier=identifier, known=known)
        typed_russian = [model.score(OrthoEvidence("ab", "lower", "ru")) for model in (plain, sourced)]
        typed_english = [model.score(OrthoEvidence("ab", "lower", "en")) for model in (plain, sourced)]
        # A token typed in Russian is judged Russian by the source channel ...
        self.assertLess(typed_russian[1].source_logprob, typed_russian[0].source_logprob)
        self.assertEqual(typed_russian[1].target_logprob, typed_russian[0].target_logprob)
        # ... while keys typed in English are compared with the curated Russian channel as before.
        self.assertEqual(typed_english[1], typed_english[0])
        self.assertEqual(set(sourced.source_channels), {"ru"})
        self.assertEqual(plain.source_channels, {})

    def test_a_score_uses_the_dictionaries_it_is_given(self) -> None:
        model = OrthoModel.load(self.write(featured()), identifier=identifier, known=lambda _script, _word: False)
        alone = model.score(OrthoEvidence("z-nj", "initial", "en"))
        given = model.score(OrthoEvidence("z-nj", "initial", "en"), known=known)
        self.assertEqual(alone.features, 0.0)
        self.assertEqual(given.features, FEATURE_FIXTURE_PARTS_WEIGHT / ORTHO_FIXTURE_SCALE)


class PolicyDictionaryTests(unittest.TestCase):
    """The context policy hands the detector's models to the orthotactic features."""

    def test_the_licence_asks_the_detectors_dictionaries(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "ortho.json"
            path.write_text(json.dumps({**featured(), "thresholds": {"en": 0, "ru": 0}}), encoding="utf-8")
            model = OrthoModel.load(path, identifier=identifier)
        policy = ContextPolicy()
        policy.ortho = model
        models = {group: LanguageModel(locale, {word: 1 for word in KNOWN[script]}, "fixture", enable_spellcheck=False)
                  for group, (script, locale) in enumerate(LOCALES.items())}
        detector = LanguageDetector(models)
        field = FieldContext("Code", "1", "", "")
        baseline = DetectionDecision(False, "z-nj", "z-nj", 0, 0, 0.0, "не требуется",
                                     WordScore(0.0, False, 0, 0.0), WordScore(0.0, False, 0, 0.0))
        with patch.object(LanguageModel, "load", side_effect=AssertionError("the policy passes its models")):
            given = policy._orthotactic(baseline, "я-то", 1, field, detector)
        assert given is not None
        # `я` and `то` are words of the detector's Russian model, so `Я-то` carries the compound weight.
        self.assertEqual(given.confidence, model.score(OrthoEvidence("z-nj", "lower", "en"), known=known).total)


if __name__ == "__main__":
    unittest.main()
