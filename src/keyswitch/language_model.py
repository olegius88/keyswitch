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

from .constants.file_formats import ARPA_BIGRAM_SECTION
from .constants.models import (
    LANGUAGE_MODEL_CACHE_SIZE,
    LANGUAGE_MODEL_CALIBRATION_MIN_CHARACTERS,
    LANGUAGE_MODEL_CALIBRATION_WORD_LIMIT,
    LANGUAGE_MODEL_DEFAULT_NGRAM_MEAN,
    LANGUAGE_MODEL_DELETION_MIN_CHARACTERS,
    LANGUAGE_MODEL_DELETION_POSITIONS,
    LANGUAGE_MODEL_EMPTY_TOKEN_SCORE,
    LANGUAGE_MODEL_EXACT_WORD_BASE_SCORE,
    LANGUAGE_MODEL_GRAM_SMOOTHING,
    LANGUAGE_MODEL_INVALID_RATIO_WEIGHT,
    LANGUAGE_MODEL_KNOWN_WORD_NATURALNESS_FLOOR,
    LANGUAGE_MODEL_MAX_GRAM_WEIGHT,
    LANGUAGE_MODEL_MIN_NGRAM_DEVIATION,
    LANGUAGE_MODEL_MIN_WORD_CHARACTERS,
    LANGUAGE_MODEL_NATURALNESS_MAX,
    LANGUAGE_MODEL_NATURALNESS_MIN,
    LANGUAGE_MODEL_NATURALNESS_WEIGHT,
    LANGUAGE_MODEL_NGRAM_ORDERS,
    LANGUAGE_MODEL_POPULARITY_WEIGHT,
    LANGUAGE_MODEL_SCORE_CACHE_MAXSIZE,
    LANGUAGE_MODEL_SPELL_KNOWN_SCORE,
    LANGUAGE_MODEL_TRIGRAM_ORDER,
    LANGUAGE_MODEL_UNSEEN_GRAM_VOCABULARY,
    SYNTHETIC_FREQUENCY_DIVISOR,
    SYNTHETIC_FREQUENCY_FLOOR,
)
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
    raw_ngram_score: float = LANGUAGE_MODEL_NATURALNESS_MIN


class _DisabledSpellChecker:
    """Deterministic no-op spell checker for sealed offline model training."""

    available = False
    source = ""

    @staticmethod
    def check(_word: str) -> bool:
        return False


class LanguageModel:
    """Frequency lexicon + Hunspell morphology + smoothed character n-grams."""

    def __init__(
        self,
        locale: str,
        frequencies: dict[str, int],
        source: str,
        bigrams: dict[tuple[str, str], int] | None = None,
        *,
        enable_spellcheck: bool = True,
    ) -> None:
        self.locale = locale
        self.frequencies = frequencies
        self.bigrams = bigrams or {}
        self.maximum = max(frequencies.values(), default=1)
        self.maximum_bigram = max(self.bigrams.values(), default=1)
        self.speller: HunspellDictionary | _DisabledSpellChecker = (
            HunspellDictionary(locale)
            if enable_spellcheck
            else _DisabledSpellChecker()
        )
        sources = [source]
        if self.speller.available:
            sources.append(f"Hunspell: {self.speller.source}")
        self.source = "; ".join(sources)
        self._gram_counts = self._build_gram_counts(frequencies)
        self._gram_totals = {
            order: sum(counter.values()) for order, counter in self._gram_counts.items()
        }
        self._grams = set(self._gram_counts[LANGUAGE_MODEL_TRIGRAM_ORDER])
        calibration_words = [
            word
            for word, _frequency in sorted(
                frequencies.items(), key=lambda item: (-item[1], item[0])
            )[:LANGUAGE_MODEL_CALIBRATION_WORD_LIMIT]
            if len(word) >= LANGUAGE_MODEL_CALIBRATION_MIN_CHARACTERS and word.isalpha()
        ]
        calibration = [self._raw_ngram_score(word) for word in calibration_words]
        self._ngram_mean = (
            statistics.fmean(calibration) if calibration else LANGUAGE_MODEL_DEFAULT_NGRAM_MEAN
        )
        self._ngram_deviation = statistics.pstdev(calibration) if len(calibration) > 1 else 1.0
        if self._ngram_deviation < LANGUAGE_MODEL_MIN_NGRAM_DEVIATION:
            self._ngram_deviation = 1.0

    @classmethod
    def load(cls, locale: str, extra_words: Iterable[str] = ()) -> "LanguageModel":
        normalized_extra = tuple(
            sorted({cls.normalize(word) for word in extra_words if cls.normalize(word)})
        )
        return cls._load_cached(locale, normalized_extra)

    @staticmethod
    @lru_cache(maxsize=LANGUAGE_MODEL_CACHE_SIZE)
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
        synthetic_frequency = max(
            max(frequencies.values(), default=SYNTHETIC_FREQUENCY_FLOOR) // SYNTHETIC_FREQUENCY_DIVISOR,
            SYNTHETIC_FREQUENCY_FLOOR,
        )
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
                        section = ARPA_BIGRAM_SECTION
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
                        if len(token) >= LANGUAGE_MODEL_MIN_WORD_CHARACTERS:
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
            order: Counter() for order in LANGUAGE_MODEL_NGRAM_ORDERS
        }
        for word, frequency in frequencies.items():
            if len(word) < LANGUAGE_MODEL_MIN_WORD_CHARACTERS or not word.isalpha():
                continue
            # The source counts are highly skewed. Logarithmic weighting keeps
            # frequent words important without erasing legitimate rare forms.
            weight = max(1, min(LANGUAGE_MODEL_MAX_GRAM_WEIGHT, int(math.log2(max(1, frequency))) + 1))
            padded = f"^{word}$"
            for order, counter in counters.items():
                for index in range(len(padded) - order + 1):
                    counter[padded[index : index + order]] += weight
        return counters

    @staticmethod
    def _build_grams(frequencies: dict[str, int]) -> set[str]:
        """Legacy helper used by older external tests."""

        return set(LanguageModel._build_gram_counts(frequencies)[LANGUAGE_MODEL_TRIGRAM_ORDER])

    def _raw_ngram_score(self, word: str) -> float:
        normalized = self.normalize(word)
        if not normalized:
            return LANGUAGE_MODEL_EMPTY_TOKEN_SCORE
        padded = f"^{normalized}$"
        order_scores: list[float] = []
        for order in LANGUAGE_MODEL_NGRAM_ORDERS:
            grams = [
                padded[index : index + order]
                for index in range(len(padded) - order + 1)
            ]
            if not grams:
                continue
            counter = self._gram_counts[order]
            total = self._gram_totals[order]
            vocabulary = len(counter) + LANGUAGE_MODEL_UNSEEN_GRAM_VOCABULARY
            alpha = LANGUAGE_MODEL_GRAM_SMOOTHING
            order_scores.append(
                sum(
                    math.log((counter.get(gram, 0) + alpha) / (total + alpha * vocabulary))
                    for gram in grams
                )
                / len(grams)
            )
        return statistics.fmean(order_scores) if order_scores else LANGUAGE_MODEL_EMPTY_TOKEN_SCORE

    def ngram_score(self, word: str) -> float:
        return (self._raw_ngram_score(word) - self._ngram_mean) / self._ngram_deviation

    def context_score(self, previous_word: str, word: str) -> float:
        pair = (self.normalize(previous_word), self.normalize(word))
        frequency = self.bigrams.get(pair, 0)
        if not frequency:
            return 0.0
        return math.log1p(frequency) / math.log1p(self.maximum_bigram)

    def best_single_deletion(
        self, word: str, limit: int = LANGUAGE_MODEL_DELETION_POSITIONS
    ) -> WordScore:
        """Return the strongest score after dropping one likely typo character."""

        if len(word) < LANGUAGE_MODEL_DELETION_MIN_CHARACTERS:
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

    @lru_cache(maxsize=LANGUAGE_MODEL_SCORE_CACHE_MAXSIZE)
    def score(self, word: str) -> WordScore:
        """Score a token with a bounded cache shared by runtime and training."""

        normalized = self.normalize(word)
        if not normalized:
            return WordScore(
                LANGUAGE_MODEL_EMPTY_TOKEN_SCORE,
                False,
                0,
                0.0,
                ngram_score=LANGUAGE_MODEL_NATURALNESS_MIN,
            )
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
            padded[index : index + LANGUAGE_MODEL_TRIGRAM_ORDER]
            for index in range(max(0, len(padded) - (LANGUAGE_MODEL_TRIGRAM_ORDER - 1)))
        ]
        hits = sum(gram in self._gram_counts[LANGUAGE_MODEL_TRIGRAM_ORDER] for gram in trigrams)
        ratio = hits / len(trigrams) if trigrams else 0.0
        invalid_ratio = 1.0 - ratio
        raw_naturalness = max(
            LANGUAGE_MODEL_NATURALNESS_MIN,
            min(LANGUAGE_MODEL_NATURALNESS_MAX, self.ngram_score(normalized)),
        )
        naturalness = raw_naturalness
        lexical = 0.0
        if exact:
            popularity = math.log1p(frequency) / math.log1p(self.maximum)
            lexical = (
                LANGUAGE_MODEL_EXACT_WORD_BASE_SCORE
                + LANGUAGE_MODEL_POPULARITY_WEIGHT * popularity
            )
        elif spell_known:
            lexical = LANGUAGE_MODEL_SPELL_KNOWN_SCORE
        if known:
            naturalness = max(naturalness, LANGUAGE_MODEL_KNOWN_WORD_NATURALNESS_FLOOR)
        value = (
            lexical
            + LANGUAGE_MODEL_NATURALNESS_WEIGHT * naturalness
            - LANGUAGE_MODEL_INVALID_RATIO_WEIGHT * invalid_ratio
        )
        return WordScore(
            value,
            known,
            frequency,
            ratio,
            exact,
            spell_known,
            naturalness,
            invalid_ratio,
            raw_naturalness,
        )
