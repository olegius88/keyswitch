#!/usr/bin/env python3
"""Freeze command families from a local, checksum-verified Debian Contents.

No package is installed and no predictor participates in selection. The existing
UD holdout is protected using only its hashed physical-family sidecar.
"""

from __future__ import annotations

import argparse
from collections import Counter, defaultdict
from collections.abc import Iterable, Mapping, Sequence
from dataclasses import asdict, dataclass
import gzip
import hashlib
import json
from pathlib import Path
import re
from typing import cast

from freeze_context_action_corpus import (
    CorpusRow, SPLITS, Union, WORDS, assigned_split, canonical, checksum, digest,
    exposed_families, load_split, typo_variants,
)
from reconcile_context_action_corpus import expanded_aliases, historical_code_forms

ROOT = Path(__file__).resolve().parents[1]
NAMESPACE = "keyswitch:context-action:debian-trixie-20260912-v1"
COMMAND_PATH = re.compile(r"(?:usr/)?s?bin/([a-z]{3,16})\Z")
HASH = re.compile(r"[0-9a-f]{64}\Z")
ACTIVE_SPLITS = ("train", "development", "calibration", "test")
DEPENDENCIES = (
    "tools/context_technical_corpus.py", "tools/freeze_context_action_corpus.py",
    "tools/reconcile_context_action_corpus.py", "tools/context_physical_keys.py",
    "tools/train_context_model.py", "src/keyswitch/short_words.py",
    "src/keyswitch/layouts.py", "tests/test_context_technical_corpus.py",
)


@dataclass(frozen=True)
class Command:
    name: str
    paths: tuple[str, ...]
    owners: tuple[str, ...]


@dataclass(frozen=True)
class Partitioned:
    rows: list[CorpusRow]
    commands: list[Command]
    aliases: dict[str, tuple[str, ...]]
    summary: dict[str, object]


def command_identifier(name: str) -> str:
    return "debian-trixie-main-amd64:command:" + name


def read_commands(contents: Path) -> list[Command]:
    paths: dict[str, set[str]] = defaultdict(set)
    owners: dict[str, set[str]] = defaultdict(set)
    with gzip.open(contents, "rt", encoding="utf-8", errors="strict") as stream:
        for line in stream:
            fields = line.rstrip("\n").rsplit(None, 1)
            if len(fields) != 2:
                continue
            match = COMMAND_PATH.fullmatch(fields[0])
            if match is None:
                continue
            packages = fields[1].split(",")
            if any(re.fullmatch(r"[a-z0-9+.-]+/[a-z0-9][a-z0-9+.-]*", item) is None for item in packages):
                raise ValueError("invalid package ownership in selected Contents row")
            name = match[1]
            paths[name].add(fields[0])
            owners[name].update(packages)
    return [Command(name, tuple(sorted(paths[name])), tuple(sorted(owners[name]))) for name in sorted(paths)]


def command_aliases(command: Command) -> set[str]:
    aliases: set[str] = set()
    for form in (command.name, *typo_variants(command.name, command_identifier(command.name))):
        aliases.update(expanded_aliases(form))
    return aliases


def partition_commands(commands: Sequence[Command], reserved_aliases: set[str]) -> Partitioned:
    records = sorted(commands, key=lambda item: item.name)
    if len({item.name for item in records}) != len(records) or any(not item.owners for item in records):
        raise ValueError("commands must have unique names and nonempty package owners")
    family_union = Union()
    aliases = {item.name: tuple(sorted(command_aliases(item))) for item in records}
    for item in records:
        anchor = "command:" + item.name
        family_union.find(anchor)
        for alias in aliases[item.name]:
            family_union.join(anchor, "alias:" + alias)
    family_roots = {item.name: family_union.find("command:" + item.name) for item in records}
    families = {name: digest(root) for name, root in family_roots.items()}
    blocked = {families[item.name] for item in records if reserved_aliases.intersection(aliases[item.name])}

    # Package edges join complete alias families before any exclusion or cap.
    # Different sections of the same binary package share the package edge.
    package_union = Union()
    for item in records:
        anchor = "family:" + families[item.name]
        for owner in item.owners:
            package_union.join(anchor, "package:" + owner.rsplit("/", 1)[-1])
    documents = {item.name: "debian-package-component:" + digest(package_union.find("family:" + families[item.name]))
                 for item in records}
    members: dict[str, list[str]] = defaultdict(list)
    for item in records:
        members[families[item.name]].append(item.name)
    retained = {name for names in members.values()
                for name in sorted(names, key=lambda value: digest(NAMESPACE + ":family-cap:" + command_identifier(value)))[:2]}
    rows = []
    for item in records:
        family = families[item.name]
        reasons = []
        if family in blocked:
            reasons.append("reserved-physical-alias")
        if item.name not in retained:
            reasons.append("fixed-hash-family-cap")
        split = "quarantine" if reasons else assigned_split(NAMESPACE, documents[item.name])
        rows.append(CorpusRow(
            identifier=command_identifier(item.name), original=item.name, group=0,
            before="", after="", lemma=item.name, family=family, document=documents[item.name],
            language="en", source="Debian-trixie-main-amd64", source_file="Contents-amd64.gz",
            source_sentence=command_identifier(item.name), source_token="1", spacing=" ",
            space_before="", literal_tail="", upos="X", features="_", misc="_",
            alignment="declared-command-name", layout_representable=True,
            split=split, quarantine_reasons=tuple(reasons),
        ))
    by_name = {item.name: item for item in records}
    split_aliases: dict[str, set[str]] = defaultdict(set)
    split_packages: dict[str, set[str]] = defaultdict(set)
    for row in rows:
        if row.split in ACTIVE_SPLITS:
            split_aliases[row.split].update(aliases[row.original])
            split_packages[row.split].update(owner.rsplit("/", 1)[-1] for owner in by_name[row.original].owners)
    alias_overlap = package_overlap = 0
    for index, left in enumerate(ACTIVE_SPLITS):
        for right in ACTIVE_SPLITS[index + 1:]:
            alias_overlap += len(split_aliases[left] & split_aliases[right])
            package_overlap += len(split_packages[left] & split_packages[right])
    reserved_overlap = sum(len(values & reserved_aliases) for values in split_aliases.values())
    if alias_overlap or package_overlap or reserved_overlap:
        raise ValueError("technical corpus partition leakage")
    summary: dict[str, object] = {
        "mode": "joint-package-and-physical-family-component-hash",
        "split_probabilities": {"train": 0.7, "development": 0.1, "calibration": 0.1, "test": 0.1},
        "commands": len(records), "families": len(members),
        "package_components": len(set(documents.values())),
        "largest_component_commands": max(Counter(documents.values()).values(), default=0),
        "reserved_families": len(blocked), "family_cap": 2,
        "rows_by_split": {split: sum(row.split == split for row in rows) for split in SPLITS},
        "documents_by_split": {split: len({row.document for row in rows if row.split == split}) for split in SPLITS},
        "families_by_split": {split: len({row.family for row in rows if row.split == split}) for split in SPLITS},
        "quarantine_reasons": dict(Counter(reason for row in rows for reason in row.quarantine_reasons)),
        "alias_split_overlap": alias_overlap, "package_split_overlap": package_overlap,
        "reserved_alias_overlap": reserved_overlap,
    }
    return Partitioned(rows, records, aliases, summary)


def read_object(path: Path) -> dict[str, object]:
    value: object = json.loads(path.read_bytes())
    if not isinstance(value, dict) or any(not isinstance(key, str) for key in value):
        raise ValueError("expected JSON object")
    return cast(dict[str, object], value)


def reserved_ud_aliases(directory: Path) -> tuple[set[str], dict[str, str]]:
    manifest_path = directory / "manifest.json"
    manifest = read_object(manifest_path)
    reconciliation = manifest.get("reconciliation")
    path = directory / "physical-family-closure.json"
    if not isinstance(reconciliation, dict) or checksum(path) != reconciliation.get("closure_sha256"):
        raise ValueError("UD physical sidecar checksum mismatch")
    aliases = read_object(path).get("alias_sha256_to_closure_sha256")
    if not isinstance(aliases, dict) or any(not isinstance(key, str) or HASH.fullmatch(key) is None
                                            or not isinstance(value, str) or HASH.fullmatch(value) is None
                                            for key, value in aliases.items()):
        raise ValueError("invalid UD physical alias sidecar")
    return set(aliases), {str(manifest_path): checksum(manifest_path), str(path): checksum(path)}


def all_internal_edits(forms: Iterable[str]) -> set[str]:
    result = set(forms)
    tokens = {word for form in result for word in WORDS.findall(form)}
    result.update(tokens)
    for word in tokens:
        if not 4 <= len(word) <= 64:
            continue
        for index in range(1, len(word) - 1):
            result.add(word[:index] + word[index + 1:])
            result.add(word[:index] + word[index] + word[index:])
        for index in range(1, len(word) - 2):
            result.add(word[:index] + word[index + 1] + word[index] + word[index + 2:])
    return result


def reservations(repository: Path, ud_directory: Path, additional: Sequence[Path],
                 candidates: set[str]) -> tuple[set[str], dict[str, str], dict[str, object]]:
    ud_aliases, provenance = reserved_ud_aliases(ud_directory)
    exposed, sources = exposed_families(repository, additional)
    provenance.update(sources)
    code = repository / "tools/train_context_model.py"
    short = repository / "src/keyswitch/short_words.py"
    historical = all_internal_edits(historical_code_forms(code.read_text(encoding="utf-8")))
    provenance.update({str(code): checksum(code), str(short): checksum(short)})
    reserved = candidates & ud_aliases
    historical_matches: set[str] = set()
    for form in sorted(exposed | historical):
        historical_matches.update(candidates & expanded_aliases(form))
    reserved.update(historical_matches)
    return reserved, provenance, {
        "ud_aliases_reserved": len(ud_aliases), "candidate_ud_alias_matches": len(candidates & ud_aliases),
        "historical_forms": len(exposed | historical), "candidate_historical_alias_matches": len(historical_matches),
        "ud_scope": "all sidecar aliases including quarantine; no UD split text opened",
        "historical_scope": "existing exposed_families plus declared historical context literals and short words; all their internal edits",
    }


def verified_source(directory: Path) -> tuple[Path, dict[str, str], dict[str, object]]:
    receipt_path = directory / "source-receipt.json"
    receipt = read_object(receipt_path)
    contents, release = directory / "Contents-amd64.gz", directory / "InRelease"
    data = receipt.get("contents")
    release_data = receipt.get("release")
    if (not isinstance(data, dict) or not isinstance(release_data, dict)
            or data.get("status") != 200 or release_data.get("status") != 200
            or receipt.get("tls_certificate_verification") is not True
            or data.get("url") != "https://deb.debian.org/debian/dists/trixie/main/Contents-amd64.gz"
            or release_data.get("url") != "https://deb.debian.org/debian/dists/trixie/InRelease"):
        raise ValueError("source receipt does not identify verified TLS Debian downloads")
    if (checksum(contents) != receipt.get("contents_sha256")
            or contents.stat().st_size != receipt.get("contents_bytes")
            or checksum(release) != receipt.get("release_sha256")):
        raise ValueError("Debian source checksum mismatch")
    expected = (str(receipt["contents_sha256"]), str(receipt["contents_bytes"]), "main/Contents-amd64.gz")
    sha256_section = False
    found = False
    for line in release.read_text(encoding="utf-8").splitlines():
        if line == "SHA256:":
            sha256_section = True
        elif sha256_section and line and not line.startswith(" "):
            sha256_section = False
        elif sha256_section and tuple(line.split()) == expected:
            found = True
    if not found:
        raise ValueError("Contents checksum absent from InRelease SHA256 section")
    return contents, {str(path): checksum(path) for path in (receipt_path, contents, release)}, {
        "source": "Debian trixie main amd64 Contents", "receipt": receipt,
        "verification": "HTTPS TLS and Contents SHA256 against downloaded InRelease; OpenPGP signature not verified by this generator",
    }


def freeze_rows(partitioned: Partitioned, output: Path, provenance: Mapping[str, str],
                metadata: Mapping[str, object]) -> dict[str, object]:
    if output.exists():
        raise ValueError("refusing to overwrite frozen technical corpus")
    generator = Path(__file__).read_bytes()
    output.mkdir(parents=True)
    (output / "generator-source.py").write_bytes(generator)
    files = {}
    for split in SPLITS:
        path = output / (split + ".jsonl.gz")
        raw_hash = hashlib.sha256()
        count = 0
        with path.open("wb") as stream, gzip.GzipFile(fileobj=stream, mode="wb", filename="", mtime=0) as compressed:
            for row in partitioned.rows:
                if row.split == split:
                    raw = canonical(asdict(row))
                    raw_hash.update(raw)
                    compressed.write(raw)
                    count += 1
        files[split] = {"path": path.name, "sha256": checksum(path), "content_sha256": raw_hash.hexdigest(), "rows": count}
    held = [row for row in partitioned.rows if row.split == "test"]
    membership = {"namespace": NAMESPACE, "scope": "declared command families and deduplicated binary-package components",
                  "row_ids_sha256": sorted(digest(row.identifier) for row in held),
                  "family_ids_sha256": sorted({row.family for row in held}),
                  "document_ids_sha256": sorted({digest(row.document) for row in held})}
    membership_path = output / "test-membership.json"
    membership_path.write_bytes(canonical(membership))
    command_path = output / "command-provenance.json"
    command_path.write_bytes(canonical({"commands": [asdict(item) for item in partitioned.commands],
                                       "aliases_sha256_by_command": partitioned.aliases}))
    by_family: dict[str, list[CorpusRow]] = defaultdict(list)
    for row in partitioned.rows:
        by_family[row.family].append(row)
    family_records = {}
    for family, rows in sorted(by_family.items()):
        active = [row for row in rows if row.split in ACTIVE_SPLITS]
        if not active:
            continue
        splits = {row.split for row in active}
        if len(splits) != 1:
            raise ValueError("family spans multiple active splits")
        family_records[family] = {
            "split": active[0].split,
            "aliases_sha256": sorted({alias for row in rows for alias in partitioned.aliases[row.original]}),
            "document_ids_sha256": sorted({digest(row.document) for row in active}),
        }
    family_path = output / "family-aliases.json"
    family_path.write_bytes(canonical({"schema_version": 1, "families": family_records}))
    manifest: dict[str, object] = {
        "schema_version": 1, "namespace": NAMESPACE, "label": "keep", "splits": files,
        "scope": "declared command-family/package holdout; not globally unseen lexicon or verified human intent",
        "normalization": "exact case-sensitive (usr/)?s?bin/[a-z]{3,16}; command basename deduplicated; all paths and owners retained",
        "context": "empty before/after; downstream action_rows adds its existing disclosed mixed contexts and declared layout interventions",
        "token_language": "ASCII command alphabet uses group 0; not an assertion of natural English prose",
        "partitioning": partitioned.summary, "source_metadata": dict(metadata),
        "provenance": dict(sorted(provenance.items())),
        "generator_sha256": hashlib.sha256(generator).hexdigest(), "generator_source": "generator-source.py",
        "command_provenance_sha256": checksum(command_path), "test_membership_sha256": checksum(membership_path),
        "family_aliases_sha256": checksum(family_path),
        "typo_policy": "all exported typo_variants(original,identifier) joined using expanded_aliases before package grouping and splitting",
        "test_policy": "no scoring or test-text output; exact split loader; test requires explicit allow_test; quarantine never used for training",
        "compression": "gzip mtime=0; content SHA256 independent of deflate implementation",
        "limitations": ["Contents lists shipped paths, not executable permissions or human command intent",
                        "binary-package ownership is grouped; shared source packages and renamed forks are not identified",
                        "only lowercase ASCII command names of 3 through 16 characters are in this supplement",
                        "existing dictionaries and pretrained model vocabularies are not globally inventoried",
                        "an intended command typo is labeled by the existing action_rows spelling policy, not human-reviewed commands"],
    }
    if Path(__file__).read_bytes() != generator:
        raise ValueError("technical generator changed during freeze")
    (output / "manifest.json").write_bytes(canonical(manifest))
    return manifest


def load_technical_split(directory: Path, split: str, *, allow_test: bool = False) -> list[CorpusRow]:
    if split == "test" and not allow_test:
        raise ValueError("technical test access requires explicit allow_test")
    if split not in ACTIVE_SPLITS:
        raise ValueError("technical corpus requires a named training split or explicit test")
    return load_split(directory, split)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-directory", type=Path, required=True)
    parser.add_argument("--ud-corpus", type=Path, required=True)
    parser.add_argument("--extra-exposure", type=Path, action="append", required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--preview-report", type=Path, required=True)
    args = parser.parse_args(argv)
    dependency_pins = {relative: checksum(ROOT / relative) for relative in DEPENDENCIES}
    contents, provenance, source_metadata = verified_source(args.source_directory)
    records = read_commands(contents)
    candidates = {alias for item in records for alias in command_aliases(item)}
    reserved, inputs, reservation_summary = reservations(ROOT, args.ud_corpus, args.extra_exposure, candidates)
    provenance.update(inputs)
    provenance.update(dependency_pins)
    result = partition_commands(records, reserved)
    metadata = {"source": source_metadata, "reservations": reservation_summary}
    report: dict[str, object] = {"namespace": NAMESPACE, "partitioning": result.summary,
                                "reservations": reservation_summary, "provenance": provenance,
                                "test_scored": False, "old_ud_split_text_read": False}
    for name, expected in provenance.items():
        path = Path(name)
        if not path.is_absolute():
            path = ROOT / path
        if checksum(path) != expected:
            raise ValueError("corpus input changed while building technical partitions")
    if args.output is not None:
        manifest = freeze_rows(result, args.output, provenance, metadata)
        report.update({"manifest_sha256": checksum(args.output / "manifest.json"),
                       "test_membership_sha256": manifest["test_membership_sha256"]})
    args.preview_report.write_bytes(canonical(report))
    print(json.dumps({"partitioning": result.summary, "test_scored": False}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
