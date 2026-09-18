"""Fast, fail-closed prefix model and exact-text evidence gate for packages."""
from __future__ import annotations

import argparse
import json
from collections.abc import Sequence
from pathlib import Path
from typing import cast

import verify_context_action_model as action_verifier
from context_corpus import ROOT
from context_evidence import checksum
from evaluate_prefix_engine import provenance as engine_provenance
from keyswitch.prefix_model import ARTIFACT, PrefixModel
from keyswitch.prefix_schema import VersionedPrefixModel
from model_protocol import ACTIVE_SPLITS, PROFILES
from prefix_corpus import DIRECTORY, RECEIPT, config
from train_prefix_model import CANDIDATE, SEAL, REPORT, accepted, evaluate, provenance
from verify_context_v2 import read_object
from verify_lexical_compatibility import verify as verify_compatibility


def verify_frozen() -> dict[str, object]:
    """Replay stored numeric features with unchanged weights; never re-fit."""
    compatibility = verify_compatibility("prefix_v1")
    raw = evaluate()
    if raw != REPORT.read_bytes() or json.loads(raw).get("accepted") is not True:
        raise ValueError("prefix frozen numeric regression changed or rejected")
    return {**compatibility, "frozen_numeric_regression": True}


def verify(*, require_active: bool = True, report_path: Path = REPORT,
           engine_path: Path = DIRECTORY / "engine-report.json", artifact: Path = ARTIFACT,
           receipt_path: Path = action_verifier.RECEIPT) -> dict[str, object]:
    """The active prefix-model gate: prefix-v1 by its frozen evidence, a later schema by the pair receipt.

    A schema-two prefix model is accepted only together with a context model, in
    one sealed evaluation, so its evidence is the context-action release receipt
    that binds both artifacts, the ledger outcome and the runtime provenance.
    """

    if artifact.exists():
        installed = VersionedPrefixModel.load(artifact)
        if installed.feature_version != 1:
            fingerprint = checksum(artifact)
            receipt = action_verifier.verify(root=ROOT, receipt_path=receipt_path, prefix_artifact=artifact)
            if checksum(artifact) != fingerprint:
                raise ValueError("active prefix artifact changed during verification")
            return {"model_version": installed.version, "feature_version": installed.feature_version, "accepted": True,
                    "active": True, "artifact_sha256": fingerprint, "engine_passed": True,
                    "evidence": "context-action release receipt",
                    "receipt_model_version": receipt["model_version"], "evidence_scope": receipt["scope"]}
    verify_compatibility("prefix_v1")
    corpus, seal, report, engine = (read_object(path) for path in (RECEIPT, SEAL, report_path, engine_path))
    if corpus.get("family_overlap") != 0 or corpus.get("profiles") != list(PROFILES):
        raise ValueError("prefix corpus provenance changed")
    if corpus.get("sha256") != {split: checksum(DIRECTORY / (split + ".jsonl.gz")) for split in ACTIVE_SPLITS}:
        raise ValueError("prefix frozen examples changed")
    if (seal.get("provenance") != provenance() or seal.get("config") != config()
            or seal.get("candidate_sha256") != checksum(CANDIDATE)):
        raise ValueError("prefix seal changed")
    model = PrefixModel.load(CANDIDATE)
    if seal.get("threshold") != model.model.conversion_threshold:
        raise ValueError("prefix threshold changed after calibration")
    if report.get("candidate_sha256") != checksum(CANDIDATE) or report.get("seal_sha256") != checksum(SEAL):
        raise ValueError("prefix test identity changed")
    test = report.get("test")
    if not isinstance(test, dict) or report.get("accepted") is not True or not accepted(test):
        raise ValueError("prefix model fails sequence quality gates")
    if engine.get("provenance") != engine_provenance():
        raise ValueError("prefix engine evidence changed")
    results = engine.get("results")
    if (engine.get("passed") is not True or engine.get("split") != "test" or not isinstance(results, dict)
            or set(results) != {profile + "/" + context for profile in PROFILES for context in ("observed", "field")}):
        raise ValueError("prefix engine quality evidence missing")
    ids = engine.get("sequence_ids")
    if not isinstance(ids, list) or len(ids) < 250 or len(set(ids)) != len(ids):
        raise ValueError("prefix engine selection incomplete")
    for variants in results.values():
        if not isinstance(variants, dict) or set(variants) != {"candidate", "shipping_no_prefix"}:
            raise ValueError("prefix engine baseline missing")
        counts_by_name: dict[str, dict[str, int]] = {}
        for name, counts in variants.items():
            if not isinstance(counts, dict) or any(type(counts.get(key)) is not int or counts[key] < 0 for key in (
                "sequences", "desired", "exact", "restored", "changed_correct", "early_restored", "length_mismatches", "injections",
            )):
                raise ValueError("invalid prefix engine counts")
            if not (counts["sequences"] == len(ids) and 0 < counts["desired"] <= counts["sequences"]
                    and counts["early_restored"] <= counts["restored"] <= counts["desired"]
                    and counts["exact"] == counts["restored"] + counts["sequences"] - counts["desired"] - counts["changed_correct"]):
                raise ValueError("inconsistent prefix engine counts")
            counts_by_name[name] = counts
        current, previous = counts_by_name["candidate"], counts_by_name["shipping_no_prefix"]
        if (current["length_mismatches"] or current["changed_correct"] > previous["changed_correct"]
                or current["restored"] < previous["restored"] or current["early_restored"] < .7 * current["desired"]):
            raise ValueError("prefix engine quality gates failed")
    regression = engine.get("runtime_regression")
    if regression is not None:
        if not isinstance(regression, dict) or not isinstance(regression.get("previous_report"), str):
            raise ValueError("invalid prefix runtime history")
        previous_path = ROOT / regression["previous_report"]
        if (not previous_path.resolve().is_relative_to(DIRECTORY / "engine-history")
                or checksum(previous_path) != regression.get("previous_sha256")
                or read_object(previous_path).get("sequence_ids") != ids):
            raise ValueError("prefix runtime history changed")
    if artifact.exists():
        if checksum(artifact) != checksum(CANDIDATE):
            raise ValueError("unapproved prefix model must not ship")
    elif require_active:
        raise ValueError("approved prefix model missing from package resources")
    return {"model_version": model.version, "feature_version": 1, "accepted": True, "active": artifact.exists(),
            "artifact_sha256": checksum(CANDIDATE), "test": cast(dict[str, object], test), "engine_passed": True}


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify-frozen", action="store_true", help="verify existing numeric evidence, without fitting or engine replay")
    args = parser.parse_args(argv)
    # Windows build pipes may use cp1252. JSON escapes preserve Unicode values
    # without requiring a Unicode-capable stdout or changing validation gates.
    print(json.dumps(verify_frozen() if args.verify_frozen else verify(), ensure_ascii=True, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
