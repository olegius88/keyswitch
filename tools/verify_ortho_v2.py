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

The evidence is historical. Its seal, report and candidate are anchored by the
digests below, and every source file the seal and the receipts pin - the shared
runtime module and the installed model's files among them - is checked against its
archived copy (tools/historical_sources.py), never against the live file, so a
later refactoring or retraining elsewhere cannot unsettle it. That the live tools
still reproduce the corpus, the dictionary evidence, the labels and the candidate
is shown by their own replays, which CI runs before this check.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import cast

import ortho_v2_corpus as corpus
import ortho_v2_known as known
import ortho_v2_verified as verified
from historical_sources import ORTHO_V2
from keyswitch.constants.model_protocol import SEALED_BEFORE_TEST
from train_ortho_v2 import (
    ARTIFACT, CANDIDATE, CONFIG, DIRECTORY, REPORT, SEAL, checksum, config, recorded_provenance,
)
import sys
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))
from keyswitch.constants.file_formats import ORTHO_METADATA_JSON_LIMIT_BYTES, REPORT_JSON_INDENT
from keyswitch.constants.training import ORTHO_V2_RECALL_DECIMALS

SCOPE = ("Historical rejected ortho-v2 evidence: anchored seal, report and candidate, source pins checked "
         "against archived copies; no new fit, new test or current-engine acceptance.")
# The rejected generation's evidence, byte for byte. The seal pins every other file of
# model/ortho_v2 it was built from.
ANCHORS = {
    "seal.json": "7b9fddeba806c3b38f21bed33c550fde3a3ef4bae9107dc5e356816cedb441b0",
    "candidate.json": "9bde75a980f8a69b2b1f9bcf385cacaee10d35c2e4385ef85af0f431f1ba3348",
    "report.json": "0c08a8858cbb412e727e60f0007ac9f5b78001d7f79b0d26d59ec980c4b77173",
}


def read_object(path: Path) -> dict[str, object]:
    with path.open("rb") as handle:
        content = handle.read(ORTHO_METADATA_JSON_LIMIT_BYTES + 1)
    if len(content) > ORTHO_METADATA_JSON_LIMIT_BYTES:
        raise ValueError("oversized orthotactic metadata")
    value: object = json.loads(content)
    if not isinstance(value, dict) or value.get("schema_version") != 1:
        raise ValueError("invalid orthotactic metadata")
    return cast(dict[str, object], value)


def summed(counts: dict[str, int], name: str) -> int:
    return counts.get(f"ru:{name}", 0) + counts.get(f"en:{name}", 0)


def verify_anchors(directory: Path = DIRECTORY) -> None:
    for name, expected in ANCHORS.items():
        if checksum(directory / name) != expected:
            raise ValueError("historical ortho-v2 anchor changed: " + name)


def verify() -> dict[str, object]:
    verify_anchors()
    ORTHO_V2.verify(ORTHO_V2.sources)
    receipt = read_object(corpus.RECEIPT)
    if receipt.get("tokens_sha256") != checksum(corpus.TOKENS):
        raise ValueError("key-space corpus does not match its receipt")
    corpus.recorded_provenance(receipt)
    dictionary = read_object(known.RECEIPT)
    if dictionary.get("evidence_sha256") != checksum(corpus.KNOWN_EVIDENCE):
        raise ValueError("dictionary evidence does not match its receipt")
    known.generator_digest()
    labels = read_object(verified.RECEIPT)
    if labels.get("labels_sha256") != checksum(verified.LABELS):
        raise ValueError("verified labels do not match their receipt")
    verified.tool_digests()
    seal = read_object(SEAL)
    if seal.get("stage") != SEALED_BEFORE_TEST or seal.get("candidate_sha256") != checksum(CANDIDATE):
        raise ValueError("orthotactic seal or provenance changed")
    recorded_provenance(seal)
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
                            / max(1, summed(counts, "positive_types")), ORTHO_V2_RECALL_DECIMALS),
        }
    return {
        "schema_version": 1, "model_version": seal["model_version"],
        "historical_evidence_verified": True, "current_runtime_verified": False, "scope": SCOPE,
        "promoted": False, "installed_artifact": None,
        "candidate_sha256": checksum(CANDIDATE),
        "configuration_sha256": checksum(CONFIG),
        "measured": measured,
        "why_not_promoted": report.get("not_promoted_because"),
    }


def main() -> int:
    print(json.dumps(verify(), ensure_ascii=False, indent=REPORT_JSON_INDENT))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
