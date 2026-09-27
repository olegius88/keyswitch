"""Freeze public, incremental input situations for the separate prefix task.

Ground truth comes from a intended full word and an author-defined setting,
not the legacy early-switch threshold. A wrong-layout prefix that is still
a possible source-language word prefix is labelled wait. No private logs.
Features are computed from the lexicon the engine serves (`lexicon`).
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

from keyswitch.context_model import ACTIONS
from keyswitch.early_switch import PrefixIndex, _dictionary_stems, early_switch_decision
from keyswitch.input_context import FieldContext, FieldRole
from keyswitch.language_model import LanguageModel
from keyswitch.layouts import LayoutPair
from keyswitch.lexicon_supplement import SUPPLEMENT_ROOT, supplement_words
from keyswitch.prefix_model import PrefixInput, features
from keyswitch.value_provenance import pin_values
from context_corpus import ROOT, SOURCE, load_source, WORDS
from context_evidence import canonical, checksum, reference_models as onboard_models
# The engine scores with the onboard lexicons plus the packaged supplement (engine.py:
# LanguageModel.load(locale, supplement_words(locale))); reference_lexicon builds exactly those
# models from the pinned lexical inputs.
from reference_lexicon import reference_models as serving_models
from verify_lexical_compatibility import contract_sha256
from keyswitch.constants.corpus import PREFIX_V2_LEGACY_DELETION_INDEX
from keyswitch.constants.file_formats import HEXADECIMAL_BASE, REPORT_JSON_INDENT
from keyswitch.constants.model_protocol import ACTIVE_SPLITS as SPLITS, PORTABLE, PROFILES, REFERENCE_HUNSPELL
from keyswitch.constants.prefix import (
    PREFIX_CORPUS_BEFORE_CHARACTERS,
    PREFIX_FAMILY_KEY_CHARACTERS,
    PREFIX_SHARED_WAIT_MAX_CHARACTERS,
    PREFIX_SPLIT_BUCKET_COUNT,
    PREFIX_TRAIN_SPLIT_BUCKETS,
)
from keyswitch.constants.training import DETERMINISTIC_CHOICE_HEX_DIGITS


DIRECTORY = ROOT / "model/prefix_v1"
CONFIG = DIRECTORY / "config.json"
RECEIPT = DIRECTORY / "corpus.json"
INTENT_CONFIG = ROOT / "model/intent_v1/config.json"
LOCALES = ("en_US", "ru_RU")
# Labels are positions in ACTIONS, the order of the model's outputs.
KEEP, CONVERT, WAIT = (ACTIONS.index(action) for action in ("keep", "convert", "wait"))


def config() -> dict[str, object]:
    return cast(dict[str, object], json.loads(CONFIG.read_bytes()))


def family(word: str, group: int, pair: LayoutPair) -> str:
    physical = pair.translate(word, "ru", "us") if group == 1 else word
    return physical.casefold().replace("`", "t")[:PREFIX_FAMILY_KEY_CHARACTERS]


def split_for(signature: str, namespace: str) -> str:
    digest = hashlib.sha256((namespace + ":" + signature).encode()).hexdigest()
    bucket = int(digest[:DETERMINISTIC_CHOICE_HEX_DIGITS], HEXADECIMAL_BASE) % PREFIX_SPLIT_BUCKET_COUNT
    return "train" if bucket < PREFIX_TRAIN_SPLIT_BUCKETS else SPLITS[bucket - PREFIX_TRAIN_SPLIT_BUCKETS + 1]


def lexicon(profile: str) -> tuple[dict[int, LanguageModel], dict[int, PrefixIndex]]:
    """The language models and prefix indexes the engine builds, from the pinned lexical inputs.

    The engine scores with LanguageModel.load(locale, supplement_words(locale)) (engine.py) and
    indexes prefixes with PrefixIndex.for_language_model, which reloads LanguageModel.load(locale):
    the onboard lexicon and its fallback words without the supplement, plus the stems of the
    Hunspell dictionary the serving model uses. The portable profile has no Hunspell.
    """
    spelling = profile == REFERENCE_HUNSPELL
    models, onboard = serving_models(spelling), onboard_models(False)
    indexes = {group: PrefixIndex(set(onboard[group].frequencies)
                                  | (_dictionary_stems(Path(model.speller.source)) if spelling else set()),
                                  onboard[group].frequencies)
               for group, model in models.items()}
    return models, indexes


def provenance() -> dict[str, str]:
    """The bytes of every input the rows are computed from, and the values its code imports.

    Of the intent configuration the rows read only the lexical contract (the language sources and
    the Hunspell section, whose digests the loaders check the files against), so that is what is
    pinned; another intent generation with the same contract leaves the rows unchanged. The
    supplement is pinned by its file and by the words served for each locale, so a supplement
    added for another locale is seen as well.
    """
    paths = [CONFIG, SOURCE, Path(__file__), ROOT / "src/keyswitch/prefix_model.py",
             ROOT / "src/keyswitch/early_switch.py", ROOT / "src/keyswitch/language_model.py",
             ROOT / "src/keyswitch/spellcheck.py", ROOT / "src/keyswitch/layouts.py",
             ROOT / "src/keyswitch/lexicon_supplement.py", SUPPLEMENT_ROOT / "lexicon-supplement-ru_RU.json",
             ROOT / "tools/context_corpus.py", ROOT / "tools/context_evidence.py", ROOT / "tools/reference_lexicon.py"]
    result = {path.relative_to(ROOT).as_posix(): checksum(path) for path in paths}
    result["constants_sha256"] = pin_values([path for path in paths if path.suffix == ".py"], source_root=ROOT / "src").sha256
    result["lexical_contract_sha256"] = contract_sha256(json.loads(INTENT_CONFIG.read_bytes()))
    result["supplement_words_sha256"] = hashlib.sha256(canonical({locale: list(supplement_words(locale)) for locale in LOCALES})).hexdigest()
    return result


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
            preceding.setdefault(key, phrase.text[max(0, match.start() - PREFIX_CORPUS_BEFORE_CHARACTERS):match.start()])
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
            (word[:PREFIX_V2_LEGACY_DELETION_INDEX] + word[PREFIX_V2_LEGACY_DELETION_INDEX + 1:], target, before, "Telegram", "text", "typo"),
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
    lexicons = {name: lexicon(name) for name in PROFILES}
    models_by_profile = {name: models for name, (models, _indexes) in lexicons.items()}
    indexes_by_profile = {name: indexes for name, (_models, indexes) in lexicons.items()}
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
                ambiguous = indexes_by_profile[PORTABLE][source].completions(original).completions > 0
                label = WAIT if length <= PREFIX_SHARED_WAIT_MAX_CHARACTERS or desired and ambiguous else CONVERT if desired else KEEP
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


def verify_receipt() -> dict[str, object]:
    """The frozen partitions are the receipt's, and it was built from exactly the current inputs."""
    receipt = cast(dict[str, object], json.loads(RECEIPT.read_bytes()))
    if (receipt.get("provenance") != provenance() or receipt.get("family_overlap") != 0
            or receipt.get("profiles") != list(PROFILES)):
        raise ValueError("prefix corpus provenance changed")
    if receipt.get("sha256") != {split: checksum(DIRECTORY / (split + ".jsonl.gz")) for split in SPLITS}:
        raise ValueError("prefix frozen examples changed")
    return receipt


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--freeze", action="store_true")
    args = parser.parse_args(argv)
    if args.freeze:
        freeze()
    print(json.dumps(verify_receipt(), ensure_ascii=False, indent=REPORT_JSON_INDENT))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
