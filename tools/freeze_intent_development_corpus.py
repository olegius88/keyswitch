#!/usr/bin/env python3
"""Freeze the model-blind unknown-typo development corpus for training."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import tempfile
from collections import defaultdict
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import cast

import evaluate_intent_model as evaluator
from keyswitch.intent_model import CorrectionTrigger, TRIGGERS
from preseal_intent_holdout import build_model_blind_external_corpora
from train_intent_model import (
    HARD_NEGATIVE_ROLE_NAMESPACE,
    HARD_NEGATIVE_SOURCE_RELATIVE_PATH,
    LexicalExample,
)


_POLICY = "keyswitch-intent-v15-frozen-unknown-typo-development"
_PREFIX = "hunspell-unknown:"


def _parse_arguments(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--config",
        type=Path,
        default=Path("model/intent_v1/config.json"),
    )
    parser.add_argument(
        "--en-model",
        type=Path,
        default=Path("model/intent_v1/sources/en_US.lm"),
    )
    parser.add_argument(
        "--ru-model",
        type=Path,
        default=Path("model/intent_v1/sources/ru_RU.lm"),
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=Path(HARD_NEGATIVE_SOURCE_RELATIVE_PATH),
    )
    return parser.parse_args(argv)


def _expected_pair(
    *,
    trigger: str,
    base_signature: str,
    variant_kind: str,
    correct_group: int,
    correct_typo: str,
    wrong_typo: str,
) -> tuple[LexicalExample, LexicalExample]:
    typed_trigger = cast(CorrectionTrigger, trigger)
    return (
        LexicalExample(
            original=correct_typo,
            alternative=wrong_typo,
            source_group=correct_group,
            target_group=1 - correct_group,
            trigger=typed_trigger,
            label=False,
            weight=1.0,
            base_signature=base_signature,
            variant_kind=variant_kind,
            source_known=False,
            target_known=False,
        ),
        LexicalExample(
            original=wrong_typo,
            alternative=correct_typo,
            source_group=1 - correct_group,
            target_group=correct_group,
            trigger=typed_trigger,
            label=True,
            weight=1.0,
            base_signature=base_signature,
            variant_kind=variant_kind,
            source_known=False,
            target_known=False,
        ),
    )


def _compact_rows(
    examples: Sequence[LexicalExample],
) -> list[dict[str, object]]:
    grouped: dict[str, list[LexicalExample]] = defaultdict(list)
    for example in examples:
        grouped[example.base_signature].append(example)
    expected_rows_per_signature = len(TRIGGERS) * 2
    records: list[dict[str, object]] = []
    for base_signature, rows in grouped.items():
        if (
            not base_signature.startswith(_PREFIX)
            or len(rows) != expected_rows_per_signature
        ):
            raise RuntimeError("development corpus has an invalid signature group")
        by_key = {(row.trigger, row.label): row for row in rows}
        if len(by_key) != expected_rows_per_signature:
            raise RuntimeError("development corpus has duplicate trigger rows")
        anchor = by_key[("space", False)]
        if anchor.source_group not in (0, 1):
            raise RuntimeError("development corpus has an invalid language group")
        correct_group = anchor.source_group
        correct_typo = anchor.original
        wrong_typo = anchor.alternative
        variant_kind = anchor.variant_kind
        expected = {
            (row.trigger, row.label): row
            for trigger in TRIGGERS
            for row in _expected_pair(
                trigger=trigger,
                base_signature=base_signature,
                variant_kind=variant_kind,
                correct_group=correct_group,
                correct_typo=correct_typo,
                wrong_typo=wrong_typo,
            )
        }
        if by_key != expected:
            raise RuntimeError("development corpus rows are not trigger-symmetric")
        records.append(
            {
                "physical_signature": base_signature.removeprefix(_PREFIX),
                "correct_group": correct_group,
                "correct_typo": correct_typo,
                "wrong_typo": wrong_typo,
                "variant_kind": variant_kind,
            }
        )
    return sorted(
        records,
        key=lambda item: (
            cast(int, item["correct_group"]),
            cast(str, item["physical_signature"]),
        ),
    )


def _source_provenance(
    corpus: evaluator.LexicalDisjointCorpus,
) -> dict[str, object]:
    result: dict[str, object] = {}
    for group in (0, 1):
        item = corpus.dictionary_provenance[group]
        result[str(group)] = {
            "locale": item.locale,
            "dictionary_sha256": item.dictionary.sha256,
            "dictionary_bytes": item.dictionary.bytes,
            "affix_sha256": item.affix.sha256,
            "affix_bytes": item.affix.bytes,
        }
    return result


def build_frozen_development_corpus(
    config_path: Path,
    english_path: Path,
    russian_path: Path,
) -> dict[str, object]:
    corpora = build_model_blind_external_corpora(
        config_path,
        english_path,
        russian_path,
        verify_frozen_source=False,
    )
    development = corpora.development
    records = _compact_rows(development.examples)
    signatures = frozenset(
        cast(str, record["physical_signature"]) for record in records
    )
    if signatures != corpora.development_signatures:
        raise RuntimeError("compact development signatures changed during freezing")
    counts = {
        str(group): sum(
            cast(int, record["correct_group"]) == group for record in records
        )
        for group in (0, 1)
    }
    if counts != {
        str(group): development.words_by_group[group] for group in (0, 1)
    }:
        raise RuntimeError("compact development group counts are inconsistent")
    return {
        "schema_version": 1,
        "policy": _POLICY,
        "role_namespace": HARD_NEGATIVE_ROLE_NAMESPACE,
        "rank_namespace": development.rank_namespace,
        "choice_namespace": development.choice_namespace,
        "expanded_corpus_sha256": development.corpus_sha256,
        "physical_signatures_sha256": (
            evaluator.physical_signature_set_sha256(signatures)
        ),
        "signature_count": len(signatures),
        "words_by_group": counts,
        "source_provenance": _source_provenance(development),
        "rows": records,
    }


def _canonical_bytes(value: Mapping[str, object]) -> bytes:
    return (
        json.dumps(
            value,
            ensure_ascii=False,
            indent=2,
            sort_keys=False,
            allow_nan=False,
        )
        + "\n"
    ).encode("utf-8")


def _atomic_write(path: Path, payload: bytes) -> None:
    if path.name in {"", ".", ".."}:
        raise ValueError("development corpus output must name a file")
    parent = path.parent.resolve(strict=True)
    destination = parent / path.name
    if destination.is_symlink():
        raise RuntimeError("development corpus output cannot be a symlink")
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{path.name}.", suffix=".tmp", dir=parent
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, destination)
        directory_descriptor = os.open(parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory_descriptor)
        finally:
            os.close(directory_descriptor)
    finally:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass


def main(argv: Sequence[str] | None = None) -> int:
    arguments = _parse_arguments(argv)
    output = cast(Path, arguments.output)
    corpus = build_frozen_development_corpus(
        cast(Path, arguments.config),
        cast(Path, arguments.en_model),
        cast(Path, arguments.ru_model),
    )
    payload = _canonical_bytes(corpus)
    _atomic_write(output, payload)
    print(
        json.dumps(
            {
                "path": str(output),
                "bytes": len(payload),
                "sha256": hashlib.sha256(payload).hexdigest(),
                "expanded_corpus_sha256": corpus["expanded_corpus_sha256"],
                "signature_count": corpus["signature_count"],
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
