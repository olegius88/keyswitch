#!/usr/bin/env python3
"""Check sealed boundary evidence and prevent rejected experimental rollout."""
from __future__ import annotations

import json
from pathlib import Path
from typing import cast

from keyswitch.boundary_model import ARTIFACT
from context_evidence import checksum
from train_boundary_model import DIRECTORY, provenance
from verify_context_v2 import read_object


def verify(directory: Path = DIRECTORY, active: Path = ARTIFACT) -> dict[str, object]:
    seal = read_object(directory / "seal.json")
    report = read_object(directory / "report.json")
    hashes = {relative.replace("\\", "/"): digest for relative, digest in provenance().items()}
    if seal.get("provenance") != hashes or seal.get("candidate_sha256") != checksum(directory / "candidate.json"):
        raise ValueError("boundary candidate/provenance changed")
    if report.get("seal_sha256") != checksum(directory / "seal.json") or report.get("candidate_sha256") != seal.get("candidate_sha256"):
        raise ValueError("boundary test identity changed")
    if seal.get("promotion_gates") != {"test_errors_max": 0, "test_decided_fraction_min": .80}:
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
    accepted = counts["errors"] == 0 and counts["correct"] / counts["rows"] >= .80
    if report.get("accepted") is not accepted:
        raise ValueError("boundary promotion contradicts metrics")
    if active.exists() and (not accepted or checksum(active) != seal["candidate_sha256"]):
        raise ValueError("unapproved boundary model must not ship")
    return {"schema_version": 1, "accepted": accepted, "active": active.exists(), "test": cast(dict[str, object], metrics)}


if __name__ == "__main__":
    print(json.dumps(verify(), ensure_ascii=False, indent=2))
