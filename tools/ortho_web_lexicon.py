#!/usr/bin/env python3
"""Freeze the Russian word forms of a public web vocabulary that the orthotactic model counts.

The character models learn what a language's words look like from train tokens and the frozen
Onboard word lists. Neither holds the words people write on the web that no dictionary knows:
slang on a borrowed stem (`фнаф`, `фнафер`), the name of a layout (`йцукен`), an abbreviation
used as a word. To the Russian model such a word is a sequence it has never seen, so the English
reading of its keys wins and the word is turned into garbage. Counting the web's word forms
teaches the model those sequences - in the channel of Russian that the model uses only when a
token was typed in Russian (`ortho_model`, `source_models`), so they can keep a Cyrillic word but
never argue that Latin keys were meant as Russian.

The source is the vocabulary of the fastText Common Crawl vectors for Russian
(`cc.ru.300.vec.gz`: Common Crawl and Wikipedia, words by descending frequency). Only forms the
web text itself writes in lowercase Cyrillic letters are taken - common words and slang, not
proper names, whose transliterations (`Мацуи`, `Фицуильям`) would teach the model the very
sequences that are evidence of a wrong layout (`цуи` of `webjs`) - and only forms of at least
ORTHO_WEB_LEXICON_MIN_CHARACTERS letters, or compounds of such words joined by hyphens
(`из-за`, `тв-шоу`, `рен-тв`), which a single word form never shows the model. Abbreviations,
which the web writes in capitals, are left out like names: `УИОП` and `МЦУИС` are the same
transliterations. A form weighs like an Onboard word: the bit length of its count per billion
words, estimated from its rank by Zipf's law (`web_weight`).

The source file is large and is not checked in; the derived forms are, with a receipt that pins
the source's digest. `--freeze SOURCE` derives them once; a plain run checks the derived file
against its receipt; `--source SOURCE` also derives them again and compares.
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import re
from collections.abc import Iterator, Sequence
from pathlib import Path
from typing import Final, cast

from keyswitch.constants.file_formats import FROZEN_CORPUS_RECEIPT_JSON_INDENT
from keyswitch.constants.ortho import (
    ORTHO_V1_MAXIMUM_LEXICON_WEIGHT,
    ORTHO_WEB_LEXICON_MIN_CHARACTERS,
    ORTHO_WEB_LEXICON_SCHEMA_VERSION,
    ORTHO_WEB_LEXICON_TOP_PER_BILLION,
)

ROOT: Final[Path] = Path(__file__).resolve().parents[1]
DIRECTORY: Final[Path] = ROOT / "model" / "ortho_v1"
FORMS: Final[Path] = DIRECTORY / "web-lexicon-ru.json.gz"
RECEIPT: Final[Path] = DIRECTORY / "web-lexicon-receipt.json"
SOURCE_URL: Final[str] = "https://dl.fbaipublicfiles.com/fasttext/vectors-crawl/cc.ru.300.vec.gz"
SOURCE_SHA256: Final[str] = "02bd7a202103a6b90c8f97617144ed230e3b4d51a08c28d2e029e1f0027917fa"
SOURCE_PAGE: Final[str] = "https://fasttext.cc/docs/en/crawl-vectors.html"
LICENSE: Final[str] = "CC BY-SA 3.0, as the source page states for the word vectors"
LICENSE_NOTE: Final[str] = (
    "the counts derived from these forms ship inside the orthotactic artifact; that an adaptation of "
    "CC BY-SA 3.0 material may be shared under a later version with the same license elements, and that "
    "CC BY-SA 4.0 is one-way compatible with the GNU GPL version 3 (creativecommons.org, compatible "
    "licenses), is a reading of the license texts that no lawyer has reviewed"
)
INTEGRITY: Final[str] = (
    "downloaded over TLS; the source publishes no checksum or signature for the file, so its SHA-256 "
    "was recorded at download and every derivation refuses any other file"
)
FORM: Final[re.Pattern[str]] = re.compile("[а-яё]+")
COMPOUND: Final[re.Pattern[str]] = re.compile("[а-яё]+(?:-[а-яё]+)+")


def checksum(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def web_weight(rank: int) -> int:
    """An Onboard-like weight from a rank: the bit length of the count per billion words (Zipf)."""

    per_billion = round(ORTHO_WEB_LEXICON_TOP_PER_BILLION / (rank + 1))
    return max(1, min(ORTHO_V1_MAXIMUM_LEXICON_WEIGHT, int(per_billion).bit_length()))


def vocabulary(source: Path) -> Iterator[str]:
    """The words of a fastText `.vec.gz`, in its order (descending frequency)."""

    with gzip.open(source, "rt", encoding="utf-8") as handle:
        handle.readline()                        # the header: word count and dimension
        for line in handle:
            yield line.split(" ", 1)[0]


def derive(source: Path) -> list[list[object]]:
    """[form, rank] for every lowercase Cyrillic word with enough letters and every lowercase compound
    with hyphens; rank is its line."""

    if checksum(source) != SOURCE_SHA256:
        raise ValueError("web vocabulary source is not the pinned file")
    return [[word, rank] for rank, word in enumerate(vocabulary(source))
            if (len(word) >= ORTHO_WEB_LEXICON_MIN_CHARACTERS and FORM.fullmatch(word)) or COMPOUND.fullmatch(word)]


def write(forms: list[list[object]], path: Path) -> str:
    raw = (json.dumps({"schema_version": ORTHO_WEB_LEXICON_SCHEMA_VERSION, "forms": forms},
                      ensure_ascii=False, separators=(",", ":")) + "\n").encode()
    content = gzip.compress(raw, mtime=0)      # no time and no file name in the header: byte-stable
    path.write_bytes(content)
    return hashlib.sha256(content).hexdigest()


def load(path: Path = FORMS) -> dict[str, int]:
    """form -> weight, from the frozen derived file."""

    with gzip.open(path, "rt", encoding="utf-8") as handle:
        payload = cast(dict[str, object], json.load(handle))
    if payload.get("schema_version") != ORTHO_WEB_LEXICON_SCHEMA_VERSION:
        raise ValueError("unsupported web lexicon")
    return {str(form): web_weight(cast(int, rank)) for form, rank in cast(list[list[object]], payload["forms"])}


def receipt(forms: list[list[object]], forms_sha: str) -> dict[str, object]:
    return {
        "schema_version": ORTHO_WEB_LEXICON_SCHEMA_VERSION,
        "source": {
            "url": SOURCE_URL, "page": SOURCE_PAGE, "sha256": SOURCE_SHA256, "integrity": INTEGRITY,
            "license": LICENSE, "license_note": LICENSE_NOTE,
            "trained_on": "Common Crawl and Wikipedia (Grave et al., Learning Word Vectors for 157 Languages)",
        },
        "selection": (
            "words of the vocabulary written in lowercase Cyrillic [а-яё]+ in the web text, at least "
            f"{ORTHO_WEB_LEXICON_MIN_CHARACTERS} letters, and lowercase compounds [а-яё]+(-[а-яё]+)+; "
            "the rank is the word's line in the source"
        ),
        "weight": (
            f"max(1, min({ORTHO_V1_MAXIMUM_LEXICON_WEIGHT}, "
            f"round({ORTHO_WEB_LEXICON_TOP_PER_BILLION} / (rank + 1)).bit_length()))"
        ),
        "use": "counted only into the channel of Russian that scores a token typed in Russian",
        "forms": len(forms),
        "forms_sha256": forms_sha,
    }


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--freeze", type=Path, metavar="SOURCE")
    parser.add_argument("--source", type=Path)
    arguments = parser.parse_args(argv)
    if arguments.freeze is not None:
        if FORMS.exists() or RECEIPT.exists():
            raise ValueError("web lexicon already frozen; do not overwrite it")
        forms = derive(arguments.freeze)
        content = receipt(forms, write(forms, FORMS))
        RECEIPT.write_text(json.dumps(content, ensure_ascii=False, sort_keys=True,
                                      indent=FROZEN_CORPUS_RECEIPT_JSON_INDENT) + "\n", encoding="utf-8")
    recorded = cast(dict[str, object], json.loads(RECEIPT.read_bytes()))
    if recorded.get("forms_sha256") != checksum(FORMS):
        raise ValueError("web lexicon does not match its receipt")
    if arguments.source is not None:
        check = FORMS.with_name(".web-lexicon-check.json.gz")
        try:
            if write(derive(arguments.source), check) != recorded["forms_sha256"]:
                raise ValueError("web lexicon is not reproducible from its source")
        finally:
            check.unlink(missing_ok=True)
    print(json.dumps(recorded, ensure_ascii=False, sort_keys=True, indent=FROZEN_CORPUS_RECEIPT_JSON_INDENT))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
