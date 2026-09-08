#!/usr/bin/env python3
"""Fail closed on tampered orthotactic evidence or an unapproved artifact."""

from __future__ import annotations

import json
from pathlib import Path
from typing import cast

import ortho_corpus
from train_ortho_model import (
    ARTIFACT, CANDIDATE, CONFIG, REPORT, SEAL, checksum, config, provenance,
)

from keyswitch.ortho_model import ARTIFACT_PATH, OrthoModel


def read_object(path: Path) -> dict[str, object]:
    with path.open("rb") as handle:
        content = handle.read(4 * 1024 * 1024 + 1)
    if len(content) > 4 * 1024 * 1024:
        raise ValueError("oversized orthotactic metadata")
    value: object = json.loads(content)
    if not isinstance(value, dict) or value.get("schema_version") != 1:
        raise ValueError("invalid orthotactic metadata")
    return cast(dict[str, object], value)


def verify(active: Path = ARTIFACT_PATH) -> dict[str, object]:
    receipt = read_object(ortho_corpus.RECEIPT)
    if receipt.get("tokens_sha256") != checksum(ortho_corpus.TOKENS):
        raise ValueError("key-space corpus does not match its receipt")
    if receipt.get("provenance") != ortho_corpus.provenance():
        raise ValueError("key-space corpus provenance changed")
    seal = read_object(SEAL)
    if (seal.get("stage") != "sealed-before-test" or seal.get("provenance") != provenance()
            or seal.get("candidate_sha256") != checksum(CANDIDATE)):
        raise ValueError("orthotactic seal or provenance changed")
    if seal.get("config") != config():
        raise ValueError("orthotactic configuration changed after the seal")
    report = read_object(REPORT)
    if (report.get("seal_sha256") != checksum(SEAL)
            or report.get("candidate_sha256") != checksum(CANDIDATE)
            or report.get("thresholds") != seal.get("thresholds")):
        raise ValueError("orthotactic report does not describe this candidate")
    if report.get("promotion_passed") is not True:
        raise ValueError("orthotactic candidate did not pass its promotion gates")
    corpus = cast(dict[str, object], report["corpus_test"])
    counts = cast(dict[str, int], corpus["counts"])
    promotion = cast(dict[str, object], config()["promotion"])
    negatives = counts.get("ru:negative_types", 0) + counts.get("en:negative_types", 0)
    false_total = counts.get("ru:false_types", 0) + counts.get("en:false_types", 0)
    if negatives < cast(int, promotion["minimum_test_negatives"]):
        raise ValueError("orthotactic test population is too small to bound the risk")
    if false_total > cast(int, promotion["maximum_test_false_conversions"]):
        raise ValueError("orthotactic test reports false conversions")
    if float(cast(float, report["recall"])) < float(cast(float, promotion["minimum_recall"])):
        raise ValueError("orthotactic recall is below the promotion gate")
    if checksum(active) != checksum(CANDIDATE):
        raise ValueError("installed orthotactic model is not the evaluated candidate")
    model = OrthoModel.load(active)
    if model.version != seal.get("model_version"):
        raise ValueError("installed orthotactic identity differs from the seal")
    return {
        "schema_version": 1, "model_version": model.version,
        "artifact_sha256": checksum(active), "thresholds": seal["thresholds"],
        "recall": report["recall"], "test_negatives": negatives,
        "test_false_conversions": false_total,
        "lexical_stress_track": cast(dict[str, object], report["lexical_stress_track"])["counts"],
        "evidence_scope": report["evidence_scope"],
    }


def main() -> int:
    print(json.dumps(verify(), ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
