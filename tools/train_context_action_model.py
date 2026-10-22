#!/usr/bin/env python3
"""Fit an isolated action candidate; never read the prospective test split.

Natural surface forms are KEEP labels. Layout corruption and spelling errors
are declared interventions, not labels inferred from a dictionary or teacher.
The engine evaluator must separately check the complete physical input stream.
"""

from __future__ import annotations

import argparse
from bisect import bisect_left
import gc
import gzip
import hashlib
import json
import math
import re
import string
from array import array
from collections import Counter, defaultdict
from collections.abc import Callable, Iterable, Iterator, Mapping, Sequence
from concurrent.futures import ThreadPoolExecutor
from dataclasses import asdict, dataclass, field, replace
from functools import partial
from itertools import product
from pathlib import Path
from typing import Final, Generic, TypeVar, cast
from unittest.mock import patch

from keyswitch.context_action_features import (
    abbreviation_question, alnum_question, alone_question, attested_abbreviation, capitals_question, extract_action_features,
    brief_question, brief_settled, dotted_question, first_question, joined_question, latin_abbreviation_question, letter_question,
    russian_ending_bucket, russian_start_bucket, start_question, term_question, unknown_question, unknown_shape, variable_question,
)
from keyswitch.context_model import (
    ACTIONS, AfterOrigin, ContextAction, ContextEvidence, ContextModel, _term_frequency, term_bucket,
)
from keyswitch.context_policy import evidence_for_decision
from keyswitch.detector import LanguageDetector
from keyswitch.identifier_lexicon import IdentifierLexicon
from keyswitch.input_context import FieldContext, FieldRole
from keyswitch.lexicon_supplement import supplement_words
from keyswitch.intent_model import CorrectionTrigger, IntentModelStatus, LinearNgramModel
from keyswitch.language_model import LanguageModel
from keyswitch.ortho_model import OrthoModel
from keyswitch.short_words import TRUSTED_SINGLE_LETTER_WORDS
from keyswitch.word_decision import automatic_word_decision

from keyswitch.constants.model_protocol import (
    CALIBRATION,
    DEVELOPMENT,
    FITTING_SPLITS,
    REFERENCE_HUNSPELL,
    REJECTED_BEFORE_TEST,
    SEALED_BEFORE_TEST,
    TRAIN,
)
from reference_lexicon import reference_models
from action_epoch_selection import EpochSelection, assess_epoch
from context_action_spans import SpanCurriculum, SpanFrame
from context_deferral import counted_reading, deferred_isolated, lookahead_focus, plausible_reading
from context_lookahead_curriculum import LookaheadAnchor, LookaheadSeed, build_lookahead_curriculum
from context_optimizer import Kernel, Packed
from context_physical_keys import translated as translated
from evaluate_context_action_sequences import LEDGER_ROOT, runtime_provenance
from freeze_context_action_corpus import CorpusRow, load_split, physical, readable, typo_variants
from freeze_context_action_holdout import ledger_test_aliases
from reconcile_context_action_corpus import expanded_aliases
from train_context_model import CapturedSource
from keyswitch.constants.file_formats import HEXADECIMAL_BASE, VERSION_HASH_CHARACTERS
from keyswitch.constants.keyboard import LAYOUT_GROUP_COUNT
from keyswitch.constants.models import (
    ABBREVIATION_FEATURE_PREFIX, ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS, ALNUM_FEATURE_PREFIX, ALONE_FEATURE_PREFIX,
    ALONE_HEAD_LETTERS,
    BRIEF_FEATURE_PREFIX, BRIEF_HEAD_LETTERS, DOTTED_FEATURE_PREFIX, FIRST_FEATURE_PREFIX, VARIABLE_FEATURE_PREFIX,
    CAPITALS_FEATURE_PREFIX, CONTEXT_ACTION_FEATURE_VERSION,
    CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS, JOINED_FEATURE_PREFIX,
    KEPT_CONTEXT_WORD_MAX_CHARACTERS, KEPT_FEATURE_PREFIX, LATIN_ABBREVIATION_FEATURE_PREFIX, LETTER_FEATURE_PREFIX,
    PLANNED_CONTEXT_AFTER_MAX_CHARACTERS, PLANNED_CONTEXT_WORD_MAX_CHARACTERS, START_FEATURE_PREFIX, TERM_FEATURE_PREFIX,
    TERM_HEAD_ENDING_LETTERS, TERM_HEAD_TRAILING_SIGNS, UNKNOWN_FEATURE_PREFIX, UNKNOWN_HEAD_MIN_CHARACTERS,
    PLANNED_CONTEXT_WORD_MAX_CHARACTERS as PLANNED_REVISIT_MAX_CHARACTERS,
    UNKNOWN_HEAD_SIGN_KEY_LETTERS,
)
from keyswitch.constants.training import (
    ABBREVIATION_SPLIT_MODULUS,
    JOINED_REPEAT_CAPITAL_CHOICES,
    JOINED_REPEAT_MAX_LETTERS,
    JOINED_REPEAT_MIN_LETTERS,
    JOINED_REPEAT_SPLIT_MODULUS,
    BRIEF_BOUNDARIES,
    BRIEF_SPLIT_MODULUS,
    FIRST_PAUSE_WORD_START_MIN_BUCKET,
    TERM_CAPITALS_MAX_LETTERS,
    TERM_COUNT_DROPOUT_FAMILIES,
    TERM_COUNT_DROPOUT_MIN_LETTERS,
    TERM_FRAGMENT_MIN_ENDING_BUCKET,
    TERM_FRAGMENT_MIN_REST,
    TERM_ALONE_WAIT_MAX_LETTERS,
    TERM_REVISIT_FOLLOWERS,
    TERM_REVISIT_FOLLOWER_MAX_LETTERS,
    TERM_SPLIT_MODULUS,
    UNKNOWN_TOKEN_MAX_CHARACTERS,
    UNKNOWN_TOKEN_SPLIT_MODULUS,
    CONTEXT_ACTION_BACKEND_AUTO,
    CONTEXT_ACTION_BACKENDS,
    CONTEXT_ACTION_BACK_END_SOURCES,
    BOUNDARY_EVENT_CHOICES,
    CAPITAL_CITATION_MAX_LETTERS,
    CAPITAL_CITATION_MIN_LETTERS,
    CAPITAL_CITATION_PUNCTUATION,
    CAPITAL_CITATION_RUSSIAN_BUCKET,
    CITATION_SIGN_HEADS,
    COMMAND_FAMILY_IDENTIFIER_PARTS,
    CONTEXT_RING_HEX_DIGITS,
    CONTEXT_RING_POINTS,
    DETERMINISTIC_CHOICE_HEX_DIGITS,
    DETERMINISTIC_ROUNDING_DECIMALS,
    EPOCH_NET_BENEFIT_TOLERANCE,
    FEATURE_MASS_TOLERANCE,
    FIELD_AFTER_SAMPLE_MODULUS,
    IDENTIFIER_DROPOUT_FAMILIES,
    IDENTIFIER_SUFFIX_SEGMENTS,
    KEPT_NEIGHBOUR_CAPITALS_MODULUS,
    KEPT_NEIGHBOUR_FRAMES_PER_WORD,
    KEPT_NEIGHBOUR_MIN_LETTERS,
    LETTER_CONTEXT_TOKENS,
    LETTER_CURRICULUM_MESSAGE_START_MODULUS,
    LEXICAL_PAIR_ANCHOR_VARIANTS,
    LOG_LOSS_PROBABILITY_FLOOR,
    LONE_WORD_BOUNDARIES,
    LONE_WORD_SPLIT_MODULUS,
    LOOKAHEAD_ANCHOR_MAX_CHARACTERS,
    LOOKAHEAD_ANCHOR_MIN_CHARACTERS,
    MASS_REPORT_DECIMALS,
    MAX_CONTEXTS_PER_FAMILY,
    QUOTE_TAIL,
    QUOTE_TAIL_MODULUS,
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
# Words joined by hyphens, as the joined-token curriculum finds them in a row's text.
JOINED_WORDS = re.compile(r"[A-Za-zА-Яа-яЁё]+(?:-[A-Za-zА-Яа-яЁё]+)+")


def canonical(value: object) -> bytes:
    return (json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":")) + "\n").encode()


def checksum(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def artifact_bytes(payload: Mapping[str, object]) -> bytes:
    """The artifact as a person opens it: keys sorted, and one feature with its weights to a line, so that a diff of two
    artifacts names the features that changed (a head fitted onto a frozen base adds its lines alone). The seal's
    artifact_sha256 is of these very bytes; the version is named after the canonical weights (weights_sha256), whatever
    the layout."""

    items = sorted(payload.items())
    lines = ["{"]
    for index, (key, value) in enumerate(items):
        comma = "," if index < len(items) - 1 else ""
        if key != "weights":
            lines.append(f' {json.dumps(key)}: {json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))}{comma}')
            continue
        weights = sorted(cast(Mapping[str, object], value).items())
        lines.append(' "weights": {')
        lines.extend(f'  {json.dumps(name, ensure_ascii=False)}: {json.dumps(row, separators=(",", ":"))}'
                     f'{"," if position < len(weights) - 1 else ""}' for position, (name, row) in enumerate(weights))
        lines.append(" }" + comma)
    return ("\n".join([*lines, "}"]) + "\n").encode()


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


Context = TypeVar("Context")


class ContextRing(Generic[Context]):
    """A pool of contexts a curriculum draws from by consistent hashing: a frame keeps its context while it stays.

    A pool built from the corpus rows gains and loses contexts at every corpus refresh. Drawn as
    `pool[variant_choice(identifier, purpose, len(pool))]`, one context more or less dealt nearly every frame a new
    one: the code of 0.57.0 fitted again on corpus v66 lost four hyphenated words of calibration to the pair it had
    fitted on corpus v65, with no line of code changed (18.10.2026). Each context takes CONTEXT_RING_POINTS points
    on a ring by the digest of its text, and a frame takes the context of the first point at or after the digest of
    its identifier: a refresh moves only the frames whose context left the pool or whose arc a new context took.
    """

    def __init__(self, pool: Sequence[Context]) -> None:
        self.pool = pool
        ring = sorted((_ring_point(f"{index}:{context}"), position)
                      for position, context in enumerate(pool) for index in range(CONTEXT_RING_POINTS))
        self.points = [point for point, _ in ring]
        self.owners = [position for _, position in ring]

    def __len__(self) -> int:
        return len(self.pool)

    def choice(self, identifier: str, purpose: str) -> Context:
        slot = bisect_left(self.points, _ring_point(purpose + ":" + identifier)) % len(self.points)
        return self.pool[self.owners[slot]]


def _ring_point(text: str) -> int:
    return int(hashlib.sha256(text.encode()).hexdigest()[:CONTEXT_RING_HEX_DIGITS], HEXADECIMAL_BASE)


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
    # Asked again as the engine asks a converted word whose next word went back (engine._revert_with_next_word): the
    # baseline is the word's own verdict at its boundary, from the text before it, with no conversion (evidence).
    revisit: bool = False

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
        # The keys of a Latin abbreviation that spell one Russian text uses (`TC` is `ЕС`) are that abbreviation
        # after Russian prose: no frame says they convert. Labelled convert, they taught the model to turn
        # `страны ЕС` into `страны TC` (0.37-0.42).
        attested = group == 0 and attested_abbreviation(alternate)
        if not attested:
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
            if not attested:
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


def _varied_boundary(identifier: str) -> tuple[CorrectionTrigger, str]:
    """The boundary a curriculum word ends at, by hash: a space (twice as often), Enter, a punctuation
    mark, Tab or a pause, with the character Enter and Tab type in half of their frames."""

    triggers: tuple[CorrectionTrigger, ...] = ("space", "space", "enter", "punctuation", "tab", "pause")
    trigger = triggers[variant_choice(identifier, "trigger", len(triggers))]
    boundary_text = ""
    if trigger == "space":
        boundary_text = " "
    elif trigger == "punctuation":
        boundary_text = CAPITAL_CITATION_PUNCTUATION[variant_choice(identifier, "punctuation", len(CAPITAL_CITATION_PUNCTUATION))]
    elif trigger in {"enter", "tab"} and variant_choice(identifier, "boundary-event", BOUNDARY_EVENT_CHOICES):
        boundary_text = "\n" if trigger == "enter" else "\t"
    return trigger, boundary_text


def capital_citation_curriculum(source_rows: Sequence[CorpusRow], refused: frozenset[str],
                                options: Mapping[str, object], models: dict[int, LanguageModel] | None = None,
                                ) -> tuple[list[ActionRow], dict[str, object]]:
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
    stand in the same frame with the convert label, so case is what the pair tells apart. `models` are
    the reference lexicons without morphology when the caller already holds them.
    """
    budgets = {int(length): int(count) for length, count in cast(dict[str, int], options["words_by_length"]).items()}
    weight = float(cast(float, options["sample_weight"]))
    contrast = options.get("lowercase_contrast") is True
    contexts = ContextRing(natural_mixed_contexts(source_rows)[1])
    if not contexts or not any(budgets.values()):
        return [], {"words": 0, "words_by_length": {}, "scope": "not used"}
    if models is None:
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
    applications = ("Telegram", "Code", "chrome", "UnseenEditor")
    rows: list[ActionRow] = []
    chosen: dict[str, list[str]] = {}
    for length, budget in sorted(budgets.items()):
        ranked = sorted(candidates[length], key=lambda pair: hashlib.sha256(("capital-citation:" + pair[0]).encode()).digest())
        chosen[str(length)] = [f"{word}/{latin}" for word, latin in ranked[:budget]]
        for word, latin in ranked[:budget]:
            identifier = "capital-citation:" + word
            trigger, boundary_text = _varied_boundary(identifier)
            field = FieldContext(applications[variant_choice(identifier, "application", len(applications))], "public-training",
                                 contexts.choice(identifier, "context"), "", "unknown")
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


def english_capital_curriculum(source_rows: Sequence[CorpusRow], refused: frozenset[str],
                               options: Mapping[str, object], models: dict[int, LanguageModel] | None = None,
                               ) -> tuple[list[ActionRow], dict[str, object]]:
    """Latin capitals inside English prose stay, whatever their Cyrillic reading is.

    Test v28, read by a candidate whose every decision outside the kept-neighbour question was the
    corpus v26 pair's, holds two Latin capitals inside an English sentence whose keys spell `из`: with
    the reference lexicons and the early switch off they became `ИЗ` at p=0.997, for `из` is among the commonest Russian words and the
    model had seen Latin capitals framed after Russian prose only (capital_citation_curriculum).
    English text cites abbreviations of a few capitals constantly, and their keys spell a Russian word
    as often as not. The frames: the keys of the Russian lexicon's commonest words of each length of
    `words_by_length`, typed in the Latin layout in capitals, after the left context of an English
    TRAIN row whose words are all Latin, chosen by hash, `frames_per_word` contexts a word - keep;
    with `lowercase_contrast` the same keys in lower case stand in the same frame with the convert
    label, so case is what the pair tells apart. Without that pair the keep frames taught that the
    keys of `не`, `на`, `как`, `что` stay in any case: the first candidates of corpus v29 left 31 more
    of the owner's Russian words typed in the Latin layout as typed (`yt`, `yf`, `rfr`, `xnj`). With
    `russian_contrast` the same keys in capitals stand after the left context of a Russian TRAIN row
    with the convert label: the word typed with Caps Lock, which a capitals head would otherwise learn
    to keep with the cited abbreviations. Words of this corpus's test and of every accessed test are
    refused by their aliases. `models` are the reference lexicons without morphology when the caller
    already holds them.
    """

    budgets = {int(length): int(count) for length, count in cast(dict[str, int], options["words_by_length"]).items()}
    per_word = int(cast(int, options["frames_per_word"]))
    weight = float(cast(float, options["sample_weight"]))
    contrast = options.get("lowercase_contrast") is True
    natural = natural_mixed_contexts(source_rows)
    contexts, russian = ContextRing(natural[0]), ContextRing(natural[1])
    russian_contrast = options.get("russian_contrast") is True and bool(russian)
    if not contexts or not any(budgets.values()) or not per_word:
        return [], {"frames": 0, "scope": "not used"}
    frequencies = (models if models is not None else reference_models(False))[1].frequencies
    candidates: dict[int, list[tuple[str, str]]] = defaultdict(list)
    for word in sorted(frequencies, key=lambda word: (-frequencies[word], word)):
        if len(word) not in budgets or not _cyrillic(word) or not word.islower():
            continue
        latin = translated(word.upper(), 1)
        if not latin.isascii() or not latin.isalpha() or refused & (expanded_aliases(word) | expanded_aliases(latin)):
            continue
        candidates[len(word)].append((word, latin))
    applications = ("Telegram", "Code", "chrome", "UnseenEditor")
    rows: list[ActionRow] = []
    chosen: dict[str, list[str]] = {}
    for length, budget in sorted(budgets.items()):
        chosen[str(length)] = [f"{word}/{latin}" for word, latin in candidates[length][:budget]]
        for word, latin in candidates[length][:budget]:
            for index in range(per_word):
                identifier = f"english-capital:{word}:{index}"
                trigger, boundary_text = _varied_boundary(identifier)
                field = FieldContext(applications[variant_choice(identifier, "application", len(applications))],
                                     "public-training", contexts.choice(identifier, "context"),
                                     "", "unknown")
                rows.append(ActionRow(identifier, latin, 0, field, trigger, "", "keep", "english_capital",
                                      boundary_text, weight))
                if contrast:
                    rows.append(ActionRow(identifier + ":lower", latin.lower(), 0, field, trigger, "", "convert",
                                          "english_capital_lower", boundary_text, weight))
                if russian_contrast:
                    other = identifier + ":russian"
                    rows.append(ActionRow(other, latin, 0, replace(field, before=russian.choice(other, "context")),
                                          trigger, "", "convert", "english_capital_russian", boundary_text, weight))
    return rows, {"frames": len(rows), "sample_weight": weight, "frames_per_word": per_word,
                  "lowercase_contrast": contrast, "russian_contrast": russian_contrast,
                  "words_by_length": chosen,
                  "candidates_by_length": {str(length): len(candidates[length]) for length in sorted(budgets)},
                  "scope": "TRAIN only: keep frames of the keys of common Russian words typed in Latin capitals after English prose (english_capital_curriculum)."}


def abbreviation_label(form: str, latin: str, options: Mapping[str, object]) -> ContextAction:
    """The abbreviation head's label for an attested Cyrillic abbreviation typed as written after Russian prose:
    convert when its Latin reading occurs `minimum_latin_count` times or more and `dominance` times as often as
    it does among the words of Russian technical text (`ФШ` is `AI`), keep otherwise (`ЕС` is not `TC`, `ИД` is
    not `BL`): both counts are the term table's, of the same text."""

    table = _term_frequency()
    cyrillic = table["cyrillic"].get(form.casefold(), 0)
    latin_count = table["latin"].get(latin.casefold(), 0)
    convert = (latin_count >= int(cast(int, options["minimum_latin_count"]))
               and latin_count >= float(cast(float, options["dominance"])) * cyrillic)
    return "convert" if convert else "keep"


def abbreviation_curriculum(split: str, source_rows: Sequence[CorpusRow], refused: frozenset[str],
                            options: Mapping[str, object]) -> tuple[list[ActionRow], dict[str, object]]:
    """Every attested Cyrillic abbreviation (context_action_features.attested_abbreviation) typed as written
    after Russian prose, the abbreviation head's class, labelled by abbreviation_label.

    The frozen model turned Russian abbreviations into the Latin readings their keys spell
    (`страны ЕС приняли` became `страны TC приняли`, `поздравляю с НГ` became `поздравляю с YU`,
    0.37-0.42): the corpus framed Latin abbreviations typed in the Russian layout after Russian prose with
    the convert label whatever their keys spelled, and Russian abbreviations are few in its rows. The forms
    are the term table's Cyrillic words of CAPITALS_HEAD_MIN_LETTERS to CAPITALS_HEAD_MAX_LETTERS letters in
    capitals whose Latin reading is letters too, the table the head reads at runtime; each goes to TRAIN or
    DEVELOPMENT by hash (ABBREVIATION_SPLIT_MODULUS), so DEVELOPMENT chooses the head's epoch on forms TRAIN
    never saw, and stands after `frames_per_form` left contexts of Russian rows of the split, each ending at
    a boundary chosen by hash (_varied_boundary). Forms of this corpus's test and of every accessed test are
    refused by their aliases.
    """

    per_form = int(cast(int, options["frames_per_form"]))
    contexts = ContextRing(natural_mixed_contexts(source_rows)[1])
    if not per_form or not contexts or split not in (TRAIN, DEVELOPMENT):
        return [], {"frames": 0, "scope": "not used"}
    weight = float(cast(float, options["sample_weight"]))
    table = _term_frequency()
    applications = ("Telegram", "Code", "chrome", "UnseenEditor")
    rows: list[ActionRow] = []
    counts: Counter[str] = Counter()
    forms = sorted(word.upper() for word in table["cyrillic"])
    for form in forms:
        if not attested_abbreviation(form):
            continue
        latin = translated(form, 1)
        if not latin.isalpha():
            continue
        share = DEVELOPMENT if variant_choice("abbreviation:" + form, "split", ABBREVIATION_SPLIT_MODULUS) == 0 else TRAIN
        if share != split:
            continue
        if refused & (expanded_aliases(form) | expanded_aliases(latin)):
            counts["refused"] += 1
            continue
        action = abbreviation_label(form, latin, options)
        counts[action] += 1
        for index in range(per_form):
            identifier = f"abbreviation:{form}:{index}"
            trigger, boundary_text = _varied_boundary(identifier)
            field = FieldContext(applications[variant_choice(identifier, "application", len(applications))], "public-training",
                                 contexts.choice(identifier, "context"), "", "unknown")
            rows.append(ActionRow(identifier, form, 1, field, trigger, "", action, "abbreviation", boundary_text, weight))
    return rows, {"frames": len(rows), "forms": dict(sorted(counts.items())), "frames_per_form": per_form, "sample_weight": weight,
                  "scope": "attested Cyrillic abbreviations of this split's share, typed as written after Russian prose, labelled "
                           "by the term counts of both readings (abbreviation_curriculum)."}


def latin_abbreviation_label(form: str, latin: str, options: Mapping[str, object]) -> ContextAction:
    """The Latin-context abbreviation head's label for an attested Cyrillic abbreviation typed as written after Latin
    text: keep when Russian technical text uses it `minimum_cyrillic_count` times or more and `dominance` times as
    often as its Latin reading occurs among the Latin words of that text or in English prose (`ТЗ`, 744 times against
    80 for `NP`; `НГ`, whose keys `YU` nobody writes), convert otherwise (`ГЫ` is `US`, 19 273 times in English prose;
    `УГ` is `EU`). After Latin text the writer is in Latin more often than not, so the abbreviation stays only where
    its own count clearly outweighs its keys'."""

    table = _term_frequency()
    cyrillic = table["cyrillic"].get(form.casefold(), 0)
    latin_count = max(table["latin"].get(latin.casefold(), 0), table["english"].get(latin.casefold(), 0))
    keep = (cyrillic >= int(cast(int, options["minimum_cyrillic_count"]))
            and cyrillic >= float(cast(float, options["dominance"])) * latin_count)
    return "keep" if keep else "convert"


def latin_abbreviation_curriculum(split: str, source_rows: Sequence[CorpusRow], refused: frozenset[str],
                                  options: Mapping[str, object]) -> tuple[list[ActionRow], dict[str, object]]:
    """Every attested Cyrillic abbreviation (context_action_features.attested_abbreviation) typed as written after
    Latin text, the Latin-context abbreviation head's class, labelled by latin_abbreviation_label.

    After Latin text the frozen model turned such abbreviations into the Latin readings their keys spell whatever
    their counts: `/designv2 ТЗ` became `/designv2 NP` in the owner's typing (08.10.2026), and `ЛС`, `ТГ` and `НГ`
    became `KC`, `NU` and `YU` the same way. The forms and their TRAIN or DEVELOPMENT share are the abbreviation
    curriculum's (the same hash), so DEVELOPMENT holds no form either abbreviation head was fitted on. Each form
    stands after `frames_per_form` left contexts, in turn of an English row of the split (prose) and of one or two
    Latin terms Russian technical text counts at least `minimum_term_count` times (_counted_terms: a command or an
    identifier the writer typed before it), each ending at a boundary chosen by hash (_varied_boundary). Forms and
    terms of this corpus's test and of every accessed test are refused by their aliases.
    """

    per_form = int(cast(int, options["frames_per_form"]))
    english = ContextRing(natural_mixed_contexts(source_rows)[0])
    terms = ContextRing(_counted_terms(refused, int(cast(int, options["minimum_term_count"]))) if per_form else [])
    if not per_form or not english or not terms or split not in (TRAIN, DEVELOPMENT):
        return [], {"frames": 0, "scope": "not used"}
    weight = float(cast(float, options["sample_weight"]))
    applications = ("Telegram", "Code", "chrome", "UnseenEditor")
    rows: list[ActionRow] = []
    counts: Counter[str] = Counter()
    for form in sorted(word.upper() for word in _term_frequency()["cyrillic"]):
        if not attested_abbreviation(form):
            continue
        latin = translated(form, 1)
        if not latin.isalpha():
            continue
        share = DEVELOPMENT if variant_choice("abbreviation:" + form, "split", ABBREVIATION_SPLIT_MODULUS) == 0 else TRAIN
        if share != split:
            continue
        if refused & (expanded_aliases(form) | expanded_aliases(latin)):
            counts["refused"] += 1
            continue
        action = latin_abbreviation_label(form, latin, options)
        counts[action] += 1
        for index in range(per_form):
            identifier = f"latin-abbreviation:{form}:{index}"
            trigger, boundary_text = _varied_boundary(identifier)
            if index % LAYOUT_GROUP_COUNT == 0:
                before = english.choice(identifier, "context")
            else:
                words = [terms.choice(identifier, "context")]
                if variant_choice(identifier, "context-words", LAYOUT_GROUP_COUNT):
                    words.insert(0, terms.choice(identifier, "first-context"))
                before = " ".join(words) + " "
            field = FieldContext(applications[variant_choice(identifier, "application", len(applications))], "public-training",
                                 before, "", "unknown")
            rows.append(ActionRow(identifier, form, 1, field, trigger, "", action, "latin_abbreviation", boundary_text, weight))
    return rows, {"frames": len(rows), "forms": dict(sorted(counts.items())), "frames_per_form": per_form, "sample_weight": weight,
                  "scope": "attested Cyrillic abbreviations of this split's share, typed as written after English prose or Latin terms, "
                           "labelled by the term counts of both readings (latin_abbreviation_curriculum)."}


def unknown_token_curriculum(split: str, source_rows: Sequence[CorpusRow], refused: frozenset[str],
                             options: Mapping[str, object]) -> tuple[list[ActionRow], dict[str, object]]:
    """Tokens of the unknown-token head's class (context_action_features.unknown_question) after Russian prose.

    The frozen model kept a token no table counts after Russian prose as Russian slang, and the terms typed in the
    wrong layout with it: `цуиоы` (`webjs`), `мгуоы` (`vuejs`), `реьч` (`htmx`) stayed (the owner's typing in a chat
    input of VS Code, 13.10.2026). The corpus already frames the class's Russian words: rare words, names and typos of
    its Russian rows typed as written, keep. This curriculum adds what it lacks:
    - terms: one in `item_modulus` of the Latin tokens of letters and digits Russian technical text uses at least
      `minimum_term_count` times whose keys in the Russian layout are a token of the class's shape that no lexicon
      holds and Russian prose does not use, each after the left context of a Russian row of the split: typed in the
      Russian layout with the term typed right a sentence before (convert; the owner typed `webjs` and then `цуиоы` in
      one message, and the head answers Russian letters only after such an echo), and typed as meant (keep), for one
      term in `echo_modulus` also after itself and after its keys left in Cyrillic by an earlier miss. Each term goes to
      TRAIN or DEVELOPMENT by hash (UNKNOWN_TOKEN_SPLIT_MODULUS);
    - echoes kept: one in `echo_modulus` of the class's Russian words of the split's own rows, after their left context,
      with their keys typed earlier in the text as a miss left them there - keep, so that the other reading earlier in
      the text alone converts nothing.
    A TRAIN family of TERM_COUNT_DROPOUT_MIN_LETTERS letters or more drops its own term counts one time in
    TERM_COUNT_DROPOUT_FAMILIES (frame_evidence). Items of this corpus's test, of every accessed test and of the
    recipe's `held_out` list (the tokens of the authored batteries that gate this head) are refused.
    """

    item_modulus = int(cast(int, options["item_modulus"]))
    contexts = ContextRing(natural_mixed_contexts(source_rows)[1])
    if not item_modulus or not contexts or split not in (TRAIN, DEVELOPMENT):
        return [], {"frames": 0, "scope": "not used"}
    weight = float(cast(float, options["sample_weight"]))
    echo_modulus = int(cast(int, options["echo_modulus"]))
    held_out = frozenset(str(token).casefold() for token in cast(list[object], options.get("held_out", [])))
    uncounted = CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS[0]
    table = _term_frequency()
    applications = ("Telegram", "Code", "chrome", "UnseenEditor")
    rows: list[ActionRow] = []
    counts: Counter[str] = Counter()

    def frame(identifier: str, typed: str, group: int, before: str, action: ContextAction) -> ActionRow:
        trigger, boundary_text = _varied_boundary(identifier)
        field = FieldContext(applications[variant_choice(identifier, "application", len(applications))], "public-training",
                             before[-ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS:], "", "unknown")
        return ActionRow(identifier, typed, group, field, trigger, "", action, "unknown_token", boundary_text, weight)

    for term, count in sorted(table["latin"].items()):
        if (count < int(cast(int, options["minimum_term_count"])) or not term.isascii() or not term.isalnum()
                or not term.islower() or len(set(term)) < UNKNOWN_HEAD_MIN_CHARACTERS
                or not UNKNOWN_HEAD_MIN_CHARACTERS <= len(term) <= UNKNOWN_TOKEN_MAX_CHARACTERS):
            continue
        typed = translated(term, 0)
        family = f"unknown-term:{typed}"
        if (not unknown_shape(typed) or plausible_reading(typed, 1) or table["cyrillic"].get(typed, 0) >= uncounted
                or table["russian"].get(typed, 0) >= uncounted or variant_choice(family, "draw", item_modulus)):
            continue
        share = DEVELOPMENT if variant_choice(family, "split", UNKNOWN_TOKEN_SPLIT_MODULUS) == 0 else TRAIN
        if share != split:
            continue
        if term in held_out or typed in held_out or refused & (expanded_aliases(term) | expanded_aliases(typed)):
            counts["refused"] += 1
            continue
        echo = bool(echo_modulus) and variant_choice(family, "echo", echo_modulus) == 0
        # (typed as, layout, label, what stands earlier in the text): the term typed in the Russian layout converts once
        # it was typed right before (the head answers Russian letters only then); typed as meant it stays, alone, after
        # itself, and after its keys left in Cyrillic by an earlier miss.
        frames: list[tuple[str, int, ContextAction, str]] = [(typed, 1, "convert", term), (term, 0, "keep", term if echo else "")]
        if echo:
            frames.append((term, 0, "keep", typed))
        for index, (shown, group, action, earlier_word) in enumerate(frames):
            identifier = f"{family}:{index}"
            before = contexts.choice(identifier, "context")
            if earlier_word:
                earlier = contexts.choice(identifier, "earlier-context").rstrip()
                before = f"{earlier} {earlier_word}. {before}"
            if not unknown_question(shown, before):
                # A long term earlier in a short text makes it Latin text: no frame of the class.
                counts["outside_class"] += 1
                continue
            counts[f"echo:{action}" if earlier_word else action] += 1
            rows.append(frame(identifier, shown, group, before, action))
    for row, group, _, _ in _monolingual_rows(source_rows):
        word = row.original
        if (group != 1 or not echo_modulus or not unknown_question(word, row.before) or plausible_reading(word, 1)
                or variant_choice(row.identifier, "echo-word", echo_modulus)):
            continue
        keys = translated(word, 1)
        if word.casefold() in held_out or keys.casefold() in held_out or refused & (expanded_aliases(word) | expanded_aliases(keys)):
            counts["refused"] += 1
            continue
        identifier = f"unknown-echo:{row.identifier}"
        earlier = contexts.choice(identifier, "earlier-context").rstrip()
        before = f"{earlier} {keys}. {row.before}"
        if not unknown_question(word, before):
            counts["outside_class"] += 1
            continue
        counts["echo:keep"] += 1
        rows.append(frame(identifier, word, 1, before, "keep"))
    return rows, {"frames": len(rows), "items": dict(sorted(counts.items())), "item_modulus": item_modulus,
                  "echo_modulus": echo_modulus, "held_out": len(held_out), "sample_weight": weight,
                  "scope": "terms of the Latin table typed in the Russian layout after Russian prose (convert), some with the term "
                           "typed right earlier in the text, and the class's Russian words of the split's rows with their keys "
                           "earlier in the text (keep), of this split's share (unknown_token_curriculum)."}


def joined_token_curriculum(split: str, source_rows: Sequence[CorpusRow], refused: frozenset[str],
                            options: Mapping[str, object]) -> tuple[list[ActionRow], dict[str, object]]:
    """Words joined by hyphens (context_action_features.joined_question), as the split's own text uses them.

    No term table counts such a token, and the frozen model, with no frequency evidence for it, kept `Re-re!` after a
    Russian sentence where `Ку-ку!` was meant (sealed test v58, both pairs). Each token of words of one script joined
    by hyphens in the text of the split's rows, one occurrence in `item_modulus` and at most `per_token` of one token,
    after the text before it: typed as written (keep) and its keys typed in the other layout (convert). An occurrence
    counts once however many rows of its sentence hold it. The text uses few repeated tokens (`чуть-чуть`, `ha-ha`), the
    shape `Ку-ку!` is: `repeat_words` words of each language, in hash order, that its prose counts at least
    `repeat_minimum_count` times, of JOINED_REPEAT_MIN_LETTERS to JOINED_REPEAT_MAX_LETTERS letters whose keys are letters
    in either layout, are each joined to itself after the left context of a row of their language, some opening a
    sentence with a capital - typed as written (keep) and in the other layout (convert); one word in
    JOINED_REPEAT_SPLIT_MODULUS goes to DEVELOPMENT by hash, the rest to TRAIN. Items of this corpus's test, of every accessed test and of the recipe's `held_out` list (the
    tokens of the authored battery that gates this head, and the general technical tokens of the owner's typing) are
    refused.
    """

    item_modulus = int(cast(int, options["item_modulus"]))
    if not item_modulus or split not in (TRAIN, DEVELOPMENT):
        return [], {"frames": 0, "scope": "not used"}
    weight = float(cast(float, options["sample_weight"]))
    per_token = int(cast(int, options["per_token"]))
    held_out = frozenset(str(token).casefold() for token in cast(list[object], options.get("held_out", [])))
    applications = ("Telegram", "Code", "chrome", "UnseenEditor")
    rows: list[ActionRow] = []
    counts: Counter[str] = Counter()
    taken: Counter[str] = Counter()
    seen: set[tuple[str, str, str]] = set()
    for row in source_rows:
        if not row.layout_representable:
            continue
        text = row.before + row.original + row.after
        for match in JOINED_WORDS.finditer(text):
            token = match.group()
            letters = token.replace("-", "")
            group = 1 if _cyrillic(letters) else 0 if letters.isascii() else None
            before = text[:match.start()][-ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS:]
            occurrence = (row.document, before, token)
            if group is None or occurrence in seen:
                continue
            seen.add(occurrence)
            identifier = "joined:" + hashlib.sha256("\0".join(occurrence).encode()).hexdigest()[:VERSION_HASH_CHARACTERS]
            if variant_choice(identifier, "draw", item_modulus):
                continue
            if taken[token.casefold()] >= per_token:
                counts["capped"] += 1
                continue
            keys = translated(token, group)
            if (token.casefold() in held_out or keys.casefold() in held_out
                    or refused & (expanded_aliases(token) | expanded_aliases(keys))):
                counts["refused"] += 1
                continue
            taken[token.casefold()] += 1
            kind = "repeat" if len({part.casefold() for part in token.split("-")}) == 1 else "words"
            framed: tuple[tuple[str, int, ContextAction], ...] = ((token, group, "keep"), (keys, 1 - group, "convert"))
            for index, (shown, layout, action) in enumerate(framed):
                frame = f"{identifier}:{index}"
                trigger, boundary_text = _varied_boundary(frame)
                field = FieldContext(applications[variant_choice(frame, "application", len(applications))], "public-training",
                                     before, "", "unknown")
                counts[f"{action}:{kind}"] += 1
                rows.append(ActionRow(frame, shown, layout, field, trigger, "", action, "joined_token", boundary_text, weight))
    repeat_words = int(cast(int, options.get("repeat_words", 0)))
    minimum = int(cast(int, options.get("repeat_minimum_count", 0)))
    contexts = {group: ContextRing(pool) for group, pool in natural_mixed_contexts(source_rows).items()}
    table = _term_frequency()
    for group, alphabet in ((1, "russian"), (0, "english")):
        words = sorted((word for word, count in table[alphabet].items()
                        if count >= minimum and JOINED_REPEAT_MIN_LETTERS <= len(word) <= JOINED_REPEAT_MAX_LETTERS
                        and word.isalpha() and (_cyrillic(word) and not set(word) & UNKNOWN_HEAD_SIGN_KEY_LETTERS
                                                if group else word.isascii())),
                       key=partial(_repeat_order, alphabet))
        # The first `repeat_words` words in hash order, of both splits: each split frames its share of them.
        for word in words[:repeat_words if contexts[group] else 0]:
            identifier = f"joined-repeat:{alphabet}:{word}"
            if (DEVELOPMENT if variant_choice(identifier, "split", JOINED_REPEAT_SPLIT_MODULUS) == 0 else TRAIN) != split:
                continue
            capital = variant_choice(identifier, "capital", JOINED_REPEAT_CAPITAL_CHOICES) == 0
            token = f"{word}-{word}"
            token = token[:1].upper() + token[1:] if capital else token
            keys = translated(token, group)
            if (token.casefold() in held_out or keys.casefold() in held_out
                    or refused & (expanded_aliases(token) | expanded_aliases(keys))):
                counts["refused"] += 1
                continue
            context = contexts[group].choice(identifier, "context")
            before = (f"{context.rstrip()}. " if capital else context)[-ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS:]
            pair: tuple[tuple[str, int, ContextAction], ...] = ((token, group, "keep"), (keys, 1 - group, "convert"))
            for index, (shown, layout, action) in enumerate(pair):
                frame = f"{identifier}:{index}"
                trigger, boundary_text = _varied_boundary(frame)
                field = FieldContext(applications[variant_choice(frame, "application", len(applications))], "public-training",
                                     before, "", "unknown")
                counts[f"{action}:synthetic"] += 1
                rows.append(ActionRow(frame, shown, layout, field, trigger, "", action, "joined_token", boundary_text, weight))
    return rows, {"frames": len(rows), "items": dict(sorted(counts.items())), "tokens": len(taken),
                  "item_modulus": item_modulus, "per_token": per_token, "repeat_words": repeat_words,
                  "repeat_minimum_count": minimum, "held_out": len(held_out), "sample_weight": weight,
                  "scope": "words joined by hyphens in the text of this split's rows, and counted words of either language joined "
                           "to themselves, after the text before them: typed as written (keep) and their keys in the other "
                           "layout (convert) (joined_token_curriculum)."}


def _repeat_order(alphabet: str, word: str) -> str:
    """The order the joined-token curriculum draws repeated words of one language in: by hash."""

    return hashlib.sha256(f"joined-repeat:{alphabet}:{word}".encode()).hexdigest()


def term_curriculum(split: str, source_rows: Sequence[CorpusRow], refused: frozenset[str],
                    options: Mapping[str, object]) -> tuple[list[ActionRow], dict[str, object]]:
    """Tokens of the term head's class (context_action_features.term_question): a term typed in the Russian layout.

    The frozen model left a term Russian technical text uses, typed in the Russian layout the first time a message held
    it, at p 0.92-0.98, below the conversion threshold: `сш` in `npm ci`, `зк` for `pr`, `рвьш` for `hdmi` stayed in the
    owner's typing. Converting the whole class fixed nine such words of the owner's typing and 46 of 1 831 ru.stackoverflow
    comments of 2024-2026 typed with their Latin words in the Russian layout, and turned four pieces of Russian words
    into Latin letters (`ась`, `ев`). This curriculum frames:
    - terms: one in `item_modulus` of the Latin terms of letters whose keys in the Russian layout are a token of the
      class, typed in the Russian layout after the left context of a Russian row of the split (convert), and once more in
      one variant chosen by hash: at the start of a message, ending in a sign (`.` or `,`, typed as `ю` or `б`), opening a
      sentence with a capital, or in capitals (a term of TERM_CAPITALS_MAX_LETTERS letters at most, as acronyms are
      written). Alone at the start of a message and ended by a space, a term of up to TERM_ALONE_WAIT_MAX_LETTERS
      letters waits for its next word, as the lone-word head leaves such a word (wait): the engine asks a waiting word
      again at a pause, a suggestion never, and labelled to convert there the head only suggested `гш` and `йк`, which
      stayed (experiment E9c). Any other converts: waiting, `тзь`, `сым` and `ШВУ` (`npm`, `csv`, `IDE`) opening a
      message stayed where the frozen model had converted them (experiment E9d), and so did `ювшые` (`.dist`) with two
      spaces after it; the head reads the letters with the boundary, so the two-letter wait does not pull the longer
      ones, and every short term stands once more alone at a space (experiments E9f and E9g made `гш` a suggestion:
      drawn by hash, too few short terms stood there). Alone in a field a reader read empty a term of up to three
      letters is the first-term head's (first_curriculum). Each term is also
      asked again as the engine asks a converted word once the word after it went back to Russian (one of the
      TERM_REVISIT_FOLLOWERS most frequent short words of Russian prose after it, convert), with the baseline the engine
      gives that question (ActionRow.revisit): `зк` in `займись зк и тз` became `pr`, the layout followed, `и` came out
      as `b`, and asked again the frozen model leaned to keeping `зк`.
      One term in TERM_SPLIT_MODULUS goes to DEVELOPMENT by hash;
    - pieces kept: the last TERM_HEAD_ENDING_LETTERS letters of the Russian words of the split's own rows that are a token
      of the class and end many words (an ending bucket of TERM_FRAGMENT_MIN_ENDING_BUCKET or more), of words with a
      word before them in their row (a tail alone in its field stands where a term does), at most
      `per_piece` occurrences of one piece, in an order by hash, each after its row's left context (keep) and asked
      again with a short Russian word after it (keep): an edit inside a word leaves its tail as a token of its own. Such
      a tail is rare beside a term typed in the wrong layout (in the owner's typing about one in thirteen tokens of the
      class), and a kept frame weighs `keep_importance` times its sample weight: the pieces weigh `piece_weight` of a
      term's frame. At full weight the 95 two-letter pieces outweighed the 125 two-letter terms, and the head, which
      then read no ending of Russian words, kept `зк` (`pr`) and `ьв` (`md`), which the frozen model converted (field
      batteries, experiment E3).
    Items of this corpus's test, of every accessed test and of the recipe's `held_out` list (the class's tokens of the
    owner's typing, which measure this head) are refused.
    """

    item_modulus = int(cast(int, options["item_modulus"]))
    contexts = ContextRing(natural_mixed_contexts(source_rows)[1])
    if not item_modulus or not contexts or split not in (TRAIN, DEVELOPMENT):
        return [], {"frames": 0, "scope": "not used"}
    weight = float(cast(float, options["sample_weight"]))
    piece_weight = float(cast(float, options.get("piece_weight", 1.0)))
    per_piece = int(cast(int, options["per_piece"]))
    held_out = frozenset(str(token).casefold() for token in cast(list[object], options.get("held_out", [])))
    applications = ("Telegram", "Code", "chrome", "UnseenEditor")
    rows: list[ActionRow] = []
    counts: Counter[str] = Counter()

    def frame(identifier: str, typed: str, before: str, action: ContextAction, share: float = 1.0) -> ActionRow:
        trigger, boundary_text = _varied_boundary(identifier)
        field = FieldContext(applications[variant_choice(identifier, "application", len(applications))], "public-training",
                             before[-ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS:], "", "unknown")
        return ActionRow(identifier, typed, 1, field, trigger, "", action, "term", boundary_text, weight * share)

    def refuse(token: str, keys: str) -> bool:
        return (token.casefold() in held_out or keys.casefold() in held_out
                or bool(refused & (expanded_aliases(token) | expanded_aliases(keys))))

    followers = revisit_followers()

    def revisit(identifier: str, typed: str, before: str, action: ContextAction, share: float = 1.0,
                role: FieldRole = "unknown") -> ActionRow:
        """The token asked again as the engine asks a converted word whose next word went back to Russian."""
        follower = followers[variant_choice(identifier, "follower", len(followers))]
        field = FieldContext(applications[variant_choice(identifier, "application", len(applications))], "public-training",
                             before[-ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS:], follower, role)
        origin: AfterOrigin = "planned_next_conversion" if len(typed) <= PLANNED_REVISIT_MAX_CHARACTERS else "field"
        return ActionRow(identifier, typed, 1, field, "space", "", action, "term", " ", weight * share, after_origin=origin,
                         revisit=True)

    for term in sorted(_term_frequency()["latin"]):
        if not term.isascii() or not term.isalpha() or not term.islower():
            continue
        typed = translated(term, 0)
        family = f"term:{typed}"
        if variant_choice(family, "draw", item_modulus) or not term_question(typed, ""):
            continue
        if (DEVELOPMENT if variant_choice(family, "split", TERM_SPLIT_MODULUS) == 0 else TRAIN) != split:
            continue
        if refuse(term, typed):
            counts["refused"] += 1
            continue
        context = contexts.choice(family, "context")
        other = contexts.choice(family, "other-context")
        variants = ["start", "signed", "capital", *(["upper"] if len(term) <= TERM_CAPITALS_MAX_LETTERS else [])]
        variant = variants[variant_choice(family, "variant", len(variants))]
        signs = translated(TERM_HEAD_TRAILING_SIGNS, 0)
        shown = {"start": typed, "signed": typed + signs[variant_choice(family, "sign", len(signs))],
                 "capital": typed[:1].upper() + typed[1:], "upper": typed.upper()}[variant]
        before = {"start": "", "capital": f"{other.rstrip()}. "}.get(variant, other)
        for index, (kind, token, left) in enumerate((("plain", typed, context), (variant, shown, before))):
            if not term_question(token, left):
                # A term written right earlier in the text is the unknown-token head's.
                counts["outside_class"] += 1
                continue
            framed = frame(f"{family}:{index}", token, left, "convert")
            if not left and framed.trigger == "space" and len(token) <= TERM_ALONE_WAIT_MAX_LETTERS:
                framed = replace(framed, action="wait")
            counts[f"{framed.action}:{kind}"] += 1
            rows.append(framed)
        if term_question(typed, context):
            counts["convert:revisit"] += 1
            rows.append(revisit(f"{family}:revisit", typed, context, "convert"))
        if len(typed) <= max(TERM_HEAD_ENDING_LETTERS) and term_question(typed, ""):
            field = FieldContext(applications[variant_choice(family, "alone-application", len(applications))],
                                 "public-training", "", "", "unknown")
            if len(typed) <= TERM_ALONE_WAIT_MAX_LETTERS:
                # Every short term once more alone at a space: drawn among the variants and boundaries by hash, too few
                # of them stood there against the longer terms that convert, and `гш` came out a suggestion (experiments
                # E9f, E9g).
                counts["wait:alone"] += 1
                rows.append(ActionRow(f"{family}:alone", typed, 1, field, "space", "", "wait", "term", " ", weight))
    taken: Counter[str] = Counter()
    russian = sorted((row for row, group, _, _ in _monolingual_rows(source_rows) if group == 1),
                     key=lambda row: hashlib.sha256(f"term-piece:{row.identifier}".encode()).digest())
    for row in russian:
        if not any(char.isalpha() for char in row.before):
            # A tail with no word before it stands alone in its field, a term's place: framed kept there, the tails of
            # words opening a row outweighed the wait of a lone short term in a field nobody read, and `зь`, `цу`
            # (`pm`, `we`), which 0.56.0 left waiting, were kept (candidate v64c).
            counts["skipped:row-start"] += 1
            continue
        for size in TERM_HEAD_ENDING_LETTERS:
            piece = row.original[-size:]
            if (len(row.original) - size < TERM_FRAGMENT_MIN_REST or taken[piece.casefold()] >= per_piece
                    or not term_question(piece, row.before)
                    or int(russian_ending_bucket(piece)) < TERM_FRAGMENT_MIN_ENDING_BUCKET):
                continue
            if refuse(piece, translated(piece, 1)):
                counts["refused"] += 1
                continue
            taken[piece.casefold()] += 1
            counts["keep:piece"] += 1
            counts["keep:piece-revisit"] += 1
            rows.append(frame(f"term-piece:{row.identifier}:{size}", piece, row.before, "keep", piece_weight))
            rows.append(revisit(f"term-piece:{row.identifier}:{size}:revisit", piece, row.before, "keep", piece_weight))
    return rows, {"frames": len(rows), "items": dict(sorted(counts.items())), "pieces": len(taken),
                  "item_modulus": item_modulus, "per_piece": per_piece, "piece_weight": piece_weight, "held_out": len(held_out),
                  "sample_weight": weight,
                  "scope": "terms of the Latin table typed in the Russian layout after Russian prose, at the start of a "
                           "message, with a sign after them, with a capital or in capitals, and asked again after a "
                           "Russian word that went back (convert; alone at a space, a term of two letters waits), and the "
                           "tails of the split's Russian words after a word that are such a token (keep), of this split's "
                           "share (term_curriculum)."}


def revisit_followers() -> list[str]:
    """The TERM_REVISIT_FOLLOWERS most frequent words of Russian prose of up to TERM_REVISIT_FOLLOWER_MAX_LETTERS letters:
    the words after which a term is asked again (term_curriculum, first_curriculum)."""

    return [word for _count, word in sorted((-count, word) for word, count in _term_frequency()["russian"].items()
                                            if len(word) <= TERM_REVISIT_FOLLOWER_MAX_LETTERS and word.isalpha())
            ][:TERM_REVISIT_FOLLOWERS]


def first_curriculum(split: str, source_rows: Sequence[CorpusRow], refused: frozenset[str],
                     options: Mapping[str, object]) -> tuple[list[ActionRow], dict[str, object]]:
    """Terms of up to three letters alone in a field a field reader read empty (context_action_features.first_question).

    There a term opens a message, and `йк ` (`qr`) is to convert at once, where the frozen model waited for a pause. The
    term head, which chooses an early epoch for its other frames, stopped short of the threshold there: `зк` and `йк`
    became suggestions at p 0.98, which no pause settles (candidates v64b to v64e). Every term of the Latin table whose
    keys in the Russian layout are a token of the class stands alone in such a field, ended by a space (convert) and by
    a pause (convert, but keep for a token whose bucket of Russian words beginning with it reaches
    FIRST_PAUSE_WORD_START_MIN_BUCKET: a word stopped in the middle, as `уче` of `учеб`, which became `ext` with its `б`
    a comma, candidate v67). Each is also asked again as the engine asks it with the word after it: a term converted at
    once, once a Russian word followed (ActionRow.revisit: the baseline the engine gives that question), stays converted
    (the term head, which asked it before, took `qr` back with `истек` after it on corpus v65 and kept it on corpus
    v64). Candidates v64c to v66 made a token that may still be the tail of a Russian word wait and keep instead (`дые`,
    `ев`, `шаю`): the field batteries then began every stream in an empty field, also the streams a Backspace or a click
    had cut in the middle of a message. A tail never stands alone in a field read empty: an edit that leaves one as a
    token leaves letters before it, or reopens its word and asks the whole of it. The batteries now begin each stream in
    the field their log shows, and the wait had cost the class 25 tokens a pause converted before (`шь`, `зь`). Each
    term goes to DEVELOPMENT or TRAIN as the term curriculum draws it (TERM_SPLIT_MODULUS), and DEVELOPMENT chooses the
    head's own epoch; the head is fitted on these frames alone (frame_chain). Tokens of this corpus's test, of every
    accessed test and of the recipe's `held_out` list (the class's tokens of the field batteries) are refused.
    """

    if not options.get("enabled") or split not in (TRAIN, DEVELOPMENT):
        return [], {"frames": 0, "scope": "not used"}
    weight = float(cast(float, options["sample_weight"]))
    held_out = frozenset(str(token).casefold() for token in cast(list[object], options.get("held_out", [])))
    applications = ("Telegram", "Code", "chrome", "UnseenEditor")
    followers = revisit_followers()
    rows: list[ActionRow] = []
    counts: Counter[str] = Counter()
    for term in sorted(_term_frequency()["latin"]):
        if not term.isascii() or not term.isalpha() or not term.islower():
            continue
        typed = translated(term, 0)
        family = f"term:{typed}"
        if not first_question(typed, "", ""):
            continue
        if (DEVELOPMENT if variant_choice(family, "split", TERM_SPLIT_MODULUS) == 0 else TRAIN) != split:
            continue
        if term.casefold() in held_out or typed.casefold() in held_out or refused & (expanded_aliases(term) | expanded_aliases(typed)):
            counts["refused"] += 1
            continue
        field = FieldContext(applications[variant_choice(f"first:{typed}", "application", len(applications))], "public-training",
                             "", "", "text", source="reader")
        start = russian_start_bucket(typed)
        stopped: ContextAction = "keep" if start != "na" and int(start) >= FIRST_PAUSE_WORD_START_MIN_BUCKET else "convert"
        for trigger, boundary_text, action in (("space", " ", "convert"), ("pause", "", stopped)):
            counts[f"{action}:{trigger}"] += 1
            rows.append(ActionRow(f"first:{typed}:{trigger}", typed, 1, field, cast(CorrectionTrigger, trigger), "",
                                  cast(ContextAction, action), "first", boundary_text, weight))
        follower = followers[variant_choice(f"first:{typed}", "follower", len(followers))]
        origin: AfterOrigin = "planned_next_conversion" if len(typed) <= PLANNED_REVISIT_MAX_CHARACTERS else "field"
        counts["convert:next"] += 1
        rows.append(ActionRow(f"first:{typed}:next", typed, 1, replace(field, after=follower), "space", "", "convert", "first",
                              " ", weight, after_origin=origin, revisit=True))
    return rows, {"frames": len(rows), "items": dict(sorted(counts.items())), "held_out": len(held_out), "sample_weight": weight,
                  "scope": "terms of up to three letters of the Latin table typed in the Russian layout alone in a field a "
                           "reader read empty, ended by a space and by a pause, and asked again with the word after them "
                           "(convert; one that many Russian words begin with is kept at the pause) (first_curriculum)."}


def brief_curriculum(split: str, source_rows: Sequence[CorpusRow], refused: frozenset[str],
                     options: Mapping[str, object]) -> tuple[list[ActionRow], dict[str, object]]:
    """Short messages typed in the English layout (context_action_features.brief_question): a token of
    BRIEF_HEAD_LETTERS letters alone in the field.

    Typed as a whole message in the English layout, `ytn` (`нет`), `jr` (`ок`), `dct` (`все`) and `tot` (`еще`) stayed:
    the frozen model kept them or waited for a next word, and the lone-word head did not overcome that wait. Each word
    of Russian prose of these lengths counted at least `minimum_count` times, typed with its keys in the English layout,
    and each word of English prose or Latin term of Russian technical text counted as often, typed as meant, stands
    alone at the start of a message, ended by every boundary of BRIEF_BOUNDARIES, in lower case and capitalised. A token
    that is both (`vs` and `мы`) is framed both ways: converted with the share of the Russian reading among the counts of
    both readings, kept with the rest, so the head learns what a message of these keys most likely meant. Only tokens
    the counts settle as a Russian word are framed (context_action_features.brief_settled), the head's class: framed
    all, the head turned pairs both readings count (`ij`, `Jq`) into Russian and kept `ds` (`вы`) at some boundaries,
    where the calibration takes the frozen model's answer (candidate v63e). One token in
    BRIEF_SPLIT_MODULUS goes to DEVELOPMENT by hash, whose frames choose the head's own epoch. Tokens of this corpus's
    test, of every accessed test and of the recipe's `held_out` list (the class's tokens of the owner's typing) are
    refused.
    """

    minimum = int(cast(int, options["minimum_count"]))
    if not minimum or split not in (TRAIN, DEVELOPMENT):
        return [], {"frames": 0, "scope": "not used"}
    weight = float(cast(float, options["sample_weight"]))
    held_out = frozenset(str(token).casefold() for token in cast(list[object], options.get("held_out", [])))
    table = _term_frequency()
    applications = ("Telegram", "Code", "chrome", "UnseenEditor")
    tokens: dict[str, str] = {}
    for word, count in table["russian"].items():
        keys = translated(word, 1)
        if count >= minimum and len(word) in BRIEF_HEAD_LETTERS and keys.isascii() and keys.isalpha():
            tokens[keys.casefold()] = word
    for alphabet in ("english", "latin"):
        for word, count in table[alphabet].items():
            if count >= minimum and len(word) in BRIEF_HEAD_LETTERS and word.isascii() and word.isalpha() and word.islower():
                keys = translated(word, 0)
                if keys.isalpha() and not keys.isascii():
                    tokens.setdefault(word, keys)
    rows: list[ActionRow] = []
    counts: Counter[str] = Counter()
    for latin, cyrillic in sorted(tokens.items()):
        family = f"brief:{latin}"
        if (DEVELOPMENT if variant_choice(family, "split", BRIEF_SPLIT_MODULUS) == 0 else TRAIN) != split:
            continue
        if latin in held_out or cyrillic in held_out or refused & (expanded_aliases(latin) | expanded_aliases(cyrillic)):
            counts["refused"] += 1
            continue
        if not brief_settled(latin, cyrillic):
            # The head answers no token the counts leave open (context_action_features.brief_settled).
            counts["unsettled"] += 1
            continue
        russian = table["russian"].get(cyrillic, 0)
        share = russian / (russian + max(table["english"].get(latin, 0), table["latin"].get(latin, 0)))
        counts["both" if 0 < share < 1 else "russian" if share else "latin"] += 1
        for index, (trigger, boundary_text) in enumerate(BRIEF_BOUNDARIES):
            for case, shown in (("lower", latin), ("title", latin.capitalize())):
                identifier = f"{family}:{index}:{case}"
                field = FieldContext(applications[variant_choice(identifier, "application", len(applications))],
                                     "public-training", "", "", "unknown")
                for action, mass in (("convert", share), ("keep", 1.0 - share)):
                    if mass:
                        rows.append(ActionRow(f"{identifier}:{action}", shown, 0, field, cast(CorrectionTrigger, trigger), "",
                                              cast(ContextAction, action), "brief", boundary_text, weight * mass))
    return rows, {"frames": len(rows), "tokens": dict(sorted(counts.items())), "minimum_count": minimum,
                  "held_out": len(held_out), "sample_weight": weight,
                  "scope": "tokens of two or three letters alone at the start of a message, typed in the English layout, that "
                           "the counts settle as the keys of a Russian word (convert, a share of the Latin count kept) "
                           "(brief_curriculum)."}


def term_counts_dropped(identifier: str, original: str) -> bool:
    """Deterministic: one unknown-token family in TERM_COUNT_DROPOUT_FAMILIES of TERM_COUNT_DROPOUT_MIN_LETTERS letters or
    more is trained with its own term counts dropped (unknown_token_curriculum)."""

    if not identifier.startswith("unknown-") or sum(char.isalpha() for char in original) < TERM_COUNT_DROPOUT_MIN_LETTERS:
        return False
    return variant_choice(identifier.rsplit(":", 1)[0], "count-dropout", TERM_COUNT_DROPOUT_FAMILIES) == 0


def _cyrillic(word: str) -> bool:
    return all("а" <= char.casefold() <= "я" or char.casefold() == "ё" for char in word)


def _monolingual_rows(source_rows: Sequence[CorpusRow]) -> Iterator[tuple[CorpusRow, int, str, list[re.Match[str]]]]:
    """Rows whose word and every word before it are in the script of the row's layout: each with its
    layout, its text through its word and the words of that text."""

    for row in source_rows:
        if row.group not in (0, 1) or not row.layout_representable or not row.original.isalpha():
            continue
        if _cyrillic(row.original) != (row.group == 1):
            continue
        text = row.before + row.original
        tokens = list(WORDS.finditer(text))
        if any(_cyrillic(token.group()) != (row.group == 1) for token in tokens):
            continue
        yield row, row.group, text, tokens


def _russian_contexts(source_rows: Sequence[CorpusRow]) -> list[tuple[str, str]]:
    """The left context and the word of every Russian row of Russian words only, in a fixed order."""

    return sorted((row.before, row.original) for row, group, _, _ in _monolingual_rows(source_rows) if group == 1)


def _counted_terms(refused: frozenset[str], minimum: int) -> list[str]:
    """Latin words Russian technical text uses at least `minimum` times whose Cyrillic reading is nothing.

    Their keys in the Russian layout are no word, no identifier and not counted in Russian text: typed
    there amid Russian prose, they are the term typed in the wrong layout (`зк` is `pr`).
    """

    uncounted = CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS[0]
    table = _term_frequency()
    terms: list[str] = []
    for term, count in table["latin"].items():
        if count < minimum or not term.isascii() or not term.isalpha() or not term.islower():
            continue
        if not KEPT_NEIGHBOUR_MIN_LETTERS <= len(term) <= KEPT_CONTEXT_WORD_MAX_CHARACTERS:
            continue
        reading = translated(term, 0)
        if (not reading.isalpha() or plausible_reading(reading, 1) or table["cyrillic"].get(reading, 0) >= uncounted
                or table["russian"].get(reading, 0) >= uncounted or refused & (expanded_aliases(term) | expanded_aliases(reading))):
            continue
        terms.append(term)
    return terms


def _counted_abbreviations(refused: frozenset[str], minimum: int, dominance: float) -> list[str]:
    """Cyrillic tokens Russian technical text uses at least `minimum` times and `dominance` times as
    often as their Latin keys, no lexicon word: `тз` (744 against 80 for `np`), not `зк`."""

    table = _term_frequency()
    abbreviations: list[str] = []
    for token, count in table["cyrillic"].items():
        if (count < minimum or not token.isalpha() or not _cyrillic(token)
                or not KEPT_NEIGHBOUR_MIN_LETTERS <= len(token) <= KEPT_CONTEXT_WORD_MAX_CHARACTERS
                or plausible_reading(token, 1)):
            continue
        keys = translated(token, 1)
        if count < dominance * table["latin"].get(keys, 0) or refused & (expanded_aliases(token) | expanded_aliases(keys)):
            continue
        abbreviations.append(token)
    return abbreviations


def _term_typed(term: str) -> str:
    """A counted term's keys in the Russian layout, one term in KEPT_NEIGHBOUR_CAPITALS_MODULUS in capitals."""

    capitals = variant_choice("kept-term:" + term, "capitals", KEPT_NEIGHBOUR_CAPITALS_MODULUS) == 0
    return translated(term.upper() if capitals else term, 0)


def kept_neighbour_curriculum(source_rows: Sequence[CorpusRow], refused: frozenset[str],
                              options: Mapping[str, object], models: dict[int, LanguageModel] | None = None,
                              ) -> tuple[list[ActionRow], dict[str, object]]:
    """A waiting word asked once more beside its next word, which stayed as typed (`kept_next_word`).

    The engine converts a waiting word together with its next word, and a term typed in the Russian
    layout amid Russian prose has a next word that is right as typed: `зк` stayed in `есть новые зк
    проверь`, `тзь сш` in `сам выполни тзь сш` (the owner's typing, 0.38 and 0.39). The engine now
    asks once more with the kept word on the right (KeySwitchEngine._decide_with_kept_neighbour), a
    question the frames below teach, under feature names of its own (KEPT_FEATURE_PREFIX):
    - natural: a short word of a TRAIN sentence of either language with the sentence's next word
      after it, both as written - keep;
    - term: a Latin word the packaged term table counts in Russian technical text at least
      `minimum_term_count` times, typed in the Russian layout (its Cyrillic reading no word, no
      identifier and not counted in Russian text), between the left context of a Russian TRAIN row
      and that row's word - convert; one term in KEPT_NEIGHBOUR_CAPITALS_MODULUS in capitals;
    - abbreviation: a Cyrillic token Russian technical text uses at least `minimum_abbreviation_count`
      times and `abbreviation_dominance` times as often as its Latin keys (`тз`, not `зк`), no lexicon
      word, in the same place - keep;
    - misspelt: a Russian lexicon word with one edit of typo_variants, no word in either layout and its
      Latin keys no counted term, in the same place - keep.
    Words of this corpus's test and of every accessed test are refused by their aliases. `models` are
    the reference lexicons without morphology when the caller already holds them.
    """

    budgets = {name: int(cast(int, options[name])) for name in ("natural_frames", "term_frames", "abbreviation_frames", "typo_frames")}
    if not any(budgets.values()):
        return [], {"frames": 0, "scope": "not used"}
    weight = float(cast(float, options["sample_weight"]))
    minimum_term = int(cast(int, options["minimum_term_count"]))
    minimum_abbreviation = int(cast(int, options["minimum_abbreviation_count"]))
    dominance = float(cast(float, options["abbreviation_dominance"]))
    applications = ("Telegram", "Code", "chrome", "UnseenEditor")

    def ranked(values: Iterable[str], purpose: str) -> list[str]:
        return sorted(values, key=lambda value: hashlib.sha256(f"kept-{purpose}:{value}".encode()).digest())

    def frame(identifier: str, word: str, group: int, before: str, after: str, action: ContextAction, category: str) -> ActionRow:
        field = FieldContext(applications[variant_choice(identifier, "application", len(applications))], "public-training",
                             before, after, "unknown")
        return ActionRow(identifier, word, group, field, "space", "", action, category, " ", weight, "kept_next_word")

    natural: dict[int, dict[str, tuple[str, str, str]]] = {0: {}, 1: {}}
    for _, group, text, tokens in _monolingual_rows(source_rows):
        # Every short word of the sentence followed, after one space, by another word is the question
        # asked as written: the row's own text up to its word holds many such pairs.
        for current, following in zip(tokens, tokens[1:]):
            word = current.group()
            if (KEPT_NEIGHBOUR_MIN_LETTERS <= len(word) <= KEPT_CONTEXT_WORD_MAX_CHARACTERS and word.isalpha()
                    and following.group().isalpha() and text[current.end():following.start()] == " "):
                natural[group][text[:current.start()] + "|" + word] = (text[:current.start()], word, following.group())
    russian = ContextRing(_russian_contexts(source_rows))
    rows: list[ActionRow] = []
    counts: Counter[str] = Counter()
    for group in (0, 1):
        for key in ranked(natural[group], f"natural-{group}")[:budgets["natural_frames"] // LAYOUT_GROUP_COUNT]:
            before, word, following_word = natural[group][key]
            identifier = "kept-natural:" + hashlib.sha256(key.encode()).hexdigest()
            rows.append(frame(identifier, word, group, before, following_word, "keep", "kept_neighbour"))
            counts[f"natural_{group}"] += 1

    def inserted(words: list[str], budget: int, purpose: str, typed: Callable[[str], str], action: ContextAction) -> list[str]:
        chosen: list[str] = []
        if not russian:
            return chosen
        for word in words:
            if counts[purpose] >= budget:
                break
            chosen.append(word)
            for index in range(KEPT_NEIGHBOUR_FRAMES_PER_WORD):
                if counts[purpose] >= budget:
                    break
                identifier = f"kept-{purpose}:{word}:{index}"
                before, following_word = russian.choice(identifier, "context")
                rows.append(frame(identifier, typed(word), 1, before, following_word, action, "kept_neighbour_" + purpose))
                counts[purpose] += 1
        return chosen

    terms = _counted_terms(refused, minimum_term)
    chosen_terms = inserted(ranked(terms, "term"), budgets["term_frames"], "term", _term_typed, "convert")
    abbreviations = _counted_abbreviations(refused, minimum_abbreviation, dominance)
    chosen_abbreviations = inserted(ranked(abbreviations, "abbreviation"), budgets["abbreviation_frames"], "abbreviation",
                                    lambda token: token, "keep")
    misspelt: list[str] = []
    for word in (models if models is not None else reference_models(False))[1].frequencies:
        if not _cyrillic(word) or not word.isalpha() or refused & expanded_aliases(word):
            continue
        for variant in typo_variants(word, "kept-typo:" + word):
            if (not KEPT_NEIGHBOUR_MIN_LETTERS <= len(variant) <= KEPT_CONTEXT_WORD_MAX_CHARACTERS
                    or not variant.isalpha() or plausible_reading(variant, 1)):
                continue
            keys = translated(variant, 1)
            if term_bucket(keys, "latin") != "0" or plausible_reading(keys, 0):
                continue
            misspelt.append(variant)
    chosen_misspelt = inserted(ranked(sorted(set(misspelt)), "typo"), budgets["typo_frames"], "typo", lambda word: word, "keep")
    return rows, {"frames": len(rows), "counts": dict(sorted(counts.items())), "sample_weight": weight,
                  "candidates": {"natural_0": len(natural[0]), "natural_1": len(natural[1]), "terms": len(terms),
                                 "abbreviations": len(abbreviations), "misspelt": len(set(misspelt)), "contexts": len(russian)},
                  "terms": chosen_terms, "abbreviations": chosen_abbreviations, "misspelt": chosen_misspelt,
                  "scope": "TRAIN only: kept_next_word frames, the question a waiting word is asked again with its kept next word (kept_neighbour_curriculum)."}


def counted_token_curriculum(source_rows: Sequence[CorpusRow], refused: frozenset[str],
                             options: Mapping[str, object]) -> tuple[list[ActionRow], dict[str, object]]:
    """Counted Russian abbreviations amid Russian prose stay; counted Latin terms typed there convert.

    The word decided at its own boundary tells a token's language by the lexicons and the language
    models, and a Russian abbreviation of technical text is in neither while its Latin keys often are
    a word: `тз` amid Russian prose became `np` at p=0.993 in the owner's typing (`если тз готов`,
    the corpus v27 candidate of 06.10.2026), and so did `2фа`, `кз` and `ви`. How often Russian
    technical text uses a token (744 times for `тз`, 80 for `np`) is a feature already, but no frame of
    this question held such a token, and the count lost to the language models. The frames put the
    tokens of the kept-neighbour curriculum (_counted_abbreviations, _counted_terms) after the left
    context of a Russian TRAIN row, chosen by hash, with nothing after them:
    - abbreviation: the token as typed - keep;
    - term: a counted Latin term's keys in the Russian layout - convert, so the pair tells the counts
      apart rather than teaching that a short unknown Cyrillic token after Russian prose stays.
    Each word gets up to `frames_per_word` frames, each after a different Russian row, with the
    boundaries of the capital citations (_varied_boundary). With `capitals` every token is typed in
    capitals (`ТЗ` keeps, `ФЗШ` for `API` converts): such frames teach the capitals head alone when a
    frozen base keeps every other weight. Words of this corpus's test and of every accessed test are
    refused by their aliases.
    """

    budgets = {name: int(cast(int, options[name])) for name in ("term_frames", "abbreviation_frames")}
    per_word = int(cast(int, options["frames_per_word"]))
    russian = ContextRing(_russian_contexts(source_rows))
    if not any(budgets.values()) or not russian:
        return [], {"frames": 0, "scope": "not used"}
    weight = float(cast(float, options["sample_weight"]))
    applications = ("Telegram", "Code", "chrome", "UnseenEditor")
    candidates = {
        "term": _counted_terms(refused, int(cast(int, options["minimum_term_count"]))),
        "abbreviation": _counted_abbreviations(refused, int(cast(int, options["minimum_abbreviation_count"])),
                                               float(cast(float, options["abbreviation_dominance"]))),
    }
    capitals = options.get("capitals") is True
    plans: tuple[tuple[str, Callable[[str], str], ContextAction], ...] = (
        ("term", (lambda term: translated(term.upper(), 0)) if capitals else _term_typed, "convert"),
        ("abbreviation", str.upper if capitals else str, "keep"))
    rows: list[ActionRow] = []
    chosen: dict[str, list[str]] = {}
    for purpose, typed, action in plans:
        words = sorted(candidates[purpose], key=lambda word: hashlib.sha256(f"counted-{purpose}:{word}".encode()).digest())
        chosen[purpose] = []
        frames = 0
        for word in words:
            if frames >= budgets[purpose + "_frames"]:
                break
            chosen[purpose].append(word)
            for index in range(min(per_word, budgets[purpose + "_frames"] - frames)):
                identifier = f"counted-{purpose}:{word}:{index}"
                trigger, boundary_text = _varied_boundary(identifier)
                before, _ = russian.choice(identifier, "context")
                field = FieldContext(applications[variant_choice(identifier, "application", len(applications))],
                                     "public-training", before, "", "unknown")
                rows.append(ActionRow(identifier, typed(word), 1, field, trigger, "", action, "counted_" + purpose,
                                      boundary_text, weight))
                frames += 1
    return rows, {"frames": len(rows), "counts": dict(sorted(Counter(row.category for row in rows).items())),
                  "candidates": {purpose: len(words) for purpose, words in candidates.items()},
                  "sample_weight": weight, "frames_per_word": per_word, "capitals": capitals,
                  "terms": chosen["term"], "abbreviations": chosen["abbreviation"],
                  "scope": "TRAIN only: counted Russian abbreviations (keep) and counted Latin terms typed in the Russian layout (convert) after Russian prose, decided at their own boundary (counted_token_curriculum)."}


def _technical_terms(refused: frozenset[str], minimum: int) -> list[str]:
    """Latin words of letters that Russian technical text uses at least `minimum` times and at least as
    often as English prose does (`redis`, `todo`, `docker`; not `plan` or `the`), in a fixed order."""

    table = _term_frequency()
    return sorted(term for term, count in table["latin"].items()
                  if count >= minimum and count >= table["english"].get(term, 0) and len(term) > 1
                  and term.isascii() and term.isalpha() and term.islower() and not refused & expanded_aliases(term))


def letter_curriculum(source_rows: Sequence[CorpusRow], refused: frozenset[str],
                      options: Mapping[str, object]) -> tuple[list[ActionRow], dict[str, object]]:
    """A word of one letter right after a word of Latin letters, the single-letter head's class
    (context_action_features.letter_question), at its boundary and beside its next word.

    Russian technical text puts a Russian word of one letter after a Latin term (`nats и redis`), and
    typed in the English layout the letter is a Latin one: the owner's `nats b redis`, `lid f номер`,
    `todo b` (0.32-0.41). No corpus frame stands there: the corpus's Russian sentences have Cyrillic
    words before their letters, and the frozen model kept `f` before the converted `номер` at p=1.00.
    The frames, by sentence chosen by hash:
    - term: a Russian TRAIN sentence's word of one letter (а и в с к у о я) and the word after it, the
      word before the letter replaced by a counted technical term (_technical_terms), or the term alone
      before it in one sentence in LETTER_CURRICULUM_MESSAGE_START_MODULUS. Typed in the English layout
      the letter waits at its boundary and converts beside its next word, converted (`номер`) or kept as
      typed (another counted term); typed as written it keeps in all three places.
    - english: an English TRAIN sentence's word between two words, replaced by the Latin keys of one of
      those Russian letters (`plan b then`), waits at its boundary and keeps beside its next word kept as
      typed; an English word of one letter (`a`, `I`) keeps at its boundary and beside it.
    Terms and words of this corpus's test and of every accessed test are refused by their aliases.
    """

    budgets = {name: int(cast(int, options[name])) for name in ("term_sentences", "english_sentences")}
    terms = ContextRing(_technical_terms(refused, int(cast(int, options["minimum_term_count"]))))
    if not any(budgets.values()) or not terms:
        return [], {"frames": 0, "scope": "not used"}
    weight = float(cast(float, options["sample_weight"]))
    applications = ("Telegram", "Code", "chrome", "UnseenEditor")
    letters = sorted(translated(letter, 1) for letter in TRUSTED_SINGLE_LETTER_WORDS)

    def frame(identifier: str, word: str, group: int, before: str, after: str, action: ContextAction,
              category: str, origin: AfterOrigin) -> ActionRow:
        field = FieldContext(applications[variant_choice(identifier, "application", len(applications))], "public-training",
                             before, after, "unknown")
        return ActionRow(identifier, word, group, field, "space", "", action, category, " ", weight, origin)

    sentences: dict[int, dict[str, tuple[str, str, str, str]]] = {0: {}, 1: {}}
    for _, group, text, tokens in _monolingual_rows(source_rows):
        for previous, current, following in zip(tokens, tokens[1:], tokens[LETTER_CONTEXT_TOKENS - 1:]):
            word, after = current.group(), following.group()
            if (text[previous.end():current.start()] != " " or text[current.end():following.start()] != " "
                    or not word.isalpha() or not after.isalpha() or len(after) > PLANNED_CONTEXT_AFTER_MAX_CHARACTERS
                    or refused & (expanded_aliases(word) | expanded_aliases(after))):
                continue
            if group == 1 and word in TRUSTED_SINGLE_LETTER_WORDS:
                sentences[1][text[:current.start()] + "|" + word] = (text[:previous.start()], word, after, "")
            elif group == 0 and previous.group().isalpha():
                sentences[0][text[:current.start()] + "|" + word] = (text[:previous.start()], previous.group(), word, after)
    rows: list[ActionRow] = []
    counts: Counter[str] = Counter()

    def ranked(values: Iterable[str], purpose: str) -> list[str]:
        return sorted(values, key=lambda value: hashlib.sha256(f"letter-{purpose}:{value}".encode()).digest())

    for key in ranked(sentences[1], "term")[:budgets["term_sentences"]]:
        prose, word, after, _ = sentences[1][key]
        identifier = "letter-term:" + hashlib.sha256(key.encode()).hexdigest()
        term = terms.choice(identifier, "term")
        other = terms.choice(identifier, "next-term")
        start = variant_choice(identifier, "message-start", LETTER_CURRICULUM_MESSAGE_START_MODULUS) == 0
        before = term + " " if start else prose + term + " "
        keys = translated(word, 1)
        rows.extend((
            frame(identifier + ":own", keys, 0, before, "", "wait", "letter_term", "none"),
            frame(identifier + ":planned", keys, 0, before, after, "convert", "letter_term", "planned_next_conversion"),
            frame(identifier + ":kept", keys, 0, before, other, "convert", "letter_term", "kept_next_word"),
            frame(identifier + ":own-keep", word, 1, before, "", "keep", "letter_term_kept", "none"),
            frame(identifier + ":planned-keep", word, 1, before, other, "keep", "letter_term_kept", "planned_next_conversion"),
            frame(identifier + ":kept-keep", word, 1, before, after, "keep", "letter_term_kept", "kept_next_word"),
        ))
        counts["term_sentences"] += 1
        counts["message_start"] += int(start)
    for key in ranked(sentences[0], "english")[:budgets["english_sentences"]]:
        prose, previous_word, word, after = sentences[0][key]
        identifier = "letter-english:" + hashlib.sha256(key.encode()).hexdigest()
        before = prose + previous_word + " "
        if len(word) == 1:
            rows.extend((frame(identifier + ":own", word, 0, before, "", "keep", "letter_english_word", "none"),
                         frame(identifier + ":kept", word, 0, before, after, "keep", "letter_english_word", "kept_next_word")))
            counts["english_letters"] += 1
            continue
        letter = letters[variant_choice(identifier, "letter", len(letters))]
        rows.extend((
            frame(identifier + ":own", letter, 0, before, "", "wait", "letter_english", "none"),
            frame(identifier + ":kept", letter, 0, before, after, "keep", "letter_english", "kept_next_word"),
        ))
        counts["english_sentences"] += 1
    return rows, {"frames": len(rows), "counts": dict(sorted(counts.items())), "sample_weight": weight,
                  "candidates": {"terms": len(terms), "russian_sentences": len(sentences[1]), "english_sentences": len(sentences[0])},
                  "scope": "TRAIN only: a word of one letter after a counted technical term in Russian sentences and in place of a "
                           "word of English ones, at its boundary and beside its next word (letter_curriculum)."}


def historical_curriculum(intent: LinearNgramModel | None = None,
                          models: dict[int, LanguageModel] | None = None) -> list[ActionRow]:
    """Reuse distinct old TRAIN situations, never its evaluation labels.

    A physical family identifies the split, not the language decision. Keep
    its different labels, neighbours, applications and field roles. Remove
    only repeated identical frames. Completed-word lookahead mirrors the
    engine's second decision; the original longer field context also remains.
    `models` are the reference lexicons with morphology when the caller already holds them.
    """
    from train_context_model import build_corpus

    if intent is None:
        intent = LinearNgramModel.load(ROOT / "src/keyswitch/resources/models/layout_intent_v1.ksm")
    if models is None:
        models = reference_models(True)
    lexicons = models
    status = IntentModelStatus(True, ROOT / "src/keyswitch/resources/models/layout_intent_v1.ksm",
                               intent.model_version, intent.checksum, None)

    def load(locale: str, extra_words: Iterable[str] = ()) -> LanguageModel:
        # The reference lexicons already carry the packaged supplements the engine
        # would pass here (build_corpus: LanguageModel.load(locale, supplement_words(locale))).
        return lexicons[0 if locale == "en_US" else 1]

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
    from train_context_model import captured_sources

    result: list[ActionRow] = []
    report: dict[str, object] = {}
    for source in captured_sources(ROOT / str(options["manifest"])):
        rows, report[source.path.name] = captured_source_curriculum(source, options)
        result.extend(rows)
    return result, report


def captured_source_curriculum(source: CapturedSource, options: Mapping[str, object]) -> tuple[list[ActionRow], dict[str, object]]:
    """The frames of one captured question file and its report; each file is chosen on its own."""
    from train_context_model import captured_rows

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
    name = source.path.name
    if any(marker in name for marker in excluded):
        return [], {"excluded": True}
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
    return result, {"available": {action: len(rows) for action, rows in ranked.items()}, "skipped_short": skipped_short,
                    "unknown_keep_available": len(unknown), "unknown_keep_chosen": len(chosen_unknown),
                    "chosen": dict(counts), "weight": source.weight * weight_scale}


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
    """Aliases of every word a sealed test holds: this corpus's test and every accessed one.

    The ledger of accessed tests lives outside the repository (LEDGER_ROOT). A corpus records how
    many test aliases it was frozen against; a fit that would refuse fewer has no ledger, or an older
    one, and stops rather than frame words of a test already read.
    """
    membership = cast(dict[str, object], json.loads((corpus / "test-membership.json").read_bytes()))
    aliases = set(cast(list[str], membership.get("alias_sha256", [])))
    ledger, _ = ledger_test_aliases(LEDGER_ROOT)
    refused = frozenset(aliases | ledger)
    metadata = cast(dict[str, object], json.loads((corpus / "manifest.json").read_bytes()).get("metadata", {}))
    frozen_against = metadata.get("refused_test_aliases")
    if isinstance(frozen_against, int) and len(refused) < frozen_against:
        raise ValueError(f"the fit would refuse {len(refused)} test aliases and the corpus was frozen refusing {frozen_against}: "
                         f"the test ledger ({LEDGER_ROOT}) is missing or older than the corpus")
    return refused


def lexical_short_pairs(refused: frozenset[str], models: dict[int, LanguageModel] | None = None) -> list[tuple[str, str, int]]:
    """Three-letter words of the pinned lexicons whose other reading is a word or a command too.

    Both readings must be real: a form of the onboard lexicon (the OpenSubtitles supplement
    marks a form as known for the deferral rule, but it also holds one-off noise, and a
    conversion into such a form is never taught) or an entry of the shipped identifier
    index. A pair whose word is held by a sealed test stays out, by the same aliases the
    fitting extension refuses. `models` are the reference lexicons without morphology when the
    caller already holds them.
    """
    if models is None:
        models = reference_models(False)
    lexicons = models
    supplement = {0: frozenset(supplement_words("en_US")), 1: frozenset(supplement_words("ru_RU"))}

    def real(text: str, group: int) -> bool:
        return (text not in supplement[group] and lexicons[group].score(text).known) or (
            plausible_reading(text, group) and not lexicons[group].score(text).known)

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
    models: dict[int, LanguageModel] | None = None,
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
    pairs = lexical_short_pairs(refused, models)
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


def following_reading(row: ActionRow) -> str:
    """What was typed right after the word, as the other layout prints those keys (automatic_word_decision).

    The engine reads the same keys from its key events; a glyph no key of the pair prints is passed
    as it is.
    """
    typed = (row.literal_tail + row.boundary_text)[:1]
    try:
        return translated(typed, row.group)
    except ValueError:
        return typed


def evidence(row: ActionRow, detector: LanguageDetector, ortho: OrthoModel | None,
             *, identifiers: IdentifierLexicon | None = None) -> ContextEvidence:
    alternative = translated(row.original, row.group)
    previous, context_group = previous_context(row)
    if row.after_origin == "planned_next_conversion" and not row.revisit:
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
        following={1 - row.group: following_reading(row)},
    )
    if row.revisit:
        # A converted word asked again keeps the verdict of its own boundary with no conversion, and no curated rule
        # answers (engine._revert_with_next_word): the term head learnt the planned question instead, and `зк` in
        # `принимай зк и`, converted at once, went back with `и` (field batteries, experiment E9).
        decision = replace(decision, should_convert=False)
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
        "tools/train_context_model.py", "model/context_v1/captured/manifest.json", "model/context_v3/captured/manifest.json",
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
    # The back ends build the same candidate; their code is pinned all the same.
    result.update({path: checksum(ROOT / path) for path in CONTEXT_ACTION_BACK_END_SOURCES})
    for key in ("frozen_base", "warm_base"):
        base = cast(dict[str, object], json.loads(RECIPE.read_bytes())).get(key)
        if isinstance(base, dict):
            for named in (base, base.get("labels")):
                if isinstance(named, dict):
                    result[str(named["artifact"])] = checksum(ROOT / str(named["artifact"]))
    return result


def frozen_base(options: Mapping[str, object]) -> ContextModel | None:
    """The action model a kept-neighbour head is fitted onto, or None for a full fit.

    The word decided at its own boundary moves with every fit: two fits of one recipe on corpora
    a few dozen TRAIN rows apart (v26, v27, v28) turned different borderline words across the 0.99
    threshold (`тз`, `гг.`, the `ша` before `hello`), more than any curriculum under test changed.
    The kept-neighbour question has feature names of its own (KEPT_FEATURE_PREFIX) and the
    optimizer touches only the weights of the features a row has, so its head can be fitted onto a
    model already sealed and shipped, whose every other weight and threshold stay as they are. The
    recipe names that model's file and its SHA-256; the kept-neighbour question and heads it already
    carries stay frozen with it (carried_prefixes).
    """

    return _named_base(options, "frozen_base")


# The heads a frozen base can carry besides the kept-neighbour question, by the recipe's name.
HEAD_PREFIXES: Final = {"capitals": CAPITALS_FEATURE_PREFIX, "alone": ALONE_FEATURE_PREFIX, "start": START_FEATURE_PREFIX,
                         "letter": LETTER_FEATURE_PREFIX, "abbreviation": ABBREVIATION_FEATURE_PREFIX,
                         "latin_abbreviation": LATIN_ABBREVIATION_FEATURE_PREFIX, "unknown": UNKNOWN_FEATURE_PREFIX,
                         "joined": JOINED_FEATURE_PREFIX, "term": TERM_FEATURE_PREFIX, "brief": BRIEF_FEATURE_PREFIX,
                         "first": FIRST_FEATURE_PREFIX, "variable": VARIABLE_FEATURE_PREFIX, "dotted": DOTTED_FEATURE_PREFIX,
                         "alnum": ALNUM_FEATURE_PREFIX}
# A recipe without single-letter frames (the recipes before corpus v34).
NO_LETTER_CURRICULUM: Final[dict[str, object]] = {"term_sentences": 0, "english_sentences": 0, "minimum_term_count": 0,
                                                  "sample_weight": 1.0}
# A recipe without abbreviation frames (the recipes before corpus v36).
NO_ABBREVIATION_CURRICULUM: Final[dict[str, object]] = {"frames_per_form": 0, "minimum_latin_count": 0, "dominance": 1.0,
                                                        "sample_weight": 1.0}
# A recipe without Latin-context abbreviation frames (the recipes before corpus v38).
NO_LATIN_ABBREVIATION_CURRICULUM: Final[dict[str, object]] = {"frames_per_form": 0, "minimum_cyrillic_count": 0, "dominance": 1.0,
                                                              "minimum_term_count": 0, "sample_weight": 1.0}
# A recipe without unknown-token frames (the recipes before corpus v57).
NO_UNKNOWN_TOKEN_CURRICULUM: Final[dict[str, object]] = {"item_modulus": 0, "echo_modulus": 0, "minimum_term_count": 0,
                                                         "held_out": [], "sample_weight": 1.0}
# A recipe without joined-token frames (the recipes before corpus v59).
NO_JOINED_TOKEN_CURRICULUM: Final[dict[str, object]] = {"item_modulus": 0, "per_token": 0, "held_out": [], "sample_weight": 1.0}
# A recipe without term frames (the recipes before corpus v62).
NO_TERM_CURRICULUM: Final[dict[str, object]] = {"item_modulus": 0, "per_piece": 0, "held_out": [], "sample_weight": 1.0}
# A recipe without short-message frames (the recipes before corpus v62).
NO_BRIEF_CURRICULUM: Final[dict[str, object]] = {"minimum_count": 0, "held_out": [], "sample_weight": 1.0}
# A recipe without first-term frames (the recipes before corpus v64).
NO_FIRST_CURRICULUM: Final[dict[str, object]] = {"enabled": False, "held_out": [], "sample_weight": 1.0}
# A recipe without variable or dotted-abbreviation frames (the recipes before corpus v69).
NO_CAPTURED_HEAD_CURRICULUM: Final[dict[str, object]] = {"captured": {}, "sample_weight": 1.0}


def captured_head_curriculum(split: str, options: Mapping[str, object], question: Callable[[ContextEvidence], bool],
                             category: str) -> tuple[list[ActionRow], dict[str, object]]:
    """A head's frames of one split: the questions about tokens of its class the engine asked while public text was typed
    the four ways a person types it (tools/mixed_typing.py capture), from the captured files the recipe names for the
    split, each pinned by SHA-256 in its manifest. The messages of the 2023 ru.stackoverflow dump were chosen for the
    class and split by question (tools/select_capture_messages.py), with every question of the developer-chat messages
    left out; the other files are the train parts of their sources and serve TRAIN alone. No sealed test draws from these
    sources, and no test alias is refused: a letter or an abbreviation of the class is no item a test holds."""

    named = cast(dict[str, list[list[str]]], options.get("captured", {})).get(split, [])
    if not named:
        return [], {"frames": 0, "scope": "not used"}
    from train_context_model import captured_rows, captured_sources

    weight = float(cast(float, options["sample_weight"]))
    # A recipe may weigh the converted frames apart (their mass against the kept frames', which weigh keep_importance
    # times as much in the fit) and keep at most token_cap frames of one token with one answer.
    convert_weight = float(cast(float, options.get("convert_weight", 1.0)))
    token_cap = int(cast(int, options.get("token_cap", 0)))
    applications = ("Telegram", "Code", "chrome", "UnseenEditor")
    rows: list[ActionRow] = []
    counts: Counter[str] = Counter()
    for manifest, name in named:
        source = next(item for item in captured_sources(ROOT / manifest) if item.path.name == name)
        for item, label, _count in captured_rows(source.path):
            action = ACTIONS[label]
            if action not in ("keep", "convert") or not question(item):
                continue
            origin = item.after_origin
            if origin == "planned_next_conversion" and len(item.original) > PLANNED_CONTEXT_WORD_MAX_CHARACTERS:
                origin = "field"
            if origin == "planned_next_conversion" and WORDS.match(item.field.after.lstrip()) is None:
                # As in captured_source_curriculum: the engine plans a next word only once one was typed.
                counts["skipped_planned_without_word"] += 1
                continue
            key = hashlib.sha256(canonical([category, name, item.original, item.source_group, item.trigger, item.literal_tail,
                                            item.boundary_text, origin, item.field.before, item.field.after,
                                            item.field.role])).hexdigest()
            identifier = f"{category}:{key}"
            field = FieldContext(applications[variant_choice(identifier, "application", len(applications))], "captured",
                                 item.field.before, item.field.after, item.field.role)
            counts[f"{name}:{action}"] += 1
            rows.append(ActionRow(identifier, item.original, item.source_group, field, cast(CorrectionTrigger, item.trigger),
                                  item.literal_tail, action, category, item.boundary_text,
                                  weight * (convert_weight if action == "convert" else 1.0), origin))
    report: dict[str, object] = {"frames": len(rows), "by_file": dict(sorted(counts.items())), "sample_weight": weight}
    if token_cap:
        # At most token_cap frames of one token with one answer, in the order of their identifiers: `1с`, written
        # 1 363 times in the 2023 dump, held more kept frames of the letters-and-digits head than all its Latin tokens
        # of one letter held converted ones (candidate v72).
        taken: Counter[tuple[str, str]] = Counter()
        capped: list[ActionRow] = []
        for row in sorted(rows, key=lambda row: row.identifier):
            token = (row.original.casefold(), row.action)
            taken[token] += 1
            if taken[token] <= token_cap:
                capped.append(row)
        report.update({"frames": len(capped), "capped": len(rows) - len(capped), "token_cap": token_cap})
        rows = capped
    if convert_weight != 1.0:
        report["convert_weight"] = convert_weight
    return rows, report


def variable_curriculum(split: str, source_rows: Sequence[CorpusRow], refused: frozenset[str],
                        options: Mapping[str, object]) -> tuple[list[ActionRow], dict[str, object]]:
    """The variable head's frames (captured_head_curriculum): a letter of the Russian layout after a word of Cyrillic
    letters (variable_question), converted where a Latin variable was typed in the Russian layout (`ч` for `x`) and kept
    where the letter was meant as written (`т.` of `и т. д.`, chat of UD Taiga)."""

    return captured_head_curriculum(
        split, options, lambda item: variable_question(item.original, item.alternative, item.field.before, item.trigger),
        "variable")


def dotted_curriculum(split: str, source_rows: Sequence[CorpusRow], refused: frozenset[str],
                      options: Mapping[str, object]) -> tuple[list[ActionRow], dict[str, object]]:
    """The dotted-abbreviation head's frames (captured_head_curriculum): a token typed in the English layout whose
    Cyrillic reading is short letter groups joined by dots (dotted_question), converted where a Russian abbreviation was
    typed in the wrong layout (`n/r/` for `т.к.`) and kept where a Latin token of short parts was meant (`n/a`)."""

    return captured_head_curriculum(split, options, lambda item: dotted_question(item.original, item.alternative), "dotted")


def alnum_curriculum(split: str, source_rows: Sequence[CorpusRow], refused: frozenset[str],
                     options: Mapping[str, object]) -> tuple[list[ActionRow], dict[str, object]]:
    """The letters-and-digits head's frames (captured_head_curriculum): a token of Russian letters and digits after
    Russian text whose keys are Latin letters and digits (alnum_question), converted where a Latin token was typed in the
    Russian layout (`ч64` for `x64`) and kept where it was meant as typed (`1с`, `100мс`)."""

    return captured_head_curriculum(split, options, lambda item: alnum_question(item.original, item.alternative, item.field.before),
                                    "alnum")


# The heads whose own frames of DEVELOPMENT choose their epoch (fit), by feature prefix: the name their epoch goes by in
# the seal, the recipe's key of their curriculum, the curriculum, and the options of a recipe without it.
OWN_EPOCH_HEADS: Final[dict[str, tuple[str, str, Callable[[str, Sequence[CorpusRow], frozenset[str], Mapping[str, object]],
                                                          tuple[list[ActionRow], dict[str, object]]], dict[str, object]]]] = {
    JOINED_FEATURE_PREFIX: ("joined", "joined_token_curriculum", joined_token_curriculum, NO_JOINED_TOKEN_CURRICULUM),
    TERM_FEATURE_PREFIX: ("term", "term_curriculum", term_curriculum, NO_TERM_CURRICULUM),
    BRIEF_FEATURE_PREFIX: ("brief", "brief_curriculum", brief_curriculum, NO_BRIEF_CURRICULUM),
    FIRST_FEATURE_PREFIX: ("first", "first_curriculum", first_curriculum, NO_FIRST_CURRICULUM),
    VARIABLE_FEATURE_PREFIX: ("variable", "variable_curriculum", variable_curriculum, NO_CAPTURED_HEAD_CURRICULUM),
    DOTTED_FEATURE_PREFIX: ("dotted", "dotted_curriculum", dotted_curriculum, NO_CAPTURED_HEAD_CURRICULUM),
    ALNUM_FEATURE_PREFIX: ("alnum", "alnum_curriculum", alnum_curriculum, NO_CAPTURED_HEAD_CURRICULUM),
}


def frozen_heads(options: Mapping[str, object]) -> dict[str, int]:
    """The heads fitted onto a frozen base over every TRAIN frame: feature prefix -> feature budget.

    A head answers a class of its own under feature names of its own (the class's features once more
    under its prefix, context_action_features): fitted with the base's every weight fixed, it moves no
    decision outside its class. Without heads a frozen base is fitted with the kept-neighbour question
    alone, and TRAIN holds the kept-neighbour frames only.
    """

    value = options.get("frozen_base")
    if value is None:
        return {}
    heads = cast(dict[str, dict[str, int]], cast(dict[str, object], value).get("heads", {}))
    return {HEAD_PREFIXES[name]: int(head["maximum_features"]) for name, head in heads.items()}


def alone_evidence(options: Mapping[str, object]) -> dict[str, float] | None:
    """The counts that label lone two-letter frames for the lone-word head, or None without that head.

    A frozen base that carries the head (the corpus v33 model) names its counts under `carried`: its
    frames are labelled as they were when it was fitted, so the calibration reads the same labels."""

    value = cast(dict[str, object], options.get("frozen_base") or {})
    heads = cast(dict[str, dict[str, float]], value.get("heads", {}))
    carried = cast(dict[str, dict[str, float]], value.get("carried", {}))
    return heads.get("alone") or carried.get("alone")


def alone_labels(rows: Sequence[ActionRow], evidence: Mapping[str, float] | None,
                 decide: Callable[[ActionRow], ContextAction | None] | None = None) -> list[ActionRow]:
    """The frames of a split with every deferred frame of the lone-word head's class labelled.

    A token of two letters with no word on either side has no observable intent label in the lexicon
    (context_deferral), and every such frame waits (`wait`, or `suggest` where the boundary acts).
    The term counts the model reads can still settle one: `гш` alone is `ui` (counted_reading). With
    the lone-word head such a frame at a boundary that ends it (alone_question) is labelled by the
    counts - convert for the weaker reading, keep for the stronger. One the counts do not settle takes
    the labelling base's own answer (`decide`, labelling_base), so the head moves nothing it has no
    evidence for: `ye` sent alone is `ну` as before, and `ой` (counted in Russian prose and `jq` in
    technical text) is not turned into Latin. Without an answer it waits as before.
    """

    if evidence is None:
        return list(rows)
    result: list[ActionRow] = []
    for row in rows:
        if row.action in ("wait", "suggest") and row.after_origin != "planned_next_conversion":
            alternate = translated(row.original, row.group)
            if alone_question(row.original, alternate, row.field.before, row.field.after, row.trigger):
                strong = counted_reading(row.original, alternate, row.group, minimum=int(evidence["minimum_count"]),
                                         other_maximum=int(evidence["other_maximum_count"]),
                                         dominance=float(evidence["dominance"]))
                own = None if strong is not None or decide is None else decide(row)
                if strong is not None:
                    row = replace(row, action="keep" if strong == row.group else "convert")
                elif own is not None:
                    row = replace(row, action=own)
        result.append(row)
    return result


def lone_word_curriculum(split: str, refused: frozenset[str], evidence: Mapping[str, float],
                         decide: Callable[[ActionRow], ContextAction | None] | None = None,
                         ) -> tuple[list[ActionRow], dict[str, object]]:
    """Every pair of two Latin letters and its Russian keys, each sent alone, for the lone-word head.

    Corpus sentences open with few words of two letters (corpus v30 TRAIN: 183 such lone frames), and
    the head needs both kinds: readings the term counts settle (`гш` is `ui`, `yf` is `на`) and
    readings they do not (`ns` and `ты` are both counted; `ha` is an English word). Each pair goes to
    one split by hash (LONE_WORD_SPLIT_MODULUS), so DEVELOPMENT chooses the head's epoch on pairs it
    never saw. Each reading ends at every boundary other than a space (LONE_WORD_BOUNDARIES) in lower
    case, capitalised and in capitals: with three frames a reading, the head of candidate B31 learned
    the frozen model's answer for `lf` at Enter and a pause and lost it for `Lf?`. alone_labels labels
    them, with `decide` for the pairs the counts leave open. Pairs with an alias of this corpus's test
    or of any accessed test are refused.
    """

    applications = ("Telegram", "Code", "chrome", "UnseenEditor")
    shares = {0: DEVELOPMENT, 1: CALIBRATION}
    rows: list[ActionRow] = []
    counts: Counter[str] = Counter()
    for first, second in product(string.ascii_lowercase, repeat=ALONE_HEAD_LETTERS):
        latin = first + second
        cyrillic = translated(latin, 0)
        if not cyrillic.isalpha():
            continue
        if shares.get(variant_choice("lone-word:" + latin, "split", LONE_WORD_SPLIT_MODULUS), TRAIN) != split:
            continue
        if refused & (expanded_aliases(latin) | expanded_aliases(cyrillic)):
            counts["refused"] += 1
            continue
        counts["pairs"] += 1
        for group, reading in ((0, latin), (1, cyrillic)):
            for typed in (reading, reading.capitalize(), reading.upper()):
                for trigger, boundary_text in LONE_WORD_BOUNDARIES:
                    identifier = f"lone-word:{typed}:{trigger}:{boundary_text!r}"
                    field = FieldContext(applications[variant_choice(identifier, "application", len(applications))],
                                         "public-training", "", "", "unknown")
                    action: ContextAction = "wait" if trigger == "pause" else "suggest"
                    rows.append(ActionRow(identifier, typed, group, field, cast(CorrectionTrigger, trigger), "", action,
                                          "lone_word", boundary_text, float(evidence["sample_weight"])))
    rows = alone_labels(rows, evidence, decide)
    counts.update(row.action for row in rows)
    return rows, {"frames": len(rows), "counts": dict(sorted(counts.items())), "sample_weight": float(evidence["sample_weight"]),
                  "scope": "lone_word frames of this split: every pair of two Latin letters whose Russian keys are letters too, "
                           "each reading alone in the field at a boundary other than a space (lone_word_curriculum), "
                           "labelled by alone_labels with the labelling base's answer where the counts settle nothing."}


_BASE_ACTIONS: dict[str, ContextModel] = {}


def base_action(row: ActionRow, inputs: FitInputs) -> ContextAction | None:
    """The labelling base's own action on a frame (labelling_base), or None when the profiles' answers differ."""

    spec = cast(dict[str, object], inputs.options["frozen_base"])
    key = str(cast(dict[str, object], spec.get("labels") or spec)["sha256"])
    if key not in _BASE_ACTIONS:
        _BASE_ACTIONS[key] = labelling_base(inputs.options)
    base = _BASE_ACTIONS[key]
    actions = {base.predict(evidence(row, inputs.detectors[profile], inputs.ortho)).action for profile in inputs.profiles}
    return actions.pop() if len(actions) == 1 else None


def warm_base(options: Mapping[str, object]) -> ContextModel | None:
    """The action model a full fit starts from, or None to start from zero weights.

    A fit from zero weights turns borderline words across the 0.99 threshold whatever the curricula
    under test do (frozen_base): the corpus v29 candidates that let `BP` stay in English prose
    converted words the corpus v26 pair had kept or left words it had converted. Started from that
    pair's weights, over its own vocabulary of the word decided at its boundary (the kept-neighbour
    question selects its features as before), at the recipe's `learning_rate` for this start, the
    fit moves the weights the new frames pull and leaves the rest near where they were. Every
    weight trains, and the epoch and threshold are chosen as in any full fit. The recipe names the
    model's file and its SHA-256; it must be an action model with no kept-neighbour weights.
    """

    return _named_base(options, "warm_base")


def labelling_base(options: Mapping[str, object]) -> ContextModel:
    """The model whose answers label the lone-word frames the counts leave open (alone_labels): the frozen base, or the
    model the recipe names under `frozen_base.labels`.

    The carried lone-word head was fitted onto the corpus v37 model with its frames labelled by that model's answers, in
    every split, CALIBRATION too. A frozen base that carries more heads (a shipped pair) answers some of those frames
    otherwise (its short-message head reads Latin letters alone), and labelled by it the calibration would change
    with the base: the recipe keeps the model the head was fitted with as the one that labels.
    """

    spec = cast(dict[str, object], options["frozen_base"])
    labels = spec.get("labels")
    return _pinned_model(cast(dict[str, object], labels), "labelling base") if labels is not None else cast(ContextModel, frozen_base(options))


def frozen_record(options: Mapping[str, object], base: ContextModel) -> dict[str, object]:
    """The seal's record of the frozen base, and of the model that labels the lone-word frames when the recipe names one."""

    spec = cast(dict[str, object], options["frozen_base"])
    record: dict[str, object] = {"artifact": spec["artifact"], "sha256": spec["sha256"], "model_version": base.version,
                                 "conversion_threshold": base.conversion_threshold}
    labels = spec.get("labels")
    if labels is not None:
        record["labels"] = {**cast(dict[str, object], labels), "model_version": labelling_base(options).version}
    return record


def _pinned_model(spec: Mapping[str, object], label: str) -> ContextModel:
    """The action model of the file a recipe names, whose bytes its SHA-256 pins."""

    path = ROOT / str(spec["artifact"])
    if checksum(path) != spec["sha256"]:
        raise ValueError(f"{label} artifact differs from the recipe")
    model = ContextModel.load(path)
    if model.feature_version != CONTEXT_ACTION_FEATURE_VERSION:
        raise ValueError(f"a {label} is an action model")
    return model


def _named_base(options: Mapping[str, object], key: str) -> ContextModel | None:
    value = options.get(key)
    if value is None:
        return None
    spec = cast(dict[str, object], value)
    label = key.replace("_", " ")
    model = _pinned_model(spec, label)
    carried = carried_prefixes(model)
    if key == "warm_base" and carried:
        raise ValueError(f"a {label} is an action model without kept-neighbour weights or heads")
    if key == "frozen_base" and carried & set(frozen_heads(options)):
        raise ValueError("a head the frozen base carries is frozen with it, not fitted again")
    if key == "frozen_base" and any(HEAD_PREFIXES[name] not in carried for name in cast(dict[str, object], spec.get("carried", {}))):
        raise ValueError("the recipe names a carried head the frozen base has no weights for")
    return model


def trainable_prefixes(options: Mapping[str, object]) -> frozenset[str] | None:
    """The feature prefixes a fit onto a frozen base with heads can move, or None for any other fit.

    Every weight of the base is fixed, so only the heads the recipe names and the kept-neighbour
    question, unless the base carries it, take a step: a TRAIN frame outside their classes holds no
    feature the fit can move (trainable_frames).
    """

    heads = frozen_heads(options)
    if not heads:
        return None
    base = cast(ContextModel, frozen_base(options))
    return frozenset(heads) | (frozenset({KEPT_FEATURE_PREFIX}) - carried_prefixes(base))


def head_question(original: str, alternative: str, before: str, after: str, trigger: str, origin: str,
                  prefixes: frozenset[str]) -> bool:
    """Whether the features of a frame (extract_action_features) hold one of these prefixes: the frame
    asks a question of a head or of the kept-neighbour question they name."""

    letter = LETTER_FEATURE_PREFIX in prefixes and letter_question(original, alternative, before)
    variable = VARIABLE_FEATURE_PREFIX in prefixes and variable_question(original, alternative, before, trigger)
    alnum = ALNUM_FEATURE_PREFIX in prefixes and alnum_question(original, alternative, before)
    if origin == "kept_next_word":
        return KEPT_FEATURE_PREFIX in prefixes or letter or variable or alnum
    return (letter or (CAPITALS_FEATURE_PREFIX in prefixes and capitals_question(original, before))
            or (ABBREVIATION_FEATURE_PREFIX in prefixes and abbreviation_question(original, alternative, before))
            or (LATIN_ABBREVIATION_FEATURE_PREFIX in prefixes and latin_abbreviation_question(original, alternative, before))
            or (ALONE_FEATURE_PREFIX in prefixes and alone_question(original, alternative, before, after, trigger))
            or (START_FEATURE_PREFIX in prefixes and start_question(original, alternative, before, after))
            or (UNKNOWN_FEATURE_PREFIX in prefixes and unknown_question(original, before))
            or (JOINED_FEATURE_PREFIX in prefixes and joined_question(original))
            or (TERM_FEATURE_PREFIX in prefixes and term_question(original, before))
            or (BRIEF_FEATURE_PREFIX in prefixes and brief_question(original, before, after))
            or (FIRST_FEATURE_PREFIX in prefixes and first_question(original, before, after)) or variable
            or (DOTTED_FEATURE_PREFIX in prefixes and dotted_question(original, alternative)) or alnum)


def carried_prefixes(model: ContextModel) -> frozenset[str]:
    """The kept-neighbour question and heads a model already has weights for, by feature prefix.

    A frozen base may be a model fitted with heads (the corpus v30 model carries the kept-neighbour
    question and the capitals head): those weights are frozen with the rest, and a fit onto it selects
    no new feature under their prefixes, so only the heads the recipe names are fitted.
    """

    prefixes = (KEPT_FEATURE_PREFIX, *HEAD_PREFIXES.values())
    return frozenset(prefix for prefix in prefixes if any(name.startswith(prefix) for name in model.weights))


def base_weights(names: Sequence[str], base: ContextModel) -> array[float]:
    """Initial weights of a head fitted onto `base`: the base's own, zero for the head's features."""

    weights = array("d", [0.0]) * (len(names) * len(ACTIONS))
    for index, name in enumerate(names):
        for action, value in enumerate(base.weights.get(name, ())):
            weights[index * len(ACTIONS) + action] = value
    return weights


def log_loss(predictions: array[float], data: Packed) -> float:
    """The importance-weighted log loss of `predictions` on `data`, summed over its rows."""

    return sum(-data.importance[row] * math.log(max(LOG_LOSS_PROBABILITY_FLOOR, predictions[row * len(ACTIONS) + label]))
               for row, label in enumerate(data.labels))


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


def outside_lone_word(predictions: Mapping[str, tuple[array[float], array[int]]],
                      rows: Callable[[str], Iterable[tuple[dict[str, float], int, float]]], threshold: float) -> dict[str, object]:
    """Calibration at the serving threshold on the frames outside the lone-word head's class.

    The lone-word curriculum puts frames of its class into every split labelled by the term counts
    (lone_word_curriculum), not by text anyone typed, and the frames of that class the corpus has are
    relabelled the same way (alone_labels): on corpus v32 that is 617 conversions per profile, of which
    the model it replaces (0.40.0) converts 17 and the corpus v32 candidate 458. The receipt's recall
    floor reads the frames outside the class, where a label is the text itself; what the pair nets is
    still compared on every frame (tests/test_quality_ratchet.py).
    """
    by_profile: dict[str, dict[str, int | float]] = {}
    for name, (values, labels) in predictions.items():
        kept = [index for index, (features, _label, _weight) in enumerate(rows(name))
                if not any(feature.startswith(ALONE_FEATURE_PREFIX) for feature in features)]
        subset = array("d", (values[index * len(ACTIONS) + action] for index in kept for action in range(len(ACTIONS))))
        by_profile[name] = metrics(subset, array("B", (labels[index] for index in kept)), threshold)
    true = sum(int(row["converted_correctly"]) for row in by_profile.values())
    possible = sum(int(row["convert_rows"]) for row in by_profile.values())
    return {"rows": sum(int(row["rows"]) for row in by_profile.values()),
            "false_conversions": sum(int(row["false_conversions"]) for row in by_profile.values()),
            "converted_correctly": true, "convert_rows": possible,
            "conversion_recall": true / possible if possible else 0.0, "by_profile": by_profile}


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


@dataclass
class FitInputs:
    """What every stage of a fit reads: the recipe, the corpus splits, the natural frames and the models."""
    options: dict[str, object]
    source_rows: dict[str, list[CorpusRow]]
    frames: dict[str, list[ActionRow]]
    intent: LinearNgramModel
    ortho: OrthoModel
    lexicons: dict[bool, dict[int, LanguageModel]]
    profiles: list[str]
    refused: frozenset[str]
    # The model a kept-neighbour head is fitted onto (frozen_base), or None for a full fit.
    base: ContextModel | None = None
    lexical_models: dict[str, dict[int, LanguageModel]] = field(init=False)
    detectors: dict[str, LanguageDetector] = field(init=False)

    def __post_init__(self) -> None:
        # Per profile: the reference lexicons, with morphology for the reference_hunspell profile.
        self.lexical_models = {name: self.lexicons[name == REFERENCE_HUNSPELL] for name in self.profiles}
        self.detectors = {name: LanguageDetector(models, self.intent) for name, models in self.lexical_models.items()}


def recipe() -> dict[str, object]:
    options = cast(dict[str, object], json.loads(RECIPE.read_bytes()))
    if options.get("schema_version") != 1 or options.get("feature_version") != CONTEXT_ACTION_FEATURE_VERSION:
        raise ValueError("invalid action recipe")
    return options


def load_inputs(corpus: Path, options: dict[str, object]) -> FitInputs:
    maximum = cast(dict[str, int], options["maximum_source_rows"])
    source_rows = {split: load_split(corpus, split) for split in FITTING_SPLITS}
    frames = {split: action_rows(select_rows(rows, maximum[split])) for split, rows in source_rows.items()}
    if any(not rows for rows in frames.values()):
        raise ValueError("empty fitting split")
    return FitInputs(options, source_rows, frames,
                     LinearNgramModel.load(ROOT / "src/keyswitch/resources/models/layout_intent_v1.ksm"),
                     OrthoModel.load(ROOT / "src/keyswitch/resources/models/ortho_v1.json"),
                     {spelling: reference_models(spelling) for spelling in (False, True)},
                     cast(list[str], options["profiles"]), refused_aliases(corpus),
                     frozen_base(options) if not frozen_heads(options) else None)


def frame_chain(inputs: FitInputs, profile: str, split: str, rows: Sequence[ActionRow],
                tail: Sequence[ActionRow] = ()) -> tuple[list[ActionRow], dict[str, object]]:
    """The curricula one profile adds to the frames of one split, and their reports.

    `tail`, the kept-neighbour and counted-token frames of TRAIN (tail_curricula), joins after both
    lookahead curricula, so that none of its frames seeds a planned one.
    """
    if split == TRAIN and inputs.base is not None:
        # A frozen base keeps every weight of the word decided at its own boundary: only the
        # kept-neighbour question is fitted, and nothing else is framed for TRAIN.
        return list(tail), {}
    options = inputs.options
    detector = inputs.detectors[profile]
    lookahead_options = cast(dict[str, int], options["lookahead_curriculum"])
    natural_options = cast(dict[str, int], options["natural_lookahead_curriculum"])
    lexical_options = cast(dict[str, int], options["lexical_short_pair_curriculum"])
    reports: dict[str, object] = {}
    result = list(rows)
    if split == TRAIN:
        result, reports["lookahead"] = legacy_lookahead_rows(
            result, detector, inputs.ortho, profile=profile,
            maximum_families=lookahead_options["maximum_families"], seeds_per_family=lookahead_options["seeds_per_family"])
    result, reports["natural"] = natural_lookahead_rows(
        result, inputs.source_rows[split], detector, inputs.ortho, profile=profile, split=split,
        maximum_families=natural_options["maximum_families"], seeds_per_family=natural_options["seeds_per_family"])
    if split == TRAIN:
        result, reports["lexical"] = lexical_short_pair_rows(
            result, inputs.source_rows[split], detector, inputs.ortho, refused=inputs.refused, profile=profile, split=split,
            maximum_families=lexical_options["maximum_families"], seeds_per_family=lexical_options["seeds_per_family"],
            models=inputs.lexicons[False])
        result, reports["balance"] = balance_planned_mass(result)
        result.extend(tail)
        if cast(dict[str, object], options.get("first_curriculum", NO_FIRST_CURRICULUM)).get("enabled"):
            # The first-term head is fitted on its own curriculum alone. Other curricula hold frames of its class with
            # older answers (a short English word opening a message waited for its next word: `щт`, `еру`, `ше`): once
            # the head's own frames were learnt they alone moved its weights, its loss on DEVELOPMENT grew from the
            # first epoch, and the head chose it and left `йк` in Code (candidate v67).
            kept = [row for row in result if not outside_first_curriculum(row)]
            reports["first_class_elsewhere"] = len(result) - len(kept)
            result = kept
    if split == DEVELOPMENT:
        # The corpus leaves words of one letter out, so DEVELOPMENT holds no frame of the single-letter head's
        # class: the head's epoch is chosen on the same curriculum drawn from DEVELOPMENT's own sentences.
        letter, reports["letter"] = letter_curriculum(inputs.source_rows[split], inputs.refused,
                                                      cast(dict[str, object], options.get("letter_curriculum", NO_LETTER_CURRICULUM)))
        result.extend(letter)
        # The abbreviation head's epoch is chosen on the same curriculum drawn from DEVELOPMENT's own rows.
        abbreviation, reports["abbreviation"] = abbreviation_curriculum(
            split, inputs.source_rows[split], inputs.refused,
            cast(dict[str, object], options.get("abbreviation_curriculum", NO_ABBREVIATION_CURRICULUM)))
        result.extend(abbreviation)
        # So is the Latin-context abbreviation head's.
        latin_abbreviation, reports["latin_abbreviation"] = latin_abbreviation_curriculum(
            split, inputs.source_rows[split], inputs.refused,
            cast(dict[str, object], options.get("latin_abbreviation_curriculum", NO_LATIN_ABBREVIATION_CURRICULUM)))
        result.extend(latin_abbreviation)
        # And the unknown-token head's, on terms and words TRAIN never framed.
        unknown, reports["unknown_token"] = unknown_token_curriculum(
            split, inputs.source_rows[split], inputs.refused,
            cast(dict[str, object], options.get("unknown_token_curriculum", NO_UNKNOWN_TOKEN_CURRICULUM)))
        result.extend(unknown)
        # The joined-token, term and short-message heads' curricula are not framed here: their epochs are their own, each
        # chosen on its curriculum (fit), and the other heads' epoch is chosen without their weights.
    evidence = alone_evidence(options)
    if evidence is not None:
        lone, reports["lone_word"] = lone_word_curriculum(split, inputs.refused, evidence, partial(base_action, inputs=inputs))
        result = [*alone_labels(result, evidence, partial(base_action, inputs=inputs)), *lone]
    return result, reports


def outside_first_curriculum(row: ActionRow) -> bool:
    """A frame of the first-term head's class (first_question, a field a reader read) another curriculum framed."""

    return (not row.identifier.startswith("first:") and row.field.role != "unknown"
            and first_question(row.original, row.field.before, row.field.after))


def tail_curricula(inputs: FitInputs) -> tuple[list[ActionRow], dict[str, object]]:
    """The TRAIN frames that join after the lookahead curricula (frame_chain): the kept-neighbour
    question and the counted tokens, with their reports by curriculum."""
    options = inputs.options
    kept, kept_report = kept_neighbour_curriculum(inputs.source_rows[TRAIN], inputs.refused,
                                                  cast(dict[str, object], options["kept_neighbour_curriculum"]), inputs.lexicons[False])
    counted, counted_report = counted_token_curriculum(inputs.source_rows[TRAIN], inputs.refused,
                                                       cast(dict[str, object], options["counted_token_curriculum"]))
    letter, letter_report = letter_curriculum(inputs.source_rows[TRAIN], inputs.refused,
                                              cast(dict[str, object], options.get("letter_curriculum", NO_LETTER_CURRICULUM)))
    abbreviation, abbreviation_report = abbreviation_curriculum(
        TRAIN, inputs.source_rows[TRAIN], inputs.refused,
        cast(dict[str, object], options.get("abbreviation_curriculum", NO_ABBREVIATION_CURRICULUM)))
    latin_abbreviation, latin_abbreviation_report = latin_abbreviation_curriculum(
        TRAIN, inputs.source_rows[TRAIN], inputs.refused,
        cast(dict[str, object], options.get("latin_abbreviation_curriculum", NO_LATIN_ABBREVIATION_CURRICULUM)))
    unknown, unknown_report = unknown_token_curriculum(
        TRAIN, inputs.source_rows[TRAIN], inputs.refused,
        cast(dict[str, object], options.get("unknown_token_curriculum", NO_UNKNOWN_TOKEN_CURRICULUM)))
    joined, joined_report = joined_token_curriculum(
        TRAIN, inputs.source_rows[TRAIN], inputs.refused,
        cast(dict[str, object], options.get("joined_token_curriculum", NO_JOINED_TOKEN_CURRICULUM)))
    term, term_report = term_curriculum(
        TRAIN, inputs.source_rows[TRAIN], inputs.refused, cast(dict[str, object], options.get("term_curriculum", NO_TERM_CURRICULUM)))
    brief, brief_report = brief_curriculum(
        TRAIN, inputs.source_rows[TRAIN], inputs.refused, cast(dict[str, object], options.get("brief_curriculum", NO_BRIEF_CURRICULUM)))
    first, first_report = first_curriculum(
        TRAIN, inputs.source_rows[TRAIN], inputs.refused, cast(dict[str, object], options.get("first_curriculum", NO_FIRST_CURRICULUM)))
    variable, variable_report = variable_curriculum(
        TRAIN, inputs.source_rows[TRAIN], inputs.refused, cast(dict[str, object], options.get("variable_curriculum", NO_CAPTURED_HEAD_CURRICULUM)))
    dotted, dotted_report = dotted_curriculum(
        TRAIN, inputs.source_rows[TRAIN], inputs.refused, cast(dict[str, object], options.get("dotted_curriculum", NO_CAPTURED_HEAD_CURRICULUM)))
    alnum, alnum_report = alnum_curriculum(
        TRAIN, inputs.source_rows[TRAIN], inputs.refused, cast(dict[str, object], options.get("alnum_curriculum", NO_CAPTURED_HEAD_CURRICULUM)))
    return [*kept, *counted, *letter, *abbreviation, *latin_abbreviation, *unknown, *joined, *term, *brief, *first, *variable, *dotted,
            *alnum], {
        "kept_neighbour_curriculum": kept_report, "counted_token_curriculum": counted_report, "letter_curriculum": letter_report,
        "abbreviation_curriculum": abbreviation_report, "latin_abbreviation_curriculum": latin_abbreviation_report,
        "unknown_token_curriculum": unknown_report, "joined_token_curriculum": joined_report, "term_curriculum": term_report,
        "brief_curriculum": brief_report, "first_curriculum": first_report, "variable_curriculum": variable_report,
        "dotted_curriculum": dotted_report, "alnum_curriculum": alnum_report}


def prepared_frames(rows: Sequence[ActionRow], spans: SpanCurriculum, split: str) -> list[tuple[str, ActionRow | SpanFrame]]:
    """The frames of one profile and split in the order the optimizer reads them."""
    prepared: list[tuple[str, ActionRow | SpanFrame]] = [(row.identifier, row) for row in rows]
    prepared.extend((f"span:{row.sequence_id}:{index}", row) for index, row in enumerate(spans.frames))
    if split == TRAIN:
        prepared.sort(key=lambda entry: (hashlib.sha256(entry[0].encode()).digest(), entry[0]))
    return prepared


def frame_evidence(row: ActionRow, split: str, detector: LanguageDetector, ortho: OrthoModel | None) -> ContextEvidence:
    """The evidence of an action frame; TRAIN drops the identifier evidence of some families, and the term counts of
    some unknown-token families (term_counts_dropped)."""
    dropped = split == TRAIN and identifier_evidence_dropped(row.identifier)
    item = evidence(row, detector, ortho, identifiers=BLIND_IDENTIFIERS if dropped else None)
    if split == TRAIN and term_counts_dropped(row.identifier, row.original):
        item = replace(item, term_counts_dropped=True)
    return item


def frame_features(frame: ActionRow | SpanFrame, split: str, detector: LanguageDetector, ortho: OrthoModel | None) -> dict[str, float]:
    if isinstance(frame, ActionRow):
        return extract_action_features(frame_evidence(frame, split, detector, ortho))
    return extract_action_features(frame.evidence)


def fit(corpus: Path, output: Path, *, backend: str = CONTEXT_ACTION_BACKEND_AUTO, jobs: int | None = None) -> dict[str, object]:
    """Fit a candidate; every back end (context_action_pipeline) builds the same one, byte for byte."""
    if output.exists():
        raise ValueError("candidate directory already exists; never overwrite an experiment")
    options = recipe()
    try:
        import context_action_pipeline
    except ImportError as error:
        raise RuntimeError(f"{error}: the trainer needs NumPy, see tools/install-training-accelerators.sh") from error

    backend = context_action_pipeline.choose_backend(backend)
    jobs = context_action_pipeline.choose_jobs(jobs)
    print(f"back end {backend}, " + (f"{jobs} worker processes" if jobs > 1 else "in this process"), flush=True)
    before_provenance = provenance()
    corpus_hash = checksum(corpus / "manifest.json")
    inputs = load_inputs(corpus, options)
    profiles = inputs.profiles
    # The pipeline frames TRAIN with the kept-neighbour frames alone for a frozen base without heads
    # (inputs.base); with heads every TRAIN frame is framed, and the base's weights stay where they are
    # because their AdaGrad accumulators start infinite: every step of theirs is zero.
    base = frozen_base(options)
    heads = frozen_heads(options)
    warm = warm_base(options)
    if base is not None and warm is not None:
        raise ValueError("a recipe names a frozen base or a warm base, not both")
    kept_options = cast(dict[str, object], options["kept_neighbour_curriculum"])
    counted_options = cast(dict[str, object], options["counted_token_curriculum"])
    english_options = cast(dict[str, object], options["english_capital_curriculum"])
    if base is not None and not heads and any(cast(dict[str, int], english_options["words_by_length"]).values()):
        raise ValueError("English-capital frames fit the word decided at its own boundary, which a frozen base keeps")
    if base is not None and not heads and any(int(cast(int, counted_options[name])) for name in ("term_frames", "abbreviation_frames")):
        raise ValueError("counted-token frames fit the word decided at its own boundary, which a frozen base keeps")
    output.mkdir(parents=True)
    (output / "recipe.json").write_bytes(canonical(options))
    features = context_action_pipeline.build_features(inputs, backend, jobs)
    # Kept-neighbour frames of DEVELOPMENT choose the epoch of a head fitted onto a frozen base: its
    # development rows are decided by the base alone and say nothing about the head.
    kept_development: dict[str, list[tuple[dict[str, float], int, float]]] = {}
    if base is not None:
        keep_importance = float(cast(float, options["keep_importance"]))
        kept_frames = kept_neighbour_curriculum(inputs.source_rows[DEVELOPMENT], inputs.refused, kept_options, inputs.lexicons[False])[0]
        # In name order, as the serial fit read them back from its feature files: the sums of a prediction
        # run in the same order and the epoch is chosen on the same losses to the last bit.
        kept_development = {profile: [(dict(sorted(frame_features(row, DEVELOPMENT, inputs.detectors[profile], inputs.ortho).items())),
                                       ACTIONS.index(row.action), row.sample_weight * (keep_importance if row.action == "keep" else 1.0))
                                      for row in kept_frames] for profile in profiles}
        print(f"features kept-development: {len(kept_frames)} per profile", flush=True)
    # The joined-token and term heads' own frames of DEVELOPMENT choose their epochs (OWN_EPOCH_HEADS), as the
    # kept-neighbour frames choose the kept-neighbour question's. Ranked with the other heads, the joined-token head's
    # frames moved their shared epoch from 38 to 31-33, and the unknown-token head lost `htpdt` (`резве`) and `f` (`а`) in
    # the owner's typing.
    own_development: dict[str, dict[str, list[tuple[dict[str, float], int, float]]]] = {}
    for prefix, (label, key, curriculum, absent) in OWN_EPOCH_HEADS.items():
        if base is None or prefix not in heads:
            continue
        own_importance = float(cast(float, options["keep_importance"]))
        own_frames = curriculum(DEVELOPMENT, inputs.source_rows[DEVELOPMENT], inputs.refused,
                                cast(dict[str, object], options.get(key, absent)))[0]
        own_development[prefix] = {
            profile: [(dict(sorted(frame_features(row, DEVELOPMENT, inputs.detectors[profile], inputs.ortho).items())),
                       ACTIONS.index(row.action), row.sample_weight * (own_importance if row.action == "keep" else 1.0))
                      for row in own_frames] for profile in profiles}
        print(f"features {label}-development: {len(own_frames)} per profile", flush=True)
    # The frames, curricula and lexicons are not needed any more; the epochs need the memory.
    splits = list(inputs.frames)
    del inputs
    gc.collect()
    minimum = float(cast(float, options["minimum_feature_mass"]))
    masses = features.masses()
    # The kept-neighbour question and every head have feature names of their own and a budget of their
    # own: the word decided at its boundary keeps exactly the features it would have without them.
    start = base if base is not None else warm
    own = (KEPT_FEATURE_PREFIX, *HEAD_PREFIXES.values())
    carried = carried_prefixes(base) if base is not None else frozenset()
    budgets = {prefix: budget for prefix, budget in {KEPT_FEATURE_PREFIX: int(cast(int, kept_options["maximum_features"])), **heads}.items()
               if prefix not in carried}
    names = sorted([
        *(start.weights if start is not None else
          select_features({name: mass for name, mass in masses.items() if not name.startswith(own)},
                          minimum, int(cast(int, options["maximum_features"])))),
        *(name for prefix, budget in budgets.items()
          for name in select_features({name: mass for name, mass in masses.items() if name.startswith(prefix)}, minimum, budget)),
    ])
    train, development, calibration = features.packed(names)
    support_model = ContextModel({name: (0.0,) * len(ACTIONS) for name in names}, "context-v3-vocabulary", feature_version=CONTEXT_ACTION_FEATURE_VERSION)
    development_masks = {name: runtime_masks(features.rows(name, DEVELOPMENT), support_model) for name in profiles}
    development_mass = sum(sum(data.importance) for data in development.values())
    kernel = Kernel.load()
    weights = array("d", [0.0]) * (len(names) * len(ACTIONS)) if start is None else base_weights(names, start)
    accumulators = array("d", [1.0]) * (len(names) * len(ACTIONS))
    if base is not None:
        for index, name in enumerate(names):
            if name in base.weights:
                accumulators[index * len(ACTIONS):(index + 1) * len(ACTIONS)] = array("d", [math.inf]) * len(ACTIONS)
        # The epochs read only the frames that hold a weight the fit can move: the others step nothing.
        frames = len(train.labels)
        train = context_action_pipeline.trainable_frames(train, accumulators)
        print(f"frames with a trainable feature: {len(train.labels)} of {frames}", flush=True)
    rate = float(cast(float, cast(dict[str, object], options["warm_base"])["learning_rate"] if warm is not None
                      else options["learning_rate"]))
    epochs = int(cast(int, options["epochs"]))
    best, best_epoch, best_loss = array("d"), 0, math.inf
    best_selection: EpochSelection | None = None
    # Every qualifying epoch with its weights: the heads' epoch is chosen once all are known (settled_epoch).
    qualifying: list[tuple[int, EpochSelection, array[float], float]] = []
    kept_best, kept_epoch, kept_loss = array("d"), 0, math.inf
    history: list[dict[str, object]] = []
    kept_check = {name: Packed.build(rows, names) for name, rows in kept_development.items()}
    kept_mass = sum(sum(data.importance) for data in kept_check.values())
    own_checks = {prefix: checks for prefix, checks in
                  ((prefix, {profile: Packed.build(rows, names) for profile, rows in development_rows.items() if rows})
                   for prefix, development_rows in own_development.items()) if checks}
    own_mass = {prefix: sum(sum(data.importance) for data in checks.values()) for prefix, checks in own_checks.items()}
    own_slots = {prefix: [index * len(ACTIONS) + action for index, name in enumerate(names)
                          if name.startswith(prefix) for action in range(len(ACTIONS))] for prefix in own_checks}
    own_best = {prefix: array("d") for prefix in own_checks}
    own_epoch = dict.fromkeys(own_checks, 0)
    own_loss = dict.fromkeys(own_checks, math.inf)
    # The kernel releases the GIL: the next epoch trains in a thread while this one is scored.
    with ThreadPoolExecutor(max_workers=1) as following:
        kernel.epoch(train, weights, accumulators, rate)
        for epoch in range(epochs):
            rounded = array("d", (round(value, DETERMINISTIC_ROUNDING_DECIMALS) for value in weights))
            next_epoch = following.submit(kernel.epoch, train, weights, accumulators, rate) if epoch + 1 < epochs else None
            record: dict[str, object] = {"epoch": epoch + 1}
            if base is not None:
                # The kept-neighbour head's epoch is the one whose kept-neighbour frames of DEVELOPMENT it
                # predicts best; the base's weights never move, so its rows cannot choose.
                loss = sum(log_loss(kernel.predict(data, rounded), data) for data in kept_check.values()) / kept_mass
                if loss < kept_loss:
                    kept_best, kept_epoch, kept_loss = rounded, epoch + 1, loss
                record["kept_development_loss"] = loss
                print(f"epoch {epoch + 1}: kept_development_loss={loss:.9f}, best={kept_epoch}", flush=True)
            for prefix, checks in own_checks.items():
                loss = sum(log_loss(kernel.predict(data, rounded), data) for data in checks.values()) / own_mass[prefix]
                if loss < own_loss[prefix]:
                    own_best[prefix], own_epoch[prefix], own_loss[prefix] = array("d", rounded), epoch + 1, loss
                label = OWN_EPOCH_HEADS[prefix][0]
                record[f"{label}_development_loss"] = loss
                print(f"epoch {epoch + 1}: {label}_development_loss={loss:.9f}, best={own_epoch[prefix]}", flush=True)
            # The other heads are ranked without the heads of their own epoch: no frame holds the features of two heads.
            for slots in own_slots.values():
                for slot in slots:
                    rounded[slot] = 0.0
            if base is not None and not heads:
                history.append(record)
                if next_epoch is not None:
                    next_epoch.result()
                continue
            predictions = {name: kernel.predict(data, rounded) for name, data in development.items()}
            loss = sum(-data.importance[row] * math.log(max(LOG_LOSS_PROBABILITY_FLOOR, predictions[name][row * len(ACTIONS) + label]))
                       for name, data in development.items() for row, label in enumerate(data.labels)) / development_mass
            selection = assess_epoch({name: (apply_support_mask(predictions[name], *development_masks[name]), data.labels)
                                      for name, data in development.items()},
                                     thresholds=development_thresholds(options), loss=loss,
                                     minimum_net_benefit=int(cast(dict[str, int], options["epoch_selection"])["minimum_net_benefit_per_profile"]))
            if selection is not None:
                qualifying.append((epoch + 1, selection, rounded, loss))
            history.append({**record, "development_loss": loss, "selection": asdict(selection) if selection is not None else None})
            leading = max(qualifying, key=lambda entry: (entry[1].rank, -entry[0]))[0] if qualifying else 0
            print(f"epoch {epoch + 1}: development_loss={loss:.9f}, best={leading}", flush=True)
            if next_epoch is not None:
                next_epoch.result()
    if qualifying:
        best_epoch, best_selection, best, best_loss = settled_epoch(qualifying)
    if (base is None or heads) and best_selection is None:
        raise ValueError("no epoch repaired more than it broke on development")
    if base is not None and not heads:
        best, best_epoch, best_loss = kept_best, kept_epoch, kept_loss
    elif base is not None:
        # The heads at the epoch development ranks best, the kept-neighbour head at its own: no frame
        # holds the features of both, so neither moves the other's answers.
        for index, name in enumerate(names):
            if name.startswith(KEPT_FEATURE_PREFIX):
                best[index * len(ACTIONS):(index + 1) * len(ACTIONS)] = kept_best[index * len(ACTIONS):(index + 1) * len(ACTIONS)]
            else:
                head = next((prefix for prefix in own_checks if name.startswith(prefix) and own_epoch[prefix]), None)
                if head is not None:
                    best[index * len(ACTIONS):(index + 1) * len(ACTIONS)] = own_best[head][index * len(ACTIONS):(index + 1) * len(ACTIONS)]
    gates = cast(dict[str, int | float | bool], options["gate_policy"])
    mapping = {name: list(best[index * len(ACTIONS):index * len(ACTIONS) + len(ACTIONS)]) for index, name in enumerate(names)}
    candidate = ContextModel({name: tuple(values) for name, values in mapping.items()},
                             "context-v3-fitting", feature_version=CONTEXT_ACTION_FEATURE_VERSION)
    calibration_predictions = {name: (apply_runtime_support(kernel.predict(data, best), features.rows(name, CALIBRATION), candidate),
                                      data.labels) for name, data in calibration.items()}
    if base is not None and any(mapping[name] != list(values) for name, values in base.weights.items()):
        raise ValueError("a weight of the frozen base moved")
    if base is not None:
        # Equal, and now the very values of the base: a zero step may leave -0.0 as 0.0.
        mapping.update({name: list(values) for name, values in base.weights.items()})
    # A head fitted onto a frozen base serves at the base's threshold: every other decision is the base's.
    threshold, calibration_report, passed = choose_threshold(
        calibration_predictions,
        cast(list[float], options["threshold_candidates"]) if base is None else [base.conversion_threshold],
        int(gates["minimum_calibration_net_benefit"]), float(gates["minimum_calibration_conversion_recall"]),
        minimum_threshold=base.conversion_threshold if base is not None else best_selection.threshold if best_selection is not None else 0.0,
        net_benefit_tolerance=float(cast(float, cast(dict[str, object], options["threshold_selection"])["net_benefit_tolerance"])),
        maximum_threshold=float(cast(float, cast(dict[str, object], options["threshold_selection"])["maximum_threshold"])),
        authored_floor=float(cast(float, cast(dict[str, object], options["threshold_selection"])["authored_floor"])),
    )
    calibration_report["outside_lone_word"] = outside_lone_word(
        calibration_predictions, lambda name: features.rows(name, CALIBRATION), threshold)
    weight_hash = hashlib.sha256(json.dumps(mapping, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode()).hexdigest()
    payload = {"actions": list(ACTIONS), "feature_version": CONTEXT_ACTION_FEATURE_VERSION, "weights": mapping,
               "weights_sha256": weight_hash, "version": "context-v3-" + weight_hash[:VERSION_HASH_CHARACTERS],
               "conversion_threshold": threshold}
    (output / ARTIFACT).write_bytes(artifact_bytes(payload))
    ContextModel.load(output / ARTIFACT)
    if provenance() != before_provenance or checksum(corpus / "manifest.json") != corpus_hash:
        raise ValueError("training inputs changed while fitting; candidate cannot be sealed")
    seal: dict[str, object] = {"schema_version": 1,
        "stage": SEALED_BEFORE_TEST if passed else REJECTED_BEFORE_TEST,
        "artifact_sha256": checksum(output / ARTIFACT), "model_version": payload["version"],
        "provenance": before_provenance, "corpus_manifest_sha256": corpus_hash,
        "recipe": options, "gate_policy": gates, "calibration": calibration_report,
        "development": {"selected_epoch": best_epoch, "loss": best_loss, "history": history,
                        "selection": asdict(best_selection) if best_selection is not None else None,
                        **({"kept_selected_epoch": kept_epoch, "kept_loss": kept_loss} if base is not None and heads else {}),
                        **{name: value for prefix in own_checks if own_epoch[prefix]
                           for name, value in ((f"{OWN_EPOCH_HEADS[prefix][0]}_selected_epoch", own_epoch[prefix]),
                                               (f"{OWN_EPOCH_HEADS[prefix][0]}_loss", own_loss[prefix]))}},
        "frozen_base": None if base is None else frozen_record(options, base),
        "warm_base": None if warm is None else {
            "artifact": cast(dict[str, object], options["warm_base"])["artifact"],
            "sha256": cast(dict[str, object], options["warm_base"])["sha256"],
            "model_version": warm.version, "learning_rate": rate},
        "conversion_threshold": threshold, "feature_count": len(names),
        # A frozen base frames TRAIN with its kept-neighbour frames alone: no span, lookahead, lexical or
        # balance report of TRAIN then.
        "span_curriculum": {profile: {split: features.spans[profile, split] for split in splits if (profile, split) in features.spans}
                            for profile in profiles},
        "lookahead_curriculum": {profile: features.chains[profile, TRAIN]["lookahead"] for profile in profiles
                                 if "lookahead" in features.chains[profile, TRAIN]},
        "natural_lookahead_curriculum": {profile: {split: features.chains[profile, split]["natural"] for split in splits
                                                   if "natural" in features.chains[profile, split]} for profile in profiles},
        "lexical_short_pair_curriculum": {profile: features.chains[profile, TRAIN]["lexical"] for profile in profiles
                                          if "lexical" in features.chains[profile, TRAIN]},
        **({"lone_word_curriculum": {profile: {split: features.chains[profile, split]["lone_word"] for split in splits
                                               if "lone_word" in features.chains[profile, split]}
                                     for profile in profiles}} if alone_evidence(options) is not None else {}),
        "captured_curriculum": features.captured,
        "capital_citation_curriculum": features.capitals,
        "english_capital_curriculum": features.english,
        **features.tail,
        "planned_mass_balance": {profile: features.chains[profile, TRAIN]["balance"] for profile in profiles
                                 if "balance" in features.chains[profile, TRAIN]},
        "test_accessed": False,
        "scope": "natural KEEP plus declared layout/mixed-context interventions; sequence evaluation required"}
    (output / SEAL).write_bytes(readable(seal))
    print(f"{seal['stage']}: {payload['version']}; threshold={threshold}; test not read", flush=True)
    return seal


def settled_epoch(qualifying: Sequence[tuple[int, EpochSelection, array[float], float]],
                  ) -> tuple[int, EpochSelection, array[float], float]:
    """The epoch the heads keep: the latest whose development balance is within EPOCH_NET_BENEFIT_TOLERANCE of the best.

    The development balance is flat over most epochs, and a difference of a row or two chose the epoch: on corpus v62
    epoch 17 netted one conversion more than epoch 48 (37 674 against 37 673), and the unknown-token head of epoch 17
    lost `htpdt` (`резве`) and `f` (`а`) in the owner's typing, which that of epoch 48 had kept. Within the tolerance
    the difference is noise, and the latest epoch is the one that fitted the heads' curricula furthest."""

    best = max(qualifying, key=lambda entry: (entry[1].rank, -entry[0]))[1]
    near = [entry for entry in qualifying
            if entry[1].minimum_net_benefit >= best.minimum_net_benefit - EPOCH_NET_BENEFIT_TOLERANCE
            and entry[1].net_benefit >= best.net_benefit - EPOCH_NET_BENEFIT_TOLERANCE]
    epoch, selection, weights, loss = max(near, key=lambda entry: entry[0])
    return epoch, selection, weights, loss


def feature_rows(path: Path) -> Iterable[tuple[dict[str, float], int, float]]:
    with gzip.open(path, "rt", encoding="utf-8") as source:
        for line in source:
            features, label, importance = cast(tuple[dict[str, float], int, float], json.loads(line))
            yield features, label, importance


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--backend", choices=CONTEXT_ACTION_BACKENDS, default=CONTEXT_ACTION_BACKEND_AUTO,
                        help="cpu: the trainer's Python in worker processes; gpu: evidence and features on CUDA; "
                             "auto: gpu when it can run here. Every back end fits the same candidate.")
    parser.add_argument("--jobs", type=int, default=None,
                        help="worker processes (default: one per core, as far as the available memory allows)")
    args = parser.parse_args(argv)
    seal = fit(args.corpus, args.output, backend=args.backend, jobs=args.jobs)
    return 0 if seal["stage"] == SEALED_BEFORE_TEST else 1


if __name__ == "__main__":
    # Run as the named module: the back ends import it by name, and its classes must be the same ones.
    import train_context_action_model

    raise SystemExit(train_context_action_model.main())
