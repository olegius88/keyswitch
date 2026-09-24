"""Opaque gzip composition keeps sealed source bytes and detects leakage."""

from __future__ import annotations

from dataclasses import asdict
import gzip
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

from freeze_context_action_corpus import CorpusRow, canonical, checksum, digest, load_split
from model_protocol import ALL_SPLITS
from merge_context_action_corpora import merge_corpora


class ContextActionMergeTests(unittest.TestCase):
    # base and extra each contribute one row to the merged test membership.
    MERGED_ROW_COUNT = 2
    # Repeats row_ids_sha256 to fabricate a duplicate membership entry.
    DOUBLED_ROW_IDS_FACTOR = 2
    # Wrong row count (the fixture actually streams one row) to trigger detection.
    FORGED_TRAIN_ROW_COUNT = 2

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.base, self.extra = self.root / "base", self.root / "extra"
        self.fixture(self.base, "base", False)
        self.fixture(self.extra, "extra", True)

    def fixture(self, directory: Path, name: str, technical: bool) -> None:
        directory.mkdir()
        generator = directory / "generator-source.py"
        generator.write_bytes(b"# fixture generator\n")
        rows: dict[str, CorpusRow] = {}
        files = {}
        for split in ALL_SPLITS:
            row = CorpusRow(
                identifier=name + ":" + split, original="orchard" if name == "base" else "nebula", group=0,
                before="", after="", lemma="fixture", family=digest(name + ":family:" + split),
                document=name + ":document:" + split, language="en", source="fixture", source_file="fixture",
                source_sentence="1", source_token="1", spacing=" ", space_before="", literal_tail="", upos="X",
                features="_", misc="_", alignment="fixture", layout_representable=True, split=split,
            )
            rows[split] = row
            raw = canonical(asdict(row))
            path = directory / (split + ".jsonl.gz")
            path.write_bytes(gzip.compress(raw, mtime=0))
            files[split] = {"path": path.name, "sha256": checksum(path),
                            "content_sha256": hashlib.sha256(raw).hexdigest(), "rows": 1}
        membership = {
            "namespace": name, "scope": "fixture only", "row_ids_sha256": [digest(rows["test"].identifier)],
            "family_ids_sha256": [rows["test"].family], "document_ids_sha256": [digest(rows["test"].document)],
        }
        member_path = directory / "test-membership.json"
        member_path.write_bytes(canonical(membership))
        manifest = {"schema_version": 1, "namespace": name, "label": "keep", "splits": files,
                    "generator_source": generator.name, "generator_sha256": checksum(generator),
                    "test_membership_sha256": checksum(member_path)}
        if technical:
            sidecar = directory / "family-aliases.json"
            sidecar.write_bytes(canonical({"schema_version": 1, "families": {
                row.family: {"split": split, "aliases_sha256": [digest(name + ":alias:" + split)],
                             "document_ids_sha256": [digest(row.document)]}
                for split, row in rows.items() if split != "quarantine"
            }}))
            manifest["family_aliases_sha256"] = checksum(sidecar)
        else:
            sidecar = directory / "physical-family-closure.json"
            sidecar.write_bytes(canonical({"schema_version": 1,
                "alias_sha256_to_closure_sha256": {digest(name + ":alias:" + split): digest(name + ":closure:" + split) for split in ALL_SPLITS}}))
            manifest["reconciliation"] = {"closure_sha256": checksum(sidecar)}
        (directory / "manifest.json").write_bytes(canonical(manifest))

    def object(self, path: Path) -> dict[str, object]:
        return json.loads(path.read_bytes())  # type: ignore[no-any-return]

    def repin(self, directory: Path, sidecar: str, field: str) -> None:
        path = directory / "manifest.json"
        manifest = self.object(path)
        manifest[field] = checksum(directory / sidecar)
        path.write_bytes(canonical(manifest))

    def test_gzip_members_are_exact_source_bytes_and_loader_reads_all_members(self) -> None:
        output = self.root / "merged"
        manifest = merge_corpora(self.base, self.extra, output)
        self.assertEqual(manifest["namespace"], "base")
        for split in ALL_SPLITS:
            source_bytes = (self.base / (split + ".jsonl.gz")).read_bytes() + (self.extra / (split + ".jsonl.gz")).read_bytes()
            self.assertEqual((output / (split + ".jsonl.gz")).read_bytes(), source_bytes)
            rows = load_split(output, split)
            self.assertEqual([row.identifier for row in rows], ["base:" + split, "extra:" + split])
        membership = self.object(output / "test-membership.json")
        self.assertEqual(len(membership["row_ids_sha256"]), self.MERGED_ROW_COUNT)  # type: ignore[arg-type]
        self.assertNotIn("reconciliation", manifest)

    def test_corrupt_compressed_source_is_rejected_before_output_exists(self) -> None:
        (self.base / "test.jsonl.gz").write_bytes(b"damaged")
        output = self.root / "merged"
        with self.assertRaisesRegex(ValueError, "compressed checksum"):
            merge_corpora(self.base, self.extra, output)
        self.assertFalse(output.exists())

    def test_forged_compressed_hash_does_not_hide_changed_raw_content(self) -> None:
        path = self.base / "train.jsonl.gz"
        path.write_bytes(gzip.compress(b"changed\n", mtime=0))
        manifest_path = self.base / "manifest.json"
        manifest = json.loads(manifest_path.read_bytes())
        manifest["splits"]["train"]["sha256"] = checksum(path)
        manifest_path.write_bytes(canonical(manifest))
        with self.assertRaisesRegex(ValueError, "raw content checksum"):
            merge_corpora(self.base, self.extra, self.root / "merged")

    def test_nonempty_file_is_required_even_when_forged_hash_matches(self) -> None:
        path = self.base / "train.jsonl.gz"
        path.write_bytes(b"")
        manifest_path = self.base / "manifest.json"
        manifest = json.loads(manifest_path.read_bytes())
        manifest["splits"]["train"].update(sha256=checksum(path), content_sha256=checksum(path), rows=0)
        manifest_path.write_bytes(canonical(manifest))
        with self.assertRaisesRegex(ValueError, "empty gzip"):
            merge_corpora(self.base, self.extra, self.root / "merged")

    def test_fake_disjoint_flag_cannot_override_actual_alias_overlap(self) -> None:
        sidecar = self.extra / "family-aliases.json"
        data = json.loads(sidecar.read_bytes())
        for split in ("test", "quarantine"):
            with self.subTest(split=split):
                next(iter(data["families"].values()))["aliases_sha256"] = [digest("base:alias:" + split)]
                data["disjoint"] = True
                sidecar.write_bytes(canonical(data))
                self.repin(self.extra, sidecar.name, "family_aliases_sha256")
                with self.assertRaisesRegex(ValueError, "base physical alias"):
                    merge_corpora(self.base, self.extra, self.root / "merged")

    def test_same_technical_alias_in_different_splits_is_rejected(self) -> None:
        path = self.extra / "family-aliases.json"
        data = json.loads(path.read_bytes())
        records = list(data["families"].values())
        records[1]["aliases_sha256"] = records[0]["aliases_sha256"]
        path.write_bytes(canonical(data))
        self.repin(self.extra, path.name, "family_aliases_sha256")
        with self.assertRaisesRegex(ValueError, "technical alias"):
            merge_corpora(self.base, self.extra, self.root / "merged")

    def test_duplicate_membership_is_not_silently_deduplicated(self) -> None:
        path = self.base / "test-membership.json"
        data = json.loads(path.read_bytes())
        data["row_ids_sha256"] *= self.DOUBLED_ROW_IDS_FACTOR
        path.write_bytes(canonical(data))
        self.repin(self.base, path.name, "test_membership_sha256")
        with self.assertRaisesRegex(ValueError, "duplicate membership"):
            merge_corpora(self.base, self.extra, self.root / "merged")

    def test_overlapping_origin_membership_is_rejected(self) -> None:
        path = self.extra / "test-membership.json"
        data = json.loads(path.read_bytes())
        data["row_ids_sha256"] = [digest("base:test")]
        path.write_bytes(canonical(data))
        self.repin(self.extra, path.name, "test_membership_sha256")
        with self.assertRaisesRegex(ValueError, "origin test membership overlap"):
            merge_corpora(self.base, self.extra, self.root / "merged")

    def test_destination_cannot_be_overwritten(self) -> None:
        output = self.root / "merged"
        output.mkdir()
        sentinel = output / "preserve"
        sentinel.write_bytes(b"original")
        with self.assertRaisesRegex(ValueError, "overwrite"):
            merge_corpora(self.base, self.extra, output)
        self.assertEqual(sentinel.read_bytes(), b"original")

    def test_test_membership_count_must_match_streamed_source_rows(self) -> None:
        path = self.base / "test-membership.json"
        data = json.loads(path.read_bytes())
        data["row_ids_sha256"] = []
        path.write_bytes(canonical(data))
        self.repin(self.base, path.name, "test_membership_sha256")
        with self.assertRaisesRegex(ValueError, "test membership row count"):
            merge_corpora(self.base, self.extra, self.root / "merged")

    def test_source_row_count_and_final_newline_are_checked_without_json_parsing(self) -> None:
        manifest_path = self.base / "manifest.json"
        manifest = json.loads(manifest_path.read_bytes())
        manifest["splits"]["train"]["rows"] = self.FORGED_TRAIN_ROW_COUNT
        manifest_path.write_bytes(canonical(manifest))
        with self.assertRaisesRegex(ValueError, "streamed row count"):
            merge_corpora(self.base, self.extra, self.root / "merged")
        path = self.base / "train.jsonl.gz"
        path.write_bytes(gzip.compress(b"opaque bytes without a final newline", mtime=0))
        manifest["splits"]["train"]["sha256"] = checksum(path)
        manifest_path.write_bytes(canonical(manifest))
        with self.assertRaisesRegex(ValueError, "complete nonempty lines"):
            merge_corpora(self.base, self.extra, self.root / "merged")

    def test_stale_alias_sidecar_hash_and_test_family_membership_are_rejected(self) -> None:
        path = self.extra / "family-aliases.json"
        data = json.loads(path.read_bytes())
        test_family = next(family for family, record in data["families"].items() if record["split"] == "test")
        data["families"][test_family]["split"] = "train"
        path.write_bytes(canonical(data))
        with self.assertRaisesRegex(ValueError, "sidecar checksum"):
            merge_corpora(self.base, self.extra, self.root / "merged")
        self.repin(self.extra, path.name, "family_aliases_sha256")
        with self.assertRaisesRegex(ValueError, "test family membership"):
            merge_corpora(self.base, self.extra, self.root / "merged")

    def test_package_document_cannot_span_technical_splits(self) -> None:
        path = self.extra / "family-aliases.json"
        data = json.loads(path.read_bytes())
        records = list(data["families"].values())
        records[1]["document_ids_sha256"] = records[0]["document_ids_sha256"]
        path.write_bytes(canonical(data))
        self.repin(self.extra, path.name, "family_aliases_sha256")
        with self.assertRaisesRegex(ValueError, "document spans multiple splits"):
            merge_corpora(self.base, self.extra, self.root / "merged")

    def test_merging_same_origins_is_byte_repeatable(self) -> None:
        first, second = self.root / "first", self.root / "second"
        self.assertEqual(merge_corpora(self.base, self.extra, first), merge_corpora(self.base, self.extra, second))
        self.assertEqual((first / "manifest.json").read_bytes(), (second / "manifest.json").read_bytes())

    def test_changed_generator_cannot_keep_an_old_origin_receipt(self) -> None:
        (self.extra / "generator-source.py").write_bytes(b"# changed\n")
        with self.assertRaisesRegex(ValueError, "generator checksum"):
            merge_corpora(self.base, self.extra, self.root / "merged")

    def test_historical_ud_without_generator_filename_still_checks_archived_sources(self) -> None:
        path = self.base / "manifest.json"
        manifest = json.loads(path.read_bytes())
        del manifest["generator_source"]
        reconciler = self.base / "reconciliation-source.py"
        reconciler.write_bytes(b"# historical derivative\n")
        manifest["reconciliation"]["source_sha256"] = checksum(reconciler)
        path.write_bytes(canonical(manifest))
        merge_corpora(self.base, self.extra, self.root / "valid")
        reconciler.write_bytes(b"# changed\n")
        with self.assertRaisesRegex(ValueError, "reconciliation generator checksum"):
            merge_corpora(self.base, self.extra, self.root / "invalid")


if __name__ == "__main__":
    unittest.main()
