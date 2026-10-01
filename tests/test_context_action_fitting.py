"""Extending the fitting splits must not hand training a row a sealed test already held."""

from __future__ import annotations

import sys
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path
from typing import cast

TOOLS = str(Path(__file__).resolve().parents[1] / "tools")
if TOOLS not in sys.path:
    sys.path.insert(0, TOOLS)

from freeze_context_action_corpus import digest, read_conllu, row_identifier, sentence_rows, Union
from freeze_context_action_fitting import BaseCorpus, base_corpus_from_rows, fitting_rows, fitting_split
from freeze_context_action_holdout import sentence_documents
from reconcile_context_action_corpus import expanded_aliases, membership_aliases, row_aliases
from test_context_action_holdout import CONLLU_MARKED, empty_exclusions

NAMESPACE = "keyswitch:test:fitting"
EMPTY_BASE = base_corpus_from_rows([])


class FittingExtensionTests(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        (self.root / "marked.conllu").write_text(CONLLU_MARKED, encoding="utf-8")
        self.sentences = sentence_documents(read_conllu(self.root / "marked.conllu", "M"))
        self.rows = [row for sentence in self.sentences for row in sentence_rows(sentence, Union())]

    def fit(self, base: BaseCorpus = EMPTY_BASE, **refusals: set[str]) -> dict[str, tuple[str, ...]]:
        rows, _summary = fitting_rows(
            self.sentences, empty_exclusions(), NAMESPACE, base, set(),
            refusals.get("test_rows", set()), refusals.get("test_documents", set()),
            refusals.get("test_aliases", set()))
        return {row.original: row.quarantine_reasons for row in rows}

    def test_a_row_of_a_sealed_test_is_refused_by_its_own_digest(self) -> None:
        """A family identifier is recomputed per freeze; a row digest is not.

        Corpus v10 excluded by family alone and let five rows of the consumed TEST v9
        into its training split, because the same sentence selected under a second
        namespace was grouped into a family with a different identifier.
        """

        held = next(row for row in self.rows if row.original == "гуляли")
        reasons = self.fit(test_rows={digest(held.identifier)})
        self.assertEqual(reasons["гуляли"], ("prior-accessed-test-row",))
        self.assertEqual(reasons["мы"], ())

    def test_a_document_of_a_sealed_test_takes_its_whole_sentence_out(self) -> None:
        held = next(row for row in self.rows if row.original == "гуляли")
        reasons = self.fit(test_documents={digest(held.document)})
        self.assertEqual(reasons["гуляли"], ("prior-accessed-test-document",))
        self.assertEqual(reasons["мы"], ("prior-accessed-test-document",))
        self.assertEqual(reasons["Ветер"], ())

    def test_a_word_of_a_sealed_test_is_refused_in_any_sentence_and_any_form(self) -> None:
        """The exclusion is by the word, not by a family identifier another freeze computed.

        TEST v8 held `гуляли` from a treebank; a Tatoeba sentence with `гулять` grouped it
        into a family with a different identifier, which no identifier set refused, and the
        word went to training before its test was evaluated.
        """

        held = next(row for row in self.rows if row.original == "гуляли")
        aliases = set(membership_aliases([replace(held, split="test")]))
        self.assertTrue(aliases >= expanded_aliases("гуляли") | expanded_aliases("гулять"))
        # The same word under another family identifier: a different sentence, a different freeze.
        other = replace(held, family=digest("another-freeze"), identifier="other-corpus:row", document="other-corpus:doc")
        reasons = self.fit(base_corpus_from_rows([], [replace(other, split="test")]))
        self.assertEqual(reasons["гуляли"], ("prior-accessed-test-family",))
        self.assertEqual(reasons["мы"], ())
        # And by the lemma alone: the test held the lemma's form, the new sentence another form.
        reasons = self.fit(test_aliases=expanded_aliases("гулять"))
        self.assertEqual(reasons["гуляли"], ("prior-accessed-test-family",))

    def test_a_membership_that_names_its_words_spares_the_test_split(self) -> None:
        held = replace(next(row for row in self.rows if row.original == "гуляли"), split="test")
        membership = {"row_ids_sha256": [digest(held.identifier)], "family_ids_sha256": [held.family],
                      "document_ids_sha256": [digest(held.document)], "alias_sha256": membership_aliases([held])}
        from_membership = base_corpus_from_rows([], membership=membership)
        from_rows = base_corpus_from_rows([], [held])
        self.assertEqual(from_membership.test_alias_source, "membership")
        self.assertEqual(from_rows.test_alias_source, "test-split")
        self.assertEqual(from_membership.test_aliases, from_rows.test_aliases)
        self.assertEqual(from_membership.test_rows, from_rows.test_rows)

    def test_without_a_refusal_every_row_joins_a_fitting_split(self) -> None:
        rows, summary = fitting_rows(self.sentences, empty_exclusions(), NAMESPACE, EMPTY_BASE, set(), set(), set(), set())
        self.assertEqual({row.split for row in rows}, {"train", "development", "calibration"} & {row.split for row in rows})
        self.assertNotIn("quarantine", {row.split for row in rows})
        self.assertEqual(summary["added_rows"], len(rows))

    def test_a_word_the_base_already_fits_keeps_that_split(self) -> None:
        """Otherwise a word would be fitted in one split and measured in another.

        The base's family identifiers are not what the new freeze computes for the same
        word (8 of 10 forms differed between a UD freeze and a Tatoeba union), so the
        split is followed by the word's aliases, whatever identifier either side carries.
        """

        held = next(row for row in self.rows if row.original == "гуляли")
        base_row = replace(held, family=digest("base-freeze"), split="calibration")
        base = base_corpus_from_rows([base_row])
        self.assertTrue(set(base.alias_splits) >= row_aliases(base_row))
        rows, summary = fitting_rows(self.sentences, empty_exclusions(), NAMESPACE, base, set(), set(), set(), set())
        self.assertEqual({row.split for row in rows if row.original == "гуляли"}, {"calibration"})
        self.assertGreaterEqual(cast(int, summary["followed_base_split"]), 1)

    def test_a_word_the_base_fits_in_two_splits_is_quarantined(self) -> None:
        held = next(row for row in self.rows if row.original == "гуляли")
        # The base fits the form in one split and its lemma in another: a word of either
        # would be fitted on one side and measured on the other.
        base = base_corpus_from_rows([replace(held, split="train"),
                                      replace(held, identifier=held.identifier + ":2", original="гулять",
                                              family=digest("lemma-family"), split="development")])
        self.assertTrue(base.contested_aliases)
        reasons = self.fit(base)
        self.assertEqual(reasons["гуляли"], ("base-split-conflict",))
        self.assertEqual(reasons["мы"], ())

    def test_the_split_of_a_new_family_is_a_function_of_its_name(self) -> None:
        self.assertEqual(fitting_split(NAMESPACE, "x"), fitting_split(NAMESPACE, "x"))
        self.assertIn(fitting_split(NAMESPACE, "x"), ("train", "development", "calibration"))
        self.assertEqual(row_identifier(self.sentences[0], self.sentences[0].tokens[0]),
                         row_identifier(self.sentences[0], self.sentences[0].tokens[0]))


if __name__ == "__main__":
    unittest.main()
