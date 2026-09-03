"""Early layout switching from an unambiguous word prefix.

Ordinary correction waits for a word boundary or a typing pause.  Many wrong
layout words are, however, obvious after a few letters: no English word starts
with ``ghbd`` while hundreds of Russian words start with ``прив``.  This module
decides whether a typed prefix already proves the wrong layout so the engine
can switch and rewrite the prefix before the word is finished.

The rule is deliberately conservative and was calibrated on the frozen EN/RU
lexicons plus the Hunspell dictionaries: the prefix must have no continuation
at all in the source language (frequency lexicon and Hunspell stems) and must
not be a known word itself, while the alternative rendering must start many
frequent words of the other language.  On Hunspell-only words (the hardest
negatives) that yields roughly 0.01% false switches at four letters.
"""

from __future__ import annotations

import bisect
from collections.abc import Iterable, Mapping
from dataclasses import dataclass
from functools import lru_cache
from pathlib import Path
from typing import Protocol

from .language_model import LanguageModel, WordScore


class PrefixScorer(Protocol):
    """The subset of ``LanguageModel`` needed to judge a prefix."""

    def score(self, word: str) -> WordScore: ...


@dataclass(frozen=True)
class PrefixEvidence:
    """What the lexicon of one language says about a prefix."""

    completions: int
    maximum_frequency: int
    known: bool

    def as_dict(self) -> dict[str, object]:
        return {
            "completions": self.completions,
            "maximum_frequency": self.maximum_frequency,
            "known": self.known,
        }


@dataclass(frozen=True)
class EarlySwitchPolicy:
    """Thresholds; the defaults are the calibrated conservative values."""

    minimum_length: int = 4
    minimum_completions: int = 10
    minimum_frequency: int = 2000
    dominant_frequency: int = 100_000

    def as_dict(self) -> dict[str, object]:
        return {
            "minimum_length": self.minimum_length,
            "minimum_completions": self.minimum_completions,
            "minimum_frequency": self.minimum_frequency,
            "dominant_frequency": self.dominant_frequency,
        }


@dataclass(frozen=True)
class EarlySwitchDecision:
    should_switch: bool
    source_group: int
    target_group: int
    original: str
    replacement: str
    reason: str
    source_evidence: PrefixEvidence | None = None
    target_evidence: PrefixEvidence | None = None

    def as_dict(self) -> dict[str, object]:
        return {
            "should_switch": self.should_switch,
            "source_group": self.source_group,
            "target_group": self.target_group,
            "replacement": self.replacement,
            "reason": self.reason,
            "source_evidence": (
                None if self.source_evidence is None else self.source_evidence.as_dict()
            ),
            "target_evidence": (
                None if self.target_evidence is None else self.target_evidence.as_dict()
            ),
        }


class PrefixIndex:
    """Sorted case-folded words of one language with lexicon frequencies."""

    def __init__(self, words: Iterable[str], frequencies: Mapping[str, int]) -> None:
        self._words = sorted({word for word in words if word})
        self._frequencies = dict(frequencies)

    def __len__(self) -> int:
        return len(self._words)

    def completions(self, prefix: str, *, limit: int = 4096) -> PrefixEvidence:
        """Count words starting with ``prefix`` and their best frequency."""

        normalized = prefix.casefold()
        if not normalized:
            return PrefixEvidence(0, 0, False)
        start = bisect.bisect_left(self._words, normalized)
        stop = bisect.bisect_left(self._words, normalized + "￿")
        matches = self._words[start : min(stop, start + limit)]
        maximum = max((self._frequencies.get(word, 0) for word in matches), default=0)
        return PrefixEvidence(
            stop - start,
            maximum,
            normalized in self._frequencies or normalized in self._words,
        )

    @classmethod
    def for_language_model(cls, model: LanguageModel) -> PrefixIndex:
        """Build (once per lexicon source) from the lexicon plus Hunspell stems."""

        return _cached_index(model.locale, model.source, id(model.frequencies))


@lru_cache(maxsize=8)
def _cached_index(locale: str, source: str, _identity: int) -> PrefixIndex:
    model = LanguageModel.load(locale)
    words = set(model.frequencies)
    dictionary = _hunspell_dictionary_path(source)
    if dictionary is not None:
        words.update(_dictionary_stems(dictionary))
    return PrefixIndex(words, model.frequencies)


def _hunspell_dictionary_path(source: str) -> Path | None:
    for part in source.split(";"):
        part = part.strip()
        if part.startswith("Hunspell: "):
            candidate = Path(part.removeprefix("Hunspell: ").strip())
            if candidate.suffix == ".dic" and candidate.is_file():
                return candidate
    return None


def _dictionary_stems(path: Path, *, limit: int = 16 * 1024 * 1024) -> set[str]:
    """Read the alphabetic stems of a Hunspell ``.dic`` file (bounded)."""

    stems: set[str] = set()
    try:
        with path.open("rb") as stream:
            payload = stream.read(limit + 1)
    except OSError:
        return stems
    if len(payload) > limit:
        return stems
    for line in payload.decode("utf-8", "replace").splitlines()[1:]:
        stem = line.split("/", 1)[0].split("\t", 1)[0].strip().casefold()
        if stem.isalpha():
            stems.add(stem)
    return stems


def _looks_like_abbreviation(token: str) -> bool:
    """ALL-CAPS, camelCase and similar shapes are never switched early."""

    return any(character.isupper() for character in token[1:])


def early_switch_decision(
    indexes: Mapping[int, PrefixIndex],
    scorers: Mapping[int, PrefixScorer],
    original: str,
    alternatives: Mapping[int, str],
    source_group: int,
    *,
    policy: EarlySwitchPolicy = EarlySwitchPolicy(),
) -> EarlySwitchDecision:
    """Decide whether the typed prefix already proves the wrong layout."""

    candidates = [
        (group, candidate)
        for group, candidate in alternatives.items()
        if group != source_group and group in indexes and group in scorers
    ]
    rejected = EarlySwitchDecision(False, source_group, source_group, original, original, "")
    if source_group not in indexes or source_group not in scorers or not candidates:
        return _replace_reason(rejected, "нет другой раскладки")
    target_group, replacement = candidates[0]
    if len(original) < policy.minimum_length:
        return _replace_reason(rejected, "слишком короткий префикс")
    if not original.isalpha() or not replacement.isalpha():
        return _replace_reason(rejected, "префикс содержит не буквы")
    if _looks_like_abbreviation(original) or _looks_like_abbreviation(replacement):
        return _replace_reason(rejected, "похоже на сокращение или camelCase")
    source_evidence = indexes[source_group].completions(original)
    rejected = EarlySwitchDecision(
        False,
        source_group,
        target_group,
        original,
        replacement,
        "",
        source_evidence,
        None,
    )
    if source_evidence.completions > 0:
        return _replace_reason(rejected, "префикс продолжается в исходном языке")
    if source_evidence.known or scorers[source_group].score(original).known:
        return _replace_reason(rejected, "префикс сам является словом исходного языка")
    target_evidence = indexes[target_group].completions(replacement)
    rejected = EarlySwitchDecision(
        False,
        source_group,
        target_group,
        original,
        replacement,
        "",
        source_evidence,
        target_evidence,
    )
    strong = (
        target_evidence.completions >= policy.minimum_completions
        and target_evidence.maximum_frequency >= policy.minimum_frequency
    )
    dominant = target_evidence.maximum_frequency >= policy.dominant_frequency
    if not strong and not dominant:
        return _replace_reason(rejected, "целевой префикс не начинает частотных слов")
    return EarlySwitchDecision(
        True,
        source_group,
        target_group,
        original,
        replacement,
        (
            "целевой префикс доминирует по частоте"
            if dominant
            else "исходный префикс невозможен, целевой начинает частотные слова"
        ),
        source_evidence,
        target_evidence,
    )


def _replace_reason(decision: EarlySwitchDecision, reason: str) -> EarlySwitchDecision:
    return EarlySwitchDecision(
        decision.should_switch,
        decision.source_group,
        decision.target_group,
        decision.original,
        decision.replacement,
        reason,
        decision.source_evidence,
        decision.target_evidence,
    )
