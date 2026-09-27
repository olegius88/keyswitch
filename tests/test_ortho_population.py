"""The orthotactic corpus measures what the engine serves the model, and the trainer counts reproducibly."""
from __future__ import annotations

import json
import sys
import tempfile
import unittest
from collections import Counter
from pathlib import Path
from typing import cast
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
TOOLS_PATH = str(ROOT / "tools")
if TOOLS_PATH not in sys.path:
    sys.path.insert(0, TOOLS_PATH)

import ortho_corpus as corpus  # noqa: E402
import ortho_known as known  # noqa: E402
import ortho_prose_lexicon as prose  # noqa: E402
import ortho_web_lexicon as web  # noqa: E402
import train_ortho_model as trainer  # noqa: E402
from keyswitch import engine  # noqa: E402
from keyswitch.backend import KeyEvent  # noqa: E402
from keyswitch.constants.model_protocol import TEST, TRAIN  # noqa: E402
from keyswitch.constants.ortho import (  # noqa: E402
    ORTHO_EXTRA_KEY_ARTIFACT_SCHEMA_VERSION,
    ORTHO_PLAUSIBILITY_ARTIFACT_SCHEMA_VERSION,
    ORTHO_REPLACEMENT_ARTIFACT_SCHEMA_VERSION,
    ORTHO_STRETCH_ARTIFACT_SCHEMA_VERSION,
    ORTHO_V1_CONFIG_SCHEMA_VERSION,
    ORTHO_V1_TEST_SPLIT_CEILING,
)
from keyswitch.constants.corpus import CONTEXT_PHRASE_SPLIT_BUCKET_COUNT  # noqa: E402
from keyswitch.identifier_lexicon import IdentifierLexicon  # noqa: E402
from keyswitch.ortho_model import FEATURES, FEATURES_SCHEMA_3, SCRIPTS, OrthoModel  # noqa: E402
from fixture_values.ortho import (  # noqa: E402
    FEATURE_FIXTURE_COLLAPSE,
    FEATURE_FIXTURE_COMPOUND_MIN_LETTERS,
    FEATURE_FIXTURE_DOT_WEIGHT,
    FEATURE_FIXTURE_IDENT_WEIGHT,
    FEATURE_FIXTURE_MIN_TYPES,
    FEATURE_FIXTURE_PARTS_WEIGHT,
    FEATURE_FIXTURE_REPEAT_WEIGHT,
    FEATURE_FIXTURE_STRETCH,
    FIRTH_FIXTURE_CLASSES,
    FIRTH_FIXTURE_MARK_EVERY,
    POPULATION_FIXTURE_DISCOUNT,
    POPULATION_FIXTURE_FOLDS,
    POPULATION_FIXTURE_GROUPS,
    POPULATION_FIXTURE_L2,
    POPULATION_FIXTURE_LEXICON_WEIGHT,
    POPULATION_FIXTURE_MINIMUM_LENGTH,
    POPULATION_FIXTURE_MINIMUM_RECALL,
    POPULATION_FIXTURE_MINIMUM_TEST_NEGATIVES,
    POPULATION_FIXTURE_ORDER,
    POPULATION_FIXTURE_SCALE,
    POPULATION_FIXTURE_TEST_SHARE_MAX,
    POPULATION_FIXTURE_TEST_SHARE_MIN,
    POPULATION_FIXTURE_THRESHOLD_EN_NATS,
    POPULATION_FIXTURE_THRESHOLD_RU_NATS,
    POPULATION_FIXTURE_TOKEN_OCCURRENCES,
    SYNTHETIC_FIXTURE_SHARE_MAX,
    SYNTHETIC_FIXTURE_SHARE_MIN,
    WEB_FIXTURE_LIST_WEIGHT,
    WEB_FIXTURE_WEB_WEIGHT,
)


def settings() -> dict[str, object]:
    return {
        "schema_version": ORTHO_V1_CONFIG_SCHEMA_VERSION, "order": POPULATION_FIXTURE_ORDER,
        "discount": POPULATION_FIXTURE_DISCOUNT, "scale": POPULATION_FIXTURE_SCALE,
        "minimum_length": POPULATION_FIXTURE_MINIMUM_LENGTH,
        "thresholds_nats": {"en": POPULATION_FIXTURE_THRESHOLD_EN_NATS, "ru": POPULATION_FIXTURE_THRESHOLD_RU_NATS},
        "feature_folds": POPULATION_FIXTURE_FOLDS, "feature_l2": POPULATION_FIXTURE_L2,
        "collapse_runs": FEATURE_FIXTURE_COLLAPSE, "unscored_shapes": ["upper"],
        "promotion": {"minimum_test_negatives": POPULATION_FIXTURE_MINIMUM_TEST_NEGATIVES,
                      "maximum_test_false_conversions": 0, "minimum_recall": POPULATION_FIXTURE_MINIMUM_RECALL},
    }


def row(split: str, script: str, keys: str, **shapes: int) -> dict[str, object]:
    return {"split": split, "script": script, "keys": keys, "shapes": dict(shapes)}


class EngineMirrorTests(unittest.TestCase):
    """Sets copied from the engine must stay the engine's."""

    def test_the_boundary_predicate_is_the_engines(self) -> None:
        probe = KeyEngine.__new__(KeyEngine)
        characters = [chr(code) for code in range(ord("!"), ord("~") + 1)] + list("«»—–…’‐‑№")
        for character in characters:
            with self.subTest(character=character):
                event = KeyEvent(True, 0, character, character, (character, character), 0, 0, 0)
                self.assertEqual(corpus.ends_a_word(character), probe._is_boundary(event))
        self.assertEqual(corpus.BOUNDARY_PUNCTUATION, frozenset(engine.PUNCTUATION))

    def test_the_joiners_are_the_ones_the_engine_appends_before_a_boundary(self) -> None:
        self.assertEqual(corpus.WORD_JOINERS, frozenset(engine.WORD_JOINERS) | frozenset("_@/\\="))


KeyEngine = engine.KeySwitchEngine


class ServedPopulationTests(unittest.TestCase):
    def test_russian_punctuation_sits_on_its_own_key(self) -> None:
        self.assertEqual(corpus.to_keys("привет,"), "ghbdtn/")
        self.assertEqual(corpus.to_keys("юг."), ".u/")
        self.assertEqual(corpus.visible("ghbdtn/", "ru"), "привет.")

    def test_the_engine_cuts_the_token_by_the_layout_being_typed(self) -> None:
        self.assertEqual(corpus.served("french,", "en"), ("french",))
        self.assertEqual(corpus.served("french,", "ru"), ("french,",))     # the comma key is `б`
        self.assertEqual(corpus.served('"ably"', "en"), ("ably",))
        self.assertEqual(corpus.served("word!more", "en"), ("word", "more"))
        self.assertEqual(corpus.served("z-nj", "en"), ("z-nj",))          # a hyphen joins a started word
        self.assertEqual(corpus.served("bild/c,jhrb", "en"), ("c,jhrb",))  # only what follows the last slash
        self.assertEqual(corpus.served("ghbdtn/", "ru"), ("ghbdtn",))      # the Russian full stop ends it
        self.assertEqual(corpus.served("!!", "en"), ())

    def test_the_sentence_language_typed_in_the_wrong_layout_is_labelled_as_that_language(self) -> None:
        words = {"ru": frozenset({"сказал", "мы", "люблю", "шею", "часу"}), "en": frozenset({"hello", "vs", "it"})}
        self.assertEqual(corpus.label("crfpfk", "rus", words), "ru")        # `сказал` typed in the Latin layout
        self.assertEqual(corpus.label("vs", "rus", words), "en")            # an English word stays English
        self.assertEqual(corpus.label("руддщ", "eng", words), "en")         # `hello` typed in the Russian layout
        self.assertEqual(corpus.label("xyzzy", "rus", words), "en")         # no Russian reading: its own alphabet
        self.assertEqual(corpus.label("привет", "rus", words), "ru")
        self.assertIsNone(corpus.label("gtk-ключ", "rus", words))
        # Punctuation: the quotes and question mark come off the core, ...
        self.assertEqual(corpus.label('crfpfk?".', "rus", words), "ru")
        self.assertEqual(corpus.label('руддщ",', "eng", words), "en")
        # ... while punctuation keys that are letters in the other layout read whole (`люблю`) ...
        self.assertEqual(corpus.label("k.,k.", "rus", words), "ru")
        # ... and a word is judged by its core: `it.` is the English `it`, not `шею` typed in the Latin layout.
        self.assertEqual(corpus.label("it.", "rus", words), "en")
        # The sentence stays part of the rule: an English abbreviation in an English sentence is English.
        self.assertEqual(corpus.label("XFCE.", "eng", words), "en")

    def test_word_cores_and_shapes(self) -> None:
        self.assertEqual(corpus.word_core('"and",', "en"), "and")
        self.assertEqual(corpus.word_core("french,", "ru"), "french,")
        self.assertEqual(corpus.shape_of("РСФСР", 0), "upper")
        self.assertEqual(corpus.shape_of("Дэн", 1), "inner")
        self.assertEqual(corpus.shape_of("Привет", 0), "initial")
        self.assertIsNone(corpus.script_of("gtk-ключ"))

    def test_the_split_is_drawn_per_group_from_the_namespace(self) -> None:
        splits = Counter(corpus.split_of(f"tatoeba:{index}") for index in range(POPULATION_FIXTURE_GROUPS))
        self.assertEqual(set(splits), {TEST, TRAIN})
        share = splits[TEST] / POPULATION_FIXTURE_GROUPS
        self.assertGreater(share, POPULATION_FIXTURE_TEST_SHARE_MIN)
        self.assertLess(share, POPULATION_FIXTURE_TEST_SHARE_MAX)
        self.assertLess(ORTHO_V1_TEST_SPLIT_CEILING, CONTEXT_PHRASE_SPLIT_BUCKET_COUNT)

    def test_the_population_is_what_the_engine_serves_the_model(self) -> None:
        rows = [
            row(TEST, "ru", corpus.to_keys("гифка"), lower=1),          # unknown Russian word: a negative
            row(TEST, "ru", corpus.to_keys("привет"), lower=1),         # known: never served
            row(TEST, "en", "htop", lower=1),                           # English typed in Russian: a positive
            row(TEST, "en", "lópez", inner=1),                          # not keys of the us/ru pair
            row(TEST, "en", "it", lower=1),                             # shorter than the model allows
            row(TEST, "ru", corpus.to_keys("дюп"), lower=1),            # its other reading `l.g` is not a word
            row(TRAIN, "ru", corpus.to_keys("гифка"), lower=1),         # another split
        ]
        dictionary = {"ru": frozenset({"привет"}), "en": frozenset()}
        labels = trainer.population(rows, TEST, dictionary, POPULATION_FIXTURE_MINIMUM_LENGTH)
        self.assertTrue(labels[("ru", "ubarf", "lower")])
        self.assertFalse(labels[("ru", "htop", "lower")])
        self.assertTrue(labels[("en", "htop", "lower")])
        self.assertNotIn(("ru", corpus.to_keys("привет"), "lower"), labels)
        self.assertFalse(any(keys in {"lópez", "it", corpus.to_keys("дюп")} for _direction, keys, _shape in labels))

    def test_a_sequence_that_is_both_a_negative_and_a_positive_counts_as_a_negative(self) -> None:
        rows = [row(TEST, "en", "ubarf", lower=1), row(TEST, "ru", corpus.to_keys("гифка"), lower=1)]
        labels = trainer.population(rows, TEST, {"ru": frozenset(), "en": frozenset()}, POPULATION_FIXTURE_MINIMUM_LENGTH)
        self.assertTrue(labels[("ru", "ubarf", "lower")])
        self.assertTrue(labels[("en", "ubarf", "lower")])


class CountingTests(unittest.TestCase):
    def test_word_lists_add_characters_but_never_a_case_shape(self) -> None:
        rows = [row(TRAIN, "en", "htop", lower=POPULATION_FIXTURE_TOKEN_OCCURRENCES),
                row(corpus.LEXICON, "en", "nginx", lower=POPULATION_FIXTURE_LEXICON_WEIGHT),
                row(TEST, "en", "unseen", lower=1)]
        grams, shapes, acronyms = trainer.counts(rows, POPULATION_FIXTURE_ORDER)
        self.assertEqual(grams["en"]["h"], POPULATION_FIXTURE_TOKEN_OCCURRENCES)
        self.assertEqual(grams["en"]["n"], "nginx".count("n") * POPULATION_FIXTURE_LEXICON_WEIGHT)
        self.assertEqual(grams["en"]["u"], 0)
        self.assertEqual(shapes["en"]["lower"], POPULATION_FIXTURE_TOKEN_OCCURRENCES)
        self.assertEqual(sum(acronyms["en"].values()), 0)

    def test_a_stretched_letter_is_counted_cut_to_the_configured_run(self) -> None:
        rows = [row(TRAIN, "en", "sooo", lower=POPULATION_FIXTURE_TOKEN_OCCURRENCES)]
        grams, _shapes, _acronyms = trainer.counts(rows, POPULATION_FIXTURE_ORDER, FEATURE_FIXTURE_COLLAPSE)
        self.assertEqual(grams["en"]["o"], FEATURE_FIXTURE_COLLAPSE * POPULATION_FIXTURE_TOKEN_OCCURRENCES)

    def test_each_distinct_letter_core_of_a_token_is_counted(self) -> None:
        # `top,` is `top` typed in English and `top,` (the comma key is `б`) typed in Russian.
        rows = [row(TRAIN, "en", "top,", lower=POPULATION_FIXTURE_TOKEN_OCCURRENCES)]
        grams, shapes, _acronyms = trainer.counts(rows, POPULATION_FIXTURE_ORDER)
        cores = {corpus.word_core("top,", layout) for layout in SCRIPTS}
        self.assertEqual(cores, {"top", "top,"})
        self.assertEqual(grams["en"]["t"], len(cores) * POPULATION_FIXTURE_TOKEN_OCCURRENCES)
        self.assertEqual(shapes["en"]["lower"], len(cores) * POPULATION_FIXTURE_TOKEN_OCCURRENCES)

    def test_the_web_forms_count_only_into_the_source_channel_of_their_script(self) -> None:
        fnaf, kringe = corpus.to_keys("фнаф"), corpus.to_keys("кринж")
        rows = [row(TRAIN, "en", "htop", lower=POPULATION_FIXTURE_TOKEN_OCCURRENCES),
                row(TRAIN, "ru", corpus.to_keys("привет"), lower=POPULATION_FIXTURE_TOKEN_OCCURRENCES),
                row(corpus.LEXICON, "ru", fnaf, lower=WEB_FIXTURE_LIST_WEIGHT),
                row(corpus.LEXICON, "en", "nginx", lower=POPULATION_FIXTURE_LEXICON_WEIGHT),
                row(TEST, "ru", corpus.to_keys("тест"), lower=1)]
        forms = [row(corpus.LEXICON, "ru", fnaf, lower=WEB_FIXTURE_WEB_WEIGHT), row(corpus.LEXICON, "ru", kringe, lower=1)]
        sourced = trainer.source_rows(rows, forms)
        # The train tokens and the word list of the script, merged with the web forms at the larger weight.
        self.assertEqual(sourced, [rows[0], rows[1], row(corpus.LEXICON, "ru", fnaf, lower=WEB_FIXTURE_WEB_WEIGHT),
                                   row(corpus.LEXICON, "ru", kringe, lower=1)])
        plain = trainer.char_payload(rows, settings())
        payload = trainer.char_payload(rows, settings(), forms)
        self.assertNotIn("source_models", plain)
        self.assertEqual({key: value for key, value in payload.items() if key != "source_models"}, plain)
        grams, _shapes, _acronyms = trainer.counts(sourced, POPULATION_FIXTURE_ORDER, FEATURE_FIXTURE_COLLAPSE)
        self.assertEqual(payload["source_models"], {"ru": trainer.channel_payload(grams["ru"], settings())})

    def test_the_trainer_reads_the_frozen_web_forms_the_configuration_names(self) -> None:
        self.assertEqual(trainer.web_rows(settings()), [])
        with self.assertRaises(ValueError):
            trainer.web_rows({**settings(), "web_source_scripts": ["en"]})
        with tempfile.TemporaryDirectory() as directory:
            forms, receipt = Path(directory) / "forms.json.gz", Path(directory) / "receipt.json"
            listed = [["фнаф", 0], ["кринж", POPULATION_FIXTURE_LEXICON_WEIGHT]]
            receipt.write_text(json.dumps(web.receipt(listed, web.write(listed, forms))), encoding="utf-8")
            with patch.object(web, "FORMS", forms), patch.object(web, "RECEIPT", receipt):
                rows = trainer.web_rows({**settings(), "web_source_scripts": ["ru"]})
                self.assertEqual(rows, [row(corpus.LEXICON, "ru", corpus.to_keys(str(form)), lower=web.web_weight(cast(int, rank)))
                                        for form, rank in sorted(listed, key=str)])
                web.write(listed[:1], forms)
                with self.assertRaises(ValueError):
                    trainer.web_rows({**settings(), "web_source_scripts": ["ru"]})

    def test_the_trainer_reads_the_frozen_prose_forms_the_configuration_names(self) -> None:
        with self.assertRaises(ValueError):
            trainer.web_rows({**settings(), "prose_source_scripts": ["en"]})
        with tempfile.TemporaryDirectory() as directory:
            forms, receipt = Path(directory) / "prose.json.gz", Path(directory) / "receipt.json"
            table = {"куафер": POPULATION_FIXTURE_LEXICON_WEIGHT, "гыр-гыр": 1}
            receipt.write_text(json.dumps({"forms_sha256": prose.write(table, forms)}), encoding="utf-8")
            with patch.object(prose, "FORMS", forms), patch.object(prose, "RECEIPT", receipt):
                rows = trainer.web_rows({**settings(), "prose_source_scripts": ["ru"]})
                self.assertEqual(rows, [row(corpus.LEXICON, "ru", corpus.to_keys(form), lower=weight) for form, weight in sorted(table.items())])
                prose.write({"куафер": 1}, forms)
                with self.assertRaises(ValueError):
                    trainer.web_rows({**settings(), "prose_source_scripts": ["ru"]})

    def test_the_artifact_carries_the_configured_thresholds_features_and_replays(self) -> None:
        rows = [row(TRAIN, "en", "hello", lower=1), row(TRAIN, "ru", corpus.to_keys("привет"), lower=1),
                row(TRAIN, "ru", corpus.to_keys("РСФСР"), upper=1)]
        fitted = {script: {name: float(index) for index, name in enumerate(FEATURES)} for script in SCRIPTS}
        lexicon = IdentifierLexicon(frozenset({"htop"}), "fixture", "identifiers-fixture")
        dictionary: dict[str, frozenset[str]] = {"en": frozenset(), "ru": frozenset()}
        with patch.object(trainer, "feature_weights", return_value=fitted):
            payload = trainer.build(rows, settings(), dictionary, lexicon)
            self.assertEqual(payload, trainer.build(rows, settings(), dictionary, lexicon))
        self.assertTrue(str(payload["version"]).startswith("ortho-v1-"))
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "candidate.json"
            path.write_bytes(trainer.canonical(payload))
            model = OrthoModel.load(path, identifier=lexicon.contains, known=lambda _script, _word: False)
        self.assertEqual(model.features["ru"], {name: fitted["ru"][name] for name in FEATURES_SCHEMA_3})
        self.assertEqual(set(model.thresholds), set(SCRIPTS))
        self.assertAlmostEqual(model.thresholds["en"], POPULATION_FIXTURE_THRESHOLD_EN_NATS)
        self.assertAlmostEqual(model.thresholds["ru"], POPULATION_FIXTURE_THRESHOLD_RU_NATS, delta=1 / POPULATION_FIXTURE_SCALE)
        outcome = trainer.outcome(model, {("en", "hello", "lower"): True, ("ru", "hello", "lower"): False})
        counts = cast(dict[str, int], outcome["counts"])
        self.assertEqual(counts["en:negative_types"], 1)
        self.assertEqual(counts["ru:positive_types"], 1)
        self.assertEqual((payload["schema_version"], model.stretch), (ORTHO_EXTRA_KEY_ARTIFACT_SCHEMA_VERSION, None))

    def test_hyphen_stretches_make_a_schema_5_artifact_with_centres(self) -> None:
        replacing = {**settings(), "stretch_runs": FEATURE_FIXTURE_STRETCH, "hyphen_stretch": True}
        self.assertEqual(trainer.scope(replacing)["schema_version"], ORTHO_REPLACEMENT_ARTIFACT_SCHEMA_VERSION)
        rows = [row(TRAIN, "en", "hello", lower=1), row(TRAIN, "ru", corpus.to_keys("привет"), lower=1)]
        model = trainer.char_model(trainer.char_payload(rows, replacing), replacing)
        self.assertTrue(model.hyphens)
        self.assertEqual(model.centres, {"en": 0.0, "ru": 0.0})
        lexicon = IdentifierLexicon(frozenset({"htop"}), "fixture", "identifiers-fixture")
        dictionary: dict[str, frozenset[str]] = {"en": frozenset(), "ru": frozenset()}
        fitted = {script: {name: 0.0 for name in FEATURES} for script in SCRIPTS}

        measured = {"en": -float(FEATURE_FIXTURE_STRETCH), "ru": -float(FEATURE_FIXTURE_COLLAPSE)}

        def weights(*args: object, **keywords: object) -> dict[str, dict[str, float]]:
            cast(dict[str, float], keywords["centres"]).update(measured)
            return fitted

        with patch.object(trainer, "feature_weights", side_effect=weights):
            payload = trainer.build(rows, replacing, dictionary, lexicon)
        self.assertEqual(payload["feature_centres"], {script: int(value * POPULATION_FIXTURE_SCALE) for script, value in measured.items()})

    def test_the_fit_measures_source_plausibility_and_reports_its_centre(self) -> None:
        rows = [row(TRAIN, "en", "hello", lower=1)]
        lexicon = IdentifierLexicon(frozenset({"htop"}), "fixture", "identifiers-fixture")
        labels = {("ru", "htop", "lower"): False, ("en", "htop", "lower"): True}
        centres: dict[str, float] = {}
        with patch.object(trainer, "population", return_value=labels), patch.object(trainer, "char_payload", return_value={}), \
                patch.object(trainer, "char_model", return_value=StubModel()), \
                patch.object(trainer, "logistic", return_value=[1.0] + [0.0] * len(FEATURES)) as fit:
            trainer.feature_weights(rows, settings(), {"en": frozenset(), "ru": frozenset()}, lexicon, centres=centres)
        first = fit.call_args_list[0].args[0][0]
        self.assertEqual(first[FEATURES.index("source_plausibility") + 1], 0.0)   # the stub scores log-probability 0
        self.assertEqual(centres, {"en": 0.0, "ru": 0.0})

    def test_a_schema_5_artifact_carries_the_compound_minimum_and_the_fit_applies_it(self) -> None:
        minimum = {"en": FEATURE_FIXTURE_COMPOUND_MIN_LETTERS}
        replacing = {**settings(), "stretch_runs": FEATURE_FIXTURE_STRETCH, "hyphen_stretch": True, "compound_min_letters": minimum}
        self.assertEqual(trainer.scope(replacing)["compound_min_letters"], minimum)
        self.assertNotIn("compound_min_letters", trainer.scope({**replacing, "compound_min_letters": {}}))
        with self.assertRaises(ValueError):
            trainer.scope({**settings(), "stretch_runs": FEATURE_FIXTURE_STRETCH, "compound_min_letters": minimum})
        rows = [row(TRAIN, "en", "hello", lower=1), row(TRAIN, "ru", corpus.to_keys("привет"), lower=1)]
        model = trainer.char_model(trainer.char_payload(rows, replacing), replacing)
        self.assertEqual(model.compound_min_letters, minimum)
        lexicon = IdentifierLexicon(frozenset({"htop"}), "fixture", "identifiers-fixture")
        labels = {("ru", "r-ut", "lower"): False, ("en", "htop", "lower"): True}
        strict = StubModel()
        strict.compound_min_letters = minimum
        asked: list[object] = []

        def features(*args: object, **keywords: object) -> dict[str, int]:
            asked.append(keywords["compound_min_letters"])
            return {name: 0 for name in FEATURES if name != "source_plausibility"}

        with patch.object(trainer, "population", return_value=labels), patch.object(trainer, "char_payload", return_value={}), \
                patch.object(trainer, "char_model", return_value=strict), patch.object(trainer, "token_features", side_effect=features), \
                patch.object(trainer, "logistic", return_value=[1.0] + [0.0] * len(FEATURES)):
            trainer.feature_weights(rows, settings(), {"en": frozenset(), "ru": frozenset()}, lexicon)
        self.assertTrue(asked)
        self.assertTrue(all(value == minimum for value in asked))

    def test_the_plausibility_of_the_replacement_makes_a_schema_6_artifact(self) -> None:
        replacing = {**settings(), "stretch_runs": FEATURE_FIXTURE_STRETCH, "hyphen_stretch": True}
        weighing = {**replacing, "target_plausibility": True, "feature_gated": ["extra_key"]}
        self.assertEqual(trainer.scope(weighing)["schema_version"], ORTHO_PLAUSIBILITY_ARTIFACT_SCHEMA_VERSION)
        for broken in ({**settings(), "stretch_runs": FEATURE_FIXTURE_STRETCH, "target_plausibility": True},
                       {**replacing, "feature_gated": ["extra_key"]}):
            with self.assertRaises(ValueError):
                trainer.scope(broken)
        rows = [row(TRAIN, "en", "hello", lower=1), row(TRAIN, "ru", corpus.to_keys("привет"), lower=1)]
        model = trainer.char_model(trainer.char_payload(rows, weighing), weighing)
        self.assertEqual((model.centres, model.target_centres), ({"en": 0.0, "ru": 0.0}, {"en": 0.0, "ru": 0.0}))
        lexicon = IdentifierLexicon(frozenset({"htop"}), "fixture", "identifiers-fixture")
        fitted = {script: {name: 0.0 for name in FEATURES} for script in SCRIPTS}
        measured = {"en": -float(FEATURE_FIXTURE_STRETCH), "ru": -float(FEATURE_FIXTURE_COLLAPSE)}

        def weights(*args: object, **keywords: object) -> dict[str, dict[str, float]]:
            cast(dict[str, float], keywords["centres"]).update(measured)
            cast(dict[str, float], keywords["target_centres"]).update(measured)
            return fitted

        with patch.object(trainer, "feature_weights", side_effect=weights):
            payload = trainer.build(rows, weighing, {"en": frozenset(), "ru": frozenset()}, lexicon)
        quantised = {script: int(value * POPULATION_FIXTURE_SCALE) for script, value in measured.items()}
        self.assertEqual((payload["feature_centres"], payload["target_centres"]), (quantised, quantised))
        self.assertEqual(payload["gated_features"], ["extra_key"])

    def test_the_fit_gates_a_column_above_the_centre_and_measures_the_target(self) -> None:
        rows = [row(TRAIN, "en", "hello", lower=1)]
        lexicon = IdentifierLexicon(frozenset({"htop"}), "fixture", "identifiers-fixture")

        class Plausible(StubModel):
            def score(self, evidence: object) -> object:
                class Score:
                    total = 0.0
                    supported = True
                    reading = getattr(evidence, "keys")
                    # One token below the centre of its direction, one above it; the target channel likes both alike.
                    source_logprob = -float(FEATURE_FIXTURE_STRETCH) if getattr(evidence, "shape") == "lower" else 0.0
                    target_logprob = -float(FEATURE_FIXTURE_STRETCH)
                return Score()

        both = {("ru", "htop", "lower"): False, ("ru", "htop", "inner"): True}
        stray = {name: 0 for name in FEATURES}
        stray["extra_key"] = 1
        centres: dict[str, float] = {}
        target_centres: dict[str, float] = {}
        with patch.object(trainer, "population", return_value=both), patch.object(trainer, "char_payload", return_value={}), \
                patch.object(trainer, "char_model", return_value=Plausible()), \
                patch.object(trainer, "token_features", return_value=stray), \
                patch.object(trainer, "logistic", return_value=[1.0] + [0.0] * len(FEATURES)) as fit:
            trainer.feature_weights(rows, {**settings(), "feature_gated": ["extra_key"]}, {"en": frozenset(), "ru": frozenset()},
                                    lexicon, centres=centres, target_centres=target_centres)
        matrix = fit.call_args_list[1].args[0]
        column = FEATURES.index("extra_key") + 1
        half = len(matrix) // len(both)
        self.assertEqual(sorted(sample[column] for sample in matrix), [0.0] * half + [1.0] * half)
        self.assertEqual(target_centres["ru"], -float(FEATURE_FIXTURE_STRETCH) / (len("htop") + 1))

    def test_a_configured_stretch_makes_a_schema_4_artifact(self) -> None:
        stretching = {**settings(), "stretch_runs": FEATURE_FIXTURE_STRETCH}
        self.assertEqual(trainer.scope(stretching)["schema_version"], ORTHO_STRETCH_ARTIFACT_SCHEMA_VERSION)
        self.assertNotIn("stretch_runs", trainer.scope(settings()))
        rows = [row(TRAIN, "en", "hello", lower=1), row(TRAIN, "ru", corpus.to_keys("привет"), lower=1)]
        model = trainer.char_model(trainer.char_payload(rows, stretching), stretching)
        self.assertEqual(model.stretch, FEATURE_FIXTURE_STRETCH)


class StubModel:
    """A counted model that scores every token the same, for tests of what surrounds the counting."""

    hyphens = False
    compound_min_letters: dict[str, int] = {}

    def score(self, evidence: object) -> object:
        class Score:
            total = 0.0
            supported = True
            reading = getattr(evidence, "keys")
            source_logprob = 0.0
            target_logprob = 0.0
        return Score()


class FeatureFitTests(unittest.TestCase):
    def test_folds_are_drawn_by_letter_core(self) -> None:
        folds = {trainer.fold_of(row(TRAIN, "en", keys, lower=1), POPULATION_FIXTURE_FOLDS) for keys in ("hello", '"hello",', "hello.")}
        self.assertEqual(len(folds), 1)
        self.assertTrue(all(0 <= fold < POPULATION_FIXTURE_FOLDS for fold in folds))

    def test_the_logistic_fit_weighs_what_separates_the_labels(self) -> None:
        count = POPULATION_FIXTURE_GROUPS // POPULATION_FIXTURE_FOLDS
        marked = [index % POPULATION_FIXTURE_FOLDS == 0 for index in range(count)]
        samples = [[float(index), float(mark)] for index, mark in enumerate(marked)]
        # Labels follow the first column except where the second is set: that column is evidence against.
        labels = [int(index >= count // POPULATION_FIXTURE_FOLDS and not mark) for index, mark in enumerate(marked)]
        coefficients = trainer.logistic(samples, labels, POPULATION_FIXTURE_L2)
        self.assertGreater(coefficients[0], 0)
        self.assertLess(coefficients[1], 0)
        self.assertEqual(coefficients, trainer.logistic(samples, labels, POPULATION_FIXTURE_L2))

    def separable(self, mark_every: int) -> tuple[list[list[float]], list[int]]:
        """Samples whose second column fires only on positives (every `mark_every`-th positive): it separates."""
        count = POPULATION_FIXTURE_GROUPS // POPULATION_FIXTURE_FOLDS
        labels = [index % FIRTH_FIXTURE_CLASSES for index in range(count)]
        samples = [[float(index % POPULATION_FIXTURE_FOLDS) * (1 if label else -1),
                    float(label == 1 and index % mark_every == 1)] for index, label in enumerate(labels)]
        return samples, labels

    def test_firths_correction_keeps_a_separating_rare_column_finite_and_smaller(self) -> None:
        samples, labels = self.separable(FIRTH_FIXTURE_MARK_EVERY)
        plain = trainer.logistic(samples, labels, POPULATION_FIXTURE_L2)
        firth = trainer.logistic(samples, labels, POPULATION_FIXTURE_L2, firth=True)
        self.assertGreater(firth[1], 0)
        self.assertLess(firth[1] / firth[0], plain[1] / plain[0])
        self.assertEqual(firth, trainer.logistic(samples, labels, POPULATION_FIXTURE_L2, firth=True))

    def test_a_separated_column_carries_a_prior_centred_on_zero(self) -> None:
        samples, labels = self.separable(FIRTH_FIXTURE_MARK_EVERY)
        plain = trainer.logistic(samples, labels, POPULATION_FIXTURE_L2)
        alone = trainer.logistic(samples, labels, POPULATION_FIXTURE_L2, separated=[1])
        self.assertGreater(alone[1], 0)
        self.assertLess(alone[1] / alone[0], plain[1] / plain[0])
        self.assertEqual(alone, trainer.logistic(samples, labels, POPULATION_FIXTURE_L2, separated=[1]))
        # Rows the first column already predicts: the prior pulls the separating column toward zero, never past it.
        known = [[(1.0 if label else -1.0) * POPULATION_FIXTURE_FOLDS, mark] for (_score, mark), label in zip(samples, labels, strict=True)]
        pulled = trainer.logistic(known, labels, POPULATION_FIXTURE_L2, separated=[1])
        self.assertGreaterEqual(pulled[1], 0)
        self.assertLess(pulled[1], trainer.logistic(known, labels, POPULATION_FIXTURE_L2)[1])

    def test_the_configuration_lists_the_separated_columns_per_direction(self) -> None:
        ident = FEATURES.index("ident") + 1
        samples, labels = self.separable(FIRTH_FIXTURE_MARK_EVERY)
        rows = [[row[0]] + [0.0] * len(FEATURES) for row in samples]
        for row, sample in zip(rows, samples, strict=True):
            row[ident] = sample[1]
        both = {script: [list(row) for row in rows] for script in SCRIPTS}
        marks = {script: list(labels) for script in SCRIPTS}
        weights = trainer.fitted_weights(both, marks, {**settings(), "feature_separated": {"ru": ["ident"]}})
        self.assertGreater(weights["ru"]["ident"], 0)
        self.assertLess(weights["ru"]["ident"], weights["en"]["ident"])         # only Russian carries the correction
        shared = {**settings(), "feature_shared": ["extra_key"]}
        for broken in ({**settings(), "feature_separated": ["ident"]},
                       {**settings(), "feature_separated": {"de": ["ident"]}},
                       {**shared, "feature_separated": {"ru": ["extra_key"]}}):
            with self.subTest(broken=broken["feature_separated"]):
                with self.assertRaises(ValueError):
                    trainer.fitted_weights(both, marks, broken)

    def test_a_shared_weight_nothing_bounds_carries_a_prior_split_between_the_directions(self) -> None:
        samples, labels = self.separable(FIRTH_FIXTURE_MARK_EVERY)
        only = trainer.logistic([[row[0]] for row in samples], labels, POPULATION_FIXTURE_L2)[0]
        parts = {script: [(only * row[0], only * row[1], label) for row, label in zip(samples, labels, strict=True)]
                 for script in SCRIPTS}
        free = trainer.shared_weight(parts)
        held = trainer.shared_weight(parts, prior={script: only for script in SCRIPTS})
        self.assertGreater(held, 0)
        self.assertLess(held, free)
        stray = FEATURES.index("extra_key") + 1
        rows = [[row[0]] + [0.0] * len(FEATURES) for row in samples]
        for row, sample in zip(rows, samples, strict=True):
            row[stray] = sample[1]
        both = {script: [list(row) for row in rows] for script in SCRIPTS}
        marks = {script: list(labels) for script in SCRIPTS}
        shared = {**settings(), "feature_shared": ["extra_key"], "feature_min_types": FEATURE_FIXTURE_MIN_TYPES}
        plain = trainer.fitted_weights(both, marks, shared)
        bounded = trainer.fitted_weights(both, marks, {**shared, "feature_separated": {script: ["extra_key"] for script in SCRIPTS}})
        self.assertEqual(bounded["en"]["extra_key"], bounded["ru"]["extra_key"])
        self.assertLess(bounded["ru"]["extra_key"], plain["ru"]["extra_key"])
        with self.assertRaises(ValueError):
            trainer.fitted_weights(both, marks, {**shared, "feature_separated": {"ru": ["extra_key"]}})

    def test_one_weight_is_fitted_for_a_feature_both_directions_share(self) -> None:
        samples, labels = self.separable(FIRTH_FIXTURE_MARK_EVERY)
        # English has negatives with the feature too, so on its own it weighs it less than Russian does.
        english = [[row[0], row[1] or float(label == 0 and index % FIRTH_FIXTURE_MARK_EVERY == 0)]
                   for index, (row, label) in enumerate(zip(samples, labels, strict=True))]
        alone = {"en": trainer.logistic(english, labels, POPULATION_FIXTURE_L2),
                 "ru": trainer.logistic(samples, labels, POPULATION_FIXTURE_L2)}
        self.assertLess(alone["en"][1] / alone["en"][0], alone["ru"][1] / alone["ru"][0])
        only = {script: trainer.logistic([[row[0]] for row in rows], labels, POPULATION_FIXTURE_L2)[0]
                for script, rows in (("en", english), ("ru", samples))}
        parts = {script: [(only[script] * row[0], only[script] * row[1], label)
                          for row, label in zip(rows, labels, strict=True)]
                 for script, rows in (("en", english), ("ru", samples))}
        shared = trainer.shared_weight(parts)
        self.assertLess(alone["en"][1] / alone["en"][0], shared)
        self.assertLess(shared, alone["ru"][1] / alone["ru"][0])
        nothing = {script: [(value, 0.0, label) for value, _column, label in rows] for script, rows in parts.items()}
        self.assertEqual(trainer.shared_weight(nothing), 0.0)
        self.assertLess(trainer.shared_weight(parts, firth=True), shared)

    def test_the_configuration_names_shared_features_and_the_prior(self) -> None:
        # [char score, *FEATURES]; `extra_key` fires on positives in both directions.
        stray = FEATURES.index("extra_key") + 1
        samples, labels = self.separable(FIRTH_FIXTURE_MARK_EVERY)
        rows = [[row[0]] + [0.0] * len(FEATURES) for row in samples]
        for row, sample in zip(rows, samples, strict=True):
            row[stray] = sample[1]
        both = {script: [list(row) for row in rows] for script in SCRIPTS}
        marks = {script: list(labels) for script in SCRIPTS}
        ties: dict[str, dict[str, str]] = {}
        shared = {**settings(), "feature_shared": ["extra_key"], "feature_prior": trainer.FIRTH_PRIOR,
                  "feature_min_types": FEATURE_FIXTURE_MIN_TYPES}
        weights = trainer.fitted_weights(both, marks, shared, ties=ties)
        self.assertEqual(weights["en"]["extra_key"], weights["ru"]["extra_key"])
        self.assertGreater(weights["en"]["extra_key"], 0)
        self.assertEqual(ties["en"]["extra_key"], trainer.SHARED_TIE)
        plain = trainer.fitted_weights(both, marks, {**shared, "feature_prior": None})
        self.assertEqual(plain["en"]["extra_key"], plain["ru"]["extra_key"])
        self.assertGreater(plain["ru"]["extra_key"], weights["ru"]["extra_key"])     # Firth shrinks the separating column

    def test_a_character_score_that_predicts_nothing_is_refused(self) -> None:
        rows = [row(TRAIN, "en", "hello", lower=1)]
        lexicon = IdentifierLexicon(frozenset({"htop"}), "fixture", "identifiers-fixture")
        labels = {("ru", "htop", "lower"): False, ("en", "htop", "lower"): True}
        with patch.object(trainer, "population", return_value=labels), patch.object(trainer, "char_payload", return_value={}), \
                patch.object(trainer, "char_model", return_value=StubModel()), \
                patch.object(trainer, "logistic", return_value=[-1.0] + [0.0] * len(FEATURES)):
            with self.assertRaises(ValueError):
                trainer.feature_weights(rows, settings(), {"en": frozenset(), "ru": frozenset()}, lexicon)

    def test_a_token_outside_the_scope_is_neither_weighed_nor_converted(self) -> None:
        rows = [row(TRAIN, "en", "hello", lower=1)]
        lexicon = IdentifierLexicon(frozenset({"htop"}), "fixture", "identifiers-fixture")
        labels = {("ru", "htop", "lower"): False, ("ru", "hcach", "upper"): False, ("en", "htop", "lower"): True}

        class Scoped(StubModel):
            def score(self, evidence: object) -> object:
                class Score:
                    total = 0.0
                    supported = getattr(evidence, "shape") != "upper"
                    reading = getattr(evidence, "keys")
                    source_logprob = 0.0
                    target_logprob = 0.0
                return Score()

        with patch.object(trainer, "population", return_value=labels), patch.object(trainer, "char_payload", return_value={}), \
                patch.object(trainer, "char_model", return_value=Scoped()), \
                patch.object(trainer, "logistic", return_value=[1.0] + [0.0] * len(FEATURES)) as fit:
            trainer.feature_weights(rows, settings(), {"en": frozenset(), "ru": frozenset()}, lexicon)
        weighed = {len(call.args[0]) for call in fit.call_args_list}
        self.assertEqual(weighed, {POPULATION_FIXTURE_FOLDS})     # one supported sample per direction per fold

    def test_the_report_never_counts_a_token_outside_the_scope_as_converted(self) -> None:
        rows = [row(TRAIN, "en", "hello", lower=1), row(TRAIN, "ru", corpus.to_keys("привет"), lower=1)]
        lexicon = IdentifierLexicon(frozenset({"htop"}), "fixture", "identifiers-fixture")
        fitted = {script: {name: 0.0 for name in FEATURES} for script in SCRIPTS}
        low = {**settings(), "thresholds_nats": {"en": -POPULATION_FIXTURE_THRESHOLD_EN_NATS,
                                                 "ru": -POPULATION_FIXTURE_THRESHOLD_RU_NATS}}
        with patch.object(trainer, "feature_weights", return_value=fitted):
            payload = trainer.build(rows, low, {"en": frozenset(), "ru": frozenset()}, lexicon)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "candidate.json"
            path.write_bytes(trainer.canonical(payload))
            model = OrthoModel.load(path, identifier=lexicon.contains, known=lambda _script, _word: False)
        counts = cast(dict[str, int], trainer.outcome(model, {("ru", "hcach", "upper"): True, ("ru", "hcach", "lower"): True})["counts"])
        self.assertEqual(counts["ru:negative_types"], len(("upper", "lower")))
        self.assertEqual(counts["ru:false_types"], 1)             # the lower one; the capitals are not scored

    def test_a_subset_of_features_is_fitted_and_the_rest_weigh_nothing(self) -> None:
        rows = [row(TRAIN, "en", "hello", lower=1)]
        lexicon = IdentifierLexicon(frozenset({"htop"}), "fixture", "identifiers-fixture")
        labels = {("ru", "htop", "lower"): False, ("en", "htop", "lower"): True}
        with patch.object(trainer, "population", return_value=labels), patch.object(trainer, "char_payload", return_value={}), \
                patch.object(trainer, "char_model", return_value=StubModel()), \
                patch.object(trainer, "logistic", return_value=[1.0, 1.0]) as fit:
            weights = trainer.feature_weights(rows, settings(), {"en": frozenset(), "ru": frozenset()}, lexicon, ("ident",))
        self.assertEqual(len(fit.call_args_list[0].args[0][0]), len(("char", "ident")))
        self.assertEqual(weights["ru"], {name: 1.0 if name == "ident" else 0.0 for name in FEATURES})

    def test_feature_weights_are_relative_to_the_character_score(self) -> None:
        rows = [row(TRAIN, "en", "hello", lower=1)]
        lexicon = IdentifierLexicon(frozenset({"htop"}), "fixture", "identifiers-fixture")
        labels = {("ru", "htop", "lower"): False, ("en", "htop", "lower"): True}
        coefficients = [float(FEATURE_FIXTURE_IDENT_WEIGHT)] + [float(FEATURE_FIXTURE_IDENT_WEIGHT)] * len(FEATURES)
        with patch.object(trainer, "population", return_value=labels), patch.object(trainer, "char_payload", return_value={}), \
                patch.object(trainer, "char_model", return_value=StubModel()), \
                patch.object(trainer, "logistic", return_value=coefficients) as fit:
            weights = trainer.feature_weights(rows, settings(), {"en": frozenset(), "ru": frozenset()}, lexicon)
        self.assertEqual(weights["en"], {name: 1.0 for name in FEATURES})
        first = fit.call_args_list[0].args[0][0]
        # `htop` is a known command, and its Russian reading `рещз` is no word of the (empty) dictionary.
        expected = {name: 0.0 for name in FEATURES}
        expected.update({"ident": 1.0, "target_unknown": 1.0})
        self.assertEqual(first[1:], [expected[name] for name in FEATURES])


    def test_a_rare_feature_shares_the_weight_of_its_parent(self) -> None:
        # [char score, *FEATURES]; `dot_word` fires once, with `parts_target`; the others never.
        def sample(parts_target: float, dot_word: float) -> list[float]:
            values = {name: 0.0 for name in FEATURES}
            values.update({"parts_target": parts_target, "dot_word": dot_word})
            return [1.0] + [values[name] for name in FEATURES]
        rows = [sample(1.0, 1.0), sample(1.0, 0.0), sample(0.0, 0.0)]
        samples = {script: [list(row) for row in rows] for script in SCRIPTS}
        labels = {script: [1, 1, 0] for script in SCRIPTS}
        scale = float(FEATURE_FIXTURE_MIN_TYPES)
        # Kept columns: ident, repeat, parts_source, parts_target (with its tied rare features), and the two
        # replacement features and the two plausibilities, which have no parent.
        pooled = [scale, float(FEATURE_FIXTURE_IDENT_WEIGHT), float(FEATURE_FIXTURE_REPEAT_WEIGHT),
                  -float(FEATURE_FIXTURE_PARTS_WEIGHT), float(FEATURE_FIXTURE_DOT_WEIGHT), 0.0, 0.0, 0.0, 0.0]
        ties: dict[str, dict[str, str]] = {}
        rare = {**settings(), "feature_min_types": FEATURE_FIXTURE_MIN_TYPES}
        with patch.object(trainer, "logistic", return_value=pooled) as fit:
            weights = trainer.fitted_weights(samples, labels, rare, ties=ties)
        # One column for the pair, the sum of the two indicators, so a token where both fire counts twice.
        self.assertEqual(fit.call_args_list[0].args[0], [[1.0, 0.0, 0.0, 0.0, scale, 0.0, 0.0, 0.0, 0.0],
                                                         [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0],
                                                         [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]])
        self.assertEqual(weights["ru"]["dot_word"], weights["ru"]["parts_target"])
        self.assertEqual(weights["ru"]["extra_key"], weights["ru"]["parts_target"])
        self.assertEqual(weights["ru"]["parts_target"], FEATURE_FIXTURE_DOT_WEIGHT / scale)
        self.assertEqual(weights["ru"]["source_word"], weights["ru"]["parts_source"])
        self.assertEqual(ties, {script: {"dot_word": "parts_target", "extra_key": "parts_target",
                                         "source_word": "parts_source"} for script in SCRIPTS})
        # A feature the configuration always ties is tied whatever its support.
        always = {**settings(), "feature_min_types": 1, "feature_tied": ["dot_word"]}
        ties.clear()
        with patch.object(trainer, "logistic", return_value=pooled) as fit:
            weights = trainer.fitted_weights(samples, labels, always, ties=ties)
        self.assertEqual(weights["ru"]["dot_word"], weights["ru"]["parts_target"])
        self.assertEqual({script: table["dot_word"] for script, table in ties.items()}, {script: "parts_target" for script in SCRIPTS})
        supported: dict[str, dict[str, str]] = {}
        with patch.object(trainer, "logistic", return_value=[scale] + [float(FEATURE_FIXTURE_IDENT_WEIGHT)] * len(FEATURES)):
            trainer.fitted_weights(samples, labels, {**always, "feature_tied": []}, ties=supported)
        self.assertNotIn("dot_word", supported["ru"])                      # the fixture's `dot_word` has support 1
        # Enough support: every feature is fitted on its own and nothing is tied.
        separate = [scale] + [float(FEATURE_FIXTURE_IDENT_WEIGHT)] * len(FEATURES)
        for script in SCRIPTS:
            for name in ("extra_key", "source_word"):                      # they fire once too
                samples[script][1][FEATURES.index(name) + 1] = 1.0
        ties.clear()
        with patch.object(trainer, "logistic", return_value=separate) as fit:
            trainer.fitted_weights(samples, labels, {**settings(), "feature_min_types": 1}, ties=ties)
        self.assertEqual(len(fit.call_args_list[0].args[0][0]), 1 + len(FEATURES))
        self.assertEqual(ties, {script: {} for script in SCRIPTS})
        # A copied feature is left out of the regression - its column is neither kept nor added to the
        # parent's - and takes the parent's weight whatever its support.
        ties.clear()
        copied = {**settings(), "feature_min_types": 1, "feature_copied": ["extra_key"]}
        with patch.object(trainer, "logistic", return_value=separate[:-1]) as fit:
            weights = trainer.fitted_weights(samples, labels, copied, ties=ties)
        matrix = fit.call_args_list[0].args[0]
        self.assertEqual(len(matrix[0]), len(FEATURES))
        self.assertEqual([row[FEATURES.index("parts_target") + 1] for row in matrix], [1.0, 1.0, 0.0])
        self.assertEqual(weights["en"]["extra_key"], weights["en"]["parts_target"])
        self.assertEqual(ties, {script: {"extra_key": "parts_target"} for script in SCRIPTS})


class SyntheticStrayKeyTests(unittest.TestCase):
    def words(self) -> list[str]:
        # Letters on letter keys in both layouts, so a word typed in the other layout stays one token.
        letters = "авгдезиклмнопрстуфцчшщыья"
        return [corpus.to_keys(f"слово{first}{second}{third}") for first in letters for second in letters
                for third in letters][:POPULATION_FIXTURE_GROUPS]

    def test_a_share_of_words_gets_one_stray_key_drawn_by_the_word(self) -> None:
        drawn = {keys: corpus.extra_key_variant("ru", keys) for keys in self.words()}
        variants = {keys: variant for keys, variant in drawn.items() if variant is not None}
        share = len(variants) / len(drawn)
        self.assertTrue(SYNTHETIC_FIXTURE_SHARE_MIN < share < SYNTHETIC_FIXTURE_SHARE_MAX, share)
        self.assertEqual(drawn, {keys: corpus.extra_key_variant("ru", keys) for keys in self.words()})   # stable
        letters = {key for key in corpus.LETTER_KEYS["en"] if key == key.lower()}
        for keys, variant in variants.items():
            self.assertEqual(len(variant), len(keys) + 1)
            extra = [index for index in range(len(variant)) if variant[:index] + variant[index + 1:] == keys]
            self.assertTrue(extra)
            self.assertIn(variant[extra[0]], letters)                     # a letter key of the layout typed in

    def test_no_variant_of_a_short_word_a_non_word_or_an_authored_case(self) -> None:
        self.assertIsNone(corpus.extra_key_variant("ru", corpus.to_keys("дом")))
        self.assertIsNone(corpus.extra_key_variant("ru", corpus.to_keys("из-за")))
        for word in corpus.AUTHORED_WORDS:
            self.assertIsNone(corpus.extra_key_variant(corpus.script_of(word) or "en", corpus.to_keys(word)))

    def test_synthetic_positives_come_from_train_words_typed_in_the_other_layout(self) -> None:
        variants = [keys for keys in self.words() if corpus.extra_key_variant("ru", keys)]
        rows = [row(TRAIN, "ru", variants[0], lower=1, initial=1), row(TEST, "ru", variants[1], lower=1)]
        dictionary: dict[str, frozenset[str]] = {"en": frozenset(), "ru": frozenset()}
        labels = trainer.synthetic_population(rows, dictionary, POPULATION_FIXTURE_MINIMUM_LENGTH)
        typed = cast(str, corpus.extra_key_variant("ru", variants[0]))
        self.assertEqual(labels, {("en", typed, "lower"): False, ("en", typed, "initial"): False})

    def test_the_feature_fit_sees_the_synthetic_positives_once(self) -> None:
        rows = [row(TRAIN, "en", "hello", lower=1)]
        lexicon = IdentifierLexicon(frozenset({"htop"}), "fixture", "identifiers-fixture")
        real = {("ru", "htop", "lower"): False, ("en", "htop", "lower"): True}
        synthetic = {("en", "htop", "lower"): False, ("en", "ahtop", "lower"): False}
        with patch.object(trainer, "population", return_value=real), \
                patch.object(trainer, "synthetic_population", return_value=synthetic), \
                patch.object(trainer, "char_payload", return_value={}), \
                patch.object(trainer, "char_model", return_value=StubModel()), \
                patch.object(trainer, "logistic", return_value=[1.0] + [0.0] * len(FEATURES)) as fit:
            trainer.feature_weights(rows, settings(), {"en": frozenset(), "ru": frozenset()}, lexicon)
        english = [call.args for call in fit.call_args_list][0]
        # Per fold: the real negative `htop`, then the synthetic positive `ahtop`; the synthetic `htop` is a real type.
        self.assertEqual(english[1], [0, 1] * POPULATION_FIXTURE_FOLDS)


class DictionaryEvidenceTests(unittest.TestCase):
    def test_the_verdict_asks_about_every_word_a_feature_needs(self) -> None:
        asked = known.queried("z-nj", "en")
        self.assertEqual(asked["en"], {"z-nj", "z", "nj"})
        self.assertEqual(asked["ru"], {"я", "то"})
        dotted = known.queried(".dist", "ru")
        self.assertEqual(dotted["en"], {"dist"})
        self.assertEqual(dotted["ru"], {"ювшые"})

    def test_the_verdict_asks_about_the_target_reading_without_each_letter(self) -> None:
        asked = known.queried("aghbdtn", "en")
        self.assertIn("фпривет", asked["ru"])
        self.assertIn("привет", asked["ru"])                          # without the stray key
        self.assertEqual(len({form for form in asked["ru"] if len(form) == len("привет")}), len("фпривет"))
        self.assertEqual(known.queried("the", "en")["ru"], {"еру"})     # a short reading: only the replacement itself

    def test_the_verdict_asks_about_every_reading_of_a_stretch(self) -> None:
        asked = known.queried("innner", "ru")
        self.assertTrue({"штттук", "шттук", "штук"} <= asked["ru"])     # as typed, cut to two, read once
        self.assertTrue({"innner", "inner", "iner"} <= asked["en"])
        self.assertIn("nner", asked["en"])                              # `inner` without a letter: the cut reading's features
        self.assertEqual(known.queried("htop", "ru"), {"en": {"htop"}, "ru": {"рещз"}})   # no run: token and replacement
        hyphen = known.queried(corpus.to_keys("ти-ише"), "ru")
        self.assertIn("тише", hyphen["ru"])                              # a hyphen stretch read without it

    def test_the_verdict_round_trips_and_stores_known_forms_only(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "known.json.gz"
            digest = known.write({"en": ["hello"], "ru": ["привет"]}, path)
            self.assertEqual(digest, known.checksum(path))
            self.assertEqual(known.load(path), {"en": frozenset({"hello"}), "ru": frozenset({"привет"})})


if __name__ == "__main__":
    unittest.main()
