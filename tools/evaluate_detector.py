#!/usr/bin/env python3
"""Reproducible precision/recall harness for the EN/RU detector."""

from __future__ import annotations

import argparse
import json
from dataclasses import asdict, dataclass
from pathlib import Path

from keyswitch.detector import PROTECTED_TOKENS, LanguageDetector
from keyswitch.language_model import LanguageModel
from keyswitch.layouts import LayoutPair


@dataclass
class DirectionMetrics:
    language: str
    samples: int
    true_positive: int = 0
    false_negative: int = 0
    true_negative: int = 0
    false_positive: int = 0

    @property
    def precision(self) -> float:
        denominator = self.true_positive + self.false_positive
        return self.true_positive / denominator if denominator else 1.0

    @property
    def recall(self) -> float:
        denominator = self.true_positive + self.false_negative
        return self.true_positive / denominator if denominator else 1.0

    @property
    def specificity(self) -> float:
        denominator = self.true_negative + self.false_positive
        return self.true_negative / denominator if denominator else 1.0

    def payload(self) -> dict[str, object]:
        result = asdict(self)
        result.update(
            precision=round(self.precision, 6),
            recall=round(self.recall, 6),
            specificity=round(self.specificity, 6),
        )
        return result


def converted(pair: LayoutPair, word: str, group: int) -> str:
    source, target = ("us", "ru") if group == 0 else ("ru", "us")
    return pair.translate(word, source, target)


def evaluate_direction(
    detector: LanguageDetector,
    models: dict[int, LanguageModel],
    pair: LayoutPair,
    group: int,
    sample_size: int,
) -> tuple[DirectionMetrics, list[dict[str, object]]]:
    locale = models[group].locale
    words = [
        word
        for word, _frequency in sorted(
            models[group].frequencies.items(),
            key=lambda item: item[1],
            reverse=True,
        )
        if 3 <= len(word) <= 18 and word.isalpha()
    ][:sample_size]
    return evaluate_words(detector, pair, group, locale, words)


def evaluate_words(
    detector: LanguageDetector,
    pair: LayoutPair,
    group: int,
    label: str,
    words: list[str],
) -> tuple[DirectionMetrics, list[dict[str, object]]]:
    metrics = DirectionMetrics(label, len(words))
    misses: list[dict[str, object]] = []
    for word in words:
        other = converted(pair, word, group)
        negative = detector.decide(word, {1 - group: other}, group)
        if negative.should_convert:
            metrics.false_positive += 1
            if len(misses) < 20:
                misses.append(
                    {
                        "kind": "false_positive",
                        "source": word,
                        "replacement": negative.replacement,
                        "reason": negative.reason,
                    }
                )
        else:
            metrics.true_negative += 1

        positive = detector.decide(other, {group: word}, 1 - group)
        if positive.should_convert:
            metrics.true_positive += 1
        else:
            metrics.false_negative += 1
            if len(misses) < 20:
                misses.append(
                    {
                        "kind": "false_negative",
                        "source": other,
                        "replacement": word,
                        "reason": positive.reason,
                    }
                )
    return metrics, misses


def hunspell_words(model: LanguageModel, sample_size: int) -> list[str]:
    """Read a deterministic broad-word sample from the active .dic file."""

    if not model.speller.available or not model.speller.source:
        return []
    dictionary_path = Path(model.speller.source)
    affix_path = dictionary_path.with_suffix(".aff")
    encoding = "utf-8"
    try:
        for raw_line in affix_path.read_bytes().splitlines()[:40]:
            if raw_line.startswith(b"SET "):
                encoding = raw_line[4:].decode("ascii", "replace")
                break
        candidates = {
            line.split("/", 1)[0].replace(r"\/", "/").strip().casefold()
            for line in dictionary_path.read_text(
                encoding=encoding, errors="replace"
            ).splitlines()[1:]
        }
    except OSError:
        return []
    words = sorted(
        word for word in candidates if 3 <= len(word) <= 18 and word.isalpha()
    )
    if len(words) <= sample_size:
        return words
    return [
        words[round(index * (len(words) - 1) / (sample_size - 1))]
        for index in range(sample_size)
    ]


def curated_cases(
    detector: LanguageDetector, pair: LayoutPair
) -> tuple[int, list[dict[str, object]]]:
    failures: list[dict[str, object]] = []
    positive_targets = (
        ("превет", 1),
        ("база", 1),
        ("общих", 1),
        ("скажи", 1),
        ("штуку", 1),
        ("переключал", 1),
        ("тестируемый", 1),
        ("документацию", 1),
        ("helllo", 0),
        ("worlld", 0),
        ("debugging", 0),
        ("reconnected", 0),
    )
    for target, target_group in positive_targets:
        wrong = converted(pair, target, target_group)
        decision = detector.decide(wrong, {target_group: target}, 1 - target_group)
        if not decision.should_convert:
            failures.append(
                {
                    "kind": "curated_false_negative",
                    "source": wrong,
                    "replacement": target,
                    "reason": decision.reason,
                }
            )

    negative_words = {
        "here", "руку", "foobar", "docker", "kubectl", "xfce", "API",
        "camelCase", "user_name", "abc123", "localhost", "github", "qwerty",
    }
    negative_words.update(token for token in PROTECTED_TOKENS if len(token) >= 3)
    for word in sorted(negative_words):
        group = 0 if word[0].isascii() else 1
        decision = detector.decide(word, {1 - group: converted(pair, word, group)}, group)
        if decision.should_convert:
            failures.append(
                {
                    "kind": "curated_false_positive",
                    "source": word,
                    "replacement": decision.replacement,
                    "reason": decision.reason,
                }
            )
    return len(positive_targets) + len(negative_words), failures


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--sample", type=int, default=5000)
    parser.add_argument("--dictionary-sample", type=int, default=5000)
    parser.add_argument("--strict", action="store_true")
    arguments = parser.parse_args()
    models = {
        0: LanguageModel.load("en_US"),
        1: LanguageModel.load("ru_RU"),
    }
    detector = LanguageDetector(models)
    pair = LayoutPair()
    directions = []
    dictionary_directions = []
    misses: list[dict[str, object]] = []
    for group in models:
        metrics, direction_misses = evaluate_direction(
            detector, models, pair, group, max(100, arguments.sample)
        )
        directions.append(metrics)
        misses.extend(direction_misses)
        dictionary_words = hunspell_words(
            models[group], max(100, arguments.dictionary_sample)
        )
        if dictionary_words:
            dictionary_metrics, dictionary_misses = evaluate_words(
                detector,
                pair,
                group,
                f"{models[group].locale} Hunspell",
                dictionary_words,
            )
            dictionary_directions.append(dictionary_metrics)
            misses.extend(dictionary_misses)
    curated_count, curated_failures = curated_cases(detector, pair)
    payload = {
        "models": {
            str(group): {
                "locale": model.locale,
                "words": len(model.frequencies),
                "bigrams": len(model.bigrams),
                "hunspell": model.speller.available,
                "source": model.source,
            }
            for group, model in models.items()
        },
        "directions": [metrics.payload() for metrics in directions],
        "dictionary_directions": [
            metrics.payload() for metrics in dictionary_directions
        ],
        "curated_samples": curated_count,
        "curated_failures": curated_failures,
        "sample_failures": misses,
    }
    print(json.dumps(payload, ensure_ascii=False, indent=2))
    if not arguments.strict:
        return 0
    quality_ok = all(
        metrics.precision >= 0.999
        and metrics.specificity >= 0.999
        and metrics.recall >= 0.985
        for metrics in directions
    )
    dictionary_quality_ok = len(dictionary_directions) == len(models) and all(
        metrics.precision >= 0.999
        and metrics.specificity >= 0.999
        and metrics.recall >= 0.90
        for metrics in dictionary_directions
    )
    return (
        0
        if quality_ok and dictionary_quality_ok and not curated_failures
        else 1
    )


if __name__ == "__main__":
    raise SystemExit(main())
