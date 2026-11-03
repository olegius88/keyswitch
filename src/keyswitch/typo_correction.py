"""Typo correction at a word's boundary: one keyboard slip, undone where a counted word list makes it clear.

A word is corrected only when nothing reads it as a word: its form never occurs in the counted list of its
language (OpenSubtitles 2018 via FrequencyWords, every form down to one occurrence, so a rare inflected form
survives), its other layout's reading occurs in neither list, and the engine knows it from nowhere else (its
lexicons, the term tables, the identifier index, the user's exclusions). The candidates are the words one slip away:
two neighbouring letters swapped, a letter left out, a letter typed in excess, a letter typed with a neighbouring key.
Each is scored as a noisy channel, the logarithm of its count plus that of its slip, and the best one is taken only
when it is frequent enough and far more likely than every other candidate together. The decision reads nothing a
platform provides (no spell checker), so it is the same on every system.

The correction types keys, not text: a swap or a deletion reuses the keys the user typed, with their Shift and Caps
Lock, and a letter not pressed in the word takes the key of the last press that typed it (KeyTemplates). A letter no
key has typed yet cannot be typed, and its candidate is not made.
"""

from __future__ import annotations

import hashlib
import itertools
import json
import math
import re
import zlib
from array import array
from collections.abc import Callable, Collection, Iterable, Iterator, Mapping, Sequence
from dataclasses import dataclass, replace
from functools import lru_cache
from pathlib import Path
from typing import Final, Literal

from .backend import KeyEvent
from .constants.typo import (
    KEYBOARD_ROW_LENGTHS,
    TYPO_ALPHABETS,
    TYPO_CANDIDATE_MIN_COUNT,
    TYPO_COUNT_LOG_SCALE,
    TYPO_DELETION_LOG_PROBABILITY,
    TYPO_FORM_SEPARATOR,
    TYPO_FREQUENCY_MAX_BYTES,
    TYPO_FREQUENCY_SCHEMA_VERSION,
    TYPO_INSERTION_LOG_PROBABILITY,
    TYPO_MIN_LETTERS,
    TYPO_OCCURRENCE_HASH_BYTES,
    TYPO_POSTERIOR_RATIO,
    TYPO_SECTIONS,
    TYPO_SUBSTITUTION_LOG_PROBABILITY,
    TYPO_TRANSPOSITION_LOG_PROBABILITY,
    TYPO_TRANSPOSED_LETTERS,
)
from .constants.file_formats import VERSION_HASH_CHARACTERS
from .constants.units import BITS_PER_BYTE
from .detector import LanguageDetector
from .identifier_lexicon import IdentifierLexicon
from .language_model import LanguageModel
from .layouts import RU_KEYS, US_KEYS
from .russian_text import LETTERS, quantity

TYPO_RESOURCE_ROOT: Final = Path(__file__).parent / "resources"
# What both settings windows say of detection.correct_typos.
CORRECT_TYPOS_TITLE: Final = "Исправлять опечатки"
CORRECT_TYPOS_ENGLISH_TITLE: Final = "Исправлять опечатки и в английских словах"
CORRECT_TYPOS_ENGLISH_DESCRIPTION: Final = (
    "По умолчанию исправляются только русские слова: в технических текстах и коде незнакомое английское слово чаще "
    "оказывается именем переменной или термином, чем опечаткой"
)
CORRECT_TYPOS_DESCRIPTION: Final = (
    "Исправлять в слове одну опечатку: переставленные буквы, лишнюю, пропущенную или соседнюю клавишу. Только если "
    "ни один словарь не знает слово, а одно частое слово намного вероятнее других; слова, в которых меньше "
    f"{quantity(TYPO_MIN_LETTERS, LETTERS)}, и имена в середине предложения не трогаются. Pause или сочетание отмены "
    "сразу после исправления возвращает слово как набрано, и больше оно не исправляется; не исправляется и слово, "
    "которое не раз оставлено как набрано. Такие слова хранятся только как хеши и только при включённой истории")
# The physical keys each locale's letters are typed with, in the order of layouts.US_KEYS.
LOCALE_KEYS: Final = {"en_US": US_KEYS, "ru_RU": RU_KEYS}
# Russian prints `ё` as `е` as often as not: a form occurs if either spelling does.
_YO: Final = "ё"
_YE: Final = "е"

Operation = Literal["transposition", "deletion", "insertion", "substitution"]
OPERATIONS: Final[tuple[Operation, ...]] = ("transposition", "deletion", "insertion", "substitution")


@dataclass(frozen=True)
class TypoPolicy:
    """The decision's parameters; the shipped ones are the constants, and the measurement tool tries others."""

    candidate_min_count: int = TYPO_CANDIDATE_MIN_COUNT
    posterior_ratio: float = TYPO_POSTERIOR_RATIO
    log_probabilities: tuple[tuple[Operation, float], ...] = (
        ("transposition", TYPO_TRANSPOSITION_LOG_PROBABILITY),
        ("deletion", TYPO_DELETION_LOG_PROBABILITY),
        ("insertion", TYPO_INSERTION_LOG_PROBABILITY),
        ("substitution", TYPO_SUBSTITUTION_LOG_PROBABILITY),
    )

    @property
    def minimum_weight(self) -> int:
        """The stored weight of `candidate_min_count`, the least a corrected word may have."""

        return round(math.log(self.candidate_min_count) * TYPO_COUNT_LOG_SCALE)


def typo_resource_path(locale: str) -> Path:
    return TYPO_RESOURCE_ROOT / f"typo-frequency-{locale}.bin"


def keyboard_neighbours(keys: str) -> dict[str, str]:
    """The letters of the keys around each letter's key, in a layout whose keys `keys` lists (layouts.US_KEYS order).

    Keys of one row touch their left and right neighbours. A lower row sits half a key to the right of the row above
    it, so a key touches the key right above it and the one after that, and the key right below it and the one before
    that. The key of the top row (left of `1`) touches no letter. ЙЦУКЕН and QWERTY are the same keys, so one table
    of positions serves both layouts.
    """

    rows: list[str] = []
    start = 0
    for length in KEYBOARD_ROW_LENGTHS:
        rows.append(keys[start:start + length])
        start += length
    if start != len(keys):
        raise ValueError("the key rows do not cover the layout's keys")
    letter_rows = rows[1:]
    neighbours: dict[str, str] = {}
    for row_index, row in enumerate(letter_rows):
        for index, key in enumerate(row):
            near = [row[position] for position in (index - 1, index + 1) if 0 <= position < len(row)]
            if row_index > 0:
                above = letter_rows[row_index - 1]
                near.extend(above[position] for position in (index, index + 1) if position < len(above))
            if row_index + 1 < len(letter_rows):
                below = letter_rows[row_index + 1]
                near.extend(below[position] for position in (index - 1, index) if 0 <= position < len(below))
            if key.isalpha():
                neighbours[key] = "".join(sorted(character for character in near if character.isalpha()))
    return neighbours


@dataclass(frozen=True)
class TypoLexicon:
    """A counted word list of one language: forms with their counts, and a membership filter for the rarer forms.

    `forms` holds the stored forms sorted, each coded one byte per letter (its place in `alphabet`, from one up) and
    ended by TYPO_FORM_SEPARATOR; `starts` the offset of each, with one more for the end; `weights` each form's count
    as `round(ln(count) * TYPO_COUNT_LOG_SCALE)`. `seen` is a Bloom filter of `seen_bits` bits and `seen_hashes`
    positions over the forms too rare to be stored: it may hold a form it was not given, never miss one it was.
    """

    locale: str
    alphabet: str
    forms: bytes
    starts: array[int]
    weights: bytes
    seen: bytes
    seen_bits: int
    seen_hashes: int
    version: str

    @property
    def size(self) -> int:
        return len(self.weights)

    def _coded(self, form: str) -> bytes | None:
        try:
            coded = form.translate(_letter_codes(self.alphabet)).encode("latin-1")
        except UnicodeEncodeError:
            return None
        if not coded or max(coded) > len(self.alphabet):
            return None
        return coded

    def weight(self, form: str) -> int | None:
        """The stored count of a lowercase form as its scaled logarithm, or None when it is not stored."""

        coded = self._coded(form)
        if coded is None:
            return None
        low, high = 0, self.size
        while low < high:
            middle = (low + high) >> 1
            stored = self.forms[self.starts[middle]:self.starts[middle + 1] - 1]
            if stored == coded:
                return self.weights[middle]
            if stored < coded:
                low = middle + 1
            else:
                high = middle
        return None

    def occurs(self, form: str) -> bool:
        """Whether a lowercase form occurs in the list at all, written with `ё` or with `е`."""

        return any(self._occurs(spelling) for spelling in dict.fromkeys((form, form.replace(_YO, _YE))))

    def _occurs(self, form: str) -> bool:
        if self.weight(form) is not None:
            return True
        if self._coded(form) is None:
            return False
        return all(
            self.seen[position // BITS_PER_BYTE] & (1 << position % BITS_PER_BYTE)
            for position in occurrence_positions(form, self.seen_bits, self.seen_hashes)
        )

    @classmethod
    def load(cls, path: Path, locale: str) -> TypoLexicon:
        with path.open("rb") as handle:
            raw = handle.read(TYPO_FREQUENCY_MAX_BYTES + 1)
        if len(raw) > TYPO_FREQUENCY_MAX_BYTES:
            raise ValueError("typo frequency file is too large")
        header_line, separator, payload = raw.partition(b"\n")
        if not separator:
            raise ValueError("typo frequency file has no header")
        header: object = json.loads(header_line)
        if (not isinstance(header, dict) or header.get("schema_version") != TYPO_FREQUENCY_SCHEMA_VERSION
                or header.get("locale") != locale or header.get("alphabet") != TYPO_ALPHABETS.get(locale)
                or header.get("count_log_scale") != TYPO_COUNT_LOG_SCALE
                or header.get("payload_sha256") != hashlib.sha256(payload).hexdigest()):
            raise ValueError("invalid typo frequency header")
        sections = header.get("sections")
        seen = header.get("seen")
        if not isinstance(sections, dict) or not isinstance(seen, dict):
            raise ValueError("invalid typo frequency header")
        sizes = [size for name in TYPO_SECTIONS if type(size := sections.get(name)) is int and size >= 0]
        bits, hashes = seen.get("bits"), seen.get("hashes")
        if (len(sizes) != len(TYPO_SECTIONS) or sum(sizes) != len(payload)
                or type(bits) is not int or type(hashes) is not int or bits < 1 or hashes < 1):
            raise ValueError("invalid typo frequency sections")
        ends = list(itertools.accumulate(sizes))
        forms = zlib.decompress(payload[:ends[0]])
        weights = zlib.decompress(payload[ends[0]:ends[1]])
        filter_bits = payload[ends[1]:]
        starts = array("I", [0])
        starts.extend(match.end() for match in re.finditer(bytes((TYPO_FORM_SEPARATOR,)), forms))
        if (len(starts) - 1 != len(weights) or (forms and forms[-1] != TYPO_FORM_SEPARATOR)
                or len(filter_bits) * BITS_PER_BYTE < bits):
            raise ValueError("invalid typo frequency payload")
        return cls(locale, str(header["alphabet"]), forms, starts, weights, filter_bits, bits, hashes,
                   "typo-frequency-" + hashlib.sha256(raw).hexdigest()[:VERSION_HASH_CHARACTERS])


@lru_cache(maxsize=len(TYPO_ALPHABETS))
def _letter_codes(alphabet: str) -> dict[int, str]:
    """The translation of each letter to its code in stored forms: its place in the alphabet, from one up
    (TYPO_FORM_SEPARATOR is below). The codes stay below every character that is no letter of the alphabet."""

    return str.maketrans({letter: chr(index) for index, letter in enumerate(alphabet, 1)})


def occurrence_positions(form: str, bits: int, hashes: int) -> Iterator[int]:
    """The filter positions of a form: two hashes from its SHA-256, combined (Kirsch and Mitzenmacher)."""

    digest = hashlib.sha256(form.encode("utf-8")).digest()
    first = int.from_bytes(digest[:TYPO_OCCURRENCE_HASH_BYTES], "little")
    step = int.from_bytes(digest[TYPO_OCCURRENCE_HASH_BYTES:TYPO_OCCURRENCE_HASH_BYTES + TYPO_OCCURRENCE_HASH_BYTES],
                          "little") | 1
    for index in range(hashes):
        yield (first + index * step) % bits


@lru_cache(maxsize=len(TYPO_ALPHABETS))
def packaged_lexicon(locale: str) -> TypoLexicon | None:
    """The shipped list of a locale, loaded once per process; None when none is shipped or it does not load."""

    path = typo_resource_path(locale)
    if locale not in TYPO_ALPHABETS or not path.is_file():
        return None
    try:
        return TypoLexicon.load(path, locale)
    except (OSError, ValueError):
        return None


@dataclass(frozen=True)
class Edit:
    """One slip undone: the operation, where it applies in the typed word, and the letter it brings, if any."""

    operation: Operation
    position: int
    letter: str = ""

    def apply(self, word: str) -> str:
        position = self.position
        if self.operation == "transposition":
            return word[:position] + word[position + 1] + word[position] + word[position + TYPO_TRANSPOSED_LETTERS:]
        if self.operation == "deletion":
            return word[:position] + word[position + 1:]
        if self.operation == "insertion":
            return word[:position] + self.letter + word[position:]
        return word[:position] + self.letter + word[position + 1:]

    def sources(self, length: int) -> list[int | None]:
        """For each letter of the corrected word, the typed letter it is, or None for the letter the edit brings."""

        order: list[int | None] = list(range(length))
        position = self.position
        if self.operation == "transposition":
            order[position], order[position + 1] = order[position + 1], order[position]
        elif self.operation == "deletion":
            del order[position]
        elif self.operation == "insertion":
            order.insert(position, None)
        else:
            order[position] = None
        return order


def edits(word: str, alphabet: str, neighbours: Mapping[str, str]) -> Iterator[Edit]:
    """Every single slip that could have turned another word into `word` (lowercase), each undone once.

    A letter typed in excess is a key pressed twice or one brushed beside the key meant: it repeats a letter next to it
    or lies on a key next to that letter's. Any other extra letter is no slip of the fingers but part of the word: an
    identifier's prefix (`nlevel`, `bweight`) or a rare inflection (`эстафетах`), which the development texts showed
    corrected away. A letter left out may be any letter, and a letter typed for another lies on a neighbouring key.
    """

    for position in range(len(word) - 1):
        if word[position] != word[position + 1]:
            yield Edit("transposition", position)
    for position, extra in enumerate(word):
        around = [word[index] for index in (position - 1, position + 1) if 0 <= index < len(word)]
        if any(extra == letter or extra in neighbours.get(letter, "") for letter in around):
            yield Edit("deletion", position)
    for position in range(len(word) + 1):
        for letter in alphabet:
            yield Edit("insertion", position, letter)
    for position, typed in enumerate(word):
        for letter in neighbours.get(typed, ""):
            yield Edit("substitution", position, letter)


@dataclass(frozen=True)
class Candidate:
    """A stored word one slip away from the typed one: its weight and every slip that turns it into the typed word."""

    word: str
    weight: int
    edits: tuple[Edit, ...]


Reason = Literal["shape", "capital", "occurs", "other_reading", "known", "no_candidate", "rare", "ambiguous", "corrected"]


@dataclass(frozen=True)
class TypoChoice:
    """The candidate a word is corrected to (cased as the word was), how, and how clearly.

    `weight` is the stored count's scaled logarithm, `candidates` how many words one slip away the list holds, and
    `margin` the natural logarithm of the best candidate's posterior over the sum of all the others' (infinite when
    there are none).
    """

    word: str
    edit: Edit
    weight: int
    candidates: int
    margin: float


@dataclass(frozen=True)
class TypoJudgement:
    reason: Reason
    choice: TypoChoice | None = None
    candidates: int = 0

    @property
    def count_order(self) -> int:
        """The decimal order of magnitude of the chosen candidate's count, for the log; -1 without one."""

        if self.choice is None:
            return -1
        return math.floor(math.log10(math.exp(self.choice.weight / TYPO_COUNT_LOG_SCALE)))


def word_shape(word: str, alphabet: str) -> bool:
    """Letters of the alphabet only, at least TYPO_MIN_LETTERS of them, lowercase or with one leading capital."""

    lower = word.casefold()
    return (len(word) >= TYPO_MIN_LETTERS and all(character in alphabet for character in lower)
            and word[1:] == lower[1:] and len(lower) == len(word))


class TypoCorrector:
    """Judges words of the languages it has lists for; the engine asks it at a word's boundary."""

    def __init__(self, lexicons: Mapping[str, TypoLexicon], policy: TypoPolicy = TypoPolicy()) -> None:
        self.lexicons = dict(lexicons)
        self.policy = policy
        self._log_probabilities = dict(policy.log_probabilities)
        self.neighbours = {locale: keyboard_neighbours(LOCALE_KEYS[locale]) for locale in self.lexicons}

    @classmethod
    def packaged(cls, locales: Sequence[str]) -> TypoCorrector:
        lexicons = {locale: lexicon for locale in locales if (lexicon := packaged_lexicon(locale)) is not None}
        return cls(lexicons)

    def judge(
        self, word: str, locale: str, readings: Mapping[str, str], *, capital_allowed: bool,
        known: Callable[[str], bool],
    ) -> TypoJudgement:
        """Whether to correct `word`, typed in the layout of `locale`, and to what.

        `readings` are the same keys read in the other layouts, by locale; `capital_allowed` says whether a capital
        may open the word here (the start of a message or of a sentence; a capital in the middle of one is a name);
        `known` says whether the engine knows a reading from elsewhere.
        """

        lexicon = self.lexicons.get(locale)
        if lexicon is None or not word_shape(word, lexicon.alphabet):
            return TypoJudgement("shape")
        lower = word.casefold()
        capital = lower != word
        if capital and not capital_allowed:
            return TypoJudgement("capital")
        if lexicon.occurs(lower):
            return TypoJudgement("occurs")
        for other, reading in readings.items():
            other_lexicon = self.lexicons.get(other)
            if other_lexicon is not None and other_lexicon.occurs(reading.casefold()):
                return TypoJudgement("other_reading")
        if known(word) or any(known(reading) for reading in readings.values()):
            return TypoJudgement("known")
        return self.decide(self.candidates(lower, locale), capital)

    def candidates(self, lower: str, locale: str) -> list[Candidate]:
        """The stored words one slip away from a lowercase word, each with every slip that leads to it.

        They do not depend on the policy, so the measurement tool scores one list under many policies.
        """

        lexicon = self.lexicons[locale]
        found: dict[str, list[Edit]] = {}
        looked: dict[str, int | None] = {}
        for edit in edits(lower, lexicon.alphabet, self.neighbours[locale]):
            candidate = edit.apply(lower)
            if candidate not in looked:
                looked[candidate] = lexicon.weight(candidate)
            if looked[candidate] is not None:
                found.setdefault(candidate, []).append(edit)
        return [Candidate(word, weight, tuple(found[word])) for word, weight in looked.items()
                if weight is not None]

    def decide(self, candidates: Sequence[Candidate], capital: bool) -> TypoJudgement:
        """The noisy-channel choice among the candidates: each scores the logarithm of its count plus that of the sum
        of its slips' probabilities; the best is taken when it is frequent enough and clear enough of the rest."""

        if not candidates:
            return TypoJudgement("no_candidate")
        probabilities = self._log_probabilities
        totals = [candidate.weight / TYPO_COUNT_LOG_SCALE
                  + _log_sum([probabilities[edit.operation] for edit in candidate.edits]) for candidate in candidates]
        best = max(range(len(candidates)), key=lambda index: (totals[index], candidates[index].word))
        others = totals[:best] + totals[best + 1:]
        margin = totals[best] - _log_sum(others) if others else math.inf
        chosen = candidates[best]
        edit = max(chosen.edits, key=lambda item: probabilities[item.operation])
        cased = chosen.word[:1].upper() + chosen.word[1:] if capital else chosen.word
        choice = TypoChoice(cased, edit, chosen.weight, len(candidates), margin)
        if chosen.weight < self.policy.minimum_weight:
            return TypoJudgement("rare", choice, len(candidates))
        if margin < math.log(self.policy.posterior_ratio):
            return TypoJudgement("ambiguous", choice, len(candidates))
        return TypoJudgement("corrected", choice, len(candidates))


def _log_sum(values: Sequence[float]) -> float:
    high = max(values)
    return high + math.log(sum(math.exp(value - high) for value in values))


def known_elsewhere(
    text: str, *, lexicons: Iterable[Mapping[str, int]], terms: Mapping[str, Mapping[str, int]],
    identifiers: IdentifierLexicon | None, excluded: Collection[str],
) -> bool:
    """Whether the engine knows a reading from anything but the counted lists: a word of its lexicons (spelt with `ё`
    or `е`, no spell checker), a word the term tables count in any alphabet, a name of the identifier index, a token
    the code guard protects, or a word the user excluded."""

    lower = text.casefold()
    spellings = {LanguageModel.normalize(text), LanguageModel.normalize(text.replace(_YO, _YE))}
    return (any(spelling in lexicon for lexicon in lexicons for spelling in spellings)
            or any(lower in table for table in terms.values())
            or (identifiers is not None and identifiers.contains(text))
            or LanguageDetector.is_protected_token(text)
            or lower in excluded)


class KeyTemplates:
    """The last key press that typed each letter, by layout: the key a correction types for a letter not in the word.

    Every press carries its character in every layout (KeyEvent.characters), so one press teaches a letter of each.
    """

    def __init__(self) -> None:
        self._keys: dict[tuple[int, str], KeyEvent] = {}

    def observe(self, event: KeyEvent) -> None:
        if not event.pressed or event.synthetic or event.control or event.alt or event.super_key:
            return
        for group, character in enumerate(event.characters):
            if len(character) == 1 and character.isalpha():
                self._keys[(group, character)] = event

    def key(self, group: int, character: str, typed_group: int) -> KeyEvent | None:
        """A press of the key that types `character` in `group`, as if typed in `typed_group` like the word's keys."""

        template = self._keys.get((group, character))
        if template is None:
            return None
        return replace(template, pressed=True, group=typed_group, character=template.character_for(typed_group),
                       synthetic=False, deferred=False, foreign=False)


def corrected_keys(
    typed: Sequence[KeyEvent], group: int, choice: TypoChoice, templates: KeyTemplates,
) -> tuple[KeyEvent, ...] | None:
    """The keys that type `choice.word` in `group`, from the keys typed: each letter the user typed is reused as it
    was pressed when it still types the right letter (case included), and any other letter takes a template; None
    when a letter has none."""

    keys: list[KeyEvent] = []
    typed_group = typed[0].group if typed else group
    for wanted, source in zip(choice.word, choice.edit.sources(len(typed)), strict=True):
        if source is not None and typed[source].character_for(group) == wanted:
            keys.append(typed[source])
            continue
        template = templates.key(group, wanted, typed_group)
        if template is None:
            return None
        keys.append(template)
    return tuple(keys)
