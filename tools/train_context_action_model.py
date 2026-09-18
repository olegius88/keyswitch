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
from keyswitch.context_model import ACTIONS, AfterOrigin, ContextAction, ContextEvidence, ContextModel
from keyswitch.context_policy import evidence_for_decision
from keyswitch.detector import LanguageDetector
from keyswitch.identifier_lexicon import IdentifierLexicon
from keyswitch.input_context import FieldContext
from keyswitch.intent_model import CorrectionTrigger, IntentModelStatus, LinearNgramModel
from keyswitch.language_model import LanguageModel
from keyswitch.ortho_model import OrthoModel
from keyswitch.short_words import TRUSTED_SINGLE_LETTER_WORDS
from keyswitch.word_decision import automatic_word_decision

from model_protocol import FITTING_SPLITS, REJECTED_BEFORE_TEST, SEALED_BEFORE_TEST
from reference_lexicon import reference_models
from action_epoch_selection import EpochSelection, assess_epoch
from context_action_spans import SpanFrame, build_span_curriculum
from context_lookahead_curriculum import LookaheadAnchor, LookaheadSeed, build_lookahead_curriculum
from context_optimizer import Kernel, Packed
from context_physical_keys import translated as translated
from evaluate_context_action_sequences import runtime_provenance
from freeze_context_action_corpus import CorpusRow, load_split, physical, typo_variants


ROOT = Path(__file__).resolve().parents[1]
RECIPE = ROOT / "model/context_v3/recipe.json"
ARTIFACT = "context-action.json"
SEAL = "candidate-seal.json"
WORDS = re.compile(r"[A-Za-zА-Яа-яЁё]+(?:['’\-][A-Za-zА-Яа-яЁё]+)*")


def canonical(value: object) -> bytes:
    return (json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":")) + "\n").encode()


def checksum(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def variant_choice(identifier: str, purpose: str, count: int) -> int:
    return int(hashlib.sha256((purpose + ":" + identifier).encode()).hexdigest()[:8], 16) % count


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
    eligible = (name for name, value in masses.items() if value >= minimum - 1e-9)
    return sorted(sorted(eligible, key=lambda name: (-round(masses[name], 9), name))[:maximum])


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
        if languages[group] >= maximum // 2 or counts[key] >= 2:
            continue
        counts[key] += 1
        languages[group] += 1
        selected.append(row)
    return selected


def action_rows(rows: Sequence[CorpusRow]) -> list[ActionRow]:
    result: list[ActionRow] = []
    triggers: tuple[CorrectionTrigger, ...] = ("space", "space", "enter", "punctuation", "tab", "pause")
    for row in rows:
        if row.group not in (0, 1) or not row.layout_representable:
            raise ValueError("action rows require a representable single-layout token")
        group = row.group
        alternate = translated(row.original, group)
        number = int(hashlib.sha256(row.identifier.encode()).hexdigest()[:8], 16)
        trigger = triggers[number % len(triggers)]
        boundary_text = ""
        if trigger == "space":
            boundary_text = row.spacing[:1] or " "
        elif trigger == "punctuation":
            boundary_text = next((char for char in row.literal_tail if not char.isspace()), ".")
        elif trigger in {"enter", "tab"} and variant_choice(row.identifier, "boundary-event", 2):
            boundary_text = "\n" if trigger == "enter" else "\t"
        application = ("Telegram", "Code", "chrome", "UnseenEditor")[variant_choice(row.identifier, "application", 4)]
        for context_name, before in (("observed", row.before), ("empty", "")):
            # Lookahead is optional field evidence, never assumed available.
            after = row.after if context_name == "observed" and variant_choice(row.identifier, "field-after", 8) == 0 else ""
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
            if alone and len(row.original) <= 2 and not curated_letter:
                # An isolated short reading has no observable intent label.
                # Digits/punctuation do not supply a neighbouring language.
                # Both members preserve text and take the same deferred action.
                # Deferring three-letter readings too was measured on 17.09.2026 and
                # made the fitted model worse on chat-like first words, not better:
                # see .t/reliable-release-2026-09-12/SHORT-ISOLATED-CURRICULUM.md.
                action = "suggest" if trigger in ("enter", "tab", "punctuation") else "wait"
                keep_action = action
            result.append(ActionRow(identity + ":keep", row.original, group, field,
                                    trigger, tail, keep_action, "natural_surface", boundary_text))
            result.append(ActionRow(identity + ":wrong", alternate, 1 - group, field,
                                    trigger, tail, action, "layout_intervention", boundary_text))
        # A legitimate insertion may have neighbours in the other language.
        mixed = "в сообщении написано " if group == 0 else "the message says "
        mixed_field = FieldContext(application, "public-training", mixed, "", "unknown")
        result.append(ActionRow(row.identifier + ":mixed", row.original, group,
                                mixed_field,
                                trigger, "", "keep", "mixed_language_insertion", boundary_text))
        result.append(ActionRow(row.identifier + ":mixed:wrong", alternate, 1 - group,
                                mixed_field, trigger, "", "convert",
                                "mixed_language_layout_intervention", boundary_text))
        for index, typo in enumerate(typo_variants(row.original, row.identifier)):
            result.append(ActionRow(row.identifier + f":spelling:{index}", typo, group,
                                    FieldContext(application, "public-training", row.before, "", "unknown"),
                                    trigger, "", "keep", "spelling_intervention", boundary_text))
    return result


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

    def load(locale: str) -> LanguageModel:
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
                if (action == "keep" and 0 < len(item.original) <= 2
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


def legacy_lookahead_rows(
    rows: Sequence[ActionRow], detector: LanguageDetector, ortho: OrthoModel | None, *,
    profile: str, maximum_families: int, seeds_per_family: int,
) -> tuple[list[ActionRow], dict[str, object]]:
    """Replace bounded old TRAIN frames with equal-mass planned variants."""
    selected = {row.identifier: row for row in rows
                if row.category.startswith("legacy_") and 0 < len(row.original) <= 2
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
        if not 3 <= len(text) <= 64 or not text.isalpha():
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
    one- or two-letter word typed in the wrong layout can be paired with the word
    that follows it, exactly as the engine's planned lookahead sees it, including
    at the start of a field where the left context is empty. Anchors are the first
    words of the same split's own right contexts; nothing crosses a split.
    """
    if any(row.split != split for row in source_rows):
        raise ValueError("natural lookahead requires rows of the declared split")
    by_identifier = {row.identifier: row for row in source_rows}
    selected: dict[str, tuple[ActionRow, str]] = {}
    for row in rows:
        if (row.category not in ("layout_intervention", "natural_surface") or not 0 < len(row.original) <= 2
                or row.trigger != "space" or row.boundary_text != " " or row.literal_tail
                or row.field.sensitive or row.field.selection or row.field.role == "password" or row.field.after):
            continue
        source = by_identifier.get(row.identifier.rsplit(":", 2)[0])
        if source is None:
            continue
        if row.category == "layout_intervention":
            selected[row.identifier] = row, source.after
        elif row.field.before.strip():
            selected[row.identifier] = row, ""
    anchors: dict[tuple[str, int], LookaheadAnchor] = {}
    for source in sorted(source_rows, key=lambda item: item.identifier):
        match = WORDS.match(source.after.lstrip())
        if match is None:
            continue
        text = match.group()
        if not 3 <= len(text) <= 64 or not text.isalpha():
            continue
        group = 1 if any("а" <= char.casefold() <= "я" or char.casefold() == "ё" for char in text) else 0
        try:
            translated(text, group)
        except ValueError:
            continue
        anchors.setdefault((text, group), LookaheadAnchor(
            source.identifier + ":first-after", "natural-after:" + hashlib.sha256(physical(text).encode()).hexdigest(),
            text, group, split))
    seeds = [LookaheadSeed(row.identifier, "natural:" + by_identifier[row.identifier.rsplit(":", 2)[0]].document,
                          evidence(row, detector, ortho),
                          row.action, "natural_short_lookahead" if row.category == "layout_intervention" else row.category,
                          row.sample_weight, split, next_words,
                          "convert" if row.category == "layout_intervention" else "keep")
             for row, next_words in selected.values()]
    curriculum = build_lookahead_curriculum(seeds, list(anchors.values()), detector, profile=profile,
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


def balance_planned_mass(rows: Sequence[ActionRow]) -> tuple[list[ActionRow], dict[str, object]]:
    """Give planned lookahead frames the mass of the situation they resolve.

    A one- or two-letter word at a space boundary has two outcomes in the corpus:
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
        if not 0 < len(row.original) <= 2:
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
              "isolated_mass": {f"{group}:{length}": round(value, 4) for (group, length), value in sorted(isolated.items())},
              "planned_mass_before": {f"{group}:{length}": round(value, 4) for (group, length), value in sorted(planned.items())},
              "scale": {f"{group}:{length}": round(value, 4) for (group, length), value in sorted(scale.items())},
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
IDENTIFIER_DROPOUT_FAMILIES = 3


def identifier_family(identifier: str) -> str:
    """The unit that shares one identifier-evidence dropout decision: a command with all its
    contexts and spelling variants, otherwise the row itself."""
    parts = identifier.split(":")
    if len(parts) >= 3 and parts[1] == "command":
        return ":".join(parts[:3])
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
        scores = probabilities[index * 4:index * 4 + 4]
        predicted = max(range(4), key=scores.__getitem__)
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
    admissible = [row for row in qualifying if (row[0], row[1]) >= floor]
    _minimum, _net, _false, threshold, chosen = min(admissible, key=lambda row: (row[2], row[3]))
    return threshold, {**chosen, "net_benefit_ceiling": ceiling[1], "net_benefit_floor": floor[1],
                       "admissible_thresholds": [row[3] for row in admissible]}, True


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
    if len(supported) * 4 != len(probabilities) or len(automatic) != len(supported):
        raise ValueError("calibration feature and prediction counts differ")
    result = array("d", probabilities)
    for index, (available, allowed) in enumerate(zip(supported, automatic)):
        scores = result[index * 4:index * 4 + 4]
        selected = max(range(4), key=scores.__getitem__)
        if (not available and selected in (0, 1)) or (not allowed and selected == 1):
            result[index * 4:index * 4 + 4] = array("d", [0.0, 0.0, 0.0, 1.0])
    return result


def fit(corpus: Path, output: Path) -> dict[str, object]:
    if output.exists():
        raise ValueError("candidate directory already exists; never overwrite an experiment")
    options = cast(dict[str, object], json.loads(RECIPE.read_bytes()))
    if options.get("schema_version") != 1 or options.get("feature_version") != 3:
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
    frames["train"] = training_order([*frames["train"], *historical_curriculum(intent)])
    feature_paths: dict[tuple[str, str], Path] = {}
    feature_mass = FeatureMass()
    span_budgets = cast(dict[str, int], options["span_maximum_families"])
    span_reports: dict[str, dict[str, dict[str, int]]] = {}
    lookahead_reports: dict[str, dict[str, object]] = {}
    lookahead_options = cast(dict[str, int], options["lookahead_curriculum"])
    natural_options = cast(dict[str, int], options["natural_lookahead_curriculum"])
    natural_reports: dict[str, dict[str, dict[str, object]]] = {}
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
    support_model = ContextModel({name: (0.0,) * 4 for name in names}, "context-v3-vocabulary", feature_version=3)
    development_masks = {name: runtime_masks(feature_rows(feature_paths[name, "development"]), support_model)
                         for name in profiles}
    development_mass = sum(sum(data.importance) for data in development.values())
    kernel = Kernel.load()
    weights = array("d", [0.0]) * (len(names) * 4)
    accumulators = array("d", [1.0]) * (len(names) * 4)
    best, best_epoch, best_loss = array("d"), 0, math.inf
    best_selection: EpochSelection | None = None
    history: list[dict[str, object]] = []
    for epoch in range(int(cast(int, options["epochs"]))):
        kernel.epoch(train, weights, accumulators, float(cast(float, options["learning_rate"])))
        rounded = array("d", (round(value, 9) for value in weights))
        predictions = {name: kernel.predict(data, rounded) for name, data in development.items()}
        loss = sum(-data.importance[row] * math.log(max(1e-15, predictions[name][row * 4 + label]))
                   for name, data in development.items() for row, label in enumerate(data.labels)) / development_mass
        selection = assess_epoch({name: (apply_support_mask(predictions[name], *development_masks[name]), data.labels)
                                  for name, data in development.items()},
                                 thresholds=cast(list[float], options["threshold_candidates"]), loss=loss,
                                 minimum_net_benefit=int(cast(dict[str, int], options["epoch_selection"])["minimum_net_benefit_per_profile"]))
        if selection is not None and (best_selection is None or selection.rank > best_selection.rank):
            best, best_epoch, best_loss, best_selection = rounded, epoch + 1, loss, selection
        history.append({"epoch": epoch + 1, "development_loss": loss,
                        "selection": asdict(selection) if selection is not None else None})
        print(f"epoch {epoch + 1}: development_loss={loss:.9f}, best={best_epoch}", flush=True)
    if best_selection is None:
        raise ValueError("no epoch repaired more than it broke on development")
    gates = cast(dict[str, int | float | bool], options["gate_policy"])
    mapping = {name: list(best[index * 4:index * 4 + 4]) for index, name in enumerate(names)}
    candidate = ContextModel({name: tuple(values) for name, values in mapping.items()},
                             "context-v3-fitting", feature_version=3)
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
    payload = {"actions": list(ACTIONS), "feature_version": 3, "weights": mapping,
               "weights_sha256": weight_hash, "version": "context-v3-" + weight_hash[:12],
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
