"""Technical corpus partitions reserve physical variants and package groups."""

from __future__ import annotations

import gzip
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

from context_technical_corpus import (
    Command, NAMESPACE, all_internal_edits, command_identifier, freeze_rows, load_technical_split,
    partition_commands, read_commands, reserved_ud_aliases, verified_source,
)
from freeze_context_action_corpus import canonical, checksum, typo_variants
from reconcile_context_action_corpus import expanded_aliases


def command(name: str, *owners: str) -> Command:
    return Command(name, ("usr/bin/" + name,), owners or ("utils/" + name,))


class ContextTechnicalCorpusTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def test_contents_selects_only_declared_paths_and_deduplicates_package_owners(self) -> None:
        path = self.root / "Contents-amd64.gz"
        with gzip.open(path, "wt") as stream:
            stream.write("usr/bin/zebrina utils/zebrina,science/zebrina-extra\n"
                         "bin/zebrina utils/zebrina\n"
                         "usr/sbin/caldera admin/caldera\n"
                         "usr/bin/a utils/a\nusr/bin/cmd-name utils/cmd-name\n"
                         "usr/bin/Camel utils/camel\nusr/lib/cmd utils/cmd\n"
                         "usr/bin/sub/path utils/path\n")
        records = read_commands(path)
        self.assertEqual([item.name for item in records], ["caldera", "zebrina"])
        self.assertEqual(records[1].paths, ("bin/zebrina", "usr/bin/zebrina"))
        self.assertEqual(records[1].owners, ("science/zebrina-extra", "utils/zebrina"))

    def test_pair_alias_and_all_declared_typos_join_before_partition(self) -> None:
        source = command("zebrina")
        variant = typo_variants(source.name, command_identifier(source.name))[0]
        records = [source, command(variant), command("яуикштф")]
        result = partition_commands(records, set())
        self.assertEqual(len({row.family for row in result.rows}), 1)
        self.assertEqual(len({row.split for row in result.rows if row.split != "quarantine"}), 1)
        self.assertEqual(len([row for row in result.rows if row.split != "quarantine"]), 2)

    def test_transitive_alias_collision_reserves_entire_family(self) -> None:
        source = command("zebrina")
        variant = typo_variants(source.name, command_identifier(source.name))[0]
        result = partition_commands([source, command(variant)], expanded_aliases(variant))
        self.assertTrue(all(row.split == "quarantine" for row in result.rows))
        self.assertTrue(all("reserved-physical-alias" in row.quarantine_reasons for row in result.rows))

    def test_package_components_do_not_leak_through_multiple_owner_edges(self) -> None:
        records = [command("zebrina", "utils/first"),
                   command("caldera", "utils/first", "science/second"),
                   command("vespers", "science/second")]
        result = partition_commands(records, set())
        self.assertEqual(len({row.document for row in result.rows}), 1)
        self.assertEqual(len({row.split for row in result.rows}), 1)
        self.assertEqual(result.summary["package_split_overlap"], 0)
        self.assertEqual(result.summary["alias_split_overlap"], 0)

    def test_same_package_in_different_sections_remains_one_document(self) -> None:
        result = partition_commands([command("zebrina", "utils/example"),
                                     command("caldera", "science/example")], set())
        self.assertEqual(len({row.document for row in result.rows}), 1)

    def test_multiple_splits_have_disjoint_package_and_alias_membership(self) -> None:
        alphabet = str.maketrans("0123456789abcdef", "abcdefghijklmnop")
        records = [command(hashlib.sha256(str(index).encode()).hexdigest()[:12].translate(alphabet),
                           "utils/package-" + str(index)) for index in range(400)]
        result = partition_commands(records, set())
        self.assertEqual({row.split for row in result.rows}, {"train", "development", "calibration", "test"})
        self.assertEqual(result.summary["alias_split_overlap"], 0)
        self.assertEqual(result.summary["package_split_overlap"], 0)
        split_by_document: dict[str, str] = {}
        for row in result.rows:
            if row.split == "quarantine":
                continue
            self.assertEqual(split_by_document.setdefault(row.document, row.split), row.split)

    def test_family_sidecar_keeps_aliases_of_fixed_cap_discarded_rows(self) -> None:
        source = command("zebrina")
        variant = typo_variants(source.name, command_identifier(source.name))[0]
        records = [source, command(variant), command("яуикштф")]
        result = partition_commands(records, set())
        output = self.root / "cap"
        freeze_rows(result, output, {}, {})
        metadata = json.loads((output / "family-aliases.json").read_bytes())
        self.assertEqual(len(metadata["families"]), 1)
        entry = next(iter(metadata["families"].values()))
        expected = {alias for aliases in result.aliases.values() for alias in aliases}
        self.assertEqual(set(entry["aliases_sha256"]), expected)

    def test_all_historical_internal_edits_cover_identifier_dependent_variants(self) -> None:
        forms = all_internal_edits({"a zebrina command"})
        for index in range(100):
            self.assertTrue(set(typo_variants("zebrina", str(index))) <= forms)
        self.assertIn("zebrina", forms)

    def test_partition_and_bounded_family_selection_are_input_order_independent(self) -> None:
        records = [command("zebrina"), command("caldera"), command("vespers")]
        first = partition_commands(records, set())
        second = partition_commands(list(reversed(records)), set())
        self.assertEqual(first, second)
        self.assertTrue(all(row.before == row.after == "" for row in first.rows))

    def test_ud_reservation_reads_hash_sidecar_without_opening_any_split(self) -> None:
        alias = next(iter(expanded_aliases("zebrina")))
        sidecar = self.root / "physical-family-closure.json"
        sidecar.write_bytes(canonical({"alias_sha256_to_closure_sha256": {alias: "a" * 64}}))
        manifest = self.root / "manifest.json"
        manifest.write_bytes(canonical({"reconciliation": {"closure_sha256": checksum(sidecar)}}))
        (self.root / "test.jsonl.gz").write_bytes(b"not readable gzip")
        aliases, pins = reserved_ud_aliases(self.root)
        self.assertEqual(aliases, {alias})
        self.assertEqual(len(pins), 2)
        sidecar.write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "sidecar checksum"):
            reserved_ud_aliases(self.root)

    def test_loader_is_explicit_and_train_never_opens_test_text(self) -> None:
        result = partition_commands([command("zebrina"), command("caldera")], set())
        output = self.root / "corpus"
        manifest = freeze_rows(result, output, {"fixture": "a" * 64}, {})
        self.assertEqual(manifest["namespace"], NAMESPACE)
        (output / "test.jsonl.gz").write_bytes(b"damaged on purpose")
        self.assertIsInstance(load_technical_split(output, "train"), list)
        with self.assertRaisesRegex(ValueError, "test access requires"):
            load_technical_split(output, "test")
        with self.assertRaisesRegex(ValueError, "compressed checksum"):
            load_technical_split(output, "test", allow_test=True)
        with self.assertRaisesRegex(ValueError, "training split"):
            load_technical_split(output, "quarantine")
        with self.assertRaisesRegex(ValueError, "overwrite"):
            freeze_rows(result, output, {}, {})

    def test_frozen_bytes_membership_and_provenance_are_repeatable(self) -> None:
        result = partition_commands([command("zebrina"), command("caldera")], set())
        first, second = self.root / "first", self.root / "second"
        a = freeze_rows(result, first, {"fixture": "a" * 64}, {})
        b = freeze_rows(result, second, {"fixture": "a" * 64}, {})
        self.assertEqual(a, b)
        self.assertEqual((first / "test-membership.json").read_bytes(),
                         (second / "test-membership.json").read_bytes())
        metadata = json.loads((first / "command-provenance.json").read_bytes())
        self.assertEqual(len(metadata["commands"]), 2)
        self.assertEqual(a["command_provenance_sha256"], checksum(first / "command-provenance.json"))
        aliases = json.loads((first / "family-aliases.json").read_bytes())
        self.assertEqual(a["family_aliases_sha256"], checksum(first / "family-aliases.json"))
        for record in aliases["families"].values():
            self.assertIn(record["split"], ("train", "development", "calibration", "test"))
            self.assertTrue(all(len(alias) == 64 for alias in record["aliases_sha256"]))
            self.assertTrue(all(len(alias) == 64 for alias in record["document_ids_sha256"]))

    def test_source_verification_requires_tls_receipt_and_exact_inrelease_sha256(self) -> None:
        contents = self.root / "Contents-amd64.gz"
        contents.write_bytes(gzip.compress(b"usr/bin/zebrina utils/zebrina\n", mtime=0))
        release = self.root / "InRelease"
        release.write_text("Header: fixture\nSHA256:\n " + checksum(contents) + " " +
                           str(contents.stat().st_size) + " main/Contents-amd64.gz\nSHA512:\n", encoding="utf-8")
        receipt = {
            "contents": {"status": 200, "url": "https://deb.debian.org/debian/dists/trixie/main/Contents-amd64.gz"},
            "release": {"status": 200, "url": "https://deb.debian.org/debian/dists/trixie/InRelease"},
            "tls_certificate_verification": True, "contents_sha256": checksum(contents),
            "contents_bytes": contents.stat().st_size, "release_sha256": checksum(release),
        }
        path = self.root / "source-receipt.json"
        path.write_bytes(canonical(receipt))
        verified, pins, metadata = verified_source(self.root)
        self.assertEqual(verified, contents)
        self.assertEqual(len(pins), 3)
        self.assertIn("not verified", str(metadata["verification"]))
        release.write_text("SHA256:\n", encoding="utf-8")
        receipt["release_sha256"] = checksum(release)
        path.write_bytes(canonical(receipt))
        with self.assertRaisesRegex(ValueError, "absent from InRelease"):
            verified_source(self.root)
        receipt["tls_certificate_verification"] = False
        path.write_bytes(canonical(receipt))
        with self.assertRaisesRegex(ValueError, "verified TLS"):
            verified_source(self.root)

    def test_malformed_selected_owner_is_rejected_and_empty_corpus_is_stable(self) -> None:
        path = self.root / "Contents-amd64.gz"
        path.write_bytes(gzip.compress(b"usr/bin/zebrina bad-owner\n", mtime=0))
        with self.assertRaisesRegex(ValueError, "package ownership"):
            read_commands(path)
        result = partition_commands([], set())
        self.assertEqual(result.summary["largest_component_commands"], 0)
        with self.assertRaisesRegex(ValueError, "unique names"):
            partition_commands([command("zebrina"), command("zebrina")], set())


class ContentsOwnershipTests(unittest.TestCase):
    def test_ubuntu_component_prefixed_ownership_is_read_and_garbage_still_refused(self) -> None:
        import gzip
        import tempfile
        from pathlib import Path

        with tempfile.TemporaryDirectory() as temporary:
            contents = Path(temporary) / "Contents-amd64.gz"
            with gzip.open(contents, "wt", encoding="utf-8") as stream:
                stream.write("usr/bin/abduco\tuniverse/utils/abduco\n")
                stream.write("usr/bin/zsh\tshells/zsh,universe/shells/zsh-static\n")
            commands = {command.name: command for command in read_commands(contents)}
            self.assertEqual(commands["abduco"].owners, ("universe/utils/abduco",))
            self.assertEqual(commands["zsh"].owners, ("shells/zsh", "universe/shells/zsh-static"))
            with gzip.open(contents, "wt", encoding="utf-8") as stream:
                stream.write("usr/bin/abduco\tUniverse/Utils/Abduco\n")
            with self.assertRaises(ValueError):
                read_commands(contents)


if __name__ == "__main__":
    unittest.main()
