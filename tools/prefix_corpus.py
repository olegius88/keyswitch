"""Freeze public, incremental input situations for the separate prefix task.

Ground truth comes from a intended full word and an author-defined setting,
not the legacy early-switch threshold. A wrong-layout prefix that is still
a possible source-language word prefix is labelled wait. No private logs.
"""
from __future__ import annotations

import argparse
import gzip
import hashlib
import json
from collections import Counter
from collections.abc import Iterator, Sequence
from pathlib import Path
from typing import cast

from keyswitch.early_switch import PrefixIndex, _dictionary_stems, early_switch_decision
from keyswitch.input_context import FieldContext, FieldRole
from keyswitch.layouts import LayoutPair
from keyswitch.prefix_model import PrefixInput, features
from context_corpus import ROOT, SOURCE, load_source, WORDS
from context_evidence import canonical, checksum, reference_models


DIRECTORY = ROOT / "model/prefix_v1"
CONFIG = DIRECTORY / "config.json"
RECEIPT = DIRECTORY / "corpus.json"
SPLITS = ("train", "development", "calibration", "test")
PROFILES = ("portable", "reference_hunspell")


def config() -> dict[str, object]:
    return cast(dict[str, object], json.loads(CONFIG.read_bytes()))


def family(word: str, group: int, pair: LayoutPair) -> str:
    physical = pair.translate(word, "ru", "us") if group == 1 else word
    return physical.casefold().replace("`", "t")[:3]


def split_for(signature: str, namespace: str) -> str:
    bucket = int(hashlib.sha256((namespace + ":" + signature).encode()).hexdigest()[:8], 16) % 10
    return "train" if bucket < 7 else SPLITS[bucket - 6]


def provenance() -> dict[str, str]:
    paths = [CONFIG, SOURCE, Path(__file__), ROOT / "src/keyswitch/prefix_model.py",
             ROOT / "src/keyswitch/early_switch.py", ROOT / "src/keyswitch/language_model.py",
             ROOT / "src/keyswitch/layouts.py", ROOT / "tools/context_evidence.py",
             ROOT / "model/intent_v1/config.json"]
    return {path.relative_to(ROOT).as_posix(): checksum(path) for path in paths}


def situations() -> Iterator[tuple[str, str, int, str, str, FieldRole, str, str]]:
    cfg, pair = config(), LayoutPair()
    words: Counter[tuple[str, int]] = Counter()
    preceding: dict[tuple[str, int], str] = {}
    for phrase in load_source():
        for match in WORDS.finditer(phrase.text):
            word = match[0].casefold()
            if not cast(int, cfg["minimum_word_length"]) <= len(word) <= cast(int, cfg["maximum_word_length"]):
                continue
            russian = all("а" <= char <= "я" or char == "ё" for char in word)
            english = all("a" <= char <= "z" for char in word)
            if not (russian or english):
                continue
            key = word, int(russian)
            words[key] += 1
            preceding.setdefault(key, phrase.text[max(0, match.start() - 160):match.start()])
    counts: Counter[int] = Counter()
    families: Counter[str] = Counter()
    for (word, target), _count in sorted(words.items(), key=lambda item: (-item[1], item[0])):
        signature = family(word, target, pair)
        if counts[target] >= cast(int, cfg["maximum_words_per_language"]) or families[signature] >= cast(int, cfg["maximum_words_per_family"]):
            continue
        counts[target] += 1
        families[signature] += 1
        split = split_for(signature, str(cfg["namespace"]))
        wrong = pair.translate(word, "ru" if target else "us", "us" if target else "ru")
        before = preceding[word, target]
        # Label whole situations first; expansion never moves a prefix to another split.
        rows: list[tuple[str, int, str, str, FieldRole, str]] = [
            (word, target, before, "Telegram", "text", "correct"),
            (wrong, 1 - target, before, "Telegram", "text", "wrong"),
            (wrong, 1 - target, "", "TestEditor", "unknown", "wrong_empty"),
            (wrong, 1 - target, "// пояснение " if target else "// explanation ", "Code", "code", "comment"),
            (wrong, 1 - target, "# описание " if target else "# description ", "WindowsTerminal", "terminal", "shell_comment"),
            (word[:3] + word[4:], target, before, "Telegram", "text", "typo"),
        ]
        physical = wrong if target else word
        rows.extend([
            (physical + "_value", 0, "const value = ", "Code", "code", "identifier"),
            (physical, 0, "$ ", "WindowsTerminal", "terminal", "command"),
        ])
        if target == 0:
            rows.append((word, 0, "в сообщении написано ", "Telegram", "text", "mixed_english"))
        for text, source, before, app, role, category in rows:
            yield signature, split, source, text, before, role, app, category


def freeze() -> None:
    if RECEIPT.exists() or any((DIRECTORY / (split + ".jsonl.gz")).exists() for split in SPLITS):
        raise ValueError("refusing to overwrite frozen prefix examples")
    DIRECTORY.mkdir(parents=True, exist_ok=True)
    cfg = config()
    models_by_profile = {name: reference_models(name == "reference_hunspell") for name in PROFILES}
    indexes_by_profile = {
        name: {group: PrefixIndex(set(model.frequencies) | (_dictionary_stems(Path(model.speller.source)) if name == "reference_hunspell" else set()), model.frequencies)
               for group, model in models.items()} for name, models in models_by_profile.items()
    }
    pair = LayoutPair()
    counts: Counter[str] = Counter()
    group_sets: dict[str, set[str]] = {split: set() for split in SPLITS}
    # gzip mtime=0 and empty filename keep snapshots reproducible.
    import contextlib
    with contextlib.ExitStack() as stack:
        streams = {split: stack.enter_context(gzip.GzipFile(filename="", mode="wb", mtime=0,
                    fileobj=stack.enter_context((DIRECTORY / (split + ".jsonl.gz")).open("xb")))) for split in SPLITS}
        for sequence, (signature, split, source, text, before, role, app, category) in enumerate(situations()):
            group_sets[split].add(signature)
            for length in range(1, min(len(text), cast(int, cfg["maximum_prefix_length"])) + 1):
                original = text[:length]
                alternate = pair.translate(original, "ru" if source else "us", "us" if source else "ru")
                desired = category in {"wrong", "wrong_empty", "comment", "shell_comment"}
                # Conflicting natural continuations are explicitly ambiguous, not errors to force.
                ambiguous = indexes_by_profile["portable"][source].completions(original).completions > 0
                label = 2 if length < 3 or desired and ambiguous else 1 if desired else 0
                for profile in PROFILES:
                    item = PrefixInput(original, alternate, source, FieldContext(app, "public-prefix", before, role=role))
                    values = features(item, indexes_by_profile[profile], models_by_profile[profile])
                    baseline = early_switch_decision(indexes_by_profile[profile], models_by_profile[profile], original, {1 - source: alternate}, source)
                    row = {"sequence": sequence, "family": signature, "source": source, "text": text,
                           "before": before, "application": app, "role": role, "category": category,
                           "length": length, "profile": profile, "label": label, "desired": desired,
                           "legacy_convert": baseline.should_switch, "features": values}
                    streams[split].write(canonical(row))
                    counts[split] += 1
    if any(group_sets[a] & group_sets[b] for a in SPLITS for b in SPLITS if a != b):
        raise ValueError("physical prefix families overlap")
    RECEIPT.write_bytes(canonical({"schema_version": 1, "provenance": provenance(),
        "sha256": {split: checksum(DIRECTORY / (split + ".jsonl.gz")) for split in SPLITS},
        "rows": dict(counts), "families": {split: len(groups) for split, groups in group_sets.items()},
        "family_overlap": 0, "profiles": list(PROFILES), "scope": cfg["evidence_scope"]}))


def rows(split: str) -> Iterator[dict[str, object]]:
    if split not in SPLITS:
        raise ValueError("invalid prefix split")
    receipt = cast(dict[str, object], json.loads(RECEIPT.read_bytes()))
    path = DIRECTORY / (split + ".jsonl.gz")
    if cast(dict[str, str], receipt["sha256"])[split] != checksum(path):
        raise ValueError("prefix corpus checksum mismatch")
    with gzip.open(path, "rt", encoding="utf-8") as stream:
        for line in stream:
            yield cast(dict[str, object], json.loads(line))


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--freeze", action="store_true")
    args = parser.parse_args(argv)
    if args.freeze:
        freeze()
    receipt = json.loads(RECEIPT.read_bytes())
    if receipt["provenance"] != provenance():
        raise ValueError("prefix corpus provenance changed")
    print(json.dumps(receipt, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
