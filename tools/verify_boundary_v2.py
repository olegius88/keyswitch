"""Fail-closed package gate for the accepted boundary policy and engine replay."""
from __future__ import annotations

import json
from pathlib import Path

from keyswitch.boundary_policy import ARTIFACT, BoundaryPolicy
from boundary_v2_corpus import DIRECTORY, RECEIPT, SPLITS
from context_evidence import checksum
from evaluate_boundary_engine import REPORT as ENGINE_REPORT, SCENARIOS, provenance as engine_provenance
from train_boundary_v2 import CANDIDATE, REPORT, evaluate
from verify_context_v2 import read_object


def verify(*, artifact: Path = ARTIFACT, report_path: Path = REPORT,
           engine_path: Path = ENGINE_REPORT) -> dict[str, object]:
    corpus = read_object(RECEIPT)
    if (corpus.get("family_overlap") != 0 or corpus.get("prior_phrase_test_family_overlap") != 0
            or corpus.get("sha256") != {split: checksum(DIRECTORY / (split + ".jsonl.gz")) for split in SPLITS}):
        raise ValueError("boundary-v2 frozen partitions changed")
    raw = evaluate()
    report = json.loads(raw)
    if raw != report_path.read_bytes() or report["accepted"] is not True:
        raise ValueError("boundary-v2 sealed quality evidence changed or rejected")
    if not artifact.exists() or checksum(artifact) != checksum(CANDIDATE):
        raise ValueError("approved boundary-v2 model missing or changed")
    engine = read_object(engine_path)
    if engine.get("provenance") != engine_provenance():
        raise ValueError("boundary-v2 engine provenance changed")
    results = engine.get("results")
    if not isinstance(results, dict) or set(results) != {"legacy_no_segmentation", "rejected_v1", "active_v2"}:
        raise ValueError("boundary-v2 engine comparisons missing")
    active = results["active_v2"]
    if (not isinstance(active, dict) or active.get("exact") != len(SCENARIOS)
            or any(active.get(key) != 0 for key in ("changed_correct", "length_mismatches", "injections_before_hard_boundary"))):
        raise ValueError("boundary-v2 exact-text gate failed")
    replay = active.get("rows")
    if not isinstance(replay, list) or len(replay) != len(SCENARIOS):
        raise ValueError("boundary-v2 engine scenarios missing")
    for row, (original, expected) in zip(replay, SCENARIOS):
        if not isinstance(row, dict) or row != {
            "original": original, "expected": expected + " ", "actual": expected + " ", "before_hard_boundary": 0,
        }:
            raise ValueError("boundary-v2 engine results changed")
    return {"model_version": BoundaryPolicy.load(artifact).version, "accepted": True,
            "test": report["test"], "engine_exact": len(SCENARIOS)}


if __name__ == "__main__":
    print(json.dumps(verify(), ensure_ascii=True, indent=2))
