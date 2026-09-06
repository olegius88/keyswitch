#!/usr/bin/env python3
"""Reproducible boundary model experiment on the unused public phrase reserve.

Train/development/calibration/test families are assigned before generating
punctuation variants. --train seals weights and thresholds BEFORE --test may
read test examples. --verify reproduces both; it is a regression replay, not
another independent evaluation. Never consumes private logs.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
from collections import Counter
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import cast

from keyswitch.boundary_model import ARTIFACT, FEATURE_VERSION, BoundaryModel, features
from keyswitch.layouts import LayoutPair
from context_corpus import ROOT, SOURCE_SHA, assign, canonical_tokens, load_source
from context_evidence import canonical, checksum, reference_models


DIRECTORY = ROOT / "model/boundary_v1"
CANDIDATE = DIRECTORY / "candidate.json"
SEAL = DIRECTORY / "seal.json"
REPORT = DIRECTORY / "report.json"
NAMESPACE = "keyswitch:boundary-v1:20260906"
THRESHOLDS = (0.9, 0.95, 0.98, 0.99, 0.995, 0.999, 0.9995)


@dataclass(frozen=True)
class Row:
    original: str
    alternative: str
    suffix: int
    label: int  # exact count of literal trailing characters
    family: str
    split: str


def rows(*, test: bool) -> list[Row]:
    assigned, _ = assign(load_source())
    pair = LayoutPair()
    words = sorted({word for item in assigned if item.split == "reserve"
                    for word in canonical_tokens(item.phrase.text) if word.isalpha() and 3 <= len(word) <= 24})
    result: set[Row] = set()
    for word in words:
        russian = all("а" <= char <= "я" or char == "ё" for char in word)
        english = all("a" <= char <= "z" for char in word)
        if not (russian or english):
            continue
        physical = pair.translate(word, "ru", "us") if russian else word
        # Shared physical stems keep case/yo variants and related prefixes
        # together. This is a declared string-family split, not lemmatization.
        family = physical.replace("`", "t")[:4]
        bucket = int(hashlib.sha256((NAMESPACE + family).encode()).hexdigest()[:8], 16) % 10
        split = "train" if bucket < 7 else "development" if bucket == 7 else "calibration" if bucket == 8 else "test"
        if (split == "test") != test:
            continue
        tail = len(physical) - len(physical.rstrip(",.;[]'`"))
        if russian and 0 < tail <= 8 and tail < len(physical):
            result.add(Row(physical, word, tail, 0, family, split))
        for suffix in (",", ".", ";", "]", "...", ",;", "'"):
            observed = physical + suffix
            tail = len(observed) - len(observed.rstrip(",.;[]'`"))
            if 0 < tail <= 8 and tail < len(observed):
                result.add(Row(observed, pair.translate(observed, "us", "ru"), tail, len(suffix), family, split))
    return sorted(result, key=lambda item: (item.split, item.family, item.original, item.label))


def encoded(data: list[Row]) -> list[tuple[Row, tuple[dict[str, float], ...]]]:
    models = reference_models(False)
    return [(row, tuple(features(row.original, row.alternative, suffix, models[0], models[1])
                        for suffix in range(row.suffix + 1))) for row in data]


def metrics(model: BoundaryModel, data: list[tuple[Row, tuple[dict[str, float], ...]]]) -> dict[str, object]:
    counts: Counter[str] = Counter()
    for row, values in data:
        prediction = model.predict(values)
        counts["rows"] += 1
        counts["literal" if row.label else "word"] += 1
        if prediction.suffix_length is None:
            counts["abstained"] += 1
        elif prediction.suffix_length != row.label:
            counts["errors"] += 1
            counts["lost_punctuation" if prediction.suffix_length < row.label else "split_word"] += 1
        else:
            counts["correct"] += 1
    return dict(sorted(counts.items()))


def provenance() -> dict[str, str]:
    paths = [Path(__file__), ROOT / "src/keyswitch/boundary_model.py", ROOT / "src/keyswitch/language_model.py",
             ROOT / "src/keyswitch/layouts.py", ROOT / "tools/context_corpus.py", ROOT / "tools/context_evidence.py"]
    return {str(path.relative_to(ROOT)): checksum(path) for path in paths}


def train() -> tuple[bytes, bytes]:
    data = encoded(rows(test=False))
    training = [(row, values) for row, values in data if row.split == "train"]
    development = [(row, values) for row, values in data if row.split == "development"]
    calibration = [(row, values) for row, values in data if row.split == "calibration"]
    names = sorted(training[0][1][0])
    weights, squared = dict.fromkeys(names, 0.0), dict.fromkeys(names, 1.0)
    label_counts = Counter(bool(row.label) for row, _ in training)
    best, best_loss, epoch_used = dict(weights), math.inf, 0
    for epoch in range(35):
        for row, values in training:
            probabilities = BoundaryModel(weights, 0.9, "training").probabilities(values)
            importance = len(training) / (2 * label_counts[bool(row.label)])
            for name in names:
                gradient = sum((probability - float(index == row.label)) * candidate[name]
                               for index, (candidate, probability) in enumerate(zip(values, probabilities))) * importance
                squared[name] += gradient * gradient
                weights[name] -= 0.10 * gradient / math.sqrt(squared[name])
        model = BoundaryModel(weights, 0.9, "candidate")
        loss = sum(-math.log(max(1e-15, model.probabilities(values)[row.label]))
                   / label_counts[bool(row.label)] for row, values in development)
        if loss < best_loss:
            best, best_loss, epoch_used = dict(weights), loss, epoch + 1
    best = {name: round(value, 12) for name, value in best.items()}
    threshold = THRESHOLDS[-1]
    for candidate in THRESHOLDS:
        result = metrics(BoundaryModel(best, candidate, "candidate"), calibration)
        if int(cast(int, result.get("errors", 0))) == 0:
            threshold = candidate
            break
    payload = {"feature_version": FEATURE_VERSION, "version": "boundary-v1-" + hashlib.sha256(canonical(best)).hexdigest()[:12],
               "weights": best, "threshold": threshold}
    raw = canonical(payload)
    model = BoundaryModel(best, threshold, str(payload["version"]))
    receipt = {"schema_version": 1, "source_sha256": SOURCE_SHA, "source_scope": "context-v2 unused reserve; public CC0 only",
               "provenance": provenance(), "candidate_sha256": hashlib.sha256(raw).hexdigest(),
               "namespace": NAMESPACE, "epoch": epoch_used, "training_rows": len(training),
               "development": metrics(model, development), "calibration": metrics(model, calibration),
               "family_counts": dict(Counter(split for split, _ in {(row.split, row.family) for row, _ in data})),
               "promotion_gates": {"test_errors_max": 0, "test_decided_fraction_min": 0.80},
               "limitations": "Synthetic segmentation labels; lexical resources may know held-out words. No independent real-user intent or native input evidence."}
    return raw, canonical(receipt)


def evaluate() -> bytes:
    seal = cast(dict[str, object], json.loads(SEAL.read_bytes()))
    if seal["candidate_sha256"] != checksum(CANDIDATE) or seal["provenance"] != provenance():
        raise ValueError("candidate/provenance changed after seal")
    data = encoded(rows(test=True))
    result = metrics(BoundaryModel.load(CANDIDATE), data)
    count = cast(int, result["rows"])
    accepted = result.get("errors", 0) == 0 and cast(int, result.get("correct", 0)) / count >= 0.80
    return canonical({"schema_version": 1, "seal_sha256": checksum(SEAL), "candidate_sha256": checksum(CANDIDATE),
                      "test": result, "test_families": len({row.family for row, _ in data}),
                      "accepted": accepted, "scope": "first sealed synthetic boundary test; replays are regression checks"})


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group(required=True)
    for flag in ("train", "test", "verify", "promote"):
        group.add_argument("--" + flag, action="store_true")
    args = parser.parse_args(argv)
    DIRECTORY.mkdir(parents=True, exist_ok=True)
    if args.train:
        if REPORT.exists():
            raise ValueError("test already observed; use --verify or a new experiment")
        # Development may be revised before opening the test. Keep the prior
        # pre-test seal visible; never silently rewrite an observed test run.
        if CANDIDATE.exists() and SEAL.exists():
            history = DIRECTORY / "development-history"
            history.mkdir(exist_ok=True)
            archive = history / (checksum(SEAL) + ".json")
            content = canonical({"candidate": json.loads(CANDIDATE.read_bytes()), "seal": json.loads(SEAL.read_bytes()),
                                 "stage": "superseded before any test was opened"})
            if archive.exists() and archive.read_bytes() != content:
                raise ValueError("development history collision")
            archive.write_bytes(content)
        raw, receipt = train()
        CANDIDATE.write_bytes(raw)
        SEAL.write_bytes(receipt)
    elif args.test:
        if REPORT.exists():
            raise ValueError("test already observed; use --verify")
        REPORT.write_bytes(evaluate())
    elif args.verify:
        raw, receipt = train()
        if raw != CANDIDATE.read_bytes() or receipt != SEAL.read_bytes() or evaluate() != REPORT.read_bytes():
            raise ValueError("boundary experiment is not reproducible")
    else:
        result = evaluate()
        if result != REPORT.read_bytes() or json.loads(result)["accepted"] is not True:
            raise ValueError("boundary model failed promotion gates")
        ARTIFACT.write_bytes(CANDIDATE.read_bytes())
    print("boundary experiment: " + (REPORT.read_text() if REPORT.exists() else SEAL.read_text()))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
