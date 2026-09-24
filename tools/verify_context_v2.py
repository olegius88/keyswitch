#!/usr/bin/env python3
"""Fail closed on tampered research evidence or accidental failed-model rollout."""
from __future__ import annotations

import argparse
import json
from collections.abc import Sequence
from pathlib import Path
from typing import cast

from keyswitch.context_model import ARTIFACT_PATH, ContextModel
from context_corpus import CORPUS_ROOT, ROOT
from context_evidence import CACHE_RECEIPT, all_frames, canonical, checksum, load_cache
from train_context_v2 import ARTIFACT, REPORT, SEAL, audit, config, metrics, promotion_failures
from verify_context_v2_history import verify_anchors, verify_sources
from model_protocol import PROFILES, SEALED_BEFORE_TEST

METADATA_LIMIT_BYTES = 1024 * 1024
CONTEXT_MODEL_FEATURE_VERSION = 2
RECEIPT_INDENT = 2


def read_object(path: Path) -> dict[str, object]:
    with path.open("rb") as source:
        content = source.read(METADATA_LIMIT_BYTES + 1)
    if len(content) > METADATA_LIMIT_BYTES:
        raise ValueError("oversized context evidence metadata")
    value: object = json.loads(content)
    if not isinstance(value, dict) or value.get("schema_version") != 1:
        raise ValueError("invalid context evidence metadata")
    return cast(dict[str, object], value)


def validate_metrics(value: object) -> dict[str, object]:
    if not isinstance(value, dict) or not isinstance(value.get("counts"), dict) or not isinstance(value.get("categories"), dict):
        raise ValueError("missing candidate metrics")
    counts = cast(dict[str, object], value["counts"])
    fields = ("rows", "desired_conversions", "converted_correctly", "false_conversions", "baseline_false_conversions")
    if any(type(counts.get(field)) is not int or cast(int, counts[field]) < 0 for field in fields):
        raise ValueError("invalid candidate counts")
    if not 0 < cast(int, counts["desired_conversions"]) < cast(int, counts["rows"]) or cast(int, counts["converted_correctly"]) > cast(int, counts["desired_conversions"]):
        raise ValueError("impossible candidate counts")
    return cast(dict[str, object], value)


def verify(directory: Path = CORPUS_ROOT, active: Path = ARTIFACT_PATH) -> dict[str, object]:
    historical = verify_anchors(directory)
    seal = read_object(directory / SEAL)
    verify_sources(seal.get("provenance"))
    if seal.get("stage") != SEALED_BEFORE_TEST or seal.get("artifact_sha256") != checksum(directory / ARTIFACT):
        raise ValueError("candidate seal or provenance changed")
    model = ContextModel.load(directory / ARTIFACT)
    if model.feature_version != CONTEXT_MODEL_FEATURE_VERSION or seal.get("model_version") != model.version or seal.get("conversion_threshold") != model.conversion_threshold:
        raise ValueError("candidate identity changed")
    report = read_object(directory / REPORT)
    engine = read_object(directory / "engine-report.json")
    regression = engine.get("runtime_regression")
    if regression is not None:
        if not isinstance(regression, dict) or not isinstance(regression.get("previous_report"), str):
            raise ValueError("invalid runtime regression history")
        previous = ROOT / regression["previous_report"]
        if not previous.resolve().is_relative_to(ROOT / "model/context_v2/engine-history") or checksum(previous) != regression.get("previous_sha256"):
            raise ValueError("runtime regression history mismatch")
        if read_object(previous).get("source_ids") != engine.get("source_ids"):
            raise ValueError("runtime regression selection changed")
    if report.get("seal_sha256") != checksum(directory / SEAL) or report.get("artifact_sha256") != checksum(directory / ARTIFACT) or report.get("audit") != seal.get("audit") or report.get("reserve_used") is not False:
        raise ValueError("candidate evaluation identity changed")
    results = report.get("results")
    expected = {f"{split}:{profile}" for split in ("test", "lexical_test") for profile in ("portable", "reference_hunspell")}
    if not isinstance(results, dict) or set(results) != expected:
        raise ValueError("missing independent evaluation track")
    failures: dict[str, list[str]] = {}
    for track, value in results.items():
        if not isinstance(value, dict):
            raise ValueError("invalid independent evaluation track")
        current, prior = validate_metrics(value.get("candidate")), validate_metrics(value.get("v1"))
        failures[track] = promotion_failures(current, prior, cast(dict[str, object], config()["promotion"]))
    if (report.get("promotion_failures") != failures or report.get("promotion_passed") is not False
            or not any(failures.values()) or engine.get("promotion_passed") is not False):
        raise ValueError("candidate promotion result contradicts the metrics")
    for manifest in (engine, read_object(CACHE_RECEIPT)):
        manifest_hashes = manifest.get("provenance", manifest.get("source_hashes"))
        if not isinstance(manifest_hashes, dict) or not manifest_hashes:
            raise ValueError("missing engine or lexical provenance")
        verify_sources(manifest_hashes)
    # Active-model acceptance belongs to the separate versioned shipping gate.
    # This historical check only prevents installing the rejected candidate.
    active_digest = checksum(active)
    if active_digest == checksum(directory / ARTIFACT):
        raise ValueError("research candidate must not replace the shipping model")
    return {**historical, "schema_version": 1, "candidate": seal["model_version"], "promotion_passed": False,
        "engine_promotion_passed": False, "rejected_candidate_not_active": True,
        "active_artifact_sha256": active_digest,
        "artifact_sha256": seal["artifact_sha256"], "corpus_rows": cast(dict[str, object], seal["audit"])["rows"]}


def verify_frozen(directory: Path = CORPUS_ROOT, active: Path = ARTIFACT_PATH) -> dict[str, object]:
    """Repeat the four observed numeric tracks; never fit or write a report."""
    historical = verify(directory, active)
    expected = read_object(directory / REPORT)
    frames, cache = all_frames(), load_cache()
    candidate = ContextModel.load(directory / ARTIFACT)
    baseline = ContextModel.load(directory / "baseline-context-v1.json")
    if candidate.feature_version != CONTEXT_MODEL_FEATURE_VERSION or baseline.feature_version != CONTEXT_MODEL_FEATURE_VERSION:
        raise ValueError("historical numeric replay requires feature-2 models")
    results: dict[str, object] = {}
    failures: dict[str, list[str]] = {}
    for split in ("test", "lexical_test"):
        selected = [row for row in frames if row.split == split]
        for profile in PROFILES:
            current, previous = metrics(candidate, selected, profile, cache), metrics(baseline, selected, profile, cache)
            name = split + ":" + profile
            results[name] = {"candidate": current, "v1": previous}
            failures[name] = promotion_failures(current, previous, cast(dict[str, object], config()["promotion"]))
    reproduced = {**expected, "audit": audit(frames), "results": results,
                  "promotion_failures": failures, "promotion_passed": not any(failures.values())}
    if canonical(reproduced) != (directory / REPORT).read_bytes() or verify(directory, active) != historical:
        raise ValueError("historical context-v2 numeric report changed")
    return {**historical, "frozen_numeric_regression": True}


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify-frozen", action="store_true", help="Repeat historical numeric results without training or current-engine replay")
    args = parser.parse_args(argv)
    print(json.dumps(verify_frozen() if args.verify_frozen else verify(), ensure_ascii=True, indent=RECEIPT_INDENT))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
