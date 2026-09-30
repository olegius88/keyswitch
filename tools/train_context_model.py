#!/usr/bin/env python3
"""Train and verify the independent contextual action policy.

The current Layout Intent artifact and its sealed corpus are never modified
or used as training rows. Scenario groups split by physical key sequence
before variants, applications or contexts are expanded. The engine's own
questions on typed public text (model/context_v1/captured) join the training
rows, except those about the families development and the holdout measure.
Test labels are used only after epoch selection on development. Epochs run on
the kernel of context_optimizer.c, or on a Python loop with the same arithmetic
where no C compiler exists. Re-running --verify must reproduce the exact
artifact. Reports name their evidence: scenarios written for the project and
questions on typed public text, not real users.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import lzma
import math
import sys
import time
from array import array
from collections import Counter
from collections.abc import Callable, Iterator, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import NamedTuple, cast

from keyswitch.constants.file_formats import HEXADECIMAL_BASE, REPORT_JSON_INDENT, VERSION_HASH_CHARACTERS
from keyswitch.constants.models import CONTEXT_LINE_FEATURE_VERSION, CONTEXT_V1_CONVERSION_THRESHOLD
from keyswitch.constants.training import (
    # Keeps its name here: tests shorten the training by patching this module's EPOCHS.
    CONTEXT_V1_EPOCHS as EPOCHS,
    CONTEXT_V1_INSIDE_WORD_MIN_CHARACTERS,
    CONTEXT_V1_KEEP_IMPORTANCE,
    CONTEXT_V1_LEARNING_RATE,
    CONTEXT_V1_MAX_REPORTED_FAILURES,
    CONTEXT_V1_MAX_TRAINED_FEATURES,
    CONTEXT_V1_SLASH_WORDS_PER_FAMILY,
    CONTEXT_V1_SPLIT_BUCKET_COUNT,
    CONTEXT_V1_TRAIN_SPLIT_BUCKETS,
    DETERMINISTIC_CHOICE_HEX_DIGITS,
    DETERMINISTIC_ROUNDING_DECIMALS,
    LOG_LOSS_PROBABILITY_FLOOR,
    SHORT_WORD_MAX_CHARACTERS,
)
from keyswitch.context_model import (
    ACTIONS, ARTIFACT_PATH, TECHNICAL_MARKS, TERM_FREQUENCY_PATH, ContextAction, ContextEvidence,
    ContextModel, extract_context_features, one_typo_from_word,
)
from keyswitch.detector import LanguageDetector
from keyswitch.input_context import FieldContext
from keyswitch.intent_model import LinearNgramModel
from keyswitch.language_model import LanguageModel
from keyswitch.layouts import LayoutPair
from keyswitch.lexicon_supplement import supplement_words
from keyswitch.short_words import (
    TRUSTED_SHORT_WORDS, natural_short_source_veto, opens_sentences, trusted_short_word_decision,
)
from context_optimizer import Kernel, Packed, kernel_softmax, python_epoch


ROOT = Path(__file__).resolve().parents[1]
SCENARIOS = ROOT / "model/context_v1/scenarios.json"
# Signs a captured token may carry at its ends that the scenario families never do.
CAPTURED_EDGE_SIGNS = ".,!?:;\"'()[]{}<>«»-"
HOLDOUT = ROOT / "model/context_v1/holdout-3.json"
# Questions the engine asked while public text was typed the way a person switches layouts
# (tools/mixed_typing.py capture): one file per source and typing, weights in the manifest.
CAPTURED = ROOT / "model/context_v1/captured/manifest.json"
CAPTURED_COLUMNS = (
    "count", "original", "alternative", "source_group", "trigger", "baseline_convert", "source_known", "target_known",
    "score_delta", "literal_tail", "boundary_text", "after_origin", "source_typo", "target_typo",
    "source_opening", "target_opening", "inside", "before", "after", "role", "label",
)
REPORT = ROOT / "model/context_v1/report.json"
NAMESPACE = "keyswitch:context-v1:candidate3"
# Rows carry no application name: the model must answer the same way in every
# application, and a name it learned covered only the few applications listed
# here. Rows used to be expanded over (application, role) pairs; each role keeps
# the share it had then, so the balance of the corpus stays as it was measured.
# Without field reading the engine reports role `unknown` for every real
# application, and a field read through accessibility reports its own role.
FIELD_ROLES = ("text", "text", "unknown", "unknown", "unknown", "unknown")
TERMINAL_ROLES = ("code", "terminal", "unknown", "unknown")
# A chat input or an editor pane holding code, logs or English prose is where a
# correctly typed Russian word most often follows English text.
LATIN_FIELD_ROLES = ("text", "text")
SLASH_ROLES = ("text", "code", "unknown", "unknown")
# English words that may follow a collision word: whole phrases, as a field read around the caret
# gives them, and single words, as the engine gives the converted neighbour of a waiting word.
COLLISION_ENGLISH_FOLLOWING = ("is not ready yet", "can wait until tomorrow", "was right about it",
                               "is", "can", "was", "the", "and")


@dataclass(frozen=True)
class Row:
    evidence: ContextEvidence
    action: ContextAction
    family: str
    split: str
    category: str


def canonical(value: object) -> bytes:
    return (json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":")) + "\n").encode()


def family_split(signature: str) -> str:
    digest = hashlib.sha256((NAMESPACE + ":" + signature).encode()).hexdigest()
    bucket = int(digest[:DETERMINISTIC_CHOICE_HEX_DIGITS], HEXADECIMAL_BASE) % CONTEXT_V1_SPLIT_BUCKET_COUNT
    return "train" if bucket < CONTEXT_V1_TRAIN_SPLIT_BUCKETS else "development"


def build_corpus(source_path: Path = SCENARIOS, *, held_out: bool = False) -> list[Row]:
    payload: object = json.loads(source_path.read_bytes())
    if not isinstance(payload, dict) or payload.get("schema_version") != 1:
        raise ValueError("invalid scenarios")
    pair = LayoutPair()
    intent, _status = LinearNgramModel.try_load_default()
    if intent is None:
        raise ValueError("baseline model unavailable")
    # The engine's detector reads both lexicons with their packaged supplements
    # (engine.py: LanguageModel.load(locale, supplement_words(locale))); the
    # baseline decision and the known flags here must be the ones it computes.
    models = {group: LanguageModel.load(locale, supplement_words(locale))
              for group, locale in ((0, "en_US"), (1, "ru_RU"))}
    detector = LanguageDetector(models, intent)
    rows: list[Row] = []
    cache: dict[tuple[str, int, str, int | None], tuple[bool, bool, bool, float]] = {}
    typos: dict[tuple[str, int], tuple[bool, bool]] = {}
    # The short Russian words are all taught, none held for development: `ты` was one
    # of the three held there, so the model had never seen it and read `ns` by its
    # letters alone - `ns` stayed in an empty field and after `привет` (28.09.2026).
    # The short English words keep their split: taught, `in` carried the correct
    # `штуке` over to `inert` through the letters they share.
    taught: set[str] = set()
    listed: object = payload.get("short_russian", [])
    for word in listed if isinstance(listed, list) else []:
        if isinstance(word, str):
            wrong = pair.translate(word, "ru", "us")
            taught.add(min(word.casefold(), wrong.casefold()))
    # Words that never open a Russian sentence (UD Taiga: `же` and `бы` none, `ли` once)
    # still wait for their neighbour at the start of a field; every other short word
    # typed in the other layout converts there on its own.
    not_opening: object = payload.get("short_russian_not_opening", [])
    if not isinstance(not_opening, list) or any(not isinstance(word, str) for word in not_opening):
        raise ValueError("invalid short_russian_not_opening scenarios")
    waits_alone = {word.casefold() for word in cast(list[str], not_opening)}

    def context_group_of(text: str) -> int | None:
        """The engine remembers the layout of the previous word, not its text.

        So the group comes from the last word in front of the caret. A field
        full of code with a Russian phrase at its end hands the engine a
        Russian previous word, whatever script dominates the field.
        """

        for char in reversed(text.casefold()):
            if "а" <= char <= "я" or char == "ё":
                return 1
            if "a" <= char <= "z":
                return 0
        return None

    def baseline_decision(word: str, group: int, alternate: str, trigger: str,
                          context_group: int | None) -> bool:
        """Reproduce exactly what `engine._decide_word` hands to the policy.

        The engine applies the short-word veto and the curated trusted
        short-word override before the model runs, so a corpus built from the
        bare detector teaches the model to reject decisions it never sees.
        """

        from keyswitch.intent_model import CorrectionTrigger

        decision = detector.decide(word, {1 - group: alternate}, group, trigger=cast(CorrectionTrigger, trigger))
        decision = natural_short_source_veto(decision, context_group=context_group)
        if not decision.should_convert:
            override = trusted_short_word_decision(
                detector, word, {1 - group: alternate}, group,
                ignored_words=(), rejected_targets=frozenset(), protect_code=True,
                context_group=context_group,
            )
            if override is not None:
                decision = override
        return decision.should_convert

    def baseline_for(word: str, group: int, before: str, trigger: str) -> bool:
        alternate = pair.translate(word, "us" if group == 0 else "ru", "ru" if group == 0 else "us")
        key = (word, group, trigger, context_group_of(before))
        if key not in cache:
            cache[key] = (
                baseline_decision(word, group, alternate, trigger, context_group_of(before)),
                models[group].score(word).known, models[1 - group].score(alternate).known,
                models[1 - group].score(alternate).value - models[group].score(word).value,
            )
        return cache[key][0]

    def add(word: str, group: int, before: str, after: str, role: str,
            trigger: str, action: ContextAction, category: str, baseline_override: bool | None = None,
            inside: bool = False, planned: bool = False) -> None:
        from keyswitch.input_context import FieldRole

        alternate = pair.translate(word, "us" if group == 0 else "ru", "ru" if group == 0 else "us")
        signature = min(word.casefold(), alternate.casefold())
        baseline_for(word, group, before, trigger)
        baseline, source_known, target_known, delta = cache[(word, group, trigger, context_group_of(before))]
        if baseline_override is not None:
            baseline = baseline_override
        # Feature schema 5 tells a reading one typo away from a word from any other
        # unknown token. Without it a lone `aghbdtn` (a stray key before `ghbdtn`)
        # and a technical name the detector converts by mistake (`nextjs`) differed
        # only in their letters, and every row keeping the name cost the other its
        # conversion. The engine computes the same flags with its own lexicons.
        if (word, group) not in typos:
            typos[(word, group)] = (one_typo_from_word(word, models[group]), one_typo_from_word(alternate, models[1 - group]))
        source_typo, target_typo = typos[(word, group)]
        item = ContextEvidence(
            word, alternate, group, FieldContext("", "training-field", before, after, cast(FieldRole, role)),
            trigger, baseline, source_known, target_known, delta,
            source_typo=source_typo, target_typo=target_typo,
            source_opening=opens_sentences(word, group), target_opening=opens_sentences(alternate, 1 - group),
            inside=inside,
            # The engine asks with the next word it plans to convert after the word
            # (a waiting word, a converted word asked again); feature schema 6 tells
            # that text from text already in the field.
            after_origin="planned_next_conversion" if planned else "none",
        )
        split = "test" if held_out else "train" if signature in taught else family_split(signature)
        rows.append(Row(item, action, signature, split, category))

    # A word typed in the other layout converts with a stray key in it too, as 0.33.0
    # promised for `aghbdtn` (a stuck `a` before `ghbdtn`): the reading is one letter
    # away from the word, which the typo evidence of schema 5 tells. The stray keys
    # are ones whose Russian letter is no word on its own (`й`, `ц`, `з`), at either
    # end of the word; `a` itself is left to the authored test. And a Russian word
    # typed as intended stays with text after the caret, whatever that text is.
    russian_words_list: object = payload.get("russian", [])
    if not isinstance(russian_words_list, list) or any(not isinstance(word, str) for word in russian_words_list):
        raise ValueError("invalid russian scenarios")
    for word in cast(list[str], russian_words_list):
        wrong = pair.translate(word, "ru", "us")
        for stray in ("q", "w", "p"):
            for typed in (stray + wrong, wrong + stray):
                for before in ("", "я думаю что "):
                    for role in FIELD_ROLES:
                        for trigger in ("space", "enter"):
                            add(typed, 0, before, "", role, trigger, "convert", "stray_key_wrong")
        # Both forms of the word get the same text after the caret, so that text
        # stays neutral and the word decides, as in the English fields above: taught
        # only as keep, it made a Russian word in the English layout stay when a
        # sentence followed, and letters typed into the middle of a word were no
        # longer corrected (`суrа` for `сука`, 76 package sentences, 28.09.2026).
        for after in ("этого достаточно", "сегодня всё получилось", "is not ready yet", "was right about it"):
            for before in ("", "я думаю что "):
                for role in FIELD_ROLES:
                    add(word, 1, before, after, role, "space", "keep", "russian_correct_after")
                    add(wrong, 0, before, after, role, "space", "convert", "russian_wrong_after")

    # A word converted at a space is asked about again when the next word goes back
    # the other way (engine._revert_with_next_word, the `short_revisit` rows below).
    # That says nothing about a word that reads clearly: `cgfcb,j` stays `спасибо`
    # before `team` typed in the Russian layout, and `руддщ` stays `hello` before a
    # Russian word typed in the English layout. Taught only with the short words both
    # readings share, the question took back any word: `спасибо team` became
    # `cgfcb,j team` (28.09.2026). A form that is itself a word of the other language
    # is left out: whether it was meant is what the next word may tell.
    english_next, russian_next = ("is", "can", "was", "the", "and"), ("привет", "сегодня", "давай", "спасибо", "хорошо")
    for group, family, other_next, own_next in ((1, russian_words_list, english_next, russian_next),
                                                (0, payload.get("english", []), russian_next, english_next)):
        for word in cast(list[str], family):
            wrong = pair.translate(word, "ru" if group == 1 else "us", "us" if group == 1 else "ru")
            if models[1 - group].score(wrong).known:
                continue
            for following, own in zip(other_next, own_next):
                for role in FIELD_ROLES:
                    add(wrong, 1 - group, "", following, role, "space", "convert", "revisit_stays",
                        baseline_override=False, planned=True)
                    # The other half, so the planned next word stays neutral and the word
                    # decides: typed as intended before a next word of its own language,
                    # it stays. Taught only as the converting half, it tipped a correct `if`
                    # after English text over the threshold (development, 28.09.2026).
                    add(word, group, "", own, role, "space", "keep", "revisit_stays_correct",
                        baseline_override=False, planned=True)

    # `russian_hard` holds frequent Russian words whose keys in the English layout
    # the detector does not convert on its own: only the context restores them
    # (`kexit` for `лучше`), so they are taught like the other Russian words.
    # An independent test set may leave the list out.
    for name, group in (("russian", 1), ("english", 0), ("short_russian", 1), ("russian_chat", 1), ("short_english", 0),
                        ("russian_hard", 1)):
        words: object = payload.get(name, [] if name == "russian_hard" else None)
        if not isinstance(words, list) or any(not isinstance(word, str) for word in words):
            raise ValueError("invalid scenario words")
        for word in cast(list[str], words):
            wrong = pair.translate(word, "ru" if group == 1 else "us", "us" if group == 1 else "ru")
            contexts: tuple[str, ...] = ("", "я думаю что ", "подскажи пожалуйста ", "мы обсуждали это вчера ") if group == 1 else ("", "I think that ", "could you please ", "we discussed this yesterday ")
            if name == "short_russian":
                # One- and two-letter words carry the ambiguity the whole policy
                # rests on, so they keep their share of the corpus as it grows.
                contexts += ("но ", "мне кажется ", "давай ", "сегодня ")
                # Eight phrases taught the model those phrases: `ns` converted after
                # `я думаю что` but not after `привет` (28.09.2026). Sixteen more make
                # it read the Russian text before the word rather than its words.
                contexts += ("слушай ", "кстати ", "вот ", "короче ", "хорошо ", "спасибо ", "понятно ",
                             "ладно ", "смотри ", "знаешь ", "а ", "ну ", "да ", "вообще ", "наверное ", "конечно ")
            # A word that stands alone may be meant either way when both readings
            # are words; short words nearly always are, so they count as such.
            collision = name in {"short_russian", "short_english"} or models[1 - group].score(wrong).known
            # A short English word on its own is as ambiguous as any short token:
            # typed in the other layout it waits for the next word (`ш` for `I`
            # at the start of a message), and typed as intended it stays.
            for before in contexts:
                for role in FIELD_ROLES:
                    for trigger in ("space", "pause", "enter", "punctuation"):
                        action: ContextAction = "convert"
                        # A short word without context stays ambiguous only when
                        # the curated trusted list did not already decide it. A
                        # conversational word whose other reading is an English
                        # word (`еще` and `tot`) is just as ambiguous on its own.
                        # A short word opening the field converts at a space or a
                        # pause: 10.1% of Russian sentences open with a curated
                        # two-letter word and their Latin readings open almost no
                        # English one (UD Taiga, UD EWT), so `ns` alone is `ты`. If
                        # the next word goes back the other way, the engine asks
                        # about this one again (the `short_revisit` rows below).
                        # Only Russian words: an English one typed in the Russian layout keeps
                        # waiting, since teaching `шт` alone as `in` carried the correct
                        # `штуке` over to `inert` (development, 28.09.2026).
                        opens = name == "short_russian" and word.casefold() not in waits_alone
                        if name in {"short_russian", "russian_chat", "short_english"} and collision \
                                and not before and not baseline_for(wrong, 1 - group, "", trigger):
                            action = ("convert" if opens
                                      else "suggest" if trigger in {"enter", "punctuation"} else "wait")
                        add(wrong, 1 - group, before, "", role, trigger, action, name + "_wrong")
                        # Standalone Latin letters may be variables. Correct
                        # short Russian words, like all valid prose, stay put.
                        add(word, group, before, "", role, trigger, "keep", name + "_correct")
            if name in {"short_russian", "short_english"}:
                # A word typed as intended stays whatever follows the caret. Only
                # the rows where a converted neighbour follows a word typed in the
                # other layout had text after the caret, so the model learned that
                # text after it means convert: `ты` typed in the middle of a
                # sentence became `ns` (28.09.2026).
                # An English word is taught at the start of the field only: after an
                # English phrase these rows taught that a word typed in the English
                # layout after English text stays, and letters typed into a Russian
                # word under a line of code stayed Latin.
                for after in ("этого достаточно", "сегодня всё получилось", "is not ready yet", "was right about it"):
                    for before in ("", contexts[1]) if group == 1 else ("",):
                        for role in FIELD_ROLES:
                            add(word, group, before, after, role, "space", "keep", "short_correct_after")
            if name == "short_russian":
                # `ns` converted on its own, then an English word converted back: the
                # engine asks about `ns` again with that word after it, and the word
                # the user typed stays (`ns code`). The engine asks that question with
                # no baseline conversion (engine._revert_with_next_word): taught with
                # the detector's own verdict, the rows of the curated words it converts
                # taught `baseline:1` to keep, and `aghbdtn` stayed.
                for following in (word for word in COLLISION_ENGLISH_FOLLOWING if " " not in word):
                    for role in FIELD_ROLES:
                        add(wrong, 0, "", following, role, "space", "keep", "short_revisit", baseline_override=False, planned=True)
                for following in ("этого достаточно", "следующего сообщения", "сегодня всё получилось",
                                  "завтра продолжим", "меня всё устраивает", "нас это не касается",
                                  "тебя ждут в офисе", "него другое мнение", "вас получилось лучше",
                                  "них уже есть решение"):
                    for role in FIELD_ROLES:
                        add(wrong, 0, "", following, role, "space", "convert", "short_lookahead")
                        # The engine asks again with only the converted next word after it;
                        # the same word may also stand in the field after the caret.
                        add(wrong, 0, "", following.split()[0], role, "space", "convert", "short_lookahead", planned=True)
                        add(wrong, 0, "", following.split()[0], role, "space", "convert", "short_lookahead")
            elif name == "short_english":
                for following in ("is not ready yet", "can wait until tomorrow", "will be fine", "was right about it",
                                  "should know that", "have seen this before"):
                    for role in FIELD_ROLES:
                        add(wrong, 1, "", following, role, "space", "convert", "short_lookahead")
                        # The engine asks again with only the converted next word after it;
                        # the same word may also stand in the field after the caret.
                        add(wrong, 1, "", following.split()[0], role, "space", "convert", "short_lookahead", planned=True)
                        add(wrong, 1, "", following.split()[0], role, "space", "convert", "short_lookahead")
                # The mirror of the `short_revisit` rows: the same English words after
                # the word and the same question without a baseline conversion, so only
                # the layout the word was typed in tells them apart. Without it `ns is`
                # staying taught `иге` (`but` in the Russian layout) before an English
                # word to stay too (package, 28.09.2026).
                for following in (word for word in COLLISION_ENGLISH_FOLLOWING if " " not in word):
                    for role in FIELD_ROLES:
                        add(wrong, 1, "", following, role, "space", "convert", "short_english_revisit", baseline_override=False,
                            planned=True)
            elif name == "russian_chat" and collision:
                # A conversational word whose other reading is English waits at the
                # start of a message (`tot` for `еще`). The engine then asks about it
                # again with the next word after it, converted, as its only context.
                # Each has its twin with nothing after it, which waits: the rows differ
                # only in the next word, so only the next word is what they teach.
                for following in ("в", "не", "раз", "тест", "работает", "можно", "будет", "немного", "надо", "есть"):
                    for role in FIELD_ROLES:
                        add(wrong, 0, "", following, role, "space", "convert", "chat_lookahead", planned=True)
                        add(wrong, 0, "", following, role, "space", "convert", "chat_lookahead")
                        add(wrong, 0, "", "", role, "space", "wait", "chat_lookahead_alone")
            # A code field must not override the actual language of its comments.
            add(wrong, 1 - group, "// " + contexts[1], "", "code", "space", "convert", "code_comment")
            # A legitimate English insertion inside Russian prose is not a
            # layout error, even when the surrounding sentence is Russian.
            if group == 0:
                add(word, 0, "в сообщении написано ", "", "text", "space", "keep", "mixed_prose")

    # A field whose text is mostly code, logs or English prose still holds
    # Russian sentences: a chat input under an editor, a comment, a reply. The
    # model sees the field's dominant script, so without these rows it learned
    # that a Russian word after English text is a layout error - `почему ты`
    # became `почему ns` and `все еще` became `все tot` in real use. Every such
    # field teaches four classes in equal measure, so the script of the field
    # stays neutral and the token decides: a correctly typed Russian or English
    # word stays, and either typed in the other layout is converted. A longer
    # wrong form that is itself a word of the other language (`tot`) is not
    # taught here: whether it was meant needs more than the field can tell.
    # Short words have their own rows below.
    latin_fields: object = payload.get("latin_fields", [])
    latin_tails: object = payload.get("latin_tails", [])
    for scenarios in (latin_fields, latin_tails):
        if not isinstance(scenarios, list) or any(not isinstance(item, str) for item in scenarios):
            raise ValueError("invalid latin field scenarios")
    russian_words: list[str] = []
    for name in ("russian_chat", "short_russian", "russian"):
        russian_words += cast(list[str], payload.get(name, []))
    english_words = cast(list[str], payload.get("english", []))
    count = min(len(russian_words), len(english_words))
    for field in cast(list[str], latin_fields):
        for tail in cast(list[str], latin_tails):
            before = field + tail
            for role in LATIN_FIELD_ROLES:
                for word, group in [*((word, 1) for word in russian_words[:count]),
                                    *((word, 0) for word in english_words[:count])]:
                    add(word, group, before, "", role, "space", "keep", "latin_field_correct")
                    wrong = pair.translate(word, "ru" if group == 1 else "us", "us" if group == 1 else "ru")
                    if len(word) > SHORT_WORD_MAX_CHARACTERS and not models[1 - group].score(wrong).known:
                        add(wrong, 1 - group, before, "", role, "space", "convert", "latin_field_wrong")
                        # A word edited in place is decided at the pause after the
                        # click: letters typed into `шмидт` under a line of code in
                        # the English layout were left at 0.96-0.98 (package,
                        # 28.09.2026). Both forms, so the pause stays neutral.
                        if group == 1:
                            add(word, 1, before, "", role, "pause", "keep", "latin_field_correct")
                            add(wrong, 0, before, "", role, "pause", "convert", "latin_field_wrong")
                    # The same Russian word with the rest of its sentence after the
                    # caret, as a word edited in place under a line of code has it:
                    # both forms, so the text after it stays neutral.
                    if group == 1 and tail and len(word) > SHORT_WORD_MAX_CHARACTERS and not models[0].score(wrong).known:
                        add(word, 1, before, "сегодня всё получилось", role, "pause", "keep", "latin_field_correct_after")
                        add(wrong, 0, before, "сегодня всё получилось", role, "pause", "convert", "latin_field_wrong_after")

    # A Russian word whose keys in the other layout also spell an English word
    # (`лун` is `key`, `ищи` is `bob`): with the packaged supplement the engine
    # knows both readings of many such tokens. Typed as it is, the Russian word
    # stays after Russian text and after an English field, as every correctly
    # typed Russian word does. English words after it make it the English word:
    # the engine hands them over when a word is edited in place and when a
    # waiting word is decided with its neighbour (`here` with one key in the
    # other layout reads `руку`, and `is` follows). Russian words after it are
    # not taught: they would outweigh the rows where a Latin collision before a
    # converted Russian word is converted (`tot привет`).
    # The Latin reading after Russian text is not taught: an English word there
    # is often meant (`вчера key`).
    collisions: object = payload.get("russian_collisions", [])
    if not isinstance(collisions, list) or any(not isinstance(word, str) for word in collisions):
        raise ValueError("invalid russian_collisions scenarios")
    for word in cast(list[str], collisions):
        for before in ("я думаю что ", "подскажи пожалуйста ", "мы обсуждали это вчера "):
            for role in FIELD_ROLES:
                for trigger in ("space", "pause", "enter", "punctuation"):
                    add(word, 1, before, "", role, trigger, "keep", "collision_correct")
        for field in cast(list[str], latin_fields):
            for tail in cast(list[str], latin_tails):
                for role in LATIN_FIELD_ROLES:
                    add(word, 1, field + tail, "", role, "space", "keep", "collision_latin_field")
        for before in ("", "I think that ", "we discussed this yesterday "):
            for following in COLLISION_ENGLISH_FOLLOWING:
                for role in ("text", "unknown"):
                    add(word, 1, before, following, role, "space", "convert", "collision_english_after", planned=" " not in following)
                    if " " not in following:
                        add(word, 1, before, following, role, "space", "convert", "collision_english_after")

    # The runtime's curated trusted short-word list decides a handful of
    # two-letter tokens on its own, in both directions. Mirroring it here keeps
    # the model from vetoing a decision the engine has already made. The list
    # is code, not corpus data, so it belongs to training only; it can never
    # form an independent test family.
    if not held_out:
        for target_group, trusted in sorted(TRUSTED_SHORT_WORDS.items()):
            for word in sorted(trusted):
                wrong = pair.translate(word, "us" if target_group == 0 else "ru", "ru" if target_group == 0 else "us")
                phrases = ("", "я думаю что ", "мы обсуждали это вчера ") if target_group == 1 else ("", "I think that ", "we discussed this yesterday ")
                for before in phrases:
                    for role in FIELD_ROLES:
                        for trigger in ("space", "pause", "enter", "punctuation"):
                            converts = baseline_for(wrong, 1 - target_group, before, trigger)
                            opening = target_group == 1 and word.casefold() not in waits_alone
                            action = "convert" if converts or before or opening else (
                                "suggest" if trigger in {"enter", "punctuation"} else "wait")
                            add(wrong, 1 - target_group, before, "", role, trigger, cast(ContextAction, action), "trusted_short_wrong")
                            add(word, target_group, before, "", role, trigger, "keep", "trusted_short_correct")

    technical: object = payload.get("technical")
    if not isinstance(technical, list) or any(not isinstance(word, str) for word in technical):
        raise ValueError("invalid technical scenarios")
    technical_contexts = ("запусти ", "введи команду ", "const value = ", "return ", "$ ", "the command is ")
    for token in cast(list[str], technical):
        wrong = pair.translate(token, "us", "ru")
        letters = "".join(char for char in wrong if char.isalpha())
        # Some technical tokens are authored to stay although their letters spell
        # a Russian word in the other layout (`2ghbdtn`, `/ntcn1`): a word with a
        # digit or a path mark is an identifier, not a layout error.
        stays = len(token) > SHORT_WORD_MAX_CHARACTERS and models[1].score(letters).known
        # Such a token and any path, address or identifier stays at the start of
        # a field too (`/c,jhrb2` there once became `.сборки2`). A plain command
        # name alone is left to both of its readings: `зь2` typed alone is `pm2`.
        alone = ("",) if stays or any(char in token for char in TECHNICAL_MARKS) else ()
        for before in (*alone, *technical_contexts):
            for role in TERMINAL_ROLES:
                for trigger in ("space", "pause", "enter", "punctuation"):
                    add(token, 0, before, "", role, trigger, "keep", "technical")
        # The same keys typed in the Russian layout are still the command:
        # `пше` is `git`. Not taught for a one- or two-letter token (the
        # short-word families decide those), for a token that stays, or when the
        # other reading keeps a path mark (`гыук_шв`): the mark is what tells a
        # path from a word. A token with a digit is taught too since feature
        # schema 5 reads a digit together with the layout it was typed in: before
        # that, such rows taught the model that a digit points to a layout error
        # and `/c,jhrb2` at the start of a field became `/сборки2`, and `зь2`
        # rested on its letters alone until the short-word rows of 28.09.2026 left
        # it at 0.966.
        if (len(token) > SHORT_WORD_MAX_CHARACTERS and not stays
                and not any(char in wrong for char in TECHNICAL_MARKS)):
            for before in ("", *technical_contexts):
                for role in TERMINAL_ROLES:
                    for trigger in ("space", "pause", "enter", "punctuation"):
                        add(wrong, 1, before, "", role, trigger, "convert", "technical_wrong")

    def unknown_family(name: str, group: int, contexts: tuple[str, ...], roles: tuple[str, ...]) -> None:
        """Tokens outside both lexicons: the physical form decides, not a dictionary.

        A technical term, dotfile, jargon word or misspelling typed in the wrong
        layout is still a layout error; the same token typed as intended stays.
        Being absent from a dictionary is not itself a reason to hesitate.
        Only a one- or two-character token stays ambiguous without context,
        exactly as a short Russian word does.
        """

        tokens: object = payload.get(name, [])
        if not isinstance(tokens, list) or any(not isinstance(word, str) for word in tokens):
            raise ValueError(f"invalid {name} scenarios")
        for token in cast(list[str], tokens):
            wrong = pair.translate(token, "us" if group == 0 else "ru", "ru" if group == 0 else "us")
            for before in contexts:
                for role in roles:
                    for trigger in ("space", "pause", "enter", "punctuation"):
                        action: ContextAction = "convert"
                        # A short Russian word outside the lexicon (`хз` as `[p`) converts on
                        # its own like the listed ones do; its Latin reading is no word.
                        if len(token) <= SHORT_WORD_MAX_CHARACTERS and not before and name != "russian_unknown" \
                                and not baseline_for(wrong, 1 - group, "", trigger):
                            action = "suggest" if trigger in {"enter", "punctuation"} else "wait"
                        add(wrong, 1 - group, before, "", role, trigger, action, name + "_wrong")
                        add(token, group, before, "", role, trigger, "keep", name + "_correct")

    unknown_family("technical_terms", 0, ("", "запусти ", "далее запускается процесс ", "$ "), TERMINAL_ROLES)
    unknown_family("dotted", 0, ("", "открой файл ", "нужно поправить ", "$ cat "), TERMINAL_ROLES)
    unknown_family("english_unknown", 0, ("", "I think that ", "could you please ", "в сообщении написано "), FIELD_ROLES)
    unknown_family("russian_unknown", 1, ("", "я думаю что ", "нужно срочно ", "мы обсуждали это вчера ", "подскажи пожалуйста "), FIELD_ROLES)

    # Short Russian words outside the Russian lexicon whose other reading the
    # English lexicon happens to know (`ок` is `jr`, `ии` is `bb`). Everywhere
    # else such a Cyrillic token is an English word typed in the wrong layout,
    # so without these rows `ок` after code or a log became `jr`. They are
    # taught in context only: alone, `ок` and `ш` for `I` look the same.
    short_unknown: object = payload.get("short_russian_unknown", [])
    if not isinstance(short_unknown, list) or any(not isinstance(word, str) for word in short_unknown):
        raise ValueError("invalid short_russian_unknown scenarios")
    for word in cast(list[str], short_unknown):
        for before in ("я думаю что ", "нужно срочно ", "мы обсуждали это вчера ", "подскажи пожалуйста "):
            for role in FIELD_ROLES:
                for trigger in ("space", "pause", "enter", "punctuation"):
                    add(word, 1, before, "", role, trigger, "keep", "short_russian_unknown_correct")
    # Short words after an English field, in the same four classes as the
    # longer ones above. The rows above already keep a short Russian word, and
    # a short English word stays too. Typed in the other layout right after the
    # English text, either is converted - except a one-letter Russian word in
    # the English layout: a lone Latin letter after English text is as often an
    # initial or part of an abbreviation (`six f b i agents`), so it waits for
    # the next word, as a short English word does at the start of a message.
    # After a Russian tail the short-word families above already teach the
    # Russian side, and a Latin token there may be a command or a path
    # (`код/dpkg`), so the tail is not taught here.
    for field in cast(list[str], latin_fields):
        for tail in cast(list[str], latin_tails):
            for role in LATIN_FIELD_ROLES:
                for word in cast(list[str], short_unknown):
                    add(word, 1, field + tail, "", role, "space", "keep", "latin_field_short_unknown")
                if not tail:
                    for word in cast(list[str], payload["short_russian"]):
                        wrong = pair.translate(word, "ru", "us")
                        action = "wait" if len(word) == 1 else "convert"
                        add(wrong, 0, field, "", role, "space", action, "latin_field_short_wrong")
                for word in cast(list[str], payload["short_english"]):
                    add(word, 0, field + tail, "", role, "space", "keep", "latin_field_short_correct")
                    if not tail:
                        wrong = pair.translate(word, "us", "ru")
                        add(wrong, 1, field, "", role, "space", "convert", "latin_field_short_wrong")

    # Letters typed into the middle of a word in the other layout: the engine asks
    # about the whole word only when every other letter of it is in the other
    # layout (engine._decide_inside_word), and feature schema 6 tells the model so.
    # Without these rows it judged such a word like any other: letters typed into
    # `шмидт` under a line of code stayed Latin at 0.97, and the longer training that
    # carried them over converted more package names after a path (28.09.2026).
    # An independent test set may leave the contexts out.
    inside_contexts: object = payload.get("inside_contexts", [])
    inside_after: object = payload.get("inside_after", {})
    if not isinstance(inside_contexts, list) or any(not isinstance(text, str) for text in inside_contexts) \
            or not isinstance(inside_after, dict) or any(not isinstance(text, str) for text in inside_after.values()):
        raise ValueError("invalid inside-word scenarios")
    for name, group in (("russian", 1), ("russian_hard", 1), ("russian_unknown", 1), ("english", 0), ("technical_terms", 0)):
        tail_after = cast(dict[str, str], inside_after).get("russian" if group == 1 else "english", "")
        for word in cast(list[str], payload.get(name, [])):
            if len(word) < CONTEXT_V1_INSIDE_WORD_MIN_CHARACTERS:
                continue
            wrong = pair.translate(word, "ru" if group == 1 else "us", "us" if group == 1 else "ru")
            for before in cast(list[str], inside_contexts):
                for after in ("", tail_after):
                    for trigger in ("pause", "space"):
                        add(wrong, 1 - group, before, after, "text", trigger, "convert", "inside_wrong", inside=True)

    # A slash is punctuation in both layouts, so the engine hands the word after
    # it to the model separately. The language of the fragment before the slash
    # does not decide: a wrong-layout word after `bild/` or after `да/` is still
    # a layout error, and a component that is valid in the layout it was typed
    # in stays either way.
    heads: object = payload.get("slash_heads", {})
    if not isinstance(heads, dict):
        raise ValueError("invalid slash scenarios")
    prefixes: list[str] = []
    for name in ("russian", "english"):
        value: object = heads.get(name, [])
        if not isinstance(value, list) or any(not isinstance(prefix, str) for prefix in value):
            raise ValueError("invalid slash scenarios")
        prefixes += cast(list[str], value)
    # Every technical term is taught after a slash: a command or package name
    # after a path segment is what this family is about, and the model cannot see
    # the slash itself - `код/dpkg` reads to it as `dpkg` after a Russian word.
    for name, group in (("russian", 1), ("english", 0), ("technical_terms", 0)):
        words = cast(list[str], payload[name])
        for word in (words if name == "technical_terms" else words[:CONTEXT_V1_SLASH_WORDS_PER_FAMILY]):
            wrong = pair.translate(word, "ru" if group == 1 else "us", "us" if group == 1 else "ru")
            for before in prefixes:
                for role in SLASH_ROLES:
                    add(wrong, 1 - group, before, "", role, "space", "convert", "slash_wrong")
                    add(word, group, before, "", role, "space", "keep", "slash_correct")
    return rows


def development_metrics(probabilities: Sequence[float], labels: Sequence[int], threshold: float) -> tuple[int, int]:
    """Score development the way the runtime does: argmax, then the threshold.

    A `convert` below the serving threshold changes no text, so an epoch that
    is right but hesitant is not an improvement. Returns correct actions and
    false conversions; the test split is never scored here.
    """

    correct = false_conversions = 0
    action_count = len(ACTIONS)
    for row, label in enumerate(labels):
        scores = probabilities[row * action_count:(row + 1) * action_count]
        selected = max(range(action_count), key=lambda index: scores[index])
        action = ACTIONS[selected]
        if action == "convert" and scores[selected] < threshold:
            action = "suggest"
        correct += int(action == ACTIONS[label])
        false_conversions += int(action == "convert" and ACTIONS[label] != "convert")
    return correct, false_conversions


class CapturedSource(NamedTuple):
    path: Path
    weight: float
    # Whether the source's answers count towards the balance of actions. Correctly typed Russian
    # text (UD Taiga) adds only `keep`: counted, it would lower the importance of every `keep`
    # and push the whole model towards converting (English words and technical tokens included).
    balanced: bool = True


def captured_sources(manifest: Path = CAPTURED) -> list[CapturedSource]:
    """The captured question files with their weights, each checked against its pinned SHA-256."""

    payload = json.loads(manifest.read_text(encoding="utf-8"))
    if not isinstance(payload, dict) or payload.get("schema_version") != 1 or tuple(payload.get("columns", ())) != CAPTURED_COLUMNS:
        raise ValueError("invalid captured question manifest")
    sources: list[CapturedSource] = []
    for source in payload.get("sources", []):
        path = manifest.parent / str(source["file"])
        if hashlib.sha256(path.read_bytes()).hexdigest() != source["sha256"]:
            raise ValueError(f"captured questions changed: {path.name}")
        weight = float(source["weight"])
        balanced = source.get("balance", True)
        if not math.isfinite(weight) or weight <= 0 or not isinstance(balanced, bool):
            raise ValueError(f"invalid weight or balance for {path.name}")
        sources.append(CapturedSource(path, weight, balanced))
    return sources


def signatures(original: str, alternative: str) -> set[str]:
    """The scenario families a question belongs to: its two readings, as typed and without edge signs."""

    plain = min(original.casefold(), alternative.casefold())
    core = min(original.strip(CAPTURED_EDGE_SIGNS).casefold(), alternative.strip(CAPTURED_EDGE_SIGNS).casefold())
    return {plain, core}


def captured_rows(path: Path) -> Iterator[tuple[ContextEvidence, int, int]]:
    """Each distinct captured question once: its evidence, label index and how often it was asked."""

    with lzma.open(path, "rt", encoding="utf-8") as handle:
        for line in handle:
            values = json.loads(line)
            row = dict(zip(CAPTURED_COLUMNS, values, strict=True))
            # No application name, as in every scenario row: the model does not read it.
            field = FieldContext("", "captured", str(row.pop("before")), str(row.pop("after")), row.pop("role"))
            count, label = int(row.pop("count")), ACTIONS.index(row.pop("label"))
            yield ContextEvidence(field=field, **row), label, count


def _python_predict(data: Packed, weights: array[float]) -> array[float]:
    """The native kernel's prediction, for a machine without a C compiler (tests on Windows)."""

    width = len(ACTIONS)
    result = array("d")
    for row in range(len(data.labels)):
        scores = [0.0] * width
        for position in range(data.offsets[row], data.offsets[row + 1]):
            for action in range(width):
                scores[action] += weights[data.indices[position] * width + action] * data.values[position]
        result.extend(kernel_softmax(scores))
    return result


def train(rows: list[Row], captured: Sequence[CapturedSource] = (),
          reserved: frozenset[str] = frozenset()) -> tuple[dict[str, list[float]], int, float]:
    base = [(extract_context_features(row.evidence, CONTEXT_LINE_FEATURE_VERSION), ACTIONS.index(row.action)) for row in rows if row.split == "train"]
    development = [(extract_context_features(row.evidence, CONTEXT_LINE_FEATURE_VERSION), ACTIONS.index(row.action))
                   for row in rows if row.split == "development"]
    if not base or not development:
        raise ValueError("empty training or development split")
    action_count = len(ACTIONS)

    def examples() -> Iterator[tuple[dict[str, float], int, float, int, bool]]:
        """Distinct training rows with weight, count and balance; read twice rather than held in memory."""

        for features, label in base:
            yield features, label, 1.0, 1, True
        for source in captured:
            for evidence, label, count in captured_rows(source.path):
                # A question about a development or held-out family would teach what those
                # families measure (`швы` for `ids` is a development family).
                if not signatures(evidence.original, evidence.alternative) & reserved:
                    yield extract_context_features(evidence, CONTEXT_LINE_FEATURE_VERSION), label, source.weight, count, source.balanced

    frequency: Counter[str] = Counter()
    label_counts: Counter[int] = Counter()
    counts = array("I")
    for features, label, _weight, count, balanced in examples():
        for name in features:
            frequency[name] += count
        if balanced:
            label_counts[label] += count
        counts.append(count)
    total = sum(label_counts.values())
    names = sorted(sorted(frequency, key=lambda name: (-frequency[name], name))[:CONTEXT_V1_MAX_TRAINED_FEATURES])
    # Inverse frequency already balances the classes. The former extra 2.0 bias
    # towards `keep` was chosen for a small, highly repetitive corpus; on the
    # larger one it held the `convert` probability under the fixed 0.985
    # serving threshold, so a correct decision still changed no text.
    # Both this weight and the step below were selected on development only.
    importance_by_label = {label: total / (action_count * count) * (CONTEXT_V1_KEEP_IMPORTANCE if label == 0 else 1.0)
                           for label, count in label_counts.items()}
    distinct = Packed.build(((features, label, importance_by_label[label] * weight) for features, label, weight, _count, _balanced in examples()), names)
    # Every question as often as it was asked, in one fixed shuffled order: an epoch does not end
    # on a single source, and the same rows always give the same weights.
    expanded = array("I", (row for row, count in enumerate(counts) for _ in range(count)))
    order = sorted(range(len(expanded)), key=lambda index: hashlib.sha256(f"{NAMESPACE}:order:{index}".encode()).hexdigest())
    data = Packed(array("Q", [0]), array("I"), array("d"), array("B"), array("d"))
    for index in order:
        row = expanded[index]
        start, end = distinct.offsets[row], distinct.offsets[row + 1]
        data.indices.extend(distinct.indices[start:end])
        data.values.extend(distinct.values[start:end])
        data.offsets.append(len(data.indices))
        data.labels.append(distinct.labels[row])
        data.importance.append(distinct.importance[row])
    del distinct, expanded, order
    held = Packed.build(((features, label, importance_by_label.get(label, 1.0)) for features, label in development), names)
    try:
        # The native kernel is a Linux training tool; elsewhere (tests on Windows) the same epoch runs in Python.
        kernel: Kernel | None = Kernel.load() if sys.platform != "win32" else None
    except RuntimeError:
        kernel = None
    step: Callable[[Packed, array[float], array[float], float], None] = kernel.epoch if kernel is not None else python_epoch
    predict: Callable[[Packed, array[float]], array[float]] = kernel.predict if kernel is not None else _python_predict
    weights = array("d", [0.0]) * (len(names) * action_count)
    accumulators = array("d", [1.0]) * (len(names) * action_count)
    best: dict[str, list[float]] = {}
    best_loss, best_epoch = math.inf, 0
    best_correct = -1
    # Selection follows the runtime rule under a zero-false-conversion budget,
    # measured on development only: the epoch with the most correct actions
    # and no false conversion, the lower log-loss breaking a tie. Weighted
    # log-loss alone is dominated by the `keep` mass and would stop before the
    # model reaches the fixed serving threshold.
    # How long to train is chosen on disclosed data (see CONTEXT_V1_EPOCHS):
    # development keeps gaining correct actions, but in the engine a word typed
    # after a stray key (`aghbdtn`) needs a few epochs before the typo evidence
    # carries it, while every further epoch converts more package names typed
    # as intended after a path and spoils more correct sentences.
    # Fixed order and optimizer parameters; test never selects an epoch.
    for epoch in range(EPOCHS):
        step(data, weights, accumulators, CONTEXT_V1_LEARNING_RATE)
        probabilities = predict(held, weights)
        loss = 0.0
        for row, label in enumerate(held.labels):
            loss -= importance_by_label.get(label, 1.0) * math.log(max(LOG_LOSS_PROBABILITY_FLOOR, probabilities[row * action_count + label]))
        loss /= len(held.labels)
        correct, false_conversions = development_metrics(probabilities, held.labels, CONTEXT_V1_CONVERSION_THRESHOLD)
        print(f"epoch {epoch + 1}: development correct {correct}, false conversions {false_conversions}, "
              f"loss {loss:.{DETERMINISTIC_ROUNDING_DECIMALS}f}", file=sys.stderr, flush=True)
        if false_conversions == 0 and (correct > best_correct or (correct == best_correct and loss < best_loss)):
            best_correct, best_loss, best_epoch = correct, loss, epoch + 1
            best = {name: [round(weights[index * action_count + action], DETERMINISTIC_ROUNDING_DECIMALS)
                           for action in range(action_count)] for index, name in enumerate(names)}
    if not best:
        raise ValueError("no epoch reached the development false-conversion budget")
    return best, best_epoch, best_loss


def evaluate(model: ContextModel, rows: list[Row], split: str) -> dict[str, object]:
    counts: Counter[str] = Counter()
    by_category: dict[str, Counter[str]] = {}
    failures: list[dict[str, object]] = []
    for row in rows:
        if row.split != split:
            continue
        prediction = model.predict(row.evidence)
        counts["rows"] += 1
        counts["correct_actions"] += int(prediction.action == row.action)
        counts["desired_conversions"] += int(row.action == "convert")
        counts["converted_correctly"] += int(prediction.action == row.action == "convert")
        counts["false_conversions"] += int(prediction.action == "convert" and row.action != "convert")
        counts["baseline_false_conversions"] += int(row.evidence.baseline_convert and row.action != "convert")
        counts["baseline_converted_correctly"] += int(row.evidence.baseline_convert and row.action == "convert")
        category = by_category.setdefault(row.category, Counter())
        category["rows"] += 1
        category["correct_actions"] += int(prediction.action == row.action)
        category["false_conversions"] += int(prediction.action == "convert" and row.action != "convert")
        if prediction.action != row.action and len(failures) < CONTEXT_V1_MAX_REPORTED_FAILURES:
            failures.append({"token": row.evidence.original, "category": row.category, "expected": row.action, "actual": prediction.action})
    return {"counts": dict(counts), "categories": {name: dict(value) for name, value in sorted(by_category.items())}, "examples": failures}


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify", action="store_true")
    parser.add_argument("--artifact", type=Path, default=ARTIFACT_PATH)
    parser.add_argument("--report", type=Path, default=REPORT)
    parser.add_argument("--development-only", action="store_true")
    args = parser.parse_args(argv)
    rows = build_corpus()
    # The families development and the holdout measure, by token only (no label is read): no
    # captured question about them is trained on.
    reserved = {row.family for row in rows if row.split == "development"}
    if not args.development_only:
        reserved |= {row.family for row in build_corpus(HOLDOUT, held_out=True)}
    weights, epoch, loss = train(rows, captured_sources(), frozenset(reserved))
    digest = hashlib.sha256(json.dumps(weights, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()
    payload = {"feature_version": CONTEXT_LINE_FEATURE_VERSION, "actions": list(ACTIONS), "version": "context-v1-" + digest[:VERSION_HASH_CHARACTERS],
               "conversion_threshold": CONTEXT_V1_CONVERSION_THRESHOLD, "weights_sha256": digest, "weights": weights}
    model = ContextModel({name: tuple(value) for name, value in weights.items()}, "context-v1-" + digest[:VERSION_HASH_CHARACTERS],
                         feature_version=CONTEXT_LINE_FEATURE_VERSION)
    # The development-only path neither reads nor scores reserved test rows.
    if not args.development_only:
        rows += build_corpus(HOLDOUT, held_out=True)
    groups = {split: {row.family for row in rows if row.split == split} for split in ("train", "development", "test")}
    overlap = len(groups["train"] & groups["test"]) + len(groups["development"] & groups["test"])
    test = evaluate(model, rows, "test")
    counts = cast(dict[str, int], test["counts"])
    if overlap:
        raise ValueError("physical token leakage into context holdout")
    passed = not args.development_only and counts.get("false_conversions", 0) == 0 and counts.get("converted_correctly", 0) >= counts.get("baseline_converted_correctly", 0)
    report = {
        "schema_version": 1, "model_version": model.version,
        "evidence_scope": "author-created-synthetic-scenarios-and-engine-questions-on-typed-public-text-not-real-users",
        "split_namespace": NAMESPACE, "family_counts": {name: len(value) for name, value in groups.items()},
        "test_overlap": overlap, "selected_epoch": epoch, "development_loss": round(loss, DETERMINISTIC_ROUNDING_DECIMALS),
        "sources_sha256": hashlib.sha256(SCENARIOS.read_bytes()).hexdigest(),
        "captured_sha256": hashlib.sha256(CAPTURED.read_bytes()).hexdigest(),
        "term_frequency_sha256": hashlib.sha256(TERM_FREQUENCY_PATH.read_bytes()).hexdigest(),
        "holdout_sha256": None if args.development_only else hashlib.sha256(HOLDOUT.read_bytes()).hexdigest(),
        "runtime_sha256": hashlib.sha256((ROOT / "src/keyswitch/context_model.py").read_bytes()).hexdigest(),
        "policy_sha256": hashlib.sha256((ROOT / "src/keyswitch/short_words.py").read_bytes()).hexdigest(),
        "trainer_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "baseline_sha256": hashlib.sha256((ROOT / "src/keyswitch/resources/models/layout_intent_v1.ksm").read_bytes()).hexdigest(),
        "supplement_sha256": hashlib.sha256((ROOT / "src/keyswitch/resources/lexicon-supplement-ru_RU.json").read_bytes()).hexdigest(),
        "artifact_sha256": hashlib.sha256(canonical(payload)).hexdigest(),
        "baseline_scope": "isolated-token LanguageDetector, not full application",
        "development": evaluate(model, rows, "development"), "test": test, "quality_gates_passed": passed,
    }
    artifact_bytes, report_bytes = canonical(payload), canonical(report)
    if args.verify:
        if args.artifact.read_bytes() != artifact_bytes or args.report.read_bytes() != report_bytes:
            raise ValueError("context model or evidence is not reproducible")
    else:
        args.artifact.parent.mkdir(parents=True, exist_ok=True)
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.artifact.write_bytes(artifact_bytes)
        args.report.write_bytes(report_bytes)
    print(json.dumps({"model": model.version, "quality_gates_passed": passed, "test": test, "selected_epoch": epoch}, ensure_ascii=False, indent=REPORT_JSON_INDENT))
    return 0 if passed or args.development_only else 1


if __name__ == "__main__":
    raise SystemExit(main())
