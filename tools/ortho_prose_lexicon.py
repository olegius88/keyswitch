#!/usr/bin/env python3
"""Freeze the Russian word forms of public-domain prose that the orthotactic model counts.

Generation 12 found a class of false conversions that the web vocabulary (`ortho_web_lexicon`) does not
cover: Russian words of literature that no dictionary lists - obsolete loans, rare forms, sounds put on
the page - whose keys read English enough to be turned into garbage. Counting the word forms of
public-domain Russian prose teaches the Russian model those sequences, in the channel that scores a
token typed in Russian only (`ortho_model`, `source_models`), as the web forms do: they can keep a
Cyrillic word but never argue that Latin keys were meant as Russian.

The source is a dump of the Russian Wikisource (`DUMP`: its articles and its page, linktarget and
categorylinks tables). The selection (`select`): the prose categories of `Категория:Проза по авторам`
whose author page (namespace `Автор`) is in `Писатели на русском языке` and in `Умершие в N году` with N
no later than ORTHO_PROSE_LAST_DEATH_YEAR - public domain under a term of life plus a century -, except
the authors whose text an opened set or the independent set holds (`EXCLUDED_AUTHORS`); every article in
such a category and its subcategories, ORTHO_PROSE_CATEGORY_DEPTH levels down, except pre-reform versions
(a `/ДО` page, or any page with a pre-reform letter), translations (a page header that names a translator)
and pages whose header names another author. The text is read as the independent sets read Wikisource
(`wikisource_text.wikitext_sentences`); the forms are the web lexicon's (`ortho_web_lexicon.FORM`,
`COMPOUND`): lowercase Cyrillic words of ORTHO_WEB_LEXICON_MIN_CHARACTERS letters or more and lowercase
compounds with hyphens, as the text writes them, so that names and abbreviations stay out. A form weighs
like an Onboard word: the bit length of its count per ORTHO_PROSE_WORDS_SCALE words of the corpus, from 1
to the word-list cap.

The dump is large and is not checked in; the derived forms are, with a receipt that pins the dump's files.
`--freeze DIR` derives them once; a plain run checks the derived file against its receipt; `--source DIR`
also derives them again and compares.
"""

from __future__ import annotations

import argparse
import bz2
import gzip
import hashlib
import json
import re
import sys
from collections import Counter, defaultdict
from collections.abc import Iterator, Sequence
from pathlib import Path
from typing import Final, cast
from xml.etree import ElementTree

sys.path.insert(0, str(Path(__file__).resolve().parent))

import ortho_web_lexicon  # noqa: E402
from wikisource_text import wikitext_sentences  # noqa: E402

from keyswitch.constants.file_formats import FROZEN_CORPUS_RECEIPT_JSON_INDENT  # noqa: E402
from keyswitch.constants.ortho import (  # noqa: E402
    MEDIAWIKI_ARTICLE_NAMESPACE,
    MEDIAWIKI_CATEGORY_NAMESPACE,
    ORTHO_PROSE_CATEGORY_DEPTH,
    ORTHO_PROSE_LAST_DEATH_YEAR,
    ORTHO_PROSE_LEXICON_SCHEMA_VERSION,
    ORTHO_PROSE_WORDS_SCALE,
    ORTHO_V1_MAXIMUM_LEXICON_WEIGHT,
    ORTHO_WEB_LEXICON_MIN_CHARACTERS,
    WIKISOURCE_AUTHOR_NAMESPACE,
)

ROOT: Final[Path] = Path(__file__).resolve().parents[1]
DIRECTORY: Final[Path] = ROOT / "model" / "ortho_v1"
FORMS: Final[Path] = DIRECTORY / "prose-lexicon-ru.json.gz"
RECEIPT: Final[Path] = DIRECTORY / "prose-lexicon-receipt.json"
DUMP: Final[str] = "ruwikisource-20260901"
DUMP_URL: Final[str] = f"https://dumps.wikimedia.org/ruwikisource/{DUMP.rsplit('-', 1)[1]}"
RETRIEVED: Final[str] = "2026-09-27T14:39+05:00"
# The dump's files: SHA-1 as the dump's own list publishes it, SHA-256 as recorded at download.
FILES: Final[dict[str, tuple[str, str]]] = {
    "pages-articles.xml.bz2": ("08d84a5193cb1c59dbdb652c3c68761d2d574fcf",
                               "900444fd4dea093a918908390ec93130575fa530dddbec327c89313a65f7fc81"),
    "page.sql.gz": ("73fb70b22a3f1e82d3ba76fc0cafc10699a30f63",
                    "4fc8ee33447f4c864086afbbbb492e46486224dbd176e37f22933dac6a2b5b5d"),
    "categorylinks.sql.gz": ("4e8583c39575edab33ef1f1f21620257994272f6",
                             "356b33624d80ca13f82ce3dc50fbab53229d42bae96ed70019e792e564150a7e"),
    "linktarget.sql.gz": ("318825703fefb9c98871b5e2980f9dbad6f310c6",
                          "5927a112017995d8632f971fe45653e79abb9ff584ea72be5447483dc29149de"),
}
LICENSE: Final[str] = (
    "public domain: works of authors who died no later than the year named in the selection, under any copyright "
    "term of life plus a century or shorter; translations and pages naming another author are left out"
)
PROSE_CATEGORY: Final[str] = "Проза_по_авторам"
RUSSIAN_WRITERS: Final[str] = "Писатели_на_русском_языке"
DEATH: Final[re.Pattern[str]] = re.compile(r"Умершие_в_(\d+)_году")
# Chekhov's stories are an opened test; Leskov's and Averchenko's are the independent set of generation 12.
EXCLUDED_AUTHORS: Final[tuple[str, ...]] = ("Антон_Павлович_Чехов", "Николай_Семёнович_Лесков",
                                            "Аркадий_Тимофеевич_Аверченко")
PRE_REFORM_LETTERS: Final[re.Pattern[str]] = re.compile("[ѣѢіІѳѲѵѴ]")
PRE_REFORM_PAGE: Final[str] = "/ДО"
HEADER: Final[re.Pattern[str]] = re.compile(r"\{\{Отексте(.*?)\n\}\}", re.DOTALL)
FIELD: Final[re.Pattern[str]] = re.compile(r"\|\s*([А-ЯЁA-Z]+)\s*=\s*([^|\n]*)")
EDGE: Final[re.Pattern[str]] = re.compile(r"^[^\w-]+|[^\w-]+$")
_SQL_ESCAPES: Final[dict[bytes, bytes]] = {b"n": b"\n", b"t": b"\t", b"r": b"\r", b"0": b"\0", b"Z": b"\x1a"}
_SQL_NUMBER: Final[re.Pattern[bytes]] = re.compile(rb"-?\d+")
_SQL_COLUMN: Final[re.Pattern[bytes]] = re.compile(rb"^\s+`(\w+)`")

SqlValue = bytes | int | None


def checksum(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def sql_rows(path: Path, table: str) -> Iterator[dict[str, SqlValue]]:
    """The rows of a MediaWiki SQL dump of `table`, by the column names of its CREATE TABLE statement:
    quoted fields as bytes, numbers as ints, NULL as None."""

    create, start = f"CREATE TABLE `{table}` (".encode(), f"INSERT INTO `{table}` VALUES ".encode()
    columns: list[str] = []
    declaring = False
    with gzip.open(path, "rb") as handle:
        for line in handle:
            if line.startswith(create):
                declaring = True
            elif declaring and (match := _SQL_COLUMN.match(line)):
                columns.append(match.group(1).decode())
            elif line.startswith(start):
                declaring = False
                yield from (dict(zip(columns, row, strict=True)) for row in _values(line, len(start)))
            else:
                declaring = declaring and not line.startswith(b")")


def _values(line: bytes, index: int) -> Iterator[list[SqlValue]]:
    size = len(line)
    while index < size:
        if line[index:index + 1] != b"(":
            index += 1
            continue
        index += 1
        row: list[SqlValue] = []
        while True:
            value, index = _field(line, index)
            row.append(value)
            closing = line[index:index + 1] == b")"
            index += 1
            if closing:
                break
        yield row


def _field(line: bytes, index: int) -> tuple[SqlValue, int]:
    if line[index:index + 1] != b"'":
        end = index
        while line[end:end + 1] not in (b",", b")"):
            end += 1
        token = line[index:end]
        return (None if token == b"NULL" else int(token) if _SQL_NUMBER.fullmatch(token) else token), end
    index += 1
    chunk = bytearray()
    while line[index:index + 1] != b"'":
        if line[index:index + 1] == b"\\":
            index += 1
            escaped = line[index:index + 1]
            chunk += _SQL_ESCAPES.get(escaped, escaped)
        else:
            chunk += line[index:index + 1]
        index += 1
    return bytes(chunk), index + 1


def text(value: SqlValue) -> str:
    return cast(bytes, value).decode("utf-8")


def select(directory: Path) -> tuple[dict[str, dict[str, object]], dict[str, int], dict[str, int]]:
    """{title: {revid, timestamp, content, author, category}} of the articles the rule selects, the authors
    with their year of death, and the count of articles left out by reason."""

    titles: dict[int, tuple[int, str, bool]] = {}
    for row in sql_rows(directory / f"{DUMP}-page.sql.gz", "page"):
        titles[cast(int, row["page_id"])] = (cast(int, row["page_namespace"]), text(row["page_title"]), bool(row["page_is_redirect"]))
    by_title = {(namespace, title): page for page, (namespace, title, _redirect) in titles.items()}
    targets = {cast(int, row["lt_id"]): text(row["lt_title"])
               for row in sql_rows(directory / f"{DUMP}-linktarget.sql.gz", "linktarget")
               if row["lt_namespace"] == MEDIAWIKI_CATEGORY_NAMESPACE}
    members: dict[str, list[tuple[int, str]]] = defaultdict(list)
    parents: dict[int, list[str]] = defaultdict(list)
    for row in sql_rows(directory / f"{DUMP}-categorylinks.sql.gz", "categorylinks"):
        category = targets.get(cast(int, row["cl_target_id"]))
        if category is not None:
            members[category].append((cast(int, row["cl_from"]), text(row["cl_type"])))
            parents[cast(int, row["cl_from"])].append(category)
    died: dict[str, int] = {}
    for page, (namespace, title, redirect) in titles.items():
        categories = parents.get(page, [])
        years = [int(match.group(1)) for category in categories if (match := DEATH.fullmatch(category))]
        if namespace == WIKISOURCE_AUTHOR_NAMESPACE and not redirect and years and RUSSIAN_WRITERS in categories:
            died[title] = years[0]
    authors: dict[str, str] = {}
    for page, kind in members.get(PROSE_CATEGORY, []):
        namespace, category, _redirect = titles.get(page, (MEDIAWIKI_ARTICLE_NAMESPACE, "", True))
        if kind != "subcat" or namespace != MEDIAWIKI_CATEGORY_NAMESPACE:
            continue
        for parent in parents.get(by_title.get((MEDIAWIKI_CATEGORY_NAMESPACE, category), -1), []):
            if died.get(parent, ORTHO_PROSE_LAST_DEATH_YEAR + 1) <= ORTHO_PROSE_LAST_DEATH_YEAR and parent not in EXCLUDED_AUTHORS:
                authors[category] = parent
    chosen: dict[int, tuple[str, str]] = {}
    for category, author in sorted(authors.items()):
        frontier, seen = [(category, 0)], {category}
        while frontier:
            current, depth = frontier.pop()
            for page, kind in members.get(current, []):
                namespace, title, redirect = titles.get(page, (MEDIAWIKI_ARTICLE_NAMESPACE, "", True))
                if kind == "page" and namespace == MEDIAWIKI_ARTICLE_NAMESPACE and not redirect and not title.endswith(PRE_REFORM_PAGE):
                    chosen.setdefault(page, (category, author))
                elif kind == "subcat" and namespace == MEDIAWIKI_CATEGORY_NAMESPACE and depth < ORTHO_PROSE_CATEGORY_DEPTH and title not in seen:
                    seen.add(title)
                    frontier.append((title, depth + 1))
    pages: dict[str, dict[str, object]] = {}
    dropped: Counter[str] = Counter()
    for page, revision, timestamp, content in articles(directory / f"{DUMP}-pages-articles.xml.bz2", set(chosen)):
        category, author = chosen[page]
        header = HEADER.search(content)
        fields = dict(FIELD.findall(header.group(1))) if header else {}
        if PRE_REFORM_LETTERS.search(content):
            dropped["pre-reform spelling"] += 1
        elif fields.get("ПЕРЕВОДЧИК", "").strip():
            dropped["translation"] += 1
        elif fields.get("АВТОР", "").strip() and author.replace("_", " ") not in fields["АВТОР"]:
            dropped["another author"] += 1
        else:
            pages[titles[page][1].replace("_", " ")] = {"revid": revision, "timestamp": timestamp, "content": content,
                                                        "author": author.replace("_", " "), "category": category.replace("_", " ")}
    return pages, {author.replace("_", " "): died[author] for author in sorted(set(authors.values()))}, dict(sorted(dropped.items()))


def articles(path: Path, wanted: set[int]) -> Iterator[tuple[int, int, str, str]]:
    """(page id, revision id, timestamp, wikitext) of the wanted pages of a pages-articles dump."""

    def name(element: ElementTree.Element) -> str:
        return element.tag.rsplit("}", 1)[-1]

    with bz2.open(path, "rb") as handle:
        for _event, element in ElementTree.iterparse(handle):
            if name(element) != "page":
                continue
            fields = {name(child): child for child in element}
            page = int(cast(str, fields["id"].text))
            if page in wanted:
                revision = {name(child): child for child in fields["revision"]}
                yield (page, int(cast(str, revision["id"].text)), cast(str, revision["timestamp"].text),
                       revision["text"].text or "")
            element.clear()


def forms(pages: dict[str, dict[str, object]]) -> tuple[dict[str, int], int]:
    """{form: weight} of the pages' text and the number of words the weights are scaled by."""

    counts: Counter[str] = Counter()
    words = 0
    for title in sorted(pages):
        for sentence in wikitext_sentences(str(pages[title]["content"])):
            for raw in sentence.split():
                token = EDGE.sub("", raw).strip("-")
                if not any(character.isalpha() for character in token):
                    continue
                words += 1
                if ((len(token) >= ORTHO_WEB_LEXICON_MIN_CHARACTERS and ortho_web_lexicon.FORM.fullmatch(token))
                        or ortho_web_lexicon.COMPOUND.fullmatch(token)):
                    counts[token] += 1
    return {form: prose_weight(count, words) for form, count in sorted(counts.items())}, words


def prose_weight(count: int, words: int) -> int:
    """An Onboard-like weight: the bit length of the count per ORTHO_PROSE_WORDS_SCALE words, from 1 to the cap."""

    return max(1, min(ORTHO_V1_MAXIMUM_LEXICON_WEIGHT, round(count * ORTHO_PROSE_WORDS_SCALE / words).bit_length()))


def pinned(directory: Path) -> None:
    for name, (_sha1, sha256) in FILES.items():
        if checksum(directory / f"{DUMP}-{name}") != sha256:
            raise ValueError(f"{DUMP}-{name} is not the pinned dump file")


def derive(directory: Path) -> tuple[dict[str, int], dict[str, object]]:
    pinned(directory)
    pages, authors, dropped = select(directory)
    table, words = forms(pages)
    listed = "".join(f"{title}\t{pages[title]['revid']}\n" for title in sorted(pages)).encode("utf-8")
    return table, {"authors": authors, "pages": len(pages), "pages_sha256": hashlib.sha256(listed).hexdigest(),
                   "left_out": dropped, "words": words}


def write(table: dict[str, int], path: Path) -> str:
    raw = (json.dumps({"schema_version": ORTHO_PROSE_LEXICON_SCHEMA_VERSION, "forms": table},
                      ensure_ascii=False, sort_keys=True, separators=(",", ":")) + "\n").encode()
    content = gzip.compress(raw, mtime=0)      # no time and no file name in the header: byte-stable
    path.write_bytes(content)
    return hashlib.sha256(content).hexdigest()


def load(path: Path = FORMS) -> dict[str, int]:
    """form -> weight, from the frozen derived file."""

    with gzip.open(path, "rt", encoding="utf-8") as handle:
        payload = cast(dict[str, object], json.load(handle))
    if payload.get("schema_version") != ORTHO_PROSE_LEXICON_SCHEMA_VERSION:
        raise ValueError("unsupported prose lexicon")
    return {str(form): int(cast(int, weight)) for form, weight in cast(dict[str, object], payload["forms"]).items()}


def receipt(table: dict[str, int], summary: dict[str, object], forms_sha: str) -> dict[str, object]:
    return {
        "schema_version": ORTHO_PROSE_LEXICON_SCHEMA_VERSION,
        "source": {"dump": DUMP, "url": DUMP_URL, "retrieved": RETRIEVED, "license": LICENSE,
                   "files": [{"name": f"{DUMP}-{name}", "sha1": sha1, "sha256": sha256} for name, (sha1, sha256) in FILES.items()]},
        "selection": (f"prose categories of {PROSE_CATEGORY} whose author page is in {RUSSIAN_WRITERS} and died in "
                      f"{ORTHO_PROSE_LAST_DEATH_YEAR} or earlier, except {', '.join(EXCLUDED_AUTHORS)}; their articles and "
                      f"those of their subcategories {ORTHO_PROSE_CATEGORY_DEPTH} levels down, except {PRE_REFORM_PAGE} pages, "
                      "pages with pre-reform letters, translations and pages naming another author"),
        **summary,
        "forms": len(table), "forms_sha256": forms_sha,
        "form_rule": (f"lowercase Cyrillic words of {ORTHO_WEB_LEXICON_MIN_CHARACTERS} or more letters and lowercase "
                      "compounds with hyphens, as the text writes them"),
        "weight": f"max(1, min({ORTHO_V1_MAXIMUM_LEXICON_WEIGHT}, round(count * {ORTHO_PROSE_WORDS_SCALE} / words).bit_length()))",
        "use": "counted only into the channel of Russian that scores a token typed in Russian",
    }


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--freeze", type=Path, metavar="DUMP_DIRECTORY")
    parser.add_argument("--source", type=Path, metavar="DUMP_DIRECTORY")
    arguments = parser.parse_args(argv)
    if arguments.freeze is not None:
        if FORMS.exists() or RECEIPT.exists():
            raise ValueError("prose lexicon already frozen; do not overwrite it")
        table, summary = derive(arguments.freeze)
        content = receipt(table, summary, write(table, FORMS))
        RECEIPT.write_text(json.dumps(content, ensure_ascii=False, sort_keys=True,
                                      indent=FROZEN_CORPUS_RECEIPT_JSON_INDENT) + "\n", encoding="utf-8")
    recorded = cast(dict[str, object], json.loads(RECEIPT.read_bytes()))
    if recorded.get("forms_sha256") != checksum(FORMS):
        raise ValueError("prose lexicon does not match its receipt")
    if arguments.source is not None:
        check = FORMS.with_name(".prose-lexicon-check.json.gz")
        try:
            table, _summary = derive(arguments.source)
            if write(table, check) != recorded["forms_sha256"]:
                raise ValueError("prose lexicon is not reproducible from its source")
        finally:
            check.unlink(missing_ok=True)
    print(json.dumps(recorded, ensure_ascii=False, sort_keys=True, indent=FROZEN_CORPUS_RECEIPT_JSON_INDENT))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
