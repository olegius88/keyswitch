#!/usr/bin/env python3
"""Sealed context-action comparison through physical, visible-editor replay.

The input is a bounded natural sentence window, not native OS keyboard evidence.
Test access and its outcome are immutable, including failed experiments. Changing
the candidate, runtime, sampling or corpus after access requires a new holdout.
"""
from __future__ import annotations

import argparse
from collections import Counter
from collections.abc import Iterable, Mapping, Sequence
from dataclasses import dataclass, replace
from functools import lru_cache
import hashlib
import json
import math
import os
from pathlib import Path
import re
import sys
import tempfile
from typing import cast
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT / "tests") not in sys.path:
    sys.path.insert(0, str(ROOT / "tests"))

from keyswitch.backend import KeyEvent, SHIFT_MASK
from keyswitch.config import SettingsStore
from keyswitch.context_model import ContextModel
from keyswitch.engine import KeySwitchEngine
from keyswitch.history import HistoryStore
from keyswitch.intent_model import IntentModelStatus, LinearNgramModel
from keyswitch.language_model import LanguageModel
from keyswitch.prefix_model import PrefixModel
from keyswitch.prefix_schema import VersionedPrefixModel
from test_input_integrity import EditorBackend

from reference_lexicon import reference_models
from context_physical_keys import KEYS as KEYS, PhysicalKey as PhysicalKey, physical_keys as physical_keys
from freeze_context_action_corpus import CorpusRow, canonical, checksum, digest, load_split

CORPUS = ROOT / ".t/reliable-release-2026-09-12/context-action-corpus"
LEDGER_ROOT = ROOT / ".t/reliable-release-2026-09-12/context-action-test-ledger"
BASELINE = ROOT / "model/context_v3/baseline-context-v1.json"
PREFIX_BASELINE = ROOT / "model/prefix_v2/baseline-prefix-v1.json"
INSTALLED_MODELS = ("context_policy_v1.json", "prefix_policy_v1.json")
SETTINGS_MODES = ("early_off", "default")
DEFAULT_KEY_DOWN_SECONDS = 0.05
DEFAULT_KEY_UP_SECONDS = 0.03
DEFAULT_WORD_IDLE_SECONDS = 1.7
PAIRS: dict[str, tuple[tuple[str, str, str], ...]] = {
    "early_off": (("baseline", "baseline", "baseline"), ("candidate", "candidate", "candidate")),
    "default": (("baseline", "baseline", "baseline"), ("candidate", "candidate", "candidate"),
                ("candidate_context_baseline_prefix", "candidate", "baseline")),
}
RECIPE = ROOT / "model/context_v3/recipe.json"
DOCUMENT_CAP = 128
PROFILES = ("portable", "reference_hunspell")
GATE_POLICY: dict[str, object] = {
    "calibration_false_conversions": 0,
    "minimum_calibration_conversion_recall": 0.9,
    "sequence_correct_text_corruptions": 0,
    "sequence_length_mismatches": 0,
    "sequence_net_restorations_at_least_baseline": True,
    "minimum_sequence_documents_per_group": 32,
}
REQUIRED_PROVENANCE = frozenset({
    "tools/train_context_action_model.py", "tools/freeze_context_action_corpus.py",
    "tools/evaluate_context_action_sequences.py", "tools/context_evidence.py", "tools/reference_lexicon.py",
    "tools/context_physical_keys.py",
    "tools/context_action_spans.py", "tests/test_context_action_spans.py",
    "tools/reconcile_context_action_corpus.py",
    "tools/action_epoch_selection.py", "tests/test_action_epoch_selection.py",
    "tests/test_context_action_training.py", "tests/test_default_input_sequences.py",
    "tests/test_context_after_origin.py", "tools/verify_context_model.py",
    "tools/context_technical_corpus.py", "tools/merge_context_action_corpora.py",
    "tests/test_context_technical_corpus.py", "tests/test_context_action_merge.py",
    "model/context_v3/recipe.json", "model/intent_v1/config.json",
    "src/keyswitch/context_action_features.py", "src/keyswitch/context_model.py",
    "src/keyswitch/context_policy.py", "src/keyswitch/word_decision.py",
    "src/keyswitch/short_words.py", "src/keyswitch/engine.py",
    "src/keyswitch/detector.py", "src/keyswitch/intent_model.py",
    "src/keyswitch/language_model.py", "src/keyswitch/layouts.py",
    "src/keyswitch/ortho_model.py", "src/keyswitch/input_context.py",
    "src/keyswitch/resources/models/layout_intent_v1.ksm",
    "src/keyswitch/resources/protected_tokens.txt",
    "src/keyswitch/identifier_lexicon.py", "src/keyswitch/resources/identifiers.json",
    "src/keyswitch/resources/lexicon-supplement-ru_RU.json",
    "model/context_v3/baseline-context-v1.json", "model/prefix_v2/baseline-prefix-v1.json",
    "src/keyswitch/prefix_model.py", "src/keyswitch/prefix_schema.py", "src/keyswitch/early_switch.py",
    "src/keyswitch/resources/models/ortho_v1.json",
    "model/intent_v1/sources/en_US.lm", "model/intent_v1/sources/ru_RU.lm",
    "model/intent_v1/sources/hunspell/en_US.dic", "model/intent_v1/sources/hunspell/en_US.aff",
    "model/intent_v1/sources/hunspell/ru_RU.dic", "model/intent_v1/sources/hunspell/ru_RU.aff",
    "tests/test_language_intent_regressions.py", "tests/test_input_sequence_matrix.py",
    "tests/test_context_policy.py",
})
PROTOCOL: dict[str, object] = {
    "version": 3,
    "document_cap_per_group": DOCUMENT_CAP,
    "selection": "hash-ranked documents and one hash-ranked focus per document/group, before screenability or scoring; no replacement of unsupported rows",
    "window": "before + original + after; spacing and literal_tail are overlapping metadata and are never appended",
    "trim": "at a full 96-character left or 64-character right clip beginning/ending inside letters, remove only that outer letter fragment; preserve focus and adjacent whitespace/punctuation",
    "keyboard": "US/RU key pairs including Shift; natural script changes and focus first-letter layout are explicit user switches; subsequent glyphs follow the live backend group",
    "intervention": "Type leading focus nonletters in intended layouts; explicitly select the intended or opposite layout immediately before the first letter of the declared US/RU group. Wrong mode must produce a different observed glyph at that letter. Nonletter-only focus has no wrong intervention.",
    "punctuation": "physical keys chosen in intended layout, including ambiguous literal signs; no extra terminator or synthetic idle correction",
    "profiles": list(PROFILES),
    "settings_modes": {
        "early_off": "context assist; early switch, learning, history, field reading and manual-layout cooldown disabled; context tracking enabled; 50 ms before every key-down, 30 ms before every key-up, engine timer callbacks after every key, 1.7 s idle with timer callbacks after every completed word, exactly as in the default mode, so the completed-word decision reaches a document that ends without a boundary key",
        "default": "application defaults with the early switch enabled explicitly, the configuration a user gets by turning that feature on: early switch from four letters, pause correction after 1.5 s, manual-layout protection, learning, context assist without field reading; 50 ms before every key-down, 30 ms before every key-up, engine timer callbacks after every key, 1.7 s idle with timer callbacks after every completed word; a layout selection is a manual switch only when it changes the live backend group",
    },
    "models": "candidate and baseline context and prefix artifacts are injected explicitly; installed context and prefix bytes are never loaded; boundary, orthotactic and intent models are the pinned packaged files",
    "pairs": {"early_off": ["baseline: baseline context + baseline prefix", "candidate: candidate context + candidate prefix"],
              "default": ["baseline: baseline context + baseline prefix", "candidate: candidate context + candidate prefix",
                          "candidate_context_baseline_prefix: ablation, reported but not gated"]},
    "acceptance": "every gate must hold for the candidate pair against the baseline pair in both settings modes and both lexical profiles",
    "identifier_lexicon": "src/keyswitch/resources/identifiers.json is lexical evidence for both readings of a token in the engine, the trainer and this replay alike; technical holdout rows therefore measure commands present in the shipped index, and behaviour on identifiers outside it is reported by a separate lexicon-blind development diagnostic",
    "restoration_gate": "net outcome: candidate exactly_restored minus correct_text_corruptions must reach the baseline pair's exactly_restored minus correct_text_corruptions, so restorations bought with corruptions of correct text do not raise the bar; candidate corruptions are separately gated to zero",
    "scope": "in-process EditorBackend integration; no IME, native key loss, real focus races, actual injection acknowledgement, Enter/Tab submission or OS E2E proof; natural KEEP labels are not reviewed keyboard-intent ground truth",
}


def object_value(value: object, name: str) -> dict[str, object]:
    if not isinstance(value, dict) or any(not isinstance(key, str) for key in value):
        raise ValueError(name + " must be an object")
    return cast(dict[str, object], value)


def read_object(path: Path) -> dict[str, object]:
    value: object = json.loads(path.read_bytes())
    return object_value(value, path.name)


def nonnegative_integer(value: object, name: str) -> int:
    if type(value) is not int or value < 0:
        raise ValueError(name + " must be a nonnegative integer")
    return value


def validate_calibration(value: object) -> None:
    aggregate = object_value(value, "calibration")
    profiles = object_value(aggregate.get("by_profile"), "calibration.by_profile")
    if set(profiles) != set(PROFILES):
        raise ValueError("calibration must contain both declared lexical profiles")
    counters: dict[str, dict[str, int]] = {}
    for name, raw in (("aggregate", aggregate), *profiles.items()):
        row = object_value(raw, "calibration." + name)
        counts = {field: nonnegative_integer(row.get(field), "calibration." + field)
                  for field in ("rows", "false_conversions", "converted_correctly", "convert_rows")}
        recall = row.get("conversion_recall")
        if (type(recall) not in (int, float) or not math.isfinite(cast(float, recall))
                or counts["convert_rows"] == 0 or counts["rows"] < counts["convert_rows"]
                or counts["converted_correctly"] > counts["convert_rows"]
                or recall != counts["converted_correctly"] / counts["convert_rows"]
                or counts["false_conversions"] != 0 or recall < 0.9):
            raise ValueError("calibration gate failed: " + name)
        counters[name] = counts
    if any(counters["aggregate"][key] != sum(counters[name][key] for name in PROFILES)
           for key in counters["aggregate"]):
        raise ValueError("calibration aggregate does not match its profiles")


def validate_candidate_seal(artifact: Path, seal_path: Path, corpus: Path) -> dict[str, object]:
    """Validate before reading test bytes; do not import the trainer."""
    seal = read_object(seal_path)
    if type(seal.get("schema_version")) is not int or seal["schema_version"] != 1:
        raise ValueError("unsupported candidate seal")
    if seal.get("stage") != "sealed-before-test" or seal.get("test_accessed") is not False:
        raise ValueError("candidate is not sealed before test")
    if seal.get("artifact_sha256") != checksum(artifact):
        raise ValueError("candidate artifact differs from seal")
    if seal.get("corpus_manifest_sha256") != checksum(corpus / "manifest.json"):
        raise ValueError("corpus differs from candidate seal")
    recipe = object_value(seal.get("recipe"), "recipe")
    if canonical(recipe) != canonical(read_object(RECIPE)):
        raise ValueError("recipe differs from candidate seal")
    if (canonical(seal.get("gate_policy")) != canonical(GATE_POLICY)
            or canonical(recipe.get("gate_policy")) != canonical(GATE_POLICY)
            or recipe.get("profiles") != list(PROFILES)):
        raise ValueError("candidate gate policy differs from required policy")
    hashes = object_value(seal.get("provenance"), "provenance")
    if not REQUIRED_PROVENANCE <= hashes.keys():
        raise ValueError("candidate seal omits required provenance")
    for name, expected in hashes.items():
        path = Path(name)
        if path.is_absolute() or ".." in path.parts or path.as_posix() != name:
            raise ValueError("candidate provenance must use repository-relative paths")
        if (not isinstance(expected, str) or not re.fullmatch(r"[a-f0-9]{64}", expected)
                or checksum(ROOT / path) != expected):
            raise ValueError("candidate provenance changed: " + name)
    validate_calibration(seal.get("calibration"))
    model = ContextModel.load(artifact)
    payload = read_object(artifact)
    weights_sha = payload.get("weights_sha256")
    if (model.feature_version != 3 or not isinstance(weights_sha, str)
            or model.version != "context-v3-" + weights_sha[:12]
            or seal.get("model_version") != model.version
            or seal.get("conversion_threshold") != model.conversion_threshold):
        raise ValueError("candidate model identity differs from seal")
    return seal


def validate_prefix_seal(artifact: Path, seal_path: Path, *, require_calibration: bool) -> dict[str, object]:
    """Validate a prefix candidate seal; calibration is mandatory before a sealed test."""
    seal = read_object(seal_path)
    if type(seal.get("schema_version")) is not int or seal["schema_version"] != 1:
        raise ValueError("unsupported prefix seal")
    if seal.get("promotion_accepted") is not False or seal.get("independent_test_evaluated") is not False:
        raise ValueError("prefix seal must not claim promotion or an evaluated test")
    if require_calibration and seal.get("calibration_passed") is not True:
        raise ValueError("prefix candidate did not pass calibration")
    if seal.get("candidate_sha256") != checksum(artifact):
        raise ValueError("prefix artifact differs from seal")
    hashes = object_value(seal.get("provenance"), "prefix provenance")
    for name, expected in hashes.items():
        path = Path(name)
        if path.is_absolute() or ".." in path.parts or path.as_posix() != name:
            raise ValueError("prefix provenance must use repository-relative paths")
        if (not isinstance(expected, str) or not re.fullmatch(r"[a-f0-9]{64}", expected)
                or checksum(ROOT / path) != expected):
            raise ValueError("prefix provenance changed: " + name)
    object_value(seal.get("recipe"), "prefix recipe")
    model = VersionedPrefixModel.load(artifact)
    if seal.get("model_version") != model.version or seal.get("threshold") != model.model.conversion_threshold:
        raise ValueError("prefix model identity differs from seal")
    return seal


def runtime_provenance() -> dict[str, str]:
    # Replay injects both context models and both prefix models explicitly; the
    # installed context and prefix files are never loaded. Their replacement
    # during promotion cannot invalidate this comparison against the separately
    # preserved baseline artifacts.
    paths = set((ROOT / "src/keyswitch").rglob("*.py"))
    paths.update(path for path in (ROOT / "src/keyswitch/resources/models").iterdir()
                 if path.is_file() and path.name not in INSTALLED_MODELS)
    paths.update(ROOT / path for path in REQUIRED_PROVENANCE)
    paths.update(ROOT / "tests" / path for path in (
        "test_input_integrity.py", "test_engine_behaviour.py", "test_windows_backend.py", "test_x11_backend.py"))
    return {path.relative_to(ROOT).as_posix(): checksum(path) for path in sorted(paths)}


def immutable_record(path: Path, value: object) -> bool:
    """Atomically create a record or verify the exact existing bytes."""
    encoded = canonical(value)
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=path.parent, prefix=".pending-", delete=False) as stream:
        temporary = Path(stream.name)
        stream.write(encoded)
        stream.flush()
        os.fsync(stream.fileno())
    try:
        try:
            os.link(temporary, path)
        except FileExistsError:
            if path.read_bytes() != encoded:
                raise ValueError("immutable test record differs: " + path.name) from None
            return False
        return True
    finally:
        temporary.unlink()


def evaluation_identity(artifact: Path, seal: Path, corpus: Path, prefix: Path, prefix_seal: Path) -> dict[str, object]:
    return {"schema_version": 1, "artifact_sha256": checksum(artifact),
            "candidate_seal_sha256": checksum(seal), "corpus_manifest_sha256": checksum(corpus / "manifest.json"),
            "test_membership": test_membership(corpus),
            "baseline_sha256": checksum(BASELINE),
            "prefix_candidate_sha256": checksum(prefix), "prefix_candidate_seal_sha256": checksum(prefix_seal),
            "prefix_baseline_path": PREFIX_BASELINE.relative_to(ROOT).as_posix(),
            "prefix_baseline_sha256": checksum(PREFIX_BASELINE),
            "settings_modes": list(SETTINGS_MODES),
            "runtime": runtime_provenance(), "protocol": PROTOCOL}


def test_membership(corpus: Path) -> dict[str, list[str]]:
    manifest = read_object(corpus / "manifest.json")
    path = corpus / "test-membership.json"
    if manifest.get("test_membership_sha256") != checksum(path):
        raise ValueError("test membership differs from manifest")
    membership = read_object(path)
    result: dict[str, list[str]] = {}
    for field in ("row_ids_sha256", "family_ids_sha256", "document_ids_sha256"):
        values = membership.get(field)
        if (not isinstance(values, list) or not values
                or any(not isinstance(value, str) or not re.fullmatch(r"[a-f0-9]{64}", value) for value in values)):
            raise ValueError("invalid test membership: " + field)
        entries = cast(list[str], values)
        if entries != sorted(set(entries)):
            raise ValueError("test membership must be sorted and unique")
        result[field] = entries
    return result


def membership_key(identity: Mapping[str, object]) -> str:
    # Source membership survives diagnostic/namespace/compression metadata
    # changes. A different technical manifest cannot make the same test unseen.
    return hashlib.sha256(canonical(identity["test_membership"])).hexdigest()


def claim_test_access(identity: dict[str, object]) -> str:
    key = membership_key(identity)
    membership = object_value(identity["test_membership"], "test membership")
    LEDGER_ROOT.mkdir(parents=True, exist_ok=True)
    lock = LEDGER_ROOT / ".access-lock"
    try:
        descriptor = os.open(lock, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    except FileExistsError:
        raise ValueError("test access ledger is locked; no test was read") from None
    try:
        os.close(descriptor)
        for path in sorted(LEDGER_ROOT.glob("*.access.json")):
            if path.name == key + ".access.json":
                continue
            prior = object_value(read_object(path).get("test_membership"), "prior test membership")
            if any(set(cast(list[str], membership[field])) & set(cast(list[str], prior[field]))
                   for field in membership):
                raise ValueError("test overlaps previously accessed rows, families or documents")
        immutable_record(LEDGER_ROOT / (key + ".access.json"), identity)
    finally:
        lock.unlink()
    return key


def script_group(character: str) -> int | None:
    if "a" <= character.lower() <= "z":
        return 0
    if "а" <= character.lower() <= "я" or character.lower() == "ё":
        return 1
    return None


def trimmed_window(row: CorpusRow) -> tuple[str, str, int, int]:
    before, after = row.before, row.after
    left = right = 0
    if len(before) == 96 and before[0].isalpha():
        while left < len(before) and before[left].isalpha():
            left += 1
        before = before[left:]
    if len(after) == 64 and after[-1].isalpha():
        while right < len(after) and after[-1 - right].isalpha():
            right += 1
        after = after[:-right]
    return before, after, left, right


@dataclass(frozen=True)
class SequencePlan:
    identifier: str
    document: str
    group: int | None
    initially_wrong: bool
    expected: str
    keys: tuple[PhysicalKey, ...]
    unsupported: tuple[str, ...]
    trimmed_left: int
    trimmed_right: int
    expected_final_group: int
    intervention_event: int | None


def sequence_plan(row: CorpusRow, initially_wrong: bool) -> SequencePlan:
    if initially_wrong and row.group not in (0, 1):
        raise ValueError("mixed or unclassified focus has no unique wrong layout")
    focus_letter = next((index for index, character in enumerate(row.original)
                         if row.group in (0, 1) and script_group(character) == row.group), None)
    if initially_wrong and focus_letter is None:
        raise ValueError("wrong intervention requires a focus letter in the declared group")
    before, after, left, right = trimmed_window(row)
    intended = next((group for character in before + row.original
                     if (group := script_group(character)) is not None), row.group or 0)
    keys: list[PhysicalKey] = []
    unsupported: set[str] = set()
    first = True
    intervention_event: int | None = None
    for phase, text in (("before", before), ("focus", row.original), ("after", after)):
        for index, character in enumerate(text):
            explicit: int | None = None
            group = script_group(character)
            if first or group is not None and group != intended:
                intended = intended if group is None else group
                explicit = intended
                first = False
            if phase == "focus" and index == 0 and row.group in (0, 1):
                intended = row.group
                explicit = row.group
            if phase == "focus" and index == focus_letter and row.group in (0, 1):
                intended = row.group
                explicit = 1 - row.group if initially_wrong else row.group
            candidates = [key for key in KEYS if key.characters[intended] == character]
            if not candidates:
                candidates = [key for key in KEYS if key.characters[1 - intended] == character]
                if candidates:
                    intended = 1 - intended
                    explicit = intended
            if not candidates:
                unsupported.add(f"U+{ord(character):04X}")
                continue
            if phase == "focus" and index == focus_letter and initially_wrong:
                intervention_event = len(keys) + 1
            keys.append(replace(candidates[0], explicit_group=explicit))
    return SequencePlan(row.identifier, row.document, row.group, initially_wrong,
                        before + row.original + after, tuple(keys), tuple(sorted(unsupported)), left, right, intended,
                        intervention_event)


def select_rows(rows: Sequence[CorpusRow]) -> list[CorpusRow]:
    selected: list[CorpusRow] = []
    for group in (0, 1, None):
        documents: dict[str, list[CorpusRow]] = {}
        for row in rows:
            if row.group == group:
                documents.setdefault(row.document, []).append(row)
        ranked = sorted(documents, key=lambda name: (digest("context-sequence:document:" + name), name))
        for document in ranked[:DOCUMENT_CAP]:
            selected.append(min(documents[document], key=lambda row: (digest("context-sequence:focus:" + row.identifier), row.identifier)))
    return selected


class TracedEditor(EditorBackend):
    def __init__(self) -> None:
        super().__init__()
        self.corrections: list[dict[str, object]] = []
        self.serial = 0

    def inject_correction(
        self, strokes: Iterable[KeyEvent], target_group: int,
        boundary: KeyEvent | None, source_group: int | None = None,
        late: Sequence[KeyEvent] = (), trailing: Sequence[KeyEvent] = (),
    ) -> int:
        before, initial, caret = self.text, self.group, self.caret
        result = super().inject_correction(strokes, target_group, boundary, source_group, late, trailing)
        self.corrections.append({"event": self.serial, "before": before, "after": self.text,
                                 "source_group": initial, "target_group": target_group,
                                 "actual_group": self.group, "caret_before": caret, "caret_after": self.caret})
        return result


@lru_cache(maxsize=1)
def _packaged_intent(path: Path, modified_ns: int, size: int) -> tuple[LinearNgramModel, IntentModelStatus]:
    # Stat fields invalidate this decoding cache; provenance binds file bytes.
    model = LinearNgramModel.load(path)
    return model, IntentModelStatus(True, path, model.model_version, model.checksum, None)


def replay(plan: SequencePlan, model: ContextModel, models: dict[int, LanguageModel], *,
           prefix: PrefixModel | None = None, mode: str = "early_off") -> dict[str, object]:
    if plan.unsupported:
        raise ValueError("unsupported physical sequence cannot be replayed")
    if mode not in SETTINGS_MODES:
        raise ValueError("unknown settings mode")
    if mode == "default" and prefix is None:
        raise ValueError("default settings replay requires an explicit prefix model")
    if plan.initially_wrong != (plan.intervention_event is not None):
        raise ValueError("sequence intervention metadata is inconsistent")
    if (plan.intervention_event is not None
            and (plan.group not in (0, 1) or not 1 <= plan.intervention_event <= len(plan.keys))):
        raise ValueError("sequence intervention event is invalid")
    intent_path = ROOT / "src/keyswitch/resources/models/layout_intent_v1.ksm"
    intent_stat = intent_path.stat()
    intent = _packaged_intent(intent_path, intent_stat.st_mtime_ns, intent_stat.st_size)
    with tempfile.TemporaryDirectory(prefix="keyswitch-context-sequence-") as temporary:
        directory = Path(temporary)
        settings = SettingsStore(directory / "settings.json")
        if mode == "default":
            # The shipped default turned the early switch off; this mode exists to measure it,
            # so it is enabled explicitly and the control mode below remains its counterpart.
            settings.set("detection.early_switch", True)
        if mode == "early_off":
            for setting, value in {
                "detection.context_policy": "assist", "detection.context_aware": True,
                "detection.context_read_field": False, "detection.early_switch": False,
                "detection.respect_manual_layout": False, "detection.learning": False,
                "general.keep_history": False, "diagnostics.technical_logging": False,
            }.items():
                settings.set(setting, value)
        backend = TracedEditor()
        clock = [1000.0]
        expected = plan.expected
        if mode == "default" and len(plan.keys) != len(expected):
            raise ValueError("default settings replay requires one key per expected character")

        def load(locale: str, extra_words: Iterable[str] = ()) -> LanguageModel:
            return models[0 if locale == "en_US" else 1]

        error: str | None = None
        intervention: dict[str, object] | None = None
        with patch("keyswitch.engine.LanguageModel.load", side_effect=load), \
                patch("keyswitch.engine.LinearNgramModel.try_load_default", return_value=intent), \
                patch("keyswitch.context_policy.ContextModel.try_load", return_value=(model, model.version)), \
                patch.object(VersionedPrefixModel, "default", return_value=prefix), \
                patch("keyswitch.engine.time.monotonic", side_effect=lambda: clock[0]):
            engine = KeySwitchEngine(settings, HistoryStore(directory / "history.jsonl"), backend)

            def timers() -> None:
                # The engine's idle loop runs exactly these callbacks when no key arrives.
                engine._expire_deferred_action()
                engine._expire_manual_correction()
                engine._poll_current_group()
                engine._maybe_correct_after_pause()
                engine._expire_learning_prompt()

            try:
                for serial, key in enumerate(plan.keys, 1):
                    backend.serial = serial
                    if key.explicit_group is not None:
                        backend.switch_group(key.explicit_group)
                    observed = key.characters[backend.group]
                    if serial == plan.intervention_event:
                        assert plan.group is not None
                        intended = key.characters[plan.group]
                        intervention = {"event": serial, "expected": intended, "observed": observed,
                                        "source_group": backend.group, "target_group": plan.group}
                        if observed == intended:
                            raise ValueError("wrong intervention did not change the focus letter")
                    clock[0] += DEFAULT_KEY_DOWN_SECONDS
                    event = KeyEvent(True, key.keycode, "space" if observed == " " else observed,
                                     observed, key.characters, backend.group, SHIFT_MASK if key.shift else 0,
                                     round(clock[0] * 1000))
                    backend.type(event)
                    engine._handle(event)
                    clock[0] += DEFAULT_KEY_UP_SECONDS
                    released = replace(event, pressed=False, timestamp=round(clock[0] * 1000))
                    backend.type(released)
                    engine._handle(released)
                    # Both modes run the engine's idle callbacks: without them a document that
                    # ends without a boundary key never reaches the completed-word decision, so
                    # the control mode would measure the early switch alone for such rows.
                    timers()
                    character = expected[serial - 1]
                    following = expected[serial:serial + 1]
                    if not character.isspace() and (not following or following.isspace()):
                        clock[0] += DEFAULT_WORD_IDLE_SECONDS
                        timers()
            except (AssertionError, ValueError, IndexError) as exception:
                error = type(exception).__name__ + ": " + str(exception)
        return {"mode": mode, "actual": backend.text, "exact": error is None and backend.text == plan.expected,
                "length_mismatch": len(backend.text) != len(plan.expected),
                "final_group": backend.group, "expected_final_group": plan.expected_final_group,
                "final_layout_matches": backend.group == plan.expected_final_group,
                "corrections": backend.corrections, "intervention": intervention, "error": error}


def profile_gates(candidate: Mapping[str, int], baseline: Mapping[str, int], documents: Mapping[str, int]) -> dict[str, bool]:
    return {"correct_text_preserved": candidate["correct_text_corruptions"] == 0,
            "length_preserved": candidate["length_mismatches"] == 0,
            "net_restorations_at_least_baseline": (candidate["exactly_restored"] - candidate["correct_text_corruptions"]
                                                  >= baseline["exactly_restored"] - baseline["correct_text_corruptions"]),
            "enough_documents_us": documents.get("0", 0) >= 32,
            "enough_documents_ru": documents.get("1", 0) >= 32,
            "execution_succeeded": candidate["execution_errors"] == baseline["execution_errors"] == 0,
            "correction_layouts_match": candidate["correction_layout_mismatches"] == 0}


def score_pair(plans: Sequence[SequencePlan], model: ContextModel, prefix: PrefixModel, models: dict[int, LanguageModel],
               mode: str) -> tuple[Counter[str], list[dict[str, object]]]:
    counts: Counter[str] = Counter({key: 0 for key in (
        "rows", "initially_correct", "initially_wrong", "preserved_correct", "exactly_restored",
        "correct_text_corruptions", "length_mismatches", "injections", "execution_errors",
        "correction_layout_mismatches", "final_layout_mismatches")})
    results: list[dict[str, object]] = []
    for plan in plans:
        if plan.unsupported:
            continue
        outcome = replay(plan, model, models, prefix=prefix, mode=mode)
        corrections = cast(list[dict[str, object]], outcome["corrections"])
        exact = outcome["exact"] is True
        counts["rows"] += 1
        counts["initially_wrong" if plan.initially_wrong else "initially_correct"] += 1
        counts["exactly_restored" if plan.initially_wrong else "preserved_correct"] += int(exact)
        counts["correct_text_corruptions"] += int(not plan.initially_wrong and not exact)
        counts["length_mismatches"] += int(outcome["length_mismatch"] is True)
        counts["execution_errors"] += int(outcome["error"] is not None)
        counts["injections"] += len(corrections)
        counts["correction_layout_mismatches"] += sum(item["actual_group"] != item["target_group"] for item in corrections)
        counts["final_layout_mismatches"] += int(outcome["final_layout_matches"] is not True)
        results.append({"identifier": plan.identifier, "document": plan.document, "group": plan.group,
                        "initially_wrong": plan.initially_wrong, "expected": plan.expected,
                        "trimmed_left": plan.trimmed_left, "trimmed_right": plan.trimmed_right, **outcome})
    return counts, results


def score_sequences(rows: Sequence[CorpusRow], candidate: ContextModel, baseline: ContextModel, *,
                    prefix_candidate: PrefixModel, prefix_baseline: PrefixModel) -> dict[str, object]:
    """Replay every pair in both settings modes; acceptance needs every gate in every mode and profile."""
    selected = select_rows(rows)
    plans = [sequence_plan(row, wrong) for row in selected
             for wrong in ((False, True) if row.group in (0, 1) else (False,))]
    documents = {str(group): len({plan.document for plan in plans if plan.group == group and not plan.unsupported})
                 for group in (0, 1, None)}
    selection: dict[str, object] = {
        "source_ids": [row.identifier for row in selected], "selected_rows": len(selected),
        "replayed_documents": documents,
        "trimmed_rows": sum(bool(plan.trimmed_left or plan.trimmed_right) for plan in plans if not plan.initially_wrong),
        "unsupported": [{"identifier": plan.identifier, "codepoints": plan.unsupported}
                        for plan in plans if plan.unsupported and not plan.initially_wrong],
    }
    contexts = {"baseline": baseline, "candidate": candidate}
    prefixes = {"baseline": prefix_baseline, "candidate": prefix_candidate}
    profiles: dict[str, object] = {}
    all_passed = True
    for profile in PROFILES:
        models = reference_models(profile == "reference_hunspell")
        blocks: dict[str, dict[str, object]] = {}
        for mode in SETTINGS_MODES:
            totals: dict[str, Counter[str]] = {}
            cases: dict[str, list[dict[str, object]]] = {}
            for name, context_name, prefix_name in PAIRS[mode]:
                totals[name], cases[name] = score_pair(plans, contexts[context_name], prefixes[prefix_name], models, mode)
            gates = profile_gates(totals["candidate"], totals["baseline"], documents)
            all_passed = all_passed and all(gates.values())
            gated = {"counts": {name: totals[name] for name in ("baseline", "candidate")},
                     "cases": {name: cases[name] for name in ("baseline", "candidate")}, "gates": gates}
            ablation = {name: {"counts": totals[name], "cases": cases[name]}
                        for name in totals if name not in ("baseline", "candidate")}
            blocks[mode] = {**gated, "ablation": ablation} if ablation else gated
        profiles[profile] = {**blocks["default"], "early_off": blocks["early_off"]}
    return {"selection": selection, "profiles": profiles, "promotion_passed": all_passed}


def evaluate(artifact: Path, seal_path: Path, corpus: Path, split: str, output: Path, *,
             prefix: Path, prefix_seal: Path) -> dict[str, object]:
    if split not in {"development", "test"}:
        raise ValueError("sequence evaluator accepts development or sealed test only")
    destination = output.resolve()
    if (destination in (artifact.resolve(), seal_path.resolve(), prefix.resolve(), prefix_seal.resolve())
            or destination.is_relative_to(corpus.resolve()) or destination.is_relative_to(LEDGER_ROOT.resolve())):
        raise ValueError("report output must not overwrite sealed inputs or test ledgers")
    validate_candidate_seal(artifact, seal_path, corpus)
    validate_prefix_seal(prefix, prefix_seal, require_calibration=split == "test")
    identity = evaluation_identity(artifact, seal_path, corpus, prefix, prefix_seal)
    ledger_key = membership_key(identity)
    if split == "test":
        claim_test_access(identity)
    # No test file, decompression, model scoring or row selection occurs above
    # the access claim. Failed outcomes are recorded below without promotion.
    report: dict[str, object] = {"schema_version": 1, "split": split, "identity": identity,
                                "protocol": PROTOCOL, "gate_policy": GATE_POLICY, "promotion_passed": False}
    environment = {"KEYSWITCH_HUNSPELL_PATH": str(ROOT / "model/intent_v1/sources/hunspell"),
                   "KEYSWITCH_INTENT_MODEL_PATH": str(ROOT / "src/keyswitch/resources/models/layout_intent_v1.ksm")}
    try:
        rows = load_split(corpus, split)
        with patch.dict(os.environ, environment):
            report.update(score_sequences(rows, ContextModel.load(artifact), ContextModel.load(BASELINE),
                                          prefix_candidate=VersionedPrefixModel.load(prefix),
                                          prefix_baseline=VersionedPrefixModel.load(PREFIX_BASELINE)))
        if evaluation_identity(artifact, seal_path, corpus, prefix, prefix_seal) != identity:
            raise ValueError("evaluation inputs changed during sequence replay")
    except Exception as error:
        report["execution_error"] = type(error).__name__ + ": " + str(error)
        report["promotion_passed"] = False
        if split == "test":
            immutable_record(LEDGER_ROOT / (ledger_key + ".outcome.json"), report)
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_bytes(canonical(report))
        raise
    if split == "test":
        immutable_record(LEDGER_ROOT / (ledger_key + ".outcome.json"), report)
    else:
        report["promotion_passed"] = False
        report["scope"] = "development diagnostic; cannot promote a model"
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_bytes(canonical(report))
    return report


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--seal", type=Path, required=True)
    parser.add_argument("--prefix-candidate", type=Path, required=True)
    parser.add_argument("--prefix-seal", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, default=CORPUS)
    parser.add_argument("--split", choices=("development", "test"), default="development")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    report = evaluate(args.candidate, args.seal, args.corpus, args.split, args.output,
                      prefix=args.prefix_candidate, prefix_seal=args.prefix_seal)
    print(json.dumps({"split": args.split, "promotion_passed": report["promotion_passed"], "report": str(args.output)}))
    return 0 if args.split == "development" or report["promotion_passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
