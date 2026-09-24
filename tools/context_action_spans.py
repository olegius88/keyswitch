"""Bounded non-test frames captured from physical engine boundaries."""
from __future__ import annotations

from collections import Counter
from collections.abc import Sequence
from dataclasses import dataclass, replace
import hashlib
from pathlib import Path
from unittest.mock import patch

from keyswitch.context_model import ContextAction, ContextEvidence, ContextModel, ContextPrediction
from keyswitch.language_model import LanguageModel

from context_physical_keys import KEYS, PhysicalKey
from evaluate_context_action_sequences import SequencePlan, TracedEditor, replay
from freeze_context_action_corpus import CorpusRow
from keyswitch.constants.model_protocol import FITTING_SPLITS
from keyswitch.constants.keyboard import LAYOUT_GROUP_COUNT
from keyswitch.constants.models import CONTEXT_ACTION_FEATURE_VERSION
from keyswitch.constants.training import (
    SPAN_ANCHOR_MAX_CHARACTERS,
    SPAN_ANCHOR_MIN_CHARACTERS,
    SPAN_DEFAULT_MAXIMUM_FAMILIES,
    SPAN_MAXIMUM_FAMILIES,
    SPAN_ORIGINAL_MAX_CHARACTERS,
)


ROOT = Path(__file__).resolve().parents[1]
VARIANTS = (("", " "), (".", " "), (",", "  "), (";", "   "), ("...", "  "), ("!", "   "))


@dataclass(frozen=True)
class SpanFrame:
    evidence: ContextEvidence
    action: ContextAction
    parent_family: str
    source_identifier: str
    sequence_id: str
    sample_weight: float


@dataclass(frozen=True)
class SpanCurriculum:
    frames: tuple[SpanFrame, ...]
    counts: dict[str, int]
    profile: str
    maximum_families: int
    split: str


def provenance() -> dict[str, str]:
    names = ("tools/context_action_spans.py", "tools/context_physical_keys.py",
             "tools/evaluate_context_action_sequences.py", "src/keyswitch/engine.py",
             "tests/test_input_integrity.py", "tests/test_engine_behaviour.py")
    return {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest() for name in names}


def _rank(purpose: str, identity: str) -> bytes:
    return hashlib.sha256(("context-action-spans-v1:" + purpose + ":" + identity).encode()).digest()


def _keys(text: str, group: int) -> tuple[PhysicalKey, ...]:
    result = []
    for character in text:
        found = next((key for key in KEYS if key.characters[group] == character), None)
        if found is None:
            raise ValueError("unrepresentable span sequence")
        result.append(found)
    return tuple(result)


def _body(text: str, group: int) -> tuple[int, ...]:
    # Shift changes case/symbol glyphs but not the parent physical key family.
    return tuple(key.keycode for key in _keys(text, group))


def _select(rows: Sequence[CorpusRow], maximum: int) -> tuple[list[CorpusRow], dict[int, list[CorpusRow]]]:
    families: dict[str, CorpusRow] = {}
    anchors: dict[int, list[CorpusRow]] = {0: [], 1: []}
    for row in sorted(rows, key=lambda row: (_rank("source", row.identifier), row.identifier)):
        if row.group not in (0, 1) or not row.layout_representable or not 1 <= len(row.original) <= SPAN_ORIGINAL_MAX_CHARACTERS:
            continue
        if not any(character.isalpha() for character in row.original):
            continue
        try:
            _keys(row.original, row.group)
        except ValueError:
            continue
        families.setdefault(row.family, row)
        if row.original.isalpha() and SPAN_ANCHOR_MIN_CHARACTERS <= len(row.original) <= SPAN_ANCHOR_MAX_CHARACTERS:
            anchors[row.group].append(row)
    selected: list[CorpusRow] = []
    per_group: Counter[int] = Counter()
    for row in sorted(families.values(), key=lambda row: (_rank("family", row.family), row.family)):
        assert row.group is not None
        if not anchors[row.group] or per_group[row.group] >= maximum // LAYOUT_GROUP_COUNT:
            continue
        per_group[row.group] += 1
        selected.append(row)
    return selected, anchors


@dataclass(frozen=True)
class _Capture:
    evidence: ContextEvidence
    observed: str
    event: int


class _Recorder(ContextModel):
    def __init__(self) -> None:
        super().__init__({}, "context-v3-span-capture", feature_version=CONTEXT_ACTION_FEATURE_VERSION)
        self.backend: TracedEditor | None = None
        self.records: list[_Capture] = []

    def predict(self, item: ContextEvidence) -> ContextPrediction:
        assert self.backend is not None
        self.records.append(_Capture(item, self.backend.text, self.backend.serial))
        return ContextPrediction("keep", 1.0, (1.0, 0.0, 0.0, 0.0), self.version, True)


def _capture(plan: SequencePlan, models: dict[int, LanguageModel]) -> list[_Capture]:
    recorder = _Recorder()

    def backend() -> TracedEditor:
        value = TracedEditor()
        recorder.backend = value
        return value

    with patch("evaluate_context_action_sequences.TracedEditor", side_effect=backend):
        outcome = replay(plan, recorder, models)
    if outcome["error"] is not None or outcome["corrections"]:
        raise ValueError("span capture must complete without corrections")
    return recorder.records


def _label(capture: _Capture, expected: str) -> ContextAction | None:
    if capture.observed == expected:
        return "keep"
    item = capture.evidence
    suffix = item.literal_tail + item.boundary_text
    anchor = item.original + suffix
    if not anchor or not capture.observed.endswith(anchor):
        return None
    converted = capture.observed[:-len(anchor)] + item.alternative + suffix
    return "convert" if converted == expected else None


def build_span_curriculum(
    rows: Sequence[CorpusRow], models: dict[int, LanguageModel], *,
    profile: str, maximum_families: int = SPAN_DEFAULT_MAXIMUM_FAMILIES, expected_split: str = "train",
) -> SpanCurriculum:
    """Capture attainable labels without dictionary or candidate predictions.

    One wrong focus follows a correct anchor from the same declared split. Suffix keys
    are chosen in the observed focus layout, preserving their declared glyphs.
    Only a body with the exact parent's physical key identity is supervised.
    Each retained family has total sample weight one, before class weighting.
    No split loader, test membership lookup or model-quality scoring is used.
    """
    if (type(maximum_families) is not int or not LAYOUT_GROUP_COUNT <= maximum_families <= SPAN_MAXIMUM_FAMILIES
            or maximum_families % LAYOUT_GROUP_COUNT):
        raise ValueError(f"span family budget must be an even integer from {LAYOUT_GROUP_COUNT} to {SPAN_MAXIMUM_FAMILIES}")
    if profile not in ("portable", "reference_hunspell") or set(models) != {0, 1}:
        raise ValueError("span curriculum requires an explicit lexical profile and both models")
    if expected_split not in FITTING_SPLITS:
        raise ValueError("span curriculum never accepts the test split")
    if any(row.split != expected_split or row.quarantine_reasons for row in rows):
        raise ValueError("span curriculum accepts only rows from the declared split")
    selected, anchors = _select(rows, maximum_families)
    counts: Counter[str] = Counter({"input_rows": len(rows), "selected_families": len(selected)})
    frames: list[SpanFrame] = []
    for row in selected:
        assert row.group is not None
        group = row.group
        counts["selected_group_" + str(group)] += 1
        anchor = min(anchors[group], key=lambda other: (_rank("anchor:" + row.family, other.identifier), other.identifier))
        parent_body = _body(row.original, group)
        for variant, (tail, spacing) in enumerate(VARIANTS):
            before = anchor.original + spacing
            for wrong in (False, True):
                source = 1 - group if wrong else group
                prefix_keys = _keys(before, group)
                focus_keys = _keys(row.original, group)
                tail_keys = _keys(tail + " ", source)
                keys = list(prefix_keys + focus_keys + tail_keys)
                keys[0] = replace(keys[0], explicit_group=group)
                keys[len(prefix_keys)] = replace(keys[len(prefix_keys)], explicit_group=source)
                expected = before + row.original + tail + " "
                identity = hashlib.sha256((row.identifier + ":" + str(variant) + ":" + str(int(wrong))).encode()).hexdigest()
                plan = SequencePlan(identity, row.document, group, False, expected,
                                    tuple(keys), (), 0, 0, source, None)
                records = _capture(plan, models)
                counts["sequences"] += 1
                focus_records = [record for record in records if record.event > len(prefix_keys)]
                counts["no_focus_call"] += not focus_records
                for record in focus_records:
                    item = record.evidence
                    counts["focus_calls"] += 1
                    if _body(item.original, item.source_group) != parent_body:
                        counts["skipped_nonparent_body"] += 1
                        continue
                    action = _label(record, expected[:record.event])
                    if action is None:
                        counts["skipped_inexpressible"] += 1
                        continue
                    frames.append(SpanFrame(item, action, row.family, row.identifier, identity, 0.0))
                    counts["action_" + action] += 1
                    counts["tail_nonempty"] += bool(item.literal_tail)
                    counts["repeated_space_before"] += item.field.before.endswith("  ")
    family_counts = Counter(frame.parent_family for frame in frames)
    weighted = tuple(replace(frame, sample_weight=1.0 / family_counts[frame.parent_family]) for frame in frames)
    counts["retained_families"] = len(family_counts)
    counts["frames"] = len(weighted)
    return SpanCurriculum(weighted, dict(counts), profile, maximum_families, expected_split)
