#!/usr/bin/env python3
"""Inventory the words the prefix model's fitting inputs expose, as physical aliases.

The context-action holdout and fitting freezers refuse into their test any word the
prefix model was fitted on, because the two models are judged together on the same
sentences. This tool lists those words without scoring anything: it reads the
prefix-v1 train, development and calibration splits - the inputs of prefix-v2
training - and hashes every physical reading of each typed word and its whole-word
family. The prefix test split is not opened: its words trained nothing.
"""

from __future__ import annotations

import argparse
from collections import defaultdict
from collections.abc import Sequence
import gzip
import json
from pathlib import Path
from typing import cast

from freeze_context_action_corpus import canonical, checksum
from reconcile_context_action_corpus import expanded_aliases

ROOT = Path(__file__).resolve().parents[1]
PREFIX_CORPUS = ROOT / "model/prefix_v1"
CURRICULUM_SPLITS = ("train", "development", "calibration")
SCOPES = {"text": "typed word form", "family": "whole-word physical family"}
KIND = "actual-prefix-model-input-exposure"


def split_aliases(path: Path) -> dict[str, set[str]]:
    """Every physical alias of the words one split holds, by the field that holds them."""
    aliases: dict[str, set[str]] = {scope: set() for scope in SCOPES}
    with gzip.open(path, "rt", encoding="utf-8") as stream:
        for line in stream:
            row = cast(dict[str, object], json.loads(line))
            for scope in SCOPES:
                value = row.get(scope)
                if not isinstance(value, str) or not value:
                    raise ValueError(f"prefix row without a {scope}: {path.name}")
                aliases[scope].update(expanded_aliases(value))
    return aliases


def inventory(corpus: Path) -> dict[str, object]:
    by_split: dict[str, set[str]] = defaultdict(set)
    by_scope: dict[str, set[str]] = defaultdict(set)
    provenance = {}
    for split in CURRICULUM_SPLITS:
        path = corpus / (split + ".jsonl.gz")
        provenance[str(path.relative_to(ROOT)) if path.is_relative_to(ROOT) else str(path)] = checksum(path)
        for scope, aliases in split_aliases(path).items():
            by_split[split].update(aliases)
            by_scope[scope].update(aliases)
    manifest = corpus / "corpus.json"
    provenance[str(manifest.relative_to(ROOT)) if manifest.is_relative_to(ROOT) else str(manifest)] = checksum(manifest)
    aliases = set().union(*by_split.values())
    return {
        "schema_version": 1, "kind": KIND, "test_json_opened": False, "model_scoring": False,
        "scope": "expanded physical aliases of every typed word and whole-word family in the prefix-v1 "
                 "train, development and calibration splits, the inputs of prefix-v2 training; "
                 "the prefix test split was not opened",
        "scopes": SCOPES,
        "aliases_sha256": sorted(aliases),
        "aliases_by_split": {split: sorted(values) for split, values in sorted(by_split.items())},
        "aliases_by_scope": {scope: sorted(values) for scope, values in sorted(by_scope.items())},
        "counts": {"aliases": len(aliases), **{split: len(values) for split, values in sorted(by_split.items())}},
        "provenance": provenance,
    }


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, default=PREFIX_CORPUS)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    if args.output.exists():
        raise ValueError("refusing to overwrite an exposure inventory")
    result = inventory(args.corpus)
    args.output.write_bytes(canonical(result))
    print(json.dumps({"output": str(args.output), **cast(dict[str, object], result["counts"])}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
