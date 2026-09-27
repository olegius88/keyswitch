"""Model-blind action interventions on real phrases; not observed user intent."""
from __future__ import annotations

import collections
import json
import re
import unicodedata
from dataclasses import dataclass
from pathlib import Path
from typing import Literal

from keyswitch.constants.corpus import (
    CONTEXT_FRAME_AFTER_MAX_CHARACTERS,
    CONTEXT_FRAME_BEFORE_MAX_CHARACTERS,
    CONTEXT_FRAME_FOCUS_POSITIONS_PER_PHRASE,
    CONTEXT_FRAME_LEXICAL_HOLDOUT_MODULUS,
    CONTEXT_FRAME_SAFETY_CALIBRATION_BUCKET,
    CONTEXT_FRAME_SAFETY_DEVELOPMENT_BUCKET,
    CONTEXT_FRAME_SAFETY_SPLIT_BUCKET_COUNT,
    CONTEXT_FRAME_SAFETY_TRAIN_BUCKETS,
    CONTEXT_FRAME_SHORT_WORD_MAX_CHARACTERS,
    CONTEXT_FRAME_SPELLING_EDGE_CHARACTERS,
    CONTEXT_FRAME_SPELLING_VARIANT_MIN_CHARACTERS,
    CONTEXT_FRAME_SPELLING_VARIANT_MODULUS,
    CONTEXT_FRAME_TEXT_FIELD_MODULUS,
    CONTEXT_FRAME_WORD_MAX_CHARACTERS,
    CONTEXT_FRAME_WRONG_LAYOUT_BEFORE_MODULUS,
)
from keyswitch.constants.file_formats import HEXADECIMAL_BASE, REPORT_JSON_INDENT
from keyswitch.constants.training import DETERMINISTIC_CHOICE_HEX_DIGITS
from keyswitch.layouts import LayoutPair

from context_corpus import AssignedPhrase, assign, digest, load_source


Action = Literal["keep", "convert", "wait", "suggest"]
TOKEN = re.compile(r"[A-Za-zА-Яа-яЁё]+(?:['’\-][A-Za-zА-Яа-яЁё]+)*")
# The trigger and the application of a frame are drawn from these, in this order.
TRIGGERS = ("space", "space", "space", "pause", "punctuation", "enter")
APPLICATIONS = ("", "Telegram", "chrome", "Code")


def draw(key: str) -> int:
    """The deterministic number a frame's choices are taken from."""

    return int(digest(key)[:DETERMINISTIC_CHOICE_HEX_DIGITS], HEXADECIMAL_BASE)


def lexical_holdout(family: str) -> bool:
    """The focus family belongs to the separate lexical test track."""

    return draw("focus-holdout:" + family) % CONTEXT_FRAME_LEXICAL_HOLDOUT_MODULUS == 0


def attached(character: str) -> bool:
    return bool(character) and (character.isalnum() or character in "_'’-‑" or unicodedata.category(character).startswith("M"))


def word_spans(text: str, group: int) -> list[re.Match[str]]:
    spans: list[re.Match[str]] = []
    for match in TOKEN.finditer(text):
        before = text[match.start() - 1] if match.start() else ""
        after = text[match.end()] if match.end() < len(text) else ""
        if attached(before) or attached(after) or not 1 <= len(match.group()) <= CONTEXT_FRAME_WORD_MAX_CHARACTERS:
            continue
        if not all(("a" <= c.casefold() <= "z") if group == 0 else ("а" <= c.casefold() <= "я" or c.casefold() == "ё") for c in match.group() if c.isalpha()):
            continue
        # Do not label a suffix of an identifier/address as a natural word.
        left = re.search(r"\S*$", text[:match.start()])
        right = re.match(r"\S*", text[match.end():])
        envelope = (left.group() if left else "") + match.group() + (right.group() if right else "")
        if any(mark in envelope for mark in ("@", "_", "/", "\\", "`")) or any(c.isdigit() for c in envelope):
            continue
        spans.append(match)
    return spans


@dataclass(frozen=True)
class Frame:
    identifier: str
    cluster: str
    split: str
    locale: str
    original: str
    alternative: str
    group: int
    before: str
    after: str
    application: str
    role: str
    trigger: str
    action: Action
    focus_family: str
    category: str


def build(phrases: list[AssignedPhrase]) -> list[Frame]:
    pair = LayoutPair()
    frames: list[Frame] = []
    for item in phrases:
        # This reservoir is never made into training/evaluation rows for the
        # first candidate. It is reserved before any model is trained.
        if item.split == "reserve":
            continue
        phrase = item.phrase
        target = 0 if phrase.locale == "eng" else 1
        matches = word_spans(phrase.text, target)
        if not matches:
            continue
        ordered = sorted(matches, key=lambda match: digest(f"focus:{phrase.identifier}:{match.start()}"))
        first = matches[0]
        selected = ([first] if len(first.group()) <= CONTEXT_FRAME_SHORT_WORD_MAX_CHARACTERS else []) + ordered
        positions: set[int] = set()
        for match in selected:
            if match.start() in positions:
                continue
            positions.add(match.start())
            if len(positions) > CONTEXT_FRAME_FOCUS_POSITIONS_PER_PHRASE:
                break
            token = match.group()
            wrong = pair.translate(token, "us" if target == 0 else "ru", "ru" if target == 0 else "us")
            if wrong == token or pair.translate(wrong, "ru" if target == 0 else "us", "us" if target == 0 else "ru") != token:
                continue
            key = f"tatoeba:{phrase.identifier}:{match.start()}"
            sample = draw(key)
            before = phrase.text[:match.start()][-CONTEXT_FRAME_BEFORE_MAX_CHARACTERS:]
            if sample % CONTEXT_FRAME_WRONG_LAYOUT_BEFORE_MODULUS == 0:
                before = pair.translate(before, "us" if target == 0 else "ru", "ru" if target == 0 else "us")
            text_field = sample % CONTEXT_FRAME_TEXT_FIELD_MODULUS == 0
            after = phrase.text[match.end():][:CONTEXT_FRAME_AFTER_MAX_CHARACTERS] if text_field else ""
            trigger = TRIGGERS[sample % len(TRIGGERS)]
            app = APPLICATIONS[sample % len(APPLICATIONS)]
            role = "text" if text_field else "unknown"
            family = min(token.casefold(), wrong.casefold())
            # A separate focus-lexical track excludes these supervision
            # families from ALL fitting/tuning sets. The primary test checks
            # unseen phrase groups and may share common focus words.
            lexical = lexical_holdout(family)
            if lexical and item.split != "test":
                continue
            split = "lexical_test" if lexical else item.split
            for original, alternative, group, action in ((token, wrong, target, "keep"), (wrong, token, 1 - target, "convert")):
                desired: Action = "keep" if action == "keep" else "convert"
                if desired == "convert" and len(token) <= CONTEXT_FRAME_SHORT_WORD_MAX_CHARACTERS and not before.strip() and not after.strip():
                    desired = "suggest" if trigger in {"enter", "punctuation"} else "wait"
                frames.append(Frame(key + ":" + action, item.group, split, phrase.locale, original, alternative, group, before, after, app, role, trigger, desired, family, "real_phrase_layout_intervention"))
            # A spelling error is not automatically a layout error. The
            # intended language here is known from our intervention, not
            # inferred from dictionary membership or a teacher prediction.
            if (split != "lexical_test" and sample % CONTEXT_FRAME_SPELLING_VARIANT_MODULUS == 0
                    and len(token) >= CONTEXT_FRAME_SPELLING_VARIANT_MIN_CHARACTERS and token.isalpha()):
                position = 1 + sample % (len(token) - CONTEXT_FRAME_SPELLING_EDGE_CHARACTERS)
                typo = token[:position] + token[position + 1:]
                alternate = pair.translate(typo, "us" if target == 0 else "ru", "ru" if target == 0 else "us")
                typo_family = min(typo.casefold(), alternate.casefold())
                if not lexical_holdout(typo_family):
                    frames.append(Frame(key + ":spelling", item.group, split, phrase.locale, typo, alternate, target, before, after, app, role, trigger, "keep", typo_family, "synthetic_spelling_keep"))
    return frames


def technical_frames(path: Path) -> list[Frame]:
    """Small authored safety curriculum; never count expansions as sources."""
    payload: object = json.loads(path.read_bytes())
    if not isinstance(payload, dict) or payload.get("schema_version") != 1:
        raise ValueError("invalid safety curriculum")
    tokens: object = payload.get("technical")
    if not isinstance(tokens, list) or any(not isinstance(token, str) for token in tokens):
        raise ValueError("invalid technical tokens")
    pair = LayoutPair()
    rows: list[Frame] = []
    for value in tokens:
        assert isinstance(value, str)
        alternate = pair.translate(value, "us", "ru")
        family = min(value.casefold(), alternate.casefold())
        bucket = draw("keyswitch:context-v2:safety:" + family) % CONTEXT_FRAME_SAFETY_SPLIT_BUCKET_COUNT
        split = ("train" if bucket < CONTEXT_FRAME_SAFETY_TRAIN_BUCKETS
                 else "development" if bucket == CONTEXT_FRAME_SAFETY_DEVELOPMENT_BUCKET
                 else "calibration" if bucket == CONTEXT_FRAME_SAFETY_CALIBRATION_BUCKET else "test")
        lexical = lexical_holdout(family)
        if lexical and split != "test":
            continue
        split = "lexical_test" if lexical else split
        for offset, before in enumerate(("запусти ", "в сообщении написано ", "const value = ", "return ", "$ ", "the command is ")):
            for app in ("Code", "WindowsTerminal", "Telegram", ""):
                for trigger in ("space", "pause", "enter", "punctuation"):
                    identifier = f"authored:{value}:{offset}:{app}:{trigger}"
                    rows.append(Frame(identifier, "authored:" + family, split, "eng", value, alternate, 0, before, "", app, "unknown", trigger, "keep", family, "authored_technical_keep"))
    return rows


if __name__ == "__main__":
    source = load_source()
    phrases, _ = assign(source)
    rows = build(phrases)
    report: dict[str, object] = {
        "rows": len(rows),
        "splits": dict(sorted(collections.Counter(row.split for row in rows).items())),
        "actions": dict(sorted(collections.Counter(row.action for row in rows).items())),
        "families": {split: len({row.focus_family for row in rows if row.split == split}) for split in sorted({row.split for row in rows})},
        "clusters": {split: len({row.cluster for row in rows if row.split == split}) for split in sorted({row.split for row in rows})},
        "model_loaded": False,
        "metrics_evaluated": False,
    }
    print(json.dumps(report, indent=REPORT_JSON_INDENT))
