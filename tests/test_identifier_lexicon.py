"""The packaged identifier lexicon: exact membership, strict loading, shared use in evidence."""

from __future__ import annotations

import hashlib
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from keyswitch.identifier_lexicon import RESOURCE_PATH, IdentifierLexicon
from keyswitch.constants.file_formats import VERSION_HASH_CHARACTERS
from keyswitch.context_action_features import extract_action_features
from keyswitch.context_model import ContextEvidence
from keyswitch.context_policy import evidence_for_decision, shared_identifiers
from keyswitch.detector import DetectionDecision, LanguageDetector
from keyswitch.input_context import FieldContext
from keyswitch.language_model import LanguageModel, WordScore
from fixture_values.corpora import IDENTIFIER_LEXICON_WORD_FREQUENCY
from fixture_values.counts import (
    LEXICON_FILLER_BYTES,
    PACKAGED_IDENTIFIER_COUNT,
    TINY_RESOURCE_LIMIT_BYTES,
)
from fixture_values.models import (
    NON_STRING_FIELD_VALUE,
    UNSUPPORTED_IDENTIFIER_LEXICON_SCHEMA_VERSION,
)
from fixture_values.scores import SAMPLE_MODEL_PROBABILITY, SAMPLE_MODEL_THRESHOLD

SCORE = WordScore(0.0, False, 0, 0.0)


class IdentifierLexiconTests(unittest.TestCase):
    def test_packaged_lexicon_is_the_frozen_debian_command_universe(self) -> None:
        lexicon = IdentifierLexicon.load()
        payload = json.loads(RESOURCE_PATH.read_bytes())
        self.assertEqual(payload["name"], "debian-trixie-commands-v1")
        self.assertEqual(payload["source"]["contents_sha256"], "3d3e6930908db0b53f780dd7397b9589e27db88b59205c331a76d76d6726e419")
        self.assertEqual(len(lexicon.identifiers), PACKAGED_IDENTIFIER_COUNT)
        self.assertEqual(
            lexicon.version,
            "identifiers-" + hashlib.sha256(RESOURCE_PATH.read_bytes()).hexdigest()[:VERSION_HASH_CHARACTERS],
        )
        for token in ("nthash", "NTHASH", "Btfsstat", "pwd", "zzuf"):
            with self.subTest(token=token):
                self.assertTrue(lexicon.contains(token))
        for token in ("терфыр", "", "nthas", "nthash ", "hello world", "гифку"):
            with self.subTest(token=token):
                self.assertFalse(lexicon.contains(token))
        self.assertIs(IdentifierLexicon.load(), lexicon)
        self.assertIs(shared_identifiers(), IdentifierLexicon.load())

    def test_oversized_resources_are_refused_before_parsing(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "identifiers.json"
            path.write_bytes(b"[" + b" " * LEXICON_FILLER_BYTES + b"]")
            with patch("keyswitch.identifier_lexicon.IDENTIFIER_LEXICON_MAX_BYTES", TINY_RESOURCE_LIMIT_BYTES), self.assertRaises(ValueError):
                IdentifierLexicon.load(path)

    def test_loader_rejects_malformed_resources_and_reports_missing_ones(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "identifiers.json"
            valid = {"schema_version": 1, "name": "fixture", "identifiers": ["abc", "def"]}
            path.write_text(json.dumps(valid), encoding="utf-8")
            self.assertTrue(IdentifierLexicon.load(path).contains("DEF"))
            broken_cases: list[object] = [
                {**valid, "schema_version": UNSUPPORTED_IDENTIFIER_LEXICON_SCHEMA_VERSION}, {**valid, "identifiers": ["def", "abc"]},
                {**valid, "identifiers": ["abc", "abc"]}, {**valid, "identifiers": ["Abc"]},
                {**valid, "identifiers": ["ab"]}, {**valid, "identifiers": []},
                {**valid, "name": NON_STRING_FIELD_VALUE}, [],
            ]
            for broken in broken_cases:
                with self.subTest(broken=broken):
                    other = Path(directory) / "broken.json"
                    other.write_text(json.dumps(broken), encoding="utf-8")
                    with self.assertRaises(ValueError):
                        IdentifierLexicon.load(other)
            lexicon, status = IdentifierLexicon.try_load(Path(directory) / "missing.json")
            self.assertIsNone(lexicon)
            self.assertIn("missing.json", status)

    def test_evidence_flags_identifiers_in_either_reading_and_features_expose_them(self) -> None:
        en = LanguageModel("en_US", {"hello": IDENTIFIER_LEXICON_WORD_FREQUENCY}, "test", enable_spellcheck=False)
        ru = LanguageModel("ru_RU", {"привет": IDENTIFIER_LEXICON_WORD_FREQUENCY}, "test", enable_spellcheck=False)
        detector = LanguageDetector({0: en, 1: ru})
        field = FieldContext("Terminal", "1", "run ", "", "text")
        wrong = DetectionDecision(
            True, "терфыр", "nthash", 1, 0, 0, "test", SCORE, SCORE, SAMPLE_MODEL_PROBABILITY, SAMPLE_MODEL_THRESHOLD
        )
        item = evidence_for_decision(wrong, "nthash", 0, detector, field, "space")
        self.assertEqual((item.source_identifier, item.target_identifier), (False, True))
        features = extract_action_features(item)
        self.assertEqual(features["identifier:0:1"], 1.0)
        self.assertEqual(features["identifier:0:1:direction:1"], 1.0)
        self.assertEqual(features["identifier:0:1:length:6"], 1.0)
        self.assertEqual(features["identifier:0:1:before:en"], 1.0)
        self.assertEqual((features["source:identifier:0"], features["target:identifier:1"]), (1.0, 1.0))
        right = DetectionDecision(
            False, "nthash", "nthash", 0, 0, 0, "test", SCORE, SCORE, SAMPLE_MODEL_PROBABILITY, SAMPLE_MODEL_THRESHOLD
        )
        item = evidence_for_decision(right, "терфыр", 1, detector, field, "space")
        self.assertEqual((item.source_identifier, item.target_identifier), (True, False))
        self.assertEqual(extract_action_features(item)["identifier:1:0:direction:0"], 1.0)
        # GNU hello ships as the command `hello`; a plain Russian word is outside the index.
        self.assertTrue(IdentifierLexicon.load().contains("hello"))
        word = DetectionDecision(
            False, "привет", "привет", 1, 1, 1, "test", SCORE, SCORE, SAMPLE_MODEL_PROBABILITY, SAMPLE_MODEL_THRESHOLD
        )
        item = evidence_for_decision(word, "ghbdtn", 0, detector, field, "space")
        features = extract_action_features(item)
        self.assertEqual(features["identifier:0:0"], 1.0)
        self.assertFalse(any(name.startswith("identifier:0:0:") for name in features))
        fixture = IdentifierLexicon(frozenset({"ghbdtn"}), "fixture", "identifiers-fixture")
        item = evidence_for_decision(word, "ghbdtn", 0, detector, field, "space", identifiers=fixture)
        self.assertEqual((item.source_identifier, item.target_identifier), (False, True))
        plain = ContextEvidence("привет", "ghbdtn", 1, field)
        self.assertEqual((plain.source_identifier, plain.target_identifier), (False, False))


if __name__ == "__main__":
    unittest.main()
