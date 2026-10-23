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
- dotted: a token whose keys in the English layout are a question of the dotted-abbreviation head
  (context_action_features.dotted_question, the predicate the engine asks): a Russian abbreviation of two or more parts
  joined by dots (`т.к.`, `т.е.`, `д.б.`) or a Latin token of short parts joined by `/` (`n/a`, `I/O`, `usr/bin`,
  `src/app/`). The predicate itself chooses, so the messages typed hold every kind of token the head is asked about,
  the Latin ones it keeps among them (a selector of its own once left out `src/app/`, and the head never saw one kept).
  The two sides have classes of their own, a message in one of them only: `dotted-latin` (a Latin token of the class:
  every one of them can be typed, they are few) and `dotted-russian` (a Russian abbreviation and no Latin token of the
  class: the abbreviations are few and repeat, a sample of them is typed).

- opening-short: a message whose first token is a word of at most three letters, the words the engine waits on before
  it decides them with the next word (`еще`, `не`, `If`): the questions the hypothesis head answers come from such
  messages. Tatoeba messages (`--prefix tatoeba-chat`) are taken only among the sentences the captured training questions
  were already typed from (`--only-ids`, model/context_v1/captured/tatoeba-authors.tsv.xz), which no corpus split holds
  (freeze_context_action_holdout.captured_tatoeba_identifiers).
- slippable: a message with two words of Russian letters in a row, for the slip typing mode (tools/mixed_typing.py).

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
from keyswitch.constants.training import DETERMINISTIC_CHOICE_HEX_DIGITS  # noqa: E402
from keyswitch.constants.models import ALNUM_HEAD_MAX_CHARACTERS, OPENING_SHORT_MAX_LETTERS  # noqa: E402
from keyswitch.context_action_features import dotted_question, letters_and_digits  # noqa: E402
from keyswitch.layouts import LayoutPair  # noqa: E402
from keyswitch.short_words import TRUSTED_SINGLE_LETTER_WORDS  # noqa: E402

TRAILING_SIGNS = ".,;:!?)"
# Signs that stand around a token in prose and are no part of it: `(v2)`, `«1с»,`.
ALNUM_EDGE_SIGNS = ".,;:!?()\"'«»"
LAYOUTS = LayoutPair()
# The key of the Russian dot (and of its comma, with Shift) prints `/` (`?`) in the English layout; LayoutPair maps the
# keys of letters and of the English `,` and `.` (`б`, `ю`).
RUSSIAN_SIGN_KEYS = str.maketrans({".": "/", ",": "?"})
ENGLISH_SIGN_KEYS = str.maketrans({"/": ".", "?": ","})


def cyrillic(char: str) -> bool:
    return "а" <= char.casefold() <= "я" or char.casefold() == "ё"


def variable_token(previous: str, token: str) -> bool:
    """A letter alone after a word of Cyrillic letters: Latin, or Cyrillic and no Russian word of one letter."""

    core = token.rstrip(TRAILING_SIGNS)
    if len(core) != 1 or not core.isalpha() or not previous or not all(cyrillic(char) for char in previous):
        return False
    return core.isascii() or (cyrillic(core) and core.casefold() not in TRUSTED_SINGLE_LETTER_WORDS)


def dotted_token(previous: str, token: str) -> bool:
    """A token the dotted-abbreviation head is asked about once typed in the English layout: a Latin token as written,
    with its Cyrillic reading; a Cyrillic one by its keys there (`т.к.` is `n/r/`)."""

    core = token.lstrip("(").rstrip(",;:)!?")
    if core.isascii():
        return dotted_question(core, LAYOUTS.translate(core, "us", "ru").translate(ENGLISH_SIGN_KEYS))
    return dotted_question(LAYOUTS.translate(core.translate(RUSSIAN_SIGN_KEYS), "ru", "us"), core)


def latin_dotted_token(previous: str, token: str) -> bool:
    return token.isascii() and dotted_token(previous, token)


def russian_dotted_token(previous: str, token: str) -> bool:
    return not token.isascii() and dotted_token(previous, token)


def alnum_token(previous: str, token: str) -> bool:
    """A token the letters-and-digits head is asked about once typed in the Russian layout (alnum_question): letters
    of one script and digits, whose keys in the other layout are letters and digits too (`v67`, `x86`, `1с`, `3д`), of at
    most ALNUM_HEAD_MAX_CHARACTERS."""

    core = token.strip(ALNUM_EDGE_SIGNS)
    other = LAYOUTS.translate(core, "us", "ru") if core.isascii() else LAYOUTS.translate(core, "ru", "us")
    return len(core) <= ALNUM_HEAD_MAX_CHARACTERS and letters_and_digits(core) and letters_and_digits(other)


def latin_alnum_token(previous: str, token: str) -> bool:
    return token.strip(ALNUM_EDGE_SIGNS).isascii() and alnum_token(previous, token)


def russian_alnum_token(previous: str, token: str) -> bool:
    return not token.strip(ALNUM_EDGE_SIGNS).isascii() and alnum_token(previous, token)


def opening_short(text: str) -> bool:
    tokens = text.split()
    return bool(tokens) and tokens[0].isalpha() and len(tokens[0]) <= OPENING_SHORT_MAX_LETTERS


def slippable(text: str) -> bool:
    """Two words of Russian letters in a row: the slip typing mode types the second with the English layout on."""
    tokens = text.split()
    return any(all(cyrillic(char) for char in first if char.isalpha()) and any(cyrillic(char) for char in first)
               and any(cyrillic(char) for char in second) and not any("a" <= char.lower() <= "z" for char in second)
               for first, second in zip(tokens, tokens[1:]))


# Classes of a whole message rather than of one of its tokens.
MESSAGE_CLASSES: dict[str, Callable[[str], bool]] = {"opening-short": opening_short, "slippable": slippable}
CLASSES: dict[str, Callable[[str, str], bool]] = {"variable": variable_token, "dotted": dotted_token,
                                                  "dotted-latin": latin_dotted_token, "dotted-russian": russian_dotted_token,
                                                  "alnum-latin": latin_alnum_token, "alnum-russian": russian_alnum_token}
# A message of a class named here is no message of the class it names: it holds a token of that one too.
EXCLUDED_BY: dict[str, str] = {"dotted-russian": "dotted-latin", "alnum-russian": "alnum-latin"}


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


def listed_ids(path: Path | None) -> frozenset[int] | None:
    """The sentence ids a Tatoeba authors file names (`<id> TAB <author>` lines), or None without one."""

    if path is None:
        return None
    import lzma

    with lzma.open(path, "rt", encoding="utf-8") as stream:
        return frozenset(int(line.split("\t", 1)[0]) for line in stream if line[:1].isdigit())


def select(messages: Path, members: Sequence[Path], name: str, development_share: float, out: Path,
           prefix: str = "so", only: Path | None = None) -> dict[str, object]:
    message_of_class = MESSAGE_CLASSES.get(name)
    token_of_class = CLASSES.get(name)
    other = CLASSES[EXCLUDED_BY[name]] if name in EXCLUDED_BY else None
    left_out = excluded_questions(members)
    allowed = listed_ids(only)
    out.mkdir(parents=True, exist_ok=True)
    counts: dict[str, int] = {"train": 0, "dev": 0, "left_out": 0}
    paths = {split: out / f"{prefix}-{name}-{split}.jsonl.gz" for split in ("train", "dev")}
    handles = {split: gzip.open(path, "wt", encoding="utf-8") for split, path in paths.items()}
    try:
        with gzip.open(messages, "rt", encoding="utf-8") as source:
            for line in source:
                row = json.loads(line)
                text = str(row["text"])
                if (message_of_class is not None and not message_of_class(text)) or (
                        token_of_class is not None and not holds(text, token_of_class)) or (other is not None and holds(text, other)):
                    continue
                if allowed is not None and int(row["qid"]) not in allowed:
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
    parser.add_argument("--class", dest="name", choices=sorted({*CLASSES, *MESSAGE_CLASSES}), required=True)
    parser.add_argument("--development-share", type=float, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--prefix", default="so", help="the name the written files start with")
    parser.add_argument("--only-ids", type=Path, help="a Tatoeba authors file (.tsv.xz): only the sentences it names")
    args = parser.parse_args(argv)
    print(json.dumps(select(args.messages, args.members, args.name, args.development_share, args.out, args.prefix,
                            args.only_ids), indent=1))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
