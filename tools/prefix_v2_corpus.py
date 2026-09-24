"""Bounded counterfactual prefix curriculum from already exposed old splits.

Only existing correct words and their existing typo surfaces are reused.
Application/role variants receive both intended-layout states. This development
curriculum does not provide a new independent test or permission to promote.
"""
from __future__ import annotations

from collections import Counter, defaultdict
from collections.abc import Iterator, Sequence
from dataclasses import dataclass, replace
from functools import lru_cache
import gzip
import hashlib
import json
from pathlib import Path
from typing import cast

from keyswitch.early_switch import PrefixIndex, _dictionary_stems
from keyswitch.input_context import FieldContext, FieldRole
from keyswitch.language_model import LanguageModel
from keyswitch.layouts import LayoutPair
from keyswitch.prefix_model import PREFIX_FEATURE_VERSION, PrefixInput
from keyswitch.prefix_schema import features_for_version
from keyswitch.constants.models import CURRENT_PREFIX_FEATURE_VERSION, PREFIX_MAX_CHARACTERS

from keyswitch.constants.model_protocol import FITTING_SPLITS, PROFILES
from auxiliary_runtime_evidence import _inside, _lexical_paths
from context_corpus import ROOT
from prefix_corpus import DIRECTORY
from keyswitch.constants.corpus import (
    PREFIX_V2_AMBIGUOUS_POSITIVE_LABEL,
    PREFIX_V2_APPLICATION_NAME_MAX_CHARACTERS,
    PREFIX_V2_CONTEXT_BEFORE_MAX_CHARACTERS,
    PREFIX_V2_DEFAULT_WORDS_PER_FAMILY,
    PREFIX_V2_INDEX_CACHE_SIZE,
    PREFIX_V2_LEGACY_DELETION_INDEX,
    PREFIX_V2_MAXIMUM_FAMILIES_LIMIT,
    PREFIX_V2_MAXIMUM_WORDS_PER_FAMILY,
    PREFIX_V2_MAX_CONTEXTS_PER_PARENT,
    PREFIX_V2_PARENT_BEFORE_MAX_CHARACTERS,
    PREFIX_V2_PARENT_FAMILY_MAX_CHARACTERS,
    PREFIX_V2_PARENT_IDENTIFIER_LIMIT,
    PREFIX_V2_PARENT_TEXT_MAX_CHARACTERS,
    PREFIX_V2_SHORT_PREFIX_CHARACTERS,
)
from keyswitch.constants.file_formats import HASH_CHUNK_BYTES

SELECTION_NAMESPACE = "keyswitch:prefix-v2:exposed-parent-selection:1"
SAFETY_CATEGORIES = ("identifier", "command", "mixed_english")
SOURCE_PATHS = (
    "tools/auxiliary_runtime_evidence.py", "tools/context_corpus.py",
    "tools/context_evidence.py", "tools/reference_lexicon.py", "tools/context_frames.py",
    "tools/prefix_corpus.py", "tools/prefix_v2_corpus.py",
    "src/keyswitch/__init__.py", "src/keyswitch/backend.py",
    "src/keyswitch/context_action_features.py", "src/keyswitch/context_model.py",
    "src/keyswitch/detector.py", "src/keyswitch/early_switch.py",
    "src/keyswitch/history.py", "src/keyswitch/input_context.py",
    "src/keyswitch/intent_model.py", "src/keyswitch/language_model.py",
    "src/keyswitch/lexicon_supplement.py", "src/keyswitch/resources/lexicon-supplement-ru_RU.json",
    "src/keyswitch/layouts.py", "src/keyswitch/prefix_model.py", "src/keyswitch/prefix_schema.py",
    "src/keyswitch/spellcheck.py",
    # The transitive detector import reads this resource immediately.
    "src/keyswitch/resources/protected_tokens.txt",
    "model/intent_v1/config.json",
)


@dataclass(frozen=True)
class ParentWord:
    original: str
    group: int
    family: str
    before: str
    identifier: str
    category: str
    parent_identifier: str = ""
    application: str = ""
    role: FieldRole = "unknown"


@dataclass(frozen=True)
class PrefixContext:
    identifier: str
    before: str
    application: str
    role: FieldRole


@dataclass(frozen=True)
class PrefixFrame:
    item: PrefixInput
    label: int
    parent_family: str
    sequence_id: str
    length: int
    desired: bool
    category: str
    profile: str
    sample_weight: float
    values: dict[str, float]
    pair_id: str
    normalization_key: str
    full_length: int
    feature_version: int = PREFIX_FEATURE_VERSION


def _digest(value: object) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, ensure_ascii=False,
                                     separators=(",", ":")).encode()).hexdigest()


def _checksum(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(HASH_CHUNK_BYTES), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _object(value: object) -> dict[str, object]:
    if not isinstance(value, dict) or any(not isinstance(key, str) for key in value):
        raise ValueError("invalid old prefix corpus object")
    return cast(dict[str, object], value)


def _split_path(split: str, directory: Path) -> Path:
    if split not in FITTING_SPLITS:
        raise ValueError("prefix-v2 may read only exposed train/development/calibration splits")
    path = directory / (split + ".jsonl.gz")
    if path.is_symlink() or not path.resolve(strict=True).is_relative_to(directory.resolve(strict=True)):
        raise ValueError("old prefix split must remain inside its source directory")
    return path


def _old_rows(split: str, directory: Path) -> Iterator[dict[str, object]]:
    # Reject test before opening even the manifest or resolving the directory.
    path = _split_path(split, directory)
    receipt = _object(json.loads((directory / "corpus.json").read_bytes()))
    hashes = _object(receipt.get("sha256"))
    expected = hashes.get(split)
    if not isinstance(expected, str) or _checksum(path) != expected:
        raise ValueError("old prefix split checksum mismatch")
    with gzip.open(path, "rt", encoding="utf-8") as stream:
        for line in stream:
            yield _object(json.loads(line))
    if _checksum(path) != expected:
        raise ValueError("old prefix split changed while reading")


def _parent(row: dict[str, object], split: str) -> ParentWord:
    if (type(row.get("sequence")) is not int or cast(int, row["sequence"]) < 0
            or type(row.get("source")) is not int or row["source"] not in (0, 1)
            or any(not isinstance(row.get(key), str) for key in ("text", "family", "before", "category"))):
        raise ValueError("invalid old prefix parent metadata")
    text, family, before = str(row["text"]), str(row["family"]), str(row["before"])
    if (not text or len(text) > PREFIX_V2_PARENT_TEXT_MAX_CHARACTERS or not family
            or len(family) > PREFIX_V2_PARENT_FAMILY_MAX_CHARACTERS or len(before) > PREFIX_V2_PARENT_BEFORE_MAX_CHARACTERS):
        raise ValueError("unbounded old prefix parent metadata")
    application, role = row.get("application", ""), row.get("role", "unknown")
    if (not isinstance(application, str) or len(application) > PREFIX_V2_APPLICATION_NAME_MAX_CHARACTERS
            or role not in ("unknown", "text", "code", "search", "terminal")):
        raise ValueError("invalid old prefix application metadata")
    return ParentWord(text, row["source"], family, before,
                      "prefix-v1:" + split + ":" + str(row["sequence"]), str(row["category"]),
                      application=application, role=role)


def load_parents(split: str, max_families: int, max_words_per_family: int = PREFIX_V2_DEFAULT_WORDS_PER_FAMILY,
                 *, directory: Path = DIRECTORY) -> list[ParentWord]:
    """Hash-select natural parents and matching existing typo forms, never test."""
    if split not in FITTING_SPLITS:
        raise ValueError("prefix-v2 may read only exposed train/development/calibration splits")
    if (type(max_families) is not int or not 1 <= max_families <= PREFIX_V2_MAXIMUM_FAMILIES_LIMIT
            or type(max_words_per_family) is not int or not 1 <= max_words_per_family <= PREFIX_V2_MAXIMUM_WORDS_PER_FAMILY):
        raise ValueError("invalid bounded prefix parent budget")
    natural: dict[str, list[ParentWord]] = defaultdict(list)
    typos: dict[tuple[str, int, str, str], ParentWord] = {}
    safety: dict[tuple[str, str, int, str, str, str, FieldRole], ParentWord] = {}
    identifiers: set[str] = set()
    for row in _old_rows(split, directory):
        if row.get("length") != 1 or row.get("profile") != "portable" or row.get("category") not in ("correct", "typo", *SAFETY_CATEGORIES):
            continue
        parent = _parent(row, split)
        if parent.identifier in identifiers:
            raise ValueError("duplicate old prefix parent identifier")
        identifiers.add(parent.identifier)
        if len(identifiers) > PREFIX_V2_PARENT_IDENTIFIER_LIMIT:
            raise ValueError("old prefix parent metadata exceeds the bounded source budget")
        if parent.category == "correct":
            natural[parent.family].append(parent)
        elif parent.category == "typo":
            typo_key = parent.family, parent.group, parent.before, parent.original
            previous = typos.get(typo_key)
            if previous is None or parent.identifier < previous.identifier:
                typos[typo_key] = parent
        else:
            safety_key = parent.family, parent.category, parent.group, parent.original, parent.before, parent.application, parent.role
            previous = safety.get(safety_key)
            if previous is None or parent.identifier < previous.identifier:
                safety[safety_key] = parent
    families = sorted(natural, key=lambda name: (_digest([SELECTION_NAMESPACE, name]), name))[:max_families]
    result: list[ParentWord] = []
    pair = LayoutPair()
    for family in families:
        candidates = sorted(natural[family], key=lambda item: (_digest([SELECTION_NAMESPACE, family, item.original, item.group]), item.identifier))
        selected: set[tuple[str, int]] = set()
        for parent in candidates:
            word_key = parent.original, parent.group
            if word_key in selected:
                continue
            selected.add(word_key)
            result.append(replace(parent, parent_identifier=parent.identifier))
            # Match the legacy deletion surface; do not synthesize a new row.
            typo = typos.get((parent.family, parent.group, parent.before,
                             parent.original[:PREFIX_V2_LEGACY_DELETION_INDEX] + parent.original[PREFIX_V2_LEGACY_DELETION_INDEX + 1:]))
            if typo is not None:
                result.append(replace(typo, identifier=typo.identifier + ":parent:" + parent.identifier,
                                      parent_identifier=parent.identifier))
            physical = pair.translate(parent.original, "ru", "us") if parent.group else parent.original
            source_contexts: list[tuple[str, str, str, str, FieldRole]] = [
                ("identifier", physical + "_value", "const value = ", "Code", "code"),
                ("command", physical, "$ ", "WindowsTerminal", "terminal"),
            ]
            if parent.group == 0:
                source_contexts.append(("mixed_english", parent.original, "в сообщении написано ", "Telegram", "text"))
            for category, text, before, app, role in source_contexts:
                existing = safety.get((parent.family, category, 0, text, before, app, role))
                if existing is not None:
                    result.append(replace(existing, identifier=existing.identifier + ":parent:" + parent.identifier,
                                          parent_identifier=parent.identifier))
            if len(selected) == max_words_per_family:
                break
    return result


def contexts_for(parent: ParentWord) -> tuple[PrefixContext, ...]:
    if parent.category in SAFETY_CATEGORIES:
        return (PrefixContext("existing-" + parent.category, parent.before, parent.application, parent.role),)
    comment = "// пояснение " if parent.group else "// explanation "
    shell = "# описание " if parent.group else "# description "
    return (
        PrefixContext("natural", parent.before, "Telegram", "text"),
        PrefixContext("empty", "", "TestEditor", "unknown"),
        PrefixContext("comment-code-unknown", comment, "Code", "unknown"),
        PrefixContext("comment-code", comment, "Code", "code"),
        PrefixContext("shell-unknown", shell, "WindowsTerminal", "unknown"),
        PrefixContext("shell-terminal", shell, "WindowsTerminal", "terminal"),
        PrefixContext("comment-telegram", comment, "Telegram", "text"),
        PrefixContext("chat-code-unknown", parent.before, "Code", "unknown"),
        PrefixContext("chat-code-text", parent.before, "Code", "text"),
    )


def _contexts(values: Sequence[PrefixContext]) -> tuple[PrefixContext, ...]:
    unique: dict[tuple[str, str, FieldRole], PrefixContext] = {}
    for item in sorted(values, key=lambda value: value.identifier):
        if (not item.identifier or len(item.before) > PREFIX_V2_CONTEXT_BEFORE_MAX_CHARACTERS or not item.application
                or len(item.application) > PREFIX_V2_APPLICATION_NAME_MAX_CHARACTERS
                or item.role not in ("unknown", "text", "code", "search", "terminal")):
            raise ValueError("invalid bounded prefix context")
        unique.setdefault((item.before, item.application, item.role), item)
    if not unique or len(unique) > PREFIX_V2_MAX_CONTEXTS_PER_PARENT:
        raise ValueError("invalid prefix context count")
    return tuple(unique[key] for key in sorted(unique))


@lru_cache(maxsize=PREFIX_V2_INDEX_CACHE_SIZE)
def _indexes(profile: str, first: LanguageModel, second: LanguageModel) -> dict[int, PrefixIndex]:
    # Keep the supplied frozen models; the runtime cache reloads system lexicons.
    return {
        group: PrefixIndex(set(model.frequencies) | (_dictionary_stems(Path(model.speller.source))
                           if profile == "reference_hunspell" else set()), model.frequencies)
        for group, model in enumerate((first, second))
    }


def generate_frames(parents: Sequence[ParentWord], models_by_profile: dict[str, dict[int, LanguageModel]],
                    *, contexts: Sequence[PrefixContext] | None = None,
                    maximum_prefix_length: int = PREFIX_MAX_CHARACTERS,
                    feature_version: int = PREFIX_FEATURE_VERSION) -> Iterator[PrefixFrame]:
    """Emit paired snapshots with unit total sampling mass per physical family.

    Each selected surface has equal family mass. Contexts, lexical profiles and
    correct/wrong states divide that mass equally, then prefixes divide their
    sequence mass. No class multiplier is applied here.
    """
    if (type(maximum_prefix_length) is not int or not 1 <= maximum_prefix_length <= PREFIX_MAX_CHARACTERS
            or "portable" not in models_by_profile or not set(models_by_profile) <= set(PROFILES)
            or any(set(models) != {0, 1} for models in models_by_profile.values())
            or type(feature_version) is not int
            or feature_version not in (1, CURRENT_PREFIX_FEATURE_VERSION)):
        raise ValueError("invalid prefix feature configuration")
    identifiers: set[str] = set()
    for parent in parents:
        if (parent.category not in ("correct", "typo", *SAFETY_CATEGORIES) or type(parent.group) is not int or parent.group not in (0, 1)
                or not parent.original or len(parent.original) > PREFIX_V2_PARENT_TEXT_MAX_CHARACTERS or not parent.family
                or not parent.identifier or parent.identifier in identifiers):
            raise ValueError("invalid or duplicate prefix parent")
        identifiers.add(parent.identifier)
    families = Counter(parent.family for parent in parents)
    indexes = {profile: _indexes(profile, models[0], models[1]) for profile, models in models_by_profile.items()}
    pair = LayoutPair()
    profiles = tuple(sorted(models_by_profile))
    for parent in parents:
        variants = _contexts(contexts_for(parent) if contexts is None or parent.category in SAFETY_CATEGORIES else contexts)
        states = (False, True) if parent.category == "correct" else (False,)
        length_count = min(len(parent.original), maximum_prefix_length)
        weight = 1.0 / (families[parent.family] * len(variants) * len(profiles) * len(states) * length_count)
        for context in variants:
            context_key = [context.before, context.application, context.role]
            for profile in profiles:
                pair_id = "prefix-v2-pair:" + _digest([parent.identifier, context_key, profile])
                for desired in states:
                    source = 1 - parent.group if desired else parent.group
                    full = pair.translate(parent.original, "ru" if parent.group else "us", "us" if parent.group else "ru") if desired else parent.original
                    if len(full) != len(parent.original):
                        raise ValueError("prefix translation changed the parent length")
                    sequence_id = "prefix-v2-sequence:" + _digest([pair_id, desired])
                    for length in range(1, length_count + 1):
                        original = full[:length]
                        alternate = pair.translate(original, "ru" if source else "us", "us" if source else "ru")
                        ambiguous = indexes["portable"][source].completions(original).completions > 0
                        label = (PREFIX_V2_AMBIGUOUS_POSITIVE_LABEL if length < PREFIX_V2_SHORT_PREFIX_CHARACTERS or ambiguous else 1) if desired else 0
                        item = PrefixInput(original, alternate, source,
                                           FieldContext(context.application, pair_id, context.before, role=context.role))
                        yield PrefixFrame(item, label, parent.family, sequence_id, length, desired,
                                          "wrong" if desired else parent.category, profile, weight,
                                          features_for_version(item, indexes[profile], models_by_profile[profile],
                                                               feature_version=feature_version),
                                          pair_id, parent.family, len(full), feature_version)


def provenance(root: Path = ROOT, directory: Path = DIRECTORY) -> dict[str, str]:
    """Pin the fit inputs, excluding engine, context trainers and model weights."""
    root = root.resolve(strict=True)
    directory = directory.resolve(strict=True)
    if not directory.is_relative_to(root):
        raise ValueError("prefix corpus provenance must stay inside the repository")
    receipt_path = _inside(root, directory / "corpus.json")
    receipt_bytes = receipt_path.read_bytes()
    receipt = _object(json.loads(receipt_bytes))
    hashes = _object(receipt.get("sha256"))
    splits = [_split_path(split, directory) for split in FITTING_SPLITS]
    lexical = _lexical_paths(root)
    paths = {_inside(root, Path(name)) for name in SOURCE_PATHS}
    paths.update(lexical)
    paths.update(_inside(root, path) for path in (receipt_path, directory / "config.json", *splits))
    result = {path.relative_to(root).as_posix(): _checksum(path) for path in sorted(paths)}
    if result[receipt_path.relative_to(root).as_posix()] != hashlib.sha256(receipt_bytes).hexdigest():
        raise ValueError("old prefix manifest changed while reading")
    for split, path in zip(FITTING_SPLITS, splits):
        if result[path.relative_to(root).as_posix()] != hashes.get(split):
            raise ValueError("old prefix split checksum mismatch")
    for path, expected in lexical.items():
        if result[path.relative_to(root).as_posix()] != expected:
            raise ValueError("reference lexical checksum mismatch")
    return result
