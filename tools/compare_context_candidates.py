#!/usr/bin/env python3
"""Score context-v3 artifacts on a corpus's calibration and development frames, as the trainer does.

Every cycle needs two measurements no test split may give. The quality ratchet's calibration reference
(tests/fixture_values/quality_floors.py) is the replaced pair scored on the calibration frames of the new
corpus; a candidate is compared with the installed pair on the development frames before its sealed test
is read. Both use the trainer's own frames (context_action_pipeline.build_features: the corpus rows, the
curricula a split holds and the span frames) scored the trainer's way: the runtime's support and
automatic-conversion checks (apply_runtime_support), then the artifact's serving threshold (metrics).

Per split, profile and artifact the report gives the conversion rows, the right and false conversions and
what they net, on every frame and by head class (the head prefix a frame's features carry, `base` for
none); with more than one artifact, how many frames each converts where the first does not and the
other way round. `--show N` names up to N of those frames for each artifact: the word, its other reading
and the words around it, spelled back from the frame's character n-grams (a frame carries features, not
text), so a cycle can say which frames a refitted head decides otherwise. The test split is never read.

    PYTHONPATH=src:tools python3 tools/compare_context_candidates.py --corpus <fit> \\
        --artifact installed=src/keyswitch/resources/models/context_policy_v1.json \\
        --artifact candidate=<dir>/context-action.json [--show 20] [--output report.json]
"""

from __future__ import annotations

import argparse
import itertools
import json
import math
from array import array
from collections import Counter
from collections.abc import Iterable, Mapping, Sequence
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Final

from keyswitch.constants.models import (
    ABBREVIATION_FEATURE_PREFIX,
    ACTION_FEATURE_CHARACTER_TEXT_FIELD_INDEX,
    ACTION_FEATURE_NEIGHBOUR_WORD_COUNT,
    ACTION_FEATURE_NEIGHBOUR_WORD_MAX_CHARACTERS,
    ACTION_FEATURE_NGRAM_ORDERS,
    ACTION_FEATURE_WORD_MAX_CHARACTERS,
    ALONE_FEATURE_PREFIX,
    CAPITALS_FEATURE_PREFIX,
    KEPT_FEATURE_PREFIX,
    LATIN_ABBREVIATION_FEATURE_PREFIX,
    LETTER_FEATURE_PREFIX,
    START_FEATURE_PREFIX,
)
from keyswitch.constants.model_protocol import CALIBRATION, DEVELOPMENT
from keyswitch.constants.training import CONTEXT_ACTION_BACKEND_AUTO, CONTEXT_ACTION_BACKENDS
from keyswitch.context_action_features import _characters
from keyswitch.context_model import ACTIONS, ContextModel

# The class of a frame: the first head prefix among its features, in this order; `base` for none.
HEAD_CLASSES: Final = (("abbreviation", ABBREVIATION_FEATURE_PREFIX), ("latin_abbreviation", LATIN_ABBREVIATION_FEATURE_PREFIX),
                       ("letter", LETTER_FEATURE_PREFIX),
                       ("alone", ALONE_FEATURE_PREFIX), ("start", START_FEATURE_PREFIX),
                       ("capitals", CAPITALS_FEATURE_PREFIX), ("kept", KEPT_FEATURE_PREFIX))
BASE_CLASS: Final = "base"
ALL_FRAMES: Final = "all"
CONVERT: Final = ACTIONS.index("convert")
# The marks a word's character n-grams are padded with (context_action_features._characters).
WORD_START: Final = "^"
WORD_END: Final = "$"
# The words a frame spells under each label, and how long each can be (context_action_features).
SPELLED_LABELS: Final = (("source", 1, ACTION_FEATURE_WORD_MAX_CHARACTERS), ("target", 1, ACTION_FEATURE_WORD_MAX_CHARACTERS),
                         ("before", ACTION_FEATURE_NEIGHBOUR_WORD_COUNT, ACTION_FEATURE_NEIGHBOUR_WORD_MAX_CHARACTERS),
                         ("after", ACTION_FEATURE_NEIGHBOUR_WORD_COUNT, ACTION_FEATURE_NEIGHBOUR_WORD_MAX_CHARACTERS))

FeatureRow = tuple[dict[str, float], int, float]


def head_class(features: Iterable[str]) -> str:
    """The head whose question a frame asks, by the prefixes of its feature names."""
    names = list(features)
    for name, prefix in HEAD_CLASSES:
        if any(feature.startswith(prefix) for feature in names):
            return name
    return BASE_CLASS


def bare(name: str) -> str:
    """A feature name without the head prefixes a head's copy of it carries, nested in any order."""
    prefixes = [prefix for _name, prefix in HEAD_CLASSES]
    while (prefix := next((prefix for prefix in prefixes if name.startswith(prefix)), None)) is not None:
        name = name.removeprefix(prefix)
    return name


def character_grams(features: Iterable[tuple[str, float]], label: str) -> dict[str, float]:
    """The values of a label's character n-grams (`label:char:direction:order:text`), by their text."""
    result: dict[str, float] = {}
    for name, value in features:
        parts = bare(name).split(":", ACTION_FEATURE_CHARACTER_TEXT_FIELD_INDEX)
        if len(parts) > ACTION_FEATURE_CHARACTER_TEXT_FIELD_INDEX and parts[0] == label and parts[1] == "char":
            result[parts[ACTION_FEATURE_CHARACTER_TEXT_FIELD_INDEX]] = value
    return result


def candidate_words(grams: Mapping[str, float], longest: int) -> list[str]:
    """Every word of at most `longest` characters made of the widest n-grams in `grams`, each used no
    more often than its value allows.

    A word of n characters contributes 1/sqrt(c) to each of its n-grams of an order per occurrence,
    c = n + 3 - order the n-grams of that order it has; the values of an order sum to sqrt(c). So an
    n-gram's value times the sum of its order is how often the word holds it, and for several words
    an upper bound of how often they hold it together.
    """
    widest = max(ACTION_FEATURE_NGRAM_ORDERS)
    sums: dict[int, float] = {}
    for text, value in grams.items():
        sums[len(text)] = sums.get(len(text), 0.0) + value
    allowed = {text: round(value * sums[len(text)]) for text, value in grams.items()}
    following: dict[str, list[str]] = {}
    for text in sorted(grams):
        if len(text) == widest and not text.startswith(WORD_START):
            following.setdefault(text[:-1], []).append(text)
    found = [text[len(WORD_START):-len(WORD_END)] for text in sorted(grams)
             if text.startswith(WORD_START) and text.endswith(WORD_END) and len(text) > len(WORD_START + WORD_END)]
    used = Counter[str]()

    def extend(padded: str) -> None:
        for text in following.get(padded[len(padded) - widest + 1:], ()):
            if used[text] >= allowed[text]:
                continue
            used[text] += 1
            if text.endswith(WORD_END):
                found.append(padded[len(WORD_START):])
            elif len(padded) - len(WORD_START) < longest:
                extend(padded + text[-1])
            used[text] -= 1

    for text in sorted(grams):
        if len(text) == widest and text.startswith(WORD_START) and not text.endswith(WORD_END):
            used[text] += 1
            extend(text)
            used[text] -= 1
    return sorted(set(found))


def spelled(features: Mapping[str, float], label: str, count: int, longest: int) -> list[str] | None:
    """The words, at most `count`, whose character n-grams under `label` are exactly the frame's: [] for
    none, None when no such words are found. Words are spelled as the features hold them, case-folded;
    several words come in no particular order, as the features keep none."""
    grams = character_grams(features.items(), label)
    if not grams:
        return []
    candidates = candidate_words(grams, longest)
    for size in range(1, count + 1):
        for words in itertools.combinations_with_replacement(candidates, size):
            expected: dict[str, float] = {}
            for word in words:
                _characters(expected, label, word, direction="")
            spelt = character_grams(expected.items(), label)
            if spelt.keys() == grams.keys() and all(math.isclose(spelt[text], grams[text]) for text in grams):
                return list(words)
    return None


def word_case(features: Mapping[str, float], label: str) -> str:
    """The case the frame says the word under `label` was typed in (`label:case:...`), `none` if it says none."""
    names = (bare(name).split(":") for name in features)
    return next((parts[-1] for parts in names if parts[:-1] == [label, "case"]), "none")


def cased(word: str, case: str) -> str:
    return word.upper() if case == "upper" else word.title() if case == "title" else word


@dataclass
class Example:
    """A frame an artifact decides otherwise than the first, as far as its features spell it: None for a word
    they do not."""
    kind: str
    head: str
    word: str | None
    reading: str | None
    before: list[str] | None
    after: list[str] | None


def example(features: Mapping[str, float], kind: str, head: str) -> Example:
    """The frame's word, its other reading and the words around it, spelled back from its n-grams."""
    words = {label: spelled(features, label, count, longest) for label, count, longest in SPELLED_LABELS}
    source, target = words["source"], words["target"]
    return Example(kind, head,
                   cased(source[0], word_case(features, "source")) if source else None,
                   cased(target[0], word_case(features, "target")) if target else None,
                   words["before"], words["after"])


def probabilities(model: ContextModel, rows: Sequence[FeatureRow]) -> array[float]:
    """The model's action probabilities for every row, in ACTIONS order, before any runtime check."""
    result = array("d")
    for features, _label, _importance in rows:
        scores = [0.0] * len(ACTIONS)
        for name, value in features.items():
            weights = model.weights.get(name)
            if weights is not None:
                for index, weight in enumerate(weights):
                    scores[index] += weight * value
        peak = max(scores)
        exponents = [math.exp(score - peak) for score in scores]
        total = sum(exponents)
        result.extend(exponent / total for exponent in exponents)
    return result


def converted(scored: array[float], threshold: float) -> list[bool]:
    """Which rows the runtime converts: convert selected and at least the serving threshold."""
    result: list[bool] = []
    for index in range(len(scored) // len(ACTIONS)):
        scores = scored[index * len(ACTIONS):(index + 1) * len(ACTIONS)]
        selected = max(range(len(ACTIONS)), key=scores.__getitem__)
        result.append(selected == CONVERT and scores[CONVERT] >= threshold)
    return result


def tally(conversions: Sequence[bool], labels: Sequence[int], classes: Sequence[str]) -> dict[str, dict[str, int]]:
    """Conversion rows, right and false conversions and their net, on every frame and by class."""
    result: dict[str, dict[str, int]] = {}
    for conversion, label, name in zip(conversions, labels, classes, strict=True):
        for key in (ALL_FRAMES, name):
            counts = result.setdefault(key, {"rows": 0, "convert_rows": 0, "right": 0, "false": 0, "net": 0})
            counts["rows"] += 1
            counts["convert_rows"] += label == CONVERT
            counts["right"] += conversion and label == CONVERT
            counts["false"] += conversion and label != CONVERT
            counts["net"] = counts["right"] - counts["false"]
    return result


def difference(converts: bool, label: int) -> str:
    """What a frame an artifact decides otherwise than the first is to it: a conversion gained or lost, right or false."""
    return ("gained" if converts else "lost") + ("_right" if label == CONVERT else "_false")


def differences(first: Sequence[bool], other: Sequence[bool], labels: Sequence[int],
                classes: Sequence[str]) -> dict[str, dict[str, int]]:
    """Frames one artifact converts and the first does not, and the other way round, by class and label."""
    result: dict[str, dict[str, int]] = {}
    for before, after, label, name in zip(first, other, labels, classes, strict=True):
        if before == after:
            continue
        kind = difference(after, label)
        for key in (ALL_FRAMES, name):
            counts = result.setdefault(key, {})
            counts[kind] = counts.get(kind, 0) + 1
    return result


@dataclass
class Scored:
    """One artifact on one profile's frames of one split."""
    model_version: str
    threshold: float
    by_class: dict[str, dict[str, int]]
    # Against the first artifact; empty for the first itself.
    differences: dict[str, dict[str, int]] = field(default_factory=dict)
    # The first of those frames, as many as --show asks for.
    examples: list[Example] = field(default_factory=list)


Report = dict[str, dict[str, dict[str, Scored]]]   # split -> profile -> artifact name -> scores


def score_split(models: Mapping[str, ContextModel], rows: Sequence[FeatureRow], show: int = 0) -> dict[str, Scored]:
    """Every artifact on one profile's frames of one split, with the decisions that differ from the first's
    and the first `show` of those frames spelled back."""
    import train_context_action_model as trainer

    labels = [label for _features, label, _importance in rows]
    classes = [head_class(features) for features, _label, _importance in rows]
    decisions: dict[str, list[bool]] = {}
    report: dict[str, Scored] = {}
    for name, model in models.items():
        scored = trainer.apply_runtime_support(probabilities(model, rows), rows, model)
        decisions[name] = converted(scored, model.conversion_threshold)
        report[name] = Scored(model.version, model.conversion_threshold, tally(decisions[name], labels, classes))
    first = next(iter(decisions.values()))
    for name in list(decisions)[1:]:
        report[name].differences = differences(first, decisions[name], labels, classes)
        changed = (index for index, (before, after) in enumerate(zip(first, decisions[name], strict=True)) if before != after)
        report[name].examples = [example(rows[index][0], difference(decisions[name][index], labels[index]), classes[index])
                                 for index in itertools.islice(changed, show)]
    return report


def compare(corpus: Path, artifacts: Mapping[str, Path], backend: str, jobs: int | None, show: int = 0) -> Report:
    """Load the corpus as the trainer does, build its frames once and score every artifact on them."""
    import context_action_pipeline
    import train_context_action_model as trainer

    models = {name: ContextModel.load(path) for name, path in artifacts.items()}
    inputs = trainer.load_inputs(corpus, trainer.recipe())
    features = context_action_pipeline.build_features(inputs, context_action_pipeline.choose_backend(backend),
                                                      context_action_pipeline.choose_jobs(jobs))
    return {split: {profile: score_split(models, list(features.rows(profile, split)), show) for profile in inputs.profiles}
            for split in (CALIBRATION, DEVELOPMENT)}


def shown(words: list[str] | None) -> str:
    return "?" if words is None else " ".join(words) or "-"


def summary(report: Report) -> list[str]:
    """One line per split, profile and artifact: every frame, then each class with a conversion or a false one;
    under it, a line for each frame --show names."""
    lines: list[str] = []
    for split, profiles in report.items():
        for profile, artifacts in profiles.items():
            for name, scored in artifacts.items():
                parts = [f"{key} {counts['right']}/{counts['convert_rows']} false {counts['false']} net {counts['net']}"
                         for key, counts in scored.by_class.items()
                         if key == ALL_FRAMES or counts["convert_rows"] or counts["false"]]
                changed = scored.differences.get(ALL_FRAMES)
                suffix = f" | differs from the first: {json.dumps(changed, sort_keys=True)}" if changed else ""
                lines.append(f"{split} {profile} {name} ({scored.model_version}): " + "; ".join(parts) + suffix)
                lines.extend(f"    {item.kind} {item.head}: [{shown(item.before)}] {item.word or '?'} -> {item.reading or '?'}"
                             f" [{shown(item.after)}]" for item in scored.examples)
    return lines


def parse_artifact(text: str) -> tuple[str, Path]:
    name, separator, path = text.partition("=")
    if not separator or not name or not path:
        raise argparse.ArgumentTypeError("an artifact is NAME=PATH")
    return name, Path(path)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--corpus", type=Path, required=True, help="the frozen fitting corpus")
    parser.add_argument("--artifact", type=parse_artifact, action="append", required=True,
                        help="NAME=PATH of a context-v3 artifact; the first is the one the others are compared with")
    parser.add_argument("--output", type=Path, help="where the full report goes as JSON")
    parser.add_argument("--backend", choices=CONTEXT_ACTION_BACKENDS, default=CONTEXT_ACTION_BACKEND_AUTO,
                        help="the back end that builds the frames' features, as the trainer's")
    parser.add_argument("--jobs", type=int, default=None, help="worker processes, as the trainer's")
    parser.add_argument("--show", type=int, default=0,
                        help="how many of the frames an artifact decides otherwise than the first to name, per split and profile")
    arguments = parser.parse_args(argv)
    artifacts = dict(arguments.artifact)
    if len(artifacts) != len(arguments.artifact):
        parser.error("every artifact needs a name of its own")
    if arguments.show < 0:
        parser.error("--show takes a count of frames")
    report = compare(arguments.corpus, artifacts, arguments.backend, arguments.jobs, arguments.show)
    for line in summary(report):
        print(line, flush=True)
    if arguments.output is not None:
        payload = {split: {profile: {name: asdict(scored) for name, scored in artifacts.items()}
                           for profile, artifacts in profiles.items()} for split, profiles in report.items()}
        arguments.output.write_text(json.dumps(payload, ensure_ascii=False, indent=1, sort_keys=True) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
