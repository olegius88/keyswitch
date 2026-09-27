#!/usr/bin/env python3
"""Freeze the token labels that can actually be trusted.

Tatoeba labels sentences, not tokens, and the orthotactic model is judged on
tokens. Every token of a Russian sentence inherits "Russian", including the ones
that are not: `пдфку` in a Russian sentence is the English word `glare` typed in
the wrong layout, `qeṛṛeḍ` is Kabyle, `áñez` is a Spanish surname. Those tokens
then serve as evidence that the model must NOT convert exactly what it exists to
convert, and because they are extreme they set the threshold for everything else.

A label is trusted here only when the sentence around it is verifiably
monolingual, which is something the frozen dictionaries can decide:

* every token of the sentence is written in that language's alphabet, so a
  sentence that quotes another script is out;
* most of its tokens are words the dictionary of that language knows, so a
  sentence that is mostly names or mostly foreign is out;
* and no token of it reads as a dictionary word of the OTHER language when
  rendered there, which is what catches text typed in the wrong layout and
  left in the corpus.

What survives is a sentence whose language is not in doubt. An unknown token
inside one is a genuine unknown word of that language - a proper noun, a
neologism, slang - and that is exactly the evidence the model was missing: it
says "leave this alone" about something that really is Russian, rather than
about something that only inherited the label.

The verdict is frozen because it needs a speller, and a replay must give the
same bytes on every machine.
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import sys
import unicodedata
from collections import Counter
from collections.abc import Sequence
from functools import lru_cache
from pathlib import Path
from typing import Final, cast

sys.path.insert(0, str(Path(__file__).resolve().parent))

import context_corpus  # noqa: E402
import ortho_v2_corpus as corpus  # noqa: E402
import ortho_v2_known as known  # noqa: E402
from historical_sources import ORTHO_V2  # noqa: E402

from keyswitch.constants.corpus import (  # noqa: E402
    ORTHO_V2_DICTIONARY_CACHE_SIZE,
    ORTHO_V2_VERIFIED_MIN_KNOWN_SHARE,
    ORTHO_V2_VERIFIED_MIN_TOKENS,
)
from keyswitch.constants.file_formats import FROZEN_CORPUS_RECEIPT_JSON_INDENT  # noqa: E402
from keyswitch.language_model import LanguageModel  # noqa: E402

ROOT: Final[Path] = Path(__file__).resolve().parents[1]
DIRECTORY: Final[Path] = ROOT / "model/ortho_v2/sources"
LABELS: Final[Path] = DIRECTORY / "verified-labels.json.gz"
RECEIPT: Final[Path] = DIRECTORY / "verified-receipt.json"
LOCALES: Final[dict[str, str]] = {"en": "en_US", "ru": "ru_RU"}
SENTENCE_LOCALE: Final[dict[str, str]] = {"eng": "en", "rus": "ru"}
GENERATOR: Final[str] = "tools/ortho_v2_verified.py"
CORPUS_TOOL: Final[str] = "tools/ortho_v2_corpus.py"
# A sentence must be long enough for its own vocabulary to say anything, and
# mostly made of words the language is known to contain: at least
# ORTHO_V2_VERIFIED_MIN_TOKENS tokens, at least ORTHO_V2_VERIFIED_MIN_KNOWN_SHARE known.


def checksum(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def spellers() -> dict[str, LanguageModel]:
    models = {script: LanguageModel.load(locale) for script, locale in LOCALES.items()}
    for script, model in sorted(models.items()):
        if not model.speller.available:
            raise ValueError(f"no {script} speller: refusing to freeze a partial verdict")
    return models


def verified_sentences() -> tuple[dict[str, set[str]], dict[str, int]]:
    """Key sequences whose label a monolingual sentence vouches for."""

    models = spellers()

    @lru_cache(maxsize=ORTHO_V2_DICTIONARY_CACHE_SIZE)
    def dictionary_knows(script: str, word: str) -> bool:
        return bool(models[script].score(word).known)

    trusted: dict[str, set[str]] = {script: set() for script in LOCALES}
    counts: Counter[str] = Counter()
    for assigned in context_corpus.assign(context_corpus.load_source())[0]:
        script = SENTENCE_LOCALE[assigned.phrase.locale]
        other = corpus.other(script)
        words = unicodedata.normalize("NFC", assigned.phrase.text).split()
        counts[f"{script}:sentences"] += 1
        shaped = [word for word in words if any(character.isalpha() for character in word)]
        if len(shaped) < ORTHO_V2_VERIFIED_MIN_TOKENS:
            counts[f"{script}:too_short"] += 1
            continue
        if any(corpus.script_of(word) not in (script, None) for word in shaped):
            counts[f"{script}:mixed_script"] += 1
            continue
        recognised = sum(1 for word in shaped if dictionary_knows(script, word))
        if recognised < ORTHO_V2_VERIFIED_MIN_KNOWN_SHARE * len(shaped):
            counts[f"{script}:too_unfamiliar"] += 1
            continue
        # Text typed in the wrong layout reads as a word of the other language.
        # One such token is enough to doubt the whole sentence's label.
        if any(dictionary_knows(other, corpus.visible(corpus.to_keys(word), other))
               for word in shaped if not dictionary_knows(script, word)):
            counts[f"{script}:other_language_reading"] += 1
            continue
        counts[f"{script}:verified"] += 1
        for word in shaped:
            keys = corpus.to_keys(word)
            for served in corpus.served(keys, script):
                trusted[script].add(served)
    return trusted, dict(sorted(counts.items()))


def write(trusted: dict[str, set[str]], path: Path) -> str:
    payload = {script: sorted(words) for script, words in sorted(trusted.items())}
    raw = (json.dumps(payload, sort_keys=True, ensure_ascii=False,
                      separators=(",", ":")) + "\n").encode("utf-8")
    content = gzip.compress(raw, mtime=0)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(content)
    return hashlib.sha256(content).hexdigest()


def tool_digests(path: Path = RECEIPT) -> tuple[str, str]:
    """The digests of this generator and of the corpus tool, as a receipt records them.

    A frozen receipt keeps both, each checked against its archived copy
    (tools/historical_sources.py); the live tools have to reproduce the labels and every
    other field.
    """

    if not path.exists():
        return checksum(Path(__file__)), checksum(ROOT / CORPUS_TOOL)
    recorded: object = json.loads(path.read_bytes())
    fields = recorded if isinstance(recorded, dict) else {}
    return (ORTHO_V2.recorded_digest(GENERATOR, fields.get("generator_sha256")),
            ORTHO_V2.recorded_digest(CORPUS_TOOL, fields.get("corpus_tool_sha256")))


def receipt(trusted: dict[str, set[str]], counts: dict[str, int], labels_sha: str) -> dict[str, object]:
    generator, corpus_tool = tool_digests()
    return {
        "schema_version": 1,
        "description": (
            "Key sequences whose language label is vouched for by a sentence that is "
            "verifiably monolingual, rather than inherited from one that is not."
        ),
        "rule": {
            "minimum_tokens": ORTHO_V2_VERIFIED_MIN_TOKENS,
            "minimum_known_share": ORTHO_V2_VERIFIED_MIN_KNOWN_SHARE,
            "requires": [
                "every token written in the sentence's own alphabet",
                "at least the given share of tokens known to that language",
                "no unknown token that reads as a word of the other language",
            ],
        },
        "labels_sha256": labels_sha,
        "source_sha256": checksum(context_corpus.SOURCE),
        "generator_sha256": generator,
        "corpus_tool_sha256": corpus_tool,
        "hunspell": known.recorded_dictionaries(),
        "sentences": counts,
        "verified_sequences": {script: len(words) for script, words in sorted(trusted.items())},
        "scope": (
            "a sentence-level verdict about a token-level label; it removes the "
            "inherited labels this corpus cannot support, and it cannot invent the "
            "ones it never had"
        ),
    }


def load(path: Path = LABELS) -> dict[str, frozenset[str]]:
    with gzip.open(path, "rt", encoding="utf-8") as handle:
        payload = json.load(handle)
    return {str(script): frozenset(map(str, words)) for script, words in payload.items()}


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--freeze", action="store_true")
    arguments = parser.parse_args(argv)
    trusted, counts = verified_sentences()
    if arguments.freeze and not LABELS.exists():
        labels_sha = write(trusted, LABELS)
    else:
        if not LABELS.exists():
            raise ValueError("verified labels are missing; run once with --freeze")
        labels_sha = checksum(LABELS)
        if write(trusted, DIRECTORY / ".verified-check.gz") != labels_sha:
            (DIRECTORY / ".verified-check.gz").unlink()
            raise ValueError("verified labels are not reproducible from these dictionaries")
        (DIRECTORY / ".verified-check.gz").unlink()
    content = (json.dumps(receipt(trusted, counts, labels_sha), sort_keys=True,
                          ensure_ascii=False, indent=FROZEN_CORPUS_RECEIPT_JSON_INDENT) + "\n").encode()
    if arguments.freeze and not RECEIPT.exists():
        RECEIPT.write_bytes(content)
    elif not RECEIPT.exists() or RECEIPT.read_bytes() != content:
        raise ValueError("verified-label receipt is missing or changed")
    print(content.decode("utf-8"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
