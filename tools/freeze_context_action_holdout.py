#!/usr/bin/env python3
"""Freeze a test-only holdout extension of a sealed context-action corpus.

New Universal Dependencies treebanks and the executables of a newer distribution
index become the TEST split of a derivative corpus whose train, development and
calibration splits are the byte-identical splits of the base corpus. A family is
admitted to the new TEST only when none of its physical aliases is already known:
not in the base corpus (all splits, through its hashed alias sidecars), not in the
previously accessed TEST (family ids and closures), not in the prefix curriculum
exposure inventory, not in the repository's declared exposure sources, and, for
commands, not in the shipped identifier lexicon. Everything else is quarantined
with its reasons. The previous TEST rows move to quarantine as well: they were
read once and can never be evaluated again. No model, dictionary score or text of
any TEST row participates in selection, and no TEST text is printed.
"""
from __future__ import annotations

import argparse
import gzip
import hashlib
import heapq
import json
import re
import shutil
from collections import Counter, defaultdict
from collections.abc import Iterable, Mapping, Sequence
from dataclasses import asdict, dataclass, replace
from pathlib import Path
from typing import cast

from context_technical_corpus import COMMAND_PATH, Command, command_aliases, read_commands
from freeze_context_action_corpus import (
    SPLITS, CorpusRow, Sentence, Union, WORDS, canonical, checksum, digest, exposed_families, family_aliases,
    physical, read_conllu, row_identifier, sentence_rows, typo_variants,
)
from reconcile_context_action_corpus import expanded_aliases, historical_code_forms

ROOT = Path(__file__).resolve().parents[1]
HASH = re.compile(r"[0-9a-f]{64}\Z")
PINS = {
    "UD_Russian-GSD": "9acc9d677327043bd416fcc89e4b3407c620d885",
    "UD_English-GUM": "b58e74bc22d17220c9198864c253a50a897bf27f",
    # corpus v6 (16.09.2026): parallel PUD pair and LinES, release r2.18 tags
    "UD_Russian-PUD": "1228be1571bca125ac19c922ef52ae71d3cdbb83",
    "UD_English-PUD": "e173a1be1b442faf34e7d5a502189ad5d9d1e197",
    "UD_English-LinES": "07a998d3fe0fa4e2bc6aaf651692615f89458a1a",
    # corpus v7 (16.09.2026)
    "UD_Russian-SynTagRus": "6377522610550b696fcc70d39074d2ce03da0e7b",
    "UD_English-ParTUT": "9cb91499ada4e284dfad3ea69b3b1dd10d2c516a",
    "UD_English-GENTLE": "93a5069df8ded256c3e038938b9ea17baf75c73c",
}
SID_CONTENTS_URL = "https://deb.debian.org/debian/dists/sid/main/Contents-amd64.gz"
ARCH_MIRROR = "https://geo.mirror.pkgbuild.com/"
ARCH_REPOSITORIES = ("core", "extra")
ARCH_SOURCE = "Arch-x86_64"
FEDORA_BASE = "https://dl.fedoraproject.org/pub/fedora/linux/releases/43/Everything/x86_64/os/"
FEDORA_SOURCE = "Fedora-x86_64"
SID_RELEASE_URL = "https://deb.debian.org/debian/dists/sid/InRelease"
SID_SOURCE = "Debian-sid-main-amd64"
ACTIVE_SPLITS = ("train", "development", "calibration", "test")
PRIOR_TEST = "prior-accessed-test"


def read_object(path: Path) -> dict[str, object]:
    value = json.loads(path.read_bytes())
    if not isinstance(value, dict):
        raise ValueError("expected a JSON object: " + path.name)
    return cast(dict[str, object], value)


def hash_set(values: object, label: str) -> set[str]:
    if not isinstance(values, (list, dict)) or any(not isinstance(v, str) or HASH.fullmatch(v) is None for v in values):
        raise ValueError("invalid hash list: " + label)
    return set(cast(Iterable[str], values))


def holdout_inventory(source_root: Path, pins: Mapping[str, str] = PINS) -> tuple[list[tuple[str, Path]], list[dict[str, object]]]:
    """Pinned treebank files of the new sources, verified by SHA-256 and git blob hash."""
    paths = []
    metadata = []
    for source, commit in pins.items():
        directory = source_root / source
        pin = read_object(directory / "pin.json")
        if pin.get("commit") != commit or pin.get("repository") != "https://github.com/UniversalDependencies/" + source:
            raise ValueError("holdout source repository or pinned commit differs: " + source)
        entries = pin.get("files")
        if not isinstance(entries, list) or not entries:
            raise ValueError("invalid pinned holdout inventory: " + source)
        for entry in entries:
            item = cast(dict[str, object], entry) if isinstance(entry, dict) else {}
            name = item.get("path")
            if not isinstance(name, str) or not name.endswith((".conllu", ".md", ".txt")):
                raise ValueError("invalid pinned holdout filename: " + source)
            path = directory / Path(name).name
            content = path.read_bytes()
            if hashlib.sha256(content).hexdigest() != item.get("sha256"):
                raise ValueError(f"pinned holdout checksum differs: {source}/{name}")
            if hashlib.sha1(b"blob %d\0" % len(content) + content).hexdigest() != item.get("sha"):
                raise ValueError("holdout content disagrees with pinned Git tree blob: " + name)
            if name.endswith(".conllu"):
                paths.append((source, path))
        metadata.append({**pin, "pin_metadata_sha256": checksum(directory / "pin.json")})
    return sorted(paths, key=lambda item: (item[0], item[1].name)), metadata


def sentence_documents(sentences: Iterable[Sentence]) -> list[Sentence]:
    """A treebank without document markers is sampled by sentence: each sentence is its own document."""
    result = []
    for sentence in sentences:
        if sentence.document == sentence.source + ":unmarked-file:" + sentence.filename:
            sentence = replace(sentence, document=sentence.source + ":sentence:" + sentence.filename + ":" + sentence.identifier)
        result.append(sentence)
    return result


def select_holdout_sentences(sentences: Sequence[Sentence], namespace: str, max_documents: int, max_sentences: int) -> tuple[list[Sentence], dict[str, object]]:
    if max_documents < 1 or max_sentences < 1:
        raise ValueError("document and sentence bounds must be positive")
    heaps: dict[str, dict[str, list[tuple[int, str]]]] = defaultdict(dict)
    counts: Counter[str] = Counter()
    by_key = {}
    for sentence in sentences:
        key = (sentence.source, sentence.filename, sentence.identifier)
        if key in by_key:
            raise ValueError("duplicate holdout sentence identity")
        by_key[key] = sentence
        counts[sentence.source] += 1
        heap = heaps[sentence.source].setdefault(sentence.document, [])
        rank = int(digest(namespace + ":sample:" + sentence.document + ":" + sentence.identifier), 16)
        item = (-rank, sentence.filename + "\0" + sentence.identifier)
        if len(heap) < max_sentences:
            heapq.heappush(heap, item)
        elif item > heap[0]:
            heapq.heapreplace(heap, item)
    selected: list[Sentence] = []
    selected_documents: Counter[str] = Counter()
    for source, documents in heaps.items():
        ranked = sorted(documents, key=lambda document: digest(namespace + ":document-sample:" + document))[:max_documents]
        selected_documents[source] = len(ranked)
        for document in ranked:
            for _rank, packed in documents[document]:
                filename, identifier = packed.split("\0", 1)
                selected.append(by_key[source, filename, identifier])
    selected.sort(key=lambda sentence: (sentence.source, sentence.filename, sentence.identifier))
    return selected, {"available_documents": {source: len(documents) for source, documents in heaps.items()},
                      "available_sentences": dict(counts), "selected_documents": dict(selected_documents),
                      "selected_sentences": len(selected), "max_documents_per_source": max_documents,
                      "max_sentences_per_document": max_sentences, "policy": "every selected document is a TEST candidate; no train/development/calibration assignment"}


@dataclass(frozen=True)
class Exclusions:
    base_aliases: frozenset[str]
    prior_test_aliases: frozenset[str]
    prefix_aliases: frozenset[str]
    prior_test_families: frozenset[str]
    exposed_physical: frozenset[str]
    historical_aliases: frozenset[str]
    lexicon: frozenset[str]
    provenance: dict[str, str]


def ledger_test_families(ledger: Path) -> tuple[set[str], dict[str, str]]:
    """Families of every sealed test that was ever accessed, not only the base corpus's own.

    The base corpus carries the membership of the test it was built with; a corpus chain
    (v5 on v4, v6 on v5, v7 on v6) loses the older ones, and the evaluator's ledger rule
    refuses a new test that shares a family with any of them. Reading the ledger here keeps
    the freeze and that rule in agreement instead of discovering the clash at the end.
    """

    families: set[str] = set()
    provenance: dict[str, str] = {}
    for path in sorted(ledger.glob("*.access.json")) if ledger.exists() else []:
        membership = read_object(path).get("test_membership")
        if not isinstance(membership, dict):
            continue
        families.update(hash_set(membership.get("family_ids_sha256"), "ledger test families"))
        provenance[str(path)] = checksum(path)
    return families, provenance


def load_exclusions(repository: Path, base: Path, ud_origin: Path, technical_origin: Path, prefix_inventory: Path,
                    lexicon: Path, extra_exposure: Sequence[Path] = (), ledger: Path | None = None) -> Exclusions:
    provenance: dict[str, str] = {}
    closure_path = ud_origin / "physical-family-closure.json"
    closure = read_object(closure_path)
    alias_map = closure.get("alias_sha256_to_closure_sha256")
    if not isinstance(alias_map, dict):
        raise ValueError("invalid UD physical alias sidecar")
    ud_aliases = hash_set(alias_map, "alias_sha256_to_closure_sha256")
    retained = hash_set(closure.get("retained_test_closure_sha256"), "retained_test_closure_sha256")
    ud_test_aliases = {alias for alias, member in cast(dict[str, str], alias_map).items() if member in retained}
    families_path = technical_origin / "family-aliases.json"
    families = read_object(families_path).get("families")
    if not isinstance(families, dict):
        raise ValueError("invalid technical family sidecar")
    technical_aliases: set[str] = set()
    technical_test_aliases: set[str] = set()
    for record in cast(dict[str, dict[str, object]], families).values():
        aliases = hash_set(record.get("aliases_sha256"), "family aliases")
        technical_aliases.update(aliases)
        if record.get("split") == "test":
            technical_test_aliases.update(aliases)
    inventory = read_object(prefix_inventory)
    prefix_aliases = hash_set(inventory.get("aliases_sha256"), "prefix inventory")
    membership = read_object(base / "test-membership.json")
    if checksum(base / "test-membership.json") != read_object(base / "manifest.json").get("test_membership_sha256"):
        raise ValueError("base test membership differs from its manifest")
    prior_families = hash_set(membership.get("family_ids_sha256"), "base test families")
    if ledger is not None:
        ledger_families, ledger_provenance = ledger_test_families(ledger)
        prior_families |= ledger_families
        provenance.update(ledger_provenance)
    exposed, sources = exposed_families(repository, extra_exposure)
    code = repository / "tools/train_context_model.py"
    historical = historical_code_forms(code.read_text(encoding="utf-8"))
    historical_aliases: set[str] = set()
    for form in sorted(exposed | historical):
        historical_aliases.update(expanded_aliases(form))
    names = read_object(lexicon).get("identifiers")
    if not isinstance(names, list) or any(not isinstance(name, str) for name in names):
        raise ValueError("invalid identifier lexicon")
    provenance.update(sources)
    for path in (closure_path, families_path, prefix_inventory, base / "manifest.json", base / "test-membership.json", lexicon, code,
                 ud_origin / "manifest.json", technical_origin / "manifest.json"):
        provenance[str(path.relative_to(repository)) if path.is_relative_to(repository) else str(path)] = checksum(path)
    return Exclusions(frozenset(ud_aliases | technical_aliases), frozenset(ud_test_aliases | technical_test_aliases),
                      frozenset(prefix_aliases), frozenset(prior_families), frozenset(exposed), frozenset(historical_aliases),
                      frozenset(cast(list[str], names)), provenance)


def alias_reasons(hashes: set[str], physical_forms: set[str], family: str, exclusions: Exclusions) -> list[str]:
    reasons = []
    if hashes & exclusions.prior_test_aliases or family in exclusions.prior_test_families:
        reasons.append("prior-accessed-test-family")
    if hashes & exclusions.base_aliases:
        reasons.append("family-in-base-corpus")
    if hashes & exclusions.prefix_aliases:
        reasons.append("prefix-curriculum-prior-exposure")
    if physical_forms & exclusions.exposed_physical or hashes & exclusions.historical_aliases:
        reasons.append("family-previously-exposed")
    return reasons


def ud_holdout_rows(sentences: Sequence[Sentence], exclusions: Exclusions) -> tuple[list[CorpusRow], dict[str, object]]:
    families = Union()
    forms: dict[str, set[str]] = defaultdict(set)
    for sentence in sentences:
        for token in sentence.tokens:
            aliases = family_aliases(token, row_identifier(sentence, token))
            for alias in aliases:
                families.join(aliases[0], alias)
    rows = [row for sentence in sentences for row in sentence_rows(sentence, families)]
    if len({row.identifier for row in rows}) != len(rows):
        raise ValueError("duplicate holdout row identifiers")
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
    reasons_by_family = {family: alias_reasons(hashes_by_family[family], physical_by_family[family], family, exclusions)
                         for family in hashes_by_family}
    result = []
    reasons_count: Counter[str] = Counter()
    for row in rows:
        reasons = list(reasons_by_family.get(row.family, []))
        if row.alignment != "exact-text-comment":
            reasons.append(row.alignment)
        split = "quarantine" if reasons else "test"
        reasons_count.update(reasons)
        result.append(replace(row, split=split, quarantine_reasons=tuple(reasons)))
    result.sort(key=lambda row: row.identifier)
    return result, {"rows": len(result), "families": len(hashes_by_family), "test_rows": sum(row.split == "test" for row in result),
                    "test_families": len({row.family for row in result if row.split == "test"}),
                    "test_documents": len({row.document for row in result if row.split == "test"}),
                    "quarantine_reasons": dict(reasons_count)}


def command_identifier(name: str, source: str = SID_SOURCE) -> str:
    return source.lower() + ":command:" + name


def sid_holdout_rows(commands: Sequence[Command], exclusions: Exclusions, namespace: str) -> tuple[list[CorpusRow], dict[str, object]]:
    """Debian sid commands absent from the shipped lexicon, as a test-only technical holdout."""
    return technical_holdout_rows(commands, exclusions, namespace, source=SID_SOURCE, source_file="Contents-amd64.gz",
                                  document_prefix="debian-package-component:")


def arch_holdout_rows(commands: Sequence[Command], exclusions: Exclusions, namespace: str) -> tuple[list[CorpusRow], dict[str, object]]:
    """Arch Linux core/extra commands absent from the shipped lexicon, as a test-only technical holdout."""
    return technical_holdout_rows(commands, exclusions, namespace, source=ARCH_SOURCE, source_file="core.files+extra.files",
                                  document_prefix="arch-package-component:")


def fedora_holdout_rows(commands: Sequence[Command], exclusions: Exclusions, namespace: str) -> tuple[list[CorpusRow], dict[str, object]]:
    """Fedora Everything commands absent from the shipped lexicon, as a test-only technical holdout."""
    return technical_holdout_rows(commands, exclusions, namespace, source=FEDORA_SOURCE, source_file="primary.xml.zst",
                                  document_prefix="fedora-package-component:")


def technical_holdout_rows(commands: Sequence[Command], exclusions: Exclusions, namespace: str, *, source: str, source_file: str,
                           document_prefix: str) -> tuple[list[CorpusRow], dict[str, object]]:
    """Commands absent from the shipped lexicon, as a test-only technical holdout; the technical
    corpus family and package grouping and its fixed family cap are reproduced."""
    records = sorted((item for item in commands if item.name not in exclusions.lexicon), key=lambda item: item.name)
    if len({item.name for item in records}) != len(records) or any(not item.owners for item in records):
        raise ValueError("commands must have unique names and nonempty package owners")
    family_union = Union()
    aliases = {item.name: frozenset(command_aliases(item)) for item in records}
    for item in records:
        anchor = "command:" + item.name
        family_union.find(anchor)
        for alias in sorted(aliases[item.name]):
            family_union.join(anchor, "alias:" + alias)
    families = {item.name: digest(family_union.find("command:" + item.name)) for item in records}
    package_union = Union()
    for item in records:
        anchor = "family:" + families[item.name]
        for owner in item.owners:
            package_union.join(anchor, "package:" + owner.rsplit("/", 1)[-1])
    documents = {item.name: document_prefix + digest(package_union.find("family:" + families[item.name])) for item in records}
    members: dict[str, list[str]] = defaultdict(list)
    for item in records:
        members[families[item.name]].append(item.name)
    retained = {name for names in members.values()
                for name in sorted(names, key=lambda value: digest(namespace + ":family-cap:" + command_identifier(value, source)))[:2]}
    family_hashes: dict[str, set[str]] = defaultdict(set)
    for item in records:
        family_hashes[families[item.name]].update(aliases[item.name])
    rows = []
    reasons_count: Counter[str] = Counter()
    for item in records:
        family = families[item.name]
        reasons = alias_reasons(family_hashes[family], {physical(item.name)}, family, exclusions)
        if item.name not in retained:
            reasons.append("fixed-hash-family-cap")
        reasons_count.update(reasons)
        rows.append(CorpusRow(
            identifier=command_identifier(item.name, source), original=item.name, group=0, before="", after="", lemma=item.name,
            family=family, document=documents[item.name], language="en", source=source, source_file=source_file,
            source_sentence=command_identifier(item.name, source), source_token="1", spacing=" ", space_before="", literal_tail="",
            upos="X", features="_", misc="_", alignment="declared-command-name", layout_representable=True,
            split="quarantine" if reasons else "test", quarantine_reasons=tuple(reasons)))
    return rows, {"commands_in_index": len(commands), "commands_outside_lexicon": len(records), "families": len(members),
                  "test_rows": sum(row.split == "test" for row in rows), "test_families": len({row.family for row in rows if row.split == "test"}),
                  "test_documents": len({row.document for row in rows if row.split == "test"}), "family_cap": 2,
                  "quarantine_reasons": dict(reasons_count)}


def verified_sid_source(directory: Path) -> tuple[Path, dict[str, str], dict[str, object]]:
    receipt_path = directory / "source-receipt.json"
    receipt = read_object(receipt_path)
    contents, release = directory / "Contents-amd64.gz", directory / "InRelease"
    data = receipt.get("contents")
    release_data = receipt.get("release")
    if (not isinstance(data, dict) or not isinstance(release_data, dict)
            or data.get("status") != 200 or release_data.get("status") != 200
            or receipt.get("tls_certificate_verification") is not True
            or data.get("url") != SID_CONTENTS_URL or release_data.get("url") != SID_RELEASE_URL):
        raise ValueError("source receipt does not identify verified TLS Debian sid downloads")
    if (checksum(contents) != receipt.get("contents_sha256") or contents.stat().st_size != receipt.get("contents_bytes")
            or checksum(release) != receipt.get("release_sha256")):
        raise ValueError("Debian sid source checksum mismatch")
    expected = (str(receipt["contents_sha256"]), str(receipt["contents_bytes"]), "main/Contents-amd64.gz")
    section = found = False
    for line in release.read_text(encoding="utf-8").splitlines():
        if line == "SHA256:":
            section = True
        elif section and line and not line.startswith(" "):
            section = False
        elif section and tuple(line.split()) == expected:
            found = True
    if not found:
        raise ValueError("sid Contents checksum absent from InRelease SHA256 section")
    return contents, {str(path): checksum(path) for path in (receipt_path, contents, release)}, {
        "source": "Debian sid main amd64 Contents", "receipt": receipt,
        "verification": "HTTPS TLS and Contents SHA256 against downloaded InRelease; OpenPGP signature not verified by this generator"}


def read_arch_commands(databases: Sequence[tuple[str, Path]]) -> list[Command]:
    """Executables of pacman ``.files`` databases: ``usr/bin`` entries of every package, owned as ``repository/package``."""
    import tarfile
    paths: dict[str, set[str]] = defaultdict(set)
    owners: dict[str, set[str]] = defaultdict(set)
    for repository, database in databases:
        with tarfile.open(database, "r:gz") as archive:
            members = {member.name: member for member in archive.getmembers() if member.isfile()}
            for name, member in sorted(members.items()):
                if not name.endswith("/files"):
                    continue
                package_directory = name.rsplit("/", 1)[0]
                description = members.get(package_directory + "/desc")
                if description is None:
                    raise ValueError("package files without a description: " + package_directory)
                stream = archive.extractfile(description)
                if stream is None:
                    raise ValueError("unreadable package description: " + package_directory)
                lines = stream.read().decode("utf-8").splitlines()
                try:
                    package = lines[lines.index("%NAME%") + 1]
                except (ValueError, IndexError):
                    raise ValueError("package description without a name: " + package_directory) from None
                if re.fullmatch(r"[a-z0-9@._+-]+", package) is None:
                    raise ValueError("invalid package name in files database: " + package)
                files = archive.extractfile(member)
                if files is None:
                    raise ValueError("unreadable package file list: " + package_directory)
                for line in files.read().decode("utf-8").splitlines():
                    match = COMMAND_PATH.fullmatch(line)
                    if match is None:
                        continue
                    paths[match[1]].add(line)
                    owners[match[1]].add(repository + "/" + package)
    return [Command(name, tuple(sorted(paths[name])), tuple(sorted(owners[name]))) for name in sorted(paths)]


def verified_arch_source(directory: Path) -> tuple[list[tuple[str, Path]], dict[str, str], dict[str, object]]:
    """The Arch file databases named by a TLS receipt whose SHA-256 and sizes match the files on disk."""
    receipt_path = directory / "source-receipt.json"
    receipt = read_object(receipt_path)
    if receipt.get("tls_certificate_verification") is not True or receipt.get("mirror") != ARCH_MIRROR:
        raise ValueError("source receipt does not identify verified TLS Arch Linux downloads")
    databases = receipt.get("databases")
    if not isinstance(databases, dict) or set(databases) != set(ARCH_REPOSITORIES):
        raise ValueError("source receipt does not cover the core and extra databases")
    paths: list[tuple[str, Path]] = []
    provenance = {str(receipt_path): checksum(receipt_path)}
    for repository in ARCH_REPOSITORIES:
        entry = cast(dict[str, object], databases[repository]) if isinstance(databases[repository], dict) else {}
        download = cast(dict[str, object], entry.get("download")) if isinstance(entry.get("download"), dict) else {}
        path = directory / f"{repository}.files"
        if (download.get("status") != 200 or download.get("url") != f"{ARCH_MIRROR}{repository}/os/x86_64/{repository}.files"
                or entry.get("file") != f"{repository}.files"):
            raise ValueError("source receipt does not identify the " + repository + " files database download")
        if checksum(path) != entry.get("sha256") or path.stat().st_size != entry.get("bytes"):
            raise ValueError("Arch " + repository + " files database checksum mismatch")
        paths.append((repository, path))
        provenance[str(path)] = checksum(path)
    return paths, provenance, {"source": "Arch Linux core and extra x86_64 package file databases", "receipt": receipt,
                               "verification": "HTTPS TLS and SHA-256 of the downloaded databases; the mirror publishes no detached signature for .files"}


def read_fedora_commands(primary: Path) -> list[Command]:
    """Executables of a Fedora ``primary.xml`` index: the primary filter keeps ``bin`` paths."""
    import xml.etree.ElementTree as ElementTree
    from compression.zstd import ZstdFile

    paths: dict[str, set[str]] = defaultdict(set)
    owners: dict[str, set[str]] = defaultdict(set)
    with ZstdFile(primary, "rb") as stream:
        package = ""
        files: list[str] = []
        for event, element in ElementTree.iterparse(stream, ("start", "end")):
            tag = element.tag.rsplit("}", 1)[-1]
            if event == "start" and tag == "package":
                package, files = "", []
                continue
            if event != "end":
                continue
            if tag == "name" and not package:
                package = (element.text or "").strip()
            elif tag == "file":
                files.append((element.text or "").strip())
            elif tag == "package":
                # Fedora package names may carry capitals and digits (0xFFFF, ImageMagick).
                if re.fullmatch(r"[A-Za-z0-9@._+-]+", package) is None:
                    raise ValueError("invalid package name in the Fedora index: " + package)
                for name in files:
                    match = COMMAND_PATH.fullmatch(name.lstrip("/"))
                    if match is not None:
                        paths[match[1]].add(name.lstrip("/"))
                        owners[match[1]].add("fedora/" + package)
                element.clear()
    return [Command(name, tuple(sorted(paths[name])), tuple(sorted(owners[name]))) for name in sorted(paths)]


def verified_fedora_source(directory: Path) -> tuple[Path, dict[str, str], dict[str, object]]:
    """The Fedora index named by a TLS receipt whose SHA-256 matches ``repomd.xml`` and the payload."""
    receipt_path = directory / "source-receipt.json"
    receipt = read_object(receipt_path)
    repomd, primary = directory / "repomd.xml", directory / "primary.xml.zst"
    entry = receipt.get("primary")
    release = receipt.get("repomd")
    if (not isinstance(entry, dict) or not isinstance(release, dict)
            or receipt.get("tls_certificate_verification") is not True
            or receipt.get("base_url") != FEDORA_BASE or release.get("status") != 200):
        raise ValueError("source receipt does not identify verified TLS Fedora downloads")
    if checksum(repomd) != receipt.get("repomd_sha256") or checksum(primary) != entry.get("sha256"):
        raise ValueError("Fedora index checksum mismatch")
    href = str(entry.get("href", ""))
    text = repomd.read_text(encoding="utf-8")
    if f'href="{href}"' not in text or f'>{entry.get("sha256")}<' not in text:
        raise ValueError("primary metadata is not the one repomd.xml names")
    return primary, {str(path): checksum(path) for path in (receipt_path, repomd, primary)}, {
        "source": "Fedora 43 Everything x86_64 package index", "receipt": receipt,
        "verification": "HTTPS TLS and the primary SHA-256 from repomd.xml; OpenPGP signature not verified by this generator"}


def gzip_member(rows: Sequence[CorpusRow]) -> tuple[bytes, bytes, int]:
    raw = b"".join(canonical(asdict(row)) for row in rows)
    buffer = bytearray()
    import io
    stream = io.BytesIO()
    with gzip.GzipFile(fileobj=stream, filename="", mode="wb", mtime=0) as compressed:
        compressed.write(raw)
    buffer.extend(stream.getvalue())
    return bytes(buffer), raw, len(rows)


def base_test_rows_to_quarantine(base: Path) -> list[CorpusRow]:
    """The previously accessed TEST rows, relabelled programmatically; their text is never printed."""
    manifest = read_object(base / "manifest.json")
    record = cast(dict[str, object], cast(dict[str, object], manifest["splits"])["test"])
    path = base / str(record["path"])
    if checksum(path) != record["sha256"]:
        raise ValueError("base test split checksum mismatch")
    rows = []
    with gzip.open(path, "rb") as stream:
        for line in stream:
            value = json.loads(line)
            value["quarantine_reasons"] = tuple(value["quarantine_reasons"])
            row = CorpusRow(**value)
            if row.split != "test":
                raise ValueError("base test split contains a foreign row")
            rows.append(replace(row, split="quarantine", quarantine_reasons=(PRIOR_TEST,)))
    if len(rows) != record["rows"]:
        raise ValueError("base test row count mismatch")
    return rows


def assemble(base: Path, output: Path, namespace: str, ud_rows: Sequence[CorpusRow], sid_rows: Sequence[CorpusRow],
             provenance: Mapping[str, str], metadata: Mapping[str, object]) -> dict[str, object]:
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
    for split in ("train", "development", "calibration"):
        source = base / str(base_splits[split]["path"])
        if checksum(source) != base_splits[split]["sha256"]:
            raise ValueError("base split checksum mismatch: " + split)
        shutil.copyfile(source, output / (split + ".jsonl.gz"))
        files[split] = dict(base_splits[split])
    test_members = [gzip_member([row for row in ud_rows if row.split == "test"]),
                    gzip_member([row for row in sid_rows if row.split == "test"])]
    quarantine_source = base / str(base_splits["quarantine"]["path"])
    if checksum(quarantine_source) != base_splits["quarantine"]["sha256"]:
        raise ValueError("base quarantine checksum mismatch")
    base_quarantine = quarantine_source.read_bytes()
    prior_rows = base_test_rows_to_quarantine(base)
    quarantine_members = [gzip_member(prior_rows), gzip_member([row for row in ud_rows if row.split == "quarantine"]),
                          gzip_member([row for row in sid_rows if row.split == "quarantine"])]
    for split, prefix_bytes, prefix_raw, prefix_rows, members in (
            ("test", b"", b"", 0, test_members),
            ("quarantine", base_quarantine, b"", int(cast(int, base_splits["quarantine"]["rows"])), quarantine_members)):
        path = output / (split + ".jsonl.gz")
        with path.open("wb") as destination:
            destination.write(prefix_bytes)
            for compressed, _raw, _count in members:
                destination.write(compressed)
        raw_digest = hashlib.sha256()
        count = 0
        with gzip.open(path, "rb") as stream:
            for line in stream:
                raw_digest.update(line)
                count += 1
        if count != prefix_rows + sum(member[2] for member in members):
            raise ValueError("assembled split row count mismatch: " + split)
        files[split] = {"path": path.name, "sha256": checksum(path), "content_sha256": raw_digest.hexdigest(), "rows": count}
    held = [row for row in (*ud_rows, *sid_rows) if row.split == "test"]
    membership = {"namespace": namespace, "scope": "test-only holdout extension; families known to the base corpus, its accessed test, the prefix curriculum or declared exposure are quarantined",
                  "row_ids_sha256": sorted(digest(row.identifier) for row in held),
                  "family_ids_sha256": sorted({row.family for row in held}),
                  "document_ids_sha256": sorted({digest(row.document) for row in held})}
    membership_path = output / "test-membership.json"
    membership_path.write_bytes(canonical(membership))
    manifest: dict[str, object] = {
        "schema_version": 1, "namespace": namespace, "label": "keep", "splits": files,
        "scope": "holdout extension: base train/development/calibration byte-identical; TEST from new treebanks and commands outside the shipped identifier lexicon; not globally unseen lexicon or verified human intent",
        "token_language": "surface alphabet determines RU/Latin group; ASCII command alphabet uses group 0",
        "context": "within the same sentence only; before<=96 and after<=64 characters; commands have empty before/after",
        "partitioning": {"mode": "test-only-holdout-extension", **dict(metadata)},
        "origins": {"base_manifest_sha256": checksum(base / "manifest.json"), "base_test_membership_sha256": checksum(base / "test-membership.json"),
                    "base_namespace": base_manifest.get("namespace"), "base_test_rows_quarantined": len(prior_rows)},
        "provenance": dict(sorted(provenance.items())),
        "generator_sha256": hashlib.sha256(generator).hexdigest(), "generator_source": "generator-source.py",
        "test_membership_sha256": checksum(membership_path),
        "layout_table_sha256": checksum(ROOT / "src/keyswitch/layouts.py"),
        "compression": "gzip members with mtime=0; base members copied byte-for-byte; raw content SHA256 is independent of deflate implementation",
        "test_policy": "sealed before model scoring; loader reads only the explicitly named split; quarantine is never a training split; the base TEST is quarantined because it was already accessed",
        "limitations": ["UD text is naturally occurring, not reviewed keyboard-intent ground truth",
                        "technical rows measure commands absent from the shipped lexicon: the unknown-identifier regime",
                        "a treebank without document markers is sampled by sentence",
                        "past morphology, dictionaries, private experiments and pretrained models are not exhaustively inventoried"],
    }
    if Path(__file__).read_bytes() != generator:
        raise ValueError("holdout generator changed while producing this corpus")
    (output / "manifest.json").write_bytes(canonical(manifest))
    return manifest


def prior_access_overlap(ledger: Path, membership: Mapping[str, object]) -> dict[str, int]:
    """Dry check of the evaluator's ledger rule: the new TEST must not share rows, families or documents."""
    overlap: Counter[str] = Counter()
    for path in sorted(ledger.glob("*.access.json")) if ledger.exists() else []:
        prior = read_object(path).get("test_membership")
        if not isinstance(prior, dict):
            continue
        for field in ("row_ids_sha256", "family_ids_sha256", "document_ids_sha256"):
            overlap[field] += len(set(cast(list[str], membership[field])) & set(cast(list[str], prior[field])))
    return dict(overlap)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-root", type=Path, required=True)
    parser.add_argument("--sid-directory", type=Path)
    parser.add_argument("--arch-directory", type=Path)
    parser.add_argument("--fedora-directory", type=Path)
    parser.add_argument("--treebank", action="append", default=[], help="pinned treebank to use; default: every pinned treebank")
    parser.add_argument("--base", type=Path, required=True)
    parser.add_argument("--ud-origin", type=Path, required=True)
    parser.add_argument("--technical-origin", type=Path, required=True)
    parser.add_argument("--prefix-inventory", type=Path, required=True)
    parser.add_argument("--lexicon", type=Path, default=ROOT / "src/keyswitch/resources/identifiers.json")
    parser.add_argument("--ledger", type=Path, default=ROOT / ".t/reliable-release-2026-09-12/context-action-test-ledger")
    parser.add_argument("--namespace", required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--extra-exposure", type=Path, action="append", default=[])
    parser.add_argument("--max-documents", type=int, default=2000)
    parser.add_argument("--max-sentences-per-document", type=int, default=12)
    args = parser.parse_args(argv)
    technical = [name for name, value in (("sid", args.sid_directory), ("arch", args.arch_directory),
                                          ("fedora", args.fedora_directory)) if value is not None]
    if len(technical) != 1:
        raise ValueError("exactly one technical source is required: --sid-directory, --arch-directory or --fedora-directory")
    if any(name not in PINS for name in args.treebank):
        raise ValueError("unpinned treebank requested")
    pins = {name: PINS[name] for name in args.treebank} if args.treebank else PINS
    paths, sources = holdout_inventory(args.source_root, pins)
    exclusions = load_exclusions(ROOT, args.base, args.ud_origin, args.technical_origin, args.prefix_inventory,
                                 args.lexicon, args.extra_exposure, ledger=args.ledger)
    sentences = sentence_documents(sentence for source, path in paths for sentence in read_conllu(path, source))
    selected, sampling = select_holdout_sentences(sentences, args.namespace, args.max_documents, args.max_sentences_per_document)
    ud_rows, ud_summary = ud_holdout_rows(selected, exclusions)
    if args.sid_directory is not None:
        contents, sid_provenance, sid_metadata = verified_sid_source(args.sid_directory)
        sid_rows, sid_summary = sid_holdout_rows(read_commands(contents), exclusions, args.namespace)
    elif args.arch_directory is not None:
        databases, sid_provenance, sid_metadata = verified_arch_source(args.arch_directory)
        sid_rows, sid_summary = arch_holdout_rows(read_arch_commands(databases), exclusions, args.namespace)
    else:
        primary, sid_provenance, sid_metadata = verified_fedora_source(args.fedora_directory)
        sid_rows, sid_summary = fedora_holdout_rows(read_fedora_commands(primary), exclusions, args.namespace)
    provenance = {**exclusions.provenance, **sid_provenance}
    for source in sources:
        provenance["holdout-source:" + str(source["repository"])] = str(source["pin_metadata_sha256"])
    held = [row for row in (*ud_rows, *sid_rows) if row.split == "test"]
    membership = {"row_ids_sha256": sorted(digest(row.identifier) for row in held), "family_ids_sha256": sorted({row.family for row in held}),
                  "document_ids_sha256": sorted({digest(row.document) for row in held})}
    overlap = prior_access_overlap(args.ledger, membership)
    metadata = {"sources": sources, "sampling": sampling, "ud": ud_summary, "technical": sid_summary, "sid_source": sid_metadata,
                "prior_access_overlap": overlap, "exclusion_counts": {"base_aliases": len(exclusions.base_aliases),
                "prior_test_aliases": len(exclusions.prior_test_aliases), "prefix_aliases": len(exclusions.prefix_aliases),
                "prior_test_families": len(exclusions.prior_test_families), "exposed_physical": len(exclusions.exposed_physical),
                "historical_aliases": len(exclusions.historical_aliases), "lexicon": len(exclusions.lexicon)}}
    report: dict[str, object] = {"namespace": args.namespace, **metadata, "test_scored": False, "test_text_printed": False}
    if any(overlap.values()):
        raise ValueError("holdout overlaps a previously accessed test: " + json.dumps(overlap))
    if args.output is not None:
        manifest = assemble(args.base, args.output, args.namespace, ud_rows, sid_rows, provenance, metadata)
        report.update({"manifest_sha256": checksum(args.output / "manifest.json"), "test_membership_sha256": manifest["test_membership_sha256"],
                       "splits": manifest["splits"]})
    args.report.write_bytes(canonical(report))
    print(json.dumps({"ud": ud_summary, "technical": sid_summary, "sampling": {k: v for k, v in sampling.items() if k != "policy"},
                      "prior_access_overlap": overlap, "written": args.output is not None}, sort_keys=True, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
