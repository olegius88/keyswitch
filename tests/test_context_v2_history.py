"""Historical anchors and feature-2 compatibility; no prospective corpus access."""
from __future__ import annotations

import ast
from contextlib import redirect_stdout
from dataclasses import replace
import io
import json
from pathlib import Path
import re
import shutil
import sys
import tempfile
from types import ModuleType
from typing import cast
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

import context_corpus
from context_evidence import canonical, key
from keyswitch.constants.model_protocol import PROFILES
from context_frames import Frame
from keyswitch.context_model import ACTIONS, TERM_FREQUENCY_PATH, ContextModel
import train_context_v2 as trainer
import verify_context_v2 as verifier
import verify_context_v2_history as history
import historical_sources
from historical_sources import CONTEXT_V2, normalized_provenance
from fixture_values.counts import CONTEXT_V2_HISTORY_REPORT_ROWS
from fixture_values.models import CONTEXT_V2_HISTORY_MAX_FEATURES
from fixture_values.scores import DOMINANT_BIAS_WEIGHT
from keyswitch.constants.file_formats import SHA256_HEX_CHARACTERS
from keyswitch.constants.models import CONTEXT_ACTION_FEATURE_VERSION

NAMED_VALUE_PREFIX = "_NAMED_VALUE_"
ARCHIVED_MODEL = CONTEXT_V2.path(history.CURRENT_MODEL)


class _NamedNumbers(ast.NodeTransformer):
    """Moves every number of a module into a module-level constant, as the refactoring will."""

    def __init__(self) -> None:
        self.constants: list[ast.stmt] = []

    def visit_Constant(self, node: ast.Constant) -> ast.expr:
        if type(node.value) not in (int, float):
            return node
        name = NAMED_VALUE_PREFIX + str(len(self.constants))
        self.constants.append(ast.Assign([ast.Name(name, ast.Store())], ast.Constant(node.value)))
        return ast.copy_location(ast.Name(name, ast.Load()), node)


def named_numbers(source: str) -> str:
    tree = ast.parse(source)
    transformer = _NamedNumbers()
    transformer.visit(tree)
    head = next(index for index, node in enumerate(tree.body)
                if not isinstance(node, (ast.Expr, ast.Import, ast.ImportFrom)))
    tree.body[head:head] = transformer.constants
    return ast.unparse(ast.fix_missing_locations(tree))


def altered(source: str, pattern: str, replacement: str) -> str:
    result, count = re.subn(pattern, replacement, source)
    if not count:
        raise AssertionError("variant pattern not found: " + pattern)
    return result


class HistoricalContextV2Tests(unittest.TestCase):
    def test_reviewed_anchors_and_archived_sources_are_exact_without_current_engine_claim(self) -> None:
        root = history.ROOT
        # No live file the evidence pins is hashed: every one of them resolves to its archived copy.
        live = {root / name for name in history.SOURCES}
        checksum = historical_sources.checksum

        def digest(path: Path) -> str:
            if path in live:
                raise AssertionError("historical evidence must not claim current engine identity")
            return checksum(path)

        with patch.object(historical_sources, "checksum", side_effect=digest), patch.object(history, "checksum", side_effect=digest):
            result = history.verify_anchors(root / "model/context_v2")
        self.assertTrue(result["historical_evidence_verified"])
        self.assertIs(result["current_runtime_verified"], False)
        self.assertIn("no new fit", str(result["scope"]))

    def test_each_historical_artifact_and_source_remains_a_required_pin(self) -> None:
        root = history.ROOT
        targets = [root / "model/context_v2" / name for name in history.ANCHORS]
        targets += [CONTEXT_V2.path(name) for name in history.SOURCES]
        checksum = historical_sources.checksum
        for target in targets:
            def altered(path: Path, target: Path = target) -> str:
                return "0" * SHA256_HEX_CHARACTERS if path == target else checksum(path)
            with self.subTest(path=target.name), patch.object(history, "checksum", side_effect=altered), \
                    patch.object(historical_sources, "checksum", side_effect=altered):
                with self.assertRaisesRegex(ValueError, "historical"):
                    history.verify_anchors(root / "model/context_v2")
        with tempfile.TemporaryDirectory(prefix="keyswitch-historical-pin-") as temporary:
            root = Path(temporary)
            (root / "model/context_v2").mkdir(parents=True)
            (root / "model/context_v2/source.py").write_text("fixed numeric source")
            with self.assertRaisesRegex(ValueError, "source changed"):
                history.verify_sources({"model/context_v2/source.py": "0" * SHA256_HEX_CHARACTERS}, root)
            (root / "source.py").write_text("fixed numeric source")
            with self.assertRaisesRegex(ValueError, "not archived"):
                history.verify_sources({"source.py": checksum(root / "source.py")}, root)
        with patch("verify_context_v2_history.json.loads", return_value=[]):
            with self.assertRaisesRegex(ValueError, "historical evidence object"):
                history.verify_anchors(history.ROOT / "model/context_v2")

    def test_provenance_paths_and_archived_digest_cannot_be_relabeled(self) -> None:
        digest = "a" * SHA256_HEX_CHARACTERS
        invalid: tuple[object, ...] = ({}, [], {1: digest}, {"tools/file.py": "invalid"}, {"/outside.py": digest},
                     {"../outside.py": digest}, {"C:\\outside.py": digest},
                     {"tools/a.py": digest, "tools\\a.py": digest})
        for pins in invalid:
            with self.subTest(pins=pins), self.assertRaises(ValueError):
                normalized_provenance(pins)
        with self.assertRaisesRegex(ValueError, "unapproved"):
            history.verify_sources({"src/keyswitch/context_model.py": digest})

    def test_trainer_and_corpus_builder_check_their_recorded_pins_against_the_archive(self) -> None:
        seal = cast(dict[str, object], json.loads((context_corpus.CORPUS_ROOT / trainer.SEAL).read_bytes()))
        self.assertEqual(trainer.validate_seal(context_corpus.CORPUS_ROOT), seal)
        pins = cast(dict[str, str], seal["provenance"])
        self.assertEqual(trainer.recorded_provenance(seal), pins)
        changed = "0" * SHA256_HEX_CHARACTERS
        for change in ({**pins, "tools/context_corpus.py": changed},
                       {name: digest for name, digest in pins.items() if name != "tools/context_optimizer.c"}):
            with self.subTest(pins=sorted(change)), self.assertRaises(ValueError):
                trainer.recorded_provenance({**seal, "provenance": change})
        with redirect_stdout(io.StringIO()):
            self.assertEqual(context_corpus.main([]), 0)
        with tempfile.TemporaryDirectory(prefix="keyswitch-corpus-receipt-") as temporary:
            receipt = Path(temporary) / "corpus-receipt.json"
            content = cast(dict[str, object], json.loads(context_corpus.RECEIPT.read_bytes()))
            receipt.write_text(json.dumps({**content, "builder_sha256": changed}), encoding="utf-8")
            with patch.object(context_corpus, "RECEIPT", receipt), redirect_stdout(io.StringIO()):
                with self.assertRaisesRegex(ValueError, "unapproved"):
                    context_corpus.main([])

    def test_feature_two_behaviour_matches_the_archive_for_the_live_and_the_archived_source(self) -> None:
        archived = ARCHIVED_MODEL
        history.verify_feature_two(archived, archived)
        history.verify_feature_two(history.ROOT / history.CURRENT_MODEL, archived)
        plans = {plan.digest: plan for plan, _outcomes in history._REFERENCES.values()}
        plan = plans[history.FEATURE_TWO_PLAN_SHA256]
        # Every serving-only field of the live evidence is varied across the corpus.
        for name in history.SERVING_ONLY_EVIDENCE:
            with self.subTest(field=name):
                self.assertGreater(len({repr(extra[name]) for _arguments, extra in plan.evidence}), 1)

    def test_behaviour_neutral_refactors_pass_and_stay_private(self) -> None:
        archived = ARCHIVED_MODEL
        source = (history.ROOT / history.CURRENT_MODEL).read_text(encoding="utf-8")
        # A removed docstring, changed feature-3-only branches and dead code.
        for pattern, replacement in (
                (r'\n\s*"""Use the same language-support gate during calibration and inference\."""', ""),
                (r'return ContextPrediction\("suggest",', 'return ContextPrediction("wait",'),
                (r"(\n\s*)return any\(\n", r"\1if False:\1    pass\1return any(\n")):
            source = altered(source, pattern, replacement)
        # Every number becomes a named module constant and the file is reformatted.
        refactored = named_numbers(source)
        self.assertNotEqual(refactored, source)
        tree = ast.parse(refactored)
        named = {id(node.value) for node in tree.body if isinstance(node, ast.Assign)
                 and isinstance(node.targets[0], ast.Name) and node.targets[0].id.startswith(NAMED_VALUE_PREFIX)}
        numbers = [node for node in ast.walk(tree) if isinstance(node, ast.Constant) and type(node.value) in (int, float)]
        self.assertTrue(numbers)
        self.assertTrue(all(id(node) in named for node in numbers))
        # The feature limit is imported from a constants folder instead.
        refactored += "\nfrom fixture_values.models import CONTEXT_V2_HISTORY_MAX_FEATURES as MAX_FEATURES\n"
        module = history.load_context_module(archived.read_bytes(), archived, "fixture")
        self.assertEqual(module.MAX_FEATURES, CONTEXT_V2_HISTORY_MAX_FEATURES)
        live = {name: value for name, value in sys.modules.items() if name.startswith("keyswitch")}
        with tempfile.TemporaryDirectory(prefix="keyswitch-feature-two-neutral-") as temporary:
            variant = Path(temporary) / "context_model.py"
            variant.write_text(refactored, encoding="utf-8")
            history.verify_feature_two(variant, archived)
            self.assertEqual([path.name for path in Path(temporary).iterdir()], ["context_model.py"])
        after = {name: value for name, value in sys.modules.items() if name.startswith("keyswitch")}
        self.assertEqual(after.keys(), live.keys())
        self.assertTrue(all(after[name] is value for name, value in live.items()))
        self.assertFalse((ARCHIVED_MODEL.parent / "__pycache__").exists())
        with patch.dict(sys.modules, {history.COMPARISON_MODULE_PREFIX + "current": ModuleType("occupied")}):
            with self.assertRaisesRegex(ValueError, "cannot be isolated"):
                history.load_context_module(archived.read_bytes(), archived, "current")
        with self.assertRaisesRegex(ValueError, "feature-2 module cannot be loaded"):
            history.load_context_module(b"def broken(:\n", archived, "broken")

    def test_numerical_changes_of_the_live_source_fail(self) -> None:
        archived = ARCHIVED_MODEL
        source = (history.ROOT / history.CURRENT_MODEL).read_text(encoding="utf-8")
        changes = (
            ("extra feature", r'"bias": ([^,]+),', r'"bias": \1, "extra": \1,'),
            ("serving field read", r"return \{name: value for name, value in features\.items\(\) if value\}",
             "return {name: value for name, value in features.items() if value and not item.source_identifier}"),
            ("clipping bound", r"min\((?P<bound>[^,()]+), item\.score_delta\)", r"min(\g<bound> * 1.5, item.score_delta)"),
            ("before window", r"item\.field\.before\[-(?P<size>[^:\]]+):\]", r"item.field.before[-(\g<size> - 1):]"),
            ("normalization", re.escape('unicodedata.normalize("NFC", text.casefold())'),
             'unicodedata.normalize("NFKC", text.lower())'),
            ("softmax shift", re.escape("math.exp(value - maximum)"), "math.exp(value)"),
            ("softmax division", re.escape("value / total for value in values"), "value * (1.0 / total) for value in values"),
            ("softmax renamed", re.escape("def softmax("), "def renamed_softmax("),
            ("score product", re.escape("weight * value"), "weight + value"),
            ("tie break", re.escape("max(range(len(ACTIONS)),"), "max(reversed(range(len(ACTIONS))),"),
            ("threshold strictness", re.escape("probabilities[selected] < self.conversion_threshold"),
             "probabilities[selected] <= self.conversion_threshold"),
            ("support gate", r"return any\(\n", "return all(\n"),
            ("feature version", r"\Z", "\nFEATURE_VERSION = FEATURE_VERSION + 1\n"),
            ("feature limit", r"\Z", "\nMAX_FEATURES = MAX_FEATURES - 1\n"),
            ("artifact limit", r"\Z", "\nMAX_ARTIFACT_BYTES = MAX_ARTIFACT_BYTES - 1\n"),
            ("feature limit comparison", re.escape("len(raw_weights) <= MAX_FEATURES"), "len(raw_weights) < MAX_FEATURES"),
            ("artifact limit comparison", re.escape("len(raw) > MAX_ARTIFACT_BYTES"), "len(raw) >= MAX_ARTIFACT_BYTES"),
            ("weights dropped", re.escape("self.weights = dict(weights)"), "self.weights = {}"),
            ("threshold ignored", re.escape("self.conversion_threshold = conversion_threshold"),
             "self.conversion_threshold = 1.0"),
            ("weights scaled", re.escape("tuple(float(value) for value in values)"),
             "tuple(float(value) * 0.5 for value in values)"),
            ("payload reversed", re.escape("json.loads(raw)"), "json.loads(raw[::-1])"),
            ("name bound", r"len\(name\) > (?P<bound>[^ ]+)", r"len(name) >= \g<bound>"),
            ("weight bound", r"abs\(value\) > (?P<bound>[^ ]+)", r"abs(value) >= \g<bound>"),
            ("threshold bound", r"not (?P<low>[^ ]+) <= threshold <=", r"not \g<low> < threshold <="),
            ("version bound", r"len\(version\) > (?P<bound>[^:]+):", r"len(version) >= \g<bound>:"),
            ("feature-3 payload as feature 2", re.escape("float(threshold), feature_version=feature_version)"),
             "float(threshold))"),
        )
        with tempfile.TemporaryDirectory(prefix="keyswitch-feature-two-numeric-") as temporary:
            variant = Path(temporary) / "context_model.py"
            # The live loader reads the frequency table beside its own file as an action model loads: without it
            # the installed artifact would not load, and the feature-3 payload could not pass as feature 2.
            table = Path(temporary) / TERM_FREQUENCY_PATH.relative_to((history.ROOT / history.CURRENT_MODEL).parent)
            table.parent.mkdir(parents=True)
            shutil.copyfile(TERM_FREQUENCY_PATH, table)
            for description, pattern, replacement in changes:
                with self.subTest(change=description):
                    variant.write_text(altered(source, pattern, replacement), encoding="utf-8")
                    with self.assertRaisesRegex(ValueError, "feature-2"):
                        history.verify_feature_two(variant, archived)

    def test_probe_plan_is_pinned_and_cannot_shrink(self) -> None:
        root = history.ROOT
        archived, current = ARCHIVED_MODEL, root / history.CURRENT_MODEL
        with patch.object(history, "_VERIFIED", set()), patch.object(history, "_REFERENCES", {}):
            with patch.object(history, "FEATURE_TWO_PLAN_SHA256", "0" * SHA256_HEX_CHARACTERS):
                with self.assertRaisesRegex(ValueError, "probe plan changed"):
                    history.verify_feature_two(current, archived)
            with tempfile.TemporaryDirectory(prefix="keyswitch-feature-two-seed-") as temporary:
                shrunk = Path(temporary)
                (shrunk / history.INSTALLED_ARTIFACT).parent.mkdir(parents=True)
                shutil.copyfile(root / history.INSTALLED_ARTIFACT, shrunk / history.INSTALLED_ARTIFACT)
                seed = cast(dict[str, list[object]], json.loads((root / history.PROBE_SEED).read_bytes()))
                (shrunk / history.PROBE_SEED).parent.mkdir(parents=True)
                for key, value in (("words", seed["words"][:-1]), ("fields", [{"before": "only"}])):
                    with self.subTest(seed=key):
                        (shrunk / history.PROBE_SEED).write_text(json.dumps({**seed, key: value}), encoding="utf-8")
                        with self.assertRaisesRegex(ValueError, "probe plan changed|probe seed"):
                            history.verify_feature_two(current, archived, root / history.HISTORY, shrunk)

    def test_identical_bytes_are_verified_once_per_process(self) -> None:
        archived = ARCHIVED_MODEL
        history.verify_feature_two(history.ROOT / history.CURRENT_MODEL, archived)
        with patch.object(history, "_observe", side_effect=AssertionError("identical bytes observed twice")):
            history.verify_feature_two(history.ROOT / history.CURRENT_MODEL, archived)

    def test_numeric_wrapper_matches_original_four_track_report_without_fitting_or_writing(self) -> None:
        frames = [Frame(split + action, split + action, split, "eng", action, "слово", 0, "", "", "Telegram",
                        "unknown", "space", action, split + action, "authored")
                  for split in ("test", "lexical_test") for action in ("keep", "convert")]
        cache = {key(row, profile): (False, False, False, 0.0) for row in frames for profile in PROFILES}
        model = ContextModel({"bias": (DOMINANT_BIAS_WEIGHT, 0.0, 0.0, 0.0), "app:telegram": (0.0,) * len(ACTIONS)}, "context-v1-fixture")
        seal = {"schema_version": 1, "model_version": model.version, "artifact_sha256": "f" * SHA256_HEX_CHARACTERS}
        historical = {"historical_evidence_verified": True, "current_runtime_verified": False, "promotion_passed": False}
        with tempfile.TemporaryDirectory(prefix="keyswitch-historical-numeric-") as temporary:
            directory = Path(temporary)
            (directory / trainer.SEAL).write_bytes(canonical(seal))
            with patch.object(trainer, "validate_seal", return_value=seal), \
                    patch.object(ContextModel, "load", return_value=model), redirect_stdout(io.StringIO()):
                original = trainer.evaluate(directory, frames, cache)
            self.assertEqual(set(cast(dict[str, object], original["results"])),
                             {split + ":" + profile for split in ("test", "lexical_test") for profile in PROFILES})
            before = (directory / trainer.REPORT).read_bytes()
            with patch.object(verifier, "verify", return_value=historical), \
                    patch.object(verifier, "all_frames", return_value=frames), \
                    patch.object(verifier, "load_cache", return_value=cache), \
                    patch.object(ContextModel, "load", return_value=model), \
                    patch.object(trainer, "fit", side_effect=AssertionError("historical numeric replay must not fit")):
                result = verifier.verify_frozen(directory)
                self.assertIs(result["frozen_numeric_regression"], True)
                self.assertIs(result["promotion_passed"], False)
                self.assertEqual((directory / trainer.REPORT).read_bytes(), before)
                with patch.object(verifier, "all_frames", return_value=[replace(frames[0], original="changed"), *frames[1:]]):
                    with self.assertRaises((ValueError, KeyError)):
                        verifier.verify_frozen(directory)
                with patch.object(verifier, "metrics", return_value={"counts": {"rows": CONTEXT_V2_HISTORY_REPORT_ROWS, "desired_conversions": 1,
                        "converted_correctly": 0, "false_conversions": 1, "baseline_false_conversions": 1}, "categories": {}}):
                    with self.assertRaisesRegex(ValueError, "numeric report changed"):
                        verifier.verify_frozen(directory)
                with patch.object(ContextModel, "load", return_value=ContextModel({}, "context-v3-fixture", feature_version=CONTEXT_ACTION_FEATURE_VERSION)):
                    with self.assertRaisesRegex(ValueError, "feature-2"):
                        verifier.verify_frozen(directory)

    def test_changed_history_is_rejected_before_numeric_inputs_are_read(self) -> None:
        with patch.object(verifier, "verify", side_effect=ValueError("historical anchor changed")), \
                patch.object(verifier, "all_frames") as frames, patch.object(verifier, "load_cache") as cache:
            with self.assertRaisesRegex(ValueError, "historical anchor"):
                verifier.verify_frozen()
            frames.assert_not_called()
            cache.assert_not_called()

    def test_fast_history_gate_cannot_treat_feature_three_as_the_rejected_model(self) -> None:
        with patch.object(ContextModel, "load", return_value=ContextModel({}, "context-v3-fixture", feature_version=CONTEXT_ACTION_FEATURE_VERSION)):
            with self.assertRaisesRegex(ValueError, "candidate identity"):
                verifier.verify()

    def test_numeric_cli_is_explicit_and_fast_cli_does_not_replay(self) -> None:
        with patch.object(verifier, "verify", return_value={"current_runtime_verified": False}) as fast, \
                patch.object(verifier, "verify_frozen", return_value={"frozen_numeric_regression": True}) as numeric, \
                redirect_stdout(io.StringIO()):
            self.assertEqual(verifier.main([]), 0)
            fast.assert_called_once_with()
            numeric.assert_not_called()
            self.assertEqual(verifier.main(["--verify-frozen"]), 0)
            numeric.assert_called_once_with()


if __name__ == "__main__":
    unittest.main()
