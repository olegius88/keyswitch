"""The prefix exposure inventory names the fitted words' aliases and never opens the prefix test."""

from __future__ import annotations

import gzip
import json
import sys
import tempfile
import unittest
from pathlib import Path
from typing import cast

TOOLS = str(Path(__file__).resolve().parents[1] / "tools")
if TOOLS not in sys.path:
    sys.path.insert(0, TOOLS)

from fixture_values.counts import PREFIX_EXPOSURE_FIXTURE_PREFIX_LENGTH
from prefix_exposure_inventory import CURRICULUM_SPLITS, inventory, main
from reconcile_context_action_corpus import expanded_aliases


class PrefixExposureInventoryTests(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.corpus = Path(directory.name)
        words = {"train": ("their", "привет"), "development": ("because",), "calibration": ("когда",), "test": ("secret",)}
        for split, forms in words.items():
            with gzip.open(self.corpus / (split + ".jsonl.gz"), "wt", encoding="utf-8") as stream:
                for form in forms:
                    stream.write(json.dumps({"text": form[:PREFIX_EXPOSURE_FIXTURE_PREFIX_LENGTH], "family": form, "split": split}) + "\n")
        (self.corpus / "corpus.json").write_text("{}", encoding="utf-8")

    def test_the_fitted_splits_are_listed_and_the_test_split_is_not_opened(self) -> None:
        result = inventory(self.corpus)
        aliases = set(cast(list[str], result["aliases_sha256"]))
        by_split = cast(dict[str, list[str]], result["aliases_by_split"])
        self.assertTrue(aliases >= expanded_aliases("their") | expanded_aliases("th") | expanded_aliases("когда"))
        self.assertFalse(aliases & expanded_aliases("secret"))
        self.assertEqual(set(by_split), set(CURRICULUM_SPLITS))
        self.assertEqual(set(cast(dict[str, list[str]], result["aliases_by_scope"])), {"text", "family"})
        self.assertEqual({alias for values in by_split.values() for alias in values}, aliases)
        self.assertIs(result["test_json_opened"], False)
        self.assertIs(result["model_scoring"], False)
        self.assertEqual(set(cast(dict[str, str], result["provenance"])), {str(self.corpus / name) for name in
                                                     ("train.jsonl.gz", "development.jsonl.gz", "calibration.jsonl.gz", "corpus.json")})

    def test_a_row_without_a_word_is_refused(self) -> None:
        with gzip.open(self.corpus / "development.jsonl.gz", "wt", encoding="utf-8") as stream:
            stream.write(json.dumps({"text": "", "family": "x"}) + "\n")
        with self.assertRaisesRegex(ValueError, "without a text"):
            inventory(self.corpus)

    def test_main_writes_once_and_never_overwrites(self) -> None:
        output = self.corpus / "inventory.json"
        self.assertEqual(main(["--corpus", str(self.corpus), "--output", str(output)]), 0)
        self.assertEqual(json.loads(output.read_bytes())["kind"], "actual-prefix-model-input-exposure")
        with self.assertRaisesRegex(ValueError, "overwrite"):
            main(["--corpus", str(self.corpus), "--output", str(output)])


if __name__ == "__main__":
    unittest.main()
