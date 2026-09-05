"""Model-blind CC0 phrase corpus preparation with separate source families."""
from __future__ import annotations

import collections
import argparse
import gzip
import hashlib
import io
import json
import re
import tarfile
import unicodedata
from dataclasses import dataclass
from pathlib import Path
from collections.abc import Iterable, Sequence
from typing import Literal


Locale = Literal["eng", "rus"]
Split = Literal["train", "development", "calibration", "test", "reserve"]
NAMESPACE = "keyswitch:context-v2:phrases-20260905"
ARCHIVE_SHA = "0b4b58b84239c4c194096040258deececa4f6a1ac3b93da16b64ae477b70f98e"
SOURCE_SHA = "1f7df948b946f1d1aed5efd2a83a9a5a09d2b8abcb9224045b40c9ec6720e793"
ROOT = Path(__file__).resolve().parents[1]
CORPUS_ROOT = ROOT / "model/context_v2"
SOURCE = CORPUS_ROOT / "sources/tatoeba-cc0-en-ru.tsv.gz"
RECEIPT = CORPUS_ROOT / "corpus-receipt.json"
WORDS = re.compile(r"[^\W_]+(?:['’\-][^\W_]+)*", re.UNICODE)


@dataclass(frozen=True)
class Phrase:
    identifier: int
    locale: Locale
    text: str
    modified: str


@dataclass(frozen=True)
class AssignedPhrase:
    phrase: Phrase
    group: str
    split: Split


def digest(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def canonical_tokens(text: str) -> tuple[str, ...]:
    normalized = unicodedata.normalize("NFC", text).casefold().replace("ё", "е").replace("’", "'")
    return tuple(WORDS.findall(normalized))


def load_archive(path: Path) -> list[Phrase]:
    if hashlib.sha256(path.read_bytes()).hexdigest() != ARCHIVE_SHA:
        raise ValueError("source archive checksum differs from the reviewed snapshot")
    with tarfile.open(path, "r:bz2") as archive:
        members = archive.getmembers()
        if len(members) != 1 or members[0].name != "sentences_CC0.csv" or not members[0].isfile() or members[0].size > 64 * 1024 * 1024:
            raise ValueError("unexpected CC0 archive member")
        stream = archive.extractfile(members[0])
        if stream is None:
            raise ValueError("missing CC0 data")
        with io.TextIOWrapper(stream, encoding="utf-8", errors="strict") as source:
            return read_phrases(source)


def read_phrases(source: Iterable[str]) -> list[Phrase]:
    phrases: list[Phrase] = []
    seen: set[int] = set()
    for line in source:
        fields = line.rstrip("\n").split("\t")
        if len(fields) != 4:
            raise ValueError("invalid CC0 TSV row")
        identifier, language, text, modified = fields
        if language not in {"eng", "rus"}:
            continue
        numeric = int(identifier)
        if numeric <= 0 or numeric in seen:
            raise ValueError("nonpositive or duplicate source ID")
        seen.add(numeric)
        locale: Locale = "eng" if language == "eng" else "rus"
        phrases.append(Phrase(numeric, locale, text, modified))
    return sorted(phrases, key=lambda item: item.identifier)


def load_source(path: Path = SOURCE) -> list[Phrase]:
    content = path.read_bytes()
    if hashlib.sha256(content).hexdigest() != SOURCE_SHA:
        raise ValueError("frozen CC0 source checksum mismatch")
    with gzip.GzipFile(fileobj=io.BytesIO(content)) as compressed:
        raw = compressed.read(32 * 1024 * 1024 + 1)
    if len(raw) > 32 * 1024 * 1024:
        raise ValueError("oversized CC0 source")
    return read_phrases(raw.decode("utf-8").splitlines(keepends=True))


def assign(phrases: list[Phrase], *, per_group: int = 4) -> tuple[list[AssignedPhrase], dict[str, object]]:
    if per_group < 1:
        raise ValueError("per_group must be positive")
    eligible = sorted((item for item in phrases if 4 <= len(item.text) <= 512 and len(canonical_tokens(item.text)) >= 2 and "\x00" not in item.text), key=lambda item: item.identifier)
    parents = list(range(len(eligible)))

    def root(index: int) -> int:
        while index != parents[index]:
            parents[index] = parents[parents[index]]
            index = parents[index]
        return index

    def merge(left: int, right: int) -> None:
        a, b = root(left), root(right)
        if a != b:
            parents[max(a, b)] = min(a, b)

    signatures: dict[str, int] = {}
    for index, item in enumerate(eligible):
        words = canonical_tokens(item.text)
        variants = [words]
        # Group one-token substitutions/insertions/deletions before splitting.
        # This is a defined near-duplicate check, not semantic paraphrase detection.
        if 4 <= len(words) <= 40:
            variants.extend(words[:offset] + words[offset + 1:] for offset in range(len(words)))
        for variant in variants:
            # Identical/mixed-language text must stay together even when the
            # source's language annotations disagree.
            key = digest("\x1f".join(variant))
            if key in signatures:
                merge(index, signatures[key])
            else:
                signatures[key] = index
    grouped: dict[int, list[Phrase]] = collections.defaultdict(list)
    for index, item in enumerate(eligible):
        grouped[root(index)].append(item)
    output: list[AssignedPhrase] = []
    split_groups: collections.Counter[str] = collections.Counter()
    for group_index, group_phrases in sorted(grouped.items()):
        identifier = f"tatoeba:{eligible[group_index].identifier}"
        bucket = int(digest(NAMESPACE + ":" + identifier)[:8], 16) % 100
        split: Split = "train" if bucket < 60 else "development" if bucket < 70 else "calibration" if bucket < 80 else "test" if bucket < 90 else "reserve"
        split_groups[split] += 1
        # Keep original text, but do not let huge connected template families
        # dominate the objective merely because they have many source IDs.
        selected = sorted(group_phrases, key=lambda item: digest(f"{NAMESPACE}:sample:{item.identifier}"))[:per_group]
        output.extend(AssignedPhrase(item, identifier, split) for item in selected)
    output.sort(key=lambda item: item.phrase.identifier)
    content = "\n".join(f"{item.phrase.identifier}\t{item.group}\t{item.split}" for item in output)
    return output, {
        "source_rows": len(phrases), "eligible_rows": len(eligible), "retained_rows": len(output),
        "groups": len(grouped), "maximum_group_size": max(map(len, grouped.values()), default=0),
        "group_cap": per_group, "groups_by_split": dict(sorted(split_groups.items())),
        "rows_by_split": dict(sorted(collections.Counter(item.split for item in output).items())),
        "locale_rows": dict(sorted(collections.Counter(item.phrase.locale for item in output).items())),
        "assignment_sha256": digest(content), "split_namespace": NAMESPACE,
        "model_loaded": False, "metrics_evaluated": False,
        "grouping_scope": "normalized lexical text and bounded single-token edits; not all paraphrases",
    }


def write_source(phrases: list[Phrase], path: Path) -> str:
    raw = "".join(f"{item.identifier}\t{item.locale}\t{item.text}\t{item.modified}\n" for item in phrases).encode("utf-8")
    content = gzip.compress(raw, mtime=0)
    path.write_bytes(content)
    return hashlib.sha256(content).hexdigest()


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--import-archive", type=Path)
    parser.add_argument("--freeze", action="store_true")
    args = parser.parse_args(argv)
    if args.import_archive:
        phrases = load_archive(args.import_archive)
        SOURCE.parent.mkdir(parents=True, exist_ok=True)
        if SOURCE.exists():
            load_source()
        else:
            if write_source(phrases, SOURCE) != SOURCE_SHA:
                raise ValueError("imported snapshot is not byte-reproducible")
    source = load_source()
    _rows, report = assign(source)
    report.update({
        "schema_version": 1, "source_sha256": SOURCE_SHA, "archive_sha256": ARCHIVE_SHA,
        "source_url": "https://downloads.tatoeba.org/exports/sentences_CC0.tar.bz2",
        "license": "CC0-1.0", "license_evidence": "https://tatoeba.org/en/downloads",
        "builder_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
    })
    content = (json.dumps(report, sort_keys=True, ensure_ascii=False, indent=2) + "\n").encode()
    if args.freeze and not RECEIPT.exists():
        RECEIPT.write_bytes(content)
    elif not RECEIPT.exists() or RECEIPT.read_bytes() != content:
        raise ValueError("corpus receipt is missing or changed; do not overwrite a frozen split")
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
