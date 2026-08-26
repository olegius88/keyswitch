"""Small local vocabulary model backed by Ubuntu Onboard ARPA files."""

from __future__ import annotations

import math
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable


MODEL_ROOTS = (
    Path("/usr/share/onboard/models"),
    Path("/usr/local/share/onboard/models"),
)

LOCALE_FALLBACKS: dict[str, tuple[str, ...]] = {
    "en_US": (
        "hello", "world", "please", "thanks", "thank", "good", "morning",
        "evening", "today", "tomorrow", "keyboard", "layout", "switch",
        "application", "program", "settings", "language", "english", "test",
        "text", "word", "ubuntu", "computer", "message", "correct", "wrong",
    ),
    "ru_RU": (
        "привет", "пока", "спасибо", "пожалуйста", "хорошо", "здравствуйте",
        "сегодня", "завтра", "клавиатура", "раскладка", "переключить",
        "приложение", "программа", "настройки", "язык", "русский", "тест",
        "текст", "слово", "убунту", "компьютер", "сообщение", "исправить",
        "ошибка", "работает", "работа", "можно", "нужно", "когда", "будет",
        "очень", "этот", "эта", "это", "мой", "моя", "для", "как", "что",
    ),
}


@dataclass(frozen=True)
class WordScore:
    value: float
    known: bool
    frequency: int
    gram_ratio: float


class LanguageModel:
    def __init__(self, locale: str, frequencies: dict[str, int], source: str) -> None:
        self.locale = locale
        self.frequencies = frequencies
        self.source = source
        self.maximum = max(frequencies.values(), default=1)
        self._grams = self._build_grams(frequencies)

    @classmethod
    def load(cls, locale: str, extra_words: Iterable[str] = ()) -> "LanguageModel":
        path = next(
            (root / f"{locale}.lm" for root in MODEL_ROOTS if (root / f"{locale}.lm").is_file()),
            None,
        )
        frequencies: dict[str, int] = {}
        source = "встроенный словарь"
        if path is not None:
            frequencies = cls._read_arpa_unigrams(path)
            source = str(path)
        synthetic_frequency = max(max(frequencies.values(), default=1000) // 20, 1000)
        for word in (*LOCALE_FALLBACKS.get(locale, ()), *tuple(extra_words)):
            normalized = cls.normalize(word)
            if normalized:
                frequencies[normalized] = max(frequencies.get(normalized, 0), synthetic_frequency)
        return cls(locale, frequencies, source)

    @staticmethod
    def normalize(word: str) -> str:
        return "".join(character for character in word.casefold() if character.isalpha() or character in "'-")

    @staticmethod
    def _read_arpa_unigrams(path: Path) -> dict[str, int]:
        result: dict[str, int] = {}
        in_unigrams = False
        try:
            with path.open("r", encoding="utf-8", errors="replace") as handle:
                for raw_line in handle:
                    line = raw_line.rstrip("\n")
                    if line == r"\1-grams:":
                        in_unigrams = True
                        continue
                    if in_unigrams and line.startswith("\\"):
                        break
                    if not in_unigrams or not line:
                        continue
                    count_text, separator, token = line.partition(" ")
                    if not separator or token.startswith("<"):
                        continue
                    try:
                        count = int(count_text)
                    except ValueError:
                        continue
                    normalized = LanguageModel.normalize(token)
                    if len(normalized) >= 2:
                        result[normalized] = result.get(normalized, 0) + count
        except OSError:
            return {}
        return result

    @staticmethod
    def _build_grams(frequencies: dict[str, int]) -> set[str]:
        grams: set[str] = set()
        for word in frequencies:
            if len(word) < 3:
                continue
            padded = f"^{word}$"
            grams.update(padded[index : index + 3] for index in range(len(padded) - 2))
        return grams

    def score(self, word: str) -> WordScore:
        normalized = self.normalize(word)
        if not normalized:
            return WordScore(-20.0, False, 0, 0.0)
        frequency = self.frequencies.get(normalized, 0)
        padded = f"^{normalized}$"
        grams = [padded[index : index + 3] for index in range(max(0, len(padded) - 2))]
        hits = sum(gram in self._grams for gram in grams)
        ratio = hits / len(grams) if grams else 0.0
        if frequency:
            popularity = math.log1p(frequency) / math.log1p(self.maximum)
            return WordScore(5.0 + 4.0 * popularity + ratio, True, frequency, ratio)
        return WordScore(-2.0 + 3.0 * ratio, False, 0, ratio)
