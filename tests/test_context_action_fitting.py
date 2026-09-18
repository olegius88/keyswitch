"""Extending the fitting splits must not hand training a row a sealed test already held."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from typing import cast

from freeze_context_action_corpus import digest, read_conllu, row_identifier, sentence_rows, Union
from freeze_context_action_fitting import fitting_rows, fitting_split
from freeze_context_action_holdout import sentence_documents
from test_context_action_holdout import CONLLU_MARKED, empty_exclusions

NAMESPACE = "keyswitch:test:fitting"


class FittingExtensionTests(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        (self.root / "marked.conllu").write_text(CONLLU_MARKED, encoding="utf-8")
        self.sentences = sentence_documents(read_conllu(self.root / "marked.conllu", "M"))
        self.rows = [row for sentence in self.sentences for row in sentence_rows(sentence, Union())]

    def fit(self, **refusals: set[str]) -> dict[str, tuple[str, ...]]:
        rows, _summary = fitting_rows(
            self.sentences, empty_exclusions(), NAMESPACE, {}, set(), set(),
            refusals.get("test_rows", set()), refusals.get("test_documents", set()))
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

    def test_without_a_refusal_every_row_joins_a_fitting_split(self) -> None:
        rows, summary = fitting_rows(self.sentences, empty_exclusions(), NAMESPACE, {}, set(), set(), set(), set())
        self.assertEqual({row.split for row in rows}, {"train", "development", "calibration"} & {row.split for row in rows})
        self.assertNotIn("quarantine", {row.split for row in rows})
        self.assertEqual(summary["added_rows"], len(rows))

    def test_a_family_the_base_already_fits_keeps_that_split(self) -> None:
        """Otherwise a family would be fitted in one split and measured in another.

        The identifier is taken from a first pass rather than from a separate grouping:
        the same rows grouped twice do not have to produce the same family name, which
        is the reason a row digest, not a family, answers "has a test seen this".
        """

        first, _ = fitting_rows(self.sentences, empty_exclusions(), NAMESPACE, {}, set(), set(), set(), set())
        family = next(row.family for row in first if row.split != "calibration")
        rows, summary = fitting_rows(self.sentences, empty_exclusions(), NAMESPACE,
                                     {family: "calibration"}, set(), set(), set(), set())
        placed = {row.split for row in rows if row.family == family}
        self.assertEqual(placed, {"calibration"})
        self.assertGreaterEqual(cast(int, summary["followed_base_split"]), 1)

    def test_the_split_of_a_new_family_is_a_function_of_its_name(self) -> None:
        self.assertEqual(fitting_split(NAMESPACE, "x"), fitting_split(NAMESPACE, "x"))
        self.assertIn(fitting_split(NAMESPACE, "x"), ("train", "development", "calibration"))
        self.assertEqual(row_identifier(self.sentences[0], self.sentences[0].tokens[0]),
                         row_identifier(self.sentences[0], self.sentences[0].tokens[0]))


if __name__ == "__main__":
    unittest.main()
