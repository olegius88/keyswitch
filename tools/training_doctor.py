#!/usr/bin/env python3
"""Reports which model training and replay stages this machine can run, and why not the others.

A cloud container starts from a fresh clone: the system lexicons, Hunspell and the C compiler
come from `.claude/hooks/session-start.sh`, while the private frozen corpora under `.t/` never
leave the author's machine through git. This check names each missing piece before an hour of
training fails on it. It reads pins and files only; it trains, scores and downloads nothing.

Usage: PYTHONPATH=src python3 tools/training_doctor.py [--stage NAME]...
With --stage the exit status is non-zero when any named stage is blocked.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import sys
from collections.abc import Callable, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Final, cast

ROOT: Final = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))
sys.path.insert(0, str(ROOT / "tools"))

from keyswitch.constants.model_protocol import FITTING_SPLITS  # noqa: E402

CONFIG: Final = ROOT / "model/intent_v1/config.json"
PINNED_HUNSPELL: Final = ROOT / "model/intent_v1/sources/hunspell"
SYSTEM_LEXICONS: Final = Path("/usr/share/onboard/models")
SYSTEM_HUNSPELL: Final = Path("/usr/share/hunspell")
LOCALES: Final = ("en_US", "ru_RU")
PRIVATE_RELEASE: Final = ROOT / ".t/reliable-release-2026-09-12"
CONTEXT_ACTION_CORPUS: Final = PRIVATE_RELEASE / "context-action-corpus"
CONTEXT_ACTION_LEDGER: Final = PRIVATE_RELEASE / "context-action-test-ledger"


@dataclass(frozen=True)
class Check:
    name: str
    passed: bool
    detail: str


@dataclass(frozen=True)
class Stage:
    name: str
    description: str
    requires: tuple[str, ...]
    command: str


STAGES: Final = (
    Stage("replay-prefix-v1", "replay of the shipped prefix-v1 seal", ("reference_models",),
          "PYTHONPATH=src python3 tools/train_prefix_model.py verify"),
    Stage("replay-boundary-v2", "replay of the shipped boundary-v2 seal", ("reference_models",),
          "PYTHONPATH=src python3 tools/train_boundary_v2.py --verify"),
    Stage("replay-ortho-v1", "replay of the shipped ortho-v1 seal", ("reference_models",),
          "PYTHONPATH=src python3 tools/train_ortho_model.py verify"),
    Stage("replay-context-v1", "byte-exact retraining of the shipped context-v1",
          ("compiler", "system_lexicons", "system_hunspell"),
          "PYTHONPATH=src python3 tools/train_context_model.py --verify"),
    Stage("train-prefix-v2", "new prefix-v2 candidate from model/prefix_v2/recipe.json",
          ("compiler", "reference_models", "prefix_v1_data"),
          "PYTHONPATH=src python3 tools/train_prefix_v2_model.py --output .t/prefix-v2/<run>"),
    Stage("train-context-v3", "new context-v3 candidate from model/context_v3/recipe.json",
          ("compiler", "reference_models", "context_action_corpus"),
          "PYTHONPATH=src python3 tools/train_context_action_model.py "
          "--corpus .t/reliable-release-2026-09-12/context-action-corpus --output .t/context-v3/<run>"),
    Stage("evaluate-context-v3", "sequence evaluation of a context-v3 + prefix-v2 pair",
          ("reference_models", "context_action_corpus", "context_action_ledger"),
          "PYTHONPATH=src python3 tools/evaluate_context_action_sequences.py --candidate ... --seal ... "
          "--prefix-candidate ... --prefix-seal ... --output ..."),
)


def checksum(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def pins() -> tuple[dict[str, str], dict[str, tuple[str, str]]]:
    """Pinned onboard lexicon and Hunspell dictionary/affix digests by locale."""
    config = cast(dict[str, object], json.loads(CONFIG.read_bytes()))
    sources = cast(dict[str, object], config["sources"])
    languages = cast(dict[str, dict[str, str]], sources["languages"])
    external = cast(dict[str, object], config["external_evaluation"])
    hunspell = cast(dict[str, dict[str, str]], external["hunspell"])
    return ({locale: languages[locale]["sha256"] for locale in LOCALES},
            {locale: (hunspell[locale]["dictionary_sha256"], hunspell[locale]["affix_sha256"]) for locale in LOCALES})


def matching(paths: dict[Path, str]) -> Check:
    missing = [str(path) for path in paths if not path.is_file()]
    if missing:
        return Check("", False, "missing " + ", ".join(missing))
    changed = [str(path) for path, digest in paths.items() if checksum(path) != digest]
    if changed:
        return Check("", False, "differs from the pin: " + ", ".join(changed))
    return Check("", True, "matches the pins in model/intent_v1/config.json")


def check_compiler() -> Check:
    compiler = shutil.which("gcc") or shutil.which("cc")
    return Check("compiler", compiler is not None,
                 compiler or "no gcc/cc for tools/context_optimizer.c (apt-get install build-essential)")


def check_system_lexicons() -> Check:
    lexicons, _ = pins()
    found = matching({SYSTEM_LEXICONS / f"{locale}.lm": digest for locale, digest in lexicons.items()})
    return Check("system_lexicons", found.passed, found.detail + ("" if found.passed else " (apt-get install onboard-data)"))


def check_system_hunspell() -> Check:
    _, dictionaries = pins()
    expected: dict[Path, str] = {}
    for locale, (dictionary, affix) in dictionaries.items():
        expected[SYSTEM_HUNSPELL / f"{locale}.dic"] = dictionary
        expected[SYSTEM_HUNSPELL / f"{locale}.aff"] = affix
    found = matching(expected)
    return Check("system_hunspell", found.passed,
                 found.detail + ("" if found.passed else " (apt-get install hunspell-en-us hunspell-ru)"))


def check_reference_models() -> Check:
    """The pinned lexicons and morphology every live trainer loads, through libhunspell."""
    previous = os.environ.get("KEYSWITCH_HUNSPELL_PATH")
    os.environ["KEYSWITCH_HUNSPELL_PATH"] = str(PINNED_HUNSPELL)
    try:
        from reference_lexicon import reference_models
        reference_models(True)
    except (ImportError, OSError, ValueError) as error:
        return Check("reference_models", False, f"{type(error).__name__}: {error} (apt-get install libhunspell-1.7-0)")
    finally:
        if previous is None:
            del os.environ["KEYSWITCH_HUNSPELL_PATH"]
        else:
            os.environ["KEYSWITCH_HUNSPELL_PATH"] = previous
    return Check("reference_models", True, "pinned lexicons and Hunspell load")


def check_prefix_v1_data() -> Check:
    paths = [ROOT / f"model/prefix_v1/{split}.jsonl.gz" for split in FITTING_SPLITS]
    missing = [str(path.relative_to(ROOT)) for path in paths if not path.is_file()]
    return Check("prefix_v1_data", not missing, "missing " + ", ".join(missing) if missing else "prefix-v1 splits present")


def check_context_action_corpus() -> Check:
    manifest = CONTEXT_ACTION_CORPUS / "manifest.json"
    if manifest.is_file():
        return Check("context_action_corpus", True, f"manifest sha256 {checksum(manifest)}")
    return Check("context_action_corpus", False,
                 f"no {manifest.relative_to(ROOT)}: the frozen corpus is private, copy it from the "
                 "author's machine (see docs/model-training-cloud.md)")


def check_context_action_ledger() -> Check:
    present = CONTEXT_ACTION_LEDGER.is_dir()
    return Check("context_action_ledger", present,
                 "test ledger present" if present else f"no {CONTEXT_ACTION_LEDGER.relative_to(ROOT)}: "
                 "the test access ledger is private and must travel with the corpus")


CHECKS: Final[tuple[Callable[[], Check], ...]] = (
    check_compiler, check_system_lexicons, check_system_hunspell, check_reference_models,
    check_prefix_v1_data, check_context_action_corpus, check_context_action_ledger,
)


def main(arguments: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--stage", action="append", default=[], choices=[stage.name for stage in STAGES],
                        help="fail when this stage is blocked; may be repeated")
    args = parser.parse_args(arguments)
    results = {check.name: check for check in (probe() for probe in CHECKS)}
    print("Checks:")
    for check in results.values():
        print(f"  [{'ok' if check.passed else 'NO'}] {check.name}: {check.detail}")
    print("Stages:")
    blocked: set[str] = set()
    for stage in STAGES:
        missing = [name for name in stage.requires if not results[name].passed]
        if missing:
            blocked.add(stage.name)
            print(f"  [blocked] {stage.name}: {stage.description}; needs {', '.join(missing)}")
        else:
            print(f"  [ready]   {stage.name}: {stage.description}\n            {stage.command}")
    failed = sorted(blocked.intersection(cast(list[str], args.stage)))
    if failed:
        print("Blocked requested stages: " + ", ".join(failed), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
