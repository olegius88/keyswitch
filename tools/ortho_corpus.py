#!/usr/bin/env python3
"""Key-space token corpus derived from the frozen CC0 phrase snapshot.

The observation a layout switcher makes is not the visible string, it is the
sequence of physical keys. Because the us/ru map is a bijection on the mapped
keys, rendering every token into key space puts Russian and English text into
one shared event space, so two character models over it can be compared as a
likelihood ratio without any cross-language renormalisation.

Two rules matter for honesty and are enforced here:

* A token is labelled by the alphabet ITS OWN letters are written in, never by
  the language of the sentence. A Latin word inside a Russian sentence was typed
  with the layout switched; counting it as Russian evidence would poison the
  negatives with exactly the tokens the model must learn to convert.
* Tokens are stored whole, with their punctuation; what the engine would hand
  the model for them is derived by `served`, because that depends on the layout
  the user is typing in. The leading dot of ``.dist`` is the character that
  produces the ``ю`` of ``ювшые``; a word regex throws that evidence away.

Generation 3 measures the population the engine actually serves the model,
not every token of the snapshot. Most of the snapshot is dictionary words,
which the runtime never asks this model about, and its phrase punctuation had
Russian commas and full stops on the wrong keys, so the old corpus scored
fragments no keyboard produces. The rules of that population live here, each
next to the runtime code it repeats, and `tests/test_ortho_population.py`
fails if a copied set drifts from the engine.

The phrases and their groups are those of the frozen context-v2 snapshot
(``tools/context_corpus.py``), so this file introduces no new corpus text and no
new licence surface; the frozen Onboard word lists of Layout Intent v1 are added
to the character counts only.

One correction to the alphabet rule (`label`): a token written in the other
alphabet inside a sentence of a language, whose reading in that language is a
word of its frozen word list while the token itself is no word of its own
alphabet's list, is that language typed in the wrong layout - `crfpfk` in a
Russian sentence is `сказал`, with or without the punctuation around it - and is
labelled so. Nothing is dropped. Which split a group falls into is drawn
here, from ``NAMESPACE``. Earlier generations' evidence is kept in
``model/ortho_v1/development-history``.
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import sys
import unicodedata
from collections import Counter
from collections.abc import Sequence
from pathlib import Path
from typing import Final, Literal

sys.path.insert(0, str(Path(__file__).resolve().parent))

import context_corpus  # noqa: E402

from keyswitch.constants.corpus import CONTEXT_PHRASE_SPLIT_BUCKET_COUNT  # noqa: E402
from keyswitch.constants.detection import ACRONYM_MIN_LETTERS  # noqa: E402
from keyswitch.constants.file_formats import (  # noqa: E402
    FROZEN_CORPUS_RECEIPT_JSON_INDENT,
    HEXADECIMAL_BASE,
)
from keyswitch.constants.model_protocol import TEST, TRAIN  # noqa: E402
from keyswitch.constants.models import CONTEXT_TYPO_MIN_CHARACTERS  # noqa: E402
from keyswitch.constants.ortho import (  # noqa: E402
    ORTHO_V1_EVIDENCE_SCHEMA_VERSION,
    ORTHO_V1_LEXICON_MIN_CHARACTERS,
    ORTHO_V1_MAXIMUM_LEXICON_WEIGHT,
    ORTHO_V1_MAXIMUM_TOKEN_CHARACTERS,
    ORTHO_SYNTHETIC_EXTRA_KEY_SHARE_PERCENT,
    ORTHO_V1_TEST_SPLIT_CEILING,
)
from keyswitch.constants.text import ASCII_CONTROL_CHARACTER_LIMIT, ASCII_DELETE_CODEPOINT  # noqa: E402
from keyswitch.constants.training import DETERMINISTIC_CHOICE_HEX_DIGITS  # noqa: E402
from keyswitch.constants.units import PERCENT_SCALE  # noqa: E402
from keyswitch.layouts import LayoutPair  # noqa: E402
from keyswitch.value_provenance import CONSTANTS_PACKAGE, pin_values  # noqa: E402

Script = Literal["en", "ru"]

ROOT: Final[Path] = Path(__file__).resolve().parents[1]
DIRECTORY: Final[Path] = ROOT / "model/ortho_v1"
TOKENS: Final[Path] = DIRECTORY / "tokens.jsonl.gz"
RECEIPT: Final[Path] = DIRECTORY / "corpus-receipt.json"
# Draws the split of every phrase group. Never reuse a retired value: generation 1 inherited
# context-v2's own split (its receipt namespace was "keyswitch:ortho-v1:key-space-20260908"),
# generation 2 used "keyswitch:ortho-v1:key-space-20260925" (rejected before its test), generations
# 3 and 4 "keyswitch:ortho-v1:served-split-20260925" (its test was opened by generation 4),
# generation 5 "keyswitch:ortho-v1:served-split-g5" (sealed, superseded before its test), generation 6
# "keyswitch:ortho-v1:served-split-g6" (its test was opened by generation 6), generation 7
# "keyswitch:ortho-v1:served-split-g7" and generation 8 "keyswitch:ortho-v1:served-split-g8" (both dropped
# after their development, before any candidate; the three visible parts were the development of generation 9);
# generations 9 to 12 "keyswitch:ortho-v1:served-split-g9" to "-g16" (their candidates were checked on independent
# sets or stopped before; the snapshot's other part was left unused). The candidate in the tree is that of "-g15".
NAMESPACE: Final[str] = "keyswitch:ortho-v1:served-split-g15"
# Rows of the frozen word lists carry this split name: they add characters to the counts and are
# never evaluated.
LEXICON: Final = "lexicon"
CYRILLIC: Final[frozenset[str]] = frozenset("абвгдеёжзийклмнопрстуфхцчшщъыьэюя")
LATIN: Final[frozenset[str]] = frozenset("abcdefghijklmnopqrstuvwxyz")
# The code that builds the corpus. The receipt pins its bytes and, since its numbers moved into
# keyswitch.constants, the values it imports from there as well.
CODE: Final[tuple[str, ...]] = (
    "tools/ortho_corpus.py", "tools/context_corpus.py", "src/keyswitch/layouts.py",
)
# The frozen Onboard word lists of Layout Intent v1, pinned by their reviewed digests.
LEXICONS: Final[dict[str, tuple[str, str]]] = {
    "en": ("model/intent_v1/sources/en_US.lm",
           "017a513a23bb37947a3a0e73e307066fa8ec0642075172cdcad6ea204cae68da"),
    "ru": ("model/intent_v1/sources/ru_RU.lm",
           "e57c14eec2b78e52a2125dce2b38044d8dfe480f3964b9d118614527195ceda9"),
}
PAIR: Final[LayoutPair] = LayoutPair()
# The keys of the us/ru pair are the printable ASCII characters of the US layout. A token spelled
# with anything else (an accented name, a Kabyle word) cannot arrive as keystrokes of this pair,
# and `ContextPolicy._orthotactic` only asks about the two groups of the pair.
KEYS: Final[frozenset[str]] = frozenset(map(chr, range(ASCII_CONTROL_CHARACTER_LIMIT + 1, ASCII_DELETE_CODEPOINT)))
# Which keys write a letter in each layout. The Russian layout writes several of the US
# punctuation keys as letters, which is why a trailing comma survives into the token when the text
# was typed there and disappears when it was not.
LETTER_KEYS: Final[dict[str, frozenset[str]]] = {
    "en": frozenset(key for key in KEYS if key.isalpha()),
    "ru": frozenset(key for key in KEYS if PAIR.translate(key, "us", "ru").isalpha()),
}
# ЙЦУКЕН puts the full stop and the comma on the key the US layout renders as `/`, and the layout
# table does not model that: it maps both characters to themselves. Uncorrected, a Russian token
# ending in a comma becomes a key sequence ending in `,`, whose Russian reading is `б` - a token
# nobody typed. The layout table itself is certified and unchanged; this override lives here,
# where a corpus is synthesised from text rather than observed from keystrokes.
RUSSIAN_PUNCTUATION_KEYS: Final[dict[str, str]] = {".": "/", ",": "/"}
# The reverse: the key the US layout writes as `/` writes the full stop in Russian.
RUSSIAN_KEY_CHARACTERS: Final[dict[str, str]] = {"/": "."}
# Mirror `engine.PUNCTUATION` and the exemptions of `KeySwitchEngine._is_boundary`. A copy rather
# than an import, because importing the engine would pull the backends into a corpus tool;
# `tests/test_ortho_population.py` fails if the two drift apart.
BOUNDARY_PUNCTUATION: Final[frozenset[str]] = frozenset(".,!?;:()[]{}—–-…\"«»")
BOUNDARY_EXEMPT: Final[frozenset[str]] = frozenset("_/\\@")
# `KeySwitchEngine._handle` appends `engine.WORD_JOINERS` and `_ @ / \ =` to a started word before
# it asks whether the key is a boundary, so `Я-то`, `из-за` and `don't` reach a model whole.
WORD_JOINERS: Final[frozenset[str]] = frozenset("'’-‐‑_@/\\=")


def checksum(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def script_of(token: str) -> Script | None:
    """The alphabet the token itself is written in, or None when it is mixed."""

    lowered = unicodedata.normalize("NFC", token).casefold()
    cyrillic = any(character in CYRILLIC for character in lowered)
    latin = any(character in LATIN for character in lowered)
    if cyrillic and not latin:
        return "ru"
    if latin and not cyrillic:
        return "en"
    return None


def to_keys(token: str) -> str:
    """The physical keys a typist pressed to produce this token."""

    lowered = unicodedata.normalize("NFC", token).casefold()
    if script_of(token) != "ru":
        return lowered
    return "".join(
        RUSSIAN_PUNCTUATION_KEYS.get(character) or PAIR.translate(character, "ru", "us")
        for character in lowered
    )


def other(script: str) -> Script:
    return "ru" if script == "en" else "en"


def visible(keys: str, layout: str) -> str:
    """What the user sees on screen while typing these keys in this layout."""

    if layout == "en":
        return keys
    return "".join(RUSSIAN_KEY_CHARACTERS.get(key) or PAIR.translate(key, "us", "ru") for key in keys)


def ends_a_word(character: str) -> bool:
    """Would the engine commit the word at this character? (`KeySwitchEngine._is_boundary`)"""

    return (character.isspace() or character in BOUNDARY_PUNCTUATION
            or (character not in BOUNDARY_EXEMPT and unicodedata.category(character[0]).startswith("P")))


def served(keys: str, layout: str) -> tuple[str, ...]:
    """Every token the engine hands to a model for these keys typed in this layout.

    One key sequence can become several: the engine commits a word at any key whose character is
    punctuation in the layout being typed, so `"ably"` is one token and `word!more` is two. A hyphen
    or an apostrophe is not such a key - the engine joins those to a started word before it tests for
    a boundary at all. Within a segment two more of the engine's rules apply: `_literal_head` keeps
    only what follows the last `/`, and a trailing key that is a letter only in the other layout is
    the ambiguous tail `_completed_word` splits off.

    Two approximations remain, both bounded: `_completed_word` asks its boundary model how much of
    that tail to take, and this takes all of it; `_literal_head` additionally requires the whole
    token to look like code, which is not reproduced. Ambiguity is judged by the layout table the
    engine converts with (`KeySwitchEngine._is_layout_letter`).
    """

    other_letters = LETTER_KEYS[other(layout)]
    keep = LETTER_KEYS[layout]
    segments: list[str] = []
    current: list[str] = []
    for key in keys:
        if key in keep or key in other_letters:
            current.append(key)
            continue
        if current and visible(key, layout) in WORD_JOINERS:
            current.append(key)
            continue
        if ends_a_word(visible(key, layout)):
            if current:
                segments.append("".join(current))
            current = []
            continue
        # A symbol stroke the engine carries into the following word: `@ _ / \`.
        current.append(key)
    if current:
        segments.append("".join(current))
    result: list[str] = []
    for segment in segments:
        head = segment.rfind("/")
        core = segment[head + 1:] if head >= 0 else segment
        end = len(core)
        while end > 0 and core[end - 1] not in keep and core[end - 1] in other_letters:
            end -= 1
        if core[:end]:
            result.append(core[:end])
    return tuple(result)


def word_core(keys: str, layout: str) -> str:
    """The letters of the token in this layout, for counting rather than for serving.

    The character model is being taught which letter sequences a language produces. A word the
    source text wrapped in quotation marks is still that evidence, so both edges come off here even
    where the engine would keep them; every sequence the model is ever asked about is word-shaped,
    so this is a no-op on it.
    """

    keep = LETTER_KEYS[layout]
    start, end = 0, len(keys)
    while start < end and keys[start] not in keep:
        start += 1
    while end > start and keys[end - 1] not in keep:
        end -= 1
    return keys[start:end]


def shape_of(token: str, position: int) -> str:
    """Case shape, which is what makes the abbreviation reading available."""

    letters = [character for character in token if character.isalpha()]
    if len(letters) >= ACRONYM_MIN_LETTERS and all(character.isupper() for character in letters):
        return "upper"
    if token[:1].isupper():
        return "initial" if position == 0 else "inner"
    return "lower"


def split_of(group: str) -> str:
    """The split of one phrase group, drawn from this corpus's own namespace."""

    bucket = int(context_corpus.digest(f"{NAMESPACE}:{group}")[:DETERMINISTIC_CHOICE_HEX_DIGITS],
                 HEXADECIMAL_BASE) % CONTEXT_PHRASE_SPLIT_BUCKET_COUNT
    return TEST if bucket < ORTHO_V1_TEST_SPLIT_CEILING else TRAIN


def read_lexicon(path: Path, expected: str) -> dict[str, int]:
    """Unigrams of a frozen ARPA word list, refusing anything but the reviewed bytes."""

    content = path.read_bytes()
    if hashlib.sha256(content).hexdigest() != expected:
        raise ValueError(f"frozen word list {path.name} checksum mismatch")
    words: dict[str, int] = {}
    section = 0
    for line in content.decode("utf-8", errors="replace").splitlines():
        if line == r"\1-grams:":
            section = 1
            continue
        if line.startswith("\\"):
            section = 0
            continue
        if section != 1 or not line:
            continue
        count_text, separator, payload = line.partition(" ")
        if not separator or payload.startswith("<"):
            continue
        try:
            count = int(count_text)
        except ValueError:
            continue
        token = payload.casefold()
        if len(token) >= ORTHO_V1_LEXICON_MIN_CHARACTERS:
            words[token] = words.get(token, 0) + count
    return words


# The synthetic stray-key examples of the feature fit (`extra_key_variant`) draw from this namespace, and
# never from the words of the authored regression cases or their bases: the cases must be answered by
# what the model learns, not by their own variants.
SYNTHESIS_NAMESPACE: Final[str] = "keyswitch:ortho-v1:extra-key-synthesis"
AUTHORED_WORDS: Final[tuple[str, ...]] = (
    "htop", "webjs", "nginx", "bild", ".dist", "я-то", "фпривет", "привет", "клаиватура", "повторнно", "превет",
    "репост", "севодня", "созвон", "уведмление", "чатик", "пересборку", "healthcheck", "websocket", "hotfix", "npm",
    "kubectl", "const", "фнафер", "йцукен",
)
AUTHORED_KEYS: Final[frozenset[str]] = frozenset(to_keys(word) for word in AUTHORED_WORDS)


def extra_key_variant(script: str, keys: str) -> str | None:
    """The keys of a train word typed in the other layout with one stray key, or None.

    Drawn for a share of the words by a hash of the word, so the choice, the position (any of
    len + 1) and the key (a letter key of the layout being typed) do not depend on any split: a
    word typed with the layout switched, as every positive of the corpus, plus one key more.
    """

    base = word_core(keys, script)
    word = visible(base, script)
    if (len(base) + 1 < CONTEXT_TYPO_MIN_CHARACTERS or not word.isalpha()
            or base in AUTHORED_KEYS):
        return None
    digest = hashlib.sha256(f"{SYNTHESIS_NAMESPACE}:{script}:{base}".encode()).hexdigest()
    share, place, key, *_ = [int(digest[index:index + DETERMINISTIC_CHOICE_HEX_DIGITS], HEXADECIMAL_BASE)
                             for index in range(0, len(digest), DETERMINISTIC_CHOICE_HEX_DIGITS)]
    if share % PERCENT_SCALE >= ORTHO_SYNTHETIC_EXTRA_KEY_SHARE_PERCENT:
        return None
    letters = sorted(letter for letter in LETTER_KEYS[other(script)] if letter == letter.lower())   # no Shift
    position = place % (len(base) + 1)
    return base[:position] + letters[key % len(letters)] + base[position:]


def word_lists() -> dict[str, frozenset[str]]:
    """The frozen Onboard words per script, as the corpus reads them (`read_lexicon`)."""

    return {script: frozenset(read_lexicon(ROOT / path, expected)) for script, (path, expected) in sorted(LEXICONS.items())}


def label(raw: str, locale: str, words: dict[str, frozenset[str]]) -> Script | None:
    """The language a token was written in: its own alphabet, unless it is the sentence's language
    typed in the wrong layout.

    A Latin token in a Russian sentence whose Russian reading is a Russian word of the frozen list,
    while the token is no English word of that list, was Russian typed with the layout switched
    (`crfpfk` is `сказал`); the same holds the other way round. Its keys are the keys of that word,
    so relabelling it keeps every key and moves it to the language it was typed in.

    The reading is taken of the whole token and of its letter core in the layout it was typed in, and
    "a word of its own alphabet" is judged on that core. Punctuation keys are letters in the other
    layout, so `k.,k.` reads `люблю` only whole, while the quotes and question mark of `crfpfk?".` hide
    `сказал` unless they come off; and `it.` is the English word `it`, not a token the list lacks.
    The sentence's language stays part of the rule: an English abbreviation whose keys spell a Russian
    word (`XFCE`, `часу`) is English in an English sentence.
    """

    script = script_of(raw)
    if script is None:
        return None
    sentence: Script = "ru" if locale == "rus" else "en"
    if script == sentence:
        return script
    keys = to_keys(raw)
    core = word_core(keys, script)
    readings = {visible(keys, sentence), visible(core, sentence)}
    if any(reading in words[sentence] for reading in readings) and visible(core, script) not in words[script]:
        return sentence
    return script


def lexicon_rows() -> dict[tuple[str, str], int]:
    """(script, keys) -> weight, compressed so a frequent word cannot dominate.

    A word list is evidence about which sequences a language admits, not about how often they are
    typed. The bit length keeps the ordering of the frequency list while flattening four orders of
    magnitude into a few steps.
    """

    counts: dict[tuple[str, str], int] = {}
    for script, (path, expected) in sorted(LEXICONS.items()):
        for word, frequency in sorted(read_lexicon(ROOT / path, expected).items()):
            if script_of(word) != script:
                continue
            keys = to_keys(word)
            if not keys or len(keys) > ORTHO_V1_MAXIMUM_TOKEN_CHARACTERS:
                continue
            weight = max(1, min(ORTHO_V1_MAXIMUM_LEXICON_WEIGHT, int(frequency).bit_length()))
            key = (script, keys)
            counts[key] = max(counts.get(key, 0), weight)
    return counts


def rows() -> list[dict[str, object]]:
    """One row per (split, script, key sequence) with shape counts, then the word-list rows."""

    counts: dict[tuple[str, str, str], Counter[str]] = {}
    lists = word_lists()
    for assigned in context_corpus.assign(context_corpus.load_source())[0]:
        split = split_of(assigned.group)
        words = unicodedata.normalize("NFC", assigned.phrase.text).split()
        for position, raw in enumerate(words):
            if len(raw) > ORTHO_V1_MAXIMUM_TOKEN_CHARACTERS:
                continue
            script = label(raw, assigned.phrase.locale, lists)
            if script is None:
                continue
            keys = to_keys(raw)
            if not keys:
                continue
            shape = counts.setdefault((split, script, keys), Counter())
            shape[shape_of(raw, position)] += 1
    payload: list[dict[str, object]] = [
        {"split": split, "script": script, "keys": keys,
         "shapes": {name: shape[name] for name in sorted(shape)}}
        for (split, script, keys), shape in sorted(counts.items())
    ]
    payload.extend(
        {"split": LEXICON, "script": script, "keys": keys, "shapes": {"lower": weight}}
        for (script, keys), weight in sorted(lexicon_rows().items())
    )
    return payload


def write_tokens(payload: list[dict[str, object]], path: Path) -> str:
    raw = "".join(
        json.dumps(row, sort_keys=True, ensure_ascii=False, separators=(",", ":")) + "\n"
        for row in payload
    ).encode("utf-8")
    content = gzip.compress(raw, mtime=0)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(content)
    return hashlib.sha256(content).hexdigest()


def load_tokens(path: Path = TOKENS) -> list[dict[str, object]]:
    with gzip.open(path, "rt", encoding="utf-8") as handle:
        return [json.loads(line) for line in handle]


def provenance() -> dict[str, str]:
    code = tuple(ROOT / name for name in CODE)
    paths = (*code, context_corpus.SOURCE, context_corpus.RECEIPT,
             *(ROOT / path for path, _expected in sorted(LEXICONS.values())))
    pins = {path.relative_to(ROOT).as_posix(): checksum(path) for path in paths}
    pins[CONSTANTS_PACKAGE] = pin_values(code, source_root=ROOT / "src").sha256
    return pins


def report(payload: list[dict[str, object]], tokens_sha: str) -> dict[str, object]:
    by_split: Counter[str] = Counter()
    by_script: Counter[str] = Counter()
    occurrences: Counter[str] = Counter()
    for row in payload:
        split, script = str(row["split"]), str(row["script"])
        shapes = row["shapes"]
        assert isinstance(shapes, dict)
        by_split[split] += 1
        by_script[f"{split}:{script}"] += 1
        occurrences[f"{split}:{script}"] += sum(int(value) for value in shapes.values())
    return {
        "schema_version": ORTHO_V1_EVIDENCE_SCHEMA_VERSION,
        "namespace": NAMESPACE,
        "description": (
            "Key-space token types from the frozen CC0 phrase snapshot, stored whole with their "
            "punctuation, Russian commas and full stops on their own key; the frozen Onboard word "
            "lists follow as rows that only add characters to the counts. What the engine serves "
            "the model is derived per layout by `served`."
        ),
        "label_rule": (
            "the alphabet of the token itself, except that a token in the other alphabet inside a "
            "sentence of a language, whose reading in that language is a word of its frozen word "
            "list while the token is no word of its own alphabet's list, is that language typed in "
            "the wrong layout; the reading is taken of the whole token and of its letter core in the "
            "layout it was typed in, and the token's own word is that core"
        ),
        "split_rule": (
            "context-v2 phrase groups (tools/context_corpus.py); bucket = "
            f"int(sha256(namespace + ':' + group)[:{DETERMINISTIC_CHOICE_HEX_DIGITS}], "
            f"{HEXADECIMAL_BASE}) mod {CONTEXT_PHRASE_SPLIT_BUCKET_COUNT}; "
            f"{TEST} below {ORTHO_V1_TEST_SPLIT_CEILING}, {TRAIN} otherwise"
        ),
        "lexicon_weight": f"max(1, min({ORTHO_V1_MAXIMUM_LEXICON_WEIGHT}, frequency.bit_length()))",
        "types_by_split": dict(sorted(by_split.items())),
        "types_by_split_script": dict(sorted(by_script.items())),
        "occurrences_by_split_script": dict(sorted(occurrences.items())),
        "maximum_token_characters": ORTHO_V1_MAXIMUM_TOKEN_CHARACTERS,
        "tokens_sha256": tokens_sha,
        "provenance": provenance(),
    }


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--freeze", action="store_true")
    arguments = parser.parse_args(argv)
    payload = rows()
    if arguments.freeze and not TOKENS.exists():
        tokens_sha = write_tokens(payload, TOKENS)
    else:
        if not TOKENS.exists():
            raise ValueError("token corpus is missing; run once with --freeze")
        tokens_sha = checksum(TOKENS)
        if write_tokens(payload, DIRECTORY / ".tokens-check.gz") != tokens_sha:
            (DIRECTORY / ".tokens-check.gz").unlink()
            raise ValueError("token corpus is not reproducible from the frozen source")
        (DIRECTORY / ".tokens-check.gz").unlink()
    content = (json.dumps(report(payload, tokens_sha), sort_keys=True, ensure_ascii=False,
                          indent=FROZEN_CORPUS_RECEIPT_JSON_INDENT) + "\n").encode()
    if arguments.freeze and not RECEIPT.exists():
        RECEIPT.write_bytes(content)
    elif not RECEIPT.exists() or RECEIPT.read_bytes() != content:
        raise ValueError("corpus receipt is missing or changed; do not overwrite a frozen split")
    print(content.decode("utf-8"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
