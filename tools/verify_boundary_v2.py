"""Fail-closed package gate for the accepted boundary policy and engine replay."""
from __future__ import annotations

import argparse
import json
from collections.abc import Sequence
from pathlib import Path
from typing import cast

from keyswitch.boundary_policy import ARTIFACT, BoundaryPolicy
from boundary_v2_corpus import CONFIG, DIRECTORY, RECEIPT, SPLITS, rows
from context_evidence import canonical, checksum
from evaluate_boundary_engine import REPORT as ENGINE_REPORT, SCENARIOS, provenance as engine_provenance
from train_boundary_v2 import CANDIDATE, REPORT, SEAL, acceptable, metrics, provenance
from verify_context_v2 import read_object
from verify_lexical_compatibility import read_object as read_compatibility_object
from verify_lexical_compatibility import verify as verify_compatibility


def evaluate() -> bytes:
    """Repeat the historical numeric evaluation after audited lexical checks.

    Frozen feature rows and original metric code are used unchanged. This is
    regression evidence, not re-extraction, retraining, or a new holdout.
    """
    verify_compatibility("boundary_v2")
    seal = read_compatibility_object(SEAL)
    if (seal.get("candidate_sha256") != checksum(CANDIDATE) or seal.get("provenance") != provenance()
            or seal.get("config") != json.loads(CONFIG.read_bytes())):
        raise ValueError("boundary candidate/provenance changed after seal")
    if not acceptable(cast(dict[str, object], seal["calibration"]), test=False):
        raise ValueError("calibration gates failed; do not open the held-out test")
    result = metrics(BoundaryPolicy.load(CANDIDATE), list(rows("test")))
    return canonical({"schema_version": 1, "seal_sha256": checksum(SEAL), "candidate_sha256": checksum(CANDIDATE),
                      "test": result, "accepted": acceptable(result, test=True),
                      "scope": "First sealed synthetic segmentation evaluation on previously unused supervision families; subsequent replays are regression only."})


def verify_frozen() -> dict[str, object]:
    raw = evaluate()
    if raw != REPORT.read_bytes() or json.loads(raw).get("accepted") is not True:
        raise ValueError("boundary-v2 frozen numeric regression changed or rejected")
    return {"corpus": "boundary_v2", "compatible": True, "frozen_numeric_regression": True,
            "scope": "Frozen numeric regression with unchanged weights; no new fit or current-runtime acceptance."}


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


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify-frozen", action="store_true", help="verify existing numeric evidence, without fitting or engine replay")
    args = parser.parse_args(argv)
    print(json.dumps(verify_frozen() if args.verify_frozen else verify(), ensure_ascii=True, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
