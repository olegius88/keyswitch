#!/usr/bin/env python3
"""Fail closed on tampered orthotactic evidence or an unapproved artifact."""

from __future__ import annotations

import json
from pathlib import Path
from typing import cast

import ortho_corpus
import ortho_independent
import ortho_known
import ortho_prose_lexicon
import ortho_web_lexicon
from train_ortho_model import (
    ARTIFACT, CANDIDATE, CONFIG, REPORT, SEAL, checksum, config, provenance,
)

from keyswitch.ortho_model import ARTIFACT_PATH, OrthoModel
from keyswitch.constants.model_protocol import SEALED_BEFORE_TEST
from keyswitch.constants.file_formats import ORTHO_METADATA_JSON_LIMIT_BYTES, REPORT_JSON_INDENT
from keyswitch.constants.ortho import (
    ORTHO_PROSE_LEXICON_SCHEMA_VERSION,
    ORTHO_V1_EVIDENCE_SCHEMA_VERSION,
    ORTHO_WEB_LEXICON_SCHEMA_VERSION,
)


def read_object(path: Path, schema_version: int = ORTHO_V1_EVIDENCE_SCHEMA_VERSION) -> dict[str, object]:
    with path.open("rb") as handle:
        content = handle.read(ORTHO_METADATA_JSON_LIMIT_BYTES + 1)
    if len(content) > ORTHO_METADATA_JSON_LIMIT_BYTES:
        raise ValueError("oversized orthotactic metadata")
    value: object = json.loads(content)
    if not isinstance(value, dict) or value.get("schema_version") != schema_version:
        raise ValueError("invalid orthotactic metadata")
    return cast(dict[str, object], value)


def independent_matches(report: dict[str, object]) -> None:
    """A report of the independent test (`tools/ortho_independent.py`) must describe its frozen files."""

    independent = report.get("independent_test")
    if independent is None:
        return
    _rows, _known, recorded = ortho_independent.frozen()
    described = cast(dict[str, object], independent)
    counts = cast(dict[str, int], cast(dict[str, object], report["corpus_test"])["counts"])
    # The report counts the same types the receipt froze before any model was scored on them.
    if (described.get("set") != recorded.get("set")
            or described.get("receipt_sha256") != checksum(ortho_independent.RECEIPT)
            or described.get("tokens_sha256") != recorded["tokens_sha256"]
            or described.get("evidence_sha256") != recorded["evidence_sha256"]
            or described.get("source") != recorded["source"]
            or ortho_independent.population_counts(counts) != recorded["types"]):
        raise ValueError("orthotactic report does not describe the frozen independent test")


def verify(active: Path = ARTIFACT_PATH) -> dict[str, object]:
    receipt = read_object(ortho_corpus.RECEIPT)
    if receipt.get("tokens_sha256") != checksum(ortho_corpus.TOKENS):
        raise ValueError("key-space corpus does not match its receipt")
    if receipt.get("provenance") != ortho_corpus.provenance():
        raise ValueError("key-space corpus provenance changed")
    known = read_object(ortho_known.RECEIPT, 1)
    if (known.get("evidence_sha256") != checksum(ortho_known.EVIDENCE)
            or known.get("tokens_sha256") != checksum(ortho_corpus.TOKENS)
            or known.get("provenance") != ortho_known.provenance()):
        raise ValueError("dictionary evidence or its provenance changed")
    web = read_object(ortho_web_lexicon.RECEIPT, ORTHO_WEB_LEXICON_SCHEMA_VERSION)
    if web.get("forms_sha256") != checksum(ortho_web_lexicon.FORMS):
        raise ValueError("web lexicon does not match its receipt")
    prose = read_object(ortho_prose_lexicon.RECEIPT, ORTHO_PROSE_LEXICON_SCHEMA_VERSION)
    if prose.get("forms_sha256") != checksum(ortho_prose_lexicon.FORMS):
        raise ValueError("prose lexicon does not match its receipt")
    seal = read_object(SEAL)
    if (seal.get("stage") != SEALED_BEFORE_TEST or seal.get("provenance") != provenance()
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
    independent_matches(report)
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
        "evidence_scope": report["evidence_scope"],
    }


def main() -> int:
    print(json.dumps(verify(), ensure_ascii=False, indent=REPORT_JSON_INDENT))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
