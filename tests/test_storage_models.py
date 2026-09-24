"""Focused branch tests for persistence, dictionaries and desktop helpers."""

from __future__ import annotations

import json
import os
import tempfile
import unittest
from collections import Counter
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, patch

from keyswitch import config, history, language_model, learning, spellcheck, system
from keyswitch.config import DEFAULTS, SettingsStore, _deep_merge
from keyswitch.history import HISTORY_CONFIDENCE_DECIMALS, HistoryEntry, HistoryStore
from keyswitch.language_model import LanguageModel
from keyswitch.learning import MAX_CONFIRMATIONS, LearningStore
from keyswitch.spellcheck import HunspellDictionary
from keyswitch.system import AutostartManager


class EnvironmentPathTests(unittest.TestCase):
    def test_configuration_and_data_paths_honor_overrides_and_xdg(self) -> None:
        with patch.dict(os.environ, {"KEYSWITCH_CONFIG_DIR": "/tmp/key-conf"}, clear=True):
            self.assertEqual(config.config_dir(), Path("/tmp/key-conf"))
        with patch.dict(os.environ, {"XDG_CONFIG_HOME": "/tmp/xdg-conf"}, clear=True):
            self.assertEqual(config.config_dir(), Path("/tmp/xdg-conf/keyswitch"))
            self.assertEqual(system.autostart_path(), Path("/tmp/xdg-conf/autostart/io.github.olegius88.KeySwitch.desktop"))
        with patch.dict(os.environ, {"KEYSWITCH_DATA_DIR": "/tmp/key-data"}, clear=True):
            self.assertEqual(history.data_dir(), Path("/tmp/key-data"))
        with patch.dict(os.environ, {"XDG_DATA_HOME": "/tmp/xdg-data"}, clear=True):
            self.assertEqual(history.data_dir(), Path("/tmp/xdg-data/keyswitch"))


class SettingsStoreBranchTests(unittest.TestCase):
    BASE_CONFIG: dict[str, object] = {"a": {"b": 1, "c": 2}, "d": 3}
    OVERRIDE_CONFIG: dict[str, object] = {"a": {"b": 4}, "d": {"e": 5}}
    MERGED_CONFIG = {"a": {"b": 4, "c": 2}, "d": {"e": 5}}
    CHILD_LIST = [1, 2]
    APPENDED_VALUE = 3

    def test_deep_merge_replaces_scalars_and_merges_nested_values(self) -> None:
        merged = _deep_merge(self.BASE_CONFIG, self.OVERRIDE_CONFIG)
        self.assertEqual(merged, self.MERGED_CONFIG)
        self.assertIsNone(config._string_keyed_mapping({1: "invalid key"}))

    def test_malformed_and_non_mapping_files_keep_defaults(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "config.json"
            for payload in ("{broken", "[]"):
                path.write_text(payload, encoding="utf-8")
                store = SettingsStore(path)
                self.assertEqual(store.get("schema_version"), DEFAULTS["schema_version"])

    def test_get_set_callbacks_snapshot_and_reset_cover_edge_cases(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "config.json"
            store = SettingsStore(path)
            calls: list[tuple[str, object]] = []
            unsubscribe = store.subscribe(lambda key, value: calls.append((key, value)))

            store.set("temporary", 1, persist=False)
            self.assertFalse(path.exists())
            store.set("temporary.child", self.CHILD_LIST)
            self.assertEqual(store.get("temporary.child"), self.CHILD_LIST)
            self.assertEqual(store.get("missing.path", "fallback"), "fallback")

            snapshot = store.snapshot()
            temporary_value = snapshot["temporary"]
            self.assertIsInstance(temporary_value, dict)
            assert isinstance(temporary_value, dict)
            child_value = temporary_value["child"]
            self.assertIsInstance(child_value, list)
            assert isinstance(child_value, list)
            child_value.append(self.APPENDED_VALUE)
            self.assertEqual(store.get("temporary.child"), self.CHILD_LIST)

            before = len(calls)
            store.set("temporary.child", self.CHILD_LIST)
            self.assertEqual(len(calls), before)
            unsubscribe()
            unsubscribe()
            store.set("enabled", False)
            self.assertEqual(len(calls), before)

            reset_calls: list[tuple[str, object]] = []
            store.subscribe(lambda key, value: reset_calls.append((key, value)))
            store.reset()
            self.assertEqual(reset_calls[0][0], "*")
            self.assertEqual(store.snapshot(), DEFAULTS)
            self.assertEqual(json.loads(path.read_text(encoding="utf-8")), DEFAULTS)


class HistoryStoreBranchTests(unittest.TestCase):
    STORE_LIMIT = 2
    ENTRY_COUNT = 3
    CONFIDENCE_OFFSET = 0.126

    def test_entry_rounding_trimming_callbacks_and_invalid_rows(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "history.jsonl"
            store = HistoryStore(path, limit=self.STORE_LIMIT)
            callbacks: list[bool] = []
            store.subscribe(lambda: callbacks.append(True))
            entries = [
                HistoryEntry.create(f"bad-{index}", f"good-{index}", "editor", index + self.CONFIDENCE_OFFSET)
                for index in range(self.ENTRY_COUNT)
            ]
            for entry in entries:
                store.append(entry)
            self.assertEqual([item.original for item in store.read()], ["bad-1", "bad-2"])
            self.assertEqual(store.read(1)[0].confidence,
                             round((self.ENTRY_COUNT - 1) + self.CONFIDENCE_OFFSET, HISTORY_CONFIDENCE_DECIMALS))
            expected_callbacks = self.ENTRY_COUNT
            self.assertEqual(len(callbacks), expected_callbacks)

            with path.open("a", encoding="utf-8") as handle:
                handle.write("not json\n")
                handle.write(json.dumps({"unexpected": True}) + "\n")
            self.assertEqual(len(store.read()), self.STORE_LIMIT)
            store.clear()
            self.assertEqual(store.read(), [])
            expected_callbacks += 1
            self.assertEqual(len(callbacks), expected_callbacks)
            store.clear()
            expected_callbacks += 1
            self.assertEqual(len(callbacks), expected_callbacks)
            path.unlink()
            store.clear()
            expected_callbacks += 1
            self.assertEqual(len(callbacks), expected_callbacks)

    def test_missing_file_read_error_and_minimum_limit(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "missing.jsonl"
            store = HistoryStore(path, limit=0)
            self.assertEqual(store.limit, 1)
            self.assertEqual(store.read(), [])
            path.write_text("{}\n", encoding="utf-8")
            with patch.object(Path, "read_text", side_effect=OSError("denied")):
                self.assertEqual(store.read(), [])


class LearningStoreBranchTests(unittest.TestCase):
    ALTERNATE_TARGET_GROUP = 2
    FIXTURE_CONFIRMATIONS = 2
    SCALAR_RULE_VALUE = 5
    CONFIRMATIONS_REQUIRED = 2
    FULL_CONFIRMATIONS = 5
    MALFORMED_RULE_VALUE = 9

    def test_invalid_persisted_shapes_are_ignored(self) -> None:
        self.assertIsNone(learning._string_keyed_dict({1: "invalid key"}))
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "learning.json"
            for payload in ("{broken", "[]", '{"rules": [], "rejections": {}}'):
                path.write_text(payload, encoding="utf-8")
                store = LearningStore(path)
                self.assertEqual(store.counts(), (0, 0))

    def test_manual_rules_rejections_invalid_values_and_clear(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "learning.json"
            store = LearningStore(path)
            self.assertEqual(store.record_manual(0, "   ", 1), 0)
            self.assertEqual(store.record_manual(0, "word", 0), 0)
            first_confirmation = store.record_manual(0, "Word", 1)
            self.assertEqual(first_confirmation, 1)
            second_confirmation = store.record_manual(0, "word", 1)
            self.assertEqual(second_confirmation, first_confirmation + 1)
            self.assertEqual(store.forced_target(0, "word", second_confirmation + 1), None)
            self.assertEqual(store.forced_target(0, "word", second_confirmation), 1)

            self.assertEqual(store.record_manual(0, "word", self.ALTERNATE_TARGET_GROUP), 1)
            self.assertEqual(store.forced_target(0, "word", 1), self.ALTERNATE_TARGET_GROUP)
            store.reject(0, "word", self.ALTERNATE_TARGET_GROUP)
            self.assertEqual(store.forced_target(0, "word", 1), None)
            self.assertEqual(store.rejected_targets(0, "word"), {self.ALTERNATE_TARGET_GROUP})
            self.assertEqual(store.counts(), (0, 1))

            store.reject(0, "", 1)
            store.reject(0, "word", 0)
            store._data["rejections"]["0:invalid"] = ["1", None, "bad"]
            store._data["rejections"]["0:not-list"] = "bad"
            self.assertEqual(store.rejected_targets(0, "invalid"), {1})
            self.assertEqual(store.rejected_targets(0, "not-list"), set())

            store._data["rules"]["0:broken"] = {"confirmations": self.FIXTURE_CONFIRMATIONS}
            self.assertIsNone(store.forced_target(0, "broken"))
            store._data["rules"]["0:scalar"] = self.SCALAR_RULE_VALUE
            self.assertIsNone(store.forced_target(0, "scalar"))
            store._data["rules"]["0:low"] = {"target_group": 1, "confirmations": 0}
            self.assertIsNone(store.forced_target(0, "low", 0))

            store._data["rejections"]["0:word"] = "bad"
            store.reject(0, "word", 1)
            self.assertEqual(store.rejected_targets(0, "word"), {1})
            store.reject(0, "word", self.ALTERNATE_TARGET_GROUP)
            store.record_manual(0, "word", 1)
            self.assertEqual(store.rejected_targets(0, "word"), {self.ALTERNATE_TARGET_GROUP})
            store.reject(0, "solo", 1)
            store.record_manual(0, "solo", 1)
            self.assertEqual(store.rejected_targets(0, "solo"), set())
            store.clear()
            self.assertEqual(store.counts(), (0, 0))

    def test_explicit_confirmation_activates_and_reconciles_rule(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            store = LearningStore(Path(temporary) / "learning.json")
            self.assertEqual(store.confirm_manual(0, "", 1, self.CONFIRMATIONS_REQUIRED), 0)
            self.assertEqual(store.confirm_manual(0, "word", 0, self.CONFIRMATIONS_REQUIRED), 0)

            store.reject(0, "word", 1)
            store.reject(0, "word", self.ALTERNATE_TARGET_GROUP)
            self.assertEqual(store.confirm_manual(0, "word", 1, self.FULL_CONFIRMATIONS), self.FULL_CONFIRMATIONS)
            self.assertEqual(store.forced_target(0, "WORD", self.FULL_CONFIRMATIONS), 1)
            self.assertEqual(store.rejected_targets(0, "word"), {self.ALTERNATE_TARGET_GROUP})

            self.assertEqual(store.confirm_manual(0, "word", 1, self.CONFIRMATIONS_REQUIRED), self.FULL_CONFIRMATIONS)
            store.reject(0, "solo", 1)
            self.assertEqual(store.confirm_manual(0, "solo", 1, 0), 1)
            self.assertEqual(store.rejected_targets(0, "solo"), set())

            store._data["rules"]["0:reset"] = self.MALFORMED_RULE_VALUE
            store._data["rejections"]["0:reset"] = "invalid"
            self.assertEqual(store.confirm_manual(0, "reset", 1, MAX_CONFIRMATIONS + 1), MAX_CONFIRMATIONS)
            self.assertEqual(store.forced_target(0, "reset", MAX_CONFIRMATIONS), 1)


class LanguageModelBranchTests(unittest.TestCase):
    HIGH_FREQUENCY = 100
    LOW_FREQUENCY = 20
    LOWEST_FREQUENCY = 10
    MODEL_HELPER_WORLD_FREQUENCY = 50
    ALPHABET_SIZE = 26
    CALIBRATION_WORD_LIMIT = 12_000
    SCORE_CACHE_MAXSIZE = 65_536
    EXPECTED_HELLO_UNIGRAM_COUNT = 15
    EXPECTED_HELLO_WORLD_BIGRAM_COUNT = 7
    UNIFORM_WORD_FREQUENCY = 7
    OVERFLOW_TRIGRAM_FREQUENCY = 2**40
    BUILT_GRAM_FREQUENCY = 10
    SINGLE_WORD_FREQUENCY = 2
    PUNCTUATION_ONLY_SCORE = -30.0
    NEAR_ZERO_DEVIATION = 0.01
    BEST_DELETION_WORD_LENGTH = 30
    BEST_DELETION_LIMIT = 3
    TRIGRAM_ORDER = 3

    def _model(
        self,
        frequencies: dict[str, int] | None = None,
        bigrams: dict[tuple[str, str], int] | None = None,
    ) -> LanguageModel:
        fake_speller = SimpleNamespace(available=False, source="", check=lambda _word: False)
        with patch("keyswitch.language_model.HunspellDictionary", return_value=fake_speller):
            return LanguageModel("en_US", frequencies or {"hello": self.HIGH_FREQUENCY, "world": self.MODEL_HELPER_WORLD_FREQUENCY}, "test", bigrams)

    def tearDown(self) -> None:
        LanguageModel._load_cached.cache_clear()
        LanguageModel.score.cache_clear()

    def test_disabled_spellcheck_is_host_independent_and_matches_noop_speller(
        self,
    ) -> None:
        frequencies = {"hello": self.HIGH_FREQUENCY, "help": self.LOW_FREQUENCY, "world": self.LOWEST_FREQUENCY}
        fake_speller = SimpleNamespace(
            available=False,
            source="",
            check=lambda _word: False,
        )
        with patch(
            "keyswitch.language_model.HunspellDictionary",
            return_value=fake_speller,
        ):
            ordinary = LanguageModel("en_US", frequencies, "sealed")

        with patch(
            "keyswitch.language_model.HunspellDictionary"
        ) as hunspell_factory:
            deterministic = LanguageModel(
                "en_US",
                dict(reversed(tuple(frequencies.items()))),
                "sealed",
                enable_spellcheck=False,
            )
        hunspell_factory.assert_not_called()
        self.assertFalse(deterministic.speller.available)
        self.assertEqual(deterministic.speller.source, "")
        self.assertEqual(deterministic.source, "sealed")

        for token in ("hello", "help", "unknown", "hello!", "!!!"):
            with self.subTest(token=token):
                self.assertEqual(deterministic.score(token), ordinary.score(token))
                self.assertFalse(deterministic.score(token).spell_known)

    def test_equal_frequency_calibration_cutoff_has_canonical_tie_order(self) -> None:
        def alpha_word(index: int) -> str:
            return "w" + "".join(
                chr(ord("a") + (index // divisor) % self.ALPHABET_SIZE)
                for divisor in (self.ALPHABET_SIZE * self.ALPHABET_SIZE, self.ALPHABET_SIZE, 1)
            )

        words = tuple(alpha_word(index) for index in range(self.CALIBRATION_WORD_LIMIT + 1))
        expected = tuple(sorted(words)[:self.CALIBRATION_WORD_LIMIT])
        observed: list[str] = []

        def record_raw_score(_model: LanguageModel, word: str) -> float:
            observed.append(word)
            return float(len(word))

        empty_counts = {
            order: Counter[str]() for order in LanguageModel.NGRAM_ORDERS
        }
        with (
            patch.object(
                LanguageModel,
                "_build_gram_counts",
                return_value=empty_counts,
            ),
            patch.object(
                LanguageModel,
                "_raw_ngram_score",
                new=record_raw_score,
            ),
        ):
            forward = LanguageModel(
                "en_US",
                {word: self.UNIFORM_WORD_FREQUENCY for word in words},
                "sealed",
                enable_spellcheck=False,
            )
            forward_selection = tuple(observed)
            observed.clear()
            reverse = LanguageModel(
                "en_US",
                {word: self.UNIFORM_WORD_FREQUENCY for word in reversed(words)},
                "sealed",
                enable_spellcheck=False,
            )
            reverse_selection = tuple(observed)

        self.assertEqual(forward_selection, expected)
        self.assertEqual(reverse_selection, expected)
        self.assertEqual(forward._ngram_mean, reverse._ngram_mean)
        self.assertEqual(forward._ngram_deviation, reverse._ngram_deviation)

    def test_score_cache_is_bounded_and_recomputation_has_exact_parity(self) -> None:
        LanguageModel.score.cache_clear()
        model = LanguageModel(
            "en_US",
            {"hello": self.HIGH_FREQUENCY, "help": self.LOW_FREQUENCY},
            "sealed",
            enable_spellcheck=False,
        )
        with patch.object(
            model,
            "ngram_score",
            wraps=model.ngram_score,
        ) as ngram_score:
            first = model.score("hello")
            second = model.score("hello")
            self.assertIs(second, first)
            expected_calls = 1
            self.assertEqual(ngram_score.call_count, expected_calls)

            cache = LanguageModel.score.cache_info()
            self.assertEqual(cache.maxsize, self.SCORE_CACHE_MAXSIZE)
            self.assertLessEqual(cache.currsize, self.SCORE_CACHE_MAXSIZE)
            self.assertEqual((cache.hits, cache.misses), (1, 1))

            LanguageModel.score.cache_clear()
            recomputed = model.score("hello")
            self.assertEqual(recomputed, first)
            expected_calls += 1
            self.assertEqual(ngram_score.call_count, expected_calls)

    def test_normalization_arpa_parsing_and_read_failure(self) -> None:
        self.assertEqual(LanguageModel.normalize(" HeL-lo_42! "), "hel-lo")
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "tiny.lm"
            path.write_text(
                "\\data\\\nignored\n\\1-grams:\n10 hello\nbad world\n3 <s>\n4 a\n5 hello\n"
                "\\2-grams:\n7 hello world\nwrong pair\n2 hello\n3 <s> world\n4 !!! world\n\\end\\\n",
                encoding="utf-8",
            )
            unigrams, bigrams = LanguageModel._read_arpa(path)
            self.assertEqual(unigrams, {"hello": self.EXPECTED_HELLO_UNIGRAM_COUNT})
            self.assertEqual(bigrams, {("hello", "world"): self.EXPECTED_HELLO_WORLD_BIGRAM_COUNT})
            self.assertEqual(LanguageModel._read_arpa_unigrams(path), unigrams)
            self.assertEqual(LanguageModel._read_arpa(Path(temporary) / "absent"), ({}, {}))

    def test_gram_building_scores_context_and_typo_candidates(self) -> None:
        counts = LanguageModel._build_gram_counts({"a": 1, "can't": self.BUILT_GRAM_FREQUENCY, "hello": self.OVERFLOW_TRIGRAM_FREQUENCY})
        self.assertTrue(counts[self.TRIGRAM_ORDER])
        self.assertEqual(LanguageModel._build_grams({"hello": self.SINGLE_WORD_FREQUENCY}), set(counts[self.TRIGRAM_ORDER]))
        model = self._model({"hello": self.HIGH_FREQUENCY, "help": self.LOW_FREQUENCY}, {("hello", "help"): self.BUILT_GRAM_FREQUENCY})
        self.assertEqual(model._raw_ngram_score("!!!"), self.PUNCTUATION_ONLY_SCORE)
        self.assertEqual(model.context_score("missing", "help"), 0.0)
        self.assertGreater(model.context_score("hello", "help"), 0.0)
        self.assertFalse(model.score("").known)
        self.assertTrue(model.score("hello").exact)
        self.assertFalse(model.score("hello!").exact)
        self.assertFalse(model.best_single_deletion("abc").known)
        self.assertEqual(model.best_single_deletion("h" * self.BEST_DELETION_WORD_LENGTH, limit=self.BEST_DELETION_LIMIT).__class__.__name__, "WordScore")

    def test_spell_only_source_and_nearly_constant_calibration(self) -> None:
        fake_speller = SimpleNamespace(available=True, source="dictionary.dic", check=lambda word: word == "morph")
        with patch("keyswitch.language_model.HunspellDictionary", return_value=fake_speller):
            model = LanguageModel("en_US", {"same": 1}, "lexicon")
        self.assertIn("Hunspell", model.source)
        score = model.score("morph")
        self.assertTrue(score.spell_known)
        self.assertTrue(score.known)

        with (
            patch("keyswitch.language_model.HunspellDictionary", return_value=fake_speller),
            patch("keyswitch.language_model.statistics.pstdev", return_value=self.NEAR_ZERO_DEVIATION),
        ):
            clamped = LanguageModel("en_US", {"alpha": self.SINGLE_WORD_FREQUENCY, "bravo": self.SINGLE_WORD_FREQUENCY}, "lexicon")
        self.assertEqual(clamped._ngram_deviation, 1.0)

    def test_load_uses_arpa_fallbacks_extras_and_cache(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "xx_XX.lm").write_text("\\1-grams:\n9 native\n\\end\\\n", encoding="utf-8")
            fake_speller = SimpleNamespace(available=False, source="", check=lambda _word: False)
            with (
                patch.object(language_model, "MODEL_ROOTS", (root,)),
                patch.dict(
                    os.environ,
                    {"KEYSWITCH_MODEL_PATH": str(root / "missing")},
                    clear=True,
                ),
                patch.object(language_model, "LOCALE_FALLBACKS", {"xx_XX": ("fallback",)}),
                patch("keyswitch.language_model.HunspellDictionary", return_value=fake_speller),
            ):
                LanguageModel._load_cached.cache_clear()
                first = LanguageModel.load("xx_XX", ["Extra", "extra", ""])
                second = LanguageModel.load("xx_XX", ["extra"])
                self.assertIs(first, second)
                self.assertIn("native", first.frequencies)
                self.assertIn("fallback", first.frequencies)
                self.assertIn("extra", first.frequencies)
            with (
                patch.object(language_model, "MODEL_ROOTS", (root / "missing",)),
                patch.object(language_model, "LOCALE_FALLBACKS", {}),
                patch("keyswitch.language_model.HunspellDictionary", return_value=fake_speller),
            ):
                LanguageModel._load_cached.cache_clear()
                empty = LanguageModel._load_cached("zz_ZZ", ("123",))
            self.assertEqual(empty.frequencies, {})


class HunspellBranchTests(unittest.TestCase):
    FAKE_HANDLE = 123
    MAX_CHECK_LENGTH = 128

    def setUp(self) -> None:
        self.original_library = HunspellDictionary._library
        self.original_attempted = HunspellDictionary._library_attempted
        HunspellDictionary._library = None
        HunspellDictionary._library_attempted = False

    def tearDown(self) -> None:
        HunspellDictionary._library = self.original_library
        HunspellDictionary._library_attempted = self.original_attempted

    @staticmethod
    def _fake_library(encoding: bytes = b"UTF-8", handle: int = FAKE_HANDLE) -> Mock:
        library = Mock()
        library.Hunspell_create.return_value = handle
        library.Hunspell_get_dic_encoding.return_value = encoding
        library.Hunspell_spell.return_value = 1
        return library

    def test_dictionary_roots_and_locale_variants(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            first = Path(temporary) / "one"
            second = Path(temporary) / "two"
            second.mkdir()
            (second / "en.aff").write_text("", encoding="utf-8")
            (second / "en.dic").write_text("", encoding="utf-8")
            with patch.dict(os.environ, {"KEYSWITCH_HUNSPELL_PATH": f"{first}{os.pathsep}{second}", "XDG_DATA_HOME": temporary}, clear=True):
                roots = HunspellDictionary._dictionary_roots()
                expected_roots = (first, second)
                self.assertEqual(roots[:len(expected_roots)], expected_roots)
                with patch.object(HunspellDictionary, "_dictionary_roots", return_value=(second,)):
                    self.assertEqual(HunspellDictionary._find_dictionary("en_ZZ"), (second / "en.aff", second / "en.dic"))
                    self.assertIsNone(HunspellDictionary._find_dictionary("zz_ZZ"))

    def test_initialization_check_encoding_and_close(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            affix = Path(temporary) / "en.aff"
            dictionary = Path(temporary) / "en.dic"
            affix.touch()
            dictionary.touch()
            library = self._fake_library(b"NOT-A-CODEC")
            HunspellDictionary._library = library
            with (
                patch.object(HunspellDictionary, "_load_library", return_value=library),
                patch.object(HunspellDictionary, "_find_dictionary", return_value=(affix, dictionary)),
            ):
                instance = HunspellDictionary("en_US")
            self.assertTrue(instance.available)
            self.assertEqual(instance._encoding, "utf-8")
            self.assertTrue(instance.check(" hello "))
            self.assertFalse(instance.check(""))
            self.assertFalse(instance.check("x" * (self.MAX_CHECK_LENGTH + 1)))
            instance._encoding = "ascii"
            self.assertFalse(instance.check("привет"))
            instance._encoding = "utf-8"
            HunspellDictionary._library = None
            self.assertFalse(instance.check("hello"))
            HunspellDictionary._library = library
            instance.close()
            instance.close()
            library.Hunspell_destroy.assert_called_once_with(self.FAKE_HANDLE)

            library = self._fake_library(encoding=b"")
            HunspellDictionary._library = library
            with (
                patch.object(HunspellDictionary, "_load_library", return_value=library),
                patch.object(HunspellDictionary, "_find_dictionary", return_value=(affix, dictionary)),
            ):
                no_encoding = HunspellDictionary("en_US")
            self.assertEqual(no_encoding._encoding, "utf-8")
            no_encoding.close()

    def test_unavailable_or_failed_handle_returns_inactive_dictionary(self) -> None:
        with patch.object(HunspellDictionary, "_load_library", return_value=None):
            self.assertFalse(HunspellDictionary("zz_ZZ").available)
        library = self._fake_library(handle=0)
        with (
            patch.object(HunspellDictionary, "_load_library", return_value=library),
            patch.object(HunspellDictionary, "_find_dictionary", return_value=(Path("a"), Path("d"))),
        ):
            self.assertFalse(HunspellDictionary("en_US").available)

    def test_library_loader_is_cached_and_handles_failures(self) -> None:
        with patch("ctypes.util.find_library", return_value=None):
            self.assertIsNone(HunspellDictionary._load_library())
            self.assertIsNone(HunspellDictionary._load_library())
        HunspellDictionary._library_attempted = False
        with patch("ctypes.util.find_library", return_value="libhunspell.so"), patch("ctypes.CDLL", side_effect=OSError("bad")):
            self.assertIsNone(HunspellDictionary._load_library())
        HunspellDictionary._library_attempted = False
        library = self._fake_library()
        with patch("ctypes.util.find_library", return_value="libhunspell.so"), patch("ctypes.CDLL", return_value=library):
            self.assertIs(HunspellDictionary._load_library(), library)


class SystemHelperBranchTests(unittest.TestCase):
    def test_launcher_command_all_fallbacks(self) -> None:
        self.assertTrue(system.source_root().is_dir())
        with patch("keyswitch.system.shutil.which", return_value="/usr/bin/keyswitch"):
            self.assertEqual(system.launcher_command(), "/usr/bin/keyswitch")
        with tempfile.TemporaryDirectory() as temporary:
            launcher = Path(temporary) / "run.sh"
            launcher.touch()
            with patch("keyswitch.system.shutil.which", return_value=None), patch("keyswitch.system.source_root", return_value=Path(temporary)):
                self.assertEqual(system.launcher_command(), str(launcher))
            launcher.unlink()
            with patch("keyswitch.system.shutil.which", return_value=None), patch("keyswitch.system.source_root", return_value=Path(temporary)):
                self.assertIn("-m keyswitch", system.launcher_command())

    def test_autostart_parse_errors_hidden_flags_and_commands(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "autostart" / "keyswitch.desktop"
            manager = AutostartManager(path)
            self.assertFalse(manager.enabled())
            with patch("keyswitch.system.launcher_command", return_value="/opt/Key Switch"):
                manager.set_enabled(True, start_hidden=False)
            self.assertTrue(manager.enabled())
            self.assertIn("Exec=/opt/Key Switch\n", path.read_text(encoding="utf-8"))
            path.write_text("Hidden=true\n", encoding="utf-8")
            self.assertFalse(manager.enabled())
            path.write_text("X-GNOME-Autostart-enabled=false\n", encoding="utf-8")
            self.assertFalse(manager.enabled())
            with patch.object(Path, "read_text", side_effect=OSError("denied")):
                self.assertFalse(manager.enabled())
            with patch("keyswitch.system.launcher_command", return_value="keyswitch"):
                manager.set_enabled(True, start_hidden=True)
            self.assertIn("Exec=keyswitch --hidden", path.read_text(encoding="utf-8"))
            manager.set_enabled(False)
            self.assertFalse(path.exists())
            manager.set_enabled(False)


if __name__ == "__main__":
    unittest.main()
