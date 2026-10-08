#!/usr/bin/env python3
"""Print one digest of everything the strict intent-model evaluation reads.

`tools/evaluate_intent_model.py --strict` takes six to twelve minutes of a CI runner and
is the longest step of the test workflow, while its inputs last changed with 0.33.0. CI
keeps the strict report under this digest and hands the kept report on while the digest
stays the same; a report kept for other inputs is never looked up.

The inputs were read off an audit trace of a whole strict run (every `open` and every
`import` of the evaluator and of its row-scoring workers): the files of
`model/intent_v1/`, the shipped intent artifact, the protected-token list, the modules of
`src/keyswitch/` the evaluation imports, the intent toolchain under `tools/`, and the
values those files import from `keyswitch.constants`. The values are pinned the way
`tools/verify_intent_strict_report.py` pins them (`keyswitch.value_provenance`), so a
constant the evaluation does not use can change without a new evaluation, and one it uses
cannot. This file and the verifier are inputs too. The system Hunspell dictionaries, the
onboard-data models and the interpreter are not files of the tree: the workflow adds them
to the cache key itself.

A kept report still has to pass `tools/verify_intent_strict_report.py` before anything
uses it, so a digest that missed an input could only make CI re-verify an old report
against the tree, never accept one the verifier would refuse.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from collections.abc import Iterable, Sequence
from pathlib import Path
from typing import Final

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))
from keyswitch.constants.model_protocol import INTENT_TOOLCHAIN_VALUE_SOURCES
from keyswitch.value_provenance import ValueProvenanceError, pin_values


PROJECT_ROOT: Final[Path] = Path(__file__).resolve().parents[1]
INPUT_DIRECTORIES: Final = ("model/intent_v1",)
INPUT_FILES: Final = (
    "src/keyswitch/resources/models/layout_intent_v1.ksm",
    "src/keyswitch/resources/protected_tokens.txt",
    "src/keyswitch/__init__.py",
    "src/keyswitch/history.py",
    "src/keyswitch/value_provenance.py",
    *INTENT_TOOLCHAIN_VALUE_SOURCES,
    "tools/verify_intent_strict_report.py",
    "tools/intent_strict_inputs.py",
)
# The detector imports the history module, which reads two constants of its own.
VALUE_SOURCES: Final = (*INTENT_TOOLCHAIN_VALUE_SOURCES, "src/keyswitch/history.py")
SKIPPED_DIRECTORY: Final = "__pycache__"


class InputsUnreadable(Exception):
    """An input of the strict evaluation is missing or cannot be pinned."""


def input_paths(project_root: Path) -> list[str]:
    """Every input file, relative to the project root, sorted."""

    paths = set(INPUT_FILES)
    for directory in INPUT_DIRECTORIES:
        root = project_root / directory
        if not root.is_dir():
            raise InputsUnreadable(f"{directory} is not a directory")
        for path in root.rglob("*"):
            if path.is_file() and SKIPPED_DIRECTORY not in path.relative_to(root).parts:
                paths.add(path.relative_to(project_root).as_posix())
    return sorted(paths)


def file_digests(project_root: Path, paths: Iterable[str]) -> dict[str, str]:
    digests: dict[str, str] = {}
    for relative in paths:
        try:
            digests[relative] = hashlib.sha256((project_root / relative).read_bytes()).hexdigest()
        except OSError as error:
            raise InputsUnreadable(f"{relative} is unreadable: {error}") from error
    return digests


def inputs_digest(project_root: Path) -> str:
    """The SHA-256 of the input files' digests and of the pinned constant values."""

    try:
        values = pin_values(
            (project_root / relative for relative in VALUE_SOURCES),
            source_root=project_root / "src",
        )
    except ValueProvenanceError as error:
        raise InputsUnreadable(f"the constants cannot be pinned: {error}") from error
    document = {
        "files": file_digests(project_root, input_paths(project_root)),
        "values_sha256": values.sha256,
    }
    canonical = json.dumps(document, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(canonical).hexdigest()


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--project-root", type=Path, default=PROJECT_ROOT)
    arguments = parser.parse_args(list(argv) if argv is not None else None)
    try:
        print(inputs_digest(Path(str(arguments.project_root)).resolve()))
    except InputsUnreadable as error:
        print(f"strict evaluation inputs unreadable: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
