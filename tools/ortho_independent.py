#!/usr/bin/env python3
"""Independent tests of the sealed orthotactic candidate, from text the development never saw.

A candidate is developed on the visible parts of several splits of the frozen snapshot, which together
hold almost every phrase group, so a test drawn from the snapshot again would test it on data it was
tuned on. An independent test comes from other text. Its downloaded files are pinned by SHA-256, and its
population is built by the corpus' own code - `ortho_corpus.label` (with the reading of the letter core),
`to_keys`, `shape_of`, `train_ortho_model.population` - and a dictionary verdict of its own, frozen with
the reference dictionaries (`ortho_known.reference_models`, `queried`). Then every type the development
may have seen is left out: its served keys, with every run of a key read once, match those of a row of
the snapshot, of a set an earlier test opened, or of a case written by hand - so a stretched or doubled
spelling of a seen word counts as seen.

A set is frozen once, before any model is scored on it, and tested once. Its single test opens it: from
then on it is development data and stays as history - its receipt, its files and the report of that test
are held to each other, while the code that built it has moved on.

`--freeze DIR` builds the active set from the downloaded files in DIR; a plain run checks every opened
set and the active one and prints the active receipt; `test` is the single test of the sealed candidate
on the active set: it writes the trainer's report (`model/ortho_v1/report.json`) with an
`independent_test` block, so `train_ortho_model.py promote` and `verify_ortho_model.py` accept it, and it
refuses when a report exists. The candidate and its seal are not touched.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
import unicodedata
from collections import Counter
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Final, cast

sys.path.insert(0, str(Path(__file__).resolve().parent))

import ortho_corpus  # noqa: E402
import ortho_known  # noqa: E402
import train_ortho_model as trainer  # noqa: E402
from wikisource_text import wikitext_sentences  # noqa: E402

from keyswitch.constants.file_formats import (  # noqa: E402
    FROZEN_CORPUS_RECEIPT_JSON_INDENT,
    HEXADECIMAL_BASE,
    REPORT_JSON_INDENT,
)
from keyswitch.constants.model_protocol import TEST  # noqa: E402
from keyswitch.constants.ortho import (  # noqa: E402
    ORTHO_INDEPENDENT_ENGLISH_SAMPLE_PERCENT,
    ORTHO_INDEPENDENT_RECEIPT_SCHEMA_VERSION,
    ORTHO_INDEPENDENT_TAKEN_ENGLISH_PERCENT,
    ORTHO_INDEPENDENT_WIKISOURCE_SAMPLE_PERCENT,
    ORTHO_MIN_STRETCH_RUN,
    ORTHO_V1_EVIDENCE_SCHEMA_VERSION,
    ORTHO_V1_MAXIMUM_TOKEN_CHARACTERS,
    ORTHO_V1_PRINTED_REPORT_MAX_CHARACTERS,
    ORTHO_V1_RECALL_DECIMALS,
)
from keyswitch.constants.training import DETERMINISTIC_CHOICE_HEX_DIGITS  # noqa: E402
from keyswitch.constants.units import PERCENT_SCALE  # noqa: E402
from keyswitch.identifier_lexicon import RESOURCE_PATH as IDENTIFIERS, IdentifierLexicon  # noqa: E402
from keyswitch.ortho_model import SCRIPTS, OrthoModel, read_once  # noqa: E402
from keyswitch.value_provenance import CONSTANTS_PACKAGE, pin_values  # noqa: E402

ROOT: Final[Path] = Path(__file__).resolve().parents[1]
TOKENS_NAME: Final[str] = "tokens.jsonl.gz"
EVIDENCE_NAME: Final[str] = "known-evidence.json.gz"
RECEIPT_NAME: Final[str] = "receipt.json"
DIRECTORY: Final[Path] = ROOT / "model" / "ortho_v1" / "independent"
TOKENS: Final[Path] = DIRECTORY / TOKENS_NAME
EVIDENCE: Final[Path] = DIRECTORY / EVIDENCE_NAME
RECEIPT: Final[Path] = DIRECTORY / RECEIPT_NAME
DEVELOPMENT_HISTORY: Final[Path] = ROOT / "model" / "ortho_v1" / "development-history"
# Sets opened by their single test: the directory of their frozen files and the report of that test.
HISTORY: Final[tuple[tuple[Path, Path], ...]] = (
    (DEVELOPMENT_HISTORY / "independent-common-voice-sentence-collector",
     DEVELOPMENT_HISTORY / "report-ortho-v1-71e34529bdd4.json"),
    (DEVELOPMENT_HISTORY / "independent-wikipedia-en-chekhov-ru",
     DEVELOPMENT_HISTORY / "report-ortho-v1-8723bff1344e.json"),
)
# Sets the development scored that were never a test (their receipts describe them): their rows are seen too.
DEVELOPMENT_SETS: Final[tuple[Path, ...]] = (DEVELOPMENT_HISTORY / "class-development-gorky-bulgakov-kuzmin",)
LINES: Final[str] = "lines"            # one sentence per line
WIKISOURCE: Final[str] = "wikisource"  # a JSON object of Wikisource pages: title -> revid, timestamp, wikitext
COMMON_VOICE_COMMIT: Final[str] = "1ce6d077cad36f99404b5414afacc314df58ad8c"
COMMON_VOICE_RAW: Final[str] = f"https://github.com/common-voice/common-voice/raw/{COMMON_VOICE_COMMIT}/server/data"
COMMON_VOICE_LICENSE: Final[str] = (
    "CC0 1.0: the README of common-voice/common-voice states that the sentence text in /server/data comes from "
    "Sentence Collector submissions or is scraped from Wikipedia with its extractor and is released under CC0 "
    "(files europarl-* are the exception and are not used); server/data/en/LICENSE is the CC0 1.0 text"
)
WIKISOURCE_LICENSE: Final[str] = (
    "Public domain: Arkady Averchenko died in 1925 and Nikolai Leskov in 1895; their author pages on Russian Wikisource "
    "({{АП|ГОД=1925}}, {{АП|ГОД=1895}}) mark their works as public domain where copyright lasts the author's life plus "
    "100 years or less; translations and later editions may carry rights of their own (Civil Code of the Russian "
    "Federation, article 1260) - these are their Russian originals"
)
SET_NAME: Final[str] = "wikipedia-en-averchenko-leskov-ru"
# When the files below were downloaded.
COMMON_VOICE_RETRIEVED: Final[str] = "2026-09-26T18:11+05:00"
WIKISOURCE_RETRIEVED: Final[str] = "2026-09-27T12:12+05:00"


@dataclass(frozen=True)
class Source:
    """One downloaded file: the corpus locale of its text, where and when it came from, its SHA-256, its
    license, how it is read (`LINES`, `WIKISOURCE`), the share of its sentences kept (by a hash of the
    sentence) and the share an earlier set took from the same file (left out, `PREVIOUS_SAMPLE_NAMESPACE`)."""

    locale: str
    name: str
    url: str
    retrieved: str
    sha256: str
    license: str
    reader: str = LINES
    sample_percent: int = PERCENT_SCALE
    taken_percent: int = 0


SOURCES: Final[tuple[Source, ...]] = (
    Source("eng", "wiki.en.rerun-2021-11.txt", f"{COMMON_VOICE_RAW}/en/wiki.en.rerun-2021-11.txt", COMMON_VOICE_RETRIEVED,
           "16647e64ba4124ccfeb7061e88ffb53261a16a15b7c4abcd2122b168b9f29639", COMMON_VOICE_LICENSE,
           sample_percent=ORTHO_INDEPENDENT_ENGLISH_SAMPLE_PERCENT, taken_percent=ORTHO_INDEPENDENT_TAKEN_ENGLISH_PERCENT),
    Source("rus", "averchenko-leskov-wikisource.json",
           "https://ru.wikisource.org/wiki/Категория:Рассказы_Аркадия_Тимофеевича_Аверченко and "
           "Категория:Рассказы_Николая_Семёновича_Лескова (API, revisions in the file)",
           WIKISOURCE_RETRIEVED, "baa56cab3e2335d2054a001047d3d41dc2667642b0bf5a8d21be095095b6f783", WIKISOURCE_LICENSE,
           reader=WIKISOURCE, sample_percent=ORTHO_INDEPENDENT_WIKISOURCE_SAMPLE_PERCENT),
)
# Pages whose text the development read while it was deciding: left out whole.
EXCLUDED_PAGES: Final[tuple[str, ...]] = ()
# Words the development scored or listed by hand besides the authored regression cases
# (`ortho_corpus.AUTHORED_WORDS`): the informative cases of its research scripts and of the ortho tests
# (the second line: the words of the ortho tests that neither the snapshot nor an opened set covers).
LOOKED_AT_WORDS: Final[tuple[str, ...]] = (
    "каеффф", "рсфср", "гыф", "фыва", "гифку", "тыща", "лдпр", "мацуи", "москва", "мда-а", "у-у", "а-б", "лут", "флуд",
    "фнаф", "ctrl", "oled", "gtk", "hfcs",
    "-тв", "тв-", "gtk-ключ", "а-бв", "ааа-ба", "абв", "квазимодо", "мцуис", "ну-ну", "тпште", "ура-а-а", "ф-привет",
    "фпревет", "сборки", "abab", "ahtop", "q-nj", "xyzzy", "zorblax", "ab-bb", "htop-", ".xyz", "dist", "гыр-гыр",
)
# The sample of this set, and the sample the first English set took from the same file (left out).
SAMPLE_NAMESPACE: Final[str] = "keyswitch:ortho-v1:independent-sample-3"
PREVIOUS_SAMPLE_NAMESPACE: Final[str] = "keyswitch:ortho-v1:independent-sample"
CODE: Final[tuple[str, ...]] = (
    "tools/ortho_independent.py", "tools/wikisource_text.py", "tools/ortho_corpus.py", "tools/ortho_known.py",
    "tools/train_ortho_model.py",
    "src/keyswitch/ortho_model.py",
)
Label = tuple[str, str, str]


def checksum(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def sampled(sentence: str, percent: int, namespace: str = SAMPLE_NAMESPACE) -> bool:
    """Whether a sentence is in the share `percent` of its source, by a hash of the sentence in `namespace`."""

    digest = hashlib.sha256(f"{namespace}:{sentence}".encode()).hexdigest()
    return int(digest[:DETERMINISTIC_CHOICE_HEX_DIGITS], HEXADECIMAL_BASE) % PERCENT_SCALE < percent


def sentences(source: Source, directory: Path) -> list[str]:
    path = directory / source.name
    if checksum(path) != source.sha256:
        raise ValueError(f"{source.name} is not the pinned file")
    text = path.read_text(encoding="utf-8")
    if source.reader == WIKISOURCE:
        pages = cast(dict[str, dict[str, object]], json.loads(text))
        lines = [sentence for title in sorted(pages) if title not in EXCLUDED_PAGES
                 for sentence in wikitext_sentences(str(pages[title]["content"]))]
    else:
        lines = text.splitlines()
    return [line for line in lines if sampled(line, source.sample_percent)
            and not sampled(line, source.taken_percent, PREVIOUS_SAMPLE_NAMESPACE)]


def rows_from(directory: Path) -> list[dict[str, object]]:
    """Rows of the sentences, as `ortho_corpus.rows` builds a split: one per (script, keys) with shapes."""

    lists = ortho_corpus.word_lists()
    counts: dict[tuple[str, str], Counter[str]] = {}
    for source in SOURCES:
        for line in sentences(source, directory):
            words = unicodedata.normalize("NFC", line).split()
            for position, raw in enumerate(words):
                if len(raw) > ORTHO_V1_MAXIMUM_TOKEN_CHARACTERS:
                    continue
                script = ortho_corpus.label(raw, source.locale, lists)
                if script is None:
                    continue
                keys = ortho_corpus.to_keys(raw)
                if not keys:
                    continue
                shapes = counts.setdefault((script, keys), Counter())
                shapes[ortho_corpus.shape_of(raw, position)] += 1
    return [{"split": TEST, "script": script, "keys": keys, "shapes": {name: shapes[name] for name in sorted(shapes)}}
            for (script, keys), shapes in sorted(counts.items())]


def forms(rows: Sequence[dict[str, object]]) -> dict[str, set[str]]:
    """Every word the population filter and the features ask a dictionary about (`ortho_known.forms`)."""

    result: dict[str, set[str]] = {script: set() for script in SCRIPTS}
    for row in rows:
        for direction in SCRIPTS:
            for served in ortho_corpus.served(str(row["keys"]), direction):
                if set(served) <= ortho_corpus.KEYS:
                    for script, words in ortho_known.queried(served, direction).items():
                        result[script].update(words)
    return result


def verdict(rows: Sequence[dict[str, object]]) -> dict[str, list[str]]:
    models = ortho_known.reference_models()
    return {script: sorted(word for word in words if models[script].score(word).known)
            for script, words in sorted(forms(rows).items())}


def seen() -> frozenset[str]:
    """Served keys, with every run read once, of every row of the frozen snapshot and of every opened set in
    either layout, and of the cases written by hand."""

    written = {*ortho_corpus.AUTHORED_KEYS, *(ortho_corpus.to_keys(word) for word in LOOKED_AT_WORDS)}
    sequences = [str(row["keys"]) for row in ortho_corpus.load_tokens() if row["split"] != ortho_corpus.LEXICON]
    for directory in opened():
        sequences.extend(str(row["keys"]) for row in ortho_corpus.load_tokens(directory / TOKENS_NAME))
    result: set[str] = set()
    for keys in [*sequences, *sorted(written)]:
        for direction in SCRIPTS:
            result.update(read_once(served, ORTHO_MIN_STRETCH_RUN) for served in ortho_corpus.served(keys, direction))
    return frozenset(result)


def type_counts(labels: dict[Label, bool]) -> dict[str, int]:
    counted: Counter[str] = Counter(f"{direction}:{'negative' if negative else 'positive'}_types"
                                    for (direction, _keys, _shape), negative in labels.items())
    return dict(sorted(counted.items()))


def population_counts(counts: dict[str, int]) -> dict[str, int]:
    """The negative and positive type counts of a report's `corpus_test` counts."""

    return {name: count for name, count in counts.items() if name.endswith(("negative_types", "positive_types"))}


def population(rows: list[dict[str, object]], known: dict[str, frozenset[str]]) -> tuple[dict[Label, bool], Counter[str]]:
    """The independent types and the count of those left out because the development saw their keys."""

    minimum = cast(int, trainer.config()["minimum_length"])
    labels = trainer.population(rows, TEST, known, minimum)
    excluded = seen()
    dropped: Counter[str] = Counter()
    kept: dict[Label, bool] = {}
    for label, negative in labels.items():
        if read_once(label[1], ORTHO_MIN_STRETCH_RUN) in excluded:
            dropped[f"{label[0]}:{'negative' if negative else 'positive'}_types"] += 1
        else:
            kept[label] = negative
    return kept, dropped


def provenance() -> dict[str, str]:
    code = tuple(ROOT / name for name in CODE)
    pins = {path.relative_to(ROOT).as_posix(): checksum(path) for path in code}
    pins[CONSTANTS_PACKAGE] = pin_values(code, source_root=ROOT / "src").sha256
    return pins


def opened() -> list[Path]:
    """The directories of every set the development has seen: the opened tests and the development sets."""

    return [*(directory for directory, _report in HISTORY), *DEVELOPMENT_SETS]


def history_tokens() -> dict[str, str]:
    return {directory.name: checksum(directory / TOKENS_NAME) for directory in opened()}


def source_description() -> dict[str, object]:
    return {"excluded_pages": list(EXCLUDED_PAGES),
            "files": [{"locale": source.locale, "name": source.name, "url": source.url, "retrieved": source.retrieved,
                       "sha256": source.sha256, "license": source.license, "reader": source.reader,
                       "sample_percent": source.sample_percent, "taken_by_an_earlier_set_percent": source.taken_percent}
                      for source in SOURCES],
            "sample_namespace": SAMPLE_NAMESPACE}


def receipt(rows: list[dict[str, object]], tokens_sha: str, evidence_sha: str,
            known: dict[str, frozenset[str]]) -> dict[str, object]:
    kept, dropped = population(rows, known)
    return {
        "schema_version": ORTHO_INDEPENDENT_RECEIPT_SCHEMA_VERSION, "set": SET_NAME, "source": source_description(),
        "population": "the corpus' own rules (ortho_corpus.label, to_keys, shape_of; train_ortho_model.population) "
                      "with this verdict; a type whose served keys, every run read once, match a row of the snapshot, "
                      "of an opened set or of a case written by hand is left out",
        "left_out_words": sorted({*ortho_corpus.AUTHORED_WORDS, *LOOKED_AT_WORDS}),
        "rows_by_script": dict(sorted(Counter(str(row["script"]) for row in rows).items())),
        "types": type_counts(kept),
        "excluded_as_seen": dict(sorted(dropped.items())),
        "snapshot_tokens_sha256": checksum(ortho_corpus.TOKENS),
        "history_tokens_sha256": history_tokens(),
        "tokens_sha256": tokens_sha,
        "evidence_sha256": evidence_sha,
        "provenance": provenance(),
    }


def history_matches(directory: Path, report_path: Path) -> dict[str, object]:
    """An opened set: its receipt, its files and the report of its single test must agree."""

    recorded = cast(dict[str, object], json.loads((directory / RECEIPT_NAME).read_bytes()))
    report = cast(dict[str, object], json.loads(report_path.read_bytes()))
    described = cast(dict[str, object], report["independent_test"])
    counts = cast(dict[str, int], cast(dict[str, object], report["corpus_test"])["counts"])
    if (described.get("receipt_sha256") != checksum(directory / RECEIPT_NAME)
            or recorded.get("tokens_sha256") != checksum(directory / TOKENS_NAME)
            or recorded.get("evidence_sha256") != checksum(directory / EVIDENCE_NAME)
            or described.get("tokens_sha256") != recorded.get("tokens_sha256")
            or described.get("evidence_sha256") != recorded.get("evidence_sha256")
            or described.get("source") != recorded.get("source")
            or population_counts(counts) != recorded.get("types")):
        raise ValueError(f"opened independent set {directory.name} does not match its receipt and report")
    return recorded


def frozen() -> tuple[list[dict[str, object]], dict[str, frozenset[str]], dict[str, object]]:
    """The frozen rows and verdict of the active set, refused unless they, the snapshot, the opened sets
    and the code match the receipt."""

    recorded = cast(dict[str, object], json.loads(RECEIPT.read_bytes()))
    if (recorded.get("schema_version") != ORTHO_INDEPENDENT_RECEIPT_SCHEMA_VERSION or recorded.get("set") != SET_NAME
            or recorded.get("tokens_sha256") != checksum(TOKENS) or recorded.get("evidence_sha256") != checksum(EVIDENCE)
            or recorded.get("snapshot_tokens_sha256") != checksum(ortho_corpus.TOKENS)
            or recorded.get("history_tokens_sha256") != history_tokens()
            or recorded.get("provenance") != provenance()):
        raise ValueError("independent test files, snapshot, opened sets or code do not match the receipt")
    return ortho_corpus.load_tokens(TOKENS), ortho_known.load(EVIDENCE), recorded


def test() -> tuple[dict[str, object], bool]:
    """The single test of the sealed candidate on the independent types."""

    rows, known, recorded = frozen()
    seal = cast(dict[str, object], json.loads(trainer.SEAL.read_bytes()))
    if seal.get("provenance") != trainer.provenance() or seal.get("candidate_sha256") != trainer.checksum(trainer.CANDIDATE):
        raise ValueError("candidate or provenance changed after the seal")
    if trainer.REPORT.exists():
        raise ValueError("a test report exists; a second test would not be independent")
    labels, _dropped = population(rows, known)
    if type_counts(labels) != recorded.get("types"):
        raise ValueError("independent population differs from its receipt")
    identifier = IdentifierLexicon.load(IDENTIFIERS)
    model = OrthoModel.load(trainer.CANDIDATE, identifier=identifier.contains,
                            known=lambda script, word: word in known[script])
    corpus = trainer.outcome(model, labels)
    counts = cast(dict[str, int], corpus["counts"])

    def summed(name: str) -> int:
        return sum(counts.get(f"{script}:{name}", 0) for script in SCRIPTS)

    promotion = cast(dict[str, object], trainer.config()["promotion"])
    negatives, false_total, positives = summed("negative_types"), summed("false_types"), summed("positive_types")
    recall = summed("recalled_types") / max(1, positives)
    passed = (negatives >= cast(int, promotion["minimum_test_negatives"])
              and false_total <= cast(int, promotion["maximum_test_false_conversions"])
              and recall >= float(cast(float, promotion["minimum_recall"])))
    report: dict[str, object] = {
        "schema_version": ORTHO_V1_EVIDENCE_SCHEMA_VERSION, "model_version": seal["model_version"],
        "seal_sha256": trainer.checksum(trainer.SEAL), "candidate_sha256": trainer.checksum(trainer.CANDIDATE),
        "thresholds": seal["thresholds"], "corpus_test": corpus,
        "recall": round(recall, ORTHO_V1_RECALL_DECIMALS), "promotion_passed": passed,
        "evidence_scope": (
            f"synthetic wrong-layout renderings of the independent set {SET_NAME} (text the development never saw), "
            "cut as the engine cuts them and limited to what it serves the model; types whose keys, runs read once, "
            "the snapshot, an opened set or a case written by hand shows are left out; one type is (direction, keys, "
            "case shape)"
        ),
        "independent_test": {"set": SET_NAME, "receipt_sha256": checksum(RECEIPT), "tokens_sha256": recorded["tokens_sha256"],
                             "evidence_sha256": recorded["evidence_sha256"], "source": recorded["source"]},
    }
    trainer.REPORT.write_bytes(trainer.canonical(report))
    return report, passed


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", nargs="?", choices=("test",))
    parser.add_argument("--freeze", type=Path, metavar="DIR")
    arguments = parser.parse_args(argv)
    for directory, report_path in HISTORY:
        history_matches(directory, report_path)
    if arguments.freeze is not None:
        if TOKENS.exists() or EVIDENCE.exists() or RECEIPT.exists():
            raise ValueError("independent test already frozen; do not overwrite it")
        DIRECTORY.mkdir(parents=True, exist_ok=True)
        rows = rows_from(arguments.freeze)
        tokens_sha = ortho_corpus.write_tokens(rows, TOKENS)
        evidence_sha = ortho_known.write(verdict(rows), EVIDENCE)
        content = receipt(rows, tokens_sha, evidence_sha, ortho_known.load(EVIDENCE))
        RECEIPT.write_text(json.dumps(content, ensure_ascii=False, sort_keys=True,
                                      indent=FROZEN_CORPUS_RECEIPT_JSON_INDENT) + "\n", encoding="utf-8")
    if arguments.command == "test":
        report, passed = test()
        print(json.dumps(report, ensure_ascii=False, indent=REPORT_JSON_INDENT)[:ORTHO_V1_PRINTED_REPORT_MAX_CHARACTERS])
        return 0 if passed else 1
    _rows, _known, recorded = frozen()
    print(json.dumps(recorded, ensure_ascii=False, sort_keys=True, indent=FROZEN_CORPUS_RECEIPT_JSON_INDENT))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
