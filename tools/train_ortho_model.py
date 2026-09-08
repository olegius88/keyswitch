#!/usr/bin/env python3
"""Train the key-space orthotactic model by counting, then seal before testing.

Counting is the whole algorithm, which is why the result replays byte for byte:
there is no optimiser, no shuffling and no floating-point accumulation order to
preserve beyond a fixed iteration over sorted keys. Character counts come only
from the TRAIN split of the frozen key-space corpus; the calibration split picks
thresholds; the test split is opened once, after the candidate is sealed.

The model answers "was this layout wrong?" without a dictionary. Two channels:

* a character model per language over the physical key sequence, so one sequence
  has exactly two readings and their ratio is a Bayes factor over one space;
* a counted case channel, because the strongest genuine-Russian sequences under
  that ratio are abbreviations, and an abbreviation is written in capitals.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import sys
from collections import Counter
from collections.abc import Sequence
from pathlib import Path
from typing import Final, cast

sys.path.insert(0, str(Path(__file__).resolve().parent))

import ortho_corpus  # noqa: E402

from keyswitch.ortho_model import BOS, EOS, SCRIPTS, SHAPES, OrthoEvidence, OrthoModel  # noqa: E402

ROOT: Final[Path] = Path(__file__).resolve().parents[1]
DIRECTORY: Final[Path] = ROOT / "model/ortho_v1"
CONFIG: Final[Path] = DIRECTORY / "config.json"
CANDIDATE: Final[Path] = DIRECTORY / "candidate.json"
SEAL: Final[Path] = DIRECTORY / "seal.json"
REPORT: Final[Path] = DIRECTORY / "report.json"
ARTIFACT: Final[Path] = ROOT / "src/keyswitch/resources/models/ortho_v1.json"
NAMESPACE: Final[str] = "keyswitch:ortho-v1:candidate1"


def canonical(value: object) -> bytes:
    return (json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":")) + "\n").encode()


def checksum(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def config() -> dict[str, object]:
    value: object = json.loads(CONFIG.read_bytes())
    if not isinstance(value, dict) or value.get("schema_version") != 1:
        raise ValueError("incompatible orthotactic configuration")
    return cast(dict[str, object], value)


def counts(rows: list[dict[str, object]], order: int) -> tuple[
        dict[str, Counter[str]], dict[str, Counter[str]], dict[str, Counter[str]]]:
    """Character n-gram counts, context totals and context type counts per script."""

    grams: dict[str, Counter[str]] = {script: Counter() for script in SCRIPTS}
    shapes: dict[str, Counter[str]] = {script: Counter() for script in SCRIPTS}
    acronyms: dict[str, Counter[str]] = {script: Counter() for script in SCRIPTS}
    upper_keys: dict[str, set[str]] = {script: set() for script in SCRIPTS}
    for row in rows:
        if row["split"] != "train":
            continue
        shape_counts = cast(dict[str, int], row["shapes"])
        if shape_counts.get("upper"):
            upper_keys[str(row["script"])].add(str(row["keys"]))
    for row in rows:
        if row["split"] != "train":
            continue
        script, keys = str(row["script"]), str(row["keys"])
        shape_counts = cast(dict[str, int], row["shapes"])
        weight = sum(shape_counts.values())
        padded = BOS * (order - 1) + keys + EOS
        for index in range(order - 1, len(padded)):
            for size in range(1, order + 1):
                grams[script][padded[index - size + 1:index + 1]] += weight
        target = acronyms if keys in upper_keys[script] else shapes
        for name, value in shape_counts.items():
            target[script][name] += value
    return grams, shapes, acronyms


def fit_channel(gram_counts: Counter[str], order: int, discount: float,
                alphabet: int) -> tuple[dict[str, float], dict[str, float], float]:
    """Interpolated absolute discounting, stored ARPA-shaped for lookup."""

    context_total: Counter[str] = Counter()
    context_types: Counter[str] = Counter()
    for gram, count in sorted(gram_counts.items()):
        if len(gram) == 1:
            continue
        context_total[gram[:-1]] += count
        context_types[gram[:-1]] += 1
    uniform = -math.log(alphabet)
    unigram_total = sum(count for gram, count in gram_counts.items() if len(gram) == 1)
    logprob: dict[str, float] = {}
    backoff: dict[str, float] = {}
    for size in range(1, order + 1):
        for gram in sorted(gram for gram in gram_counts if len(gram) == size):
            count = gram_counts[gram]
            if size == 1:
                probability = (count + 0.5) / (unigram_total + 0.5 * alphabet)
                logprob[gram] = math.log(probability)
                continue
            context = gram[:-1]
            total = context_total[context]
            weight = discount * context_types[context] / total
            lower = math.exp(logprob[gram[1:]]) if gram[1:] in logprob else math.exp(uniform)
            probability = max(count - discount, 0.0) / total + weight * lower
            logprob[gram] = math.log(probability)
            backoff[context] = math.log(weight)
    return logprob, backoff, uniform


def quantise(value: float, scale: int) -> int:
    return int(round(value * scale))


def build(rows: list[dict[str, object]], settings: dict[str, object]) -> dict[str, object]:
    order = cast(int, settings["order"])
    discount = float(cast(float, settings["discount"]))
    scale = cast(int, settings["scale"])
    gram_counts, prose, acronym = counts(rows, order)
    models: dict[str, object] = {}
    for script in SCRIPTS:
        alphabet = len({gram for gram in gram_counts[script] if len(gram) == 1})
        logprob, backoff, uniform = fit_channel(gram_counts[script], order, discount, alphabet)
        names = sorted(logprob)
        models[script] = {
            "grams": "\n".join(names),
            "logprob": [quantise(logprob[name], scale) for name in names],
            "backoff": [[name, quantise(backoff[name], scale)] for name in sorted(backoff)],
            "uniform": quantise(uniform, scale),
        }
    payload: dict[str, object] = {
        "schema_version": 1, "order": order, "scale": scale,
        "models": models,
        "prose_shape": shape_channel(prose, scale),
        "acronym_shape": shape_channel(acronym, scale),
    }
    digest = hashlib.sha256(canonical(payload)).hexdigest()
    payload["version"] = f"ortho-v1-{digest[:12]}"
    payload["weights_sha256"] = digest
    return payload


def shape_channel(observed: dict[str, Counter[str]], scale: int) -> dict[str, object]:
    """P(shape | reading), smoothed so an unseen shape is unlikely, not impossible."""

    result: dict[str, object] = {}
    for script in SCRIPTS:
        counter = observed[script]
        total = sum(counter.values()) + 0.5 * len(SHAPES)
        result[script] = {
            shape: quantise(math.log((counter.get(shape, 0) + 0.5) / total), scale)
            for shape in SHAPES
        }
    return result


def evidence_rows(rows: list[dict[str, object]], split: str) -> list[tuple[str, str, str, int]]:
    """(script, keys, shape, weight) for every token occurrence shape in a split."""

    result: list[tuple[str, str, str, int]] = []
    for row in rows:
        if row["split"] != split:
            continue
        shape_counts = cast(dict[str, int], row["shapes"])
        for shape in sorted(shape_counts):
            result.append((str(row["script"]), str(row["keys"]), shape, shape_counts[shape]))
    return result


def governed(keys: str, shape: str, minimum_length: int) -> bool:
    """Only what the engine actually hands the model reaches an evaluation.

    The certified code guard already refuses tokens with digits, path or address
    characters, ALL-CAPS and camelCase, so a realistic capitalised abbreviation
    never reaches this model. Measuring on a population the runtime excludes
    would flatter the result.
    """

    if len(keys) < minimum_length or shape == "upper":
        return False
    return not any(character.isdigit() or character in "_/\\=:@" for character in keys)


def measure(model: OrthoModel, samples: list[tuple[str, str, str, int]],
            minimum_length: int) -> list[tuple[str, float, int, str, str]]:
    """Score each token in BOTH directions.

    The score depends only on the keys, never on the label: for a user typing in
    layout d it is log P_other(keys) - log P_d(keys). What the label decides is
    whether that token is a negative for direction d (its own script) or a
    positive (the other script, typed with the layout wrong). Scoring a token
    only in its own layout, as an earlier version of this function did, measures
    the negatives twice and never measures recall at all.
    """

    scored: list[tuple[str, float, int, str, str]] = []
    for script, keys, shape, weight in samples:
        if not governed(keys, shape, minimum_length):
            continue
        for direction in SCRIPTS:
            score = model.score(OrthoEvidence(keys, shape, direction))
            scored.append((f"{script}|{direction}", score.total, weight, keys, shape))
    return scored


def lexical_samples(minimum_length: int) -> list[tuple[str, str, str, int]]:
    """A harder, independent negative track: the frozen word-frequency lexicons.

    These lists are lowercased, so an abbreviation such as РСФСР appears without
    the capitals that would protect it in real typing. That makes this a stress
    track, deliberately harsher than what a user can produce, and it is reported
    separately rather than mixed into the corpus result.
    """

    from keyswitch.language_model import LanguageModel

    result: list[tuple[str, str, str, int]] = []
    for script, code in (("en", "en_US"), ("ru", "ru_RU")):
        for word in sorted(LanguageModel.load(code).frequencies):
            if ortho_corpus.script_of(word) != script:
                continue
            keys = ortho_corpus.to_keys(word)
            if governed(keys, "lower", minimum_length):
                result.append((script, keys, "lower", 1))
    return result


def outcome(scored: list[tuple[str, float, int, str, str]],
            thresholds: dict[str, float]) -> dict[str, object]:
    """False conversions and recall, counted by type and by occurrence."""

    counted: Counter[str] = Counter()
    worst: dict[str, list[tuple[float, str, str]]] = {script: [] for script in SCRIPTS}
    for label, total, weight, keys, shape in scored:
        script, direction = label.split("|")
        converts = total > thresholds[direction]
        if script == direction:
            # The user was in this token's own layout: converting corrupts text.
            counted[f"{direction}:negative_types"] += 1
            counted[f"{direction}:negative_occurrences"] += weight
            if converts:
                counted[f"{direction}:false_types"] += 1
                counted[f"{direction}:false_occurrences"] += weight
                worst[direction].append((total, keys, shape))
        else:
            # The token belongs to the other language: the layout was wrong.
            counted[f"{direction}:positive_types"] += 1
            counted[f"{direction}:positive_occurrences"] += weight
            if converts:
                counted[f"{direction}:recalled_types"] += 1
                counted[f"{direction}:recalled_occurrences"] += weight
    report: dict[str, object] = {"counts": dict(sorted(counted.items()))}
    report["worst_false"] = {
        script: [{"score": round(value, 4), "keys": keys, "shape": shape}
                 for value, keys, shape in sorted(items, reverse=True)[:10]]
        for script, items in worst.items()
    }
    return report


def threshold_for(scored: list[tuple[str, float, int, str, str]], script: str,
                  budget: int, margin: float) -> float:
    """Smallest threshold whose false conversions stay inside the budget, plus margin.

    Never the observed maximum of the negatives: that fits the threshold to the
    single worst sample in the calibration set and carries no margin at all.
    """

    negatives = sorted((row[1] for row in scored if row[0] == f"{script}|{script}"), reverse=True)
    if not negatives:
        raise ValueError("calibration split has no negatives for this direction")
    index = min(budget, len(negatives) - 1)
    return negatives[index] + margin


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("fit", "test", "verify", "promote"))
    arguments = parser.parse_args(argv)
    settings = config()
    rows = ortho_corpus.load_tokens()
    if checksum(ortho_corpus.TOKENS) != cast(str, cast(dict[str, object], json.loads(
            ortho_corpus.RECEIPT.read_bytes()))["tokens_sha256"]):
        raise ValueError("key-space corpus does not match its receipt")
    if arguments.command == "fit":
        if CANDIDATE.exists() or SEAL.exists():
            raise ValueError("candidate already exists; do not refit over sealed evidence")
        payload = build(rows, settings)
        # The thresholds ship inside the artifact: the runtime has no other file
        # to read them from. They are computed on calibration only, after the
        # weights are fixed, so the model identity above does not depend on them.
        payload["minimum_length"] = settings["minimum_length"]
        payload["thresholds"] = {script: 0 for script in SCRIPTS}
        CANDIDATE.write_bytes(canonical(payload))
        model = OrthoModel.load(CANDIDATE)
        calibration = measure(model, evidence_rows(rows, "calibration"),
                              cast(int, settings["minimum_length"]))
        thresholds = {
            script: threshold_for(calibration, script,
                                  cast(int, settings["calibration_false_budget"]),
                                  float(cast(float, settings["margin_nats"])))
            for script in SCRIPTS
        }
        payload["thresholds"] = {script: quantise(thresholds[script], cast(int, settings["scale"]))
                                 for script in SCRIPTS}
        CANDIDATE.write_bytes(canonical(payload))
        seal = {
            "schema_version": 1, "stage": "sealed-before-test", "namespace": NAMESPACE,
            "model_version": payload["version"], "candidate_sha256": checksum(CANDIDATE),
            "thresholds": {script: round(value, 6) for script, value in thresholds.items()},
            "config": settings, "provenance": provenance(),
            "calibration_rows": len(calibration),
        }
        SEAL.write_bytes(canonical(seal))
        print(json.dumps(seal, ensure_ascii=False, indent=2))
        return 0
    seal = cast(dict[str, object], json.loads(SEAL.read_bytes()))
    if seal.get("provenance") != provenance() or seal.get("candidate_sha256") != checksum(CANDIDATE):
        raise ValueError("candidate or provenance changed after the seal")
    model = OrthoModel.load(CANDIDATE)
    thresholds = {script: float(value) for script, value
                  in cast(dict[str, float], seal["thresholds"]).items()}
    if arguments.command == "test":
        if REPORT.exists():
            raise ValueError("test already observed; re-scoring it would not be an independent test")
        minimum = cast(int, settings["minimum_length"])
        corpus = outcome(measure(model, evidence_rows(rows, "test"), minimum), thresholds)
        lexical = outcome(measure(model, lexical_samples(minimum), minimum), thresholds)
        promotion = cast(dict[str, object], settings["promotion"])
        counts_map = cast(dict[str, int], corpus["counts"])
        negatives = (counts_map.get("ru:negative_types", 0)
                     + counts_map.get("en:negative_types", 0))
        false_total = (counts_map.get("ru:false_types", 0)
                       + counts_map.get("en:false_types", 0))
        positives = (counts_map.get("ru:positive_types", 0)
                     + counts_map.get("en:positive_types", 0))
        recall = ((counts_map.get("ru:recalled_types", 0) + counts_map.get("en:recalled_types", 0))
                  / max(1, positives))
        passed = (negatives >= cast(int, promotion["minimum_test_negatives"])
                  and false_total <= cast(int, promotion["maximum_test_false_conversions"])
                  and recall >= float(cast(float, promotion["minimum_recall"])))
        payload = {
            "schema_version": 1, "model_version": seal["model_version"],
            "seal_sha256": checksum(SEAL), "candidate_sha256": checksum(CANDIDATE),
            "thresholds": seal["thresholds"], "corpus_test": corpus,
            "lexical_stress_track": lexical, "recall": round(recall, 6),
            "promotion_passed": passed,
            "evidence_scope": (
                "synthetic wrong-layout renderings of frozen CC0 corpus tokens; "
                "the lexical track is lowercased and therefore harsher than real typing"
            ),
        }
        REPORT.write_bytes(canonical(payload))
        print(json.dumps(payload, ensure_ascii=False, indent=2)[:4000])
        return 0 if passed else 1
    if arguments.command == "verify":
        replay = build(rows, settings)
        replay["minimum_length"] = settings["minimum_length"]
        replay["thresholds"] = cast(dict[str, object], json.loads(CANDIDATE.read_bytes()))["thresholds"]
        rebuilt = canonical(replay)
        if rebuilt != CANDIDATE.read_bytes():
            raise ValueError("orthotactic candidate is not reproducible")
        print(json.dumps({"model_version": seal["model_version"], "reproducible": True}))
        return 0
    report = cast(dict[str, object], json.loads(REPORT.read_bytes()))
    if report.get("promotion_passed") is not True or report.get("candidate_sha256") != checksum(CANDIDATE):
        raise ValueError("refusing to promote a candidate without a passing report")
    ARTIFACT.write_bytes(CANDIDATE.read_bytes())
    print(json.dumps({"installed": seal["model_version"]}))
    return 0


def provenance() -> dict[str, str]:
    paths = (ROOT / "tools/train_ortho_model.py", ROOT / "tools/ortho_corpus.py",
             ROOT / "src/keyswitch/ortho_model.py", ortho_corpus.TOKENS,
             ortho_corpus.RECEIPT, CONFIG)
    return {path.relative_to(ROOT).as_posix(): checksum(path) for path in paths}


if __name__ == "__main__":
    raise SystemExit(main())
