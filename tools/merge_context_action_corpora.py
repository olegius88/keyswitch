#!/usr/bin/env python3
"""Compose frozen corpora as opaque gzip members without reading test JSON."""

from __future__ import annotations

import argparse
from collections.abc import Sequence
from dataclasses import dataclass
import gzip
import hashlib
import json
from pathlib import Path
import re
import shutil
from typing import Protocol, cast
import zlib

from freeze_context_action_corpus import canonical, checksum
from model_protocol import ACTIVE_SPLITS, ALL_SPLITS

ROOT = Path(__file__).resolve().parents[1]
MEMBERSHIP_FIELDS = ("row_ids_sha256", "family_ids_sha256", "document_ids_sha256")
HEX = re.compile(r"[0-9a-f]{64}\Z")


def object_value(value: object) -> dict[str, object]:
    if not isinstance(value, dict) or any(not isinstance(key, str) for key in value):
        raise ValueError("expected corpus metadata object")
    return cast(dict[str, object], value)


def read_object(path: Path) -> dict[str, object]:
    return object_value(json.loads(path.read_bytes()))


def hash_value(value: object) -> str:
    if not isinstance(value, str) or HEX.fullmatch(value) is None:
        raise ValueError("invalid metadata SHA256")
    return value


def hash_list(value: object, *, label: str) -> list[str]:
    if not isinstance(value, list):
        raise ValueError("expected metadata hash list")
    result = [hash_value(item) for item in value]
    if len(set(result)) != len(result):
        raise ValueError("duplicate " + label)
    return result


@dataclass(frozen=True)
class Streamed:
    sha256: str
    content_sha256: str
    rows: int
    raw_bytes: int


class Digest(Protocol):
    def update(self, data: bytes) -> None: ...


def inspect_gzip(path: Path, raw_digest: Digest | None = None, *, expected_sha256: str | None = None) -> Streamed:
    """Count complete JSONL lines as bytes; never deserialize source rows."""
    if not path.is_file() or path.stat().st_size == 0:
        raise ValueError("missing or empty gzip source")
    compressed = checksum(path)
    if expected_sha256 is not None and compressed != expected_sha256:
        raise ValueError("source compressed checksum mismatch")
    digestor = hashlib.sha256()
    count = size = 0
    try:
        with gzip.open(path, "rb") as stream:
            for line in stream:
                if not line.endswith(b"\n") or not line.strip():
                    raise ValueError("source JSONL must contain complete nonempty lines")
                digestor.update(line)
                if raw_digest is not None:
                    raw_digest.update(line)
                count += 1
                size += len(line)
    except (OSError, EOFError, zlib.error):
        raise ValueError("invalid gzip source") from None
    return Streamed(compressed, digestor.hexdigest(), count, size)


@dataclass(frozen=True)
class Origin:
    directory: Path
    manifest: dict[str, object]
    namespace: str
    membership: dict[str, list[str]]
    files: dict[str, Streamed]
    pins: dict[Path, str]


def verify_origin(directory: Path) -> Origin:
    manifest_path = directory / "manifest.json"
    manifest = read_object(manifest_path)
    namespace = manifest.get("namespace")
    if (manifest.get("schema_version") != 1 or manifest.get("label") != "keep"
            or not isinstance(namespace, str) or not namespace):
        raise ValueError("invalid corpus manifest identity")
    records = object_value(manifest.get("splits"))
    if set(records) != set(ALL_SPLITS):
        raise ValueError("source manifest must declare all corpus splits")
    pins = {manifest_path: checksum(manifest_path)}
    files = {}
    for split in ALL_SPLITS:
        record = object_value(records[split])
        if record.get("path") != split + ".jsonl.gz":
            raise ValueError("unexpected source split filename")
        count = record.get("rows")
        if type(count) is not int or count < 0:
            raise ValueError("invalid source row count")
        path = directory / (split + ".jsonl.gz")
        observed = inspect_gzip(path, expected_sha256=hash_value(record.get("sha256")))
        if observed.content_sha256 != hash_value(record.get("content_sha256")):
            raise ValueError("source raw content checksum mismatch")
        if observed.rows != count:
            raise ValueError("source streamed row count mismatch")
        files[split] = observed
        pins[path] = observed.sha256
    if not sum(item.rows for item in files.values()):
        raise ValueError("empty source corpus")
    # The reconciled historical UD manifest omits the filename but retains the
    # original generator SHA and the standard archived generator-source.py.
    generator_name = manifest.get("generator_source", "generator-source.py")
    if not isinstance(generator_name, str) or Path(generator_name).name != generator_name:
        raise ValueError("unexpected origin generator filename")
    generator = directory / generator_name
    if checksum(generator) != hash_value(manifest.get("generator_sha256")):
        raise ValueError("origin generator checksum mismatch")
    pins[generator] = checksum(generator)
    reconciliation = manifest.get("reconciliation")
    if isinstance(reconciliation, dict) and "source_sha256" in reconciliation:
        reconciler = directory / "reconciliation-source.py"
        if checksum(reconciler) != hash_value(reconciliation["source_sha256"]):
            raise ValueError("origin reconciliation generator checksum mismatch")
        pins[reconciler] = checksum(reconciler)
    membership_path = directory / "test-membership.json"
    if checksum(membership_path) != hash_value(manifest.get("test_membership_sha256")):
        raise ValueError("test membership checksum mismatch")
    membership = read_object(membership_path)
    if membership.get("namespace") != namespace:
        raise ValueError("test membership namespace mismatch")
    lists = {field: hash_list(membership.get(field), label="membership hashes") for field in MEMBERSHIP_FIELDS}
    if len(lists["row_ids_sha256"]) != files["test"].rows:
        raise ValueError("test membership row count mismatch")
    if files["test"].rows and any(not lists[field] for field in MEMBERSHIP_FIELDS):
        raise ValueError("nonempty test requires family and document membership")
    pins[membership_path] = checksum(membership_path)
    return Origin(directory, manifest, namespace, lists, files, pins)


def verify_aliases(base: Origin, extension: Origin) -> tuple[dict[str, object], dict[Path, str]]:
    base_path = base.directory / "physical-family-closure.json"
    reconciliation = object_value(base.manifest.get("reconciliation"))
    if checksum(base_path) != hash_value(reconciliation.get("closure_sha256")):
        raise ValueError("base physical sidecar checksum mismatch")
    closure = read_object(base_path)
    base_aliases = object_value(closure.get("alias_sha256_to_closure_sha256"))
    for key, value in base_aliases.items():
        hash_value(key)
        hash_value(value)
    if not base_aliases:
        raise ValueError("empty base physical alias sidecar")
    extension_path = extension.directory / "family-aliases.json"
    if checksum(extension_path) != hash_value(extension.manifest.get("family_aliases_sha256")):
        raise ValueError("technical family sidecar checksum mismatch")
    sidecar = read_object(extension_path)
    if sidecar.get("schema_version") != 1:
        raise ValueError("unsupported technical family sidecar")
    families = object_value(sidecar.get("families"))
    aliases_seen: dict[str, tuple[str, str]] = {}
    documents_seen: dict[str, str] = {}
    test_families: set[str] = set()
    test_documents: set[str] = set()
    for family, raw in families.items():
        hash_value(family)
        record = object_value(raw)
        split = record.get("split")
        if not isinstance(split, str) or split not in ACTIVE_SPLITS:
            raise ValueError("technical family has invalid retained split")
        aliases = hash_list(record.get("aliases_sha256"), label="technical aliases")
        documents = hash_list(record.get("document_ids_sha256"), label="technical documents")
        if not aliases or not documents:
            raise ValueError("technical family requires aliases and documents")
        for alias in aliases:
            if alias in base_aliases:
                raise ValueError("technical family overlaps base physical alias")
            if alias in aliases_seen and aliases_seen[alias] != (family, split):
                raise ValueError("technical alias belongs to multiple families or splits")
            aliases_seen[alias] = family, split
        for document in documents:
            if document in documents_seen and documents_seen[document] != split:
                raise ValueError("technical document spans multiple splits")
            documents_seen[document] = split
        if split == "test":
            test_families.add(family)
            test_documents.update(documents)
    if test_families != set(extension.membership["family_ids_sha256"]):
        raise ValueError("technical sidecar test family membership mismatch")
    if test_documents != set(extension.membership["document_ids_sha256"]):
        raise ValueError("technical sidecar test document membership mismatch")
    for field in MEMBERSHIP_FIELDS:
        if set(base.membership[field]) & set(extension.membership[field]):
            raise ValueError("origin test membership overlap: " + field)
    return {
        "base_aliases": len(base_aliases), "technical_retained_families": len(families),
        "technical_aliases": len(aliases_seen), "base_technical_alias_overlap": 0,
        "technical_cross_split_alias_overlap": 0, "technical_cross_split_document_overlap": 0,
        "test_membership_origin_overlap": 0,
        "scope": "source metadata hashes and all physical aliases; no model scoring or test row parsing",
    }, {base_path: checksum(base_path), extension_path: checksum(extension_path)}


def merge_corpora(base_directory: Path, extension_directory: Path, output: Path) -> dict[str, object]:
    if output.exists():
        raise ValueError("refusing to overwrite combined corpus")
    base, extension = verify_origin(base_directory), verify_origin(extension_directory)
    audit, sidecar_pins = verify_aliases(base, extension)
    generator = Path(__file__).read_bytes()
    pins = {**base.pins, **extension.pins, **sidecar_pins,
            Path(__file__): hashlib.sha256(generator).hexdigest(),
            ROOT / "tools/freeze_context_action_corpus.py": checksum(ROOT / "tools/freeze_context_action_corpus.py"),
            ROOT / "tests/test_context_action_merge.py": checksum(ROOT / "tests/test_context_action_merge.py")}
    membership: dict[str, object] = {
        "namespace": base.namespace,
        "scope": "unchanged original UD membership plus separately frozen command-family/package membership",
    }
    for field in MEMBERSHIP_FIELDS:
        membership[field] = sorted(base.membership[field] + extension.membership[field])
    output.mkdir(parents=True)
    origin_directory = output / "origins"
    origin_directory.mkdir()
    origins = []
    for name, origin in (("base", base), ("technical", extension)):
        saved = origin_directory / (name + "-manifest.json")
        shutil.copyfile(origin.directory / "manifest.json", saved)
        origins.append({
            "name": name, "namespace": origin.namespace, "manifest": "origins/" + saved.name,
            "manifest_sha256": checksum(saved), "generator_sha256": origin.manifest["generator_sha256"],
            "test_membership_sha256": origin.manifest["test_membership_sha256"],
            "splits": origin.manifest["splits"],
        })
    (output / "generator-source.py").write_bytes(generator)
    files = {}
    for split in ALL_SPLITS:
        destination = output / (split + ".jsonl.gz")
        raw_hash = hashlib.sha256()
        count = raw_bytes = 0
        with destination.open("xb") as stream:
            for origin in (base, extension):
                source = origin.directory / destination.name
                current = inspect_gzip(source, raw_hash)
                if current != origin.files[split]:
                    raise ValueError("source changed during corpus composition")
                copied = hashlib.sha256()
                with source.open("rb") as incoming:
                    for chunk in iter(lambda: incoming.read(1024 * 1024), b""):
                        copied.update(chunk)
                        stream.write(chunk)
                if copied.hexdigest() != current.sha256:
                    raise ValueError("copied source bytes differ from verified source")
                count += current.rows
                raw_bytes += current.raw_bytes
        combined = inspect_gzip(destination)
        if combined.content_sha256 != raw_hash.hexdigest() or combined.rows != count or combined.raw_bytes != raw_bytes:
            raise ValueError("combined gzip stream differs from source streams")
        files[split] = {"path": destination.name, "sha256": checksum(destination),
                        "content_sha256": raw_hash.hexdigest(), "rows": count, "raw_bytes": raw_bytes,
                        "gzip_members_from": ["base", "technical"]}
    if len(cast(list[str], membership["row_ids_sha256"])) != files["test"]["rows"]:
        raise ValueError("combined test membership row count mismatch")
    membership_path = output / "test-membership.json"
    membership_path.write_bytes(canonical(membership))
    for path, expected in pins.items():
        if checksum(path) != expected:
            raise ValueError("source or generator changed while merging corpora")
    manifest: dict[str, object] = {
        "schema_version": 1, "namespace": base.namespace, "label": "keep", "splits": files,
        "scope": "union of unchanged frozen UD and command-family/package partitions; not new independent evidence",
        "origins": origins, "alias_audit": audit,
        "partitioning": {"mode": "unchanged-source-split-concatenation", "replacements": 0,
                         "rows_by_split": {split: files[split]["rows"] for split in ALL_SPLITS}},
        "generator_source": "generator-source.py", "generator_sha256": hashlib.sha256(generator).hexdigest(),
        "test_membership_sha256": checksum(membership_path),
        "provenance": {str(path): expected for path, expected in sorted(pins.items(), key=lambda item: str(item[0]))},
        "compression": "byte-exact base gzip members followed by byte-exact technical gzip members; no recompression",
        "test_policy": "test JSON never parsed or printed during merge; only opaque byte-stream checks and hashed membership read; future scoring requires a new final recipe/seal and joint gate",
        "limitations": [
            "source counts and byte hashes are verified, not the semantic correctness of labels or row JSON",
            "source alias sidecars attest declared physical families, not globally unseen lexicons or human intent",
            "original source-specific reconciliation and evaluation statistics remain only in origin manifests",
            "union membership adds an unscored technical holdout and does not erase any prior UD exposure",
            "cryptographic source hashes provide consistency, not signed provenance or a new quality evaluation",
        ],
    }
    (output / "manifest.json").write_bytes(canonical(manifest))
    return manifest


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", type=Path, required=True)
    parser.add_argument("--technical", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    manifest = merge_corpora(args.base, args.technical, args.output)
    print(json.dumps({"manifest_sha256": checksum(args.output / "manifest.json"),
                      "test_membership_sha256": manifest["test_membership_sha256"],
                      "partitioning": manifest["partitioning"], "alias_audit": manifest["alias_audit"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
