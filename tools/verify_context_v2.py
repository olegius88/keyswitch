#!/usr/bin/env python3
"""Fail closed on tampered research evidence or accidental failed-model rollout."""
from __future__ import annotations

import json
from pathlib import Path
from typing import cast

from keyswitch.context_model import ARTIFACT_PATH, ContextModel
from context_corpus import CORPUS_ROOT, ROOT
from context_evidence import CACHE_RECEIPT, checksum
from train_context_v2 import ARTIFACT, BASELINE, REPORT, SEAL, config, promotion_failures, provenance


def read_object(path: Path) -> dict[str, object]:
    with path.open("rb") as source:
        content = source.read(1024 * 1024 + 1)
    if len(content) > 1024 * 1024:
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
    # The trainer runs on Linux; package validation also runs on Windows.
    # Normalize only path separators, never file bytes or expected digests.
    seal = read_object(directory / SEAL)
    hashes = {relative.replace("\\", "/"): digest for relative, digest in provenance().items()}
    if seal.get("stage") != "sealed-before-test" or seal.get("provenance") != hashes or seal.get("artifact_sha256") != checksum(directory / ARTIFACT):
        raise ValueError("candidate seal or provenance changed")
    model = ContextModel.load(directory / ARTIFACT)
    if seal.get("model_version") != model.version or seal.get("conversion_threshold") != model.conversion_threshold:
        raise ValueError("candidate identity changed")
    report = read_object(directory / REPORT)
    engine = read_object(directory / "engine-report.json")
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
    if report.get("promotion_failures") != failures or report.get("promotion_passed") is not (not any(failures.values())):
        raise ValueError("candidate promotion result contradicts the metrics")
    for manifest in (engine, read_object(CACHE_RECEIPT)):
        manifest_hashes = manifest.get("provenance", manifest.get("source_hashes"))
        if not isinstance(manifest_hashes, dict) or not manifest_hashes:
            raise ValueError("missing engine or lexical provenance")
        for relative, expected_hash in manifest_hashes.items():
            path = ROOT / str(relative)
            if not path.resolve().is_relative_to(ROOT) or checksum(path) != expected_hash:
                raise ValueError("engine or lexical provenance mismatch")
    # This release ships infrastructure and a rejected research candidate,
    # not new runtime weights. A future promotion requires a new review and
    # independent holdout, not changing this boolean or copying candidate.json.
    if checksum(active) != checksum(BASELINE):
        raise ValueError("research candidate must not replace the shipping model")
    return {"schema_version": 1, "candidate": seal["model_version"], "promotion_passed": report["promotion_passed"],
        "engine_promotion_passed": engine.get("promotion_passed"), "active_model_unchanged": True,
        "artifact_sha256": seal["artifact_sha256"], "corpus_rows": cast(dict[str, object], seal["audit"])["rows"]}


def main() -> int:
    print(json.dumps(verify(), ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
