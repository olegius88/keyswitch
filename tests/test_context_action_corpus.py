"""Model-blind UD surface reconstruction and leakage prevention."""

from __future__ import annotations

from dataclasses import replace
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

from freeze_context_action_corpus import (
    PINS, Sentence, SurfaceToken, Union, aligned_text, assigned_split,
    canonical, exposed_families, family_aliases, freeze, intended_window,
    load_split, partition, physical, read_conllu, sentence_rows,
    source_inventory, token_language, typo_variants,
)


def token(form: str, lemma: str | None = None, identifier: str = "1") -> SurfaceToken:
    return SurfaceToken(identifier, form, (lemma or form,), "NOUN", "_", "_")


def sentence(form: str, document: str, lemma: str | None = None) -> Sentence:
    return Sentence(document, document, "fixture", "fixture.conllu", "shared " + form,
                    (token("shared"), token(form, lemma, "2")))


def documents(namespace: str) -> dict[str, str]:
    result: dict[str, str] = {}
    for index in range(1000):
        document = "fixture:d" + str(index)
        result.setdefault(assigned_split(namespace, "document:" + document), document)
        if len(result) == 4:
            return result
    raise AssertionError("test fixture could not populate four splits")


class ContextActionCorpusTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def write(self, name: str, content: str) -> Path:
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
        return path

    def test_multiword_surface_and_empty_nodes_preserve_punctuation_and_spaces(self) -> None:
        path = self.write("fixture.conllu", """# newdoc id = discussion
# sent_id = 1
# text = I can't  wait!
1\tI\tI\tPRON\t_\t_\t0\troot\t_\t_
2-3\tcan't\t_\t_\t_\t_\t_\t_\t_\tSpacesAfter=\\s\\s
2\tca\tcan\tAUX\t_\t_\t1\tdep\t_\t_
3\tn't\tnot\tPART\t_\t_\t1\tdep\t_\t_
3.1\tphantom\tphantom\tNOUN\t_\t_\t_\t_\t1:dep\t_
4\twait\twait\tVERB\t_\t_\t1\tdep\t_\tSpaceAfter=No
5\t!\t!\tPUNCT\t_\t_\t1\tpunct\t_\t_

""")
        parsed = list(read_conllu(path, "fixture"))
        self.assertEqual(len(parsed), 1)
        item = parsed[0]
        self.assertEqual([entry.form for entry in item.tokens], ["I", "can't", "wait", "!"])
        text, spans, status = aligned_text(item)
        self.assertEqual((text, status), ("I can't  wait!", "exact-text-comment"))
        start, end = spans[1]
        self.assertEqual(text[start:end], "can't")
        rows = sentence_rows(item, Union())
        contraction = rows[1]
        self.assertEqual((contraction.lemma, contraction.before, contraction.after), ("can not", "I ", "  wait!"))
        self.assertEqual((contraction.spacing, contraction.literal_tail), ("  ", "  "))
        self.assertEqual(intended_window(contraction), "I can't  wait!")
        self.assertEqual(rows[2].literal_tail, "!")
        self.assertEqual(item.document, "fixture:discussion")

    def test_misc_spacing_reconstruction_is_retained_but_quarantined_without_exact_text(self) -> None:
        items = (replace(token("слово"), misc=r"SpacesBefore=\t|SpacesAfter=\s\s"),
                 replace(token("!", identifier="2"), misc=r"SpacesAfter=\n"))
        item = Sentence("s", "d", "fixture", "f", None, items)
        text, _spans, status = aligned_text(item)
        self.assertEqual(text, "\tслово  !\n")
        self.assertEqual(status, "misc-reconstructed-no-text")
        rows, _ = partition([item], "fixture", set())
        self.assertEqual(len(rows), 2)
        self.assertTrue(all(row.split == "quarantine" for row in rows))
        self.assertTrue(all("misc-reconstructed-no-text" in row.quarantine_reasons for row in rows))
        conflicting = replace(item, text="another surface")
        self.assertEqual(aligned_text(conflicting)[2], "text-alignment-conflict")

    def test_document_aliases_and_unmarked_sections_are_conservative(self) -> None:
        path = self.write("documents.conllu", """# sent_id = s1
# text = a
1\ta\ta\tNOUN\t_\t_\t0\troot\t_\t_

# sent_id = s2
# text = b
1\tb\tb\tNOUN\t_\t_\t0\troot\t_\t_

# newdoc_id = named
# sent_id = s3
# text = c
1\tc\tc\tNOUN\t_\t_\t0\troot\t_\t_

# newdoc
# sent_id = s4
# text = d
1\td\td\tNOUN\t_\t_\t0\troot\t_\t_
""")
        items = list(read_conllu(path, "fixture"))
        self.assertEqual(items[0].document, items[1].document)
        self.assertEqual(items[2].document, "fixture:named")
        self.assertNotEqual(items[2].document, items[3].document)

    def test_giant_component_quarantine_never_discards_conflicting_rows(self) -> None:
        namespace = "fixture:partition"
        docs = documents(namespace)
        forms = {"train": "orchard", "development": "nebula", "calibration": "turbine", "test": "marmalade"}
        items = [sentence(form, docs[split]) for split, form in forms.items()]
        rows, report = partition(items, namespace, set())
        self.assertEqual(report["mode"], "document-hash-with-family-conflict-quarantine")
        self.assertEqual(report["largest_component_documents"], 4)
        self.assertEqual(len(rows), 8)
        self.assertEqual(sum(row.split == "quarantine" for row in rows), 4)
        for split, form in forms.items():
            self.assertEqual(next(row.split for row in rows if row.original == form), split)
        for field in ("family", "document"):
            groups = [{getattr(row, field) for row in rows if row.split == split} for split in forms]
            self.assertTrue(all(not left & right for index, left in enumerate(groups) for right in groups[index + 1:]))

    def test_lemmas_and_prospective_typos_join_before_quarantining(self) -> None:
        namespace = "fixture:lemma"
        docs = documents(namespace)
        source = sentence("spelled", docs["train"], "spell")
        identifier = "fixture:fixture.conllu:" + source.identifier + ":2"
        variant = typo_variants("spelled", identifier)[0]
        items = [source, sentence(variant, docs["test"], "other-lemma")]
        rows, _ = partition(items, namespace, set())
        affected = [row for row in rows if row.original != "shared"]
        self.assertEqual(len({row.family for row in affected}), 1)
        self.assertTrue(all(row.split == "quarantine" for row in affected))
        self.assertEqual(typo_variants("cat", "same"), ())
        self.assertEqual(typo_variants("spell", "same"), typo_variants("spell", "same"))
        self.assertTrue(all(word[0] == "s" and word[-1] == "l" for word in typo_variants("spell", "same")))
        self.assertIn(physical("spell"), family_aliases(token("spelled", "spell")))

    def test_previously_exposed_lemma_cannot_become_a_new_test_target(self) -> None:
        namespace = "fixture:exposure"
        docs = documents(namespace)
        items = [sentence("orchard", docs["train"]), sentence("novel-form", docs["test"], "revealed")]
        rows, _ = partition(items, namespace, {physical("revealed")})
        found = next(row for row in rows if row.original == "novel-form")
        self.assertEqual(found.split, "quarantine")
        self.assertIn("family-previously-exposed", found.quarantine_reasons)

    def test_script_labels_do_not_inherit_the_surrounding_corpus_language(self) -> None:
        self.assertEqual(token_language("healthcheck"), ("en", 0, True))
        self.assertEqual(token_language("перезапуск"), ("ru", 1, True))
        self.assertEqual(token_language("apiВызов"), ("mixed", None, False))
        self.assertEqual(token_language("!"), ("und", None, False))
        self.assertEqual(token_language("café"), ("en", 0, False))

    def test_freeze_is_repeatable_and_loading_train_does_not_open_a_test_file(self) -> None:
        path = self.write("tiny.conllu", """# newdoc id = d
# sent_id = s
# text = orchard
1\torchard\torchard\tNOUN\t_\t_\t0\troot\t_\t_

""")
        first = self.root / "first"
        second = self.root / "second"
        a = freeze([("fixture", path)], first, "repeatable", set(), {}, [], 8, 8)
        b = freeze([("fixture", path)], second, "repeatable", set(), {}, [], 8, 8)
        self.assertEqual(a, b)
        (first / "test.jsonl.gz").write_bytes(b"intentionally inaccessible as a gzip split")
        train = load_split(first, "train")
        self.assertIsInstance(train, list)
        with self.assertRaisesRegex(ValueError, "compressed checksum"):
            load_split(first, "test")
        with self.assertRaisesRegex(ValueError, "overwrite"):
            freeze([("fixture", path)], first, "repeatable", set(), {}, [])
        with self.assertRaisesRegex(ValueError, "unknown corpus split"):
            load_split(first, "../test")

    def test_pinned_git_blobs_are_verified_before_corpus_use(self) -> None:
        for name, commit in PINS.items():
            directory = self.root / name
            directory.mkdir()
            data = b"# source fixture\n"
            path = directory / "fixture.conllu"
            path.write_bytes(data)
            pin = {"commit": commit, "repository": "https://github.com/UniversalDependencies/" + name,
                   "files": [{"path": path.name, "sha256": hashlib.sha256(data).hexdigest(),
                              "sha": hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()}]}
            (directory / "pin.json").write_bytes(canonical(pin))
        self.assertEqual(len(source_inventory(self.root)[0]), 2)
        damaged = self.root / next(iter(PINS)) / "fixture.conllu"
        damaged.write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "checksum differs"):
            source_inventory(self.root)

    def test_bounded_context_is_a_literal_source_slice(self) -> None:
        text = "a" * 120 + " sample " + "b" * 100
        item = Sentence("s", "d", "fixture", "f", text,
                        (token("a" * 120), token("sample", identifier="2"), token("b" * 100, identifier="3")))
        row = sentence_rows(item, Union())[1]
        self.assertEqual((len(row.before), len(row.after)), (96, 64))
        self.assertEqual(row.before, text[:121][-96:])
        self.assertEqual(row.after, text[127:191])
