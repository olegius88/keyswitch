#!/usr/bin/env python3
"""Fail closed on tampered v2 evidence, or on a v2 candidate slipping into the package.

No orthotactic v2 candidate has been promoted. The pipeline and its evidence are
kept because the measurement they produce is the correct one - four population
defects and seven rejected separations are recorded under
`model/ortho_v2/development-history/` - but every candidate built on it has been
either unsafe or no better than the model already installed. The last of them
looked like two free points of accuracy and turned `гифка` into `ubarf`.

So this verifier checks the opposite of the usual thing: that the recorded
evidence still describes the sealed candidate, and that the candidate has NOT
been installed behind the promotion rule's back.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import cast

import ortho_v2_corpus as corpus
import ortho_v2_verified as verified
from train_ortho_v2 import (
    ARTIFACT, CANDIDATE, CONFIG, REPORT, SEAL, checksum, config, provenance,
)

MAX_METADATA_BYTES = 4 * 1024 * 1024


def read_object(path: Path) -> dict[str, object]:
    with path.open("rb") as handle:
        content = handle.read(MAX_METADATA_BYTES + 1)
    if len(content) > MAX_METADATA_BYTES:
        raise ValueError("oversized orthotactic metadata")
    value: object = json.loads(content)
    if not isinstance(value, dict) or value.get("schema_version") != 1:
        raise ValueError("invalid orthotactic metadata")
    return cast(dict[str, object], value)


def summed(counts: dict[str, int], name: str) -> int:
    return counts.get(f"ru:{name}", 0) + counts.get(f"en:{name}", 0)


def verify() -> dict[str, object]:
    receipt = read_object(corpus.RECEIPT)
    if receipt.get("tokens_sha256") != checksum(corpus.TOKENS):
        raise ValueError("key-space corpus does not match its receipt")
    if receipt.get("provenance") != corpus.provenance():
        raise ValueError("key-space corpus provenance changed")
    known = read_object(corpus.KNOWN_EVIDENCE.parent / "known-receipt.json")
    if known.get("evidence_sha256") != checksum(corpus.KNOWN_EVIDENCE):
        raise ValueError("dictionary evidence does not match its receipt")
    labels = read_object(verified.RECEIPT)
    if labels.get("labels_sha256") != checksum(verified.LABELS):
        raise ValueError("verified labels do not match their receipt")
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
    if report.get("promotion_passed") is not False:
        raise ValueError(
            "a v2 candidate now passes its promotion rule; publish it deliberately "
            "and replace this verifier rather than letting it ship unnoticed"
        )
    if ARTIFACT.exists():
        raise ValueError("a v2 artifact is installed although no candidate was promoted")
    measured: dict[str, object] = {}
    for name, section in (("mutually_unseen", "mutually_unseen"),
                          ("gate", "gate_families"), ("test", "corpus_test")):
        counts = cast(dict[str, int], cast(dict[str, object], report[section])["counts"])
        measured[name] = {
            "negatives": summed(counts, "negative_types"),
            "false_types": summed(counts, "false_types"),
            "recall": round(summed(counts, "recalled_types")
                            / max(1, summed(counts, "positive_types")), 6),
        }
    return {
        "schema_version": 1, "model_version": seal["model_version"],
        "promoted": False, "installed_artifact": None,
        "candidate_sha256": checksum(CANDIDATE),
        "configuration_sha256": checksum(CONFIG),
        "measured": measured,
        "why_not_promoted": report.get("not_promoted_because"),
    }


def main() -> int:
    print(json.dumps(verify(), ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
