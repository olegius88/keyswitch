"""Bounded TRAIN-only candidates for an unexecuted next-word conversion.

The original first-word annotation supplies the label. A baseline proposal for
the next word only establishes an entry condition; it never supplies that label.
Callers replace the supplied seeds with returned frames, preserving their mass.
No candidate context-model predictions, split loader or private corpus reads
occur here. The shared baseline may consult its installed intent model.
"""

from __future__ import annotations

from collections import Counter, defaultdict
from collections.abc import Sequence
from dataclasses import dataclass, replace
import hashlib
import json
import math
from pathlib import Path

from context_physical_keys import translated
from keyswitch.context_action_features import extract_action_features
from keyswitch.context_model import ACTIONS, ContextAction, ContextEvidence
from keyswitch.detector import LanguageDetector
from keyswitch.word_decision import automatic_word_decision
from model_protocol import FITTING_SPLITS


ROOT = Path(__file__).resolve().parents[1]
MAXIMUM_SHORT_WORD_CHARACTERS = 2
DEFAULT_MAXIMUM_FAMILIES = 64
MAXIMUM_FAMILIES_LIMIT = 4096
# Also the ceiling `seeds_per_family` may not exceed; the default sits at the cap.
MAXIMUM_SEEDS_PER_FAMILY = 128
MINIMUM_ANCHOR_WORD_CHARACTERS = 3
MAXIMUM_ANCHOR_WORD_CHARACTERS = 64
PLANNED_VARIANT_MASS_DIVISOR = 2.0


@dataclass(frozen=True)
class LookaheadSeed:
    identifier: str
    parent_family: str
    evidence: ContextEvidence
    action: ContextAction
    category: str
    sample_weight: float = 1.0
    split: str = "train"
    # Annotated right context of a natural typing frame whose evidence keeps
    # after empty: it only selects the anchor, it never enters the original frame.
    next_words: str = ""
    # The label once the next word is known. A natural isolated short frame is
    # deferred (wait/suggest) at its own boundary, yet its planned variant with
    # the real continuation is the declared intervention of the row.
    planned_action: ContextAction | None = None


@dataclass(frozen=True)
class LookaheadAnchor:
    identifier: str
    parent_family: str
    text: str
    group: int
    split: str = "train"


@dataclass(frozen=True)
class LookaheadFrame:
    source_identifier: str
    parent_family: str
    evidence: ContextEvidence
    action: ContextAction
    sample_weight: float
    kind: str = "original"
    anchor_identifier: str = ""
    anchor_family: str = ""
    next_original: str = ""


@dataclass(frozen=True)
class LookaheadCurriculum:
    frames: tuple[LookaheadFrame, ...]
    counts: dict[str, int]
    mass_by_origin_action: dict[str, float]


def provenance() -> dict[str, str]:
    names = ("tools/context_lookahead_curriculum.py", "tools/context_physical_keys.py",
             "src/keyswitch/engine.py", "src/keyswitch/word_decision.py", "src/keyswitch/detector.py",
             "src/keyswitch/context_model.py", "src/keyswitch/context_action_features.py",
             "src/keyswitch/context_policy.py")
    return {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest() for name in names}


def _rank(purpose: str, identifier: str) -> bytes:
    return hashlib.sha256(("context-lookahead-v1:" + purpose + ":" + identifier).encode()).digest()


def _fingerprint(item: ContextEvidence) -> str:
    features = json.dumps(extract_action_features(item), sort_keys=True, ensure_ascii=False, allow_nan=False)
    return hashlib.sha256(features.encode()).hexdigest()


def _eligible(seed: LookaheadSeed) -> bool:
    item = seed.evidence
    if (not 0 < len(item.original) <= MAXIMUM_SHORT_WORD_CHARACTERS or not item.original.isalpha() or not item.alternative.isalpha()
            or item.trigger != "space" or item.boundary_text != " "
            or item.literal_tail or item.after_origin == "planned_next_conversion"):
        return False
    try:
        if translated(item.original, item.source_group) != item.alternative:
            return False
    except ValueError:
        return False
    if seed.action == "convert" and seed.category == "legacy_short_lookahead":
        return bool(item.field.after.strip())
    planned = seed.planned_action or seed.action
    if planned == "convert" and seed.category == "natural_short_lookahead":
        # A natural typing frame knows its own continuation; the left context may
        # be empty, exactly as at the start of a field.
        return not item.field.after and bool(seed.next_words.strip())
    return (planned == "keep" and (seed.category.startswith("legacy_") or seed.category == "natural_surface")
            and bool(item.field.before.strip()) and not item.field.after)


def build_lookahead_curriculum(
    seeds: Sequence[LookaheadSeed], anchors: Sequence[LookaheadAnchor], detector: LanguageDetector, *,
    profile: str, maximum_families: int = DEFAULT_MAXIMUM_FAMILIES,
    seeds_per_family: int = MAXIMUM_SEEDS_PER_FAMILY, split: str = "train",
) -> LookaheadCurriculum:
    """Return original plus labelled, unexecuted planned-context variants.

    Seeds and anchors must already belong to the one declared split (TRAIN by
    default; a development or calibration curriculum uses only its own rows, so
    nothing crosses a split). No membership is inferred from lexical recognition. KEEP seeds retain their original/before
    annotation while an explicitly mixed next word uses an existing TRAIN anchor.
    CONVERT seeds use the first complete word of their annotated right context,
    only when it is an exact supplied anchor. A fixed hash chooses at most one
    anchor before the baseline check; failed proposals are never resampled.

    These are reconstructed candidates, not evidence that the current context
    model enters WAIT or converts the next word. Sequence acceptance remains
    necessary. Original and planned variants divide each input seed's mass.
    """
    if (type(maximum_families) is not int or not 1 <= maximum_families <= MAXIMUM_FAMILIES_LIMIT
            or type(seeds_per_family) is not int or not 1 <= seeds_per_family <= MAXIMUM_SEEDS_PER_FAMILY):
        raise ValueError(
            f"lookahead budgets must be integers: families 1 to {MAXIMUM_FAMILIES_LIMIT}, "
            f"seeds per family 1 to {MAXIMUM_SEEDS_PER_FAMILY}"
        )
    if profile not in ("portable", "reference_hunspell") or set(detector.models) != {0, 1}:
        raise ValueError("lookahead requires an explicit lexical profile and both models")
    if split not in FITTING_SPLITS:
        raise ValueError("lookahead split must be train, development or calibration")
    members: tuple[LookaheadSeed | LookaheadAnchor, ...] = (*seeds, *anchors)
    if any(item.split != split for item in members):
        raise ValueError("lookahead accepts only existing members of the declared split")
    if any(not item.identifier or not item.parent_family for item in members):
        raise ValueError("lookahead requires source and parent identities")
    if len({seed.identifier for seed in seeds}) != len(seeds) or len({anchor.identifier for anchor in anchors}) != len(anchors):
        raise ValueError("lookahead requires unique source identities")
    for seed in seeds:
        if (type(seed.sample_weight) not in (int, float) or not math.isfinite(seed.sample_weight)
                or seed.sample_weight <= 0 or seed.action not in ACTIONS):
            raise ValueError("lookahead requires finite positive mass and an existing action")
        if seed.evidence.field.sensitive or seed.evidence.field.selection or seed.evidence.field.role == "password":
            raise ValueError("unsafe field cannot become a lookahead curriculum")
    counts: Counter[str] = Counter({"input_seeds": len(seeds), "input_anchors": len(anchors)})
    available: dict[int, list[LookaheadAnchor]] = {0: [], 1: []}
    for anchor in sorted(anchors, key=lambda row: row.identifier):
        try:
            if (not MINIMUM_ANCHOR_WORD_CHARACTERS <= len(anchor.text) <= MAXIMUM_ANCHOR_WORD_CHARACTERS
                    or not anchor.text.isalpha()):
                raise ValueError("anchor is not one complete word")
            translated(anchor.text, anchor.group)
        except ValueError:
            counts["skipped_anchor"] += 1
            continue
        available[anchor.group].append(anchor)
    families: defaultdict[str, list[LookaheadSeed]] = defaultdict(list)
    labels: defaultdict[str, set[ContextAction]] = defaultdict(set)
    for seed in seeds:
        labels[_fingerprint(seed.evidence)].add(seed.action)
        if _eligible(seed):
            families[seed.parent_family].append(seed)
    selected_families = sorted(families, key=lambda family: (_rank("family", family), family))[:maximum_families]
    selected: list[LookaheadSeed] = []
    for family in selected_families:
        # Interleave the existing actions so many KEEP contexts cannot hide all
        # positive lookahead seeds in the bounded reconstruction sample.
        action_index: Counter[str] = Counter()
        ranked: list[tuple[int, bytes, str, LookaheadSeed]] = []
        for seed in sorted(families[family], key=lambda row: (_rank("seed", row.identifier), row.identifier)):
            ranked.append((action_index[seed.action], _rank("seed", seed.identifier), seed.identifier, seed))
            action_index[seed.action] += 1
        selected.extend(seed for *_rank, seed in sorted(ranked)[:seeds_per_family])
    counts.update({"selected_families": len(selected_families), "selected_seeds": len(selected)})
    proposals: dict[str, tuple[ContextEvidence, LookaheadAnchor, str]] = {}
    for seed in selected:
        item = seed.evidence
        group = 1 - item.source_group
        choices = available[group]
        planned_label = seed.planned_action or seed.action
        if planned_label == "convert":
            first = (item.field.after or seed.next_words).split()[0]
            choices = [anchor for anchor in choices if anchor.text == first]
        if not choices:
            counts["skipped_missing_anchor"] += 1
            continue
        anchor = min(choices, key=lambda row: (_rank("anchor:" + seed.identifier, row.identifier), row.identifier))
        next_original = translated(anchor.text, group)
        proposed = automatic_word_decision(
            detector, next_original, {group: anchor.text}, item.source_group,
            previous_words={item.source_group: item.original, group: item.alternative},
            context_group=item.source_group, trigger="space",
        )
        if not proposed.should_convert:
            counts["skipped_next_not_proposed"] += 1
            continue
        # The engine re-decides a waiting word with its planned next word as the only
        # language context; the planned frame carries that same baseline verdict.
        contextual = automatic_word_decision(
            detector, item.original, {group: item.alternative}, item.source_group,
            previous_words={group: anchor.text}, context_group=group, trigger="space",
        )
        candidate = replace(item, field=replace(item.field, after=anchor.text), after_origin="planned_next_conversion",
                            baseline_convert=contextual.should_convert, model_probability=contextual.model_probability,
                            model_threshold=contextual.model_threshold)
        labels[_fingerprint(candidate)].add(planned_label)
        proposals[seed.identifier] = candidate, anchor, next_original
    frames: list[LookaheadFrame] = []
    mass: defaultdict[str, float] = defaultdict(float)
    for seed in sorted(seeds, key=lambda row: row.identifier):
        proposal = proposals.get(seed.identifier)
        if proposal is not None and len(labels[_fingerprint(proposal[0])]) != 1:
            counts["skipped_label_conflict"] += 1
            proposal = None
        weight = seed.sample_weight if proposal is None else seed.sample_weight / PLANNED_VARIANT_MASS_DIVISOR
        frames.append(LookaheadFrame(seed.identifier, seed.parent_family, seed.evidence, seed.action, weight))
        mass[seed.evidence.after_origin + ":" + seed.action] += weight
        if proposal is not None:
            item, anchor, next_original = proposal
            planned_label = seed.planned_action or seed.action
            frames.append(LookaheadFrame(seed.identifier, seed.parent_family, item, planned_label, weight,
                                         "planned", anchor.identifier, anchor.parent_family, next_original))
            counts["planned_" + planned_label] += 1
            mass[item.after_origin + ":" + planned_label] += weight
    return LookaheadCurriculum(tuple(frames), dict(counts), dict(mass))
