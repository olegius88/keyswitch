"""Freeze balanced segmentation examples before any held-out evaluation."""
from __future__ import annotations

import argparse
import gzip
import hashlib
import json
from collections import Counter
from collections.abc import Iterator, Sequence
from pathlib import Path
from typing import cast

from keyswitch.boundary_policy import features, misspelled
from keyswitch.language_model import LOCALE_FALLBACKS, LanguageModel
from keyswitch.layouts import LayoutPair
from keyswitch.value_provenance import pin_values
from context_corpus import ROOT, SOURCE, canonical_tokens, load_source
# The engine scores words with the onboard lexicons plus the packaged supplement
# (engine.py: LanguageModel.load(locale, supplement_words(locale))); reference_lexicon builds
# exactly those models, so the features and the ambiguity labels here are the ones it serves.
from reference_lexicon import reference_models as serving_models
from verify_lexical_compatibility import contract_sha256
from keyswitch.constants.boundary import (
    BOUNDARY_AMBIGUOUS_PUNCTUATION as AMBIGUOUS,
    BOUNDARY_MAX_LITERAL_SUFFIX_CHARACTERS,
    BOUNDARY_V2_FAMILY_KEY_CHARACTERS,
    BOUNDARY_V2_LITERAL_SUFFIXES,
    BOUNDARY_V2_SPLIT_BUCKET_COUNT,
    BOUNDARY_V2_TRAIN_SPLIT_BUCKETS,
    BOUNDARY_V2_TYPO_CHOICE_HEX_DIGITS,
    BOUNDARY_V2_TYPO_MIN_CHARACTERS,
    BOUNDARY_V2_TYPO_SAMPLE_MODULUS,
    BOUNDARY_V2_WORD_MAX_CHARACTERS,
    BOUNDARY_V2_WORD_MIN_CHARACTERS,
)
from keyswitch.constants.file_formats import HEXADECIMAL_BASE, REPORT_JSON_INDENT
from keyswitch.constants.model_protocol import ACTIVE_SPLITS as SPLITS
from keyswitch.constants.training import DETERMINISTIC_CHOICE_HEX_DIGITS


DIRECTORY = ROOT / "model/boundary_v2"
CONFIG = DIRECTORY / "config.json"
RECEIPT = DIRECTORY / "corpus.json"
INTENT_CONFIG = ROOT / "model/intent_v1/config.json"
LOCALES = ("en_US", "ru_RU")


def canonical(value: object) -> bytes:
    """One row or receipt as sorted, compact UTF-8 JSON and a newline (the partitions' byte format)."""
    return (json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":")) + "\n").encode("utf-8")


def checksum(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def onboard_vocabulary() -> dict[int, set[str]]:
    """Words the samples are drawn from: the frozen onboard lexicons the packages ship, and the built-in
    fallback words; the packaged supplement is served but not sampled."""
    languages = cast(dict[str, dict[str, object]],
                     cast(dict[str, object], json.loads(INTENT_CONFIG.read_bytes())["sources"])["languages"])
    vocabulary: dict[int, set[str]] = {}
    for group, locale in enumerate(LOCALES):
        path = ROOT / str(languages[locale]["path"])
        if checksum(path) != languages[locale]["sha256"]:
            raise ValueError("onboard lexicon checksum mismatch")
        frequencies, _ = LanguageModel._read_arpa(path)
        vocabulary[group] = set(frequencies) | set(LOCALE_FALLBACKS[locale])
    return vocabulary


def family(word: str, group: int, pair: LayoutPair) -> str:
    physical = pair.translate(word, "ru", "us") if group else word
    return physical.casefold().replace("`", "t")[:BOUNDARY_V2_FAMILY_KEY_CHARACTERS]


def split_for(signature: str, namespace: str) -> str:
    digest = hashlib.sha256((namespace + signature).encode()).hexdigest()
    bucket = int(digest[:DETERMINISTIC_CHOICE_HEX_DIGITS], HEXADECIMAL_BASE) % BOUNDARY_V2_SPLIT_BUCKET_COUNT
    return ("train" if bucket < BOUNDARY_V2_TRAIN_SPLIT_BUCKETS
            else "development" if bucket == BOUNDARY_V2_TRAIN_SPLIT_BUCKETS
            else "calibration" if bucket == BOUNDARY_V2_TRAIN_SPLIT_BUCKETS + 1 else "test")


def typo_sampled(word: str) -> bool:
    digest = hashlib.sha256(word.encode()).hexdigest()
    return int(digest[:BOUNDARY_V2_TYPO_CHOICE_HEX_DIGITS], HEXADECIMAL_BASE) % BOUNDARY_V2_TYPO_SAMPLE_MODULUS == 0


def typo_position(word: str, keys: int) -> int:
    """The key a misspelling drops, chosen per word: any key but the last.

    The last key is the one this model segments. Dropping it and then typing a literal on the same
    key would spell the correct word again, and the merged row would call a correctly typed known
    word ambiguous, against the word_ending rows the gates require.
    """
    digest = hashlib.sha256(("typo:" + word).encode()).hexdigest()
    return int(digest[:DETERMINISTIC_CHOICE_HEX_DIGITS], HEXADECIMAL_BASE) % (keys - 1)


def typo_suffix(word: str) -> str:
    """The literal suffix typed after a misspelled word: any of the ones a correct word gets, alike."""
    digest = hashlib.sha256(("literal:" + word).encode()).hexdigest()
    index = int(digest[:DETERMINISTIC_CHOICE_HEX_DIGITS], HEXADECIMAL_BASE) % len(BOUNDARY_V2_LITERAL_SUFFIXES)
    return BOUNDARY_V2_LITERAL_SUFFIXES[index]


def provenance() -> dict[str, str]:
    """The bytes of every input the rows are computed from, and the values its code imports.

    Of the intent configuration the rows read only the lexical contract (the language sources
    and the Hunspell section, whose digests the loaders check the files against), so that is
    what is pinned; another intent generation with the same contract leaves the rows unchanged.
    """
    paths = [Path(__file__), CONFIG, SOURCE, ROOT / "src/keyswitch/boundary_policy.py",
             ROOT / "src/keyswitch/boundary_model.py", ROOT / "src/keyswitch/language_model.py",
             ROOT / "src/keyswitch/layouts.py", ROOT / "src/keyswitch/lexicon_supplement.py",
             ROOT / "src/keyswitch/resources/lexicon-supplement-ru_RU.json", ROOT / "tools/context_corpus.py",
             ROOT / "tools/reference_lexicon.py"]
    result = {path.relative_to(ROOT).as_posix(): checksum(path) for path in paths}
    result["constants_sha256"] = pin_values([path for path in paths if path.suffix == ".py"], source_root=ROOT / "src").sha256
    result["lexical_contract_sha256"] = contract_sha256(json.loads(INTENT_CONFIG.read_bytes()))
    return result


def freeze() -> None:
    if RECEIPT.exists() or any((DIRECTORY / (split + ".jsonl.gz")).exists() for split in SPLITS):
        raise ValueError("refusing to overwrite frozen boundary examples")
    cfg = cast(dict[str, object], json.loads(CONFIG.read_bytes()))
    # Intended words come from the onboard lexicons, as before; every feature, and every other
    # reading counted as a valid word, comes from the lexicon the engine serves.
    pair, pool, models = LayoutPair(), onboard_vocabulary(), serving_models(False)
    consumed = {family(word, int(any("а" <= c <= "я" or c == "ё" for c in word)), pair)
                for phrase in load_source() for word in canonical_tokens(phrase.text) if word.isalpha()}
    source_rows: dict[str, dict[str, object]] = {}
    strata: Counter[str] = Counter()
    families: Counter[str] = Counter()
    def add(word: str, physical: str, signature: str, split: str, group: int, suffix: str, typo: bool = False) -> None:
        original = physical + suffix
        tail = len(original) - len(original.rstrip(AMBIGUOUS))
        if not 0 < tail <= BOUNDARY_MAX_LITERAL_SUFFIX_CHARACTERS or tail >= len(original):
            return
        # A misspelled word is categorised exactly like a correct one, with the prefix typo_, so the
        # trainer's per-category balance weighs its ending against its literals as it does for
        # correct words: without a lexicon entry the prior between the readings must not change.
        category = ("typo_" if typo else "") + ("word_ending" if not suffix else "literal_ru" if group else "literal_en")
        key = split + ":" + original
        # Misspellings stand for the input no lexicon reading explains. When some span of the typed
        # keys reads as a known word, the model sees that word (boundary_policy.features: the
        # word:known_* features) exactly as for a correctly typed one, whose rows label it by its
        # known readings only; a misspelled reading is never counted there ("эстонию" stays whole
        # although "эстони" could be a misspelled "эстонии"), so a misspelling that collides with a
        # known reading would label the same input differently and is not generated.
        if key in source_rows:
            row = source_rows[key]
            if typo and any(value["word:known_either"] for value in cast(list[dict[str, float]], row["features"])):
                return
            labels = cast(list[int], row["labels"])
            if len(suffix) not in labels:
                labels.append(len(suffix))
                labels.sort()
                row["category"] = "ambiguous"
            return
        alternative = pair.translate(original, "us", "ru")
        values = [features(original, alternative, count, models[0], models[1]) for count in range(tail + 1)]
        if typo and any(value["word:known_either"] for value in values):
            return
        source_rows[key] = {"family": signature, "split": split, "original": original,
                           "intended_word": word, "labels": [len(suffix)], "category": category,
                           "features": values}
    for group, vocabulary in pool.items():
        words = sorted(vocabulary, key=lambda word: hashlib.sha256(word.encode()).digest())
        for word in words:
            if (not BOUNDARY_V2_WORD_MIN_CHARACTERS <= len(word) <= BOUNDARY_V2_WORD_MAX_CHARACTERS
                    or not all(("а" <= c <= "я" or c == "ё") if group else "a" <= c <= "z" for c in word)):
                continue
            physical = pair.translate(word, "ru", "us") if group else word
            signature = family(word, group, pair)
            stratum = "en" if not group else "ru_ending" if physical[-1] in AMBIGUOUS else "ru_other"
            split = split_for(signature, str(cfg["namespace"]))
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
            for suffix in BOUNDARY_V2_LITERAL_SUFFIXES:
                add(word, physical, signature, split, group, suffix)
            if len(physical) >= BOUNDARY_V2_TYPO_MIN_CHARACTERS and typo_sampled(word):
                # A typist drops whichever key they miss; a fixed fifth key deformed whole word classes
                # alike (every seven-letter -ных adjective lost its н and read as a plural noun + `[`).
                position = typo_position(word, len(physical))
                changed = physical[:position] + physical[position + 1:]
                # The engine segments every trailing key alike (engine._completed_word counts any
                # key that is a letter in another layout; boundary_policy.features gives each its
                # own tail feature), so the literal after a misspelled word is any of them, not
                # always a comma: a comma-only literal taught that an unknown word's final `,` is
                # never its `б` and that its final `.` or `[` is never a literal.
                for suffix in ("", typo_suffix(word)):
                    add(word, changed, signature, split, group, suffix, True)
    for row in source_rows.values():
        # Enumerate counterfactual, valid lexicon words for *every* span, not
        # only the sampled words. The same physical input with two intended
        # outputs is ambiguous without context; neither label may be forced.
        labels = set(cast(list[int], row["labels"]))
        values = cast(list[dict[str, float]], row["features"])
        known = {index for index, value in enumerate(values) if value["word:known_either"]}
        if known:
            labels.update(known)
        else:
            # No reading is a known word, so the row is a misspelling (they are generated only there):
            # every reading that is a known word with one key missed - the corpus's own misspelling,
            # keyswitch.boundary_policy.misspelled, the same test the version-3 features answer - is an
            # intended output just as well, whichever words the sample happened to misspell.
            original = str(row["original"])
            alternative = pair.translate(original, "us", "ru")
            labels.update(index for index in range(len(values))
                          if misspelled(original[:len(original) - index], models[0])
                          or misspelled(alternative[:len(alternative) - index], models[1]))
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


def verify_receipt() -> dict[str, object]:
    """The frozen partitions are the receipt's, and it was built from exactly the current inputs."""
    receipt = cast(dict[str, object], json.loads(RECEIPT.read_bytes()))
    if (receipt.get("provenance") != provenance() or receipt.get("family_overlap") != 0
            or receipt.get("prior_phrase_test_family_overlap") != 0):
        raise ValueError("boundary corpus provenance changed")
    if receipt.get("sha256") != {split: checksum(DIRECTORY / (split + ".jsonl.gz")) for split in SPLITS}:
        raise ValueError("boundary frozen examples changed")
    return receipt


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--freeze", action="store_true")
    args = parser.parse_args(argv)
    if args.freeze:
        freeze()
    print(json.dumps(verify_receipt(), indent=REPORT_JSON_INDENT))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
