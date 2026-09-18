"""The packaged Russian lexicon supplement: strict loading and identical use by the training reference models."""

from __future__ import annotations

import hashlib
import json
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

from keyswitch.language_model import LanguageModel
from keyswitch.lexicon_supplement import SUPPLEMENT_ROOT, supplement_words

RESOURCE = SUPPLEMENT_ROOT / "lexicon-supplement-ru_RU.json"


class LexiconSupplementTests(unittest.TestCase):
    def test_packaged_supplement_is_the_pinned_derivation(self) -> None:
        payload = json.loads(RESOURCE.read_bytes())
        self.assertEqual((payload["schema_version"], payload["locale"], payload["name"]), (1, "ru_RU", "opensubtitles-2018-ru-full-min10-outside-onboard-v3"))
        self.assertEqual(payload["source"]["sha256"], "32dfd94138aea266" + payload["source"]["sha256"][16:])
        self.assertEqual(payload["selection"]["minimum_count"], 10)
        self.assertEqual(payload["base_lexicon"]["sha256"], hashlib.sha256((Path(__file__).resolve().parents[1] / "model/intent_v1/sources/ru_RU.lm").read_bytes()).hexdigest())
        words = supplement_words("ru_RU")
        self.assertEqual(len(words), payload["selection"]["selected"])
        self.assertEqual(list(words), sorted(set(words)))
        for word in ("зум", "окей", "вау"):
            with self.subTest(word=word):
                self.assertIn(word, words)
        self.assertEqual(supplement_words("en_US"), ())
        self.assertIs(supplement_words("ru_RU"), supplement_words("ru_RU"))

    def test_language_model_and_reference_models_know_the_supplement_identically(self) -> None:
        engine = LanguageModel.load("ru_RU", supplement_words("ru_RU"))
        self.assertTrue(engine.score("зум").exact)
        self.assertTrue(engine.score("Окей").known)
        self.assertFalse(LanguageModel.load("ru_RU").score("зум").exact)
        from reference_lexicon import reference_models
        portable = reference_models(False)[1]
        self.assertTrue(portable.score("зум").exact)
        self.assertEqual(portable.score("зум").frequency, engine.score("зум").frequency)
        self.assertFalse(LanguageModel.load("en_US").score("зум").exact)

    def test_the_eight_single_letter_russian_words_are_carried(self) -> None:
        """They are words - "я" is the most frequent token of the source list - and a lone
        Latin letter is not, so this is the evidence that tells `z` from `я` apart."""
        words = set(supplement_words("ru_RU"))
        self.assertTrue({"а", "и", "с", "в", "к", "у", "о", "я"} <= words)
        portable = LanguageModel("ru_RU", {}, "test", enable_spellcheck=False)
        self.assertFalse(portable.score("я").known)
        english = LanguageModel("en_US", {}, "test", enable_spellcheck=False)
        self.assertFalse(english.score("z").known)

    def test_malformed_supplements_are_rejected_not_ignored(self) -> None:
        valid = {"schema_version": 1, "locale": "ru_RU", "words": ["абв", "где"]}
        for broken in ({**valid, "schema_version": 2}, {**valid, "locale": "en_US"}, {**valid, "words": ["где", "абв"]},
                       {**valid, "words": ["абв", "абв"]}, {**valid, "words": ["abc"]}, {**valid, "words": ["ы"]},  # a letter, not one of the eight words
                       {**valid, "words": ["аб1"]}, {**valid, "words": []}):
            with self.subTest(broken=broken):
                supplement_words.cache_clear()
                with patch.object(Path, "read_bytes", return_value=json.dumps(broken, ensure_ascii=False).encode()), \
                        patch.object(Path, "is_file", return_value=True):
                    with self.assertRaises(ValueError):
                        supplement_words("ru_RU")
        supplement_words.cache_clear()
        with patch("keyswitch.lexicon_supplement.MAX_SUPPLEMENT_BYTES", 16), self.assertRaises(ValueError):
            supplement_words("ru_RU")
        supplement_words.cache_clear()
        with patch.object(Path, "is_file", return_value=False):
            self.assertEqual(supplement_words("ru_RU"), ())
        supplement_words.cache_clear()
        self.assertTrue(supplement_words("ru_RU"))


if __name__ == "__main__":
    unittest.main()
