#!/usr/bin/env python3
"""Messages of fresh ru.stackoverflow comments for the developer-chat test (tools/evaluate_devchat_test.py).

The comments were written in 2024-2026, after the pinned 2023 dump (IlyaGusev/ru_stackoverflow) that earlier models
were fitted on, and fetched through the public Stack Exchange API (CC BY-SA 4.0); `posts/question-of-post.json` names
the question of each commented post. Each comment is cut into messages as mixed_typing cuts posts (post_messages), and
the messages go to the development or the test split by their QUESTION (DEVCHAT_NAMESPACE), so the comments of one
discussion never sit on both sides. Dropped: a message with a code-like token (a sign of DEVCHAT_CODE_SIGNS, or three
ASCII signs in a row that are not prose punctuation alone), a message the 2023 dump holds exactly, and a message where
`c`/`C` stands alone among Russian words or `с`/`С` among Latin ones: both letters lie on one key, so the source's typo
looks like a word of the other layout.

Beside each split's messages a members file lists, per message, the SHA-256 of its text, its question and its mix: a
read of the test draws and claims its messages from the members alone, before any text is opened. Test text is written,
never printed; only counts are.

    python3 tools/build_devchat_messages.py --comments <dir> --dump <ru_stackoverflow.jsonl.zst> --out <dir>
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import re
import subprocess
import sys
from collections import Counter
from collections.abc import Iterable, Iterator, Sequence
from pathlib import Path
from typing import Final

ROOT: Final = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "src"), str(ROOT / "tests"), str(ROOT / "tools")]

from keyswitch.constants.corpus import (  # noqa: E402
    DEVCHAT_CODE_SIGNS, DEVCHAT_DUMP_SHA256, DEVCHAT_NAMESPACE, DEVCHAT_PROSE_SIGNS, DEVCHAT_SIGN_RUN, DEVCHAT_SPLIT_HEX_DIGITS,
)
from keyswitch.constants.file_formats import HEXADECIMAL_BASE  # noqa: E402
from mixed_typing import HAS_CYRILLIC, HAS_LATIN, post_messages  # noqa: E402

DEVELOPMENT: Final = "dev"
TEST: Final = "test"
MIXED: Final = "mixed"
RUSSIAN: Final = "russian"
TAG: Final = re.compile(r"<[^>]+>")
SIGN_RUN: Final = re.compile(r"[!-/:-@\[-`{-~]{%d,}" % DEVCHAT_SIGN_RUN)


def split_of(question: int) -> str:
    draw = int(hashlib.sha256(f"{DEVCHAT_NAMESPACE}:{question}".encode()).hexdigest()[:DEVCHAT_SPLIT_HEX_DIGITS], HEXADECIMAL_BASE)
    return TEST if draw % len((DEVELOPMENT, TEST)) else DEVELOPMENT


def text_key(text: str) -> str:
    return hashlib.sha256(text.encode()).hexdigest()


def code_like(token: str) -> bool:
    return any(sign in token for sign in DEVCHAT_CODE_SIGNS) or any(set(run.group()) - set(DEVCHAT_PROSE_SIGNS)
                                                                    for run in SIGN_RUN.finditer(token))


def lookalike(tokens: Sequence[str]) -> bool:
    """`c` alone among Russian words, or `с` alone among Latin ones: the letters share a key."""

    for index, token in enumerate(tokens):
        neighbours = [*tokens[max(0, index - 1):index], *tokens[index + 1:][:1]]
        if not neighbours:
            continue
        if token in ("c", "C") and all(HAS_CYRILLIC.search(word) for word in neighbours):
            return True
        if token in ("с", "С") and all(HAS_LATIN.search(word) and not HAS_CYRILLIC.search(word) for word in neighbours):
            return True
    return False


def dump_texts(dump: Path) -> Iterator[str]:
    """Every question, answer and comment of the 2023 dump."""

    with subprocess.Popen(["zstd", "-dc", str(dump)], stdout=subprocess.PIPE) as process:
        assert process.stdout is not None
        for line in process.stdout:
            record = json.loads(line)
            answers = record.get("answers", [])
            yield record.get("text_markdown") or ""
            yield from (answer.get("text_markdown") or "" for answer in answers)
            yield from (comment.get("text") or "" for comment in record.get("comments", []))
            yield from (comment.get("text") or "" for answer in answers for comment in answer.get("comments", []))


def messages_of(text: str) -> list[str]:
    # Tags first: mixed_typing.clean removes URLs before tags, and a link's URL swallowed the `">` of its own tag.
    return [" ".join(tokens) for tokens in post_messages(TAG.sub(" ", text))]


def build(comments: Iterable[dict[str, object]], question_of: dict[int, int], known: set[str], out: Path) -> dict[str, int]:
    counts: Counter[str] = Counter()
    seen: set[str] = set()
    out.mkdir(parents=True, exist_ok=True)
    handles = {(split, kind): gzip.open(out / f"devchat-{split}.{kind}.jsonl.gz", "wt", encoding="utf-8")
               for split in (DEVELOPMENT, TEST) for kind in ("messages", "members")}
    try:
        for comment in comments:
            post = int(str(comment["post_id"]))
            question = question_of.get(post, post)
            for text in messages_of(str(comment.get("body") or "")):
                if text in seen:
                    continue
                seen.add(text)
                tokens = text.split()
                reason = ("code" if any(code_like(token) for token in tokens) else "dump" if text_key(text) in known
                          else "lookalike" if lookalike(tokens) else None)
                if reason is not None:
                    counts[f"dropped:{reason}"] += 1
                    continue
                split = split_of(question)
                mix = MIXED if any(HAS_LATIN.search(token) for token in tokens) else RUSSIAN
                handles[split, "messages"].write(json.dumps({"qid": question, "kind": "comment", "mix": mix, "text": text},
                                                            ensure_ascii=False) + "\n")
                handles[split, "members"].write(json.dumps({"text": text_key(text), "question": question, "mix": mix}) + "\n")
                counts[f"{split}:{mix}"] += 1
    finally:
        for handle in handles.values():
            handle.close()
    return dict(sorted(counts.items()))


def content_checksum(path: Path) -> str:
    """The SHA-256 of a gzip file's content: its header holds a time, so its own bytes differ from build to build."""

    with gzip.open(path, "rb") as handle:
        return hashlib.sha256(handle.read()).hexdigest()


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--comments", type=Path, required=True, help="the fetched comments-*.json.gz and posts/")
    parser.add_argument("--dump", type=Path, required=True, help="the pinned 2023 dump")
    parser.add_argument("--out", type=Path, required=True)
    arguments = parser.parse_args(argv)
    if hashlib.sha256(arguments.dump.read_bytes()).hexdigest() != DEVCHAT_DUMP_SHA256:
        print("the 2023 dump differs from the pinned one", flush=True)
        return 1
    known = {text_key(text) for source in dump_texts(arguments.dump) for text in messages_of(source)}
    question_of = {int(post): int(question) for post, question in
                   json.loads((arguments.comments / "posts" / "question-of-post.json").read_text("utf-8")).items()}
    comments = [comment for path in sorted(arguments.comments.glob("comments-*.json.gz"))
                for comment in json.loads(gzip.open(path).read())]
    counts = build(comments, question_of, known, arguments.out)
    receipt = {"namespace": DEVCHAT_NAMESPACE, "split": "by question", "counts": counts,
               "content_sha256": {path.name: content_checksum(path) for path in sorted(arguments.out.glob("devchat-*.jsonl.gz"))},
               "comments_sha256": {path.name: hashlib.sha256(path.read_bytes()).hexdigest()
                                   for path in sorted(arguments.comments.glob("comments-*.json.gz"))},
               "posts_sha256": hashlib.sha256((arguments.comments / "posts" / "question-of-post.json").read_bytes()).hexdigest(),
               "dump_sha256": DEVCHAT_DUMP_SHA256, "builder_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest()}
    (arguments.out / "receipt.json").write_text(json.dumps(receipt, indent=1) + "\n", encoding="utf-8")
    print(json.dumps(counts), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
