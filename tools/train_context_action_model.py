#!/usr/bin/env python3
"""Fit an isolated action candidate; never read the prospective test split.

Natural surface forms are KEEP labels. Layout corruption and spelling errors
are declared interventions, not labels inferred from a dictionary or teacher.
The engine evaluator must separately check the complete physical input stream.
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import math
import re
from array import array
from collections import Counter, defaultdict
from collections.abc import Callable, Iterable, Mapping, Sequence
from dataclasses import asdict, dataclass, replace
from pathlib import Path
from typing import cast
from unittest.mock import patch

from keyswitch.context_action_features import extract_action_features
from keyswitch.context_model import ACTIONS, AfterOrigin, ContextAction, ContextEvidence, ContextModel, term_bucket
from keyswitch.context_policy import evidence_for_decision
from keyswitch.detector import LanguageDetector
from keyswitch.identifier_lexicon import IdentifierLexicon
from keyswitch.input_context import FieldContext
from keyswitch.lexicon_supplement import supplement_words
from keyswitch.intent_model import CorrectionTrigger, IntentModelStatus, LinearNgramModel
from keyswitch.language_model import LanguageModel
from keyswitch.ortho_model import OrthoModel
from keyswitch.short_words import TRUSTED_SINGLE_LETTER_WORDS
from keyswitch.word_decision import automatic_word_decision

from keyswitch.constants.model_protocol import (
    FITTING_SPLITS,
    REJECTED_BEFORE_TEST,
    SEALED_BEFORE_TEST,
)
from reference_lexicon import reference_models
from action_epoch_selection import EpochSelection, assess_epoch
from context_action_spans import SpanFrame, build_span_curriculum
from context_deferral import deferred_isolated, lookahead_focus, plausible_reading
from context_lookahead_curriculum import LookaheadAnchor, LookaheadSeed, build_lookahead_curriculum
from context_optimizer import Kernel, Packed
from context_physical_keys import translated as translated
from evaluate_context_action_sequences import LEDGER_ROOT, runtime_provenance
from freeze_context_action_corpus import CorpusRow, load_split, physical, typo_variants
from freeze_context_action_holdout import ledger_test_aliases
from reconcile_context_action_corpus import expanded_aliases
from keyswitch.constants.file_formats import HEXADECIMAL_BASE, VERSION_HASH_CHARACTERS
from keyswitch.constants.keyboard import LAYOUT_GROUP_COUNT
from keyswitch.constants.models import CONTEXT_ACTION_FEATURE_VERSION, PLANNED_CONTEXT_WORD_MAX_CHARACTERS
from keyswitch.constants.training import (
    BOUNDARY_EVENT_CHOICES,
    CAPITAL_CITATION_MAX_LETTERS,
    CAPITAL_CITATION_MIN_LETTERS,
    CAPITAL_CITATION_PUNCTUATION,
    CAPITAL_CITATION_RUSSIAN_BUCKET,
    CITATION_SIGN_HEADS,
    DOTTED_RUSSIAN_ABBREVIATIONS,
    COMMAND_FAMILY_IDENTIFIER_PARTS,
    DETERMINISTIC_CHOICE_HEX_DIGITS,
    DETERMINISTIC_ROUNDING_DECIMALS,
    FEATURE_MASS_TOLERANCE,
    FIELD_AFTER_SAMPLE_MODULUS,
    IDENTIFIER_DROPOUT_FAMILIES,
    IDENTIFIER_SUFFIX_SEGMENTS,
    LEXICAL_PAIR_ANCHOR_VARIANTS,
    LOG_LOSS_PROBABILITY_FLOOR,
    LOOKAHEAD_ANCHOR_MAX_CHARACTERS,
    LOOKAHEAD_ANCHOR_MIN_CHARACTERS,
    MASS_REPORT_DECIMALS,
    MAX_CONTEXTS_PER_FAMILY,
    QUOTE_TAIL,
    QUOTE_TAIL_MODULUS,
    RUSSIAN_INITIAL_LETTERS,
    STRANDED_PREVIOUS_MIN_LETTERS,
    STRANDED_PREVIOUS_WEIGHT,
    ACTION_DEFERRED_WORD_MAX_CHARACTERS,
    ACTION_SHORT_WORD_MAX_CHARACTERS,
    NET_BENEFIT_FALSE_INDEX,
    NET_BENEFIT_THRESHOLD_INDEX,
)


ROOT = Path(__file__).resolve().parents[1]
RECIPE = ROOT / "model/context_v3/recipe.json"
ARTIFACT = "context-action.json"
SEAL = "candidate-seal.json"
WORDS = re.compile(r"[A-Za-zА-Яа-яЁё]+(?:['’\-][A-Za-zА-Яа-яЁё]+)*")


def canonical(value: object) -> bytes:
    return (json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":")) + "\n").encode()


def checksum(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


# The other-language text a legitimately inserted word follows, by the word's layout group:
# a plain sentence, and a quoted name with an opening parenthesis as edited prose cites it.
MIXED_INSERTION_CONTEXTS: dict[int, tuple[str, ...]] = {
    0: ("в сообщении написано ", "в оригинале «Ночь» ("),
    1: ("the message says ", "in the original \"Night\" ("),
}


def natural_mixed_contexts(rows: Sequence[CorpusRow]) -> dict[int, tuple[str, ...]]:
    """Left contexts of a split's own rows, per language, for the insertion frames.

    The base treebanks are monolingual: TRAIN of corpus v13 holds 130 Latin rows after
    Russian text among 18 363, while encyclopedic prose (the GSD part of test v13) cites a
    Latin name or abbreviation in every other sentence, and the pair typed in the wrong
    layout (an abbreviation of three capitals, a capitalised name) is what the v15 candidate left as
    typed. The two fixed phrases of MIXED_INSERTION_CONTEXTS stay; a third insertion pair
    per row takes the left context of a real row of the other language from the same
    split, chosen by hash, so the model sees an insertion after prose it did not write;
    only a citation-shaped word takes it (citation_shaped). A context qualifies when it
    has words and all of them are in the language's script.
    """
    pools: dict[int, set[str]] = {0: set(), 1: set()}
    for row in rows:
        if row.group not in (0, 1) or not row.layout_representable:
            continue
        words = WORDS.findall(row.before)
        if not words:
            continue
        cyrillic = [any("а" <= char.casefold() <= "я" or char.casefold() == "ё" for char in word) for word in words]
        if all(cyrillic) if row.group == 1 else not any(cyrillic):
            pools[row.group].add(row.before)
    return {group: tuple(sorted(pool)) for group, pool in pools.items()}


def citation_shaped(original: str, group: int) -> bool:
    """A word prose of the other language cites: a capitalised or upper-case form, or one no lexicon knows.

    Encyclopedic Russian cites abbreviations and brand names, not `deployment`; a lowercase
    word the lexicon knows is framed after the fixed insertion phrases only. Framing every
    word after real prose of the other language taught the corpus v16 candidate that any
    Latin token after a Russian clause may be an insertion, and `ns` after `мы решили, что `
    fell to convert p=0.982 under the serving threshold.
    """
    letters = "".join(char for char in original if char.isalpha())
    return bool(letters) and (letters.isupper() or letters.istitle() or not plausible_reading(original, group))


_ONBOARD_CAPITALS: frozenset[str] | None = None


def onboard_capital_forms() -> frozenset[str]:
    """Russian forms the onboard lexicon writes in capitals (`США`, `ВДВ`), casefolded.

    The ARPA list keeps the case its source wrote; the language model folds it away on load.
    """
    global _ONBOARD_CAPITALS
    if _ONBOARD_CAPITALS is None:
        forms: set[str] = set()
        section = ""
        with (ROOT / "model/intent_v1/sources/ru_RU.lm").open(encoding="utf-8") as stream:
            for line in stream:
                line = line.strip()
                if line.startswith("\\"):
                    section = line
                    continue
                form = line.split()[1] if section == "\\1-grams:" and len(line.split()) > 1 else ""
                if len(form) > 1 and form.isalpha() and form.isupper():
                    forms.add(form.casefold())
        _ONBOARD_CAPITALS = frozenset(forms)
    return _ONBOARD_CAPITALS


def capital_citation(original: str, alternate: str) -> bool:
    """A Latin abbreviation in capitals whose Cyrillic reading is a rare word, not a Russian abbreviation.

    `WBC` reads `ЦИС`, `IBF` reads `ШИА`: both readings are known to the lexicon (the OpenSubtitles
    supplement holds `цис` and `шиа`), so citation_shaped framing left them out, and after Russian
    prose the model converted them (the corpus v22 candidate turned `по версии WBC` into `ЦИС` on
    test v22 with the early switch off, as did the baseline pair). Russian text counts fewer than five
    such Cyrillic words (CAPITAL_CITATION_RUSSIAN_BUCKET), and none of them is written in capitals by
    the onboard lexicon, which writes `США`, `ВДВ`, `МВД` that way: those typed in the Latin layout
    still convert.
    """
    letters = "".join(char for char in original if char.isalpha())
    return (original.isascii() and original.isalpha() and original.isupper()
            and CAPITAL_CITATION_MIN_LETTERS <= len(letters) <= CAPITAL_CITATION_MAX_LETTERS
            and plausible_reading(alternate, 1)
            and term_bucket(alternate, "russian") == CAPITAL_CITATION_RUSSIAN_BUCKET
            and alternate.casefold() not in onboard_capital_forms())


def stranded_previous(before: str, group: int) -> str | None:
    """`before` with its last word as typed in the other layout, when that reading is a word there too.

    The left context of a phrase typed whole in the wrong layout after the engine has converted all
    but the word whose wrong reading it could not tell from a real one (`we went in` typed in the
    Russian layout leaves `we went шт`). None when the last word does not end `before`, has fewer
    than two letters or signs, or reads as no word of the other language.
    """
    words = WORDS.findall(before)
    if not words:
        return None
    previous = words[-1]
    head = before.rstrip()
    if not head.endswith(previous) or len(previous) < STRANDED_PREVIOUS_MIN_LETTERS or not previous.isalpha():
        return None
    try:
        reading = translated(previous, group)
    except ValueError:
        return None
    if not reading.isalpha() or not plausible_reading(reading, 1 - group):
        return None
    return head[:-len(previous)] + reading + before[len(head):]


def variant_choice(identifier: str, purpose: str, count: int) -> int:
    return int(hashlib.sha256((purpose + ":" + identifier).encode()).hexdigest()[:DETERMINISTIC_CHOICE_HEX_DIGITS], HEXADECIMAL_BASE) % count


@dataclass(frozen=True)
class ActionRow:
    identifier: str
    original: str
    group: int
    field: FieldContext
    trigger: CorrectionTrigger
    literal_tail: str
    action: ContextAction
    category: str
    boundary_text: str = ""
    sample_weight: float = 1.0
    after_origin: AfterOrigin = "none"
    parent_family: str = ""

    def __post_init__(self) -> None:
        if type(self.sample_weight) not in (int, float) or not math.isfinite(self.sample_weight) or self.sample_weight <= 0:
            raise ValueError("action sample weight must be positive and finite")


def training_order(rows: Sequence[ActionRow]) -> list[ActionRow]:
    """Interleave corpus sources deterministically before optimizer updates."""
    return sorted(rows, key=lambda row: hashlib.sha256(row.identifier.encode()).digest())


class FeatureMass:
    """Stream frame-presence mass with constant summation state per feature.

    Each frame contributes its sample weight once, independently of feature
    magnitude or class importance. Compensated addition avoids accumulating
    rounding drift when one family is represented by many fractional frames.
    """

    def __init__(self) -> None:
        self.values: dict[str, float] = {}
        self._errors: dict[str, float] = {}

    def add(self, features: Mapping[str, float], sample_weight: float) -> None:
        if type(sample_weight) not in (int, float) or not math.isfinite(sample_weight) or sample_weight <= 0:
            raise ValueError("feature sample weight must be positive and finite")
        for name in features:
            previous = self.values.get(name, 0.0)
            increment = sample_weight - self._errors.get(name, 0.0)
            total = previous + increment
            if not math.isfinite(total):
                raise ValueError("feature mass overflow")
            self._errors[name] = (total - previous) - increment
            self.values[name] = total


def select_features(masses: Mapping[str, float], minimum: float, maximum: int) -> list[str]:
    """Use sample mass, with absolute 1e-9 cutoff tolerance and stable ties.

    Ranking rounds mass to nine decimal places, then breaks ties by name.
    Returned columns retain the canonical alphabetical order used by Packed.
    """
    if type(minimum) not in (int, float) or not math.isfinite(minimum) or minimum <= 0:
        raise ValueError("minimum feature mass must be positive and finite")
    if type(maximum) is not int or maximum <= 0:
        raise ValueError("maximum features must be a positive integer")
    if any(type(value) not in (int, float) or not math.isfinite(value) or value < 0 for value in masses.values()):
        raise ValueError("invalid accumulated feature mass")
    eligible = (name for name, value in masses.items() if value >= minimum - FEATURE_MASS_TOLERANCE)
    return sorted(sorted(eligible, key=lambda name: (-round(masses[name], DETERMINISTIC_ROUNDING_DECIMALS), name))[:maximum])


def select_rows(rows: Sequence[CorpusRow], maximum: int) -> list[CorpusRow]:
    """Balance languages before scoring; retain at most two contexts per family."""
    selected: list[CorpusRow] = []
    counts: Counter[tuple[int, str]] = Counter()
    languages: Counter[int] = Counter()
    for row in sorted(rows, key=lambda item: hashlib.sha256(item.identifier.encode()).digest()):
        if row.group not in (0, 1) or not row.layout_representable:
            continue
        group = row.group
        try:
            translated(row.original, group)
        except ValueError:
            continue
        key = group, row.family
        if languages[group] >= maximum // LAYOUT_GROUP_COUNT or counts[key] >= MAX_CONTEXTS_PER_FAMILY:
            continue
        counts[key] += 1
        languages[group] += 1
        selected.append(row)
    return selected


def action_rows(rows: Sequence[CorpusRow]) -> list[ActionRow]:
    result: list[ActionRow] = []
    natural_contexts = natural_mixed_contexts(rows)
    triggers: tuple[CorrectionTrigger, ...] = ("space", "space", "enter", "punctuation", "tab", "pause")
    for row in rows:
        if row.group not in (0, 1) or not row.layout_representable:
            raise ValueError("action rows require a representable single-layout token")
        group = row.group
        alternate = translated(row.original, group)
        number = int(hashlib.sha256(row.identifier.encode()).hexdigest()[:DETERMINISTIC_CHOICE_HEX_DIGITS], HEXADECIMAL_BASE)
        trigger = triggers[number % len(triggers)]
        boundary_text = ""
        if trigger == "space":
            boundary_text = row.spacing[:1] or " "
        elif trigger == "punctuation":
            boundary_text = next((char for char in row.literal_tail if not char.isspace()), ".")
        elif trigger in {"enter", "tab"} and variant_choice(row.identifier, "boundary-event", BOUNDARY_EVENT_CHOICES):
            boundary_text = "\n" if trigger == "enter" else "\t"
        applications = ("Telegram", "Code", "chrome", "UnseenEditor")
        application = applications[variant_choice(row.identifier, "application", len(applications))]
        for context_name, before in (("observed", row.before), ("empty", "")):
            # Lookahead is optional field evidence, never assumed available.
            after = row.after if context_name == "observed" and variant_choice(row.identifier, "field-after", FIELD_AFTER_SAMPLE_MODULUS) == 0 else ""
            field = FieldContext(application, "public-training", before, after, "unknown")
            # Corpus punctuation/spacing is not the engine's segmented trailing
            # strokes. These frames describe a word at a direct boundary.
            tail = ""
            identity = row.identifier + ":" + context_name
            keep_action: ContextAction = "keep"
            action: ContextAction = "convert"
            alone = not WORDS.search(before) and not WORDS.search(after)
            # One curated letter is the exception to that: the intent *is* observable,
            # and telling the model otherwise is why it answered "wait" for a lone `z`.
            # A Russian utterance opens with one of а/и/с/в/к/у/о/я once in seven
            # sentences (UD Taiga, 121 967 sentences); no English sentence in UD EWT
            # opens with a lone f/b/c/d/r/e/j/z. The lexicon separates the two members -
            # the own reading is a word or it is not - so this is a decidable label, not
            # a coin toss. Measured 17.09.2026; see short_words.TRUSTED_SINGLE_LETTER_WORDS.
            curated_letter = (len(row.original) == 1 and len(alternate) == 1
                              and (row.original.casefold() in TRUSTED_SINGLE_LETTER_WORDS
                                   or alternate.casefold() in TRUSTED_SINGLE_LETTER_WORDS))
            if alone and not curated_letter and deferred_isolated(row.original, alternate, group):
                # An isolated short reading has no observable intent label.
                # Digits/punctuation do not supply a neighbouring language.
                # Both members preserve text and take the same deferred action.
                # Deferring every three-letter reading was measured on 17.09.2026 and
                # made the fitted model worse on chat-like first words, not better
                # (.t/reliable-release-2026-09-12/SHORT-ISOLATED-CURRICULUM.md), and again
                # on 01.10.2026 (corpus v12: `rjn` by the pause and `pm2` broke); deciding
                # them all at once turned `зум` alone into the Debian command `pev` (corpus
                # v10, v11). A three-letter reading is deferred only when both of its
                # readings are plausible (context_deferral). Deciding two letters the same
                # way was measured on 04.10.2026 (corpus v19, against the replay of the
                # owner's typing): one-word messages such as `гш` became `ui`, but so did one
                # the owner kept, and `чс`, `ер`, `ым`, `тз` alone turned into Latin - 9 more
                # words converted right, as many left in the wrong layout and 12 more false
                # conversions; alone, two letters stay deferred.
                action = "suggest" if trigger in ("enter", "tab", "punctuation") else "wait"
                keep_action = action
            result.append(ActionRow(identity + ":keep", row.original, group, field,
                                    trigger, tail, keep_action, "natural_surface", boundary_text))
            result.append(ActionRow(identity + ":wrong", alternate, 1 - group, field,
                                    trigger, tail, action, "layout_intervention", boundary_text))
        # A legitimate insertion may have neighbours in the other language. Half of the
        # rows stand inside a parenthesis after a quoted name, the way edited prose
        # cites a foreign word: on its sealed test the candidate of corpus v10 (01.10.2026)
        # converted short Latin words inside two Russian sentences of UD GSD.
        mixed_contexts = MIXED_INSERTION_CONTEXTS[group]
        mixed = mixed_contexts[variant_choice(row.identifier, "mixed-context", len(mixed_contexts))]
        mixed_field = FieldContext(application, "public-training", mixed, "", "unknown")
        result.append(ActionRow(row.identifier + ":mixed", row.original, group,
                                mixed_field,
                                trigger, "", "keep", "mixed_language_insertion", boundary_text))
        result.append(ActionRow(row.identifier + ":mixed:wrong", alternate, 1 - group,
                                mixed_field, trigger, "", "convert",
                                "mixed_language_layout_intervention", boundary_text))
        other = natural_contexts[1 - group]
        if other and group == 0 and capital_citation(row.original, alternate) and plausible_reading(alternate, 1 - group):
            # A capital citation whose Cyrillic reading is a rare word keeps after Russian prose. Its
            # Cyrillic reading gets no convert frame: a Cyrillic word in capitals that no lexicon writes
            # so may be the writer's own abbreviation (`ГА`, `ГАК` in the owner's typing) and stays.
            natural_field = FieldContext(application, "public-training",
                                         other[variant_choice(row.identifier, "mixed-natural", len(other))], "", "unknown")
            result.append(ActionRow(row.identifier + ":mixed-capital", row.original, group, natural_field,
                                    trigger, "", "keep", "mixed_language_insertion", boundary_text))
        if other and group == 0 and citation_shaped(row.original, group) and not plausible_reading(alternate, 1 - group):
            # Only the direction that failed: a Latin citation inside Russian prose. The mirror
            # (a Russian name cited by English prose) made the corpus v16 candidate convert
            # Latin tokens after English text it had kept before. And only a citation whose
            # Cyrillic reading is no word: the Cyrillic keys of a Latin abbreviation after Russian
            # prose are that abbreviation, but `чем` after Russian prose is `чем` whatever command
            # its Latin keys spell (the corpus v16 candidate converted such a `чем` on
            # development), and `ns` after Russian prose is `ты`, which the keep frame of an
            # unknown Latin token after Russian prose taught against (corpus v17: p=0.983).
            natural_field = FieldContext(application, "public-training",
                                         other[variant_choice(row.identifier, "mixed-natural", len(other))], "", "unknown")
            result.append(ActionRow(row.identifier + ":mixed-natural", row.original, group, natural_field,
                                    trigger, "", "keep", "mixed_language_insertion", boundary_text))
            result.append(ActionRow(row.identifier + ":mixed-natural:wrong", alternate, 1 - group, natural_field,
                                    trigger, "", "convert", "mixed_language_layout_intervention", boundary_text))
            # Edited prose opens the citation with a sign typed in the Latin layout; the keys of
            # ``, ' and " are the letters ёё, э and Э in the Russian layout, so the Cyrillic reading
            # of the whole token is letters only and the token looks like a word of five. The
            # corpus v19 candidate converted two such citations of test v15 (two backticks before
            # a capitalised Latin word, Russian prose on the left) that the baseline pair kept.
            # One sign-headed pair per citation teaches that the head changes nothing.
            head = CITATION_SIGN_HEADS[variant_choice(row.identifier, "citation-head", len(CITATION_SIGN_HEADS))]
            headed = head + row.original
            result.append(ActionRow(row.identifier + ":mixed-natural:head", headed, group, natural_field,
                                    trigger, "", "keep", "mixed_language_insertion", boundary_text))
            result.append(ActionRow(row.identifier + ":mixed-natural:head:wrong", translated(headed, group), 1 - group,
                                    natural_field, trigger, "", "convert", "mixed_language_layout_intervention", boundary_text))
        stranded = stranded_previous(row.before, group)
        if stranded is not None:
            # A phrase typed whole in the other layout: the engine converts each word, but a previous
            # word whose wrong reading is a word of the other language stays as typed (`here` typed in
            # the Russian layout is `руку`). The word after it is still the phrase's, and converts:
            # `руку ерун` is `here they`. Natural frames only ever stand after correctly typed text,
            # and the corpus v23 candidates left `ерун` after `руку` as typed (p=0.977).
            # The phrase may open with that word too, the field holding nothing else.
            opening = stranded[len(stranded.rstrip()) - len(WORDS.findall(stranded)[-1]):]
            for suffix, left in (("", stranded), (":opening", opening)):
                stranded_field = FieldContext(application, "public-training", left, "", "unknown")
                result.append(ActionRow(row.identifier + ":stranded-previous" + suffix + ":wrong", alternate, 1 - group,
                                        stranded_field, trigger, "", "convert", "layout_intervention", boundary_text,
                                        STRANDED_PREVIOUS_WEIGHT))
        isolated = not WORDS.search(row.before) and deferred_isolated(row.original, alternate, group)
        if (group == 1 and row.original.isalpha() and not isolated
                and variant_choice(row.identifier, "quote-tail", QUOTE_TAIL_MODULUS) == 0):
            # A quotation closes with its quote typed in the layout of the word, and the Russian `"` is
            # the `@` key: `привет"` typed in the Latin layout is `ghbdtn@`. The corpus splits the quote
            # off as a token of its own, so no frame held a Russian word with the `@` of its quote, and
            # every corpus v23 candidate left `ghbdtn@` as typed with the early switch off (p=0.988).
            # A short word standing alone keeps its deferred action: the quote tells no intent.
            quoted = row.original + QUOTE_TAIL
            quote_field = FieldContext(application, "public-training", row.before, "", "unknown")
            result.append(ActionRow(row.identifier + ":quote-tail", quoted, group, quote_field,
                                    trigger, "", "keep", "natural_surface", boundary_text))
            result.append(ActionRow(row.identifier + ":quote-tail:wrong", translated(quoted, group), 1 - group,
                                    quote_field, trigger, "", "convert", "layout_intervention", boundary_text))
        for index, typo in enumerate(typo_variants(row.original, row.identifier)):
            result.append(ActionRow(row.identifier + f":spelling:{index}", typo, group,
                                    FieldContext(application, "public-training", row.before, "", "unknown"),
                                    trigger, "", "keep", "spelling_intervention", boundary_text))
    return result


def capital_citation_curriculum(source_rows: Sequence[CorpusRow], refused: frozenset[str],
                                options: Mapping[str, object]) -> tuple[list[ActionRow], dict[str, object]]:
    """Latin abbreviations in capitals after Russian prose whose Cyrillic reading is a rare word: keep.

    Natural text holds almost none (corpus v22 TRAIN: 17 Latin rows in capitals whose Cyrillic
    reading is a known word counted fewer than five times in Russian text, against 421 whose reading
    is no word), and the model learned from the lexicon alone that a known Cyrillic reading after
    Russian prose is the word meant. The physical class is the same whichever language named it: the
    keys of a rare Russian word of three to five letters, typed in the Latin layout and in capitals,
    are such an abbreviation (`цис` is `WBC`). Words are drawn from the portable lexicon by hash,
    `words_by_length` per length; a word held by this corpus's test or by any accessed test is refused
    by its aliases, as the other lexical curricula refuse it, and so is a word the onboard lexicon
    writes in capitals (capital_citation). Each stands after the left context of a Russian row of
    TRAIN, chosen by hash, with the keep label; with `lowercase_contrast` the same keys in lower case
    stand in the same frame with the convert label, so case is what the pair tells apart.
    """
    budgets = {int(length): int(count) for length, count in cast(dict[str, int], options["words_by_length"]).items()}
    weight = float(cast(float, options["sample_weight"]))
    contrast = options.get("lowercase_contrast") is True
    contexts = natural_mixed_contexts(source_rows)[1]
    if not contexts or not any(budgets.values()):
        return [], {"words": 0, "words_by_length": {}, "scope": "not used"}
    models = reference_models(False)
    candidates: dict[int, list[tuple[str, str]]] = defaultdict(list)
    for word in models[1].frequencies:
        if len(word) not in budgets or not all("а" <= char <= "я" or char == "ё" for char in word):
            continue
        try:
            latin = translated(word.upper(), 1)
        except ValueError:
            continue
        if not capital_citation(latin, word.upper()):
            continue
        if refused & (expanded_aliases(word) | expanded_aliases(latin)):
            continue
        candidates[len(word)].append((word, latin))
    triggers: tuple[CorrectionTrigger, ...] = ("space", "space", "enter", "punctuation", "tab", "pause")
    applications = ("Telegram", "Code", "chrome", "UnseenEditor")
    rows: list[ActionRow] = []
    chosen: dict[str, list[str]] = {}
    for length, budget in sorted(budgets.items()):
        ranked = sorted(candidates[length], key=lambda pair: hashlib.sha256(("capital-citation:" + pair[0]).encode()).digest())
        chosen[str(length)] = [f"{word}/{latin}" for word, latin in ranked[:budget]]
        for word, latin in ranked[:budget]:
            identifier = "capital-citation:" + word
            trigger = triggers[variant_choice(identifier, "trigger", len(triggers))]
            boundary_text = ""
            if trigger == "space":
                boundary_text = " "
            elif trigger == "punctuation":
                boundary_text = CAPITAL_CITATION_PUNCTUATION[variant_choice(identifier, "punctuation", len(CAPITAL_CITATION_PUNCTUATION))]
            elif trigger in {"enter", "tab"} and variant_choice(identifier, "boundary-event", BOUNDARY_EVENT_CHOICES):
                boundary_text = "\n" if trigger == "enter" else "\t"
            field = FieldContext(applications[variant_choice(identifier, "application", len(applications))], "public-training",
                                 contexts[variant_choice(identifier, "context", len(contexts))], "", "unknown")
            rows.append(ActionRow(identifier, latin, 0, field, trigger, "", "keep", "mixed_language_insertion",
                                  boundary_text, weight))
            if contrast:
                # The same keys in lower case are the Russian word typed in the wrong layout and convert:
                # without this pair the keep frames taught that any Latin token after Russian prose stays
                # (the first candidate missed `f` for `а` and `lkz` for `для` in the owner's typing).
                rows.append(ActionRow(identifier + ":lower", latin.lower(), 0, field, trigger, "", "convert",
                                      "mixed_language_layout_intervention", boundary_text, weight))
    return rows, {"words": sum(len(words) for words in chosen.values()), "frames": len(rows),
                  "lowercase_contrast": contrast, "words_by_length": chosen,
                  "candidates_by_length": {str(length): len(candidates[length]) for length in sorted(budgets)},
                  "sample_weight": weight,
                  "scope": "TRAIN only: keep frames of Latin capitals after Russian prose whose Cyrillic reading is a rare lexicon word (capital_citation_curriculum)."}


def dotted_abbreviation_curriculum(source_rows: Sequence[CorpusRow], refused: frozenset[str],
                                   options: Mapping[str, object]) -> tuple[list[ActionRow], dict[str, object]]:
    """Russian initials and dotted abbreviations keep; their keys typed in the Latin layout convert.

    The Russian period is the slash key of the Latin layout, so `Р.Ф.` reads `H/A/`, the shape of a
    path typed in the Russian layout (`.ыкс.` is `/src/`), and natural TRAIN holds almost no Russian
    token of single letters and periods (corpus v26: one, against 27 English initials). The corpus
    v26 candidate converted `Р.Ф.` after Russian prose and alone (calibration v26: three of its ten
    false conversions). Each form - the dotted abbreviations of DOTTED_RUSSIAN_ABBREVIATIONS and
    `initials` pairs of RUSSIAN_INITIAL_LETTERS drawn by hash - stands after the left context of
    `contexts` Russian TRAIN rows and alone, labelled keep, and its Latin keys after the same contexts,
    labelled convert. A form held by this corpus's test or any accessed test is refused by its aliases.
    """
    weight = float(cast(float, options["sample_weight"]))
    count = int(cast(int, options["contexts"]))
    contexts = natural_mixed_contexts(source_rows)[1]
    pairs = [first + "." + second + "." for first in RUSSIAN_INITIAL_LETTERS for second in RUSSIAN_INITIAL_LETTERS]
    pairs.sort(key=lambda form: hashlib.sha256(("dotted-initials:" + form).encode()).digest())
    forms = [*DOTTED_RUSSIAN_ABBREVIATIONS, *pairs[:int(cast(int, options["initials"]))]]
    if not contexts or not count:
        return [], {"forms": 0, "scope": "not used"}
    applications = ("Telegram", "Code", "chrome", "UnseenEditor")
    rows: list[ActionRow] = []
    kept: list[str] = []
    for form in forms:
        latin = translated(form, 1)
        if refused & (expanded_aliases(form) | expanded_aliases(latin)):
            continue
        kept.append(f"{form}/{latin}")
        for variant in range(count):
            identifier = f"dotted:{form}:{variant}"
            application = applications[variant_choice(identifier, "application", len(applications))]
            before = contexts[variant_choice(identifier, "context", len(contexts))]
            field = FieldContext(application, "public-training", before, "", "unknown")
            rows.append(ActionRow(identifier + ":keep", form, 1, field, "space", "", "keep", "natural_surface", " ", weight))
            rows.append(ActionRow(identifier + ":wrong", latin, 0, field, "space", "", "convert", "layout_intervention",
                                  " ", weight))
        alone = FieldContext(applications[variant_choice(form, "application", len(applications))], "public-training", "", "", "unknown")
        rows.append(ActionRow(f"dotted:{form}:alone", form, 1, alone, "space", "", "keep", "natural_surface", " ", weight))
    return rows, {"forms": len(kept), "frames": len(rows), "words": kept, "sample_weight": weight,
                  "scope": "TRAIN only: Russian dotted abbreviations and initials keep after Russian prose and alone; their Latin keys convert after Russian prose."}


def historical_curriculum(intent: LinearNgramModel | None = None) -> list[ActionRow]:
    """Reuse distinct old TRAIN situations, never its evaluation labels.

    A physical family identifies the split, not the language decision. Keep
    its different labels, neighbours, applications and field roles. Remove
    only repeated identical frames. Completed-word lookahead mirrors the
    engine's second decision; the original longer field context also remains.
    """
    from train_context_model import build_corpus

    if intent is None:
        intent = LinearNgramModel.load(ROOT / "src/keyswitch/resources/models/layout_intent_v1.ksm")
    models = reference_models(True)
    status = IntentModelStatus(True, ROOT / "src/keyswitch/resources/models/layout_intent_v1.ksm",
                               intent.model_version, intent.checksum, None)

    def load(locale: str, extra_words: Iterable[str] = ()) -> LanguageModel:
        # The reference lexicons already carry the packaged supplements the engine
        # would pass here (build_corpus: LanguageModel.load(locale, supplement_words(locale))).
        return models[0 if locale == "en_US" else 1]

    with patch("train_context_model.LinearNgramModel.try_load_default", return_value=(intent, status)), \
            patch("train_context_model.LanguageModel.load", side_effect=load):
        curriculum = build_corpus()
    result: list[ActionRow] = []
    selected: set[bytes] = set()
    parents: dict[str, tuple[str, str, str]] = {}
    variants: Counter[tuple[tuple[str, str, str], ContextAction]] = Counter()
    actions: dict[tuple[str, str, str], set[ContextAction]] = {}
    for row in curriculum:
        if row.split != "train":
            continue
        item = row.evidence
        fields = [item.field]
        if item.field.after:
            following = WORDS.match(item.field.after.lstrip())
            if following is not None:
                completed = following.group()
                if completed != item.field.after:
                    fields.append(replace(item.field, after=completed))
        boundaries = {"space": (" ",), "pause": ("",), "enter": ("", "\n"),
                      "tab": ("", "\t"), "punctuation": (".",)}
        if item.trigger not in boundaries:
            raise ValueError("unsupported historical action trigger")
        for field in fields:
            for boundary in boundaries[item.trigger]:
                key = canonical((row.family, row.category, row.action, item.original,
                                 item.source_group, field.before, field.after,
                                 field.application, field.role, item.trigger, boundary))
                if key in selected:
                    continue
                selected.add(key)
                identifier = "legacy-train:" + hashlib.sha256(key).hexdigest()
                parent = row.family, row.category, item.trigger
                parents[identifier] = parent
                action: ContextAction = row.action
                if (action == "keep" and deferred_isolated(item.original, item.alternative, item.source_group)
                        and not WORDS.search(field.before) and not WORDS.search(field.after)):
                    # The shared isolated-short policy: a correct reading with no
                    # neighbouring word has no observable intent label either, so
                    # natural and historical frames of one observable situation
                    # take the same text-preserving deferred action.
                    action = "suggest" if item.trigger in ("enter", "tab", "punctuation") else "wait"
                variants[parent, action] += 1
                actions.setdefault(parent, set()).add(action)
                result.append(ActionRow(
                    identifier,
                    item.original, item.source_group, field,
                    cast(CorrectionTrigger, item.trigger), "", action, "legacy_" + row.category,
                    boundary, parent_family=row.family,
                ))
    # Keep the fixed parent budget and equal mass for each represented action.
    # Adding applications to one action must not dilute another action.
    return [replace(row, sample_weight=1.0 / (
        len(actions[parents[row.identifier]]) * variants[parents[row.identifier], row.action]
    )) for row in result]


def captured_curriculum(options: Mapping[str, object]) -> tuple[list[ActionRow], dict[str, object]]:
    """Questions the engine asked while public mixed text was typed the way a person types it.

    model/context_v1/captured holds them (tools/mixed_typing.py capture, pinned by SHA-256 in
    its manifest): ru.stackoverflow messages with their English terms and code lines, Tatoeba
    chat and the English sentence started in the Russian layout, correctly typed UD Taiga - each
    token typed in its own layout, the switch at a language boundary sometimes forgotten, the
    message sometimes begun in the wrong layout. context-v1 learned from them since 0.35.0; the
    action model only ever saw the scenario families, and on the owner's own typing (field logs
    of 0.31-0.36, replayed through 0.37.0) it left Russian words typed in the English layout after
    an inline English term (`git nfr` for `git так`) and English terms typed in the Russian layout
    amid Russian prose about twice as often as context-v1 did. TRAIN only: the files hold the
    train parts of their sources.

    The same isolated-short policy applies as to every other frame: a token with no word on
    either side, short enough to have no observable intent, takes the deferred action whatever
    the typing meant. Inside-word questions are left out (the action features have no inside
    flag), and a planned frame of a word longer than the planned context allows is asked with
    the field origin, as the engine asks it (KeySwitchEngine._neighbour_origin).
    """
    from train_context_model import captured_rows, captured_sources

    budget = int(cast(int, options["maximum_rows_per_source"]))
    # None keeps each file's own share of convert questions.
    convert_share = None if options["convert_share"] is None else float(cast(float, options["convert_share"]))
    keep_unknown_share = float(cast(float, options["keep_unknown_share"]))
    unknown_keep_groups = frozenset(cast(list[int], options["unknown_keep_groups"]))
    weight_scale = float(cast(float, options["weight_scale"]))
    minimum_characters = int(cast(int, options["minimum_word_characters"]))
    excluded = tuple(cast(list[str], options["excluded_file_markers"]))
    applications = ("Telegram", "Code", "chrome", "UnseenEditor")
    result: list[ActionRow] = []
    report: dict[str, object] = {}
    for source in captured_sources(ROOT / str(options["manifest"])):
        name = source.path.name
        if any(marker in name for marker in excluded):
            report[name] = {"excluded": True}
            continue
        ranked: dict[ContextAction, list[tuple[bytes, ContextEvidence]]] = {"keep": [], "convert": []}
        skipped_short = 0
        for item, label, _count in captured_rows(source.path):
            action = ACTIONS[label]
            if action not in ranked:
                continue
            if len(item.original) < minimum_characters:
                # A lone letter is left out, as the corpus leaves it out: in these files a lone
                # `b`, `c`, `d` or `f` is nearly always a Russian word typed in the English layout,
                # and a model taught so converted a capital letter in English prose.
                skipped_short += 1
                continue
            try:
                translated(item.original, item.source_group)
            except ValueError:
                continue
            key = hashlib.sha256(canonical(["captured", name, item.original, item.source_group, item.trigger,
                                            item.literal_tail, item.boundary_text, item.after_origin,
                                            item.field.before, item.field.after, item.field.role])).digest()
            ranked[action].append((key, item))
        available = len(ranked["convert"]) + len(ranked["keep"])
        share = convert_share if convert_share is not None else len(ranked["convert"]) / max(1, available)
        converts = sorted(ranked["convert"], key=lambda entry: entry[0])[:int(budget * share)]
        # Correctly typed words no lexicon knows - slang, names, brands, abbreviations, English
        # terms typed in the English layout - are the keep answers a uniform sample nearly never
        # draws, and exactly the ones both model lines turned into gibberish of the other layout
        # (an invented `шупшуп` became `iegieg` at p=0.994). Those of the layouts the recipe names
        # get their own share of the keeps; without the Latin ones, a month name and a capital
        # abbreviation in English prose were converted.
        keep_budget = budget - len(converts)
        ordered_keeps = sorted(ranked["keep"], key=lambda entry: entry[0])
        unknown = [entry for entry in ordered_keeps if entry[1].source_group in unknown_keep_groups
                   and not plausible_reading(entry[1].original, entry[1].source_group)]
        chosen_unknown = unknown[:int(keep_budget * keep_unknown_share)]
        taken = {entry[0] for entry in chosen_unknown}
        keeps = chosen_unknown + [entry for entry in ordered_keeps if entry[0] not in taken][:keep_budget - len(chosen_unknown)]
        counts: Counter[str] = Counter()
        for action, chosen in (("convert", converts), ("keep", keeps)):
            for key, item in chosen:
                identifier = f"captured:{source.path.stem}:{key.hex()}"
                application = applications[variant_choice(identifier, "application", len(applications))]
                field = FieldContext(application, "captured", item.field.before, item.field.after, item.field.role)
                origin = item.after_origin
                if origin == "planned_next_conversion" and len(item.original) > PLANNED_CONTEXT_WORD_MAX_CHARACTERS:
                    origin = "field"
                if origin == "planned_next_conversion" and WORDS.match(item.field.after.lstrip()) is None:
                    # The engine plans a next word only once one was typed; a planned
                    # question without a leading word cannot have been asked.
                    counts["skipped_planned_without_word"] += 1
                    continue
                labelled: ContextAction = action
                curated_letter = (len(item.original) == 1 and len(item.alternative) == 1
                                  and (item.original.casefold() in TRUSTED_SINGLE_LETTER_WORDS
                                       or item.alternative.casefold() in TRUSTED_SINGLE_LETTER_WORDS))
                if (origin != "planned_next_conversion" and not curated_letter
                        and not WORDS.search(item.field.before) and not WORDS.search(item.field.after)
                        and deferred_isolated(item.original, item.alternative, item.source_group)):
                    labelled = "suggest" if item.trigger in ("enter", "tab", "punctuation") else "wait"
                counts[labelled] += 1
                result.append(ActionRow(identifier, item.original, item.source_group, field,
                                        cast(CorrectionTrigger, item.trigger), item.literal_tail, labelled,
                                        "captured_" + source.path.name.split(".", 1)[0].replace("-", "_"),
                                        item.boundary_text, source.weight * weight_scale, origin))
        report[name] = {"available": {action: len(rows) for action, rows in ranked.items()}, "skipped_short": skipped_short,
                        "unknown_keep_available": len(unknown), "unknown_keep_chosen": len(chosen_unknown),
                        "chosen": dict(counts), "weight": source.weight * weight_scale}
    return result, report


def legacy_lookahead_rows(
    rows: Sequence[ActionRow], detector: LanguageDetector, ortho: OrthoModel | None, *,
    profile: str, maximum_families: int, seeds_per_family: int,
) -> tuple[list[ActionRow], dict[str, object]]:
    """Replace bounded old TRAIN frames with equal-mass planned variants."""
    selected = {row.identifier: row for row in rows
                if row.category.startswith("legacy_") and lookahead_focus(row.original, row.group)
                and row.trigger == "space" and row.boundary_text == " " and not row.literal_tail
                and not row.field.sensitive and not row.field.selection and row.field.role != "password"}
    anchors: dict[tuple[str, int], LookaheadAnchor] = {}
    for row in sorted(rows, key=lambda item: item.identifier):
        if not row.category.startswith("legacy_"):
            continue
        match = WORDS.match(row.field.after.lstrip())
        if match is None:
            continue
        text = match.group()
        if not LOOKAHEAD_ANCHOR_MIN_CHARACTERS <= len(text) <= LOOKAHEAD_ANCHOR_MAX_CHARACTERS or not text.isalpha():
            continue
        group = 1 if any("а" <= char.casefold() <= "я" or char.casefold() == "ё" for char in text) else 0
        try:
            translated(text, group)
        except ValueError:
            continue
        anchors.setdefault((text, group), LookaheadAnchor(
            row.identifier + ":first-after", "legacy-after:" + hashlib.sha256(physical(text).encode()).hexdigest(), text, group))
    seeds = [LookaheadSeed(row.identifier, row.parent_family, evidence(row, detector, ortho),
                          row.action, row.category, row.sample_weight) for row in selected.values()]
    curriculum = build_lookahead_curriculum(seeds, list(anchors.values()), detector,
                                          profile=profile, maximum_families=maximum_families,
                                          seeds_per_family=seeds_per_family)
    result = [row for row in rows if row.identifier not in selected]
    for frame in curriculum.frames:
        row = selected[frame.source_identifier]
        identifier = row.identifier if frame.kind == "original" else row.identifier + ":planned:" + frame.anchor_identifier
        result.append(replace(row, identifier=identifier, field=frame.evidence.field,
                              sample_weight=frame.sample_weight, after_origin=frame.evidence.after_origin))
    return result, {"counts": curriculum.counts, "mass_by_origin_action": curriculum.mass_by_origin_action,
                    "input_mass": math.fsum(row.sample_weight for row in rows),
                    "output_mass": math.fsum(row.sample_weight for row in result),
                    "scope": "Old TRAIN annotations and first words of their existing right contexts; no new split or candidate context-model labels."}


def natural_lookahead_rows(
    rows: Sequence[ActionRow], source_rows: Sequence[CorpusRow], detector: LanguageDetector,
    ortho: OrthoModel | None, *, profile: str, split: str, maximum_families: int, seeds_per_family: int,
) -> tuple[list[ActionRow], dict[str, object]]:
    """Replace bounded natural short-space frames with equal-mass original/planned variants.

    A natural row knows its own continuation (the corpus keeps the sentence), so a
    deferrable short word typed in the wrong layout can be paired with the word
    that follows it, exactly as the engine's planned lookahead sees it, including
    at the start of a field where the left context is empty. Anchors are the first
    words of the same split's own right contexts; nothing crosses a split.
    """
    if any(row.split != split for row in source_rows):
        raise ValueError("natural lookahead requires rows of the declared split")
    by_identifier = {row.identifier: row for row in source_rows}
    selected: dict[str, tuple[ActionRow, str]] = {}
    for row in rows:
        if (row.category not in ("layout_intervention", "natural_surface") or not lookahead_focus(row.original, row.group)
                or row.trigger != "space" or row.boundary_text != " " or row.literal_tail
                or row.field.sensitive or row.field.selection or row.field.role == "password" or row.field.after):
            continue
        source = by_identifier.get(row.identifier.rsplit(":", IDENTIFIER_SUFFIX_SEGMENTS)[0])
        if source is None:
            continue
        if row.category == "layout_intervention":
            selected[row.identifier] = row, source.after
        elif row.field.before.strip():
            selected[row.identifier] = row, ""
    anchors = natural_anchors(source_rows, split)
    seeds = [LookaheadSeed(row.identifier, "natural:" + by_identifier[row.identifier.rsplit(":", IDENTIFIER_SUFFIX_SEGMENTS)[0]].document,
                          evidence(row, detector, ortho),
                          row.action, "natural_short_lookahead" if row.category == "layout_intervention" else row.category,
                          row.sample_weight, split, next_words,
                          "convert" if row.category == "layout_intervention" else "keep")
             for row, next_words in selected.values()]
    curriculum = build_lookahead_curriculum(seeds, anchors, detector, profile=profile,
                                          maximum_families=maximum_families, seeds_per_family=seeds_per_family, split=split)
    result = [row for row in rows if row.identifier not in selected]
    for frame in curriculum.frames:
        row = selected[frame.source_identifier][0]
        identifier = row.identifier if frame.kind == "original" else row.identifier + ":planned:" + frame.anchor_identifier
        result.append(replace(row, identifier=identifier, field=frame.evidence.field, action=frame.action,
                              sample_weight=frame.sample_weight, after_origin=frame.evidence.after_origin))
    return result, {"counts": curriculum.counts, "mass_by_origin_action": curriculum.mass_by_origin_action,
                    "input_mass": math.fsum(row.sample_weight for row in rows),
                    "output_mass": math.fsum(row.sample_weight for row in result),
                    "scope": "Natural short typing frames of one split paired with the first word of their own sentence's continuation; no cross-split anchors and no candidate context-model labels."}


def natural_anchors(source_rows: Sequence[CorpusRow], split: str) -> list[LookaheadAnchor]:
    """The first words of a split's own right contexts, one anchor per word and language."""
    anchors: dict[tuple[str, int], LookaheadAnchor] = {}
    for source in sorted(source_rows, key=lambda item: item.identifier):
        match = WORDS.match(source.after.lstrip())
        if match is None:
            continue
        text = match.group()
        if not LOOKAHEAD_ANCHOR_MIN_CHARACTERS <= len(text) <= LOOKAHEAD_ANCHOR_MAX_CHARACTERS or not text.isalpha():
            continue
        group = 1 if any("а" <= char.casefold() <= "я" or char.casefold() == "ё" for char in text) else 0
        try:
            translated(text, group)
        except ValueError:
            continue
        anchors.setdefault((text, group), LookaheadAnchor(
            source.identifier + ":first-after", "natural-after:" + hashlib.sha256(physical(text).encode()).hexdigest(),
            text, group, split))
    return list(anchors.values())


def refused_aliases(corpus: Path) -> frozenset[str]:
    """Aliases of every word a sealed test holds: this corpus's test and every accessed one."""
    membership = cast(dict[str, object], json.loads((corpus / "test-membership.json").read_bytes()))
    aliases = set(cast(list[str], membership.get("alias_sha256", [])))
    ledger, _ = ledger_test_aliases(LEDGER_ROOT)
    return frozenset(aliases | ledger)


def lexical_short_pairs(refused: frozenset[str]) -> list[tuple[str, str, int]]:
    """Three-letter words of the pinned lexicons whose other reading is a word or a command too.

    Both readings must be real: a form of the onboard lexicon (the OpenSubtitles supplement
    marks a form as known for the deferral rule, but it also holds one-off noise, and a
    conversion into such a form is never taught) or an entry of the shipped identifier
    index. A pair whose word is held by a sealed test stays out, by the same aliases the
    fitting extension refuses.
    """
    models = reference_models(False)
    supplement = {0: frozenset(supplement_words("en_US")), 1: frozenset(supplement_words("ru_RU"))}

    def real(text: str, group: int) -> bool:
        return (text not in supplement[group] and models[group].score(text).known) or (
            plausible_reading(text, group) and not models[group].score(text).known)

    pairs: list[tuple[str, str, int]] = []
    seen: set[str] = set()
    for group in (0, 1):
        for word in sorted(models[group].frequencies):
            if not ACTION_SHORT_WORD_MAX_CHARACTERS < len(word) <= ACTION_DEFERRED_WORD_MAX_CHARACTERS or not word.isalpha():
                continue
            if word in supplement[group]:
                continue
            try:
                alternate = translated(word, group)
            except ValueError:
                continue
            if not real(alternate, 1 - group) or not deferred_isolated(word, alternate, group):
                continue
            if refused & (expanded_aliases(word) | expanded_aliases(alternate)):
                continue
            # Both lexicons name the same physical pair; it enters once, in the reading met first.
            if physical(word) in seen:
                continue
            seen.add(physical(word))
            pairs.append((word, alternate, group))
    return pairs


def lexical_short_pair_rows(
    rows: Sequence[ActionRow], source_rows: Sequence[CorpusRow], detector: LanguageDetector, ortho: OrthoModel | None,
    *, refused: frozenset[str], profile: str, split: str, maximum_families: int, seeds_per_family: int,
) -> tuple[list[ActionRow], dict[str, object]]:
    """Teach a deferred three-letter word to follow its converted neighbour.

    Natural text rarely holds a word of three letters whose other reading is a word too, so
    the corpus v13 candidate had no planned frame with a convert label for such a word in the
    Latin direction and kept `tot` after `привет` was converted. Each lexicon pair stands
    alone in both of its readings with the deferred label, and each reading gets planned
    variants with words of the split's own right contexts, labelled convert: with a
    neighbour converted to the other language the pair is that language, which is the
    decision the engine asks the model for. The frames carry one unit of mass per reading.
    """
    pairs = lexical_short_pairs(refused)
    applications = ("Telegram", "Code", "chrome", "UnseenEditor")
    seeds: list[LookaheadSeed] = []
    originals: dict[str, ActionRow] = {}
    for word, alternate, group in pairs:
        family = "lexical:" + hashlib.sha256(physical(word).encode()).hexdigest()
        for member, member_group in ((word, group), (alternate, 1 - group)):
            for variant in range(LEXICAL_PAIR_ANCHOR_VARIANTS):
                identifier = f"lexical:{member_group}:{member}:{variant}"
                application = applications[variant_choice(identifier, "application", len(applications))]
                row = ActionRow(identifier, member, member_group, FieldContext(application, "public-training", "", "", "unknown"),
                                "space", "", "wait", "lexical_short_pair", " ", 1.0 / LEXICAL_PAIR_ANCHOR_VARIANTS)
                originals[identifier] = row
                seeds.append(LookaheadSeed(identifier, family, evidence(row, detector, ortho), "wait", "lexical_short_pair",
                                           row.sample_weight, split, "", "convert"))
    curriculum = build_lookahead_curriculum(seeds, natural_anchors(source_rows, split), detector, profile=profile,
                                          maximum_families=maximum_families, seeds_per_family=seeds_per_family, split=split)
    result = list(rows)
    for frame in curriculum.frames:
        row = originals[frame.source_identifier]
        identifier = row.identifier if frame.kind == "original" else row.identifier + ":planned:" + frame.anchor_identifier
        result.append(replace(row, identifier=identifier, field=frame.evidence.field, action=frame.action,
                              sample_weight=frame.sample_weight, after_origin=frame.evidence.after_origin))
    return result, {"pairs": len(pairs), "pairs_by_direction": {str(group): sum(1 for *_, g in pairs if g == group) for group in (0, 1)},
                    "words": sorted(f"{word}/{alternate}" for word, alternate, _ in pairs),
                    "counts": curriculum.counts, "mass_by_origin_action": curriculum.mass_by_origin_action,
                    "input_mass": math.fsum(row.sample_weight for row in rows),
                    "output_mass": math.fsum(row.sample_weight for row in result),
                    "scope": "TRAIN only: three-letter words of the pinned lexicons and the shipped identifier index whose other reading is a real word or command too, standing alone with the deferred label and with planned variants labelled convert; words of this corpus's test and of every accessed test are refused by their aliases."}


def balance_planned_mass(rows: Sequence[ActionRow]) -> tuple[list[ActionRow], dict[str, object]]:
    """Give planned lookahead frames the mass of the situation they resolve.

    A deferrable short word at a space boundary has two outcomes in the corpus:
    with no word on either side the corpus defers (wait/suggest), and with the
    planned next word known the declared intervention applies. Both are the same
    physical moment seen at two times, so per direction and focus length the
    planned frames receive exactly the total mass of the isolated deferred frames.
    Legacy parent budgets alone left the planned regime at about one hundredth of
    that mass, and the model could not learn the after-context distinction.
    """
    isolated: dict[tuple[int, int], float] = defaultdict(float)
    planned: dict[tuple[int, int], float] = defaultdict(float)
    for row in rows:
        if not lookahead_focus(row.original, row.group):
            continue
        key = (row.group, len(row.original))
        if row.after_origin == "planned_next_conversion":
            planned[key] += row.sample_weight
        elif (row.after_origin == "none" and row.action in ("wait", "suggest")
              and not WORDS.search(row.field.before) and not WORDS.search(row.field.after)):
            isolated[key] += row.sample_weight
    scale = {key: isolated[key] / planned[key] for key in planned if planned[key] > 0 and isolated.get(key, 0.0) > 0}
    result = []
    for row in rows:
        key = (row.group, len(row.original))
        if row.after_origin == "planned_next_conversion" and key in scale:
            row = replace(row, sample_weight=row.sample_weight * scale[key])
        result.append(row)
    report = {"policy": "planned frames per (direction, focus length) carry the total mass of the isolated deferred frames of the same class",
              "isolated_mass": {f"{group}:{length}": round(value, MASS_REPORT_DECIMALS) for (group, length), value in sorted(isolated.items())},
              "planned_mass_before": {f"{group}:{length}": round(value, MASS_REPORT_DECIMALS) for (group, length), value in sorted(planned.items())},
              "scale": {f"{group}:{length}": round(value, MASS_REPORT_DECIMALS) for (group, length), value in sorted(scale.items())},
              "input_mass": math.fsum(row.sample_weight for row in rows), "output_mass": math.fsum(row.sample_weight for row in result)}
    return result, report


def previous_context(row: ActionRow) -> tuple[dict[int, str], int | None]:
    """Estimate the last completed natural word for a frozen phrase frame.

    This reconstruction is not an event replay. The separate evaluator types
    the complete phrase to check actual token boundaries and remembered layout.
    """
    matches = WORDS.findall(row.field.before)
    if not matches or not row.field.application.strip():
        return {}, None
    word = matches[-1]
    group = 1 if any("а" <= c.casefold() <= "я" or c.casefold() == "ё" for c in word) else 0
    try:
        alternative = translated(word, group)
    except ValueError:
        # A clipped mixed-script or curly-apostrophe word is not a known pair.
        return {}, None
    return {group: word, 1 - group: alternative}, group


BLIND_IDENTIFIERS = IdentifierLexicon(frozenset(), "none", "identifiers-none")


def identifier_family(identifier: str) -> str:
    """The unit that shares one identifier-evidence dropout decision: a command with all its
    contexts and spelling variants, otherwise the row itself."""
    parts = identifier.split(":")
    if len(parts) >= COMMAND_FAMILY_IDENTIFIER_PARTS and parts[1] == "command":
        return ":".join(parts[:COMMAND_FAMILY_IDENTIFIER_PARTS])
    return identifier


def identifier_evidence_dropped(identifier: str) -> bool:
    """Deterministic: one family in IDENTIFIER_DROPOUT_FAMILIES is trained without the
    identifier lexicon, so command-like strings outside the shipped index still convert."""
    return variant_choice(identifier_family(identifier), "identifier-dropout", IDENTIFIER_DROPOUT_FAMILIES) == 0


def evidence(row: ActionRow, detector: LanguageDetector, ortho: OrthoModel | None,
             *, identifiers: IdentifierLexicon | None = None) -> ContextEvidence:
    alternative = translated(row.original, row.group)
    previous, context_group = previous_context(row)
    if row.after_origin == "planned_next_conversion":
        # The engine re-decides a waiting word with its converted next word as the
        # only language context (KeySwitchEngine._planned_baseline); the planned
        # frame's baseline verdict is built the same way, never from the left context.
        match = WORDS.match(row.field.after.lstrip())
        if match is None:
            raise ValueError("planned frame without a next word")
        previous, context_group = {1 - row.group: match.group()}, 1 - row.group
    decision = automatic_word_decision(
        detector, row.original, {1 - row.group: alternative}, row.group,
        previous_words=previous, context_group=context_group, trigger=row.trigger,
    )
    return evidence_for_decision(
        decision, alternative, 1 - row.group, detector, row.field, row.trigger,
        literal_tail=row.literal_tail, ortho=ortho, boundary_text=row.boundary_text,
        after_origin=row.after_origin, identifiers=identifiers,
    )


def provenance() -> dict[str, str]:
    paths = (
        "tools/train_context_action_model.py", "tools/freeze_context_action_corpus.py",
        "tools/evaluate_context_action_sequences.py", "tools/context_optimizer.py",
        "tools/context_optimizer.c", "tools/context_evidence.py", "tools/reference_lexicon.py", "model/context_v3/recipe.json",
        "tools/action_epoch_selection.py", "tests/test_action_epoch_selection.py",
        "tools/context_lookahead_curriculum.py", "tests/test_context_lookahead_curriculum.py",
        "tools/context_deferral.py",
        "tools/train_context_model.py", "model/context_v1/captured/manifest.json",
        "tests/test_context_action_training.py", "tests/test_default_input_sequences.py",
        "tools/context_technical_corpus.py", "tools/merge_context_action_corpora.py",
        "tests/test_context_technical_corpus.py", "tests/test_context_action_merge.py",
        "tools/context_physical_keys.py", "tools/reconcile_context_action_corpus.py",
        "tests/test_language_intent_regressions.py", "tests/test_input_sequence_matrix.py",
        "tests/test_context_policy.py",
        "tools/train_context_model.py", "model/context_v1/scenarios.json",
        "src/keyswitch/context_action_features.py", "src/keyswitch/context_model.py",
        "src/keyswitch/context_policy.py", "src/keyswitch/word_decision.py",
        "src/keyswitch/short_words.py", "src/keyswitch/engine.py",
        "src/keyswitch/detector.py", "src/keyswitch/intent_model.py",
        "src/keyswitch/language_model.py", "src/keyswitch/layouts.py",
        "src/keyswitch/spellcheck.py", "src/keyswitch/identifier_lexicon.py",
        "src/keyswitch/resources/protected_tokens.txt", "src/keyswitch/resources/identifiers.json",
        "src/keyswitch/resources/lexicon-supplement-ru_RU.json",
        "src/keyswitch/ortho_model.py", "src/keyswitch/input_context.py",
        "src/keyswitch/resources/models/layout_intent_v1.ksm",
        "model/context_v3/baseline-context-v1.json",
        "src/keyswitch/resources/models/ortho_v1.json", "model/intent_v1/config.json",
        "model/intent_v1/sources/en_US.lm", "model/intent_v1/sources/ru_RU.lm",
        "model/intent_v1/sources/hunspell/en_US.dic", "model/intent_v1/sources/hunspell/en_US.aff",
        "model/intent_v1/sources/hunspell/ru_RU.dic", "model/intent_v1/sources/hunspell/ru_RU.aff",
    )
    # Span frames use the real engine and editor harness. Every runtime input
    # must be fixed before fitting, including auxiliary packaged models.
    result = runtime_provenance()
    result.update({path: checksum(ROOT / path) for path in paths})
    result.update({path: checksum(ROOT / path) for path in (
        "tools/context_action_spans.py", "tests/test_context_action_spans.py",
    )})
    return result


def metrics(probabilities: array[float], labels: array[int], threshold: float) -> dict[str, int | float]:
    false = true = possible = correct = 0
    for index, label in enumerate(labels):
        scores = probabilities[index * len(ACTIONS):index * len(ACTIONS) + len(ACTIONS)]
        predicted = max(range(len(ACTIONS)), key=scores.__getitem__)
        converted = predicted == 1 and scores[1] >= threshold
        false += int(converted and label != 1)
        true += int(converted and label == 1)
        possible += int(label == 1)
        correct += int(predicted == label)
    return {"rows": len(labels), "false_conversions": false, "converted_correctly": true,
            "convert_rows": possible, "conversion_recall": true / possible if possible else 0.0,
            "correct_classes": correct}


def choose_threshold(
    predictions: dict[str, tuple[array[float], array[int]]], candidates: list[float],
    minimum_net_benefit: int, minimum_recall: float, *, minimum_threshold: float = 0.0,
    net_benefit_tolerance: float = 0.0, maximum_threshold: float = 1.0,
    authored_floor: float = 0.0,
) -> tuple[float, dict[str, object], bool]:
    """Choose the serving threshold on calibration by what it nets, in both profiles.

    ``minimum_threshold`` is the development operating threshold of the selected
    epoch: the balance below it was already worse on development, so it is never a
    serving candidate, whatever calibration says about it.

    Until 17.09.2026 this took the lowest threshold with zero false conversions. That
    rule only held because class-wide vetoes removed the rows that convert falsely;
    with the model deciding, no threshold below 1.0 reaches zero and the rule chose a
    model that converts nothing. A threshold is now judged by repairs minus breakages,
    every profile must be ahead by ``minimum_net_benefit``, and among the qualifying
    thresholds the best balance wins, and among equals the one that converts least
    falsely - the lowest such threshold on a tie.

    ``net_benefit_tolerance`` reads that balance as the plateau it is rather than a
    single point. Measured on corpus v12 (18.09.2026), raising the threshold from 0.95
    to 0.995 gives up 0.8 % of the correct conversions and removes two thirds of the
    false ones: the two sides are not equally sensitive, so the highest-netting point
    is not the safest point of nearly equal value. Within the tolerance the threshold
    with the fewest false conversions wins, the lowest such threshold on a tie. The
    balance still decides which thresholds are admissible at all.

    ``authored_floor`` and ``maximum_threshold`` are the two ends of the same argument. Calibration counts
    rows; the product also has cases it has promised to handle, and a threshold above
    the confidence the model gives those is cautious about the wrong thing. The ceiling
    is read from them, never from a sealed test: on 18.09.2026 the tightest pinned case
    was `rjn` to `кот` at 0.994424, which a served threshold of 0.995 refused while the
    calibration counts showed no reason to go that high. The floor is the same reading
    from the other side - the pinned cases that must stay as typed, `лут` against the
    command `ken` at 0.9887 and `Вас` against `Dfc` at 0.9889 - and a threshold at or
    below those converts them. Calibration counts rows and cannot see either end.
    """
    if not predictions or not candidates:
        raise ValueError("calibration requires profiles and threshold candidates")
    threshold_floor = max(minimum_threshold, authored_floor)
    if not threshold_floor <= maximum_threshold <= 1.0:
        raise ValueError("threshold ceiling must not fall below the floor it has to clear")
    grid = [threshold for threshold in sorted(candidates)
            if threshold_floor <= threshold <= maximum_threshold]
    if not grid:
        raise ValueError("no threshold candidate reaches the development operating threshold")
    if not 0.0 <= net_benefit_tolerance < 1.0:
        raise ValueError("net benefit tolerance must be a fraction below one")
    qualifying: list[tuple[int, int, int, float, dict[str, object]]] = []
    report: dict[str, object] = {}
    for threshold in grid:
        by_profile = {name: metrics(values, labels, threshold)
                      for name, (values, labels) in predictions.items()}
        false = sum(int(row["false_conversions"]) for row in by_profile.values())
        true = sum(int(row["converted_correctly"]) for row in by_profile.values())
        possible = sum(int(row["convert_rows"]) for row in by_profile.values())
        per_profile_net = [int(row["converted_correctly"]) - int(row["false_conversions"])
                           for row in by_profile.values()]
        report = {"false_conversions": false,
            "conversion_recall": true / possible if possible else 0.0,
            "converted_correctly": true, "convert_rows": possible,
            "net_benefit": true - false, "minimum_net_benefit": min(per_profile_net),
            "rows": sum(int(row["rows"]) for row in by_profile.values()), "by_profile": by_profile}
        qualifies = (min(per_profile_net) >= minimum_net_benefit
                     and all(row["conversion_recall"] >= minimum_recall for row in by_profile.values()))
        if qualifies:
            qualifying.append((min(per_profile_net), true - false, false, threshold, report))
    if not qualifying:
        return grid[-1], report, False
    ceiling = max((minimum, net) for minimum, net, _false, _threshold, _report in qualifying)
    floor = (math.floor(ceiling[0] * (1.0 - net_benefit_tolerance)),
             math.floor(ceiling[1] * (1.0 - net_benefit_tolerance)))
    # Both parts of the balance have to hold: a tuple comparison admitted any threshold whose
    # worst profile matched the best, whatever its pooled net, and the tolerance never applied.
    admissible = [row for row in qualifying if row[0] >= floor[0] and row[1] >= floor[1]]
    _minimum, _net, _false, threshold, chosen = min(
        admissible, key=lambda row: (row[NET_BENEFIT_FALSE_INDEX], row[NET_BENEFIT_THRESHOLD_INDEX])
    )
    return threshold, {**chosen, "net_benefit_ceiling": ceiling[1], "net_benefit_floor": floor[1],
                       "admissible_thresholds": [row[NET_BENEFIT_THRESHOLD_INDEX] for row in admissible]}, True


def development_thresholds(options: Mapping[str, object]) -> list[float]:
    """The operating points an epoch may be selected for: those the serving band can hold.

    The development threshold of the selected epoch is the floor the calibration grid starts
    from, and the band's ceiling ends it. An epoch selected for a point above the ceiling
    had no servable threshold left, and the fit failed after its last epoch.
    """
    band = cast(dict[str, object], options["threshold_selection"])
    ceiling = float(cast(float, band["maximum_threshold"]))
    candidates = [float(value) for value in cast(list[float], options["threshold_candidates"]) if value <= ceiling]
    if not candidates:
        raise ValueError("no threshold candidate lies inside the serving band")
    return candidates


def runtime_masks(features: Iterable[tuple[dict[str, float], int, float]], model: ContextModel) -> tuple[list[bool], list[bool]]:
    """The runtime's shared support check and automatic-conversion policy, per row."""
    rows = [(model.supports_features(values), model.allows_automatic_conversion(values)) for values, _, _ in features]
    return [supported for supported, _ in rows], [automatic for _, automatic in rows]


def apply_runtime_support(
    probabilities: array[float], features: Iterable[tuple[dict[str, float], int, float]],
    model: ContextModel,
) -> array[float]:
    """Calibrate conversion counts after the runtime's shared support and policy checks."""
    supported, automatic = runtime_masks(features, model)
    return apply_support_mask(probabilities, supported, automatic)


def apply_support_mask(
    probabilities: array[float], supported: Sequence[bool], automatic: Sequence[bool] | None = None,
) -> array[float]:
    """Reuse the vocabulary-dependent support and policy decisions across fitting epochs.

    An unsupported row may neither keep nor convert; a row the runtime policy
    excludes from automatic conversion may not convert. Both become suggestions,
    exactly as ``ContextModel.predict`` reports them.
    """
    if automatic is None:
        automatic = [True] * len(supported)
    if len(supported) * len(ACTIONS) != len(probabilities) or len(automatic) != len(supported):
        raise ValueError("calibration feature and prediction counts differ")
    result = array("d", probabilities)
    for index, (available, allowed) in enumerate(zip(supported, automatic)):
        scores = result[index * len(ACTIONS):index * len(ACTIONS) + len(ACTIONS)]
        selected = max(range(len(ACTIONS)), key=scores.__getitem__)
        if (not available and selected in (0, 1)) or (not allowed and selected == 1):
            result[index * len(ACTIONS):index * len(ACTIONS) + len(ACTIONS)] = array("d", [0.0, 0.0, 0.0, 1.0])
    return result


def fit(corpus: Path, output: Path) -> dict[str, object]:
    if output.exists():
        raise ValueError("candidate directory already exists; never overwrite an experiment")
    options = cast(dict[str, object], json.loads(RECIPE.read_bytes()))
    if options.get("schema_version") != 1 or options.get("feature_version") != CONTEXT_ACTION_FEATURE_VERSION:
        raise ValueError("invalid action recipe")
    before_provenance = provenance()
    corpus_hash = checksum(corpus / "manifest.json")
    maximum = cast(dict[str, int], options["maximum_source_rows"])
    source_rows = {split: load_split(corpus, split) for split in FITTING_SPLITS}
    frames = {split: action_rows(select_rows(rows, maximum[split])) for split, rows in source_rows.items()}
    if any(not rows for rows in frames.values()):
        raise ValueError("empty fitting split")
    intent = LinearNgramModel.load(ROOT / "src/keyswitch/resources/models/layout_intent_v1.ksm")
    ortho = OrthoModel.load(ROOT / "src/keyswitch/resources/models/ortho_v1.json")
    profiles = cast(list[str], options["profiles"])
    lexical_models = {name: reference_models(name == "reference_hunspell") for name in profiles}
    detectors = {name: LanguageDetector(lexical_models[name], intent) for name in profiles}
    output.mkdir(parents=True)
    (output / "recipe.json").write_bytes(canonical(options))
    captured, captured_report = captured_curriculum(cast(dict[str, object], options["captured_curriculum"]))
    refused = refused_aliases(corpus)
    capitals, capital_report = capital_citation_curriculum(
        source_rows["train"], refused, cast(dict[str, object], options["capital_citation_curriculum"]))
    dotted, dotted_report = dotted_abbreviation_curriculum(
        source_rows["train"], refused, cast(dict[str, object], options["dotted_abbreviation_curriculum"]))
    frames["train"] = training_order([*frames["train"], *historical_curriculum(intent), *captured, *capitals, *dotted])
    feature_paths: dict[tuple[str, str], Path] = {}
    feature_mass = FeatureMass()
    span_budgets = cast(dict[str, int], options["span_maximum_families"])
    span_reports: dict[str, dict[str, dict[str, int]]] = {}
    lookahead_reports: dict[str, dict[str, object]] = {}
    lookahead_options = cast(dict[str, int], options["lookahead_curriculum"])
    natural_options = cast(dict[str, int], options["natural_lookahead_curriculum"])
    natural_reports: dict[str, dict[str, dict[str, object]]] = {}
    lexical_options = cast(dict[str, int], options["lexical_short_pair_curriculum"])
    lexical_reports: dict[str, dict[str, object]] = {}
    balance_reports: dict[str, dict[str, object]] = {}
    for profile, detector in detectors.items():
        span_reports[profile] = {}
        natural_reports[profile] = {}
        for split, rows in frames.items():
            if split == "train":
                rows, lookahead_reports[profile] = legacy_lookahead_rows(
                    rows, detector, ortho, profile=profile,
                    maximum_families=lookahead_options["maximum_families"],
                    seeds_per_family=lookahead_options["seeds_per_family"])
            rows, natural_reports[profile][split] = natural_lookahead_rows(
                rows, source_rows[split], detector, ortho, profile=profile, split=split,
                maximum_families=natural_options["maximum_families"],
                seeds_per_family=natural_options["seeds_per_family"])
            if split == "train":
                rows, lexical_reports[profile] = lexical_short_pair_rows(
                    rows, source_rows[split], detector, ortho, refused=refused, profile=profile, split=split,
                    maximum_families=lexical_options["maximum_families"], seeds_per_family=lexical_options["seeds_per_family"])
                rows, balance_reports[profile] = balance_planned_mass(rows)
            spans = build_span_curriculum(source_rows[split], lexical_models[profile], profile=profile,
                                          maximum_families=span_budgets[split], expected_split=split)
            span_reports[profile][split] = spans.counts
            prepared: list[tuple[str, ActionRow | SpanFrame]] = [(row.identifier, row) for row in rows]
            prepared.extend((f"span:{row.sequence_id}:{index}", row) for index, row in enumerate(spans.frames))
            if split == "train":
                prepared.sort(key=lambda entry: (hashlib.sha256(entry[0].encode()).digest(), entry[0]))
            path = output / f"features-{profile}-{split}.jsonl.gz"
            feature_paths[profile, split] = path
            with gzip.open(path, "wb") as handle:
                for _, row in prepared:
                    label = ACTIONS.index(row.action)
                    importance = row.sample_weight * (float(cast(float, options["keep_importance"])) if label == 0 else 1.0)
                    if isinstance(row, ActionRow):
                        dropped = split == "train" and identifier_evidence_dropped(row.identifier)
                        item = evidence(row, detector, ortho, identifiers=BLIND_IDENTIFIERS if dropped else None)
                    else:
                        item = row.evidence
                    features = extract_action_features(item)
                    if split == "train":
                        feature_mass.add(features, row.sample_weight)
                    handle.write(canonical([features, label, importance]))
            print(f"features {profile}/{split}: {len(prepared)}, span frames={len(spans.frames)}", flush=True)
    minimum = float(cast(float, options["minimum_feature_mass"]))
    names = select_features(feature_mass.values, minimum, int(cast(int, options["maximum_features"])))
    def combined(split: str) -> Iterable[tuple[dict[str, float], int, float]]:
        for profile in profiles:
            yield from feature_rows(feature_paths[profile, split])
    train = Packed.build(combined("train"), names)
    development = {name: Packed.build(feature_rows(feature_paths[name, "development"]), names) for name in profiles}
    calibration = {name: Packed.build(feature_rows(feature_paths[name, "calibration"]), names) for name in profiles}
    support_model = ContextModel({name: (0.0,) * len(ACTIONS) for name in names}, "context-v3-vocabulary", feature_version=CONTEXT_ACTION_FEATURE_VERSION)
    development_masks = {name: runtime_masks(feature_rows(feature_paths[name, "development"]), support_model)
                         for name in profiles}
    development_mass = sum(sum(data.importance) for data in development.values())
    kernel = Kernel.load()
    weights = array("d", [0.0]) * (len(names) * len(ACTIONS))
    accumulators = array("d", [1.0]) * (len(names) * len(ACTIONS))
    best, best_epoch, best_loss = array("d"), 0, math.inf
    best_selection: EpochSelection | None = None
    history: list[dict[str, object]] = []
    for epoch in range(int(cast(int, options["epochs"]))):
        kernel.epoch(train, weights, accumulators, float(cast(float, options["learning_rate"])))
        rounded = array("d", (round(value, DETERMINISTIC_ROUNDING_DECIMALS) for value in weights))
        predictions = {name: kernel.predict(data, rounded) for name, data in development.items()}
        loss = sum(-data.importance[row] * math.log(max(LOG_LOSS_PROBABILITY_FLOOR, predictions[name][row * len(ACTIONS) + label]))
                   for name, data in development.items() for row, label in enumerate(data.labels)) / development_mass
        selection = assess_epoch({name: (apply_support_mask(predictions[name], *development_masks[name]), data.labels)
                                  for name, data in development.items()},
                                 thresholds=development_thresholds(options), loss=loss,
                                 minimum_net_benefit=int(cast(dict[str, int], options["epoch_selection"])["minimum_net_benefit_per_profile"]))
        if selection is not None and (best_selection is None or selection.rank > best_selection.rank):
            best, best_epoch, best_loss, best_selection = rounded, epoch + 1, loss, selection
        history.append({"epoch": epoch + 1, "development_loss": loss,
                        "selection": asdict(selection) if selection is not None else None})
        print(f"epoch {epoch + 1}: development_loss={loss:.9f}, best={best_epoch}", flush=True)
    if best_selection is None:
        raise ValueError("no epoch repaired more than it broke on development")
    gates = cast(dict[str, int | float | bool], options["gate_policy"])
    mapping = {name: list(best[index * len(ACTIONS):index * len(ACTIONS) + len(ACTIONS)]) for index, name in enumerate(names)}
    candidate = ContextModel({name: tuple(values) for name, values in mapping.items()},
                             "context-v3-fitting", feature_version=CONTEXT_ACTION_FEATURE_VERSION)
    calibration_predictions = {name: (apply_runtime_support(kernel.predict(data, best),
                        feature_rows(feature_paths[name, "calibration"]), candidate), data.labels)
                   for name, data in calibration.items()}
    threshold, calibration_report, passed = choose_threshold(
        calibration_predictions, cast(list[float], options["threshold_candidates"]),
        int(gates["minimum_calibration_net_benefit"]), float(gates["minimum_calibration_conversion_recall"]),
        minimum_threshold=best_selection.threshold,
        net_benefit_tolerance=float(cast(float, cast(dict[str, object], options["threshold_selection"])["net_benefit_tolerance"])),
        maximum_threshold=float(cast(float, cast(dict[str, object], options["threshold_selection"])["maximum_threshold"])),
        authored_floor=float(cast(float, cast(dict[str, object], options["threshold_selection"])["authored_floor"])),
    )
    weight_hash = hashlib.sha256(json.dumps(mapping, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode()).hexdigest()
    payload = {"actions": list(ACTIONS), "feature_version": CONTEXT_ACTION_FEATURE_VERSION, "weights": mapping,
               "weights_sha256": weight_hash, "version": "context-v3-" + weight_hash[:VERSION_HASH_CHARACTERS],
               "conversion_threshold": threshold}
    (output / ARTIFACT).write_bytes(canonical(payload))
    ContextModel.load(output / ARTIFACT)
    if provenance() != before_provenance or checksum(corpus / "manifest.json") != corpus_hash:
        raise ValueError("training inputs changed while fitting; candidate cannot be sealed")
    seal: dict[str, object] = {"schema_version": 1,
        "stage": SEALED_BEFORE_TEST if passed else REJECTED_BEFORE_TEST,
        "artifact_sha256": checksum(output / ARTIFACT), "model_version": payload["version"],
        "provenance": before_provenance, "corpus_manifest_sha256": corpus_hash,
        "recipe": options, "gate_policy": gates, "calibration": calibration_report,
        "development": {"selected_epoch": best_epoch, "loss": best_loss, "history": history,
                        "selection": asdict(best_selection)},
        "conversion_threshold": threshold, "feature_count": len(names),
        "span_curriculum": span_reports,
        "lookahead_curriculum": lookahead_reports,
        "natural_lookahead_curriculum": natural_reports,
        "lexical_short_pair_curriculum": lexical_reports,
        "captured_curriculum": captured_report,
        "capital_citation_curriculum": capital_report,
        "dotted_abbreviation_curriculum": dotted_report,
        "planned_mass_balance": balance_reports,
        "test_accessed": False,
        "scope": "natural KEEP plus declared layout/mixed-context interventions; sequence evaluation required"}
    (output / SEAL).write_bytes(canonical(seal))
    print(f"{seal['stage']}: {payload['version']}; threshold={threshold}; test not read", flush=True)
    return seal


def feature_rows(path: Path) -> Iterable[tuple[dict[str, float], int, float]]:
    with gzip.open(path, "rt", encoding="utf-8") as source:
        for line in source:
            features, label, importance = cast(tuple[dict[str, float], int, float], json.loads(line))
            yield features, label, importance


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    seal = fit(args.corpus, args.output)
    return 0 if seal["stage"] == SEALED_BEFORE_TEST else 1


if __name__ == "__main__":
    raise SystemExit(main())
