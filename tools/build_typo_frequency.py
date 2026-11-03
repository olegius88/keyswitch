#!/usr/bin/env python3
"""Build the counted word lists the typo correction reads (src/keyswitch/resources/typo-frequency-<locale>.bin).

Source: the full frequency lists of OpenSubtitles 2018 by Hermit Dave (FrequencyWords, pinned commit below), one per
language: `ru_full.txt` and `en_full.txt`, each `<form> <count>` per line. The content is CC BY-SA 4.0 (the code MIT);
the derived lists carry that licence and the attribution in their header. Only forms written entirely in the
language's letters (typo.TYPO_ALPHABETS) are read; a form listed twice has its counts added.

Each file holds a one-line JSON header and three sections (constants/typo.py TYPO_SECTIONS):
- `forms`: the forms with at least TYPO_STORED_MIN_COUNT occurrences and at least TYPO_MIN_LETTERS - 1 letters (a
  candidate one letter shorter than the shortest corrected word), sorted, coded one byte per letter, zlib;
- `weights`: their counts, one byte each, `round(ln(count) * TYPO_COUNT_LOG_SCALE)`, zlib;
- `seen`: a Bloom filter over the rarer forms (from TYPO_OCCURRENCE_MIN_COUNT occurrences) of at least
  TYPO_MIN_LETTERS letters, and over the technical vocabulary below, sized for TYPO_OCCURRENCE_FALSE_POSITIVE_RATE.

Technical vocabulary: subtitles hold few words of programmers (`деплой`, `закомитить`, `unselected`). The forms of at
least TYPO_MIN_LETTERS letters, lowercase or with one leading capital, that the training split of the ru.stackoverflow
dump uses at least TYPO_VOCABULARY_MIN_COUNT times - its messages (`so-train.jsonl.gz`) and its code
(`code-train.txt.gz`), Cyrillic forms for Russian and Latin ones for English - are known forms too, and the corrector
leaves them as typed. The development and test splits are never read here. The content of ru.stackoverflow is
CC BY-SA 4.0, as the counted lists are.

The receipt (src/keyswitch/resources/typo-frequency.receipt.json) records the source digests, the parameters, the
counts and the SHA-256 of every decompressed section; `--verify` rebuilds the sections from the sources and checks the
shipped files against it, so another zlib that compresses the same bytes otherwise still verifies.

Usage: build_typo_frequency.py [--ru RU_FULL] [--en EN_FULL] [--messages SO_TRAIN] [--code CODE_TRAIN] [--verify]
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import itertools
import json
import math
import re
import sys
import zlib
from collections import Counter
from collections.abc import Iterable
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))
from keyswitch.constants.file_formats import REPORT_JSON_INDENT  # noqa: E402
from keyswitch.constants.typo import (  # noqa: E402
    TYPO_ALPHABETS,
    TYPO_COUNT_LOG_SCALE,
    TYPO_COUNT_MAX_STORED,
    TYPO_FORM_SEPARATOR,
    TYPO_FREQUENCY_SCHEMA_VERSION,
    TYPO_MIN_LETTERS,
    TYPO_OCCURRENCE_FALSE_POSITIVE_RATE,
    TYPO_OCCURRENCE_MIN_COUNT,
    TYPO_SECTIONS,
    TYPO_STORED_MIN_COUNT,
    TYPO_VOCABULARY_MIN_COUNT,
    TYPO_ZLIB_LEVEL,
)
from keyswitch.constants.units import BITS_PER_BYTE  # noqa: E402
from keyswitch.typo_correction import occurrence_positions, typo_resource_path  # noqa: E402

RECEIPT = ROOT / "src/keyswitch/resources/typo-frequency.receipt.json"
PRIVATE = ROOT / ".t"
REPOSITORY = "https://github.com/hermitdave/FrequencyWords"
COMMIT = "525f9b560de45753a5ea01069454e72e9aa541c6"
ATTRIBUTION = "Hermit Dave, FrequencyWords, from OpenSubtitles 2018 (OPUS, http://opus.nlpl.eu/OpenSubtitles2018.php)"
LICENSE = "CC BY-SA 4.0 (content), MIT (code), per the repository README at the commit"


@dataclass(frozen=True)
class Source:
    locale: str
    path_in_repository: str
    default: Path
    sha256: str


# The training split of the ru.stackoverflow dump (tools/build_devchat_messages.py): its messages and its code lines.
VOCABULARY_MESSAGES = (PRIVATE / "research-2026-09-29/messages-tool/so-train.jsonl.gz",
                       "b22ec01e8e62931f35b171814b68a2192014abb097ed0feffa4eb42d7109b9cd")
VOCABULARY_CODE = (PRIVATE / "research-2026-09-29/messages-tool/code-train.txt.gz",
                   "8ae3f9d850fb712b78d00aaed1a04d22037a6dd8405909fed0bb4472ea587066")
VOCABULARY_ATTRIBUTION = "ru.stackoverflow.com posts and their code, training split, CC BY-SA 4.0"
_TOKEN = re.compile(r"[A-Za-zА-Яа-яЁё]+")


SOURCES = (
    Source("ru_RU", "content/2018/ru/ru_full.txt",
           PRIVATE / "reliable-release-2026-09-12/lexicon-sources/FrequencyWords-full/ru_full.txt",
           "32dfd94138aea2667ec455dcc8512029e39bd4b8ea70b09410d569b69d511a30"),
    Source("en_US", "content/2018/en/en_full.txt", PRIVATE / "typo/FrequencyWords-en/en_full.txt",
           "7fea67ab954e2c01df6c608c9826e594cf36f8823b3243554f88245fb75dc506"),
)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def read_counts(lines: Iterable[str], alphabet: str) -> tuple[dict[str, int], int]:
    """Counts of the forms written in `alphabet` alone, and how many lines the list has."""

    letters = set(alphabet)
    counts: dict[str, int] = {}
    total = 0
    for line in lines:
        total += 1
        form, _, count = line.rstrip("\n").rpartition(" ")
        if form and set(form) <= letters:
            counts[form] = counts.get(form, 0) + int(count)
    return counts, total


def vocabulary_counts(messages: Iterable[str], code: Iterable[str]) -> dict[str, Counter[str]]:
    """How often each form of at least TYPO_MIN_LETTERS letters, lowercase or with one leading capital, occurs in the
    messages (JSON lines with `text`) and the code lines, by locale: Cyrillic forms Russian, Latin ones English."""

    counts: dict[str, Counter[str]] = {locale: Counter() for locale in TYPO_ALPHABETS}
    letters = {locale: set(alphabet) for locale, alphabet in TYPO_ALPHABETS.items()}
    texts = (str(json.loads(line)["text"]) for line in messages)
    for text in itertools.chain(texts, code):
        for match in _TOKEN.finditer(text):
            word = match.group()
            lower = word.casefold()
            if len(word) < TYPO_MIN_LETTERS or word[1:] != lower[1:]:
                continue
            for locale, alphabet in letters.items():
                if set(lower) <= alphabet:
                    counts[locale][lower] += 1
    return counts


def bloom(forms: list[str]) -> tuple[bytes, int, int]:
    """The filter's bits, its size in bits and its number of positions per form."""

    # The optimal Bloom filter: ln(1 / p) / ln(2)^2 bits per form, ln(2) bits per form and position.
    ln_two = 1 / math.log2(math.e)
    bits = max(1, math.ceil(-len(forms) * math.log(TYPO_OCCURRENCE_FALSE_POSITIVE_RATE) / ln_two / ln_two))
    hashes = max(1, round(bits / max(1, len(forms)) * ln_two))
    filter_bits = bytearray(math.ceil(bits / BITS_PER_BYTE))
    for form in forms:
        for position in occurrence_positions(form, bits, hashes):
            filter_bits[position // BITS_PER_BYTE] |= 1 << position % BITS_PER_BYTE
    return bytes(filter_bits), bits, hashes


@dataclass(frozen=True)
class Built:
    file: bytes
    sections: dict[str, bytes]
    selection: dict[str, int]
    seen: dict[str, float]


def build(source: Source, lines: Iterable[str], source_sha256: str, vocabulary: Counter[str] | None = None) -> Built:
    """The file of one locale; `vocabulary` counts the technical forms (vocabulary_counts) that join the filter."""

    alphabet = TYPO_ALPHABETS[source.locale]
    counts, total = read_counts(lines, alphabet)
    codes = {letter: index for index, letter in enumerate(alphabet, 1)}
    stored = sorted(
        (bytes(codes[letter] for letter in form), count) for form, count in counts.items()
        if count >= TYPO_STORED_MIN_COUNT and len(form) >= TYPO_MIN_LETTERS - 1
    )
    rare = sorted(form for form, count in counts.items()
                  if TYPO_OCCURRENCE_MIN_COUNT <= count < TYPO_STORED_MIN_COUNT and len(form) >= TYPO_MIN_LETTERS)
    technical = sorted(form for form, count in (vocabulary or Counter()).items()
                       if count >= TYPO_VOCABULARY_MIN_COUNT and counts.get(form, 0) < TYPO_OCCURRENCE_MIN_COUNT)
    rare = sorted({*rare, *technical})
    forms = b"".join(form + bytes((TYPO_FORM_SEPARATOR,)) for form, _ in stored)
    weights = bytes(min(TYPO_COUNT_MAX_STORED, round(math.log(count) * TYPO_COUNT_LOG_SCALE)) for _, count in stored)
    filter_bits, bits, hashes = bloom(rare)
    sections = {"forms": forms, "weights": weights, "seen": filter_bits}
    packed = {"forms": zlib.compress(forms, TYPO_ZLIB_LEVEL), "weights": zlib.compress(weights, TYPO_ZLIB_LEVEL),
              "seen": filter_bits}
    payload = b"".join(packed[name] for name in TYPO_SECTIONS)
    selection = {"list_entries": total, "letter_forms": len(counts), "stored": len(stored), "seen_items": len(rare),
                 "vocabulary_forms": len(technical)}
    seen = {"bits": bits, "hashes": hashes, "items": len(rare),
            "false_positive_rate": (1 - math.exp(-hashes * len(rare) / bits)) ** hashes}
    header = {
        "schema_version": TYPO_FREQUENCY_SCHEMA_VERSION,
        "name": f"opensubtitles-2018-{source.locale}-typo-frequency",
        "locale": source.locale,
        "alphabet": alphabet,
        "count_log_scale": TYPO_COUNT_LOG_SCALE,
        "stored_min_count": TYPO_STORED_MIN_COUNT,
        "occurrence_min_count": TYPO_OCCURRENCE_MIN_COUNT,
        "selection": selection,
        "seen": seen,
        "sections": {name: len(packed[name]) for name in TYPO_SECTIONS},
        "payload_sha256": sha256(payload),
        "source": {"attribution": ATTRIBUTION, "repository": REPOSITORY, "commit": COMMIT,
                   "path": source.path_in_repository, "sha256": source_sha256, "license": LICENSE},
        "vocabulary": {"attribution": VOCABULARY_ATTRIBUTION, "min_count": TYPO_VOCABULARY_MIN_COUNT,
                       "messages_sha256": VOCABULARY_MESSAGES[1], "code_sha256": VOCABULARY_CODE[1]},
    }
    line = json.dumps(header, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return Built(line + b"\n" + payload, sections, selection, seen)


def receipt_entry(source: Source, built: Built) -> dict[str, object]:
    return {
        "source": {"repository": REPOSITORY, "commit": COMMIT, "path": source.path_in_repository,
                   "sha256": source.sha256, "license": LICENSE, "attribution": ATTRIBUTION},
        "selection": built.selection,
        "seen": built.seen,
        "sections_sha256": {name: sha256(data) for name, data in built.sections.items()},
        "file": {"path": typo_resource_path(source.locale).relative_to(ROOT).as_posix(), "bytes": len(built.file),
                 "sha256": sha256(built.file)},
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--ru", type=Path, default=SOURCES[0].default)
    parser.add_argument("--en", type=Path, default=SOURCES[1].default)
    parser.add_argument("--messages", type=Path, default=VOCABULARY_MESSAGES[0])
    parser.add_argument("--code", type=Path, default=VOCABULARY_CODE[0])
    parser.add_argument("--verify", action="store_true", help="check the shipped files against the sources")
    arguments = parser.parse_args(argv)
    paths = {"ru_RU": arguments.ru, "en_US": arguments.en}
    entries: dict[str, dict[str, object]] = {}
    failures: list[str] = []
    for dump, (_, pinned) in ((arguments.messages, VOCABULARY_MESSAGES), (arguments.code, VOCABULARY_CODE)):
        if sha256(dump.read_bytes()) != pinned:
            raise SystemExit(f"{dump}: SHA-256 differs from the pinned {pinned}")
    with gzip.open(arguments.messages, "rt", encoding="utf-8") as messages, \
            gzip.open(arguments.code, "rt", encoding="utf-8") as code:
        vocabulary = vocabulary_counts(messages, code)
    for source in SOURCES:
        raw = paths[source.locale].read_bytes()
        if sha256(raw) != source.sha256:
            raise SystemExit(f"{paths[source.locale]}: SHA-256 differs from the pinned {source.sha256}")
        built = build(source, raw.decode("utf-8").splitlines(), source.sha256, vocabulary[source.locale])
        entry = receipt_entry(source, built)
        if arguments.verify:
            shipped = typo_resource_path(source.locale).read_bytes()
            header_line, _, payload = shipped.partition(b"\n")
            header = json.loads(header_line)
            sizes = [header["sections"][name] for name in TYPO_SECTIONS]
            forms = zlib.decompress(payload[:sizes[0]])
            weights = zlib.decompress(payload[sizes[0]:sizes[0] + sizes[1]])
            seen = payload[sizes[0] + sizes[1]:]
            if {"forms": forms, "weights": weights, "seen": seen} != built.sections:
                failures.append(source.locale)
        else:
            typo_resource_path(source.locale).write_bytes(built.file)
        entries[source.locale] = entry
    receipt = {
        "schema_version": TYPO_FREQUENCY_SCHEMA_VERSION,
        "tool": "tools/build_typo_frequency.py",
        "parameters": {
            "stored_min_count": TYPO_STORED_MIN_COUNT, "occurrence_min_count": TYPO_OCCURRENCE_MIN_COUNT,
            "min_letters": TYPO_MIN_LETTERS, "count_log_scale": TYPO_COUNT_LOG_SCALE,
            "occurrence_false_positive_rate": TYPO_OCCURRENCE_FALSE_POSITIVE_RATE, "zlib_level": TYPO_ZLIB_LEVEL,
            "vocabulary_min_count": TYPO_VOCABULARY_MIN_COUNT,
        },
        "vocabulary": {
            "attribution": VOCABULARY_ATTRIBUTION,
            "messages": {"file": VOCABULARY_MESSAGES[0].name, "sha256": VOCABULARY_MESSAGES[1]},
            "code": {"file": VOCABULARY_CODE[0].name, "sha256": VOCABULARY_CODE[1]},
            "forms_counted": {locale: len(table) for locale, table in vocabulary.items()},
        },
        "locales": entries,
    }
    if arguments.verify:
        shipped_receipt = json.loads(RECEIPT.read_bytes())
        for locale, entry in entries.items():
            recorded = shipped_receipt["locales"][locale]
            if recorded["sections_sha256"] != entry["sections_sha256"] or recorded["selection"] != entry["selection"]:
                failures.append(locale + " receipt")
        print("verified" if not failures else "differs: " + ", ".join(failures))
        return 1 if failures else 0
    RECEIPT.write_text(json.dumps(receipt, ensure_ascii=False, indent=REPORT_JSON_INDENT) + "\n", encoding="utf-8")
    for locale, entry in entries.items():
        print(locale, json.dumps(entry["file"]), json.dumps(entry["selection"]))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
