"""Offline lexical and character language models for short EN/RU tokens."""

from __future__ import annotations

import math
import os
import statistics
from collections import Counter
from dataclasses import dataclass
from functools import lru_cache
from pathlib import Path
from typing import Iterable

from .spellcheck import HunspellDictionary


MODEL_ROOTS = (
    Path(__file__).resolve().parent / "resources" / "models",
    Path("/usr/share/onboard/models"),
    Path("/usr/local/share/onboard/models"),
)


def model_roots() -> tuple[Path, ...]:
    override = tuple(
        Path(item)
        for item in os.environ.get("KEYSWITCH_MODEL_PATH", "").split(os.pathsep)
        if item
    )
    return override + MODEL_ROOTS

LOCALE_FALLBACKS: dict[str, tuple[str, ...]] = {
    "en_US": (
        "hello", "world", "please", "thanks", "thank", "good", "morning",
        "evening", "today", "tomorrow", "keyboard", "layout", "switch",
        "application", "program", "settings", "language", "english", "test",
        "text", "word", "ubuntu", "computer", "message", "correct", "wrong",
        "github", "docker", "linux", "python", "terminal", "browser", "email",
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
    """Evidence that a decoded token belongs to one language."""

    value: float
    known: bool
    frequency: int
    gram_ratio: float
    exact: bool = False
    spell_known: bool = False
    ngram_score: float = 0.0
    invalid_ratio: float = 1.0


class LanguageModel:
    """Frequency lexicon + Hunspell morphology + smoothed character n-grams."""

    NGRAM_ORDERS = (2, 3, 4)

    def __init__(
        self,
        locale: str,
        frequencies: dict[str, int],
        source: str,
        bigrams: dict[tuple[str, str], int] | None = None,
    ) -> None:
        self.locale = locale
        self.frequencies = frequencies
        self.bigrams = bigrams or {}
        self.maximum = max(frequencies.values(), default=1)
        self.maximum_bigram = max(self.bigrams.values(), default=1)
        self.speller = HunspellDictionary(locale)
        sources = [source]
        if self.speller.available:
            sources.append(f"Hunspell: {self.speller.source}")
        self.source = "; ".join(sources)
        self._gram_counts = self._build_gram_counts(frequencies)
        self._gram_totals = {
            order: sum(counter.values()) for order, counter in self._gram_counts.items()
        }
        self._grams = set(self._gram_counts[3])
        calibration_words = [
            word
            for word, _frequency in sorted(
                frequencies.items(), key=lambda item: item[1], reverse=True
            )[:12_000]
            if len(word) >= 3 and word.isalpha()
        ]
        calibration = [self._raw_ngram_score(word) for word in calibration_words]
        self._ngram_mean = statistics.fmean(calibration) if calibration else -10.0
        self._ngram_deviation = statistics.pstdev(calibration) if len(calibration) > 1 else 1.0
        if self._ngram_deviation < 0.05:
            self._ngram_deviation = 1.0

    @classmethod
    def load(cls, locale: str, extra_words: Iterable[str] = ()) -> "LanguageModel":
        normalized_extra = tuple(
            sorted({cls.normalize(word) for word in extra_words if cls.normalize(word)})
        )
        return cls._load_cached(locale, normalized_extra)

    @staticmethod
    @lru_cache(maxsize=16)
    def _load_cached(locale: str, extra_words: tuple[str, ...]) -> "LanguageModel":
        path = next(
            (
                root / f"{locale}.lm"
                for root in model_roots()
                if (root / f"{locale}.lm").is_file()
            ),
            None,
        )
        frequencies: dict[str, int] = {}
        bigrams: dict[tuple[str, str], int] = {}
        source = "встроенный аварийный словарь"
        if path is not None:
            frequencies, bigrams = LanguageModel._read_arpa(path)
            source = str(path)
        synthetic_frequency = max(max(frequencies.values(), default=1000) // 20, 1000)
        for word in (*LOCALE_FALLBACKS.get(locale, ()), *extra_words):
            normalized = LanguageModel.normalize(word)
            if normalized:
                frequencies[normalized] = max(
                    frequencies.get(normalized, 0), synthetic_frequency
                )
        return LanguageModel(locale, frequencies, source, bigrams)

    @staticmethod
    def normalize(word: str) -> str:
        return "".join(
            character
            for character in word.casefold()
            if character.isalpha() or character in "'-"
        )

    @staticmethod
    def _read_arpa(path: Path) -> tuple[dict[str, int], dict[tuple[str, str], int]]:
        unigrams: dict[str, int] = {}
        bigrams: dict[tuple[str, str], int] = {}
        section = 0
        try:
            with path.open("r", encoding="utf-8", errors="replace") as handle:
                for raw_line in handle:
                    line = raw_line.rstrip("\n")
                    if line == r"\1-grams:":
                        section = 1
                        continue
                    if line == r"\2-grams:":
                        section = 2
                        continue
                    if line.startswith("\\"):
                        section = 0
                        continue
                    if section == 0 or not line:
                        continue
                    count_text, separator, payload = line.partition(" ")
                    if not separator or payload.startswith("<"):
                        continue
                    try:
                        count = int(count_text)
                    except ValueError:
                        continue
                    if section == 1:
                        token = LanguageModel.normalize(payload)
                        if len(token) >= 2:
                            unigrams[token] = unigrams.get(token, 0) + count
                    else:
                        # Section zero was skipped above and section one was
                        # handled by the preceding branch, so only bigrams
                        # remain here.
                        left, separator, right = payload.partition(" ")
                        if not separator:
                            continue
                        pair = (
                            LanguageModel.normalize(left),
                            LanguageModel.normalize(right),
                        )
                        if pair[0] and pair[1]:
                            bigrams[pair] = bigrams.get(pair, 0) + count
        except OSError:
            return {}, {}
        return unigrams, bigrams

    @staticmethod
    def _read_arpa_unigrams(path: Path) -> dict[str, int]:
        """Compatibility helper retained for callers and focused tests."""

        return LanguageModel._read_arpa(path)[0]

    @classmethod
    def _build_gram_counts(
        cls, frequencies: dict[str, int]
    ) -> dict[int, Counter[str]]:
        counters: dict[int, Counter[str]] = {
            order: Counter() for order in cls.NGRAM_ORDERS
        }
        for word, frequency in frequencies.items():
            if len(word) < 2 or not word.isalpha():
                continue
            # The source counts are highly skewed. Logarithmic weighting keeps
            # frequent words important without erasing legitimate rare forms.
            weight = max(1, min(32, int(math.log2(max(1, frequency))) + 1))
            padded = f"^{word}$"
            for order, counter in counters.items():
                for index in range(len(padded) - order + 1):
                    counter[padded[index : index + order]] += weight
        return counters

    @staticmethod
    def _build_grams(frequencies: dict[str, int]) -> set[str]:
        """Legacy helper used by older external tests."""

        return set(LanguageModel._build_gram_counts(frequencies)[3])

    def _raw_ngram_score(self, word: str) -> float:
        normalized = self.normalize(word)
        if not normalized:
            return -30.0
        padded = f"^{normalized}$"
        order_scores: list[float] = []
        for order in self.NGRAM_ORDERS:
            grams = [
                padded[index : index + order]
                for index in range(len(padded) - order + 1)
            ]
            if not grams:
                continue
            counter = self._gram_counts[order]
            total = self._gram_totals[order]
            vocabulary = len(counter) + 2048
            alpha = 0.2
            order_scores.append(
                sum(
                    math.log((counter.get(gram, 0) + alpha) / (total + alpha * vocabulary))
                    for gram in grams
                )
                / len(grams)
            )
        return statistics.fmean(order_scores) if order_scores else -30.0

    def ngram_score(self, word: str) -> float:
        return (self._raw_ngram_score(word) - self._ngram_mean) / self._ngram_deviation

    def context_score(self, previous_word: str, word: str) -> float:
        pair = (self.normalize(previous_word), self.normalize(word))
        frequency = self.bigrams.get(pair, 0)
        if not frequency:
            return 0.0
        return math.log1p(frequency) / math.log1p(self.maximum_bigram)

    def best_single_deletion(self, word: str, limit: int = 12) -> WordScore:
        """Return the strongest score after dropping one likely typo character."""

        if len(word) < 4:
            return self.score("")
        indices = list(range(len(word)))
        if len(indices) > limit:
            indices = sorted(
                {
                    round(index * (len(word) - 1) / (limit - 1))
                    for index in range(limit)
                }
            )
        return max(
            (self.score(word[:index] + word[index + 1 :]) for index in indices),
            key=lambda item: item.value,
        )

    def score(self, word: str) -> WordScore:
        normalized = self.normalize(word)
        if not normalized:
            return WordScore(-30.0, False, 0, 0.0, ngram_score=-15.0)
        lexical_eligible = all(
            character.isalpha() or character in "'-" for character in word
        )
        frequency = self.frequencies.get(normalized, 0) if lexical_eligible else 0
        exact = bool(frequency)
        spell_known = (
            self.speller.check(normalized)
            if lexical_eligible and normalized.isalpha()
            else False
        )
        known = exact or spell_known
        padded = f"^{normalized}$"
        trigrams = [
            padded[index : index + 3]
            for index in range(max(0, len(padded) - 2))
        ]
        hits = sum(gram in self._gram_counts[3] for gram in trigrams)
        ratio = hits / len(trigrams) if trigrams else 0.0
        invalid_ratio = 1.0 - ratio
        naturalness = max(-15.0, min(4.0, self.ngram_score(normalized)))
        lexical = 0.0
        if exact:
            popularity = math.log1p(frequency) / math.log1p(self.maximum)
            lexical = 7.0 + 3.0 * popularity
        elif spell_known:
            lexical = 6.5
        if known:
            naturalness = max(naturalness, -4.0)
        value = lexical + 1.15 * naturalness - 0.75 * invalid_ratio
        return WordScore(
            value,
            known,
            frequency,
            ratio,
            exact,
            spell_known,
            naturalness,
            invalid_ratio,
        )
