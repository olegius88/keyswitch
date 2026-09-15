#!/usr/bin/env python3
"""Blind transfer of prefix-curriculum-exposed TEST units into quarantine.

The combined corpus is read as opaque gzip members; TEST rows are parsed only
to relabel whole exposed units. No row text is printed, scored or returned.
"""

from __future__ import annotations

import argparse
from collections import Counter, defaultdict
from collections.abc import Iterable, Mapping, Sequence
from dataclasses import dataclass
import gzip
import hashlib
import json
from pathlib import Path
import shutil
from typing import cast
import zlib

from freeze_context_action_corpus import SPLITS, canonical, checksum, digest, load_split
from merge_context_action_corpora import Origin, hash_list, hash_value, inspect_gzip, object_value, read_object, verify_origin

ROOT = Path(__file__).resolve().parents[1]
REASON = "prefix-curriculum-prior-exposure"
CURRICULUM_SPLITS = ("train", "development", "calibration")
MEMBERSHIP_FIELDS = ("row_ids_sha256", "family_ids_sha256", "document_ids_sha256")


def resolve_pin(name: str) -> Path:
    path = Path(name)
    return path if path.is_absolute() else ROOT / path


def pin_name(path: Path) -> str:
    resolved = path.resolve()
    return str(resolved.relative_to(ROOT)) if resolved.is_relative_to(ROOT) else str(resolved)


def verify_pins(pins: Mapping[str, str], message: str) -> None:
    for name, expected in pins.items():
        if checksum(resolve_pin(name)) != expected:
            raise ValueError(message)


def gzip_members(path: Path) -> list[tuple[bytes, bytes]]:
    """Split a multi-member gzip file into (compressed member, raw content) pairs."""
    data = path.read_bytes()
    result: list[tuple[bytes, bytes]] = []
    position = 0
    while position < len(data):
        decoder = zlib.decompressobj(16 + zlib.MAX_WBITS)
        try:
            raw = decoder.decompress(data[position:])
        except zlib.error:
            raise ValueError("invalid gzip member") from None
        if not decoder.eof:
            raise ValueError("truncated gzip member")
        used = len(data) - position - len(decoder.unused_data)
        result.append((data[position:position + used], raw))
        position += used
    return result


def raw_lines(raw: bytes) -> tuple[bytes, ...]:
    if raw and not raw.endswith(b"\n"):
        raise ValueError("member must end with a newline")
    lines = tuple(line + b"\n" for line in raw.split(b"\n")[:-1])
    if any(not line.strip() for line in lines):
        raise ValueError("member contains an empty line")
    return lines


@dataclass(frozen=True)
class TestRow:
    line: bytes
    row_sha256: str
    family: str
    document_sha256: str
    unit: str | None


@dataclass(frozen=True)
class TestMember:
    name: str
    compressed: bytes
    rows: tuple[TestRow, ...]

    @property
    def moved(self) -> bool:
        return any(row.unit is not None for row in self.rows)

    @property
    def retained_lines(self) -> bytes:
        return b"".join(row.line for row in self.rows if row.unit is None)


@dataclass(frozen=True)
class Plan:
    corpus: Path
    origins: tuple[Origin, ...]
    manifest: dict[str, object]
    records: dict[str, dict[str, object]]
    test_members: tuple[TestMember, ...]
    moved_lines: tuple[bytes, ...]
    membership: dict[str, object]
    sidecar: dict[str, object]
    summary: dict[str, object]
    pins: dict[str, str]


def load_inventory(path: Path, expected_sha256: str | None, pins: dict[str, str]) -> tuple[set[str], dict[str, set[str]], dict[str, set[str]]]:
    observed = checksum(path)
    if expected_sha256 is not None and observed != expected_sha256:
        raise ValueError("inventory checksum mismatch")
    inventory = read_object(path)
    if inventory.get("schema_version") != 1 or not isinstance(inventory.get("kind"), str):
        raise ValueError("unsupported inventory identity")
    if inventory.get("test_json_opened") is not False or inventory.get("model_scoring") is not False:
        raise ValueError("inventory must attest closed test and no scoring")
    aliases = set(hash_list(inventory.get("aliases_sha256"), label="inventory aliases"))
    splits_of: dict[str, set[str]] = defaultdict(set)
    scopes_of: dict[str, set[str]] = defaultdict(set)
    for field, index in (("aliases_by_split", splits_of), ("aliases_by_scope", scopes_of)):
        groups = object_value(inventory.get(field))
        if field == "aliases_by_split" and not set(groups) <= set(CURRICULUM_SPLITS):
            raise ValueError("inventory alias splits must be curriculum splits")
        for group, values in groups.items():
            for alias in hash_list(values, label="inventory group aliases"):
                index[alias].add(group)
        if set(index) != aliases:
            raise ValueError("inventory alias lists are inconsistent")
    provenance = object_value(inventory.get("provenance"))
    for name, expected in provenance.items():
        if checksum(resolve_pin(name)) != hash_value(expected):
            raise ValueError("inventory input changed since the audit")
        pins[name] = hash_value(expected)
    pins[pin_name(path)] = observed
    return aliases, splits_of, scopes_of


def matched_units(aliases: set[str], closure: Mapping[str, object], families: Mapping[str, object]) -> tuple[dict[str, set[str]], dict[str, set[str]]]:
    """Re-derive every exposed TEST unit from the full alias intersection."""
    retained = set(hash_list(closure.get("retained_test_closure_sha256"), label="retained closures"))
    ud: dict[str, set[str]] = defaultdict(set)
    for alias, target in object_value(closure.get("alias_sha256_to_closure_sha256")).items():
        family = hash_value(target)
        if family in retained and hash_value(alias) in aliases:
            ud[family].add(alias)
    technical: dict[str, set[str]] = {}
    for family, raw in families.items():
        record = object_value(raw)
        if record.get("split") != "test":
            continue
        matched = aliases.intersection(hash_list(record.get("aliases_sha256"), label="technical aliases"))
        if matched:
            technical[hash_value(family)] = matched
    return dict(ud), technical


def cross_check(collisions: Path, ud: Mapping[str, object], technical: Mapping[str, object], pins: dict[str, str]) -> None:
    expected: dict[str, set[str]] = defaultdict(set)
    for raw in cast(list[object], read_object(collisions).get("families")):
        item = object_value(raw)
        expected[str(item.get("target_origin"))].add(hash_value(item.get("target_family_closure_sha256")))
    if expected.get("ud", set()) != set(ud) or expected.get("technical", set()) != set(technical):
        raise ValueError("collision inventory differs from re-derived units")
    pins[pin_name(collisions)] = checksum(collisions)


def parse_test_member(name: str, compressed: bytes, raw: bytes, unit_of: Mapping[str, str]) -> TestMember:
    rows = []
    for line in raw_lines(raw):
        value: object = json.loads(line)
        if not isinstance(value, dict) or canonical(value) != line:
            raise ValueError("test row is not canonical JSON")
        record = cast(dict[str, object], value)
        family, identifier, document = record.get("family"), record.get("identifier"), record.get("document")
        if not isinstance(family, str) or not isinstance(identifier, str) or not isinstance(document, str):
            raise ValueError("test row lacks identity fields")
        if record.get("split") != "test" or record.get("quarantine_reasons") != []:
            raise ValueError("test row is not a clean retained test row")
        rows.append(TestRow(line, digest(identifier), family, digest(document), unit_of.get(family)))
    return TestMember(name, compressed, tuple(rows))


def relabel(line: bytes) -> bytes:
    record = cast(dict[str, object], json.loads(line))
    reasons = cast(list[str], record["quarantine_reasons"])
    record["split"] = "quarantine"
    record["quarantine_reasons"] = sorted({*reasons, REASON})
    return canonical(record)


def reconcile(corpus: Path, inventory: Path, ud_origin: Path, technical_origin: Path, *,
              inventory_sha256: str | None = None, collisions: Path | None = None) -> Plan:
    manifest_path = corpus / "manifest.json"
    manifest = read_object(manifest_path)
    namespace = manifest.get("namespace")
    if manifest.get("schema_version") != 1 or manifest.get("label") != "keep" or not isinstance(namespace, str):
        raise ValueError("invalid parent corpus manifest identity")
    records = {split: object_value(record) for split, record in object_value(manifest.get("splits")).items()}
    if set(records) != set(SPLITS):
        raise ValueError("parent manifest must declare all corpus splits")
    pins: dict[str, str] = {pin_name(manifest_path): checksum(manifest_path)}
    for name, expected in object_value(manifest.get("provenance", {})).items():
        if checksum(resolve_pin(name)) != hash_value(expected):
            raise ValueError("parent provenance input changed")
        pins[name] = hash_value(expected)
    declared = [object_value(item) for item in cast(list[object], manifest.get("origins"))]
    if [item.get("name") for item in declared] != ["base", "technical"]:
        raise ValueError("parent must declare base and technical origins")
    origins = []
    for item, directory in zip(declared, (ud_origin, technical_origin), strict=True):
        archived = corpus / str(item.get("manifest"))
        if not archived.resolve().is_relative_to(corpus.resolve()):
            raise ValueError("archived origin manifest must live inside the parent corpus")
        expected = hash_value(item.get("manifest_sha256"))
        if checksum(archived) != expected or checksum(directory / "manifest.json") != expected:
            raise ValueError("origin manifest differs from the archived parent origin")
        origin = verify_origin(directory)
        if origin.manifest.get("splits") != item.get("splits"):
            raise ValueError("origin split records differ from the parent declaration")
        pins[pin_name(archived)] = expected
        pins.update({pin_name(path): value for path, value in origin.pins.items()})
        origins.append(origin)
    base, technical = origins
    closure_path = base.directory / "physical-family-closure.json"
    if checksum(closure_path) != hash_value(object_value(base.manifest.get("reconciliation")).get("closure_sha256")):
        raise ValueError("closure sidecar checksum mismatch")
    families_path = technical.directory / "family-aliases.json"
    if checksum(families_path) != hash_value(technical.manifest.get("family_aliases_sha256")):
        raise ValueError("technical family sidecar checksum mismatch")
    pins[pin_name(closure_path)] = checksum(closure_path)
    pins[pin_name(families_path)] = checksum(families_path)
    for split in SPLITS:
        path = corpus / (split + ".jsonl.gz")
        if records[split].get("path") != path.name or checksum(path) != hash_value(records[split].get("sha256")):
            raise ValueError("parent compressed split checksum mismatch")
        pins[pin_name(path)] = hash_value(records[split].get("sha256"))
    membership_path = corpus / "test-membership.json"
    if checksum(membership_path) != hash_value(manifest.get("test_membership_sha256")):
        raise ValueError("parent test membership checksum mismatch")
    pins[pin_name(membership_path)] = checksum(membership_path)
    generator = corpus / str(manifest.get("generator_source", "generator-source.py"))
    if checksum(generator) != hash_value(manifest.get("generator_sha256")):
        raise ValueError("parent generator checksum mismatch")
    pins[pin_name(generator)] = checksum(generator)
    parent_membership = {field: hash_list(read_object(membership_path).get(field), label="membership hashes") for field in MEMBERSHIP_FIELDS}

    aliases, splits_of, scopes_of = load_inventory(inventory, inventory_sha256, pins)
    closure = read_object(closure_path)
    families = object_value(read_object(families_path).get("families"))
    ud_units, technical_units = matched_units(aliases, closure, families)
    if collisions is not None:
        cross_check(collisions, ud_units, technical_units, pins)
    components = object_value(closure.get("components"))
    affected: dict[str, dict[str, set[str]]] = {"ud": {}, "technical": {}}
    for unit in ud_units:
        if unit not in components:
            raise ValueError("matched closure has no component record")
        affected["ud"][unit] = {unit, *hash_list(components[unit], label="component families")}
    for unit in technical_units:
        affected["technical"][unit] = {unit}
    unit_of: dict[str, dict[str, str]] = {"ud": {}, "technical": {}}
    for origin_name, unit_map in affected.items():
        for unit, identifiers in unit_map.items():
            for identifier in identifiers:
                if unit_of[origin_name].setdefault(identifier, unit) != unit:
                    raise ValueError("family belongs to two matched units")
    if set(unit_of["ud"]) & set(unit_of["technical"]):
        raise ValueError("origin family identifiers overlap")

    members_by_split: dict[str, list[tuple[bytes, bytes]]] = {}
    for split in SPLITS:
        members = gzip_members(corpus / (split + ".jsonl.gz"))
        if len(members) != len(origins):
            raise ValueError("parent split must contain one gzip member per origin")
        whole = hashlib.sha256()
        total = 0
        for (compressed, raw), origin in zip(members, origins, strict=True):
            expected_member = object_value(cast(dict[str, object], origin.manifest["splits"])[split])
            lines = raw_lines(raw)
            if (hashlib.sha256(raw).hexdigest() != hash_value(expected_member.get("content_sha256"))
                    or len(lines) != expected_member.get("rows") or hashlib.sha256(compressed).hexdigest() != origin.files[split].sha256):
                raise ValueError("parent gzip member differs from its origin")
            whole.update(raw)
            total += len(lines)
        if whole.hexdigest() != hash_value(records[split].get("content_sha256")) or total != records[split].get("rows"):
            raise ValueError("parent split content differs from its manifest")
        members_by_split[split] = members
    test_members = tuple(
        parse_test_member(str(item.get("name")), compressed, raw, unit_of["ud" if index == 0 else "technical"])
        for index, ((compressed, raw), item) in enumerate(zip(members_by_split["test"], declared, strict=True)))
    all_rows = [row for member in test_members for row in member.rows]
    observed = {"row_ids_sha256": sorted(row.row_sha256 for row in all_rows),
                "family_ids_sha256": sorted({row.family for row in all_rows}),
                "document_ids_sha256": sorted({row.document_sha256 for row in all_rows})}
    if observed != {field: sorted(values) for field, values in parent_membership.items()} or len(all_rows) != len(set(observed["row_ids_sha256"])):
        raise ValueError("parent test membership differs from its rows")
    retained = [row for row in all_rows if row.unit is None]
    moved = [row for row in all_rows if row.unit is not None]
    membership: dict[str, object] = {
        "namespace": namespace,
        "scope": "parent membership minus whole prefix-exposed units; not newly independent of prior exposure",
        "row_ids_sha256": sorted(row.row_sha256 for row in retained),
        "family_ids_sha256": sorted({row.family for row in retained}),
        "document_ids_sha256": sorted({row.document_sha256 for row in retained}),
    }
    retained_documents = {row.document_sha256 for row in retained}
    moved_documents = {row.document_sha256 for row in moved}
    units: list[dict[str, object]] = []
    for origin_name, matched, index in (("ud", ud_units, 0), ("technical", technical_units, 1)):
        for unit, matched_aliases in sorted(matched.items()):
            rows = [row for row in test_members[index].rows if row.unit == unit]
            units.append({
                "origin": origin_name, "unit_sha256": unit,
                "affected_family_ids_sha256": sorted(affected[origin_name][unit]),
                "matched_aliases_sha256": sorted(matched_aliases),
                "matched_alias_splits": dict(sorted(Counter(split for alias in matched_aliases for split in splits_of[alias]).items())),
                "matched_alias_scopes": dict(sorted(Counter(scope for alias in matched_aliases for scope in scopes_of[alias]).items())),
                "moved_rows": len(rows), "moved_row_ids_sha256": sorted(row.row_sha256 for row in rows),
                "moved_document_ids_sha256": sorted({row.document_sha256 for row in rows}),
                "fully_removed_document_ids_sha256": sorted({row.document_sha256 for row in rows} - retained_documents),
            })
    moved_by_origin = {name: sum(row.unit is not None for row in member.rows) for name, member in zip(("ud", "technical"), test_members, strict=True)}
    summary: dict[str, object] = {
        "namespace": namespace, "reason": REASON,
        "parent_manifest_sha256": pins[pin_name(manifest_path)],
        "parent_test_membership_sha256": pins[pin_name(membership_path)],
        "inventory_sha256": checksum(inventory), "inventory_aliases": len(aliases),
        "units_by_origin": {"ud": len(ud_units), "technical": len(technical_units)},
        "parent_test_rows": len(all_rows), "moved_rows": len(moved), "retained_test_rows": len(retained),
        "moved_rows_by_origin": moved_by_origin,
        "removed_families": len(parent_membership["family_ids_sha256"]) - len(cast(list[str], membership["family_ids_sha256"])),
        "removed_documents": len(parent_membership["document_ids_sha256"]) - len(retained_documents),
        "documents_retained_with_moved_rows": len(moved_documents & retained_documents),
        "rows_by_split": {**{split: int(cast(int, records[split]["rows"])) for split in SPLITS},
                          "test": len(retained), "quarantine": int(cast(int, records["quarantine"]["rows"])) + len(moved)},
        "replacements": 0, "reshuffled": False, "namespace_changed": False, "threshold_tuning": False,
        "already_quarantined_rows_returned": 0, "test_text_printed": False, "model_scoring": False,
        "scope": "blind whole-unit transfer of TEST units whose declared aliases equal actual prefix curriculum inputs; no quality claim",
    }
    sidecar: dict[str, object] = {
        "schema_version": 1, "kind": "prefix-curriculum-prior-exposure-transfer", "reason": REASON,
        "inventory_sha256": summary["inventory_sha256"],
        "inventory_alias_counts": {"total": len(aliases), **{split: sum(split in splits_of[alias] for alias in aliases) for split in CURRICULUM_SPLITS}},
        "parent_manifest_sha256": summary["parent_manifest_sha256"],
        "parent_test_membership_sha256": summary["parent_test_membership_sha256"],
        "units": units,
        "moved_row_ids_sha256": sorted(row.row_sha256 for row in moved),
        "removed_family_ids_sha256": sorted(set(parent_membership["family_ids_sha256"]) - set(cast(list[str], membership["family_ids_sha256"]))),
        "removed_document_ids_sha256": sorted(set(parent_membership["document_ids_sha256"]) - retained_documents),
        "counts": {key: summary[key] for key in ("parent_test_rows", "moved_rows", "retained_test_rows", "moved_rows_by_origin",
                                                  "units_by_origin", "removed_families", "removed_documents",
                                                  "documents_retained_with_moved_rows")},
        "policy": {key: summary[key] for key in ("replacements", "reshuffled", "namespace_changed", "threshold_tuning",
                                                  "already_quarantined_rows_returned", "test_text_printed", "model_scoring")},
        "exposure_statement": "every listed unit was exposed through actual TRAIN/CAL prefix curriculum inputs; its source families are not newly independent",
        "limitations": ["hash-only evidence over declared families; text and labels were not reviewed",
                        "units are moved whole, so the derivative keeps no partial variant of an exposed family",
                        "the derivative inherits every prior exposure recorded by its parents"],
    }
    moved_lines = tuple(relabel(row.line) for row in moved)
    verify_pins(pins, "source changed during reconciliation")
    return Plan(corpus, tuple(origins), manifest, records, test_members, moved_lines, membership, sidecar, summary, pins)


def write_derivative(plan: Plan, output: Path) -> dict[str, object]:
    if output.exists():
        raise ValueError("refusing to overwrite a corpus directory")
    resolved = output.resolve()
    for source in (plan.corpus, *(origin.directory for origin in plan.origins)):
        if resolved == source.resolve() or resolved.is_relative_to(source.resolve()):
            raise ValueError("derivative must not be inside a source corpus")
    verify_pins(plan.pins, "source changed after reconciliation")
    output.mkdir(parents=True)
    (output / "origins").mkdir()
    for item in cast(list[object], plan.manifest["origins"]):
        name = str(object_value(item).get("manifest"))
        shutil.copyfile(plan.corpus / name, output / name)
    shutil.copyfile(plan.corpus / "manifest.json", output / "origins" / "parent-manifest.json")
    generator_name = str(plan.manifest.get("generator_source", "generator-source.py"))
    shutil.copyfile(plan.corpus / generator_name, output / generator_name)
    source = output / "reconciliation-source.py"
    source.write_bytes(Path(__file__).read_bytes())
    files: dict[str, dict[str, object]] = {}
    for split in CURRICULUM_SPLITS:
        destination = output / (split + ".jsonl.gz")
        shutil.copyfile(plan.corpus / destination.name, destination)
        streamed = inspect_gzip(destination, expected_sha256=hash_value(plan.records[split].get("sha256")))
        files[split] = {"path": destination.name, "sha256": streamed.sha256, "content_sha256": streamed.content_sha256,
                        "rows": streamed.rows, "raw_bytes": streamed.raw_bytes, "gzip_members_from": ["base", "technical"],
                        "members_rewritten": []}
    test_path = output / "test.jsonl.gz"
    rewritten = [member.name for member in plan.test_members if member.moved]
    with test_path.open("xb") as stream:
        for member in plan.test_members:
            stream.write(gzip.compress(member.retained_lines, mtime=0) if member.moved else member.compressed)
    retained_raw = b"".join(member.retained_lines for member in plan.test_members)
    streamed = inspect_gzip(test_path)
    if streamed.content_sha256 != hashlib.sha256(retained_raw).hexdigest() or streamed.rows != retained_raw.count(b"\n"):
        raise ValueError("retained test stream differs from plan")
    files["test"] = {"path": test_path.name, "sha256": streamed.sha256, "content_sha256": streamed.content_sha256,
                     "rows": streamed.rows, "raw_bytes": streamed.raw_bytes, "gzip_members_from": ["base", "technical"],
                     "members_rewritten": rewritten}
    quarantine_path = output / "quarantine.jsonl.gz"
    parent_quarantine = plan.corpus / quarantine_path.name
    with quarantine_path.open("xb") as stream:
        with parent_quarantine.open("rb") as incoming:
            shutil.copyfileobj(incoming, stream)
        stream.write(gzip.compress(b"".join(plan.moved_lines), mtime=0))
    streamed = inspect_gzip(quarantine_path)
    expected_rows = int(cast(int, plan.records["quarantine"]["rows"])) + len(plan.moved_lines)
    if streamed.rows != expected_rows:
        raise ValueError("quarantine stream differs from plan")
    files["quarantine"] = {"path": quarantine_path.name, "sha256": streamed.sha256, "content_sha256": streamed.content_sha256,
                           "rows": streamed.rows, "raw_bytes": streamed.raw_bytes,
                           "gzip_members_from": ["base", "technical", REASON], "members_rewritten": []}
    membership_path = output / "test-membership.json"
    membership_path.write_bytes(canonical(plan.membership))
    sidecar_path = output / "prefix-exposure-transfer.json"
    sidecar_path.write_bytes(canonical(plan.sidecar))
    manifest: dict[str, object] = {
        "schema_version": 1, "namespace": plan.manifest["namespace"], "label": "keep", "splits": files,
        "scope": "parent union corpus minus whole TEST units exposed through actual prefix curriculum inputs; not new independent evidence",
        "origins": plan.manifest["origins"],
        "parent_manifest": "origins/parent-manifest.json",
        "parent_manifest_sha256": plan.summary["parent_manifest_sha256"],
        "parent_test_membership_sha256": plan.summary["parent_test_membership_sha256"],
        "generator_source": generator_name, "generator_sha256": plan.manifest["generator_sha256"],
        "generator_role": "unchanged parent merger; derivative generated by reconciliation-source.py",
        "partitioning": {"mode": "parent-test-minus-prefix-exposure-units", "replacements": 0,
                         "rows_by_split": {split: files[split]["rows"] for split in SPLITS}},
        "exposure_reconciliation": {"summary": plan.summary, "sidecar": sidecar_path.name,
                                    "sidecar_sha256": checksum(sidecar_path), "source_sha256": checksum(source),
                                    "inventory_sha256": plan.summary["inventory_sha256"],
                                    "test_policy": "parent test minus whole exposed units; no replacements, reshuffle, threshold tuning or new namespace; test JSON parsed only to relabel split and reason, never printed or scored"},
        "test_membership_sha256": checksum(membership_path),
        "provenance": dict(sorted(plan.pins.items())),
        "compression": "byte-exact parent gzip members where unchanged; rewritten test members and the appended quarantine member use gzip mtime=0",
        "test_policy": "test JSON parsed only to relabel exposed units; retained rows keep exact bytes and order; no scoring",
        "limitations": [
            "hash-only exposure evidence over declared families; labels and text were not reviewed",
            "the derivative inherits every prior exposure of its parents and adds none of its own independence",
            "cryptographic pins provide consistency, not signed provenance or a quality evaluation",
        ],
    }
    (output / "manifest.json").write_bytes(canonical(manifest))
    verify_pins(plan.pins, "source changed while writing derivative")
    for split in CURRICULUM_SPLITS:
        if len(load_split(output, split)) != files[split]["rows"]:
            raise ValueError("derivative split loader disagrees with manifest")
    return manifest


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--inventory", type=Path, required=True)
    parser.add_argument("--ud-origin", type=Path, required=True)
    parser.add_argument("--technical-origin", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--inventory-sha256")
    parser.add_argument("--collisions", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        report: Path = args.report
        sources: Iterable[Path] = (args.corpus, args.ud_origin, args.technical_origin)
        if any(report.resolve().is_relative_to(source.resolve()) for source in sources):
            raise ValueError("report must not overwrite corpus inputs")
        plan = reconcile(args.corpus, args.inventory, args.ud_origin, args.technical_origin,
                         inventory_sha256=args.inventory_sha256, collisions=args.collisions)
        report.parent.mkdir(parents=True, exist_ok=True)
        report.write_bytes(canonical({"summary": plan.summary, "transfer": plan.sidecar}))
        result: dict[str, object] = {"summary": plan.summary, "report_sha256": checksum(report)}
        if args.output is not None:
            manifest = write_derivative(plan, args.output)
            result.update({"manifest_sha256": checksum(args.output / "manifest.json"),
                           "test_membership_sha256": manifest["test_membership_sha256"],
                           "rows_by_split": object_value(manifest["partitioning"])["rows_by_split"]})
        print(json.dumps(result, sort_keys=True), flush=True)
    except PermissionError as error:
        print(json.dumps({"status": "access-denied", "operation": "blind prefix exposure reconciliation", "path": error.filename}), flush=True)
        return 1
    except Exception as error:
        # Never put a test row, identifier or third-party parser text into the observer's output.
        print(json.dumps({"status": "blind-reconciliation-failed", "error_type": type(error).__name__}), flush=True)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
