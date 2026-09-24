"""Small fixtures for exact auxiliary replay pins and explicit model loading."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile
from typing import cast
import unittest
from unittest.mock import Mock, patch

TOOLS = str(Path(__file__).resolve().parents[1] / "tools")
if TOOLS not in sys.path:
    sys.path.insert(0, TOOLS)
import auxiliary_runtime_evidence as evidence
from keyswitch.intent_model import LinearNgramModel


class AuxiliaryRuntimeEvidenceTests(unittest.TestCase):
    EXPECTED_LEXICAL_SOURCE_COUNT = 6
    LOAD_CALLS_AFTER_CONTENT_CHANGE = 2
    LOAD_CALLS_AFTER_MTIME_CHANGE = 3
    MTIME_BUMP_NANOSECONDS = 1_000_000

    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix="keyswitch-auxiliary-pins-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        for name in evidence.REQUIRED_PATHS:
            self.write(name, name.encode())
        languages: dict[str, object] = {}
        dictionaries: dict[str, object] = {}
        for locale in ("en_US", "ru_RU"):
            language = "model/intent_v1/sources/" + locale + ".lm"
            languages[locale] = {"path": language, "sha256": self.write(language, locale.encode())}
            dictionaries[locale] = {
                "dictionary_sha256": self.write("model/intent_v1/sources/hunspell/" + locale + ".dic", b"dictionary" + locale.encode()),
                "affix_sha256": self.write("model/intent_v1/sources/hunspell/" + locale + ".aff", b"affix" + locale.encode()),
            }
        self.config: dict[str, object] = {"sources": {"languages": languages},
                                         "external_evaluation": {"hunspell": dictionaries}}
        self.save_config()
        self.extra = self.root / "model/prefix_v1/candidate.json"
        self.write("model/prefix_v1/candidate.json", b"candidate")
        self.write("src/keyswitch/transitive_fixture.py", b"transitive source")
        self.write("src/keyswitch/resources/models/extra-model.json", b"extra model")

    def write(self, name: str, data: bytes) -> str:
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return hashlib.sha256(data).hexdigest()

    def save_config(self) -> None:
        self.write(evidence.CONFIG_PATH, json.dumps(self.config).encode())

    def test_pins_transitive_source_models_helpers_editor_and_extras(self) -> None:
        before = evidence.runtime_provenance(self.root, [self.extra])
        expected = set(evidence.REQUIRED_PATHS) | {
            "src/keyswitch/transitive_fixture.py", "src/keyswitch/resources/models/extra-model.json",
            "model/prefix_v1/candidate.json",
            *("model/intent_v1/sources/" + locale + ".lm" for locale in ("en_US", "ru_RU")),
            *("model/intent_v1/sources/hunspell/" + locale + extension
              for locale in ("en_US", "ru_RU") for extension in (".dic", ".aff")),
        }
        self.assertEqual(set(before), expected)
        for name in sorted(expected - {evidence.CONFIG_PATH}):
            if name.startswith("model/intent_v1/sources/"):
                continue
            with self.subTest(name=name):
                path = self.root / name
                original = path.read_bytes()
                path.write_bytes(original + b"changed")
                after = evidence.runtime_provenance(self.root, [self.extra])
                self.assertNotEqual(before[name], after[name])
                self.assertEqual({key for key in before if before[key] != after[key]}, {name})
                path.write_bytes(original)

    def test_lexical_mutation_fails_even_if_config_pin_would_change(self) -> None:
        names = [name for name in evidence.runtime_provenance(self.root, [])
                 if name.startswith("model/intent_v1/sources/")]
        self.assertEqual(len(names), self.EXPECTED_LEXICAL_SOURCE_COUNT)
        for name in names:
            with self.subTest(name=name):
                path = self.root / name
                original = path.read_bytes()
                path.write_bytes(original + b"changed")
                with self.assertRaisesRegex(ValueError, "reference lexical checksum mismatch"):
                    evidence.runtime_provenance(self.root, [])
                path.write_bytes(original)

    def test_missing_required_inputs_and_extras_are_not_omitted(self) -> None:
        for name in (*evidence.REQUIRED_PATHS, "model/intent_v1/sources/en_US.lm", "model/prefix_v1/candidate.json"):
            with self.subTest(name=name):
                path = self.root / name
                original = path.read_bytes()
                path.unlink()
                with self.assertRaises(FileNotFoundError):
                    evidence.runtime_provenance(self.root, [self.extra])
                path.write_bytes(original)

    def test_optional_protected_json_and_added_removed_sources_change_pin_set(self) -> None:
        before = evidence.runtime_provenance(self.root, [])
        for name in ("src/keyswitch/resources/protected_tokens.json", "src/keyswitch/nested/new.py"):
            self.write(name, b"new input")
            after = evidence.runtime_provenance(self.root, [])
            self.assertEqual(set(after) - set(before), {name})
            (self.root / name).unlink()
            self.assertEqual(evidence.runtime_provenance(self.root, []), before)

    def test_external_extra_and_lexical_paths_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory(prefix="keyswitch-outside-pins-") as temporary:
            external = Path(temporary) / "outside"
            external.write_bytes(b"outside")
            with self.assertRaisesRegex(ValueError, "escapes repository"):
                evidence.runtime_provenance(self.root, [external])
            languages = cast(dict[str, object], cast(dict[str, object], self.config["sources"])["languages"])
            language = cast(dict[str, object], languages["en_US"])
            for name in (str(external), os.path.relpath(external, self.root)):
                with self.subTest(name=name):
                    language["path"] = name
                    self.save_config()
                    with self.assertRaisesRegex(ValueError, "repository relative|escapes repository"):
                        evidence.runtime_provenance(self.root, [])

    def test_symlink_escape_and_directory_extra_are_rejected(self) -> None:
        with self.assertRaisesRegex(ValueError, "not a file"):
            evidence.runtime_provenance(self.root, [self.root / "src"])
        with tempfile.TemporaryDirectory(prefix="keyswitch-outside-pins-") as temporary:
            external = Path(temporary) / "outside"
            external.write_bytes(b"outside")
            link = self.root / "src/keyswitch/escaped.py"
            try:
                link.symlink_to(external)
            except OSError:
                self.skipTest("symlink creation unavailable")
            with self.assertRaisesRegex(ValueError, "escapes repository"):
                evidence.runtime_provenance(self.root, [])

    def test_invalid_lexical_configuration_is_rejected(self) -> None:
        variants: tuple[object, ...] = ([], {"sources": []},
            {"sources": {"languages": {}}, "external_evaluation": {"hunspell": {}}})
        for variant in variants:
            with self.subTest(variant=variant):
                self.write(evidence.CONFIG_PATH, json.dumps(variant).encode())
                with self.assertRaises(ValueError):
                    evidence.runtime_provenance(self.root, [])
        self.save_config()
        languages = cast(dict[str, object], cast(dict[str, object], self.config["sources"])["languages"])
        cast(dict[str, object], languages["en_US"])["sha256"] = "not-a-hash"
        self.save_config()
        with self.assertRaisesRegex(ValueError, "invalid reference lexical checksum"):
            evidence.runtime_provenance(self.root, [])

    def test_explicit_packaged_loader_ignores_environment_and_invalidates_cache(self) -> None:
        path = self.root / evidence.INTENT_PATH
        fake = cast(LinearNgramModel, Mock(model_version="fixture", checksum="fixture-hash"))
        with patch.dict(os.environ, {"KEYSWITCH_INTENT_MODEL_PATH": "/missing/environment.ksm"}), \
                patch("auxiliary_runtime_evidence.LinearNgramModel.try_load_default", side_effect=AssertionError("discovery forbidden")), \
                patch("auxiliary_runtime_evidence.LinearNgramModel.load", return_value=fake) as loader:
            model, status = evidence.packaged_intent(self.root)
            self.assertIs(model, fake)
            self.assertEqual((status.available, status.path, status.version, status.checksum, status.error),
                             (True, path, "fixture", "fixture-hash", None))
            self.assertEqual(evidence.packaged_intent(self.root), (model, status))
            loader.assert_called_once_with(path)
            path.write_bytes(b"different size invalidates cache")
            evidence.packaged_intent(self.root)
            self.assertEqual(loader.call_count, self.LOAD_CALLS_AFTER_CONTENT_CHANGE)
            stat = path.stat()
            os.utime(path, ns=(stat.st_atime_ns, stat.st_mtime_ns + self.MTIME_BUMP_NANOSECONDS))
            evidence.packaged_intent(self.root)
            self.assertEqual(loader.call_count, self.LOAD_CALLS_AFTER_MTIME_CHANGE)

    def test_packaged_loader_propagates_missing_and_invalid_model_failures(self) -> None:
        with patch("auxiliary_runtime_evidence.LinearNgramModel.load", side_effect=ValueError("invalid fixture")):
            with self.assertRaisesRegex(ValueError, "invalid fixture"):
                evidence.packaged_intent(self.root)
        (self.root / evidence.INTENT_PATH).unlink()
        with self.assertRaises(FileNotFoundError):
            evidence.packaged_intent(self.root)


if __name__ == "__main__":
    unittest.main()
