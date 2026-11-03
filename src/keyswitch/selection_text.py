"""What the selection hotkeys do to the selected text: its layout, its case and its transliteration.

Pure functions of the text, verified without any platform; the platforms only read the selection and type the
result over it (selection_commands).
"""

from __future__ import annotations

import re
from collections.abc import Callable
from dataclasses import dataclass
from typing import Final

from .constants.text import CAPS_LOCK_WORD_MIN_CAPITALS
from .layouts import RU_KEYS, US_KEYS, LayoutPair

# The two layouts the selection is converted between, by the names `detection.layouts` gives them.
US_LAYOUT: Final = "us"
RU_LAYOUT: Final = "ru"

# Signs the two layouts type with Shift, on the keys of US_KEYS and RU_KEYS and on the keys those rows leave out (the
# number row and the slash key). LayoutPair maps the unshifted rows and the capital letters; these are the rest of the
# keys a selection typed in the wrong layout holds. Read from xkeyboard-config: /usr/share/X11/xkb/symbols/us,
# variant "basic", and /usr/share/X11/xkb/symbols/ru, its default variant "winkeys" (the Russian layout of Windows):
# Shift+2 is `@` and `"`, Shift+3 `#` and `№`, Shift+4 `$` and `;`, Shift+6 `^` and `:`, Shift+7 `&` and `?`, the
# slash key `/` `?` and `.` `,`, the backslash key with Shift `|` and `/`.
US_SHIFTED_SIGNS: Final = '~{}|:"<>/?@#$^&'
RU_SHIFTED_SIGNS: Final = 'ЁХЪ/ЖЭБЮ.,"№;:?'


def _full_map(base: dict[str, str], source: str, target: str) -> dict[int, str]:
    mapping = dict(base)
    mapping.update(zip(source, target, strict=True))
    return str.maketrans(mapping)


_PAIR: Final = LayoutPair()
# What the same key types in the other layout, for every key of the two layouts that types different characters in
# them. A character the source layout does not type at all (a digit, a space, a sign on the same key in both, a letter
# of the other script, an emoji) is left as it is.
_US_TO_RU: Final = _full_map(_PAIR.us_to_ru, US_SHIFTED_SIGNS, RU_SHIFTED_SIGNS)
_RU_TO_US: Final = _full_map(_PAIR.ru_to_us, RU_SHIFTED_SIGNS, US_SHIFTED_SIGNS)
# The letters each layout types, small and capital: they decide which layout a selection was typed in.
LATIN_LETTERS: Final = frozenset(
    character for key in US_KEYS if key.isalpha() for character in (key, key.upper()))
CYRILLIC_LETTERS: Final = frozenset(
    character for key in RU_KEYS if key.isalpha() for character in (key, key.upper()))


@dataclass(frozen=True)
class LayoutConversion:
    """The selection as the same keys type it in the other layout, and the layouts it went between."""

    text: str
    source: str
    target: str


def _letter_counts(text: str) -> tuple[int, int]:
    latin = sum(character in LATIN_LETTERS for character in text)
    cyrillic = sum(character in CYRILLIC_LETTERS for character in text)
    return latin, cyrillic


# For each layout: the table to the other layout and the letters it types.
_CONVERSIONS: Final = {
    US_LAYOUT: (_US_TO_RU, RU_LAYOUT, LATIN_LETTERS),
    RU_LAYOUT: (_RU_TO_US, US_LAYOUT, CYRILLIC_LETTERS),
}


def _converted_runs(text: str, source: str) -> str:
    """``text`` with only the runs typed in ``source`` retyped in the other layout.

    A run goes from a letter of the source layout to its last letter before a space, a digit or a letter of the other
    script, signs on the source layout's keys inside it included: `j,]trn` is `объект` wherever it stands. A sign at
    either end of a run stays as typed (`vbh.` in `привет vbh.` is `мир.`): in a sentence of the other script it most
    likely belongs to that sentence, and a sign alone between words (the comma of `привет, vbh`) is no run at all.
    """

    table, _target, letters = _CONVERSIONS[source]
    parts: list[str] = []
    run: list[str] = []
    for character in (*text, ""):
        if character and ord(character) in table:
            run.append(character)
            continue
        inside = [index for index, item in enumerate(run) if item in letters]
        chunk = "".join(run)
        if inside:
            first, last = inside[0], inside[-1] + 1
            chunk = chunk[:first] + chunk[first:last].translate(table) + chunk[last:]
        parts.extend((chunk, character))
        run = []
    return "".join(parts)


def convert_layout(text: str, current_layout: str) -> LayoutConversion:
    """The selection retyped on the same keys in the other layout: `ghbdtn vbh` is `привет мир`.

    A selection in one script was typed in the other layout as a whole, and all of it is retyped; pressing again
    converts back. A selection mixing the scripts is mostly right: the script with more letters is what was meant, and
    only the runs of the other one are retyped into its layout (`привет vbh` is `привет мир`, `hello руддщ` is
    `hello hello`), with the signs on their keys inside them (_converted_runs). A selection with as many letters of
    one as of the other, or with no letters at all (`?/`), was typed in the layout that is on now and is retyped whole.
    The conversion names the layout the keyboard is switched to afterwards: the other one for a whole selection, that
    of the majority for a mixed one.
    """

    latin, cyrillic = _letter_counts(text)
    if latin and cyrillic and latin != cyrillic:
        minority = US_LAYOUT if latin < cyrillic else RU_LAYOUT
        return LayoutConversion(_converted_runs(text, minority), minority, _CONVERSIONS[minority][1])
    if latin > cyrillic:
        source = US_LAYOUT
    elif cyrillic > latin:
        source = RU_LAYOUT
    else:
        source = RU_LAYOUT if current_layout == RU_LAYOUT else US_LAYOUT
    table, target, _letters = _CONVERSIONS[source]
    return LayoutConversion(text.translate(table), source, target)


# Letters inside a word: an apostrophe does not start a new word (`don't` is one word, `Don't` in Title case).
_WORD_INSIDE: Final = frozenset("'’")
# A run of letters, for telling text typed under Caps Lock.
_LETTER_RUN: Final = re.compile(r"[^\W\d_]+")


def _cased(text: str) -> list[str]:
    return [character for character in text if character.lower() != character.upper()]


def _title(text: str) -> str:
    """Every word with its first letter capital and the rest small."""

    result: list[str] = []
    before = previous = ""
    for character in text:
        # An apostrophe is inside a word only after a letter: `don't`, but `'hello'` starts a word after it.
        inside = previous.isalnum() or (previous in _WORD_INSIDE and before.isalnum())
        result.append(character.lower() if inside else character.upper())
        before, previous = previous, character
    return "".join(result)


def _typed_under_caps_lock(text: str) -> bool:
    """Whether the text looks typed with Caps Lock on: `пРИВЕТ мИР`, every word small at its first letter only.

    A word of one letter says nothing, and a word all in capitals fits too (`пРИВЕТ, Я ТУТ`); at least one word must
    show the small first letter followed by CAPS_LOCK_WORD_MIN_CAPITALS capitals or more. `iOS` and `eSIM` have
    fewer and are names written that way, not Caps Lock.
    """

    seen = False
    for word in _LETTER_RUN.findall(text):
        letters = _cased(word)
        if len(letters) <= 1:
            continue
        if not all(letter.isupper() for letter in letters[1:]):
            return False
        seen = seen or (letters[0].islower() and len(letters) - 1 >= CAPS_LOCK_WORD_MIN_CAPITALS)
    return seen


def cycle_case(text: str) -> str:
    """The next case of the selection: small → CAPITALS → Title Case → small, and Caps Lock text turned over.

    The cycle follows from the text alone, because the selection is read anew at every press: whatever was selected,
    the press moves it one step on, and three presses bring back what was there. Text typed with Caps Lock on
    (`пРИВЕТ`) is turned over first, to `Привет`, as Punto Switcher's case command does with this very example; any
    other mixture of cases (`Привет мир`) goes to small letters. Title Case capitalizes every word rather than the first
    of a sentence: words are found by letters alone, sentences are not (`т. е.`, `Dr.`). A step that would not change
    the text (`Я` is in capitals and in Title Case at once) is passed over.
    """

    letters = _cased(text)
    if not letters:
        return text
    if _typed_under_caps_lock(text):
        return text.swapcase()
    if all(letter.islower() for letter in letters):
        steps: tuple[Callable[[str], str], ...] = (str.upper, _title, str.lower)
    elif all(letter.isupper() for letter in letters):
        steps = (_title, str.lower, str.upper)
    else:
        steps = (str.lower, str.upper, _title)
    return next((result for result in (step(text) for step in steps) if result != text), text)


# Russian letters in Latin by ICAO Doc 9303, Machine Readable Travel Documents, Eighth Edition 2021, Part 3, section 6,
# table B "Transliteration of Cyrillic Characters" (https://www.icao.int/sites/default/files/publications/DocSeries/
# 9303_p3_cons_en.pdf), the transliteration of machine readable travel documents. Ё and Э are E like Е, Й is I like И,
# Ъ is IE, and Ь has no row in the table, so it is left out.
CYRILLIC_TO_LATIN: Final = {
    "а": "a", "б": "b", "в": "v", "г": "g", "д": "d", "е": "e", "ё": "e", "ж": "zh", "з": "z", "и": "i", "й": "i",
    "к": "k", "л": "l", "м": "m", "н": "n", "о": "o", "п": "p", "р": "r", "с": "s", "т": "t", "у": "u", "ф": "f",
    "х": "kh", "ц": "ts", "ч": "ch", "ш": "sh", "щ": "shch", "ъ": "ie", "ы": "y", "ь": "", "э": "e", "ю": "iu",
    "я": "ia",
}
# Latin back to Russian, the best the lossy standard allows: each sequence it writes for one letter goes back to the
# letter it is most often written for (`e` is е, not ё or э; `ia` is я, not и followed by а). `ie` is left to its two
# letters, since ICAO writes и followed by е the same way and that is far more common than ъ. Common informal
# spellings the standard never writes are read too: `ya`, `yu`, `yo` and `ja`, `ju`, `jo` are я, ю, ё (ICAO would
# write ы before a vowel, which no Russian word has), and the letters `c` ц, `h` х, `j` й, `q` к, `w` в, `x` кс.
LATIN_TO_CYRILLIC: Final = {
    "shch": "щ", "zh": "ж", "kh": "х", "ts": "ц", "ch": "ч", "sh": "ш", "iu": "ю", "ia": "я",
    "ya": "я", "yu": "ю", "yo": "ё", "ja": "я", "ju": "ю", "jo": "ё",
    "a": "а", "b": "б", "v": "в", "g": "г", "d": "д", "e": "е", "z": "з", "i": "и", "k": "к", "l": "л", "m": "м",
    "n": "н", "o": "о", "p": "п", "r": "р", "s": "с", "t": "т", "u": "у", "f": "ф", "y": "ы",
    "c": "ц", "h": "х", "j": "й", "q": "к", "w": "в", "x": "кс",
}
# Sequences read by their place in the word, before the table. Each rule was measured on the Russian lexicon of the
# language model (every word written by ICAO and read back, weighted by frequency: 92.5 % of the text came back before
# them, 96.6 % with them): a word-final `tsia` is the -тся of verbs (`iavliaetsia`, not являеця; -ция is `tsiia`),
# `ts` before `k` is тс (`sovetskii`, not совецкий), and a word that starts with `ye` starts with е (`Yelena`), since
# no Russian word starts with ые.
_VERB_ENDING: Final = ("tsia", "тся")
_TS_BEFORE_K: Final = ("tsk", "тс")
_INITIAL_YE: Final = ("ye", "е")
# `i` after a vowel is й before a consonant (`voina`, `Chaikovskii`) and at the end of a word (`Sergei`, `krasnyi`),
# except after и, where -ии of nouns (`Rossii`, `istorii`) is more common than -ий, unless к, г or х come before it
# (`russkii`, `dolgii`): the measure above counts these too.
_SHORT_I: Final = "й"
_CYRILLIC_VOWELS: Final = frozenset("аеёиоуыэюя")
_LATIN_VOWELS: Final = frozenset("aeiouy")
_BEFORE_ADJECTIVE_ENDING: Final = frozenset("кгх")
# An apostrophe between Latin letters is the soft sign of informal spelling (`Ol'ga`); ICAO writes none.
_SOFT_SIGN: Final = "ь"
_APOSTROPHES: Final = frozenset("'’")
# The lengths of the table's sequences, longest first: the longest one that matches is read (`shch` before `sh`).
_SEQUENCE_LENGTHS: Final = tuple(sorted({len(sequence) for sequence in LATIN_TO_CYRILLIC}, reverse=True))


def _in_case(output: str, text: str, start: int, end: int) -> str:
    """`output` in the case of `text[start:end]`, which it stands for: small, Capitalized, or all CAPITALS.

    A capital stands for a word in capitals when the letter after it is a capital, or when the sequence it starts is
    in capitals, or when it ends a word that a capital comes before: `ЖУК` is `ZHUK`, `Жук` is `Zhuk`, `ВЬЮ` is `VIU`.
    """

    first = text[start]
    if not first.isupper():
        return output.lower()
    after = text[end] if end < len(text) else ""
    before = text[start - 1] if start > 0 else ""
    capitals = (
        after.isupper()
        or (end - start > 1 and text[start:end].isupper())
        or (not after.isalpha() and before.isupper())
    )
    return output.upper() if capitals else output.capitalize()


def _to_latin(text: str) -> str:
    parts: list[str] = []
    for index, character in enumerate(text):
        latin = CYRILLIC_TO_LATIN.get(character.lower())
        parts.append(character if latin is None else _in_case(latin, text, index, index + 1))
    return "".join(parts)


def _latin_letter(character: str) -> bool:
    return character.isascii() and character.isalpha()


def _word_edge(text: str, index: int) -> bool:
    """Whether no Latin letter stands at ``index`` (the end of the text counts)."""

    return index >= len(text) or not _latin_letter(text[index])


def _placed_sequence(text: str, index: int) -> tuple[str, str] | None:
    """A sequence read by its place in the word (_VERB_ENDING, _TS_BEFORE_K, _INITIAL_YE), with its Cyrillic."""

    for (latin, cyrillic), placed in (
        (_VERB_ENDING, _word_edge(text, index + len(_VERB_ENDING[0]))),
        (_TS_BEFORE_K, True),
        (_INITIAL_YE, index == 0 or _word_edge(text, index - 1)),
    ):
        if placed and text[index:index + len(latin)].lower() == latin:
            # The `k` after `ts` is read on its own: only тс is written for this match.
            return (latin[:len(latin) - 1], cyrillic) if (latin, cyrillic) == _TS_BEFORE_K else (latin, cyrillic)
    return None


def _short_i(text: str, end: int, parts: list[str]) -> bool:
    """Whether the `i` that ends at ``end`` is й (see _SHORT_I)."""

    written = "".join(parts).lower()
    vowel = written[-1:]
    if vowel not in _CYRILLIC_VOWELS:
        return False
    if not _word_edge(text, end):
        return text[end].lower() not in _LATIN_VOWELS
    return vowel != "и" or written[:-1][-1:] in _BEFORE_ADJECTIVE_ENDING


def _to_cyrillic(text: str) -> str:
    parts: list[str] = []
    index = 0
    while index < len(text):
        placed = _placed_sequence(text, index)
        # Only ASCII letters are read: `İ` folds to two characters and would shift every match after it.
        sequence = placed[0] if placed is not None else next((
            chunk.lower() for length in _SEQUENCE_LENGTHS
            for chunk in (text[index:index + length],)
            if chunk.isascii() and chunk.lower() in LATIN_TO_CYRILLIC
        ), None)
        if sequence is None:
            between_letters = index > 0 and _latin_letter(text[index - 1]) and not _word_edge(text, index + 1)
            parts.append(_SOFT_SIGN if text[index] in _APOSTROPHES and between_letters else text[index])
            index += 1
            continue
        end = index + len(sequence)
        cyrillic = placed[1] if placed is not None else LATIN_TO_CYRILLIC[sequence]
        if sequence == "i" and _short_i(text, end, parts):
            cyrillic = _SHORT_I
        parts.append(_in_case(cyrillic, text, index, end))
        index = end
    return "".join(parts)


def transliterate(text: str) -> str:
    """The selection in the other script: `Привет` is `Privet` and `Privet` is `Привет`.

    The script with more letters in the selection is the one written over, and only its letters change: `Привет, John`
    is `Privet, John`. A tie goes to the Russian letters, which the standard is written for. Letters of neither
    alphabet stay as they are.
    """

    latin, cyrillic = _letter_counts(text)
    if cyrillic and cyrillic >= latin:
        return _to_latin(text)
    if latin:
        return _to_cyrillic(text)
    return text
