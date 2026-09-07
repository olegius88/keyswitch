"""Freeze balanced segmentation examples before any held-out evaluation."""
from __future__ import annotations

import argparse
import gzip
import hashlib
import json
from collections import Counter
from collections.abc import Iterator
from pathlib import Path
from typing import cast

from keyswitch.boundary_model import MAX_SUFFIX
from keyswitch.boundary_policy import features
from keyswitch.layouts import LayoutPair
from context_corpus import ROOT, SOURCE, canonical_tokens, load_source
from context_evidence import canonical, checksum, reference_models


DIRECTORY = ROOT / "model/boundary_v2"
CONFIG = DIRECTORY / "config.json"
RECEIPT = DIRECTORY / "corpus.json"
SPLITS = ("train", "development", "calibration", "test")
AMBIGUOUS = ",.;[]'`"


def family(word: str, group: int, pair: LayoutPair) -> str:
    return (pair.translate(word, "ru", "us") if group else word).casefold().replace("`", "t")[:4]


def provenance() -> dict[str, str]:
    paths = [Path(__file__), CONFIG, SOURCE, ROOT / "src/keyswitch/boundary_policy.py",
             ROOT / "src/keyswitch/boundary_model.py", ROOT / "src/keyswitch/language_model.py",
             ROOT / "src/keyswitch/layouts.py", ROOT / "tools/context_evidence.py",
             ROOT / "model/intent_v1/config.json"]
    return {path.relative_to(ROOT).as_posix(): checksum(path) for path in paths}


def freeze() -> None:
    if RECEIPT.exists() or any((DIRECTORY / (split + ".jsonl.gz")).exists() for split in SPLITS):
        raise ValueError("refusing to overwrite frozen boundary examples")
    cfg = cast(dict[str, object], json.loads(CONFIG.read_bytes()))
    pair, models = LayoutPair(), reference_models(False)
    consumed = {family(word, int(any("а" <= c <= "я" or c == "ё" for c in word)), pair)
                for phrase in load_source() for word in canonical_tokens(phrase.text) if word.isalpha()}
    source_rows: dict[str, dict[str, object]] = {}
    strata: Counter[str] = Counter()
    families: Counter[str] = Counter()
    def add(word: str, physical: str, signature: str, split: str, group: int, suffix: str, typo: bool = False) -> None:
        original = physical + suffix
        tail = len(original) - len(original.rstrip(AMBIGUOUS))
        if not 0 < tail <= MAX_SUFFIX or tail >= len(original):
            return
        category = "typo" if typo else "word_ending" if not suffix else "literal_ru" if group else "literal_en"
        key = split + ":" + original
        if key in source_rows:
            row = source_rows[key]
            labels = cast(list[int], row["labels"])
            if len(suffix) not in labels:
                labels.append(len(suffix))
                labels.sort()
                row["category"] = "ambiguous"
            return
        alternative = pair.translate(original, "us", "ru")
        source_rows[key] = {"family": signature, "split": split, "original": original,
                           "intended_word": word, "labels": [len(suffix)], "category": category,
                           "features": [features(original, alternative, count, models[0], models[1]) for count in range(tail + 1)]}
    for group, model in models.items():
        words = sorted(model.frequencies, key=lambda word: hashlib.sha256(word.encode()).digest())
        for word in words:
            if not 3 <= len(word) <= 24 or not all(("а" <= c <= "я" or c == "ё") if group else "a" <= c <= "z" for c in word):
                continue
            physical = pair.translate(word, "ru", "us") if group else word
            signature = family(word, group, pair)
            stratum = "en" if not group else "ru_ending" if physical[-1] in AMBIGUOUS else "ru_other"
            bucket = int(hashlib.sha256((str(cfg["namespace"]) + signature).encode()).hexdigest()[:8], 16) % 10
            split = "train" if bucket < 6 else "development" if bucket == 6 else "calibration" if bucket == 7 else "test"
            # Test families are fresh for all earlier phrase-derived tasks,
            # not a newly shuffled version of the rejected boundary-v1 test.
            if split == "test" and signature in consumed:
                continue
            if strata[stratum] >= cast(int, cfg["maximum_words_per_stratum"]) or families[signature] >= cast(int, cfg["maximum_words_per_family"]):
                continue
            strata[stratum] += 1
            families[signature] += 1
            if group and physical[-1] in AMBIGUOUS:
                add(word, physical, signature, split, group, "")
            for suffix in (",", ".", ";", "[", "]", "...", "'", "`"):
                add(word, physical, signature, split, group, suffix)
            if len(physical) >= 7 and int(hashlib.sha256(word.encode()).hexdigest()[:2], 16) % 7 == 0:
                changed = physical[:4] + physical[5:]
                for suffix in ("", ","):
                    add(word, changed, signature, split, group, suffix, True)
    for row in source_rows.values():
        # Enumerate counterfactual, valid lexicon words for *every* span, not
        # only the sampled words. The same physical input with two intended
        # outputs is ambiguous without context; neither label may be forced.
        labels = set(cast(list[int], row["labels"]))
        labels.update(index for index, value in enumerate(cast(list[dict[str, float]], row["features"]))
                      if value["word:known_either"])
        if len(labels) > 1:
            row["labels"] = sorted(labels)
            row["category"] = "ambiguous"
    counts: Counter[str] = Counter()
    categories: dict[str, Counter[str]] = {split: Counter() for split in SPLITS}
    groups: dict[str, set[str]] = {split: set() for split in SPLITS}
    for split in SPLITS:
        data = []
        for key, row in sorted(source_rows.items()):
            if row["split"] != split:
                continue
            data.append(canonical(row))
            counts[split] += 1
            categories[split][str(row["category"])] += 1
            groups[split].add(str(row["family"]))
        with (DIRECTORY / (split + ".jsonl.gz")).open("xb") as stream:
            stream.write(gzip.compress(b"".join(data), mtime=0))
    if any(groups[a] & groups[b] for a in SPLITS for b in SPLITS if a != b) or groups["test"] & consumed:
        raise ValueError("boundary family contamination")
    RECEIPT.write_bytes(canonical({"schema_version": 1, "provenance": provenance(),
        "sha256": {split: checksum(DIRECTORY / (split + ".jsonl.gz")) for split in SPLITS},
        "rows": dict(counts), "categories": {split: dict(values) for split, values in categories.items()},
        "families": {split: len(values) for split, values in groups.items()}, "family_overlap": 0,
        "prior_phrase_test_family_overlap": 0, "selected_words": dict(strata), "scope": cfg["evidence_scope"]}))


def rows(split: str) -> Iterator[dict[str, object]]:
    if split not in SPLITS:
        raise ValueError("invalid boundary split")
    receipt = cast(dict[str, object], json.loads(RECEIPT.read_bytes()))
    path = DIRECTORY / (split + ".jsonl.gz")
    if checksum(path) != cast(dict[str, str], receipt["sha256"])[split]:
        raise ValueError("boundary corpus checksum mismatch")
    with gzip.open(path, "rt", encoding="utf-8") as stream:
        for line in stream:
            yield cast(dict[str, object], json.loads(line))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--freeze", action="store_true")
    args = parser.parse_args()
    if args.freeze:
        freeze()
    receipt = cast(dict[str, object], json.loads(RECEIPT.read_bytes()))
    if receipt["provenance"] != provenance():
        raise ValueError("boundary corpus provenance changed")
    print(json.dumps(receipt, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
