#!/usr/bin/env python3
"""Fast fail-closed context artifact/provenance gate for both native packages."""

from __future__ import annotations

import argparse
from collections.abc import Sequence
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
from typing import cast

from keyswitch.context_model import ARTIFACT_PATH, FEATURE_VERSION, ContextModel
import verify_context_action_model as action_verifier


ROOT = Path(__file__).resolve().parents[1]
REPORT = ROOT / "model/context_v1/report.json"
RECEIPT = ROOT / "model/context_v3/release-receipt.json"
REJECTED_ARTIFACT = action_verifier.REJECTED_ARTIFACT
MAX_REPORT_BYTES = 1024 * 1024
LEGACY_FEATURE_VERSION = FEATURE_VERSION
# The frozen src/keyswitch/context_model.py (PENDING_RESEAL) checks feature_version
# against this alongside its own FEATURE_VERSION but never names it.
ACTION_FEATURE_VERSION = 3
# Mirrors the frozen ContextModel.__init__ default; re-checked independently here.
LEGACY_CONVERSION_THRESHOLD = 0.985
MINIMUM_TEST_ROWS = 10_000
REPORT_JSON_INDENT = 2


def provenance_paths(root: Path = ROOT, artifact: Path = ARTIFACT_PATH) -> dict[str, Path]:
    """Report field -> file whose bytes it pins.

    Exposed so a test can assert every one of them is protected from end-of-line
    conversion: a checkout that rewrites such a file invalidates the model on
    that platform alone, which is invisible on the machine that trained it.
    """

    return {
        "sources_sha256": root / "model/context_v1/scenarios.json",
        "holdout_sha256": root / "model/context_v1/holdout-3.json",
        "runtime_sha256": root / "src/keyswitch/context_model.py",
        # The corpus reproduces the runtime short-word policy, so its source
        # is part of the evidence a package must be able to re-check.
        "policy_sha256": root / "src/keyswitch/short_words.py",
        "trainer_sha256": root / "tools/train_context_model.py",
        "baseline_sha256": root / "src/keyswitch/resources/models/layout_intent_v1.ksm",
        "artifact_sha256": artifact,
    }


def verify_legacy(root: Path = ROOT, report_path: Path = REPORT, artifact: Path = ARTIFACT_PATH) -> dict[str, object]:
    """Preserve the historical feature-2 contract independently of active weights."""
    with report_path.open("rb") as handle:
        raw = handle.read(MAX_REPORT_BYTES + 1)
    if len(raw) > MAX_REPORT_BYTES:
        raise ValueError("oversized context report")
    report: object = json.loads(raw)
    if not isinstance(report, dict) or report.get("schema_version") != 1 or report.get("quality_gates_passed") is not True or report.get("test_overlap") != 0:
        raise ValueError("context quality gates failed or report missing")
    paths = provenance_paths(root, artifact)
    for name, path in paths.items():
        if report.get(name) != hashlib.sha256(path.read_bytes()).hexdigest():
            raise ValueError(f"context provenance mismatch: {name}")
    model = ContextModel.load(artifact)
    if model.feature_version != LEGACY_FEATURE_VERSION or model.version != report.get("model_version") or model.conversion_threshold != LEGACY_CONVERSION_THRESHOLD:
        raise ValueError("context model identity or threshold differs from evaluation")
    test = report.get("test")
    counts = test.get("counts") if isinstance(test, dict) else None
    if not isinstance(counts, dict):
        raise ValueError("missing context evaluation counts")
    required = ("rows", "desired_conversions", "converted_correctly", "false_conversions", "baseline_converted_correctly")
    if any(type(counts.get(name)) is not int or counts[name] < 0 for name in required):
        raise ValueError("invalid context evaluation counts")
    if counts["rows"] < MINIMUM_TEST_ROWS or counts["false_conversions"] != 0 or counts["converted_correctly"] < counts["baseline_converted_correctly"]:
        raise ValueError("context evaluation fails release policy")
    return {"model_version": model.version, "artifact_sha256": report["artifact_sha256"], "counts": cast(dict[str, object], counts), "evidence_scope": report.get("evidence_scope")}


def verify(
    root: Path = ROOT, report_path: Path = REPORT, artifact: Path = ARTIFACT_PATH,
    receipt_path: Path = RECEIPT,
) -> dict[str, object]:
    """The single active-model gate used by CI and native packaging."""

    fingerprint = action_verifier.checksum(artifact)
    if fingerprint == REJECTED_ARTIFACT:
        raise ValueError("rejected context-v2 research artifact cannot be activated")
    model = ContextModel.load(artifact)
    if model.feature_version == LEGACY_FEATURE_VERSION:
        result = verify_legacy(root, report_path, artifact)
        # Local import avoids the historical verifier's REPORT import cycle.
        from verify_context_v2 import verify as verify_research

        verify_research(root / "model/context_v2", artifact)
    elif model.feature_version == ACTION_FEATURE_VERSION:
        receipt = action_verifier.verify(root=root, receipt_path=receipt_path, artifact=artifact)
        result = {"model_version": receipt["model_version"], "artifact_sha256": receipt["artifact_sha256"],
                  "evidence_scope": receipt["scope"]}
    else:
        raise ValueError("unsupported active context feature version")
    if action_verifier.checksum(artifact) != fingerprint:
        raise ValueError("active context artifact changed during verification")
    return {**result, "feature_version": model.feature_version, "quality_gates_passed": True}


def replay_commands(feature_version: int) -> tuple[tuple[str, ...], ...]:
    if feature_version == LEGACY_FEATURE_VERSION:
        # Rejected context-v2 has its separate historical numeric CI gate.
        return (("tools/train_context_model.py", "--verify"),)
    if feature_version == ACTION_FEATURE_VERSION:
        # Explicit classes fail if a required module/class disappears; an empty
        # discovery pattern would otherwise exit successfully with zero tests.
        return tuple(("-m", "unittest", "-v", target) for target in (
            "test_language_intent_regressions.LanguageIntentRegressions",
            "test_input_sequence_matrix.InputSequenceMatrixTests",
            "test_context_policy.ContextEngineTests.test_bundled_trained_model_resolves_user_phrase_and_retains_code",
            "test_default_input_sequences.DefaultInputSequenceTests"))
    raise ValueError("unsupported context replay feature version")


def replay(root: Path, feature_version: int) -> None:
    environment = dict(os.environ)
    environment["PYTHONPATH"] = os.pathsep.join(str(root / name) for name in ("src", "tools", "tests"))
    for command in replay_commands(feature_version):
        subprocess.run([sys.executable, *command], cwd=root, env=environment,
                       check=True, stdout=sys.stderr)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--replay", action="store_true", help="Run only the active feature version's public verification protocol")
    args = parser.parse_args(argv)
    result = verify()
    if args.replay:
        replay(ROOT, cast(int, result["feature_version"]))
        if verify() != result:
            raise ValueError("active context evidence changed during replay")
    print(json.dumps(result, ensure_ascii=False, indent=REPORT_JSON_INDENT))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
