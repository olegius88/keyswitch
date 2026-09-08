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
from keyswitch.short_words import (
    TRUSTED_SHORT_WORDS, natural_short_source_veto, trusted_short_word_decision,
)


ROOT = Path(__file__).resolve().parents[1]
SCENARIOS = ROOT / "model/context_v1/scenarios.json"
HOLDOUT = ROOT / "model/context_v1/holdout-3.json"
REPORT = ROOT / "model/context_v1/report.json"
NAMESPACE = "keyswitch:context-v1:candidate3"
EPOCHS = 70
KEEP_IMPORTANCE = 1.0
LEARNING_RATE = 0.2
CONVERSION_THRESHOLD = 0.985
# Without field reading the engine reports role `unknown` for every real
# application, so each application appears both with a declared role and with
# `unknown`. Pairing `unknown` only with a stand-in editor left the serving
# combination unseen.
FIELDS = (("Telegram", "text"), ("Code", "text"), ("TestEditor", "unknown"),
          ("Telegram", "unknown"), ("chrome", "unknown"), ("Code", "unknown"))
TERMINAL_FIELDS = (("Code", "code"), ("WindowsTerminal", "terminal"),
                   ("Telegram", "unknown"), ("Code", "unknown"))


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
    cache: dict[tuple[str, int, str, int | None], tuple[bool, bool, bool, float]] = {}

    def context_group_of(text: str) -> int | None:
        """The engine remembers the layout of the previous word, not its text."""

        lowered = text.casefold()
        russian = sum("а" <= char <= "я" or char == "ё" for char in lowered)
        english = sum("a" <= char <= "z" for char in lowered)
        return 1 if russian > english else 0 if english > russian else None

    def baseline_decision(word: str, group: int, alternate: str, trigger: str,
                          context_group: int | None) -> bool:
        """Reproduce exactly what `engine._decide_word` hands to the policy.

        The engine applies the short-word veto and the curated trusted
        short-word override before the model runs, so a corpus built from the
        bare detector teaches the model to reject decisions it never sees.
        """

        from keyswitch.intent_model import CorrectionTrigger

        decision = detector.decide(word, {1 - group: alternate}, group, trigger=cast(CorrectionTrigger, trigger))
        decision = natural_short_source_veto(decision, context_group=context_group)
        if not decision.should_convert:
            override = trusted_short_word_decision(
                detector, word, {1 - group: alternate}, group,
                ignored_words=(), rejected_targets=frozenset(), protect_code=True,
                context_group=context_group,
            )
            if override is not None:
                decision = override
        return decision.should_convert

    def baseline_for(word: str, group: int, before: str, trigger: str) -> bool:
        alternate = pair.translate(word, "us" if group == 0 else "ru", "ru" if group == 0 else "us")
        key = (word, group, trigger, context_group_of(before))
        if key not in cache:
            cache[key] = (
                baseline_decision(word, group, alternate, trigger, context_group_of(before)),
                models[group].score(word).known, models[1 - group].score(alternate).known,
                models[1 - group].score(alternate).value - models[group].score(word).value,
            )
        return cache[key][0]

    def add(word: str, group: int, before: str, after: str, app: str, role: str,
            trigger: str, action: ContextAction, category: str) -> None:
        from keyswitch.input_context import FieldRole

        alternate = pair.translate(word, "us" if group == 0 else "ru", "ru" if group == 0 else "us")
        signature = min(word.casefold(), alternate.casefold())
        baseline_for(word, group, before, trigger)
        baseline, source_known, target_known, delta = cache[(word, group, trigger, context_group_of(before))]
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
            contexts: tuple[str, ...] = ("", "я думаю что ", "подскажи пожалуйста ", "мы обсуждали это вчера ") if group == 1 else ("", "I think that ", "could you please ", "we discussed this yesterday ")
            if name == "short_russian":
                # One- and two-letter words carry the ambiguity the whole policy
                # rests on, so they keep their share of the corpus as it grows.
                contexts += ("но ", "мне кажется ", "давай ", "сегодня ")
            for before in contexts:
                for app, role in FIELDS:
                    for trigger in ("space", "pause", "enter", "punctuation"):
                        action: ContextAction = "convert"
                        # A short word without context stays ambiguous only when
                        # the curated trusted list did not already decide it.
                        if name == "short_russian" and not before and not baseline_for(wrong, 1 - group, "", trigger):
                            action = "suggest" if trigger in {"enter", "punctuation"} else "wait"
                        add(wrong, 1 - group, before, "", app, role, trigger, action, name + "_wrong")
                        # Standalone Latin letters may be variables. Correct
                        # short Russian words, like all valid prose, stay put.
                        add(word, group, before, "", app, role, trigger, "keep", name + "_correct")
            if name == "short_russian":
                for following in ("этого достаточно", "следующего сообщения", "сегодня всё получилось",
                                  "завтра продолжим", "меня всё устраивает", "нас это не касается",
                                  "тебя ждут в офисе", "него другое мнение", "вас получилось лучше",
                                  "них уже есть решение"):
                    for app, role in FIELDS:
                        add(wrong, 0, "", following, app, role, "space", "convert", "short_lookahead")
            # App identity must not override the actual language of comments.
            add(wrong, 1 - group, "// " + contexts[1], "", "Code", "code", "space", "convert", "code_comment")
            # A legitimate English insertion inside Russian prose is not a
            # layout error, even when the surrounding sentence is Russian.
            if group == 0:
                add(word, 0, "в сообщении написано ", "", "Telegram", "text", "space", "keep", "mixed_prose")

    # The runtime's curated trusted short-word list decides a handful of
    # two-letter tokens on its own, in both directions. Mirroring it here keeps
    # the model from vetoing a decision the engine has already made. The list
    # is code, not corpus data, so it belongs to training only; it can never
    # form an independent test family.
    if not held_out:
        for target_group, trusted in sorted(TRUSTED_SHORT_WORDS.items()):
            for word in sorted(trusted):
                wrong = pair.translate(word, "us" if target_group == 0 else "ru", "ru" if target_group == 0 else "us")
                phrases = ("", "я думаю что ", "мы обсуждали это вчера ") if target_group == 1 else ("", "I think that ", "we discussed this yesterday ")
                for before in phrases:
                    for app, role in FIELDS:
                        for trigger in ("space", "pause", "enter", "punctuation"):
                            converts = baseline_for(wrong, 1 - target_group, before, trigger)
                            action = "convert" if converts or before else (
                                "suggest" if trigger in {"enter", "punctuation"} else "wait")
                            add(wrong, 1 - target_group, before, "", app, role, trigger, cast(ContextAction, action), "trusted_short_wrong")
                            add(word, target_group, before, "", app, role, trigger, "keep", "trusted_short_correct")

    technical: object = payload.get("technical")
    if not isinstance(technical, list) or any(not isinstance(word, str) for word in technical):
        raise ValueError("invalid technical scenarios")
    for token in cast(list[str], technical):
        for before in ("запусти ", "введи команду ", "const value = ", "return ", "$ ", "the command is "):
            for app, role in TERMINAL_FIELDS:
                for trigger in ("space", "pause", "enter", "punctuation"):
                    add(token, 0, before, "", app, role, trigger, "keep", "technical")

    def unknown_family(name: str, group: int, contexts: tuple[str, ...], apps: tuple[tuple[str, str], ...]) -> None:
        """Tokens outside both lexicons: the physical form decides, not a dictionary.

        A technical term, dotfile, jargon word or misspelling typed in the wrong
        layout is still a layout error; the same token typed as intended stays.
        Being absent from a dictionary is not itself a reason to hesitate.
        Only a one- or two-character token stays ambiguous without context,
        exactly as a short Russian word does.
        """

        tokens: object = payload.get(name, [])
        if not isinstance(tokens, list) or any(not isinstance(word, str) for word in tokens):
            raise ValueError(f"invalid {name} scenarios")
        for token in cast(list[str], tokens):
            wrong = pair.translate(token, "us" if group == 0 else "ru", "ru" if group == 0 else "us")
            for before in contexts:
                for app, role in apps:
                    for trigger in ("space", "pause", "enter", "punctuation"):
                        action: ContextAction = "convert"
                        if len(token) <= 2 and not before and not baseline_for(wrong, 1 - group, "", trigger):
                            action = "suggest" if trigger in {"enter", "punctuation"} else "wait"
                        add(wrong, 1 - group, before, "", app, role, trigger, action, name + "_wrong")
                        add(token, group, before, "", app, role, trigger, "keep", name + "_correct")

    terminal_apps = TERMINAL_FIELDS
    unknown_family("technical_terms", 0, ("", "запусти ", "далее запускается процесс ", "$ "), terminal_apps)
    unknown_family("dotted", 0, ("", "открой файл ", "нужно поправить ", "$ cat "), terminal_apps)
    unknown_family("english_unknown", 0, ("", "I think that ", "could you please ", "в сообщении написано "), FIELDS)
    unknown_family("russian_unknown", 1, ("", "я думаю что ", "нужно срочно ", "мы обсуждали это вчера ", "подскажи пожалуйста "), FIELDS)

    # A slash is punctuation in both layouts, so the engine hands the word after
    # it to the model separately. The language of the fragment before the slash
    # does not decide: a wrong-layout word after `bild/` or after `да/` is still
    # a layout error, and a component that is valid in the layout it was typed
    # in stays either way.
    heads: object = payload.get("slash_heads", {})
    if not isinstance(heads, dict):
        raise ValueError("invalid slash scenarios")
    prefixes: list[str] = []
    for name in ("russian", "english"):
        value: object = heads.get(name, [])
        if not isinstance(value, list) or any(not isinstance(prefix, str) for prefix in value):
            raise ValueError("invalid slash scenarios")
        prefixes += cast(list[str], value)
    for name, group in (("russian", 1), ("english", 0), ("technical_terms", 0)):
        for word in cast(list[str], payload[name])[:10]:
            wrong = pair.translate(word, "ru" if group == 1 else "us", "us" if group == 1 else "ru")
            for before in prefixes:
                for app, role in (("Telegram", "text"), ("Code", "code"), ("Telegram", "unknown"), ("Code", "unknown")):
                    add(wrong, 1 - group, before, "", app, role, "space", "convert", "slash_wrong")
                    add(word, group, before, "", app, role, "space", "keep", "slash_correct")
    return rows


def development_metrics(weights: dict[str, list[float]], rows: list[tuple[dict[str, float], int]],
                        threshold: float) -> tuple[int, int]:
    """Score development the way the runtime does: argmax, then the threshold.

    A `convert` below the serving threshold changes no text, so an epoch that
    is right but hesitant is not an improvement. Returns correct actions and
    false conversions; the test split is never scored here.
    """

    correct = false_conversions = 0
    for features, label in rows:
        scores = [0.0] * 4
        for name, value in features.items():
            for index, weight in enumerate(weights.get(name, (0.0, 0.0, 0.0, 0.0))):
                scores[index] += weight * value
        probabilities = softmax(scores)
        selected = max(range(4), key=lambda index: probabilities[index])
        action = ACTIONS[selected]
        if action == "convert" and probabilities[selected] < threshold:
            action = "suggest"
        correct += int(action == ACTIONS[label])
        false_conversions += int(action == "convert" and ACTIONS[label] != "convert")
    return correct, false_conversions


def train(rows: list[Row]) -> tuple[dict[str, list[float]], int, float]:
    train_rows = [(extract_context_features(row.evidence), ACTIONS.index(row.action)) for row in rows if row.split == "train"]
    development = [(extract_context_features(row.evidence), ACTIONS.index(row.action)) for row in rows if row.split == "development"]
    if not train_rows or not development:
        raise ValueError("empty training or development split")
    names = sorted({name for features, _label in train_rows for name in features})
    weights = {name: [0.0] * 4 for name in names}
    accumulators = {name: [1.0] * 4 for name in names}
    label_counts = Counter(label for _features, label in train_rows)
    # Inverse frequency already balances the classes. The former extra 2.0 bias
    # towards `keep` was chosen for a small, highly repetitive corpus; on the
    # larger one it held the `convert` probability under the fixed 0.985
    # serving threshold, so a correct decision still changed no text.
    # Both this weight and the step below were selected on development only.
    importance_by_label = {label: len(train_rows) / (4 * count) * (KEEP_IMPORTANCE if label == 0 else 1.0) for label, count in label_counts.items()}
    best: dict[str, list[float]] = {}
    best_loss, best_epoch = math.inf, 0
    # Selection follows the runtime rule under a zero-false-conversion budget.
    # Weighted log-loss is dominated by the `keep` mass and stopped before the
    # model reached the fixed serving threshold, so a correct decision still
    # changed no text. Both quantities are measured on development only.
    best_correct = -1
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
                    vector[index] -= LEARNING_RATE * gradient / math.sqrt(squared[index])
        loss = 0.0
        for features, label in development:
            scores = [0.0] * 4
            for name, value in features.items():
                for index, weight in enumerate(weights.get(name, (0.0, 0.0, 0.0, 0.0))):
                    scores[index] += weight * value
            loss -= importance_by_label[label] * math.log(max(1e-15, softmax(scores)[label]))
        loss /= len(development)
        correct, false_conversions = development_metrics(weights, development, CONVERSION_THRESHOLD)
        if false_conversions == 0 and (correct > best_correct or (correct == best_correct and loss < best_loss)):
            best_correct, best_loss, best_epoch = correct, loss, epoch + 1
            best = {name: [round(value, 9) for value in vector] for name, vector in weights.items()}
    if not best:
        raise ValueError("no epoch reached the development false-conversion budget")
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
    payload = {"feature_version": FEATURE_VERSION, "actions": list(ACTIONS), "version": "context-v1-" + digest[:12], "conversion_threshold": CONVERSION_THRESHOLD, "weights_sha256": digest, "weights": weights}
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
        "policy_sha256": hashlib.sha256((ROOT / "src/keyswitch/short_words.py").read_bytes()).hexdigest(),
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
