"""Fixed physical-key replay for the prefix candidate, before promotion.

In-process editor, not a native keyboard/IME test. The selection is hash-ranked
before scoring, from the sealed split, and never used to select model parameters.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import multiprocessing
import os
import sys
import tempfile
import time
from collections import Counter
from collections.abc import Iterable, Sequence
from dataclasses import replace
from functools import cache
from pathlib import Path
from typing import cast
from unittest.mock import patch

from auxiliary_runtime_evidence import packaged_intent, runtime_provenance
from context_corpus import ROOT
from context_evidence import canonical, checksum
from keyswitch.constants.model_protocol import PROFILES
# The replay scores and indexes prefixes with what the engine builds (prefix_corpus.lexicon): the
# onboard lexicon plus the packaged supplement for scoring, the onboard lexicon for the index.
from prefix_corpus import DIRECTORY, lexicon, rows, verify_receipt
from train_prefix_model import CANDIDATE, SEAL
from keyswitch.backend import KeyEvent
from keyswitch.config import SettingsStore
from keyswitch.context_model import ContextModel
from keyswitch.early_switch import PrefixIndex
from keyswitch.engine import KeySwitchEngine
from keyswitch.history import HistoryStore
from keyswitch.input_context import FieldContext, FieldRole
from keyswitch.language_model import LanguageModel
from keyswitch.layouts import LayoutPair
from keyswitch.prefix_model import PrefixModel

if str(ROOT / "tests") not in sys.path:
    sys.path.insert(0, str(ROOT / "tests"))
from test_input_integrity import EditorBackend
from keyswitch.constants.training import (
    PREFIX_EARLY_RESTORED_MIN_FRACTION,
    PREFIX_EVALUATION_KEYCODE_BASE,
    PREFIX_EVALUATION_MAX_RECORDED_FAILURES,
    PREFIX_EVALUATION_SEQUENCES_PER_CATEGORY,
    PREFIX_EVALUATION_TIMESTAMP_BASE,
)

REPORT = DIRECTORY / "engine-report.json"


def provenance() -> dict[str, str]:
    return runtime_provenance(ROOT, [Path(__file__), CANDIDATE, SEAL, DIRECTORY / "corpus.json",
        ROOT / "tools/prefix_corpus.py", ROOT / "tools/train_prefix_model.py", ROOT / "tools/reference_lexicon.py",
        ROOT / "src/keyswitch/resources/lexicon-supplement-ru_RU.json"])


def select(split: str) -> list[dict[str, object]]:
    unique = {cast(int, row["sequence"]): row for row in rows(split) if row["length"] == 1 and row["profile"] == "portable"}
    selected: list[dict[str, object]] = []
    for category in sorted({str(row["category"]) for row in unique.values()}):
        candidates = sorted((row for row in unique.values() if row["category"] == category),
                            key=lambda row: hashlib.sha256(("prefix-engine:" + str(row["sequence"])).encode()).hexdigest())
        selected.extend(candidates[:PREFIX_EVALUATION_SEQUENCES_PER_CATEGORY])
    return selected


class Reader:
    def __init__(self, backend: EditorBackend, role: FieldRole) -> None:
        self.backend, self.role = backend, role

    def read(self, application: str, window: int) -> FieldContext:
        return FieldContext(application, str(window), self.backend.text[:self.backend.caret], role=self.role, source="fixture-field")


@cache
def installed_context() -> tuple[ContextModel | None, str]:
    """The installed context model, parsed once per process. Each replayed row builds an engine, whose
    context policy parsed the artifact again: 0.7 s a row, 97% of the replay. The model is not changed
    once loaded, and the provenance binds the artifact's bytes."""
    return ContextModel.try_load()


def replay(row: dict[str, object], model: PrefixModel | None, models: dict[int, LanguageModel],
           indexes: dict[int, PrefixIndex], read_field: bool) -> dict[str, object]:
    pair = LayoutPair()
    original, before, application = str(row["text"]), str(row["before"]), str(row["application"])
    source, desired = cast(int, row["source"]), bool(row["desired"])
    expected = pair.translate(original, "ru" if source else "us", "us" if source else "ru") if desired else original
    with tempfile.TemporaryDirectory(prefix="keyswitch-prefix-editor-") as temporary:
        root = Path(temporary)
        settings = SettingsStore(root / "settings.json")
        for name, value in {"detection.context_policy": "assist", "detection.early_switch": True,
                            "detection.context_read_field": read_field, "detection.learning": False,
                            "detection.respect_manual_layout": False, "general.keep_history": False}.items():
            settings.set(name, value)
        backend = EditorBackend()
        backend.text, backend.caret, backend.group = before, len(before), source
        def load(locale: str, extra_words: Iterable[str] = ()) -> LanguageModel:
            return models[0 if locale == "en_US" else 1]
        with patch("keyswitch.engine.LanguageModel.load", side_effect=load), \
                patch("keyswitch.engine.LinearNgramModel.try_load_default", return_value=packaged_intent(ROOT)), \
                patch("keyswitch.context_policy.ContextModel.try_load", return_value=installed_context()), \
                patch.object(backend, "active_application", return_value=application):
            engine = KeySwitchEngine(settings, HistoryStore(root / "history.jsonl"), backend,
                                     context_reader=Reader(backend, cast(FieldRole, row["role"])))
            engine.prefix_model, engine._prefix_indexes = model, indexes
            engine._focus_window = backend.window
            engine.context_policy.stream.focus(application, backend.window)
            engine.context_policy.stream.text = before
            engine.context_policy.stream.updated_at = time.monotonic()
            early_at: int | None = None
            for index, char in enumerate(original + "  "):
                other = pair.translate(char, "ru" if source else "us", "us" if source else "ru")
                characters = (other, char) if source else (char, other)
                observed = characters[backend.group]
                event = KeyEvent(True, index + PREFIX_EVALUATION_KEYCODE_BASE, "space" if observed == " " else observed, observed, characters, backend.group, 0, index + PREFIX_EVALUATION_TIMESTAMP_BASE)
                backend.type(event)
                engine._handle(event)
                engine._handle(replace(event, pressed=False))
                if early_at is None and engine._early_switch_origin is not None:
                    early_at = index + 1
            return {"expected": before + expected + "  ", "actual": backend.text,
                    "early_at": early_at, "injections": len(backend.injections)}


# The replays of each profile, field context and variant share nothing: each builds its own engine per
# row from the lexicons and model it is given. They run in worker processes (fork, where the platform
# has it), as many as the cores this process may use unless EVALUATION_JOBS_VARIABLE says otherwise
# (1 runs them here), and the report is assembled in the order the serial loop wrote it.
EVALUATION_JOBS_VARIABLE = "KEYSWITCH_EVALUATION_JOBS"
_REPLAY_STATE: dict[str, object] = {}
Task = tuple[str, bool, str]


def _replay_task(task: Task) -> tuple[dict[str, int], list[dict[str, object]]]:
    """One profile, field context and variant over every selected row: its counts and first failures."""

    profile, native, name = task
    lexicons = cast(dict[str, tuple[dict[int, LanguageModel], dict[int, PrefixIndex]]], _REPLAY_STATE["lexicons"])
    model = cast(dict[str, PrefixModel | None], _REPLAY_STATE["variants"])[name]
    models, indexes = lexicons[profile]
    context = profile + ("/field" if native else "/observed")
    counts: Counter[str] = Counter()
    failures: list[dict[str, object]] = []
    for row in cast(list[dict[str, object]], _REPLAY_STATE["selected"]):
        result = replay(row, model, models, indexes, native)
        desired = bool(row["desired"])
        exact = result["actual"] == result["expected"]
        early = result["early_at"] is not None and cast(int, result["early_at"]) < len(str(row["text"]))
        counts.update({"sequences": 1, "desired": int(desired), "exact": int(exact),
                       "restored": int(desired and exact), "changed_correct": int(not desired and not exact),
                       "early_restored": int(desired and exact and early),
                       "length_mismatches": int(len(str(result["actual"])) != len(str(result["expected"]))),
                       "injections": cast(int, result["injections"])})
        if not exact and len(failures) < PREFIX_EVALUATION_MAX_RECORDED_FAILURES:
            failures.append({"variant": name, "profile": context, "sequence": row["sequence"],
                             "category": row["category"], "desired": desired, **result})
    return dict(counts), failures


def _replayed(tasks: list[Task]) -> list[tuple[dict[str, int], list[dict[str, object]]]]:
    """Every task's counts and failures, in task order; in worker processes when more than one may run."""

    cores = len(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else os.cpu_count() or 1
    jobs = min(len(tasks), int(os.environ.get(EVALUATION_JOBS_VARIABLE, cores)))
    if jobs <= 1 or "fork" not in multiprocessing.get_all_start_methods():
        return [_replay_task(task) for task in tasks]
    with multiprocessing.get_context("fork").Pool(jobs) as pool:
        return pool.map(_replay_task, tasks, chunksize=1)


def evaluate(split: str) -> dict[str, object]:
    verify_receipt()
    runtime = provenance()
    selected = select(split)
    variants = {"shipping_no_prefix": None, "candidate": PrefixModel.load(CANDIDATE)}
    results: dict[str, dict[str, dict[str, int]]] = {}
    failures: list[dict[str, object]] = []
    tasks = [(profile, native, name) for profile in PROFILES for native in (False, True) for name in variants]
    _REPLAY_STATE.update(selected=selected, variants=variants, lexicons={profile: lexicon(profile) for profile in PROFILES})
    try:
        replayed = _replayed(tasks)
    finally:
        _REPLAY_STATE.clear()
    for (profile, native, name), (counts, failed) in zip(tasks, replayed):
        context = profile + ("/field" if native else "/observed")
        failures.extend(failed[:PREFIX_EVALUATION_MAX_RECORDED_FAILURES - len(failures)])
        results.setdefault(context, {})[name] = dict(sorted(counts.items()))
        print(context, name, counts, flush=True)
    passed = all(
        variant["candidate"]["length_mismatches"] == 0
        and variant["candidate"]["changed_correct"] <= variant["shipping_no_prefix"]["changed_correct"]
        and variant["candidate"]["restored"] >= variant["shipping_no_prefix"]["restored"]
        and variant["candidate"]["early_restored"] >= PREFIX_EARLY_RESTORED_MIN_FRACTION * variant["candidate"]["desired"]
        for variant in results.values()
    )
    if provenance() != runtime:
        raise ValueError("prefix runtime inputs changed during replay")
    return {"schema_version": 1, "split": split, "selection": f"{PREFIX_EVALUATION_SEQUENCES_PER_CATEGORY} hash-ranked situations per category, selected before scoring",
            "sequence_ids": [row["sequence"] for row in selected], "provenance": runtime,
            "scope": "in-process physical keys/live layout/exact final text/two spaces; separate observed and simulated field contexts; not native OS proof",
            "results": results, "examples": failures, "passed": passed}


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--development", action="store_true")
    parser.add_argument("--verify", action="store_true")
    parser.add_argument("--refresh-runtime", action="store_true")
    args = parser.parse_args(argv)
    if sum((args.development, args.verify, args.refresh_runtime)) > 1:
        parser.error("choose one replay mode")
    if not args.development and REPORT.exists() and not (args.verify or args.refresh_runtime):
        raise ValueError("prefix engine test already observed; use verify or explicit runtime regression refresh")
    result = evaluate("development" if args.development else "test")
    if args.development:
        print("development gates:", result["passed"])
        return 0
    if REPORT.exists():
        previous = json.loads(REPORT.read_bytes())
        if args.refresh_runtime:
            if result["sequence_ids"] != previous["sequence_ids"] or previous["provenance"][CANDIDATE.relative_to(ROOT).as_posix()] != checksum(CANDIDATE):
                raise ValueError("runtime refresh cannot change the observed selection or candidate")
            archive = DIRECTORY / "engine-history" / (checksum(REPORT) + ".json")
            archive.parent.mkdir(exist_ok=True)
            archive.write_bytes(REPORT.read_bytes())
            result["runtime_regression"] = {"previous_report": archive.relative_to(ROOT).as_posix(), "previous_sha256": checksum(REPORT)}
        elif "runtime_regression" in previous:
            result["runtime_regression"] = previous["runtime_regression"]
    content = canonical(result)
    if args.verify:
        if REPORT.read_bytes() != content:
            raise ValueError("prefix engine replay changed")
    else:
        REPORT.write_bytes(content)
    print("prefix engine gates:", result["passed"])
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
