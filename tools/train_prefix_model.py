"""Train/seal/test a prefix-specific model; test never selects its parameters."""
from __future__ import annotations

import argparse
import hashlib
import json
import math
from array import array
from collections import Counter
from collections.abc import Sequence
from dataclasses import dataclass, field
from pathlib import Path
from typing import cast

from keyswitch.context_model import ACTIONS, ContextModel
from keyswitch.prefix_model import ARTIFACT, PREFIX_FEATURE_VERSION, PrefixModel
from context_evidence import canonical, checksum
from context_optimizer import Kernel, Packed, SOURCE as OPTIMIZER
from prefix_corpus import DIRECTORY, RECEIPT, PROFILES, config, rows, provenance as corpus_provenance


CANDIDATE = DIRECTORY / "candidate.json"
SEAL = DIRECTORY / "seal.json"
REPORT = DIRECTORY / "report.json"


@dataclass
class SequenceResult:
    text: str
    category: str
    desired: bool
    profile: str
    candidates: list[tuple[int, float]] = field(default_factory=list)
    legacy_at: int | None = None


def sequence_results(model: PrefixModel, split: str) -> list[SequenceResult]:
    results: dict[tuple[int, str], SequenceResult] = {}
    for row in rows(split):
        key = cast(int, row["sequence"]), str(row["profile"])
        result = results.setdefault(key, SequenceResult(str(row["text"]), str(row["category"]), bool(row["desired"]), str(row["profile"])))
        length = cast(int, row["length"])
        if length < 4:
            continue  # Runtime default; shorter prefixes are only wait/keep supervision.
        prediction = model.predict_features(cast(dict[str, float], row["features"]))
        best = max(range(4), key=prediction.probabilities.__getitem__)
        if ACTIONS[best] == "convert":
            result.candidates.append((length, prediction.probabilities[best]))
        if row["legacy_convert"] and result.legacy_at is None:
            result.legacy_at = length
    return list(results.values())


def metrics(data: list[SequenceResult], threshold: float) -> dict[str, object]:
    counts: Counter[str] = Counter()
    profiles: dict[str, Counter[str]] = {}
    examples: list[dict[str, object]] = []
    for result in data:
        local: Counter[str] = Counter()
        local["sequences"] = 1
        local["desired"] = int(result.desired)
        first = next((length for length, probability in result.candidates if probability >= threshold), None)
        local["converted"] = int(first is not None and result.desired)
        local["converted_before_end"] = int(first is not None and first < len(result.text) and result.desired)
        local["false"] = int(first is not None and not result.desired)
        local["technical_false"] = int(bool(local["false"]) and result.category in {"identifier", "command"})
        local["legacy_converted"] = int(result.legacy_at is not None and result.desired)
        local["legacy_false"] = int(result.legacy_at is not None and not result.desired)
        if first is not None and result.desired:
            local["characters_before_conversion"] = first
        if len(examples) < 15 and (local["false"] or result.desired and first is None):
            examples.append({"text": result.text, "category": result.category, "profile": result.profile,
                             "desired": result.desired, "first_conversion": first})
        counts.update(local)
        profiles.setdefault(result.profile, Counter()).update(local)
    return {"counts": dict(sorted(counts.items())),
            "profiles": {name: dict(sorted(value.items())) for name, value in sorted(profiles.items())},
            "examples": examples}


def provenance() -> dict[str, str]:
    return {"trainer": checksum(Path(__file__)), "optimizer": checksum(OPTIMIZER), "corpus": checksum(RECEIPT)}


def train() -> tuple[bytes, bytes]:
    if json.loads(RECEIPT.read_bytes())["provenance"] != corpus_provenance():
        raise ValueError("prefix corpus provenance changed")
    cfg = config()
    counts: Counter[str] = Counter()
    labels: Counter[int] = Counter()
    for row in rows("train"):
        counts.update(cast(dict[str, float], row["features"]).keys())
        labels[cast(int, row["label"])] += 1
    names = sorted(name for name, count in sorted(counts.items(), key=lambda item: (-item[1], item[0]))[:cast(int, cfg["maximum_features"])] if count >= 2)
    factors = {0: cast(float, cfg["keep_importance"]), 1: 1.0, 2: cast(float, cfg["wait_importance"])}
    importance = {label: sum(labels.values()) / (len(labels) * count) * factors[label] for label, count in labels.items()}
    packed = {
        split: Packed.build(((cast(dict[str, float], row["features"]), cast(int, row["label"]), importance[cast(int, row["label"])])
                             for row in rows(split)), names) for split in ("train", "development")
    }
    kernel = Kernel.load()
    weights, accumulators = array("d", [0.0]) * (len(names) * 4), array("d", [1.0]) * (len(names) * 4)
    best, best_loss, selected_epoch = array("d", weights), math.inf, 0
    for epoch in range(cast(int, cfg["epochs"])):
        kernel.epoch(packed["train"], weights, accumulators, cast(float, cfg["learning_rate"]))
        probabilities = kernel.predict(packed["development"], weights)
        loss = sum(-packed["development"].importance[row] * math.log(max(1e-15, probabilities[row * 4 + label]))
                   for row, label in enumerate(packed["development"].labels)) / len(packed["development"].labels)
        if loss < best_loss:
            best, best_loss, selected_epoch = array("d", weights), loss, epoch + 1
        if epoch % 5 == 0:
            print(f"prefix epoch {epoch + 1}: development loss {loss:.6f}", flush=True)
    learned = {name: [round(best[index * 4 + action], 9) for action in range(4)] for index, name in enumerate(names)}
    digest = hashlib.sha256(json.dumps(learned, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()
    version = "prefix-v1-" + digest[:12]
    model = PrefixModel(ContextModel({name: tuple(values) for name, values in learned.items()}, version))
    calibration = sequence_results(model, "calibration")
    threshold = 1.0
    for candidate in cast(list[float], cfg["thresholds"]):
        result = cast(dict[str, int], metrics(calibration, candidate)["counts"])
        if result["false"] == 0:
            threshold = candidate
            break
    payload = {"feature_version": PREFIX_FEATURE_VERSION, "actions": list(ACTIONS), "version": version,
               "conversion_threshold": threshold, "weights_sha256": digest, "weights": learned,
               "kind": "keyswitch.prefix-policy", "prefix_feature_version": 1}
    raw = canonical(payload)
    seal = {"schema_version": 1, "candidate_sha256": hashlib.sha256(raw).hexdigest(),
            "provenance": provenance(), "config": cfg, "features": len(names), "epoch": selected_epoch,
            "development_loss": round(best_loss, 9), "training_rows": len(packed["train"].labels),
            "development": metrics(sequence_results(model, "development"), threshold),
            "calibration": metrics(calibration, threshold), "threshold": threshold}
    return raw, canonical(seal)


def accepted(result: dict[str, object]) -> bool:
    gates = cast(dict[str, float], config()["promotion"])
    profiles = cast(dict[str, dict[str, int]], result.get("profiles", {}))
    if set(profiles) != set(PROFILES):
        return False
    for counts in profiles.values():
        if any(type(counts.get(key)) is not int or counts[key] < 0 for key in (
            "sequences", "desired", "converted", "converted_before_end", "false", "technical_false", "legacy_false",
        )):
            return False
        if not (counts["converted_before_end"] <= counts["converted"] <= counts["desired"] <= counts["sequences"]
                and counts["technical_false"] <= counts["false"] <= counts["sequences"] - counts["desired"]):
            return False
        if (counts["sequences"] < gates["minimum_test_sequences"] or not counts["desired"]
                or counts["false"] / counts["sequences"] > gates["maximum_false_sequence_rate"]
                or counts["technical_false"] > gates["maximum_technical_false_sequences"]
                or counts["converted_before_end"] / counts["desired"] < gates["minimum_wrong_layout_sequence_recall"]
                or counts["false"] - counts["legacy_false"] > gates["maximum_false_sequences_over_legacy"]):
            return False
    return True


def evaluate() -> bytes:
    seal = json.loads(SEAL.read_bytes())
    if seal["candidate_sha256"] != checksum(CANDIDATE) or seal["provenance"] != provenance() or seal["config"] != config():
        raise ValueError("prefix candidate or provenance changed after seal")
    result = metrics(sequence_results(PrefixModel.load(CANDIDATE), "test"), seal["threshold"])
    return canonical({"schema_version": 1, "candidate_sha256": checksum(CANDIDATE), "seal_sha256": checksum(SEAL),
                      "test": result, "accepted": accepted(result),
                      "scope": "First sealed synthetic prefix-sequence evaluation; repeats are regression, not a fresh test."})


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("fit", "test", "verify", "promote"))
    args = parser.parse_args(argv)
    if args.action == "fit":
        if REPORT.exists():
            raise ValueError("prefix test already observed; new experiments need a fresh holdout")
        if CANDIDATE.exists() and SEAL.exists():
            history = DIRECTORY / "development-history"
            history.mkdir(exist_ok=True)
            path = history / (checksum(SEAL) + ".json")
            content = canonical({"candidate": json.loads(CANDIDATE.read_bytes()), "seal": json.loads(SEAL.read_bytes())})
            if path.exists() and path.read_bytes() != content:
                raise ValueError("prefix development history collision")
            path.write_bytes(content)
        raw, seal = train()
        CANDIDATE.write_bytes(raw)
        SEAL.write_bytes(seal)
    elif args.action == "test":
        if REPORT.exists():
            raise ValueError("prefix test already observed; use verify")
        REPORT.write_bytes(evaluate())
    elif args.action == "verify":
        raw, seal = train()
        if raw != CANDIDATE.read_bytes() or seal != SEAL.read_bytes() or evaluate() != REPORT.read_bytes():
            raise ValueError("prefix model is not reproducible")
    else:
        from verify_prefix_model import verify
        report = evaluate()
        if report != REPORT.read_bytes() or json.loads(report)["accepted"] is not True:
            raise ValueError("prefix model failed promotion gates")
        verify(require_active=False)
        ARTIFACT.write_bytes(CANDIDATE.read_bytes())
    print((REPORT if REPORT.exists() else SEAL).read_text())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
