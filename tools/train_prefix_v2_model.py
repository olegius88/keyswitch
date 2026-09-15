"""Train a separate prefix candidate; no test access, installation or promotion."""
from __future__ import annotations

import argparse
from array import array
from collections import defaultdict
from collections.abc import Iterable, Mapping, Sequence
from dataclasses import asdict, dataclass, field
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import sys
from typing import cast

from action_epoch_selection import EpochSelection
from context_evidence import canonical, checksum
from reference_lexicon import reference_models
from context_optimizer import Kernel, Packed, SOURCE as OPTIMIZER
from keyswitch.context_model import ACTIONS, ContextModel
from keyswitch.prefix_schema import CURRENT_PREFIX_FEATURE_VERSION, VersionedPrefixModel
from prefix_v2_corpus import PrefixFrame, generate_frames, load_parents, provenance as corpus_provenance

ROOT = Path(__file__).resolve().parents[1]
RECIPE = ROOT / "model/prefix_v2/recipe.json"
PROFILES = ("portable", "reference_hunspell")
SPLITS = ("train", "development", "calibration")
MASS_TOLERANCE = 1e-9


def _positive(value: object, name: str) -> float:
    if type(value) not in (int, float):
        raise ValueError("invalid " + name)
    result = float(cast(float, value))
    if not math.isfinite(result) or result <= 0:
        raise ValueError("invalid " + name)
    return result


def recipe(path: Path = RECIPE) -> dict[str, object]:
    value: object = json.loads(path.read_bytes())
    if not isinstance(value, dict):
        raise ValueError("invalid prefix-v2 recipe")
    cfg = cast(dict[str, object], value)
    schema = cfg.get("runtime_feature_version")
    if (cfg.get("schema_version") != 1 or type(schema) is not int
            or schema not in (1, CURRENT_PREFIX_FEATURE_VERSION)):
        raise ValueError("unsupported prefix-v2 recipe")
    limits = cfg.get("maximum_families")
    if not isinstance(limits, dict) or set(limits) != set(SPLITS):
        raise ValueError("only train/development/calibration are allowed")
    for name, count in limits.items():
        if type(count) is not int or count <= 0:
            raise ValueError("invalid family cap: " + str(name))
    for name in ("epochs", "maximum_features", "maximum_words_per_family", "maximum_prefix_length"):
        if type(cfg.get(name)) is not int or cast(int, cfg[name]) <= 0:
            raise ValueError("invalid " + name)
    if cast(int, cfg["maximum_prefix_length"]) > 12:
        raise ValueError("prefix runtime supports at most twelve letters")
    for name in ("learning_rate", "keep_importance", "wait_importance", "minimum_feature_mass"):
        _positive(cfg.get(name), name)
    recall = _positive(cfg.get("minimum_early_recall"), "minimum_early_recall")
    if not .70 <= recall <= 1:
        raise ValueError("invalid recall")
    thresholds = cfg.get("thresholds")
    if not isinstance(thresholds, list) or not thresholds:
        raise ValueError("thresholds missing")
    parsed = [_positive(item, "threshold") for item in thresholds]
    if parsed != sorted(set(parsed)) or parsed[0] < .985 or parsed[-1] > 1:
        raise ValueError("thresholds must increase within the runtime range")
    return cfg


def feature_vocabulary(frames: Iterable[PrefixFrame], minimum: float, maximum: int) -> list[str]:
    """Kahan feature mass; family weight precedes class multipliers."""
    _positive(minimum, "minimum feature mass")
    if type(maximum) is not int or maximum < 1:
        raise ValueError("invalid feature cap")
    totals: dict[str, float] = {}
    errors: dict[str, float] = {}
    for row in frames:
        weight = _positive(row.sample_weight, "sample weight")
        for name, value in row.values.items():
            if not math.isfinite(value):
                raise ValueError("invalid feature value")
            previous = totals.get(name, 0.0)
            corrected = weight - errors.get(name, 0.0)
            total = previous + corrected
            errors[name] = (total - previous) - corrected
            totals[name] = total
    # Quantised ordering and a 1e-9 cutoff tolerate divided-parent roundoff.
    selected = sorted((name for name, mass in totals.items() if mass + MASS_TOLERANCE >= minimum),
                      key=lambda name: (-round(totals[name], 9), name))[:maximum]
    return sorted(selected)


def importance(row: PrefixFrame, cfg: Mapping[str, object]) -> float:
    factor = cfg["keep_importance"] if row.label == 0 else cfg["wait_importance"] if row.label == 2 else 1.0
    return _positive(row.sample_weight, "sample weight") * _positive(factor, "class multiplier")


@dataclass
class SequenceScore:
    profile: str
    desired: bool
    full_length: int
    family: str
    candidates: list[tuple[int, float]] = field(default_factory=list)


@dataclass(frozen=True)
class SequenceMetadata:
    key: str
    profile: str
    desired: bool
    full_length: int
    family: str
    length: int
    supported: bool


def sequence_metadata(row: PrefixFrame) -> SequenceMetadata:
    """Cache only sequence identity and existing default early execution support.

    The installed boundary path rejects nonalphabetic strokes; the prefix
    decision additionally requires four to twelve letters and no interior
    capitals. This mirrors that support, without consulting dictionary labels.
    It is not a joint engine replay or a new runtime policy.
    """
    item = row.item
    supported = (4 <= row.length <= 12 and len(item.original) == row.length
                 and len(item.alternative) == row.length and item.original.isalpha() and item.alternative.isalpha()
                 and item.source_group in (0, 1)
                 and not any(char.isupper() for char in item.original[1:] + item.alternative[1:]))
    return SequenceMetadata(row.profile + ":" + row.sequence_id, row.profile, row.desired,
                            row.full_length, row.parent_family, row.length, supported)


def _sequence(results: dict[str, SequenceScore], row: SequenceMetadata) -> SequenceScore:
    value = results.setdefault(row.key, SequenceScore(row.profile, row.desired, row.full_length, row.family))
    if (value.profile, value.desired, value.full_length, value.family) != (
            row.profile, row.desired, row.full_length, row.family):
        raise ValueError("inconsistent prefix sequence metadata")
    return value


def scores_from_probabilities(metadata: Sequence[SequenceMetadata], probabilities: Sequence[float]) -> dict[str, SequenceScore]:
    """Consume fixed CSR row order; never reconstruct lexical features per epoch."""
    if not metadata or len(probabilities) != len(metadata) * len(ACTIONS):
        raise ValueError("invalid prefix probability dimensions")
    results: dict[str, SequenceScore] = {}
    for index, row in enumerate(metadata):
        scores = probabilities[index * 4:index * 4 + 4]
        if (any(not math.isfinite(value) or not 0 <= value <= 1 for value in scores)
                or not math.isclose(sum(scores), 1., rel_tol=0., abs_tol=1e-8)):
            raise ValueError("invalid prefix probabilities")
        value = _sequence(results, row)
        if row.supported and max(range(4), key=scores.__getitem__) == 1:
            value.candidates.append((row.length, scores[1]))
    return results


def sequence_scores(model: VersionedPrefixModel, frames: Iterable[PrefixFrame]) -> dict[str, SequenceScore]:
    results: dict[str, SequenceScore] = {}
    for row in frames:
        if row.feature_version != model.feature_version:
            raise ValueError("prefix model and frame features differ")
        metadata = sequence_metadata(row)
        value = _sequence(results, metadata)
        if not metadata.supported:
            continue
        prediction = model.predict_features(row.values)
        if max(range(4), key=prediction.probabilities.__getitem__) == 1:
            value.candidates.append((row.length, prediction.probabilities[1]))
    return results


def assess_prefix_epoch(scores: Mapping[str, SequenceScore], *, thresholds: Sequence[float], loss: float) -> EpochSelection | None:
    """Choose a DEV frontier on whole sequences, requiring zero false in both profiles.

    Earlier conversion before the final letter determines recall. A conversion
    at the complete word may count for debugging, but cannot improve this rank.
    This operating threshold is not the separately calibrated serving threshold.
    """
    if not math.isfinite(loss) or loss < 0 or not thresholds:
        raise ValueError("invalid prefix development selection")
    if any(type(value) not in (float, int) or not math.isfinite(value) or not .985 <= value <= 1 for value in thresholds):
        raise ValueError("invalid prefix development threshold")
    best: EpochSelection | None = None
    for threshold in sorted(set(thresholds)):
        metrics = sequence_metrics(scores, threshold)
        if (set(metrics) != set(PROFILES) or any(not row["desired"] or row["desired"] == row["sequences"]
                                                for row in metrics.values())):
            raise ValueError("prefix development requires both classes in both lexical profiles")
        if any(row["false"] for row in metrics.values()):
            continue
        by_profile: dict[str, dict[str, int | float]] = {
            profile: {**row, "conversion_recall": row["converted_before_end"] / row["desired"]}
            for profile, row in metrics.items()
        }
        candidate = EpochSelection(float(threshold), min(row["conversion_recall"] for row in by_profile.values()),
                                   sum(row["converted_before_end"] for row in metrics.values()) /
                                   sum(row["desired"] for row in metrics.values()), loss, by_profile)
        if best is None or candidate.rank > best.rank:
            best = candidate
    return best


def sequence_metrics(scores: Mapping[str, SequenceScore], threshold: float) -> dict[str, dict[str, int]]:
    if not 0 < threshold <= 1 or not math.isfinite(threshold):
        raise ValueError("invalid sequence threshold")
    results: dict[str, dict[str, int]] = {}
    for value in scores.values():
        if value.profile not in PROFILES:
            raise ValueError("unknown lexical profile")
        counts = results.setdefault(value.profile, dict(sequences=0, desired=0, converted=0, converted_before_end=0, false=0))
        lengths = [length for length, probability in value.candidates if probability >= threshold]
        first = min(lengths) if lengths else None
        counts["sequences"] += 1
        counts["desired"] += int(value.desired)
        counts["converted"] += int(value.desired and first is not None)
        counts["converted_before_end"] += int(value.desired and first is not None and first < value.full_length)
        counts["false"] += int(not value.desired and first is not None)
    return results


def calibration_passed(metrics: Mapping[str, Mapping[str, int]], minimum_recall: float) -> bool:
    if not .70 <= minimum_recall <= 1 or set(metrics) != set(PROFILES):
        return False
    for counts in metrics.values():
        if (any(type(counts.get(key)) is not int or counts[key] < 0 for key in (
                "sequences", "desired", "converted", "converted_before_end", "false"))
                or not 0 <= counts["converted_before_end"] <= counts["converted"] <= counts["desired"] <= counts["sequences"]
                or counts["false"] > counts["sequences"] - counts["desired"]):
            return False
        if (not counts["desired"] or counts["sequences"] == counts["desired"] or counts["false"]
                or counts["converted_before_end"] / counts["desired"] < minimum_recall):
            return False
    return True


def calibrate(scores: Mapping[str, SequenceScore], thresholds: Sequence[float], minimum_recall: float) -> tuple[float, bool]:
    for threshold in thresholds:
        if calibration_passed(sequence_metrics(scores, threshold), minimum_recall):
            return threshold, True
    return 1.0, False


def artifact_payload(weights: dict[str, tuple[float, ...]], feature_version: int, threshold: float) -> dict[str, object]:
    """Use one schema namespace for training, exported checksums and runtime loading."""
    if type(feature_version) is not int or feature_version not in (1, CURRENT_PREFIX_FEATURE_VERSION):
        raise ValueError("unsupported prefix artifact features")
    if type(threshold) not in (float, int) or not .985 <= threshold <= 1:
        raise ValueError("unsafe prefix artifact threshold")
    digest = hashlib.sha256(json.dumps(weights, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()
    return {"kind": "keyswitch.prefix-policy", "feature_version": feature_version,
            "prefix_feature_version": feature_version, "actions": list(ACTIONS),
            "version": f"prefix-v{feature_version}-" + digest[:12], "conversion_threshold": threshold,
            "weights_sha256": digest, "weights": weights}


def source_provenance(recipe_path: Path = RECIPE) -> dict[str, str]:
    paths = [
        Path(__file__), recipe_path, OPTIMIZER, ROOT / "tools/context_optimizer.py",
        ROOT / "tools/prefix_v2_corpus.py", ROOT / "tests/test_prefix_v2_training.py",
        ROOT / "tools/action_epoch_selection.py", ROOT / "tests/test_action_epoch_selection.py",
        ROOT / "tests/test_prefix_model.py",
    ]
    return {**corpus_provenance(), **{path.resolve(strict=True).relative_to(ROOT).as_posix(): checksum(path)
                                    for path in paths}}


def archive_sources(output: Path, fingerprints: Mapping[str, str]) -> None:
    for name, expected in fingerprints.items():
        path = ROOT / name
        if not path.resolve(strict=True).is_relative_to(ROOT) or Path(name).is_absolute() or ".." in Path(name).parts:
            raise ValueError("invalid prefix source path")
        raw = path.read_bytes()
        if hashlib.sha256(raw).hexdigest() != expected:
            raise ValueError("prefix source changed before archive: " + name)
        target = output / "source-snapshot" / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(raw)
    (output / "source-pins.json").write_bytes(canonical(dict(fingerprints)))


def environment_snapshot() -> dict[str, object]:
    executable = Path(sys.executable).resolve(strict=True)
    libraries: dict[str, str] = {}
    maps = Path("/proc/self/maps")
    if maps.is_file():
        for line in maps.read_text().splitlines():
            name = line.rsplit(maxsplit=1)[-1]
            if name.startswith("/") and Path(name).name.startswith("libhunspell"):
                path = Path(name).resolve(strict=True)
                libraries[str(path)] = checksum(path)
    return {"python": sys.version, "executable": str(executable), "executable_sha256": checksum(executable),
            "platform": platform.platform(), "hunspell_libraries": libraries,
            "environment": {name: os.environ.get(name) for name in ("PYTHONHASHSEED", "LC_ALL", "LANG")}}


def fit(output: Path, recipe_path: Path = RECIPE) -> dict[str, object]:
    """Explicit candidate fit, preserving all existing model directories."""
    if output.exists():
        raise ValueError("refusing to overwrite a prefix experiment")
    cfg = recipe(recipe_path)
    frozen = source_provenance(recipe_path)
    output.mkdir(parents=True)
    archive_sources(output, frozen)
    limits = cast(dict[str, int], cfg["maximum_families"])
    parents = {split: load_parents(split, limits[split], cast(int, cfg["maximum_words_per_family"])) for split in SPLITS}
    families = {split: {row.family for row in selected} for split, selected in parents.items()}
    if any(families[left] & families[right] for left in SPLITS for right in SPLITS if left != right):
        raise ValueError("source physical families overlap")
    models = {profile: reference_models(profile == "reference_hunspell") for profile in PROFILES}
    environment = environment_snapshot()
    (output / "environment.json").write_bytes(canonical(environment))
    feature_version = cast(int, cfg["runtime_feature_version"])

    def frames(split: str) -> Iterable[PrefixFrame]:
        return generate_frames(parents[split], models, maximum_prefix_length=cast(int, cfg["maximum_prefix_length"]),
                               feature_version=feature_version)

    names = feature_vocabulary(frames("train"), cast(float, cfg["minimum_feature_mass"]), cast(int, cfg["maximum_features"]))
    if not names:
        raise ValueError("no prefix features survive the family-mass cutoff")
    metadata: list[SequenceMetadata] = []

    def packed_rows(split: str) -> Iterable[tuple[dict[str, float], int, float]]:
        for row in frames(split):
            if split == "development":
                metadata.append(sequence_metadata(row))
            yield row.values, row.label, importance(row, cfg)

    packed = {split: Packed.build(packed_rows(split), names)
              for split in ("train", "development")}
    if any(not value.labels for value in packed.values()):
        raise ValueError("empty training or development split")
    kernel = Kernel.load()
    environment["optimizer_library"] = {"path": str(kernel.library._name), "sha256": checksum(Path(kernel.library._name))}
    (output / "environment.json").write_bytes(canonical(environment))
    weights, accumulator = array("d", [0.0]) * (len(names) * 4), array("d", [1.0]) * (len(names) * 4)
    best, selected_epoch = array("d", weights), 0
    selected: EpochSelection | None = None
    history: list[dict[str, object]] = []
    development = packed["development"]
    denominator = math.fsum(development.importance)
    for epoch in range(cast(int, cfg["epochs"])):
        kernel.epoch(packed["train"], weights, accumulator, cast(float, cfg["learning_rate"]))
        rounded = array("d", (round(value, 9) for value in weights))
        probabilities = kernel.predict(development, rounded)
        loss = math.fsum(-development.importance[index] * math.log(max(1e-15, probabilities[index * 4 + label]))
                         for index, label in enumerate(development.labels)) / denominator
        if not math.isfinite(loss):
            raise ValueError("prefix optimisation diverged")
        candidate = assess_prefix_epoch(scores_from_probabilities(metadata, probabilities),
                                        thresholds=cast(list[float], cfg["thresholds"]), loss=loss)
        if candidate is not None and (selected is None or candidate.rank > selected.rank):
            best, selected, selected_epoch = rounded, candidate, epoch + 1
        history.append({"epoch": epoch + 1, "development_loss": loss,
                        "selection": None if candidate is None else asdict(candidate)})
        recall = None if candidate is None else candidate.minimum_recall
        print(f"prefix-v2 epoch {epoch + 1}: development loss {loss:.9f}, early recall {recall}", flush=True)
    if selected is None:
        raise ValueError("no zero-false development epoch frontier")
    learned = {name: tuple(best[index * 4 + action] for action in range(4)) for index, name in enumerate(names)}
    initial_payload = artifact_payload(learned, feature_version, 1.0)
    version = cast(str, initial_payload["version"])
    model = VersionedPrefixModel(ContextModel(learned, version), feature_version=feature_version)
    scores = sequence_scores(model, frames("calibration"))
    threshold, passed = calibrate(scores, cast(list[float], cfg["thresholds"]), cast(float, cfg["minimum_early_recall"]))
    payload = artifact_payload(learned, feature_version, threshold)
    raw = canonical(payload)
    budgets: dict[str, dict[str, float]] = {}
    pair_counts: dict[str, int] = {}
    for split in SPLITS:
        amounts: dict[str, float] = defaultdict(float)
        errors: dict[str, float] = defaultdict(float)
        pairs: set[str] = set()
        for row in frames(split):
            previous = amounts[row.parent_family]
            corrected = row.sample_weight - errors[row.parent_family]
            total = previous + corrected
            errors[row.parent_family] = (total - previous) - corrected
            amounts[row.parent_family] = total
            pairs.add(row.pair_id)
        budgets[split] = dict(sorted(amounts.items()))
        if any(abs(value - 1.0) > MASS_TOLERANCE for value in budgets[split].values()):
            raise ValueError("physical-family budget drift")
        pair_counts[split] = len(pairs)
    seal: dict[str, object] = {
        "schema_version": 1, "candidate_sha256": hashlib.sha256(raw).hexdigest(),
        "provenance": frozen, "recipe": cfg, "model_version": version, "features": len(names),
        "environment": environment,
        "epoch": selected_epoch, "development_loss": selected.loss, "threshold": threshold,
        "epoch_selection": asdict(selected), "epoch_history": history,
        "training_rows": len(packed["train"].labels), "development_rows": len(development.labels),
        "family_budgets": budgets, "pair_counts": pair_counts,
        "selected_parents": {split: [row.identifier for row in values] for split, values in parents.items()},
        "development": sequence_metrics(scores_from_probabilities(metadata, kernel.predict(development, best)), threshold),
        "calibration": sequence_metrics(scores, threshold), "calibration_passed": passed,
        "promotion_accepted": False, "independent_test_evaluated": False,
        "scope": cfg["scope"],
    }
    if source_provenance(recipe_path) != frozen:
        raise ValueError("prefix fit inputs changed")
    (output / "prefix-candidate.json").write_bytes(raw)
    loaded = VersionedPrefixModel.load(output / "prefix-candidate.json")
    if loaded.feature_version != feature_version or loaded.version != version or loaded.model.weights != learned:
        raise ValueError("prefix export and runtime model differ")
    (output / "seal.json").write_bytes(canonical(seal))
    (output / "recipe.json").write_bytes(recipe_path.read_bytes())
    return seal


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--recipe", type=Path, default=RECIPE)
    args = parser.parse_args(argv)
    seal = fit(args.output, args.recipe)
    print(json.dumps({key: seal[key] for key in ("model_version", "calibration_passed", "promotion_accepted", "independent_test_evaluated")}, indent=2))
    return 0 if seal["calibration_passed"] else 2


if __name__ == "__main__":
    raise SystemExit(main())
