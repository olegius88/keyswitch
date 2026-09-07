"""Pairwise boundary learning, development selection, calibration and sealed test."""
from __future__ import annotations

import argparse
import hashlib
import json
import math
from array import array
from collections import Counter
from collections.abc import Iterator
from pathlib import Path
from typing import cast

from keyswitch.boundary_policy import ARTIFACT, FEATURE_VERSION, BoundaryPolicy
from boundary_v2_corpus import CONFIG, DIRECTORY, RECEIPT, provenance as corpus_provenance, rows
from context_evidence import canonical, checksum
from context_optimizer import Kernel, Packed, SOURCE as OPTIMIZER


CANDIDATE = DIRECTORY / "candidate.json"
SEAL = DIRECTORY / "seal.json"
REPORT = DIRECTORY / "report.json"


def provenance() -> dict[str, str]:
    return {"trainer": checksum(Path(__file__)), "optimizer": checksum(OPTIMIZER), "corpus": checksum(RECEIPT)}


def metrics(model: BoundaryPolicy, data: list[dict[str, object]]) -> dict[str, object]:
    counts: Counter[str] = Counter()
    categories: dict[str, Counter[str]] = {}
    for row in data:
        values = tuple(cast(list[dict[str, float]], row["features"]))
        labels = cast(list[int], row["labels"])
        prediction = model.predict(values)
        local: Counter[str] = Counter({"rows": 1})
        if prediction.suffix_length is None:
            local["abstained"] = 1
        elif len(labels) != 1 or prediction.suffix_length != labels[0]:
            local["errors"] = 1
        else:
            local["correct"] = 1
        counts.update(local)
        categories.setdefault(str(row["category"]), Counter()).update(local)
    return {"counts": dict(counts), "categories": {key: dict(value) for key, value in sorted(categories.items())}}


def acceptable(result: dict[str, object], *, test: bool) -> bool:
    cfg = cast(dict[str, object], json.loads(CONFIG.read_bytes()))
    gates = cast(dict[str, object], cfg["promotion"])
    counts = cast(dict[str, int], result["counts"])
    categories = cast(dict[str, dict[str, int]], result["categories"])
    if counts.get("errors", 0) > cast(int, gates["maximum_errors"]):
        return False
    for category in cast(list[str], gates["required_recall_strata"]):
        local = categories.get(category, {})
        if local.get("correct", 0) / max(1, local.get("rows", 0)) < cast(float, gates["minimum_decided_fraction_per_stratum"]):
            return False
    return not test or (counts["rows"] >= cast(int, gates["minimum_test_rows"])
                        and categories.get("word_ending", {}).get("rows", 0) >= cast(int, gates["minimum_test_word_endings"]))


def comparisons(data: list[dict[str, object]]) -> Iterator[tuple[dict[str, float], int, float]]:
    categories = Counter(str(row["category"]) for row in data)
    for row in data:
        values = cast(list[dict[str, float]], row["features"])
        labels = cast(list[int], row["labels"])
        importance = len(data) / (len(categories) * categories[str(row["category"])] * len(labels) * (len(values) - 1))
        for correct in labels:
            for other in range(len(values)):
                if correct == other:
                    continue
                names = sorted(set(values[correct]) | set(values[other]))
                difference = {name: values[correct].get(name, 0.0) - values[other].get(name, 0.0) for name in names}
                yield difference, 1, importance
                yield {name: -value for name, value in difference.items()}, 0, importance


def train() -> tuple[bytes, bytes]:
    receipt = cast(dict[str, object], json.loads(RECEIPT.read_bytes()))
    if receipt["provenance"] != corpus_provenance():
        raise ValueError("boundary corpus provenance changed")
    cfg = cast(dict[str, object], json.loads(CONFIG.read_bytes()))
    data = {split: list(rows(split)) for split in ("train", "development", "calibration")}
    names = sorted({name for row in data["train"] for values in cast(list[dict[str, float]], row["features"]) for name in values})
    packed = {split: Packed.build(comparisons(data[split]), names) for split in ("train", "development")}
    kernel = Kernel.load()
    weights, accumulators = array("d", [0.0]) * (len(names) * 4), array("d", [1.0]) * (len(names) * 4)
    best, best_loss, selected = array("d", weights), math.inf, 0
    for epoch in range(cast(int, cfg["epochs"])):
        kernel.epoch(packed["train"], weights, accumulators, cast(float, cfg["learning_rate"]))
        probabilities = kernel.predict(packed["development"], weights)
        loss = sum(-packed["development"].importance[index] * math.log(max(1e-15, probabilities[index * 4 + label]))
                   for index, label in enumerate(packed["development"].labels)) / len(packed["development"].labels)
        if loss < best_loss:
            best, best_loss, selected = array("d", weights), loss, epoch + 1
        if epoch % 10 == 0:
            print(f"boundary-v2 epoch {epoch + 1}: development loss {loss:.6f}", flush=True)
    learned = {name: round(best[index * 4 + 1] - best[index * 4], 12) for index, name in enumerate(names)}
    version = "boundary-v2-" + hashlib.sha256(canonical(learned)).hexdigest()[:12]
    calibration = []
    threshold = 1.0
    for candidate in cast(list[float], cfg["thresholds"]):
        result = metrics(BoundaryPolicy(learned, candidate, version), data["calibration"])
        calibration.append({"threshold": candidate, "result": result})
        if acceptable(result, test=False):
            threshold = candidate
            break
    model = BoundaryPolicy(learned, threshold, version)
    raw = canonical({"feature_version": FEATURE_VERSION, "version": version, "weights": learned, "threshold": threshold})
    seal = {"schema_version": 1, "candidate_sha256": hashlib.sha256(raw).hexdigest(),
            "provenance": provenance(), "epoch": selected, "training_rows": len(data["train"]),
            "development_loss": round(best_loss, 9), "development": metrics(model, data["development"]),
            "calibration": metrics(model, data["calibration"]), "threshold_search": calibration,
            "config": cfg, "scope": cfg["evidence_scope"]}
    return raw, canonical(seal)


def evaluate() -> bytes:
    seal = cast(dict[str, object], json.loads(SEAL.read_bytes()))
    receipt = cast(dict[str, object], json.loads(RECEIPT.read_bytes()))
    if (seal["candidate_sha256"] != checksum(CANDIDATE) or seal["provenance"] != provenance()
            or receipt["provenance"] != corpus_provenance() or seal["config"] != json.loads(CONFIG.read_bytes())):
        raise ValueError("boundary candidate/provenance changed after seal")
    if not acceptable(cast(dict[str, object], seal["calibration"]), test=False):
        raise ValueError("calibration gates failed; do not open the held-out test")
    result = metrics(BoundaryPolicy.load(CANDIDATE), list(rows("test")))
    return canonical({"schema_version": 1, "seal_sha256": checksum(SEAL), "candidate_sha256": checksum(CANDIDATE),
                      "test": result, "accepted": acceptable(result, test=True),
                      "scope": "First sealed synthetic segmentation evaluation on previously unused supervision families; subsequent replays are regression only."})


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group(required=True)
    for flag in ("train", "test", "verify", "promote"):
        group.add_argument("--" + flag, action="store_true")
    args = parser.parse_args()
    if args.train:
        if REPORT.exists():
            raise ValueError("test already observed; use a new experiment")
        if SEAL.exists():
            history = DIRECTORY / "development-history"
            history.mkdir(exist_ok=True)
            (history / (checksum(SEAL) + ".json")).write_bytes(canonical({
                "candidate": json.loads(CANDIDATE.read_bytes()), "seal": json.loads(SEAL.read_bytes()),
                "scope": "superseded before opening test"}))
        raw, seal = train()
        CANDIDATE.write_bytes(raw)
        SEAL.write_bytes(seal)
    elif args.test:
        if REPORT.exists():
            raise ValueError("test already observed; use --verify")
        REPORT.write_bytes(evaluate())
    elif args.verify:
        raw, seal = train()
        if raw != CANDIDATE.read_bytes() or seal != SEAL.read_bytes() or evaluate() != REPORT.read_bytes():
            raise ValueError("boundary-v2 experiment is not reproducible")
    else:
        result = evaluate()
        if result != REPORT.read_bytes() or json.loads(result)["accepted"] is not True:
            raise ValueError("boundary-v2 failed promotion gates")
        ARTIFACT.write_bytes(CANDIDATE.read_bytes())
    if REPORT.exists():
        print(REPORT.read_text())
    else:
        seal = json.loads(SEAL.read_bytes())
        print(json.dumps({"epoch": seal["epoch"], "calibration": seal["calibration"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
