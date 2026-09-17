"""Test-only holdout extension: sampling, exclusion by known aliases, sid commands, assembly and ledger dry check."""

from __future__ import annotations

import gzip
import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path
from typing import cast

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

from context_technical_corpus import read_commands, Command
from freeze_context_action_corpus import CorpusRow, Union, canonical, checksum, digest, freeze, load_split, physical, read_conllu
from freeze_context_action_holdout import (
    PRIOR_TEST, Exclusions, alias_reasons, assemble, base_test_rows_to_quarantine, holdout_inventory, prior_access_overlap,
    select_holdout_sentences, sentence_documents, sid_holdout_rows, ud_holdout_rows, verified_sid_source,
    arch_holdout_rows, read_arch_commands, verified_arch_source, ARCH_MIRROR,
    fedora_holdout_rows, read_fedora_commands, verified_fedora_source, FEDORA_BASE, OPENSUSE_BASE,
    read_tatoeba, tatoeba_sentence, verified_tatoeba_source, TATOEBA_BASE, UBUNTU_CONTENTS_URL, UBUNTU_RELEASE_URL,
)
from reconcile_context_action_corpus import expanded_aliases

CONLLU_MARKED = """# newdoc id = doc-a
# sent_id = a1
# text = Сегодня мы гуляли долго.
1\tСегодня\tсегодня\tADV\t_\t_\t3\tadvmod\t_\t_
2\tмы\tмы\tPRON\t_\t_\t3\tnsubj\t_\t_
3\tгуляли\tгулять\tVERB\t_\t_\t0\troot\t_\t_
4\tдолго\tдолго\tADV\t_\t_\t3\tadvmod\t_\tSpaceAfter=No
5\t.\t.\tPUNCT\t_\t_\t3\tpunct\t_\t_

# newdoc id = doc-b
# sent_id = b1
# text = Ветер стих.
1\tВетер\tветер\tNOUN\t_\t_\t2\tnsubj\t_\t_
2\tстих\tстихнуть\tVERB\t_\t_\t0\troot\t_\tSpaceAfter=No
3\t.\t.\tPUNCT\t_\t_\t2\tpunct\t_\t_

"""
CONLLU_UNMARKED = """# sent_id = s1
# text = Река широка.
1\tРека\tрека\tNOUN\t_\t_\t2\tnsubj\t_\t_
2\tширока\tширокий\tADJ\t_\t_\t0\troot\t_\tSpaceAfter=No
3\t.\t.\tPUNCT\t_\t_\t2\tpunct\t_\t_

# sent_id = s2
# text = Лес темнеет.
1\tЛес\tлес\tNOUN\t_\t_\t2\tnsubj\t_\t_
2\tтемнеет\tтемнеть\tVERB\t_\t_\t0\troot\t_\tSpaceAfter=No
3\t.\t.\tPUNCT\t_\t_\t2\tpunct\t_\t_

"""


def empty_exclusions(**overrides: object) -> Exclusions:
    values: dict[str, object] = {name: frozenset() for name in ("base_aliases", "prior_test_aliases", "prefix_aliases", "prior_test_families",
                                                                  "exposed_physical", "historical_aliases", "lexicon")}
    values["provenance"] = {}
    values.update(overrides)
    return Exclusions(**values)  # type: ignore[arg-type]


class HoldoutSamplingTests(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        (self.root / "marked.conllu").write_text(CONLLU_MARKED, encoding="utf-8")
        (self.root / "unmarked.conllu").write_text(CONLLU_UNMARKED, encoding="utf-8")

    def test_unmarked_treebank_is_sampled_by_sentence_and_bounds_apply_per_source(self) -> None:
        sentences = sentence_documents([*read_conllu(self.root / "marked.conllu", "M"), *read_conllu(self.root / "unmarked.conllu", "U")])
        self.assertEqual({s.document for s in sentences if s.source == "M"}, {"M:doc-a", "M:doc-b"})
        self.assertEqual({s.document for s in sentences if s.source == "U"}, {"U:sentence:unmarked.conllu:s1", "U:sentence:unmarked.conllu:s2"})
        selected, sampling = select_holdout_sentences(sentences, "ns", max_documents=1, max_sentences=12)
        self.assertEqual(sampling["selected_documents"], {"M": 1, "U": 1})
        self.assertEqual(len(selected), 2)
        again, _ = select_holdout_sentences(list(reversed(sentences)), "ns", max_documents=1, max_sentences=12)
        self.assertEqual([s.identifier for s in again], [s.identifier for s in selected])
        with self.assertRaises(ValueError):
            select_holdout_sentences(sentences, "ns", max_documents=0, max_sentences=12)
        with self.assertRaises(ValueError):
            select_holdout_sentences([*sentences, sentences[0]], "ns", max_documents=2, max_sentences=12)

    def test_every_family_known_anywhere_is_quarantined_with_its_reason(self) -> None:
        sentences = sentence_documents(read_conllu(self.root / "marked.conllu", "M"))
        rows, summary = ud_holdout_rows(sentences, empty_exclusions())
        self.assertEqual({row.split for row in rows}, {"test"})
        self.assertEqual(summary["test_documents"], 2)
        known = next(iter(expanded_aliases("гуляли")))
        prefix = next(iter(expanded_aliases("ветер")))
        exclusions = empty_exclusions(base_aliases=frozenset({known}), prefix_aliases=frozenset({prefix}),
                                      exposed_physical=frozenset({physical("долго")}))
        rows, summary = ud_holdout_rows(sentences, exclusions)
        by_form = {row.original: row for row in rows}
        self.assertEqual(by_form["гуляли"].quarantine_reasons, ("family-in-base-corpus",))
        self.assertEqual(by_form["Ветер"].quarantine_reasons, ("prefix-curriculum-prior-exposure",))
        self.assertEqual(by_form["долго"].quarantine_reasons, ("family-previously-exposed",))
        self.assertEqual(by_form["мы"].split, "test")
        self.assertEqual(summary["quarantine_reasons"], {"family-in-base-corpus": 1, "prefix-curriculum-prior-exposure": 1, "family-previously-exposed": 1})
        family = by_form["мы"].family
        rows, _ = ud_holdout_rows(sentences, empty_exclusions(prior_test_families=frozenset({family})))
        self.assertEqual({row.original: row.quarantine_reasons for row in rows}["мы"], ("prior-accessed-test-family",))
        self.assertEqual(alias_reasons(set(), set(), "x", empty_exclusions()), [])


class SidHoldoutTests(unittest.TestCase):
    def test_commands_outside_the_lexicon_form_the_technical_holdout_with_cap_and_reasons(self) -> None:
        commands = [Command("abduco", ("usr/bin/abduco",), ("utils/abduco",)), Command("nthash", ("usr/bin/nthash",), ("utils/x",)),
                    Command("zzuf", ("usr/bin/zzuf",), ("utils/zzuf",)), Command("zzufa", ("usr/bin/zzufa",), ("utils/zzuf",)),
                    Command("zzufb", ("usr/bin/zzufb",), ("utils/zzuf",))]
        known = next(iter(expanded_aliases("abduco")))
        exclusions = empty_exclusions(lexicon=frozenset({"nthash"}), prefix_aliases=frozenset({known}))
        rows, summary = sid_holdout_rows(commands, exclusions, "ns")
        by_name = {row.original: row for row in rows}
        self.assertNotIn("nthash", by_name)
        self.assertEqual(by_name["abduco"].quarantine_reasons, ("prefix-curriculum-prior-exposure",))
        self.assertEqual(by_name["abduco"].identifier, "debian-sid-main-amd64:command:abduco")
        self.assertEqual(by_name["abduco"].source, "Debian-sid-main-amd64")
        self.assertEqual({row.document for row in rows if row.original.startswith("zzuf")}.__len__(), 1)
        self.assertEqual(summary["commands_outside_lexicon"], 4)
        capped = cast(dict[str, int], summary["quarantine_reasons"]).get("fixed-hash-family-cap", 0)
        self.assertEqual(cast(int, summary["test_rows"]) + capped, 3 + capped)
        self.assertTrue(all(row.split == "test" for row in rows if not row.quarantine_reasons))
        with self.assertRaises(ValueError):
            sid_holdout_rows([Command("dup", ("usr/bin/dup",), ("a/b",)), Command("dup", ("usr/bin/dup",), ("a/b",))], empty_exclusions(), "ns")


class ArchHoldoutTests(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)

    def database(self, repository: str, packages: dict[str, list[str]]) -> Path:
        import io
        import tarfile
        path = self.root / f"{repository}.files"
        with tarfile.open(path, "w:gz") as archive:
            for package, files in packages.items():
                for member, text in ((f"{package}-1.0-1/desc", f"%FILENAME%\n{package}-1.0-1-x86_64.pkg.tar.zst\n\n%NAME%\n{package}\n\n%VERSION%\n1.0-1\n"),
                                     (f"{package}-1.0-1/files", "%FILES%\n" + "\n".join(files) + "\n")):
                    data = text.encode("utf-8")
                    info = tarfile.TarInfo(member)
                    info.size = len(data)
                    archive.addfile(info, io.BytesIO(data))
        return path

    def test_arch_databases_yield_usr_bin_commands_with_repository_owners(self) -> None:
        core = self.database("core", {"acl": ["usr/", "usr/bin/", "usr/bin/chacl", "usr/bin/getfacl", "usr/lib/libacl.so"]})
        extra = self.database("extra", {"zzuf": ["usr/bin/zzuf", "usr/share/doc/zzuf/README"], "acl-extra": ["usr/bin/chacl"]})
        commands = {item.name: item for item in read_arch_commands([("core", core), ("extra", extra)])}
        self.assertEqual(set(commands), {"chacl", "getfacl", "zzuf"})
        self.assertEqual(commands["chacl"].owners, ("core/acl", "extra/acl-extra"))
        self.assertEqual(commands["zzuf"].paths, ("usr/bin/zzuf",))
        rows, summary = arch_holdout_rows(list(commands.values()), empty_exclusions(lexicon=frozenset({"getfacl"})), "ns")
        by_name = {row.original: row for row in rows}
        self.assertNotIn("getfacl", by_name)
        self.assertEqual(by_name["zzuf"].identifier, "arch-x86_64:command:zzuf")
        self.assertEqual((by_name["zzuf"].source, by_name["zzuf"].source_file), ("Arch-x86_64", "core.files+extra.files"))
        self.assertTrue(by_name["zzuf"].document.startswith("arch-package-component:"))
        self.assertEqual(summary["commands_outside_lexicon"], 2)

    def test_arch_source_requires_tls_receipt_matching_both_databases(self) -> None:
        core = self.database("core", {"acl": ["usr/bin/chacl"]})
        extra = self.database("extra", {"zzuf": ["usr/bin/zzuf"]})
        receipt: dict[str, object] = {"tls_certificate_verification": True, "mirror": ARCH_MIRROR, "databases": {
            name: {"file": f"{name}.files", "sha256": checksum(path), "bytes": path.stat().st_size,
                   "download": {"status": 200, "url": f"{ARCH_MIRROR}{name}/os/x86_64/{name}.files"}}
            for name, path in (("core", core), ("extra", extra))}}
        (self.root / "source-receipt.json").write_text(json.dumps(receipt), encoding="utf-8")
        databases, provenance, metadata = verified_arch_source(self.root)
        self.assertEqual([name for name, _ in databases], ["core", "extra"])
        self.assertIn(str(extra), provenance)
        self.assertIn("no detached signature", str(metadata["verification"]))
        extra.write_bytes(extra.read_bytes() + b"\x00")
        with self.assertRaises(ValueError):
            verified_arch_source(self.root)
        extra.write_bytes(extra.read_bytes()[:-1])
        cast(dict[str, object], cast(dict[str, object], receipt["databases"])["core"])["download"] = {"status": 200, "url": "https://example.org/core.files"}
        (self.root / "source-receipt.json").write_text(json.dumps(receipt), encoding="utf-8")
        with self.assertRaises(ValueError):
            verified_arch_source(self.root)


class FedoraHoldoutTests(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)

    def primary(self, packages: dict[str, list[str]]) -> Path:
        from compression.zstd import ZstdFile
        body = "".join(
            f'<package type="rpm"><name>{name}</name><version epoch="0"/>'
            + "".join(f"<file>{path}</file>" for path in files)
            + "</package>"
            for name, files in packages.items()
        )
        document = f'<?xml version="1.0"?><metadata xmlns="http://linux.duke.edu/metadata/common">{body}</metadata>'
        path = self.root / "primary.xml.zst"
        with ZstdFile(path, "wb") as stream:
            stream.write(document.encode("utf-8"))
        return path

    def test_the_primary_index_yields_bin_commands_with_their_packages(self) -> None:
        primary = self.primary({
            "acl": ["/usr/bin/chacl", "/usr/share/doc/acl/README", "/usr/bin/getfacl"],
            "zzuf": ["/usr/bin/zzuf"],
            "other": ["/usr/bin/chacl", "/usr/lib/libz.so", "/usr/bin/x"],
        })
        commands = {item.name: item for item in read_fedora_commands(primary)}
        self.assertEqual(set(commands), {"chacl", "getfacl", "zzuf"})
        self.assertEqual(commands["chacl"].owners, ("fedora/acl", "fedora/other"))
        self.assertEqual(commands["zzuf"].paths, ("usr/bin/zzuf",))
        rows, summary = fedora_holdout_rows(list(commands.values()), empty_exclusions(lexicon=frozenset({"getfacl"})), "ns")
        by_name = {row.original: row for row in rows}
        self.assertNotIn("getfacl", by_name)
        self.assertEqual(by_name["zzuf"].identifier, "fedora-x86_64:command:zzuf")
        self.assertEqual((by_name["zzuf"].source, by_name["zzuf"].source_file), ("Fedora-x86_64", "primary.xml.zst"))
        self.assertTrue(by_name["zzuf"].document.startswith("fedora-package-component:"))
        self.assertEqual(summary["commands_outside_lexicon"], 2)

    def test_the_receipt_must_match_repomd_and_the_payload(self) -> None:
        primary = self.primary({"acl": ["/usr/bin/chacl"]})
        digest = checksum(primary)
        repomd = self.root / "repomd.xml"
        repomd.write_text(
            '<?xml version="1.0"?><repomd><data type="primary"><checksum type="sha256">'
            + digest + '</checksum><location href="repodata/' + digest + '-primary.xml.zst"/></data></repomd>',
            encoding="utf-8")
        receipt: dict[str, object] = {
            "tls_certificate_verification": True, "base_url": FEDORA_BASE,
            "repomd": {"status": 200, "url": FEDORA_BASE + "repodata/repomd.xml"},
            "repomd_sha256": checksum(repomd),
            "primary": {"sha256": digest, "href": "repodata/" + digest + "-primary.xml.zst"},
        }
        (self.root / "source-receipt.json").write_text(json.dumps(receipt), encoding="utf-8")
        path, provenance, metadata = verified_fedora_source(self.root)
        self.assertEqual(path, primary)
        self.assertIn(str(repomd), provenance)
        self.assertIn("OpenPGP signature not verified", str(metadata["verification"]))
        cast(dict[str, object], receipt["primary"])["href"] = "repodata/other-primary.xml.zst"
        (self.root / "source-receipt.json").write_text(json.dumps(receipt), encoding="utf-8")
        with self.assertRaises(ValueError):
            verified_fedora_source(self.root)
        primary.write_bytes(primary.read_bytes() + b"\x00")
        with self.assertRaises(ValueError):
            verified_fedora_source(self.root)

    def test_an_opensuse_receipt_is_verified_with_the_sha512_its_index_names(self) -> None:
        import hashlib

        primary = self.primary({"zypper": ["/usr/bin/zypper"]})
        payload = primary.read_bytes()
        declared, pin = hashlib.sha512(payload).hexdigest(), checksum(primary)
        repomd = self.root / "repomd.xml"
        repomd.write_text(
            '<?xml version="1.0"?><repomd><data type="primary"><checksum type="sha512">' + declared
            + '</checksum><location href="repodata/' + declared + '-primary.xml.zst"/></data></repomd>', encoding="utf-8")
        receipt: dict[str, object] = {
            "tls_certificate_verification": True, "base_url": OPENSUSE_BASE,
            "repomd": {"status": 200, "url": OPENSUSE_BASE + "repodata/repomd.xml"}, "repomd_sha256": checksum(repomd),
            "primary": {"sha256": pin, "checksum_type": "sha512", "checksum": declared,
                        "href": "repodata/" + declared + "-primary.xml.zst"},
        }
        (self.root / "source-receipt.json").write_text(json.dumps(receipt), encoding="utf-8")
        path, _provenance, metadata = verified_fedora_source(self.root)
        self.assertEqual((path, metadata["base_url"]), (primary, OPENSUSE_BASE))
        self.assertIn("sha512", str(metadata["verification"]))
        rows, _summary = fedora_holdout_rows(read_fedora_commands(primary), empty_exclusions(), "ns", base_url=OPENSUSE_BASE)
        self.assertEqual((rows[0].source, rows[0].identifier), ("openSUSE-x86_64", "opensuse-x86_64:command:zypper"))
        self.assertTrue(rows[0].document.startswith("opensuse-package-component:"))
        # A base URL the freezer does not know is refused, whatever the checksums say.
        receipt["base_url"] = "https://example.invalid/repo/"
        (self.root / "source-receipt.json").write_text(json.dumps(receipt), encoding="utf-8")
        with self.assertRaises(ValueError):
            verified_fedora_source(self.root)


class TatoebaHoldoutTests(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)

    def export(self, language: str, lines: list[tuple[str, str, str]]) -> Path:
        import bz2
        path = self.root / f"{language}_sentences.tsv.bz2"
        with bz2.open(path, "wt", encoding="utf-8") as stream:
            for identifier, declared, text in lines:
                stream.write(f"{identifier}\t{declared}\t{text}\n")
        return path

    def test_tokens_tile_the_sentence_and_punctuation_stands_alone(self) -> None:
        from freeze_context_action_corpus import aligned_text, sentence_rows

        sentence = tatoeba_sentence("42", "rus", "Давайте что-нибудь попробуем! Это - тест...", "Tatoeba-rus", "rus.tsv.bz2")
        self.assertEqual([token.form for token in sentence.tokens],
                         ["Давайте", "что-нибудь", "попробуем", "!", "Это", "-", "тест", ".", ".", "."])
        self.assertEqual({token.upos for token in sentence.tokens}, {"X", "PUNCT"})
        text, spans, alignment = aligned_text(sentence)
        self.assertEqual((alignment, len(spans)), ("exact-text-comment", len(sentence.tokens)))
        rows = sentence_rows(sentence, Union())
        by_form = {row.original: row for row in rows}
        self.assertEqual((by_form["попробуем"].after[:1], by_form["попробуем"].literal_tail), ("!", "! "))
        self.assertEqual((sentence.identifier, sentence.document), ("tatoeba:rus:42", "Tatoeba-rus:sentence:42"))

    def test_the_reader_thins_deterministically_and_refuses_a_foreign_line(self) -> None:
        lines = [(str(index), "rus", f"Предложение номер {index}.") for index in range(1, 2001)]
        path = self.export("rus", lines)
        kept = list(read_tatoeba(path, "Tatoeba-rus", "rus", "ns", per_mille=100))
        again = list(read_tatoeba(path, "Tatoeba-rus", "rus", "ns", per_mille=100))
        self.assertEqual([item.identifier for item in kept], [item.identifier for item in again])
        self.assertTrue(120 < len(kept) < 280, len(kept))
        self.assertNotEqual([item.identifier for item in kept], [item.identifier for item in read_tatoeba(path, "Tatoeba-rus", "rus", "other", per_mille=100)])
        foreign = self.export("eng", [("7", "rus", "Не тот язык.")])
        with self.assertRaises(ValueError):
            list(read_tatoeba(foreign, "Tatoeba-eng", "eng", "ns"))

    def test_the_receipt_pins_each_export_by_size_and_digest(self) -> None:
        path = self.export("rus", [("1", "rus", "Привет.")])
        receipt: dict[str, object] = {
            "tls_certificate_verification": True, "base_url": TATOEBA_BASE, "licence": "CC BY 2.0 FR",
            "files": {"rus_sentences.tsv.bz2": {"status": 200, "language": "rus", "url": TATOEBA_BASE + "rus/rus_sentences.tsv.bz2",
                                                "sha256": checksum(path), "bytes": path.stat().st_size}},
        }
        (self.root / "source-receipt.json").write_text(json.dumps(receipt), encoding="utf-8")
        exports, provenance, metadata = verified_tatoeba_source(self.root, ["rus"])
        self.assertEqual(exports, [("Tatoeba-rus", "rus", path)])
        self.assertIn(str(path), provenance)
        self.assertIn("no upstream signature", str(metadata["verification"]))
        with self.assertRaises(ValueError):
            verified_tatoeba_source(self.root, ["eng"])
        path.write_bytes(path.read_bytes() + b"\x00")
        with self.assertRaises(ValueError):
            verified_tatoeba_source(self.root, ["rus"])


class HoldoutAssemblyTests(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        source = self.root / "base.conllu"
        text = "".join(f"# newdoc id = d{index}\n# sent_id = d{index}-1\n# text = Слово{index} здесь.\n"
                       f"1\tСлово{index}\tслово{index}\tNOUN\t_\t_\t0\troot\t_\t_\n2\tздесь\tздесь\tADV\t_\t_\t1\tadvmod\t_\tSpaceAfter=No\n"
                       f"3\t.\t.\tPUNCT\t_\t_\t1\tpunct\t_\t_\n\n" for index in range(40))
        source.write_text(text, encoding="utf-8")
        self.base = self.root / "base"
        self.manifest = freeze([("fixture", source)], self.base, "base-ns", set(), {}, [{"repository": "fixture"}], 2000, 12)
        self.splits = cast(dict[str, dict[str, object]], self.manifest["splits"])
        self.assertGreater(cast(int, self.splits["test"]["rows"]), 0)
        marked = self.root / "new.conllu"
        marked.write_text(CONLLU_MARKED, encoding="utf-8")
        sentences = sentence_documents(read_conllu(marked, "M"))
        self.ud_rows, _ = ud_holdout_rows(sentences, empty_exclusions(base_aliases=frozenset(expanded_aliases("долго"))))
        self.sid_rows, _ = sid_holdout_rows([Command("abduco", ("usr/bin/abduco",), ("utils/abduco",))], empty_exclusions(), "ns")

    def test_assembly_keeps_base_splits_byte_for_byte_and_quarantines_the_accessed_test(self) -> None:
        output = self.root / "derived"
        manifest = assemble(self.base, output, "new-ns", self.ud_rows, self.sid_rows, {"p": "0" * 64}, {"note": "fixture"})
        for split in ("train", "development", "calibration"):
            self.assertEqual((self.base / f"{split}.jsonl.gz").read_bytes(), (output / f"{split}.jsonl.gz").read_bytes())
            self.assertEqual(cast(dict[str, dict[str, object]], manifest["splits"])[split], self.splits[split])
        test_rows = load_split(output, "test")
        self.assertEqual({row.split for row in test_rows}, {"test"})
        self.assertEqual({row.source for row in test_rows}, {"M", "Debian-sid-main-amd64"})
        quarantine = load_split(output, "quarantine")
        prior = [row for row in quarantine if row.quarantine_reasons == (PRIOR_TEST,)]
        self.assertEqual(len(prior), self.splits["test"]["rows"])
        self.assertEqual(len(quarantine), cast(int, self.splits["quarantine"]["rows"]) + len(prior)
                         + sum(row.split == "quarantine" for row in self.ud_rows))
        self.assertTrue((output / "quarantine.jsonl.gz").read_bytes().startswith((self.base / "quarantine.jsonl.gz").read_bytes()))
        membership = json.loads((output / "test-membership.json").read_bytes())
        base_membership = json.loads((self.base / "test-membership.json").read_bytes())
        for field in ("row_ids_sha256", "family_ids_sha256", "document_ids_sha256"):
            self.assertFalse(set(membership[field]) & set(base_membership[field]))
            self.assertEqual(membership[field], sorted(set(membership[field])))
        self.assertEqual(manifest["test_membership_sha256"], checksum(output / "test-membership.json"))
        self.assertEqual(cast(dict[str, object], manifest["origins"])["base_manifest_sha256"], checksum(self.base / "manifest.json"))
        self.assertEqual(cast(dict[str, object], manifest["partitioning"])["mode"], "test-only-holdout-extension")
        self.assertEqual(len(base_test_rows_to_quarantine(self.base)), len(prior))
        with self.assertRaises(ValueError):
            assemble(self.base, output, "new-ns", self.ud_rows, self.sid_rows, {}, {})
        with self.assertRaises(ValueError):
            assemble(self.base, self.base / "inside", "new-ns", self.ud_rows, self.sid_rows, {}, {})

    def test_ledger_dry_check_counts_shared_rows_families_and_documents(self) -> None:
        ledger = self.root / "ledger"
        ledger.mkdir()
        base_membership = json.loads((self.base / "test-membership.json").read_bytes())
        (ledger / ("a" * 64 + ".access.json")).write_bytes(canonical({"test_membership": base_membership}))
        self.assertEqual(prior_access_overlap(ledger, base_membership),
                         {field: len(base_membership[field]) for field in ("row_ids_sha256", "family_ids_sha256", "document_ids_sha256")})
        fresh = {"row_ids_sha256": [digest("x")], "family_ids_sha256": [digest("y")], "document_ids_sha256": [digest("z")]}
        self.assertEqual(prior_access_overlap(ledger, fresh), {"row_ids_sha256": 0, "family_ids_sha256": 0, "document_ids_sha256": 0})
        self.assertEqual(prior_access_overlap(self.root / "missing", fresh), {})


class HoldoutSourceVerificationTests(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)

    def test_pinned_blobs_are_verified_before_use(self) -> None:
        source = self.root / "UD_Russian-GSD"
        source.mkdir()
        content = CONLLU_UNMARKED.encode()
        (source / "ru_gsd-ud-test.conllu").write_bytes(content)
        blob = hashlib.sha1(b"blob %d\0" % len(content) + content).hexdigest()
        pin = {"repository": "https://github.com/UniversalDependencies/UD_Russian-GSD", "commit": "c" * 40,
               "files": [{"path": "ru_gsd-ud-test.conllu", "sha": blob, "sha256": hashlib.sha256(content).hexdigest(), "size": len(content)}]}
        (source / "pin.json").write_text(json.dumps(pin), encoding="utf-8")
        paths, metadata = holdout_inventory(self.root, {"UD_Russian-GSD": "c" * 40})
        self.assertEqual([p.name for _s, p in paths], ["ru_gsd-ud-test.conllu"])
        self.assertEqual(metadata[0]["pin_metadata_sha256"], checksum(source / "pin.json"))
        with self.assertRaises(ValueError):
            holdout_inventory(self.root, {"UD_Russian-GSD": "d" * 40})
        (source / "ru_gsd-ud-test.conllu").write_bytes(content + b"\n")
        with self.assertRaises(ValueError):
            holdout_inventory(self.root, {"UD_Russian-GSD": "c" * 40})

    def test_sid_source_requires_tls_receipt_and_inrelease_checksum(self) -> None:
        contents = self.root / "Contents-amd64.gz"
        with gzip.open(contents, "wt", encoding="utf-8") as stream:
            stream.write("usr/bin/abduco\tutils/abduco\n")
        sha = checksum(contents)
        release = self.root / "InRelease"
        release.write_text("SHA256:\n " + sha + " " + str(contents.stat().st_size) + " main/Contents-amd64.gz\nSHA512:\n", encoding="utf-8")
        receipt: dict[str, object] = {"contents": {"status": 200, "url": "https://deb.debian.org/debian/dists/sid/main/Contents-amd64.gz"},
                   "release": {"status": 200, "url": "https://deb.debian.org/debian/dists/sid/InRelease"},
                   "tls_certificate_verification": True, "contents_sha256": sha, "contents_bytes": contents.stat().st_size,
                   "release_sha256": checksum(release)}
        (self.root / "source-receipt.json").write_text(json.dumps(receipt), encoding="utf-8")
        path, provenance, metadata = verified_sid_source(self.root)
        self.assertEqual(path, contents)
        self.assertEqual(metadata["source"], "Debian sid main amd64 Contents")
        self.assertIn(str(contents), provenance)
        cast(dict[str, object], receipt["contents"])["url"] = "https://deb.debian.org/debian/dists/trixie/main/Contents-amd64.gz"
        (self.root / "source-receipt.json").write_text(json.dumps(receipt), encoding="utf-8")
        with self.assertRaises(ValueError):
            verified_sid_source(self.root)

    def test_an_ubuntu_receipt_names_the_merged_contents_and_its_verified_signature(self) -> None:
        contents = self.root / "Contents-amd64.gz"
        with gzip.open(contents, "wt", encoding="utf-8") as stream:
            stream.write("usr/bin/snapcraft\tdevel/snapcraft\n")
        sha = checksum(contents)
        release = self.root / "InRelease"
        release.write_text("SHA256:\n " + sha + " " + str(contents.stat().st_size) + " Contents-amd64.gz\nSHA512:\n", encoding="utf-8")
        receipt: dict[str, object] = {"contents": {"status": 200, "url": UBUNTU_CONTENTS_URL},
                   "release": {"status": 200, "url": UBUNTU_RELEASE_URL},
                   "tls_certificate_verification": True, "contents_sha256": sha, "contents_bytes": contents.stat().st_size,
                   "release_sha256": checksum(release), "signature": {"verified": True, "exit_code": 0}}
        (self.root / "source-receipt.json").write_text(json.dumps(receipt), encoding="utf-8")
        path, provenance, metadata = verified_sid_source(self.root)
        self.assertEqual((path, metadata["contents_url"]), (contents, UBUNTU_CONTENTS_URL))
        self.assertIn("signature verified", str(metadata["verification"]))
        rows, _summary = sid_holdout_rows(read_commands(contents), empty_exclusions(), "ns", contents_url=UBUNTU_CONTENTS_URL)
        self.assertEqual((rows[0].source, rows[0].identifier), ("Ubuntu-noble-amd64", "ubuntu-noble-amd64:command:snapcraft"))
        self.assertTrue(rows[0].document.startswith("ubuntu-package-component:"))


if __name__ == "__main__":
    unittest.main()
