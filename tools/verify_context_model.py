#!/usr/bin/env python3
"""Fast fail-closed context artifact/provenance gate for both native packages."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
from typing import cast

from keyswitch.context_model import ARTIFACT_PATH, ContextModel


ROOT = Path(__file__).resolve().parents[1]
REPORT = ROOT / "model/context_v1/report.json"


def verify(root: Path = ROOT, report_path: Path = REPORT, artifact: Path = ARTIFACT_PATH) -> dict[str, object]:
    with report_path.open("rb") as handle:
        raw = handle.read(1024 * 1024 + 1)
    if len(raw) > 1024 * 1024:
        raise ValueError("oversized context report")
    report: object = json.loads(raw)
    if not isinstance(report, dict) or report.get("schema_version") != 1 or report.get("quality_gates_passed") is not True or report.get("test_overlap") != 0:
        raise ValueError("context quality gates failed or report missing")
    paths = {
        "sources_sha256": root / "model/context_v1/scenarios.json",
        "holdout_sha256": root / "model/context_v1/holdout-2.json",
        "runtime_sha256": root / "src/keyswitch/context_model.py",
        "trainer_sha256": root / "tools/train_context_model.py",
        "baseline_sha256": root / "src/keyswitch/resources/models/layout_intent_v1.ksm",
        "artifact_sha256": artifact,
    }
    for name, path in paths.items():
        if report.get(name) != hashlib.sha256(path.read_bytes()).hexdigest():
            raise ValueError(f"context provenance mismatch: {name}")
    model = ContextModel.load(artifact)
    if model.version != report.get("model_version") or model.conversion_threshold != 0.985:
        raise ValueError("context model identity or threshold differs from evaluation")
    test = report.get("test")
    counts = test.get("counts") if isinstance(test, dict) else None
    if not isinstance(counts, dict):
        raise ValueError("missing context evaluation counts")
    required = ("rows", "desired_conversions", "converted_correctly", "false_conversions", "baseline_converted_correctly")
    if any(type(counts.get(name)) is not int or counts[name] < 0 for name in required):
        raise ValueError("invalid context evaluation counts")
    if counts["rows"] < 10000 or counts["false_conversions"] != 0 or counts["converted_correctly"] < counts["baseline_converted_correctly"]:
        raise ValueError("context evaluation fails release policy")
    return {"model_version": model.version, "artifact_sha256": report["artifact_sha256"], "counts": cast(dict[str, object], counts), "evidence_scope": report.get("evidence_scope")}


def main() -> int:
    print(json.dumps(verify(), ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
