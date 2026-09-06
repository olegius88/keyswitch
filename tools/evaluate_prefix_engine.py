"""Fixed physical-key replay for the prefix candidate, before promotion.

In-process editor, not a native keyboard/IME test. The selection is hash-ranked
before scoring, from the sealed split, and never used to select model parameters.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import sys
import tempfile
import time
from collections import Counter
from collections.abc import Sequence
from dataclasses import replace
from pathlib import Path
from typing import cast
from unittest.mock import patch

from context_corpus import ROOT
from context_evidence import canonical, checksum, reference_models
from prefix_corpus import DIRECTORY, PROFILES, rows
from train_prefix_model import CANDIDATE, SEAL
from keyswitch.backend import KeyEvent
from keyswitch.config import SettingsStore
from keyswitch.early_switch import PrefixIndex, _dictionary_stems
from keyswitch.engine import KeySwitchEngine
from keyswitch.history import HistoryStore
from keyswitch.input_context import FieldContext, FieldRole
from keyswitch.language_model import LanguageModel
from keyswitch.layouts import LayoutPair
from keyswitch.prefix_model import PrefixModel

if str(ROOT / "tests") not in sys.path:
    sys.path.insert(0, str(ROOT / "tests"))
from test_input_integrity import EditorBackend

REPORT = DIRECTORY / "engine-report.json"
PER_CATEGORY = 32


def provenance() -> dict[str, str]:
    paths = [Path(__file__), CANDIDATE, SEAL, ROOT / "src/keyswitch/engine.py",
             ROOT / "src/keyswitch/context_policy.py", ROOT / "src/keyswitch/input_context.py",
             ROOT / "src/keyswitch/prefix_model.py", ROOT / "src/keyswitch/early_switch.py",
             ROOT / "src/keyswitch/resources/models/context_policy_v1.json", ROOT / "tests/test_input_integrity.py"]
    return {path.relative_to(ROOT).as_posix(): checksum(path) for path in paths}


def select(split: str) -> list[dict[str, object]]:
    unique = {cast(int, row["sequence"]): row for row in rows(split) if row["length"] == 1 and row["profile"] == "portable"}
    selected: list[dict[str, object]] = []
    for category in sorted({str(row["category"]) for row in unique.values()}):
        candidates = sorted((row for row in unique.values() if row["category"] == category),
                            key=lambda row: hashlib.sha256(("prefix-engine:" + str(row["sequence"])).encode()).hexdigest())
        selected.extend(candidates[:PER_CATEGORY])
    return selected


class Reader:
    def __init__(self, backend: EditorBackend, role: FieldRole) -> None:
        self.backend, self.role = backend, role

    def read(self, application: str, window: int) -> FieldContext:
        return FieldContext(application, str(window), self.backend.text[:self.backend.caret], role=self.role, source="fixture-field")


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
        def load(locale: str) -> LanguageModel:
            return models[0 if locale == "en_US" else 1]
        with patch("keyswitch.engine.LanguageModel.load", side_effect=load), patch.object(backend, "active_application", return_value=application):
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
                event = KeyEvent(True, index + 100, "space" if observed == " " else observed, observed, characters, backend.group, 0, index + 100)
                backend.type(event)
                engine._handle(event)
                engine._handle(replace(event, pressed=False))
                if early_at is None and engine._early_switch_origin is not None:
                    early_at = index + 1
            return {"expected": before + expected + "  ", "actual": backend.text,
                    "early_at": early_at, "injections": len(backend.injections)}


def evaluate(split: str) -> dict[str, object]:
    selected = select(split)
    variants = {"shipping_no_prefix": None, "candidate": PrefixModel.load(CANDIDATE)}
    results: dict[str, dict[str, dict[str, int]]] = {}
    failures: list[dict[str, object]] = []
    for profile in PROFILES:
        models = reference_models(profile == "reference_hunspell")
        indexes = {group: PrefixIndex(set(model.frequencies) | (_dictionary_stems(Path(model.speller.source)) if profile == "reference_hunspell" else set()), model.frequencies)
                   for group, model in models.items()}
        for native in (False, True):
            context = profile + ("/field" if native else "/observed")
            for name, model in variants.items():
                counts: Counter[str] = Counter()
                for row in selected:
                    result = replay(row, model, models, indexes, native)
                    desired = bool(row["desired"])
                    exact = result["actual"] == result["expected"]
                    early = result["early_at"] is not None and cast(int, result["early_at"]) < len(str(row["text"]))
                    counts.update({"sequences": 1, "desired": int(desired), "exact": int(exact),
                                   "restored": int(desired and exact), "changed_correct": int(not desired and not exact),
                                   "early_restored": int(desired and exact and early),
                                   "length_mismatches": int(len(str(result["actual"])) != len(str(result["expected"]))),
                                   "injections": cast(int, result["injections"])})
                    if not exact and len(failures) < 24:
                        failures.append({"variant": name, "profile": context, "sequence": row["sequence"],
                                         "category": row["category"], "desired": desired, **result})
                results.setdefault(context, {})[name] = dict(sorted(counts.items()))
                print(context, name, dict(counts), flush=True)
    passed = all(
        variant["candidate"]["length_mismatches"] == 0
        and variant["candidate"]["changed_correct"] <= variant["shipping_no_prefix"]["changed_correct"]
        and variant["candidate"]["restored"] >= variant["shipping_no_prefix"]["restored"]
        and variant["candidate"]["early_restored"] >= .7 * variant["candidate"]["desired"]
        for variant in results.values()
    )
    return {"schema_version": 1, "split": split, "selection": "32 hash-ranked situations per category, selected before scoring",
            "sequence_ids": [row["sequence"] for row in selected], "provenance": provenance(),
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
