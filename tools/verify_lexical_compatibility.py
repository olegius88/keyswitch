"""Verify one audited lexical-config transition without changing frozen evidence.

This attests unchanged lexical inputs and frozen bytes, not a new fit, a new
independent evaluation, or current engine quality. Trust anchors are reviewed
constants; a self-consistent replacement receipt cannot authorize new artifacts.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from collections.abc import Sequence
from pathlib import Path
from typing import cast
from keyswitch.constants.model_protocol import ACTIVE_SPLITS
import sys
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))
from keyswitch.constants.file_formats import (
    HASH_CHUNK_BYTES,
    METADATA_JSON_LIMIT_BYTES,
    REPORT_JSON_INDENT,
)

ROOT = Path(__file__).resolve().parents[1]
CONFIG = "model/intent_v1/config.json"
GENERATION_CONFIG = "model/intent_v1/compatibility/generation-config-v21.json"
GENERATION_REF = "ad4c1af:model/intent_v1/config.json"
GENERATION_SHA256 = "2a8105d749422e9a8ef2bf6894d848ee8fdaff11df948d42a952fea27eda4116"
AUDITED_CONFIG_SHA256 = "76fde35bec793c2c1d6a168753e28cb1cb0c62ac63fa1da40a6d36c85c7fd532"
LEXICAL_SHA256 = "42f4b35de9f96a30f570b116326d20079da665fad4ca53728f1b1185e52cf7fc"
SCOPE = "Unchanged consumed lexical configuration and source bytes; frozen numeric regression only. No new training, independent evaluation, or current-runtime acceptance."
# The anchors of corpora frozen before an audited transition of the intent configuration. The
# GENERATION_* and AUDITED_CONFIG_SHA256 values above describe the last such transition (the
# generation-21 configuration audited against generation 23). Both corpora it covered were refit on
# the configuration they now pin in their own receipts (model/prefix_v1/corpus.json and
# model/boundary_v2/corpus.json); their retired receipts are kept in
# model/*/generation-history/candidate1/lexical-compatibility.json. No corpus relies on a transition
# now, so the list is empty; a corpus is added here only with a newly audited transition.
ANCHORS: dict[str, dict[str, str]] = {}
TRAINERS: dict[str, str] = {}


def checksum(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(HASH_CHUNK_BYTES), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _object(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate compatibility metadata field")
        result[key] = value
    return result


def read_object(path: Path) -> dict[str, object]:
    with path.open("rb") as source:
        raw = source.read(METADATA_JSON_LIMIT_BYTES + 1)
    if len(raw) > METADATA_JSON_LIMIT_BYTES:
        raise ValueError("oversized compatibility metadata")
    value: object = json.loads(raw, object_pairs_hook=_object)
    if not isinstance(value, dict):
        raise ValueError("invalid compatibility metadata")
    return cast(dict[str, object], value)


def _mapping(value: object) -> dict[str, object]:
    if not isinstance(value, dict) or any(not isinstance(key, str) for key in value):
        raise ValueError("invalid lexical configuration")
    return cast(dict[str, object], value)


def lexical_contract(config: dict[str, object]) -> dict[str, object]:
    return {"languages": _mapping(config.get("sources")).get("languages"),
            "hunspell": _mapping(config.get("external_evaluation")).get("hunspell")}


def contract_sha256(config: dict[str, object]) -> str:
    raw = json.dumps(lexical_contract(config), ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(raw).hexdigest()


def lexical_files(config: dict[str, object]) -> dict[str, str]:
    languages = _mapping(_mapping(config.get("sources")).get("languages"))
    dictionaries = _mapping(_mapping(config.get("external_evaluation")).get("hunspell"))
    files: dict[str, str] = {}
    for locale in ("en_US", "ru_RU"):
        spec, dictionary = _mapping(languages.get(locale)), _mapping(dictionaries.get(locale))
        files[str(spec["path"])] = str(spec["sha256"])
        for extension, field in (("dic", "dictionary_sha256"), ("aff", "affix_sha256")):
            files[f"model/intent_v1/sources/hunspell/{locale}.{extension}"] = str(dictionary[field])
    return files


def _path(root: Path, relative: str) -> Path:
    path = root / relative
    if Path(relative).is_absolute() or not path.resolve().is_relative_to(root.resolve()):
        raise ValueError("compatibility path escapes project")
    return path


def _verify_files(root: Path, expected: dict[str, object]) -> None:
    for relative, digest in expected.items():
        if checksum(_path(root, relative)) != digest:
            raise ValueError("compatibility input changed: " + relative)


def expected_receipt(kind: str, generation_config: dict[str, object]) -> dict[str, object]:
    """Describe fixed audited anchors; this does not authorize or write a receipt."""
    if kind not in ANCHORS:
        raise ValueError("unsupported compatibility corpus")
    return {"schema_version": 1, "corpus": kind, "scope": SCOPE,
            "generation_config": {"path": GENERATION_CONFIG, "git_object": GENERATION_REF, "sha256": GENERATION_SHA256},
            "audited_config": {"path": CONFIG, "sha256": AUDITED_CONFIG_SHA256},
            "consumed_contract_sha256": LEXICAL_SHA256,
            "frozen_artifacts": {f"model/{kind}/{name}": digest for name, digest in ANCHORS[kind].items()},
            "lexical_files": lexical_files(generation_config)}


def verify(kind: str, *, root: Path = ROOT) -> dict[str, object]:
    if kind not in ANCHORS:
        raise ValueError("unsupported compatibility corpus")
    directory = root / "model" / kind
    receipt = read_object(directory / "lexical-compatibility.json")
    _verify_files(root, {GENERATION_CONFIG: GENERATION_SHA256, CONFIG: AUDITED_CONFIG_SHA256})
    generation = read_object(root / GENERATION_CONFIG)
    current = read_object(root / CONFIG)
    if (lexical_contract(generation) != lexical_contract(current)
            or contract_sha256(generation) != LEXICAL_SHA256):
        raise ValueError("consumed lexical contract changed")
    if type(receipt.get("schema_version")) is not int or receipt != expected_receipt(kind, generation):
        raise ValueError("unapproved lexical compatibility receipt")
    _verify_files(root, {f"model/{kind}/{name}": digest for name, digest in ANCHORS[kind].items()})
    _verify_files(root, dict(lexical_files(current)))
    corpus = read_object(directory / "corpus.json")
    frozen = _mapping(corpus.get("sha256"))
    if set(frozen) != set(ACTIVE_SPLITS):
        raise ValueError("incomplete frozen partitions")
    _verify_files(root, {f"model/{kind}/{split}.jsonl.gz": frozen[split] for split in ACTIVE_SPLITS})
    original_provenance = _mapping(corpus.get("provenance"))
    if original_provenance.get(CONFIG) != GENERATION_SHA256:
        raise ValueError("corpus generation configuration changed")
    _verify_files(root, {relative: digest for relative, digest in original_provenance.items() if relative != CONFIG})
    seal = read_object(directory / "seal.json")
    if seal.get("provenance") != {
        "corpus": ANCHORS[kind]["corpus.json"], "trainer": checksum(_path(root, TRAINERS[kind])),
        "optimizer": checksum(root / "tools/context_optimizer.c"),
    }:
        raise ValueError("frozen trainer or optimizer changed")
    return {"corpus": kind, "compatible": True, "consumed_contract_sha256": LEXICAL_SHA256,
            "scope": SCOPE, "verified_partitions": list(ACTIVE_SPLITS)}


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("corpus", choices=tuple(ANCHORS))
    args = parser.parse_args(argv)
    print(json.dumps(verify(args.corpus), ensure_ascii=True, indent=REPORT_JSON_INDENT))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
