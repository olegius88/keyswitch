"""The shared engine models a test module may use are the models the engine would load."""

from __future__ import annotations

import tempfile
import unittest
from collections.abc import Callable
from pathlib import Path
from unittest.mock import patch

import engine_models
from keyswitch.context_model import ARTIFACT_PATH, ContextModel
from keyswitch.language_model import LanguageModel
from keyswitch.lexicon_supplement import supplement_words


class SharedEngineModelsTests(unittest.TestCase):
    def test_a_language_with_its_packaged_supplement_is_the_model_the_cache_returns(self) -> None:
        for locale in ("en_US", "ru_RU"):
            with self.subTest(locale=locale):
                shared = engine_models.shared_language(locale, supplement_words(locale))
                self.assertIs(shared, LanguageModel.load(locale, supplement_words(locale)))
                self.assertIs(engine_models.shared_language(locale, supplement_words(locale)), shared)

    def test_any_other_language_load_is_the_real_one(self) -> None:
        sentinel = object()
        with patch.object(engine_models, "_load_language", return_value=sentinel) as load:
            self.assertIs(engine_models.shared_language("ru_RU", ("слово",)), sentinel)
            self.assertIs(engine_models.shared_language("en_US", ["word"]), sentinel)
        self.assertEqual([call.args for call in load.call_args_list], [("ru_RU", ("слово",)), ("en_US", ["word"])])

    def test_the_installed_context_model_is_a_new_model_of_the_same_weights_each_time(self) -> None:
        installed, version = ContextModel.try_load()
        assert installed is not None
        first, first_version = engine_models.shared_context()
        second, _second_version = engine_models.shared_context()
        assert first is not None and second is not None
        self.assertIsNot(first, second)
        self.assertIsNot(first.weights, second.weights)
        self.assertEqual((first_version, first.version), (version, installed.version))
        self.assertEqual(first.weights, installed.weights)
        self.assertEqual((first.conversion_threshold, first.feature_version, first.answers_letters),
                         (installed.conversion_threshold, installed.feature_version, installed.answers_letters))

    def test_another_artifact_and_a_missing_model_load_as_they_would(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            missing = Path(directory) / "context.json"
            model, status = engine_models.shared_context(missing)
            self.assertIsNone(model)
            self.assertEqual(status, ContextModel.try_load(missing)[1])
        with patch.object(engine_models, "_context", return_value=(None, "context model is too large")):
            self.assertEqual(engine_models.shared_context(), (None, "context model is too large"))
        self.assertEqual(engine_models.shared_context(ARTIFACT_PATH)[1], ContextModel.try_load()[1])

    def test_a_module_that_shares_them_gives_its_engines_the_shared_loaders_until_it_ends(self) -> None:
        cleanups: list[Callable[[], object]] = []
        with patch.object(unittest, "addModuleCleanup", side_effect=cleanups.append):
            engine_models.share_engine_models()
        try:
            self.assertIs(LanguageModel.load("ru_RU", supplement_words("ru_RU")),
                          engine_models.shared_language("ru_RU", supplement_words("ru_RU")))
            self.assertIsNotNone(ContextModel.try_load()[0])
        finally:
            for cleanup in reversed(cleanups):
                cleanup()
        # Stopped with the module: the real loaders are back.
        self.assertIsInstance(LanguageModel.__dict__["load"], classmethod)
        self.assertIsInstance(ContextModel.__dict__["try_load"], classmethod)


if __name__ == "__main__":
    unittest.main()
