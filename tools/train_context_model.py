#!/usr/bin/env python3
"""Train and verify the independent contextual action policy using stdlib.

The current Layout Intent artifact and its sealed corpus are never modified
or used as training rows. Scenario groups split by physical key sequence
before variants, applications or contexts are expanded. Test labels are used
only after epoch selection on development. Re-running --verify must reproduce
the exact artifact. Reports deliberately identify this as synthetic evidence.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import time
from collections import Counter
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import cast

from keyswitch.context_model import (
    ACTIONS, ARTIFACT_PATH, FEATURE_VERSION, ContextAction, ContextEvidence,
    ContextModel, extract_context_features, softmax,
)
from keyswitch.detector import LanguageDetector
from keyswitch.input_context import FieldContext
from keyswitch.intent_model import LinearNgramModel
from keyswitch.language_model import LanguageModel
from keyswitch.layouts import LayoutPair


ROOT = Path(__file__).resolve().parents[1]
SCENARIOS = ROOT / "model/context_v1/scenarios.json"
HOLDOUT = ROOT / "model/context_v1/holdout-2.json"
REPORT = ROOT / "model/context_v1/report.json"
NAMESPACE = "keyswitch:context-v1:candidate2"
EPOCHS = 70


@dataclass(frozen=True)
class Row:
    evidence: ContextEvidence
    action: ContextAction
    family: str
    split: str
    category: str


def canonical(value: object) -> bytes:
    return (json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":")) + "\n").encode()


def family_split(signature: str) -> str:
    bucket = int(hashlib.sha256((NAMESPACE + ":" + signature).encode()).hexdigest()[:8], 16) % 10
    return "train" if bucket < 8 else "development"


def build_corpus(source_path: Path = SCENARIOS, *, held_out: bool = False) -> list[Row]:
    payload: object = json.loads(source_path.read_bytes())
    if not isinstance(payload, dict) or payload.get("schema_version") != 1:
        raise ValueError("invalid scenarios")
    pair = LayoutPair()
    intent, _status = LinearNgramModel.try_load_default()
    if intent is None:
        raise ValueError("baseline model unavailable")
    models = {0: LanguageModel.load("en_US"), 1: LanguageModel.load("ru_RU")}
    detector = LanguageDetector(models, intent)
    rows: list[Row] = []
    cache: dict[tuple[str, int, str], tuple[bool, bool, bool, float]] = {}

    def add(word: str, group: int, before: str, after: str, app: str, role: str,
            trigger: str, action: ContextAction, category: str) -> None:
        from keyswitch.input_context import FieldRole
        from keyswitch.intent_model import CorrectionTrigger

        alternate = pair.translate(word, "us" if group == 0 else "ru", "ru" if group == 0 else "us")
        signature = min(word.casefold(), alternate.casefold())
        key = (word, group, trigger)
        if key not in cache:
            decision = detector.decide(word, {1 - group: alternate}, group, trigger=cast(CorrectionTrigger, trigger))
            source, target = models[group].score(word), models[1 - group].score(alternate)
            cache[key] = decision.should_convert, source.known, target.known, target.value - source.value
        baseline, source_known, target_known, delta = cache[key]
        item = ContextEvidence(
            word, alternate, group, FieldContext(app, "training-field", before, after, cast(FieldRole, role)),
            trigger, baseline, source_known, target_known, delta,
        )
        rows.append(Row(item, action, signature, "test" if held_out else family_split(signature), category))

    for name, group in (("russian", 1), ("english", 0), ("short_russian", 1)):
        words: object = payload.get(name)
        if not isinstance(words, list) or any(not isinstance(word, str) for word in words):
            raise ValueError("invalid scenario words")
        for word in cast(list[str], words):
            wrong = pair.translate(word, "ru" if group == 1 else "us", "us" if group == 1 else "ru")
            contexts = ("", "я думаю что ", "подскажи пожалуйста ", "мы обсуждали это вчера ") if group == 1 else ("", "I think that ", "could you please ", "we discussed this yesterday ")
            for before in contexts:
                for app, role in (("Telegram", "text"), ("chrome", "text"), ("Code", "text"), ("TestEditor", "unknown")):
                    for trigger in ("space", "pause", "enter", "punctuation"):
                        action: ContextAction = "convert"
                        if name == "short_russian" and not before:
                            action = "suggest" if trigger in {"enter", "punctuation"} else "wait"
                        add(wrong, 1 - group, before, "", app, role, trigger, action, name + "_wrong")
                        # Standalone Latin letters may be variables. Correct
                        # short Russian words, like all valid prose, stay put.
                        add(word, group, before, "", app, role, trigger, "keep", name + "_correct")
            if name == "short_russian":
                for following in ("этого достаточно", "следующего сообщения", "сегодня всё получилось", "завтра продолжим"):
                    for app, role in (("Telegram", "text"), ("chrome", "text"), ("Code", "text"), ("TestEditor", "unknown")):
                        add(wrong, 0, "", following, app, role, "space", "convert", "short_lookahead")
            # App identity must not override the actual language of comments.
            add(wrong, 1 - group, "// " + contexts[1], "", "Code", "code", "space", "convert", "code_comment")
            # A legitimate English insertion inside Russian prose is not a
            # layout error, even when the surrounding sentence is Russian.
            if group == 0:
                add(word, 0, "в сообщении написано ", "", "Telegram", "text", "space", "keep", "mixed_prose")

    technical: object = payload.get("technical")
    if not isinstance(technical, list) or any(not isinstance(word, str) for word in technical):
        raise ValueError("invalid technical scenarios")
    for token in cast(list[str], technical):
        for before in ("запусти ", "введи команду ", "const value = ", "return ", "$ ", "the command is "):
            for app, role in (("Code", "code"), ("WindowsTerminal", "terminal"), ("Telegram", "text")):
                for trigger in ("space", "pause", "enter", "punctuation"):
                    add(token, 0, before, "", app, role, trigger, "keep", "technical")
    return rows


def train(rows: list[Row]) -> tuple[dict[str, list[float]], int, float]:
    train_rows = [(extract_context_features(row.evidence), ACTIONS.index(row.action)) for row in rows if row.split == "train"]
    development = [(extract_context_features(row.evidence), ACTIONS.index(row.action)) for row in rows if row.split == "development"]
    if not train_rows or not development:
        raise ValueError("empty training or development split")
    names = sorted({name for features, _label in train_rows for name in features})
    weights = {name: [0.0] * 4 for name in names}
    accumulators = {name: [1.0] * 4 for name in names}
    label_counts = Counter(label for _features, label in train_rows)
    importance_by_label = {label: len(train_rows) / (4 * count) * (2.0 if label == 0 else 1.0) for label, count in label_counts.items()}
    best: dict[str, list[float]] = {}
    best_loss, best_epoch = math.inf, 0
    # Fixed order and optimizer parameters; test never selects an epoch.
    for epoch in range(EPOCHS):
        for features, label in train_rows:
            scores = [0.0] * 4
            for name, value in features.items():
                for index, weight in enumerate(weights[name]):
                    scores[index] += weight * value
            probabilities = softmax(scores)
            importance = importance_by_label[label]
            for name, value in features.items():
                vector, squared = weights[name], accumulators[name]
                for index in range(4):
                    gradient = importance * (probabilities[index] - float(index == label)) * value
                    squared[index] += gradient * gradient
                    vector[index] -= 0.12 * gradient / math.sqrt(squared[index])
        loss = 0.0
        for features, label in development:
            scores = [0.0] * 4
            for name, value in features.items():
                for index, weight in enumerate(weights.get(name, (0.0, 0.0, 0.0, 0.0))):
                    scores[index] += weight * value
            loss -= importance_by_label[label] * math.log(max(1e-15, softmax(scores)[label]))
        loss /= len(development)
        if loss < best_loss:
            best_loss, best_epoch = loss, epoch + 1
            best = {name: [round(value, 9) for value in vector] for name, vector in weights.items()}
    return best, best_epoch, best_loss


def evaluate(model: ContextModel, rows: list[Row], split: str) -> dict[str, object]:
    counts: Counter[str] = Counter()
    by_category: dict[str, Counter[str]] = {}
    failures: list[dict[str, object]] = []
    for row in rows:
        if row.split != split:
            continue
        prediction = model.predict(row.evidence)
        counts["rows"] += 1
        counts["correct_actions"] += int(prediction.action == row.action)
        counts["desired_conversions"] += int(row.action == "convert")
        counts["converted_correctly"] += int(prediction.action == row.action == "convert")
        counts["false_conversions"] += int(prediction.action == "convert" and row.action != "convert")
        counts["baseline_false_conversions"] += int(row.evidence.baseline_convert and row.action != "convert")
        counts["baseline_converted_correctly"] += int(row.evidence.baseline_convert and row.action == "convert")
        category = by_category.setdefault(row.category, Counter())
        category["rows"] += 1
        category["correct_actions"] += int(prediction.action == row.action)
        category["false_conversions"] += int(prediction.action == "convert" and row.action != "convert")
        if prediction.action != row.action and len(failures) < 30:
            failures.append({"token": row.evidence.original, "category": row.category, "expected": row.action, "actual": prediction.action})
    return {"counts": dict(counts), "categories": {name: dict(value) for name, value in sorted(by_category.items())}, "examples": failures}


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify", action="store_true")
    parser.add_argument("--artifact", type=Path, default=ARTIFACT_PATH)
    parser.add_argument("--report", type=Path, default=REPORT)
    parser.add_argument("--development-only", action="store_true")
    args = parser.parse_args(argv)
    rows = build_corpus()
    weights, epoch, loss = train(rows)
    digest = hashlib.sha256(json.dumps(weights, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()
    payload = {"feature_version": FEATURE_VERSION, "actions": list(ACTIONS), "version": "context-v1-" + digest[:12], "conversion_threshold": 0.985, "weights_sha256": digest, "weights": weights}
    model = ContextModel({name: tuple(value) for name, value in weights.items()}, "context-v1-" + digest[:12])
    # The development-only path neither reads nor scores reserved test rows.
    if not args.development_only:
        rows += build_corpus(HOLDOUT, held_out=True)
    groups = {split: {row.family for row in rows if row.split == split} for split in ("train", "development", "test")}
    overlap = len(groups["train"] & groups["test"]) + len(groups["development"] & groups["test"])
    test = evaluate(model, rows, "test")
    counts = cast(dict[str, int], test["counts"])
    if overlap:
        raise ValueError("physical token leakage into context holdout")
    passed = not args.development_only and counts.get("false_conversions", 0) == 0 and counts.get("converted_correctly", 0) >= counts.get("baseline_converted_correctly", 0)
    report = {
        "schema_version": 1, "model_version": model.version, "evidence_scope": "author-created-synthetic-scenarios-not-real-world-quality",
        "split_namespace": NAMESPACE, "family_counts": {name: len(value) for name, value in groups.items()},
        "test_overlap": overlap, "selected_epoch": epoch, "development_loss": round(loss, 9),
        "sources_sha256": hashlib.sha256(SCENARIOS.read_bytes()).hexdigest(),
        "holdout_sha256": None if args.development_only else hashlib.sha256(HOLDOUT.read_bytes()).hexdigest(),
        "runtime_sha256": hashlib.sha256((ROOT / "src/keyswitch/context_model.py").read_bytes()).hexdigest(),
        "trainer_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "baseline_sha256": hashlib.sha256((ROOT / "src/keyswitch/resources/models/layout_intent_v1.ksm").read_bytes()).hexdigest(),
        "artifact_sha256": hashlib.sha256(canonical(payload)).hexdigest(),
        "baseline_scope": "isolated-token LanguageDetector, not full application",
        "development": evaluate(model, rows, "development"), "test": test, "quality_gates_passed": passed,
    }
    artifact_bytes, report_bytes = canonical(payload), canonical(report)
    if args.verify:
        if args.artifact.read_bytes() != artifact_bytes or args.report.read_bytes() != report_bytes:
            raise ValueError("context model or evidence is not reproducible")
    else:
        args.artifact.parent.mkdir(parents=True, exist_ok=True)
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.artifact.write_bytes(artifact_bytes)
        args.report.write_bytes(report_bytes)
    print(json.dumps({"model": model.version, "quality_gates_passed": passed, "test": test, "selected_epoch": epoch}, ensure_ascii=False, indent=2))
    return 0 if passed or args.development_only else 1


if __name__ == "__main__":
    raise SystemExit(main())
