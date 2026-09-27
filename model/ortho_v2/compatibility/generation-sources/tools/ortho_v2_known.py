#!/usr/bin/env python3
"""Freeze the dictionary verdict the runtime would give for every corpus token.

The orthotactic model is consulted only for tokens the dictionary of the current
layout does not know, and at runtime "known" means the frequency list OR
Hunspell. Measuring with the frequency list alone made the evidence pessimistic
in a way that cost real accuracy: `ищи`, `куш` and `афиш` are ordinary Russian
words that Hunspell knows and the frozen list does not, so they entered the
negative population, set the Russian threshold three nats higher than it needed
to be, and silenced conversions the engine could safely have made. They are also
tokens the runtime never presents to this model at all.

Hunspell cannot be called during a replay: its dictionaries differ between
machines and even between users on one machine. So the verdict is computed once,
here, from the dictionaries whose digests are already recorded in
`model/intent_v1/config.json`, and checked in. Replays read the answer; only
this tool needs a speller. That is the same device `model/context_v2/sources`
uses for its lexical evidence, for the same reason.
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import sys
from collections.abc import Sequence
from pathlib import Path
from typing import Final, cast

sys.path.insert(0, str(Path(__file__).resolve().parent))

import ortho_v2_corpus as corpus  # noqa: E402

from keyswitch.language_model import LanguageModel  # noqa: E402

ROOT: Final[Path] = Path(__file__).resolve().parents[1]
DIRECTORY: Final[Path] = ROOT / "model/ortho_v2/sources"
EVIDENCE: Final[Path] = DIRECTORY / "known-evidence.json.gz"
RECEIPT: Final[Path] = DIRECTORY / "known-receipt.json"
INTENT_CONFIG: Final[Path] = ROOT / "model/intent_v1/config.json"
LOCALES: Final[dict[str, str]] = {"en": "en_US", "ru": "ru_RU"}


def checksum(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def recorded_dictionaries() -> dict[str, dict[str, object]]:
    """The Hunspell digests the certified intent model was built against."""

    payload = cast(dict[str, object], json.loads(INTENT_CONFIG.read_bytes()))
    external = cast(dict[str, object], payload["external_evaluation"])
    recorded = cast(dict[str, object], external["hunspell"])
    return {locale: cast(dict[str, object], recorded[locale])
            for locale in sorted(LOCALES.values()) if locale in recorded}


def forms() -> dict[str, set[str]]:
    """Every visible token the engine can present, per layout being typed.

    Derived from the frozen key corpus, so the set is fixed by the same bytes the
    model is trained on and cannot drift with the machine that builds it.
    """

    result: dict[str, set[str]] = {script: set() for script in LOCALES}
    for row in corpus.load_tokens():
        keys = str(row["keys"])
        for direction in LOCALES:
            for served in corpus.served(keys, direction):
                result[direction].add(corpus.visible(served, direction))
    return result


def build() -> dict[str, dict[str, bool]]:
    models = {script: LanguageModel.load(locale) for script, locale in LOCALES.items()}
    for script, model in sorted(models.items()):
        if not model.speller.available:
            raise ValueError(f"no {script} speller: refusing to freeze a partial verdict")
    return {
        script: {word: bool(models[script].score(word).known) for word in sorted(words)}
        for script, words in sorted(forms().items())
    }


def write(payload: dict[str, dict[str, bool]], path: Path) -> str:
    # Only the known words are stored: the answer for anything absent is False,
    # which keeps the file to the fraction of the corpus a dictionary contains.
    reduced = {script: sorted(word for word, known in words.items() if known)
               for script, words in sorted(payload.items())}
    raw = (json.dumps(reduced, sort_keys=True, ensure_ascii=False,
                      separators=(",", ":")) + "\n").encode("utf-8")
    content = gzip.compress(raw, mtime=0)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(content)
    return hashlib.sha256(content).hexdigest()


def receipt(payload: dict[str, dict[str, bool]], evidence_sha: str) -> dict[str, object]:
    return {
        "schema_version": 1,
        "description": (
            "Whether the dictionary of each layout knows each token the engine can "
            "present, frozen so a replay needs no speller. Absent means not known."
        ),
        "evidence_sha256": evidence_sha,
        "tokens_sha256": checksum(corpus.TOKENS),
        "generator_sha256": checksum(Path(__file__)),
        "hunspell": recorded_dictionaries(),
        "known_by_script": {script: sum(words.values()) for script, words in sorted(payload.items())},
        "forms_by_script": {script: len(words) for script, words in sorted(payload.items())},
        "scope": (
            "the frequency list of model/intent_v1/sources plus Hunspell, which is "
            "what the runtime means by `known`; a machine whose speller differs will "
            "license more or fewer conversions than this evidence measured"
        ),
    }


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--freeze", action="store_true")
    arguments = parser.parse_args(argv)
    payload = build()
    if arguments.freeze and not EVIDENCE.exists():
        evidence_sha = write(payload, EVIDENCE)
    else:
        if not EVIDENCE.exists():
            raise ValueError("dictionary evidence is missing; run once with --freeze")
        evidence_sha = checksum(EVIDENCE)
        if write(payload, DIRECTORY / ".known-check.gz") != evidence_sha:
            (DIRECTORY / ".known-check.gz").unlink()
            raise ValueError("dictionary evidence is not reproducible from these dictionaries")
        (DIRECTORY / ".known-check.gz").unlink()
    content = (json.dumps(receipt(payload, evidence_sha), sort_keys=True,
                          ensure_ascii=False, indent=2) + "\n").encode()
    if arguments.freeze and not RECEIPT.exists():
        RECEIPT.write_bytes(content)
    elif not RECEIPT.exists() or RECEIPT.read_bytes() != content:
        raise ValueError("dictionary receipt is missing or changed")
    print(content.decode("utf-8"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
