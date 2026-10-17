"""Words that look like a password reach the technical log as stars (keyswitch.log_privacy)."""

from __future__ import annotations

import unittest

from keyswitch.constants.log_files import LOG_SECRET_MIN_CHARACTERS, LOG_SECRET_REMEMBERED_WORDS
from keyswitch.log_privacy import MARKERS, LogRedactor, masked_text, secret_token


class LogPrivacyTests(unittest.TestCase):
    def test_a_long_token_of_letters_and_digits_is_a_secret(self) -> None:
        for token in ("parol123", "Qwerty12!", "(Config_v2)", "ключ2026года"):
            self.assertTrue(secret_token(token), token)
        # Short tokens, letters alone, digits alone and a version with no letter stay.
        for token in ("ipv6", "pm32", "password", "12345678", "0.56.0.1234", "a" * LOG_SECRET_MIN_CHARACTERS):
            self.assertFalse(secret_token(token), token)

    def test_the_word_after_a_marker_in_either_layout_is_masked_in_a_text(self) -> None:
        self.assertTrue({"пароль", "gfhjkm", "password", "зфыыцщкв", "gfhjk"} <= MARKERS)
        self.assertEqual(masked_text("мой пароль: тайна, ок"), "мой пароль: ****** ок")
        self.assertEqual(masked_text("gfhjkm  секрет"), "gfhjkm  ******")
        self.assertEqual(masked_text("Password Hunter2 и parol123"), "Password ******* и ********")
        # A marker at the end masks nothing in the same text, and a known secret is masked wherever it stands.
        self.assertEqual(masked_text("введите пароль"), "введите пароль")
        self.assertEqual(masked_text("тайна снова", frozenset({"тайна"})), "***** снова")

    def test_a_record_after_a_marker_hides_its_word_and_later_records_too(self) -> None:
        redactor = LogRedactor()
        first = redactor.record({"event": "word_evaluation", "original": "тайна", "alternatives": {0: "nfqyf"},
                                 "context": {"group": 1, "words": {0: "gfhjkm", 1: "пароль"}},
                                 "decision": {"replacement": "nfqyf", "model_version": "context-v3-20be427844f6"}})
        self.assertEqual(first["original"], "*****")
        self.assertEqual(first["alternatives"], {0: "*****"})
        decision = first["decision"]
        assert isinstance(decision, dict)
        # A version, an application's name and the event's name are no typed text.
        self.assertEqual(decision, {"replacement": "*****", "model_version": "context-v3-20be427844f6"})
        later = redactor.record({"event": "correction_applied", "original": "тайна", "application": "mail2024app",
                                 "words": ["тайна", "дальше"], "count": 1})
        self.assertEqual(later, {"event": "correction_applied", "original": "*****", "application": "mail2024app",
                                 "words": ["*****", "дальше"], "count": 1})
        # Words in lists are found at any depth: under a word key and under another key; a word key without text holds none.
        listed = redactor.record({"original": ("ключ",), "steps": [{"word": "шифр"}], "replacement": None,
                                  "context": {"words": {1: "пароль"}}})
        self.assertEqual((listed["original"], listed["steps"], listed["replacement"]), (["****"], [{"word": "****"}], None))
        # Without a marker before it a word stays, and the remembered words are the last few only.
        self.assertEqual(redactor.record({"original": "слово", "context": {"words": {1: "обычное", 0: ""}}})["original"], "слово")
        for index in range(LOG_SECRET_REMEMBERED_WORDS):
            redactor.record({"original": f"секрет{index}", "context": {"words": {1: "пароль"}}})
        self.assertEqual(redactor.record({"original": "тайна"})["original"], "тайна")
        self.assertEqual(redactor.record({"context": "none", "original": ("тайна",)})["original"], ["тайна"])


if __name__ == "__main__":
    unittest.main()
