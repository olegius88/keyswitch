"""Blind transfer of prefix-exposed TEST units into quarantine, on fixtures only."""

from __future__ import annotations

from collections.abc import Callable
from contextlib import redirect_stdout
from dataclasses import asdict, replace
import gzip
import hashlib
import io
import json
from pathlib import Path
import runpy
import shutil
import sys
import tempfile
from typing import cast
import unittest
from unittest.mock import patch
import zlib

TOOLS = str(Path(__file__).resolve().parents[1] / "tools")
if TOOLS not in sys.path:
    sys.path.insert(0, TOOLS)

from freeze_context_action_corpus import CorpusRow, canonical, checksum, digest, load_split
from model_protocol import ALL_SPLITS
from merge_context_action_corpora import Digest, Streamed, inspect_gzip
import reconcile_prefix_exposure_corpus as transfer

REASON = "prefix-curriculum-prior-exposure"
SECRET_WORDS = ("orchard", "nebula", "marigold", "quasar", "saffron", "lantern", "cobalt", "zephyr")


def row(identifier: str, original: str, split: str, family: str, document: str, *, reasons: tuple[str, ...] = (),
        alignment: str = "exact-text-comment") -> CorpusRow:
    return CorpusRow(identifier=identifier, original=original, group=0, before="", after="", lemma=original,
                     family=family, document=document, language="en", source="fixture", source_file="fixture",
                     source_sentence="1", source_token="1", spacing=" ", space_before="", literal_tail="", upos="X",
                     features="_", misc="_", alignment=alignment, layout_representable=True, split=split,
                     quarantine_reasons=reasons)


def member(rows: list[CorpusRow]) -> bytes:
    return gzip.compress(b"".join(canonical(asdict(item)) for item in rows), mtime=0)


def members(path: Path) -> list[bytes]:
    data = path.read_bytes()
    result = []
    position = 0
    while position < len(data):
        decoder = zlib.decompressobj(16 + zlib.MAX_WBITS)
        decoder.decompress(data[position:])
        used = len(data) - position - len(decoder.unused_data)
        result.append(data[position:position + used])
        position += used
    return result


class Fixture:
    """A tiny combined corpus with UD-like and technical origins plus exposure inventory."""

    def __init__(self, root: Path) -> None:
        self.root = root
        self.ud = root / "ud"
        self.technical = root / "technical"
        self.parent = root / "parent"
        self.inventory = root / "inventory.json"
        self.collisions = root / "collisions.json"
        self.snapshot = root / "prefix_snapshot.py"
        self.snapshot.write_bytes(b"# audited prefix generator snapshot\n")
        self.closure_one = digest("ud:closure:1")
        self.closure_two = digest("ud:closure:2")
        self.closure_three = digest("ud:closure:3")
        self.old_family_one = digest("ud:old-family:1")
        self.old_family_one_b = digest("ud:old-family:1b")
        self.old_family_two = digest("ud:old-family:2")
        self.old_family_three = digest("ud:old-family:3")
        self.family_one = digest("technical:family:1")
        self.family_two = digest("technical:family:2")
        self.family_three = digest("technical:family:3")
        self.aliases = {name: digest("alias:" + name) for name in ("A1", "A2", "A3", "B1", "B2", "B3", "B4", "T")}
        self.ud_rows = {
            "train": [row("ud:train", "orchard", "train", digest("ud:closure:train"), "ud:doc:train")],
            "development": [row("ud:dev", "orchard", "development", digest("ud:closure:dev"), "ud:doc:dev")],
            "calibration": [row("ud:cal", "orchard", "calibration", digest("ud:closure:cal"), "ud:doc:cal")],
            "test": [row("ud:test:1", "nebula", "test", self.old_family_one, "ud:doc:1"),
                     row("ud:test:1b", "marigold", "test", self.old_family_one_b, "ud:doc:1b"),
                     row("ud:test:2", "quasar", "test", self.old_family_two, "ud:doc:2"),
                     row("ud:test:3", "saffron", "test", self.closure_three, "ud:doc:3")],
            "quarantine": [row("ud:quarantine", "nebula", "quarantine", self.old_family_one, "ud:doc:q",
                               reasons=("family-previously-exposed",))],
        }
        self.technical_rows = {
            "train": [row("tech:train", "lantern", "train", digest("technical:family:train"), "tech:doc:train",
                          alignment="declared-command-name")],
            "development": [row("tech:dev", "lantern", "development", digest("technical:family:dev"), "tech:doc:dev",
                                alignment="declared-command-name")],
            "calibration": [row("tech:cal", "lantern", "calibration", digest("technical:family:cal"), "tech:doc:cal",
                                alignment="declared-command-name")],
            "test": [row("tech:test:1", "cobalt", "test", self.family_one, "tech:doc:1a", alignment="declared-command-name"),
                     row("tech:test:2", "cobalt", "test", self.family_one, "tech:doc:1b", alignment="declared-command-name"),
                     row("tech:test:3", "zephyr", "test", self.family_two, "tech:doc:shared", alignment="declared-command-name"),
                     row("tech:test:4", "zephyr", "test", self.family_three, "tech:doc:shared", alignment="declared-command-name")],
            "quarantine": [row("tech:quarantine", "lantern", "quarantine", digest("technical:family:q"), "tech:doc:q",
                               reasons=("fixed-hash-family-cap",), alignment="declared-command-name")],
        }
        self.write_origin(self.ud, "ud", self.ud_rows, self.ud_sidecar())
        self.write_origin(self.technical, "technical", self.technical_rows, self.technical_sidecar())
        self.write_parent()
        self.write_inventory()
        self.collisions.write_bytes(canonical({"families": [
            {"target_origin": "ud", "target_family_closure_sha256": self.closure_one},
            {"target_origin": "ud", "target_family_closure_sha256": self.closure_three},
            {"target_origin": "technical", "target_family_closure_sha256": self.family_one},
            {"target_origin": "technical", "target_family_closure_sha256": self.family_three},
        ]}))

    def ud_sidecar(self) -> tuple[str, dict[str, object]]:
        return "physical-family-closure.json", {
            "schema_version": 1,
            "alias_sha256_to_closure_sha256": {
                self.aliases["A1"]: self.closure_one, self.aliases["A2"]: self.closure_two,
                self.aliases["A3"]: self.closure_three, self.aliases["T"]: digest("ud:closure:train"),
            },
            "components": {self.closure_one: [self.old_family_one, self.old_family_one_b],
                           self.closure_two: [self.old_family_two],
                           self.closure_three: [self.old_family_three],
                           digest("ud:closure:train"): [digest("ud:closure:train")]},
            "retained_test_closure_sha256": [self.closure_one, self.closure_two, self.closure_three],
        }

    def technical_sidecar(self) -> tuple[str, dict[str, object]]:
        return "family-aliases.json", {"schema_version": 1, "families": {
            self.family_one: {"split": "test", "aliases_sha256": [self.aliases["B1"], self.aliases["B2"]],
                              "document_ids_sha256": [digest("tech:doc:1a"), digest("tech:doc:1b")]},
            self.family_two: {"split": "test", "aliases_sha256": [self.aliases["B3"]],
                              "document_ids_sha256": [digest("tech:doc:shared")]},
            self.family_three: {"split": "test", "aliases_sha256": [self.aliases["B4"]],
                                "document_ids_sha256": [digest("tech:doc:shared")]},
            digest("technical:family:train"): {"split": "train", "aliases_sha256": [digest("alias:train")],
                                               "document_ids_sha256": [digest("tech:doc:train")]},
        }}

    @staticmethod
    def membership(rows: dict[str, list[CorpusRow]], namespace: str) -> dict[str, object]:
        held = rows["test"]
        return {"namespace": namespace, "scope": "fixture", "row_ids_sha256": sorted(digest(item.identifier) for item in held),
                "family_ids_sha256": sorted({item.family for item in held}),
                "document_ids_sha256": sorted({digest(item.document) for item in held})}

    def write_origin(self, directory: Path, name: str, rows: dict[str, list[CorpusRow]], sidecar: tuple[str, dict[str, object]]) -> None:
        directory.mkdir()
        files = {}
        for split in ALL_SPLITS:
            path = directory / (split + ".jsonl.gz")
            path.write_bytes(member(rows[split]))
            raw = b"".join(canonical(asdict(item)) for item in rows[split])
            files[split] = {"path": path.name, "sha256": checksum(path), "content_sha256": hashlib.sha256(raw).hexdigest(),
                            "rows": len(rows[split])}
        generator = directory / "generator-source.py"
        generator.write_bytes(("# " + name + " generator\n").encode())
        membership_path = directory / "test-membership.json"
        membership_path.write_bytes(canonical(self.membership(rows, name)))
        sidecar_path = directory / sidecar[0]
        sidecar_path.write_bytes(canonical(sidecar[1]))
        manifest: dict[str, object] = {"schema_version": 1, "namespace": name, "label": "keep", "splits": files,
                                       "generator_source": generator.name, "generator_sha256": checksum(generator),
                                       "test_membership_sha256": checksum(membership_path)}
        if name == "ud":
            manifest["reconciliation"] = {"closure_sha256": checksum(sidecar_path)}
        else:
            manifest["family_aliases_sha256"] = checksum(sidecar_path)
        (directory / "manifest.json").write_bytes(canonical(manifest))

    def write_parent(self) -> None:
        self.parent.mkdir()
        (self.parent / "origins").mkdir()
        origins = []
        pins: dict[str, str] = {}
        for name, directory in (("base", self.ud), ("technical", self.technical)):
            saved = self.parent / "origins" / (name + "-manifest.json")
            shutil.copyfile(directory / "manifest.json", saved)
            manifest = json.loads(saved.read_bytes())
            origins.append({"name": name, "namespace": manifest["namespace"], "manifest": "origins/" + saved.name,
                            "manifest_sha256": checksum(saved), "generator_sha256": manifest["generator_sha256"],
                            "test_membership_sha256": manifest["test_membership_sha256"], "splits": manifest["splits"]})
            for item in directory.iterdir():
                pins[str(item)] = checksum(item)
        files = {}
        for split in ALL_SPLITS:
            path = self.parent / (split + ".jsonl.gz")
            ud_bytes = (self.ud / path.name).read_bytes()
            technical_bytes = (self.technical / path.name).read_bytes()
            path.write_bytes(ud_bytes + technical_bytes)
            raw = gzip.decompress(ud_bytes) + gzip.decompress(technical_bytes)
            files[split] = {"path": path.name, "sha256": checksum(path), "content_sha256": hashlib.sha256(raw).hexdigest(),
                            "rows": raw.count(b"\n"), "raw_bytes": len(raw), "gzip_members_from": ["base", "technical"]}
        generator = self.parent / "generator-source.py"
        generator.write_bytes(b"# fixture merger\n")
        ud_membership = self.membership(self.ud_rows, "ud")
        technical_membership = self.membership(self.technical_rows, "technical")
        union: dict[str, object] = {"namespace": "ud", "scope": "fixture union"}
        for field in ("row_ids_sha256", "family_ids_sha256", "document_ids_sha256"):
            union[field] = sorted(cast(list[str], ud_membership[field]) + cast(list[str], technical_membership[field]))
        membership_path = self.parent / "test-membership.json"
        membership_path.write_bytes(canonical(union))
        manifest = {"schema_version": 1, "namespace": "ud", "label": "keep", "splits": files, "origins": origins,
                    "generator_source": generator.name, "generator_sha256": checksum(generator),
                    "test_membership_sha256": checksum(membership_path), "provenance": pins,
                    "partitioning": {"mode": "unchanged-source-split-concatenation", "replacements": 0}}
        (self.parent / "manifest.json").write_bytes(canonical(manifest))

    def write_inventory(self, extra: dict[str, object] | None = None) -> None:
        by_split = {"train": [self.aliases["A1"], self.aliases["A3"], self.aliases["B4"], digest("alias:unrelated")],
                    "development": [digest("alias:unrelated")],
                    "calibration": [self.aliases["B2"], digest("alias:unrelated")]}
        union = sorted(set().union(*(set(values) for values in by_split.values())))
        inventory: dict[str, object] = {
            "schema_version": 1, "kind": "actual-prefix-model-input-exposure", "aliases_sha256": union,
            "aliases_by_split": {split: sorted(values) for split, values in by_split.items()},
            "aliases_by_scope": {"model-input-original": union, "model-input-alternate": union},
            "provenance": {str(self.snapshot): checksum(self.snapshot)}, "test_json_opened": False, "model_scoring": False,
        }
        inventory.update(extra or {})
        self.inventory.write_bytes(canonical(inventory))

    def repin(self, directory: Path, sidecar: str, *field: str) -> None:
        path = directory / "manifest.json"
        manifest = cast(dict[str, object], json.loads(path.read_bytes()))
        target = manifest
        for key in field[:-1]:
            target = cast(dict[str, object], target[key])
        target[field[-1]] = checksum(directory / sidecar)
        path.write_bytes(canonical(manifest))

    def repin_parent_provenance(self, path: Path) -> None:
        manifest_path = self.parent / "manifest.json"
        manifest = cast(dict[str, object], json.loads(manifest_path.read_bytes()))
        cast(dict[str, object], manifest["provenance"])[str(path)] = checksum(path)
        manifest_path.write_bytes(canonical(manifest))

    def edit_parent_manifest(self, mutate: Callable[[dict[str, object]], object]) -> None:
        manifest_path = self.parent / "manifest.json"
        manifest = cast(dict[str, object], json.loads(manifest_path.read_bytes()))
        mutate(manifest)
        manifest_path.write_bytes(canonical(manifest))

    def repin_parent_split(self, split: str) -> None:
        path = self.parent / (split + ".jsonl.gz")
        raw = b"".join(gzip.decompress(item) for item in members(path))

        def mutate(manifest: dict[str, object]) -> None:
            record = cast(dict[str, dict[str, object]], manifest["splits"])[split]
            record.update(sha256=checksum(path), content_sha256=hashlib.sha256(raw).hexdigest(), rows=raw.count(b"\n"))

        self.edit_parent_manifest(mutate)

    def rewrite_sidecar(self, directory: Path, name: str, mutate: Callable[[dict[str, object]], object], *field: str) -> None:
        path = directory / name
        data = cast(dict[str, object], json.loads(path.read_bytes()))
        mutate(data)
        path.write_bytes(canonical(data))
        self.repin(directory, name, *field)
        self.rebuild_parent()

    def rebuild_parent(self) -> None:
        shutil.rmtree(self.parent)
        self.write_parent()

    def reset(self) -> None:
        for directory in (self.ud, self.technical, self.parent):
            shutil.rmtree(directory)
        self.write_origin(self.ud, "ud", self.ud_rows, self.ud_sidecar())
        self.write_origin(self.technical, "technical", self.technical_rows, self.technical_sidecar())
        self.write_parent()
        self.write_inventory()

    def rewrite_ud_test(self, lines: list[bytes], rows: list[CorpusRow]) -> None:
        """Replace the UD TEST member with raw lines; membership follows the given rows."""
        path = self.ud / "test.jsonl.gz"
        raw = b"".join(lines)
        path.write_bytes(gzip.compress(raw, mtime=0))
        manifest_path = self.ud / "manifest.json"
        manifest = cast(dict[str, object], json.loads(manifest_path.read_bytes()))
        splits = cast(dict[str, dict[str, object]], manifest["splits"])
        splits["test"].update(sha256=checksum(path), content_sha256=hashlib.sha256(raw).hexdigest(), rows=len(lines))
        membership_path = self.ud / "test-membership.json"
        membership_path.write_bytes(canonical(self.membership({"test": rows}, "ud")))
        manifest["test_membership_sha256"] = checksum(membership_path)
        manifest_path.write_bytes(canonical(manifest))
        self.ud_rows = {**self.ud_rows, "test": rows}
        self.rebuild_parent()

    def snapshot_bytes(self, *directories: Path) -> dict[str, bytes]:
        return {str(path): path.read_bytes() for directory in directories for path in sorted(directory.rglob("*")) if path.is_file()}

    def plan(self, *, inventory_sha256: str | None = "", collisions: Path | None = None,
             cross_check: bool = True) -> transfer.Plan:
        expected = checksum(self.inventory) if inventory_sha256 == "" else inventory_sha256
        if collisions is None and cross_check:
            collisions = self.collisions
        return transfer.reconcile(self.parent, self.inventory, self.ud, self.technical,
                                  inventory_sha256=expected, collisions=collisions)


class PrefixExposureReconciliationTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.fixture = Fixture(Path(temporary.name))
        self.output = self.fixture.root / "derivative"

    def test_units_are_re_derived_from_inventory_and_whole_families_are_moved(self) -> None:
        plan = self.fixture.plan()
        summary = plan.summary
        self.assertEqual(summary["units_by_origin"], {"ud": 2, "technical": 2})
        self.assertEqual(summary["moved_rows_by_origin"], {"ud": 3, "technical": 3})
        self.assertEqual(summary["parent_test_rows"], 8)
        self.assertEqual(summary["retained_test_rows"], 2)
        self.assertEqual(summary["replacements"], 0)
        moved = {digest(name) for name in ("ud:test:1", "ud:test:1b", "ud:test:3", "tech:test:1", "tech:test:2", "tech:test:4")}
        self.assertEqual(set(cast(list[str], plan.sidecar["moved_row_ids_sha256"])), moved)
        units = {(item["origin"], item["unit_sha256"]) for item in cast(list[dict[str, object]], plan.sidecar["units"])}
        self.assertEqual(units, {("ud", self.fixture.closure_one), ("ud", self.fixture.closure_three),
                                 ("technical", self.fixture.family_one), ("technical", self.fixture.family_three)})

    def test_ud_rows_match_through_old_family_ids_and_through_closure_ids(self) -> None:
        plan = self.fixture.plan()
        by_unit = {str(item["unit_sha256"]): item for item in cast(list[dict[str, object]], plan.sidecar["units"])}
        one = by_unit[self.fixture.closure_one]
        self.assertEqual(set(cast(list[str], one["affected_family_ids_sha256"])),
                         {self.fixture.closure_one, self.fixture.old_family_one, self.fixture.old_family_one_b})
        self.assertEqual(set(cast(list[str], one["moved_row_ids_sha256"])), {digest("ud:test:1"), digest("ud:test:1b")})
        three = by_unit[self.fixture.closure_three]
        self.assertEqual(set(cast(list[str], three["moved_row_ids_sha256"])), {digest("ud:test:3")})
        self.assertEqual(one["matched_alias_splits"], {"train": 1})

    def test_membership_drops_moved_rows_families_and_only_fully_removed_documents(self) -> None:
        plan = self.fixture.plan()
        membership = plan.membership
        self.assertEqual(membership["row_ids_sha256"], sorted([digest("ud:test:2"), digest("tech:test:3")]))
        self.assertEqual(membership["family_ids_sha256"], sorted([self.fixture.old_family_two, self.fixture.family_two]))
        self.assertEqual(membership["document_ids_sha256"], sorted([digest("ud:doc:2"), digest("tech:doc:shared")]))
        self.assertEqual(set(cast(list[str], plan.sidecar["removed_document_ids_sha256"])),
                         {digest(name) for name in ("ud:doc:1", "ud:doc:1b", "ud:doc:3", "tech:doc:1a", "tech:doc:1b")})
        self.assertEqual(plan.summary["documents_retained_with_moved_rows"], 1)

    def test_derivative_keeps_other_splits_and_retained_test_bytes_and_appends_quarantine(self) -> None:
        before = self.fixture.snapshot_bytes(self.fixture.parent, self.fixture.ud, self.fixture.technical)
        plan = self.fixture.plan()
        manifest = transfer.write_derivative(plan, self.output)
        self.assertEqual(self.fixture.snapshot_bytes(self.fixture.parent, self.fixture.ud, self.fixture.technical), before)
        for split in ("train", "development", "calibration"):
            self.assertEqual((self.output / (split + ".jsonl.gz")).read_bytes(), before[str(self.fixture.parent / (split + ".jsonl.gz"))])
        parent_quarantine = before[str(self.fixture.parent / "quarantine.jsonl.gz")]
        derived_quarantine = (self.output / "quarantine.jsonl.gz").read_bytes()
        self.assertTrue(derived_quarantine.startswith(parent_quarantine))
        self.assertEqual(len(members(self.output / "quarantine.jsonl.gz")), 3)
        retained = [canonical(asdict(item)) for item in self.fixture.ud_rows["test"] if item.identifier == "ud:test:2"]
        retained += [canonical(asdict(item)) for item in self.fixture.technical_rows["test"] if item.identifier == "tech:test:3"]
        with gzip.open(self.output / "test.jsonl.gz", "rb") as stream:
            self.assertEqual(stream.read(), b"".join(retained))
        rows = load_split(self.output, "quarantine")
        moved = [item for item in rows if REASON in item.quarantine_reasons]
        self.assertEqual([item.identifier for item in moved], ["ud:test:1", "ud:test:1b", "ud:test:3", "tech:test:1", "tech:test:2", "tech:test:4"])
        self.assertTrue(all(item.split == "quarantine" for item in moved))
        original = {item.identifier: item for item in self.fixture.ud_rows["test"] + self.fixture.technical_rows["test"]}
        for item in moved:
            expected = asdict(original[item.identifier])
            expected.update(split="quarantine", quarantine_reasons=(REASON,))
            self.assertEqual(asdict(item), expected)
        self.assertEqual(sum(item.identifier == "ud:quarantine" for item in rows), 1)
        self.assertEqual([item.identifier for item in load_split(self.output, "test")], ["ud:test:2", "tech:test:3"])
        for split in ("train", "development", "calibration"):
            self.assertEqual(len(load_split(self.output, split)), 2)
        self.assertEqual(manifest["namespace"], "ud")
        splits = cast(dict[str, dict[str, object]], manifest["splits"])
        self.assertEqual(splits["test"]["rows"], 2)
        self.assertEqual(splits["quarantine"]["rows"], 8)
        self.assertEqual(splits["quarantine"]["gzip_members_from"], ["base", "technical", REASON])
        self.assertEqual(cast(dict[str, object], manifest["exposure_reconciliation"])["summary"], plan.summary)
        self.assertEqual(manifest["test_membership_sha256"], checksum(self.output / "test-membership.json"))
        self.assertEqual(json.loads((self.output / "test-membership.json").read_bytes()), plan.membership)
        self.assertEqual(json.loads((self.output / "prefix-exposure-transfer.json").read_bytes()), plan.sidecar)
        self.assertEqual((self.output / "reconciliation-source.py").read_bytes(), Path(transfer.__file__).read_bytes())
        self.assertEqual((self.output / "origins" / "base-manifest.json").read_bytes(), (self.fixture.ud / "manifest.json").read_bytes())

    def test_unchanged_gzip_member_is_copied_byte_exact(self) -> None:
        # Drop the technical collisions so the technical TEST member is untouched.
        self.fixture.write_inventory({"aliases_by_split": {"train": [self.fixture.aliases["A1"], self.fixture.aliases["A3"]]},
                                      "aliases_sha256": sorted([self.fixture.aliases["A1"], self.fixture.aliases["A3"]]),
                                      "aliases_by_scope": {"model-input-original": sorted([self.fixture.aliases["A1"], self.fixture.aliases["A3"]])}})
        collisions = self.fixture.root / "ud-only.json"
        collisions.write_bytes(canonical({"families": [
            {"target_origin": "ud", "target_family_closure_sha256": self.fixture.closure_one},
            {"target_origin": "ud", "target_family_closure_sha256": self.fixture.closure_three}]}))
        plan = self.fixture.plan(collisions=collisions)
        manifest = transfer.write_derivative(plan, self.output)
        technical_member = members(self.fixture.parent / "test.jsonl.gz")[1]
        self.assertEqual(members(self.output / "test.jsonl.gz")[1], technical_member)
        self.assertEqual(cast(dict[str, dict[str, object]], manifest["splits"])["test"]["members_rewritten"], ["base"])
        self.assertEqual(plan.summary["units_by_origin"], {"ud": 2, "technical": 0})

    def test_fake_disjoint_flags_do_not_hide_actual_alias_overlap(self) -> None:
        self.fixture.write_inventory({"disjoint": True, "collisions": []})
        path = self.fixture.technical / "family-aliases.json"
        data = json.loads(path.read_bytes())
        data["disjoint"] = True
        path.write_bytes(canonical(data))
        self.fixture.repin(self.fixture.technical, path.name, "family_aliases_sha256")
        self.fixture.rebuild_parent()
        plan = self.fixture.plan()
        self.assertEqual(plan.summary["units_by_origin"], {"ud": 2, "technical": 2})

    def test_collision_cross_check_must_match_re_derived_units(self) -> None:
        stale = self.fixture.root / "stale.json"
        stale.write_bytes(canonical({"families": [{"target_origin": "ud", "target_family_closure_sha256": self.fixture.closure_one}]}))
        with self.assertRaisesRegex(ValueError, "collision"):
            self.fixture.plan(collisions=stale)

    def test_tampered_inventory_sidecar_or_source_is_rejected_before_output_exists(self) -> None:
        with self.assertRaisesRegex(ValueError, "inventory checksum"):
            self.fixture.plan(inventory_sha256="0" * 64)
        self.fixture.write_inventory({"aliases_sha256": [digest("alias:unrelated")]})
        with self.assertRaisesRegex(ValueError, "inventory alias"):
            self.fixture.plan()
        self.fixture.write_inventory()
        self.fixture.snapshot.write_bytes(b"# changed\n")
        with self.assertRaisesRegex(ValueError, "inventory input"):
            self.fixture.plan()
        self.fixture.snapshot.write_bytes(b"# audited prefix generator snapshot\n")
        sidecar = self.fixture.ud / "physical-family-closure.json"
        data = json.loads(sidecar.read_bytes())
        data["retained_test_closure_sha256"] = [self.fixture.closure_two]
        sidecar.write_bytes(canonical(data))
        with self.assertRaisesRegex(ValueError, "provenance"):
            self.fixture.plan()
        self.fixture.repin_parent_provenance(sidecar)
        with self.assertRaisesRegex(ValueError, "closure sidecar"):
            self.fixture.plan()
        self.fixture.repin(self.fixture.ud, sidecar.name, "reconciliation", "closure_sha256")
        self.fixture.repin_parent_provenance(self.fixture.ud / "manifest.json")
        with self.assertRaisesRegex(ValueError, "origin manifest"):
            self.fixture.plan()
        self.fixture.reset()
        families = self.fixture.technical / "family-aliases.json"
        families.write_bytes(canonical(json.loads(families.read_bytes()) | {"note": 1}))
        self.fixture.repin_parent_provenance(families)
        with self.assertRaisesRegex(ValueError, "technical family sidecar"):
            self.fixture.plan()
        self.fixture.reset()
        (self.fixture.parent / "test.jsonl.gz").write_bytes(b"damaged")
        with self.assertRaisesRegex(ValueError, "compressed"):
            self.fixture.plan()
        self.assertFalse(self.output.exists())

    def test_non_canonical_or_pre_quarantined_test_rows_are_rejected(self) -> None:
        rows = list(self.fixture.ud_rows["test"])
        flagged = replace(rows[0], quarantine_reasons=("already",))
        self.fixture.rewrite_ud_test([canonical(asdict(item)) for item in (flagged, *rows[1:])], [flagged, *rows[1:]])
        with self.assertRaisesRegex(ValueError, "test row"):
            self.fixture.plan()
        loose = json.dumps(asdict(rows[1]), ensure_ascii=False).encode() + b"\n"
        self.assertNotEqual(loose, canonical(asdict(rows[1])))
        self.fixture.rewrite_ud_test([canonical(asdict(rows[0])), loose, *(canonical(asdict(item)) for item in rows[2:])], rows)
        with self.assertRaisesRegex(ValueError, "canonical"):
            self.fixture.plan()

    def test_parent_membership_must_match_rows(self) -> None:
        path = self.fixture.parent / "test-membership.json"
        data = json.loads(path.read_bytes())
        data["family_ids_sha256"].append(digest("phantom"))
        path.write_bytes(canonical(data))
        self.fixture.repin(self.fixture.parent, path.name, "test_membership_sha256")
        with self.assertRaisesRegex(ValueError, "membership"):
            self.fixture.plan()

    def test_destination_immutability_and_repeatability(self) -> None:
        plan = self.fixture.plan()
        self.output.mkdir()
        sentinel = self.output / "keep"
        sentinel.write_bytes(b"x")
        with self.assertRaisesRegex(ValueError, "overwrite"):
            transfer.write_derivative(plan, self.output)
        self.assertEqual(sentinel.read_bytes(), b"x")
        with self.assertRaisesRegex(ValueError, "inside"):
            transfer.write_derivative(plan, self.fixture.parent / "nested")
        first, second = self.fixture.root / "first", self.fixture.root / "second"
        transfer.write_derivative(plan, first)
        transfer.write_derivative(self.fixture.plan(), second)
        for path in sorted(first.rglob("*")):
            if path.is_file():
                self.assertEqual(path.read_bytes(), (second / path.relative_to(first)).read_bytes(), str(path.name))
        (self.fixture.parent / "train.jsonl.gz").write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "changed"):
            transfer.write_derivative(plan, self.fixture.root / "third")

    def test_command_line_prints_only_hashes_and_counts_and_hides_failures(self) -> None:
        report = self.fixture.root / "preview.json"
        arguments = ["--corpus", str(self.fixture.parent), "--inventory", str(self.fixture.inventory),
                     "--ud-origin", str(self.fixture.ud), "--technical-origin", str(self.fixture.technical),
                     "--inventory-sha256", checksum(self.fixture.inventory), "--collisions", str(self.fixture.collisions),
                     "--report", str(report)]
        output = io.StringIO()
        with redirect_stdout(output):
            self.assertEqual(transfer.main(arguments), 0)
        printed = output.getvalue()
        self.assertEqual(json.loads(printed)["summary"]["moved_rows"], 6)
        self.assertFalse(self.output.exists())
        preview = json.loads(report.read_bytes())
        self.assertEqual(preview["summary"]["retained_test_rows"], 2)
        blob = printed + report.read_text(encoding="utf-8") + canonical(preview).decode()
        for word in SECRET_WORDS:
            self.assertNotIn(word, blob)
        self.assertNotIn("ud:test", blob)
        output = io.StringIO()
        with redirect_stdout(output):
            self.assertEqual(transfer.main([*arguments, "--output", str(self.output)]), 0)
        self.assertTrue((self.output / "manifest.json").exists())
        self.assertNotIn("nebula", output.getvalue())
        for word in SECRET_WORDS:
            self.assertNotIn(word, (self.output / "manifest.json").read_text(encoding="utf-8"))
            self.assertNotIn(word, (self.output / "prefix-exposure-transfer.json").read_text(encoding="utf-8"))
        output = io.StringIO()
        with patch.object(transfer, "reconcile", side_effect=ValueError("PRIVATE_TEST_TEXT")), redirect_stdout(output):
            self.assertEqual(transfer.main(arguments), 1)
        self.assertEqual(json.loads(output.getvalue())["status"], "blind-reconciliation-failed")
        self.assertNotIn("PRIVATE_TEST_TEXT", output.getvalue())
        output = io.StringIO()
        with patch.object(transfer, "reconcile", side_effect=PermissionError(13, "denied", "/fixture")), redirect_stdout(output):
            self.assertEqual(transfer.main(arguments), 1)
        self.assertEqual(json.loads(output.getvalue())["status"], "access-denied")
        with redirect_stdout(io.StringIO()):
            self.assertEqual(transfer.main([*arguments[:-1], str(self.fixture.parent / "manifest.json")]), 1)


class PrefixExposureGuardTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.fixture = Fixture(Path(temporary.name))
        self.output = self.fixture.root / "derivative"

    def test_gzip_member_and_line_structure_are_fail_closed(self) -> None:
        damaged = self.fixture.root / "damaged.gz"
        damaged.write_bytes(b"damaged")
        with self.assertRaisesRegex(ValueError, "invalid gzip member"):
            transfer.gzip_members(damaged)
        truncated = self.fixture.root / "truncated.gz"
        truncated.write_bytes(gzip.compress(b"a\n" * 64, mtime=0)[:-6])
        with self.assertRaisesRegex(ValueError, "truncated gzip member"):
            transfer.gzip_members(truncated)
        with self.assertRaisesRegex(ValueError, "newline"):
            transfer.raw_lines(b"a\nb")
        with self.assertRaisesRegex(ValueError, "empty line"):
            transfer.raw_lines(b"a\n\nb\n")
        self.assertEqual(transfer.raw_lines(b""), ())

    def test_inventory_identity_attestation_and_split_names_are_required(self) -> None:
        self.fixture.write_inventory({"schema_version": 2})
        with self.assertRaisesRegex(ValueError, "inventory identity"):
            self.fixture.plan()
        self.fixture.write_inventory({"test_json_opened": True})
        with self.assertRaisesRegex(ValueError, "attest"):
            self.fixture.plan()
        self.fixture.write_inventory({"aliases_by_split": {"test": [self.fixture.aliases["A1"]]}})
        with self.assertRaisesRegex(ValueError, "curriculum splits"):
            self.fixture.plan()

    def test_test_rows_need_identity_fields_and_object_shape(self) -> None:
        with self.assertRaisesRegex(ValueError, "canonical"):
            transfer.parse_test_member("x", b"", canonical([]), {})
        line = canonical({"family": 1, "identifier": "a", "document": "b", "split": "test", "quarantine_reasons": []})
        with self.assertRaisesRegex(ValueError, "identity fields"):
            transfer.parse_test_member("x", b"", line, {})

    def test_parent_manifest_identity_splits_origins_and_archives_are_checked(self) -> None:
        self.fixture.edit_parent_manifest(lambda manifest: manifest.update(label="candidate"))
        with self.assertRaisesRegex(ValueError, "manifest identity"):
            self.fixture.plan()
        self.fixture.rebuild_parent()
        self.fixture.edit_parent_manifest(lambda manifest: cast(dict[str, object], manifest["splits"]).pop("quarantine"))
        with self.assertRaisesRegex(ValueError, "all corpus splits"):
            self.fixture.plan()
        self.fixture.rebuild_parent()
        self.fixture.edit_parent_manifest(lambda manifest: cast(list[dict[str, object]], manifest["origins"])[0].update(name="ud"))
        with self.assertRaisesRegex(ValueError, "base and technical"):
            self.fixture.plan()
        self.fixture.rebuild_parent()
        self.fixture.edit_parent_manifest(lambda manifest: cast(list[dict[str, object]], manifest["origins"])[0].update(manifest="../outside.json"))
        with self.assertRaisesRegex(ValueError, "inside the parent"):
            self.fixture.plan()
        self.fixture.rebuild_parent()
        self.fixture.edit_parent_manifest(
            lambda manifest: cast(dict[str, dict[str, object]], cast(list[dict[str, object]], manifest["origins"])[0]["splits"])["train"].update(rows=9))
        with self.assertRaisesRegex(ValueError, "split records differ"):
            self.fixture.plan()
        self.fixture.rebuild_parent()
        (self.fixture.parent / "test-membership.json").write_bytes(b"{}\n")
        with self.assertRaisesRegex(ValueError, "test membership checksum"):
            self.fixture.plan()
        self.fixture.rebuild_parent()
        (self.fixture.parent / "generator-source.py").write_bytes(b"# changed\n")
        with self.assertRaisesRegex(ValueError, "generator checksum"):
            self.fixture.plan()
        self.fixture.rebuild_parent()
        self.fixture.edit_parent_manifest(
            lambda manifest: cast(dict[str, dict[str, object]], manifest["splits"])["train"].update(content_sha256="0" * 64))
        with self.assertRaisesRegex(ValueError, "content differs"):
            self.fixture.plan()

    def test_parent_gzip_members_must_be_the_origin_members(self) -> None:
        path = self.fixture.parent / "train.jsonl.gz"
        path.write_bytes(path.read_bytes() + gzip.compress(b"", mtime=0))
        self.fixture.repin_parent_split("train")
        with self.assertRaisesRegex(ValueError, "one gzip member per origin"):
            self.fixture.plan()
        self.fixture.rebuild_parent()
        ud_raw = gzip.decompress(members(path)[0])
        path.write_bytes(gzip.compress(ud_raw, mtime=1) + members(path)[1])
        self.fixture.repin_parent_split("train")
        with self.assertRaisesRegex(ValueError, "member differs from its origin"):
            self.fixture.plan()

    def test_unit_structure_conflicts_are_rejected(self) -> None:
        closure = self.fixture.closure_one
        self.fixture.rewrite_sidecar(self.fixture.ud, "physical-family-closure.json",
                                     lambda data: cast(dict[str, object], data["components"]).pop(closure),
                                     "reconciliation", "closure_sha256")
        with self.assertRaisesRegex(ValueError, "no component record"):
            self.fixture.plan(cross_check=False)
        self.fixture.reset()
        self.fixture.rewrite_sidecar(self.fixture.ud, "physical-family-closure.json",
                                     lambda data: cast(dict[str, list[str]], data["components"])[self.fixture.closure_three].append(self.fixture.old_family_one),
                                     "reconciliation", "closure_sha256")
        with self.assertRaisesRegex(ValueError, "two matched units"):
            self.fixture.plan(cross_check=False)
        self.fixture.reset()
        record = {"split": "test", "aliases_sha256": [self.fixture.aliases["B4"]], "document_ids_sha256": [digest("tech:doc:x")]}
        self.fixture.rewrite_sidecar(self.fixture.technical, "family-aliases.json",
                                     lambda data: cast(dict[str, object], data["families"]).update({self.fixture.old_family_one: record}),
                                     "family_aliases_sha256")
        with self.assertRaisesRegex(ValueError, "identifiers overlap"):
            self.fixture.plan(cross_check=False)

    def test_write_self_checks_detect_stream_and_loader_disagreement(self) -> None:
        def wrong(name: str) -> Callable[[Path], Streamed]:
            def fake(path: Path, raw_digest: Digest | None = None, *, expected_sha256: str | None = None) -> Streamed:
                streamed = inspect_gzip(path, raw_digest, expected_sha256=expected_sha256)
                return replace(streamed, rows=streamed.rows + 1) if path.name == name else streamed
            return fake

        plan = self.fixture.plan()
        with patch.object(transfer, "inspect_gzip", wrong("test.jsonl.gz")), self.assertRaisesRegex(ValueError, "retained test stream"):
            transfer.write_derivative(plan, self.fixture.root / "one")
        with patch.object(transfer, "inspect_gzip", wrong("quarantine.jsonl.gz")), self.assertRaisesRegex(ValueError, "quarantine stream"):
            transfer.write_derivative(plan, self.fixture.root / "two")
        with patch.object(transfer, "load_split", return_value=[]), self.assertRaisesRegex(ValueError, "loader disagrees"):
            transfer.write_derivative(plan, self.fixture.root / "three")

    def test_module_entry_point_runs_main(self) -> None:
        report = self.fixture.root / "entry-preview.json"
        arguments = ["reconcile_prefix_exposure_corpus", "--corpus", str(self.fixture.parent), "--inventory", str(self.fixture.inventory),
                     "--ud-origin", str(self.fixture.ud), "--technical-origin", str(self.fixture.technical), "--report", str(report)]
        with patch.object(sys, "argv", arguments), redirect_stdout(io.StringIO()), self.assertRaises(SystemExit) as context:
            runpy.run_module("reconcile_prefix_exposure_corpus", run_name="__main__", alter_sys=True)
        self.assertEqual(context.exception.code, 0)
        self.assertTrue(report.exists())


if __name__ == "__main__":
    unittest.main()
