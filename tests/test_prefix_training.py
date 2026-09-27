"""Frozen prefix evidence and fail-closed package gates (no retraining here)."""
from __future__ import annotations

import copy
import json
import os
import subprocess
import sys
import tempfile
import unittest
from fnmatch import fnmatchcase
from pathlib import Path
from typing import cast
from unittest.mock import patch

from keyswitch.layouts import LayoutPair
from keyswitch.prefix_schema import VersionedPrefixModel
from keyswitch.constants.file_formats import SHA256_HEX_CHARACTERS, VERSION_HASH_CHARACTERS
from keyswitch.constants.models import CURRENT_PREFIX_FEATURE_VERSION, PREFIX_MIN_CHARACTERS
from keyswitch.prefix_model import ARTIFACT, PrefixModel
from keyswitch.value_provenance import PinnedValues, pin_values

TOOLS = str(Path(__file__).resolve().parents[1] / "tools")
if TOOLS not in sys.path:
    sys.path.insert(0, TOOLS)
import prefix_corpus as corpus
import train_prefix_model as trainer
import verify_prefix_model as verifier
import verify_context_action_model as action_verifier
from keyswitch.constants.model_protocol import ACTIVE_SPLITS, PORTABLE, REFERENCE_HUNSPELL
from verify_lexical_compatibility import contract_sha256
from fixture_values.clock import PREFIX_TRAINING_SUBPROCESS_TIMEOUT_SECONDS
from fixture_values.corpora import PREFIX_LEXICON_HIGH_FREQUENCY, PREFIX_LEXICON_LOW_FREQUENCY
from fixture_values.counts import (
    PREFIX_EXPECTED_CHARACTERS_BEFORE_CONVERSION,
    PREFIX_OUT_OF_RANGE_CONVERTED_COUNT,
    PREFIX_SPLIT_PROBE_SAMPLE_COUNT,
)
from fixture_values.scores import (
    PREFIX_METRICS_IDENTIFIER_CANDIDATE_PROBABILITY,
    PREFIX_METRICS_LENIENT_THRESHOLD,
    PREFIX_METRICS_STRICT_THRESHOLD,
    PREFIX_METRICS_WRONG_FIRST_CANDIDATE_PROBABILITY,
    PREFIX_METRICS_WRONG_NEXT_CANDIDATE_PROBABILITY,
    PREFIX_POLICY_ALTERED_CONVERSION_THRESHOLD,
    PREFIX_POLICY_SAMPLE_CONVERSION_THRESHOLD,
    PREFIX_POLICY_SAMPLE_FEATURE_WEIGHT,
)

EXAMPLE_SEQUENCE_RESULTS = [
    trainer.SequenceResult("ghbdtn", "wrong", True, "portable",
                           [(PREFIX_MIN_CHARACTERS, PREFIX_METRICS_WRONG_FIRST_CANDIDATE_PROBABILITY),
                            (PREFIX_MIN_CHARACTERS + 1, PREFIX_METRICS_WRONG_NEXT_CANDIDATE_PROBABILITY)],
                           PREFIX_MIN_CHARACTERS + 1),
    trainer.SequenceResult("function", "identifier", False, "portable",
                           [(PREFIX_MIN_CHARACTERS, PREFIX_METRICS_IDENTIFIER_CANDIDATE_PROBABILITY)], None),
    trainer.SequenceResult("hello", "correct", False, "portable", [], None),
]


class PrefixEvidenceTests(unittest.TestCase):
    def test_verifier_cli_preserves_unicode_report_on_legacy_stdout(self) -> None:
        root = Path(TOOLS).parent
        expected = verifier.verify()
        # The schema-2 report is English throughout; the schema-1 evidence this
        # verifier still reads is not, and it is what the escaping is for.
        self.assertFalse(json.dumps(verifier.verify(artifact=trainer.CANDIDATE), ensure_ascii=False).isascii())
        for encoding in ("cp1252", "ascii", "utf-8"):
            with self.subTest(encoding=encoding):
                completed = subprocess.run(
                    [sys.executable, str(Path(TOOLS) / "verify_prefix_model.py")],
                    cwd=root,
                    env={**os.environ, "PYTHONPATH": str(root / "src"),
                         "PYTHONIOENCODING": encoding},
                    capture_output=True,
                    timeout=PREFIX_TRAINING_SUBPROCESS_TIMEOUT_SECONDS,
                    check=False,
                )
                self.assertEqual(completed.returncode, 0, completed.stderr.decode("utf-8", "replace"))
                self.assertTrue(completed.stdout.isascii())
                self.assertEqual(json.loads(completed.stdout), expected)

    def test_hashed_prefix_text_inputs_pin_checkout_line_endings(self) -> None:
        engine = json.loads((corpus.DIRECTORY / "engine-report.json").read_bytes())
        paths = set(corpus.provenance()) | set(trainer.provenance()) | set(engine["provenance"])
        root = Path(TOOLS).parent
        paths.add(ARTIFACT.relative_to(root).as_posix())
        attributes = (root / ".gitattributes").read_text(encoding="utf-8")
        patterns = [parts[0] for line in attributes.splitlines()
                    if (parts := line.split())[1:] == ["text", "eol=lf"]]
        for path in sorted(paths):
            if Path(path).suffix in {".py", ".json", ".c", ".txt"}:
                self.assertTrue(any(fnmatchcase(path, pattern) for pattern in patterns), path)

    def test_receipt_and_seal_pin_the_values_their_code_imports(self) -> None:
        """A value moved into keyswitch.constants stays covered by the corpus receipt and the seal."""
        root = Path(TOOLS).parent
        receipt = json.loads(corpus.RECEIPT.read_bytes())
        self.assertEqual(receipt["provenance"], corpus.provenance())
        code = [root / path for path in receipt["provenance"] if path.endswith(".py")]
        corpus_values = pin_values(code, source_root=root / "src")
        self.assertEqual(receipt["provenance"]["constants_sha256"], corpus_values.sha256)
        for name in ("PREFIX_SPLIT_BUCKET_COUNT", "PREFIX_FAMILY_KEY_CHARACTERS", "PREFIX_V1_BEFORE_CONTEXT_CHARACTERS",
                     "EARLY_SWITCH_MIN_TARGET_FREQUENCY", "PREFIX_INDEX_DICTIONARY_MAX_BYTES"):
            self.assertIn("keyswitch.constants.prefix." + name, corpus_values.values)
        seal = json.loads(trainer.SEAL.read_bytes())
        self.assertEqual(seal["provenance"], trainer.provenance())
        trainer_values = pin_values([Path(trainer.__file__)], source_root=root / "src")
        self.assertEqual(seal["provenance"]["constants_sha256"], trainer_values.sha256)
        self.assertIn("keyswitch.constants.prefix.PREFIX_MIN_FEATURE_OCCURRENCES", trainer_values.values)
        # Of the intent configuration only the lexical contract is pinned, so another intent
        # generation with the same lexicons keeps the corpus valid; the supplement is pinned itself.
        self.assertNotIn("model/intent_v1/config.json", receipt["provenance"])
        self.assertEqual(receipt["provenance"]["lexical_contract_sha256"],
                         contract_sha256(json.loads((root / "model/intent_v1/config.json").read_bytes())))
        self.assertIn("src/keyswitch/resources/lexicon-supplement-ru_RU.json", receipt["provenance"])
        self.assertIn("supplement_words_sha256", receipt["provenance"])
        # A changed value fails the receipt before any frozen row is read, and so the package gate.
        changed = PinnedValues("0" * SHA256_HEX_CHARACTERS, {})
        with patch.object(corpus, "pin_values", return_value=changed):
            with self.assertRaisesRegex(ValueError, "provenance changed"):
                corpus.verify_receipt()
            with self.assertRaisesRegex(ValueError, "provenance changed"):
                verifier.verify(artifact=trainer.CANDIDATE)
        with patch.object(trainer, "pin_values", return_value=changed):
            with self.assertRaisesRegex(ValueError, "seal changed"):
                verifier.verify(artifact=trainer.CANDIDATE)

    def test_corpus_scores_with_the_served_lexicon_and_indexes_prefixes_like_the_engine(self) -> None:
        """The engine scores with the supplemented model and indexes the onboard lexicon; the corpus too.

        engine.py serves LanguageModel.load(locale, supplement_words(locale)) and builds its prefix
        index with PrefixIndex.for_language_model, which reloads LanguageModel.load(locale): a
        supplement form is known to the scorer but starts no completion in the index.
        """
        from keyswitch import early_switch
        from keyswitch.language_model import LanguageModel

        onboard = {"привет": PREFIX_LEXICON_HIGH_FREQUENCY, "привод": PREFIX_LEXICON_LOW_FREQUENCY}
        onboard_model = LanguageModel("ru_RU", dict(onboard), "fixture", enable_spellcheck=False)
        served_model = LanguageModel("ru_RU", {**onboard, "приветик": PREFIX_LEXICON_LOW_FREQUENCY}, "fixture",
                                     enable_spellcheck=False)
        early_switch._cached_index.cache_clear()
        self.addCleanup(early_switch._cached_index.cache_clear)
        with tempfile.TemporaryDirectory() as temporary:
            dictionary = Path(temporary) / "ru_RU.dic"
            dictionary.write_text("2\nприветствие/A\nпривал\n", encoding="utf-8")
            for profile, spelling in ((PORTABLE, False), (REFERENCE_HUNSPELL, True)):
                with self.subTest(profile=profile):
                    early_switch._cached_index.cache_clear()
                    with patch.object(served_model, "source", "fixture; Hunspell: " + str(dictionary) if spelling else "fixture"), \
                            patch.object(served_model, "speller", type("Speller", (), {"source": str(dictionary), "available": True})()
                                         if spelling else served_model.speller):
                        with patch.object(LanguageModel, "load", return_value=onboard_model) as load:
                            engine_index = early_switch.PrefixIndex.for_language_model(served_model)
                        load.assert_called_once_with("ru_RU")
                        with patch.object(corpus, "serving_models", return_value={1: served_model}) as serving, \
                                patch.object(corpus, "onboard_models", return_value={1: onboard_model}):
                            models, indexes = corpus.lexicon(profile)
                    serving.assert_called_once_with(spelling)
                    self.assertIs(models[1], served_model)
                    self.assertEqual((indexes[1]._words, indexes[1]._frequencies), (engine_index._words, engine_index._frequencies))
                    self.assertEqual(indexes[1].completions("приветик").completions, 0)
                    self.assertTrue(models[1].score("приветик").known)
                    self.assertEqual(indexes[1].completions("приветс").completions, int(spelling))

    def test_physical_prefix_families_stay_together_and_frozen_test_cannot_be_replaced(self) -> None:
        pair = LayoutPair()
        self.assertEqual(corpus.family("привет", 1, pair), corpus.family("ghbdtn", 0, pair))
        self.assertEqual(corpus.family("ghbdtn_value", 0, pair), corpus.family("ghbdtn", 0, pair))
        namespace = str(corpus.config()["namespace"])
        self.assertEqual(
            {corpus.split_for(str(index), namespace) for index in range(PREFIX_SPLIT_PROBE_SAMPLE_COUNT)},
            set(ACTIVE_SPLITS),
        )
        with self.assertRaisesRegex(ValueError, "overwrite"):
            corpus.freeze()
        with self.assertRaisesRegex(ValueError, "already observed"):
            trainer.main(["fit"])
        with self.assertRaisesRegex(ValueError, "already observed"):
            trainer.main(["test"])
        with self.assertRaisesRegex(ValueError, "invalid prefix split"):
            next(corpus.rows("private"))

    def test_metrics_count_the_first_conversion_per_sequence_not_each_prefix(self) -> None:
        examples = EXAMPLE_SEQUENCE_RESULTS
        counts = cast(dict[str, int], trainer.metrics(examples, PREFIX_METRICS_STRICT_THRESHOLD)["counts"])
        self.assertEqual(
            (counts["converted"], counts["false"], counts["characters_before_conversion"]),
            (1, 0, PREFIX_EXPECTED_CHARACTERS_BEFORE_CONVERSION),
        )
        counts = cast(dict[str, int], trainer.metrics(examples, PREFIX_METRICS_LENIENT_THRESHOLD)["counts"])
        self.assertEqual((counts["false"], counts["technical_false"]), (1, 1))
        self.assertFalse(trainer.accepted({"profiles": {}}))
        report = json.loads(trainer.REPORT.read_bytes())
        self.assertTrue(trainer.accepted(report["test"]))
        for key, value in (("sequences", True), ("desired", 0), ("false", -1),
                           ("converted", PREFIX_OUT_OF_RANGE_CONVERTED_COUNT), ("technical_false", 1),
                           ("converted_before_end", 0)):
            modified = copy.deepcopy(report["test"])
            modified["profiles"]["portable"][key] = value
            self.assertFalse(trainer.accepted(modified))

    def test_a_schema_two_prefix_model_is_gated_by_the_pair_receipt(self) -> None:
        """Once a context-action pair is installed, the prefix artifact's evidence is that receipt."""
        from keyswitch.context_model import ACTIONS
        import hashlib

        weights = {
            "bias": [0.0] * len(ACTIONS),
            "source:prefix_char:0:1:^a": [PREFIX_POLICY_SAMPLE_FEATURE_WEIGHT] + [0.0] * (len(ACTIONS) - 1),
        }
        digest = hashlib.sha256(json.dumps(weights, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()
        payload = {"kind": "keyswitch.prefix-policy", "feature_version": CURRENT_PREFIX_FEATURE_VERSION,
                   "prefix_feature_version": CURRENT_PREFIX_FEATURE_VERSION,
                   "actions": list(ACTIONS), "version": "prefix-v2-" + digest[:VERSION_HASH_CHARACTERS],
                   "conversion_threshold": PREFIX_POLICY_SAMPLE_CONVERSION_THRESHOLD,
                   "weights": weights, "weights_sha256": digest}
        with tempfile.TemporaryDirectory() as temporary:
            artifact = Path(temporary) / "prefix_policy_v1.json"
            artifact.write_text(json.dumps(payload), encoding="utf-8")
            receipt: dict[str, object] = {"model_version": "context-v3-000000000000", "scope": "sealed pair evaluation"}
            with patch.object(action_verifier, "verify", return_value=receipt) as gate:
                result = verifier.verify(artifact=artifact)
            self.assertEqual(gate.call_args.kwargs["prefix_artifact"], artifact)
            self.assertEqual((result["feature_version"], result["model_version"], result["accepted"], result["active"]),
                             (CURRENT_PREFIX_FEATURE_VERSION, payload["version"], True, True))
            self.assertEqual(result["receipt_model_version"], receipt["model_version"])
            # A receipt that does not verify blocks the package, and so does an artifact that moves under the gate.
            with patch.object(action_verifier, "verify", side_effect=ValueError("ledger outcome missing")):
                with self.assertRaises(ValueError):
                    verifier.verify(artifact=artifact)

            def swap(**_kwargs: object) -> dict[str, object]:
                artifact.write_text(
                    json.dumps({**payload, "conversion_threshold": PREFIX_POLICY_ALTERED_CONVERSION_THRESHOLD}),
                    encoding="utf-8",
                )
                return receipt

            with patch.object(action_verifier, "verify", side_effect=swap):
                with self.assertRaises(ValueError):
                    verifier.verify(artifact=artifact)

    def test_package_gate_binds_weights_test_counts_engine_and_runtime(self) -> None:
        accepted = verifier.verify()
        self.assertTrue(accepted["accepted"])
        # The shipped artifact is the one its evidence names, in whichever schema
        # that evidence is written: schema one carries its own report, schema two
        # is bound by the context-action release receipt.
        installed = VersionedPrefixModel.load()
        assert installed is not None
        self.assertEqual(installed.version, accepted["model_version"])
        self.assertEqual(PrefixModel.load(trainer.CANDIDATE).version,
                         verifier.verify(artifact=trainer.CANDIDATE)["model_version"])
        report = json.loads(trainer.REPORT.read_bytes())
        engine = json.loads((corpus.DIRECTORY / "engine-report.json").read_bytes())
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "evidence.json"
            modifications: list[dict[str, object]] = [{"accepted": False}, {"test": {}}, {"candidate_sha256": "changed"}]
            # The schema-one report gates the schema-one artifact; the installed pair
            # is schema two and is gated by the release receipt instead, so the report
            # is tampered with against the artifact it actually describes.
            for modification in modifications:
                path.write_text(json.dumps({**report, **modification}), encoding="utf-8")
                with self.assertRaises(ValueError):
                    verifier.verify(report_path=path, artifact=trainer.CANDIDATE)
            variants = [{**engine, "provenance": {}}, {**engine, "passed": False}, {**engine, "sequence_ids": []}]
            modified = copy.deepcopy(engine)
            modified["results"]["portable/observed"]["candidate"]["length_mismatches"] = 1
            variants.append(modified)
            for modified in variants:
                path.write_text(json.dumps(modified), encoding="utf-8")
                with self.assertRaises(ValueError):
                    verifier.verify(engine_path=path, artifact=trainer.CANDIDATE)
            path.write_bytes(b"different weights")
            with self.assertRaises(ValueError):
                verifier.verify(artifact=path)
            with patch.object(verifier, "checksum", return_value="changed"):
                with self.assertRaises(ValueError):
                    verifier.verify(artifact=trainer.CANDIDATE)
            # Whatever schema is installed, an artifact its evidence does not name is
            # refused even when it loads: schema one by its own report, schema two by
            # the release receipt that binds the pair.
            other = Path(temporary) / "prefix.json"
            payload = json.loads(ARTIFACT.read_bytes())
            other.write_text(
                json.dumps({**payload, "conversion_threshold": PREFIX_POLICY_ALTERED_CONVERSION_THRESHOLD}), encoding="utf-8"
            )
            self.assertEqual(VersionedPrefixModel.load(other).feature_version, installed.feature_version)
            with self.assertRaises(ValueError):
                verifier.verify(artifact=other)
            self.assertTrue(ARTIFACT.is_file())


if __name__ == "__main__":
    unittest.main()
