#!/usr/bin/env python3
"""Key-space token corpus v2: engine-shaped tokens plus the frozen lexicons.

Version 1 of this corpus was right about the event space and wrong about the
population. Two corrections define version 2, and both were measured before
they were written down.

* **Tokens are evaluated in the shape the engine hands over, and that shape
  depends on the layout in use.** Version 1 split text on whitespace, so it
  scored fragments the runtime cannot produce - ``"z".``, ``krv,`` - and four of
  the six tokens that set the published thresholds were of that kind.

  What the engine hands over is what ``served`` reproduces, key by key, from the
  engine's own three rules. A key that writes a letter in EITHER layout joins the
  word: ``.`` is ``ю`` in Russian, so ``.dist`` typed in the US layout arrives
  whole and the runtime then refuses it for not being a word. A key that writes
  punctuation in the layout being typed ends the word there and starts another:
  ``word!`` is served as ``word``, ``"ably"`` as ``ably``, and a Russian sentence
  typed in the US layout breaks at every ``!`` and ``?``. A key that is a letter
  in the other layout only is an ambiguous tail and comes off the end: ``French,``
  typed in the US layout is served as ``french``, while the same keys typed in
  Russian read ``Акутсрб`` - the comma key is the letter ``б`` there - and arrive
  whole, so the model is asked about ``french,``.

  Getting this wrong in either direction falsifies the evidence. Trimming a
  leading edge the engine keeps credited the model with recall it cannot deliver;
  keeping punctuation the engine splits on hid nine percent of the population,
  including the rows that turned out to hold real false conversions.

  Counting is a different question from serving and uses a different shape.
  ``word_core`` trims both edges down to the letters, because the character
  model is being taught which letter sequences a language produces, and ``and``
  is that evidence whether the sentence wrapped it in quotes or not. Every
  sequence the model is ever ASKED about is word-shaped, so the two shapes
  coincide wherever it matters.
* **The frozen word lists are counted.** They carry no new licence surface -
  they are the same Onboard lexicons already frozen for Layout Intent v1 - and
  they are the only available source of Russian orthotactics at scale: the CC0
  phrase snapshot gives Russian a third of the English material. They enter the
  character counts ONLY. They are lowercased, so they contain abbreviations
  without the capitals that protect them in real typing, and using them as
  negative examples would raise the threshold until ``htop`` stopped converting.

One family rule governs the split of what may be counted: a key sequence whose
family falls in the gate share contributes no character to training and is
scored exactly once, by the promotion test. That is a stronger guarantee than a
sentence split, because the unit this model generalises over is the sequence.
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

from keyswitch.layouts import LayoutPair  # noqa: E402

Script = Literal["en", "ru"]

ROOT: Final[Path] = Path(__file__).resolve().parents[1]
DIRECTORY: Final[Path] = ROOT / "model/ortho_v2"
TOKENS: Final[Path] = DIRECTORY / "tokens.jsonl.gz"
RECEIPT: Final[Path] = DIRECTORY / "corpus-receipt.json"
NAMESPACE: Final[str] = "keyswitch:ortho-v2:key-space-20260910b"
GATE_NAMESPACE: Final[str] = "keyswitch:ortho-v2:gate-20260910b"
GATE_SHARE: Final[int] = 16
CYRILLIC: Final[frozenset[str]] = frozenset("абвгдеёжзийклмнопрстуфхцчшщъыьэюя")
LATIN: Final[frozenset[str]] = frozenset("abcdefghijklmnopqrstuvwxyz")
MAXIMUM_TOKEN: Final[int] = 32
PAIR: Final[LayoutPair] = LayoutPair()
# Which keys write a letter in each layout. The Russian layout writes several of
# the US punctuation keys as letters, which is why a trailing comma survives into
# the token when the text was typed there and disappears when it was not.
LETTER_KEYS: Final[dict[str, frozenset[str]]] = {
    "en": frozenset(key for key in map(chr, range(32, 127)) if key.isalpha()),
    "ru": frozenset(key for key in map(chr, range(32, 127))
                    if PAIR.translate(key, "us", "ru").isalpha()),
}
LEXICONS: Final[dict[str, tuple[str, str]]] = {
    "en": ("model/intent_v1/sources/en_US.lm",
           "017a513a23bb37947a3a0e73e307066fa8ec0642075172cdcad6ea204cae68da"),
    "ru": ("model/intent_v1/sources/ru_RU.lm",
           "e57c14eec2b78e52a2125dce2b38044d8dfe480f3964b9d118614527195ceda9"),
}
MAXIMUM_LEXICON_WEIGHT: Final[int] = 32


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


# ЙЦУКЕН puts the full stop and the comma on the key the US layout renders as
# `/`, and the layout table does not model that: it maps both characters to
# themselves. Uncorrected, a Russian token ending in a comma becomes a key
# sequence ending in `,`, and reading that back gives `обманулиб` - a token
# nobody typed and that no engine can produce. Sixteen percent of the Russian
# tokens in the snapshot end this way, so the phantom is not a curiosity.
# The layout table itself is certified and unchanged; this override lives here,
# where a corpus is synthesised from text rather than observed from keystrokes.
RUSSIAN_PUNCTUATION_KEYS: Final[dict[str, str]] = {".": "/", ",": "/"}


def to_keys(token: str) -> str:
    """The physical keys a typist pressed to produce this token."""

    lowered = unicodedata.normalize("NFC", token).casefold()
    if script_of(token) != "ru":
        return lowered
    return "".join(
        RUSSIAN_PUNCTUATION_KEYS.get(character)
        or PAIR.translate(character, "ru", "us")
        for character in lowered
    )


def other(script: str) -> str:
    return "ru" if script == "en" else "en"


# The reverse of RUSSIAN_PUNCTUATION_KEYS: the key the US layout writes as `/`
# writes the full stop in Russian, and the layout table maps it to itself. Left
# alone, the corpus reads that key as a word character and never notices that the
# engine ends the word there.
RUSSIAN_KEY_CHARACTERS: Final[dict[str, str]] = {"/": "."}


def visible(keys: str, layout: str) -> str:
    """What the user sees on screen while typing these keys in this layout."""

    if layout == "en":
        return keys
    return "".join(
        RUSSIAN_KEY_CHARACTERS.get(key) or PAIR.translate(key, "us", "ru")
        for key in keys
    )


# Mirrors `KeySwitchEngine.PUNCTUATION` and the exemptions of `_is_boundary`.
# A copy rather than an import, because importing the engine would pull the
# backends into a corpus tool; `tests/test_ortho_model.py` fails if the two
# ever drift apart.
BOUNDARY_PUNCTUATION: Final[frozenset[str]] = frozenset(".,!?;:()[]{}—–-…\"«»")
BOUNDARY_EXEMPT: Final[frozenset[str]] = frozenset("_/\\@")
# `KeySwitchEngine` appends these to the word before it ever asks whether the
# key is a boundary, so a hyphen or an apostrophe inside a token never ends it:
# `Я-то`, `из-за` and `don't` reach a model whole.
WORD_JOINERS: Final[frozenset[str]] = frozenset("\'’-‐‑_@/\\=")


def ends_a_word(character: str) -> bool:
    """Would the engine commit the word at this character? (`_is_boundary`)"""

    return (character.isspace() or character in BOUNDARY_PUNCTUATION
            or (character not in BOUNDARY_EXEMPT
                and unicodedata.category(character[0]).startswith("P")))


def served(keys: str, layout: str) -> tuple[str, ...]:
    """Every token the engine hands to a model for these keys in this layout.

    One key sequence can become several: the engine commits a word at any key
    whose character is punctuation in the layout being typed, so `"ably"` is one
    token and `word!more` is two. A hyphen or an apostrophe is not such a key -
    the engine joins those to the word before it tests for a boundary at all, so
    `Я-то` and `don't` arrive whole. Within a segment two more of the engine's
    rules apply: the code guard keeps only what follows the last `/`
    (`_literal_head`), and a trailing key that is a letter only in the other
    layout is the ambiguous tail `_completed_word` splits off.

    Two approximations remain, both bounded and both in the safe direction.
    `_completed_word` asks its boundary model how much of that tail to take, and
    this takes all of it; `_literal_head` additionally requires the whole token to
    look like code, which is not reproduced here. Ambiguity is judged by the same
    layout table the engine converts with, so where a host keymap writes a shifted
    key as a letter this keeps it and the engine might split it - under-claiming
    recall rather than over-claiming it.
    """

    other_letters = LETTER_KEYS["ru" if layout == "en" else "en"]
    keep = LETTER_KEYS[layout]
    segments: list[str] = []
    current: list[str] = []
    for key in keys:
        if key in keep or key in other_letters:
            current.append(key)
            continue
        if current and visible(key, layout) in WORD_JOINERS:
            # Joined to the word before any boundary test, as the engine does.
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
    """The letters of the token, for counting rather than for serving.

    The character model is being taught which letter sequences a language
    produces. A word the source text wrapped in quotation marks is still that
    evidence, so the edges come off here even though the engine would keep them
    and refuse the token. Nothing the model is ever asked about is affected:
    every such sequence is already word-shaped, and this is a no-op on it.
    """

    keep = LETTER_KEYS[layout]
    start, end = 0, len(keys)
    while start < end and keys[start] not in keep:
        start += 1
    while end > start and keys[end - 1] not in keep:
        end -= 1
    return keys[start:end]


def gate_family(keys: str) -> bool:
    """True when this key sequence is reserved for the one-shot promotion test."""

    return int(hashlib.sha256(
        (GATE_NAMESPACE + ":" + keys).encode("utf-8")).hexdigest()[:8], 16) % 100 < GATE_SHARE


def shape_of(token: str, position: int) -> str:
    """Case shape, which is what makes the abbreviation reading available."""

    letters = [character for character in token if character.isalpha()]
    if len(letters) >= 2 and all(character.isupper() for character in letters):
        return "upper"
    if token[:1].isupper():
        return "initial" if position == 0 else "inner"
    return "lower"


def read_lexicon(path: Path, expected: str) -> dict[str, int]:
    """Unigrams of a frozen ARPA lexicon, refusing anything but the reviewed bytes."""

    content = path.read_bytes()
    digest = hashlib.sha256(content).hexdigest()
    if digest != expected:
        raise ValueError(f"frozen lexicon {path.name} checksum mismatch")
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
        if len(token) >= 2:
            words[token] = words.get(token, 0) + count
    return words


def lexicon_rows() -> dict[tuple[str, str], int]:
    """(script, keys) -> weight, compressed so a frequent word cannot dominate.

    A dictionary is evidence about which sequences a language admits, not about
    how often they are typed. The bit length keeps the ordering of the frequency
    list while flattening four orders of magnitude into five.
    """

    counts: dict[tuple[str, str], int] = {}
    for script, (path, expected) in sorted(LEXICONS.items()):
        for word, frequency in sorted(read_lexicon(ROOT / path, expected).items()):
            if script_of(word) != script:
                continue
            keys = to_keys(word)
            if not keys or len(keys) > MAXIMUM_TOKEN:
                continue
            weight = max(1, min(MAXIMUM_LEXICON_WEIGHT, int(frequency).bit_length()))
            key = (script, keys)
            counts[key] = max(counts.get(key, 0), weight)
    return counts


KNOWN_EVIDENCE: Final[Path] = DIRECTORY / "sources/known-evidence.json.gz"
_KNOWN: dict[str, frozenset[str]] = {}


def dictionary_known(script: str, keys: str) -> bool:
    """Does the dictionary of this layout know what the user sees?

    The orthotactic model exists for tokens no dictionary contains. When the
    dictionary of the current layout does know the token, the word models hold
    stronger evidence and their refusal must stand: `руку` is an ordinary Russian
    word whose keys read as the ordinary English word `here`, and no character
    model can or should separate the two.

    "Known" means what the runtime means by it - the frozen frequency list or
    Hunspell - read from the verdict `tools/ortho_v2_known.py` froze, because a
    replay cannot call a speller and get the same answer on every machine. Using
    the frequency list alone made this pessimistic rather than conservative:
    `ищи`, `куш` and `афиш` are ordinary Russian words it does not list, and
    counting them as negatives raised the Russian threshold by three nats over
    tokens the runtime never presents.
    """

    if script not in _KNOWN:
        with gzip.open(KNOWN_EVIDENCE, "rt", encoding="utf-8") as handle:
            payload = json.load(handle)
        for name, words in payload.items():
            _KNOWN[str(name)] = frozenset(map(str, words))
    return visible(keys, script) in _KNOWN[script]


def rows() -> list[dict[str, object]]:
    """One row per (split, script, key sequence) with occurrence and shape counts."""

    counts: dict[tuple[str, str, str], Counter[str]] = {}
    for assigned in context_corpus.assign(context_corpus.load_source())[0]:
        words = unicodedata.normalize("NFC", assigned.phrase.text).split()
        for position, raw in enumerate(words):
            if len(raw) > MAXIMUM_TOKEN:
                continue
            script = script_of(raw)
            if script is None:
                continue
            keys = to_keys(raw)
            if not keys:
                continue
            key = (assigned.split, script, keys)
            shape = counts.setdefault(key, Counter())
            shape[shape_of(raw, position)] += 1
    payload: list[dict[str, object]] = [
        {"split": split, "script": script, "keys": keys,
         "shapes": {name: shape[name] for name in sorted(shape)}}
        for (split, script, keys), shape in sorted(counts.items())
    ]
    payload.extend(
        {"split": "lexicon", "script": script, "keys": keys, "shapes": {"lower": weight}}
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
    paths = (
        ROOT / "tools/ortho_v2_corpus.py",
        ROOT / "tools/context_corpus.py",
        ROOT / "src/keyswitch/layouts.py",
        context_corpus.SOURCE,
        context_corpus.RECEIPT,
        *(ROOT / path for path, _expected in sorted(LEXICONS.values())),
    )
    return {path.relative_to(ROOT).as_posix(): checksum(path) for path in paths}


def report(payload: list[dict[str, object]], tokens_sha: str) -> dict[str, object]:
    by_split: Counter[str] = Counter()
    by_script: Counter[str] = Counter()
    occurrences: Counter[str] = Counter()
    gated: Counter[str] = Counter()
    for row in payload:
        split, script, keys = str(row["split"]), str(row["script"]), str(row["keys"])
        shapes = row["shapes"]
        assert isinstance(shapes, dict)
        by_split[split] += 1
        by_script[f"{split}:{script}"] += 1
        occurrences[f"{split}:{script}"] += sum(int(value) for value in shapes.values())
        if gate_family(keys):
            gated[f"{split}:{script}"] += 1
    return {
        "schema_version": 1,
        "namespace": NAMESPACE,
        "description": (
            "Key-space token types from the frozen CC0 phrase snapshot and the frozen "
            "Onboard lexicons, stored whole. What the engine presents is the token "
            "trimmed for the layout being typed, which `presented` derives. "
            "Lexicon rows carry compressed frequency weight and are for character "
            "counts only; they are lowercased and must never be used as negatives."
        ),
        "gate": {
            "namespace": GATE_NAMESPACE, "share_percent": GATE_SHARE,
            "rule": "sha256(namespace + ':' + keys)[:8] mod 100 < share",
            "meaning": "excluded from every count; scored once by the promotion test",
            "types_by_split_script": dict(sorted(gated.items())),
        },
        "lexicon_weight": "max(1, min(32, frequency.bit_length()))",
        "types_by_split": dict(sorted(by_split.items())),
        "types_by_split_script": dict(sorted(by_script.items())),
        "occurrences_by_split_script": dict(sorted(occurrences.items())),
        "maximum_token_characters": MAXIMUM_TOKEN,
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
    content = (json.dumps(report(payload, tokens_sha), sort_keys=True,
                          ensure_ascii=False, indent=2) + "\n").encode()
    if arguments.freeze and not RECEIPT.exists():
        RECEIPT.write_bytes(content)
    elif not RECEIPT.exists() or RECEIPT.read_bytes() != content:
        raise ValueError("corpus receipt is missing or changed; do not overwrite a frozen split")
    print(content.decode("utf-8"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
