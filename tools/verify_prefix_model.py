"""Fast, fail-closed prefix model and exact-text evidence gate for packages."""
from __future__ import annotations

import json
from pathlib import Path
from typing import cast

from context_corpus import ROOT
from context_evidence import checksum
from keyswitch.prefix_model import ARTIFACT, PrefixModel
from prefix_corpus import DIRECTORY, PROFILES, RECEIPT, SPLITS, config, provenance as corpus_provenance
from train_prefix_model import CANDIDATE, SEAL, REPORT, accepted, provenance
from verify_context_v2 import read_object


def verify(*, require_active: bool = True, report_path: Path = REPORT,
           engine_path: Path = DIRECTORY / "engine-report.json", artifact: Path = ARTIFACT) -> dict[str, object]:
    corpus, seal, report, engine = (read_object(path) for path in (RECEIPT, SEAL, report_path, engine_path))
    if corpus.get("family_overlap") != 0 or corpus.get("profiles") != list(PROFILES) or corpus.get("provenance") != corpus_provenance():
        raise ValueError("prefix corpus provenance changed")
    if corpus.get("sha256") != {split: checksum(DIRECTORY / (split + ".jsonl.gz")) for split in SPLITS}:
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
    paths = [ROOT / "tools/evaluate_prefix_engine.py", CANDIDATE, SEAL, ROOT / "src/keyswitch/engine.py",
             ROOT / "src/keyswitch/context_policy.py", ROOT / "src/keyswitch/input_context.py",
             ROOT / "src/keyswitch/prefix_model.py", ROOT / "src/keyswitch/early_switch.py",
             ROOT / "src/keyswitch/resources/models/context_policy_v1.json", ROOT / "tests/test_input_integrity.py"]
    if engine.get("provenance") != {path.relative_to(ROOT).as_posix(): checksum(path) for path in paths}:
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
    return {"model_version": model.version, "accepted": True, "active": artifact.exists(),
            "test": cast(dict[str, object], test), "engine_passed": True}


if __name__ == "__main__":
    print(json.dumps(verify(), ensure_ascii=False, indent=2))
