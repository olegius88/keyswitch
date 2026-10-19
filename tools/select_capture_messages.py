#!/usr/bin/env python3
"""Messages of the 2023 ru.stackoverflow dump that hold a token of a head's class, for a capture (tools/mixed_typing.py).

The capture types messages the four ways a person types them and records what the engine asks; a class the dump holds
rarely needs its own messages. This tool reads the messages file the capture reads (`so-train.jsonl.gz`, written by
mixed_typing.py build), keeps the messages with a token of the class, leaves out every question the developer-chat
messages come from (their members files name the questions: no message of a question the developer-chat test or its
development split draws from is typed for training), and writes the rest in two files by question, so that the
messages of one question are typed for one split only:

- variable: a one-letter token right after a word of Cyrillic letters only: a Latin letter (a variable in Russian
  prose: `от a до b`) or a Cyrillic letter that is no Russian word of one letter (`т.` of `и т. д.`, `п.` of a list);
- dotted: a Russian abbreviation of parts joined by dots (`т.к.`, `т.е.`, `т.д.`, `д.б.`) or a Latin token of short
  parts joined by `/` (`n/a`, `I/O`, `w/o`); a short word ending a sentence (`см.`, `все.`) stands in those messages too.

    python3 tools/select_capture_messages.py --messages <so-train.jsonl.gz> --members <devchat-*.members.jsonl.gz> ... \\
        --class variable --development-share 0.2 --out <dir>
"""
from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import re
import sys
from collections.abc import Callable, Iterable, Sequence
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from keyswitch.constants.file_formats import HEXADECIMAL_BASE  # noqa: E402
from keyswitch.constants.models import DOTTED_PART_MAX_LETTERS  # noqa: E402
from keyswitch.constants.training import DETERMINISTIC_CHOICE_HEX_DIGITS  # noqa: E402
from keyswitch.short_words import TRUSTED_SINGLE_LETTER_WORDS  # noqa: E402

TRAILING_SIGNS = ".,;:!?)"
RUSSIAN_DOTTED = re.compile(rf"[а-яё]{{1,{DOTTED_PART_MAX_LETTERS}}}(?:\.[а-яё]{{1,{DOTTED_PART_MAX_LETTERS}}})+\.?", re.IGNORECASE)
LATIN_SLASHED = re.compile(rf"[a-z]{{1,{DOTTED_PART_MAX_LETTERS}}}(?:/[a-z]{{1,{DOTTED_PART_MAX_LETTERS}}})+", re.IGNORECASE)


def cyrillic(char: str) -> bool:
    return "а" <= char.casefold() <= "я" or char.casefold() == "ё"


def variable_token(previous: str, token: str) -> bool:
    """A letter alone after a word of Cyrillic letters: Latin, or Cyrillic and no Russian word of one letter."""

    core = token.rstrip(TRAILING_SIGNS)
    if len(core) != 1 or not core.isalpha() or not previous or not all(cyrillic(char) for char in previous):
        return False
    return core.isascii() or (cyrillic(core) and core.casefold() not in TRUSTED_SINGLE_LETTER_WORDS)


def dotted_token(previous: str, token: str) -> bool:
    """A Russian abbreviation with dots, or a Latin token of short parts joined by `/`."""

    core = token.lstrip("(").rstrip(",;:)!?")
    return RUSSIAN_DOTTED.fullmatch(core) is not None or LATIN_SLASHED.fullmatch(core) is not None


CLASSES: dict[str, Callable[[str, str], bool]] = {"variable": variable_token, "dotted": dotted_token}


def holds(text: str, token_of_class: Callable[[str, str], bool]) -> bool:
    tokens = text.split()
    return any(token_of_class(previous, token) for previous, token in zip(["", *tokens], tokens))


def excluded_questions(members: Iterable[Path]) -> set[int]:
    questions: set[int] = set()
    for path in members:
        with gzip.open(path, "rt", encoding="utf-8") as handle:
            questions.update(int(json.loads(line)["question"]) for line in handle)
    return questions


def split_of(question: int, development_share: float) -> str:
    digest = hashlib.sha256(f"capture-split:{question}".encode()).hexdigest()[:DETERMINISTIC_CHOICE_HEX_DIGITS]
    draw = int(digest, HEXADECIMAL_BASE) / HEXADECIMAL_BASE ** DETERMINISTIC_CHOICE_HEX_DIGITS
    return "dev" if draw < development_share else "train"


def select(messages: Path, members: Sequence[Path], name: str, development_share: float, out: Path) -> dict[str, object]:
    token_of_class = CLASSES[name]
    left_out = excluded_questions(members)
    out.mkdir(parents=True, exist_ok=True)
    counts: dict[str, int] = {"train": 0, "dev": 0, "left_out": 0}
    paths = {split: out / f"so-{name}-{split}.jsonl.gz" for split in ("train", "dev")}
    handles = {split: gzip.open(path, "wt", encoding="utf-8") for split, path in paths.items()}
    try:
        with gzip.open(messages, "rt", encoding="utf-8") as source:
            for line in source:
                row = json.loads(line)
                if not holds(str(row["text"]), token_of_class):
                    continue
                if int(row["qid"]) in left_out:
                    counts["left_out"] += 1
                    continue
                split = split_of(int(row["qid"]), development_share)
                handles[split].write(json.dumps(row, ensure_ascii=False) + "\n")
                counts[split] += 1
    finally:
        for handle in handles.values():
            handle.close()
    return {"class": name, "messages": counts, "files": {split: path.name for split, path in paths.items()},
            "excluded_questions": len(left_out)}


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--messages", type=Path, required=True)
    parser.add_argument("--members", type=Path, action="append", default=[], help="a developer-chat members file (repeatable)")
    parser.add_argument("--class", dest="name", choices=sorted(CLASSES), required=True)
    parser.add_argument("--development-share", type=float, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args(argv)
    print(json.dumps(select(args.messages, args.members, args.name, args.development_share, args.out), indent=1))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
