"""Focused branch tests for persistence, dictionaries and desktop helpers."""

from __future__ import annotations

import json
import os
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, patch

from keyswitch import config, history, language_model, learning, spellcheck, system
from keyswitch.config import DEFAULTS, SettingsStore, _deep_merge
from keyswitch.history import HistoryEntry, HistoryStore
from keyswitch.language_model import LanguageModel
from keyswitch.learning import LearningStore
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
    def test_deep_merge_replaces_scalars_and_merges_nested_values(self) -> None:
        merged = _deep_merge({"a": {"b": 1, "c": 2}, "d": 3}, {"a": {"b": 4}, "d": {"e": 5}})
        self.assertEqual(merged, {"a": {"b": 4, "c": 2}, "d": {"e": 5}})
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
            store.set("temporary.child", [1, 2])
            self.assertEqual(store.get("temporary.child"), [1, 2])
            self.assertEqual(store.get("missing.path", "fallback"), "fallback")

            snapshot = store.snapshot()
            temporary_value = snapshot["temporary"]
            self.assertIsInstance(temporary_value, dict)
            assert isinstance(temporary_value, dict)
            child_value = temporary_value["child"]
            self.assertIsInstance(child_value, list)
            assert isinstance(child_value, list)
            child_value.append(3)
            self.assertEqual(store.get("temporary.child"), [1, 2])

            before = len(calls)
            store.set("temporary.child", [1, 2])
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
    def test_entry_rounding_trimming_callbacks_and_invalid_rows(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "history.jsonl"
            store = HistoryStore(path, limit=2)
            callbacks: list[bool] = []
            store.subscribe(lambda: callbacks.append(True))
            entries = [
                HistoryEntry.create(f"bad-{index}", f"good-{index}", "editor", index + 0.126)
                for index in range(3)
            ]
            for entry in entries:
                store.append(entry)
            self.assertEqual([item.original for item in store.read()], ["bad-1", "bad-2"])
            self.assertEqual(store.read(1)[0].confidence, 2.13)
            self.assertEqual(len(callbacks), 3)

            with path.open("a", encoding="utf-8") as handle:
                handle.write("not json\n")
                handle.write(json.dumps({"unexpected": True}) + "\n")
            self.assertEqual(len(store.read()), 2)
            store.clear()
            self.assertEqual(store.read(), [])
            self.assertEqual(len(callbacks), 4)
            store.clear()
            self.assertEqual(len(callbacks), 5)
            path.unlink()
            store.clear()
            self.assertEqual(len(callbacks), 6)

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
            self.assertEqual(store.record_manual(0, "Word", 1), 1)
            self.assertEqual(store.record_manual(0, "word", 1), 2)
            self.assertEqual(store.forced_target(0, "word", 3), None)
            self.assertEqual(store.forced_target(0, "word", 2), 1)

            self.assertEqual(store.record_manual(0, "word", 2), 1)
            self.assertEqual(store.forced_target(0, "word", 1), 2)
            store.reject(0, "word", 2)
            self.assertEqual(store.forced_target(0, "word", 1), None)
            self.assertEqual(store.rejected_targets(0, "word"), {2})
            self.assertEqual(store.counts(), (0, 1))

            store.reject(0, "", 1)
            store.reject(0, "word", 0)
            store._data["rejections"]["0:invalid"] = ["1", None, "bad"]
            store._data["rejections"]["0:not-list"] = "bad"
            self.assertEqual(store.rejected_targets(0, "invalid"), {1})
            self.assertEqual(store.rejected_targets(0, "not-list"), set())

            store._data["rules"]["0:broken"] = {"confirmations": 2}
            self.assertIsNone(store.forced_target(0, "broken"))
            store._data["rules"]["0:scalar"] = 5
            self.assertIsNone(store.forced_target(0, "scalar"))
            store._data["rules"]["0:low"] = {"target_group": 1, "confirmations": 0}
            self.assertIsNone(store.forced_target(0, "low", 0))

            store._data["rejections"]["0:word"] = "bad"
            store.reject(0, "word", 1)
            self.assertEqual(store.rejected_targets(0, "word"), {1})
            store.reject(0, "word", 2)
            store.record_manual(0, "word", 1)
            self.assertEqual(store.rejected_targets(0, "word"), {2})
            store.reject(0, "solo", 1)
            store.record_manual(0, "solo", 1)
            self.assertEqual(store.rejected_targets(0, "solo"), set())
            store.clear()
            self.assertEqual(store.counts(), (0, 0))


class LanguageModelBranchTests(unittest.TestCase):
    def _model(
        self,
        frequencies: dict[str, int] | None = None,
        bigrams: dict[tuple[str, str], int] | None = None,
    ) -> LanguageModel:
        fake_speller = SimpleNamespace(available=False, source="", check=lambda _word: False)
        with patch("keyswitch.language_model.HunspellDictionary", return_value=fake_speller):
            return LanguageModel("en_US", frequencies or {"hello": 100, "world": 50}, "test", bigrams)

    def tearDown(self) -> None:
        LanguageModel._load_cached.cache_clear()

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
            self.assertEqual(unigrams, {"hello": 15})
            self.assertEqual(bigrams, {("hello", "world"): 7})
            self.assertEqual(LanguageModel._read_arpa_unigrams(path), unigrams)
            self.assertEqual(LanguageModel._read_arpa(Path(temporary) / "absent"), ({}, {}))

    def test_gram_building_scores_context_and_typo_candidates(self) -> None:
        counts = LanguageModel._build_gram_counts({"a": 1, "can't": 10, "hello": 2**40})
        self.assertTrue(counts[3])
        self.assertEqual(LanguageModel._build_grams({"hello": 2}), set(counts[3]))
        model = self._model({"hello": 100, "help": 20}, {("hello", "help"): 10})
        self.assertEqual(model._raw_ngram_score("!!!"), -30.0)
        self.assertEqual(model.context_score("missing", "help"), 0.0)
        self.assertGreater(model.context_score("hello", "help"), 0.0)
        self.assertFalse(model.score("").known)
        self.assertTrue(model.score("hello").exact)
        self.assertFalse(model.score("hello!").exact)
        self.assertFalse(model.best_single_deletion("abc").known)
        self.assertEqual(model.best_single_deletion("h" * 30, limit=3).__class__.__name__, "WordScore")

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
            patch("keyswitch.language_model.statistics.pstdev", return_value=0.01),
        ):
            clamped = LanguageModel("en_US", {"alpha": 2, "bravo": 2}, "lexicon")
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
    def setUp(self) -> None:
        self.original_library = HunspellDictionary._library
        self.original_attempted = HunspellDictionary._library_attempted
        HunspellDictionary._library = None
        HunspellDictionary._library_attempted = False

    def tearDown(self) -> None:
        HunspellDictionary._library = self.original_library
        HunspellDictionary._library_attempted = self.original_attempted

    @staticmethod
    def _fake_library(encoding: bytes = b"UTF-8", handle: int = 123) -> Mock:
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
                self.assertEqual(roots[:2], (first, second))
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
            self.assertFalse(instance.check("x" * 129))
            instance._encoding = "ascii"
            self.assertFalse(instance.check("привет"))
            instance._encoding = "utf-8"
            HunspellDictionary._library = None
            self.assertFalse(instance.check("hello"))
            HunspellDictionary._library = library
            instance.close()
            instance.close()
            library.Hunspell_destroy.assert_called_once_with(123)

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
