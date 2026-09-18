#!/usr/bin/env python3
"""Freeze model-blind natural KEEP rows from pinned official UD treebanks.

No dictionaries or classifiers select rows. A document/lexeme union is audited
before deterministic conflict quarantine. Test text is written, never printed.
The frozen output is prospective sequence evidence, not globally unseen words.
"""

from __future__ import annotations

import argparse
from collections import Counter, defaultdict
from collections.abc import Iterable, Iterator, Mapping, Sequence
from dataclasses import asdict, dataclass, replace
import gzip
import hashlib
import heapq
import json
from pathlib import Path
import re
from typing import cast
import unicodedata

from keyswitch.layouts import LayoutPair
from model_protocol import ALL_SPLITS


ROOT = Path(__file__).resolve().parents[1]
PINS = {
    "UD_Russian-Taiga": "d1e48cd29c1f19b6aad1aec361d9f724db764f66",
    "UD_English-EWT": "b7711cce01cdd4f5fcc0a8199b8a50d951b16c0c",
}
PAIR = LayoutPair()
WORDS = re.compile(r"[^\W\d_]+(?:['’\-][^\W\d_]+)*", re.UNICODE)


def canonical(value: object) -> bytes:
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")) + "\n").encode()


def digest(value: str) -> str:
    return hashlib.sha256(value.encode()).hexdigest()


def checksum(path: Path) -> str:
    result = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(chunk)
    return result.hexdigest()


@dataclass(frozen=True, slots=True)
class CorpusRow:
    identifier: str
    original: str
    group: int | None
    before: str
    after: str
    lemma: str
    family: str
    document: str
    language: str
    source: str
    source_file: str
    source_sentence: str
    source_token: str
    spacing: str
    space_before: str
    literal_tail: str
    upos: str
    features: str
    misc: str
    alignment: str
    layout_representable: bool
    split: str = ""
    quarantine_reasons: tuple[str, ...] = ()


@dataclass(frozen=True, slots=True)
class SurfaceToken:
    identifier: str
    form: str
    lemmas: tuple[str, ...]
    upos: str
    features: str
    misc: str


@dataclass(frozen=True, slots=True)
class Sentence:
    identifier: str
    document: str
    source: str
    filename: str
    text: str | None
    tokens: tuple[SurfaceToken, ...]


class Union:
    def __init__(self) -> None:
        self.parents: dict[str, str] = {}

    def find(self, value: str) -> str:
        self.parents.setdefault(value, value)
        path: list[str] = []
        while self.parents[value] != value:
            path.append(value)
            value = self.parents[value]
        for previous in path:
            self.parents[previous] = value
        return value

    def join(self, left: str, right: str) -> None:
        a, b = self.find(left), self.find(right)
        if a != b:
            self.parents[max(a, b)] = min(a, b)


def physical(token: str) -> str:
    normalized = unicodedata.normalize("NFC", token).casefold().replace("’", "'")
    return PAIR.translate(normalized, "ru", "us")


def typo_variants(original: str, identifier: str) -> tuple[str, ...]:
    """Three predetermined internal edits, included in family grouping first."""

    if len(original) < 4:
        return ()
    inside = 1 + int(digest("delete-duplicate:" + identifier)[:16], 16) % (len(original) - 2)
    adjacent = 1 + int(digest("transpose:" + identifier)[:16], 16) % (len(original) - 3)
    variants = (
        original[:inside] + original[inside + 1:],
        original[:adjacent] + original[adjacent + 1] + original[adjacent] + original[adjacent + 2:],
        original[:inside] + original[inside] + original[inside:],
    )
    return tuple(dict.fromkeys(variant for variant in variants if variant != original))


def row_identifier(sentence: Sentence, token: SurfaceToken) -> str:
    return sentence.source + ":" + sentence.filename + ":" + sentence.identifier + ":" + token.identifier


def family_aliases(token: SurfaceToken, identifier: str = "") -> tuple[str, ...]:
    # Surface and every annotated lemma are joined, including MWT components.
    values = {physical(token.form), *(physical(lemma) for lemma in token.lemmas),
              *(physical(variant) for variant in typo_variants(token.form, identifier))}
    return tuple(sorted(values))


def misc_values(misc: str) -> dict[str, str]:
    return dict(part.split("=", 1) for part in misc.split("|") if "=" in part)


def unescape_space(value: str) -> str:
    escapes = {"s": " ", "t": "\t", "n": "\n", "r": "\r", "p": "|", "\\": "\\"}
    return re.sub(r"\\([stnrp\\])", lambda match: escapes[match[1]], value)


def surface_tokens(lines: list[list[str]]) -> tuple[SurfaceToken, ...]:
    members = {int(parts[0]): parts for parts in lines if parts[0].isdigit()}
    result: list[SurfaceToken] = []
    covered: set[int] = set()
    for parts in lines:
        identifier, form, lemma, upos, _xpos, features, _head, _rel, _deps, misc = parts
        if "." in identifier:
            continue
        if "-" in identifier:
            begin, end = map(int, identifier.split("-"))
            if begin > end or any(index in covered or index not in members for index in range(begin, end + 1)):
                raise ValueError("invalid or overlapping CoNLL-U multiword token")
            covered.update(range(begin, end + 1))
            lemmas = tuple(members[index][2] if members[index][2] != "_" else members[index][1]
                           for index in range(begin, end + 1))
        elif int(identifier) in covered:
            continue
        else:
            lemmas = (lemma if lemma != "_" else form,)
        result.append(SurfaceToken(identifier, form, lemmas, upos, features, misc))
    return tuple(result)


def read_conllu(path: Path, source: str) -> Iterator[Sentence]:
    document = source + ":unmarked-file:" + path.name
    document_serial = sentence_serial = 0
    comments: dict[str, str] = {}
    tokens: list[list[str]] = []

    def complete() -> Sentence:
        nonlocal sentence_serial
        sentence_serial += 1
        identifier = comments.get("sent_id", path.name + ":sentence:" + str(sentence_serial))
        return Sentence(identifier, document, source, path.name,
                        comments.get("text"), surface_tokens(tokens))

    with path.open(encoding="utf-8") as stream:
        for raw in stream:
            line = raw.rstrip("\r\n")
            if not line:
                if tokens:
                    yield complete()
                comments, tokens = {}, []
                continue
            if line.startswith("#"):
                name, separator, value = line[1:].strip().partition(" = ")
                if name in {"newdoc", "newdoc id", "newdoc_id"}:
                    document_serial += 1
                    document = source + ":" + (value if separator else path.name + ":anonymous:" + str(document_serial))
                comments[name] = value if separator else ""
                continue
            columns = line.split("\t")
            if len(columns) != 10:
                raise ValueError(f"CoNLL-U row must have ten columns: {path.name}")
            tokens.append(columns)
        if tokens:
            yield complete()


def aligned_text(sentence: Sentence) -> tuple[str, list[tuple[int, int]], str]:
    if sentence.text is not None:
        position = 0
        spans = []
        for token in sentence.tokens:
            while position < len(sentence.text) and sentence.text[position].isspace():
                position += 1
            if not sentence.text.startswith(token.form, position):
                break
            spans.append((position, position + len(token.form)))
            position += len(token.form)
        else:
            if not sentence.text[position:].strip():
                return sentence.text, spans, "exact-text-comment"
    text = ""
    spans = []
    for index, token in enumerate(sentence.tokens):
        misc = misc_values(token.misc)
        if index == 0:
            text += unescape_space(misc.get("SpacesBefore", ""))
        start = len(text)
        text += token.form
        spans.append((start, len(text)))
        if "SpacesAfter" in misc:
            text += unescape_space(misc["SpacesAfter"])
        elif misc.get("SpaceAfter") != "No" and index + 1 < len(sentence.tokens):
            text += " "
    status = "misc-reconstructed-no-text" if sentence.text is None else "text-alignment-conflict"
    return text, spans, status


def token_language(form: str) -> tuple[str, int | None, bool]:
    ru = any("а" <= char <= "я" or char == "ё" for char in form.casefold())
    en = any("a" <= char <= "z" for char in form.casefold())
    language, group = ("mixed", None) if ru and en else ("ru", 1) if ru else ("en", 0) if en else ("und", None)
    supported = group is not None and all(
        char.isascii() or "а" <= char <= "я" or char == "ё"
        for char in form.casefold()
    ) and not any(char.isspace() for char in form)
    return language, group, supported


def sentence_rows(sentence: Sentence, families: Union) -> list[CorpusRow]:
    text, spans, alignment = aligned_text(sentence)
    rows = []
    for index, (token, (start, end)) in enumerate(zip(sentence.tokens, spans)):
        previous = spans[index - 1][1] if index else 0
        following = spans[index + 1][0] if index + 1 < len(spans) else len(text)
        tail_end = end
        while tail_end < len(text) and tail_end - end < 64 and not text[tail_end].isalnum():
            tail_end += 1
        language, group, representable = token_language(token.form)
        row = CorpusRow(
            identifier=row_identifier(sentence, token),
            original=token.form, group=group, before=text[max(0, start - 96):start],
            after=text[end:end + 64], lemma=" ".join(token.lemmas),
            family=digest(families.find(physical(token.form))), document=sentence.document,
            language=language, source=sentence.source, source_file=sentence.filename,
            source_sentence=sentence.identifier, source_token=token.identifier,
            spacing=text[end:following], space_before=text[previous:start],
            literal_tail=text[end:tail_end], upos=token.upos, features=token.features,
            misc=token.misc, alignment=alignment, layout_representable=representable,
        )
        rows.append(row)
    return rows


def assigned_split(namespace: str, key: str) -> str:
    bucket = int(digest(namespace + ":" + key)[:16], 16) % 100
    return "train" if bucket < 70 else "development" if bucket < 80 else "calibration" if bucket < 90 else "test"


def intended_window(row: CorpusRow) -> str:
    """The bounded sentence window; spacing and literal_tail are already in it."""

    return row.before + row.original + row.after


def choose_sentences(
    paths: Sequence[tuple[str, Path]], namespace: str,
    max_documents: int, max_sentences: int,
) -> tuple[set[tuple[str, str, str]], dict[str, object]]:
    if max_documents < 1 or max_sentences < 1:
        raise ValueError("document and sentence bounds must be positive")
    documents: dict[str, dict[str, list[tuple[int, str, str]]]] = defaultdict(dict)
    counts: Counter[str] = Counter()
    for source, path in paths:
        for sentence in read_conllu(path, source):
            counts[source] += 1
            heap = documents[source].setdefault(sentence.document, [])
            rank = int(digest(namespace + ":sample:" + sentence.document + ":" + sentence.identifier), 16)
            item = (-rank, path.name, sentence.identifier)
            if len(heap) < max_sentences:
                heapq.heappush(heap, item)
            elif item > heap[0]:
                heapq.heapreplace(heap, item)
    selected: set[tuple[str, str, str]] = set()
    selected_documents: Counter[str] = Counter()
    for source, docs in documents.items():
        ranked = sorted(docs, key=lambda document: digest(namespace + ":document-sample:" + document))[:max_documents]
        selected_documents[source] = len(ranked)
        for document in ranked:
            selected.update((source, filename, identifier) for _rank, filename, identifier in docs[document])
    return selected, {
        "available_documents": {source: len(docs) for source, docs in documents.items()},
        "available_sentences": dict(counts), "selected_documents": dict(selected_documents),
        "selected_sentences": len(selected), "max_documents_per_source": max_documents,
        "max_sentences_per_document": max_sentences,
    }


def exposed_families(repository: Path, additional: Sequence[Path] = ()) -> tuple[set[str], dict[str, str]]:
    paths = [repository / "model/context_v1/scenarios.json",
             *sorted((repository / "model/context_v1").glob("holdout*.json")),
             repository / "model/context_v2/sources/lexical-evidence.json.gz",
             repository / "model/context_v2/sources/tatoeba-cc0-en-ru.tsv.gz", *additional]
    exposed: set[str] = set()
    provenance = {}

    expand_old_edits = False

    def add(text: str) -> None:
        exposed.add(physical(text))
        exposed.update(physical(token) for token in WORDS.findall(text))
        if expand_old_edits:
            for token in WORDS.findall(text):
                if 4 <= len(token) <= 64:
                    for index in range(1, len(token) - 1):
                        exposed.add(physical(token[:index] + token[index + 1:]))
                        exposed.add(physical(token[:index] + token[index] + token[index:]))
                    for index in range(1, len(token) - 2):
                        exposed.add(physical(token[:index] + token[index + 1] + token[index] + token[index + 2:]))

    def walk(value: object) -> None:
        if isinstance(value, str):
            add(value)
        elif isinstance(value, list):
            for child in value:
                walk(child)
        elif isinstance(value, dict):
            for key, child in value.items():
                if isinstance(key, str) and key.startswith("["):
                    parts = json.loads(key)
                    if isinstance(parts, list) and len(parts) >= 2 and isinstance(parts[1], str):
                        add(parts[1])
                elif key in {"word", "original", "alternative", "text", "before", "after", "expected"}:
                    walk(child)
                elif isinstance(child, (list, dict)):
                    walk(child)

    for path in dict.fromkeys(paths):
        if not path.is_file():
            raise ValueError(f"exposure input is absent: {path}")
        provenance[str(path.relative_to(repository)) if path.is_relative_to(repository) else str(path)] = checksum(path)
        expand_old_edits = path.parent.name == "context_v1"
        if path.name.endswith(".tsv.gz"):
            with gzip.open(path, "rt", encoding="utf-8") as stream:
                for line in stream:
                    fields = line.rstrip("\n").split("\t")
                    if len(fields) != 4:
                        raise ValueError("invalid prior context TSV")
                    add(fields[2])
        else:
            raw = gzip.decompress(path.read_bytes()) if path.suffix == ".gz" else path.read_bytes()
            walk(json.loads(raw))
    return exposed, provenance


def partition(
    sentences: Sequence[Sentence], namespace: str, exposed: set[str],
) -> tuple[list[CorpusRow], dict[str, object]]:
    families = Union()
    for sentence in sentences:
        for token in sentence.tokens:
            aliases = family_aliases(token, row_identifier(sentence, token))
            for alias in aliases:
                families.join(aliases[0], alias)
    exposed_roots = {families.find(alias) for alias in exposed if alias in families.parents}
    rows = [row for sentence in sentences for row in sentence_rows(sentence, families)]
    if len({row.identifier for row in rows}) != len(rows):
        raise ValueError("duplicate source row identifiers")
    full = Union()
    documents = {row.document for row in rows}
    for row in rows:
        full.join("document:" + row.document, "family:" + row.family)
    component_documents = Counter(full.find("document:" + document) for document in documents)
    giant = max(component_documents.values(), default=0)
    conflict_mode = bool(documents) and giant > max(1, len(documents) // 2)
    assignment = {
        document: assigned_split(namespace, "document:" + document if conflict_mode else full.find("document:" + document))
        for document in documents
    }
    splits_by_family: dict[str, set[str]] = defaultdict(set)
    for row in rows:
        splits_by_family[row.family].add(assignment[row.document])
    exposed_ids = {digest(root) for root in exposed_roots}
    result = []
    reasons_count: Counter[str] = Counter()
    for row in rows:
        split = assignment[row.document]
        reasons = []
        if conflict_mode and len(splits_by_family[row.family]) > 1:
            reasons.append("family-crosses-document-splits")
        if split != "train" and row.family in exposed_ids:
            reasons.append("family-previously-exposed")
        if row.alignment != "exact-text-comment":
            reasons.append(row.alignment)
        if reasons:
            split = "quarantine"
            reasons_count.update(reasons)
        result.append(replace(row, split=split, quarantine_reasons=tuple(reasons)))
    return sorted(result, key=lambda row: row.identifier), {
        "mode": "document-hash-with-family-conflict-quarantine" if conflict_mode else "whole-document-lexeme-components",
        "documents": len(documents), "union_components": len(component_documents),
        "largest_component_documents": giant, "family_components": len(splits_by_family),
        "exposed_family_components": len(exposed_ids), "quarantine_reasons": dict(reasons_count),
        "policy": "70/10/10/10 by SHA256; conflicting rows retained in quarantine; no model or dictionary scores",
    }


def source_inventory(source_root: Path) -> tuple[list[tuple[str, Path]], list[dict[str, object]]]:
    paths = []
    metadata = []
    for source, commit in PINS.items():
        directory = source_root / source
        pin = cast(dict[str, object], json.loads((directory / "pin.json").read_bytes()))
        if pin.get("commit") != commit or pin.get("repository") != "https://github.com/UniversalDependencies/" + source:
            raise ValueError("source repository or pinned commit differs")
        entries = pin.get("files")
        if not isinstance(entries, list):
            raise ValueError("invalid pinned source inventory")
        for entry in entries:
            if not isinstance(entry, dict):
                raise ValueError("invalid pinned file record")
            item = cast(dict[str, object], entry)
            name = item.get("path")
            if not isinstance(name, str) or Path(name).name != name or not name.endswith((".conllu", ".md", ".txt")):
                raise ValueError("invalid pinned source filename")
            path = directory / name
            if checksum(path) != item["sha256"]:
                raise ValueError(f"pinned source checksum differs: {source}/{name}")
            content = path.read_bytes()
            git_blob = hashlib.sha1(b"blob " + str(len(content)).encode() + b"\0" + content).hexdigest()
            if git_blob != item["sha"]:
                raise ValueError("source content disagrees with pinned Git tree blob")
            if name.endswith(".conllu"):
                paths.append((source, path))
        metadata.append({**pin, "pin_metadata_sha256": checksum(directory / "pin.json")})
    return sorted(paths, key=lambda item: (item[0], item[1].name)), metadata


def freeze(
    paths: Sequence[tuple[str, Path]], output: Path, namespace: str,
    exposed: set[str], exposure_provenance: Mapping[str, str],
    sources: Sequence[Mapping[str, object]], max_documents: int = 2000,
    max_sentences: int = 12,
) -> dict[str, object]:
    if output.exists():
        raise ValueError("refusing to overwrite a frozen corpus directory")
    generator_source = Path(__file__).read_bytes()
    selected, sampling = choose_sentences(paths, namespace, max_documents, max_sentences)
    sentences = [sentence for source, path in paths for sentence in read_conllu(path, source)
                 if (source, path.name, sentence.identifier) in selected]
    rows, partitioning = partition(sentences, namespace, exposed)
    if Path(__file__).read_bytes() != generator_source:
        raise ValueError("freezer source changed while producing this corpus")
    output.mkdir(parents=True)
    (output / "generator-source.py").write_bytes(generator_source)
    files = {}
    for split in ALL_SPLITS:
        path = output / (split + ".jsonl.gz")
        raw_digest = hashlib.sha256()
        count = 0
        with path.open("wb") as destination, gzip.GzipFile(fileobj=destination, filename="", mode="wb", mtime=0) as compressed:
            for row in rows:
                if row.split == split:
                    content = canonical(asdict(row))
                    raw_digest.update(content)
                    compressed.write(content)
                    count += 1
        files[split] = {"path": path.name, "sha256": checksum(path),
                        "content_sha256": raw_digest.hexdigest(), "rows": count}
    held_out = [row for row in rows if row.split == "test"]
    membership = {
        "namespace": namespace, "scope": "prospective sequence holdout; not globally unseen lexicon",
        "row_ids_sha256": sorted(digest(row.identifier) for row in held_out),
        "family_ids_sha256": sorted({row.family for row in held_out}),
        "document_ids_sha256": sorted({digest(row.document) for row in held_out}),
    }
    membership_path = output / "test-membership.json"
    membership_path.write_bytes(canonical(membership))
    manifest = {
        "schema_version": 1, "namespace": namespace, "label": "keep",
        "scope": "prospective document/physical-lemma sequence holdout; not globally unseen lexicon or verified human language intent",
        "token_language": "surface alphabet determines RU/Latin group; foreign Latin text is not asserted to be English; mixed/und group is null",
        "context": "within the same sentence only; before<=96 and after<=64 characters; punctuation/whitespace preserved",
        "sources": list(sources), "sampling": sampling, "partitioning": partitioning,
        "exposure_inputs": dict(exposure_provenance),
        "exposed_physical_families": len(exposed),
        "exposed_membership_sha256": digest("\n".join(sorted(exposed))),
        "typo_policy": "only exported typo_variants(original,identifier); all three aliases joined before splitting; old context-v1 scenarios quarantine every internal deletion/duplication/transposition",
        "splits": files, "test_membership_sha256": checksum(membership_path),
        "generator_sha256": hashlib.sha256(generator_source).hexdigest(),
        "generator_source": "generator-source.py",
        "layout_table_sha256": checksum(ROOT / "src/keyswitch/layouts.py"),
        "compression": "gzip mtime=0; raw content SHA256 is independent of deflate implementation",
        "test_policy": "sealed before model scoring; loader reads only the explicitly named split; quarantine is never a training split",
        "limitations": ["UD text is naturally occurring, not reviewed keyboard-intent ground truth",
                        "unmarked source documents are conservatively grouped by file; anonymous newdoc boundaries remain local to that file",
                        "past morphology, dictionaries, private experiments and pretrained models are not exhaustively inventoried",
                        "identical documents under different upstream IDs and paraphrases are not detected"],
    }
    (output / "manifest.json").write_bytes(canonical(manifest))
    return manifest


def load_split(directory: Path, split: str) -> list[CorpusRow]:
    """Read exactly one requested split, never a neighbouring train/test file."""

    if split not in ALL_SPLITS:
        raise ValueError("unknown corpus split")
    manifest = json.loads((directory / "manifest.json").read_bytes())
    record = manifest["splits"][split]
    if record["path"] != split + ".jsonl.gz":
        raise ValueError("unexpected split filename")
    path = directory / record["path"]
    if checksum(path) != record["sha256"]:
        raise ValueError("split compressed checksum mismatch")
    content_hash = hashlib.sha256()
    rows = []
    with gzip.open(path, "rb") as stream:
        for line in stream:
            content_hash.update(line)
            value = json.loads(line)
            value["quarantine_reasons"] = tuple(value["quarantine_reasons"])
            row = CorpusRow(**value)
            if row.split != split or len(row.before) > 96 or len(row.after) > 64:
                raise ValueError("invalid corpus row bounds or split")
            rows.append(row)
    if len(rows) != record["rows"] or content_hash.hexdigest() != record["content_sha256"]:
        raise ValueError("split content checksum or row count mismatch")
    return rows


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--namespace", required=True)
    parser.add_argument("--repository", type=Path, default=ROOT)
    parser.add_argument("--extra-exposure", type=Path, action="append", default=[])
    parser.add_argument("--max-documents", type=int, default=2000)
    parser.add_argument("--max-sentences-per-document", type=int, default=12)
    args = parser.parse_args(argv)
    paths, sources = source_inventory(args.source_root)
    exposed, provenance = exposed_families(args.repository, args.extra_exposure)
    manifest = freeze(paths, args.output, args.namespace, exposed, provenance, sources,
                      args.max_documents, args.max_sentences_per_document)
    print(json.dumps({"namespace": args.namespace, "sampling": manifest["sampling"],
                      "partitioning": manifest["partitioning"],
                      "splits": manifest["splits"], "test_membership_sha256": manifest["test_membership_sha256"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
