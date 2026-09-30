#!/usr/bin/env python3
"""Type public text the way a person switches layouts, through the engine.

A person types every token with the physical keys of its intended layout (Cyrillic -> RU,
Latin -> EN; digits and signs in whatever layout is on) and selects the layout by hand where the
language changes; the engine sees that as a manual layout change, as in the program. Typing modes:

  correct      every change is made: any change of the text is a false intervention;
  forget_ru    one change from English back to Russian is forgotten (the person believes they made
               it and selects the next layout as usual): the Russian run comes out in Latin;
  forget_en    one change from Russian to English is forgotten;
  start_wrong  the message starts in the other layout;
  insert       letters removed from inside a word are typed back after a click, in the other layout.

Commands:
  messages  --so <ru_stackoverflow.jsonl.zst> --tatoeba <dir with rus_/eng_sentences.tsv.bz2>
            [--taiga <dir with ru_taiga-ud-*.conllu>] --out <dir>: messages of the sources and code lines,
            split by thread or sentence id (train/dev/test; Taiga by its own parts);
  terms     --messages <dir> --out <context-term-frequency.json>
            how often plain words occur inside Russian text and in text of their own language
            (train splits only; the Taiga messages are needed too);
  capture   --manifest <model/context_v1/captured/manifest.json> --messages <dir> --model <artifact>
            [--only NAME] [--verify]: the questions the engine asks the context model while each
            source's recipe is typed, labelled from what the person meant;
  sources   --manifest <model/context_v1/captured/manifest.json> --messages <dir> --tatoeba <dir with
            rus_/eng_sentences_detailed.tsv.bz2>: whose text the captured files hold - the questions of
            ru.stackoverflow, the Tatoeba sentences with their authors and the Taiga sentence ids;
  evaluate  --root <tree> --messages <file> --count N --output <rows.json> [--model] [--typo-rate R]
            [--prefix-rate R --code <file>] [--kinds k,k]: final text against intended text.

Test-split messages are never typed by `capture`; `evaluate` reads whatever file it is given.
"""

from __future__ import annotations

import argparse
import bz2
import gzip
import hashlib
import html
import json
import lzma
import re
import subprocess
import sys
from collections import Counter
from collections.abc import Callable, Iterator, Sequence
from concurrent.futures import ProcessPoolExecutor
from contextlib import AbstractContextManager, ExitStack
from dataclasses import replace
from pathlib import Path
from typing import Final, Protocol, cast
from unittest.mock import patch

ROOT: Final = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "src"), str(ROOT / "tests")]

from keyswitch.constants.file_formats import HEXADECIMAL_BASE  # noqa: E402
from keyswitch.constants.models import ACTION_FEATURE_AFTER_CONTEXT_CHARACTERS  # noqa: E402
from keyswitch.constants.corpus import (  # noqa: E402
    MIXED_CAPTURE_XZ_PRESET, MIXED_EVALUATE_RUSSIAN_SHARE, MIXED_PREFIX_KINDS, TATOEBA_AUTHOR_COLUMN, TATOEBA_TEXT_COLUMN,
    MIXED_CODE_LINE_MAX_CHARACTERS, MIXED_CODE_LINE_MIN_CHARACTERS, MIXED_DEVELOPMENT_SPLIT_BUCKETS,
    MIXED_DRAW_HEX_DIGITS, MIXED_DRAW_RESOLUTION, MIXED_EDIT_CLOCK_START_SECONDS, MIXED_EDIT_KEY_INTERVAL_SECONDS,
    MIXED_EDIT_LENGTH_SHIFT, MIXED_EDIT_MAX_REMOVED_LETTERS, MIXED_EDIT_PAUSE_SECONDS, MIXED_EDIT_PLACE_SHIFT,
    MIXED_EDIT_TWO_LETTERS_FROM, MIXED_EDIT_WORD_MIN_LETTERS, MIXED_MESSAGE_MAX_TOKENS, MIXED_MESSAGE_MIN_TOKENS,
    MIXED_PREFIX_CODE_LINES, MIXED_SPLIT_HEX_DIGITS, MIXED_TEST_SPLIT_BUCKETS, MIXED_TYPING_CHUNK_JOBS,
    MIXED_TYPING_DEFAULT_WORKERS, MIXED_TYPO_HEX_DIGITS, MIXED_TYPO_MIN_LETTERS,
    MIXED_TYPO_NEIGHBOUR_SHIFT, MIXED_TYPO_PLACE_SHIFT, SPLIT_BUCKET_COUNT, TATOEBA_SENTENCE_MAX_TOKENS,
    TATOEBA_SENTENCE_MIN_TOKENS,
)
from keyswitch.constants.models import CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS  # noqa: E402
from keyswitch.context_model import ContextEvidence, ContextModel, ContextPrediction  # noqa: E402
from keyswitch.input_context import CONTEXT_LIMIT, FieldContext  # noqa: E402
from test_input_sequence_matrix import PhysicalSession  # noqa: E402

TYPING_MODES: Final = ("correct", "forget_ru", "forget_en", "start_wrong")
EDIT_MODE: Final = "insert"
CYRILLIC: Final = "а-яёА-ЯЁ"
LATIN: Final = "a-zA-Z"
TOKEN: Final = re.compile(rf"^[{CYRILLIC}{LATIN}0-9.,!?:;\-()'\"/_=+*<>\[\]{{}}#@$%&|\\~^`]+$")
HAS_CYRILLIC: Final = re.compile(rf"[{CYRILLIC}]")
HAS_LATIN: Final = re.compile(rf"[{LATIN}]")
CODE_FENCE: Final = re.compile(r"```.*?```", re.S)
CODE_FENCE_BODY: Final = re.compile(r"```[^\n]*\n(.*?)```", re.S)
INDENTED: Final = re.compile(r"^(?: {4}|\t).*$", re.M)
LINK: Final = re.compile(r"!?\[([^\]]*)\]\([^)]*\)")
URL: Final = re.compile(r"(?:https?://|www\.)\S+")
TAG: Final = re.compile(r"<[^>]+>")
MENTION: Final = re.compile(r"(?:^|\s)@[\w.\-]+")
SENTENCE: Final = re.compile(r"(?<=[.!?])\s+")
CODE_LINE: Final = re.compile(r"^[\x20-\x7e]+$")
TYPEABLE: Final = str.maketrans({"«": '"', "»": '"', "“": '"', "”": '"', "„": '"', "’": "'", "‘": "'", "—": "-", "–": "-",
                                 "…": "...", " ": " ", " ": " ", " ": " "})
TERM_EDGE_SIGNS: Final = ".,!?:;\"'()[]{}<>«»-"
US_ROWS: Final = ("qwertyuiop[]", "asdfghjkl;'", "zxcvbnm,./")
RU_ROWS: Final = ("йцукенгшщзхъ", "фывапролджэ", "ячсмитьбю.")
CAPTURED_FIELDS: Final = (
    "original", "alternative", "source_group", "trigger", "baseline_convert", "source_known", "target_known",
    "score_delta", "literal_tail", "boundary_text", "after_origin", "source_typo", "target_typo",
    "source_opening", "target_opening", "inside",
)

Job = tuple[int, str, str, int, str]
Step = tuple[str, int, int | None]


def _draw(text: str, digits: int = MIXED_DRAW_HEX_DIGITS) -> int:
    return int(hashlib.sha256(text.encode()).hexdigest()[:digits], HEXADECIMAL_BASE)


def split_of(namespace: str, identifier: str) -> str:
    bucket = _draw(f"{namespace}:{identifier}", MIXED_SPLIT_HEX_DIGITS) % SPLIT_BUCKET_COUNT
    return "test" if bucket < MIXED_TEST_SPLIT_BUCKETS else "dev" if bucket < MIXED_DEVELOPMENT_SPLIT_BUCKETS else "train"


# --- messages ------------------------------------------------------------------------------------

def clean(text: str) -> str:
    text = CODE_FENCE.sub(" ", text)
    text = INDENTED.sub(" ", text)
    text = LINK.sub(r"\1", text)
    text = URL.sub(" ", text)
    text = TAG.sub(" ", html.unescape(text))
    text = MENTION.sub(" ", text)
    text = text.replace("`", "").replace("**", "").replace("__", " ")
    return text.translate(TYPEABLE)


def typeable_tokens(tokens: Sequence[str]) -> bool:
    return all(TOKEN.match(token) and not (HAS_CYRILLIC.search(token) and HAS_LATIN.search(token)) for token in tokens)


def post_messages(text: str) -> list[list[str]]:
    """Messages of one post: typeable, with Cyrillic, of a chat message's length."""

    out: list[list[str]] = []
    for paragraph in re.split(r"\n\s*\n|\n", clean(text)):
        paragraph = paragraph.strip().lstrip("#>*- ").strip()
        if not paragraph:
            continue
        parts = [paragraph] if len(paragraph.split()) <= MIXED_MESSAGE_MAX_TOKENS else SENTENCE.split(paragraph)
        for part in parts:
            tokens = part.split()
            if (MIXED_MESSAGE_MIN_TOKENS <= len(tokens) <= MIXED_MESSAGE_MAX_TOKENS and typeable_tokens(tokens)
                    and any(HAS_CYRILLIC.search(token) for token in tokens)):
                out.append(tokens)
    return out


def code_lines(markdown: str) -> list[str]:
    lines: list[str] = []
    for block in CODE_FENCE_BODY.findall(markdown):
        lines += block.split("\n")
    lines += [line[len("    "):] for line in markdown.split("\n") if line.startswith("    ")]
    return [line.strip() for line in lines
            if MIXED_CODE_LINE_MIN_CHARACTERS <= len(line.strip()) <= MIXED_CODE_LINE_MAX_CHARACTERS
            and CODE_LINE.match(line.strip()) and re.search(r"[A-Za-z]{2}", line)]


def build_taiga(taiga: Path, out: Path) -> dict[str, int]:
    """Sentences of UD Russian-Taiga by its own parts: Russian with names, slang and rare words."""

    counts: Counter[str] = Counter()
    for lower in (False, True):
        name = "taiga-chat" if lower else "taiga"
        for part, files in (("train", sorted(taiga.glob("ru_taiga-ud-train-*.conllu"))), ("dev", [taiga / "ru_taiga-ud-dev.conllu"]),
                            ("test", [taiga / "ru_taiga-ud-test.conllu"])):
            number = 0
            with gzip.open(out / f"{name}-{part}.jsonl.gz", "wt", encoding="utf-8") as handle:
                for path in files:
                    sentence_id = ""
                    for line in path.open(encoding="utf-8"):
                        if line.startswith("# sent_id = "):
                            sentence_id = line[len("# sent_id = "):].strip()
                        if not line.startswith("# text = "):
                            continue
                        number += 1
                        tokens = line[len("# text = "):].translate(TYPEABLE).split()
                        joined = " ".join(tokens)
                        if lower:
                            # Chat style: lowercase, no final sign.
                            joined = joined.lower().rstrip(".!?…").strip()
                            tokens = joined.split()
                        if (not MIXED_MESSAGE_MIN_TOKENS <= len(tokens) <= MIXED_MESSAGE_MAX_TOKENS or not typeable_tokens(tokens)
                                or not any(HAS_CYRILLIC.search(token) for token in tokens)):
                            continue
                        mix = "mixed" if any(HAS_LATIN.search(token) for token in tokens) else "russian"
                        handle.write(json.dumps({"qid": number, "sid": sentence_id, "kind": name.replace("-", "_"), "mix": mix,
                                                 "text": joined}, ensure_ascii=False) + "\n")
                        counts[f"{name}:{part}"] += 1
    return dict(counts)


BARE_SIGNS: Final = re.compile(r"[,.!?…:;\"()]|(?<!\w)-|-(?!\w)")


def build_tatoeba_bare(tatoeba: Path, out: Path) -> dict[str, int]:
    """Tatoeba sentences as chat lines are often typed: lowercase, no signs; only words of letters of the language.

    The same parts as the other two styles (split by sentence id); a sentence with a digit, an apostrophe or a
    word of the other script is left out.
    """

    counts: Counter[str] = Counter()
    files = {part: gzip.open(out / f"tatoeba-bare-{part}.jsonl.gz", "wt", encoding="utf-8") for part in ("train", "dev", "test")}
    for language, cyrillic in (("rus", True), ("eng", False)):
        word = re.compile(rf"[{CYRILLIC if cyrillic else LATIN}]+")
        for line in bz2.open(tatoeba / f"{language}_sentences.tsv.bz2", "rt", encoding="utf-8"):
            fields = line.rstrip("\n").split("\t")
            if len(fields) <= TATOEBA_TEXT_COLUMN:
                continue
            sentence_id = fields[0]
            tokens = BARE_SIGNS.sub(" ", fields[TATOEBA_TEXT_COLUMN].translate(TYPEABLE).lower()).split()
            if not TATOEBA_SENTENCE_MIN_TOKENS <= len(tokens) <= TATOEBA_SENTENCE_MAX_TOKENS or not all(word.fullmatch(token) for token in tokens):
                continue
            part = split_of(f"keyswitch:research-2026-09-29:tatoeba:{language}", sentence_id)
            files[part].write(json.dumps({"qid": int(sentence_id), "kind": "tatoeba_bare", "mix": "russian" if cyrillic else "mixed",
                                          "text": " ".join(tokens)}, ensure_ascii=False) + "\n")
            counts[f"tatoeba-bare:{part}"] += 1
    for handle in files.values():
        handle.close()
    return dict(counts)


def build_messages(so: Path, tatoeba: Path, out: Path) -> dict[str, int]:
    out.mkdir(parents=True, exist_ok=True)
    counts: Counter[str] = Counter()
    handles = {name: gzip.open(out / f"so-{name}.jsonl.gz", "wt", encoding="utf-8") for name in ("train", "dev", "test")}
    code = {name: gzip.open(out / f"code-{name}.txt.gz", "wt", encoding="utf-8") for name in ("train", "dev", "test")}
    # The question each code line came from, line for line: whose text it is (`sources`).
    code_ids = {name: gzip.open(out / f"code-{name}.qids.txt.gz", "wt", encoding="utf-8") for name in ("train", "dev", "test")}
    seen: set[str] = set()
    seen_code: set[str] = set()
    process = subprocess.Popen(["zstd", "-dc", str(so)], stdout=subprocess.PIPE)
    assert process.stdout is not None
    for raw in process.stdout:
        record = json.loads(raw)
        qid = int(record["question_id"])
        part = split_of("keyswitch:research-2026-09-29:ru_stackoverflow", str(qid))
        pieces = [("title", record.get("title") or "")] + [("comment", c.get("text") or "") for c in record.get("comments") or []]
        pieces.append(("prose", record.get("text_markdown") or ""))
        markdowns = [record.get("text_markdown") or ""]
        for answer in record.get("answers") or []:
            pieces.append(("prose", answer.get("text_markdown") or ""))
            pieces += [("comment", c.get("text") or "") for c in answer.get("comments") or []]
            markdowns.append(answer.get("text_markdown") or "")
        for kind, text in pieces:
            for tokens in post_messages(text):
                joined = " ".join(tokens)
                if joined not in seen:
                    seen.add(joined)
                    mix = "mixed" if any(HAS_LATIN.search(token) for token in tokens) else "russian"
                    handles[part].write(json.dumps({"qid": qid, "kind": kind, "mix": mix, "text": joined}, ensure_ascii=False) + "\n")
                    counts[f"so:{part}"] += 1
        for markdown in markdowns:
            for line in code_lines(markdown):
                if line not in seen_code:
                    seen_code.add(line)
                    code[part].write(line + "\n")
                    code_ids[part].write(f"{qid}\n")
                    counts[f"code:{part}"] += 1
    for handle in (*handles.values(), *code.values(), *code_ids.values()):
        handle.close()
    for lower in (False, True):
        name = "tatoeba-chat" if lower else "tatoeba"
        files = {part: gzip.open(out / f"{name}-{part}.jsonl.gz", "wt", encoding="utf-8") for part in ("train", "dev", "test")}
        for language, cyrillic in (("rus", True), ("eng", False)):
            for line in bz2.open(tatoeba / f"{language}_sentences.tsv.bz2", "rt", encoding="utf-8"):
                fields = line.rstrip("\n").split("\t")
                if len(fields) <= TATOEBA_TEXT_COLUMN:
                    continue
                sentence_id, text = fields[0], fields[TATOEBA_TEXT_COLUMN].translate(TYPEABLE).strip()
                tokens = text.split()
                if not TATOEBA_SENTENCE_MIN_TOKENS <= len(tokens) <= TATOEBA_SENTENCE_MAX_TOKENS or not typeable_tokens(tokens):
                    continue
                if any((HAS_LATIN if cyrillic else HAS_CYRILLIC).search(token) for token in tokens):
                    continue
                if not any((HAS_CYRILLIC if cyrillic else HAS_LATIN).search(token) for token in tokens):
                    continue
                part = split_of(f"keyswitch:research-2026-09-29:tatoeba:{language}", sentence_id)
                joined = " ".join(tokens)
                if lower:
                    # Chat style: lowercase, no final sign.
                    joined = joined.lower().rstrip(".!?…").strip()
                    if len(joined.split()) < TATOEBA_SENTENCE_MIN_TOKENS:
                        continue
                files[part].write(json.dumps({"qid": int(sentence_id), "kind": name.replace("-", "_"),
                                              "mix": "russian" if cyrillic else "mixed", "text": joined}, ensure_ascii=False) + "\n")
                counts[f"{name}:{part}"] += 1
        for handle in files.values():
            handle.close()
    return dict(counts)


def build_terms(messages: Path, lexicon: Callable[[str], bool]) -> dict[str, dict[str, int]]:
    """How often plain words occur in the training parts.

    Inside Russian text: Latin words of the SO messages (`latin`), Cyrillic words outside the
    lexicon of the SO messages and the Russian Tatoeba sentences (`cyrillic`). In text of their
    own language: words of the English Tatoeba sentences (`english`), Cyrillic words of all the
    Russian text - SO, Tatoeba and Taiga - the lexicon's words included (`russian`).
    """

    latin: Counter[str] = Counter()
    cyrillic: Counter[str] = Counter()
    english: Counter[str] = Counter()
    russian: Counter[str] = Counter()
    for name in ("so-train.jsonl.gz", "tatoeba-train.jsonl.gz", "taiga-train.jsonl.gz"):
        for line in gzip.open(messages / name, "rt", encoding="utf-8"):
            row = json.loads(line)
            english_sentence = name.startswith("tatoeba") and row["mix"] != "russian"
            for token in row["text"].split():
                core = token.strip(TERM_EDGE_SIGNS).lower()
                if re.fullmatch(r"[a-z]+", core):
                    if name.startswith("so"):
                        latin[core] += 1
                    elif english_sentence:
                        english[core] += 1
                elif re.fullmatch(r"[а-яё]+", core):
                    russian[core] += 1
                    if not name.startswith("taiga"):
                        cyrillic[core] += 1
    least = CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS[0]

    def frequent(counts: Counter[str]) -> dict[str, int]:
        return {word: count for word, count in counts.items() if count >= least}
    return {"latin": frequent(latin), "cyrillic": {word: count for word, count in frequent(cyrillic).items() if not lexicon(word)},
            "english": frequent(english), "russian": frequent(russian)}


# --- typing --------------------------------------------------------------------------------------

def script(token: str) -> int | None:
    if HAS_CYRILLIC.search(token):
        return 1
    if HAS_LATIN.search(token):
        return 0
    return None


def plan(tokens: Sequence[str], mode: str, choice: int) -> tuple[int, list[Step]] | None:
    """The starting layout and, per token, (token, intended layout, layout selected by hand before it)."""

    intended: list[int] = []
    current: int | None = None
    for token in tokens:
        group = script(token)
        current = group if group is not None else current
        intended.append(current if current is not None else -1)
    first = next((group for group in intended if group >= 0), None)
    if first is None:
        return None
    intended = [group if group >= 0 else first for group in intended]
    transitions = [index for index in range(1, len(tokens)) if intended[index] != intended[index - 1]]
    options = ([index for index in transitions if intended[index] == 1] if mode == "forget_ru"
               else [index for index in transitions if intended[index] == 0] if mode == "forget_en"
               else [0] if mode == "start_wrong" else [-1])
    if not options:
        return None
    forgotten = options[choice % len(options)]
    start = (1 - first) if mode == "start_wrong" else first
    return start, [(token, intended[index], intended[index] if index in transitions and index != forgotten else None)
                   for index, token in enumerate(tokens)]


def _neighbours(char: str) -> str:
    lower = char.lower()
    for rows in (US_ROWS, RU_ROWS):
        for row in rows:
            index = row.find(lower)
            if index >= 0:
                near = "".join(c for c in (row[index - 1] if index else "") + row[index + 1:][:1] if c.isalpha())
                return near.upper() if char.isupper() else near
    return ""


def typo(token: str, seed: str) -> str:
    """One typo among a token's letters: a letter missed, two swapped, one pressed twice, or the key next to it."""

    positions = [index for index, char in enumerate(token) if char.isalpha()]
    if len(positions) < MIXED_TYPO_MIN_LETTERS:
        return token
    digest = _draw(seed, MIXED_TYPO_HEX_DIGITS)
    kinds = ("missed", "swapped", "doubled", "neighbour")
    kind = kinds[digest % len(kinds)]
    at = positions[(digest >> MIXED_TYPO_PLACE_SHIFT) % len(positions)]
    if kind == "missed" and len(positions) > MIXED_TYPO_MIN_LETTERS:
        return token[:at] + token[at + 1:]
    if kind == "swapped" and at + 1 < len(token) and token[at + 1].isalpha():
        return token[:at] + token[at + 1] + token[at] + token[at + 1:][1:]
    if kind == "doubled":
        return token[:at + 1] + token[at] + token[at + 1:]
    near = _neighbours(token[at])
    if near:
        return token[:at] + near[(digest >> MIXED_TYPO_NEIGHBOUR_SHIFT) % len(near)] + token[at + 1:]
    return token[:at + 1] + token[at] + token[at + 1:]


def with_typos(text: str, rate: float, seed: str) -> str:
    largest = HEXADECIMAL_BASE ** MIXED_DRAW_HEX_DIGITS - 1
    return " ".join(typo(token, f"{seed}:{index}:t") if _draw(f"{seed}:{index}") / largest < rate else token
                    for index, token in enumerate(text.split()))


Spy = Callable[[ContextEvidence], ContextPrediction]
Recipe = dict[str, object]
_state: dict[str, object] = {}


class Editor(Protocol):
    text: str
    caret: int


class Reader:
    """The field as a field reader reports it: the text before the caret, and after it when editing."""

    status = "available"

    def __init__(self, backend: Editor, *, after: bool = False) -> None:
        self.backend, self.after = backend, after

    def read(self, application: str, window: int) -> FieldContext:
        text, caret = self.backend.text, self.backend.caret
        if self.after:
            return FieldContext(application, "1", text[:caret][-CONTEXT_LIMIT:], text[caret:][:ACTION_FEATURE_AFTER_CONTEXT_CHARACTERS],
                                role="text", source="uia")
        return FieldContext(application, "1", text[-CONTEXT_LIMIT:], "", role="text", source="native")


def _init(model_path: str) -> None:
    from keyswitch.language_model import LanguageModel
    import test_input_sequence_matrix as sequences
    payload = json.loads(Path(model_path).read_text(encoding="utf-8"))
    weights = {name: tuple(float(value) for value in values) for name, values in payload["weights"].items()}
    _state["model"] = ContextModel(weights, payload["version"], float(payload["conversion_threshold"]),
                                   feature_version=payload["feature_version"])
    original = LanguageModel.load.__func__  # type: ignore[attr-defined]
    memo: dict[tuple[str, tuple[str, ...]], LanguageModel] = {}

    def load(cls: type[LanguageModel], locale: str, extra_words: Sequence[str] = ()) -> LanguageModel:
        key = (locale, tuple(extra_words))
        if key not in memo:
            memo[key] = original(cls, locale, key[1])
        return memo[key]
    LanguageModel.load = classmethod(load)  # type: ignore[method-assign,assignment]
    with sequences.session(1) as current:
        inverse: dict[str, str] = {}
        for code in range(ord("!"), ord("~") + 1):
            inverse.setdefault(current.key(chr(code)).character, chr(code))
    _state["ru_keys"] = inverse


def keys_for(token: str, group: int) -> str | None:
    if group == 0:
        return token
    inverse = cast(dict[str, str], _state["ru_keys"])
    keys = [inverse.get(character) for character in token]
    return None if any(key is None for key in keys) else "".join(cast(list[str], keys))


def edit_point(text: str) -> tuple[int, int, int] | None:
    """A deterministic gap inside one word of the message: (start, length, layout of the word)."""

    words, position = [], 0
    for word in text.split(" "):
        group = script(word)
        if len(word) >= MIXED_EDIT_WORD_MIN_LETTERS and word.isalpha() and group is not None:
            words.append((position, word, group))
        position += len(word) + 1
    if not words:
        return None
    digest = _draw(text)
    offset, word, group = words[digest % len(words)]
    length = 1 + (digest >> MIXED_EDIT_LENGTH_SHIFT) % MIXED_EDIT_MAX_REMOVED_LETTERS if len(word) >= MIXED_EDIT_TWO_LETTERS_FROM else 1
    inner = 1 + (digest >> MIXED_EDIT_PLACE_SHIFT) % (len(word) - length - 1)
    return offset + inner, length, group


def _session(start: int, spy: Spy | None, clock: list[float] | None = None) -> tuple[ExitStack, PhysicalSession]:
    """An engine on an editor, the model patched in; typing respects manual layout changes, editing does not."""

    import test_input_sequence_matrix as sequences
    model = cast(ContextModel, _state["model"])
    stack = ExitStack()
    stack.enter_context(cast(AbstractContextManager[object], patch("keyswitch.context_policy.ContextModel.try_load",
                                                                   return_value=(model, model.version))))
    if clock is not None:
        stack.enter_context(cast(AbstractContextManager[object], patch("keyswitch.engine.time.monotonic", side_effect=lambda: clock[0])))
    if spy is not None:
        stack.enter_context(cast(AbstractContextManager[object], patch.object(model, "predict", spy)))
    current = stack.enter_context(sequences.session(start))
    if clock is None:
        current.settings.set("detection.respect_manual_layout", True)
    current.settings.set("detection.context_read_field", True)
    current.backend.active_application = lambda: "Telegram"  # type: ignore[method-assign]
    return stack, current


def _type_message(job: Job, spy_factory: Callable[[list[tuple[str, int, int]]], Spy] | None) -> str | None:
    """Type one message in one mode; the final text, or None if it cannot be typed."""

    _index, text, mode, choice, prefix = job
    planned = plan(text.split(), mode, choice)
    if planned is None:
        return None
    start, steps = planned
    typed_tokens: list[tuple[str, int, int]] = []
    stack, current = _session(start, spy_factory(typed_tokens) if spy_factory else None)
    with stack:
        current.engine.context_policy.reader = Reader(current.backend)
        if prefix:
            current.backend.text, current.backend.caret = prefix, len(prefix)
        for token, intended, selected in steps:
            if selected is not None and current.backend.group != selected:
                current.backend.group = selected
                current.engine._observe_group(selected, source="hotkey")
            keys = keys_for(token, intended)
            if keys is None:
                return None
            group = current.backend.group
            typed_tokens.append(("".join(current.key(key).character for key in keys).casefold(), group, intended))
            current.physical(keys + " ")
            current.flush()
        return current.backend.text


def _edit_message(job: Job, spy: Spy | None) -> str | None:
    """Letters removed from inside a word are typed back after a click in the other layout."""

    from keyswitch.backend import KeyEvent
    _index, text, _mode, _choice, prefix = job
    point = edit_point(text)
    if point is None:
        return None
    start, length, own = point
    keys = keys_for(text[start:start + length], own)
    if keys is None:
        return None
    clock = [MIXED_EDIT_CLOCK_START_SECONDS]
    stack, current = _session(1 - own, spy, clock)
    with stack:
        current.engine.context_policy.reader = Reader(current.backend, after=True)
        current.backend.text = prefix + text[:start] + text[start + length:]
        current.backend.caret = len(prefix) + start
        pointer = KeyEvent(True, 0, "Pointer", "", ("", ""), current.backend.group, 0, 0)
        current.engine._handle(pointer)
        current.engine._handle(replace(pointer, pressed=False))
        for key in keys:
            clock[0] += MIXED_EDIT_KEY_INTERVAL_SECONDS
            current.tap(current.key(key))
        clock[0] += MIXED_EDIT_PAUSE_SECONDS
        current.engine._maybe_correct_after_pause()
        current.flush()
        return current.backend.text


def _row(item: ContextEvidence, label: str) -> list[object]:
    return [*(getattr(item, name) for name in CAPTURED_FIELDS), item.field.before, item.field.after, item.field.role, label]


def _capture(job: Job) -> list[list[object]]:
    model = cast(ContextModel, _state["model"])
    predict = model.predict
    rows: list[list[object]] = []
    _index, _text, mode, _choice, _prefix = job
    if mode == EDIT_MODE:
        def edit_spy(item: ContextEvidence) -> ContextPrediction:
            result = predict(item)
            if item.inside:
                rows.append(_row(item, "convert"))
            return result
        _edit_message(job, edit_spy)
        return rows

    def factory(typed: list[tuple[str, int, int]]) -> Spy:
        def spy(item: ContextEvidence) -> ContextPrediction:
            result = predict(item)
            core = item.original.casefold()
            for text, group, intended in reversed(typed):
                if core and core in text and item.source_group == group:
                    rows.append(_row(item, "convert" if group != intended else "keep"))
                    break
            return result
        return spy
    # A message with a token it cannot type gives nothing, not the questions before that token.
    return [] if _type_message(job, factory) is None else rows


def _evaluate(job: Job) -> dict[str, object]:
    index, text, mode, _choice, prefix = job
    got = _edit_message(job, None) if mode == EDIT_MODE else _type_message(job, None)
    if got is None:
        return {"index": index, "mode": mode, "skipped": True}
    expected = prefix + text + ("" if mode == EDIT_MODE else " ")
    return {"index": index, "mode": mode, "expected": expected, "got": got, "ok": got == expected}


def recipe_messages(messages: Path, recipe: Recipe) -> list[tuple[str, int]]:
    """The recipe's messages by seed, mixed ones first, as typed (typos included), with their ids."""

    seed = str(recipe["seed"])
    kinds = set(cast(list[str], recipe["kinds"]))
    pool: list[tuple[str, str, str, int]] = []
    for line in gzip.open(messages, "rt", encoding="utf-8"):
        row = json.loads(line)
        if row["kind"] in kinds:
            pool.append((hashlib.sha256(f"{seed}:{row['text']}".encode()).hexdigest(), row["text"], row["mix"], int(row["qid"])))
    pool.sort()
    count = int(cast(int, recipe["count"]))
    chosen = ([(text, qid) for _d, text, mix, qid in pool if mix == "mixed"][:count]
              + [(text, qid) for _d, text, mix, qid in pool if mix == "russian"][:int(count * float(cast(float, recipe["russian_share"])))])
    typo_rate = float(cast(float, recipe.get("typo_rate", 0.0)))
    if typo_rate:
        chosen = [(with_typos(text, typo_rate, f"{seed}:typo:{text}"), qid) for text, qid in chosen]
    return chosen


def previous_line(recipe: Recipe, text: str, index: int, messages: int, code_lines: int,
                  lines_above: int = 0) -> tuple[str, int] | None:
    """What stands on the line above a message: ("code", line), ("message", index) or ("above", index), by draw."""

    draw = _draw(f"{recipe['seed']}:prefix:{text}")
    if not draw % MIXED_DRAW_RESOLUTION < float(cast(float, recipe.get("prefix_rate", 0.0))) * MIXED_DRAW_RESOLUTION:
        return None
    half = draw // MIXED_PREFIX_KINDS
    if code_lines and draw % MIXED_PREFIX_KINDS == 0:
        return "code", half % code_lines
    if lines_above:
        return "above", half % lines_above
    return "message", (half + index + 1) % messages


def lines_above(messages: Path, recipe: Recipe) -> list[tuple[str, int]]:
    """Messages of another pool a recipe puts on the line above its own (`lines_above`), by seed."""

    pool = cast(dict[str, object], recipe.get("lines_above", {}))
    if not pool:
        return []
    kinds = set(cast(list[str], pool["kinds"]))
    seed, mix = str(pool["seed"]), str(pool["mix"])
    chosen: list[tuple[str, str, int]] = []
    for line in gzip.open(messages.parent / str(pool["messages"]), "rt", encoding="utf-8"):
        row = json.loads(line)
        if row["kind"] in kinds and row["mix"] == mix:
            chosen.append((hashlib.sha256(f"{seed}:{row['text']}".encode()).hexdigest(), row["text"], int(row["qid"])))
    chosen.sort()
    return [(text, qid) for _digest, text, qid in chosen[:int(cast(int, pool["count"]))]]


def _code_lines(path: Path | None, recipe: Recipe) -> list[str]:
    if path is None or not recipe.get("prefix_rate"):
        return []
    with gzip.open(path, "rt", encoding="utf-8") as handle:
        return [line.rstrip("\n") for _n, line in zip(range(MIXED_PREFIX_CODE_LINES), handle)]


def jobs_for(messages: Path, recipe: Recipe, code_path: Path | None) -> list[Job]:
    """The recipe's messages (by seed), typos and previous lines, and one job per mode."""

    everyone = [text for text, _qid in recipe_messages(messages, recipe)]
    code = _code_lines(code_path, recipe)
    above_pool = [text for text, _qid in lines_above(messages, recipe)]
    # A recipe may type in some modes only: English sentences typed wholly in the other layout
    # teach the first word asked again once the next one is converted.
    modes = (EDIT_MODE,) if recipe.get("edit") else tuple(cast(list[str], recipe.get("modes", list(TYPING_MODES))))
    if not set(modes) <= {EDIT_MODE, *TYPING_MODES}:
        raise ValueError(f"unknown typing modes: {modes}")
    jobs: list[Job] = []
    for index, text in enumerate(everyone):
        above = previous_line(recipe, text, index, len(everyone), len(code), len(above_pool))
        prefix = "" if above is None else ({"code": code, "above": above_pool}.get(above[0], everyone)[above[1]]) + "\n"
        jobs += [(index, text, mode, _draw(f"{recipe['seed']}:choice:{text}"), prefix) for mode in modes]
    return jobs


def recipe_sources(messages: Path, recipe: Recipe, code_path: Path | None) -> tuple[set[int], set[int]]:
    """Whose text a recipe types: ids of its messages, and questions of the code lines above them."""

    chosen = recipe_messages(messages, recipe)
    code_count = len(_code_lines(code_path, recipe))
    above_pool = lines_above(messages, recipe)
    ids = {qid for _text, qid in chosen}
    lines: set[int] = set()
    for index, (text, _qid) in enumerate(chosen):
        above = previous_line(recipe, text, index, len(chosen), code_count, len(above_pool))
        if above is not None and above[0] == "code":
            lines.add(above[1])
        elif above is not None and above[0] == "above":
            # The pool is from the same source as the recipe's own messages (its ids join theirs).
            ids.add(above_pool[above[1]][1])
    questions: set[int] = set()
    if lines and code_path is not None:
        with gzip.open(code_path.with_name(code_path.name.replace(".txt.gz", ".qids.txt.gz")), "rt", encoding="utf-8") as handle:
            questions = {int(qid) for number, qid in enumerate(handle) if number in lines}
    return ids, questions


def capture(manifest: Path, messages: Path, model: Path, only: str | None, verify: bool, workers: int) -> dict[str, object]:
    payload = json.loads(manifest.read_text(encoding="utf-8"))
    results: dict[str, object] = {}
    for source in payload["sources"]:
        if only and source["file"] != only:
            continue
        recipe = cast(Recipe, source["recipe"])
        code = messages / str(recipe["code"]) if recipe.get("code") else None
        jobs = jobs_for(messages / str(recipe["messages"]), recipe, code)
        # A recipe may keep only some answers: correctly typed Russian text teaches `keep` alone.
        labels = set(cast(list[str], recipe["labels"])) if recipe.get("labels") else None
        seen: Counter[str] = Counter()
        with ProcessPoolExecutor(workers, initializer=_init, initargs=(str(model),)) as executor:
            for rows in executor.map(_capture, jobs, chunksize=MIXED_TYPING_CHUNK_JOBS):
                for row in rows:
                    if row[0] != row[1] and (labels is None or row[-1] in labels):
                        seen[json.dumps(row, ensure_ascii=False, separators=(",", ":"))] += 1
        data = lzma.compress("".join(f"[{count},{row[1:]}\n" for row, count in sorted(seen.items())).encode(),
                             preset=MIXED_CAPTURE_XZ_PRESET)
        digest = hashlib.sha256(lzma.decompress(data)).hexdigest()
        path = manifest.parent / str(source["file"])
        if verify:
            if hashlib.sha256(lzma.decompress(path.read_bytes())).hexdigest() != digest:
                raise ValueError(f"captured questions differ: {path.name}")
        else:
            path.write_bytes(data)
            source.update({"sha256": hashlib.sha256(data).hexdigest(), "rows": sum(seen.values()), "unique": len(seen)})
        results[str(source["file"])] = {"rows": sum(seen.values()), "unique": len(seen), "content_sha256": digest}
    if not verify:
        # The trainer checks every file against the SHA-256 recorded here.
        manifest.write_text(json.dumps(payload, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
    return results


def build_sources(manifest: Path, messages: Path, tatoeba: Path) -> dict[str, int]:
    """Whose text the captured files hold, written next to the manifest for attribution."""

    payload = json.loads(manifest.read_text(encoding="utf-8"))
    questions: set[int] = set()
    sentences: set[int] = set()
    treebank: set[str] = set()
    for source in payload["sources"]:
        recipe = cast(Recipe, source["recipe"])
        code = messages / str(recipe["code"]) if recipe.get("code") else None
        ids, code_questions = recipe_sources(messages / str(recipe["messages"]), recipe, code)
        questions |= code_questions
        if str(recipe["messages"]).startswith("taiga"):
            # Taiga sentences are named by their UD sentence id.
            names = {int(row["qid"]): str(row["sid"]) for row in map(json.loads, gzip.open(messages / str(recipe["messages"]), "rt", encoding="utf-8"))}
            treebank |= {names[number] for number in ids}
        else:
            (sentences if str(recipe["messages"]).startswith("tatoeba") else questions).update(ids)
    authors: dict[int, str] = {}
    for language in ("rus", "eng"):
        for line in bz2.open(tatoeba / f"{language}_sentences_detailed.tsv.bz2", "rt", encoding="utf-8"):
            fields = line.rstrip("\n").split("\t")
            if len(fields) > TATOEBA_AUTHOR_COLUMN and int(fields[0]) in sentences:
                authors[int(fields[0])] = fields[TATOEBA_AUTHOR_COLUMN]
    if set(authors) != sentences:
        raise ValueError("a typed sentence is missing from the detailed export")
    (manifest.parent / "so-questions.txt").write_text(
        "# ru.stackoverflow.com questions whose messages or code lines the captured files hold;\n"
        "# https://ru.stackoverflow.com/questions/<id> names the authors of the question, its answers and comments.\n"
        + "".join(f"{qid}\n" for qid in sorted(questions)), encoding="utf-8")
    (manifest.parent / "tatoeba-authors.tsv.xz").write_bytes(lzma.compress((
        "# Tatoeba sentences the captured files hold and their authors (\\N: no owner);\n"
        "# https://tatoeba.org/sentences/show/<id>\n"
        + "".join(f"{sentence}\t{authors[sentence]}\n" for sentence in sorted(sentences))).encode(), preset=MIXED_CAPTURE_XZ_PRESET))
    if treebank:
        (manifest.parent / "taiga-sentences.txt").write_text(
            "# UD Russian-Taiga r2.18 sentences the captured files hold, by `sent_id`;\n"
            "# https://github.com/UniversalDependencies/UD_Russian-Taiga names their sources.\n"
            + "".join(f"{name}\n" for name in sorted(treebank)), encoding="utf-8")
    return {"questions": len(questions), "sentences": len(sentences), "authors": len(set(authors.values())), "taiga": len(treebank)}


def evaluate(args: argparse.Namespace) -> dict[str, object]:
    recipe: Recipe = {"seed": args.seed, "kinds": args.kinds.split(","), "count": args.count, "russian_share": args.russian_share,
                      "typo_rate": args.typo_rate, "prefix_rate": args.prefix_rate, "edit": args.edit}
    jobs = jobs_for(args.messages, recipe, args.code)
    model = args.model or ROOT / "src/keyswitch/resources/models/context_policy_v1.json"
    with ProcessPoolExecutor(args.workers, initializer=_init, initargs=(str(model),)) as executor:
        rows = list(executor.map(_evaluate, jobs, chunksize=MIXED_TYPING_CHUNK_JOBS))
    summary: dict[str, dict[str, int]] = {}
    for row in rows:
        if row.get("skipped"):
            continue
        entry = summary.setdefault(str(row["mode"]), {"n": 0, "ok": 0})
        entry["n"] += 1
        entry["ok"] += int(bool(row["ok"]))
    args.output.write_text(json.dumps({"summary": summary, "rows": rows}, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
    return {"summary": summary}


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    build = commands.add_parser("messages")
    build.add_argument("--so", type=Path)
    build.add_argument("--tatoeba", type=Path)
    build.add_argument("--taiga", type=Path)
    build.add_argument("--out", type=Path, required=True)
    terms = commands.add_parser("terms")
    terms.add_argument("--messages", type=Path, required=True)
    terms.add_argument("--out", type=Path, required=True)
    grab = commands.add_parser("capture")
    grab.add_argument("--manifest", type=Path, required=True)
    grab.add_argument("--messages", type=Path, required=True)
    grab.add_argument("--model", type=Path, required=True)
    grab.add_argument("--only")
    grab.add_argument("--verify", action="store_true")
    grab.add_argument("--workers", type=int, default=MIXED_TYPING_DEFAULT_WORKERS)
    sources = commands.add_parser("sources")
    sources.add_argument("--manifest", type=Path, required=True)
    sources.add_argument("--messages", type=Path, required=True)
    sources.add_argument("--tatoeba", type=Path, required=True)
    judge = commands.add_parser("evaluate")
    judge.add_argument("--messages", type=Path, required=True)
    judge.add_argument("--count", type=int, required=True)
    judge.add_argument("--output", type=Path, required=True)
    judge.add_argument("--model", type=Path)
    judge.add_argument("--kinds", default="comment")
    judge.add_argument("--seed", default="mixbench-v1")
    judge.add_argument("--russian-share", type=float, default=MIXED_EVALUATE_RUSSIAN_SHARE)
    judge.add_argument("--typo-rate", type=float, default=0.0)
    judge.add_argument("--prefix-rate", type=float, default=0.0)
    judge.add_argument("--code", type=Path)
    judge.add_argument("--edit", action="store_true")
    judge.add_argument("--workers", type=int, default=MIXED_TYPING_DEFAULT_WORKERS)
    args = parser.parse_args(argv)
    if args.command == "messages":
        built: dict[str, int] = {}
        if args.so is not None and args.tatoeba is not None:
            built.update(build_messages(args.so, args.tatoeba, args.out))
        if args.tatoeba is not None:
            args.out.mkdir(parents=True, exist_ok=True)
            built.update(build_tatoeba_bare(args.tatoeba, args.out))
        if args.taiga is not None:
            args.out.mkdir(parents=True, exist_ok=True)
            built.update(build_taiga(args.taiga, args.out))
        result: object = built
    elif args.command == "terms":
        from keyswitch.language_model import LanguageModel
        from keyswitch.lexicon_supplement import supplement_words
        russian = LanguageModel.load("ru_RU", supplement_words("ru_RU"))
        table = build_terms(args.messages, lambda word: russian.score(word).known)
        args.out.write_text(json.dumps(table, ensure_ascii=False), encoding="utf-8")
        result = {name: len(words) for name, words in table.items()}
    elif args.command == "capture":
        result = capture(args.manifest, args.messages, args.model, args.only, args.verify, args.workers)
    elif args.command == "sources":
        result = build_sources(args.manifest, args.messages, args.tatoeba)
    else:
        result = evaluate(args)
    print(json.dumps(result, ensure_ascii=False, indent=1))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
