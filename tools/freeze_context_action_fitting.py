"""Extend a frozen corpus's fitting splits with short utterances, leaving its test alone.

The holdout freezer adds rows to `test`; this one adds them to `train`, `development` and
`calibration`. The reason is measured: the context-action model is fitted on treebank text,
where a token almost always has a neighbour, and it hedges on the first word of a message -
the shape most chat input has (.t/reliable-release-2026-09-12/SHORT-ISOLATED-CURRICULUM.md).

Two invariants make the extension safe to fit on:

* a row or document that any sealed test ever held - the base corpus's own test split and every
  ledger membership - never enters a fitting split, so the receipts' claim that training never
  saw test material still holds. The check is by row and document as well as by family, because
  a family identifier is recomputed from whatever rows a freeze happens to see: the same sentence
  selected under two namespaces can be grouped differently and carry two different identifiers.
  Corpus v10 was frozen with the family check alone and let five rows of the consumed TEST v9
  into its training split;
* a family already present in a fitting split keeps that split, so no family is fitted in one
  split and measured in another. Unlike the holdout freezer, a family already in the base
  corpus is therefore *kept*: `You` or `кот` are exactly the short words this extension is for,
  and they are already in the base by being ordinary words.

The base corpus's `test` bytes are copied verbatim; rows the exclusions reject land in
quarantine. Nothing is overwritten: the output directory must not exist.
"""
from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import shutil
import sys
from collections import Counter, defaultdict
from collections.abc import Mapping, Sequence
from dataclasses import replace
from pathlib import Path
from typing import cast

sys.path.insert(0, str(Path(__file__).resolve().parent))

from context_evidence import checksum
from freeze_context_action_corpus import (
    WORDS, CorpusRow, Sentence, Union, canonical, digest, family_aliases, load_split, physical,
    row_identifier, sentence_rows, typo_variants,
)
from freeze_context_action_holdout import (
    Exclusions, alias_reasons, gzip_member, ledger_test_families, ledger_test_rows, load_exclusions,
    read_object, read_tatoeba, select_holdout_sentences, sentence_documents, verified_tatoeba_source,
)
from reconcile_context_action_corpus import expanded_aliases
from keyswitch.constants.model_protocol import FITTING_SPLITS
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))
from keyswitch.constants.corpus import (
    DEFAULT_MAX_SENTENCES_PER_DOCUMENT,
    DETERMINISTIC_DRAW_HEX_DIGITS,
    FITTING_DEFAULT_MAX_DOCUMENTS_PER_SOURCE,
    FITTING_SHARES,
    SPLIT_BUCKET_COUNT,
)
from keyswitch.constants.file_formats import HEXADECIMAL_BASE

ROOT = Path(__file__).resolve().parents[1]


def fitting_split(namespace: str, family: str) -> str:
    """Which fitting split a new family belongs to, deterministically."""
    bucket = int(digest(namespace + ":fitting:" + family)[:DETERMINISTIC_DRAW_HEX_DIGITS], HEXADECIMAL_BASE) % SPLIT_BUCKET_COUNT
    for limit, split in FITTING_SHARES:
        if bucket < limit:
            return split
    raise ValueError("unreachable share table")


def base_family_splits(base: Path) -> tuple[dict[str, str], set[str], set[str], set[str]]:
    """The base's families by split, and the families, rows and documents its test holds."""
    families: dict[str, str] = {}
    test_families: set[str] = set()
    test_rows: set[str] = set()
    test_documents: set[str] = set()
    for split in (*FITTING_SPLITS, "test"):
        for row in load_split(base, split):
            if split == "test":
                test_families.add(row.family)
                test_rows.add(digest(row.identifier))
                test_documents.add(digest(row.document))
            else:
                families.setdefault(row.family, split)
    return families, test_families, test_rows, test_documents


def fitting_rows(sentences: Sequence[Sentence], exclusions: Exclusions, namespace: str,
                 base_families: Mapping[str, str], test_families: set[str], ledger_families: set[str],
                 test_rows: set[str], test_documents: set[str]) -> tuple[list[CorpusRow], dict[str, object]]:
    """New rows in their fitting split, or in quarantine with the reason they were refused."""
    families = Union()
    for sentence in sentences:
        for token in sentence.tokens:
            aliases = family_aliases(token, row_identifier(sentence, token))
            for alias in aliases:
                families.join(aliases[0], alias)
    rows = [row for sentence in sentences for row in sentence_rows(sentence, families)]
    if len({row.identifier for row in rows}) != len(rows):
        raise ValueError("duplicate fitting row identifiers")
    physical_by_family: dict[str, set[str]] = defaultdict(set)
    hashes_by_family: dict[str, set[str]] = defaultdict(set)
    for sentence in sentences:
        for token in sentence.tokens:
            identifier = row_identifier(sentence, token)
            family = digest(families.find(physical(token.form)))
            values = (token.form, *token.lemmas, *typo_variants(token.form, identifier))
            physical_by_family[family].update(physical(value) for value in values)
            physical_by_family[family].update(physical(word) for value in values for word in WORDS.findall(value))
            for value in values:
                hashes_by_family[family].update(expanded_aliases(value))
    reasons_by_family: dict[str, list[str]] = {}
    for family in hashes_by_family:
        # Being in the base corpus is what this extension is for; being in anyone's test is not.
        reasons = [reason for reason in alias_reasons(hashes_by_family[family], physical_by_family[family], family, exclusions)
                   if reason != "family-in-base-corpus"]
        if family in test_families or family in ledger_families:
            if "prior-accessed-test-family" not in reasons:
                reasons.append("prior-accessed-test-family")
        reasons_by_family[family] = reasons
    result: list[CorpusRow] = []
    reasons_count: Counter[str] = Counter()
    split_counts: Counter[str] = Counter()
    for row in rows:
        reasons = list(reasons_by_family.get(row.family, []))
        if row.alignment != "exact-text-comment":
            reasons.append(row.alignment)
        # A family identifier is recomputed per freeze, so it cannot answer "has a sealed test
        # seen this row". The row and document digests can, and they are what the ledger stores.
        if digest(row.identifier) in test_rows:
            reasons.append("prior-accessed-test-row")
        if digest(row.document) in test_documents:
            reasons.append("prior-accessed-test-document")
        if reasons:
            split = "quarantine"
        else:
            split = base_families.get(row.family) or fitting_split(namespace, row.family)
        reasons_count.update(reasons)
        split_counts[split] += 1
        result.append(replace(row, split=split, quarantine_reasons=tuple(reasons)))
    result.sort(key=lambda row: row.identifier)
    kept = [row for row in result if row.split != "quarantine"]
    return result, {
        "rows": len(result), "families": len(hashes_by_family),
        "added_rows": len(kept), "added_families": len({row.family for row in kept}),
        "added_by_split": dict(sorted(split_counts.items())),
        "followed_base_split": sum(1 for row in kept if row.family in base_families),
        "quarantine_reasons": dict(reasons_count),
    }


def write_extension(base: Path, output: Path, new_rows: Sequence[CorpusRow], namespace: str,
                    provenance: Mapping[str, str], metadata: Mapping[str, object]) -> dict[str, object]:
    """Copy the base corpus, append the new rows to their splits, keep test byte-identical."""
    if output.exists():
        raise ValueError("refusing to overwrite a corpus directory")
    resolved = output.resolve()
    if resolved == base.resolve() or resolved.is_relative_to(base.resolve()):
        raise ValueError("derivative must not be inside the base corpus")
    generator = Path(__file__).read_bytes()
    base_manifest = read_object(base / "manifest.json")
    base_splits = cast(dict[str, dict[str, object]], base_manifest["splits"])
    output.mkdir(parents=True)
    (output / "generator-source.py").write_bytes(generator)
    (output / "origins").mkdir()
    shutil.copyfile(base / "manifest.json", output / "origins" / "base-manifest.json")
    shutil.copyfile(base / "test-membership.json", output / "origins" / "base-test-membership.json")
    files: dict[str, dict[str, object]] = {}
    for split in ("test",):
        source = base / str(base_splits[split]["path"])
        if checksum(source) != base_splits[split]["sha256"]:
            raise ValueError("base split checksum mismatch: " + split)
        shutil.copyfile(source, output / (split + ".jsonl.gz"))
        files[split] = dict(base_splits[split])
    for split in (*FITTING_SPLITS, "quarantine"):
        source = base / str(base_splits[split]["path"])
        if checksum(source) != base_splits[split]["sha256"]:
            raise ValueError("base split checksum mismatch: " + split)
        path = output / (split + ".jsonl.gz")
        compressed, _raw, added = gzip_member([row for row in new_rows if row.split == split])
        with path.open("wb") as destination:
            destination.write(source.read_bytes())
            destination.write(compressed)
        raw_digest = hashlib.sha256()
        count = 0
        with gzip.open(path, "rb") as stream:
            for line in stream:
                raw_digest.update(line)
                count += 1
        expected = int(cast(int, base_splits[split]["rows"])) + added
        if count != expected:
            raise ValueError(f"assembled split row count mismatch: {split} ({count} against {expected})")
        files[split] = {"path": path.name, "sha256": checksum(path),
                        "content_sha256": raw_digest.hexdigest(), "rows": count}
    shutil.copyfile(base / "test-membership.json", output / "test-membership.json")
    manifest = {
        "schema_version": base_manifest.get("schema_version"), "namespace": namespace,
        "kind": "fitting-extension", "base_namespace": base_manifest.get("namespace"),
        "base_manifest_sha256": checksum(base / "manifest.json"),
        "test_membership_sha256": checksum(output / "test-membership.json"),
        "splits": files, "provenance": dict(sorted(provenance.items())), "metadata": metadata,
        "generator_sha256": hashlib.sha256(generator).hexdigest(),
    }
    (output / "manifest.json").write_bytes(canonical(manifest))
    return manifest


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--namespace", required=True)
    parser.add_argument("--tatoeba-directory", type=Path, required=True)
    parser.add_argument("--tatoeba-language", action="append", required=True)
    parser.add_argument("--ud-origin", type=Path, required=True)
    parser.add_argument("--technical-origin", type=Path, required=True)
    parser.add_argument("--prefix-inventory", type=Path, required=True)
    parser.add_argument("--lexicon", type=Path, default=ROOT / "src/keyswitch/resources/identifiers.json")
    parser.add_argument("--ledger", type=Path, default=ROOT / ".t/reliable-release-2026-09-12/context-action-test-ledger")
    parser.add_argument("--extra-exposure", action="append", type=Path, default=[])
    parser.add_argument("--max-documents", type=int, default=FITTING_DEFAULT_MAX_DOCUMENTS_PER_SOURCE)
    parser.add_argument("--max-sentences-per-document", type=int, default=DEFAULT_MAX_SENTENCES_PER_DOCUMENT)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args(argv)

    exclusions = load_exclusions(ROOT, args.base, args.ud_origin, args.technical_origin,
                                 args.prefix_inventory, args.lexicon, args.extra_exposure, ledger=args.ledger)
    exports, tatoeba_provenance, tatoeba_metadata = verified_tatoeba_source(args.tatoeba_directory, args.tatoeba_language)
    sentences: list[Sentence] = []
    for label, language, path in exports:
        sentences.extend(read_tatoeba(path, label, language, args.namespace))
    sentences = sentence_documents(sentences)
    selected, sampling = select_holdout_sentences(sentences, args.namespace, args.max_documents,
                                                  args.max_sentences_per_document)
    base_families, test_families, base_test_rows, base_test_documents = base_family_splits(args.base)
    ledger_families = ledger_test_families(args.ledger)[0]
    ledger_rows, ledger_documents = ledger_test_rows(args.ledger)
    rows, summary = fitting_rows(selected, exclusions, args.namespace, base_families, test_families,
                                 ledger_families, base_test_rows | ledger_rows,
                                 base_test_documents | ledger_documents)
    provenance = {**exclusions.provenance, **tatoeba_provenance}
    metadata = {"sampling": sampling, "fitting": summary, "source": tatoeba_metadata,
                "base_families": len(base_families), "base_test_families": len(test_families),
                "ledger_families": len(ledger_families),
                "refused_test_rows": len(base_test_rows | ledger_rows),
                "refused_test_documents": len(base_test_documents | ledger_documents)}
    manifest = write_extension(args.base, args.output, rows, args.namespace, provenance, metadata)
    report = {"manifest_sha256": checksum(args.output / "manifest.json"), "splits": manifest["splits"],
              **metadata}
    if args.report:
        args.report.write_text(json.dumps(report, ensure_ascii=False, indent=1, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"written": True, "manifest_sha256": report["manifest_sha256"],
                      "added_by_split": summary["added_by_split"], "added_rows": summary["added_rows"],
                      "quarantine_reasons": summary["quarantine_reasons"]}, ensure_ascii=False, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
