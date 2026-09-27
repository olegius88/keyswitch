#!/usr/bin/env python3
"""Check sealed boundary evidence and prevent rejected experimental rollout.

Boundary-v1 was rejected by its sealed test and is never trained again. Its evidence
is historical: the seal, the candidate and the report are anchored by the digests
below, and every source file the seal pins is checked against its archived copy
(tools/historical_sources.py), never against the live file, so refactoring or
retraining the files it shares with the installed models cannot unsettle it.

`--verify-frozen` also replays the training and the sealed test with the live tools
and compares candidate, seal and report byte for byte. The seal records source
hashes, and the live files have moved on since on purpose, so the replay puts the
sealed pins back in; everything else has to come out identical. It is a regression
replay of an observed test, not a new evaluation.
"""
from __future__ import annotations

import argparse
import json
from collections.abc import Sequence
from pathlib import Path
from typing import cast
from unittest.mock import patch

from keyswitch.boundary_model import ARTIFACT
from context_evidence import checksum
from historical_sources import BOUNDARY_V1
import train_boundary_model as trainer
from verify_context_v2 import read_object
from keyswitch.constants.file_formats import REPORT_JSON_INDENT
from keyswitch.constants.training import BOUNDARY_PROMOTION_MIN_DECIDED_FRACTION

DIRECTORY = trainer.DIRECTORY
SCOPE = ("Historical rejected boundary-v1 evidence: anchored seal, candidate and report, source pins checked "
         "against archived copies; a replay repeats the sealed fit and test and is no new evaluation, "
         "new test or current-engine acceptance.")
ANCHORS = {
    "seal.json": "8ef2284b990f7b8ba8a08405341db168811459b41f59ef5f5b2d32bbfae29f31",
    "candidate.json": "5462c7ffa9b43ebf584f782dd1aeffc6ffe0d2323eb3ffc733277b978d968f75",
    "report.json": "ee0c0c807d19aee57a0de6328f1206deb750b0202b26c6279066e24071bf7c89",
}


def sealed_sources(seal: dict[str, object]) -> dict[str, str]:
    """The seal's pins: exactly the archived files, each with its archived digest."""

    return BOUNDARY_V1.recorded(seal.get("provenance"), BOUNDARY_V1.sources)


def verify(directory: Path = DIRECTORY, active: Path = ARTIFACT) -> dict[str, object]:
    for name, expected in ANCHORS.items():
        if checksum(directory / name) != expected:
            raise ValueError("historical boundary-v1 anchor changed: " + name)
    seal = read_object(directory / "seal.json")
    report = read_object(directory / "report.json")
    sealed_sources(seal)
    if seal.get("candidate_sha256") != checksum(directory / "candidate.json"):
        raise ValueError("boundary candidate/provenance changed")
    if report.get("seal_sha256") != checksum(directory / "seal.json") or report.get("candidate_sha256") != seal.get("candidate_sha256"):
        raise ValueError("boundary test identity changed")
    if seal.get("promotion_gates") != {
        "test_errors_max": 0, "test_decided_fraction_min": BOUNDARY_PROMOTION_MIN_DECIDED_FRACTION
    }:
        raise ValueError("boundary promotion gates changed")
    metrics = report.get("test")
    if not isinstance(metrics, dict):
        raise ValueError("boundary test metrics missing")
    fields = ("rows", "correct", "abstained", "errors", "literal", "word", "lost_punctuation", "split_word")
    counts: dict[str, int] = {}
    for name in fields:
        value = metrics.get(name, 0)
        if type(value) is not int or value < 0:
            raise ValueError("invalid boundary count")
        counts[name] = value
    if (counts["rows"] <= 0 or counts["correct"] + counts["abstained"] + counts["errors"] != counts["rows"]
            or counts["literal"] + counts["word"] != counts["rows"]
            or counts["lost_punctuation"] + counts["split_word"] != counts["errors"]):
        raise ValueError("inconsistent boundary counts")
    accepted = counts["errors"] == 0 and counts["correct"] / counts["rows"] >= BOUNDARY_PROMOTION_MIN_DECIDED_FRACTION
    if report.get("accepted") is not accepted:
        raise ValueError("boundary promotion contradicts metrics")
    if active.exists() and (not accepted or checksum(active) != seal["candidate_sha256"]):
        raise ValueError("unapproved boundary model must not ship")
    return {"schema_version": 1, "accepted": accepted, "active": active.exists(), "test": cast(dict[str, object], metrics),
            "historical_evidence_verified": True, "current_runtime_verified": False, "scope": SCOPE}


def verify_frozen() -> dict[str, object]:
    """Replay training and the sealed test; never writes a file."""

    result = verify()
    pins = sealed_sources(read_object(trainer.SEAL))
    with patch.object(trainer, "provenance", return_value=pins):
        candidate, seal = trainer.train()
        report = trainer.evaluate()
    if (candidate != trainer.CANDIDATE.read_bytes() or seal != trainer.SEAL.read_bytes()
            or report != trainer.REPORT.read_bytes()):
        raise ValueError("boundary-v1 training or test replay changed")
    return {**result, "frozen_training_replay": True}


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify-frozen", action="store_true",
                        help="Also replay training and the sealed test with the live tools")
    args = parser.parse_args(argv)
    result = verify_frozen() if args.verify_frozen else verify()
    print(json.dumps(result, ensure_ascii=False, indent=REPORT_JSON_INDENT))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
