"""Fail-closed package gate for the accepted boundary policy and engine replay."""
from __future__ import annotations

import argparse
import json
from collections.abc import Sequence
from pathlib import Path

from keyswitch.boundary_policy import ARTIFACT, BoundaryPolicy
from boundary_v2_corpus import verify_receipt
from context_evidence import checksum
from evaluate_boundary_engine import REPORT as ENGINE_REPORT, SCENARIOS, provenance as engine_provenance
from train_boundary_v2 import CANDIDATE, REPORT, evaluate as evaluate_test
from verify_context_v2 import read_object
from keyswitch.constants.file_formats import REPORT_JSON_INDENT


def evaluate() -> bytes:
    """Repeat the sealed numeric evaluation once the corpus receipt matches the current inputs.

    The receipt check comes first, so changed inputs never open the frozen test rows. The
    evaluation itself is the trainer's: frozen feature rows, sealed weights and threshold. This is
    regression evidence, not re-extraction, retraining, or a new holdout.
    """
    verify_receipt()
    return evaluate_test()


def verify_frozen() -> dict[str, object]:
    raw = evaluate()
    if raw != REPORT.read_bytes() or json.loads(raw).get("accepted") is not True:
        raise ValueError("boundary-v2 frozen numeric regression changed or rejected")
    return {"corpus": "boundary_v2", "frozen_numeric_regression": True,
            "scope": "Frozen numeric regression with unchanged weights; no new fit or current-runtime acceptance."}


def verify(*, artifact: Path = ARTIFACT, report_path: Path = REPORT,
           engine_path: Path = ENGINE_REPORT) -> dict[str, object]:
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
    print(json.dumps(verify_frozen() if args.verify_frozen else verify(), ensure_ascii=True, indent=REPORT_JSON_INDENT))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
