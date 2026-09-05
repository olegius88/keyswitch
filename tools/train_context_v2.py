#!/usr/bin/env python3
"""Fit, seal, then independently evaluate the larger public-phrase candidate.

This tool never installs an artifact into the application. Failed candidates
remain research evidence. Promotion is a separate, reviewed release decision.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import tempfile
from array import array
from collections import Counter
from collections.abc import Iterable, Sequence
from pathlib import Path
from typing import cast

from keyswitch.context_model import ACTIONS, ARTIFACT_PATH, FEATURE_VERSION, ContextModel, extract_context_features

from context_corpus import CORPUS_ROOT, ROOT, digest
from context_evidence import CACHE, CACHE_RECEIPT, PROFILES, EvidenceValues, all_frames, canonical, checksum, evidence, load_cache
from context_frames import Frame
from context_optimizer import Kernel, Packed

CONFIG = CORPUS_ROOT / "config.json"
BASELINE = CORPUS_ROOT / "baseline-context-v1.json"
SEAL = "candidate-seal.json"
ARTIFACT = "candidate.json"
REPORT = "report.json"


def config() -> dict[str, object]:
    value: object = json.loads(CONFIG.read_bytes())
    if not isinstance(value, dict) or value.get("schema_version") != 1 or value.get("feature_version") != FEATURE_VERSION:
        raise ValueError("incompatible experiment configuration")
    return cast(dict[str, object], value)


def provenance() -> dict[str, str]:
    paths = (CONFIG, BASELINE, CACHE, CACHE_RECEIPT, CORPUS_ROOT / "corpus-receipt.json",
        CORPUS_ROOT / "sources/tatoeba-cc0-en-ru.tsv.gz", CORPUS_ROOT / "safety.json",
        ROOT / "tools/context_corpus.py", ROOT / "tools/context_frames.py",
        ROOT / "tools/context_evidence.py", Path(__file__), ROOT / "tools/context_optimizer.py",
        ROOT / "tools/context_optimizer.c", ROOT / "src/keyswitch/context_model.py",
        ROOT / "src/keyswitch/resources/models/layout_intent_v1.ksm")
    return {str(path.relative_to(ROOT)): checksum(path) for path in paths}


def audit(frames: list[Frame]) -> dict[str, object]:
    partitions = ("train", "development", "calibration", "test", "lexical_test")
    groups = {split: {row.cluster for row in frames if row.split == split} for split in partitions}
    fitted = groups["train"] | groups["development"] | groups["calibration"]
    if fitted & (groups["test"] | groups["lexical_test"]) or groups["train"] & groups["development"] or groups["train"] & groups["calibration"] or groups["development"] & groups["calibration"]:
        raise ValueError("source-group leakage")
    fit_families = {row.focus_family for row in frames if row.split in {"train", "development", "calibration"}}
    lexical_families = {row.focus_family for row in frames if row.split == "lexical_test"}
    if fit_families & lexical_families:
        raise ValueError("focus-family leakage")
    return {"rows": len(frames), "rows_by_split": dict(sorted(Counter(row.split for row in frames).items())),
        "groups_by_split": {split: len(value) for split, value in groups.items()},
        "actions": dict(sorted(Counter(row.action for row in frames).items())),
        "source_group_overlap": 0, "focus_lexical_overlap": 0,
        "lexical_scope": "unseen supervised focus family; surrounding words and external lexicons can contain it",
        "frames_sha256": hashlib.sha256(canonical([row.__dict__ for row in frames])).hexdigest()}


def samples(frames: list[Frame], split: str) -> list[tuple[Frame, str]]:
    items: list[tuple[Frame, str]] = []
    for row in frames:
        if row.split != split:
            continue
        profiles = (PROFILES[int(digest(row.identifier)[:8], 16) % 2],) if split == "train" else PROFILES
        items.extend((row, profile) for profile in profiles)
    return sorted(items, key=lambda item: digest("optimizer-order:" + item[0].identifier + ":" + item[1]))


def feature_rows(items: list[tuple[Frame, str]], cache: dict[str, EvidenceValues], importance: dict[int, float], technical: float) -> Iterable[tuple[dict[str, float], int, float]]:
    for row, profile in items:
        label = ACTIONS.index(row.action)
        multiplier = technical if row.category == "authored_technical_keep" else 1.0
        yield extract_context_features(evidence(row, profile, cache)), label, importance[label] * multiplier


def select_threshold(predictions: array[float], labels: array[int], candidates: list[float], maximum_false: int) -> tuple[float, dict[str, int]]:
    for threshold in sorted(candidates):
        false, true = 0, 0
        for row, label in enumerate(labels):
            probabilities = predictions[row * 4:row * 4 + 4]
            converted = max(range(4), key=probabilities.__getitem__) == 1 and probabilities[1] >= threshold
            false += int(converted and label != 1)
            true += int(converted and label == 1)
        if false <= maximum_false:
            return threshold, {"rows": len(labels), "false_conversions": false, "converted_correctly": true}
    # Even exact softmax saturation at 1.0 must not silently pass the gate.
    raise ValueError("no calibration threshold meets the declared safety budget")


def fit(directory: Path, frames: list[Frame], cache: dict[str, EvidenceValues]) -> dict[str, object]:
    if (directory / SEAL).exists() or (directory / ARTIFACT).exists():
        raise ValueError("candidate already sealed; do not overwrite it after evaluation")
    options = config()
    rows = samples(frames, "train")
    counts = Counter(ACTIONS.index(row.action) for row, _ in rows)
    if set(counts) != set(range(4)):
        raise ValueError("training needs all four actions")
    importance = {label: len(rows) / (4 * count) * (float(cast(float, options["keep_importance"])) if label == 0 else 1.0) for label, count in counts.items()}
    technical = float(cast(float, options["technical_importance"]))
    frequencies: Counter[str] = Counter()
    for features, _label, _weight in feature_rows(rows, cache, importance, technical):
        frequencies.update(features.keys())
    names = sorted(sorted((name for name, count in frequencies.items() if count >= int(cast(int, options["minimum_feature_rows"]))), key=lambda name: (-frequencies[name], name))[:int(cast(int, options["maximum_features"]))])
    train = Packed.build(feature_rows(rows, cache, importance, technical), names)
    development = Packed.build(feature_rows(samples(frames, "development"), cache, importance, technical), names)
    calibration = Packed.build(feature_rows(samples(frames, "calibration"), cache, importance, technical), names)
    if not development.labels or not calibration.labels:
        raise ValueError("empty development or calibration split")
    print(f"packed: train={len(train.labels)}, development={len(development.labels)}, calibration={len(calibration.labels)}, features={len(names)}", flush=True)
    kernel = Kernel.load()
    weights, accumulators = array("d", [0.0]) * (len(names) * 4), array("d", [1.0]) * (len(names) * 4)
    best, best_epoch, best_loss = array("d"), 0, math.inf
    for epoch in range(int(cast(int, options["epochs"]))):
        kernel.epoch(train, weights, accumulators, float(cast(float, options["learning_rate"])))
        probabilities = kernel.predict(development, weights)
        loss = sum(-development.importance[row] * math.log(max(1e-15, probabilities[row * 4 + label])) for row, label in enumerate(development.labels)) / len(development.labels)
        if loss < best_loss:
            best, best_epoch, best_loss = array("d", (round(value, 9) for value in weights)), epoch + 1, loss
        print(f"epoch {epoch + 1}: development_loss={loss:.9f}, best={best_epoch}", flush=True)
    threshold, calibration_metrics = select_threshold(kernel.predict(calibration, best), calibration.labels, cast(list[float], options["threshold_candidates"]), int(cast(int, options["calibration_max_false_conversions"])))
    mapping = {name: list(best[index * 4:index * 4 + 4]) for index, name in enumerate(names)}
    weight_hash = hashlib.sha256(json.dumps(mapping, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode()).hexdigest()
    # v1 names the artifact format; corpus revision is recorded in the seal.
    payload = {"actions": list(ACTIONS), "feature_version": FEATURE_VERSION, "weights": mapping,
        "weights_sha256": weight_hash, "version": "context-v1-" + weight_hash[:12], "conversion_threshold": threshold}
    directory.mkdir(parents=True, exist_ok=True)
    (directory / ARTIFACT).write_bytes(canonical(payload))
    ContextModel.load(directory / ARTIFACT)
    seal: dict[str, object] = {"schema_version": 1, "stage": "sealed-before-test", "corpus_revision": 2,
        "selected_epoch": best_epoch, "development_loss": round(best_loss, 9),
        "calibration": calibration_metrics, "conversion_threshold": threshold,
        "feature_count": len(names), "audit": audit(frames), "provenance": provenance(),
        "artifact_sha256": checksum(directory / ARTIFACT), "model_version": payload["version"]}
    (directory / SEAL).write_bytes(canonical(seal))
    print(f"sealed {payload['version']}, threshold={threshold}; test not scored", flush=True)
    return seal


def validate_seal(directory: Path) -> dict[str, object]:
    seal: object = json.loads((directory / SEAL).read_bytes())
    if not isinstance(seal, dict) or seal.get("stage") != "sealed-before-test" or seal.get("provenance") != provenance() or seal.get("artifact_sha256") != checksum(directory / ARTIFACT):
        raise ValueError("candidate seal or provenance changed")
    model = ContextModel.load(directory / ARTIFACT)
    if seal.get("model_version") != model.version or seal.get("conversion_threshold") != model.conversion_threshold:
        raise ValueError("candidate identity changed")
    return cast(dict[str, object], seal)


def metrics(model: ContextModel, frames: list[Frame], profile: str, cache: dict[str, EvidenceValues]) -> dict[str, object]:
    counts: Counter[str] = Counter()
    categories: dict[str, Counter[str]] = {}
    examples: list[dict[str, object]] = []
    for row in frames:
        item = evidence(row, profile, cache)
        prediction = model.predict(item)
        # ContextPolicy falls back to the detector for unsupported keep/convert.
        effective = item.baseline_convert if not prediction.supported and prediction.action in {"keep", "convert"} else prediction.action == "convert"
        desired = row.action == "convert"
        updates = {"rows": 1, "desired_conversions": int(desired),
            "correct_actions": int(prediction.action == row.action),
            "converted_correctly": int(effective and desired), "false_conversions": int(effective and not desired),
            "raw_converted_correctly": int(prediction.action == "convert" and desired),
            "raw_false_conversions": int(prediction.action == "convert" and not desired),
            "baseline_converted_correctly": int(item.baseline_convert and desired),
            "baseline_false_conversions": int(item.baseline_convert and not desired)}
        counts.update(updates)
        categories.setdefault(row.category, Counter()).update(updates)
        if effective and not desired and len(examples) < 12:
            examples.append({"source_id": row.identifier, "original": row.original, "alternative": row.alternative, "expected": row.action, "actual": prediction.action})
    return {"counts": dict(counts), "categories": {name: dict(value) for name, value in sorted(categories.items())}, "false_conversion_examples": examples}


def promotion_failures(candidate: dict[str, object], baseline: dict[str, object], gate: dict[str, object]) -> list[str]:
    counts = cast(dict[str, int], candidate["counts"])
    prior = cast(dict[str, int], baseline["counts"])
    negatives = counts["rows"] - counts["desired_conversions"]
    failures: list[str] = []
    if not negatives or not counts["desired_conversions"]:
        failures.append("empty evaluation class")
        return failures
    if counts["converted_correctly"] / counts["desired_conversions"] < cast(float, gate["minimum_recall"]):
        failures.append("recall below declared minimum")
    if counts["false_conversions"] / negatives > cast(float, gate["maximum_false_conversion_rate"]):
        failures.append("false conversion rate above budget")
    if counts["false_conversions"] > prior["false_conversions"]:
        failures.append("more false conversions than v1")
    if counts["converted_correctly"] < prior["converted_correctly"]:
        failures.append("fewer correct conversions than v1")
    if counts["false_conversions"] > counts["baseline_false_conversions"]:
        failures.append("more false conversions than detector")
    categories = cast(dict[str, dict[str, int]], candidate["categories"])
    if categories.get("authored_technical_keep", {}).get("false_conversions", 0) > cast(int, gate["maximum_technical_false_conversions"]):
        failures.append("technical text converted")
    return failures


def evaluate(directory: Path, frames: list[Frame], cache: dict[str, EvidenceValues], *, verify: bool = False) -> dict[str, object]:
    seal = validate_seal(directory)
    if (directory / REPORT).exists() and not verify:
        raise ValueError("test already evaluated; use --verify for identical replay, not tuning")
    candidate, baseline = ContextModel.load(directory / ARTIFACT), ContextModel.load(BASELINE)
    results: dict[str, object] = {}
    failures: dict[str, list[str]] = {}
    for split in ("test", "lexical_test"):
        selected = [row for row in frames if row.split == split]
        for profile in PROFILES:
            name = split + ":" + profile
            current, previous = metrics(candidate, selected, profile, cache), metrics(baseline, selected, profile, cache)
            results[name] = {"candidate": current, "v1": previous}
            failures[name] = promotion_failures(current, previous, cast(dict[str, object], config()["promotion"]))
            print(f"{name}: {json.dumps(current['counts'])}; gates={failures[name]}", flush=True)
    report: dict[str, object] = {"schema_version": 1, "candidate": seal["model_version"],
        "artifact_sha256": seal["artifact_sha256"], "seal_sha256": checksum(directory / SEAL),
        "audit": audit(frames), "results": results, "promotion_failures": failures,
        "promotion_passed": not any(failures.values()),
        "scope": "public CC0 phrases with synthetic layout/spelling interventions and authored safety cases; not observed user intent or full-engine quality",
        "threshold_selected_on": "calibration only, before test", "reserve_used": False}
    content = canonical(report)
    if verify:
        if (directory / REPORT).read_bytes() != content:
            raise ValueError("evaluation is not reproducible")
    else:
        (directory / REPORT).write_bytes(content)
    return report


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("fit", "evaluate", "verify"))
    parser.add_argument("--directory", type=Path, default=CORPUS_ROOT)
    args = parser.parse_args(argv)
    if not BASELINE.exists():
        if args.action != "fit":
            raise ValueError("missing frozen comparison artifact")
        BASELINE.write_bytes(ARTIFACT_PATH.read_bytes())
    frames, cache = all_frames(), load_cache()
    print(json.dumps(audit(frames), ensure_ascii=False), flush=True)
    if args.action == "fit":
        fit(args.directory, frames, cache)
    elif args.action == "evaluate":
        evaluate(args.directory, frames, cache)
    else:
        validate_seal(args.directory)
        with tempfile.TemporaryDirectory(prefix="keyswitch-context-replay-") as temporary:
            replay = Path(temporary)
            fit(replay, frames, cache)
            for filename in (ARTIFACT, SEAL):
                if (replay / filename).read_bytes() != (args.directory / filename).read_bytes():
                    raise ValueError(f"training replay differs: {filename}")
        evaluate(args.directory, frames, cache, verify=True)
        print("training and held-out evaluation replayed byte-for-byte", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
