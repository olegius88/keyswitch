#!/usr/bin/env python3
"""Key-space token corpus derived from the frozen CC0 phrase snapshot.

The observation a layout switcher makes is not the visible string, it is the
sequence of physical keys. Because the us/ru map is a bijection on the mapped
keys, rendering every token into key space puts Russian and English text into
one shared event space, so two character models over it can be compared as a
likelihood ratio without any cross-language renormalisation.

Two rules matter for honesty and are enforced here:

* A token is labelled by the alphabet ITS OWN letters are written in, never by
  the language of the sentence. A Latin word inside a Russian sentence was typed
  with the layout switched; counting it as Russian evidence would poison the
  negatives with exactly the tokens the model must learn to convert.
* Tokens keep their punctuation. The leading dot of ``.dist`` is the character
  that produces the ``ю`` of ``ювшые``; a word regex throws that evidence away.

Splits are inherited from ``model/context_v2/corpus-receipt.json`` so this file
introduces no new corpus and no new licence surface.
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
from pathlib import Path
from typing import Final, Literal

sys.path.insert(0, str(Path(__file__).resolve().parent))

import context_corpus  # noqa: E402

from keyswitch.layouts import LayoutPair  # noqa: E402

Script = Literal["en", "ru"]

ROOT: Final[Path] = Path(__file__).resolve().parents[1]
DIRECTORY: Final[Path] = ROOT / "model/ortho_v1"
TOKENS: Final[Path] = DIRECTORY / "tokens.jsonl.gz"
RECEIPT: Final[Path] = DIRECTORY / "corpus-receipt.json"
NAMESPACE: Final[str] = "keyswitch:ortho-v1:key-space-20260908"
CYRILLIC: Final[frozenset[str]] = frozenset("абвгдеёжзийклмнопрстуфхцчшщъыьэюя")
LATIN: Final[frozenset[str]] = frozenset("abcdefghijklmnopqrstuvwxyz")
MAXIMUM_TOKEN: Final[int] = 32
PAIR: Final[LayoutPair] = LayoutPair()


def checksum(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def script_of(token: str) -> Script | None:
    """The alphabet the token itself is written in, or None when it is mixed."""

    lowered = unicodedata.normalize("NFC", token).casefold()
    cyrillic = any(character in CYRILLIC for character in lowered)
    latin = any(character in LATIN for character in lowered)
    if cyrillic and not latin:
        return "ru"
    if latin and not cyrillic:
        return "en"
    return None


def to_keys(token: str) -> str:
    """The physical keys a typist pressed to produce this token."""

    lowered = unicodedata.normalize("NFC", token).casefold()
    if script_of(token) == "ru":
        return PAIR.translate(lowered, "ru", "us")
    return lowered


def shape_of(token: str, position: int) -> str:
    """Case shape, which is what makes the abbreviation reading available."""

    letters = [character for character in token if character.isalpha()]
    if len(letters) >= 2 and all(character.isupper() for character in letters):
        return "upper"
    if token[:1].isupper():
        return "initial" if position == 0 else "inner"
    return "lower"


def rows() -> list[dict[str, object]]:
    """One row per (split, script, key sequence) with occurrence and shape counts."""

    counts: dict[tuple[str, str, str], Counter[str]] = {}
    for assigned in context_corpus.assign(context_corpus.load_source())[0]:
        words = unicodedata.normalize("NFC", assigned.phrase.text).split()
        for position, raw in enumerate(words):
            if len(raw) > MAXIMUM_TOKEN:
                continue
            script = script_of(raw)
            if script is None:
                continue
            keys = to_keys(raw)
            if not keys:
                continue
            key = (assigned.split, script, keys)
            shape = counts.setdefault(key, Counter())
            shape[shape_of(raw, position)] += 1
    return [
        {"split": split, "script": script, "keys": keys,
         "shapes": {name: shape[name] for name in sorted(shape)}}
        for (split, script, keys), shape in sorted(counts.items())
    ]


def write_tokens(payload: list[dict[str, object]], path: Path) -> str:
    raw = "".join(
        json.dumps(row, sort_keys=True, ensure_ascii=False, separators=(",", ":")) + "\n"
        for row in payload
    ).encode("utf-8")
    content = gzip.compress(raw, mtime=0)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(content)
    return hashlib.sha256(content).hexdigest()


def load_tokens(path: Path = TOKENS) -> list[dict[str, object]]:
    with gzip.open(path, "rt", encoding="utf-8") as handle:
        return [json.loads(line) for line in handle]


def provenance() -> dict[str, str]:
    paths = (
        ROOT / "tools/ortho_corpus.py",
        ROOT / "tools/context_corpus.py",
        ROOT / "src/keyswitch/layouts.py",
        context_corpus.SOURCE,
        context_corpus.RECEIPT,
    )
    return {path.relative_to(ROOT).as_posix(): checksum(path) for path in paths}


def report(payload: list[dict[str, object]], tokens_sha: str) -> dict[str, object]:
    by_split: Counter[str] = Counter()
    by_script: Counter[str] = Counter()
    occurrences: Counter[str] = Counter()
    for row in payload:
        split, script = str(row["split"]), str(row["script"])
        shapes = row["shapes"]
        assert isinstance(shapes, dict)
        by_split[split] += 1
        by_script[f"{split}:{script}"] += 1
        occurrences[f"{split}:{script}"] += sum(int(value) for value in shapes.values())
    return {
        "schema_version": 1,
        "namespace": NAMESPACE,
        "description": (
            "Key-space token types derived from the frozen CC0 phrase snapshot. "
            "Tokens are labelled by their own alphabet, never by the sentence "
            "language, and keep their punctuation."
        ),
        "types_by_split": dict(sorted(by_split.items())),
        "types_by_split_script": dict(sorted(by_script.items())),
        "occurrences_by_split_script": dict(sorted(occurrences.items())),
        "maximum_token_characters": MAXIMUM_TOKEN,
        "tokens_sha256": tokens_sha,
        "provenance": provenance(),
    }


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--freeze", action="store_true")
    arguments = parser.parse_args(argv)
    payload = rows()
    if arguments.freeze and not TOKENS.exists():
        tokens_sha = write_tokens(payload, TOKENS)
    else:
        if not TOKENS.exists():
            raise ValueError("token corpus is missing; run once with --freeze")
        tokens_sha = checksum(TOKENS)
        if write_tokens(payload, DIRECTORY / ".tokens-check.gz") != tokens_sha:
            (DIRECTORY / ".tokens-check.gz").unlink()
            raise ValueError("token corpus is not reproducible from the frozen source")
        (DIRECTORY / ".tokens-check.gz").unlink()
    content = (json.dumps(report(payload, tokens_sha), sort_keys=True,
                          ensure_ascii=False, indent=2) + "\n").encode()
    if arguments.freeze and not RECEIPT.exists():
        RECEIPT.write_bytes(content)
    elif not RECEIPT.exists() or RECEIPT.read_bytes() != content:
        raise ValueError("corpus receipt is missing or changed; do not overwrite a frozen split")
    print(content.decode("utf-8"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
