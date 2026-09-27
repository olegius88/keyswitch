"""Frozen lexical inputs for portable and reference-Hunspell evaluation.

The cache contains public tokens and numeric evidence, never model targets
or private input. Training replay reads it rather than discovering whatever
system dictionaries happen to be installed on the replay machine.
"""
from __future__ import annotations

import argparse
import gzip
import hashlib
import json
from pathlib import Path
from typing import cast
from collections.abc import Sequence

from keyswitch.context_model import ContextEvidence
from keyswitch.detector import LanguageDetector
from keyswitch.input_context import FieldContext, FieldRole
from keyswitch.intent_model import CorrectionTrigger, LinearNgramModel
from keyswitch.language_model import LanguageModel, LOCALE_FALLBACKS

from context_corpus import CORPUS_ROOT, ROOT, assign, load_source
from context_frames import Frame, build, technical_frames

PROFILES = ("portable", "reference_hunspell")
CACHE = CORPUS_ROOT / "sources/lexical-evidence.json.gz"
CACHE_RECEIPT = CORPUS_ROOT / "lexical-receipt.json"
EvidenceValues = tuple[bool, bool, bool, float]


def canonical(value: object) -> bytes:
    return (json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":")) + "\n").encode("utf-8")


def checksum(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def all_frames() -> list[Frame]:
    phrases, _ = assign(load_source())
    frames = build(phrases) + technical_frames(CORPUS_ROOT / "safety.json")
    identifiers = [row.identifier for row in frames]
    if len(set(identifiers)) != len(identifiers):
        raise ValueError("duplicate frame IDs")
    return frames


def key(row: Frame, profile: str) -> str:
    return json.dumps([profile, row.original, row.group, row.trigger], ensure_ascii=False, separators=(",", ":"))


def reference_models(spelling: bool) -> dict[int, LanguageModel]:
    config = cast(dict[str, object], json.loads((ROOT / "model/intent_v1/config.json").read_bytes()))
    sources = cast(dict[str, object], config["sources"])
    languages = cast(dict[str, dict[str, object]], sources["languages"])
    external = cast(dict[str, object], config["external_evaluation"])
    dictionaries = cast(dict[str, dict[str, object]], external["hunspell"])
    models: dict[int, LanguageModel] = {}
    for group, locale in enumerate(("en_US", "ru_RU")):
        spec = languages[locale]
        path = ROOT / str(spec["path"])
        if checksum(path) != spec["sha256"]:
            raise ValueError("reference lexicon checksum mismatch")
        frequencies, bigrams = LanguageModel._read_arpa(path)
        frequency = max(max(frequencies.values(), default=1000) // 20, 1000)
        for word in LOCALE_FALLBACKS[locale]:
            frequencies[word] = max(frequencies.get(word, 0), frequency)
        model = LanguageModel(locale, frequencies, str(path), bigrams, enable_spellcheck=spelling)
        if spelling:
            dictionary = Path(model.speller.source)
            if not model.speller.available or checksum(dictionary) != dictionaries[locale]["dictionary_sha256"] or checksum(dictionary.with_suffix(".aff")) != dictionaries[locale]["affix_sha256"]:
                raise ValueError("reference morphology unavailable or changed")
        models[group] = model
    return models


def generate(frames: list[Frame]) -> dict[str, EvidenceValues]:
    intent, _ = LinearNgramModel.try_load_default()
    if intent is None:
        raise ValueError("certified baseline unavailable")
    cache: dict[str, EvidenceValues] = {}
    for profile in PROFILES:
        models = reference_models(profile == "reference_hunspell")
        detector = LanguageDetector(models, intent)
        for row in frames:
            identity = key(row, profile)
            if identity in cache:
                continue
            decision = detector.decide(row.original, {1 - row.group: row.alternative}, row.group, trigger=cast(CorrectionTrigger, row.trigger))
            source, target = models[row.group].score(row.original), models[1 - row.group].score(row.alternative)
            cache[identity] = decision.should_convert, source.known, target.known, round(target.value - source.value, 9)
        print(f"lexical profile {profile}: {len(cache)} cached inputs", flush=True)
    return cache


def load_cache() -> dict[str, EvidenceValues]:
    receipt: object = json.loads(CACHE_RECEIPT.read_bytes())
    if not isinstance(receipt, dict) or receipt.get("sha256") != checksum(CACHE):
        raise ValueError("lexical cache checksum mismatch")
    with gzip.open(CACHE, "rb") as source:
        raw = source.read(64 * 1024 * 1024 + 1)
    if len(raw) > 64 * 1024 * 1024:
        raise ValueError("oversized lexical cache")
    payload: object = json.loads(raw)
    if not isinstance(payload, dict):
        raise ValueError("invalid lexical cache")
    cache: dict[str, EvidenceValues] = {}
    for name, values in payload.items():
        if not isinstance(name, str) or not isinstance(values, list) or len(values) != 4 or any(type(value) is not bool for value in values[:3]) or type(values[3]) is not float:
            raise ValueError("invalid lexical evidence row")
        cache[name] = values[0], values[1], values[2], values[3]
    return cache


def evidence(row: Frame, profile: str, cache: dict[str, EvidenceValues]) -> ContextEvidence:
    baseline, source, target, delta = cache[key(row, profile)]
    return ContextEvidence(row.original, row.alternative, row.group,
        FieldContext(row.application, "public-corpus", row.before, row.after, cast(FieldRole, row.role)),
        row.trigger, baseline, source, target, delta)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--freeze", action="store_true")
    args = parser.parse_args(argv)
    if not args.freeze:
        print(f"verified lexical inputs: {len(load_cache())}")
        return 0
    if CACHE.exists() or CACHE_RECEIPT.exists():
        raise ValueError("refusing to replace frozen lexical evidence")
    frames = all_frames()
    content = gzip.compress(canonical(generate(frames)), mtime=0)
    CACHE.parent.mkdir(parents=True, exist_ok=True)
    CACHE.write_bytes(content)
    receipt = {"schema_version": 1, "sha256": checksum(CACHE), "profiles": list(PROFILES),
        "frames": len(frames), "frames_sha256": hashlib.sha256(canonical([row.__dict__ for row in frames])).hexdigest(),
        "source_hashes": {str(path.relative_to(ROOT)): checksum(path) for path in (
            Path(__file__), ROOT / "tools/context_frames.py", CORPUS_ROOT / "corpus-receipt.json",
            CORPUS_ROOT / "safety.json", ROOT / "model/intent_v1/config.json",
            ROOT / "src/keyswitch/language_model.py", ROOT / "src/keyswitch/detector.py",
            ROOT / "src/keyswitch/spellcheck.py", ROOT / "src/keyswitch/resources/models/layout_intent_v1.ksm")},
        "scope": "fixed reference lexicon and Hunspell dictionary hashes from intent_v1 config; no personal words or rules"}
    CACHE_RECEIPT.write_bytes(canonical(receipt))
    print(json.dumps(receipt, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
