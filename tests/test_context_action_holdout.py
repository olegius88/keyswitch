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

from context_technical_corpus import Command
from freeze_context_action_corpus import CorpusRow, canonical, checksum, digest, freeze, load_split, physical, read_conllu
from freeze_context_action_holdout import (
    PRIOR_TEST, Exclusions, alias_reasons, assemble, base_test_rows_to_quarantine, holdout_inventory, prior_access_overlap,
    select_holdout_sentences, sentence_documents, sid_holdout_rows, ud_holdout_rows, verified_sid_source,
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


if __name__ == "__main__":
    unittest.main()
