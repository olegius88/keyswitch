#!/usr/bin/env python3
"""Visible-editor replay, with live layout changes, on reserved public phrases.

This is an in-process engine integration test, not an OS/application E2E.
It does not model IME composition, native key loss, idle timing or submissions;
the repository's separate native and action-boundary tests cover those paths.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
import tempfile
from collections import Counter
from collections.abc import Sequence
from dataclasses import replace
from pathlib import Path
from typing import cast
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
TESTS_PATH = str(ROOT / "tests")
if TESTS_PATH not in sys.path:
    sys.path.insert(0, TESTS_PATH)

from keyswitch.backend import KeyEvent, SHIFT_MASK
from keyswitch.config import SettingsStore
from keyswitch.context_model import ContextModel
from keyswitch.engine import KeySwitchEngine
from keyswitch.history import HistoryStore
from keyswitch.language_model import LanguageModel
from keyswitch.layouts import LayoutPair
from test_input_integrity import EditorBackend

from context_corpus import CORPUS_ROOT, AssignedPhrase, assign, digest, load_source
from context_evidence import canonical, checksum, reference_models
from train_context_v2 import ARTIFACT, BASELINE, validate_seal

REPORT = CORPUS_ROOT / "engine-report.json"
ROWS_PER_LOCALE = 64


def select_phrases(rows: list[AssignedPhrase]) -> list[AssignedPhrase]:
    eligible = [row for row in rows if row.split == "test" and 8 <= len(row.phrase.text) <= 96
        and re.fullmatch(r"[A-Za-z\s.,!?'’\-:;()]+" if row.phrase.locale == "eng" else r"[А-Яа-яЁё\s.,!?'’\-:;()]+", row.phrase.text)]
    selected: list[AssignedPhrase] = []
    for locale in ("eng", "rus"):
        unique: dict[str, AssignedPhrase] = {}
        for row in sorted(eligible, key=lambda item: digest("engine-replay:" + str(item.phrase.identifier))):
            if row.phrase.locale == locale:
                unique.setdefault(row.group, row)
        selected.extend(list(unique.values())[:ROWS_PER_LOCALE])
    return selected


def replay(text: str, target: int, initial: int, model: ContextModel | None, models: dict[int, LanguageModel]) -> tuple[str, int]:
    pair = LayoutPair()
    with tempfile.TemporaryDirectory(prefix="keyswitch-editor-replay-") as temporary:
        root = Path(temporary)
        settings = SettingsStore(root / "settings.json")
        settings.set("detection.context_policy", "off" if model is None else "assist")
        settings.set("detection.early_switch", False)
        settings.set("detection.learning", False)
        settings.set("general.keep_history", False)
        settings.set("diagnostics.technical_logging", False)
        backend = EditorBackend()
        backend.group = initial
        def load(locale: str) -> LanguageModel:
            return models[0 if locale == "en_US" else 1]
        with patch("keyswitch.engine.LanguageModel.load", side_effect=load):
            engine = KeySwitchEngine(settings, HistoryStore(root / "history.jsonl"), backend)
        engine.context_policy.model = model
        for serial, desired in enumerate(text, 100):
            other = pair.translate(desired, "us" if target == 0 else "ru", "ru" if target == 0 else "us")
            characters = (desired, other) if target == 0 else (other, desired)
            # Physical keys remain fixed; glyphs follow the backend's NEW
            # layout after every successful correction. Forcing initial
            # glyphs throughout would be an invalid streaming benchmark.
            observed = characters[backend.group]
            name = "space" if observed == " " else observed
            event = KeyEvent(True, serial, name, observed, characters, backend.group, SHIFT_MASK if desired.isupper() else 0, serial)
            backend.type(event)
            engine._handle(event)
            engine._handle(replace(event, pressed=False))
        return backend.text, len(backend.injections)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify", action="store_true")
    args = parser.parse_args(argv)
    validate_seal(CORPUS_ROOT)
    if REPORT.exists() and not args.verify:
        raise ValueError("engine test already observed; only identical replay is allowed")
    selected = select_phrases(assign(load_source())[0])
    if len(selected) != ROWS_PER_LOCALE * 2:
        raise ValueError("insufficient independent phrase groups")
    models = reference_models(False)
    variants = {"detector": None, "v1": ContextModel.load(BASELINE), "candidate": ContextModel.load(CORPUS_ROOT / ARTIFACT)}
    results: dict[str, dict[str, int]] = {}
    examples: dict[str, list[dict[str, object]]] = {}
    for name, model in variants.items():
        counts: Counter[str] = Counter()
        failures: list[dict[str, object]] = []
        for row in selected:
            target = 0 if row.phrase.locale == "eng" else 1
            text = row.phrase.text + " "
            for initial in (target, 1 - target):
                actual, injections = replay(text, target, initial, model, models)
                correct = initial == target
                counts["rows"] += 1
                counts["initially_correct" if correct else "initially_wrong"] += 1
                counts["preserved_correct" if correct else "exactly_restored"] += int(actual == text)
                counts["changed_correct"] += int(correct and actual != text)
                counts["length_mismatches"] += int(len(actual) != len(text))
                counts["injections"] += injections
                if actual != text and len(failures) < 12:
                    failures.append({"source_id": row.phrase.identifier, "initially_correct": correct, "expected": text, "actual": actual})
        results[name], examples[name] = dict(counts), failures
        print(f"{name}: {json.dumps(counts)}", flush=True)
    candidate, prior = results["candidate"], results["v1"]
    report: dict[str, object] = {"schema_version": 1, "scope": "in-process visible editor, portable dictionary, boundary-only; not native OS E2E or human-intent labels",
        "selection": "source test partition, hash-ranked distinct groups, 64 per locale, before model scoring",
        "source_ids": [row.phrase.identifier for row in selected], "results": results, "examples": examples,
        "provenance": {str(path.relative_to(ROOT)): checksum(path) for path in (Path(__file__), CORPUS_ROOT / ARTIFACT, BASELINE, ROOT / "src/keyswitch/engine.py", ROOT / "src/keyswitch/context_policy.py", ROOT / "src/keyswitch/input_context.py", ROOT / "tests/test_input_integrity.py")},
        "promotion_passed": candidate["length_mismatches"] == 0 and candidate["changed_correct"] <= prior["changed_correct"] and candidate["exactly_restored"] >= prior["exactly_restored"]}
    if args.verify:
        if REPORT.read_bytes() != canonical(report):
            raise ValueError("engine replay changed")
    else:
        REPORT.write_bytes(canonical(report))
    print(json.dumps({"promotion_passed": report["promotion_passed"]}), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
